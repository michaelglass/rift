## A BSP layout system's trees (`rift_core::layout::bsp_tree::Tree`), ported operation by
## operation: every layout's root in one slot table and every node in another, so keys are
## allocated in one sequence, as upstream's slotmap allocates them. Keys are slotmap's ffi form,
## one word each (`version << 32 | index`); windows are one word each.
##
## A node is looked up only by a key the tree holds (a layout's root, a split's child, a node's
## parent): a missing one is a broken tree, as it is in Rust (whose `Index` panics), so the
## lookups crash, naming what looked and the key.

import pf.Boundary exposing [Aspect, BspContent, BspInsertion, BspKind, BspLayoutSlot, BspNodeSlot, BspParent, BspParticipation, BspTakesPart, BspTreeAnswer, BspTreeEvent, BspTreeNode]

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
	make_leaf = |trees, content| make_node(trees, Leaf({ content, preselect: Unset }))

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
	clone_node = |trees, source, parent| {
		original = node(trees, source, "clone_node")
		(with_clone, clone) = make_node(trees, original.kind)
		placed =
			match original.kind {
				Split(split) => {
					(with_first, first) = clone_node(with_clone, split.first, Split(clone))
					(with_second, second) = clone_node(with_first, split.second, Split(clone))
					set_node(with_second, clone, { parent, kind: Split({ ..split, first, second }) }, "clone_node")
				}
				Leaf(_) => set_node(with_clone, clone, { parent, kind: original.kind }, "clone_node")
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
