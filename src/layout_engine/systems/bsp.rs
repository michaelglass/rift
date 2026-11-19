use objc2_core_foundation::CGRect;
use rift_core::layout::{bsp, bsp_tree};
use serde::{Deserialize, Serialize};
use slotmap::Key;

use crate::actor::app::{WindowId, pid_t};
use crate::common::collections::{HashMap, HashSet};
use crate::common::config::WindowInsertionPoint;
use crate::layout_engine::systems::{
    LayoutSystem, WindowLayoutConstraints, reconcile_app_membership,
};
use crate::layout_engine::utils::compute_tiling_area;
use crate::layout_engine::{Direction, LayoutId, LayoutKind, Orientation, ResizeOrientation};
use crate::model::selection::*;
use crate::model::tree::{NodeId, NodeMap, Tree};

#[derive(Serialize, Deserialize, Clone, Debug)]
enum NodeKind {
    Split {
        orientation: Orientation,
        ratio: f32,
    },
    Leaf {
        window: Option<WindowId>,
        fullscreen: bool,
        fullscreen_within_gaps: bool,
        preselected: Option<Direction>,
    },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
struct LayoutState {
    root: NodeId,
}

/// The BSP layout system. Its trees are held twice while the core takes over (phase B): as
/// the core holds them (`core`, what every caller is answered from) and in rift's NodeId form
/// (what is saved, and what the core is checked against in test builds).
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(try_from = "SavedBsp")]
pub struct BspLayoutSystem {
    #[serde(skip)]
    inactive: HashSet<WindowId>,
    layouts: slotmap::SlotMap<crate::layout_engine::LayoutId, LayoutState>,
    tree: Tree<Components>,
    kind: slotmap::SecondaryMap<NodeId, NodeKind>,
    /// Ordered members for BSP leaves that have been explicitly stacked.
    #[serde(default)]
    stacks: HashMap<NodeId, Vec<WindowId>>,
    #[serde(skip, default)]
    window_insertion_point: WindowInsertionPoint,
    #[serde(skip, default)]
    single_window_aspect_ratio: Option<f64>,
    /// The trees as the core holds them.
    #[serde(skip)]
    core: bsp_tree::Layouts<LayoutId, WindowId>,
    /// How deep inside an operation this system is: operations call each other, and the core
    /// takes each change once, at the outermost; inside one, queries read the NodeId trees.
    #[serde(skip)]
    depth: u32,
}

/// A saved BSP layout system, as rift writes it: parsed into the core's trees on load.
#[derive(Deserialize)]
struct SavedBsp {
    layouts: slotmap::SlotMap<crate::layout_engine::LayoutId, LayoutState>,
    tree: Tree<Components>,
    kind: slotmap::SecondaryMap<NodeId, NodeKind>,
    #[serde(default)]
    stacks: HashMap<NodeId, Vec<WindowId>>,
}

impl TryFrom<SavedBsp> for BspLayoutSystem {
    type Error = OldTree;

    fn try_from(saved: SavedBsp) -> Result<Self, OldTree> {
        let mut system = Self {
            layouts: saved.layouts,
            tree: saved.tree,
            kind: saved.kind,
            stacks: saved.stacks,
            ..Self::default()
        };
        system.core = system.core_layouts()?;
        Ok(system)
    }
}

impl BspLayoutSystem {
    fn participating_window(&self, node: NodeId) -> Option<WindowId> {
        let Some(NodeKind::Leaf { window, .. }) = self.kind.get(node) else {
            return None;
        };
        window.filter(|w| !self.inactive.contains(w)).or_else(|| {
            self.stacks.get(&node)?.iter().copied().find(|w| !self.inactive.contains(w))
        })
    }

    fn find_neighbor_leaf(&self, from_leaf: NodeId, direction: Direction) -> Option<NodeId> {
        let mut current = from_leaf;
        while let Some(next) = self.find_structural_neighbor_leaf(current, direction) {
            if self.inactive.is_empty() || self.participating_window(next).is_some() {
                return Some(next);
            }
            current = next;
        }
        None
    }

    fn find_structural_neighbor_leaf(
        &self,
        from_leaf: NodeId,
        direction: Direction,
    ) -> Option<NodeId> {
        // Track the source center across the movement axis, relative to each ancestor.
        let mut anchor = 0.5;
        let mut current = from_leaf;

        while let Some(parent) = current.parent(&self.tree.map) {
            if let Some(NodeKind::Split { orientation, ratio }) = self.kind.get(parent) {
                if *orientation == direction.orientation() {
                    let children: Vec<_> = parent.children(&self.tree.map).collect();
                    if children.len() == 2 {
                        let is_first = children[0] == current;
                        let target_child = match direction {
                            Direction::Left | Direction::Up => {
                                if !is_first {
                                    Some(children[0])
                                } else {
                                    None
                                }
                            }
                            Direction::Right | Direction::Down => {
                                if is_first {
                                    Some(children[1])
                                } else {
                                    None
                                }
                            }
                        };

                        if let Some(target) = target_child {
                            return Some(
                                self.find_closest_leaf_in_direction(target, direction, anchor),
                            );
                        }
                    }
                } else {
                    let ratio = f64::from(*ratio);
                    anchor = if parent.first_child(&self.tree.map) == Some(current) {
                        anchor * ratio
                    } else {
                        ratio + anchor * (1.0 - ratio)
                    };
                }
            }
            current = parent;
        }

        None
    }

    fn find_closest_leaf_in_direction(
        &self,
        root: NodeId,
        direction: Direction,
        mut anchor: f64,
    ) -> NodeId {
        match self.kind.get(root) {
            Some(NodeKind::Leaf { .. }) => root,
            Some(NodeKind::Split { orientation, ratio }) => {
                let children: Vec<_> = root.children(&self.tree.map).collect();
                if children.is_empty() {
                    return root;
                }

                let target_child = if *orientation == direction.orientation() {
                    match direction {
                        Direction::Left | Direction::Up => children.last().copied(),
                        Direction::Right | Direction::Down => children.first().copied(),
                    }
                } else {
                    let ratio = f64::from(*ratio);
                    // Choose the adjacent side and express the anchor within that child.
                    if anchor <= ratio {
                        anchor /= ratio;
                        children.first().copied()
                    } else {
                        anchor = (anchor - ratio) / (1.0 - ratio);
                        children.last().copied()
                    }
                };

                if let Some(child) = target_child {
                    self.find_closest_leaf_in_direction(child, direction, anchor)
                } else {
                    root
                }
            }
            None => root,
        }
    }

    fn window_in_direction_from(&self, node: NodeId, direction: Direction) -> Option<WindowId> {
        match self.kind.get(node) {
            Some(NodeKind::Leaf { window: Some(_), .. }) => self.participating_window(node),
            Some(NodeKind::Leaf { .. }) => None,
            Some(NodeKind::Split { .. }) => {
                let mut children: Vec<_> = node.children(&self.tree.map).collect();
                match direction {
                    Direction::Left | Direction::Up => children.reverse(),
                    Direction::Right | Direction::Down => {}
                }
                for child in children {
                    if let Some(window) = self.window_in_direction_from(child, direction) {
                        return Some(window);
                    }
                }
                None
            }
            None => None,
        }
    }

    fn smart_insert_window(&mut self, layout: LayoutId, window: WindowId) -> bool {
        if let Some(sel) = self.selection_of_layout(layout) {
            let leaf = self.descend_to_leaf(sel);

            if let Some(NodeKind::Leaf {
                preselected: Some(direction), ..
            }) = self.kind.get(leaf).cloned()
            {
                self.split_leaf_in_direction(leaf, direction, window);

                if let Some(NodeKind::Leaf { preselected, .. }) = self.kind.get_mut(leaf) {
                    *preselected = None;
                }
                return true;
            }
        }
        false
    }

    fn split_leaf_in_direction(
        &mut self,
        leaf: NodeId,
        direction: Direction,
        new_window: WindowId,
    ) {
        if let Some(NodeKind::Leaf { window, .. }) = self.kind.get(leaf).cloned() {
            let existing_stack = self.stacks.remove(&leaf);
            let orientation = direction.orientation();

            let existing_node = self.make_leaf(window);
            let new_node = self.make_leaf(Some(new_window));

            if let Some(stack) = existing_stack {
                self.stacks.insert(existing_node, stack);
            }

            self.kind.insert(leaf, NodeKind::Split { orientation, ratio: 0.5 });

            let (first_child, second_child) = match direction {
                Direction::Left | Direction::Up => (new_node, existing_node),
                Direction::Right | Direction::Down => (existing_node, new_node),
            };

            first_child.detach(&mut self.tree).push_back(leaf);
            second_child.detach(&mut self.tree).push_back(leaf);

            self.tree.data.selection.select(&self.tree.map, new_node);
        }
    }
}

impl Default for BspLayoutSystem {
    fn default() -> Self {
        Self {
            inactive: Default::default(),
            layouts: Default::default(),
            tree: Tree::with_observer(Components::default()),
            kind: Default::default(),
            stacks: Default::default(),
            window_insertion_point: WindowInsertionPoint::default(),
            single_window_aspect_ratio: None,
            core: Default::default(),
            depth: 0,
        }
    }
}

impl BspLayoutSystem {
    pub fn new(window_insertion_point: WindowInsertionPoint) -> Self {
        Self {
            window_insertion_point,
            ..Self::default()
        }
    }

    pub fn set_window_insertion_point(&mut self, value: WindowInsertionPoint) {
        self.window_insertion_point = value;
    }

    pub fn set_single_window_aspect_ratio(&mut self, ratio: Option<f64>) {
        self.single_window_aspect_ratio = ratio.filter(|ratio| ratio.is_finite() && *ratio > 0.0);
    }

    fn node_for_window_in_layout(&self, layout: LayoutId, wid: WindowId) -> Option<NodeId> {
        let root = self.layouts.get(layout)?.root;
        root.traverse_preorder(&self.tree.map).find(|&node| match self.kind.get(node) {
            Some(NodeKind::Leaf { window, .. }) => self
                .stacks
                .get(&node)
                .map_or(*window == Some(wid), |stack| stack.contains(&wid)),
            _ => false,
        })
    }

    fn make_leaf(&mut self, window: Option<WindowId>) -> NodeId {
        let id = self.tree.mk_node().into_id();
        self.kind.insert(id, NodeKind::Leaf {
            window,
            fullscreen: false,
            fullscreen_within_gaps: false,
            preselected: None,
        });
        id
    }

    fn descend_to_leaf(&self, mut node: NodeId) -> NodeId {
        if !self.inactive.is_empty()
            && let Some(active) = node
                .traverse_preorder(&self.tree.map)
                .find(|&candidate| self.participating_window(candidate).is_some())
        {
            return active;
        }
        loop {
            match self.kind.get(node) {
                Some(NodeKind::Leaf { .. }) => return node,
                Some(NodeKind::Split { .. }) => {
                    if let Some(child) = node.first_child(&self.tree.map) {
                        node = child;
                    } else {
                        return node;
                    }
                }
                None => return node,
            }
        }
    }

    /// Calculates the depth of a node in the BSP tree by counting parent levels.
    /// The root node has depth 0, its direct children have depth 1, and so on.
    fn node_depth(&self, node: NodeId) -> usize {
        let mut depth = 0;
        let mut current = node;
        while let Some(parent) = current.parent(&self.tree.map) {
            depth += 1;
            current = parent;
        }
        depth
    }

    /// Returns the orientation for a new split based on the node's depth.
    /// Creates a fibonacci spiral pattern by alternating orientations:
    /// - Even depth (0, 2, 4, ...) → Horizontal split
    /// - Odd depth (1, 3, 5, ...) → Vertical split
    fn orientation_for_depth(&self, depth: usize) -> Orientation {
        if depth % 2 == 0 {
            Orientation::Horizontal
        } else {
            Orientation::Vertical
        }
    }

    fn collect_windows_under(&self, node: NodeId, out: &mut Vec<WindowId>) {
        match self.kind.get(node) {
            Some(NodeKind::Leaf { window, .. }) => {
                if let Some(stack) = self.stacks.get(&node) {
                    out.extend(stack.iter().copied());
                } else if let Some(w) = window {
                    out.push(*w);
                }
            }
            Some(NodeKind::Split { .. }) => {
                for child in node.children(&self.tree.map) {
                    self.collect_windows_under(child, out);
                }
            }
            None => {}
        }
    }

    fn collect_visible_windows_under(&self, node: NodeId, out: &mut Vec<WindowId>) {
        match self.kind.get(node) {
            Some(NodeKind::Leaf { window: Some(_), .. }) => {
                out.extend(self.participating_window(node))
            }
            Some(NodeKind::Leaf { .. }) => {}
            Some(NodeKind::Split { .. }) => {
                for child in node.children(&self.tree.map) {
                    self.collect_visible_windows_under(child, out);
                }
            }
            None => {}
        }
    }

    fn has_fullscreen_in_subtree(&self, node: NodeId) -> bool {
        match self.kind.get(node) {
            Some(NodeKind::Leaf {
                fullscreen,
                fullscreen_within_gaps,
                ..
            }) => {
                self.participating_window(node).is_some()
                    && (*fullscreen || *fullscreen_within_gaps)
            }
            Some(NodeKind::Split { .. }) => {
                node.children(&self.tree.map).any(|child| self.has_fullscreen_in_subtree(child))
            }
            None => false,
        }
    }

    // Return the number of tiles sharing the parent's axis; perpendicular splits count as one.
    fn rebalance_node(&mut self, node: NodeId, parent_axis: Orientation) -> usize {
        let Some(&NodeKind::Split { orientation, .. }) = self.kind.get(node) else {
            return 1;
        };
        let (Some(first), Some(second)) =
            (node.first_child(&self.tree.map), node.last_child(&self.tree.map))
        else {
            return 1;
        };
        let first_span = self.rebalance_node(first, orientation);
        let second_span = self.rebalance_node(second, orientation);
        let span = first_span + second_span;
        if let Some(NodeKind::Split { ratio, .. }) = self.kind.get_mut(node) {
            *ratio = (first_span as f32 / span as f32).clamp(0.05, 0.95);
        }
        if orientation == parent_axis { span } else { 1 }
    }

    fn cleanup_after_removal(&mut self, node: NodeId) -> NodeId {
        let Some(parent_id) = node.parent(&self.tree.map) else {
            return node;
        };

        if !matches!(self.kind.get(parent_id), Some(NodeKind::Split { .. })) {
            return parent_id;
        }

        let children: Vec<_> = parent_id.children(&self.tree.map).collect();
        if children.len() != 2 {
            return parent_id;
        }
        node.detach(&mut self.tree).remove();
        self.kind.remove(node);
        self.stacks.remove(&node);
        self.collapse_split(parent_id);
        parent_id
    }

    /// Promote the sole remaining child while keeping the layout's root stable.
    fn collapse_split(&mut self, parent_id: NodeId) {
        if !matches!(self.kind.get(parent_id), Some(NodeKind::Split { .. })) {
            return;
        }
        let children: Vec<_> = parent_id.children(&self.tree.map).collect();
        let [sibling] = children.as_slice() else { return };
        let sibling = *sibling;
        let sibling_kind = match self.kind.get(sibling) {
            Some(k) => k.clone(),
            None => return,
        };

        match &sibling_kind {
            NodeKind::Split { .. } => {
                let sib_children: Vec<_> = sibling.children(&self.tree.map).collect();
                for c in sib_children {
                    c.detach(&mut self.tree).push_back(parent_id);
                }
            }
            NodeKind::Leaf { .. } => {
                if let Some(stack) = self.stacks.remove(&sibling) {
                    self.stacks.insert(parent_id, stack);
                }
            }
        }
        self.kind.insert(parent_id, sibling_kind);

        sibling.detach(&mut self.tree).remove();
        self.kind.remove(sibling);
        self.stacks.remove(&sibling);
    }

    fn selection_of_layout(&self, layout: crate::layout_engine::LayoutId) -> Option<NodeId> {
        let state = self.layouts.get(layout)?;
        if !self.inactive.is_empty()
            && !state
                .root
                .traverse_preorder(&self.tree.map)
                .any(|n| self.participating_window(n).is_some())
        {
            return None;
        }
        let selected = self.tree.data.selection.current_selection(state.root);
        if matches!(
            self.kind.get(selected),
            Some(NodeKind::Leaf { window: Some(_), .. })
        ) && self.participating_window(selected).is_none()
        {
            return state
                .root
                .traverse_preorder(&self.tree.map)
                .find(|&n| self.participating_window(n).is_some());
        }
        Some(selected)
    }

