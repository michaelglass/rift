## Rift's core in Roc: the frame reconciler and the user's grip, as one pure step. Events arrive
## and effects leave as words (`rift_core::plain::wire`); they are parsed into the types below at
## the boundary. It mirrors `rift_core::RustCore` decision for decision; the differential test
## holds it to that.
app [core] { pf: platform "../platform/main.roc" }

import Wire exposing [finished, put_list, put_rect, put_window, take, take_flag, take_list, take_rect, take_u32, take_window]
import Bsp
import Traditional
import Scrolling
import Floating
import Topology
import Lifecycle

WindowId : { pid : I32, idx : U32 }

Rect : { x : F64, y : F64, w : F64, h : F64 }

## Words being read, and where.
Words : { words : List(U64), at : U64 }

## A value read and where the rest starts, or the words were malformed.
Taken(a) : Try((a, Words), [Malformed])

Desire : { window : WindowId, frame : Rect, request : U32 }

WriteFx : { window : WindowId, frame : Rect, request : U32 }

Button : [Left, Right]

Event : [
	Press({ on : [OnWindow(WindowId), OnNothing], button : Button }),
	Drag(WindowId),
	Release([ButtonUp(Button), Missed]),
	LetGo,
	Gone(WindowId),
	Rekeyed({ from : WindowId, to : WindowId }),
	Seen({ window : WindowId, frame : Rect }),
	Ceded(WindowId),
	Unseen(WindowId),
	Distrust(List(WindowId)),
	Arrange({ now : U64, want : [Arrange, ArrangePositions], desires : List(Desire) }),
	Animate({ now : U64, desires : List(Desire) }),
	AnimationEnded({ now : U64, request : U32 }),
	AnimationStopped({ window : WindowId, request : U32, at : [StoppedAt(Rect), NotPublished] }),
	Follow({ request : U32, window : WindowId, frame : Rect }),
	Observe({ now : U64, request : U32, window : WindowId, frame : Rect, as_of : [AsOf(U32), Unstamped] }),
	Tick({ now : U64, request : U32 }),
	Forget(WindowId),
	SentOutside({ window : WindowId, request : U32 }),
]

Reply : [NoNews, InTransit, Acked, Refused, Foreign]

Effect : [
	Write(WriteFx),
	Animate({ window : WindowId, from : Rect, to : Rect, request : U32 }),
	Pending({ window : WindowId, target : [Target(Rect), NoTarget] }),
	Plan({ request : U32, positions : List(WriteFx), frames : List(WriteFx) }),
	Queued(WindowId),
	Seen({ window : WindowId, frame : Rect }),
	Replied({ window : WindowId, reply : Reply }),
	Note({ window : WindowId, note : [Discarded, TimedOut] }),
	Timer(Armed),
	Grip(WindowId),
	DragBegan(WindowId),
	Ungrip({ window : WindowId, why : [Released, LetGo, Gone], changed : List(WindowId) }),
	Regrip({ from : WindowId, to : WindowId }),
]

## The request of the latest frame Rift sent a window, if any.
Last : [Sent(U32), NeverSent]

## What the app had acknowledged when a write was sent.
Before : [AckedBefore(Rect), UnknownBefore]

## A desire waiting behind the write in flight: the latest, or none.
Waiting : [Waiting(Rect), NoneWaiting]

## Where one window stands between what Rift asked of its app and what the app answered.
Sync : [
	Settled({ acked : Rect, last : Last }),
	Unknown(Last),
	InFlight({ target : Rect, deadline : U64, queued : Waiting, request : U32, before : Before }),
	Animating({ target : Rect, request : U32 }),
	Refused({ wanted : Rect, got : Rect, request : U32 }),

	## Rift moves the window outside the core (a gesture's camera) under `request`, which the
	## platform reported; `at` is where the app last reported it.
	Outside({ request : U32, at : Rect }),
]

## What the model has of one window: where it takes it to be, and where it stands.
Record : { seen : Rect, sync : Sync }

Grip : [Idle, Held({ window : WindowId, button : Button, phase : [Pressed, Dragging(List(WindowId))] })]

Armed : [Armed(U64), Disarmed]

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

core = { init, step, tile }

## The model-free questions: layout geometry and the topology decisions. A question's words
## (its tag first: 0 BSP, 1 traditional, 2 a scrolling strip, 3 a settle question on a strip's
## placement, 4 a floating layout's newcomers; 5 to 8 the topology decisions, `Topology`; 9 to
## 16 the lifecycle decisions, `Lifecycle`) in;
## out, a 0 and the answer's words (the frames; for a strip, where it placed everything; for a
## settle question, where the camera settles; for newcomers, their first frames; for a topology
## question, its answer), or a 1: the words were not a question it answers.
tile : List(U64) -> List(U64)
tile = |words|
	match shape_answer(words) {
		Ok(answer) => [0].concat(answer)
		# Not a shape: one explicit rejection.
		Err(Malformed) => [1]
	}

