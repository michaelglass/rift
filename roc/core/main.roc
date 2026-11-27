## Rift's core in Roc: the frame reconciler and the user's grip, as one pure step. Events arrive
## and effects leave as the platform's typed values (`Boundary.Event`, `Boundary.Effect`), which
## the types below mirror. It mirrors `rift_core::RustCore` decision for decision; the
## differential test holds it to that.
app [core] { pf: platform "../platform/main.roc" }

import pf.Boundary exposing [Event, Effect, UngripWhy, NoteKind, PlanEffect, Observation, Want, WorkspaceMode, PendingTarget, SpaceActivation, NativeFocus, WindowId, Rect, Desire, WriteEffect, Button, Reply, Armed, Present, Ownership, WorkspaceSpace, AppStanding, Initiator, MaybeWord, AxisQuestion, TranslateQuestion, FloatingLayout, BspShape, Frame, TraditionalShape, Strip, Placed, SettleQuestion, Settled, FloatingQuestion, ActivationQuestion, CandidateQuestion, DesktopQuestion, DisplayFollowQuestion, FollowAnswer, HidesQuestion, RemovalAnswer, RemovalQuestion, UntrackedAnswer, UntrackedQuestion, VerdictAnswer, WordAnswer, Admission, AdmitQuestion, CandidatesAnswer, CandidatesQuestion, DiscoveredAnswer, DiscoveredQuestion, DiscoveryAnswer, DiscoveryQuestion, AdmissionFacts, Change, InventoryQuestion, KeepsTransientQuestion, MisplacedQuestion, NativeTransientQuestion, NetQuestion, OutOfViewQuestion, PointerTargetQuestion, PresenceAnswer, PresenceQuestion, ReadAnswer, ReadQuestion, RetirementAnswer, RuleRefreshQuestion, RuleTargetsAnswer, RuleTargetsQuestion, SpaceAnswer, TitleChangeQuestion, VerdictAnswer, WindowAdmission, JoinedQuestion, JoinedView, PreviousQuestion, Reached, ResolveQuestion, StepQuestion, ReachedAnswer, EligibleQuestion, LifecycleEvent, PresentWindow, WindowOwnership, WorkspaceSpace, SpaceActivation, JoinedEvent, Outcome, WorkspaceWindows, Invalidation, Revalidation, ModesQuestion, ModeVerdict, MainWindowEvent, NativeFocus, MainApp, SizesEvent, SizeRecord, AppMinimum, WindowConstraints, Size, FloatsEvent, FloatFullscreen, FloatFullscreenEntry, FloatFrameEntry, EmitsQuestion, QuarantineQuestion, TargetQuestion, TargetAnswer, RaiseQuestion, RaiseAnswer, BspTreeSeed, BspInsertion, BspTreeEvent, BspTreeAnswer, BspLayoutSlot, BspNodeSlot, BspTreeNode, Aspect, BspFramesQuestion, StackSettings, StackTreeSeed, StackTreeEvent, StackTreeAnswer, StackLayoutSlot, StackTreeStack, StackFramesQuestion, GroupTreeNode, TraditionalSettings, TraditionalSeed, TraditionalEvent, MasterSettings, MasterStackSeed, MasterStackEvent, GroupLayoutQuestion, GroupLayoutAnswer, FloatingGeometry, FloatingSeed, FloatingEvent, FloatingAnswer, FloatingLayoutQuestion, FloatingLayoutAnswer, ColumnsSettings, ColumnsSeed, ColumnsEvent, ColumnsAnswer, ColumnsArrangement]
import Geometry
import Bsp
import Traditional
import Scrolling
import Floating
import Topology
import Lifecycle
import Focus
import Workspaces
import Modes
import MainWindow
import Sizes
import Floats
import BspTree
import Slots
import StackTree
import GroupForest
import Columns

## The events, effects and their payloads are the platform's (`Boundary.Event`,
## `Boundary.Effect`): taking the platform's own types, not ones of the same shape, is what keeps
## `roc build` from hanging on the step.
WriteFx : WriteEffect

## The request of the latest frame Rift sent a window, if any.
Last : [Sent(U32), NeverSent]

## What the app had acknowledged when a write was sent.
Before : [AckedBefore(Rect), UnknownBefore]

## A desire waiting behind the write in flight: the latest, or none.
Waiting : [Waiting(Rect), NoneWaiting]

## How many deliveries of one desire failed in a row; a third gives it up, so there is no third.
Failures : [Once, Twice]

## Where a write in flight came from: only an ended animation's can be stopped by the platform.
## A retry is Rift writing a frame again after `Failures` deliveries of it failed.
Origin : [Written, AnimationEnded, Retry(Failures)]

## Where one window stands between what Rift asked of its app and what the app answered.
Sync : [
	Settled({ acked : Rect, last : Last }),
	Unknown(Last),
	InFlight({ target : Rect, deadline : U64, queued : Waiting, request : U32, before : Before, origin : Origin }),
	Animating({ target : Rect, request : U32 }),
	Refused({ wanted : Rect, got : Rect, request : U32 }),

	## The write under `request` asking for `wanted` never reached the app, `failures` times in
	## a row for this desire; the next desire for `wanted` writes it again.
	Failed({ wanted : Rect, failures : Failures, request : U32, before : Before }),

	## Rift moves the window outside the core (a gesture's camera) under `request`, which the
	## platform reported; `at` is where the app last reported it.
	Outside({ request : U32, at : Rect }),
]

## What the model has of one window: where it takes it to be, and where it stands.
Record : { seen : Rect, sync : Sync }

Grip : [Idle, Held({ window : WindowId, button : Button, phase : [Pressed, Dragging(List(WindowId))] })]


## The lifecycle kept between events: every window with AX state and its presence (valid, or
## invalidated), by window (`Lifecycle`'s, written out: the platform needs the name here).
Windows : Dict(U64, Present)

## Who places each window, kept between events, keyed by window, so no window has two owners
## (`Workspaces.Ownerships`, written out: the platform needs the name here).
Owners : Dict(U64, Ownership)

## The workspaces joined, kept between events: the catalog, what each Space shows and who places
## each window (`Workspaces.Joined`, written out: the platform needs the name here).
Joined : { catalog : List(WorkspaceSpace), activations : List(SpaceActivation), owners : Owners }

## Which window is the main one, kept between events: every app by pid, the globally frontmost
## app and the native focus (`MainWindow.Mains`, written out: the platform needs the name here).
Mains : { apps : Dict(U64, { standing : AppStanding, last_activation : Initiator, main_window : MaybeWord }), global_frontmost : MaybeWord, native_focus : NativeFocus }

## A BSP layout system's trees, kept between operations: every layout's root and every node in
## slot tables, the windows left out of layout, the settings (`BspTree.Trees`, written out: the
## platform needs the name here).
Trees : {
	layouts : Slots(U64),
	nodes : Slots(BspTreeNode),
	left_out : Set(U64),
	insertion : BspInsertion,
	aspect : Aspect,
}

## A stack layout system's stacks, kept between operations: every layout's stack in a slot
## table, the id counter, the settings, the windows left out of layout (`StackTree.Stacks`,
## written out: the platform needs the name here).
Stacks : {
	layouts : Slots(StackTreeStack),
	next_node : U32,
	settings : StackSettings,
	left_out : Set(U64),
}

## A group forest (`GroupForest.Forest`, written out: the platform needs the name here).
GroupForestHeld : {
	layouts : Slots(U64),
	nodes : Slots(GroupTreeNode),
	left_out : Set(U64),
	next_stamp : U64,
}

## A traditional layout system, kept between operations: its forest and its settings.
Traditionals : { forest : GroupForestHeld, settings : TraditionalSettings }

## A master-stack layout system, kept between operations: its forest and its settings.
Masters : { forest : GroupForestHeld, settings : MasterSettings }

## A floating layout system, kept between operations: its forest and each window's kept frame.
Floatings : { forest : GroupForestHeld, geometry : Dict(U64, FloatingGeometry) }

## A scrolling layout's arrangements, kept between operations.
Cols : { layouts : Slots(ColumnsArrangement), left_out : Set(U64), settings : ColumnsSettings }

Model : {
	windows : Dict(WindowId, Record),

	## Every write sent, with its deadline, in send order (= deadline order).
	deadlines : List({ deadline : U64, window : WindowId }),

	## Decided along the way, emitted after the step's decisions.
	notes : List(Effect),

	## Windows whose state changed this step, in order.
	touched : List(WindowId),
	grip : Grip,
	armed : Armed,
}

