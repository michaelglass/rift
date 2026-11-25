//! The stack layout system: the core's stacks decide (layout trees, stack slice 3, the flip,
//! policy (a)) and are saved in upstream's form, byte for byte; upstream's system
//! ([`Upstream`]) is their shadow, stepped on every operation in every build and held to them:
//! each answer is compared on every operation, the whole system (the view) on every operation in
//! tests and `roc-core` builds and every [`VIEW_EVERY`]th in a plain release build, and each
//! pass's shape and frames on every pass. A difference is counted, fails a test, or in a running
//! Rift is warned and the shadow re-seeded from the core (the deciding side stands).

use std::collections::BTreeSet;
use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};

use objc2_core_foundation::CGRect;
use rift_core::layout::stack_tree::{
    self as core, Beyond, DefaultOrientation, Fill, Id, Member as CoreMember, Members, NonEmpty,
    Orientation, Participation, Query, Resize, Selection, Stack as CoreStack, Stacks,
};
#[cfg(feature = "roc-core")]
use rift_core_roc::stack_tree::{Answer, Event, Scope};
use serde::{Deserialize, Serialize};
use slotmap::Key;

#[cfg(feature = "roc-core")]
use super::roc_shadow::{self as roc, RocShadow};
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
use crate::model::slot_table::{SavedSlot, of_slotmap, saved, to_slotmap};
use crate::model::tree::NodeId;

/// The core's stacks over rift's ids.
pub(crate) type CoreStacks = Stacks<LayoutId, WindowId>;

/// How often a plain release build compares the whole system (the twins' sampling).
const VIEW_EVERY: u64 = 32;

/// Whether every operation compares the whole system.
const VIEW_ALWAYS: bool = cfg!(any(test, feature = "roc-core"));

static STACK_TREE_DIVERGENCES: AtomicU64 = AtomicU64::new(0);

/// Differences between the core's stacks and upstream's stack system, counted process-wide.
pub(crate) fn stack_tree_divergences() -> u64 { STACK_TREE_DIVERGENCES.load(Ordering::Relaxed) }

/// The stack layout system: the core's stacks decide, and upstream's system is stepped beside
/// them. Loaded through upstream's load (which parses the legacy form and normalizes what only a
/// file holds), saved from the core in the same form.
#[derive(Clone, Debug, Deserialize)]
#[serde(from = "Upstream")]
pub struct StackLayoutSystem {
    core: CoreStacks,
    /// Boxed: the system is one variant of `LayoutSystemKind`.
    shadow: Box<Shadow>,
}

/// Upstream's system and how many operations stepped it; under `roc-core` also the Roc stacks,
/// a second shadow.
#[derive(Clone, Debug)]
struct Shadow {
    upstream: Upstream,
    steps: u64,
    #[cfg(feature = "roc-core")]
    roc: RocShadow,
}

/// The core's stacks as upstream saves its system (the field names and order of upstream's
/// derived form, under its original name), byte for byte.
#[derive(Serialize)]
#[serde(rename = "StackLayoutSystem")]
struct Saved {
    layouts: Vec<SavedSlot<Stack>>,
    next_node: u32,
    default_orientation: StackDefaultOrientation,
}

impl Serialize for StackLayoutSystem {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Saved {
            layouts: saved(self.core.layouts(), |_, stack| upstream_stack(stack)).collect(),
            next_node: self.core.next_node(),
            default_orientation: stack_default_orientation(self.core.orientation()),
        }
        .serialize(serializer)
    }
}