shape_answer : List(U64) -> Try(List(U64), [Malformed])
shape_answer = |words| {
	(system, cursor) = take({ words, at: 0 })?
	match system {
		0 => Bsp.frames_of(cursor).map_ok(put_frames)
		1 => Traditional.frames_of(cursor).map_ok(put_frames)
		2 => Scrolling.placed_words(cursor)
		3 => Scrolling.settled_words(cursor)
		4 => Floating.first_frames_words(cursor)
		5 => Topology.discovered_words(cursor)
		6 => Topology.discovery_words(cursor)
		7 => Topology.admit_words(cursor)
		8 => Topology.candidates_words(cursor)
		9 => Lifecycle.rule_targets_words(cursor)
		10 => Lifecycle.rule_refresh_words(cursor)
		11 => Lifecycle.pointer_target_words(cursor)
		12 => Lifecycle.native_transient_words(cursor)
		13 => Lifecycle.keeps_transient_words(cursor)
		14 => Lifecycle.misplaced_words(cursor)
		15 => Lifecycle.out_of_view_words(cursor)
		16 => Lifecycle.title_change_words(cursor)
		_ => Err(Malformed)
	}
}

put_frames : List((WindowId, Rect)) -> List(U64)
put_frames = |frames| put_list([], frames, |out, (window, frame)| put_rect(put_window(out, window), frame))

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
		Outside({ request, .. }) => Sent(request)
	}

pending_of : Sync -> [Target(Rect), NoTarget]
pending_of = |sync|
	match sync {
		InFlight({ target, .. }) => Target(target)
		Animating({ target, .. }) => Target(target)
		Refused({ wanted, .. }) => Target(wanted)
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
							Outside(_) => Unknown(last_of(sync))
						}
					{ ..model, windows: model.windows.insert(to, { seen, sync: kept }), touched: model.touched.append(to) }
				}

			Err(_) => model
		}
	remove(moved, from)
}

note : Model, WindowId, [Discarded, TimedOut] -> Model
note = |model, window, what| { ..model, notes: model.notes.append(Note({ window, note: what })) }

distrust : Model, WindowId -> Model
distrust = |model, window|
	match model.windows.get(window) {
		Err(_) => model
		Ok(record) =>
			match record.sync {
				InFlight(flight) => set(model, window, InFlight({ ..flight, before: UnknownBefore }))
				Settled(_) | Refused(_) | Unknown(_) | Animating(_) | Outside(_) => set(model, window, Unknown(last_of(record.sync)))
			}
	}

send : Model, U64, U32, WindowId, Rect, Before -> (Model, WriteFx)
send = |model, now, request, window, frame, before| {
	deadline = now + write_deadline_ns
	queued_up = { ..model, deadlines: model.deadlines.append({ deadline, window }) }
	sent = set(queued_up, window, InFlight({ target: frame, deadline, queued: NoneWaiting, request, before }))
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

					Settled(_) | Refused(_) | Unknown(_) | Animating(_) | Outside(_) => (model0, sync0)
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
		(sent, fx) = send(model, now, request, window, desired, before_of(sync))
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
			(sent, fx) = send(model, now, request, window, next, before_of(after))
			(sent, [fx])
		}
	}

Read : { now : U64, request : U32, window : WindowId, frame : Rect, as_of : [AsOf(U32), Unstamped] }

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
		# The answer to a write Rift stopped waiting for: news like any report.
		Settled(_) | Unknown(_) | Refused(_) => placed({})
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
				Settled(_) | Unknown(_) | InFlight(_) | Animating(_) => settled(NoNews)
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

		Settled(_) | Refused(_) | Unknown(_) | Outside(_) => news({})
	}
}

