## Layout geometry every system shares (`rift_core::layout` and `layout::axis`), bit for bit
## with the Rust: the float helpers are Rust's (`f64::round`, `max`, `min`, `sum`), not Roc's.

import Wire exposing [take_flag, take_float]

WindowId : { pid : I32, idx : U32 }

Rect : { x : F64, y : F64, w : F64, h : F64 }

## Words being read, and where.
Words : { words : List(U64), at : U64 }

WindowConstraints : {
	is_resizable : Bool,
	locked_width : F64,
	locked_height : F64,
	min_width : F64,
	min_height : F64,
	max_width : F64,
	max_height : F64,
}

Gaps : { top : F64, left : F64, bottom : F64, right : F64, horizontal : F64, vertical : F64 }

## One subtree's constraints along an axis.
Summary : { min : F64, fixed : [Fixed(F64), Free], max : [Max(F64), NoMax], can_grow : Bool }

AxisItem : { min : F64, fixed : [Fixed(F64), Free], max : [Max(F64), NoMax], weight : F64, can_grow : Bool }

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
		min_width = clean(c.min_width)
		min_height = clean(c.min_height)
		max_width0 = clean(c.max_width)
		max_height0 = clean(c.max_height)
		max_width = if (max_width0 > 0.0 and max_width0 < min_width) min_width else max_width0
		max_height = if (max_height0 > 0.0 and max_height0 < min_height) min_height else max_height0
		{
			is_resizable: c.is_resizable,
			locked_width: clean(c.locked_width),
			locked_height: clean(c.locked_height),
			min_width,
			min_height,
			max_width,
			max_height,
		}
	}

	min_for_axis : WindowConstraints, Bool -> F64
	min_for_axis = |c, horizontal| if (horizontal) c.min_width else c.min_height

	max_for_axis : WindowConstraints, Bool -> F64
	max_for_axis = |c, horizontal| if (horizontal) c.max_width else c.max_height

	fixed_for_axis : WindowConstraints, Bool -> [Fixed(F64), Free]
	fixed_for_axis = |c, horizontal| {
		locked = if (horizontal) c.locked_width else c.locked_height
		low = min_for_axis(c, horizontal)
		high = max_for_axis(c, horizontal)
		if low > 0.0 and high > 0.0 and (low - high).abs() <= epsilon {
			Fixed(high)
		} else if !c.is_resizable {
			if (locked > 0.0) Fixed(locked) else Free
		} else {
			Free
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
	container_summary = |children, aligned, stacked, gap, stack_offset, reserve| {
		combine = |a, b| if (aligned and !stacked) a + max(b, 0.0) else max(a, max(b, 0.0))
		var $low = 0.0
		var $fixed = Fixed(0.0)
		var $high = Max(0.0)
		var $can_grow = False
		var $focused = 0.0
		for child in children {
			$low = combine($low, child.min)
			$fixed =
				match ($fixed, child.fixed) {
					(Fixed(a), Fixed(b)) => Fixed(combine(a, b))
					_ => Free
				}
			$high =
				match ($high, child.max) {
					(Max(a), Max(b)) => Max(combine(a, b))
					_ => NoMax
				}
			$can_grow = $can_grow or child.can_grow
			$focused =
				max(
					$focused,
					match child.fixed {
						Fixed(v) => v
						Free => child.min
					},
				)
		}
		count = children.len()
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
			(low, fixed) = if (count > 1 and stacked and aligned) ($focused, Free) else ($low, $fixed)
			{
				min: low + extra,
				fixed: match fixed {
					Fixed(v) => Fixed(v + extra)
					Free => Free
				},
				max: match $high {
					Max(v) => Max(v + extra)
					NoMax => NoMax
				},
				can_grow: $can_grow,
			}
		}
	}

	## Segment lengths along one axis (see `solve_axis_lengths`).
	solve : List(AxisItem), F64 -> List(F64)
	solve = |items, usable0|
		if items.is_empty() {
			[]
		} else {
			usable = sanitize(usable0)
			n = items.len()
			mins0 = items.map(|i| sanitize(i.min))
			maxs =
				items.map(
					|i|
						match i.max {
							Max(v) => {
								s = sanitize(v)
								if (s > 0.0 and F64.is_finite(s)) Max(s) else NoMax
							}

							NoMax => NoMax
						},
				)
			weights = items.map(|i| sanitize(i.weight))
			fixed =
				indices_to(n).map(
					|idx| {
						item = get(items, idx)
						match item.fixed {
							Fixed(v0) => {
								v1 = sanitize(v0)
								if F64.is_finite(v1) {
									low = getf(mins0, idx)
									v2 = if (v1 < low) low else v1
									v3 =
										match get(maxs, idx) {
											Max(high) => if (v2 > high) high else v2
											NoMax => v2
										}
									Fixed(v3)
								} else {
									Free
								}
							}

							Free => Free
						}
					},
				)
			fixed_value = |f|
				match f {
					Fixed(v) => [v]
					Free => []
				}
			fixed_sum = sum(fixed.join_map(fixed_value))
			if fixed_sum > usable and fixed_sum > 0.0 {
				scale = usable / fixed_sum
				fixed.map(
					|f|
						match f {
							Fixed(v) => v * scale
							Free => 0.0
						},
				)
			} else {
				is_free = |idx|
					match get(fixed, idx) {
						Free => True
						Fixed(_) => False
					}
				indices = indices_to(n)
				min_sum = sum(indices.keep_if(is_free).map(|idx| getf(mins0, idx)))
				remaining_for_mins = max(usable - fixed_sum, 0.0)
				mins =
					if min_sum > remaining_for_mins and min_sum > 0.0 {
						scale = remaining_for_mins / min_sum
						indices.map(|idx| if (is_free(idx)) getf(mins0, idx) * scale else getf(mins0, idx))
					} else {
						mins0
					}
				var $lengths = List.repeat(0.0, n)
				var $remaining = usable
				for idx in indices {
					match get(fixed, idx) {
						Fixed(v) => {
							assigned = min(v, $remaining)
							$lengths = put_at($lengths, idx, assigned)
							$remaining = max($remaining - assigned, 0.0)
						}

						Free => {}
					}
				}
				var $growable = []
				for idx in indices {
					if is_free(idx) {
						if get(items, idx).can_grow {
							$growable = $growable.append(idx)
						} else {
							assigned = min(getf(mins, idx), $remaining)
							$lengths = put_at($lengths, idx, assigned)
							$remaining = max($remaining - assigned, 0.0)
						}
					}
				}
				var $going = True
				while $going and !$growable.is_empty() and $remaining > epsilon {
					total_weight = sum($growable.map(|idx| getf(weights, idx)))
					equal_weights = total_weight <= epsilon
					divisor = if (equal_weights) $growable.len().to_f64() else total_weight
					weight_of = |idx| if (equal_weights) 1.0 else getf(weights, idx)
					var $clamped = []
					for idx in $growable {
						proposed = $remaining * weight_of(idx) / divisor
						low = getf(mins, idx)
						if proposed + epsilon < low {
							$lengths = put_at($lengths, idx, min(low, $remaining))
							$clamped = $clamped.append(idx)
						} else {
							match get(maxs, idx) {
								Max(high0) => {
									high = max(high0, low)
									if proposed > high + epsilon {
										$lengths = put_at($lengths, idx, min(high, $remaining))
										$clamped = $clamped.append(idx)
									}
								}

								NoMax => {}
							}
						}
					}
					if $clamped.is_empty() {
						for idx in $growable {
							$lengths = put_at($lengths, idx, $remaining * weight_of(idx) / divisor)
						}
						$remaining = 0.0
						$going = False
					} else {
						clamped_sum = sum($clamped.map(|idx| getf($lengths, idx)))
						$remaining = max($remaining - clamped_sum, 0.0)
						clamped = $clamped
						$growable = $growable.keep_if(|idx| !clamped.contains(idx))
					}
				}
				if $remaining <= epsilon {
					drift = usable - sum($lengths)
					if drift.abs() > epsilon {
						lengths = $lengths
						match indices.keep_if(|idx| getf(lengths, idx) > 0.0).last() {
							Ok(idx) => put_at($lengths, idx, max(getf(lengths, idx) + drift, 0.0))
							Err(_) => $lengths
						}
					} else {
						$lengths
					}
				} else {
					$lengths
				}
			}
		}

	## 0, 1, …, n - 1.
	indices_to : U64 -> List(U64)
	indices_to = |n| {
		var $out = []
		var $i = 0
		while $i < n {
			$out = $out.append($i)
			$i = $i + 1
		}
		$out
	}

	## `list` with `value` at `idx`, an index within it.
	put_at : List(F64), U64, F64 -> List(F64)
	put_at = |list, idx, value| list.set(idx, value) ?? crash "solve: an index within its own list is out of bounds"

	get : List(a), U64 -> a
	get = |list, idx|
		match list.get(idx) {
			Ok(value) => value
			Err(_) => crash "solve: an index within its own list is out of bounds"
		}

	getf : List(F64), U64 -> F64
	getf = |list, idx| get(list, idx)

	take_constraints : Words -> (WindowConstraints, Words)
	take_constraints = |c0| {
		(is_resizable, c1) = take_flag(c0)
		(locked_width, c2) = take_float(c1)
		(locked_height, c3) = take_float(c2)
		(min_width, c4) = take_float(c3)
		(min_height, c5) = take_float(c4)
		(max_width, c6) = take_float(c5)
		(max_height, c7) = take_float(c6)
		({ is_resizable, locked_width, locked_height, min_width, min_height, max_width, max_height }, c7)
	}
}
