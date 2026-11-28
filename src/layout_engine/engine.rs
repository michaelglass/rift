use std::sync::Arc;

use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use rift_core::display_memory::{DisplayEvent, LastSpace};
use rift_core::evictions::EvictionEvent;
use rift_core::focus::{
    Committed, CycleAsk, EdgeAsk, Eligible, EntryAsk, FallbackAsk, FocusAsk, FocusRecord,
    PastTheEdge, RecordAsk, SpaceAsk, WorkspaceFocus,
};
use rift_core::native_memory::{Candidate, Current, Held as _, MemoryAnswer, MemoryEvent};
use rift_core::workspaces::{Arrival, Moved, SourceAfter, Switch};
use rift_protocol::{FloatingWindowSize, FloatingWindowSizePreset, ToggleWindowFloatingOptions};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::{Direction, FloatingManager, LayoutId, LayoutSystemKind, ResizeOrientation};
#[cfg(test)]
use crate::actor::app::AppInfo;
use crate::actor::app::{WindowId, pid_t};
use crate::common::collections::HashMap;
use crate::common::config::{LayoutMode, LayoutSettings, WorkspaceSelector};
use crate::layout_engine::LayoutSystem;
use crate::layout_engine::floating::FloatingFullscreenKind;
use crate::layout_engine::systems::{
    Focus, GroupEdit, LayoutEdit, WindowChange, WindowLayoutConstraints, WindowQuery,
};
use crate::model::app_rules::{AppRuleFrame, AppRuleOutcome, AppRuleResize, AppRuleWorkspaceFocus};
use crate::model::virtual_workspace::{VirtualWorkspaceId, WorkspaceContext, WorkspaceStore};
use crate::model::window_store::{Windows, WindowsMut};
use crate::model::{
    AppRuleEffects, AppRuleEngine, AppRuleResult, WindowRuleContext, WindowVisibility,
};
use crate::sys::screen::SpaceId;

mod persistence;

use persistence::PersistenceState;
pub use persistence::{RestoreReport, RestoreRequest, RestoreScope, RestoreSource};
pub use rift_protocol::LayoutCommand;

const SMART_FLOATING_WIDTH_RATIO: f64 = 0.8;
const SMART_FLOATING_HEIGHT_RATIO: f64 = 0.93;

#[derive(Clone, Copy)]
pub(crate) struct DropPreview {
    pub frame: CGRect,
    pub same_slot: bool,
}

impl DropPreview {
    pub(crate) fn action(
        self,
        action: crate::layout_engine::WindowDropAction,
        center: crate::common::config::MouseDropAction,
    ) -> crate::layout_engine::WindowDropAction {
        if self.same_slot && matches!(action, crate::layout_engine::WindowDropAction::Insert(_)) {
            center.into()
        } else {
            action
        }
    }
}

fn requested_floating_frame(
    mut frame: CGRect,
    screen: CGRect,
    center: bool,
    size: Option<FloatingWindowSize>,
) -> CGRect {
    if let Some(size) = size {
        frame.size = match size {
            FloatingWindowSize::Dimensions { w, h } => CGSize::new(w.get(), h.get()),
            FloatingWindowSize::Preset(FloatingWindowSizePreset::Smart) => CGSize::new(
                screen.size.width * SMART_FLOATING_WIDTH_RATIO,
                screen.size.height * SMART_FLOATING_HEIGHT_RATIO,
            ),
        };
    }
    if center {
        frame.origin = CGPoint::new(
            screen.mid().x - frame.size.width / 2.0,
            screen.mid().y - frame.size.height / 2.0,
        );
    }
    frame
}

#[derive(Debug, Clone)]
pub struct GroupContainerInfo {
    pub node_id: crate::model::tree::NodeId,
    pub container_kind: super::LayoutKind,
    pub frame: CGRect,
    pub total_count: usize,
    pub selected_index: usize,
    pub window_ids: Vec<crate::actor::app::WindowId>,
}

#[derive(Debug, Clone)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct WindowLayoutInfo {
    pub(crate) window_id: WindowId,
    pub(crate) title: Option<String>,
    pub(crate) ax_role: Option<String>,
    pub(crate) ax_subrole: Option<String>,
    pub(crate) is_resizable: bool,
    pub(crate) current_size: CGSize,
    pub(crate) min_size: Option<CGSize>,
    pub(crate) max_size: Option<CGSize>,
}

#[derive(Debug, Clone)]
pub struct ResolvedWindow {
    pub(crate) info: WindowLayoutInfo,
    pub(crate) effects: AppRuleEffects,
}

#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum LayoutEvent {
    /// Admit or update one window without making claims about any sibling window.
    WindowObserved(SpaceId, ResolvedWindow),
    #[cfg(test)]
    #[allow(private_interfaces)]
    TestWindowsObserved(SpaceId, Vec<WindowLayoutInfo>, Option<AppInfo>),
    AppClosed(pid_t),
    WindowAdded(SpaceId, WindowId),
    WindowRemoved(WindowId),
    WindowRemovedPreserveFloating(WindowId),
    /// Suspend projection only; lifecycle state is owned by the window store.
    WindowSuspended(WindowId),
    WindowFocused(SpaceId, WindowId),
    WindowResized {
        wid: WindowId,
        old_frame: CGRect,
        new_frame: CGRect,
        screens: Arc<[(SpaceId, CGRect, Option<String>)]>,
    },
    SpaceExposed(SpaceId, CGSize),
}

#[cfg(test)]
impl LayoutEvent {
    fn windows_observed(
        space: SpaceId,
        _pid: pid_t,
        windows: Vec<WindowLayoutInfo>,
        app: Option<AppInfo>,
    ) -> Self {
        Self::TestWindowsObserved(space, windows, app)
    }
}

#[must_use]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EventResponse {
    /// Whether handling the request changed layout-engine state.
    #[serde(default)]
    pub changed: bool,
    pub raise_windows: Vec<WindowId>,
    pub focus_window: Option<WindowId>,
    pub boundary_hit: Option<Direction>,
    /// Destination presentation context, resolved once by the command.
    #[serde(default)]
    pub workspace_space: Option<SpaceId>,
    /// Logical relocations needing native placement before arrangement.
    #[serde(default)]
    pub transferred_windows: Vec<WindowId>,
    /// Space edited by a command whose target may differ from the command display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_space: Option<SpaceId>,
}

#[must_use]
pub struct LayoutEventOutcome {
    pub response: EventResponse,
    pub(crate) app_rules: AppRuleOutcome,
}

impl std::ops::Deref for LayoutEventOutcome {
    type Target = EventResponse;

    fn deref(&self) -> &Self::Target { &self.response }
}

/// A scrolling layout's pass given every window's constraints (the core's records, in the
/// engine's shape) and the core's revision of them.
fn prepare_scrolling(
    constraints: &HashMap<WindowId, WindowLayoutConstraints>,
    revision: u64,
    system: &mut LayoutSystemKind,
    layout: LayoutId,
    screen: CGRect,
    gaps: &crate::common::config::GapSettings,
    shadow: crate::layout_engine::Shadow,
) {
    if let LayoutSystemKind::Scrolling(scrolling) = system {
        scrolling.prepare(layout, screen, constraints, Some(revision), gaps, shadow);
    }
}

enum WindowRelocation {
    Workspace,
    Observed,
    Drop(Option<LayoutSystemKind>),
    Directional(Direction),
}

pub struct LayoutEngine {
    /// The floating windows, their frames and the focused window (engine slice E2).
    floats: super::floating::EngineFloats,
    app_rules: AppRuleEngine,
    /// Every window's size constraints and each app's last reported minimum (engine slice E4).
    constraints: crate::roc_twin::TwinnedConstraints,
    workspaces: WorkspaceStore,
    layout_settings: LayoutSettings,
    /// Each Space's display and each display's last Space (engine slice E3).
    displays: crate::roc_twin::TwinnedDisplays,
    persistence: PersistenceState,
    /// Legacy strip commands report boundary excess; workspace arbitration lives here.
    scroll_boundary: Option<((VirtualWorkspaceId, LayoutId, Direction), f64)>,
    // Display-relocation recovery only; temporary absence never snapshots topology.
    /// Workspace arrangements kept for a departed display's return, and the reconcile batch
    /// (engine slice E3c).
    native: crate::roc_twin::TwinnedNative<NativeSnapshot>,
    // Windows a display departure moved to another display, by where they came from.
    /// Where a display departure moved windows from (engine slice E3b).
    evicted: crate::roc_twin::TwinnedEvictions,
    /// Each Space's layout passes, deciding which run the layout systems' shadows.
    shadow_passes: super::core_geometry::ShadowPasses,
}

/// A layout engine and the topology its workspaces start with, joined into the stores by
/// [`crate::model::Stores::joining`]: an engine loaded from a file brings the file's topology, a
/// new one none.
pub struct StartingLayout {
    pub(crate) engine: LayoutEngine,
    pub(crate) topology: crate::model::virtual_workspace::SavedTopology,
}

impl StartingLayout {
    /// A new engine, with no workspaces yet.
    pub fn new(
        virtual_workspace_config: &crate::common::config::VirtualWorkspaceSettings,
        layout_settings: &LayoutSettings,
    ) -> StartingLayout {
        StartingLayout {
            engine: LayoutEngine::new(virtual_workspace_config, layout_settings),
            topology: Default::default(),
        }
    }
}

/// A workspace's arrangement kept for a departed display's return (engine slice E3c): its layout
/// system's state as the cores hold it (never a live system), its active layout and the size it
/// was laid out for. Its windows are computed from the value.
#[derive(Clone, Debug)]
struct NativeSnapshot {
    system: super::systems::SystemSnapshot,
    layout: LayoutId,
    size: super::workspaces::Size,
}

impl rift_core::native_memory::Held<WindowId> for NativeSnapshot {
    fn windows(&self) -> Vec<WindowId> { self.system.windows(self.layout) }
}

impl NativeSnapshot {
    /// `workspace`'s arrangement, if it is laid out in a system that keeps one.
    fn of(workspace: &crate::model::VirtualWorkspace) -> Option<Self> {
        let super::workspaces::WorkspaceLayoutState::Laid { layout, size } = workspace.layout_state
        else {
            return None;
        };
        Some(NativeSnapshot {
            system: workspace.layout_system.snapshot()?,
            layout,
            size,
        })
    }

    /// The snapshot after `change`, as its system makes it (a scrolling camera's answer kept).
    fn updated(&self, change: WindowChange<'_>) -> Self {
        let mut live = self.system.clone().restored();
        live.update(change);
        let system = live.snapshot().expect("a snapshot's system keeps one");
        NativeSnapshot { system, ..self.clone() }
    }

    /// The live system and layout state it holds.
    fn restored(&self) -> (LayoutSystemKind, super::workspaces::WorkspaceLayoutState) {
        let state = super::workspaces::WorkspaceLayoutState::Laid {
            layout: self.layout,
            size: self.size,
        };
        (self.system.clone().restored(), state)
    }
}

pub(crate) struct WorkspaceLayoutQuerySnapshot {
    pub workspace_id: VirtualWorkspaceId,
    pub workspace_index: usize,
    pub is_active: bool,
    pub mode: LayoutMode,
    pub selected_window: Option<WindowId>,
    pub container_tree: rift_protocol::ContainerTreeNode,
}

impl LayoutEngine {
    pub fn focused_window(&self) -> Option<WindowId> { self.floats.focused() }

    pub(crate) fn apply_window_drop(
        &mut self,
        windows: Windows<'_>,
        request: crate::layout_engine::WindowDropRequest,
    ) -> bool {
        if self.active_layout_mode_at(windows, request.space) == LayoutMode::Floating {
            return false;
        }
        let Some((workspace, layout)) =
            self.workspaces.reading(windows).active_layout_for_space(request.space)
        else {
            return false;
        };
        self.forget_native_layout_for_window_id(request.source);
        self.native.step(MemoryEvent::Forgotten(workspace));
        self.workspaces[workspace]
            .layout_system
            .edit(layout, LayoutEdit::Drop {
                source: request.source,
                target: request.target,
                action: request.action,
            })
            .changed()
    }

    /// Validate a prospective drop on a workspace-local tree before touching membership.
    /// The regular relocation and target-drop primitives perform the committed operation.
    pub(crate) fn relocate_window_with_drop(
        &mut self,
        windows: &mut WindowsMut<'_>,
        drop: &crate::layout_engine::WorkspaceDropRequest,
    ) -> bool {
        let Some(source) = windows.read().workspace_for_window(drop.window) else {
            return false;
        };
        if !self.workspaces.workspaces.contains_key(source) {
            return false;
        }
        let destination = drop.workspace;
        let Some(space) = self.workspaces.reading(windows.read()).workspace_space(destination)
        else {
            return false;
        };
        let floating = self.is_window_floating(drop.window);
        let Some(layout) =
            self.workspaces.reading(windows.read()).active_layout(space, destination)
        else {
            return false;
        };
        let different = source != destination;
        let prospective = if floating {
            let Some(frame) = drop.frame else { return false };
            if ![
                frame.origin.x,
                frame.origin.y,
                frame.size.width,
                frame.size.height,
            ]
            .iter()
            .all(|v| v.is_finite())
                || frame.size.width <= 0.0
                || frame.size.height <= 0.0
            {
                return false;
            }
            if drop.target.is_some()
                || (!different
                    && self.get_floating_position(destination, drop.window) == Some(frame))
            {
                return false;
            }
            None
        } else {
            if drop.frame.is_some() {
                return false;
            }

            if let Some((target, action)) = drop.target {
                if (target == drop.window
                    && !matches!(
                        action,
                        crate::layout_engine::WindowDropAction::Move(_)
                            | crate::layout_engine::WindowDropAction::ConsumeOrExpel(_)
                    ))
                    || windows.read().workspace_for_window(target) != Some(destination)
                    || self.is_window_floating(target)
                {
                    return false;
                }
                // The same prospective layout the preview showed (upstream #709).
                let Some((mut tree, before)) =
                    self.prospective_drop_tree(destination, layout, drop.window, target, action)
                else {
                    return false;
                };
                if before.is_some() && before == tree.window_slot(layout, drop.window) {
                    return false;
                }
                tree.focus(layout, Focus::Select(drop.window)).handled;
                Some(tree)
            } else if different {
                None // Generic relocation already inserts a rendered member.
            } else {
                return false;
            }
        };
        if different {
            if !self.relocate_window_to_workspace(
                windows,
                source,
                (space, destination),
                drop.window,
                WindowRelocation::Drop(prospective),
            ) {
                return false;
            }
        } else if let Some(tree) = prospective {
            self.native.step(MemoryEvent::Forgotten(destination));
            self.workspaces[destination].layout_system = tree;
        }
        if self.floats.focused() == Some(drop.window) {
            self.floats.set_focused(None);
        }
        if let Some(frame) = drop.frame {
            windows.clear_floating_display_geometry(drop.window);
            self.store_floating_position(destination, drop.window, frame);
        }
        self.workspaces.set_last_focused_window(
            &windows.core,
            space,
            destination,
            Some(drop.window),
        );
        true
    }

    /// Return the visible logical tiles eligible for drag targeting.
    /// Hidden members of stacked/grouped containers are excluded.
    pub(crate) fn drop_scene_windows(
        &self,
        windows: Windows<'_>,
        space: SpaceId,
        source: WindowId,
    ) -> Vec<WindowId> {
        if self.active_layout_mode_at(windows, space) == LayoutMode::Floating {
            return Vec::new();
        }
        let Some((workspace, layout)) =
            self.workspaces.reading(windows).active_layout_for_space(space)
        else {
            return Vec::new();
        };
        let system = &self.workspaces[workspace].layout_system;
        let mut windows = system.windows(layout, WindowQuery::Visible);
        for window in system.windows(layout, WindowQuery::Stack(source)) {
            if !windows.contains(&window) {
                windows.push(window);
            }
        }
        windows
    }

    pub(crate) fn drop_action_override(
        &self,
        windows: Windows<'_>,
        space: SpaceId,
    ) -> Option<crate::layout_engine::WindowDropAction> {
        let workspace = self.workspaces.reading(windows).active_workspace(space)?;
        matches!(
            self.workspaces[workspace].layout_system,
            LayoutSystemKind::Stack(_)
        )
        .then_some(crate::layout_engine::WindowDropAction::Swap)
    }

    pub(crate) fn drop_preview_frame(
        &self,
        core: &crate::model::virtual_workspace::CoreWorkspaces,
        space: SpaceId,
        source: WindowId,
        target: WindowId,
        action: crate::layout_engine::WindowDropAction,
        screen: CGRect,
        display_uuid: Option<&str>,
        stack_line: &crate::common::config::StackLineSettings,
    ) -> Option<DropPreview> {
        let (workspace, layout) = self.workspaces.active_layout_in(core, space)?;
        let system = &self.workspaces[workspace].layout_system;
        if !system.window_participates(source)
            || !system.window_participates(target)
            || !system.contains_window(layout, target)
        {
            return None;
        }
        let gaps = self.layout_settings.gaps.effective_for_display(display_uuid);
        let (mut system, before) =
            self.prospective_drop_tree(workspace, layout, source, target, action)?;
        // A drag preview is not a Space's layout pass.
        let shadow = crate::layout_engine::Shadow::outside_a_pass();
        prepare_scrolling(
            &self.constraint_records(),
            self.constraints.revision(),
            &mut system,
            layout,
            screen,
            &gaps,
            shadow,
        );
        let frame = self
            .project_arrangement(&system, layout, screen, &gaps, stack_line.reserve(), shadow)
            .into_iter()
            .find_map(|(window, frame)| (window == source).then_some(frame))?;
        Some(DropPreview {
            frame,
            same_slot: before.is_some() && system.window_slot(layout, source) == before,
        })
    }

    /// The layout a drop of `source` on `target` would leave, and `source`'s slot before it.
    /// A window from elsewhere (no slot) joins the layout first, as the drop adds it. The
    /// preview and the drop both build it here, so a drop lands where it was previewed.
    fn prospective_drop_tree(
        &self,
        workspace: VirtualWorkspaceId,
        layout: LayoutId,
        source: WindowId,
        target: WindowId,
        action: crate::layout_engine::WindowDropAction,
    ) -> Option<(LayoutSystemKind, Option<Vec<usize>>)> {
        let mut tree = self.workspaces[workspace].layout_system.preview_clone()?;
        let before = tree.window_slot(layout, source);
        if before.is_none() {
            tree.update(WindowChange::Add(layout, source, None));
        }
        let changed = tree.edit(layout, LayoutEdit::Drop { source, target, action }).changed();
        changed.then_some((tree, before))
    }

    pub(crate) fn source_move_neighbors(
        &self,
        windows: Windows<'_>,
        space: SpaceId,
        source: WindowId,
    ) -> [bool; 4] {
        let Some((workspace, layout)) =
            self.workspaces.reading(windows).active_layout_for_space(space)
        else {
            return [false; 4];
        };
        let baseline = &self.workspaces[workspace].layout_system;
        let directions = [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ];
        directions.map(|direction| match baseline {
            LayoutSystemKind::Traditional(system) => {
                system.can_move_window(layout, source, direction)
            }
            LayoutSystemKind::Bsp(system) => system.can_move_window(layout, source, direction),
            LayoutSystemKind::MasterStack(system) => {
                system.can_move_window(layout, source, direction)
            }
            LayoutSystemKind::Scrolling(system) => {
                system.can_move_window(layout, source, direction)
            }
            LayoutSystemKind::Stack(_) | LayoutSystemKind::Floating(_) => false,
        })
    }

    /// Resolve an optional workspace index and snapshot its layout for read-only consumers.
    pub(crate) fn query_workspace_layout(
        &self,
        windows: Windows<'_>,
        target: crate::model::virtual_workspace::WorkspaceTarget,
    ) -> Option<WorkspaceLayoutQuerySnapshot> {
        let workspace_id = target.id;
        let workspace = self.workspaces.workspaces.get(workspace_id)?;
        if self.workspaces.reading(windows).workspace_space(workspace_id) != Some(target.space) {
            return None;
        }
        let active = self.workspaces.reading(windows).active_workspace(target.space)?;
        let workspace_index =
            self.workspaces.reading(windows).workspace_index(target.space, workspace_id)?;
        let layout = workspace.active_layout()?;
        let selected_window = workspace.layout_system.selected_window(layout);
        let container_tree = workspace.layout_system.container_tree(layout);

        Some(WorkspaceLayoutQuerySnapshot {
            workspace_id,
            workspace_index,
            is_active: workspace_id == active,
            mode: workspace.layout_mode,
            selected_window,
            container_tree,
        })
    }

    pub(crate) fn calculate_workspace_layout(
        &self,
        windows: Windows<'_>,
        space: SpaceId,
        workspace_id: VirtualWorkspaceId,
        screen: CGRect,
        gaps: &crate::common::config::GapSettings,
        stack_line: crate::common::config::StackLineReserve,
    ) -> Vec<(WindowId, CGRect)> {
        let Some(layout) = self.workspaces.reading(windows).active_layout(space, workspace_id)
        else {
            return Vec::new();
        };
        // A query is not a Space's layout pass.
        self.calculate_arrangement(
            workspace_id,
            layout,
            screen,
            gaps,
            stack_line,
            crate::layout_engine::Shadow::outside_a_pass(),
        )
    }

    fn calculate_arrangement(
        &self,
        workspace_id: VirtualWorkspaceId,
        layout: LayoutId,
        screen: CGRect,
        gaps: &crate::common::config::GapSettings,
        stack_line: crate::common::config::StackLineReserve,
        shadow: crate::layout_engine::Shadow,
    ) -> Vec<(WindowId, CGRect)> {
        self.project_arrangement(
            &self.workspaces[workspace_id].layout_system,
            layout,
            screen,
            gaps,
            stack_line,
            shadow,
        )
    }

    fn project_arrangement(
        &self,
        system: &LayoutSystemKind,
        layout: LayoutId,
        screen: CGRect,
        gaps: &crate::common::config::GapSettings,
        stack_line: crate::common::config::StackLineReserve,
        shadow: crate::layout_engine::Shadow,
    ) -> Vec<(WindowId, CGRect)> {
        if let LayoutSystemKind::Scrolling(scrolling) = system {
            return scrolling.calculate_frames(
                layout,
                screen,
                &self.constraint_records(),
                Some(self.constraints.revision()),
                gaps,
                true,
                shadow,
            );
        }
        system.calculate_layout(
            layout,
            screen,
            self.layout_settings.stack.stack_offset,
            &self.constraint_records(),
            gaps,
            stack_line,
            shadow,
        )
    }

    fn switch_workspace_layout_mode(
        &mut self,
        window_store: Windows<'_>,
        space: SpaceId,
        workspace_id: VirtualWorkspaceId,
        mode: LayoutMode,
    ) -> bool {
        let (current_mode, selected_window, mut window_order) = {
            let Some(workspace) =
                self.workspaces.reading(window_store).workspace_info(space, workspace_id)
            else {
                return false;
            };
            if workspace.layout_mode == mode {
                return false;
            }
            let old_layout = workspace.active_layout();
            let selected =
                old_layout.and_then(|layout| workspace.layout_system.selected_window(layout));
            let mut ordered = old_layout
                .map(|layout| workspace.layout_system.windows(layout, WindowQuery::All))
                .unwrap_or_default();
            // Keep windows hidden by stack/group selection when rebuilding into a new mode.
            let mut hidden_windows: Vec<_> = window_store
                .workspace_windows(workspace_id)
                .into_iter()
                .filter(|wid| !ordered.contains(wid))
                .collect();
            hidden_windows.sort();
            ordered.extend(hidden_windows);
            (workspace.layout_mode, selected, ordered)
        };
        self.forget_native_layout_for_workspace(workspace_id);

        if current_mode == LayoutMode::Floating || mode == LayoutMode::Floating {
            for &wid in &window_order {
                self.floats.remove_floating(wid);
                self.floats.remove_window(wid);
            }
        }
        window_order.retain(|wid| !self.floats.is_floating(*wid));

        if mode == LayoutMode::Scrolling && self.layout_settings.scrolling.preserve_window_sizes {
            for &wid in &window_order {
                self.preserve_scrolling_window_width(window_store, wid);
            }
        }

        let Some(workspace) = self.workspaces.workspaces.get_mut(workspace_id) else {
            return false;
        };
        let new_layout = workspace.replace_layout_mode(mode, &self.layout_settings);

        for wid in window_order {
            workspace.layout_system.update(WindowChange::Add(new_layout, wid, None));
            if window_store.is_temporarily_unavailable(wid) {
                workspace.layout_system.set_window_participation(wid, false);
            }
        }

        if let LayoutSystemKind::Floating(system) = &mut workspace.layout_system {
            system.spread_initial_windows();
        }

        if let Some(selected) = selected_window.filter(|wid| !self.floats.is_floating(*wid)) {
            let _ = workspace.layout_system.focus(new_layout, Focus::Select(selected)).handled;
        }

        if mode == LayoutMode::Scrolling {
            self.sync_scrolling_widths_for_space(window_store, space);
        }

        true
    }

