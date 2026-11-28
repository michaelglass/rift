# The core boundary, in Rust and in Roc

The goal: all of rift's decisions in a pure core, `step(model, event) -> effects`, written in
Roc, with the Rust platform doing only effects (AX, taps, CG/SkyLight, run loops, IPC, the
animation presenter). Each slice of rift moves into the core in Rust first (`RustCore`), then
the Roc app takes it over, held to the Rust core decision for decision.

History: the first Roc port stopped at `experiment/roc-paused` (2026-11-18) while the Rust
rearch finished on `experiment/rust-core` (the play-test line, now the owner's daily driver).
The Roc work resumed on `experiment/roc-2`, branched from `experiment/rust-core`.

## Handoff (2026-11-26, layout trees done: every layout system on the core): read this first

**State.** Workspace `rift-roc`, line `experiment/roc-2`. Every layout system is the core's:
BSP, the stack, the group forest (traditional and master-stack) and floating, each through its
five slices; under `roc-core` the Roc core decides each, the Rust core its shadow. Upstream's
layout code is gone (floating's step 5 deleted the last of it, with `GroupLayouts`); the loads
read upstream's saved forms through `group_stored` and `stack/stored`, frozen by fixtures.
`rearch/roc-next@fork` carries everything through this docs commit once it is published (publish
procedure: "Rulings that bind", Public-clean; the published tip's id is in the slice report). All
checks green at the tip: workspace tests 1070 (7 ignored), the lib with `roc-core` 943 (10
ignored), `cargo test -p rift-core-roc --features roc` 61 (14 ignored), `cargo test -p rift-core`
119, the fuzzer at 2000 × 40 both ways, `cargo check --locked` both ways, fmt, and `cargo clippy
--all-targets --features roc-core` compiles with no errors and 318 warnings, counted as `cargo
clippy --all-targets --features roc-core 2>&1 | grep -E '^(warning|error)' | grep -v generated |
wc -l` (the bound: at most 336).

**Next:** (1) a window's place by type: done 2026-11-26 (below, "A window's place by type").
(2) The Roc known copies need the compiler's ownership decisions (a dev-diagnostics build's
`ROC_LIR_DUMP`): the owner's call. (`--opt=dev` for test builds: tried and rejected, below.)
(3) The open perf leads below, only if a profile at real sizes asks. The two Roc issue drafts
below ("Roc issue drafts") wait for the owner.

**Every pure decision decided in Roc (2026-11-26, the lead's go).** The decisions on
`roc_path` were checked (the Rust answer stood); now `roc_path::decided` makes the Roc answer
rift's, the Rust answer its shadow, in families, each counted and logged at exit (`roc-core:
<family> divergences`): focus (8 sites), the mode gate (the verdict), topology (6), lifecycle
(12), workspaces (5). `roc_path::check` is gone. A different Roc answer is read back to rift's
values (`FromWord`, `window_of_plain`, `with_window`, `Verdict::try_map`, `Axis::try_map`, a
screen by its id); in tests, equal answers are read back too and must give the Rust value, so
every reading is exercised by the tests reaching its site. One naming nothing rift holds, or a
Roc decision that read an unasked fact, leaves the Rust answer with a warn. An activation refused
by a mode names the refusing axis across the boundary (`FactRefusal`, `Stay::Mode(RefusingAxis)`),
so that answer is complete. The reconciler (whose effects were already Roc's) counts a difference
instead of stopping Rift, and retires its Rust shadow in a running Rift (no view of the Roc model
crosses the boundary to re-seed it). The machines kept between events (lifecycle, main window,
workspaces) were decided already as twins. Plants per family in each commit; two meta-plants
(removing the read-back check, removing the reconciler's report) survive by their nature.
Gaps: no reactor test acks between 1.5 and 3 points (the ack tolerance is held by rift-core-roc's
edge differential). On hold (the lead): scrolling slice 1 until the owner confirms; the #713
merge when we get there.

**Roc nightly policy (the owner, 2026-11-27): rift tracks the newest Roc nightly.** The pin is
`roc/NIGHTLY` (a roc-lang/nightlies tag; roc-lang/roc's own releases stop at 0.1.0-RC2).
- `mise run roc-bump` (`roc/bump.sh`) reads the newest nightly, installs its compiler, its
  `-safe` build and its source (the glue spec, `RustGlue.roc`, is in the source tarball) under
  `~/.local/share/roc-nightly` (`ROC_NIGHTLY_HOME`), points `current`, `safe` and `src-current`
  at them (build.rs's defaults for `ROC` and `ROC_SRC`), writes `roc/NIGHTLY` and runs the check
  run. A failed check run puts the old pin and links back and names the checks that broke (logs
  in `target/check-logs`); the new nightly stays installed for a repro. The bump is its own
  commit (`roc/NIGHTLY` alone).
- `mise run check` (`roc/check.sh`) is the full check run, one line per check, exit 1 on any
  failure. Since 2026-11-27 it builds everything first, then runs the test runs side by side:
  the two lib runs and the drag fuzzers through libtest (nextest's process per test costs about
  0.4 s of system time each for the lib's 950 tests: 144 s against libtest's 54 s), the crates
  with few long tests through cargo-nextest (pinned in `mise.toml`, 0.9.149; rift-core-roc 43 s
  to 14 s). `mise run check-fast` (`roc/check.sh fast`) is the inner loop: fmt, the `roc-core`
  check, the lib with `roc-core`, rift-core-roc and rift-core, each drawn test cut to the counts
  in `FAST_CUTS` (one table in `roc/check.sh`; `RIFT_SCROLLING_CASES` added for the scrolling
  strips); not a publish gate. Wall times (12 cores), before and after: the full run after a
  Rust change 500 s to 133 s, after a `.roc` change 441 s to 177 s, after nothing 347 s to 91 s;
  check-fast 58 s, 109 s and 23 s. It starts by warning (never failing) when `roc/NIGHTLY` is older than the newest
  nightly; offline, it says nothing.
- Every slice starts with `mise run roc-bump`.
- A failed bump is reported to the lead with the check that broke and, for a Roc regression, a
  minimal repro (roc-bug-verify's repros and `both.sh` are the model). Never a workaround without
  telling the lead.
- When a bump fixes a bug rift works around, the workaround goes in the same or the next commit.
- Kept on purpose (roc-bug-verify, at nightly-2026-11-26-890d6c8): the one-tag answer-union
  convention (every answer union has two or more tags; a one-tag union still breaks), the deep
  `decref_list_of_*` calls, the build's `unnamed_types` guard, Geometry's rounding and
  `fmaxnm`/`fminnm`, and the host's own allocator hooks.

**The dev backend: tried and rejected (2026-11-27, the lead's ruling).** Every Roc build is
`--opt=speed`. Building tests with `--opt=dev` (a 0.2 s Roc build instead of about 58 s) assumed
the dev backend runs the same program, and at nightly-2026-11-26-890d6c8 it does not:
- **Misaligned static literals** (roc-lang/roc#13568): the dev backend's Mach-O writer places
  `__DATA,__const` right after `__text` without aligning it, so when `__text` is 4 mod 8 a static
  `Str`'s refcount sits where its atomic add faults (SIGBUS in `roc_builtins_incref_data_ptr`),
  depending on code size.
- **A separate crash in rift's dev build**, with every `__DATA,__const` at 0 mod 8 (so not the
  above): `the_roc_core_tiles_traditional_layouts_*` and `the_roc_core_tiles_what_the_rust_core_tiles`
  SIGSEGV in `Traditional.tree_of` (`roc_builtins_incref_data_ptr` on 0x4088F8…, the F64 799.0, a
  window's `locked_height`, read as a pointer) on a host-built shape. `--opt=size` crashes the
  same two tests the same way. Not our host (checked 2026-11-27): the host builds the shape
  field for field as the glue lays it out (the glue is identical under every opt level), and
  speed reads it correctly. The trigger is a nominal record as a recursive union's variant
  payload (`TraditionalAxis := {…}` in `TraditionalNode`): under dev the generated code reads
  the `Axis` payload's first word as a pointer to the 40-byte record, where the glue and speed
  keep it inline. Standalone repro (speed 7; size and dev SIGSEGV; the same record written
  inline prints 7 everywhere): `roc/repros/recursive-nominal-payload/`. The earlier reductions
  (`roc/repros/dev-from-platform-crash/`) wrote the payload inline, which is why they never
  crashed.
- Retry on a bump once both are fixed: `roc/bump.sh` runs rift-core-roc's layout tests under dev
  (`ROC_OPT=dev`, the build script's only other setting, in `target/roc-dev`) and says whether they
  pass, and runs `roc/repros/recursive-nominal-payload` under dev and size and says whether it
  prints 7; neither gates. The key cache stays (one optimized Roc build per `.roc` change).
- No workaround for the nominal-payload crash (ruled 2026-11-27): written inline, the payloads
  would get hashed glue names (`AnonStruct…`), against the named boundary and its guard; the gain
  (a size build in check-fast) does not pay for that.
- `mise run check-release` (what `rift-local.sh` runs before an install) is an alias of the check
  run: the release build's Roc is the one the check run tests.

**Scrolling onto the core, slice 1 (2026-11-27): the arrangement as the system's shadow.** The
core takes the scrolling strip's arrangement (`rift_core::layout::columns`): a strip is `Empty |
Columns { before, active, after }`, a column `{ id, width, before, active: Row, after }` (never
empty, its active row always there), a row `{ window, height, fill }` (a window's fullscreen mode
on its row, so it moves, swaps and is replaced with it); the operations run on an editing form
with rift's indices and are parsed back after each, the active indices clamped as rift clamps
them. Every structural operation and query is rift's `ScrollingLayoutSystem`'s, op for op. The
camera (offset, spring, gesture, bookmarks, reveal) stays rift's: it reads world geometry. Its
writes to the arrangement are its decisions: the column a gesture settles on or a remembered
view restores (`activate_column`), and, after each removal, the columns it makes active (since
slice 3, the caller's: see "Slice 3"); a layout pass's preserved widths read the pass's
constraints (`preserve_window_sizes`).
- The shadow (`systems/scrolling/core_shadow.rs`): rift's system decides; each operation is run
  again on the core seeded from rift's state just before it, and its answer and the arrangements
  it leaves are held to rift's (every operation in tests and debug builds, every 32nd in
  release; `core: scrolling arrangement divergences` at exit). The camera's and a pass's writes
  are logged (`Logged::Activated | Preserved | Removed`) and replayed on the core in order, a
  removal's answer taken at its marker. Arrangements compare as written (a height that is not a
  number is the same on both sides).
- One state only a saved file holds, which the core cannot: a fullscreen mode for a window no
  column holds. Rift kept it, so the layout had a fullscreen node forever; the load now drops it
  with a `scrolling-load:` warn (pinned by a fixture). The other file-only states (a window in
  two columns, two columns with one id, heights and widths that are zero, negative or not a
  number, active indices past the end, the legacy fields) step alike.
- Tests: the drawn differential (600 drawn systems, then replacements, an app closing, native
  resizes, layout clones and removals, gestures settling, every query; each reached), the
  file-built fixtures, two core parse tests. Plants 12/12.
- Slice 2 (2026-11-27): the Roc mirror, `roc/core/Columns.roc`, the same shape over the
  platform's types (`Columns*` in Boundary.roc: the strip and column zippers, the settings, an
  event per operation, an answer per kind) and the same editing form, ported function for
  function; `roc_columns_{init,seed,step,view,release}`; `rift_core_roc::columns` (the events,
  answers, view and `step` on the Rust system) and `RocColumns`. A removal's camera answers
  crossed as data (`camera: List(List(U64))`, taken in order; gone since slice 3). Differential
  (`tests/columns.rs`): 4000 runs of 60 drawn operations (every kind; layouts and column ids
  live and gone; reloads with heights, widths and fills only a file holds), each answer and the
  whole system compared as written: identical (240,000 operations). A tripwire: no step copies
  the layout table (64 layouts; the largest allocation of a step 1576 bytes, a copy several
  times that); the edited layout's own columns are rebuilt each step (O(its columns and rows),
  as the Rust core's editing form). Plants 15/15 in Roc, and a table copy caught by the
  tripwire.
- Slice 3, part 1 (2026-11-27, the lead's ruling): **no callback into the camera.** The core's
  `remove_window` and `close_app` asked the camera mid-operation (`camera: FnMut(..) ->
  Vec<ColumnId>`; in Roc, the answers crossed in advance as `camera: List(List(U64))`). Removal
  is now per layout and stops where the camera must choose: `forget_window(window) -> Vec<L>`
  (no longer left out; the layouts holding it), `forget_app(is_app) -> Vec<(L, Vec<W>)>` (each
  layout's windows of the app, in order) and `remove_from(layout, window) -> Option<Removal>`;
  the caller runs removal, the camera's answer (`activate_column`), the next removal. Roc's
  events are `ForgetWindow`, `ForgetApp`, `RemoveFrom`, answering `Layouts`, `Closing` and
  `Removal(Removed | NotHeld)`. The differential draws each step on its own, so every
  interleaving with other operations is reached.
- Slice 3, part 2 (2026-11-27): **the flip** (policy (a); `systems/scrolling/twinned.rs`, was
  `core_shadow.rs`). `ScrollingLayoutSystem` holds the core (`core: ColumnSystem<LayoutId,
  WindowId>`), stepped with every operation in every build; every answer, an operation's and a
  query's, is the core's. Rift's own arrangements (in `layouts`, beside each layout's camera and
  geometry) step as its shadow, because the camera still reads them: held to the core's after
  every operation in tests and debug builds, every 32nd in release; a difference is counted
  (`core: scrolling arrangement divergences`), fails a test, or warns `layout-divergence:` and
  re-seeds rift's arrangements and windows left out from the core's (`rift_arrangement`). The
  camera's writes (`Logged`) are made on the core after rift's side, a removal's answer after
  that removal. **Saving is the core's**: each layout's arrangement as the core holds it, in
  rift's saved form, so the bytes are rift's by construction; a load reads rift's form and seeds
  the core from it (`SavedSystem`). Settings (`update_settings`, `update_width_settings`) reach
  the core. Tests: a difference re-seeded, the answers the core's, saving the core's, width
  settings reaching the core (each makes rift's side differ on purpose, `twinned::tolerating`).
  Plants 9/9.
- **Fullscreen has one source, the row's fill** (the lead's ruling, audited): `ViewBookmark` is
  `{ column, relative_offset }` and the camera's `fullscreen_restore` is one of them, a camera
  position and nothing else; whether a window is fullscreen is read from rift's `fullscreen`
  map, which is the core's fills (re-seeded from them, and saved as `fullscreen_modes` from
  them). Left for the camera slice: a `fullscreen_restore` can outlive its fullscreen (it is
  dropped only when the selection leaves its column or the fullscreen ends through the camera);
  every use is gated on the active column having no fullscreen row, so it only ever restores a
  position.
- Slice 4 (2026-11-27): **under `roc-core` the Roc columns decide**
  (`systems/scrolling/roc_columns.rs`, as floating's step 4). The system's arrangements are a
  `Decider`: without `roc-core` the Rust core; with it `ColumnsDecider { rust, roc:
  RefCell<RocColumns> }`, whose methods are the Rust core's, name for name, so the scrolling
  system and `twinned` call the same methods in either build. Each operation and query goes to
  both; the Roc answer is rift's, the Rust core's held to it on every call, and the whole
  systems (`ColumnsView`, compared as written) after every operation in tests and debug builds
  and every 32nd in a `roc-core` release build. A difference is counted (`roc-core: scrolling
  columns re-seeds` at exit), fails a test, or warns `roc-divergence:`; an operation's re-seeds
  the Rust core from the Roc view (`core_of`). An answer of another kind, or naming a window rift
  never gave the Roc columns, leaves the Rust answer (after the report). An app's windows cross
  as a list (the windows the Rust core holds that `is_app` tells, left-out ones included), as
  the differential lists them. Saving stays the Rust core's (equal after every compared
  operation). Tests: the Roc answer is rift's (an operation's and a query's), a differing Rust
  core re-seeded from the Roc view, an app closing forgetting its windows left out of every
  layout. Plants 8/8 (answers and re-seed, the view comparison, the app's and locked widths'
  lists, the select event, a Roc insertion bug).
- Slice 5 (2026-11-27): **rift's arrangement deleted** (option (A), proposed to the lead; (B) below). Every operation
  is the core's (`Decider`), then the camera's answer to it; rift's own steps (`Arrangement`'s
  `move_destination`, `new_column`, `detach`, `transfer`, the `*_system` queries,
  `set_height_share`, `update_participation`) and the compare-and-re-seed machinery
  (`twinned`'s `decide`, `Logged`, the replays, `core: scrolling arrangement divergences`) are
  gone; `systems/scrolling/projection.rs` keeps the conversions. The camera reads each layout's
  arrangement as a **projection** in rift's shape, refreshed from the core after every change
  (`edited`: the core's edit inside the camera's `mutate` transaction, the projection refreshed
  before the geometry is rebuilt) and never stepped on its own; `inactive` is the projection of
  the core's windows left out. The camera's own writes go to the core: `restore`,
  `restore_fullscreen_view` and `settle` name the column, and `activate_chosen` makes it active
  on the core; a pass's preserved widths are the core's (`prepare_geometry` takes the preserve
  step from the system). A removal is the caller's sequence of slice 3: `forget_window` /
  `forget_app`, then per layout `remove_from` and the camera's answer (`remove_from_layout`,
  which reads the core's `Removal`). Saving writes the core's arrangement; a load seeds the core
  and projects from it. Tests: the projection is the core's after every operation, the answers
  the core's, saving the core's, a window that comes and goes giving back the view before it, a
  window left out removed and re-added taking part, a pass preserving a locked width on the
  core, a fullscreen ended giving back the view before it. Plants 9/10 (the tenth, a load
  leaving the projections as rift read them, is equivalent: rift's load and the core normalize
  the same states). The scrolling tests run 8x faster (no per-operation seed).
- **The view is computed, not stored** (the lead's ruling on slice 5, 2026-11-27): the stored,
  refreshed projection was a second copy that could go stale (the participation bug above was
  one). `LayoutState` holds only the camera and the geometry; `projection::view(&core
  arrangement) -> ArrangementView` builds rift's shape at each camera or geometry entry point and
  is passed down by reference; the windows left out are read from the core (`left_out()`), and
  `inactive` is gone. An edit is the system's (`edited`): the view before (the core's selection
  and its column), the core's edit, the view after, then the camera's half (`rebuilt`: the
  geometry rebuilt, the camera rebased on the selection). Rift's selection rule on the view
  (`selected`, `selected_location`, `Column::selected`) is deleted: the core is asked. A pass's
  view is taken after the core fixes preserved widths (`prepare_geometry` takes the view as a
  closure). The camera states sit in a `SecondaryMap` under the core's layout ids (one
  allocator); saving writes the core's table slot for slot (`slot_table::saved`, slotmap's own
  form, held to a slotmap's bytes by a test), a load reads `LoadedLayout`s and seeds the core.
  Cost: a view of 70 windows builds in 3.8 us (release), an edit with its geometry 24.5 us: no
  cache. Plants 6/6.
- **Left for the camera slice:** the view itself (the camera's reads of rift's shape, to be
  rewritten against the core's types), and `fullscreen_restore` living outside the fullscreen
  (the lead's ruling: put it inside the fullscreen state and delete the gates).
- **Upstream ports before the #718 merge** (the lead's plan, 2026-11-27): the merge of
  `pr/scrolling-clamp-after-edit` (eb00c2f1) brings upstream changes to decisions the core owns
  here; each is ported first, on this line, in the core (Rust, Roc, the differentials, plants),
  so the merge commit holds resolutions only.
  - (a) #706 (d8b83aba), cyclic navigation in the master-stack layout: `rotate_master_stack(layout,
    backwards) -> bool` on the core's `MasterStack` and in GroupForest.roc (`Rotate(MasterRotate)`
    answering `Verdict`): the windows taking part, master region then stack in tree order, each
    carried one slot toward the first master slot, the first wrapping to the last, by swaps (the
    containers stay); fewer than two taking part is no rotation. The commands
    `rotate_master_stack_next` / `_prev` as upstream's (protocol, CLI, default config, the
    engine forgetting a native layout only when the rotation changed something). Tests (upstream
    had none): the slots carried and carried back, a window left out keeping its slot, the
    commands going the ways they name through the reactor. Plants 9/9.
    **Open: Roc's rotation copies the node table** (the copy tripwire, 45 KB for a 400-window
    forest's 25.8 KB bound): the first swap after the order is listed copies, though a lone swap
    does not. Listing the order without closures holding the forest, swapping by recursion and
    writing the update inline through the record all still copy; time-boxed, `Rotate` joins the
    known copies (`Twin::known_copy`) for the copy hunt.
  - A plant-runner fix found on the way: a Roc plant that does not build was counted as caught
    (the build script failing); one slice-3 plant (`.reverse()`, not a Roc list method) proved
    nothing. The runner now reports a build failure as such, and that plant, redone with a fold,
    is caught.
  - (b) the unified scrolling column fallback (bafcc444) and #705's reveal of the surviving
    column (f168b6c6), the scrolling halves: when the selection's column goes, or no window in
    it takes part any more, the core shows an available column (`select_available_column`, Rust
    and Roc): the one the camera remembers from before a window came, if a window in it takes
    part, else the nearest from the active one rightwards, then leftwards (its active row kept).
    The camera's remembered column crosses as data, the core never asking the camera:
    `remove_from(layout, window, remembered)` (taken only when the window's column went, as
    upstream's restore) and `set_window_participation(window, participation, remembered)` (each
    layout's). The camera goes back to its remembered view only when that is the column the
    core showed, and otherwise reveals the one shown (participation: only when the column
    changed). #705's reactor half (frame writes cancelled without committing the presentation)
    and upstream's reactor tests for both come with the merge. Tests: upstream's
    `removing_selected_column_skips_suspended_neighbor`, carried over; the remembered column
    else the nearest (rightwards before leftwards) when a selection is left out; a column shown
    after a selection is left out revealed whole; a removal revealing the shown column when the
    remembered one takes no part. Plants 9/9.
  - (c) #663 (f08825a4), ghost tiles after a restore: the windows a restore retains start out
    dormant, left out until seen again, every layout's selection and saved view kept
    (`initialize_dormant_windows`; for scrolling the core's `leave_out_dormant`, Rust and Roc,
    with no selection repair); the camera keeps its saved view until its column has geometry
    (`apply_restored_view`, after a pass and after an edit), forgets it once the column is gone,
    a new window comes or a window is selected; and a participation change does not repair a
    layout's selection before its first pass or while its saved view is pending (the camera's
    `CameraMemory::Restoring`, alongside `Remembers(column)`, crossing as data). Upstream's three
    tests carried over (a window minimized through the store's setter, which keeps the
    lifecycle's facts in step, where upstream writes the AX state directly) and its legacy-load
    assertion; tests for what upstream left untested: windows seen before the first pass keeping
    the saved selection (with and without a saved view), a new window or a selection forgetting
    a dormant saved view. Plants 10/11 (the eleventh, dormant windows put through a
    participation change, is equivalent here: at a load the saved view or no window taking part
    keeps the selection).
  - (d) #707 (f6847f70): a window's size constraints live as long as the window, dropped by
    its retirement (`LayoutEngine::retire_window`, from the reactor's two retirement paths),
    never by leaving a layout. This line already kept them while the store held the window,
    the engine reading the store on a layout removal; upstream's three tests pass on both, and
    upstream's shape is taken for its single owner of the lifetime. Plants 4/4.
  - (e) 17a3c021, tears in scrolling animation: a resize command in a scrolling layout holds
    the camera still (our outcome's `is_resize`, where upstream picks `Presentation::Resize`),
    so the neighbours are placed on the same sample as the resized window; and a camera that
    holds still writes each resized window's target frame at once (bounded to the display),
    where a moving one still resizes at the old origin and eases from there. Upstream's test
    carried over (it fails here before the port: the neighbour lagged 50 px); two new ones (the
    resize commands hold the camera only in a scrolling layout; a moving camera resizes at the
    old origin). Shape difference: our outcome's `is_resize` covers every Space that pass
    arranges, where upstream's presentation is the target Space's; a resize changes only its
    own layout, so nothing else moves. Plants 5/6 (the sixth, the display bound dropped, is
    equivalent here: a still camera's viewport keeps the resized column on screen).
    Then the outcome's `is_resize` flag (which covered every Space a pass arranges) became
    data true where it is: `HeldCameras`, the Spaces whose camera holds still, merged as a set;
    each Space's pass reads its `CameraMotion::{Follows, HoldsStill}`, matched exhaustively in
    `animate_layout`. Plants 5/5.
  - (g) #705's reactor half (f168b6c6): upstream stopped a suspension's presentation cancel
    from committing the camera's in-flight viewport over the target the layout change had just
    set (`cancel_window_frame_writes`, without the commit). Nothing to port: this line's
    suspension (`reconcile_window_membership` → `WindowSuspended`) cancels no presentation.
    Upstream's test carried over and passes. Its plants survive (upstream's bug planted back,
    a cancel after the suspension; participation's reveal removed): with two full-width
    columns the strip's clamp places the camera on the survivor anyway, so the test guards the
    outcome, not the mechanism; a sharper one belongs with #718's clamp tests in (h).
  - (f) #709 (c12631bd), a drag drops on the display under the pointer, native drags too: this
    line already had it (the pointer's display is the drag's `current_space`, entering a display
    swaps in its scene, `plan_drop`'s `Across { space, onto }` carries a target there), with three
    of upstream's six tests. The other three carried over and pass in both builds (a drag whose
    first frame is on another display targets it; a drop with the pointer on its own display
    stays; one with no target lands on the pointer's display). Taken from upstream: one
    `prospective_drop_tree` builds both the preview and the drop's layout, so a drop lands where
    it was previewed. New test for what neither side tested: a drop that leaves the window in
    its slot previews `same_slot` and is refused. Plants 6/6.
- The rest of #718's range needs ports before the merge: our reactor no longer has upstream's
  presentation policy, so 17a3c021 (tears), #705's reactor half and #709 are ported into its
  shape, and #718's camera commits are the camera slice.
  - (h) #718 proper (cfa1e3b4..eb00c2f1), the camera slice. The viewport is `Uninitialized`,
    a `Gesture`, or `Resting(Rest::{At, Centred}, Motion::{Static, Pending, Animation})`. Every
    geometry change goes through `LayoutState::set_geometry`, which settles the viewport
    (`settle_viewport`): into the strip's bounds, or re-centred on the selected column when
    centred; a gesture keeps its overscroll until its release. What an edit keeps still is
    `Hold::Selection` or `Hold::Strip` (a move between columns). Upstream's thirteen camera
    tests carried over (fixed widths through a saved layout) and its changed persistence
    expectation (a restored view settles into bounds).
    - Beyond #718, the presented viewport: the presenter's camera is taken back by
      `Viewport::presented`, only out of a gesture or out of a resting viewport on its way to
      the very target presented, so a stale snapshot can never overwrite a target the layout
      set since. A cancel after a suspension (what #705's reactor half fixed upstream) cannot
      regress the camera by construction. Tested with quarter-width columns (moving and
      arrived presenters, presented and ended) and a reactor test where a presenter run
      through an invalidate-then-close settles the survivors where a headless run does.
    - The fullscreen return: `Camera.showing` is `Showing::{Strip, Fullscreen { window,
      return_to }}`, a return owed to `window`, not a second fullscreen fact (the core's fills
      say what is fullscreen). It changes only on the core's answers: the toggle's
      `Entered` (a return already owed is kept) and `Left` naming the window (consumed:
      restored, or dropped while the core's active column still shows a fullscreen window), a
      replacement (it follows the window); selecting or snapping to another column owes none.
      The settle point reconciles it with the core: a return whose window the core no longer
      shows fullscreen, by any path, is consumed there (a removal goes this way: the window's
      column, if it survives, is the active one). Saved with its window; a file saved before
      that is owed to its one fullscreen window.
    - Plants 18/19: #718's three (no clamp after a close; a centred view clamped, not
      re-centred; overscroll clamped mid-gesture), the settle and hold, centring's ends, the
      dormant saved view, the presented overwrite (moving, still), the fullscreen return's
      transitions (the toggle's answer, the settle's reconciliation, gates 1-3, the
      replacement, saving, the legacy load). By construction: the cancel after a suspension.
  - (i) bafcc444's reactor test: the #705 test as upstream extended it (two and three columns,
    one app each; closing the last column's AX-invalidated window shows the column before it,
    and focus goes there), carried over in place of (g)'s copy; it passes in both builds.
    Found while resolving the merge (upstream's copy auto-merged unadapted). Plant 1/1 (the
    nearest column leftwards taken from the far end).
- The merge (2026-11-27): upstream through #718 (eb00c2f1), resolutions only; each hunk
  resolved to this line's side names its port in the merge message.

### The engine slice, E0: an inventory of `LayoutEngine` (2026-11-27)

What `src/layout_engine/engine.rs` (5.2k lines) still decides itself, after W4d/W6 moved the
topology into the core. Extracted mechanically (each function's own reads and writes of the
engine's fields, then its calls followed for the routing question), classed by hand:
**decision** (chooses), **effect** (carries out or records), **query** (answers from state; it
moves when its state moves), **glue**. Steps: E1 routing, E2 floating and focus, E3 native
memory and displays, E4 constraints; *config* is the later config and mode-table slice; *stays*
is effect code or code already over a core.

**The engine's state and its owner after the engine slice:**

| field | what it is | owner after | step |
|---|---|---|---|
| `floating` | the floating set, each Space's active floating windows, the last floating focus, a floating window's fullscreen kind | the core | E2 |
| `floating_positions` | each workspace's floating frames | the core | E2 |
| `focused_window` | the engine's focused window | the core, beside floating's last focus | E2 |
| `window_layout_constraints` | each window's constraints (#707: live as long as the window) | the core, beside the window's lifecycle | E4 |
| `app_min_sizes` | an app's last reported minimum, shared by its rule windows | the core, with the constraints | E4 |
| `native_layouts` | layout snapshots kept for a departed display's return | the core | E3 |
| `native_reconcile_batch` | the workspaces a reconcile batch restores | the core | E3 |
| `evicted` | where a display departure moved a window from | the core | E3 |
| `space_display_map`, `display_last_space` | a Space's display, a display's last Space | the core, with the topology | E3 |
| `scroll_boundary` | the legacy strip command's accumulated boundary excess | the core, as routing's own state | E1 |
| `app_rules` | the app rule engine | config | config |
| `layout_settings` | the layout settings | config | config |
| `workspaces` | each workspace's layout systems (each over its own core) and mode | the engine, as the systems' container | stays |
| `persistence` | the saved form's bookkeeping | the engine (effects) | stays |
| `shadow_passes` | which passes run the systems' shadows | the dual-run scaffolding; goes with the shadows | stays |

**Does routing read that state?** Followed through calls from `handle_command`,
`handle_virtual_workspace_command` and `handle_event_inner`:
- floating and focus: **yes, as decisions.** `handle_command` itself reads
  `floating.is_floating(focused_window)`, `last_focus`, `fullscreen_kind` and the active
  floating windows to choose what a command does; `move_focus_internal` and
  `preferred_focus_for_workspace` read them too. E2 must precede E1, or E1's function takes
  them as facts to be deleted later.
- native memory: **only as effects.** Routing calls `forget_native_layout_*` (invalidation), and
  `relocate_window_to_workspace` / `transfer_workspace` write `native_layouts` and `evicted`; no
  routing decision reads them. E1 first would make its `Route` name "forget native memory" as an
  effect, scaffolding that E3 then folds into the core: a weaker reason than E2's, but the same
  direction.
- constraints: **no.** Routing reaches them only through `relocate_window_to_workspace` and the
  layout pass, both effects; E4 is independent of E1.
- `scroll_boundary` is routing's own state: it moves with E1.

**Proposed order: E4 → E2 → E3 → E1**, as the lead leaned, now checked: E2 is required before E1
(decisions read it); E3 before E1 avoids a forget-effect scaffold; E4 is independent and smallest,
next to the lifecycle. `focused_window` joins E2 (routing reads it), and the display maps join E3
(the display-relocation state). By count: E2 18 functions, E3 16, E1 13, E4 7, config 12,
stays 48.

**Every function** (line numbers at the merge, 84944850):

| line | function | class | state written / read | step | note |
|---|---|---|---|---|---|
| 44 | `action` | decision | - / - | stays | DropPreview: the centre action for a same-slot drop; pure, already a value |
| 140 | `windows_observed` | glue | - / - | stays | LayoutEvent constructor |
| 179 | `deref` | glue | - / - | stays |  |
| 189 | `insert` | effect | - / - | E4 | LayoutConstraints: record a window's constraints |
| 195 | `remove` | effect | - / - | E4 | LayoutConstraints |
| 201 | `set_width` | effect | - / - | E4 | LayoutConstraints: a preserved width |
| 210 | `prepare` | effect | - / - | E4 | LayoutConstraints: hand constraints to a system's pass |
| 267 | `new` | glue | - / - | stays |  |
| 299 | `focused_window` | query | - / focused_window | E2 |  |
| 301 | `apply_window_drop` | effect | native_layouts / workspaces | stays | a drop on a layout system (its own core decides); forgets native memory (E3 effect) |
| 328 | `relocate_window_with_drop` | decision | focused_window, workspaces, native_layouts / - | E1 | which workspace/layout a drop lands in, floating or tiled |
| 439 | `drop_scene_windows` | query | - / workspaces | stays | reads layout systems (their cores) |
| 463 | `drop_action_override` | query | - / workspaces | stays |  |
| 476 | `drop_preview_frame` | query | - / window_layout_constraints, workspaces, layout_settings | stays | geometry of a prospective layout (layout cores) |
| 515 | `prospective_drop_tree` | query | - / workspaces | stays |  |
| 532 | `source_move_neighbors` | query | - / workspaces | stays |  |
| 566 | `query_workspace_layout` | query | - / workspaces | stays | IPC query |
| 593 | `calculate_workspace_layout` | effect | - / workspaces | stays | layout pass |
| 617 | `calculate_arrangement` | effect | - / workspaces | stays | layout pass |
| 636 | `project_arrangement` | effect | - / window_layout_constraints, layout_settings | stays | layout pass |
| 667 | `switch_workspace_layout_mode` | decision | floating, floating_positions, workspaces / layout_settings | config | the mode table; carries floating positions across (E2 data) |
| 742 | `response_for_raised_windows` | glue | - / - | stays |  |
| 755 | `toggle_orientation_for_system` | effect | - / - | stays |  |
| 770 | `toggle_stack_for_workspace` | effect | workspaces / - | stays |  |
| 807 | `collect_group_containers_for_space` | query | - / window_layout_constraints, workspaces, layout_settings | stays |  |
| 835 | `set_layout_settings` | effect | - / layout_settings | config |  |
| 843 | `apply_layout_settings` | effect | layout_settings, native_layouts / workspaces, space_display_map | config | also forgets native memory |
| 857 | `update_virtual_workspace_settings` | effect | app_rules / workspaces, layout_settings | config | app rules |
| 906 | `active_layout_mode_at` | query | - / workspaces | config | the mode table |
| 914 | `focus_changes_geometry` | query | - / workspaces | config | by mode |
| 926 | `layout_specific_animate_settings` | query | - / workspaces, layout_settings | config |  |
| 943 | `active_floating_windows_in_workspace` | query | - / floating | E2 |  |
| 954 | `preferred_focus_for_workspace` | decision | - / floating, workspaces | E2 | focus on activation: floating last focus vs tiled selection |
| 988 | `commit_workspace_focus` | effect | floating, focused_window, workspaces / - | E2 | writes focused_window and floating last focus |
| 1031 | `activate_workspace` | effect | workspaces / - | stays | topology is the core's (W4d); the engine carries the activation |
| 1058 | `switch_to_workspace_id` | effect | - / workspaces | E1 | routes a switch |
| 1077 | `switch_to_previous_workspace` | effect | - / workspaces | E1 |  |
| 1089 | `navigate_to_workspace` | decision | workspaces / - | E1 | which workspace a next/prev names |
| 1121 | `filter_active_workspace_windows` | query | - / - | stays |  |
| 1131 | `filter_active_workspace_window` | query | - / - | stays |  |
| 1140 | `is_active_workspace_window` | query | - / - | stays |  |
| 1153 | `resize_selection` | effect | - / workspaces | stays | a layout system op |
| 1169 | `apply_focus_response` | effect | floating, focused_window, workspaces / - | E2 | writes focus from a system's answer |
| 1194 | `move_focus_internal` | decision | workspaces / focused_window | E1 | focus across tiled, floating and displays; reads floating (E2) |
| 1322 | `next_space_for_direction` | decision | - / - | E1 | the display in a direction; pure geometry |
| 1391 | `forget_native_layout_for_window_id` | effect | evicted / native_layouts | E3 |  |
| 1399 | `forget_native_layout_for_workspace` | effect | native_layouts / - | E3 |  |
| 1411 | `prune_native_layouts` | effect | native_layouts / - | E3 |  |
| 1419 | `forget_native_layout_for_window` | effect | native_layouts / - | E3 |  |
| 1426 | `remember_native_departure` | decision | native_layouts / floating, workspaces | E3 | what a display departure keeps |
| 1440 | `begin_native_reconcile` | effect | native_reconcile_batch / workspaces | E3 |  |
| 1456 | `finish_native_reconcile` | decision | native_reconcile_batch / - | E3 | what a reconcile batch restores or drops |
| 1477 | `note_eviction` | decision | evicted / workspaces | E3 | where an evicted window came from |
| 1505 | `eviction_ordinal` | query | - / evicted | E3 |  |
| 1522 | `take_eviction` | effect | evicted / - | E3 |  |
| 1526 | `restore_native_layout` | decision | workspaces, native_layouts / floating, focused_window, native_reconcile_batch | E3 | a returning display's layouts and windows back |
| 1603 | `suspend_window` | effect | floating, focused_window, workspaces / - | E2 | floating and focus bookkeeping of a suspension |
| 1623 | `remove_window_internal` | effect | floating, floating_positions, focused_window, workspaces, evicted / - | E2 | floating, positions, focus, evicted (E3) and the systems |
| 1651 | `remove_window_layout_membership` | effect | workspaces / floating | stays |  |
| 1683 | `add_window_to_layout` | decision | floating / workspaces, layout_settings | E2 | floating or tiled for a new window; constraints (E4) |
| 1745 | `preserve_scrolling_window_width` | effect | window_layout_constraints / - | E4 |  |
| 1766 | `remove_window_from_all_tiling_trees` | effect | - / workspaces | stays |  |
| 1772 | `workspace_contains_window` | query | - / workspaces | stays |  |
| 1780 | `active_workspace_contains_window` | query | - / workspaces | stays |  |
| 1795 | `projected_space_in` | query | - / floating, workspaces | E2 | reads floating |
| 1811 | `space_with_window` | query | - / workspaces | stays |  |
| 1818 | `active_workspace_id_and_name` | query | - / workspaces | stays |  |
| 1827 | `update_space_display` | decision | workspaces, space_display_map, display_last_space / - | E3 | a Space's display; display_last_space |
| 1872 | `sync_scrolling_widths_for_space` | effect | workspaces / layout_settings, space_display_map | config | per-display widths |
| 1882 | `last_space_for_display_uuid` | query | - / display_last_space | E3 |  |
| 1886 | `display_uuid_for_space` | query | - / space_display_map | E3 |  |
| 1891 | `remap_space` | decision | space_display_map / floating, workspaces, display_last_space | E3 | a Space replaced by another |
| 1922 | `forget_workspace` | effect | floating_positions, native_layouts / native_reconcile_batch | E3 | also floating positions (E2) |
| 1931 | `new` | glue | - / - | stays |  |
| 1957 | `apply_app_rule_outcome` | effect | floating / - | config | floats a window an app rule names |
| 1981 | `apply_app_rule_resize` | effect | window_layout_constraints, workspaces / layout_settings | config | constraints from a rule (E4 data) |
| 2016 | `debug_tree` | glue | - / - | stays |  |
| 2020 | `debug_tree_desc` | glue | - / workspaces | stays |  |
| 2051 | `normalize_event` | decision | - / workspaces | stays | already reads the core (#688) |
| 2066 | `handle_event` | glue | - / - | stays |  |
| 2077 | `handle_event_inner` | decision | floating, floating_positions, focused_window, window_layout_constraints, app_min_sizes, workspaces, evicted / layout_settings, native_layouts | E1 | the event router: which systems and state an event changes |
| 2474 | `handle_command` | decision | floating, floating_positions, workspaces, scroll_boundary / focused_window, layout_settings | E1 | the command router; reads floating and focus (E2), scroll_boundary (E1) |
| 3034 | `calculate_layout` | effect | workspaces / window_layout_constraints, shadow_passes | stays | test entry to the layout pass |
| 3058 | `calculate_layout_with_virtual_workspaces` | effect | - / - | stays | layout pass |
| 3084 | `calculate_layout_with_groups` | effect | floating_positions, workspaces / floating, window_layout_constraints, layout_settings, shadow_passes | stays | layout pass; reads floating (E2), positions (E2), constraints (E4) |
| 3254 | `active_workspace_for_space_has_fullscreen` | query | - / workspaces | stays |  |
| 3267 | `workspace_has_fullscreen` | query | - / floating, workspaces | E2 | reads floating fullscreen |
| 3286 | `collect_group_containers` | query | - / - | stays |  |
| 3298 | `logical_window_frames` | query | - / window_layout_constraints, workspaces | stays |  |
| 3327 | `calculate_layout_for_workspace` | effect | - / floating, floating_positions | stays | layout pass; floating frames (E2) |
| 3353 | `selected_window` | query | - / workspaces | stays |  |
| 3362 | `handle_virtual_workspace_command` | decision | focused_window, workspaces / floating | E1 | the workspace command router; reads floating (E2) |
| 3634 | `workspaces` | glue | - / workspaces | stays |  |
| 3636 | `workspaces_mut` | glue | workspaces / - | stays |  |
| 3638 | `assign_window_with_app_info` | decision | - / app_rules, workspaces | config | app rules |
| 3714 | `ensure_active_workspace_info` | effect | workspaces / - | stays |  |
| 3730 | `move_window_to_space` | decision | focused_window, workspaces / - | E1 | routes a move to a Space |
| 3792 | `move_active_workspace_to_space` | decision | workspaces / space_display_map | E1 |  |
| 3866 | `app_rule_names_workspace` | query | - / app_rules | config |  |
| 3872 | `transfer_workspace` | effect | floating, workspaces, native_layouts, evicted / - | E1 | carries a workspace transfer; floating (E2), native (E3) |
| 3926 | `ensure_workspace_layouts` | effect | - / workspaces | stays |  |
| 3936 | `relocate_window_to_workspace` | effect | floating, floating_positions, focused_window, native_layouts / workspaces, layout_settings | E1 | carries a relocation; floating (E2), positions (E2), native (E3) |
| 4043 | `workspace_name` | query | - / workspaces | stays |  |
| 4055 | `is_window_floating` | query | - / floating, workspaces | E2 |  |
| 4064 | `prepare_window_placement` | glue | - / - | stays |  |
| 4077 | `store_floating_position` | effect | floating_positions, workspaces / - | E2 |  |
| 4090 | `get_floating_position` | query | - / floating_positions, workspaces | E2 |  |
| 4101 | `workspace_floating_positions` | query | - / floating_positions | E2 |  |
| 4108 | `remove_floating_position` | effect | floating_positions / - | E2 |  |
| 4114 | `retire_window` | effect | window_layout_constraints / - | E4 | #707 |
| 4119 | `has_window_constraints` | glue | - / window_layout_constraints | E4 | test |
| 4123 | `transfer_persistent_window_identity` | effect | floating, floating_positions, focused_window, window_layout_constraints, workspaces, evicted / native_layouts | E2 | identity change for floating, positions, focus, constraints (E4), evicted (E3) |
| 4154 | `update_active_floating_windows` | effect | - / floating | E2 |  |
| 4165 | `store_floating_window_positions` | effect | - / workspaces | E2 |  |

### E4, window constraints (2026-11-27)

The engine's `window_layout_constraints` and `app_min_sizes` are a core machine,
`rift_core::constraints::Constraints` (Rust) and `roc/core/Sizes.roc` (Roc; Boundary's
`Constraints` already names the traditional layout's `Constrained | Unconstrained`), stepped by
`ConstraintsEvent`: `Observed` (the record from what the window reports; under
`respect_min_size` a window reporting no minimum takes its app's last one, and one reporting a
minimum makes it its app's), `WidthSet`, `WidthPreserved` (recording the window without
`respect_min_size` if it has none), `Retired` and `Replaced` (#707: a record lives as long as the
window and follows its identity), `AppQuit`. Records are normalized, widths set included;
`revision` moves on every change to the records. In five steps, each proven by planted bugs:
the Rust core (8/8); the engine shadowing it (every lib test checking the machine against the
engine's records on every step; 2/2); the flip (the engine's own records gone, each pass that
reads constraints getting them from the core, computed, never kept; three engine tests for what
had none; 8/8); the Roc port (the differential, 80,000 steps, state compared after each, and a
copy tripwire; 6/6); and Roc deciding (`roc_twin::TwinnedConstraints`, the Rust machine its
shadow, re-seeded on a difference; 3/4, the fourth, no re-seed after a difference, being
equivalent under tests, where a difference panics). A raw NaN width had been recorded as it
came; the differential found it (NaN never equals itself), and both cores now normalize it.

### E2, floating and focus (2026-11-28)

The engine's `FloatingManager`, `FloatingPositionStore` and `focused_window` are a core machine,
`rift_core::floats::Floats` (Rust) and `roc/core/Floats.roc` (Roc): the floating windows, a
floating window's fullscreen, each Space's active floating windows, the floating focus, the
focused window and each workspace's floating frames, stepped by `FloatsEvent`. A window's app
(for an app quitting) is `OfApp`: its pid, or for a window word its high half, sign-extended.
The engine holds it as `floating::EngineFloats`, one field carrying both old APIs, so the 110
sites only renamed (the paired calls merged: `replace_identity`, `app_quit`). The old types are
now only persistence's saved form: `FloatingManager` the floating windows and floating focus,
`FloatingPositionStore` the saved frames; their live operations went with the flip.

Five steps: the Rust core (9/9); the shadow (every lib test checking the machine's whole state
against the engine's after each step; two engine tests for what had none; 3/3); the flip (five
engine tests for what had none: a workspace's activation, a Space remapped, a workspace
forgotten, an app closing, a save and load; 9/9); the Roc port (the differential, 80,000 steps,
and a copy tripwire, which found a frame store copying its workspace's frame table: the copy
hunt's known shape, a field bound while its record stays live, fixed by taking the entry out of
the outer table first, `with_frames` and `with_active`; 7/7); and Roc deciding
(`roc_twin::TwinnedFloats`, seeded from a saved layout and compared at once in tests; 2/3, the
third, a quit app's pid crossing as its low half, being equivalent: Roc compares a window's app
with the pid's low 32 bits).

**Then (2026-11-28, the lead's ruling): the active floating windows are computed, never kept.**
Whether a floating window is active on a Space is now `LayoutEngine::is_floating_active`:
floating (the machine), in the Space's active workspace and available (the topology and the
window store). The machine's per-Space set, its five events and the dozen hand-sync sites are
gone, in Rust, Roc, the boundary and the twin; the machine has no Space type any more. The kept
set had drifted from its own definition: three of the hand-sync sites added a floating window to
a Space whose active workspace was not the window's, until the next rebuild. Tests for what the
hand-sync covered and nothing tested: a floating window is active only while its workspace is
(a switch away and back; a tiled window never), it projects onto its Space only from the active
workspace and while available (minimized, it does not), a remapped Space keeps them. Plants 3/3
(a tiled window counted; any workspace counted; an unavailable window counted).

**And pids cross as I32** (macOS's pid_t), in Boundary and both Roc machines (`Floats`'
`AppQuit`, `Sizes`' observations, app minimums and `AppQuit`), so no width differs between the
sides: a window word's app is its high half as an I32 (`OfApp for u64`, `Floats.of_app`).
Plants 3/3 (a window's app read from its low half, in Roc and in Rust; a quit app's minimum
removed under another pid). The earlier "equivalent" pid plant has nothing left to cross.

#### E3, in three steps

E3's state is three things with different shapes, so it goes in three steps, data first:
**E3a** the display memory (`space_display_map`, `display_last_space`), **E3b** the evictions
(`evicted`), **E3c** the native-layout memory (`native_layouts`, `native_reconcile_batch`). The
snapshots E3c keeps are layout systems, whose own cores hold their state; the E3c machine
decides which workspaces keep one, which windows each covers and when one is restored or
dropped. **The lead's ruling (2026-11-28):** no host-side snapshot store. A snapshot is the
core's own layout-state value, the type the core arrangement uses, held by the E3c machine:
restore hands it back, drop deletes it, and the host keeps nothing. A layout system whose state
is not core-owned yet is named before E3c, then brought into the core first or kept as a
named, temporary exception.

**E3a (2026-11-28): the display memory is a core machine.**
`rift_core::display_memory::DisplayMemory<S, D>`, twinned (policy (a)) by
`roc_twin::TwinnedDisplays` over `roc/core/DisplayMemory.roc`, holds each Space's display and
each display's last Space; it was the engine's `space_display_map` and `display_last_space`.
Events: `Shown { space, display, last }`, `Hidden`, `Remapped`, `Forgotten`; each step answers
whether either map changed.
- **Whether a display's last Space follows is a `LastSpace::{Follow, Hold}` the engine passes**,
  not a flag: `Hold` while that last Space is a saved Space not resolved yet, whose evidence it
  still is. Which saved Spaces are unresolved is persistence state, outside this slice.
  **On E1's "stays" list, owned by the config slice:** `update_space_display` reads
  `saved_space_is_unresolved` to choose `Hold`. It is a decision input the engine computes,
  accepted for now (the lead, 2026-11-28); it moves when persistence becomes a core machine.
- **A Space's display is a `D`, never an `Option<D>`.** The saved form keeps its
  `HashMap<SpaceId, Option<String>>` for compatibility; a `None` read from a file is a Space
  whose display is not known, so it is dropped at load (parsed once) and never written.
- **Displays cross as words**: each display's place in the twin's table of display names
  (`roc_twin::DisplayNames`), which only grows: bounded by the displays the process has ever
  seen. The seed steps the Roc core to a loaded memory (each last Space shown and
  hidden again, then each Space shown with `Hold`) and is compared at once in tests.
- `update_space_display` resyncs a Space's column widths when the memory changed or a workspace
  on the Space took the display as its home; it was an early return on "nothing to do".
- The step updates in place: the first port read `last_space` after inserting into it, so the
  insert copied the table; the tripwire (300 Spaces on 300 displays) caught it, and every read
  now comes first.
- Tests for what nothing tested: a hidden Space loses its display and stays its display's last;
  a remapped Space takes its display along; a workspace homed with no display takes the
  Space's display and its column widths; a display holds its unresolved saved Space until it
  is resolved (then follows); a gone saved Space leaves no display behind. Differential 80 000
  steps and the copy tripwire in `rift-core-roc/tests/display_memory.rs`.
- Plants 10/10: an unresolved saved Space's last Space followed; a hidden Space keeps its
  display; no width resync for a newly homed workspace; a remap leaves the display behind; a
  gone saved Space keeps its display; a display saved as none; in Roc, a held last Space
  followed, a remap leaving last Spaces, a forgotten Space still a last Space, a self-remap
  moving.

**E3b (2026-11-28): the evictions are a core machine.**
`rift_core::evictions::Evictions<W, D, Ws>`, twinned (policy (a)) by `roc_twin::TwinnedEvictions`
over `roc/core/Evictions.roc`, holds, for each window a display departure moved, the display it
left, its workspace's position there and the workspace it landed in; it was the engine's
`evicted`. Events: `Evicted`, `Forgotten`, `Replaced`, `AppQuit`.
- **The machine decides whether a departure is recorded**: `Evicted` carries the display the
  window landed on (`Option<D>`: a Space can show on no known display), and a window that
  stayed on its display leaves no record. The engine only gathers the facts (`note_eviction`).
- **Whether a record still applies is a query of the machine**: `ordinal(window, assigned,
  showing)` (still in its landing, and on the display it left), `stale(assigned)` (for a
  reconcile batch) and `landed_in(workspace)`. The engine's `eviction_ordinal`,
  `finish_native_reconcile` and `forget_native_layout_for_workspace` read them.
- `remove_window_internal` forgot the record a second time; its only caller that drops the window
  (`WindowRemoved`) has already forgotten it through `forget_native_layout_for_window_id`, so
  that step is gone.
- Displays cross as words through the same `DisplayNames` table type as E3a (each twin its own).
- Tests for what nothing tested in the engine: a window moved to another display is recorded,
  one that stayed is not, a return consumes the record; a record goes with its window, its app
  or its workspace's transfer, and follows its window's new identity. Differential (80 000
  steps) and copy tripwire in `rift-core-roc/tests/evictions.rs`.
- Plants 13/13: a window that stayed recorded; a returned window keeps its record; a quit app's,
  a rekeyed window's, a transferred workspace's and a removed window's records kept; in the
  core, a record applying in any workspace, on any display, none stale, `landed_in` finding none;
  in Roc, a window that stayed recorded, a replacement keeping the target's record, a quit
  app's records kept.

**Rulings before E3c (the lead, 2026-11-28).** Not core-owned today: Scrolling's per-layout
camera, and each workspace's `WorkspaceLayoutState`. (1) The camera reaches the core first, as
its own slice, **E3-cam**, with no exception: resetting it on restore would lose a restored
workspace's scroll position. (2) `WorkspaceLayoutState` folds into E3c: a snapshot is (core
layout value, active layout, active size, camera); the obsolete migration ids are parsed once at
load and dropped from live state. (3) E3c holds the Rust core value and reseeds the Roc twin on
restore, counting those reseeds apart from reseeds after a disagreement. Order: E3-cam, E3c, E1.

#### E3-cam: the scrolling camera as a core machine (plan, cam-0)

What the camera is today: per layout, in `ScrollingLayoutSystem.layouts`, a `Camera` (the
`Viewport` with its `Rest` and `Motion`, the gesture's `MotionHistory`, `transient_restore`,
`showing`, `restored_view`) beside a `Geometry`. About 120 reads and writes in `scrolling.rs`;
19 callers outside it (the reactor's presenter and gestures). (h)'s types stay as they are.

| State | Kind | Decided by | Moves to |
|---|---|---|---|
| `Viewport` (`Uninitialized`, `Gesture`, `Resting(Rest, Motion)`) | camera state | camera | the core |
| `Motion::Animation(CameraSpring)` (from, target, velocity, current, started, sampled) | camera state; its two instants are inputs it stored | camera | the core, instants as `Time` |
| `MotionHistory` (recent deltas with their timestamps, overscroll) | camera state | camera | the core |
| `Showing` (`Strip`, `Fullscreen { window, return_to }`) | camera state, changed only on the columns core's answers | camera | the core |
| `transient_restore`, `restored_view` (bookmarks) | camera state | camera | the core |
| `Geometry` (screen, tiling, gaps, constraints, column spans, frames, bounds, snaps) | prepared-frame cache, rebuilt from core state | host | **stays host**; the camera reads a projection of it |
| `ViewportPresentation` (the presenter's clone, motion behind a mutex) | the presenter's copy | presenter (effect) | stays the presenter's; its result returns as an event |

**Design.**
- **One machine for every scrolling layout's camera**, `rift_core::camera::Cameras<L, C, W>`
  (layout, column, window), held by `ScrollingLayoutSystem` beside the columns core, as the
  columns core holds every arrangement. Saved as today: the saved form does not change.
- **Geometry crosses as an input, never as state.** Each event that reads it carries a `Spans`
  projection of the host's cache: each laid-out column's id, world x and width, the bounds, the
  view width, the snap points, and the anchor offsets for the configured alignment. The camera
  never keeps it; a pass with new geometry is one event, `Settled`, which is the single settle
  point (rebase on the held bookmark, reconcile the owed return against the columns core's
  fullscreen facts, move the target into bounds or onto the centred column).
- **The clock crosses as an input.** `Instant` becomes `Time`, seconds on the host's monotonic
  clock, carried by every event that samples, aims or springs (the settle point's
  `Instant::now()` becomes the event's `now`). A spring keeps the `Time`s it was given.
- **The arrangement's facts cross as inputs too**: the active column, the selected window and
  the fullscreen windows, from the columns core's view at each entry point, as today.
- **The camera's choices are answers.** A column the camera chooses (a gesture's release, a
  snap, a restored bookmark) is answered, and the host has the columns core make it active, as
  `activate_chosen` does now; `scroll_by_delta`'s boundary excess and a release's
  `ViewportRelease` are answers.
- **Invariants kept, and planted:** the single settle point; `Viewport::presented` never
  overwrites a target the layout set; `Showing` changes only on the columns core's answers
  (`Fullscreened`, a removal or replacement naming the window) and is reconciled at the settle
  point.

**Steps** (each published on its own):
1. **cam-1, the Rust core**: `rift_core::camera` with (h)'s types, events and answers, `Time` and
   `Spans` as inputs, unit tests moved with it; the scrolling system steps it beside its own
   camera and compares in tests (the shadow).
2. **cam-2, flip**: the scrolling system's camera code goes; it projects `Spans`, steps the core
   and applies its answers. Plants on the invariants.
3. **cam-3, the Roc port**: `roc/core/Cameras.roc`, a differential over random gestures, edits,
   presentations and settles, and a copy tripwire.
4. **cam-4, Roc decides**: twinned (policy (a)) like the other machines.

Then E3c, then E1.

**The lead's adjustments (2026-11-28):** `Time` is integer nanoseconds (U64), not F64 seconds:
it crosses and compares exactly, and becomes F64 only inside the spring and velocity
arithmetic, in the same order of operations on both sides. A spring keeps the times it was given
(facts), never an elapsed duration (derived, stale every tick). At least one plant feeds a stale
or reordered `now`.

**Follow-up (named, not blocking E3-cam):** `Spans` is core state making a round trip through
the host. `Geometry` is computed from state the core already owns (the columns, the
constraints since E4, the screen), so it becomes a pure core function and the camera reads it
directly; the host's prepared-frame cache then holds only what the presenter draws.

**cam-1 and cam-2 together (2026-11-28): the camera is a core machine, and the scrolling
system steps it.** A deviation from the plan, for a concrete reason: `rift_core::camera` is the
camera code itself, moved with its inputs made explicit, so a shadow would have mirrored about
thirty call sites into a second copy only to delete them one step later. What a shadow step
would have proven is proven instead by the scrolling system's own tests (all of them, unchanged
in what they assert) passing through the core, by the core's unit tests, and by plants. The Roc
port (cam-3) brings the real differential.
- `rift_core::camera::Cameras<L, C, W>` holds every scrolling layout's camera beside the columns
  core; `ScrollingLayoutSystem` keeps only each layout's `Geometry` cache. Every camera change is
  a `CamerasEvent` (`Created`, `Cloned`, `Removed`, or one layout's `CameraEvent`): `Prepared`
  (a pass with new geometry, the settle point), `Rebuilt` (an edit's), `Revealed`, `Centred`,
  `Inserting`, `WindowRemoved`, `Selected`, `Followed`, `TransientForgotten`, `ViewForgotten`,
  `Replaced`, `FullscreenLeft`, `FullscreenEntered`, `Finished`, `Presented`, `ScrollBy`,
  `GestureBegan`, `GestureMoved`, `GestureEnded`, `GestureCancelled`, `Snapped`.
- Each event carries the `Spans` (the cache's projection) and `Facts` (the columns core's
  arrangement) it reads; the camera keeps neither. Which column an edit holds still is the
  camera's choice from those facts (`HeldAt` carries the selection as it was before the edit).
- Answers: `Activate(column)` (a fullscreen's return), `Released` (a release or snap: the column
  chosen, which the host has the columns core make active), `Excess`, `Began`, `Moved`.
- The clock: the host converts `Instant` once, at its edge (`camera_time`, nanoseconds since the
  process first asked), and a gesture's timestamps (`gesture_time`); the settle point's
  `Instant::now()` is the event's `now`. The presenter's copy (`ViewportPresentation`) and the
  reactor's frame animation use the core's `Viewport` and `Spring` with the same conversion.
- The saved form is unchanged (a saved non-finite offset still reads as 0).
- Tests for what nothing tested: a retarget at a stale moment restarts from the last sample; an
  edit holds the selected window's new column still; a snap to another column owes no return.
- Plants 13/13: the three invariants (a presented spring overwriting a target the layout set; an
  edit's rebuild skipping the settle point; the settle point not reconciling the owed return);
  leaving fullscreen not consuming the return; the clock (a spring sampling a stale `now`
  backwards, a retarget restarting at a stale `now`, gesture deltas all at one stale moment, a
  release springing from a stale start); and the decisions (an edit holding the old column, the
  first view animating, a gesture beginning on a fullscreen selection, a snap keeping the return
  owed, a removal restoring a view of a column the core does not show).

**The equivalence check, confirmed (the lead asked):** in the cam-1+2 commit no existing scrolling
or reactor test's assertions changed. Every changed line in the test modules is an accessor path
(`f.system.layouts[l].camera.viewport` → `camera(&f.system, l).viewport()`) or a test's setup
seeding a viewport (`seed_viewport`, `aim`) or calling the camera's settle decision (`settle`);
no reactor test file changed. The asserted values are the same.

**cam-3 (2026-11-28): the Roc port.** `roc/core/Cameras.roc` holds the same machine over words
(layouts, columns, windows; times in nanoseconds), with every Boundary type (`Cam*`,
`CameraEvent`, `CamerasEvent`, `CamAnswer`) and `RocCameras` beside it.
- **`exp` is portable.** Roc has no `exp` (and its `pow` and Rust's libm differ in the last
  bits), so both cores use one `camera::exp` written from `+`, `*` and floor: range reduction by
  ln 2 (fdlibm's split constants), a degree-13 Taylor polynomial, 2^k from its bits; within
  4e-16 relative of the library's over the spring's range. √800 and ln 0.997 are shared
  literals.
- **Differential** (`rift-core-roc/tests/cameras.rs`): 120 000 steps, every answer and the whole
  machine compared after each. Random strips and arrangements (dormant columns, stray columns,
  ties between resting points, a dormant column's lone resting point), a clock that moves
  forward, repeats a moment and goes back (and lands inside a live spring's flight), presented
  springs aimed at the camera's own target, histories at the 64-sample cap; plus a scripted run
  through a full history, and a copy tripwire (200 cameras).
- **Found on the way:** the view's release of its list was shallow (`RocList::decref`), so each
  camera's motion-history list leaked once per view; it is now the generated deep release
  (`decref_list_of_boundary_cam_entry`). The other views were checked: their lists are flat or
  already released deep.
- Plants 11/12 in Roc: the clock (a spring sampling a stale `now` backwards, a retarget at a
  stale `now`, a stale gesture moment accepted, samples kept past 150 ms), the invariants (a
  presented spring overwriting a target, the settle point skipping the owed return), an edit
  holding the old column, a tie broken the other way, niri travel from a resting point whose
  column is not laid out, the 64-sample cap dropped, and an `exp` coefficient off in its fourth
  digit. The twelfth, a coefficient off by one ulp, survives and is near-equivalent: measured,
  it changes the polynomial's result in 0.012% of evaluations. Five of the eleven survived the
  first generator; each led to the generator change named above, not to a code change.

**A shallow list release can no longer be written (the lead's ruling, 2026-11-28).** A view
releasing a list of refcounted elements with `RocListWith::decref` leaks every nested
allocation. `crates/rift-core-roc/clippy.toml` disallows that method (the reason names both
ways out) and the crate denies `clippy::disallowed_methods`, allowed only on the generated ABI
module and on `boundary::release_flat`, whose type takes only `RocListWith<T, false>` (lists
of plain elements). A list of refcounted elements goes through its generated deep release
(`abi::decref_list_of_...`). The 25 shallow releases in view and seed code all type-checked as
flat lists, so no second leak was hiding. Planted: a shallow `decref` in a view is refused by
clippy; `release_flat` on a list of refcounted elements does not compile.

**cam-4 (2026-11-28): Roc decides.** `scrolling/cameras_twin.rs`'s `TwinnedCameras` holds the
cameras for the scrolling system (policy (a)): under `roc-core` the Roc core's answers are the
system's, and the Rust machine is its shadow, read through `Deref` and held to it (every answer
on every step; the whole machine on every step in tests and debug builds, every 32nd in a
release build). A difference is reported (`roc-divergence:`, a failure in tests), counted
(`cameras_roc_reseeds`) and re-seeds the shadow from the Roc core's view.
- **`CamerasEvent::Seeded { layout, camera }`** carries a loaded or restored camera to both
  sides (it replaced `Cameras::seed`); a loaded system and a copied one (`Clone`, which E3c's
  restore will use) seed the Roc core this way. **Seeds are counted apart** (`cameras_roc_seeds`)
  from re-seeds after a disagreement, as ruled for E3c, so a disagreement never hides among
  restores. The test setups that seed a viewport go through the same event.
- Ids cross through `map` on every camera type (`CameraEvent`, `Camera`, `Cameras`, `Answer`,
  ...), written once in the core: layouts as their slotmap words, columns as their ids, windows
  as their words.
- The run log reports both counts.
- Plants 6/6: a loaded camera not counted as a seed; seeds counted as re-seeds; a re-seed leaving
  the shadow as it was; a copy not seeding the Roc core; a column crossing as the wrong word; a
  loaded camera reaching only the shadow. (That the Roc core's answer is returned rather than the
  shadow's is the step's shape; while the two agree, a plant there is equivalent.)

**The clippy +1 after cam-4, found:** `ScrollingLayoutSystem` held `TwinnedCameras` inline, and the
twin (the Roc box, its host table, the step count) grew `LayoutSystemKind` from 336 to 408
bytes, which made `large_enum_variant` fire in `systems.rs`, a file cam-4 did not touch. The
twin is boxed now (`Box<Twin>`); the warning list is back to what it was (the enum 344 bytes).

#### E3c

Three steps, data first (the lead approved, 2026-11-28): **E3c-1** the snapshot as core values,
**E3c-2** the native-memory machine in rift-core, **E3c-3** its Roc twin. E3c-1 lands in two
commits.

**E3c-1a (2026-11-28): a workspace's layout state is a sum type, and an older file's obsolete
layouts are parsed at the boundary.**
- `WorkspaceLayoutState` is `Unlaid | Laid { layout, size }`: no size without a layout. It is
  written as files have always held it (`layout`, `active_size`).
- **The saved form is the only one that knows obsolete layouts** (the lead's ruling).
  `SavedLayoutState` (the file's layout, size and an older multi-size file's other layouts) and
  `SavedWorkspaces` (the file's workspaces, their catalog order checked as it is read) are what
  the file deserializes into; `PersistedFile` holds them, and `PersistedLayout` is the parsed,
  live result. `SavedWorkspaces::into_loaded` converts them once: each workspace's members are
  worked out over every layout the file holds (the old `all_layouts` rule), the active layout is
  kept, the obsolete layouts' trees are dropped there, and the topology is checked. Live
  workspaces have no obsolete ids, no retire-on-first-activation, and no parse-time field; the
  persistence pass takes membership from the conversion (`Members`), not from live trees.
- **One existing test's assertions changed, on purpose:** the legacy-v2 load test asserted the old
  representation (two layouts live after load, four dormant windows across both trees, one layout
  after the first activation). It now asserts the active layout kept, the other display size's
  layout gone at load, and two dormant windows; membership (`pending`) is asserted as before.
- Tests: a window saved only in an obsolete layout stays its workspace's member after load, with
  no saved position (the lead's plant); a layout state keeps its layout across sizes and its size
  across a new layout. Plants 4/4: members from the active layout only; obsolete trees kept; a
  laid-out workspace forgetting its size on a replace; a saved layout's validation skipped.

**The clippy gate (2026-11-28, the lead's ruling).** `check.sh`'s clippy count depended on the
cache: clippy re-emits warnings only for crates it re-lints (one run printed 4). Now:
- **Every full check re-lints the workspace's own crates**: clippy gets a new
  `--cfg clippy_gate="<nonce>"` (declared with `--check-cfg`) each run. Clippy's arguments are in
  the fingerprint of the workspace's crates only, so exactly they re-lint; dependencies, build
  scripts and the test builds stay cached (touching the sources instead invalidated the test
  builds too and took the full check from about 200 s to 431 s). The re-lint costs 28.8 s
  steady (the cached run took 0.2 s), so check-fast, which runs no clippy, stays without it.
- **`roc/clippy-baseline.txt`** lists the warnings (293 today, 245 keys), each by lint, file
  and the item it is in (the nearest item declaration above it), with a count: no line
  numbers, so unrelated edits do not churn it.
- **`roc/clippy_gate.py check` fails** on a warning not in the baseline, and on one the baseline
  lists that is gone, naming it: shrink the baseline in the same commit (`shrink`, which never
  adds one). The list can only go down. Tried both ways: an unused variable added fails the gate;
  a listed warning that is gone fails it until `shrink` removes it.
- **It runs beside the tests, in its own target dir** (`target/clippy`), so it never waits on
  the build lock and never invalidates the main target's `cargo check` artifacts: the full check
  with everything built went from 99 s (gate serial) to 50 s and 51 s, more than the gate's own
  29 s.

**E3c-1b (2026-11-28): a native snapshot holds the cores' values, never a live system.**
- `systems::SystemSnapshot` is one of `traditional`, `bsp`, `master_stack`, `stack` (each its
  core's value: `Traditional`, `Tree`, `MasterStack`, `Stacks`) or `scrolling` (the columns
  core's arrangements, every layout's camera, and the settings: never the geometry, a cache
  rebuilt from these). `LayoutSystemKind::snapshot` takes it (a floating system keeps none);
  `SystemSnapshot::restored` builds the live system from it, its Roc twins seeded (the cameras'
  counted as seeds).
- The engine's `NativeSnapshot` is (that value, the active layout, the size it was laid out for,
  that layout's windows): the shape ruled. It replaces the `(LayoutSystemKind,
  WorkspaceLayoutState)` pair. A workspace not laid out keeps none.
- An edit to a held snapshot (a window closing, an app quitting, an identity changing) goes
  through its system: rebuilt, edited, taken back, so a scrolling camera answers a removal as
  it did; it runs only for a snapshot holding the window (before, every snapshot's system took
  every window's edit).
- Tests for what nothing tested: a snapshot keeps the workspace's layout and windows, a window
  that closes leaves it, and once the rest are back a restore spends it; a snapshot follows a
  window's new identity and goes with its last window; a scrolling snapshot's round trip keeps
  every camera (a restored workspace keeps its scroll position), the arrangement and the
  settings, and no geometry. Plants 5/5: windows not refreshed after an edit; a forgotten window
  left in its snapshots; a scrolling snapshot without its cameras; a restore without the
  snapshot's layout; a snapshot taken without its windows.

**E3c-2 (2026-11-28): the native memory is a core machine.**
`rift_core::native_memory::NativeMemory<Ws, S>` holds each workspace's snapshot and the reconcile
batch (`Batch::{Idle, Collecting(set)}`); it replaced the engine's `native_layouts` and
`native_reconcile_batch`. Events: `Kept`, `Edited`, `Forgotten`, `Cleared`, `ReconcileBegan`,
`ReconcileEnded` (answers the workspaces that waited), `RestoreAsked` (answers `Deferred`, a
`Plan`, or nothing).
- **A snapshot's windows are computed from its value** (the lead's question): every core value
  knows its windows, so the list `NativeSnapshot` kept beside the value was a second copy kept in
  step on every edit, the shape of the deleted active-floating set. It is gone:
  `SystemSnapshot::windows(layout)` asks the core value, and the machine reads it through
  `Held::windows`. "Which snapshot holds window w" (`holding`) scans the snapshots, which are few
  (one per workspace a departure moved) and small; no index.
- **The machine decides** whether a departure keeps a snapshot (not for a floating window, not
  over one held, not of nothing), when a restore waits for the batch, which of the workspace's
  current windows a restore takes back (from each one's facts: in the workspace, present,
  floating), what it changes (`Plan`: the snapshot's windows to remove, in its order; the windows
  taken back, in the workspace's order; those to add; whether it is spent), and drops a snapshot
  an edit empties. The engine gathers the facts, makes snapshot values and edits them through
  their systems, and applies a plan.
- A forgotten workspace also leaves the batch. Removals follow the snapshot's order (before, a
  hash set's).
- Tests for what nothing tested: a forgotten workspace no longer waits for the batch; a restore
  takes back only the workspace's own windows (one still in its layout but assigned elsewhere is
  left out, and the snapshot is not spent). Plants 11/11: in the core, a floating window's
  departure kept, a second departure replacing the snapshot, an emptied snapshot kept, a restore
  during a batch not deferred, a forgotten workspace still waiting, an absent window taken back,
  a held window added again, never spent; in the engine, a closed window left in the snapshots,
  an identity change not followed, a window of another workspace taken back.

**E3c-3 (2026-11-28): the native memory's Roc twin.** The lead's change to the plan: the Roc
core does not hold each snapshot's windows either (that would bring the deleted copy back on the
Roc side). As the camera takes `Spans`, each event that reads a snapshot's windows carries them,
computed from the values (`NativeMemory::crossing` turns an event into `NativeEvent`): `Kept` and
`Edited` carry the snapshot's windows, `RestoreAsked` the held snapshot's. The Roc core owns only
what it alone owns: which workspaces hold a snapshot, and the batch (`Owned`); it answers a
`Decided` (a plan without the snapshot value). No event needs every snapshot's windows at once.
- `roc/core/NativeMemory.roc`, `RocNative`, and the Boundary types (`Native*`).
- `roc_twin::TwinnedNative<S>` (policy (a)): the Roc core decides, the Rust machine holds the
  values and its decisions are the Roc core's shadow; a Roc restore decision is applied to the
  Rust machine's value of that snapshot. On a disagreement both are brought to what they share
  (the shadow keeps only the snapshots the Roc core holds, and its batch; the Roc core forgets a
  workspace the shadow has no value for), counted (`native_reseeds`, in the run log).
- Differential (`rift-core-roc/tests/native_memory.rs`): the Rust machine holding each snapshot
  as its list of windows against the Roc core fed each event's crossing, 80 000 steps (answers
  and owned state after each), and a copy tripwire; check-fast cuts it to 200 runs.
- Plants 9/9: in the twin, the shadow keeping a snapshot the Roc core lacks, the Roc core keeping
  a workspace the shadow has no value for, a snapshot's windows crossing wrong; in Roc, an empty
  departure kept, an emptied snapshot kept, a forgotten workspace still waiting, an absent window
  taken back, a spent snapshot kept, a restore for a returning window it does not hold.

**E3 is done:** the display memory, the evictions, the cameras and the native memory are core
machines, each deciding in Roc.

### E1-0: an inventory of what `engine.rs` still decides (2026-11-28)

The end-of-E1 target: every function left in `engine.rs` is an **effect**, **glue** or a
**query of core state**; no **decision** stays. After E2, E3 and E4, the file has 119 functions
above its tests: 24 decisions (7 borderline), 35 effects, 22 glue, 38 queries. Classed as E0 was
(each function's own reads and writes, its calls followed), strictly: a function that decides and
acts is a decision, and "(borderline)" marks a judgement call, with why. Since E0, five moved to
effect because their core machine now decides (`remember_native_departure`,
`finish_native_reconcile`, `note_eviction`, `restore_native_layout`, `remap_space`); three E0
effects are decisions under the stricter rule (`commit_workspace_focus`, the two `toggle_*`,
`calculate_layout_with_groups`).

**The decisions, by theme, each with its step** (order: the pieces routing calls first, routing
last; (b) marks a borderline one):

| step | theme | decisions | what moves to the core |
|---|---|---|---|
| E1a | focus choice | `preferred_focus_for_workspace`, `commit_workspace_focus` (b), `move_focus_internal`, `next_space_for_direction` | which window gets focus, whether a focus is kept and where it is recorded, the next Space in a direction: a focus question over the topology and the floats facts. Folded in: `suspend_window` clearing focus, `apply_focus_response` routing by floating, `focus_changes_geometry` |
| E1b | workspace switching | `switch_to_workspace_id`, `navigate_to_workspace`, `ensure_active_workspace_info` (b) | whether and where to switch, the first workspace a Space activates: into the workspaces core, which holds the topology. Folded in: `transfer_workspace` re-laying the source only for an Explicit transfer |
| E1c | placement and relocation | `add_window_to_layout`, `relocate_window_to_workspace`, `relocate_window_with_drop`, `move_window_to_space`, `move_active_workspace_to_space`, `DropPreview::action` (b), `assign_window_with_app_info` | where a window goes (floating or tiled, which workspace and Space, a drop's action, a persistence match over a rule's workspace), answered as a plan the engine applies. Folded in: `apply_window_drop`'s floating refusal, `drop_action_override`, `apply_app_rule_outcome`, `requested_floating_frame` (a pure conversion, moved with it) |
| E1d | displays and the native reconcile | `update_space_display`, `begin_native_reconcile` (b) | which workspaces take a display as home (the display memory or the workspaces core); which moves are departures (the native memory takes the moves as facts). `LastSpace::Hold` stays a computed input until the config slice |
| E1e | floating geometry in the layout pass | `calculate_layout_with_groups` (b) | a floating window's fallback frame (centred when hidden), its fullscreen frame by kind, where a hidden window is parked: a pure function in the floats core |
| E1f | layout mode and settings | `switch_workspace_layout_mode`, `update_virtual_workspace_settings`, `toggle_orientation_for_system` (b), `toggle_stack_for_workspace` (b) | which mode or setting a command or a config change leaves a workspace with. Overlaps the config slice; done here so E1 meets its target |
| E1g | command routing | `handle_command`, `handle_virtual_workspace_command` | which route a command takes, from facts: a `Route` the core returns and the engine carries out over the pieces above |
| E1h | event routing | `handle_event_inner` | the same for layout events. Folded in: `normalize_event` taking the Space from the topology |

**Stays, by the lead's decision:** `update_space_display`'s `saved_space_is_unresolved` input to
`LastSpace::Hold` (persistence is the config slice's).

**Every function** (line numbers at ff957f8c):

| line | function | class | reads / writes | what it decides (decisions only) |
|---|---|---|---|---|
| 46 | `DropPreview::action` | decision (borderline) | - / preview value | the drop action for a same-slot Insert (config centre action instead), from `same_slot` and the action; pure value, could live with the drop machinery as a query |
| 59 | `interval_gap` | glue | - / - | |
| 63 | `requested_floating_frame` | glue (borderline) | - / - | (picks size from a preset and centres on request: a pure conversion of the request's options, no engine facts) |
| 142 | `LayoutEvent::windows_observed` | glue | - / - | |
| 181 | `LayoutEventOutcome::deref` | glue | - / - | |
| 186 | `prepare_scrolling` | effect | W: a scrolling system / - | |
| 241 | `StartingLayout::new` | glue | - / - | |
| 263 | `NativeSnapshot::windows` | query | - / snapshot value | |
| 268 | `NativeSnapshot::of` | glue | - / a workspace's system, layout state | |
| 281 | `NativeSnapshot::updated` | glue | - / snapshot (its system applies the change) | |
| 289 | `NativeSnapshot::restored` | glue | - / snapshot | |
| 308 | `focused_window` | query | - / floats | |
| 310 | `apply_window_drop` | effect (borderline) | W: systems, native, evicted / R: topology, mode | (refuses a drop in floating mode itself: a guard on the mode, else a system edit) |
| 337 | `relocate_window_with_drop` | decision | W: systems, native, floats (focus, positions), topology last focus / R: topology, floats, systems | whether a drop is valid and how it lands: floating (frame) vs tiled (target tree), same vs other workspace, no-op checks; clears focus; from floats, topology, the prospective tree |
| 448 | `drop_scene_windows` | query | - / topology, systems | |
| 472 | `drop_action_override` | query (borderline) | - / topology, systems | (Swap for a stack layout: a fixed rule over the system kind) |
| 485 | `drop_preview_frame` | query | - / topology, systems, constraints, layout_settings | |
| 531 | `prospective_drop_tree` | query | - / systems (on a clone) | |
| 548 | `source_move_neighbors` | query | - / topology, systems | |
| 582 | `query_workspace_layout` | query | - / topology, systems | |
| 609 | `calculate_workspace_layout` | query | - / topology, systems, constraints | |
| 633 | `calculate_arrangement` | glue | - / systems | |
| 652 | `project_arrangement` | glue | - / systems, constraints, layout_settings | |
| 683 | `switch_workspace_layout_mode` | decision | W: systems, floats, native, constraints / R: topology, layout_settings, floats | which windows carry into the new mode (hidden members kept, floating dropped on entering/leaving floating mode), whether widths are preserved, which window is reselected; from mode, floats, settings |
| 758 | `response_for_raised_windows` | glue | - / - | |
| 771 | `toggle_orientation_for_system` | decision (borderline) | W: a system / R: the system | stack-edit vs orientation toggle, from the system's "parent is stacked" answer; arguably the system's own op |
| 786 | `toggle_stack_for_workspace` | decision (borderline) | W: systems / R: systems | unstack, else stack, else raise visible; refocus selection; from the system's answers; arguably the system's own op |
| 823 | `collect_group_containers_for_space` | query | - / topology, systems, constraints, layout_settings | |
| 851 | `set_layout_settings` | effect | W: via apply / R: layout_settings | |
| 859 | `apply_layout_settings` | effect | W: layout_settings, native, systems / R: topology, displays | |
| 873 | `update_virtual_workspace_settings` | decision | W: app_rules, topology, systems / R: topology, settings | which workspaces switch layout mode on a settings change (mode policy unchanged, current != desired, previous != desired) |
| 922 | `active_layout_mode_at` | query | - / topology, systems | |
| 930 | `focus_changes_geometry` | query (borderline) | - / topology, systems | (scrolling/floating/stacked "changes geometry": a fixed rule over system kind) |
| 942 | `layout_specific_animate_settings` | query | - / topology, systems, layout_settings | |
| 959 | `active_floating_windows_in_workspace` | query | - / topology, floats | |
| 974 | `is_floating_active` | query | - / floats, topology | |
| 984 | `preferred_focus_for_workspace` | decision | - / topology, systems, floats | the window focused on activation: preferred, else workspace last focus, else tiled selection, else first visible, else floating last focus, else first floating |
| 1018 | `commit_workspace_focus` | decision (borderline) | W: floats (focus, last focus), topology last focus, systems / R: topology, floats | whether a focus is kept (member of the active workspace and available) and where it is recorded (floating last focus vs system selection); the write-routing alone would be effect |
| 1061 | `activate_workspace` | effect | W: topology / R: (focus via `preferred_focus_for_workspace`) | |
| 1087 | `switch_to_workspace_id` | decision | W: topology / R: topology, settings | already-active target: back-and-forth to previous or nothing, from topology and `workspace_auto_back_and_forth` |
| 1106 | `switch_to_previous_workspace` | glue | W: via navigate / R: topology `previous_target` | |
| 1118 | `navigate_to_workspace` | decision | W: topology / R: topology | a target already shown on another display: focus there without activation vs full activation, from was-active and Space |
| 1150 | `filter_active_workspace_windows` | query | - / topology | |
| 1160 | `filter_active_workspace_window` | query | - / topology | |
| 1169 | `is_active_workspace_window` | query | - / topology | |
| 1182 | `resize_selection` | effect | W: systems, native, evicted / - | |
| 1198 | `apply_focus_response` | effect (borderline) | W: floats, systems, topology last focus / R: floats | (routes the write by floating vs tiled; chooses nothing about which window) |
| 1223 | `move_focus_internal` | decision | W: floats, systems, topology last focus / R: topology, systems, floats | which window directional focus lands on: floating cycle, floating to tiled, tiled in-layout, scrolling stop, next display, fallbacks; from floats, systems, display geometry |
| 1351 | `next_space_for_direction` | decision | - / Space frames (args) | the display in a direction: aligned first, then nearest edge; pure geometry |
| 1420 | `forget_native_layout_for_window_id` | effect | W: evicted, native / - | |
| 1426 | `edit_native_snapshots` | effect | W: native / R: native | |
| 1433 | `forget_native_layout_for_workspace` | effect | W: native, evicted / R: evicted | |
| 1440 | `forget_native_layout_for_window` | effect | W: native, evicted / R: topology | |
| 1447 | `remember_native_departure` | effect | W: native / R: topology, floats, native `keeps` | |
| 1460 | `begin_native_reconcile` | decision (borderline) | W: native / R: topology | which moves count as departures (source Space != target); a fact filter the native core could take |
| 1476 | `finish_native_reconcile` | effect | W: native, systems, evicted / R: native answer, evicted `stale` | |
| 1491 | `note_eviction` | effect | W: evicted / R: topology, displays | |
| 1522 | `eviction_ordinal` | query | - / evicted, displays, topology | |
| 1534 | `take_eviction` | effect | W: evicted / R: evicted | |
| 1540 | `restore_native_layout` | effect (borderline) | W: systems, native / R: topology, floats, native plan | (carries out the core's plan; reselects the focused window if restored, a small rule of its own) |
| 1599 | `suspend_window` | effect (borderline) | W: systems, floats focus / R: topology, floats | (clears focus of a suspended window itself) |
| 1618 | `remove_window_internal` | effect (borderline) | W: systems, floats, topology, persistence / R: - | (`preserve_floating` picks what is dropped, but the caller's event chose it) |
| 1644 | `remove_window_layout_membership` | effect | W: systems, workspace last focus / R: topology, floats | |
| 1677 | `add_window_to_layout` | decision | W: floats, systems, constraints, native / R: topology, floats, layout_settings | floating vs tiled for an added window (floating layout un-floats it), whether its width is preserved, whether native memory restores |
| 1738 | `preserve_scrolling_window_width` | effect | W: constraints / R: window store | |
| 1755 | `remove_window_from_all_tiling_trees` | effect | W: systems / - | |
| 1761 | `workspace_contains_window` | query | - / systems | |
| 1770 | `active_workspace_contains_window` | query | - / topology, systems | |
| 1785 | `projected_space_in` | query | - / topology, systems, floats | |
| 1801 | `space_with_window` | query | - / topology | |
| 1808 | `active_workspace_id_and_name` | query | - / topology | |
| 1817 | `update_space_display` | decision | W: topology homes, displays, systems / R: topology, displays, persistence | which workspaces get this display as home (unplaced ones), Hold vs Follow for the display's last Space (unresolved saved Space) |
| 1857 | `sync_scrolling_widths_for_space` | effect | W: systems / R: displays, layout_settings, topology | |
| 1867 | `last_space_for_display_uuid` | query | - / displays | |
| 1871 | `display_uuid_for_space` | query | - / displays | |
| 1876 | `remap_space` | effect | W: topology, displays, native, floats, persistence / - | |
| 1897 | `forget_workspace` | effect | W: native, floats, persistence / - | |
| 1903 | `LayoutEngine::new` | glue | - / - | |
| 1924 | `apply_app_rule_outcome` | effect (borderline) | W: floats, app-rule outcome / R: rule effects | (the rule effects decide float and frame; the engine maps them, incl. un-floating a was-floating window) |
| 1947 | `apply_app_rule_resize` | effect | W: constraints, systems / R: topology, layout_settings | |
| 1985 | `debug_tree` | glue | - / - | |
| 1989 | `debug_tree_desc` | glue | - / topology, systems | |
| 2020 | `normalize_event` | glue (borderline) | - / topology | (a WindowAdded's Space becomes its assigned workspace's: answered by the topology) |
| 2035 | `handle_event` | glue | - / - | |
| 2046 | `handle_event_inner` | decision | W: floats, constraints, systems, topology, native, evicted, persistence / R: topology, floats, systems, layout_settings | the event router: claimed windows ignored; an observed window's floating vs tiled, relocation, scrub, reselection, and workspace-focus request vs commit; WindowFocused's floating vs tiled write and selection preservation; AppClosed and resize handling |
| 2438 | `handle_command` | decision | W: native, scroll_boundary, floats, systems, topology / R: topology, floats, systems, layout_settings | the command router: which commands forget native memory, floating toggle (re-add vs remove, frame), floating fullscreen, ToggleFocusFloating's target, Next/PrevWindow's cycle set and target, Ascend refusal when floating, MoveNode's cross-display spill and order, resize refusal when floating, scroll boundary accumulation |
| 2995 | `calculate_layout` | effect | W: systems / R: topology, constraints, shadow_passes | |
| 3021 | `calculate_layout_with_virtual_workspaces` | glue | - / - | |
| 3047 | `calculate_layout_with_groups` | decision (borderline) | W: systems, floats positions / R: topology, floats, constraints, layout_settings, shadow_passes | floating frames: stored frame unless hidden, else centred on the screen; fullscreen frames by kind; hidden windows' parking; the layout pass itself is effect |
| 3221 | `active_workspace_for_space_has_fullscreen` | query | - / topology, systems | |
| 3234 | `workspace_has_fullscreen` | query | - / topology, systems, floats | |
| 3253 | `collect_group_containers` | glue | - / - | |
| 3265 | `logical_window_frames` | query | - / topology, systems, constraints | |
| 3294 | `calculate_layout_for_workspace` | query | - / topology, systems, floats | |
| 3320 | `selected_window` | query | - / topology, systems | |
| 3329 | `handle_virtual_workspace_command` | decision | W: topology, floats focus, systems / R: topology, floats | the workspace command router: next/prev and "next"/"prev" names, which window a move takes (index or focused), no-op on same workspace, follow vs stay, focus after a move (moved window, first remaining, none), raise/focus after SetWorkspaceLayout |
| 3597 | `workspaces` | glue | - / topology | |
| 3599 | `workspaces_mut` | glue | W: topology / - | |
| 3601 | `assign_window_with_app_info` | decision | W: topology (assignment, manage override), native, persistence / R: app_rules, persistence | a claimed window only gets its manage override; a persistence match cancels the rule's workspace; a reapplied workspace rule forgets native memory |
| 3677 | `ensure_active_workspace_info` | decision (borderline) | W: topology / R: topology | a Space with no active workspace activates its first (index 0); a default the topology could own |
| 3693 | `move_window_to_space` | decision | W: topology, systems, floats focus / R: topology | the destination workspace (target Space's active one) and whether the moved window is focused (placeable) |
| 3755 | `move_active_workspace_to_space` | decision | W: topology, systems, native, evicted / R: topology, displays, floats | per-display mode: move windows into the same-index workspace there (and focus), else transfer the workspace object |
| 3828 | `app_rule_names_workspace` | query | - / app_rules | |
| 3834 | `transfer_workspace` | effect (borderline) | W: topology, native, systems, displays, evicted / R: topology, floats | (an Explicit transfer re-lays and re-places the source Space; transferred windows only across Spaces) |
| 3884 | `ensure_workspace_layouts` | effect | W: topology, systems / - | |
| 3894 | `relocate_window_to_workspace` | decision | W: topology, systems, floats, native, evicted, constraints / R: topology, floats, systems, layout_settings | whether a relocation is allowed (ownership, Space), whether the window stays floating (policy, floating layouts, direction), width preservation, suspend vs resume, focus clear |
| 3999 | `workspace_name` | query | - / topology | |
| 4011 | `is_window_floating` | query | - / floats, systems | |
| 4020 | `prepare_window_placement` | effect | W: floats or floating system / R: topology, floats | |
| 4033 | `store_floating_position` | effect | W: floating system or floats / R: systems | |
| 4046 | `get_floating_position` | query | - / systems, floats | |
| 4057 | `workspace_floating_positions` | query | - / floats | |
| 4064 | `remove_floating_position` | effect | W: floats / - | |
| 4070 | `retire_window` | effect | W: constraints / - | |
| 4076 | `constraint_records` | query | - / constraints | |
| 4084 | `observed_size` | glue | - / - | |
| 4101 | `has_window_constraints` | query | - / constraints | |
| 4105 | `transfer_persistent_window_identity` | effect | W: evicted, native, systems, topology, floats, constraints / R: floats | |
| 4129 | `store_floating_window_positions` | effect | W: floats or floating systems / R: topology | |

**The lead's review (2026-11-28):** approved, strict classing kept. **E1f stays in E1:** mode
switching and the toggles are behaviour, and the engine target needs them. **The config slice
keeps what is left:** parsing and interpreting config files into typed settings, and
`active_layout_mode_at`. The order stands: focus, workspace switching, placement, displays,
floating geometry, mode, then command and event routing. After E1e, judge whether the named
follow-up (Geometry as a pure core function the camera reads, removing the `Spans` round trip)
is small enough to do there; say which in the E1e report. E1c's placement plan is a sum type per
kind of outcome (a plain add, a relocation, a drop with a target), never one struct with optional
fields.

**E1a-1 (2026-11-28): the engine's focus choices are core functions** (`rift_core::focus`,
beside the reactor's focus decisions), the engine gathering facts and carrying out the answers:
- `workspace_focus` (was `preferred_focus_for_workspace`): the window asked for, else the last
  focused, else the layout's selection, else its first visible window, each only if it belongs
  (in the workspace and available), else the last floating focus if still active, else the first
  active floating window. The facts are `WorkspaceFocus` (each candidate an `Eligible`).
- `focus_record` (`commit_workspace_focus`, `apply_focus_response`): `FocusRecord::{NoWorkspace,
  Cleared, Floating, Tiled}`, where a committed focus is recorded; the two callers' effects for
  each answer are as they were.
- `floating_cycle`, `past_the_edge` (`PastTheEdge::{Stop, NextSpace}`: a strip ends left and
  right), `entry_window` and `focus_fallback` (`move_focus_internal`'s choices; the layout system
  still answers the directional focus itself), and `space_in_direction` (was
  `next_space_for_direction`, the same arithmetic).
- `FloatsEvent::FocusLeft(w)`: a window that is suspended or removed takes the focus with it only
  if it held it, in the floats core (Rust and Roc) instead of the engine comparing and clearing.
- `focus_changes_geometry` is reclassed as a **query**: it asks each layout system about itself
  (its kind, whether its selection is stacked); a core function would only wrap one `or`.
- Tests for what nothing tested: a workspace's focus skips an unavailable window, even asked
  for; a focus committed to another workspace's window is cleared. Plants 11/11: in the core, the
  last focused before the window asked for, a floating last focus no longer active, a floating
  window recorded as tiled, a focus that does not belong kept, the floating cycle reversed, a
  strip stopping vertical focus, the nearest Space before the best aligned, the fallback order,
  focus leaving any window clearing it; in the engine, an unavailable window taking a workspace's
  focus, another workspace's window kept.

- **The gate's second catch, and a flaw it showed:** the rewrite added a third needless
  `&window_store.core` (one per recorded branch); fixed, with the two others in the same code, so
  the baseline shrank to 291. The gate keyed every `pub(crate)` item as "pub" (it cut the line at
  the first parenthesis, the visibility's); it now strips the visibility first. The baseline was
  re-keyed once (24 entries got their item's name; the count is unchanged by the re-key).

- Next: E1a-2 (the focus choices in Roc, deciding).

**Pruning (the lead, 2026-11-26):** at a milestone, delete only what no future build reuses:
stale `target/roc-build` keys (all but the current sources', which the build script's line
names), plant and bisect variant builds, scratch outputs. Keep `target/debug/incremental` (it
serves the next build); a full `cargo clean` only when disk is tight.

**Build times (2026-11-26).** An optimized Roc build (`--opt=speed`) takes about 50 s every
time: Roc caches its front end (every module a cache hit) but runs LLVM on the whole program,
so it has no incremental reuse. The build script ran it on every rerun, and each build-script
unit (check, test and clippy profiles; the `roc` and `roc-core` feature sets) has its own
`OUT_DIR`, so one `.roc` change cost four builds in a check run. `build.rs` now keys the archive
and glue by a hash of their inputs (the Roc sources' paths and bytes, the compiler, the glue
spec, the arguments), builds each key once under `target/roc-build/<key>/` and copies it into
`OUT_DIR` only when the bytes differ. A check run after a `.roc` change: 422 s to 285 s (one
Roc build); after nothing: 198 s to 152 s; after a Rust-only change: 277 s to 203 s (what is
left is test running and relinking). `target/roc-build` keeps every key built (about 8 MB
each); `cargo clean` or deleting it is safe. The `--opt=dev` backend builds the archive in
under a second, but miscompiles rift: rejected (above, "The dev backend").

**Roc uniqueness: the node-table copies still open (2026-11-26, fix 2 of the lead's three Roc
cost fixes).** A step that copies the forest's node table pays O(n) per event (about 112 bytes
a slot: 150 KB at 1000 windows). The rift-core-roc tripwire
(`the_roc_*_system_steps_without_copying_a_table`: a 400-window forest through 3000 drawn
steps, failing on a fresh allocation of 64 bytes a slot or more; `take_largest_allocation`, with
`count-allocations`) holds every step but the known copies, which are stepped and exempt
(`Twin::known_copy`) and checked by four ignored tests (`--ignored`: two drawn, every step
checked; two scripted). Fixed: remove, remove-and-rebalance and close_app (our ordering:
`delete_subtree`'s `NotHeld` branch returned the original forest after its table went to
`Slots.remove`). Still copying, on the pinned nightly and on nightly-2026-11-25-d5a4f1e:
traditional `AddWindow` toward a direction; master-stack `CreateLayout`; the forest's
`RemoveLayout` and `ReplaceWindow`; the edits `Drop`, `Move`, `Join`, `ConsumeOrExpel`
(`Rebalance` fixed since: its sizes now read in one walk, then written in a fold whose state is
the forest alone, where the change flag rode beside it). The bisect of the directional add (13 builds, each variant of
`traditional_add_window` / `add_window_toward` probed for its largest allocation on a 400-window
forest), for whoever picks up the repro (the lead's option (b)):
- no copy: `add_window_under(forest, root(..), w)` then `select_node`; the same with the root
  bound first; with `(nested, at) = (forest, top)` before it; with a `children(..).is_empty()`
  or `container_kind(..)` read before it; `set_kind` (or `set_fill`) on the root alone;
  `set_fill` then `create_root`; `create_root` then `set_window`; the end-of-tree add
  (`add_window_after_selection`, the same `add_window_under` to the root);
- copy: `set_kind` or `set_fill` on the root, then `insert(_, Back(root))` (`placement_parent`,
  `create_root`, `link`); the full directional body with any of `set_kind`, the nesting branch
  or the final move removed; taking the forest as `system.forest` in the caller or as `system`
  inside; reading the stamp before the record update in `set_window`; reading `Back`'s index
  without the children; `Back(key) => Some(key)` without `holds_node`;
- an earlier variant (the root bound, `add_window_under`, no select) copied where an otherwise
  equal one did not, so the cause looks like Roc's refcount and borrow inference rather than an
  ordering in the core; not reduced outside rift yet.

**The copy hunt's second round (2026-11-26, time-boxed to 3 hours, the lead's option 1).**
- **The host is not the cause.** Read from the host just before `roc_traditional_tree_step` on
  a 400-window forest: the model box at refcount 1, the node and layout slot lists at 1, on
  every probed step, the copying ones (the directional add, 90 KB; `Rebalance` and
  `ReplaceWindow`, 60 KB) included. The sharing starts inside the Roc step.
- **The copies are the first write to the shared table:** `Slots.put` under `Slots.insert`,
  `remove` or `replace`, reached through `set_node`, `create_root` or `delete_subtree`.
- **How to trace one** (no compiler support needed): build the Roc archive with
  `roc build --debug`, have the host's `roc_alloc` print `std::backtrace::Backtrace::force_capture()`
  for any allocation over a threshold (64 bytes a slot), and name the hashed `roc__p<hash>`
  frames from `dwarfdump --debug-info` on the archive (`DW_AT_linkage_name` to `DW_AT_name`).
- **One shape copies in a standalone app** (a boxed model, the host counting allocations):
  a field bound to a local while its record stays live for a later update. `nodes =
  forest.nodes` then `{ ..forest, nodes: { ..nodes, slots: put(nodes.slots, ..) } }` copies the
  list on every step; the same update written inline through the records, or after taking the
  records apart, does not. That is Roc's sharing as documented (the record still holds the
  field), not a bug. rift's Roc has that shape only at the system level (`forest =
  system.forest`, then `{ ..system, forest: .. }`: `traditional_add_window`,
  `add_window_after_selection`, `smart_window_insertion`, `traditional_edit`,
  `insert_window_in_container`, `promote_to_master`, `move_target`), and taking the system apart
  instead (the parked refactor) left the copies where they were and made the plain add copy.
  Not the forest helpers or `Slots`: they project a field straight into a call.
- **Did not copy standalone**, each built to rift's shape: a crash branch before the forest is
  consumed; a fold whose state is a tuple holding the forest; rift's recursive rebalance;
  rift's `Slots` module verbatim, with tag-union slots holding records of lists; `Set` and
  second-table fields beside the table; `set_size`'s branch returning the forest unchanged; a
  step taking the system apart, then calling helpers that take the forest and the settings.
- **Inside rift:** splitting the edit branch out of `traditional_step` changed nothing.
  `Rebalance` stopped copying when its fold's state became the forest alone (fixed, above); the
  same tuple-state fold does not copy standalone.
- **Reading the compiler's decisions:** the compiler has `ROC_LIR_DUMP` (the final LIR of every
  procedure whose name contains the value) and `ROC_VERIFY_ARC`, but only in a compiler built
  with developer diagnostics (`-Ddev-diagnostics`, every Debug compiler); the nightly is a
  release build and reads neither, and there is no Zig toolchain here (the source asks for
  0.17.1). Next step for whoever picks this up: build a diagnostics compiler at the pinned
  nightly's commit and dump `GroupForest`'s write path for `Rebalance`'s old shape and the
  directional add. A question upstream, should one be wanted (the owner's call, not asked):
  "A record update on a uniquely held model copies a list field in one call graph and not in
  an otherwise equal smaller one; is there a supported way to see the inferred borrow and
  ownership of each procedure in a release build, or should we build with dev diagnostics?"
- **Crash branches on the write path**, as the lead asked (a defensive branch for a state meant
  to be impossible; none was implicated by the bisect):
  - `Slots.put`'s `OutOfBounds`: the index was just read from the same list. A type could
    remove it only with a list update that cannot miss (none in the builtins); kept.
  - `Slots.insert`'s free slot not vacant: the free list holds only vacated slots. A type can
    remove it: the free list carrying each slot's next version (`List({ index, version })`)
    makes the read, and the branch, unnecessary.
  - `set_node` (and `node`, `set_root`, `root`): a key the table does not hold. Keys come from
    the forest itself; a type tying a key to its table is not expressible here; kept.
  - `insert`'s region-pair parent and its invalid placement (`placement_parent` finding no
    parent), and `link`'s unplaced sibling and missing index: a type can remove them, by
    `insert` taking a resolved place (a regular container's key and an index) instead of a
    `Placement` it resolves and may fail to.
  - `listed_children`'s region pair: the same as `insert`'s (a regular container's key).
  - `create_root`: none. `delete_subtree`: none (its `NotHeld` returns the forest since fix 2).

**A window's place by type (2026-11-26, the lead's go).** A window node is always a container's
child, by type: `Body::Window { at: Child, window, stamp }` and `Body::Container { place, container
}`, `Place` (`Root | Child(Child)`) now only a container's (`GroupLeaf`, `GroupContainerNode` at
the boundary, alike in Roc). A leaf is created where it goes (`insert_window`, Rust and Roc),
never as a root then filled in; `unlink` no longer makes a node a root (every caller links the
node again or removes it, so its place is not rewritten in between). Deleted: the crashes in Roc's
`drop_stacking` (the target leaf's place now comes with it from `window_leaf`) and
`smart_window_insertion` (it takes the chosen node's place: the selection's own, or, for a root
selected, its chosen child's from `chosen_child`), and Rust's "no parent" defaults on the same
paths (the drop's `map_or(Horizontal, ..)`, smart insertion's `After(root)`). The load: a window
saved as a layout's root is wrapped in a new root container of the default kind that selects it
(`{system}-load: a window as a layout's root; wrapped in a container`; upstream kept it and
crashed at the next insert beside it), and a parentless window no layout reaches is dropped, its
slot freed (`{system}-load: a window no layout reaches; dropped`); both pinned by fixtures in
the traditional system's tests. Plants 9/9, among them one that survived until a test of
upstream's nesting beside a crowded split's selection was added (it had none outside the
differentials).

**Floating onto the core forest (2026-11-26, the lead's go; steps 1-2 of 5).** Step 1 put the
floating system in the core (`group_forest::FloatingSystem`, its frames a `Geometry` union);
step 2 holds it to upstream's `FloatingLayoutSystem` as a shadow (`systems/floating_twin.rs`),
upstream still deciding; step 3 flipped it (the core decides and is saved, upstream's system
the shadow); step 4 ported it to Roc and, under `roc-core`, made the Roc system decide (the
core's its shadow, upstream's the core's until deleted); step 5 deleted upstream's system,
`groups.rs` and the re-seed helpers (done 2026-11-26). Rulings and notes:
- **"Byte-compatible", precisely:** the core's save equals upstream's byte for byte except the
  order of the frames, which the core writes by window (upstream's `FxHashMap` iterates in an
  order its history decides); both orders load the same. The core's saves are deterministic,
  an improvement (the lead's ruling). Frozen: `systems/floating/*.loaded.ron`.
- **Step 4, the Roc port (2026-11-26):** floating is a third system in `GroupForest.roc`
  (`Floats { forest, geometry : Dict(U64, FloatingGeometry) }`, its own `FloatingEvent` and
  `FloatingAnswer`, `roc_floating_tree_*`, `RocFloating`), held to the Rust core by
  `rift-core-roc/tests/floating.rs` (300 runs of 120 drawn steps: answers and the whole view,
  frames included, every step; the shape, bars, container view and rows on 40%; reloads; queries).
  **Known copy, open:** the floating step copies the node table on nearly every write to the
  forest (adds, removes, replaces, closing an app, layouts made, cloned or removed, selections,
  frames stored, every edit; participation, which writes only the windows left out, stays in
  place; queries allocate nothing table-sized). The same forest functions run in place under the traditional system (a probe: 400
  adds and clones at 192 bytes, against 67-135 KB here), so the sharing comes from the floating
  step's context, not the forest's code. The tripwire
  (`the_roc_floating_system_steps_without_copying_a_table_or_its_frames`, 64 bytes a node slot,
  32 bytes a window for the frame map once it is large) exempts those steps (`known_copy`) and
  holds queries and participation; the
  ignored `..._even_on_known_copies` checks them. Tried without effect: taking the system apart
  in the step, the traditional step's exact shape, no closure holding the forest, the add taking
  only the forest, taking the table apart before `Slots.insert`'s append, and `create_root`
  taking the forest apart.
  The time-boxed hunt after it (1 hour, 6 builds) narrowed it without explaining it: at
  `roc_floating_tree_step`'s entry the box and the node slot list are each held once (refcount
  1), so the sharing starts inside the step; the frame map is not it (the `Dict` swapped for a
  list, the same copies); and the traditional system's own `forest_step`, reached from
  `floating_step`, copies on a selection where the same call under `traditional_step` stays in
  place (a selection and a clone at 192 bytes and 3 KB there, 45 KB and 67 KB here). What decides
  is the floating step's call context, not any function's code: the open traditional and
  master-stack copies have the same signature (refcount 1 at entry, a copy inside, sensitive to
  the caller). The next step is still the compiler's ownership decisions (a dev-diagnostics
  build's `ROC_LIR_DUMP`, see above).
- **Deleted in step 5, with upstream:** `model/slot_table::to_slotmap`, `NodeMap::of_slots`
  and `group_stored::upstream_tiles` (restored only to re-seed the floating shadow from the core
  with the same slots, versions and free lists).
- **`Geometry::Restorable` stays** (a frame to return to and none kept): upstream reaches it
  only by loading its older file form (`independent_frames` without `frames`), and saves stay
  byte-compatible. A candidate for normalizing at load (to `Kept` of the restore, or dropped)
  once upstream compatibility is no longer pinned.
- **A stale frame to return to is kept, as upstream keeps it:** a swap or a tab reorder can move
  an aligned window out of its group with its `restore`; a later stack or unjoin then uses it.
  Likely an upstream quirk; if it is fixed, as a deliberate behaviour change, not in the port.
- **Reachable only from a file (the shadow cannot reach them; the core's unit tests hold
  them):** a stack inside a stack (floating's own operations nest a group only at the root, and
  join only leaves), so "the outermost stack shares a stored frame" and "the innermost" agree on
  every state floating builds; an aligned window aligned again into a new stack (its remembered
  frame kept), and a stale restore on a window joined into a split (forgotten), which need the
  stale-restore path above.

**The 1000-window perf track, closed (2026-11-26, the lead's ruling).** At real sizes (10-30
windows) a Roc step or pass costs a few microseconds; the track's job was removing shapes that
allocate O(n) per event, and it stops here. Done: the walk without materialising, the sampled
view comparison, Rebalance's read-then-write, the axis solver's folds, participation marked by
slot, the walk reading each node once (each commit has its table). Kept as regression guards:
the tripwire (`the_roc_*_system_steps_without_copying_a_table`) and its ignored known-copy tests.
Open leads, not pursued:
- **Rebalance at 10 windows** measured 0.97 -> 1.73 us across its read-then-write commit, then
  1.05-1.09 us in two later runs of the same harness (100 and 1000 windows improved); not
  re-measured with more iterations. If real, the extra walk for the answer is the suspect.
- **`reveal` in a directional focus move:** 62% of a 36 us move at 1000 windows (Rust 2.1 us),
  with no table copied (largest allocation 192 bytes); the sample flattens Roc's frames, so
  nothing inside `reveal` is attributed yet. Next step: an instrumented look (allocation trap or
  a split of `reveal` into named steps) before any fix.
- **The scan for a window** (`window_node`, used by swap, contains and focus) costs about 22 ns a
  node against Rust's 4: the copy of the node record out of its slot on every read.
- **The node-table copies** still open (above): directional add, master-stack `CreateLayout`,
  `RemoveLayout`, `ReplaceWindow`, `Drop`, `Move`, `Join`, `ConsumeOrExpel`.

**The group forest, slice 5, done (2026-11-26): upstream's traditional and master-stack systems
deleted.** Three commits after the two of the lead's tasks below; each message has its plants,
checks and costs.
- **The loads build the core's forest** (`systems/group_stored.rs`, as the stack's `stored.rs`):
  the saved forest (upstream's `GroupLayouts` form, read through the old tree's deserializer) is
  normalized and made into the core's forest; master-stack's previous forms (regions saved beside
  the forest, a root of two containers, a plain traditional forest) are migrated on the parsed
  tree step for step as upstream's load did, with the default settings (none are saved; a
  traditional load starts with the default insertion point and halving the selection, as
  upstream's did). Three normalizations are now only their warn (both fullscreen flags, a
  window's kind, a root's size: the core cannot hold them); the window index is normalized as
  the leaves are stamped (saved order, stray entries dropped, a repeated leaf keeps its first
  place, missed leaves after in layout and tree order). Fixtures: `traditional_saved.ron`,
  `master_stack_saved.ron` (bytes, settings, literal allocations), the three previous master-stack
  forms with their loaded bytes frozen as upstream's load gave them. Plants 13/13.
- **What the shadow held is held literally:** rift-cli's drawing (`traditional_drawn.txt`,
  `master_stack_drawn.txt`, each ending with its own newline) and container view, the group bar
  (every bar and the selection path's; none on the path while the selected window fills), each
  fill's frames. (A first cut of this commit held recording prints, not assertions; it was
  amended before publishing.)
- **Deleted:** both `Upstream`s with their geometry and legacy loads, `tiles.rs`, the
  `GroupLayouts` methods and `constraints.rs` / `model::tree` helpers only they used,
  `to_slotmap`, the shadow plumbing and the `group-forest-divergence:` counter with its `rift-cli
  metrics show-timing` line. `traditional/twinned.rs` and `master_stack/twinned.rs` are
  `system.rs`; `group_twin.rs` is `group_system.rs`. The three trees share one decision:
  `decide_tree` (plain) and `decide_roc_tree` (`roc-core`), replacing `decide_stack(_tree)`,
  `decide_group_tree` and `decide_traditional` (whose last user, `core_geometry`'s divergence
  test, holds the shared `decide`). Upstream's unit tests are retargeted to the systems (sizes set
  on the core's forest with `with_sizes`); `stack_legacy.ron` is written by the traditional
  system and its tripwire passes. Plants 11/13, two equivalent (in the commit).
- **Upstream tests dropped rather than retargeted, and what holds their ground now** (the
  lead's ask): `tiles.rs`'s `small_positive_weights_stand_as_upstream_reads_them` is kept as a
  core-against-literal test in `traditional.rs` (same widths); `tiles.rs`'s
  `the_core_weighs_an_axis_as_upstream_does` (5000 drawn axes of zero, negative, NaN, infinite and
  huge weights, core against upstream) by `degenerate_weights_share_the_axis_as_upstream_read_them`
  (literal widths for each kind; the same scene run through upstream's projection at the
  revision before upstream was deleted, with throwaway code, gives the same widths for each
  kind, and for the small weights) and, Rust against Roc, rift-core-roc's
  `the_roc_core_tiles_traditional_layouts_as_the_rust_core_does` (the same weight draws);
  both systems' `a_query_answered_otherwise_is_counted_and_fails_tests` and
  `a_group_forest_divergence_is_counted_and_fails_tests` held the shadow's counter and report,
  which are gone (nothing left to hold); `measure_*_twin` became `measure_*_system`; the legacy
  half of master-stack's `region_identity_and_grouping_survive_removal_and_settings_reload` (the
  regions saved beside the forest, master on the right) by `the_previous_forms_load_into_regions`.
  Every other upstream test was retargeted, not dropped.
- **What the forest is held by now:** the rift-core-roc differential (Rust against Roc), the lib's
  traditional and master-stack tests and drawn fuzzers (under `roc-core` every operation held to
  the Roc forest), the fixtures and literal tests.
- **Cost (release, `measure_traditional_system` / `measure_master_stack_system`, 10 / 100 / 1000
  windows):** a plain build pays only the core: traditional 0.28 / 1.1 / 11.8 us an operation,
  1.9 / 15 / 126 us a pass; master-stack 0.36 / 1.7 / 11.2 us, 2.3 / 14 / 59 us. `roc-core`:
  traditional 2.8 / 15 / 208 us an operation, 7.8 / 67 / 739 us a pass; master-stack 4.5 / 11 / 82
  us, 13 / 46 / 382 us.

**The lead's tasks before slice 5 (2026-11-25):**
- **The reach gap closed:** a scripted case in the rift-core-roc group forest differential
  reaches a hierarchical join moving Down into a vertical container
  (`a_hierarchical_join_down_into_a_vertical_container_takes_its_first_child`); Roc's
  `opposite(Down)` planted as `Down` is caught, and the same plant in `rift_core`. Drawn runs
  stay blind to it.
- **Two Roc glue problems from slice 4, investigated** (drafts in "Roc issue drafts" under the Roc
  section): (a) a new bug, not roc-lang/roc#13348 returning: a one-tag union's generated type
  alias names the element of its `List` or `Box` payload; (b) `RocList<T>::decref` being shallow
  is documented, deliberate behaviour (the deep `decref_list_of_…` helper exists), drafted as a
  sharp edge only. Both checked at the pinned and the newest nightly (`nightly-2026-11-25-d5a4f1e`).

**The group forest, slice 4, done (2026-11-25): under `roc-core` the Roc forest decides.** Three
commits after the slice 3 rulings' docs commit (the first, the stamp counter carried, is below);
each message has its plants, checks and costs. The lead's rulings: no zip pairing (Roc's
placement answers each stack's key with its frame); the drawing as rows (Roc decides structure
and content, the platform writes ids and floats as text).
- **The Roc geometry is labelled** (`Traditional.Tree(s)`, `Traditional.Laid(s)`, as Rust's
  `Traditional<W, S>`): `Traditional.placed` answers each stack's label with its frame beside
  the windows' frames, from the one function that places them. The platform's own shape (the
  `traditional` entry) and the stack system's label with `{}`. `window_id` and
  `known_constraints` are `Geometry`'s (were written out in `BspTree` and `StackTree`).
