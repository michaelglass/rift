//! A BSP layout's tree as the core holds it: splits of exactly two children, leaves holding
//! nothing, a window, or a stack of windows, and the selection as each split's memory of where
//! it was last entered. No node ids and no child indices: a place in the tree is a path of
//! sides, and a node's identity, for the platform's queries, is a tag the core mints.
//!
//! This module is the state and its read model ([`Projection`]); the events that change it come
//! in a later slice. Every query answers what rift's `BspLayoutSystem` answers for the same
//! tree, including how windows that don't participate (minimized, on another space) are skipped.

use std::collections::BTreeSet;
use std::fmt::{Debug, Write as _};

use super::{WindowConstraints, bsp};

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

/// One of a split's two children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    First,
    Second,
}

/// The first child's share of a split, kept within 0.05..=0.95 by every way of making one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ratio(f32);

impl Ratio {
    pub const HALF: Self = Self(0.5);
    const HIGH: f32 = 0.95;
    const LOW: f32 = 0.05;

    /// `share` within the range, or `None` if it is not a number.
    pub fn clamped(share: f32) -> Option<Self> {
        (!share.is_nan()).then(|| Self(share.clamp(Self::LOW, Self::HIGH)))
    }

    pub fn get(self) -> f32 { self.0 }
}

/// An identity for a node, minted by the core, unique among the nodes it has made: what the
/// platform's tree queries name a node by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tag(u64);

impl Tag {
    pub fn get(self) -> u64 { self.0 }
}

/// Where the next tag comes from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tags(u64);

impl Tags {
    pub fn mint(&mut self) -> Tag {
        self.0 += 1;
        Tag(self.0)
    }
}

/// Where the selection goes when it reaches a split: it stops there, or goes on into a side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Memory {
    Stops,
    Toward(Side),
}

/// How a window leaf fills its place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    Tiled,
    Screen,
    WithinGaps,
}

/// A direction the next window put into a leaf will split it toward, if one was chosen.
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

impl<W: Copy + PartialEq> Tabs<W> {
    /// The windows in stack order.
    pub fn members(&self) -> impl Iterator<Item = W> + '_ {
        self.before
            .iter()
            .copied()
            .chain(std::iter::once(self.top))
            .chain(self.after.iter().copied())
    }

    fn contains(&self, window: W) -> bool { self.members().any(|member| member == window) }
}

/// What a leaf holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Content<W> {
    Empty,
    Window { window: W, fill: Fill },
    Stack { tabs: Tabs<W>, fill: Fill },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Leaf<W> {
    pub tag: Tag,
    pub content: Content<W>,
    pub preselect: Preselect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Split<W> {
    pub tag: Tag,
    pub orientation: Orientation,
    pub ratio: Ratio,
    pub memory: Memory,
    pub first: Node<W>,
    pub second: Node<W>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node<W> {
    Split(Box<Split<W>>),
    Leaf(Leaf<W>),
}

/// The windows that take part in layout: all of them, or all but a set of at least one.
/// Rift's BSP queries treat "nobody is left out" differently from "these are left out" even
/// where the set does not touch the layout, so the distinction is kept.
#[derive(Debug, Clone, Copy)]
pub enum Participation<'a, W> {
    Everyone,
    Except(&'a BTreeSet<W>),
}

impl<'a, W: Ord> Participation<'a, W> {
    /// Everyone if `left_out` is empty.
    pub fn of(left_out: &'a BTreeSet<W>) -> Self {
        if left_out.is_empty() {
            Self::Everyone
        } else {
            Self::Except(left_out)
        }
    }

    fn takes_part(&self, window: &W) -> bool {
        match self {
            Self::Everyone => true,
            Self::Except(left_out) => !left_out.contains(window),
        }
    }
}

/// A node's place in the tree: the sides taken from the root.
pub type Path = Vec<Side>;

/// Where a window is: its leaf's path, and for a stacked window its place in the stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub path: Path,
    pub tab: Tab,
}

/// A window's place in its leaf: alone, or at a position in its stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Alone,
    Stacked(usize),
}

impl<W: Copy + Ord + Debug> Node<W> {
    /// The window the leaf shows among those taking part: its window, or the top of its stack
    /// if that takes part, else the first stacked window that does.
    fn shown(&self, who: Participation<W>) -> Option<W> {
        match self {
            Node::Split(_) => None,
            Node::Leaf(leaf) => match &leaf.content {
                Content::Empty => None,
                Content::Window { window, .. } => Some(*window).filter(|w| who.takes_part(w)),
                Content::Stack { tabs, .. } => Some(tabs.top)
                    .filter(|w| who.takes_part(w))
                    .or_else(|| tabs.members().find(|w| who.takes_part(w))),
            },
        }
    }

    fn holds_window(&self) -> bool {
        matches!(
            self,
            Node::Leaf(Leaf {
                content: Content::Window { .. } | Content::Stack { .. },
                ..
            })
        )
    }

    /// The nodes under this one (itself first), each with its path from here.
    fn preorder(&self) -> Vec<(Path, &Node<W>)> {
        let mut out = Vec::new();
        let mut pending = vec![(Path::new(), self)];
        while let Some((path, node)) = pending.pop() {
            if let Node::Split(split) = node {
                let mut second = path.clone();
                second.push(Side::Second);
                let mut first = path.clone();
                first.push(Side::First);
                pending.push((second, &split.second));
                pending.push((first, &split.first));
            }
            out.push((path, node));
        }
        out
    }

    fn at(&self, path: &[Side]) -> &Node<W> {
        path.iter().fold(self, |node, side| match node {
            Node::Split(split) => match side {
                Side::First => &split.first,
                Side::Second => &split.second,
            },
            Node::Leaf(_) => node,
        })
    }

    /// The selection: where the splits' memories lead from here.
    fn selection(&self) -> Path {
        let mut path = Path::new();
        let mut node = self;
        while let Node::Split(split) = node {
            match split.memory {
                Memory::Stops => break,
                Memory::Toward(side) => {
                    path.push(side);
                    node = match side {
                        Side::First => &split.first,
                        Side::Second => &split.second,
                    };
                }
            }
        }
        path
    }

    /// The leaf a selection at `path` stands for: with someone left out, the first leaf under
    /// it (in order) showing a window; otherwise, and if there is none, its first leaf.
    fn leaf_under(&self, path: &[Side], who: Participation<W>) -> Path {
        let base = self.at(path);
        let found = match who {
            Participation::Everyone => None,
            Participation::Except(_) => {
                base.preorder().into_iter().find(|(_, node)| node.shown(who).is_some())
            }
        };
        let rest = found.map(|(rest, _)| rest).unwrap_or_else(|| {
            let mut rest = Path::new();
            let mut node = base;
            while let Node::Split(split) = node {
                rest.push(Side::First);
                node = &split.first;
            }
            rest
        });
        path.iter().copied().chain(rest).collect()
    }
}

/// A BSP layout: its tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout<W> {
    pub root: Node<W>,
}

