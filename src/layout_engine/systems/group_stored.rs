//! The group forest as upstream saves it (`GroupLayouts`, the traditional and master-stack
//! systems' saved form): written from the core byte for byte, and loaded into the core (the
//! current form and master-stack's legacy forms, parsed through the old tree; the states only a
//! saved file holds normalized here, at the boundary, one `{system}-load:` warn each). Also
//! upstream's `GroupLayouts` built from the core (the shadow re-seeded after a divergence).

use std::collections::BTreeMap;

use rift_core::layout::group_forest::{Body, Container, Fill, Members, Memory, Node, Place, Side};
use serde::{Deserialize, Serialize};
use slotmap::{SecondaryMap, SlotMap};

use super::group_twin::{CoreForest, core_kind, layout_kind};
use super::groups::{GroupLayouts, LayoutRoot, deserialize_tile_tree};
use crate::actor::app::WindowId;
use crate::common::collections::{HashMap, HashSet};
use crate::common::config::{MasterStackSettings, MasterStackSide};
use crate::layout_engine::{LayoutId, LayoutKind, Orientation};
use crate::model::selection::SelectionInfo;
use crate::model::slot_table::{SavedSlot, SlotTable, of_slotmap, saved, to_slotmap};
use crate::model::tree::{
    self, Held, NodeId, NodeMap, Placement, Tree, WindowIndex, WindowNodeInfo,
};

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

/// The forest as rift saves it (upstream's `GroupLayouts`: the tree, in any form the old tree
/// reads, then the layout roots), parsed.
#[derive(Deserialize)]
pub(super) struct StoredForest {
    #[serde(deserialize_with = "deserialize_tile_tree")]
    tree: Tree,
    layout_roots: SlotMap<LayoutId, LayoutRoot>,
}

/// A saved forest, parsed and normalized: the old tree the core is built from, and each
/// window leaf's entry stamp (the index's order, normalized to the tree).
pub(super) struct Loaded {
    tree: Tree,
    layout_roots: SlotMap<LayoutId, LayoutRoot>,
    stamps: HashMap<NodeId, u64>,
}

impl StoredForest {
    /// The states only a saved file holds, normalized with one `{system}-load:` warn each (the
    /// W2b rule; upstream keeps them, an upstream candidate). Three are only warned, as the
    /// core's types cannot hold them: both fullscreen flags on a node (a `Fill` is one of three,
    /// fullscreen first, as the projection reads them), a window's layout kind (a window has
    /// none), a root's size other than 0 (a root has no size). The fourth is the window index
    /// disagreeing with the tree: the leaves are stamped in the saved index's order, entries
    /// naming no leaf of theirs in that layout dropped, a leaf listed again after its first
    /// entry dropped, and the leaves it misses stamped after, in layout and tree order (one
    /// warn). The order kept is behaviour: it orders a window's leaves by when they entered
    /// their layouts.
    pub(super) fn normalized(self, system: &str) -> Loaded {
        let StoredForest { tree, layout_roots } = self;
        let map = &tree.map;
        for (node, entry) in map.slots() {
            let info = entry.layout;
            if info.is_fullscreen && info.is_fullscreen_within_gaps {
                tracing::warn!(?node, "{system}-load: both fullscreen flags; loaded fullscreen");
            }
            if map.window(node).is_some() && info.kind != LayoutKind::default() {
                tracing::warn!(?node, kind = ?info.kind, "{system}-load: a window's layout kind; loaded as the default");
            }
            if node.parent(map).is_none() && info.size.to_bits() != 0.0_f32.to_bits() {
                tracing::warn!(
                    ?node,
                    size = info.size,
                    "{system}-load: a root's size; loaded as 0"
                );
            }
        }
        let leaves: Vec<(WindowId, LayoutId, NodeId)> = layout_roots
            .iter()
            .flat_map(|(layout, root)| {
                root.0
                    .traverse_preorder(map)
                    .filter_map(move |node| map.window(node).map(|window| (window, layout, node)))
            })
            .collect();
        let is_leaf: HashSet<(WindowId, LayoutId, NodeId)> = leaves.iter().copied().collect();
        let mut stamps = HashMap::default();
        let mut dropped = 0;
        for (&window, infos) in &tree.window.window_nodes {
            for info in &infos.0 {
                let next = stamps.len() as u64;
                if is_leaf.contains(&(window, info.layout, info.node))
                    && !stamps.contains_key(&info.node)
                {
                    stamps.insert(info.node, next);
                } else {
                    dropped += 1;
                }
            }
        }
        let mut added = 0;
        for (_, _, node) in leaves {
            let next = stamps.len() as u64;
            if let std::collections::hash_map::Entry::Vacant(entry) = stamps.entry(node) {
                entry.insert(next);
                added += 1;
            }
        }
        if dropped + added > 0 {
            tracing::warn!(
                dropped,
                added,
                "{system}-load: the window index disagreed with the tree; normalized to it"
            );
        }
        Loaded { tree, layout_roots, stamps }
    }
}

