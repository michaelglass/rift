//! The master-stack layout system with the core's forest beside it (layout trees, the group
//! forest's slice 1): upstream's system ([`Upstream`]) decides, the core's [`MasterStack`] is
//! stepped on every operation and held to it (`group_twin` says how).

use objc2_core_foundation::CGRect;
use rift_core::layout::group_forest::{
    self as core, Arrangement, MasterSettings, MasterSide, MasterStack, NewWindowPlacement,
    RegionInsertion,
};
use serde::{Deserialize, Serialize};

use super::Upstream;
use crate::actor::app::{WindowId, pid_t};
use crate::common::collections::HashMap;
use crate::common::config::{
    GapSettings, MasterStackNewWindowPlacement, MasterStackSettings, MasterStackSide,
    StackLineReserve, WindowInsertionPoint,
};
use crate::layout_engine::engine::GroupContainerInfo;
use crate::layout_engine::systems::group_twin::{
    VIEW_ALWAYS, VIEW_EVERY, container_node, core_edit, core_focus, core_query, edit_result,
    edit_words, focus_result, focus_words, participation, pass, project_groups, report, resize,
    seed, view_difference,
};
use crate::layout_engine::systems::{
    EditResult, Focus, FocusResult, LayoutEdit, LayoutSystem, WindowChange,
    WindowLayoutConstraints, WindowQuery, core_direction,
};
use crate::layout_engine::{Direction, LayoutId, Orientation};
use crate::model::tree::NodeId;

type CoreMasterStack = MasterStack<LayoutId, NodeId, WindowId>;

const SYSTEM: &str = "master-stack";

/// The master-stack layout system: upstream's decides, and the core's is stepped beside it.
#[derive(Clone, Debug, Deserialize)]
#[serde(from = "Upstream")]
pub struct MasterStackLayoutSystem {
    upstream: Upstream,
    /// Boxed: the system is one variant of `LayoutSystemKind`.
    twin: Box<Twinned>,
}

/// The core's system and how many operations stepped it.
#[derive(Clone, Debug)]
struct Twinned {
    core: CoreMasterStack,
    steps: u64,
}

impl Serialize for MasterStackLayoutSystem {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.upstream.serialize(serializer)
    }
}

fn core_orientation(orientation: Orientation) -> core::Orientation {
    match orientation {
        Orientation::Horizontal => core::Orientation::Horizontal,
        Orientation::Vertical => core::Orientation::Vertical,
    }
}

fn arrangement(arrangement: Option<Orientation>) -> Arrangement {
    arrangement.map_or(Arrangement::Derived, |o| Arrangement::Set(core_orientation(o)))
}

/// Upstream's settings as the core takes them.
fn core_settings(settings: &MasterStackSettings) -> MasterSettings {
    MasterSettings {
        insertion: match settings.base.window_insertion_point {
            Some(WindowInsertionPoint::NextToSelection) => RegionInsertion::NextToSelection,
            Some(WindowInsertionPoint::EndOfTree) => RegionInsertion::EndOfTree,
            None => RegionInsertion::Front,
        },
        ratio: settings.master_ratio,
        count: settings.master_count,
        side: match settings.master_side {
            MasterStackSide::Left => MasterSide::Left,
            MasterStackSide::Right => MasterSide::Right,
            MasterStackSide::Top => MasterSide::Top,
            MasterStackSide::Bottom => MasterSide::Bottom,
        },
        placement: match settings.new_window_placement {
            MasterStackNewWindowPlacement::Master => NewWindowPlacement::Master,
            MasterStackNewWindowPlacement::Stack => NewWindowPlacement::Stack,
            MasterStackNewWindowPlacement::Focused => NewWindowPlacement::Focused,
        },
        master: arrangement(settings.master_arrangement),
        stack: arrangement(settings.stack_arrangement),
    }
}

fn core_of(upstream: &Upstream) -> CoreMasterStack {
    CoreMasterStack::from_parts(seed(&upstream.tiles), core_settings(&upstream.settings))
}

impl From<Upstream> for MasterStackLayoutSystem {
    fn from(mut upstream: Upstream) -> Self {
        upstream.tiles.normalize_load(SYSTEM);
        let twin = Box::new(Twinned {
            core: core_of(&upstream),
            steps: 0,
        });
        Self { upstream, twin }
    }
}

impl Default for MasterStackLayoutSystem {
    fn default() -> Self { Upstream::default().into() }
}

impl MasterStackLayoutSystem {
    pub fn new(settings: MasterStackSettings) -> Self { Upstream::new(settings).into() }

