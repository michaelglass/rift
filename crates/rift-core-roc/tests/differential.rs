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
