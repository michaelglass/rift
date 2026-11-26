//! The traditional layout system: the core's forest decides (layout trees, the group forest),
//! and is saved in upstream's form, byte for byte (`group_stored`). Under `roc-core` the Roc
//! forest decides, the core's its shadow (`group_roc`).

use objc2_core_foundation::CGRect;
use rift_core::layout::group_forest::{NewSibling, Traditional};
#[cfg(feature = "roc-core")]
use rift_core_roc::RocTraditional;
#[cfg(feature = "roc-core")]
use rift_core_roc::group_forest::{Answer, ForestEvent as F, TraditionalEvent as E};
use serde::{Deserialize, Serialize};

use crate::actor::app::{WindowId, pid_t};
use crate::common::collections::HashMap;
use crate::common::config::{GapSettings, StackLineReserve, WindowInsertionPoint};
use crate::layout_engine::engine::GroupContainerInfo;
#[cfg(feature = "roc-core")]
use crate::layout_engine::systems::bsp::roc_tree as bsp_roc;
#[cfg(feature = "roc-core")]
use crate::layout_engine::systems::group_roc::{self as roc, RocForest};
use crate::layout_engine::systems::group_stored::{Saved, StoredForest};
use crate::layout_engine::systems::group_system::{
    container_node, core_edit, core_focus, core_query, edit_result, focus_result, participation,
    pass, project_groups, resize,
};
use crate::layout_engine::systems::{
    EditResult, Focus, FocusResult, LayoutEdit, LayoutSystem, WindowChange,
    WindowLayoutConstraints, WindowQuery, core_direction, insertion_point,
};
use crate::layout_engine::{Direction, LayoutId};
use crate::model::tree::NodeId;

pub(in crate::layout_engine::systems) type CoreTraditional =
    Traditional<LayoutId, NodeId, WindowId>;

const SYSTEM: &str = "traditional";

/// The traditional layout system: the core's forest decides (under `roc-core`, the Roc forest,
/// the core's then its shadow). Loaded into the core (`group_stored`: the saved forest parsed,
/// what only a file holds normalized), saved from the core in upstream's form.
#[derive(Clone, Debug, Deserialize)]
#[serde(from = "StoredForest")]
pub struct TraditionalLayoutSystem {
    core: CoreTraditional,
    /// Under `roc-core`, the deciding forest (the core's is then its shadow).
    #[cfg(feature = "roc-core")]
    roc: RocForest<RocTraditional>,
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

impl From<CoreTraditional> for TraditionalLayoutSystem {
    fn from(core: CoreTraditional) -> Self {
        Self {
            #[cfg(feature = "roc-core")]
            roc: RocForest::seeded(&core),
            core,
        }
    }
}

impl From<StoredForest> for TraditionalLayoutSystem {
    /// A saved system: the core built from the loaded forest, with the settings a load starts
    /// with (none are saved; as upstream's load, the default insertion point and halving the
    /// selection, until rift applies its config).
    fn from(stored: StoredForest) -> Self {
        CoreTraditional::from_parts(
            stored.normalized(SYSTEM).forest(),
            insertion_point(WindowInsertionPoint::default()),
            new_sibling(false),
        )
        .into()
    }
}

impl Default for TraditionalLayoutSystem {
    fn default() -> Self { Self::new(WindowInsertionPoint::default(), true) }
}

impl TraditionalLayoutSystem {
    pub fn new(window_insertion_point: WindowInsertionPoint, equalize_nodes: bool) -> Self {
        CoreTraditional::new(
            insertion_point(window_insertion_point),
            new_sibling(equalize_nodes),
        )
        .into()
    }

    pub fn set_window_insertion_point(&mut self, value: WindowInsertionPoint) {
        self.core.set_window_insertion_point(insertion_point(value));
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "set_window_insertion_point",
            E::Settings(self.core.insertion(), self.core.new_sibling()),
            Answer::Done,
        );
    }

