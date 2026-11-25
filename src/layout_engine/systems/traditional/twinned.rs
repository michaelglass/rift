//! The traditional layout system with the core's forest beside it (layout trees, the group
//! forest's slice 1): upstream's system ([`Upstream`]) decides, the core's [`Traditional`] is
//! stepped on every operation and held to it (`group_twin` says how).

use objc2_core_foundation::CGRect;
use rift_core::layout::group_forest::{NewSibling, Traditional};
use serde::{Deserialize, Serialize};

use super::Upstream;
use crate::actor::app::{WindowId, pid_t};
use crate::common::collections::HashMap;
use crate::common::config::{GapSettings, StackLineReserve, WindowInsertionPoint};
use crate::layout_engine::systems::group_twin::{
    VIEW_ALWAYS, VIEW_EVERY, core_edit, core_focus, core_query, edit_result, edit_words,
    focus_result, focus_words, participation, pass, report, resize, seed, view_difference,
};
use crate::layout_engine::systems::groups::GroupLayouts;
use crate::layout_engine::systems::{
    EditResult, Focus, FocusResult, LayoutEdit, LayoutSystem, WindowChange,
    WindowLayoutConstraints, WindowQuery, core_direction, insertion_point,
};
use crate::layout_engine::{Direction, LayoutId};
use crate::model::tree::NodeId;

type CoreTraditional = Traditional<LayoutId, NodeId, WindowId>;

const SYSTEM: &str = "traditional";

/// The traditional layout system: upstream's decides, and the core's is stepped beside it.
#[derive(Clone, Debug, Deserialize)]
#[serde(from = "Upstream")]
pub struct TraditionalLayoutSystem {
    upstream: Upstream,
    /// Boxed: the system is one variant of `LayoutSystemKind`.
    twin: Box<Twinned>,
}

/// The core's system and how many operations stepped it.
#[derive(Clone, Debug)]
struct Twinned {
    core: CoreTraditional,
    steps: u64,
}

impl Serialize for TraditionalLayoutSystem {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.upstream.serialize(serializer)
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

impl From<Upstream> for TraditionalLayoutSystem {
    fn from(upstream: Upstream) -> Self {
        let twin = Box::new(Twinned {
            core: core_of(&upstream),
            steps: 0,
        });
        Self { upstream, twin }
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
        self.upstream.set_window_insertion_point(value);
        self.twin.core.set_window_insertion_point(insertion_point(value));
        self.stepped("set_window_insertion_point", &(), &());
    }

    pub fn set_equalize_nodes(&mut self, value: bool) {
        self.upstream.set_equalize_nodes(value);
        self.twin.core.set_new_sibling(new_sibling(value));
        self.stepped("set_equalize_nodes", &(), &());
    }

    /// Upstream's forest (the group bar's projection and rift-cli's views read it).
    pub(in crate::layout_engine::systems) fn tiles(&self) -> &GroupLayouts { &self.upstream.tiles }

    pub(crate) fn window_slot(&self, layout: LayoutId, window: WindowId) -> Option<Vec<usize>> {
        self.answered(
            "window_slot",
            self.upstream.tiles.window_slot(layout, window),
            self.twin.core.forest().window_slot(layout, window),
        )
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

    /// After an operation both took: their answers compared, and the forests when due; a
    /// difference is reported and the core re-seeded.
    fn stepped<A: PartialEq + std::fmt::Debug>(&mut self, what: &str, upstream: &A, core: &A) {
        self.twin.steps += 1;
        let answers_differ = upstream != core;
        if answers_differ {
            report(
                SYSTEM,
                what,
                &format!("answers: upstream {upstream:?}, core {core:?}"),
            );
        }
        let view_due = VIEW_ALWAYS || self.twin.steps.is_multiple_of(VIEW_EVERY);
        let view = (answers_differ || view_due).then(|| {
            self.settings_difference()
                .or_else(|| view_difference(self.twin.core.forest(), &self.upstream.tiles))
        });
        if let Some(Some(difference)) = &view {
            report(SYSTEM, what, difference);
        }
        if answers_differ || matches!(view, Some(Some(_))) {
            self.twin.core = core_of(&self.upstream);
        }
    }

    fn settings_difference(&self) -> Option<String> {
        let upstream = (
            insertion_point(self.upstream.window_insertion_point),
            new_sibling(self.upstream.equalize_nodes),
        );
        let core = (self.twin.core.insertion(), self.twin.core.new_sibling());
        (upstream != core).then(|| format!("settings: upstream {upstream:?}, core {core:?}"))
    }

    /// A query both answered: a difference is reported (the next step's view re-seeds the
    /// core).
    fn answered<A: PartialEq + std::fmt::Debug>(&self, what: &str, upstream: A, core: A) -> A {
        if upstream != core {
            report(
                SYSTEM,
                what,
                &format!("answers: upstream {upstream:?}, core {core:?}"),
            );
        }
        upstream
    }
}

impl LayoutSystem for TraditionalLayoutSystem {
    fn has_windows_for_app(&self, layout: LayoutId, pid: pid_t) -> bool {
        self.answered(
            "has_windows_for_app",
            self.upstream.has_windows_for_app(layout, pid),
            self.twin.core.forest().has_windows_for_app(layout, |window| window.pid == pid),
        )
    }

    fn window_participates(&self, window: WindowId) -> bool {
        self.answered(
            "window_participates",
            self.upstream.window_participates(window),
            self.twin.core.forest().window_participates(window),
        )
    }