    fn insert_window_at_selection(
        &mut self,
        layout: crate::layout_engine::LayoutId,
        wid: WindowId,
    ) {
        let Some(state) = self.layouts.get(layout).copied() else {
            return;
        };
        let sel = self
            .selection_of_layout(layout)
            .unwrap_or_else(|| self.tree.data.selection.current_selection(state.root));
        match self.kind.get_mut(sel) {
            Some(NodeKind::Leaf {
                window,
                fullscreen,
                fullscreen_within_gaps,
                ..
            }) => {
                if window.is_none() {
                    *window = Some(wid);
                    *fullscreen = false;
                    *fullscreen_within_gaps = false;
                } else {
                    let existing = *window;
                    let left = self.make_leaf(existing);
                    let right = self.make_leaf(Some(wid));
                    if let Some(stack) = self.stacks.remove(&sel) {
                        self.stacks.insert(left, stack);
                    }

                    // Use alternating orientations based on depth for fibonacci spiral
                    let depth = self.node_depth(sel);
                    let orientation = self.orientation_for_depth(depth);
                    self.kind.insert(sel, NodeKind::Split { orientation, ratio: 0.5 });
                    left.detach(&mut self.tree).push_back(sel);
                    right.detach(&mut self.tree).push_back(sel);
                    self.tree.data.selection.select(&self.tree.map, right);
                }
            }
            Some(NodeKind::Split { .. }) => {
                let leaf = self.descend_to_leaf(sel);
                self.tree.data.selection.select(&self.tree.map, leaf);
                self.insert_window_at_selection(layout, wid);
            }
            None => {}
        }
    }

    /// Remove an occurrence and report progress so callers can clear legacy duplicates.
    fn remove_window_internal(&mut self, layout: LayoutId, wid: WindowId) -> bool {
        let Some(node) = self.node_for_window_in_layout(layout, wid) else {
            return false;
        };
        if let Some(stack) = self.stacks.get_mut(&node) {
            stack.retain(|window| *window != wid);
            if let Some(&visible) = stack.last() {
                if let Some(NodeKind::Leaf { window, .. }) = self.kind.get_mut(node) {
                    *window = Some(visible);
                }
                self.tree.data.selection.select(&self.tree.map, node);
                return true;
            }
            self.stacks.remove(&node);
        }
        // An emptied leaf fills nothing: it keeps no fullscreen flags (as filling one clears them).
        if let Some(NodeKind::Leaf {
            window,
            fullscreen,
            fullscreen_within_gaps,
            ..
        }) = self.kind.get_mut(node)
        {
            *window = None;
            *fullscreen = false;
            *fullscreen_within_gaps = false;
        }
        let fallback = self.cleanup_after_removal(node);
        let selection = self.tree.data.selection.current_selection(self.layouts[layout].root);
        let new_selection = if self.kind.contains_key(selection) {
            self.descend_to_leaf(selection)
        } else {
            self.descend_to_leaf(fallback)
        };
        self.tree.data.selection.select(&self.tree.map, new_selection);
        true
    }

    /// The layout's shape for the core, pruned to what takes space: a leaf whose window
    /// participates, an empty slot (unless some windows sit out), and splits over them.
    fn shape(
        &self,
        node: NodeId,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
    ) -> Option<bsp::Node<WindowId>> {
        match self.kind.get(node)? {
            NodeKind::Leaf {
                window,
                fullscreen,
                fullscreen_within_gaps,
                ..
            } => {
                let slot = match self.participating_window(node) {
                    Some(window) => bsp::Slot::Window {
                        window,
                        constraints: constraints.get(&window).copied(),
                    },
                    None if self.inactive.is_empty() && window.is_none() => bsp::Slot::Empty,
                    None => return None,
                };
                let fill = match (fullscreen, fullscreen_within_gaps) {
                    (true, _) => bsp::Fill::Screen,
                    (false, true) => bsp::Fill::WithinGaps,
                    (false, false) => bsp::Fill::Tiled,
                };
                Some(bsp::Node::Leaf {
                    slot,
                    fill,
                    stacked: self.stacks.contains_key(&node),
                })
            }
            NodeKind::Split { orientation, ratio } => {
                let mut children = node
                    .children(&self.tree.map)
                    .filter_map(|child| self.shape(child, constraints));
                let children = match (children.next(), children.next()) {
                    (None, _) => return None,
                    (Some(one), None) => bsp::Children::One(Box::new(one)),
                    (Some(first), Some(second)) => {
                        bsp::Children::Two(Box::new(first), Box::new(second))
                    }
                };
                Some(bsp::Node::Split {
                    horizontal: *orientation == Orientation::Horizontal,
                    ratio: *ratio,
                    children,
                })
            }
        }
    }

    fn selection_window(&self, state: &LayoutState) -> Option<WindowId> {
        let sel = self.tree.data.selection.current_selection(state.root);
        self.participating_window(sel)
    }
}

#[derive(Clone, Default, Serialize, Deserialize, Debug)]
struct Components {
    selection: Selection,
}

impl crate::model::tree::Observer for Components {
    fn added_to_forest(&mut self, map: &NodeMap, node: NodeId) {
        self.selection.handle_event(map, TreeEvent::AddedToForest(node))
    }

    fn added_to_parent(&mut self, map: &NodeMap, node: NodeId) {
        self.selection.handle_event(map, TreeEvent::AddedToParent(node))
    }

    fn removing_from_parent(&mut self, map: &NodeMap, node: NodeId) {
        self.selection.handle_event(map, TreeEvent::RemovingFromParent(node))
    }

    fn removed_child(_tree: &mut Tree<Self>, _parent: NodeId) {}

    fn removed_from_forest(&mut self, map: &NodeMap, node: NodeId) {
        self.selection.handle_event(map, TreeEvent::RemovedFromForest(node))
    }
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::{CGPoint, CGSize};
    use rift_core::layout::{Size, Sizing};

    use super::*;

    fn w(idx: u32) -> WindowId { WindowId::new(1, idx) }

    fn tiled(windows: &[u32]) -> (BspLayoutSystem, LayoutId) {
        let mut system = BspLayoutSystem::default();
        let layout = system.create_layout();
        for &window in windows {
            system.add_window_after_selection(layout, w(window));
        }
        (system, layout)
    }

    fn calculate_frames(
        system: &BspLayoutSystem,
        layout: LayoutId,
        size: CGSize,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
    ) -> HashMap<WindowId, CGRect> {
        system
            .calculate_layout(
                layout,
                CGRect::new(CGPoint::new(0.0, 0.0), size),
                0.0,
                constraints,
                &Default::default(),
                Default::default(),
            )
            .into_iter()
            .collect()
    }

    fn assert_frame(
        frames: &HashMap<WindowId, CGRect>,
        window: WindowId,
        rect: (f64, f64, f64, f64),
    ) {
        let (x, y, width, height) = rect;
        assert_eq!(
            frames[&window],
            CGRect::new(CGPoint::new(x, y), CGSize::new(width, height))
        );
    }

    #[test]
    fn app_exit_clears_saved_display_layouts_and_reclaims_frames() {
        use crate::layout_engine::workspaces::WorkspaceLayoutState;

        let mut system = BspLayoutSystem::default();
        let mut workspace = WorkspaceLayoutState::default();
        let sizes = [CGSize::new(1000.0, 800.0), CGSize::new(1600.0, 900.0)];
        workspace.ensure_active(sizes[0], &mut system);
        let original = workspace.active().unwrap();
        system.add_window_after_selection(original, w(1));
        let survivor = WindowId::new(2, 1);
        system.add_window_after_selection(original, survivor);
        workspace.ensure_active(sizes[1], &mut system);
        let cloned = workspace.active().unwrap();
        system.remove_windows_for_app(1);
        for (layout, size) in [(cloned, sizes[1]), (original, sizes[0])] {
            workspace.ensure_active(size, &mut system);
            assert_eq!(workspace.active(), Some(layout));
            assert_eq!(system.all_windows_in_layout(layout), vec![survivor]);
            assert_eq!(system.selected_window(layout), Some(survivor));
            let frames = calculate_frames(&system, layout, size, &Default::default());
            assert_frame(&frames, survivor, (0.0, 0.0, size.width, size.height));
        }
    }

    #[test]
    fn closing_the_last_window_while_fullscreen_leaves_an_empty_leaf_that_is_not_fullscreen() {
        let mut system = BspLayoutSystem::default();
        let layout = system.create_layout();
        system.add_window_after_selection(layout, w(1));
        system.toggle_fullscreen_of_selection(layout);
        system.remove_window(w(1));
        assert_eq!(system.draw_tree(layout), "Leaf None\n");
        let tree = system.container_tree(layout);
        assert_eq!(tree.node_type, rift_protocol::ContainerNodeType::Placeholder);
        assert!(
            !tree.is_fullscreen && !tree.is_fullscreen_within_gaps,
            "{tree:?}"
        );
    }

    #[test]
    fn swapping_a_stacked_window_with_itself_keeps_it_once_on_top() {
        let (mut system, layout) = tiled(&[1, 2, 3]);
        assert!(system.select_window(layout, w(2)));
        system.join_selection_with_direction(layout, Direction::Down);
        assert_eq!(system.stack_members(layout, w(2)), vec![w(3), w(2)]);
        assert!(system.swap_windows(layout, w(3), w(3)));
        assert_eq!(system.stack_members(layout, w(3)), vec![w(2), w(3)]);
        assert_eq!(system.selected_window(layout), Some(w(3)));
    }

    #[test]
    fn removal_clears_legacy_duplicates_and_empty_roots() {
        let (mut system, layout) = tiled(&[1, 2, 3]);
        // Simulate duplicate membership from an older saved tree.
        let duplicate = system.node_for_window_in_layout(layout, w(2)).unwrap();
        if let Some(NodeKind::Leaf { window, .. }) = system.kind.get_mut(duplicate) {
            *window = Some(w(1));
        }
        system.resync_core();
        system.remove_window(w(1));
        assert_eq!(system.all_windows_in_layout(layout), vec![w(3)]);
        let frames =
            calculate_frames(&system, layout, CGSize::new(1000.0, 800.0), &Default::default());
        assert_frame(&frames, w(3), (0.0, 0.0, 1000.0, 800.0));
        let stack_layout = system.clone_layout(layout);
        let node = system.node_for_window_in_layout(stack_layout, w(3)).unwrap();
        system.stacks.insert(node, vec![w(3), w(3)]);
        system.resync_core();
        system.remove_window(w(3));
        system.remove_window(w(3));
        assert!(system.all_windows_in_layout(layout).is_empty());
        assert_eq!(system.selected_window(layout), None);
        assert!(system.all_windows_in_layout(stack_layout).is_empty());
        assert!(system.visible_windows_in_layout(stack_layout).is_empty());
    }

    #[test]
    fn cloned_stacks_replace_and_remove_all_members() {
        use crate::layout_engine::WindowDropAction::Stack;

        let (mut system, original) = tiled(&[1, 2, 3]);
        assert!(system.apply_window_drop(original, w(2), w(1), Stack));
        let cloned = system.clone_layout(original);
        system.replace_window(w(1), w(4));
        system.replace_window(w(2), w(5));
        for layout in [original, cloned] {
            assert_eq!(system.stack_members(layout, w(4)), vec![w(4), w(5)]);
            assert_eq!(system.visible_windows_in_layout(layout), vec![w(5), w(3)]);
        }
        system.remove_window(w(5));
        for layout in [original, cloned] {
            assert_eq!(system.stack_members(layout, w(4)), vec![w(4)]);
            assert_eq!(system.visible_windows_in_layout(layout), vec![w(4), w(3)]);
        }
        system.replace_window(w(4), w(3));
        for layout in [original, cloned] {
            assert_eq!(system.all_windows_in_layout(layout), vec![w(3)]);
        }
        system.remove_windows_for_app(1);
        for layout in [original, cloned] {
            assert!(system.all_windows_in_layout(layout).is_empty());
            assert!(system.visible_windows_in_layout(layout).is_empty());
            assert_eq!(system.selected_window(layout), None);
        }
    }

    #[test]
    fn interactions_are_scoped_and_destroying_a_clone_preserves_original() {
        use crate::layout_engine::WindowDropAction::{Insert, Stack};

        let (mut system, original) = tiled(&[1, 2]);
        let cloned = system.clone_layout(original);
        assert!(system.swap_windows(original, w(1), w(2)));
        assert_eq!(system.all_windows_in_layout(original), vec![w(2), w(1)]);
        assert_eq!(system.all_windows_in_layout(cloned), vec![w(1), w(2)]);
        assert!(system.apply_window_drop(original, w(1), w(2), Stack));
        assert!(system.select_window(original, w(2)));
        assert_eq!(system.selected_window(original), Some(w(2)));
        assert_eq!(system.selected_window(cloned), Some(w(2)));
        assert!(system.select_window(cloned, w(1)));
        assert_eq!(system.selected_window(original), Some(w(2)));
        assert!(system.apply_window_drop(original, w(2), w(1), Insert(Direction::Left)));
        assert_eq!(system.all_windows_in_layout(original), vec![w(2), w(1)]);
        system.remove_layout(cloned);
        assert!(system.select_window(original, w(1)));
        system.remove_window(w(2));
        assert_eq!(system.all_windows_in_layout(original), vec![w(1)]);
    }

    #[test]
    fn insertion_preserves_stacks_and_rejects_existing_members() {
        use crate::layout_engine::WindowDropAction::Stack;

        for (insertion, explicit_split) in [
            (WindowInsertionPoint::default(), false),
            (WindowInsertionPoint::default(), true),
            (WindowInsertionPoint::EndOfTree, false),
        ] {
            let (mut system, layout) = tiled(&[1, 2]);
            system.set_window_insertion_point(insertion);
            assert!(system.apply_window_drop(layout, w(2), w(1), Stack));
            system.add_window_after_selection(layout, w(1));
            system.add_window_after_selection(layout, w(2));
            assert_eq!(system.all_windows_in_layout(layout), vec![w(1), w(2)]);
            if explicit_split {
                system.split_selection(layout, LayoutKind::Horizontal);
            }
            system.add_window_after_selection(layout, w(3));
            assert_eq!(system.all_windows_in_layout(layout), vec![w(1), w(2), w(3)]);
            assert_eq!(system.stack_members(layout, w(1)), vec![w(1), w(2)]);
            system.remove_windows_for_app(1);
            assert!(system.all_windows_in_layout(layout).is_empty());
        }
    }

    #[test]
    fn joining_preserves_membership_for_cloning_and_app_exit() {
        let (mut system, layout) = tiled(&[1, 2, 3]);
        system.join_selection_with_direction(layout, Direction::Up);
        assert_eq!(system.all_windows_in_layout(layout), vec![w(1), w(2), w(3)]);
        assert_eq!(system.stack_members(layout, w(2)), vec![w(2), w(3)]);
        let cloned = system.clone_layout(layout);
        system.remove_windows_for_app(1);
        for layout in [layout, cloned] {
            assert!(system.all_windows_in_layout(layout).is_empty());
        }
    }

    #[test]
    fn unjoining_retiles_the_stack_in_place() {
        for (windows, direction, joined) in [
            // Stack directly under the root: [1 | stack(2, 3)].
            (&[1, 2, 3][..], Direction::Up, [2, 3]),
            // Deeper stack in the spiral: [1 | [2 / stack(3, 4)]].
            (&[1, 2, 3, 4][..], Direction::Left, [3, 4]),
        ] {
            let (mut system, layout) = tiled(windows);
            let tiled_tree = system.draw_tree(layout);
            let focused = w(joined[1]);

            system.join_selection_with_direction(layout, direction);

            assert_eq!(system.stack_members(layout, focused), joined.map(w));

            system.unjoin_selection(layout);

            assert_eq!(system.draw_tree(layout), tiled_tree);
            assert!(system.stack_members(layout, focused).is_empty());
            assert_eq!(system.selected_window(layout), Some(focused));
        }

        // Members keep their stack order and the active member keeps the selection.
        let (mut system, layout) = tiled(&[1, 2, 3]);
        system.join_selection_with_direction(layout, Direction::Up);
        assert!(system.select_window(layout, w(2)));

        system.unjoin_selection(layout);

        assert_eq!(system.visible_windows_in_layout(layout), vec![w(1), w(3), w(2)]);
        assert_eq!(system.selected_window(layout), Some(w(2)));
    }

    #[test]
    fn unjoining_a_window_that_is_not_stacked_is_a_noop() {
        for windows in [&[1][..], &[1, 2], &[1, 2, 3], &[1, 2, 3, 4]] {
            let (mut system, layout) = tiled(windows);
            let tree = system.draw_tree(layout);
            let selected = system.selected_window(layout);

            system.unjoin_selection(layout);

            assert_eq!(system.draw_tree(layout), tree);
            assert_eq!(system.selected_window(layout), selected);
        }
    }

    #[test]
    fn unjoining_keeps_fullscreen_on_the_visible_member() {
        let toggles: [fn(&mut BspLayoutSystem, LayoutId) -> Vec<WindowId>; 2] = [
            BspLayoutSystem::toggle_fullscreen_of_selection,
            BspLayoutSystem::toggle_fullscreen_within_gaps_of_selection,
        ];

        for toggle in toggles {
            let (mut system, layout) = tiled(&[1, 2, 3]);
            system.join_selection_with_direction(layout, Direction::Up);
            toggle(&mut system, layout);

            system.unjoin_selection(layout);

            assert_eq!(system.selected_window(layout), Some(w(3)));
            assert!(system.has_any_fullscreen_node(layout));

            let frames =
                calculate_frames(&system, layout, CGSize::new(1000.0, 800.0), &Default::default());

            assert_frame(&frames, w(2), (500.0, 0.0, 500.0, 400.0));
            assert_frame(&frames, w(3), (0.0, 0.0, 1000.0, 800.0));

            toggle(&mut system, layout);

            assert!(!system.has_any_fullscreen_node(layout));
        }
    }