/// A master-stack layout's regions as the previous form saved them beside the forest.
#[derive(Deserialize)]
struct Regions {
    master: NodeId,
    stack: NodeId,
}

/// A master-stack system as rift saves it (the forest as `inner`; the settings are not saved),
/// or as the previous forms did: the regions saved beside the forest, or not at all.
#[derive(Deserialize)]
pub(super) struct StoredMasterStack {
    inner: StoredForest,
    #[serde(default)]
    regions: SecondaryMap<LayoutId, Regions>,
}

impl StoredMasterStack {
    /// The forest with every layout's root a region pair, as `settings` (the load's: settings
    /// are not saved) place one. A root already a pair is kept; else the regions saved beside
    /// the forest; else a root whose two children are containers holds them as (master, stack);
    /// else the root's children are moved into two new region containers, the first windows up
    /// to the master count into the master region, the ratio applied, and the selected window
    /// selected again. Upstream's load, step for step.
    pub(super) fn migrated(self, settings: &MasterStackSettings) -> StoredForest {
        let StoredMasterStack { mut inner, regions } = self;
        let layouts: Vec<(LayoutId, NodeId)> =
            inner.layout_roots.iter().map(|(layout, root)| (layout, root.0)).collect();
        for (layout, root) in layouts {
            let tree = &mut inner.tree;
            if tree.map.region_pair(root).is_some() {
                continue;
            }
            let pair = if let Some(&Regions { master, stack }) = regions.get(layout) {
                (master, stack)
            } else {
                let children: Vec<_> = root.children(&tree.map).collect();
                if let [master, stack] = children[..]
                    && children.iter().all(|&node| tree.map.window(node).is_none())
                {
                    (master, stack)
                } else {
                    let selected = focused_window(&tree.map, root);
                    let (master, stack) = region_containers(tree, root, settings);
                    let mut count = 0;
                    for child in children {
                        let n = window_count(&tree.map, child);
                        let target = if count < settings.master_count {
                            master
                        } else {
                            stack
                        };
                        tree.move_node(child, Placement::Back(target));
                        count += n;
                    }
                    let ratio = settings.master_ratio.clamp(0.05, 0.95) as f32;
                    let total = 2.0_f32;
                    let master_size = (ratio * total).max(0.05);
                    tree.map[master].layout.size = master_size;
                    tree.map[stack].layout.size = (total - master_size).max(0.05);
                    if let Some(window) = selected
                        && let Some(node) = tree.window.node_for(layout, window)
                    {
                        tree.map.select(node);
                    }
                    (master, stack)
                }
            };
            inner.tree.map.set_region_pair(root, pair.0, pair.1);
        }
        inner
    }
}

/// The window a layout's selection leads to (none taking part is left out at a load).
fn focused_window(map: &NodeMap, root: NodeId) -> Option<WindowId> {
    let selection = map.current_selection(root);
    map.focus_target(selection, &|_| true)
        .or_else(|| (selection != root).then(|| map.focus_target(root, &|_| true)).flatten())
        .map(|(_, window)| window)
}

fn window_count(map: &NodeMap, node: NodeId) -> usize {
    if map.window(node).is_some() {
        return 1;
    }
    node.children(map).map(|child| window_count(map, child)).sum()
}

