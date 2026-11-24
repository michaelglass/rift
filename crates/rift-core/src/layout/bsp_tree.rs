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
}

/// One of a split's two children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    First,
    Second,
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
}
