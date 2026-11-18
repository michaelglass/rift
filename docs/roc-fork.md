# The core boundary, in Rust and in Roc

Experiment on bookmark `experiment/roc`, on top of `experiment/listen-only-tap`. The Roc code
may be thrown away; the boundary (`crates/rift-core`) is the part meant to go back to rift.

## The boundary

One step function. Events are facts the platform parsed; effects are decisions as data. The
model stays behind the trait and is moved through every step, never lent out.

```rust
// crates/rift-core
pub trait Core<K: Kinds> { fn step(&mut self, event: Event<K>) -> Vec<Effect<K>>; }

pub trait Kinds {                     // the platform's types
    type Window; type Frame; type Time;
    fn is_within(a: Frame, tolerance: f64, b: Frame) -> bool;
    fn after(t: Time, d: Duration) -> Time;
}

pub enum Event<K> {
    Pointer(Pointer<K::Window>),      // Idle | Pressing(w) | Dragging(w), sent when it changes
    Want   { now, window, desired, known, want: Want },  // layout's desire; Arrange | ArrangePositions | Place
    Follow { window, frame },         // modifier drag's interactive frame
    Answer { now, window, frame },    // the app's reply to its latest write
    Report { window, frame, known },  // unprompted frame report
    Tick   { now, known: Vec<(Window, Frame)> },  // deadline timer fired; absent = gone
    Forget { window },                // an animation / topology change took the frame
    Gate   { initiative: Initiative },// the mode gate: may Rift do this now?
}

pub enum Effect<K> {
    Write(Write<K>),                  // send this frame (only a Core can make one)
    Queued { window },                // the desire waits behind the write in flight
    Answered { window, answer },      // Acked | Refused | Unasked
    Late { window },                  // answer after giving up: comes back as a Report
    Observed { window, observation }, // Ack | InTransit | Foreign
    Note { window, note },            // Discarded | TimedOut | Forgotten (for the log)
    Gate { initiative, allowed },     // refused = discarded, never queued
    Timer(Option<Time>),              // arm the deadline timer at, or disarm
}
```

Order inside one step is fixed: decisions, then notes, then the timer if its deadline moved.

`RustCore<K>` holds the reconciler (moved here unchanged in behaviour; it is handed `now`
instead of reading a clock), the pointer as last told, and the armed deadline. rift runs it as
`RustCore<Rift>` on its own `WindowId`/`CGRect`/`Instant`, so the reactor converts nothing.
`plain::Plain` instantiates it on i32/u32 ids, f64 rects and u64 nanoseconds: what a foreign
core sees.

## What lives where

- **Core (pure):** the reconciler (`Sync`: Settled / InFlight / Refused / Adrift, the one write
  in flight plus one queued, deadlines in send order), whose a window is (`Owner`, from the
  pointer), whose hand a report is under (`Holder`), the drag mode gate (`may`), the timer.
