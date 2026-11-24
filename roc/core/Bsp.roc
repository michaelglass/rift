## A BSP layout's frames (`rift_core::layout::bsp`): every split divides its area in two along
## one axis, by a ratio, as far as its windows' constraints allow.

import pf.Boundary exposing [BspNode, BspLeaf, BspShape, Gaps, WindowId, Rect, WindowConstraints, AxisItem, AxisFixed, AxisMax]
import Geometry exposing [container_summary, fixed_for_axis, max, max_for_axis, min, normalized, reported_min, resizable_for_axis, solve, tiling_area]

## One subtree's constraints along an axis.
Summary : { min : F64, fixed : AxisFixed, max : AxisMax, can_grow : Bool }

## A subtree's constraints along both axes: its parent asks along its own axis.
Along : { horizontal : Summary, vertical : Summary }

## A BSP node is the platform's (`Boundary.BspNode`).
Node : BspNode

Shape : BspShape

# Rust's float operations.

# Constraints and gaps.

# One axis of a container (`rift_core::layout::axis`).

# The BSP layout.

# The shape's words and the frames' words.

Bsp := [].{

	## A node with its subtree's constraints along both axes, made for one `frames` and dropped
	## with it (as the Rust core's `Summarized`).
	Summarized := { along : Along, node : Annotated }

	Annotated := [Leaf(BspLeaf), Split(SummarizedSplit)]

	SummarizedSplit := { horizontal : Bool, ratio : F32, children : SummarizedChildren }

	SummarizedChildren := [One(Summarized), Two(Summarized, Summarized)]

	## The constraints along one axis.
	on : Along, Bool -> Summary
	on = |along, horizontal| if (horizontal) along.horizontal else along.vertical

	## The constraints along each axis, as `summary_of` gives them.
	both : (Bool -> Summary) -> Along
	both = |summary_of| { horizontal: summary_of(True), vertical: summary_of(False) }

	## A leaf's constraints along one axis.
	leaf_summary : BspLeaf, Bool -> Summary
	leaf_summary = |leaf, horizontal|
		match leaf.slot {
			Window({ constraints: Constrained(raw), .. }) => {
				c = normalized(raw)
				high = max_for_axis(c, horizontal)
				{
					min: reported_min(c, horizontal),
					fixed: fixed_for_axis(c, horizontal),
					max: if (high > 0.0) Max(high) else NoMax,
					can_grow: resizable_for_axis(c, horizontal),
				}
			}

			Window({ constraints: Unconstrained, .. }) => { min: 0.0, fixed: Free, max: NoMax, can_grow: True }
			Empty => { min: 0.0, fixed: Free, max: NoMax, can_grow: True }
		}

	## A split's constraints along one axis, from its children's.
	split_summary : Gaps, Bool, List(Along), Bool -> Summary
	split_summary = |gaps, split, children, horizontal| {
		gap = if (horizontal) gaps.horizontal else gaps.vertical
		combined = container_summary(children.map(|along| on(along, horizontal)), split == horizontal, False, gap, 0.0, 0.0)
		# BSP has always left the perpendicular maximum unconstrained.
		if (split != horizontal) { ..combined, max: NoMax } else combined
	}

	## The tree with each subtree's constraints along both axes, folded once bottom-up (a split
	## asks each child's along its own axis; recomputing them at every level made a deep tree's
	## frames quadratic).
	summarized : Gaps, Node -> Summarized
	summarized = |gaps, node|
		match node {
			BspNode.Leaf(leaf) => Summarized.{ along: both(|horizontal| leaf_summary(leaf, horizontal)), node: Annotated.Leaf(leaf) }
			BspNode.Split({ horizontal: split, ratio, children }) => {
				(summarized_children, alongs) =
					match children {
						One(child) => {
							only = summarized(gaps, child)
							(SummarizedChildren.One(only), [only.along])
						}

						Two(first, second) => {
							(a, b) = (summarized(gaps, first), summarized(gaps, second))
							(SummarizedChildren.Two(a, b), [a.along, b.along])
						}
					}
				Summarized.{
					along: both(|horizontal| split_summary(gaps, split, alongs, horizontal)),
					node: Annotated.Split(SummarizedSplit.{ horizontal: split, ratio, children: summarized_children }),
				}
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

	place : Shape, Summarized, Rect, List((WindowId, Rect)) -> List((WindowId, Rect))
	place = |bsp, node, rect, out|
		match node.node {
			Annotated.Leaf({ slot: Empty, .. }) => out
			Annotated.Leaf({ slot: Window({ window, constraints }), fill, .. }) => {
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

			Annotated.Split({ children: One(child), .. }) => place(bsp, child, rect, out)
			Annotated.Split({ horizontal, ratio: share, children: Two(first, second) }) => {
				ratio = F32.to_f64(share)
				(total, gap) = if (horizontal) (rect.w, bsp.gaps.horizontal) else (rect.h, bsp.gaps.vertical)
				available = max(total - gap, 0.0)
				axis = |child, weight| {
					s = on(child.along, horizontal)
					AxisItem.{ min: s.min, fixed: s.fixed, max: s.max, weight: max(weight, 0.0), can_grow: s.can_grow }
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
		out = place(bsp, summarized(bsp.gaps, bsp.root), area, [])
		# A layout's only window, tiled and not stacked, is fitted to the aspect.
		match (bsp.aspect, bsp.root) {
			(Aspect(ratio), BspNode.Leaf({ fill: Tiled, stacked: False, .. })) =>
				match out {
					[(window, frame)] => {
						(w, h) = if (frame.w > frame.h * ratio) (frame.h * ratio, frame.h) else (frame.w, frame.w / ratio)
						[(window, Rect.{ x: area.x + (area.w - w) / 2.0, y: area.y + (area.h - h) / 2.0, w, h })]
					}

					[] | [_, _, ..] => out
				}

			(Aspect(_), BspNode.Leaf(_)) | (Aspect(_), BspNode.Split(_)) | (Free, _) => out
		}
	}
}