    fn response_for_raised_windows(raise_windows: Vec<WindowId>) -> EventResponse {
        if raise_windows.is_empty() {
            EventResponse::default()
        } else {
            EventResponse {
                changed: true,
                raise_windows,

                ..EventResponse::default()
            }
        }
    }

    fn toggle_orientation_for_system<S: LayoutSystem>(
        system: &mut S,
        layout: LayoutId,
        default_orientation: crate::common::config::StackDefaultOrientation,
    ) -> EventResponse {
        if system.parent_of_selection_is_stacked(layout) {
            let toggled_windows = system
                .edit(layout, LayoutEdit::Group(GroupEdit::Stack(default_orientation)))
                .into_windows();
            return Self::response_for_raised_windows(toggled_windows);
        }
        system.edit(layout, LayoutEdit::ToggleOrientation).discard();
        EventResponse::default()
    }

    fn toggle_stack_for_workspace(
        &mut self,
        workspace_id: VirtualWorkspaceId,
        layout: LayoutId,
        default_orientation: crate::common::config::StackDefaultOrientation,
    ) -> EventResponse {
        let system = &mut self.workspaces[workspace_id].layout_system;
        let selected_window = system.selected_window(layout);
        let unstacked_windows = system
            .edit(
                layout,
                LayoutEdit::Group(GroupEdit::Unstack(default_orientation)),
            )
            .into_windows();
        let raised_windows = if !unstacked_windows.is_empty() {
            unstacked_windows
        } else {
            let stacked_windows = system
                .edit(layout, LayoutEdit::Group(GroupEdit::Stack(default_orientation)))
                .into_windows();
            if !stacked_windows.is_empty() {
                stacked_windows
            } else {
                system.windows(layout, WindowQuery::Visible)
            }
        };

        let mut response = Self::response_for_raised_windows(raised_windows);
        if response.changed
            && let Some(selected_window) = selected_window
            && system.focus(layout, Focus::Select(selected_window)).handled
        {
            response.focus_window = Some(selected_window);
        }
        response
    }

    fn collect_group_containers_for_space(
        &self,
        windows: Windows<'_>,
        space: SpaceId,
        screen: CGRect,
        gaps: &crate::common::config::GapSettings,
        stack_line: crate::common::config::StackLineReserve,
        selection_path_only: bool,
    ) -> Vec<GroupContainerInfo> {
        let Some((ws_id, layout_id)) =
            self.workspaces.reading(windows).active_layout_for_space(space)
        else {
            return Vec::new();
        };
        let stack_offset = self.layout_settings.stack.stack_offset;
        self.workspaces[ws_id].layout_system.collect_group_containers(
            layout_id,
            screen,
            stack_offset,
            &self.constraint_records(),
            gaps,
            stack_line,
            selection_path_only,
        )
    }
}

impl LayoutEngine {
    pub(crate) fn set_layout_settings(&mut self, windows: Windows<'_>, settings: &LayoutSettings) {
        if &self.layout_settings != settings {
            self.apply_layout_settings(windows, settings);
        }
    }

    // Hydrated systems carry saved settings even though the runtime settings
    // field starts at its default. Loading must apply the current configuration.
    fn apply_layout_settings(&mut self, windows: Windows<'_>, settings: &LayoutSettings) {
        self.native.step(MemoryEvent::Cleared);
        self.layout_settings = settings.clone();

        let spaces: HashMap<_, _> = windows.core.catalog().iter().copied().collect();
        for (id, ws) in self.workspaces.workspaces.iter_mut() {
            let display = spaces
                .get(&id)
                .and_then(|space| self.displays.display_of(*space))
                .map(String::as_str);
            ws.apply_layout_settings(settings, display);
        }
    }

    pub(crate) fn update_virtual_workspace_settings(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        settings: &crate::common::config::VirtualWorkspaceSettings,
        plan: crate::model::virtual_workspace::WorkspaceSettingsPlan,
    ) {
        let mode_changed = self.workspaces.settings.mode != settings.mode;
        let previous_modes: HashMap<_, _> = self
            .workspaces
            .reading(window_store.read())
            .catalog()
            .filter_map(|id| {
                let workspace = &self.workspaces[id];
                let view = self.workspaces.reading(window_store.read());
                let index = view.workspace_index(view.workspace_space(id)?, id)?;
                Some((
                    id,
                    self.workspaces.resolve_layout_mode_for_workspace(index, &workspace.name),
                ))
            })
            .collect();
        self.app_rules = AppRuleEngine::new(&settings.app_rules);
        self.workspaces
            .apply_settings(window_store, settings, &self.layout_settings, plan);

        // Changing namespace policy must preserve every existing layout tree.
        let ids: Vec<_> = self.workspaces.reading(window_store.read()).catalog().collect();
        for id in ids {
            let workspace = &self.workspaces[id];
            let view = self.workspaces.reading(window_store.read());
            let Some(space) = view.workspace_space(id) else {
                continue;
            };
            let Some(index) = view.workspace_index(space, id) else {
                continue;
            };
            let desired = self.workspaces.resolve_layout_mode_for_workspace(index, &workspace.name);
            if !mode_changed
                && workspace.layout_mode() != desired
                && previous_modes.get(&id).is_some_and(|previous| *previous != desired)
            {
                self.switch_workspace_layout_mode(window_store.read(), space, id, desired);
            }
        }
        for space in self.workspaces.reading(window_store.read()).initialized_spaces() {
            self.sync_scrolling_widths_for_space(window_store.read(), space);
        }
    }

    pub(crate) fn active_layout_mode_at(&self, windows: Windows<'_>, space: SpaceId) -> LayoutMode {
        self.workspaces
            .reading(windows)
            .active_workspace(space)
            .map(|id| self.workspaces[id].layout_mode())
            .unwrap_or_default()
    }

    pub(crate) fn focus_changes_geometry(&self, windows: Windows<'_>, space: SpaceId) -> bool {
        self.workspaces.reading(windows).active_layout_for_space(space).is_some_and(
            |(workspace, layout)| {
                let system = &self.workspaces[workspace].layout_system;
                matches!(
                    system,
                    LayoutSystemKind::Scrolling(_) | LayoutSystemKind::Floating(_)
                ) || system.parent_of_selection_is_stacked(layout)
            },
        )
    }

    pub(crate) fn layout_specific_animate_settings(
        &self,
        windows: Windows<'_>,
        space: SpaceId,
    ) -> Option<bool> {
        if let Some(ws_id) = self.workspaces.reading(windows).active_workspace(space) {
            match &self.workspaces[ws_id].layout_system {
                LayoutSystemKind::Scrolling(_) => {
                    Some(self.layout_settings.scrolling.animate.unwrap_or(true))
                }
                _ => None,
            }
        } else {
            None
        }
    }

    fn active_floating_windows_in_workspace(
        &self,
        window_store: Windows<'_>,
        space: SpaceId,
    ) -> Vec<WindowId> {
        self.workspaces
            .reading(window_store)
            .windows_in_active_workspace(window_store, space)
            .into_iter()
            .filter(|&window| self.is_floating_active(window_store, space, window))
            .collect()
    }

    /// Whether `window` is an active floating window on `space`: floating, in the Space's active
    /// workspace and available. Computed from the core's floating set and topology, never kept.
    fn is_floating_active(
        &self,
        window_store: Windows<'_>,
        space: SpaceId,
        window: WindowId,
    ) -> bool {
        self.floats.is_floating(window)
            && self.is_active_workspace_window(window_store, space, window)
    }

    /// One of the engine's focus choices, answered in its own kind: the Rust core's answer, and
    /// under `roc-core` the Roc core's, which decides. A Roc answer of another kind is a
    /// difference like any other (reported; the Rust answer stands).
    fn focus<A: FocusAsk<WindowId, SpaceId>>(ask: A) -> A::Answer {
        let rust = ask.answer();
        #[cfg(feature = "roc-core")]
        let rust = {
            use crate::roc_path::Word;
            let words = ask.question().map(Word::word, Word::word);
            let roc = rift_core_roc::focus::engine_focus(words.clone());
            crate::roc_path::decided(
                &crate::roc_path::FOCUS,
                "engine_focus",
                &[],
                &words,
                rust,
                &A::choice(rust)
                    .map(|w| Some(w.word()), |s| Some(s.word()))
                    .expect("words for every id"),
                Ok(roc),
                |roc| {
                    (roc.map(crate::roc_path::window_of_word, |s| Some(SpaceId::new(s))))
                        .and_then(A::of)
                },
            )
        };
        rust
    }

    fn preferred_focus_for_workspace(
        &self,
        window_store: Windows<'_>,
        space: SpaceId,
        workspace_id: VirtualWorkspaceId,
        preferred_focus_window: Option<WindowId>,
    ) -> Option<WindowId> {
        let eligible = |window: WindowId| Eligible {
            window,
            belongs: window_store.workspace_for_window(window) == Some(workspace_id)
                && !window_store.is_temporarily_unavailable(window),
        };
        let workspace = &self.workspaces[workspace_id];
        let layout = workspace.active_layout();
        let facts = WorkspaceFocus {
            preferred: preferred_focus_window.map(eligible),
            last_focused: workspace.last_focused().map(eligible),
            selected: layout
                .and_then(|layout| workspace.layout_system.selected_window(layout))
                .map(eligible),
            visible: layout
                .map(|layout| workspace.layout_system.windows(layout, WindowQuery::Visible))
                .unwrap_or_default()
                .into_iter()
                .map(eligible)
                .collect(),
            floating: self.active_floating_windows_in_workspace(window_store, space),
            floating_last_focus: self.floats.last_focus(),
        };
        Self::focus(facts)
    }

    pub(crate) fn commit_workspace_focus(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        space: SpaceId,
        focus_window: Option<WindowId>,
    ) {
        let workspace = self.workspaces.reading(window_store.read()).active_workspace(space);
        let focus = focus_window.map(|window| Committed {
            window,
            belongs: workspace.is_some()
                && window_store.read().workspace_for_window(window) == workspace
                && !window_store.is_temporarily_unavailable(window),
            floating: self.floats.is_floating(window),
        });
        let record = Self::focus(RecordAsk {
            workspace: workspace.is_some(),
            focus,
        });
        match (record, workspace) {
            (FocusRecord::NoWorkspace, _) | (_, None) => self.floats.set_focused(None),
            (FocusRecord::Cleared, Some(workspace_id)) => {
                self.floats.set_focused(None);
                self.workspaces.set_last_focused_window(
                    window_store.core,
                    space,
                    workspace_id,
                    None,
                );
            }
            (FocusRecord::Floating(wid), Some(workspace_id)) => {
                self.floats.set_focused(Some(wid));
                self.workspaces.set_last_focused_window(
                    window_store.core,
                    space,
                    workspace_id,
                    Some(wid),
                );
                self.floats.set_last_focus(Some(wid));
            }
            (FocusRecord::Tiled(wid), Some(workspace_id)) => {
                self.floats.set_focused(Some(wid));
                self.workspaces.set_last_focused_window(
                    window_store.core,
                    space,
                    workspace_id,
                    Some(wid),
                );
                if let Some(layout) =
                    self.workspaces.reading(window_store.read()).active_layout(space, workspace_id)
                {
                    let _ = self.workspaces[workspace_id]
                        .layout_system
                        .focus(layout, Focus::Select(wid))
                        .handled;
                }
            }
        }
    }

    pub(crate) fn activate_workspace(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        space: SpaceId,
        workspace_id: VirtualWorkspaceId,
        preferred_focus_window: Option<WindowId>,
    ) -> EventResponse {
        if !self.workspaces.set_active_workspace(window_store, space, workspace_id) {
            return EventResponse::default();
        }

        EventResponse {
            changed: true,
            workspace_space: Some(space),
            focus_window: self.preferred_focus_for_workspace(
                window_store.read(),
                space,
                workspace_id,
                preferred_focus_window,
            ),
            raise_windows: vec![],

            ..EventResponse::default()
        }
    }

    fn switch_to_workspace_id(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        context: WorkspaceContext<'_>,
        id: VirtualWorkspaceId,
        preferred_focus_window: Option<WindowId>,
    ) -> EventResponse {
        let command_space = context.space;
        let shown_here =
            self.workspaces.reading(window_store.read()).active_workspace(command_space);
        let back_and_forth = self.workspaces.settings.workspace_auto_back_and_forth;
        match rift_core::workspaces::switch(shown_here, id, back_and_forth) {
            Switch::Stay => EventResponse::default(),
            Switch::Back => self.switch_to_previous_workspace(window_store, context),
            Switch::Navigate => {
                self.navigate_to_workspace(window_store, command_space, id, preferred_focus_window)
            }
        }
    }

    fn switch_to_previous_workspace(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        context: WorkspaceContext<'_>,
    ) -> EventResponse {
        self.workspaces
            .reading(window_store.read())
            .previous_target(context)
            .map(|target| self.navigate_to_workspace(window_store, context.space, target.id, None))
            .unwrap_or_default()
    }

    fn navigate_to_workspace(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        command_space: SpaceId,
        id: VirtualWorkspaceId,
        preferred_focus_window: Option<WindowId>,
    ) -> EventResponse {
        let space = self
            .workspaces
            .reading(window_store.read())
            .workspace_space(id)
            .expect("a listed workspace has a Space");
        let shown_there = self.workspaces.reading(window_store.read()).active_workspace(space);
        if !self.workspaces.set_active_workspace_from(window_store, command_space, id) {
            return EventResponse::default();
        }
        match rift_core::workspaces::arrival(shown_there, id, command_space, space) {
            Arrival::Focus(space) => EventResponse {
                workspace_space: Some(space),
                focus_window: self.preferred_focus_for_workspace(
                    window_store.read(),
                    space,
                    id,
                    preferred_focus_window,
                ),
                ..EventResponse::default()
            },
            Arrival::Activate => {
                self.activate_workspace(window_store, space, id, preferred_focus_window)
            }
        }
    }

    fn filter_active_workspace_windows(
        &self,
        window_store: Windows<'_>,
        space: SpaceId,
        mut windows: Vec<WindowId>,
    ) -> Vec<WindowId> {
        windows.retain(|&window| self.is_active_workspace_window(window_store, space, window));
        windows
    }

    fn filter_active_workspace_window(
        &self,
        window_store: Windows<'_>,
        space: SpaceId,
        window: Option<WindowId>,
    ) -> Option<WindowId> {
        window.filter(|&window| self.is_active_workspace_window(window_store, space, window))
    }

    fn is_active_workspace_window(
        &self,
        window_store: Windows<'_>,
        space: SpaceId,
        window: WindowId,
    ) -> bool {
        !window_store.is_temporarily_unavailable(window)
            && self
                .workspaces
                .reading(window_store)
                .is_window_in_active_workspace(space, window)
    }

    pub fn resize_selection(
        &mut self,
        ws_id: VirtualWorkspaceId,
        layout: LayoutId,
        resize_amount: f64,
    ) {
        self.forget_native_layout_for_workspace(ws_id);
        self.workspaces[ws_id]
            .layout_system
            .edit(layout, LayoutEdit::Resize {
                amount: resize_amount,
                orientation: ResizeOrientation::Horizontal,
            })
            .discard();
    }

    fn apply_focus_response(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        space: SpaceId,
        ws_id: VirtualWorkspaceId,
        layout: LayoutId,
        response: &EventResponse,
    ) {
        let Some(window) = response.focus_window else {
            return;
        };
        self.floats.set_focused(Some(window));
        let committed = Committed {
            window,
            belongs: true,
            floating: self.floats.is_floating(window),
        };
        match Self::focus(RecordAsk {
            workspace: true,
            focus: Some(committed),
        }) {
            FocusRecord::Floating(wid) => self.floats.set_last_focus(Some(wid)),
            FocusRecord::Tiled(wid) => {
                let _ =
                    self.workspaces[ws_id].layout_system.focus(layout, Focus::Select(wid)).handled;
                self.workspaces.set_last_focused_window(
                    &window_store.core,
                    space,
                    ws_id,
                    Some(wid),
                );
            }
            FocusRecord::NoWorkspace | FocusRecord::Cleared => {
                unreachable!("a window that belongs, with a workspace, is recorded")
            }
        }
    }

    fn move_focus_internal(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        space: SpaceId,
        visible_spaces: &[SpaceId],
        visible_space_frames: &HashMap<SpaceId, CGRect>,
        direction: Direction,
        is_floating: bool,
    ) -> EventResponse {
        let Some((ws_id, layout)) =
            self.workspaces.reading(window_store.read()).active_layout_for_space(space)
        else {
            warn!(
                "No active workspace/layout for space {:?}; move_focus ignored",
                space
            );
            return EventResponse::default();
        };

        let commit = |engine: &mut Self,
                      window_store: &mut WindowsMut<'_>,
                      (space, ws, layout),
                      window,
                      raise_windows| {
            let response = EventResponse {
                changed: true,
                focus_window: Some(window),
                raise_windows,
                ..Default::default()
            };
            engine.apply_focus_response(window_store, space, ws, layout, &response);
            response
        };
        let current = (space, ws_id, layout);
        let visible =
            |engine: &Self,
             windows: Windows<'_>,
             (space, ws, layout): (SpaceId, VirtualWorkspaceId, LayoutId)| {
                engine.filter_active_workspace_windows(
                    windows,
                    space,
                    engine.workspaces[ws].layout_system.windows(layout, WindowQuery::Visible),
                )
            };

        if is_floating {
            let windows = self.active_floating_windows_in_workspace(window_store.read(), space);
            let cycled = Self::focus(CycleAsk {
                floating: windows,
                focused: self.floats.focused(),
                direction: super::systems::core_direction(direction),
            });
            if let Some(window) = cycled {
                return commit(self, window_store, current, window, Vec::new());
            }
            let tiled = visible(self, window_store.read(), current);
            return match tiled.first().copied() {
                Some(window) => commit(self, window_store, current, window, tiled),
                None => EventResponse::default(),
            };
        }

        let system = &mut self.workspaces[ws_id].layout_system;
        let previous = system.selected_window(layout);
        let focus = system.focus(layout, Focus::Direction(direction));
        if let Some(window) =
            self.filter_active_workspace_window(window_store.read(), space, focus.window)
        {
            let raise =
                self.filter_active_workspace_windows(window_store.read(), space, focus.raise);
            return commit(self, window_store, current, window, raise);
        }
        if let Some(window) = previous {
            self.workspaces[ws_id].layout_system.focus(layout, Focus::Select(window));
        }
        let scrolling = matches!(
            self.workspaces[ws_id].layout_system,
            LayoutSystemKind::Scrolling(_)
        );
        let edge = EdgeAsk {
            direction: super::systems::core_direction(direction),
            scrolling,
        };
        if Self::focus(edge) == PastTheEdge::Stop {
            return EventResponse::default();
        }

        if let Some(destination) =
            self.next_space_for_direction(space, direction, visible_spaces, visible_space_frames)
        {
            let Some((ws, layout)) = self
                .workspaces
                .reading(window_store.read())
                .active_layout_for_space(destination)
            else {
                return EventResponse::default();
            };
            let target = (destination, ws, layout);
            let raise = visible(self, window_store.read(), target);
            let directional =
                self.workspaces[ws].layout_system.window_in_direction(layout, direction);
            let directional =
                self.filter_active_workspace_window(window_store.read(), destination, directional);
            let entry = EntryAsk {
                directional,
                visible: raise.clone(),
            };
            if let Some(window) = Self::focus(entry) {
                self.workspaces[ws].layout_system.focus(layout, Focus::Select(window));
                return commit(self, window_store, target, window, raise);
            }
        }

        let fallback = Self::focus(FallbackAsk {
            first_floating: self
                .active_floating_windows_in_workspace(window_store.read(), space)
                .first()
                .copied(),
            previous: self.filter_active_workspace_window(window_store.read(), space, previous),
            first_visible: visible(self, window_store.read(), current).first().copied(),
        });
        match fallback {
            Some(window) => commit(self, window_store, current, window, Vec::new()),
            None => EventResponse::default(),
        }
    }

    fn next_space_for_direction(
        &self,
        current_space: SpaceId,
        direction: Direction,
        visible_spaces: &[SpaceId],
        space_frames: &HashMap<SpaceId, CGRect>,
    ) -> Option<SpaceId> {
        use crate::layout_engine::core_rect;
        let current = core_rect(*space_frames.get(&current_space)?);
        let candidates: Vec<_> = (visible_spaces.iter())
            .filter(|&&space| space != current_space)
            .filter_map(|&space| Some((space, core_rect(*space_frames.get(&space)?))))
            .collect();
        Self::focus(SpaceAsk {
            current,
            direction: super::systems::core_direction(direction),
            candidates,
        })
    }

    // Explicit placement edits revoke recovery even when logical membership is unchanged.
    // Native reconciliation deliberately does not use this boundary.
    fn forget_native_layout_for_window_id(&mut self, wid: WindowId) {
        self.evicted.step(EvictionEvent::Forgotten(wid));
        self.edit_native_snapshots(wid, WindowChange::Remove(wid));
    }

    /// Every snapshot holding `window` edited by `change` through its system.
    fn edit_native_snapshots(&mut self, window: WindowId, change: WindowChange<'_>) {
        for workspace in self.native.holding(window) {
            let snapshot = self.native.get(workspace).expect("a snapshot it named").updated(change);
            self.native.step(MemoryEvent::Edited { workspace, snapshot });
        }
    }

    fn forget_native_layout_for_workspace(&mut self, workspace: VirtualWorkspaceId) {
        self.native.step(MemoryEvent::Forgotten(workspace));
        for window in self.evicted.landed_in(workspace) {
            self.forget_native_layout_for_window_id(window);
        }
    }

    pub(crate) fn forget_native_layout_for_window(&mut self, windows: Windows<'_>, wid: WindowId) {
        self.forget_native_layout_for_window_id(wid);
        if let Some(assignment) = windows.workspace_for_window(wid) {
            self.native.step(MemoryEvent::Forgotten(assignment));
        }
    }

    pub(crate) fn remember_native_departure(&mut self, windows: Windows<'_>, wid: WindowId) {
        let Some(assignment) = windows.workspace_for_window(wid) else {
            return;
        };
        let workspace = assignment;
        if !self.native.keeps(workspace, self.floats.is_floating(wid)) {
            return;
        }
        if let Some(snapshot) = NativeSnapshot::of(&self.workspaces[workspace]) {
            self.native.step(MemoryEvent::Kept { workspace, snapshot });
        }
    }

    pub(crate) fn begin_native_reconcile(
        &mut self,
        windows: Windows<'_>,
        moves: &[(WindowId, SpaceId)],
    ) {
        // Capture every source before the first arrival can modify another source tree.
        for &(window, target) in moves {
            if windows.workspace_for_window(window).is_some_and(|source| {
                self.workspaces.reading(windows).workspace_space(source) != Some(target)
            }) {
                self.remember_native_departure(windows, window);
            }
        }
        self.native.step(MemoryEvent::ReconcileBegan);
    }

    pub(crate) fn finish_native_reconcile(&mut self, windows: Windows<'_>) {
        let MemoryAnswer::Waited(workspaces) = self.native.step(MemoryEvent::ReconcileEnded) else {
            unreachable!("the batch's end answers the workspaces that waited");
        };
        for workspace in workspaces {
            self.restore_native_layout(windows, workspace, None);
        }
        // Forget windows that were closed, rekeyed or placed somewhere else since.
        for wid in self.evicted.stale(|wid| windows.workspace_for_window(wid)) {
            self.forget_native_layout_for_window_id(wid);
        }
    }

