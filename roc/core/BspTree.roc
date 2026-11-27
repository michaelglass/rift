## A BSP layout system's trees (`rift_core::layout::bsp_tree::Tree`), ported operation by
## operation: every layout's root in one slot table and every node in another, so keys are
## allocated in one sequence, as upstream's slotmap allocates them. Keys are slotmap's ffi form,
## one word each (`version << 32 | index`); windows are one word each.
##
## A node is looked up only by a key the tree holds (a layout's root, a split's child, a node's
## parent): a missing one is a broken tree, as it is in Rust (whose `Index` panics), so the
## lookups crash, naming what looked and the key.

import Geometry
import Slots
import pf.Boundary exposing [BspFramesQuestion, BspChildren, BspNode, BspShape, Constraints, WindowConstraints, WindowId, Aspect, BspTreeSeed, BspDrop, BspEditIn, BspEdited, BspFullscreen, BspResize, BspResizeAlong, BspFocusIn, BspFocused, BspAppIn, BspLayoutDirection, BspMoveQuery, BspSlot, BspWindowsQuery, BspContent, BspDirection, BspMembers, BspNativeResize, BspOrientation, BspReplacement, BspSide, BspTabs, BspTreeSplit, BspWindowIn, Fill, MaybeWord, Rect, BspInsertion, BspKind, BspLayoutSlot, BspNodeSlot, BspParent, BspParticipation, BspTakesPart, BspTreeAnswer, BspTreeEvent, BspTreeNode]

## A BSP layout system: every layout's root, every node, the windows left out of layout, and
## its settings (the width/height a layout's only window is fitted to is finite and positive).
Trees : {
	layouts : Slots(U64),
	nodes : Slots(BspTreeNode),
	left_out : Set(U64),
	insertion : BspInsertion,
	aspect : Aspect,
}

## The system as a view gives it: each table's slots and free list, the windows left out in
## order, the settings.
View : {
	layouts : List(BspLayoutSlot),
	layout_free : List(U32),
	nodes : List(BspNodeSlot),
	node_free : List(U32),
	left_out : List(U64),
	insertion : BspInsertion,
	aspect : Aspect,
}

