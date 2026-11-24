## A BSP layout system's trees (`rift_core::layout::bsp_tree::Tree`), ported operation by
## operation: every layout's root in one slot table and every node in another, so keys are
## allocated in one sequence, as upstream's slotmap allocates them. Keys are slotmap's ffi form,
## one word each (`version << 32 | index`); windows are one word each.
##
## A node is looked up only by a key the tree holds (a layout's root, a split's child, a node's
## parent): a missing one is a broken tree, as it is in Rust (whose `Index` panics), so the
## lookups crash, naming what looked and the key.

import Geometry
import pf.Boundary exposing [Aspect, BspContent, BspDirection, BspMembers, BspNativeResize, BspOrientation, BspReplacement, BspSide, BspTabs, BspTreeSplit, BspWindowIn, Fill, MaybeWord, Rect, BspInsertion, BspKind, BspLayoutSlot, BspNodeSlot, BspParent, BspParticipation, BspTakesPart, BspTreeAnswer, BspTreeEvent, BspTreeNode]

## One slot: a value under an odd version, or vacant under an even one.
Slot(v) : [Occupied({ version : U32, value : v }), Vacant(U32)]

## Values by key, in slots, with slotmap's (1.1) rules. Slot 0 is the sentinel: always vacant,
## never handed out. `free` holds the vacant slots in the order they are taken: the last first.
Slots(v) : { slots : List(Slot(v)), free : List(U32) }

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
		layouts: no_slots({}),
		nodes: no_slots({}),
		left_out: Set.empty(),
		insertion,
		aspect: Free,
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
		}

	view : Trees -> View
	view = |trees| {
		layouts: trees.layouts.slots.map(
			|slot|
				match slot {
					Occupied({ version, value }) => Occupied({ version, root: value })
					Vacant(version) => Vacant(version)
				},
		),
		layout_free: trees.layouts.free,
		nodes: trees.nodes.slots.map(
			|slot|
				match slot {
					Occupied({ version, value }) => Occupied({ version, node: value })
					Vacant(version) => Vacant(version)
				},
		),
		node_free: trees.nodes.free,
		left_out: trees.left_out.to_list().sort_with(|a, b| a.order_relative_to(b)),
		insertion: trees.insertion,
		aspect: trees.aspect,
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
		(layouts, layout) = insert(with_leaf.layouts, leaf)
		({ ..with_leaf, layouts }, layout)
	}

	## A copy of the layout's tree under new keys (a new layout when there is none).
	clone_layout : Trees, U64 -> (Trees, U64)
	clone_layout = |trees, layout|
		match get(trees.layouts, layout) {
			Held(root) => {
				(cloned, clone) = clone_node(trees, root, Root)
				(layouts, key) = insert(cloned.layouts, clone)
				({ ..cloned, layouts }, key)
			}
			NotHeld => create_layout(trees)
		}

	remove_layout : Trees, U64 -> Trees
	remove_layout = |trees, layout| {
		(layouts, took) = remove(trees.layouts, layout)
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
			match get(trees.layouts, layout) {
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
			slots = cleared.nodes.slots.map(
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

	# Selection and insertion.

	## A layout's selection: where the split memories lead, or, while windows are left out and
	## that leaf holds none taking part, the first leaf that holds one (none: no layout, or no
	## window takes part).
	selection_of_layout : Trees, U64 -> [Selection(U64), NoSelection]
	selection_of_layout = |trees, layout|
		match get(trees.layouts, layout) {
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
		match get(trees.layouts, layout) {
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
							(nodes, _) = remove(detached.nodes, leaf)
							{ ..detached, nodes }
						}
						NoParent => emptied
					}
				root =
					match get(pruned.layouts, layout) {
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
		(nodes, took) = remove(trees.nodes, sibling)
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
		match get(trees.layouts, layout) {
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
					Occupied({ version, .. }) => [key_of(index, version)]
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
		match get(trees.nodes, key) {
			Held(found) => found
			NotHeld => {
				crash "BspTree.${op}: no node ${key.to_str()}"
			}
		}

	## The tree with `key`'s node replaced, which the tree holds (`op` names what wrote).
	set_node : Trees, U64, BspTreeNode, Str -> Trees
	set_node = |trees, key, value, op|
		match replace(trees.nodes, key, value) {
			Held(nodes) => { ..trees, nodes }
			NotHeld => {
				crash "BspTree.${op}: no node ${key.to_str()}"
			}
		}

	kind : Trees, U64 -> BspKind
	kind = |trees, key| node(trees, key, "kind").kind

	make_node : Trees, BspKind -> (Trees, U64)
	make_node = |trees, made| {
		(nodes, key) = insert(trees.nodes, { parent: Root, kind: made })
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
		(nodes, _) = remove(pruned.nodes, root)
		{ ..pruned, nodes }
	}

	# The slot table (`rift_core::slot_table`, slotmap's rules).

	no_slots : {} -> Slots(v)
	no_slots = |{}| { slots: [Vacant(0)], free: [] }

	## The key of slot `index` under `version`.
	key_of : U64, U32 -> U64
	key_of = |index, version| version.to_u64().shl_wrap(32).bitwise_or(index)

	## A key's slot index and version.
	parts : U64 -> (U64, U32)
	parts = |key| (key.bitwise_and(0xFFFF_FFFF), key.shr_zf_wrap(32).to_u32_wrap())

	## `value` stored in the most recently vacated slot (at its version `| 1`), or a new one
	## (at version 1).
	insert : Slots(v), v -> (Slots(v), U64)
	insert = |table, value|
		match table.free.last() {
			Ok(index) => {
				at = index.to_u64()
				version =
					match table.slots.get(at) {
						Ok(Vacant(vacant)) => vacant.bitwise_or(1)
						_ => {
							crash "BspTree.insert: free slot ${at.to_str()} is not vacant"
						}
					}
				slots = put(table.slots, at, Occupied({ version, value }))
				({ slots, free: table.free.drop_last(1) }, key_of(at, version))
			}
			Err(_) => {
				at = table.slots.len()
				({ ..table, slots: table.slots.append(Occupied({ version: 1, value })) }, key_of(at, 1))
			}
		}

	## `key`'s value taken out; its slot (at its version plus one) is the next one taken.
	remove : Slots(v), U64 -> (Slots(v), [Took(v), NotHeld])
	remove = |table, key| {
		(index, version) = parts(key)
		match table.slots.get(index) {
			Ok(Occupied(held)) if held.version == version => {
				slots = put(table.slots, index, Vacant(version.plus_wrap(1)))
				({ slots, free: table.free.append(index.to_u32_wrap()) }, Took(held.value))
			}
			_ => (table, NotHeld)
		}
	}

	get : Slots(v), U64 -> [Held(v), NotHeld]
	get = |table, key| {
		(index, version) = parts(key)
		match table.slots.get(index) {
			Ok(Occupied(held)) if held.version == version => Held(held.value)
			_ => NotHeld
		}
	}

	## The table with `key`'s value replaced, if it holds one.
	replace : Slots(v), U64, v -> [Held(Slots(v)), NotHeld]
	replace = |table, key, value| {
		(index, version) = parts(key)
		match table.slots.get(index) {
			Ok(Occupied(held)) if held.version == version =>
				Held({ ..table, slots: put(table.slots, index, Occupied({ version, value })) })
			_ => NotHeld
		}
	}

	## The slots with slot `index`, one just read, set.
	put : List(Slot(v)), U64, Slot(v) -> List(Slot(v))
	put = |slots, index, slot|
		match slots.set(index, slot) {
			Ok(set) => set
			Err(OutOfBounds) => {
				crash "BspTree.put: no slot ${index.to_str()}"
			}
		}
}
