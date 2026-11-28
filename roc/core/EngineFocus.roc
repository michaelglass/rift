## The layout engine's focus choices (`rift_core::focus::choose`): which window a workspace
## focuses, where a committed focus is recorded, directional focus past a layout's edge, and the
## Space in a direction. Questions on plain data; windows and Spaces are one word each.

import pf.Boundary exposing [FocusQuestion, FocusChoice, MaybeEligible, EligibleWindow, BspDirection, SpaceRect, Rect, MaybeWord]

EngineFocus := [].{

	choose : FocusQuestion -> FocusChoice
	choose = |question|
		match question {
			Workspace(f) => Window(workspace_focus(f.preferred, f.last_focused, f.selected, f.visible, f.floating, f.floating_last_focus))
			Record(r) =>
				if !r.workspace {
					Record(NoWorkspace)
				} else {
					match r.focus {
						Focus(c) =>
							if !c.belongs {
								Record(Cleared)
							} else if c.floating {
								Record(Floating(c.window))
							} else {
								Record(Tiled(c.window))
							}
						NoFocus => Record(Cleared)
					}
				}
			Cycle(c) => Window(floating_cycle(c.floating, c.focused, c.direction))
			Edge(e) =>
				if e.scrolling and is_horizontal(e.direction) {
					Edge(Stop)
				} else {
					Edge(NextSpace)
				}
			Entry(e) =>
				match e.directional {
					Some(w) => Window(Some(w))
					Nothing => Window(first(e.visible))
				}
			Fallback(f) => Window(first_of([f.first_floating, f.previous, f.first_visible]))
			SpaceInDirection(s) => Space(space_in_direction(s.current, s.direction, s.candidates))
		}

	is_horizontal : BspDirection -> Bool
	is_horizontal = |direction|
		match direction {
			Left => True
			Right => True
			Up => False
			Down => False
		}

	first : List(U64) -> MaybeWord
	first = |list|
		match list.first() {
			Ok(w) => Some(w)
			Err(_) => Nothing
		}

	first_of : List(MaybeWord) -> MaybeWord
	first_of = |maybes| maybes.fold(Nothing, |found, m| match found {
		Some(_) => found
		Nothing => m
	})

	belonging : MaybeEligible -> MaybeWord
	belonging = |e|
		match e {
			Candidate(c) => if c.belongs Some(c.window) else Nothing
			NoCandidate => Nothing
		}

	workspace_focus : MaybeEligible, MaybeEligible, MaybeEligible, List(EligibleWindow), List(U64), MaybeWord -> MaybeWord
	workspace_focus = |preferred, last_focused, selected, visible, floating, floating_last_focus| {
		visible_one =
			match visible.find_first(|e| e.belongs) {
				Ok(e) => Some(e.window)
				Err(_) => Nothing
			}
		last_floating =
			match floating_last_focus {
				Some(w) => if floating.contains(w) Some(w) else Nothing
				Nothing => Nothing
			}
		first_of([belonging(preferred), belonging(last_focused), belonging(selected), visible_one, last_floating, first(floating)])
	}

	floating_cycle : List(U64), MaybeWord, BspDirection -> MaybeWord
	floating_cycle = |floating, focused, direction| {
			n = floating.len()
			if !is_horizontal(direction) or n < 2 {
				Nothing
			} else {
				match focused {
					Some(w) =>
						match floating.find_first_index(|x| x == w) {
							Ok(index) => {
								step =
									match direction {
										Left => n - 1
										_ => 1
									}
								match floating.get((index + step) % n) {
									Ok(next) => Some(next)
									Err(_) => Nothing
								}
							}
							Err(_) => Nothing
						}
					Nothing => Nothing
				}
			}
	}

	gap : F64, F64, F64, F64 -> F64
	gap = |a_min, a_max, b_min, b_max| F64.max(F64.max(b_min - a_max, a_min - b_max), 0.0)

	space_in_direction : Rect, BspDirection, List(SpaceRect) -> MaybeWord
	space_in_direction = |current, direction, candidates| {
		cx0 = current.x
		cy0 = current.y
		cx1 = current.x + current.w
		cy1 = current.y + current.h
		best = candidates.fold(Nothing, |found, candidate| {
			c = candidate.rect
			x0 = c.x
			y0 = c.y
			x1 = c.x + c.w
			y1 = c.y + c.h
			(primary, across) =
				match direction {
					Left => (cx0 - x1, gap(cy0, cy1, y0, y1))
					Right => (x0 - cx1, gap(cy0, cy1, y0, y1))
					Up => (cy0 - y1, gap(cx0, cx1, x0, x1))
					Down => (y0 - cy1, gap(cx0, cx1, x0, x1))
				}
			if primary < 0.0 {
				found
			} else {
				match found {
					Best(b) => if across < b.across or (across == b.across and primary < b.primary) Best({ across, primary, space: candidate.space }) else found
					Nothing => Best({ across, primary, space: candidate.space })
				}
			}
		})
		match best {
			Best(b) => Some(b.space)
			Nothing => Nothing
		}
	}
}
