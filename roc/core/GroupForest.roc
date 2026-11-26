## The group forest (`rift_core::layout::group_forest`), ported operation by operation: the tree
## the traditional and master-stack layout systems share, and each system's own operations over
## it. One forest per system: every layout's root in one slot table and every node in another
## (keys as slotmap allocates them), the windows left out of layout, and the stamp the next
## window to enter a layout takes. Keys and windows are one word each.
##
## A node or layout is looked up only by a key the forest holds: a missing one is a broken
## forest, as it is in Rust (whose `Index` panics and whose `expect`s and `unreachable!`s stop),
## so the lookup crashes, naming what looked and the key.

import Geometry
import Slots
import Traditional exposing [Branch, Laid, Tree, TreeAxis, TreeStack]
import pf.Boundary exposing [Frame, GroupBar, GroupBars, GroupBranch, GroupContainerView, GroupProjected, GroupRole, GroupRow, StackFramesQuestion, WindowConstraints, BspAppIn, BspDirection, BspEdited, BspFocusIn, BspFocused, BspFullscreen, BspLayoutDirection, BspMoveQuery, BspNativeResize, BspOrientation, BspParticipation, BspReplacement, BspResize, BspSlot, BspTreeAnswer, BspWindowIn, BspWindowsQuery, Fill, GroupContainer, GroupDrop, GroupDropAction, GroupEditIn, GroupForestEvent, GroupForestSeed, GroupKind, GroupMemory, GroupNewSibling, GroupPlace, GroupTreeNode, MasterArrangement, MasterSettings, MasterStackEvent, MasterStackSeed, MaybeWord, Rect, StackBeyond, StackDefault, TraditionalAdd, TraditionalEvent, TraditionalSeed, TraditionalSettings]

## The nodes a layout pass takes part with: a mark per slot of the forest's node table
## (`Slots.unmarked`, `mark`, `marked`).
TakingPart : List(Bool)

## A forest: every layout's root, every node, the windows left out of layout, the next stamp.
Forest : {
	layouts : Slots(U64),
	nodes : Slots(GroupTreeNode),
	left_out : Set(U64),
	next_stamp : U64,
}

## The traditional layout system: the forest and its settings (`Traditional`).
Traditionals : { forest : Forest, settings : TraditionalSettings }

## The master-stack layout system: the forest, each layout's root a region pair, and its
## settings (`MasterStack`).
Master : { forest : Forest, settings : MasterSettings }

## Where a node goes among a container's children (`Placement`).
Placement : [Back(U64), Before(U64), After(U64)]

## A resolved place among a container's children: the container, and the index a child put there
## takes. `position` answers one for a placed node (`At`); `back_of` is a container's end.
Place : { above : U64, index : U64 }

## How far a walk over a container's children goes (`BranchLimit`).
Limit : [All, First, FirstVisible]

## The order a walk visits a subtree in (`GroupForest.walk`).
WalkOrder : [Preorder, Postorder]

## A walk step's answer: go on with the state, or end the walk with it.
Visit(s) : [Continue(s), Break(s)]

## A node's place among its parent's children.
Position : [At({ above : U64, index : U64 }), Unplaced]

## What moving windows between a master-stack layout's regions keeps fixed.
Moves : { source : U64, destination : U64, forward : Bool, protected : MaybeWord, selected : MaybeWord, front : Bool }

