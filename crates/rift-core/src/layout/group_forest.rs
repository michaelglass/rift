//! The group forest as the core holds it: the tree upstream's (#688) traditional and master-stack
//! layout systems share (`GroupLayouts`, `model::tree`), its state and its operations, ported
//! operation by operation, and each system's own operations over it ([`Traditional`],
//! [`MasterStack`]). One value per system, as upstream keeps it: every layout's root in one slot
//! table and every node in another, so ids are allocated in one sequence and equal upstream's
//! slotmap ids for the same operations.
//!
//! Upstream's rules are the spec. Where this module's types differ from upstream's they hold the
//! states upstream's operations reach (the QA lessons): a container's children are an ordered
//! list of ids (no sibling links); its selection memory names one of them ([`Memory`]); a region
//! pair's primary is read from the children's order ([`Members::Regions`]); each pair of
//! fullscreen flags is one [`Fill`]; a layout kind lives only on containers ([`Kind`]), never on
//! a window; a node's size is where it is stored ([`Child`]), so a root has none; a window is
//! always a container's child ([`Body::Window`] holds its [`Child`] place, so no window is a
//! root, not even while it is being placed); the window index is derived from the leaves,
//! ordered by when each window entered its layout ([`Body::Window`]'s stamp, the one fact the
//! index holds that the tree does not).

use std::collections::{BTreeMap, BTreeSet};
use std::ops::ControlFlow;

pub use super::bsp_tree::{
    Direction, Edited, Focus, Focused, InsertionPoint, Orientation, Participation, Query, Resize,
    ResizeAlong, Side,
};
pub use super::stack_tree::{Beyond, DefaultOrientation};
pub use super::traditional::Fill;
use super::traditional::{self as geometry, StackLine};
use super::{Gaps, WindowConstraints};
use crate::plain::Rect;
use crate::slot_table::{SlotKey, SlotSet, SlotTable};

mod floating_system;
mod views;
pub use floating_system::{FloatingSystem, Geometry};
pub use views::{Branch, ContainerView, Drawn, Held, Mark, Role, Row, render};

/// What a container does with its children: lays them out side by side along an axis, or
/// stacks them (one shown) offset along an axis. Upstream's `LayoutKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Split(Orientation),
    Stack(Orientation),
}

impl Kind {
    /// The kind a node is made with (upstream's default `LayoutKind`, `Horizontal`).
    pub const NEW: Kind = Kind::Split(Orientation::Horizontal);

    pub fn orientation(self) -> Orientation {
        match self {
            Kind::Split(orientation) | Kind::Stack(orientation) => orientation,
        }
    }

    pub fn is_stacked(self) -> bool { matches!(self, Kind::Stack(_)) }
}

/// Where a container's selection goes: nowhere remembered, on into a child, or it stops at the
/// container while remembering the child it came through (upstream's `SelectionInfo`, its
/// `selected_child` and `stop_here`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Memory<N> {
    Unset,
    Toward(N),
    Stops(N),
}

impl<N: Copy> Memory<N> {
    /// The child remembered, whether or not the selection stops here.
    pub fn last(self) -> Option<N> {
        match self {
            Memory::Unset => None,
            Memory::Toward(child) | Memory::Stops(child) => Some(child),
        }
    }

    /// The child the selection goes on into.
    pub fn local(self) -> Option<N> {
        match self {
            Memory::Toward(child) => Some(child),
            Memory::Unset | Memory::Stops(_) => None,
        }
    }

    /// The same memory on another child.
    fn on(self, child: N) -> Memory<N> {
        match self {
            Memory::Stops(_) => Memory::Stops(child),
            Memory::Unset | Memory::Toward(_) => Memory::Toward(child),
        }
    }
}

/// A container's children: an ordered list, or a master-stack layout's two regions (in order;
/// `primary` says which is the master region). A region pair's children never change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Members<N> {
    Children(Vec<N>),
    Regions { order: [N; 2], primary: Side },
}

impl<N: Copy> Members<N> {
    pub fn list(&self) -> &[N] {
        match self {
            Members::Children(children) => children,
            Members::Regions { order, .. } => order,
        }
    }

    /// The (primary, secondary) regions of a region pair.
    pub fn regions(&self) -> Option<(N, N)> {
        match self {
            Members::Children(_) => None,
            Members::Regions {
                order: [first, second],
                primary: Side::First,
            } => Some((*first, *second)),
            Members::Regions {
                order: [first, second],
                primary: Side::Second,
            } => Some((*second, *first)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container<N> {
    pub kind: Kind,
    pub members: Members<N>,
    pub memory: Memory<N>,
}

/// What a node is, with its place: a window (always a container's child; with the stamp taken
/// when the window entered this layout, which orders the window index), or a container (a
/// layout's root or a child).
#[derive(Debug, Clone, PartialEq)]
pub enum Body<N, W> {
    Window {
        at: Child<N>,
        window: W,
        stamp: u64,
    },
    Container {
        place: Place<N>,
        container: Container<N>,
    },
}

/// A child's place: its container, and its size among its siblings (upstream keeps it as an
/// `f32`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Child<N> {
    pub parent: N,
    pub size: f32,
}

/// A container's place: a layout's root (or a container no layout reaches), or a child.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place<N> {
    Root,
    Child(Child<N>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node<N, W> {
    pub fill: Fill,
    pub body: Body<N, W>,
}

/// Where a node goes among a container's children (upstream's `Placement`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placement<N> {
    Back(N),
    Before(N),
    After(N),
}

/// How far a walk over a container's children goes (upstream's `BranchLimit`).
#[derive(Clone, Copy)]
enum BranchLimit {
    All,
    First,
    FirstVisible,
}

impl<N: Copy, W: Copy> Node<N, W> {
    /// The node's place (a window's is always a child's).
    pub fn place(&self) -> Place<N> {
        match self.body {
            Body::Window { at, .. } => Place::Child(at),
            Body::Container { place, .. } => place,
        }
    }

    /// The node's place among its siblings, a root having none.
    pub fn child_mut(&mut self) -> Option<&mut Child<N>> {
        match &mut self.body {
            Body::Window { at, .. } | Body::Container { place: Place::Child(at), .. } => Some(at),
            Body::Container { place: Place::Root, .. } => None,
        }
    }

    /// The node on other keys (`node`) and windows (`window`).
    pub fn map<M, V>(&self, node: impl Fn(N) -> M, window: impl Fn(W) -> V) -> Node<M, V> {
        let child = |at: Child<N>| Child {
            parent: node(at.parent),
            size: at.size,
        };
        Node {
            fill: self.fill,
            body: match &self.body {
                Body::Window { at, window: held, stamp } => Body::Window {
                    at: child(*at),
                    window: window(*held),
                    stamp: *stamp,
                },
                Body::Container { place, container } => Body::Container {
                    place: match *place {
                        Place::Root => Place::Root,
                        Place::Child(at) => Place::Child(child(at)),
                    },
                    container: Container {
                        kind: container.kind,
                        members: match &container.members {
                            Members::Children(children) => Members::Children(
                                children.iter().map(|&child| node(child)).collect(),
                            ),
                            Members::Regions { order, primary } => Members::Regions {
                                order: order.map(&node),
                                primary: *primary,
                            },
                        },
                        memory: match container.memory {
                            Memory::Unset => Memory::Unset,
                            Memory::Toward(child) => Memory::Toward(node(child)),
                            Memory::Stops(child) => Memory::Stops(node(child)),
                        },
                    },
                },
            },
        }
    }
}

/// What a window dropped onto another does, of the actions that reach the forest (upstream's
/// `WindowDropAction` without `Move` and `ConsumeOrExpel`, which `LayoutSystem::edit` turns into
/// their edits first).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropAction {
    Swap,
    Stack,
    Insert(Direction),
}

/// Which stacks a layout pass gives a group bar: every one, or those along the selection path
/// (upstream's `selection_path_only`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bars {
    Every,
    SelectionPath,
}

/// A stack's group bar (upstream's `GroupContainerInfo`): the stack, its orientation, its frame,
/// a window for each child holding one that takes part, and which of them is the focused child's.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupBar<N, W> {
    pub stack: N,
    pub orientation: Orientation,
    pub frame: Rect,
    pub windows: Vec<W>,
    pub selected: usize,
}

/// A layout pass with its group bars: each window's frame and each bar, in tree order.
#[derive(Debug, Clone, PartialEq)]
pub struct Projected<N, W> {
    pub frames: Vec<(W, Rect)>,
    pub bars: Vec<GroupBar<N, W>>,
}

/// What a tiled layout is laid out with besides its tree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pass {
    pub screen: Rect,
    pub gaps: Gaps,
    pub stack_offset: f64,
    pub stack_line: StackLine,
}

/// A group forest: every layout's root, every node, and the windows left out of layout.
#[derive(Debug, Clone, PartialEq)]
pub struct Forest<L, N, W> {
    layouts: SlotTable<L, N>,
    nodes: SlotTable<N, Node<N, W>>,
    left_out: BTreeSet<W>,
    /// The stamp the next window to enter a layout takes.
    next_stamp: u64,
}

impl<L: SlotKey + Eq, N: SlotKey + Eq, W: Copy + Ord> Default for Forest<L, N, W> {
    fn default() -> Self {
        Self {
            layouts: SlotTable::default(),
            nodes: SlotTable::default(),
            left_out: BTreeSet::new(),
            next_stamp: 0,
        }
    }
}

fn opposite(direction: Direction) -> Direction {
    match direction {
        Direction::Left => Direction::Right,
        Direction::Right => Direction::Left,
        Direction::Up => Direction::Down,
        Direction::Down => Direction::Up,
    }
}

fn backwards(direction: Direction) -> bool { matches!(direction, Direction::Left | Direction::Up) }

fn perpendicular(orientation: Orientation) -> Orientation {
    match orientation {
        Orientation::Horizontal => Orientation::Vertical,
        Orientation::Vertical => Orientation::Horizontal,
    }
}

/// A new stack's or split's orientation from the container's (upstream's `group_orientation`).
fn group_orientation(base: Orientation, configured: DefaultOrientation) -> Orientation {
    match configured {
        DefaultOrientation::Same => base,
        DefaultOrientation::Perpendicular => perpendicular(base),
        DefaultOrientation::Horizontal => Orientation::Horizontal,
        DefaultOrientation::Vertical => Orientation::Vertical,
    }
}

/// A fill toggled (upstream's `toggle_fullscreen_mode`), and whether a fullscreen mode is on.
fn toggled(fill: Fill, within_gaps: bool) -> Fill {
    match (fill, within_gaps) {
        (Fill::WithinGaps, true) | (Fill::Screen, false) => Fill::Tiled,
        (_, true) => Fill::WithinGaps,
        (_, false) => Fill::Screen,
    }
}

/// A fill after a native resize, if it entered or left a fullscreen frame (upstream's
/// `track_fullscreen_resize`).
fn resized(fill: Fill, Resize { old, new, screen, tiling }: Resize) -> Option<Fill> {
    if new == screen {
        Some(Fill::Screen)
    } else if old == screen {
        Some(if fill == Fill::Screen {
            Fill::Tiled
        } else {
            fill
        })
    } else if new == tiling {
        Some(Fill::WithinGaps)
    } else if old == tiling {
        Some(if fill == Fill::WithinGaps {
            Fill::Tiled
        } else {
            fill
        })
    } else {
        None
    }
}

/// The tree: nodes, their places and contents, and the edits upstream's `Tree` and `NodeMap`
/// make.
impl<L: SlotKey + Eq, N: SlotKey + Eq, W: Copy + Ord> Forest<L, N, W> {
    pub fn node(&self, node: N) -> &Node<N, W> { &self.nodes[node] }

