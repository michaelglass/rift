//! A BSP layout system's trees as the core holds them: upstream's (#688) `BspLayoutSystem`, its
//! state and its operations, ported operation by operation. One value per system, as upstream
//! keeps it: every layout's root in one slot table and every node in another, so ids are
//! allocated in one sequence and equal upstream's slotmap ids for the same operations (the
//! platform's shadow compares them directly).
//!
//! Upstream's rules are the spec; where this module's types differ from upstream's (a leaf's
//! fullscreen flags as one [`Fill`], a stack's shown window as [`Tabs::top`]), they hold the
//! states upstream's operations reach.

use std::collections::BTreeSet;

pub use super::bsp::Fill;
use super::bsp::{self as geometry, Bsp};
use super::{Gaps, WindowConstraints};
use crate::plain::Rect;
use crate::slot_table::{SlotKey, SlotTable};

/// Which way a split divides its area: side by side, or one above the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    /// The axis the direction moves along.
    pub fn orientation(self) -> Orientation {
        match self {
            Direction::Left | Direction::Right => Orientation::Horizontal,
            Direction::Up | Direction::Down => Orientation::Vertical,
        }
    }

    /// Whether the direction goes toward a split's first child.
    fn backwards(self) -> bool { matches!(self, Direction::Left | Direction::Up) }

    /// The side a window moved or put this way takes.
    fn side(self) -> Side {
        if self.backwards() {
            Side::First
        } else {
            Side::Second
        }
    }
}

/// One of a split's two children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    First,
    Second,
}

impl Side {
    fn index(self) -> usize {
        match self {
            Side::First => 0,
            Side::Second => 1,
        }
    }
}

/// Where the selection goes when it reaches a split: it stops there, or goes on into a side
/// (upstream's `selected: Option<bool>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Memory {
    Stops,
    Toward(Side),
}

/// A direction the next window put into a leaf splits it toward, if one was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preselect {
    Unset,
    Toward(Direction),
}

/// A stack's windows in order, with the one shown on top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tabs<W> {
    pub before: Vec<W>,
    pub top: W,
    pub after: Vec<W>,
}

impl<W: Copy> Tabs<W> {
    /// The windows in stack order.
    pub fn windows(&self) -> impl Iterator<Item = W> + '_ {
        self.before
            .iter()
            .copied()
            .chain(std::iter::once(self.top))
            .chain(self.after.iter().copied())
    }
}

/// What a leaf holding windows holds: one window, or a stack (of one or more).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Members<W> {
    Window(W),
    Stack(Tabs<W>),
}

impl<W: Copy> Members<W> {
    /// The window shown.
    pub fn top(&self) -> W {
        match self {
            Members::Window(window) => *window,
            Members::Stack(tabs) => tabs.top,
        }
    }

    /// The windows in stack order.
    pub fn windows(&self) -> impl Iterator<Item = W> + '_ {
        let (one, stack) = match self {
            Members::Window(window) => (Some(*window), None),
            Members::Stack(tabs) => (None, Some(tabs.windows())),
        };
        one.into_iter().chain(stack.into_iter().flatten())
    }
}

/// What a leaf holds: nothing (a placeholder), or windows filling their place one way.
#[derive(Debug, Clone, PartialEq)]
pub enum Content<W> {
    Empty,
    Held { members: Members<W>, fill: Fill },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Leaf<W> {
    pub content: Content<W>,
    pub preselect: Preselect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Split<N> {
    pub orientation: Orientation,
    /// The first child's share (upstream keeps it as an `f32`).
    pub ratio: f32,
    pub children: [N; 2],
    pub memory: Memory,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind<N, W> {
    Split(Split<N>),
    Leaf(Leaf<W>),
}

/// A node's place: a layout's root (or a node not placed yet), or a split's child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parent<N> {
    Root,
    Split(N),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node<N, W> {
    pub parent: Parent<N>,
    pub kind: Kind<N, W>,
}

/// A layout: its root node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout<N> {
    pub root: N,
}

/// Where a new window goes when no direction was preselected (upstream's
/// `WindowInsertionPoint`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertionPoint {
    NextToSelection,
    EndOfTree,
}

/// Which of a layout's windows to list (upstream's `WindowQuery`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Query<W> {
    All,
    Visible,
    Selection,
    /// The stack a window is in.
    Stack(W),
}

/// Whether a window takes part in layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Participation {
    TakesPart,
    LeftOut,
}

/// A BSP layout system: every layout's tree, the windows left out of layout, and its settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Tree<L, N, W> {
    layouts: SlotTable<L, Layout<N>>,
    nodes: SlotTable<N, Node<N, W>>,
    left_out: BTreeSet<W>,
    insertion: InsertionPoint,
    /// The width/height a layout's only window is fitted to, if set (finite and positive).
    aspect: Option<f64>,
}

