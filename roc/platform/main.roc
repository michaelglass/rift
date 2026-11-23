## Rift's core platform: the host (Rust) owns the event loop and every effect; the app is a
## pure step function and pure model-free questions. Everything crosses as the typed values of
## `Boundary`. The model is opaque to
## the host: it holds a `Box(Model)` it can neither read nor free, and hands it back on every
## step.
platform ""
	requires {
		[Model : model, Windows : windows, Mains : mains, Joined : joined] for core : {
			init : {} -> model,
			step : model, Event -> { model : model, effects : List(Effect) },
			bsp : BspShape -> List(Frame),
			traditional : TraditionalShape -> List(Frame),
			strip : Strip -> Placed,
			settle : SettleQuestion -> Settled,
			floating : FloatingQuestion -> List(Rect),
			removal_focus : RemovalQuestion -> RemovalAnswer,
			focus_candidate : CandidateQuestion -> WordAnswer,
			hides_focus : HidesQuestion -> VerdictAnswer,
			activation_follow : ActivationQuestion -> FollowAnswer,
			display_to_follow : DisplayFollowQuestion -> WordAnswer,
			untracked_focus : UntrackedQuestion -> UntrackedAnswer,
			focuses_desktop : DesktopQuestion -> VerdictAnswer,
			discovered_space : DiscoveredQuestion -> DiscoveredAnswer,
			discovery_spaces : DiscoveryQuestion -> DiscoveryAnswer,
			admit_space_snapshot : AdmitQuestion -> Admission,
			app_rule_candidates : CandidatesQuestion -> CandidatesAnswer,
			rule_targets : RuleTargetsQuestion -> RuleTargetsAnswer,
			rule_refreshes_layout : RuleRefreshQuestion -> Bool,
			pointer_display_target : PointerTargetQuestion -> SpaceAnswer,
			is_native_transient : NativeTransientQuestion -> VerdictAnswer,
			keeps_transient : KeepsTransientQuestion -> Bool,
			misplaced : MisplacedQuestion -> Bool,
			out_of_inventory_view : OutOfViewQuestion -> VerdictAnswer,
			reapplies_on_title_change : TitleChangeQuestion -> VerdictAnswer,
			window_admission : AdmissionFacts -> WindowAdmission,
			lifecycle_read : ReadQuestion -> ReadAnswer,
			lifecycle_presence : PresenceQuestion -> PresenceAnswer,
			lifecycle_net : NetQuestion -> Change,
			inventory_retirement : InventoryQuestion -> RetirementAnswer,
			workspace_step : StepQuestion -> WordAnswer,
			workspace_resolve : ResolveQuestion -> Reached,
			workspace_previous : PreviousQuestion -> ReachedAnswer,
			workspace_eligible : EligibleQuestion -> Bool,
			emits_window : EmitsQuestion -> VerdictAnswer,
			quarantines : QuarantineQuestion -> VerdictAnswer,
			target_display : TargetQuestion -> TargetAnswer,
			mouse_raise : RaiseQuestion -> RaiseAnswer,
			joined_steps : JoinedQuestion -> JoinedView,
			lifecycle_init : {} -> windows,
			lifecycle_step : windows, LifecycleEvent -> { model : windows, change : Change },
			lifecycle_invalidate : windows, U64 -> { model : windows, invalidation : Invalidation },
			lifecycle_revalidate : windows, U64 -> { model : windows, revalidation : Revalidation },
			lifecycle_view : windows -> { model : windows, windows : List(PresentWindow) },
			workspaces_init : {} -> joined,
			workspaces_step : joined, JoinedEvent -> { model : joined, outcome : Outcome },
			workspaces_view : joined -> { model : joined, catalog : List(WorkspaceSpace), activations : List(SpaceActivation), owners : List(WindowOwnership), windows : List(WorkspaceWindows) },
			mode_verdict : ModesQuestion -> ModeVerdict,
			main_window_init : {} -> mains,
			main_window_step : mains, MainWindowEvent -> { model : mains, focus : MaybeWord },
			main_window_view : mains -> { model : mains, main_window : MaybeWord, native_focus : NativeFocus, apps : List(MainApp), global_frontmost : MaybeWord },
		}
	}
	exposes [Boundary]
	packages {}
	provides {
		"roc_init": init_for_host,
		"roc_lifecycle_init": lifecycle_init_for_host,
		"roc_lifecycle_step": lifecycle_step_for_host,
		"roc_lifecycle_invalidate": lifecycle_invalidate_for_host,
		"roc_lifecycle_revalidate": lifecycle_revalidate_for_host,
		"roc_lifecycle_view": lifecycle_view_for_host,
		"roc_lifecycle_release": lifecycle_release_for_host,
		"roc_workspaces_init": workspaces_init_for_host,
		"roc_workspaces_step": workspaces_step_for_host,
		"roc_workspaces_view": workspaces_view_for_host,
		"roc_workspaces_release": workspaces_release_for_host,
		"roc_main_window_init": main_window_init_for_host,
		"roc_main_window_step": main_window_step_for_host,
		"roc_main_window_view": main_window_view_for_host,
		"roc_main_window_release": main_window_release_for_host,
		"roc_mode_verdict": mode_verdict_for_host,
		"roc_release_model": release_model_for_host,
		"roc_step": step_for_host,
		"roc_bsp": bsp_for_host,
		"roc_traditional": traditional_for_host,
		"roc_strip": strip_for_host,
		"roc_settle": settle_for_host,
		"roc_floating": floating_for_host,
		"roc_removal_focus": removal_focus_for_host,
		"roc_focus_candidate": focus_candidate_for_host,
		"roc_hides_focus": hides_focus_for_host,
		"roc_activation_follow": activation_follow_for_host,
		"roc_display_to_follow": display_to_follow_for_host,
		"roc_untracked_focus": untracked_focus_for_host,
		"roc_focuses_desktop": focuses_desktop_for_host,
		"roc_discovered_space": discovered_space_for_host,
		"roc_discovery_spaces": discovery_spaces_for_host,
		"roc_admit_space_snapshot": admit_space_snapshot_for_host,
		"roc_app_rule_candidates": app_rule_candidates_for_host,
		"roc_rule_targets": rule_targets_for_host,
		"roc_rule_refreshes_layout": rule_refreshes_layout_for_host,
		"roc_pointer_display_target": pointer_display_target_for_host,
		"roc_is_native_transient": is_native_transient_for_host,
		"roc_keeps_transient": keeps_transient_for_host,
		"roc_misplaced": misplaced_for_host,
		"roc_out_of_inventory_view": out_of_inventory_view_for_host,
		"roc_reapplies_on_title_change": reapplies_on_title_change_for_host,
		"roc_window_admission": window_admission_for_host,
		"roc_lifecycle_read": lifecycle_read_for_host,
		"roc_lifecycle_presence": lifecycle_presence_for_host,
		"roc_lifecycle_net": lifecycle_net_for_host,
		"roc_inventory_retirement": inventory_retirement_for_host,
		"roc_workspace_step": workspace_step_for_host,
		"roc_workspace_resolve": workspace_resolve_for_host,
		"roc_workspace_previous": workspace_previous_for_host,
		"roc_workspace_eligible": workspace_eligible_for_host,
		"roc_emits_window": emits_window_for_host,
		"roc_quarantines": quarantines_for_host,
		"roc_target_display": target_display_for_host,
		"roc_mouse_raise": mouse_raise_for_host,
		"roc_joined_steps": joined_steps_for_host,
	}
	targets: {
		arm64mac: {
			inputs: [app],
			output: Archive,
		},
	}

