//! The floating layout system: the core's floating system (`group_forest::FloatingSystem`)
//! decides and is saved, in upstream's form (layout trees, floating's steps 3 and 5: upstream's
//! system and its tree, `GroupLayouts`, are deleted). Under `roc-core` the Roc floating system
//! decides and the core's is its shadow (`floating_roc`), saving still from the core. A load
//! reads the forest as rift saves it (`group_stored::StoredForest`, normalized as the other
//! group systems' loads are), and each window's frames in either form a file holds.

use std::collections::BTreeMap;

use objc2_core_foundation::CGRect;
#[cfg(test)]
use objc2_core_foundation::{CGPoint, CGSize};
use rift_core::layout::floating::Floating;
use rift_core::layout::group_forest::{FloatingSystem, Geometry};
#[cfg(feature = "roc-core")]
use rift_core_roc::group_forest::{Answer, FloatingAnswer as RocAnswer, FloatingEvent as RocEvent};
use serde::{Deserialize, Serialize};

#[cfg(test)]
use super::GroupEdit;
#[cfg(feature = "roc-core")]
use super::bsp::roc_tree::{
    as_edited, as_focused, as_layout, as_participated, as_slot, as_verdict, as_window, as_windows,
};
#[cfg(feature = "roc-core")]
use super::floating_roc::{
    FloatingRoc, as_frame, as_tree, decided, plain_focus, plain_query, plain_windows, tree,
};
#[cfg(feature = "roc-core")]
use super::group_roc::known;
use super::group_stored::{Saved, StoredForest};
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
use crate::layout_engine::{Direction, LayoutId};
use crate::model::tree::NodeId;

/// The core's floating system over rift's ids.
pub(super) type CoreFloating = FloatingSystem<LayoutId, NodeId, WindowId>;

const SYSTEM: &str = "floating";

#[cfg(feature = "roc-core")]
fn roc_layout(layout: LayoutId) -> rift_core_roc::group_forest::LayoutKey {
    super::group_roc::layout_key(layout)
}

#[cfg(feature = "roc-core")]
fn roc_window(window: WindowId) -> u64 { super::group_roc::window(window) }

/// The windows of app `pid` the core's forest holds, as words (Roc has no pid).
#[cfg(feature = "roc-core")]
fn app_words(core: &CoreFloating, pid: pid_t) -> Vec<u64> {
    super::group_roc::app_windows(core.forest(), |window| window.pid == pid)
}

/// A window's frames as a saved file holds them (upstream's `FloatingGeometry`): the frame the
/// layout keeps, and the frame to return to.
#[serde_with::serde_as]
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct SavedFrames {
    #[serde_as(as = "Option<crate::sys::geometry::CGRectDef>")]
    frame: Option<CGRect>,
    #[serde_as(as = "Option<crate::sys::geometry::CGRectDef>")]
    restore: Option<CGRect>,
}

/// A floating system as rift saves it (upstream's `FloatingLayoutSystem`: the forest as
/// `inner`, each window's frames, whether first frames spread), or as older files do: the frames
/// in two maps (`frames`, `independent_frames`, the frame to return to). Settings are not saved.
#[serde_with::serde_as]
#[derive(Deserialize)]
struct StoredFloating {
    inner: StoredForest,
    #[serde(default)]
    geometry: HashMap<WindowId, SavedFrames>,
    #[serde(default)]
    #[serde_as(as = "HashMap<_, crate::sys::geometry::CGRectDef>")]
    frames: HashMap<WindowId, CGRect>,
    #[serde(default)]
    #[serde_as(as = "HashMap<_, crate::sys::geometry::CGRectDef>")]
    independent_frames: HashMap<WindowId, CGRect>,
    #[serde(default)]
    spread_on_initialization: bool,
}

/// The core in upstream's saved form: the forest as `inner`, each window's frames (by window: a
/// save is the same bytes for the same system), whether first frames spread.
#[derive(Serialize)]
#[serde(rename = "FloatingLayoutSystem")]
struct SavedFloating {
    inner: Saved,
    geometry: BTreeMap<WindowId, SavedFrames>,
    spread_on_initialization: bool,
}

