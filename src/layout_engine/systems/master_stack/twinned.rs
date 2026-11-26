//! The master-stack layout system: the core's forest decides (layout trees, the group forest's
//! slice 3, the flip, policy (a)) and is saved in upstream's form, byte for byte; upstream's
//! system ([`Upstream`]) is its shadow, stepped on every operation in every build and held to it
//! (`group_twin` says how), and re-seeded from the core on a difference (the deciding side
//! stands).

use objc2_core_foundation::CGRect;
use rift_core::layout::group_forest::{
    self as core, Arrangement, MasterSettings, MasterSide, MasterStack, NewWindowPlacement,
    RegionInsertion,
};
#[cfg(feature = "roc-core")]
use rift_core_roc::RocMasterStack;
#[cfg(feature = "roc-core")]
use rift_core_roc::group_forest::{Answer, ForestEvent as F, MasterStackEvent as E};
use serde::{Deserialize, Serialize};

use super::Upstream;
use crate::actor::app::{WindowId, pid_t};
use crate::common::collections::HashMap;
use crate::common::config::{
    GapSettings, MasterStackNewWindowPlacement, MasterStackSettings, MasterStackSide,
    StackLineReserve, WindowInsertionPoint,
};
use crate::layout_engine::engine::GroupContainerInfo;
#[cfg(feature = "roc-core")]
use crate::layout_engine::systems::bsp::roc_tree as bsp_roc;
#[cfg(feature = "roc-core")]
use crate::layout_engine::systems::group_roc::{self as roc, RocForest};
use crate::layout_engine::systems::group_stored::{Saved, StoredMasterStack, upstream_tiles};
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

pub(in crate::layout_engine::systems) type CoreMasterStack =
    MasterStack<LayoutId, NodeId, WindowId>;

const SYSTEM: &str = "master-stack";

/// The master-stack layout system: the core's forest decides, and upstream's system is stepped
/// beside it. Loaded into the core (`group_stored`: the saved forest parsed, the legacy forms'
/// regions made, what only a file holds normalized), saved from the core in the same form.
#[derive(Clone, Debug, Deserialize)]
#[serde(from = "StoredMasterStack")]
pub struct MasterStackLayoutSystem {
    core: CoreMasterStack,
    /// Under `roc-core`, the deciding forest (the core's is then its shadow).
    #[cfg(feature = "roc-core")]
    roc: RocForest<RocMasterStack>,
    /// Boxed: the system is one variant of `LayoutSystemKind`.
    shadow: Box<Shadow>,
}

/// Upstream's system and how many operations stepped it.
#[derive(Clone, Debug)]
struct Shadow {
    upstream: Upstream,
    steps: u64,
}

/// The core's forest as upstream's master-stack system saves itself (its forest as `inner`).
#[derive(Serialize)]
#[serde(rename = "Upstream")]
struct SavedMasterStack {
    inner: Saved,
}