    pub fn layouts(&self) -> impl Iterator<Item = (L, N)> + '_ {
        self.layouts.iter().map(|(layout, &root)| (layout, root))
    }

    pub fn nodes(&self) -> &SlotTable<N, Node<N, W>> { &self.nodes }

    pub fn layout_table(&self) -> &SlotTable<L, N> { &self.layouts }

    pub fn left_out(&self) -> &BTreeSet<W> { &self.left_out }

    /// A forest as the platform holds it, the stamp the next window to enter a layout takes
    /// included (it counts past leaves since removed, so the leaves cannot tell it).
    pub fn from_parts(
        layouts: SlotTable<L, N>,
        nodes: SlotTable<N, Node<N, W>>,
        left_out: BTreeSet<W>,
        next_stamp: u64,
    ) -> Self {
        Self {
            layouts,
            nodes,
            left_out,
            next_stamp,
        }
    }

    /// A forest read from a form that keeps no stamp counter (upstream's saved form, whose load
    /// stamps the leaves in the index's order): the next stamp follows the highest a window
    /// holds.
    pub fn from_leaves(
        layouts: SlotTable<L, N>,
        nodes: SlotTable<N, Node<N, W>>,
        left_out: BTreeSet<W>,
    ) -> Self {
        let next_stamp = nodes
            .values()
            .filter_map(|node| match node.body {
                Body::Window { stamp, .. } => Some(stamp + 1),
                Body::Container { .. } => None,
            })
            .max()
            .unwrap_or(0);
        Self::from_parts(layouts, nodes, left_out, next_stamp)
    }

    /// The stamp the next window to enter a layout takes.
    pub fn next_stamp(&self) -> u64 { self.next_stamp }

    pub fn root(&self, layout: L) -> N { self.layouts[layout] }

    /// A node's place (a window's is always a child's).
    pub fn place(&self, node: N) -> Option<Place<N>> { Some(self.nodes.get(node)?.place()) }

    pub fn parent(&self, node: N) -> Option<N> {
        match self.place(node)? {
            Place::Root => None,
            Place::Child(at) => Some(at.parent),
        }
    }

    /// A node's size among its siblings (a root's reads as 0, as upstream's root keeps).
    pub fn size(&self, node: N) -> f32 {
        match self.place(node) {
            Some(Place::Child(at)) => at.size,
            Some(Place::Root) | None => 0.0,
        }
    }

    fn set_size(&mut self, node: N, size: f32) {
        if let Some(at) = self.nodes[node].child_mut() {
            at.size = size;
        }
    }

    pub fn window(&self, node: N) -> Option<W> {
        match self.nodes.get(node)?.body {
            Body::Window { window, .. } => Some(window),
            Body::Container { .. } => None,
        }
    }

    fn container(&self, node: N) -> Option<&Container<N>> {
        match &self.nodes.get(node)?.body {
            Body::Container { container, .. } => Some(container),
            Body::Window { .. } => None,
        }
    }

    fn container_mut(&mut self, node: N) -> &mut Container<N> {
        match &mut self.nodes[node].body {
            Body::Container { container, .. } => container,
            Body::Window { .. } => unreachable!("a window has no children"),
        }
    }

    /// A container's kind (a window has none).
    pub fn kind(&self, node: N) -> Option<Kind> { self.container(node).map(|c| c.kind) }

    /// The kind of a node that is a container by construction (a parent, a root).
    fn container_kind(&self, node: N) -> Kind {
        self.kind(node).expect("a container by construction")
    }

    fn set_kind(&mut self, node: N, kind: Kind) { self.container_mut(node).kind = kind; }

    fn is_stacked(&self, node: N) -> bool { self.kind(node).is_some_and(Kind::is_stacked) }

    fn is_region_pair(&self, node: N) -> bool {
        self.container(node)
            .is_some_and(|c| matches!(c.members, Members::Regions { .. }))
    }

    /// A master-stack layout's (primary, secondary) regions.
    pub fn regions(&self, root: N) -> Option<(N, N)> { self.container(root)?.members.regions() }

    pub fn children(&self, node: N) -> &[N] {
        self.container(node).map_or(&[], |c| c.members.list())
    }

    fn position(&self, node: N) -> Option<(N, usize)> {
        let parent = self.parent(node)?;
        let at = self.children(parent).iter().position(|&c| c == node).expect("a child");
        Some((parent, at))
    }

    fn next_sibling(&self, node: N) -> Option<N> {
        let (parent, at) = self.position(node)?;
        self.children(parent).get(at + 1).copied()
    }

    fn prev_sibling(&self, node: N) -> Option<N> {
        let (parent, at) = self.position(node)?;
        at.checked_sub(1).map(|at| self.children(parent)[at])
    }

    fn first_child(&self, node: N) -> Option<N> { self.children(node).first().copied() }

    fn last_child(&self, node: N) -> Option<N> { self.children(node).last().copied() }

    fn memory(&self, node: N) -> Memory<N> {
        self.container(node).map_or(Memory::Unset, |c| c.memory)
    }

    fn set_memory(&mut self, node: N, memory: Memory<N>) {
        self.container_mut(node).memory = memory;
    }

    /// The node and its ancestors, nearest first.
    pub fn ancestors(&self, node: N) -> impl Iterator<Item = N> + '_ {
        std::iter::successors(Some(node), move |&node| self.parent(node))
    }

    fn is_within(&self, node: N, ancestor: N) -> bool {
        self.ancestors(node).any(|a| a == ancestor)
    }

    fn common_ancestor(&self, a: N, b: N) -> Option<N> {
        if a == b {
            return Some(a);
        }
        let (depth_a, depth_b) = (self.ancestors(a).count(), self.ancestors(b).count());
        self.ancestors(a)
            .skip(depth_a.saturating_sub(depth_b))
            .zip(self.ancestors(b).skip(depth_b.saturating_sub(depth_a)))
            .find_map(|(left, right)| (left == right).then_some(left))
    }

    fn last_descendant(&self, mut node: N) -> N {
        while let Some(last) = self.last_child(node) {
            node = last;
        }
        node
    }

    fn next_in_subtree(&self, node: N, root: N) -> Option<N> {
        self.first_child(node).or_else(|| {
            self.ancestors(node)
                .take_while(|&n| n != root)
                .find_map(|n| self.next_sibling(n))
        })
    }

    fn prev_in_subtree(&self, node: N, root: N) -> Option<N> {
        if node == root {
            return None;
        }
        self.prev_sibling(node)
            .map(|n| self.last_descendant(n))
            .or_else(|| self.parent(node))
    }

    fn step(&self, node: N, root: N, forward: bool) -> Option<N> {
        if forward {
            self.next_in_subtree(node, root)
        } else {
            self.prev_in_subtree(node, root)
        }
    }

    /// The subtree in preorder (children read from their lists, not stepped through sibling by
    /// sibling: a sibling step searches its parent's list).
    fn preorder(&self, root: N) -> impl Iterator<Item = N> + '_ {
        let mut pending = vec![root];
        std::iter::from_fn(move || {
            let node = pending.pop()?;
            pending.extend(self.children(node).iter().rev());
            Some(node)
        })
    }

    /// The subtree in postorder (children before their parent).
    fn postorder(&self, root: N) -> Vec<N> {
        let mut out = Vec::new();
        let mut stack = vec![(root, false)];
        while let Some((node, expanded)) = stack.pop() {
            if expanded {
                out.push(node);
            } else {
                stack.push((node, true));
                for &child in self.children(node).iter().rev() {
                    stack.push((child, false));
                }
            }
        }
        out
    }

    fn create_root(&mut self) -> N { self.new_container(Place::Root, Kind::NEW) }

    fn new_container(&mut self, place: Place<N>, kind: Kind) -> N {
        self.nodes.insert(Node {
            fill: Fill::Tiled,
            body: Body::Container {
                place,
                container: Container {
                    kind,
                    members: Members::Children(Vec::new()),
                    memory: Memory::Unset,
                },
            },
        })
    }

    fn placement_parent(&self, placement: Placement<N>) -> Option<N> {
        match placement {
            Placement::Back(parent) => self.nodes.contains_key(parent).then_some(parent),
            Placement::Before(sibling) | Placement::After(sibling) => self.parent(sibling),
        }
        .filter(|&parent| self.window(parent).is_none())
    }

    /// `node` put into `parent`'s children where `placement` says, its place made
    /// `parent`'s child with `size` (a node `unlink` took out, or a root).
    fn link(&mut self, node: N, parent: N, placement: Placement<N>, size: f32) {
        self.attach(node, parent, placement);
        let at = Child { parent, size };
        match &mut self.nodes[node].body {
            Body::Window { at: held, .. } => *held = at,
            Body::Container { place, .. } => *place = Place::Child(at),
        }
    }

    /// `node` listed among `parent`'s children where `placement` says (its place already says
    /// so, or `link` sets it).
    fn attach(&mut self, node: N, parent: N, placement: Placement<N>) {
        let at = match placement {
            Placement::Back(_) => self.children(parent).len(),
            Placement::Before(sibling) => self.position(sibling).expect("a placed sibling").1,
            Placement::After(sibling) => self.position(sibling).expect("a placed sibling").1 + 1,
        };
        let Members::Children(children) = &mut self.container_mut(parent).members else {
            unreachable!("only regular containers accept structural edits");
        };
        children.insert(at, node);
    }

    /// `node` taken out of its parent's children (its memory of it kept, as upstream's
    /// `unlink`). Its place still names the parent: every caller links it again or removes it
    /// (a window is never a root, so there is no place to give it in between).
    fn unlink(&mut self, node: N) {
        let Some((parent, at)) = self.position(node) else {
            return;
        };
        let Members::Children(children) = &mut self.container_mut(parent).members else {
            unreachable!("only regular containers accept structural edits");
        };
        children.remove(at);
    }

    /// A new container where `placement` says.
    fn insert(&mut self, placement: Placement<N>) -> N {
        let parent = self.placement_parent(placement).expect("invalid tile insertion");
        assert!(!self.is_region_pair(parent));
        let node = self.new_container(Place::Child(Child { parent, size: 1.0 }), Kind::NEW);
        self.attach(node, parent, placement);
        node
    }

    /// A new leaf holding `window` where `placement` says: it enters the layout now.
    fn insert_window(&mut self, placement: Placement<N>, window: W) -> N {
        let parent = self.placement_parent(placement).expect("invalid tile insertion");
        assert!(!self.is_region_pair(parent));
        let stamp = self.next_stamp;
        self.next_stamp += 1;
        let node = self.nodes.insert(Node {
            fill: Fill::Tiled,
            body: Body::Window {
                at: Child { parent, size: 1.0 },
                window,
                stamp,
            },
        });
        self.attach(node, parent, placement);
        node
    }

    fn move_node(&mut self, node: N, placement: Placement<N>) -> bool {
        let Some(parent) = self.placement_parent(placement) else {
            return false;
        };
        if !self.nodes.contains_key(node)
            || matches!(placement, Placement::Before(anchor) | Placement::After(anchor) if anchor == node)
        {
            return false;
        }
        let old_parent = self.parent(node);
        let changed_parent = old_parent != Some(parent);
        if self.is_region_pair(parent)
            || old_parent.is_some_and(|old| self.is_region_pair(old))
            || self.is_region_pair(node)
            || (changed_parent && self.is_within(parent, node))
        {
            return false;
        }
        if !changed_parent
            && match placement {
                Placement::Back(_) => self.next_sibling(node).is_none(),
                Placement::Before(anchor) => self.next_sibling(node) == Some(anchor),
                Placement::After(anchor) => self.prev_sibling(node) == Some(anchor),
            }
        {
            return false;
        }
        let size = self.size(node);
        match old_parent.filter(|_| changed_parent) {
            Some(old) => self.detach(node, old),
            None => self.unlink(node),
        }
        self.link(node, parent, placement, if changed_parent { 1.0 } else { size });
        if let Some(old) = old_parent.filter(|_| changed_parent) {
            self.prune_container(old);
        }
        true
    }

    fn remove_node(&mut self, node: N) {
        if let Some(parent) = self.parent(node) {
            if self.is_region_pair(parent) {
                return;
            }
            self.detach(node, parent);
            self.prune_container(parent);
        }
        self.delete_subtree(node);
    }

    fn detach(&mut self, node: N, parent: N) {
        self.detach_selection(node);
        self.unlink(node);
        if self.window(node).is_none() {
            return;
        }
        if !self.normalize_weights(parent) {
            for child in self.children(parent).to_vec() {
                self.set_size(child, 1.0);
            }
        }
    }

    fn prune_container(&mut self, mut node: N) {
        while let Some(parent) = self.parent(node) {
            if self.is_region_pair(parent) {
                break;
            }
            if let Some(child) = self.first_child(node) {
                if self.children(node).len() > 1 {
                    break;
                }
                self.unlink(child);
                let size = self.size(node);
                self.link(child, parent, Placement::After(node), size);
            }
            self.detach_selection(node);
            self.unlink(node);
            self.nodes.remove(node);
            node = parent;
        }
    }

    fn delete_subtree(&mut self, root: N) {
        let Some(node) = self.nodes.remove(root) else { return };
        if let Body::Container { container, .. } = node.body {
            for &child in container.members.list() {
                self.delete_subtree(child);
            }
        }
    }

    /// A copy of `source` (a window with a new stamp, a container empty) at the back of
    /// `parent`, with `source`'s size.
    fn copy_tile(&mut self, source: N, parent: N) -> N {
        let at = Child {
            parent,
            size: self.size(source),
        };
        let Node { fill, body } = self.nodes[source].clone();
        let body = match body {
            Body::Window { window, .. } => {
                let stamp = self.next_stamp;
                self.next_stamp += 1;
                Body::Window { at, window, stamp }
            }
            Body::Container {
                container: Container { kind, .. },
                ..
            } => Body::Container {
                place: Place::Child(at),
                container: Container {
                    kind,
                    members: Members::Children(Vec::new()),
                    memory: Memory::Unset,
                },
            },
        };
        let node = self.nodes.insert(Node { fill, body });
        self.attach(node, parent, Placement::Back(parent));
        node
    }

    /// A copy of the layout root `source` and everything under it, as a new root.
    fn clone_branch(&mut self, source: N) -> N {
        let root = self.new_container(Place::Root, self.container_kind(source));
        self.nodes[root].fill = self.nodes[source].fill;
        let mut stack = vec![(0, source, root)];
        while let Some((next, original_parent, new_parent)) = stack.last_mut() {
            let (original_parent, new_parent) = (*original_parent, *new_parent);
            let Some(&old) = self.children(original_parent).get(*next) else {
                stack.pop();
                continue;
            };
            *next += 1;
            let new = self.copy_tile(old, new_parent);
            let memory = self.memory(original_parent);
            if memory.last() == Some(old) {
                self.set_memory(new_parent, memory.on(new));
            }
            if !self.children(old).is_empty() {
                stack.push((0, old, new));
            }
        }
        if let Some(Members::Regions { primary, .. }) = self.container(source).map(|c| &c.members) {
            let primary = *primary;
            let container = self.container_mut(root);
            let Members::Children(children) = &container.members else {
                unreachable!()
            };
            let order = [children[0], children[1]];
            container.members = Members::Regions { order, primary };
        }
        root
    }

    /// `root`'s two children made its regions, `primary` the master region (upstream's
    /// `set_region_pair`).
    fn set_region_pair(&mut self, root: N, primary: N) {
        let container = self.container_mut(root);
        let Members::Children(children) = &container.members else {
            panic!("expected an arrangement container");
        };
        let [first, second] = children[..] else {
            panic!("expected two region containers");
        };
        let primary = if first == primary {
            Side::First
        } else {
            Side::Second
        };
        container.members = Members::Regions {
            order: [first, second],
            primary,
        };
    }

    fn set_primary_region_first(&mut self, root: N, first: bool) {
        let container = self.container_mut(root);
        let Members::Regions { order: [left, right], primary } = &mut container.members else {
            unreachable!("expected a region pair");
        };
        if (*primary == Side::First) != first {
            (*left, *right) = (*right, *left);
            *primary = if first { Side::First } else { Side::Second };
        }
    }

    fn children_total(&self, parent: N) -> f32 {
        self.children(parent).iter().map(|&child| self.size(child).max(0.0)).sum()
    }

    /// Sibling proportions kept with a mean weight of one.
    fn normalize_weights(&mut self, parent: N) -> bool {
        let children = self.children(parent).to_vec();
        let (count, total) = children.iter().fold((0, 0.0_f32), |(count, total), &child| {
            (count + 1, total + self.size(child).max(0.0))
        });
        if !total.is_finite() || total <= f32::EPSILON {
            return false;
        }
        let scale = count as f32 / total;
        for child in children {
            let size = self.size(child) * scale;
            self.set_size(child, size);
        }
        true
    }

    /// `new` takes `old`'s size, `old` keeps none (siblings both).
    fn assume_size_of(&mut self, new: N, old: N) {
        assert!(self.parent(new) == self.parent(old));
        let size = self.size(old);
        self.set_size(old, 0.0);
        self.set_size(new, size);
    }
}

