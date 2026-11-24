//! The BSP layout system with the core's tree beside it (layout trees, slice 2). Upstream's
//! system ([`Upstream`]) decides; the core's [`Tree`] is stepped on every operation, in every
//! build, and held to it: each answer is compared on every operation, the whole tree (the view)
//! on every operation in tests and `roc-core` builds and every [`VIEW_EVERY`]th in a plain
//! release build, and each pass's shape (what the core's geometry is handed) on every pass. A
//! difference is counted, fails a test, or in a running Rift is warned and the core re-seeded
//! from upstream's system.

use std::collections::BTreeSet;
use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};

use objc2_core_foundation::CGRect;
use rift_core::layout::bsp_tree::{
    self as core, Content, InsertionPoint, Kind, Layout, Leaf, Members, Memory, Parent,
    Participation, Preselect, Query, Resize, Side, Split, Tabs, Tree,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use slotmap::SlotMap;

use super::{LayoutState, LeafMembers, Node, NodeKind, Upstream};
use crate::actor::app::{WindowId, pid_t};
use crate::common::collections::HashMap;
use crate::common::config::WindowInsertionPoint;
use crate::layout_engine::systems::{
    EditResult, Focus, FocusResult, LayoutEdit, LayoutSystem, WindowChange,
    WindowLayoutConstraints, WindowQuery,
};
use crate::layout_engine::{Direction, LayoutId, Orientation};
use crate::model::slot_table::{SavedSlot, SlotKey, SlotTable};
use crate::model::tree::NodeId;

/// The core's tree over rift's ids.
pub(super) type CoreTree = Tree<LayoutId, NodeId, WindowId>;

/// How often a plain release build compares the whole tree (the twins' sampling).
const VIEW_EVERY: u64 = 32;

/// Whether every operation compares the whole tree.
const VIEW_ALWAYS: bool = cfg!(any(test, feature = "roc-core"));

static BSP_TREE_DIVERGENCES: AtomicU64 = AtomicU64::new(0);

/// Differences between the core's BSP tree and upstream's system, counted process-wide.
pub(crate) fn bsp_tree_divergences() -> u64 { BSP_TREE_DIVERGENCES.load(Ordering::Relaxed) }

/// The BSP layout system: upstream's decides, and the core's tree is stepped beside it.
#[derive(Clone, Debug, Deserialize)]
#[serde(from = "Upstream")]
pub struct BspLayoutSystem {
    pub(super) upstream: Upstream,
    /// Boxed: the system is one variant of `LayoutSystemKind`.
    twin: Box<TwinnedBsp>,
}

/// The core's tree and how many operations stepped it.
#[derive(Clone, Debug)]
struct TwinnedBsp {
    core: CoreTree,
    steps: u64,
}

impl Serialize for BspLayoutSystem {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.upstream.serialize(serializer)
    }
}

impl From<Upstream> for BspLayoutSystem {
    fn from(upstream: Upstream) -> Self {
        let twin = Box::new(TwinnedBsp {
            core: seed(&upstream),
            steps: 0,
        });
        Self { upstream, twin }
    }
}

impl Default for BspLayoutSystem {
    fn default() -> Self { Upstream::default().into() }
}

impl BspLayoutSystem {
    pub fn new(window_insertion_point: WindowInsertionPoint) -> Self {
        Upstream::new(window_insertion_point).into()
    }

    pub fn set_window_insertion_point(&mut self, value: WindowInsertionPoint) {
        self.upstream.set_window_insertion_point(value);
        self.twin.core.set_window_insertion_point(insertion_point(value));
        self.stepped("set_window_insertion_point", &(), &());
    }

    pub fn set_single_window_aspect_ratio(&mut self, ratio: Option<f64>) {
        self.upstream.set_single_window_aspect_ratio(ratio);
        self.twin.core.set_single_window_aspect_ratio(ratio);
        self.stepped("set_single_window_aspect_ratio", &(), &());
    }

    pub(in crate::layout_engine) fn can_move_window(
        &self,
        layout: LayoutId,
        window: WindowId,
        direction: Direction,
    ) -> bool {
        self.answered(
            "can_move_window",
            self.upstream.can_move_window(layout, window, direction),
            self.twin.core.can_move_window(layout, window, core_direction(direction)),
        )
    }

