//! The stack layout system with the core's stacks beside it (layout trees, stack slice 1).
//! Upstream's system ([`Upstream`]) decides; the core's [`Stacks`] is stepped on every
//! operation, in every build, and held to it: each answer is compared on every operation, the
//! whole system (the view) on every operation in tests and `roc-core` builds and every
//! [`VIEW_EVERY`]th in a plain release build, and each pass's shape (what the core's geometry is
//! handed) on every pass. A difference is counted, fails a test, or in a running Rift is warned
//! and the core re-seeded from upstream's system.

use std::collections::BTreeSet;
use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};

use objc2_core_foundation::CGRect;
use rift_core::layout::stack_tree::{
    self as core, Beyond, DefaultOrientation, Fill, Id, Member as CoreMember, Members, NonEmpty,
    Orientation, Participation, Query, Resize, Selection, Stack as CoreStack, Stacks,
};
use serde::{Deserialize, Serialize};
use slotmap::Key;

use super::{Member, Stack, Upstream};
use crate::actor::app::{WindowId, pid_t};
use crate::common::collections::HashMap;
use crate::common::config::{
    GapSettings, StackDefaultOrientation, StackLineReserve, WindowInsertionPoint,
};
use crate::layout_engine::engine::GroupContainerInfo;
use crate::layout_engine::systems::{
    EditResult, Focus, FocusResult, GroupEdit, LayoutEdit, LayoutSystem, WindowChange,
    WindowLayoutConstraints, WindowQuery, core_direction, insertion_point,
};
use crate::layout_engine::{Direction, LayoutId, LayoutKind};
use crate::model::slot_table::of_slotmap;
use crate::model::tree::NodeId;

/// The core's stacks over rift's ids.
pub(super) type CoreStacks = Stacks<LayoutId, WindowId>;

/// How often a plain release build compares the whole system (the twins' sampling).
const VIEW_EVERY: u64 = 32;

/// Whether every operation compares the whole system.
const VIEW_ALWAYS: bool = cfg!(any(test, feature = "roc-core"));

static STACK_TREE_DIVERGENCES: AtomicU64 = AtomicU64::new(0);

/// Differences between the core's stacks and upstream's stack system, counted process-wide.
pub(crate) fn stack_tree_divergences() -> u64 { STACK_TREE_DIVERGENCES.load(Ordering::Relaxed) }

/// The stack layout system: upstream's decides, and the core's stacks are stepped beside it.
#[derive(Clone, Debug, Deserialize)]
#[serde(from = "Upstream")]
pub struct StackLayoutSystem {
    upstream: Upstream,
    /// Boxed: the system is one variant of `LayoutSystemKind`.
    twin: Box<Twinned>,
}

/// The core's stacks and how many operations stepped them.
#[derive(Clone, Debug)]
struct Twinned {
    core: CoreStacks,
    steps: u64,
}

impl Serialize for StackLayoutSystem {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.upstream.serialize(serializer)
    }
}

impl From<Upstream> for StackLayoutSystem {
    fn from(upstream: Upstream) -> Self {
        let twin = Box::new(Twinned {
            core: seed(&upstream),
            steps: 0,
        });
        Self { upstream, twin }
    }
}

impl Default for StackLayoutSystem {
    fn default() -> Self { Upstream::default().into() }
}

impl StackLayoutSystem {
    pub fn new(orientation: StackDefaultOrientation) -> Self { Upstream::new(orientation).into() }

    pub fn new_with_insertion_point(
        orientation: StackDefaultOrientation,
        insertion: WindowInsertionPoint,
    ) -> Self {
        Upstream::new_with_insertion_point(orientation, insertion).into()
    }

    pub fn update_settings(
        &mut self,
        orientation: StackDefaultOrientation,
        insertion: WindowInsertionPoint,
    ) {
        self.upstream.update_settings(orientation, insertion);
        self.twin
            .core
            .update_settings(default_orientation(orientation), insertion_point(insertion));
        self.stepped("update_settings", &(), &());
    }

