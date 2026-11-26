//! The floating layout system: the core's floating system decides and is saved (layout trees,
//! floating's step 3, policy (a)); upstream's system ([`upstream::Upstream`]) is its shadow,
//! stepped on every operation, its answers held to the core's on every operation and its whole
//! state on every operation in tests and debug builds and every 32nd otherwise. A difference is
//! counted, fails a test, or warns `floating-divergence:` and re-seeds the shadow from the core
//! (the deciding side stands). Saving writes the core in upstream's form; a load goes through
//! upstream's load and seeds the core.

mod upstream;

use std::collections::BTreeMap;

use objc2_core_foundation::CGRect;
#[cfg(test)]
use objc2_core_foundation::{CGPoint, CGSize};
use rift_core::layout::floating::Floating;
use serde::{Deserialize, Serialize};
pub(super) use upstream::Upstream;

#[cfg(test)]
use super::GroupEdit;
use super::floating_twin::{CoreFloating, Shadow, held, seed};
use super::group_stored::{Saved, upstream_tiles};
use super::group_system::{
    container_node, core_edit, core_focus, core_query, edit_result, focus_result, layout_kind,
    participation,
};
use super::{
    EditResult, Focus, FocusResult, LayoutEdit, LayoutSystem, WindowChange,
    WindowLayoutConstraints, WindowQuery, core_direction, insertion_point,
};
use crate::actor::app::{WindowId, pid_t};
use crate::common::collections::HashMap;
use crate::common::config::WindowInsertionPoint;
use crate::layout_engine::core_geometry::{cg_rect, core_constraints, core_gaps, core_rect};
use crate::layout_engine::{Direction, LayoutId, LayoutKind};
use crate::model::tree::NodeId;

/// The floating layout system: the core's, its settings, and upstream's beside it.
#[derive(Debug)]
pub struct FloatingLayoutSystem {
    core: CoreFloating,
    insertion: WindowInsertionPoint,
    /// Whether the next first frames of a layout spread out (cleared once they are placed).
    spread_on_initialization: bool,
    shadow: Shadow,
}

impl Default for FloatingLayoutSystem {
    fn default() -> Self { Self::new(WindowInsertionPoint::default()) }
}

/// A load goes through upstream's (its forms and its legacy maps); the core is seeded from it.
impl<'de> Deserialize<'de> for FloatingLayoutSystem {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let upstream = Upstream::deserialize(deserializer)?;
        let core = seed(&upstream.groups, upstream.frame_pairs());
        Ok(Self {
            core,
            insertion: upstream.insertion,
            spread_on_initialization: upstream.spread_on_initialization,
            shadow: Shadow::of(upstream),
        })
    }
}

/// The core in upstream's saved form: the forest as `inner`, each window's frames (by window),
/// whether first frames spread.
#[derive(Serialize)]
#[serde(rename = "FloatingLayoutSystem")]
struct SavedFloating {
    inner: Saved,
    geometry: BTreeMap<WindowId, upstream::FloatingGeometry>,
    spread_on_initialization: bool,
}

impl Serialize for FloatingLayoutSystem {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.saved().serialize(serializer)
    }
}

impl FloatingLayoutSystem {
    pub fn new(point: WindowInsertionPoint) -> Self {
        Self {
            core: CoreFloating::default(),
            insertion: point,
            spread_on_initialization: false,
            shadow: Shadow::of(Upstream::new(point)),
        }
    }

    fn saved(&self) -> SavedFloating {
        SavedFloating {
            inner: Saved::of(self.core.forest()),
            geometry: self
                .core
                .geometry()
                .iter()
                .map(|(&window, geometry)| {
                    (window, upstream::FloatingGeometry {
                        frame: geometry.frame().map(cg_rect),
                        restore: geometry.restore().map(cg_rect),
                    })
                })
                .collect(),
            spread_on_initialization: self.spread_on_initialization,
        }
    }

    /// The shadow made the core again: the forest with the same slots, versions and free lists
    /// (so the ids each allocates next agree), the frames, the settings.
    fn reseed_shadow(&mut self) {
        let mut upstream = Upstream::new(self.insertion);
        upstream.groups = upstream_tiles(self.core.forest());
        upstream.geometry = self.saved().geometry.into_iter().collect();
        upstream.spread_on_initialization = self.spread_on_initialization;
        self.shadow = Shadow::of(upstream);
    }

    /// After an operation both took: the whole systems compared when due; the shadow re-seeded
    /// from the core on a difference.
    fn settle(&mut self, what: &str) {
        if self.shadow.differs(what, &self.core) {
            self.reseed_shadow();
        }
    }

    /// An answer: the core's, upstream's held to it (a difference re-seeds the shadow).
    fn decided<T: PartialEq + std::fmt::Debug>(&mut self, what: &str, core: T, upstream: T) -> T {
        if core != upstream {
            super::floating_twin::report(what, &format!("core {core:?}, upstream {upstream:?}"));
            self.reseed_shadow();
        }
        core
    }

    fn floating(&self, screen: CGRect) -> Floating {
        Floating {
            screen: core_rect(screen),
            spread: self.spread_on_initialization,
        }
    }