/// The selection: each container's memory (upstream's `model::selection`).
impl<L: SlotKey + Eq, N: SlotKey + Eq, W: Copy + Ord> Forest<L, N, W> {
    fn current_selection(&self, mut node: N) -> N {
        while let Memory::Toward(child) = self.memory(node) {
            node = child;
        }
        node
    }

    fn last_selection(&self, node: N) -> Option<N> { self.memory(node).last() }

    fn local_selection_of(&self, node: N) -> Option<N> { self.memory(node).local() }

    fn select_locally(&mut self, node: N) -> bool {
        let Some(parent) = self.parent(node) else { return false };
        let old = std::mem::replace(&mut self.container_mut(parent).memory, Memory::Toward(node));
        old.last() != Some(node)
    }

    fn select_node(&mut self, selection: N) {
        if let Some(container) = match &mut self.nodes[selection].body {
            Body::Container { container, .. } => Some(container),
            Body::Window { .. } => None,
        } && let Memory::Toward(child) = container.memory
        {
            container.memory = Memory::Stops(child);
        }
        let mut node = selection;
        while let Some(parent) = self.parent(node) {
            self.set_memory(parent, Memory::Toward(node));
            node = parent;
        }
    }

    fn detach_selection(&mut self, node: N) {
        let parent = self.parent(node).expect("a placed node");
        let memory = self.memory(parent);
        if memory.last() == Some(node) {
            let replacement = self.next_sibling(node).or_else(|| self.prev_sibling(node));
            self.set_memory(
                parent,
                replacement.map_or(Memory::Unset, |child| memory.on(child)),
            );
        }
    }
}

/// Walks over a subtree's windows (upstream's `walk_windows` and its users).
impl<L: SlotKey + Eq, N: SlotKey + Eq, W: Copy + Ord> Forest<L, N, W> {
    /// `preferred` first, then the children in order (or reversed) without it.
    fn ordered_children(&self, node: N, preferred: Option<N>, reverse: bool) -> Vec<N> {
        let mut out: Vec<N> = preferred.into_iter().collect();
        let rest = self.children(node).iter().copied().filter(|&c| Some(c) != preferred);
        if reverse {
            out.extend(rest.rev());
        } else {
            out.extend(rest);
        }
        out
    }

    fn participating_child(
        &self,
        node: N,
        reverse: bool,
        mut participates: impl FnMut(N) -> bool,
    ) -> Option<N> {
        self.ordered_children(node, self.local_selection_of(node), reverse)
            .into_iter()
            .find(|&child| participates(child))
    }

    fn walk_windows<B>(
        &self,
        node: N,
        participates: &impl Fn(W) -> bool,
        order: &impl Fn(N) -> (Option<N>, bool, BranchLimit),
        visit: &mut impl FnMut(N, W) -> ControlFlow<B>,
    ) -> ControlFlow<B, bool> {
        if let Some(window) = self.window(node) {
            let active = participates(window);
            if active {
                visit(node, window)?;
            }
            return ControlFlow::Continue(active);
        }
        let (preferred, reverse, limit) = order(node);
        let mut visited = false;
        for child in self.ordered_children(node, preferred, reverse) {
            visited |= self.walk_windows(child, participates, order, visit)?;
            if matches!(limit, BranchLimit::First)
                || (visited && matches!(limit, BranchLimit::FirstVisible))
            {
                break;
            }
        }
        ControlFlow::Continue(visited)
    }

    fn find_window(
        &self,
        node: N,
        participates: &impl Fn(W) -> bool,
        order: &impl Fn(N) -> (Option<N>, bool),
    ) -> Option<(N, W)> {
        self.walk_windows(
            node,
            participates,
            &|node| {
                let (preferred, reverse) = order(node);
                (preferred, reverse, BranchLimit::All)
            },
            &mut |node, window| ControlFlow::Break((node, window)),
        )
        .break_value()
    }

    pub fn window_participates(&self, window: W) -> bool { !self.left_out.contains(&window) }

    fn find_best_focus_target(&self, node: N) -> Option<(N, W)> {
        self.find_window(node, &|w| self.window_participates(w), &|n| {
            (self.last_selection(n), false)
        })
    }

    /// Whether windows are left out and none of the layout's takes part (most edits then do
    /// nothing).
    fn nothing_takes_part(&self, layout: L) -> bool {
        !self.left_out.is_empty() && self.find_best_focus_target(self.root(layout)).is_none()
    }

    fn visit_visible_windows<B>(
        &self,
        node: N,
        visit: &mut impl FnMut(N, W) -> ControlFlow<B>,
    ) -> ControlFlow<B, bool> {
        let group_limit = if self.left_out.is_empty() {
            BranchLimit::First
        } else {
            BranchLimit::FirstVisible
        };
        self.walk_windows(
            node,
            &|w| self.window_participates(w),
            &|node| {
                if self.is_stacked(node) {
                    (self.local_selection_of(node), false, group_limit)
                } else {
                    (None, false, BranchLimit::All)
                }
            },
            visit,
        )
    }

    fn visible_windows_under(&self, node: N) -> Vec<W> {
        let mut windows = Vec::new();
        let _ = self.visit_visible_windows(node, &mut |_, window| {
            windows.push(window);
            ControlFlow::<()>::Continue(())
        });
        windows
    }

    fn subtree_participates(&self, node: N, child_participates: impl FnMut(&N) -> bool) -> bool {
        match self.window(node) {
            Some(window) => self.window_participates(window),
            None => self.children(node).iter().any(child_participates),
        }
    }

    fn participating_nodes(&self, root: N) -> BTreeSet<N>
    where N: Ord {
        let mut active = BTreeSet::new();
        if self.left_out.is_empty() {
            return active;
        }
        for node in self.postorder(root) {
            if self.subtree_participates(node, |child| active.contains(child)) {
                active.insert(node);
            }
        }
        active
    }
}

/// The window index, derived: which leaf holds which window in each layout, a window's leaves
/// ordered by when it entered each layout.
impl<L: SlotKey + Eq, N: SlotKey + Eq, W: Copy + Ord> Forest<L, N, W> {
    /// Every (window, layout, leaf) the index holds, by window, then by stamp.
    pub fn index(&self) -> Vec<(W, L, N)> {
        let mut entries: Vec<(W, u64, L, N)> = Vec::new();
        for (layout, root) in self.layouts() {
            for node in self.preorder(root) {
                if let Body::Window { window, stamp, .. } = self.nodes[node].body {
                    entries.push((window, stamp, layout, node));
                }
            }
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        entries
            .into_iter()
            .map(|(window, _, layout, node)| (window, layout, node))
            .collect()
    }

    fn occurrences(&self, window: W) -> Vec<(L, N)> {
        self.index()
            .into_iter()
            .filter(|&(w, ..)| w == window)
            .map(|(_, layout, node)| (layout, node))
            .collect()
    }

    /// The leaf holding `window` in `layout` (the first to enter, as upstream's index lists).
    fn window_node(&self, layout: L, window: W) -> Option<N> {
        self.window_leaf(layout, window).map(|(node, _)| node)
    }

    /// The leaf holding `window` in `layout`, with its place.
    fn window_leaf(&self, layout: L, window: W) -> Option<(N, Child<N>)> {
        self.preorder(self.root(layout))
            .filter_map(|node| match self.nodes[node].body {
                Body::Window { at, window: held, stamp } if held == window => {
                    Some((stamp, node, at))
                }
                _ => None,
            })
            .min_by_key(|&(stamp, ..)| stamp)
            .map(|(_, node, at)| (node, at))
    }

    fn windows_with_nodes(&self, layout: L) -> impl Iterator<Item = (N, W)> + '_ {
        self.preorder(self.root(layout))
            .filter_map(|node| self.window(node).map(|w| (node, w)))
    }

    /// A leaf's window replaced: the new window enters the layout now.
    fn set_window(&mut self, node: N, window: W) {
        let next = self.next_stamp;
        if let Body::Window { window: held, stamp, .. } = &mut self.nodes[node].body {
            *held = window;
            *stamp = next;
            self.next_stamp += 1;
        }
    }
}

/// The operations both systems share (upstream's `GroupLayouts`).
impl<L: SlotKey + Eq, N: SlotKey + Eq + Ord, W: Copy + Ord> Forest<L, N, W> {
    pub fn set_window_participation(&mut self, window: W, participation: Participation) -> bool {
        match participation {
            Participation::TakesPart => self.left_out.remove(&window),
            Participation::LeftOut => self.left_out.insert(window),
        }
    }

    pub fn has_windows_for_app(&self, layout: L, is_app: impl Fn(W) -> bool) -> bool {
        self.windows_with_nodes(layout).any(|(_, window)| is_app(window))
    }

    pub fn create_layout(&mut self) -> L {
        let root = self.create_root();
        self.layouts.insert(root)
    }

    pub fn contains_layout(&self, layout: L) -> bool { self.layouts.contains_key(layout) }

    pub fn clone_layout(&mut self, layout: L) -> L {
        let root = self.clone_branch(self.root(layout));
        self.layouts.insert(root)
    }

    pub fn remove_layout(&mut self, layout: L) {
        let root = self.layouts.remove(layout).expect("a layout the system holds");
        self.remove_node(root);
    }

    /// The window's leaves removed; the layouts it was in, in the index's order.
    pub fn remove_window(&mut self, window: W) -> Vec<L> {
        self.left_out.remove(&window);
        let removed = self.occurrences(window);
        for &(_, node) in &removed {
            self.remove_node(node);
        }
        removed.into_iter().map(|(layout, _)| layout).collect()
    }

    pub fn replace_window(&mut self, from: W, to: W) {
        if self.left_out.remove(&from) {
            self.left_out.insert(to);
        }
        if from == to {
            return;
        }
        for (_, node) in self.occurrences(from) {
            self.set_window(node, to);
        }
    }