core = { init, step, bsp, traditional, strip, settle, floating, axis, translate, floating_layout, removal_focus, focus_candidate, hides_focus, activation_follow, display_to_follow, untracked_focus, focuses_desktop, discovered_space, discovery_spaces, admit_space_snapshot, app_rule_candidates, rule_targets, rule_refreshes_layout, pointer_display_target, is_native_transient, keeps_transient, misplaced, out_of_inventory_view, reapplies_on_title_change, window_admission, lifecycle_read, lifecycle_presence, lifecycle_net, inventory_retirement, workspace_step, workspace_resolve, workspace_previous, joined_steps, workspace_eligible, emits_window, quarantines, target_display, mouse_raise, lifecycle_init, lifecycle_step, lifecycle_invalidate, lifecycle_revalidate, lifecycle_view, workspaces_init, workspaces_step, workspaces_view, mode_verdict, main_window_init, main_window_step, main_window_view, sizes_init, sizes_step, sizes_view, floats_init, floats_step, floats_view, bsp_tree_init, bsp_tree_seed, bsp_tree_step, bsp_tree_view, bsp_tree_frames, stack_tree_init, stack_tree_seed, stack_tree_step, stack_tree_view, stack_tree_frames, traditional_tree_init, traditional_tree_seed, traditional_tree_step, traditional_tree_view, traditional_tree_layout, master_stack_init, master_stack_seed, master_stack_step, master_stack_view, master_stack_layout, floating_tree_init, floating_tree_seed, floating_tree_step, floating_tree_view, floating_tree_layout, columns_init, columns_seed, columns_step, columns_view, narrow }

# The layout questions, typed (`Boundary`). The core modules take and answer the platform's own
# types: a plain record or union never passes for a named one, nor a named one for a plain one.

## A BSP layout's frames.
bsp : BspShape -> List(Frame)
bsp = |shape| Bsp.frames(shape).map(|(window, frame)| { window, frame })

## A traditional layout's frames.
traditional : TraditionalShape -> List(Frame)
traditional = |shape| Traditional.frames(Traditional.from_platform(shape)).map(|(window, frame)| { window, frame })

## Where a scrolling strip places its columns, windows and camera.
strip : Strip -> Placed
strip = |columns| Scrolling.place(columns)

## Where the camera settles on a strip's placement.
settle : SettleQuestion -> Settled
settle = |question| Scrolling.settle(question.placed, question.view, question.projected, question.from, question.how)

## A floating layout's newcomers' first frames.
floating : FloatingQuestion -> List(Rect)
floating = |question| Floating.first_frames(question.screen, question.spread, question.windows)

## A floating layout's frames.
floating_layout : FloatingLayout -> List(Frame)
floating_layout = |layout| Floating.frames(layout.screen, layout.spread, layout.gaps, layout.windows).map(|(window, frame)| { window, frame })

## A placed strip's frames on screen.
translate : TranslateQuestion -> List(Frame)
translate = |q|
	Scrolling.translate(
		q.frames.map(|f| (f.window, f.frame)),
		{ screen: q.screen, tiling: q.tiling, offset: q.offset, parking: q.parking },
		q.fullscreen.map(|f| (f.window, f.mode)),
	).map(|(window, frame)| { window, frame })

## Rust's `value as f32` (the BSP tree's shares are `f32`s).
narrow : F64 -> F32
narrow = |value| Geometry.narrow(value)

## The lengths of the segments along one axis.
axis : AxisQuestion -> List(F64)
axis = |question| Geometry.solve(question.items, question.usable)

# The focus questions, typed (`Boundary`): each decision on its question's facts, or the
# first fact it read that the question does not carry. Each answer is its question's own
# platform type, so each entry builds it (a generic helper could not name it).

removal_focus : RemovalQuestion -> RemovalAnswer
removal_focus = |q|
	match Focus.removal_focus(q.removal, q.focused, q.removed_space, q.native_focus, q.successors, q.inactive) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}

focus_candidate : CandidateQuestion -> WordAnswer
focus_candidate = |q|
	match Focus.focus_candidate(q.preferred, q.last_focused, q.in_workspace, q.visible) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}

hides_focus : HidesQuestion -> VerdictAnswer
hides_focus = |q|
	match Focus.hides_focus(q.window, q.main, q.focused, q.in_inactive_workspace) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}

activation_follow : ActivationQuestion -> FollowAnswer
activation_follow = |q|
	match Focus.activation_follow(q) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}

display_to_follow : DisplayFollowQuestion -> WordAnswer
display_to_follow = |q|
	match Focus.display_to_follow(q.focused_space, q.previous, q.active, q.on_known_display) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}

untracked_focus : UntrackedQuestion -> UntrackedAnswer
untracked_focus = |q|
	match Focus.untracked_focus(q.under_cursor, q.tracked, q.known) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}

mouse_raise : RaiseQuestion -> RaiseAnswer
mouse_raise = |q|
	match Focus.mouse_raise(q) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}

focuses_desktop : DesktopQuestion -> VerdictAnswer
focuses_desktop = |q|
	match Focus.focuses_desktop(q.active, q.workspace_empty) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}

# The topology questions, typed (`Boundary`).

discovered_space : DiscoveredQuestion -> DiscoveredAnswer
discovered_space = |q|
	match Topology.discovered_space(q.standing, q.live, q.observable, q.recorded, q.resolved) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}

discovery_spaces : DiscoveryQuestion -> DiscoveryAnswer
discovery_spaces = |q|
	match Topology.discovery_spaces(q.standing, q.native, q.assigned, q.by_frame) {
		Ok((authoritative, discovery)) => Decided({ authoritative, discovery })
		Err(Unasked(fact)) => Unasked(fact)
	}

quarantines : QuarantineQuestion -> VerdictAnswer
quarantines = |q|
	match Topology.quarantines(q.topology, q.class) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}

target_display : TargetQuestion -> TargetAnswer
target_display = |q|
	match Topology.target_display(q.source_center, q.current_center, q.screens, q.active) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}

admit_space_snapshot : AdmitQuestion -> Admission
admit_space_snapshot = |q| {
	open =
		match q.mission_control {
			Open => True
			Closed => False
		}
	Topology.admit(open, q.current, q.incoming)
}

app_rule_candidates : CandidatesQuestion -> CandidatesAnswer
app_rule_candidates = |q|
	match Topology.candidates(q.window_spaces, q.activated, q.tracked, q.reconcilable) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}

# The lifecycle questions, typed (`Boundary`).

rule_targets : RuleTargetsQuestion -> RuleTargetsAnswer
rule_targets = |q|
	match Lifecycle.rule_targets(q.windows, q.run, q.reconcilable, q.spaces, q.active) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}

rule_refreshes_layout : RuleRefreshQuestion -> Bool
rule_refreshes_layout = |q| Lifecycle.rule_refreshes_layout(q.run, q.outcome)

pointer_display_target : PointerTargetQuestion -> SpaceAnswer
pointer_display_target = |q| {
	global =
		match q.mode {
			Global => True
			PerDisplay => False
		}
	match Lifecycle.pointer_display_target(global, q.opened_on, q.pointer, q.active, q.window, q.rule_names_workspace) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}
}

is_native_transient : NativeTransientQuestion -> VerdictAnswer
is_native_transient = |q|
	match Lifecycle.is_native_transient(q.entry, q.owner, q.exempt) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}

keeps_transient : KeepsTransientQuestion -> Bool
keeps_transient = |q| Lifecycle.keeps_transient(q.departure, q.id, q.origin, q.owner)

misplaced : MisplacedQuestion -> Bool
misplaced = |q| Lifecycle.misplaced(q.assigned, q.native)

out_of_inventory_view : OutOfViewQuestion -> VerdictAnswer
out_of_inventory_view = |q|
	match Lifecycle.out_of_inventory_view(q.native, q.assigned, q.active, q.parked) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}

reapplies_on_title_change : TitleChangeQuestion -> VerdictAnswer
reapplies_on_title_change = |q| {
	configured =
		match q.configured {
			Reapply => True
			Ignore => False
		}
	match Lifecycle.reapplies_on_title_change(configured, q.space, q.active, q.reconcilable) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}
}

window_admission : AdmissionFacts -> WindowAdmission
window_admission = |q| Lifecycle.admits(q)

lifecycle_read : ReadQuestion -> ReadAnswer
lifecycle_read = |q| {
	(after, change) = Lifecycle.read(q.before, q.reading)
	{ after, change }
}

