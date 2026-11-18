## A BSP layout's frames (`rift_core::layout::bsp`): every split divides its area in two along
## one axis, by a ratio, as far as its windows' constraints allow. `tile` takes the shape's
## words and returns the frames' words; it mirrors the Rust bit for bit, so the float helpers
## below are Rust's (`f64::round`, `max`, `min`, `sum`), not Roc's.

import Wire exposing [put_list, put_rect, put_window, take, take_flag, take_float, take_rect, take_u32, take_window]

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

Slot : [Empty, Window({ window : WindowId, constraints : [Constrained(WindowConstraints), Unconstrained] })]

Fill : [Tiled, Screen, WithinGaps]

Node := [
	Split({ horizontal : Bool, ratio : F64, children : [One(Node), Two(Node, Node)] }),
	Leaf({ slot : Slot, fill : Fill, stacked : Bool }),
]

Shape : { screen : Rect, gaps : Gaps, aspect : [Aspect(F64), Free], root : [Root(Node), Nothing] }

## One subtree's constraints along an axis.
Summary : { min : F64, fixed : [Fixed(F64), Free], max : [Max(F64), NoMax], can_grow : Bool }

# ---------------------------------------------------------------------------------------------
# Rust's float operations.

# ---------------------------------------------------------------------------------------------
# Constraints and gaps.

# ---------------------------------------------------------------------------------------------
# One axis of a container (`rift_core::layout::axis`).

AxisItem : { min : F64, fixed : [Fixed(F64), Free], max : [Max(F64), NoMax], weight : F64, can_grow : Bool }

# ---------------------------------------------------------------------------------------------
# The BSP layout.

# ---------------------------------------------------------------------------------------------
# The shape's words and the frames' words.