impl From<Upstream> for StackLayoutSystem {
    fn from(upstream: Upstream) -> Self {
        let core = seed(&upstream);
        Self {
            shadow: Box::new(Shadow {
                #[cfg(feature = "roc-core")]
                roc: RocShadow::seeded(&core),
                upstream,
                steps: 0,
            }),
            core,
        }
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
        self.shadow.upstream.update_settings(orientation, insertion);
        let (orientation, insertion) =
            (default_orientation(orientation), insertion_point(insertion));
        self.core.update_settings(orientation, insertion);
        self.stepped("update_settings", &(), &());
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "update_settings",
            Event::Settings(orientation, insertion),
            Answer::Done,
        );
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
        let upstream = self.shadow.upstream.collect_group_containers(
            layout,
            screen,
            offset,
            gaps,
            line,
            selection_path_only,
        );
        let core =
            self.core
                .group_container(layout, selection_path_only)
                .map(|c| GroupContainerInfo {
                    node_id: node_id(c.stack),
                    container_kind: layout_kind(c.orientation),
                    frame: crate::layout_engine::utils::compute_tiling_area(screen, gaps),
                    total_count: c.windows.len(),
                    selected_index: c.selected,
                    window_ids: c.windows,
                });
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "collect_group_containers",
            Event::GroupContainer(
                roc::layout_key(layout),
                if selection_path_only {
                    Scope::SelectionPath
                } else {
                    Scope::Whole
                },
            ),
            Answer::Group(roc::group(
                &self.core.group_container(layout, selection_path_only),
            )),
        );
        let core: Vec<GroupContainerInfo> = core.into_iter().collect();
        self.answered(
            "collect_group_containers",
            upstream.iter().map(container_words).collect::<Vec<_>>(),
            core.iter().map(container_words).collect(),
        );
        core
    }

    /// rift-cli's drawing of the layout: the core's (upstream's held to it on every call).
    pub(crate) fn draw_tree(&self, layout: LayoutId) -> String {
        let core = self.core.draw(layout);
        self.answered("draw_tree", self.shadow.upstream.draw_tree(layout), core.clone());
        core
    }

    /// rift-cli's container view of the layout: the core's (upstream's held to it on every
    /// call).
    pub(crate) fn container_tree(&self, layout: LayoutId) -> rift_protocol::ContainerTreeNode {
        let core = container_node(self.core.container(layout));
        self.answered(
            "container_tree",
            self.shadow.upstream.container_tree(layout),
            core.clone(),
        );
        core
    }

    pub(crate) fn window_slot(&self, layout: LayoutId, window: WindowId) -> Option<Vec<usize>> {
        let core = self.core.window_slot(layout, window);
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "window_slot",
            Event::WindowSlot(roc::layout_key(layout), roc::window(window)),
            Answer::WindowSlot(core),
        );
        self.answered(
            "window_slot",
            self.shadow.upstream.window_slot(layout, window),
            core.map(|i| vec![i]),
        )
    }

    /// An edit the core decides (under `roc-core` the Roc stacks take it too).
    fn core_edit(
        &mut self,
        layout: LayoutId,
        edit: core::Edit<WindowId>,
    ) -> core::Edited<WindowId> {
        let edited = self.core.edit(layout, edit);
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "edit_layout",
            Event::Edit(roc::layout_key(layout), roc::edit(edit)),
            Answer::Edited(roc::edited(&edited)),
        );
        edited
    }

    /// Under `roc-core`, the Roc stacks take an operation the core decided (answering
    /// `answer`); a difference re-seeds them from the core.
    #[cfg(feature = "roc-core")]
    fn roc_stepped(&mut self, what: &str, event: Event, answer: Answer) {
        self.shadow.roc.stepped(&self.core, what, event, answer);
    }

    /// Under `roc-core`, the Roc stacks answer a query the core answered (`answer`).
    #[cfg(feature = "roc-core")]
    fn roc_asked(&self, what: &str, event: Event, answer: Answer) {
        self.shadow.roc.asked(what, event, answer);
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
        self.shadow
            .upstream
            .shadow_frames(layout, screen, offset, constraints, gaps, line)
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
        use crate::layout_engine::core_geometry::{core_constraints, core_stack_line};
        use crate::layout_engine::{core_gaps, core_rect};
        self.core.shape(
            layout,
            core_rect(screen),
            core_gaps(gaps),
            offset,
            core_stack_line(line),
            |window| constraints.get(&window).copied().map(core_constraints),
        )
    }

    /// The deciding core, for tests that reach past the system.
    #[cfg(test)]
    pub(crate) fn core_for_tests(&self) -> &CoreStacks { &self.core }

    /// Upstream's system re-seeded from the core, as after a divergence.
    #[cfg(test)]
    pub(crate) fn reseed_shadow(&mut self) {
        self.shadow.upstream = upstream_of(&self.core);
        #[cfg(feature = "roc-core")]
        {
            self.shadow.roc = RocShadow::seeded(&self.core);
        }
    }

    /// Upstream's system saved as upstream saves it (the core's save is held to it).
    #[cfg(test)]
    pub(crate) fn upstream_saved(&self) -> String {
        ron::to_string(&self.shadow.upstream).expect("upstream's system is written")
    }

    /// After an operation the core and upstream's system both took: their answers compared,
    /// and the systems when due; a difference is reported and the shadow re-seeded from the
    /// core.
    fn stepped<A: PartialEq + Debug>(&mut self, what: &str, upstream: &A, core: &A) {
        self.shadow.steps += 1;
        let answers_differ = upstream != core;
        if answers_differ {
            report(what, &format!("answers: upstream {upstream:?}, core {core:?}"));
        }
        let view_due = VIEW_ALWAYS || self.shadow.steps.is_multiple_of(VIEW_EVERY);
        let view = (answers_differ || view_due)
            .then(|| view_difference(&self.core, &self.shadow.upstream));
        if let Some(Some(difference)) = &view {
            report(what, difference);
        }
        if answers_differ || matches!(view, Some(Some(_))) {
            self.shadow.upstream = upstream_of(&self.core);
        }
    }

    /// A query both answered, the core's answer given: a difference is reported (the next
    /// step's view re-seeds the shadow).
    fn answered<A: PartialEq + Debug>(&self, what: &str, upstream: A, core: A) -> A {
        if upstream != core {
            report(what, &format!("answers: upstream {upstream:?}, core {core:?}"));
        }
        core
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
        "stack-tree-divergence: upstream's stack system differs from the core's stacks; re-seeded from the core"
    );
}

