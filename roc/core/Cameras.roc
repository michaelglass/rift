## The scrolling layouts' cameras (`rift_core::camera`): for each layout, where its viewport is
## on the strip and how it comes to rest, the gesture's recent motion, the views it remembers,
## and the view it owes a fullscreen window. Each event carries the geometry (`CamSpans`) and
## the arrangement (`CamFacts`) it reads; the camera keeps neither. Layouts, columns and windows
## are one word each; a time is nanoseconds on the host's clock.

import pf.Boundary exposing [CamerasEvent, CameraEvent, CamAnswer, CamCamera, CamEntry, CamViewport, CamMotion, CamResting, CamSpring, CamPending, CamHistory, CamSample, CamShowing, CamOwed, CamBookmark, MaybeBookmark, CamSpans, CamSpan, CamSnap, MaybeSpans, CamFacts, CamFactColumn, CamSettings, MaybeHeld, CamPresented, CamEnded, CamRelease, CamExcess, CamRest, MaybeWord]

## Every layout's camera.
Kept : { cameras : Dict(U64, CamCamera) }

Cameras := [].{

	empty : {} -> Kept
	empty = |{}| { cameras: Dict.empty() }

	fresh : {} -> CamCamera
	fresh = |{}| CamCamera.{ viewport: Uninitialized, motion: no_history({}), transient_restore: NoBookmark, showing: Strip, restored_view: NoBookmark }

	no_history : {} -> CamHistory
	no_history = |{}| CamHistory.{ samples: [], overscroll: 0.0 }

	## One event; what the camera answered.
	step : Kept, CamerasEvent -> (Kept, CamAnswer)
	step = |kept, event|
		match event {
			Created(layout) => ({ cameras: kept.cameras.insert(layout, fresh({})) }, Nothing)
			Cloned(clone) => {
				copy =
					match kept.cameras.get(clone.from) {
						Ok(c) => cloned(c, clone.spans)
						Err(_) => fresh({})
					}
				({ cameras: kept.cameras.insert(clone.to, copy) }, Nothing)
			}
			Removed(layout) => ({ cameras: kept.cameras.remove(layout) }, Nothing)
			Seeded(seed) => ({ cameras: kept.cameras.insert(seed.layout, seed.camera) }, Nothing)
			Camera(on) =>
				match kept.cameras.get(on.layout) {
					Ok(c) => {
						# Taken out of the table first, so the table no longer shares it.
						rest = kept.cameras.remove(on.layout)
						(after, answer) = camera_step(c, on.event)
						({ cameras: rest.insert(on.layout, after) }, answer)
					}
					Err(_) => (kept, Nothing)
				}
		}

	# ---- time and the spring ------------------------------------------------------------------

	seconds_since : U64, U64 -> F64
	seconds_since = |now, earlier| if now > earlier U64.to_f64(now - earlier) / 1000000000.0 else 0.0

	later : U64, U64 -> U64
	later = |a, b| if a > b a else b

	omega : F64
	omega = 28.284271247461902

	deceleration_ln : F64
	deceleration_ln = -0.0030045090202987243

	## e^x from `+`, `*` and floor only: the Rust core's `camera::exp`, operation for operation.
	exp : F64 -> F64
	exp = |x|
		if x.is_nan() {
			x
		} else {
			match F64.floor_to_i32_try(x * 1.4426950408889634 + 0.5) {
				Ok(k) =>
					if k < -1020 {
						0.0
					} else if k > 1023 {
						F64.infinity
					} else {
						kf = I32.to_f64(k)
						r = (x - kf * 0.6931471803691238) - kf * 0.00000000019082149292705877
						p0 = 0.00000000016059043836821613
						p1 = p0 * r + 0.00000000208767569878681
						p2 = p1 * r + 0.00000002505210838544172
						p3 = p2 * r + 0.0000002755731922398589
						p4 = p3 * r + 0.0000027557319223985893
						p5 = p4 * r + 0.0000248015873015873
						p6 = p5 * r + 0.0001984126984126984
						p7 = p6 * r + 0.001388888888888889
						p8 = p7 * r + 0.008333333333333333
						p9 = p8 * r + 0.041666666666666664
						p10 = p9 * r + 0.16666666666666666
						p11 = p10 * r + 0.5
						p12 = p11 * r + 1.0
						p13 = p12 * r + 1.0
						p13 * F64.from_bits(I32.to_u64_wrap(k + 1023) * 4503599627370496)
					}
				Err(_) => if x < 0.0 0.0 else F64.infinity
			}
		}

	spring_new : F64, F64, F64, U64 -> CamSpring
	spring_new = |from, target, velocity, started| CamSpring.{ from, target, velocity, current: from, current_velocity: velocity, started, sampled: started }

	position_velocity : CamSpring, U64 -> (F64, F64)
	position_velocity = |s, now| {
		t = seconds_since(now, s.started)
		x0 = s.from - s.target
		b = omega * x0 + s.velocity
		envelope = exp(-omega * t)
		(s.target + envelope * (x0 + b * t), envelope * (s.velocity - omega * b * t))
	}

	## The spring advanced to `now` (never back), and whether it has come to rest.
	spring_sample : CamSpring, U64, F64 -> (CamSpring, Bool)
	spring_sample = |s, now0, scale| {
		now = later(now0, s.sampled)
		(position, speed) = position_velocity(s, now)
		sampled = CamSpring.{ from: s.from, target: s.target, velocity: s.velocity, current: position, current_velocity: speed, started: s.started, sampled: now }
		rests = (position - s.target).abs() <= 0.25 / scale and speed.abs() / omega <= 0.25 / scale
		(sampled, rests)
	}

	spring_rebased : CamSpring, F64 -> CamSpring
	spring_rebased = |s, delta| CamSpring.{ from: s.from + delta, target: s.target + delta, velocity: s.velocity, current: s.current + delta, current_velocity: s.current_velocity, started: s.started, sampled: s.sampled }

	# ---- motion and viewport ------------------------------------------------------------------

	towards : F64, F64 -> CamMotion
	towards = |from, target| if from == target Static(target) else Pending(CamPending.{ from, target })

	motion_offset : CamMotion -> F64
	motion_offset = |m|
		match m {
			Static(o) => o
			Pending(p) => p.target
			Animation(s) => s.current
		}

	motion_start : CamMotion -> F64
	motion_start = |m|
		match m {
			Pending(p) => p.from
			_ => motion_offset(m)
		}

	motion_target : CamMotion -> F64
	motion_target = |m|
		match m {
			Animation(s) => s.target
			_ => motion_offset(m)
		}

	## Where the motion comes to rest moved to `target`.
	move_target : CamMotion, F64, U64 -> CamMotion
	move_target = |m, target, now|
		match m {
			Animation(s) =>
				if target != s.target {
					when = later(now, s.sampled)
					(_, speed) = position_velocity(s, when)
					Animation(spring_new(s.current, target, speed, when))
				} else {
					m
				}
			_ => if target != motion_target(m) towards(motion_start(m), target) else m
		}

	motion_rebased : CamMotion, F64 -> CamMotion
	motion_rebased = |m, delta|
		match m {
			Static(o) => Static(o + delta)
			Pending(p) => Pending(CamPending.{ from: p.from + delta, target: p.target + delta })
			Animation(s) => Animation(spring_rebased(s, delta))
		}

	resting : CamRest, CamMotion -> CamViewport
	resting = |rest, motion| Resting(CamResting.{ rest, motion })

	at_rest : F64 -> CamViewport
	at_rest = |place| resting(At, Static(place))

	offset_of : CamViewport -> F64
	offset_of = |v|
		match v {
			Uninitialized => 0.0
			Gesture(o) => o
			Resting(r) => motion_offset(r.motion)
		}

	start_offset : CamViewport -> F64
	start_offset = |v|
		match v {
			Resting(r) => motion_start(r.motion)
			_ => offset_of(v)
		}

	target_of : CamViewport -> F64
	target_of = |v|
		match v {
			Resting(r) => motion_target(r.motion)
			_ => offset_of(v)
		}

	moving : CamViewport -> Bool
	moving = |v|
		match v {
			Gesture(_) => True
			Resting(r) =>
				match r.motion {
					Animation(_) => True
					_ => False
				}
			Uninitialized => False
		}

	uncentred : CamViewport -> CamViewport
	uncentred = |v|
		match v {
			Resting(r) => resting(At, r.motion)
			_ => v
		}

	finished : CamViewport -> CamViewport
	finished = |v|
		match v {
			Resting(r) => resting(r.rest, Static(motion_target(r.motion)))
			_ => v
		}

	aimed : CamViewport, F64, CamRest -> CamViewport
	aimed = |v, to, rest| resting(rest, towards(start_offset(v), to))

	set_target : CamViewport, F64 -> CamViewport
	set_target = |v, to| aimed(v, to, At)

	viewport_sampled : CamViewport, U64, F64 -> CamViewport
	viewport_sampled = |v, now, scale|
		match v {
			Resting(r) =>
				match r.motion {
					Static(_) => v
					Pending(p) => resting(r.rest, Static(p.target))
					Animation(s) => {
						(sampled, rests) = spring_sample(s, now, scale)
						if rests resting(r.rest, Static(sampled.target)) else resting(r.rest, Animation(sampled))
					}
				}
			_ => v
		}

	viewport_rebased : CamViewport, F64 -> CamViewport
	viewport_rebased = |v, delta|
		match v {
			Uninitialized => v
			Gesture(o) => Gesture(o + delta)
			Resting(r) => resting(r.rest, motion_rebased(r.motion, delta))
		}

	## The presenter's camera taken back: never over a target the layout set.
	presented : CamViewport, CamPresented, CamEnded -> CamViewport
	presented = |v, shown, ended|
		match v {
			Gesture(_) =>
				match shown {
					Gesture(o) => Gesture(o)
					Still(o) => at_rest(o)
					Moving(s) =>
						match ended {
							No => resting(At, Animation(s))
							Yes => at_rest(s.target)
						}
				}
			Resting(r) =>
				match shown {
					Still(o) => if motion_target(r.motion) == o resting(r.rest, Static(o)) else v
					Moving(s) =>
						if motion_target(r.motion) == s.target {
							match ended {
								No => resting(r.rest, Animation(s))
								Yes => resting(r.rest, Static(s.target))
							}
						} else {
							v
						}
					Gesture(_) => v
				}
			Uninitialized => v
		}

	# ---- motion history -----------------------------------------------------------------------

	## One delta at `time`; none when older than the last.
	pushed : CamHistory, F64, U64 -> [Pushed(CamHistory), Refused]
	pushed = |h, delta, time| {
		stale =
			match h.samples.last() {
				Ok(last) => time < last.time
				Err(_) => False
			}
		if stale {
			Refused
		} else {
			idle =
				match h.samples.last() {
					Ok(last) => delta == 0.0 and last.delta == 0.0
					Err(_) => False
				}
			samples0 =
				if idle {
					h.samples.drop_last(1).append(CamSample.{ time, delta: 0.0 })
				} else {
					capped = if h.samples.len() == 64 h.samples.drop_first(1) else h.samples
					capped.append(CamSample.{ time, delta })
				}
			fresh_from = samples0.find_first_index(|s| !(time > s.time and time - s.time > 150000000))
			samples =
				match fresh_from {
					Ok(i) => samples0.drop_first(i)
					Err(_) => []
				}
			Pushed(CamHistory.{ samples, overscroll: h.overscroll })
		}
	}

	velocity_of : CamHistory -> F64
	velocity_of = |h|
		match (h.samples.first(), h.samples.last()) {
			(Ok(first), Ok(last)) => {
				dt = seconds_since(last.time, first.time)
				if dt > 0.0 h.samples.fold(0.0, |sum, s| sum + s.delta) / dt else 0.0
			}
			_ => 0.0
		}

	clamp : F64, F64, F64 -> F64
	clamp = |x, low, high| if x < low low else if x > high high else x

	# ---- geometry and arrangement -------------------------------------------------------------

	span_of : CamSpans, U64 -> [Laid(CamSpan), NotLaid]
	span_of = |spans, id|
		match spans.columns.find_first(|c| c.id == id) {
			Ok(c) => Laid(c)
			Err(_) => NotLaid
		}

	clamped : CamSpans, F64 -> F64
	clamped = |spans, x| clamp(x, spans.low, spans.high)

	centered_offset : F64, CamSpan -> F64
	centered_offset = |view, c| {
		spare = view - c.width
		c.world_x - (if spare > 0.0 spare else 0.0) / 2.0
	}

	fit_offset : F64, F64, CamSpan -> F64
	fit_offset = |current, view, c| if c.width >= view c.world_x else clamp(current, c.world_x + c.width - view, c.world_x)

	anchor_offset : CamSpans, U64, CamSettings -> F64
	anchor_offset = |spans, index, settings|
		match spans.columns.get(index) {
			Ok(c) => {
				view = spans.view
				n = spans.columns.len()
				if c.width >= view {
					c.world_x
				} else if n > 1 and index == 0 {
					c.world_x
				} else if n > 1 and index + 1 == n {
					c.world_x + c.width - view
				} else {
					match settings.alignment {
						Left => c.world_x
						Center => centered_offset(view, c)
						Right => c.world_x + c.width - view
					}
				}
			}
			Err(_) => 0.0
		}

	active_id : CamFacts -> [Active(U64), NoColumn]
	active_id = |facts|
		match facts.columns.get(facts.active_column) {
			Ok(c) => Active(c.id)
			Err(_) => NoColumn
		}

	is_active : CamFacts, U64 -> Bool
	is_active = |facts, id|
		match active_id(facts) {
			Active(a) => a == id
			NoColumn => False
		}

	holds_column : CamFacts, U64 -> Bool
	holds_column = |facts, id| facts.columns.any(|c| c.id == id)

	column_of : CamFacts, U64 -> [Holds(U64), Nowhere]
	column_of = |facts, window|
		match facts.columns.find_first(|c| c.windows.contains(window)) {
			Ok(c) => Holds(c.id)
			Err(_) => Nowhere
		}

	active_column_fullscreen : CamFacts -> Bool
	active_column_fullscreen = |facts|
		match facts.columns.get(facts.active_column) {
			Ok(c) => c.windows.any(|w| facts.fullscreen.contains(w))
			Err(_) => False
		}

	active_span : CamSpans, CamFacts -> [Laid(CamSpan), NotLaid]
	active_span = |spans, facts|
		match active_id(facts) {
			Active(id) => span_of(spans, id)
			NoColumn => NotLaid
		}

	# ---- the camera ---------------------------------------------------------------------------

	with_viewport : CamCamera, CamViewport -> CamCamera
	with_viewport = |c, v| CamCamera.{ viewport: v, motion: c.motion, transient_restore: c.transient_restore, showing: c.showing, restored_view: c.restored_view }

	with_transient : CamCamera, MaybeBookmark -> CamCamera
	with_transient = |c, b| CamCamera.{ viewport: c.viewport, motion: c.motion, transient_restore: b, showing: c.showing, restored_view: c.restored_view }

	with_restored : CamCamera, MaybeBookmark -> CamCamera
	with_restored = |c, b| CamCamera.{ viewport: c.viewport, motion: c.motion, transient_restore: c.transient_restore, showing: c.showing, restored_view: b }

	with_showing : CamCamera, CamShowing -> CamCamera
	with_showing = |c, s| CamCamera.{ viewport: c.viewport, motion: c.motion, transient_restore: c.transient_restore, showing: s, restored_view: c.restored_view }

	with_motion : CamCamera, CamHistory -> CamCamera
	with_motion = |c, m| CamCamera.{ viewport: c.viewport, motion: m, transient_restore: c.transient_restore, showing: c.showing, restored_view: c.restored_view }

	bookmark : CamCamera, CamSpans, CamFacts -> MaybeBookmark
	bookmark = |c, spans, facts|
		match active_span(spans, facts) {
			Laid(col) => Bookmark(CamBookmark.{ column: col.id, relative_offset: offset_of(c.viewport) - col.world_x })
			NotLaid => NoBookmark
		}

	bookmark_in : CamCamera, MaybeSpans, CamFacts -> MaybeBookmark
	bookmark_in = |c, spans, facts|
		match spans {
			Spans(s) => bookmark(c, s, facts)
			NoSpans => NoBookmark
		}

	cloned : CamCamera, MaybeSpans -> CamCamera
	cloned = |c, spans|
		match c.viewport {
			Resting(r) =>
				match r.motion {
					Pending(_) => with_viewport(c, finished(c.viewport))
					Animation(_) => {
						o = offset_of(c.viewport)
						bounded =
							match spans {
								Spans(s) => clamped(s, o)
								NoSpans => o
							}
						with_viewport(c, at_rest(bounded))
					}
					Static(_) => c
				}
			_ => c
		}

	## The camera aimed back at a remembered view, if the strip still holds its column.
	restore : CamCamera, MaybeSpans, CamFacts, CamBookmark -> (CamCamera, [Restored(U64), NotRestored])
	restore = |c, spans, facts, b|
		if !holds_column(facts, b.column) {
			(c, NotRestored)
		} else {
			aimed_c =
				match spans {
					Spans(s) =>
						match span_of(s, b.column) {
							Laid(col) => with_viewport(c, set_target(c.viewport, fit_offset(col.world_x + b.relative_offset, s.view, col)))
							NotLaid => c
						}
					NoSpans => c
				}
			(aimed_c, Restored(b.column))
		}

	consume_return : CamCamera, MaybeSpans, CamFacts, CamBookmark -> (CamCamera, [Restored(U64), NotRestored])
	consume_return = |c, spans, facts, b| if active_column_fullscreen(facts) (c, NotRestored) else restore(c, spans, facts, b)

	reveal : CamCamera, CamSpans, CamFacts, CamSettings -> CamCamera
	reveal = |c, spans, facts, settings|
		match active_span(spans, facts) {
			Laid(col) => {
				to =
					match settings.navigation {
						Anchored =>
							match spans.columns.find_first_index(|x| x.id == col.id) {
								Ok(i) => anchor_offset(spans, i, settings)
								Err(_) => 0.0
							}
						Niri => fit_offset(target_of(c.viewport), spans.view, col)
					}
				if target_of(c.viewport) != to with_viewport(c, set_target(c.viewport, to)) else c
			}
			NotLaid => c
		}

	reveal_in : CamCamera, MaybeSpans, CamFacts, CamSettings -> CamCamera
	reveal_in = |c, spans, facts, settings|
		match spans {
			Spans(s) => reveal(c, s, facts, settings)
			NoSpans => c
		}

	centred_offset : CamSpans, CamFacts -> [Centre(F64), NoCentre]
	centred_offset = |spans, facts|
		match active_span(spans, facts) {
			Laid(col) => Centre(centered_offset(spans.view, col))
			NotLaid => NoCentre
		}

	## The return owed reconciled with the columns core, at the settle point.
	reconcile_return : CamCamera, CamSpans, CamFacts -> CamCamera
	reconcile_return = |c, spans, facts|
		match c.showing {
			Fullscreen(owed) =>
				if facts.fullscreen.contains(owed.window) {
					c
				} else {
					strip = with_showing(c, Strip)
					if is_active(facts, owed.return_to.column) {
						(after, _) = consume_return(strip, Spans(spans), facts, owed.return_to)
						after
					} else {
						strip
					}
				}
			Strip => c
		}

	settle_viewport : CamCamera, CamSpans, CamFacts, U64 -> CamCamera
	settle_viewport = |c0, spans, facts, now| {
		c = reconcile_return(c0, spans, facts)
		match c.viewport {
			Resting(r) => {
				bounded = clamped(spans, motion_target(r.motion))
				to =
					match r.rest {
						At => bounded
						Centred =>
							match centred_offset(spans, facts) {
								Centre(x) => x
								NoCentre => bounded
							}
					}
				with_viewport(c, resting(r.rest, move_target(r.motion, to, now)))
			}
			_ => c
		}
	}

	## The single settle point.
	set_geometry : CamCamera, CamSpans, CamFacts, MaybeBookmark, U64 -> CamCamera
	set_geometry = |c, spans, facts, hold, now| {
		(restored, dormant) =
			match c.restored_view {
				Bookmark(b) =>
					match span_of(spans, b.column) {
						Laid(_) => (Bookmark(b), NoBookmark)
						NotLaid => if holds_column(facts, b.column) (NoBookmark, Bookmark(b)) else (NoBookmark, NoBookmark)
					}
				NoBookmark => (NoBookmark, NoBookmark)
			}
		held =
			match restored {
				Bookmark(b) => Bookmark(b)
				NoBookmark => hold
			}
		rebased =
			match held {
				Bookmark(b) =>
					match span_of(spans, b.column) {
						Laid(col) => with_viewport(c, viewport_rebased(c.viewport, col.world_x + b.relative_offset - offset_of(c.viewport)))
						NotLaid => c
					}
				NoBookmark => c
			}
		settle_viewport(with_restored(rebased, dormant), spans, facts, now)
	}

	gesture_began : CamCamera, U64, Bool, CamFacts -> (CamCamera, Bool)
	gesture_began = |c, now, laid_out, facts| {
		available =
			laid_out
			and (
				match facts.selected {
					Some(w) => !facts.fullscreen.contains(w)
					Nothing => False
				}
			)
		if !available {
			(c, False)
		} else {
			c1 = with_transient(c, NoBookmark)
			match c1.viewport {
				Gesture(_) => (c1, False)
				_ => {
					sampled = viewport_sampled(c1.viewport, now, 1.0)
					(with_viewport(with_motion(c1, no_history({})), Gesture(offset_of(sampled))), True)
				}
			}
		}
	}

	## The camera settled on the resting point nearest `projected`.
	settle : CamCamera, CamSpans, CamFacts, CamSettings, F64, F64, Bool -> (CamCamera, [Settled(CamRelease), Unsettled])
	settle = |c, spans, facts, settings, projected, vel, gesture_release| {
		from_offset = offset_of(c.viewport)
		niri =
			match settings.navigation {
				Niri => True
				Anchored => False
			}
		away = |s| gesture_release and !is_active(facts, s.column)
		# Strictly before: the first of equals stays, as Rust's `min_by` keeps it.
		before = |a, b| {
			da = (a.offset - projected).abs()
			db = (b.offset - projected).abs()
			if da < db {
				True
			} else if da > db {
				False
			} else if niri {
				a.offset < b.offset
			} else {
				!away(a) and away(b)
			}
		}
		nearest = spans.snaps.fold(Unfound, |best, s| match best {
			Found(b) => if before(s, b) Found(s) else Found(b)
			Unfound => Found(s)
		})
		match nearest {
			Unfound => (c, Unsettled)
			Found(snap) => {
				column =
					if niri {
						if projected >= from_offset {
							after = spans.columns.drop_first(index_of(spans, snap.column) + 1)
							visible = after.keep_while(|n| n.world_x + n.width <= snap.offset + spans.view)
							match visible.last() {
								Ok(n) => n.id
								Err(_) => snap.column
							}
						} else {
							# Not laid out: Rust's reversed `skip_while` skips every column.
							place = index_of(spans, snap.column)
							before_cols = if place == spans.columns.len() [] else spans.columns.take_first(place).rev()
							visible = before_cols.keep_while(|n| n.world_x >= snap.offset)
							match visible.last() {
								Ok(n) => n.id
								Err(_) => snap.column
							}
						}
					} else {
						snap.column
					}
				if !holds_column(facts, column) {
					(c, Unsettled)
				} else {
					c1 = if is_active(facts, column) c else with_showing(c, Strip)
					c2 = with_transient(c1, NoBookmark)
					c3 = with_viewport(c2, set_target(c2.viewport, snap.offset))
					(c3, Settled(CamRelease.{ column, offset: snap.offset, from_offset, velocity: vel }))
				}
			}
		}
	}

	## The place of `id` among the laid-out columns (past the end when it is not laid out: Rust's
	## `skip_while` then skips them all).
	index_of : CamSpans, U64 -> U64
	index_of = |spans, id|
		match spans.columns.find_first_index(|c| c.id == id) {
			Ok(i) => i
			Err(_) => spans.columns.len()
		}

	settled_answer : (CamCamera, [Settled(CamRelease), Unsettled]) -> (CamCamera, CamAnswer)
	settled_answer = |(c, s)|
		match s {
			Settled(release) => (c, Released(release))
			Unsettled => (c, Nothing)
		}

	camera_step : CamCamera, CameraEvent -> (CamCamera, CamAnswer)
	camera_step = |c, event|
		match event {
			Prepared(p) => {
				c1 =
					match c.restored_view {
						NoBookmark => with_restored(c, bookmark_in(c, p.before, p.facts))
						Bookmark(_) => c
					}
				initial =
					match c1.viewport {
						Uninitialized => True
						_ => False
					}
				c2 = set_geometry(c1, p.spans, p.facts, NoBookmark, p.now)
				if initial {
					c3 = with_viewport(c2, at_rest(0.0))
					c4 =
						if p.spans.columns.len() == 1 {
							with_viewport(c3, set_target(c3.viewport, anchor_offset(p.spans, 0, p.settings)))
						} else {
							reveal(c3, p.spans, p.facts, p.settings)
						}
					(with_viewport(c4, finished(c4.viewport)), Nothing)
				} else {
					(c2, Nothing)
				}
			}
			Rebuilt(r) => {
				c1 =
					match r.spans {
						Spans(s) => {
							o = offset_of(c.viewport)
							hold =
								match r.hold {
									Hold(h) => {
										by_window =
											match h.window {
												Some(w) => column_of(r.facts, w)
												Nothing => Nowhere
											}
										anchor =
											match by_window {
												Holds(col) => Holds(col)
												Nowhere => if holds_column(r.facts, h.column) Holds(h.column) else Nowhere
											}
										match anchor {
											Holds(col) => Bookmark(CamBookmark.{ column: col, relative_offset: o - h.x })
											Nowhere => NoBookmark
										}
									}
									NoHold => NoBookmark
								}
							set_geometry(c, s, r.facts, hold, r.now)
						}
						NoSpans => c
					}
				if r.facts.columns.is_empty() (with_viewport(c1, at_rest(0.0)), Nothing) else (c1, Nothing)
			}
			Revealed(r) => (reveal_in(c, r.spans, r.facts, r.settings), Nothing)
			Centred(a) =>
				match a.spans {
					Spans(s) =>
						match centred_offset(s, a.facts) {
							Centre(x) => (with_viewport(c, aimed(c.viewport, x, Centred)), Nothing)
							NoCentre => (c, Nothing)
						}
					NoSpans => (c, Nothing)
				}
			Inserting(a) => (with_restored(with_transient(c, bookmark_in(c, a.spans, a.facts)), NoBookmark), Nothing)
			WindowRemoved(w) => {
				(restore_from, c1) =
					if w.active_removed and w.column_removed {
						(c.transient_restore, with_transient(c, NoBookmark))
					} else {
						(NoBookmark, c)
					}
				(c2, restored) =
					match restore_from {
						Bookmark(b) => if is_active(w.facts, b.column) restore(c1, w.spans, w.facts, b) else (c1, NotRestored)
						NoBookmark => (c1, NotRestored)
					}
				c3 =
					match restored {
						NotRestored => if w.active_removed reveal_in(c2, w.spans, w.facts, w.settings) else c2
						Restored(_) => c2
					}
				pruned =
					match c3.transient_restore {
						Bookmark(b) => if holds_column(w.facts, b.column) c3 else with_transient(c3, NoBookmark)
						NoBookmark => c3
					}
				(pruned, Nothing)
			}
			Selected(sel) => {
				c1 = if sel.column_changed with_viewport(c, uncentred(c.viewport)) else c
				c2 = with_transient(with_restored(c1, NoBookmark), NoBookmark)
				c3 =
					match c2.showing {
						Fullscreen(owed) => if is_active(sel.facts, owed.return_to.column) c2 else with_showing(c2, Strip)
						Strip => c2
					}
				(reveal_in(c3, sel.spans, sel.facts, sel.settings), Nothing)
			}
			Followed(f) => {
				(c1, restored) =
					match c.transient_restore {
						Bookmark(b) => {
							taken = with_transient(c, NoBookmark)
							if is_active(f.facts, b.column) restore(taken, f.spans, f.facts, b) else (taken, NotRestored)
						}
						NoBookmark => (c, NotRestored)
					}
				match restored {
					NotRestored => (reveal_in(c1, f.spans, f.facts, f.settings), Nothing)
					Restored(_) => (c1, Nothing)
				}
			}
			TransientForgotten => (with_transient(c, NoBookmark), Nothing)
			ViewForgotten => (with_restored(c, NoBookmark), Nothing)
			Replaced(pair) =>
				match c.showing {
					Fullscreen(owed) => if owed.window == pair.from (with_showing(c, Fullscreen(CamOwed.{ window: pair.to, return_to: owed.return_to })), Nothing) else (c, Nothing)
					Strip => (c, Nothing)
				}
			FullscreenLeft(l) =>
				match c.showing {
					Fullscreen(owed) =>
						if owed.window == l.window {
							(after, restored) = consume_return(with_showing(c, Strip), l.spans, l.facts, owed.return_to)
							match restored {
								Restored(col) => (after, Activate(col))
								NotRestored => (after, Nothing)
							}
						} else {
							(c, Nothing)
						}
					Strip => (c, Nothing)
				}
			FullscreenEntered(e) => {
				owed = bookmark_in(c, e.spans, e.before)
				c1 =
					match (c.showing, owed) {
						(Strip, Bookmark(b)) => with_showing(c, Fullscreen(CamOwed.{ window: e.window, return_to: b }))
						_ => c
					}
				c2 =
					match e.spans {
						Spans(s) =>
							match active_span(s, e.after) {
								Laid(col) => with_viewport(c1, set_target(c1.viewport, col.world_x))
								NotLaid => c1
							}
						NoSpans => c1
					}
				(c2, Nothing)
			}
			Finished => (with_viewport(c, finished(c.viewport)), Nothing)
			Presented(p) => (with_motion(with_viewport(c, presented(c.viewport, p.presented, p.ended)), p.motion), Nothing)
			ScrollBy(sc) =>
				if !sc.delta.is_finite() {
					(c, Nothing)
				} else {
					match sc.spans {
						Spans(s) =>
							match active_span(s, sc.facts) {
								Laid(col) => {
									step_w = col.width + sc.gap
									raw = offset_of(c.viewport) + sc.delta * step_w
									c1 = with_viewport(c, set_target(c.viewport, clamped(s, raw)))
									if raw < s.low {
										(c1, Excess(CamExcess.{ edge: Left, columns: (s.low - raw) / step_w }))
									} else if raw > s.high {
										(c1, Excess(CamExcess.{ edge: Right, columns: (raw - s.high) / step_w }))
									} else {
										(c1, Nothing)
									}
								}
								NotLaid => (c, Nothing)
							}
						NoSpans => (c, Nothing)
					}
				}
			GestureBegan(g) => {
				laid_out =
					match g.spans {
						Spans(_) => True
						NoSpans => False
					}
				(c1, began) = gesture_began(c, g.now, laid_out, g.facts)
				(c1, Began(began))
			}
			GestureMoved(m) =>
				if !m.delta.is_finite() {
					(c, Nothing)
				} else {
					match (c.viewport, m.spans) {
						(Gesture(o), Spans(s)) => {
							raw = o + c.motion.overscroll + m.delta
							bounded = clamp(raw, s.low, s.high)
							moved = bounded - o
							match pushed(c.motion, moved, m.at) {
								Pushed(h) => (with_viewport(with_motion(c, CamHistory.{ samples: h.samples, overscroll: raw - bounded }), Gesture(bounded)), Moved(moved))
								Refused => (c, Nothing)
							}
						}
						_ => (c, Nothing)
					}
				}
			GestureEnded(g) =>
				match c.viewport {
					Gesture(o) =>
						match pushed(c.motion, 0.0, g.at) {
							Refused => (c, Nothing)
							Pushed(h) => {
								c1 = with_motion(c, h)
								vel = velocity_of(h)
								projected = o - vel / (1000.0 * deceleration_ln)
								match g.spans {
									Spans(s) =>
										match settle(c1, s, g.facts, g.settings, projected, vel, True) {
											(c2, Settled(release)) =>
												if g.animate {
													(with_viewport(c2, resting(At, Animation(spring_new(release.from_offset, release.offset, release.velocity, g.started)))), Released(release))
												} else {
													(c2, Released(release))
												}
											(c2, Unsettled) => (c2, Nothing)
										}
									NoSpans => (c1, Nothing)
								}
							}
						}
					_ => (c, Nothing)
				}
			GestureCancelled(spans) =>
				if moving(c.viewport) {
					o = offset_of(c.viewport)
					bounded =
						match spans {
							Spans(s) => clamped(s, o)
							NoSpans => o
						}
					(with_motion(with_viewport(c, at_rest(bounded)), no_history({})), Nothing)
				} else {
					(c, Nothing)
				}
			Snapped(sn) =>
				match sn.spans {
					Spans(s) => settled_answer(settle(c, s, sn.facts, sn.settings, offset_of(c.viewport), 0.0, False))
					NoSpans => (c, Nothing)
				}
		}

	## Every layout's camera, in layout order.
	entries : Kept -> List(CamEntry)
	entries = |kept|
		kept.cameras
			.to_list()
			.sort_with(|(a, _), (b, _)| if a < b Before else if a > b After else Same)
			.map(|(layout, camera)| CamEntry.{ layout, camera })
}