impl<L: SlotKey + Eq, N: SlotKey + Eq, W: Copy + Ord> Tree<L, N, W> {
    pub fn new(insertion: InsertionPoint) -> Self {
        Self {
            layouts: SlotTable::default(),
            nodes: SlotTable::default(),
            left_out: BTreeSet::new(),
            insertion,
            aspect: None,
        }
    }

    /// A system as the platform holds it (each table's slots, vacant ones included, and its
    /// free list).
    pub fn from_parts(
        layouts: SlotTable<L, Layout<N>>,
        nodes: SlotTable<N, Node<N, W>>,
        left_out: BTreeSet<W>,
        insertion: InsertionPoint,
        aspect: Option<f64>,
    ) -> Self {
        Self {
            layouts,
            nodes,
            left_out,
            insertion,
            aspect,
        }
    }

    pub fn layouts(&self) -> &SlotTable<L, Layout<N>> { &self.layouts }

    pub fn nodes(&self) -> &SlotTable<N, Node<N, W>> { &self.nodes }

    pub fn left_out(&self) -> &BTreeSet<W> { &self.left_out }

    pub fn insertion(&self) -> InsertionPoint { self.insertion }

    pub fn aspect(&self) -> Option<f64> { self.aspect }

    pub fn set_window_insertion_point(&mut self, insertion: InsertionPoint) {
        self.insertion = insertion;
    }

    pub fn set_single_window_aspect_ratio(&mut self, ratio: Option<f64>) {
        self.aspect = ratio.filter(|ratio| ratio.is_finite() && *ratio > 0.0);
    }

    pub fn window_participates(&self, window: W) -> bool { !self.left_out.contains(&window) }

    /// Whether the window's participation changed.
    pub fn set_window_participation(&mut self, window: W, participation: Participation) -> bool {
        match participation {
            Participation::TakesPart => self.left_out.remove(&window),
            Participation::LeftOut => self.left_out.insert(window),
        }
    }

    pub fn create_layout(&mut self) -> L {
        let leaf = self.make_leaf(Content::Empty);
        self.layouts.insert(Layout { root: leaf })
    }

    pub fn contains_layout(&self, layout: L) -> bool { self.layouts.contains_key(layout) }

    pub fn clone_layout(&mut self, layout: L) -> L {
        let Some(state) = self.layouts.get(layout).copied() else {
            return self.create_layout();
        };
        let root = self.clone_node(state.root, Parent::Root);
        self.layouts.insert(Layout { root })
    }

    pub fn remove_layout(&mut self, layout: L) {
        if let Some(state) = self.layouts.remove(layout) {
            self.remove_subtree(state.root);
        }
    }

    pub fn selected_window(&self, layout: L) -> Option<W> {
        let state = self.layouts.get(layout)?;
        self.participating_window(self.current_selection(state.root))
    }

    pub fn window_in_direction(&self, layout: L, direction: Direction) -> Option<W> {
        self.window_in_direction_from(self.layouts.get(layout)?.root, direction)
    }

    pub fn contains_window(&self, layout: L, window: W) -> bool {
        self.node_for_window_in_layout(layout, window).is_some()
    }

    pub fn has_any_fullscreen_node(&self, layout: L) -> bool {
        self.layouts
            .get(layout)
            .is_some_and(|state| self.has_fullscreen_in_subtree(state.root))
    }

    /// Whether the layout holds a window of the app `of_app` tells.
    pub fn has_windows_for_app(&self, layout: L, of_app: impl Fn(W) -> bool) -> bool {
        self.layouts
            .get(layout)
            .is_some_and(|state| self.windows_under(state.root).any(of_app))
    }

    pub fn windows(&self, layout: L, query: Query<W>) -> Vec<W> {
        match query {
            Query::All => self.all_windows_in_layout(layout),
            Query::Visible => self.visible_windows_in_layout(layout),
            Query::Selection => self
                .selection_of_layout(layout)
                .and_then(|node| self.participating_window(self.descend_to_leaf(node)))
                .into_iter()
                .collect(),
            Query::Stack(window) => self
                .node_for_window_in_layout(layout, window)
                .and_then(|node| self.stack(node))
                .map(|tabs| tabs.windows().collect())
                .unwrap_or_default(),
        }
    }

    pub fn can_move_window(&self, layout: L, window: W, direction: Direction) -> bool {
        self.window_participates(window)
            && self
                .node_for_window_in_layout(layout, window)
                .is_some_and(|node| self.find_neighbor_leaf(node, direction).is_some())
    }

    /// The window's place: the side taken at each split from the root (0 first, 1 second), then
    /// its position in its leaf's stack, if it is stacked.
    pub fn window_slot(&self, layout: L, window: W) -> Option<Vec<usize>> {
        let root = self.layouts.get(layout)?.root;
        let node = self.node_for_window_in_layout(layout, window)?;
        let mut current = node;
        let mut slot = Vec::new();
        while current != root {
            let parent = self.parent(current)?;
            slot.push(usize::from(self.children(parent)?[1] == current));
            current = parent;
        }
        slot.reverse();
        if let Some(tabs) = self.stack(node) {
            slot.push(tabs.windows().position(|member| member == window)?);
        }
        Some(slot)
    }

