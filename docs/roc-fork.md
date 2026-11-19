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
news; stamped with it, it is the answer; newer, stamped when nothing was sent, or unstamped, it
is placed by geometry. A frame Rift sends outside the core (the gesture camera) is announced
with `SentOutside { window, request }`, so nothing is inferred from a stamp.

### Plain data and the wire

`rift_core::plain` instantiates the core on plain types (`WindowId { pid: i32, idx: u32 }`,
`Rect` of f64, `RequestId(u32)`, `Nanos(u64)`); `Translate` and `Event`/`Effect::translate`
move a step between platforms' types. `plain::wire` puts events and effects into u64 words: a
tag per event and effect, one word per field (a pid sign-extended, a float as its bits), an
option as a flag word, a list as its length. The words are both the Roc core's boundary and the
recorded-trace format. Both sides parse a word list once, whole, into a typed value: words that
are not one (an unknown tag or flag, a field out of range, a list longer than the words left,
words left over) are one explicit rejection, never a crash inside a decode. The Roc core
answers `0` and the effects (or frames), or `1`: rejected, the model untouched; the host
returns `Result<_, RocError>` (`Rejected`, or `Unreadable` for Roc words it cannot read), and
rift under `roc-core` stops naming the error.

### Layout geometry (first layout slice)

`rift_core::layout` holds the geometry the tiling systems share (the axis solver, window
constraints, the tiling area) and a BSP layout's frames: `Bsp { screen, gaps, root, aspect }`,
the layout's tree pruned to what takes space (a layout where nothing does has no shape, so
`root` is a node, never empty; `Split { horizontal, ratio, children: One | Two }`,
`Leaf { slot: Empty | Window { window, constraints }, fill: Tiled | Screen | WithinGaps,
stacked }`), and `Bsp::frames`. It is a pure function, not a step: the frames are needed in the
middle of the engine's layout pass, and geometry needs no model. The engine still owns the BSP
trees and describes the shape on every arrange (`BspLayoutSystem::shape`); when tree ownership
moves into the core, the shape becomes the core's own state and the description goes.

`layout::traditional` does the same for stack and master-stack layouts (an axis is a first and
the rest, a stack a zipper), and `layout::scrolling` for a scrolling layout's strip: `Strip`
(columns, a first and the rest, of rows, a first and the rest; the width settings, alignment,
navigation) `place`s into `Placed` (each column's spot with its anchor, the world frames, the
non-empty snaps keyed by column id, the bounds), and `Placed::settle` is where the camera comes
to rest after a release (`Settle::Niri | Anchored | AnchoredTowards(column)` decides ties).
rift's strip geometry is `Empty | Placed`; the camera spring and viewport stay rift's.
`layout::floating` places a floating window the first time: `Floating { screen, spread }` and
the layout's `Newcomer`s (constraints, and `Seen::Live(frame) | Unseen`) give each its first
frame (the default 70%, a live frame no larger than it, the cascade when spreading); after that
the platform keeps the frame the user gives it.

Window constraints carry a `Sizing` (`Fixed | Resizable | ResizableAtLeastMin`, each with the
`Size` the app reported, parsed once): BSP, scrolling and floating hold a window to its
`reported_min`; traditional tiling to its `tiled_min` (a resizable window's advisory minimum
counts only under the `respect_min_size` app rule).

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
- `roc/core/Traditional.roc`: the traditional geometry (stack and master-stack layouts are
  traditional trees); `roc/core/Geometry.roc`: what the systems share (Rust's float helpers,
  constraints, the tiling area, the axis solver). A shape's words start with its system's tag
  (0 BSP, 1 traditional) and `main.roc`'s `tile` dispatches on it.
- `roc/core/Topology.roc`: the topology decisions (`rift_core::topology`: where an inventory
  finds a window, which Space discovery may admit it on, a space snapshot's admission, the
  windows an activation re-applies app rules to), as questions on plain data behind the
  model-free entry (tags 5 to 8). Held to the Rust by `tests/topology.rs`; not on rift's path.
- `roc/core/Lifecycle.roc`: the lifecycle decisions (`rift_core::lifecycle`: app-rule targets
  and refreshes, a new window's display, native transients, inventory omissions, title-change
  re-application), tags 9 to 16. Held to the Rust by `tests/lifecycle.rs`; not on rift's path.