/// What the platform asks of a layout, answered once per change.
#[derive(Debug, Clone, PartialEq)]
pub struct Projection<W> {
    /// Every window, in tree order (a stack's in stack order).
    pub windows: Vec<W>,
    /// The window each leaf shows, in tree order.
    pub visible: Vec<W>,
    /// The window the selection is on, if it is on a leaf showing one.
    pub selected_window: Option<W>,
    /// Where the selection is, as the splits' memories lead.
    pub selection: Path,
    /// Where commands act: the selection, moved onto a window that takes part if it is on one
    /// that doesn't; none if nobody in the layout takes part (with someone left out).
    pub focus: Option<Path>,
    /// The windows shown under the focus (its leaf's).
    pub under_focus: Vec<W>,
    /// Whether a leaf showing a window fills the screen or the gaps.
    pub fullscreen: bool,
}

impl<W: Copy + Ord + Debug> Layout<W> {
    pub fn projection(&self, who: Participation<W>) -> Projection<W> {
        let nodes = self.root.preorder();
        let selection = self.root.selection();
        let focus = self.focus(&nodes, &selection, who);
        let under_focus = focus.as_ref().map_or_else(Vec::new, |focus| {
            let leaf = self.root.at(&self.root.leaf_under(focus, who));
            match leaf {
                Node::Leaf(Leaf {
                    content: Content::Window { .. } | Content::Stack { .. },
                    ..
                }) => leaf.shown(who).into_iter().collect(),
                Node::Leaf(_) | Node::Split(_) => Vec::new(),
            }
        });
        Projection {
            windows: nodes
                .iter()
                .flat_map(|(_, node)| match node {
                    Node::Leaf(Leaf {
                        content: Content::Window { window, .. },
                        ..
                    }) => {
                        vec![*window]
                    }
                    Node::Leaf(Leaf {
                        content: Content::Stack { tabs, .. },
                        ..
                    }) => tabs.members().collect(),
                    Node::Leaf(Leaf { content: Content::Empty, .. }) | Node::Split(_) => Vec::new(),
                })
                .collect(),
            visible: nodes
                .iter()
                .filter(|(_, node)| node.holds_window())
                .filter_map(|(_, node)| node.shown(who))
                .collect(),
            selected_window: self.root.at(&selection).shown(who),
            fullscreen: nodes.iter().any(|(_, node)| match node {
                Node::Leaf(Leaf {
                    content: Content::Window { fill, .. } | Content::Stack { fill, .. },
                    ..
                }) => *fill != Fill::Tiled && node.shown(who).is_some(),
                Node::Leaf(Leaf { content: Content::Empty, .. }) | Node::Split(_) => false,
            }),
            selection,
            focus,
            under_focus,
        }
    }

    fn focus(
        &self,
        nodes: &[(Path, &Node<W>)],
        selection: &Path,
        who: Participation<W>,
    ) -> Option<Path> {
        let first_shown = || nodes.iter().find(|(_, node)| node.shown(who).is_some());
        match who {
            Participation::Except(_) if first_shown().is_none() => return None,
            Participation::Except(_) | Participation::Everyone => {}
        }
        let selected = self.root.at(selection);
        if selected.holds_window() && selected.shown(who).is_none() {
            return first_shown().map(|(path, _)| path.clone());
        }
        Some(selection.clone())
    }

    /// The leaf holding `window` (the first, in tree order), and its path.
    fn leaf_of(&self, window: W) -> Option<(Path, &Leaf<W>)> {
        self.root.preorder().into_iter().find_map(|(path, node)| match node {
            Node::Leaf(leaf) => match &leaf.content {
                Content::Window { window: held, .. } if *held == window => Some((path, leaf)),
                Content::Stack { tabs, .. } if tabs.contains(window) => Some((path, leaf)),
                Content::Window { .. } | Content::Stack { .. } | Content::Empty => None,
            },
            Node::Split(_) => None,
        })
    }

    /// The windows stacked with `window`, in stack order (none if it is not stacked).
    pub fn stack_members(&self, window: W) -> Vec<W> {
        match self.leaf_of(window) {
            Some((
                _,
                Leaf {
                    content: Content::Stack { tabs, .. },
                    ..
                },
            )) => tabs.members().collect(),
            Some(_) | None => Vec::new(),
        }
    }

    /// Where `window` is.
    pub fn slot(&self, window: W) -> Option<Slot> {
        let (path, leaf) = self.leaf_of(window)?;
        let tab = match &leaf.content {
            Content::Stack { tabs, .. } => Tab::Stacked(tabs.members().position(|w| w == window)?),
            Content::Window { .. } | Content::Empty => Tab::Alone,
        };
        Some(Slot { path, tab })
    }

    /// The shape geometry lays out: the tree pruned to what takes space, or none if nothing
    /// does. With someone left out, a leaf showing nothing takes no space, not even an empty
    /// one.
    pub fn shape(
        &self,
        who: Participation<W>,
        constraints: &impl Fn(W) -> Option<WindowConstraints>,
    ) -> Option<bsp::Node<W>> {
        shape(&self.root, who, constraints)
    }

    /// The tree as text, one node a line, as rift's `draw_tree` writes it.
    pub fn draw(&self) -> String {
        let mut out = String::new();
        draw(&self.root, &mut out, 0);
        out
    }

    /// The tree for the platform's container query.
    pub fn view(&self) -> View<W> {
        let selection = self.root.at(&self.root.selection()).tag();
        view(&self.root, None, selection)
    }
}

impl<W> Node<W> {
    fn tag(&self) -> Tag {
        match self {
            Node::Split(split) => split.tag,
            Node::Leaf(leaf) => leaf.tag,
        }
    }
}

fn same_nodes<W: PartialEq>(a: &Node<W>, b: &Node<W>) -> bool {
    match (a, b) {
        (Node::Split(a), Node::Split(b)) => {
            a.orientation == b.orientation
                && a.ratio == b.ratio
                && a.memory == b.memory
                && same_nodes(&a.first, &b.first)
                && same_nodes(&a.second, &b.second)
        }
        (Node::Leaf(a), Node::Leaf(b)) => a.content == b.content && a.preselect == b.preselect,
        (Node::Split(_), Node::Leaf(_)) | (Node::Leaf(_), Node::Split(_)) => false,
    }
}