    pub(crate) fn collect_group_containers(
        &self,
        layout: LayoutId,
        screen: CGRect,
        offset: f64,
        gaps: &GapSettings,
        line: StackLineReserve,
        selection_path_only: bool,
    ) -> Vec<GroupContainerInfo> {
        let upstream = self.upstream.collect_group_containers(
            layout,
            screen,
            offset,
            gaps,
            line,
            selection_path_only,
        );
        let core = self.twin.core.group_container(layout, selection_path_only).map(|c| {
            GroupContainerInfo {
                node_id: node_id(c.stack),
                container_kind: layout_kind(c.orientation),
                frame: crate::layout_engine::utils::compute_tiling_area(screen, gaps),
                total_count: c.windows.len(),
                selected_index: c.selected,
                window_ids: c.windows,
            }
        });
        self.answered(
            "collect_group_containers",
            upstream.iter().map(container_words).collect::<Vec<_>>(),
            core.iter().map(container_words).collect(),
        );
        upstream
    }

    pub(crate) fn draw_tree(&self, layout: LayoutId) -> String { self.upstream.draw_tree(layout) }

    pub(crate) fn container_tree(&self, layout: LayoutId) -> rift_protocol::ContainerTreeNode {
        self.upstream.container_tree(layout)
    }

    pub(crate) fn window_slot(&self, layout: LayoutId, window: WindowId) -> Option<Vec<usize>> {
        self.answered(
            "window_slot",
            self.upstream.window_slot(layout, window),
            self.twin.core.window_slot(layout, window).map(|i| vec![i]),
        )
    }

    #[cfg(test)]
    pub(crate) fn shadow_frames(
        &self,
        layout: LayoutId,
        screen: CGRect,
        offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &GapSettings,
        line: StackLineReserve,
    ) -> Vec<(WindowId, CGRect)> {
        self.upstream.shadow_frames(layout, screen, offset, constraints, gaps, line)
    }

    #[cfg(test)]
    pub(crate) fn shape(
        &self,
        layout: LayoutId,
        screen: CGRect,
        offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &GapSettings,
        line: StackLineReserve,
    ) -> Option<rift_core::layout::traditional::Traditional<WindowId>> {
        self.upstream.shape(layout, screen, offset, constraints, gaps, line)
    }

    /// After an operation upstream's system and the core both took: their answers compared,
    /// and the systems when due; a difference is reported and the core re-seeded.
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
}

/// Counts a difference, then fails the test, or in a running Rift warns it.
fn report(what: &str, difference: &str) {
    let divergences = STACK_TREE_DIVERGENCES.fetch_add(1, Ordering::Relaxed) + 1;
    #[cfg(test)]
    panic!("stack-tree-divergence ({divergences} so far) after {what}: {difference}");
    #[cfg(not(test))]
    tracing::warn!(
        what,
        divergences,
        difference,
        "stack-tree-divergence: the core's stacks differ from upstream's; re-seeded"
    );
}

impl LayoutSystem for StackLayoutSystem {
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

    /// Upstream's frames (the core's geometry decides them, policy (a)); the core's stacks'
    /// shape is held to upstream's system's on every pass.
    fn calculate_layout(
        &self,
        layout: LayoutId,
        screen: CGRect,
        offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &GapSettings,
        line: StackLineReserve,
        shadow: crate::layout_engine::Shadow,
    ) -> Vec<(WindowId, CGRect)> {
        use crate::layout_engine::core_geometry::{core_constraints, core_stack_line};
        use crate::layout_engine::{core_gaps, core_rect};
        let upstream = self.upstream.shape(layout, screen, offset, constraints, gaps, line);
        let core = self.twin.core.shape(
            layout,
            core_rect(screen),
            core_gaps(gaps),
            offset,
            core_stack_line(line),
            |window| constraints.get(&window).copied().map(core_constraints),
        );
        if upstream != core {
            report(
                "calculate_layout",
                &format!("shapes: upstream {upstream:?}, core {core:?}"),
            );
        }
        self.upstream
            .calculate_layout(layout, screen, offset, constraints, gaps, line, shadow)
    }

    fn selected_window(&self, layout: LayoutId) -> Option<WindowId> {
        self.answered(
            "selected_window",
            self.upstream.selected_window(layout),
            self.twin.core.selected_window(layout),
        )
    }

    fn selection_contains_window(&self, layout: LayoutId, window: WindowId) -> bool {
        self.answered(
            "selection_contains_window",
            self.upstream.selection_contains_window(layout, window),
            self.twin.core.selection_contains_window(layout, window),
        )
    }

    fn window_in_direction(&self, layout: LayoutId, direction: Direction) -> Option<WindowId> {
        self.answered(
            "window_in_direction",
            self.upstream.window_in_direction(layout, direction),
            self.twin.core.window_in_direction(layout, core_direction(direction)),
        )
    }