impl LayoutSystem for StackLayoutSystem {
    fn has_windows_for_app(&self, layout: LayoutId, pid: pid_t) -> bool {
        let core = self.core.has_windows_for_app(layout, |window| window.pid == pid);
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "has_windows_for_app",
            Event::HasWindowsForApp(
                roc::layout_key(layout),
                roc::app_windows(&self.core, |window| window.pid == pid),
            ),
            Answer::Verdict(core),
        );
        self.answered(
            "has_windows_for_app",
            self.shadow.upstream.has_windows_for_app(layout, pid),
            core,
        )
    }

    fn window_participates(&self, window: WindowId) -> bool {
        let core = self.core.window_participates(window);
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "window_participates",
            Event::WindowParticipates(roc::window(window)),
            Answer::Verdict(core),
        );
        self.answered(
            "window_participates",
            self.shadow.upstream.window_participates(window),
            core,
        )
    }

    fn set_window_participation(&mut self, window: WindowId, participates: bool) -> bool {
        let upstream = self.shadow.upstream.set_window_participation(window, participates);
        let participation = if participates {
            Participation::TakesPart
        } else {
            Participation::LeftOut
        };
        let core = self.core.set_window_participation(window, participation);
        self.stepped("set_window_participation", &upstream, &core);
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "set_window_participation",
            Event::Participation(roc::window(window), participation),
            Answer::Participated(core),
        );
        core
    }

    fn create_layout(&mut self) -> LayoutId {
        let upstream = self.shadow.upstream.create_layout();
        let core = self.core.create_layout();
        self.stepped("create_layout", &upstream, &core);
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "create_layout",
            Event::CreateLayout,
            Answer::Layout(roc::layout_key(core)),
        );
        core
    }

    fn contains_layout(&self, layout: LayoutId) -> bool {
        let core = self.core.contains_layout(layout);
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "contains_layout",
            Event::ContainsLayout(roc::layout_key(layout)),
            Answer::Verdict(core),
        );
        self.answered(
            "contains_layout",
            self.shadow.upstream.contains_layout(layout),
            core,
        )
    }

    fn clone_layout(&mut self, layout: LayoutId) -> LayoutId {
        let upstream = self.shadow.upstream.clone_layout(layout);
        let core = self.core.clone_layout(layout);
        self.stepped("clone_layout", &upstream, &core);
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "clone_layout",
            Event::CloneLayout(roc::layout_key(layout)),
            Answer::Layout(roc::layout_key(core)),
        );
        core
    }

    fn remove_layout(&mut self, layout: LayoutId) {
        self.shadow.upstream.remove_layout(layout);
        self.core.remove_layout(layout);
        self.stepped("remove_layout", &(), &());
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "remove_layout",
            Event::RemoveLayout(roc::layout_key(layout)),
            Answer::Done,
        );
    }

    /// The core geometry's frames for the core's shape (policy (a)): upstream's shape is held to
    /// the core's on every pass, upstream's geometry is the frames' shadow.
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
        let upstream = self.shadow.upstream.shape(layout, screen, offset, constraints, gaps, line);
        let core = self.core.shape(
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
        crate::layout_engine::core_geometry::decide_traditional("stack", core, shadow, || {
            self.shadow
                .upstream
                .shadow_frames(layout, screen, offset, constraints, gaps, line)
        })
    }

    fn selected_window(&self, layout: LayoutId) -> Option<WindowId> {
        let core = self.core.selected_window(layout);
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "selected_window",
            Event::SelectedWindow(roc::layout_key(layout)),
            Answer::Window(core.map(roc::window)),
        );
        self.answered(
            "selected_window",
            self.shadow.upstream.selected_window(layout),
            core,
        )
    }

    fn selection_contains_window(&self, layout: LayoutId, window: WindowId) -> bool {
        let core = self.core.selection_contains_window(layout, window);
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "selection_contains_window",
            Event::SelectionContainsWindow(roc::layout_key(layout), roc::window(window)),
            Answer::Verdict(core),
        );
        self.answered(
            "selection_contains_window",
            self.shadow.upstream.selection_contains_window(layout, window),
            core,
        )
    }

    fn window_in_direction(&self, layout: LayoutId, direction: Direction) -> Option<WindowId> {
        let core = self.core.window_in_direction(layout, core_direction(direction));
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "window_in_direction",
            Event::WindowInDirection(roc::layout_key(layout), core_direction(direction)),
            Answer::Window(core.map(roc::window)),
        );
        self.answered(
            "window_in_direction",
            self.shadow.upstream.window_in_direction(layout, direction),
            core,
        )
    }

    fn contains_window(&self, layout: LayoutId, window: WindowId) -> bool {
        let core = self.core.contains_window(layout, window);
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "contains_window",
            Event::ContainsWindow(roc::layout_key(layout), roc::window(window)),
            Answer::Verdict(core),
        );
        self.answered(
            "contains_window",
            self.shadow.upstream.contains_window(layout, window),
            core,
        )
    }

    fn has_any_fullscreen_node(&self, layout: LayoutId) -> bool {
        let core = self.core.has_any_fullscreen_node(layout);
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "has_any_fullscreen_node",
            Event::HasAnyFullscreenNode(roc::layout_key(layout)),
            Answer::Verdict(core),
        );
        self.answered(
            "has_any_fullscreen_node",
            self.shadow.upstream.has_any_fullscreen_node(layout),
            core,
        )
    }

    fn parent_of_selection_is_stacked(&self, layout: LayoutId) -> bool {
        self.shadow.upstream.parent_of_selection_is_stacked(layout)
    }

    fn windows(&self, layout: LayoutId, query: WindowQuery) -> Vec<WindowId> {
        let core_query = match query {
            WindowQuery::All => Query::All,
            WindowQuery::Visible => Query::Visible,
            WindowQuery::Selection => Query::Selection,
            WindowQuery::Stack(window) => Query::Stack(window),
        };
        let core = self.core.windows(layout, core_query);
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "windows",
            Event::Windows(roc::layout_key(layout), roc::query(core_query)),
            Answer::Windows(roc::windows(&core)),
        );
        self.answered("windows", self.shadow.upstream.windows(layout, query), core)
    }

    fn focus(&mut self, layout: LayoutId, focus: Focus) -> FocusResult {
        let upstream = self.shadow.upstream.focus(layout, focus);
        let core_focus = match focus {
            Focus::Select(window) => core::Focus::Select(window),
            Focus::Ascend => core::Focus::Ascend,
            Focus::Descend => core::Focus::Descend,
            Focus::Direction(direction) => core::Focus::Direction(core_direction(direction)),
        };
        let focused = self.core.focus(layout, core_focus);
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "focus",
            Event::Focus(roc::layout_key(layout), roc::focus(core_focus)),
            Answer::Focused(roc::focused(&focused)),
        );
        let core = match focused {
            core::Focused::Selection { handled } => FocusResult::selection(handled),
            core::Focused::Movement { window, raise } => FocusResult::movement((window, raise)),
        };
        self.stepped("focus", &focus_words(&upstream), &focus_words(&core));
        core
    }

    fn update(&mut self, change: WindowChange<'_>) {
        let applied = Applied::of(&change);
        // Read before the core applies it: an app's windows are the ones the core holds.
        #[cfg(feature = "roc-core")]
        let event = applied.event(&self.core);
        applied.apply(&mut self.core);
        self.shadow.upstream.update(change);
        self.stepped("update", &(), &());
        #[cfg(feature = "roc-core")]
        self.roc_stepped("update", event, Answer::Done);
    }

    fn edit_layout(&mut self, layout: LayoutId, edit: LayoutEdit) -> EditResult {
        let upstream = self.shadow.upstream.edit_layout(layout, edit);
        let core = match core_edit(edit) {
            Some(edit) => match self.core_edit(layout, edit) {
                core::Edited::Unsupported => EditResult::Unsupported,
                core::Edited::Unchanged => EditResult::Unchanged,
                core::Edited::Changed => EditResult::Changed,
                core::Edited::Raise(windows) => EditResult::Raise(windows),
            },
            // An edit upstream's stack system has no case for.
            None => EditResult::Unsupported,
        };
        self.stepped("edit_layout", &edit_words(&upstream), &edit_words(&core));
        core
    }
}