Bsp := [].{
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
					if (ceiling - x >= 0.5) {
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

	summary : Gaps, Node, Bool -> Summary
	summary = |gaps, node, horizontal|
		match node {
			Node.Leaf({ slot: Window({ constraints: Constrained(raw), .. }), .. }) => {
				c = normalized(raw)
				high = max_for_axis(c, horizontal)
				{
					min: min_for_axis(c, horizontal),
					fixed: fixed_for_axis(c, horizontal),
					max: if (high > 0.0) Max(high) else NoMax,
					can_grow: resizable_for_axis(c, horizontal),
				}
			}

			Node.Leaf({ slot: Window({ constraints: Unconstrained, .. }), .. }) => { min: 0.0, fixed: Free, max: NoMax, can_grow: True }
			Node.Leaf({ slot: Empty, .. }) => { min: 0.0, fixed: Free, max: NoMax, can_grow: True }
			Node.Split({ horizontal: split, children, .. }) => {
				summaries =
					match children {
						One(child) => [summary(gaps, child, horizontal)]
						Two(first, second) => [summary(gaps, first, horizontal), summary(gaps, second, horizontal)]
					}
				gap = if (horizontal) gaps.horizontal else gaps.vertical
				combined = container_summary(summaries, split == horizontal, False, gap, 0.0, 0.0)
				# BSP has always left the perpendicular maximum unconstrained.
				if (split != horizontal) { ..combined, max: NoMax } else combined
			}
		}

	## A tiled window's frame inside its area: as large as the area, within its constraints.
	fit : Rect, WindowConstraints -> Rect
	fit = |target, c| {
		or_else = |fixed, value|
			match fixed {
				Fixed(v) => v
				Free => value
			}
		desired_w0 = max(or_else(fixed_for_axis(c, True), target.w), min_for_axis(c, True))
		desired_h0 = max(or_else(fixed_for_axis(c, False), target.h), min_for_axis(c, False))
		desired_w = if (max_for_axis(c, True) > 0.0) min(desired_w0, max_for_axis(c, True)) else desired_w0
		desired_h = if (max_for_axis(c, False) > 0.0) min(desired_h0, max_for_axis(c, False)) else desired_h0
		{ ..target, w: max(min(desired_w, target.w), 0.0), h: max(min(desired_h, target.h), 0.0) }
	}

	place : Shape, Node, Rect, List((WindowId, Rect)) -> List((WindowId, Rect))
	place = |bsp, node, rect, out|
		match node {
			Node.Leaf({ slot: Empty, .. }) => out
			Node.Leaf({ slot: Window({ window, constraints }), fill, .. }) => {
				target =
					match fill {
						Screen => bsp.screen
						WithinGaps => tiling_area(bsp.screen, bsp.gaps)
						Tiled =>
							match constraints {
								Constrained(c) => fit(rect, normalized(c))
								Unconstrained => rect
							}
					}
				out.append((window, target))
			}

			Node.Split({ children: One(child), .. }) => place(bsp, child, rect, out)
			Node.Split({ horizontal, ratio, children: Two(first, second) }) => {
				(total, gap) = if (horizontal) (rect.w, bsp.gaps.horizontal) else (rect.h, bsp.gaps.vertical)
				available = max(total - gap, 0.0)
				axis = |child, weight| {
					s = summary(bsp.gaps, child, horizontal)
					{ min: s.min, fixed: s.fixed, max: s.max, weight: max(weight, 0.0), can_grow: s.can_grow }
				}
				solved = solve([axis(first, ratio), axis(second, 1.0 - ratio)], available)
				first_len = getf(solved, 0)
				second_len = getf(solved, 1)
				(r1, r2) =
					if horizontal {
						({ ..rect, w: first_len }, { ..rect, x: rect.x + (first_len + gap), w: second_len })
					} else {
						({ ..rect, h: first_len }, { ..rect, y: rect.y + (first_len + gap), h: second_len })
					}
				place(bsp, second, r2, place(bsp, first, r1, out))
			}
		}

	frames : Shape -> List((WindowId, Rect))
	frames = |bsp|
		match bsp.root {
			Nothing => []
			Root(root) => {
				area = tiling_area(bsp.screen, bsp.gaps)
				out = place(bsp, root, area, [])
				match (bsp.aspect, out, root) {
					(Aspect(ratio), [(window, frame)], Node.Leaf({ fill: Tiled, stacked: False, .. })) => {
						(w, h) = if (frame.w > frame.h * ratio) (frame.h * ratio, frame.h) else (frame.w, frame.w / ratio)
						[(window, { x: area.x + (area.w - w) / 2.0, y: area.y + (area.h - h) / 2.0, w, h })]
					}

					_ => out
				}
			}
		}

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

	take_node : Words -> (Node, Words)
	take_node = |c0| {
		(tag, c1) = take(c0)
		match tag {
			0 => {
				(horizontal, c2) = take_flag(c1)
				(bits, c3) = take_u32(c2)
				ratio = F32.to_f64(F32.from_bits(bits))
				(children_tag, c4) = take(c3)
				match children_tag {
					0 => {
						(child, c5) = take_node(c4)
						(Node.Split({ horizontal, ratio, children: One(child) }), c5)
					}

					1 => {
						(first, c5) = take_node(c4)
						(second, c6) = take_node(c5)
						(Node.Split({ horizontal, ratio, children: Two(first, second) }), c6)
					}

					_ => crash "wire: unknown split children"
				}
			}

			1 => {
				(slot_tag, c2) = take(c1)
				(slot, c3) =
					match slot_tag {
						0 => (Empty, c2)
						1 => {
							(window, after_window) = take_window(c2)
							(flag, after_flag) = take(after_window)
							match flag {
								0 => (Window({ window, constraints: Unconstrained }), after_flag)
								1 => {
									(c, after) = take_constraints(after_flag)
									(Window({ window, constraints: Constrained(c) }), after)
								}

								_ => crash "wire: unknown option"
							}
						}

						_ => crash "wire: unknown slot"
					}
				(fill_tag, c4) = take(c3)
				fill =
					match fill_tag {
						0 => Tiled
						1 => Screen
						2 => WithinGaps
						_ => crash "wire: unknown fill"
					}
				(stacked, c5) = take_flag(c4)
				(Node.Leaf({ slot, fill, stacked }), c5)
			}

			_ => crash "wire: unknown node"
		}
	}

	take_bsp : Words -> Shape
	take_bsp = |c0| {
		(screen, c1) = take_rect(c0)
		(top, c2) = take_float(c1)
		(left, c3) = take_float(c2)
		(bottom, c4) = take_float(c3)
		(right, c5) = take_float(c4)
		(horizontal, c6) = take_float(c5)
		(vertical, c7) = take_float(c6)
		gaps = { top, left, bottom, right, horizontal, vertical }
		(aspect_flag, c8) = take(c7)
		(aspect, c9) =
			match aspect_flag {
				0 => (Free, c8)
				1 => {
					(ratio, after) = take_float(c8)
					(Aspect(ratio), after)
				}

				_ => crash "wire: unknown option"
			}
		(root_flag, c10) = take(c9)
		root =
			match root_flag {
				0 => Nothing
				1 => Root(take_node(c10).0)
				_ => crash "wire: unknown option"
			}
		{ screen, gaps, aspect, root }
	}

	## A BSP shape's words in, its frames' words out.
	tile : List(U64) -> List(U64)
	tile = |words|
		put_list([], frames(take_bsp({ words, at: 0 })), |out, (window, frame)| put_rect(put_window(out, window), frame))
}