    fn contains_window(&self, layout: LayoutId, window: WindowId) -> bool {
        self.answered(
            "contains_window",
            self.upstream.contains_window(layout, window),
            self.twin.core.contains_window(layout, window),
        )
    }

    fn has_any_fullscreen_node(&self, layout: LayoutId) -> bool {
        self.answered(
            "has_any_fullscreen_node",
            self.upstream.has_any_fullscreen_node(layout),
            self.twin.core.has_any_fullscreen_node(layout),
        )
    }

    fn parent_of_selection_is_stacked(&self, layout: LayoutId) -> bool {
        self.upstream.parent_of_selection_is_stacked(layout)
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
        let core = match core_edit(edit) {
            Some(edit) => match self.twin.core.edit(layout, edit) {
                core::Edited::Unsupported => EditResult::Unsupported,
                core::Edited::Unchanged => EditResult::Unchanged,
                core::Edited::Changed => EditResult::Changed,
                core::Edited::Raise(windows) => EditResult::Raise(windows),
            },
            // An edit upstream's stack system has no case for.
            None => EditResult::Unsupported,
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

/// A group bar's container, comparable.
fn container_words(
    info: &GroupContainerInfo,
) -> (NodeId, LayoutKind, CGRect, usize, usize, Vec<WindowId>) {
    (
        info.node_id,
        info.container_kind,
        info.frame,
        info.total_count,
        info.selected_index,
        info.window_ids.clone(),
    )
}

/// An edit as the core takes it; none for those upstream's stack system has no case for.
fn core_edit(edit: LayoutEdit) -> Option<core::Edit<WindowId>> {
    Some(match edit {
        LayoutEdit::Move {
            direction,
            has_display_neighbor,
        } => core::Edit::Move {
            direction: core_direction(direction),
            beyond: if has_display_neighbor {
                Beyond::AnotherDisplay
            } else {
                Beyond::Nothing
            },
        },
        LayoutEdit::Fullscreen { within_gaps } => core::Edit::Fullscreen { within_gaps },
        LayoutEdit::Swap(a, b) => core::Edit::Swap(a, b),
        LayoutEdit::ReorderTab { window, target } => core::Edit::ReorderTab { window, target },
        LayoutEdit::Drop { source, target, .. } => core::Edit::Drop { source, target },
        LayoutEdit::Group(GroupEdit::Stack(_)) => core::Edit::Restack,
        LayoutEdit::ToggleOrientation => core::Edit::ToggleOrientation,
        LayoutEdit::Split(_)
        | LayoutEdit::Resize { .. }
        | LayoutEdit::Rebalance
        | LayoutEdit::Group(
            GroupEdit::Join(_)
            | GroupEdit::ConsumeOrExpel(_)
            | GroupEdit::Unstack(_)
            | GroupEdit::Unjoin,
        ) => return None,
    })
}

fn default_orientation(orientation: StackDefaultOrientation) -> DefaultOrientation {
    match orientation {
        StackDefaultOrientation::Perpendicular => DefaultOrientation::Perpendicular,
        StackDefaultOrientation::Same => DefaultOrientation::Same,
        StackDefaultOrientation::Horizontal => DefaultOrientation::Horizontal,
        StackDefaultOrientation::Vertical => DefaultOrientation::Vertical,
    }
}

fn node_id(id: Id) -> NodeId { NodeId::from(slotmap::KeyData::from_ffi(id.word())) }

fn core_id(id: NodeId) -> Id { Id::from_word(id.data().as_ffi()) }

fn layout_kind(orientation: Orientation) -> LayoutKind {
    match orientation {
        Orientation::Horizontal => LayoutKind::HorizontalStack,
        Orientation::Vertical => LayoutKind::VerticalStack,
    }
}

/// Upstream's two fullscreen flags as one fill (`fullscreen` first, as its projection reads
/// them; the load leaves no saved state holding both).
fn core_fill(fullscreen: bool, within_gaps: bool) -> Fill {
    match (fullscreen, within_gaps) {
        (true, _) => Fill::Screen,
        (false, true) => Fill::WithinGaps,
        (false, false) => Fill::Tiled,
    }
}

/// An upstream stack as the core holds it (its load leaves a stack kind, and a selection naming
/// a member exactly when there are members, as its operations keep it).
fn core_stack(stack: &Stack) -> CoreStack<WindowId> {
    let members = stack
        .members
        .iter()
        .map(|m: &Member| CoreMember {
            id: core_id(m.id),
            window: m.window,
            weight: m.weight,
            fill: core_fill(m.fullscreen, m.within_gaps),
        })
        .collect();
    CoreStack {
        id: core_id(stack.id),
        members: match (NonEmpty::new(members), stack.active) {
            (None, _) => Members::Empty,
            (Some(members), Some(id)) => Members::Populated {
                members,
                selection: if stack.root_selected {
                    Selection::Root(core_id(id))
                } else {
                    Selection::Member(core_id(id))
                },
            },
            (Some(_), None) => {
                unreachable!("upstream's load and operations select a member of a populated stack")
            }
        },
        orientation: if stack.kind == LayoutKind::HorizontalStack {
            Orientation::Horizontal
        } else {
            Orientation::Vertical
        },
        fill: core_fill(stack.fullscreen, stack.within_gaps),
    }
}

/// Upstream's system as the core holds it.
fn seed(upstream: &Upstream) -> CoreStacks {
    let dummy = Stack {
        id: NodeId::default(),
        members: Vec::new(),
        active: None,
        root_selected: false,
        kind: LayoutKind::VerticalStack,
        fullscreen: false,
        within_gaps: false,
    };
    CoreStacks::from_parts(
        of_slotmap(&upstream.layouts, dummy, core_stack),
        upstream.next_node,
        default_orientation(upstream.default_orientation),
        upstream.inactive.iter().copied().collect::<BTreeSet<_>>(),
        insertion_point(upstream.window_insertion_point),
    )
}

/// Where the core's stacks differ from upstream's system, if they do: layouts (ids compared
/// directly), the member counter, the windows left out, the settings.
fn view_difference(core: &CoreStacks, upstream: &Upstream) -> Option<String> {
    let layouts: Vec<_> =
        upstream.layouts.iter().map(|(id, stack)| (id, core_stack(stack))).collect();
    let core_layouts: Vec<_> =
        core.layouts().iter().map(|(id, stack)| (id, stack.clone())).collect();
    if layouts != core_layouts {
        return Some(format!("layouts: upstream {layouts:?}, core {core_layouts:?}"));
    }
    if upstream.next_node != core.next_node() {
        return Some(format!(
            "next node: upstream {}, core {}",
            upstream.next_node,
            core.next_node()
        ));
    }
    let left_out: BTreeSet<_> = upstream.inactive.iter().copied().collect();
    if &left_out != core.left_out() {
        return Some(format!(
            "left out: upstream {left_out:?}, core {:?}",
            core.left_out()
        ));
    }
    let settings = (
        default_orientation(upstream.default_orientation),
        insertion_point(upstream.window_insertion_point),
    );
    if settings != (core.orientation(), core.insertion()) {
        return Some(format!(
            "settings: upstream {settings:?}, core {:?}",
            (core.orientation(), core.insertion())
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(idx: u32) -> WindowId { WindowId::new(1, idx) }

    /// The core's stacks seeded from a system take its free list as the system's slotmap holds
    /// it, not as a load would rebuild it (the highest vacant slot first): the next ids match.
    #[test]
    fn a_seeded_core_allocates_the_ids_upstream_would() {
        let mut system = StackLayoutSystem::default();
        let layouts: Vec<_> = (0..4).map(|_| system.create_layout()).collect();
        // Vacate slots 1 and 3, the lower last: upstream hands slot 1 out next, a load would
        // hand out slot 3.
        system.remove_layout(layouts[2]);
        system.remove_layout(layouts[0]);
        system.twin.core = seed(&system.upstream);
        let upstream = system.upstream.create_layout();
        let core = system.twin.core.create_layout();
        assert_eq!(core, upstream);
        assert_eq!(
            system.twin.core.layouts()[core].id,
            core_id(system.upstream.layouts[upstream].id)
        );
    }

    /// A query the core answers otherwise than upstream's system is counted and fails a test.
    #[test]
    fn a_query_answered_otherwise_is_counted_and_fails_tests() {
        let mut system = StackLayoutSystem::default();
        system.create_layout();
        system.twin.core.set_window_participation(w(1), Participation::LeftOut);
        let before = stack_tree_divergences();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            system.window_participates(w(1));
        }));
        assert!(failed.is_err());
        assert!(stack_tree_divergences() > before);
    }

    /// A core that differs from upstream's system is counted and fails a test (a running Rift
    /// warns and re-seeds it).
    #[test]
    fn a_stack_tree_divergence_is_counted_and_fails_tests() {
        let mut system = StackLayoutSystem::default();
        let layout = system.create_layout();
        system.twin.core.set_window_participation(w(1), Participation::LeftOut);
        let before = stack_tree_divergences();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            system.clone_layout(layout);
        }));
        assert!(failed.is_err());
        assert!(stack_tree_divergences() > before);
    }

