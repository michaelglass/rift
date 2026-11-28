## The layout engine's placement choices (`rift_core::placement`): for now the drop's (what a
## drop does, whether a target drop moved the window, the action, the drag override, whether a
## drop edits a layout). Questions on plain data; windows and workspaces are one word each, and
## payloads the engine holds (the place, the trees) are tokens.

import pf.Boundary exposing [DropQuestion, DropChoice, DropFactsWire, DropActionWire, Rect, MaybeRect, MaybeSlot, AcrossWire, RelocationFactsWire, RelocationWire, FamilyWire, EditedToken, WidthWire, ObservedWire, ProjectionWire, PlaceQuestion, PlaceChoice, AdditionFactsWire, AdditionWire, InsertWidth, InsertToken, FrameQuestion]

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

	## What relocating a window does (`rift_core::placement::relocation`), on the facts' token form.
	relocation : RelocationFactsWire -> RelocationWire
	relocation = |f|
		match f.families {
			Unknown => Refused(UnknownWorkspace)
			Families(families) => {
				observed = match f.policy {
					Observed => True
					_ => False
				}
				commanded = match f.policy {
					Workspace => True
					_ => False
				}
				on_space =
					match f.destination_space {
						Some(space) => space == f.space
						Nothing => False
					}
				owned_elsewhere =
					match f.owner {
						Some(owner) => owner != f.source and !(observed and owner == f.destination)
						Nothing => False
					}
				directional_without_layout = match f.policy {
					Directional(_) => !f.has_layout
					_ => False
				}
				if !on_space {
					Refused(OtherSpace)
				} else if owned_elsewhere {
					Refused(OwnedElsewhere)
				} else if directional_without_layout {
					Refused(DirectionalWithoutLayout)
				} else {
					width = |laid| if is_scrolling(families.destination) and f.preserve_widths and laid Preserve else AsIs
					floating_layouts = is_floating(families.source) or is_floating(families.destination)
					into_floating_layout = is_floating(families.destination)
					projection : ProjectionWire
					projection =
						match f.policy {
							Directional(direction) => Toward({ direction, width: width(True) })
							Workspace => projected(f.in_floats and !floating_layouts, Moved, into_floating_layout, NoTree, width(f.has_layout), f.has_layout)
							Observed => projected(f.projected_floating, WasMember, into_floating_layout, NoTree, width(f.has_layout), f.has_layout)
							Drop(edited) => projected(f.projected_floating, Moved, into_floating_layout, edited, width(f.has_layout), f.has_layout)
						}
					assign =
						match f.owner {
							Some(owner) => if owner == f.destination Already else Assign
							Nothing => Assign
						}
					afterwards =
						if f.unavailable or (commanded and !f.placeable) {
							Suspended
						} else if observed {
							Resumed
						} else {
							AsIs
						}
					Move({ assign, projection, afterwards })
				}
			}
		}

	is_floating : FamilyWire -> Bool
	is_floating = |family|
		match family {
			Floating => True
			_ => False
		}

	is_scrolling : FamilyWire -> Bool
	is_scrolling = |family|
		match family {
			Scrolling => True
			_ => False
		}

	projected : Bool, ObservedWire, Bool, EditedToken, WidthWire, Bool -> ProjectionWire
	projected = |floating, observed, into_floating_layout, edited, width, has_layout|
		if floating and !into_floating_layout {
			Float(observed)
		} else if floating {
			Unfloat(
				match edited {
					Tree => Edited
					NoTree => if has_layout Add else NoLayout
				},
			)
		} else {
			Tile(
				match edited {
					Tree => Edited(width)
					NoTree => if has_layout Add(width) else NoLayout
				},
			)
		}

	## One of the remaining placement choices (`rift_core::placement::choose_place`).
	choose_place : PlaceQuestion -> PlaceChoice
	choose_place = |question|
		match question {
			Addition(facts) => Addition(addition(facts))
			WindowMove(q) =>
				match q.from {
					Nothing => WindowMove(Refused(NotAMember))
					Some(from) =>
						match q.to {
							Nothing => WindowMove(Refused(NoWorkspace))
							Some(to) => WindowMove(Move({ from, to }))
						}
				}
			Landing(placeable) => if placeable Landing(Focused) else Landing(Hidden)
			WorkspaceMove(q) =>
				match q.shown {
					Nothing => WorkspaceMove(Refused(Nothing))
					Some(shown) =>
						if !q.per_display {
							WorkspaceMove(Transfer(shown))
						} else if q.same_space {
							WorkspaceMove(Refused(SameSpace))
						} else {
							match q.index {
								Nothing => WorkspaceMove(Refused(NoIndex))
								Some(index) => WorkspaceMove(Members({ from: shown, index }))
							}
						}
				}
			Members(found) =>
				match found {
					Some(id) => Members(To(id))
					Nothing => Members(Refused(NoDestination))
				}
			MovedFocus(q) => {
				preferred =
					match q.preferred {
						Some(w) => if q.moved.contains(w) Some(w) else Nothing
						Nothing => Nothing
					}
				match preferred {
					Some(w) => MovedFocus(Preferred(w))
					Nothing =>
						match q.moved.first() {
							Ok(w) => MovedFocus(First(w))
							Err(_) => MovedFocus(Nothing)
						}
				}
			}
			Claim(q) =>
				if !q.claimed {
					Claim(Free)
				} else {
					match q.management_override {
						NoBool => Claim(Claimed(ByRule))
						SomeBool(o) => Claim(Claimed(Override(o)))
					}
				}
			Assignment(q) => {
				rule_workspace = if q.restored DroppedByRestore else Kept
				native = if q.reapply and q.rule_names_workspace and !q.restored Forgotten else Kept
				Assignment({ rule_workspace, native })
			}
			RuleFloat(q) =>
				if q.should_float {
					RuleFloat(Float)
				} else if q.was_floating {
					RuleFloat(Unfloat)
				} else {
					RuleFloat(Keep)
				}
			RuleFrame(q) =>
				if q.has_place {
					RuleFrame(Place)
				} else if q.has_resize {
					RuleFrame(Resize)
				} else {
					RuleFrame(Keep)
				}
			Frame(q) => Frame(requested_frame(q))
		}

	addition : AdditionFactsWire -> AdditionWire
	addition = |f|
		if !f.admitted {
			Skip(Unadmitted)
		} else if f.fullscreen_suspended {
			Skip(FullscreenSuspended)
		} else {
			match f.workspace {
				Unassigned => Skip(Unassigned)
				Assigned(a) =>
					match a.family {
						Floating => IntoFloatingLayout({ workspace: a.workspace, insert: insert_token(f) })
						_ =>
							if f.in_floats {
								Float(a.workspace)
							} else {
								width = match a.family {
									Scrolling => if f.preserve_widths Preserve else AsIs
									_ => AsIs
								}
								Tile({ workspace: a.workspace, insert: insert_width(f, width) })
							}
					}
			}
		}

	insert_width : AdditionFactsWire, WidthWire -> InsertWidth
	insert_width = |f, width|
		match f.tree {
			NoTree => NoLayout
			Tree(holds) => if holds Present else Add(width)
		}

	insert_token : AdditionFactsWire -> InsertToken
	insert_token = |f|
		match f.tree {
			NoTree => NoLayout
			Tree(holds) => if holds Present else Add
		}

	smart_width : F64
	smart_width = 0.8

	smart_height : F64
	smart_height = 0.93

	requested_frame : FrameQuestion -> Rect
	requested_frame = |q| {
		(w, h) =
			match q.sizing {
				Keep => (q.frame.w, q.frame.h)
				Dimensions(size) => (size.w, size.h)
				Smart => (q.screen.w * smart_width, q.screen.h * smart_height)
			}
		(x, y) =
			match q.centering {
				Keep => (q.frame.x, q.frame.y)
				Center => (q.screen.x + q.screen.w / 2.0 - w / 2.0, q.screen.y + q.screen.h / 2.0 - h / 2.0)
			}
		{ x, y, w, h }
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