## The lifecycle kept between events: every window with AX state, empty at first.
lifecycle_init : {} -> Windows
lifecycle_init = |{}| Lifecycle.windows_empty({})

## A reading of one window's facts.
lifecycle_step : Windows, LifecycleEvent -> { model : Windows, change : Change }
lifecycle_step = |windows, event| {
	(model, change) = Lifecycle.windows_read(windows, event.window, event.reading)
	{ model, change }
}

## A window's AX element invalidated.
lifecycle_invalidate : Windows, U64 -> { model : Windows, invalidation : Invalidation }
lifecycle_invalidate = |windows, window| {
	(model, invalidation) = Lifecycle.windows_invalidate(windows, window)
	{ model, invalidation }
}

## A positive AX observation of a window.
lifecycle_revalidate : Windows, U64 -> { model : Windows, revalidation : Revalidation }
lifecycle_revalidate = |windows, window| {
	(model, revalidation) = Lifecycle.windows_revalidate(windows, window)
	{ model, revalidation }
}

## Every window with AX state.
lifecycle_view : Windows -> { model : Windows, windows : List(PresentWindow) }
lifecycle_view = |windows| { model: windows, windows: Lifecycle.windows_view(windows) }

## The workspaces joined, kept between events: none, at first.
workspaces_init : {} -> Joined
workspaces_init = |{}| { catalog: [], activations: [], owners: Dict.empty() }

## A change to the joined workspaces, and what it met: a refused event changes nothing.
workspaces_step : Joined, JoinedEvent -> { model : Joined, outcome : Outcome }
workspaces_step = |joined, event| {
	outcome = Workspaces.admit(joined, event)
	match outcome {
		Accepted => { model: Workspaces.apply(joined, event), outcome }
		Exists(_) | Unknown(_) | OwnSuccessor(_) | Shown(_) => { model: joined, outcome }
	}
}

## The catalog, every Space's activation, every owned window and each catalogued workspace's
## assigned windows, in catalog order.
workspaces_view : Joined -> { model : Joined, catalog : List(WorkspaceSpace), activations : List(SpaceActivation), owners : List(WindowOwnership), windows : List(WorkspaceWindows) }
workspaces_view = |joined| {
	owners = Workspaces.in_order(joined.owners)
	assigned_to = |workspace|
		owners
			.keep_if(
				|entry|
					match entry.ownership {
						Assigned(id) => id == workspace
						Claimed(_) => False
						Held => False
					},
			)
			.map(|entry| entry.window)
	{
		model: joined,
		catalog: joined.catalog,
		activations: joined.activations,
		owners,
		windows: joined.catalog.map(|entry| { workspace: entry.workspace, windows: assigned_to(entry.workspace) }),
	}
}

## Which window is the main one: no apps and no native focus, at first.
## Window size constraints (`Sizes.Sizes`, written out: the platform needs the name here).
SizesModel : { windows : Dict(U64, WindowConstraints), app_minimums : Dict(I32, Size), revision : U64 }

sizes_init : {} -> SizesModel
sizes_init = |{}| Sizes.empty({})

sizes_step : SizesModel, SizesEvent -> { model : SizesModel, changed : Bool }
sizes_step = |sizes, event| {
	(model, changed) = Sizes.step(sizes, event)
	{ model, changed }
}

sizes_view : SizesModel -> { model : SizesModel, windows : List(SizeRecord), app_minimums : List(AppMinimum), revision : U64 }
sizes_view = |sizes| { model: sizes, windows: Sizes.records(sizes), app_minimums: Sizes.minimums(sizes), revision: sizes.revision }

## The floating state (`Floats.Kept`, written out: the platform needs the name here).
FloatsModel : { floating : Set(U64), fullscreen : Dict(U64, FloatFullscreen), floating_focus : MaybeWord, focused : MaybeWord, frames : Dict(U64, Dict(U64, Rect)) }

floats_init : {} -> FloatsModel
floats_init = |{}| Floats.empty({})

floats_step : FloatsModel, FloatsEvent -> FloatsModel
floats_step = |kept, event| Floats.step(kept, event)

floats_view : FloatsModel -> { model : FloatsModel, floating : List(U64), fullscreen : List(FloatFullscreenEntry), floating_focus : MaybeWord, focused : MaybeWord, frames : List(FloatFrameEntry) }
floats_view = |kept| { model: kept, floating: Floats.floating_list(kept), fullscreen: Floats.fullscreens(kept), floating_focus: kept.floating_focus, focused: kept.focused, frames: Floats.frames_list(kept) }

main_window_init : {} -> Mains
main_window_init = |{}| MainWindow.empty({})

## An observed event, and the window to focus, if it makes one known.
main_window_step : Mains, MainWindowEvent -> { model : Mains, focus : MaybeWord }
main_window_step = |mains, event| {
	(model, focus) = MainWindow.step(mains, event)
	{ model, focus }
}

## The main window, the native focus, every app in pid order and the globally frontmost app.
main_window_view : Mains -> { model : Mains, main_window : MaybeWord, native_focus : NativeFocus, apps : List(MainApp), global_frontmost : MaybeWord }
main_window_view = |mains| { model: mains, main_window: MainWindow.main_window(mains), native_focus: mains.native_focus, apps: MainWindow.apps_in_order(mains), global_frontmost: mains.global_frontmost }

## A BSP layout system's trees: no layouts, no windows left out, no aspect ratio.
bsp_tree_init : BspInsertion -> Trees
bsp_tree_init = |insertion| BspTree.empty(insertion)

## A BSP layout system as the platform holds it.
bsp_tree_seed : BspTreeSeed -> Trees
bsp_tree_seed = |seed| BspTree.seeded(seed)

## One operation of the system, and what it answers.
bsp_tree_step : Trees, BspTreeEvent -> { model : Trees, answer : BspTreeAnswer }
bsp_tree_step = |trees, event| {
	(model, answer) = BspTree.step(trees, event)
	{ model, answer }
}

## A layout's frames: the trees' shape for it (`Tree::shape`) through the BSP geometry; none
## when the layout is unknown or no window takes part.
bsp_tree_frames : Trees, BspFramesQuestion -> { model : Trees, frames : List(Frame) }
bsp_tree_frames = |trees, question| {
	frames : List(Frame)
	frames =
		match BspTree.shape(trees, question) {
			Shaped(shape) => Bsp.frames(shape).map(|(window, frame)| { window, frame })
			Unshaped => []
		}
	{ model: trees, frames }
}

## Each table's slots and free list, the windows left out in order, and the settings.
bsp_tree_view : Trees -> { model : Trees, layouts : List(BspLayoutSlot), layout_free : List(U32), nodes : List(BspNodeSlot), node_free : List(U32), left_out : List(U64), insertion : BspInsertion, aspect : Aspect }
bsp_tree_view = |trees| {
	viewed = BspTree.view(trees)
	{ model: trees, layouts: viewed.layouts, layout_free: viewed.layout_free, nodes: viewed.nodes, node_free: viewed.node_free, left_out: viewed.left_out, insertion: viewed.insertion, aspect: viewed.aspect }
}

## A stack layout system's stacks: no layouts, no windows left out.
stack_tree_init : StackSettings -> Stacks
stack_tree_init = |settings| StackTree.empty(settings)

## A stack layout system as the platform holds it.
stack_tree_seed : StackTreeSeed -> Stacks
stack_tree_seed = |seed| StackTree.seeded(seed)

## One operation of the system, and what it answers.
stack_tree_step : Stacks, StackTreeEvent -> { model : Stacks, answer : StackTreeAnswer }
stack_tree_step = |stacks, event| {
	(model, answer) = StackTree.step(stacks, event)
	{ model, answer }
}

## The table's slots and free list, the counter, the settings, the windows left out in order.
stack_tree_view : Stacks -> { model : Stacks, layouts : List(StackLayoutSlot), layout_free : List(U32), next_node : U32, settings : StackSettings, left_out : List(U64) }
stack_tree_view = |stacks| {
	viewed = StackTree.view(stacks)
	{ model: stacks, layouts: viewed.layouts, layout_free: viewed.layout_free, next_node: viewed.next_node, settings: viewed.settings, left_out: viewed.left_out }
}