/// The floating layout system: the core's, and its settings.
#[derive(Debug)]
pub struct FloatingLayoutSystem {
    core: CoreFloating,
    insertion: WindowInsertionPoint,
    /// Whether the next first frames of a layout spread out (cleared once they are placed).
    spread_on_initialization: bool,
    /// Under `roc-core`, the deciding Roc floating system (the core's is then its shadow).
    #[cfg(feature = "roc-core")]
    roc: FloatingRoc,
}

impl Default for FloatingLayoutSystem {
    fn default() -> Self { Self::new(WindowInsertionPoint::default()) }
}

impl<'de> Deserialize<'de> for FloatingLayoutSystem {
    /// A saved system: the core built from the loaded forest and frames (the older form's two
    /// maps merged as upstream's load merged them), with the default insertion point until rift
    /// applies its config.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let stored = StoredFloating::deserialize(deserializer)?;
        let mut frames = stored.geometry;
        for (window, frame) in stored.frames {
            frames.entry(window).or_default().frame = Some(frame);
        }
        for (window, frame) in stored.independent_frames {
            frames.entry(window).or_default().restore = Some(frame);
        }
        let geometry = frames
            .into_iter()
            .filter_map(|(window, saved)| {
                Geometry::from_pair(saved.frame.map(core_rect), saved.restore.map(core_rect))
                    .map(|geometry| (window, geometry))
            })
            .collect();
        let core = CoreFloating::from_parts(stored.inner.normalized(SYSTEM).forest(), geometry);
        Ok(Self::of(
            core,
            WindowInsertionPoint::default(),
            stored.spread_on_initialization,
        ))
    }
}

impl Serialize for FloatingLayoutSystem {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        SavedFloating {
            inner: Saved::of(self.core.forest()),
            geometry: self
                .core
                .geometry()
                .iter()
                .map(|(&window, geometry)| {
                    (window, SavedFrames {
                        frame: geometry.frame().map(cg_rect),
                        restore: geometry.restore().map(cg_rect),
                    })
                })
                .collect(),
            spread_on_initialization: self.spread_on_initialization,
        }
        .serialize(serializer)
    }
}

impl FloatingLayoutSystem {
    pub fn new(point: WindowInsertionPoint) -> Self {
        Self::of(CoreFloating::default(), point, false)
    }

    fn of(core: CoreFloating, insertion: WindowInsertionPoint, spread: bool) -> Self {
        Self {
            #[cfg(feature = "roc-core")]
            roc: FloatingRoc::seeded(&core),
            core,
            insertion,
            spread_on_initialization: spread,
        }
    }

    /// The core's system set by hand (a test): the Roc system is seeded from it.
    #[cfg(test)]
    fn set_core(&mut self, core: CoreFloating) {
        #[cfg(feature = "roc-core")]
        {
            self.roc = FloatingRoc::seeded(&core);
        }
        self.core = core;
    }

    /// An operation the core took (answering `rust`): under `roc-core`, the Roc system takes it
    /// and decides (a difference re-seeds the core).
    #[cfg(feature = "roc-core")]
    fn roc_stepped(&mut self, what: &str, event: RocEvent, rust: RocAnswer) -> RocAnswer {
        self.roc.decided(&mut self.core, what, event, rust).0
    }

    /// A query the core answered (`rust`): under `roc-core`, the Roc system answers.
    #[cfg(feature = "roc-core")]
    fn roc_asked(&self, what: &str, event: RocEvent, rust: RocAnswer) -> RocAnswer {
        self.roc.asked(what, event, rust)
    }

    fn floating(&self, screen: CGRect) -> Floating {
        Floating {
            screen: core_rect(screen),
            spread: self.spread_on_initialization,
        }
    }

    pub fn set_window_insertion_point(&mut self, point: WindowInsertionPoint) {
        self.insertion = point;
    }

    pub(crate) fn spread_initial_windows(&mut self) { self.spread_on_initialization = true; }

    /// rift-cli's drawing of the layout.
    pub(crate) fn draw_tree(&self, layout: LayoutId) -> String {
        #[cfg(feature = "roc-core")]
        return rift_core::layout::group_forest::render(
            &self.roc.rows(layout, &self.core.forest().rows(layout)),
        );
        #[cfg(not(feature = "roc-core"))]
        self.core.forest().draw(layout)
    }