fn shape<W: Copy + Ord + Debug>(
    node: &Node<W>,
    who: Participation<W>,
    constraints: &impl Fn(W) -> Option<WindowConstraints>,
) -> Option<bsp::Node<W>> {
    match node {
        Node::Leaf(leaf) => {
            let (slot, fill) = match &leaf.content {
                // An empty leaf keeps its place only while nobody is left out.
                Content::Empty => match who {
                    Participation::Everyone => (bsp::Slot::Empty, Fill::Tiled),
                    Participation::Except(_) => return None,
                },
                Content::Window { fill, .. } | Content::Stack { fill, .. } => {
                    let window = node.shown(who)?;
                    (
                        bsp::Slot::Window {
                            window,
                            constraints: constraints(window),
                        },
                        *fill,
                    )
                }
            };
            Some(bsp::Node::Leaf {
                slot,
                fill: match fill {
                    Fill::Tiled => bsp::Fill::Tiled,
                    Fill::Screen => bsp::Fill::Screen,
                    Fill::WithinGaps => bsp::Fill::WithinGaps,
                },
                stacked: matches!(leaf.content, Content::Stack { .. }),
            })
        }
        Node::Split(split) => {
            let children = match (
                shape(&split.first, who, constraints),
                shape(&split.second, who, constraints),
            ) {
                (None, None) => return None,
                (Some(one), None) | (None, Some(one)) => bsp::Children::One(Box::new(one)),
                (Some(first), Some(second)) => {
                    bsp::Children::Two(Box::new(first), Box::new(second))
                }
            };
            Some(bsp::Node::Split {
                horizontal: split.orientation == Orientation::Horizontal,
                ratio: split.ratio.get(),
                children,
            })
        }
    }
}

fn draw<W: Debug>(node: &Node<W>, out: &mut String, indent: usize) {
    out.push_str(&"  ".repeat(indent));
    match node {
        Node::Leaf(leaf) => {
            let window = match &leaf.content {
                Content::Empty => None,
                Content::Window { window, .. } => Some(window),
                Content::Stack { tabs, .. } => Some(&tabs.top),
            };
            // Writing to a String does not fail.
            let _ = writeln!(out, "Leaf {window:?}");
        }
        Node::Split(split) => {
            let _ = writeln!(out, "Split {:?} {:.2}", split.orientation, split.ratio.get());
            draw(&split.first, out, indent + 1);
            draw(&split.second, out, indent + 1);
        }
    }
}

/// A node as the platform's container query reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct View<W> {
    pub tag: Tag,
    /// The share of its parent split this node takes (none for the root).
    pub weight: Option<f64>,
    /// Whether the selection is on this node.
    pub selected: bool,
    pub kind: ViewKind<W>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ViewKind<W> {
    Split {
        orientation: Orientation,
        first: Box<View<W>>,
        second: Box<View<W>>,
    },
    Leaf {
        content: Content<W>,
        preselect: Preselect,
    },
}

fn view<W: Clone>(node: &Node<W>, weight: Option<f64>, selection: Tag) -> View<W> {
    let kind = match node {
        Node::Split(split) => {
            let share = f64::from(split.ratio.get());
            ViewKind::Split {
                orientation: split.orientation,
                first: Box::new(view(&split.first, Some(share), selection)),
                second: Box::new(view(&split.second, Some(1.0 - share), selection)),
            }
        }
        Node::Leaf(leaf) => ViewKind::Leaf {
            content: leaf.content.clone(),
            preselect: leaf.preselect,
        },
    };
    View {
        tag: node.tag(),
        weight,
        selected: node.tag() == selection,
        kind,
    }
}

// ---------------------------------------------------------------------------------------------
// Changing the tree. Each change does what rift's BspLayoutSystem does to its tree and its
// Selection, so the two can be held to each other step by step.

impl Direction {
    fn orientation(self) -> Orientation {
        match self {
            Direction::Left | Direction::Right => Orientation::Horizontal,
            Direction::Up | Direction::Down => Orientation::Vertical,
        }
    }
}

/// Where a new window goes when its leaf has no preselected direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Insertion {
    /// At the focus (splitting its leaf, or filling it if empty).
    AtFocus,
    /// After the last leaf in tree order.
    EndOfTree,
}

impl<W> Split<W> {
    fn child_mut(&mut self, side: Side) -> &mut Node<W> {
        match side {
            Side::First => &mut self.first,
            Side::Second => &mut self.second,
        }
    }
}

impl<W> Content<W> {
    /// The same content as a fresh leaf holds it: a window's or stack's fill back to tiled.
    fn refilled(self) -> Self {
        match self {
            Content::Empty => Content::Empty,
            Content::Window { window, .. } => Content::Window { window, fill: Fill::Tiled },
            Content::Stack { tabs, .. } => Content::Stack { tabs, fill: Fill::Tiled },
        }
    }
}

impl<W: Copy + Ord + Debug> Node<W> {
    /// The node at `path`, or where the path runs past a leaf, that leaf.
    fn at_mut(&mut self, path: &[Side]) -> &mut Node<W> {
        match path.split_first() {
            None => self,
            Some((side, rest)) => match self {
                Node::Split(split) => split.child_mut(*side).at_mut(rest),
                Node::Leaf(_) => self,
            },
        }
    }

    /// Selects the node at `path`: every split on the way remembers the side toward it, and a
    /// split selected itself stops the selection there.
    fn select(&mut self, path: &[Side]) {
        match (path.split_first(), self) {
            (Some((side, rest)), Node::Split(split)) => {
                split.memory = Memory::Toward(*side);
                split.child_mut(*side).select(rest);
            }
            (None, Node::Split(split)) => split.memory = Memory::Stops,
            (Some(_) | None, Node::Leaf(_)) => {}
        }
    }

    /// The leaf after which leaves come no more, in tree order.
    fn last_leaf(&self) -> Path {
        self.preorder()
            .into_iter()
            .filter(|(_, node)| matches!(node, Node::Leaf(_)))
            .map(|(path, _)| path)
            .last()
            .unwrap_or_default()
    }

    /// The node at `from`'s neighbour toward `direction` by the tree's structure: up to the
    /// nearest split along that axis with something on that side, then down into it to the
    /// leaf nearest the source's centre (tracked through the splits across the axis).
    fn structural_neighbor(&self, from: &[Side], direction: Direction) -> Option<Path> {
        let mut anchor = 0.5;
        for depth in (0..from.len()).rev() {
            let parent = &from[..depth];
            let side = from[depth];
            let Node::Split(split) = self.at(parent) else { continue };
            if split.orientation == direction.orientation() {
                let target = match (direction, side) {
                    (Direction::Left | Direction::Up, Side::Second) => Some(Side::First),
                    (Direction::Right | Direction::Down, Side::First) => Some(Side::Second),
                    (Direction::Left | Direction::Up, Side::First)
                    | (Direction::Right | Direction::Down, Side::Second) => None,
                };
                if let Some(target) = target {
                    let mut path = parent.to_vec();
                    path.push(target);
                    return Some(self.closest_leaf(path, direction, anchor));
                }
            } else {
                let ratio = f64::from(split.ratio.get());
                anchor = match side {
                    Side::First => anchor * ratio,
                    Side::Second => ratio + anchor * (1.0 - ratio),
                };
            }
        }
        None
    }

    fn closest_leaf(&self, mut path: Path, direction: Direction, mut anchor: f64) -> Path {
        while let Node::Split(split) = self.at(&path) {
            let side = if split.orientation == direction.orientation() {
                match direction {
                    Direction::Left | Direction::Up => Side::Second,
                    Direction::Right | Direction::Down => Side::First,
                }
            } else {
                let ratio = f64::from(split.ratio.get());
                if anchor <= ratio {
                    anchor /= ratio;
                    Side::First
                } else {
                    anchor = (anchor - ratio) / (1.0 - ratio);
                    Side::Second
                }
            };
            path.push(side);
        }
        path
    }