    #[test]
    fn consume_or_expel_toggles_between_joined_and_unjoined() {
        for (windows, direction, joined) in [
            (&[1, 2, 3][..], Direction::Up, [2, 3]),
            (&[1, 2, 3, 4][..], Direction::Left, [3, 4]),
        ] {
            let (mut system, layout) = tiled(windows);
            let tiled_tree = system.draw_tree(layout);

            system.consume_or_expel_selection(layout, direction);

            assert_eq!(system.stack_members(layout, w(joined[0])), joined.map(w));

            system.consume_or_expel_selection(layout, direction);

            assert_eq!(system.draw_tree(layout), tiled_tree);
        }

        // A joined first child keeps its side: [[2 / 5] / [3 | 4]].
        let (mut system, layout) = tiled(&[1, 2, 3, 4]);
        system.remove_window(w(1));
        assert!(system.select_window(layout, w(2)));
        system.add_window_after_selection(layout, w(5));
        assert!(system.select_window(layout, w(5)));
        let tiled_tree = system.draw_tree(layout);

        system.consume_or_expel_selection(layout, Direction::Up);

        assert_eq!(system.stack_members(layout, w(5)), vec![w(2), w(5)]);
        assert_eq!(system.visible_windows_in_layout(layout), [5, 3, 4].map(w));

        system.consume_or_expel_selection(layout, Direction::Up);

        assert_eq!(system.draw_tree(layout), tiled_tree);
        assert_eq!(system.visible_windows_in_layout(layout), [2, 5, 3, 4].map(w));
    }

    #[test]
    fn consume_or_expel_joins_a_window_left_alone_in_its_stack() {
        // [1 | [2 / stack(3)]] once the other stacked window has closed.
        let (mut system, layout) = tiled(&[1, 2, 3, 4]);
        system.join_selection_with_direction(layout, Direction::Left);
        system.remove_window(w(4));

        assert_eq!(system.stack_members(layout, w(3)), vec![w(3)]);

        system.consume_or_expel_selection(layout, Direction::Up);

        assert_eq!(system.stack_members(layout, w(3)), vec![w(2), w(3)]);
    }

    #[test]
    fn unjoining_a_window_left_alone_in_its_stack_clears_the_stack() {
        // [1 | [2 / stack(3)]] once the other stacked window has closed.
        let (mut system, layout) = tiled(&[1, 2, 3, 4]);
        system.join_selection_with_direction(layout, Direction::Left);
        system.remove_window(w(4));
        let tree = system.draw_tree(layout);

        assert_eq!(system.stack_members(layout, w(3)), vec![w(3)]);

        system.unjoin_selection(layout);

        assert_eq!(system.draw_tree(layout), tree);
        assert!(system.stack_members(layout, w(3)).is_empty());
        assert_eq!(system.selected_window(layout), Some(w(3)));
    }

    #[test]
    fn joining_keeps_the_selected_window_visible() {
        let (mut system, layout) = tiled(&[1, 2, 3]);
        assert!(system.select_window(layout, w(2)));

        system.join_selection_with_direction(layout, Direction::Down);

        assert_eq!(system.stack_members(layout, w(2)), vec![w(3), w(2)]);
        assert_eq!(system.visible_windows_in_layout(layout), vec![w(1), w(2)]);
        assert_eq!(system.selected_window(layout), Some(w(2)));

        system.unjoin_selection(layout);

        let (reference, reference_layout) = tiled(&[1, 3, 2]);

        assert_eq!(system.draw_tree(layout), reference.draw_tree(reference_layout));
        assert_eq!(system.selected_window(layout), Some(w(2)));
    }

    #[test]
    fn legacy_ron_index_is_ignored_when_loading_saved_layouts() {
        let (system, original) = tiled(&[1, 2]);
        let node = system.node_for_window_in_layout(original, w(1)).unwrap();
        let obsolete_index: HashMap<_, _> = [(w(1), node)].into_iter().collect();
        let saved = ron::to_string(&system).unwrap();
        let legacy = saved.replacen(
            "stacks:",
            &format!(
                "window_to_node:{},stacks:",
                ron::to_string(&obsolete_index).unwrap()
            ),
            1,
        );
        let mut restored: BspLayoutSystem = ron::from_str(&legacy).unwrap();
        assert_eq!(restored.all_windows_in_layout(original), vec![w(1), w(2)]);
        restored.remove_window(w(1));
        assert_eq!(restored.all_windows_in_layout(original), vec![w(2)]);
    }

    #[test]
    fn a_saved_layout_parses_into_the_cores_tree_or_says_why_not() {
        let (system, layout) = tiled(&[1, 2, 3]);
        let saved = ron::to_string(&system).unwrap();
        let restored: BspLayoutSystem = ron::from_str(&saved).unwrap();
        let core = restored.core_layouts().expect("a saved layout parses");
        let projection = core.projection(layout).expect("the layout");
        assert_eq!(projection.windows, vec![w(1), w(2), w(3)]);
        assert_eq!(projection.selected_window, restored.selected_window(layout));
        assert_eq!(core.layout(layout).unwrap().draw(), restored.draw_tree(layout));
        // A stack on a leaf showing nothing: refused, not a panic mid-restore.
        let (mut broken, layout) = tiled(&[]);
        let root = broken.layouts[layout].root;
        broken.stacks.insert(root, vec![w(1)]);
        assert_eq!(broken.core_layouts().err(), Some(OldTree::StackShowsNothing));
    }

    #[test]
    fn insertion_alternates_splits_and_navigation_follows_orientation() {
        let (mut system, layout) = tiled(&[1, 2]);
        assert_eq!(system.window_in_direction(layout, Direction::Right), Some(w(1)));
        assert_eq!(system.window_in_direction(layout, Direction::Left), Some(w(2)));
        system.toggle_tile_orientation(layout);
        assert_eq!(system.window_in_direction(layout, Direction::Down), Some(w(1)));
        assert_eq!(system.window_in_direction(layout, Direction::Up), Some(w(2)));

        let (mut spiral, layout) = tiled(&[1]);
        for (window, horizontal, vertical) in [(2, 1, 0), (3, 1, 1), (4, 2, 1), (5, 2, 2)] {
            spiral.add_window_after_selection(layout, w(window));
            let tree = spiral.draw_tree(layout);
            assert_eq!(tree.matches("Horizontal").count(), horizontal);
            assert_eq!(tree.matches("Vertical").count(), vertical);
        }
    }

    #[test]
    fn rebalance_splits_each_axis_evenly() {
        let size = CGSize::new(1200.0, 900.0);

        let (mut system, layout) = tiled(&[1, 2, 3]);
        assert!(system.select_window(layout, w(1)));
        system.resize_selection_by(layout, -0.4, ResizeOrientation::Smart);

        assert!(system.select_window(layout, w(2)));
        system.resize_selection_by(layout, -0.3, ResizeOrientation::Vertical);

        system.rebalance(layout);

        let frames = calculate_frames(&system, layout, size, &Default::default());
        assert_frame(&frames, w(1), (0.0, 0.0, 600.0, 900.0));
        assert_frame(&frames, w(2), (600.0, 0.0, 600.0, 450.0));
        assert_frame(&frames, w(3), (600.0, 450.0, 600.0, 450.0));
    }

    #[test]
    fn rebalance_shares_an_axis_across_nested_splits() {
        let size = CGSize::new(1200.0, 900.0);

        let (mut system, layout) = tiled(&[1]);
        system.split_selection(layout, LayoutKind::Horizontal);
        system.add_window_after_selection(layout, w(2));
        system.split_selection(layout, LayoutKind::Horizontal);
        system.add_window_after_selection(layout, w(3));

        let frames = calculate_frames(&system, layout, size, &Default::default());
        assert_frame(&frames, w(1), (0.0, 0.0, 600.0, 900.0));

        system.rebalance(layout);

        let frames = calculate_frames(&system, layout, size, &Default::default());
        for window in [1, 2, 3] {
            assert!((frames[&w(window)].size.width - 400.0).abs() < 1.0);
        }
    }

    #[test]
    fn rebalance_counts_a_stacked_leaf_as_one_tile() {
        let size = CGSize::new(1200.0, 900.0);

        let (mut system, layout) = tiled(&[1]);
        system.split_selection(layout, LayoutKind::Horizontal);
        system.add_window_after_selection(layout, w(2));
        system.split_selection(layout, LayoutKind::Horizontal);
        system.add_window_after_selection(layout, w(3));
        system.join_selection_with_direction(layout, Direction::Left);

        assert_eq!(system.stack_members(layout, w(2)), vec![w(2), w(3)]);

        assert!(system.select_window(layout, w(1)));
        system.resize_selection_by(layout, -0.4, ResizeOrientation::Smart);

        system.rebalance(layout);

        let frames = calculate_frames(&system, layout, size, &Default::default());
        assert_frame(&frames, w(1), (0.0, 0.0, 600.0, 900.0));
    }

    #[test]
    fn focus_into_perpendicular_splits_tracks_the_source_center() {
        for vertical in [false, true] {
            for (left_resize, right_resize, targets) in [
                (0.0, 0.0, [2, 4, 1, 3]),
                (-0.5, 0.5, [2, 2, 3, 3]),
                (0.5, -0.5, [4, 4, 1, 1]),
            ] {
                let (mut system, layout) = tiled(&[1, 2]);
                if vertical {
                    system.toggle_tile_orientation(layout);
                }
                for (window, new, resize) in [(1, 3, left_resize), (2, 4, right_resize)] {
                    assert!(system.select_window(layout, w(window)));
                    system.add_window_after_selection(layout, w(new));
                    if vertical {
                        system.toggle_tile_orientation(layout);
                    }
                    assert!(system.select_window(layout, w(window)));
                    system.resize_selection_by(layout, resize, ResizeOrientation::Smart);
                }
                let (forward, backward) = if vertical {
                    (Direction::Down, Direction::Up)
                } else {
                    (Direction::Right, Direction::Left)
                };
                for ((from, direction), to) in
                    [(1, forward), (3, forward), (2, backward), (4, backward)]
                        .into_iter()
                        .zip(targets)
                {
                    assert!(system.select_window(layout, w(from)));
                    assert_eq!(system.move_focus(layout, direction), (Some(w(to)), vec![w(to)]));
                }
            }
        }
    }

    #[test]
    fn focus_raises_only_its_target_and_layout_membership_is_independent() {
        let (mut system, first) = tiled(&[1, 2, 3]);
        assert_eq!(
            system.move_focus(first, Direction::Left),
            (Some(w(1)), vec![w(1)])
        );
        let second = system.create_layout();
        system.add_window_after_selection(second, w(1));
        system.set_windows_for_app(first, w(1).pid, vec![w(1)]);
        assert_eq!(system.all_windows_in_layout(first), vec![w(1)]);
        assert_eq!(system.all_windows_in_layout(second), vec![w(1)]);
    }

    #[test]
    fn directional_moves_retile_in_all_axes_and_stop_at_the_edge() {
        for direction in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ] {
            let vertical = matches!(direction, Direction::Up | Direction::Down);
            let backwards = matches!(direction, Direction::Left | Direction::Up);
            let (mut system, layout) = tiled(&[1, 2]);
            if vertical {
                system.toggle_tile_orientation(layout);
            }
            system.add_window_after_selection(layout, w(3));
            if vertical {
                system.toggle_tile_orientation(layout);
            }

            let moving = if backwards { w(3) } else { w(1) };
            assert!(system.select_window(layout, moving));
            assert!(system.move_selection(layout, direction));
            assert_eq!(system.selected_window(layout), Some(moving));

            let expected = if backwards {
                [
                    (250.0, 0.0, 250.0, 1000.0),
                    (500.0, 0.0, 500.0, 1000.0),
                    (0.0, 0.0, 250.0, 1000.0),
                ]
            } else {
                [
                    (500.0, 0.0, 500.0, 500.0),
                    (0.0, 0.0, 500.0, 500.0),
                    (0.0, 500.0, 1000.0, 500.0),
                ]
            };
            let frames =
                calculate_frames(&system, layout, CGSize::new(1000.0, 1000.0), &Default::default());
            for (index, (x, y, width, height)) in expected.into_iter().enumerate() {
                assert_frame(
                    &frames,
                    w(index as u32 + 1),
                    if vertical {
                        (y, x, height, width)
                    } else {
                        (x, y, width, height)
                    },
                );
            }
            assert!(!system.move_selection(layout, direction));
            assert_eq!(system.selected_window(layout), Some(moving));
        }
    }

    #[test]
    fn directional_move_fills_an_empty_slot_and_collapses_the_old_split() {
        let (mut system, layout) = tiled(&[1, 2, 3]);
        assert!(system.select_window(layout, w(1)));
        system.split_selection(layout, LayoutKind::Horizontal);
        assert!(!system.move_selection(layout, Direction::Right));
        assert!(system.select_window(layout, w(3)));
        assert!(system.move_selection(layout, Direction::Left));
        assert_eq!(system.selected_window(layout), Some(w(3)));

        let frames =
            calculate_frames(&system, layout, CGSize::new(1000.0, 1000.0), &Default::default());
        assert_eq!(frames.len(), 3);
        for (window, x, width) in [
            (w(1), 0.0, 250.0),
            (w(3), 250.0, 250.0),
            (w(2), 500.0, 500.0),
        ] {
            assert_frame(&frames, window, (x, 0.0, width, 1000.0));
        }
        assert!(system.select_window(layout, w(3)));
        system.split_selection(layout, LayoutKind::Horizontal);
        let before =
            calculate_frames(&system, layout, CGSize::new(1000.0, 1000.0), &Default::default());
        system.set_window_participation(w(3), false);
        let collapsed =
            calculate_frames(&system, layout, CGSize::new(1000.0, 1000.0), &Default::default());
        assert_eq!(collapsed.len(), 2);
        assert_frame(&collapsed, w(1), (0.0, 0.0, 500.0, 1000.0));
        assert_frame(&collapsed, w(2), (500.0, 0.0, 500.0, 1000.0));
        system.set_window_participation(w(3), true);
        assert_eq!(
            calculate_frames(&system, layout, CGSize::new(1000.0, 1000.0), &Default::default()),
            before
        );
    }

    #[test]
    fn moving_a_stack_keeps_its_membership_and_fullscreen_state() {
        use crate::layout_engine::WindowDropAction::Stack;

        for (windows, source) in [(&[1, 2, 3][..], w(3)), (&[1, 2][..], w(2))] {
            let (mut system, layout) = tiled(windows);
            for window in [4, 5] {
                system.add_window_after_selection(layout, w(window));
            }
            assert!(system.apply_window_drop(layout, w(4), source, Stack));
            assert!(system.apply_window_drop(layout, w(5), w(1), Stack));
            assert!(system.select_window(layout, w(4)));
            system.toggle_fullscreen_of_selection(layout);
            assert!(system.move_selection(layout, Direction::Left));

            assert_eq!(system.stack_members(layout, w(4)), vec![source, w(4)]);
            assert_eq!(system.stack_members(layout, w(5)), vec![w(1), w(5)]);
            assert!(system.has_any_fullscreen_node(layout));
            system.toggle_fullscreen_of_selection(layout);
            assert!(!system.has_any_fullscreen_node(layout));
            assert!(system.select_window(layout, w(4)));
            system.set_window_participation(w(4), false);
            assert_eq!(system.selected_window(layout), Some(source));
            assert_eq!(system.toggle_fullscreen_of_selection(layout), vec![source]);
            assert_eq!(system.stack_members(layout, source), vec![source, w(4)]);
            system.set_window_participation(w(4), true);
            assert_eq!(system.toggle_fullscreen_of_selection(layout), vec![w(4)]);
            for window in system.all_windows_in_layout(layout) {
                system.remove_window(window);
            }
            assert!(system.all_windows_in_layout(layout).is_empty());
        }
    }

    #[test]
    fn split_constraints_stay_local_and_nonbinding_minima_keep_equal_halves() {
        let (system, layout) = tiled(&[101, 102]);
        let mut constraints = HashMap::default();
        constraints.insert(
            w(101),
            WindowLayoutConstraints {
                sizing: Sizing::new(true, false, Size::reported(0.0, 0.0)),
                max_width: 600.0,
                ..Default::default()
            }
            .normalized(),
        );
        let frames = calculate_frames(&system, layout, CGSize::new(1600.0, 900.0), &constraints);
        assert!((frames[&w(101)].size.width - 600.0).abs() < 1.0);
        assert!((frames[&w(102)].size.width - 1000.0).abs() < 1.0);
        assert!((frames[&w(102)].origin.x - 600.0).abs() < 1.0);

        let (system, layout) = tiled(&[106, 107]);
        constraints.clear();
        constraints.insert(
            w(107),
            WindowLayoutConstraints {
                sizing: Sizing::new(true, false, Size::reported(400.0, 0.0)),
                ..Default::default()
            }
            .normalized(),
        );
        let frames = calculate_frames(&system, layout, CGSize::new(1200.0, 900.0), &constraints);
        assert!((frames[&w(106)].size.width - 600.0).abs() < 1.0);
        assert!((frames[&w(107)].size.width - 600.0).abs() < 1.0);
        assert!((frames[&w(107)].origin.x - 600.0).abs() < 1.0);

        let (mut system, layout) = tiled(&[103]);
        system.split_selection(layout, LayoutKind::Vertical);
        system.add_window_after_selection(layout, w(105));
        assert!(system.select_window(layout, w(103)));
        system.split_selection(layout, LayoutKind::Horizontal);
        system.add_window_after_selection(layout, w(104));
        constraints.clear();
        constraints.insert(
            w(103),
            WindowLayoutConstraints {
                sizing: Sizing::new(true, false, Size::reported(0.0, 0.0)),
                max_height: 200.0,
                ..Default::default()
            }
            .normalized(),
        );
        let frames = calculate_frames(&system, layout, CGSize::new(1200.0, 800.0), &constraints);
        assert!(frames[&w(103)].size.height <= 201.0);
        assert!(frames[&w(104)].size.height >= 399.0);
        assert!((frames[&w(105)].size.height - 400.0).abs() < 1.0);
    }
}