    pub(crate) fn draw_tree(&self, layout: LayoutId) -> String { self.upstream.draw_tree(layout) }

    pub(crate) fn container_tree(&self, layout: LayoutId) -> rift_protocol::ContainerTreeNode {
        self.upstream.container_tree(layout)
    }

    pub(crate) fn window_slot(&self, layout: LayoutId, window: WindowId) -> Option<Vec<usize>> {
        self.answered(
            "window_slot",
            self.upstream.window_slot(layout, window),
            self.twin.core.window_slot(layout, window),
        )
    }

    /// After an operation upstream's system and the core both took: their answers compared,
    /// and the trees when due; a difference is reported and the core re-seeded.
    fn stepped<A: PartialEq + Debug>(&mut self, what: &str, upstream: &A, core: &A) {
        self.twin.steps += 1;
        let answers_differ = upstream != core;
        if answers_differ {
            report(what, &format!("answers: upstream {upstream:?}, core {core:?}"));
        }
        let view_due = VIEW_ALWAYS || self.twin.steps.is_multiple_of(VIEW_EVERY);
        let view =
            (answers_differ || view_due).then(|| view_difference(&self.twin.core, &self.upstream));
        if let Some(Some(difference)) = &view {
            report(what, difference);
        }
        if answers_differ || matches!(view, Some(Some(_))) {
            self.twin.core = seed(&self.upstream);
        }
    }

    /// A query both answered: a difference is reported (the next step's view re-seeds the
    /// core).
    fn answered<A: PartialEq + Debug>(&self, what: &str, upstream: A, core: A) -> A {
        if upstream != core {
            report(what, &format!("answers: upstream {upstream:?}, core {core:?}"));
        }
        upstream
    }

    /// After an operation the core has no port of yet: the core takes upstream's tree.
    fn unported(&mut self) { self.twin.core = seed(&self.upstream); }
}

/// Counts a difference, then fails the test, or in a running Rift warns it.
fn report(what: &str, difference: &str) {
    let divergences = BSP_TREE_DIVERGENCES.fetch_add(1, Ordering::Relaxed) + 1;
    #[cfg(test)]
    panic!("bsp-tree-divergence ({divergences} so far) after {what}: {difference}");
    #[cfg(not(test))]
    tracing::warn!(
        what,
        divergences,
        difference,
        "bsp-tree-divergence: the core's BSP tree differs from upstream's; re-seeded"
    );
}

impl LayoutSystem for BspLayoutSystem {
    fn has_windows_for_app(&self, layout: LayoutId, pid: pid_t) -> bool {
        self.answered(
            "has_windows_for_app",
            self.upstream.has_windows_for_app(layout, pid),
            self.twin.core.has_windows_for_app(layout, |window| window.pid == pid),
        )
    }

    fn window_participates(&self, window: WindowId) -> bool {
        self.answered(
            "window_participates",
            self.upstream.window_participates(window),
            self.twin.core.window_participates(window),
        )
    }

    fn set_window_participation(&mut self, window: WindowId, participates: bool) -> bool {
        let upstream = self.upstream.set_window_participation(window, participates);
        let participation = if participates {
            Participation::TakesPart
        } else {
            Participation::LeftOut
        };
        let core = self.twin.core.set_window_participation(window, participation);
        self.stepped("set_window_participation", &upstream, &core);
        upstream
    }

    fn create_layout(&mut self) -> LayoutId {
        let upstream = self.upstream.create_layout();
        let core = self.twin.core.create_layout();
        self.stepped("create_layout", &upstream, &core);
        upstream
    }

    fn contains_layout(&self, layout: LayoutId) -> bool {
        self.answered(
            "contains_layout",
            self.upstream.contains_layout(layout),
            self.twin.core.contains_layout(layout),
        )
    }

    fn clone_layout(&mut self, layout: LayoutId) -> LayoutId {
        let upstream = self.upstream.clone_layout(layout);
        let core = self.twin.core.clone_layout(layout);
        self.stepped("clone_layout", &upstream, &core);
        upstream
    }

    fn remove_layout(&mut self, layout: LayoutId) {
        self.upstream.remove_layout(layout);
        self.twin.core.remove_layout(layout);
        self.stepped("remove_layout", &(), &());
    }