    pub fn close_app(&mut self, is_app: impl Fn(W) -> bool) {
        self.left_out.retain(|&window| !is_app(window));
        let nodes: Vec<N> = self
            .index()
            .into_iter()
            .filter(|&(window, ..)| is_app(window))
            .map(|(.., node)| node)
            .collect();
        for node in nodes {
            self.remove_node(node);
        }
    }

    pub fn contains_window(&self, layout: L, window: W) -> bool {
        self.window_node(layout, window).is_some()
    }

    pub fn selection(&self, layout: L) -> N {
        let root = self.root(layout);
        let selection = self.current_selection(root);
        if selection != root
            && !self.left_out.is_empty()
            && self.find_best_focus_target(selection).is_none()
        {
            return self.find_best_focus_target(root).map_or(root, |(node, _)| node);
        }
        selection
    }

    /// A container's chosen child: the one the selection goes into, else the one remembered.
    fn local_selection(&self, node: N) -> Option<N> {
        self.local_selection_of(node).or_else(|| self.last_selection(node))
    }

    fn focused_leaf(&self, layout: L) -> Option<(N, W)> {
        let root = self.root(layout);
        let selection = self.current_selection(root);
        self.find_best_focus_target(selection)
            .or_else(|| (selection != root).then(|| self.find_best_focus_target(root)).flatten())
    }

    pub fn selected_window(&self, layout: L) -> Option<W> {
        self.focused_leaf(layout).map(|(_, w)| w)
    }

    pub fn selection_contains_window(&self, layout: L, window: W) -> bool {
        let selection = self.selection(layout);
        self.window_node(layout, window)
            .is_some_and(|node| self.is_within(node, selection))
    }

    pub fn window_in_direction(&self, layout: L, direction: Direction) -> Option<W> {
        let reverse = backwards(direction);
        self.find_window(self.root(layout), &|w| self.window_participates(w), &|node| {
            let kind = self.container_kind(node);
            let preferred = if kind.is_stacked() || kind.orientation() != direction.orientation() {
                self.local_selection_of(node)
            } else {
                None
            };
            (preferred, reverse)
        })
        .map(|(_, window)| window)
    }

    pub fn has_any_fullscreen_node(&self, layout: L) -> bool {
        self.preorder(self.root(layout)).any(|node| {
            self.nodes[node].fill != Fill::Tiled
                && (self.left_out.is_empty() || self.find_best_focus_target(node).is_some())
        })
    }

    fn stacked_selection_container(&self, layout: L) -> Option<N> {
        let selection = self.selection(layout);
        if self.window(selection).is_some() {
            self.ancestors(selection).skip(1).find(|&n| self.is_stacked(n))
        } else if self.is_stacked(selection) {
            Some(selection)
        } else {
            self.children(selection).iter().copied().find(|&n| self.is_stacked(n))
        }
    }

    pub fn parent_of_selection_is_stacked(&self, layout: L) -> bool {
        self.stacked_selection_container(layout).is_some()
    }

    fn stack_members(&self, layout: L, window: W) -> Vec<W> {
        let Some(parent) = self.window_node(layout, window).and_then(|n| self.parent(n)) else {
            return Vec::new();
        };
        if !self.is_stacked(parent) {
            return Vec::new();
        }
        self.children(parent).iter().filter_map(|&n| self.window(n)).collect()
    }

    pub fn windows(&self, layout: L, query: Query<W>) -> Vec<W> {
        match query {
            Query::All => self.windows_with_nodes(layout).map(|(_, w)| w).collect(),
            Query::Visible => self.visible_windows_under(self.root(layout)),
            Query::Selection => self.visible_windows_under(self.selection(layout)),
            Query::Stack(window) => self.stack_members(layout, window),
        }
    }

    /// Where a window sits: the child positions from the root down.
    pub fn window_slot(&self, layout: L, window: W) -> Option<Vec<usize>> {
        let root = self.root(layout);
        let mut node = self.window_node(layout, window)?;
        let mut path = Vec::new();
        while node != root {
            let (parent, at) = self.position(node)?;
            path.push(at);
            node = parent;
        }
        path.reverse();
        Some(path)
    }

    fn move_over(&self, from: N, direction: Direction) -> Option<N> {
        let parent = self.parent(from)?;
        if self.container_kind(parent).orientation() != direction.orientation() {
            return None;
        }
        let sibling = |node: N| {
            if backwards(direction) {
                self.prev_sibling(node)
            } else {
                self.next_sibling(node)
            }
        };
        let mut next = sibling(from);
        while let Some(node) = next {
            if self.left_out.is_empty() || self.find_best_focus_target(node).is_some() {
                return Some(node);
            }
            next = sibling(node);
        }
        None
    }

    fn traverse(&self, from: N, direction: Direction) -> Option<N> {
        if let Some(sibling) = self.move_over(from, direction) {
            return Some(sibling);
        }
        self.ancestors(from)
            .skip(1)
            .find_map(|ancestor| self.move_over(ancestor, direction))
            .map(|target| self.descend_into_target(target, direction))
    }

    fn descend_into_target(&self, target: N, direction: Direction) -> N {
        let active = self.participating_nodes(target);
        let mut current = target;
        while let Some(child) = self.participating_child(
            current,
            !self.kind(current).is_some_and(Kind::is_stacked) && !backwards(direction),
            |child| self.left_out.is_empty() || active.contains(&child),
        ) {
            current = child;
        }
        current
    }

    fn descend_selection(&mut self, layout: L) -> bool {
        if self.nothing_takes_part(layout) {
            return false;
        }
        if let Some(child) = self.last_selection(self.selection(layout)) {
            self.select_node(child);
            return true;
        }
        false
    }

    fn move_focus(&mut self, layout: L, direction: Direction) -> (Option<W>, Vec<W>) {
        let selection = self.selection(layout);
        let Some(new_node) = self.traverse(selection, direction) else {
            return (None, Vec::new());
        };
        let Some((focus_node, focus_window)) = self.find_best_focus_target(new_node) else {
            return (None, Vec::new());
        };
        let mut highest_revealed = focus_node;
        let mut node = focus_node;
        while let Some(parent) = self.parent(node) {
            let is_group = self.is_stacked(parent);
            if self.select_locally(node) && is_group {
                highest_revealed = node;
            }
            node = parent;
        }
        (Some(focus_window), self.visible_windows_under(highest_revealed))
    }

    pub fn focus(&mut self, layout: L, focus: Focus<W>) -> Focused<W> {
        let handled = match focus {
            Focus::Select(window) => {
                if self.left_out.contains(&window) {
                    false
                } else if let Some(node) = self.window_node(layout, window) {
                    self.select_node(node);
                    true
                } else {
                    false
                }
            }
            Focus::Ascend => {
                if self.nothing_takes_part(layout) {
                    false
                } else if let Some(parent) = self.parent(self.selection(layout)) {
                    self.select_node(parent);
                    true
                } else {
                    false
                }
            }
            Focus::Descend => self.descend_selection(layout),
            Focus::Direction(direction) => {
                let (window, raise) = self.move_focus(layout, direction);
                return Focused::Movement { window, raise };
            }
        };
        Focused::Selection { handled }
    }

    /// `node` put into a container of `kind` (upstream's `nest_in_container_internal`): its
    /// parent when it is an only child, else a new container in its place (a new root at the
    /// root).
    fn nest_in_container(&mut self, layout: L, node: N, kind: Kind) -> N {
        let old_parent = self.parent(node);
        let parent = match old_parent {
            Some(parent) if self.children(parent).len() == 1 => parent,
            Some(old_parent) => {
                let is_selection = self.local_selection_of(old_parent) == Some(node);
                let new_parent = self.insert(Placement::Before(node));
                self.assume_size_of(new_parent, node);
                self.move_node(node, Placement::Back(new_parent));
                if is_selection {
                    self.select_locally(new_parent);
                }
                self.select_locally(node);
                new_parent
            }
            None => {
                let new_root = self.create_root();
                self.layouts[layout] = new_root;
                self.move_node(node, Placement::Back(new_root));
                self.select_locally(node);
                new_root
            }
        };
        self.set_kind(parent, kind);
        parent
    }

    fn toggle_orientation(&mut self, layout: L) -> Edited<W> {
        if self.nothing_takes_part(layout) {
            return Edited::Unchanged;
        }
        let selection = self.selection(layout);
        let target = self.parent(selection).unwrap_or_else(|| self.root(layout));
        match self.container_kind(target) {
            Kind::Stack(_) => Edited::Unchanged,
            Kind::Split(orientation) => {
                self.set_kind(target, Kind::Split(perpendicular(orientation)));
                Edited::Changed
            }
        }
    }

    fn toggle_fullscreen(&mut self, layout: L, within_gaps: bool) -> Vec<W> {
        if self.nothing_takes_part(layout) {
            return Vec::new();
        }
        let node = self.selection(layout);
        let fill = toggled(self.nodes[node].fill, within_gaps);
        self.nodes[node].fill = fill;
        if fill == Fill::Tiled {
            Vec::new()
        } else {
            self.visible_windows_under(node)
        }
    }

    fn split_selection(&mut self, layout: L, kind: Kind) -> Edited<W> {
        if self.nothing_takes_part(layout) {
            return Edited::Unchanged;
        }
        let selection = self.selection(layout);
        self.nest_in_container(layout, selection, kind);
        Edited::Changed
    }

    fn apply_stacking(&mut self, layout: L, default: DefaultOrientation) -> Vec<W> {
        if self.nothing_takes_part(layout) {
            return Vec::new();
        }
        let selection = self.selection(layout);
        let container = if self.window(selection).is_some() {
            self.parent(selection)
        } else {
            Some(selection)
        };
        let Some(container) = container else { return Vec::new() };
        let orientation = match self.container_kind(container) {
            Kind::Stack(orientation) => perpendicular(orientation),
            Kind::Split(orientation) => group_orientation(orientation, default),
        };
        self.set_kind(container, Kind::Stack(orientation));
        if let Some(first) = self.first_child(container) {
            self.select_node(first);
        }
        self.visible_windows_under(container)
    }

    fn unstack(&mut self, layout: L, default: DefaultOrientation) -> Vec<W> {
        if self.nothing_takes_part(layout) {
            return Vec::new();
        }
        let Some(container) = self.stacked_selection_container(layout) else {
            return Vec::new();
        };
        let orientation = group_orientation(self.container_kind(container).orientation(), default);
        self.set_kind(container, Kind::Split(orientation));
        self.visible_windows_under(container)
    }

    fn move_stack_tab(&mut self, layout: L, window: W, target: W) -> bool {
        if !self.window_participates(window) || !self.window_participates(target) {
            return false;
        }
        let (Some(source), Some(target)) = (
            self.window_node(layout, window),
            self.window_node(layout, target),
        ) else {
            return false;
        };
        let Some(stack) = self.common_ancestor(source, target) else {
            return false;
        };
        if !self.is_stacked(stack) {
            return false;
        }
        let tab = |node: N| self.ancestors(node).find(|&n| self.parent(n) == Some(stack));
        let (Some(moving), Some(anchor)) = (tab(source), tab(target)) else {
            return false;
        };
        if moving == anchor {
            return false;
        }
        let children = self.children(stack);
        let forward =
            children.iter().position(|&c| c == moving) < children.iter().position(|&c| c == anchor);
        let place = if forward {
            Placement::After(anchor)
        } else {
            Placement::Before(anchor)
        };
        self.move_node(moving, place)
    }

    fn swap_windows(&mut self, layout: L, a: W, b: W) -> bool {
        if !self.window_participates(a) || !self.window_participates(b) {
            return false;
        }
        let (Some(node_a), Some(node_b)) =
            (self.window_node(layout, a), self.window_node(layout, b))
        else {
            return false;
        };
        if node_a == node_b {
            return false;
        }
        let selection = self.selection(layout);
        // Each window keeps when it entered the layout (upstream's index entries move with it).
        let held = |f: &Self, node: N| match f.nodes[node].body {
            Body::Window { window, stamp, .. } => (window, stamp),
            Body::Container { .. } => unreachable!("leaves hold windows"),
        };
        let (a_held, b_held) = (held(self, node_a), held(self, node_b));
        for (node, (window, stamp)) in [(node_a, b_held), (node_b, a_held)] {
            if let Body::Window { window: w, stamp: s, .. } = &mut self.nodes[node].body {
                (*w, *s) = (window, stamp);
            }
        }
        self.select_node(if selection == node_a {
            node_b
        } else if selection == node_b {
            node_a
        } else {
            selection
        });
        true
    }
}

fn edited<W>(changed: bool) -> Edited<W> {
    if changed {
        Edited::Changed
    } else {
        Edited::Unchanged
    }
}

/// Sizes, resizes, drops and the shape (upstream's `tiles`).
impl<L: SlotKey + Eq, N: SlotKey + Eq + Ord, W: Copy + Ord> Forest<L, N, W> {
    fn proportion(&self, node: N) -> Option<f64> {
        let parent = self.parent(node)?;
        let total = self.children_total(parent);
        (total.is_finite() && total > f32::EPSILON)
            .then(|| f64::from(self.size(node).max(0.0)) / f64::from(total))
    }