    /// A window added to a layout: after the selection, split toward a preselected direction,
    /// or at the end of the tree (upstream's `add_window_after_selection`).
    pub fn add_window(&mut self, layout: L, window: W) {
        if self.contains_window(layout, window) {
            return;
        }
        let Some(state) = self.layouts.get(layout).copied() else {
            return;
        };
        if self.insertion == InsertionPoint::EndOfTree {
            let mut leaf = state.root;
            while let Some([_, right]) = self.children(leaf) {
                leaf = right;
            }
            self.select(leaf);
            self.insert_window_at_selection(layout, window);
            return;
        }
        if !self.smart_insert_window(layout, window) {
            self.insert_window_at_selection(layout, window);
        }
    }

    /// A window gone from every layout.
    pub fn remove_window(&mut self, window: W) {
        self.left_out.remove(&window);
        let layouts: Vec<L> = self.layouts.keys().collect();
        for layout in layouts {
            while self.remove_window_internal(layout, window) {}
        }
    }

    /// A window that took another's place (a native tab, a restore's match), in every layout.
    pub fn replace_window(&mut self, from: W, to: W) {
        if self.left_out.remove(&from) {
            self.left_out.insert(to);
        }
        if from == to {
            return;
        }
        let layouts: Vec<L> = self.layouts.keys().collect();
        for layout in layouts {
            if !self.contains_window(layout, from) {
                continue;
            }
            // A replacement may already be present after restore matching.
            while self.remove_window_internal(layout, to) {}
        }
        for node in self.nodes.values_mut() {
            if let Kind::Leaf(Leaf {
                content: Content::Held { members, .. },
                ..
            }) = &mut node.kind
            {
                members.replace(from, to);
            }
        }
    }

    /// An app gone: its windows (those `of_app` tells) leave every layout.
    pub fn close_app(&mut self, of_app: impl Fn(W) -> bool) {
        self.left_out.retain(|&window| !of_app(window));
        let mut windows: Vec<W> = self
            .layouts
            .values()
            .flat_map(|state| self.windows_under(state.root))
            .filter(|&window| of_app(window))
            .collect();
        windows.sort_unstable();
        windows.dedup();
        for window in windows {
            self.remove_window(window);
        }
    }

    /// A window the user (or its app) resized from `old` to `new`: a frame that enters or leaves
    /// the screen or the tiling area sets or clears its fill; otherwise the nearest split along
    /// a changed axis takes the new share.
    pub fn resize_window(&mut self, layout: L, window: W, resize: Resize) {
        let Resize { old, new, screen, tiling } = resize;
        if !self.window_participates(window) {
            return;
        }
        let Some(node) = self.node_for_window_in_layout(layout, window) else {
            return;
        };
        if let Content::Held { fill, .. } = self.content_mut(node)
            && track_fullscreen_resize(fill, old, new, screen, tiling)
        {
            return;
        }
        let width_changed = (new.w - old.w).abs() > 0.5;
        let height_changed = (new.h - old.h).abs() > 0.5;
        if !width_changed && !height_changed {
            return;
        }
        let mut current = node;
        while let Some(parent) = self.parent(current) {
            let Kind::Split(split) = self.kind_mut(parent) else {
                unreachable!("parent of a BSP node is a split");
            };
            let (old_len, new_len) = match split.orientation {
                Orientation::Horizontal if width_changed => (old.w, new.w),
                Orientation::Vertical if height_changed => (old.h, new.h),
                _ => {
                    current = parent;
                    continue;
                }
            };
            let current_ratio = f64::from(split.ratio);
            let is_first = current == split.children[0];
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
            split.ratio = if is_first {
                raw_ratio as f32
            } else {
                (1.0 - raw_ratio) as f32
            };
            break;
        }
    }

    /// The selection moved (upstream's `focus`).
    pub fn focus(&mut self, layout: L, focus: Focus<W>) -> Focused<W> {
        match focus {
            Focus::Select(window) => Focused::Selection {
                handled: self.select_window(layout, window),
            },
            Focus::Ascend => Focused::Selection { handled: self.ascend(layout) },
            Focus::Descend => Focused::Selection { handled: self.descend(layout) },
            Focus::Direction(direction) => self.move_focus(layout, direction),
        }
    }

    fn select_window(&mut self, layout: L, window: W) -> bool {
        if self.left_out.contains(&window) {
            return false;
        }
        let Some(node) = self.node_for_window_in_layout(layout, window) else {
            return false;
        };
        if let Content::Held { members, .. } = self.content_mut(node) {
            members.focus(window);
        }
        self.select(node);
        true
    }

    fn ascend(&mut self, layout: L) -> bool {
        if let Some(selection) = self.selection_of_layout(layout)
            && let Some(parent) = self.parent(selection)
        {
            let leaf = self.descend_to_leaf(parent);
            self.select(leaf);
            return true;
        }
        false
    }