    /// Upstream's frames (the core's geometry decides them, policy (a)); the core's tree's
    /// shape is held to upstream's system's on every pass.
    fn calculate_layout(
        &self,
        layout: LayoutId,
        screen: CGRect,
        stack_offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &crate::common::config::GapSettings,
        stack_line: crate::common::config::StackLineReserve,
        shadow: crate::layout_engine::Shadow,
    ) -> Vec<(WindowId, CGRect)> {
        use crate::layout_engine::core_geometry::core_constraints;
        use crate::layout_engine::{core_gaps, core_rect};
        let upstream = self
            .upstream
            .layouts
            .get(layout)
            .and_then(|state| self.upstream.shape(state.root, screen, constraints, gaps));
        let core = self.twin.core.shape(layout, core_rect(screen), core_gaps(gaps), |window| {
            constraints.get(&window).copied().map(core_constraints)
        });
        if upstream != core {
            report(
                "calculate_layout",
                &format!("shapes: upstream {upstream:?}, core {core:?}"),
            );
        }
        self.upstream.calculate_layout(
            layout,
            screen,
            stack_offset,
            constraints,
            gaps,
            stack_line,
            shadow,
        )
    }

    fn selected_window(&self, layout: LayoutId) -> Option<WindowId> {
        self.answered(
            "selected_window",
            self.upstream.selected_window(layout),
            self.twin.core.selected_window(layout),
        )
    }

    fn window_in_direction(&self, layout: LayoutId, direction: Direction) -> Option<WindowId> {
        self.answered(
            "window_in_direction",
            self.upstream.window_in_direction(layout, direction),
            self.twin.core.window_in_direction(layout, core_direction(direction)),
        )
    }

    fn contains_window(&self, layout: LayoutId, wid: WindowId) -> bool {
        self.answered(
            "contains_window",
            self.upstream.contains_window(layout, wid),
            self.twin.core.contains_window(layout, wid),
        )
    }

    fn has_any_fullscreen_node(&self, layout: LayoutId) -> bool {
        self.answered(
            "has_any_fullscreen_node",
            self.upstream.has_any_fullscreen_node(layout),
            self.twin.core.has_any_fullscreen_node(layout),
        )
    }

    fn windows(&self, layout: LayoutId, query: WindowQuery) -> Vec<WindowId> {
        let core_query = match query {
            WindowQuery::All => Query::All,
            WindowQuery::Visible => Query::Visible,
            WindowQuery::Selection => Query::Selection,
            WindowQuery::Stack(window) => Query::Stack(window),
        };
        self.answered(
            "windows",
            self.upstream.windows(layout, query),
            self.twin.core.windows(layout, core_query),
        )
    }

    fn focus(&mut self, layout: LayoutId, focus: Focus) -> FocusResult {
        let upstream = self.upstream.focus(layout, focus);
        let core_focus = match focus {
            Focus::Select(window) => core::Focus::Select(window),
            Focus::Ascend => core::Focus::Ascend,
            Focus::Descend => core::Focus::Descend,
            Focus::Direction(direction) => core::Focus::Direction(core_direction(direction)),
        };
        let core = match self.twin.core.focus(layout, core_focus) {
            core::Focused::Selection { handled } => FocusResult::selection(handled),
            core::Focused::Movement { window, raise } => FocusResult::movement((window, raise)),
        };
        self.stepped("focus", &focus_words(&upstream), &focus_words(&core));
        upstream
    }

    fn update(&mut self, change: WindowChange<'_>) {
        use crate::layout_engine::core_rect;
        let core = &mut self.twin.core;
        match change {
            WindowChange::Add(layout, window, _) => core.add_window(layout, window),
            WindowChange::Remove(window) | WindowChange::RemoveAndRebalance(window) => {
                core.remove_window(window)
            }
            WindowChange::Replace(from, to) => core.replace_window(from, to),
            WindowChange::CloseApp(pid) => core.close_app(|window| window.pid == pid),
            WindowChange::Resize {
                layout,
                window,
                old,
                new,
                screen,
                gaps,
            } => core.resize_window(layout, window, Resize {
                old: core_rect(old),
                new: core_rect(new),
                screen: core_rect(screen),
                tiling: core_rect(crate::layout_engine::utils::compute_tiling_area(screen, gaps)),
            }),
        }
        self.upstream.update(change);
        self.stepped("update", &(), &());
    }