    /// The first window shown toward `direction`: the first leaf in tree order (the last, going
    /// left or up) that holds a window, and what it shows.
    fn window_toward(&self, direction: Direction, who: Participation<W>) -> Option<W> {
        match self {
            Node::Leaf(_) if self.holds_window() => self.shown(who),
            Node::Leaf(_) => None,
            Node::Split(split) => {
                let (near, far) = match direction {
                    Direction::Left | Direction::Up => (&split.second, &split.first),
                    Direction::Right | Direction::Down => (&split.first, &split.second),
                };
                near.window_toward(direction, who).or_else(|| far.window_toward(direction, who))
            }
        }
    }

    fn rename(&mut self, from: W, to: W) {
        match self {
            Node::Split(split) => {
                split.first.rename(from, to);
                split.second.rename(from, to);
            }
            Node::Leaf(leaf) => match &mut leaf.content {
                Content::Empty => {}
                Content::Window { window, .. } => {
                    if *window == from {
                        *window = to;
                    }
                }
                Content::Stack { tabs, .. } => {
                    for member in
                        tabs.before.iter_mut().chain([&mut tabs.top]).chain(tabs.after.iter_mut())
                    {
                        if *member == from {
                            *member = to;
                        }
                    }
                }
            },
        }
    }
}

impl<W: Copy + Ord + Debug> Layout<W> {
    /// A layout of one empty leaf.
    pub fn new(tags: &mut Tags) -> Self {
        Layout {
            root: Node::Leaf(Leaf {
                tag: tags.mint(),
                content: Content::Empty,
                preselect: Preselect::Unset,
            }),
        }
    }

    pub fn contains(&self, window: W) -> bool { self.leaf_of(window).is_some() }

    /// Whether `other` is the same tree, nodes' tags aside.
    pub fn same_as(&self, other: &Layout<W>) -> bool { same_nodes(&self.root, &other.root) }

    /// Where commands act: the focus, or with none the selection.
    fn acting_at(&self, who: Participation<W>) -> Path {
        let nodes = self.root.preorder();
        let selection = self.root.selection();
        self.focus(&nodes, &selection, who).unwrap_or(selection)
    }

    /// Puts `window` in the layout, unless it is there: into a preselected split of the focused
    /// leaf, else as `insertion` says.
    pub fn add(&mut self, window: W, insertion: Insertion, who: Participation<W>, tags: &mut Tags) {
        if self.contains(window) {
            return;
        }
        match insertion {
            Insertion::EndOfTree => {
                let last = self.root.last_leaf();
                self.root.select(&last);
                self.insert_at_selection(window, who, tags);
            }
            Insertion::AtFocus => {
                if !self.insert_preselected(window, who, tags) {
                    self.insert_at_selection(window, who, tags);
                }
            }
        }
    }

    /// Splits the focused leaf toward its preselected direction, if it has one.
    fn insert_preselected(&mut self, window: W, who: Participation<W>, tags: &mut Tags) -> bool {
        let nodes = self.root.preorder();
        let selection = self.root.selection();
        let Some(focus) = self.focus(&nodes, &selection, who) else {
            return false;
        };
        let at = self.root.leaf_under(&focus, who);
        let Node::Leaf(Leaf {
            preselect: Preselect::Toward(direction),
            ..
        }) = self.root.at(&at)
        else {
            return false;
        };
        let direction = *direction;
        let side_of_new = match direction {
            Direction::Left | Direction::Up => Side::First,
            Direction::Right | Direction::Down => Side::Second,
        };
        let new = Content::Window { window, fill: Fill::Tiled };
        self.split_leaf(&at, new, direction.orientation(), side_of_new, tags);
        true
    }

    /// Puts `window` at the selection: into its leaf if empty, else splitting it (alternating
    /// orientation with depth), and selects it.
    fn insert_at_selection(&mut self, window: W, who: Participation<W>, tags: &mut Tags) {
        let at = self.acting_at(who);
        match self.root.at_mut(&at) {
            Node::Leaf(Leaf {
                content: content @ Content::Empty,
                ..
            }) => {
                *content = Content::Window { window, fill: Fill::Tiled };
            }
            Node::Leaf(Leaf {
                content: Content::Window { .. } | Content::Stack { .. },
                ..
            }) => {
                let orientation = if at.len() % 2 == 0 {
                    Orientation::Horizontal
                } else {
                    Orientation::Vertical
                };
                let new = Content::Window { window, fill: Fill::Tiled };
                self.split_leaf(&at, new, orientation, Side::Second, tags);
            }
            Node::Split(_) => {
                let leaf = self.root.leaf_under(&at, who);
                self.root.select(&leaf);
                self.insert_at_selection(window, who, tags);
            }
        }
    }

    /// Replaces the leaf at `at` by a split (keeping the leaf's tag) of a fresh leaf with what
    /// it held and a fresh leaf holding `new` on `side_of_new`, and selects the new leaf.
    fn split_leaf(
        &mut self,
        at: &[Side],
        new: Content<W>,
        orientation: Orientation,
        side_of_new: Side,
        tags: &mut Tags,
    ) {
        let node = self.root.at_mut(at);
        let Node::Leaf(leaf) = node else { return };
        let tag = leaf.tag;
        let held = std::mem::replace(&mut leaf.content, Content::Empty).refilled();
        let existing = Node::Leaf(Leaf {
            tag: tags.mint(),
            content: held,
            preselect: Preselect::Unset,
        });
        let new = Node::Leaf(Leaf {
            tag: tags.mint(),
            content: new,
            preselect: Preselect::Unset,
        });
        let (first, second) = match side_of_new {
            Side::First => (new, existing),
            Side::Second => (existing, new),
        };
        *node = Node::Split(Box::new(Split {
            tag,
            orientation,
            ratio: Ratio::HALF,
            memory: Memory::Stops,
            first,
            second,
        }));
        let mut path = at.to_vec();
        path.push(side_of_new);
        self.root.select(&path);
    }

    /// Takes one occurrence of `window` out, if there is one: out of its stack (the last left
    /// shows), or out of the tree (its split collapses into the other side), and reselects.
    fn remove_one(&mut self, window: W, who: Participation<W>) -> bool {
        let Some((path, _)) = self.leaf_of(window) else {
            return false;
        };
        let Node::Leaf(leaf) = self.root.at_mut(&path) else {
            return false;
        };
        if let Content::Stack { tabs, fill } = &leaf.content {
            let mut members: Vec<W> = tabs.members().filter(|w| *w != window).collect();
            if let Some(top) = members.pop() {
                leaf.content = Content::Stack {
                    tabs: Tabs {
                        before: members,
                        top,
                        after: Vec::new(),
                    },
                    fill: *fill,
                };
                self.root.select(&path);
                return true;
            }
        }
        leaf.content = Content::Empty;
        if let Some((_, parent)) = path.split_last() {
            self.collapse(parent, path[path.len() - 1]);
        }
        let selection = self.root.selection();
        let leaf = self.root.leaf_under(&selection, who);
        self.root.select(&leaf);
        true
    }