    pub fn set_window_insertion_point(&mut self, point: WindowInsertionPoint) {
        self.insertion = point;
        self.shadow.upstream_mut().set_window_insertion_point(point);
    }

    pub(crate) fn spread_initial_windows(&mut self) {
        self.spread_on_initialization = true;
        self.shadow.upstream_mut().spread_initial_windows();
    }

    /// rift-cli's drawing of the layout (the core's; upstream's held to it).
    pub(crate) fn draw_tree(&self, layout: LayoutId) -> String {
        held(
            "draw_tree",
            self.core.forest().draw(layout),
            self.shadow.upstream().groups.draw_tree(layout, &[]),
        )
    }

    pub(crate) fn container_tree(&self, layout: LayoutId) -> rift_protocol::ContainerTreeNode {
        held(
            "container_tree",
            container_node(self.core.forest().container_view(layout)),
            self.shadow.upstream().groups.container_tree(layout),
        )
    }

    pub(crate) fn window_slot(&self, layout: LayoutId, window: WindowId) -> Option<Vec<usize>> {
        held(
            "window_slot",
            self.core.forest().window_slot(layout, window),
            self.shadow.upstream().groups.window_slot(layout, window),
        )
    }

    pub(crate) fn collect_group_containers(
        &self,
        layout: LayoutId,
        screen: CGRect,
        gaps: &crate::common::config::GapSettings,
        selection_path_only: bool,
    ) -> Vec<crate::layout_engine::engine::GroupContainerInfo> {
        let core: Vec<_> = self
            .core
            .group_bars(layout, self.floating(screen), selection_path_only)
            .into_iter()
            .map(|bar| crate::layout_engine::engine::GroupContainerInfo {
                node_id: bar.stack,
                container_kind: layout_kind(rift_core::layout::group_forest::Kind::Stack(
                    bar.orientation,
                )),
                frame: cg_rect(bar.frame),
                total_count: bar.windows.len(),
                selected_index: bar.selected,
                window_ids: bar.windows,
            })
            .collect();
        let upstream = self.shadow.upstream().collect_group_containers(
            layout,
            screen,
            gaps,
            selection_path_only,
        );
        held(
            "collect_group_containers",
            core.iter().map(bar_words).collect::<Vec<BarWords>>(),
            upstream.iter().map(bar_words).collect(),
        );
        core
    }

    pub(crate) fn cycle_windows(&self, layout: LayoutId) -> Vec<WindowId> {
        held(
            "cycle_windows",
            self.core.cycle_windows(layout),
            self.shadow.upstream().cycle_windows(layout),
        )
    }

    pub(crate) fn initialize_frames(
        &mut self,
        layout: LayoutId,
        screen: CGRect,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        get_frame: &impl Fn(WindowId) -> Option<CGRect>,
    ) {
        let floating = self.floating(screen);
        self.core.initialize_frames(
            layout,
            floating,
            |window| core_constraints(constraints.get(&window).copied().unwrap_or_default()),
            |window| get_frame(window).map(core_rect),
        );
        self.spread_on_initialization = false;
        self.shadow
            .upstream_mut()
            .initialize_frames(layout, screen, constraints, get_frame);
        self.settle("initialize_frames");
    }

    pub fn frame(&self, wid: WindowId) -> Option<CGRect> {
        held(
            "frame",
            self.core.frame(wid),
            self.shadow.upstream().frame(wid).map(core_rect),
        )
        .map(cg_rect)
    }

    pub fn store_frame(&mut self, wid: WindowId, frame: CGRect) {
        self.core.store_frame(wid, core_rect(frame));
        self.shadow.upstream_mut().store_frame(wid, frame);
        self.settle("store_frame");
    }

    pub fn contains_floating_window(&self, wid: WindowId) -> bool {
        held(
            "contains_floating_window",
            self.core.forest().index().iter().any(|&(window, ..)| window == wid),
            self.shadow.upstream().contains_floating_window(wid),
        )
    }

    /// A floating layout as the core's geometry takes it.
    pub(crate) fn shape(
        &self,
        layout: LayoutId,
        screen: CGRect,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &crate::common::config::GapSettings,
    ) -> rift_core::layout::floating::Layout<WindowId> {
        held(
            "shape",
            self.core.shape(layout, self.floating(screen), core_gaps(gaps), |window| {
                core_constraints(constraints.get(&window).copied().unwrap_or_default())
            }),
            self.shadow.upstream().shape(layout, screen, constraints, gaps),
        )
    }
}

impl LayoutSystem for FloatingLayoutSystem {
    fn window_participates(&self, window: WindowId) -> bool {
        held(
            "window_participates",
            self.core.forest().window_participates(window),
            self.shadow.upstream().window_participates(window),
        )
    }

    fn set_window_participation(&mut self, window: WindowId, participates: bool) -> bool {
        let core = self.core.set_window_participation(window, participation(participates));
        let upstream = self.shadow.upstream_mut().set_window_participation(window, participates);
        let answer = self.decided("set_window_participation", core, upstream);
        self.settle("set_window_participation");
        answer
    }

