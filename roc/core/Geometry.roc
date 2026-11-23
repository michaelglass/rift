## Layout geometry every system shares (`rift_core::layout` and `layout::axis`), bit for bit
## with the Rust: the float helpers are Rust's (`f64::round`, `max`, `min`, `sum`), not Roc's.

WindowId : { pid : I32, idx : U32 }

Rect : { x : F64, y : F64, w : F64, h : F64 }

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

Gaps : { top : F64, left : F64, bottom : F64, right : F64, horizontal : F64, vertical : F64 }

## One subtree's constraints along an axis.
Summary : { min : F64, fixed : [Fixed(F64), Free], max : [Max(F64), NoMax], can_grow : Bool }

## One segment to solve, and what it is the length of.
AxisItem : { min : F64, fixed : [Fixed(F64), Free], max : [Max(F64), NoMax], weight : F64, can_grow : Bool }

## A container's summary being gathered, one child at a time.
Gathered : { count : U64, low : F64, fixed : [Fixed(F64), Free], high : [Max(F64), NoMax], can_grow : Bool, focused : F64 }

## A segment's bounds, sanitized.
Segment : { low : F64, high : [Max(F64), NoMax], weight : F64, grows : Bool, fixed : [Fixed(F64), Free] }

## A segment and how far solving it has got.
Placed : { segment : Segment, state : [Set(F64), Growing] }