import Boundary exposing [Stepped, BspShape, Frame, TraditionalShape, Strip, Placed, SettleQuestion, Settled, FloatingQuestion, Rect, ActivationQuestion, CandidateQuestion, DesktopQuestion, DisplayFollowQuestion, FollowAnswer, HidesQuestion, RemovalAnswer, RemovalQuestion, UntrackedAnswer, UntrackedQuestion, VerdictAnswer, WordAnswer, Admission, AdmitQuestion, CandidatesAnswer, CandidatesQuestion, DiscoveredAnswer, DiscoveredQuestion, DiscoveryAnswer, DiscoveryQuestion, AdmissionFacts, Change, InventoryQuestion, KeepsTransientQuestion, MisplacedQuestion, NativeTransientQuestion, NetQuestion, OutOfViewQuestion, PointerTargetQuestion, PresenceAnswer, PresenceQuestion, ReadAnswer, ReadQuestion, RetirementAnswer, RuleRefreshQuestion, RuleTargetsAnswer, RuleTargetsQuestion, SpaceAnswer, TitleChangeQuestion, VerdictAnswer, WindowAdmission, JoinedQuestion, JoinedView, PreviousQuestion, Reached, ResolveQuestion, StepQuestion, MaybeWord, Event, Effect, ReachedAnswer, EligibleQuestion, LifecycleEvent, PresentWindow, LifecycleStepped, LifecycleInvalidated, LifecycleRevalidated, Invalidation, Revalidation, LifecycleViewed, OwnershipEvent, Owned, WindowOwnership, WorkspaceSpace, SpaceActivation, JoinedEvent, Outcome, WorkspaceWindows, WorkspacesStepped, WorkspacesViewed, ModesQuestion, ModeVerdict, MainWindowEvent, NativeFocus, MainApp, MainWindowsStepped, MainWindowsViewed, EmitsQuestion, QuarantineQuestion, TargetQuestion, TargetAnswer, RaiseQuestion, RaiseAnswer]

