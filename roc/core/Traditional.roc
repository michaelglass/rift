## A traditional (i3-style) layout's frames (`rift_core::layout::traditional`): containers lay
## their children out along an axis by weight, or stack them with an offset (one shown on top),
## as far as the windows' constraints allow. Stack and master-stack layouts are traditional too.

import pf.Boundary exposing [TraditionalNode, TraditionalShape, StackLine, Child, Fill, WindowId, Rect, WindowConstraints, AxisItem, AxisFixed, AxisMax]
import Geometry exposing [container_summary, fixed_for_axis, gather, gather_start, gathered, max, max_for_axis, min, normalized, round_rect, solve, tiled_min, tiling_area]

## One subtree's constraints along an axis.
Summary : { min : F64, fixed : AxisFixed, max : AxisMax, can_grow : Bool }

## Along an axis by weight, or a stack with the child at `focused` on top.
## A stack's frames: its container (without the bar), each window's size, and the offset.
StackLayout : { container : Rect, w : F64, h : F64, offset : F64, horizontal : Bool }

## A traditional node is the platform's (`Boundary.TraditionalNode`).
Node : TraditionalNode

Shape : TraditionalShape

Traditional := [].{

	## `f64::clamp`.
	clamp : F64, F64, F64 -> F64
	clamp = |value, low, high| {
		raised = if (value < low) low else value
		if (raised > high) high else raised
	}

	line_total : StackLine -> F64
	line_total = |line| if (line.thickness > 0.0) line.thickness + line.spacing else 0.0

	## A horizontal stack gives up height for its bar, a vertical one width.
	on_axis : StackLine, Bool, Bool -> F64
	on_axis = |line, stack_horizontal, horizontal| if (stack_horizontal != horizontal) line_total(line) else 0.0

	## A stack's frame without its bar.
	inset : StackLine, Rect, Bool -> Rect
	inset = |line, rect, horizontal_stack|
		if horizontal_stack {
			reserve = min(line_total(line), rect.h)
			{ ..rect, y: if (line.top) rect.y + reserve else rect.y, h: rect.h - reserve }
		} else {
			reserve = min(line_total(line), rect.w)
			{ ..rect, x: if (line.left) rect.x + reserve else rect.x, w: rect.w - reserve }
		}

	## A leaf's constraints along one axis: a resizable window keeps a locked axis and, under
	## respect_min_size, its minimum; a fixed window keeps its locked length.
	leaf_summary : WindowConstraints, Bool -> Summary
	leaf_summary = |raw, horizontal| {
		c = normalized(raw)
		low = tiled_min(c, horizontal)
		high = if (max_for_axis(c, horizontal) > 0.0) Max(max_for_axis(c, horizontal)) else NoMax
		match c.sizing {
			Resizable(_) | ResizableAtLeastMin(_) =>
				match fixed_for_axis(c, horizontal) {
					Fixed(fixed) => { min: fixed, fixed: Fixed(fixed), max: Max(fixed), can_grow: False }
					Free => { min: low, fixed: Free, max: high, can_grow: True }
				}

			Fixed(_) => {
				fixed =
					match fixed_for_axis(c, horizontal) {
						Fixed(v) => if (v > 0.0) Fixed(v) else Free
						Free => Free
					}
				match fixed {
					Fixed(v) =>
						{
							min: low,
							fixed,
							max: match high {
								Max(m) => Max(m)
								NoMax => Max(v)
							},
							can_grow: False,
						}

					Free => { min: low, fixed, max: high, can_grow: True }
				}
			}
		}
	}

	summary : Shape, Node, Bool -> Summary
	summary = |shape, node, horizontal|
		match node {
			TraditionalNode.Window({ constraints: Constrained(c), .. }) => leaf_summary(c, horizontal)
			TraditionalNode.Window({ constraints: Unconstrained, .. }) => { min: 0.0, fixed: Free, max: NoMax, can_grow: True }
			TraditionalNode.Axis({ horizontal: along, first, rest, .. }) => {
				aligned = along == horizontal
				add = |acc, child| gather(acc, summary(shape, child.node, horizontal), aligned, False)
				acc = rest.fold(add(gather_start, first), add)
				gathered(acc, aligned, False, gap_along(shape, horizontal), shape.stack_offset, 0.0)
			}

			TraditionalNode.Stack({ horizontal: along, before, focused, after, .. }) => {
				aligned = along == horizontal
				add = |acc, child| gather(acc, summary(shape, child.node, horizontal), aligned, True)
				acc = after.fold(add(before.fold(gather_start, add), focused), add)
				gathered(acc, aligned, True, gap_along(shape, horizontal), shape.stack_offset, on_axis(shape.stack_line, along, horizontal))
			}
		}

	gap_along : Shape, Bool -> F64
	gap_along = |shape, horizontal| if (horizontal) shape.gaps.horizontal else shape.gaps.vertical

	filled : Shape, Fill, Rect -> Rect
	filled = |shape, fill, rect|
		match fill {
			Tiled => rect
			Screen => shape.screen
			WithinGaps => tiling_area(shape.screen, shape.gaps)
		}

	## How a stack of `count` windows in `rect` is laid out, each `offset` further along than the
	## one before (leaving room for the bar when there is more than one).
	stack_layout : Rect, U64, F64, Bool, StackLine -> StackLayout
	stack_layout = |rect, count, offset, horizontal, line| {
		container = if (count > 1) inset(line, rect, horizontal) else rect
		spread = (if (count > 0) count - 1 else 0).to_f64() * offset
		(window_w, window_h) =
			if horizontal {
				(max(container.w - spread, 100.0), max(container.h, 100.0))
			} else {
				(max(container.w, 100.0), max(container.h - spread, 100.0))
			}
		{ container, w: min(window_w, container.w), h: min(window_h, container.h), offset, horizontal }
	}

	## The frame of the stack's window at `position` (a formula of the position, not a lookup).
	stack_frame : StackLayout, U64 -> Rect
	stack_frame = |layout, position| {
		container = layout.container
		shift = position.to_f64() * layout.offset
		(dx, dy) = if (layout.horizontal) (shift, 0.0) else (0.0, shift)
		max_x = max(container.x + container.w - layout.w, container.x)
		max_y = max(container.y + container.h - layout.h, container.y)
		round_rect(Rect.{
			x: clamp(container.x + dx, container.x, max_x),
			y: clamp(container.y + dy, container.y, max_y),
			w: layout.w,
			h: layout.h,
		})
	}

	place : Shape, Node, Rect, List((WindowId, Rect)) -> List((WindowId, Rect))
	place = |shape, node, rect0, out|
		match node {
			TraditionalNode.Window({ window, constraints, fill }) => {
				rect = filled(shape, fill, rect0)
				frame =
					match constraints {
						Constrained(c) => {
							sw = leaf_summary(c, True)
							sh = leaf_summary(c, False)
							or_else = |fixed, value|
								match fixed {
									Fixed(v) => v
									Free => value
								}
							capped = |value, high|
								match high {
									Max(m) => min(value, m)
									NoMax => value
								}
							desired_w = capped(max(or_else(sw.fixed, rect.w), sw.min), sw.max)
							desired_h = capped(max(or_else(sh.fixed, rect.h), sh.min), sh.max)
							{ ..rect, w: max(min(desired_w, rect.w), 0.0), h: max(min(desired_h, rect.h), 0.0) }
						}

						Unconstrained => rect
					}
				out.append((window, frame))
			}

			TraditionalNode.Stack({ horizontal, fill, before, focused, after }) =>
				stack(shape, before, focused, after, horizontal, filled(shape, fill, rect0), out)

			TraditionalNode.Axis({ horizontal, fill, first, rest }) =>
				axis(shape, first, rest, horizontal, filled(shape, fill, rect0), out)
		}

	stack : Shape, List(Child), Child, List(Child), Bool, Rect, List((WindowId, Rect)) -> List((WindowId, Rect))
	stack = |shape, before, focused, after, horizontal, rect, out0| {
		count = before.len() + 1 + after.len()
		axis_len = if (horizontal) rect.w else rect.h
		# The offset is capped so the window on top keeps its required length.
		offset =
			if count > 1 {
				s = summary(shape, focused.node, horizontal)
				low =
					match s.fixed {
						Fixed(v) => v
						Free => s.min
					}
				desired = clamp(low, 0.0, max(axis_len, 0.0))
				max_offset = max(axis_len - desired, 0.0) / (count - 1).to_f64()
				min(shape.stack_offset, max_offset)
			} else {
				shape.stack_offset
			}
		layout = stack_layout(rect, count, offset, horizontal, shape.stack_line)
		# Behind the window on top, then it, then the rest, each at its position.
		add = |(position, out), child| (position + 1, place(shape, child.node, stack_frame(layout, position), out))
		(_, placed) = after.fold(add(before.fold((0, out0), add), focused), add)
		placed
	}

	axis : Shape, Child, List(Child), Bool, Rect, List((WindowId, Rect)) -> List((WindowId, Rect))
	axis = |shape, first, rest, horizontal, rect, out0| {
		children = [first].concat(rest)
		# Weights as upstream's `layout_axis` reads them (since #688): each as an F64, and equal
		# weights only when one is negative or not finite, or they add up to nothing.
		(total, invalid) =
			children.fold(
				(0.0, False),
				|(sum_, bad), child| {
					weight = F32.to_f64(child.size)
					(sum_ + weight, bad or !F64.is_finite(weight) or weight < 0.0)
				},
			)
		equal = invalid or !F64.is_finite(total) or total <= 0.0
		gap = if (horizontal) shape.gaps.horizontal else shape.gaps.vertical
		axis_len = if (horizontal) rect.w else rect.h
		total_gap = rest.len().to_f64() * gap
		usable = if (gap == 0.0) axis_len else max(axis_len - total_gap, 0.0)
		items =
			children.map(
				|child| {
					s = summary(shape, child.node, horizontal)
					weight = if (equal) 1.0 else F32.to_f64(child.size)
					AxisItem.{ min: s.min, fixed: s.fixed, max: s.max, weight, can_grow: s.can_grow }
				},
			)
		# Each child starts where the one before ended, after the gap; the first where the
		# container does.
		(_, placed) =
			List.map2(children, solve(items, usable), |child, length| (child.node, length)).fold(
				(Start, out0),
				|(at, out), (node, length)| {
					offset =
						match at {
							Start => if (horizontal) rect.x else rect.y
							After(end) => end + gap
						}
					frame = round_rect(if (horizontal) { ..rect, x: offset, w: length } else { ..rect, y: offset, h: length })
					(After(offset + length), place(shape, node, frame, out))
				},
			)
		placed
	}

	frames : Shape -> List((WindowId, Rect))
	frames = |shape|
		place(shape, shape.root, shape.area, [])
}