/// The bodies of the operations and queries as they act on rift's NodeId trees (see
/// `operate`): the core is held to them in test builds.
impl BspLayoutSystem {
    fn calculate_layout_from_tree(
        &self,
        layout: LayoutId,
        screen: CGRect,
        _stack_offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &crate::common::config::GapSettings,
        _stack_line: crate::common::config::StackLineReserve,
    ) -> Vec<(WindowId, CGRect)> {
        let Some(state) = self.layouts.get(layout).copied() else {
            return Vec::new();
        };
        // Nothing in the layout takes space: no shape, no frames.
        let Some(root) = self.shape(state.root, constraints) else {
            return Vec::new();
        };
        let shape = bsp::Bsp {
            screen: crate::layout_engine::core_rect(screen),
            gaps: crate::layout_engine::core_gaps(gaps),
            root,
            aspect: self.single_window_aspect_ratio,
        };
        crate::layout_engine::tile_bsp(&shape)
    }

    fn contains_layout_from_tree(&self, layout: LayoutId) -> bool {
        self.layouts.contains_key(layout)
    }

    fn draw_tree_from_tree(&self, layout: LayoutId) -> String {
        fn write_node(this: &BspLayoutSystem, node: NodeId, out: &mut String, indent: usize) {
            for _ in 0..indent {
                out.push_str("  ");
            }
            match this.kind.get(node) {
                Some(NodeKind::Leaf { window, .. }) => {
                    out.push_str(&format!("Leaf {:?}\n", window));
                }
                Some(NodeKind::Split { orientation, ratio }) => {
                    out.push_str(&format!("Split {:?} {:.2}\n", orientation, ratio));
                    let mut it = node.children(&this.tree.map);
                    if let Some(first) = it.next() {
                        write_node(this, first, out, indent + 1);
                    }
                    if let Some(second) = it.next() {
                        write_node(this, second, out, indent + 1);
                    }
                }
                None => {}
            }
        }
        if let Some(state) = self.layouts.get(layout).copied() {
            let mut s = String::new();
            write_node(self, state.root, &mut s, 0);
            s
        } else {
            "<empty bsp>".to_string()
        }
    }

    fn container_tree_from_tree(&self, layout: LayoutId) -> rift_protocol::ContainerTreeNode {
        fn snapshot(
            system: &BspLayoutSystem,
            node: NodeId,
            selected: NodeId,
        ) -> rift_protocol::ContainerTreeNode {
            let weight = node.parent(&system.tree.map).and_then(|parent| {
                let NodeKind::Split { ratio, .. } = system.kind.get(parent)? else {
                    return None;
                };
                let first = parent.first_child(&system.tree.map);
                Some(if first == Some(node) {
                    f64::from(*ratio)
                } else {
                    1.0 - f64::from(*ratio)
                })
            });

            match system.kind.get(node) {
                Some(NodeKind::Split { orientation, .. }) => rift_protocol::ContainerTreeNode {
                    node_id: node.data().as_ffi(),
                    node_type: rift_protocol::ContainerNodeType::Container,
                    frame: Default::default(),
                    layout_kind: Some(rift_protocol::LayoutKind::from(*orientation)),
                    weight,
                    window_id: None,
                    is_selected: node == selected,
                    is_fullscreen: false,
                    is_fullscreen_within_gaps: false,
                    role: None,
                    pending_split: None,
                    children: node
                        .children(&system.tree.map)
                        .map(|child| snapshot(system, child, selected))
                        .collect(),
                },
                Some(NodeKind::Leaf {
                    window,
                    fullscreen,
                    fullscreen_within_gaps,
                    preselected,
                }) => {
                    let members = system.stacks.get(&node);
                    let stacked = members.is_some();
                    rift_protocol::ContainerTreeNode {
                        node_id: node.data().as_ffi(),
                        node_type: if stacked {
                            rift_protocol::ContainerNodeType::Container
                        } else if window.is_some() {
                            rift_protocol::ContainerNodeType::Window
                        } else {
                            rift_protocol::ContainerNodeType::Placeholder
                        },
                        frame: Default::default(),
                        layout_kind: stacked.then_some(rift_protocol::LayoutKind::HorizontalStack),
                        weight,
                        window_id: if stacked {
                            None
                        } else {
                            window.map(Into::into)
                        },
                        is_selected: node == selected,
                        is_fullscreen: *fullscreen,
                        is_fullscreen_within_gaps: *fullscreen_within_gaps,
                        role: None,
                        pending_split: preselected.map(Into::into),
                        children: members.map_or_else(Vec::new, |members| {
                            members
                                .iter()
                                .map(|member| rift_protocol::ContainerTreeNode {
                                    node_id: (u64::from(member.pid as u32) << 32)
                                        | u64::from(member.idx.get()),
                                    node_type: rift_protocol::ContainerNodeType::Window,
                                    frame: Default::default(),
                                    layout_kind: None,
                                    weight: None,
                                    window_id: Some((*member).into()),
                                    is_selected: node == selected && *window == Some(*member),
                                    is_fullscreen: *fullscreen,
                                    is_fullscreen_within_gaps: *fullscreen_within_gaps,
                                    role: None,
                                    pending_split: None,
                                    children: Vec::new(),
                                })
                                .collect()
                        }),
                    }
                }
                None => unreachable!("BSP layout contains a node without metadata"),
            }
        }

        let state = self.layouts.get(layout).expect("unknown BSP layout");
        let selected = self.tree.data.selection.current_selection(state.root);
        snapshot(self, state.root, selected)
    }

    fn selected_window_from_tree(&self, layout: LayoutId) -> Option<WindowId> {
        self.layouts.get(layout).and_then(|s| self.selection_window(s))
    }

    fn all_windows_in_layout_from_tree(&self, layout: LayoutId) -> Vec<WindowId> {
        let mut out = Vec::new();
        if let Some(state) = self.layouts.get(layout).copied() {
            self.collect_windows_under(state.root, &mut out);
        }
        out
    }

    fn window_slot_from_tree(&self, layout: LayoutId, window: WindowId) -> Option<Vec<usize>> {
        let root = self.layouts.get(layout)?.root;
        let node = self.node_for_window_in_layout(layout, window)?;
        let mut slot = crate::layout_engine::systems::node_slot(node, root, &self.tree.map)?;
        if let Some(members) = self.stacks.get(&node) {
            slot.push(members.iter().position(|&member| member == window)?);
        }
        Some(slot)
    }

    fn visible_windows_in_layout_from_tree(&self, layout: LayoutId) -> Vec<WindowId> {
        let mut out = Vec::new();
        if let Some(state) = self.layouts.get(layout).copied() {
            self.collect_visible_windows_under(state.root, &mut out);
        }
        out
    }

    fn stack_members_from_tree(&self, layout: LayoutId, window: WindowId) -> Vec<WindowId> {
        if !self.contains_window(layout, window) {
            return Vec::new();
        }
        self.node_for_window_in_layout(layout, window)
            .and_then(|node| self.stacks.get(&node))
            .cloned()
            .unwrap_or_default()
    }

    fn visible_windows_under_selection_from_tree(&self, layout: LayoutId) -> Vec<WindowId> {
        let mut out = Vec::new();
        if let Some(sel) = self.selection_of_layout(layout) {
            if self.kind.get(sel).is_some() {
                let leaf = self.descend_to_leaf(sel);
                self.collect_visible_windows_under(leaf, &mut out);
            }
        }

        out
    }

    fn window_in_direction_from_tree(
        &self,
        layout: LayoutId,
        direction: Direction,
    ) -> Option<WindowId> {
        self.layouts
            .get(layout)
            .and_then(|state| self.window_in_direction_from(state.root, direction))
    }

    fn contains_window_from_tree(&self, layout: LayoutId, wid: WindowId) -> bool {
        self.node_for_window_in_layout(layout, wid).is_some()
    }

    fn has_any_fullscreen_node_from_tree(&self, layout: LayoutId) -> bool {
        if let Some(state) = self.layouts.get(layout).copied() {
            self.has_fullscreen_in_subtree(state.root)
        } else {
            false
        }
    }

    fn clone_layout_unshadowed(&mut self, layout: LayoutId) -> LayoutId {
        let mut groups = Vec::new();
        if let Some(state) = self.layouts.get(layout).copied() {
            for node in state.root.traverse_preorder(&self.tree.map) {
                if let Some(NodeKind::Leaf { window: Some(window), .. }) = self.kind.get(node) {
                    groups.push(self.stacks.get(&node).cloned().unwrap_or_else(|| vec![*window]));
                }
            }
        }
        let new_layout = self.create_layout();
        for group in groups {
            let mut members = group.into_iter();
            let Some(first) = members.next() else { continue };
            self.add_window_after_selection(new_layout, first);
            let mut target = first;
            for window in members {
                self.add_window_after_selection(new_layout, window);
                let _ = self.apply_window_drop(
                    new_layout,
                    window,
                    target,
                    crate::layout_engine::WindowDropAction::Stack,
                );
                target = window;
            }
        }
        new_layout
    }

    fn on_window_resized_unshadowed(
        &mut self,
        layout: LayoutId,
        wid: WindowId,
        old_frame: CGRect,
        new_frame: CGRect,
        screen: CGRect,
        gaps: &crate::common::config::GapSettings,
    ) {
        if !self.window_participates(wid) {
            return;
        }
        if let Some(node) = self.node_for_window_in_layout(layout, wid) {
            let tiling = compute_tiling_area(screen, gaps);
            let mut fullscreen_transition = false;
            if let Some(NodeKind::Leaf {
                window: _,
                fullscreen,
                fullscreen_within_gaps,
                ..
            }) = self.kind.get_mut(node)
            {
                if new_frame == screen {
                    *fullscreen = true;
                    *fullscreen_within_gaps = false;
                    fullscreen_transition = true;
                } else if old_frame == screen {
                    *fullscreen = false;
                    fullscreen_transition = true;
                } else {
                    if new_frame == tiling {
                        *fullscreen_within_gaps = true;
                        *fullscreen = false;
                        fullscreen_transition = true;
                    } else if old_frame == tiling {
                        *fullscreen_within_gaps = false;
                        fullscreen_transition = true;
                    }
                }
            }

            if fullscreen_transition {
                return;
            }

            let width_changed = (new_frame.size.width - old_frame.size.width).abs() > 0.5;
            let height_changed = (new_frame.size.height - old_frame.size.height).abs() > 0.5;
            if !width_changed && !height_changed {
                return;
            }

            let mut current = node;
            while let Some(parent) = current.parent(&self.tree.map) {
                let Some(NodeKind::Split { orientation, ratio }) = self.kind.get_mut(parent) else {
                    current = parent;
                    continue;
                };

                let (old_len, new_len) = match orientation {
                    Orientation::Horizontal if width_changed => {
                        (old_frame.size.width, new_frame.size.width)
                    }
                    Orientation::Vertical if height_changed => {
                        (old_frame.size.height, new_frame.size.height)
                    }
                    _ => {
                        current = parent;
                        continue;
                    }
                };

                let current_ratio = f64::from(*ratio);
                let is_first = Some(current) == parent.first_child(&self.tree.map);
                let denom = if is_first {
                    current_ratio
                } else {
                    1.0 - current_ratio
                };
                if old_len <= 0.0 || denom <= f64::EPSILON {
                    break;
                }

                let parent_len = old_len / denom;
                if parent_len <= 0.0 {
                    break;
                }

                let raw_ratio = (new_len / parent_len).clamp(0.05, 0.95);
                *ratio = if is_first {
                    raw_ratio as f32
                } else {
                    (1.0 - raw_ratio) as f32
                };
                break;
            }
        }
    }

    fn set_windows_for_app_unshadowed(
        &mut self,
        layout: LayoutId,
        pid: pid_t,
        desired: Vec<WindowId>,
    ) {
        let current = if let Some(state) = self.layouts.get(layout).copied() {
            let mut under: Vec<WindowId> = Vec::new();
            self.collect_windows_under(state.root, &mut under);
            under.into_iter().filter(|w| w.pid == pid).collect()
        } else {
            Vec::new()
        };
        let delta =
            reconcile_app_membership(pid, current, desired, |w| self.window_participates(w));
        for wid in delta.removals {
            if let Some(node) = self.node_for_window_in_layout(layout, wid)
                && let Some(NodeKind::Leaf {
                    fullscreen,
                    fullscreen_within_gaps,
                    ..
                }) = self.kind.get(node)
                && (*fullscreen || *fullscreen_within_gaps)
            {
                continue;
            }
            self.remove_window_internal(layout, wid);
        }
        for wid in delta.additions {
            self.add_window_after_selection(layout, wid);
        }
    }

    fn apply_target_drop_unshadowed(
        &mut self,
        layout: LayoutId,
        source: WindowId,
        target: WindowId,
        action: crate::layout_engine::WindowDropAction,
    ) -> bool {
        if !self.window_participates(source) || !self.window_participates(target) {
            return false;
        }
        if source == target
            || !self.contains_window(layout, source)
            || !self.contains_window(layout, target)
        {
            return false;
        }
        if action == crate::layout_engine::WindowDropAction::Swap {
            return self.swap_windows(layout, source, target);
        }

        while self.remove_window_internal(layout, source) {}
        let Some(target_node) = self.node_for_window_in_layout(layout, target) else {
            return false;
        };
        match action {
            crate::layout_engine::WindowDropAction::Stack => {
                let stack = self.stacks.entry(target_node).or_insert_with(|| vec![target]);
                stack.retain(|window| *window != source);
                stack.push(source);
                if let Some(NodeKind::Leaf { window, .. }) = self.kind.get_mut(target_node) {
                    *window = Some(source);
                }

                self.tree.data.selection.select(&self.tree.map, target_node);
                true
            }
            crate::layout_engine::WindowDropAction::Insert(direction) => {
                self.split_leaf_in_direction(target_node, direction, source);
                true
            }
            crate::layout_engine::WindowDropAction::Swap
            | crate::layout_engine::WindowDropAction::Move(_)
            | crate::layout_engine::WindowDropAction::ConsumeOrExpel(_) => {
                unreachable!()
            }
        }
    }

    fn move_selection_to_layout_after_selection_unshadowed(
        &mut self,
        from_layout: LayoutId,
        to_layout: LayoutId,
    ) {
        let sel = self.selected_window(from_layout);
        if let Some(w) = sel {
            while self.remove_window_internal(from_layout, w) {}
            self.add_window_after_selection(to_layout, w);
        }
    }

    fn join_selection_with_direction_unshadowed(&mut self, layout: LayoutId, direction: Direction) {
        let Some(sel) = self.selection_of_layout(layout) else {
            return;
        };
        let sel_leaf = self.descend_to_leaf(sel);

        let Some(neighbor) = self.find_neighbor_leaf(sel_leaf, direction) else {
            return;
        };

        let mut current = sel_leaf;
        while let Some(parent) = current.parent(&self.tree.map) {
            let children: Vec<_> = parent.children(&self.tree.map).collect();
            if children.contains(&neighbor) {
                if parent.parent(&self.tree.map).is_some() {
                    let mut windows = Vec::new();
                    self.collect_windows_under(parent, &mut windows);

                    // The last member is the visible one, so keep the selected window there.
                    let selected = self.selected_window(layout);

                    if let Some(index) = windows.iter().position(|window| Some(*window) == selected)
                    {
                        windows[index..].rotate_left(1);
                    }

                    if let Some(visible) = windows.last().copied() {
                        // Take the collapsed split's place so a first child stays first.
                        let new_leaf = self.make_leaf(Some(visible));
                        new_leaf.detach(&mut self.tree).insert_after(parent);

                        let ids: Vec<_> = parent.traverse_preorder(&self.tree.map).collect();

                        for id in ids {
                            self.kind.remove(id);
                            self.stacks.remove(&id);
                        }

                        parent.detach(&mut self.tree).remove();

                        self.stacks.insert(new_leaf, windows);
                        self.tree.data.selection.select(&self.tree.map, new_leaf);
                    }
                }
                break;
            }
            current = parent;
        }
    }