    fn descend(&mut self, layout: L) -> bool {
        if let Some(selection) = self.selection_of_layout(layout) {
            let leaf = self.descend_to_leaf(selection);
            if leaf != selection {
                self.select(leaf);
                return true;
            }
        }
        false
    }

    fn move_focus(&mut self, layout: L, direction: Direction) -> Focused<W> {
        let Some(selection) = self.selection_of_layout(layout) else {
            return Focused::Movement {
                window: None,
                raise: Vec::new(),
            };
        };
        let leaf = self.descend_to_leaf(selection);
        let Some(next) = self.find_neighbor_leaf(leaf, direction) else {
            return Focused::Movement {
                window: None,
                raise: Vec::new(),
            };
        };
        self.select(next);
        let window = self.participating_window(next);
        // Tiled BSP windows never overlap, so only the focus target is raised.
        Focused::Movement {
            window,
            raise: window.into_iter().collect(),
        }
    }

    /// An edit of a layout (upstream's `edit_layout`).
    pub fn edit(&mut self, layout: L, edit: Edit) -> Edited<W> {
        match edit {
            Edit::Split(SplitKind::Stacked) => Edited::Unsupported,
            Edit::Split(SplitKind::Tiled(orientation)) => {
                let Some(selection) = self.selection_of_layout(layout) else {
                    return Edited::Unchanged;
                };
                let leaf = self.descend_to_leaf(selection);
                self.split_leaf(leaf, orientation, Content::Empty, Side::Second);
                Edited::Changed
            }
            Edit::Fullscreen { within_gaps } => {
                Edited::Raise(self.toggle_fullscreen(layout, within_gaps))
            }
            Edit::Resize { amount, along } => self.resize_selection(layout, amount, along),
            Edit::Rebalance => {
                if let Some(state) = self.layouts.get(layout).copied() {
                    self.rebalance_node(state.root, Orientation::Horizontal);
                }
                Edited::Changed
            }
            Edit::ToggleOrientation => {
                let Some(start) = self.selection_of_layout(layout) else {
                    return Edited::Unchanged;
                };
                let split = self.ancestors(start).find(|&node| self.children(node).is_some());
                if let Some(node) = split
                    && let Kind::Split(split) = self.kind_mut(node)
                {
                    split.orientation = match split.orientation {
                        Orientation::Horizontal => Orientation::Vertical,
                        Orientation::Vertical => Orientation::Horizontal,
                    };
                    return Edited::Changed;
                }
                Edited::Unchanged
            }
        }
    }

    fn resize_selection(&mut self, layout: L, amount: f64, along: ResizeAlong) -> Edited<W> {
        let Some(mut node) = self.selection_of_layout(layout) else {
            return Edited::Unchanged;
        };
        while let Some(parent) = self.parent(node) {
            if let Kind::Split(split) = self.kind_mut(parent)
                && match along {
                    ResizeAlong::Horizontal => split.orientation == Orientation::Horizontal,
                    ResizeAlong::Vertical => split.orientation == Orientation::Vertical,
                    ResizeAlong::Smart => true,
                }
            {
                let delta = (amount as f32) * if node == split.children[0] { 0.5 } else { -0.5 };
                let next = (split.ratio + delta).clamp(0.05, 0.95);
                return Edited::of(std::mem::replace(&mut split.ratio, next) != next);
            }
            node = parent;
        }
        Edited::Unchanged
    }

    // Return the number of tiles sharing the parent's axis; perpendicular splits count as one.
    fn rebalance_node(&mut self, node: N, parent_axis: Orientation) -> usize {
        let Kind::Split(Split {
            orientation,
            children: [first, second],
            ..
        }) = *self.kind(node)
        else {
            return 1;
        };
        let first_span = self.rebalance_node(first, orientation);
        let second_span = self.rebalance_node(second, orientation);
        let span = first_span + second_span;
        let Kind::Split(split) = self.kind_mut(node) else {
            unreachable!("rebalancing children does not replace their parent");
        };
        split.ratio = (first_span as f32 / span as f32).clamp(0.05, 0.95);
        if orientation == parent_axis { span } else { 1 }
    }

    fn toggle_fullscreen(&mut self, layout: L, within_gaps: bool) -> Vec<W> {
        let Some(selection) = self.selection_of_layout(layout) else {
            return Vec::new();
        };
        let leaf = self.descend_to_leaf(selection);
        let active = self.participating_window(leaf);
        let Content::Held { fill, .. } = self.content_mut(leaf) else {
            return Vec::new();
        };
        *fill = toggled(*fill, within_gaps);
        active.into_iter().collect()
    }

