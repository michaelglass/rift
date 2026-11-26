//! The core's floating system and upstream's beside it (layout trees, floating's steps 2-3):
//! since the flip the core decides and upstream's system is its shadow, stepped on every
//! operation; each answer is held to the core's on every operation (queries, the pass's shape,
//! the group bars and rift-cli's views too), the whole systems (the forest's view and the frames
//! kept) on every operation in tests and debug builds and every [`VIEW_EVERY`]th otherwise. A
//! difference is counted ([`floating_divergences`]), fails a test, or in a running Rift is warned
//! (`floating-divergence:`) and the shadow re-seeded from the core.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};

use objc2_core_foundation::CGRect;
use rift_core::layout::group_forest::{Body, FloatingSystem, Geometry, Node};

use super::floating::Upstream;
use super::group_stored::{core_node, forest_of};
use super::group_system::CoreForest;
use super::groups::GroupLayouts;
use crate::actor::app::WindowId;
use crate::common::collections::HashMap;
use crate::layout_engine::LayoutId;
use crate::layout_engine::core_geometry::core_rect;
use crate::model::tree::{NodeId, WindowIndex};

/// The core's floating system over rift's ids.
pub(super) type CoreFloating = FloatingSystem<LayoutId, NodeId, WindowId>;

/// How often a release build compares the whole system (the twins' sampling).
const VIEW_EVERY: u64 = 32;

/// Whether every operation compares the whole system: in tests and debug builds (and so the
/// drawn fuzzer).
const VIEW_ALWAYS: bool = cfg!(any(test, debug_assertions));

static DIVERGENCES: AtomicU64 = AtomicU64::new(0);

/// Differences between the core's floating system and upstream's, counted process-wide.
pub(crate) fn floating_divergences() -> u64 { DIVERGENCES.load(Ordering::Relaxed) }

/// Counts a difference, then fails the test, or in a running Rift warns it.
pub(super) fn report(what: &str, difference: &str) {
    let divergences = DIVERGENCES.fetch_add(1, Ordering::Relaxed) + 1;
    #[cfg(test)]
    panic!("floating-divergence ({divergences} so far) after {what}: {difference}");
    #[cfg(not(test))]
    tracing::warn!(
        what,
        divergences,
        difference,
        "floating-divergence: upstream's floating system differs from the core's; re-seeded"
    );
}

/// A query's answer: the core's, upstream's held to it (reported when it differs; a query
/// changes nothing, so the next operation compares the whole systems).
pub(super) fn held<T: PartialEq + Debug>(what: &str, core: T, upstream: T) -> T {
    if upstream != core {
        report(what, &format!("core {core:?}, upstream {upstream:?}"));
    }
    core
}

/// Each leaf's stamp in upstream's index order (a window's leaves in their order).
fn stamps(index: &WindowIndex) -> HashMap<NodeId, u64> {
    let mut stamps = HashMap::default();
    let entries = index.window_nodes.values().flat_map(|nodes| nodes.0.iter());
    for (stamp, info) in (0u64..).zip(entries) {
        stamps.insert(info.node, stamp);
    }
    stamps
}

/// Upstream's pair of frames as the core holds it (both none is no entry).
fn core_geometry(
    geometry: impl IntoIterator<Item = (WindowId, Option<CGRect>, Option<CGRect>)>,
) -> BTreeMap<WindowId, Geometry> {
    geometry
        .into_iter()
        .filter_map(|(window, frame, restore)| {
            Geometry::from_pair(frame.map(core_rect), restore.map(core_rect))
                .map(|geometry| (window, geometry))
        })
        .collect()
}

/// The core's system seeded from upstream's: the forest as upstream's slotmaps hold it, each
/// leaf stamped in the index's order, and the frames.
pub(super) fn seed(
    groups: &GroupLayouts,
    geometry: impl IntoIterator<Item = (WindowId, Option<CGRect>, Option<CGRect>)>,
) -> CoreFloating {
    let forest = forest_of(
        &groups.tree,
        &groups.layout_roots,
        &stamps(&groups.tree.window),
        groups.inactive.iter().copied(),
    );
    CoreFloating::from_parts(forest, core_geometry(geometry))
}

