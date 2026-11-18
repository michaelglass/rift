## Rift's core in Roc: the frame reconciler and the pointer's mode gate, as one pure step.
## It mirrors `rift_core::RustCore` decision for decision; the differential test holds it to that.
app [core] { pf: platform "../platform/main.roc" }

import pf.Event
import pf.Effect

WindowId : { pid : I32, idx : U32 }

Rect : { x : F64, y : F64, w : F64, h : F64 }

Pointer : [Idle, Pressing(WindowId), Dragging(WindowId)]

## Where one window stands between what Rift asked of its app and what the app answered.
## A window with nothing outstanding is not in `unsettled` at all (it is settled).
Sync : [
	InFlight({ target : Rect, deadline : U64, queued : Queued, request : U32 }),
	Refused({ wanted : Rect, got : Rect }),
	Adrift,
]

## A desire waiting behind the write in flight: the latest, or none.
Queued : [Waiting(Rect), NoneWaiting]

Armed : [Disarmed, Armed(U64)]

Model := {
	unsettled : Dict(WindowId, Sync),

	## Every write sent, with its deadline, in send order (= deadline order).
	deadlines : List({ deadline : U64, window : WindowId }),
	pointer : Pointer,
	armed : Armed,

	## What was decided along the way, emitted after the step's decisions.
	notes : List(Effect),
}

core = { init, step }

init : {} -> Model
init = |{}| {
	unsettled: Dict.empty(),
	deadlines: [],
	pointer: Idle,
	armed: Disarmed,
	notes: [],
}

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

answers : Rect, Rect -> Bool
answers = |target, frame| is_within(frame, ack_tolerance, target)

## Whose `window`'s frame is: the user's while their hand is on it, and every window's while
## they drag one.
owner : Pointer, WindowId -> [Rift, User]
owner = |pointer, window|
	match pointer {
		Dragging(_) => User
		Pressing(pressed) => if (pressed == window) User else Rift
		Idle => Rift
	}

holder : Pointer, WindowId -> [User, Nobody]
holder = |pointer, window|
	match pointer {
		Pressing(held) => if (held == window) User else Nobody
		Dragging(held) => if (held == window) User else Nobody
		Idle => Nobody
	}

may : [UserInput([Steering, Inert]), Arrange, RetireWindows, SwitchWorkspaceFocus, WarpFocus], Pointer -> Bool
may = |initiative, pointer|
	match (initiative, pointer) {
		(UserInput(Inert), Idle) | (UserInput(Inert), Pressing(_)) | (UserInput(Inert), Dragging(_)) => True
		(UserInput(Steering), Idle) => True
		(UserInput(Steering), Pressing(_)) => False
		(UserInput(Steering), Dragging(_)) => False
		(Arrange, Dragging(_)) => False
		(RetireWindows, Dragging(_)) => False
		(SwitchWorkspaceFocus, Dragging(_)) => False
		(WarpFocus, Dragging(_)) => False
		(Arrange, Idle) | (Arrange, Pressing(_)) => True
		(RetireWindows, Idle) | (RetireWindows, Pressing(_)) => True
		(SwitchWorkspaceFocus, Idle) | (SwitchWorkspaceFocus, Pressing(_)) => True
		(WarpFocus, Idle) | (WarpFocus, Pressing(_)) => True
	}

## The window's sync state; `Settled` when nothing is outstanding.
sync_of : Model, WindowId -> [Settled, InFlight({ target : Rect, deadline : U64, queued : Queued, request : U32 }), Refused({ wanted : Rect, got : Rect }), Adrift]
sync_of = |model, window|
	match model.unsettled.get(window) {
		Ok(InFlight(flight)) => InFlight(flight)
		Ok(Refused(refusal)) => Refused(refusal)
		Ok(Adrift) => Adrift
		Err(KeyNotFound) => Settled
	}

set : Model, WindowId, Sync -> Model
set = |model, window, sync| { ..model, unsettled: model.unsettled.insert(window, sync) }

settle : Model, WindowId -> Model
settle = |model, window| { ..model, unsettled: model.unsettled.remove(window) }

note : Model, WindowId, [Discarded, TimedOut, Forgotten] -> Model
note = |model, window, what| { ..model, notes: model.notes.append(Note({ window, note: what })) }

