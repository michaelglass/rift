# The core boundary, in Rust and in Roc

The goal: all of rift's decisions in a pure core, `step(model, event) -> effects`, written in
Roc, with the Rust platform doing only effects (AX, taps, CG/SkyLight, run loops, IPC, the
animation presenter). Each slice of rift moves into the core in Rust first (`RustCore`), then
the Roc app takes it over, held to the Rust core decision for decision.

History: the first Roc port stopped at `experiment/roc-paused` (2026-11-18) while the Rust
rearch finished on `experiment/rust-core` (the play-test line, now the owner's daily driver).
The Roc work resumed on `experiment/roc-2`, branched from `experiment/rust-core`.

## The boundary

```rust
// crates/rift-core
impl<K: Kinds> RustCore<K> { pub fn step(&mut self, event: Event<K>) -> Vec<Effect<K>>; }

pub trait Kinds {                         // the platform's types
    type Window: Ord + Hash; type Frame; type Time: Ord; type Request: Ord;
    fn is_within(a: Frame, tolerance: f64, b: Frame) -> bool;
    fn after(t: Time, d: Duration) -> Time;
    fn same_size(a: Frame, b: Frame) -> bool;
}
```

Events are facts the platform parsed: the hand (`Press`, `Drag`, `Release`, `LetGo`), the
window lifecycle (`Seen`, `Unseen`, `Gone`, `Ceded`, `Rekeyed`, `Distrust`, `Forget`), layout's
desires (`Arrange { now, want, desires }`, `Animate { now, desires }`), the animation's end
(`AnimationEnded`, `AnimationStopped`), a modifier drag's frame (`Follow`), a window's reported
frame (`Observe { now, request, window, frame, as_of }`) and the deadline timer (`Tick`).

Effects are decisions: `Write` (only the core can make one), `Animate`, `Plan` (an arrange
pass's writes, per request, position-only vs full), `Queued`, `Seen` (where the model now takes
a window to be), `Pending` (a window's pending target, for every window whose state changed),
`Replied` (what a report was: NoNews, InTransit, Acked, Refused, Foreign), `Note`, `Timer`, and
the grip's `Grip`, `DragBegan`, `Ungrip`, `Regrip`. Order in a step is fixed: decisions, notes,
pending targets, then the timer if its deadline moved.

The model, one record per window: `seen` (where the model takes it to be) and `sync`, one of
`Settled { acked, last }`, `Unknown { last }`, `InFlight { target, deadline, queued, request,
before }`, `Animating { target, request }`, `Refused { wanted, got, request }`; the hand,
`Grip::Idle | Held { window, button, phase: Pressed | Dragging { changed } }`; and the armed
deadline. The one rule for reads: a report stamped older than the window's latest request is no
news; stamped with it, it is the answer; newer (or stamped with no request sent) on a settled
window, it is a frame Rift sent outside the core; unstamped, it is placed by geometry.

### Plain data and the wire

`rift_core::plain` instantiates the core on plain types (`WindowId { pid: i32, idx: u32 }`,
`Rect` of f64, `RequestId(u32)`, `Nanos(u64)`); `Translate` and `Event`/`Effect::translate`
move a step between platforms' types. `plain::wire` puts events and effects into u64 words: a
tag per event and effect, one word per field (a pid sign-extended, a float as its bits), an
option as a flag word, a list as its length. The words are both the Roc core's boundary and the
recorded-trace format.

### Layout geometry (first layout slice)

`rift_core::layout` holds the geometry the tiling systems share (the axis solver, window
constraints, the tiling area) and a BSP layout's frames: `Bsp { screen, gaps, root, aspect }`,
the layout's tree pruned to what takes space (`Split { horizontal, ratio, children: One | Two }`,
`Leaf { slot: Empty | Window { window, constraints }, fill: Tiled | Screen | WithinGaps,
stacked }`), and `Bsp::frames`. It is a pure function, not a step: the frames are needed in the
middle of the engine's layout pass, and geometry needs no model. The engine still owns the BSP
trees and describes the shape on every arrange (`BspLayoutSystem::shape`); when tree ownership
moves into the core, the shape becomes the core's own state and the description goes.

## What lives where

- **Core (pure):** the reconciler (writes, acks, refusals, deadlines, the queued desire, the
  animation's targets, plans), where the model sees each window, the hand, the timer; and the
  geometry of BSP layouts and the axis solver.
- **Platform (the reactor, Rust):** everything effectful, and what has not moved yet: layout
  (which makes the desires), workspaces and spaces, focus and raise, the mode table, config,
  window admission. `RiftState::step` applies each step's effects in one place (`project`); the
  window store's `frame_monotonic` and the transaction store's targets are projections of
  effects, with debug assertions that store and core agree.

## The Roc side

- `roc/platform/main.roc`: requires `core : { init : {} -> model, step : model, List(U64) -> {
  model, effects : List(U64) } }`; provides `roc_init`, `roc_step`, `roc_release_model`; target
  `arm64mac: { inputs: [app], output: Archive }`. The model crosses only as `Box(Model)`.
- `roc/core/main.roc` (~1,260 lines, ~330 of them the wire codec): `RustCore` ported decision for
  decision. State inside the variants, `Option`s as small tag unions (`Last : [Sent(U32),
  NeverSent]`, `Waiting : [Waiting(Rect), NoneWaiting]`), every match naming its variants. The
  app parses the event's words into its own `Event` union at the start of `step` and writes the
  effects' words at the end.
- `crates/rift-core-roc`: a member of rift's workspace (cargo makes every path dependency inside
  it one, `exclude` or not) but **empty unless its `roc` feature is on**, so rift's own build
  never needs the Roc nightly (`ROC=/nonexistent cargo test --workspace` passes). With `roc`,
  `build.rs` runs `roc build` and links the archive; `RocCore::step_words` steps on words,
  `RocCore::step` adds the wire; the allocator hooks count allocations; `roc_crashed` aborts.
- In rift, `--features roc-core` (which turns on `rift-core-roc/roc`) makes the reactor's core
  `Shadowed`: the Roc core decides (the event translated to plain data, across as words, its
  effects translated back), the Rust core steps the same event beside it, and the two must
  decide identical effects or rift stops. `Plainly` is the one translation both ways.
- `roc/core/Bsp.roc`: the BSP geometry, behind the platform's second, model-free entry
  `tile : List(U64) -> List(U64)` (a shape's words in, the frames' words out); under `roc-core`
  rift's `layout_engine::tile_bsp` takes its frames and asserts the Rust core's are the same.
  Bit for bit means Rust's float semantics written out: `f64::round` (half away from zero,
  keeping a zero's sign; Roc's own is `floor(x + 0.5)`), `max`/`min` as arm64's
  `fmaxnm`/`fminnm` (+0 beats -0 for max), `sum` from -0. `roc/core/Wire.roc` holds the word
  primitives both modules use.
