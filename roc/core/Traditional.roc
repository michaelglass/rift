## A traditional (i3-style) layout's frames (`rift_core::layout::traditional`): containers lay
## their children out along an axis by weight, or stack them with an offset (one shown on top),
## as far as the windows' constraints allow. Stack and master-stack layouts are traditional too.

import Wire exposing [take, take_flag, take_float, take_list, take_rect, take_u32, take_window]
import Geometry exposing [container_summary, fixed_for_axis, max, max_for_axis, min, min_for_axis, normalized, round_rect, solve, take_constraints, tiling_area]

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

Fill : [Tiled, Screen, WithinGaps]

## Along an axis by weight, or a stack with the child at `focused` on top.
Kind : [Axis(Bool), Stack({ horizontal : Bool, focused : U64 })]

## The bar a stack reserves room for (none when `thickness` is 0); `top`: a horizontal stack's
## bar is above it, `left`: a vertical stack's is left of it.
StackLine : { thickness : F64, spacing : F64, top : Bool, left : Bool }

Node := [
	Window({ window : WindowId, constraints : [Constrained(WindowConstraints), Unconstrained], fill : Fill }),

	## Never without children: a container with nothing that takes space is pruned.
	Container({ kind : Kind, fill : Fill, children : List({ size : F32, node : Node }) }),
]

Shape : { screen : Rect, area : Rect, gaps : Gaps, stack_offset : F64, stack_line : StackLine, root : [Root(Node), Nothing] }