## A layout's frames: the stacks' shape for it (`Stacks::shape`) through the traditional
## geometry; none when no window takes part.
stack_tree_frames : Stacks, StackFramesQuestion -> { model : Stacks, frames : List(Frame) }
stack_tree_frames = |stacks, question| {
	frames : List(Frame)
	frames =
		match StackTree.shape(stacks, question) {
			Shaped(shape) => Traditional.frames(shape).map(|(window, frame)| { window, frame })
			Unshaped => []
		}
	{ model: stacks, frames }
}

## A traditional layout system: no layouts, no windows left out.
traditional_tree_init : TraditionalSettings -> Traditionals
traditional_tree_init = |settings| GroupForest.traditional_empty(settings)

## A traditional layout system as the platform holds it.
traditional_tree_seed : TraditionalSeed -> Traditionals
traditional_tree_seed = |seed| GroupForest.traditional_seeded(seed)

## One operation of the traditional system, and what it answers.
traditional_tree_step : Traditionals, TraditionalEvent -> { model : Traditionals, answer : BspTreeAnswer }
traditional_tree_step = |system, event| {
	(model, answer) = GroupForest.traditional_step(system, event)
	{ model, answer }
}

## The traditional system as the platform holds it.
traditional_tree_view : Traditionals -> { model : Traditionals, view : TraditionalSeed }
traditional_tree_view = |system| { model: system, view: GroupForest.traditional_view(system) }

## A traditional layout question's answer (the system unchanged).
traditional_tree_layout : Traditionals, GroupLayoutQuestion -> { model : Traditionals, answer : GroupLayoutAnswer }
traditional_tree_layout = |system, question| { model: system, answer: forest_layout(system.forest, question) }

## A master-stack layout system: no layouts, no windows left out.
master_stack_init : MasterSettings -> Masters
master_stack_init = |settings| GroupForest.master_empty(settings)

## A master-stack layout system as the platform holds it.
master_stack_seed : MasterStackSeed -> Masters
master_stack_seed = |seed| GroupForest.master_seeded(seed)

## One operation of the master-stack system, and what it answers.
master_stack_step : Masters, MasterStackEvent -> { model : Masters, answer : BspTreeAnswer }
master_stack_step = |system, event| {
	(model, answer) = GroupForest.master_step(system, event)
	{ model, answer }
}

## The master-stack system as the platform holds it.
master_stack_view : Masters -> { model : Masters, view : MasterStackSeed }
master_stack_view = |system| { model: system, view: GroupForest.master_view(system) }

## A master-stack layout question's answer (the system unchanged).
master_stack_layout : Masters, GroupLayoutQuestion -> { model : Masters, answer : GroupLayoutAnswer }
master_stack_layout = |system, question| { model: system, answer: forest_layout(system.forest, question) }

## A group forest layout question's answer: a layout's frames are its shape (`Forest::shape`)
## through the traditional geometry (none when nothing takes part); a pass with its group bars,
## the container view and the drawing are the forest's own (`Forest::project`, `container_view`,
## `rows`).
forest_layout : GroupForestHeld, GroupLayoutQuestion -> GroupLayoutAnswer
forest_layout = |forest, question|
	match question {
		Frames(asked) =>
			Frames(
				match GroupForest.shape(forest, asked) {
					Shaped(shape) => Traditional.frames(shape).map(|(window, frame)| { window, frame })
					Unshaped => []
				},
			)

		Project({ pass, bars }) => Projected(GroupForest.project(forest, pass, bars))
		Container(layout) => Container(GroupForest.container_view(forest, layout))
		Draw(layout) => Drawn(GroupForest.rows(forest, layout))
	}

## A floating layout system: no layouts, no frames.
floating_tree_init : {} -> Floatings
floating_tree_init = |{}| GroupForest.floating_empty({})

## A floating layout system as the platform holds it.
floating_tree_seed : FloatingSeed -> Floatings
floating_tree_seed = |seed| GroupForest.floating_seeded(seed)

## One operation or query of the floating system, and what it answers.
floating_tree_step : Floatings, FloatingEvent -> { model : Floatings, answer : FloatingAnswer }
floating_tree_step = |system, event| {
	(model, answer) = GroupForest.floating_step(system, event)
	{ model, answer }
}

## The floating system as the platform holds it.
floating_tree_view : Floatings -> { model : Floatings, view : FloatingSeed }
floating_tree_view = |system| { model: system, view: GroupForest.floating_view(system) }

## A floating layout question's answer (the system unchanged).
floating_tree_layout : Floatings, FloatingLayoutQuestion -> { model : Floatings, answer : FloatingLayoutAnswer }
floating_tree_layout = |system, question| { model: system, answer: GroupForest.floating_layout(system, question) }

## A scrolling layout's arrangements: no layouts, no windows left out.
columns_init : ColumnsSettings -> Cols
columns_init = |settings| Columns.empty(settings)

## A scrolling layout's arrangements as the platform holds them.
columns_seed : ColumnsSeed -> Cols
columns_seed = |seed| Columns.seeded(seed)

## One operation of the arrangements, and what it answers.
columns_step : Cols, ColumnsEvent -> { model : Cols, answer : ColumnsAnswer }
columns_step = |cols, event| {
	(model, answer) = Columns.step(cols, event)
	{ model, answer }
}

## The arrangements as the platform holds them.
columns_view : Cols -> { model : Cols, view : ColumnsSeed }
columns_view = |cols| {
	viewed = Columns.view(cols)
	{ model: cols, view: { layouts: viewed.layouts, layout_free: viewed.layout_free, left_out: viewed.left_out, settings: viewed.settings } }
}

## The mode table's verdict on an initiative.
mode_verdict : ModesQuestion -> ModeVerdict
mode_verdict = |q| Modes.verdict({ pointer: q.pointer, menu: q.menu, mission_control: q.mission_control, modal: q.modal, viewport: q.viewport, focus_claim: q.focus_claim, user_spaces: q.user_spaces, wake: q.wake, workspace_switch: q.workspace_switch, topology: q.topology }, q.initiative)

lifecycle_presence : PresenceQuestion -> PresenceAnswer
lifecycle_presence = |q| Lifecycle.presence_step(q.before, q.event)

lifecycle_net : NetQuestion -> Change
lifecycle_net = |q| Lifecycle.net(q.first, q.then)

inventory_retirement : InventoryQuestion -> RetirementAnswer
inventory_retirement = |q|
	match Lifecycle.retirement(q.facts, q.evidence) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}

# The workspace questions, typed (`Boundary`).

## Whether workspaces map onto displays one set across them (global) or per display.
global : WorkspaceMode -> Bool
global = |mode|
	match mode {
		Global => True
		PerDisplay => False
	}

workspace_step : StepQuestion -> WordAnswer
workspace_step = |q| {
	backwards =
		match q.way {
			Previous => True
			Next => False
		}
	skip =
		match q.skip_empty {
			Skip => True
			Keep => False
		}
	stop =
		match q.wrapping {
			Stop => True
			Wrap => False
		}
	match Workspaces.step(global(q.mode), q.catalog, q.context, q.current, backwards, skip, stop, q.has_windows) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}
}

workspace_resolve : ResolveQuestion -> Reached
workspace_resolve = |q| Workspaces.resolve(global(q.mode), q.scope, q.context, q.selector)

workspace_previous : PreviousQuestion -> ReachedAnswer
workspace_previous = |q|
	match Workspaces.previous(global(q.mode), q.context, q.last, q.on) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}

workspace_eligible : EligibleQuestion -> Bool
workspace_eligible = |q| Workspaces.eligible(global(q.mode), q.context, q.on)

emits_window : EmitsQuestion -> VerdictAnswer
emits_window = |q|
	match Workspaces.emits_window(q.assigned, q.active) {
		Ok(answer) => Decided(answer)
		Err(Unasked(fact)) => Unasked(fact)
	}

joined_steps : JoinedQuestion -> JoinedView
joined_steps = |q| Workspaces.joined_after(q.events)

init : {} -> Model
init = |{}| {
	windows: Dict.empty(),
	deadlines: [],
	notes: [],
	touched: [],
	grip: Idle,
	armed: Disarmed,
}

# ---------------------------------------------------------------------------------------------
# Geometry.

write_deadline_ns : U64
write_deadline_ns = 300_000_000

ack_tolerance : F64
ack_tolerance = 1.5

within : F64, F64, F64 -> Bool
within = |a, tolerance, b| (a - b).abs() < tolerance

is_within : Rect, F64, Rect -> Bool
is_within = |a, tolerance, b|
	within(a.x, tolerance, b.x) and within(a.y, tolerance, b.y) and within(a.w, tolerance, b.w) and within(a.h, tolerance, b.h)