    fn edit_layout(&mut self, layout: LayoutId, edit: LayoutEdit) -> EditResult {
        let upstream = self.upstream.edit_layout(layout, edit);
        let Some(edit) = core_edit(edit) else {
            self.unported();
            return upstream;
        };
        let core = match self.twin.core.edit(layout, edit) {
            core::Edited::Unsupported => EditResult::Unsupported,
            core::Edited::Unchanged => EditResult::Unchanged,
            core::Edited::Changed => EditResult::Changed,
            core::Edited::Raise(windows) => EditResult::Raise(windows),
        };
        self.stepped("edit_layout", &edit_words(&upstream), &edit_words(&core));
        upstream
    }
}

/// A focus's answer, comparable.
fn focus_words(result: &FocusResult) -> (bool, Option<WindowId>, Vec<WindowId>) {
    (result.handled, result.window, result.raise.clone())
}

/// An edit's answer, comparable.
fn edit_words(result: &EditResult) -> (&'static str, Vec<WindowId>) {
    match result {
        EditResult::Unsupported => ("unsupported", Vec::new()),
        EditResult::Unchanged => ("unchanged", Vec::new()),
        EditResult::Changed => ("changed", Vec::new()),
        EditResult::Raise(windows) => ("raise", windows.clone()),
    }
}

/// An edit as the core takes it; none for those the core has no port of yet.
fn core_edit(edit: LayoutEdit) -> Option<core::Edit<WindowId>> {
    use crate::layout_engine::{LayoutKind, ResizeOrientation, WindowDropAction};
    Some(match edit {
        LayoutEdit::Move { direction, .. } => core::Edit::Move(core_direction(direction)),
        LayoutEdit::Swap(a, b) => core::Edit::Swap(a, b),
        LayoutEdit::Drop { source, target, action } => core::Edit::Drop {
            source,
            target,
            action: match action {
                WindowDropAction::Swap => core::DropAction::Swap,
                WindowDropAction::Stack => core::DropAction::Stack,
                WindowDropAction::Insert(direction) => {
                    core::DropAction::Insert(core_direction(direction))
                }
                WindowDropAction::Move(direction) => {
                    core::DropAction::Move(core_direction(direction))
                }
                WindowDropAction::ConsumeOrExpel(direction) => {
                    core::DropAction::ConsumeOrExpel(core_direction(direction))
                }
            },
        },
        LayoutEdit::Split(kind) => core::Edit::Split(match kind {
            LayoutKind::Horizontal => core::SplitKind::Tiled(core::Orientation::Horizontal),
            LayoutKind::Vertical => core::SplitKind::Tiled(core::Orientation::Vertical),
            LayoutKind::HorizontalStack | LayoutKind::VerticalStack => core::SplitKind::Stacked,
        }),
        LayoutEdit::Fullscreen { within_gaps } => core::Edit::Fullscreen { within_gaps },
        LayoutEdit::Resize { amount, orientation } => core::Edit::Resize {
            amount,
            along: match orientation {
                ResizeOrientation::Horizontal => core::ResizeAlong::Horizontal,
                ResizeOrientation::Vertical => core::ResizeAlong::Vertical,
                ResizeOrientation::Smart => core::ResizeAlong::Smart,
            },
        },
        LayoutEdit::Rebalance => core::Edit::Rebalance,
        LayoutEdit::ToggleOrientation => core::Edit::ToggleOrientation,
        _ => return None,
    })
}

/// Upstream's system as the core holds it.
fn seed(upstream: &Upstream) -> CoreTree {
    let dummy_node = Node {
        parent: None,
        kind: NodeKind::Leaf {
            members: LeafMembers::Empty,
            fullscreen: false,
            fullscreen_within_gaps: false,
            preselected: None,
        },
    };
    let dummy_layout = LayoutState { root: NodeId::default() };
    CoreTree::from_parts(
        table(&upstream.layouts, dummy_layout, |state| Layout {
            root: state.root,
        }),
        table(&upstream.nodes, dummy_node, core_node),
        upstream.inactive.iter().copied().collect::<BTreeSet<_>>(),
        insertion_point(upstream.window_insertion_point),
        upstream.single_window_aspect_ratio,
    )
}

