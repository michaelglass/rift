//! A traditional (and stack, master-stack) layout's tree as the core holds it: containers of
//! any number of children along an axis or stacked, window leaves, each child's weight, and the
//! selection as what each container remembers. No node ids: a node's identity, for the
//! platform's queries, is a tag the core mints; a place in the tree is the path of child
//! positions the tree itself produces.
//!
//! This module is the state and its read model ([`Projection`], [`View`]); the changes come in a
//! later slice. Every query answers what rift's `TraditionalLayoutSystem` answers for the same
//! tree, including how windows that don't take part are skipped.

use std::fmt::Debug;

use super::bsp_tree::{Direction, Participation, Tag};
use super::{WindowConstraints, traditional};

/// Which way a container lays its children out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

/// How a container shows its children: side by side along an axis, or stacked (one shown),
/// remembering the axis it goes back to when unstacked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Axis(Orientation),
    Stack {
        along: Orientation,
        ungrouped: Orientation,
    },
}

impl Kind {
    fn orientation(self) -> Orientation {
        match self {
            Kind::Axis(orientation) | Kind::Stack { along: orientation, .. } => orientation,
        }
    }

    fn is_stack(self) -> bool { matches!(self, Kind::Stack { .. }) }
}

/// How a node fills its place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    Tiled,
    Screen,
    WithinGaps,
}

/// A child's share of its container: rift's size, a finite number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weight(f32);

impl Weight {
    pub const ONE: Self = Self(1.0);

    /// `size`, or `None` if it is not finite.
    pub fn new(size: f32) -> Option<Self> { size.is_finite().then_some(Self(size)) }

    pub fn get(self) -> f32 { self.0 }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Child<W> {
    pub weight: Weight,
    pub node: Node<W>,
}

/// Whether the selection, reaching a container, goes on into the child it remembers or stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Walk {
    Enters,
    Stops,
}

/// A container's children (at least one), with the child it remembers, if any.
#[derive(Debug, Clone, PartialEq)]
pub enum Children<W> {
    Remembered {
        before: Vec<Child<W>>,
        at: Box<Child<W>>,
        after: Vec<Child<W>>,
        walk: Walk,
    },
    Unremembered {
        first: Box<Child<W>>,
        rest: Vec<Child<W>>,
    },
}

impl<W> Children<W> {
    /// The children in order.
    pub fn iter(&self) -> impl Iterator<Item = &Child<W>> {
        let (head, middle, tail): (&[Child<W>], &Child<W>, &[Child<W>]) = match self {
            Children::Remembered { before, at, after, .. } => (before, at, after),
            Children::Unremembered { first, rest } => (&[], first, rest),
        };
        head.iter().chain(std::iter::once(middle)).chain(tail.iter())
    }

    /// The position of the child remembered, and the walk; none if nothing is remembered.
    fn remembered(&self) -> Option<(usize, Walk)> {
        match self {
            Children::Remembered { before, walk, .. } => Some((before.len(), *walk)),
            Children::Unremembered { .. } => None,
        }
    }

    /// The position of the child the selection enters, if it enters one (rift's
    /// local_selection).
    fn entered(&self) -> Option<usize> {
        match self.remembered() {
            Some((at, Walk::Enters)) => Some(at),
            Some((_, Walk::Stops)) | None => None,
        }
    }