send : Model, U64, U32, WindowId, Rect -> (Model, Effect)
send = |model, now, request, window, frame| {
	deadline = now + write_deadline_ns
	queued_up = { ..model, deadlines: model.deadlines.append({ deadline, window }) }
	(set(queued_up, window, InFlight({ target: frame, deadline, queued: NoneWaiting, request })), Write({ window, frame, request }))
}

time_out : Model, WindowId, Rect, Rect -> Model
time_out = |model, window, target, known|
	set(note(model, window, TimedOut), window, Refused({ wanted: target, got: known }))

want : Model, { now : U64, request : U32, window : WindowId, desired : Rect, known : Rect, want : [Arrange, ArrangePositions, Place] } -> (Model, List(Effect))
want = |model0, { now, request, window, desired, known, want: how }| {
	# A write nobody answered in time no longer holds newer desires back.
	(model, sync) =
		match sync_of(model0, window) {
			InFlight({ target, deadline, .. }) if now >= deadline => {
				timed_out = time_out(model0, window, target, known)
				(timed_out, Refused({ wanted: target, got: known }))
			}
			other => (model0, other)
		}
	match owner(model.pointer, window) {
		User => {
			discarded = note(model, window, Discarded)
			next =
				match (how, sync) {
					(Place, Settled) | (Place, Refused(_)) | (Place, Adrift) => set(discarded, window, Adrift)
					(Place, InFlight(_)) => discarded
					(Arrange, Settled) | (Arrange, InFlight(_)) | (Arrange, Refused(_)) | (Arrange, Adrift) => discarded
					(ArrangePositions, Settled) | (ArrangePositions, InFlight(_)) | (ArrangePositions, Refused(_)) | (ArrangePositions, Adrift) => discarded
				}
			(next, [])
		}

		Rift =>
			match (sync, how) {
				(InFlight({ target, deadline, queued, request: sent }), Arrange) | (InFlight({ target, deadline, queued, request: sent }), ArrangePositions) | (InFlight({ target, deadline, queued, request: sent }), Place) =>
					if same_as(desired, target) {
						match queued {
							Waiting(_) => (set(model, window, InFlight({ target, deadline, queued: NoneWaiting, request: sent })), [])
							NoneWaiting => (model, [])
						}
					} else {
						(set(model, window, InFlight({ target, deadline, queued: Waiting(desired), request: sent })), [Queued(window)])
					}

				(Refused({ wanted, got }), Arrange) | (Refused({ wanted, got }), ArrangePositions) if same_as(desired, wanted) and same_as(known, got) =>
					(model, [])

				(Settled, Arrange) | (Settled, ArrangePositions) | (Refused(_), Arrange) | (Refused(_), ArrangePositions) if same_as(desired, known) =>
					(model, [])

				(Settled, Place) | (Refused(_), Place) | (Adrift, Arrange) | (Adrift, ArrangePositions) | (Adrift, Place) | (Settled, Arrange) | (Settled, ArrangePositions) | (Refused(_), Arrange) | (Refused(_), ArrangePositions) => {
					(written, write) = send(model, now, request, window, desired)
					(written, [write])
				}
			}
	}
}

## What waited behind a write that has been answered or given up on.
release : Model, U64, U32, WindowId, Queued, [Rift, User] -> (Model, List(Effect))
release = |model, now, request, window, queued, whose|
	match (whose, queued) {
		(Rift, Waiting(next)) => {
			(sent, write) = send(model, now, request, window, next)
			(sent, [write])
		}

		(Rift, NoneWaiting) | (User, NoneWaiting) => (model, [])
		(User, Waiting(_)) => (set(note(model, window, Discarded), window, Adrift), [])
	}

answer : Model, { now : U64, request : U32, seen : [Seen(U32), Unnamed], window : WindowId, frame : Rect } -> (Model, List(Effect))
answer = |model, { now, request, seen, window, frame }|
	match sync_of(model, window) {
		Settled | Adrift => (model, [Answered({ window, answer: Unasked })])
		InFlight({ request: sent, .. }) if names_another(seen, sent) => (model, [Stale(window)])
		InFlight({ target, queued, .. }) => {
			(answered, verdict) =
				if answers(target, frame) {
					(settle(model, window), Acked)
				} else {
					(set(model, window, Refused({ wanted: target, got: frame })), Refused)
				}
			(released, writes) = release(answered, now, request, window, queued, owner(model.pointer, window))
			(released, List.concat([Answered({ window, answer: verdict })], writes))
		}

		Refused(_) => (model, [Late(window)])
	}