    fn take_share(&mut self, node: N, from: N, share: f32) {
        assert!(self.parent(node) == self.parent(from));
        const MIN_NODE_SIZE: f32 = 0.05;
        let share = share.min(self.size(from) - MIN_NODE_SIZE);
        let share = share.max(MIN_NODE_SIZE - self.size(node));
        self.set_size(from, self.size(from) - share);
        self.set_size(node, self.size(node) + share);
        self.normalize_weights(self.parent(node).expect("a sibling"));
    }

    fn rebalance(&mut self, root: N) -> bool {
        let mut changed = false;
        for child in self.children(root).to_vec() {
            changed |= self.size(child) != 1.0;
            self.set_size(child, 1.0);
            changed |= self.rebalance(child);
        }
        changed
    }

    pub fn remove_and_rebalance(&mut self, window: W) {
        self.left_out.remove(&window);
        for (_, node) in self.occurrences(window) {
            let parent = self.parent(node);
            self.remove_node(node);
            if let Some(parent) = parent.filter(|&p| self.nodes.contains_key(p)) {
                self.rebalance(parent);
            }
        }
    }

    pub fn resize_window(&mut self, layout: L, window: W, resize: Resize) {
        if !self.window_participates(window) {
            return;
        }
        let Some(node) = self.window_node(layout, window) else {
            return;
        };
        match resized(self.nodes[node].fill, resize) {
            Some(fill) => self.nodes[node].fill = fill,
            None => self.set_frame_from_resize(node, resize),
        }
    }

    fn resize_pair(&self, node: N, direction: Direction) -> Option<(N, N)> {
        self.ancestors(node).find_map(|candidate| {
            if self.is_stacked(self.parent(candidate)?) {
                return None;
            }
            self.move_over(candidate, direction).map(|sibling| (candidate, sibling))
        })
    }

    fn resize_internal(&mut self, node: N, screen_ratio: f64, direction: Direction) -> bool {
        let Some((resizing, sibling)) = self.resize_pair(node, direction) else {
            return false;
        };
        let exchange_rate = self
            .ancestors(resizing)
            .skip(1)
            .try_fold(1.0, |r, node| match self.parent(node) {
                Some(parent)
                    if self.container_kind(parent).orientation() == direction.orientation()
                        && !self.is_stacked(parent) =>
                {
                    self.proportion(node).map(|p| r * p)
                }
                _ => Some(r),
            })
            .unwrap_or(1.0);
        let parent = self.parent(resizing).expect("a sibling");
        let local_ratio = screen_ratio * f64::from(self.children_total(parent)) / exchange_rate;
        self.take_share(resizing, sibling, local_ratio as f32);
        true
    }

    fn set_frame_from_resize(&mut self, node: N, Resize { old, new, screen, .. }: Resize) {
        const RESIZE_DELTA_EPSILON: f64 = 1.0;
        let choose_edge = |mut edges: [(Direction, f64); 2]| {
            for (_, delta) in &mut edges {
                if delta.abs() < RESIZE_DELTA_EPSILON {
                    *delta = 0.0;
                }
            }
            // Upstream's comparison, kept: a NaN delta swaps the edges.
            #[allow(clippy::neg_cmp_op_on_partial_ord)]
            if !(edges[0].1.abs() >= edges[1].1.abs()) {
                edges.swap(0, 1);
            }
            edges.into_iter().find(|&(direction, delta)| {
                delta != 0.0 && self.resize_pair(node, direction).is_some()
            })
        };
        let horizontal = choose_edge([
            (Direction::Left, old.x - new.x),
            (Direction::Right, (new.x + new.w) - (old.x + old.w)),
        ]);
        let vertical = choose_edge([
            (Direction::Up, old.y - new.y),
            (Direction::Down, (new.y + new.h) - (old.y + old.h)),
        ]);
        for (edge, extent) in [(horizontal, screen.w), (vertical, screen.h)] {
            if let Some((direction, delta)) = edge {
                self.resize_internal(node, delta / extent, direction);
            }
        }
    }

    fn resize_selection(&mut self, layout: L, amount: f64, along: ResizeAlong) -> Edited<W> {
        if self.nothing_takes_part(layout) || amount == 0.0 {
            return Edited::Unchanged;
        }
        let selection = self.selection(layout);
        if self.window(selection).is_none() {
            return Edited::Unchanged;
        }
        let pair = |orientation| match orientation {
            Orientation::Horizontal => [Direction::Right, Direction::Left],
            Orientation::Vertical => [Direction::Down, Direction::Up],
        };
        let changed = match along {
            ResizeAlong::Horizontal => pair(Orientation::Horizontal)
                .into_iter()
                .any(|d| self.resize_internal(selection, amount, d)),
            ResizeAlong::Vertical => pair(Orientation::Vertical)
                .into_iter()
                .any(|d| self.resize_internal(selection, amount, d)),
            ResizeAlong::Smart => {
                let mut node = selection;
                while let Some(parent) = self.parent(node) {
                    if let Kind::Split(orientation) = self.container_kind(parent)
                        && pair(orientation)
                            .into_iter()
                            .any(|d| self.resize_internal(node, amount, d))
                    {
                        return Edited::Changed;
                    }
                    node = parent;
                }
                false
            }
        };
        edited(changed)
    }

    fn drop_window(&mut self, layout: L, source: W, target: W, action: DropAction) -> bool {
        if source == target
            || !self.window_participates(source)
            || !self.window_participates(target)
        {
            return false;
        }
        let direction = match action {
            DropAction::Swap => return self.swap_windows(layout, source, target),
            DropAction::Stack | DropAction::Insert(_) => action,
        };
        let (Some(source_node), Some((target_node, target_at))) = (
            self.window_node(layout, source),
            self.window_leaf(layout, target),
        ) else {
            return false;
        };
        let DropAction::Insert(direction) = direction else {
            if self.is_stacked(target_at.parent) {
                self.move_node(source_node, Placement::After(target_node));
                self.select_node(source_node);
                return true;
            }
            let orientation = self.container_kind(target_at.parent).orientation();
            let container = self.insert(Placement::Before(target_node));
            self.assume_size_of(container, target_node);
            self.move_node(target_node, Placement::Back(container));
            self.move_node(source_node, Placement::Back(container));
            self.set_kind(container, Kind::Stack(orientation));
            self.select_node(source_node);
            return true;
        };
        let target_anchor =
            self.parent(target_node).filter(|&p| self.is_stacked(p)).unwrap_or(target_node);
        if self.is_within(target_anchor, source_node) {
            return false;
        }
        let before = backwards(direction);
        match self.parent(target_anchor) {
            Some(parent)
                if !self.is_stacked(parent)
                    && self.container_kind(parent).orientation() == direction.orientation() =>
            {
                let placement = if before {
                    Placement::Before(target_anchor)
                } else {
                    Placement::After(target_anchor)
                };
                self.move_node(source_node, placement);
            }
            parent => {
                let kind = Kind::Split(direction.orientation());
                let container = match parent {
                    None => self.nest_in_container(layout, target_anchor, kind),
                    Some(_) => {
                        let container = self.insert(Placement::Before(target_anchor));
                        self.assume_size_of(container, target_anchor);
                        self.set_kind(container, kind);
                        container
                    }
                };
                self.move_node(target_anchor, Placement::Back(container));
                if before {
                    self.move_node(source_node, Placement::Before(target_anchor));
                } else {
                    self.move_node(source_node, Placement::Back(container));
                }
            }
        }
        self.select_node(source_node);
        true
    }

    /// The layout as the core's traditional geometry takes it (none when nothing takes space):
    /// pruned to the nodes that take part, from the master region when a region pair's other
    /// region takes none.
    pub fn shape(
        &self,
        layout: L,
        pass: Pass,
        constraints: &impl Fn(W) -> Option<WindowConstraints>,
    ) -> Option<geometry::Traditional<W>> {
        let (active, start) = self.laid_out(layout)?;
        Some(self.shape_from(start, &active, pass, constraints, &|_| ()))
    }

    /// What a layout pass lays out: the nodes that take part, and the node laid out from (none
    /// when it takes no part).
    fn laid_out(&self, layout: L) -> Option<(SlotSet<N>, N)> {
        let node = self.root(layout);
        let mut active = SlotSet::default();
        for child in self.postorder(node) {
            if self.subtree_participates(child, |&n| active.contains(n)) {
                active.insert(child);
            }
        }
        let start = if self.nodes[node].fill != Fill::Tiled {
            node
        } else {
            self.regions(node)
                .filter(|&(_, stack)| !active.contains(stack))
                .map_or(node, |(master, _)| master)
        };
        active.contains(start).then_some((active, start))
    }

    /// The shape laid out from `start`, each stack labelled by `label`.
    fn shape_from<S>(
        &self,
        start: N,
        active: &SlotSet<N>,
        pass: Pass,
        constraints: &impl Fn(W) -> Option<WindowConstraints>,
        label: &impl Fn(N) -> S,
    ) -> geometry::Traditional<W, S> {
        geometry::Traditional {
            screen: pass.screen,
            area: super::tiling_area(pass.screen, pass.gaps),
            gaps: pass.gaps,
            stack_offset: pass.stack_offset,
            stack_line: pass.stack_line,
            root: self.shape_node(start, active, constraints, label),
        }
    }

    /// A layout pass with its group bars (upstream's `project` with a group output): each
    /// window's frame and the bar of each stack `bars` asks for, none while the layout's root
    /// fills.
    pub fn project(
        &self,
        layout: L,
        pass: Pass,
        constraints: &impl Fn(W) -> Option<WindowConstraints>,
        bars: Bars,
    ) -> Projected<N, W> {
        let Some((active, start)) = self.laid_out(layout) else {
            return Projected {
                frames: Vec::new(),
                bars: Vec::new(),
            };
        };
        let placed = self.shape_from(start, &active, pass, constraints, &|n| n).placed();
        let stacks: BTreeMap<N, Rect> = placed.stacks.into_iter().collect();
        let mut out = Vec::new();
        if self.nodes[self.root(layout)].fill == Fill::Tiled {
            self.bars_from(start, &active, &stacks, bars, &mut out);
        }
        Projected {
            frames: placed.frames,
            bars: out,
        }
    }

    /// The bars under `node` (taking part), each stack's frame as the pass placed it: a node
    /// that fills ends the walk below it (and along the selection path, drops the bars found).
    fn bars_from(
        &self,
        node: N,
        active: &SlotSet<N>,
        stacks: &BTreeMap<N, Rect>,
        bars: Bars,
        out: &mut Vec<GroupBar<N, W>>,
    ) {
        if self.nodes[node].fill != Fill::Tiled {
            if bars == Bars::SelectionPath {
                out.clear();
            }
            return;
        }
        let Body::Container { container, .. } = &self.nodes[node].body else {
            return;
        };
        let taking_part = |child: N| active.contains(child);
        let children: Vec<N> =
            container.members.list().iter().copied().filter(|&c| taking_part(c)).collect();
        let chosen = match container.kind {
            Kind::Stack(orientation) => {
                let focused = self
                    .participating_child(node, false, taking_part)
                    .expect("a participating stack has a participating child");
                out.push(self.group_bar(node, orientation, stacks[&node], &children, focused));
                Some(focused)
            }
            Kind::Split(_) => match bars {
                Bars::SelectionPath => self.participating_child(node, false, taking_part),
                Bars::Every => None,
            },
        };
        for child in children {
            if bars == Bars::Every || Some(child) == chosen {
                self.bars_from(child, active, stacks, bars, out);
            }
        }
    }

    /// A stack's bar: one window per child holding one that takes part (the child's focus
    /// target), and which of them stands for the focused child.
    fn group_bar(
        &self,
        stack: N,
        orientation: Orientation,
        frame: Rect,
        children: &[N],
        focused: N,
    ) -> GroupBar<N, W> {
        let mut windows = Vec::with_capacity(children.len());
        let mut selected = 0;
        for &child in children {
            if let Some((_, window)) = self.find_best_focus_target(child) {
                if child == focused {
                    selected = windows.len();
                }
                windows.push(window);
            }
        }
        GroupBar {
            stack,
            orientation,
            frame,
            windows,
            selected,
        }
    }

