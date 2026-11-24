## A scrolling layout's strip (`rift_core::layout::scrolling`): columns side by side in world
## space, each column's windows in rows sharing its height, and the camera's resting points.

import pf.Boundary exposing [WindowId, Rect, WindowConstraints, AxisItem, Widths, Row, Column, Strip, Spot, Snap, Placed, Settle, Settled, Frame, Parking, FullscreenMode]
import Geometry exposing [fixed_for_axis, max, min, normalized, reported_min, resizable_for_axis, round_rect, solve]

## A column's place among the strip's, which decides its anchor.
Position : [Only, First, Middle, Last]

## How the camera shows a placed strip (`rift_core::layout::scrolling::Camera`).
Camera : { screen : Rect, tiling : Rect, offset : F64, parking : Parking }

Scrolling := [].{

	## The smallest row weight (Rust's 0.001).
	least_weight : F64
	least_weight = F64.from_bits(4562254508917369340)

	proportional_width : F64, F64, F64 -> F64
	proportional_width = |view, gap, ratio| max((view + gap) * ratio - gap, 1.0)

	width_ratio : F64, F64, F64 -> F64
	width_ratio = |view, gap, width| (width + gap) / max(view + gap, 1.0)

	## `ratio` within the configured range (Rust's f64::clamp); a non-finite one is the smallest.
	clamp_ratio : Widths, F64 -> F64
	clamp_ratio = |widths, ratio| {
		low = max(widths.min_ratio, 0.05)
		high = max(widths.max_ratio, low)
		if F64.is_finite(ratio) {
			if ratio < low {
				low
			} else if ratio > high {
				high
			} else {
				ratio
			}
		} else {
			low
		}
	}

	## The width a column asks for, before its windows' constraints.
	requested_width : Strip, Column -> F64
	requested_width = |strip, column| {
		view = strip.tiling.w
		gap = strip.column_gap
		widths = strip.widths
		only =
			match strip.rest {
				[] => True
				[_, ..] => False
			}
		if widths.expand_single_column and only {
			view
		} else {
			match column.width {
				Exactly(width) => max(width, 1.0)
				Proportion(ratio) => proportional_width(view, gap, clamp_ratio(widths, ratio))
				Default => {
					locked = column.first.constraints.locked_width
					ratio =
						if widths.preserve_window_sizes and F64.is_finite(locked) and locked > 0.0 {
							width_ratio(view, gap, locked)
						} else {
							widths.ratio
						}
					proportional_width(view, gap, clamp_ratio(widths, ratio))
				}
			}
		}
	}

	## The column's width: what it asks for, within what its windows allow.
	column_width : Strip, Column, List((Row, WindowConstraints)) -> F64
	column_width = |strip, column, rows| {
		base = requested_width(strip, column)
		bounds =
			rows.fold(
				{ low: 1.0, fixed: 0.0, high: NoMax, flexible: False },
				|acc, (_, c)| {
					high =
						if c.max_width > 0.0 {
							match acc.high {
								Max(m) => Max(min(m, c.max_width))
								NoMax => Max(c.max_width)
							}
						} else {
							acc.high
						}
					{
						low: max(acc.low, reported_min(c, True)),
						fixed: max(
							acc.fixed,
							match fixed_for_axis(c, True) {
								Fixed(v) => v
								Free => 0.0
							},
						),
						high,
						flexible: acc.flexible or resizable_for_axis(c, True),
					}
				},
			)
		if bounds.flexible {
			capped =
				match bounds.high {
					Max(m) => min(base, m)
					NoMax => base
				}
			max(max(capped, bounds.low), bounds.fixed)
		} else {
			max(bounds.low, bounds.fixed)
		}
	}

	## Where the camera stops to show a column.
	anchor : Strip, F64, F64, Position -> F64
	anchor = |strip, world_x, width, position| {
		view = strip.tiling.w
		right = world_x + width - view
		if width >= view {
			world_x
		} else {
			match position {
				First => world_x
				Last => right
				Only | Middle =>
					match strip.alignment {
						Left => world_x
						Center => world_x - max(view - width, 0.0) / 2.0
						Right => right
					}
			}
		}
	}

	## A column's spot, where the next one starts, and its windows' frames.
	place_column : Strip, Column, Position, F64 -> (Spot, F64, List((WindowId, Rect)))
	place_column = |strip, column, position, x| {
		rows = [column.first].concat(column.rest).map(|row| (row, normalized(row.constraints)))
		width = column_width(strip, column, rows)
		available = max(strip.tiling.h - strip.row_gap * (rows.len() - 1).to_f64(), 0.0)
		items =
			rows.map(
				|(row, c)| {
					low = reported_min(c, False)
					{
						min: low,
						fixed: fixed_for_axis(c, False),
						max: if (c.max_height > 0.0) Max(c.max_height) else NoMax,
						weight: max(row.weight - low, least_weight),
						can_grow: resizable_for_axis(c, False),
					}
				},
			)
		(frames, _) =
			List.map2(rows, solve(items, available), |pair, height| (pair, height)).fold(
				([], strip.tiling.y),
				|(out, y), ((row, c), height)| {
					w0 = min(
						max(
							match fixed_for_axis(c, True) {
								Fixed(v) => v
								Free => width
							},
							reported_min(c, True),
						),
						width,
					)
					h0 = min(
						max(
							match fixed_for_axis(c, False) {
								Fixed(v) => v
								Free => height
							},
							reported_min(c, False),
						),
						height,
					)
					w = if (c.max_width > 0.0) min(w0, c.max_width) else w0
					h = if (c.max_height > 0.0) min(h0, c.max_height) else h0
					# As Rust's `y += height + row_gap`: the gap joins the height first.
					(out.append((row.window, { x, y, w, h })), y + (height + strip.row_gap))
				},
			)
		spot = { id: column.id, world_x: x, width, anchor: anchor(strip, x, width, position) }
		# Upstream's order (`x += width + gap`: the gap joins the width first); it must match
		# upstream's `Geometry::build` bit for bit.
		(spot, x + (width + strip.column_gap), frames)
	}

	## Where the columns and windows go, and where the camera may rest.
	place : Strip -> Placed
	place = |strip| {
		first_position =
			match strip.rest {
				[] => Only
				[_, ..] => First
			}
		(first, after_first, first_frames) = place_column(strip, strip.first, first_position, 0.0)
		(rest, frames) =
			match strip.rest {
				[] => ([], first_frames)
				[.. as middle, last] => {
					(spots, x, gathered) =
						middle.fold(
							([], after_first, first_frames),
							|(out, at, acc), column| {
								(spot, next, placed) = place_column(strip, column, Middle, at)
								(out.append(spot), next, acc.concat(placed))
							},
						)
					(spot, _, placed) = place_column(strip, last, Last, x)
					(spots.append(spot), gathered.concat(placed))
				}
			}
		(head, tail) = snaps_of(strip, first, rest)
		(low, high) =
			tail.fold(
				(head.offset, head.offset),
				|(lowest, highest), snap| (min(lowest, snap.offset), max(highest, snap.offset)),
			)
		{
			first,
			rest,
			frames: frames.map(|(window, frame)| { window, frame }),
			first_snap: head,
			rest_snaps: tail,
			low,
			high,
		}
	}

	## Anchored: each column's anchor. Free: the strip's two ends, then every column edge
	## strictly between them.
	snaps_of : Strip, Spot, List(Spot) -> (Snap, List(Snap))
	snaps_of = |strip, first, rest|
		match strip.navigation {
			Anchored => ({ offset: first.anchor, column: first.id }, rest.map(|s| { offset: s.anchor, column: s.id }))
			Free => {
				view = strip.tiling.w
				last =
					match rest {
						[.., spot] => spot
						[] => first
					}
				left = first.world_x
				right = last.world_x + last.width - view
				between =
					[first].concat(rest).fold(
						[{ offset: right, column: last.id }],
						|out, spot|
							[spot.world_x, spot.world_x + spot.width - view].fold(
								out,
								|acc, offset| if (left < offset and offset < right) acc.append({ offset, column: spot.id }) else acc,
							),
					)
				({ offset: left, column: first.id }, between)
			}
		}

	## Rust's f64::total_cmp: -0 before +0, NaNs at the ends by sign.
	total_order : F64 -> U64
	total_order = |value| {
		bits = F64.to_bits(value)
		sign = 9223372036854775808
		if (bits >= sign) U64.bitwise_not(bits) else U64.bitwise_xor(bits, sign)
	}

	## The spots after the column `id`, left to right.
	past : List(Spot), U64 -> List(Spot)
	past = |spots, id|
		spots.fold(
			{ seen: False, after: [] },
			|acc, spot|
				if acc.seen {
					{ ..acc, after: acc.after.append(spot) }
				} else {
					{ ..acc, seen: spot.id == id }
				},
		).after

	## The spots before the column `id`, right to left.
	before : List(Spot), U64 -> List(Spot)
	before = |spots, id|
		spots.fold_rev(
			{ seen: False, after: [] },
			|spot, acc|
				if acc.seen {
					{ ..acc, after: acc.after.append(spot) }
				} else {
					{ ..acc, seen: spot.id == id }
				},
		).after

	## Where the camera settles when a release would carry it to `projected` from `from`.
	settle : Placed, F64, F64, F64, Settle -> Settled
	settle = |placed, view, projected, from, how| {
		(head, tail) = (placed.first_snap, placed.rest_snaps)
		order = |a, b| {
			da = total_order((a.offset - projected).abs())
			db = total_order((b.offset - projected).abs())
			if da != db {
				if (da < db) Less else Greater
			} else {
				match how {
					Niri => {
						oa = total_order(a.offset)
						ob = total_order(b.offset)
						if oa == ob {
							Same
						} else if oa < ob {
							Less
						} else {
							Greater
						}
					}

					Anchored => Same
					AnchoredTowards(active) => {
						away_a = a.column != active
						away_b = b.column != active
						if away_a == away_b {
							Same
						} else if away_b {
							Less
						} else {
							Greater
						}
					}
				}
			}
		}
		# The first nearest point (as Rust's min_by): a later one wins only if nearer.
		snap = tail.fold(
			head,
			|best, candidate|
				match order(best, candidate) {
					Greater => candidate
					Less | Same => best
				},
		)
		spots = [placed.first].concat(placed.rest)
		column =
			match how {
				Anchored | AnchoredTowards(_) => snap.column
				Niri =>
					if projected >= from {
						past(spots, snap.column).fold(
							{ open: True, last: snap.column },
							|acc, spot|
								if acc.open and spot.world_x + spot.width <= snap.offset + view {
									{ ..acc, last: spot.id }
								} else {
									{ ..acc, open: False }
								},
						).last
					} else {
						before(spots, snap.column).fold(
							{ open: True, last: snap.column },
							|acc, spot|
								if acc.open and spot.world_x >= snap.offset {
									{ ..acc, last: spot.id }
								} else {
									{ ..acc, open: False }
								},
						).last
					}
			}
		{ column, offset: snap.offset }
	}

	## How much of a parked window stays on screen, in points.
	reveal : F64
	reveal = 1.0

	## Each window's frame on screen, in order: a fullscreen window's fullscreen frame; any other
	## its world frame moved by the camera, parked when the camera parks and it is wholly outside
	## the tiling area, then rounded (upstream's `translate_frames`, operation for operation).
	translate : List((WindowId, Rect)), Camera, List((WindowId, FullscreenMode)) -> List((WindowId, Rect))
	translate = |frames, camera, fullscreen|
		frames.map(
			|(window, frame)| {
				mode =
					fullscreen.fold(
						NotFullscreen,
						|found, (w, m)|
							match found {
								NotFullscreen => if (w == window) Found(m) else NotFullscreen
								Found(_) => found
							},
					)
				shown =
					match mode {
						Found(Screen) => camera.screen
						Found(WithinGaps) => camera.tiling
						NotFullscreen => round_rect(moved(frame, camera))
					}
				(window, shown)
			},
		)

	## A world frame moved by the camera (`x += tiling.x - offset`), and parked if it must be.
	moved : Rect, Camera -> Rect
	moved = |frame, camera| {
		tiling = camera.tiling
		screen = camera.screen
		x = frame.x + (tiling.x - camera.offset)
		parked =
			match camera.parking {
				InPlace => x
				Park =>
					if x + frame.w <= tiling.x {
						screen.x - frame.w + reveal
					} else if x >= tiling.x + tiling.w {
						screen.x + screen.w - reveal
					} else {
						x
					}
			}
		{ ..frame, x: parked }
	}
}