    /// rift-cli's container view of the layout.
    pub(crate) fn container_tree(&self, layout: LayoutId) -> rift_protocol::ContainerTreeNode {
        let view = self.core.forest().container_view(layout);
        #[cfg(feature = "roc-core")]
        let view = self.roc.container_view(layout, &view);
        container_node(view)
    }

    pub(crate) fn window_slot(&self, layout: LayoutId, window: WindowId) -> Option<Vec<usize>> {
        let core = self.core.forest().window_slot(layout, window);
        #[cfg(feature = "roc-core")]
        let core = decided(
            self.roc_asked(
                "window_slot",
                RocEvent::WindowSlot(roc_layout(layout), roc_window(window)),
                tree(Answer::WindowSlot(core.clone())),
            ),
            |answer| as_tree(answer).and_then(as_slot),
            core,
        );
        core
    }

    pub(crate) fn collect_group_containers(
        &self,
        layout: LayoutId,
        screen: CGRect,
        _gaps: &crate::common::config::GapSettings,
        selection_path_only: bool,
    ) -> Vec<crate::layout_engine::engine::GroupContainerInfo> {
        let bars = self.core.group_bars(layout, self.floating(screen), selection_path_only);
        #[cfg(feature = "roc-core")]
        let bars = self.roc.group_bars(layout, self.floating(screen), selection_path_only, &bars);
        bars.into_iter()
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
            .collect()
    }

    pub(crate) fn cycle_windows(&self, layout: LayoutId) -> Vec<WindowId> {
        let core = self.core.cycle_windows(layout);
        #[cfg(feature = "roc-core")]
        let core = decided(
            self.roc_asked(
                "cycle_windows",
                RocEvent::Cycle(roc_layout(layout)),
                tree(Answer::Windows(plain_windows(&core))),
            ),
            |answer| as_tree(answer).and_then(as_windows),
            core,
        );
        core
    }

    pub(crate) fn initialize_frames(
        &mut self,
        layout: LayoutId,
        screen: CGRect,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        get_frame: &impl Fn(WindowId) -> Option<CGRect>,
    ) {
        #[cfg(feature = "roc-core")]
        let asked = RocEvent::InitializeFrames {
            layout: roc_layout(layout),
            floating: self.floating(screen),
            known: known(constraints),
            live: self
                .core
                .forest()
                .windows(layout, rift_core::layout::group_forest::Query::All)
                .into_iter()
                .filter_map(|held| {
                    get_frame(held).map(|frame| (roc_window(held), core_rect(frame)))
                })
                .collect(),
        };
        let floating = self.floating(screen);
        self.core.initialize_frames(
            layout,
            floating,
            |window| core_constraints(constraints.get(&window).copied().unwrap_or_default()),
            |window| get_frame(window).map(core_rect),
        );
        self.spread_on_initialization = false;
        #[cfg(feature = "roc-core")]
        self.roc_stepped("initialize_frames", asked, tree(Answer::Done));
    }

    pub fn frame(&self, wid: WindowId) -> Option<CGRect> {
        let core = self.core.frame(wid);
        #[cfg(feature = "roc-core")]
        let core = decided(
            self.roc_asked(
                "frame",
                RocEvent::FrameOf(roc_window(wid)),
                RocAnswer::Frame(core),
            ),
            as_frame,
            core,
        );
        core.map(cg_rect)
    }

    pub fn store_frame(&mut self, wid: WindowId, frame: CGRect) {
        self.core.store_frame(wid, core_rect(frame));
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "store_frame",
            RocEvent::StoreFrame(roc_window(wid), core_rect(frame)),
            tree(Answer::Done),
        );
    }

    pub fn contains_floating_window(&self, wid: WindowId) -> bool {
        let core = self.core.forest().index().iter().any(|&(window, ..)| window == wid);
        #[cfg(feature = "roc-core")]
        let core = decided(
            self.roc_asked(
                "contains_floating_window",
                RocEvent::Floats(roc_window(wid)),
                tree(Answer::Verdict(core)),
            ),
            |answer| as_tree(answer).and_then(as_verdict),
            core,
        );
        core
    }

    /// A floating layout as the floating geometry takes it.
    pub(crate) fn shape(
        &self,
        layout: LayoutId,
        screen: CGRect,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &crate::common::config::GapSettings,
    ) -> rift_core::layout::floating::Layout<WindowId> {
        let core = self.core.shape(layout, self.floating(screen), core_gaps(gaps), |window| {
            core_constraints(constraints.get(&window).copied().unwrap_or_default())
        });
        #[cfg(feature = "roc-core")]
        let core = self.roc.shape(
            layout,
            self.floating(screen),
            core_gaps(gaps),
            &known(constraints),
            &core,
        );
        core
    }
}