    /// Tells the evictions machine where a display departure moved `wid` from. Call after the
    /// window has been reassigned; `from` is its previous assignment.
    pub(crate) fn note_eviction(
        &mut self,
        windows: Windows<'_>,
        wid: WindowId,
        from: VirtualWorkspaceId,
    ) {
        let view = self.workspaces.reading(windows);
        let (Some(landed), Some(from_space)) =
            (windows.workspace_for_window(wid), view.workspace_space(from))
        else {
            return;
        };
        let Some(display) = self.display_uuid_for_space(from_space) else {
            return;
        };
        let Some(ordinal) = view.workspaces_on(from_space).position(|id| id == from) else {
            return;
        };
        let landed_space = view.workspace_space(landed).expect("a listed workspace has a Space");
        let event = EvictionEvent::Evicted {
            window: wid,
            from: display.to_owned(),
            ordinal: ordinal as u64,
            landed,
            landed_on: self.display_uuid_for_space(landed_space).map(str::to_owned),
        };
        self.evicted.step(event);
    }

    /// The workspace position `wid` was evicted from, if the display now showing `space` is the
    /// one it was evicted from and nothing has placed the window elsewhere since.
    pub(crate) fn eviction_ordinal(
        &self,
        windows: Windows<'_>,
        wid: WindowId,
        space: SpaceId,
    ) -> Option<usize> {
        let showing = self.displays.display_of(space);
        let ordinal = self.evicted.ordinal(wid, windows.workspace_for_window(wid), showing)?;
        Some(ordinal as usize)
    }

    /// Consume recovery only after native reassignment has succeeded, including a fallback.
    pub(crate) fn take_eviction(&mut self, wid: WindowId) -> Option<usize> {
        let ordinal = self.evicted.get(wid).map(|record| record.ordinal as usize);
        self.evicted.step(EvictionEvent::Forgotten(wid));
        ordinal
    }

    fn restore_native_layout(
        &mut self,
        windows: Windows<'_>,
        workspace: VirtualWorkspaceId,
        returning: Option<WindowId>,
    ) {
        let current = &self.workspaces[workspace];
        let size = current.layout_state.active_size();
        let facts = match current.layout_state.active() {
            None => Current::Unlaid,
            Some(layout) => Current::Laid(
                (current.layout_system.windows(layout, WindowQuery::All).into_iter())
                    .map(|window| Candidate {
                        window,
                        in_workspace: windows
                            .workspace_for_window(window)
                            .or_else(|| windows.claim(window).and_then(|claim| claim.workspace()))
                            == Some(workspace),
                        present: windows.is_admitted(window)
                            || windows.is_temporarily_unavailable(window),
                        floating: self.floats.is_floating(window),
                    })
                    .collect(),
            ),
        };
        let asked = MemoryEvent::RestoreAsked {
            workspace,
            returning,
            current: facts,
        };
        let MemoryAnswer::Restore(plan) = self.native.step(asked) else {
            return;
        };
        let size = size.expect("a laid-out workspace has a size");
        let (mut system, mut state) = plan.snapshot.restored();
        // Only a working copy is pruned: partial returns share one original arrangement.
        for &window in &plan.remove {
            system.update(WindowChange::Remove(window));
        }
        if let Some(layout) = state.active() {
            for &window in &plan.add {
                system.update(WindowChange::Add(layout, window, None));
            }
        }
        // Display recovery restores topology, never historical eligibility.
        for &window in &plan.ordered {
            system.set_window_participation(window, !windows.is_temporarily_unavailable(window));
        }
        state.ensure_active(size, &mut system);
        if let Some(focused) = self.floats.focused().filter(|window| plan.ordered.contains(window))
            && let Some(layout) = state.active()
        {
            system.focus(layout, Focus::Select(focused)).handled;
        }
        let current = &mut self.workspaces[workspace];
        current.layout_system = system;
        current.layout_state = state;
    }

    fn suspend_window(&mut self, window_store: Windows<'_>, wid: WindowId) -> bool {
        if let Some(assignment) = window_store.workspace_for_window(wid)
            && let Some(space) = self.workspaces.reading(window_store).workspace_space(assignment)
            && let Some(workspace) = self.workspaces.workspaces.get_mut(assignment)
        {
            let projected = workspace
                .active_layout()
                .is_some_and(|layout| workspace.layout_system.contains_window(layout, wid));
            let changed = (workspace.layout_system.set_window_participation(wid, false)
                && projected)
                || self.is_floating_active(window_store, space, wid);
            self.floats.focus_left(wid);
            return changed;
        }
        false
    }

    fn remove_window_internal(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        wid: WindowId,
        preserve_floating: bool,
    ) -> bool {
        let removal = self.remove_window_layout_membership(window_store.read(), wid);

        if preserve_floating {
        } else {
            self.floats.remove_floating(wid);
        }

        if !preserve_floating {
            self.workspaces.remove_window(window_store, wid);
            self.floats.remove_window(wid);
            self.forget_persisted_window(wid);
        }

        self.floats.focus_left(wid);

        removal.is_some()
    }

    fn remove_window_layout_membership(
        &mut self,
        window_store: Windows<'_>,
        wid: WindowId,
    ) -> Option<SpaceId> {
        if let Some(assignment) = window_store.workspace_for_window(wid)
            && let Some(space) = self.workspaces.reading(window_store).workspace_space(assignment)
            && let Some(workspace) = self.workspaces.workspaces.get_mut(assignment)
        {
            let projected = workspace
                .active_layout()
                .is_some_and(|layout| workspace.layout_system.contains_window(layout, wid));
            workspace.layout_system.update(WindowChange::Remove(wid));
            if workspace.last_focused() == Some(wid) {
                workspace.set_last_focused(None);
            }
            return (projected || self.is_floating_active(window_store, space, wid))
                .then_some(space);
        }
        let active_space = self.space_with_window(window_store, wid);

        // The store may already have dropped the record (for example after
        // WindowDestroyed). Layout membership is only a projection, so scrub
        // every tree when its authoritative assignment is unavailable.
        for workspace in self.workspaces.workspaces.values_mut() {
            workspace.layout_system.update(WindowChange::RemoveAndRebalance(wid));
            if workspace.last_focused() == Some(wid) {
                workspace.set_last_focused(None);
            }
        }
        active_space
    }

    fn add_window_to_layout(
        &mut self,
        window_store: Windows<'_>,
        space: SpaceId,
        wid: WindowId,
    ) -> bool {
        if !window_store.is_admitted(wid) || window_store.is_window_native_fullscreen_suspended(wid)
        {
            return false;
        }
        let Some(assigned_workspace) =
            self.workspaces.reading(window_store).workspace_for_window(space, wid)
        else {
            return false;
        };

        let active_space_before = self.projected_space_in(window_store, assigned_workspace, wid);
        let mut inserted = false;

        if matches!(
            self.workspaces[assigned_workspace].layout_system,
            LayoutSystemKind::Floating(_)
        ) {
            self.floats.remove_floating(wid);
        }
        let should_be_floating = self.floats.is_floating(wid);
        let resumed = self.workspaces[assigned_workspace]
            .layout_system
            .set_window_participation(wid, true);

        if should_be_floating {
        } else if let Some(layout) =
            self.workspaces.reading(window_store).active_layout(space, assigned_workspace)
        {
            if !self.workspaces[assigned_workspace].layout_system.contains_window(layout, wid) {
                if matches!(
                    self.workspaces[assigned_workspace].layout_system,
                    LayoutSystemKind::Scrolling(_)
                ) && self.layout_settings.scrolling.preserve_window_sizes
                {
                    self.preserve_scrolling_window_width(window_store, wid);
                }
                self.workspaces[assigned_workspace]
                    .layout_system
                    .update(WindowChange::Add(layout, wid, None));
                inserted = true;
            }
        } else {
            warn!(
                "No active layout for workspace {:?} on space {:?}; window {:?} not added to tree",
                assigned_workspace, space, wid
            );
        }

        if inserted {
            self.restore_native_layout(window_store, assigned_workspace, Some(wid));
        }
        resumed
            || self.projected_space_in(window_store, assigned_workspace, wid) != active_space_before
    }

    fn preserve_scrolling_window_width(&mut self, window_store: Windows<'_>, wid: WindowId) {
        if let Some(window) = window_store.window(wid) {
            let frame = window.frame_monotonic().size;
            let observed = Self::observed_size(
                window.info.is_resizable,
                frame,
                window.info.min_size,
                window.info.max_size,
            );
            self.constraints.step(rift_core::constraints::ConstraintsEvent::WidthPreserved {
                window: wid,
                observed,
                width: frame.width,
            });
        }
    }

    fn remove_window_from_all_tiling_trees(&mut self, wid: WindowId) {
        for workspace in self.workspaces.workspaces.values_mut() {
            workspace.layout_system.update(WindowChange::Remove(wid));
        }
    }

    fn workspace_contains_window(&self, workspace_id: VirtualWorkspaceId, wid: WindowId) -> bool {
        let workspace = &self.workspaces[workspace_id];
        workspace
            .layout_state
            .active()
            .into_iter()
            .any(|layout| workspace.layout_system.contains_window(layout, wid))
    }

    fn active_workspace_contains_window(
        &self,
        windows: Windows<'_>,
        space: SpaceId,
        workspace_id: VirtualWorkspaceId,
        wid: WindowId,
    ) -> bool {
        self.workspaces
            .reading(windows)
            .active_layout(space, workspace_id)
            .is_some_and(|layout| {
                self.workspaces[workspace_id].layout_system.contains_window(layout, wid)
            })
    }

    fn projected_space_in(
        &self,
        windows: Windows<'_>,
        workspace: VirtualWorkspaceId,
        window: WindowId,
    ) -> Option<SpaceId> {
        let current = self.workspaces.workspaces.get(workspace)?;
        let view = self.workspaces.reading(windows);
        let space = view.workspace_space(workspace)?;
        let visible = view.active_workspace(space) == Some(workspace)
            && current
                .active_layout()
                .is_some_and(|layout| current.layout_system.contains_window(layout, window));
        (visible || self.is_floating_active(windows, space, window)).then_some(space)
    }

    pub(crate) fn space_with_window(&self, windows: Windows<'_>, wid: WindowId) -> Option<SpaceId> {
        self.workspaces
            .reading(windows)
            .catalog()
            .find_map(|id| self.projected_space_in(windows, id, wid))
    }

    fn active_workspace_id_and_name(
        &self,
        windows: Windows<'_>,
        space_id: SpaceId,
    ) -> Option<(crate::model::VirtualWorkspaceId, String)> {
        let workspace_id = self.workspaces.reading(windows).active_workspace(space_id)?;
        Some((workspace_id, self.workspaces[workspace_id].name.clone()))
    }

    pub(crate) fn update_space_display(
        &mut self,
        windows: Windows<'_>,
        space: SpaceId,
        display_uuid: Option<&str>,
    ) {
        let Some(uuid) = display_uuid else {
            if self.displays.step(DisplayEvent::Hidden(space)) {
                self.sync_scrolling_widths_for_space(windows, space);
            }
            return;
        };
        let mut placed = false;
        let on_space: Vec<_> = self.workspaces.reading(windows).workspaces_on(space).collect();
        for id in on_space {
            let workspace = &mut self.workspaces[id];
            if workspace.home.display().is_none() {
                let by = workspace.home.placed_by().unwrap_or_default();
                workspace.home =
                    crate::model::virtual_workspace::Home::placed(space, Some(uuid.to_owned()), by);
                placed = true;
            }
        }
        // An unresolved saved Space still needs this display's evidence.
        let last = if self.saved_space_is_unresolved(self.last_space_for_display_uuid(uuid)) {
            LastSpace::Hold
        } else {
            LastSpace::Follow
        };
        let shown = self.displays.step(DisplayEvent::Shown {
            space,
            display: uuid.to_owned(),
            last,
        });
        if !(shown || placed) {
            return;
        }
        self.sync_scrolling_widths_for_space(windows, space);
    }

    fn sync_scrolling_widths_for_space(&mut self, windows: Windows<'_>, space: SpaceId) {
        let display = self.display_uuid_for_space(space);
        let widths = self.layout_settings.scrolling.widths_for_display(display);
        for id in self.workspaces.reading(windows).workspaces_on(space).collect::<Vec<_>>() {
            if let LayoutSystemKind::Scrolling(system) = &mut self.workspaces[id].layout_system {
                system.update_width_settings(widths);
            }
        }
    }

    pub fn last_space_for_display_uuid(&self, display_uuid: &str) -> Option<SpaceId> {
        self.displays.last_space(display_uuid)
    }

    pub(crate) fn display_uuid_for_space(&self, space: SpaceId) -> Option<&str> {
        self.displays.display_of(space).map(String::as_str)
    }

    /// Move all per-space layout state from `old_space` to `new_space`.
    pub(crate) fn remap_space(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        old_space: SpaceId,
        new_space: SpaceId,
    ) {
        if old_space == new_space {
            return;
        }

        // Saved native arrangements are keyed by workspace; one for a deleted workspace
        // would no longer resolve.
        for deleted in self.workspaces.remap_space(window_store, old_space, new_space) {
            self.forget_workspace(deleted);
        }

        self.displays.step(DisplayEvent::Remapped { from: old_space, to: new_space });
    }

    /// Drop everything keyed by a workspace that no longer exists; one left behind would no
    /// longer resolve.
    fn forget_workspace(&mut self, workspace: VirtualWorkspaceId) {
        self.native.step(MemoryEvent::Forgotten(workspace));
        self.floats.remove_workspace(workspace);
        self.forget_persisted_workspace(workspace);
    }

    pub fn new(
        virtual_workspace_config: &crate::common::config::VirtualWorkspaceSettings,
        layout_settings: &LayoutSettings,
    ) -> Self {
        let workspaces = WorkspaceStore::new_with_config(virtual_workspace_config, layout_settings);

        LayoutEngine {
            floats: super::floating::EngineFloats::new(),
            app_rules: AppRuleEngine::new(&virtual_workspace_config.app_rules),
            constraints: Default::default(),
            workspaces,
            layout_settings: layout_settings.clone(),
            displays: Default::default(),
            persistence: PersistenceState::default(),
            scroll_boundary: None,
            native: Default::default(),
            evicted: Default::default(),
            shadow_passes: Default::default(),
        }
    }

    fn apply_app_rule_outcome(
        &mut self,
        window: WindowId,
        space: SpaceId,
        was_floating: bool,
        effects: AppRuleEffects,
        app_rule_outcome: &mut AppRuleOutcome,
    ) -> Option<(WindowId, VirtualWorkspaceId)> {
        let should_float = effects.should_float(was_floating);
        if should_float {
            self.floats.add_floating(window);
        } else if was_floating {
            self.floats.remove_floating(window);
        }

        app_rule_outcome.frame =
            effects.floating_placement(window, space).map(AppRuleFrame::Place).or_else(|| {
                effects.tiled_resize(window, space, was_floating).map(AppRuleFrame::Resize)
            });

        effects.focus.then_some((window, effects.workspace_id))
    }

    pub(crate) fn apply_app_rule_resize(
        &mut self,
        windows: Windows<'_>,
        resize: AppRuleResize,
        old_frame: CGRect,
        new_frame: CGRect,
        screen_frame: CGRect,
        display_uuid: Option<&str>,
    ) {
        self.constraints.step(rift_core::constraints::ConstraintsEvent::WidthSet {
            window: resize.window,
            width: new_frame.size.width,
        });
        let Some(layout) = self
            .workspaces
            .reading(windows)
            .active_layout(resize.space, resize.workspace_id)
        else {
            return;
        };
        let gaps = self.layout_settings.gaps.effective_for_display(display_uuid);
        let previous_selection =
            self.workspaces[resize.workspace_id].layout_system.selected_window(layout);
        let tree = &mut self.workspaces[resize.workspace_id].layout_system;
        let _ = tree.focus(layout, Focus::Select(resize.window)).handled;
        tree.update(WindowChange::Resize {
            layout,
            window: resize.window,
            old: old_frame,
            new: new_frame,
            screen: screen_frame,
            gaps: &gaps,
        });
        if let Some(previous) = previous_selection.filter(|window| *window != resize.window) {
            let _ = tree.focus(layout, Focus::Select(previous)).handled;
        }
    }

    pub(crate) fn debug_tree(&self, windows: Windows<'_>, space: SpaceId) {
        self.debug_tree_desc(windows, space, "", false);
    }

    pub(crate) fn debug_tree_desc(
        &self,
        windows: Windows<'_>,
        space: SpaceId,
        desc: &'static str,
        print: bool,
    ) {
        if let Some(workspace_id) = self.workspaces.reading(windows).active_workspace(space) {
            if let Some(layout) =
                self.workspaces.reading(windows).active_layout(space, workspace_id)
            {
                if print {
                    println!(
                        "Tree {desc}\n{}",
                        self.workspaces[workspace_id].layout_system.draw_tree(layout).trim()
                    );
                } else {
                    debug!(
                        "Tree {desc}\n{}",
                        self.workspaces[workspace_id].layout_system.draw_tree(layout).trim()
                    );
                }
            } else {
                debug!("No layout for workspace {workspace_id:?} on space {space:?}");
            }
        } else {
            debug!("No active workspace for space {space:?}");
        }
    }

    /// Lifecycle additions restore participation, never logical ownership.
    fn normalize_event(&self, windows: Windows<'_>, event: LayoutEvent) -> LayoutEvent {
        match event {
            LayoutEvent::WindowAdded(space, window) => LayoutEvent::WindowAdded(
                windows
                    .workspace_for_window(window)
                    .and_then(|workspace| {
                        self.workspaces.reading(windows).workspace_space(workspace)
                    })
                    .unwrap_or(space),
                window,
            ),
            event => event,
        }
    }

    pub(crate) fn handle_event(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        event: LayoutEvent,
    ) -> LayoutEventOutcome {
        let event = self.normalize_event(window_store.read(), event);
        let mut app_rules = AppRuleOutcome::default();
        let response = self.handle_event_inner(window_store, event, &mut app_rules);
        LayoutEventOutcome { response, app_rules }
    }

    fn handle_event_inner(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        event: LayoutEvent,
        app_rule_outcome: &mut AppRuleOutcome,
    ) -> EventResponse {
        debug!(?event);
        // Claims keep a nonparticipating canonical slot until release. Ordinary
        // observations cannot reactivate it; destruction ends the claim first.
        if let LayoutEvent::WindowAdded(_, wid)
        | LayoutEvent::WindowSuspended(wid)
        | LayoutEvent::WindowRemoved(wid)
        | LayoutEvent::WindowObserved(
            _,
            ResolvedWindow {
                info: WindowLayoutInfo { window_id: wid, .. },
                ..
            },
        ) = &event
            && window_store.read().is_claimed(*wid)
        {
            return EventResponse::default();
        }
        match event {
            LayoutEvent::SpaceExposed(space, size) => {
                self.debug_tree(window_store.read(), space);

                self.ensure_workspace_layouts(window_store, space, size);
            }
            #[cfg(test)]
            LayoutEvent::TestWindowsObserved(space, windows, app_info) => {
                let (app_bundle_id, app_name) = app_info.as_ref().map_or((None, None), |app| {
                    (app.bundle_id.as_deref(), app.localized_name.as_deref())
                });
                let mut response = EventResponse::default();
                for info in windows {
                    if window_store.window(info.window_id).is_none() {
                        let _ = self.observe_window_for_persistence(
                            window_store,
                            space,
                            info.window_id,
                            info.title.as_deref(),
                            info.current_size,
                            app_bundle_id,
                        );
                    }
                    let Ok(Some(AppRuleResult::Managed(effects))) = self
                        .assign_window_with_app_info(
                            window_store,
                            info.window_id,
                            space,
                            WindowRuleContext {
                                app_bundle_id,
                                app_name,
                                window_title: info.title.as_deref(),
                                ax_role: info.ax_role.as_deref(),
                                ax_subrole: info.ax_subrole.as_deref(),
                                window_tags: None,
                            },
                            false,
                        )
                    else {
                        continue;
                    };
                    let next = self.handle_event_inner(
                        window_store,
                        LayoutEvent::WindowObserved(space, ResolvedWindow { info, effects }),
                        &mut AppRuleOutcome::default(),
                    );
                    response.changed |= next.changed;
                    response.raise_windows.extend(next.raise_windows);
                    response.focus_window = next.focus_window.or(response.focus_window);
                    response.boundary_hit = next.boundary_hit.or(response.boundary_hit);
                }
                return response;
            }
            LayoutEvent::WindowObserved(_, ResolvedWindow { info, effects }) => {
                let space = self
                    .workspaces
                    .reading(window_store.read())
                    .workspace_space(effects.workspace_id)
                    .expect("a listed workspace has a Space");
                self.debug_tree(window_store.read(), space);
                let wid = info.window_id;
                // A rule window that reports no minimum yet takes its app's last one (the
                // constraints machine decides).
                self.constraints.step(rift_core::constraints::ConstraintsEvent::Observed {
                    window: wid,
                    app: wid.pid,
                    respect_min_size: effects.respect_min_size,
                    observed: Self::observed_size(
                        info.is_resizable,
                        info.current_size,
                        info.min_size,
                        info.max_size,
                    ),
                });

                let target_workspace = effects.workspace_id;
                let source_workspace = effects.source_workspace;
                let should_focus = effects.focus;
                let projected_space_before = source_workspace.map_or_else(
                    || self.space_with_window(window_store.read(), wid),
                    |id| self.projected_space_in(window_store.read(), id, wid),
                );
                let was_floating = self.floats.is_floating(wid);
                let focus_request = self.apply_app_rule_outcome(
                    wid,
                    space,
                    was_floating,
                    effects,
                    app_rule_outcome,
                );

                // Workspace policy owns floating behavior without a global flag.
                // Rule reapplication must not remove these windows from their groups.
                if matches!(
                    self.workspaces[target_workspace].layout_system,
                    LayoutSystemKind::Floating(_)
                ) {
                    self.floats.remove_floating(wid);
                }
                let should_float = self.floats.is_floating(wid);
                let relocated = source_workspace
                    .filter(|&source| source != target_workspace)
                    .is_some_and(|source| {
                        self.relocate_window_to_workspace(
                            window_store,
                            source,
                            (space, target_workspace),
                            wid,
                            WindowRelocation::Observed,
                        )
                    });
                let mut membership_changed = relocated;
                if source_workspace.is_some() {
                    if should_float && !relocated {
                        membership_changed |= self.workspace_contains_window(target_workspace, wid);
                        self.workspaces[target_workspace]
                            .layout_system
                            .update(WindowChange::Remove(wid));
                    }
                } else {
                    // Initial discovery and snapshot recovery have no authoritative source
                    // projection. Keep the defensive scrub at that boundary only.
                    let ids: Vec<_> =
                        self.workspaces.reading(window_store.read()).catalog().collect();
                    for id in ids {
                        if !should_float && id == target_workspace {
                            continue;
                        }
                        membership_changed |= self.workspace_contains_window(id, wid);
                        self.workspaces[id].layout_system.update(WindowChange::Remove(wid));
                    }
                }

                if !relocated {
                    let previous_selection = self
                        .workspaces
                        .reading(window_store.read())
                        .active_layout(space, target_workspace)
                        .filter(|_| !should_float && !should_focus)
                        .and_then(|layout| {
                            self.workspaces[target_workspace].layout_system.selected_window(layout)
                        });
                    let contained = self.active_workspace_contains_window(
                        window_store.read(),
                        space,
                        target_workspace,
                        wid,
                    );
                    membership_changed |=
                        self.add_window_to_layout(window_store.read(), space, wid);
                    membership_changed |= !contained
                        && self.active_workspace_contains_window(
                            window_store.read(),
                            space,
                            target_workspace,
                            wid,
                        );
                    if let (Some(previous), Some(layout)) = (
                        previous_selection,
                        self.workspaces
                            .reading(window_store.read())
                            .active_layout(space, target_workspace),
                    ) && self.workspaces[target_workspace]
                        .layout_system
                        .contains_window(layout, previous)
                    {
                        self.workspaces[target_workspace]
                            .layout_system
                            .focus(layout, Focus::Select(previous))
                            .handled;
                    }
                }
                membership_changed |=
                    self.projected_space_in(window_store.read(), target_workspace, wid)
                        != projected_space_before;

                if let Some((window, workspace)) = focus_request {
                    if self.workspaces.reading(window_store.read()).active_workspace(space)
                        != Some(workspace)
                    {
                        app_rule_outcome.workspace_focus =
                            Some(AppRuleWorkspaceFocus { window, space, workspace });
                        return EventResponse {
                            changed: membership_changed,
                            ..EventResponse::default()
                        };
                    }
                    self.commit_workspace_focus(window_store, space, Some(window));
                    return EventResponse {
                        changed: membership_changed,
                        raise_windows: vec![window],
                        focus_window: Some(window),

                        ..EventResponse::default()
                    };
                }
                if membership_changed {
                    return EventResponse {
                        changed: true,
                        ..EventResponse::default()
                    };
                }
            }
            LayoutEvent::AppClosed(pid) => {
                let holding: Vec<_> = (self.native.snapshots())
                    .filter(|(_, snapshot)| snapshot.windows().iter().any(|w| w.pid == pid))
                    .map(|(workspace, _)| workspace)
                    .collect();
                for workspace in holding {
                    let snapshot =
                        self.native.get(workspace).unwrap().updated(WindowChange::CloseApp(pid));
                    self.native.step(MemoryEvent::Edited { workspace, snapshot });
                }
                let changed =
                    self.workspaces.reading(window_store.read()).layout_spaces().into_iter().any(
                        |space| {
                            self.workspaces
                                .reading(window_store.read())
                                .active_layout_for_space(space)
                                .is_some_and(|(workspace, layout)| {
                                    self.workspaces[workspace]
                                        .layout_system
                                        .has_windows_for_app(layout, pid)
                                })
                        },
                    );
                if self.floats.focused().is_some_and(|wid| wid.pid == pid) {
                    self.floats.set_focused(None);
                }
                for (_, ws) in self.workspaces.workspaces.iter_mut() {
                    ws.layout_system.update(WindowChange::CloseApp(pid));
                    if ws.last_focused().is_some_and(|wid| wid.pid == pid) {
                        ws.set_last_focused(None);
                    }
                }
                self.evicted.step(EvictionEvent::AppQuit(pid));
                self.floats.app_quit(pid);
                self.constraints.step(rift_core::constraints::ConstraintsEvent::AppQuit(pid));
                self.forget_persisted_app(pid);

                return EventResponse {
                    changed,
                    ..EventResponse::default()
                };
            }
            LayoutEvent::WindowAdded(space, wid) => {
                self.debug_tree(window_store.read(), space);
                return EventResponse {
                    changed: self.add_window_to_layout(window_store.read(), space, wid),
                    ..EventResponse::default()
                };
            }
            LayoutEvent::WindowRemoved(wid) => {
                self.forget_native_layout_for_window_id(wid);
                return EventResponse {
                    changed: self.remove_window_internal(window_store, wid, false),
                    ..EventResponse::default()
                };
            }
            LayoutEvent::WindowSuspended(wid) => {
                return EventResponse {
                    changed: self.suspend_window(window_store.read(), wid),
                    ..EventResponse::default()
                };
            }
            LayoutEvent::WindowRemovedPreserveFloating(wid) => {
                return EventResponse {
                    changed: self.remove_window_internal(window_store, wid, true),
                    ..EventResponse::default()
                };
            }
            LayoutEvent::WindowFocused(space, wid) => {
                if window_store.is_temporarily_unavailable(wid) {
                    return EventResponse::default();
                }
                if self.floats.is_floating(wid) {
                    self.floats.set_focused(Some(wid));
                    self.floats.set_last_focus(Some(wid));
                } else if let Some((ws_id, layout)) =
                    self.workspaces.reading(window_store.read()).active_layout_for_space(space)
                {
                    if !self.workspaces[ws_id].layout_system.contains_window(layout, wid) {
                        warn!(
                            "WindowFocused ignored: wid={:?} not in active layout for space {:?}",
                            wid, space
                        );
                        return EventResponse::default();
                    }
                    let preserve_selection = self.floats.focused() == Some(wid)
                        && self.workspaces[ws_id]
                            .layout_system
                            .selection_contains_window(layout, wid);
                    self.floats.set_focused(Some(wid));
                    let selection_changed =
                        self.workspaces[ws_id].layout_system.selected_window(layout) != Some(wid);
                    if !preserve_selection {
                        let _ = self.workspaces[ws_id]
                            .layout_system
                            .focus(layout, Focus::Select(wid))
                            .handled;
                    }
                    self.workspaces.set_last_focused_window(
                        &window_store.core,
                        space,
                        ws_id,
                        Some(wid),
                    );
                    return EventResponse {
                        changed: selection_changed
                            && matches!(
                                self.active_layout_mode_at(window_store.read(), space),
                                LayoutMode::Scrolling | LayoutMode::Floating
                            ),
                        ..EventResponse::default()
                    };
                } else {
                    warn!(
                        "No active workspace/layout for focused window {:?} on space {:?}",
                        wid, space
                    );
                }
            }
            LayoutEvent::WindowResized {
                wid,
                old_frame,
                new_frame,
                screens,
            } => {
                // Native departure no longer suspends logical availability. Its
                // ordered-out transition frames still cannot edit the saved split.
                if window_store
                    .read()
                    .record(wid)
                    .is_some_and(|r| r.visibility() == WindowVisibility::Hidden)
                {
                    return EventResponse::default();
                }
                self.constraints.step(rift_core::constraints::ConstraintsEvent::WidthSet {
                    window: wid,
                    width: new_frame.size.width,
                });
                for (space, screen_frame, display_uuid) in screens.iter() {
                    let Some((ws_id, layout)) = self
                        .workspaces
                        .reading(window_store.read())
                        .active_layout_for_space(*space)
                    else {
                        debug!(
                            "No active workspace/layout for resized window {:?} on space {:?}; skipping",
                            wid, space
                        );
                        continue;
                    };
                    let gaps =
                        self.layout_settings.gaps.effective_for_display(display_uuid.as_deref());
                    self.workspaces[ws_id].layout_system.update(WindowChange::Resize {
                        layout,
                        window: wid,
                        old: old_frame,
                        new: new_frame,
                        screen: *screen_frame,
                        gaps: &gaps,
                    });
                }
            }
        }
        EventResponse::default()
    }