    fn get(&self, position: usize) -> Option<&Child<W>> { self.iter().nth(position) }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowLeaf<W> {
    pub tag: Tag,
    pub window: W,
    pub fill: Fill,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Container<W> {
    pub tag: Tag,
    pub kind: Kind,
    pub fill: Fill,
    pub children: Children<W>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node<W> {
    Window(WindowLeaf<W>),
    Container(Box<Container<W>>),
}

/// A layout's root: a container that may be empty (a layout with no windows).
#[derive(Debug, Clone, PartialEq)]
pub struct Root<W> {
    pub tag: Tag,
    pub kind: Kind,
    pub fill: Fill,
    pub children: Option<Children<W>>,
}

/// A traditional layout: its root.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout<W> {
    pub root: Root<W>,
}

/// A node's place: the child positions from the root (produced by the tree, never made up).
pub type Path = Vec<usize>;

/// A node, as the read model walks it: the root, or a node below it.
#[derive(Debug, Clone, Copy)]
enum At<'a, W> {
    Root(&'a Root<W>),
    Node(&'a Node<W>),
}

impl<'a, W: Copy + Ord + Debug> At<'a, W> {
    fn children(self) -> Option<&'a Children<W>> {
        match self {
            At::Root(root) => root.children.as_ref(),
            At::Node(Node::Container(container)) => Some(&container.children),
            At::Node(Node::Window(_)) => None,
        }
    }

    fn kind(self) -> Option<Kind> {
        match self {
            At::Root(root) => Some(root.kind),
            At::Node(Node::Container(container)) => Some(container.kind),
            At::Node(Node::Window(_)) => None,
        }
    }

    fn fill(self) -> Fill {
        match self {
            At::Root(root) => root.fill,
            At::Node(Node::Container(container)) => container.fill,
            At::Node(Node::Window(leaf)) => leaf.fill,
        }
    }

    fn window(self) -> Option<W> {
        match self {
            At::Node(Node::Window(leaf)) => Some(leaf.window),
            At::Root(_) | At::Node(Node::Container(_)) => None,
        }
    }

    fn child(self, position: usize) -> Option<At<'a, W>> {
        Some(At::Node(&self.children()?.get(position)?.node))
    }

    /// The nodes under this one (itself first), each with its path from here.
    fn preorder(self) -> Vec<(Path, At<'a, W>)> {
        let mut out = vec![(Path::new(), self)];
        let mut index = 0;
        while index < out.len() {
            let (path, at) = out[index].clone();
            let below: Vec<(Path, At<'a, W>)> = at
                .children()
                .into_iter()
                .flat_map(|children| children.iter())
                .enumerate()
                .map(|(position, child)| {
                    let mut child_path = path.clone();
                    child_path.push(position);
                    (child_path, At::Node(&child.node))
                })
                .collect();
            out.splice(index + 1..index + 1, below);
            index += 1;
        }
        out
    }
}

impl<W: Copy + Ord + Debug> Layout<W> {
    fn top(&self) -> At<'_, W> { At::Root(&self.root) }

    fn at(&self, path: &[usize]) -> Option<At<'_, W>> {
        path.iter().try_fold(self.top(), |at, position| at.child(*position))
    }

    /// The selection: where the containers' memories lead from the root (rift's
    /// current_selection).
    fn selection(&self) -> Path {
        let mut path = Path::new();
        let mut at = self.top();
        while let Some(position) = at.children().and_then(Children::entered) {
            path.push(position);
            match at.child(position) {
                Some(child) => at = child,
                None => break,
            }
        }
        path
    }

    /// The nodes that take part: those holding a window that takes part (rift's
    /// participating_nodes); none are listed while nobody is left out.
    fn active(&self, who: Participation<W>) -> Vec<Path> {
        match who {
            Participation::Everyone => Vec::new(),
            Participation::Except(_) => {
                let nodes = self.top().preorder();
                nodes
                    .iter()
                    .filter(|(path, _)| {
                        nodes.iter().any(|(below, at)| {
                            below.starts_with(path)
                                && at.window().is_some_and(|w| who_takes_part(who, &w))
                        })
                    })
                    .map(|(path, _)| path.clone())
                    .collect()
            }
        }
    }

    /// The first window that takes part under `path`, trying the child remembered first (rift's
    /// find_best_focus_target), and its path.
    fn best_focus(&self, path: &[usize], who: Participation<W>) -> Option<(Path, W)> {
        let at = self.at(path)?;
        if let Some(window) = at.window() {
            return who_takes_part(who, &window).then(|| (path.to_vec(), window));
        }
        let children = at.children()?;
        let remembered = children.remembered().map(|(position, _)| position);
        let mut order: Vec<usize> = remembered.into_iter().collect();
        order.extend((0..children.iter().count()).filter(|position| Some(*position) != remembered));
        order.into_iter().find_map(|position| {
            let mut child = path.to_vec();
            child.push(position);
            self.best_focus(&child, who)
        })
    }

    /// The windows shown under `path`: a stack shows the child it enters (or its first), an axis
    /// all its children (rift's visible_windows_under_internal).
    fn visible_under(&self, path: &[usize], who: Participation<W>, active: &[Path]) -> Vec<W> {
        let takes_part = |p: &Path| match who {
            Participation::Everyone => true,
            Participation::Except(_) => active.contains(p),
        };
        let mut windows = Vec::new();
        let mut pending = vec![path.to_vec()];
        while let Some(path) = pending.pop() {
            if !takes_part(&path) {
                continue;
            }
            let Some(at) = self.at(&path) else { continue };
            let child_path = |position: usize| {
                let mut child = path.clone();
                child.push(position);
                child
            };
            match (at.kind(), at.children()) {
                (Some(kind), Some(children)) if kind.is_stack() => {
                    let shown = children
                        .entered()
                        .filter(|position| takes_part(&child_path(*position)))
                        .or_else(|| {
                            (0..children.iter().count())
                                .find(|position| takes_part(&child_path(*position)))
                        });
                    pending.extend(shown.map(child_path));
                }
                (Some(_), Some(children)) => {
                    let count = children.iter().count();
                    pending.extend((0..count).rev().map(child_path));
                }
                (Some(_), None) | (None, _) => {}
            }
            windows.extend(at.window());
        }
        windows
    }

    /// Where commands act (rift's `selection(layout)`): the selection, or, with someone left out
    /// and nothing that takes part under it, the first window that takes part (else the root).
    fn focus(&self, who: Participation<W>) -> Path {
        let selection = self.selection();
        match who {
            Participation::Except(_) if self.best_focus(&selection, who).is_none() => {
                self.best_focus(&[], who).map(|(path, _)| path).unwrap_or_default()
            }
            Participation::Except(_) | Participation::Everyone => selection,
        }
    }

    pub fn projection(&self, who: Participation<W>) -> Projection<W> {
        let active = self.active(who);
        let selection = self.selection();
        let focus = self.focus(who);
        let nodes = self.top().preorder();
        let selected_window = self
            .at(&selection)
            .and_then(At::window)
            .filter(|window| who_takes_part(who, window))
            .or_else(|| {
                self.best_focus(&selection, who)
                    .or_else(|| self.best_focus(&[], who))
                    .map(|(_, w)| w)
            });
        Projection {
            windows: nodes.iter().filter_map(|(_, at)| at.window()).collect(),
            visible: self.visible_under(&[], who, &active),
            selected_window,
            under_focus: self.visible_under(&focus, who, &active),
            fullscreen: nodes.iter().any(|(path, at)| {
                let takes_part = match who {
                    Participation::Everyone => true,
                    Participation::Except(_) => active.contains(path),
                };
                takes_part && at.fill() != Fill::Tiled
            }),
            selection,
            focus,
        }
    }

    /// The path of `window`'s leaf (the first, in tree order).
    fn path_of(&self, window: W) -> Option<Path> {
        self.top()
            .preorder()
            .into_iter()
            .find(|(_, at)| at.window() == Some(window))
            .map(|(path, _)| path)
    }

    pub fn contains(&self, window: W) -> bool { self.path_of(window).is_some() }

    /// Where `window` is: its leaf's child positions from the root.
    pub fn slot(&self, window: W) -> Option<Path> { self.path_of(window) }

    /// The windows stacked with `window` (its stack's window children, in order), if its
    /// container is a stack.
    pub fn stack_members(&self, window: W) -> Vec<W> {
        let Some(path) = self.path_of(window) else {
            return Vec::new();
        };
        let Some((_, parent)) = path.split_last() else {
            return Vec::new();
        };
        let Some(parent) = self.at(parent) else {
            return Vec::new();
        };
        match (parent.kind(), parent.children()) {
            (Some(kind), Some(children)) if kind.is_stack() => {
                children.iter().filter_map(|child| At::Node(&child.node).window()).collect()
            }
            (Some(_), _) | (None, _) => Vec::new(),
        }
    }

    /// Whether `window` is at or under where commands act.
    pub fn focus_contains(&self, window: W, who: Participation<W>) -> bool {
        self.path_of(window).is_some_and(|path| path.starts_with(&self.focus(who)))
    }

    /// The first window shown toward `direction` from the layout's edge (rift's
    /// window_in_direction_from the root).
    pub fn window_in_direction(&self, direction: Direction, who: Participation<W>) -> Option<W> {
        let (axis, toward_end) = match direction {
            Direction::Left => (Orientation::Horizontal, false),
            Direction::Right => (Orientation::Horizontal, true),
            Direction::Up => (Orientation::Vertical, false),
            Direction::Down => (Orientation::Vertical, true),
        };
        self.window_toward(self.top(), axis, toward_end, who)
    }

    fn window_toward(
        &self,
        at: At<'_, W>,
        axis: Orientation,
        toward_end: bool,
        who: Participation<W>,
    ) -> Option<W> {
        if let Some(window) = at.window() {
            return who_takes_part(who, &window).then_some(window);
        }
        let (kind, children) = (at.kind()?, at.children()?);
        // Along the axis, enter at the near edge; across it, or in a stack, retrace the child
        // entered.
        let entered = (kind.is_stack() || kind.orientation() != axis)
            .then(|| children.entered())
            .flatten();
        if let Some(window) = entered.and_then(|position| {
            self.window_toward(At::Node(&children.get(position)?.node), axis, toward_end, who)
        }) {
            return Some(window);
        }
        let others: Vec<(usize, &Child<W>)> = children
            .iter()
            .enumerate()
            .filter(|(position, _)| Some(*position) != entered)
            .collect();
        let try_child = |(_, child): &(usize, &Child<W>)| {
            self.window_toward(At::Node(&child.node), axis, toward_end, who)
        };
        if toward_end {
            others.iter().find_map(try_child)
        } else {
            others.iter().rev().find_map(try_child)
        }
    }

    /// The shape geometry lays out: the tree pruned to what takes space, or none.
    pub fn shape(
        &self,
        who: Participation<W>,
        constraints: &impl Fn(W) -> Option<WindowConstraints>,
    ) -> Option<traditional::Node<W>> {
        shape(self.top(), who, constraints)
    }

    /// The tree as the platform's draw and container queries show it.
    pub fn view(&self, who: Participation<W>) -> View<W> {
        let focus = self.focus(who);
        view(self.top(), &Path::new(), None, false, &focus)
    }
}

fn who_takes_part<W: Ord>(who: Participation<W>, window: &W) -> bool {
    match who {
        Participation::Everyone => true,
        Participation::Except(left_out) => !left_out.contains(window),
    }
}

fn traditional_fill(fill: Fill) -> traditional::Fill {
    match fill {
        Fill::Tiled => traditional::Fill::Tiled,
        Fill::Screen => traditional::Fill::Screen,
        Fill::WithinGaps => traditional::Fill::WithinGaps,
    }
}

fn shape<W: Copy + Ord + Debug>(
    at: At<'_, W>,
    who: Participation<W>,
    constraints: &impl Fn(W) -> Option<WindowConstraints>,
) -> Option<traditional::Node<W>> {
    let fill = traditional_fill(at.fill());
    if let Some(window) = at.window() {
        return who_takes_part(who, &window).then(|| traditional::Node::Window {
            window,
            constraints: constraints(window),
            fill,
        });
    }
    let (kind, children) = (at.kind()?, at.children()?);
    let entered = children.entered();
    let mut kept: Vec<(usize, traditional::Child<W>)> = children
        .iter()
        .enumerate()
        .filter_map(|(position, child)| {
            shape(At::Node(&child.node), who, constraints)
                .map(|node| (position, traditional::Child { size: child.weight.get(), node }))
        })
        .collect();
    if kept.is_empty() {
        return None;
    }
    match kind {
        Kind::Axis(orientation) => {
            let mut kept = kept.into_iter().map(|(_, child)| child);
            Some(traditional::Node::Axis {
                horizontal: orientation == Orientation::Horizontal,
                fill,
                first: Box::new(kept.next()?),
                rest: kept.collect(),
            })
        }
        Kind::Stack { along, .. } => {
            // The child shown on top: the one entered, if it takes space, else the first.
            let on_top = entered
                .and_then(|e| kept.iter().position(|(position, _)| *position == e))
                .unwrap_or(0);
            let after: Vec<traditional::Child<W>> =
                kept.split_off(on_top + 1).into_iter().map(|(_, c)| c).collect();
            let (_, focused) = kept.pop()?;
            Some(traditional::Node::Stack {
                horizontal: along == Orientation::Horizontal,
                fill,
                before: kept.into_iter().map(|(_, c)| c).collect(),
                focused: Box::new(focused),
                after,
            })
        }
    }
}

/// A node as the platform's draw and container queries report it.
#[derive(Debug, Clone, PartialEq)]
pub struct View<W> {
    pub tag: Tag,
    /// Its weight in its container (none for the root).
    pub weight: Option<Weight>,
    pub fill: Fill,
    /// Whether commands act here.
    pub focused: bool,
    /// Whether its container's selection enters it (rift's ☒).
    pub entered: bool,
    pub kind: ViewKind<W>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ViewKind<W> {
    Window(W),
    Container { kind: Kind, children: Vec<View<W>> },
}

fn view<W: Copy + Ord + Debug>(
    at: At<'_, W>,
    path: &Path,
    weight: Option<Weight>,
    entered: bool,
    focus: &Path,
) -> View<W> {
    let container = |tag: Tag, kind: Kind, children: Option<&Children<W>>| {
        let entered_here = children.and_then(Children::entered);
        let children = children
            .into_iter()
            .flat_map(Children::iter)
            .enumerate()
            .map(|(position, child)| {
                let mut child_path = path.clone();
                child_path.push(position);
                view(
                    At::Node(&child.node),
                    &child_path,
                    Some(child.weight),
                    entered_here == Some(position),
                    focus,
                )
            })
            .collect();
        (tag, ViewKind::Container { kind, children })
    };
    let (tag, kind) = match at {
        At::Root(root) => container(root.tag, root.kind, root.children.as_ref()),
        At::Node(Node::Container(node)) => container(node.tag, node.kind, Some(&node.children)),
        At::Node(Node::Window(leaf)) => (leaf.tag, ViewKind::Window(leaf.window)),
    };
    View {
        tag,
        weight,
        fill: at.fill(),
        focused: path == focus,
        entered,
        kind,
    }
}

impl<W> Node<W> {
    pub fn tag(&self) -> Tag {
        match self {
            Node::Window(leaf) => leaf.tag,
            Node::Container(container) => container.tag,
        }
    }
}

/// What the platform asks of a layout, answered once per change.
#[derive(Debug, Clone, PartialEq)]
pub struct Projection<W> {
    /// Every window, in tree order.
    pub windows: Vec<W>,
    /// The windows shown (a stack shows one child).
    pub visible: Vec<W>,
    /// The window the selection is on, or the first that takes part under it (else anywhere).
    pub selected_window: Option<W>,
    /// Where the containers' memories lead.
    pub selection: Path,
    /// Where commands act.
    pub focus: Path,
    /// The windows shown under the focus.
    pub under_focus: Vec<W>,
    /// Whether a node that takes part fills the screen or the gaps.
    pub fullscreen: bool,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::layout::bsp_tree::Tags;

