//! rift-cli's views of a group forest's layout (upstream's `container_tree_with_roles` and
//! `draw_tree`, IPC output): the container view as a value the platform maps to its protocol, and
//! the drawing as text.

use std::fmt::Debug;

use super::{Body, Fill, Forest, Kind, Orientation};
use crate::slot_table::SlotKey;

/// What a node plays in a layout's views: its root region pair's master (primary) or stack
/// (secondary) region, or nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Plain,
    Master,
    Stack,
}

impl Role {
    /// The role's name in rift-cli's views (upstream's labels), none for a plain node.
    pub fn name(self) -> Option<&'static str> {
        match self {
            Role::Plain => None,
            Role::Master => Some("master"),
            Role::Stack => Some("stack"),
        }
    }
}

/// A node as rift's container view shows it: whether it is the layout's selection, its size
/// among its siblings (none at the root), its fill, its role, and what it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerView<N, W> {
    pub node: N,
    pub selected: bool,
    pub weight: Option<f32>,
    pub fill: Fill,
    pub role: Role,
    pub held: Held<N, W>,
}

/// What a node of the container view holds: a window, or a container's kind and children.
#[derive(Debug, Clone, PartialEq)]
pub enum Held<N, W> {
    Window(W),
    Container {
        kind: Kind,
        children: Vec<ContainerView<N, W>>,
    },
}

/// A kind as upstream's `LayoutKind` writes itself (`{:?}`).
fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Split(Orientation::Horizontal) => "Horizontal",
        Kind::Split(Orientation::Vertical) => "Vertical",
        Kind::Stack(Orientation::Horizontal) => "HorizontalStack",
        Kind::Stack(Orientation::Vertical) => "VerticalStack",
    }
}

impl<L: SlotKey + Eq, N: SlotKey + Eq + Ord, W: Copy + Ord> Forest<L, N, W> {
    /// The role `node` plays in the layout rooted at `root`.
    fn role(&self, root: N, node: N) -> Role {
        match self.regions(root) {
            Some((master, _)) if master == node => Role::Master,
            Some((_, stack)) if stack == node => Role::Stack,
            _ => Role::Plain,
        }
    }

    /// A layout's nodes as rift's container view shows them (upstream's
    /// `container_tree_with_roles`, the roles its root's regions).
    pub fn container_view(&self, layout: L) -> ContainerView<N, W> {
        let root = self.root(layout);
        self.container_of(root, root, self.selection(layout))
    }

    fn container_of(&self, root: N, node: N, selected: N) -> ContainerView<N, W> {
        let held = match &self.nodes[node].body {
            Body::Window { window, .. } => Held::Window(*window),
            Body::Container(container) => Held::Container {
                kind: container.kind,
                children: container
                    .members
                    .list()
                    .iter()
                    .map(|&child| self.container_of(root, child, selected))
                    .collect(),
            },
        };
        ContainerView {
            node,
            selected: node == selected,
            weight: self.parent(node).map(|_| self.size(node)),
            fill: self.nodes[node].fill,
            role: self.role(root, node),
            held,
        }
    }

    /// A layout drawn as text (upstream's `draw_tree`, written as the `ascii_tree` crate writes
    /// a tree): each node with whether its parent's selection goes into it, its id, its window
    /// or kind, its size (and a container's children's total), and its role.
    pub fn draw(&self, layout: L) -> String
    where
        N: Debug,
        W: Debug, {
        let root = self.root(layout);
        let mut out = String::new();
        self.draw_node(root, root, &mut Vec::new(), &mut out);
        out
    }

    /// One node's line and its children's, `level` holding how many siblings were left to draw
    /// at each ancestor's depth (this node's included).
    fn draw_node(&self, root: N, node: N, level: &mut Vec<usize>, out: &mut String)
    where
        N: Debug,
        W: Debug, {
        let depth = level.len();
        for (at, &left) in level.iter().enumerate() {
            let last = at + 1 == depth;
            out.push_str(match (left == 1, last) {
                (true, false) => "   ",
                (true, true) => " └─",
                (false, false) => " │ ",
                (false, true) => " ├─",
            });
        }
        let status = match self.parent(node) {
            None => "",
            Some(parent) if self.local_selection_of(parent) == Some(node) => "☒ ",
            Some(_) => "☐ ",
        };
        let size = self.size(node);
        let held = match &self.nodes[node].body {
            Body::Window { window, .. } => format!("{window:?} [size {size}]"),
            Body::Container(container) => format!(
                "{} [size {size} total={}]",
                kind_name(container.kind),
                self.children_total(node)
            ),
        };
        out.push_str(&format!(" {status}{node:?} {held}"));
        if let Some(role) = self.role(root, node).name() {
            out.push_str(&format!(" [{role}]"));
        }
        out.push('\n');
        let children = self.children(node);
        for (at, &child) in children.iter().enumerate() {
            level.push(children.len() - at);
            self.draw_node(root, child, level, out);
            level.pop();
        }
    }
}