impl Serialize for MasterStackLayoutSystem {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        SavedMasterStack {
            inner: Saved::of(self.core.forest()),
        }
        .serialize(serializer)
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

/// The core's settings as upstream holds them, over `settings` (the fields the core does not
/// hold kept).
fn upstream_settings(core: MasterSettings, settings: &MasterStackSettings) -> MasterStackSettings {
    let orientation = |arrangement: Arrangement| match arrangement {
        Arrangement::Derived => None,
        Arrangement::Set(core::Orientation::Horizontal) => Some(Orientation::Horizontal),
        Arrangement::Set(core::Orientation::Vertical) => Some(Orientation::Vertical),
    };
    let mut settings = settings.clone();
    settings.base.window_insertion_point = match core.insertion {
        RegionInsertion::NextToSelection => Some(WindowInsertionPoint::NextToSelection),
        RegionInsertion::EndOfTree => Some(WindowInsertionPoint::EndOfTree),
        RegionInsertion::Front => None,
    };
    settings.master_ratio = core.ratio;
    settings.master_count = core.count;
    settings.master_side = match core.side {
        MasterSide::Left => MasterStackSide::Left,
        MasterSide::Right => MasterStackSide::Right,
        MasterSide::Top => MasterStackSide::Top,
        MasterSide::Bottom => MasterStackSide::Bottom,
    };
    settings.new_window_placement = match core.placement {
        NewWindowPlacement::Master => MasterStackNewWindowPlacement::Master,
        NewWindowPlacement::Stack => MasterStackNewWindowPlacement::Stack,
        NewWindowPlacement::Focused => MasterStackNewWindowPlacement::Focused,
    };
    settings.master_arrangement = orientation(core.master);
    settings.stack_arrangement = orientation(core.stack);
    settings
}

/// Upstream's system re-seeded from the core: its forest, and its settings over the old ones.
fn upstream_of(core: &CoreMasterStack, old: &Upstream) -> Upstream {
    Upstream {
        tiles: upstream_tiles(core.forest()),
        settings: upstream_settings(core.settings(), &old.settings),
    }
}

impl From<StoredMasterStack> for MasterStackLayoutSystem {
    /// A saved system: the core built from the loaded forest, with the settings a load starts
    /// with (none are saved: the default, until rift applies its config); upstream's system,
    /// the shadow, re-seeded from the core.
    fn from(stored: StoredMasterStack) -> Self {
        let settings = MasterStackSettings::default();
        let core = CoreMasterStack::from_parts(
            stored.migrated(&settings).normalized(SYSTEM).forest(),
            core_settings(&settings),
        );
        let upstream = Upstream {
            tiles: upstream_tiles(core.forest()),
            settings,
        };
        Self {
            #[cfg(feature = "roc-core")]
            roc: RocForest::seeded(&core),
            shadow: Box::new(Shadow { upstream, steps: 0 }),
            core,
        }
    }
}

impl From<Upstream> for MasterStackLayoutSystem {
    /// A new system (upstream's, empty, with its settings).
    fn from(upstream: Upstream) -> Self {
        let core = core_of(&upstream);
        Self {
            #[cfg(feature = "roc-core")]
            roc: RocForest::seeded(&core),
            shadow: Box::new(Shadow { upstream, steps: 0 }),
            core,
        }
    }
}

impl Default for MasterStackLayoutSystem {
    fn default() -> Self { Upstream::default().into() }
}

impl MasterStackLayoutSystem {
    pub fn new(settings: MasterStackSettings) -> Self { Upstream::new(settings).into() }

    pub fn update_settings(&mut self, settings: MasterStackSettings) {
        self.core.update_settings(core_settings(&settings));
        self.shadow.upstream.update_settings(settings);
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "update_settings",
            E::Settings(self.core.settings()),
            Answer::Done,
        );
        self.stepped("update_settings", &(), &());
    }

    pub fn adjust_master_ratio(&mut self, layout: LayoutId, delta: f64) {
        self.shadow.upstream.adjust_master_ratio(layout, delta);
        self.core.adjust_master_ratio(delta);
        #[cfg(feature = "roc-core")]
        self.roc_stepped("adjust_master_ratio", E::AdjustRatio(delta), Answer::Done);
        self.stepped("adjust_master_ratio", &(), &());
    }

    pub fn adjust_master_count(&mut self, layout: LayoutId, delta: i32) {
        self.shadow.upstream.adjust_master_count(layout, delta);
        self.core.adjust_master_count(delta);
        #[cfg(feature = "roc-core")]
        self.roc_stepped("adjust_master_count", E::AdjustCount(delta), Answer::Done);
        self.stepped("adjust_master_count", &(), &());
    }