    fn shape_node<S>(
        &self,
        node: N,
        active: &SlotSet<N>,
        constraints: &impl Fn(W) -> Option<WindowConstraints>,
        label: &impl Fn(N) -> S,
    ) -> geometry::Node<W, S> {
        let fill = self.nodes[node].fill;
        let kind = match &self.nodes[node].body {
            Body::Window { window, .. } => {
                return geometry::Node::Window {
                    window: *window,
                    constraints: constraints(*window),
                    fill,
                };
            }
            Body::Container { container, .. } => container.kind,
        };
        let child = |child: N| geometry::Child {
            size: self.size(child),
            node: self.shape_node(child, active, constraints, label),
        };
        let mut children: Vec<N> =
            self.children(node).iter().copied().filter(|&c| active.contains(c)).collect();
        match kind {
            Kind::Stack(orientation) => {
                let focused = self
                    .participating_child(node, false, |c| active.contains(c))
                    .expect("a participating stack has a participating child");
                let at = children.iter().position(|&c| c == focused).expect("one of its children");
                let after = children.split_off(at + 1);
                children.pop();
                geometry::Node::Stack {
                    label: label(node),
                    horizontal: orientation == Orientation::Horizontal,
                    fill,
                    before: children.into_iter().map(child).collect(),
                    focused: Box::new(child(focused)),
                    after: after.into_iter().map(child).collect(),
                }
            }
            Kind::Split(orientation) => {
                let rest = children.split_off(1);
                let first =
                    children.pop().expect("a participating container has a participating child");
                geometry::Node::Axis {
                    horizontal: orientation == Orientation::Horizontal,
                    fill,
                    first: Box::new(child(first)),
                    rest: rest.into_iter().map(child).collect(),
                }
            }
        }
    }
}

/// A tree helper both systems' own operations use: a new leaf holding `window` at the back of
/// `parent` (upstream's `add_window_under`).
impl<L: SlotKey + Eq, N: SlotKey + Eq + Ord, W: Copy + Ord> Forest<L, N, W> {
    fn add_window_under(&mut self, parent: N, window: W) -> N {
        self.insert_window(Placement::Back(parent), window)
    }

    /// How many windows a subtree holds, taking part or not (upstream's `count_windows`).
    fn count_windows(&self, node: N) -> usize {
        match self.window(node) {
            Some(_) => 1,
            None => self.children(node).iter().map(|&child| self.count_windows(child)).sum(),
        }
    }

    /// The windows from `start` on, stepping through `region` in preorder (or back).
    fn windows_from(
        &self,
        region: N,
        start: Option<N>,
        forward: bool,
    ) -> impl Iterator<Item = W> + '_ {
        std::iter::successors(start, move |&node| self.step(node, region, forward))
            .filter_map(|node| self.window(node))
    }
}

/// An edit of a layout (upstream's `LayoutEdit`; a drop that moves or consumes reaches the
/// system as a selection and the edit it stands for).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Edit<W> {
    Move {
        direction: Direction,
        beyond: Beyond,
    },
    Split(Kind),
    Fullscreen {
        within_gaps: bool,
    },
    Swap(W, W),
    ReorderTab {
        window: W,
        target: W,
    },
    Drop {
        source: W,
        target: W,
        action: DropAction,
    },
    Join(Direction),
    ConsumeOrExpel(Direction),
    Stack(DefaultOrientation),
    Unstack(DefaultOrientation),
    Unjoin,
    Resize {
        amount: f64,
        along: ResizeAlong,
    },
    Rebalance,
    ToggleOrientation,
}

/// The weight a window added next to the selection takes (upstream's `equalize_nodes`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewSibling {
    /// The mean of its siblings, 1 (siblings are kept normalized to their count), as Sway does.
    AverageWeight,
    /// Half of the selection's weight, the selection keeping the other half.
    HalvesSelection,
}

/// The traditional layout system: the forest and its settings (upstream's
/// `TraditionalLayoutSystem`).
#[derive(Debug, Clone, PartialEq)]
pub struct Traditional<L, N, W> {
    forest: Forest<L, N, W>,
    insertion: InsertionPoint,
    new_sibling: NewSibling,
}

impl<L: SlotKey + Eq, N: SlotKey + Eq + Ord, W: Copy + Ord> Traditional<L, N, W> {
    pub fn new(insertion: InsertionPoint, new_sibling: NewSibling) -> Self {
        Self::from_parts(Forest::default(), insertion, new_sibling)
    }

    pub fn from_parts(
        forest: Forest<L, N, W>,
        insertion: InsertionPoint,
        new_sibling: NewSibling,
    ) -> Self {
        Self { forest, insertion, new_sibling }
    }

    pub fn forest(&self) -> &Forest<L, N, W> { &self.forest }

    pub fn insertion(&self) -> InsertionPoint { self.insertion }

    pub fn new_sibling(&self) -> NewSibling { self.new_sibling }

    pub fn set_window_insertion_point(&mut self, insertion: InsertionPoint) {
        self.insertion = insertion;
    }

    pub fn set_new_sibling(&mut self, new_sibling: NewSibling) { self.new_sibling = new_sibling; }

    pub fn set_window_participation(&mut self, window: W, participation: Participation) -> bool {
        self.forest.set_window_participation(window, participation)
    }

    pub fn create_layout(&mut self) -> L { self.forest.create_layout() }

    pub fn clone_layout(&mut self, layout: L) -> L { self.forest.clone_layout(layout) }

    pub fn remove_layout(&mut self, layout: L) { self.forest.remove_layout(layout) }

    pub fn remove_window(&mut self, window: W) { self.forest.remove_window(window); }

    pub fn remove_and_rebalance(&mut self, window: W) { self.forest.remove_and_rebalance(window) }

    pub fn replace_window(&mut self, from: W, to: W) { self.forest.replace_window(from, to) }

    pub fn close_app(&mut self, is_app: impl Fn(W) -> bool) { self.forest.close_app(is_app) }

    pub fn resize_window(&mut self, layout: L, window: W, resize: Resize) {
        self.forest.resize_window(layout, window, resize)
    }

    pub fn focus(&mut self, layout: L, focus: Focus<W>) -> Focused<W> {
        self.forest.focus(layout, focus)
    }

    /// A window added: next to the selection, or along `toward` at the root's edge (upstream's
    /// `WindowChange::Add`).
    pub fn add_window(&mut self, layout: L, window: W, toward: Option<Direction>) {
        if self.forest.contains_window(layout, window) {
            return;
        }
        let Some(direction) = toward else {
            self.add_window_after_selection(layout, window);
            return;
        };
        let f = &mut self.forest;
        let mut root = f.root(layout);
        let kind = Kind::Split(direction.orientation());
        let current = f.container_kind(root);
        if !f.children(root).is_empty()
            && (current.is_stacked() || current.orientation() != direction.orientation())
        {
            root = f.nest_in_container(layout, root, kind);
        }
        f.set_kind(root, kind);
        let node = f.add_window_under(root, window);
        if !backwards(direction)
            && let Some(first) = f.first_child(root)
        {
            f.move_node(node, Placement::Before(first));
        }
        f.select_node(node);
    }

    fn add_window_after_selection(&mut self, layout: L, window: W) {
        if self.insertion == InsertionPoint::EndOfTree {
            let root = self.forest.root(layout);
            let node = self.forest.add_window_under(root, window);
            self.forest.select_node(node);
            return;
        }
        let selection = self.forest.selection(layout);
        let node = match self.forest.parent(selection) {
            Some(parent) => self.smart_window_insertion(layout, selection, parent, window),
            // The root selected with children: split from its chosen child rather than append
            // a fresh full-weight sibling.
            None => match self
                .forest
                .local_selection(selection)
                .or_else(|| self.forest.last_child(selection))
            {
                Some(anchor) => self.smart_window_insertion(layout, anchor, selection, window),
                None => self.forest.add_window_under(selection, window),
            },
        };
        self.forest.select_node(node);
    }

    /// A new leaf for `window` beside `selection`, a child of `parent`.
    fn smart_window_insertion(&mut self, layout: L, selection: N, parent: N, window: W) -> N {
        let f = &mut self.forest;
        let kind = f.container_kind(parent);
        let placement = if !kind.is_stacked() && f.children(parent).len() > 3 {
            Placement::Back(f.nest_in_container(layout, selection, kind))
        } else {
            Placement::After(selection)
        };
        let node = f.insert_window(placement, window);
        self.split_new_sibling_from_selection(selection, node);
        node
    }

    fn split_new_sibling_from_selection(&mut self, selection: N, new_sibling: N) {
        let f = &mut self.forest;
        let Some(parent) = f.parent(selection) else { return };
        if f.parent(new_sibling) != Some(parent) {
            return;
        }
        match self.new_sibling {
            // Siblings stay normalized to their count, so the average is 1, the new node's
            // weight already.
            NewSibling::AverageWeight => {}
            NewSibling::HalvesSelection => {
                let selected = f.size(selection).max(0.0);
                let (kept, given) = if selected <= f32::EPSILON {
                    // Stale or unset sizes: an even split.
                    (1.0, 1.0)
                } else {
                    (selected * 0.5, selected * 0.5)
                };
                f.set_size(selection, kept);
                f.set_size(new_sibling, given);
            }
        }
    }

    pub fn can_move_window(&self, layout: L, window: W, direction: Direction) -> bool {
        self.forest.window_participates(window)
            && self
                .forest
                .window_node(layout, window)
                .is_some_and(|node| self.move_destination(node, direction).is_some())
    }

    pub fn edit(&mut self, layout: L, edit: Edit<W>) -> Edited<W> {
        let f = &mut self.forest;
        match edit {
            Edit::Move { direction, beyond } => {
                edited(self.move_selection_with_display_neighbor(layout, direction, beyond))
            }
            Edit::Split(kind) => f.split_selection(layout, kind),
            Edit::Fullscreen { within_gaps } => {
                Edited::Raise(f.toggle_fullscreen(layout, within_gaps))
            }
            Edit::Swap(a, b) => edited(f.swap_windows(layout, a, b)),
            Edit::ReorderTab { window, target } => edited(f.move_stack_tab(layout, window, target)),
            Edit::Drop { source, target, action } => {
                edited(f.drop_window(layout, source, target, action))
            }
            Edit::Join(direction) => edited(self.join_selection_with_direction(layout, direction)),
            Edit::ConsumeOrExpel(direction) => {
                let selection = f.selection(layout);
                let joined = f.parent(selection).and_then(|parent| f.parent(parent)).is_some();
                edited(if joined {
                    self.unjoin_selection(layout)
                } else {
                    self.join_selection_with_direction(layout, direction)
                })
            }
            Edit::Stack(default) => Edited::Raise(f.apply_stacking(layout, default)),
            Edit::Unstack(default) => Edited::Raise(f.unstack(layout, default)),
            Edit::Unjoin => edited(self.unjoin_selection(layout)),
            Edit::Resize { amount, along } => f.resize_selection(layout, amount, along),
            Edit::Rebalance => {
                let root = f.root(layout);
                edited(f.rebalance(root))
            }
            Edit::ToggleOrientation => f.toggle_orientation(layout),
        }
    }

    fn move_selection_with_display_neighbor(
        &mut self,
        layout: L,
        direction: Direction,
        beyond: Beyond,
    ) -> bool {
        let f = &self.forest;
        let selection = f.selection(layout);
        if beyond == Beyond::AnotherDisplay
            && !f.ancestors(selection).any(|node| f.move_over(node, direction).is_some())
        {
            return false;
        }
        if !f.left_out.is_empty() && f.focused_leaf(layout).is_none() {
            return false;
        }
        self.move_node(layout, selection, direction)
    }

    fn move_node(&mut self, layout: L, moving: N, direction: Direction) -> bool {
        let Some(old_parent) = self.forest.parent(moving) else {
            return false;
        };
        let is_selection = self.forest.local_selection_of(old_parent) == Some(moving);
        let Some((placement, wrap_root)) = self.move_destination(moving, direction) else {
            return false;
        };
        let f = &mut self.forest;
        if let Some(root) = wrap_root {
            f.nest_in_container(layout, root, Kind::Split(direction.orientation()));
        }
        let moved = f.move_node(moving, placement);
        if moved && is_selection {
            let mut current = Some(moving);
            while let Some(node) = current.filter(|&node| node != old_parent) {
                current = f.parent(node);
                f.select_locally(node);
            }
        }
        moved
    }

    /// Where a move puts `moving`, and the root to wrap first when it leaves the root's edge.
    fn move_destination(
        &self,
        moving: N,
        direction: Direction,
    ) -> Option<(Placement<N>, Option<N>)> {
        let f = &self.forest;
        f.parent(moving)?;
        let mut wrap_root = None;
        let (target, behind) = if let Some(sibling) = f.move_over(moving, direction) {
            let mut node = sibling;
            let target = loop {
                let Some(next) = f.local_selection_of(node).or(f.first_child(node)) else {
                    break node;
                };
                if f.container_kind(node).orientation() == direction.orientation() {
                    break next;
                }
                node = next;
            };
            (target, target != sibling)
        } else {
            let along = f.ancestors(moving).skip(1).find(|&node| {
                f.parent(node).is_some_and(|parent| {
                    f.container_kind(parent).orientation() == direction.orientation()
                })
            });
            match along {
                Some(target) => (target, false),
                None => {
                    let old_root = f.ancestors(moving).last().expect("a node is its own ancestor");
                    let root_kind = f.container_kind(old_root);
                    // Leaving the edge of a root group splits the window out of it; at the edge
                    // of a same-oriented split there is nowhere further to go.
                    if root_kind == Kind::Split(direction.orientation()) {
                        return None;
                    }
                    wrap_root = Some(old_root);
                    (old_root, false)
                }
            }
        };
        let placement = if backwards(direction) != behind {
            Placement::Before(target)
        } else {
            Placement::After(target)
        };
        Some((placement, wrap_root))
    }