- **`GroupForest.shape`, `project`, `container_view`, `rows`** (`Forest::shape`, `project`,
  `container_view`, `rows`, one for one), behind one entry per system
  (`traditional_tree_layout`, `master_stack_layout`: `GroupLayoutQuestion = Frames | Project |
  Container | Draw`, `GroupLayoutAnswer`). The glue flattens a one-tag union and mis-describes
  its payload, so the union is whole from the start. Rust: `RocTraditional` /
  `RocMasterStack` `frames`, `project`, `container_view`, `rows`; an answer is read by kind and
  released whole (a glue list's `decref` is shallow; the generated union `decref` is deep).
- **`Forest::draw` is `Forest::rows` rendered by `group_forest::render`**, one renderer for the
  Rust and the Roc rows (`Row`, `Branch`, `Mark`, `Drawn`).
- **The differential** gains a layout pass on 40% of steps: frames bit for bit, bars, views and
  rows by value and written form; 191,276 passes identical at 3000 runs per system (about 30 s).
  Plants 27/29 (two equivalent).
- **The decision** (`systems/group_roc.rs`, `RocForest` over a `Machine` per system): under
  `roc-core` every answer, frame, bar and view is the Roc forest's; the Rust forest is its
  shadow, re-seeded from the Roc view (`Machine::core_of`, `ForestView::forest_on`,
  `Node::map`); upstream stays the Rust forest's shadow; saving stays from the Rust forest. The
  answer parsers are the BSP tree's (`bsp::roc_tree`, the same `Answer`). `decide_group_tree`
  beside `decide_traditional` (which upstream's own systems and a test still use). Plants 13/14.
- **A reach gap found (slice 2's differential):** Roc's `opposite(Down)` planted as `Down`
  survived the lib and the rift-core-roc differential: only a hierarchical join moving Down into
  an aligned container reads it, and neither drawn run reaches one. Closed since by a scripted
  case (above).
- **Cost (release, `roc-core`, `measure_traditional_roc_forest`, 10 / 100 / 1000 windows):** the
  Roc step 0.78 / 6.4 / 105 us, the view comparison 0.36 / 2.3 / 22 us, a Rust re-seed 0.37 / 1.7
  / 14 us; a Roc layout pass 7.6 / 55 / 624 us against the Rust shape and geometry 2.7 / 15 / 134
  us (a `roc-core` pass pays both). Both Roc costs are for after slice 5 (the lead).
- **Next: slice 5:** delete what traditional and master-stack use of upstream's `GroupLayouts`
  (their `Upstream` systems, the shadow plumbing), the legacy load moved to the core side first
  (a `group_stored` load building the core, fixtures byte for byte, as the stack's
  `stored.rs`); floating keeps `GroupLayouts` until its own turn; `stack_legacy.ron` must keep
  passing.

The first slice 4 commit: **the stamp counter is state** (the lead's ruling on slice 3), carried in
`ForestView` and the Roc seed, stamps compared by value (`Forest::from_parts` takes it,
`Forest::from_leaves` derives it for upstream's counter-less form). Plants 7/7. Also ruled: the
test-only frames-with-groups path stays as upstream has it (an upstream candidate).

**The group forest, slice 3, done (2026-11-25): the flip (policy (a)).** Five commits after the
slice 2 docs commit; each message has its plants, checks and costs.
- **rift-cli's views read the core** (`group_forest::views`): `Forest::container_view(layout)`
  (each node, whether it is the selection, its size among its siblings, its fill, its `Role`:
  the root region pair's master or stack, else `Plain`; a window or a container's kind and
  children), mapped by the platform to `ContainerTreeNode`; `Forest::draw(layout)`, upstream's
  `draw_tree` with the `ascii_tree` writer ported (rift-core has no dependencies). Plants 9/9.
  The drawn fuzzers' reloads now also write a negative child size (a saved-only state).
- **The group bar reads the core:** the traditional geometry's shape labels each stack
  (`Traditional<W, S = ()>`; `Traditional::placed` answers each stack's frame beside the
  windows'); `Forest::project(layout, pass, constraints, Bars)` gives the frames and each
  stack's `GroupBar` (frame, one window per child taking part, the focused child's index) as
  upstream's projection walks (`Bars::SelectionPath` follows the selection; a filled node ends
  the walk and, on the selection path, drops the bars found). A group bar pass's frames are
  decided as `calculate_layout` decides them. Plants 9/9 (five equivalent by construction,
  listed in the commit). Found on the way: rift's only caller passing a group output to
  `calculate_layout_with_groups` passes none, so the emit-frames path is reached only by tests;
  the bar itself comes through `collect_group_containers`.
- **The flip** (`traditional/twinned.rs`, `master_stack/twinned.rs`): `{ core, shadow:
  Box<Shadow { upstream, steps, roc (roc-core only) }> }`. Every answer is the core's; upstream
  steps in every build, compared as before, and is re-seeded from the core on a difference
  (`upstream_of`: `group_stored::upstream_tiles`, slotmaps through `model::slot_table::to_slotmap`,
  restored and now writing each value in the saved form of the slotmap's type; master-stack's
  settings written over the old ones). `calculate_layout`: the core geometry's frames for the
  core's shape (`decide_traditional`), upstream's projection the shadow. **Saving is the
  core's** (`group_stored::Saved`: node slots with their links, sizes, kinds, fill flags,
  selections, windows and region order; the index in stamp order; master-stack's under
  `inner`), held to upstream's bytes at every drawn reload. Loads still go through upstream's
  load and the slice 1 normalizations. Plants 12/12.
- **Under `roc-core`, the Roc forest is a second shadow** (`systems/group_roc.rs`, `RocShadow`
  over `RocTraditional` / `RocMasterStack`): seeded from the core, stepped with every operation
  and query, answer and whole view compared; a difference counted (`roc-core: group forest
  re-seeds`), panicking in tests, or warned `roc-divergence:` and re-seeded from the core. **The
  views compare entry stamps by rank:** a seed derives the next stamp from the leaves, while a
  live forest keeps counting past leaves since removed, so a Roc forest seeded from a live core
  stamps lower; only the order is behaviour. (Carrying the counter in the view would be the
  alternative; the slice 4 decision flips which side seeds which, so worth a look then.) Plants
  11/11 (two equivalent).
- **The shape, optimized:** the nodes taking part were a `BTreeSet<N>`; they are a
  `rift_core::slot_table::SlotSet` (keys by slot and version). The core's shape at 10 / 100 /
  1000 windows: 1.1 / 9.5 / 159 us before, 0.78 / 5.1 / 45 us after (upstream's 0.85 / 5.5 / 50
  us); master-stack 1.1 / 4.8 / 55 us before, 0.77 / 2.4 / 14 us after (upstream's 0.74 / 2.8 /
  22 us). An allocation-free `participating_child` was tried and measured no gain; left out.
  Plants 3/3.
- **Cost (release, after the slice):** a plain build pays the core (0.24 / 1.0 / 11 us an
  operation; master-stack 0.35 / 1.0 / 8.6 us) and upstream as the shadow (83 ns / 0.24 / 1.9
  us; 86 ns / 0.22 / 1.4 us), the view 1 in 32, a shadow re-seed 58 us / 0.55 / 5.6 ms.
  `roc-core` adds the Roc step (0.76 / 6.4 / 101 us at 10 / 100 / 1000 windows in one layout),
  a view comparison (0.46 / 2.9 / 25 us) on every operation. **The 101 us is a known cost to
  revisit after slice 5** (the lead).
- **Next: slice 4, under `roc-core` the Roc forest decides:** `Forest::shape` in Roc (words to
  `WindowId` at the boundary) with a frames entry and a differential; then the Roc forest's
  answers and frames are rift's under `roc-core`, the Rust forest its shadow re-seeded from the
  Roc view, upstream staying the Rust forest's shadow. The group bar and rift-cli's views stay
  on the Rust forest until Roc has them (or move with the shape: a question for the lead).

**The group forest, slice 2, done (2026-11-25): the Roc mirror.** One commit after the slice 1
docs commit; its message has the plants, checks and costs.
- **`roc/core/GroupForest.roc`:** the forest as the Rust core holds it (`layouts : Slots(U64)`,
  `nodes : Slots(GroupTreeNode)`, `left_out`, `next_stamp`; the platform's node types
  `GroupPlace`, `GroupBody`, `GroupContainer`, `GroupMembers`, `GroupMemory`, `GroupKind`), every
  forest operation and query and both systems' own, one for one, in the same order of
  allocations and stamps. The seed derives the next stamp as `Forest::from_parts` does. Floats
  follow Rust bit for bit: `f32::max`/`min` as arm64's `fmaxnm`/`fminnm` (`max32`, `min32`),
  `as f32` through `Geometry.narrow`, sums from -0. A missing node or layout crashes naming the
  helper and the key (hand-checked: `[roc crashed] GroupForest.window_index: no node
  4294967297`). Not ported yet: `Forest::shape` (slice 4).
- **The boundary: one machine per system** (`Traditionals`, `Masters`; `traditional_tree_*`,
  `master_stack_*`: init, seed, step, view, release), so neither system's operations can reach
  the other. Events: the forest's shared ones (`GroupForestEvent`) inside each system's
  (`TraditionalEvent`, `MasterStackEvent`); answers `BspTreeAnswer`; a view is the seed's type
  (`TraditionalSeed`, `MasterStackSeed`). Rust: `RocTraditional`, `RocMasterStack`,
  `rift_core_roc::group_forest` (`ForestEvent`, the system events, `step_traditional`,
  `step_master_stack`, `ForestView`, `TraditionalView`, `MasterStackView`), the conversions in
  `boundary/group_forest.rs`.
- **The differential** (`crates/rift-core-roc/tests/group_forest.rs`, 3000 runs of 80
  operations per system, about 11 s): 480,000 operations and 239,000 queries identical, the
  whole system (both tables, every node, sizes by their written form, stamps, settings) after
  each; reloads with saved-only states; master-stack settings changes often of one field; a
  scripted focus move into a stack whose selection stops there (drawn runs reach it once in
  about 30,000). `RIFT_GROUP_FOREST_RUNS` sets the count.
- **Plants:** Roc 56/57 (one equivalent: a move's in-place guard), the conversions 3/3.
- **Next: slice 3, the flip** (done above).

**The group forest, slice 1, done (2026-11-25): the core's forest beside upstream's traditional
and master-stack systems.** Four commits after the rulings' docs commit; each message has its
plants, checks and costs. The parked WIP (`qkxqunyv`) was reviewed against upstream and taken
over (no behaviour difference found in it); it can be abandoned.
- **The core** (`rift_core::layout::group_forest`): `Forest<L, N, W>` (a `SlotTable` of layout
  roots and one of nodes, the windows left out, a stamp counter), `Node { place: Root | Child {
  parent, size }, fill, body: Window { window, stamp } | Container { kind: Split(o) | Stack(o),
  members: Children(Vec) | Regions { order, primary: Side }, memory: Unset | Toward(c) |
  Stops(c) } }`. The window index is derived (`Forest::index`, by window, then stamp). Every
  tree edit, walk, query, focus and shared edit ported from `model/tree.rs`, `groups.rs`,
  `tiles.rs`; `Traditional` (adds with and without a direction, smart insertion, `NewSibling`,
  the joins, unjoin, consume-or-expel, the move and `can_move_window`) and `MasterStack`
  (`MasterSettings` with `RegionInsertion`, `Arrangement`, `MasterSide`, `NewWindowPlacement`;
  regions, settings changes, ratio and count, `enforce_master_count`, add, remove and close by
  app with redistribution, promote, swap master and stack, the move target) over it.
- **The twins** (`systems/traditional/twinned.rs`, `systems/master_stack/twinned.rs`, shared
  `systems/group_twin.rs`): upstream's structs are each module's `Upstream`, held by
  `TraditionalLayoutSystem` / `MasterStackLayoutSystem { upstream, twin: Box<Twinned> }`.
  Answers compared on every operation (queries, `can_move_window`, `window_slot` too), the view
  (layouts and nodes by id with stamps left out, left out, the index in order, the settings) on
  every operation in tests and `roc-core` and every 32nd in a plain release build, each pass's
  shape. A difference is counted (`core: group forest divergences`), fails a test, or warns
  `group-forest-divergence:` and re-seeds the core (`group_twin::seed`: stamps in the index's
  order). Saving is upstream's. The group bar's projection and rift-cli's views still read
  upstream (`tiles()`, `draw_tree`, `container_tree`): slice 3 moves them first.
- **The load normalizes** (`GroupLayouts::normalize_load`, each twin's load; `traditional-load:`
  / `master-stack-load:`): both fill flags, a window's non-default kind, a root size other than 0
  (bitwise), and an index disagreeing with the tree (entries for no leaf of theirs and duplicates
  dropped, missed leaves appended, the saved order otherwise kept). Floating's load is untouched.
- **The drawn fuzzer** (`group_twin::drawn`, 3000 systems of up to 120 steps per system): layouts
  made, cloned and removed, windows in one or two layouts, every change, focus and edit, the
  systems' own operations and settings, every query and a pass on a third of steps, reloads
  (some with a selection cleared, a saved-only state). `RIFT_FOREST_CASES` sets the count.
- **Plants:** core 28/28 (shared and traditional) and 23/25 (master-stack; two equivalent: the
  kept window in promote and in an add to the stack region is never reached by the trim), the
  clone test 1/1, the shadow's seed and conversions 9/9, the normalizations 8/8.
- **Cost (release, `measure_traditional_twin` / `measure_master_stack_twin`, 10 / 100 / 1000
  windows):** upstream 74 ns / 0.24 / 1.9 us an operation (master-stack 97 ns / 0.24 / 1.3 us),
  the core 0.24 / 1.0 / 10.7 us (0.36 / 1.3 / 8.4 us); the view 1.0 / 5.8 / 45 us (1 in 32: 30 ns
  / 0.18 / 1.4 us an operation); a re-seed 54 us / 0.53 / 6.5 ms; a pass's shape upstream 0.64 /
  5.5 / 54 us, the core 0.86 / 9.0 / 167 us. A sibling step searches its parent's list (no
  sibling links), so walks read child lists directly (preorder): before that fix a flat
  1000-window region cost 370 us an operation.
- The lead's rulings (2026-11-25), held by the code and tests:
  - **The window index's order: an entry stamp on each window leaf** (add, clone, replace
    appending from's after to's; swap moves it; the save writes it in order; the load stamps in
    saved order). Slice 3 compares saved bytes exactly. Tests
    `a_clone_in_a_lower_freed_slot_is_indexed_after_its_source`,
    `a_saved_window_index_keeps_its_order`.
  - **Floating is out of scope for now.** Slice 5 deletes only what traditional and
    master-stack use; floating moves onto the core forest last and `GroupLayouts` goes then.
  - **The shape is approved as above**, with the root-size and window-kind normalizations.
  - **A saved index disagreeing with the tree:** every refusal upstream's load makes kept,
    everything else normalized with one warn.
- **Next: slice 2, the Roc mirror** (`roc/core/GroupForest.roc` over `Slots.roc`, a
  differential in `crates/rift-core-roc/tests/group_forest.rs`, a seed entry), as the stack's.

**A stack member's weight (the lead asked for its readers, 2026-11-25): kept on the core's
members.** Readers beyond the saved form: rift-cli's drawing (`draw_tree`) and container view
(`container_tree`, `ContainerTreeNode.weight`), both IPC output; nothing inside rift reads the
protocol's field. The shape passes it as a child's size, which the stack geometry ignores; a
layout-mode switch re-adds windows in order and carries no weights; no operation writes it
(new members get 1.0, only the legacy forest writes others). Moving it to the `stored.rs`
boundary would make the IPC views report 1.0 (or none) and stop legacy-derived files saving
back byte for byte. The lead's ruling: keep it (upstream shows it over IPC; the fork matches
upstream), and list it as an upstream candidate.

**Layout trees, the stack, slice 5, done: upstream's stack system deleted.** Three commits after
the slice 4 docs commit; each message has its plants, checks and costs.
- **The load builds the core's stacks** (`systems/stack/stored.rs`): the saved form (upstream's,
  byte for byte: `Stack`, `Member`) and its load (the current form, the legacy forest, the slice
  1 normalizations) answer the parsed layouts (`Loaded`), made into the core's stacks
  (`core_of_loaded`). Saving is `Saved`, from the core (`saved_stack`). Three normalizations
  (both fullscreen flags on a stack or a member, an empty stack's selection) are now only their
  `stack-load:` warn: the core's types cannot hold those states.
- **Fixtures:** `stack/stack_saved.ron` (written by the core while its bytes were still held to
  upstream's): the operations save to it, it loads and saves back byte for byte (a weight other
  than 1 too), the loaded stacks allocate their next layout where the saved slotmap does and
  mint from the saved counter. `stack/stack_legacy.ron`: the legacy forest as the traditional
  system writes it (the live text held to it while it does), with shares other than 1.
- **What the shadow held is held literally:** rift-cli's drawing (`stack/stack_drawn.txt`) and
  container view, the group bar (windows taking part, the selected index among them, a window
  left out before the selection, none while the shown member or the stack fills), each fill's
  frames. Upstream's unit tests already drove the system.
- **Deleted:** `Upstream` and its geometry, the shadow plumbing (answers and views compared,
  `upstream_of`, `seed`, `model::slot_table::to_slotmap`, `view_difference`, the
  `stack-tree-divergence:` counter and its `rift-cli metrics show-timing` line), the twin's
  tests. `systems/stack/twinned.rs` is now `systems/stack/system.rs`; `stack.rs` is 775 lines
  (from 1498). A stack pass has no system geometry to shadow (`decide_stack`,
  `decide_stack_tree`, both through `decide_unshadowed`).
- **What the core's stacks are held by now:** the rift-core-roc differential (the Rust stacks
  against the Roc stacks, operations and layout passes), the lib's stack tests and the drawn
  fuzzer (under `roc-core` every operation held to the Roc stacks), the fixtures and literal
  tests.