    pub fn set_equalize_nodes(&mut self, value: bool) {
        self.core.set_new_sibling(new_sibling(value));
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "set_equalize_nodes",
            E::Settings(self.core.insertion(), self.core.new_sibling()),
            Answer::Done,
        );
    }

    /// A layout pass with its group bars, the core's.
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
            layout,
            screen,
            offset,
            constraints,
            gaps,
            line,
            emit_frames,
            selection_path_only,
            #[cfg(feature = "roc-core")]
            |pass, bars, rust| self.roc.project(layout, pass, constraints, bars, rust),
        )
    }

    /// rift-cli's drawing of the layout: the core's (under `roc-core` the Roc forest's rows,
    /// held to the Rust forest's).
    pub(crate) fn draw_tree(&self, layout: LayoutId) -> String {
        #[cfg(feature = "roc-core")]
        return self.roc.draw(layout, self.core.forest().rows(layout));
        #[cfg(not(feature = "roc-core"))]
        self.core.forest().draw(layout)
    }

    /// rift-cli's container view of the layout: the core's (under `roc-core` the Roc forest's,
    /// held to the Rust forest's).
    pub(crate) fn container_tree(&self, layout: LayoutId) -> rift_protocol::ContainerTreeNode {
        let view = self.core.forest().container_view(layout);
        #[cfg(feature = "roc-core")]
        let view = self.roc.container_view(layout, view);
        container_node(view)
    }

    pub(crate) fn window_slot(&self, layout: LayoutId, window: WindowId) -> Option<Vec<usize>> {
        let core = self.core.forest().window_slot(layout, window);
        #[cfg(feature = "roc-core")]
        let core = bsp_roc::decided(
            self.roc_asked(
                "window_slot",
                E::Forest(F::WindowSlot(roc::layout_key(layout), roc::window(window))),
                Answer::WindowSlot(core.clone()),
            ),
            bsp_roc::as_slot,
            core,
        );
        core
    }

    pub(in crate::layout_engine) fn can_move_window(
        &self,
        layout: LayoutId,
        window: WindowId,
        direction: Direction,
    ) -> bool {
        let core = self.core.can_move_window(layout, window, core_direction(direction));
        #[cfg(feature = "roc-core")]
        let core = bsp_roc::decided(
            self.roc_asked(
                "can_move_window",
                E::CanMoveWindow(
                    roc::layout_key(layout),
                    roc::window(window),
                    core_direction(direction),
                ),
                Answer::Verdict(core),
            ),
            bsp_roc::as_verdict,
            core,
        );
        core
    }

    /// The settings as rift configures them (the insertion point, whether new siblings take
    /// the average weight).
    #[cfg(test)]
    fn configured(&self) -> (WindowInsertionPoint, bool) {
        let insertion = match self.core.insertion() {
            rift_core::layout::group_forest::InsertionPoint::NextToSelection => {
                WindowInsertionPoint::NextToSelection
            }
            rift_core::layout::group_forest::InsertionPoint::EndOfTree => {
                WindowInsertionPoint::EndOfTree
            }
        };
        (insertion, self.core.new_sibling() == NewSibling::AverageWeight)
    }

    /// The core forest's shape of a layout for a pass (what `calculate_layout` tiles).
    #[cfg(test)]
    pub(super) fn shape(
        &self,
        layout: LayoutId,
        screen: CGRect,
        offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &GapSettings,
        line: StackLineReserve,
    ) -> Option<rift_core::layout::traditional::Traditional<WindowId>> {
        self.core.forest().shape(layout, pass(screen, gaps, offset, line), &|w| {
            constraints
                .get(&w)
                .copied()
                .map(crate::layout_engine::core_geometry::core_constraints)
        })
    }

    /// A window's leaf in a layout.
    #[cfg(test)]
    pub(super) fn leaf(&self, layout: LayoutId, window: WindowId) -> NodeId {
        let index = self.core.forest().index();
        let found = index.into_iter().find(|&(w, l, _)| w == window && l == layout);
        found.expect("the window is in the layout").2
    }

    /// A node's parent.
    #[cfg(test)]
    pub(super) fn parent(&self, node: NodeId) -> NodeId {
        self.core.forest().parent(node).expect("the node has a parent")
    }

    /// The system with these nodes' sizes among their siblings set (a state operations reach
    /// only by many resizes, or a saved file holds).
    #[cfg(test)]
    pub(super) fn with_sizes(&mut self, sizes: &[(NodeId, f32)]) {
        let forest =
            crate::layout_engine::systems::group_system::with_sizes(self.core.forest(), sizes);
        *self = CoreTraditional::from_parts(forest, self.core.insertion(), self.core.new_sibling())
            .into();
    }

    /// The Rust forest re-seeded from the deciding Roc forest's view, as after a divergence.
    #[cfg(all(test, feature = "roc-core"))]
    fn reseed_rust_from_roc(&mut self) {
        use crate::layout_engine::systems::group_roc::Machine;
        self.core = rift_core_roc::RocTraditional::core_of(&self.roc.view());
    }

    /// Under `roc-core`, an operation the core decided (answering `rust`): the Roc forest takes
    /// it too, and is held to the core's answer and whole system.
    #[cfg(feature = "roc-core")]
    fn roc_stepped(&mut self, what: &str, event: E, rust: Answer) -> Answer {
        self.roc.decided(&mut self.core, what, event, rust)
    }

    /// Under `roc-core`, a query the core answered (`rust`): the Roc forest's answer is held to
    /// it.
    #[cfg(feature = "roc-core")]
    fn roc_asked(&self, what: &str, event: E, rust: Answer) -> Answer {
        self.roc.asked(what, event, rust)
    }
}

