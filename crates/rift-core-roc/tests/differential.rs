//! The Roc core against the Rust core, on the steps the reactor really took.
//!
//! Each trace is one fuzzer seed's run through rift's reactor, recorded as the events the
//! reactor stepped the core with and the effects the (Rust) core decided, on plain data. Both
//! cores replay every event from a fresh model and must decide exactly the recorded effects.
//!
//! Traces come from `$RIFT_TRACE_DIR` when set (write them with
//! `RIFT_TRACE_DIR=… cargo test dump_core_traces -- --ignored` in rift), else from the small
//! sample committed under `traces/`.

use std::path::PathBuf;

use rift_core::plain::Plain;
use rift_core::{Core, Effect, Event, RustCore};
use rift_core_roc::{AllocationCount, RocCore};

type Trace = Vec<(Event<Plain>, Vec<Effect<Plain>>)>;

fn traces() -> Vec<(String, Trace)> {
    let dir = std::env::var("RIFT_TRACE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("traces"));
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no traces in {}", dir.display());
    files
        .into_iter()
        .map(|path| {
            let trace = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            (path.file_name().unwrap().to_string_lossy().into_owned(), trace)
        })
        .collect()
}

#[test]
fn the_roc_core_decides_what_the_rust_core_decided() {
    let before = AllocationCount::now();
    let mut steps = 0;
    for (name, trace) in traces() {
        let mut rust = RustCore::<Plain>::default();
        let mut roc = RocCore::default();
        for (index, (event, recorded)) in trace.into_iter().enumerate() {
            let from_rust = rust.step(event.clone());
            let from_roc = roc.step(event.clone());
            assert_eq!(
                &from_rust, &recorded,
                "{name} step {index}: Rust core, {event:?}"
            );
            assert_eq!(&from_roc, &recorded, "{name} step {index}: Roc core, {event:?}");
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
}
