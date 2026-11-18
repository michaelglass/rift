## Rift's core platform: the host (Rust) owns the event loop and every effect; the app is one
## pure step function. The model is opaque to the host: it holds a `Box(Model)` it can neither
## read nor free, and hands it back on every step.
platform ""
	requires {
		[Model : model] for core : {
			init : {} -> model,
			step : model, Event -> { model : model, effects : List(Effect) },
		}
	}
	exposes [Effect, Event]
	packages {}
	provides {
		"roc_init": init_for_host,
		"roc_release_model": release_model_for_host,
		"roc_step": step_for_host,
	}
	targets: {
		arm64mac: {
			inputs: [app],
			output: Archive,
		},
	}

import Event
import Effect

init_for_host : {} -> Box(Model)
init_for_host = |{}| Box.box((core.init)({}))

## One step. The box is unboxed and boxed again so the model stays unique: the host keeps no
## reference to the old one.
step_for_host : Box(Model), Event -> { model : Box(Model), effects : List(Effect) }
step_for_host = |boxed, event| {
	stepped = (core.step)(Box.unbox(boxed), event)
	{ model: Box.box(stepped.model), effects: stepped.effects }
}

## The host cannot release a `Box(Model)`: only Roc knows the model's layout.
release_model_for_host : Box(Model) -> {}
release_model_for_host = |_| {}