    pub fn update_settings(&mut self, settings: MasterStackSettings) {
        self.twin.core.update_settings(core_settings(&settings));
        self.upstream.update_settings(settings);
        self.stepped("update_settings", &(), &());
    }

    pub fn adjust_master_ratio(&mut self, layout: LayoutId, delta: f64) {
        self.upstream.adjust_master_ratio(layout, delta);
        self.twin.core.adjust_master_ratio(delta);
        self.stepped("adjust_master_ratio", &(), &());
    }

    pub fn adjust_master_count(&mut self, layout: LayoutId, delta: i32) {
        self.upstream.adjust_master_count(layout, delta);
        self.twin.core.adjust_master_count(delta);
        self.stepped("adjust_master_count", &(), &());
    }

    pub fn promote_to_master(&mut self, layout: LayoutId) {
        self.upstream.promote_to_master(layout);
        self.twin.core.promote_to_master(layout);
        self.stepped("promote_to_master", &(), &());
    }

    pub fn swap_master_stack(&mut self, layout: LayoutId) {
        self.upstream.swap_master_stack(layout);
        self.twin.core.swap_master_stack(layout);
        self.stepped("swap_master_stack", &(), &());
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
            self.twin.core.forest(),
            (&self.upstream.tiles, Some(self.upstream.regions(layout))),
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
        let core = self.twin.core.forest().draw(layout);
        self.viewed("draw_tree", self.upstream.draw_tree(layout), core)
    }

    /// rift-cli's container view of the layout: the core's (upstream's held to it on every
    /// call).
    pub(crate) fn container_tree(&self, layout: LayoutId) -> rift_protocol::ContainerTreeNode {
        let core = container_node(self.twin.core.forest().container_view(layout));
        self.viewed("container_tree", self.upstream.container_tree(layout), core)
    }

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
        let upstream = core_settings(&self.upstream.settings);
        let core = self.twin.core.settings();
        (upstream != core).then(|| format!("settings: upstream {upstream:?}, core {core:?}"))
    }

    /// A view both answered, the core's given: a difference is reported (the next step's view
    /// re-seeds the core).
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

impl LayoutSystem for MasterStackLayoutSystem {
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
        let upstream = self.upstream.shape(layout, screen, offset, constraints, gaps, line);
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
    use super::*;
    use crate::layout_engine::systems::drawn::Rng;
    use crate::layout_engine::systems::group_twin::drawn;

    fn drawn_side(rng: &mut Rng) -> MasterStackSide {
        match rng.below(4) {
            0 => MasterStackSide::Left,
            1 => MasterStackSide::Right,
            2 => MasterStackSide::Top,
            _ => MasterStackSide::Bottom,
        }
    }

    fn drawn_arrangement(rng: &mut Rng) -> Option<Orientation> {
        match rng.below(3) {
            0 => None,
            1 => Some(Orientation::Horizontal),
            _ => Some(Orientation::Vertical),
        }
    }

    /// Settings over every field, changed one or a few at a time from `from`.
    fn drawn_settings(rng: &mut Rng, from: &MasterStackSettings) -> MasterStackSettings {
        let mut settings = from.clone();
        for _ in 0..1 + rng.below(3) {
            match rng.below(7) {
                0 => {
                    settings.base.window_insertion_point = match rng.below(3) {
                        0 => None,
                        1 => Some(WindowInsertionPoint::NextToSelection),
                        _ => Some(WindowInsertionPoint::EndOfTree),
                    }
                }
                1 => settings.master_ratio = rng.length(0.0, 1.0),
                2 => settings.master_count = rng.below(4) as usize,
                3 => settings.master_side = drawn_side(rng),
                4 => {
                    settings.new_window_placement = match rng.below(3) {
                        0 => MasterStackNewWindowPlacement::Master,
                        1 => MasterStackNewWindowPlacement::Stack,
                        _ => MasterStackNewWindowPlacement::Focused,
                    }
                }
                5 => settings.master_arrangement = drawn_arrangement(rng),
                _ => settings.stack_arrangement = drawn_arrangement(rng),
            }
        }
        settings
    }

    /// A saved system read back, its settings (not saved) configured again.
    fn reloaded(rng: &mut Rng, system: &MasterStackLayoutSystem) -> MasterStackLayoutSystem {
        let text = drawn::with_saved_only_states(rng, &ron::to_string(system).expect("written"));
        let mut loaded: MasterStackLayoutSystem = ron::from_str(&text).expect("and read back");
        assert_eq!(
            ron::to_string(&loaded).unwrap(),
            text,
            "a reload saves the same bytes"
        );
        loaded.update_settings(system.upstream.settings.clone());
        loaded
    }

