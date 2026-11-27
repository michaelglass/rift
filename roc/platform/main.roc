## Rift's core platform: the host (Rust) owns the event loop and every effect; the app is a
## pure step function and pure model-free questions. Everything crosses as the typed values of
## `Boundary`. The model is opaque to
## the host: it holds a `Box(Model)` it can neither read nor free, and hands it back on every
## step.
platform ""
	requires {
		[Model : model, Windows : windows, Mains : mains, Joined : joined, Trees : trees, Stacks : stacks, Traditionals : traditionals, Masters : masters, Floatings : floatings, Cols : cols, Sizes : sizes, Floats : floats] for core : {
			init : {} -> model,
			step : model, Event -> { model : model, effects : List(Effect) },
			bsp : BspShape -> List(Frame),
			traditional : TraditionalShape -> List(Frame),
			strip : Strip -> Placed,
			settle : SettleQuestion -> Settled,
			floating : FloatingQuestion -> List(Rect),
			axis : AxisQuestion -> List(F64),
			translate : TranslateQuestion -> List(Frame),
			floating_layout : FloatingLayout -> List(Frame),
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
			sizes_init : {} -> sizes,
			sizes_step : sizes, SizesEvent -> { model : sizes, changed : Bool },
			sizes_view : sizes -> { model : sizes, windows : List(SizeRecord), app_minimums : List(AppMinimum), revision : U64 },
			floats_init : {} -> floats,
			floats_step : floats, FloatsEvent -> floats,
			floats_view : floats -> { model : floats, floating : List(U64), fullscreen : List(FloatFullscreenEntry), active : List(FloatSpaceWindow), floating_focus : MaybeWord, focused : MaybeWord, frames : List(FloatFrameEntry) },
			narrow : F64 -> F32,
			bsp_tree_init : BspInsertion -> trees,
			bsp_tree_seed : BspTreeSeed -> trees,
			bsp_tree_step : trees, BspTreeEvent -> { model : trees, answer : BspTreeAnswer },
			bsp_tree_view : trees -> { model : trees, layouts : List(BspLayoutSlot), layout_free : List(U32), nodes : List(BspNodeSlot), node_free : List(U32), left_out : List(U64), insertion : BspInsertion, aspect : Aspect },
			bsp_tree_frames : trees, BspFramesQuestion -> { model : trees, frames : List(Frame) },
			stack_tree_init : StackSettings -> stacks,
			stack_tree_seed : StackTreeSeed -> stacks,
			stack_tree_step : stacks, StackTreeEvent -> { model : stacks, answer : StackTreeAnswer },
			stack_tree_view : stacks -> { model : stacks, layouts : List(StackLayoutSlot), layout_free : List(U32), next_node : U32, settings : StackSettings, left_out : List(U64) },
			stack_tree_frames : stacks, StackFramesQuestion -> { model : stacks, frames : List(Frame) },
			traditional_tree_init : TraditionalSettings -> traditionals,
			traditional_tree_seed : TraditionalSeed -> traditionals,
			traditional_tree_step : traditionals, TraditionalEvent -> { model : traditionals, answer : BspTreeAnswer },
			traditional_tree_view : traditionals -> { model : traditionals, view : TraditionalSeed },
			traditional_tree_layout : traditionals, GroupLayoutQuestion -> { model : traditionals, answer : GroupLayoutAnswer },
			master_stack_init : MasterSettings -> masters,
			master_stack_seed : MasterStackSeed -> masters,
			master_stack_step : masters, MasterStackEvent -> { model : masters, answer : BspTreeAnswer },
			master_stack_view : masters -> { model : masters, view : MasterStackSeed },
			master_stack_layout : masters, GroupLayoutQuestion -> { model : masters, answer : GroupLayoutAnswer },
			floating_tree_init : {} -> floatings,
			floating_tree_seed : FloatingSeed -> floatings,
			floating_tree_step : floatings, FloatingEvent -> { model : floatings, answer : FloatingAnswer },
			floating_tree_view : floatings -> { model : floatings, view : FloatingSeed },
			floating_tree_layout : floatings, FloatingLayoutQuestion -> { model : floatings, answer : FloatingLayoutAnswer },
			columns_init : ColumnsSettings -> cols,
			columns_seed : ColumnsSeed -> cols,
			columns_step : cols, ColumnsEvent -> { model : cols, answer : ColumnsAnswer },
			columns_view : cols -> { model : cols, view : ColumnsSeed },
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
		"roc_sizes_init": sizes_init_for_host,
		"roc_sizes_step": sizes_step_for_host,
		"roc_sizes_view": sizes_view_for_host,
		"roc_sizes_release": sizes_release_for_host,
		"roc_floats_init": floats_init_for_host,
		"roc_floats_step": floats_step_for_host,
		"roc_floats_view": floats_view_for_host,
		"roc_floats_release": floats_release_for_host,
		"roc_narrow": narrow_for_host,
		"roc_bsp_tree_init": bsp_tree_init_for_host,
		"roc_bsp_tree_seed": bsp_tree_seed_for_host,
		"roc_bsp_tree_step": bsp_tree_step_for_host,
		"roc_bsp_tree_view": bsp_tree_view_for_host,
		"roc_bsp_tree_frames": bsp_tree_frames_for_host,
		"roc_bsp_tree_release": bsp_tree_release_for_host,
		"roc_stack_tree_init": stack_tree_init_for_host,
		"roc_stack_tree_seed": stack_tree_seed_for_host,
		"roc_stack_tree_step": stack_tree_step_for_host,
		"roc_stack_tree_view": stack_tree_view_for_host,
		"roc_stack_tree_frames": stack_tree_frames_for_host,
		"roc_stack_tree_release": stack_tree_release_for_host,
		"roc_traditional_tree_init": traditional_tree_init_for_host,
		"roc_traditional_tree_seed": traditional_tree_seed_for_host,
		"roc_traditional_tree_step": traditional_tree_step_for_host,
		"roc_traditional_tree_view": traditional_tree_view_for_host,
		"roc_traditional_tree_layout": traditional_tree_layout_for_host,
		"roc_traditional_tree_release": traditional_tree_release_for_host,
		"roc_master_stack_init": master_stack_init_for_host,
		"roc_master_stack_seed": master_stack_seed_for_host,
		"roc_master_stack_step": master_stack_step_for_host,
		"roc_master_stack_view": master_stack_view_for_host,
		"roc_master_stack_layout": master_stack_layout_for_host,
		"roc_master_stack_release": master_stack_release_for_host,
		"roc_floating_tree_init": floating_tree_init_for_host,
		"roc_floating_tree_seed": floating_tree_seed_for_host,
		"roc_floating_tree_step": floating_tree_step_for_host,
		"roc_floating_tree_view": floating_tree_view_for_host,
		"roc_floating_tree_layout": floating_tree_layout_for_host,
		"roc_floating_tree_release": floating_tree_release_for_host,
		"roc_columns_init": columns_init_for_host,
		"roc_columns_seed": columns_seed_for_host,
		"roc_columns_step": columns_step_for_host,
		"roc_columns_view": columns_view_for_host,
		"roc_columns_release": columns_release_for_host,
		"roc_mode_verdict": mode_verdict_for_host,
		"roc_release_model": release_model_for_host,
		"roc_step": step_for_host,
		"roc_bsp": bsp_for_host,
		"roc_traditional": traditional_for_host,
		"roc_strip": strip_for_host,
		"roc_settle": settle_for_host,
		"roc_floating": floating_for_host,
		"roc_axis": axis_for_host,
		"roc_translate": translate_for_host,
		"roc_floating_layout": floating_layout_for_host,
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

import Boundary exposing [AxisQuestion, TranslateQuestion, FloatingLayout, BspShape, Frame, TraditionalShape, Strip, Placed, SettleQuestion, Settled, FloatingQuestion, Rect, ActivationQuestion, CandidateQuestion, DesktopQuestion, DisplayFollowQuestion, FollowAnswer, HidesQuestion, RemovalAnswer, RemovalQuestion, UntrackedAnswer, UntrackedQuestion, VerdictAnswer, WordAnswer, Admission, AdmitQuestion, CandidatesAnswer, CandidatesQuestion, DiscoveredAnswer, DiscoveredQuestion, DiscoveryAnswer, DiscoveryQuestion, AdmissionFacts, Change, InventoryQuestion, KeepsTransientQuestion, MisplacedQuestion, NativeTransientQuestion, NetQuestion, OutOfViewQuestion, PointerTargetQuestion, PresenceAnswer, PresenceQuestion, ReadAnswer, ReadQuestion, RetirementAnswer, RuleRefreshQuestion, RuleTargetsAnswer, RuleTargetsQuestion, SpaceAnswer, TitleChangeQuestion, VerdictAnswer, WindowAdmission, JoinedQuestion, JoinedView, PreviousQuestion, Reached, ResolveQuestion, StepQuestion, MaybeWord, Event, Effect, ReachedAnswer, EligibleQuestion, LifecycleEvent, PresentWindow, Invalidation, Revalidation, OwnershipEvent, Owned, WindowOwnership, WorkspaceSpace, SpaceActivation, JoinedEvent, Outcome, WorkspaceWindows, ModesQuestion, ModeVerdict, MainWindowEvent, NativeFocus, MainApp, SizesEvent, SizeRecord, AppMinimum, FloatsEvent, FloatFullscreenEntry, FloatSpaceWindow, FloatFrameEntry, EmitsQuestion, QuarantineQuestion, TargetQuestion, TargetAnswer, RaiseQuestion, RaiseAnswer, BspTreeSeed, BspInsertion, BspTreeEvent, BspTreeAnswer, BspLayoutSlot, BspNodeSlot, BspTreeNode, Aspect, BspFramesQuestion, StackSettings, StackTreeSeed, StackTreeEvent, StackTreeAnswer, StackLayoutSlot, StackFramesQuestion, TraditionalSettings, TraditionalSeed, TraditionalEvent, MasterSettings, MasterStackSeed, MasterStackEvent, GroupLayoutQuestion, GroupLayoutAnswer, FloatingSeed, FloatingEvent, FloatingAnswer, FloatingLayoutQuestion, FloatingLayoutAnswer, ColumnsSettings, ColumnsSeed, ColumnsEvent, ColumnsAnswer]

## The entries' results: each machine's box and what its step answers. They name the
## platform's model variables, so they live here, not in `Boundary`.

## One step's result: the model's box and the effects.
Stepped := { model : Box(Model), effects : List(Effect) }

## The lifecycle after a reading, and the change in the window's admission.
LifecycleStepped := { model : Box(Windows), change : Change }

## The lifecycle after an invalidation, and what it found.
LifecycleInvalidated := { model : Box(Windows), invalidation : Invalidation }

## The lifecycle after a revalidation, and what it found.
LifecycleRevalidated := { model : Box(Windows), revalidation : Revalidation }

## The lifecycle, unchanged, and every window with AX state.
LifecycleViewed := { model : Box(Windows), windows : List(PresentWindow) }

## The joined workspaces after an event, and what the event met.
WorkspacesStepped := { model : Box(Joined), outcome : Outcome }

## The joined workspaces, unchanged: the catalog, every Space's activation, every owned window
## and each catalogued workspace's assigned windows, in catalog order.
WorkspacesViewed := { model : Box(Joined), catalog : List(WorkspaceSpace), activations : List(SpaceActivation), owners : List(WindowOwnership), windows : List(WorkspaceWindows) }

## The main windows after an event, and the window to focus, if the event makes one known.
MainWindowsStepped := { model : Box(Mains), focus : MaybeWord }

## The main windows, unchanged, and what they hold: the main window, the native focus, every
## app in pid order and the globally frontmost app.
MainWindowsViewed := { model : Box(Mains), main_window : MaybeWord, native_focus : NativeFocus, apps : List(MainApp), global_frontmost : MaybeWord }

## The constraints machine after an event, and whether the records or app minimums changed.
SizesStepped := { model : Box(Sizes), changed : Bool }

## The constraints machine, unchanged, and what it holds: every window's record and each app's
## last minimum (in word order) and its revision.
SizesViewed := { model : Box(Sizes), windows : List(SizeRecord), app_minimums : List(AppMinimum), revision : U64 }

## The floating machine, unchanged, and what it holds (each list in word order).
FloatsViewed := { model : Box(Floats), floating : List(U64), fullscreen : List(FloatFullscreenEntry), active : List(FloatSpaceWindow), floating_focus : MaybeWord, focused : MaybeWord, frames : List(FloatFrameEntry) }

## A BSP layout system's trees after an operation, and what it answered.
BspTreeStepped := { model : Box(Trees), answer : BspTreeAnswer }

## A BSP layout system's trees, unchanged, and a layout's frames.
BspTreeFramed := { model : Box(Trees), frames : List(Frame) }

## A BSP layout system's trees, unchanged: each table's slots and free list, the windows left
## out in order, and the settings.
BspTreeViewed := { model : Box(Trees), layouts : List(BspLayoutSlot), layout_free : List(U32), nodes : List(BspNodeSlot), node_free : List(U32), left_out : List(U64), insertion : BspInsertion, aspect : Aspect }

## A stack layout system's stacks after an operation, and what it answered.
StackTreeStepped := { model : Box(Stacks), answer : StackTreeAnswer }

## A stack layout system's stacks, unchanged: the table's slots and free list, the id counter,
## the settings, the windows left out in order.
StackTreeViewed := { model : Box(Stacks), layouts : List(StackLayoutSlot), layout_free : List(U32), next_node : U32, settings : StackSettings, left_out : List(U64) }

## A stack layout system's stacks, unchanged, and a layout's frames.
StackTreeFramed := { model : Box(Stacks), frames : List(Frame) }

## A traditional layout system's forest after an operation, and what it answered.
TraditionalTreeStepped := { model : Box(Traditionals), answer : BspTreeAnswer }

## A traditional layout system, unchanged, and the system as the platform holds it.
TraditionalTreeViewed := { model : Box(Traditionals), view : TraditionalSeed }

## A master-stack layout system's forest after an operation, and what it answered.
MasterStackStepped := { model : Box(Masters), answer : BspTreeAnswer }

## A master-stack layout system, unchanged, and the system as the platform holds it.
MasterStackViewed := { model : Box(Masters), view : MasterStackSeed }

## A traditional system, unchanged, and what a layout question answers.
TraditionalTreeLaidOut := { model : Box(Traditionals), answer : GroupLayoutAnswer }

## A master-stack system, unchanged, and what a layout question answers.
MasterStackLaidOut := { model : Box(Masters), answer : GroupLayoutAnswer }

## A floating layout system after an operation, and what it answered.
FloatingTreeStepped := { model : Box(Floatings), answer : FloatingAnswer }

## A floating layout system, unchanged, and the system as the platform holds it.
FloatingTreeViewed := { model : Box(Floatings), view : FloatingSeed }

## A floating system, unchanged, and what a layout question answers.
FloatingTreeLaidOut := { model : Box(Floatings), answer : FloatingLayoutAnswer }

## A scrolling layout's arrangements after an operation, and what it answered.
ColumnsStepped := { model : Box(Cols), answer : ColumnsAnswer }

## A scrolling layout's arrangements, unchanged, and the system as the platform holds it.
ColumnsViewed := { model : Box(Cols), view : ColumnsSeed }

init_for_host : {} -> Box(Model)
init_for_host = |{}| Box.box((core.init)({}))

## One step: an event in, its effects out. The box is unboxed and boxed again so the model
## stays unique: the host keeps no reference to the old one.
step_for_host : Box(Model), Event -> Stepped
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

## A floating layout's frames.
floating_layout_for_host : FloatingLayout -> List(Frame)
floating_layout_for_host = |layout| (core.floating_layout)(layout)

## A placed strip's frames on screen.
translate_for_host : TranslateQuestion -> List(Frame)
translate_for_host = |question| (core.translate)(question)

## The lengths of the segments along one axis.
axis_for_host : AxisQuestion -> List(F64)
axis_for_host = |question| (core.axis)(question)

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
lifecycle_step_for_host : Box(Windows), LifecycleEvent -> LifecycleStepped
lifecycle_step_for_host = |boxed, event| {
	stepped = (core.lifecycle_step)(Box.unbox(boxed), event)
	{ model: Box.box(stepped.model), change: stepped.change }
}

## A window's AX element invalidated: whether it had AX state to invalidate.
lifecycle_invalidate_for_host : Box(Windows), U64 -> LifecycleInvalidated
lifecycle_invalidate_for_host = |boxed, window| {
	stepped = (core.lifecycle_invalidate)(Box.unbox(boxed), window)
	{ model: Box.box(stepped.model), invalidation: stepped.invalidation }
}

## A positive AX observation of a window: whether it had been invalidated.
lifecycle_revalidate_for_host : Box(Windows), U64 -> LifecycleRevalidated
lifecycle_revalidate_for_host = |boxed, window| {
	stepped = (core.lifecycle_revalidate)(Box.unbox(boxed), window)
	{ model: Box.box(stepped.model), revalidation: stepped.revalidation }
}

## Every window with AX state, and the lifecycle back unchanged.
lifecycle_view_for_host : Box(Windows) -> LifecycleViewed
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
workspaces_step_for_host : Box(Joined), JoinedEvent -> WorkspacesStepped
workspaces_step_for_host = |boxed, event| {
	stepped = (core.workspaces_step)(Box.unbox(boxed), event)
	{ model: Box.box(stepped.model), outcome: stepped.outcome }
}

## The catalog, every Space's activation, every owned window and each workspace's windows, and
## the workspaces back unchanged.
workspaces_view_for_host : Box(Joined) -> WorkspacesViewed
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
main_window_step_for_host : Box(Mains), MainWindowEvent -> MainWindowsStepped
main_window_step_for_host = |boxed, event| {
	stepped = (core.main_window_step)(Box.unbox(boxed), event)
	{ model: Box.box(stepped.model), focus: stepped.focus }
}

## The main window, the native focus, every app and the globally frontmost app, and the main
## windows back unchanged.
main_window_view_for_host : Box(Mains) -> MainWindowsViewed
main_window_view_for_host = |boxed| {
	viewed = (core.main_window_view)(Box.unbox(boxed))
	{ model: Box.box(viewed.model), main_window: viewed.main_window, native_focus: viewed.native_focus, apps: viewed.apps, global_frontmost: viewed.global_frontmost }
}

## The host cannot release a `Box(Mains)`: only Roc knows its layout.
main_window_release_for_host : Box(Mains) -> {}
main_window_release_for_host = |_| {}

## Window size constraints (`rift_core::constraints::Constraints`), kept between events: no
## records, no app minimums, revision 0.
sizes_init_for_host : {} -> Box(Sizes)
sizes_init_for_host = |{}| Box.box((core.sizes_init)({}))

## An observed event, and whether it changed the records or the app minimums.
sizes_step_for_host : Box(Sizes), SizesEvent -> SizesStepped
sizes_step_for_host = |boxed, event| {
	stepped = (core.sizes_step)(Box.unbox(boxed), event)
	{ model: Box.box(stepped.model), changed: stepped.changed }
}

## Every record, each app minimum and the revision, and the machine back unchanged.
sizes_view_for_host : Box(Sizes) -> SizesViewed
sizes_view_for_host = |boxed| {
	viewed = (core.sizes_view)(Box.unbox(boxed))
	{ model: Box.box(viewed.model), windows: viewed.windows, app_minimums: viewed.app_minimums, revision: viewed.revision }
}

## The host cannot release a `Box(Sizes)`: only Roc knows its layout.
sizes_release_for_host : Box(Sizes) -> {}
sizes_release_for_host = |_| {}

## The floating state (`rift_core::floats::Floats`), kept between events: nothing floating, no
## frames, no focus.
floats_init_for_host : {} -> Box(Floats)
floats_init_for_host = |{}| Box.box((core.floats_init)({}))

## An event: the machine after it.
floats_step_for_host : Box(Floats), FloatsEvent -> Box(Floats)
floats_step_for_host = |boxed, event| Box.box((core.floats_step)(Box.unbox(boxed), event))

## Everything the machine holds, and the machine back unchanged.
floats_view_for_host : Box(Floats) -> FloatsViewed
floats_view_for_host = |boxed| {
	viewed = (core.floats_view)(Box.unbox(boxed))
	{ model: Box.box(viewed.model), floating: viewed.floating, fullscreen: viewed.fullscreen, active: viewed.active, floating_focus: viewed.floating_focus, focused: viewed.focused, frames: viewed.frames }
}

## The host cannot release a `Box(Floats)`: only Roc knows its layout.
floats_release_for_host : Box(Floats) -> {}
floats_release_for_host = |_| {}

## The mode table's verdict on an initiative.
mode_verdict_for_host : ModesQuestion -> ModeVerdict
mode_verdict_for_host = |question| (core.mode_verdict)(question)

## A BSP layout system's trees (`rift_core::layout::bsp_tree::Tree`), kept between operations:
## no layouts, inserting as `insertion` says.
bsp_tree_init_for_host : BspInsertion -> Box(Trees)
bsp_tree_init_for_host = |insertion| Box.box((core.bsp_tree_init)(insertion))

## A BSP layout system as the platform holds it (a load, or a re-seed): the host has parsed
## it (slotmap's refusals, the free list naming the vacant slots).
bsp_tree_seed_for_host : BspTreeSeed -> Box(Trees)
bsp_tree_seed_for_host = |seed| Box.box((core.bsp_tree_seed)(seed))

## One operation of the system: the trees after it, and what it answered.
bsp_tree_step_for_host : Box(Trees), BspTreeEvent -> BspTreeStepped
bsp_tree_step_for_host = |boxed, event| {
	stepped = (core.bsp_tree_step)(Box.unbox(boxed), event)
	{ model: Box.box(stepped.model), answer: stepped.answer }
}

## Each table's slots and free list, the windows left out and the settings, and the trees back
## unchanged.
bsp_tree_view_for_host : Box(Trees) -> BspTreeViewed
bsp_tree_view_for_host = |boxed| {
	viewed = (core.bsp_tree_view)(Box.unbox(boxed))
	{ model: Box.box(viewed.model), layouts: viewed.layouts, layout_free: viewed.layout_free, nodes: viewed.nodes, node_free: viewed.node_free, left_out: viewed.left_out, insertion: viewed.insertion, aspect: viewed.aspect }
}

## A layout's frames, from the trees' own shape (`Tree::shape`) through the BSP geometry, and
## the trees back unchanged.
bsp_tree_frames_for_host : Box(Trees), BspFramesQuestion -> BspTreeFramed
bsp_tree_frames_for_host = |boxed, question| {
	framed = (core.bsp_tree_frames)(Box.unbox(boxed), question)
	{ model: Box.box(framed.model), frames: framed.frames }
}

## The host cannot release a `Box(Trees)`: only Roc knows its layout.
bsp_tree_release_for_host : Box(Trees) -> {}
bsp_tree_release_for_host = |_| {}

## A stack layout system's stacks (`rift_core::layout::stack_tree::Stacks`), kept between
## operations: no layouts, under `settings`.
stack_tree_init_for_host : StackSettings -> Box(Stacks)
stack_tree_init_for_host = |settings| Box.box((core.stack_tree_init)(settings))

## A stack layout system as the platform holds it (a load, or a re-seed): the host has parsed
## it (slotmap's refusals, the free list naming the vacant slots).
stack_tree_seed_for_host : StackTreeSeed -> Box(Stacks)
stack_tree_seed_for_host = |seed| Box.box((core.stack_tree_seed)(seed))

## One operation of the system: the stacks after it, and what it answered.
stack_tree_step_for_host : Box(Stacks), StackTreeEvent -> StackTreeStepped
stack_tree_step_for_host = |boxed, event| {
	stepped = (core.stack_tree_step)(Box.unbox(boxed), event)
	{ model: Box.box(stepped.model), answer: stepped.answer }
}

## The table's slots and free list, the counter, the settings and the windows left out, and the
## stacks back unchanged.
stack_tree_view_for_host : Box(Stacks) -> StackTreeViewed
stack_tree_view_for_host = |boxed| {
	viewed = (core.stack_tree_view)(Box.unbox(boxed))
	{ model: Box.box(viewed.model), layouts: viewed.layouts, layout_free: viewed.layout_free, next_node: viewed.next_node, settings: viewed.settings, left_out: viewed.left_out }
}

## A layout's frames, from the stacks' own shape (`Stacks::shape`) through the traditional
## geometry, and the stacks back unchanged.
stack_tree_frames_for_host : Box(Stacks), StackFramesQuestion -> StackTreeFramed
stack_tree_frames_for_host = |boxed, question| {
	framed = (core.stack_tree_frames)(Box.unbox(boxed), question)
	{ model: Box.box(framed.model), frames: framed.frames }
}

## The host cannot release a `Box(Stacks)`: only Roc knows its layout.
stack_tree_release_for_host : Box(Stacks) -> {}
stack_tree_release_for_host = |_| {}

## Rust's `value as f32`.
narrow_for_host : F64 -> F32
narrow_for_host = |value| (core.narrow)(value)

## A traditional layout system (`rift_core::layout::group_forest::Traditional`), kept between
## operations: no layouts, no windows left out.
traditional_tree_init_for_host : TraditionalSettings -> Box(Traditionals)
traditional_tree_init_for_host = |settings| Box.box((core.traditional_tree_init)(settings))

## A traditional layout system as the platform holds it (the host parses it first).
traditional_tree_seed_for_host : TraditionalSeed -> Box(Traditionals)
traditional_tree_seed_for_host = |seed| Box.box((core.traditional_tree_seed)(seed))

## One operation of the system: the forest after it, and what it answered.
traditional_tree_step_for_host : Box(Traditionals), TraditionalEvent -> TraditionalTreeStepped
traditional_tree_step_for_host = |boxed, event| {
	stepped = (core.traditional_tree_step)(Box.unbox(boxed), event)
	{ model: Box.box(stepped.model), answer: stepped.answer }
}

## The system as the platform holds it, and the system back unchanged.
traditional_tree_view_for_host : Box(Traditionals) -> TraditionalTreeViewed
traditional_tree_view_for_host = |boxed| {
	viewed = (core.traditional_tree_view)(Box.unbox(boxed))
	{ model: Box.box(viewed.model), view: viewed.view }
}

## A traditional layout question's answer, and the system back unchanged.
traditional_tree_layout_for_host : Box(Traditionals), GroupLayoutQuestion -> TraditionalTreeLaidOut
traditional_tree_layout_for_host = |boxed, question| {
	laid = (core.traditional_tree_layout)(Box.unbox(boxed), question)
	{ model: Box.box(laid.model), answer: laid.answer }
}

## The host cannot release a `Box(Traditionals)`: only Roc knows its layout.
traditional_tree_release_for_host : Box(Traditionals) -> {}
traditional_tree_release_for_host = |_| {}

## A master-stack layout system (`rift_core::layout::group_forest::MasterStack`), kept between
## operations: no layouts, no windows left out.
master_stack_init_for_host : MasterSettings -> Box(Masters)
master_stack_init_for_host = |settings| Box.box((core.master_stack_init)(settings))

## A master-stack layout system as the platform holds it (the host parses it first).
master_stack_seed_for_host : MasterStackSeed -> Box(Masters)
master_stack_seed_for_host = |seed| Box.box((core.master_stack_seed)(seed))

## One operation of the system: the forest after it, and what it answered.
master_stack_step_for_host : Box(Masters), MasterStackEvent -> MasterStackStepped
master_stack_step_for_host = |boxed, event| {
	stepped = (core.master_stack_step)(Box.unbox(boxed), event)
	{ model: Box.box(stepped.model), answer: stepped.answer }
}

## The system as the platform holds it, and the system back unchanged.
master_stack_view_for_host : Box(Masters) -> MasterStackViewed
master_stack_view_for_host = |boxed| {
	viewed = (core.master_stack_view)(Box.unbox(boxed))
	{ model: Box.box(viewed.model), view: viewed.view }
}

## A master-stack layout question's answer, and the system back unchanged.
master_stack_layout_for_host : Box(Masters), GroupLayoutQuestion -> MasterStackLaidOut
master_stack_layout_for_host = |boxed, question| {
	laid = (core.master_stack_layout)(Box.unbox(boxed), question)
	{ model: Box.box(laid.model), answer: laid.answer }
}

## The host cannot release a `Box(Masters)`: only Roc knows its layout.
master_stack_release_for_host : Box(Masters) -> {}
master_stack_release_for_host = |_| {}

## A floating layout system (`rift_core::layout::group_forest::FloatingSystem`), kept between
## operations: no layouts, no frames.
floating_tree_init_for_host : {} -> Box(Floatings)
floating_tree_init_for_host = |{}| Box.box((core.floating_tree_init)({}))

## A floating layout system as the platform holds it.
floating_tree_seed_for_host : FloatingSeed -> Box(Floatings)
floating_tree_seed_for_host = |seed| Box.box((core.floating_tree_seed)(seed))

## One operation or query of the system: the system after it, and what it answered.
floating_tree_step_for_host : Box(Floatings), FloatingEvent -> FloatingTreeStepped
floating_tree_step_for_host = |boxed, event| {
	stepped = (core.floating_tree_step)(Box.unbox(boxed), event)
	{ model: Box.box(stepped.model), answer: stepped.answer }
}

## The system as the platform holds it, and the system back unchanged.
floating_tree_view_for_host : Box(Floatings) -> FloatingTreeViewed
floating_tree_view_for_host = |boxed| {
	viewed = (core.floating_tree_view)(Box.unbox(boxed))
	{ model: Box.box(viewed.model), view: viewed.view }
}

## A floating layout question's answer, and the system back unchanged.
floating_tree_layout_for_host : Box(Floatings), FloatingLayoutQuestion -> FloatingTreeLaidOut
floating_tree_layout_for_host = |boxed, question| {
	laid = (core.floating_tree_layout)(Box.unbox(boxed), question)
	{ model: Box.box(laid.model), answer: laid.answer }
}

## The host cannot release a `Box(Floatings)`: only Roc knows its layout.
floating_tree_release_for_host : Box(Floatings) -> {}
floating_tree_release_for_host = |_| {}

## A scrolling layout's arrangements: no layouts, no windows left out.
columns_init_for_host : ColumnsSettings -> Box(Cols)
columns_init_for_host = |settings| Box.box((core.columns_init)(settings))

## A scrolling layout's arrangements as the platform holds them (the host has parsed them).
columns_seed_for_host : ColumnsSeed -> Box(Cols)
columns_seed_for_host = |seed| Box.box((core.columns_seed)(seed))

## One operation: the arrangements after it, and what it answered.
columns_step_for_host : Box(Cols), ColumnsEvent -> ColumnsStepped
columns_step_for_host = |boxed, event| {
	stepped = (core.columns_step)(Box.unbox(boxed), event)
	{ model: Box.box(stepped.model), answer: stepped.answer }
}

## The system as the platform holds it, and the arrangements back unchanged.
columns_view_for_host : Box(Cols) -> ColumnsViewed
columns_view_for_host = |boxed| {
	viewed = (core.columns_view)(Box.unbox(boxed))
	{ model: Box.box(viewed.model), view: viewed.view }
}

columns_release_for_host : Box(Cols) -> {}
columns_release_for_host = |_| {}