    /// A layout as the core's geometry takes it, pruned as upstream's `prepare_geometry` prunes
    /// it (none: nothing takes space): a leaf shows its participating member; a leaf with none
    /// is left out, except an empty leaf while no window is left out of the system; a split
    /// keeps the children that remain.
    pub fn shape(
        &self,
        layout: L,
        screen: Rect,
        gaps: Gaps,
        constraints: impl Fn(W) -> Option<WindowConstraints>,
    ) -> Option<Bsp<W>> {
        Some(Bsp {
            screen,
            gaps,
            root: self.shape_node(self.layouts.get(layout)?.root, &constraints)?,
            aspect: self.aspect,
        })
    }

    fn shape_node(
        &self,
        node: N,
        constraints: &impl Fn(W) -> Option<WindowConstraints>,
    ) -> Option<geometry::Node<W>> {
        match self.kind(node) {
            Kind::Leaf(leaf) => {
                let slot = match self.participating_member(&leaf.content) {
                    Some(window) => geometry::Slot::Window {
                        window,
                        constraints: constraints(window),
                    },
                    None if self.left_out.is_empty() && matches!(leaf.content, Content::Empty) => {
                        geometry::Slot::Empty
                    }
                    None => return None,
                };
                let (fill, stacked) = match &leaf.content {
                    Content::Empty => (Fill::Tiled, false),
                    Content::Held { members, fill } => {
                        (*fill, matches!(members, Members::Stack(_)))
                    }
                };
                Some(geometry::Node::Leaf { slot, fill, stacked })
            }
            Kind::Split(split) => {
                let [first, second] = split.children;
                let children = match (
                    self.shape_node(first, constraints),
                    self.shape_node(second, constraints),
                ) {
                    (None, None) => return None,
                    (Some(only), None) | (None, Some(only)) => {
                        geometry::Children::One(Box::new(only))
                    }
                    (Some(first), Some(second)) => {
                        geometry::Children::Two(Box::new(first), Box::new(second))
                    }
                };
                Some(geometry::Node::Split {
                    horizontal: split.orientation == Orientation::Horizontal,
                    ratio: split.ratio,
                    children,
                })
            }
        }
    }

    fn kind(&self, node: N) -> &Kind<N, W> { &self.nodes[node].kind }

    fn make_node(&mut self, kind: Kind<N, W>) -> N {
        self.nodes.insert(Node { parent: Parent::Root, kind })
    }

    fn make_leaf(&mut self, content: Content<W>) -> N {
        self.make_node(Kind::Leaf(Leaf {
            content,
            preselect: Preselect::Unset,
        }))
    }

    fn children(&self, node: N) -> Option<[N; 2]> {
        match self.kind(node) {
            Kind::Split(split) => Some(split.children),
            Kind::Leaf(_) => None,
        }
    }

    fn clone_node(&mut self, source: N, parent: Parent<N>) -> N {
        let clone = self.nodes.insert(self.nodes[source].clone());
        self.nodes[clone].parent = parent;
        if let Some([first, second]) = self.children(source) {
            let new = [
                self.clone_node(first, Parent::Split(clone)),
                self.clone_node(second, Parent::Split(clone)),
            ];
            if let Kind::Split(split) = &mut self.nodes[clone].kind {
                split.children = new;
            }
        }
        clone
    }

    fn remove_subtree(&mut self, root: N) {
        if let Some(children) = self.children(root) {
            for child in children {
                self.remove_subtree(child);
            }
        }
        self.nodes.remove(root);
    }

    fn participating_member(&self, content: &Content<W>) -> Option<W> {
        let Content::Held { members, .. } = content else {
            return None;
        };
        let top = members.top();
        if self.window_participates(top) {
            Some(top)
        } else {
            members.windows().find(|&window| self.window_participates(window))
        }
    }

    fn parent(&self, node: N) -> Option<N> {
        match self.nodes[node].parent {
            Parent::Root => None,
            Parent::Split(parent) => Some(parent),
        }
    }

    fn first_child(&self, node: N) -> Option<N> { Some(self.children(node)?[0]) }

    fn child_iter(&self, node: N) -> impl DoubleEndedIterator<Item = N> {
        self.children(node).into_iter().flatten()
    }