    fn consume_or_expel_selection_unshadowed(&mut self, layout: LayoutId, direction: Direction) {
        let is_joined = self
            .selection_of_layout(layout)
            .map(|selection| self.descend_to_leaf(selection))
            .and_then(|leaf| self.stacks.get(&leaf))
            .is_some_and(|members| members.len() > 1);

        if is_joined {
            self.unjoin_selection(layout);
        } else {
            self.join_selection_with_direction(layout, direction);
        }
    }

    fn unjoin_selection_unshadowed(&mut self, layout: LayoutId) {
        let Some(sel) = self.selection_of_layout(layout) else {
            return;
        };

        let leaf = self.descend_to_leaf(sel);
        let Some(NodeKind::Leaf {
            window,
            fullscreen,
            fullscreen_within_gaps,
            ..
        }) = self.kind.get_mut(leaf)
        else {
            return;
        };

        let Some(members) = self.stacks.remove(&leaf) else {
            return;
        };

        let flags = (*fullscreen, *fullscreen_within_gaps);

        // Retile this stack in place using depth-based splits.
        // The visible member is last in the stack, so it ends up selected.
        *window = None;
        self.tree.data.selection.select(&self.tree.map, leaf);

        for member in members {
            self.insert_window_at_selection(layout, member);
        }

        // Insertion rebuilds leaves without fullscreen, so hand it back to the visible member.
        if let Some(visible) = self.selection_of_layout(layout)
            && let Some(NodeKind::Leaf {
                fullscreen,
                fullscreen_within_gaps,
                ..
            }) = self.kind.get_mut(visible)
        {
            (*fullscreen, *fullscreen_within_gaps) = flags;
        }
    }

    fn swap_windows_unshadowed(&mut self, layout: LayoutId, a: WindowId, b: WindowId) -> bool {
        if !self.window_participates(a) || !self.window_participates(b) {
            return false;
        }
        let Some(node_a) = self.node_for_window_in_layout(layout, a) else {
            return false;
        };
        let Some(node_b) = self.node_for_window_in_layout(layout, b) else {
            return false;
        };
        if node_a == node_b {
            let Some(stack) = self.stacks.get_mut(&node_a) else {
                return false;
            };
            if !stack.contains(&a) || !stack.contains(&b) {
                return false;
            }
            // A BSP stack has one active leaf window. Keep the dragged source active
            // while moving the target immediately behind it in stack order.
            stack.retain(|window| *window != a && *window != b);
            // Swapping a window with itself brings it to the top, once.
            if b != a {
                stack.push(b);
            }
            stack.push(a);
            if let Some(NodeKind::Leaf { window, .. }) = self.kind.get_mut(node_a) {
                *window = Some(a);
            }
            self.tree.data.selection.select(&self.tree.map, node_a);
            return true;
        }

        if self.stacks.contains_key(&node_a) || self.stacks.contains_key(&node_b) {
            let group_a = self.stacks.remove(&node_a).unwrap_or_else(|| vec![a]);
            let group_b = self.stacks.remove(&node_b).unwrap_or_else(|| vec![b]);
            let visible_a = *group_a.last().unwrap();
            let visible_b = *group_b.last().unwrap();
            if let Some(NodeKind::Leaf { window, .. }) = self.kind.get_mut(node_a) {
                *window = Some(visible_b);
            }
            if let Some(NodeKind::Leaf { window, .. }) = self.kind.get_mut(node_b) {
                *window = Some(visible_a);
            }
            if group_b.len() > 1 {
                self.stacks.insert(node_a, group_b);
            }
            if group_a.len() > 1 {
                self.stacks.insert(node_b, group_a);
            }
            return true;
        }

        let mut a_window = None;
        let mut b_window = None;
        if let Some(NodeKind::Leaf { window, .. }) = self.kind.get(node_a) {
            a_window = *window;
        }
        if let Some(NodeKind::Leaf { window, .. }) = self.kind.get(node_b) {
            b_window = *window;
        }

        if a_window.is_none() && b_window.is_none() {
            return false;
        }

        if let Some(NodeKind::Leaf { window, .. }) = self.kind.get_mut(node_a) {
            *window = b_window;
        }
        if let Some(NodeKind::Leaf { window, .. }) = self.kind.get_mut(node_b) {
            *window = a_window;
        }

        true
    }

    fn move_selection_unshadowed(&mut self, layout: LayoutId, direction: Direction) -> bool {
        let Some(sel) = self.selection_of_layout(layout) else {
            return false;
        };
        let moving = self.descend_to_leaf(sel);
        if !matches!(
            self.kind.get(moving),
            Some(NodeKind::Leaf { window: Some(_), .. })
        ) {
            return false;
        }
        let Some(target) = self.find_neighbor_leaf(moving, direction) else {
            return false;
        };
        let Some(old_parent) = moving.parent(&self.tree.map) else {
            return false;
        };

        // Siblings already share the requested split: reorder them without
        // resetting the split ratio or exchanging their window state.
        if target.parent(&self.tree.map) == Some(old_parent) {
            match direction {
                Direction::Left | Direction::Up => {
                    moving.detach(&mut self.tree).insert_before(target);
                }
                Direction::Right | Direction::Down => {
                    moving.detach(&mut self.tree).push_back(old_parent);
                }
            }
        } else if matches!(self.kind.get(target), Some(NodeKind::Leaf { window: None, .. })) {
            // A pre-created empty slot receives the leaf directly.
            moving.detach(&mut self.tree).insert_before(target);
            target.detach(&mut self.tree).remove();
            self.kind.remove(target);
            self.collapse_split(old_parent);
        } else {
            // Transfer the entire leaf (including stacks and fullscreen state)
            // onto a new split at the directional neighbor, then reclaim its
            // old region for the sibling left behind.
            let split = self.tree.mk_node().into_id();
            self.kind.insert(split, NodeKind::Split {
                orientation: direction.orientation(),
                ratio: 0.5,
            });
            split.detach(&mut self.tree).insert_before(target);
            let (first, second) = match direction {
                Direction::Left | Direction::Up => (moving, target),
                Direction::Right | Direction::Down => (target, moving),
            };
            first.detach(&mut self.tree).push_back(split);
            second.detach(&mut self.tree).push_back(split);
            self.collapse_split(old_parent);
        }
        self.tree.data.selection.select(&self.tree.map, moving);
        true
    }

    fn split_selection_unshadowed(&mut self, layout: LayoutId, kind: LayoutKind) {
        if !self.inactive.is_empty() && self.visible_windows_in_layout(layout).is_empty() {
            return;
        }
        let orientation = match kind {
            LayoutKind::Horizontal => Orientation::Horizontal,
            LayoutKind::Vertical => Orientation::Vertical,
            _ => return,
        };
        let state = if let Some(s) = self.layouts.get(layout).copied() {
            s
        } else {
            return;
        };

        let sel = self
            .selection_of_layout(layout)
            .unwrap_or_else(|| self.tree.data.selection.current_selection(state.root));
        let target = self.descend_to_leaf(sel);
        match self.kind.get(target).cloned() {
            Some(NodeKind::Leaf { window, .. }) => {
                let left = self.make_leaf(window);
                let right = self.make_leaf(None);
                if let Some(stack) = self.stacks.remove(&target) {
                    self.stacks.insert(left, stack);
                }
                self.kind.insert(target, NodeKind::Split { orientation, ratio: 0.5 });
                left.detach(&mut self.tree).push_back(target);
                right.detach(&mut self.tree).push_back(target);
                self.tree.data.selection.select(&self.tree.map, right);
            }
            _ => {}
        }
    }

    fn toggle_fullscreen_of_selection_unshadowed(&mut self, layout: LayoutId) -> Vec<WindowId> {
        if let Some(sel) = self.selection_of_layout(layout) {
            let sel_leaf = self.descend_to_leaf(sel);
            let active = self.participating_window(sel_leaf);
            if let Some(NodeKind::Leaf {
                window: Some(_),
                fullscreen,
                fullscreen_within_gaps,
                ..
            }) = self.kind.get_mut(sel_leaf)
            {
                *fullscreen = !*fullscreen;
                if *fullscreen {
                    *fullscreen_within_gaps = false;
                }
                return active.into_iter().collect();
            }
        }
        vec![]
    }

    fn toggle_fullscreen_within_gaps_of_selection_unshadowed(
        &mut self,
        layout: LayoutId,
    ) -> Vec<WindowId> {
        if let Some(sel) = self.selection_of_layout(layout) {
            let sel_leaf = self.descend_to_leaf(sel);
            let active = self.participating_window(sel_leaf);
            if let Some(NodeKind::Leaf {
                window: Some(_),
                fullscreen_within_gaps,
                fullscreen,
                ..
            }) = self.kind.get_mut(sel_leaf)
            {
                *fullscreen_within_gaps = !*fullscreen_within_gaps;
                if *fullscreen_within_gaps {
                    *fullscreen = false;
                }
                return active.into_iter().collect();
            }
        }
        vec![]
    }

    fn rebalance_unshadowed(&mut self, layout: LayoutId) {
        if let Some(state) = self.layouts.get(layout).copied() {
            self.rebalance_node(state.root, Orientation::Horizontal);
        }
    }

    fn toggle_tile_orientation_unshadowed(&mut self, layout: LayoutId) {
        if !self.inactive.is_empty() && self.visible_windows_in_layout(layout).is_empty() {
            return;
        }
        let sel_snapshot = self.selection_of_layout(layout);

        let start_node = if let Some(sel) = sel_snapshot {
            sel
        } else {
            let Some(state) = self.layouts.get(layout) else {
                return;
            };
            state.root
        };

        let mut node_opt = Some(start_node);
        while let Some(node) = node_opt {
            if let Some(NodeKind::Split { orientation, .. }) = self.kind.get_mut(node) {
                *orientation = match *orientation {
                    Orientation::Horizontal => Orientation::Vertical,
                    Orientation::Vertical => Orientation::Horizontal,
                };
                return;
            }
            node_opt = node.parent(&self.tree.map);
        }

        if let Some(state) = self.layouts.get_mut(layout) {
            let root = state.root;
            if let Some(NodeKind::Split { orientation, .. }) = self.kind.get_mut(root) {
                *orientation = match *orientation {
                    Orientation::Horizontal => Orientation::Vertical,
                    Orientation::Vertical => Orientation::Horizontal,
                };
            }
        }
    }

    fn resize_selection_by_unshadowed(
        &mut self,
        layout: LayoutId,
        amount: f64,
        orientation: ResizeOrientation,
    ) {
        let sel_snapshot = self.selection_of_layout(layout);
        let Some(mut node) = sel_snapshot else {
            return;
        };

        while let Some(parent) = node.parent(&self.tree.map) {
            if let Some(NodeKind::Split {
                orientation: split_orientation,
                ratio,
            }) = self.kind.get_mut(parent)
                && match orientation {
                    ResizeOrientation::Horizontal => *split_orientation == Orientation::Horizontal,
                    ResizeOrientation::Vertical => *split_orientation == Orientation::Vertical,
                    ResizeOrientation::Smart => true,
                }
            {
                let is_first = Some(node) == parent.first_child(&self.tree.map);
                let delta = (amount as f32) * 0.5;
                if is_first {
                    *ratio = (*ratio + delta).clamp(0.05, 0.95);
                } else {
                    *ratio = (*ratio - delta).clamp(0.05, 0.95);
                }
                break;
            }
            node = parent;
        }
    }

    fn set_window_participation_unshadowed(
        &mut self,
        window: WindowId,
        participates: bool,
    ) -> bool {
        if participates {
            self.inactive.remove(&window)
        } else {
            self.inactive.insert(window)
        }
    }

    fn remove_layout_unshadowed(&mut self, layout: LayoutId) {
        if let Some(state) = self.layouts.remove(layout) {
            let ids: Vec<_> = state.root.traverse_preorder(&self.tree.map).collect();
            for id in ids {
                self.kind.remove(id);
                self.stacks.remove(&id);
            }
            state.root.remove_root(&mut self.tree);
        }
    }

    fn ascend_selection_unshadowed(&mut self, layout: LayoutId) -> bool {
        if let Some(sel) = self.selection_of_layout(layout) {
            if self.kind.get(sel).is_none() {
                return false;
            }
            let parent_opt = sel.parent(&self.tree.map);
            if let Some(parent) = parent_opt {
                let new_sel = self.descend_to_leaf(parent);
                self.tree.data.selection.select(&self.tree.map, new_sel);
                return true;
            }
        }
        false
    }

    fn descend_selection_unshadowed(&mut self, layout: LayoutId) -> bool {
        if let Some(sel) = self.selection_of_layout(layout) {
            let new_sel = self.descend_to_leaf(sel);
            if new_sel != sel {
                self.tree.data.selection.select(&self.tree.map, new_sel);
                return true;
            }
        }
        false
    }

    fn move_focus_unshadowed(
        &mut self,
        layout: LayoutId,
        direction: Direction,
    ) -> (Option<WindowId>, Vec<WindowId>) {
        if !self.inactive.is_empty() && self.visible_windows_in_layout(layout).is_empty() {
            return (None, vec![]);
        }
        let sel_snapshot = self.selection_of_layout(layout);
        let Some(current_sel) = sel_snapshot else {
            return (None, vec![]);
        };
        let current_leaf = self.descend_to_leaf(current_sel);
        let Some(next_leaf) = self.find_neighbor_leaf(current_leaf, direction) else {
            return (None, vec![]);
        };
        self.tree.data.selection.select(&self.tree.map, next_leaf);
        let focus = self.participating_window(next_leaf);
        // Tiled BSP windows never overlap, so only the focus target needs to be
        // raised. Raising every visible window makes the raise manager front
        // each app in turn (via make_key_window) before the target, which
        // flickers the menu bar and any focus border on every focus move. The
        // traditional engine already raises only the revealed group.
        (focus, focus.into_iter().collect())
    }

    fn add_window_after_selection_unshadowed(&mut self, layout: LayoutId, wid: WindowId) {
        if self.contains_window(layout, wid) {
            return;
        }
        if self.layouts.get(layout).is_some() {
            if self.window_insertion_point == WindowInsertionPoint::EndOfTree {
                let root = self.layouts[layout].root;
                if let Some(leaf) = root
                    .traverse_preorder(&self.tree.map)
                    .filter(|node| matches!(self.kind.get(*node), Some(NodeKind::Leaf { .. })))
                    .last()
                {
                    self.tree.data.selection.select(&self.tree.map, leaf);
                }
                self.insert_window_at_selection(layout, wid);
                return;
            }
            // Try smart insertion first (with preselection support)
            if !self.smart_insert_window(layout, wid) {
                // Fall back to default insertion
                self.insert_window_at_selection(layout, wid);
            }
        }
    }

    fn replace_window_unshadowed(&mut self, from: WindowId, to: WindowId) {
        if self.inactive.remove(&from) {
            self.inactive.insert(to);
        }
        if from == to {
            return;
        }
        let layouts: Vec<_> = self.layouts.keys().collect();
        for layout in layouts {
            if !self.contains_window(layout, from) {
                continue;
            }
            // A replacement may already be present after restore matching.
            while self.remove_window_internal(layout, to) {}
            let root = self.layouts[layout].root;
            for node in root.traverse_preorder(&self.tree.map) {
                if let Some(stack) = self.stacks.get_mut(&node) {
                    for member in stack.iter_mut().filter(|member| **member == from) {
                        *member = to;
                    }
                }
                if let Some(NodeKind::Leaf { window, .. }) = self.kind.get_mut(node)
                    && *window == Some(from)
                {
                    *window = Some(to);
                }
            }
        }
    }

    fn remove_window_unshadowed(&mut self, wid: WindowId) {
        self.inactive.remove(&wid);
        let layouts: Vec<_> = self.layouts.keys().collect();
        for layout in layouts {
            while self.remove_window_internal(layout, wid) {}
        }
    }

    fn select_window_unshadowed(&mut self, layout: LayoutId, wid: WindowId) -> bool {
        if self.inactive.contains(&wid) {
            return false;
        }
        let Some(node) = self.node_for_window_in_layout(layout, wid) else {
            return false;
        };
        if let Some(stack) = self.stacks.get_mut(&node)
            && let Some(index) = stack.iter().position(|member| *member == wid)
        {
            stack.remove(index);
            stack.push(wid);
            if let Some(NodeKind::Leaf { window, .. }) = self.kind.get_mut(node) {
                *window = Some(wid);
            }
        }
        self.tree.data.selection.select(&self.tree.map, node);
        true
    }

    fn create_layout_unshadowed(&mut self) -> LayoutId {
        let leaf = self.make_leaf(None);
        let state = LayoutState { root: leaf };
        self.layouts.insert(state)
    }
}