    fn contains_layout(&self, layout: LayoutId) -> bool {
        held(
            "contains_layout",
            self.core.forest().contains_layout(layout),
            self.shadow.upstream().contains_layout(layout),
        )
    }

    fn selected_window(&self, layout: LayoutId) -> Option<WindowId> {
        held(
            "selected_window",
            self.core.forest().selected_window(layout),
            self.shadow.upstream().selected_window(layout),
        )
    }

    fn selection_contains_window(&self, layout: LayoutId, window: WindowId) -> bool {
        held(
            "selection_contains_window",
            self.core.forest().selection_contains_window(layout, window),
            self.shadow.upstream().selection_contains_window(layout, window),
        )
    }

    fn contains_window(&self, layout: LayoutId, wid: WindowId) -> bool {
        held(
            "contains_window",
            self.core.forest().contains_window(layout, wid),
            self.shadow.upstream().contains_window(layout, wid),
        )
    }

    fn has_any_fullscreen_node(&self, layout: LayoutId) -> bool {
        held(
            "has_any_fullscreen_node",
            self.core.forest().has_any_fullscreen_node(layout),
            self.shadow.upstream().has_any_fullscreen_node(layout),
        )
    }

    fn create_layout(&mut self) -> LayoutId {
        let core = self.core.create_layout();
        let upstream = self.shadow.upstream_mut().create_layout();
        let answer = self.decided("create_layout", core, upstream);
        self.settle("create_layout");
        answer
    }

    fn clone_layout(&mut self, layout: LayoutId) -> LayoutId {
        let core = self.core.clone_layout(layout);
        let upstream = self.shadow.upstream_mut().clone_layout(layout);
        let answer = self.decided("clone_layout", core, upstream);
        self.settle("clone_layout");
        answer
    }

    fn remove_layout(&mut self, layout: LayoutId) {
        self.core.remove_layout(layout);
        self.shadow.upstream_mut().remove_layout(layout);
        self.settle("remove_layout");
    }

    /// The core geometry's frames for the core's shape (policy (a)); upstream's own frames
    /// ([`Upstream::shadow_frames`]) are their shadow, upstream's shape held to the core's.
    fn calculate_layout(
        &self,
        layout: LayoutId,
        screen: CGRect,
        _stack_offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &crate::common::config::GapSettings,
        _stack_line: crate::common::config::StackLineReserve,
        shadow: crate::layout_engine::Shadow,
    ) -> Vec<(WindowId, CGRect)> {
        crate::layout_engine::core_geometry::decide_floating(
            self.shape(layout, screen, constraints, gaps),
            shadow,
            || self.shadow.upstream().shadow_frames(layout, screen, constraints, gaps),
        )
    }

    fn window_in_direction(&self, layout: LayoutId, direction: Direction) -> Option<WindowId> {
        held(
            "window_in_direction",
            self.core.window_in_direction(layout, core_direction(direction)),
            self.shadow.upstream().window_in_direction(layout, direction),
        )
    }

    fn has_windows_for_app(&self, layout: LayoutId, pid: pid_t) -> bool {
        held(
            "has_windows_for_app",
            self.core.forest().has_windows_for_app(layout, |window| window.pid == pid),
            self.shadow.upstream().has_windows_for_app(layout, pid),
        )
    }

    fn parent_of_selection_is_stacked(&self, layout: LayoutId) -> bool {
        held(
            "parent_of_selection_is_stacked",
            self.core.forest().parent_of_selection_is_stacked(layout),
            self.shadow.upstream().parent_of_selection_is_stacked(layout),
        )
    }

    fn windows(&self, layout: LayoutId, query: WindowQuery) -> Vec<WindowId> {
        held(
            "windows",
            self.core.forest().windows(layout, core_query(query)),
            self.shadow.upstream().windows(layout, query),
        )
    }

    fn focus(&mut self, layout: LayoutId, focus: Focus) -> FocusResult {
        let core = focus_result(self.core.focus(layout, core_focus(focus)));
        let upstream = self.shadow.upstream_mut().focus(layout, focus);
        if focus_words(&core) != focus_words(&upstream) {
            super::floating_twin::report(
                "focus",
                &format!(
                    "core {:?}, upstream {:?}",
                    focus_words(&core),
                    focus_words(&upstream)
                ),
            );
            self.reseed_shadow();
        }
        self.settle("focus");
        core
    }

    fn update(&mut self, change: WindowChange<'_>) {
        match &change {
            WindowChange::Add(layout, window, _) => {
                self.core.add_window(*layout, *window, insertion_point(self.insertion))
            }
            WindowChange::Remove(window) | WindowChange::RemoveAndRebalance(window) => {
                self.core.remove_window(*window)
            }
            WindowChange::Replace(from, to) => self.core.replace_window(*from, *to),
            WindowChange::CloseApp(pid) => self.core.close_app(|window| window.pid == *pid),
            WindowChange::Resize { window, new, .. } => {
                self.core.store_frame(*window, core_rect(*new))
            }
        }
        self.shadow.upstream_mut().update(change);
        self.settle("update");
    }