- **Platform (the reactor, Rust):** everything effectful and everything not yet moved: the
  window store with the press/drag model (`Records`, `Grip`), layout (`calculate_layout` makes
  the desires), `frame_monotonic` (the model's last-seen frame, sent as `known`), the
  transaction ids of the app protocol, the senders, the run loop's timer.
- `src/actor/reactor/reconciler.rs` is the thin applier: each former reconciler call is now
  "tell the pointer if it changed, step one event, turn the effects back into what the reactor
  acts on". In test builds it records every step.

Three Rust-only reads remain on `RustCore` (a foreign core has no such door):
`sync` (tests and fuzzer invariants), `in_flight` (the transaction-id check in
`answers_latest_write`), and `due` (the adapter uses it to send `Tick` only the known frames of
windows whose writes are due; a host without it sends every window it knows, which decides the
same).

## The Roc side

- `roc/platform`: requires `core : { init : {} -> model, step : model, Event -> { model, effects
  : List(Effect) } }`, provides `roc_init`, `roc_step`, `roc_release_model`; target
  `arm64mac: { inputs: [app], output: Archive }`. The model crosses only as `Box(Model)`.
- `roc/core/main.roc`: `RustCore` ported decision for decision (~340 lines). State inside the
  variants (`InFlight({ target, deadline, queued })`, `Queued : [Waiting(Rect), NoneWaiting]`),
  exhaustive matches without wildcards.
- `crates/rift-core-roc` (outside the workspace: building it needs the nightly): `build.rs` runs
  `roc build` and links the archive; `RocCore` implements `Core<Plain>`, converts to and from the
  generated glue, copies effects out and releases the Roc list at once; the allocator hooks count
  allocations; `roc_crashed` aborts.
- Pinned nightly: `roc/NIGHTLY`. After a bump, run `roc/glue.sh` (the glue ABI changes between
  nightlies).

## Evidence

- `cargo test --workspace`: 949 pass. The reactor makes every frame-write and mode-gate decision
  by stepping `RustCore<Rift>`.
- `the_core_on_plain_data_decides_what_the_reactor_decided` (fuzz.rs): every fuzzer seed's run is
  recorded, translated to plain data and replayed through `RustCore<Plain>`; same effects at every
  step (200 seeds × 30 steps by default).
- `crates/rift-core-roc/tests/differential.rs`: the recorded traces replayed through both cores.
  2000 seeds × 40 steps = **413,825 core steps, identical effect streams**, no Roc allocation
  live afterwards. The same on the `-safe` nightly. A planted bug (ack tolerance 1.5 → 2.5) fails
  it at seed 1 step 230.
- Coverage gap: the fuzzer has no modifier drags or animations, so `Follow` and `Forget` never
  occur in the traces (counts: Gate 158,926, Want 136,164, Report 67,135, Answer 33,699,
  Pointer 14,299, Tick 3,602).

Measured (`tests/measure.rs`, release, M-series laptop, adapter conversions included):

| | Rust core | Roc core |
|---|---|---|
| fuzzer corpus, per step | mean 35 ns, p99 125 ns | mean 127 ns, p99 333 ns |
| synthetic, 10 → 2000 windows | mean 53–56 ns, p99 125 ns | mean 162–169 ns, p99 ~291 ns |
| Roc allocations | | ~2.2 per step (model box + effect list), 4 live, 0 after drop |
| 1M steps, 500 windows | | live allocations flat at 4, 0 after drop |

Flat latency from 10 to 2000 windows (half of them refused, so the model tracks ~1000) shows
the model stays unique: Roc updates it in place, it never copies it.
Build: `roc build` of the archive ~2 s; clean `cargo build --release` of the Roc crate 2.8 s.

## Roc friction found (nightly-2026-11-15-f25da45)

- `output: Archive` links into cargo with nothing but `rustc-link-lib=static` (the research
  could not find an example; it works).
- The glue's names for structural records are hashes (`AnonStructE34ca824b553e6e2`), with
  per-position aliases (`EventWantWindow`); one record shape gets one name everywhere. Nominal
  records in the platform would read better.
- The generated glue is not clean under Rust 2024 (`unsafe_op_in_unsafe_fn`): allowed on the
  module.
- The nightly tarball has no glue specs: `roc glue` needs `RustGlue.roc` from a roc checkout at
  the same commit (`ROC_SRC`).
- No `F64 -> F64` rounding (only `round_to_i64_try` and friends). The core never rounds, so
  `Kinds::round` was dropped; rounding stays in the reactor's `Desired::of`.
- Record patterns are closed: `InFlight({ target })` against a three-field record is a "can
  never match" type error; it needs `{ target, .. }`. The message does say which field is
  missing.
- No `Option`: the core's `Option`s became small tag unions (`Waiting | NoneWaiting`,
  `Arm(U64) | Disarm`). Arguably better.
- Unbox and rebox per step costs one allocation and free per step. Cheap; noted because a
  per-step allocation count of 0 is not reachable with `Box(Model)` round-tripping.
- `var` + `while` made the deadline-queue loops direct ports of the Rust.
- Nothing crashed, leaked or miscompiled on 413k steps.

## Next slice

1. Request ids on `Write`, answers carrying them: `answers_latest_write` and `in_flight` move into
   the core (removes a Rust-only read; the transaction ids become platform detail).
2. The press/drag model (`Records`/`Grip`, `PointerReleased`) into the core: pointer events
   (`Pressed{window, button}`, `Moved`, `Released`) instead of the adapter telling it the
   pointer's state. Then `Owner`/`Holder` have one source.
3. `frame_monotonic` owned by the core (inventory and animation send events), so `Want`/`Report`
   stop carrying `known` and `Tick` stops needing `due`.
4. The fuzzer drives modifier drags and an animation, so `Follow`/`Forget` are covered.
5. Run `RocCore` inside the reactor behind a cargo feature (a translating `Core<Rift>` adapter),
   so the 949 reactor tests run on the Roc core too.
