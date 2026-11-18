# Modes: the table, what was deleted, what was kept

Rift's implicit modes (flags, latches, timers and deferred one-shots scattered through the
reactor, the input tap, the app actors and the raise manager) are being replaced by explicit
state. Numbers (#N) refer to the inventory of 30 implicit modes made before this work
(`MODES-inventory.md` in the owner's notes).

The rule: each mode declares which initiatives (Rift's own, and the user's input by class) it
admits. Anything refused is **discarded, never queued** (one debug line, `mode-gate: discarded
…`; hover samples log at trace, since they arrive at mouse rate). When a mode ends, Rift acts on
the facts as they are then; it does not replay what it refused.

## Representation

`src/actor/reactor/modes.rs`. Modes overlap, so the active state is a **product of independent
axes**, one enum per axis, not a set of flags. Each axis implements `Mode::admits` as an
exhaustive match over (initiative, variant) with no wildcard: a new initiative or a new mode
does not compile until its row is written. `Modes::refusal` asks every axis and returns the first
that refuses ("deny if any active mode denies"). `Reactor::modes()` builds `Modes` from the facts
each time it is asked; it is never stored, so it cannot drift from them. State that only one
mode needs lives inside that mode's variant (`MenuState::Open(pid)`,
`WorkspaceSwitchState::Active(origin)`, `Overview::Open { bounds, buttons }`).

Initiatives (`Initiative`): `UserInput(InputClass)`, `Arrange`, `RetireWindows`,
`SwitchWorkspaceFocus`, `WarpFocus`, `FollowActivation`.

Input classes (`InputClass`, parsed once where input enters `handle_loop_event`):
- `Inert`: touches no window and not the pointer (settings, binding modes, diagnostics, saving).
- `Steering`: changes which windows are where, shown, focused or raised, or where the pointer is.
- `Arranging`: steering that rearranges by Space assignment (every layout command,
  `MoveWindowToDisplay`, `MoveWorkspaceToDisplay`).
- `Hover`: the pointer resting over a window (focus follows mouse, autoraise).

## The table

✓ admits, ✗ refuses (in the variant named). Blank = admits in every variant.

| Initiative ↓ / Axis → | Pointer | MenuState | native MissionControlState | Modal | Viewport | FocusClaim | UserSpaces | Wake | WorkspaceSwitch | Topology |
|---|---|---|---|---|---|---|---|---|---|---|
| UserInput(Inert) | | | | | | | | | | |
| UserInput(Steering) | ✗ Pressing, Dragging | | | | | | | | | |
| UserInput(Arranging) | ✗ Pressing, Dragging | | | | | | | | | ✗ Unsettled |
| UserInput(Hover) | ✗ Pressing, Dragging | ✗ Open | ✗ Active | ✗ Frontmost | ✗ Gesturing | ✗ PausesHover | | | | ✗ Unsettled |
| Arrange | ✗ Dragging | | | | | | | | | |
| RetireWindows | ✗ Dragging | | ✗ Active | | | | ✗ None | | | |
| SwitchWorkspaceFocus | ✗ Dragging | | | | | | | | | |
| WarpFocus | ✗ Dragging | | | | | | | | | |
| FollowActivation | ✗ Pressing, Dragging | | | | | | | ✗ AwaitingInput | ✗ Switching | |

Axes, and where each value comes from:

| Axis | Variants | Derived from | Inventory # |
|---|---|---|---|
| `Pointer` | Idle, Pressing(w), Dragging(w) | the window store's grip | #9 (#10 deleted earlier) |
| `MenuState` | Closed, Open(pid) | menu open/close notifications | #17 |
| `MissionControlState` | Inactive, Active | macOS Mission Control enter/exit | #1 (hover, retirement and preview parts), #11 |
| `Modal` | Clear, Frontmost | a visible sheet/popover of the frontmost app on an active Space | #18 |
| `Viewport` | Free, Gesturing | an unreleased scrolling-viewport gesture | #19 |
| `FocusClaim` | Free, PausesHover | the focused window's `SUPPRESS_FOCUS_FOLLOWS_MOUSE` claim | #28 (FFM part) |
| `UserSpaces` | Shown, None | whether any screen shows a user Space | #22 |
| `Wake` | UserPresent, AwaitingInput | wake / session active → first pointer, key or command | #7 |
| `WorkspaceSwitch` | Idle, Switching | the in-flight switch generation, or a manual switch | #8 |
| `Topology` | Settled, Unsettled | `refreshes_blocked()` (authoritative sample, revision guard) | #3 (input half) |

Modes outside the reactor's table, and why:

| Where | Mode | Why not a row |
|---|---|---|
| input tap | `Startup { Registering, Settling, Running }` (#13, #14) | Decides what the tap subscribes to before any event exists. Forward-only. Ends on a fixed 600 ms delay, kept. |
| input tap | `Overview { Closed, Open { bounds, buttons } }` (#12) | Rift's overview reinterprets raw pointer/scroll/gesture/key events in the tap; the events it takes never become reactor input. |
| input tap | FFM pause key held (`ffm_paused`, #15) | A held chord, read per pointer sample in the tap. |
| reactor, per window | `pending_mouse_focus: Option<Expecting<WindowId>>` (#20) | Refuses repeat hovers over one window until its raise completes or 1 s: per window, not per class. |
| reactor | `overview_focus: Option<Expecting<(SpaceId, WindowId)>>` (#21) | Reinterprets arranges as instant, does not refuse them. |
| app actor | `last_activated`, `PendingActivation::Ax` (#26) | Reinterpret an activation as Rift's own (quiet) within 1 s. |
| app actor, per window | animating (#27) | Per window, in the app actor. Not touched. |

## Deleted

- `event_processing_enabled` and the `hotkeys_active` latch (one forward-only `Startup` enum);
  `SetEventProcessing(false)` (redundant), the startup drag cancel (unreachable).
- The release mask bit's dependency on startup: releases are always delivered (bug: a drag rift
  saw start in the first 600 ms could miss its end). `build_event_mask`'s release flag.
- The derived focus-follows-mouse switch pushed to the input actor (#16):
  `SetFocusFollowsMouseEnabled`, `focus_follows_mouse_enabled`, `update_focus_follows_mouse_state`,
  `set_focus_follows_mouse_enabled`, the outcome's `refresh_focus_follows_mouse`,
  `with_claim_focus_policy`, the transient/viewport checks in `MouseFocusPending` and
  `MouseMoved`, the menu check in `should_raise_on_mouse_over`.
- `MissionControlState::Transitioning` (never constructed).
- `StaleCleanupState` and its three writers (derived `UserSpaces`); the separate Mission Control
  check in the inventory path.
- `DragManager::preview_suppressed` and `suppress_preview` (the preview reads Mission Control).
- Four hand-written 1 s `Instant` comparisons (one `Expecting<T>` type, `common/expecting.rs`).
- The replay of a refocus an explicit focus superseded (#23).
- `pending_focus_mouse_warp` and `pending_display_warp` (one `Option<OwedWarp>`, its target read
  when paid) (#24).
- FIFO replay of superseded queued focus requests in the raise manager (#25).
- `suppress_auto_workspace_switch_until_input` and three hand-written auto-switch guards (#7, #8);
  `pending_workspace_switch_origin` (the origin lives in `WorkspaceSwitchState::Active`).
- `mission_control_active`, `mission_control_bounds`, `overview_buttons` in the input actor (one
  `Overview` enum); `Reactor::set_mission_control_active` and the branch through which dismissing
  Rift's overview cleared macOS's Mission Control state (bug).
- Layout commands, move-to-display commands and hover from the topology quarantine's list (now a
  logged row of the table).

## Kept on purpose

- **The 600 ms startup delay** (#13). It is a delay, not a fact; a fact-based end ("every app
  discovered at registration has attached") needs the app actors to report attachment.
- **The Mission Control snapshot hold and delta merge** (#1, #2). Not discarded: see the design
  question below. Geometry and membership in a held snapshot are already latest-wins; only the
  deltas (Space remaps, resizes, destroyed Spaces, refresh flags) are merged.
- **The topology quarantine of reports** (#3, other half): window lifecycle and frame reports,
  press reports and every gesture phase. They are reports, not input classes, and the revision
  guard with the spaces actor depends on them not being applied.
- **Stacking-only raise requests stay queued in order** (#25). They carry no intent, only the
  z-order a layout pass needs; dropping one leaves windows behind others until the next layout.
- **A pending refocus a response did not consume** (#23) when nothing superseded it (the response
  only raised windows). Its target is re-derived when it runs.
- **`focus_grace`** in the raise manager. Not an ignore window: a one-time 250 ms → 1 s timeout
  extension for the last focus of a sequence.
- **#29 parked transfers**: owned by the transfers work (`rust-followups`), not touched.

## Open design questions

1. **#1/#2: can the Mission Control hold re-derive instead of merge?** A remap is the only record
   that "desktop 2 on display A is now Space 57". A fresh snapshot carries only the delta since the
   producer's previous sample, and a display UUID does not tell several desktops on one display
   apart. Dropping a held sample's remaps would orphan the workspaces keyed by the old Space ids.
   Re-derivation needs an identity for a desktop that survives Mission Control (e.g. the spaces
   actor publishing an absolute map from a stable desktop identity to the current Space id, or
   the reactor keying workspaces by that identity rather than by Space id). Until then the merge is
   the correct behaviour.
2. **#3: should every steering command be refused while the topology is unsettled?** Today only
   `Arranging` (layout and move-to-display) is; `FocusWindow`, `SwitchSpace`, `CloseWindow` and
   the rest still pass, as they did under the quarantine.
4. **#13: what fact should end startup?** See "Kept on purpose".