- Pinned nightly: `roc/NIGHTLY`. After a bump, run `roc/glue.sh`. The glue is now 1,469 generated
  lines (it was 4,020 with typed events and effects) and only three functions use it.

## Evidence

- `cargo test --workspace`: 963 pass (1 ignored, the trace dump), fuzzer clean at 2000 × 40.
- `the_core_on_plain_data_decides_what_the_reactor_decided` (fuzz.rs): every seed's run,
  translated to plain data, replayed through `RustCore<Plain>`: the same effects at every step,
  and every event and effect survives the wire.
- `crates/rift-core-roc/tests/differential.rs` (`cargo test -p rift-core-roc --features roc`):
  recorded traces replayed through both cores, the Roc core compared **word for word** with the
  recorded effects (so float bits must match). 16 committed seeds (every fuzzer setup: plain,
  animated, animated with a scrolling camera) = 2,715 steps; 2000 × 40 = 339,772 steps; 4000 ×
  60 = **847,945 steps, identical**, no Roc allocation live afterwards.
- `crates/rift-core-roc/tests/layout.rs`: random BSP shapes leaning on the edges (maxima under
  minima, locked axes, ratios 0 and 1, one-child splits, fractional screens) through both
  geometries: 200,000 shapes, **1,057,049 frames identical word for word** (20,000 shapes by
  default). Its first failure was a signed zero (`round(-0.3)` is -0 in Rust).
- A planted bug in `Bsp.roc` fails the BSP and engine tests under `roc-core`: the Roc geometry
  is on rift's path. The fuzzer drives no BSP layout yet.
- With `--features roc-core`: all reactor tests pass, and the fuzzer at 2000 × 40 and 4000
  × 60 finds nothing, every step agreeing with the Rust shadow.
- Coverage of the 4000 × 60 corpus by event: Observe 335,314, Arrange 191,974, Seen 177,126,
  Animate 60,787, Release 23,185, AnimationEnded 16,612, Press 13,597, Drag 12,771, Tick 6,388,
  Gone 4,397, Follow 3,727, AnimationStopped 1,718, Forget 349. **Never in the corpus:**
  `LetGo`, `Rekeyed`, `Ceded`, `Unseen`, `Distrust`; the Roc core decides those only in the
  reactor tests under `roc-core` (shadowed). The fuzzer should drive them.

### Measured

`tests/measure.rs` (`cargo test --release -p rift-core-roc --features roc --test measure --
--ignored --nocapture`), release, M-series laptop. "Roc words" is the step across the
boundary (the Roc app parses the event and writes the effects); "Roc step" adds the Rust side's
wire encode and decode.