/// A live slotmap as the core's table: its slots, vacant ones and their versions included (read
/// from its saved form), and its free list (the order a copy hands its vacant slots out; a load
/// rebuilds it otherwise).
fn table<K: slotmap::Key + SlotKey, V: Clone + Serialize + DeserializeOwned, C>(
    map: &SlotMap<K, V>,
    dummy: V,
    convert: impl Fn(&V) -> C,
) -> SlotTable<K, C> {
    let written = ron::to_string(map).expect("a slotmap is written");
    let saved: Vec<SavedSlot<V>> = ron::from_str(&written).expect("a written slotmap is read");
    let vacant = saved.len() - 1 - map.len();
    let mut copy = map.clone();
    let order: Vec<u32> = (0..vacant).map(|_| copy.insert(dummy.clone()).parts().0).collect();
    crate::model::slot_table::parse::<K, V>(saved)
        .expect("a written slotmap is a slot sequence")
        .map(|_, value| Ok::<_, std::convert::Infallible>(convert(&value)))
        .unwrap_or_else(|never| match never {})
        .resumed(&order)
        .expect("a slotmap's free list is its vacant slots")
}

fn insertion_point(value: WindowInsertionPoint) -> InsertionPoint {
    match value {
        WindowInsertionPoint::NextToSelection => InsertionPoint::NextToSelection,
        WindowInsertionPoint::EndOfTree => InsertionPoint::EndOfTree,
    }
}

fn core_orientation(orientation: Orientation) -> core::Orientation {
    match orientation {
        Orientation::Horizontal => core::Orientation::Horizontal,
        Orientation::Vertical => core::Orientation::Vertical,
    }
}

fn core_direction(direction: Direction) -> core::Direction {
    match direction {
        Direction::Left => core::Direction::Left,
        Direction::Right => core::Direction::Right,
        Direction::Up => core::Direction::Up,
        Direction::Down => core::Direction::Down,
    }
}

/// Upstream's two fullscreen flags as one fill (`fullscreen` first, as its projection reads
/// them; its operations never set both).
fn core_fill(fullscreen: bool, within_gaps: bool) -> core::Fill {
    match (fullscreen, within_gaps) {
        (true, _) => core::Fill::Screen,
        (false, true) => core::Fill::WithinGaps,
        (false, false) => core::Fill::Tiled,
    }
}

/// An upstream node as the core holds it.
fn core_node(node: &Node) -> core::Node<NodeId, WindowId> {
    let kind = match &node.kind {
        NodeKind::Split {
            orientation,
            ratio,
            children,
            selected,
        } => Kind::Split(Split {
            orientation: core_orientation(*orientation),
            ratio: *ratio,
            children: *children,
            memory: match selected {
                None => Memory::Stops,
                Some(false) => Memory::Toward(Side::First),
                Some(true) => Memory::Toward(Side::Second),
            },
        }),
        NodeKind::Leaf {
            members,
            fullscreen,
            fullscreen_within_gaps,
            preselected,
        } => Kind::Leaf(Leaf {
            content: core_content(members, core_fill(*fullscreen, *fullscreen_within_gaps)),
            preselect: preselected.map_or(Preselect::Unset, |direction| {
                Preselect::Toward(core_direction(direction))
            }),
        }),
    };
    core::Node {
        parent: node.parent.map_or(Parent::Root, Parent::Split),
        kind,
    }
}

/// A leaf's members as the core holds them (an empty leaf keeps no fill: upstream's operations
/// clear it as the leaf empties).
fn core_content(members: &LeafMembers, fill: core::Fill) -> Content<WindowId> {
    match members {
        LeafMembers::Empty => Content::Empty,
        LeafMembers::Single(window) => Content::Held {
            members: Members::Window(*window),
            fill,
        },
        LeafMembers::Stack { windows, active } => Content::Held {
            members: Members::Stack(Tabs {
                before: windows[..*active].to_vec(),
                top: windows[*active],
                after: windows[*active + 1..].to_vec(),
            }),
            fill,
        },
    }
}