init_for_host : {} -> Box(Model)
init_for_host = |{}| Box.box((core.init)({}))

## One step: an event in, its effects out. The box is unboxed and boxed again so the model
## stays unique: the host keeps no reference to the old one.
step_for_host : Box(Model), Event -> Stepped(Model)
step_for_host = |boxed, event| {
	stepped = (core.step)(Box.unbox(boxed), event)
	{ model: Box.box(stepped.model), effects: stepped.effects }
}

## The host cannot release a `Box(Model)`: only Roc knows the model's layout.
release_model_for_host : Box(Model) -> {}
release_model_for_host = |_| {}

## A BSP layout's frames.
bsp_for_host : BspShape -> List(Frame)
bsp_for_host = |shape| (core.bsp)(shape)

## A traditional layout's frames.
traditional_for_host : TraditionalShape -> List(Frame)
traditional_for_host = |shape| (core.traditional)(shape)

## Where a scrolling strip places its columns, windows and camera.
strip_for_host : Strip -> Placed
strip_for_host = |strip| (core.strip)(strip)

## Where the camera settles on a strip's placement.
settle_for_host : SettleQuestion -> Settled
settle_for_host = |question| (core.settle)(question)

## A floating layout's newcomers' first frames.
floating_for_host : FloatingQuestion -> List(Rect)
floating_for_host = |question| (core.floating)(question)

## What a removal does to focus.
removal_focus_for_host : RemovalQuestion -> RemovalAnswer
removal_focus_for_host = |question| (core.removal_focus)(question)

## The window to focus in a Space's active workspace.
focus_candidate_for_host : CandidateQuestion -> WordAnswer
focus_candidate_for_host = |question| (core.focus_candidate)(question)

## Whether focus is hidden in an inactive workspace.
hides_focus_for_host : HidesQuestion -> VerdictAnswer
hides_focus_for_host = |question| (core.hides_focus)(question)

## Whether an activation follows its window, and where.
activation_follow_for_host : ActivationQuestion -> FollowAnswer
activation_follow_for_host = |question| (core.activation_follow)(question)

## The display to activate after a command moved the focused window.
display_to_follow_for_host : DisplayFollowQuestion -> WordAnswer
display_to_follow_for_host = |question| (core.display_to_follow)(question)

## What to do about the untracked window under the cursor.
untracked_focus_for_host : UntrackedQuestion -> UntrackedAnswer
untracked_focus_for_host = |question| (core.untracked_focus)(question)

## Whether to focus a Space's desktop.
focuses_desktop_for_host : DesktopQuestion -> VerdictAnswer
focuses_desktop_for_host = |question| (core.focuses_desktop)(question)

## Where an app inventory finds a window.
discovered_space_for_host : DiscoveredQuestion -> DiscoveredAnswer
discovered_space_for_host = |question| (core.discovered_space)(question)

## A reported window's authoritative and discovery Spaces.
discovery_spaces_for_host : DiscoveryQuestion -> DiscoveryAnswer
discovery_spaces_for_host = |question| (core.discovery_spaces)(question)

## Whether a space snapshot is committed, and what it invalidates.
admit_space_snapshot_for_host : AdmitQuestion -> Admission
admit_space_snapshot_for_host = |question| (core.admit_space_snapshot)(question)

## The windows an activation re-applies app rules to, by app.
app_rule_candidates_for_host : CandidatesQuestion -> CandidatesAnswer
app_rule_candidates_for_host = |question| (core.app_rule_candidates)(question)

## The windows app rules run over, by Space.
rule_targets_for_host : RuleTargetsQuestion -> RuleTargetsAnswer
rule_targets_for_host = |question| (core.rule_targets)(question)