- **Cost (release, `measure_stack_system`, 10 / 100 / 1000 windows):** a plain build pays only
  the core: 32 ns / 0.12 / 1.0 us an operation, 1.1 / 3.8 / 32 us a pass (before: the core and
  upstream's step, about 0.11 / 0.42 / 1.8 us an operation, plus the sampled view). `roc-core`
  pays the Roc stacks, the Rust stacks as their shadow and the view comparison on every
  operation, and both geometries on a pass: 1.9 / 2.2 / 16 us an operation, 6.8 / 18 / 172 us a
  pass.

**Layout trees, the stack, slice 4, done: under `roc-core` the Roc stacks decide** (BSP's 4b).
Two commits after the slice 3 docs commit; each message has its plants, checks and costs.
- **`Stacks::shape` in Roc** (`StackTree.shape`): the words become the geometry's `WindowId`
  there, a member's weight its share through `Geometry.narrow`; a layout the system does not
  hold crashes (`stack_at`), as Rust's `Index` panics. `stack_tree_frames`
  (`RocStackTree::frames`) answers a layout's frames from that shape through the Roc traditional
  geometry; the question (`StackFramesQuestion`) carries a `Pass` (screen, gaps, stack offset,
  stack line) and every known window's constraints by word (`BspKnown`, shared). The
  rift-core-roc differential gains layout passes on 40% of steps (live layouts only): 191,836
  identical bit for bit with the Rust shape through the Rust geometry. Plants 13/15, two
  equivalent (below).
- **A stack member's weight reaches no frame:** the traditional geometry reads a child's size
  only on an axis, and a stack layout is a stack of windows. So a weight plant is equivalent
  through frames; slice 3's weight plant is caught since the Roc stacks decide, through the
  Rust stacks re-seeded from the Roc view (upstream, holding the saved weight, then differs).
  Also equivalent: a `WithinGaps` stack passed through to the shape (at the root it places at
  the tiling area, the root's own rect).
- **Under `roc-core` the Roc stacks and their shape decide** (`systems/stack/roc_stacks.rs`,
  `RocStacks`, in `StackLayoutSystem` beside the core): every answer is the Roc stacks' (parsed
  back; an answer of another kind falls back to the Rust stacks', already reported), and
  `calculate_layout` gives the Roc stacks' frames (`decide_stack_tree`), with the Rust geometry's
  frames for the Rust stacks' shape and upstream's the shadows. The Rust stacks are the Roc
  stacks' shadow: answer and whole view compared on every operation, a difference counted
  (`roc-core: stack tree re-seeds`), panicking in tests, or warned `roc-divergence:` and the Rust
  stacks re-seeded from the Roc view (`core_of`). Upstream stays the Rust stacks' shadow. Saving
  stays from the Rust stacks. Plants 15/15. The drawn fuzzer re-seeds the Rust stacks from the
  Roc view on 4% of its steps.
- **Cost (release, `roc-core`, `measure_stack_roc_stacks`, 10 / 100 / 1000 windows):** a Roc
  layout pass 4.2 / 29 / 142 us against the Rust shape and geometry 1.9 / 8.3 / 30 us (a
  `roc-core` pass pays both); the Roc step 0.53 / 1.7 / 10.9 us, the view comparison 0.61 / 0.80 /
  1.9 us, a Rust re-seed from the Roc view 0.24 / 0.32 / 0.66 us.
- **Next: slice 5** deletes `Upstream`: the legacy forest load and the slice 1 normalizations
  move to a `systems/stack/stored.rs` building the core first (as BSP's), the saved form held to
  fixtures byte for byte; upstream's unit tests retargeted to the system; the shadow plumbing
  deleted; costs re-measured. Then the group forest (traditional and master-stack) waits for the
  lead's go.

**Layout trees, the stack, slice 3, done: the flip (policy (a)).** Four commits after the slice 2
docs commit; each message has its plants, checks and costs. At the tip: workspace tests 1038
(5 ignored), the lib with `roc-core` 916 (7 ignored), rift-core-roc 52, rift-core 107, the
fuzzer 2000 x 40 both ways, clippy 331.
- **rift-cli's views read the core first** (`Stacks::container`, `Stacks::draw`: upstream's
  stack written by `{:#?}`, ids as slotmap's `new_key_type` writes a key; upstream's views held
  to them on every call). Plants 5/5.
- **A core geometry fix found on the way:** the traditional geometry's stack frames added a
  `0.0` shift on the axis a stack does not run along, so a container at x = -0.0 gave 0.0 where
  upstream's frames keep -0.0 (a bit-level `layout-divergence:`, no position moved). Rust
  `stack_frames` and Roc `Traditional.stack_frame` now leave that coordinate as the
  container's. Test `a_stack_keeps_a_negative_zero_origin_across_its_axis`; plants 2/2 (the
  Rust test, the Roc layout differential).
- **The flip** (`systems/stack/twinned.rs`): `StackLayoutSystem { core, shadow: Box<Shadow {
  upstream, steps, roc (roc-core only) }> }`. Every operation's and query's answer is the
  core's; upstream steps in every build, compared as before; a difference re-seeds the shadow
  from the core (`upstream_of`, through `model::slot_table::to_slotmap`, BSP's former
  `slot_map`). `calculate_layout`: the core geometry's frames for the core's shape, upstream's
  geometry the shadow. **Saving is the core's** (`Saved`, under upstream's original name
  `StackLayoutSystem`), byte-compatible by construction and held to upstream's bytes at every
  drawn reload. Loads still go through upstream's load (legacy forest, the slice 1
  normalizations). Plants 8/8.