    /// The split at `parent` loses its `gone` side: it becomes the other side, keeping its own
    /// tag; if it was remembering a side, it now remembers its first.
    fn collapse(&mut self, parent: &[Side], gone: Side) {
        let node = self.root.at_mut(parent);
        let Node::Split(split) = node else { return };
        let remembered = split.memory;
        let kept = match gone {
            Side::First => std::mem::replace(
                &mut split.second,
                Node::Leaf(Leaf {
                    tag: split.tag,
                    content: Content::Empty,
                    preselect: Preselect::Unset,
                }),
            ),
            Side::Second => std::mem::replace(
                &mut split.first,
                Node::Leaf(Leaf {
                    tag: split.tag,
                    content: Content::Empty,
                    preselect: Preselect::Unset,
                }),
            ),
        };
        let tag = split.tag;
        *node = match kept {
            Node::Leaf(leaf) => Node::Leaf(Leaf { tag, ..leaf }),
            Node::Split(inner) => Node::Split(Box::new(Split {
                tag,
                memory: match remembered {
                    Memory::Stops => Memory::Stops,
                    Memory::Toward(_) => Memory::Toward(Side::First),
                },
                ..*inner
            })),
        };
    }

    /// Selects `window`'s leaf (bringing it to the top of its stack); false if it is left out
    /// or not here.
    pub fn select_window(&mut self, window: W, who: Participation<W>) -> bool {
        if !who.takes_part(&window) {
            return false;
        }
        let Some((path, _)) = self.leaf_of(window) else {
            return false;
        };
        if let Node::Leaf(Leaf {
            content: Content::Stack { tabs, .. },
            ..
        }) = self.root.at_mut(&path)
        {
            let before: Vec<W> = tabs.members().filter(|w| *w != window).collect();
            *tabs = Tabs {
                before,
                top: window,
                after: Vec::new(),
            };
        }
        self.root.select(&path);
        true
    }

    /// Selects the first leaf of the focus's parent; false at the root.
    pub fn ascend(&mut self, who: Participation<W>) -> bool {
        let nodes = self.root.preorder();
        let selection = self.root.selection();
        let Some(focus) = self.focus(&nodes, &selection, who) else {
            return false;
        };
        let Some((_, parent)) = focus.split_last() else {
            return false;
        };
        let leaf = self.root.leaf_under(parent, who);
        self.root.select(&leaf);
        true
    }

    /// Selects the leaf the focus stands for; false if it is already a leaf.
    pub fn descend(&mut self, who: Participation<W>) -> bool {
        let nodes = self.root.preorder();
        let selection = self.root.selection();
        let Some(focus) = self.focus(&nodes, &selection, who) else {
            return false;
        };
        let leaf = self.root.leaf_under(&focus, who);
        if leaf == focus {
            return false;
        }
        self.root.select(&leaf);
        true
    }

    /// Moves the selection to the focused leaf's neighbour toward `direction` (skipping leaves
    /// that show nothing, while someone is left out) and answers the window it shows.
    pub fn move_focus(&mut self, direction: Direction, who: Participation<W>) -> Option<W> {
        let nodes = self.root.preorder();
        if let Participation::Except(_) = who
            && !nodes.iter().any(|(_, node)| node.holds_window() && node.shown(who).is_some())
        {
            return None;
        }
        let selection = self.root.selection();
        let focus = self.focus(&nodes, &selection, who)?;
        let mut from = self.root.leaf_under(&focus, who);
        let next = loop {
            let next = self.root.structural_neighbor(&from, direction)?;
            match who {
                Participation::Everyone => break next,
                Participation::Except(_) if self.root.at(&next).shown(who).is_some() => break next,
                Participation::Except(_) => from = next,
            }
        };
        self.root.select(&next);
        self.root.at(&next).shown(who)
    }

    /// The first window shown toward `direction`, from the layout's edge.
    pub fn window_in_direction(&self, direction: Direction, who: Participation<W>) -> Option<W> {
        self.root.window_toward(direction, who)
    }
}

/// Which splits a resize may move: those along one axis, or the nearest whichever its axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Along {
    Horizontal,
    Vertical,
    Either,
}

/// A fill a leaf can be toggled into (and out of, back to tiled).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fullscreen {
    Screen,
    WithinGaps,
}

impl<W: Copy + Ord + Debug> Layout<W> {
    /// Toggles the focused leaf into `fullscreen` (out of the other) or back to tiled, if it
    /// holds a window, and answers the window it shows.
    pub fn toggle_fullscreen(
        &mut self,
        fullscreen: Fullscreen,
        who: Participation<W>,
    ) -> Option<W> {
        let nodes = self.root.preorder();
        let selection = self.root.selection();
        let focus = self.focus(&nodes, &selection, who)?;
        let at = self.root.leaf_under(&focus, who);
        let shown = self.root.at(&at).shown(who);
        let Node::Leaf(Leaf {
            content: Content::Window { fill, .. } | Content::Stack { fill, .. },
            ..
        }) = self.root.at_mut(&at)
        else {
            return None;
        };
        let into = match fullscreen {
            Fullscreen::Screen => Fill::Screen,
            Fullscreen::WithinGaps => Fill::WithinGaps,
        };
        *fill = if *fill == into { Fill::Tiled } else { into };
        shown
    }

    /// Every split's ratio becomes its first side's share of the tiles along its axis (a
    /// subtree across the axis counts as one).
    pub fn rebalance(&mut self) { rebalance(&mut self.root, Orientation::Horizontal); }

    /// Flips the nearest split at or above the focus (the root's, with no focus) to the other
    /// axis, unless nobody in the layout takes part (with someone left out).
    pub fn toggle_orientation(&mut self, who: Participation<W>) {
        let nodes = self.root.preorder();
        if let Participation::Except(_) = who
            && !nodes.iter().any(|(_, node)| node.holds_window() && node.shown(who).is_some())
        {
            return;
        }
        let selection = self.root.selection();
        let mut at = self.focus(&nodes, &selection, who).unwrap_or_default();
        loop {
            if let Node::Split(split) = self.root.at_mut(&at) {
                split.orientation = match split.orientation {
                    Orientation::Horizontal => Orientation::Vertical,
                    Orientation::Vertical => Orientation::Horizontal,
                };
                return;
            }
            if at.pop().is_none() {
                return;
            }
        }
    }

