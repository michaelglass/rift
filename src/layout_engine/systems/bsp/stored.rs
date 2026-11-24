//! The BSP system's saved form (upstream's, byte for byte) and its load: the current form and
//! the older topology-and-components one, parsed into the core's tree's tables; the states only
//! a saved file holds are normalized here, at the boundary.

use serde::{Deserialize, Serialize};
use slotmap::{KeyData, SlotMap};

use crate::actor::app::WindowId;
use crate::common::collections::{HashMap, HashSet};
use crate::layout_engine::{Direction, LayoutId, Orientation};
use crate::model::selection::*;
use crate::model::tree::NodeId;

/// A node's kind as rift saves it (upstream's form).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub(super) enum NodeKind {
    Split {
        orientation: Orientation,
        ratio: f32,
        children: [NodeId; 2],
        #[serde(skip)]
        selected: Option<bool>,
    },
    Leaf {
        members: LeafMembers,
        fullscreen: bool,
        fullscreen_within_gaps: bool,
        preselected: Option<Direction>,
    },
}

/// A leaf's windows as rift saves them (upstream's form): a stack's windows in order, with the
/// index of the one on top.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) enum LeafMembers {
    #[default]
    Empty,
    Single(WindowId),
    Stack {
        windows: Vec<WindowId>,
        active: usize,
    },
}

/// A layout as rift saves it: its root.
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub(super) struct LayoutState {
    pub(super) root: NodeId,
}

#[derive(Clone, Deserialize, Debug)]
#[serde(from = "StoredNode")]
pub(super) struct Node {
    pub(super) parent: Option<NodeId>,
    pub(super) kind: NodeKind,
}

#[derive(Deserialize)]
#[serde(rename = "Node")]
struct StoredNode {
    parent: Option<NodeId>,
    selected: Option<bool>,
    kind: NodeKind,
}

impl From<StoredNode> for Node {
    fn from(mut stored: StoredNode) -> Self {
        if let NodeKind::Split { selected, .. } = &mut stored.kind {
            *selected = stored.selected;
        }
        Self {
            parent: stored.parent,
            kind: stored.kind,
        }
    }
}

impl Serialize for Node {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let selected = match self.kind {
            NodeKind::Split { selected, .. } => selected,
            NodeKind::Leaf { .. } => None,
        };
        let mut state = serializer.serialize_struct("Node", 3)?;
        state.serialize_field("parent", &self.parent)?;
        state.serialize_field("selected", &selected)?;
        state.serialize_field("kind", &self.kind)?;
        state.end()
    }
}

/// A saved BSP system in any form rift has written (the current one, and the older
/// topology-and-components form).
#[derive(Deserialize)]
pub(super) struct StoredBspSystem {
    layouts: SlotMap<LayoutId, LayoutState>,
    #[serde(default)]
    nodes: SlotMap<NodeId, Node>,
    #[serde(default, deserialize_with = "super::super::present")]
    tree: Option<LegacyTree>,
    #[serde(default)]
    kind: slotmap::SecondaryMap<NodeId, LegacyKind>,
    #[serde(default)]
    stacks: HashMap<NodeId, Vec<WindowId>>,
}

#[derive(Deserialize)]
struct LegacyTree {
    map: LegacyTopology,
    data: LegacyComponents,
}

#[derive(Deserialize)]
struct LegacyTopology {
    map: Vec<super::super::SavedSlot<LegacyNode>>,
}

#[derive(Deserialize)]
struct LegacyNode {
    parent: Option<NodeId>,
    first_child: Option<NodeId>,
    last_child: Option<NodeId>,
}

#[derive(Deserialize)]
struct LegacyComponents {
    selection: Selection,
    #[serde(default)]
    kind: slotmap::SecondaryMap<NodeId, LegacyKind>,
}

#[derive(Deserialize)]
enum LegacyKind {
    Split {
        orientation: Orientation,
        ratio: f32,
    },
    Leaf {
        window: Option<WindowId>,
        #[serde(default)]
        stack: Option<Vec<WindowId>>,
        fullscreen: bool,
        fullscreen_within_gaps: bool,
        preselected: Option<Direction>,
    },
}

/// A saved split's share that no operation makes (not a finite number) as a fresh split's.
/// Upstream's load keeps it (an upstream candidate); finite shares stand as saved.
fn normalize_ratio(layout: LayoutId, node: NodeId, ratio: &mut f32) {
    if !ratio.is_finite() {
        tracing::warn!(?layout, ?node, saved = %ratio, "bsp-load: normalized a split's share to 0.5");
        *ratio = 0.5;
    }
}