- **Under `roc-core`, the Roc stacks are a second shadow** (`systems/stack/roc_shadow.rs`,
  `RocShadow`): seeded from the core, stepped with every operation and query, answer and whole
  view compared; a difference counted (`roc-core: stack tree re-seeds`), panicking in tests, or
  warned `roc-divergence:` and re-seeded from the core. A window change is read once into
  `Applied` (core step and Roc event from it). Plants 6/7 (the weight's conversion is equivalent
  until the Roc shape reads weights). The drawn fuzzer's reloads now write saved weights other
  than 1.
- **Cost (release):** a plain build pays the core (52 ns / 0.22 us / 0.97 us an operation at 10 /
  100 / 1000 windows) and upstream (58 ns / 0.20 us / 0.86 us), the view 87 ns an operation at
  1000 windows (1 in 32), a shadow re-seed 43 us / 0.15 ms / 1.5 ms. `roc-core` adds the Roc
  step (0.52 / 1.8 / 13.6 us) and a view comparison (0.61 / 0.81 / 2.3 us) on every operation.
- **Next: slice 4, under `roc-core` the Roc stacks decide** (BSP's 4b): `Stacks::shape` in Roc
  (`StackTree.shape`, the question carrying screen, gaps, stack offset, the stack line and every
  known window's constraints by word; `stack_tree_frames` through the Roc traditional geometry;
  the rift-core-roc differential gains layout passes); then the Roc stacks' answers and frames
  are rift's under `roc-core`, the Rust core their shadow re-seeded from the Roc view (`core_of`),
  upstream staying the Rust core's shadow; saving stays from the Rust core (equal after every
  operation). Then slice 5 deletes `Upstream` (the legacy load and the normalizations move to a
  `stored.rs` building the core first, as BSP's).

**Layout trees, the stack, slice 2, done: the Roc mirror.** Three commits after the slice 1
docs commit (the lead's slice 1 rulings first); each message has its plants and checks.
- **A stack's members are `Empty | Populated { members: NonEmpty, selection }`** (the lead's
  ruling on slice 1: `Nothing` was an invariant the type did not hold). `Selection = Member(Id)
  | Root(Id)`; `NonEmpty` changes length only through operations that keep an element; retain
  rebuilds it through `NonEmpty::new` (the last member gone makes the stack `Empty`) and picks
  the selection's successor from the kept members' positions, with no unreachable branch.
  Plants 4/4.
- **`Slots.roc`:** BspTree's slot table, unchanged, as its own nominal type module `Slots(v)`,
  shared by both trees (plus `map_values`, which the stacks use). Plants 3/3 (the BSP
  differential).
- **`roc/core/StackTree.roc`:** every `Stacks` operation and query, one for one, in the same
  order of mintings. Members are found as zippers (before, the member, after; from the front or
  the back), never by index. A missing layout crashes naming the lookup and the key (hand-checked:
  `[roc crashed] StackTree.climb: no layout 4294967297`). The boundary (`StackTreeSeed`,
  `StackTreeEvent`, `StackTreeAnswer`, `stack_tree_init/seed/step/view/release`) reuses the BSP
  tree's shared types (orientation, direction, insertion, participation, window-in-layout,
  replacement, native resize, queries, focus, pair, edit answer). Rust: `RocStackTree`,
  `rift_core_roc::stack_tree` (`Event`, `Answer`, `Scope`, `step`, `StacksView`), the conversions
  in `boundary/stack_tree.rs`.