impl LayoutSystem for TraditionalLayoutSystem {
    fn has_windows_for_app(&self, layout: LayoutId, pid: pid_t) -> bool {
        let core = self.core.forest().has_windows_for_app(layout, |window| window.pid == pid);
        #[cfg(feature = "roc-core")]
        let core = bsp_roc::decided(
            self.roc_asked(
                "has_windows_for_app",
                E::Forest(F::HasWindowsForApp(
                    roc::layout_key(layout),
                    roc::app_windows(self.core.forest(), |w| w.pid == pid),
                )),
                Answer::Verdict(core),
            ),
            bsp_roc::as_verdict,
            core,
        );
        core
    }

    fn window_participates(&self, window: WindowId) -> bool {
        let core = self.core.forest().window_participates(window);
        #[cfg(feature = "roc-core")]
        let core = bsp_roc::decided(
            self.roc_asked(
                "window_participates",
                E::Forest(F::WindowParticipates(roc::window(window))),
                Answer::Verdict(core),
            ),
            bsp_roc::as_verdict,
            core,
        );
        core
    }

    fn set_window_participation(&mut self, window: WindowId, participates: bool) -> bool {
        let core = self.core.set_window_participation(window, participation(participates));
        #[cfg(feature = "roc-core")]
        let core = bsp_roc::decided(
            self.roc_stepped(
                "set_window_participation",
                E::Forest(F::Participation(
                    roc::window(window),
                    participation(participates),
                )),
                Answer::Participated(core),
            ),
            bsp_roc::as_participated,
            core,
        );
        core
    }

    fn create_layout(&mut self) -> LayoutId {
        let core = self.core.create_layout();
        #[cfg(feature = "roc-core")]
        let core = bsp_roc::decided(
            self.roc_stepped(
                "create_layout",
                E::CreateLayout,
                Answer::Layout(roc::layout_key(core)),
            ),
            bsp_roc::as_layout,
            core,
        );
        core
    }

    fn contains_layout(&self, layout: LayoutId) -> bool {
        let core = self.core.forest().contains_layout(layout);
        #[cfg(feature = "roc-core")]
        let core = bsp_roc::decided(
            self.roc_asked(
                "contains_layout",
                E::Forest(F::ContainsLayout(roc::layout_key(layout))),
                Answer::Verdict(core),
            ),
            bsp_roc::as_verdict,
            core,
        );
        core
    }

    fn clone_layout(&mut self, layout: LayoutId) -> LayoutId {
        let core = self.core.clone_layout(layout);
        #[cfg(feature = "roc-core")]
        let core = bsp_roc::decided(
            self.roc_stepped(
                "clone_layout",
                E::Forest(F::CloneLayout(roc::layout_key(layout))),
                Answer::Layout(roc::layout_key(core)),
            ),
            bsp_roc::as_layout,
            core,
        );
        core
    }