    pub fn promote_to_master(&mut self, layout: LayoutId) {
        self.shadow.upstream.promote_to_master(layout);
        self.core.promote_to_master(layout);
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "promote_to_master",
            E::Promote(roc::layout_key(layout)),
            Answer::Done,
        );
        self.stepped("promote_to_master", &(), &());
    }

    pub fn swap_master_stack(&mut self, layout: LayoutId) {
        self.shadow.upstream.swap_master_stack(layout);
        self.core.swap_master_stack(layout);
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "swap_master_stack",
            E::SwapMasterStack(roc::layout_key(layout)),
            Answer::Done,
        );
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
            self.core.forest(),
            (
                &self.shadow.upstream.tiles,
                Some(self.shadow.upstream.regions(layout)),
            ),
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

    /// rift-cli's drawing of the layout: the core's (upstream's held to it on every call).
    pub(crate) fn draw_tree(&self, layout: LayoutId) -> String {
        let core = self.core.forest().draw(layout);
        let core = self.viewed("draw_tree", self.shadow.upstream.draw_tree(layout), core);
        #[cfg(feature = "roc-core")]
        let core = {
            let _ = core;
            self.roc.draw(layout, self.core.forest().rows(layout))
        };
        core
    }

    /// rift-cli's container view of the layout: the core's (upstream's held to it on every
    /// call).
    pub(crate) fn container_tree(&self, layout: LayoutId) -> rift_protocol::ContainerTreeNode {
        let view = self.core.forest().container_view(layout);
        #[cfg(feature = "roc-core")]
        let view = self.roc.container_view(layout, view);
        let core = container_node(view);
        self.viewed(
            "container_tree",
            self.shadow.upstream.container_tree(layout),
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
        let core = self.answered(
            "can_move_window",
            self.shadow.upstream.can_move_window(layout, window, direction),
            self.core.can_move_window(layout, window, core_direction(direction)),
        );
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

    /// Upstream's system re-seeded from the core, as after a divergence.
    #[cfg(test)]
    fn reseed_shadow(&mut self) {
        self.shadow.upstream = upstream_of(&self.core, &self.shadow.upstream);
    }

    /// The Rust forest re-seeded from the deciding Roc forest's view, as after a divergence.
    #[cfg(all(test, feature = "roc-core"))]
    fn reseed_rust_from_roc(&mut self) {
        use crate::layout_engine::systems::group_roc::Machine;
        self.core = rift_core_roc::RocMasterStack::core_of(&self.roc.view());
    }

    /// Upstream's system saved as upstream saves it (the core's save is held to it).
    #[cfg(test)]
    fn upstream_saved(&self) -> String {
        ron::to_string(&self.shadow.upstream).expect("upstream's system is written")
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
            self.shadow.upstream = upstream_of(&self.core, &self.shadow.upstream);
        }
    }

    fn settings_difference(&self) -> Option<String> {
        let upstream = core_settings(&self.shadow.upstream.settings);
        let core = self.core.settings();
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

impl LayoutSystem for MasterStackLayoutSystem {
    fn has_windows_for_app(&self, layout: LayoutId, pid: pid_t) -> bool {
        let core = self.answered(
            "has_windows_for_app",
            self.shadow.upstream.has_windows_for_app(layout, pid),
            self.core.forest().has_windows_for_app(layout, |window| window.pid == pid),
        );
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
        let core = self.answered(
            "window_participates",
            self.shadow.upstream.window_participates(window),
            self.core.forest().window_participates(window),
        );
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
        let upstream = self.shadow.upstream.set_window_participation(window, participates);
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
        self.stepped("set_window_participation", &upstream, &core);
        core
    }

    fn create_layout(&mut self) -> LayoutId {
        let upstream = self.shadow.upstream.create_layout();
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
        self.stepped("create_layout", &upstream, &core);
        core
    }

    fn contains_layout(&self, layout: LayoutId) -> bool {
        let core = self.answered(
            "contains_layout",
            self.shadow.upstream.contains_layout(layout),
            self.core.forest().contains_layout(layout),
        );
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
        let upstream = self.shadow.upstream.clone_layout(layout);
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
        self.stepped("clone_layout", &upstream, &core);
        core
    }

    fn remove_layout(&mut self, layout: LayoutId) {
        self.shadow.upstream.remove_layout(layout);
        self.core.remove_layout(layout);
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "remove_layout",
            E::Forest(F::RemoveLayout(roc::layout_key(layout))),
            Answer::Done,
        );
        self.stepped("remove_layout", &(), &());
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
        let upstream = self.shadow.upstream.shape(layout, screen, offset, constraints, gaps, line);
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
        let upstream_frames = || {
            self.shadow
                .upstream
                .shadow_frames(layout, screen, offset, constraints, gaps, line)
        };
        // Under `roc-core` the Roc forest's own shape through the Roc geometry decides.
        #[cfg(feature = "roc-core")]
        return crate::layout_engine::core_geometry::decide_group_tree(
            SYSTEM,
            core,
            shadow,
            upstream_frames,
            || self.roc.frames(layout, pass(screen, gaps, offset, line), constraints),
        );
        #[cfg(not(feature = "roc-core"))]
        crate::layout_engine::core_geometry::decide_traditional(
            SYSTEM,
            core,
            shadow,
            upstream_frames,
        )
    }

    fn selected_window(&self, layout: LayoutId) -> Option<WindowId> {
        let core = self.answered(
            "selected_window",
            self.shadow.upstream.selected_window(layout),
            self.core.forest().selected_window(layout),
        );
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
        let core = self.answered(
            "selection_contains_window",
            self.shadow.upstream.selection_contains_window(layout, window),
            self.core.forest().selection_contains_window(layout, window),
        );
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
        let core = self.answered(
            "window_in_direction",
            self.shadow.upstream.window_in_direction(layout, direction),
            self.core.forest().window_in_direction(layout, core_direction(direction)),
        );
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
        let core = self.answered(
            "contains_window",
            self.shadow.upstream.contains_window(layout, window),
            self.core.forest().contains_window(layout, window),
        );
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
        let core = self.answered(
            "has_any_fullscreen_node",
            self.shadow.upstream.has_any_fullscreen_node(layout),
            self.core.forest().has_any_fullscreen_node(layout),
        );
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
        let core = self.answered(
            "parent_of_selection_is_stacked",
            self.shadow.upstream.parent_of_selection_is_stacked(layout),
            self.core.forest().parent_of_selection_is_stacked(layout),
        );
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
        let core = self.answered(
            "windows",
            self.shadow.upstream.windows(layout, query),
            self.core.forest().windows(layout, core_query(query)),
        );
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
        let upstream = self.shadow.upstream.focus(layout, focus);
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
        self.stepped("focus", &focus_words(&upstream), &focus_words(&core));
        core
    }

    fn update(&mut self, change: WindowChange<'_>) {
        #[cfg(feature = "roc-core")]
        let event = match change {
            WindowChange::Add(layout, window, _) => {
                E::AddWindow(roc::layout_key(layout), roc::window(window))
            }
            WindowChange::Remove(window) | WindowChange::RemoveAndRebalance(window) => {
                E::RemoveWindow(roc::window(window))
            }
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
        self.shadow.upstream.update(change);
        #[cfg(feature = "roc-core")]
        self.roc_stepped("update", event, Answer::Done);
        self.stepped("update", &(), &());
    }

    fn edit_layout(&mut self, layout: LayoutId, edit: LayoutEdit) -> EditResult {
        let upstream = self.shadow.upstream.edit_layout(layout, edit);
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
        self.stepped("edit_layout", &edit_words(&upstream), &edit_words(&core));
        core
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
        let saved = ron::to_string(system).expect("written");
        assert_eq!(
            saved,
            system.upstream_saved(),
            "the core saves as upstream's system"
        );
        let text = drawn::with_saved_only_states(rng, &saved);
        let mut loaded: MasterStackLayoutSystem = ron::from_str(&text).expect("and read back");
        assert_eq!(
            ron::to_string(&loaded).unwrap(),
            text,
            "a reload saves the same bytes"
        );
        loaded.update_settings(system.shadow.upstream.settings.clone());
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
                            let settings = drawn_settings(rng, &system.shadow.upstream.settings);
                            system.update_settings(settings);
                        }
                        1 => system.adjust_master_ratio(layout, rng.length(-0.5, 0.5)),
                        2 => system.adjust_master_count(layout, rng.below(5) as i32 - 2),
                        3 => system.promote_to_master(layout),
                        _ => system.swap_master_stack(layout),
                    }
                    if rng.chance(10) {
                        system.reseed_shadow();
                    }
                    #[cfg(feature = "roc-core")]
                    if rng.chance(4) {
                        system.reseed_rust_from_roc();
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

    fn w(idx: u32) -> WindowId { WindowId::new(1, idx) }

    /// A system made by operations as rift saves it: a group in the stack region, the master
    /// resized, a window filling the gaps, a cloned layout, a vacant layout slot, a freed leaf.
    fn saved_system() -> (MasterStackLayoutSystem, LayoutId) {
        use crate::layout_engine::{ResizeOrientation, WindowDropAction};
        let mut system = MasterStackLayoutSystem::default();
        let layout = system.create_layout();
        for window in [w(1), w(2), w(3), w(4), w(5)] {
            system.update(WindowChange::Add(layout, window, None));
        }
        system
            .edit_layout(layout, LayoutEdit::Drop {
                source: w(3),
                target: w(2),
                action: WindowDropAction::Stack,
            })
            .discard();
        assert!(system.focus(layout, Focus::Select(w(5))).handled);
        system
            .edit_layout(layout, LayoutEdit::Resize {
                amount: 0.1,
                orientation: ResizeOrientation::Horizontal,
            })
            .discard();
        assert!(system.focus(layout, Focus::Select(w(4))).handled);
        system
            .edit_layout(layout, LayoutEdit::Fullscreen { within_gaps: true })
            .discard();
        system.clone_layout(layout);
        let removed = system.create_layout();
        system.remove_layout(removed);
        system.update(WindowChange::Remove(w(1)));
        (system, layout)
    }

    /// `master_stack_saved.ron`, written by the core while its bytes were held to upstream's:
    /// the operations save to it, it loads and saves back byte for byte, and the loaded forest
    /// allocates its next layout and node where the saved slotmaps do.
    #[test]
    fn the_saved_fixture_is_what_the_operations_write_and_loads_back() {
        use slotmap::Key;
        let file = include_str!("master_stack_saved.ron").trim_end();
        let (system, layout) = saved_system();
        assert_eq!(ron::to_string(&system).unwrap(), file);
        assert_eq!(system.upstream_saved(), file);
        let mut loaded: MasterStackLayoutSystem = ron::from_str(file).unwrap();
        assert_eq!(ron::to_string(&loaded).unwrap(), file);
        assert_eq!(loaded.draw_tree(layout), system.draw_tree(layout));
        // The vacant slots are taken as the saved slotmaps' free lists give them (upstream's
        // load, while it exists, allocating alike).
        let mut upstream: Upstream = ron::from_str(file).unwrap();
        // No settings are saved: a load starts with the default ones.
        assert_eq!(loaded.core.settings(), core_settings(&upstream.settings));
        assert_eq!(
            loaded.core.settings(),
            core_settings(&MasterStackSettings::default())
        );
        let next = loaded.create_layout();
        assert_eq!(next, upstream.create_layout());
        loaded.update(WindowChange::Add(next, w(9), None));
        upstream.update(WindowChange::Add(next, w(9), None));
        assert_eq!(loaded.container_tree(next), upstream.container_tree(next));
        assert_eq!(next.data().as_ffi(), (3 << 32) | 3, "the vacant layout slot");
        let tree = loaded.container_tree(next);
        let ids = (
            tree.node_id,
            tree.children[0].node_id,
            tree.children[0].children[0].node_id,
        );
        assert_eq!(
            ids,
            ((3 << 32) | 21, (3 << 32) | 20, (3 << 32) | 18),
            "the root, the master region and the leaf in vacant node slots, freed last first"
        );
    }

    /// The previous forms (upstream's load, while it exists, loading each alike): the regions
    /// saved beside the forest, or not at all, the root holding two containers (taken as master
    /// and stack) or anything else (moved into new region containers by the master count, the
    /// ratio applied, the selected window selected again). Settings are not saved: each loads
    /// with the default ones.
    #[test]
    fn the_previous_forms_load_into_regions() {
        let saved = include_str!("master_stack_saved.ron").trim_end();
        let forms = [
            (
                "regions beside the forest",
                include_str!("master_stack_legacy_regions.ron").trim_end(),
                include_str!("master_stack_legacy_regions_loaded.ron").trim_end(),
            ),
            (
                "two containers",
                include_str!("master_stack_legacy_containers.ron").trim_end(),
                saved,
            ),
            (
                "a traditional forest",
                include_str!("master_stack_legacy_tiles.ron").trim_end(),
                include_str!("master_stack_legacy_tiles_loaded.ron").trim_end(),
            ),
        ];
        for (form, file, expected) in forms {
            let loaded: MasterStackLayoutSystem = ron::from_str(file).unwrap();
            let upstream: Upstream = ron::from_str(file).unwrap();
            let upstream = ron::to_string(&upstream).unwrap();
            assert_eq!(
                ron::to_string(&loaded).unwrap(),
                upstream,
                "{form}: as upstream's load"
            );
            assert_eq!(ron::to_string(&loaded).unwrap(), expected, "{form}");
        }
    }

    /// A master and a stack region holding a window and a group of two (the group's second
    /// selected; one window left out).
    fn regions_with_a_group() -> (MasterStackLayoutSystem, LayoutId) {
        use crate::layout_engine::WindowDropAction;
        let mut system = MasterStackLayoutSystem::default();
        let layout = system.create_layout();
        for window in [w(1), w(2), w(3), w(4)] {
            system.update(WindowChange::Add(layout, window, None));
        }
        system
            .edit_layout(layout, LayoutEdit::Drop {
                source: w(3),
                target: w(2),
                action: WindowDropAction::Stack,
            })
            .discard();
        assert!(system.focus(layout, Focus::Select(w(3))).handled);
        system.set_window_participation(w(1), false);
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

    fn frames(system: &MasterStackLayoutSystem, layout: LayoutId) -> Vec<(WindowId, CGRect)> {
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
        system: &MasterStackLayoutSystem,
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

    /// rift-cli's drawing of a master-stack layout (`master_stack_drawn.txt`: as upstream's system
    /// drew it) and its container view: the root and its regions with their roles, the group, each node's weight, the
    /// selection, the window left out still listed.
    #[test]
    fn rift_cli_draws_and_lists_a_master_stack_layout() {
        let (system, layout) = regions_with_a_group();
        use crate::layout_engine::LayoutKind;
        assert_eq!(
            system.draw_tree(layout),
            include_str!("master_stack_drawn.txt"),
            "the drawing (ending with its own newline)"
        );
        let tree = system.container_tree(layout);
        let node = |n: &rift_protocol::ContainerTreeNode| {
            (
                n.node_id,
                n.layout_kind,
                n.window_id,
                n.weight,
                n.is_selected,
                n.role.clone(),
            )
        };
        let id = |n: u64| (1 << 32) | n;
        let role = |r: &str| Some(r.to_owned());
        assert_eq!(
            node(&tree),
            (id(1), Some(LayoutKind::Horizontal), None, None, false, None)
        );
        let regions: Vec<_> = tree.children.iter().map(node).collect();
        assert_eq!(regions, [
            (
                id(2),
                Some(LayoutKind::Vertical),
                None,
                Some(f64::from(1.2_f32)),
                false,
                role("master")
            ),
            (
                id(3),
                Some(LayoutKind::Vertical),
                None,
                Some(f64::from(0.799_999_95_f32)),
                false,
                role("stack")
            ),
        ]);
        assert_eq!(
            node(&tree.children[0].children[0]),
            (id(7), None, Some(w(4).into()), Some(1.0), false, None)
        );
        let stack: Vec<_> = tree.children[1].children.iter().map(node).collect();
        assert_eq!(stack, [
            (
                id(8),
                Some(LayoutKind::VerticalStack),
                None,
                Some(1.0),
                false,
                None
            ),
            (id(4), None, Some(w(1).into()), Some(1.0), false, None),
        ]);
        let group: Vec<_> = tree.children[1].children[0].children.iter().map(node).collect();
        assert_eq!(group, [
            (id(5), None, Some(w(2).into()), Some(1.0), false, None),
            (id(6), None, Some(w(3).into()), Some(1.0), true, None),
        ]);
    }

    /// The group bar shows the group's windows over the group, the selected one counted among
    /// them, the same on the selection path; while the selected window fills the tiling area
    /// or the screen, the selection path shows none.
    #[test]
    fn the_group_bar_shows_the_group_windows_taking_part() {
        use crate::layout_engine::LayoutKind;
        let (mut system, layout) = regions_with_a_group();
        let shown = vec![(
            (1 << 32) | 8,
            LayoutKind::VerticalStack,
            rect(587.0, 35.0, 373.0, 760.0),
            2,
            1,
            vec![w(2), w(3)],
        )];
        assert_eq!(bars(&system, layout, false), shown);
        assert_eq!(bars(&system, layout, true), shown);
        for within_gaps in [true, false] {
            system.edit_layout(layout, LayoutEdit::Fullscreen { within_gaps }).discard();
            assert_eq!(bars(&system, layout, false), shown);
            assert!(bars(&system, layout, true).is_empty());
        }
    }

    /// Each fill's frames: tiled (the master, the group's windows offset along it), the
    /// selected window filling the tiling area, then the screen (the window left out placed
    /// nowhere).
    #[test]
    fn a_master_stack_layout_places_each_fill() {
        let (mut system, layout) = regions_with_a_group();
        let master = (w(4), rect(20.0, 35.0, 559.0, 760.0));
        let first = (w(2), rect(587.0, 35.0, 373.0, 748.0));
        assert_eq!(frames(&system, layout), vec![
            master,
            first,
            (w(3), rect(587.0, 47.0, 373.0, 748.0))
        ]);
        system
            .edit_layout(layout, LayoutEdit::Fullscreen { within_gaps: true })
            .discard();
        assert_eq!(frames(&system, layout), vec![
            master,
            first,
            (w(3), rect(20.0, 35.0, 940.0, 760.0))
        ]);
        system
            .edit_layout(layout, LayoutEdit::Fullscreen { within_gaps: false })
            .discard();
        assert_eq!(frames(&system, layout), vec![
            master,
            first,
            (w(3), rect(0.0, 25.0, 1000.0, 800.0))
        ]);
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
                    system.core.forest(),
                    &system.shadow.upstream.tiles,
                ));
            }
            let view_each = each(started, rounds);
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(upstream_of(&system.core, &system.shadow.upstream));
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
                black_box(system.core.forest().shape(
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
