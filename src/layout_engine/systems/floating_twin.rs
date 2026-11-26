//! The core's floating system beside upstream's (layout trees, floating's step 2): upstream's
//! `FloatingLayoutSystem` decides; the core's [`FloatingSystem`] is stepped on every operation
//! and held to it. Each answer on every operation (queries, the pass's shape, the group bars and
//! rift-cli's views too); the whole system (the forest's view and the frames kept) on every
//! operation in tests and debug builds and every [`VIEW_EVERY`]th otherwise. A difference is
//! counted ([`floating_divergences`]), fails a test, or in a running Rift is warned
//! (`floating-divergence:`) and the core re-seeded from upstream's system.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};

use objc2_core_foundation::CGRect;
use rift_core::layout::group_forest::{Body, FloatingSystem, Geometry, Node};

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
        "floating-divergence: the core's floating system differs from upstream's; re-seeded"
    );
}

/// An answer held to upstream's: reported when it differs.
pub(super) fn held<T: PartialEq + Debug>(what: &str, upstream: T, core: T) -> T {
    if upstream != core {
        report(what, &format!("upstream {upstream:?}, core {core:?}"));
    }
    upstream
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

/// The core's system and how many operations it has taken.
#[derive(Debug, Default)]
pub(super) struct Twin {
    core: Box<CoreFloating>,
    steps: u64,
}

impl Twin {
    pub(super) fn seeded(core: CoreFloating) -> Self { Self { core: Box::new(core), steps: 0 } }

    pub(super) fn core(&self) -> &CoreFloating { &self.core }

    pub(super) fn core_mut(&mut self) -> &mut CoreFloating { &mut self.core }

    /// After an operation: the whole system compared when due, and on a difference reported and
    /// re-seeded from upstream's.
    pub(super) fn settle<G>(
        &mut self,
        what: &str,
        groups: &GroupLayouts,
        geometry: impl Fn() -> G,
    ) where
        G: IntoIterator<Item = (WindowId, Option<CGRect>, Option<CGRect>)>,
    {
        self.steps += 1;
        if !(VIEW_ALWAYS || self.steps.is_multiple_of(VIEW_EVERY)) {
            return;
        }
        if let Some(difference) = view_difference(&self.core, groups, geometry()) {
            report(what, &difference);
            *self.core = seed(groups, geometry());
        }
    }
}
