# The core boundary, in Rust and in Roc

The goal: all of rift's decisions in a pure core, `step(model, event) -> effects`, written in
Roc, with the Rust platform doing only effects (AX, taps, CG/SkyLight, run loops, IPC, the
animation presenter). Each slice of rift moves into the core in Rust first (`RustCore`), then
the Roc app takes it over, held to the Rust core decision for decision.

History: the first Roc port stopped at `experiment/roc-paused` (2026-11-18) while the Rust
rearch finished on `experiment/rust-core` (the play-test line, now the owner's daily driver).
The Roc work resumed on `experiment/roc-2`, branched from `experiment/rust-core`.

## Handoff (2026-11-24, layout trees: slices 1 and 2 done, slice 3 next): read this first

**State.** Workspace `rift-roc`, line `experiment/roc-2`. W4d+W6 is done; core geometry on
upstream's layout systems (Next item 2) is complete: stack, master-stack, traditional, floating, BSP, scrolling's placement and its translation, and the core's
shared geometry is brought up to upstream's #688 (below). `rearch/roc-next@fork` carries everything
through this docs commit once it is published (publish procedure: "Rulings that bind",
Public-clean; the published tip's id is in the slice report). All checks green: 885 lib tests (3
ignored, one of them `measure_layout_shadow`), 894 with `roc-core`, the fuzzer at 2000 × 40 both ways, `cargo test -p rift-core-roc
--features roc`, `cargo test -p rift-core`, `cargo check --locked` both ways, fmt, and `cargo clippy
--all-targets --features roc-core` compiles with no errors and 331 warnings (332 before the dead trees went), counted as `cargo clippy
--all-targets --features roc-core 2>&1 | grep -E '^(warning|error)' | grep -v generated | wc -l`
(the bound: at most 336); no build warnings outside test code (`cargo test` prints three old
unused-variable / unused-mut warnings in fuzz.rs, tests.rs and systems.rs).

**Layout trees, slice 2, done: the core's BSP tree, stepped beside upstream's system.** Commits
(after 4d365f91): 09b58190 (the tree, layouts, participation, settings, the shape; the twin),
7c1387d6 (queries), 628587c6 (window changes), c8790070 (focus), 28de94e3 (shape edits),
1e2bdd50 (moves, swaps, drops), 1ef6c735 (joins, consume-or-expel, unjoin; every operation
steps the core), a2d238d3 (the cost). Each message has its plants. Checks at the tip: 890 lib
tests (3 ignored), 899 with `roc-core`, rift-core 106, the rest as above, clippy 331.
- **The core:** `rift_core::layout::bsp_tree::Tree<L, N, W>`: `SlotTable<L, Layout { root }>`,
  `SlotTable<N, Node { parent: Parent::Root | Split(N), kind }>`, the windows left out (a
  `BTreeSet`), `InsertionPoint`, the aspect ratio. `Kind::Split(Split { orientation, ratio: f32,
  children: [N; 2], memory: Memory::Stops | Toward(Side) })` / `Kind::Leaf(Leaf { content:
  Content::Empty | Held { members: Members::Window(W) | Stack(Tabs { before, top, after }),
  fill: Fill }, preselect: Preselect })`. Operations, upstream's one for one: `create_layout`,
  `clone_layout`, `remove_layout`, `set_window_participation`, the settings, every query,
  `add_window`, `remove_window`, `replace_window`, `close_app(predicate)`, `resize_window(Resize
  { old, new, screen, tiling })`, `focus(Focus) -> Focused`, `edit(Edit) -> Edited`, and
  `shape(layout, screen, gaps, constraints) -> Option<Bsp>` (the geometry's input, pruned as
  upstream prunes). Ids are the SlotTable's, equal to upstream's slotmap ids (compared directly).
- **The twin (`systems/bsp/twinned.rs`):** upstream's struct is `Upstream`, unchanged; rift's
  `BspLayoutSystem { upstream, twin: Box<TwinnedBsp> }` (serialized as upstream, byte for byte;
  deserialized through `From<Upstream>`, which seeds the core). Every operation steps both;
  answers are compared on every operation (queries too), the whole tree (layouts, nodes by id,
  left out, settings) on every operation in tests and `roc-core` builds and every 32nd in a plain
  release build, each pass's shape on every pass. A difference: counted (`rift-cli metrics
  show-timing`: `core: bsp tree divergences: N`), a test fails, a running Rift warns
  `bsp-tree-divergence:` and re-seeds the core from upstream. Seeding reads each slotmap's slots
  (vacant versions from its saved form) and its free list (the order a copy hands vacant slots
  out), taken by `SlotTable::resumed` (a load would take the highest vacant slot first).
- **The drawn fuzzer** (`bsp.rs`'s `drawn_system`, under the BSP differential, 3000 systems)
  drives `BspLayoutSystem`: every edit and focus, layouts created, cloned and removed, windows
  replaced, native resizes (into and out of the screen and tiling frames, nudges around the
  half point), participation (windows in no layout too), the settings, the edits upstream has no
  case for, drops straight to `edit_layout`, every query on a third of its steps, and reloads
  through serde, most with a preselected leaf (only a saved file holds one).
- **Upstream states the core does not hold (no operation reaches them; only a saved file can):**
  both fullscreen flags set (read as `Fill::Screen`, as upstream's projection reads them; a
  later toggle would then differ from upstream), and an empty leaf with a fullscreen flag
  (dropped: upstream clears the flags as a leaf empties; only IPC's `container_tree` showed
  it). Upstream's load normalizes neither. For slice 4 (saving from the core, old files
  byte-compatible): normalize them at the load boundary with one warn line, or keep two flags;
  the lead's call. A NaN ratio from a file compares unequal to itself (`f32`), so would report a
  divergence; none is drawn.
- **Cost (release, `measure_bsp_twin`, everyday operations: focus moves, swaps, resizes, a
  window added and removed):**

| windows | upstream / op | core step / op | whole-tree view | view per op (every 32nd) | re-seed | a pass's shape (upstream, core) |
|---|---|---|---|---|---|---|
| 10 | 68 ns | 80 ns | 181 ns | 6 ns | 110 us | 1.1 us, 1.0 us |
| 100 | 0.79 us | 1.46 us | 2.7 us | 84 ns | 0.57 ms | 5.3 us, 5.3 us |
| 1000 | 7.6 us | 14.0 us | 13.2 us | 0.41 us | 5.0 ms | 49 us, 49 us |

  So a plain release build adds about 14.4 us to an operation at 1000 windows (the core's step,
  about twice upstream's, plus the sampled view), and each pass builds the shape twice (about
  49 us more at 1000 windows). Not optimized (premature bug-fixing first); a re-seed happens
  only after a divergence.

**Next: layout trees, slice 3, the Roc mirror of the BSP tree** (the lead's plan): a Roc
`BspTree` with the same operations, held to the Rust core by a Rust/Roc differential over drawn
operation sequences (answers and trees), plants in Roc. Then 4, the flip (policy (a)): the
core's tree decides and is saved (old files byte-compatible); 5 deletes `Upstream`. The queued
Roc nightly bump (below) is still waiting: the newest nightly is still nightly-2026-11-23-b5c77fc
(checked 2026-11-24).

**Slice 2's plan and rulings, as given (the lead's, 2026-11-24; done, above).** Slice 1
(the dead pre-#688 core trees deleted) is done, and `SlotTable` now lives in the core
(`rift_core::slot_table`, generic over `SlotKey`; see below). The plan, accepted: BSP, then
traditional (covering stack, master-stack and traditional), then scrolling's columns, then
floating. Per system: (1) delete (done for both trees), (2) port with a two-way shadow, (3) the Roc
mirror, (4) the flip, (5) the old tree code goes once nothing reads it. Report to the lead at the
end of each slice and on any behaviour difference.

Rulings for slice 2:
- **Where it lives:** a `TwinnedBsp` inside `BspLayoutSystem`; every `LayoutSystem` operation
  steps both. The engine glue stays untouched. Note what the code shows: `BspLayoutSystem` keeps
  ONE node table for all its layouts (`layouts: SlotMap<LayoutId, LayoutState { root }>`, `nodes:
  SlotMap<NodeId, Node>`, `model::tree::NodeId`), so the core value is per system, not per layout:
  `SlotTable<LayoutId, Root>` and `SlotTable<NodeId, Node>`, so that ids are allocated in one
  sequence as upstream's are.
- **Port, not adapt:** each operation is ported from #688's `systems/bsp.rs` (`impl LayoutSystem
  for BspLayoutSystem`, line 1211 on, and its helpers: `smart_insert_window`, `split_leaf`,
  `remove_window_internal`, `rebalance_node`, `detach_child`, `LeafMembers::{focus, remove,
  append, prepare_slot_swap, take_stack, swap_tabs}`, join/unjoin, swap, target drop, fullscreen,
  preselection, the insertion point and the single window's aspect ratio). Upstream's rules are
  the spec; the old `bsp_tree`'s type shapes (in history, deleted in 89748de1: `Content::Empty |
  Window | Stack(Tabs)`, `Fill`, `Preselect`, `Memory`) are reused where they fit, and no rule
  upstream lacks is kept. A difference the shadow shows: upstream wins, unless it is an upstream
  bug: then stop, report, and it joins the upstream candidates.
- **Node ids:** minted by the core's `SlotTable` with upstream's keys (`NodeId`, `LayoutId` get
  `SlotKey` through `crate::model::slot_table`'s `slot_keys!` macro), so the ids equal upstream's
  for the same sequence of operations: no change over IPC (`container_tree`, `rift-cli query
  layout`), saved files byte-compatible without a mapping, and the shadow compares ids directly
  (no renumbering). If minted tags turn out to be needed anywhere, stop and report.
- **The shadow:** on all lib traffic, plus a drawn tree fuzzer (`systems::drawn`), as phase A did.
  The stateful shadow steps on every operation in every build (tree operations are per command
  and cheap); only the view comparison is sampled, every 32nd operation in a plain release build.
  Measure the per-operation cost at 1000 windows and put it here.
- **The NoAxState arm** stays doing nothing, with its one-line reason (done).

**Queued, between slices (the lead, 2026-11-24): the next Roc nightly bump.** roc#13363 (PR 13389,
`LifecycleStepped`'s `S(m)`), roc#13348 (PR 13386, a one-tag union with several payloads described
by its first payload only) and roc#13349 (PR 13393, RustGlue output not `include!`-able, edition
2024 / clippy lints) are fixed on roc main; roc#13350 (a tag named `Release` colliding with the
union's helper) is still open. The pinned nightly-2026-11-23-b5c77fc predates all three. At the
first nightly containing those PRs, bump, then remove the workarounds, one commit each:
`LifecycleStepped`'s `S(m)` (into the platform module beside the other entries' results); the
build script's `//!`/`#![...]` comment rewrite and the glue module's lint allows, if the new output
needs neither; any workaround for the one-tag union description, if there is one. Do it between
slices, not inside one.

**`SlotTable` moved into the core (the lead's ruling (c), before slice 2).** `rift_core::slot_table`
holds the table and slotmap 1.1's rules (versions, free list, the sentinel, refusals) over a
`SlotKey` trait (`from_parts(index, version)`, `parts()`); the core has no slotmap or serde
dependency. `SlotTable::saved()` yields each slot's key and value (or none) and version, and
`SlotTable::parse` takes `(value, version)` pairs (`len` gained `is_empty`, held by the differential). `crate::model::slot_table` keeps the platform
side: `SavedSlot<T>` (slotmap's `SerdeSlot`), `saved` / `parse` over it, the `slot_keys!` macro
(slotmap keys as `SlotKey`, by their ffi form `version << 32 | index`;
`VirtualWorkspaceId` today), and the differential against a real `SlotMap`. Plants: the removal's
version bump (1 lib failure), the free list first-in-first-out (2), the sentinel kept as saved (1);
`vacant | 1` as `vacant + 1` is equivalent (a vacant version is even).

**The window store's single writer, done (the lead's ruling, 2026-11-24: the upstream PR plan's
C1 step 2, narrow).** C1 step 2 is about the admission facts only (manageability, rule, AX kind,
native kind, minimize): one place, one writer, setters returning the transition. On this line
that was already true since L3 (the core's lifecycle holds them; the store's setters are their
only writers) except for two details, each its own commit: `invalidate_ax_identity` /
`validate_ax_identity` answer the core's `Invalidation` / `Revalidation` and the callers match
on them (2594b590; plants and two literal tests in the message: a shown event for an invalidated
window is a positive AX return, a rediscovered invalidated window resumes though natively
hidden), and two reactor tests that set a snapshot's minimized flag around the writer go through
`set_minimized` (77dfd124). Checked: no admission fact is written around the writers in rift
(`Effect::Seen` writes only the model frame, the title handler only the title); C1 step 3
(8207d229) and C2.1 (c173c69f) were done. `window_mut` stays: it still writes the title and
`Effect::Seen`'s model frame (the ruling: it goes only if left writing nothing but the title).
Ruled out: a universal `apply(WindowChange)` over the store's other writers (window server,
visibility, native fullscreen, rule floating, claims, the grip, topology); nothing upstream
asks for it. Topology stays its own writer on the core. The old doc comment's "false if it
already was" was wrong: invalidating an invalidated window answers `Invalidated` again.
Clippy with `roc-core` is 332 (the count above) on both sides of these commits.

**The core's layout path made linear (the lead's approved design, after roc-fork-7's diagnosis).**
Since the flip the core's frames are rift's cost, and the core's `place` recomputed each child's
subtree summary at every level: O(n x depth), quadratic for the deep chains BSP and traditional
trees grow (traditional is 333 deep at 1000 windows). Now `frames()` folds the shape once,
bottom-up, into a transient annotated tree of the same form (`Summarized { along: Along, node:
Annotated }`; `axis::Along` holds a subtree's summary along both axes, since a parent asks along
its own), and `place` (with `axis`, `stack` and the stack offset's cap) reads the children's
summaries from it. Nothing is kept across passes; the shape stays the only source; the recompute
path is gone. Same in Roc: `Summarized := { along : Along, node : Annotated }` inside `Bsp` and
`Traditional`, with the children inside `Split`/`Axis`/`Stack`. Summaries are bit-identical (the
upstream and Rust/Roc differentials pass unchanged). Plants (the annotated summary read along the
wrong axis): Rust BSP 2 lib failures (the BSP differential), Rust traditional 7 (the master-stack
differential and behaviour tests), Roc BSP 1 lib failure and the Roc crate's BSP differential, Roc
traditional 4. A pass at 1000 windows (release, `measure_layout_shadow`, the shadow skipped):

| | before | after | the system (the shadow) |
|---|---|---|---|
| BSP, Rust core | 13.3 ms | 175 us | 117 us |
| traditional, Rust core | 1.13 ms | 132 us | 84 us |
| BSP, `roc-core` (Rust and Roc cores) | 43.9 ms | 829 us | 115 us |
| traditional, `roc-core` | 6.91 ms | 554 us | 87 us |

At 100 windows BSP went from 114 us to 17 us (Rust core), 444 us to 88 us (`roc-core`). The rest
of a core pass over the system is mostly the shape builder (about 50 us at 1000 windows).

**The scrolling placement flipped (the last site, 37dafa4f).** `Geometry::build` takes its columns,
frames, resting points and bounds from `core_geometry::decide_strip(strip, shadow, ||
shadow_placement(..))`: the core's `Placed` (Roc under `roc-core`), with the system's own
placement (`shadow_placement`, upstream's `build`) the shadow, held word for word
(`StripPlacement::words`). Snaps name their column by id at the boundary and in `Geometry`
(`SnapPoint.column: ColumnId`); `settle` ties and walks by id, as the core's `past`, held by a
session-local differential against the old index-based settle (32,000 drawn releases, alike).
`check_strip` and the old `diverged` are gone; `decided` is generic (frames or a placement) and
`report_all` reports for both. Anchors are still computed by rift at reveal time
(`Geometry::anchor_offset`, upstream's) from the core's columns; the core's `Spot.anchor` is not
used. Plants in the commit message; four literal tests added (gaps between columns and rows, a
centred snap, the gesture tie-break, the niri walk both ways). The niri tie-break by offset (the
lead's ruling: settle it, delete or test): a layout tells it apart (free navigation lists the right
end second, before the interior edges, so a release midway between an interior edge and the right
end goes to the edge only by the tie-break); a literal case in
`a_niri_release_shows_the_furthest_column_in_view_its_way` now fails when it is dropped.

**The shadow, sampled (the lead's ruling, 2026-11-24; see the commit after 37dafa4f).** The core
decides in every build. `core_geometry::Shadow { Run, Skip }` is handed to every decision
(`decide_*(shape, shadow, || shadow_frames(..))`; the shadow is a closure, asked only on `Run`,
`Shadowed::{Ran, Skipped}`). The layout engine counts each Space's passes
(`LayoutEngine::shadow_passes: ShadowPasses`, in `calculate_layout_with_groups`, the reactor's
pass) and `SAMPLING` decides: tests and `roc-core` builds every pass, a plain release build every
32nd pass per Space (the first, then each 32nd, after the twins' `VIEW_EVERY`). Layout work outside
a Space's pass (IPC queries, drag previews, an edit's rebuild of a scrolling strip) runs the shadow
where every pass does and skips it in release (`Shadow::outside_a_pass`); so in release the
scrolling placement's shadow runs only when a sampled pass rebuilds the strip (a screen, gaps or
constraints change). Option if wanted: hold the cached placement to the shadow on each sampled
pass. `LayoutSystem::calculate_layout` and `prepare_layout` take the `Shadow`. `show-timing`
prints `core: layout divergences: N (shadows run: M)`. Cost per pass (release,
`measure_layout_shadow`, `#[ignore]`d in `systems.rs`: gaps, no constraints; pass with the shadow
skipped, the shadow's extra, its share):

| system | 10 windows | 100 windows | 1000 windows |
|---|---|---|---|
| stack | 1.08 us, +0.20 us (18%) | 1.76 us, +1.23 us (70%) | 13.1 us, +9.5 us (73%) |
| master-stack | 2.12 us, +0.67 us (31%) | 5.52 us, +5.86 us (106%) | 47.0 us, +54.7 us (116%) |
| traditional | 1.11 us, +1.06 us (95%) | 18.5 us, +9.4 us (51%) | 1.13 ms, +72 us (6%) |
| BSP | 1.72 us, +1.27 us (74%) | 114 us, +10.6 us (9%) | 13.3 ms, +131 us (1%) |
| floating | 0.40 us, +0.36 us (90%) | 2.16 us, +2.05 us (95%) | 19.2 us, +18.5 us (97%) |
| scrolling translation | 72 ns, +90 ns (125%) | 239 ns, +334 ns (140%) | 2.35 us, +4.39 us (187%) |
| scrolling placement (a rebuild) | 1.69 us, +1.41 us (83%) | 11.9 us, +8.9 us (75%) | 109 us, +77 us (70%) |

(Measured before the core's layout path was made linear, below; after it, the shadow is 63-67% of
a traditional or BSP pass at 100 and 1000 windows.) Far above the 5% that would make every pass
affordable. Sampled every 32nd pass, the
shadow adds at most about 6% of a pass (187% / 32).

**The layout flip (the lead's ruling, 2026-11-24, policy (a)): all seven sites done.** The core
decides the frames of stack (e19d50a8), master-stack (b0af4547), traditional (fc76dc2f), floating
(a0a85f58), BSP (b53a4f20) and scrolling's translation (1a3fb330): the Roc core's under `roc-core`,
else the Rust core's, in every build. Each system's `calculate_layout` (scrolling:
`translate_frames`) returns `core_geometry::decide_*(shape, &self.shadow_frames(..))`: the
system's own frames, now `shadow_frames` (`shadow_translation`), are the shadow. The decision is
the pure `decided(upstream, rust, roc) -> (frames, Vec<Divergence>)` (`System`, `RustCore`,
`RocFailed`: the Roc core failing to answer lets the Rust core decide) and one reporting edge,
`report_divergence`, that counts (`LAYOUT_DIVERGENCES`, printed by `rift-cli metrics show-timing`
as `core: layout divergences: N` in every build), then panics in tests or warns
`layout-divergence:` (a system) / `roc-divergence:` (the Rust core, or the Roc core's failure) in a
running Rift; the core's frames are used. The shape builders (`shape`, `tiles::shape`, `shown`)
and the conversions are built in every build. Each differential now holds the core to the
system's `shadow_frames`, not to `calculate_layout` (which is the core). The shadow is sampled
in a plain release build (above).
- Plants per system, run with the divergence panics on and then off (`report_divergence` and the
  old `diverged` both warn instead): a Rust-core bug fails behaviour tests with the panic off (rift
  follows the core); the same bug in the system's shadow path fails none (only the differential and
  `a_layout_divergence_is_counted_and_fails_tests`, which expects the panic); a Roc-core bug under
  `roc-core` fails behaviour tests with the panic off. Counts are in each commit message. Most
  systems had no behaviour test that saw their frames (the reactor's stack tests run the
  traditional system; most draws have no gaps and even splits), so each commit adds one literal
  test: `a_stack_reserves_its_bar_and_offsets_each_window`, `a_master_stack_layout_keeps_its_inner_gaps`,
  `a_new_floating_window_takes_most_of_the_screen_centred`,
  `a_resized_split_gives_its_first_window_the_larger_share`,
  `columns_out_of_view_are_parked_with_a_point_showing`; traditional's existing
  `toggling_stack_tabs_reserves_and_publishes_only_while_enabled` and
  `native_tabs_reserve_the_configured_edge_without_changing_stack_direction` caught its plant.
- Tooling (session-local, the pattern): a plant runner applies one replacement, runs `cargo test
  --lib` (with `--features roc-core` for Roc plants) with the panics on, then with both
  `#[cfg(test)] panic!` lines in `core_geometry` turned to warnings, lists the failures and
  restores the files.

**The Roc nightly bump (2026-11-24).** Pinned `nightly-2026-11-23-b5c77fc` (`roc/NIGHTLY`; the local
install's `current` and `safe` point at it, `src` is a roc-lang/roc checkout at b5c77fc for the glue
spec). Three commits: (1) the core takes and answers the platform's own types, since nightlies after
f25da45 never pass a plain value for a nominal one (93 `roc check` errors at b5c77fc); green on both
nightlies; see "Design we follow" for the shape (Boundary's `answer`/`entry` methods keep
`Asking` generic, `is_eq`/`to_hash` on a few types, each entry builds its own answer type);
(2) the pin; (3) the `S(m)` workaround goes: the entries' results are declared in the platform
module on `Model`, `Windows`, `Joined`, `Mains`, except `LifecycleStepped`, which roc-lang/roc#13363
keeps in `Boundary` (generic) until the next nightly (see "Unknown"). No Rust change; the glue's
names are unchanged. Checks: 1009 workspace tests (868 lib, 2 ignored), 877 lib with `roc-core`, the
Roc crate's 49, rift-core's 116, the fuzzer 2000 × 40 both ways, `cargo check --locked` both ways,
fmt, clippy with `roc-core` compiles (old warnings).

**Scrolling's translation (slice 5b, d37970b1; the lead's ruling, 2026-11-24).**
`scrolling::translate(frames, Camera { screen, tiling, offset, parking: Park | InPlace },
fullscreen)`: a fullscreen window's frame (the screen, or the tiling area), else its world frame
moved by `tiling.x - offset`, parked at the screen's nearer edge (`REVEAL` = 1 point showing) when
parking and wholly outside the tiling area, then rounded; upstream's `translate_frames` operation
for operation. `Scrolling.roc`'s `translate`, through `TranslateQuestion -> List(Frame)`.
`translate_frames` holds the core's frames to its own (`check_translation`; `shown` reads the
geometry, the camera's offset, parking and the fullscreen windows). The camera spring and its
sampling stay rift's. Differentials: against upstream (12,000 cameras at and 1e-9 past the parking
edges, fractional and half-point offsets; 38,896 frames, 18,869 parked) and Rust/Roc (20,000).
Plants: 7 Rust + 3 Roc, all caught. No difference.

**Scrolling's placement (slice 5a, 02ea31d2; the column order, 5f5c33bb).** The lead ruled
(2026-11-24) that the placement is checked where it is decided, `Geometry::build`, since
`calculate_layout` reads rift's camera; the translation follows as slice 5b. The differential
found one difference: upstream moves to the next column by `x += width + gap`, the core by
`(x + width) + gap` (one ulp, then every later column, frame, snap and bound); not a pre-#688 rule
(upstream's order predates #688), the core's own. Brought to upstream's order in Rust and Roc
with a comment at both sites (the lead's go), named regression
`a_columns_x_adds_its_width_and_gap_in_upstreams_order`. `strip` builds the core's `Strip` from
the arrangement as `build` reads it; `check_strip` holds `Strip::place` to `Geometry::build` word
for word (columns, world frames, resting points, bounds). Differential: 3000 systems through
every edit and drawn settings, 9,477 frames identical. Plants: 8 Rust + 1 Roc caught, one
equivalent.

**Layout slice 5, BSP (see the slice's commit).** `BspLayoutSystem::shape` builds the core's
`Bsp` pruned as `prepare_geometry` prunes: a leaf shows its participating member; a leaf with none
is left out, except an empty leaf while nothing in the system is left out; a split keeps one or two
children; `stacked` when the leaf holds a stack whoever takes part; the aspect ratio. `check_bsp`
in its `calculate_layout`. Differential: 3000 systems through every edit (every window stacked onto
one among them), 9,886 frames identical, no difference. Plants: 9 Rust (the stack one caught once
the draws stack everything onto one window), 1 Roc.

**Layout slice 4, floating (0fa71401).** The core gained a whole floating layout:
`floating::Layout { floating, gaps, windows: Vec<Slot> }`, `Slot::LeftOut | In { window, place }`,
`Place::Screen | WithinGaps | Kept(Rect) | First(WindowConstraints)`, and `Layout::frames` (a first
frame cascades by its position among all the layout's windows, left-out ones included, as
upstream's `calculate_layout` counts them). `Floating.roc`'s `frames` mirrors it, answered through
a new typed question (`FloatingLayout -> List(Frame)`; the slot tag is `Placed` because the glue
does not escape Rust's `in`). `FloatingLayoutSystem::shape` builds it (the nearest fullscreen
ancestor decides, else the kept frame, else a first frame); `check_floating` in its
`calculate_layout`. Differentials: against upstream (3000 systems, 9,315 frames) and Rust/Roc
(20,000 layouts). Plants: 7 Rust, 2 Roc, all caught. No difference.

**Layout slice 3, traditional (6e6f79e3).** `tiles::shape(root, None)`: the traditional system
lays its tree out from the root, with no regions. Check in its `calculate_layout`; differential
`the_core_tiles_traditional_layouts_as_the_traditional_system_does` (3000 systems through every
edit the system takes; 10,515 frames identical, no difference). Plants: within-gaps (1), weights
(3), the stack's top (1), stack orientation (2), axis orientation (3), the offset not handed on
(1), Roc's vertical stack clamp (1, `roc-core`); the fullscreen root is equivalent here.

**The core's geometry predated #688, and is brought up to it (the lead's ruling, 2026-11-23).**
The core's layout geometry was written against rift's systems before upstream's #688 rewrote them;
merge (b) took upstream's systems, so where #688 changed a rule the core was the stale side. The
master-stack differential found the first: the axis solver. Each rule moves to upstream's in its
own commit, Rust and Roc, held by a differential against upstream's own function, with plants.
rift's behaviour does not change (upstream's systems still decide).
- **The axis solver (a02886b5).** `rift_core::layout::axis::solve_axis_lengths` and
  `Geometry.roc`'s `solve`/`grow` are upstream's #688 solver step for step (growable shares
  clamped one side at a time, settling within the tolerance; the old drift settling went). On one
  axis 919 wide (maxima 105 and 1746, a minimum of 869, weights 1) upstream gives 25, 25, 869, the
  old core 105, 0, 814 (`a_minimum_that_fits_holds_while_maxima_share_the_rest`). Held by
  `the_cores_axis_solver_is_upstreams` (rift lib, against upstream's function) and, length for
  length, `the_roc_core_solves_axis_lengths_as_the_rust_core_does` through a new typed question
  (`AxisQuestion -> List(F64)`, `rift_core_roc::solve_axis_lengths`): frames round most solver
  differences away, so a frames-only differential missed a planted tolerance bug.
- **The weight fallback (6cebd282).** A traditional axis's weights are each read as an f64, made
  equal only when one is negative or not finite or they add up to nothing (upstream's
  `layout_axis`); the core made them equal when any was under 0.05 (f32). Held by
  `the_core_weighs_an_axis_as_upstream_does` (against upstream's `project`) and
  `small_positive_weights_stand_as_upstream_reads_them`. The shared builder `tiles::shape` landed
  with it.
- **The min/fixed leaf summary: not changed (the differential does not show it).** For a resizable
  window whose axis is locked within `f64::EPSILON`, upstream's leaf summary starts with `min` and
  the core's with `fixed` (the max). The draws include such axes (`systems::drawn`: a minimum
  under 1 with the maximum one ulp above it, 5%), and every frame matches: the difference is at
  most one ulp of a length under 1, which rounding to whole points removes. The lead's ruling:
  change it only if a differential shows it.

**Layout slice 2, master-stack.** `MasterStackLayoutSystem::shape` hands `tiles::shape` its root
and regions: `tiles::shape` builds the core's `Traditional` from rift's tiled tree as `project`
lays it out (the nodes whose subtree participates; from the root when the root is effectively
fullscreen, else from the master region when the stack region takes no space (`region_root`, which
`project_root` asks too), else the root; each node's fill `Screen` when fullscreen, `WithinGaps`
when within gaps, else `Tiled`; an axis's children with their weights; a stack's children with the
one `participating_child` picks on top). The check is `check_traditional("master-stack", ..)` in
its `calculate_layout`. Differential: `the_core_tiles_master_stack_layouts_as_the_master_stack_system_does`
(3000 systems drawn through the system's own edits: adds, removals, every focus, fullscreen of a
window or of the whole layout, participation, moves, drops of every kind, resizes, swaps,
rebalances, settings changes; 9,710 frames identical). Plants (lib tests failing): regions ignored
(1), the fullscreen root ignored (1, after the draws learnt to fullscreen the whole layout; 0
before), a node's within-gaps ignored (1), weights dropped (10), a stack's first child on top
instead of the selected (1), a stack's orientation ignored (1), an axis's orientation flipped (10),
the master-stack shape without its regions (1); in `Traditional.roc`, the axis gap dropped (2,
under `roc-core`).

**Layout slice 1, stack (aec5f920).** `StackLayoutSystem::shape` builds the core's
`Traditional` shape of a stack layout: one `Node::Stack` of the participating members (the
system's `window_participates`), the visible member (`visible_index`: the first participating one
when the root is selected) on top; a member's fill is `Screen` when fullscreen, else `WithinGaps`
when within gaps, else `Tiled`; the stack's is `Screen` when the stack is fullscreen, else `Tiled`
(upstream reads no within-gaps flag on the stack itself); no shape when no member participates.
`core_geometry` gains `core_constraints` (upstream's `Sizing` tag and min numbers into the core's
`Sizing` with its `Size`, parsed by `Size::reported`), `core_stack_line`, and `check_traditional`:
the core's frames held to the system's bit for bit (`f64::to_bits`), in tests and under `roc-core`
(Rust core, `layout-divergence:`), and under `roc-core` the Roc core's too (`roc-divergence:`). The
system's frames stand; tests panic, a running Rift warns. The check is called from the system's
`calculate_layout`, and lives in `core_geometry` (where the Roc layouts were checked before merge
(b)), not on `roc_path`. The Roc mirror is `Traditional.roc`, unchanged: the stack is a traditional
shape it already tiles, held to the Rust core by `the_roc_core_tiles_traditional_layouts_as_the_rust_core_does`.
Differential: `the_core_tiles_stack_layouts_as_the_stack_system_does` (3000 systems drawn through
the system's own edits, 12,493 frames identical). Plants (lib tests failing): member fullscreen
ignored (1), member within-gaps ignored (1), stack fullscreen ignored (1), the active member on top
instead of the visible (1), left-out members kept (5), orientation ignored (11), constraints
dropped (3), offset dropped (11), bar placement swapped (1), respect_min_size dropped (1), locked
size dropped (2); in `Traditional.roc`, the bar's spacing dropped (1, under `roc-core`).
**For the traditional slice:** upstream's `traditional_leaf_axis_constraints` answers a resizable
window's locked axis as `(min, Some(fixed), ..)`, the core's `leaf_summary` as `(fixed, Some(fixed),
..)`; `fixed` is the max, which may exceed the min by up to `f64::EPSILON`. A stack reads only
`fixed` there, so the stack slice cannot see it; a container's summary sums the first element, so
the traditional differential may. If it does, that is a reported difference, not a fix.

**Joined workspaces flipped (step 3, policy (a)).** `TwinnedWorkspaces::step` reports the Roc
core's outcome (a refusal is `workspace-refused:` as before), and the Rust `Workspaces` is its
shadow, read through `Deref`. An outcome that differs, or a view that differs when due (every
topology change; every ownership change in tests, every 32nd in a running Rift), warns
`roc-divergence:` (tests panic) and re-seeds the shadow from the Roc core's view:
`Workspaces::from_view` (new in the core) rebuilds it by stepping the view's catalog, activations
and owners, so the step's refusals hold a re-seed too, and the words go back to rift's types
(`workspace_of_word`, `window_of_word`, `Claimant::from_plain`, the inverse of step 2's
`Claimant::plain`). A view the shadow cannot hold leaves it as it was, warned. Counted
process-wide and printed by `rift-cli metrics show-timing` (`roc-core: workspaces re-seeds: N`).
Tests: `a_workspaces_reseed_makes_the_shadow_the_roc_cores_and_is_counted` (a claim lost by the
shadow comes back whole, flags and original id), `a_workspaces_shadow_out_of_step_panics_in_tests`,
`workspaces_from_a_view_hold_that_view_and_refuse_one_naming_what_it_lacks` (core). Plants (a
removed workspace's windows unassigned instead of handed to the heir; roc-core lib failures): in
`Workspaces.roc`, 5 by roc-divergence, and with the divergence panic off 4 behaviour tests fail on
what rift does (the reactor's remap onto a Space holding workspaces, the store's two remap tests,
and removing a Space): rift follows the Roc core; in the Rust shadow, 5 by roc-divergence, and with
the panic off none on behaviour (only the 4 tests that expect the panic). No added cost per step:
the twin already compared outcomes and views; a re-seed costs one Roc view and one rebuild. Also
`WorkspaceStore::rename_workspace` is `#[cfg(test)]`: nothing in rift calls it (making it
`pub(crate)` in step 1 showed it dead).

**Per-Space answers as core queries (step 1).** `rift_core::workspaces::Workspaces` answers
`space_of`, `is_on`, `assigned_on` (a window's workspace when it is on a Space) and
`resolve_named` (the assigned window a name picks out: command Space, then visible, then any; a
tie in the best scope is `Resolved::Ambiguous`, nothing `NotFound`). The view's `workspace_space`,
`workspace_info`, `last_focused_window`, `workspace_for_window` and `resolve_window_index` answer
through them, the writers `set_last_focused_window` and `rename_workspace` take the core (no
catalog slice, no `listed_on`), and the native reconcile asks `space_of`. No behaviour change;
`resolve_window_index` still answers `Option` at the platform edge (ambiguous and not found refuse
alike). Plants (rift-core / lib failures): `is_on` ignoring the Space (1 / 8), `assigned_on`
ignoring it (1 / 7), a visible Space ranked as the command Space (1 / 0), a tie kept as its first
window (1 / 0): the last two only the core's unit test catches, as before the move (nothing on
rift's path told those scopes apart). **Merge leftover, removed to match upstream (the lead's ruling):**
`MoveWindowToDisplay` with no window id, no main window and nothing under the cursor fell back to
`FirstOnCommandDisplay`, index 0 on the command display, which no window has (`WindowId.idx` is
non-zero), so it never moved anything. Merge (b)'s event-flow resolution (178c9657) introduced it,
putting our old `find_window_by_idx(space, 0)` fallback on #702's index resolution; upstream main
has no fallback and ignores the command. A first fix (f38cfa4e) moved the first window in layout
order instead; it is replaced by the removal (the fork matches upstream here): the command is
ignored with a warning, `WindowToMove::FirstOnCommandDisplay` and `LayoutEngine::first_window_on`
are gone. Regression test:
`moving_to_a_display_with_no_window_named_and_none_under_the_cursor_moves_nothing`; a planted
fallback (window index 1) fails it. Not an upstream candidate.

**Roc holds the whole claim (step 2).** `rift_core::workspaces::plain::Claimant { owner, flags:
ClaimFlags { focus_follows_mouse: FocusFollowsMouse::{Followed, Suppressed} }, original }` is the
plain owner of a claim (it replaces the bare owner word); Boundary.roc's `Claim := { owner : U64,
flags : ClaimFlags, original : U64, destination : Destination }` with `ClaimFlags := {
focus_follows_mouse : FocusFollowsMouse }` and `FocusFollowsMouse := [Followed, Suppressed]`;
Workspaces.roc keeps the whole claim and hands it on whole when its workspace is removed (`{
..claim, destination }`). The platform's `Claimant::plain` maps the flag bitset to the tag (a claim
holds only supported flags: `claim_window` refuses the rest), and `Word for Claimant` is gone. The
differential draws both flags and an original id, and both views (the differential's and rift's
path) compare the whole claim. Plants (Roc crate / roc-core lib failures, each a `roc-divergence`):
Roc dropping the flag as it records a claim (2 / 11), as it hands a claim on (2 / 1).

**Clippy fix (40a2b091).** `roc_realloc` and `roc_dealloc` (the Roc allocator's entries) failed
clippy's deny-by-default `not_unsafe_ptr_arg_deref` since a159f227 (before W4d). They are
`unsafe extern "C" fn`s with a `# Safety` contract; the C ABI and symbols are unchanged, and the
glue's host hooks stay safe function pointers that call them in an `unsafe` block.

**Slice 5, done (tests only).** What 2a and 2b left of it: each workspace's Space is written from
the core (2b part 2), and the runtime restore needs no join: it reads the file's `SavedTopology`
as data (which workspace each saved Space showed, the saved catalog) and writes the live core
through the workspace store's writers (`set_active_workspace`), so the file is never a second
holder. Saved files of every age keep loading (the 2a fixtures and the legacy tests, unchanged).
Nothing tested the restore's reads of the file's topology: plants of the restore never showing
the file's shown workspaces, reading the shown workspace as the Space's first, and the moves
report comparing a window's new Space with itself each failed 0 tests; with
`a_restore_shows_what_the_file_showed_and_reports_windows_it_moves_between_spaces` each fails 1.

**Slices 3 and 4, done by 2b (the lead's ruling (a)).** Every topology read already goes through
a `Workspaces<'a>` view built from the core, so nothing was left to move.

**Slice 2b part 2, done.** `VirtualWorkspace` has no `space`: the core's catalog is the one holder
of where each workspace is. The catalog is a `model::slot_table::SlotTable` (no longer test-only).
Loading parses the saved slot sequence (`SavedStore.workspaces: Vec<SavedSlot<SavedWorkspace>>`,
`SlotTable::parse`) and `Catalog::parse` splits each record into the workspace and its Space
(`SavedWorkspace::split`), the Spaces going only into `SavedTopology`'s catalog. Saving writes each
record with the core's Space (`SavedRecords`, `VirtualWorkspace::saved(space)`; the slot sequence
by `SlotTable::saved`), so files are byte-compatible (the 2a fixtures pass unchanged). Every
`space` read went to the view: `Workspaces::workspace_space`, and the per-Space answers moved onto
it (`workspace_info`, `last_focused_window`, `workspace_for_window`, `active_layout`,
`layout_spaces`, `resolve_window_index`, `eligible`); the two writers that checked a Space
(`set_last_focused_window`, `rename_workspace`) take the core's catalog slice (`listed_on`); the
window store's native reconcile reads the core's catalog it already holds (its workspace-store
argument went). `move_workspace` only tells the core. The tripwire retired whole
(`workspace_divergences`, `WorkspaceDivergence`, the reactor's catalog check; the
`workspace-divergence:` lines left are ownership's). `.expect("a listed workspace has a Space")`
stands where the code indexed the store before (same panic, one holder). Kept on purpose:
`Workspaces::topology()` reads the core (live validation and the quarantine question), and the
three test re-joins through it fork a fresh window store over the same core (fixture reuse, not a
second holder). No bare-store topology test is left: every topology read needs a view. New tests:
`a_saved_catalog_allocates_after_a_removal_as_the_store_it_was_saved_from` (300 drawn stores per
mode, removals among the draws: the next id after a load is the saving store's, the ids alike,
reused slots reached in both modes), `a_workspace_answers_only_on_the_space_it_is_listed_on`
(store), `a_window_on_an_inactive_space_is_not_missing_from_the_active_snapshot` (reactor).
Plants (lib tests failing): the load pairing every id with the first Space (9), the load
rebuilding the table without its vacant slots (2), the save writing every record on one Space
(7), a move not told to the core (34), `listed_on` ignoring the Space (0, then 1 with the store
test), `workspace_info` ignoring the Space (0, then 1), the native reconcile ignoring the active
Spaces (0, then 1 with the reactor test).

**Slice 2b part 1, done.** The workspace store keeps no catalog order and no activations: the
core holds them. Its topology reads live on `Workspaces<'a>` (virtual_workspace.rs: the store and
a `Windows<'a>`, `Copy`, deref to the store), built by `WorkspaceStore::reading(windows)`; reads
that took a `Windows` argument dropped it (`step_workspace`, `window_counts`, the
active-workspace window queries). Writers keep their shape (`&mut self` plus `&mut WindowsMut`,
which carries the core), read through the view and tell the core; no `WorkspacesMut` was needed.
`update_display_count` and `rename_workspace` take the core's catalog slice. Load-time code reads
`SavedTopology` (`ids`, `workspace_ids`, `active_workspace`): `normalize_loaded_names(&topology)`,
the runtime restore's saved side. A drag preview holds the window records mutably, so it reads
the shown workspace from the core alone (`WorkspaceStore::active_layout_in`, which the view's
`active_layout_for_space` also uses). About 25 engine and reactor functions gained a
`windows: Windows<'_>` argument (slice 3/4 territory, done here because the store could no
longer answer without the core); nine engine `pub fn`s went `pub(crate)` (they now take a
crate-private view); `collect_group_containers_in_selection_path` was dead and went;
`calculate_layout` is test-only. The tripwire's activation half retired (its catalog half went
in part 2). The overview
engine-test fixture initialised its Space through an unrelated window store (harmless only while
the store kept activations); fixed. Plants: `update_activation` not telling the core (4 lib tests
fail), `clear_activation` not telling it (9), `workspaces_on` in reverse catalog order (62), the
shown workspace read as the previous one (122), load-time names told apart in reverse catalog
order (0: a new test, `duplicate_saved_names_are_told_apart_in_catalog_order`, now 1).

**Slice 2a, done.** `SavedTopology` (virtual_workspace.rs): every workspace with its Space in
catalog order, and the activations. A saved store deserializes as `LoadedWorkspaces { store,
topology }`; `LoadedWorkspaces::checked` runs the load-time checks on the topology
(`SavedTopology::migrate_legacy_activation`, `SavedTopology::validate`, the same refusal
messages) and gives the store its copy of the activations (2a only). Loading answers
`StartingLayout { engine, topology }` (`LayoutEngine::deserialize_from_str`,
`load_for_startup_restore`; `StartingLayout::new` for a new engine, no topology);
`Stores::joining(StartingLayout)` seeds the core with `window_store::joined_core(&SavedTopology)`.
Saving writes `order` and `active_workspace_per_space` from the core
(`WorkspaceStore::saved(core)`, `SavedStoreRef`); `WorkspaceStore` itself no longer implements
`Serialize`/`Deserialize`. Tests: the drawn round trip
(`a_saved_topology_seeds_the_core_it_was_saved_from`: 300 seeds per mode, all six activation
shapes, the flagged stronger seeding check), the refusals
(`a_saved_topology_naming_what_it_cannot_hold_is_refused`: nothing caught skipping validation
before), and byte compatibility both ways through the old serializer's fixtures
(`persistence/fixtures/saved-before-2a-*.ron`,
`files_saved_before_the_topology_boundary_load_and_are_written_alike`; the activations compared
as a map, since a hash map's order is not the format).

**Next: slice 2b.** The workspace store's topology reads move onto `Workspaces<'a>` /
`WorkspacesMut<'a>` views (store + core, built from a split borrow of `Stores`; inside an engine
method that already holds a `WindowsMut`, build the view from it plus `&self.workspaces`). Its
`activations` map, the catalog's `order` and each `VirtualWorkspace`'s `space` leave the store;
the tripwire's topology half retires. Load-time code that reads the topology before the core
exists reads `SavedTopology` instead (`await_topology`, `normalize_loaded_names`,
`migrate_workspace_layouts`, the legacy floating positions, `resolve_locations`, the runtime
restore in `persistence/restore.rs`, which reads the file's `loaded.store` topology today). The
2a crutches go: `WorkspaceStore::topology()` (live validation and the tests' re-joins),
`LoadedWorkspaces::checked`'s copy into the store, and the two reactor tests that swap
`layout_engine` for a reloaded one in place (they keep the old core; a reload re-joins). The 43
store tests built on `WorkspaceStore::new*` move to a store+core fixture.
**Ruled for 2b (the lead, 2026-11-23): the catalog owns its saved format.** Each workspace's
`space` is written inside its record in the slotmap's serde format: a sequence of `(value,
version)` slots, vacant ones included, whose versions slotmap does not expose. To write `space`
from the core and keep files byte-compatible, the catalog tracks the vacant slots' versions (read
at load, set on remove) and writes the slot sequence itself. A differential test holds its writer
to slotmap's own over generated catalogs, removals and id reuse included. Ruled out: writing
vacant slots with version 0 (not compatible, and a reused id could collide with a removed one),
and a scoped lookup of Spaces during serialization (it hides a dependency).
**Ruled for 2b, loading (the lead, 2026-11-23): the catalog owns its slot table.** Reading the
vacant versions at load means parsing the slot sequence ourselves, and slotmap takes no parsed
slots back (no constructor; its free list, which decides the ids allocated after a load, is built
only by its own `Deserialize`). So the catalog keeps its own table, `model::slot_table`
(`Occupied { version, value } | Vacant { version }`), with slotmap 1.1's rules copied exactly:
a new slot at version 1, a vacant one taken at `version | 1`, a removal bumping the version by one
(wrapping) and pushing the slot on the free list, which a load rebuilds as slotmap does (the
highest vacant slot taken first), slot 0 a sentinel held at version 0; keys via
`KeyData::from_ffi`. The rules are small pure functions in their own module, apart from workspace
logic. The authority is the differential test (`the_table_allocates_and_writes_as_slotmap_does`:
500 generated runs of inserts, removals of live and stale ids, and saves each loaded by the other
side, comparing ids allocated, lookups, slot order and the bytes written against a real
`SlotMap`), with slotmap's refusals held alike. Plants: the removal's version bump (caught), the
free list taken first-in-first-out (caught), the loaded free list's order reversed (caught),
the sentinel's version kept as saved (caught), an occupied version with no value accepted
(caught).

**Not elsewhere in this doc yet:**
- The old twins' per-step costs in "Measured" are the separate topology and ownerships
  machines'; the joined twin's are in the W4b section (`measure_workspaces_twin`).
- `WindowStore::remove_workspace` (W4b) still re-sends each handed-on window's ownership after
  the core's `Removed` (now redundant: the core already holds it). It is kept because
  `remove_window_assignment` also clears a transfer and prunes the record, and
  `assign_window_to_workspace` updates `app_windows` and native fullscreen records; slice 2b or
  later can split those record-side effects from the ownership re-send.
- `OwnershipDivergence` now means "the claims index (or a missing record) disagrees with the
  core", not store-vs-core ownership (W4c); the reactor's `workspace-divergence:` lines keep
  their text.
- `WorkspaceStore::initialized` is the one definition of "a Space is initialised" (the IPC's
  `NotYetKnown` and the reactor tripwire both use it); keep it when slice 2b moves activations
  into the core.
- `SavedTopology::validate`'s "previous names what it shows" refusal is unreachable from a file:
  the legacy normalization before it clears such a history. Kept as it was.
- Helper scripts used for the plumbing lived in the session scratchpad and are gone; the pattern
  is in this doc (compiler JSON → insert the argument, or the field rename by span).

## Handover (2026-11-19): read this first

Workspace: jj workspace `rift-roc` (not colocated; jj only), bookmark `experiment/roc-2`. Every
commit below is on that line, in this order, after `ed797a2e` (the merge of upstream v0.8.5 +
#695). Docs-only commits (`docs: roc-fork.md, …`) sit between them and are left out of the list.

### Held commits, in play order

"Rust" changes rift's behaviour-bearing code (always with no behaviour change); "Roc" adds a
Roc mirror and its differential, inert unless the `roc-core` feature is on. Which of these
already reached the owner's play line is the lead's record; the line is linear, so a play batch
takes them in this order. Some commits marked Roc also refactor `crates/rift-core` (wire
encoders, plain questions, a pure step a later Rust commit builds on), so skipping one can
conflict; take the line in order unless the lead says otherwise.

| Commit | Kind | What |
|---|---|---|
| 6027249e | Rust | traditional: adding a window a layout already holds changes nothing |
| aba68bce | Rust | traditional: joining toward an adjacent stack joins it (upstream issue draft) |
| 57426401 | Rust (tests only) | tests never read a window's Spaces or ordering from the live window server |
| 91726e1b | Rust | step 3 #1: where an app inventory finds a window, pure |
| e2757007 | Rust | step 3 #1: which Space discovery may admit a window on, pure |
| f0fd51bd | Rust (tests only) | tests never read a Space's kind or a window's suitability live |
| da3e5ab8 | Rust | window store: one map of entries, one hand (one held window, structurally) |
| 4225e354 | Rust | step 3 #1: space snapshot admission, pure |
| e38c5551 | Rust | step 3 #1: app-rule candidates on activation, pure |
| 7a49c531 | Rust | window store: no grip on an unknown window; a re-press keeps the record |
| 7938584b | Rust + Roc | #1's decisions move to `rift_core::topology`; Roc tags 5–8 (later Rust commits depend on the move) |
| e5aeb256, 8d4f04bd, 968544ed, 9e5a8f6f, 15e3d902 | Rust | step 3 #2, window lifecycle decisions in `rift_core::lifecycle` |
| 7a76eb0a | Roc | #2 mirror, tags 9–16 |
| 1f24c75c, aec2fce0, fc6f3fc8, f887a6d4 | Rust | step 3 #3, focus decisions in `rift_core::focus` |
| ac7017a4 | Roc | #3 mirror, tags 17–23 |
| f0ea99d4 | Rust | L1: admission on typed facts |
| cf5c2042 | Roc | L1 mirror, tag 24 |
| 9715b121 | Rust | L2: lifecycle shadow + `lifecycle-divergence:` tripwire |
| 7e9e980d | Roc + core refactor | L2: the lifecycle step as the pure `read`; tag 25 (L3 builds on the refactor) |
| 5c2b0c50 | Rust | L3: the flip (admission facts live only in the core); held for the L2 soak |
| c173c69f | Rust | L4: AX invalidation as presence |
| fc3d474e | Roc + core refactor | L4: presence as the pure `present`; tag 26 |
| 95606e2e | Rust | L5: a window's ownership is one value on its record |
| 391ac12d | Rust | L6: retirement decided by the core |
| b151281a | Roc | L6 mirror, tag 27 |
| 8207d229 | Rust | no call site compares admission before/after; typed `Availability` |
| 06b22687 | Roc | `net` mirror, tag 28 |
| 1026e20a | Rust | W2a: `Home`, `RuleStanding` (saved form unchanged) |
| 6f757581 | Rust | W2b: `SpaceActivation = Showing {..} \| Vacant {..}`; DisplayDeparts/Returns fuzzer steps |
| 18144116 | Rust | W2c: one `Catalog` (saved form unchanged) |
| 3fb35b51 | Rust | W1: workspace command decisions in `rift_core::workspaces` |
| a9b59c3c | Roc | W1 mirror, tags 29–31 |
| da583e7f | Rust | W3: workspaces core shadow + `workspace-divergence:` tripwire |
| cce4745b | Roc + core refactor | `Topology::step` mirror, tag 32 |
| b536fbb8 | Rust | W3b: window ownership in a core shadow (`Ownerships`, held by the window store) + orphans, on the `workspace-divergence:` tripwire |
| e4ce0ba8 | Roc + core refactor | `Ownerships::step` + `orphans` mirror, tag 33 |
| 8a8709e8, 275fe2f6 | docs | remap gap logged, then closed |
| eb466e5d | Rust (tests only) | reactor test: a Space remapped onto one holding workspaces takes its windows and claims along |
| 202c536c | Rust | W4a: the joined `Workspaces` (catalog, activations, ownership; refusals; Removed carries its Successor) |
| 7300b552 | Roc + core refactor | W4a mirror, tag 34 |
| 45932e6e | Rust (tests only) | fuzzer setups and steps for the workspaces' topology (not drawn) |
| 8d1be68a | Rust | C1: a topology change no longer resends a parked window's unanswered write |
| 61750e77 | Rust | A: a display returning mid-drag leaves the held window where it is (Held) |
| db6fea40 | Rust + Roc | Held in the core's ownership, the tripwire and the Roc mirror |
| 09d4d4b7 | Rust | B: a vacant Space drops its history where it goes vacant (tripwire gone) |
| 9bfb5ce9 | Rust + Roc | C2: `Sync::InFlight` records its origin |
| 7f5688b0 | Rust (tests only) | the fuzzer draws the new setups and steps; activation-shape measurement |

Play-line tripwires (debug, target `rift_wm::actor::reactor`): `lifecycle-divergence:` (from
L2), `workspace-divergence:` (from W3), `workspace-activation:` (W2b runtime drop),
`workspace-load: normalized` (W2b load normalization, warn). L3 and later wait for the L2
soak (zero `lifecycle-divergence:` lines on the owner's machine); the lead decides when they
ship.

### Open items

0. **Candidate simplification (not done):** `reapplies_on_title_change`'s check that the
   window's Space is active is mirrored by `rule_targets`, which drops windows on inactive
   Spaces; ignoring it in the Rust decision changes nothing visible (S3 found it; the reactor
   test `a_title_change_reapplies_rules_only_on_an_active_space` reaches the branch).

1. **W3b's shadow** was `rift_core::workspaces::Ownerships`, held by the window store and
   written only through `set_ownership`; W4b joined it with `Topology` (both retired).
2. **W4, the flip (plan approved).** The joined `rift_core::workspaces::Workspaces` (W4a,
   202c536c) holds the catalog, activations and ownership; its step refuses (typed `Refused`, no
   change) a workspace created twice, anything naming a workspace the catalog lacks, a workspace
   succeeding itself, and removing a workspace a Space still shows; `Removed { successor:
   Into(K) | Unassign }` hands windows and claims on and drops the workspace from every Space's
   history and memory in the same step (the W4 obligation). Rulings: it lives in the window
   store (placement A); a refused event is a bug: `workspace-refused:` at **warn** in a running
   Rift, a panic in tests. **W4b and W4c waited for #688 too** (ruled after W4a; #688 is merged, W4b is done): the workspace
   store's topology writers sit under about 100 call sites, many in engine glue #688 rewrites,
   and a saved workspace store loads without a window store. Plumbing `&mut WindowStore` through
   them (A1) would churn that glue twice. A shared `Rc<RefCell<Workspaces>>` handle in both
   stores (A2) was rejected: an unlinked window store is a representable, meaningless state; the
   one link call is a convention, not a type; and `RefCell` turns aliasing mistakes into runtime
   panics. After #688 the joined value gets one real owner that lends `&mut` to both stores. W4a
   stays done and held. Projections stay platform-side, keyed by workspace: name, home, rule
   standing, layout state; a claim's flags, original id and the claims-by-owner index (its order
   is observable, checked by the tripwire); native fullscreen records' workspace (a third
   reference: the platform applies the same Successor, the tripwire checks it). Slices (all after #688, with W6): W4b one
   joined shadow (remap_space computes each deleted workspace's successor before removal and
   clears a shown Space before removing; tags 32 and 33 retire), W4c ownership flip (the window
   store answers from the core; its index and the record's assignment go). **W4d, the catalog
   and activation flip with the saved form written from the core, waits for #688 and goes with
   W6: no persistence change before #688.**
3. **W5 fuzzer gap: done.** Why no departure transferred: the test reactor is **per
   display**, not global (an earlier note was wrong), and `reconcile_workspace_placements`
   returns early outside global mode. The fuzzer now has setups (`Workspaces::PerDisplay |
   Global | GlobalRuled`, half the seeds per display; the rule pins workspace 2 to display 1),
   steps (`DesktopReplaced` with its remap, `DisplayReturns { renumbered }`, `SwitchWorkspace`)
   and sim corrections (stable display UUIDs; `DestroyIdentity` skips the held window). What it
   found, each fixed in its own commit with literal tests from the shrunk cases:
   - **A** (61750e77, then db6fea40 for the core): a display returning mid-drag carried the held
     window off with its workspace. Only when a transfer would carry it, the held window leaves
     the workspace as `Ownership::Held` (placed by no workspace; the store refuses to assign
     it); the drop places it on the drop display's shown workspace, a let-go without a drop on
     its Space's. `held_member -> Option<HeldMember>` (the index can't hold a Held window).
     **Upstream candidate:** main has the same planner and transfer loop.
   - **B** (09d4d4b7): a switch then a departure left history on a vacant Space, which W2b's
     tripwire panicked on. Pre-W2b the next activation cleared it, so it was never read; repair
     now drops it where the Space goes vacant. **Lesson: W2b's shape measurement ran without a
     global-mode fuzzer; a shape measurement only counts across every mode.**
   - **C1** (8d1be68a): a topology change Forgot parked windows, dropping a write in flight;
     now Distrust.
   - **C2** (9bfb5ce9): `Sync::InFlight` records its origin (`Written | AnimationEnded`);
     `animation_stopped` touches only an animation's. **After #688 (reactor/animation.rs is in
     its file list): the platform presents only animations the core emitted** (it presented a
     still animation under a plain write's request, a projection without an effect).
   Activation shapes (`activation_shapes_across_the_fuzzer`, ignored; 6000 × 60, after every
   step): showing, showing+previous, vacant+remembered (and, on other runs, vacant) in global
   modes; showing and showing+previous per display. A renumbered return carries its remap, and
   the held-window invariant follows a remap (a rename, not a move). Never showing+remembered: covered by the store-level literal
   test only (no natural step leaves a workspace behind on a departed Space).
4. **Later:** W4 (flip catalog + activation to the core), W5 (topology into the core), W6
   (engine glue and persistence; collides with upstream #688, waits for it). BSP phase C waits
   for the soak; traditional trees wait for #688.

### Next steps

Done: tags 32 and 33, W3b, the remap reactor test, W4a and its Roc mirror (tag 34), the W5
fuzzer gap and its four fixes (open item 3). W4b to W4d wait for #688 (open item 2).

**Done (2026-11-20): the Roc mirrors on rift's path under `roc-core`, S1 to S3, then the
boundary typed.**
- Each call site asks the Roc core the question it just answered in Rust and compares the
  answers (`src/roc_path.rs::check`): equal, the Rust answer stands; different, or the Roc
  decision read a fact the Rust one did not ask, tests panic `roc-divergence:` and a running
  Rift warns it and keeps the Rust answer.
- A question carries exactly the facts the Rust decision asked: each fact is a
  `rift_core::plain::Fact` (`Asked(answer) | Unasked`, or `Answers` about keys), recorded at the
  site through `roc_path::Asked` / `AskedAbout` (zero-sized without `roc-core`). The Roc
  decisions read facts lazily in the Rust order; reading an unasked one is a divergence by
  name. The differentials run each decision on full facts and with facts left unasked at
  random (`tests/facts`).
- S1 focus (7 sites, 9b058515), S2 topology (4, 57746016), S3 lifecycle (12, 51723ea1), each
  with one planted bug per decision proven caught (new reactor tests where a branch had no
  coverage: a display mid-change, a placeholder window, a title change on an inactive Space,
  a reading that admits and suspends).
- The boundary is typed (owner's decision, after S3): every entry takes and returns named
  `Boundary.roc` types, the glue generated at build time; the word codec is only the trace
  format (1b1acb1d, 1c733288, c98b1eb1, 1260394b, 99d903a6, 53098cb3, 1f0069d4, cf9f84d0).

- S4 workspaces (9cb2018a facts and an `eligible` entry, 0710152c the 4 VWM sites: eligible,
  previous, resolve, step; engine.rs only calls the wrappers, so #688's ground is untouched).
- The reconciler's ack tolerance has a literal differential at its edge (the committed traces
  alone let 1.5 to 2.5 through).

- The state machines kept between events have Roc twins on rift's path (`src/roc_twin.rs`):
  one platform, one archive, a model each (`requires` takes several model variables; the
  spike showed it), each box a type of its own on the Rust side. The lifecycle (19b8c59c,
  872e9fd0), the ownerships (02bda0d7, c6446dfe), the topology (fc1050e9, 215cb864). A twinned
  machine reads as its Rust machine (`Deref`) and only its own methods change it, so no change
  reaches the Rust machine without the twin. Outcomes are compared every step; the whole state
  every step in tests, and in a running Rift every 32nd step (lifecycle, ownerships:
  `VIEW_EVERY`) or every step (the topology, which is small and changes rarely). Each has a
  differential on persistent models and planted bugs caught by the regular checks.

- A twin's box is stepped by value: each step takes the box and gives back its successor
  (`boundary::in_place` puts it back in the owner, aborting on a panic in between), so a box
  that has been stepped cannot be stepped again.
- The lifecycle's invalidate and revalidate answer named tags at the boundary
  (`[Invalidated, NoAxState]`, `[Revalidated, NotInvalidated]`). **Candidate for the Rust
  core too:** `Lifecycle::invalidate` and `revalidate` answer the same as bare bools, and
  `PresenceOutcome::Invalidated(bool)` / `Revalidated(bool)` carry them; the twin maps the
  bools to the tags to compare.

- Who places each window is a `Dict` keyed by window in Roc (the questions' and the twin's
  alike), so **a window with two owners cannot be built**: there is no duplicate entry to
  plant, which is why the ownerships' plants are a held window misreported and owners left out
  of the view. Views sort by window (`Workspaces.in_order`) where a stable order is needed.
  The step is flat: 160-200 ns at 10 to 1000 windows (a sorted list rebuilt per change was
  153 ns, 641 ns, 7 us at 10, 100, 1000).

**Upstream #688 / #700 (2026-11-20).** Upstream main rewrote the layout systems (#688, with
#698's drag and reconciliation fixes) and the reactor's event flow (#700). Merging it into this
line is three parts, each the lead's ruling:
- (a) layout: upstream's systems. Built as its own tested line, `experiment/upstream-layout`
  on upstream main, with what main still lacks re-landed red-first: keep-constraints, the
  scrolling camera that ends before arriving, and `respect_min_size` (the owner's config uses
  it). There `Sizing` is a tag (Fixed | Resizable | ResizableAtLeastMin) beside upstream's
  min/max/locked numbers, not the core's variants that carry the minimum. Dead with (a): BSP
  phase B and the core's BSP trees, the traditional core shadow, and the traditional fixes
  (still bugs upstream; their tests are kept).
- (b) the event flow and (c) reconciliation and drag: (b) and the lifecycle flip wait for
  upstream PR #702 (window admission centralised in the reactor, with flags); (c) keeps the core
  reconciler and grip.
- **Upstream PR #678 (feat/respect-min-size) is based on pre-#688 code.** That branch is the
  owner's to decide about; nothing here touches it.

- (c) reconciliation and drag, against upstream #698's behaviours (2026-11-20). Already ours,
  held by tests (76091292): the drag owns its window, releases at startup, a window created
  mid-drag tiled at the release, superseded and redundant writes skipped. Changed by the lead's
  rulings:
  - 7 (5c3fbea9): a refused write leaves the model at the app's frame (`Seen { got }`); the
    desire stays Refused, not asked again.
  - 8 (10428f8c): `WriteFailed { window, failed }`, a write that never reached its app (the app
    actor reports a stale authority, a failed `set_frame`, a missing window, the rest of an
    aborted batch). `Sync::Failed` with `Failures = Once | Twice`: rewritten at the next
    arrange, given up as refused at the third (Note::GaveUp), the model then back at what the
    app last acknowledged. A timeout is unchanged.
  - 9 (8a12614e): a floating window dropped on another display stays floating there, at its
    release frame.
- The mode table is the core's (`rift_core::modes`, 76f180a7), mirrored in Roc exhaustively
  (24af76b4) and on rift's path (bc0a0dc2).
- **Re-recording the committed traces.** A reviewed behaviour change of the core changes the
  effects the traces recorded. `rerecord_the_committed_traces_effects` (ignored, in
  `tests/differential.rs`) keeps every event and re-records the effects through the Rust core,
  refusing any changed step the reviewed change does not explain (its `explained` closure is
  rewritten for each change). Regenerating the corpus from today's fuzzer is not the same thing:
  that records different runs.

**Merge (b) done (2026-11-21).** Upstream main (#688 layout, #700 event outcome, #702 window
admission) and `experiment/upstream-layout` are merged into this line: one three-parent merge, then
1/4 layout (upstream's systems with the three re-lands), 2/4 event flow and admission, 4/4 tests (3/4,
reconciliation and drag, is ours wholesale and needed no resolution of its own). Every adoption and
difference, instance by instance, is in [roc-fork-merge-b.md](roc-fork-merge-b.md).
- Handlers stay pure and return `EventOutcome` (the lead's ruling: no convergence on #700's
  `&mut Changes`); `changes.rs` is not adopted. The outcome gains `memberships`, `retirements` and
  `reprocess_app_rules` for the work #702 routes through its reactor methods.
- `window_lifecycle.rs` keeps #702's names and module: `reconcile_window_membership`,
  `retire_window`, `replace_window_identity`, `retire_app_windows`, `invalidate_window_identity`,
  `restore_native_fullscreen`, `reconcile_native_presence`, `reassign_window_to_authoritative_space`,
  backed by the core's lifecycle and ownerships through the window store. #702's handler-level
  `observe_*` names live on as comments on our pure handlers.
- Minimized is the core's `Minimize` fact (never a write to manageability); a workspace's manual
  display choice is `PlacedBy` inside the placed `Home` variants.
- D4 rulings: upstream's on both (no discard of unmatched persisted windows when an app's discovery
  completes; a focus command arranges only where focus changes geometry).
- Two merge bugs the fuzzer and the tests found, fixed in 2/4: a cross-display drop's queued removal
  cleared the window's new assignment; an added window arranged once per window.
- Plants after the merge: all 27 roc_path sites reached by tests; behaviour plants caught at the
  retirement verdict, discovery's net (both the dropped and the uncancelled change) and the lifecycle twin.

**Made obsolete by #702/#700 on our side:** `WindowStore::reconcile_admission`; `observe_minimized`
(it wrote manageability); `NativeDeparture`'s `remove_projection`; `WindowStore::retire_window`
(retirement goes through `RiftState` so a grip ends via the core); our `WindowDiscoveryCompleted`
emission; `find_window_by_idx` (now `resolve_window_index`, which refuses an ambiguous index); the
seeding expectations (`seed_space` is gone); `bsp_saved.rs`; the old discovery visibility filter; the
arrange on any focus change; `AppRuleEffects.names_workspace` (one source: our placement question).

**Lifecycle flipped (2026-11-22, policy (a)).** Under `roc-core` the Roc core decides the
lifecycle: `TwinnedLifecycle`'s step, invalidate and revalidate return the Roc core's answers, and
the Rust `Lifecycle` is its shadow (read through `Deref`). Any difference (an outcome, or the views
compared every step in tests and every 32nd in a running Rift) warns `roc-divergence:` (tests panic)
and re-seeds the shadow from the Roc core's windows (`Lifecycle::from_presences`), counted
process-wide and printed by `rift-cli metrics show-timing` (`roc-core: lifecycle re-seeds: N`). The
Rust core's `invalidate`/`revalidate` answer named tags (`Invalidation`, `Revalidation`) like the
boundary. Plants from both sides with the flip on (a revalidation that leaves the window invalidated,
in the Rust shadow and in `Lifecycle.roc`): 6 failures each, all roc-divergence. Cost per flipped
step (release): shadow + Roc step about 240 ns at 10 windows, 256 at 100, 280 at 1000; the views every
32nd step add about 13, 38, 190 ns a step; a re-seed costs about one Roc view.

**MainWindowTracker twinned (2026-11-23, not flipped).** Its fields typed (`NativeFocus`:
NotYetReported | Reported(Option), `Standing`, the latest activation's `Initiator`), then the machine moved
to the core (`rift_core::main_window::MainWindows`, events in, the window to focus out), with #702's
main-window clearing as its own event (`Replaced { pid, window, replacement }`: a retired window clears
the main window and the native focus, a rekeyed one moves them). `MainWindow.roc` mirrors it on a fifth
platform model (`Mains`), with a differential (1,000 seeded runs of 60 steps, answers and the whole
view every step; three plants caught). On rift's path `TwinnedMainWindows` compares every answer and,
in tests every step, the whole state; plants caught from both sides (85 and 23 failures).

**Main window flipped (2026-11-23, policy (a)).** As the lifecycle: the Roc core's answer is rift's,
any difference warns `roc-divergence:` (tests panic) and re-seeds the Rust shadow from the Roc core's
view (`MainWindows::from_parts`), counted and printed by `rift-cli metrics show-timing` (`roc-core:
main-window re-seeds: N`). Plant check: `MainWindow.roc`'s Replaced leaving the main window fails 23
tests by roc-divergence, and with the divergence panic off (behaviour only) the reactor test
`a_destroyed_main_window_is_no_longer_the_main_window` fails on what rift does.

**The four reactor predicates on rift's path (2026-11-23, not flipped).** Each is a pure core
function over typed facts, a Roc mirror behind a named boundary type, a `roc_path::check` site (the
Rust answer stands), a differential on full facts and with facts left unasked at random (20,000
questions each way, no Roc allocation left live), and a planted Roc bug caught:
- 1f22b0eb `rift_core::workspaces::emits_window` (discovery's emit check; the Space's active
  workspace is now asked only for an assigned window). Plant: emitting every assigned window fails
  18 roc-core lib tests.
- 919d300b `rift_core::topology::quarantines` (an unsettled topology holds back window reports and
  window input; `EventClass`). Plant: letting window input through was caught by nothing; the new
  reactor test `a_press_during_an_unsettled_topology_is_quarantined` (written first) catches it (1
  failure).
- c42e0370 `rift_core::topology::target_display` (MoveWindowToDisplay / MoveWorkspaceToDisplay;
  `NoTarget`, `plain::Point`). Plant: taking an inactive Space as the target was caught by nothing;
  the new reactor test `a_workspace_is_not_moved_to_a_display_whose_space_is_inactive` catches it
  (1 failure).
- 7ef619cb `rift_core::focus::mouse_raise` (focus-follows-mouse raise through `RaiseFacts`, asked in
  the Rust order; `MouseRaise`, `Unraised`; `raise_buries_a_floater` and `contains` move into the
  core on plain `Rect`, the same comparisons as `CGRectExt::contains_rect`). Plant: a floating,
  unadmitted window treated as outside admission was caught by nothing; the new reactor test
  `mouse_raises_a_floating_window_whose_admission_is_suspended` catches it (1 failure).

**W4b, one joined shadow (2026-11-23, not flipped).**
- d4494458 (Roc): the joined workspaces kept in Roc between events (`RocWorkspaces`, a model
  `Joined`; `roc_workspaces_init/_step/_view/_release`), the step answering what each event met.
  Differential on persistent models: 24,000 steps identical (6,284 refused, 854 removals with
  owners); three plants caught.
- bb610794 (Rust): the window store holds the core's joined `Workspaces` (`TwinnedWorkspaces`)
  in place of the ownerships and topology twins. Ownership reaches it through `set_ownership`;
  the workspace store's topology writers take `&mut WindowStore` and tell it through
  `WindowStore::topology` (topology changes only). A refused change is `workspace-refused:` (a
  panic in tests, a warning in a running Rift). A removal is one call,
  `WindowStore::remove_workspace(id, successor)`: the core hands windows and claims on and the
  records follow what it decided (native fullscreen records take the same successor).
  `remap_space` computes each heir before anything moves; `remove_space` clears the Space and
  drops other Spaces' history and memory of the removed workspaces before removing them
  (`removing_a_space_clears_it_first_and_ends_where_removing_first_did`: 500 drawn stores, same
  end state as the old order). A loaded workspace store goes live only joined: `Stores::joining`
  builds the window store from the whole catalog and activations; `Reactor::new`/`spawn` take
  `Stores`. Plants: removing before clearing (6, workspace-refused), handing to nobody in
  `remap_space` (3 behaviour tests), the Rust core sending claims nowhere (2), `Workspaces.roc`
  dropping heirs under roc-core (4, roc-divergence).
- a159f227 retires tags 32 and 33 and the separate topology and ownerships machines (Rust
  `Topology`, `Ownerships`, `orphans`; the Roc machines `Owners`/`Places` and their entries);
  `measure_workspaces_twin` replaces their measurements.
- Cost (release, `measure_workspaces_twin`): Roc step about 100-180 ns per ownership change and
  190-240 ns per topology change at 10 to 1000 windows (Rust 20-140 ns); the whole view 1, 5 and
  46 us at 10, 100 and 1000 windows, compared after every topology change (rare: switches,
  moves) and every 32nd ownership change in a running Rift.

**No getter initialises a Space (2026-11-23, the lead's ruling A).** A Space is initialised by
the events that bring it: its exposure (`ensure_visible_spaces`) and a window placed on it (app
rule placement, a window or workspace moved onto it). `list_workspaces`, `workspace_id_at` and
`get_default_workspace` take `&self`; the list answers `Listing::Known(..) | NotYetKnown`, and the
IPC state query (`serialize_state`, now `&self`) warns `workspace-unknown:` on `NotYetKnown`.
`WorkspaceStore::initialized` names the state: showing a workspace, or (global mode) showing
nothing with no workspace of its own and others to show. Proofs: a property test
(`every_space_ensure_visible_spaces_exposes_is_initialised`, 400 stores, both modes, displays
coming and going) and a test-time tripwire after every reactor event (every active screen's
Space initialised); a planted `initialized` fails 401 lib tests. The property test found the one
gap, and ruling A closed it: in global mode with more displays than workspaces, the first
exposure left a display without a workspace with no activation at all; the first IPC query (the
menu bar, `rift-cli`) then wrote `Vacant { remembered: None }` as a side effect. Now the exposure
writes it. `remap_and_the_saved_store_do_not_depend_on_a_query_having_run` shows the remap and
the saved store are the same with and without a query first (without the exposure fix, it and
the property test fail).
- **Upstream candidate (a small standalone PR against main):** a query's side effect changed
  remap and the saved file. Upstream's workspace query initialises Spaces lazily; in global
  mode with more displays than workspaces, whether a vacant display had an activation (so
  whether `remap_space` carried it along, and what the saved layout held) depended on whether a
  query had run first.

**W4c, ownership from the core (2026-11-23).** The window store keeps no ownership of its own:
a record's ownership field and the per-workspace index are gone, and the store answers who
places a window (`workspace_for_window`, `claim`, `workspace_windows`, the assignment iteration
and counts) from the core's joined `Workspaces`; `set_ownership` only tells the core. A record
stays while the core owns its window, as it did while the record held the ownership.
`workspace_windows` keeps its order: the core's per-workspace set is ordered by `WindowId`,
whose derived order is the (pid, index) the store sorted by. **A deviation from the W4 plan,
decided here, accepted by the lead:** a claim's flags and original id are not a platform-side
projection beside the core (a second copy that must agree with the core's "claimed"); they ride
in the core's opaque owner, `Claimant { owner, flags, original }`, so the core holds the whole
claim. Only the owner
takes part in a decision, and the Roc twin sees only the owner's word. What stays platform-side
is the claims-by-owner index (its order is the owner's); the `workspace-divergence:` check now
compares it with the core's claimed windows, and checks every owned window has a record.
Plants: the core's index keeping a reassigned window (13 lib tests fail), a held window read as
unassigned (3), a claim left in the index when it ends (20, the fuzzer among them).

**Next:**
1. W4d with W6: catalog and activations answered from the core, the engine glue on it, and the
   saved form written from it (plan below).

### W4d + W6 plan (the lead's ruling: one owner, `Stores`)

Goal: `Stores` is the one owner of the layout engine, the window store and the joined core
(`TwinnedWorkspaces`). Neither store holds the core; whatever needs the workspaces' topology or
ownership gets it from `Stores`, explicitly (a split borrow, never a shared handle). The
workspace store keeps only what is keyed by workspace and is not the core's: name, home, rule
standing, layout state, last focus. Today: the core lives in the window store (W4b/W4c), and the
workspace store still holds the catalog order, each workspace's Space and every activation,
which the core shadows. About 300 reads of that topology (`active_workspace`, `catalog`,
`workspace_ids`, `workspaces_on`, `workspace_space`, `active_workspace_per_space`, ...): the
workspace store itself 109 (tests among them), the engine 46, persistence 22, the reactor 45
(commands, gestures, the IPC query, drag, discovery, lifecycle), reactor tests 77.

Every slice is green (the full check set, with and without `roc-core`) and published on its own.
A slice that needs a behaviour change stops and is reported.

- **Slice 1, mechanical: one owner.** The reactor's `state: RiftState` and
  `layout_manager.layout_engine` become one `stores: Stores` (the layout engine, the window
  store, the reconciler and the pending store, and the joined core moved out of the window
  store). The window store's ownership reads and writes and the workspace store's topology
  writers take the core as an argument from a `Stores` split borrow instead of reaching it
  through `WindowStore`; `Stores::joining` stays the only way to build them. Renames and
  arguments only: no read changes source, no behaviour changes. The tripwire is unchanged.
- **Slice 1 done (2026-11-23).** 1a (b19cf57b): the reactor holds one `Stores` (the layout
  manager and the `RiftState`), renames only. 1b (807cbcbf, the lead's ruling (d)): the core is
  no longer in the window store; `RiftState` holds it beside the window records and lends the
  two as borrowed views that hold exactly those two, `Windows<'a>` (read, `Copy`) and
  `WindowsMut<'a>` (write), built by `RiftState::windows()` / `windows_mut()` from a split
  borrow. The 58 core-dependent window-store methods live on the views (24 read, 34 write; both
  views deref to the records, so record-only methods stay reachable); engine, workspace-store
  and reactor-helper functions that took the window store take a view, never the whole state.
  `window_store::joined_core` seeds the core from a workspace store (`Stores::joining`, and the
  tests' `OwnedWindows` fixture). Handlers that needed more than an added argument: none beyond
  binding the view to a local (the workspace rehome loop in reactor.rs, one window-store test).
  23 functions that take the crate-private views went from `pub` to `pub(crate)` (nothing
  outside the crate used them). A plant dropping one activation from the seeding fails the
  load-agreement test.
- **Slice 2a, the persistence boundary first (the lead's ruling (A), done 2026-11-23).** The
  saved topology is parsed into `SavedTopology` at the load boundary; the load-time checks run on
  it; `Stores::joining(StartingLayout)` seeds the core from it; the save writes the catalog order
  and the activations from the core. Saved files load and are written alike both ways (the old
  serializer's fixtures). The strong round trip flagged for slice 5 is here. The store keeps its
  copy (the tripwire stays) until 2b. Tests first: 0fc40f84; the change: edb6dbca.
- **Slice 2b, the workspace store's own reads.** The workspace store answers its topology reads
  (`active_workspace`, `workspace_space`, `workspaces_on`, catalog order, the activations) from
  the core it is lent, through `Workspaces<'a>` / `WorkspacesMut<'a>`; its `activations` map,
  catalog order and each workspace's `space` go. Writers still write the store's per-workspace
  data and tell the core. Its tests move with it, onto a store+core fixture. Load-time code reads
  `SavedTopology`. The tripwire's topology half retires (one holder). Done 2026-11-23: part 1
  (4d3afda4) the catalog order and activations, part 2 each workspace's Space and the catalog on
  the slot table (Handoff).
- **Slice 3, the engine glue (W6):** the layout engine's workspace operations (activation,
  transfer, remap, moving windows and workspaces between Spaces, focus by workspace) take the
  topology from `Stores`. Done by 2b: every topology read goes through a view built from the core.
- **Slice 4, the reactor:** commands, gestures, the IPC query, drag, discovery and lifecycle
  read the topology through `Stores`. Done by 2b, as slice 3.
- **Slice 5, persistence (W6), shrunk by 2a:** what slices 2b to 4 leave of the saved form (each
  workspace's Space written from the core if 2b has not, the runtime restore from a file joining
  its topology), with the files saved before kept loading (parsed at the load boundary,
  normalized with one `workspace-load: normalized` line where W2b did) and the 2a fixtures still
  passing. Done 2026-11-23, tests only: the runtime restore reads the file's topology as data and
  writes the live core through the writers, so it needs no join (Handoff).
- **Then:** the Roc twin decides (policy (a), as the lifecycle and main window): done
  2026-11-23 (the per-Space queries, the whole claim in Roc, the flip; Handoff). Next, core
  geometry on upstream's layout systems (Next item 2).
2. Core geometry on upstream's five layout systems (after W4d+W6, since the layout engine glue
   moves there): merge (b) took upstream's layout systems, and the shape builders that fed the
   core's geometry died with ours, so the converters were dead code and are deleted (the lead's
   ruling; the core's layout modules and their Rust/Roc differentials stay). One slice per
   system (BSP, traditional, master-stack and stack, scrolling, floating): a builder from
   upstream's tree to the core's shape, and a bit-for-bit differential of the core's frames
   against upstream's `calculate_layout`; a difference is a behaviour question, stopped and
   reported.
   Done: stack (aec5f920), the core's solver and weights brought up to #688 (a02886b5, 6cebd282),
   master-stack, traditional, floating, BSP, scrolling's placement and translation (Handoff). The
   series is complete.
3. The Roc nightly bump: done 2026-11-24 (Handoff). The core speaks the platform's types (green
   on f25da45 and b5c77fc), then the pin moved to nightly-2026-11-23-b5c77fc, then the `S(m)`
   workaround went (all but `LifecycleStepped`: roc-lang/roc#13363).

- The window store's single writer (the upstream PR plan's C1 step 2): done 2026-11-24, admission
  facts only; `invalidate_ax_identity` / `validate_ax_identity` answer `Invalidation` /
  `Revalidation` (Handoff).
- `LifecycleStepped` moves into the platform module beside the other entries' results on the first
  nightly with roc-lang/roc#13363's fix (3ef41a6).

No call site is in `layout_engine/`; keep it that way.

### Rulings that bind (the lead's and the owner's)

- **Design:** make impossible states impossible; parse, don't validate; no indices into
  structures; non-empty types; validated newtypes; no bools or Options standing for meaning
  (name the cases). A check a caller could do means the callee's type is too loose: move it to
  the boundary. Logic in pure functions, effects a thin edge. Premature bug-fixing over
  premature optimisation. Discard, never queue.
- **Process:** test-first; each commit verified on its own (`cargo +nightly fmt --all
  --check`, workspace tests, fuzzer `RIFT_FUZZ_CASES=2000 RIFT_FUZZ_STEPS=40 cargo test --lib
  drag_invariants_hold_across_interleavings`, `cargo check --locked`; for Roc work also `cargo
  test -p rift-core-roc --features roc` and the `roc-core` reactor tests). Prove each test
  catches its fix with a planted mutation.
- **No behaviour change** unless the lead approves one; a change visible to the user beyond
  IPC needs the lead's go.
- **Every differential needs a planted bug proven caught** (see Evidence); bias generators
  toward the deciding facts.
- **One holder per fact (W4c, the lead's):** a claim's flags and original id are part of the
  core's owner (`Claimant`), not a platform copy beside the core's "claimed": a projection that
  must agree with the core is a second holder, and the core holding the whole claim is the
  single holder we want.
- **Saved files keep loading:** parse old forms at the load boundary into new types; an
  impossible saved state is normalized with one `workspace-load: normalized` warn line (the
  W2b ruling) or refused with a typed error where validation refused it before.
- **Public-clean:** roc-2 is published (a copy, as `rearch/roc` on the owner's fork, linked
  from upstream issue #697; the current line as `rearch/roc-next`). No private paths or
  references in files or commit messages, no session trailers; keep the `Co-Authored-By` line.
  Publishing `rearch/roc-next` (the local line and the published copy have the same trees, not
  the same commits):
  1. `jj log -r 'rearch/roc-next@fork'` and find the local commit with the same tree (`jj diff
     --from <local> --to <published> --stat` is empty); everything after it is new.
  2. `jj duplicate -r '<local>+::experiment/roc-2' -d <published>` copies the new commits onto
     the published tip.
  3. Check the copy: `jj diff --from experiment/roc-2 --to <copy tip> --stat` is empty; `jj log
     -r '<published>..<copy tip>' --no-graph -T description` has no session trailer, no home
     directory path and no name of a private repository; neither do the `+` lines of `jj diff
     --git --from <published> --to <copy tip>`. A commit that has them gets `jj describe` on the
     copy.
  4. `jj bookmark set rearch/roc-next -r <copy tip> --allow-backwards` (the bookmark tracks
     `rearch/roc-next@fork`; `jj bookmark track rearch/roc-next@fork` once if it does not).
  5. Push over HTTPS: `T=$(gh auth token) && GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_COUNT=1
     GIT_CONFIG_KEY_0="url.https://x-access-token:${T}@github.com/.insteadOf"
     GIT_CONFIG_VALUE_0="git@github.com:" jj git push --remote fork -b rearch/roc-next`. jj
     signs the copies on push, so the published tip's id is the one `jj log -r
     rearch/roc-next@fork` shows afterwards. `experiment/roc-2` stays on the local line.
- **Never install or push;** never open a PR or comment upstream. One clean commit per slice;
  never `jj rebase`/`squash` to tidy history.
- **Layout checks live in `layout_engine/core_geometry.rs` (the lead's, 2026-11-23).** "No call
  site in `layout_engine/`" is about `roc_path` sites. The check that holds the core's frames to
  a layout system's (`check_traditional` and its kin) is called from the system's
  `calculate_layout` and lives in `core_geometry`, where the Roc layouts were checked before
  merge (b).
- **Old-system bugs** found on the way: fix each in its own commit with a literal regression
  test, and report it.
- Report to the lead by message, short, decisions first.

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

On upstream's layout systems (merge (b)), the shapes are built again, one system at a time, from
each system's own tree (`StackLayoutSystem::shape`, `tiles::shape` for the tiled systems,
`FloatingLayoutSystem::shape` onto the core's whole-layout `floating::Layout`,
`BspLayoutSystem::shape`, ...), and `core_geometry::check_traditional`
(and its kin per shape) holds the core's frames to the system's bit for bit on rift's path; the
system's frames still stand.

The core's geometry was written before upstream's #688 and is brought up to it rule by rule as
the differentials find each (the solver, the weights: Handoff).

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

- `roc/platform/main.roc` and `roc/platform/Boundary.roc`: the platform requires `core` with
  `init`, `step : model, Event -> { model, effects : List(Effect) }` and one typed function per
  model-free question (the five layouts, the focus, topology, lifecycle and workspace
  decisions); it provides `roc_init`, `roc_step`, `roc_release_model` and a `roc_<question>`
  per question, each with its own answer type (a fact-bearing question answers `[Decided(..),
  Unasked(U64)]`). Every record and union that crosses is a named nominal in `Boundary.roc`,
  mirroring the Rust core's type of the same meaning. Target `arm64mac: { inputs: [app],
  output: Archive }`; the model crosses only as `Box(Model)`.
- `roc/core/main.roc`: `RustCore` ported decision for decision, stepping on the platform's
  `Event` and deciding its `Effect`s. State inside the variants, `Option`s as small tag unions,
  every match naming its variants. The entries for the model-free questions hand their typed
  questions to the modules below.
- `crates/rift-core-roc`: empty unless its `roc` feature is on, so rift's own build never needs
  the Roc nightly. With `roc`, `build.rs` runs `roc build`, generates the glue (`roc glue`,
  `ROC_SRC`) into `OUT_DIR`, checks it (no hash- or number-named type, no single-tag union with
  several values) and states how long both took. `src/boundary.rs` is the one place Rust values
  become the glue's types and back (every unsafe operation on Roc memory is there; each boxed
  type has its own allocation helper). `RocCore::step` steps the core; `focus`, `topology`,
  `lifecycle` and `workspaces` ask the questions; the allocator hooks count allocations.
- In rift, `--features roc-core` makes the reactor's core `Shadowed` (the Roc core decides,
  the Rust core steps beside it, and they must decide identical effects), and puts the focus,
  topology and lifecycle decisions on `roc_path::check`. (The Roc layouts were on rift's path
  through `layout_engine::core_geometry` until merge (b) took upstream's layout systems; see
  Next item 2.) `Plainly` is the one translation of the reactor's events both ways.
- `roc/core/Geometry.roc`, `Bsp.roc`, `Traditional.roc`, `Scrolling.roc`, `Floating.roc`: the
  geometry; the recursive nodes are the platform's (`Boundary.BspNode`,
  `Boundary.TraditionalNode`). Bit for bit means Rust's float semantics written out:
  `f64::round` (half away from zero, keeping a zero's sign; Roc's own is `floor(x + 0.5)`),
  `max`/`min` as arm64's `fmaxnm`/`fminnm` (+0 beats -0 for max), `sum` from -0.
- `roc/core/Topology.roc`, `Lifecycle.roc`, `Focus.roc`, `Workspaces.roc`: the decisions, each
  reading its facts through `Asking.roc` (a fact, or that it was not asked, read in order).
- The machines kept between events: the platform requires `Windows` (the lifecycle), `Joined`
  (the workspaces: catalog, activations, who places each window) and `Mains` (the main window)
  beside `Model`, each with
  `roc_<machine>_init`, `_step` (and the lifecycle's `_invalidate`, `_revalidate`), `_view`
  and `_release`; a step unboxes and boxes again, as the core's does, so each stays unique.
- Pinned nightly: `roc/NIGHTLY`. The glue is generated on every build of the Roc core, from
  `RustGlue.roc` in a roc-lang/roc checkout at the same commit (`ROC_SRC`); about 24,000
  generated lines, never committed.

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

**The layout systems' shadows (2026-11-24, release):** see the Handoff, "The shadow, sampled":
6-187% of a pass. **The core's layout path:** BSP at 1000 windows 13.3 ms a pass before the
annotated tree, 175 us after (Handoff, "made linear").

**Twins (2026-11-20, release), per step / whole view:** lifecycle 75 ns at 10 to 1000
windows / 0.15, 0.36, 3 us at 10, 100, 1000; ownerships 160-200 ns at 10 to 1000 windows (on a
`Dict`; on a list it was 153 ns, 641 ns, 7 us at 10, 100, 1000) / 0.1 to 0.7 us; topology 210-300 ns at 5 to 100 workspaces / about 0.25 us. Rust's
steps are 20-81 ns. (Ownerships and topology were joined at W4b; see there for the joined
twin's costs.)

**Typed boundary (2026-11-20, release):** `RocCore::step` with the event's and effects'
conversions, 480-510 ns per step (p50 about 290 ns) against the Rust core's 120 ns, 3.5 Roc
allocations per step, flat from 10 to 2000 windows, none live afterwards (on words it was
680-970 ns and 4.6). Model-free questions on rift's path: `focus_candidate` over 10 windows
121 ns, `untracked_focus` 23 ns, `is_native_transient` 92 ns, `keeps_transient` 96 ns,
`retirement` 139 ns. The tables below are the word era's.

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

## Roc: the design we follow, and what is left (nightly-2026-11-23-b5c77fc)

Sources are the Roc checkout at the pinned commit (`b5c77fc`; written at `f25da45`, the bump's
changes noted): `docs/langref/`, `design.md`,
`docs/mini-tutorial-new-compiler.md`, `src/build/roc/Builtin.roc`, `src/cli/main.zig`. Earlier
notes called several of these friction; they are Roc's design, and the core follows them.

### Design we follow

- **A plain value never passes for a nominal type, nor a nominal for a plain one** (since the
  nightlies after `f25da45`; at `f25da45` a structural value still passed where a nominal was
  expected). Two nominals of the same shape are different types too. So the core modules take
  and answer the platform's own types (`Boundary`'s), never structural copies of them, and
  nothing is copied between the two. A record literal or a tag where the annotation names the
  nominal type is that type; `Rect.{ x, y, w, h }` builds one where nothing names it, and `{
  ..rect, w }` keeps it. Internal state that never crosses (the reconciler's `Sync`, a
  container's `Summary`) stays structural, with nominal fields.
- **Methods make a nominal type usable generically** (langref `static-dispatch.md`): each
  `Fact…` type answers what the platform asked (`answer`), each keyed answer gives its key and
  answer (`entry`), so `Asking.ask`, `ask_window` and `ask_word` stay generic over them
  (`where [f.answer : ..]`). `==` is a method, `is_eq`: `WindowId`, `MaybeWord`, `Screen`,
  `Destination` and `Armed` define it, and `WindowId` defines `to_hash` (a `Dict` key). A generic
  helper cannot build a nominal answer, so each model-free entry matches its decision into its
  own answer type.

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
- **Glue and packaging.** The glue is generated at build time (`crates/rift-core-roc/build.rs`,
  into `OUT_DIR`, never committed) from `RustGlue.roc` in a roc-lang/roc checkout (`ROC_SRC`; the
  nightly tarball has none). The generated file is a module (inner `//!` docs and `#![...]`
  attributes), so the build script turns those lines into plain comments for `include!`, and
  the crate allows the generator's lints (`unsafe_op_in_unsafe_fn`, `unexpected_cfgs`, ...) on
  the module. The glue's default `make_roc_host` allocators prefix their own header, so the
  host keeps its own allocator hooks (mixing the two crashed in `free`).
- **Glue names.** An anonymous record is named by hash (`AnonStruct<hex>`); an inline union by
  its tags (`NothingOrSome`), and a collision, or a generic nominal's instances (`Fact(a)`), by
  a number (`AskedOrUnaskedType24`, `TypesFactType26`) that shifts as other types come and go.
  Workaround: every boundary type is a named nominal in `roc/platform/Boundary.roc`, one per
  payload, and the build fails on `AnonStruct<hex>` or `...Type<digits>` in the glue.
- **Glue bug: a single-tag union with several values** (`[Stepped(Box(model), List(U64))]`) is
  described by its first value only: the glue declared `roc_step` as returning just the box.
  Cause: `src/glue/platform/TypeTable.roc:89`, `single_variant_payload`, keeps only
  `List.first(tag.payload)`, and `RustGlue.roc:159` emits that. Unfixed on main and unreported
  (2026-11-20). Workaround and guard: such values are a named record, and the build fails on a
  platform union with one tag carrying several values.
- **`roc check` hung** on a nominal type in a platform module that mentions the platform's
  `Model` variable (roc-lang/roc#13347, fixed in 8712e637, in nightly-2026-11-23-b5c77fc). The
  workaround, the entries' results declared in `Boundary` generic in the model (`Stepped(model)`
  and its kin), went with the bump: they are declared in the platform module on `Model`,
  `Windows`, `Joined` and `Mains`. All but one: **roc-lang/roc#13363** (in b5c77fc; fixed upstream
  in 3ef41a6, PR #13389, merged 2026-11-24, in no nightly yet) rejects a provided function
  answering a bare type as "polymorphic" when a type declared in the platform module wraps that
  type (30-line repro: `S := { c : C }` with `C` from an imported module makes `net : U64 -> C`
  polymorphic; a type parameter on `S` does not help). `LifecycleStepped` wraps `Change`, which
  `lifecycle_net` answers bare, so it stays in `Boundary`, generic; it moves on the next nightly.
  Also check #13249 on a bump (an `--opt=dev` SIGTRAP when a platform closure captures a
  Model-typed callback; closed).
- **`roc build` hung** (`roc check` passing) on an entry rebuilding tuples out of a nominal
  record's fields (the settle question's placement); giving the core's own type the record
  shape removed the hang. Not reduced further.
- **A tag named `Release`** gives a per-variant alias (`BoundaryEventRelease`) that collides
  with the glue's own `<Type>Release` helper struct; the event's tag is `Released` at the
  boundary.
- **Nominal types have no `==`** (equality is a method a nominal type would define), so a
  value of a platform type that reaches a comparison (a window or button from an event, stored
  in the model) is compared by its fields or by matching.
- **`roc build` hung again** on the step taking the platform's `Event` into a core-declared
  `Event` of the same shape; the core using the platform's `Event` itself removed it (as the
  layouts use the platform's recursive nodes).
- **Recursive types at the boundary.** Recursion through a `List` is a plain `RocList`; direct
  recursion (BSP's `One(BspNode) | Two(BspNode, BspNode)`, a traditional `Child`) comes out as
  raw `*mut T` Roc boxes, boxed at each level the glue names. The host allocates them with
  `allocate_box`, whose header depends on whether the payload holds refcounted values (the
  glue's own `decref` for each field says which); a wrong header crashes the differential.

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

Not in the list: the input mode table, already pure; now the core's (`rift_core::modes`).

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

## Workspaces and spaces

Plan approved (W1 activation decisions as pure core functions; W2 the catalog made structural on
the store side; W3 core shadow with a `workspace-divergence:` tripwire; W4 the flip; W5
topology into the core; W6 last, after upstream #688: the engine glue and persistence, which
#688 rewrites). Order: W2 then W1, so W1's functions take the typed state.

- 1026e20a (W2a, held): a workspace's `preferred_display` + `home_space` options are one
  `Home::Unplaced | Space(space) | OnDisplay { space, display } | DisplayOnly(display)`, and
  `rule_overridden: bool` is `RuleStanding::Followed | Overridden`. The saved form is unchanged
  (loads through `SavedWorkspace`, writes through `SavedWorkspaceRef`); a test loads four
  workspaces saved by the code before (every combination of the old fields) into the right
  types and writes each back byte for byte.
- W2b (activation), measured before typing it: across every reactor test and the fuzzer at
  2000 × 40 (about 517,000 observations after reactor events) a Space's activation is only
  ever showing a workspace (with or without a different previous one) or showing none (with or
  without one remembered). Shown and remembered at once, or a previous with nothing shown,
  never occur after an event; the store's own methods make them mid-operation, and old saved
  files could hold them.
- 6f757581 (W2b, held): `SpaceActivation = Showing { current, previous, remembered } | Vacant {
  remembered }`. The first proposal (remembered only while vacant) was wrong: an upstream
  legacy-load test showed a Space can show a fallback and still remember the workspace its
  departed display showed, so a returning display takes it back; the measurement missed it
  because every tested departure emptied the display. What the type rules out is history with
  nothing shown: repair panics on it in tests and logs `workspace-activation:` in a running Rift;
  a saved file with it loads as `Vacant { remembered }` with one `workspace-load: normalized`
  line. A literal test covers a departure that leaves a fallback shown and the return.
- 18144116 (W2c, held): the workspace SlotMap and the catalog order are one `Catalog` (private
  fields; `insert` appends, `remove` takes a workspace out of both; lookups keep the SlotMap's
  names, so the engine and persistence code #688 rewrites are untouched). The saved form is
  unchanged; at load the order is parsed into the catalog, and an unknown, duplicate or missing
  entry is refused with a typed `CatalogError`, as validation refused it before. A test loads a
  store saved by the code before and writes it back byte for byte.
- 3fb35b51 (W1, held): `rift_core::workspaces` decides which workspace a command reaches and
  where stepping goes (`eligible`, `step`, `resolve`, `previous`), from the catalog the store
  hands it; typed `Step`, `SkipEmpty`, `Wrapping` replace a Direction, an `Option<bool>` and a
  flag; a Direction other than left or right goes nowhere at the edge.
- a9b59c3c (W1, Roc): `roc/core/Workspaces.roc`, tags 29 to 31; 200,000 random questions each,
  identical word for word; a planted bug in the Roc reachability rule fails it.
- da583e7f (W3, held): the core shadow. `rift_core::workspaces::Topology` holds the catalog
  (workspace and Space, in order) and each Space's activation, told by `TopologyEvent::Created |
  Removed | Moved | Activated | Cleared`. The store's activation map is private, written only
  through its activation helpers; catalog inserts, removals and Space moves go through
  `catalog_insert`, `catalog_remove`, `move_workspace`; a loaded store seeds the shadow once.
  After every reactor event `workspace_divergences` must be empty (tests fail; a running Rift
  logs `workspace-divergence:` at debug under `rift_wm::actor::reactor`). A direct write to a
  workspace's Space is caught; `move_workspace` skipping the core fails reactor tests. Window
  ownership (still on the window record) joins the shadow in a later W3 slice. Not yet
  mirrored in Roc: `Topology::step`.
- **Open item for W5:** no fuzzer step reaches DisplayDeparture. The new DisplayDeparts /
  DisplayReturns steps run (not while a window is held), but in the test reactor's global-mode
  setup a departure moves no workspace (the departed display's workspaces stay dormant on its
  Space). Find the snapshot that drives `plan_workspace_placements` to a DisplayDeparture
  transfer, make a fuzzer step reach it, and re-measure activation shapes after W5. The
  store-level literal test covers the shape until then.

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
  to windows built from layout events alone.
- fc3d474e (L4, Roc; not needed for play): presence steps through the pure `present` (presence
  before + `PresenceEvent::Reading | Invalidate | Revalidate` → presence after +
  `PresenceOutcome`; `manages`); tag 26; the differential checks all 12,615 pairs; a planted
  bug fails it. (L3 added no core logic to mirror.)
- 95606e2e (L5, held): a window's ownership is one value on its record, `Ownership::Unassigned
  | Assigned(workspace) | Claimed(claim)`, the claim holding its destination outside the index;
  a claimed window in the index is unrepresentable (the debug_assert goes). The rekey merge is
  one exhaustive match over the two identities' ownership; a literal test of all ten
  combinations passes on the code before the change too. This stays in the store, not the
  core: a claim outlives AX state, so it is not part of the core's per-AX-state presence, and
  there is no core logic to mirror.
- 391ac12d (L6, held): whether an inventory retires a window, `rift_core::lifecycle::retirement`
  on `InventoryFacts` (with `NativeIdentity::Known(id)`, so evidence is asked only with an id)
  and lazily asked `NativeEvidence` → `Retire | Keep(why)`; held to the old rule over all
  2,592 combinations.
- b151281a (L6, Roc): tag 27; the differential checks all 2,592 combinations; a planted bug
  fails it.
- 8207d229 (held): no call site compares admission before and after. `refresh_heuristic` and
  `replace_info` return the core's `Transition`; `rift_core::lifecycle::net` composes two in a
  row (the inventory path's fresh roles and heuristic as one change); suspended vs removed is
  chosen from a typed `Availability` in `utils::layout_departure`. `AdmissionTransition` goes.
- 06b22687 (Roc): `net`, tag 28; the differential checks all 49 pairs; a planted bug fails it.
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
| Layout trees into the core | L+ | **Re-planned 2026-11-24 (Handoff).** The pre-#688 core trees are deleted (no caller since merge (b)). Next: a core BSP tree ported from #688's `systems/bsp.rs` under an operation shadow, a Roc mirror, the flip (saved from the core), the old tree gone. Then stack, traditional and master-stack, scrolling's columns, floating. |
| Window lifecycle complete | M | Admission, visibility, minimize, claims as core events. The admission facts' single writer (upstream plan C1 step 2) is done. A universal `apply(WindowChange)` over the store's other writers is not planned: raise it only when a Roc claim needs one step for the store, with that claim as the reason. |
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