Geometry := [].{
	epsilon : F64
	epsilon = F64.from_bits(4372995238176751616)

	## `f64::max` on arm64 (`fmaxnm`): a NaN loses, and +0 beats -0.
	max : F64, F64 -> F64
	max = |a, b|
		if a > b {
			a
		} else if b > a {
			b
		} else if F64.is_nan(a) {
			b
		} else if F64.is_nan(b) {
			a
		} else if a == 0.0 {
			a + b
		} else {
			a
		}

	## `f64::min` on arm64 (`fminnm`): a NaN loses, and -0 beats +0.
	min : F64, F64 -> F64
	min = |a, b|
		if a < b {
			a
		} else if b < a {
			b
		} else if F64.is_nan(a) {
			b
		} else if F64.is_nan(b) {
			a
		} else if a == 0.0 {
			-(-a + -b)
		} else {
			a
		}

	## `f64::round`: half away from zero, keeping the sign of a zero result.
	round : F64 -> F64
	round = |x|
		if x == 0.0 {
			# Either zero keeps its sign.
			x
		} else if x > 0.0 {
			match F64.floor_to_i64_try(x) {
				Ok(whole) => {
					floor = whole.to_f64()
					if (x - floor >= 0.5) floor + 1.0 else floor
				}

				Err(_) => x
			}
		} else {
			match F64.ceiling_to_i64_try(x) {
				Ok(whole) => {
					ceiling = whole.to_f64()
					if ceiling - x >= 0.5 {
						ceiling - 1.0
					} else if ceiling == 0.0 {
						# A negative number rounding to zero rounds to -0.
						-0.0
					} else {
						ceiling
					}
				}

				Err(_) => x
			}
		}

	## `Iterator::sum` on f64, which starts from -0.
	sum : List(F64) -> F64
	sum = |values| {
		var $total = -0.0
		for value in values {
			$total = $total + value
		}
		$total
	}

	sanitize : F64 -> F64
	sanitize = |v| if (F64.is_finite(v)) max(v, 0.0) else 0.0

	normalized : WindowConstraints -> WindowConstraints
	normalized = |c| {
		clean = |v| if (F64.is_finite(v)) max(v, 0.0) else 0.0
		low = reported_size(c.sizing)
		max_width0 = clean(c.max_width)
		max_height0 = clean(c.max_height)
		max_width = if (max_width0 > 0.0 and max_width0 < low.width) low.width else max_width0
		max_height = if (max_height0 > 0.0 and max_height0 < low.height) low.height else max_height0
		{
			sizing: c.sizing,
			locked_width: clean(c.locked_width),
			locked_height: clean(c.locked_height),
			max_width,
			max_height,
		}
	}

	## The minimum the window reports, advisory or not.
	reported_size : Sizing -> Size
	reported_size = |sizing|
		match sizing {
			Fixed(size) | Resizable(size) | ResizableAtLeastMin(size) => size
		}

	## The window's reported minimum on an axis, advisory or not.
	reported_min : WindowConstraints, Bool -> F64
	reported_min = |c, horizontal| {
		size = reported_size(c.sizing)
		if (horizontal) size.width else size.height
	}

	## The minimum traditional tiling holds a window to: a resizable window's advisory minimum
	## counts only under respect_min_size.
	tiled_min : WindowConstraints, Bool -> F64
	tiled_min = |c, horizontal|
		match c.sizing {
			Fixed(_) | ResizableAtLeastMin(_) => reported_min(c, horizontal)
			Resizable(_) => 0.0
		}

	max_for_axis : WindowConstraints, Bool -> F64
	max_for_axis = |c, horizontal| if (horizontal) c.max_width else c.max_height

	fixed_for_axis : WindowConstraints, Bool -> [Fixed(F64), Free]
	fixed_for_axis = |c, horizontal| {
		locked = if (horizontal) c.locked_width else c.locked_height
		low = reported_min(c, horizontal)
		high = max_for_axis(c, horizontal)
		if low > 0.0 and high > 0.0 and (low - high).abs() <= epsilon {
			Fixed(high)
		} else {
			match c.sizing {
				Fixed(_) => if (locked > 0.0) Fixed(locked) else Free
				Resizable(_) | ResizableAtLeastMin(_) => Free
			}
		}
	}

	resizable_for_axis : WindowConstraints, Bool -> Bool
	resizable_for_axis = |c, horizontal|
		match fixed_for_axis(c, horizontal) {
			Fixed(_) => False
			Free => True
		}

	round_rect : Rect -> Rect
	round_rect = |rect| {
		x0 = round(rect.x)
		y0 = round(rect.y)
		x1 = round(rect.x + rect.w)
		y1 = round(rect.y + rect.h)
		{ x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
	}

	tiling_area : Rect, Gaps -> Rect
	tiling_area = |screen, gaps|
		if gaps.top == 0.0 and gaps.left == 0.0 and gaps.bottom == 0.0 and gaps.right == 0.0 {
			screen
		} else {
			round_rect({
				x: screen.x + gaps.left,
				y: screen.y + gaps.top,
				w: max(screen.w - gaps.left - gaps.right, 0.0),
				h: max(screen.h - gaps.top - gaps.bottom, 0.0),
			})
		}

	## The children's summaries combined into the container's, along one axis.
	container_summary : List(Summary), Bool, Bool, F64, F64, F64 -> Summary
	container_summary = |children, aligned, stacked, gap, stack_offset, reserve|
		gathered(children.fold(gather_start, |acc, child| gather(acc, child, aligned, stacked)), aligned, stacked, gap, stack_offset, reserve)

	## Nothing gathered yet.
	gather_start : Gathered
	gather_start = { count: 0, low: 0.0, fixed: Fixed(0.0), high: Max(0.0), can_grow: False, focused: 0.0 }

	## One more child's summary into a container's.
	gather : Gathered, Summary, Bool, Bool -> Gathered
	gather = |acc, child, aligned, stacked| {
		combine = |a, b| if (aligned and !stacked) a + max(b, 0.0) else max(a, max(b, 0.0))
		{
			count: acc.count + 1,
			low: combine(acc.low, child.min),
			fixed: match (acc.fixed, child.fixed) {
				(Fixed(a), Fixed(b)) => Fixed(combine(a, b))
				(Fixed(_), Free) | (Free, Fixed(_)) | (Free, Free) => Free
			},
			high: match (acc.high, child.max) {
				(Max(a), Max(b)) => Max(combine(a, b))
				(Max(_), NoMax) | (NoMax, Max(_)) | (NoMax, NoMax) => NoMax
			},
			can_grow: acc.can_grow or child.can_grow,
			focused: max(
				acc.focused,
				match child.fixed {
					Fixed(v) => v
					Free => child.min
				},
			),
		}
	}

	## The container's summary from all its children's.
	gathered : Gathered, Bool, Bool, F64, F64, F64 -> Summary
	gathered = |acc, aligned, stacked, gap, stack_offset, reserve| {
		count = acc.count
		if count == 0 {
			{ min: 0.0, fixed: Free, max: NoMax, can_grow: True }
		} else {
			extra =
				if count == 1 {
					0.0
				} else if !aligned {
					reserve
				} else if stacked {
					max(stack_offset, 0.0) * (count - 1).to_f64() + reserve
				} else {
					gap * (count - 1).to_f64()
				}
			(low, fixed) = if (count > 1 and stacked and aligned) (acc.focused, Free) else (acc.low, acc.fixed)
			{
				min: low + extra,
				fixed: match fixed {
					Fixed(v) => Fixed(v + extra)
					Free => Free
				},
				max: match acc.high {
					Max(v) => Max(v + extra)
					NoMax => NoMax
				},
				can_grow: acc.can_grow,
			}
		}
	}

	## Segment lengths along one axis (see `solve_axis_lengths`: upstream's solver since #688, step
	## for step), in order. Every step walks the segments in order; none indexes them.
	solve : List(AxisItem), F64 -> List(F64)
	solve = |items, usable0| {
		usable = sanitize(usable0)
		segments =
			items.map(
				|item| {
					low = sanitize(item.min)
					high =
						match item.max {
							Max(v) => {
								sane = sanitize(v)
								if (sane > 0.0) Max(sane) else NoMax
							}

							NoMax => NoMax
						}
					fixed =
						match item.fixed {
							Fixed(v0) => {
								v = max(sanitize(v0), low)
								Fixed(
									match high {
										Max(h) => min(v, h)
										NoMax => v
									},
								)
							}

							Free => Free
						}
					{ low, high, weight: sanitize(item.weight), grows: item.can_grow, fixed }
				},
			)
		fixed_sum = sum(
			segments.join_map(
				|segment|
					match segment.fixed {
						Fixed(v) => [v]
						Free => []
					},
			),
		)
		if fixed_sum > usable {
			scale = usable / fixed_sum
			segments.map(
				|segment|
					match segment.fixed {
						Fixed(v) => v * scale
						Free => 0.0 * scale
					},
			)
		} else {
			min_sum = sum(
				segments.join_map(
					|segment|
						match segment.fixed {
							Free => [segment.low]
							Fixed(_) => []
						},
				),
			)
			remaining_for_mins = max(usable - fixed_sum, 0.0)
			scaled =
				if min_sum > remaining_for_mins {
					scale = remaining_for_mins / min_sum
					segments.map(
						|segment|
							match segment.fixed {
								Free => { ..segment, low: segment.low * scale }
								Fixed(_) => segment
							},
					)
				} else {
					segments
				}
			# Fixed lengths first, in order; then the minima of what cannot grow.
			(with_fixed, after_fixed) =
				scaled.fold(
					([], usable),
					|(out, remaining), segment|
						match segment.fixed {
							Fixed(v) => {
								assigned = min(v, remaining)
								(out.append({ segment, state: Set(assigned) }), max(remaining - assigned, 0.0))
							}

							Free => (out.append({ segment, state: Growing }), remaining)
						},
				)
			(settled, after_minima) =
				with_fixed.fold(
					([], after_fixed),
					|(out, remaining), placed|
						match (placed.state, placed.segment.fixed, placed.segment.grows) {
							(Growing, Free, False) => {
								assigned = min(placed.segment.low, remaining)
								(out.append({ ..placed, state: Set(assigned) }), max(remaining - assigned, 0.0))
							}

							(Growing, Free, True) | (Growing, Fixed(_), _) | (Set(_), _, _) => (out.append(placed), remaining)
						},
				)
			grow(settled, after_minima).map(
				|placed|
					match placed.state {
						Set(length) => length
						Growing => 0.0
					},
			)
		}
	}

	## `f64::clamp`: below `low` is `low`, then above `high` is `high` (a NaN stays NaN).
	clamp : F64, F64, F64 -> F64
	clamp = |x, low, high| {
		lifted = if (x < low) low else x
		if (lifted > high) high else lifted
	}

	infinity : F64
	infinity = F64.from_bits(9218868437227405312)

	## The growing segments share what remains by weight, clamped one side at a time: when the
	## bounded shares overshoot what remains, those a bound raises are set at it; when they fall
	## short, those a bound lowers are; until they fit, nothing remains, or nothing grows.
	grow : List(Placed), F64 -> List(Placed)
	grow = |placed0, remaining0| {
		var $placed = placed0
		var $remaining = remaining0
		var $going = True
		while $going and $remaining > epsilon and $placed.any(|p| is_growing(p.state)) {
			weights = $placed.join_map(|p| if (is_growing(p.state)) [p.segment.weight] else [])
			count = weights.len().to_f64()
			total_weight = sum(weights)
			remaining = $remaining
			proposed = |segment| if (total_weight == 0.0) remaining / count else remaining * (segment.weight / total_weight)
			bounded = |segment| {
				high =
					match segment.high {
						Max(h) => h
						NoMax => infinity
					}
				clamp(proposed(segment), segment.low, max(high, segment.low))
			}
			total = sum($placed.join_map(|p| if (is_growing(p.state)) [bounded(p.segment)] else []))
			tolerance = (remaining * epsilon) * count
			settle_all = |placed|
				placed.map(
					|p|
						match p.state {
							Growing => { ..p, state: Set(bounded(p.segment)) }
							Set(_) => p
						},
				)
			if (total - remaining).abs() <= tolerance {
				$placed = settle_all($placed)
				$going = False
			} else {
				clamps =
					$placed.map(
						|p|
							match p.state {
								Growing => {
									share = proposed(p.segment)
									bound = bounded(p.segment)
									if (total > remaining and bound > share) or (total < remaining and bound < share) ClampTo(bound) else Keep
								}

								Set(_) => Keep
							},
					)
				clamped = clamps.join_map(
					|clamp_|
						match clamp_ {
							ClampTo(length) => [length]
							Keep => []
						},
				)
				if clamped.is_empty() {
					$placed = settle_all($placed)
					$going = False
				} else {
					# Rust adds the assigned lengths to a +0, not `sum`'s -0.
					assigned = clamped.fold(0.0, |acc, length| acc + length)
					$remaining = max(remaining - assigned, 0.0)
					$placed = List.map2(
						$placed,
						clamps,
						|p, clamp_|
							match clamp_ {
								ClampTo(length) => { ..p, state: Set(length) }
								Keep => p
							},
					)
				}
			}
		}
		$placed
	}

	is_growing : [Set(F64), Growing] -> Bool
	is_growing = |state|
		match state {
			Growing => True
			Set(_) => False
		}
}
