//! The group forest as upstream saves it (`GroupLayouts`, the traditional and master-stack
//! systems' saved form), written from the core byte for byte, and upstream's `GroupLayouts`
//! built from the core (the shadow re-seeded after a divergence).

use std::collections::BTreeMap;

use rift_core::layout::group_forest::{Body, Fill, Members, Memory, Node, Place, Side};
use serde::Serialize;

use super::group_twin::{CoreForest, layout_kind};
use super::groups::{GroupLayouts, LayoutRoot};
use crate::actor::app::WindowId;
use crate::common::collections::HashMap;
use crate::layout_engine::{LayoutId, LayoutKind};
use crate::model::slot_table::{SavedSlot, saved, to_slotmap};
use crate::model::tree::{self, NodeId, NodeMap, Tree, WindowIndex, WindowNodeInfo};

/// The forest as upstream's `GroupLayouts` is saved (its derived form: the tree, then the layout
/// roots).
#[derive(Serialize)]
#[serde(rename = "GroupLayouts")]
pub(super) struct Saved {
    tree: SavedTree,
    layout_roots: Vec<SavedSlot<LayoutRoot>>,
}

#[derive(Serialize)]
#[serde(rename = "Tree")]
struct SavedTree {
    map: SavedMap,
    window: SavedIndex,
}

#[derive(Serialize)]
#[serde(rename = "NodeMap")]
struct SavedMap {
    map: Vec<SavedSlot<SavedNode>>,
}

/// The window index: each window's leaves, by when the window entered each layout.
#[derive(Serialize)]
#[serde(rename = "WindowIndex")]
struct SavedIndex {
    window_nodes: BTreeMap<WindowId, SavedEntries>,
}

#[derive(Serialize)]
#[serde(rename = "WindowNodeInfoVec")]
struct SavedEntries(Vec<SavedEntry>);

#[derive(Serialize)]
#[serde(rename = "WindowNodeInfo")]
struct SavedEntry {
    layout: LayoutId,
    node: NodeId,
}

/// A node as upstream saves it (`StoredNode`): its links, its layout info, a container's
/// selection, a leaf's window, a region pair's order.
#[derive(Serialize)]
#[serde(rename = "StoredNode")]
struct SavedNode {
    parent: Option<NodeId>,
    prev_sibling: Option<NodeId>,
    next_sibling: Option<NodeId>,
    first_child: Option<NodeId>,
    last_child: Option<NodeId>,
    layout: SavedInfo,
    selection: Option<SavedSelection>,
    window: Option<WindowId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    master_first: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename = "LayoutInfo")]
struct SavedInfo {
    size: f32,
    kind: LayoutKind,
    is_fullscreen: bool,
    is_fullscreen_within_gaps: bool,
}

#[derive(Serialize)]
#[serde(rename = "SelectionInfo")]
struct SavedSelection {
    selected_child: NodeId,
    stop_here: bool,
}

/// Each node's siblings, read from its parent's list (a root has none).
fn siblings(forest: &CoreForest) -> HashMap<NodeId, (Option<NodeId>, Option<NodeId>)> {
    let mut siblings = HashMap::default();
    for (_, node) in forest.nodes().iter() {
        if let Body::Container(container) = &node.body {
            let list = container.members.list();
            for (at, &child) in list.iter().enumerate() {
                let prev = at.checked_sub(1).map(|before| list[before]);
                siblings.insert(child, (prev, list.get(at + 1).copied()));
            }
        }
    }
    siblings
}

fn saved_node(
    node: &Node<NodeId, WindowId>,
    (prev_sibling, next_sibling): (Option<NodeId>, Option<NodeId>),
) -> SavedNode {
    let (parent, size) = match node.place {
        Place::Root => (None, 0.0),
        Place::Child { parent, size } => (Some(parent), size),
    };
    let (first_child, last_child, kind, selection, window, master_first) = match &node.body {
        Body::Window { window, .. } => {
            (None, None, LayoutKind::default(), None, Some(*window), None)
        }
        Body::Container(container) => {
            let list = container.members.list();
            let selection = match container.memory {
                Memory::Unset => None,
                Memory::Toward(child) => Some(SavedSelection {
                    selected_child: child,
                    stop_here: false,
                }),
                Memory::Stops(child) => Some(SavedSelection {
                    selected_child: child,
                    stop_here: true,
                }),
            };
            let master_first = match container.members {
                Members::Children(_) => None,
                Members::Regions { primary, .. } => Some(primary == Side::First),
            };
            (
                list.first().copied(),
                list.last().copied(),
                layout_kind(container.kind),
                selection,
                None,
                master_first,
            )
        }
    };
    SavedNode {
        parent,
        prev_sibling,
        next_sibling,
        first_child,
        last_child,
        layout: SavedInfo {
            size,
            kind,
            is_fullscreen: node.fill == Fill::Screen,
            is_fullscreen_within_gaps: node.fill == Fill::WithinGaps,
        },
        selection,
        window,
        master_first,
    }
}

impl Saved {
    /// The core's forest in upstream's saved form.
    pub(super) fn of(forest: &CoreForest) -> Self {
        let siblings = siblings(forest);
        let mut window_nodes: BTreeMap<WindowId, SavedEntries> = BTreeMap::new();
        for (window, layout, node) in forest.index() {
            window_nodes
                .entry(window)
                .or_insert_with(|| SavedEntries(Vec::new()))
                .0
                .push(SavedEntry { layout, node });
        }
        Saved {
            tree: SavedTree {
                map: SavedMap {
                    map: saved(forest.nodes(), |key, node| {
                        saved_node(node, siblings.get(&key).copied().unwrap_or_default())
                    })
                    .collect(),
                },
                window: SavedIndex { window_nodes },
            },
            layout_roots: saved(forest.layout_table(), |_, &root| LayoutRoot(root)).collect(),
        }
    }
}

/// Upstream's forest built from the core's (the shadow re-seeded): the same slots, versions and
/// free lists, the index in the core's order, the windows left out.
pub(super) fn upstream_tiles(forest: &CoreForest) -> GroupLayouts {
    let siblings = siblings(forest);
    let nodes: slotmap::SlotMap<NodeId, tree::Node> = to_slotmap(
        forest.nodes(),
        |key, node| saved_node(node, siblings.get(&key).copied().unwrap_or_default()),
        tree::Node::default(),
    );
    let mut window = WindowIndex::default();
    for (id, layout, node) in forest.index() {
        window
            .window_nodes
            .entry(id)
            .or_default()
            .0
            .push(WindowNodeInfo { layout, node });
    }
    GroupLayouts {
        inactive: forest.left_out().iter().copied().collect(),
        tree: Tree {
            map: NodeMap::of_slots(nodes),
            window,
        },
        layout_roots: to_slotmap(
            forest.layout_table(),
            |_, &root| LayoutRoot(root),
            LayoutRoot(NodeId::default()),
        ),
    }
}