/// Two new region containers under `root`, (master, stack), placed and arranged as `settings`
/// say, the root running across them.
fn region_containers(
    tree: &mut Tree,
    root: NodeId,
    settings: &MasterStackSettings,
) -> (NodeId, NodeId) {
    let (root_orientation, master_first) = match settings.master_side {
        MasterStackSide::Left => (Orientation::Horizontal, true),
        MasterStackSide::Right => (Orientation::Horizontal, false),
        MasterStackSide::Top => (Orientation::Vertical, true),
        MasterStackSide::Bottom => (Orientation::Vertical, false),
    };
    let across = match root_orientation {
        Orientation::Horizontal => Orientation::Vertical,
        Orientation::Vertical => Orientation::Horizontal,
    };
    tree.map.set_kind(root, LayoutKind::from(root_orientation));
    let first = tree.insert(Placement::Back(root));
    let second = tree.insert(Placement::Back(root));
    let (master, stack) = if master_first {
        (first, second)
    } else {
        (second, first)
    };
    tree.map.set_kind(
        master,
        LayoutKind::from(settings.master_arrangement.unwrap_or(across)),
    );
    tree.map.set_kind(
        stack,
        LayoutKind::from(settings.stack_arrangement.unwrap_or(across)),
    );
    (master, stack)
}

fn core_fill(fullscreen: bool, within_gaps: bool) -> Fill {
    match (fullscreen, within_gaps) {
        (true, _) => Fill::Screen,
        (false, true) => Fill::WithinGaps,
        (false, false) => Fill::Tiled,
    }
}

fn core_memory(selection: Option<SelectionInfo>) -> Memory<NodeId> {
    match selection.map(SelectionInfo::parts) {
        None => Memory::Unset,
        Some((child, false)) => Memory::Toward(child),
        Some((child, true)) => Memory::Stops(child),
    }
}

/// Each indexed leaf's stamp, in the index's order: a window's leaves by when they entered.
pub(super) fn stamps(index: &WindowIndex) -> HashMap<NodeId, u64> {
    let mut stamps = HashMap::default();
    let entries = index.window_nodes.values().flat_map(|nodes| nodes.0.iter());
    for (stamp, info) in (0u64..).zip(entries) {
        stamps.insert(info.node, stamp);
    }
    stamps
}

/// An old tree's node as the core holds it (a leaf no layout reaches is not indexed: the load
/// leaves every other one indexed; it takes the stamp after every indexed one).
pub(super) fn core_node(
    map: &NodeMap,
    stamps: &HashMap<NodeId, u64>,
    key: NodeId,
    node: &tree::Node,
) -> Node<NodeId, WindowId> {
    let (parent, held) = map.held(key);
    let info = node.layout;
    let place = match parent {
        None => Place::Root,
        Some(parent) => Place::Child { parent, size: info.size },
    };
    let body = match held {
        Held::Window(window) => Body::Window {
            window,
            stamp: stamps.get(&key).copied().unwrap_or(stamps.len() as u64),
        },
        Held::Container { children, selection } => Body::Container(Container {
            kind: core_kind(info.kind),
            members: Members::Children(children),
            memory: core_memory(selection),
        }),
        Held::RegionPair {
            regions,
            primary_first,
            selection,
        } => Body::Container(Container {
            kind: core_kind(info.kind),
            members: Members::Regions {
                order: regions,
                primary: if primary_first {
                    Side::First
                } else {
                    Side::Second
                },
            },
            memory: core_memory(selection),
        }),
    };
    Node {
        place,
        fill: core_fill(info.is_fullscreen, info.is_fullscreen_within_gaps),
        body,
    }
}

/// An old tree as the core holds it: both slot tables as the slotmaps hold them (free lists
/// included), each leaf stamped as `stamps` say, `left_out` the windows left out.
pub(super) fn forest_of(
    tree: &Tree,
    layout_roots: &SlotMap<LayoutId, LayoutRoot>,
    stamps: &HashMap<NodeId, u64>,
    left_out: impl IntoIterator<Item = WindowId>,
) -> CoreForest {
    let layouts = of_slotmap(layout_roots, LayoutRoot(NodeId::default()), |root| root.0);
    let nodes: SlotTable<NodeId, tree::Node> =
        of_slotmap(tree.map.slots(), tree::Node::default(), Clone::clone);
    let nodes = nodes
        .map(|key, node| {
            Ok::<_, std::convert::Infallible>(core_node(&tree.map, stamps, key, &node))
        })
        .unwrap_or_else(|never| match never {});
    CoreForest::from_leaves(layouts, nodes, left_out.into_iter().collect())
}

impl Loaded {
    /// The loaded forest as the core holds it (no window is left out at a load).
    pub(super) fn forest(&self) -> CoreForest {
        forest_of(&self.tree, &self.layout_roots, &self.stamps, [])
    }
}