    fn ancestors(&self, node: N) -> impl Iterator<Item = N> + '_ {
        std::iter::successors(Some(node), |&node| self.parent(node))
    }

    fn preorder(&self, root: N) -> impl Iterator<Item = N> + '_ {
        std::iter::successors(Some(root), move |&node| {
            self.first_child(node).or_else(|| {
                self.ancestors(node)
                    .take_while(|&ancestor| ancestor != root)
                    .find_map(|ancestor| {
                        let children = self.children(self.parent(ancestor)?)?;
                        (children[0] == ancestor).then_some(children[1])
                    })
            })
        })
    }

    fn current_selection(&self, mut node: N) -> N {
        while let Kind::Split(Split {
            children,
            memory: Memory::Toward(side),
            ..
        }) = self.kind(node)
        {
            node = children[side.index()];
        }
        node
    }

    fn content(&self, node: N) -> Option<&Content<W>> {
        match self.kind(node) {
            Kind::Leaf(leaf) => Some(&leaf.content),
            Kind::Split(_) => None,
        }
    }

    fn stack(&self, node: N) -> Option<&Tabs<W>> {
        match self.content(node)? {
            Content::Held {
                members: Members::Stack(tabs), ..
            } => Some(tabs),
            Content::Held { .. } | Content::Empty => None,
        }
    }

    fn participating_window(&self, node: N) -> Option<W> {
        self.participating_member(self.content(node)?)
    }

    fn find_neighbor_leaf(&self, from_leaf: N, direction: Direction) -> Option<N> {
        let mut current = from_leaf;
        while let Some(next) = self.find_structural_neighbor_leaf(current, direction) {
            if self.left_out.is_empty() || self.participating_window(next).is_some() {
                return Some(next);
            }
            current = next;
        }
        None
    }

    fn find_structural_neighbor_leaf(&self, from_leaf: N, direction: Direction) -> Option<N> {
        let backwards = direction.backwards();
        let axis = direction.orientation();
        let mut anchor = 0.5;
        let mut current = from_leaf;
        while let Some(parent) = self.parent(current) {
            let Kind::Split(Split {
                orientation, ratio, children, ..
            }) = *self.kind(parent)
            else {
                unreachable!("a BSP parent is a split")
            };
            let second = children[1] == current;
            if orientation == axis {
                if second == backwards {
                    let mut target = children[usize::from(!second)];
                    while let Kind::Split(Split {
                        orientation, ratio, children, ..
                    }) = *self.kind(target)
                    {
                        let ratio = f64::from(ratio);
                        let second = if orientation == axis {
                            backwards
                        } else if anchor <= ratio {
                            anchor /= ratio;
                            false
                        } else {
                            anchor = (anchor - ratio) / (1.0 - ratio);
                            true
                        };
                        target = children[usize::from(second)];
                    }
                    return Some(target);
                }
            } else {
                let ratio = f64::from(ratio);
                anchor = if second {
                    ratio + anchor * (1.0 - ratio)
                } else {
                    anchor * ratio
                };
            }
            current = parent;
        }
        None
    }

    fn window_in_direction_from(&self, node: N, direction: Direction) -> Option<W> {
        let Some(mut children) = self.children(node) else {
            return self.participating_window(node);
        };
        if direction.backwards() {
            children.swap(0, 1);
        }
        children
            .into_iter()
            .find_map(|child| self.window_in_direction_from(child, direction))
    }

    fn node_for_window_in_layout(&self, layout: L, window: W) -> Option<N> {
        self.find_window(self.layouts.get(layout)?.root, window)
    }

    fn find_window(&self, node: N, window: W) -> Option<N> {
        match self.kind(node) {
            Kind::Split(Split { children: [first, second], .. }) => {
                self.find_window(*first, window).or_else(|| self.find_window(*second, window))
            }
            Kind::Leaf(leaf) => match &leaf.content {
                Content::Held { members, .. } => {
                    members.windows().any(|member| member == window).then_some(node)
                }
                Content::Empty => None,
            },
        }
    }

    fn descend_to_leaf(&self, mut node: N) -> N {
        if !self.left_out.is_empty()
            && let Some(active) = self
                .preorder(node)
                .find(|&candidate| self.participating_window(candidate).is_some())
        {
            return active;
        }
        while let Some(child) = self.first_child(node) {
            node = child;
        }
        node
    }

    fn windows_under(&self, root: N) -> impl Iterator<Item = W> + '_ {
        self.preorder(root)
            .filter_map(|node| match self.content(node)? {
                Content::Held { members, .. } => Some(members.windows()),
                Content::Empty => None,
            })
            .flatten()
    }

    fn has_fullscreen_in_subtree(&self, node: N) -> bool {
        match self.kind(node) {
            Kind::Leaf(leaf) => {
                self.participating_window(node).is_some()
                    && matches!(leaf.content, Content::Held {
                        fill: Fill::Screen | Fill::WithinGaps,
                        ..
                    })
            }
            Kind::Split(_) => {
                self.child_iter(node).any(|child| self.has_fullscreen_in_subtree(child))
            }
        }
    }

    fn selection_of_layout(&self, layout: L) -> Option<N> {
        let root = self.layouts.get(layout)?.root;
        let selected = self.current_selection(root);
        if self.left_out.is_empty() || self.participating_window(selected).is_some() {
            return Some(selected);
        }
        let first_active =
            self.preorder(root).find(|&node| self.participating_window(node).is_some())?;
        if matches!(self.content(selected), Some(Content::Held { .. })) {
            Some(first_active)
        } else {
            Some(selected)
        }
    }

    fn all_windows_in_layout(&self, layout: L) -> Vec<W> {
        self.layouts
            .get(layout)
            .into_iter()
            .flat_map(|state| self.windows_under(state.root))
            .collect()
    }

    fn visible_windows_in_layout(&self, layout: L) -> Vec<W> {
        self.layouts
            .get(layout)
            .into_iter()
            .flat_map(|state| self.preorder(state.root))
            .filter_map(|node| self.participating_window(node))
            .collect()
    }

    fn kind_mut(&mut self, node: N) -> &mut Kind<N, W> { &mut self.nodes[node].kind }

    fn content_mut(&mut self, leaf: N) -> &mut Content<W> {
        let Kind::Leaf(leaf) = self.kind_mut(leaf) else {
            unreachable!("expected a BSP leaf");
        };
        &mut leaf.content
    }

    fn select(&mut self, mut node: N) {
        if let Kind::Split(split) = self.kind_mut(node) {
            split.memory = Memory::Stops;
        }
        while let Some(parent) = self.parent(node) {
            let Kind::Split(split) = self.kind_mut(parent) else {
                unreachable!("parent of a BSP node is a split");
            };
            split.memory = Memory::Toward(if split.children[1] == node {
                Side::Second
            } else {
                Side::First
            });
            node = parent;
        }
    }

    fn set_split(&mut self, node: N, orientation: Orientation, children: [N; 2]) {
        self.nodes[node].kind = Kind::Split(Split {
            orientation,
            ratio: 0.5,
            children,
            memory: Memory::Stops,
        });
        for child in children {
            self.nodes[child].parent = Parent::Split(node);
        }
    }

    fn smart_insert_window(&mut self, layout: L, window: W) -> bool {
        let Some(selection) = self.selection_of_layout(layout) else {
            return false;
        };
        let leaf = self.descend_to_leaf(selection);
        let Kind::Leaf(Leaf {
            preselect: Preselect::Toward(direction),
            ..
        }) = *self.kind(leaf)
        else {
            return false;
        };
        self.split_leaf(
            leaf,
            direction.orientation(),
            Content::window(window),
            direction.side(),
        );
        true
    }

    /// The leaf becomes a split of its windows (tiled, no preselection) and `new`, on `side`;
    /// the new leaf is selected.
    fn split_leaf(&mut self, leaf: N, orientation: Orientation, new: Content<W>, side: Side) {
        let content = std::mem::replace(self.content_mut(leaf), Content::Empty);
        let existing = self.make_leaf(content.tiled());
        let new = self.make_leaf(new);
        let children = match side {
            Side::First => [new, existing],
            Side::Second => [existing, new],
        };
        self.set_split(leaf, orientation, children);
        self.select(new);
    }

    fn orientation_for_leaf(&self, node: N) -> Orientation {
        if self.ancestors(node).count() % 2 == 1 {
            Orientation::Horizontal
        } else {
            Orientation::Vertical
        }
    }

    // Remove one child and promote the other into the parent's stable slot.
    fn detach_child(&mut self, node: N) {
        let parent = self.parent(node).expect("cannot detach BSP root");
        let children = self.children(parent).expect("a BSP parent is a split");
        let sibling = children[usize::from(children[0] == node)];
        let sibling = self.nodes.remove(sibling).expect("a split's children are live");
        self.nodes[node].parent = Parent::Root;
        let parent_node = &mut self.nodes[parent];
        parent_node.kind = sibling.kind;
        // The former parent's selection pointed to one of the two removed child slots.
        if let Kind::Split(split) = &mut parent_node.kind {
            split.memory = Memory::Stops;
        }
        if let Some(children) = self.children(parent) {
            for child in children {
                self.nodes[child].parent = Parent::Split(parent);
            }
        }
    }

    fn insert_window_at_selection(&mut self, layout: L, window: W) {
        let Some(state) = self.layouts.get(layout).copied() else {
            return;
        };
        let selection = self
            .selection_of_layout(layout)
            .unwrap_or_else(|| self.current_selection(state.root));
        let leaf = self.descend_to_leaf(selection);
        if leaf != selection {
            self.select(leaf);
        }
        match self.content_mut(leaf) {
            content @ Content::Empty => *content = Content::window(window),
            Content::Held { .. } => {
                let orientation = self.orientation_for_leaf(leaf);
                self.split_leaf(leaf, orientation, Content::window(window), Side::Second);
            }
        }
    }

    /// Remove an occurrence and report progress so callers can clear legacy duplicates.
    fn remove_window_internal(&mut self, layout: L, window: W) -> bool {
        let Some(node) = self.node_for_window_in_layout(layout, window) else {
            return false;
        };
        self.remove_member(layout, node, window);
        true
    }

    fn remove_member(&mut self, layout: L, node: N, window: W) {
        let content = self.content_mut(node);
        content.remove(window);
        if let Content::Held { .. } = content {
            self.select(node);
            return;
        }
        if self.parent(node).is_some() {
            self.detach_child(node);
            self.nodes.remove(node);
        }
        let selection = self.current_selection(self.layouts[layout].root);
        let leaf = self.descend_to_leaf(selection);
        self.select(leaf);
    }
}