    fn set_window_participation(&mut self, window: WindowId, participates: bool) -> bool {
        let upstream = self.upstream.set_window_participation(window, participates);
        let core = self.twin.core.set_window_participation(window, participation(participates));
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
            self.twin.core.forest().contains_layout(layout),
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

    /// Upstream's frames (the core's geometry decides them, policy (a)); the core's shape is
    /// held to upstream's on every pass.
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
        let tiles = &self.upstream.tiles;
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
        let core = self.twin.core.forest().shape(layout, pass(screen, gaps, offset, line), &|w| {
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
        self.upstream
            .calculate_layout(layout, screen, offset, constraints, gaps, line, shadow)
    }

    fn selected_window(&self, layout: LayoutId) -> Option<WindowId> {
        self.answered(
            "selected_window",
            self.upstream.selected_window(layout),
            self.twin.core.forest().selected_window(layout),
        )
    }

    fn selection_contains_window(&self, layout: LayoutId, window: WindowId) -> bool {
        self.answered(
            "selection_contains_window",
            self.upstream.selection_contains_window(layout, window),
            self.twin.core.forest().selection_contains_window(layout, window),
        )
    }

    fn window_in_direction(&self, layout: LayoutId, direction: Direction) -> Option<WindowId> {
        self.answered(
            "window_in_direction",
            self.upstream.window_in_direction(layout, direction),
            self.twin.core.forest().window_in_direction(layout, core_direction(direction)),
        )
    }

    fn contains_window(&self, layout: LayoutId, window: WindowId) -> bool {
        self.answered(
            "contains_window",
            self.upstream.contains_window(layout, window),
            self.twin.core.forest().contains_window(layout, window),
        )
    }

    fn has_any_fullscreen_node(&self, layout: LayoutId) -> bool {
        self.answered(
            "has_any_fullscreen_node",
            self.upstream.has_any_fullscreen_node(layout),
            self.twin.core.forest().has_any_fullscreen_node(layout),
        )
    }

    fn parent_of_selection_is_stacked(&self, layout: LayoutId) -> bool {
        self.answered(
            "parent_of_selection_is_stacked",
            self.upstream.parent_of_selection_is_stacked(layout),
            self.twin.core.forest().parent_of_selection_is_stacked(layout),
        )
    }

    fn windows(&self, layout: LayoutId, query: WindowQuery) -> Vec<WindowId> {
        self.answered(
            "windows",
            self.upstream.windows(layout, query),
            self.twin.core.forest().windows(layout, core_query(query)),
        )
    }

    fn focus(&mut self, layout: LayoutId, focus: Focus) -> FocusResult {
        let upstream = self.upstream.focus(layout, focus);
        let core = focus_result(self.twin.core.focus(layout, core_focus(focus)));
        self.stepped("focus", &focus_words(&upstream), &focus_words(&core));
        upstream
    }

    fn update(&mut self, change: WindowChange<'_>) {
        let core = &mut self.twin.core;
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
        self.upstream.update(change);
        self.stepped("update", &(), &());
    }

    fn edit_layout(&mut self, layout: LayoutId, edit: LayoutEdit) -> EditResult {
        let upstream = self.upstream.edit_layout(layout, edit);
        let core = match core_edit(edit) {
            Some(edit) => edit_result(self.twin.core.edit(layout, edit)),
            None => EditResult::Unchanged,
        };
        self.stepped("edit_layout", &edit_words(&upstream), &edit_words(&core));
        upstream
    }
}

#[cfg(test)]
mod tests {
    use rift_core::layout::group_forest::Participation;

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
        let text = drawn::with_saved_only_states(rng, &ron::to_string(system).expect("written"));
        let mut loaded: TraditionalLayoutSystem = ron::from_str(&text).expect("and read back");
        assert_eq!(
            ron::to_string(&loaded).unwrap(),
            text,
            "a reload saves the same bytes"
        );
        loaded.set_window_insertion_point(system.upstream.window_insertion_point);
        loaded.set_equalize_nodes(system.upstream.equalize_nodes);
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
        let index = system.twin.core.forest().index();
        let layouts: Vec<_> = index.iter().map(|&(window, layout, _)| (window, layout)).collect();
        assert_eq!(layouts, vec![(w(1), source), (w(1), clone)]);
        let clone_leaf = index[1].2;
        system.update(WindowChange::Remove(w(1)));
        system.update(WindowChange::Add(source, w(2), None));
        let index = system.twin.core.forest().index();
        assert_eq!(index.len(), 1);
        assert_eq!(
            index[0].2.parts().0,
            clone_leaf.parts().0,
            "the clone's leaf, freed last"
        );
    }

    /// A query the core answers otherwise than upstream's system is counted and fails a test.
    #[test]
    fn a_query_answered_otherwise_is_counted_and_fails_tests() {
        let mut system = TraditionalLayoutSystem::default();
        system.create_layout();
        system.twin.core.set_window_participation(w(1), Participation::LeftOut);
        let before = crate::layout_engine::systems::group_forest_divergences();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            system.window_participates(w(1));
        }));
        assert!(failed.is_err());
        assert!(crate::layout_engine::systems::group_forest_divergences() > before);
    }

    /// A core that differs from upstream's system is counted and fails a test (a running Rift
    /// warns and re-seeds it).
    #[test]
    fn a_group_forest_divergence_is_counted_and_fails_tests() {
        let mut system = TraditionalLayoutSystem::default();
        let layout = system.create_layout();
        system.twin.core.set_window_participation(w(1), Participation::LeftOut);
        let before = crate::layout_engine::systems::group_forest_divergences();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            system.clone_layout(layout);
        }));
        assert!(failed.is_err());
        assert!(crate::layout_engine::systems::group_forest_divergences() > before);
    }
}