- **The differential** (`crates/rift-core-roc/tests/stack_tree.rs`, 8000 runs of 60 operations,
  2.1 s): operations on live layouts (stale keys only for remove and contains: upstream's
  `Index` panics on the rest), a query after half, a reload on 3% with saved-only states
  (weights other than 1, a fill on an empty stack). 480,000 operations and 239,000 queries
  identical, the whole view compared after each, no Roc allocation left live. Plants 28/28 in
  Roc, 2/2 in the conversions.
- **Not ported to Roc yet: `Stacks::shape`** (slice 4 builds it in Roc, as BSP's 4b did).
- **Next: slice 3, the flip** (policy (a): the core decides and is saved byte-compatible;
  upstream's system its shadow, re-seeded from the core on a divergence; rift-cli's views moved
  to the core early; under `roc-core` the Roc stacks a second shadow).

**Layout trees, the stack, slice 1, done: the core's stacks beside upstream's stack system.** Two
commits after the rulings' docs commit; each message has its plants and checks. At the tip:
workspace tests 1036 (4 ignored), the lib with `roc-core` 914 (5 ignored), the rest as below,
clippy 331.
- **The core:** `rift_core::layout::stack_tree::Stacks<L, W>`: `SlotTable<L, Stack>`, the
  saved `next_node` counter (member and stack ids are `Id`, upstream's `NodeId` word; minted
  `1 << 32 | n`), the default orientation (upstream's four cases), the windows left out, the
  insertion point. `Stack { id, members: Vec<Member { id, window, weight, fill }>, selection,
  orientation, fill }`; `Selection = Nothing | Member(Id) | Root(Id)`. **Root carries the member
  it was selected from:** upstream's `root_selected` keeps `active`, which stays the selected
  window (`selected_window`, a fullscreen toggle's fallback, a new window's insertion point) and
  is where `Descend` returns; a `Root` without it would change behaviour. Nothing exactly when
  there are no members (held by every operation and the load). Every operation and query
  ported one for one; `Fill` is the traditional geometry's.
- **The twin (`systems/stack/twinned.rs`):** upstream's struct is `Upstream`, unchanged but for
  its load; `StackLayoutSystem { upstream, twin: Box<Twinned> }`, serialized as upstream. Every
  operation steps both; answers compared on every operation (queries too, the group bar's
  container included), the whole system on every operation in tests and `roc-core` builds and
  every 32nd in a plain release build, each pass's shape on every pass. A difference: counted
  (`rift-cli metrics show-timing`: `core: stack tree divergences: N`), a test panics, a running
  Rift warns `stack-tree-divergence:` and re-seeds the core from upstream. `draw_tree` and
  `container_tree` still read upstream (slice 3 moves them).
- **The load normalizes (the W2b rule, one `stack-load:` warn each):** a tiled layout kind on a
  stack (to its stack kind, as the legacy load maps one), both fullscreen flags on a stack or a
  member (fullscreen), a selection naming no member while there are members (the first member),
  an empty stack with a selection (none). Upstream keeps them all (an upstream candidate).
  Files rift writes load and save byte for byte as before.
- **The drawn fuzzer** (`stack.rs`'s `drawn_system`, 3000 systems) draws every operation,
  every query on a third of its steps, and reloads; each case's pass goes through the twin.
- **Plants:** 15/15 in the core, 2/2 in the shadow's conversion, 5/5 for the load's
  normalizations.
- **Cost (release, `measure_stack_twin`):** upstream 72 ns / 0.10 us / 0.86 us an operation,
  the core 76 ns / 94 ns / 0.77 us at 10 / 100 / 1000 windows; the whole-system view 0.43 /
  0.37 / 2.7 us (85 ns an operation at 1 in 32, 1000 windows); a re-seed 28 us / 0.15 ms /
  1.5 ms; a pass's shape 0.38 / 0.98 / 6.9 us upstream's, 0.26 / 0.93 / 6.8 us the core's.
- **Next: slice 2, the Roc mirror** (`roc/core/StackTree.roc`, a differential in
  `crates/rift-core-roc/tests/stack_tree.rs`, a seed entry), as BSP's slice 3.

**Layout trees, slice 5, done: upstream's BSP system deleted.** Three commits after the 4b docs
commit; each message has its plants and checks.
- **The load builds the core's tree** (`systems/bsp/stored.rs`): the saved form (upstream's,
  byte for byte: `LayoutState`, `Node` with its `selected`, `NodeKind`, `LeafMembers`) and its
  load (the current form, the older topology-and-components one, abandoned nodes dropped, the
  slice 2 normalizations) answer the parsed tables (`Loaded`), made into the core's tree with
  the slotmaps' free lists (`core_of_tables`). Saving is `Saved`, from the core. Two of the
  normalizations (both fullscreen flags, an empty leaf's flags) are now only their `bsp-load:`
  warn: the core's types cannot hold either state (`Fill` is one of three; an empty leaf holds
  none).
- **Fixtures:** `bsp_saved.ron` (written by the core while its bytes were still held to
  upstream's): the operations save to it, it loads and saves back byte for byte, and the loaded
  tree allocates its next layout and node where the saved form's slotmaps do. The normalization
  tests write each corrupt state into the text of a system saved from operations;
  `bsp_legacy.ron` variants hold the legacy branches (a stack without its window, a node no
  layout reaches).
- **Upstream's unit tests** drive `BspLayoutSystem` through the layout system's interface; new
  literal tests hold what the shadow held before: a rebalance across alternating splits, and
  rift-cli's container view and text drawing.
- **Deleted:** `Upstream` and its geometry (the BSP projection, `shadow_frames`, the constraint
  helpers), the shadow plumbing (answers and views compared, `upstream_of`, the
  `bsp-tree-divergence:` counter and its `rift-cli metrics show-timing` line), the twin's tests
  that only held that plumbing. `systems/bsp/twinned.rs` is now `systems/bsp/system.rs`;
  `bsp.rs` is 598 lines (from 2552). A BSP pass has no system geometry to shadow
  (`decide_unshadowed`): a plain build takes the Rust core's frames; `roc-core` the Roc tree's,
  the Rust core's the shadow.
- **What the core's tree is held by now:** the rift-core-roc differential (the Rust tree against
  the Roc tree, operations and layout passes), the lib's BSP tests and the drawn fuzzer (under
  `roc-core` every operation held to the Roc tree), the fixtures.
- **Cost (release, `measure_bsp_system`, everyday operations through the system, and a layout
  pass):** a plain build pays only the core: 0.21 / 1.3 / 16 us an operation and 5 / 45 / 171 us a
  pass at 10 / 100 / 1000 windows (before slice 5, upstream's step and the sampled view came on
  top: about 21 us an operation at 1000 windows). `roc-core` pays the Roc tree, the Rust tree as
  its shadow and the view comparison on every operation, and both geometries on a pass: 4.2 / 12 /
  171 us an operation, 15 / 82 / 743 us a pass.

**Layout trees, 4b, done: the shape in Roc; under `roc-core` the Roc tree decides.** Three commits
after the slice 4 docs commit; each message has its plants, checks and costs.
- **rift-cli's BSP views read the core** (the lead's ruling: first, so slice 5 only deletes):
  `Tree::container(layout)` (each node, whether the selection's memory rests on it, its share of
  its parent, a split's children or the leaf as held) and `Tree::draw(layout)`, pure, in
  `rift_core`; the platform maps a container to `ContainerTreeNode`. Upstream's views are held
  to them on every call (the drawn fuzzer asks both on every query step).
- **`Tree::shape` in Roc** (`BspTree.shape`): the words become the geometry's `WindowId` there;
  `bsp_tree_frames` (`RocBspTree::frames`) answers a layout's frames from the Roc tree's own shape
  through the Roc geometry, the question carrying the screen, gaps and every known window's
  constraints by word. Held by the rift-core-roc differential: layout passes on 40% of steps,
  192,174 identical bit for bit with the Rust tree's shape through the Rust geometry (136,424
  windows placed, 54,567 constrained).