    fn join_selection_with_direction(&mut self, layout: L, direction: Direction) -> bool {
        if self.forest.nothing_takes_part(layout) {
            return false;
        }
        let mut selection = self.forest.selection(layout);
        let Some(target) = self.find_natural_join_target(selection, direction) else {
            return false;
        };
        let f = &self.forest;
        // A leaf at the edge of a container along the direction lifts the selection to that
        // container, so whole groups merge.
        if let Some(parent) = f.parent(selection) {
            let parent_kind = f.container_kind(parent);
            let is_edge = if backwards(direction) {
                f.prev_sibling(selection).is_none()
            } else {
                f.next_sibling(selection).is_none()
            };
            if parent_kind == Kind::Split(direction.orientation())
                && (is_edge || f.children(parent).len() == 2)
                && f.parent(target) != Some(parent)
                && !f.is_within(target, parent)
            {
                selection = parent;
            }
        }
        // A selection that is now a container along the direction absorbs the target, with no
        // extra nesting.
        let target_is_within = f.is_within(target, selection);
        let selection_is_within = f.is_within(selection, target);
        if f.kind(selection) == Some(Kind::Split(direction.orientation()))
            && !target_is_within
            && !selection_is_within
            && f.parent(target) != Some(selection)
        {
            let placement = match f.first_child(selection) {
                Some(first) if backwards(direction) => Placement::Before(first),
                _ => Placement::Back(selection),
            };
            self.forest.move_node(target, placement);
            self.forest.select_node(selection);
            return true;
        }
        self.perform_natural_join(selection, target, direction);
        if self.forest.window(selection).is_some() {
            self.forest.select_node(selection);
        } else {
            self.forest.descend_selection(layout);
        }
        true
    }

    fn find_natural_join_target(&self, from: N, direction: Direction) -> Option<N> {
        let f = &self.forest;
        let sibling = self.find_direct_sibling_target(from, direction);
        if let Some(parent) = f.parent(from)
            && f.container_kind(parent) == Kind::Split(direction.orientation())
            && (sibling.is_none() || f.children(parent).len() == 2)
            && let Some(neighbor) = self.find_direct_sibling_target(parent, direction)
        {
            return Some(neighbor);
        }
        sibling
            .or_else(|| {
                f.traverse(from, direction).map(|target| {
                    if f.window(target).is_some() {
                        target
                    } else {
                        self.find_best_container_child_for_joining(target, direction)
                            .unwrap_or(target)
                    }
                })
            })
            .or_else(|| self.find_hierarchical_join_target(from, direction))
    }

    fn find_direct_sibling_target(&self, from: N, direction: Direction) -> Option<N> {
        let f = &self.forest;
        f.parent(from)?;
        if backwards(direction) {
            f.prev_sibling(from)
        } else {
            f.next_sibling(from)
        }
    }

    fn find_best_container_child_for_joining(
        &self,
        container: N,
        direction: Direction,
    ) -> Option<N> {
        let f = &self.forest;
        let first = f.first_child(container)?;
        if f.container_kind(container).orientation() != direction.orientation() {
            return f.local_selection(container).or(Some(first));
        }
        if backwards(direction) {
            Some(first)
        } else {
            f.last_child(container)
        }
    }

    fn find_hierarchical_join_target(&self, from: N, direction: Direction) -> Option<N> {
        self.forest.ancestors(from).skip(1).find_map(|ancestor| {
            self.find_direct_sibling_target(ancestor, direction).map(|target| {
                self.find_best_container_child_for_joining(target, opposite(direction))
            })
        })?
    }

    fn perform_natural_join(&mut self, selection: N, target: N, direction: Direction) {
        let f = &self.forest;
        let selection_parent = f.parent(selection);
        let target_parent = f.parent(target);
        let selection_stack = selection_parent.filter(|&parent| f.is_stacked(parent));
        let target_stack = target_parent.filter(|&parent| f.is_stacked(parent));
        let back = backwards(direction);
        let join = match (selection_stack, target_stack) {
            (Some(stack), None) => Some((stack, target, back)),
            (None, Some(stack)) => Some((stack, selection, !back)),
            _ => None,
        };
        if let Some((stack, moving, prepend)) = join {
            let placement = match f.first_child(stack).filter(|_| prepend) {
                Some(first) => Placement::Before(first),
                None => Placement::Back(stack),
            };
            self.forest.move_node(moving, placement);
            self.forest.select_node(stack);
            return;
        }
        let kind = Kind::Split(direction.orientation());
        if let (Some(sp), Some(tp)) = (selection_parent, target_parent) {
            if sp == tp && self.forest.is_stacked(sp) {
                self.forest.set_kind(sp, kind);
                self.forest.select_node(sp);
                return;
            }
            if sp != tp && self.are_containers_mergeable(sp, tp, direction) {
                let parent = self.find_or_create_smart_common_parent(sp, tp, direction);
                self.forest.set_kind(parent, kind);
                self.forest.select_node(parent);
                return;
            }
        }
        let parent = self.find_or_create_smart_common_parent(selection, target, direction);
        if !self.forest.is_stacked(parent) {
            self.forest.set_kind(parent, kind);
        }
        self.forest.select_node(parent);
    }

    fn are_containers_mergeable(&self, a: N, b: N, direction: Direction) -> bool {
        let along = Kind::Split(direction.orientation());
        self.forest.kind(a) == Some(along) && self.forest.kind(b) == Some(along)
    }

    fn find_or_create_smart_common_parent(&mut self, a: N, b: N, direction: Direction) -> N {
        let f = &self.forest;
        if let (Some(pa), Some(pb)) = (f.parent(a), f.parent(b))
            && pa == pb
            && f.container_kind(pa) == Kind::Split(direction.orientation())
            && f.children(pa).len() == 2
        {
            return pa;
        }
        self.find_or_create_common_parent(a, b)
    }

    fn find_or_create_common_parent(&mut self, a: N, b: N) -> N {
        let f = &mut self.forest;
        let ancestor = f.common_ancestor(a, b).expect("nodes of the same tree");
        if ancestor == a || ancestor == b {
            return ancestor;
        }
        if f.parent(a) == Some(ancestor) && f.parent(b) == Some(ancestor) {
            let (size_a, size_b) = (f.size(a).max(0.0), f.size(b).max(0.0));
            let container = f.insert(Placement::Before(a));
            // Both shares go to the replacement before the moves normalize the siblings left,
            // so every surviving weight scales together.
            f.set_size(container, size_a + size_b);
            f.set_size(a, 0.0);
            f.set_size(b, 0.0);
            f.move_node(a, Placement::Back(container));
            f.move_node(b, Placement::Back(container));
            f.set_size(a, size_a);
            f.set_size(b, size_b);
            return container;
        }
        let container = f.insert(Placement::Back(ancestor));
        f.set_kind(container, Kind::NEW);
        f.move_node(a, Placement::Back(container));
        f.move_node(b, Placement::Back(container));
        container
    }

    fn unjoin_selection(&mut self, layout: L) -> bool {
        if self.forest.nothing_takes_part(layout) {
            return false;
        }
        let f = &mut self.forest;
        let Some(parent) = f.parent(f.selection(layout)) else {
            return false;
        };
        let Some(grandparent) = f.parent(parent) else {
            return false;
        };
        let chosen = f.local_selection_of(parent);
        let next_sibling = f.next_sibling(parent);
        let parent_size = f.size(parent).max(0.0);
        // Weights read before the moves normalize the remaining siblings.
        let children: Vec<(N, f32)> = f
            .children(parent)
            .iter()
            .map(|&child| (child, f.size(child).max(0.0)))
            .collect();
        let total: f32 = children.iter().map(|(_, weight)| weight).sum();
        let count = children.len() as f32;
        for (child, weight) in children {
            let promoted = if total.is_finite() && total > f32::EPSILON {
                parent_size * weight / total
            } else {
                parent_size / count
            };
            let placement = match next_sibling {
                Some(next) => Placement::Before(next),
                None => Placement::Back(grandparent),
            };
            f.move_node(child, placement);
            f.set_size(child, promoted);
        }
        f.remove_node(parent);
        if let Some(selected) = chosen.or_else(|| f.first_child(grandparent)) {
            f.select_node(selected);
        }
        true
    }
}

/// Which side of the screen the master region takes (upstream's `MasterStackSide`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasterSide {
    Left,
    Right,
    Top,
    Bottom,
}

/// Where a new window goes once the master region is full (upstream's
/// `MasterStackNewWindowPlacement`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewWindowPlacement {
    Master,
    Stack,
    /// The region holding the focused window.
    Focused,
}

/// A region's orientation: across the root's (derived from the side), or set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrangement {
    Derived,
    Set(Orientation),
}

/// Where a new window goes within its region (upstream's optional `window_insertion_point`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionInsertion {
    /// After the selection when it is in the region, else at the back.
    NextToSelection,
    /// At the back, also after the master count is enforced.
    EndOfTree,
    /// At the front (no insertion point configured).
    Front,
}

/// The master-stack settings (upstream's `MasterStackSettings`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MasterSettings {
    pub insertion: RegionInsertion,
    pub ratio: f64,
    pub count: usize,
    pub side: MasterSide,
    pub placement: NewWindowPlacement,
    pub master: Arrangement,
    pub stack: Arrangement,
}

/// The master-stack layout system: the forest, each layout's root a region pair, and its
/// settings (upstream's `MasterStackLayoutSystem`).
#[derive(Debug, Clone, PartialEq)]
pub struct MasterStack<L, N, W> {
    forest: Forest<L, N, W>,
    settings: MasterSettings,
}

impl<L: SlotKey + Eq, N: SlotKey + Eq + Ord, W: Copy + Ord> MasterStack<L, N, W> {
    pub fn new(settings: MasterSettings) -> Self { Self::from_parts(Forest::default(), settings) }

    pub fn from_parts(forest: Forest<L, N, W>, settings: MasterSettings) -> Self {
        Self { forest, settings }
    }

    pub fn forest(&self) -> &Forest<L, N, W> { &self.forest }

    pub fn settings(&self) -> MasterSettings { self.settings }

    fn root_orientation(&self) -> Orientation {
        match self.settings.side {
            MasterSide::Left | MasterSide::Right => Orientation::Horizontal,
            MasterSide::Top | MasterSide::Bottom => Orientation::Vertical,
        }
    }

    fn arranged(&self, arrangement: Arrangement) -> Orientation {
        match arrangement {
            Arrangement::Derived => perpendicular(self.root_orientation()),
            Arrangement::Set(orientation) => orientation,
        }
    }

    fn master_orientation(&self) -> Orientation { self.arranged(self.settings.master) }

    fn stack_orientation(&self) -> Orientation { self.arranged(self.settings.stack) }

    fn master_first(&self) -> bool {
        matches!(self.settings.side, MasterSide::Left | MasterSide::Top)
    }

    /// A layout's root and its (master, stack) regions.
    fn region_roots(&self, layout: L) -> (N, N, N) {
        let root = self.forest.root(layout);
        let (master, stack) = self.forest.regions(root).expect("a master-stack region pair");
        (root, master, stack)
    }

    /// Settings changed: each layout's regions rearranged, the ratio applied when it changed,
    /// the master count enforced (upstream's `update_settings`).
    pub fn update_settings(&mut self, settings: MasterSettings) {
        if self.settings == settings {
            return;
        }
        let with_new_insertion = MasterSettings {
            insertion: settings.insertion,
            ..self.settings
        };
        if with_new_insertion == settings {
            self.settings = settings;
            return;
        }
        let old_ratio = self.settings.ratio;
        self.settings = settings;
        let layouts: Vec<L> = self.forest.layouts.keys().collect();
        for layout in layouts {
            let (root, master, stack) = self.region_roots(layout);
            let master_first = self.master_first();
            self.forest.set_primary_region_first(root, master_first);
            self.forest.set_kind(root, Kind::Split(self.root_orientation()));
            self.forest.set_kind(master, Kind::Split(self.master_orientation()));
            self.forest.set_kind(stack, Kind::Split(self.stack_orientation()));
            if old_ratio != self.settings.ratio {
                self.apply_master_ratio(master, stack);
            }
            self.distribute_regions(layout);
        }
    }

    pub fn set_window_participation(&mut self, window: W, participation: Participation) -> bool {
        self.forest.set_window_participation(window, participation)
    }

    pub fn create_layout(&mut self) -> L {
        let layout = self.forest.create_layout();
        let root = self.forest.root(layout);
        let (master, stack) = self.create_containers(root);
        self.apply_master_ratio(master, stack);
        self.forest.set_region_pair(root, master);
        layout
    }

    pub fn clone_layout(&mut self, layout: L) -> L { self.forest.clone_layout(layout) }

