//! The traditional layout system: the core's forest decides (layout trees, the group forest's
//! slice 3, the flip, policy (a)) and is saved in upstream's form, byte for byte; upstream's
//! system ([`Upstream`]) is its shadow, stepped on every operation in every build and held to it
//! (`group_twin` says how), and re-seeded from the core on a difference (the deciding side
//! stands).

use objc2_core_foundation::CGRect;
use rift_core::layout::group_forest::{InsertionPoint, NewSibling, Traditional};
#[cfg(feature = "roc-core")]
use rift_core_roc::RocTraditional;
#[cfg(feature = "roc-core")]
use rift_core_roc::group_forest::{
    Answer, ForestEvent as F, TraditionalEvent as E, TraditionalView,
};
use serde::{Deserialize, Serialize};

use super::Upstream;
use crate::actor::app::{WindowId, pid_t};
use crate::common::collections::HashMap;
use crate::common::config::{GapSettings, StackLineReserve, WindowInsertionPoint};
use crate::layout_engine::engine::GroupContainerInfo;
#[cfg(feature = "roc-core")]
use crate::layout_engine::systems::group_roc::{self as roc, RocShadow};
use crate::layout_engine::systems::group_stored::{Saved, upstream_tiles};
use crate::layout_engine::systems::group_twin::{
    VIEW_ALWAYS, VIEW_EVERY, container_node, core_edit, core_focus, core_query, edit_result,
    edit_words, focus_result, focus_words, participation, pass, project_groups, report, resize,
    seed, view_difference,
};
use crate::layout_engine::systems::{
    EditResult, Focus, FocusResult, LayoutEdit, LayoutSystem, WindowChange,
    WindowLayoutConstraints, WindowQuery, core_direction, insertion_point,
};
use crate::layout_engine::{Direction, LayoutId};
use crate::model::tree::NodeId;

type CoreTraditional = Traditional<LayoutId, NodeId, WindowId>;

const SYSTEM: &str = "traditional";

/// The traditional layout system: the core's forest decides, and upstream's system is stepped
/// beside it. Loaded through upstream's load (which parses the legacy forms and normalizes what
/// only a file holds), saved from the core in the same form.
#[derive(Clone, Debug, Deserialize)]
#[serde(from = "Upstream")]
pub struct TraditionalLayoutSystem {
    core: CoreTraditional,
    /// Boxed: the system is one variant of `LayoutSystemKind`.
    shadow: Box<Shadow>,
}

/// Upstream's system and how many operations stepped it.
#[derive(Clone, Debug)]
struct Shadow {
    upstream: Upstream,
    steps: u64,
    /// Under `roc-core`, the Roc forest: the core's second shadow.
    #[cfg(feature = "roc-core")]
    roc: RocShadow<RocTraditional>,
}

impl Serialize for TraditionalLayoutSystem {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Saved::of(self.core.forest()).serialize(serializer)
    }
}

fn new_sibling(equalize_nodes: bool) -> NewSibling {
    if equalize_nodes {
        NewSibling::AverageWeight
    } else {
        NewSibling::HalvesSelection
    }
}

fn core_of(upstream: &Upstream) -> CoreTraditional {
    CoreTraditional::from_parts(
        seed(&upstream.tiles),
        insertion_point(upstream.window_insertion_point),
        new_sibling(upstream.equalize_nodes),
    )
}

/// Upstream's system re-seeded from the core: its forest and its settings.
fn upstream_of(core: &CoreTraditional) -> Upstream {
    Upstream {
        tiles: upstream_tiles(core.forest()),
        window_insertion_point: match core.insertion() {
            InsertionPoint::NextToSelection => WindowInsertionPoint::NextToSelection,
            InsertionPoint::EndOfTree => WindowInsertionPoint::EndOfTree,
        },
        equalize_nodes: core.new_sibling() == NewSibling::AverageWeight,
    }
}

/// The core's system as the Roc forest's view gives it.
#[cfg(feature = "roc-core")]
fn traditional_view(core: &CoreTraditional) -> TraditionalView {
    TraditionalView {
        forest: roc::forest_view(core.forest()),
        insertion: core.insertion(),
        new_sibling: core.new_sibling(),
    }
}

impl From<Upstream> for TraditionalLayoutSystem {
    fn from(mut upstream: Upstream) -> Self {
        upstream.tiles.normalize_load(SYSTEM);
        let core = core_of(&upstream);
        Self {
            shadow: Box::new(Shadow {
                #[cfg(feature = "roc-core")]
                roc: RocShadow::seeded(&traditional_view(&core)),
                upstream,
                steps: 0,
            }),
            core,
        }
    }
}

impl Default for TraditionalLayoutSystem {
    fn default() -> Self { Upstream::default().into() }
}

impl TraditionalLayoutSystem {
    pub fn new(window_insertion_point: WindowInsertionPoint, equalize_nodes: bool) -> Self {
        Upstream::new(window_insertion_point, equalize_nodes).into()
    }