- **Under `roc-core` the Roc tree and its shape decide** (`systems/bsp/roc_tree.rs`, `RocTree`,
  in `BspLayoutSystem` beside the core): every operation's and query's answer is the Roc tree's
  (parsed back to rift's types; an answer of another kind than the operation's falls back to the
  Rust tree's, already reported), and `calculate_layout` gives the Roc tree's frames
  (`decide_bsp_tree`), with the Rust geometry's frames for the Rust tree's shape and upstream's
  the shadows. The Rust tree is the Roc tree's shadow: its answer and whole view compared on every
  operation, a difference counted (`roc-core: bsp tree re-seeds`), panicking in tests, or warned
  `roc-divergence:` and the Rust tree re-seeded from the Roc tree's view (`core_of`; a query's
  difference is only reported). Upstream stays the Rust tree's shadow, compared after the Roc
  decision. Saving stays from the Rust tree, which under `roc-core` equals the Roc tree after
  every operation. Without `roc-core` nothing changed: the Rust tree decides.
- **Cost (release, `roc-core`, `measure_bsp_roc_tree`):** a Roc layout pass 7 / 62 / 557 us
  against the Rust tree's shape and geometry 2.3 / 19 / 170 us at 10 / 100 / 1000 windows (a
  `roc-core` pass pays both, the Rust one as the shadow); the Roc step 0.35 / 1.5 / 29 us, the view
  comparison 0.45 / 2.7 / 34 us, a Rust re-seed from the Roc view 0.8 / 3.8 / 42 us.