    /// Moves the nearest split above the focus that `along` allows by half of `amount`, toward
    /// growing the focus's side. A ratio that would not be a number is left as it was.
    pub fn resize(&mut self, amount: f64, along: Along, who: Participation<W>) {
        let nodes = self.root.preorder();
        let selection = self.root.selection();
        let Some(mut at) = self.focus(&nodes, &selection, who) else {
            return;
        };
        while let Some(side) = at.pop() {
            let Node::Split(split) = self.root.at_mut(&at) else {
                continue;
            };
            let moves = match along {
                Along::Horizontal => split.orientation == Orientation::Horizontal,
                Along::Vertical => split.orientation == Orientation::Vertical,
                Along::Either => true,
            };
            if moves {
                let delta = (amount as f32) * 0.5;
                let share = match side {
                    Side::First => split.ratio.get() + delta,
                    Side::Second => split.ratio.get() - delta,
                };
                if let Some(ratio) = Ratio::clamped(share) {
                    split.ratio = ratio;
                }
                return;
            }
        }
    }
}

impl<W: Copy + Ord + Debug> Layout<W> {
    /// Splits the focused leaf along `orientation`: what it held moves to a fresh first leaf,
    /// an empty leaf takes the second side and is selected. Nothing, if nobody in the layout
    /// takes part (with someone left out).
    pub fn split(&mut self, orientation: Orientation, who: Participation<W>, tags: &mut Tags) {
        let nodes = self.root.preorder();
        if let Participation::Except(_) = who
            && !nodes.iter().any(|(_, node)| node.holds_window() && node.shown(who).is_some())
        {
            return;
        }
        let at = self.root.leaf_under(&self.acting_at(who), who);
        self.split_leaf(&at, Content::Empty, orientation, Side::Second, tags);
    }

    /// Swaps two windows' places. In one stack, `b` goes just behind `a`, which stays on top
    /// (and is selected). Where either is stacked, the two groups trade leaves (the last of
    /// each shows; a group of one is a plain window). Otherwise the two windows trade leaves.
    /// Fills stay with their leaves. False if either is left out or not here.
    pub fn swap(&mut self, a: W, b: W, who: Participation<W>) -> bool {
        if !who.takes_part(&a) || !who.takes_part(&b) {
            return false;
        }
        let (Some((path_a, leaf_a)), Some((path_b, leaf_b))) = (self.leaf_of(a), self.leaf_of(b))
        else {
            return false;
        };
        let stacked = |leaf: &Leaf<W>| matches!(leaf.content, Content::Stack { .. });
        if path_a == path_b {
            let Content::Stack { tabs, .. } = &leaf_a.content else {
                return false;
            };
            let mut members: Vec<W> = tabs.members().filter(|w| *w != a && *w != b).collect();
            members.push(b);
            if let Node::Leaf(Leaf {
                content: Content::Stack { tabs, .. },
                ..
            }) = self.root.at_mut(&path_a)
            {
                *tabs = Tabs {
                    before: members,
                    top: a,
                    after: Vec::new(),
                };
            }
            self.root.select(&path_a);
            return true;
        }
        // What each leaf will hold, as (all but the last, the last) of its new windows.
        let (into_a, into_b) = if stacked(leaf_a) || stacked(leaf_b) {
            // The groups trade leaves: a leaf's group is its stack, or its one window.
            let group = |leaf: &Leaf<W>, window: W| match &leaf.content {
                // A stack's last member is its last after the top, or the top.
                Content::Stack { tabs, .. } => match tabs.after.split_last() {
                    Some((last, middle)) => {
                        let rest = tabs
                            .before
                            .iter()
                            .copied()
                            .chain([tabs.top])
                            .chain(middle.iter().copied());
                        (rest.collect(), *last)
                    }
                    None => (tabs.before.clone(), tabs.top),
                },
                Content::Window { .. } | Content::Empty => (Vec::new(), window),
            };
            (group(leaf_b, b), group(leaf_a, a))
        } else {
            ((Vec::new(), b), (Vec::new(), a))
        };
        let put = |node: &mut Node<W>, (before, top): (Vec<W>, W)| {
            let Node::Leaf(leaf) = node else { return };
            let fill = match &leaf.content {
                Content::Window { fill, .. } | Content::Stack { fill, .. } => *fill,
                Content::Empty => Fill::Tiled,
            };
            leaf.content = if before.is_empty() {
                Content::Window { window: top, fill }
            } else {
                Content::Stack {
                    tabs: Tabs { before, top, after: Vec::new() },
                    fill,
                }
            };
        };
        put(self.root.at_mut(&path_a), into_a);
        put(self.root.at_mut(&path_b), into_b);
        true
    }

    /// Moves the focused leaf (with everything it holds) to its neighbour toward `direction`:
    /// past a sibling neighbour within their split, into an empty neighbour's place, or beside
    /// the neighbour in a new split; its old split collapses, and it is selected. False if the
    /// focus is not a leaf holding a window, or it has no neighbour or no parent.
    pub fn move_selection(
        &mut self,
        direction: Direction,
        who: Participation<W>,
        tags: &mut Tags,
    ) -> bool {
        let nodes = self.root.preorder();
        let selection = self.root.selection();
        let Some(focus) = self.focus(&nodes, &selection, who) else {
            return false;
        };
        let moving_at = self.root.leaf_under(&focus, who);
        if !self.root.at(&moving_at).holds_window() {
            return false;
        }
        let mut from = moving_at.clone();
        let target_at = loop {
            let Some(next) = self.root.structural_neighbor(&from, direction) else {
                return false;
            };
            match who {
                Participation::Everyone => break next,
                Participation::Except(_) if self.root.at(&next).shown(who).is_some() => break next,
                Participation::Except(_) => from = next,
            }
        };
        let Some((moving_side, old_parent)) = moving_at.split_last() else {
            return false;
        };
        let (moving_side, old_parent) = (*moving_side, old_parent.to_vec());
        if target_at.split_last().map(|(_, parent)| parent) == Some(&old_parent[..]) {
            // Siblings: they trade sides, the split and its ratio untouched.
            if let Node::Split(split) = self.root.at_mut(&old_parent) {
                std::mem::swap(&mut split.first, &mut split.second);
            }
            let mut moved = old_parent;
            moved.push(match moving_side {
                Side::First => Side::Second,
                Side::Second => Side::First,
            });
            self.root.select(&moved);
            return true;
        }
        // Leaving a placeholder leaf where the moving leaf was changes no path; nor does putting
        // a split where the target leaf is (the old split is not under a leaf).
        let placeholder = Node::Leaf(Leaf {
            tag: self.root.at(&moving_at).tag(),
            content: Content::Empty,
            preselect: Preselect::Unset,
        });
        let moving = std::mem::replace(self.root.at_mut(&moving_at), placeholder);
        let target_node = self.root.at_mut(&target_at);
        let mut moved = target_at.clone();
        if let Node::Leaf(Leaf { content: Content::Empty, .. }) = target_node {
            *target_node = moving;
        } else {
            let target_tag = target_node.tag();
            let target = std::mem::replace(
                target_node,
                Node::Leaf(Leaf {
                    tag: target_tag,
                    content: Content::Empty,
                    preselect: Preselect::Unset,
                }),
            );
            let (first, second, side) = match direction {
                Direction::Left | Direction::Up => (moving, target, Side::First),
                Direction::Right | Direction::Down => (target, moving, Side::Second),
            };
            *target_node = Node::Split(Box::new(Split {
                tag: tags.mint(),
                orientation: direction.orientation(),
                ratio: Ratio::HALF,
                memory: Memory::Stops,
                first,
                second,
            }));
            moved.push(side);
        }
        // The old split loses the placeholder's side; what was under its other side moves up.
        self.collapse(&old_parent, moving_side);
        let kept = match moving_side {
            Side::First => Side::Second,
            Side::Second => Side::First,
        };
        if moved.starts_with(&old_parent) && moved.get(old_parent.len()) == Some(&kept) {
            moved.remove(old_parent.len());
        }
        self.root.select(&moved);
        true
    }
}