impl LayoutSystem for FloatingLayoutSystem {
    fn window_participates(&self, window: WindowId) -> bool {
        let core = self.core.forest().window_participates(window);
        #[cfg(feature = "roc-core")]
        let core = decided(
            self.roc_asked(
                "window_participates",
                RocEvent::WindowParticipates(roc_window(window)),
                tree(Answer::Verdict(core)),
            ),
            |answer| as_tree(answer).and_then(as_verdict),
            core,
        );
        core
    }

    fn set_window_participation(&mut self, window: WindowId, participates: bool) -> bool {
        let core = self.core.set_window_participation(window, participation(participates));
        #[cfg(feature = "roc-core")]
        let core = decided(
            self.roc_stepped(
                "set_window_participation",
                RocEvent::Participation(roc_window(window), participation(participates)),
                tree(Answer::Participated(core)),
            ),
            |answer| as_tree(answer).and_then(as_participated),
            core,
        );
        core
    }

    fn contains_layout(&self, layout: LayoutId) -> bool {
        let core = self.core.forest().contains_layout(layout);
        #[cfg(feature = "roc-core")]
        let core = decided(
            self.roc_asked(
                "contains_layout",
                RocEvent::ContainsLayout(roc_layout(layout)),
                tree(Answer::Verdict(core)),
            ),
            |answer| as_tree(answer).and_then(as_verdict),
            core,
        );
        core
    }

    fn selected_window(&self, layout: LayoutId) -> Option<WindowId> {
        let core = self.core.forest().selected_window(layout);
        #[cfg(feature = "roc-core")]
        let core = decided(
            self.roc_asked(
                "selected_window",
                RocEvent::SelectedWindow(roc_layout(layout)),
                tree(Answer::Window(core.map(roc_window))),
            ),
            |answer| as_tree(answer).and_then(as_window),
            core,
        );
        core
    }

    fn selection_contains_window(&self, layout: LayoutId, window: WindowId) -> bool {
        let core = self.core.forest().selection_contains_window(layout, window);
        #[cfg(feature = "roc-core")]
        let core = decided(
            self.roc_asked(
                "selection_contains_window",
                RocEvent::SelectionContainsWindow(roc_layout(layout), roc_window(window)),
                tree(Answer::Verdict(core)),
            ),
            |answer| as_tree(answer).and_then(as_verdict),
            core,
        );
        core
    }

    fn contains_window(&self, layout: LayoutId, wid: WindowId) -> bool {
        let core = self.core.forest().contains_window(layout, wid);
        #[cfg(feature = "roc-core")]
        let core = decided(
            self.roc_asked(
                "contains_window",
                RocEvent::ContainsWindow(roc_layout(layout), roc_window(wid)),
                tree(Answer::Verdict(core)),
            ),
            |answer| as_tree(answer).and_then(as_verdict),
            core,
        );
        core
    }

    fn has_any_fullscreen_node(&self, layout: LayoutId) -> bool {
        let core = self.core.forest().has_any_fullscreen_node(layout);
        #[cfg(feature = "roc-core")]
        let core = decided(
            self.roc_asked(
                "has_any_fullscreen_node",
                RocEvent::HasAnyFullscreenNode(roc_layout(layout)),
                tree(Answer::Verdict(core)),
            ),
            |answer| as_tree(answer).and_then(as_verdict),
            core,
        );
        core
    }

    fn create_layout(&mut self) -> LayoutId {
        let core = self.core.create_layout();
        #[cfg(feature = "roc-core")]
        let core = decided(
            self.roc_stepped(
                "create_layout",
                RocEvent::CreateLayout,
                tree(Answer::Layout(roc_layout(core))),
            ),
            |answer| as_tree(answer).and_then(as_layout),
            core,
        );
        core
    }