same_as : Rect, Rect -> Bool
same_as = |a, b| is_within(a, 0.1, b)

same_size : Rect, Rect -> Bool
same_size = |a, b| within(a.w, 0.1, b.w) and within(a.h, 0.1, b.h)

answers : Rect, Rect -> Bool
answers = |target, frame| is_within(frame, ack_tolerance, target)

## Exactly the same number (Rust's `==` on f64).
equal : F64, F64 -> Bool
equal = |a, b| !(a < b) and !(a > b)

rect_eq : Rect, Rect -> Bool
rect_eq = |a, b| equal(a.x, b.x) and equal(a.y, b.y) and equal(a.w, b.w) and equal(a.h, b.h)

# ---------------------------------------------------------------------------------------------
# The grip.

owner : Grip, WindowId -> [Rift, User]
owner = |grip, window|
	match grip {
		Held({ phase: Dragging(_), .. }) => User
		Held({ window: held, phase: Pressed, .. }) => if (held == window) User else Rift
		Idle => Rift
	}

holder : Grip, WindowId -> [User, Nobody]
holder = |grip, window|
	match grip {
		Held({ window: held, .. }) => if (held == window) User else Nobody
		Idle => Nobody
	}

# ---------------------------------------------------------------------------------------------
# The reconciler's state.

last_of : Sync -> Last
last_of = |sync|
	match sync {
		Settled({ last, .. }) => last
		Unknown(last) => last
		InFlight({ request, .. }) => Sent(request)
		Animating({ request, .. }) => Sent(request)
		Refused({ request, .. }) => Sent(request)
		Failed({ request, .. }) => Sent(request)
		Outside({ request, .. }) => Sent(request)
	}

pending_of : Sync -> PendingTarget
pending_of = |sync|
	match sync {
		InFlight({ target, .. }) => Target(target)
		Animating({ target, .. }) => Target(target)
		Refused({ wanted, .. }) => Target(wanted)
		Failed({ wanted, .. }) => Target(wanted)
		Settled(_) => NoTarget
		Unknown(_) => NoTarget
		Outside(_) => NoTarget
	}

before_of : Sync -> Before
before_of = |sync|
	match sync {
		Settled({ acked, .. }) => AckedBefore(acked)
		Animating({ target, .. }) => AckedBefore(target)
		Refused({ got, .. }) => AckedBefore(got)
		Unknown(_) => UnknownBefore
		InFlight({ before, .. }) => before
		Failed({ before, .. }) => before
		Outside({ at, .. }) => AckedBefore(at)
	}

## A window's sync state changes; only a window the model knows has one.
set : Model, WindowId, Sync -> Model
set = |model, window, sync|
	match model.windows.get(window) {
		Ok(record) => { ..model, windows: model.windows.insert(window, { ..record, sync }), touched: model.touched.append(window) }
		Err(_) => model
	}

## The model takes `window` to be at `frame`; a window it did not know appears there.
see : Model, WindowId, Rect -> Model
see = |model, window, frame|
	match model.windows.get(window) {
		Ok(record) => { ..model, windows: model.windows.insert(window, { ..record, seen: frame }) }
		Err(_) => { ..model, windows: model.windows.insert(window, { seen: frame, sync: Settled({ acked: frame, last: NeverSent }) }) }
	}

remove : Model, WindowId -> Model
remove = |model, window|
	if model.windows.contains(window) {
		{ ..model, windows: model.windows.remove(window), touched: model.touched.append(window) }
	} else {
		model
	}

## macOS replaced `from` by `to`: unless `to` is already known, `from`'s record moves to it. A
## settled (or unknown) window keeps that; anything pending went to a dead identity, so where
## the app has `to` is unknown. No desire moves across.
rekey : Model, WindowId, WindowId -> Model
rekey = |model, from, to| {
	moved =
		match model.windows.get(from) {
			Ok({ seen, sync }) =>
				if model.windows.contains(to) {
					model
				} else {
					kept =
						match sync {
							Settled(_) => sync
							Unknown(_) => sync
							InFlight(_) => Unknown(last_of(sync))
							Animating(_) => Unknown(last_of(sync))
							Refused(_) => Unknown(last_of(sync))
							Failed(_) => Unknown(last_of(sync))
							Outside(_) => Unknown(last_of(sync))
						}
					{ ..model, windows: model.windows.insert(to, { seen, sync: kept }), touched: model.touched.append(to) }
				}

			Err(_) => model
		}
	remove(moved, from)
}

note : Model, WindowId, NoteKind -> Model
note = |model, window, what| { ..model, notes: model.notes.append(Note({ window, note: what })) }

distrust : Model, WindowId -> Model
distrust = |model, window|
	match model.windows.get(window) {
		Err(_) => model
		Ok(record) =>
			match record.sync {
				InFlight(flight) => set(model, window, InFlight({ ..flight, before: UnknownBefore }))
				Settled(_) | Refused(_) | Failed(_) | Unknown(_) | Animating(_) | Outside(_) => set(model, window, Unknown(last_of(record.sync)))
			}
	}

send : Model, U64, U32, WindowId, Rect, Before, Origin -> (Model, WriteFx)
send = |model, now, request, window, frame, before, origin| {
	deadline = now + write_deadline_ns
	queued_up = { ..model, deadlines: model.deadlines.append({ deadline, window }) }
	sent = set(queued_up, window, InFlight({ target: frame, deadline, queued: NoneWaiting, request, before, origin }))
	(sent, { window, frame, request })
}

time_out : Model, WindowId, Rect, U32, Before -> (Model, Sync)
time_out = |model, window, target, request, before| {
	late =
		match before {
			AckedBefore(got) => Refused({ wanted: target, got, request })
			UnknownBefore => Unknown(Sent(request))
		}
	(set(note(model, window, TimedOut), window, late), late)
}

Verdict : [Send(WriteFx), QueuedUp, Nothing]

## What Rift decides about one desired frame for a window.
want : Model, U64, U32, WindowId, Rect, [Rift, User] -> (Model, Verdict)
want = |model0, now, request, window, desired, whose|
	match model0.windows.get(window) {
		Err(_) => (model0, Nothing)
		Ok({ seen: known, sync: sync0 }) => {
			# A write nobody answered in time no longer holds newer desires back.
			(model, sync) =
				match sync0 {
					InFlight(flight) =>
						if now >= flight.deadline {
							time_out(model0, window, flight.target, flight.request, flight.before)
						} else {
							(model0, sync0)
						}

					Settled(_) | Refused(_) | Failed(_) | Unknown(_) | Animating(_) | Outside(_) => (model0, sync0)
				}
			match whose {
				# Discarded, not queued: the release re-derives the window's frame.
				User => (note(model, window, Discarded), Nothing)
				Rift => want_from(model, now, request, window, desired, known, sync)
			}
		}
	}

want_from : Model, U64, U32, WindowId, Rect, Rect, Sync -> (Model, Verdict)
want_from = |model, now, request, window, desired, known, sync| {
	write = |{}| {
		(sent, fx) = send(model, now, request, window, desired, before_of(sync), Written)
		(sent, Send(fx))
	}
	match sync {
		InFlight(flight) =>
			if same_as(desired, flight.target) {
				match flight.queued {
					Waiting(_) => (set(model, window, InFlight({ ..flight, queued: NoneWaiting })), Nothing)
					NoneWaiting => (model, Nothing)
				}
			} else {
				(set(model, window, InFlight({ ..flight, queued: Waiting(desired) })), QueuedUp)
			}

		# Asking again for what was refused would be a retry loop; a window seen elsewhere since
		# is no longer the one that refused. A refused window is where the model has it.
		Refused({ wanted, got, .. }) =>
			if same_as(desired, wanted) and same_as(known, got) {
				(model, Nothing)
			} else if same_as(desired, known) {
				(model, Nothing)
			} else {
				write({})
			}

		# A delivery of it failed: written again, the failures counted.
		Failed({ wanted, failures, before, .. }) =>
			if same_as(desired, wanted) {
				(sent, fx) = send(model, now, request, window, desired, before, Retry(failures))
				(sent, Send(fx))
			} else {
				write({})
			}

		# As near as an answer to this very desire would be (a terminal rounds to whole cells).
		Settled({ acked, .. }) => if (answers(desired, acked)) (model, Nothing) else write({})
		Animating({ target, .. }) => if (same_as(desired, target)) (model, Nothing) else write({})
		Outside({ at, .. }) => if (answers(desired, at)) (model, Nothing) else write({})
		Unknown(_) => write({})
	}
}

