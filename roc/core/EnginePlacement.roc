## The layout engine's placement choices (`rift_core::placement`): for now the drop's (what a
## drop does, whether a target drop moved the window, the action, the drag override, whether a
## drop edits a layout). Questions on plain data; windows and workspaces are one word each, and
## payloads the engine holds (the place, the trees) are tokens.

import pf.Boundary exposing [DropQuestion, DropChoice, DropFactsWire, DropActionWire, Rect, MaybeRect, MaybeSlot, AcrossWire]

EnginePlacement := [].{

	choose_drop : DropQuestion -> DropChoice
	choose_drop = |question|
		match question {
			Plan(facts) => Plan(drop_plan(facts))
			Kept(q) =>
				match q.before {
					Slot(before) =>
						match q.after {
							Slot(after) => if before == after Kept(Unchanged) else Kept(Moved)
							NoSlot => Kept(Moved)
						}
					NoSlot => Kept(Moved)
				}
			Action(q) =>
				match q.action {
					Insert(_) => if q.same_slot Action(q.center) else Action(q.action)
					_ => Action(q.action)
				}
			Override(shown) =>
				match shown {
					Arranged(Stack) => Override(Swap)
					_ => Override(ByZone)
				}
			Edit(arrangement) =>
				match arrangement {
					Floating => Edit(Refused)
					_ => Edit(Edit)
				}
		}

	usable : Rect -> Bool
	usable = |r| {
		finite = |v| v == v and v - v == 0.0
		finite(r.x) and finite(r.y) and finite(r.w) and finite(r.h) and r.w > 0.0 and r.h > 0.0
	}

	into_itself : DropActionWire -> Bool
	into_itself = |action|
		match action {
			Move(_) => True
			ConsumeOrExpel(_) => True
			_ => False
		}

	drop_plan : DropFactsWire -> _
	drop_plan = |facts|
		match facts.source {
			Nothing => Refused(NotAMember)
			Some(source) =>
				match facts.laid {
					NoSpace => Refused(NoDestination)
					NoLayout => Refused(NoLayout)
					Laid => {
						across : AcrossWire
						across = if source == facts.destination Same else Into(source)
						if facts.floating {
							match facts.frame {
								NoRect => Refused(FloatWithoutFrame)
								SomeRect(frame) =>
									if !usable(frame) {
										Refused(BadFrame)
									} else {
										match facts.target {
											Target(_) => Refused(FloatOntoTarget)
											NoTarget => {
												unmoved =
													match (across, facts.at) {
														(Same, SomeRect(at)) => at.x == frame.x and at.y == frame.y and at.w == frame.w and at.h == frame.h
														_ => False
													}
												if unmoved Refused(FloatUnmoved) else Float({ frame, across })
											}
										}
									}
							}
						} else {
							match facts.frame {
								SomeRect(_) => Refused(TiledWithFrame)
								NoRect =>
									match facts.target {
										Target(t) =>
											if t.window == facts.window and !into_itself(t.action) {
												Refused(SelfInsert)
											} else if !t.in_destination {
												Refused(ForeignTarget)
											} else if t.floating {
												Refused(FloatingTarget)
											} else {
												Target({ target: t.window, action: t.action, across })
											}
										NoTarget =>
											match across {
												Into(from) => Relocate(from)
												Same => Refused(InPlace)
											}
									}
							}
						}
					}
				}
		}
}