    fn remove_layout(&mut self, layout: LayoutId) {
        self.core.remove_layout(layout);
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "remove_layout",
            E::Forest(F::RemoveLayout(roc::layout_key(layout))),
            Answer::Done,
        );
    }

    /// The core's frames: under `roc-core` the Roc forest's own shape through the Roc geometry,
    /// with the Rust geometry's frames for the Rust forest's shape the shadow; else the Rust
    /// forest's shape through the Rust geometry.
    fn calculate_layout(
        &self,
        layout: LayoutId,
        screen: CGRect,
        offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &GapSettings,
        line: StackLineReserve,
        _shadow: crate::layout_engine::Shadow,
    ) -> Vec<(WindowId, CGRect)> {
        let core = self.core.forest().shape(layout, pass(screen, gaps, offset, line), &|w| {
            constraints
                .get(&w)
                .copied()
                .map(crate::layout_engine::core_geometry::core_constraints)
        });
        #[cfg(feature = "roc-core")]
        return crate::layout_engine::core_geometry::decide_roc_tree(SYSTEM, core, || {
            self.roc.frames(layout, pass(screen, gaps, offset, line), constraints)
        });
        #[cfg(not(feature = "roc-core"))]
        crate::layout_engine::core_geometry::decide_tree(SYSTEM, core)
    }

    fn selected_window(&self, layout: LayoutId) -> Option<WindowId> {
        let core = self.core.forest().selected_window(layout);
        #[cfg(feature = "roc-core")]
        let core = bsp_roc::decided(
            self.roc_asked(
                "selected_window",
                E::Forest(F::SelectedWindow(roc::layout_key(layout))),
                Answer::Window(core.map(roc::window)),
            ),
            bsp_roc::as_window,
            core,
        );
        core
    }

    fn selection_contains_window(&self, layout: LayoutId, window: WindowId) -> bool {
        let core = self.core.forest().selection_contains_window(layout, window);
        #[cfg(feature = "roc-core")]
        let core = bsp_roc::decided(
            self.roc_asked(
                "selection_contains_window",
                E::Forest(F::SelectionContainsWindow(
                    roc::layout_key(layout),
                    roc::window(window),
                )),
                Answer::Verdict(core),
            ),
            bsp_roc::as_verdict,
            core,
        );
        core
    }

    fn window_in_direction(&self, layout: LayoutId, direction: Direction) -> Option<WindowId> {
        let core = self.core.forest().window_in_direction(layout, core_direction(direction));
        #[cfg(feature = "roc-core")]
        let core = bsp_roc::decided(
            self.roc_asked(
                "window_in_direction",
                E::Forest(F::WindowInDirection(
                    roc::layout_key(layout),
                    core_direction(direction),
                )),
                Answer::Window(core.map(roc::window)),
            ),
            bsp_roc::as_window,
            core,
        );
        core
    }

    fn contains_window(&self, layout: LayoutId, window: WindowId) -> bool {
        let core = self.core.forest().contains_window(layout, window);
        #[cfg(feature = "roc-core")]
        let core = bsp_roc::decided(
            self.roc_asked(
                "contains_window",
                E::Forest(F::ContainsWindow(roc::layout_key(layout), roc::window(window))),
                Answer::Verdict(core),
            ),
            bsp_roc::as_verdict,
            core,
        );
        core
    }

    fn has_any_fullscreen_node(&self, layout: LayoutId) -> bool {
        let core = self.core.forest().has_any_fullscreen_node(layout);
        #[cfg(feature = "roc-core")]
        let core = bsp_roc::decided(
            self.roc_asked(
                "has_any_fullscreen_node",
                E::Forest(F::HasAnyFullscreenNode(roc::layout_key(layout))),
                Answer::Verdict(core),
            ),
            bsp_roc::as_verdict,
            core,
        );
        core
    }

    fn parent_of_selection_is_stacked(&self, layout: LayoutId) -> bool {
        let core = self.core.forest().parent_of_selection_is_stacked(layout);
        #[cfg(feature = "roc-core")]
        let core = bsp_roc::decided(
            self.roc_asked(
                "parent_of_selection_is_stacked",
                E::Forest(F::ParentOfSelectionIsStacked(roc::layout_key(layout))),
                Answer::Verdict(core),
            ),
            bsp_roc::as_verdict,
            core,
        );
        core
    }

    fn windows(&self, layout: LayoutId, query: WindowQuery) -> Vec<WindowId> {
        let core = self.core.forest().windows(layout, core_query(query));
        #[cfg(feature = "roc-core")]
        let core = bsp_roc::decided(
            self.roc_asked(
                "windows",
                E::Forest(F::Windows(
                    roc::layout_key(layout),
                    roc::query(core_query(query)),
                )),
                Answer::Windows(roc::windows(&core)),
            ),
            bsp_roc::as_windows,
            core,
        );
        core
    }

    fn focus(&mut self, layout: LayoutId, focus: Focus) -> FocusResult {
        let focused = self.core.focus(layout, core_focus(focus));
        #[cfg(feature = "roc-core")]
        let focused = bsp_roc::decided(
            self.roc_stepped(
                "focus",
                E::Forest(F::Focus(roc::layout_key(layout), roc::focus(core_focus(focus)))),
                roc::focused(&focused),
            ),
            bsp_roc::as_focused,
            focused,
        );
        let core = focus_result(focused);
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
        #[cfg(feature = "roc-core")]
        self.roc_stepped("update", event, Answer::Done);
    }

    fn edit_layout(&mut self, layout: LayoutId, edit: LayoutEdit) -> EditResult {
        let edit = core_edit(edit);
        let edited = edit.map(|edit| {
            let edited = self.core.edit(layout, edit);
            #[cfg(feature = "roc-core")]
            let edited = bsp_roc::decided(
                self.roc_stepped(
                    "edit_layout",
                    E::Edit(roc::layout_key(layout), roc::edit(edit)),
                    roc::edited(&edited),
                ),
                bsp_roc::as_edited,
                edited,
            );
            edited
        });
        let core = edited.map_or(EditResult::Unchanged, edit_result);
        core
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout_engine::systems::drawn::{Rng, drawn_direction};
    use crate::layout_engine::systems::group_system::drawn;

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
        let text = drawn::with_saved_only_states(rng, &saved);
        let mut loaded: TraditionalLayoutSystem = ron::from_str(&text).expect("and read back");
        assert_eq!(
            ron::to_string(&loaded).unwrap(),
            text,
            "a reload saves the same bytes"
        );
        let (insertion, equalize) = system.configured();
        loaded.set_window_insertion_point(insertion);
        loaded.set_equalize_nodes(equalize);
        loaded
    }

    /// Drawn traditional systems: every operation, query, view and pass through the system
    /// (under `roc-core` the Roc forest held to the Rust forest on each), reloads saving back
    /// byte for byte.
    #[test]
    fn drawn_traditional_systems_step() {
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
                    #[cfg(feature = "roc-core")]
                    if rng.chance(4) {
                        system.reseed_rust_from_roc();
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
        eprintln!("traditional: {layouts} layouts, {windows} windows at the end");
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

    /// `corrupt` loads and saves as `normalized`.
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

    /// A window in two layouts: a leaf listed again keeps its first place, an entry naming no
    /// leaf of its layout is dropped, and leaves the index misses are stamped in layout and
    /// tree order.
    #[test]
    fn a_saved_window_index_keeps_first_entries_and_appends_in_layout_order() {
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
        let (s, c) = (entry(source, source_leaf), entry(clone, clone_leaf));
        let saved = format!("[{s},{c}]");
        loads_normalized(&written_as(&file, &saved, "[]"), &file);
        loads_normalized(
            &written_as(&file, &saved, &format!("[{c},{s},{c}]")),
            &written_as(&file, &saved, &format!("[{c},{s}]")),
        );
        // An entry naming the clone's leaf under the source layout names no leaf of its
        // layout: dropped, so it stamps nothing ahead of the source's leaf.
        let stray = entry(source, clone_leaf);
        loads_normalized(&written_as(&file, &saved, &format!("[{stray},{s},{c}]")), &file);
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

    /// A system made by operations as rift saves it: windows resized, one filling the gaps, a
    /// split made a stack, a cloned layout, a vacant layout slot, a freed leaf.
    fn saved_system() -> (TraditionalLayoutSystem, LayoutId) {
        use crate::common::config::StackDefaultOrientation;
        use crate::layout_engine::systems::GroupEdit;
        use crate::layout_engine::{LayoutKind, ResizeOrientation};
        let mut system = TraditionalLayoutSystem::default();
        let layout = system.create_layout();
        for window in [w(1), w(2), w(3), w(4)] {
            system.update(WindowChange::Add(layout, window, None));
        }
        assert!(system.focus(layout, Focus::Select(w(2))).handled);
        system.edit_layout(layout, LayoutEdit::Split(LayoutKind::Vertical)).discard();
        system.update(WindowChange::Add(layout, w(5), None));
        assert!(system.focus(layout, Focus::Select(w(3))).handled);
        system
            .edit_layout(layout, LayoutEdit::Resize {
                amount: 0.15,
                orientation: ResizeOrientation::Horizontal,
            })
            .discard();
        assert!(system.focus(layout, Focus::Select(w(4))).handled);
        system
            .edit_layout(layout, LayoutEdit::Fullscreen { within_gaps: true })
            .discard();
        assert!(system.focus(layout, Focus::Select(w(5))).handled);
        system
            .edit_layout(
                layout,
                LayoutEdit::Group(GroupEdit::Stack(StackDefaultOrientation::Horizontal)),
            )
            .discard();
        system.clone_layout(layout);
        let removed = system.create_layout();
        system.remove_layout(removed);
        system.update(WindowChange::Remove(w(1)));
        (system, layout)
    }

    /// `traditional_saved.ron`, written by the core while its bytes were held to upstream's: the
    /// operations save to it, it loads and saves back byte for byte with the settings a load
    /// starts with, and the loaded forest allocates its next layout and node where the saved
    /// slotmaps do (as upstream's load did).
    #[test]
    fn the_saved_fixture_is_what_the_operations_write_and_loads_back() {
        use slotmap::Key;
        let file = include_str!("traditional_saved.ron").trim_end();
        let (system, layout) = saved_system();
        assert_eq!(ron::to_string(&system).unwrap(), file);
        let mut loaded: TraditionalLayoutSystem = ron::from_str(file).unwrap();
        assert_eq!(ron::to_string(&loaded).unwrap(), file);
        assert_eq!(loaded.draw_tree(layout), system.draw_tree(layout));
        // No settings are saved: a load starts with upstream's (the default insertion point,
        // and halving the selection: its skipped `equalize_nodes` read false).
        assert_eq!(
            (loaded.core.insertion(), loaded.core.new_sibling()),
            (
                insertion_point(WindowInsertionPoint::default()),
                NewSibling::HalvesSelection
            )
        );
        let next = loaded.create_layout();
        assert_eq!(next.data().as_ffi(), (3 << 32) | 3, "the vacant layout slot");
        loaded.update(WindowChange::Add(next, w(9), None));
        let leaf = loaded.container_tree(next).children[0].node_id;
        assert_eq!(leaf, (3 << 32) | 9, "the vacant node slot freed last");
    }

    /// One window beside a horizontal stack of three (the second selected, the first left out).
    fn a_window_and_a_stack() -> (TraditionalLayoutSystem, LayoutId) {
        use crate::common::config::StackDefaultOrientation;
        use crate::layout_engine::LayoutKind;
        use crate::layout_engine::systems::GroupEdit;
        let mut system = TraditionalLayoutSystem::default();
        let layout = system.create_layout();
        for window in [w(1), w(2)] {
            system.update(WindowChange::Add(layout, window, None));
        }
        assert!(system.focus(layout, Focus::Select(w(2))).handled);
        system.edit_layout(layout, LayoutEdit::Split(LayoutKind::Vertical)).discard();
        for window in [w(3), w(4)] {
            system.update(WindowChange::Add(layout, window, None));
        }
        system
            .edit_layout(
                layout,
                LayoutEdit::Group(GroupEdit::Stack(StackDefaultOrientation::Horizontal)),
            )
            .discard();
        assert!(system.focus(layout, Focus::Select(w(3))).handled);
        system.set_window_participation(w(2), false);
        (system, layout)
    }

    fn screen_and_gaps() -> (CGRect, GapSettings) {
        let mut gaps = GapSettings::default();
        gaps.outer.top = 10.0;
        gaps.outer.left = 20.0;
        gaps.outer.bottom = 30.0;
        gaps.outer.right = 40.0;
        gaps.inner.horizontal = 8.0;
        (rect(0.0, 25.0, 1000.0, 800.0), gaps)
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect {
        use objc2_core_foundation::{CGPoint, CGSize};
        CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
    }

    fn frames(system: &TraditionalLayoutSystem, layout: LayoutId) -> Vec<(WindowId, CGRect)> {
        let (screen, gaps) = screen_and_gaps();
        system.calculate_layout(
            layout,
            screen,
            12.0,
            &HashMap::default(),
            &gaps,
            StackLineReserve::default(),
            crate::layout_engine::Shadow::Run,
        )
    }

    /// The group bars of a pass (all of them, or the selection path's).
    fn bars(
        system: &TraditionalLayoutSystem,
        layout: LayoutId,
        path_only: bool,
    ) -> Vec<(
        u64,
        crate::layout_engine::LayoutKind,
        CGRect,
        usize,
        usize,
        Vec<WindowId>,
    )> {
        use slotmap::Key;
        let (screen, gaps) = screen_and_gaps();
        let (_, bars) = system.project_groups(
            layout,
            screen,
            0.0,
            &HashMap::default(),
            &gaps,
            StackLineReserve::default(),
            false,
            path_only,
        );
        bars.into_iter()
            .map(|bar| {
                (
                    bar.node_id.data().as_ffi(),
                    bar.container_kind,
                    bar.frame,
                    bar.total_count,
                    bar.selected_index,
                    bar.window_ids,
                )
            })
            .collect()
    }

    /// rift-cli's drawing of a traditional layout (`traditional_drawn.txt`: as upstream's system
    /// drew it) and its container view: the root and the stack, each node's weight, the
    /// selection, the window left out still listed.
    #[test]
    fn rift_cli_draws_and_lists_a_traditional_layout() {
        use crate::layout_engine::LayoutKind;
        let (system, layout) = a_window_and_a_stack();
        assert_eq!(
            system.draw_tree(layout),
            include_str!("traditional_drawn.txt"),
            "the drawing (ending with its own newline)"
        );
        let tree = system.container_tree(layout);
        let node = |n: &rift_protocol::ContainerTreeNode| {
            (n.node_id, n.layout_kind, n.window_id, n.weight, n.is_selected)
        };
        let id = |n: u64| (1 << 32) | n;
        assert_eq!(
            node(&tree),
            (id(1), Some(LayoutKind::Horizontal), None, None, false)
        );
        let children: Vec<_> = tree.children.iter().map(node).collect();
        assert_eq!(children, [
            (id(2), None, Some(w(1).into()), Some(1.0), false),
            (id(4), Some(LayoutKind::HorizontalStack), None, Some(1.0), false),
        ]);
        let stacked: Vec<_> = tree.children[1].children.iter().map(node).collect();
        assert_eq!(stacked, [
            (id(3), None, Some(w(2).into()), Some(1.0), false),
            (id(5), None, Some(w(3).into()), Some(1.0), true),
            (id(6), None, Some(w(4).into()), Some(1.0), false),
        ]);
    }

    /// The group bar shows the stack's windows taking part (the window left out before the
    /// selected one not counted), the same on the selection path; while the selected window
    /// fills the tiling area or the screen, the selection path shows none.
    #[test]
    fn the_group_bar_shows_the_stack_windows_taking_part() {
        use crate::layout_engine::LayoutKind;
        let (mut system, layout) = a_window_and_a_stack();
        let shown = vec![(
            (1 << 32) | 4,
            LayoutKind::HorizontalStack,
            rect(494.0, 35.0, 466.0, 760.0),
            2,
            0,
            vec![w(3), w(4)],
        )];
        assert_eq!(bars(&system, layout, false), shown);
        assert_eq!(bars(&system, layout, true), shown);
        for within_gaps in [true, false] {
            system.edit_layout(layout, LayoutEdit::Fullscreen { within_gaps }).discard();
            assert_eq!(bars(&system, layout, false), shown);
            assert!(bars(&system, layout, true).is_empty());
        }
    }

    /// Each fill's frames: tiled (the stack's windows offset along it), the selected window
    /// filling the tiling area, then the screen (the window left out placed nowhere).
    #[test]
    fn a_traditional_layout_places_each_fill() {
        let (mut system, layout) = a_window_and_a_stack();
        let first = (w(1), rect(20.0, 35.0, 466.0, 760.0));
        let last = (w(4), rect(506.0, 35.0, 454.0, 760.0));
        assert_eq!(frames(&system, layout), vec![
            first,
            (w(3), rect(494.0, 35.0, 454.0, 760.0)),
            last
        ]);
        system
            .edit_layout(layout, LayoutEdit::Fullscreen { within_gaps: true })
            .discard();
        assert_eq!(frames(&system, layout), vec![
            first,
            (w(3), rect(20.0, 35.0, 940.0, 760.0)),
            last
        ]);
        system
            .edit_layout(layout, LayoutEdit::Fullscreen { within_gaps: false })
            .discard();
        assert_eq!(frames(&system, layout), vec![
            first,
            (w(3), rect(0.0, 25.0, 1000.0, 800.0)),
            last
        ]);
    }

    /// What rift pays through the traditional system at 10, 100 and 1000 windows (release:
    /// `cargo test --release --lib measure_traditional_system -- --ignored --nocapture`):
    /// everyday operations (focus moves, swaps, fullscreen, a window coming and going) and a
    /// layout pass.
    #[test]
    #[ignore]
    fn measure_traditional_system() {
        use std::hint::black_box;
        use std::time::Instant;
        let directions = [
            Direction::Left,
            Direction::Up,
            Direction::Right,
            Direction::Down,
        ];
        let ops = 4000_u32;
        for count in [10_u32, 100, 1000] {
            let mut system = TraditionalLayoutSystem::default();
            let layout = system.create_layout();
            for idx in 1..=count {
                system.update(WindowChange::Add(layout, w(idx), None));
            }
            let started = Instant::now();
            for i in 0..ops {
                match i % 4 {
                    0 => drop(black_box(
                        system.focus(layout, Focus::Direction(directions[(i / 4 % 4) as usize])),
                    )),
                    1 => drop(black_box(system.edit_layout(
                        layout,
                        LayoutEdit::Swap(w(1 + i % count), w(1 + (i * 7) % count)),
                    ))),
                    2 => {
                        drop(black_box(system.edit_layout(layout, LayoutEdit::Fullscreen {
                            within_gaps: false,
                        })))
                    }
                    _ => {
                        system.update(WindowChange::Add(layout, w(count + 1), None));
                        system.update(WindowChange::Remove(w(count + 1)));
                    }
                }
            }
            let op_each = started.elapsed() / ops;
            let screen = CGRect::new(
                objc2_core_foundation::CGPoint::ZERO,
                objc2_core_foundation::CGSize::new(1600.0, 1000.0),
            );
            let (gaps, constraints, line) = (
                GapSettings::default(),
                HashMap::default(),
                StackLineReserve::default(),
            );
            let rounds = 200;
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(system.calculate_layout(
                    layout,
                    screen,
                    12.0,
                    &constraints,
                    &gaps,
                    line,
                    crate::layout_engine::Shadow::Run,
                ));
            }
            let pass_each = started.elapsed() / rounds;
            eprintln!(
                "traditional {count} windows: {op_each:?} an operation, {pass_each:?} a layout pass"
            );
        }
    }

    /// A Rust forest that differs from the deciding Roc forest is counted and fails a test (a
    /// running Rift warns and re-seeds it from the Roc forest).
    #[cfg(feature = "roc-core")]
    #[test]
    fn a_rust_forest_differing_from_the_roc_forest_is_counted_and_fails_tests() {
        let mut system = TraditionalLayoutSystem::default();
        let layout = system.create_layout();
        system.roc.machine().step(E::CreateLayout);
        let before = crate::layout_engine::systems::group_forest_reseeds();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            system.update(WindowChange::Add(layout, w(1), None));
        }));
        assert!(failed.is_err());
        assert!(crate::layout_engine::systems::group_forest_reseeds() > before);
    }

    /// The Rust forest re-seeded from the Roc forest's view holds what the Rust forest held, and
    /// allocates and stamps alike after it.
    #[cfg(feature = "roc-core")]
    #[test]
    fn the_rust_forest_reseeded_from_the_roc_view_holds_and_allocates_alike() {
        let mut system = TraditionalLayoutSystem::default();
        let layout = system.create_layout();
        for idx in 1..=6 {
            system.update(WindowChange::Add(layout, w(idx), None));
        }
        system.update(WindowChange::Remove(w(3)));
        let other = system.create_layout();
        system.remove_layout(other);
        let held = system.core.clone();
        system.reseed_rust_from_roc();
        assert_eq!(system.core, held);
        system.update(WindowChange::Add(layout, w(9), None));
        assert_eq!(system.create_layout(), {
            let mut again = held.clone();
            again.add_window(layout, w(9), None);
            again.create_layout()
        });
    }

    /// The Roc forest's cost at 10, 100 and 1000 windows (release, roc-core: `cargo test
    /// --release --lib --features roc-core measure_traditional_roc_forest -- --ignored
    /// --nocapture`): the Roc step, the view comparison, a Rust re-seed from the Roc view, a Roc
    /// layout pass against the Rust forest's shape and geometry.
    #[cfg(feature = "roc-core")]
    #[test]
    #[ignore]
    fn measure_traditional_roc_forest() {
        use std::hint::black_box;
        use std::time::Instant;

        use rift_core::layout::group_forest::{
            Direction as CoreDirection, Edit, Focus as CoreFocus,
        };

        use crate::layout_engine::systems::group_roc::Machine as _;
        for count in [10_u32, 100, 1000] {
            let mut system = TraditionalLayoutSystem::default();
            let layout = system.create_layout();
            for idx in 1..=count {
                system.update(WindowChange::Add(layout, w(idx), None));
            }
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
                black_box(system.roc.machine().step(event));
            }
            let step_each = started.elapsed() / ops;
            let held = system.roc.view();
            let rounds = 100;
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(system.roc.view() == held);
            }
            let view_each = started.elapsed() / rounds;
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(rift_core_roc::RocTraditional::core_of(&held));
            }
            let seed_each = started.elapsed() / rounds;
            let core = rift_core_roc::RocTraditional::core_of(&held);
            let screen = CGRect::new(
                objc2_core_foundation::CGPoint::new(0.0, 0.0),
                objc2_core_foundation::CGSize::new(1000.0, 800.0),
            );
            let pass = pass(
                screen,
                &GapSettings::default(),
                40.0,
                StackLineReserve::default(),
            );
            let constraints = HashMap::default();
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(system.roc.frames(layout, pass, &constraints));
            }
            let roc_pass = started.elapsed() / rounds;
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(core.forest().shape(layout, pass, &|_| None).map(|shape| shape.frames()));
            }
            let rust_pass = started.elapsed() / rounds;
            eprintln!(
                "traditional roc forest {count} windows: step {step_each:?}/op, view comparison \
                 {view_each:?}, rust re-seed {seed_each:?}, pass roc {roc_pass:?} rust \
                 {rust_pass:?}"
            );
        }
    }
}