**Layout trees, slice 4, done: the flip (policy (a)).** Two commits after d911eb9b: the flip, and
the Roc tree as a shadow under `roc-core`. Each message has its plants, checks and costs.
- **The core's `Tree` decides in every build** (`systems/bsp/twinned.rs`): `BspLayoutSystem {
  core, shadow: Box<Shadow { upstream, steps, roc (roc-core only) }> }`. Every operation steps
  the core and gives its answer; upstream's system (the shadow) steps in every build, its answers
  compared on every operation, the whole tree on every operation in tests and `roc-core` builds
  and every 32nd in a plain release build, the shape on every pass. A difference: counted
  (`core: bsp tree divergences`), a test panics, a running Rift warns `bsp-tree-divergence:` and
  **re-seeds the shadow from the core** (`upstream_of`; the lead's ruling: the deciding side
  stands, as in every policy (a) flip). `calculate_layout` gives the core geometry's frames for
  the core tree's shape (option (a)), upstream's geometry the shadow.
- **Saving is written from the core** in upstream's form (`Saved`, renamed `Upstream`: the
  `SerdeSlot` sequences, a split's `selected`), byte-compatible by construction; the drawn fuzzer
  holds the core's save to upstream's bytes at every reload. Loads still go through upstream's
  load (old forms, the slice 2 normalizations) and `seed()` the core.
- **The re-seed keeps ids in step:** a slotmap is only built through its saved form, whose load
  rebuilds the free list highest first, so `slot_map` writes each vacant slot two versions back,
  then takes (`| 1`) and frees (`+ 1`) it again in the core's order. Held by
  `a_shadow_reseeded_from_the_core_allocates_the_ids_the_core_would` and by the drawn fuzzer,
  which re-seeds the shadow on 4% of its steps.
- **Under `roc-core` the Roc tree is a shadow of the core's** (`systems/bsp/roc_shadow.rs`, the
  lead's ruling for slice 4): seeded through `bsp_tree_seed` at load, stepped with every operation
  (queries too, through a cell: the layout system's queries take `&self`), answer and whole view
  compared on every operation after upstream's; a difference is counted (`roc-core: bsp tree
  re-seeds`), panics in tests, or warns `roc-divergence:` and re-seeds the Roc tree from the core.
  An app closing lists the app's windows the core holds (in layouts or left out). Windows are
  `Word`s, ordered as `WindowId` (pid, then index), so the trees' orderings agree.
- **`draw_tree` and `container_tree` (IPC)** read the shadow at slice 4's end; 4b moved them to the
  core.
- **Plants:** 10/10 for the flip (core, shadow, conversion, save only, re-seed), 8/8 for the Roc
  shadow (Roc and conversion). A trap found on the way: a plant runner that restores the file with
  an older mtime than the planted build leaves cargo's planted object in place (the next plants
  then run on it); restore, then touch the file.
- **Cost (release):** a plain build steps both trees (upstream 64 ns / 0.42 us / 7.4 us, core
  76 ns / 0.76 us / 13.5 us at 10 / 100 / 1000 windows), the view every 32nd (5 / 43 / 410 ns per
  operation), a shadow re-seed 63 us / 0.52 ms / 5.0 ms; a `roc-core` build adds the Roc tree's
  step (0.85 / 3.7 / 29 us) and a view comparison (1.1 / 5.5 / 34 us) on every operation, a Roc
  re-seed 1.4 / 7.4 / 38 us.

**Layout trees, slice 3, done: the Roc mirror of the BSP tree.** Commits (after b6ef8a80's task
0): the slot tables, layouts, participation and settings; the window changes; the queries; the
focus; the shape edits; the moves, swaps and drops; the joins (every edit ported); the seed. Each
message has its plants and checks.
- **The core:** `roc/core/BspTree.roc` holds `Trees` as the Rust core holds `Tree`: `layouts :
  Slots(U64)` (each layout's root), `nodes : Slots(BspTreeNode)`, `left_out : Set(U64)`, the
  insertion point, the aspect ratio. `Slots(v) : { slots : List([Occupied({ version, value }),
  Vacant(U32)]), free : List(U32) }` with slotmap's rules (slot 0 the sentinel, the free list
  taken last-first, a new slot at version 1, a vacant one at `version | 1`, a removal bumping by
  one); keys are words, `version << 32 | index` (slotmap's ffi form). Nodes are the platform's
  types (`BspTreeNode { parent : [Root, Split(U64)], kind : [Split(BspTreeSplit),
  Leaf(BspTreeLeaf)] }`, a split's children `first`/`second`). Every `Tree` operation is ported
  one for one, in the same order of allocations, so keys match.
- **A missing node is `crash` (the lead's ruling (b), Rust parity).** A node is looked up only
  by a key the tree holds; `BspTree.node` (and `set_node`, `split_of`, `content_of`,
  `set_content`, the root and detach checks) crash naming what looked and the key, as Rust's
  `Index` panics and its `unreachable!`s stop. `roc_crashed` aborts, as it always did ("a core
  crash is fatal: Roc cannot unwind, and the platform restores window state on restart"), so
  under `roc-core` a Roc core bug on an impossible state aborts rift exactly as the Rust core's
  panic does. Checked once by hand: a plant (node lookups off by one) aborts the differential's
  binary with `[roc crashed] BspTree.kind: no node 4294967297`. The message names the helper
  that looked (`kind`, `parent`, ...) and the key word; not the public operation, which would
  mean threading a name through every helper.
- **The boundary:** a stateful machine beside the workspaces and main windows: the platform
  requires `Trees : trees`, with `bsp_tree_init : BspInsertion -> trees`, `bsp_tree_seed :
  BspTreeSeed -> trees` (a system as the platform holds it: each table's slots and free list in
  taking order, the windows left out, the settings; the host parses it first), `bsp_tree_step :
  trees, BspTreeEvent -> { model, answer : BspTreeAnswer }` (one event per operation and query;
  an app closing or a has-windows-for-app question lists the app's windows, as Roc has no pid)
  and `bsp_tree_view`, plus a release. Rust: `RocBspTree` (`new`, `seeded`, `step`, `view`) and
  `rift_core_roc::bsp_tree`: `LayoutKey`/`NodeKey` (`SlotKey` newtypes), `Event`/`Answer`,
  `step` (an event applied to a `Tree`), `TreeView` (comparable; `From<&Tree>`, and `tree()`
  parsing it back through `SlotTable::parse`/`resumed`). `SlotTable` gained `free()`.
- **`Geometry.narrow`:** Roc has no `F64 -> F32`, and a share is an `f32` (`as f32` in Rust), so
  the core writes Rust's conversion out bit for bit, answered through a typed question (`narrow :
  F64 -> F32`, `rift_core_roc::narrow`) and held by `the_roc_core_narrows_an_f64_as_rust_does`
  (600,000 values: every class, the ties between neighbouring `f32`s, just under each power of
  two, the range's edges).
- **The differential** (`crates/rift-core-roc/tests/bsp_tree.rs`, 8000 runs of 60 operations,
  2.9 s): every operation drawn (layouts made, cloned and removed with stale keys, participation,
  the settings, windows added, removed, replaced and closed by app, native resizes between the
  screen, the tiling area and frames nudged around half a point, every focus and edit, resize
  amounts of every magnitude), a query after half of them, and a reload on 3%: both trees seeded
  from the same system with preselected leaves and splits whose memory stops (states only a saved
  file holds). The windows a move, swap, drop or query decides on are mostly the layout's own.
  Answers and the whole view (slots, versions, free lists, left out, settings) compared after
  each: 480,000 operations identical, 240,000 queries (83,000 telling something), 14,000
  operations leaving a stack, 200 descents moving the selection, no Roc allocation left live.
  Plants: 71 in Roc across the families (each commit lists its own), 70 caught, one
  equivalent in the drawn space: rebalance's 0.05 clamp (a share under 0.05 needs more than 19
  tiles along one axis; runs draw 8 windows).
- **Not ported to Roc: `Tree::shape`** (the geometry's input, with its constraints closure). The
  Roc geometry takes a `BspShape` of `WindowId`s; the tree's windows are words. Slice 4 decides
  whether the Roc tree builds its own shape or the Rust side keeps building it (a design
  question in the slice report).
- **Glue:** a tag named after a Rust keyword (`Move`) becomes a field named `move`, which does
  not compile; upstream's `Move` is `Moved` at the boundary (`BspEdit`, `BspDropAction`), as the
  floating slot's `Placed` was for `in`.

**Next: layout trees, the traditional family (the lead's go, 2026-11-25), same five slices as
BSP. The lead ruled on the plan's three questions (2026-11-25):**
1. **Order: as proposed.** The stack goes alone, through all five slices. Then the group forest
   once, as one core tree serving both traditional and master-stack, each system a thin shim
   over it.
2. **A saved window index that disagrees with its tree:** keep every refusal upstream's load
   makes; normalize everything else to the tree, with one `*-load:` warn (the W2b rule). The
   index is derived in the core, still written for byte-compatibility, and checked on load.
3. **Stack member ids:** keep the saved `next_node` counter as the id source (byte-compatible).
   Don't mint from a slot table.

The QA lessons apply as planned: `Selection = Root | Member(id) | Nothing` instead of `active`
plus `root_selected`; each fullscreen bool pair a `Fill`; the forest's children an ordered id
list; a container's selection memory a child id; `primary_first` read from the children's order;
nothing stored that can be derived. The stack's slices, BSP's pattern: (1) a core tree beside
the system, with an operation shadow and the drawn fuzzer; (2) a Roc mirror with a differential
and a seed entry; (3) the flip (policy (a), byte-compatible save, divergence counted, warned,
panicking in tests, the shadow re-seeded from the core; rift-cli's views moved to the core
early); (4) under `roc-core` the Roc tree decides, the Rust tree its shadow; (5) the old stack
system deleted, the legacy load moved to the core side first.

*What the code shows (2026-11-25).* The three systems already hand the core's traditional
geometry (`rift_core::layout::traditional`, mirrored in `Traditional.roc`) a `Traditional` shape
through `decide_traditional`, so only their trees move. They hold two different trees:
- `StackLayoutSystem` (`systems/stack.rs`, 1108 lines): per layout one `Stack { id, members:
  Vec<Member { id, window, weight, fullscreen, within_gaps }>, active: Option<NodeId>,
  root_selected, kind, fullscreen, within_gaps }`, members minted by a `next_node: u32` counter,
  a legacy load from the old forest (`LegacyStackForest`).
- `TraditionalLayoutSystem` and `MasterStackLayoutSystem` share `GroupLayouts` (`groups.rs`,
  `tiles.rs`, `model/tree.rs`): one slotmap forest (`NodeMap`, sibling links `parent`,
  `prev_sibling`, `next_sibling`, `first_child`, `last_child`) of nodes with `LayoutInfo { size,
  kind, is_fullscreen, is_fullscreen_within_gaps }` and contents `Container(children with a
  selection) | RegionPair { regions, selection, primary_first } | Window`, a saved window index
  (`window_nodes: BTreeMap<WindowId, [(layout, node)]>`), and `layout_roots: SlotMap<LayoutId,
  LayoutRoot>`. Master-stack adds its regions (master and stack containers) and settings; a
  legacy load reads the older topology-and-components form.

*Proposed order.* (1) Stack, alone: the smallest tree, its own ids, its own save. (2) The group
forest once, as one core tree serving both traditional and master-stack (they share every
operation on it; master-stack's regions are a node kind), each system then a thin shim over it.
Each goes through the five slices; the stack's slices 1-5 come before the forest's slice 1.

*The QA lessons, as they apply (one fact per thing, nothing stored that can be derived, ids not
positions).*
- Stack: `active: Option<NodeId>` with `root_selected: bool` is one fact in two fields
  (`Selection = Root | Member(id) | Nothing`); each `fullscreen`/`within_gaps` pair is a `Fill`
  (as BSP's); members are found by id, never by index. The forest: a node's children as an
  ordered list of ids (no sibling links to keep consistent); a container's selection memory as an
  id of one of its children; `RegionPair`'s `primary_first` read from the children's order.
- The forest's window index is derived (which leaf holds which window, per layout): the core
  derives it and does not store it; it is written into the saved form (byte-compatible) and, on
  load, checked against the tree.

*Questions for the lead.*
1. The order above (stack alone first, then the forest once for traditional and master-stack):
   right?
2. A saved window index that disagrees with its tree (a file holding a window the tree does not,
   or the other way): normalize to the tree with one `*-load:` warn (the W2b rule), or refuse with
   a typed error (where upstream's load refuses, keep refusing)? Proposed: keep every refusal
   upstream's load makes, normalize the rest to the tree.
3. The stack's `next_node` counter is saved and mints member ids: keep it as the core's id source
   (saved as now), or mint member ids from a slot table like BSP's nodes (changing saved ids, so
   not byte-compatible)? Proposed: keep the counter (byte-compatible; ids are already ids).
- **4 (done, above):** the Rust core's tree decides and is saved in every build; upstream's
  system is its shadow, re-seeded from the core; under `roc-core` the Roc tree is a second shadow,
  re-seeded from the core through `bsp_tree_seed`. The shape is built from the Rust core's tree
  (option (a)).
- **4b (done, above):** the shape in Roc; under `roc-core` the Roc tree and its shape decide, the
  Rust tree its shadow, re-seeded from the Roc tree's view.
- **5 (done, above):** `Upstream` deleted; the load builds the core's tree.
- **The Roc nightly bump (done 2026-11-25, below):** pinned nightly-2026-11-24-ff5e544; every
  workaround for roc#13363, #13348, #13349 and #13350 removed, one commit each.

**Upstream candidate:** upstream's saved stack layout can hold states no operation reaches (a
tiled layout kind on a stack, both fullscreen flags on a stack or a member, a selection naming
no member while there are members, an empty stack with a selection), and its load keeps them
(the fork normalizes each with a `stack-load:` warn, the lead's ruling).

**Upstream candidate:** a stack member's weight is saved and shown over IPC but never affects a
frame; no operation writes it.

**Upstream candidate (fix pending upstream, fork branch `pr/layout-ghost-windows`):** master-stack
counts windows left out of layout (away) toward `master_count`, so a master window that is away
empties the master area. The fork matches upstream main and keeps the behaviour until it lands.

**Upstream candidate:** the layout pass that also returns group bars with its frames
(`project_groups` with frames asked for, from `calculate_layout_with_groups`) is reached only by
tests: rift's one caller passes no group output, and the bar comes through
`collect_group_containers`. The fork keeps it as upstream has it (the lead's ruling).

**Upstream candidate:** `rift-cli`'s help example `rift-cli execute window resize-by --amount
-0.10` fails: clap reads `-0.10` as a flag; `--amount=-0.10` works. Not fixed in the fork.

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
  `bsp-tree-divergence:` and re-seeds the core from upstream (slice 2; since the flip the core
  decides and the shadow is re-seeded from it, slice 4 above). Seeding reads each slotmap's slots
  (vacant versions from its saved form) and its free list (the order a copy hands vacant slots
  out), taken by `SlotTable::resumed` (a load would take the highest vacant slot first).
- **The drawn fuzzer** (`bsp.rs`'s `drawn_system`, under the BSP differential, 3000 systems)
  drives `BspLayoutSystem`: every edit and focus, layouts created, cloned and removed, windows
  replaced, native resizes (into and out of the screen and tiling frames, nudges around the
  half point), participation (windows in no layout too), the settings, the edits upstream has no
  case for, drops straight to `edit_layout`, every query on a third of its steps, and reloads
  through serde, most with a preselected leaf (only a saved file holds one).
- **States only a saved file holds, parsed at the load boundary (the lead's ruling,
  2026-11-24):** `Upstream`'s load (`TryFrom<StoredBspSystem>`)
  normalizes, on each node a layout reaches, with one `bsp-load:` warn naming the layout and
  the node: both fullscreen flags set -> fullscreen (as the projection reads them); an empty
  leaf's fullscreen flags -> none; a split's share that is not a finite number -> 0.5 (a fresh
  split's). Finite shares stand as saved, even outside the 0.05..0.95 edits keep (upstream's
  load does not clamp them, and no other bound applies). Upstream's copy and the core are both
  seeded from the parsed form, so the twin never diverges on a file; files rift writes save
  byte for byte as before. Tests: `a_saved_leaf_with_both_fullscreen_flags_loads_fullscreen`,
  `a_saved_empty_leaf_loads_without_fullscreen_flags`,
  `a_saved_split_with_a_share_not_a_number_loads_even` (each: the parsed form, the save, the
  normalized bytes); a plant skipping each normalization fails its test. The one operation
  that made a NaN share, a resize by a NaN amount, can no longer be asked for (below).
- **Command numbers are finite (the lead's ruling, 2026-11-24).** `rift_protocol::Amount` is
  every number a command carries (resize amounts, the scroll delta, the master ratio's change, a
  floating size, the animation, stack-offset and gap settings, a command's editable `Number`
  argument): parsed once where a command enters rift (serde for IPC and config files, `FromStr`
  for rift-cli and the settings window), refused when not finite (`rift-cli` names it; a config
  file fails to read; the settings field keeps the command as it was). IPC's JSON could not carry
  one already (`NaN` becomes `null`, refused); TOML (`nan`, `inf`) and the settings window
  could. Tests: `a_command_number_that_is_not_finite_is_refused` (rift-cli),
  `a_bound_command_whose_number_is_not_finite_is_refused` (config).
- **Upstream candidate:** IPC accepts NaN/inf amounts; a NaN resize writes a NaN share that
  saves and reloads (upstream's config files and settings window carry them; its JSON IPC
  refuses them only because `NaN` is written as `null`).
- **Upstream candidate:** upstream's saved BSP layout can hold states no operation reaches
  (both fullscreen flags, an empty leaf's fullscreen flag, a share that is not a finite
  number), and its load does not normalize them.
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

**Slice 3's plan, as handed off (the lead's go, 2026-11-24; done, above; the missing-node
question ruled `crash`, the lead's (b)).** The plan, from the patterns already in the crate:
- *Roc:* a `roc/core/BspTree.roc` type module holding the tree as the Rust core does: a slot
  table with slotmap's rules (`slots : List([Occupied({ version : U32, value : v }), Vacant(U32)])`,
  slot 0 the sentinel, the free list taken last-first, a new slot at version 1, a vacant one
  taken at `version | 1`, a removal bumping by one), keys as words (`version << 32 | index`,
  slotmap's ffi form, so the Rust side's `SlotKey` maps one to one), nodes `Split { orientation,
  ratio : F32, children, memory }` / `Leaf { content : [Empty, Held({ members, fill })],
  preselect }`, members `[Window(U64), Stack({ before, top, after })]`. Each Rust operation
  ported one for one, by family as in slice 2. A node lookup by a key the tree holds cannot
  fail in Rust (`Index` panics); in Roc, `List.get` answers `Try`: give operations one lookup
  helper and decide with the lead whether a missing node is `crash` (none exists in the core
  yet) or an `Err(Corrupt)` the step answers (the differential would catch it).
- *The boundary:* a stateful model like `Joined`/`Windows`: `[.., BspTree : bsp_tree]` in the
  platform's `requires`, entries `bsp_tree_init : InsertionPoint -> bsp_tree`, `bsp_tree_step :
  bsp_tree, BspTreeEvent -> { model, answer : BspTreeAnswer }` (one event per operation:
  create/clone/remove layout, participation, settings, add/remove/replace/close-app (the app's
  windows listed by the host: Roc has no pid), resize, focus, edit, and each query) and
  `bsp_tree_view : bsp_tree -> { model, layouts, nodes, left_out, settings }`, plus a release;
  `RocBspTree` in `rift-core-roc/src/lib.rs` beside `RocWorkspaces`, the conversions in
  `boundary.rs`. Avoid a one-tag union with several payloads (roc#13348) and a tag named
  `Release` (roc#13350) until the nightly bump (both fixed at ff5e544).
- *The differential:* `rift-core-roc/tests/`, drawn operation sequences (the platform fuzzer's
  mix: every edit, focus, layouts cloned and removed, replaces, resizes, participation,
  settings, queries) stepped on `rift_core::layout::bsp_tree::Tree` (keys as a plain `SlotKey`
  newtype) and on `RocBspTree`, every answer and the whole view compared; plants in Roc, each
  caught. Then slice 4 can put the Roc tree beside rift's under `roc-core`.
- *Then* 4, the flip (policy (a)): the core's tree decides and is saved (old files
  byte-compatible; the load's normalization above already parses what only a file holds); 5
  deletes `Upstream`. The queued Roc nightly bump (below) still waits: the newest nightly is
  still nightly-2026-11-23-b5c77fc (checked 2026-11-24).

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

**The Roc nightly bump to nightly-2026-11-24-ff5e544 (done 2026-11-25).** Six commits after the
slice 5 docs commit, each with full checks green: the pin; `LifecycleStepped` in the platform
module on `Windows` (roc#13363); `bsp_tree_frames` answering its frames bare, `BspFrames` gone
(roc#13363); the glue included as generated with no lint allowed (roc#13349); the build's
one-tag-union scan gone (roc#13348; a probe union `[Probe(U64, U32)]` on a copy of the platform
came out with both values, 16 bytes); the event tag `Release` again (roc#13350). Each removal
confirms its issue's fix at that nightly. The local install's `current` and `safe` point at
ff5e544, and `src` is checked out at ff5e544 for the glue spec.

**Queued, between slices (the lead, 2026-11-24; done, above): the next Roc nightly bump.** roc#13363 (PR 13389,
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
   workaround went (all but `LifecycleStepped`: roc-lang/roc#13363). The next bump, to
   nightly-2026-11-24-ff5e544 (2026-11-25), removed the rest (Handoff).

- The window store's single writer (the upstream PR plan's C1 step 2): done 2026-11-24, admission
  facts only; `invalidate_ax_identity` / `validate_ax_identity` answer `Invalidation` /
  `Revalidation` (Handoff).
- `LifecycleStepped` moved into the platform module beside the other entries' results at
  nightly-2026-11-24-ff5e544 (roc-lang/roc#13363's fix, 3ef41a6).

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

## Roc: the design we follow, and what is left (nightly-2026-11-24-ff5e544)

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

- **`crash` is for impossible states, and it is fatal** (langref `errors.md`, "When to Crash";
  `statements.md`: what follows a crash "is determined by the platform"). Rift's host aborts in
  `roc_crashed` ("a core crash is fatal: Roc cannot unwind, and the platform restores window
  state on restart"). The core uses it in one place, `BspTree.roc`: a node looked up by a key the
  tree holds that is not there, a parent that is not a split, a leaf that is not a leaf (the
  lead's ruling (b), 2026-11-24). That is Rust parity: the Rust tree's `Index` panics and its
  `unreachable!`s stop on the same states, and under `roc-core` either aborts rift. No `Try`
  answer stands for them (it would be a defensive branch for a state that cannot occur). Each
  message names the helper that looked and the key (checked by hand with a plant:
  `[roc crashed] BspTree.kind: no node 4294967297`).
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

- **Fixed at nightly-2026-11-24-ff5e544, workarounds removed:** roc-lang/roc#13363 (a provided
  function answering a bare type "polymorphic" when a platform-module type wraps it:
  `LifecycleStepped` sat in `Boundary` generic in the model, and `bsp_tree_frames`'s frames sat
  in a nominal `BspFrames`); #13348 (a one-tag union with several values described by its first
  only: the build scanned for and refused such unions); #13349 (the glue not `include!`-able and
  lint-dirty: the build rewrote its `//!`/`#![...]` lines and the crate allowed its lints);
  #13350 (a tag named `Release` colliding with the glue's `<Type>Release` helper: the event's
  tag was `Released`). The entries below describe them as they were at b5c77fc.