/// A window change as the core takes it.
enum Applied {
    Add(LayoutId, WindowId),
    Remove(WindowId),
    Replace(WindowId, WindowId),
    CloseApp(pid_t),
    Resize(LayoutId, WindowId, Resize),
}

impl Applied {
    fn of(change: &WindowChange<'_>) -> Self {
        use crate::layout_engine::core_rect;
        match *change {
            WindowChange::Add(layout, window, _) => Applied::Add(layout, window),
            WindowChange::Remove(window) | WindowChange::RemoveAndRebalance(window) => {
                Applied::Remove(window)
            }
            WindowChange::Replace(from, to) => Applied::Replace(from, to),
            WindowChange::CloseApp(pid) => Applied::CloseApp(pid),
            WindowChange::Resize {
                layout,
                window,
                old,
                new,
                screen,
                gaps,
            } => Applied::Resize(layout, window, Resize {
                old: core_rect(old),
                new: core_rect(new),
                screen: core_rect(screen),
                tiling: core_rect(crate::layout_engine::utils::compute_tiling_area(screen, gaps)),
            }),
        }
    }

    fn apply(&self, core: &mut CoreStacks) {
        match *self {
            Applied::Add(layout, window) => core.add_window(layout, window),
            Applied::Remove(window) => core.remove_window(window),
            Applied::Replace(from, to) => core.replace_window(from, to),
            Applied::CloseApp(pid) => core.close_app(|window| window.pid == pid),
            Applied::Resize(layout, window, resize) => core.resize_window(layout, window, resize),
        }
    }