observe : Model, { window : WindowId, frame : Rect, known : Rect } -> (Model, List(Effect))
observe = |model, { window, frame, known }| {
	hand = holder(model.pointer, window)
	seen = |what| [Observed({ window, observation: what })]
	match (sync_of(model, window), hand) {
		(InFlight({ target, .. }), User) | (InFlight({ target, .. }), Nobody) if answers(target, frame) => (model, seen(Ack))
		(InFlight(_), Nobody) => (model, seen(InTransit))
		(InFlight(_), User) | (Settled, User) | (Settled, Nobody) | (Refused(_), User) | (Refused(_), Nobody) | (Adrift, User) | (Adrift, Nobody) =>
			if same_as(frame, known) {
				(model, seen(Ack))
			} else {
				# Whatever Rift was waiting for, the window is someone else's now.
				next =
					match hand {
						User => set(model, window, Adrift)
						Nobody => settle(model, window)
					}
				(next, seen(Foreign))
			}
	}
}

## Whether an answer names a request other than `sent`.
names_another : [Seen(U32), Unnamed], U32 -> Bool
names_another = |seen, sent|
	match seen {
		Seen(request) => request != sent
		Unnamed => False
	}

## The write a deadlines entry stands for, if it is still waiting.
waiting : Model, { deadline : U64, window : WindowId } -> [Waits({ target : Rect, queued : Queued }), NotWaiting]
waiting = |model, entry|
	match sync_of(model, entry.window) {
		InFlight({ target, deadline, queued, .. }) if deadline == entry.deadline => Waits({ target, queued })
		InFlight(_) | Settled | Refused(_) | Adrift => NotWaiting
	}

expire : Model, { now : U64, request : U32, known : List({ window : WindowId, frame : Rect }) } -> (Model, List(Effect))
expire = |model0, { now, request, known }| {
	var $model = model0
	var $released = []
	var $going = True
	while $going {
		match $model.deadlines.first() {
			Ok(entry) if entry.deadline <= now => {
				popped = { ..$model, deadlines: $model.deadlines.drop_first(1) }
				match waiting(popped, entry) {
					NotWaiting => {
						$model = popped
					}

					Waits({ target, queued }) =>
						match known.find_first(|k| k.window == entry.window) {
							Ok({ frame, .. }) => {
								timed_out = time_out(popped, entry.window, target, frame)
								(released, writes) = release(timed_out, now, request, entry.window, queued, owner(popped.pointer, entry.window))
								$model = released
								$released = $released.concat(writes)
							}

							Err(NotFound) => {
								$model = settle(note(popped, entry.window, Forgotten), entry.window)
							}
						}
				}
			}

			# The front write is not due yet, or nothing is waiting.
			Ok(_) | Err(ListWasEmpty) => {
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
		match $model.deadlines.first() {
			Ok(entry) =>
				match waiting($model, entry) {
					Waits(_) => {
						$found = Armed(entry.deadline)
						$going = False
					}

					NotWaiting => {
						$model = { ..$model, deadlines: $model.deadlines.drop_first(1) }
					}
				}

			Err(ListWasEmpty) => {
				$going = False
			}
		}
	}
	($model, $found)
}

decide : Model, Event -> (Model, List(Effect))
decide = |model, event|
	match event {
		Pointer(now) => ({ ..model, pointer: now }, [])
		Want(desire) => want(model, desire)
		Follow({ request, window, frame }) => (set(model, window, Adrift), [Write({ window, frame, request })])
		Answer(reply) => answer(model, reply)
		Report(report) => observe(model, report)
		Tick(tick) => expire(model, tick)
		Forget(window) => (settle(model, window), [])
		Gate(initiative) => (model, [Gate({ initiative, allowed: may(initiative, model.pointer) })])
	}

## Decisions first, then the notes taken while deciding, then the timer if its deadline moved.
step : Model, Event -> { model : Model, effects : List(Effect) }
step = |model0, event| {
	(decided, decisions) = decide({ ..model0, notes: [] }, event)
	(pruned, next) = next_deadline(decided)
	timer =
		if next == pruned.armed {
			[]
		} else {
			match next {
				Armed(at) => [Timer(Arm(at))]
				Disarmed => [Timer(Disarm)]
			}
		}
	effects = decisions.concat(pruned.notes).concat(timer)
	{ model: { ..pruned, armed: next, notes: [] }, effects }
}