impl<W: Copy + PartialEq> Content<W> {
    /// A leaf's content for one window, tiled.
    pub fn window(window: W) -> Self {
        Content::Held {
            members: Members::Window(window),
            fill: Fill::Tiled,
        }
    }

    /// The same windows, tiled (a split's new leaf keeps its windows, not its fill).
    fn tiled(self) -> Self {
        match self {
            Content::Empty => Content::Empty,
            Content::Held { members, .. } => Content::Held { members, fill: Fill::Tiled },
        }
    }

    /// Without `window`: a stack keeps its other windows, the last on top, and its fill; a
    /// leaf left with none is empty.
    fn remove(&mut self, window: W) {
        if let Content::Held {
            members: Members::Stack(tabs),
            fill,
        } = self
        {
            let rest: Vec<W> = tabs.windows().filter(|&member| member != window).collect();
            if let Some(tabs) = Tabs::last_on_top(rest) {
                *self = Content::Held {
                    members: Members::Stack(tabs),
                    fill: *fill,
                };
                return;
            }
        }
        *self = Content::Empty;
    }
}

impl<W: Copy + PartialEq> Members<W> {
    fn replace(&mut self, from: W, to: W) {
        let swap = |member: &mut W| {
            if *member == from {
                *member = to;
            }
        };
        match self {
            Members::Window(window) => swap(window),
            Members::Stack(tabs) => {
                tabs.before.iter_mut().for_each(swap);
                swap(&mut tabs.top);
                tabs.after.iter_mut().for_each(swap);
            }
        }
    }
}