## Whether the layout hears of a window after its app rules ran.
rule_refreshes_layout_for_host : RuleRefreshQuestion -> Bool
rule_refreshes_layout_for_host = |question| (core.rule_refreshes_layout)(question)

## The Space a new window moves to.
pointer_display_target_for_host : PointerTargetQuestion -> SpaceAnswer
pointer_display_target_for_host = |question| (core.pointer_display_target)(question)

## Whether a window is a native transient.
is_native_transient_for_host : NativeTransientQuestion -> VerdictAnswer
is_native_transient_for_host = |question| (core.is_native_transient)(question)

## Whether a transient entry survives a departure.
keeps_transient_for_host : KeepsTransientQuestion -> Bool
keeps_transient_for_host = |question| (core.keeps_transient)(question)

## Whether a window's assignment and its native Space disagree.
misplaced_for_host : MisplacedQuestion -> Bool
misplaced_for_host = |question| (core.misplaced)(question)

## Whether an inventory that omits a window says nothing about it.
out_of_inventory_view_for_host : OutOfViewQuestion -> VerdictAnswer
out_of_inventory_view_for_host = |question| (core.out_of_inventory_view)(question)

## Whether a title change re-applies a window's app rules.
reapplies_on_title_change_for_host : TitleChangeQuestion -> VerdictAnswer
reapplies_on_title_change_for_host = |question| (core.reapplies_on_title_change)(question)

## Whether rift manages a window, by its facts.
window_admission_for_host : AdmissionFacts -> WindowAdmission
window_admission_for_host = |question| (core.window_admission)(question)

## A window's facts after a reading, and the change in its admission.
lifecycle_read_for_host : ReadQuestion -> ReadAnswer
lifecycle_read_for_host = |question| (core.lifecycle_read)(question)

## A window's presence after an event, and what the event came to.
lifecycle_presence_for_host : PresenceQuestion -> PresenceAnswer
lifecycle_presence_for_host = |question| (core.lifecycle_presence)(question)

## Two admission changes in a row, as one.
lifecycle_net_for_host : NetQuestion -> Change
lifecycle_net_for_host = |question| (core.lifecycle_net)(question)

## Whether an inventory retires a window.
inventory_retirement_for_host : InventoryQuestion -> RetirementAnswer
inventory_retirement_for_host = |question| (core.inventory_retirement)(question)

## Where stepping through the workspaces lands.
workspace_step_for_host : StepQuestion -> WordAnswer
workspace_step_for_host = |question| (core.workspace_step)(question)

## The workspace a selector names, if reachable.
workspace_resolve_for_host : ResolveQuestion -> Reached
workspace_resolve_for_host = |question| (core.workspace_resolve)(question)

## The workspace navigation history points at, if reachable.
workspace_previous_for_host : PreviousQuestion -> ReachedAnswer
workspace_previous_for_host = |question| (core.workspace_previous)(question)

## What each event of a run met, and the joined workspaces it leaves.
joined_steps_for_host : JoinedQuestion -> JoinedView
joined_steps_for_host = |question| (core.joined_steps)(question)

## Whether a workspace on a Space is reachable from where a command is given.
workspace_eligible_for_host : EligibleQuestion -> Bool
workspace_eligible_for_host = |question| (core.workspace_eligible)(question)

## Whether window discovery emits a window it found on a Space.
emits_window_for_host : EmitsQuestion -> VerdictAnswer
emits_window_for_host = |question| (core.emits_window)(question)

## Whether an unsettled topology quarantines an event.
quarantines_for_host : QuarantineQuestion -> VerdictAnswer
quarantines_for_host = |question| (core.quarantines)(question)

## The display a selector names, and its active Space.
target_display_for_host : TargetQuestion -> TargetAnswer
target_display_for_host = |question| (core.target_display)(question)

## Whether focus-follows-mouse raises the window under the pointer.
mouse_raise_for_host : RaiseQuestion -> RaiseAnswer
mouse_raise_for_host = |question| (core.mouse_raise)(question)

## The lifecycle kept between events (`rift_core::lifecycle::Lifecycle`), empty. Like the
## core's model, it crosses only as a box, unboxed and boxed again on every step so it stays
## unique.
lifecycle_init_for_host : {} -> Box(Windows)
lifecycle_init_for_host = |{}| Box.box((core.lifecycle_init)({}))