/// Rebalances `node`'s splits; answers how many tiles it spans along `parent_axis`.
fn rebalance<W>(node: &mut Node<W>, parent_axis: Orientation) -> usize {
    let Node::Split(split) = node else { return 1 };
    let first = rebalance(&mut split.first, split.orientation);
    let second = rebalance(&mut split.second, split.orientation);
    let span = first + second;
    if let Some(ratio) = Ratio::clamped(first as f32 / span as f32) {
        split.ratio = ratio;
    }
    if split.orientation == parent_axis {
        span
    } else {
        1
    }
}

/// A set of BSP layouts (keyed by the platform's layout id), the windows left out of them all,
/// and where node tags come from.
#[derive(Debug, Clone, PartialEq)]
pub struct Layouts<L, W> {
    layouts: std::collections::BTreeMap<L, Layout<W>>,
    left_out: BTreeSet<W>,
    tags: Tags,
}

impl<L, W> Default for Layouts<L, W> {
    fn default() -> Self {
        Layouts {
            layouts: std::collections::BTreeMap::new(),
            left_out: BTreeSet::new(),
            tags: Tags::default(),
        }
    }
}

impl<L: Copy + Ord, W: Copy + Ord + Debug> Layouts<L, W> {
    pub fn layout(&self, layout: L) -> Option<&Layout<W>> { self.layouts.get(&layout) }

    pub fn participation(&self) -> Participation<'_, W> { Participation::of(&self.left_out) }

    pub fn projection(&self, layout: L) -> Option<Projection<W>> {
        Some(self.layout(layout)?.projection(self.participation()))
    }

    /// A layout built elsewhere (from a saved layout, or the platform's own tree), under the
    /// platform's id, its nodes tagged from this set's tags.
    pub fn adopt(&mut self, layout: L, build: impl FnOnce(&mut Tags) -> Layout<W>) {
        let tree = build(&mut self.tags);
        self.layouts.insert(layout, tree);
    }

    /// The layouts, by the platform's id.
    pub fn iter(&self) -> impl Iterator<Item = (&L, &Layout<W>)> { self.layouts.iter() }

    /// A new layout of one empty leaf, under the platform's id.
    pub fn create(&mut self, layout: L) {
        let new = Layout::new(&mut self.tags);
        self.layouts.insert(layout, new);
    }

    pub fn remove_layout(&mut self, layout: L) { self.layouts.remove(&layout); }

    /// Whether `window` takes part; answers whether that changed.
    pub fn set_participation(&mut self, window: W, takes_part: bool) -> bool {
        if takes_part {
            self.left_out.remove(&window)
        } else {
            self.left_out.insert(window)
        }
    }

    /// Changes one layout, with the participation and the tags it needs.
    fn change<A>(
        &mut self,
        layout: L,
        change: impl FnOnce(&mut Layout<W>, Participation<W>, &mut Tags) -> A,
    ) -> Option<A> {
        let tree = self.layouts.get_mut(&layout)?;
        Some(change(tree, Participation::of(&self.left_out), &mut self.tags))
    }

    pub fn add(&mut self, layout: L, window: W, insertion: Insertion) {
        self.change(layout, |tree, who, tags| tree.add(window, insertion, who, tags));
    }

    pub fn select_window(&mut self, layout: L, window: W) -> bool {
        self.change(layout, |tree, who, _| tree.select_window(window, who))
            .unwrap_or(false)
    }

    pub fn ascend(&mut self, layout: L) -> bool {
        self.change(layout, |tree, who, _| tree.ascend(who)).unwrap_or(false)
    }

    pub fn descend(&mut self, layout: L) -> bool {
        self.change(layout, |tree, who, _| tree.descend(who)).unwrap_or(false)
    }

    pub fn toggle_fullscreen(&mut self, layout: L, fullscreen: Fullscreen) -> Option<W> {
        self.change(layout, |tree, who, _| tree.toggle_fullscreen(fullscreen, who))
            .flatten()
    }

    pub fn rebalance(&mut self, layout: L) { self.change(layout, |tree, _, _| tree.rebalance()); }

    pub fn split(&mut self, layout: L, orientation: Orientation) {
        self.change(layout, |tree, who, tags| tree.split(orientation, who, tags));
    }

    pub fn swap(&mut self, layout: L, a: W, b: W) -> bool {
        self.change(layout, |tree, who, _| tree.swap(a, b, who)).unwrap_or(false)
    }

    pub fn move_selection(&mut self, layout: L, direction: Direction) -> bool {
        self.change(layout, |tree, who, tags| {
            tree.move_selection(direction, who, tags)
        })
        .unwrap_or(false)
    }

    pub fn toggle_orientation(&mut self, layout: L) {
        self.change(layout, |tree, who, _| tree.toggle_orientation(who));
    }

    pub fn resize(&mut self, layout: L, amount: f64, along: Along) {
        self.change(layout, |tree, who, _| tree.resize(amount, along, who));
    }

    pub fn move_focus(&mut self, layout: L, direction: Direction) -> Option<W> {
        self.change(layout, |tree, who, _| tree.move_focus(direction, who)).flatten()
    }

    /// Takes `window` out of every layout (every occurrence) and out of the left-out set.
    pub fn remove_window(&mut self, window: W) {
        self.left_out.remove(&window);
        for tree in self.layouts.values_mut() {
            while tree.remove_one(window, Participation::of(&self.left_out)) {}
        }
    }

