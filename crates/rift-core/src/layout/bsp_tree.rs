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
}