    pub fn set_window_insertion_point(&mut self, value: WindowInsertionPoint) {
        self.shadow.upstream.set_window_insertion_point(value);
        self.core.set_window_insertion_point(insertion_point(value));
        self.stepped("set_window_insertion_point", &(), &());
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "set_window_insertion_point",
            E::Settings(self.core.insertion(), self.core.new_sibling()),
            Answer::Done,
        );
    }

    pub fn set_equalize_nodes(&mut self, value: bool) {
        self.shadow.upstream.set_equalize_nodes(value);
        self.core.set_new_sibling(new_sibling(value));
        self.stepped("set_equalize_nodes", &(), &());
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "set_equalize_nodes",
            E::Settings(self.core.insertion(), self.core.new_sibling()),
            Answer::Done,
        );
    }

    /// A layout pass with its group bars, the core's (upstream's projection held to them).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn project_groups(
        &self,
        layout: LayoutId,
        screen: CGRect,
        offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &GapSettings,
        line: StackLineReserve,
        emit_frames: bool,
        selection_path_only: bool,
    ) -> (Vec<(WindowId, CGRect)>, Vec<GroupContainerInfo>) {
        project_groups(
            SYSTEM,
            self.core.forest(),
            (&self.shadow.upstream.tiles, None),
            layout,
            screen,
            offset,
            constraints,
            gaps,
            line,
            emit_frames,
            selection_path_only,
        )
    }

    /// rift-cli's drawing of the layout: the core's (upstream's held to it on every call).
    pub(crate) fn draw_tree(&self, layout: LayoutId) -> String {
        let core = self.core.forest().draw(layout);
        self.viewed(
            "draw_tree",
            self.shadow.upstream.tiles.draw_tree(layout, &[]),
            core,
        )
    }

    /// rift-cli's container view of the layout: the core's (upstream's held to it on every
    /// call).
    pub(crate) fn container_tree(&self, layout: LayoutId) -> rift_protocol::ContainerTreeNode {
        let core = container_node(self.core.forest().container_view(layout));
        self.viewed(
            "container_tree",
            self.shadow.upstream.tiles.container_tree(layout),
            core,
        )
    }

    pub(crate) fn window_slot(&self, layout: LayoutId, window: WindowId) -> Option<Vec<usize>> {
        let core = self.answered(
            "window_slot",
            self.shadow.upstream.tiles.window_slot(layout, window),
            self.core.forest().window_slot(layout, window),
        );
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "window_slot",
            E::Forest(F::WindowSlot(roc::layout_key(layout), roc::window(window))),
            Answer::WindowSlot(core.clone()),
        );
        core
    }

    pub(in crate::layout_engine) fn can_move_window(
        &self,
        layout: LayoutId,
        window: WindowId,
        direction: Direction,
    ) -> bool {
        let core = self.answered(
            "can_move_window",
            self.shadow.upstream.can_move_window(layout, window, direction),
            self.core.can_move_window(layout, window, core_direction(direction)),
        );
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "can_move_window",
            E::CanMoveWindow(
                roc::layout_key(layout),
                roc::window(window),
                core_direction(direction),
            ),
            Answer::Verdict(core),
        );
        core
    }

    /// Upstream's system (and under `roc-core` the Roc forest) re-seeded from the core, as
    /// after a divergence.
    #[cfg(test)]
    fn reseed_shadow(&mut self) {
        self.shadow.upstream = upstream_of(&self.core);
        #[cfg(feature = "roc-core")]
        {
            self.shadow.roc = RocShadow::seeded(&traditional_view(&self.core));
        }
    }

    /// Upstream's system saved as upstream saves it (the core's save is held to it).
    #[cfg(test)]
    fn upstream_saved(&self) -> String {
        ron::to_string(&self.shadow.upstream).expect("upstream's system is written")
    }

    /// Under `roc-core`, an operation the core decided (answering `rust`): the Roc forest takes
    /// it too, and is held to the core's answer and whole system.
    #[cfg(feature = "roc-core")]
    fn roc_stepped(&mut self, what: &str, event: E, rust: Answer) {
        let Self { core, shadow } = self;
        shadow.roc.stepped(what, event, rust, || traditional_view(core));
    }

    /// Under `roc-core`, a query the core answered (`rust`): the Roc forest's answer is held to
    /// it.
    #[cfg(feature = "roc-core")]
    fn roc_asked(&self, what: &str, event: E, rust: Answer) {
        self.shadow.roc.asked(what, event, rust);
    }

    /// After an operation both took: their answers compared, and the forests when due; a
    /// difference is reported and the shadow re-seeded from the core (the deciding side
    /// stands).
    fn stepped<A: PartialEq + std::fmt::Debug>(&mut self, what: &str, upstream: &A, core: &A) {
        self.shadow.steps += 1;
        let answers_differ = upstream != core;
        if answers_differ {
            report(
                SYSTEM,
                what,
                &format!("answers: upstream {upstream:?}, core {core:?}"),
            );
        }
        let view_due = VIEW_ALWAYS || self.shadow.steps.is_multiple_of(VIEW_EVERY);
        let view = (answers_differ || view_due).then(|| {
            self.settings_difference()
                .or_else(|| view_difference(self.core.forest(), &self.shadow.upstream.tiles))
        });
        if let Some(Some(difference)) = &view {
            report(SYSTEM, what, difference);
        }
        if answers_differ || matches!(view, Some(Some(_))) {
            self.shadow.upstream = upstream_of(&self.core);
        }
    }

    fn settings_difference(&self) -> Option<String> {
        let upstream = (
            insertion_point(self.shadow.upstream.window_insertion_point),
            new_sibling(self.shadow.upstream.equalize_nodes),
        );
        let core = (self.core.insertion(), self.core.new_sibling());
        (upstream != core).then(|| format!("settings: upstream {upstream:?}, core {core:?}"))
    }

    /// A view both answered, the core's given: a difference is reported (the next step's view
    /// re-seeds the shadow).
    fn viewed<A: PartialEq + std::fmt::Debug>(&self, what: &str, upstream: A, core: A) -> A {
        if upstream != core {
            report(
                SYSTEM,
                what,
                &format!("views: upstream {upstream:?}, core {core:?}"),
            );
        }
        core
    }

    /// A query both answered, the core's answer given: a difference is reported (the next
    /// step's view re-seeds the shadow).
    fn answered<A: PartialEq + std::fmt::Debug>(&self, what: &str, upstream: A, core: A) -> A {
        if upstream != core {
            report(
                SYSTEM,
                what,
                &format!("answers: upstream {upstream:?}, core {core:?}"),
            );
        }
        core
    }
}