impl LayoutSystem for BspLayoutSystem {
    fn window_participates(&self, window: WindowId) -> bool { !self.inactive.contains(&window) }

    fn set_window_participation(&mut self, window: WindowId, participates: bool) -> bool {
        self.operate(
            "set_window_participation",
            Scope::All,
            |this| this.set_window_participation_unshadowed(window, participates),
            |core| core.set_participation(window, participates),
        )
    }

    fn create_layout(&mut self) -> LayoutId {
        let layout = self.create_layout_unshadowed();
        if self.depth == 0 {
            self.core.create(layout);
            #[cfg(test)]
            self.assert_held_as("create_layout", Scope::All, &self.core);
        }
        layout
    }

    fn contains_layout(&self, layout: LayoutId) -> bool {
        if self.depth > 0 {
            return self.contains_layout_from_tree(layout);
        }
        let answer = self.core.layout(layout).is_some();
        #[cfg(test)]
        assert_eq!(
            answer,
            self.contains_layout_from_tree(layout),
            "contains_layout: the core answers differently"
        );
        answer
    }

    /// shallow
    fn clone_layout(&mut self, layout: LayoutId) -> LayoutId {
        self.depth += 1;
        let copy = self.clone_layout_unshadowed(layout);
        self.depth -= 1;
        if self.depth == 0 {
            let insertion = self.insertion();
            self.core.clone_layout(layout, copy, insertion);
            #[cfg(test)]
            self.assert_held_as("clone_layout", Scope::All, &self.core);
        }
        copy
    }

    fn remove_layout(&mut self, layout: LayoutId) {
        self.operate(
            "remove_layout",
            Scope::All,
            |this| this.remove_layout_unshadowed(layout),
            |core| core.remove_layout(layout),
        )
    }

    fn draw_tree(&self, layout: LayoutId) -> String {
        if self.depth > 0 {
            return self.draw_tree_from_tree(layout);
        }
        let answer = self
            .core
            .layout(layout)
            .map_or_else(|| "<empty bsp>".to_string(), bsp_tree::Layout::draw);
        #[cfg(test)]
        assert_eq!(
            answer,
            self.draw_tree_from_tree(layout),
            "draw_tree: the core answers differently"
        );
        answer
    }

    fn container_tree(&self, layout: LayoutId) -> rift_protocol::ContainerTreeNode {
        if self.depth > 0 {
            return self.container_tree_from_tree(layout);
        }
        let answer = container_node(&self.core.layout(layout).expect("unknown BSP layout").view());
        #[cfg(test)]
        assert_eq!(
            normalized(answer.clone()),
            normalized(self.container_tree_from_tree(layout)),
            "container_tree: the core answers differently"
        );
        answer
    }

    fn calculate_layout(
        &self,
        layout: LayoutId,
        screen: CGRect,
        stack_offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &crate::common::config::GapSettings,
        stack_line: crate::common::config::StackLineReserve,
    ) -> Vec<(WindowId, CGRect)> {
        if self.depth > 0 {
            return self.calculate_layout_from_tree(
                layout,
                screen,
                stack_offset,
                constraints,
                gaps,
                stack_line,
            );
        }
        let shape = self.core.layout(layout).and_then(|tree| {
            tree.shape(self.core.participation(), &|window| {
                constraints.get(&window).copied()
            })
        });
        // Nothing in the layout takes space: no shape, no frames.
        let answer = shape.map_or_else(Vec::new, |root| {
            crate::layout_engine::tile_bsp(&bsp::Bsp {
                screen: crate::layout_engine::core_rect(screen),
                gaps: crate::layout_engine::core_gaps(gaps),
                root,
                aspect: self.single_window_aspect_ratio,
            })
        });
        #[cfg(test)]
        assert_eq!(
            answer,
            self.calculate_layout_from_tree(
                layout,
                screen,
                stack_offset,
                constraints,
                gaps,
                stack_line
            ),
            "calculate_layout: the core answers differently"
        );
        answer
    }

    fn selected_window(&self, layout: LayoutId) -> Option<WindowId> {
        if self.depth > 0 {
            return self.selected_window_from_tree(layout);
        }
        let answer = self.core.projection(layout).and_then(|projection| projection.selected_window);
        #[cfg(test)]
        assert_eq!(
            answer,
            self.selected_window_from_tree(layout),
            "selected_window: the core answers differently"
        );
        answer
    }

    fn all_windows_in_layout(&self, layout: LayoutId) -> Vec<WindowId> {
        if self.depth > 0 {
            return self.all_windows_in_layout_from_tree(layout);
        }
        let answer = self
            .core
            .projection(layout)
            .map_or_else(Vec::new, |projection| projection.windows);
        #[cfg(test)]
        assert_eq!(
            answer,
            self.all_windows_in_layout_from_tree(layout),
            "all_windows_in_layout: the core answers differently"
        );
        answer
    }

    fn window_slot(&self, layout: LayoutId, window: WindowId) -> Option<Vec<usize>> {
        if self.depth > 0 {
            return self.window_slot_from_tree(layout, window);
        }
        let answer = self.core.layout(layout).and_then(|tree| tree.slot(window)).map(slot_path);
        #[cfg(test)]
        assert_eq!(
            answer,
            self.window_slot_from_tree(layout, window),
            "window_slot: the core answers differently"
        );
        answer
    }

    fn visible_windows_in_layout(&self, layout: LayoutId) -> Vec<WindowId> {
        if self.depth > 0 {
            return self.visible_windows_in_layout_from_tree(layout);
        }
        let answer = self
            .core
            .projection(layout)
            .map_or_else(Vec::new, |projection| projection.visible);
        #[cfg(test)]
        assert_eq!(
            answer,
            self.visible_windows_in_layout_from_tree(layout),
            "visible_windows_in_layout: the core answers differently"
        );
        answer
    }

    fn stack_members(&self, layout: LayoutId, window: WindowId) -> Vec<WindowId> {
        if self.depth > 0 {
            return self.stack_members_from_tree(layout, window);
        }
        let answer = self
            .core
            .layout(layout)
            .map_or_else(Vec::new, |tree| tree.stack_members(window));
        #[cfg(test)]
        assert_eq!(
            answer,
            self.stack_members_from_tree(layout, window),
            "stack_members: the core answers differently"
        );
        answer
    }

    fn visible_windows_under_selection(&self, layout: LayoutId) -> Vec<WindowId> {
        if self.depth > 0 {
            return self.visible_windows_under_selection_from_tree(layout);
        }
        let answer = self
            .core
            .projection(layout)
            .map_or_else(Vec::new, |projection| projection.under_focus);
        #[cfg(test)]
        assert_eq!(
            answer,
            self.visible_windows_under_selection_from_tree(layout),
            "visible_windows_under_selection: the core answers differently"
        );
        answer
    }

    fn ascend_selection(&mut self, layout: LayoutId) -> bool {
        self.operate(
            "ascend_selection",
            Scope::One(layout),
            |this| this.ascend_selection_unshadowed(layout),
            |core| core.ascend(layout),
        )
    }

    fn descend_selection(&mut self, layout: LayoutId) -> bool {
        self.operate(
            "descend_selection",
            Scope::One(layout),
            |this| this.descend_selection_unshadowed(layout),
            |core| core.descend(layout),
        )
    }

    fn move_focus(
        &mut self,
        layout: LayoutId,
        direction: Direction,
    ) -> (Option<WindowId>, Vec<WindowId>) {
        self.operate(
            "move_focus",
            Scope::One(layout),
            |this| this.move_focus_unshadowed(layout, direction),
            |core| {
                let focus = core.move_focus(layout, core_direction(direction));
                (focus, focus.into_iter().collect())
            },
        )
    }

    fn window_in_direction(&self, layout: LayoutId, direction: Direction) -> Option<WindowId> {
        if self.depth > 0 {
            return self.window_in_direction_from_tree(layout, direction);
        }
        let answer = self.core.layout(layout).and_then(|tree| {
            tree.window_in_direction(core_direction(direction), self.core.participation())
        });
        #[cfg(test)]
        assert_eq!(
            answer,
            self.window_in_direction_from_tree(layout, direction),
            "window_in_direction: the core answers differently"
        );
        answer
    }

    fn add_window_after_selection(&mut self, layout: LayoutId, wid: WindowId) {
        let insertion = match self.window_insertion_point {
            WindowInsertionPoint::EndOfTree => bsp_tree::Insertion::EndOfTree,
            _ => bsp_tree::Insertion::AtFocus,
        };
        self.operate(
            "add_window_after_selection",
            Scope::One(layout),
            |this| this.add_window_after_selection_unshadowed(layout, wid),
            |core| core.add(layout, wid, insertion),
        )
    }

    fn replace_window(&mut self, from: WindowId, to: WindowId) {
        self.operate(
            "replace_window",
            Scope::All,
            |this| this.replace_window_unshadowed(from, to),
            |core| core.rekey(from, to),
        )
    }

    fn remove_window(&mut self, wid: WindowId) {
        self.operate(
            "remove_window",
            Scope::All,
            |this| this.remove_window_unshadowed(wid),
            |core| core.remove_window(wid),
        )
    }

    fn remove_windows_for_app(&mut self, pid: pid_t) {
        let mut windows = Vec::new();
        for state in self.layouts.values() {
            self.collect_windows_under(state.root, &mut windows);
        }
        windows.retain(|window| window.pid == pid);
        windows.sort_unstable();
        windows.dedup();
        for window in windows {
            self.remove_window(window);
        }
    }

    fn set_windows_for_app(&mut self, layout: LayoutId, pid: pid_t, desired: Vec<WindowId>) {
        let (removals, additions) = self.app_delta(layout, pid, desired.clone());
        let insertion = match self.window_insertion_point {
            WindowInsertionPoint::EndOfTree => bsp_tree::Insertion::EndOfTree,
            _ => bsp_tree::Insertion::AtFocus,
        };
        self.operate(
            "set_windows_for_app",
            Scope::One(layout),
            |this| this.set_windows_for_app_unshadowed(layout, pid, desired),
            |core| core.set_app_windows(layout, &removals, &additions, insertion),
        )
    }

    fn contains_window(&self, layout: LayoutId, wid: WindowId) -> bool {
        if self.depth > 0 {
            return self.contains_window_from_tree(layout, wid);
        }
        let answer = self.core.layout(layout).is_some_and(|tree| tree.contains(wid));
        #[cfg(test)]
        assert_eq!(
            answer,
            self.contains_window_from_tree(layout, wid),
            "contains_window: the core answers differently"
        );
        answer
    }

    fn select_window(&mut self, layout: LayoutId, wid: WindowId) -> bool {
        self.operate(
            "select_window",
            Scope::One(layout),
            |this| this.select_window_unshadowed(layout, wid),
            |core| core.select_window(layout, wid),
        )
    }

    fn on_window_resized(
        &mut self,
        layout: LayoutId,
        wid: WindowId,
        old_frame: CGRect,
        new_frame: CGRect,
        screen: CGRect,
        gaps: &crate::common::config::GapSettings,
    ) {
        let resized = bsp_tree::Resized {
            old: crate::layout_engine::core_rect(old_frame),
            new: crate::layout_engine::core_rect(new_frame),
            screen: crate::layout_engine::core_rect(screen),
            tiling: crate::layout_engine::core_rect(compute_tiling_area(screen, gaps)),
        };
        self.operate(
            "on_window_resized",
            Scope::One(layout),
            |this| {
                this.on_window_resized_unshadowed(layout, wid, old_frame, new_frame, screen, gaps)
            },
            |core| core.resized(layout, wid, resized),
        )
    }

    fn move_selection(&mut self, layout: LayoutId, direction: Direction) -> bool {
        self.operate(
            "move_selection",
            Scope::One(layout),
            |this| this.move_selection_unshadowed(layout, direction),
            |core| core.move_selection(layout, core_direction(direction)),
        )
    }

    fn swap_windows(&mut self, layout: LayoutId, a: WindowId, b: WindowId) -> bool {
        self.operate(
            "swap_windows",
            Scope::One(layout),
            |this| this.swap_windows_unshadowed(layout, a, b),
            |core| core.swap(layout, a, b),
        )
    }

    fn apply_target_drop(
        &mut self,
        layout: LayoutId,
        source: WindowId,
        target: WindowId,
        action: crate::layout_engine::WindowDropAction,
    ) -> bool {
        use crate::layout_engine::WindowDropAction as Action;
        self.operate(
            "apply_target_drop",
            Scope::One(layout),
            |this| this.apply_target_drop_unshadowed(layout, source, target, action),
            |core| match action {
                Action::Swap => core.drop_on(layout, source, target, bsp_tree::Drop::Swap),
                Action::Stack => core.drop_on(layout, source, target, bsp_tree::Drop::Stack),
                Action::Insert(direction) => core.drop_on(
                    layout,
                    source,
                    target,
                    bsp_tree::Drop::Insert(core_direction(direction)),
                ),
                Action::Move(_) | Action::ConsumeOrExpel(_) => false,
            },
        )
    }

    fn move_selection_to_layout_after_selection(
        &mut self,
        from_layout: LayoutId,
        to_layout: LayoutId,
    ) {
        let insertion = match self.window_insertion_point {
            WindowInsertionPoint::EndOfTree => bsp_tree::Insertion::EndOfTree,
            _ => bsp_tree::Insertion::AtFocus,
        };
        self.operate(
            "move_selection_to_layout_after_selection",
            Scope::All,
            |this| this.move_selection_to_layout_after_selection_unshadowed(from_layout, to_layout),
            |core| core.move_selected(from_layout, to_layout, insertion),
        )
    }

    fn split_selection(&mut self, layout: LayoutId, kind: LayoutKind) {
        self.operate(
            "split_selection",
            Scope::One(layout),
            |this| this.split_selection_unshadowed(layout, kind),
            |core| match kind {
                LayoutKind::Horizontal => core.split(layout, bsp_tree::Orientation::Horizontal),
                LayoutKind::Vertical => core.split(layout, bsp_tree::Orientation::Vertical),
                _ => {}
            },
        )
    }

    fn toggle_fullscreen_of_selection(&mut self, layout: LayoutId) -> Vec<WindowId> {
        self.operate(
            "toggle_fullscreen",
            Scope::One(layout),
            |this| this.toggle_fullscreen_of_selection_unshadowed(layout),
            |core| {
                core.toggle_fullscreen(layout, bsp_tree::Fullscreen::Screen)
                    .into_iter()
                    .collect()
            },
        )
    }

    fn toggle_fullscreen_within_gaps_of_selection(&mut self, layout: LayoutId) -> Vec<WindowId> {
        self.operate(
            "toggle_fullscreen_within_gaps",
            Scope::One(layout),
            |this| this.toggle_fullscreen_within_gaps_of_selection_unshadowed(layout),
            |core| {
                core.toggle_fullscreen(layout, bsp_tree::Fullscreen::WithinGaps)
                    .into_iter()
                    .collect()
            },
        )
    }

    fn has_any_fullscreen_node(&self, layout: LayoutId) -> bool {
        if self.depth > 0 {
            return self.has_any_fullscreen_node_from_tree(layout);
        }
        let answer = self.core.projection(layout).is_some_and(|projection| projection.fullscreen);
        #[cfg(test)]
        assert_eq!(
            answer,
            self.has_any_fullscreen_node_from_tree(layout),
            "has_any_fullscreen_node: the core answers differently"
        );
        answer
    }

    fn join_selection_with_direction(&mut self, layout: LayoutId, direction: Direction) {
        self.operate(
            "join_selection_with_direction",
            Scope::One(layout),
            |this| this.join_selection_with_direction_unshadowed(layout, direction),
            |core| core.join(layout, core_direction(direction)),
        )
    }

    fn consume_or_expel_selection(&mut self, layout: LayoutId, direction: Direction) {
        self.operate(
            "consume_or_expel_selection",
            Scope::One(layout),
            |this| this.consume_or_expel_selection_unshadowed(layout, direction),
            |core| core.consume_or_expel(layout, core_direction(direction)),
        )
    }

    fn unjoin_selection(&mut self, layout: LayoutId) {
        self.operate(
            "unjoin_selection",
            Scope::One(layout),
            |this| this.unjoin_selection_unshadowed(layout),
            |core| core.unjoin(layout),
        )
    }

    fn resize_selection_by(
        &mut self,
        layout: LayoutId,
        amount: f64,
        orientation: ResizeOrientation,
    ) {
        let along = match orientation {
            ResizeOrientation::Horizontal => bsp_tree::Along::Horizontal,
            ResizeOrientation::Vertical => bsp_tree::Along::Vertical,
            ResizeOrientation::Smart => bsp_tree::Along::Either,
        };
        self.operate(
            "resize_selection_by",
            Scope::One(layout),
            |this| this.resize_selection_by_unshadowed(layout, amount, orientation),
            |core| core.resize(layout, amount, along),
        )
    }

    fn rebalance(&mut self, layout: LayoutId) {
        self.operate(
            "rebalance",
            Scope::One(layout),
            |this| this.rebalance_unshadowed(layout),
            |core| core.rebalance(layout),
        )
    }

