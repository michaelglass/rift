## A stack layout system's stacks (`rift_core::layout::stack_tree::Stacks`), ported operation by
## operation: every layout's stack in one slot table (keys as slotmap allocates them), stack and
## member ids minted from one counter (upstream's saved `next_node`). Keys, ids and windows are
## one word each.
##
## A stack is looked up only by a key the system holds: a missing one is a broken system, as it
## is in Rust (whose `Index` panics), so the lookup crashes, naming what looked and the key.

import Geometry
import Slots
import pf.Boundary exposing [Child, Constraints, StackFramesQuestion, TraditionalNode, TraditionalShape, WindowConstraints, WindowId, BspAppIn, BspDirection, BspEdited, BspFocusIn, BspFocused, BspFullscreen, BspLayoutDirection, BspNativeResize, BspOrientation, BspPair, BspParticipation, BspReplacement, BspWindowIn, BspWindowsQuery, Fill, MaybeWord, Rect, StackEditIn, StackGroupAnswer, StackGroupIn, StackLayoutSlot, StackMember, StackMembers, StackMove, StackSelection, StackSettings, StackSlot, StackTab, StackTreeAnswer, StackTreeEvent, StackTreeSeed, StackTreeStack]

## A stack layout system: every layout's stack, the id counter, the settings, the windows left
## out of layout.
Stacks : {
	layouts : Slots(StackTreeStack),
	next_node : U32,
	settings : StackSettings,
	left_out : Set(U64),
}

## The system as a view gives it: the table's slots and free list, the counter, the settings,
## the windows left out in order.
View : {
	layouts : List(StackLayoutSlot),
	layout_free : List(U32),
	next_node : U32,
	settings : StackSettings,
	left_out : List(U64),
}

## A member found in a list: the members before it, it, and those after it.
Zipper : { before : List(StackMember), member : StackMember, after : List(StackMember) }

