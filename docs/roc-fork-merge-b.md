# Merge (b): upstream #688, #700 and #702 into this line (2026-11-21)

Every place the merge adopted, kept or set aside upstream behaviour, in the classes the lead ruled (D1 the mode table over `refresh_ffm`; D2 #702's behaviour under our state, each instance; D3 reconciliation and drag ours; D4 handler decisions that differ, listed). Then the test triage and the plants.


## D1 (ours: the mode table replaces refresh_ffm)
- events/system.rs handle_menu_closed: no ffm refresh (upstream returns bool -> refresh_ffm).

## D2 (#702 behaviour adopted under our state; each instance)
- (h) config reload with changed app rules re-applies rules to every existing window (#702 command.rs apply_config).
- (i) a frame change that moves a window to another active Space goes through
  reassign_window_to_authoritative_space (instead of remove-preserving-floating + add to that Space's active workspace).
- (a) MainWindowTracker: WindowDestroyed no longer clears window_server_focus there; retire/rekey call
  replace_window(wid, replacement), which also clears/rekeys the app's main_window.

## D3 (ours)
- claims.rs: #702 cancels a pending transfer in the window store (cancel_transfer); ours: drag/transfer state is our core's.
- transaction_manager.rs: ours (#700's begin/invalidate_for_window not taken).

## D4 (handler decides differently; listed for the lead)

## Method notes
- Handlers keep returning EventOutcome; dispatch folds outcomes into #700's per-operation Changes.
  - (h) as an outcome field `reprocess_app_rules`; the reactor runs process_windows_for_app_rules per app.
  - (i) note: with #702 a move to an inactive Space or to no Space no longer removes the window from its layout here.
- (j) native departure (space.rs): fullscreen departure asks for an inventory instead of WindowMaybeDestroyed;
  Closed retires (outcome.retirements; upstream retire_window, which skips destroy_window's fullscreen-suspension try);
  Hidden reconciles membership (outcome.memberships (wid, None)) and asks for an inventory (remove_projection gone).
- (k) native appearance (space.rs): known-window handling restructured like #702: a known window during Mission
  Control is ignored outright; fullscreen appearance takes the pid also from the event's info and returns early
  without one; suspension goes through membership reconciliation (not LayoutEvent::WindowSuspended).
- D3 events/drag.rs: #702 rewrote finish_drag (drop display from the frame's best Space, a native move's
  crossing corroborated by a complete membership sample + window_spaces, NativeResize storing a floating
  window's position, the transfer fence after a crossing). Ours kept (pointer's display, item 9). Worth
  checking later against ours: NativeResize storing the floating position on release.
- (b) can_reconcile_admission: AX-valid, unclaimed, not minimized (the core's Minimize fact); the
  WindowState-level check (transient etc.) no longer gates it.
- (c) WindowStore::reconcile_admission removed; membership goes through reconcile_window_membership.
- (d) NativeDeparture::Hidden(window): remove_projection gone.
- (f) observe_minimized -> set_minimized_visibility (visibility only); minimized is the core's Minimize fact,
  never a write to manageability.
- (l) can_move_assigned_window (new, #702): expressed as the core's admission with minimize forced Shown,
  Admitted or Vetoed(Unmanageable) (= upstream allows_management: transient and rule vetoes still apply,
  the heuristic does not).
- (m) can_place_window gains #702's conditions: AX-valid, unclaimed, admitted (or no AX state).
- (n) invalidate_ax_identity returns whether it newly invalidated (the core's Presence transition).
- (o) inventory_retirements: #702's signature (&self, inactive as a predicate, returns windows); the verdict
  stays the core's retirement with the Roc check.
- (p) minimization (events/window.rs visibility handler = #702 observe_window_minimization): minimizing an
  already-minimized window re-reconciles membership (was: no-op); the heuristic is refreshed on minimize too;
  the return path is gated by can_reconcile_admission (AX-valid, unclaimed, shown) instead of "unclaimed".
  Not ported: #702's overview_changed flag on an unavailability change (ours has no overview flag in the outcome).
- (a) retirement: WindowDestroyed now retires at the edge (window_lifecycle::retire_window): store (through
  RiftState, grip ends via the core), transient focus, tx, then main window / overview focus / pending refocus
  all let go (MainWindowTracker::replace_window). Rekey: replace_window_identity moves them to the new id.
- (q) AX invalidation suspends through membership reconciliation (only when newly invalidated; refreshes
  notifications), and always asks for an inventory.
- (r) restore_native_fullscreen: previous_assignment (not removed_window); no restorable window ->
  membership reconciliation (was WindowSuspended); rejoin via membership with placement (was WindowAdded,
  which skipped the claimed/ignored checks).
- (s) reassign_window_to_authoritative_space: a non-admitted or unavailable window reconciles membership
  (was reconcile_admission + Removed/Suspended); a window with no owner is admitted on an active Space
  (was: assigned to the Space's active workspace via the move path); also refuses while a hand holds it.
- (t) native presence of an unassigned window admits it (finalization path) instead of a bare WindowAdded.
## Not adopted (yet)
- #700's changes.rs (Changes / Presentation precedence): our EventOutcome remains the per-operation
  accumulator, applied once. Deleted the orphan file; adopting Presentation precedence is a follow-up.
## D4 candidates (listed for the lead, provisional choice in brackets)
- D4-1 (ruled: upstream's) discovery completion: upstream #700 removed LayoutEvent::WindowDiscoveryCompleted, so unmatched persisted
  windows are no longer discarded when an app's discovery completes (only deferred replays in matcher.rs still
  discard). Ours emitted it. [provisionally upstream's: not emitted]
## Kept for our call sites
- engine.rs app_rule_names_workspace restored (removed upstream in #700; our new-window placement question reads it).
- D4-2 (ruled: upstream's) focus commands: upstream #700 arranges after a focus command only where focus changes geometry
  (focus_changes_geometry: scrolling, stacks), else only tabs redraw; ours arranged on any change.
  [provisionally upstream's]
- (g) workspace distribution: balance_display_workspaces / place_workspace ported into our placement
  reconciler (balancing only when a display arrived, restorations excluded); a newly active pin resets the
  workspace's home to automatic (PlacedBy) and a visible pin change rebalances. Saved form writes
  display_manually_selected always (as upstream).
- (u) app quit: retire_app_windows at the edge (transient focus, tx, references, store, AppClosed).
## Test triage (after the build compiled: 29 failing)
- Bugs in our merge, fixed:
  - drag.rs cross-display drop queued WindowRemoved after assigning the destination; the removal cleared the
    new assignment (and #702's WindowAdded then needs it). Now removal only when there is no destination.
    (Fixed every fuzz "live-window-has-a-space" signature.)
  - send_layout_event arranged on every WindowAdded since upstream's engine now reports it as a change;
    ours arranges where admission asks (once per Space). WindowAdded is treated as no geometry change there.
- Test setup outdated by #702 (layout takes only store-owned, admitted windows): insert + assign before
  WindowAdded (gesture setup, engine re-land tests, 5 reactor tests); store test wrote is_minimized directly
  (now set_minimized); save-format fixtures moved to #688's traditional tree format.
- Upstream expectations adopted (#702: no seeding, display balancing): connecting_display..., named_pin_reload
  (renamed ..._without_creating_workspaces), shared_destroyed_desktop..., window_moves_never_relocate...
  (upstream's version incl. the at-capacity case).
- (v) ported ensure_window_move_destination: moving a window to an empty display creates a workspace there
  (global mode), as upstream.
- (w) upstream's resolve_window_index refuses an ambiguous index: the fuzz sim now focuses the window and
  moves the focused one (the simulated apps share indexes).
- names_workspace: one source. AppRuleEffects.names_workspace (upstream, #702) dropped; our new-window
  placement question reads engine.app_rule_names_workspace (restored) on the roc_path.
## Plants after the merge
- Every roc_path check site (27) is reached by the roc-core tests: a temporary test-only hook that fails one
  named site, run per site, failed 4–389 tests at each (hook removed, never committed).
- Behaviour plants, each caught by roc-divergence panics: retirement keep-on-ordering needs invalidated AX
  (Rust core) 4 failures; discovery net dropping the heuristic change 11; lifecycle twin seeing a minimize as
  shown 18. A net that never cancels (heuristic.or(reading)) was not caught at first: no test drove two
  cancelling admission changes through discovery's net site. The test
  `a_reading_whose_roles_admit_and_heuristic_suspends_changes_nothing` closes it (1 failure, roc-divergence).

## Workspace distribution (virtual_workspace.rs), how #702 landed under our types

## virtual_workspace.rs: ours + #688 + #702

`resolved.rs` parses (`rustfmt --check` finds no syntax errors). It has not been compiled.

### (a) Each upstream change and how it landed

**#688**
- `create_layout_system` now builds `Default` systems and calls `configure_layout_system`. New `apply_layout_settings(&mut self, settings, display)`. The `systems` import is added. Applied as upstream wrote it, because ours had not touched this code.

**#702**
- `distribute`: the all-unpinned case now produces contiguous blocks, and it no longer appends empty slots. It ends with `balance_owners`. Applied verbatim.
- `balance_owners` (new) and `plan_display_balance` (new). The `fixed` test inside it is rewritten in our types:
  - `home.placed_by() == Some(PlacedBy::User)`;
  - `rule_standing == RuleStanding::Overridden`;
  - `home.space() != space`;
  - restored workspaces;
  - display rules.
- `WorkspaceTransferReason::DisplayArrival` (new). In `transfer_workspace`, the reason is now handled by an exhaustive `match`:
  - Explicit sets `home = placed(target, display, User)`;
  - DisplayArrival sets `home = placed(target, display, <previous by, else Automatic>)`;
  - Departure and Restoration leave the home alone.
- Explicit move to the same Space: `changed` is still `home != new_home`. `by` is part of `Home`'s equality, so this covers upstream's `!display_manually_selected ||` term.
- Removed: the move refusal at `MAX_WORKSPACES`, the global limit check in `create_workspace`, `seed_space` and its calls, and `auto_assign_window`. `create_workspace` now uses `count` for the layout-mode index.
- `ensure_visible_spaces`: the count is now `default_workspace_count.clamp(1, MAX)`. Comments are updated.
- `find_window_by_idx` is replaced by `resolve_window_index`, verbatim.
- `apply_app_rule_decision` sets `names_workspace: workspace.is_some()`.
- Tests are applied and renamed as upstream did:
  - `moving_the_last_global_workspace_preserves_the_catalog`;
  - `shared_initial_count_stays_fixed_across_topology_changes`;
  - the distribution case, and the 3→2 view count.

**Saved form**
- `SavedWorkspace` gains `#[serde(default)] display_manually_selected: bool`. `SavedWorkspaceRef` writes it on every workspace, in upstream's position, between `home_space` and `rule_overridden`.
- `Home::from_saved` parses it once. If the flag is set on a home with no Space, it is dropped with a `workspace-load: normalized` warning. Upstream never writes that combination.

**Tests**
- Our two "saved form is unchanged" tests now expect `display_manually_selected:false` in the written form. The typed-homes test also gains a `true` case.
- New tests:
  - `display_balance_moves_only_workspaces_rift_placed_and_arrival_keeps_who_chose`;
  - `a_saved_manual_selection_without_a_home_space_is_dropped`.
- The explicit-home test asserts the value of `placed_by`.

### (b) Where upstream fights our types

- **Home**: upstream adds a free `display_manually_selected: bool`. It now lives in the placed variants, as `by: PlacedBy` (`Automatic | User`). That makes "manually selected but never placed" unrepresentable. This matches upstream's invariant: the flag is only ever set together with `home_space`, which is never cleared.
- **DisplayArrival**: upstream leaves the flag as it was. We carry the previous `by` over and use `Automatic` when the old home had no Space. This is equivalent.
- **remap_space / with_space**: these keep `by`. Upstream keeps the flag when it remaps `home_space`.
- **insert_workspace**: always `Automatic`, as in upstream (flag false). It does not copy `by` from the neighbour whose display it borrows.
- **RuleStanding**: `rule_overridden` in `plan_display_balance` is expressed as `== Overridden`. No conflict.
- **SpaceActivation**: removing `seed_space` makes `Vacant` a normal steady state in global mode (an empty display, or the source of the last workspace moved away). Our type already models this, and `repair_activation` yields `Vacant { remembered }`. No change needed. The core shadow is still told through the same helpers.
- **Catalog**: no conflict. `plan_display_balance` iterates `catalog()` (the order).
- **Saved form**: the byte-for-byte round trip changes for every workspace, because `display_manually_selected:false` is now written. Writing it always was chosen to match upstream's files and the other-file tests that compare strings. With `skip_serializing_if` the old round trip would stay unchanged instead. That is a choice for you, not a forced one.

### (c) API changes other files need

- **`Home`**:
  - `Space(SpaceId)` becomes `Space { space, by }`.
  - `OnDisplay { space, display }` gains `by`.
  - `Home::placed(space, display)` becomes `Home::placed(space, display, by: PlacedBy)`.
- **New**: `pub enum PlacedBy { Automatic, User }` (Default = Automatic) and `Home::placed_by() -> Option<PlacedBy>`.
- **Writers of `home` in other files** must choose a `by`, or use `with_space` to keep it. Upstream's equivalents are `engine.rs` (filling in a missing display preference) and `reactor.rs` (re-homing `home_space` to a target). Upstream's reactor moves workspaces with `DisplayArrival` and calls `plan_display_balance`. Code that reads `display_manually_selected` should use `home.placed_by() == Some(PlacedBy::User)`.
- **New enum variant**: `WorkspaceTransferReason::DisplayArrival`. Any exhaustive match on the reason needs an arm.
- **New functions**:
  - `WorkspaceStore::plan_display_balance(&self, &[SpaceId], &HashSet<VirtualWorkspaceId>) -> Vec<(VirtualWorkspaceId, SpaceId)>`;
  - `VirtualWorkspace::apply_layout_settings(&mut self, &LayoutSettings, Option<&str>)`;
  - the private free functions `balance_owners` and `configure_layout_system`.
- **Renamed**: `find_window_by_idx(store, space, idx)` becomes `pub(crate) resolve_window_index(windows, command_space: Option<SpaceId>, is_visible: impl Fn(SpaceId) -> bool, index)`. Callers were `reactor.rs` and `engine.rs`.
- **Removed**: `WorkspaceStore::auto_assign_window` (used by `engine.rs` and its tests) and the private `seed_space`.
- **`AppRuleEffects`** needs a new field, `names_workspace: bool` (in `app_rules.rs`).
- **#688 needs these in the layout systems**:
  - `Default` for every layout system;
  - `set_window_insertion_point` and `set_equalize_nodes`;
  - `update_settings` on Stack, MasterStack and Scrolling;
  - `ScrollingSettings::effective_for_display`.
- **Behaviour**:
  - Global displays are no longer seeded. A display can be `Vacant`, and the catalog stays at `default_workspace_count`.
  - Explicit moves are no longer refused at `MAX_WORKSPACES`.
