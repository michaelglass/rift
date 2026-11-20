//! The Roc core against the Rust core, on the steps the reactor really took.
//!
//! Each trace is one fuzzer seed's run through rift's reactor, recorded as the events the
//! reactor stepped the core with and the effects the (Rust) core decided, in the wire format
//! (`rift_core::plain::wire`, little-endian words; the traces' only use of it). Both cores
//! replay every event from a fresh model and must decide exactly the recorded effects: the Roc
//! core's compared as words, bit for bit.
//!
//! Traces come from `$RIFT_TRACE_DIR` when set (write them with
//! `RIFT_TRACE_DIR=… cargo test dump_core_traces -- --ignored` in rift), else from the small
//! sample committed under `traces/`.

use std::path::PathBuf;

use rift_core::RustCore;
use rift_core::plain::{Plain, wire};
use rift_core_roc::{AllocationCount, RocCore};

fn traces() -> Vec<(String, Vec<u64>)> {
    let dir = std::env::var("RIFT_TRACE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("traces"));
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "trace"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no traces in {}", dir.display());
    files
        .into_iter()
        .map(|path| {
            let bytes = std::fs::read(&path).unwrap();
            let words = bytes
                .chunks_exact(8)
                .map(|word| u64::from_le_bytes(word.try_into().unwrap()))
                .collect();
            (path.file_name().unwrap().to_string_lossy().into_owned(), words)
        })
        .collect()
}

#[test]
fn the_roc_core_decides_what_the_rust_core_decided() {
    let before = AllocationCount::now();
    let mut steps = 0;
    let mut kinds = std::collections::BTreeMap::<String, usize>::new();
    for (name, words) in traces() {
        let trace = wire::decode_trace(&words).unwrap_or_else(|error| panic!("{name}: {error:?}"));
        let mut rust = RustCore::<Plain>::default();
        let mut roc = RocCore::default();
        for (index, (event, recorded)) in trace.into_iter().enumerate() {
            let kind = format!("{event:?}");
            let kind = kind.split([' ', '(', '{']).next().unwrap_or_default().to_owned();
            *kinds.entry(kind).or_default() += 1;
            let from_rust = rust.step(event.clone());
            assert_eq!(
                &from_rust, &recorded,
                "{name} step {index}: Rust core, {event:?}"
            );
            // Compared as words: bit for bit, floats included.
            let from_roc = roc.step(event.clone());
            if wire::encode_effects(from_roc.clone()) != wire::encode_effects(recorded.clone()) {
                panic!(
                    "{name} step {index}: Roc core, {event:?}\n  roc:      {from_roc:?}\n  recorded: {recorded:?}"
                );
            }
            steps += 1;
        }
    }
    let after = AllocationCount::now();
    assert_eq!(
        after.live(),
        before.live(),
        "Roc allocations left live after every core was dropped: {before:?} -> {after:?}"
    );
    eprintln!(
        "differential: {steps} steps identical; Roc allocated {} times",
        after.allocated - before.allocated
    );
    eprintln!("differential: events by kind: {kinds:?}");
}

/// The reconciler's ack tolerance, held at its edge: a window is told a frame, then answers
/// (stamped with the write's request) at offsets either side of the tolerance and of values a
/// slip could give it, on every edge of the frame. Both cores must decide the same effects at
/// every step, bit for bit: the committed traces alone pass a wrong tolerance.
#[test]
fn the_cores_answer_a_write_alike_at_the_edge_of_the_ack_tolerance() {
    use rift_core::plain::{Nanos, Rect, RequestId, WindowId};
    use rift_core::{Desire, Event, Want};
    let window = WindowId { pid: 7, idx: 1 };
    let target = Rect {
        x: 100.,
        y: 100.,
        w: 400.,
        h: 300.,
    };
    let tolerance = rift_core::reconciler::ACK_TOLERANCE;
    let offsets = [
        0.,
        tolerance - 0.01,
        tolerance,
        tolerance + 0.01,
        2.,
        2.5 - 0.01,
        2.5,
        3.,
    ];
    let before = AllocationCount::now();
    let mut checked = 0;
    for offset in offsets {
        for edge in 0..4 {
            let mut answered = target;
            match edge {
                0 => answered.x += offset,
                1 => answered.y -= offset,
                2 => answered.w += offset,
                _ => answered.h -= offset,
            }
            let request = RequestId(1);
            let events = [
                Event::Seen {
                    window,
                    frame: Rect { x: 0., y: 0., w: 400., h: 300. },
                },
                Event::Arrange {
                    now: Nanos(1_000),
                    want: Want::Arrange,
                    desires: vec![Desire { window, frame: target, request }],
                },
                Event::Observe {
                    now: Nanos(2_000),
                    request,
                    window,
                    frame: answered,
                    as_of: Some(request),
                },
                Event::Tick {
                    now: Nanos(1_000_000_000),
                    request,
                },
            ];
            let mut rust = RustCore::<Plain>::default();
            let mut roc = RocCore::default();
            for event in events {
                let from_rust = rust.step(event.clone());
                let from_roc = roc.step(event.clone());
                assert_eq!(
                    wire::encode_effects(from_roc.clone()),
                    wire::encode_effects(from_rust.clone()),
                    "{offset} off on edge {edge}, {event:?}\n  roc:  {from_roc:?}\n  rust: {from_rust:?}"
                );
                checked += 1;
            }
        }
    }
    assert_eq!(
        AllocationCount::now().live(),
        before.live(),
        "Roc allocations left live"
    );
    eprintln!("differential: {checked} steps at the ack tolerance's edge identical");
}