| per step | Rust core | Roc words | Roc step |
|---|---|---|---|
| fuzzer corpus (4000 seeds, 847,945 steps, a fresh core per seed) | mean 95 ns, p50 83 ns, p99 416 ns, max 79 µs | mean 538 ns, p50 459 ns, p99 2 µs, max 45 µs | mean 682 ns, p50 625 ns, p99 2 µs |
| synthetic, 10 windows | mean 123 ns, p99 1 µs | mean 704 ns, p99 5 µs | mean 863 ns, p99 6 µs |
| synthetic, 2000 windows | mean 136 ns, p99 250 ns | mean 702 ns, p99 792 ns | mean 971 ns, p99 1 µs |

- Roc is about 5× the Rust core per step; both are far under a frame (16 ms) and under the cost
  of one AX call.
- Flat from 10 to 2000 windows: the model stays unique and Roc updates it in place.
- Allocations: about 5.5 per step (the event list, the model's unbox/rebox, the effect list,
  and whatever the step builds), at most 4 live in a session, 0 after every core is dropped; a
  million steps over 500 windows keep live allocations flat at 4.
- At `experiment/roc-paused` the Roc core was ~130 ns per step against Rust's ~35 ns: today's
  core does more per step (a record per window, pending targets for every touched window, plans)
  and the wire codec runs in Roc.
- **Seen once, not explained:** one Roc core carried across all 4000 seeds (each seed's clock
  starting at 0 again, which no real session does) had steps of 40–100 ms at random events;
  per-seed cores have none over 45 µs. Likely the deadline list: with time running backwards,
  deadlines that never come pile up, and `drop_first` followed by `append` may copy the whole
  list in Roc where Rust's `VecDeque` does not. Not a production path, but the deadline queue
  is the first thing to look at if Roc ever shows spikes.

Build:

| | time |
|---|---|
| `roc check core/main.roc` | 0.02–0.26 s |
| `roc build --opt=speed` (archive, 464 KB) | 8.7–10 s |
| rift with `roc-core`, after touching `main.roc` (Roc build + relinking rift) | 32 s |
| rift without it, after touching `rift-core` | 16 s |

## Roc friction found (nightly-2026-11-15-f25da45)

- `output: Archive` links into cargo with nothing but `rustc-link-lib=static`.
- Typed glue names structural records by hash (`AnonStruct4045c5188ea548d`); with words across
  the boundary only the step's result record is generated, which no longer matters.
- The generated glue is not clean under Rust 2024 (`unsafe_op_in_unsafe_fn`): allowed on the
  module. The nightly tarball has no glue specs (`ROC_SRC` points at a checkout).
- No `F64 -> F64` rounding; the core never rounds.
- Record patterns are closed: `{ target, .. }` is needed against a bigger record.
- `module [..]` headers are deprecated in favour of type modules (`Wire := [].{ .. }`), and
  `roc build` exits 2 on any warning, which fails a cargo build script. Type modules do not
  expose type aliases; structural aliases are interchangeable, so each module declares its own
  `Rect`. Recursive types must be nominal (`Node := [..]`, built as `Node.Split(..)`).
- `List.set` returns a `Try`; there is no `List.range`.
- Names in a `match` branch shadow top-level definitions as an error (`Release(release)` next to
  a `release` function).
- `List.update` returns a `Try` (index out of bounds), so updating a found index needs a second
  match; `map` over the (short) plan list was simpler.
- `var` + `while`/`for` make the deadline loops and the wire codec direct ports.
- Nothing crashed, leaked or miscompiled on 847,945 steps.

## Roadmap: to "the reactor is only an applier"

Each slice moves state into the core (Rust first), deletes reactor code, keeps the fuzzer, the
replay test and the differential green, and ends with the Roc app deciding it.

| Slice | Size | State |
|---|---|---|
| Reconciler (writes, acks, deadlines, queued desire) | M | Done, Rust and Roc. |
| Request ids, the grip, the model's frame | M | Done, Rust and Roc. |
| One pending store, plans, animation targets | M–L | Done, Rust and Roc. |
| `DragKind` union | M | Done on the Rust line (platform side; the drop target is derived there until layout moves). |
| Layout geometry: BSP | M | Done, Rust and Roc (the engine describes the shape; the core computes the frames). |
| Layout geometry: traditional (stack, master-stack), scrolling, floating | M each | **Next**: the same for the other systems; the axis solver is already the core's. Scrolling has cached world geometry (`prepare_layout`) to carry. |
| Layout trees into the core | L+ | Tree mutations as core events, the engine's trees a projection of effects; the shapes become core state. Per system, BSP first. |
| Window lifecycle complete | M | Admission, visibility, minimize, claims as core events. |
| Workspaces and spaces | L | Virtual workspaces, space membership, the topology quarantine and snapshot merge. |
| Focus and raise | M | Focus follows mouse, raise sequencing; raise FIFO replay conflicts with discard-never-queue. |
| The mode table | L | The other ~30 modes as one `Mode x InputClass` table in the core. |
| Config | S–M | Parsed config as data the core is initialized and updated with. |

Out of the core for good: AX calls, event taps, CG/SkyLight, run loops and timers, IPC, the
animation presenter itself.