follow : Model, U32, WindowId, Rect -> (Model, WriteFx)
follow = |model, request, window, frame|
	(set(model, window, Settled({ acked: frame, last: Sent(request) })), { window, frame, request })

## The desire `next` waited behind a write just answered or given up on, the window left
## `after`: it goes out now, unless the user's hand is on the window (then it is discarded).
release : Model, U64, U32, WindowId, Rect, [Rift, User], Sync -> (Model, List(WriteFx))
release = |model, now, request, window, next, whose, after|
	match whose {
		User => (note(model, window, Discarded), [])
		Rift => {
			(sent, fx) = send(model, now, request, window, next, before_of(after), Written)
			(sent, [fx])
		}
	}

Read : Observation

## A report of where a window is (see `Reconciler::observe`).
observe : Model, Read, [Rift, User], [User, Nobody] -> (Model, Reply, List(WriteFx))
observe = |model, read, whose, hand| {
	window = read.window
	frame = read.frame
	match model.windows.get(window) {
		Err(_) => (model, Foreign, [])
		Ok({ seen: known, sync }) => {
			placed = |{}| {
				(after, reply) = place(model, window, sync, frame, known, hand)
				(after, reply, [])
			}
			match read.as_of {
				Unstamped => placed({})
				AsOf(stamp) =>
					match last_of(sync) {
						Sent(latest) =>
							if stamp < latest {
								(model, NoNews, [])
							} else if stamp == latest {
								answer(model, read, whose, latest, sync, placed)
							} else {
								# A stamp the core never heard of: placed by its geometry.
								placed({})
							}

						NeverSent => placed({})
					}
			}
		}
	}
}

## The read the latest write asked for.
answer : Model, Read, [Rift, User], U32, Sync, ({} -> (Model, Reply, List(WriteFx))) -> (Model, Reply, List(WriteFx))
answer = |model, read, whose, latest, sync, placed| {
	window = read.window
	frame = read.frame
	match sync {
		InFlight({ target, queued, .. }) => {
			(after, reply) =
				if answers(target, frame) {
					(Settled({ acked: frame, last: Sent(latest) }), Acked)
				} else {
					(Refused({ wanted: target, got: frame, request: latest }), Refused)
				}
			answered = set(model, window, after)
			(released, writes) =
				match queued {
					Waiting(next) => release(answered, read.now, read.request, window, next, whose, after)
					NoneWaiting => (answered, [])
				}
			(released, reply, writes)
		}

		Animating({ target, .. }) =>
			if answers(target, frame) {
				(set(model, window, Settled({ acked: frame, last: Sent(latest) })), Acked, [])
			} else {
				(model, InTransit, [])
			}

		# One of Rift's frames sent outside the core: where the app has the window, the frames
		# still coming.
		Outside({ request, .. }) => (set(model, window, Outside({ request, at: frame })), InTransit, [])
		# The answer to a write Rift stopped waiting for, or to one whose delivery failed: news
		# like any report.
		Settled(_) | Unknown(_) | Refused(_) | Failed(_) => placed({})
	}
}

## An unstamped report, placed by its geometry.
place : Model, WindowId, Sync, Rect, Rect, [User, Nobody] -> (Model, Reply)
place = |model, window, sync, frame, known, hand| {
	news = |{}| {
		settled = |reply| (set(model, window, Settled({ acked: frame, last: last_of(sync) })), reply)
		if same_as(frame, known) {
			match sync {
				# A refusal stands while the window stays where the model has it, and so do
				# Rift's frames sent outside the core.
				Refused(_) | Outside(_) => (model, NoNews)
				# A failed write landed after all, or the app reports the window where the model
				# has it: settled there.
				Settled(_) | Unknown(_) | Failed(_) | InFlight(_) | Animating(_) => settled(NoNews)
			}
		} else {
			settled(Foreign)
		}
	}
	match sync {
		InFlight({ target, .. }) =>
			if answers(target, frame) {
				(model, NoNews)
			} else {
				match hand {
					Nobody => (model, InTransit)
					User => news({})
				}
			}

		Animating({ target, request }) =>
			match hand {
				Nobody =>
					if answers(target, frame) {
						(set(model, window, Settled({ acked: frame, last: Sent(request) })), NoNews)
					} else {
						(model, InTransit)
					}

				User => news({})
			}

		Settled(_) | Refused(_) | Failed(_) | Unknown(_) | Outside(_) => news({})
	}
}

## The write a deadlines entry stands for, if it is still waiting.
waiting : Model, { deadline : U64, window : WindowId } -> [Waits({ target : Rect, queued : Waiting, request : U32, before : Before }), NotWaiting]
waiting = |model, entry|
	match model.windows.get(entry.window) {
		Ok(record) =>
			match record.sync {
				InFlight({ target, deadline, queued, request, before, .. }) =>
					if deadline == entry.deadline {
						Waits({ target, queued, request, before })
					} else {
						NotWaiting
					}

				Settled(_) | Refused(_) | Failed(_) | Unknown(_) | Animating(_) | Outside(_) => NotWaiting
			}

		Err(_) => NotWaiting
	}

expire : Model, U64, U32, Grip -> (Model, List(WriteFx))
expire = |model0, now, request, grip| {
	var $model = model0
	var $released = []
	var $going = True
	while $going {
		match $model.deadlines {
			[entry, .. as later] if entry.deadline <= now => {
				popped = { ..$model, deadlines: later }
				match waiting(popped, entry) {
					NotWaiting => {
						$model = popped
					}

					Waits({ target, queued, request: sent, before }) => {
						(timed_out, after) = time_out(popped, entry.window, target, sent, before)
						(released, writes) =
							match queued {
								Waiting(next) => release(timed_out, now, request, entry.window, next, owner(grip, entry.window), after)
								NoneWaiting => (timed_out, [])
							}
						$model = released
						$released = $released.concat(writes)
					}
				}
			}

			# The earliest deadline is still ahead, or there are none.
			[_, ..] | [] => {
				$going = False
			}
		}
	}
	($model, $released)
}

## When the earliest unanswered write stops being waited for; stale entries are dropped.
next_deadline : Model -> (Model, Armed)
next_deadline = |model0| {
	var $model = model0
	var $found = Disarmed
	var $going = True
	while $going {
		match $model.deadlines {
			[] => {
				$going = False
			}

			[entry, .. as later] =>
				match waiting($model, entry) {
					Waits(_) => {
						$found = Armed(entry.deadline)
						$going = False
					}

					NotWaiting => {
						$model = { ..$model, deadlines: later }
					}
				}
		}
	}
	($model, $found)
}

Planned : [AnimateIt, Decided(Verdict)]

animate_one : Model, U64, U32, WindowId, Rect, [Rift, User] -> (Model, Planned)
animate_one = |model, now, request, window, desired, whose|
	match model.windows.get(window) {
		Err(_) => (model, Decided(Nothing))
		Ok({ seen: known, sync }) => {
			# Where the window is taken to be: what is pending for it, else where the model sees it.
			already =
				match sync {
					# A write is with the app, or where the app has the window is unknown: written. A
					# delivery that failed is written again, as a write.
					InFlight(_) | Unknown(_) | Failed(_) => Written
					Animating({ target, .. }) => if (same_as(target, desired)) There else NotThere
					Refused({ wanted, .. }) => if (same_as(wanted, desired)) There else NotThere
					Settled(_) => if (answers(desired, known)) There else NotThere
					Outside({ at, .. }) => if (answers(desired, at)) There else NotThere
				}
			match already {
				Written => {
					(wanted, verdict) = want(model, now, request, window, desired, whose)
					(wanted, Decided(verdict))
				}

				There => (model, Decided(Nothing))
				NotThere =>
					match whose {
						User => (model, Decided(Nothing))
						Rift => (set(model, window, Animating({ target: desired, request })), AnimateIt)
					}
			}
		}
	}

window_order : WindowId, WindowId -> [Before, Same, After]
window_order = |a, b|
	if a.pid < b.pid {
		Before
	} else if a.pid > b.pid {
		After
	} else if a.idx < b.idx {
		Before
	} else if a.idx > b.idx {
		After
	} else {
		Same
	}