    fn clone_layout(&mut self, layout: LayoutId) -> LayoutId {
        let core = self.core.clone_layout(layout);
        #[cfg(feature = "roc-core")]
        let core = decided(
            self.roc_stepped(
                "clone_layout",
                RocEvent::CloneLayout(roc_layout(layout)),
                tree(Answer::Layout(roc_layout(core))),
            ),
            |answer| as_tree(answer).and_then(as_layout),
            core,
        );
        core
    }

    fn remove_layout(&mut self, layout: LayoutId) {
        self.core.remove_layout(layout);
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "remove_layout",
            RocEvent::RemoveLayout(roc_layout(layout)),
            tree(Answer::Done),
        );
    }

    /// The frames of the layout's shape through the floating geometry (under `roc-core`, the
    /// Roc system's shape through the Roc geometry, the Rust core's frames the shadow).
    fn calculate_layout(
        &self,
        layout: LayoutId,
        screen: CGRect,
        _stack_offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &crate::common::config::GapSettings,
        _stack_line: crate::common::config::StackLineReserve,
        _shadow: crate::layout_engine::Shadow,
    ) -> Vec<(WindowId, CGRect)> {
        crate::layout_engine::core_geometry::decide_floating(self.shape(
            layout,
            screen,
            constraints,
            gaps,
        ))
    }

    fn window_in_direction(&self, layout: LayoutId, direction: Direction) -> Option<WindowId> {
        let core = self.core.window_in_direction(layout, core_direction(direction));
        #[cfg(feature = "roc-core")]
        let core = decided(
            self.roc_asked(
                "window_in_direction",
                RocEvent::WindowInDirection(roc_layout(layout), core_direction(direction)),
                tree(Answer::Window(core.map(roc_window))),
            ),
            |answer| as_tree(answer).and_then(as_window),
            core,
        );
        core
    }

    fn has_windows_for_app(&self, layout: LayoutId, pid: pid_t) -> bool {
        let core = self.core.forest().has_windows_for_app(layout, |window| window.pid == pid);
        #[cfg(feature = "roc-core")]
        let core = decided(
            self.roc_asked(
                "has_windows_for_app",
                RocEvent::HasWindowsForApp(roc_layout(layout), app_words(&self.core, pid)),
                tree(Answer::Verdict(core)),
            ),
            |answer| as_tree(answer).and_then(as_verdict),
            core,
        );
        core
    }

    fn parent_of_selection_is_stacked(&self, layout: LayoutId) -> bool {
        let core = self.core.forest().parent_of_selection_is_stacked(layout);
        #[cfg(feature = "roc-core")]
        let core = decided(
            self.roc_asked(
                "parent_of_selection_is_stacked",
                RocEvent::ParentOfSelectionIsStacked(roc_layout(layout)),
                tree(Answer::Verdict(core)),
            ),
            |answer| as_tree(answer).and_then(as_verdict),
            core,
        );
        core
    }

    fn windows(&self, layout: LayoutId, query: WindowQuery) -> Vec<WindowId> {
        let core = self.core.forest().windows(layout, core_query(query));
        #[cfg(feature = "roc-core")]
        let core = decided(
            self.roc_asked(
                "windows",
                RocEvent::Windows(roc_layout(layout), plain_query(core_query(query))),
                tree(Answer::Windows(plain_windows(&core))),
            ),
            |answer| as_tree(answer).and_then(as_windows),
            core,
        );
        core
    }

    fn focus(&mut self, layout: LayoutId, focus: Focus) -> FocusResult {
        let focused = self.core.focus(layout, core_focus(focus));
        #[cfg(feature = "roc-core")]
        let held_focus = super::group_roc::focused(&focused);
        let core = focus_result(focused);
        #[cfg(feature = "roc-core")]
        let core = decided(
            self.roc_stepped(
                "focus",
                RocEvent::Focus(roc_layout(layout), plain_focus(core_focus(focus))),
                tree(held_focus),
            ),
            |answer| as_tree(answer).and_then(as_focused).map(focus_result),
            core,
        );
        core
    }

    fn update(&mut self, change: WindowChange<'_>) {
        #[cfg(feature = "roc-core")]
        let asked = match &change {
            WindowChange::Add(layout, window, _) => RocEvent::AddWindow(
                roc_layout(*layout),
                roc_window(*window),
                insertion_point(self.insertion),
            ),
            WindowChange::Remove(window) | WindowChange::RemoveAndRebalance(window) => {
                RocEvent::RemoveWindow(roc_window(*window))
            }
            WindowChange::Replace(from, to) => RocEvent::ReplaceWindow {
                from: roc_window(*from),
                to: roc_window(*to),
            },
            WindowChange::CloseApp(pid) => RocEvent::CloseApp(app_words(&self.core, *pid)),
            WindowChange::Resize { window, new, .. } => {
                RocEvent::StoreFrame(roc_window(*window), core_rect(*new))
            }
        };
        match change {
            WindowChange::Add(layout, window, _) => {
                self.core.add_window(layout, window, insertion_point(self.insertion))
            }
            WindowChange::Remove(window) | WindowChange::RemoveAndRebalance(window) => {
                self.core.remove_window(window)
            }
            WindowChange::Replace(from, to) => self.core.replace_window(from, to),
            WindowChange::CloseApp(pid) => self.core.close_app(|window| window.pid == pid),
            WindowChange::Resize { window, new, .. } => {
                self.core.store_frame(window, core_rect(new))
            }
        }
        #[cfg(feature = "roc-core")]
        self.roc_stepped("update", asked, tree(Answer::Done));
    }

    fn edit_layout(&mut self, layout: LayoutId, edit: LayoutEdit) -> EditResult {
        let edited = match core_edit(edit) {
            Some(asked) => self.core.edit(layout, asked),
            // A drop that moves or consumes reaches a system only through `edit`.
            None => rift_core::layout::group_forest::Edited::Unsupported,
        };
        #[cfg(feature = "roc-core")]
        let held_edit = super::group_roc::edited(&edited);
        let core = edit_result(edited);
        #[cfg(feature = "roc-core")]
        let core = match core_edit(edit) {
            Some(asked) => decided(
                self.roc_stepped(
                    "edit_layout",
                    RocEvent::Edit(roc_layout(layout), super::group_roc::edit(asked)),
                    tree(held_edit),
                ),
                |answer| as_tree(answer).and_then(as_edited).map(edit_result),
                core,
            ),
            None => core,
        };
        core
    }
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
        // A stale frame to return to, set by hand on the core.
        let mut geometry = system.core.geometry().clone();
        geometry.insert(w(4), rift_core::layout::group_forest::Geometry::Aligned {
            frame: core_rect(rect(770., 40.)),
            restore: core_rect(rect(999., 999.)),
        });
        system.set_core(CoreFloating::from_parts(system.core.forest().clone(), geometry));
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

    /// The floating system over drawn systems (under `roc-core` the Roc system deciding, the
    /// core's held to it): each drawn, then layouts cloned and removed, windows replaced, closed by
    /// app, removed (rebalancing or not) and resized through `update`, in two layouts; every
    /// question asked along the way, and the system reloaded through serde. Any difference fails
    /// the test (the twin panics in tests).
    #[test]
    fn the_floating_system_takes_drawn_operations_and_reloads() {
        use crate::layout_engine::systems::drawn::*;
        let mut rng = Rng(0xf_10a7_7713);
        let mut steps = 0;
        for _ in 0..1500 {
            let (mut system, first, mut windows) = drawn_system(&mut rng);
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
                        let mut restored: FloatingLayoutSystem = ron::from_str(&bytes).unwrap();
                        assert_eq!(
                            ron::to_string(&restored).unwrap(),
                            bytes,
                            "a load saves the bytes it read"
                        );
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
    fn states_only_a_file_builds_step_through_the_systems() {
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

    /// Each saved form a load reads (the three file-built states and the older form with its
    /// `frames` and `independent_frames` maps, a frame to return to with no frame among them):
    /// the load saves the bytes upstream's load saved (frozen in `*.loaded.ron` while upstream
    /// existed).
    #[test]
    fn a_load_saves_upstreams_frozen_bytes() {
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
            let system: FloatingLayoutSystem = ron::from_str(text).unwrap();
            assert_eq!(
                ron::to_string(&system).unwrap(),
                frozen.trim_end(),
                "{name}: the load saves upstream's frozen bytes"
            );
        }
    }
}