GroupForest := [].{

	# The systems' boundary: init, seed, step, view.

	traditional_empty : TraditionalSettings -> Traditionals
	traditional_empty = |settings| { forest: empty_forest({}), settings }

	traditional_seeded : TraditionalSeed -> Traditionals
	traditional_seeded = |seed| { forest: seeded_forest(seed.forest), settings: seed.settings }

	traditional_view : Traditionals -> TraditionalSeed
	traditional_view = |system| { forest: forest_view(system.forest), settings: system.settings }

	master_empty : MasterSettings -> Master
	master_empty = |settings| { forest: empty_forest({}), settings }

	master_seeded : MasterStackSeed -> Master
	master_seeded = |seed| { forest: seeded_forest(seed.forest), settings: seed.settings }

	master_view : Master -> MasterStackSeed
	master_view = |system| { forest: forest_view(system.forest), settings: system.settings }

	empty_forest : {} -> Forest
	empty_forest = |{}| { layouts: Slots.empty({}), nodes: Slots.empty({}), left_out: Set.empty(), next_stamp: 0 }

	## A forest as the platform holds it (`Forest::from_parts`), its stamp counter included.
	seeded_forest : GroupForestSeed -> Forest
	seeded_forest = |seed| {
		slots = seed.nodes.map(
			|slot|
				match slot {
					Occupied({ version, node: held }) => Occupied({ version, value: held })
					Vacant(version) => Vacant(version)
				},
		)
		{
			layouts: Slots.from_parts(
				seed.layouts.map(
					|slot|
						match slot {
							Occupied({ version, root: held }) => Occupied({ version, value: held })
							Vacant(version) => Vacant(version)
						},
				),
				seed.layout_free,
			),
			nodes: Slots.from_parts(slots, seed.node_free),
			left_out: Set.from_list(seed.left_out),
			next_stamp: seed.next_stamp,
		}
	}

	forest_view : Forest -> GroupForestSeed
	forest_view = |forest| {
		layouts: Slots.slot_list(forest.layouts).map(
			|slot|
				match slot {
					Occupied({ version, value }) => Occupied({ version, root: value })
					Vacant(version) => Vacant(version)
				},
		),
		layout_free: Slots.free_indices(forest.layouts),
		nodes: Slots.slot_list(forest.nodes).map(
			|slot|
				match slot {
					Occupied({ version, value }) => Occupied({ version, node: value })
					Vacant(version) => Vacant(version)
				},
		),
		node_free: Slots.free_indices(forest.nodes),
		left_out: forest.left_out.to_list().sort_with(|a, b| a.order_relative_to(b)),
		next_stamp: forest.next_stamp,
	}

	## One operation of the traditional system, and what it answers.
	traditional_step : Traditionals, TraditionalEvent -> (Traditionals, BspTreeAnswer)
	traditional_step = |system, event|
		match event {
			Forest(asked) => {
				(forest, answer) = forest_step(system.forest, asked)
				({ ..system, forest }, answer)
			}
			CreateLayout => {
				(forest, layout) = create_layout(system.forest)
				({ ..system, forest }, Layout(layout))
			}
			Settings(settings) => ({ ..system, settings }, Done)
			AddWindow(added) => (traditional_add_window(system, added), Done)
			RemoveWindow(window) => {
				(forest, _) = remove_window(system.forest, window)
				({ ..system, forest }, Done)
			}
			RemoveAndRebalance(window) => ({ ..system, forest: remove_and_rebalance(system.forest, window) }, Done)
			CloseApp(windows) => ({ ..system, forest: close_app(system.forest, windows) }, Done)
			CanMoveWindow(asked) => (system, Verdict(traditional_can_move_window(system.forest, asked)))
			Edit(asked) => {
				(forest, edited) = traditional_edit(system, asked)
				({ ..system, forest }, Edited(edited))
			}
		}

	## One operation of the master-stack system, and what it answers.
	master_step : Master, MasterStackEvent -> (Master, BspTreeAnswer)
	master_step = |system, event|
		match event {
			Forest(asked) => {
				(forest, answer) = forest_step(system.forest, asked)
				({ ..system, forest }, answer)
			}
			CreateLayout => {
				(after, layout) = master_create_layout(system)
				(after, Layout(layout))
			}
			Settings(settings) => (master_update_settings(system, settings), Done)
			AddWindow(added) => (master_add_window(system, added), Done)
			RemoveWindow(window) => {
				(forest, layouts) = remove_window(system.forest, window)
				(distribute_each({ ..system, forest }, layouts), Done)
			}
			CloseApp(windows) => (master_close_app(system, windows), Done)
			AdjustRatio(delta) => (adjust_master_ratio(system, delta), Done)
			AdjustCount(delta) => (adjust_master_count(system, delta), Done)
			Promote(layout) => (promote_to_master(system, layout), Done)
			SwapMasterStack(layout) => (swap_master_stack(system, layout), Done)
			CanMoveWindow(asked) => (system, Verdict(master_can_move_window(system, asked)))
			Edit(asked) => {
				(after, edited) = master_edit(system, asked)
				(after, Edited(edited))
			}
		}

	## One of the forest's own operations, the same in both systems.
	forest_step : Forest, GroupForestEvent -> (Forest, BspTreeAnswer)
	forest_step = |forest, event|
		match event {
			CloneLayout(layout) => {
				(after, clone) = clone_layout(forest, layout)
				(after, Layout(clone))
			}
			RemoveLayout(layout) => (remove_layout(forest, layout), Done)
			Participation(asked) => {
				(after, changed) = set_window_participation(forest, asked)
				(after, Participated(changed))
			}
			ReplaceWindow(replacement) => (replace_window(forest, replacement), Done)
			ResizeWindow(resize) => (resize_window(forest, resize), Done)
			ContainsLayout(layout) =>
				match Slots.get(forest.layouts, layout) {
					Held(_) => (forest, Verdict(True))
					NotHeld => (forest, Verdict(False))
				}
			WindowParticipates(window) => (forest, Verdict(takes_part(forest, window)))
			SelectedWindow(layout) => (forest, Window(selected_window(forest, layout)))
			SelectionContainsWindow({ layout, window: window_word }) => (forest, Verdict(selection_contains_window(forest, layout, window_word)))
			WindowInDirection(asked) => (forest, Window(window_in_direction(forest, asked)))
			ContainsWindow({ layout, window: window_word }) => (forest, Verdict(contains_window(forest, layout, window_word)))
			HasAnyFullscreenNode(layout) => (forest, Verdict(has_any_fullscreen_node(forest, layout)))
			HasWindowsForApp(asked) => (forest, Verdict(has_windows_for_app(forest, asked)))
			ParentOfSelectionIsStacked(layout) =>
				match stacked_selection_container(forest, layout) {
					Some(_) => (forest, Verdict(True))
					Nothing => (forest, Verdict(False))
				}
			Windows(asked) => (forest, Windows(listed_windows(forest, asked)))
			WindowSlot({ layout, window: window_word }) => (forest, WindowSlot(window_slot(forest, layout, window_word)))
			Focus(asked) => {
				(after, focused) = focus(forest, asked)
				(after, Focused(focused))
			}
		}

	# Small helpers over the boundary's types.

	orientation_of : BspDirection -> BspOrientation
	orientation_of = |direction|
		match direction {
			Left | Right => Horizontal
			Up | Down => Vertical
		}

	same_orientation : BspOrientation, BspOrientation -> Bool
	same_orientation = |a, b|
		match (a, b) {
			(Horizontal, Horizontal) | (Vertical, Vertical) => True
			_ => False
		}

	backwards : BspDirection -> Bool
	backwards = |direction|
		match direction {
			Left | Up => True
			Right | Down => False
		}

	opposite : BspDirection -> BspDirection
	opposite = |direction|
		match direction {
			Left => Right
			Right => Left
			Up => Down
			Down => Up
		}

	perpendicular : BspOrientation -> BspOrientation
	perpendicular = |orientation|
		match orientation {
			Horizontal => Vertical
			Vertical => Horizontal
		}

	## A new stack's or split's orientation from the container's (`group_orientation`).
	group_orientation : BspOrientation, StackDefault -> BspOrientation
	group_orientation = |base, configured|
		match configured {
			Same => base
			Perpendicular => perpendicular(base)
			Horizontal => Horizontal
			Vertical => Vertical
		}

	kind_orientation : GroupKind -> BspOrientation
	kind_orientation = |made|
		match made {
			Split(orientation) | Stack(orientation) => orientation
		}

	kind_is_stacked : GroupKind -> Bool
	kind_is_stacked = |made|
		match made {
			Stack(_) => True
			Split(_) => False
		}

	## Whether a kind is a split along `orientation` (`kind == Kind::Split(orientation)`).
	splits_along : GroupKind, BspOrientation -> Bool
	splits_along = |made, orientation|
		match made {
			Split(held) => same_orientation(held, orientation)
			Stack(_) => False
		}

	is_tiled : Fill -> Bool
	is_tiled = |fill|
		match fill {
			Tiled => True
			Screen | WithinGaps => False
		}

	same_rect : Rect, Rect -> Bool
	same_rect = |a, b| a.x == b.x and a.y == b.y and a.w == b.w and a.h == b.h

	## The child remembered, whether or not the selection stops at the container.
	memory_last : GroupMemory -> MaybeWord
	memory_last = |memory|
		match memory {
			Unset => Nothing
			Toward(child) | Stops(child) => Some(child)
		}

	## The child the selection goes on into.
	memory_local : GroupMemory -> MaybeWord
	memory_local = |memory|
		match memory {
			Toward(child) => Some(child)
			Unset | Stops(_) => Nothing
		}

	## The same memory on another child.
	memory_on : GroupMemory, U64 -> GroupMemory
	memory_on = |memory, child|
		match memory {
			Stops(_) => Stops(child)
			Unset | Toward(_) => Toward(child)
		}

	or_else : MaybeWord, ({} -> MaybeWord) -> MaybeWord
	or_else = |first, then|
		match first {
			Some(_) => first
			Nothing => then({})
		}

	## The first of a list, if any.
	first_of : List(U64) -> MaybeWord
	first_of = |list|
		match list.first() {
			Ok(head) => Some(head)
			Err(_) => Nothing
		}

	found : Try(U64, [NotFound]) -> MaybeWord
	found = |tried|
		match tried {
			Ok(at) => Some(at)
			Err(_) => Nothing
		}

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
		} else if F32.is_zero(a) {
			a + b
		} else {
			a
		}

	## `f32::min` on arm64 (`fminnm`): a NaN loses, and -0 beats +0.
	min32 : F32, F32 -> F32
	min32 = |a, b|
		if a < b {
			a
		} else if b < a {
			b
		} else if F32.is_nan(a) {
			b
		} else if F32.is_nan(b) {
			a
		} else if F32.is_zero(a) {
			-(-a + -b)
		} else {
			a
		}

	## `f32::EPSILON`.
	epsilon32 : F32
	epsilon32 = F32.from_bits(0x3400_0000)

	## `0.05_f32`.
	twentieth32 : F32
	twentieth32 = F32.from_bits(0x3D4C_CCCD)

	## `f64::clamp`: a NaN passes.
	clamp64 : F64, F64, F64 -> F64
	clamp64 = |value, low, high| if value < low low else if value > high high else value

	# The tree: nodes, their places and contents (`Forest`'s tree section).

	## The node at `key`. The crash on a key the table does not hold stays (here and in
	## `set_node`, `root`, `set_root`): keys come from the forest itself, and a key type tied to
	## the table that issued it is not expressible here.
	node : Forest, U64, Str -> GroupTreeNode
	node = |forest, key, who|
		match Slots.get(forest.nodes, key) {
			Held(held) => held
			NotHeld => {
				crash "GroupForest.${who}: no node ${key.to_str()}"
			}
		}

	set_node : Forest, U64, GroupTreeNode, Str -> Forest
	set_node = |forest, key, value, who|
		match Slots.replace(forest.nodes, key, value) {
			Held(nodes) => { ..forest, nodes }
			NotHeld => {
				crash "GroupForest.${who}: no node ${key.to_str()}"
			}
		}

	holds_node : Forest, U64 -> Bool
	holds_node = |forest, key|
		match Slots.get(forest.nodes, key) {
			Held(_) => True
			NotHeld => False
		}

	root : Forest, U64, Str -> U64
	root = |forest, layout, who|
		match Slots.get(forest.layouts, layout) {
			Held(held) => held
			NotHeld => {
				crash "GroupForest.${who}: no layout ${layout.to_str()}"
			}
		}

	set_root : Forest, U64, U64, Str -> Forest
	set_root = |forest, layout, key, who|
		match Slots.replace(forest.layouts, layout, key) {
			Held(layouts) => { ..forest, layouts }
			NotHeld => {
				crash "GroupForest.${who}: no layout ${layout.to_str()}"
			}
		}

	## Every layout's key, in slot order.
	layout_keys : Forest -> List(U64)
	layout_keys = |forest|
		forest.layouts.slots.map_with_index(
			|slot, index|
				match slot {
					Occupied({ version, .. }) => [Slots.key_of(index, version)]
					Vacant(_) => []
				},
		).join_map(|keys| keys)

	## Every (layout, root), in slot order.
	layout_roots : Forest -> List((U64, U64))
	layout_roots = |forest|
		forest.layouts.slots.map_with_index(
			|slot, index|
				match slot {
					Occupied({ version, value }) => [(Slots.key_of(index, version), value)]
					Vacant(_) => []
				},
		).join_map(|pairs| pairs)

	## A node's parent (none for a root, or a node the forest does not hold).
	parent : Forest, U64 -> MaybeWord
	parent = |forest, key|
		match Slots.get(forest.nodes, key) {
			Held(held) =>
				match held.place {
					Root => Nothing
					Child({ parent: above, .. }) => Some(above)
				}
			NotHeld => Nothing
		}

	## A node's size among its siblings (a root's reads as 0).
	size : Forest, U64 -> F32
	size = |forest, key| size_of(node(forest, key, "size"))

	## A node's size, read from the node itself.
	size_of : GroupTreeNode -> F32
	size_of = |held|
		match held.place {
			Root => 0.0
			Child({ size: share, .. }) => share
		}

	set_size : Forest, U64, F32 -> Forest
	set_size = |forest, key, value| {
		at = node(forest, key, "set_size")
		match at.place {
			Root => forest
			Child(placed) => set_node(forest, key, { ..at, place: Child({ ..placed, size: value }) }, "set_size")
		}
	}

	set_fill : Forest, U64, Fill -> Forest
	set_fill = |forest, key, fill| {
		at = node(forest, key, "set_fill")
		set_node(forest, key, { ..at, fill }, "set_fill")
	}

	window_at : Forest, U64 -> MaybeWord
	window_at = |forest, key|
		match Slots.get(forest.nodes, key) {
			Held(held) =>
				match held.body {
					Window({ window: leaf, .. }) => Some(leaf)
					Container(_) => Nothing
				}
			NotHeld => Nothing
		}

	is_window : Forest, U64 -> Bool
	is_window = |forest, key|
		match window_at(forest, key) {
			Some(_) => True
			Nothing => False
		}

	container : Forest, U64 -> [Is(GroupContainer), NotContainer]
	container = |forest, key|
		match Slots.get(forest.nodes, key) {
			Held(held) =>
				match held.body {
					Container(holding) => Is(holding)
					Window(_) => NotContainer
				}
			NotHeld => NotContainer
		}

	## A node that is a container by construction (a parent, a root).
	container_at : Forest, U64, Str -> GroupContainer
	container_at = |forest, key, who|
		match node(forest, key, who).body {
			Container(holding) => holding
			Window(_) => {
				crash "GroupForest.${who}: node ${key.to_str()} is a window"
			}
		}

	set_container : Forest, U64, GroupContainer, Str -> Forest
	set_container = |forest, key, holding, who| {
		at = node(forest, key, who)
		match at.body {
			Container(_) => set_node(forest, key, { ..at, body: Container(holding) }, who)
			Window(_) => {
				crash "GroupForest.${who}: node ${key.to_str()} is a window"
			}
		}
	}

	## A container's kind (a window has none).
	kind : Forest, U64 -> [Kind(GroupKind), NoKind]
	kind = |forest, key|
		match container(forest, key) {
			Is(holding) => Kind(holding.kind)
			NotContainer => NoKind
		}

	## The kind of a node that is a container by construction.
	container_kind : Forest, U64, Str -> GroupKind
	container_kind = |forest, key, who| container_at(forest, key, who).kind

	set_kind : Forest, U64, GroupKind -> Forest
	set_kind = |forest, key, made| {
		holding = container_at(forest, key, "set_kind")
		set_container(forest, key, { ..holding, kind: made }, "set_kind")
	}

	is_stacked : Forest, U64 -> Bool
	is_stacked = |forest, key|
		match kind(forest, key) {
			Kind(made) => kind_is_stacked(made)
			NoKind => False
		}

	## Whether a kind is the node's (`kind(node) == Some(made)` for a split along `orientation`).
	is_split_along : Forest, U64, BspOrientation -> Bool
	is_split_along = |forest, key, orientation|
		match kind(forest, key) {
			Kind(made) => splits_along(made, orientation)
			NoKind => False
		}

	is_region_pair : Forest, U64 -> Bool
	is_region_pair = |forest, key|
		match container(forest, key) {
			Is({ members: Regions(_), .. }) => True
			_ => False
		}

	## A master-stack layout's (primary, secondary) regions.
	regions : Forest, U64 -> [Regions(U64, U64), NoRegions]
	regions = |forest, key|
		match container(forest, key) {
			Is({ members: Regions({ first, second, primary }), .. }) =>
				match primary {
					First => Regions(first, second)
					Second => Regions(second, first)
				}
			_ => NoRegions
		}

	children : Forest, U64 -> List(U64)
	children = |forest, key|
		match Slots.get(forest.nodes, key) {
			Held(held) => children_of(held)
			NotHeld => []
		}

	## A node's children, read from the node itself: a container's members (a region pair's two
	## regions), none for a window.
	children_of : GroupTreeNode -> List(U64)
	children_of = |held|
		match held.body {
			Container(holding) =>
				match holding.members {
					Children(listed) => listed
					Regions({ first, second, .. }) => [first, second]
				}
			Window(_) => []
		}

	position : Forest, U64 -> Position
	position = |forest, key|
		match parent(forest, key) {
			Some(above) =>
				match children(forest, above).find_first_index(|child| child == key) {
					Ok(index) => At({ above, index })
					Err(_) => {
						crash "GroupForest.position: node ${key.to_str()} is not a child of ${above.to_str()}"
					}
				}
			Nothing => Unplaced
		}

	next_sibling : Forest, U64 -> MaybeWord
	next_sibling = |forest, key|
		match position(forest, key) {
			At({ above, index }) =>
				match children(forest, above).get(index + 1) {
					Ok(sibling) => Some(sibling)
					Err(_) => Nothing
				}
			Unplaced => Nothing
		}

	prev_sibling : Forest, U64 -> MaybeWord
	prev_sibling = |forest, key|
		match position(forest, key) {
			At({ above, index }) =>
				if index == 0 {
					Nothing
				} else {
					match children(forest, above).get(index - 1) {
						Ok(sibling) => Some(sibling)
						Err(_) => Nothing
					}
				}
			Unplaced => Nothing
		}

	first_child : Forest, U64 -> MaybeWord
	first_child = |forest, key| first_of(children(forest, key))

	last_child : Forest, U64 -> MaybeWord
	last_child = |forest, key|
		match children(forest, key).last() {
			Ok(child) => Some(child)
			Err(_) => Nothing
		}

	memory_at : Forest, U64 -> GroupMemory
	memory_at = |forest, key|
		match container(forest, key) {
			Is(holding) => holding.memory
			NotContainer => Unset
		}

	set_memory : Forest, U64, GroupMemory -> Forest
	set_memory = |forest, key, remembered| {
		holding = container_at(forest, key, "set_memory")
		set_container(forest, key, { ..holding, memory: remembered }, "set_memory")
	}

	## The node and its ancestors, nearest first.
	ancestors : Forest, U64 -> List(U64)
	ancestors = |forest, key|
		match parent(forest, key) {
			Some(above) => [key].concat(ancestors(forest, above))
			Nothing => [key]
		}

	is_within : Forest, U64, U64 -> Bool
	is_within = |forest, key, ancestor| ancestors(forest, key).contains(ancestor)

	common_ancestor : Forest, U64, U64 -> MaybeWord
	common_ancestor = |forest, a, b|
		if a == b {
			Some(a)
		} else {
			left = ancestors(forest, a)
			right = ancestors(forest, b)
			(depth_a, depth_b) = (left.len(), right.len())
			skip_a = if depth_a > depth_b depth_a - depth_b else 0
			skip_b = if depth_b > depth_a depth_b - depth_a else 0
			pairs = List.map2(left.drop_first(skip_a), right.drop_first(skip_b), |x, y| (x, y))
			match pairs.find_first(|(x, y)| x == y) {
				Ok((x, _)) => Some(x)
				Err(_) => Nothing
			}
		}

	last_descendant : Forest, U64 -> U64
	last_descendant = |forest, key|
		match last_child(forest, key) {
			Some(last) => last_descendant(forest, last)
			Nothing => key
		}

	next_in_subtree : Forest, U64, U64 -> MaybeWord
	next_in_subtree = |forest, key, top|
		match first_child(forest, key) {
			Some(child) => Some(child)
			Nothing =>
				ancestors(forest, key)
				.keep_while(|at| at != top)
				.fold_until(
					Nothing,
					|_, at|
						match next_sibling(forest, at) {
							Some(sibling) => Break(Some(sibling))
							Nothing => Continue(Nothing)
						},
				)
		}

	prev_in_subtree : Forest, U64, U64 -> MaybeWord
	prev_in_subtree = |forest, key, top|
		if key == top {
			Nothing
		} else {
			match prev_sibling(forest, key) {
				Some(sibling) => Some(last_descendant(forest, sibling))
				Nothing => parent(forest, key)
			}
		}

	step_in : Forest, U64, U64, Bool -> MaybeWord
	step_in = |forest, key, top, forward|
		if forward next_in_subtree(forest, key, top) else prev_in_subtree(forest, key, top)

	## Folds `step` over the subtree at `key`, each node once, children in order: in
	## `Preorder` a node before its children, in `Postorder` after them. `step` answering
	## `Break` ends the walk with that state. The one walk the traversals share: nothing is
	## materialised (as Rust's `preorder` and `postorder` iterate). Each node is read once:
	## `step` gets it with its key, and its children come from that same read.
	walk : Forest, U64, WalkOrder, s, (s, U64, GroupTreeNode -> Visit(s)) -> s
	walk = |forest, key, order, state, step|
		match walk_node(forest, key, order, state, step) {
			Continue(walked) => walked
			Break(walked) => walked
		}

	walk_node : Forest, U64, WalkOrder, s, (s, U64, GroupTreeNode -> Visit(s)) -> Visit(s)
	walk_node = |forest, key, order, state, step| {
		held = node(forest, key, "walk")
		entered =
			match order {
				Preorder => step(state, key, held)
				Postorder => Continue(state)
			}
		match entered {
			Break(stopped) => Break(stopped)
			Continue(going) => {
				through = children_of(held).fold_until(
					Continue(going),
					|visited, child|
						match visited {
							Continue(before) =>
								match walk_node(forest, child, order, before, step) {
									Continue(after) => Continue(Continue(after))
									Break(after) => Break(Break(after))
								}
							Break(_) => Break(visited)
						},
				)
				match (through, order) {
					(Continue(below), Postorder) => step(below, key, held)
					_ => through
				}
			}
		}
	}

	create_root : Forest -> (Forest, U64)
	create_root = |forest| {
		(nodes, key) = Slots.insert(
			forest.nodes,
			{
				place: Root,
				fill: Tiled,
				body: Container({ kind: Split(Horizontal), members: Children([]), memory: Unset }),
			},
		)
		({ ..forest, nodes }, key)
	}

	placement_parent : Forest, Placement -> MaybeWord
	placement_parent = |forest, placement| {
		above =
			match placement {
				Back(key) => if holds_node(forest, key) Some(key) else Nothing
				Before(sibling) | After(sibling) => parent(forest, sibling)
			}
		match above {
			Some(key) => if is_window(forest, key) Nothing else Some(key)
			Nothing => Nothing
		}
	}

	## The children of a regular container, which only those accept structural edits.
	listed_children : Forest, U64, Str -> List(U64)
	listed_children = |forest, key, who|
		match container_at(forest, key, who).members {
			Children(listed) => listed
			Regions(_) => {
				crash "GroupForest.${who}: node ${key.to_str()} is a region pair"
			}
		}

	set_children : Forest, U64, List(U64), Str -> Forest
	set_children = |forest, key, listed, who| {
		holding = container_at(forest, key, who)
		set_container(forest, key, { ..holding, members: Children(listed) }, who)
	}

	set_place : Forest, U64, GroupPlace -> Forest
	set_place = |forest, key, placed| {
		at = node(forest, key, "set_place")
		set_node(forest, key, { ..at, place: placed }, "set_place")
	}

	## The place at the back of `above`'s children.
	back_of : Forest, U64 -> Place
	back_of = |forest, above| { above, index: children(forest, above).len() }

	## The place just after a placed node's.
	after_place : Place -> Place
	after_place = |at| { ..at, index: at.index + 1 }

	## Where `placement` puts a child of `above`, resolved after the moving node is taken out (the
	## move's resolution; inserts take a `Place`). The crash for an unplaced sibling stays here:
	## `move_node` found the sibling's parent before taking the node out, and the type cannot carry
	## that.
	placement_place : Forest, U64, Placement -> Place
	placement_place = |forest, above, placement| {
		index =
			match placement {
				Back(_) => children(forest, above).len()
				Before(sibling) =>
					match position(forest, sibling) {
						At({ index: at, .. }) => at
						Unplaced => {
							crash "GroupForest.link: sibling ${sibling.to_str()} is not placed"
						}
					}
				After(sibling) =>
					match position(forest, sibling) {
						At({ index: at, .. }) => at + 1
						Unplaced => {
							crash "GroupForest.link: sibling ${sibling.to_str()} is not placed"
						}
					}
			}
		{ above, index }
	}

	## `key` (unplaced) put at `place`, with `held` as its size. Two crashes stay, both about a
	## key read just before: `listed_children`'s for a container that is a region pair (or not a
	## container), and the list insert's for an index past the children (every place is read from
	## them); neither is expressible as a type here.
	link : Forest, U64, Place, F32 -> Forest
	link = |forest, key, { above, index }, held| {
		listed = listed_children(forest, above, "link")
		inserted =
			match listed.insert(index, key) {
				Ok(list) => list
				Err(_) => {
					crash "GroupForest.link: no place ${index.to_str()} in ${above.to_str()}"
				}
			}
		set_place(set_children(forest, above, inserted, "link"), key, Child({ parent: above, size: held }))
	}

	## `key` taken out of its parent's children (its memory of it kept).
	unlink : Forest, U64 -> Forest
	unlink = |forest, key|
		match position(forest, key) {
			At({ above, index }) => {
				listed = listed_children(forest, above, "unlink")
				set_place(set_children(forest, above, listed.drop_at(index), "unlink"), key, Root)
			}
			Unplaced => forest
		}

	## A new container at `place` (the caller resolved it from what it knows).
	insert : Forest, Place -> (Forest, U64)
	insert = |forest, place| {
		(created, key) = create_root(forest)
		(link(created, key, place, 1.0), key)
	}

	move_node : Forest, U64, Placement -> (Forest, Bool)
	move_node = |forest, key, placement|
		match placement_parent(forest, placement) {
			Nothing => (forest, False)
			Some(above) => {
				anchored_on_itself =
					match placement {
						Before(anchor) | After(anchor) => anchor == key
						Back(_) => False
					}
				if !holds_node(forest, key) or anchored_on_itself {
					(forest, False)
				} else {
					old_parent = parent(forest, key)
					changed_parent = old_parent != Some(above)
					old_is_pair =
						match old_parent {
							Some(old) => is_region_pair(forest, old)
							Nothing => False
						}
					if is_region_pair(forest, above) or old_is_pair or is_region_pair(forest, key) or (changed_parent and is_within(forest, above, key)) {
						(forest, False)
					} else {
						in_place =
							!changed_parent
							and (
								match placement {
									Back(_) => next_sibling(forest, key) == Nothing
									Before(anchor) => next_sibling(forest, key) == Some(anchor)
									After(anchor) => prev_sibling(forest, key) == Some(anchor)
								}
							)
						if in_place {
							(forest, False)
						} else {
							held = size(forest, key)
							taken =
								match old_parent {
									Some(old) if changed_parent => detach(forest, key, old)
									_ => unlink(forest, key)
								}
							linked = link(taken, key, placement_place(taken, above, placement), if changed_parent 1.0 else held)
							pruned =
								match old_parent {
									Some(old) if changed_parent => prune_container(linked, old)
									_ => linked
								}
							(pruned, True)
						}
					}
				}
			}
		}

	## A move whose answer is not read.
	moved : Forest, U64, Placement -> Forest
	moved = |forest, key, placement| {
		(after, _) = move_node(forest, key, placement)
		after
	}

	remove_node : Forest, U64 -> Forest
	remove_node = |forest, key|
		match parent(forest, key) {
			Some(above) =>
				if is_region_pair(forest, above) {
					forest
				} else {
					delete_subtree(prune_container(detach(forest, key, above), above), key)
				}
			Nothing => delete_subtree(forest, key)
		}

	detach : Forest, U64, U64 -> Forest
	detach = |forest, key, above| {
		unlinked = unlink(detach_selection(forest, key), key)
		if is_window(unlinked, key) {
			(normalized, done) = normalize_weights(unlinked, above)
			if done {
				normalized
			} else {
				children(normalized, above).fold(normalized, |at, child| set_size(at, child, 1.0))
			}
		} else {
			unlinked
		}
	}

	prune_container : Forest, U64 -> Forest
	prune_container = |forest, key|
		match position(forest, key) {
			Unplaced => forest
			At(at) =>
				if is_region_pair(forest, at.above) {
					forest
				} else {
					listed = children(forest, key)
					if listed.len() > 1 {
						forest
					} else {
						lifted =
							match first_of(listed) {
								Some(child) => {
									unlinked = unlink(forest, child)
									held = size(unlinked, key)
									link(unlinked, child, after_place(at), held)
								}
								Nothing => forest
							}
						removed = unlink(detach_selection(lifted, key), key)
						(nodes, _) = Slots.remove(removed.nodes, key)
						prune_container({ ..removed, nodes }, at.above)
					}
				}
		}

	delete_subtree : Forest, U64 -> Forest
	delete_subtree = |forest, key| {
		(nodes, took) = Slots.remove(forest.nodes, key)
		match took {
			Took(held) =>
				match held.body {
					Container(holding) => {
						listed =
							match holding.members {
								Children(list) => list
								Regions({ first, second, .. }) => [first, second]
							}
						listed.fold({ ..forest, nodes }, |at, child| delete_subtree(at, child))
					}
					Window(_) => { ..forest, nodes }
				}
			NotHeld => { ..forest, nodes }
		}
	}

	copy_tile : Forest, U64 -> (Forest, U64)
	copy_tile = |forest, source| {
		at = node(forest, source, "copy_tile")
		(stamped, body) =
			match at.body {
				Window({ window: leaf, .. }) => ({ ..forest, next_stamp: forest.next_stamp + 1 }, Window({ window: leaf, stamp: forest.next_stamp }))
				Container(holding) => (forest, Container({ kind: holding.kind, members: Children([]), memory: Unset }))
			}
		(nodes, key) = Slots.insert(stamped.nodes, { place: Root, fill: at.fill, body })
		({ ..stamped, nodes }, key)
	}

	clone_branch : Forest, U64 -> (Forest, U64)
	clone_branch = |forest, source| {
		(copied, top) = copy_tile(forest, source)
		cloned = clone_children(copied, source, top)
		paired =
			match container(cloned, source) {
				Is({ members: Regions({ primary, .. }), .. }) => {
					holding = container_at(cloned, top, "clone_branch")
					match holding.members {
						Children([first, second]) => set_container(cloned, top, { ..holding, members: Regions({ first, second, primary }) }, "clone_branch")
						_ => {
							crash "GroupForest.clone_branch: region pair ${top.to_str()} cloned without two regions"
						}
					}
				}
				_ => cloned
			}
		(paired, top)
	}

	## Each of `original`'s children copied under `copy`, in preorder (each subtree before the
	## next sibling), the memory carried to the copy.
	clone_children : Forest, U64, U64 -> Forest
	clone_children = |forest, original, copy|
		children(forest, original).fold(
			forest,
			|at, old| {
				(copied, new) = copy_tile(at, old)
				held = size(copied, old)
				linked = link(copied, new, back_of(copied, copy), held)
				remembered = memory_at(linked, original)
				with_memory =
					if memory_last(remembered) == Some(old) {
						set_memory(linked, copy, memory_on(remembered, new))
					} else {
						linked
					}
				if children(with_memory, old).is_empty() {
					with_memory
				} else {
					clone_children(with_memory, old, new)
				}
			},
		)

	## `top`'s two children made its regions, `primary` the master region.
	set_region_pair : Forest, U64, U64 -> Forest
	set_region_pair = |forest, top, primary| {
		holding = container_at(forest, top, "set_region_pair")
		match holding.members {
			Children([first, second]) => {
				side = if first == primary First else Second
				set_container(forest, top, { ..holding, members: Regions({ first, second, primary: side }) }, "set_region_pair")
			}
			_ => {
				crash "GroupForest.set_region_pair: node ${top.to_str()} does not have two region containers"
			}
		}
	}

	set_primary_region_first : Forest, U64, Bool -> Forest
	set_primary_region_first = |forest, top, first_wanted| {
		holding = container_at(forest, top, "set_primary_region_first")
		match holding.members {
			Regions({ first, second, primary }) => {
				is_first =
					match primary {
						First => True
						Second => False
					}
				if is_first != first_wanted {
					swapped = Regions({ first: second, second: first, primary: if first_wanted First else Second })
					set_container(forest, top, { ..holding, members: swapped }, "set_primary_region_first")
				} else {
					forest
				}
			}
			Children(_) => {
				crash "GroupForest.set_primary_region_first: node ${top.to_str()} is not a region pair"
			}
		}
	}

	## The sum of the children's sizes, each at least zero (`Iterator::sum`, from -0).
	children_total : Forest, U64 -> F32
	children_total = |forest, above| {
		var $total = F32.from_bits(0x8000_0000)
		for child in children(forest, above) {
			$total = $total + max32(size(forest, child), 0.0)
		}
		$total
	}

	## Sibling proportions kept with a mean weight of one.
	normalize_weights : Forest, U64 -> (Forest, Bool)
	normalize_weights = |forest, above| {
		listed = children(forest, above)
		var $total = 0.0
		for child in listed {
			$total = $total + max32(size(forest, child), 0.0)
		}
		total = $total
		if !F32.is_finite(total) or total <= epsilon32 {
			(forest, False)
		} else {
			scale = listed.len().to_f32() / total
			(listed.fold(forest, |at, child| set_size(at, child, size(at, child) * scale)), True)
		}
	}

	## `new` takes `old`'s size, `old` keeps none (siblings both).
	assume_size_of : Forest, U64, U64 -> Forest
	assume_size_of = |forest, new, old|
		if parent(forest, new) != parent(forest, old) {
			crash "GroupForest.assume_size_of: ${new.to_str()} and ${old.to_str()} are not siblings"
		} else {
			held = size(forest, old)
			set_size(set_size(forest, old, 0.0), new, held)
		}

	# The selection: each container's memory.

	current_selection : Forest, U64 -> U64
	current_selection = |forest, key|
		match memory_at(forest, key) {
			Toward(child) => current_selection(forest, child)
			_ => key
		}

	last_selection : Forest, U64 -> MaybeWord
	last_selection = |forest, key| memory_last(memory_at(forest, key))

	local_selection_of : Forest, U64 -> MaybeWord
	local_selection_of = |forest, key| memory_local(memory_at(forest, key))

	select_locally : Forest, U64 -> (Forest, Bool)
	select_locally = |forest, key|
		match parent(forest, key) {
			Some(above) => {
				old = memory_at(forest, above)
				(set_memory(forest, above, Toward(key)), memory_last(old) != Some(key))
			}
			Nothing => (forest, False)
		}

	## A local selection whose answer is not read.
	selected_locally : Forest, U64 -> Forest
	selected_locally = |forest, key| {
		(after, _) = select_locally(forest, key)
		after
	}

	select_node : Forest, U64 -> Forest
	select_node = |forest, chosen| {
		at = node(forest, chosen, "select_node")
		stopped =
			match at.body {
				Container({ memory: Toward(child), .. }) => set_memory(forest, chosen, Stops(child))
				_ => forest
			}
		lead_up(stopped, chosen)
	}

	## Each ancestor's memory toward the child the walk came from.
	lead_up : Forest, U64 -> Forest
	lead_up = |forest, key|
		match parent(forest, key) {
			Some(above) => lead_up(set_memory(forest, above, Toward(key)), above)
			Nothing => forest
		}

	detach_selection : Forest, U64 -> Forest
	detach_selection = |forest, key|
		match parent(forest, key) {
			Some(above) => {
				remembered = memory_at(forest, above)
				if memory_last(remembered) == Some(key) {
					replacement = or_else(next_sibling(forest, key), |{}| prev_sibling(forest, key))
					set_memory(
						forest,
						above,
						match replacement {
							Some(child) => memory_on(remembered, child)
							Nothing => Unset
						},
					)
				} else {
					forest
				}
			}
			Nothing => {
				crash "GroupForest.detach_selection: node ${key.to_str()} is not placed"
			}
		}

	# Walks over a subtree's windows.

	## `preferred` first, then the children in order (or reversed) without it.
	ordered_children : Forest, U64, MaybeWord, Bool -> List(U64)
	ordered_children = |forest, key, preferred, reverse| {
		rest =
			match preferred {
				Some(chosen) => children(forest, key).drop_if(|child| child == chosen)
				Nothing => children(forest, key)
			}
		ordered = if reverse rest.rev() else rest
		match preferred {
			Some(chosen) => [chosen].concat(ordered)
			Nothing => ordered
		}
	}

	participating_child : Forest, U64, Bool, (U64 -> Bool) -> MaybeWord
	participating_child = |forest, key, reverse, participates|
		found(ordered_children(forest, key, local_selection_of(forest, key), reverse).find_first(participates))

	takes_part : Forest, U64 -> Bool
	takes_part = |forest, window_word| !forest.left_out.contains(window_word)

	## The first window taking part, walking the subtree in the order `order` gives each
	## container (`find_window`).
	find_window : Forest, U64, (U64 -> (MaybeWord, Bool)) -> [Found(U64, U64), NotFound]
	find_window = |forest, key, order|
		match window_at(forest, key) {
			Some(leaf) => if takes_part(forest, leaf) Found(key, leaf) else NotFound
			Nothing => {
				(preferred, reverse) = order(key)
				ordered_children(forest, key, preferred, reverse).fold_until(
					NotFound,
					|_, child|
						match find_window(forest, child, order) {
							Found(at, leaf) => Break(Found(at, leaf))
							NotFound => Continue(NotFound)
						},
				)
			}
		}

	find_best_focus_target : Forest, U64 -> [Found(U64, U64), NotFound]
	find_best_focus_target = |forest, key| find_window(forest, key, |at| (last_selection(forest, at), False))

	has_focus_target : Forest, U64 -> Bool
	has_focus_target = |forest, key|
		match find_best_focus_target(forest, key) {
			Found(_, _) => True
			NotFound => False
		}

	## Whether windows are left out and none of the layout's takes part.
	nothing_takes_part : Forest, U64 -> Bool
	nothing_takes_part = |forest, layout| !forest.left_out.is_empty() and !has_focus_target(forest, root(forest, layout, "nothing_takes_part"))

	## The visible windows under a node, and whether any was visited (`visit_visible_windows`).
	visible_walk : Forest, U64, Limit -> (List(U64), Bool)
	visible_walk = |forest, key, group_limit|
		match window_at(forest, key) {
			Some(leaf) => if takes_part(forest, leaf) ([leaf], True) else ([], False)
			Nothing => {
				(preferred, limit) = if is_stacked(forest, key) (local_selection_of(forest, key), group_limit) else (Nothing, All)
				ordered_children(forest, key, preferred, False).fold_until(
					([], False),
					|(listed, seen), child| {
						(under, child_seen) = visible_walk(forest, child, group_limit)
						now = seen or child_seen
						after = (listed.concat(under), now)
						stop =
							match limit {
								First => True
								FirstVisible => now
								All => False
							}
						if stop Break(after) else Continue(after)
					},
				)
			}
		}

	visible_windows_under : Forest, U64 -> List(U64)
	visible_windows_under = |forest, key| {
		group_limit = if forest.left_out.is_empty() First else FirstVisible
		(windows, _) = visible_walk(forest, key, group_limit)
		windows
	}

	## Whether a node takes part: a window when it is not left out, a container when a child does.
	node_participates : Forest, GroupTreeNode, (U64 -> Bool) -> Bool
	node_participates = |forest, held, child_participates|
		match held.body {
			Window({ window: leaf, .. }) => takes_part(forest, leaf)
			Container(_) => children_of(held).any(child_participates)
		}

	## The nodes under `top` taking part: every one while no window is left out, else those marked.
	participating_nodes : Forest, U64 -> [Everyone, Only(TakingPart)]
	participating_nodes = |forest, top|
		if forest.left_out.is_empty() {
			Everyone
		} else {
			Only(taking_part_under(forest, top))
		}

	## The nodes under `top` taking part, marked bottom-up (a container takes part when a child
	## does).
	taking_part_under : Forest, U64 -> TakingPart
	taking_part_under = |forest, top|
		walk(
			forest,
			top,
			Postorder,
			Slots.unmarked(forest.nodes),
			|marks, key, held| Continue(if node_participates(forest, held, |child| Slots.marked(marks, child)) Slots.mark(marks, key) else marks),
		)

	# The shape (`Forest::shape`): the traditional geometry's input.

	## A layout's shape for the traditional geometry: the nodes taking part, laid out from the
	## root, or from the master region when a tiled root's stack region takes none; none when that
	## node takes no part. Each stack is labelled with its key; the windows' words become the
	## geometry's `WindowId`s here, with the constraints the host knows.
	shape : Forest, StackFramesQuestion -> [Shaped(Laid(U64)), Unshaped]
	shape = |forest, question|
		match laid_out(forest, question.layout) {
			NothingLaidOut => Unshaped
			LaidOut(active, start) => {
				Shaped(shape_from(forest, question, active, start))
			}
		}

	## The shape laid out from `start`.
	shape_from : Forest, StackFramesQuestion, TakingPart, U64 -> Laid(U64)
	shape_from = |forest, question, active, start| {
		known = Dict.from_list(question.known.map(|k| (k.window, k.constraints)))
		Laid.{
			screen: question.screen,
			area: Geometry.tiling_area(question.screen, question.gaps),
			gaps: question.gaps,
			stack_offset: question.stack_offset,
			stack_line: question.stack_line,
			root: shape_node(forest, active, known, start),
		}
	}

	## A layout pass with its group bars (`Forest::project`): each window's frame and the bar of
	## each stack `bars` asks for, each stack's frame as the geometry placed it; none while the
	## layout's root fills.
	project : Forest, StackFramesQuestion, GroupBars -> GroupProjected
	project = |forest, question, bars|
		match laid_out(forest, question.layout) {
			NothingLaidOut => { frames: [], bars: [] }
			LaidOut(active, start) => {
				placed = Traditional.placed(shape_from(forest, question, active, start))
				stacks = Dict.from_list(placed.stacks)
				top = root(forest, question.layout, "project")
				barred = if is_tiled(node(forest, top, "project").fill) bars_from(forest, start, active, stacks, bars, []) else []
				frames : List(Frame)
				frames = placed.frames.map(|(window, frame)| { window, frame })
				{ frames, bars: barred }
			}
		}

	## The bars under `key` (taking part) after `out`: a node that fills ends the walk below it
	## (and along the selection path drops every bar found).
	bars_from : Forest, U64, TakingPart, Dict(U64, Rect), GroupBars, List(GroupBar) -> List(GroupBar)
	bars_from = |forest, key, active, stacks, bars, out| {
		held = node(forest, key, "bars_from")
		if is_tiled(held.fill) {
			match held.body {
				Window(_) => out
				Container(holding) => {
					taking_part = children(forest, key).keep_if(|child| Slots.marked(active, child))
					shown = |{}| participating_child(forest, key, False, |child| Slots.marked(active, child))
					(with_bar, chosen) =
						match holding.kind {
							Stack(orientation) => {
								focused =
									match shown({}) {
										Some(child) => child
										Nothing => crash "GroupForest.bars_from: stack ${key.to_str()} takes part with no child taking part"
									}
								frame =
									match stacks.get(key) {
										Ok(placed) => placed
										Err(KeyNotFound) => crash "GroupForest.bars_from: stack ${key.to_str()} was not placed"
									}
								(out.append(group_bar(forest, key, orientation, frame, taking_part, focused)), Some(focused))
							}

							Split(_) =>
								match bars {
									SelectionPath => (out, shown({}))
									Every => (out, Nothing)
								}
						}
					walked = |child|
						match bars {
							Every => True
							SelectionPath =>
								match chosen {
									Some(on_path) => on_path == child
									Nothing => False
								}
						}
					taking_part.fold(with_bar, |acc, child| if walked(child) bars_from(forest, child, active, stacks, bars, acc) else acc)
				}
			}
		} else {
			match bars {
				SelectionPath => []
				Every => out
			}
		}
	}

	## A stack's bar: one window per child holding one that takes part (the child's focus
	## target), and which of them stands for the focused child.
	group_bar : Forest, U64, BspOrientation, Rect, List(U64), U64 -> GroupBar
	group_bar = |forest, stack, orientation, frame, taking_part, focused| {
		(windows, selected) =
			taking_part.fold(
				([], 0),
				|(listed, chosen), child|
					match find_best_focus_target(forest, child) {
						Found(_, window) => (listed.append(window), if child == focused listed.len() else chosen)
						NotFound => (listed, chosen)
					},
			)
		{ stack, orientation, frame, windows, selected }
	}

	# rift-cli's views (`views.rs`).

	## The role `key` plays in the layout rooted at `top`.
	role : Forest, U64, U64 -> GroupRole
	role = |forest, top, key|
		match regions(forest, top) {
			Regions(master, stack) => if key == master Master else if key == stack Stack else Plain
			NoRegions => Plain
		}

	## A layout's nodes as rift's container view shows them (`Forest::container_view`).
	container_view : Forest, U64 -> GroupContainerView
	container_view = |forest, layout| {
		top = root(forest, layout, "container_view")
		container_of(forest, top, top, selection(forest, layout))
	}

	container_of : Forest, U64, U64, U64 -> GroupContainerView
	container_of = |forest, top, key, selected| {
		held = node(forest, key, "container_of")
		{
			node: key,
			selected: key == selected,
			weight: match held.place {
				Root => AtRoot
				Child({ size: share, .. }) => Weight(share)
			},
			fill: held.fill,
			role: role(forest, top, key),
			held: match held.body {
				Window({ window, .. }) => Window(window)
				Container(holding) => Container({ kind: holding.kind, children: children(forest, key).map(|child| container_of(forest, top, child, selected)) })
			},
		}
	}

	## A layout's drawing as rows, in preorder (`Forest::rows`).
	rows : Forest, U64 -> List(GroupRow)
	rows = |forest, layout| {
		top = root(forest, layout, "rows")
		row_of(forest, top, top, [], [], [])
	}

	## One node's row and its children's after `out`: `lead` the glyphs before it, `rest` the
	## glyphs its children's rows start with.
	row_of : Forest, U64, U64, List(GroupBranch), List(GroupBranch), List(GroupRow) -> List(GroupRow)
	row_of = |forest, top, key, lead, rest, out| {
		held = node(forest, key, "row_of")
		mark =
			match held.place {
				Root => Root
				Child({ parent: above, .. }) =>
					match local_selection_of(forest, above) {
						Some(chosen) => if chosen == key Chosen else NotChosen
						Nothing => NotChosen
					}
			}
		drawn =
			match held.body {
				Window({ window, .. }) => Window(window)
				Container(holding) => Container({ kind: holding.kind, total: children_total(forest, key) })
			}
		row = { branches: lead, mark, node: key, held: drawn, size: size(forest, key), role: role(forest, top, key) }
		listed = children(forest, key)
		(_, drawn_out) =
			listed.fold(
				(listed.len(), out.append(row)),
				|(left, acc), child| {
					(glyph, below) = if left == 1 (Last, Blank) else (Tee, Through)
					(left - 1, row_of(forest, top, child, rest.append(glyph), rest.append(below), acc))
				},
			)
		drawn_out
	}

	## What a layout pass lays out (`laid_out`): the nodes taking part, and the node laid out from.
	laid_out : Forest, U64 -> [LaidOut(TakingPart, U64), NothingLaidOut]
	laid_out = |forest, layout| {
		top = root(forest, layout, "laid_out")
		active = taking_part_under(forest, top)
		start =
			if is_tiled(node(forest, top, "laid_out").fill) {
				match regions(forest, top) {
					Regions(master, stack) => if Slots.marked(active, stack) top else master
					NoRegions => top
				}
			} else {
				top
			}
		if Slots.marked(active, start) LaidOut(active, start) else NothingLaidOut
	}

	## A node taking part as the geometry takes it: a container keeps its children taking part, a
	## stack the child shown (its selection if it takes part, else the first that does) between
	## those before and after it.
	shape_node : Forest, TakingPart, Dict(U64, WindowConstraints), U64 -> Tree(U64)
	shape_node = |forest, active, known, key| {
		held = node(forest, key, "shape_node")
		match held.body {
			Window({ window, .. }) => Tree.Window({ window: Geometry.window_id(window), constraints: Geometry.known_constraints(known, window), fill: held.fill })
			Container(holding) => {
				branch = |child| Branch.{ size: size(forest, child), node: shape_node(forest, active, known, child) }
				taking_part = children(forest, key).keep_if(|child| Slots.marked(active, child))
				match holding.kind {
					Stack(orientation) => {
						focused =
							match participating_child(forest, key, False, |child| Slots.marked(active, child)) {
								Some(chosen) => chosen
								Nothing => crash "GroupForest.shape_node: stack ${key.to_str()} takes part with no child taking part"
							}
						around =
							taking_part.fold(
								Before([]),
								|at, child|
									match at {
										Before(before) => if child == focused Around(before, []) else Before(before.append(child))
										Around(before, after) => Around(before, after.append(child))
									},
							)
						match around {
							Around(before, after) =>
								Tree.Stack(TreeStack.{ label: key, horizontal: same_orientation(orientation, Horizontal), fill: held.fill, before: before.map(branch), focused: branch(focused), after: after.map(branch) })

							Before(_) => crash "GroupForest.shape_node: stack ${key.to_str()} shows ${focused.to_str()}, not a child taking part"
						}
					}

					Split(orientation) =>
						match taking_part {
							[first, .. as rest] => Tree.Axis(TreeAxis.{ horizontal: same_orientation(orientation, Horizontal), fill: held.fill, first: branch(first), rest: rest.map(branch) })
							[] => crash "GroupForest.shape_node: container ${key.to_str()} takes part with no child taking part"
						}
				}
			}
		}
	}

	# The window index, derived.

	## Every (window, layout, leaf), by window, then by stamp.
	window_index : Forest -> List((U64, U64, U64))
	window_index = |forest| indexed_where(forest, |_| True)

	## The index's entries for the windows `keep` keeps, in the index's order (by window, then
	## stamp): each layout walked once, only the kept leaves gathered and sorted.
	indexed_where : Forest, (U64 -> Bool) -> List((U64, U64, U64))
	indexed_where = |forest, keep|
		layout_roots(forest)
		.fold(
			[],
			|gathered, (layout, top)|
				walk(
					forest,
					top,
					Preorder,
					gathered,
					|held, key, at|
						match at.body {
							Window({ window: leaf, stamp }) if keep(leaf) => Continue(held.append((leaf, stamp, layout, key)))
							_ => Continue(held)
						},
				),
		)
		.sort_with(
			|(wa, sa, _, _), (wb, sb, _, _)|
				if wa < wb {
					Before
				} else if wa > wb {
					After
				} else if sa < sb {
					Before
				} else if sa > sb {
					After
				} else {
					Same
				},
		)
		.map(|(leaf, _, layout, key)| (leaf, layout, key))

	occurrences : Forest, U64 -> List((U64, U64))
	occurrences = |forest, window_word|
		indexed_where(forest, |leaf| leaf == window_word).map(|(_, layout, key)| (layout, key))

	## The leaf holding `window_word` in `layout` (the first to enter).
	window_node : Forest, U64, U64 -> MaybeWord
	window_node = |forest, layout, window_word| {
		best = walk(
			forest,
			root(forest, layout, "window_node"),
			Preorder,
			NoLeaf,
			|chosen, key, at|
				match at.body {
					Window({ window: leaf, stamp }) if leaf == window_word =>
						match chosen {
							Leaf(_, held) if held <= stamp => Continue(chosen)
							_ => Continue(Leaf(key, stamp))
						}
					_ => Continue(chosen)
				},
		)
		match best {
			Leaf(key, _) => Some(key)
			NoLeaf => Nothing
		}
	}

	windows_with_nodes : Forest, U64 -> List(U64)
	windows_with_nodes = |forest, layout|
		walk(
			forest,
			root(forest, layout, "windows_with_nodes"),
			Preorder,
			[],
			|held, _, at|
				match at.body {
					Window({ window: leaf, .. }) => Continue(held.append(leaf))
					Container(_) => Continue(held)
				},
		)

	## A new leaf's window: it enters the layout now.
	set_window : Forest, U64, U64 -> Forest
	set_window = |forest, key, window_word| {
		at = node(forest, key, "set_window")
		stamped = { ..forest, next_stamp: forest.next_stamp + 1 }
		set_node(stamped, key, { ..at, body: Window({ window: window_word, stamp: forest.next_stamp }) }, "set_window")
	}

	# The operations both systems share.

	set_window_participation : Forest, BspParticipation -> (Forest, Bool)
	set_window_participation = |forest, { window: window_word, participation }|
		match participation {
			TakesPart => ({ ..forest, left_out: forest.left_out.remove(window_word) }, forest.left_out.contains(window_word))
			LeftOut => ({ ..forest, left_out: forest.left_out.insert(window_word) }, !forest.left_out.contains(window_word))
		}

	has_windows_for_app : Forest, BspAppIn -> Bool
	has_windows_for_app = |forest, { layout, windows }| windows_with_nodes(forest, layout).any(|leaf| windows.contains(leaf))

	create_layout : Forest -> (Forest, U64)
	create_layout = |forest| {
		(created, top) = create_root(forest)
		(layouts, layout) = Slots.insert(created.layouts, top)
		({ ..created, layouts }, layout)
	}

	clone_layout : Forest, U64 -> (Forest, U64)
	clone_layout = |forest, layout| {
		(cloned, top) = clone_branch(forest, root(forest, layout, "clone_layout"))
		(layouts, clone) = Slots.insert(cloned.layouts, top)
		({ ..cloned, layouts }, clone)
	}

	remove_layout : Forest, U64 -> Forest
	remove_layout = |forest, layout| {
		(layouts, took) = Slots.remove(forest.layouts, layout)
		match took {
			Took(top) => remove_node({ ..forest, layouts }, top)
			NotHeld => {
				crash "GroupForest.remove_layout: no layout ${layout.to_str()}"
			}
		}
	}

	## The window's leaves removed; the layouts it was in, in the index's order.
	remove_window : Forest, U64 -> (Forest, List(U64))
	remove_window = |forest, window_word| {
		cleared = { ..forest, left_out: forest.left_out.remove(window_word) }
		removed = occurrences(cleared, window_word)
		(removed.fold(cleared, |at, (_, key)| remove_node(at, key)), removed.map(|(layout, _)| layout))
	}

	replace_window : Forest, BspReplacement -> Forest
	replace_window = |forest, { from, to }| {
		left_out = if forest.left_out.contains(from) forest.left_out.remove(from).insert(to) else forest.left_out
		replaced = { ..forest, left_out }
		if from == to {
			replaced
		} else {
			occurrences(replaced, from).fold(replaced, |at, (_, key)| set_window(at, key, to))
		}
	}

	close_app : Forest, List(U64) -> Forest
	close_app = |forest, windows| {
		kept = { ..forest, left_out: Set.from_list(forest.left_out.to_list().drop_if(|leaf| windows.contains(leaf))) }
		indexed_where(kept, |leaf| windows.contains(leaf))
		.fold(kept, |at, (_, _, key)| remove_node(at, key))
	}

	contains_window : Forest, U64, U64 -> Bool
	contains_window = |forest, layout, window_word|
		match window_node(forest, layout, window_word) {
			Some(_) => True
			Nothing => False
		}

	selection : Forest, U64 -> U64
	selection = |forest, layout| {
		top = root(forest, layout, "selection")
		current = current_selection(forest, top)
		if current != top and !forest.left_out.is_empty() and !has_focus_target(forest, current) {
			match find_best_focus_target(forest, top) {
				Found(key, _) => key
				NotFound => top
			}
		} else {
			current
		}
	}

	## A container's chosen child: the one the selection goes into, else the one remembered.
	local_selection : Forest, U64 -> MaybeWord
	local_selection = |forest, key| or_else(local_selection_of(forest, key), |{}| last_selection(forest, key))

	focused_leaf : Forest, U64 -> [Found(U64, U64), NotFound]
	focused_leaf = |forest, layout| {
		top = root(forest, layout, "focused_leaf")
		current = current_selection(forest, top)
		match find_best_focus_target(forest, current) {
			Found(key, leaf) => Found(key, leaf)
			NotFound => if current != top find_best_focus_target(forest, top) else NotFound
		}
	}

	selected_window : Forest, U64 -> MaybeWord
	selected_window = |forest, layout|
		match focused_leaf(forest, layout) {
			Found(_, leaf) => Some(leaf)
			NotFound => Nothing
		}

	selection_contains_window : Forest, U64, U64 -> Bool
	selection_contains_window = |forest, layout, window_word| {
		chosen = selection(forest, layout)
		match window_node(forest, layout, window_word) {
			Some(key) => is_within(forest, key, chosen)
			Nothing => False
		}
	}

	window_in_direction : Forest, BspLayoutDirection -> MaybeWord
	window_in_direction = |forest, { layout, direction }| {
		reverse = backwards(direction)
		order = |key| {
			made = container_kind(forest, key, "window_in_direction")
			preferred = if kind_is_stacked(made) or !same_orientation(kind_orientation(made), orientation_of(direction)) local_selection_of(forest, key) else Nothing
			(preferred, reverse)
		}
		match find_window(forest, root(forest, layout, "window_in_direction"), order) {
			Found(_, leaf) => Some(leaf)
			NotFound => Nothing
		}
	}

	has_any_fullscreen_node : Forest, U64 -> Bool
	has_any_fullscreen_node = |forest, layout|
		walk(
			forest,
			root(forest, layout, "has_any_fullscreen_node"),
			Preorder,
			False,
			|_, key, at| if !is_tiled(at.fill) and (forest.left_out.is_empty() or has_focus_target(forest, key)) Break(True) else Continue(False),
		)

	stacked_selection_container : Forest, U64 -> MaybeWord
	stacked_selection_container = |forest, layout| {
		chosen = selection(forest, layout)
		if is_window(forest, chosen) {
			found(ancestors(forest, chosen).drop_first(1).find_first(|key| is_stacked(forest, key)))
		} else if is_stacked(forest, chosen) {
			Some(chosen)
		} else {
			found(children(forest, chosen).find_first(|key| is_stacked(forest, key)))
		}
	}

	stack_members : Forest, U64, U64 -> List(U64)
	stack_members = |forest, layout, window_word|
		match window_node(forest, layout, window_word) {
			Some(key) =>
				match parent(forest, key) {
					Some(above) =>
						if is_stacked(forest, above) {
							children(forest, above).join_map(
								|child|
									match window_at(forest, child) {
										Some(leaf) => [leaf]
										Nothing => []
									},
							)
						} else {
							[]
						}
					Nothing => []
				}
			Nothing => []
		}

	listed_windows : Forest, BspWindowsQuery -> List(U64)
	listed_windows = |forest, { layout, query }|
		match query {
			All => windows_with_nodes(forest, layout)
			Visible => visible_windows_under(forest, root(forest, layout, "windows"))
			Selection => visible_windows_under(forest, selection(forest, layout))
			Stack(window_word) => stack_members(forest, layout, window_word)
		}

	## Where a window sits: the child positions from the root down.
	window_slot : Forest, U64, U64 -> BspSlot
	window_slot = |forest, layout, window_word| {
		top = root(forest, layout, "window_slot")
		match window_node(forest, layout, window_word) {
			Some(key) => slot_path(forest, key, top, [])
			Nothing => NoSlot
		}
	}

	slot_path : Forest, U64, U64, List(U64) -> BspSlot
	slot_path = |forest, key, top, path|
		if key == top {
			Slot(path)
		} else {
			match position(forest, key) {
				At({ above, index: at }) => slot_path(forest, above, top, [at].concat(path))
				Unplaced => NoSlot
			}
		}

	move_over : Forest, U64, BspDirection -> MaybeWord
	move_over = |forest, from, direction|
		match parent(forest, from) {
			Some(above) =>
				if !same_orientation(kind_orientation(container_kind(forest, above, "move_over")), orientation_of(direction)) {
					Nothing
				} else {
					step_over(forest, from, direction)
				}
			Nothing => Nothing
		}

	## The first sibling past `from` along `direction` that takes part.
	step_over : Forest, U64, BspDirection -> MaybeWord
	step_over = |forest, from, direction| {
		next = if backwards(direction) prev_sibling(forest, from) else next_sibling(forest, from)
		match next {
			Some(sibling) =>
				if forest.left_out.is_empty() or has_focus_target(forest, sibling) {
					Some(sibling)
				} else {
					step_over(forest, sibling, direction)
				}
			Nothing => Nothing
		}
	}

	traverse : Forest, U64, BspDirection -> MaybeWord
	traverse = |forest, from, direction|
		match move_over(forest, from, direction) {
			Some(sibling) => Some(sibling)
			Nothing => {
				target = ancestors(forest, from).drop_first(1).fold_until(
					Nothing,
					|_, ancestor|
						match move_over(forest, ancestor, direction) {
							Some(over) => Break(Some(over))
							Nothing => Continue(Nothing)
						},
				)
				match target {
					Some(over) => Some(descend_into_target(forest, over, direction))
					Nothing => Nothing
				}
			}
		}

	descend_into_target : Forest, U64, BspDirection -> U64
	descend_into_target = |forest, target, direction| {
		active = participating_nodes(forest, target)
		descend_from(forest, target, direction, active)
	}

	descend_from : Forest, U64, BspDirection, [Everyone, Only(TakingPart)] -> U64
	descend_from = |forest, current, direction, active| {
		reverse = !is_stacked(forest, current) and !backwards(direction)
		takes = |child|
			match active {
				Everyone => True
				Only(marks) => Slots.marked(marks, child)
			}
		match participating_child(forest, current, reverse, takes) {
			Some(child) => descend_from(forest, child, direction, active)
			Nothing => current
		}
	}

	descend_selection : Forest, U64 -> (Forest, Bool)
	descend_selection = |forest, layout|
		if nothing_takes_part(forest, layout) {
			(forest, False)
		} else {
			match last_selection(forest, selection(forest, layout)) {
				Some(child) => (select_node(forest, child), True)
				Nothing => (forest, False)
			}
		}

	move_focus : Forest, U64, BspDirection -> (Forest, MaybeWord, List(U64))
	move_focus = |forest, layout, direction| {
		chosen = selection(forest, layout)
		match traverse(forest, chosen, direction) {
			Nothing => (forest, Nothing, [])
			Some(new_node) =>
				match find_best_focus_target(forest, new_node) {
					NotFound => (forest, Nothing, [])
					Found(focus_node, focus_window) => {
						(after, highest) = reveal(forest, focus_node, focus_node)
						(after, Some(focus_window), visible_windows_under(after, highest))
					}
				}
		}
	}

	## The selection led up from `key`; the highest node a stack newly shows.
	reveal : Forest, U64, U64 -> (Forest, U64)
	reveal = |forest, key, highest|
		match parent(forest, key) {
			Some(above) => {
				is_group = is_stacked(forest, above)
				(after, changed) = select_locally(forest, key)
				reveal(after, above, if changed and is_group key else highest)
			}
			Nothing => (forest, highest)
		}

	focus : Forest, BspFocusIn -> (Forest, BspFocused)
	focus = |forest, { layout, focus: asked }|
		match asked {
			Select(window_word) =>
				if forest.left_out.contains(window_word) {
					(forest, Selection(False))
				} else {
					match window_node(forest, layout, window_word) {
						Some(key) => (select_node(forest, key), Selection(True))
						Nothing => (forest, Selection(False))
					}
				}
			Ascend =>
				if nothing_takes_part(forest, layout) {
					(forest, Selection(False))
				} else {
					match parent(forest, selection(forest, layout)) {
						Some(above) => (select_node(forest, above), Selection(True))
						Nothing => (forest, Selection(False))
					}
				}
			Descend => {
				(after, handled) = descend_selection(forest, layout)
				(after, Selection(handled))
			}
			Direction(direction) => {
				(after, moved_to, raise) = move_focus(forest, layout, direction)
				(after, Movement({ window: moved_to, raise }))
			}
		}

	## `key` put into a container of `made` (`nest_in_container_internal`): its parent when it is
	## an only child, else a new container in its place (a new root at the root).
	nest_in_container : Forest, U64, U64, GroupKind -> (Forest, U64)
	nest_in_container = |forest, layout, key, made| {
		(nested, container_key) =
			match position(forest, key) {
				At({ above, .. }) if children(forest, above).len() == 1 => (forest, above)
				At(at) => {
					is_chosen = local_selection_of(forest, at.above) == Some(key)
					(inserted, new_parent) = insert(forest, at)
					sized = assume_size_of(inserted, new_parent, key)
					shifted = moved(sized, key, Back(new_parent))
					reselected = if is_chosen selected_locally(shifted, new_parent) else shifted
					(selected_locally(reselected, key), new_parent)
				}
				Unplaced => {
					(created, new_root) = create_root(forest)
					rooted = set_root(created, layout, new_root, "nest_in_container")
					shifted = moved(rooted, key, Back(new_root))
					(selected_locally(shifted, key), new_root)
				}
			}
		(set_kind(nested, container_key, made), container_key)
	}

	toggle_orientation : Forest, U64 -> (Forest, BspEdited)
	toggle_orientation = |forest, layout|
		if nothing_takes_part(forest, layout) {
			(forest, Unchanged)
		} else {
			chosen = selection(forest, layout)
			target =
				match parent(forest, chosen) {
					Some(above) => above
					Nothing => root(forest, layout, "toggle_orientation")
				}
			match container_kind(forest, target, "toggle_orientation") {
				Stack(_) => (forest, Unchanged)
				Split(orientation) => (set_kind(forest, target, Split(perpendicular(orientation))), Changed)
			}
		}

	## A fill toggled (`toggle_fullscreen_mode`).
	toggled : Fill, BspFullscreen -> Fill
	toggled = |fill, mode|
		match (fill, mode) {
			(WithinGaps, WithinGaps) | (Screen, Screen) => Tiled
			(_, WithinGaps) => WithinGaps
			(_, Screen) => Screen
		}

	toggle_fullscreen : Forest, U64, BspFullscreen -> (Forest, List(U64))
	toggle_fullscreen = |forest, layout, mode|
		if nothing_takes_part(forest, layout) {
			(forest, [])
		} else {
			key = selection(forest, layout)
			fill = toggled(node(forest, key, "toggle_fullscreen").fill, mode)
			after = set_fill(forest, key, fill)
			if is_tiled(fill) (after, []) else (after, visible_windows_under(after, key))
		}

	split_selection : Forest, U64, GroupKind -> (Forest, BspEdited)
	split_selection = |forest, layout, made|
		if nothing_takes_part(forest, layout) {
			(forest, Unchanged)
		} else {
			(after, _) = nest_in_container(forest, layout, selection(forest, layout), made)
			(after, Changed)
		}

	apply_stacking : Forest, U64, StackDefault -> (Forest, List(U64))
	apply_stacking = |forest, layout, default|
		if nothing_takes_part(forest, layout) {
			(forest, [])
		} else {
			chosen = selection(forest, layout)
			stacking = if is_window(forest, chosen) parent(forest, chosen) else Some(chosen)
			match stacking {
				Nothing => (forest, [])
				Some(key) => {
					orientation =
						match container_kind(forest, key, "apply_stacking") {
							Stack(held) => perpendicular(held)
							Split(held) => group_orientation(held, default)
						}
					stacked = set_kind(forest, key, Stack(orientation))
					after =
						match first_child(stacked, key) {
							Some(first) => select_node(stacked, first)
							Nothing => stacked
						}
					(after, visible_windows_under(after, key))
				}
			}
		}

	unstack : Forest, U64, StackDefault -> (Forest, List(U64))
	unstack = |forest, layout, default|
		if nothing_takes_part(forest, layout) {
			(forest, [])
		} else {
			match stacked_selection_container(forest, layout) {
				Nothing => (forest, [])
				Some(key) => {
					orientation = group_orientation(kind_orientation(container_kind(forest, key, "unstack")), default)
					after = set_kind(forest, key, Split(orientation))
					(after, visible_windows_under(after, key))
				}
			}
		}

	move_stack_tab : Forest, U64, U64, U64 -> (Forest, Bool)
	move_stack_tab = |forest, layout, window_word, target_word|
		if !takes_part(forest, window_word) or !takes_part(forest, target_word) {
			(forest, False)
		} else {
			match (window_node(forest, layout, window_word), window_node(forest, layout, target_word)) {
				(Some(source), Some(target)) =>
					match common_ancestor(forest, source, target) {
						Some(stack) if is_stacked(forest, stack) => {
							tab = |key| found(ancestors(forest, key).find_first(|at| parent(forest, at) == Some(stack)))
							match (tab(source), tab(target)) {
								(Some(moving), Some(anchor)) if moving != anchor => {
									listed = children(forest, stack)
									forward =
										match (listed.find_first_index(|c| c == moving), listed.find_first_index(|c| c == anchor)) {
											(Ok(m), Ok(a)) => m < a
											_ => False
										}
									move_node(forest, moving, if forward After(anchor) else Before(anchor))
								}
								_ => (forest, False)
							}
						}
						_ => (forest, False)
					}
				_ => (forest, False)
			}
		}

	swap_windows : Forest, U64, U64, U64 -> (Forest, Bool)
	swap_windows = |forest, layout, a, b|
		if !takes_part(forest, a) or !takes_part(forest, b) {
			(forest, False)
		} else {
			match (window_node(forest, layout, a), window_node(forest, layout, b)) {
				(Some(node_a), Some(node_b)) if node_a != node_b => {
					chosen = selection(forest, layout)
					held_a = node(forest, node_a, "swap_windows")
					held_b = node(forest, node_b, "swap_windows")
					# Each window keeps when it entered the layout.
					(stamp_a, stamp_b) =
						match (held_a.body, held_b.body) {
							(Window({ stamp: sa, .. }), Window({ stamp: sb, .. })) => (sa, sb)
							_ => {
								crash "GroupForest.swap_windows: a window's leaf holds no window"
							}
						}
					swapped = set_node(
						set_node(forest, node_a, { ..held_a, body: Window({ window: b, stamp: stamp_b }) }, "swap_windows"),
						node_b,
						{ ..held_b, body: Window({ window: a, stamp: stamp_a }) },
						"swap_windows",
					)
					reselected = if chosen == node_a node_b else if chosen == node_b node_a else chosen
					(select_node(swapped, reselected), True)
				}
				_ => (forest, False)
			}
		}

	edited_by : (Forest, Bool) -> (Forest, BspEdited)
	edited_by = |(after, did)| (after, if did Changed else Unchanged)

	# Sizes, resizes and drops.

	proportion : Forest, U64 -> [Share(F64), NoShare]
	proportion = |forest, key|
		match parent(forest, key) {
			Some(above) => {
				total = children_total(forest, above)
				if F32.is_finite(total) and total > epsilon32 {
					Share(max32(size(forest, key), 0.0).to_f64() / total.to_f64())
				} else {
					NoShare
				}
			}
			Nothing => NoShare
		}

	take_share : Forest, U64, U64, F32 -> Forest
	take_share = |forest, key, from, asked|
		if parent(forest, key) != parent(forest, from) {
			crash "GroupForest.take_share: ${key.to_str()} and ${from.to_str()} are not siblings"
		} else {
			capped = min32(asked, size(forest, from) - twentieth32)
			share = max32(capped, twentieth32 - size(forest, key))
			given = set_size(forest, from, size(forest, from) - share)
			taken = set_size(given, key, size(given, key) + share)
			match parent(taken, key) {
				Some(above) => {
					(normalized, _) = normalize_weights(taken, above)
					normalized
				}
				Nothing => {
					crash "GroupForest.take_share: node ${key.to_str()} has no sibling"
				}
			}
		}

	## Every node under `top` sized 1, and whether any was not (`Rebalance`'s answer): the
	## question read first, then the sizes written.
	rebalance : Forest, U64 -> (Forest, Bool)
	rebalance = |forest, top| {
		changed = walk(forest, top, Preorder, False, |_, key, at| if key != top and size_of(at) != 1.0 Break(True) else Continue(False))
		(balance_sizes(forest, top), changed)
	}

	## Every node under `top` sized 1.
	balance_sizes : Forest, U64 -> Forest
	balance_sizes = |forest, top| children(forest, top).fold(forest, |at, child| balance_sizes(set_size(at, child, 1.0), child))

	remove_and_rebalance : Forest, U64 -> Forest
	remove_and_rebalance = |forest, window_word| {
		cleared = { ..forest, left_out: forest.left_out.remove(window_word) }
		occurrences(cleared, window_word).fold(
			cleared,
			|at, (_, key)| {
				above = parent(at, key)
				removed = remove_node(at, key)
				match above {
					Some(p) if holds_node(removed, p) => {
						(balanced, _) = rebalance(removed, p)
						balanced
					}
					_ => removed
				}
			},
		)
	}

	## A fill after a native resize, if it entered or left a fullscreen frame
	## (`track_fullscreen_resize`).
	resized : Fill, BspNativeResize -> [Refilled(Fill), Unmoved]
	resized = |fill, { old, new, screen, tiling, .. }|
		if same_rect(new, screen) {
			Refilled(Screen)
		} else if same_rect(old, screen) {
			Refilled(
				match fill {
					Screen => Tiled
					_ => fill
				},
			)
		} else if same_rect(new, tiling) {
			Refilled(WithinGaps)
		} else if same_rect(old, tiling) {
			Refilled(
				match fill {
					WithinGaps => Tiled
					_ => fill
				},
			)
		} else {
			Unmoved
		}

	resize_window : Forest, BspNativeResize -> Forest
	resize_window = |forest, resize|
		if !takes_part(forest, resize.window) {
			forest
		} else {
			match window_node(forest, resize.layout, resize.window) {
				Some(key) =>
					match resized(node(forest, key, "resize_window").fill, resize) {
						Refilled(fill) => set_fill(forest, key, fill)
						Unmoved => set_frame_from_resize(forest, key, resize)
					}
				Nothing => forest
			}
		}

	resize_pair : Forest, U64, BspDirection -> [Pair(U64, U64), NoPair]
	resize_pair = |forest, key, direction|
		ancestors(forest, key).fold_until(
			NoPair,
			|_, candidate|
				match parent(forest, candidate) {
					Some(above) if !is_stacked(forest, above) =>
						match move_over(forest, candidate, direction) {
							Some(sibling) => Break(Pair(candidate, sibling))
							Nothing => Continue(NoPair)
						}
					_ => Continue(NoPair)
				},
		)

	has_resize_pair : Forest, U64, BspDirection -> Bool
	has_resize_pair = |forest, key, direction|
		match resize_pair(forest, key, direction) {
			Pair(_, _) => True
			NoPair => False
		}

	resize_internal : Forest, U64, F64, BspDirection -> (Forest, Bool)
	resize_internal = |forest, key, screen_ratio, direction|
		match resize_pair(forest, key, direction) {
			NoPair => (forest, False)
			Pair(resizing, sibling) => {
				rate = ancestors(forest, resizing).drop_first(1).fold_until(
					Rate(1.0),
					|held, at|
						match held {
							Rate(r) =>
								match parent(forest, at) {
									Some(above) if same_orientation(kind_orientation(container_kind(forest, above, "resize_internal")), orientation_of(direction)) and !is_stacked(forest, above) =>
										match proportion(forest, at) {
											Share(p) => Continue(Rate(r * p))
											NoShare => Break(NoRate)
										}
									_ => Continue(Rate(r))
								}
							NoRate => Break(NoRate)
						},
				)
				exchange_rate =
					match rate {
						Rate(r) => r
						NoRate => 1.0
					}
				above =
					match parent(forest, resizing) {
						Some(p) => p
						Nothing => {
							crash "GroupForest.resize_internal: node ${resizing.to_str()} has no sibling"
						}
					}
				local_ratio = screen_ratio * children_total(forest, above).to_f64() / exchange_rate
				(take_share(forest, resizing, sibling, Geometry.narrow(local_ratio)), True)
			}
		}

	## An edge whose delta moved at least a point and has a pair to resize, the larger first.
	choose_edge : Forest, U64, (BspDirection, F64), (BspDirection, F64) -> [Edge(BspDirection, F64), NoEdge]
	choose_edge = |forest, key, (first_direction, first_delta), (second_direction, second_delta)| {
		cleaned = |delta| if delta.abs() < 1.0 0.0 else delta
		a_delta = cleaned(first_delta)
		b_delta = cleaned(second_delta)
		a = (first_direction, a_delta)
		b = (second_direction, b_delta)
		# Upstream's comparison, kept: a NaN delta swaps the edges.
		ordered = if !(a_delta.abs() >= b_delta.abs()) [b, a] else [a, b]
		match ordered.find_first(|(direction, delta)| delta != 0.0 and has_resize_pair(forest, key, direction)) {
			Ok((direction, delta)) => Edge(direction, delta)
			Err(_) => NoEdge
		}
	}

	set_frame_from_resize : Forest, U64, BspNativeResize -> Forest
	set_frame_from_resize = |forest, key, { old, new, screen, .. }| {
		horizontal = choose_edge(forest, key, (Left, old.x - new.x), (Right, (new.x + new.w) - (old.x + old.w)))
		vertical = choose_edge(forest, key, (Up, old.y - new.y), (Down, (new.y + new.h) - (old.y + old.h)))
		[(horizontal, screen.w), (vertical, screen.h)].fold(
			forest,
			|at, (edge, extent)|
				match edge {
					Edge(direction, delta) => {
						(after, _) = resize_internal(at, key, delta / extent, direction)
						after
					}
					NoEdge => at
				},
		)
	}

	## The two directions along an orientation, forward first.
	resize_directions : BspOrientation -> List(BspDirection)
	resize_directions = |orientation|
		match orientation {
			Horizontal => [Right, Left]
			Vertical => [Down, Up]
		}

	## The first direction that resizes (`[..].any(..)`).
	resize_any : Forest, U64, F64, List(BspDirection) -> (Forest, Bool)
	resize_any = |forest, key, amount, directions|
		directions.fold_until(
			(forest, False),
			|(at, _), direction| {
				(after, did) = resize_internal(at, key, amount, direction)
				if did Break((after, True)) else Continue((after, False))
			},
		)

	resize_selection : Forest, U64, BspResize -> (Forest, BspEdited)
	resize_selection = |forest, layout, { amount, along }|
		if nothing_takes_part(forest, layout) or amount == 0.0 {
			(forest, Unchanged)
		} else {
			chosen = selection(forest, layout)
			if !is_window(forest, chosen) {
				(forest, Unchanged)
			} else {
				match along {
					Horizontal => edited_by(resize_any(forest, chosen, amount, resize_directions(Horizontal)))
					Vertical => edited_by(resize_any(forest, chosen, amount, resize_directions(Vertical)))
					Smart => edited_by(resize_smart(forest, chosen, amount))
				}
			}
		}

	## Up from `key`, the nearest split parent that resizes.
	resize_smart : Forest, U64, F64 -> (Forest, Bool)
	resize_smart = |forest, key, amount|
		match parent(forest, key) {
			Some(above) =>
				match container_kind(forest, above, "resize_selection") {
					Split(orientation) => {
						(after, did) = resize_any(forest, key, amount, resize_directions(orientation))
						if did (after, True) else resize_smart(after, above, amount)
					}
					Stack(_) => resize_smart(forest, above, amount)
				}
			Nothing => (forest, False)
		}

	drop_window : Forest, U64, U64, U64, GroupDropAction -> (Forest, Bool)
	drop_window = |forest, layout, source, target, action|
		if source == target or !takes_part(forest, source) or !takes_part(forest, target) {
			(forest, False)
		} else {
			match action {
				Swap => swap_windows(forest, layout, source, target)
				Stack =>
					match (window_node(forest, layout, source), window_node(forest, layout, target)) {
						(Some(source_node), Some(target_node)) => (drop_stacking(forest, source_node, target_node), True)
						_ => (forest, False)
					}
				Insert(direction) =>
					match (window_node(forest, layout, source), window_node(forest, layout, target)) {
						(Some(source_node), Some(target_node)) => drop_inserting(forest, layout, source_node, target_node, direction)
						_ => (forest, False)
					}
			}
		}

	drop_stacking : Forest, U64, U64 -> Forest
	drop_stacking = |forest, source_node, target_node|
		match position(forest, target_node) {
			At(at) =>
				if is_stacked(forest, at.above) {
					select_node(moved(forest, source_node, After(target_node)), source_node)
				} else {
					orientation = kind_orientation(container_kind(forest, at.above, "drop_window"))
					(inserted, made) = insert(forest, at)
					sized = assume_size_of(inserted, made, target_node)
					gathered = moved(moved(sized, target_node, Back(made)), source_node, Back(made))
					select_node(set_kind(gathered, made, Stack(orientation)), source_node)
				}

			# Rust panics here too (`insert`'s "invalid tile insertion"): the target is a window's
			# leaf, and a layout's root is always a container.
			Unplaced => {
				crash "GroupForest.drop_stacking: window leaf ${target_node.to_str()} is a root"
			}
		}

	drop_inserting : Forest, U64, U64, U64, BspDirection -> (Forest, Bool)
	drop_inserting = |forest, layout, source_node, target_node, direction| {
		target_anchor =
			match parent(forest, target_node) {
				Some(above) if is_stacked(forest, above) => above
				_ => target_node
			}
		if is_within(forest, target_anchor, source_node) {
			(forest, False)
		} else {
			before = backwards(direction)
			anchor_parent = parent(forest, target_anchor)
			along =
				match anchor_parent {
					Some(above) => !is_stacked(forest, above) and same_orientation(kind_orientation(container_kind(forest, above, "drop_window")), orientation_of(direction))
					Nothing => False
				}
			placed =
				if along {
					moved(forest, source_node, if before Before(target_anchor) else After(target_anchor))
				} else {
					made = Split(orientation_of(direction))
					(nested, made_key) =
						match position(forest, target_anchor) {
							Unplaced => nest_in_container(forest, layout, target_anchor, made)
							At(at) => {
								(inserted, key) = insert(forest, at)
								sized = assume_size_of(inserted, key, target_anchor)
								(set_kind(sized, key, made), key)
							}
						}
					gathered = moved(nested, target_anchor, Back(made_key))
					if before moved(gathered, source_node, Before(target_anchor)) else moved(gathered, source_node, Back(made_key))
				}
			(select_node(placed, source_node), True)
		}
	}

	# The tree helpers both systems' own operations use.

	## A new leaf holding the window at the back of `above` (`add_window_under`).
	add_window_under : Forest, U64, U64 -> (Forest, U64)
	add_window_under = |forest, above, window_word| {
		(inserted, key) = insert(forest, back_of(forest, above))
		(set_window(inserted, key, window_word), key)
	}

	## How many windows a subtree holds, taking part or not.
	count_windows : Forest, U64 -> U64
	count_windows = |forest, key|
		if is_window(forest, key) 1 else children(forest, key).fold(0, |sum, child| sum + count_windows(forest, child))

	## The windows from `start` on, stepping through `region` in preorder (or back).
	windows_from : Forest, U64, MaybeWord, Bool -> List(U64)
	windows_from = |forest, region, start, forward|
		match start {
			Some(key) => {
				rest = windows_from(forest, region, step_in(forest, key, region, forward), forward)
				match window_at(forest, key) {
					Some(leaf) => [leaf].concat(rest)
					Nothing => rest
				}
			}
			Nothing => []
		}

	edit_fullscreen : Forest, U64, BspFullscreen -> (Forest, BspEdited)
	edit_fullscreen = |forest, layout, mode| {
		(after, raised) = toggle_fullscreen(forest, layout, mode)
		(after, Raise(raised))
	}

	edit_drop : Forest, U64, GroupDrop -> (Forest, BspEdited)
	edit_drop = |forest, layout, { source, target, action }| edited_by(drop_window(forest, layout, source, target, action))

	# The traditional system.

	traditional_add_window : Traditionals, TraditionalAdd -> Traditionals
	traditional_add_window = |system, { layout, window: window_word, toward }| {
		forest = system.forest
		if contains_window(forest, layout, window_word) {
			system
		} else {
			match toward {
				AfterSelection => { ..system, forest: add_window_after_selection(system, layout, window_word) }
				Toward(direction) => {
					made = Split(orientation_of(direction))
					top = root(forest, layout, "add_window")
					current = container_kind(forest, top, "add_window")
					(nested, at) =
						if !children(forest, top).is_empty() and (kind_is_stacked(current) or !same_orientation(kind_orientation(current), orientation_of(direction))) {
							nest_in_container(forest, layout, top, made)
						} else {
							(forest, top)
						}
					(added, key) = add_window_under(set_kind(nested, at, made), at, window_word)
					placed =
						if !backwards(direction) {
							match first_child(added, at) {
								Some(first) => moved(added, key, Before(first))
								Nothing => added
							}
						} else {
							added
						}
					{ ..system, forest: select_node(placed, key) }
				}
			}
		}
	}

	add_window_after_selection : Traditionals, U64, U64 -> Forest
	add_window_after_selection = |system, layout, window_word| {
		forest = system.forest
		match system.settings.insertion {
			EndOfTree => {
				(added, key) = add_window_under(forest, root(forest, layout, "add_window"), window_word)
				select_node(added, key)
			}
			NextToSelection => {
				chosen = selection(forest, layout)
				(added, key) =
					match parent(forest, chosen) {
						Nothing =>
							# The root selected with children: split from its chosen child rather
							# than append a fresh full-weight sibling.
							match or_else(local_selection(forest, chosen), |{}| last_child(forest, chosen)) {
								Some(anchor) => smart_window_insertion(system, layout, anchor, window_word)
								Nothing => add_window_under(forest, chosen, window_word)
							}
						Some(_) => smart_window_insertion(system, layout, chosen, window_word)
					}
				select_node(added, key)
			}
		}
	}

	smart_window_insertion : Traditionals, U64, U64, U64 -> (Forest, U64)
	smart_window_insertion = |system, layout, chosen, window_word| {
		forest = system.forest
		(nested, place) =
			match position(forest, chosen) {
				At(at) => {
					made = container_kind(forest, at.above, "smart_window_insertion")
					if !kind_is_stacked(made) and children(forest, at.above).len() > 3 {
						(after, container_key) = nest_in_container(forest, layout, chosen, made)
						(after, back_of(after, container_key))
					} else {
						(forest, after_place(at))
					}
				}

				# Rust panics here too (`insert(After(selection))` of a root): both callers pass a
				# node with a parent (the selection when it has one, else the root's chosen child).
				Unplaced => {
					crash "GroupForest.smart_window_insertion: ${chosen.to_str()} is a root"
				}
			}
		(inserted, key) = insert(nested, place)
		split = split_new_sibling_from_selection(inserted, system.settings.new_sibling, chosen, key)
		(set_window(split, key, window_word), key)
	}

	split_new_sibling_from_selection : Forest, GroupNewSibling, U64, U64 -> Forest
	split_new_sibling_from_selection = |forest, new_sibling, chosen, sibling|
		match parent(forest, chosen) {
			Some(above) if parent(forest, sibling) == Some(above) =>
				match new_sibling {
					# Siblings stay normalized to their count, so the average is 1, the new
					# node's weight already.
					AverageWeight => forest
					HalvesSelection => {
						selected = max32(size(forest, chosen), 0.0)
						(kept, given) = if selected <= epsilon32 (1.0, 1.0) else (selected * 0.5, selected * 0.5)
						set_size(set_size(forest, chosen, kept), sibling, given)
					}
				}
			_ => forest
		}

	traditional_can_move_window : Forest, BspMoveQuery -> Bool
	traditional_can_move_window = |forest, { layout, window: window_word, direction }|
		takes_part(forest, window_word)
		and (
			match window_node(forest, layout, window_word) {
				Some(key) =>
					match move_destination(forest, key, direction) {
						Destination(_, _) => True
						NoDestination => False
					}
				Nothing => False
			}
		)

	traditional_edit : Traditionals, GroupEditIn -> (Forest, BspEdited)
	traditional_edit = |system, { layout, edit }| {
		forest = system.forest
		match edit {
					Fullscreen(mode) => edit_fullscreen(forest, layout, mode)
					Swap({ a, b }) => edited_by(swap_windows(forest, layout, a, b))
					ReorderTab({ window: window_word, target }) => edited_by(move_stack_tab(forest, layout, window_word, target))
					Resize(asked) => resize_selection(forest, layout, asked)
					Drop(dropped) => edit_drop(forest, layout, dropped)
					Moved({ direction, beyond }) => edited_by(move_selection_with_display_neighbor(forest, layout, direction, beyond))
					Split(made) => split_selection(forest, layout, made)
					Join(direction) => edited_by(join_selection_with_direction(forest, layout, direction))
					ConsumeOrExpel(direction) => {
						chosen = selection(forest, layout)
						joined =
							match parent(forest, chosen) {
								Some(above) => parent(forest, above) != Nothing
								Nothing => False
							}
						edited_by(if joined unjoin_selection(forest, layout) else join_selection_with_direction(forest, layout, direction))
					}
					Stack(default) => {
						(after, raised) = apply_stacking(forest, layout, default)
						(after, Raise(raised))
					}
					Unstack(default) => {
						(after, raised) = unstack(forest, layout, default)
						(after, Raise(raised))
					}
					Unjoin => edited_by(unjoin_selection(forest, layout))
					Rebalance => edited_by(rebalance(forest, root(forest, layout, "rebalance")))
					ToggleOrientation => toggle_orientation(forest, layout)
				}
	}

	move_selection_with_display_neighbor : Forest, U64, BspDirection, StackBeyond -> (Forest, Bool)
	move_selection_with_display_neighbor = |forest, layout, direction, beyond| {
		chosen = selection(forest, layout)
		no_neighbor =
			match beyond {
				AnotherDisplay => !ancestors(forest, chosen).any(|key| move_over(forest, key, direction) != Nothing)
				Nothing => False
			}
		unfocused =
			!forest.left_out.is_empty()
			and (
				match focused_leaf(forest, layout) {
					Found(_, _) => False
					NotFound => True
				}
			)
		if no_neighbor or unfocused {
			(forest, False)
		} else {
			move_tree_node(forest, layout, chosen, direction)
		}
	}

	move_tree_node : Forest, U64, U64, BspDirection -> (Forest, Bool)
	move_tree_node = |forest, layout, moving, direction|
		match parent(forest, moving) {
			Nothing => (forest, False)
			Some(old_parent) => {
				is_chosen = local_selection_of(forest, old_parent) == Some(moving)
				match move_destination(forest, moving, direction) {
					NoDestination => (forest, False)
					Destination(placement, wrap) => {
						wrapped =
							match wrap {
								Wrap(old_root) => {
									(after, _) = nest_in_container(forest, layout, old_root, Split(orientation_of(direction)))
									after
								}
								NoWrap => forest
							}
						(after, did) = move_node(wrapped, moving, placement)
						if did and is_chosen (reselect_up_to(after, moving, old_parent), True) else (after, did)
					}
				}
			}
		}

	## Each node from `key` up to (not including) `stop` selected locally.
	reselect_up_to : Forest, U64, U64 -> Forest
	reselect_up_to = |forest, key, stop|
		if key == stop {
			forest
		} else {
			above = parent(forest, key)
			selected = selected_locally(forest, key)
			match above {
				Some(next) => reselect_up_to(selected, next, stop)
				Nothing => selected
			}
		}

	## Where a move puts `moving`, and the root to wrap first when it leaves the root's edge.
	move_destination : Forest, U64, BspDirection -> [Destination(Placement, [Wrap(U64), NoWrap]), NoDestination]
	move_destination = |forest, moving, direction|
		if parent(forest, moving) == Nothing {
			NoDestination
		} else {
			along = orientation_of(direction)
			chosen =
				match move_over(forest, moving, direction) {
					Some(sibling) => {
						target = destination_within(forest, sibling, along)
						Chosen(target, target != sibling, NoWrap)
					}
					Nothing => {
						reached = ancestors(forest, moving).drop_first(1).find_first(
							|key|
								match parent(forest, key) {
									Some(above) => same_orientation(kind_orientation(container_kind(forest, above, "move_destination")), along)
									Nothing => False
								},
						)
						match reached {
							Ok(target) => Chosen(target, False, NoWrap)
							Err(_) => {
								old_root =
									match ancestors(forest, moving).last() {
										Ok(top) => top
										Err(_) => moving
									}
								# Leaving the edge of a root group splits the window out of it; at
								# the edge of a same-oriented split there is nowhere further to go.
								if splits_along(container_kind(forest, old_root, "move_destination"), along) {
									Stuck
								} else {
									Chosen(old_root, False, Wrap(old_root))
								}
							}
						}
					}
				}
			match chosen {
				Stuck => NoDestination
				Chosen(target, behind, wrap) =>
					Destination(if backwards(direction) != behind Before(target) else After(target), wrap)
			}
		}

	## The node a move into `key` lands next to: down through chosen (or first) children until
	## a container along the move.
	destination_within : Forest, U64, BspOrientation -> U64
	destination_within = |forest, key, along|
		match or_else(local_selection_of(forest, key), |{}| first_child(forest, key)) {
			Nothing => key
			Some(next) =>
				if same_orientation(kind_orientation(container_kind(forest, key, "move_destination")), along) {
					next
				} else {
					destination_within(forest, next, along)
				}
		}

	join_selection_with_direction : Forest, U64, BspDirection -> (Forest, Bool)
	join_selection_with_direction = |forest, layout, direction|
		if nothing_takes_part(forest, layout) {
			(forest, False)
		} else {
			initial = selection(forest, layout)
			match find_natural_join_target(forest, initial, direction) {
				Nothing => (forest, False)
				Some(target) => {
					along = orientation_of(direction)
					# A leaf at the edge of a container along the direction lifts the selection
					# to that container, so whole groups merge.
					chosen =
						match parent(forest, initial) {
							Some(above) => {
								is_edge = if backwards(direction) prev_sibling(forest, initial) == Nothing else next_sibling(forest, initial) == Nothing
								if splits_along(container_kind(forest, above, "join"), along) and (is_edge or children(forest, above).len() == 2) and parent(forest, target) != Some(above) and !is_within(forest, target, above) {
									above
								} else {
									initial
								}
							}
							Nothing => initial
						}
					# A selection that is now a container along the direction absorbs the
					# target, with no extra nesting.
					target_is_within = is_within(forest, target, chosen)
					chosen_is_within = is_within(forest, chosen, target)
					if is_split_along(forest, chosen, along) and !target_is_within and !chosen_is_within and parent(forest, target) != Some(chosen) {
						placement =
							match first_child(forest, chosen) {
								Some(first) if backwards(direction) => Before(first)
								_ => Back(chosen)
							}
						(select_node(moved(forest, target, placement), chosen), True)
					} else {
						joined = perform_natural_join(forest, chosen, target, direction)
						if is_window(joined, chosen) {
							(select_node(joined, chosen), True)
						} else {
							(descended, _) = descend_selection(joined, layout)
							(descended, True)
						}
					}
				}
			}
		}

	find_natural_join_target : Forest, U64, BspDirection -> MaybeWord
	find_natural_join_target = |forest, from, direction| {
		sibling = find_direct_sibling_target(forest, from, direction)
		lifted =
			match parent(forest, from) {
				Some(above) if splits_along(container_kind(forest, above, "join"), orientation_of(direction)) and (sibling == Nothing or children(forest, above).len() == 2) =>
					find_direct_sibling_target(forest, above, direction)
				_ => Nothing
			}
		or_else(
			lifted,
			|{}|
				or_else(
					sibling,
					|{}|
						or_else(
							match traverse(forest, from, direction) {
								Some(target) =>
									if is_window(forest, target) {
										Some(target)
									} else {
										match find_best_container_child_for_joining(forest, target, direction) {
											Some(child) => Some(child)
											Nothing => Some(target)
										}
									}
								Nothing => Nothing
							},
							|{}| find_hierarchical_join_target(forest, from, direction),
						),
				),
		)
	}

	find_direct_sibling_target : Forest, U64, BspDirection -> MaybeWord
	find_direct_sibling_target = |forest, from, direction|
		match parent(forest, from) {
			Some(_) => if backwards(direction) prev_sibling(forest, from) else next_sibling(forest, from)
			Nothing => Nothing
		}

	find_best_container_child_for_joining : Forest, U64, BspDirection -> MaybeWord
	find_best_container_child_for_joining = |forest, key, direction|
		match first_child(forest, key) {
			Nothing => Nothing
			Some(first) =>
				if !same_orientation(kind_orientation(container_kind(forest, key, "join")), orientation_of(direction)) {
					or_else(local_selection(forest, key), |{}| Some(first))
				} else if backwards(direction) {
					Some(first)
				} else {
					last_child(forest, key)
				}
		}

	## The first ancestor with a direct sibling: that sibling's best child for joining (none
	## when it has none; the search stops at that ancestor).
	find_hierarchical_join_target : Forest, U64, BspDirection -> MaybeWord
	find_hierarchical_join_target = |forest, from, direction| {
		reached = ancestors(forest, from).drop_first(1).fold_until(
			NoTarget,
			|_, ancestor|
				match find_direct_sibling_target(forest, ancestor, direction) {
					Some(target) => Break(Target(find_best_container_child_for_joining(forest, target, opposite(direction))))
					Nothing => Continue(NoTarget)
				},
		)
		match reached {
			Target(child) => child
			NoTarget => Nothing
		}
	}

	perform_natural_join : Forest, U64, U64, BspDirection -> Forest
	perform_natural_join = |forest, chosen, target, direction| {
		chosen_parent = parent(forest, chosen)
		target_parent = parent(forest, target)
		stacked = |above|
			match above {
				Some(key) if is_stacked(forest, key) => Some(key)
				_ => Nothing
			}
		back = backwards(direction)
		join =
			match (stacked(chosen_parent), stacked(target_parent)) {
				(Some(stack), Nothing) => Join(stack, target, back)
				(Nothing, Some(stack)) => Join(stack, chosen, !back)
				_ => NoJoin
			}
		match join {
			Join(stack, moving, prepend) => {
				placement =
					match first_child(forest, stack) {
						Some(first) if prepend => Before(first)
						_ => Back(stack)
					}
				select_node(moved(forest, moving, placement), stack)
			}
			NoJoin => {
				made = Split(orientation_of(direction))
				merged =
					match (chosen_parent, target_parent) {
						(Some(sp), Some(tp)) =>
							if sp == tp and is_stacked(forest, sp) {
								Merged(select_node(set_kind(forest, sp, made), sp))
							} else if sp != tp and are_containers_mergeable(forest, sp, tp, direction) {
								(found_forest, common) = find_or_create_smart_common_parent(forest, sp, tp, direction)
								Merged(select_node(set_kind(found_forest, common, made), common))
							} else {
								NotMerged
							}
						_ => NotMerged
					}
				match merged {
					Merged(after) => after
					NotMerged => {
						(found_forest, common) = find_or_create_smart_common_parent(forest, chosen, target, direction)
						kinded = if is_stacked(found_forest, common) found_forest else set_kind(found_forest, common, made)
						select_node(kinded, common)
					}
				}
			}
		}
	}

	are_containers_mergeable : Forest, U64, U64, BspDirection -> Bool
	are_containers_mergeable = |forest, a, b, direction| {
		along = orientation_of(direction)
		is_split_along(forest, a, along) and is_split_along(forest, b, along)
	}

	find_or_create_smart_common_parent : Forest, U64, U64, BspDirection -> (Forest, U64)
	find_or_create_smart_common_parent = |forest, a, b, direction|
		match (parent(forest, a), parent(forest, b)) {
			(Some(pa), Some(pb)) if pa == pb and splits_along(container_kind(forest, pa, "join"), orientation_of(direction)) and children(forest, pa).len() == 2 => (forest, pa)
			_ => find_or_create_common_parent(forest, a, b)
		}

	find_or_create_common_parent : Forest, U64, U64 -> (Forest, U64)
	find_or_create_common_parent = |forest, a, b|
		match common_ancestor(forest, a, b) {
			Nothing => {
				crash "GroupForest.find_or_create_common_parent: ${a.to_str()} and ${b.to_str()} are in different trees"
			}
			Some(ancestor) =>
				if ancestor == a or ancestor == b {
					(forest, ancestor)
				} else {
					match (position(forest, a), parent(forest, b)) {
						(At(at), Some(b_parent)) if at.above == ancestor and b_parent == ancestor => {
							size_a = max32(size(forest, a), 0.0)
							size_b = max32(size(forest, b), 0.0)
							(inserted, made) = insert(forest, at)
					# Both shares go to the replacement before the moves normalize the siblings
					# left, so every surviving weight scales together.
							emptied = set_size(set_size(set_size(inserted, made, size_a + size_b), a, 0.0), b, 0.0)
							gathered = moved(moved(emptied, a, Back(made)), b, Back(made))
							(set_size(set_size(gathered, a, size_a), b, size_b), made)
						}

						_ => {
							(inserted, made) = insert(forest, back_of(forest, ancestor))
							kinded = set_kind(inserted, made, Split(Horizontal))
							(moved(moved(kinded, a, Back(made)), b, Back(made)), made)
						}
					}
				}
		}

	unjoin_selection : Forest, U64 -> (Forest, Bool)
	unjoin_selection = |forest, layout|
		if nothing_takes_part(forest, layout) {
			(forest, False)
		} else {
			match parent(forest, selection(forest, layout)) {
				Nothing => (forest, False)
				Some(above) =>
					match parent(forest, above) {
						Nothing => (forest, False)
						Some(grandparent) => {
							chosen = local_selection_of(forest, above)
							after_above = next_sibling(forest, above)
							above_size = max32(size(forest, above), 0.0)
							# Weights read before the moves normalize the remaining siblings.
							weighted = children(forest, above).map(|child| (child, max32(size(forest, child), 0.0)))
							var $total = F32.from_bits(0x8000_0000)
							for (_, weight) in weighted {
								$total = $total + weight
							}
							total = $total
							count = weighted.len().to_f32()
							placement =
								match after_above {
									Some(next) => Before(next)
									Nothing => Back(grandparent)
								}
							lifted = weighted.fold(
								forest,
								|at, (child, weight)| {
									promoted = if F32.is_finite(total) and total > epsilon32 above_size * weight / total else above_size / count
									set_size(moved(at, child, placement), child, promoted)
								},
							)
							removed = remove_node(lifted, above)
							reselected =
								match or_else(chosen, |{}| first_child(removed, grandparent)) {
									Some(key) => select_node(removed, key)
									Nothing => removed
								}
							(reselected, True)
						}
					}
			}
		}

	# The master-stack system.

	root_orientation : MasterSettings -> BspOrientation
	root_orientation = |settings|
		match settings.side {
			Left | Right => Horizontal
			Top | Bottom => Vertical
		}

	arranged : MasterSettings, MasterArrangement -> BspOrientation
	arranged = |settings, arrangement|
		match arrangement {
			Derived => perpendicular(root_orientation(settings))
			Fixed(orientation) => orientation
		}

	master_orientation : MasterSettings -> BspOrientation
	master_orientation = |settings| arranged(settings, settings.master)

	stack_orientation : MasterSettings -> BspOrientation
	stack_orientation = |settings| arranged(settings, settings.stack)

	master_first : MasterSettings -> Bool
	master_first = |settings|
		match settings.side {
			Left | Top => True
			Right | Bottom => False
		}

	## A layout's root and its (master, stack) regions.
	region_roots : Forest, U64 -> { top : U64, master : U64, stack : U64 }
	region_roots = |forest, layout| {
		top = root(forest, layout, "region_roots")
		match regions(forest, top) {
			Regions(master, stack) => { top, master, stack }
			NoRegions => {
				crash "GroupForest.region_roots: layout ${layout.to_str()} is not a region pair"
			}
		}
	}

	same_arrangement : MasterArrangement, MasterArrangement -> Bool
	same_arrangement = |a, b|
		match (a, b) {
			(Derived, Derived) => True
			(Fixed(x), Fixed(y)) => same_orientation(x, y)
			_ => False
		}

	## Settings equal field by field (`MasterSettings`'s `PartialEq`).
	same_settings : MasterSettings, MasterSettings -> Bool
	same_settings = |a, b| {
		insertion =
			match (a.insertion, b.insertion) {
				(NextToSelection, NextToSelection) | (EndOfTree, EndOfTree) | (Front, Front) => True
				_ => False
			}
		side =
			match (a.side, b.side) {
				(Left, Left) | (Right, Right) | (Top, Top) | (Bottom, Bottom) => True
				_ => False
			}
		placement =
			match (a.placement, b.placement) {
				(Master, Master) | (Stack, Stack) | (Focused, Focused) => True
				_ => False
			}
		insertion and a.ratio == b.ratio and a.count == b.count and side and placement and same_arrangement(a.master, b.master) and same_arrangement(a.stack, b.stack)
	}

	## Settings changed: each layout's regions rearranged, the ratio applied when it changed,
	## the master count enforced (`update_settings`).
	master_update_settings : Master, MasterSettings -> Master
	master_update_settings = |system, settings|
		if same_settings(system.settings, settings) {
			system
		} else if same_settings({ ..system.settings, insertion: settings.insertion }, settings) {
			{ ..system, settings }
		} else {
			old_ratio = system.settings.ratio
			updated = { ..system, settings }
			layout_keys(updated.forest).fold(
				updated,
				|at, layout| {
					{ top, master, stack } = region_roots(at.forest, layout)
					arranged_forest = set_kind(
						set_kind(set_kind(set_primary_region_first(at.forest, top, master_first(settings)), top, Split(root_orientation(settings))), master, Split(master_orientation(settings))),
						stack,
						Split(stack_orientation(settings)),
					)
					ratioed = if old_ratio != settings.ratio apply_master_ratio(arranged_forest, settings, master, stack) else arranged_forest
					distributed({ ..at, forest: ratioed }, layout)
				},
			)
		}

	master_create_layout : Master -> (Master, U64)
	master_create_layout = |system| {
		settings = system.settings
		(created, layout) = create_layout(system.forest)
		top = root(created, layout, "create_layout")
		kinded = set_kind(created, top, Split(root_orientation(settings)))
		(first_added, first) = insert(kinded, back_of(kinded, top))
		(second_added, second) = insert(first_added, back_of(first_added, top))
		(master, stack) = if master_first(settings) (first, second) else (second, first)
		regioned = set_kind(set_kind(second_added, master, Split(master_orientation(settings))), stack, Split(stack_orientation(settings)))
		ratioed = apply_master_ratio(regioned, settings, master, stack)
		({ ..system, forest: set_region_pair(ratioed, top, master) }, layout)
	}

	master_close_app : Master, List(U64) -> Master
	master_close_app = |system, windows| {
		layouts = layout_keys(system.forest).keep_if(|layout| has_windows_for_app(system.forest, { layout, windows }))
		distribute_each({ ..system, forest: close_app(system.forest, windows) }, layouts)
	}

	distribute_each : Master, List(U64) -> Master
	distribute_each = |system, layouts| layouts.fold(system, |at, layout| distributed(at, layout))

	apply_master_ratio : Forest, MasterSettings, U64, U64 -> Forest
	apply_master_ratio = |forest, settings, master, stack| {
		ratio = Geometry.narrow(clamp64(settings.ratio, 0.05, 0.95))
		total = F32.from_bits(0x4000_0000)
		master_size = max32(ratio * total, twentieth32)
		stack_size = max32(total - master_size, twentieth32)
		set_size(set_size(forest, master, master_size), stack, stack_size)
	}

	distribute_regions : Master, U64 -> (Master, Bool)
	distribute_regions = |system, layout| {
		{ master, stack, .. } = region_roots(system.forest, layout)
		current = count_windows(system.forest, master)
		enforce_master_count(system, layout, master, stack, current, Nothing)
	}

	distributed : Master, U64 -> Master
	distributed = |system, layout| {
		(after, _) = distribute_regions(system, layout)
		after
	}

	## Windows moved between the regions until the master region holds the configured count
	## (`enforce_master_count`).
	enforce_master_count : Master, U64, U64, U64, U64, MaybeWord -> (Master, Bool)
	enforce_master_count = |system, layout, master, stack, current, keep_in_master| {
		settings = system.settings
		desired = settings.count
		if current == desired {
			(system, False)
		} else {
			overflow = current > desired
			count = if overflow current - desired else desired - current
			(source, destination) = if overflow (master, stack) else (stack, master)
			forward = !overflow and master_first(settings)
			start = if forward source else last_descendant(system.forest, source)
			protected =
				match keep_in_master {
					Some(kept) if overflow and desired != 0 => window_node(system.forest, layout, kept)
					_ => Nothing
				}
			match next_window(system.forest, source, forward, protected, Some(start)) {
				Nothing => (system, False)
				Some(leaf) => {
					selected =
						match focused_leaf(system.forest, layout) {
							Found(key, _) => Some(key)
							NotFound => Nothing
						}
					moves = { source, destination, forward, protected, selected, front: overflow == master_first(settings) }
					({ ..system, forest: move_leaves(system.forest, moves, leaf, count) }, True)
				}
			}
		}
	}

	## From `leaf` on, windows (or whole groups that fit the quota) moved to the destination
	## region until `remaining` have gone.
	move_leaves : Forest, Moves, U64, U64 -> Forest
	move_leaves = |forest, moves, leaf, remaining| {
		(node_moving, moved_count) =
			if remaining > 1 {
				# Each sibling subtree counted once, walking out from the leaf: a whole group
				# moves at once while a partial quota still moves leaves.
				widest(forest, moves.source, moves.protected, remaining, leaf, 1, (leaf, 1))
			} else {
				(leaf, 1)
			}
		restore_focus =
			match moves.selected {
				Some(focus_key) if is_within(forest, focus_key, node_moving) => Some(focus_key)
				_ => Nothing
			}
		left = remaining - moved_count
		next =
			if left == 0 {
				Nothing
			} else {
				# The next leaf keeps its identity even if moving this branch collapses an
				# ancestor.
				edge = if moves.forward last_descendant(forest, node_moving) else node_moving
				next_window(forest, moves.source, moves.forward, moves.protected, step_in(forest, edge, moves.source, moves.forward))
			}
		shifted = move_to_region(forest, node_moving, moves.destination, moves.front)
		refocused =
			match restore_focus {
				Some(focus_key) => select_node(shifted, focus_key)
				Nothing => shifted
			}
		match next {
			Some(following) => move_leaves(refocused, moves, following, left)
			Nothing => refocused
		}
	}

	## The widest subtree around `child` whose windows fit the quota (`remaining`), walking out
	## to the region.
	widest : Forest, U64, MaybeWord, U64, U64, U64, (U64, U64) -> (U64, U64)
	widest = |forest, source, protected, remaining, child, counted, chosen|
		match parent(forest, child) {
			Some(above) if above != source => {
				others = children(forest, above).drop_if(|key| key == child).fold(0, |sum, key| sum + count_windows(forest, key))
				total = counted + others
				guarded =
					match protected {
						Some(keep) => is_within(forest, keep, above)
						Nothing => False
					}
				if total > remaining or guarded {
					chosen
				} else {
					widest(forest, source, protected, remaining, above, total, if total > 1 (above, total) else chosen)
				}
			}
			_ => chosen
		}

	## The first window from `start` on, stepping through `region`, other than `protected`.
	next_window : Forest, U64, Bool, MaybeWord, MaybeWord -> MaybeWord
	next_window = |forest, region, forward, protected, start|
		match start {
			Some(key) =>
				if Some(key) != protected and is_window(forest, key) {
					Some(key)
				} else {
					next_window(forest, region, forward, protected, step_in(forest, key, region, forward))
				}
			Nothing => Nothing
		}

	move_to_region : Forest, U64, U64, Bool -> Forest
	move_to_region = |forest, key, region, front| {
		placement =
			match first_child(forest, region) {
				Some(first) if front => Before(first)
				_ => Back(region)
			}
		moved(forest, key, placement)
	}

	focused_container : Forest, U64, U64, U64 -> MaybeWord
	focused_container = |forest, layout, master, stack|
		match focused_leaf(forest, layout) {
			Found(key, _) => found(ancestors(forest, key).find_first(|at| at == master or at == stack))
			NotFound => Nothing
		}

	target_container_for_new_window : Master, U64, U64, U64, U64 -> U64
	target_container_for_new_window = |system, layout, master, stack, count|
		if count < system.settings.count {
			master
		} else {
			match system.settings.placement {
				Master => master
				Stack => stack
				Focused =>
					match focused_container(system.forest, layout, master, stack) {
						Some(key) => key
						Nothing => master
					}
			}
		}

	insert_window_in_container : Master, U64, U64, U64 -> (Forest, U64)
	insert_window_in_container = |system, layout, region, window_word| {
		forest = system.forest
		place =
			match system.settings.insertion {
				NextToSelection =>
					match position(forest, selection(forest, layout)) {
						At(at) if at.above == region => after_place(at)
						_ => back_of(forest, region)
					}
				EndOfTree => back_of(forest, region)
				# The front of the region: before its first child, or its back when it has none.
				Front => { above: region, index: 0 }
			}
		(inserted, key) = insert(forest, place)
		(set_window(inserted, key, window_word), key)
	}

	## A window added to the region the settings choose, the master count then enforced.
	master_add_window : Master, BspWindowIn -> Master
	master_add_window = |system, { layout, window: window_word }|
		if contains_window(system.forest, layout, window_word) {
			system
		} else {
			{ master, stack, .. } = region_roots(system.forest, layout)
			count = count_windows(system.forest, master)
			target = target_container_for_new_window(system, layout, master, stack, count)
			(inserted, key) = insert_window_in_container(system, layout, target, window_word)
			selected = { ..system, forest: select_node(inserted, key) }
			next_to_selection =
				match system.settings.insertion {
					NextToSelection => True
					_ => False
				}
			keep = if next_to_selection and target == master Some(window_word) else Nothing
			(enforced, _) = enforce_master_count(selected, layout, master, stack, count + (if target == master 1 else 0), keep)
			at_end =
				match system.settings.insertion {
					EndOfTree => True
					_ => False
				}
			match parent(enforced.forest, key) {
				Some(above) if at_end => {
					# Appended in the final region, after the master overflow is resolved.
					{ ..enforced, forest: select_node(moved(enforced.forest, key, Back(above)), key) }
				}
				_ => enforced
			}
		}

	adjust_master_ratio : Master, F64 -> Master
	adjust_master_ratio = |system, delta| {
		next = clamp64(system.settings.ratio + delta, 0.05, 0.95)
		if (next - system.settings.ratio).abs() < Geometry.epsilon {
			system
		} else {
			updated = { ..system, settings: { ..system.settings, ratio: next } }
			layout_keys(updated.forest).fold(
				updated,
				|at, layout| {
					distributed_system = distributed(at, layout)
					{ master, stack, .. } = region_roots(distributed_system.forest, layout)
					{ ..distributed_system, forest: apply_master_ratio(distributed_system.forest, distributed_system.settings, master, stack) }
				},
			)
		}
	}

	adjust_master_count : Master, I32 -> Master
	adjust_master_count = |system, delta| {
		current = system.settings.count.to_i32_wrap()
		summed = current + delta
		next = (if summed > 1 summed else 1).to_u64_wrap()
		if next == system.settings.count {
			system
		} else {
			updated = { ..system, settings: { ..system.settings, count: next } }
			distribute_each(updated, layout_keys(updated.forest))
		}
	}

	promote_to_master : Master, U64 -> Master
	promote_to_master = |system, layout| {
		{ master, stack, .. } = region_roots(system.forest, layout)
		match focused_leaf(system.forest, layout) {
			NotFound => system
			Found(key, focused) => {
				forest = system.forest
				overflow = last_descendant(forest, master)
				(swapped_forest, promoted) =
					match window_at(forest, overflow) {
						Some(overflow_window) if !is_within(forest, key, master) and count_windows(forest, master) >= system.settings.count => {
							(after, swapped) = swap_windows(forest, layout, focused, overflow_window)
							(demoted, raised) = if swapped (key, overflow) else (overflow, key)
							(move_to_region(after, demoted, stack, True), raised)
						}
						_ => (forest, key)
					}
				promoted_forest = move_to_region(swapped_forest, promoted, master, True)
				current = count_windows(promoted_forest, master)
				enforced =
					if current > system.settings.count {
						(after, _) = enforce_master_count({ ..system, forest: promoted_forest }, layout, master, stack, current, Some(focused))
						after
					} else {
						{ ..system, forest: promoted_forest }
					}
				{ ..enforced, forest: select_node(enforced.forest, promoted) }
			}
		}
	}

	## The window chosen in a region: its chosen child (or first), the first window under it.
	focused_window_in_container : Forest, U64 -> MaybeWord
	focused_window_in_container = |forest, region|
		match or_else(local_selection(forest, region), |{}| first_child(forest, region)) {
			Some(candidate) =>
				walk(
					forest,
					candidate,
					Preorder,
					Nothing,
					|_, _, at|
						match at.body {
							Window({ window: leaf, .. }) => Break(Some(leaf))
							Container(_) => Continue(Nothing)
						},
				)
			Nothing => Nothing
		}

	swap_master_stack : Master, U64 -> Master
	swap_master_stack = |system, layout| {
		{ master, stack, .. } = region_roots(system.forest, layout)
		match (focused_window_in_container(system.forest, master), focused_window_in_container(system.forest, stack)) {
			(Some(master_window), Some(stack_window)) => {
				(after, _) = swap_windows(system.forest, layout, master_window, stack_window)
				{ ..system, forest: after }
			}
			_ => system
		}
	}

	master_can_move_window : Master, BspMoveQuery -> Bool
	master_can_move_window = |system, { layout, window: window_word, direction }|
		takes_part(system.forest, window_word)
		and (
			match window_node(system.forest, layout, window_word) {
				Some(key) => move_target(system, layout, key, direction) != Nothing
				Nothing => False
			}
		)

	master_edit : Master, GroupEditIn -> (Master, BspEdited)
	master_edit = |system, { layout, edit }| {
		on_forest = |(after, edited)| ({ ..system, forest: after }, edited)
		match edit {
			Fullscreen(mode) => on_forest(edit_fullscreen(system.forest, layout, mode))
			Swap({ a, b }) => on_forest(edited_by(swap_windows(system.forest, layout, a, b)))
			ReorderTab({ window: window_word, target }) => on_forest(edited_by(move_stack_tab(system.forest, layout, window_word, target)))
			Resize(asked) => on_forest(resize_selection(system.forest, layout, asked))
			Drop(dropped) => on_forest(edit_drop(system.forest, layout, dropped))
			Moved({ direction, .. }) => {
				(after, did) = master_move_selection(system, layout, direction)
				(after, if did Changed else Unchanged)
			}
			Split(_) | Join(_) | ConsumeOrExpel(_) | Stack(_) | Unstack(_) | Unjoin | Rebalance | ToggleOrientation => {
				(after, did) = distribute_regions(system, layout)
				(after, if did Changed else Unchanged)
			}
		}
	}

	master_move_selection : Master, U64, BspDirection -> (Master, Bool)
	master_move_selection = |system, layout, direction|
		match focused_leaf(system.forest, layout) {
			NotFound => (system, False)
			Found(key, focused) =>
				match move_target(system, layout, key, direction) {
					Nothing => (system, False)
					Some(target) => {
						(swapped, did) = swap_windows(system.forest, layout, focused, target)
						if did {
							(refocused, _) = focus(swapped, { layout, focus: Select(focused) })
							({ ..system, forest: refocused }, True)
						} else {
							(system, False)
						}
					}
				}
		}

	## The window a move from `key` swaps with (`move_target`).
	move_target : Master, U64, U64, BspDirection -> MaybeWord
	move_target = |system, layout, key, direction| {
		settings = system.settings
		forest = system.forest
		{ master, stack, .. } = region_roots(forest, layout)
		in_master = is_within(forest, key, master)
		(region, other, axis) = if in_master (master, stack, master_orientation(settings)) else (stack, master, stack_orientation(settings))
		forward = !backwards(direction)
		parallel = same_orientation(orientation_of(direction), axis)
		toward_other = same_orientation(orientation_of(direction), root_orientation(settings)) and ((forward == master_first(settings)) == in_master)
		start = step_in(forest, key, region, forward)
		neighbors = windows_from(forest, region, start, forward)
		first_taking_part = |listed| found(listed.find_first(|leaf| takes_part(forest, leaf)))
		if toward_other and (!parallel or neighbors.is_empty()) {
			# The destination entered from its near edge, past windows left out.
			edge = if forward other else last_descendant(forest, other)
			destination = windows_from(forest, other, Some(edge), forward)
			if !destination.is_empty() {
				first_taking_part(destination)
			} else if in_master {
				# No stack region windows: the next master window, if there is one.
				first_taking_part(windows_from(forest, region, next_in_subtree(forest, key, region), True))
			} else if parallel {
				first_taking_part(neighbors)
			} else {
				Nothing
			}
		} else if parallel {
			first_taking_part(neighbors)
		} else {
			Nothing
		}
	}
}