impl LayoutSystem for TraditionalLayoutSystem {
    fn has_windows_for_app(&self, layout: LayoutId, pid: pid_t) -> bool {
        let core = self.answered(
            "has_windows_for_app",
            self.shadow.upstream.has_windows_for_app(layout, pid),
            self.core.forest().has_windows_for_app(layout, |window| window.pid == pid),
        );
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "has_windows_for_app",
            E::Forest(F::HasWindowsForApp(
                roc::layout_key(layout),
                roc::app_windows(self.core.forest(), |w| w.pid == pid),
            )),
            Answer::Verdict(core),
        );
        core
    }

    fn window_participates(&self, window: WindowId) -> bool {
        let core = self.answered(
            "window_participates",
            self.shadow.upstream.window_participates(window),
            self.core.forest().window_participates(window),
        );
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "window_participates",
            E::Forest(F::WindowParticipates(roc::window(window))),
            Answer::Verdict(core),
        );
        core
    }

    fn set_window_participation(&mut self, window: WindowId, participates: bool) -> bool {
        let upstream = self.shadow.upstream.set_window_participation(window, participates);
        let core = self.core.set_window_participation(window, participation(participates));
        self.stepped("set_window_participation", &upstream, &core);
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "set_window_participation",
            E::Forest(F::Participation(
                roc::window(window),
                participation(participates),
            )),
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
            E::CreateLayout,
            Answer::Layout(roc::layout_key(core)),
        );
        core
    }

    fn contains_layout(&self, layout: LayoutId) -> bool {
        let core = self.answered(
            "contains_layout",
            self.shadow.upstream.contains_layout(layout),
            self.core.forest().contains_layout(layout),
        );
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "contains_layout",
            E::Forest(F::ContainsLayout(roc::layout_key(layout))),
            Answer::Verdict(core),
        );
        core
    }

    fn clone_layout(&mut self, layout: LayoutId) -> LayoutId {
        let upstream = self.shadow.upstream.clone_layout(layout);
        let core = self.core.clone_layout(layout);
        self.stepped("clone_layout", &upstream, &core);
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "clone_layout",
            E::Forest(F::CloneLayout(roc::layout_key(layout))),
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
            E::Forest(F::RemoveLayout(roc::layout_key(layout))),
            Answer::Done,
        );
    }

    /// The core's frames for the core's shape (under `roc-core`, the Roc geometry's), upstream's
    /// projection the frames' shadow; upstream's shape is held to the core's on every pass.
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
        let tiles = &self.shadow.upstream.tiles;
        let upstream = crate::layout_engine::systems::tiles::shape(
            tiles,
            tiles.root(layout),
            None,
            screen,
            offset,
            constraints,
            gaps,
            line,
        );
        let core = self.core.forest().shape(layout, pass(screen, gaps, offset, line), &|w| {
            constraints
                .get(&w)
                .copied()
                .map(crate::layout_engine::core_geometry::core_constraints)
        });
        if upstream != core {
            report(
                SYSTEM,
                "calculate_layout",
                &format!("shapes: upstream {upstream:?}, core {core:?}"),
            );
        }
        crate::layout_engine::core_geometry::decide_traditional(SYSTEM, core, shadow, || {
            self.shadow
                .upstream
                .shadow_frames(layout, screen, offset, constraints, gaps, line)
        })
    }

    fn selected_window(&self, layout: LayoutId) -> Option<WindowId> {
        let core = self.answered(
            "selected_window",
            self.shadow.upstream.selected_window(layout),
            self.core.forest().selected_window(layout),
        );
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "selected_window",
            E::Forest(F::SelectedWindow(roc::layout_key(layout))),
            Answer::Window(core.map(roc::window)),
        );
        core
    }

    fn selection_contains_window(&self, layout: LayoutId, window: WindowId) -> bool {
        let core = self.answered(
            "selection_contains_window",
            self.shadow.upstream.selection_contains_window(layout, window),
            self.core.forest().selection_contains_window(layout, window),
        );
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "selection_contains_window",
            E::Forest(F::SelectionContainsWindow(
                roc::layout_key(layout),
                roc::window(window),
            )),
            Answer::Verdict(core),
        );
        core
    }

    fn window_in_direction(&self, layout: LayoutId, direction: Direction) -> Option<WindowId> {
        let core = self.answered(
            "window_in_direction",
            self.shadow.upstream.window_in_direction(layout, direction),
            self.core.forest().window_in_direction(layout, core_direction(direction)),
        );
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "window_in_direction",
            E::Forest(F::WindowInDirection(
                roc::layout_key(layout),
                core_direction(direction),
            )),
            Answer::Window(core.map(roc::window)),
        );
        core
    }

    fn contains_window(&self, layout: LayoutId, window: WindowId) -> bool {
        let core = self.answered(
            "contains_window",
            self.shadow.upstream.contains_window(layout, window),
            self.core.forest().contains_window(layout, window),
        );
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "contains_window",
            E::Forest(F::ContainsWindow(roc::layout_key(layout), roc::window(window))),
            Answer::Verdict(core),
        );
        core
    }

    fn has_any_fullscreen_node(&self, layout: LayoutId) -> bool {
        let core = self.answered(
            "has_any_fullscreen_node",
            self.shadow.upstream.has_any_fullscreen_node(layout),
            self.core.forest().has_any_fullscreen_node(layout),
        );
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "has_any_fullscreen_node",
            E::Forest(F::HasAnyFullscreenNode(roc::layout_key(layout))),
            Answer::Verdict(core),
        );
        core
    }

    fn parent_of_selection_is_stacked(&self, layout: LayoutId) -> bool {
        let core = self.answered(
            "parent_of_selection_is_stacked",
            self.shadow.upstream.parent_of_selection_is_stacked(layout),
            self.core.forest().parent_of_selection_is_stacked(layout),
        );
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "parent_of_selection_is_stacked",
            E::Forest(F::ParentOfSelectionIsStacked(roc::layout_key(layout))),
            Answer::Verdict(core),
        );
        core
    }

    fn windows(&self, layout: LayoutId, query: WindowQuery) -> Vec<WindowId> {
        let core = self.answered(
            "windows",
            self.shadow.upstream.windows(layout, query),
            self.core.forest().windows(layout, core_query(query)),
        );
        #[cfg(feature = "roc-core")]
        self.roc_asked(
            "windows",
            E::Forest(F::Windows(
                roc::layout_key(layout),
                roc::query(core_query(query)),
            )),
            Answer::Windows(roc::windows(&core)),
        );
        core
    }

    fn focus(&mut self, layout: LayoutId, focus: Focus) -> FocusResult {
        let upstream = self.shadow.upstream.focus(layout, focus);
        let focused = self.core.focus(layout, core_focus(focus));
        #[cfg(feature = "roc-core")]
        let answer = roc::focused(&focused);
        let core = focus_result(focused);
        self.stepped("focus", &focus_words(&upstream), &focus_words(&core));
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "focus",
            E::Forest(F::Focus(roc::layout_key(layout), roc::focus(core_focus(focus)))),
            answer,
        );
        core
    }

    fn update(&mut self, change: WindowChange<'_>) {
        #[cfg(feature = "roc-core")]
        let event = match change {
            WindowChange::Add(layout, window, toward) => E::AddWindow(
                roc::layout_key(layout),
                roc::window(window),
                toward.map(core_direction),
            ),
            WindowChange::Remove(window) => E::RemoveWindow(roc::window(window)),
            WindowChange::RemoveAndRebalance(window) => E::RemoveAndRebalance(roc::window(window)),
            WindowChange::Replace(from, to) => E::Forest(F::ReplaceWindow {
                from: roc::window(from),
                to: roc::window(to),
            }),
            WindowChange::CloseApp(pid) => {
                E::CloseApp(roc::app_windows(self.core.forest(), |w| w.pid == pid))
            }
            WindowChange::Resize {
                layout,
                window,
                old,
                new,
                screen,
                gaps,
            } => E::Forest(F::ResizeWindow(
                roc::layout_key(layout),
                roc::window(window),
                resize(old, new, screen, gaps),
            )),
        };
        let core = &mut self.core;
        match change {
            WindowChange::Add(layout, window, toward) => {
                core.add_window(layout, window, toward.map(core_direction))
            }
            WindowChange::Remove(window) => core.remove_window(window),
            WindowChange::RemoveAndRebalance(window) => core.remove_and_rebalance(window),
            WindowChange::Replace(from, to) => core.replace_window(from, to),
            WindowChange::CloseApp(pid) => core.close_app(|window| window.pid == pid),
            WindowChange::Resize {
                layout,
                window,
                old,
                new,
                screen,
                gaps,
            } => core.resize_window(layout, window, resize(old, new, screen, gaps)),
        }
        self.shadow.upstream.update(change);
        self.stepped("update", &(), &());
        #[cfg(feature = "roc-core")]
        self.roc_stepped("update", event, Answer::Done);
    }

    fn edit_layout(&mut self, layout: LayoutId, edit: LayoutEdit) -> EditResult {
        let upstream = self.shadow.upstream.edit_layout(layout, edit);
        let edit = core_edit(edit);
        let edited = edit.map(|edit| self.core.edit(layout, edit));
        let core = edited.clone().map_or(EditResult::Unchanged, edit_result);
        self.stepped("edit_layout", &edit_words(&upstream), &edit_words(&core));
        #[cfg(feature = "roc-core")]
        if let (Some(edit), Some(edited)) = (edit, &edited) {
            self.roc_stepped(
                "edit_layout",
                E::Edit(roc::layout_key(layout), roc::edit(edit)),
                roc::edited(edited),
            );
        }
        core
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout_engine::systems::drawn::{Rng, drawn_direction};
    use crate::layout_engine::systems::group_twin::drawn;

    fn w(idx: u32) -> WindowId { WindowId::new(1, idx) }

    fn drawn_settings(rng: &mut Rng) -> (WindowInsertionPoint, bool) {
        let insertion = if rng.chance(50) {
            WindowInsertionPoint::NextToSelection
        } else {
            WindowInsertionPoint::EndOfTree
        };
        (insertion, rng.chance(50))
    }

    /// A saved system read back, its settings (not saved) configured again.
    fn reloaded(rng: &mut Rng, system: &TraditionalLayoutSystem) -> TraditionalLayoutSystem {
        let saved = ron::to_string(system).expect("written");
        assert_eq!(
            saved,
            system.upstream_saved(),
            "the core saves as upstream's system"
        );
        let text = drawn::with_saved_only_states(rng, &saved);
        let mut loaded: TraditionalLayoutSystem = ron::from_str(&text).expect("and read back");
        assert_eq!(
            ron::to_string(&loaded).unwrap(),
            text,
            "a reload saves the same bytes"
        );
        loaded.set_window_insertion_point(system.shadow.upstream.window_insertion_point);
        loaded.set_equalize_nodes(system.shadow.upstream.equalize_nodes);
        loaded
    }

    /// Drawn traditional systems: every operation and query held to upstream's by the twin.
    #[test]
    fn drawn_traditional_systems_step_the_core_as_upstream() {
        let mut rng = Rng(0x7ad1_7104);
        let (mut windows, mut layouts) = (0, 0);
        for _ in 0..drawn::cases(3000) {
            let (insertion, equalize) = drawn_settings(&mut rng);
            let mut system = TraditionalLayoutSystem::new(insertion, equalize);
            let steps = rng.below(120);
            let drawn = drawn::run(
                &mut rng,
                &mut system,
                steps,
                &mut |rng, system, drawn| {
                    if rng.chance(10) {
                        system.reseed_shadow();
                    }
                    if rng.chance(30) {
                        let (insertion, equalize) = drawn_settings(rng);
                        system.set_window_insertion_point(insertion);
                        system.set_equalize_nodes(equalize);
                    }
                    let (layout, window) = (drawn.layout(rng), drawn.window(rng));
                    system.can_move_window(layout, window, drawn_direction(rng));
                    system.window_slot(layout, window);
                },
                &reloaded,
                &|rng, system, layout, windows| {
                    system.draw_tree(layout);
                    system.container_tree(layout);
                    let (screen, constraints, gaps, offset, line, frames, path) =
                        drawn::bar_pass(rng, windows);
                    system.project_groups(
                        layout,
                        screen,
                        offset,
                        &constraints,
                        &gaps,
                        line,
                        frames,
                        path,
                    );
                },
            );
            windows += drawn.windows.len();
            layouts += drawn.layouts.len();
        }
        eprintln!("traditional twin: {layouts} layouts, {windows} windows at the end");
    }

    /// The window index is ordered by when each leaf entered its layout, not by layout: a clone
    /// that takes a lower freed layout slot lists its leaf after the source's, so removing the
    /// window frees the clone's leaf last and the next leaf takes its slot (the lead's ruling:
    /// the order decides later ids, which match upstream's).
    #[test]
    fn a_clone_in_a_lower_freed_slot_is_indexed_after_its_source() {
        use rift_core::slot_table::SlotKey;
        let mut system = TraditionalLayoutSystem::default();
        let first = system.create_layout();
        let source = system.create_layout();
        system.update(WindowChange::Add(source, w(1), None));
        system.remove_layout(first);
        let clone = system.clone_layout(source);
        assert!(
            clone.parts().0 < source.parts().0,
            "the clone takes the lower freed slot"
        );
        let index = system.core.forest().index();
        let layouts: Vec<_> = index.iter().map(|&(window, layout, _)| (window, layout)).collect();
        assert_eq!(layouts, vec![(w(1), source), (w(1), clone)]);
        let clone_leaf = index[1].2;
        system.update(WindowChange::Remove(w(1)));
        system.update(WindowChange::Add(source, w(2), None));
        let index = system.core.forest().index();
        assert_eq!(index.len(), 1);
        assert_eq!(
            index[0].2.parts().0,
            clone_leaf.parts().0,
            "the clone's leaf, freed last"
        );
    }

    /// `file` with `from` (which it holds once) written as `to`.
    fn written_as(file: &str, from: &str, to: &str) -> String {
        assert_eq!(file.matches(from).count(), 1, "{from} in {file}");
        file.replace(from, to)
    }

    /// `corrupt` loads (its twin agreeing with upstream's system) and saves as `normalized`.
    fn loads_normalized(corrupt: &str, normalized: &str) {
        assert_ne!(corrupt, normalized);
        let mut loaded: TraditionalLayoutSystem = ron::from_str(corrupt).unwrap();
        assert_eq!(ron::to_string(&loaded).unwrap(), normalized);
        loaded.create_layout();
    }

    /// Two windows side by side in one layout, the second selected.
    fn two_windows() -> (TraditionalLayoutSystem, LayoutId) {
        let mut system = TraditionalLayoutSystem::default();
        let layout = system.create_layout();
        system.update(WindowChange::Add(layout, w(1), None));
        system.update(WindowChange::Add(layout, w(2), None));
        (system, layout)
    }

    #[test]
    fn a_saved_node_with_both_fullscreen_flags_loads_fullscreen() {
        let (mut system, layout) = two_windows();
        system
            .edit_layout(layout, LayoutEdit::Fullscreen { within_gaps: false })
            .discard();
        let normalized = ron::to_string(&system).unwrap();
        let corrupt = written_as(
            &normalized,
            "is_fullscreen:true,is_fullscreen_within_gaps:false",
            "is_fullscreen:true,is_fullscreen_within_gaps:true",
        );
        loads_normalized(&corrupt, &normalized);
    }

    #[test]
    fn a_saved_window_kind_loads_as_the_default() {
        let (system, _) = two_windows();
        let normalized = ron::to_string(&system).unwrap();
        let corrupt = written_as(
            &normalized,
            "kind:horizontal,is_fullscreen:false,is_fullscreen_within_gaps:false),selection:None,\
             window:Some((pid:1,idx:1))",
            "kind:vertical_stack,is_fullscreen:false,is_fullscreen_within_gaps:false),\
             selection:None,window:Some((pid:1,idx:1))",
        );
        loads_normalized(&corrupt, &normalized);
    }

    #[test]
    fn a_saved_root_size_loads_as_zero() {
        let (system, _) = two_windows();
        let normalized = ron::to_string(&system).unwrap();
        for size in ["2.5", "-0.0", "NaN"] {
            let corrupt = written_as(&normalized, "size:0.0,", &format!("size:{size},"));
            loads_normalized(&corrupt, &normalized);
        }
    }

    /// Entries naming no leaf of theirs (another window's leaf, a missing layout), and a leaf
    /// listed twice, are dropped; a leaf the index misses is appended.
    #[test]
    fn a_saved_window_index_is_normalized_to_the_tree() {
        let (system, _) = two_windows();
        let normalized = ron::to_string(&system).unwrap();
        let first = "(pid:1,idx:1):([(layout:(idx:1,version:1),node:(idx:2,version:1))])";
        let second = "(pid:1,idx:2):([(layout:(idx:1,version:1),node:(idx:3,version:1))])";
        let entry = |layout: u32, node: u32| {
            format!("(layout:(idx:{layout},version:1),node:(idx:{node},version:1))")
        };
        for (from, to) in [
            (
                first,
                format!("(pid:1,idx:1):([{},{}])", entry(1, 2), entry(1, 3)),
            ),
            (
                first,
                format!("(pid:1,idx:1):([{},{}])", entry(1, 2), entry(7, 2)),
            ),
            (
                first,
                format!("(pid:1,idx:1):([{},{}])", entry(1, 2), entry(1, 2)),
            ),
            (second, "(pid:1,idx:2):([])".to_owned()),
            (second, format!("(pid:1,idx:3):([{}])", entry(1, 3))),
        ] {
            loads_normalized(&written_as(&normalized, from, &to), &normalized);
        }
    }

    /// A window's leaves keep the index's saved order (the load stamps them in it): listed
    /// clone first, the source's leaf is freed last and the next leaf takes its slot.
    #[test]
    fn a_saved_window_index_keeps_its_order() {
        use rift_core::slot_table::SlotKey;
        let mut system = TraditionalLayoutSystem::default();
        let source = system.create_layout();
        system.update(WindowChange::Add(source, w(1), None));
        let clone = system.clone_layout(source);
        let file = ron::to_string(&system).unwrap();
        let index = system.core.forest().index();
        let (source_leaf, clone_leaf) = (index[0].2, index[1].2);
        let entry = |layout: LayoutId, node: NodeId| {
            let ((l, lv), (n, nv)) = (layout.parts(), node.parts());
            format!("(layout:(idx:{l},version:{lv}),node:(idx:{n},version:{nv}))")
        };
        let reordered = written_as(
            &file,
            &format!("[{},{}]", entry(source, source_leaf), entry(clone, clone_leaf)),
            &format!("[{},{}]", entry(clone, clone_leaf), entry(source, source_leaf)),
        );
        let mut system: TraditionalLayoutSystem = ron::from_str(&reordered).unwrap();
        assert_eq!(ron::to_string(&system).unwrap(), reordered);
        system.update(WindowChange::Remove(w(1)));
        system.update(WindowChange::Add(clone, w(2), None));
        let index = system.core.forest().index();
        assert_eq!(
            index[0].2.parts().0,
            source_leaf.parts().0,
            "the source's leaf, freed last"
        );
    }

    /// A query upstream's system answers otherwise than the core is counted and fails a test.
    #[test]
    fn a_query_answered_otherwise_is_counted_and_fails_tests() {
        let mut system = TraditionalLayoutSystem::default();
        system.create_layout();
        system.shadow.upstream.set_window_participation(w(1), false);
        let before = crate::layout_engine::systems::group_forest_divergences();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            system.window_participates(w(1));
        }));
        assert!(failed.is_err());
        assert!(crate::layout_engine::systems::group_forest_divergences() > before);
    }

    /// Upstream's system differing from the core is counted and fails a test (a running Rift
    /// warns and re-seeds it from the core).
    #[test]
    fn a_group_forest_divergence_is_counted_and_fails_tests() {
        let mut system = TraditionalLayoutSystem::default();
        let layout = system.create_layout();
        system.shadow.upstream.set_window_participation(w(1), false);
        let before = crate::layout_engine::systems::group_forest_divergences();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            system.clone_layout(layout);
        }));
        assert!(failed.is_err());
        assert!(crate::layout_engine::systems::group_forest_divergences() > before);
    }
    /// The twin's cost per operation at 10, 100 and 1000 windows (release:
    /// `cargo test --release --lib measure_traditional_twin -- --ignored --nocapture`): upstream's
    /// operation alone, the core's alone, the view comparison, a re-seed, and a pass's shape built
    /// by each. A plain release build pays the core's step on every operation and the comparison
    /// on every 32nd.
    #[test]
    #[ignore]
    fn measure_traditional_twin() {
        use std::hint::black_box;
        use std::time::Instant;

        use rift_core::layout::group_forest::{Edit, Focus as CoreFocus};

        use crate::layout_engine::core_geometry::core_constraints;
        let directions = [
            Direction::Left,
            Direction::Up,
            Direction::Right,
            Direction::Down,
        ];
        let ops = 4000_u32;
        let each = |started: Instant, n: u32| started.elapsed() / n;
        for count in [10_u32, 100, 1000] {
            let mut system = TraditionalLayoutSystem::default();
            let layout = system.create_layout();
            for idx in 1..=count {
                system.update(WindowChange::Add(layout, WindowId::new(1, idx), None));
            }
            let w = |i: u32| WindowId::new(1, 1 + i % count);
            // Everyday operations: focus moves, swaps, fullscreen, a window coming and going.
            let mut upstream = system.shadow.upstream.clone();
            let started = Instant::now();
            for i in 0..ops {
                match i % 4 {
                    0 => drop(black_box(
                        upstream.focus(layout, Focus::Direction(directions[(i / 4 % 4) as usize])),
                    )),
                    1 => drop(black_box(
                        upstream.edit_layout(layout, LayoutEdit::Swap(w(i), w(i * 7))),
                    )),
                    2 => {
                        drop(black_box(upstream.edit_layout(layout, LayoutEdit::Fullscreen {
                            within_gaps: false,
                        })))
                    }
                    _ => {
                        upstream.update(WindowChange::Add(layout, WindowId::new(2, 1), None));
                        upstream.update(WindowChange::Remove(WindowId::new(2, 1)));
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
                        CoreFocus::Direction(core_direction(directions[(i / 4 % 4) as usize])),
                    ))),
                    1 => drop(black_box(core.edit(layout, Edit::Swap(w(i), w(i * 7))))),
                    2 => drop(black_box(
                        core.edit(layout, Edit::Fullscreen { within_gaps: false }),
                    )),
                    _ => {
                        core.add_window(layout, WindowId::new(2, 1), None);
                        core.remove_window(WindowId::new(2, 1));
                    }
                }
            }
            let core_each = each(started, ops);
            let rounds = 200;
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(view_difference(
                    system.core.forest(),
                    &system.shadow.upstream.tiles,
                ));
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
            let (gaps, line) = (GapSettings::default(), StackLineReserve::default());
            let constraints: HashMap<WindowId, WindowLayoutConstraints> = HashMap::default();
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(crate::layout_engine::systems::tiles::shape(
                    &system.shadow.upstream.tiles,
                    system.shadow.upstream.tiles.root(layout),
                    None,
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
                black_box(system.core.forest().shape(
                    layout,
                    pass(screen, &gaps, 12.0, line),
                    &|window| constraints.get(&window).copied().map(core_constraints),
                ));
            }
            let core_shape = each(started, rounds);
            let per_op = view_each / VIEW_EVERY as u32;
            eprintln!(
                "traditional {count} windows: upstream {upstream_each:?}/op, core {core_each:?}/op, \
                 view {view_each:?} ({per_op:?}/op 1 in {VIEW_EVERY}), re-seed {seed_each:?}, \
                 shape upstream {upstream_shape:?} core {core_shape:?}"
            );
        }
    }

    /// A Roc forest that differs from the core is counted and fails a test (a running Rift warns
    /// and re-seeds it from the core).
    #[cfg(feature = "roc-core")]
    #[test]
    fn a_roc_group_forest_divergence_is_counted_and_fails_tests() {
        let mut system = TraditionalLayoutSystem::default();
        let layout = system.create_layout();
        system.shadow.roc.machine().step(E::CreateLayout);
        let before = crate::layout_engine::systems::group_forest_reseeds();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            system.update(WindowChange::Add(layout, w(1), None));
        }));
        assert!(failed.is_err());
        assert!(crate::layout_engine::systems::group_forest_reseeds() > before);
    }

    /// The Roc shadow's cost at 10, 100 and 1000 windows (release, roc-core: `cargo test
    /// --release --lib --features roc-core measure_traditional_roc_shadow -- --ignored
    /// --nocapture`): the Roc step, the view comparison, a re-seed.
    #[cfg(feature = "roc-core")]
    #[test]
    #[ignore]
    fn measure_traditional_roc_shadow() {
        use std::hint::black_box;
        use std::time::Instant;

        use rift_core::layout::group_forest::{
            Direction as CoreDirection, Edit, Focus as CoreFocus,
        };
        for count in [10_u32, 100, 1000] {
            let mut system = TraditionalLayoutSystem::default();
            let layout = system.create_layout();
            for idx in 1..=count {
                system.update(WindowChange::Add(layout, w(idx), None));
            }
            let core = system.core.clone();
            let held = traditional_view(&core);
            let mut shadow = RocShadow::<rift_core_roc::RocTraditional>::seeded(&held);
            let key = roc::layout_key(layout);
            let ops = 2000_u32;
            let started = Instant::now();
            for i in 0..ops {
                let event = match i % 3 {
                    0 => E::Forest(F::Focus(
                        key,
                        CoreFocus::Direction(
                            [CoreDirection::Left, CoreDirection::Right][(i / 3 % 2) as usize],
                        ),
                    )),
                    1 => E::Edit(
                        key,
                        Edit::Swap(
                            roc::window(w(1 + i % count)),
                            roc::window(w(1 + (i * 7) % count)),
                        ),
                    ),
                    _ => E::Forest(F::SelectedWindow(key)),
                };
                black_box(shadow.machine().step(event));
            }
            let step_each = started.elapsed() / ops;
            let rounds = 100;
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(shadow.machine().view() == traditional_view(&core));
            }
            let view_each = started.elapsed() / rounds;
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(RocShadow::<rift_core_roc::RocTraditional>::seeded(&held));
            }
            let seed_each = started.elapsed() / rounds;
            eprintln!(
                "traditional roc shadow {count} windows: step {step_each:?}/op, view comparison \
                 {view_each:?}, re-seed {seed_each:?}"
            );
        }
    }
}