    fn toggle_tile_orientation(&mut self, layout: LayoutId) {
        self.operate(
            "toggle_tile_orientation",
            Scope::One(layout),
            |this| this.toggle_tile_orientation_unshadowed(layout),
            |core| core.toggle_orientation(layout),
        )
    }
}

/// A tree the core cannot hold, as this system (or a saved layout) has it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum OldTree {
    NodeWithoutKind,
    /// A split of other than two children.
    SplitOf(usize),
    /// A split whose remembered child is not one of its children.
    SplitRemembersAStranger,
    /// A split ratio outside 0.05..=0.95, or not a number.
    Ratio(f32),
    /// A leaf with a selection memory (only splits remember).
    LeafWithMemory,
    /// A leaf flagged both fullscreen and fullscreen within gaps.
    BothFullscreens,
    /// An empty leaf with a fill.
    EmptyFilled(bsp_tree::Fill),
    /// A stack whose shown window is not one of its members.
    StackShowsAStranger,
    /// A stack on a leaf showing no window.
    StackShowsNothing,
}

impl std::fmt::Display for OldTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a BSP layout the core cannot hold: {self:?}")
    }
}

/// Every operation runs on rift's NodeId trees (`old`) and, at the outermost, on the core's
/// (`core`), whose answer is the answer; in test builds the two must agree, answers and trees.
impl BspLayoutSystem {
    fn operate<A: PartialEq + std::fmt::Debug>(
        &mut self,
        operation: &str,
        scope: Scope,
        old: impl FnOnce(&mut Self) -> A,
        core: impl FnOnce(&mut bsp_tree::Layouts<LayoutId, WindowId>) -> A,
    ) -> A {
        self.depth += 1;
        let answer = old(self);
        self.depth -= 1;
        if self.depth > 0 {
            return answer;
        }
        let core_answer = core(&mut self.core);
        #[cfg(test)]
        {
            assert_eq!(answer, core_answer, "{operation}: the core answered differently");
            self.assert_held_as(operation, scope, &self.core);
        }
        #[cfg(not(test))]
        let _ = (operation, scope, answer);
        core_answer
    }

    /// The windows `set_windows_for_app` will take out and put in, as it computes them.
    fn app_delta(
        &self,
        layout: LayoutId,
        pid: pid_t,
        desired: Vec<WindowId>,
    ) -> (Vec<WindowId>, Vec<WindowId>) {
        let current: Vec<WindowId> = self
            .all_windows_in_layout(layout)
            .into_iter()
            .filter(|w| w.pid == pid)
            .collect();
        let delta =
            reconcile_app_membership(pid, current, desired, |w| self.window_participates(w));
        (delta.removals, delta.additions)
    }

    /// The window insertion setting as the core takes it.
    fn insertion(&self) -> bsp_tree::Insertion {
        match self.window_insertion_point {
            WindowInsertionPoint::EndOfTree => bsp_tree::Insertion::EndOfTree,
            _ => bsp_tree::Insertion::AtFocus,
        }
    }
}

/// This system's trees parsed into the core's: the boundary where a saved layout (rift's
/// NodeId form) becomes the core's state.
impl BspLayoutSystem {
    /// Every layout as the core holds it, and the windows left out; or why one cannot be held.
    pub(crate) fn core_layouts(&self) -> Result<bsp_tree::Layouts<LayoutId, WindowId>, OldTree> {
        let mut core = bsp_tree::Layouts::default();
        for window in &self.inactive {
            core.set_participation(*window, false);
        }
        for (layout, state) in &self.layouts {
            let mut failed = None;
            core.adopt(layout, |tags| match self.core_node(state.root, tags) {
                Ok(root) => bsp_tree::Layout { root },
                Err(problem) => {
                    failed = Some(problem);
                    bsp_tree::Layout::new(tags)
                }
            });
            if let Some(problem) = failed {
                return Err(problem);
            }
        }
        Ok(core)
    }

    fn core_node(
        &self,
        node: NodeId,
        tags: &mut bsp_tree::Tags,
    ) -> Result<bsp_tree::Node<WindowId>, OldTree> {
        let memory = self.tree.data.selection.memory_of(node);
        match self.kind.get(node).ok_or(OldTree::NodeWithoutKind)? {
            NodeKind::Split { orientation, ratio } => {
                let children: Vec<NodeId> = node.children(&self.tree.map).collect();
                let [first, second] = children.as_slice() else {
                    return Err(OldTree::SplitOf(children.len()));
                };
                let memory = match memory {
                    Some((child, false)) if child == *first => {
                        bsp_tree::Memory::Toward(bsp_tree::Side::First)
                    }
                    Some((child, false)) if child == *second => {
                        bsp_tree::Memory::Toward(bsp_tree::Side::Second)
                    }
                    Some((_, false)) => {
                        return Err(OldTree::SplitRemembersAStranger);
                    }
                    Some((_, true)) | None => bsp_tree::Memory::Stops,
                };
                Ok(bsp_tree::Node::Split(Box::new(bsp_tree::Split {
                    tag: tags.mint(),
                    orientation: match orientation {
                        Orientation::Horizontal => bsp_tree::Orientation::Horizontal,
                        Orientation::Vertical => bsp_tree::Orientation::Vertical,
                    },
                    ratio: bsp_tree::Ratio::clamped(*ratio)
                        .filter(|clamped| clamped.get() == *ratio)
                        .ok_or(OldTree::Ratio(*ratio))?,
                    memory,
                    first: self.core_node(*first, tags)?,
                    second: self.core_node(*second, tags)?,
                })))
            }
            NodeKind::Leaf {
                window,
                fullscreen,
                fullscreen_within_gaps,
                preselected,
            } => {
                if memory.is_some() {
                    return Err(OldTree::LeafWithMemory);
                }
                let fill = match (fullscreen, fullscreen_within_gaps) {
                    (false, false) => bsp_tree::Fill::Tiled,
                    (true, false) => bsp_tree::Fill::Screen,
                    (false, true) => bsp_tree::Fill::WithinGaps,
                    (true, true) => return Err(OldTree::BothFullscreens),
                };
                let content = match (window, self.stacks.get(&node)) {
                    (None, None) if fill == bsp_tree::Fill::Tiled => bsp_tree::Content::Empty,
                    (None, None) => return Err(OldTree::EmptyFilled(fill)),
                    (Some(window), None) => bsp_tree::Content::Window { window: *window, fill },
                    (Some(top), Some(members)) => {
                        let at = members
                            .iter()
                            .position(|member| member == top)
                            .ok_or(OldTree::StackShowsAStranger)?;
                        bsp_tree::Content::Stack {
                            tabs: bsp_tree::Tabs {
                                before: members[..at].to_vec(),
                                top: *top,
                                after: members[at + 1..].to_vec(),
                            },
                            fill,
                        }
                    }
                    (None, Some(_)) => return Err(OldTree::StackShowsNothing),
                };
                Ok(bsp_tree::Node::Leaf(bsp_tree::Leaf {
                    tag: tags.mint(),
                    content,
                    preselect: match preselected {
                        None => bsp_tree::Preselect::Unset,
                        Some(direction) => bsp_tree::Preselect::Toward(core_direction(*direction)),
                    },
                }))
            }
        }
    }
}

/// In test builds every operation the core models runs on both this system and the core's
/// BSP tree (`rift_core::layout::bsp_tree`), starting from this system's state converted: the
/// answers, the trees (tags aside) and the container query (node ids aside) must agree.
#[cfg(test)]
impl BspLayoutSystem {
    /// This system as the core holds it: every layout converted, the windows left out.
    fn core_state(&self) -> bsp_tree::Layouts<LayoutId, WindowId> { self.core_state_of(Scope::All) }

    /// After a test changes the NodeId trees by hand (to stand in for an old saved layout), the
    /// core parses them again, as it would on load.
    fn resync_core(&mut self) { self.core = self.core_state(); }

    /// This system as the core holds it, the layouts in `scope` converted.
    fn core_state_of(&self, scope: Scope) -> bsp_tree::Layouts<LayoutId, WindowId> {
        let mut core = bsp_tree::Layouts::default();
        for window in &self.inactive {
            core.set_participation(*window, false);
        }
        for (layout, state) in self.layouts.iter().filter(|(id, _)| scope.covers(*id)) {
            let root = state.root;
            core.adopt(layout, |tags| bsp_tree::Layout {
                root: self.core_node(root, tags).unwrap_or_else(|problem| {
                    panic!(
                        "the core cannot hold this tree: {problem:?}\n{}",
                        self.draw_tree(layout)
                    )
                }),
            });
        }
        core
    }

    /// This system's state must be `shadow`'s: the same layouts, the same trees (tags aside),
    /// the same windows left out, the same container query (node ids aside).
    fn assert_held_as(
        &self,
        operation: &str,
        scope: Scope,
        shadow: &bsp_tree::Layouts<LayoutId, WindowId>,
    ) {
        let now = self.core_state_of(scope);
        let ids = |layouts: &bsp_tree::Layouts<LayoutId, WindowId>| {
            layouts
                .iter()
                .map(|(id, _)| *id)
                .filter(|id| scope.covers(*id))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(&now),
            ids(shadow),
            "{operation}: the core holds other layouts"
        );
        let left_out =
            |layouts: &bsp_tree::Layouts<LayoutId, WindowId>| match layouts.participation() {
                bsp_tree::Participation::Everyone => Vec::new(),
                bsp_tree::Participation::Except(set) => set.iter().copied().collect(),
            };
        assert_eq!(
            left_out(&now),
            left_out(shadow),
            "{operation}: the core leaves out other windows"
        );
        for (id, tree) in shadow.iter().filter(|(id, _)| scope.covers(**id)) {
            let held = now.layout(*id).expect("the same layouts");
            assert!(
                held.same_as(tree),
                "{operation}: the core's tree differs\n  this system: {held:?}\n  the core:    {tree:?}"
            );
            let container = normalized(self.container_tree(*id));
            let core_container = normalized(container_node(&tree.view()));
            assert_eq!(
                container, core_container,
                "{operation}: the container query differs"
            );
        }
    }
}

/// Which layouts an operation can change: one, or (acting on windows or participation) all.
#[derive(Debug, Clone, Copy)]
enum Scope {
    One(
        #[cfg_attr(
            not(test),
            expect(
                dead_code,
                reason = "only test builds check the layouts an operation touched"
            )
        )]
        LayoutId,
    ),
    All,
}

#[cfg(test)]
impl Scope {
    fn covers(self, layout: LayoutId) -> bool {
        match self {
            Scope::One(one) => one == layout,
            Scope::All => true,
        }
    }
}

/// A window's slot as the engine compares slots: the sides from the root as child positions,
/// then its position in its stack if it is stacked.
fn slot_path(slot: bsp_tree::Slot) -> Vec<usize> {
    let mut path: Vec<usize> = slot
        .path
        .iter()
        .map(|side| match side {
            bsp_tree::Side::First => 0,
            bsp_tree::Side::Second => 1,
        })
        .collect();
    match slot.tab {
        bsp_tree::Tab::Alone => {}
        bsp_tree::Tab::Stacked(position) => path.push(position),
    }
    path
}

fn core_direction(direction: Direction) -> bsp_tree::Direction {
    match direction {
        Direction::Left => bsp_tree::Direction::Left,
        Direction::Right => bsp_tree::Direction::Right,
        Direction::Up => bsp_tree::Direction::Up,
        Direction::Down => bsp_tree::Direction::Down,
    }
}

/// The container query's node for the core's view of a BSP layout, field for field what
/// `container_tree` reports.
fn container_node(view: &bsp_tree::View<WindowId>) -> rift_protocol::ContainerTreeNode {
    let fullscreen = |fill: bsp_tree::Fill| match fill {
        bsp_tree::Fill::Tiled => (false, false),
        bsp_tree::Fill::Screen => (true, false),
        bsp_tree::Fill::WithinGaps => (false, true),
    };
    match &view.kind {
        bsp_tree::ViewKind::Split { orientation, first, second } => {
            rift_protocol::ContainerTreeNode {
                node_id: view.tag.get(),
                node_type: rift_protocol::ContainerNodeType::Container,
                frame: Default::default(),
                layout_kind: Some(rift_protocol::LayoutKind::from(match orientation {
                    bsp_tree::Orientation::Horizontal => Orientation::Horizontal,
                    bsp_tree::Orientation::Vertical => Orientation::Vertical,
                })),
                weight: view.weight,
                window_id: None,
                is_selected: view.selected,
                is_fullscreen: false,
                is_fullscreen_within_gaps: false,
                role: None,
                pending_split: None,
                children: vec![container_node(first), container_node(second)],
            }
        }
        bsp_tree::ViewKind::Leaf { content, preselect } => {
            let (
                node_type,
                layout_kind,
                window_id,
                (is_fullscreen, is_fullscreen_within_gaps),
                children,
            ) = match content {
                bsp_tree::Content::Empty => (
                    rift_protocol::ContainerNodeType::Placeholder,
                    None,
                    None,
                    (false, false),
                    Vec::new(),
                ),
                bsp_tree::Content::Window { window, fill } => (
                    rift_protocol::ContainerNodeType::Window,
                    None,
                    Some((*window).into()),
                    fullscreen(*fill),
                    Vec::new(),
                ),
                bsp_tree::Content::Stack { tabs, fill } => {
                    let (screen, gaps) = fullscreen(*fill);
                    let members = tabs
                        .members()
                        .map(|member| rift_protocol::ContainerTreeNode {
                            node_id: (u64::from(member.pid as u32) << 32)
                                | u64::from(member.idx.get()),
                            node_type: rift_protocol::ContainerNodeType::Window,
                            frame: Default::default(),
                            layout_kind: None,
                            weight: None,
                            window_id: Some(member.into()),
                            is_selected: view.selected && tabs.top == member,
                            is_fullscreen: screen,
                            is_fullscreen_within_gaps: gaps,
                            role: None,
                            pending_split: None,
                            children: Vec::new(),
                        })
                        .collect();
                    (
                        rift_protocol::ContainerNodeType::Container,
                        Some(rift_protocol::LayoutKind::HorizontalStack),
                        None,
                        (screen, gaps),
                        members,
                    )
                }
            };
            rift_protocol::ContainerTreeNode {
                node_id: view.tag.get(),
                node_type,
                frame: Default::default(),
                layout_kind,
                weight: view.weight,
                window_id,
                is_selected: view.selected,
                is_fullscreen,
                is_fullscreen_within_gaps,
                role: None,
                pending_split: match preselect {
                    bsp_tree::Preselect::Unset => None,
                    bsp_tree::Preselect::Toward(direction) => Some(
                        match direction {
                            bsp_tree::Direction::Left => Direction::Left,
                            bsp_tree::Direction::Right => Direction::Right,
                            bsp_tree::Direction::Up => Direction::Up,
                            bsp_tree::Direction::Down => Direction::Down,
                        }
                        .into(),
                    ),
                },
                children,
            }
        }
    }
}

/// The container query with the `node_id` of every split and leaf replaced by the order of its
/// first appearance (depth first): two trees then serialise to the same bytes exactly when they
/// agree in everything but those ids' values. A stack member's id is its window's (pid and
/// index), so it is kept as it is.
#[cfg(test)]
fn normalized(mut node: rift_protocol::ContainerTreeNode) -> rift_protocol::ContainerTreeNode {
    fn walk(node: &mut rift_protocol::ContainerTreeNode, seen: &mut Vec<u64>) {
        let index = seen.iter().position(|id| *id == node.node_id).unwrap_or_else(|| {
            seen.push(node.node_id);
            seen.len() - 1
        });
        node.node_id = index as u64;
        // A stack's members keep their windows' ids.
        if node.layout_kind != Some(rift_protocol::LayoutKind::HorizontalStack) {
            for child in &mut node.children {
                walk(child, seen);
            }
        }
    }
    walk(&mut node, &mut Vec::new());
    node
}

/// The core's BSP tree (`rift_core::layout::bsp_tree`) held to this system: random small steps
/// applied to both, and everything a caller can observe compared after each one.
#[cfg(test)]
mod core_tree_differential {
    use objc2_core_foundation::{CGPoint, CGSize};
    use rift_core::layout::bsp_tree;

    use super::*;
    use crate::common::config::GapSettings;