    fn edit_layout(&mut self, layout: LayoutId, edit: LayoutEdit) -> EditResult {
        let core = edit_result(match core_edit(edit) {
            Some(edit) => self.core.edit(layout, edit),
            None => rift_core::layout::group_forest::Edited::Unsupported,
        });
        let upstream = self.shadow.upstream_mut().edit_layout(layout, edit);
        if edit_words(&core) != edit_words(&upstream) {
            super::floating_twin::report(
                "edit_layout",
                &format!(
                    "core {:?}, upstream {:?}",
                    edit_words(&core),
                    edit_words(&upstream)
                ),
            );
            self.reseed_shadow();
        }
        self.settle("edit_layout");
        core
    }
}

/// A focus answer, comparable.
fn focus_words(result: &FocusResult) -> (bool, Option<WindowId>, Vec<WindowId>) {
    (result.handled, result.window, result.raise.clone())
}

/// An edit answer, comparable.
fn edit_words(result: &EditResult) -> (&'static str, Vec<WindowId>) {
    match result {
        EditResult::Unsupported => ("unsupported", Vec::new()),
        EditResult::Unchanged => ("unchanged", Vec::new()),
        EditResult::Changed => ("changed", Vec::new()),
        EditResult::Raise(windows) => ("raise", windows.clone()),
    }
}

/// A group bar, comparable (its frame by bits).
type BarWords = (NodeId, LayoutKind, [u64; 4], usize, usize, Vec<WindowId>);