## The animation under `request` has ended: each window it was taking somewhere now waits for
## its app to report the target.
animation_ended : Model, U64, U32 -> Model
animation_ended = |model0, now, request| {
	ended =
		model0.windows.to_list().keep_if(
			|(_, record)|
				match record.sync {
					Animating({ request: sent, .. }) => sent == request
					Settled(_) | Refused(_) | Failed(_) | Unknown(_) | InFlight(_) | Outside(_) => False
				},
		).sort_with(|(a, _), (b, _)| window_order(a, b))
	deadline = now + write_deadline_ns
	var $model = model0
	for (window, record) in ended {
		match record.sync {
			Animating({ target, .. }) => {
				queued_up = { ..$model, deadlines: $model.deadlines.append({ deadline, window }) }
				$model = set(queued_up, window, InFlight({ target, deadline, queued: NoneWaiting, request, before: UnknownBefore, origin: AnimationEnded }))
			}

			Settled(_) | Refused(_) | Failed(_) | Unknown(_) | InFlight(_) | Outside(_) => {}
		}
	}
	$model
}

animation_stopped : Model, WindowId, U32 -> Model
animation_stopped = |model, window, request|
	match model.windows.get(window) {
		Err(_) => model
		Ok({ seen, sync }) =>
			match sync {
				Animating({ request: sent, .. }) =>
					if (sent == request) set(model, window, Settled({ acked: seen, last: Sent(request) })) else model

				InFlight({ request: sent, origin: AnimationEnded, .. }) =>
					if (sent == request) set(model, window, Unknown(Sent(request))) else model

				InFlight({ origin: Written, .. }) => model
				InFlight({ origin: Retry(_), .. }) => model

				Settled(_) | Refused(_) | Failed(_) | Unknown(_) | Outside(_) => model
			}
	}

## Nothing is outstanding for `window`: what the app acknowledged, if that is known.
forget_sync : Model, WindowId -> (Model, [AckedAt(Rect), NotAcked])
forget_sync = |model, window|
	match model.windows.get(window) {
		Err(_) => (model, NotAcked)
		Ok({ sync, .. }) =>
			match sync {
				Settled({ acked, .. }) => (model, AckedAt(acked))
				Refused({ got, .. }) => (set(model, window, Settled({ acked: got, last: last_of(sync) })), AckedAt(got))
				Failed({ before: AckedBefore(got), .. }) => (set(model, window, Settled({ acked: got, last: last_of(sync) })), AckedAt(got))
				# A failed write never reached the app, but where it had the window before is not
				# known either.
				Failed({ before: UnknownBefore, .. }) => (set(model, window, Unknown(last_of(sync))), NotAcked)
				InFlight(_) | Animating(_) | Unknown(_) | Outside(_) => (set(model, window, Unknown(last_of(sync))), NotAcked)
			}
	}

# ---------------------------------------------------------------------------------------------
# The core: decisions as effects.

see_fx : Model, WindowId, Rect -> (Model, Effect)
see_fx = |model, window, frame| (see(model, window, frame), Seen({ window, frame }))

written : Model, WriteFx -> (Model, List(Effect))
written = |model, write| {
	(seen, fx) = see_fx(model, write.window, write.frame)
	(seen, [Write(write), fx])
}

all_written : Model, List(WriteFx) -> (Model, List(Effect))
all_written = |model0, writes| {
	var $model = model0
	var $effects = []
	for write in writes {
		(next, fx) = written($model, write)
		$model = next
		$effects = $effects.concat(fx)
	}
	($model, $effects)
}

queued : Model, WindowId, Rect -> (Model, List(Effect))
queued = |model, window, desired| {
	(seen, fx) = see_fx(model, window, desired)
	(seen, [Queued(window), fx])
}

ungrip : Model, UngripWhy -> (Model, List(Effect))
ungrip = |model, why|
	match model.grip {
		Idle => (model, [])
		Held({ window, phase, .. }) =>
			match phase {
				Pressed => ({ ..model, grip: Idle }, [Ungrip({ window, why, changed: [] })])
				Dragging(changed) => {
					var $model = model
					for each in [window].concat(changed) {
						$model = distrust($model, each)
					}
					({ ..$model, grip: Idle }, [Ungrip({ window, why, changed })])
				}
			}
	}

ungrip_window : Model, WindowId -> (Model, List(Effect))
ungrip_window = |model, window|
	match model.grip {
		Held({ window: held, .. }) => if (held == window) ungrip(model, Gone) else (model, [])
		Idle => (model, [])
	}

rekey_grip : Model, WindowId, WindowId -> (Model, List(Effect))
rekey_grip = |model, from, to|
	match model.grip {
		Held(held) =>
			if held.window == from {
				({ ..model, grip: Held({ ..held, window: to }) }, [Regrip({ from, to })])
			} else {
				(model, [])
			}

		Idle => (model, [])
	}

## An outside owner took `window`: a write already sent is still waited for (only what was
## queued behind it is dropped); anything else is forgotten.
cede : Model, WindowId -> (Model, List(Effect))
cede = |model, window|
	match model.windows.get(window) {
		Ok({ sync, .. }) =>
			match sync {
				InFlight(flight) =>
					match flight.queued {
						Waiting(_) => (set(model, window, InFlight({ ..flight, queued: NoneWaiting })), [])
						NoneWaiting => (model, [])
					}

				Settled(_) | Unknown(_) | Animating(_) | Refused(_) | Failed(_) | Outside(_) => forget(model, window)
			}

		Err(_) => forget(model, window)
	}

forget : Model, WindowId -> (Model, List(Effect))
forget = |model, window| {
	(forgotten, acked) = forget_sync(model, window)
	match acked {
		AckedAt(frame) =>
			match forgotten.windows.get(window) {
				Ok(record) =>
					if rect_eq(record.seen, frame) {
						(forgotten, [])
					} else {
						(seen, fx) = see_fx(forgotten, window, frame)
						(seen, [fx])
					}

				Err(_) => (forgotten, [])
			}

		NotAcked => (forgotten, [])
	}
}

PlanFx : PlanEffect

## One write of an arrange pass into the plan for its request (plans in the order first written).
add_to_plan : List(PlanFx), U32, WriteFx, Bool -> List(PlanFx)
add_to_plan = |plans, request, write, moves_only| {
	add = |plan|
		if moves_only {
			{ ..plan, positions: plan.positions.append(write) }
		} else {
			{ ..plan, frames: plan.frames.append(write) }
		}
	if plans.any(|plan| plan.request == request) {
		plans.map(|plan| if (plan.request == request) add(plan) else plan)
	} else {
		plans.append(add({ request, positions: [], frames: [] }))
	}
}

arrange : Model, U64, Want, List(Desire) -> (Model, List(Effect))
arrange = |model0, now, how, desires| {
	grip = model0.grip
	var $model = model0
	var $effects = []
	var $plans = []
	for desire in desires {
		window = desire.window
		desired = desire.frame
		match $model.windows.get(window) {
			# A window the model does not know is not written.
			Err(_) => {}
			Ok({ seen: known, .. }) => {
				(wanted, verdict) = want($model, now, desire.request, window, desired, owner(grip, window))
				match verdict {
					Send(write) => {
						moves_only =
							match how {
								ArrangePositions => same_size(known, desired)
								Arrange => False
							}
						(seen, fx) = see_fx(wanted, window, desired)
						$model = seen
						$effects = $effects.append(fx)
						$plans = add_to_plan($plans, desire.request, write, moves_only)
					}

					QueuedUp => {
						(seen, fx) = queued(wanted, window, desired)
						$model = seen
						$effects = $effects.concat(fx)
					}

					Nothing => {
						$model = wanted
					}
				}
			}
		}
	}
	($model, $effects.concat($plans.map(|plan| Plan(plan))))
}

animate : Model, U64, List(Desire) -> (Model, List(Effect))
animate = |model0, now, desires| {
	grip = model0.grip
	var $model = model0
	var $effects = []
	for desire in desires {
		window = desire.window
		to = desire.frame
		match $model.windows.get(window) {
			Err(_) => {}
			Ok({ seen: from, .. }) => {
				(planned, plan) = animate_one($model, now, desire.request, window, to, owner(grip, window))
				match plan {
					AnimateIt => {
						(seen, fx) = see_fx(planned, window, to)
						$model = seen
						$effects = $effects.concat([Animate({ window, from, to, request: desire.request }), fx])
					}

					Decided(Send(write)) => {
						(seen, fx) = written(planned, write)
						$model = seen
						$effects = $effects.concat(fx)
					}

					Decided(QueuedUp) => {
						(seen, fx) = queued(planned, window, to)
						$model = seen
						$effects = $effects.concat(fx)
					}

					Decided(Nothing) => {
						$model = planned
					}
				}
			}
		}
	}
	($model, $effects)
}