impl<W> Tabs<W> {
    /// `windows` as a stack with the last on top (none: no windows).
    fn last_on_top(mut windows: Vec<W>) -> Option<Self> {
        let top = windows.pop()?;
        Some(Tabs {
            before: windows,
            top,
            after: Vec::new(),
        })
    }
}

/// A native resize, with the screen and its tiling area it happened on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resize {
    pub old: Rect,
    pub new: Rect,
    pub screen: Rect,
    pub tiling: Rect,
}

/// Whether a native resize entered or left a fullscreen frame (upstream's
/// `track_fullscreen_resize`, on the leaf's one fill).
fn track_fullscreen_resize(
    fill: &mut Fill,
    old: Rect,
    new: Rect,
    screen: Rect,
    tiling: Rect,
) -> bool {
    if new == screen {
        *fill = Fill::Screen;
    } else if old == screen {
        if *fill == Fill::Screen {
            *fill = Fill::Tiled;
        }
    } else if new == tiling {
        *fill = Fill::WithinGaps;
    } else if old == tiling {
        if *fill == Fill::WithinGaps {
            *fill = Fill::Tiled;
        }
    } else {
        return false;
    }
    true
}

/// How the selection moves (upstream's `Focus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus<W> {
    Select(W),
    Ascend,
    Descend,
    Direction(Direction),
}

/// What a focus did (upstream's `FocusResult`, by how it was made): a selection change says
/// whether it was handled; a movement names the window it reached, if any, and those to raise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Focused<W> {
    Selection { handled: bool },
    Movement { window: Option<W>, raise: Vec<W> },
}

impl<W: Copy + PartialEq> Members<W> {
    /// `window` on top: a stack moves it to the end, the others keeping their order.
    fn focus(&mut self, window: W) {
        if let Members::Stack(tabs) = self {
            let rest: Vec<W> = tabs.windows().filter(|&member| member != window).collect();
            *tabs = Tabs {
                before: rest,
                top: window,
                after: Vec::new(),
            };
        }
    }
}

/// Which split a resize changes (upstream's `ResizeOrientation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeAlong {
    Horizontal,
    Vertical,
    /// The nearest split, whichever its orientation.
    Smart,
}

/// What splitting the selection makes (upstream's `LayoutKind`): a tiled split, or a stacked
/// one (which BSP does not do).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitKind {
    Tiled(Orientation),
    Stacked,
}

/// An edit of a layout (upstream's `LayoutEdit`, the ones its BSP system reads).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Edit {
    Split(SplitKind),
    Fullscreen { within_gaps: bool },
    Resize { amount: f64, along: ResizeAlong },
    Rebalance,
    ToggleOrientation,
}

/// What an edit did (upstream's `EditResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edited<W> {
    Unsupported,
    Unchanged,
    Changed,
    Raise(Vec<W>),
}

impl<W> Edited<W> {
    fn of(changed: bool) -> Self {
        if changed {
            Edited::Changed
        } else {
            Edited::Unchanged
        }
    }
}

/// A fill toggled (upstream's `toggle_fullscreen_mode`): the mode asked for is turned off if it
/// is on, else on (replacing the other).
fn toggled(fill: Fill, within_gaps: bool) -> Fill {
    match (fill, within_gaps) {
        (Fill::WithinGaps, true) | (Fill::Screen, false) => Fill::Tiled,
        (_, true) => Fill::WithinGaps,
        (_, false) => Fill::Screen,
    }
}