    pub(crate) fn handle_command(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        space: Option<SpaceId>,
        visible_spaces: &[SpaceId],
        visible_space_frames: &HashMap<SpaceId, CGRect>,
        command: LayoutCommand,
    ) -> EventResponse {
        if !matches!(
            command,
            LayoutCommand::NextWindow
                | LayoutCommand::PrevWindow
                | LayoutCommand::MoveFocus(_)
                | LayoutCommand::ToggleFocusFloating
                | LayoutCommand::Ascend
                | LayoutCommand::Descend
                | LayoutCommand::ScrollStrip { .. }
                | LayoutCommand::MoveStackTab { .. }
                | LayoutCommand::NextWorkspace(_)
                | LayoutCommand::PrevWorkspace(_)
                | LayoutCommand::SwitchToWorkspace(_)
                | LayoutCommand::SwitchToLastWorkspace
                | LayoutCommand::CreateWorkspace
                | LayoutCommand::SetWorkspaceLayout { .. }
                | LayoutCommand::SnapStrip
                | LayoutCommand::CenterSelection
                | LayoutCommand::RotateMasterStackNext
                | LayoutCommand::RotateMasterStackPrev
        ) && let Some(workspace) = space
            .and_then(|space| self.workspaces.reading(window_store.read()).active_workspace(space))
        {
            self.forget_native_layout_for_workspace(workspace);
        }
        if !matches!(command, LayoutCommand::ScrollStrip { .. }) {
            self.scroll_boundary = None;
        }
        if let Some(space) = space {
            if let Some(ws_id) =
                self.workspaces.reading(window_store.read()).active_workspace(space)
            {
                if let Some(layout) =
                    self.workspaces.reading(window_store.read()).active_layout(space, ws_id)
                {
                    debug!(
                        "Tree:\n{}",
                        self.workspaces[ws_id].layout_system.draw_tree(layout).trim()
                    );
                    debug!(selection_window = ?self.workspaces[ws_id].layout_system.selected_window(layout));
                } else {
                    debug!("No active layout for workspace {:?} on space {:?}", ws_id, space);
                }
            } else {
                debug!("No active workspace for space {:?}", space);
            }
        }
        if let LayoutCommand::MoveStackTab { window, target } = command {
            // The stack line belongs to whichever display it was dragged on, not the
            // command space, so find the layout that holds the window.
            let window = crate::actor::app::WindowId::new(window.pid, window.idx);
            let target = crate::actor::app::WindowId::new(target.pid, target.idx);
            self.forget_native_layout_for_window(window_store.read(), window);
            let changed = visible_spaces.iter().any(|&space| {
                self.workspaces
                    .reading(window_store.read())
                    .active_layout_for_space(space)
                    .is_some_and(|(ws_id, layout)| {
                        let system = &mut self.workspaces[ws_id].layout_system;
                        system.contains_window(layout, window)
                            && system
                                .edit(layout, LayoutEdit::ReorderTab { window, target })
                                .changed()
                    })
            });
            return EventResponse {
                changed,
                ..EventResponse::default()
            };
        }
        let is_floating = if let Some(focus) = self.floats.focused() {
            self.floats.is_floating(focus)
        } else {
            false
        };
        debug!(focused = ?self.floats.focused(), last_floating_focus = ?self.floats.last_focus(), ?is_floating);

        let floating_options = match &command {
            LayoutCommand::ToggleWindowFloating => Some(ToggleWindowFloatingOptions::default()),
            LayoutCommand::ToggleWindowFloatingWithOptions(options) => Some(*options),
            _ => None,
        };
        if let Some(options) = floating_options {
            if let Some(window) = self.floats.focused() {
                window_store.clear_floating_display_geometry(window);
            }
            if space.is_some_and(|space| {
                self.active_layout_mode_at(window_store.read(), space) == LayoutMode::Floating
            }) {
                return EventResponse::default();
            }
            let Some(wid) = self.floats.focused() else {
                return EventResponse::default();
            };
            if is_floating {
                if let Some(space) = space {
                    let Some(assigned_workspace) = self
                        .workspaces()
                        .reading(window_store.read())
                        .workspace_for_window(space, wid)
                        .or_else(|| {
                            self.workspaces.reading(window_store.read()).active_workspace(space)
                        })
                    else {
                        return EventResponse::default();
                    };

                    if let Some(layout) = self
                        .workspaces
                        .reading(window_store.read())
                        .active_layout(space, assigned_workspace)
                    {
                        self.workspaces[assigned_workspace]
                            .layout_system
                            .update(WindowChange::Add(layout, wid, None));
                        debug!(
                            "Re-added floating window {:?} to tiling tree in workspace {:?}",
                            wid, assigned_workspace
                        );
                    }
                }
                self.floats.remove_floating(wid);
                self.floats.set_last_focus(None);
            } else {
                if let Some(space) = space {
                    if let Some((ws_id, _)) =
                        self.workspaces.reading(window_store.read()).active_layout_for_space(space)
                    {
                        self.workspaces[ws_id].layout_system.update(WindowChange::Remove(wid));
                        if options != ToggleWindowFloatingOptions::default()
                            && let (Some(center), Some(size), Some(current)) = (
                                visible_space_frames.get(&space).map(|frame| frame.mid()),
                                self.workspaces[ws_id].layout_state.active_size(),
                                window_store.window(wid).map(|window| window.frame_monotonic()),
                            )
                        {
                            let screen = CGRect::new(
                                CGPoint::new(
                                    center.x - size.width / 2.0,
                                    center.y - size.height / 2.0,
                                ),
                                size,
                            );
                            let frame = requested_floating_frame(
                                current,
                                screen,
                                options.center,
                                options.size,
                            );
                            self.floats.store(ws_id, wid, frame);
                        }
                    } else {
                        debug!(
                            "No active workspace/layout for space {:?}; leaving window {:?} out of tiling removal",
                            space, wid
                        );
                    }
                }
                self.floats.add_floating(wid);
                self.floats.set_last_focus(Some(wid));
                debug!("Removed window {:?} from tiling tree, now floating", wid);
            }
            return EventResponse::default();
        }

        if let LayoutCommand::ToggleFullscreen | LayoutCommand::ToggleFullscreenWithinGaps =
            &command
            && is_floating
        {
            let Some(wid) = self.floats.focused() else {
                return EventResponse::default();
            };
            let target = match command {
                LayoutCommand::ToggleFullscreenWithinGaps => FloatingFullscreenKind::WithinGaps,
                _ => FloatingFullscreenKind::Full,
            };
            if self.floats.fullscreen_kind(wid) == Some(target) {
                self.floats.set_fullscreen(wid, None);
            } else {
                // Only save the pre-fullscreen frame when switching from a non-fullscreen state,
                // and _not_ when switching between fullscreen kinds.
                if self.floats.fullscreen_kind(wid).is_none()
                    && let Some(space) = space
                {
                    let ws = self
                        .workspaces()
                        .reading(window_store.read())
                        .workspace_for_window(space, wid)
                        .or_else(|| {
                            self.workspaces.reading(window_store.read()).active_workspace(space)
                        });
                    if let (Some(ws), Some(frame)) =
                        (ws, window_store.window(wid).map(|w| w.frame_monotonic()))
                    {
                        self.floats.store(ws, wid, frame);
                    }
                }
                self.floats.set_fullscreen(wid, Some(target));
            }
            return EventResponse {
                changed: true,
                raise_windows: vec![wid],
                focus_window: Some(wid),

                ..EventResponse::default()
            };
        }

        let Some(space) = space else {
            return EventResponse::default();
        };
        let workspace_id =
            match self.workspaces.reading(window_store.read()).active_workspace(space) {
                Some(id) => id,
                None => {
                    warn!("No active virtual workspace for space {:?}", space);
                    return EventResponse::default();
                }
            };
        let layout =
            match self.workspaces.reading(window_store.read()).active_layout(space, workspace_id) {
                Some(id) => id,
                None => {
                    warn!(
                        "No active layout for workspace {:?} on space {:?}; command ignored",
                        workspace_id, space
                    );
                    return EventResponse::default();
                }
            };

        if let LayoutCommand::ToggleFocusFloating = &command {
            let (mut raise_windows, selection) = if is_floating {
                let system = &self.workspaces[workspace_id].layout_system;
                (
                    system.windows(layout, WindowQuery::Visible),
                    system.selected_window(layout),
                )
            } else {
                let selection = self.floats.last_focus();
                let windows = self
                    .active_floating_windows_in_workspace(window_store.read(), space)
                    .into_iter()
                    .filter(|wid| Some(*wid) != selection)
                    .collect();
                (windows, selection)
            };
            let focus_window = selection.or_else(|| raise_windows.pop());
            let response = EventResponse {
                changed: true,
                raise_windows,
                focus_window,
                ..Default::default()
            };
            self.apply_focus_response(window_store, space, workspace_id, layout, &response);
            return response;
        }

        match command {
            LayoutCommand::ToggleWindowFloating
            | LayoutCommand::ToggleWindowFloatingWithOptions(_) => {
                unreachable!()
            }
            LayoutCommand::ToggleFocusFloating => unreachable!(),
            LayoutCommand::MoveStackTab { .. } => unreachable!(),

            LayoutCommand::SwapWindows(a, b) => {
                let a = crate::actor::app::WindowId::new(a.pid, a.idx);
                let b = crate::actor::app::WindowId::new(b.pid, b.idx);
                self.forget_native_layout_for_window_id(a);
                self.forget_native_layout_for_window_id(b);
                let _ = self.workspaces[workspace_id]
                    .layout_system
                    .edit(layout, LayoutEdit::Swap(a, b))
                    .changed();

                EventResponse::default()
            }
            LayoutCommand::NextWindow | LayoutCommand::PrevWindow => {
                let forward = matches!(command, LayoutCommand::NextWindow);
                let windows = if let LayoutSystemKind::Floating(system) =
                    &mut self.workspaces[workspace_id].layout_system
                {
                    system.cycle_windows(layout)
                } else if is_floating {
                    self.active_floating_windows_in_workspace(window_store.read(), space)
                } else {
                    self.filter_active_workspace_windows(
                        window_store.read(),
                        space,
                        self.workspaces[workspace_id]
                            .layout_system
                            .windows(layout, WindowQuery::Visible),
                    )
                };
                let focus_window = if let Some(idx) =
                    windows.iter().position(|&w| Some(w) == self.floats.focused())
                {
                    let next = if forward {
                        (idx + 1) % windows.len()
                    } else {
                        (idx + windows.len() - 1) % windows.len()
                    };
                    Some(windows[next])
                } else {
                    self.workspaces[workspace_id]
                        .layout_system
                        .selected_window(layout)
                        .filter(|wid| windows.contains(wid))
                        .or_else(|| windows.first().copied())
                };
                let response = EventResponse {
                    changed: true,
                    focus_window,
                    raise_windows: focus_window.into_iter().collect(),
                    ..Default::default()
                };
                self.apply_focus_response(window_store, space, workspace_id, layout, &response);
                return response;
            }

            LayoutCommand::MoveFocus(direction) => {
                debug!(
                    "MoveFocus command received, direction: {:?}, is_floating: {}",
                    direction, is_floating
                );
                return self.move_focus_internal(
                    window_store,
                    space,
                    visible_spaces,
                    visible_space_frames,
                    direction,
                    is_floating,
                );
            }
            LayoutCommand::Ascend | LayoutCommand::Descend => {
                let focus = if matches!(command, LayoutCommand::Descend) {
                    Focus::Descend
                } else if is_floating {
                    return EventResponse::default();
                } else {
                    Focus::Ascend
                };
                let changed =
                    self.workspaces[workspace_id].layout_system.focus(layout, focus).handled;
                EventResponse { changed, ..Default::default() }
            }
            LayoutCommand::MoveNode(direction) => {
                let adjacent = self
                    .next_space_for_direction(
                        space,
                        direction,
                        visible_spaces,
                        visible_space_frames,
                    )
                    .and_then(|space| {
                        let (workspace, layout) = self
                            .workspaces
                            .reading(window_store.read())
                            .active_layout_for_space(space)?;
                        Some((space, workspace, layout))
                    });
                let changed = self.workspaces[workspace_id]
                    .layout_system
                    .edit(layout, LayoutEdit::Move {
                        direction,
                        has_display_neighbor: adjacent.is_some(),
                    })
                    .changed();
                let mut response = EventResponse {
                    changed,
                    ..EventResponse::default()
                };
                if !changed && let Some((space, target_workspace, _)) = adjacent {
                    let mut windows = self.workspaces[workspace_id]
                        .layout_system
                        .windows(layout, WindowQuery::Selection);
                    let target_system = &self.workspaces[target_workspace].layout_system;
                    // Preserve subtree order when the destination prepends members.
                    if matches!(target_system, LayoutSystemKind::Traditional(_))
                        && matches!(direction, Direction::Right | Direction::Down)
                    {
                        windows.reverse();
                    }
                    for wid in windows {
                        if self.relocate_window_to_workspace(
                            window_store,
                            workspace_id,
                            (space, target_workspace),
                            wid,
                            WindowRelocation::Directional(direction),
                        ) {
                            response.transferred_windows.push(wid);
                        }
                    }
                }
                response
            }
            LayoutCommand::ToggleFullscreen | LayoutCommand::ToggleFullscreenWithinGaps => {
                let within_gaps = matches!(command, LayoutCommand::ToggleFullscreenWithinGaps);
                let windows = self.workspaces[workspace_id]
                    .layout_system
                    .edit(layout, LayoutEdit::Fullscreen { within_gaps })
                    .into_windows();
                Self::response_for_raised_windows(windows)
            }
            // handled by upper reactor
            LayoutCommand::NextWorkspace(_)
            | LayoutCommand::PrevWorkspace(_)
            | LayoutCommand::SwitchToWorkspace(_)
            | LayoutCommand::MoveWindowToWorkspace { .. }
            | LayoutCommand::SetWorkspaceLayout { .. }
            | LayoutCommand::CreateWorkspace
            | LayoutCommand::SwitchToLastWorkspace => EventResponse::default(),
            LayoutCommand::JoinWindow(_)
            | LayoutCommand::ConsumeOrExpelWindow(_)
            | LayoutCommand::UnjoinWindows
            | LayoutCommand::Balance => {
                let edit = match command {
                    LayoutCommand::JoinWindow(direction) => {
                        LayoutEdit::Group(GroupEdit::Join(direction))
                    }
                    LayoutCommand::ConsumeOrExpelWindow(direction) => {
                        LayoutEdit::Group(GroupEdit::ConsumeOrExpel(direction))
                    }
                    LayoutCommand::UnjoinWindows => LayoutEdit::Group(GroupEdit::Unjoin),
                    _ => LayoutEdit::Rebalance,
                };
                self.workspaces[workspace_id].layout_system.edit(layout, edit).discard();
                EventResponse::default()
            }
            LayoutCommand::ToggleStack => self.toggle_stack_for_workspace(
                workspace_id,
                layout,
                self.layout_settings.stack.default_orientation,
            ),
            LayoutCommand::ToggleOrientation => {
                let default_orientation = self.layout_settings.stack.default_orientation;
                Self::toggle_orientation_for_system(
                    &mut self.workspaces[workspace_id].layout_system,
                    layout,
                    default_orientation,
                )
            }
            LayoutCommand::ResizeWindowGrow(_)
            | LayoutCommand::ResizeWindowShrink(_)
            | LayoutCommand::ResizeWindowBy { .. } => {
                if is_floating {
                    return EventResponse::default();
                }
                let (amount, orientation) = match command {
                    LayoutCommand::ResizeWindowGrow(orientation) => (0.05, orientation),
                    LayoutCommand::ResizeWindowShrink(orientation) => (-0.05, orientation),
                    LayoutCommand::ResizeWindowBy { amount } => {
                        (amount.get(), ResizeOrientation::Horizontal)
                    }
                    _ => unreachable!(),
                };
                self.workspaces[workspace_id]
                    .layout_system
                    .edit(layout, LayoutEdit::Resize { amount, orientation })
                    .discard();
                EventResponse::default()
            }
            LayoutCommand::AdjustMasterRatio(_)
            | LayoutCommand::AdjustMasterCount { .. }
            | LayoutCommand::PromoteToMaster
            | LayoutCommand::SwapMasterStack => {
                if let LayoutSystemKind::MasterStack(system) =
                    &mut self.workspaces[workspace_id].layout_system
                {
                    match command {
                        LayoutCommand::AdjustMasterRatio(delta) => {
                            system.adjust_master_ratio(layout, delta.get())
                        }
                        LayoutCommand::AdjustMasterCount { delta } => {
                            system.adjust_master_count(layout, delta)
                        }
                        LayoutCommand::PromoteToMaster => system.promote_to_master(layout),
                        LayoutCommand::SwapMasterStack => system.swap_master_stack(layout),
                        _ => unreachable!(),
                    }
                }
                EventResponse::default()
            }
            LayoutCommand::RotateMasterStackNext | LayoutCommand::RotateMasterStackPrev => {
                let backwards = matches!(command, LayoutCommand::RotateMasterStackPrev);
                let changed = match &mut self.workspaces[workspace_id].layout_system {
                    LayoutSystemKind::MasterStack(system) => {
                        system.rotate_master_stack(layout, backwards)
                    }
                    _ => false,
                };
                if changed {
                    self.forget_native_layout_for_workspace(workspace_id);
                }
                EventResponse {
                    changed,
                    ..EventResponse::default()
                }
            }
            LayoutCommand::ScrollStrip { .. }
            | LayoutCommand::SnapStrip
            | LayoutCommand::SwitchPresetColumnWidth { .. }
            | LayoutCommand::CenterSelection => {
                let LayoutSystemKind::Scrolling(system) =
                    &mut self.workspaces[workspace_id].layout_system
                else {
                    return EventResponse::default();
                };
                let mut response = EventResponse::default();
                match command {
                    LayoutCommand::ScrollStrip { delta } => {
                        if let Some((direction, excess)) =
                            system.scroll_by_delta(layout, delta.get())
                        {
                            let key = (workspace_id, layout, direction);
                            let accumulated = self
                                .scroll_boundary
                                .filter(|(previous, _)| *previous == key)
                                .map_or(excess, |(_, amount)| amount + excess);
                            let boundary = accumulated
                                >= self
                                    .layout_settings
                                    .scrolling
                                    .gestures
                                    .workspace_switch_threshold;
                            self.scroll_boundary = (!boundary).then_some((key, accumulated));
                            response.boundary_hit = boundary.then_some(direction);
                        } else {
                            self.scroll_boundary = None;
                        }
                    }
                    LayoutCommand::SnapStrip => {
                        response.focus_window = system.snap_to_nearest_column(layout);
                        response.changed = response.focus_window.is_some();
                    }
                    LayoutCommand::SwitchPresetColumnWidth { backwards } => {
                        response.changed = system.switch_preset_column_width(layout, backwards);
                    }
                    LayoutCommand::CenterSelection => system.center_selected_column(layout),
                    _ => unreachable!(),
                }
                response
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn calculate_layout(
        &mut self,
        windows: Windows<'_>,
        space: SpaceId,
        screen: CGRect,
        gaps: &crate::common::config::GapSettings,
        stack_line: crate::common::config::StackLineReserve,
    ) -> Vec<(WindowId, CGRect)> {
        let Some((workspace_id, layout)) =
            self.workspaces.reading(windows).active_layout_for_space(space)
        else {
            return Vec::new();
        };
        let shadow = self.shadow_passes.pass(space);
        prepare_scrolling(
            &self.constraint_records(),
            self.constraints.revision(),
            &mut self.workspaces[workspace_id].layout_system,
            layout,
            screen,
            gaps,
            shadow,
        );
        self.calculate_arrangement(workspace_id, layout, screen, gaps, stack_line, shadow)
    }

    pub(crate) fn calculate_layout_with_virtual_workspaces<F>(
        &mut self,
        window_store: Windows<'_>,
        space: SpaceId,
        screen: CGRect,
        gaps: &crate::common::config::GapSettings,
        stack_line: crate::common::config::StackLineReserve,
        get_window_frame: F,
        all_screens: &[CGRect],
    ) -> Vec<(WindowId, CGRect)>
    where
        F: Fn(WindowId) -> Option<CGRect>,
    {
        self.calculate_layout_with_groups(
            window_store,
            space,
            screen,
            gaps,
            stack_line,
            get_window_frame,
            all_screens,
            None,
            false,
        )
    }

    pub(crate) fn calculate_layout_with_groups<F>(
        &mut self,
        window_store: Windows<'_>,
        space: SpaceId,
        screen: CGRect,
        gaps: &crate::common::config::GapSettings,
        stack_line: crate::common::config::StackLineReserve,
        get_window_frame: F,
        all_screens: &[CGRect],
        groups: Option<&mut Vec<GroupContainerInfo>>,
        finish_scrolling: bool,
    ) -> Vec<(WindowId, CGRect)>
    where
        F: Fn(WindowId) -> Option<CGRect>,
    {
        use crate::model::{HiddenWindowPlacement, HideCorner};
        let mut positions = HashMap::default();
        let center = screen.mid();
        let floating_frame = |window, candidate: Option<CGRect>| {
            candidate
                .filter(|rect| !HiddenWindowPlacement::is_hidden(screen, *rect, all_screens))
                .unwrap_or_else(|| {
                    let size = get_window_frame(window)
                        .map_or(CGSize::new(500.0, 500.0), |frame| frame.size);
                    CGRect::new(
                        CGPoint::new(center.x - size.width / 2.0, center.y - size.height / 2.0),
                        size,
                    )
                })
        };

        if let Some(active_workspace_id) =
            self.workspaces.reading(window_store).active_workspace(space)
        {
            if let Some(layout) =
                self.workspaces.reading(window_store).active_layout(space, active_workspace_id)
            {
                let constraints = &self.constraint_records();
                if let LayoutSystemKind::Floating(system) =
                    &mut self.workspaces[active_workspace_id].layout_system
                {
                    system.initialize_frames(layout, screen, constraints, &|wid| {
                        get_window_frame(wid).filter(|frame| {
                            !HiddenWindowPlacement::is_hidden(screen, *frame, all_screens)
                        })
                    });
                }
                let system = &self.workspaces[active_workspace_id].layout_system;
                let frames = groups.and_then(|groups| {
                    let offset = self.layout_settings.stack.stack_offset;
                    if let Some(frames) = system.project_groups(
                        layout,
                        screen,
                        offset,
                        constraints,
                        gaps,
                        stack_line,
                        true,
                        groups,
                        false,
                    ) {
                        return Some(frames);
                    }
                    *groups = system.collect_group_containers(
                        layout,
                        screen,
                        offset,
                        constraints,
                        gaps,
                        stack_line,
                        false,
                    );
                    None
                });
                let records = self.constraint_records();
                positions.extend(frames.unwrap_or_else(|| {
                    let shadow = self.shadow_passes.pass(space);
                    let system = &mut self.workspaces[active_workspace_id].layout_system;
                    prepare_scrolling(
                        &records,
                        self.constraints.revision(),
                        system,
                        layout,
                        screen,
                        gaps,
                        shadow,
                    );
                    if finish_scrolling && let LayoutSystemKind::Scrolling(scrolling) = system {
                        scrolling.finish_viewport(layout);
                    }
                    self.calculate_arrangement(
                        active_workspace_id,
                        layout,
                        screen,
                        gaps,
                        stack_line,
                        shadow,
                    )
                }));
            }

            // Look up this workspace's frames directly. Scanning the entire
            // floating-position store makes every switch depend on windows in
            // unrelated workspaces (and displays).
            for window_id in window_store.workspace_windows(active_workspace_id) {
                if self.floats.is_floating(window_id)
                    && let Some(stored_position) = self.floats.get(active_workspace_id, window_id)
                {
                    let frame = floating_frame(window_id, Some(stored_position));
                    positions.insert(window_id, frame);
                    self.floats.store(active_workspace_id, window_id, frame);
                }
            }

            let floating_windows = self.active_floating_windows_in_workspace(window_store, space);
            for wid in floating_windows {
                let frame = floating_frame(wid, positions.get(&wid).copied());
                positions.insert(wid, frame);
                self.floats.store(active_workspace_id, wid, frame);
            }

            for (wid, frame) in &mut positions {
                if let Some(kind) = self.floats.fullscreen_kind(*wid) {
                    *frame = match kind {
                        FloatingFullscreenKind::Full => screen,
                        FloatingFullscreenKind::WithinGaps => {
                            let o = &gaps.outer;
                            CGRect::new(
                                CGPoint::new(screen.origin.x + o.left, screen.origin.y + o.top),
                                CGSize::new(
                                    screen.size.width - o.left - o.right,
                                    screen.size.height - o.top - o.bottom,
                                ),
                            )
                        }
                    };
                }
            }
        }

        let hidden_windows = self
            .workspaces
            .reading(window_store)
            .windows_in_inactive_workspaces(window_store, space);
        for wid in hidden_windows {
            let original_frame = get_window_frame(wid);

            if self.floats.is_floating(wid)
                && let Some(workspace_id) =
                    self.workspaces().reading(window_store).workspace_for_window(space, wid)
                && self.floats.get(workspace_id, wid).is_none()
            {
                let candidate = original_frame.or_else(|| positions.get(&wid).copied());
                self.floats.store_if_absent(workspace_id, wid, floating_frame(wid, candidate));
            }

            let reference_frame = original_frame
                .unwrap_or_else(|| CGRect::new(screen.origin, CGSize::new(500.0, 500.0)));
            let hidden_rect = HiddenWindowPlacement::calculate(
                screen,
                reference_frame,
                HideCorner::BottomRight,
                all_screens,
            );
            positions.insert(wid, hidden_rect);
        }

        // Membership survives native fullscreen; geometry never targets it.
        positions
            .into_iter()
            .filter(|(wid, _)| !window_store.is_temporarily_unavailable(*wid))
            .collect()
    }

    pub(crate) fn active_workspace_for_space_has_fullscreen(
        &mut self,
        windows: Windows<'_>,
        space: SpaceId,
    ) -> bool {
        let Some((ws_id, layout_id)) =
            self.workspaces.reading(windows).active_layout_for_space(space)
        else {
            return false;
        };
        self.workspaces[ws_id].layout_system.has_any_fullscreen_node(layout_id)
    }

    pub(crate) fn workspace_has_fullscreen(
        &self,
        window_store: Windows<'_>,
        space: SpaceId,
        workspace: VirtualWorkspaceId,
        windows: &[WindowId],
    ) -> bool {
        if windows.is_empty() {
            return false;
        }
        self.workspaces
            .reading(window_store)
            .active_layout(space, workspace)
            .is_some_and(|layout_id| {
                self.workspaces[workspace].layout_system.has_any_fullscreen_node(layout_id)
            })
            || windows.iter().any(|wid| self.floats.fullscreen_kind(*wid).is_some())
    }

    pub(crate) fn collect_group_containers(
        &mut self,
        windows: Windows<'_>,
        space: SpaceId,
        screen: CGRect,
        gaps: &crate::common::config::GapSettings,
        stack_line: crate::common::config::StackLineReserve,
    ) -> Vec<GroupContainerInfo> {
        self.collect_group_containers_for_space(windows, space, screen, gaps, stack_line, false)
    }

    /// Scrolling strips expose their calculated coordinates before native offscreen parking.
    pub(crate) fn logical_window_frames(
        &self,
        windows: Windows<'_>,
        space: SpaceId,
        workspace: VirtualWorkspaceId,
        screen: CGRect,
        gaps: &crate::common::config::GapSettings,
    ) -> HashMap<WindowId, CGRect> {
        match (
            &self.workspaces[workspace].layout_system,
            self.workspaces.reading(windows).active_layout(space, workspace),
        ) {
            (LayoutSystemKind::Scrolling(system), Some(layout)) => system
                .calculate_frames(
                    layout,
                    screen,
                    &self.constraint_records(),
                    Some(self.constraints.revision()),
                    gaps,
                    false,
                    // A query is not a Space's layout pass.
                    crate::layout_engine::Shadow::outside_a_pass(),
                )
                .into_iter()
                .collect(),
            _ => HashMap::default(),
        }
    }

    pub(crate) fn calculate_layout_for_workspace(
        &self,
        window_store: Windows<'_>,
        space: SpaceId,
        workspace_id: crate::model::VirtualWorkspaceId,
        screen: CGRect,
        gaps: &crate::common::config::GapSettings,
        stack_line: crate::common::config::StackLineReserve,
    ) -> Vec<(WindowId, CGRect)> {
        let mut positions: HashMap<_, _> = self
            .calculate_workspace_layout(window_store, space, workspace_id, screen, gaps, stack_line)
            .into_iter()
            .collect();

        let floating_positions = self.floats.workspace_positions(workspace_id);
        for (window_id, stored_position) in floating_positions {
            if self.floats.is_floating(window_id)
                && window_store.workspace_for_window(window_id) == Some(workspace_id)
            {
                positions.insert(window_id, stored_position);
            }
        }

        positions.into_iter().collect()
    }

    pub(crate) fn selected_window(
        &mut self,
        windows: Windows<'_>,
        space: SpaceId,
    ) -> Option<WindowId> {
        let (ws_id, layout) = self.workspaces.reading(windows).active_layout_for_space(space)?;
        self.workspaces[ws_id].layout_system.selected_window(layout)
    }

    pub(crate) fn handle_virtual_workspace_command<'a>(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        context: impl Into<crate::model::virtual_workspace::WorkspaceContext<'a>>,
        command: &LayoutCommand,
    ) -> EventResponse {
        let context = context.into();
        let space = context.space;
        match command {
            LayoutCommand::NextWorkspace(skip_empty) | LayoutCommand::PrevWorkspace(skip_empty) => {
                let Some(current) =
                    self.workspaces.reading(window_store.read()).active_workspace(space)
                else {
                    return EventResponse::default();
                };
                let direction = if matches!(command, LayoutCommand::NextWorkspace(_)) {
                    Direction::Right
                } else {
                    Direction::Left
                };
                let target = self.workspaces.reading(window_store.read()).step_workspace(
                    context,
                    current,
                    *skip_empty,
                    direction,
                );
                // Global targets on another display are shown there and focus follows.
                if let Some(target) = target.filter(|&target| target != current) {
                    return self.switch_to_workspace_id(window_store, context, target, None);
                }
                EventResponse::default()
            }
            LayoutCommand::SwitchToWorkspace(workspace_index) => {
                let Some(target) = self
                    .workspaces
                    .reading(window_store.read())
                    .resolve_workspace(context, &WorkspaceSelector::Index(*workspace_index))
                else {
                    return EventResponse::default();
                };
                self.switch_to_workspace_id(window_store, context, target.id, None)
            }
            LayoutCommand::MoveWindowToWorkspace {
                workspace,
                follow,
                window_id: maybe_id,
            } => {
                let window = match maybe_id {
                    Some(index) => {
                        self.workspaces.reading(window_store.read()).resolve_window_index(
                            Some(space),
                            |space| context.visible.contains(&space),
                            *index,
                        )
                    }
                    None => self.floats.focused(),
                };
                let Some(focused_window) =
                    window.filter(|&window| window_store.read().can_move_assigned_window(window))
                else {
                    return EventResponse::default();
                };
                let current_workspace_id =
                    window_store.read().workspace_for_window(focused_window).unwrap();
                let Some(op_space) = self
                    .workspaces
                    .reading(window_store.read())
                    .workspace_space(current_workspace_id)
                else {
                    return EventResponse::default();
                };
                let target_workspace_id = match workspace {
                    WorkspaceSelector::Name(name) if name == "next" || name == "prev" => {
                        let direction = if name == "next" {
                            Direction::Right
                        } else {
                            Direction::Left
                        };
                        self.workspaces.reading(window_store.read()).step_workspace(
                            WorkspaceContext {
                                space: op_space,
                                visible: context.visible,
                            },
                            current_workspace_id,
                            None,
                            direction,
                        )
                    }
                    selector => self
                        .workspaces
                        .reading(window_store.read())
                        .resolve_workspace(context, selector)
                        .map(|target| target.id),
                };
                let Some(target_workspace_id) = target_workspace_id else {
                    return EventResponse::default();
                };

                if current_workspace_id == target_workspace_id {
                    return EventResponse::default();
                }
                let target_space = self
                    .workspaces
                    .reading(window_store.read())
                    .workspace_space(target_workspace_id)
                    .expect("a listed workspace has a Space");
                if !self.relocate_window_to_workspace(
                    window_store,
                    current_workspace_id,
                    (target_space, target_workspace_id),
                    focused_window,
                    WindowRelocation::Workspace,
                ) {
                    return EventResponse::default();
                }
                let placeable = window_store.read().can_place_window(focused_window);
                if *follow || target_space != op_space {
                    let mut response = if *follow {
                        self.activate_workspace(
                            window_store,
                            target_space,
                            target_workspace_id,
                            placeable.then_some(focused_window),
                        )
                    } else {
                        EventResponse {
                            changed: true,
                            ..Default::default()
                        }
                    };
                    response.changed = true;
                    response.layout_space = Some(op_space);
                    if target_space != op_space {
                        response.transferred_windows.push(focused_window);
                    }
                    return response;
                }

                if !placeable {
                    return EventResponse {
                        changed: true,
                        layout_space: Some(op_space),
                        ..EventResponse::default()
                    };
                }
                let active_workspace =
                    self.workspaces.reading(window_store.read()).active_workspace(op_space);

                if Some(target_workspace_id) == active_workspace {
                    return EventResponse {
                        changed: true,
                        layout_space: Some(op_space),
                        focus_window: Some(focused_window),
                        raise_windows: vec![],

                        ..EventResponse::default()
                    };
                } else if Some(current_workspace_id) == active_workspace {
                    self.floats.set_focused(None);
                    self.workspaces.set_last_focused_window(
                        &window_store.core,
                        op_space,
                        current_workspace_id,
                        None,
                    );

                    let remaining_windows = self
                        .workspaces
                        .reading(window_store.read())
                        .windows_in_active_workspace(window_store.read(), op_space);
                    if let Some(&new_focus) = remaining_windows.first() {
                        return EventResponse {
                            changed: true,
                            layout_space: Some(op_space),
                            focus_window: Some(new_focus),
                            raise_windows: vec![],

                            ..EventResponse::default()
                        };
                    }
                }

                self.workspaces.set_last_focused_window(
                    &window_store.core,
                    op_space,
                    target_workspace_id,
                    Some(focused_window),
                );

                EventResponse {
                    changed: true,
                    layout_space: Some(op_space),
                    ..EventResponse::default()
                }
            }
            LayoutCommand::CreateWorkspace => {
                match self.workspaces.create_workspace(window_store, space, None) {
                    Ok(_workspace_id) => {
                        self.sync_scrolling_widths_for_space(window_store.read(), space);
                        EventResponse {
                            changed: true,
                            ..EventResponse::default()
                        }
                    }
                    Err(e) => {
                        warn!("Failed to create new workspace: {:?}", e);
                        EventResponse::default()
                    }
                }
            }
            LayoutCommand::SwitchToLastWorkspace => {
                self.switch_to_previous_workspace(window_store, context)
            }
            LayoutCommand::SetWorkspaceLayout { workspace, mode } => {
                let workspace_id = if let Some(index) = workspace {
                    self.workspaces
                        .reading(window_store.read())
                        .resolve_workspace(context, &WorkspaceSelector::Index(*index))
                        .map(|target| target.id)
                } else {
                    self.workspaces.reading(window_store.read()).active_workspace(space)
                };
                let Some(workspace_id) = workspace_id else {
                    return EventResponse::default();
                };
                let space = self
                    .workspaces
                    .reading(window_store.read())
                    .workspace_space(workspace_id)
                    .expect("a listed workspace has a Space");

                if !self.switch_workspace_layout_mode(
                    window_store.read(),
                    space,
                    workspace_id,
                    *mode,
                ) {
                    return EventResponse::default();
                }

                let is_active_workspace =
                    self.workspaces.reading(window_store.read()).active_workspace(space)
                        == Some(workspace_id);
                let raise_windows = if is_active_workspace {
                    self.workspaces
                        .reading(window_store.read())
                        .windows_in_active_workspace(window_store.read(), space)
                } else {
                    Vec::new()
                };

                EventResponse {
                    changed: true,
                    layout_space: Some(space),
                    raise_windows,
                    focus_window: if is_active_workspace {
                        self.floats.focused()
                    } else {
                        None
                    },

                    ..EventResponse::default()
                }
            }
            _ => EventResponse::default(),
        }
    }

    pub fn workspaces(&self) -> &WorkspaceStore { &self.workspaces }

    pub fn workspaces_mut(&mut self) -> &mut WorkspaceStore { &mut self.workspaces }

    pub(crate) fn assign_window_with_app_info<'a>(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        window_id: WindowId,
        workspace_context: impl Into<crate::model::virtual_workspace::WorkspaceContext<'a>>,
        context: WindowRuleContext<'_>,
        reapply_workspace_rule: bool,
    ) -> Result<Option<AppRuleResult>, crate::model::virtual_workspace::WorkspaceError> {
        let mut decision = {
            let window = window_store.window(window_id).map(|window| &window.info);
            let context = WindowRuleContext {
                window_title: context
                    .window_title
                    .or_else(|| window.map(|info| info.title.as_str())),
                ax_role: context
                    .ax_role
                    .or_else(|| window.and_then(|info| info.ax_role.as_deref())),
                ax_subrole: context
                    .ax_subrole
                    .or_else(|| window.and_then(|info| info.ax_subrole.as_deref())),
                ..context
            };
            self.app_rules.evaluate_observation(context, reapply_workspace_rule, || {
                window
                    .and_then(|info| info.sys_id)
                    .and_then(crate::sys::window_server::window_tags)
            })
        };
        if window_store.read().is_claimed(window_id) {
            window_store.set_manage_override(
                window_id,
                decision.as_ref().and_then(|rule| rule.management_override()),
            );
            return Ok(None);
        }
        let workspace_context = workspace_context.into();
        let space = workspace_context.space;
        let observation =
            window_store.window(window_id).filter(|_| self.is_restoring()).map(|window| {
                (
                    context.window_title.unwrap_or(&window.info.title).to_owned(),
                    window.frame_monotonic().size,
                )
            });
        let restored = observation.is_some_and(|(title, size)| {
            self.observe_window_for_persistence(
                window_store,
                space,
                window_id,
                Some(&title),
                size,
                context.app_bundle_id,
            )
        });
        // A persistence match is an explicit restoration of the user's previous
        // workspace. App rules still control admission and other effects, but
        // their default placement must not relocate the window during restore.
        if restored && let Some(decision) = &mut decision {
            decision.workspace = None;
        }
        if reapply_workspace_rule
            && decision.as_ref().is_some_and(|decision| decision.workspace.is_some())
        {
            self.forget_native_layout_for_window(window_store.read(), window_id);
        }
        self.workspaces
            .apply_app_rule_decision(
                window_store,
                window_id,
                workspace_context,
                decision,
                !reapply_workspace_rule,
            )
            .map(Some)
    }

    pub(crate) fn ensure_active_workspace_info(
        &mut self,
        windows: &mut WindowsMut<'_>,
        space: SpaceId,
    ) -> Option<(crate::model::VirtualWorkspaceId, String)> {
        // Initialising shows what it chooses (the default workspace, or the first one a Space
        // owns); a Space it leaves vacant owns no workspace, so there is none to show.
        self.workspaces.ensure_space_initialized(windows, space);
        self.active_workspace_id_and_name(windows.read(), space)
    }

    pub(crate) fn move_window_to_space(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        source_space: SpaceId,
        target_space: SpaceId,
        target_screen_size: CGSize,
        window_id: WindowId,
    ) -> EventResponse {
        let Some(source_workspace_id) = self
            .workspaces
            .reading(window_store.read())
            .workspace_for_window(source_space, window_id)
        else {
            return EventResponse::default();
        };

        let Some(target_workspace_id) =
            self.workspaces.reading(window_store.read()).active_workspace(target_space)
        else {
            debug!(
                ?window_id,
                ?target_space,
                "Window move ignored: destination has no workspace"
            );
            return EventResponse::default();
        };

        self.ensure_workspace_layouts(window_store, target_space, target_screen_size);
        if !self.relocate_window_to_workspace(
            window_store,
            source_workspace_id,
            (target_space, target_workspace_id),
            window_id,
            WindowRelocation::Workspace,
        ) {
            return EventResponse::default();
        }

        if !window_store.read().can_place_window(window_id) {
            return EventResponse {
                changed: true,
                ..EventResponse::default()
            };
        }
        self.workspaces.set_last_focused_window(
            &window_store.core,
            target_space,
            target_workspace_id,
            Some(window_id),
        );
        self.floats.set_focused(Some(window_id));

        EventResponse {
            changed: true,
            raise_windows: vec![window_id],
            focus_window: Some(window_id),

            ..EventResponse::default()
        }
    }

    /// Transfer the workspace object; no window changes logical workspace identity.
    pub(crate) fn move_active_workspace_to_space(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        source_space: SpaceId,
        target_space: SpaceId,
        target_screen_size: CGSize,
    ) -> EventResponse {
        let Some(id) = self.workspaces.reading(window_store.read()).active_workspace(source_space)
        else {
            return EventResponse::default();
        };
        if self.workspaces.effective_mode() == crate::common::config::WorkspaceMode::PerDisplay {
            if source_space == target_space {
                return EventResponse::default();
            }
            let Some(index) =
                self.workspaces.reading(window_store.read()).active_workspace_idx(source_space)
            else {
                return EventResponse::default();
            };
            // Moving onto a Space initialises it, as placing a window there does.
            self.workspaces.ensure_space_initialized(window_store, target_space);
            let Some(destination) = self
                .workspaces
                .reading(window_store.read())
                .workspace_id_at(target_space, Some(index as usize))
            else {
                return EventResponse::default();
            };
            let focus =
                self.preferred_focus_for_workspace(window_store.read(), source_space, id, None);
            let members = window_store.read().workspace_windows(id);
            self.ensure_workspace_layouts(window_store, target_space, target_screen_size);
            let moved: Vec<_> = members
                .into_iter()
                .filter(|&window| {
                    self.relocate_window_to_workspace(
                        window_store,
                        id,
                        (target_space, destination),
                        window,
                        WindowRelocation::Workspace,
                    )
                })
                .collect();
            let changed =
                self.workspaces.reading(window_store.read()).active_workspace(target_space)
                    != Some(destination);
            self.workspaces.set_active_workspace(window_store, target_space, destination);
            return EventResponse {
                changed: changed || !moved.is_empty(),
                workspace_space: Some(target_space),
                focus_window: focus
                    .filter(|window| moved.contains(window))
                    .or_else(|| moved.first().copied()),
                transferred_windows: moved.clone(),
                raise_windows: moved,
                ..EventResponse::default()
            };
        }
        let display = self.display_uuid_for_space(target_space).map(str::to_owned);
        self.transfer_workspace(
            window_store,
            id,
            target_space,
            target_screen_size,
            display.as_deref(),
            crate::model::virtual_workspace::WorkspaceTransferReason::Explicit,
        )
    }

    /// Whether an app rule names the workspace for a window with this context (read by the
    /// reactor's new-window placement question; upstream removed it in #700).
    pub fn app_rule_names_workspace(&self, context: crate::model::WindowRuleContext<'_>) -> bool {
        self.app_rules
            .evaluate(context)
            .is_some_and(|decision| decision.workspace.is_some())
    }

    pub(crate) fn transfer_workspace(
        &mut self,
        windows: &mut WindowsMut<'_>,
        id: VirtualWorkspaceId,
        target: SpaceId,
        size: CGSize,
        display: Option<&str>,
        reason: crate::model::virtual_workspace::WorkspaceTransferReason,
    ) -> EventResponse {
        let source = self
            .workspaces
            .reading(windows.read())
            .workspace_space(id)
            .expect("a listed workspace has a Space");
        let source_size = self.workspaces[id].layout_state.active_size();
        let members = windows.read().workspace_windows(id);
        let focus = self.preferred_focus_for_workspace(windows.read(), source, id, None);
        if !self.workspaces.transfer_workspace(windows, id, target, display, reason) {
            return EventResponse::default();
        }
        self.native.step(MemoryEvent::Forgotten(id));
        match rift_core::workspaces::source_after(reason) {
            SourceAfter::Relaid => {
                if let Some(source_size) = source_size {
                    self.ensure_workspace_layouts(windows, source, source_size);
                }
                let display = self.display_uuid_for_space(source).map(str::to_owned);
                self.update_space_display(windows.read(), source, display.as_deref());
            }
            SourceAfter::Untouched => {}
        }
        self.sync_scrolling_widths_for_space(windows.read(), target);
        self.workspaces[id].ensure_layout_for_size(size);
        for &window in &members {
            self.evicted.step(EvictionEvent::Forgotten(window));
        }

        EventResponse {
            changed: true,
            workspace_space: Some(target),
            raise_windows: members.clone(),
            transferred_windows: match rift_core::workspaces::moved(source, target) {
                Moved::InPlace => Vec::new(),
                Moved::Across => members,
            },
            focus_window: focus,
            ..EventResponse::default()
        }
    }

    fn ensure_workspace_layouts(
        &mut self,
        windows: &mut WindowsMut<'_>,
        space: SpaceId,
        screen_size: CGSize,
    ) {
        self.workspaces.ensure_layouts_for_size(windows, space, screen_size);
        self.sync_scrolling_widths_for_space(windows.read(), space);
    }

    fn relocate_window_to_workspace(
        &mut self,
        windows: &mut WindowsMut<'_>,
        source: VirtualWorkspaceId,
        (space, destination): (SpaceId, VirtualWorkspaceId),
        window: WindowId,
        policy: WindowRelocation,
    ) -> bool {
        let Some(source_workspace) = self.workspaces.workspaces.get(source) else {
            return false;
        };
        let Some(target_workspace) = self.workspaces.workspaces.get(destination) else {
            return false;
        };
        let observed = matches!(policy, WindowRelocation::Observed);
        let suspended = windows.is_temporarily_unavailable(window)
            || (matches!(policy, WindowRelocation::Workspace)
                && !windows.read().can_place_window(window));
        let owner = windows.read().workspace_for_window(window);
        // App rules can commit assignment before observation applies the layout change.
        if self.workspaces.reading(windows.read()).workspace_space(destination) != Some(space)
            || owner.is_some_and(|owner| owner != source && !(observed && owner == destination))
        {
            return false;
        }

        let source_floating_layout =
            matches!(source_workspace.layout_system, LayoutSystemKind::Floating(_));
        let target_floating_layout =
            matches!(target_workspace.layout_system, LayoutSystemKind::Floating(_));
        let directional = match policy {
            WindowRelocation::Directional(direction) => Some(direction),
            _ => None,
        };
        let preserve_floating = observed || matches!(policy, WindowRelocation::Drop(_));
        let was_floating = if preserve_floating {
            self.is_window_floating(window)
        } else {
            self.floats.is_floating(window)
                && (directional.is_some() || !(source_floating_layout || target_floating_layout))
        };
        let target_layout =
            self.workspaces.reading(windows.read()).active_layout(space, destination);
        if (directional.is_some() && target_layout.is_none())
            || (owner != Some(destination)
                && !self.workspaces.assign_window_to_workspace(windows, space, window, destination))
        {
            return false;
        }

        self.forget_native_layout_for_window_id(window);
        self.native.step(MemoryEvent::Forgotten(source));
        self.native.step(MemoryEvent::Forgotten(destination));
        self.workspaces[source].layout_system.update(WindowChange::Remove(window));
        let project_as_floating = directional.is_none() && was_floating && !target_floating_layout;
        if directional.is_none() {
            if project_as_floating {
                self.floats.add_floating(window);
                if observed {
                    self.workspaces[destination].layout_system.update(WindowChange::Remove(window));
                }
            } else {
                self.floats.remove_floating(window);
            }
            if was_floating {
                self.floats.remove_window(window);
                self.floats.set_last_focus(Some(window));
            }
        }

        if !project_as_floating {
            if (!was_floating || directional.is_some())
                && matches!(
                    self.workspaces[destination].layout_system,
                    LayoutSystemKind::Scrolling(_)
                )
                && self.layout_settings.scrolling.preserve_window_sizes
                && target_layout.is_some()
            {
                self.preserve_scrolling_window_width(windows.read(), window);
            }
            if let WindowRelocation::Drop(Some(edited)) = policy {
                self.workspaces[destination].layout_system = edited;
            } else if let Some(layout) = target_layout {
                self.workspaces[destination].layout_system.update(WindowChange::Add(
                    layout,
                    window,
                    directional,
                ));
            }
        }

        if suspended {
            self.suspend_window(windows.read(), window);
        } else if observed {
            self.workspaces[destination]
                .layout_system
                .set_window_participation(window, true);
        }
        if directional.is_none() && self.floats.focused() == Some(window) {
            self.floats.set_focused(None);
        }
        true
    }

    pub(crate) fn workspace_name(
        &self,
        windows: Windows<'_>,
        space: SpaceId,
        workspace_id: crate::model::VirtualWorkspaceId,
    ) -> Option<String> {
        self.workspaces
            .reading(windows)
            .workspace_info(space, workspace_id)
            .map(|ws| ws.name.clone())
    }

    pub fn is_window_floating(&self, window_id: WindowId) -> bool {
        self.floats.is_floating(window_id)
            || self.workspaces.workspaces.values().any(|ws| {
                matches!(&ws.layout_system, LayoutSystemKind::Floating(system) if system.contains_floating_window(window_id))
            })
    }

    /// Requested geometry is presentation intent, never native membership evidence. The
    /// model's frame for the window is the core's (the caller tells it first).
    pub(crate) fn prepare_window_placement(
        &mut self,
        windows: &mut WindowsMut<'_>,
        window: WindowId,
        frame: CGRect,
    ) {
        if self.is_window_floating(window)
            && let Some(assignment) = windows.read().workspace_for_window(window)
        {
            self.store_floating_position(assignment, window, frame);
        }
    }

    pub fn store_floating_position(
        &mut self,
        workspace: VirtualWorkspaceId,
        window: WindowId,
        frame: CGRect,
    ) {
        if let LayoutSystemKind::Floating(system) = &mut self.workspaces[workspace].layout_system {
            system.store_frame(window, frame);
            return;
        }
        self.floats.store(workspace, window, frame);
    }

    pub fn get_floating_position(
        &self,
        workspace: VirtualWorkspaceId,
        window: WindowId,
    ) -> Option<CGRect> {
        if let LayoutSystemKind::Floating(system) = &self.workspaces[workspace].layout_system {
            return system.frame(window);
        }
        self.floats.get(workspace, window)
    }

    pub fn workspace_floating_positions(
        &self,
        workspace: VirtualWorkspaceId,
    ) -> Vec<(WindowId, CGRect)> {
        self.floats.workspace_positions(workspace)
    }

    pub fn remove_floating_position(&mut self, window: WindowId) {
        self.floats.remove_window(window);
    }

    /// A window's size constraints live as long as the window, so only its retirement drops
    /// them. Leaving a layout doesn't: a move between Spaces arrives as a layout removal.
    pub(crate) fn retire_window(&mut self, wid: WindowId) {
        self.constraints.step(rift_core::constraints::ConstraintsEvent::Retired(wid));
    }

    /// Every window's constraints in the engine's shape, from the core's records: computed for
    /// each pass that reads them, never kept.
    fn constraint_records(&self) -> HashMap<WindowId, WindowLayoutConstraints> {
        self.constraints
            .windows()
            .map(|(window, c)| (window, super::core_geometry::engine_constraints(c)))
            .collect()
    }

    /// What an observation reports of `window`'s size, as the constraints machine reads it.
    fn observed_size(
        resizable: bool,
        size: CGSize,
        min: Option<CGSize>,
        max: Option<CGSize>,
    ) -> rift_core::constraints::Observed {
        rift_core::constraints::Observed {
            resizable,
            width: size.width,
            height: size.height,
            min: min.and_then(|min| rift_core::layout::Size::reported(min.width, min.height)),
            max_width: max.map_or(0.0, |s| s.width),
            max_height: max.map_or(0.0, |s| s.height),
        }
    }

    #[cfg(test)]
    pub(crate) fn has_window_constraints(&self, wid: WindowId) -> bool {
        self.constraints.get(wid).is_some()
    }

    pub(crate) fn transfer_persistent_window_identity(&mut self, from: WindowId, to: WindowId) {
        if from == to {
            return;
        }

        self.evicted.step(EvictionEvent::Replaced { from, to });
        self.edit_native_snapshots(to, WindowChange::Remove(to));
        self.edit_native_snapshots(from, WindowChange::Replace(from, to));
        // A live `to` identity can already be provisionally present when a saved `from` identity
        // is matched. Remove that projection before replacement; LayoutSystem::replace_window is
        // not required to deduplicate and otherwise the same window can survive in two workspaces.
        self.remove_window_from_all_tiling_trees(to);
        for (_, workspace) in self.workspaces.workspaces.iter_mut() {
            workspace.layout_system.update(WindowChange::Replace(from, to));
        }
        self.workspaces.transfer_window_identity(from, to);
        self.floats.replace_identity(from, to);
        self.constraints
            .step(rift_core::constraints::ConstraintsEvent::Replaced { from, to });
        if self.floats.focused() == Some(from) {
            self.floats.set_focused(Some(to));
        }
    }

    pub(crate) fn store_floating_window_positions(
        &mut self,
        windows: Windows<'_>,
        space: SpaceId,
        floating_positions: &[(WindowId, CGRect)],
    ) {
        if let Some(workspace) = self.workspaces.reading(windows).active_workspace(space) {
            for &(window, frame) in floating_positions {
                self.store_floating_position(workspace, window, frame);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::{CGPoint, CGSize};

    use super::*;
    use crate::common::collections::HashMap;
    use crate::common::config::{
        LayoutMode, LayoutSettings, VirtualWorkspaceSettings, WorkspaceSelector,
    };
    use crate::layout_engine::systems::{Focus, WindowQuery};
    use crate::model::window_store::OwnedWindows;

    #[test]
    fn simultaneous_suspensions_keep_workspace_owners_through_inactive_returns_and_mode_changes() {
        for mode in [
            LayoutMode::Traditional,
            LayoutMode::Bsp,
            LayoutMode::Stack,
            LayoutMode::MasterStack,
            LayoutMode::Scrolling,
        ] {
            let mut settings = LayoutSettings::default();
            settings.mode = mode;
            let mut engine = LayoutEngine::new(&VirtualWorkspaceSettings::default(), &settings);
            let mut store = OwnedWindows::default();
            let space = SpaceId::new(91);
            let _ = engine.handle_event(
                &mut store.view_mut(),
                LayoutEvent::SpaceExposed(space, CGSize::new(1200.0, 800.0)),
            );
            let owner = engine.workspaces.reading(store.view()).active_workspace(space).unwrap();
            let other = engine.workspaces.listed_after_init(&mut store.view_mut(), space)[1].0;
            let windows = [WindowId::new(1, 1), WindowId::new(2, 1)];
            for &window in &windows {
                store
                    .view_mut()
                    .insert_manageable_window(window, live_window(CGSize::new(500.0, 500.0)));
                assert!(engine.workspaces.assign_window_to_workspace(
                    &mut store.view_mut(),
                    space,
                    window,
                    owner
                ));
                let _ = engine
                    .handle_event(&mut store.view_mut(), LayoutEvent::WindowAdded(space, window));
            }
            assert!(engine.relocate_window_to_workspace(
                &mut store.view_mut(),
                owner,
                (space, other),
                windows[1],
                WindowRelocation::Observed
            ));
            for &window in &windows {
                store.invalidate_ax_identity(window);
                let _ = engine
                    .handle_event(&mut store.view_mut(), LayoutEvent::WindowSuspended(window));
            }
            let _ = engine.handle_event(
                &mut store.view_mut(),
                LayoutEvent::WindowFocused(space, windows[0]),
            );
            assert_eq!(
                engine.focused_window(),
                None,
                "stale focus must not reactivate a suspended member"
            );
            assert!(engine.native.is_empty(), "suspension never snapshots topology");
            engine.workspaces.set_active_workspace(&mut store.view_mut(), space, other);
            store.validate_ax_identity(windows[0]);
            let _ = engine.handle_event(
                &mut store.view_mut(),
                LayoutEvent::WindowAdded(space, windows[0]),
            );
            assert_eq!(
                engine.workspaces.reading(store.view()).active_workspace(space),
                Some(other)
            );
            assert_eq!(store.view().workspace_for_window(windows[0]), Some(owner));
            assert!(
                engine.workspaces[owner].layout_system.window_participates(windows[0]),
                "a confirmed AX return restores its slot without selecting its workspace"
            );
            assert!(!engine.workspaces[other].layout_system.window_participates(windows[1]));
            let next_mode = if mode == LayoutMode::Bsp {
                LayoutMode::Traditional
            } else {
                LayoutMode::Bsp
            };
            engine.switch_workspace_layout_mode(store.view(), space, other, next_mode);
            assert!(!engine.workspaces[other].layout_system.window_participates(windows[1]));
            store.validate_ax_identity(windows[1]);
            let _ = engine.handle_event(
                &mut store.view_mut(),
                LayoutEvent::WindowAdded(space, windows[1]),
            );
            assert_eq!(store.view().workspace_for_window(windows[1]), Some(other));
            store.view_mut().remove_window(windows[0]);
            let _ =
                engine.handle_event(&mut store.view_mut(), LayoutEvent::WindowRemoved(windows[0]));
            assert!(
                engine.workspaces[owner]
                    .layout_system
                    .windows(
                        engine.workspaces[owner].active_layout().unwrap(),
                        WindowQuery::All
                    )
                    .is_empty()
            );
            let _ =
                engine.handle_event(&mut store.view_mut(), LayoutEvent::AppClosed(windows[1].pid));
            assert!(
                engine.workspaces[other]
                    .layout_system
                    .windows(
                        engine.workspaces[other].active_layout().unwrap(),
                        WindowQuery::All
                    )
                    .is_empty()
            );
        }
    }

    fn test_engine() -> LayoutEngine {
        LayoutEngine::new(&VirtualWorkspaceSettings::default(), &LayoutSettings::default())
    }

    fn live_window(size: CGSize) -> crate::model::reactor::WindowState {
        let frame = CGRect::new(CGPoint::ZERO, size);
        crate::actor::reactor::testing::make_window_info(frame, None, "Window", None).into()
    }

    fn window_layout_info(window_id: WindowId, current_size: CGSize) -> WindowLayoutInfo {
        WindowLayoutInfo {
            window_id,
            title: None,
            ax_role: None,
            ax_subrole: None,
            is_resizable: true,
            current_size,
            min_size: None,
            max_size: None,
        }
    }

    fn test_display_frame(center: CGPoint) -> CGRect {
        CGRect::new(
            CGPoint::new(center.x - 500.0, center.y - 500.0),
            CGSize::new(1000.0, 1000.0),
        )
    }

    #[test]
    fn move_node_crosses_display_edge_and_enters_at_near_edge() {
        for direction in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ] {
            for stacked in [false, true] {
                let mut engine = test_engine();
                let mut store = OwnedWindows::default();
                let source = SpaceId::new(1);
                let target = SpaceId::new(2);
                let size = CGSize::new(1000.0, 1000.0);
                let moving = WindowId::new(1, 2);
                for (space, ids) in [(source, vec![1, 2]), (target, vec![3, 4, 5])] {
                    for &idx in &ids {
                        store
                            .view_mut()
                            .insert_manageable_window(WindowId::new(1, idx), live_window(size));
                    }
                    let _ = engine.handle_event(
                        &mut store.view_mut(),
                        LayoutEvent::SpaceExposed(space, size),
                    );
                    let _ = engine.handle_event(
                        &mut store.view_mut(),
                        LayoutEvent::windows_observed(
                            space,
                            1,
                            ids.into_iter()
                                .map(|idx| window_layout_info(WindowId::new(1, idx), size))
                                .collect(),
                            None,
                        ),
                    );
                    let (ws, layout) = engine
                        .workspaces
                        .reading(store.view())
                        .active_layout_for_space(space)
                        .unwrap();
                    let LayoutSystemKind::Traditional(system) =
                        &mut engine.workspaces[ws].layout_system
                    else {
                        panic!("expected traditional layout");
                    };
                    if stacked {
                        system
                            .edit(
                                layout,
                                LayoutEdit::Group(GroupEdit::Stack(
                                    crate::common::config::StackDefaultOrientation::Vertical,
                                )),
                            )
                            .discard();
                    } else if matches!(direction, Direction::Up | Direction::Down) {
                        assert!(system.edit(layout, LayoutEdit::ToggleOrientation).changed());
                    }
                    // Select the far side on the destination, so selection-based insertion is wrong.
                    let selected = if space == source {
                        moving
                    } else {
                        WindowId::new(1, 5)
                    };
                    system.focus(layout, Focus::Select(selected)).handled;
                    if space == source
                        && (direction == Direction::Up
                            || (direction == Direction::Left && !stacked))
                    {
                        system.focus(layout, Focus::Select(WindowId::new(1, 1))).handled;
                    }
                }
                let moving =
                    if direction == Direction::Up || (direction == Direction::Left && !stacked) {
                        WindowId::new(1, 1)
                    } else {
                        moving
                    };
                let _ = engine.handle_event(
                    &mut store.view_mut(),
                    LayoutEvent::WindowFocused(source, moving),
                );
                let offset = match direction {
                    Direction::Left => CGPoint::new(-1000.0, 0.0),
                    Direction::Right => CGPoint::new(1000.0, 0.0),
                    Direction::Up => CGPoint::new(0.0, -1000.0),
                    Direction::Down => CGPoint::new(0.0, 1000.0),
                };
                let centers = HashMap::from_iter([
                    (source, test_display_frame(CGPoint::new(0.0, 0.0))),
                    (target, test_display_frame(offset)),
                ]);
                let _ = engine.handle_command(
                    &mut store.view_mut(),
                    Some(source),
                    &[source, target],
                    &centers,
                    LayoutCommand::MoveNode(direction),
                );
                let (source_ws, source_layout) = engine
                    .workspaces
                    .reading(store.view())
                    .active_layout_for_space(source)
                    .unwrap();
                let (target_ws, target_layout) = engine
                    .workspaces
                    .reading(store.view())
                    .active_layout_for_space(target)
                    .unwrap();
                assert!(
                    !engine.workspaces[source_ws]
                        .layout_system
                        .contains_window(source_layout, moving),
                    "{direction:?}, stacked={stacked}"
                );
                assert!(
                    engine.workspaces[target_ws]
                        .layout_system
                        .contains_window(target_layout, moving)
                );
                let gaps = engine.layout_settings.gaps.clone();
                let frames: HashMap<_, _> = engine
                    .calculate_layout(
                        store.view(),
                        target,
                        CGRect::new(CGPoint::new(0.0, 0.0), size),
                        &gaps,
                        Default::default(),
                    )
                    .into_iter()
                    .collect();
                for other in [3, 4, 5].map(|idx| WindowId::new(1, idx)) {
                    let a = frames[&moving];
                    let b = frames[&other];
                    assert!(
                        match direction {
                            Direction::Right => a.max().x <= b.min().x,
                            Direction::Left => a.min().x >= b.max().x,
                            Direction::Down => a.max().y <= b.min().y,
                            Direction::Up => a.min().y >= b.max().y,
                        },
                        "{direction:?}, stacked={stacked}: {a:?} vs {b:?}"
                    );
                }
            }
        }
    }

    /// Each layout pass of a Space is counted for that Space (which passes run a layout
    /// system's shadow, floating's or the scrolling translation's, is decided by its count:
    /// every one in tests). The layouts here are traditional, which has no shadow of its own.
    #[test]
    fn each_spaces_layout_passes_are_counted_for_the_shadow() {
        let mut window_store = OwnedWindows::default();
        let mut engine = test_engine();
        let (a, b) = (SpaceId::new(95), SpaceId::new(96));
        let screen = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(1000.0, 1000.0));
        let size = CGSize::new(500.0, 500.0);
        for (space, pid) in [(a, 61), (b, 62)] {
            let window = WindowId::new(pid, 1);
            window_store.view_mut().insert_manageable_window(window, live_window(size));
            let _ = engine.handle_event(
                &mut window_store.view_mut(),
                LayoutEvent::SpaceExposed(space, screen.size),
            );
            let _ = engine.handle_event(
                &mut window_store.view_mut(),
                LayoutEvent::windows_observed(
                    space,
                    pid,
                    vec![window_layout_info(window, size)],
                    None,
                ),
            );
        }
        let gaps = engine.layout_settings.gaps.clone();
        for space in [a, a, b] {
            let frames = engine.calculate_layout_with_virtual_workspaces(
                window_store.view(),
                space,
                screen,
                &gaps,
                Default::default(),
                |_| None,
                &[screen],
            );
            assert_eq!(frames.len(), 1);
        }
        let mut passes = super::super::core_geometry::ShadowPasses::default();
        passes.pass(a);
        passes.pass(a);
        passes.pass(b);
        assert_eq!(engine.shadow_passes, passes);
    }

    #[test]
    fn observing_one_window_cannot_rewrite_sibling_topology() {
        let mut window_store = OwnedWindows::default();
        let mut engine = test_engine();
        let space = SpaceId::new(94);
        let screen = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(1000.0, 1000.0));
        let pid: pid_t = 5153;
        let w1 = WindowId::new(pid, 1);
        let w2 = WindowId::new(pid, 2);
        let size = CGSize::new(500.0, 500.0);
        for window in [w1, w2] {
            window_store.view_mut().insert_manageable_window(window, live_window(size));
        }
        let info = |wid| window_layout_info(wid, size);

        let _ = engine.handle_event(
            &mut window_store.view_mut(),
            LayoutEvent::SpaceExposed(space, screen.size),
        );
        let _ = engine.handle_event(
            &mut window_store.view_mut(),
            LayoutEvent::windows_observed(space, pid, vec![info(w1), info(w2)], None),
        );
        let _ = engine.handle_event(
            &mut window_store.view_mut(),
            LayoutEvent::WindowFocused(space, w1),
        );
        let _ = engine.handle_command(
            &mut window_store.view_mut(),
            Some(space),
            &[space],
            &HashMap::default(),
            LayoutCommand::ResizeWindowBy {
                amount: rift_protocol::Amount::literal(0.2),
            },
        );

        let gaps = engine.layout_settings.gaps.clone();
        let before =
            engine.calculate_layout(window_store.view(), space, screen, &gaps, Default::default());

        // A partial AX inventory observes w1 but says nothing about w2.
        let _ = engine.handle_event(
            &mut window_store.view_mut(),
            LayoutEvent::windows_observed(space, pid, vec![info(w1)], None),
        );

        assert_eq!(
            engine.calculate_layout(window_store.view(), space, screen, &gaps, Default::default()),
            before,
            "partial discovery must not remove an assigned window or reset its split"
        );
    }

    #[test]
    fn drag_preview_matches_committed_geometry_with_asymmetric_constraints() {
        use crate::layout_engine::{WindowDropAction, WindowDropRequest};

        for mode in [
            LayoutMode::Traditional,
            LayoutMode::Bsp,
            LayoutMode::MasterStack,
            LayoutMode::Scrolling,
            LayoutMode::Stack,
        ] {
            for action in [
                crate::layout_engine::WindowDropAction::Swap,
                WindowDropAction::Insert(Direction::Down),
            ] {
                let settings = LayoutSettings {
                    mode,
                    ..LayoutSettings::default()
                };
                let mut engine = LayoutEngine::new(&VirtualWorkspaceSettings::default(), &settings);
                let mut store = OwnedWindows::default();
                let space = SpaceId::new(96);
                let screen = CGRect::new(CGPoint::ZERO, CGSize::new(1200.0, 800.0));
                let gaps = settings.gaps.effective_for_display(None);
                let line = crate::common::config::StackLineSettings::default();
                let source = WindowId::new(1, 1);
                let target = WindowId::new(1, 2);
                let _ = engine.handle_event(
                    &mut store.view_mut(),
                    LayoutEvent::SpaceExposed(space, screen.size),
                );
                let owner =
                    engine.workspaces.reading(store.view()).active_workspace(space).unwrap();
                for window in [source, target, WindowId::new(1, 3)] {
                    store.view_mut().insert_manageable_window(window, live_window(screen.size));
                    assert!(engine.workspaces.assign_window_to_workspace(
                        &mut store.view_mut(),
                        space,
                        window,
                        owner
                    ));
                    let _ = engine.handle_event(
                        &mut store.view_mut(),
                        LayoutEvent::WindowAdded(space, window),
                    );
                }
                engine.constraints.step(rift_core::constraints::ConstraintsEvent::Observed {
                    window: source,
                    app: source.pid,
                    respect_min_size: false,
                    observed: LayoutEngine::observed_size(
                        true,
                        CGSize::new(0.0, 0.0),
                        None,
                        Some(CGSize::new(180.0, 0.0)),
                    ),
                });
                let before =
                    engine.calculate_layout(store.view(), space, screen, &gaps, line.reserve());
                let preview = engine
                    .drop_preview_frame(
                        store.view().core,
                        space,
                        source,
                        target,
                        action,
                        screen,
                        None,
                        &line,
                    )
                    .unwrap_or_else(|| panic!("missing {mode:?} {action:?} preview"));

                assert_eq!(
                    engine.calculate_layout(store.view(), space, screen, &gaps, line.reserve()),
                    before
                );
                assert!(engine.apply_window_drop(store.view(), WindowDropRequest {
                    source,
                    target,
                    space,
                    action
                }));
                let after =
                    engine.calculate_layout(store.view(), space, screen, &gaps, line.reserve());
                assert_eq!(
                    after.iter().find(|(window, _)| *window == source).unwrap().1,
                    preview.frame,
                    "{mode:?} {action:?}"
                );
            }
        }
    }

    #[test]
    fn a_drop_that_leaves_the_window_in_its_slot_previews_so_and_is_refused() {
        for (action, stays) in [
            (
                crate::layout_engine::WindowDropAction::Insert(Direction::Left),
                true,
            ),
            (
                crate::layout_engine::WindowDropAction::Insert(Direction::Right),
                false,
            ),
        ] {
            let settings = LayoutSettings {
                mode: LayoutMode::Traditional,
                ..LayoutSettings::default()
            };
            let mut engine = LayoutEngine::new(&VirtualWorkspaceSettings::default(), &settings);
            let mut store = OwnedWindows::default();
            let space = SpaceId::new(96);
            let screen = CGRect::new(CGPoint::ZERO, CGSize::new(1200.0, 800.0));
            let line = crate::common::config::StackLineSettings::default();
            // `source` is left of `target`: dropping it on target's left changes nothing.
            let (source, target) = (WindowId::new(1, 1), WindowId::new(1, 2));
            let _ = engine.handle_event(
                &mut store.view_mut(),
                LayoutEvent::SpaceExposed(space, screen.size),
            );
            let owner = engine.workspaces.reading(store.view()).active_workspace(space).unwrap();
            for window in [source, target] {
                store.view_mut().insert_manageable_window(window, live_window(screen.size));
                assert!(engine.workspaces.assign_window_to_workspace(
                    &mut store.view_mut(),
                    space,
                    window,
                    owner
                ));
                let _ = engine
                    .handle_event(&mut store.view_mut(), LayoutEvent::WindowAdded(space, window));
            }
            let preview = engine
                .drop_preview_frame(
                    store.view().core,
                    space,
                    source,
                    target,
                    action,
                    screen,
                    None,
                    &line,
                )
                .expect("a preview");
            assert_eq!(preview.same_slot, stays, "{action:?}");
            let drop = crate::layout_engine::WorkspaceDropRequest {
                window: source,
                workspace: owner,
                target: Some((target, action)),
                frame: None,
            };
            assert_eq!(
                engine.relocate_window_with_drop(&mut store.view_mut(), &drop),
                !stays,
                "{action:?}"
            );
        }
    }

    fn overview_fixture() -> (LayoutEngine, OwnedWindows, crate::actor::reactor::OverviewDrop) {
        let mut engine = test_engine();
        let mut store = OwnedWindows::default();
        let space = SpaceId::new(95);
        let _ = engine.handle_event(
            &mut store.view_mut(),
            LayoutEvent::SpaceExposed(space, CGSize::new(1000.0, 800.0)),
        );
        let window = WindowId::new(6001, 1);
        store
            .view_mut()
            .insert_manageable_window(window, live_window(CGSize::new(500.0, 500.0)));
        let _ = engine.handle_event(
            &mut store.view_mut(),
            LayoutEvent::windows_observed(
                space,
                window.pid,
                vec![window_layout_info(window, CGSize::new(500.0, 500.0))],
                None,
            ),
        );
        let workspaces = engine.workspaces.listed_after_init(&mut store.view_mut(), space);
        let drop = crate::actor::reactor::OverviewDrop {
            window,
            workspace: workspaces[1].0,
            target: None,
            frame: None,
        };
        (engine, store, drop)
    }

    #[test]
    fn overview_drop_moves_membership_once_without_switching_workspace() {
        let (mut engine, mut store, drop) = overview_fixture();
        let active = engine.workspaces().reading(store.view()).active_workspace(SpaceId::new(95));
        assert!(engine.relocate_window_with_drop(&mut store.view_mut(), &drop));
        assert_eq!(
            engine.workspaces().reading(store.view()).active_workspace(SpaceId::new(95)),
            active
        );
        let destination = engine
            .workspaces()
            .reading(store.view())
            .workspace_for_window(SpaceId::new(95), drop.window)
            .unwrap();
        assert_eq!(destination, drop.workspace);
        let layout = engine
            .workspaces
            .reading(store.view())
            .active_layout(SpaceId::new(95), destination)
            .unwrap();
        assert_eq!(
            engine.workspaces[destination].layout_system.selected_window(layout),
            Some(drop.window)
        );
        assert!(
            !engine.relocate_window_with_drop(&mut store.view_mut(), &drop),
            "stale source rejects duplicate mouse-up"
        );
    }

    #[test]
    fn window_added_projects_only_admitted_windows_with_canonical_ownership() {
        let mut engine = test_engine();
        let mut store = OwnedWindows::default();
        let space = SpaceId::new(95);
        let screen = test_display_frame(CGPoint::new(500., 500.));
        let window = WindowId::new(1, 1);
        store.view_mut().insert_manageable_window(window, live_window(screen.size));
        let _ = engine.handle_event(
            &mut store.view_mut(),
            LayoutEvent::SpaceExposed(space, screen.size),
        );

        assert!(
            !engine
                .handle_event(&mut store.view_mut(), LayoutEvent::WindowAdded(space, window))
                .response
                .changed
        );
        assert!(store.view().workspace_for_window(window).is_none());

        let workspace = engine.workspaces.reading(store.view()).active_workspace(space).unwrap();
        assert!(engine.workspaces.assign_window_to_workspace(
            &mut store.view_mut(),
            space,
            window,
            workspace
        ));
        let _ = store.set_manage_override(window, Some(false));
        assert!(
            !engine
                .handle_event(&mut store.view_mut(), LayoutEvent::WindowAdded(space, window))
                .response
                .changed
        );
        let _ = store.set_manage_override(window, None);
        assert!(
            engine
                .handle_event(&mut store.view_mut(), LayoutEvent::WindowAdded(space, window))
                .response
                .changed
        );
    }

    #[test]
    fn move_window_to_workspace_updates_authoritative_workspace_membership() {
        let mut window_store = OwnedWindows::default();
        let mut engine = test_engine();
        let space = SpaceId::new(95);
        let screen = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(1000.0, 1000.0));
        let pid: pid_t = 6001;
        let wid = WindowId::new(pid, 1);

        let window = crate::model::reactor::WindowState::from(
            crate::actor::reactor::testing::make_window_info(screen, None, "Window", None),
        );
        window_store.view_mut().insert_manageable_window(wid, window);

        let _ = engine.handle_event(
            &mut window_store.view_mut(),
            LayoutEvent::SpaceExposed(space, screen.size),
        );
        let _ = engine.handle_event(
            &mut window_store.view_mut(),
            LayoutEvent::windows_observed(
                space,
                pid,
                vec![window_layout_info(wid, CGSize::new(500.0, 500.0))],
                None,
            ),
        );

        let _ = engine.handle_virtual_workspace_command(
            &mut window_store.view_mut(),
            space,
            &LayoutCommand::CreateWorkspace,
        );
        let workspaces = engine
            .workspaces_mut()
            .listed_after_init(&mut window_store.view_mut(), space)
            .to_vec();
        let ws1 = workspaces[0].0;
        let ws2 = workspaces[1].0;

        let layout = engine
            .workspaces
            .reading(window_store.view())
            .active_layout(space, ws1)
            .unwrap();
        let before = engine.workspaces[ws1].layout_system.container_tree(layout);
        assert!(!engine.relocate_window_to_workspace(
            &mut window_store.view_mut(),
            ws1,
            (SpaceId::new(96), ws2),
            wid,
            WindowRelocation::Workspace,
        ));
        assert_eq!(window_store.view().workspace_for_window(wid), Some(ws1));
        assert_eq!(
            engine.workspaces[ws1].layout_system.container_tree(layout),
            before
        );

        let _ = engine.handle_virtual_workspace_command(
            &mut window_store.view_mut(),
            space,
            &LayoutCommand::MoveWindowToWorkspace {
                workspace: WorkspaceSelector::Index(1),
                follow: false,
                window_id: Some(wid.idx.get()),
            },
        );

        assert!(
            window_store.view().workspace_windows(ws1).is_empty(),
            "source workspace must be empty after a same-space workspace move"
        );
        assert_eq!(
            engine
                .workspaces()
                .reading(window_store.view())
                .workspace_for_window(space, wid),
            Some(ws2)
        );
        assert_eq!(window_store.view().workspace_windows(ws2), vec![wid]);

        for (target, expected) in [("next", workspaces[2].0), ("prev", ws2)] {
            let _ = engine.handle_virtual_workspace_command(
                &mut window_store.view_mut(),
                space,
                &LayoutCommand::MoveWindowToWorkspace {
                    workspace: WorkspaceSelector::Name(target.into()),
                    follow: false,
                    window_id: Some(wid.idx.get()),
                },
            );
            assert_eq!(
                engine
                    .workspaces()
                    .reading(window_store.view())
                    .workspace_for_window(space, wid),
                Some(expected)
            );
        }
        window_store.invalidate_ax_identity(wid);
        let _ =
            engine.handle_event(&mut window_store.view_mut(), LayoutEvent::WindowSuspended(wid));
        assert!(engine.relocate_window_to_workspace(
            &mut window_store.view_mut(),
            ws2,
            (space, ws1),
            wid,
            WindowRelocation::Observed
        ));
        assert!(window_store.is_temporarily_unavailable(wid));
        assert!(!engine.workspaces[ws1].layout_system.window_participates(wid));
        window_store.validate_ax_identity(wid);
        let _ = engine.handle_event(
            &mut window_store.view_mut(),
            LayoutEvent::WindowAdded(space, wid),
        );
        assert_eq!(window_store.view().workspace_for_window(wid), Some(ws1));
        for (id, workspace) in engine.workspaces.workspaces.iter() {
            let layout = workspace.active_layout().unwrap();
            assert_eq!(workspace.layout_system.contains_window(layout, wid), id == ws1);
        }
    }