/// A system as the twin compares it: layouts and nodes by id (a leaf's stamp left out: only its
/// order among the window's leaves is behaviour, which the index holds), the windows left out,
/// the index, and the frames kept. Vacant slots and free lists are not compared: a difference
/// there shows as the next id allocated.
type View = (
    Vec<(LayoutId, NodeId)>,
    Vec<(NodeId, Node<NodeId, WindowId>)>,
    BTreeSet<WindowId>,
    Vec<(WindowId, LayoutId, NodeId)>,
    BTreeMap<WindowId, Geometry>,
);

fn unstamped(mut node: Node<NodeId, WindowId>) -> Node<NodeId, WindowId> {
    if let Body::Window { stamp, .. } = &mut node.body {
        *stamp = 0;
    }
    node
}

fn core_view(core: &CoreFloating) -> View {
    let forest: &CoreForest = core.forest();
    (
        forest.layouts().collect(),
        forest
            .nodes()
            .iter()
            .map(|(key, node)| (key, unstamped(node.clone())))
            .collect(),
        forest.left_out().clone(),
        forest.index(),
        core.geometry().clone(),
    )
}

fn upstream_view(
    groups: &GroupLayouts,
    geometry: impl IntoIterator<Item = (WindowId, Option<CGRect>, Option<CGRect>)>,
) -> View {
    let stamps = HashMap::default();
    (
        groups.layout_roots.iter().map(|(layout, root)| (layout, root.0)).collect(),
        groups
            .tree
            .map
            .slots()
            .iter()
            .map(|(key, node)| (key, unstamped(core_node(&groups.tree.map, &stamps, key, node))))
            .collect(),
        groups.inactive.iter().copied().collect(),
        groups
            .tree
            .window
            .window_nodes
            .iter()
            .flat_map(|(&window, nodes)| {
                nodes.0.iter().map(move |info| (window, info.layout, info.node))
            })
            .collect(),
        core_geometry(geometry),
    )
}

/// Where the core's system differs from upstream's, if it does.
fn view_difference(
    core: &CoreFloating,
    groups: &GroupLayouts,
    geometry: impl IntoIterator<Item = (WindowId, Option<CGRect>, Option<CGRect>)>,
) -> Option<String> {
    let (ours, theirs) = (core_view(core), upstream_view(groups, geometry));
    let parts = [
        (
            "layouts",
            format!("{:?}", theirs.0),
            format!("{:?}", ours.0),
            ours.0 == theirs.0,
        ),
        (
            "nodes",
            format!("{:?}", theirs.1),
            format!("{:?}", ours.1),
            ours.1 == theirs.1,
        ),
        (
            "left out",
            format!("{:?}", theirs.2),
            format!("{:?}", ours.2),
            ours.2 == theirs.2,
        ),
        (
            "index",
            format!("{:?}", theirs.3),
            format!("{:?}", ours.3),
            ours.3 == theirs.3,
        ),
        (
            "frames",
            format!("{:?}", theirs.4),
            format!("{:?}", ours.4),
            ours.4 == theirs.4,
        ),
    ];
    parts
        .into_iter()
        .find(|(.., same)| !same)
        .map(|(what, upstream, core, _)| format!("{what}: upstream {upstream}, core {core}"))
}

/// Upstream's system beside the core's, and how many operations it has taken.
#[derive(Debug)]
pub(super) struct Shadow {
    upstream: Box<Upstream>,
    steps: u64,
}

impl Shadow {
    pub(super) fn of(upstream: Upstream) -> Self {
        Self {
            upstream: Box::new(upstream),
            steps: 0,
        }
    }

    pub(super) fn upstream(&self) -> &Upstream { &self.upstream }

    pub(super) fn upstream_mut(&mut self) -> &mut Upstream { &mut self.upstream }

    /// After an operation: whether the whole systems, compared when due, differ (reported).
    pub(super) fn differs(&mut self, what: &str, core: &CoreFloating) -> bool {
        self.steps += 1;
        if !(VIEW_ALWAYS || self.steps.is_multiple_of(VIEW_EVERY)) {
            return false;
        }
        match view_difference(core, &self.upstream.groups, self.upstream.frame_pairs()) {
            Some(difference) => {
                report(what, &difference);
                true
            }
            None => false,
        }
    }
}