    /// The twin's cost per operation at 10, 100 and 1000 windows (release:
    /// `cargo test --release --lib measure_stack_twin -- --ignored --nocapture`): upstream's
    /// operation alone, the core's step alone, the whole-system comparison, a re-seed, and a
    /// pass's shape built by each. A plain release build pays the step on every operation and
    /// the comparison on every 32nd.
    #[test]
    #[ignore]
    fn measure_stack_twin() {
        use std::hint::black_box;
        use std::time::Instant;

        use crate::layout_engine::core_geometry::{core_constraints, core_stack_line};
        use crate::layout_engine::{core_gaps, core_rect};
        let directions = [
            Direction::Left,
            Direction::Up,
            Direction::Right,
            Direction::Down,
        ];
        let ops = 4000_u32;
        let each = |started: Instant, n: u32| started.elapsed() / n;
        for count in [10_u32, 100, 1000] {
            let mut system = StackLayoutSystem::new(StackDefaultOrientation::Horizontal);
            let layout = system.create_layout();
            for idx in 1..=count {
                system.update(WindowChange::Add(layout, w(idx), None));
            }
            // A mix of everyday operations: focus moves, swaps, fullscreen, a window coming and
            // going.
            let mut upstream = system.upstream.clone();
            let started = Instant::now();
            for i in 0..ops {
                match i % 4 {
                    0 => drop(black_box(
                        upstream.focus(layout, Focus::Direction(directions[(i / 4 % 4) as usize])),
                    )),
                    1 => drop(black_box(upstream.edit_layout(
                        layout,
                        LayoutEdit::Swap(w(1 + i % count), w(1 + (i * 7) % count)),
                    ))),
                    2 => {
                        drop(black_box(upstream.edit_layout(layout, LayoutEdit::Fullscreen {
                            within_gaps: false,
                        })))
                    }
                    _ => {
                        upstream.update(WindowChange::Add(layout, w(count + 1), None));
                        upstream.update(WindowChange::Remove(w(count + 1)));
                    }
                }
            }
            let upstream_each = each(started, ops);
            let mut core = system.twin.core.clone();
            let started = Instant::now();
            for i in 0..ops {
                match i % 4 {
                    0 => drop(black_box(core.focus(
                        layout,
                        core::Focus::Direction(core_direction(directions[(i / 4 % 4) as usize])),
                    ))),
                    1 => drop(black_box(core.edit(
                        layout,
                        core::Edit::Swap(w(1 + i % count), w(1 + (i * 7) % count)),
                    ))),
                    2 => drop(black_box(
                        core.edit(layout, core::Edit::Fullscreen { within_gaps: false }),
                    )),
                    _ => {
                        core.add_window(layout, w(count + 1));
                        core.remove_window(w(count + 1));
                    }
                }
            }
            let core_each = each(started, ops);
            let rounds = 200;
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(view_difference(&system.twin.core, &system.upstream));
            }
            let view_each = each(started, rounds);
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(seed(&system.upstream));
            }
            let seed_each = each(started, rounds);
            let screen = CGRect::new(
                objc2_core_foundation::CGPoint::ZERO,
                objc2_core_foundation::CGSize::new(1600.0, 1000.0),
            );
            let gaps = GapSettings::default();
            let constraints = HashMap::default();
            let line = StackLineReserve::default();
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(system.upstream.shape(layout, screen, 12.0, &constraints, &gaps, line));
            }
            let upstream_shape = each(started, rounds);
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(system.twin.core.shape(
                    layout,
                    core_rect(screen),
                    core_gaps(&gaps),
                    12.0,
                    core_stack_line(line),
                    |window| constraints.get(&window).copied().map(core_constraints),
                ));
            }
            let core_shape = each(started, rounds);
            let per_op = view_each / VIEW_EVERY as u32;
            eprintln!(
                "{count} windows: upstream {upstream_each:?}/op, core {core_each:?}/op, view \
                 {view_each:?} ({per_op:?}/op 1 in {VIEW_EVERY}), re-seed {seed_each:?}, \
                 shape upstream {upstream_shape:?} core {core_shape:?}"
            );
        }
    }
}