## The write a deadlines entry stands for, if it is still waiting.
waiting : Model, { deadline : U64, window : WindowId } -> [Waits({ target : Rect, queued : Waiting, request : U32, before : Before }), NotWaiting]
waiting = |model, entry|
	match model.windows.get(entry.window) {
		Ok(record) =>
			match record.sync {
				InFlight({ target, deadline, queued, request, before }) =>
					if deadline == entry.deadline {
						Waits({ target, queued, request, before })
					} else {
						NotWaiting
					}

				Settled(_) | Refused(_) | Unknown(_) | Animating(_) | Outside(_) => NotWaiting
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
					# A write is with the app, or where the app has the window is unknown: written.
					InFlight(_) | Unknown(_) => Written
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
					Settled(_) | Refused(_) | Unknown(_) | InFlight(_) | Outside(_) => False
				},
		).sort_with(|(a, _), (b, _)| window_order(a, b))
	deadline = now + write_deadline_ns
	var $model = model0
	for (window, record) in ended {
		match record.sync {
			Animating({ target, .. }) => {
				queued_up = { ..$model, deadlines: $model.deadlines.append({ deadline, window }) }
				$model = set(queued_up, window, InFlight({ target, deadline, queued: NoneWaiting, request, before: UnknownBefore }))
			}

			Settled(_) | Refused(_) | Unknown(_) | InFlight(_) | Outside(_) => {}
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

				InFlight({ request: sent, .. }) =>
					if (sent == request) set(model, window, Unknown(Sent(request))) else model

				Settled(_) | Refused(_) | Unknown(_) | Outside(_) => model
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

ungrip : Model, [Released, LetGo, Gone] -> (Model, List(Effect))
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

				Settled(_) | Unknown(_) | Animating(_) | Refused(_) | Outside(_) => forget(model, window)
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

PlanFx : { request : U32, positions : List(WriteFx), frames : List(WriteFx) }

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

arrange : Model, U64, [Arrange, ArrangePositions], List(Desire) -> (Model, List(Effect))
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
							if pressed == window {
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
							ButtonUp(released) => released == button
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
			(sent, fx) = all_written(noted, released)
			(sent, [Replied({ window, reply })].concat(fx))
		}

		Tick({ now, request }) => {
			(expired, released) = expire(model, now, request, grip)
			all_written(expired, released)
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
step : Model, List(U64) -> { model : Model, effects : List(U64) }
step = |model0, words|
	match parse_event(words) {
		Ok(event) => {
			(decided, decisions) = decide({ ..model0, notes: [], touched: [] }, event)
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
			{ model: { ..pruned, armed: next, notes: [], touched: [] }, effects: [0].concat(put_effects(effects)) }
		}

		# Not an event: one explicit rejection, the model untouched.
		Err(Malformed) => { model: model0, effects: [1] }
	}

# ---------------------------------------------------------------------------------------------
# The wire: words in, words out (the layout is `rift_core::plain::wire`'s).

take_desire : Words -> Taken(Desire)
take_desire = |cursor| {
	(window, c1) = take_window(cursor)?
	(frame, c2) = take_rect(c1)?
	(request, c3) = take_u32(c2)?
	Ok(({ window, frame, request }, c3))
}

take_button : Words -> Taken(Button)
take_button = |cursor| {
	(word, rest) = take(cursor)?
	match word {
		0 => Ok((Left, rest))
		1 => Ok((Right, rest))
		_ => Err(Malformed)
	}
}

## The event `words` hold, all of them: anything else is `Malformed`.
parse_event : List(U64) -> Try(Event, [Malformed])
parse_event = |words| {
	(event, rest) = take_event({ words, at: 0 })?
	finished(rest)?
	Ok(event)
}

take_event : Words -> Taken(Event)
take_event = |c0| {
	(tag, c) = take(c0)?
	match tag {
		0 => {
			(flag, c1) = take_flag(c)?
			if flag {
				(window, c2) = take_window(c1)?
				(button, c3) = take_button(c2)?
				Ok((Press({ on: OnWindow(window), button }), c3))
			} else {
				(button, c2) = take_button(c1)?
				Ok((Press({ on: OnNothing, button }), c2))
			}
		}

		1 => {
			(window, c1) = take_window(c)?
			Ok((Drag(window), c1))
		}

		2 => {
			(word, c1) = take(c)?
			match word {
				0 => Ok((Release(ButtonUp(Left)), c1))
				1 => Ok((Release(ButtonUp(Right)), c1))
				2 => Ok((Release(Missed), c1))
				_ => Err(Malformed)
			}
		}

		3 => Ok((LetGo, c))
		4 => {
			(window, c1) = take_window(c)?
			Ok((Gone(window), c1))
		}

		5 => {
			(from, c1) = take_window(c)?
			(to, c2) = take_window(c1)?
			Ok((Rekeyed({ from, to }), c2))
		}

		6 => {
			(window, c1) = take_window(c)?
			(frame, c2) = take_rect(c1)?
			Ok((Seen({ window, frame }), c2))
		}

		7 => {
			(window, c1) = take_window(c)?
			Ok((Ceded(window), c1))
		}

		8 => {
			(window, c1) = take_window(c)?
			Ok((Unseen(window), c1))
		}

		9 => {
			(windows, c1) = take_list(c, take_window)?
			Ok((Distrust(windows), c1))
		}

		10 => {
			(now, c1) = take(c)?
			(how, c2) = take(c1)?
			(desires, c3) = take_list(c2, take_desire)?
			match how {
				0 => Ok((Arrange({ now, want: Arrange, desires }), c3))
				1 => Ok((Arrange({ now, want: ArrangePositions, desires }), c3))
				_ => Err(Malformed)
			}
		}

		11 => {
			(now, c1) = take(c)?
			(desires, c2) = take_list(c1, take_desire)?
			Ok((Animate({ now, desires }), c2))
		}

		12 => {
			(now, c1) = take(c)?
			(request, c2) = take_u32(c1)?
			Ok((AnimationEnded({ now, request }), c2))
		}

		13 => {
			(window, c1) = take_window(c)?
			(request, c2) = take_u32(c1)?
			(published, c3) = take_flag(c2)?
			if published {
				(frame, c4) = take_rect(c3)?
				Ok((AnimationStopped({ window, request, at: StoppedAt(frame) }), c4))
			} else {
				Ok((AnimationStopped({ window, request, at: NotPublished }), c3))
			}
		}

		14 => {
			(request, c1) = take_u32(c)?
			(window, c2) = take_window(c1)?
			(frame, c3) = take_rect(c2)?
			Ok((Follow({ request, window, frame }), c3))
		}

		15 => {
			(now, c1) = take(c)?
			(request, c2) = take_u32(c1)?
			(window, c3) = take_window(c2)?
			(frame, c4) = take_rect(c3)?
			(stamped, c5) = take_flag(c4)?
			if stamped {
				(as_of, c6) = take_u32(c5)?
				Ok((Observe({ now, request, window, frame, as_of: AsOf(as_of) }), c6))
			} else {
				Ok((Observe({ now, request, window, frame, as_of: Unstamped }), c5))
			}
		}

		16 => {
			(now, c1) = take(c)?
			(request, c2) = take_u32(c1)?
			Ok((Tick({ now, request }), c2))
		}

		17 => {
			(window, c1) = take_window(c)?
			Ok((Forget(window), c1))
		}

		18 => {
			(window, c1) = take_window(c)?
			(request, c2) = take_u32(c1)?
			Ok((SentOutside({ window, request }), c2))
		}

		_ => Err(Malformed)
	}
}

put_request : List(U64), U32 -> List(U64)
put_request = |words, request| words.append(request.to_u64())

put_write : List(U64), WriteFx -> List(U64)
put_write = |words, write| put_request(put_rect(put_window(words, write.window), write.frame), write.request)

reply_code : Reply -> U64
reply_code = |reply|
	match reply {
		NoNews => 0
		InTransit => 1
		Acked => 2
		Refused => 3
		Foreign => 4
	}

put_effect : List(U64), Effect -> List(U64)
put_effect = |words, effect|
	match effect {
		Write(write) => put_write(words.append(0), write)
		Animate({ window, from, to, request }) => put_request(put_rect(put_rect(put_window(words.append(1), window), from), to), request)
		Pending({ window, target }) => {
			at = put_window(words.append(2), window)
			match target {
				Target(frame) => put_rect(at.append(1), frame)
				NoTarget => at.append(0)
			}
		}

		Plan({ request, positions, frames }) => put_list(put_list(put_request(words.append(3), request), positions, put_write), frames, put_write)
		Queued(window) => put_window(words.append(4), window)
		Seen({ window, frame }) => put_rect(put_window(words.append(5), window), frame)
		Replied({ window, reply }) => put_window(words.append(6), window).append(reply_code(reply))
		Note({ window, note: what }) =>
			match what {
				Discarded => put_window(words.append(7), window).append(0)
				TimedOut => put_window(words.append(7), window).append(1)
			}

		Timer(armed) =>
			match armed {
				Armed(at) => words.append(8).append(1).append(at)
				Disarmed => words.append(8).append(0)
			}

		Grip(window) => put_window(words.append(9), window)
		DragBegan(window) => put_window(words.append(10), window)
		Ungrip({ window, why, changed }) => {
			code =
				match why {
					Released => 0
					LetGo => 1
					Gone => 2
				}
			put_list(put_window(words.append(11), window).append(code), changed, put_window)
		}

		Regrip({ from, to }) => put_window(put_window(words.append(12), from), to)
	}

put_effects : List(Effect) -> List(U64)
put_effects = |effects| {
	var $words = [effects.len()]
	for effect in effects {
		$words = put_effect($words, effect)
	}
	$words
}