## The platform could not deliver the write to `window` sent under `failed`: it never reached
## the app. The write in flight under that request becomes `Failed`, so the next desire for its
## frame writes it again; a third failure in a row gives the desire up as refused (noted). What
## waited behind it goes out under `request`. A failure of any other write changes nothing.
failed_write : Model, U64, U32, WindowId, U32, [Rift, User] -> (Model, [StaleFailure, WillRetry, GivenUp(Before)], List(WriteFx))
failed_write = |model, now, request, window, failed, whose|
	match model.windows.get(window) {
		Err(_) => (model, StaleFailure, [])
		Ok(record) =>
			match record.sync {
				InFlight(flight) =>
					if flight.request != failed {
						(model, StaleFailure, [])
					} else {
						next =
							match flight.origin {
								Written => RetryAs(Once)
								AnimationEnded => RetryAs(Once)
								Retry(Once) => RetryAs(Twice)
								Retry(Twice) => GiveUp
							}
						(failure, after, noted) =
							match next {
								RetryAs(failures) => (WillRetry, Failed({ wanted: flight.target, failures, request: flight.request, before: flight.before }), model)
								GiveUp => {
									given_up =
										match flight.before {
											AckedBefore(got) => Refused({ wanted: flight.target, got, request: flight.request })
											UnknownBefore => Unknown(Sent(flight.request))
										}
									(GivenUp(flight.before), given_up, note(model, window, GaveUp))
								}
							}
						answered = set(noted, window, after)
						(released, writes) =
							match flight.queued {
								Waiting(desire) => release(answered, now, request, window, desire, whose, after)
								NoneWaiting => (answered, [])
							}
						(released, failure, writes)
					}

				Settled(_) | Refused(_) | Failed(_) | Unknown(_) | Animating(_) | Outside(_) => (model, StaleFailure, [])
			}
	}

decide : Model, Event -> (Model, List(Effect))
decide = |model, event| {
	grip = model.grip
	match event {
		Press({ on, button }) => {
			(let_go, fx) = ungrip(model, LetGo)
			match on {
				OnWindow(window) => ({ ..let_go, grip: Held({ window, button, phase: Pressed }) }, fx.append(Grip(window)))
				OnNothing => (let_go, fx)
			}
		}

		Drag(window) =>
			match grip {
				Held({ window: pressed, button, phase }) =>
					match phase {
						Pressed =>
							if same_window(pressed, window) {
								({ ..model, grip: Held({ window, button, phase: Dragging([]) }) }, [DragBegan(window)])
							} else {
								(model, [])
							}

						Dragging(_) => (model, [])
					}

				Idle => (model, [])
			}

		Release(up) => {
			ends =
				match grip {
					Idle => False
					Held({ button, .. }) =>
						match up {
							Missed => True
							ButtonUp(released) => same_button(released, button)
						}
				}
			if (ends) ungrip(model, Released) else (model, [])
		}

		LetGo => ungrip(model, LetGo)
		Gone(window) => {
			(ungripped, fx) = ungrip_window(model, window)
			(remove(ungripped, window), fx)
		}

		Distrust(windows) => {
			var $model = model
			for window in windows {
				$model = distrust($model, window)
			}
			($model, [])
		}

		Ceded(window) => {
			(ungripped, fx) = ungrip_window(model, window)
			(ceded, more) = cede(ungripped, window)
			(ceded, fx.concat(more))
		}

		Seen({ window, frame }) => {
			(seen, fx) = see_fx(model, window, frame)
			(seen, [fx])
		}

		Unseen(window) => (remove(model, window), [])
		Rekeyed({ from, to }) => rekey_grip(rekey(model, from, to), from, to)
		Arrange({ now, want: how, desires }) => arrange(model, now, how, desires)
		Animate({ now, desires }) => animate(model, now, desires)
		AnimationEnded({ now, request }) => (animation_ended(model, now, request), [])
		AnimationStopped({ window, request, at }) => {
			(seen, fx) =
				match at {
					StoppedAt(frame) => {
						(after, one) = see_fx(model, window, frame)
						(after, [one])
					}

					NotPublished => (model, [])
				}
			(animation_stopped(seen, window, request), fx)
		}

		Follow({ request, window, frame }) => {
			(followed, write) = follow(model, request, window, frame)
			(followed, [Write(write)])
		}

		Observe(read) => {
			window = read.window
			(observed, reply, released) = observe(model, read, owner(grip, window), holder(grip, window))
			# News of another window during a drag (someone else moved it): its desires are the
			# user's until the drag ends, so it is re-derived then.
			noted =
				match reply {
					Foreign =>
						match observed.grip {
							Held({ window: held, button, phase: Dragging(changed) }) =>
								if held != window and !changed.contains(window) {
									{ ..observed, grip: Held({ window: held, button, phase: Dragging(changed.append(window)) }) }
								} else {
									observed
								}

							Held({ phase: Pressed, .. }) | Idle => observed
						}

					NoNews | InTransit | Acked | Refused => observed
				}
			# The app answered with a frame of its own: the model takes the window to be where the
			# app has it. The desire stays refused, so it is not asked for again.
			(seen_model, seen_fx) =
				match reply {
					Refused => {
						(at_got, fx) = see_fx(noted, window, read.frame)
						(at_got, [fx])
					}
					NoNews | InTransit | Acked | Foreign => (noted, [])
				}
			(sent, fx) = all_written(seen_model, released)
			(sent, [Replied({ window, reply })].concat(seen_fx).concat(fx))
		}

		Tick({ now, request }) => {
			(expired, released) = expire(model, now, request, grip)
			all_written(expired, released)
		}

		WriteFailed({ now, request, window, failed }) => {
			(after, failure, released) = failed_write(model, now, request, window, failed, owner(grip, window))
			# Given up: the frame Rift asked for never reached the app, so the model takes the
			# window to be where the app last acknowledged it, if that is known.
			(seen_model, seen_fx) =
				match failure {
					GivenUp(AckedBefore(got)) => {
						(at_got, fx) = see_fx(after, window, got)
						(at_got, [fx])
					}
					GivenUp(UnknownBefore) | WillRetry | StaleFailure => (after, [])
				}
			(sent, fx) = all_written(seen_model, released)
			(sent, seen_fx.concat(fx))
		}

		Forget(window) => forget(model, window)
		SentOutside({ window, request }) =>
			match model.windows.get(window) {
				Ok({ seen, .. }) => (set(model, window, Outside({ request, at: seen })), [])
				Err(_) => (model, [])
			}
	}
}

## Consecutive repeats collapse (as Rust's `Vec::dedup`).
dedup : List(WindowId) -> List(WindowId)
dedup = |windows| {
	(kept, _) =
		windows.fold(
			([], Nothing),
			|(out, last), window|
				match last {
					Last(previous) if previous == window => (out, last)
					Last(_) | Nothing => (out.append(window), Last(window))
				},
		)
	kept
}

## Decisions first, then the notes taken while deciding, then the pending frames that changed,
## then the timer if its deadline moved.
## Whether two windows are the same one. The platform's named types have no `==` (it is a
## method a nominal type would define), so a window from an event is compared by its fields.
same_window : WindowId, WindowId -> Bool
same_window = |a, b| a.pid == b.pid and a.idx == b.idx

## Whether two buttons are the same one (a button from an event is the platform's named type).
same_button : Button, Button -> Bool
same_button = |a, b|
	match (a, b) {
		(Left, Left) => True
		(Right, Right) => True
		(Left, Right) | (Right, Left) => False
	}

step : Model, Event -> { model : Model, effects : List(Effect) }
step = |model0, event| {
	(decided, decisions) = decide({ ..model0, notes: [], touched: [] }, event)
	pending : List(Effect)
	pending =
		dedup(decided.touched).map(
			|window| {
				target =
					match decided.windows.get(window) {
						Ok(record) => pending_of(record.sync)
						# Removed this step: nothing is pending for it.
						Err(_) => NoTarget
					}
				Pending({ window, target })
			},
		)
	(pruned, next) = next_deadline(decided)
	timer = if (next == pruned.armed) [] else [Timer(next)]
	effects = decisions.concat(pruned.notes).concat(pending).concat(timer)
	{ model: { ..pruned, armed: next, notes: [], touched: [] }, effects }
}