## A reading of one window's facts, and the change in its admission.
lifecycle_step_for_host : Box(Windows), LifecycleEvent -> LifecycleStepped(Windows)
lifecycle_step_for_host = |boxed, event| {
	stepped = (core.lifecycle_step)(Box.unbox(boxed), event)
	{ model: Box.box(stepped.model), change: stepped.change }
}

## A window's AX element invalidated: whether it had AX state to invalidate.
lifecycle_invalidate_for_host : Box(Windows), U64 -> LifecycleInvalidated(Windows)
lifecycle_invalidate_for_host = |boxed, window| {
	stepped = (core.lifecycle_invalidate)(Box.unbox(boxed), window)
	{ model: Box.box(stepped.model), invalidation: stepped.invalidation }
}

## A positive AX observation of a window: whether it had been invalidated.
lifecycle_revalidate_for_host : Box(Windows), U64 -> LifecycleRevalidated(Windows)
lifecycle_revalidate_for_host = |boxed, window| {
	stepped = (core.lifecycle_revalidate)(Box.unbox(boxed), window)
	{ model: Box.box(stepped.model), revalidation: stepped.revalidation }
}

## Every window with AX state, and the lifecycle back unchanged.
lifecycle_view_for_host : Box(Windows) -> LifecycleViewed(Windows)
lifecycle_view_for_host = |boxed| {
	viewed = (core.lifecycle_view)(Box.unbox(boxed))
	{ model: Box.box(viewed.model), windows: viewed.windows }
}

## The host cannot release a `Box(Windows)`: only Roc knows its layout.
lifecycle_release_for_host : Box(Windows) -> {}
lifecycle_release_for_host = |_| {}

## The workspaces joined (`rift_core::workspaces::Workspaces`), kept between events, empty.
workspaces_init_for_host : {} -> Box(Joined)
workspaces_init_for_host = |{}| Box.box((core.workspaces_init)({}))

## A change to the joined workspaces: the workspaces after it (unchanged if refused), and what
## the event met.
workspaces_step_for_host : Box(Joined), JoinedEvent -> WorkspacesStepped(Joined)
workspaces_step_for_host = |boxed, event| {
	stepped = (core.workspaces_step)(Box.unbox(boxed), event)
	{ model: Box.box(stepped.model), outcome: stepped.outcome }
}

## The catalog, every Space's activation, every owned window and each workspace's windows, and
## the workspaces back unchanged.
workspaces_view_for_host : Box(Joined) -> WorkspacesViewed(Joined)
workspaces_view_for_host = |boxed| {
	viewed = (core.workspaces_view)(Box.unbox(boxed))
	{ model: Box.box(viewed.model), catalog: viewed.catalog, activations: viewed.activations, owners: viewed.owners, windows: viewed.windows }
}

## The host cannot release a `Box(Joined)`: only Roc knows its layout.
workspaces_release_for_host : Box(Joined) -> {}
workspaces_release_for_host = |_| {}

## Which window is the main one (`rift_core::main_window::MainWindows`), kept between events:
## no apps, no frontmost app, and no native focus reported yet.
main_window_init_for_host : {} -> Box(Mains)
main_window_init_for_host = |{}| Box.box((core.main_window_init)({}))

## An observed event, and the window to focus, if it makes one known.
main_window_step_for_host : Box(Mains), MainWindowEvent -> MainWindowsStepped(Mains)
main_window_step_for_host = |boxed, event| {
	stepped = (core.main_window_step)(Box.unbox(boxed), event)
	{ model: Box.box(stepped.model), focus: stepped.focus }
}

## The main window, the native focus, every app and the globally frontmost app, and the main
## windows back unchanged.
main_window_view_for_host : Box(Mains) -> MainWindowsViewed(Mains)
main_window_view_for_host = |boxed| {
	viewed = (core.main_window_view)(Box.unbox(boxed))
	{ model: Box.box(viewed.model), main_window: viewed.main_window, native_focus: viewed.native_focus, apps: viewed.apps, global_frontmost: viewed.global_frontmost }
}

## The host cannot release a `Box(Mains)`: only Roc knows its layout.
main_window_release_for_host : Box(Mains) -> {}
main_window_release_for_host = |_| {}

## The mode table's verdict on an initiative.
mode_verdict_for_host : ModesQuestion -> ModeVerdict
mode_verdict_for_host = |question| (core.mode_verdict)(question)