    fn window(tags: &mut Tags, window: u32) -> Child<u32> {
        Child {
            weight: Weight::ONE,
            node: Node::Window(WindowLeaf {
                tag: tags.mint(),
                window,
                fill: Fill::Tiled,
            }),
        }
    }

    /// [1 | stack(2, 3 entered) | 4], the selection entering the stack's 3.
    fn sample() -> Layout<u32> {
        let mut tags = Tags::default();
        let one = window(&mut tags, 1);
        let (two, three) = (window(&mut tags, 2), window(&mut tags, 3));
        let stack = Child {
            weight: Weight::new(2.0).unwrap(),
            node: Node::Container(Box::new(Container {
                tag: tags.mint(),
                kind: Kind::Stack {
                    along: Orientation::Horizontal,
                    ungrouped: Orientation::Vertical,
                },
                fill: Fill::Tiled,
                children: Children::Remembered {
                    before: vec![two],
                    at: Box::new(three),
                    after: vec![],
                    walk: Walk::Enters,
                },
            })),
        };
        let four = window(&mut tags, 4);
        Layout {
            root: Root {
                tag: tags.mint(),
                kind: Kind::Axis(Orientation::Horizontal),
                fill: Fill::Tiled,
                children: Some(Children::Remembered {
                    before: vec![one],
                    at: Box::new(stack),
                    after: vec![four],
                    walk: Walk::Enters,
                }),
            },
        }
    }