Traditional := [].{

	## `f32::max` on arm64 (`fmaxnm`): a NaN loses, and +0 beats -0.
	max32 : F32, F32 -> F32
	max32 = |a, b|
		if a > b {
			a
		} else if b > a {
			b
		} else if F32.is_nan(a) {
			b
		} else if F32.is_nan(b) {
			a
		} else if a == 0.0 {
			a + b
		} else {
			a
		}

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

	## A leaf's constraints along one axis: a resizable window's bounds are not honoured when
	## splitting, only a fixed length is.
	leaf_summary : WindowConstraints, Bool -> Summary
	leaf_summary = |raw, horizontal| {
		c = normalized(raw)
		low = min_for_axis(c, horizontal)
		high = if (max_for_axis(c, horizontal) > 0.0) Max(max_for_axis(c, horizontal)) else NoMax
		if c.is_resizable {
			match fixed_for_axis(c, horizontal) {
				Fixed(fixed) => { min: low, fixed: Fixed(fixed), max: Max(fixed), can_grow: False }
				Free => { min: 0.0, fixed: Free, max: high, can_grow: True }
			}
		} else {
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

	summary : Shape, Node, Bool -> Summary
	summary = |shape, node, horizontal|
		match node {
			Node.Window({ constraints: Constrained(c), .. }) => leaf_summary(c, horizontal)
			Node.Window({ constraints: Unconstrained, .. }) => { min: 0.0, fixed: Free, max: NoMax, can_grow: True }
			Node.Container({ kind, children, .. }) => {
				(along, stacked, reserve) =
					match kind {
						Axis(along_axis) => (along_axis, False, 0.0)
						Stack({ horizontal: along_stack, .. }) => (along_stack, True, on_axis(shape.stack_line, along_stack, horizontal))
					}
				gap = if (horizontal) shape.gaps.horizontal else shape.gaps.vertical
				container_summary(
					children.map(|child| summary(shape, child.node, horizontal)),
					along == horizontal,
					stacked,
					gap,
					shape.stack_offset,
					reserve,
				)
			}
		}

	filled : Shape, Fill, Rect -> Rect
	filled = |shape, fill, rect|
		match fill {
			Tiled => rect
			Screen => shape.screen
			WithinGaps => tiling_area(shape.screen, shape.gaps)
		}

	## The frames of a stack of `count` windows in `rect`, each `offset` further along than the
	## one before (leaving room for the bar when there is more than one).
	stack_frames : Rect, U64, F64, Bool, StackLine -> List(Rect)
	stack_frames = |rect, count, offset, horizontal, line| {
		container = if (count > 1) inset(line, rect, horizontal) else rect
		spread = (if (count > 0) count - 1 else 0).to_f64() * offset
		(window_w, window_h) =
			if horizontal {
				(max(container.w - spread, 100.0), max(container.h, 100.0))
			} else {
				(max(container.w, 100.0), max(container.h - spread, 100.0))
			}
		var $frames = []
		var $index = 0
		while $index < count {
			shift = $index.to_f64() * offset
			(dx, dy) = if (horizontal) (shift, 0.0) else (0.0, shift)
			w = min(window_w, container.w)
			h = min(window_h, container.h)
			max_x = max(container.x + container.w - w, container.x)
			max_y = max(container.y + container.h - h, container.y)
			$frames = $frames.append(
				round_rect({
					x: clamp(container.x + dx, container.x, max_x),
					y: clamp(container.y + dy, container.y, max_y),
					w,
					h,
				}),
			)
			$index = $index + 1
		}
		$frames
	}

	place : Shape, Node, Rect, List((WindowId, Rect)) -> List((WindowId, Rect))
	place = |shape, node, rect0, out|
		match node {
			Node.Window({ window, constraints, fill }) => {
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

			Node.Container({ kind: Stack({ horizontal, focused }), fill, children }) =>
				stack(shape, children, horizontal, focused, filled(shape, fill, rect0), out)

			Node.Container({ kind: Axis(horizontal), fill, children }) =>
				axis(shape, children, horizontal, filled(shape, fill, rect0), out)
		}

	stack : Shape, List({ size : F32, node : Node }), Bool, U64, Rect, List((WindowId, Rect)) -> List((WindowId, Rect))
	stack = |shape, children, horizontal, focused, rect, out0| {
		count = children.len()
		axis_len = if (horizontal) rect.w else rect.h
		# The offset is capped so the window on top keeps its required length.
		offset =
			if count > 1 {
				on_top =
					match children.get(focused) {
						Ok(child) => child.node
						Err(_) => crash "traditional: a stack's focused child is one of its children"
					}
				s = summary(shape, on_top, horizontal)
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
		placed = stack_frames(rect, count, offset, horizontal, shape.stack_line)
		var $out = out0
		var $index = 0
		for child in children {
			frame =
				match placed.get($index) {
					Ok(f) => f
					Err(_) => crash "traditional: a stack has a frame per child"
				}
			$out = place(shape, child.node, frame, $out)
			$index = $index + 1
		}
		$out
	}

	axis : Shape, List({ size : F32, node : Node }), Bool, Rect, List((WindowId, Rect)) -> List((WindowId, Rect))
	axis = |shape, children, horizontal, rect, out0| {
		f32_epsilon = F32.from_bits(872415232)
		min_size = F32.from_bits(1028443341)
		var $total = 0.0
		var $needs_normalization = False
		for child in children {
			$total = $total + child.size
			if !F32.is_finite(child.size) or child.size < min_size - f32_epsilon {
				$needs_normalization = True
			}
		}
		total : F32
		total = $total
		normalize = !F32.is_finite(total) or total <= f32_epsilon or $needs_normalization
		gap = if (horizontal) shape.gaps.horizontal else shape.gaps.vertical
		axis_len = if (horizontal) rect.w else rect.h
		total_gap = (children.len() - 1).to_f64() * gap
		usable = if (gap == 0.0) axis_len else max(axis_len - total_gap, 0.0)
		items =
			children.map(
				|child| {
					s = summary(shape, child.node, horizontal)
					weight = if (normalize) 1.0 else F32.to_f64(max32(child.size, 0.0))
					{ min: s.min, fixed: s.fixed, max: s.max, weight, can_grow: s.can_grow }
				},
			)
		lengths = solve(items, usable)
		var $out = out0
		var $offset = if (horizontal) rect.x else rect.y
		var $index = 0
		last = children.len() - 1
		for child in children {
			length =
				match lengths.get($index) {
					Ok(l) => l
					Err(_) => crash "traditional: the solver gives a length per child"
				}
			frame = round_rect(if (horizontal) { ..rect, x: $offset, w: length } else { ..rect, y: $offset, h: length })
			$out = place(shape, child.node, frame, $out)
			$offset = $offset + length
			if $index < last {
				$offset = $offset + gap
			}
			$index = $index + 1
		}
		$out
	}

	frames : Shape -> List((WindowId, Rect))
	frames = |shape|
		match shape.root {
			Nothing => []
			Root(root) => place(shape, root, shape.area, [])
		}

	take_fill : Words -> (Fill, Words)
	take_fill = |cursor| {
		(word, rest) = take(cursor)
		match word {
			0 => (Tiled, rest)
			1 => (Screen, rest)
			2 => (WithinGaps, rest)
			_ => crash "wire: unknown fill"
		}
	}

	take_node : Words -> (Node, Words)
	take_node = |c0| {
		(tag, c1) = take(c0)
		match tag {
			0 => {
				(window, c2) = take_window(c1)
				(flag, c3) = take(c2)
				(constraints, c4) =
					match flag {
						0 => (Unconstrained, c3)
						1 => {
							(c, after) = take_constraints(c3)
							(Constrained(c), after)
						}

						_ => crash "wire: unknown option"
					}
				(fill, c5) = take_fill(c4)
				(Node.Window({ window, constraints, fill }), c5)
			}

			1 => {
				(kind_tag, c2) = take(c1)
				(kind, c3) =
					match kind_tag {
						0 => {
							(horizontal, after) = take_flag(c2)
							(Axis(horizontal), after)
						}

						1 => {
							(horizontal, after_flag) = take_flag(c2)
							(focused, after) = take(after_flag)
							(Stack({ horizontal, focused }), after)
						}

						_ => crash "wire: unknown container kind"
					}
				(fill, c4) = take_fill(c3)
				(children, c5) =
					take_list(
						c4,
						|cursor| {
							(bits, after_size) = take_u32(cursor)
							(child, after) = take_node(after_size)
							({ size: F32.from_bits(bits), node: child }, after)
						},
					)
				(Node.Container({ kind, fill, children }), c5)
			}

			_ => crash "wire: unknown node"
		}
	}

	take_shape : Words -> Shape
	take_shape = |c0| {
		(screen, c1) = take_rect(c0)
		(area, c2) = take_rect(c1)
		(top, c3) = take_float(c2)
		(left, c4) = take_float(c3)
		(bottom, c5) = take_float(c4)
		(right, c6) = take_float(c5)
		(horizontal, c7) = take_float(c6)
		(vertical, c8) = take_float(c7)
		(stack_offset, c9) = take_float(c8)
		(thickness, c10) = take_float(c9)
		(spacing, c11) = take_float(c10)
		(line_top, c12) = take_flag(c11)
		(line_left, c13) = take_flag(c12)
		(root_flag, c14) = take(c13)
		root =
			match root_flag {
				0 => Nothing
				1 => Root(take_node(c14).0)
				_ => crash "wire: unknown option"
			}
		{
			screen,
			area,
			gaps: { top, left, bottom, right, horizontal, vertical },
			stack_offset,
			stack_line: { thickness, spacing, top: line_top, left: line_left },
			root,
		}
	}

	## The frames of the traditional shape `cursor` holds (after its system's tag).
	frames_of : Words -> List((WindowId, Rect))
	frames_of = |cursor| frames(take_shape(cursor))
}