/// A saved leaf's fullscreen flags as an operation leaves them: never both set (fullscreen
/// wins, as the projection reads them), none on an empty leaf (cleared as a leaf empties).
/// Upstream's load keeps both states (an upstream candidate).
fn normalize_fullscreen(
    layout: LayoutId,
    node: NodeId,
    members: &LeafMembers,
    fullscreen: &mut bool,
    within_gaps: &mut bool,
) {
    if matches!(members, LeafMembers::Empty) && (*fullscreen || *within_gaps) {
        tracing::warn!(
            ?layout,
            ?node,
            fullscreen = *fullscreen,
            within_gaps = *within_gaps,
            "bsp-load: normalized an empty leaf's fullscreen flags away"
        );
        (*fullscreen, *within_gaps) = (false, false);
    } else if *fullscreen && *within_gaps {
        tracing::warn!(
            ?layout,
            ?node,
            "bsp-load: normalized a leaf with both fullscreen flags to fullscreen"
        );
        *within_gaps = false;
    }
}

/// A saved system's tables, parsed: older forms read into the current one, abandoned nodes
/// dropped (keeping the live ids and versions), and on each node a layout reaches the states only
/// a saved file holds normalized (one `bsp-load:` warn each). Refused when a layout's nodes do not
/// form one tree.
pub(super) struct Loaded {
    pub(super) layouts: SlotMap<LayoutId, LayoutState>,
    pub(super) nodes: SlotMap<NodeId, Node>,
}

impl TryFrom<StoredBspSystem> for Loaded {
    type Error = String;

    fn try_from(stored: StoredBspSystem) -> Result<Self, String> {
        let mut nodes = if let Some(tree) = stored.tree {
            let mut kinds = if tree.data.kind.is_empty() {
                stored.kind
            } else {
                tree.data.kind
            };
            let mut stacks = stored.stacks;
            let mut slots = Vec::with_capacity(tree.map.map.len());
            for (index, slot) in tree.map.map.into_iter().enumerate() {
                let value = if let Some(node) = slot.value {
                    let id = NodeId::from(KeyData::from_ffi(
                        (u64::from(slot.version) << 32) | index as u64,
                    ));
                    let kind = match kinds.remove(id).ok_or("missing saved BSP node kind")? {
                        LegacyKind::Split { orientation, ratio } => {
                            let first = node.first_child.ok_or("missing first BSP child")?;
                            let second = node
                                .last_child
                                .filter(|&last| last != first)
                                .ok_or("missing second BSP child")?;
                            NodeKind::Split {
                                orientation,
                                ratio,
                                children: [first, second],
                                selected: None,
                            }
                        }
                        LegacyKind::Leaf {
                            window,
                            stack,
                            fullscreen,
                            fullscreen_within_gaps,
                            preselected,
                        } => NodeKind::Leaf {
                            members: if let Some(windows) = stacks.remove(&id).or(stack) {
                                let active = windows
                                    .iter()
                                    .position(|wid| Some(*wid) == window)
                                    .unwrap_or_else(|| windows.len().saturating_sub(1));
                                LeafMembers::Stack { windows, active }
                            } else {
                                window.map_or(LeafMembers::Empty, LeafMembers::Single)
                            },
                            fullscreen,
                            fullscreen_within_gaps,
                            preselected,
                        },
                    };
                    let selected = match (&kind, tree.data.selection.selected_child(id)) {
                        (NodeKind::Split { children, .. }, Some(child)) => {
                            if !children.contains(&child) {
                                return Err("saved BSP selection is not a split child".into());
                            }
                            Some(child == children[1])
                        }
                        _ => None,
                    };
                    Some(Node::from(StoredNode {
                        parent: node.parent,
                        selected,
                        kind,
                    }))
                } else {
                    None
                };
                slots.push(super::super::SavedSlot { value, version: slot.version });
            }
            let wire = ron::to_string(&slots).map_err(|error| error.to_string())?;
            ron::from_str(&wire).map_err(|error| error.to_string())?
        } else {
            stored.nodes
        };
        let mut pending: Vec<_> = stored
            .layouts
            .iter()
            .map(|(layout, state)| (layout, state.root, None))
            .collect();
        let mut visited = HashSet::default();
        while let Some((layout, id, parent)) = pending.pop() {
            let node = nodes.get_mut(id).ok_or("missing saved BSP node")?;
            if node.parent != parent || !visited.insert(id) {
                return Err("saved BSP nodes must belong to exactly one branch".into());
            }
            match &mut node.kind {
                NodeKind::Split { children, ratio, .. } => {
                    pending.extend(children.iter().map(|&child| (layout, child, Some(id))));
                    normalize_ratio(layout, id, ratio);
                }
                NodeKind::Leaf {
                    members,
                    fullscreen,
                    fullscreen_within_gaps,
                    ..
                } => {
                    if let LeafMembers::Stack { windows, active } = members {
                        if windows.is_empty() {
                            *members = LeafMembers::Empty;
                        } else if *active >= windows.len() {
                            *active = 0;
                        }
                    }
                    normalize_fullscreen(layout, id, members, fullscreen, fullscreen_within_gaps);
                }
            }
        }
        // Older arenas can contain abandoned nodes. Keep the live IDs and generations.
        nodes.retain(|id, _| visited.contains(&id));
        Ok(Self { layouts: stored.layouts, nodes })
    }
}
