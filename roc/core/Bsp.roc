## A BSP layout's frames (`rift_core::layout::bsp`): every split divides its area in two along
## one axis, by a ratio, as far as its windows' constraints allow.

import Wire exposing [finished, take, take_flag, take_float, take_rect, take_u32, take_window]
import Geometry exposing [container_summary, fixed_for_axis, max, max_for_axis, min, normalized, reported_min, resizable_for_axis, solve, take_constraints, tiling_area]

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

Gaps : { top : F64, left : F64, bottom : F64, right : F64, horizontal : F64, vertical : F64 }

## One subtree's constraints along an axis.
Summary : { min : F64, fixed : [Fixed(F64), Free], max : [Max(F64), NoMax], can_grow : Bool }

AxisItem : { min : F64, fixed : [Fixed(F64), Free], max : [Max(F64), NoMax], weight : F64, can_grow : Bool }

Slot : [Empty, Window({ window : WindowId, constraints : [Constrained(WindowConstraints), Unconstrained] })]

Fill : [Tiled, Screen, WithinGaps]

Node := [
	Split({ horizontal : Bool, ratio : F64, children : [One(Node), Two(Node, Node)] }),
	Leaf({ slot : Slot, fill : Fill, stacked : Bool }),
]

Shape : { screen : Rect, gaps : Gaps, aspect : [Aspect(F64), Free], root : Node }

# Rust's float operations.

# Constraints and gaps.

# One axis of a container (`rift_core::layout::axis`).

# The BSP layout.

# The shape's words and the frames' words.

Bsp := [].{
	summary : Gaps, Node, Bool -> Summary
	summary = |gaps, node, horizontal|
		match node {
			Node.Leaf({ slot: Window({ constraints: Constrained(raw), .. }), .. }) => {
				c = normalized(raw)
				high = max_for_axis(c, horizontal)
				{
					min: reported_min(c, horizontal),
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
		desired_w0 = max(or_else(fixed_for_axis(c, True), target.w), reported_min(c, True))
		desired_h0 = max(or_else(fixed_for_axis(c, False), target.h), reported_min(c, False))
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
				# The first child starts where the split does; the second after the first and the
				# gap (as one sum, the way the Rust adds it).
				(_, placed) =
					List.map2([first, second], solve([axis(first, ratio), axis(second, 1.0 - ratio)], available), |child, length| (child, length)).fold(
						(Start, out),
						|(at, acc), (child, length)| {
							frame =
								match (at, horizontal) {
									(Start, True) => { ..rect, w: length }
									(Start, False) => { ..rect, h: length }
									(After(advance), True) => { ..rect, x: rect.x + advance, w: length }
									(After(advance), False) => { ..rect, y: rect.y + advance, h: length }
								}
							(After(length + gap), place(bsp, child, frame, acc))
						},
					)
				placed
			}
		}

	frames : Shape -> List((WindowId, Rect))
	frames = |bsp| {
		area = tiling_area(bsp.screen, bsp.gaps)
		out = place(bsp, bsp.root, area, [])
		# A layout's only window, tiled and not stacked, is fitted to the aspect.
		match (bsp.aspect, bsp.root) {
			(Aspect(ratio), Node.Leaf({ fill: Tiled, stacked: False, .. })) =>
				match out {
					[(window, frame)] => {
						(w, h) = if (frame.w > frame.h * ratio) (frame.h * ratio, frame.h) else (frame.w, frame.w / ratio)
						[(window, { x: area.x + (area.w - w) / 2.0, y: area.y + (area.h - h) / 2.0, w, h })]
					}

					[] | [_, _, ..] => out
				}

			(Aspect(_), Node.Leaf(_)) | (Aspect(_), Node.Split(_)) | (Free, _) => out
		}
	}

	take_node : Words -> Taken(Node)
	take_node = |c0| {
		(tag, c1) = take(c0)?
		match tag {
			0 => {
				(horizontal, c2) = take_flag(c1)?
				(bits, c3) = take_u32(c2)?
				ratio = F32.to_f64(F32.from_bits(bits))
				(children_tag, c4) = take(c3)?
				match children_tag {
					0 => {
						(child, c5) = take_node(c4)?
						Ok((Node.Split({ horizontal, ratio, children: One(child) }), c5))
					}

					1 => {
						(first, c5) = take_node(c4)?
						(second, c6) = take_node(c5)?
						Ok((Node.Split({ horizontal, ratio, children: Two(first, second) }), c6))
					}

					_ => Err(Malformed)
				}
			}

			1 => {
				(slot, c3) = take_slot(c1)?
				(fill, c4) = take_fill(c3)?
				(stacked, c5) = take_flag(c4)?
				Ok((Node.Leaf({ slot, fill, stacked }), c5))
			}

			_ => Err(Malformed)
		}
	}

	take_slot : Words -> Taken(Slot)
	take_slot = |c0| {
		(tag, c1) = take(c0)?
		match tag {
			0 => Ok((Empty, c1))
			1 => {
				(window, c2) = take_window(c1)?
				(constrained, c3) = take_flag(c2)?
				if constrained {
					(c, c4) = take_constraints(c3)?
					Ok((Window({ window, constraints: Constrained(c) }), c4))
				} else {
					Ok((Window({ window, constraints: Unconstrained }), c3))
				}
			}

			_ => Err(Malformed)
		}
	}

	take_fill : Words -> Taken(Fill)
	take_fill = |c0| {
		(tag, c1) = take(c0)?
		match tag {
			0 => Ok((Tiled, c1))
			1 => Ok((Screen, c1))
			2 => Ok((WithinGaps, c1))
			_ => Err(Malformed)
		}
	}

	take_bsp : Words -> Taken(Shape)
	take_bsp = |c0| {
		(screen, c1) = take_rect(c0)?
		(top, c2) = take_float(c1)?
		(left, c3) = take_float(c2)?
		(bottom, c4) = take_float(c3)?
		(right, c5) = take_float(c4)?
		(horizontal, c6) = take_float(c5)?
		(vertical, c7) = take_float(c6)?
		gaps = { top, left, bottom, right, horizontal, vertical }
		(has_aspect, c8) = take_flag(c7)?
		(aspect, c9) =
			if has_aspect {
				(ratio, after) = take_float(c8)?
				(Aspect(ratio), after)
			} else {
				(Free, c8)
			}
		(root, c10) = take_node(c9)?
		Ok(({ screen, gaps, aspect, root }, c10))
	}

	## The frames of the BSP shape `cursor` holds (after its system's tag), all of it.
	frames_of : Words -> Try(List((WindowId, Rect)), [Malformed])
	frames_of = |cursor| {
		(shape, rest) = take_bsp(cursor)?
		finished(rest)?
		Ok(frames(shape))
	}
}