    /// The change as the Roc stacks take it (an app closing lists the app's windows the core
    /// holds, as Roc has no pid).
    #[cfg(feature = "roc-core")]
    fn event(&self, core: &CoreStacks) -> Event {
        match *self {
            Applied::Add(layout, window) => {
                Event::AddWindow(roc::layout_key(layout), roc::window(window))
            }
            Applied::Remove(window) => Event::RemoveWindow(roc::window(window)),
            Applied::Replace(from, to) => Event::ReplaceWindow {
                from: roc::window(from),
                to: roc::window(to),
            },
            Applied::CloseApp(pid) => {
                Event::CloseApp(roc::app_windows(core, |window| window.pid == pid))
            }
            Applied::Resize(layout, window, resize) => {
                Event::ResizeWindow(roc::layout_key(layout), roc::window(window), resize)
            }
        }
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

/// The core's container view as rift-cli's node: the stack, then each member.
fn container_node(container: core::Container<WindowId>) -> rift_protocol::ContainerTreeNode {
    use rift_protocol::{ContainerNodeType, ContainerTreeNode as Node};
    let flags = |fill: Fill| (fill == Fill::Screen, fill == Fill::WithinGaps);
    let (is_fullscreen, is_fullscreen_within_gaps) = flags(container.fill);
    Node {
        node_id: container.stack.word(),
        layout_kind: Some(layout_kind(container.orientation)),
        is_selected: container.selected,
        is_fullscreen,
        is_fullscreen_within_gaps,
        children: container
            .members
            .into_iter()
            .map(|held| {
                let (is_fullscreen, is_fullscreen_within_gaps) = flags(held.member.fill);
                Node {
                    node_id: held.member.id.word(),
                    node_type: ContainerNodeType::Window,
                    window_id: Some(held.member.window.into()),
                    weight: Some(held.member.weight),
                    is_selected: held.selected,
                    is_fullscreen,
                    is_fullscreen_within_gaps,
                    ..Default::default()
                }
            })
            .collect(),
        ..Default::default()
    }
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

/// The core's stacks as upstream's system: the shadow re-seeded from the core. The table keeps
/// the core's slots and versions and takes its vacant slots in the core's order, so both
/// allocate the same layouts next.
fn upstream_of(core: &CoreStacks) -> Upstream {
    let dummy = upstream_stack(&CoreStack {
        id: Id::from_word(0),
        members: Members::Empty,
        orientation: Orientation::Vertical,
        fill: Fill::Tiled,
    });
    Upstream {
        layouts: to_slotmap(core.layouts(), upstream_stack, dummy),
        next_node: core.next_node(),
        default_orientation: stack_default_orientation(core.orientation()),
        inactive: core.left_out().iter().copied().collect(),
        window_insertion_point: window_insertion_point(core.insertion()),
    }
}

/// A core stack as upstream's: the selection as `active` and `root_selected`, each fill as two
/// flags, the orientation as a stack layout kind.
fn upstream_stack(stack: &CoreStack<WindowId>) -> Stack {
    let flags = |fill: Fill| (fill == Fill::Screen, fill == Fill::WithinGaps);
    let (fullscreen, within_gaps) = flags(stack.fill);
    let selection = stack.selection();
    Stack {
        id: node_id(stack.id),
        members: stack
            .list()
            .iter()
            .map(|member| {
                let (fullscreen, within_gaps) = flags(member.fill);
                Member {
                    id: node_id(member.id),
                    window: member.window,
                    weight: member.weight,
                    fullscreen,
                    within_gaps,
                }
            })
            .collect(),
        active: selection.map(|chosen| node_id(chosen.member())),
        root_selected: matches!(selection, Some(Selection::Root(_))),
        kind: layout_kind(stack.orientation),
        fullscreen,
        within_gaps,
    }
}

fn stack_default_orientation(orientation: DefaultOrientation) -> StackDefaultOrientation {
    match orientation {
        DefaultOrientation::Perpendicular => StackDefaultOrientation::Perpendicular,
        DefaultOrientation::Same => StackDefaultOrientation::Same,
        DefaultOrientation::Horizontal => StackDefaultOrientation::Horizontal,
        DefaultOrientation::Vertical => StackDefaultOrientation::Vertical,
    }
}

fn window_insertion_point(value: core::InsertionPoint) -> WindowInsertionPoint {
    match value {
        core::InsertionPoint::NextToSelection => WindowInsertionPoint::NextToSelection,
        core::InsertionPoint::EndOfTree => WindowInsertionPoint::EndOfTree,
    }
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
        system.core = seed(&system.shadow.upstream);
        let upstream = system.shadow.upstream.create_layout();
        let core = system.core.create_layout();
        assert_eq!(core, upstream);
        assert_eq!(
            system.core.layouts()[core].id,
            core_id(system.shadow.upstream.layouts[upstream].id)
        );
    }

    /// Upstream's system re-seeded from the core (after a divergence) takes the core's free list,
    /// so both hand out the same layouts next.
    #[test]
    fn a_shadow_reseeded_from_the_core_allocates_the_ids_the_core_would() {
        let mut system = StackLayoutSystem::default();
        let layouts: Vec<_> = (0..5).map(|_| system.create_layout()).collect();
        for layout in [layouts[3], layouts[0], layouts[2]] {
            system.remove_layout(layout);
        }
        system.shadow.upstream = upstream_of(&system.core);
        for _ in 0..4 {
            let upstream = system.shadow.upstream.create_layout();
            let core = system.core.create_layout();
            assert_eq!(upstream, core);
        }
        assert_eq!(view_difference(&system.core, &system.shadow.upstream), None);
    }

    /// A query the core answers otherwise than upstream's system is counted and fails a test.
    #[test]
    fn a_query_answered_otherwise_is_counted_and_fails_tests() {
        let mut system = StackLayoutSystem::default();
        system.create_layout();
        system.shadow.upstream.set_window_participation(w(1), false);
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
        system.shadow.upstream.set_window_participation(w(1), false);
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
            let mut upstream = system.shadow.upstream.clone();
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
            let mut core = system.core.clone();
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
                black_box(view_difference(&system.core, &system.shadow.upstream));
            }
            let view_each = each(started, rounds);
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(upstream_of(&system.core));
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
                black_box(system.shadow.upstream.shape(
                    layout,
                    screen,
                    12.0,
                    &constraints,
                    &gaps,
                    line,
                ));
            }
            let upstream_shape = each(started, rounds);
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(system.core.shape(
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