    #[test]
    fn the_selection_follows_memories_and_a_stack_shows_the_child_it_enters() {
        let layout = sample();
        let projection = layout.projection(Participation::Everyone);
        assert_eq!(projection.selection, vec![1, 1]);
        assert_eq!(projection.selected_window, Some(3));
        assert_eq!(projection.windows, vec![1, 2, 3, 4]);
        assert_eq!(projection.visible, vec![1, 3, 4]);
        assert_eq!(projection.under_focus, vec![3]);
        assert_eq!(layout.stack_members(2), vec![2, 3]);
        assert_eq!(layout.stack_members(1), Vec::<u32>::new());
        assert_eq!(layout.slot(4), Some(vec![2]));
        assert!(layout.focus_contains(3, Participation::Everyone));
        assert!(!layout.focus_contains(1, Participation::Everyone));
    }

    #[test]
    fn a_left_out_window_is_skipped_and_focus_finds_another() {
        let layout = sample();
        let left_out = BTreeSet::from([3]);
        let who = Participation::of(&left_out);
        let projection = layout.projection(who);
        // The stack shows its first child that takes part; the selection falls back to 2.
        assert_eq!(projection.visible, vec![1, 2, 4]);
        assert_eq!(projection.selected_window, Some(2));
        // The selection's own window is left out: commands act at the first that takes part.
        assert_eq!(projection.focus, vec![1, 0]);
        assert_eq!(projection.under_focus, vec![2]);
    }

    #[test]
    fn the_shape_is_the_tree_pruned_with_the_stack_showing_its_entered_child() {
        let layout = sample();
        let shape = layout.shape(Participation::Everyone, &|_| None).unwrap();
        let traditional::Node::Axis { rest, .. } = shape else {
            panic!("an axis")
        };
        let traditional::Node::Stack { before, after, .. } = &rest[0].node else {
            panic!("a stack")
        };
        assert_eq!((before.len(), after.len()), (1, 0));
        assert_eq!(rest[0].size, 2.0);
    }
}