    /// xorshift64*: deterministic, no dependency.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn below(&mut self, n: u64) -> u64 { self.next() % n }
    }

    fn w(idx: u32) -> WindowId { WindowId::new(1 + (idx % 2) as pid_t, idx) }

    const POOL: u32 = 9;

    fn direction(rng: &mut Rng) -> (Direction, bsp_tree::Direction) {
        match rng.below(4) {
            0 => (Direction::Left, bsp_tree::Direction::Left),
            1 => (Direction::Right, bsp_tree::Direction::Right),
            2 => (Direction::Up, bsp_tree::Direction::Up),
            _ => (Direction::Down, bsp_tree::Direction::Down),
        }
    }

    /// One small step, applied to both; what each answered must agree.
    #[derive(Debug, Clone, Copy)]
    enum Step {
        Add(u32),
        Remove(u32),
        Select(u32),
        Ascend,
        Descend,
        MoveFocus(Direction, bsp_tree::Direction),
        Rekey(u32, u32),
        Participate(u32, bool),
        Fullscreen(bool),
        Rebalance,
        ToggleOrientation,
        Resize(f64, ResizeOrientation),
        Swap(u32, u32),
        MoveSelection(Direction, bsp_tree::Direction),
        Split(bool),
        Join(Direction, bsp_tree::Direction),
        Unjoin,
        ConsumeOrExpel(Direction, bsp_tree::Direction),
        Drop(u32, u32, u8, Direction, bsp_tree::Direction),
        Resized(u32, u8, u8, f64, f64),
        Clone,
    }

    fn step(rng: &mut Rng) -> Step {
        let window = |rng: &mut Rng| 1 + rng.below(u64::from(POOL)) as u32;
        match rng.below(36) {
            0..=4 => Step::Add(window(rng)),
            5 | 6 => Step::Remove(window(rng)),
            7 | 8 => Step::Select(window(rng)),
            9 => Step::Ascend,
            10 => Step::Descend,
            11..=13 => {
                let (old, core) = direction(rng);
                Step::MoveFocus(old, core)
            }
            14 => Step::Rekey(window(rng), window(rng)),
            15 => Step::Participate(window(rng), rng.below(3) != 0),
            16 => Step::Fullscreen(rng.below(2) == 0),
            17 => Step::Rebalance,
            18 => Step::ToggleOrientation,
            21 => Step::Swap(window(rng), window(rng)),
            22 | 23 => {
                let (old, core) = direction(rng);
                Step::MoveSelection(old, core)
            }
            24 => Step::Split(rng.below(2) == 0),
            25 | 26 => {
                let (old, core) = direction(rng);
                Step::Join(old, core)
            }
            27 => Step::Unjoin,
            32 | 33 => Step::Resized(
                window(rng),
                rng.below(3) as u8,
                rng.below(3) as u8,
                (rng.below(1600) as f64) - 50.0,
                (rng.below(1000) as f64) - 50.0,
            ),
            34 => Step::Clone,
            29..=31 => {
                let (old, core) = direction(rng);
                Step::Drop(window(rng), window(rng), rng.below(3) as u8, old, core)
            }
            28 => {
                let (old, core) = direction(rng);
                Step::ConsumeOrExpel(old, core)
            }
            _ => Step::Resize((rng.below(41) as f64 - 20.0) / 20.0, match rng.below(3) {
                0 => ResizeOrientation::Horizontal,
                1 => ResizeOrientation::Vertical,
                _ => ResizeOrientation::Smart,
            }),
        }
    }

    const LAYOUT: u8 = 0;

    fn apply(
        old: &mut BspLayoutSystem,
        layout: LayoutId,
        core: &mut bsp_tree::Layouts<u8, WindowId>,
        insertion: bsp_tree::Insertion,
        step: Step,
    ) -> Result<(), String> {
        let differ = |what: &str, old: &dyn std::fmt::Debug, core: &dyn std::fmt::Debug| {
            Err(format!("{what}: old {old:?}, core {core:?}"))
        };
        match step {
            Step::Add(i) => {
                old.add_window_after_selection(layout, w(i));
                core.add(LAYOUT, w(i), insertion);
            }
            Step::Remove(i) => {
                old.remove_window(w(i));
                core.remove_window(w(i));
            }
            Step::Select(i) => {
                let (a, b) = (old.select_window(layout, w(i)), core.select_window(LAYOUT, w(i)));
                if a != b {
                    return differ("select_window", &a, &b);
                }
            }
            Step::Ascend => {
                let (a, b) = (old.ascend_selection(layout), core.ascend(LAYOUT));
                if a != b {
                    return differ("ascend", &a, &b);
                }
            }
            Step::Descend => {
                let (a, b) = (old.descend_selection(layout), core.descend(LAYOUT));
                if a != b {
                    return differ("descend", &a, &b);
                }
            }
            Step::MoveFocus(direction, core_direction) => {
                let (focus, raise) = old.move_focus(layout, direction);
                let moved = core.move_focus(LAYOUT, core_direction);
                if (focus, raise.clone()) != (moved, moved.into_iter().collect()) {
                    return differ("move_focus", &(focus, raise), &moved);
                }
            }
            Step::Rekey(from, to) => {
                old.replace_window(w(from), w(to));
                core.rekey(w(from), w(to));
            }
            Step::Fullscreen(screen) => {
                let (a, b) = if screen {
                    (
                        old.toggle_fullscreen_of_selection(layout),
                        core.toggle_fullscreen(LAYOUT, bsp_tree::Fullscreen::Screen),
                    )
                } else {
                    (
                        old.toggle_fullscreen_within_gaps_of_selection(layout),
                        core.toggle_fullscreen(LAYOUT, bsp_tree::Fullscreen::WithinGaps),
                    )
                };
                if a != b.into_iter().collect::<Vec<_>>() {
                    return differ("toggle_fullscreen", &a, &b);
                }
            }
            Step::Swap(a, b) => {
                let (x, y) = (
                    old.swap_windows(layout, w(a), w(b)),
                    core.swap(LAYOUT, w(a), w(b)),
                );
                if x != y {
                    return differ("swap", &x, &y);
                }
            }
            Step::MoveSelection(direction, core_direction) => {
                let (x, y) = (
                    old.move_selection(layout, direction),
                    core.move_selection(LAYOUT, core_direction),
                );
                if x != y {
                    return differ("move_selection", &x, &y);
                }
            }
            Step::Split(horizontal) => {
                if horizontal {
                    old.split_selection(layout, LayoutKind::Horizontal);
                    core.split(LAYOUT, bsp_tree::Orientation::Horizontal);
                } else {
                    old.split_selection(layout, LayoutKind::Vertical);
                    core.split(LAYOUT, bsp_tree::Orientation::Vertical);
                }
            }
            Step::Join(direction, core_direction) => {
                old.join_selection_with_direction(layout, direction);
                core.join(LAYOUT, core_direction);
            }
            Step::Unjoin => {
                old.unjoin_selection(layout);
                core.unjoin(LAYOUT);
            }
            Step::ConsumeOrExpel(direction, core_direction) => {
                old.consume_or_expel_selection(layout, direction);
                core.consume_or_expel(LAYOUT, core_direction);
            }
            Step::Drop(source, target, kind, direction, core_direction) => {
                use crate::layout_engine::WindowDropAction as Action;
                let (action, drop) = match kind {
                    0 => (Action::Swap, bsp_tree::Drop::Swap),
                    1 => (Action::Stack, bsp_tree::Drop::Stack),
                    _ => (Action::Insert(direction), bsp_tree::Drop::Insert(core_direction)),
                };
                let (x, y) = (
                    old.apply_window_drop(layout, w(source), w(target), action),
                    core.drop_on(LAYOUT, w(source), w(target), drop),
                );
                if x != y {
                    return differ("drop", &x, &y);
                }
            }
            Step::Resized(i, old_kind, new_kind, width, height) => {
                let screen = CGRect::new(CGPoint::new(0., 25.), CGSize::new(1512., 957.));
                let gaps = GapSettings::default();
                let tiling = compute_tiling_area(screen, &gaps);
                let frame = |kind: u8| match kind {
                    0 => screen,
                    1 => tiling,
                    _ => CGRect::new(CGPoint::new(10., 40.), CGSize::new(width, height)),
                };
                let (from, to) = (
                    frame(old_kind),
                    CGRect::new(CGPoint::new(10., 40.), CGSize::new(height, width)),
                );
                let to = if new_kind < 2 { frame(new_kind) } else { to };
                old.on_window_resized(layout, w(i), from, to, screen, &gaps);
                core.resized(LAYOUT, w(i), bsp_tree::Resized {
                    old: crate::layout_engine::core_rect(from),
                    new: crate::layout_engine::core_rect(to),
                    screen: crate::layout_engine::core_rect(screen),
                    tiling: crate::layout_engine::core_rect(tiling),
                });
            }
            Step::Clone => {
                // The shadow holds the clone to the core's (and every later step checks it).
                let _ = old.clone_layout(layout);
            }
            Step::Rebalance => {
                old.rebalance(layout);
                core.rebalance(LAYOUT);
            }
            Step::ToggleOrientation => {
                old.toggle_tile_orientation(layout);
                core.toggle_orientation(LAYOUT);
            }
            Step::Resize(amount, orientation) => {
                old.resize_selection_by(layout, amount, orientation);
                core.resize(LAYOUT, amount, match orientation {
                    ResizeOrientation::Horizontal => bsp_tree::Along::Horizontal,
                    ResizeOrientation::Vertical => bsp_tree::Along::Vertical,
                    ResizeOrientation::Smart => bsp_tree::Along::Either,
                });
            }
            Step::Participate(i, takes_part) => {
                let (a, b) = (
                    old.set_window_participation(w(i), takes_part),
                    core.set_participation(w(i), takes_part),
                );
                if a != b {
                    return differ("set_participation", &a, &b);
                }
            }
        }
        Ok(())
    }

    /// Everything a caller can ask both, which must agree.
    fn compare(
        old: &BspLayoutSystem,
        layout: LayoutId,
        core: &bsp_tree::Layouts<u8, WindowId>,
    ) -> Result<(), String> {
        let tree = core.layout(LAYOUT).expect("the core's layout");
        let who = core.participation();
        let projection = tree.projection(who);
        let mut problems = Vec::new();
        let mut check = |what: &str, a: String, b: String| {
            if a != b {
                problems.push(format!("{what}:\n  old  {a}\n  core {b}"));
            }
        };
        check(
            "windows",
            format!("{:?}", old.all_windows_in_layout(layout)),
            format!("{:?}", projection.windows),
        );
        check(
            "visible",
            format!("{:?}", old.visible_windows_in_layout(layout)),
            format!("{:?}", projection.visible),
        );
        check(
            "selected",
            format!("{:?}", old.selected_window(layout)),
            format!("{:?}", projection.selected_window),
        );
        check(
            "under selection",
            format!("{:?}", old.visible_windows_under_selection(layout)),
            format!("{:?}", projection.under_focus),
        );
        check(
            "fullscreen",
            format!("{:?}", old.has_any_fullscreen_node(layout)),
            format!("{:?}", projection.fullscreen),
        );
        check("draw_tree", old.draw_tree(layout), tree.draw());
        for (direction, core_direction) in [
            (Direction::Left, bsp_tree::Direction::Left),
            (Direction::Right, bsp_tree::Direction::Right),
            (Direction::Up, bsp_tree::Direction::Up),
            (Direction::Down, bsp_tree::Direction::Down),
        ] {
            check(
                &format!("window_in_direction {direction:?}"),
                format!("{:?}", old.window_in_direction(layout, direction)),
                format!("{:?}", tree.window_in_direction(core_direction, who)),
            );
        }
        for i in 1..=POOL {
            let window = w(i);
            let slot = tree.slot(window).map(|slot| {
                let mut path: Vec<usize> = slot
                    .path
                    .iter()
                    .map(|side| match side {
                        bsp_tree::Side::First => 0,
                        bsp_tree::Side::Second => 1,
                    })
                    .collect();
                match slot.tab {
                    bsp_tree::Tab::Alone => {}
                    bsp_tree::Tab::Stacked(position) => path.push(position),
                }
                path
            });
            check(
                &format!("slot {i}"),
                format!("{:?}", old.window_slot(layout, window)),
                format!("{slot:?}"),
            );
            check(
                &format!("stack {i}"),
                format!("{:?}", old.stack_members(layout, window)),
                format!("{:?}", tree.stack_members(window)),
            );
        }
        let screen = CGRect::new(CGPoint::new(0., 25.), CGSize::new(1512., 957.));
        let gaps = GapSettings::default();
        let frames = old.calculate_layout(
            layout,
            screen,
            0.0,
            &HashMap::default(),
            &gaps,
            Default::default(),
        );
        let core_frames = tree.shape(who, &|_| None).map_or_else(Vec::new, |root| {
            bsp::Bsp {
                screen: crate::layout_engine::core_rect(screen),
                gaps: crate::layout_engine::core_gaps(&gaps),
                root,
                aspect: None,
            }
            .frames()
            .into_iter()
            .map(|(window, frame)| (window, crate::layout_engine::cg_rect(frame)))
            .collect()
        });
        check("frames", format!("{frames:?}"), format!("{core_frames:?}"));
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems.join("\n"))
        }
    }

    #[test]
    fn the_core_tree_answers_what_the_bsp_layout_system_answers() {
        let cases: u64 = std::env::var("RIFT_TREE_CASES").map_or(2000, |n| n.parse().unwrap());
        let steps: usize = std::env::var("RIFT_TREE_STEPS").map_or(60, |n| n.parse().unwrap());
        for seed in 0..cases {
            let mut rng =
                Rng(0x9E37_79B9_7F4A_7C15 ^ (seed + 1).wrapping_mul(0xBF58_476D_1CE4_E5B9));
            let mut old = BspLayoutSystem::default();
            let insertion = if rng.below(2) == 0 {
                bsp_tree::Insertion::AtFocus
            } else {
                old.set_window_insertion_point(WindowInsertionPoint::EndOfTree);
                bsp_tree::Insertion::EndOfTree
            };
            let layout = old.create_layout();
            let mut core = bsp_tree::Layouts::<u8, WindowId>::default();
            core.create(LAYOUT);
            let mut history = Vec::new();
            for _ in 0..steps {
                let step = step(&mut rng);
                history.push(step);
                let outcome = apply(&mut old, layout, &mut core, insertion, step)
                    .and_then(|()| compare(&old, layout, &core));
                if let Err(problem) = outcome {
                    panic!(
                        "seed {seed} ({insertion:?}), after {history:?}:\n{problem}\nold tree:\n{}",
                        old.draw_tree(layout)
                    );
                }
            }
        }
    }

    /// The owner's scripts read `rift-cli query layout` with these patterns (copied from
    /// `apply-display-layouts.sh` and `swap-rotate.sh`): on the core's container tree they
    /// must find what they find on this system's, field order, adjacency and tree order included.
    #[test]
    fn the_owners_script_patterns_read_the_cores_container_tree_as_this_systems() {
        let kind = regex::Regex::new(
            r#""layout_kind":"[a-z_]*","node_id":[0-9]*,"node_type":"container""#,
        )
        .unwrap();
        let ids = regex::Regex::new(r#""window_id":\{"idx":[0-9]*,"pid":[0-9]*\}"#).unwrap();
        let mut old = BspLayoutSystem::default();
        let layout = old.create_layout();
        for i in 1..=5 {
            old.add_window_after_selection(layout, w(i));
        }
        old.join_selection_with_direction(layout, Direction::Left);
        let core = old.core_state();
        let tree = core.layout(layout).expect("the layout");
        assert!(
            tree.stack_members(w(5)).len() > 1,
            "a stack is in the tree: {}",
            old.draw_tree(layout)
        );
        let printed = |node: &rift_protocol::ContainerTreeNode| {
            serde_json::to_value(node).unwrap().to_string()
        };
        let (old_json, core_json) = (
            printed(&old.container_tree(layout)),
            printed(&container_node(&tree.view())),
        );
        // apply-display-layouts.sh: the first container's kind (`head -1 | cut -d'"' -f4`).
        let first_kind =
            |json: &str| kind.find(json).map(|m| m.as_str().split('"').nth(3).unwrap().to_string());
        assert!(
            first_kind(&core_json).is_some(),
            "the pattern finds a container: {core_json}"
        );
        assert_eq!(first_kind(&core_json), first_kind(&old_json));
        assert_eq!(
            kind.find_iter(&core_json).count(),
            kind.find_iter(&old_json).count()
        );
        // swap-rotate.sh: the windows in the order the tree is written out.
        let order =
            |json: &str| ids.find_iter(json).map(|m| m.as_str().to_string()).collect::<Vec<_>>();
        assert_eq!(order(&core_json).len(), 5);
        assert_eq!(order(&core_json), order(&old_json));
    }

    /// Decision (c): what a drag preview's clone costs, the old system's against the core's
    /// plain values. `cargo test --release --lib clone_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn clone_cost() {
        for (layouts, windows) in [(1, 4), (1, 20), (4, 20), (8, 50)] {
            let mut old = BspLayoutSystem::default();
            for l in 0..layouts {
                let layout = old.create_layout_unshadowed();
                for i in 0..windows {
                    old.add_window_after_selection_unshadowed(layout, WindowId::new(1 + l, i + 1));
                }
            }
            let core = old.core_state();
            let rounds = 20_000;
            let time = |f: &dyn Fn()| {
                let start = std::time::Instant::now();
                for _ in 0..rounds {
                    f();
                }
                start.elapsed() / rounds
            };
            let old_clone = time(&|| drop(std::hint::black_box(old.clone())));
            let core_clone = time(&|| drop(std::hint::black_box(core.clone())));
            eprintln!(
                "CLONE {layouts} layouts x {windows} windows: old system {old_clone:?}, core {core_clone:?}"
            );
        }
    }
}
