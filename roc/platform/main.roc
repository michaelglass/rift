## Rift's core platform: the host (Rust) owns the event loop and every effect; the app is a
## pure step function and pure model-free questions. The layout questions cross as the typed
## values of `Boundary`; events, effects and the other questions still cross as words
## (`rift_core::plain::wire`), which the app parses into its own types. The model is opaque to
## the host: it holds a `Box(Model)` it can neither read nor free, and hands it back on every
## step.
platform ""
	requires {
		[Model : model] for core : {
			init : {} -> model,
			step : model, List(U64) -> { model : model, effects : List(U64) },
			tile : List(U64) -> List(U64),
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
		}
	}
	exposes [Boundary]
	packages {}
	provides {
		"roc_init": init_for_host,
		"roc_release_model": release_model_for_host,
		"roc_step": step_for_host,
		"roc_tile": tile_for_host,
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
	}
	targets: {
		arm64mac: {
			inputs: [app],
			output: Archive,
		},
	}

import Boundary exposing [Stepped, BspShape, Frame, TraditionalShape, Strip, Placed, SettleQuestion, Settled, FloatingQuestion, Rect, ActivationQuestion, CandidateQuestion, DesktopQuestion, DisplayFollowQuestion, FollowAnswer, HidesQuestion, RemovalAnswer, RemovalQuestion, UntrackedAnswer, UntrackedQuestion, VerdictAnswer, WordAnswer]

init_for_host : {} -> Box(Model)
init_for_host = |{}| Box.box((core.init)({}))

## One step: an event's words in, the effect list's words out. The box is unboxed and boxed
## again so the model stays unique: the host keeps no reference to the old one.
step_for_host : Box(Model), List(U64) -> Stepped(Model)
step_for_host = |boxed, event| {
	stepped = (core.step)(Box.unbox(boxed), event)
	{ model: Box.box(stepped.model), effects: stepped.effects }
}

## The model-free questions still on words: a question's words in, its answer's words out.
tile_for_host : List(U64) -> List(U64)
tile_for_host = |shape| (core.tile)(shape)

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
