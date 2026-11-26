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
            Body::Container { container, .. } => Held::Container {
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
    /// a tree): its rows, rendered.
    pub fn draw(&self, layout: L) -> String
    where
        N: Debug,
        W: Debug, {
        render(&self.rows(layout))
    }

    /// A layout's drawing as rows, in preorder: each node with the branch glyphs leading to it,
    /// whether its parent's selection goes into it, its id, its window or kind (and a
    /// container's children's total), its size, and its role.
    pub fn rows(&self, layout: L) -> Vec<Row<N, W>> {
        let root = self.root(layout);
        let mut out = Vec::new();
        self.row_of(root, root, Vec::new(), Vec::new(), &mut out);
        out
    }

    /// One node's row and its children's: `lead` the glyphs before it, `rest` the glyphs its
    /// children's rows start with.
    fn row_of(
        &self,
        root: N,
        node: N,
        lead: Vec<Branch>,
        rest: Vec<Branch>,
        out: &mut Vec<Row<N, W>>,
    ) {
        let mark = match self.parent(node) {
            None => Mark::Root,
            Some(parent) if self.local_selection_of(parent) == Some(node) => Mark::Chosen,
            Some(_) => Mark::NotChosen,
        };
        let held = match &self.nodes[node].body {
            Body::Window { window, .. } => Drawn::Window(*window),
            Body::Container { container, .. } => Drawn::Container {
                kind: container.kind,
                total: self.children_total(node),
            },
        };
        out.push(Row {
            branches: lead,
            mark,
            node,
            held,
            size: self.size(node),
            role: self.role(root, node),
        });
        let children = self.children(node);
        for (at, &child) in children.iter().enumerate() {
            let last = at + 1 == children.len();
            let with = |glyph: Branch| {
                let mut glyphs = rest.clone();
                glyphs.push(glyph);
                glyphs
            };
            let (lead, below) = if last {
                (with(Branch::Last), with(Branch::Blank))
            } else {
                (with(Branch::Tee), with(Branch::Through))
            };
            self.row_of(root, child, lead, below, out);
        }
    }
}

/// A glyph leading to a row: before the row's own node, the last of its siblings (`Last`) or not
/// (`Tee`); at an ancestor's depth, past that ancestor's last sibling (`Blank`) or not
/// (`Through`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Branch {
    Blank,
    Last,
    Through,
    Tee,
}

/// Whether a row's node is its parent's selection (none at the root).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Root,
    Chosen,
    NotChosen,
}

/// What a row shows of its node: a window, or a container's kind and its children's total.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Drawn<W> {
    Window(W),
    Container { kind: Kind, total: f32 },
}

/// One node of a layout's drawing.
#[derive(Debug, Clone, PartialEq)]
pub struct Row<N, W> {
    pub branches: Vec<Branch>,
    pub mark: Mark,
    pub node: N,
    pub held: Drawn<W>,
    pub size: f32,
    pub role: Role,
}

/// A drawing's rows as text: the glyphs, ids and numbers as upstream's `draw_tree` writes them
/// (ids by `{:?}`, sizes by `f32`'s display).
pub fn render<N: Debug, W: Debug>(rows: &[Row<N, W>]) -> String {
    let mut out = String::new();
    for row in rows {
        for glyph in &row.branches {
            out.push_str(match glyph {
                Branch::Blank => "   ",
                Branch::Last => " └─",
                Branch::Through => " │ ",
                Branch::Tee => " ├─",
            });
        }
        let mark = match row.mark {
            Mark::Root => "",
            Mark::Chosen => "☒ ",
            Mark::NotChosen => "☐ ",
        };
        let size = row.size;
        let held = match row.held {
            Drawn::Window(ref window) => format!("{window:?} [size {size}]"),
            Drawn::Container { kind, total } => {
                format!("{} [size {size} total={total}]", kind_name(kind))
            }
        };
        out.push_str(&format!(" {mark}{:?} {held}", row.node));
        if let Some(role) = row.role.name() {
            out.push_str(&format!(" [{role}]"));
        }
        out.push('\n');
    }
    out
}