StackTree := [].{

	empty : StackSettings -> Stacks
	empty = |settings| { layouts: Slots.empty({}), next_node: 1, settings, left_out: Set.empty() }

	## A system as the platform holds it (the table's slots, vacant ones included, and its free
	## list in taking order; the counter; the settings; the windows left out), as
	## `Stacks::from_parts` takes it. The host parses it first.
	seeded : StackTreeSeed -> Stacks
	seeded = |seed| {
		layouts: {
			slots: seed.layouts.map(
				|slot|
					match slot {
						Occupied({ version, stack: held }) => Occupied({ version, value: held })
						Vacant(version) => Vacant(version)
					},
			),
			free: seed.layout_free,
		},
		next_node: seed.next_node,
		settings: seed.settings,
		left_out: Set.from_list(seed.left_out),
	}

	## One operation, and what it answers.
	step : Stacks, StackTreeEvent -> (Stacks, StackTreeAnswer)
	step = |stacks, event|
		match event {
			CreateLayout => {
				(after, layout) = create_layout(stacks)
				(after, Layout(layout))
			}
			CloneLayout(layout) => {
				(after, clone) = clone_layout(stacks, layout)
				(after, Layout(clone))
			}
			RemoveLayout(layout) => {
				(layouts, _) = Slots.remove(stacks.layouts, layout)
				({ ..stacks, layouts }, Done)
			}
			Participation(asked) => {
				(after, changed) = set_window_participation(stacks, asked)
				(after, Participated(changed))
			}
			Settings(settings) => ({ ..stacks, settings }, Done)
			AddWindow(added) => (add_window(stacks, added), Done)
			RemoveWindow(window) => (remove_window(stacks, window), Done)
			ReplaceWindow(replacement) => (replace_window(stacks, replacement), Done)
			CloseApp(windows) => (close_app(stacks, windows), Done)
			ResizeWindow(resize) => (resize_window(stacks, resize), Done)
			ContainsLayout(layout) =>
				match Slots.get(stacks.layouts, layout) {
					Held(_) => (stacks, Verdict(True))
					NotHeld => (stacks, Verdict(False))
				}
			WindowParticipates(window) => (stacks, Verdict(takes_part(stacks, window)))
			SelectedWindow(layout) => (stacks, Window(selected_window(stacks, layout)))
			SelectionContainsWindow(asked) => (stacks, Verdict(selection_contains_window(stacks, asked)))
			WindowInDirection(asked) => (stacks, Window(window_in_direction(stacks, asked)))
			ContainsWindow({ layout, window }) => (stacks, Verdict(contains_window(stacks, layout, window)))
			HasAnyFullscreenNode(layout) => (stacks, Verdict(has_any_fullscreen_node(stacks, layout)))
			HasWindowsForApp(asked) => (stacks, Verdict(has_windows_for_app(stacks, asked)))
			Windows(asked) => (stacks, Windows(listed_windows(stacks, asked)))
			WindowSlot(asked) => (stacks, WindowSlot(window_slot(stacks, asked)))
			GroupContainer(asked) => (stacks, Group(group_container(stacks, asked)))
			Focus(asked) => {
				(after, focused) = focus(stacks, asked)
				(after, Focused(focused))
			}
			Edit(asked) => {
				(after, edited) = edit(stacks, asked)
				(after, Edited(edited))
			}
		}

	view : Stacks -> View
	view = |stacks| {
		layouts: stacks.layouts.slots.map(
			|slot|
				match slot {
					Occupied({ version, value }) => Occupied({ version, stack: value })
					Vacant(version) => Vacant(version)
				},
		),
		layout_free: stacks.layouts.free,
		next_node: stacks.next_node,
		settings: stacks.settings,
		left_out: stacks.left_out.to_list().sort_with(|a, b| a.order_relative_to(b)),
	}

	# Layouts, ids, participation.

	## A fresh id: version 1 over the counter, which moves on (`Stacks::node_id`).
	mint : Stacks -> (Stacks, U64)
	mint = |stacks| {
		index = stacks.next_node
		if index == 0xFFFF_FFFF {
			crash "StackTree.mint: stack node identity exhausted"
		} else {
			({ ..stacks, next_node: index + 1 }, Slots.key_of(index.to_u64(), 1))
		}
	}

	create_layout : Stacks -> (Stacks, U64)
	create_layout = |stacks| {
		(minted, id) = mint(stacks)
		orientation : BspOrientation
		orientation =
			match stacks.settings.orientation {
				Horizontal => Horizontal
				Same => Horizontal
				Perpendicular => Vertical
				Vertical => Vertical
			}
		(layouts, layout) = Slots.insert(minted.layouts, { id, members: Empty, orientation, fill: Tiled })
		({ ..minted, layouts }, layout)
	}

	## A copy of the layout under fresh ids: the stack's first, then each member's in order.
	clone_layout : Stacks, U64 -> (Stacks, U64)
	clone_layout = |stacks, layout| {
		source = stack_at(stacks, layout, "clone_layout")
		(with_id, id) = mint(stacks)
		(minted, copied) =
			match source.members {
				Empty => (with_id, Empty)
				Populated(held) => {
					(after_first, first_id) = mint(with_id)
					start = {
						stacks: after_first,
						rest: [],
						chosen: rechosen(held.selection, held.first.id, first_id),
					}
					done = held.rest.fold(
						start,
						|acc, member| {
							(next, fresh) = mint(acc.stacks)
							{
								stacks: next,
								rest: acc.rest.append({ ..member, id: fresh }),
								chosen: rechosen(acc.chosen, member.id, fresh),
							}
						},
					)
					(done.stacks, Populated({ first: { ..held.first, id: first_id }, rest: done.rest, selection: done.chosen }))
				}
			}
		(layouts, clone) = Slots.insert(minted.layouts, { ..source, id, members: copied })
		({ ..minted, layouts }, clone)
	}

	## The selection, following a member from its old id to its new one.
	rechosen : StackSelection, U64, U64 -> StackSelection
	rechosen = |chosen, old, fresh|
		match chosen {
			Member(id) if id == old => Member(fresh)
			Root(id) if id == old => Root(fresh)
			_ => chosen
		}

	set_window_participation : Stacks, BspParticipation -> (Stacks, Bool)
	set_window_participation = |stacks, { window, participation }|
		match participation {
			TakesPart => ({ ..stacks, left_out: stacks.left_out.remove(window) }, stacks.left_out.contains(window))
			LeftOut => ({ ..stacks, left_out: stacks.left_out.insert(window) }, !stacks.left_out.contains(window))
		}

	takes_part : Stacks, U64 -> Bool
	takes_part = |stacks, window| !stacks.left_out.contains(window)

	# Windows.

	## `window` put into the layout's stack, selected (nothing if the stack holds it already):
	## after the selected member, or at the end.
	add_window : Stacks, BspWindowIn -> Stacks
	add_window = |stacks, { layout, window }|
		if contains_window(stacks, layout, window) {
			stacks
		} else {
			(minted, id) = mint(stacks)
			held = stack_at(minted, layout, "add_window")
			member = { id, window, weight: 1.0, fill: Tiled }
			all = listed(held)
			placed =
				match (stacks.settings.insertion, held.members) {
					(NextToSelection, Populated(populated)) =>
						match zip(all, |m| m.id == remembered(populated.selection)) {
							Found(at) => at.before.append(at.member).append(member).concat(at.after)
							NotFound => all.append(member)
						}
					_ => all.append(member)
				}
			set_stack(minted, layout, { ..held, members: rebuilt(placed, Member(id)) }, "add_window")
		}

	## A window gone from every layout.
	remove_window : Stacks, U64 -> Stacks
	remove_window = |stacks, window| {
		without = { ..stacks, left_out: stacks.left_out.remove(window) }
		each_stack(without, |held| retained(held, |m| m.window != window))
	}

	## A window that took another's place, in every layout.
	replace_window : Stacks, BspReplacement -> Stacks
	replace_window = |stacks, { from, to }| {
		left_out = if (stacks.left_out.contains(from)) stacks.left_out.remove(from).insert(to) else stacks.left_out
		each_stack(
			{ ..stacks, left_out },
			|held| renamed(held, |m| if (m.window == from) { ..m, window: to } else m),
		)
	}

	## An app's windows (`windows`, the ones the system holds) gone from every layout.
	close_app : Stacks, List(U64) -> Stacks
	close_app = |stacks, windows| {
		left_out = stacks.left_out.drop_if(|window| windows.contains(window))
		each_stack({ ..stacks, left_out }, |held| retained(held, |m| !windows.contains(m.window)))
	}

	## A native resize onto or off the screen or the tiling area: the member's fill follows it.
	resize_window : Stacks, BspNativeResize -> Stacks
	resize_window = |stacks, resize|
		if takes_part(stacks, resize.window) {
			held = stack_at(stacks, resize.layout, "resize_window")
			match zip(listed(held), |m| m.window == resize.window) {
				Found(at) => {
					member = { ..at.member, fill: resized(at.member.fill, resize) }
					set_stack(stacks, resize.layout, refilled(held, at, member), "resize_window")
				}
				NotFound => stacks
			}
		} else {
			stacks
		}

	# Queries.

	selected_window : Stacks, U64 -> MaybeWord
	selected_window = |stacks, layout|
		match active(stacks, stack_at(stacks, layout, "selected_window")) {
			Found(at) => Some(at.member.window)
			NotFound => Nothing
		}

	selection_contains_window : Stacks, BspWindowIn -> Bool
	selection_contains_window = |stacks, { layout, window }| {
		held = stack_at(stacks, layout, "selection_contains_window")
		match held.members {
			Empty => False
			Populated(populated) =>
				match populated.selection {
					Root(_) => listed(held).any(|m| m.window == window)
					Member(id) => listed(held).any(|m| m.id == id and m.window == window)
				}
		}
	}

	window_in_direction : Stacks, BspLayoutDirection -> MaybeWord
	window_in_direction = |stacks, { layout, direction }| {
		held = stack_at(stacks, layout, "window_in_direction")
		all = listed(held)
		chosen =
			match held.members {
				Populated(populated) =>
					match populated.selection {
						Member(id) => zip(all, |m| m.id == id and takes_part(stacks, m.window))
						Root(_) => NotFound
					}
				Empty => NotFound
			}
		match chosen {
			Found(at) => Some(at.member.window)
			NotFound => {
				part = all.keep_if(|m| takes_part(stacks, m.window))
				edge =
					match direction {
						Left => part.last()
						Up => part.last()
						Right => part.first()
						Down => part.first()
					}
				match edge {
					Ok(member) => Some(member.window)
					Err(_) => Nothing
				}
			}
		}
	}

	contains_window : Stacks, U64, U64 -> Bool
	contains_window = |stacks, layout, window|
		listed(stack_at(stacks, layout, "contains_window")).any(|m| m.window == window)

	has_any_fullscreen_node : Stacks, U64 -> Bool
	has_any_fullscreen_node = |stacks, layout| {
		held = stack_at(stacks, layout, "has_any_fullscreen_node")
		match selected_window(stacks, layout) {
			Some(_) => !is_tiled(held.fill) or listed(held).any(|m| takes_part(stacks, m.window) and !is_tiled(m.fill))
			Nothing => False
		}
	}

	has_windows_for_app : Stacks, BspAppIn -> Bool
	has_windows_for_app = |stacks, { layout, windows }|
		listed(stack_at(stacks, layout, "has_windows_for_app")).any(|m| windows.contains(m.window))

	listed_windows : Stacks, BspWindowsQuery -> List(U64)
	listed_windows = |stacks, { layout, query }| {
		held = stack_at(stacks, layout, "windows")
		match query {
			All => listed(held).map(|m| m.window)
			Visible => visible_windows(stacks, held)
			Selection => visible_windows(stacks, held)
			Stack(window) => if (listed(held).any(|m| m.window == window)) listed(held).map(|m| m.window) else []
		}
	}

	visible_windows : Stacks, StackTreeStack -> List(U64)
	visible_windows = |stacks, held|
		match visible(stacks, held) {
			Found(at) => [at.member.window]
			NotFound => []
		}

	window_slot : Stacks, BspWindowIn -> StackSlot
	window_slot = |stacks, { layout, window }|
		match zip(listed(stack_at(stacks, layout, "window_slot")), |m| m.window == window) {
			Found(at) => Slot(at.before.len())
			NotFound => NoSlot
		}

	## The stack as the group bar shows it; none when nothing is shown, the stack fills the
	## screen, or (the selection path only) the shown member does.
	group_container : Stacks, StackGroupIn -> StackGroupAnswer
	group_container = |stacks, { layout, scope }| {
		held = stack_at(stacks, layout, "group_container")
		match visible(stacks, held) {
			NotFound => NoGroup
			Found(at) => {
				shown_fills =
					match scope {
						SelectionPath => !is_tiled(at.member.fill)
						Whole => False
					}
				if !is_tiled(held.fill) or shown_fills {
					NoGroup
				} else {
					Group(
						{
							stack: held.id,
							orientation: held.orientation,
							windows: listed(held).keep_if(|m| takes_part(stacks, m.window)).map(|m| m.window),
							selected: at.before.count_if(|m| takes_part(stacks, m.window)),
						},
					)
				}
			}
		}
	}

	# The shape (`Stacks::shape`): the traditional geometry's input.

	## A layout's shape for the traditional geometry: its stack, with the shown member focused
	## and the other members taking part before and after it, each with the constraints the host
	## knows; none when no member takes part. The stack's words become the geometry's
	## `WindowId`s here, and a member's weight its share (Rust's `as f32`).
	shape : Stacks, StackFramesQuestion -> [Shaped(TraditionalShape), Unshaped]
	shape = |stacks, question| {
		held = stack_at(stacks, question.layout, "shape")
		match visible(stacks, held) {
			NotFound => Unshaped
			Found(at) => {
				known = Dict.from_list(question.known.map(|k| (k.window, k.constraints)))
				child = |member| Child.{
					size: Geometry.narrow(member.weight),
					node: TraditionalNode.Window({ window: window_id(member.window), constraints: known_constraints(known, member.window), fill: member.fill }),
				}
				taking_part = |members| members.keep_if(|m| takes_part(stacks, m.window)).map(child)
				fill =
					match held.fill {
						Screen => Screen
						_ => Tiled
					}
				horizontal =
					match held.orientation {
						Horizontal => True
						Vertical => False
					}
				Shaped(
					{
						screen: question.screen,
						area: Geometry.tiling_area(question.screen, question.gaps),
						gaps: question.gaps,
						stack_offset: question.stack_offset,
						stack_line: question.stack_line,
						root: TraditionalNode.Stack(
							{
								horizontal,
								fill,
								before: taking_part(at.before),
								focused: child(at.member),
								after: taking_part(at.after),
							},
						),
					},
				)
			}
		}
	}

	## A window's constraints, if the host knows them.
	known_constraints : Dict(U64, WindowConstraints), U64 -> Constraints
	known_constraints = |known, word|
		match known.get(word) {
			Ok(constraints) => Constrained(constraints)
			Err(KeyNotFound) => Unconstrained
		}

	## The window a word names: its pid in the high half, its index in the low half.
	window_id : U64 -> WindowId
	window_id = |word| WindowId.{ pid: word.shr_zf_wrap(32).to_u32_wrap().to_i32_wrap(), idx: word.to_u32_wrap() }

	# Focus.

	focus : Stacks, BspFocusIn -> (Stacks, BspFocused)
	focus = |stacks, { layout, focus: asked }|
		match asked {
			Select(window) => {
				(after, handled) = select_window(stacks, layout, window)
				(after, Selection(handled))
			}
			Ascend => climb(stacks, layout, Up)
			Descend => climb(stacks, layout, Down)
			Direction(direction) => {
				held = stack_at(stacks, layout, "focus")
				match (neighbor(stacks, held, direction), held.members) {
					(Neighbor(_, target), Populated(populated)) => {
						chosen = { ..held, members: Populated({ ..populated, selection: Member(target.member.id) }) }
						(set_stack(stacks, layout, chosen, "focus"), Movement({ window: Some(target.member.window), raise: [target.member.window] }))
					}
					_ => (stacks, Movement({ window: Nothing, raise: [] }))
				}
			}
		}

	## The stack itself selected (up) or its remembered member (down), while a window is shown.
	climb : Stacks, U64, [Up, Down] -> (Stacks, BspFocused)
	climb = |stacks, layout, way| {
		held = stack_at(stacks, layout, "climb")
		match (selected_window(stacks, layout), held.members) {
			(Some(_), Populated(populated)) => {
				id = remembered(populated.selection)
				chosen : StackSelection
				chosen =
					match way {
						Up => Root(id)
						Down => Member(id)
					}
				handled = !same_selection(populated.selection, chosen)
				(set_stack(stacks, layout, { ..held, members: Populated({ ..populated, selection: chosen }) }, "climb"), Selection(handled))
			}
			_ => (stacks, Selection(False))
		}
	}

	select_window : Stacks, U64, U64 -> (Stacks, Bool)
	select_window = |stacks, layout, window|
		if takes_part(stacks, window) {
			held = stack_at(stacks, layout, "select_window")
			match held.members {
				Empty => (stacks, False)
				Populated(populated) =>
					match zip(listed(held), |m| m.window == window) {
						Found(at) => (set_stack(stacks, layout, { ..held, members: Populated({ ..populated, selection: Member(at.member.id) }) }, "select_window"), True)
						NotFound => (stacks, False)
					}
			}
		} else {
			(stacks, False)
		}

	# Edits.

	edit : Stacks, StackEditIn -> (Stacks, BspEdited)
	edit = |stacks, { layout, edit: asked }|
		match asked {
			Moved(move) => edited_by(move_selection(stacks, layout, move))
			Fullscreen(mode) => {
				(after, raised) = toggle_fullscreen(stacks, layout, mode)
				(after, Raise(raised))
			}
			Swap({ a, b }) => edited_by(swap_windows(stacks, layout, a, b))
			ReorderTab(tab) => edited_by(move_tab(stacks, layout, tab))
			Drop({ a: source, b: target }) =>
				if source == target {
					(stacks, Unchanged)
				} else {
					match swap_windows(stacks, layout, source, target) {
						(swapped, True) => edited_by(select_window(swapped, layout, source))
						(unswapped, False) => (unswapped, Unchanged)
					}
				}
			Restack => {
				turned = toggle_orientation(stacks, layout)
				(turned, Raise(visible_windows(turned, stack_at(turned, layout, "restack"))))
			}
			ToggleOrientation => (toggle_orientation(stacks, layout), Changed)
		}

	edited_by : (Stacks, Bool) -> (Stacks, BspEdited)
	edited_by = |(stacks, changed)| if changed (stacks, Changed) else (stacks, Unchanged)

	## The selected member moved one place along the stack's axis, past members left out; past
	## the end on the other axis, to that end, the stack turning onto that axis.
	move_selection : Stacks, U64, StackMove -> (Stacks, Bool)
	move_selection = |stacks, layout, { direction, beyond }| {
		held = stack_at(stacks, layout, "move_selection")
		match (held.members, active(stacks, held)) {
			(Populated(populated), Found(at)) =>
				match populated.selection {
					Root(_) => (stacks, False)
					Member(_) => {
						moved =
							match neighbor(stacks, held, direction) {
								Neighbor(_, target) =>
									if is_backwards(direction) {
										Placed(target.before.append(at.member).append(target.member).concat(target.after).concat(at.after), held.orientation)
									} else {
										Placed(at.before.concat(target.before).append(target.member).append(at.member).concat(target.after), held.orientation)
									}
								NoNeighbor =>
									match beyond {
										AnotherDisplay => Stays
										Nothing =>
											if same_axis(held.orientation, direction) {
												Stays
											} else if is_backwards(direction) {
												Placed(at.before.concat(at.after).prepend(at.member), axis_of(direction))
											} else {
												Placed(at.before.concat(at.after).append(at.member), axis_of(direction))
											}
									}
							}
						match moved {
							Placed(order, orientation) =>
								(set_stack(stacks, layout, { ..held, orientation, members: rebuilt(order, Member(at.member.id)) }, "move_selection"), True)
							Stays => (stacks, False)
						}
					}
				}
			_ => (stacks, False)
		}
	}

	toggle_orientation : Stacks, U64 -> Stacks
	toggle_orientation = |stacks, layout| {
		held = stack_at(stacks, layout, "toggle_orientation")
		orientation : BspOrientation
		orientation =
			match held.orientation {
				Horizontal => Vertical
				Vertical => Horizontal
			}
		set_stack(stacks, layout, { ..held, orientation }, "toggle_orientation")
	}

	## The selection's fill toggled (the stack's while it is selected); the shown window is
	## raised when a fullscreen mode turned on.
	toggle_fullscreen : Stacks, U64, BspFullscreen -> (Stacks, List(U64))
	toggle_fullscreen = |stacks, layout, mode| {
		held = stack_at(stacks, layout, "toggle_fullscreen")
		match (held.members, active(stacks, held)) {
			(Populated(populated), Found(at)) => {
				(after, now) =
					match populated.selection {
						Root(_) => {
							fill = toggled(held.fill, mode)
							({ ..held, fill }, fill)
						}
						Member(_) => {
							fill = toggled(at.member.fill, mode)
							(refilled(held, at, { ..at.member, fill }), fill)
						}
					}
				toggled_stacks = set_stack(stacks, layout, after, "toggle_fullscreen")
				if is_tiled(now) {
					(toggled_stacks, [])
				} else {
					(toggled_stacks, visible_windows(toggled_stacks, after))
				}
			}
			_ => (stacks, [])
		}
	}

	swap_windows : Stacks, U64, U64, U64 -> (Stacks, Bool)
	swap_windows = |stacks, layout, a, b|
		if takes_part(stacks, a) and takes_part(stacks, b) {
			held = stack_at(stacks, layout, "swap_windows")
			all = listed(held)
			match (held.members, zip(all, |m| m.window == a), zip(all, |m| m.window == b)) {
				(Populated(populated), Found(at_a), Found(at_b)) => {
					(i, j) = (at_a.before.len(), at_b.before.len())
					if i == j {
						(stacks, False)
					} else {
						chosen : StackSelection
						chosen =
							match populated.selection {
								Member(id) if id == at_a.member.id => Member(at_b.member.id)
								Member(id) if id == at_b.member.id => Member(at_a.member.id)
								_ => populated.selection
							}
						swapped = all.map_with_index(
							|m, k| if k == i { ..m, window: b } else if k == j { ..m, window: a } else m,
						)
						(set_stack(stacks, layout, { ..held, members: rebuilt(swapped, chosen) }, "swap_windows"), True)
					}
				}
				_ => (stacks, False)
			}
		} else {
			(stacks, False)
		}

	## A tab moved to another's place, the others keeping their order.
	move_tab : Stacks, U64, StackTab -> (Stacks, Bool)
	move_tab = |stacks, layout, { window, target }|
		if takes_part(stacks, window) and takes_part(stacks, target) {
			held = stack_at(stacks, layout, "move_tab")
			all = listed(held)
			match (held.members, zip(all, |m| m.window == window), zip(all, |m| m.window == target)) {
				(Populated(populated), Found(at), Found(to)) => {
					j = to.before.len()
					if at.before.len() == j {
						(stacks, False)
					} else {
						parts = at.before.concat(at.after).split_at(j)
						order = parts.before.append(at.member).concat(parts.others)
						(set_stack(stacks, layout, { ..held, members: rebuilt(order, populated.selection) }, "move_tab"), True)
					}
				}
				_ => (stacks, False)
			}
		} else {
			(stacks, False)
		}

	# Members.

	## The member taking part next to the selected one along `direction`, if the stack runs that
	## way and a member (not the stack) is selected: the selected member's place, and the
	## neighbour's within the members on that side.
	neighbor : Stacks, StackTreeStack, BspDirection -> [Neighbor(Zipper, Zipper), NoNeighbor]
	neighbor = |stacks, held, direction|
		match (held.members, active(stacks, held)) {
			(Populated(populated), Found(at)) =>
				match populated.selection {
					Root(_) => NoNeighbor
					Member(_) =>
						if same_axis(held.orientation, direction) {
							found =
								if is_backwards(direction) {
									zip_last(at.before, |m| takes_part(stacks, m.window))
								} else {
									zip(at.after, |m| takes_part(stacks, m.window))
								}
							match found {
								Found(target) => Neighbor(at, target)
								NotFound => NoNeighbor
							}
						} else {
							NoNeighbor
						}
				}
			_ => NoNeighbor
		}

	## The selected member, or the first taking part when it does not.
	active : Stacks, StackTreeStack -> [Found(Zipper), NotFound]
	active = |stacks, held| {
		all = listed(held)
		chosen =
			match held.members {
				Populated(populated) => zip(all, |m| m.id == remembered(populated.selection) and takes_part(stacks, m.window))
				Empty => NotFound
			}
		match chosen {
			Found(at) => Found(at)
			NotFound => zip(all, |m| takes_part(stacks, m.window))
		}
	}

	## The member shown: the first taking part while the stack itself is selected.
	visible : Stacks, StackTreeStack -> [Found(Zipper), NotFound]
	visible = |stacks, held|
		match held.members {
			Populated(populated) =>
				match populated.selection {
					Root(_) => zip(listed(held), |m| takes_part(stacks, m.window))
					Member(_) => active(stacks, held)
				}
			Empty => NotFound
		}

	## The members `keep` keeps; a selected member that goes passes the selection to the next
	## kept member, else the last kept before it (the stack itself no longer selected).
	retained : StackTreeStack, (StackMember -> Bool) -> StackTreeStack
	retained = |held, keep|
		match held.members {
			Empty => held
			Populated(populated) => {
				all = listed(held)
				chosen =
					match zip(all, |m| m.id == remembered(populated.selection)) {
						Found(at) if !keep(at.member) =>
							match (at.after.keep_if(keep).first(), at.before.keep_if(keep).last()) {
								(Ok(next), _) => Member(next.id)
								(_, Ok(previous)) => Member(previous.id)
								# Nothing kept: the stack is empty, with no selection.
								_ => populated.selection
							}
						_ => populated.selection
					}
				{ ..held, members: rebuilt(all.keep_if(keep), chosen) }
			}
		}

	## Every member changed by `change`, the selection as it was.
	renamed : StackTreeStack, (StackMember -> StackMember) -> StackTreeStack
	renamed = |held, change|
		match held.members {
			Empty => held
			Populated(populated) => { ..held, members: Populated({ ..populated, first: change(populated.first), rest: populated.rest.map(change) }) }
		}

	## The stack with the member `at` found replaced by `member`, the selection as it was.
	refilled : StackTreeStack, Zipper, StackMember -> StackTreeStack
	refilled = |held, at, member|
		match held.members {
			Empty => held
			Populated(populated) => { ..held, members: rebuilt(at.before.append(member).concat(at.after), populated.selection) }
		}

	## The members in order (none for an empty stack).
	listed : StackTreeStack -> List(StackMember)
	listed = |held|
		match held.members {
			Empty => []
			Populated(populated) => populated.rest.prepend(populated.first)
		}

	## Members as a stack holds them: none, or the first and the rest with the selection.
	rebuilt : List(StackMember), StackSelection -> StackMembers
	rebuilt = |order, chosen|
		match order {
			[first, .. as rest] => Populated({ first, rest, selection: chosen })
			[] => Empty
		}

	## The member the selection rests on (the stack's memory while the stack is selected).
	remembered : StackSelection -> U64
	remembered = |chosen|
		match chosen {
			Member(id) => id
			Root(id) => id
		}

	same_selection : StackSelection, StackSelection -> Bool
	same_selection = |a, b|
		match (a, b) {
			(Member(x), Member(y)) => x == y
			(Root(x), Root(y)) => x == y
			_ => False
		}

	## The first member `pick` picks: those before it, it, and those after it.
	zip : List(StackMember), (StackMember -> Bool) -> [Found(Zipper), NotFound]
	zip = |members, pick| zip_from([], members, pick)

	zip_from : List(StackMember), List(StackMember), (StackMember -> Bool) -> [Found(Zipper), NotFound]
	zip_from = |before, rest, pick|
		match rest {
			[member, .. as after] => if pick(member) Found({ before, member, after }) else zip_from(before.append(member), after, pick)
			[] => NotFound
		}

	## The last member `pick` picks: those before it, it, and those after it.
	zip_last : List(StackMember), (StackMember -> Bool) -> [Found(Zipper), NotFound]
	zip_last = |members, pick| zip_back(members, [], pick)

	zip_back : List(StackMember), List(StackMember), (StackMember -> Bool) -> [Found(Zipper), NotFound]
	zip_back = |rest, after, pick|
		match rest {
			[.. as before, member] => if pick(member) Found({ before, member, after }) else zip_back(before, after.prepend(member), pick)
			[] => NotFound
		}

	## Every layout's stack changed by `change`.
	each_stack : Stacks, (StackTreeStack -> StackTreeStack) -> Stacks
	each_stack = |stacks, change| { ..stacks, layouts: Slots.map_values(stacks.layouts, change) }

	stack_at : Stacks, U64, Str -> StackTreeStack
	stack_at = |stacks, layout, op|
		match Slots.get(stacks.layouts, layout) {
			Held(held) => held
			NotHeld => {
				crash "StackTree.${op}: no layout ${layout.to_str()}"
			}
		}

	set_stack : Stacks, U64, StackTreeStack, Str -> Stacks
	set_stack = |stacks, layout, held, op|
		match Slots.replace(stacks.layouts, layout, held) {
			Held(layouts) => { ..stacks, layouts }
			NotHeld => {
				crash "StackTree.${op}: no layout ${layout.to_str()}"
			}
		}

	# Fills and directions.

	is_tiled : Fill -> Bool
	is_tiled = |fill|
		match fill {
			Tiled => True
			_ => False
		}

	## A fill toggled (upstream's `toggle_fullscreen_mode`): the mode asked for is turned off if
	## it is on, else on (replacing the other).
	toggled : Fill, BspFullscreen -> Fill
	toggled = |fill, mode|
		match (fill, mode) {
			(WithinGaps, WithinGaps) => Tiled
			(Screen, Screen) => Tiled
			(_, WithinGaps) => WithinGaps
			(_, Screen) => Screen
		}

	## A fill after a native resize (upstream's `track_fullscreen_resize`): onto the screen or
	## the tiling area takes that fill, off it leaves that fill.
	resized : Fill, BspNativeResize -> Fill
	resized = |fill, { old, new, screen, tiling, .. }|
		if same_rect(new, screen) {
			Screen
		} else if same_rect(old, screen) {
			match fill {
				Screen => Tiled
				_ => fill
			}
		} else if same_rect(new, tiling) {
			WithinGaps
		} else if same_rect(old, tiling) {
			match fill {
				WithinGaps => Tiled
				_ => fill
			}
		} else {
			fill
		}

	## Rust's `Rect` equality: each field equal as an `f64`.
	same_rect : Rect, Rect -> Bool
	same_rect = |a, b| a.x == b.x and a.y == b.y and a.w == b.w and a.h == b.h

	is_backwards : BspDirection -> Bool
	is_backwards = |direction|
		match direction {
			Left => True
			Up => True
			Right => False
			Down => False
		}

	axis_of : BspDirection -> BspOrientation
	axis_of = |direction|
		match direction {
			Left => Horizontal
			Right => Horizontal
			Up => Vertical
			Down => Vertical
		}

	same_axis : BspOrientation, BspDirection -> Bool
	same_axis = |orientation, direction|
		match (orientation, axis_of(direction)) {
			(Horizontal, Horizontal) => True
			(Vertical, Vertical) => True
			_ => False
		}
}