    /// Drawn master-stack systems: every operation and query held to upstream's by the twin.
    #[test]
    fn drawn_master_stack_systems_step_the_core_as_upstream() {
        let mut rng = Rng(0x3a57_e125);
        let (mut windows, mut layouts) = (0, 0);
        for _ in 0..drawn::cases(3000) {
            let settings = drawn_settings(&mut rng, &MasterStackSettings::default());
            let mut system = MasterStackLayoutSystem::new(settings);
            let steps = rng.below(120);
            let drawn = drawn::run(
                &mut rng,
                &mut system,
                steps,
                &mut |rng, system, drawn| {
                    let layout = drawn.layout(rng);
                    match rng.below(5) {
                        0 => {
                            let settings = drawn_settings(rng, &system.upstream.settings);
                            system.update_settings(settings);
                        }
                        1 => system.adjust_master_ratio(layout, rng.length(-0.5, 0.5)),
                        2 => system.adjust_master_count(layout, rng.below(5) as i32 - 2),
                        3 => system.promote_to_master(layout),
                        _ => system.swap_master_stack(layout),
                    }
                    if rng.chance(30) {
                        let window = drawn.window(rng);
                        system.can_move_window(layout, window, drawn_direction_of(rng));
                        system.window_slot(layout, window);
                    }
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
        eprintln!("master-stack twin: {layouts} layouts, {windows} windows at the end");
    }

    /// The master-stack load normalizes as the traditional one does (both fullscreen flags here).
    #[test]
    fn a_saved_master_stack_node_with_both_fullscreen_flags_loads_fullscreen() {
        let mut system = MasterStackLayoutSystem::default();
        let layout = system.create_layout();
        system.update(WindowChange::Add(layout, WindowId::new(1, 1), None));
        system
            .edit_layout(layout, LayoutEdit::Fullscreen { within_gaps: true })
            .discard();
        let normalized = ron::to_string(&system).unwrap();
        let flags = "is_fullscreen:false,is_fullscreen_within_gaps:true";
        assert_eq!(normalized.matches(flags).count(), 1);
        let corrupt =
            normalized.replace(flags, "is_fullscreen:true,is_fullscreen_within_gaps:true");
        let normalized =
            normalized.replace(flags, "is_fullscreen:true,is_fullscreen_within_gaps:false");
        let mut loaded: MasterStackLayoutSystem = ron::from_str(&corrupt).unwrap();
        assert_eq!(ron::to_string(&loaded).unwrap(), normalized);
        loaded.create_layout();
    }

    fn drawn_direction_of(rng: &mut Rng) -> Direction {
        crate::layout_engine::systems::drawn::drawn_direction(rng)
    }
    /// The twin's cost per operation at 10, 100 and 1000 windows (release:
    /// `cargo test --release --lib measure_master_stack_twin -- --ignored --nocapture`): upstream's
    /// operation alone, the core's alone, the view comparison, a re-seed, and a pass's shape built
    /// by each. A plain release build pays the core's step on every operation and the comparison
    /// on every 32nd.
    #[test]
    #[ignore]
    fn measure_master_stack_twin() {
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
            let mut system = MasterStackLayoutSystem::default();
            let layout = system.create_layout();
            for idx in 1..=count {
                system.update(WindowChange::Add(layout, WindowId::new(1, idx), None));
            }
            let w = |i: u32| WindowId::new(1, 1 + i % count);
            // Everyday operations: focus moves, swaps, fullscreen, a window coming and going.
            let mut upstream = system.upstream.clone();
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
            let mut core = system.twin.core.clone();
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
                        core.add_window(layout, WindowId::new(2, 1));
                        core.remove_window(WindowId::new(2, 1));
                    }
                }
            }
            let core_each = each(started, ops);
            let rounds = 200;
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(view_difference(
                    system.twin.core.forest(),
                    &system.upstream.tiles,
                ));
            }
            let view_each = each(started, rounds);
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(core_of(&system.upstream));
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
                black_box(system.upstream.shape(layout, screen, 12.0, &constraints, &gaps, line));
            }
            let upstream_shape = each(started, rounds);
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(system.twin.core.forest().shape(
                    layout,
                    pass(screen, &gaps, 12.0, line),
                    &|window| constraints.get(&window).copied().map(core_constraints),
                ));
            }
            let core_shape = each(started, rounds);
            let per_op = view_each / VIEW_EVERY as u32;
            eprintln!(
                "master_stack {count} windows: upstream {upstream_each:?}/op, core {core_each:?}/op, \
                 view {view_each:?} ({per_op:?}/op 1 in {VIEW_EVERY}), re-seed {seed_each:?}, \
                 shape upstream {upstream_shape:?} core {core_shape:?}"
            );
        }
    }
}