/// Where the core's tree differs from upstream's system, if it does: layouts, nodes (ids
/// compared directly), the windows left out, the settings.
fn view_difference(core: &CoreTree, upstream: &Upstream) -> Option<String> {
    let layouts: Vec<_> = upstream
        .layouts
        .iter()
        .map(|(id, state)| (id, Layout { root: state.root }))
        .collect();
    let core_layouts: Vec<_> = core.layouts().iter().map(|(id, layout)| (id, *layout)).collect();
    if layouts != core_layouts {
        return Some(format!("layouts: upstream {layouts:?}, core {core_layouts:?}"));
    }
    let mut core_nodes = core.nodes().iter();
    for (id, node) in upstream.nodes.iter() {
        let expected = core_node(node);
        match core_nodes.next() {
            Some((core_id, core_node)) if core_id == id && *core_node == expected => {}
            other => {
                return Some(format!("node {id:?}: upstream {expected:?}, core {other:?}"));
            }
        }
    }
    if let Some(extra) = core_nodes.next() {
        return Some(format!("a node upstream lacks: {extra:?}"));
    }
    let left_out: BTreeSet<_> = upstream.inactive.iter().copied().collect();
    if &left_out != core.left_out() {
        return Some(format!(
            "left out: upstream {left_out:?}, core {:?}",
            core.left_out()
        ));
    }
    let settings = (
        insertion_point(upstream.window_insertion_point),
        upstream.single_window_aspect_ratio,
    );
    if settings != (core.insertion(), core.aspect()) {
        return Some(format!(
            "settings: upstream {settings:?}, core {:?}",
            (core.insertion(), core.aspect())
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(idx: u32) -> WindowId { WindowId::new(1, idx) }

    /// The core's tree seeded from a system takes its free list as the system's slotmap holds
    /// it, not as a load would rebuild it (the highest vacant slot first): the next ids match.
    #[test]
    fn a_seeded_core_allocates_the_ids_upstream_would() {
        let mut system = BspLayoutSystem::default();
        let layouts: Vec<_> = (0..4).map(|_| system.create_layout()).collect();
        // Vacate slots 1 and 3 (nodes and layouts), the lower last: upstream hands slot 1 out
        // next, a load would hand out slot 3.
        system.remove_layout(layouts[2]);
        system.remove_layout(layouts[0]);
        system.twin.core = seed(&system.upstream);
        let upstream = system.upstream.create_layout();
        let core = system.twin.core.create_layout();
        assert_eq!(core, upstream);
        assert_eq!(
            system.twin.core.layouts()[core].root,
            system.upstream.layouts[upstream].root
        );
    }

    /// A query the core answers otherwise than upstream's system is counted and fails a test.
    #[test]
    fn a_query_answered_otherwise_is_counted_and_fails_tests() {
        let mut system = BspLayoutSystem::default();
        system.create_layout();
        system.twin.core.set_window_participation(w(1), Participation::LeftOut);
        let before = bsp_tree_divergences();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            system.window_participates(w(1));
        }));
        assert!(failed.is_err());
        assert!(bsp_tree_divergences() > before);
    }

    /// A leaf with a preselected direction (only a saved file holds one) is split that way by
    /// the next window, which goes on that side and is selected; a window after it goes next to
    /// the selection as usual.
    #[test]
    fn a_preselected_direction_puts_the_next_window_on_that_side() {
        let mut upstream = Upstream::new(WindowInsertionPoint::NextToSelection);
        let layout = upstream.create_layout();
        upstream.update(WindowChange::Add(layout, w(1), None));
        let root = upstream.layouts[layout].root;
        let NodeKind::Leaf { preselected, .. } = &mut upstream.nodes[root].kind else {
            unreachable!("a layout of one window is a leaf");
        };
        *preselected = Some(Direction::Left);
        let mut system = BspLayoutSystem::from(upstream);
        system.update(WindowChange::Add(layout, w(2), None));
        assert_eq!(system.window_slot(layout, w(2)), Some(vec![0]));
        assert_eq!(system.window_slot(layout, w(1)), Some(vec![1]));
        assert_eq!(system.selected_window(layout), Some(w(2)));
    }

    /// A core that differs from upstream's system is counted and fails a test (a running Rift
    /// warns and re-seeds it).
    #[test]
    fn a_bsp_tree_divergence_is_counted_and_fails_tests() {
        let mut system = BspLayoutSystem::default();
        let layout = system.create_layout();
        system.twin.core.set_window_participation(w(1), Participation::LeftOut);
        let before = bsp_tree_divergences();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            system.clone_layout(layout);
        }));
        assert!(failed.is_err());
        assert!(bsp_tree_divergences() > before);
    }
}