- `roc/core/Focus.roc`: the focus decisions (`rift_core::focus`: a removal's effect on focus,
  the window to focus, hidden focus, following an activation, the fallbacks), tags 17 to 23.
  Held to the Rust by `tests/focus.rs`; not on rift's path.
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

- **Every differential has a planted bug proven caught.** A differential is evidence only if a
  deliberate bug in the Roc side fails it: plant one that changes an answer in a case the
  decision turns on, and that compiles (an unused variable is a warning, and warnings fail the
  Roc build). If it passes, the generator is too uniform to line up the deciding facts: bias it
  toward them (as `tests/focus.rs`'s removal questions are) until the bug fails it.
- `cargo test --workspace`: 1107 pass at 57426401 (2 ignored: the trace dump and the BSP clone-cost benchmark), fuzzer clean at 2000 × 40.
- `the_core_on_plain_data_decides_what_the_reactor_decided` (fuzz.rs): every seed's run,
  translated to plain data, replayed through `RustCore<Plain>`: the same effects at every step,
  and every event and effect survives the wire.
- `crates/rift-core-roc/tests/differential.rs` (`cargo test -p rift-core-roc --features roc`):
  recorded traces replayed through both cores, the Roc core compared **word for word** with the
  recorded effects (so float bits must match). 16 committed seeds (every fuzzer setup: plain,
  animated, animated with a scrolling camera) = 2,715 steps; 2000 × 40 = 339,772 steps; 4000 ×
  60 = **847,945 steps, identical**, no Roc allocation live afterwards.
- `crates/rift-core-roc/tests/layout.rs`: random traditional shapes (tiny, zero and NaN
  weights, stacks with bars and focused children, fills): 200,000 shapes, **2,446,623 frames
  identical word for word**; and random BSP shapes leaning on the edges (maxima under
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

- **Host allocator and buffers (after the rebase onto `fix/native-drag-destination`, 4000 × 60
  corpus, 819,643 steps, means):**

  | | Roc words | `RocCore::step` | Roc allocations per step |
  |---|---|---|---|
  | before (system malloc, counted, a list and two Vecs per step) | 548 ns | 680 ns | 5.62 |
  | boundary buffers reused (event list, encoding, effects copy) | 543 ns | 611 ns | 4.63 |
  | mimalloc behind `roc_alloc`/`roc_realloc`/`roc_dealloc` | 432 ns (p50 375) | 500 ns | 4.63 |
  | counting only in test builds | about 13 ns less (3%) | | |

  The Rust core is about 97 ns. Of the original gap (about 450 ns): allocation (the allocator's
  speed, the counter, the per-step input list) was about 130 ns; the boundary itself (the FFI
  call, Roc parsing the event and writing a small effect list, the host's copy) is about 110 ns,
  measured with a Roc step that parses and answers without deciding; the Rust side's wire encode
  and decode is about 70 ns more in `RocCore::step`; Roc's decisions are the rest, about 320 ns,
  some 3.3× the Rust core's. The remaining 4.6 allocations per step are Roc's own (the model's
  rebox, the effect list, the lists a step builds); no arena. Size-class pools were not tried:
  mimalloc already serves these small blocks from thread-local free lists.
; both are far under a frame (16 ms) and under the cost
  of one AX call.
- Flat from 10 to 2000 windows: the model stays unique and Roc updates it in place.
- Allocations: about 5.5 per step (the event list, the model's unbox/rebox, the effect list,
  and whatever the step builds), at most 4 live in a session, 0 after every core is dropped; a
  million steps over 500 windows keep live allocations flat at 4.
- At `experiment/roc-paused` the Roc core was ~130 ns per step against Rust's ~35 ns: today's
  core does more per step (a record per window, pending targets for every touched window, plans)
  and the wire codec runs in Roc.
- **Long sessions (`tests/session.rs`):** one core over the recorded seeds back to back, each
  shifted so its clock and request ids carry on (time only moves forward): 4000 seeds × 5 =
  4,239,725 steps over 14.4 simulated hours, slowest Roc step 216 µs, decisions identical to the
  Rust core's throughout, at most 5 Roc allocations live. The default test (the committed seeds,
  200 rounds, 2.1 h) fails if more than 10 steps take over 5 ms. An earlier run had shown 40–100
  ms steps on one core carried across seeds whose clocks restart; it does not reproduce, neither
  with time running backwards nor on the build that showed it (`b9675712`, rerun three times:
  slowest step under 1 ms), so it was the machine (other builds running), not the core. The
  deadline queue cannot grow with a forward clock: a due or stale entry is dropped, so it holds
  only the writes of the last 300 ms.

- **No index in the geometry (index audit), a known cost.** The owner chose unrepresentable
  states over microseconds ("premature bug-fixing rather than premature optimisation"):
  the solver walks segment records instead of indexing parallel lists, a traditional stack is a
  zipper and an axis a first and the rest (in the Rust core too), and children are zipped with
  their lengths and frames. Tile, best of 5 over 200,000 random shapes: BSP 4.04 → 4.9 µs per
  shape, traditional 5.6–5.9 → 8.1 µs. If a profile ever asks, optimise inside these
  structures (the measured costs: bigger records than parallel float lists, and one list for
  each zip). The per-step core is unaffected.
- **BSP trees in the core (phase A).** Every operation is checked from the old system's true state: its state converted into the core's, the operation run on both, the answers, the trees (memories included, tags aside) and the container query (node ids renumbered, everything else byte for byte, stack members' ids exact) compared. The owner's scripts' patterns (`apply-display-layouts.sh`, `swap-rotate.sh`) read the core's container tree as the old one. Found and fixed in the old system on the way (upstream candidates): an emptied fullscreen leaf kept its fullscreen flags (739cddd3); a stacked window swapped with itself was duplicated (18607987). Noted, not changed: the old container query's node ids can collide (a slotmap key equal to a member's pid<<32|idx). Clone cost (decision c), release, per clone: 1 layout x 4 windows 113 ns old / 121 ns core; 1 x 20: 0.2 / 0.43 us; 4 x 20: 0.55 / 1.6 us; 8 x 50: 2.7 / 12.3 us (a box per split). Microseconds against a 16 ms frame: no sharing needed unless a profile says otherwise.
- **Traditional trees in the core: paused at 952a2a0b** for upstream #688 (a rework of every
  layout system, tree.rs, selection and persistence). Done: 1198d8b4 (old-system fix, an
  upstream candidate: draw_tree prints the weights' real sum, not a running cache only it read);
  a5455f5b (`layout::traditional_tree`: Kind `Axis | Stack { along, ungrouped }`, Fill on every
  node, a Weight per child and no stored total, the selection as `Children::Remembered { before,
  at, after, walk: Enters | Stops } | Unremembered { first, rest }`, and the read model);
  952a2a0b (the conversion from rift's NodeId trees, with a typed OldTraditionalTree, and every
  query held to the core's answer on all 998 lib tests, draw_tree and container_tree compared
  with node ids renumbered). Found: an emptied container is kept as a placeholder a restore
  refills. **Stopped, 2026-11-19, after two old-system fixes (upstream candidates) on roc-2:**
  6027249e (rnnxzrzu) adding a window a layout already holds changes nothing (literal test
  `adding_a_window_a_layout_already_holds_changes_nothing`); aba68bce (wmowqklx) joining toward
  an adjacent stack joins that stack (the lead's ruling), at the end it came from and still
  selected. Upstream's `joining_into_existing_stack_keeps_it_stacked` passed only through the
  duplicate add; it is rewritten without it (ignored in the first commit, un-ignored by the
  fix). Removing either fix fails its test. No more traditional work until #688 lands; the bug is
  written up as an upstream issue draft. Still parked off roc-2:
  the first operations with an operation-level shadow (change mnzktyzo; wxsqyrxs is now
  superseded by the two commits). To apply on resuming: the placeholder named in
  the type (`Placeholder` with its weight, or `Holding(Children)`), not `Option<Children>`; the
  leaves' Sizing as the play line has it; no resync seam (master-stack and stack policy into the
  core before the flip); phase A to the play line first, with a `traditional-divergence:` line.
- **Tests read the live window server (fixed, 57426401).** In test builds `window_spaces` and
  `window_ordered_in` fell through to SkyLight without an override; fixture ids are real window
  ids too, so after a restart two reactor tests failed on every revision back to a9d2a909. Test
  builds now answer from the override alone. The remaining two followed in f0fd51bd.
- **Tests read the live window server, the rest (fixed, f0fd51bd).** `space_is_user` and
  `space_is_fullscreen` now read a Space's kind off its id in test builds, by the convention
  `actor::spaces` uses (below 0x400000000 a user desktop, 0x400000000..0x800000000
  fullscreen); `app_window_suitability` answers as macOS does for a window that does not exist.
- **One held window, by construction (da3e5ab8).** The window store's records are one map of
  `Entry::Loose { record, transfer } | Gripped { record, grip, hand }`: a window is in one
  place and a held one has no transfer. The store mints one `Hand` (neither `Clone` nor
  `Copy`), which gripping moves into the entry, so a second held window would need a second
  hand. Entries leave the map in one place (`take`), which returns a held entry's hand. Grip
  effects the store can't apply come back as `GripRefused`: logged in a running Rift, a panic
  in tests. A property test drives random pointer, re-key, removal and transfer sequences
  through the core. The Roc core is unchanged (its `Grip` was already one value).
- **Floating, a known cost.** 0.42 µs per layout of one to six newcomers in Roc (best of 5
  over 200,000); the parity runs 200,000 random layouts bit-identical.
- **Scrolling, a known cost.** 4.4 µs per strip in Roc (best of 5 over 200,000 random strips;
  the strip parity runs 200,000 strips and 200,000 settle questions bit-identical). A Rust
  float sum and Roc's differ when grouped differently: a row's y advances by `(height + gap)`,
  as Rust adds it; the random strips found that. The Roc build script watches `roc/core` and
  `roc/platform` as directories: a module missing from its old file list (`Scrolling.roc`) left
  a stale archive under test until the mismatch was traced to it.
- **Hardening: parse at the boundary, protective types, narrow signatures (the owner's "a, and
  then some").** What changed, Rust core first and Roc in parity:
  - *Parse once.* `Wire.roc` reads with `Try` (`Taken(a) : Try((a, Words), [Malformed])`),
    chained with `?`; `finished` rejects leftover words; `take_u32`/`take_window` check ranges,
    `take_list` checks the length against the words left. No `crash` remains in Roc's decode.
    Known cost: Roc words ~418 → ~486 ns per step (+16%); tile unchanged.
  - *`Sent<R>` and `Stamp<R>`.* A request the core sent (constructible only in the reconciler)
    and an id the platform handed back (compares only against a `Sent`): the as_of rule is an
    `Ordering` match with no inference branch. Rules out: treating a platform id as one of ours.
  - *A plan only the core makes* (`Plan`'s fields private, `Plan::starting(write, ..)`): a plan
    always has its write, so the empty plan branch is gone.
  - *Narrowed signatures (the corollary: a check that can move to the caller means the
    parameter is too loose).* `release` takes the queued desire and the state the window was
    left in, not an `Option` and a lookup: its "nothing queued" and "window unknown" branches
    moved to (or vanished from) its callers. `time_out` and `waiting` take and give a `Sent`.
    `Bsp::frames` / `Traditional::frames` take a shape whose root is a node: the "nothing takes
    space" branch moved to rift's builders, which already knew it, and off the wire.
  - *Empty is a state, not an unwrap.* The deadline queue is matched `[] | [entry, .. as
    later]`; `dedup` folds `[Nothing, Last(window)]`. No `first()`, `last()` or
    `drop_first(1)` left in Roc.
  - *No wildcard over state.* Every `_ =>` over core state is spelled out, in rift-core and
    Roc (gather's `Fixed`/`Free` and `Max`/`NoMax` pairs, the minima pass, BSP aspect fitting,
    the queue). The only `_ =>` left in Roc map an unknown wire tag to `Err(Malformed)`.

  What is left, and why:
  - *Ratio newtype (0..1).* Every writer clamps to 0.05..0.95 or uses 0.5; the boundary that
    could carry a bad ratio is persistence (serde of saved layouts), which is rift's, not the
    core's. Worth doing there, with the layout trees.
  - *Seen / acked / desired frame newtypes.* No bug has mixed them (seed 59 was a semantic bug),
    and `seen` legitimately takes desired values; the states (`Settled { acked }`, `InFlight {
    target }`) already name them.
  - *Request ids minted by the core.* The platform mints them from a counter shared with other
    writers, so the core cannot; `Sent`/`Stamp` give the protection that matters.
  - *Window id vs window-server id.* Distinct types already, on the platform side; the core
    sees only window ids.
  - *`keys[&plan.request()]` in rift's reconciler.* An index into the platform's own map of
    in-flight batches, keyed by the request it minted; not core state.
  - *The `roc` feature between a Rust-only commit and its Roc parity.* A Rust-only commit that
    changes the wire (b71d8cd9) leaves the Roc build's layout test failing until the next
    commit; the Roc-free workspace, which the play line uses, is green at every commit.
    roc-2 joined `feat/respect-min-size-core` by a merge (2518ba1b), not a rebase: the base
    already held the final Rust shapes (a required root, the traditional zipper, Sizing), so
    rebased, every older Roc commit's wire would have failed to compile.
- **Pure functions.** No Roc function is effectful (`=>`): the platform's entry points are
  pure too. rift-core reads no clock, randomness or I/O (time and request ids come in as
  arguments); the Roc host only moves words and memory.

Build:

| | time |
|---|---|
| `roc check core/main.roc` | 0.02–0.26 s |
| `roc build --opt=speed` (archive, 464 KB) | 8.7–10 s |
| rift with `roc-core`, after touching `main.roc` (Roc build + relinking rift) | 32 s |
| rift without it, after touching `rift-core` | 16 s |

## Roc: the design we follow, and what is left (nightly-2026-11-15-f25da45)

Sources are the Roc checkout at the pinned commit (`f25da45`): `docs/langref/`, `design.md`,
`docs/mini-tutorial-new-compiler.md`, `src/build/roc/Builtin.roc`, `src/cli/main.zig`. Earlier
notes called several of these friction; they are Roc's design, and the core follows them.

### Design we follow

- **No `Option`, no null.** Failures are `Try`, absence is a descriptive tag union (mini
  tutorial, "`Try`": "Roc does not have `null`, `nil`, `undefined`, or anything similar").
  The core says what absence means: `Last : [Sent(U32), NeverSent]`, `Waiting : [Waiting(Rect),
  NoneWaiting]`, `Armed : [Armed(U64), Disarmed]`.
- **`List.get` / `List.set` return `Try`** (langref `lists.md`: "there's no such thing as an
  'index out of bounds' crash in Roc"). So an index the code knows is valid means the data
  structure is wrong: the index audit replaced those with shapes where it cannot happen (a
  stack is a zipper, an axis a first and the rest, frames and lengths are zipped with the
  children, a stack's frame is a formula of its position).
- **Closed and open records.** `{ target }` is a closed record (pattern), `{ target, .. }` an
  open one (langref `records.md`, "Open Record Types"; `pattern-matching.md`).
- **Type modules.** A file `Wire.roc` defines `Wire := …` and exposes only that type's
  associated items; everything else at the top level, type aliases included, is private
  (langref `modules.md`, "Hiding implementation details"). The old `module [..]` header is
  deprecated (the compiler's diagnostic at this commit). Structural aliases are
  interchangeable, so each module declares the `Rect` it uses.
- **Warnings exit 2.** Every `roc` command that finishes with warnings exits 2, so CI fails
  (mini tutorial; `src/cli/main.zig`, `exitOnWarnings`: "the one warning exit policy"). The
  build script treats it as a failure, and the core is kept warning-free.
- **Shadowing is a warning** (langref `naming.md`, "Shadowing"), which with the exit policy
  fails a build: a match branch binding `release` next to a `release` function had to be
  renamed.
- **Recursive types are nominal** (langref `types.md`, "Recursive Types"; `boxes.md`):
  `Node := [..]`, built as `Node.Split(..)`.
- **Ranges are syntax**, `0..<n` (langref `numbers.md`, "Ranges"; `loops.md`), not a
  `List.range`. (The hand-written `indices_to` this replaced was a misreading, and the audit
  removed the need for it.)
- **`where` clauses ask for methods**, not record fields (langref `static-dispatch.md`): a
  function over records names their alias instead.

### Not yet implemented

- **Alias modules** (exposing a type alias as a module): "planned for after 0.1.0 and have not
  been implemented yet" (langref `modules.md`, "Alias modules").

### Unknown (behaviour at this commit, not documented)

- **`F64` rounding.** `round_to_*_try` rounds by `floor(x + 0.5)` (`Builtin.roc`,
  `f64_round_to_whole`), which differs from Rust's `f64::round` at `0.49999999999999994` and
  loses the sign of a zero; there is no `F64 -> F64` round or floor. The geometry writes Rust's
  rounding out (`Geometry.round`) to stay bit-identical.
- **`F64.max` / `F64.min`** are `if a > b then a else b` (`Builtin.roc`): NaN and signed-zero
  behaviour is not specified; the geometry writes out arm64's `fmaxnm`/`fminnm`.
- **Glue and packaging** (only the three boundary functions use it now): structural records
  are named by hash, the generated Rust is not clean under Rust 2024
  (`unsafe_op_in_unsafe_fn`, allowed on the module), and the nightly tarball has no glue specs
  (`roc/glue.sh` takes `ROC_SRC`).

### Worked well

- `output: Archive` links into cargo with nothing but `rustc-link-lib=static`.
- `var` with `while` / `for` makes the deadline loops and the wire codec direct ports.
- Nothing crashed, leaked or miscompiled on 829,148 differential steps, 4.2 million session
  steps and 3.5 million geometry frames.

## Pure edges, step 3 (the reactor's remaining effectful handlers)

Steps 1–2 are the `refactor/pure-edges` commits ending at a9d2a909 (on the play stack and
roc-2). Step 3 moves the decisions left in effectful handlers into pure functions with a thin
edge: test first, each commit verified on its own (fmt, workspace tests, fuzz 2000 × 40),
layout_engine untouched, then mirrored into Roc with a bit-identical differential. Ranked by
fuzzer and bug history:

1. **Space activation and the topology snapshot merge**: the per-window native Space and
   fullscreen observation in `on_windows_discovered_with_app_info`; SpaceStateChanged admission
   and commit (`events/space.rs`); `reconcile_windows_with_authoritative_spaces`; the
   `set_active_spaces*` family. History: #691, duplicate workspaces on single-display changes,
   fullscreen-return fixes, #665.
2. **Window lifecycle**: `sync_native_admission`, `observe_transient_lifecycle`,
   `maybe_reapply_app_rules_for_window`, `place_new_window_on_pointer_display`, inventory
   retirement's inputs. History: #690, #684, #652, #651, #658, unminimize.
3. **Raise and focus sequencing**: `focus_desktop_if_active_workspace_empty`,
   `request_refocus_if_hidden`, `prepare_refocus_*`, `follow_focused_window_to_its_display`,
   `auto_switch_to_window_workspace`, `focus_untracked_window_under_cursor`. History: #677,
   raise_manager and Mission Control, #661 twice.

Not in the list: the input mode table, already pure (`src/actor/reactor/modes.rs`).

Done (Rust; the Roc mirror follows the Rust side):

- 91726e1b: where an app inventory finds a window, `discovered_space` (`Standing` →
  `DiscoveredSpace::Observed | Live | Fallback`), out of `on_windows_discovered_with_app_info`.
- e2757007: which Space discovery may admit a reported window on, `discovery_spaces`
  (`DiscoveryStanding::FullscreenRecorded | Placeholder | Admissible`), out of
  `discovery_spaces_for_window`.

- 4225e354: whether a space snapshot is held or committed and what it invalidates,
  `admit_space_snapshot` (`events/space.rs`; `Admission::Hold(MissionControl |
  MembershipIncomplete) | Commit { invalidated, lost_local_desktops }`), out of
  `handle_authoritative_space_snapshot`.
- e38c5551: which windows an activated Space re-applies app rules to, `app_rule_candidates`,
  out of `apply_app_rules_for_activated_spaces`. (Computing the active set was already pure,
  `SpaceActivationPolicy::compute_active_spaces`; whether it changed is one set comparison.)

- 7938584b: the Roc mirror of #1. The four decisions moved into `rift_core::topology`
  (generic over Spaces, windows, window-server ids and displays; the reactor builds the plain
  views and asks), and `roc/core/Topology.roc` answers them behind the model-free entry, tags
  5 to 8. `crates/rift-core-roc/tests/topology.rs`: 200,000 random questions each, every answer
  identical word for word, no Roc allocation left live; a planted bug in the Roc admission
  fails it. The reactor still decides with the Rust.

#2, window lifecycle, in `rift_core::lifecycle` from the start (the reactor asks it):

- e5aeb256: which windows app rules run over, and whether the layout hears of one,
  `rule_targets` / `rule_refreshes_layout` (`RuleRun::Admission | Reapply`, `RuleOutcome`),
  out of `process_windows_for_app_rules`.
- 8d4f04bd: where a new window opens in global mode, `pointer_display_target`
  (`NewWindow::Untracked | NotStandard | Standard(rule inputs)`), out of
  `place_new_window_on_pointer_display`.
- 968544ed: native transients and which entries outlive a window or app,
  `is_native_transient` / `keeps_transient` (`TransientOrigin`, `Departure`), out of
  `sync_native_admission` and `observe_transient_lifecycle`.
- 9e5a8f6f: what an inventory says about the windows it omits, `misplaced` /
  `out_of_inventory_view`, out of `on_windows_discovered_with_app_info`.
- 15e3d902: whether a title change re-applies app rules, `reapplies_on_title_change`
  (`OnTitleChange`), out of `maybe_reapply_app_rules_for_window`.
- 7a76eb0a: the Roc mirror, `roc/core/Lifecycle.roc`, tags 9 to 16;
  `crates/rift-core-roc/tests/lifecycle.rs`, 200,000 random questions each, identical word for
  word; a planted bug in the Roc Space ordering fails it.

#3, raise and focus sequencing, in `rift_core::focus`:

- 1f24c75c: what a removal does to focus, `removal_focus` (`Removal` → `RemovalFocus::
  Unaffected | KeepNative | Refocus`), out of `prepare_refocus_before_removal`.
- aec2fce0: the window to focus in a workspace and hidden focus, `focus_candidate` /
  `hides_focus`, out of `visible_focus_candidate_in_active_workspace` and
  `request_refocus_if_hidden`.
- fc6f3fc8: whether an activation follows its window, `activation_follow` over an
  `ActivationFacts` trait (asked in order, no further than needed) → `Follow::To | Stay(why)`
  with eleven typed reasons, out of `auto_switch_target`; the reactor's `ActivationQuery`
  answers and logs as before.
- f887a6d4: the fallbacks, `display_to_follow`, `untracked_focus`
  (`UntrackedFocus::Nothing | SystemSurface | MakeKey`) and `focuses_desktop`.
- ac7017a4: the Roc mirror, `roc/core/Focus.roc`, tags 17 to 23;
  `crates/rift-core-roc/tests/focus.rs`, 200,000 random questions each, identical word for
  word. Its first generator missed a planted bug (a removed window that macOS still reports as
  key, with a successor listed): uniform random ids rarely line up the facts that decide. The
  removal questions now lean on those cases, and the bug fails it. A planted bug must also
  compile: an unused variable is a warning, and warnings fail the Roc build.

Step 3 is done (the input mode table was already pure).

## Window lifecycle as core events

The lead's ruling: move window lifecycle (then workspaces and spaces) into the core as events;
as each piece becomes core events, the Roc core decides it behind `roc-core`, shadowed by the
Rust. Plan: facts parsed at the boundary (`Manageability`, `Rule`, `AxKind`, `NativeKind`,
`Minimize`); presence `Placeholder | Live(facts) | Invalidated(facts) | Claimed { owner, held }`;
admission never stored, a function of the facts; `Admitted` / `Suspended` effects only on a
transition. Rules out: a claimed window with a workspace in the index, an invalidated record
with no AX state, an admitted minimized or native-transient window, a rule override without a
rule, a suspension for a window that was not admitted. Slices: L1 admission as a pure core
function; L2 the core holds facts and presence in shadow (a tripwire compares admission on every
step); L3 the flip (the store's facts become a projection of core effects); L4 AX invalidation
as presence; L5 claims as presence; L6 retirement in the core.

- f0ea99d4 (L1): `rift_core::lifecycle::admission` on `AdmissionFacts` → `Admitted |
  Vetoed(Veto)`; `WindowState::is_admitted_with_override` builds the facts and asks. Held to
  the old boolean rule over all 96 combinations of the loose facts.
- cf5c2042 (L1, Roc): tag 24; the differential checks all 72 combinations of typed facts;
  a planted bug in the Roc dialog-window rule fails it.
- 9715b121 (L2): the shadow. `rift_core::lifecycle::Lifecycle` holds every live window's
  facts (`LifecycleEvent::Live | Changed | Gone` in, `Transition::Admitted | Suspended(why)`
  out). The window store owns it and is the only writer of the admission facts:
  `WindowState`'s manageability, native-transient flag and manage override are private to the
  model, and `set_manageable`, `set_manage_override`, `set_native_transient`, `set_minimized`,
  `replace_info`, `insert_window` and the AX-state drops tell the core. After every reactor
  event `lifecycle_divergences` must be empty: tests fail on it; a running Rift logs
  `lifecycle-divergence:` at debug under `rift_wm::actor::reactor`. A literal test shows a
  write around the setters is caught; a setter that does not tell the core fails three reactor
  tests.
- 7e9e980d (L2, Roc): the step's logic is the pure `read` (facts before + `Reading` → facts
  after + `Change`); tag 25; the differential checks all 6,205 pairs; a planted bug fails it.
- 5c2b0c50 (L3, the flip; held for the play line until L2 has soaked with no
  `lifecycle-divergence:` line): the heuristic's verdict, the manage override and the
  native-transient flag leave `WindowState` and live only in the core's `Lifecycle`. The store
  answers admission from it (`is_admitted`, `is_admitted_state`, `admitted_with_override`,
  `rule`, `manage_override`); its setters write the core alone and return the transition;
  `insert_window` keeps the old defaults and `insert_window_as` carries a known verdict and rule.
  `sync_native_admission` suspends on the transition `set_native_transient` returns, not on a
  before/after comparison. Minimize and the AX roles stay in the AX snapshot; the tripwire now
  checks the snapshot and the core agree on them. Making `set_native_transient` skip the core
  fails a reactor test. Left for later: `refresh_heuristic`'s callers still compare admission
  before and after.
- c173c69f (L4, AX invalidation as presence; held behind the L2 soak with L3): the record's
  `ax_invalidated` flag goes. The core's `Lifecycle` holds a window with AX state as
  `Live(facts) | Invalidated(facts)`: invalidated keeps its facts and place, is not managed, and
  comes back on a positive AX observation or a fresh reading; an identity moving on carries
  its invalidation. Invalidating a window with no AX state is nothing (it never had an AX
  element, or its fullscreen identity is already unavailable). Three engine tests gave AX state
  to windows built from layout events alone. The lifecycle line pauses here until the L2 soak.
- A placeholder (a record with an assignment and no AX state: a restored layout's window not
  yet seen, a rule's pre-assignment, a fullscreen AX identity being replaced) has no facts in
  the core, so it can never be admitted; L4 names it as presence.

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
| Layout geometry: traditional (stack, master-stack) | M | Done, Rust and Roc (`layout::traditional`, `Traditional.roc`); the stack bar's group frames use the core's `stack_frames`. |
| Sizing (respect_min_size in the core) | S | Done, Rust (the play line) and Roc, with its own wire encoding. |
| Layout geometry: scrolling | M | Done, Rust and Roc: the strip's placement and where the camera settles (`layout::scrolling`, `Scrolling.roc`). The camera's spring and gesture motion stay rift's (time-sampled presentation). |
| Layout geometry: floating | S | Done, Rust and Roc: first frames and the cascade (`layout::floating`, `Floating.roc`). |
| Layout trees into the core | L+ | **In progress, BSP first; traditional paused for upstream #688.** `layout::bsp_tree` holds a BSP layout (splits of two, leaves of nothing / a window / a stack, the selection as each split's memory, node tags the core mints) and every change rift makes to one; in test builds each of `BspLayoutSystem`'s operations is shadowed by the core on all traffic, and a tree fuzzer composes them. Next: the core answers (the flip), then the old system goes. Then traditional, scrolling's columns, floating. |
| Window lifecycle complete | M | Admission, visibility, minimize, claims as core events. |
| Workspaces and spaces | L | Virtual workspaces, space membership, the topology quarantine and snapshot merge. |
| Focus and raise | M | Focus follows mouse, raise sequencing; raise FIFO replay conflicts with discard-never-queue. |
| The mode table | L | The other ~30 modes as one `Mode x InputClass` table in the core. |
| Config | S–M | Parsed config as data the core is initialized and updated with. |

**roc-2 ends** when the layout trees are the core's and the slices above them have moved.

**After roc-2, in a new fork: a publishable macOS platform for Roc, and a fat Roc rift on it.**
The platform is general, not rift's: its API has no rift types. About ten primitives, each an
effect as data with request ids and opaque handles: accessibility elements and attributes (get,
set, observe), window-server windows and spaces (queries), apps (launch, terminate, activate),
the input tap driven by a policy table that the app pushes, timers, and files and IPC. Per-app
worker threads stay native, inside the platform; every decision moves into the Roc app, whose
step stays pure. Rift is one app on that platform. Host code extracted from rift for it stays
Apache-2.0, keeping rift's and glide-wm's attribution.

Out of the core for good: AX calls, event taps, CG/SkyLight, run loops and timers, IPC, the
animation presenter itself.