- **`F64` rounding.** `round_to_*_try` rounds by `floor(x + 0.5)` (`Builtin.roc`,
  `f64_round_to_whole`), which differs from Rust's `f64::round` at `0.49999999999999994` and
  loses the sign of a zero; there is no `F64 -> F64` round or floor. The geometry writes Rust's
  rounding out (`Geometry.round`) to stay bit-identical.
- **`F64.max` / `F64.min`** are `if a > b then a else b` (`Builtin.roc`): NaN and signed-zero
  behaviour is not specified; the geometry writes out arm64's `fmaxnm`/`fminnm`.
- **Glue and packaging.** The glue is generated at build time (`crates/rift-core-roc/build.rs`,
  into `OUT_DIR`, never committed) from `RustGlue.roc` in a roc-lang/roc checkout (`ROC_SRC`; the
  nightly tarball has none). At b5c77fc the generated file was a module (inner `//!` docs and
  `#![...]` attributes) that the build script rewrote for `include!`, with the generator's lints
  allowed on the module; since ff5e544 (roc#13349) it is included as generated, no lint allowed.
  The glue's default `make_roc_host` allocators prefix their own header, so the
  host keeps its own allocator hooks (mixing the two crashed in `free`).
- **Glue names.** An anonymous record is named by hash (`AnonStruct<hex>`); an inline union by
  its tags (`NothingOrSome`), and a collision, or a generic nominal's instances (`Fact(a)`), by
  a number (`AskedOrUnaskedType24`, `TypesFactType26`) that shifts as other types come and go.
  Workaround: every boundary type is a named nominal in `roc/platform/Boundary.roc`, one per
  payload, and the build fails on `AnonStruct<hex>` or `...Type<digits>` in the glue.
- **Glue bug: a single-tag union with several values** (`[Stepped(Box(model), List(U64))]`) is
  described by its first value only: the glue declared `roc_step` as returning just the box.
  Cause: `src/glue/platform/TypeTable.roc:89`, `single_variant_payload`, keeps only
  `List.first(tag.payload)`, and `RustGlue.roc:159` emits that (reported as roc#13348, fixed by
  PR 13386, in ff5e544). Such values are a named record; the build's guard against such unions
  went with the bump.
- **`roc check` hung** on a nominal type in a platform module that mentions the platform's
  `Model` variable (roc-lang/roc#13347, fixed in 8712e637, in nightly-2026-11-23-b5c77fc). The
  workaround, the entries' results declared in `Boundary` generic in the model (`Stepped(model)`
  and its kin), went with the bump: they are declared in the platform module on `Model`,
  `Windows`, `Joined` and `Mains`. All but one: **roc-lang/roc#13363** (in b5c77fc; fixed upstream
  in 3ef41a6, PR #13389, merged 2026-11-24, in no nightly yet) rejects a provided function
  answering a bare type as "polymorphic" when a type declared in the platform module wraps that
  type (30-line repro: `S := { c : C }` with `C` from an imported module makes `net : U64 -> C`
  polymorphic; a type parameter on `S` does not help). `LifecycleStepped` wraps `Change`, which
  `lifecycle_net` answers bare, so it stayed in `Boundary`, generic, until ff5e544, where it moved.
  Also check #13249 on a bump (an `--opt=dev` SIGTRAP when a platform closure captures a
  Model-typed callback; closed).
- **`roc build` hung** (`roc check` passing) on an entry rebuilding tuples out of a nominal
  record's fields (the settle question's placement); giving the core's own type the record
  shape removed the hang. Not reduced further.
- **A tag named after a Rust keyword** becomes a payload field of that name (`move`, `in`), which
  does not compile; such tags are renamed at the boundary (`Moved` for upstream's `Move` in
  `BspEdit` and `BspDropAction`, `Placed` for the floating slot).
- **A tag named `Release`** gave a per-variant alias (`BoundaryEventRelease`) that collided
  with the glue's own `<Type>Release` helper struct (roc#13350); the event's tag was `Released`
  until ff5e544 (PR 13390), and is `Release` since.
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

### Roc issue drafts (2026-11-25; the owner decides about filing)

Two glue problems seen in the group forest's slice 4, each reduced to a platform and app outside
rift and checked at the pinned nightly (`nightly-2026-11-24-ff5e544`) and the newest
(`nightly-2026-11-25-d5a4f1e`): the same output at both. Neither is roc-lang/roc#13348 coming
back: that issue's repro (a one-tag union with two payloads) still gives a `Stepped` struct with
both at both nightlies. No open or closed roc-lang/roc issue reports either (searched
2026-11-25).

**(a) A one-tag union's alias names the element of its `List` or `Box` payload.** A new bug,
not #13348's (several payloads; this is one). Flattening the union to its payload is the glue's
design (`TypeTable.declares_tag_union`: "Every other tag union is its payload's type"), and
every function signature and record field is right; only the generated `pub type` aliases are
wrong. Rift's way round it (since slice 4): every answer union has several tags.

> **roc glue: a one-tag union's type alias names the element of its List or Box payload**
>
> `nightly-2026-11-25-d5a4f1e` (and `nightly-2026-11-24-ff5e544`), macOS arm64, `RustGlue.roc`
> at the same commit.
>
> `platform/Abi.roc`:
>
> ```roc
> Abi := [].{
> 	Rect := { x : F64, y : F64, w : F64, h : F64 }
> 	Frame := { window : U64, frame : Rect }
> 	Answer := [Frames(List(Frame))]
> 	Boxed := [Boxed(Box(Frame))]
> }
> ```
>
> `platform/main.roc`:
>
> ```roc
> platform ""
> 	requires {} { answer : U64 -> Answer, boxed : U64 -> Boxed }
> 	exposes [Abi]
> 	packages {}
> 	provides { "roc_answer": answer_for_host, "roc_boxed": boxed_for_host }
> 	targets: {
> 		arm64mac: { inputs: [app], output: Archive },
> 	}
>
> import Abi exposing [Answer, Boxed]
>
> answer_for_host : U64 -> Answer
> answer_for_host = |n| answer(n)
>
> boxed_for_host : U64 -> Boxed
> boxed_for_host = |n| boxed(n)
> ```
>
> ```sh
> roc glue src/glue/src/RustGlue.roc out platform/main.roc
> ```
>
> ### Actual
>
> The functions return the payloads, but the aliases name the list's and the box's element:
>
> ```rust
> pub type AbiAnswer = AbiFrame;
> pub type AbiAnswerFrames = AbiFrame;
> pub type AbiBoxed = AbiFrame;
> pub type AbiBoxedBoxed = AbiFrame;
>
> pub fn roc_answer(arg0: u64) -> RocListWith<AbiFrame, false>;
> pub fn roc_boxed(arg0: u64) -> *mut AbiFrame;
> ```
>
> So `let a: AbiAnswer = roc_answer(1);` does not compile. A payload with no named type in it
> (`[Words(List(U64))]`, `[Word(U64)]`, `[Text(Str)]`) gets no alias at all; a record payload
> (`Record := [One(Frame)]`) gets the right one (`AbiRecord = AbiFrame`). A record field of the union's type
> is right (`pub answer: RocListWith<AbiFrame, false>`).
>
> ### Expected
>
> Each alias names the type the functions use:
>
> ```rust
> pub type AbiAnswer = RocListWith<AbiFrame, false>;
> pub type AbiAnswerFrames = RocListWith<AbiFrame, false>;
> pub type AbiBoxed = *mut AbiFrame;
> pub type AbiBoxedBoxed = *mut AbiFrame;
> ```
>
> (and, for consistency, `AbiWords = RocListWith<u64, false>`, or no aliases for one-tag unions).

**(b) `RocList<T>::decref` is shallow: by design and documented, so not a bug as such.** The
generated text says so ("Shallow: it never touches the elements. To release an owned list whose
elements are refcounted, call that list's generated `decref_list_of_...` helper instead"), the
deep helper exists for every list of refcounted elements at the boundary
(`decref_list_of_abi_bar`, built on `release_with`), and a generated union's `decref` is deep.
The repro below shows the leak and the right call. What is left is a sharp edge, which the owner
may or may not want to raise: `RocList<T>` is `RocListWith<T, true>`, the glue's own marker for
"the elements hold references", yet its `decref` frees the spine and leaks every element when it
drops the last reference. Rift's `boundary/group_forest.rs` calls only the deep releases now (no
`decref` on a list field anywhere in `rift-core-roc`).

> **RustGlue: `RocList<T>::decref` leaks the elements of a list whose elements hold references**
>
> `nightly-2026-11-25-d5a4f1e` (and `nightly-2026-11-24-ff5e544`), macOS arm64, `RustGlue.roc`
> at the same commit.
>
> `platform/Abi.roc`:
>
> ```roc
> Abi := [].{
> 	Bar := { windows : List(U64) }
> }
> ```
>
> `platform/main.roc`:
>
> ```roc
> platform ""
> 	requires {} { bars : U64 -> List(Bar) }
> 	exposes [Abi]
> 	packages {}
> 	provides { "roc_bars": bars_for_host }
> 	targets: {
> 		arm64mac: { inputs: [app], output: Archive },
> 	}
>
> import Abi exposing [Bar]
>
> bars_for_host : U64 -> List(Bar)
> bars_for_host = |n| bars(n)
> ```
>
> `app.roc`:
>
> ```roc
> app [bars] { pf: platform "platform/main.roc" }
>
> import pf.Abi exposing [Bar]
>
> bars : U64 -> List(Bar)
> bars = |n| [{ windows: [n, n + 1] }, { windows: [n + 2] }]
> ```
>
> ```sh
> roc build --target=arm64mac --output=libapp.a app.roc
> roc glue src/glue/src/RustGlue.roc glue platform/main.roc
> ```
>
> A Rust host (`include!`s the glue, links `libapp.a`, its `roc_alloc` / `roc_dealloc` count
> live allocations):
>
> ```rust
> let bars = unsafe { abi::roc_bars(1) };            // 3 live: the list, two window lists
> unsafe { bars.decref(&host) };                      // 2 live
>
> let bars = unsafe { abi::roc_bars(1) };
> unsafe { abi::decref_list_of_abi_bar(bars, &host) } // 0 live
> ```
>
> ### Actual
>
> `decref` frees the list and leaves both window lists allocated, with nothing left that refers
> to them. Its doc comment says it is shallow, but the type is `RocListWith<AbiBar, true>`: the
> glue knows the elements hold references.
>
> ### Expected
>
> One of: `decref` on `RocListWith<T, true>` releases the elements too (as a union's generated
> `decref` does); or the shallow release has a name that says so (`decref_spine`) and only
> `RocListWith<T, false>` has a plain `decref`.

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
| Layout trees into the core | L+ | **Re-planned 2026-11-24 (Handoff).** The pre-#688 core trees are deleted (no caller since merge (b)). BSP: the core tree (ported from #688's `systems/bsp.rs`), its Roc mirror, the flip (saved from the core), the shape in Roc (under `roc-core` the Roc tree decides) and upstream's system deleted are done; next traditional (stack, master-stack, traditional), on the lead's go. Then stack, traditional and master-stack, scrolling's columns, floating. |
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

### Lessons from manual QA (2026-11-24)

Items to schedule, not started (the lead's, from the day's manual QA):

1. **One `Appeared` fact at the boundary.** Upstream admits a new window by three paths: AX
   `WindowCreated`, `WindowsDiscovered` (the inventory refresh after `WindowServerAppeared` /
   `FocusChanged`) and main-window-changed registration. A gate on one path missed Chrome's
   tear-off, which arrives through the inventory refresh (#689, PR #713). The platform should
   parse all three into one `Appeared` fact. A window appearing under a held left button should
   be an admission fact (`Vetoed(UnderPress)`) and the hand's held window. The fork matches
   upstream, so it has #689 today.
2. **The actual frame from more than one witness.** With TextEdit frozen (SIGSTOP), a window
   dragged and then thawed reports a stale AX frame (its tile) while the window server knows it
   moved; rift trusts AX and never retiles. Consider letting the window server's bounds feed the
   actual-frame fact.
3. **A fuzzer invariant: every window id in a layout exists in the window store.** On upstream
   main a Safari tab-drag preview window stayed in a master slot (a 0 × 0 frame) after the store
   had forgotten it.
4. **Scrolling's parking on several displays.** Upstream parks hidden columns left of the
   display (x = -1439, inside the neighbouring display's area), and macOS keeps them at x = 0
   behind the visible column. Check what `translate`'s `Park | InPlace` does with neighbours on
   both sides.
5. **Record and replay real sessions.** Differentials prove "same as upstream" and inherit
   upstream's blind spots. rift has `--record`: plan a replay harness in which recorded QA
   sessions (the Chrome tear-off, the scrolling close) become fixtures, so each manual find is a
   permanent test through the real event sequence.