fn bar_words(bar: &crate::layout_engine::engine::GroupContainerInfo) -> BarWords {
    let frame = bar.frame;
    (
        bar.node_id,
        bar.container_kind,
        [
            frame.origin.x,
            frame.origin.y,
            frame.size.width,
            frame.size.height,
        ]
        .map(f64::to_bits),
        bar.total_count,
        bar.selected_index,
        bar.window_ids.clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(index: u32) -> WindowId { WindowId::new(1, index) }
    fn rect(x: f64, y: f64) -> CGRect { CGRect::new(CGPoint::new(x, y), CGSize::new(200., 150.)) }
    fn fixture() -> (FloatingLayoutSystem, LayoutId) {
        let mut system = FloatingLayoutSystem::default();
        let layout = system.create_layout();
        for index in 1..=4 {
            system.update(WindowChange::Add(layout, w(index), None));
            system.store_frame(w(index), rect(20. + (index - 1) as f64 * 250., 40.));
        }
        system.focus(layout, Focus::Select(w(1))).handled;
        (system, layout)
    }
    fn stack(system: &mut FloatingLayoutSystem, layout: LayoutId) {
        system
            .edit(
                layout,
                LayoutEdit::Group(GroupEdit::Stack(
                    crate::common::config::default_stack_orientation(),
                )),
            )
            .into_windows();
    }

    /// A floating window rift has no frame for yet takes 70% of the screen, centred, and one
    /// with a frame keeps it (what rift does with a floating layout, whichever core decides it).
    #[test]
    fn a_new_floating_window_takes_most_of_the_screen_centred() {
        let mut system = FloatingLayoutSystem::default();
        let layout = system.create_layout();
        system.update(WindowChange::Add(layout, w(1), None));
        system.update(WindowChange::Add(layout, w(2), None));
        system.store_frame(w(2), rect(20., 40.));
        let frames = system.calculate_layout(
            layout,
            CGRect::new(CGPoint::ZERO, CGSize::new(1000., 800.)),
            0.0,
            &HashMap::default(),
            &crate::common::config::GapSettings::default(),
            Default::default(),
            crate::layout_engine::Shadow::Run,
        );
        assert_eq!(frames, vec![
            (
                w(1),
                CGRect::new(CGPoint::new(150., 120.), CGSize::new(700., 560.))
            ),
            (w(2), rect(20., 40.)),
        ]);
    }

    #[test]
    fn suspended_stack_selection_leaves_the_visible_tab_interactive() {
        let (mut system, layout) = fixture();
        system.edit(layout, LayoutEdit::Group(GroupEdit::Join(Direction::Right)));
        stack(&mut system, layout);
        system.set_window_participation(w(1), false);
        assert!(system.windows(layout, WindowQuery::Visible).contains(&w(2)));

        let moved = rect(100., 80.);
        system.store_frame(w(2), moved);
        assert_eq!(system.frame(w(2)), Some(moved));
        let screen = CGRect::new(CGPoint::ZERO, CGSize::new(1000., 800.));
        let groups = system.collect_group_containers(layout, screen, &Default::default(), false);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].window_ids, [w(2)]);
        assert_eq!(groups[0].frame, moved);
    }

    #[test]
    fn explicit_stacks_share_geometry_and_restore_only_their_members() {
        let (mut system, layout) = fixture();
        stack(&mut system, layout);
        assert_eq!(system.windows(layout, WindowQuery::Visible).len(), 4);
        system.edit(layout, LayoutEdit::Group(GroupEdit::Join(Direction::Right)));
        assert_ne!(system.frame(w(1)), system.frame(w(2)));
        stack(&mut system, layout);
        system.edit(layout, LayoutEdit::Group(GroupEdit::Join(Direction::Right)));
        assert_eq!(system.cycle_windows(layout), vec![w(1), w(2), w(3)]);
        system.update(WindowChange::Add(layout, w(5), None));
        system.update(WindowChange::Add(layout, w(6), None));
        assert_eq!(system.windows(layout, WindowQuery::Visible).len(), 4);
        system.focus(layout, Focus::Select(w(2))).handled;
        system.store_frame(w(2), rect(100., 80.));
        system.store_frame(w(1), rect(999., 999.));
        assert_eq!(system.frame(w(1)), Some(rect(100., 80.)));
        // A stale frame to return to, set by hand on the core; the shadow takes it too.
        let mut geometry = system.core.geometry().clone();
        geometry.insert(w(4), rift_core::layout::group_forest::Geometry::Aligned {
            frame: core_rect(rect(770., 40.)),
            restore: core_rect(rect(999., 999.)),
        });
        system.core = CoreFloating::from_parts(system.core.forest().clone(), geometry);
        system.reseed_shadow();
        system.edit(layout, LayoutEdit::Group(GroupEdit::Unjoin));
        assert_eq!(system.frame(w(2)), Some(rect(270., 40.)));
        assert_eq!(system.frame(w(4)), Some(rect(770., 40.)));
        system.focus(layout, Focus::Select(w(1))).handled;
        system.edit(
            layout,
            LayoutEdit::Group(GroupEdit::Unstack(
                crate::common::config::default_stack_orientation(),
            )),
        );
        assert_eq!(system.frame(w(1)), Some(rect(100., 80.)));
        assert_eq!(system.frame(w(3)), Some(rect(520., 40.)));
    }

    #[test]
    fn directional_focus_reaches_hidden_members_and_neighboring_windows() {
        let (mut system, layout) = fixture();
        system.edit(layout, LayoutEdit::Group(GroupEdit::Join(Direction::Right)));
        assert!(system.focus(layout, Focus::Ascend).handled);
        stack(&mut system, layout);
        assert_eq!(system.selected_window(layout), Some(w(1)));
        system.store_frame(w(3), rect(20., 400.));
        for (direction, expected) in [
            (Direction::Down, w(2)),
            (Direction::Down, w(3)),
            (Direction::Up, w(2)),
            (Direction::Up, w(1)),
        ] {
            assert_eq!(
                system.focus(layout, Focus::Direction(direction)).window,
                Some(expected)
            );
        }
        system.update(WindowChange::Remove(w(3)));
        system.update(WindowChange::Remove(w(4)));
        assert_eq!(
            system.focus(layout, Focus::Direction(Direction::Right)).window,
            Some(w(2))
        );
        assert_eq!(
            system.focus(layout, Focus::Direction(Direction::Right)).window,
            Some(w(1))
        );
        system.edit(layout, LayoutEdit::Group(GroupEdit::Unjoin));
        assert_eq!(system.windows(layout, WindowQuery::All).len(), 2);
        assert!(!system.parent_of_selection_is_stacked(layout));
    }

    #[test]
    fn saved_stack_frames_survive_identity_changes_and_layout_clones() {
        let (mut system, layout) = fixture();
        system.edit(layout, LayoutEdit::Group(GroupEdit::Join(Direction::Right)));
        stack(&mut system, layout);
        system.focus(layout, Focus::Select(w(3))).handled;
        system.edit(layout, LayoutEdit::Group(GroupEdit::Join(Direction::Right)));
        stack(&mut system, layout);
        system.focus(layout, Focus::Select(w(1))).handled;
        system.edit(layout, LayoutEdit::Group(GroupEdit::Join(Direction::Right)));
        assert_eq!(system.cycle_windows(layout), vec![w(1), w(2), w(3)]);
        assert_eq!(system.frame(w(4)), Some(rect(770., 40.)));
        let bytes = ron::to_string(&system).unwrap();
        let mut restored: FloatingLayoutSystem = ron::from_str(&bytes).unwrap();
        assert_eq!(restored.core.geometry(), system.core.geometry());
        restored.update(WindowChange::Replace(w(2), w(9)));
        assert_eq!(restored.frame(w(9)), system.frame(w(2)));
        assert_eq!(
            restored.core.geometry()[&w(9)].restore(),
            Some(core_rect(rect(270., 40.)))
        );
        let clone = restored.clone_layout(layout);
        restored.remove_layout(layout);
        assert!(restored.frame(w(9)).is_some());
        restored.remove_layout(clone);
        assert!(restored.core.geometry().is_empty());
    }

    /// A floating system after random edits: windows added and removed, frames kept (the user
    /// moving them, or the first pass placing them, spread or not), every focus, fullscreen of a
    /// window or a group, participation, joins, stacks, unstacks, swaps, tab reorders.
    fn drawn_system(
        rng: &mut crate::layout_engine::systems::drawn::Rng,
    ) -> (FloatingLayoutSystem, LayoutId, Vec<WindowId>) {
        use crate::common::config::StackDefaultOrientation;
        use crate::layout_engine::systems::drawn::{drawn_direction, drawn_screen};
        let mut system = FloatingLayoutSystem::default();
        let layout = system.create_layout();
        let mut windows: Vec<WindowId> = Vec::new();
        let mut next = 0;
        if rng.chance(30) {
            system.spread_initial_windows();
        }
        let orientation = |rng: &mut crate::layout_engine::systems::drawn::Rng| match rng.below(4) {
            0 => StackDefaultOrientation::Perpendicular,
            1 => StackDefaultOrientation::Same,
            2 => StackDefaultOrientation::Horizontal,
            _ => StackDefaultOrientation::Vertical,
        };
        for _ in 0..rng.below(30) {
            let pick = |rng: &mut crate::layout_engine::systems::drawn::Rng,
                        windows: &[WindowId]| {
                windows[rng.below(windows.len() as u64) as usize]
            };
            match rng.below(14) {
                0..=3 => {
                    next += 1;
                    let window = WindowId::new(1 + rng.below(3) as i32, next);
                    windows.push(window);
                    system.update(WindowChange::Add(layout, window, None));
                }
                _ if windows.is_empty() => {}
                4 => {
                    let window = pick(rng, &windows);
                    windows.retain(|w| *w != window);
                    system.update(WindowChange::Remove(window));
                }
                5 => {
                    let window = pick(rng, &windows);
                    let frame = drawn_screen(rng);
                    system.store_frame(window, frame);
                }
                6 => {
                    let focus = match rng.below(4) {
                        0 => Focus::Select(pick(rng, &windows)),
                        1 => Focus::Ascend,
                        2 => Focus::Descend,
                        _ => Focus::Direction(drawn_direction(rng)),
                    };
                    system.focus(layout, focus);
                }
                7 => {
                    if rng.chance(30) {
                        system.focus(layout, Focus::Ascend);
                    }
                    let within_gaps = rng.chance(50);
                    system.edit(layout, LayoutEdit::Fullscreen { within_gaps });
                }
                8 => {
                    let window = pick(rng, &windows);
                    system.set_window_participation(window, rng.chance(40));
                }
                9 => {
                    let edit = match rng.below(5) {
                        0 => GroupEdit::Join(drawn_direction(rng)),
                        1 => GroupEdit::ConsumeOrExpel(drawn_direction(rng)),
                        2 => GroupEdit::Stack(orientation(rng)),
                        3 => GroupEdit::Unstack(orientation(rng)),
                        _ => GroupEdit::Unjoin,
                    };
                    system.edit(layout, LayoutEdit::Group(edit));
                }
                10 => {
                    let (a, b) = (pick(rng, &windows), pick(rng, &windows));
                    system.edit(layout, LayoutEdit::Swap(a, b));
                }
                11 => {
                    let (window, target) = (pick(rng, &windows), pick(rng, &windows));
                    system.edit(layout, LayoutEdit::ReorderTab { window, target });
                }
                12 => {
                    // The first pass places the windows that have no frame (seen or not).
                    let screen = drawn_screen(rng);
                    let mut seen: HashMap<WindowId, CGRect> = HashMap::default();
                    for &window in &windows {
                        if rng.chance(50) {
                            seen.insert(window, drawn_screen(rng));
                        }
                    }
                    system.initialize_frames(layout, screen, &HashMap::default(), &|window| {
                        seen.get(&window).copied()
                    });
                    if rng.chance(30) {
                        system.spread_initial_windows();
                    }
                }
                _ => {
                    system.edit(layout, LayoutEdit::ToggleOrientation);
                }
            }
        }
        (system, layout, windows)
    }

    /// The core's frames for a floating layout are the system's own (`shadow_frames`), bit for bit,
    /// over drawn systems, screens, gaps and window constraints.
    #[test]
    fn the_core_places_floating_layouts_as_the_floating_system_does() {
        use crate::layout_engine::systems::drawn::*;
        let mut rng = Rng(0x5eed_f10a);
        let (mut cases, mut frames_seen) = (0, 0);
        for _ in 0..3000 {
            let (system, layout, windows) = drawn_system(&mut rng);
            let screen = drawn_screen(&mut rng);
            let constraints = drawn_constraints_for(&mut rng, &windows);
            let gaps = drawn_gaps(&mut rng);
            // Drawn but unused (a floating layout has no bar): the draws stay as they were.
            let _ = drawn_line(&mut rng);
            let upstream =
                system.shadow.upstream().shadow_frames(layout, screen, &constraints, &gaps);
            let core: Vec<(WindowId, CGRect)> = system
                .shape(layout, screen, &constraints, &gaps)
                .frames()
                .into_iter()
                .map(|(window, frame)| (window, crate::layout_engine::cg_rect(frame)))
                .collect();
            assert_eq!(bits(&core), bits(&upstream));
            cases += 1;
            frames_seen += upstream.len();
        }
        eprintln!("floating: {cases} layouts, {frames_seen} frames identical");
        assert!(frames_seen > 3000, "the draws reach frames ({frames_seen})");
    }

    /// Every question the floating system answers, asked of `system` (each held to the core's).
    fn ask_everything(system: &FloatingLayoutSystem, layouts: &[LayoutId], windows: &[WindowId]) {
        use crate::layout_engine::systems::drawn::*;
        let screen = CGRect::new(CGPoint::new(10., 20.), CGSize::new(1200., 900.));
        let gaps = crate::common::config::GapSettings::default();
        for &layout in layouts {
            if !system.contains_layout(layout) {
                continue;
            }
            system.selected_window(layout);
            system.has_any_fullscreen_node(layout);
            system.parent_of_selection_is_stacked(layout);
            for query in [
                WindowQuery::All,
                WindowQuery::Visible,
                WindowQuery::Selection,
            ] {
                system.windows(layout, query);
            }
            for direction in [
                Direction::Left,
                Direction::Right,
                Direction::Up,
                Direction::Down,
            ] {
                system.window_in_direction(layout, direction);
            }
            system.cycle_windows(layout);
            system.draw_tree(layout);
            system.container_tree(layout);
            for only_path in [false, true] {
                system.collect_group_containers(layout, screen, &gaps, only_path);
            }
            let _ = system.calculate_layout(
                layout,
                screen,
                0.0,
                &HashMap::default(),
                &gaps,
                Default::default(),
                crate::layout_engine::Shadow::Run,
            );
            for &window in windows {
                system.contains_window(layout, window);
                system.selection_contains_window(layout, window);
                system.window_slot(layout, window);
                system.windows(layout, WindowQuery::Stack(window));
                system.has_windows_for_app(layout, window.pid);
            }
        }
        for &window in windows {
            system.window_participates(window);
            system.frame(window);
            system.contains_floating_window(window);
        }
        let _ = bits(&[]);
    }

    /// The core's floating system beside upstream's (step 2; since step 3 upstream's is the
    /// shadow), over drawn systems: each drawn as the frames test draws it and the shadow
    /// re-seeded from the core, then layouts cloned and removed, windows replaced, closed by
    /// app, removed (rebalancing or not) and resized through `update`, in two layouts; every
    /// question asked along the way, and the system reloaded through serde. Any difference fails
    /// the test (the twin panics in tests).
    #[test]
    fn the_core_floating_system_takes_every_operation_as_upstreams_does() {
        use crate::layout_engine::systems::drawn::*;
        let mut rng = Rng(0xf_10a7_7713);
        let mut steps = 0;
        for _ in 0..1500 {
            let (mut system, first, mut windows) = drawn_system(&mut rng);
            // The shadow re-seeded from the core (what a difference does), then held to it.
            system.reseed_shadow();
            let mut layouts = vec![first];
            ask_everything(&system, &layouts, &windows);
            let mut next = 1000;
            for _ in 0..rng.below(25) {
                let layout = layouts[rng.below(layouts.len() as u64) as usize];
                match rng.below(9) {
                    0 => layouts.push(system.clone_layout(layout)),
                    1 if layouts.len() > 1 => {
                        system.remove_layout(layout);
                        layouts.retain(|&l| l != layout);
                    }
                    2 => {
                        next += 1;
                        let window = WindowId::new(1 + rng.below(3) as i32, next);
                        windows.push(window);
                        let second = layouts[rng.below(layouts.len() as u64) as usize];
                        system.update(WindowChange::Add(second, window, None));
                    }
                    3 if !windows.is_empty() => {
                        let from = windows[rng.below(windows.len() as u64) as usize];
                        next += 1;
                        let to = if rng.chance(20) {
                            windows[rng.below(windows.len() as u64) as usize]
                        } else {
                            WindowId::new(from.pid, next)
                        };
                        system.update(WindowChange::Replace(from, to));
                        if from != to {
                            windows.retain(|&w| w != from);
                            if !windows.contains(&to) {
                                windows.push(to);
                            }
                        }
                    }
                    4 => {
                        let pid = 1 + rng.below(3) as i32;
                        system.update(WindowChange::CloseApp(pid));
                        windows.retain(|w| w.pid != pid);
                    }
                    5 if !windows.is_empty() => {
                        let window = windows[rng.below(windows.len() as u64) as usize];
                        windows.retain(|&w| w != window);
                        system.update(if rng.chance(50) {
                            WindowChange::RemoveAndRebalance(window)
                        } else {
                            WindowChange::Remove(window)
                        });
                    }
                    6 if !windows.is_empty() => {
                        let window = windows[rng.below(windows.len() as u64) as usize];
                        let gaps = crate::common::config::GapSettings::default();
                        system.update(WindowChange::Resize {
                            layout,
                            window,
                            old: drawn_screen(&mut rng),
                            new: drawn_screen(&mut rng),
                            screen: drawn_screen(&mut rng),
                            gaps: &gaps,
                        });
                    }
                    7 => {
                        let bytes = ron::to_string(&system).unwrap();
                        assert_eq!(
                            bytes,
                            upstream_save(system.shadow.upstream()),
                            "the core saves upstream's bytes"
                        );
                        let mut restored: FloatingLayoutSystem = ron::from_str(&bytes).unwrap();
                        restored.set_window_insertion_point(system.insertion);
                        system = restored;
                    }
                    _ if !windows.is_empty() => {
                        let window = windows[rng.below(windows.len() as u64) as usize];
                        system.focus(layout, Focus::Select(window));
                        system.edit(
                            layout,
                            LayoutEdit::Group(GroupEdit::Join(drawn_direction(&mut rng))),
                        );
                    }
                    _ => {}
                }
                steps += 1;
                if rng.chance(30) {
                    ask_everything(&system, &layouts, &windows);
                }
            }
            ask_everything(&system, &layouts, &windows);
        }
        eprintln!("floating twin: {steps} steps past the drawn systems, no difference");
        assert!(steps > 5000, "the draws reach steps ({steps})");
    }

    /// States only a saved file builds (floating's own operations never make them), loaded and
    /// stepped through the twin, so the core is held to upstream from each: a stack inside a
    /// stack (a stored frame shared through the outermost), an aligned window in a group that is
    /// not a stack (stacking it keeps its remembered frame), and a window at the root with a stale
    /// frame to return to (joining it into a split forgets it). Written once by upstream's own
    /// save from hand-built states; the follow-ups reach each state's rule.
    #[test]
    fn states_only_a_file_builds_step_through_the_twin() {
        let load = |text: &str| -> (FloatingLayoutSystem, LayoutId) {
            let system: FloatingLayoutSystem = ron::from_str(text).unwrap();
            let (layout, _) = system.core.forest().layouts().next().unwrap();
            (system, layout)
        };
        let all = [w(1), w(2), w(3), w(4)];

        let (mut system, layout) = load(include_str!("floating/stack_in_stack.ron"));
        for (i, window) in all.into_iter().enumerate() {
            system.store_frame(window, rect(5. + i as f64, 5.));
            ask_everything(&system, &[layout], &all);
        }
        assert_eq!(
            system.frame(w(3)),
            system.frame(w(1)),
            "the outer stack shares the frame"
        );

        let (mut system, layout) = load(include_str!("floating/aligned_in_split.ron"));
        system.focus(layout, Focus::Select(w(1)));
        stack(&mut system, layout);
        ask_everything(&system, &[layout], &all);
        assert_eq!(
            system.core.geometry()[&w(2)].restore().map(cg_rect),
            Some(rect(900., 600.)),
            "the remembered frame kept"
        );
        system.edit(
            layout,
            LayoutEdit::Group(GroupEdit::Unstack(
                crate::common::config::default_stack_orientation(),
            )),
        );
        ask_everything(&system, &[layout], &all);

        let (mut system, layout) = load(include_str!("floating/stale_restore_at_root.ron"));
        system.focus(layout, Focus::Select(w(2)));
        system.edit(layout, LayoutEdit::Group(GroupEdit::Join(Direction::Right)));
        ask_everything(&system, &[layout], &all);
        assert_eq!(
            system.core.geometry()[&w(3)].restore().map(cg_rect),
            None,
            "joined into a split, the stale frame is forgotten"
        );
        system.edit(layout, LayoutEdit::Group(GroupEdit::Unjoin));
        ask_everything(&system, &[layout], &all);
    }

    /// Upstream's system as upstream saves it, its frames in window order (upstream's map
    /// iterates in an order its history decides; the core writes them by window).
    fn upstream_save(upstream: &Upstream) -> String {
        #[derive(Serialize)]
        struct Sorted<'a> {
            inner: &'a super::super::groups::GroupLayouts,
            geometry: BTreeMap<WindowId, &'a upstream::FloatingGeometry>,
            spread_on_initialization: bool,
        }
        ron::to_string(&Sorted {
            inner: &upstream.groups,
            geometry: upstream
                .geometry
                .iter()
                .map(|(&window, geometry)| (window, geometry))
                .collect(),
            spread_on_initialization: upstream.spread_on_initialization,
        })
        .unwrap()
    }

    /// Each saved form a load reads (the three file-built states and the older form with its
    /// `frames` and `independent_frames` maps, a frame to return to with no frame among them),
    /// loaded by upstream and by the flipped system: the core saves upstream's load byte for
    /// byte, and upstream's load is frozen as bytes (`*.loaded.ron`).
    #[test]
    fn a_load_saves_upstreams_bytes_and_upstreams_loads_are_frozen() {
        for (name, text, frozen) in [
            (
                "stack_in_stack",
                include_str!("floating/stack_in_stack.ron"),
                include_str!("floating/stack_in_stack.loaded.ron"),
            ),
            (
                "aligned_in_split",
                include_str!("floating/aligned_in_split.ron"),
                include_str!("floating/aligned_in_split.loaded.ron"),
            ),
            (
                "stale_restore_at_root",
                include_str!("floating/stale_restore_at_root.ron"),
                include_str!("floating/stale_restore_at_root.loaded.ron"),
            ),
            (
                "legacy_frames",
                include_str!("floating/legacy_frames.ron"),
                include_str!("floating/legacy_frames.loaded.ron"),
            ),
        ] {
            let upstream: Upstream = ron::from_str(text).unwrap();
            let expected = upstream_save(&upstream);
            assert_eq!(expected, frozen.trim_end(), "{name}: upstream's load as frozen");
            let system: FloatingLayoutSystem = ron::from_str(text).unwrap();
            assert_eq!(
                ron::to_string(&system).unwrap(),
                expected,
                "{name}: the core saves upstream's load"
            );
        }
    }
}
