## A floating layout's first frames (`rift_core::layout::floating`): where a window goes the
## first time the layout places it.

import Geometry exposing [fixed_for_axis, max, max_for_axis, min, normalized, reported_min, take_constraints]

WindowId : { pid : I32, idx : U32 }

Rect : { x : F64, y : F64, w : F64, h : F64 }

## Words being read, and where.
Words : { words : List(U64), at : U64 }

## A value read and where the rest starts, or the words were malformed.
Taken(a) : Try((a, Words), [Malformed])

## A size an app reported, as rift parsed it: each axis finite and not negative.
Size : { width : F64, height : F64 }

## How tiling may size a window, with the minimum it reports: kept at its own size; resizable,
## the minimum advisory (traditional tiling may shrink it below); resizable, never below it.
Sizing : [Fixed(Size), Resizable(Size), ResizableAtLeastMin(Size)]

WindowConstraints : {
	sizing : Sizing,
	locked_width : F64,
	locked_height : F64,
	max_width : F64,
	max_height : F64,
}

## A window placed for the first time: its constraints, and the frame it is seen at, if seen.
Newcomer : { constraints : WindowConstraints, seen : [Live(Rect), Unseen] }

Floating := [].{

	## The step between cascaded windows, in points on each axis.
	cascade_step : F64
	cascade_step = 36.0

	## A `w` by `h` frame centred on the screen (Rust's CGRect::mid: x + w * 0.5).
	centred : Rect, F64, F64 -> Rect
	centred = |screen, w, h| {
		mid_x = screen.x + screen.w * 0.5
		mid_y = screen.y + screen.h * 0.5
		{ x: mid_x - w / 2.0, y: mid_y - h / 2.0, w, h }
	}

	## A window's frame when nothing else is known: 70% of the screen on each axis, within its
	## limits (a locked axis keeps its length), centred.
	default_frame : Rect, WindowConstraints -> Rect
	default_frame = |screen, constraints| {
		limits = normalized(constraints)
		dimension = |value, horizontal|
			match fixed_for_axis(limits, horizontal) {
				Fixed(fixed) => fixed
				Free => {
					at_least = max(value, reported_min(limits, horizontal))
					high = max_for_axis(limits, horizontal)
					if (high > 0.0) min(at_least, high) else at_least
				}
			}
		centred(screen, dimension(screen.w * 0.7, True), dimension(screen.h * 0.7, False))
	}

	## Its live frame, no larger than the default (recentred if it had to shrink), or the default.
	initial_frame : Rect, Newcomer -> Rect
	initial_frame = |screen, window| {
		default = default_frame(screen, window.constraints)
		frame =
			match window.seen {
				Live(live) => live
				Unseen => default
			}
		w = min(frame.w, default.w)
		h = min(frame.h, default.h)
		# As Rust's (w, h) != (frame.w, frame.h): a NaN axis differs from itself.
		if w == frame.w and h == frame.h {
			frame
		} else {
			centred(screen, w, h)
		}
	}

	## `frame` moved `fraction` of the way along a diagonal of `slots` steps, centred on the
	## screen and no longer than the room the frame leaves.
	cascade : Rect, Rect, F64, F64 -> Rect
	cascade = |screen, frame, fraction, slots| {
		available_x = max(screen.w - frame.w, 0.0)
		available_y = max(screen.h - frame.h, 0.0)
		span_x = min(slots * cascade_step, available_x)
		span_y = min(slots * cascade_step, available_y)
		{
			x: screen.x + available_x / 2.0 - span_x / 2.0 + span_x * fraction,
			y: screen.y + available_y / 2.0 - span_y / 2.0 + span_y * fraction,
			w: frame.w,
			h: frame.h,
		}
	}

	## Each window's first frame, in order; cascaded when spreading. A window's place in the
	## cascade is its place in the list.
	first_frames : Rect, Bool, List(Newcomer) -> List(Rect)
	first_frames = |screen, spread, windows| {
		count = windows.len()
		slots = (if (count > 0) count - 1 else 0).to_f64()
		cascading = spread and count > 1
		windows.fold(
			{ position: 0.U64, out: [] },
			|acc, window| {
				frame = initial_frame(screen, window)
				placed = if (cascading) cascade(screen, frame, acc.position.to_f64() / slots, slots) else frame
				{ position: acc.position + 1, out: acc.out.append(placed) }
			},
		).out
	}
}
