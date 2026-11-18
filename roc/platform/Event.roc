## A fact for the core: what the platform observed, parsed into plain data.
Event := [
	## The pointer is now doing this.
	Pointer([Idle, Pressing({ pid : I32, idx : U32 }), Dragging({ pid : I32, idx : U32 })]),

	## Layout wants `window` at `desired`; the model last saw it at `known`.
	Want(
		{
			now : U64,
			window : { pid : I32, idx : U32 },
			desired : { x : F64, y : F64, w : F64, h : F64 },
			known : { x : F64, y : F64, w : F64, h : F64 },
			want : [Arrange, ArrangePositions, Place],
		},
	),

	## A modifier drag moved the dragged window's interactive frame.
	Follow({ window : { pid : I32, idx : U32 }, frame : { x : F64, y : F64, w : F64, h : F64 } }),

	## The app answered its latest write to `window`.
	Answer(
		{
			now : U64,
			window : { pid : I32, idx : U32 },
			frame : { x : F64, y : F64, w : F64, h : F64 },
		},
	),

	## `window` reported `frame` unprompted; the model last saw it at `known`.
	Report(
		{
			window : { pid : I32, idx : U32 },
			frame : { x : F64, y : F64, w : F64, h : F64 },
			known : { x : F64, y : F64, w : F64, h : F64 },
		},
	),

	## The deadline timer fired; `known` is where the model last saw each window it knows.
	Tick(
		{
			now : U64,
			known : List({ window : { pid : I32, idx : U32 }, frame : { x : F64, y : F64, w : F64, h : F64 } }),
		},
	),

	## Nothing is outstanding for `window` any more.
	Forget({ pid : I32, idx : U32 }),

	## May Rift take this initiative now?
	Gate([UserInput([Steering, Inert]), Arrange, RetireWindows, SwitchWorkspaceFocus, WarpFocus]),
]