    pub fn remove_layout(&mut self, layout: L) { self.forest.remove_layout(layout) }

    pub fn replace_window(&mut self, from: W, to: W) { self.forest.replace_window(from, to) }

    pub fn resize_window(&mut self, layout: L, window: W, resize: Resize) {
        self.forest.resize_window(layout, window, resize)
    }

    pub fn focus(&mut self, layout: L, focus: Focus<W>) -> Focused<W> {
        self.forest.focus(layout, focus)
    }

    /// A window removed (with or without rebalancing: master-stack has one removal), each
    /// layout it was in redistributed.
    pub fn remove_window(&mut self, window: W) {
        for layout in self.forest.remove_window(window) {
            self.distribute_regions(layout);
        }
    }

    pub fn close_app(&mut self, is_app: impl Fn(W) -> bool) {
        let layouts: Vec<L> = self
            .forest
            .layouts
            .keys()
            .filter(|&layout| self.forest.has_windows_for_app(layout, &is_app))
            .collect();
        self.forest.close_app(is_app);
        for layout in layouts {
            self.distribute_regions(layout);
        }
    }

    fn create_containers(&mut self, root: N) -> (N, N) {
        self.forest.set_kind(root, Kind::Split(self.root_orientation()));
        let first = self.forest.insert(Placement::Back(root));
        let second = self.forest.insert(Placement::Back(root));
        let (master, stack) = if self.master_first() {
            (first, second)
        } else {
            (second, first)
        };
        self.forest.set_kind(master, Kind::Split(self.master_orientation()));
        self.forest.set_kind(stack, Kind::Split(self.stack_orientation()));
        (master, stack)
    }

    fn apply_master_ratio(&mut self, master: N, stack: N) {
        let ratio = self.settings.ratio.clamp(0.05, 0.95) as f32;
        let total = 2.0_f32;
        let master_size = (ratio * total).max(0.05);
        let stack_size = (total - master_size).max(0.05);
        self.forest.set_size(master, master_size);
        self.forest.set_size(stack, stack_size);
    }

    fn distribute_regions(&mut self, layout: L) -> bool {
        let (_, master, stack) = self.region_roots(layout);
        let current = self.forest.count_windows(master);
        self.enforce_master_count(layout, master, stack, current, None)
    }

    /// Windows moved between the regions until the master region holds the configured count
    /// (upstream's `enforce_master_count`).
    fn enforce_master_count(
        &mut self,
        layout: L,
        master: N,
        stack: N,
        current: usize,
        keep_in_master: Option<W>,
    ) -> bool {
        let desired = self.settings.count;
        if current == desired {
            return false;
        }
        let overflow = current > desired;
        let count = current.abs_diff(desired);
        let (source, destination) = if overflow {
            (master, stack)
        } else {
            (stack, master)
        };
        let forward = !overflow && self.master_first();
        let start = if forward {
            source
        } else {
            self.forest.last_descendant(source)
        };
        let protected = keep_in_master
            .filter(|_| overflow && desired != 0)
            .and_then(|window| self.forest.window_node(layout, window));
        let next_window = |f: &Forest<L, N, W>, start: Option<N>| {
            std::iter::successors(start, |&node| f.step(node, source, forward))
                .find(|&node| Some(node) != protected && f.window(node).is_some())
        };
        let mut next = next_window(&self.forest, Some(start));
        if next.is_none() {
            return false;
        }
        let selected = self.forest.focused_leaf(layout).map(|(node, _)| node);
        let front = overflow == self.master_first();
        let mut remaining = count;
        while let Some(leaf) = next {
            let f = &self.forest;
            let (mut node, mut moved) = (leaf, 1);
            if remaining > 1 {
                // Each sibling subtree counted once, walking out from the leaf: a whole group
                // moves at once while a partial quota still moves leaves.
                let mut child = leaf;
                let mut size = 1;
                while let Some(parent) = f.parent(child).filter(|&node| node != source) {
                    size += f
                        .children(parent)
                        .iter()
                        .filter(|&&node| node != child)
                        .map(|&node| f.count_windows(node))
                        .sum::<usize>();
                    if size > remaining || protected.is_some_and(|keep| f.is_within(keep, parent)) {
                        break;
                    }
                    if size > 1 {
                        (node, moved) = (parent, size);
                    }
                    child = parent;
                }
            }
            let restore_focus = selected.filter(|&focus| f.is_within(focus, node));
            remaining -= moved;
            next = if remaining == 0 {
                None
            } else {
                // The next leaf keeps its identity even if moving this branch collapses an
                // ancestor.
                let edge = if forward {
                    f.last_descendant(node)
                } else {
                    node
                };
                next_window(f, f.step(edge, source, forward))
            };
            self.move_to_region(node, destination, front);
            if let Some(focus) = restore_focus {
                self.forest.select_node(focus);
            }
        }
        true
    }

    fn move_to_region(&mut self, node: N, region: N, front: bool) {
        let placement = match self.forest.first_child(region) {
            Some(first) if front => Placement::Before(first),
            _ => Placement::Back(region),
        };
        self.forest.move_node(node, placement);
    }

    fn focused_container(&self, layout: L, master: N, stack: N) -> Option<N> {
        let (node, _) = self.forest.focused_leaf(layout)?;
        self.forest
            .ancestors(node)
            .find(|&ancestor| ancestor == master || ancestor == stack)
    }

    fn target_container_for_new_window(&self, layout: L, master: N, stack: N, count: usize) -> N {
        if count < self.settings.count {
            return master;
        }
        match self.settings.placement {
            NewWindowPlacement::Master => master,
            NewWindowPlacement::Stack => stack,
            NewWindowPlacement::Focused => {
                self.focused_container(layout, master, stack).unwrap_or(master)
            }
        }
    }

    fn insert_window_in_container(&mut self, layout: L, container: N, window: W) -> N {
        let f = &mut self.forest;
        let relative = match self.settings.insertion {
            RegionInsertion::NextToSelection => {
                let selection = f.selection(layout);
                (f.parent(selection) == Some(container)).then_some(Placement::After(selection))
            }
            RegionInsertion::EndOfTree => None,
            RegionInsertion::Front => f.first_child(container).map(Placement::Before),
        };
        f.insert_window(relative.unwrap_or(Placement::Back(container)), window)
    }

    /// A window added to the region the settings choose, the master count then enforced
    /// (upstream's `add_window_after_selection`; a direction is not read).
    pub fn add_window(&mut self, layout: L, window: W) {
        if self.forest.contains_window(layout, window) {
            return;
        }
        let (_, master, stack) = self.region_roots(layout);
        let count = self.forest.count_windows(master);
        let target = self.target_container_for_new_window(layout, master, stack, count);
        let node = self.insert_window_in_container(layout, target, window);
        self.forest.select_node(node);
        let insertion = self.settings.insertion;
        let keep =
            (insertion == RegionInsertion::NextToSelection && target == master).then_some(window);
        self.enforce_master_count(
            layout,
            master,
            stack,
            count + usize::from(target == master),
            keep,
        );
        if insertion == RegionInsertion::EndOfTree
            && let Some(parent) = self.forest.parent(node)
        {
            // Appended in the final region, after the master overflow is resolved.
            self.forest.move_node(node, Placement::Back(parent));
            self.forest.select_node(node);
        }
    }

    pub fn adjust_master_ratio(&mut self, delta: f64) {
        let next = (self.settings.ratio + delta).clamp(0.05, 0.95);
        if (next - self.settings.ratio).abs() < f64::EPSILON {
            return;
        }
        self.settings.ratio = next;
        let layouts: Vec<L> = self.forest.layouts.keys().collect();
        for layout in layouts {
            self.distribute_regions(layout);
            let (_, master, stack) = self.region_roots(layout);
            self.apply_master_ratio(master, stack);
        }
    }

    pub fn adjust_master_count(&mut self, delta: i32) {
        let current = self.settings.count as i32;
        let next = (current + delta).max(1) as usize;
        if next == self.settings.count {
            return;
        }
        self.settings.count = next;
        let layouts: Vec<L> = self.forest.layouts.keys().collect();
        for layout in layouts {
            self.distribute_regions(layout);
        }
    }

    pub fn promote_to_master(&mut self, layout: L) {
        let (_, master, stack) = self.region_roots(layout);
        let Some((node, focused)) = self.forest.focused_leaf(layout) else {
            return;
        };
        let overflow = self.forest.last_descendant(master);
        let promoted = match self.forest.window(overflow) {
            Some(overflow_window)
                if !self.forest.is_within(node, master)
                    && self.forest.count_windows(master) >= self.settings.count =>
            {
                let swapped = self.forest.swap_windows(layout, focused, overflow_window);
                let (demoted, promoted) = if swapped {
                    (node, overflow)
                } else {
                    (overflow, node)
                };
                self.move_to_region(demoted, stack, true);
                promoted
            }
            _ => node,
        };
        self.move_to_region(promoted, master, true);
        let current = self.forest.count_windows(master);
        if current > self.settings.count {
            self.enforce_master_count(layout, master, stack, current, Some(focused));
        }
        self.forest.select_node(promoted);
    }

    /// The window chosen in a region: its chosen child (or first), the first window under it.
    fn focused_window_in_container(&self, container: N) -> Option<W> {
        let f = &self.forest;
        let candidate = f.local_selection(container).or_else(|| f.first_child(container))?;
        f.preorder(candidate).find_map(|node| f.window(node))
    }

    pub fn swap_master_stack(&mut self, layout: L) {
        let (_, master, stack) = self.region_roots(layout);
        let (Some(master_window), Some(stack_window)) = (
            self.focused_window_in_container(master),
            self.focused_window_in_container(stack),
        ) else {
            return;
        };
        self.forest.swap_windows(layout, master_window, stack_window);
    }

    pub fn can_move_window(&self, layout: L, window: W, direction: Direction) -> bool {
        self.forest.window_participates(window)
            && self
                .forest
                .window_node(layout, window)
                .is_some_and(|node| self.move_target(layout, node, direction).is_some())
    }

    pub fn edit(&mut self, layout: L, edit: Edit<W>) -> Edited<W> {
        let f = &mut self.forest;
        match edit {
            Edit::Move { direction, .. } => edited(self.move_selection(layout, direction)),
            Edit::Fullscreen { within_gaps } => {
                Edited::Raise(f.toggle_fullscreen(layout, within_gaps))
            }
            Edit::Swap(a, b) => edited(f.swap_windows(layout, a, b)),
            Edit::ReorderTab { window, target } => edited(f.move_stack_tab(layout, window, target)),
            Edit::Resize { amount, along } => f.resize_selection(layout, amount, along),
            Edit::Drop { source, target, action } => {
                edited(f.drop_window(layout, source, target, action))
            }
            Edit::Split(_)
            | Edit::Join(_)
            | Edit::ConsumeOrExpel(_)
            | Edit::Stack(_)
            | Edit::Unstack(_)
            | Edit::Unjoin
            | Edit::Rebalance
            | Edit::ToggleOrientation => edited(self.distribute_regions(layout)),
        }
    }

    fn move_selection(&mut self, layout: L, direction: Direction) -> bool {
        let Some((node, focused)) = self.forest.focused_leaf(layout) else {
            return false;
        };
        let Some(target) = self.move_target(layout, node, direction) else {
            return false;
        };
        if !self.forest.swap_windows(layout, focused, target) {
            return false;
        }
        self.forest.focus(layout, Focus::Select(focused));
        true
    }

    /// The window a move from `node` swaps with (upstream's `move_target`).
    fn move_target(&self, layout: L, node: N, direction: Direction) -> Option<W> {
        let (_, master, stack) = self.region_roots(layout);
        let f = &self.forest;
        let in_master = f.is_within(node, master);
        let (region, other, axis) = if in_master {
            (master, stack, self.master_orientation())
        } else {
            (stack, master, self.stack_orientation())
        };
        let forward = !backwards(direction);
        let parallel = direction.orientation() == axis;
        let toward_other = direction.orientation() == self.root_orientation()
            && (forward == self.master_first()) == in_master;
        let start = f.step(node, region, forward);
        let mut neighbors = f.windows_from(region, start, forward).peekable();
        let participates = |window| f.window_participates(window);
        if toward_other && (!parallel || neighbors.peek().is_none()) {
            // The destination entered from its near edge, past windows left out.
            let edge = if forward {
                other
            } else {
                f.last_descendant(other)
            };
            let mut destination = f.windows_from(other, Some(edge), forward).peekable();
            if destination.peek().is_some() {
                destination.find(|&window| participates(window))
            } else if in_master {
                // No stack region windows: the next master window, if there is one.
                f.windows_from(region, f.next_in_subtree(node, region), true)
                    .find(|&window| participates(window))
            } else if parallel {
                neighbors.find(|&window| participates(window))
            } else {
                None
            }
        } else if parallel {
            neighbors.find(|&window| participates(window))
        } else {
            None
        }
    }
}