    /// A fixed-size window (`fixed`, 300 wide) tiled beside a resizable one on a 1200x800 screen.
    fn fixed_window_beside_another() -> (LayoutEngine, OwnedWindows, SpaceId, CGRect, WindowId) {
        let mut engine = test_engine();
        let mut window_store = OwnedWindows::default();
        let space = SpaceId::new(93);
        let screen = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(1200.0, 800.0));
        let _ = engine.handle_event(
            &mut window_store.view_mut(),
            LayoutEvent::SpaceExposed(space, screen.size),
        );
        let fixed = WindowId::new(7, 1);
        let frame = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(300.0, 800.0));
        let info = crate::actor::app::WindowInfo {
            has_native_tabs: false,
            is_standard: true,
            is_root: true,
            is_minimized: false,
            is_resizable: false,
            min_size: None,
            max_size: None,
            title: String::new(),
            frame,
            sys_id: None,
            bundle_id: None,
            path: None,
            ax_role: None,
            ax_subrole: None,
        };
        window_store.view_mut().insert_manageable_window(fixed, info.into());
        // The engine projects only windows the store admits (#702).
        window_store
            .view_mut()
            .insert_manageable_window(WindowId::new(8, 1), live_window(CGSize::new(500.0, 500.0)));
        for (window, info) in [
            (fixed, WindowLayoutInfo {
                is_resizable: false,
                ..window_layout_info(fixed, CGSize::new(300.0, 800.0))
            }),
            (
                WindowId::new(8, 1),
                window_layout_info(WindowId::new(8, 1), CGSize::new(500.0, 500.0)),
            ),
        ] {
            let _ = engine.handle_event(
                &mut window_store.view_mut(),
                LayoutEvent::windows_observed(space, window.pid, vec![info], None),
            );
        }
        (engine, window_store, space, screen, fixed)
    }

    /// The fixed window's tile width.
    fn fixed_width(
        engine: &mut LayoutEngine,
        windows: Windows<'_>,
        space: SpaceId,
        screen: CGRect,
        fixed: WindowId,
    ) -> f64 {
        engine
            .calculate_layout(windows, space, screen, &Default::default(), Default::default())
            .into_iter()
            .find(|(w, _)| *w == fixed)
            .expect("fixed window is tiled")
            .1
            .size
            .width
    }

    /// One observed window on a 1200x800 Space, made floating, with a frame in its workspace.
    fn floating_window() -> (LayoutEngine, OwnedWindows, SpaceId, VirtualWorkspaceId, WindowId) {
        let mut engine = test_engine();
        let mut store = OwnedWindows::default();
        let space = SpaceId::new(97);
        let _ = engine.handle_event(
            &mut store.view_mut(),
            LayoutEvent::SpaceExposed(space, CGSize::new(1200.0, 800.0)),
        );
        let window = WindowId::new(9, 1);
        store
            .view_mut()
            .insert_manageable_window(window, live_window(CGSize::new(400.0, 300.0)));
        let _ = engine.handle_event(
            &mut store.view_mut(),
            LayoutEvent::windows_observed(
                space,
                window.pid,
                vec![window_layout_info(window, CGSize::new(400.0, 300.0))],
                None,
            ),
        );
        let workspace = engine.workspaces.reading(store.view()).active_workspace(space).unwrap();
        // A floating window leaves the tiling layout, as the toggle does.
        engine.remove_window_from_all_tiling_trees(window);
        engine.floats.add_floating(window);
        engine.floats.store(
            workspace,
            window,
            CGRect::new(CGPoint::ZERO, CGSize::new(400.0, 300.0)),
        );
        (engine, store, space, workspace, window)
    }

    /// A floating window is active on its Space while its workspace is the Space's active one,
    /// and only then (computed from the topology, never kept).
    #[test]
    fn a_floating_window_is_active_only_while_its_workspace_is() {
        let (mut engine, mut store, space, workspace, window) = floating_window();
        assert!(
            engine
                .active_floating_windows_in_workspace(store.view(), space)
                .contains(&window)
        );
        let other = engine
            .workspaces
            .reading(store.view())
            .workspaces_on(space)
            .into_iter()
            .find(|&id| id != workspace)
            .expect("a second workspace on the Space");
        let _ = engine.activate_workspace(&mut store.view_mut(), space, other, None);
        assert!(
            !engine
                .active_floating_windows_in_workspace(store.view(), space)
                .contains(&window)
        );
        let _ = engine.activate_workspace(&mut store.view_mut(), space, workspace, None);
        assert!(
            engine
                .active_floating_windows_in_workspace(store.view(), space)
                .contains(&window)
        );
        // A tiled window is no active floating window.
        engine.floats.remove_floating(window);
        assert!(
            !engine
                .active_floating_windows_in_workspace(store.view(), space)
                .contains(&window)
        );
    }

    fn shown_space(engine: &mut LayoutEngine, store: &mut OwnedWindows, space: SpaceId) {
        let _ = engine.handle_event(
            &mut store.view_mut(),
            LayoutEvent::SpaceExposed(space, CGSize::new(1200.0, 800.0)),
        );
    }

    /// A Space shown on no display loses its display; the display keeps it as its last Space.
    #[test]
    fn a_hidden_space_loses_its_display_and_stays_the_displays_last() {
        let (mut engine, mut store) = (test_engine(), OwnedWindows::default());
        let space = SpaceId::new(97);
        shown_space(&mut engine, &mut store, space);
        engine.update_space_display(store.view(), space, Some("a"));
        assert_eq!(engine.display_uuid_for_space(space), Some("a"));
        engine.update_space_display(store.view(), space, None);
        assert_eq!(engine.display_uuid_for_space(space), None);
        assert_eq!(engine.last_space_for_display_uuid("a"), Some(space));
    }

    /// A remapped Space takes its display, and the display's memory of it, to its successor.
    #[test]
    fn a_remapped_space_takes_its_display_along() {
        let (mut engine, mut store) = (test_engine(), OwnedWindows::default());
        let (old, new) = (SpaceId::new(97), SpaceId::new(98));
        shown_space(&mut engine, &mut store, old);
        engine.update_space_display(store.view(), old, Some("a"));
        engine.remap_space(&mut store.view_mut(), old, new);
        assert_eq!(engine.display_uuid_for_space(new), Some("a"));
        assert_eq!(engine.display_uuid_for_space(old), None);
        assert_eq!(engine.last_space_for_display_uuid("a"), Some(new));
    }

    /// A workspace homed with no display on a Space whose display is known (as a file can hold
    /// one) takes the display and its column widths when the display is next reported, though the
    /// Space's display did not change.
    #[test]
    fn a_workspace_without_a_display_takes_the_spaces_and_its_column_widths() {
        use crate::common::config::ScrollingWidthOverride;
        use crate::model::virtual_workspace::{Home, PlacedBy};
        let mut layout = LayoutSettings {
            mode: LayoutMode::Scrolling,
            ..LayoutSettings::default()
        };
        layout.scrolling.per_display.insert("a".into(), ScrollingWidthOverride {
            column_width_ratio: Some(0.45),
            min_column_width_ratio: None,
            max_column_width_ratio: None,
        });
        let mut engine = LayoutEngine::new(&VirtualWorkspaceSettings::default(), &layout);
        let mut store = OwnedWindows::default();
        let space = SpaceId::new(97);
        shown_space(&mut engine, &mut store, space);
        engine.update_space_display(store.view(), space, Some("a"));
        let unplaced = engine.workspaces.reading(store.view()).active_workspace(space).unwrap();
        let workspace = &mut engine.workspaces[unplaced];
        workspace.home = Home::placed(space, None, PlacedBy::Automatic);
        workspace.apply_layout_settings(&layout, None);
        let widths = |engine: &LayoutEngine| match &engine.workspaces[unplaced].layout_system {
            LayoutSystemKind::Scrolling(system) => system.configured_widths(),
            _ => panic!("a scrolling workspace"),
        };
        assert_eq!(widths(&engine), layout.scrolling.widths_for_display(None));
        assert_ne!(widths(&engine), layout.scrolling.widths_for_display(Some("a")));
        engine.update_space_display(store.view(), space, Some("a"));
        assert_eq!(engine.workspaces[unplaced].home.display(), Some("a"));
        assert_eq!(widths(&engine), layout.scrolling.widths_for_display(Some("a")));
    }

    /// A workspace on Space 97 with three tiled windows, and its arrangement kept as for a
    /// display's departure.
    fn departed_workspace() -> (LayoutEngine, OwnedWindows, VirtualWorkspaceId, [WindowId; 3]) {
        let (mut engine, mut store) = (test_engine(), OwnedWindows::default());
        let space = SpaceId::new(97);
        shown_space(&mut engine, &mut store, space);
        let windows = [
            WindowId::new(9, 1),
            WindowId::new(9, 2),
            WindowId::new(9, 3),
        ];
        for window in windows {
            store
                .view_mut()
                .insert_manageable_window(window, live_window(CGSize::new(400.0, 300.0)));
        }
        let infos = windows
            .iter()
            .map(|&w| window_layout_info(w, CGSize::new(400.0, 300.0)))
            .collect();
        let _ = engine.handle_event(
            &mut store.view_mut(),
            LayoutEvent::windows_observed(space, windows[0].pid, infos, None),
        );
        let workspace = engine.workspaces.reading(store.view()).active_workspace(space).unwrap();
        engine.remember_native_departure(store.view(), windows[0]);
        (engine, store, workspace, windows)
    }

    /// A workspace's focus never goes to a window that is unavailable (minimized), even asked for.
    #[test]
    fn a_workspace_focus_skips_an_unavailable_window() {
        let (mut engine, mut store, workspace, [a, b, c]) = departed_workspace();
        let space = SpaceId::new(97);
        store.view_mut().set_minimized(a, true);
        let response = engine.activate_workspace(&mut store.view_mut(), space, workspace, Some(a));
        let focused = engine.preferred_focus_for_workspace(store.view(), space, workspace, Some(a));
        assert_ne!(focused, Some(a));
        assert!(focused == Some(b) || focused == Some(c), "{focused:?}");
        let _ = response;
    }

    /// Every kind of focus question through the engine's one door, each answer in its own
    /// kind; under `roc-core` the Roc core answers too (a difference, or an answer of another
    /// kind, fails here).
    #[test]
    fn each_focus_question_answers_in_its_own_kind() {
        use rift_core::focus::{CycleAsk, EdgeAsk, EntryAsk, FallbackAsk, RecordAsk, SpaceAsk};
        use rift_core::layout::bsp_tree::Direction;
        use rift_core::plain::Rect;
        let w = |n| WindowId::new(1, n);
        let rect = |x| Rect { x, y: 0.0, w: 100.0, h: 100.0 };
        for direction in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ] {
            for scrolling in [false, true] {
                let horizontal = matches!(direction, Direction::Left | Direction::Right);
                let expected = if scrolling && horizontal {
                    PastTheEdge::Stop
                } else {
                    PastTheEdge::NextSpace
                };
                assert_eq!(LayoutEngine::focus(EdgeAsk { direction, scrolling }), expected);
            }
        }
        let cycle = CycleAsk {
            floating: vec![w(1), w(2), w(3)],
            focused: Some(w(1)),
            direction: Direction::Left,
        };
        assert_eq!(LayoutEngine::focus(cycle), Some(w(3)));
        let entry = EntryAsk {
            directional: None,
            visible: vec![w(2), w(3)],
        };
        assert_eq!(LayoutEngine::focus(entry), Some(w(2)));
        let fallback = FallbackAsk {
            first_floating: None,
            previous: Some(w(4)),
            first_visible: Some(w(5)),
        };
        assert_eq!(LayoutEngine::focus(fallback), Some(w(4)));
        let committed = Committed {
            window: w(1),
            belongs: true,
            floating: true,
        };
        let record = RecordAsk {
            workspace: true,
            focus: Some(committed),
        };
        assert_eq!(LayoutEngine::focus(record), FocusRecord::Floating(w(1)));
        let space = SpaceId::new(7);
        let ask = SpaceAsk {
            current: rect(0.0),
            direction: Direction::Right,
            candidates: vec![(space, rect(100.0))],
        };
        assert_eq!(LayoutEngine::focus(ask), Some(space));
        let workspace = WorkspaceFocus {
            preferred: Some(Eligible { window: w(6), belongs: false }),
            last_focused: Some(Eligible { window: w(7), belongs: true }),
            selected: None,
            visible: vec![],
            floating: vec![],
            floating_last_focus: None,
        };
        assert_eq!(LayoutEngine::focus(workspace), Some(w(7)));
    }

    /// A focus committed to a window of another workspace is not kept: the focus is cleared.
    #[test]
    fn a_committed_focus_of_another_workspace_is_cleared() {
        let (mut engine, mut store, workspace, [a, _, c]) = departed_workspace();
        let space = SpaceId::new(97);
        engine.commit_workspace_focus(&mut store.view_mut(), space, Some(a));
        assert_eq!(engine.floats.focused(), Some(a));
        let other = engine
            .workspaces
            .reading(store.view())
            .workspaces_on(space)
            .find(|&id| id != workspace)
            .expect("a second workspace on the Space");
        assert!(engine.workspaces.assign_window_to_workspace(
            &mut store.view_mut(),
            space,
            c,
            other
        ));
        engine.commit_workspace_focus(&mut store.view_mut(), space, Some(c));
        assert_eq!(engine.floats.focused(), None);
    }

    /// A native snapshot's windows, in window order.
    fn kept(engine: &LayoutEngine, workspace: VirtualWorkspaceId) -> Vec<WindowId> {
        let mut windows =
            rift_core::native_memory::Held::windows(engine.native.get(workspace).unwrap());
        windows.sort();
        windows
    }

    /// A native snapshot keeps the workspace's layout and its windows; a window that closes
    /// leaves it; once the rest are back, a restore spends it.
    #[test]
    fn a_native_snapshot_forgets_a_closed_window_and_is_spent_once_the_rest_return() {
        let (mut engine, mut store, workspace, [a, b, c]) = departed_workspace();
        let snapshot = engine.native.get(workspace).unwrap();
        assert_eq!(
            Some(snapshot.layout),
            engine.workspaces[workspace].layout_state.active()
        );
        assert_eq!(kept(&engine, workspace), [a, b, c]);
        let _ = engine.handle_event(&mut store.view_mut(), LayoutEvent::WindowRemoved(c));
        assert_eq!(kept(&engine, workspace), [a, b]);
        engine.restore_native_layout(store.view(), workspace, None);
        assert!(engine.native.is_empty(), "every window it kept is back");
    }

    /// A window still in the workspace's layout but assigned to another workspace is not taken
    /// back: the restore leaves it out, and the snapshot is not spent.
    #[test]
    fn a_restore_takes_back_only_the_workspaces_own_windows() {
        let (mut engine, mut store, workspace, [_, _, c]) = departed_workspace();
        let space = SpaceId::new(97);
        let other = engine
            .workspaces
            .reading(store.view())
            .workspaces_on(space)
            .find(|&id| id != workspace)
            .expect("a second workspace on the Space");
        assert!(engine.workspaces.assign_window_to_workspace(
            &mut store.view_mut(),
            space,
            c,
            other
        ));
        // Still in the workspace's layout, as a window is between its reassignment and the
        // layout pass that moves it.
        let layout = engine.workspaces[workspace].layout_state.active().unwrap();
        engine.workspaces[workspace]
            .layout_system
            .update(WindowChange::Add(layout, c, None));
        engine.restore_native_layout(store.view(), workspace, None);
        assert!(engine.native.get(workspace).is_some(), "window c is not back");
    }

    /// A snapshot follows a window's new identity, and is dropped once its last window is gone.
    #[test]
    fn a_native_snapshot_follows_an_identity_and_goes_with_its_last_window() {
        let (mut engine, mut store, workspace, [a, b, c]) = departed_workspace();
        let rekeyed = WindowId::new(9, 7);
        engine.transfer_persistent_window_identity(a, rekeyed);
        assert_eq!(kept(&engine, workspace), [b, c, rekeyed]);
        for window in [rekeyed, b, c] {
            let _ = engine.handle_event(&mut store.view_mut(), LayoutEvent::WindowRemoved(window));
        }
        assert!(engine.native.is_empty());
    }

    /// A window evicted from Space 97 (display `a`) into Space 98's active workspace (display
    /// `landed_on`), recorded; with the engine, store, both Spaces and the workspace it left.
    /// A fresh Space shows the configured default workspace, also when asking for its active
    /// workspace is what initialises it.
    #[test]
    fn a_fresh_space_shows_the_default_workspace() {
        let settings = VirtualWorkspaceSettings {
            default_workspace_count: 4,
            default_workspace: 2,
            ..VirtualWorkspaceSettings::default()
        };
        let mut engine = LayoutEngine::new(&settings, &LayoutSettings::default());
        let mut store = OwnedWindows::default();
        let space = SpaceId::new(5);
        let shown = engine.ensure_active_workspace_info(&mut store.view_mut(), space);
        let third = engine.workspaces.reading(store.view()).workspace_id_at(space, Some(2));
        assert!(third.is_some());
        assert_eq!(shown.map(|(id, _)| id), third);
        assert_eq!(
            engine.workspaces.reading(store.view()).active_workspace(space),
            third
        );
    }

    /// In Global mode, a second display's Space owns no workspace: it stays vacant (it shows
    /// none of the first display's).
    #[test]
    fn a_global_space_that_owns_no_workspace_stays_vacant() {
        let settings = VirtualWorkspaceSettings {
            mode: crate::common::config::WorkspaceMode::Global,
            ..VirtualWorkspaceSettings::default()
        };
        let mut engine = LayoutEngine::new(&settings, &LayoutSettings::default());
        let mut store = OwnedWindows::default();
        let (a, b) = (SpaceId::new(97), SpaceId::new(98));
        shown_space(&mut engine, &mut store, a);
        let catalog = store.view().core.catalog().to_vec();
        engine.workspaces.update_display_count(&catalog, 2);
        let on_a: Vec<_> = engine.workspaces.reading(store.view()).workspaces_on(a).collect();
        assert!(!on_a.is_empty());
        assert_eq!(
            engine.ensure_active_workspace_info(&mut store.view_mut(), b),
            None
        );
        assert_eq!(engine.workspaces.reading(store.view()).active_workspace(b), None);
        let still_on_a: Vec<_> = engine.workspaces.reading(store.view()).workspaces_on(a).collect();
        assert_eq!(still_on_a, on_a);
    }

    /// Two Spaces on displays "a" and "b", with `count` workspaces each.
    fn two_displays(count: usize) -> (LayoutEngine, OwnedWindows, SpaceId, SpaceId) {
        let settings = VirtualWorkspaceSettings {
            default_workspace_count: count,
            ..VirtualWorkspaceSettings::default()
        };
        let mut engine = LayoutEngine::new(&settings, &LayoutSettings::default());
        let mut store = OwnedWindows::default();
        let (a, b) = (SpaceId::new(97), SpaceId::new(98));
        for (space, display) in [(a, "a"), (b, "b")] {
            shown_space(&mut engine, &mut store, space);
            engine.update_space_display(store.view(), space, Some(display));
        }
        (engine, store, a, b)
    }

    /// An explicit move of a Space's last workspace leaves the Space a new one, laid out at the
    /// Space's size and homed on its display; a display departure leaves the Space empty.
    #[test]
    fn only_an_explicit_transfer_lays_out_and_rehomes_the_space_it_leaves() {
        use crate::model::virtual_workspace::WorkspaceTransferReason as Reason;
        let size = CGSize::new(1200.0, 800.0);
        for (reason, left) in [
            (Reason::Explicit, vec![(Some("a".to_owned()), Some(size))]),
            (Reason::DisplayDeparture, vec![]),
        ] {
            let (mut engine, mut store, a, b) = two_displays(1);
            let moved = engine.workspaces.reading(store.view()).active_workspace(a).unwrap();
            let target_size = CGSize::new(1000.0, 700.0);
            let _ = engine.transfer_workspace(
                &mut store.view_mut(),
                moved,
                b,
                target_size,
                Some("b"),
                reason,
            );
            let on_a: Vec<_> = engine.workspaces.reading(store.view()).workspaces_on(a).collect();
            let homes: Vec<_> = (on_a.iter())
                .map(|&id| {
                    let workspace = &engine.workspaces[id];
                    (
                        workspace.home.display().map(str::to_owned),
                        workspace.layout_state.active_size(),
                    )
                })
                .collect();
            assert_eq!(homes, left, "{reason:?}");
        }
    }

    /// Going to a workspace of another Space shows it there (a change); going to one that Space
    /// shows already only moves focus there.
    #[test]
    fn a_workspace_shown_on_another_space_only_takes_focus_there() {
        let (mut engine, mut store, a, b) = two_displays(2);
        let on_b: Vec<_> = engine.workspaces.reading(store.view()).workspaces_on(b).collect();
        let shown = engine.workspaces.reading(store.view()).active_workspace(b).unwrap();
        let hidden = *on_b.iter().find(|&&id| id != shown).unwrap();
        let focus_only = engine.navigate_to_workspace(&mut store.view_mut(), a, shown, None);
        assert_eq!(focus_only.workspace_space, Some(b));
        assert!(!focus_only.changed, "a shown workspace only takes focus");
        let shows = engine.navigate_to_workspace(&mut store.view_mut(), a, hidden, None);
        assert_eq!(
            engine.workspaces.reading(store.view()).active_workspace(b),
            Some(hidden)
        );
        assert!(shows.changed, "a hidden workspace is shown");
    }

    fn evicted_window(
        landed_on: &str,
    ) -> (
        LayoutEngine,
        OwnedWindows,
        SpaceId,
        SpaceId,
        VirtualWorkspaceId,
        WindowId,
    ) {
        let (mut engine, mut store) = (test_engine(), OwnedWindows::default());
        let (from_space, to_space) = (SpaceId::new(97), SpaceId::new(98));
        for (space, display) in [(from_space, "a"), (to_space, landed_on)] {
            shown_space(&mut engine, &mut store, space);
            engine.update_space_display(store.view(), space, Some(display));
        }
        let window = WindowId::new(9, 1);
        store
            .view_mut()
            .insert_manageable_window(window, live_window(CGSize::new(400.0, 300.0)));
        let from = engine.workspaces.reading(store.view()).active_workspace(from_space).unwrap();
        let to = engine.workspaces.reading(store.view()).active_workspace(to_space).unwrap();
        assert!(engine.workspaces.assign_window_to_workspace(
            &mut store.view_mut(),
            from_space,
            window,
            from
        ));
        assert!(engine.workspaces.assign_window_to_workspace(
            &mut store.view_mut(),
            to_space,
            window,
            to
        ));
        engine.note_eviction(store.view(), window, from);
        (engine, store, from_space, to_space, from, window)
    }

    /// A window a departure moved to another display is recorded; one that stayed on its display
    /// is not, and returning consumes the record.
    #[test]
    fn an_eviction_is_recorded_only_off_its_display_and_consumed_on_return() {
        let (mut engine, store, from_space, ..) = evicted_window("b");
        let window = WindowId::new(9, 1);
        assert_eq!(
            engine.eviction_ordinal(store.view(), window, from_space),
            Some(0)
        );
        assert_eq!(engine.take_eviction(window), Some(0));
        assert_eq!(engine.take_eviction(window), None);
        let (engine, store, from_space, ..) = evicted_window("a");
        assert_eq!(engine.eviction_ordinal(store.view(), window, from_space), None);
        assert!(engine.evicted.get(window).is_none());
    }

    /// An eviction record goes with its window, its app or its workspace's transfer, and follows
    /// its window's new identity.
    #[test]
    fn an_eviction_record_goes_with_its_window_app_or_transfer_and_follows_its_identity() {
        let window = WindowId::new(9, 1);
        let (mut engine, mut store, ..) = evicted_window("b");
        let _ = engine.handle_event(&mut store.view_mut(), LayoutEvent::WindowRemoved(window));
        assert!(engine.evicted.get(window).is_none(), "a removed window");
        let (mut engine, mut store, ..) = evicted_window("b");
        let _ = engine.handle_event(&mut store.view_mut(), LayoutEvent::AppClosed(window.pid));
        assert!(engine.evicted.get(window).is_none(), "a quit app");
        let (mut engine, mut store, from_space, to_space, ..) = evicted_window("b");
        let landed = engine.workspaces.reading(store.view()).active_workspace(to_space).unwrap();
        let _ = engine.transfer_workspace(
            &mut store.view_mut(),
            landed,
            from_space,
            CGSize::new(1200.0, 800.0),
            Some("a"),
            crate::model::virtual_workspace::WorkspaceTransferReason::Explicit,
        );
        assert!(engine.evicted.get(window).is_none(), "a transferred workspace");
        let (mut engine, ..) = evicted_window("b");
        let rekeyed = WindowId::new(9, 2);
        engine.transfer_persistent_window_identity(window, rekeyed);
        assert!(engine.evicted.get(window).is_none());
        assert_eq!(engine.evicted.get(rekeyed).map(|record| record.ordinal), Some(0));
    }

    /// A floating window projects onto its Space only while its workspace is the active one and
    /// it is available (the active floating windows, computed).
    #[test]
    fn a_floating_window_projects_only_from_the_active_workspace_and_while_available() {
        let (mut engine, mut store, space, workspace, window) = floating_window();
        assert_eq!(
            engine.projected_space_in(store.view(), workspace, window),
            Some(space)
        );
        let other = engine
            .workspaces
            .reading(store.view())
            .workspaces_on(space)
            .into_iter()
            .find(|&id| id != workspace)
            .expect("a second workspace on the Space");
        let _ = engine.activate_workspace(&mut store.view_mut(), space, other, None);
        assert_eq!(engine.projected_space_in(store.view(), workspace, window), None);
        let _ = engine.activate_workspace(&mut store.view_mut(), space, workspace, None);
        assert_eq!(
            engine.projected_space_in(store.view(), workspace, window),
            Some(space)
        );
        store.view_mut().set_minimized(window, true);
        assert_eq!(engine.projected_space_in(store.view(), workspace, window), None);
    }

    /// A remapped Space keeps its active floating windows: they follow its workspaces.
    #[test]
    fn a_remapped_space_keeps_its_active_floating_windows() {
        let (mut engine, mut store, space, _, window) = floating_window();
        let new = SpaceId::new(98);
        engine.remap_space(&mut store.view_mut(), space, new);
        assert!(engine.active_floating_windows_in_workspace(store.view(), new).contains(&window));
        assert!(
            !engine
                .active_floating_windows_in_workspace(store.view(), space)
                .contains(&window)
        );
    }

    /// A workspace forgotten takes its floating frames.
    #[test]
    fn a_forgotten_workspace_takes_its_floating_frames() {
        let (mut engine, _store, _, workspace, window) = floating_window();
        assert!(engine.floats.get(workspace, window).is_some());
        engine.forget_workspace(workspace);
        assert_eq!(engine.floats.get(workspace, window), None);
    }

    /// An app closing takes its windows' floating state and frames.
    #[test]
    fn a_closing_app_takes_its_windows_floating_state_and_frames() {
        let (mut engine, mut store, _, workspace, window) = floating_window();
        let _ = engine.handle_event(&mut store.view_mut(), LayoutEvent::AppClosed(window.pid));
        assert!(!engine.floats.is_floating(window));
        assert_eq!(engine.floats.get(workspace, window), None);
    }

    /// A window's constraints follow it to a new identity.
    #[test]
    fn a_new_identity_keeps_the_windows_constraints() {
        let (mut engine, _window_store, _, _, fixed) = fixed_window_beside_another();
        let record = engine.constraints.get(fixed).expect("observed");
        let new = WindowId::new(7, 2);
        engine.transfer_persistent_window_identity(fixed, new);
        assert_eq!(engine.constraints.get(fixed), None);
        assert_eq!(engine.constraints.get(new), Some(record));
    }

    /// A native resize locks a window at its new width: a fixed window's tile is that wide.
    #[test]
    fn a_native_resize_locks_the_window_at_its_width() {
        let (mut engine, mut window_store, space, screen, fixed) = fixed_window_beside_another();
        assert_eq!(
            fixed_width(&mut engine, window_store.view(), space, screen, fixed),
            300.0
        );
        let old_frame = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(300.0, 800.0));
        let new_frame = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(350.0, 800.0));
        let _ = engine.handle_event(&mut window_store.view_mut(), LayoutEvent::WindowResized {
            wid: fixed,
            old_frame,
            new_frame,
            screens: vec![(space, screen, None)].into(),
        });
        assert_eq!(
            fixed_width(&mut engine, window_store.view(), space, screen, fixed),
            350.0
        );
    }

    /// Switched to a scrolling layout that preserves window sizes, each window keeps its width.
    #[test]
    fn a_switch_to_scrolling_preserving_sizes_keeps_each_windows_width() {
        let (mut engine, mut window_store, space, screen, fixed) = fixed_window_beside_another();
        // The resizable window is resized after its observation: the switch preserves 450.
        let other = WindowId::new(8, 1);
        window_store
            .view_mut()
            .window_mut(other)
            .unwrap()
            .project_seen(CGRect::new(CGPoint::ZERO, CGSize::new(450.0, 500.0)));
        let settings = LayoutSettings {
            scrolling: crate::common::config::ScrollingLayoutSettings {
                preserve_window_sizes: true,
                ..Default::default()
            },
            ..LayoutSettings::default()
        };
        engine.set_layout_settings(window_store.view(), &settings);
        let workspace =
            engine.workspaces.reading(window_store.view()).active_workspace(space).unwrap();
        engine.switch_workspace_layout_mode(
            window_store.view(),
            space,
            workspace,
            LayoutMode::Scrolling,
        );
        let widths: HashMap<WindowId, f64> = engine
            .calculate_layout(
                window_store.view(),
                space,
                screen,
                &Default::default(),
                Default::default(),
            )
            .into_iter()
            .map(|(w, frame)| (w, frame.size.width))
            .collect();
        assert_eq!(widths.get(&fixed), Some(&300.0), "{widths:?}");
        assert_eq!(widths.get(&other), Some(&450.0), "{widths:?}");
    }

    #[test]
    fn moving_a_window_between_spaces_keeps_its_size_constraints() {
        let mut engine = test_engine();
        let mut window_store = OwnedWindows::default();
        let space = SpaceId::new(93);
        let screen = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(1200.0, 800.0));
        let _ = engine.handle_event(
            &mut window_store.view_mut(),
            LayoutEvent::SpaceExposed(space, screen.size),
        );
        let fixed = WindowId::new(7, 1);
        let frame = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(300.0, 800.0));
        let info = crate::actor::app::WindowInfo {
            has_native_tabs: false,
            is_standard: true,
            is_root: true,
            is_minimized: false,
            is_resizable: false,
            min_size: None,
            max_size: None,
            title: String::new(),
            frame,
            sys_id: None,
            bundle_id: None,
            path: None,
            ax_role: None,
            ax_subrole: None,
        };
        window_store.view_mut().insert_manageable_window(fixed, info.into());
        // The engine projects only windows the store admits (#702).
        window_store
            .view_mut()
            .insert_manageable_window(WindowId::new(8, 1), live_window(CGSize::new(500.0, 500.0)));
        for (window, info) in [
            (fixed, WindowLayoutInfo {
                is_resizable: false,
                ..window_layout_info(fixed, CGSize::new(300.0, 800.0))
            }),
            (
                WindowId::new(8, 1),
                window_layout_info(WindowId::new(8, 1), CGSize::new(500.0, 500.0)),
            ),
        ] {
            let _ = engine.handle_event(
                &mut window_store.view_mut(),
                LayoutEvent::windows_observed(space, window.pid, vec![info], None),
            );
        }
        let width = |engine: &mut LayoutEngine, windows: Windows<'_>| {
            let frames = engine.calculate_layout(
                windows,
                space,
                screen,
                &Default::default(),
                Default::default(),
            );
            frames
                .into_iter()
                .find(|(w, _)| *w == fixed)
                .expect("fixed window is tiled")
                .1
                .size
                .width
        };
        assert_eq!(width(&mut engine, window_store.view()), 300.0);

        for removal in [
            LayoutEvent::WindowRemovedPreserveFloating(fixed),
            LayoutEvent::WindowRemoved(fixed),
        ] {
            let _ = engine.handle_event(&mut window_store.view_mut(), removal);
            // A move assigns the window before it is added: the layout takes only windows the
            // store owns (#702).
            if window_store.view().workspace_for_window(fixed).is_none() {
                let owner =
                    engine.workspaces.reading(window_store.view()).active_workspace(space).unwrap();
                assert!(engine.workspaces.assign_window_to_workspace(
                    &mut window_store.view_mut(),
                    space,
                    fixed,
                    owner
                ));
            }
            let _ = engine.handle_event(
                &mut window_store.view_mut(),
                LayoutEvent::WindowAdded(space, fixed),
            );
            assert_eq!(width(&mut engine, window_store.view()), 300.0);
        }

        // Leaving the layout for good still isn't the window's end; its retirement is (#707).
        let _ =
            engine.handle_event(&mut window_store.view_mut(), LayoutEvent::WindowRemoved(fixed));
        assert!(engine.has_window_constraints(fixed));
        window_store.view_mut().remove_window(fixed);
        engine.retire_window(fixed);
        assert!(!engine.has_window_constraints(fixed));
    }

    /// A master_stack engine whose new windows join the stack, with a rule for
    /// `com.example.Constrained`, on an 1200x800 screen.
    fn master_stack_with_rule(
        respect_min_size: bool,
    ) -> (LayoutEngine, OwnedWindows, SpaceId, CGRect) {
        let settings = VirtualWorkspaceSettings {
            app_rules: vec![crate::common::config::AppWorkspaceRule {
                app_id: Some("com.example.Constrained".into()),
                respect_min_size,
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut layout_settings = LayoutSettings {
            mode: LayoutMode::MasterStack,
            ..Default::default()
        };
        layout_settings.master_stack.new_window_placement =
            crate::common::config::MasterStackNewWindowPlacement::Stack;
        let mut engine = LayoutEngine::new(&settings, &layout_settings);
        let mut store = OwnedWindows::default();
        let space = SpaceId::new(92);
        let screen = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(1200.0, 800.0));
        let _ = engine.handle_event(
            &mut store.view_mut(),
            LayoutEvent::SpaceExposed(space, screen.size),
        );
        (engine, store, space, screen)
    }

    fn observe(
        engine: &mut LayoutEngine,
        store: &mut WindowsMut<'_>,
        space: SpaceId,
        window: WindowId,
        bundle_id: Option<&str>,
        min_size: CGSize,
    ) {
        let info = WindowLayoutInfo {
            min_size: Some(min_size),
            ..window_layout_info(window, CGSize::new(500.0, 500.0))
        };
        let app = AppInfo {
            bundle_id: bundle_id.map(Into::into),
            localized_name: None,
        };
        // The engine projects only windows the store admits (#702).
        if !store.contains_window(window) {
            store.insert_manageable_window(window, live_window(CGSize::new(500.0, 500.0)));
        }
        let _ = engine.handle_event(
            store,
            LayoutEvent::windows_observed(space, window.pid, vec![info], Some(app)),
        );
    }

    fn tiled_height(
        engine: &mut LayoutEngine,
        windows: Windows<'_>,
        space: SpaceId,
        screen: CGRect,
        window: WindowId,
    ) -> f64 {
        let frames = engine.calculate_layout(
            windows,
            space,
            screen,
            &Default::default(),
            Default::default(),
        );
        frames
            .into_iter()
            .find_map(|(w, f)| (w == window).then_some(f.size.height))
            .expect("window is tiled")
    }

    #[test]
    fn respect_min_size_keeps_a_stacked_window_at_its_apps_minimum_height() {
        const CONSTRAINED: Option<&str> = Some("com.example.Constrained");
        for (respect_min_size, expected) in [(false, 400.0), (true, 600.0)] {
            let (mut engine, mut store, space, screen) = master_stack_with_rule(respect_min_size);
            for pid in [7, 8] {
                observe(
                    &mut engine,
                    &mut store.view_mut(),
                    space,
                    WindowId::new(pid, 1),
                    None,
                    CGSize::ZERO,
                );
            }
            let first = WindowId::new(9, 1);
            observe(
                &mut engine,
                &mut store.view_mut(),
                space,
                first,
                CONSTRAINED,
                CGSize::new(400.0, 600.0),
            );
            assert_eq!(
                tiled_height(&mut engine, store.view(), space, screen, first),
                expected
            );

            // Its replacement reports no minimum yet: the app's last one stands in.
            let replacement = WindowId::new(9, 2);
            let _ = engine.handle_event(&mut store.view_mut(), LayoutEvent::WindowRemoved(first));
            observe(
                &mut engine,
                &mut store.view_mut(),
                space,
                replacement,
                CONSTRAINED,
                CGSize::ZERO,
            );
            assert_eq!(
                tiled_height(&mut engine, store.view(), space, screen, replacement),
                expected
            );
        }
    }
}
