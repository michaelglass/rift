## Rift's core platform: the host (Rust) owns the event loop and every effect; the app is a
## pure step function (and a pure geometry function). Events and effects cross as words (`rift_core::plain::wire`); the app
## parses them into its own types. The model is opaque to the host: it holds a `Box(Model)` it
## can neither read nor free, and hands it back on every step.
platform ""
	requires {
		[Model : model] for core : {
			init : {} -> model,
			step : model, List(U64) -> { model : model, effects : List(U64) },
			tile : List(U64) -> List(U64),
		}
	}
	exposes [Boundary]
	packages {}
	provides {
		"roc_init": init_for_host,
		"roc_release_model": release_model_for_host,
		"roc_step": step_for_host,
		"roc_tile": tile_for_host,
	}
	targets: {
		arm64mac: {
			inputs: [app],
			output: Archive,
		},
	}

import Boundary exposing [Stepped]

init_for_host : {} -> Box(Model)
init_for_host = |{}| Box.box((core.init)({}))

## One step: an event's words in, the effect list's words out. The box is unboxed and boxed
## again so the model stays unique: the host keeps no reference to the old one.
step_for_host : Box(Model), List(U64) -> Stepped(Model)
step_for_host = |boxed, event| {
	stepped = (core.step)(Box.unbox(boxed), event)
	{ model: Box.box(stepped.model), effects: stepped.effects }
}

## Layout geometry: a layout shape's words in, its frames' words out. Needs no model.
tile_for_host : List(U64) -> List(U64)
tile_for_host = |shape| (core.tile)(shape)

## The host cannot release a `Box(Model)`: only Roc knows the model's layout.
release_model_for_host : Box(Model) -> {}
release_model_for_host = |_| {}