    /// `from` is now `to` everywhere: in the left-out set, and in every layout holding `from`
    /// (where any `to` already there goes first).
    pub fn rekey(&mut self, from: W, to: W) {
        if self.left_out.remove(&from) {
            self.left_out.insert(to);
        }
        if from == to {
            return;
        }
        for tree in self.layouts.values_mut() {
            if !tree.contains(from) {
                continue;
            }
            while tree.remove_one(to, Participation::of(&self.left_out)) {}
            tree.root.rename(from, to);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(tags: &mut Tags, window: u32) -> Node<u32> {
        Node::Leaf(Leaf {
            tag: tags.mint(),
            content: Content::Window { window, fill: Fill::Tiled },
            preselect: Preselect::Unset,
        })
    }

    fn split(tags: &mut Tags, memory: Memory, first: Node<u32>, second: Node<u32>) -> Node<u32> {
        Node::Split(Box::new(Split {
            tag: tags.mint(),
            orientation: Orientation::Horizontal,
            ratio: Ratio::HALF,
            memory,
            first,
            second,
        }))
    }

    /// [1 | [2 / stack(3, 4 on top, 5)]], the selection on the stack.
    fn sample() -> Layout<u32> {
        let mut tags = Tags::default();
        let one = leaf(&mut tags, 1);
        let two = leaf(&mut tags, 2);
        let stack = Node::Leaf(Leaf {
            tag: tags.mint(),
            content: Content::Stack {
                tabs: Tabs {
                    before: vec![3],
                    top: 4,
                    after: vec![5],
                },
                fill: Fill::Tiled,
            },
            preselect: Preselect::Unset,
        });
        let right = split(&mut tags, Memory::Toward(Side::Second), two, stack);
        Layout {
            root: split(&mut tags, Memory::Toward(Side::Second), one, right),
        }
    }

    #[test]
    fn the_selection_follows_each_splits_memory_and_stops_where_told() {
        let layout = sample();
        let everyone = layout.projection(Participation::Everyone);
        assert_eq!(everyone.selection, vec![Side::Second, Side::Second]);
        assert_eq!(everyone.selected_window, Some(4));
        assert_eq!(everyone.windows, vec![1, 2, 3, 4, 5]);
        assert_eq!(everyone.visible, vec![1, 2, 4]);
        assert_eq!(everyone.under_focus, vec![4]);
        let mut stopped = layout.clone();
        if let Node::Split(root) = &mut stopped.root {
            root.memory = Memory::Stops;
        }
        let at_root = stopped.projection(Participation::Everyone);
        assert_eq!(at_root.selection, Vec::<Side>::new());
        assert_eq!(at_root.selected_window, None);
        // Commands on a split act on its first leaf.
        assert_eq!(at_root.under_focus, vec![1]);
    }

    #[test]
    fn windows_left_out_are_skipped_and_the_focus_moves_off_them() {
        let layout = sample();
        let left_out = BTreeSet::from([4, 1]);
        let some = layout.projection(Participation::of(&left_out));
        // The stack shows its first member that takes part; 1 shows nothing.
        assert_eq!(some.visible, vec![2, 3]);
        assert_eq!(some.selected_window, Some(3));
        let all = BTreeSet::from([1, 2, 3, 4, 5]);
        let none = layout.projection(Participation::of(&all));
        assert_eq!((none.focus, none.selected_window), (None, None));
        let only_top = BTreeSet::from([2]);
        let selected_on_left_out = Layout {
            root: match layout.root.clone() {
                Node::Split(mut root) => {
                    root.memory = Memory::Toward(Side::Second);
                    if let Node::Split(right) = &mut root.second {
                        right.memory = Memory::Toward(Side::First);
                    }
                    Node::Split(root)
                }
                leaf @ Node::Leaf(_) => leaf,
            },
        };
        // The selection is on 2, which is left out: the focus moves to the first shown, 1.
        let moved = selected_on_left_out.projection(Participation::of(&only_top));
        assert_eq!(moved.focus, Some(vec![Side::First]));
        assert_eq!(moved.under_focus, vec![1]);
    }

    #[test]
    fn stacks_and_slots_name_places_by_side_and_stack_position() {
        let layout = sample();
        assert_eq!(layout.stack_members(5), vec![3, 4, 5]);
        assert_eq!(layout.stack_members(2), Vec::<u32>::new());
        assert_eq!(
            layout.slot(5),
            Some(Slot {
                path: vec![Side::Second, Side::Second],
                tab: Tab::Stacked(2)
            })
        );
        assert_eq!(
            layout.slot(1),
            Some(Slot {
                path: vec![Side::First],
                tab: Tab::Alone
            })
        );
        assert_eq!(layout.slot(9), None);
    }

    #[test]
    fn the_shape_drops_what_shows_nothing_and_the_text_matches_rifts() {
        let layout = sample();
        let left_out = BTreeSet::from([1]);
        let shape = layout.shape(Participation::of(&left_out), &|_| None).unwrap();
        let bsp::Node::Split {
            children: bsp::Children::One(_),
            ..
        } = shape
        else {
            panic!("the root keeps one child: {shape:?}");
        };
        assert_eq!(
            layout.draw(),
            "Split Horizontal 0.50\n  Leaf Some(1)\n  Split Horizontal 0.50\n    Leaf Some(2)\n    Leaf Some(4)\n"
        );
        let view = layout.view();
        assert_eq!((view.weight, view.selected), (None, false));
        let ViewKind::Split { first, second, .. } = view.kind else {
            panic!("a split")
        };
        assert_eq!((first.weight, second.weight), (Some(0.5), Some(0.5)));
    }

    #[test]
    fn a_ratio_is_kept_in_range_and_never_not_a_number() {
        assert_eq!(Ratio::clamped(0.99).map(Ratio::get), Some(0.95));
        assert_eq!(Ratio::clamped(0.01).map(Ratio::get), Some(0.05));
        assert_eq!(Ratio::clamped(f32::NAN), None);
    }

    #[test]
    fn removing_a_window_collapses_its_split_and_the_selection_finds_a_leaf() {
        let mut layouts = Layouts::<u8, u32>::default();
        layouts.create(0);
        for window in [1, 2, 3] {
            layouts.add(0, window, Insertion::AtFocus);
        }
        // [1 | [2 / 3]], the selection on 3.
        assert_eq!(layouts.projection(0).unwrap().selected_window, Some(3));
        layouts.remove_window(3);
        let after = layouts.projection(0).unwrap();
        assert_eq!((after.windows, after.selected_window), (vec![1, 2], Some(2)));
        assert_eq!(
            layouts.layout(0).unwrap().draw(),
            "Split Horizontal 0.50\n  Leaf Some(1)\n  Leaf Some(2)\n"
        );
        layouts.remove_window(1);
        layouts.remove_window(2);
        assert_eq!(layouts.layout(0).unwrap().draw(), "Leaf None\n");
    }

    #[test]
    fn a_stack_losing_a_window_shows_its_last_and_focus_moves_by_structure() {
        let mut tags = Tags::default();
        let stack = Node::Leaf(Leaf {
            tag: tags.mint(),
            content: Content::Stack {
                tabs: Tabs {
                    before: vec![3],
                    top: 4,
                    after: vec![5],
                },
                fill: Fill::Tiled,
            },
            preselect: Preselect::Unset,
        });
        let one = leaf(&mut tags, 1);
        let mut layout = Layout {
            root: split(&mut tags, Memory::Toward(Side::Second), one, stack),
        };
        assert!(layout.remove_one(5, Participation::Everyone));
        assert_eq!(layout.stack_members(3), vec![3, 4]);
        assert!(layout.remove_one(4, Participation::Everyone));
        assert_eq!(layout.stack_members(3), vec![3]);
        assert_eq!(
            layout.move_focus(Direction::Left, Participation::Everyone),
            Some(1)
        );
        assert_eq!(layout.move_focus(Direction::Left, Participation::Everyone), None);
        assert_eq!(
            layout.move_focus(Direction::Right, Participation::Everyone),
            Some(3)
        );
    }
}