BspTree := [].{

	empty : BspInsertion -> Trees
	empty = |insertion| {
		layouts: Slots.empty({}),
		nodes: Slots.empty({}),
		left_out: Set.empty(),
		insertion,
		aspect: Free,
	}

	## A system as the platform holds it (each table's slots, vacant ones included, and its free
	## list in taking order; the windows left out; the settings), as `Tree::from_parts` takes it.
	## The host parses it first (slotmap's refusals, the free list naming the vacant slots).
	seeded : BspTreeSeed -> Trees
	seeded = |seed| {
		layouts: Slots.from_parts(
			seed.layouts.map(
				|slot|
					match slot {
						Occupied({ version, root }) => Occupied({ version, value: root })
						Vacant(version) => Vacant(version)
					},
			),
			seed.layout_free,
		),
		nodes: Slots.from_parts(
			seed.nodes.map(
				|slot|
					match slot {
						Occupied({ version, node: held }) => Occupied({ version, value: held })
						Vacant(version) => Vacant(version)
					},
			),
			seed.node_free,
		),
		left_out: Set.from_list(seed.left_out),
		insertion: seed.insertion,
		aspect: seed.aspect,
	}

	## One operation, and what it answers.
	step : Trees, BspTreeEvent -> (Trees, BspTreeAnswer)
	step = |trees, event|
		match event {
			CreateLayout => {
				(after, layout) = create_layout(trees)
				(after, Layout(layout))
			}
			CloneLayout(layout) => {
				(after, clone) = clone_layout(trees, layout)
				(after, Layout(clone))
			}
			RemoveLayout(layout) => (remove_layout(trees, layout), Done)
			Participation(participation) => {
				(after, changed) = set_window_participation(trees, participation)
				(after, Participated(changed))
			}
			Insertion(insertion) => ({ ..trees, insertion }, Done)
			AspectRatio(aspect) => ({ ..trees, aspect: finite_positive(aspect) }, Done)
			AddWindow(added) => (add_window(trees, added), Done)
			RemoveWindow(window) => (remove_window(trees, window), Done)
			ReplaceWindow(replacement) => (replace_window(trees, replacement), Done)
			CloseApp(windows) => (close_app(trees, windows), Done)
			ResizeWindow(resize) => (resize_window(trees, resize), Done)
			ContainsLayout(layout) =>
				match Slots.get(trees.layouts, layout) {
					Held(_) => (trees, Verdict(True))
					NotHeld => (trees, Verdict(False))
				}
			WindowParticipates(window) => (trees, Verdict(!trees.left_out.contains(window)))
			SelectedWindow(layout) => (trees, Window(selected_window(trees, layout)))
			WindowInDirection(asked) => (trees, Window(window_in_direction(trees, asked)))
			ContainsWindow({ layout, window }) => (trees, Verdict(contains_window(trees, layout, window)))
			HasAnyFullscreenNode(layout) => (trees, Verdict(has_any_fullscreen_node(trees, layout)))
			HasWindowsForApp(asked) => (trees, Verdict(has_windows_for_app(trees, asked)))
			Windows(asked) => (trees, Windows(listed_windows(trees, asked)))
			CanMoveWindow(asked) => (trees, Verdict(can_move_window(trees, asked)))
			WindowSlot(asked) => (trees, WindowSlot(window_slot(trees, asked)))
			Edit(asked) => {
				(after, edited) = edit(trees, asked)
				(after, Edited(edited))
			}
			Focus(asked) => {
				(after, focused) = focus(trees, asked)
				(after, Focused(focused))
			}
		}

	view : Trees -> View
	view = |trees| {
		layouts: Slots.slot_list(trees.layouts).map(
			|slot|
				match slot {
					Occupied({ version, value }) => Occupied({ version, root: value })
					Vacant(version) => Vacant(version)
				},
		),
		layout_free: Slots.free_indices(trees.layouts),
		nodes: Slots.slot_list(trees.nodes).map(
			|slot|
				match slot {
					Occupied({ version, value }) => Occupied({ version, node: value })
					Vacant(version) => Vacant(version)
				},
		),
		node_free: Slots.free_indices(trees.nodes),
		left_out: trees.left_out.to_list().sort_with(|a, b| a.order_relative_to(b)),
		insertion: trees.insertion,
		aspect: trees.aspect,
	}

	# The shape (`Tree::shape`): the geometry's input.

	## A layout's shape for the BSP geometry: its windows taking part, each with the constraints
	## the host knows, pruned as the Rust tree prunes; none when the layout is unknown or nothing
	## takes part. The tree's window words become the geometry's `WindowId`s here.
	shape : Trees, BspFramesQuestion -> [Shaped(BspShape), Unshaped]
	shape = |trees, question| {
		known = Dict.from_list(question.known.map(|k| (k.window, k.constraints)))
		match Slots.get(trees.layouts, question.layout) {
			Held(root) =>
				match shape_node(trees, known, root) {
					Shaped(top) => Shaped({ screen: question.screen, gaps: question.gaps, aspect: trees.aspect, root: top })
					Pruned => Unshaped
				}
			NotHeld => Unshaped
		}
	}

	## A node's shape: a leaf's window taking part (or an empty leaf, while no window is left
	## out), a split with the children that have one; pruned when none is left.
	shape_node : Trees, Dict(U64, WindowConstraints), U64 -> [Shaped(BspNode), Pruned]
	shape_node = |trees, known, key|
		match kind(trees, key) {
			Leaf(leaf) => {
				placed =
					match participating_member(trees, leaf.content) {
						Some(word) => Placed(Window({ window: Geometry.window_id(word), constraints: Geometry.known_constraints(known, word) }))
						Nothing =>
							match leaf.content {
								Empty if trees.left_out.is_empty() => Placed(Empty)
								_ => Unplaced
							}
					}
				match placed {
					Placed(slot) => {
						(fill, stacked) =
							match leaf.content {
								Empty => (Tiled, False)
								Held({ members, fill: held_fill }) =>
									match members {
										Window(_) => (held_fill, False)
										Stack(_) => (held_fill, True)
									}
							}
						Shaped(BspNode.Leaf({ slot, fill, stacked }))
					}
					Unplaced => Pruned
				}
			}
			Split(split) => {
				sides =
					match (shape_node(trees, known, split.first), shape_node(trees, known, split.second)) {
						(Pruned, Pruned) => NoChildren
						(Shaped(only), Pruned) => Children(BspChildren.One(only))
						(Pruned, Shaped(only)) => Children(BspChildren.One(only))
						(Shaped(first), Shaped(second)) => Children(BspChildren.Two(first, second))
					}
				match sides {
					Children(shaped) => Shaped(BspNode.Split({ horizontal: same_orientation(split.orientation, Horizontal), ratio: split.ratio, children: shaped }))
					NoChildren => Pruned
				}
			}
		}


	# Layouts, participation and the settings.

	## The ratio, if finite and positive (`set_single_window_aspect_ratio`'s filter).
	finite_positive : Aspect -> Aspect
	finite_positive = |aspect|
		match aspect {
			Aspect(ratio) => if (ratio.is_finite() and ratio > 0.0) Aspect(ratio) else Free
			Free => Free
		}

	## Whether the window's participation changed.
	set_window_participation : Trees, BspParticipation -> (Trees, Bool)
	set_window_participation = |trees, { window, participation }| {
		held = trees.left_out.contains(window)
		match participation {
			TakesPart => ({ ..trees, left_out: trees.left_out.remove(window) }, held)
			LeftOut => ({ ..trees, left_out: trees.left_out.insert(window) }, !held)
		}
	}

	create_layout : Trees -> (Trees, U64)
	create_layout = |trees| {
		(with_leaf, leaf) = make_leaf(trees, Empty)
		(layouts, layout) = Slots.insert(with_leaf.layouts, leaf)
		({ ..with_leaf, layouts }, layout)
	}

	## A copy of the layout's tree under new keys (a new layout when there is none).
	clone_layout : Trees, U64 -> (Trees, U64)
	clone_layout = |trees, layout|
		match Slots.get(trees.layouts, layout) {
			Held(root) => {
				(cloned, clone) = clone_node(trees, root, Root)
				(layouts, key) = Slots.insert(cloned.layouts, clone)
				({ ..cloned, layouts }, key)
			}
			NotHeld => create_layout(trees)
		}

	remove_layout : Trees, U64 -> Trees
	remove_layout = |trees, layout| {
		(layouts, took) = Slots.remove(trees.layouts, layout)
		match took {
			Took(root) => remove_subtree({ ..trees, layouts }, root)
			NotHeld => trees
		}
	}

	# Window changes.

	## A window added to a layout: after the selection, split toward a preselected direction, or
	## at the end of the tree (upstream's `add_window_after_selection`).
	add_window : Trees, BspWindowIn -> Trees
	add_window = |trees, { layout, window }|
		if contains_window(trees, layout, window) {
			trees
		} else {
			match Slots.get(trees.layouts, layout) {
				NotHeld => trees
				Held(root) =>
					match trees.insertion {
						EndOfTree => insert_window_at_selection(select(trees, last_leaf(trees, root)), layout, window)
						NextToSelection =>
							match smart_insert_window(trees, layout, window) {
								Inserted(inserted) => inserted
								NotPreselected => insert_window_at_selection(trees, layout, window)
							}
					}
			}
		}

	## The leaf reached by always taking a split's second child.
	last_leaf : Trees, U64 -> U64
	last_leaf = |trees, key|
		match children(trees, key) {
			Children(_, second) => last_leaf(trees, second)
			Childless => key
		}

	## A window gone from every layout.
	remove_window : Trees, U64 -> Trees
	remove_window = |trees, window| {
		without = { ..trees, left_out: trees.left_out.remove(window) }
		layout_keys(without).fold(without, |acc, layout| remove_every(acc, layout, window))
	}

	## Every occurrence of `window` removed from the layout (legacy duplicates included).
	remove_every : Trees, U64, U64 -> Trees
	remove_every = |trees, layout, window|
		match remove_window_internal(trees, layout, window) {
			Removed(after) => remove_every(after, layout, window)
			Absent => trees
		}

	## A window that took another's place (a native tab, a restore's match), in every layout.
	replace_window : Trees, BspReplacement -> Trees
	replace_window = |trees, { from, to }| {
		left_out = if (trees.left_out.contains(from)) trees.left_out.remove(from).insert(to) else trees.left_out
		renamed = { ..trees, left_out }
		if from == to {
			renamed
		} else {
			# A replacement may already be present after restore matching.
			cleared = layout_keys(renamed).fold(
				renamed,
				|acc, layout| if (contains_window(acc, layout, from)) remove_every(acc, layout, to) else acc,
			)
			slots = Slots.slot_list(cleared.nodes).map(
				|slot|
					match slot {
						Occupied({ version, value: { parent: placed, kind: Leaf({ content: Held(held), preselect }) } }) =>
							Occupied({ version, value: { parent: placed, kind: Leaf({ content: Held({ ..held, members: replaced(held.members, from, to) }), preselect }) } })
						_ => slot
					},
			)
			{ ..cleared, nodes: { ..cleared.nodes, slots } }
		}
	}

	## An app gone: its windows (those listed) leave every layout, in window order.
	close_app : Trees, List(U64) -> Trees
	close_app = |trees, of_app| {
		kept = trees.left_out.to_list().keep_if(|window| !of_app.contains(window))
		retained = { ..trees, left_out: Set.from_list(kept) }
		gone =
			layout_roots(retained)
			.join_map(|root| windows_under(retained, root))
			.keep_if(|window| of_app.contains(window))
			.sort_with(|a, b| a.order_relative_to(b))
		dedup(gone).fold(retained, |acc, window| remove_window(acc, window))
	}

	## A window the user (or its app) resized from `old` to `new`: a frame that enters or leaves
	## the screen or the tiling area sets or clears its fill; otherwise the nearest split along a
	## changed axis takes the new share.
	resize_window : Trees, BspNativeResize -> Trees
	resize_window = |trees, resize|
		if trees.left_out.contains(resize.window) {
			trees
		} else {
			match node_for_window_in_layout(trees, resize.layout, resize.window) {
				NotFound => trees
				Found(leaf) =>
					match content(trees, leaf) {
						Content(Held(held)) =>
							match track_fullscreen_resize(held.fill, resize) {
								Tracked(fill) => set_content(trees, leaf, Held({ ..held, fill }), "resize_window")
								Untracked => resize_share(trees, leaf, resize)
							}
						_ => resize_share(trees, leaf, resize)
					}
			}
		}

	## The nearest split along a changed axis takes the new share (none when neither axis
	## changed by more than half a point).
	resize_share : Trees, U64, BspNativeResize -> Trees
	resize_share = |trees, leaf, { old, new, .. }| {
		width_changed = (new.w - old.w).abs() > 0.5
		height_changed = (new.h - old.h).abs() > 0.5
		if width_changed or height_changed {
			reshare(trees, leaf, { old, new, width_changed, height_changed })
		} else {
			trees
		}
	}

	reshare : Trees, U64, { old : Rect, new : Rect, width_changed : Bool, height_changed : Bool } -> Trees
	reshare = |trees, current, change|
		match parent(trees, current) {
			NoParent => trees
			Parent(above) => {
				split = split_of(trees, above, "resize_window")
				lengths =
					match split.orientation {
						Horizontal => if change.width_changed Lengths(change.old.w, change.new.w) else Unchanged
						Vertical => if change.height_changed Lengths(change.old.h, change.new.h) else Unchanged
					}
				match lengths {
					Unchanged => reshare(trees, above, change)
					Lengths(old_len, new_len) => {
						current_ratio = split.ratio.to_f64()
						is_first = current == split.first
						denom = if is_first current_ratio else 1.0 - current_ratio
						if old_len <= 0.0 or denom <= epsilon {
							trees
						} else {
							parent_len = old_len / denom
							if parent_len <= 0.0 {
								trees
							} else {
								raw = clamp_f64(new_len / parent_len, 0.05, 0.95)
								ratio = if is_first Geometry.narrow(raw) else Geometry.narrow(1.0 - raw)
								set_kind(trees, above, Split({ ..split, ratio }), "resize_window")
							}
						}
					}
				}
			}
		}

	# Queries.

	## The window the layout's selection shows, if one takes part.
	selected_window : Trees, U64 -> MaybeWord
	selected_window = |trees, layout|
		match Slots.get(trees.layouts, layout) {
			Held(root) => participating_window(trees, current_selection(trees, root))
			NotHeld => Nothing
		}

	## The first window taking part met going `direction` from the root: at each split, the
	## side that way first.
	window_in_direction : Trees, BspLayoutDirection -> MaybeWord
	window_in_direction = |trees, { layout, direction }|
		match Slots.get(trees.layouts, layout) {
			Held(root) => window_in_direction_from(trees, root, direction)
			NotHeld => Nothing
		}

	window_in_direction_from : Trees, U64, BspDirection -> MaybeWord
	window_in_direction_from = |trees, key, direction|
		match children(trees, key) {
			Childless => participating_window(trees, key)
			Children(first, second) => {
				(nearer, farther) =
					match side_of(direction) {
						First => (second, first)
						Second => (first, second)
					}
				match window_in_direction_from(trees, nearer, direction) {
					Some(window) => Some(window)
					Nothing => window_in_direction_from(trees, farther, direction)
				}
			}
		}

	## Whether a leaf of the layout holding a window that takes part fills the screen or the
	## screen within its gaps.
	has_any_fullscreen_node : Trees, U64 -> Bool
	has_any_fullscreen_node = |trees, layout|
		match Slots.get(trees.layouts, layout) {
			Held(root) => has_fullscreen_in_subtree(trees, root)
			NotHeld => False
		}

	has_fullscreen_in_subtree : Trees, U64 -> Bool
	has_fullscreen_in_subtree = |trees, key|
		match kind(trees, key) {
			Leaf({ content: Held({ fill: Screen, .. }), .. }) | Leaf({ content: Held({ fill: WithinGaps, .. }), .. }) => takes_part(trees, key)
			Leaf(_) => False
			Split({ first, second, .. }) => has_fullscreen_in_subtree(trees, first) or has_fullscreen_in_subtree(trees, second)
		}

	## Whether the layout holds one of the app's windows (those listed).
	has_windows_for_app : Trees, BspAppIn -> Bool
	has_windows_for_app = |trees, { layout, windows }|
		match Slots.get(trees.layouts, layout) {
			Held(root) => windows_under(trees, root).any(|window| windows.contains(window))
			NotHeld => False
		}

	## The layout's windows a query lists (upstream's `WindowQuery`).
	listed_windows : Trees, BspWindowsQuery -> List(U64)
	listed_windows = |trees, { layout, query }|
		match Slots.get(trees.layouts, layout) {
			NotHeld => []
			Held(root) =>
				match query {
					All => windows_under(trees, root)
					Visible =>
						preorder(trees, root).join_map(
							|at|
								match participating_window(trees, at) {
									Some(window) => [window]
									Nothing => []
								},
						)
					Selection =>
						match selection_of_layout(trees, layout) {
							Selection(selected) =>
								match participating_window(trees, descend_to_leaf(trees, selected)) {
									Some(window) => [window]
									Nothing => []
								}
							NoSelection => []
						}
					Stack(window) =>
						match node_for_window_in_layout(trees, layout, window) {
							Found(leaf) =>
								match content(trees, leaf) {
									Content(Held({ members: Stack(tabs), .. })) => tabs_windows(tabs)
									_ => []
								}
							NotFound => []
						}
				}
		}

	## Whether the window takes part and has a neighbour leaf that way.
	can_move_window : Trees, BspMoveQuery -> Bool
	can_move_window = |trees, { layout, window, direction }|
		if trees.left_out.contains(window) {
			False
		} else {
			match node_for_window_in_layout(trees, layout, window) {
				Found(leaf) =>
					match find_neighbor_leaf(trees, leaf, direction) {
						Neighbor(_) => True
						NoNeighbor => False
					}
				NotFound => False
			}
		}

	## The window's place: the side taken at each split from the root (0 first, 1 second),
	## then its position in its leaf's stack, if it is stacked.
	window_slot : Trees, BspWindowIn -> BspSlot
	window_slot = |trees, { layout, window }|
		match node_for_window_in_layout(trees, layout, window) {
			NotFound => NoSlot
			Found(leaf) => {
				sides = ancestors(trees, leaf).drop_last(1).map(
					|at|
						match parent(trees, at) {
							Parent(above) => if split_of(trees, above, "window_slot").second == at 1 else 0
							NoParent => {
								crash "BspTree.window_slot: node ${at.to_str()} below its root has no parent"
							}
						},
				)
				path = sides.rev()
				match content(trees, leaf) {
					Content(Held({ members: Stack(tabs), .. })) =>
						match tabs_windows(tabs).find_first_index(|member| member == window) {
							Ok(position) => Slot(path.append(position))
							Err(_) => {
								crash "BspTree.window_slot: leaf ${leaf.to_str()} holds no ${window.to_str()}"
							}
						}
					_ => Slot(path)
				}
			}
		}

	## The nearest leaf `direction` of `from` holding a window that takes part (any leaf while
	## none is left out).
	find_neighbor_leaf : Trees, U64, BspDirection -> [Neighbor(U64), NoNeighbor]
	find_neighbor_leaf = |trees, from, direction|
		match find_structural_neighbor_leaf(trees, from, direction) {
			Neighbor(next) =>
				if trees.left_out.is_empty() or takes_part(trees, next) {
					Neighbor(next)
				} else {
					find_neighbor_leaf(trees, next, direction)
				}
			NoNeighbor => NoNeighbor
		}

	## The leaf across the nearest split along `direction`'s axis, entered at the point level
	## with `from`'s middle (the anchor, a share of the span across the axis).
	find_structural_neighbor_leaf : Trees, U64, BspDirection -> [Neighbor(U64), NoNeighbor]
	find_structural_neighbor_leaf = |trees, from, direction| {
		backwards = is_backwards(direction)
		axis = orientation_of(direction)
		climb(trees, from, 0.5, backwards, axis)
	}

	climb : Trees, U64, F64, Bool, BspOrientation -> [Neighbor(U64), NoNeighbor]
	climb = |trees, current, anchor, backwards, axis|
		match parent(trees, current) {
			NoParent => NoNeighbor
			Parent(above) => {
				split = split_of(trees, above, "find_structural_neighbor_leaf")
				second = split.second == current
				if same_orientation(split.orientation, axis) {
					if second == backwards {
						Neighbor(enter(trees, if second split.first else split.second, anchor, backwards, axis))
					} else {
						climb(trees, above, anchor, backwards, axis)
					}
				} else {
					ratio = split.ratio.to_f64()
					across = if second ratio + anchor * (1.0 - ratio) else anchor * ratio
					climb(trees, above, across, backwards, axis)
				}
			}
		}

	## Down from `key` to a leaf: along the axis, the side nearest where the move came from;
	## across it, the side holding the anchor.
	enter : Trees, U64, F64, Bool, BspOrientation -> U64
	enter = |trees, key, anchor, backwards, axis|
		match kind(trees, key) {
			Leaf(_) => key
			Split(split) =>
				if same_orientation(split.orientation, axis) {
					enter(trees, if backwards split.second else split.first, anchor, backwards, axis)
				} else {
					ratio = split.ratio.to_f64()
					if anchor <= ratio {
						enter(trees, split.first, anchor / ratio, backwards, axis)
					} else {
						enter(trees, split.second, (anchor - ratio) / (1.0 - ratio), backwards, axis)
					}
				}
		}

	## Whether the direction goes toward a split's first child.
	is_backwards : BspDirection -> Bool
	is_backwards = |direction|
		match direction {
			Left | Up => True
			Right | Down => False
		}

	same_orientation : BspOrientation, BspOrientation -> Bool
	same_orientation = |a, b|
		match (a, b) {
			(Horizontal, Horizontal) | (Vertical, Vertical) => True
			(Horizontal, Vertical) | (Vertical, Horizontal) => False
		}

	# Focus.

	## The selection moved (upstream's `focus`): a selection change says whether it was
	## handled; a movement names the window it reached, if any, and those to raise.
	focus : Trees, BspFocusIn -> (Trees, BspFocused)
	focus = |trees, { layout, focus: moved }|
		match moved {
			Select(window) => {
				(after, handled) = select_window(trees, layout, window)
				(after, Selection(handled))
			}
			Ascend => {
				(after, handled) = ascend(trees, layout)
				(after, Selection(handled))
			}
			Descend => {
				(after, handled) = descend(trees, layout)
				(after, Selection(handled))
			}
			Direction(direction) => move_focus(trees, layout, direction)
		}

	## The window's leaf selected, the window on top of its stack (none: it is left out or
	## not in the layout).
	select_window : Trees, U64, U64 -> (Trees, Bool)
	select_window = |trees, layout, window|
		if trees.left_out.contains(window) {
			(trees, False)
		} else {
			match node_for_window_in_layout(trees, layout, window) {
				NotFound => (trees, False)
				Found(leaf) => {
					focused =
						match content_of(trees, leaf, "select_window") {
							Held(held) => set_content(trees, leaf, Held({ ..held, members: on_top(held.members, window) }), "select_window")
							Empty => trees
						}
					(select(focused, leaf), True)
				}
			}
		}

	## The leaf standing for the selection's parent selected.
	ascend : Trees, U64 -> (Trees, Bool)
	ascend = |trees, layout|
		match selection_of_layout(trees, layout) {
			Selection(selected) =>
				match parent(trees, selected) {
					Parent(above) => (select(trees, descend_to_leaf(trees, above)), True)
					NoParent => (trees, False)
				}
			NoSelection => (trees, False)
		}

	## The leaf standing for the selection selected, if that is another node.
	descend : Trees, U64 -> (Trees, Bool)
	descend = |trees, layout|
		match selection_of_layout(trees, layout) {
			Selection(selected) => {
				leaf = descend_to_leaf(trees, selected)
				if leaf != selected (select(trees, leaf), True) else (trees, False)
			}
			NoSelection => (trees, False)
		}

	## The neighbour leaf `direction` of the selection selected; its window, if one takes
	## part, is reached and raised (tiled BSP windows never overlap, so only it is raised).
	move_focus : Trees, U64, BspDirection -> (Trees, BspFocused)
	move_focus = |trees, layout, direction| {
		nowhere = Movement({ window: Nothing, raise: [] })
		match selection_of_layout(trees, layout) {
			NoSelection => (trees, nowhere)
			Selection(selected) =>
				match find_neighbor_leaf(trees, descend_to_leaf(trees, selected), direction) {
					NoNeighbor => (trees, nowhere)
					Neighbor(next) => {
						window = participating_window(trees, next)
						raise =
							match window {
								Some(reached) => [reached]
								Nothing => []
							}
						(select(trees, next), Movement({ window, raise }))
					}
				}
		}
	}

	## `window` on top: a stack moves it to the end, the others keeping their order.
	on_top : BspMembers, U64 -> BspMembers
	on_top = |members, window|
		match members {
			Window(_) => members
			Stack(tabs) => Stack({ before: tabs_windows(tabs).keep_if(|member| member != window), top: window, after: [] })
		}

	# Edits (upstream's `edit_layout`).

	## An edit of a layout, and what it did.
	edit : Trees, BspEditIn -> (Trees, BspEdited)
	edit = |trees, { layout, edit: asked }|
		match asked {
			Move(direction) => edited_by(move_selection(trees, layout, direction))
			Swap({ a, b }) => edited_by(swap_windows(trees, layout, a, b))
			Drop(dropped) => edited_by(apply_target_drop(trees, layout, dropped))
			Join(direction) => edited_by(join_selection_with_direction(trees, layout, direction))
			ConsumeOrExpel(direction) => edited_by(consume_or_expel(trees, layout, direction))
			Unjoin => edited_by(unjoin_selection(trees, layout))
			Split(Stacked) => (trees, Unsupported)
			Split(Tiled(orientation)) =>
				match selection_of_layout(trees, layout) {
					NoSelection => (trees, Unchanged)
					Selection(selected) => (split_leaf(trees, descend_to_leaf(trees, selected), orientation, Empty, Second), Changed)
				}
			Fullscreen(mode) => {
				(after, raised) = toggle_fullscreen(trees, layout, mode)
				(after, Raise(raised))
			}
			Resize(resize) => resize_selection(trees, layout, resize)
			Rebalance =>
				match Slots.get(trees.layouts, layout) {
					Held(root) => {
						(after, _) = rebalance_node(trees, root, Horizontal)
						(after, Changed)
					}
					NotHeld => (trees, Changed)
				}
			ToggleOrientation =>
				match selection_of_layout(trees, layout) {
					NoSelection => (trees, Unchanged)
					Selection(start) =>
						match ancestors(trees, start).find_first(|at| is_split(trees, at)) {
							Ok(found) => {
								split = split_of(trees, found, "toggle_orientation")
								turned =
									match split.orientation {
										Horizontal => Vertical
										Vertical => Horizontal
									}
								(set_kind(trees, found, Split({ ..split, orientation: turned }), "toggle_orientation"), Changed)
							}
							Err(_) => (trees, Unchanged)
						}
				}
		}

	## The nearest split above the selection along `along` (any, smart) moves its share by half
	## the amount toward the selection's side, kept within 0.05..0.95 (in `f32`s, as upstream).
	resize_selection : Trees, U64, BspResize -> (Trees, BspEdited)
	resize_selection = |trees, layout, { amount, along }|
		match selection_of_layout(trees, layout) {
			NoSelection => (trees, Unchanged)
			Selection(selected) => resize_from(trees, selected, Geometry.narrow(amount), along)
		}

	resize_from : Trees, U64, F32, BspResizeAlong -> (Trees, BspEdited)
	resize_from = |trees, current, amount, along|
		match parent(trees, current) {
			NoParent => (trees, Unchanged)
			Parent(above) => {
				split = split_of(trees, above, "resize_selection")
				matches =
					match (along, split.orientation) {
						(Smart, _) | (Horizontal, Horizontal) | (Vertical, Vertical) => True
						(Horizontal, Vertical) | (Vertical, Horizontal) => False
					}
				if matches {
					half : F32
					half = if current == split.first 0.5 else -0.5
					next = clamp_f32(split.ratio + amount * half, 0.05, 0.95)
					(set_kind(trees, above, Split({ ..split, ratio: next }), "resize_selection"), if split.ratio != next Changed else Unchanged)
				} else {
					resize_from(trees, above, amount, along)
				}
			}
		}

	## Each split's share made its first side's tiles over all its tiles, where a tile is a leaf
	## or a split across its parent's axis; the subtree's tiles along `parent_axis`.
	rebalance_node : Trees, U64, BspOrientation -> (Trees, U64)
	rebalance_node = |trees, key, parent_axis|
		match kind(trees, key) {
			Leaf(_) => (trees, 1)
			Split(split) => {
				(with_first, first_span) = rebalance_node(trees, split.first, split.orientation)
				(with_second, second_span) = rebalance_node(with_first, split.second, split.orientation)
				span = first_span + second_span
				ratio = clamp_f32(first_span.to_f32() / span.to_f32(), 0.05, 0.95)
				balanced = set_kind(with_second, key, Split({ ..split_of(with_second, key, "rebalance_node"), ratio }), "rebalance_node")
				(balanced, if same_orientation(split.orientation, parent_axis) span else 1)
			}
		}

	## The selected leaf's fill toggled; its window taking part is raised.
	toggle_fullscreen : Trees, U64, BspFullscreen -> (Trees, List(U64))
	toggle_fullscreen = |trees, layout, mode|
		match selection_of_layout(trees, layout) {
			NoSelection => (trees, [])
			Selection(selected) => {
				leaf = descend_to_leaf(trees, selected)
				match content_of(trees, leaf, "toggle_fullscreen") {
					Empty => (trees, [])
					Held(held) => {
						raised =
							match participating_window(trees, leaf) {
								Some(window) => [window]
								Nothing => []
							}
						(set_content(trees, leaf, Held({ ..held, fill: toggled(held.fill, mode) }), "toggle_fullscreen"), raised)
					}
				}
			}
		}

	## A fill toggled (upstream's `toggle_fullscreen_mode`): the mode asked for is turned off if
	## it is on, else on (replacing the other).
	toggled : Fill, BspFullscreen -> Fill
	toggled = |fill, mode|
		match (fill, mode) {
			(WithinGaps, WithinGaps) | (Screen, Screen) => Tiled
			(_, WithinGaps) => WithinGaps
			(_, Screen) => Screen
		}

	## An edit's answer by whether it changed the layout.
	edited_by : (Trees, Bool) -> (Trees, BspEdited)
	edited_by = |(after, did)| (after, if did Changed else Unchanged)

	is_split : Trees, U64 -> Bool
	is_split = |trees, key|
		match kind(trees, key) {
			Split(_) => True
			Leaf(_) => False
		}

	## Rust's `f32::clamp`.
	clamp_f32 : F32, F32, F32 -> F32
	clamp_f32 = |value, low, high| if value < low low else if value > high high else value

	# Moves, swaps and drops.

	## The selected leaf moved `direction`: beside its neighbour in their split, into an empty
	## neighbour's place, or into a new split with a held one.
	move_selection : Trees, U64, BspDirection -> (Trees, Bool)
	move_selection = |trees, layout, direction|
		match selection_of_layout(trees, layout) {
			NoSelection => (trees, False)
			Selection(selected) => {
				moving = descend_to_leaf(trees, selected)
				match (content(trees, moving), find_neighbor_leaf(trees, moving, direction), parent(trees, moving)) {
					(Content(Held(_)), Neighbor(target), Parent(old_parent)) => {
						side = side_of(direction)
						moved =
							match parent(trees, target) {
								Parent(target_parent) if target_parent == old_parent => {
									split = split_of(trees, old_parent, "move_selection")
									moving_first = split.first == moving
									going_first =
										match side {
											First => True
											Second => False
										}
									if moving_first != going_first {
										set_kind(trees, old_parent, Split({ ..split, first: split.second, second: split.first }), "move_selection")
									} else {
										trees
									}
								}
								_ => {
									detached = detach_child(trees, moving)
									match content(detached, target) {
										Content(Empty) => {
											placed = replace_child(detached, target, moving)
											(nodes, _) = Slots.remove(placed.nodes, target)
											{ ..placed, nodes }
										}
										_ => {
											(with_split, made) = make_leaf(detached, Empty)
											pair =
												match side {
													First => (moving, target)
													Second => (target, moving)
												}
											set_split(replace_child(with_split, target, made), made, orientation_of(direction), pair)
										}
									}
								}
							}
						(select(moved, moving), True)
					}
					_ => (trees, False)
				}
			}
		}

	## `new` in `old`'s place in its split; `old` is not a root.
	replace_child : Trees, U64, U64 -> Trees
	replace_child = |trees, old, new| {
		above =
			match parent(trees, old) {
				Parent(found) => found
				NoParent => {
					crash "BspTree.replace_child: node ${old.to_str()} is a root"
				}
			}
		split = split_of(trees, above, "replace_child")
		swapped_in = if split.second == old { ..split, second: new } else { ..split, first: new }
		set_parent(set_kind(trees, above, Split(swapped_in), "replace_child"), new, Split(above))
	}

	## Two windows taking part swapped: in one stack, both moved to its end (the first on top);
	## across leaves, their windows trade places and each leaf keeps its fill.
	swap_windows : Trees, U64, U64, U64 -> (Trees, Bool)
	swap_windows = |trees, layout, a, b|
		if a == b or trees.left_out.contains(a) or trees.left_out.contains(b) {
			(trees, False)
		} else {
			match (node_for_window_in_layout(trees, layout, a), node_for_window_in_layout(trees, layout, b)) {
				(Found(node_a), Found(node_b)) =>
					if node_a == node_b {
						match content_of(trees, node_a, "swap_windows") {
							Held({ members: Stack(tabs), fill }) => {
								before = tabs_windows(tabs).keep_if(|window| window != a and window != b).append(b)
								swapped = set_content(trees, node_a, Held({ members: Stack({ before, top: a, after: [] }), fill }), "swap_windows")
								(select(swapped, node_a), True)
							}
							_ => {
								crash "BspTree.swap_windows: leaf ${node_a.to_str()} holds two windows and no stack"
							}
						}
					} else {
						match (content_of(trees, node_a, "swap_windows"), content_of(trees, node_b, "swap_windows")) {
							(Held(first), Held(second)) => {
								traded = set_content(trees, node_a, Held({ members: for_slot_swap(second.members), fill: first.fill }), "swap_windows")
								(set_content(traded, node_b, Held({ members: for_slot_swap(first.members), fill: second.fill }), "swap_windows"), True)
							}
							_ => {
								crash "BspTree.swap_windows: a window's leaf holds none"
							}
						}
					}
				_ => (trees, False)
			}
		}

	## A window dropped onto another (upstream's `apply_target_drop`): swapped with it, stacked
	## onto it, or inserted beside it; both must take part and be in the layout.
	apply_target_drop : Trees, U64, BspDrop -> (Trees, Bool)
	apply_target_drop = |trees, layout, { source, target, action }|
		if trees.left_out.contains(source) or trees.left_out.contains(target) or source == target or !contains_window(trees, layout, source) or !contains_window(trees, layout, target) {
			(trees, False)
		} else {
			match action {
				Swap => swap_windows(trees, layout, source, target)
				Move(_) | ConsumeOrExpel(_) => (trees, False)
				Stack => {
					(taken, onto) = taken_from_beside(trees, layout, source, target)
					stacked =
						match content_of(taken, onto, "apply_target_drop") {
							Held(held) => set_content(taken, onto, Held({ ..held, members: appended(held.members, source) }), "apply_target_drop")
							Empty => taken
						}
					(select(stacked, onto), True)
				}
				Insert(direction) => {
					(taken, beside) = taken_from_beside(trees, layout, source, target)
					(split_leaf(taken, beside, orientation_of(direction), lone(source), side_of(direction)), True)
				}
			}
		}

	## `source` removed from the layout, and the leaf now holding `target` (removal can promote
	## it into another slot).
	taken_from_beside : Trees, U64, U64, U64 -> (Trees, U64)
	taken_from_beside = |trees, layout, source, target| {
		taken = remove_every(trees, layout, source)
		match node_for_window_in_layout(taken, layout, target) {
			Found(leaf) => (taken, leaf)
			NotFound => {
				crash "BspTree.taken_from_beside: removing ${source.to_str()} lost ${target.to_str()}"
			}
		}
	}

	## `window` added to the end of the stack, on top (a lone window becomes a stack of two).
	appended : BspMembers, U64 -> BspMembers
	appended = |members, window| Stack({ before: members_windows(members), top: window, after: [] })

	## The windows as they move to another slot: a stack shows its last window, a stack of one is
	## a lone window.
	for_slot_swap : BspMembers -> BspMembers
	for_slot_swap = |members|
		match members {
			Window(_) => members
			Stack({ before, top, after }) => {
				tabs =
					match after.last() {
						Ok(last) => { before: before.append(top).concat(after.drop_last(1)), top: last, after: [] }
						Err(_) => { before, top, after }
					}
				if tabs.before.is_empty() Window(tabs.top) else Stack(tabs)
			}
		}

	# Joins.

	## The selection joined with its neighbour `direction`: onto the neighbour's stack, or, when
	## the two share a split below the root, that split's windows as one stack (the selected one
	## on top).
	join_selection_with_direction : Trees, U64, BspDirection -> (Trees, Bool)
	join_selection_with_direction = |trees, layout, direction|
		match selection_of_layout(trees, layout) {
			NoSelection => (trees, False)
			Selection(selected) => {
				leaf = descend_to_leaf(trees, selected)
				match find_neighbor_leaf(trees, leaf, direction) {
					NoNeighbor => (trees, False)
					Neighbor(neighbor) =>
						match (is_stack(trees, neighbor), is_stack(trees, leaf), participating_window(trees, leaf), participating_window(trees, neighbor)) {
							(True, False, Some(source), Some(target)) => apply_target_drop(trees, layout, { source, target, action: Stack })
							_ => join_under(trees, layout, leaf, neighbor)
						}
				}
			}
		}

	## Up from `current` to the split the neighbour is a child of: unless it is the root, its
	## windows become one stack in its place.
	join_under : Trees, U64, U64, U64 -> (Trees, Bool)
	join_under = |trees, layout, current, neighbor|
		match parent(trees, current) {
			NoParent => (trees, False)
			Parent(above) =>
				match parent(trees, neighbor) {
					Parent(shared) if shared == above =>
						match parent(trees, above) {
							NoParent => (trees, False)
							Parent(_) => {
								under = windows_under(trees, above)
								# The last member is the visible one, so keep the selected window there.
								ordered =
									match selected_window(trees, layout) {
										Some(chosen) =>
											match under.find_first_index(|window| window == chosen) {
												Ok(index) => under.take_first(index).concat(under.drop_first(index + 1)).append(chosen)
												Err(_) => under
											}
										Nothing => under
									}
								match last_on_top(ordered) {
									NoTabs => (trees, False)
									Tabs(tabs) => {
										(with_leaf, joined) = make_leaf(trees, Held({ members: Stack(tabs), fill: Tiled }))
										in_place = replace_child(with_leaf, above, joined)
										(select(remove_subtree(in_place, above), joined), True)
									}
								}
							}
						}
					_ => join_under(trees, layout, above, neighbor)
				}
		}

	## The selected stack's windows put back one by one at the selection, the stack's fill
	## handed to the visible member.
	unjoin_selection : Trees, U64 -> (Trees, Bool)
	unjoin_selection = |trees, layout|
		match selection_of_layout(trees, layout) {
			NoSelection => (trees, False)
			Selection(selected) => {
				leaf = descend_to_leaf(trees, selected)
				match content_of(trees, leaf, "unjoin_selection") {
					Held({ members: Stack(tabs), fill }) => {
						emptied = select(set_content(trees, leaf, Empty, "unjoin_selection"), leaf)
						inserted = tabs_windows(tabs).fold(emptied, |acc, member| insert_window_at_selection(acc, layout, member))
						# Insertion rebuilds leaves without fullscreen, so hand it back to the visible member.
						refilled =
							match selection_of_layout(inserted, layout) {
								Selection(visible) =>
									match kind(inserted, visible) {
										Leaf({ content: Held(held), .. }) => set_content(inserted, visible, Held({ ..held, fill }), "unjoin_selection")
										_ => inserted
									}
								NoSelection => inserted
							}
						(refilled, True)
					}
					_ => (trees, False)
				}
			}
		}

	## Whether the selected leaf holds a stack of two or more: unjoin it, else join.
	consume_or_expel : Trees, U64, BspDirection -> (Trees, Bool)
	consume_or_expel = |trees, layout, direction| {
		joined =
			match selection_of_layout(trees, layout) {
				Selection(selected) =>
					match content(trees, descend_to_leaf(trees, selected)) {
						Content(Held({ members: Stack(tabs), .. })) => tabs_windows(tabs).len() >= 2
						_ => False
					}
				NoSelection => False
			}
		if joined unjoin_selection(trees, layout) else join_selection_with_direction(trees, layout, direction)
	}

	is_stack : Trees, U64 -> Bool
	is_stack = |trees, key|
		match content(trees, key) {
			Content(Held({ members: Stack(_), .. })) => True
			_ => False
		}

	# Selection and insertion.

	## A layout's selection: where the split memories lead, or, while windows are left out and
	## that leaf holds none taking part, the first leaf that holds one (none: no layout, or no
	## window takes part).
	selection_of_layout : Trees, U64 -> [Selection(U64), NoSelection]
	selection_of_layout = |trees, layout|
		match Slots.get(trees.layouts, layout) {
			NotHeld => NoSelection
			Held(root) => {
				selected = current_selection(trees, root)
				if trees.left_out.is_empty() or takes_part(trees, selected) {
					Selection(selected)
				} else {
					match preorder(trees, root).find_first(|at| takes_part(trees, at)) {
						Err(_) => NoSelection
						Ok(first_active) =>
							match content(trees, selected) {
								Content(Held(_)) => Selection(first_active)
								_ => Selection(selected)
							}
					}
				}
			}
		}

	## Where the split memories lead from `key`.
	current_selection : Trees, U64 -> U64
	current_selection = |trees, key|
		match kind(trees, key) {
			Split({ first, second, memory: Toward(side), .. }) =>
				match side {
					First => current_selection(trees, first)
					Second => current_selection(trees, second)
				}
			_ => key
		}

	## The leaf a selection stands for: while windows are left out, the first leaf in order
	## holding one that takes part; else the first leaf by first children.
	descend_to_leaf : Trees, U64 -> U64
	descend_to_leaf = |trees, key| {
		active =
			if trees.left_out.is_empty() {
				Err(NotFound)
			} else {
				preorder(trees, key).find_first(|at| takes_part(trees, at))
			}
		match active {
			Ok(leaf) => leaf
			Err(_) => first_leaf(trees, key)
		}
	}

	first_leaf : Trees, U64 -> U64
	first_leaf = |trees, key|
		match children(trees, key) {
			Children(first, _) => first_leaf(trees, first)
			Childless => key
		}

	## `key` selected: a split there stops; each split above leads toward it.
	select : Trees, U64 -> Trees
	select = |trees, key| {
		stopped =
			match kind(trees, key) {
				Split(split) => set_kind(trees, key, Split({ ..split, memory: Stops }), "select")
				Leaf(_) => trees
			}
		lead_up(stopped, key)
	}

	lead_up : Trees, U64 -> Trees
	lead_up = |trees, key|
		match parent(trees, key) {
			NoParent => trees
			Parent(above) => {
				split = split_of(trees, above, "select")
				side = if split.second == key Second else First
				lead_up(set_kind(trees, above, Split({ ..split, memory: Toward(side) }), "select"), above)
			}
		}

	## The node becomes a split of `first` and `second` (an even share, its memory stopped),
	## their parent.
	set_split : Trees, U64, BspOrientation, (U64, U64) -> Trees
	set_split = |trees, key, orientation, (first, second)| {
		at = node(trees, key, "set_split")
		split = set_node(trees, key, { ..at, kind: Split({ orientation, ratio: 0.5, first, second, memory: Stops }) }, "set_split")
		set_parent(set_parent(split, first, Split(key)), second, Split(key))
	}

	## A window put in the selected leaf when one is preselected: it splits toward that
	## direction.
	smart_insert_window : Trees, U64, U64 -> [Inserted(Trees), NotPreselected]
	smart_insert_window = |trees, layout, window|
		match selection_of_layout(trees, layout) {
			NoSelection => NotPreselected
			Selection(selection) => {
				leaf = descend_to_leaf(trees, selection)
				match kind(trees, leaf) {
					Leaf({ preselect: Toward(direction), .. }) =>
						Inserted(split_leaf(trees, leaf, orientation_of(direction), lone(window), side_of(direction)))
					_ => NotPreselected
				}
			}
		}

	## The leaf becomes a split of its windows (tiled, no preselection) and `new`, on `side`;
	## the new leaf is selected.
	split_leaf : Trees, U64, BspOrientation, BspContent, BspSide -> Trees
	split_leaf = |trees, leaf, orientation, new, side| {
		existing_content = tiled(content_of(trees, leaf, "split_leaf"))
		emptied = set_content(trees, leaf, Empty, "split_leaf")
		(with_existing, existing) = make_leaf(emptied, existing_content)
		(with_new, made) = make_leaf(with_existing, new)
		pair =
			match side {
				First => (made, existing)
				Second => (existing, made)
			}
		select(set_split(with_new, leaf, orientation, pair), made)
	}

	## Horizontal at an odd depth (the root counted as one), vertical at an even one.
	orientation_for_leaf : Trees, U64 -> BspOrientation
	orientation_for_leaf = |trees, key| if ancestors(trees, key).len().is_odd() Horizontal else Vertical

	## A window put at the layout's selection: into an empty leaf, or splitting a held one.
	insert_window_at_selection : Trees, U64, U64 -> Trees
	insert_window_at_selection = |trees, layout, window|
		match Slots.get(trees.layouts, layout) {
			NotHeld => trees
			Held(root) => {
				selection =
					match selection_of_layout(trees, layout) {
						Selection(selected) => selected
						NoSelection => current_selection(trees, root)
					}
				leaf = descend_to_leaf(trees, selection)
				selected = if leaf != selection select(trees, leaf) else trees
				match content_of(selected, leaf, "insert_window_at_selection") {
					Empty => set_content(selected, leaf, lone(window), "insert_window_at_selection")
					Held(_) => split_leaf(selected, leaf, orientation_for_leaf(selected, leaf), lone(window), Second)
				}
			}
		}

	## One occurrence removed (none: the layout does not hold the window).
	remove_window_internal : Trees, U64, U64 -> [Removed(Trees), Absent]
	remove_window_internal = |trees, layout, window|
		match node_for_window_in_layout(trees, layout, window) {
			NotFound => Absent
			Found(leaf) => Removed(remove_member(trees, layout, leaf, window))
		}

	## `window` out of its leaf: a stack keeps its others; an emptied leaf leaves its split (its
	## sibling takes the split's place), and the layout's selection is selected again.
	remove_member : Trees, U64, U64, U64 -> Trees
	remove_member = |trees, layout, leaf, window|
		match removed(content_of(trees, leaf, "remove_member"), window) {
			Held(held) => select(set_content(trees, leaf, Held(held), "remove_member"), leaf)
			Empty => {
				emptied = set_content(trees, leaf, Empty, "remove_member")
				pruned =
					match parent(emptied, leaf) {
						Parent(_) => {
							detached = detach_child(emptied, leaf)
							(nodes, _) = Slots.remove(detached.nodes, leaf)
							{ ..detached, nodes }
						}
						NoParent => emptied
					}
				root =
					match Slots.get(pruned.layouts, layout) {
						Held(found) => found
						NotHeld => {
							crash "BspTree.remove_member: no layout ${layout.to_str()}"
						}
					}
				select(pruned, descend_to_leaf(pruned, current_selection(pruned, root)))
			}
		}

	## `key` taken out of its split: its sibling's subtree takes the split's place (the split's
	## memory stopped); `key` is left unplaced.
	detach_child : Trees, U64 -> Trees
	detach_child = |trees, key| {
		above =
			match parent(trees, key) {
				Parent(found) => found
				NoParent => {
					crash "BspTree.detach_child: node ${key.to_str()} is a root"
				}
			}
		split = split_of(trees, above, "detach_child")
		sibling = if split.first == key split.second else split.first
		(nodes, took) = Slots.remove(trees.nodes, sibling)
		sibling_node =
			match took {
				Took(found) => found
				NotHeld => {
					crash "BspTree.detach_child: no node ${sibling.to_str()}"
				}
			}
		unplaced = set_parent({ ..trees, nodes }, key, Root)
		promoted =
			match sibling_node.kind {
				Split(inner) => Split({ ..inner, memory: Stops })
				Leaf(_) => sibling_node.kind
			}
		replaced_kind = set_kind(unplaced, above, promoted, "detach_child")
		match promoted {
			Split(inner) => set_parent(set_parent(replaced_kind, inner.first, Split(above)), inner.second, Split(above))
			Leaf(_) => replaced_kind
		}
	}

	# Reading the tree.

	contains_window : Trees, U64, U64 -> Bool
	contains_window = |trees, layout, window|
		match node_for_window_in_layout(trees, layout, window) {
			Found(_) => True
			NotFound => False
		}

	## The leaf of the layout holding `window`, first in order.
	node_for_window_in_layout : Trees, U64, U64 -> [Found(U64), NotFound]
	node_for_window_in_layout = |trees, layout, window|
		match Slots.get(trees.layouts, layout) {
			Held(root) => find_window(trees, root, window)
			NotHeld => NotFound
		}

	find_window : Trees, U64, U64 -> [Found(U64), NotFound]
	find_window = |trees, key, window|
		match kind(trees, key) {
			Split({ first, second, .. }) =>
				match find_window(trees, first, window) {
					Found(leaf) => Found(leaf)
					NotFound => find_window(trees, second, window)
				}
			Leaf({ content: Held({ members, .. }), .. }) => if members_windows(members).contains(window) Found(key) else NotFound
			Leaf({ content: Empty, .. }) => NotFound
		}

	## Every layout's key, in slot order.
	layout_keys : Trees -> List(U64)
	layout_keys = |trees|
		trees.layouts.slots.map_with_index(
			|slot, index|
				match slot {
					Occupied({ version, .. }) => [Slots.key_of(index, version)]
					Vacant(_) => []
				},
		)
		.join()

	## Every layout's root, in slot order.
	layout_roots : Trees -> List(U64)
	layout_roots = |trees|
		trees.layouts.slots.join_map(
			|slot|
				match slot {
					Occupied({ value, .. }) => [value]
					Vacant(_) => []
				},
		)

	## The subtree's nodes in order: a split, then its first child's, then its second's.
	preorder : Trees, U64 -> List(U64)
	preorder = |trees, key|
		match children(trees, key) {
			Children(first, second) => [key].concat(preorder(trees, first)).concat(preorder(trees, second))
			Childless => [key]
		}

	## The node and each node above it, up to its root.
	ancestors : Trees, U64 -> List(U64)
	ancestors = |trees, key|
		match parent(trees, key) {
			Parent(above) => [key].concat(ancestors(trees, above))
			NoParent => [key]
		}

	## The windows of the subtree's leaves, in order (a stack's in stack order).
	windows_under : Trees, U64 -> List(U64)
	windows_under = |trees, key|
		preorder(trees, key).join_map(
			|at|
				match content(trees, at) {
					Content(Held({ members, .. })) => members_windows(members)
					_ => []
				},
		)

	parent : Trees, U64 -> [Parent(U64), NoParent]
	parent = |trees, key|
		match node(trees, key, "parent").parent {
			Split(above) => Parent(above)
			Root => NoParent
		}

	set_parent : Trees, U64, BspParent -> Trees
	set_parent = |trees, key, placed| {
		at = node(trees, key, "set_parent")
		set_node(trees, key, { ..at, parent: placed }, "set_parent")
	}

	set_kind : Trees, U64, BspKind, Str -> Trees
	set_kind = |trees, key, made, op| {
		at = node(trees, key, op)
		set_node(trees, key, { ..at, kind: made }, op)
	}

	## The split `key` names, which is a split (a node's parent, or a node known to have
	## children).
	split_of : Trees, U64, Str -> BspTreeSplit
	split_of = |trees, key, op|
		match node(trees, key, op).kind {
			Split(split) => split
			Leaf(_) => {
				crash "BspTree.${op}: node ${key.to_str()} is not a split"
			}
		}

	## A node's content, if it is a leaf.
	content : Trees, U64 -> [Content(BspContent), NotLeaf]
	content = |trees, key|
		match kind(trees, key) {
			Leaf(leaf) => Content(leaf.content)
			Split(_) => NotLeaf
		}

	## The content of `key`, which is a leaf.
	content_of : Trees, U64, Str -> BspContent
	content_of = |trees, key, op|
		match node(trees, key, op).kind {
			Leaf(leaf) => leaf.content
			Split(_) => {
				crash "BspTree.${op}: node ${key.to_str()} is not a leaf"
			}
		}

	## `key`'s content replaced; `key` is a leaf.
	set_content : Trees, U64, BspContent, Str -> Trees
	set_content = |trees, key, held, op| {
		at = node(trees, key, op)
		match at.kind {
			Leaf(leaf) => set_node(trees, key, { ..at, kind: Leaf({ ..leaf, content: held }) }, op)
			Split(_) => {
				crash "BspTree.${op}: node ${key.to_str()} is not a leaf"
			}
		}
	}

	## Whether the node is a leaf holding a window that takes part.
	takes_part : Trees, U64 -> Bool
	takes_part = |trees, key|
		match participating_window(trees, key) {
			Some(_) => True
			Nothing => False
		}

	## The window a leaf shows: its top, if it takes part, else its first member that does.
	participating_window : Trees, U64 -> MaybeWord
	participating_window = |trees, key|
		match content(trees, key) {
			Content(held) => participating_member(trees, held)
			NotLeaf => Nothing
		}

	participating_member : Trees, BspContent -> MaybeWord
	participating_member = |trees, held|
		match held {
			Empty => Nothing
			Held({ members, .. }) => {
				top = members_top(members)
				if !trees.left_out.contains(top) {
					Some(top)
				} else {
					match members_windows(members).find_first(|window| !trees.left_out.contains(window)) {
						Ok(window) => Some(window)
						Err(_) => Nothing
					}
				}
			}
		}

	# Contents and members.

	## A leaf's content for one window, tiled.
	lone : U64 -> BspContent
	lone = |window| Held({ members: Window(window), fill: Tiled })

	## The same windows, tiled (a split's new leaf keeps its windows, not its fill).
	tiled : BspContent -> BspContent
	tiled = |held|
		match held {
			Empty => Empty
			Held({ members, .. }) => Held({ members, fill: Tiled })
		}

	## Without `window`: a stack keeps its other windows, the last on top, and its fill; a leaf
	## left with none is empty.
	removed : BspContent, U64 -> BspContent
	removed = |held, window|
		match held {
			Held({ members: Stack(tabs), fill }) =>
				match last_on_top(tabs_windows(tabs).keep_if(|member| member != window)) {
					Tabs(rest) => Held({ members: Stack(rest), fill })
					NoTabs => Empty
				}
			_ => Empty
		}

	## The members with `from` renamed `to`.
	replaced : BspMembers, U64, U64 -> BspMembers
	replaced = |members, from, to| {
		swap = |member| if member == from to else member
		match members {
			Window(window) => Window(swap(window))
			Stack({ before, top, after }) => Stack({ before: before.map(swap), top: swap(top), after: after.map(swap) })
		}
	}

	members_top : BspMembers -> U64
	members_top = |members|
		match members {
			Window(window) => window
			Stack(tabs) => tabs.top
		}

	## The windows in stack order.
	members_windows : BspMembers -> List(U64)
	members_windows = |members|
		match members {
			Window(window) => [window]
			Stack(tabs) => tabs_windows(tabs)
		}

	tabs_windows : BspTabs -> List(U64)
	tabs_windows = |{ before, top, after }| before.append(top).concat(after)

	## `windows` as a stack with the last on top (none: no windows).
	last_on_top : List(U64) -> [Tabs(BspTabs), NoTabs]
	last_on_top = |windows|
		match windows.last() {
			Ok(top) => Tabs({ before: windows.drop_last(1), top, after: [] })
			Err(_) => NoTabs
		}

	# Geometry rules.

	## Whether a native resize entered or left a fullscreen frame, and the fill it leaves
	## (upstream's `track_fullscreen_resize`).
	track_fullscreen_resize : Fill, BspNativeResize -> [Tracked(Fill), Untracked]
	track_fullscreen_resize = |fill, { old, new, screen, tiling, .. }|
		if same_rect(new, screen) {
			Tracked(Screen)
		} else if same_rect(old, screen) {
			Tracked(
				match fill {
					Screen => Tiled
					_ => fill
				},
			)
		} else if same_rect(new, tiling) {
			Tracked(WithinGaps)
		} else if same_rect(old, tiling) {
			Tracked(
				match fill {
					WithinGaps => Tiled
					_ => fill
				},
			)
		} else {
			Untracked
		}

	## Rust's `Rect` equality: each field equal as an `f64`.
	same_rect : Rect, Rect -> Bool
	same_rect = |a, b| a.x == b.x and a.y == b.y and a.w == b.w and a.h == b.h

	## Rust's `f64::clamp` (a NaN stays NaN).
	clamp_f64 : F64, F64, F64 -> F64
	clamp_f64 = |value, low, high| if value < low low else if value > high high else value

	## Rust's `f64::EPSILON`.
	epsilon : F64
	epsilon = 2.220446049250313e-16

	orientation_of : BspDirection -> BspOrientation
	orientation_of = |direction|
		match direction {
			Left | Right => Horizontal
			Up | Down => Vertical
		}

	## The side a window moved or put this way takes: the first going left or up.
	side_of : BspDirection -> BspSide
	side_of = |direction|
		match direction {
			Left | Up => First
			Right | Down => Second
		}

	## A sorted list without its repeats.
	dedup : List(U64) -> List(U64)
	dedup = |sorted|
		sorted.fold(
			[],
			|kept, window|
				match kept.last() {
					Ok(last) if last == window => kept
					_ => kept.append(window)
				},
		)

	# Nodes.

	## The node `key` names, which the tree holds (`op` names what looked).
	node : Trees, U64, Str -> BspTreeNode
	node = |trees, key, op|
		match Slots.get(trees.nodes, key) {
			Held(found) => found
			NotHeld => {
				crash "BspTree.${op}: no node ${key.to_str()}"
			}
		}

	## The tree with `key`'s node replaced, which the tree holds (`op` names what wrote).
	set_node : Trees, U64, BspTreeNode, Str -> Trees
	set_node = |trees, key, value, op|
		match Slots.replace(trees.nodes, key, value) {
			Held(nodes) => { ..trees, nodes }
			NotHeld => {
				crash "BspTree.${op}: no node ${key.to_str()}"
			}
		}

	kind : Trees, U64 -> BspKind
	kind = |trees, key| node(trees, key, "kind").kind

	make_node : Trees, BspKind -> (Trees, U64)
	make_node = |trees, made| {
		(nodes, key) = Slots.insert(trees.nodes, { parent: Root, kind: made })
		({ ..trees, nodes }, key)
	}

	make_leaf : Trees, BspContent -> (Trees, U64)
	make_leaf = |trees, held| make_node(trees, Leaf({ content: held, preselect: Unset }))

	## A split's children, first then second; none for a leaf.
	children : Trees, U64 -> [Children(U64, U64), Childless]
	children = |trees, key|
		match kind(trees, key) {
			Split(split) => Children(split.first, split.second)
			Leaf(_) => Childless
		}

	## A copy of `source`'s subtree placed under `parent`: the copy first, then its first
	## child's subtree, then its second's.
	clone_node : Trees, U64, BspParent -> (Trees, U64)
	clone_node = |trees, source, under| {
		original = node(trees, source, "clone_node")
		(with_clone, clone) = make_node(trees, original.kind)
		placed =
			match original.kind {
				Split(split) => {
					(with_first, first) = clone_node(with_clone, split.first, Split(clone))
					(with_second, second) = clone_node(with_first, split.second, Split(clone))
					set_node(with_second, clone, { parent: under, kind: Split({ ..split, first, second }) }, "clone_node")
				}
				Leaf(_) => set_node(with_clone, clone, { parent: under, kind: original.kind }, "clone_node")
			}
		(placed, clone)
	}

	## The subtree's nodes removed, children before their parent, first before second.
	remove_subtree : Trees, U64 -> Trees
	remove_subtree = |trees, root| {
		pruned =
			match children(trees, root) {
				Children(first, second) => remove_subtree(remove_subtree(trees, first), second)
				Childless => trees
			}
		(nodes, _) = Slots.remove(pruned.nodes, root)
		{ ..pruned, nodes }
	}

}
