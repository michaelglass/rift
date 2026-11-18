## A decision for the platform.
Effect := [
	## Send this frame to the window's app, under its request.
	Write({ window : { pid : I32, idx : U32 }, frame : { x : F64, y : F64, w : F64, h : F64 }, request : U32 }),

	## A write is with the app; the desire waits behind it.
	Queued({ pid : I32, idx : U32 }),

	## What the app's answer was.
	Answered({ window : { pid : I32, idx : U32 }, answer : [Acked, Refused, Unasked] }),

	## The answer names a write Rift has since replaced: dropped.
	Stale({ pid : I32, idx : U32 }),

	## The answer came after Rift stopped waiting: news, to come back as a report.
	Late({ pid : I32, idx : U32 }),

	## What an unprompted report was.
	Observed({ window : { pid : I32, idx : U32 }, observation : [Ack, InTransit, Foreign] }),

	## Something decided along the way, for the log.
	Note({ window : { pid : I32, idx : U32 }, note : [Discarded, TimedOut, Forgotten] }),

	## Whether Rift may take `initiative`.
	Gate(
		{
			initiative : [UserInput([Steering, Inert]), Arrange, RetireWindows, SwitchWorkspaceFocus, WarpFocus],
			allowed : Bool,
		},
	),

	## Arm the deadline timer, or disarm it.
	Timer([Disarm, Arm(U64)]),
]
