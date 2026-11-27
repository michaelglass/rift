use serde::{Deserialize, Serialize};

use crate::actor::app::{WindowId, pid_t};
use crate::common::collections::{BTreeExt, BTreeSet, HashMap, HashSet};
use crate::sys::screen::SpaceId;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FloatingFullscreenKind {
    Full,
    WithinGaps,
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct FloatingManager {
    floating_windows: BTreeSet<WindowId>,
    #[serde(skip)]
    active_floating_windows: HashMap<SpaceId, HashMap<pid_t, HashSet<WindowId>>>,
    last_floating_focus: Option<WindowId>,
    #[serde(skip)]
    fullscreen_windows: HashMap<WindowId, FloatingFullscreenKind>,
}

impl FloatingManager {
    pub(crate) fn new() -> Self { Self::default() }

    pub(crate) fn is_floating(&self, window_id: WindowId) -> bool {
        self.floating_windows.contains(&window_id)
    }

    /// Saved floating windows leave the live marker set: they exist as restoration candidates
    /// until they match a window.
    pub(crate) fn take_persisted_windows(&mut self) -> BTreeSet<WindowId> {
        std::mem::take(&mut self.floating_windows)
    }

    /// The serialized view of `windows`, which are live and pending floating windows alike.
    pub(crate) fn persisted_view(&self, windows: impl Iterator<Item = WindowId>) -> Self {
        let floating_windows: BTreeSet<_> = windows.collect();
        Self {
            last_floating_focus: self
                .last_floating_focus
                .filter(|focus| floating_windows.contains(focus)),
            floating_windows,
            ..Self::default()
        }
    }

    pub(crate) fn add_floating(&mut self, window_id: WindowId) {
        self.floating_windows.insert(window_id);
    }

    pub(crate) fn remove_floating(&mut self, window_id: WindowId) {
        self.floating_windows.remove(&window_id);
        self.fullscreen_windows.remove(&window_id);
        self.remove_active_entries(window_id);
        if self.last_floating_focus == Some(window_id) {
            self.last_floating_focus = None;
        }
    }

    pub(crate) fn set_fullscreen(
        &mut self,
        window_id: WindowId,
        kind: Option<FloatingFullscreenKind>,
    ) {
        match kind {
            Some(k) => {
                self.fullscreen_windows.insert(window_id, k);
            }
            None => {
                self.fullscreen_windows.remove(&window_id);
            }
        }
    }

    pub(crate) fn fullscreen_kind(&self, window_id: WindowId) -> Option<FloatingFullscreenKind> {
        self.fullscreen_windows.get(&window_id).copied()
    }

    pub(crate) fn add_active(&mut self, space: SpaceId, pid: pid_t, wid: WindowId) {
        self.active_floating_windows
            .entry(space)
            .or_default()
            .entry(pid)
            .or_default()
            .insert(wid);
    }

    pub(crate) fn remove_active(&mut self, space: SpaceId, pid: pid_t, wid: WindowId) {
        if let Some(space_map) = self.active_floating_windows.get_mut(&space) {
            if let Some(app_set) = space_map.get_mut(&pid) {
                app_set.remove(&wid);
                if app_set.is_empty() {
                    space_map.remove(&pid);
                }
            }
        }
    }

    pub(crate) fn remove_active_for_window(&mut self, window_id: WindowId) {
        self.remove_active_entries(window_id);
    }

    pub(crate) fn transfer_window_identity(&mut self, from: WindowId, to: WindowId) {
        if from == to {
            return;
        }

        // Identity transfer is replacement, not union. `to` may already have provisional live
        // state while `from` carries restored state. Keeping both lets stale floating/fullscreen
        // flags survive reconciliation and disagree with the restored workspace tree.
        let was_floating = self.floating_windows.remove(&from);
        self.floating_windows.remove(&to);
        if was_floating {
            self.floating_windows.insert(to);
        }

        let fullscreen = self.fullscreen_windows.remove(&from);
        self.fullscreen_windows.remove(&to);
        if let Some(k) = fullscreen {
            self.fullscreen_windows.insert(to, k);
        }

        let active_spaces: Vec<_> = self
            .active_floating_windows
            .iter()
            .filter_map(|(space, apps)| {
                apps.get(&from.pid)
                    .is_some_and(|windows| windows.contains(&from))
                    .then_some(*space)
            })
            .collect();
        self.remove_active_entries(from);
        self.remove_active_entries(to);
        for space in active_spaces {
            self.add_active(space, to.pid, to);
        }

        if self.last_floating_focus == Some(from) {
            self.last_floating_focus = Some(to);
        }
    }

    pub(crate) fn active_windows(&self, space: SpaceId) -> impl Iterator<Item = WindowId> + '_ {
        self.active_floating_windows
            .get(&space)
            .into_iter()
            .flat_map(|apps| apps.values().flatten().copied())
    }

    pub(crate) fn is_active(&self, space: SpaceId, window: WindowId) -> bool {
        self.active_floating_windows
            .get(&space)
            .and_then(|apps| apps.get(&window.pid))
            .is_some_and(|windows| windows.contains(&window))
    }

    pub(crate) fn set_last_focus(&mut self, wid: Option<WindowId>) {
        self.last_floating_focus = wid;
    }

    pub(crate) fn last_focus(&self) -> Option<WindowId> { self.last_floating_focus }

    pub(crate) fn remove_all_for_pid(&mut self, pid: pid_t) {
        let _ = self.floating_windows.remove_all_for_pid(pid);

        self.fullscreen_windows.retain(|w, _| w.pid != pid);

        for space_map in self.active_floating_windows.values_mut() {
            space_map.remove(&pid);
        }

        if let Some(focus) = self.last_floating_focus {
            if focus.pid == pid {
                self.last_floating_focus = None;
            }
        }
    }

    pub(crate) fn rebuild_active_for_workspace(
        &mut self,
        space: SpaceId,
        windows_in_workspace: Vec<WindowId>,
    ) {
        let space_map = self.active_floating_windows.entry(space).or_default();
        space_map.clear();
        for wid in windows_in_workspace.into_iter().filter(|&w| self.floating_windows.contains(&w))
        {
            space_map.entry(wid.pid).or_default().insert(wid);
        }
    }

    pub(crate) fn remap_space(&mut self, old_space: SpaceId, new_space: SpaceId) {
        if old_space == new_space {
            return;
        }

        let mut merged = self.active_floating_windows.remove(&new_space).unwrap_or_default();

        if let Some(old) = self.active_floating_windows.remove(&old_space) {
            for (pid, windows) in old {
                merged.entry(pid).or_default().extend(windows);
            }
        }

        if !merged.is_empty() {
            self.active_floating_windows.insert(new_space, merged);
        }
    }

    fn remove_active_entries(&mut self, window_id: WindowId) {
        for space_map in self.active_floating_windows.values_mut() {
            if let Some(app_set) = space_map.get_mut(&window_id.pid) {
                app_set.remove(&window_id);
                if app_set.is_empty() {
                    space_map.remove(&window_id.pid);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame stored only if absent leaves one already there.
    #[test]
    fn a_frame_stored_if_absent_keeps_the_one_there() {
        use objc2_core_foundation::{CGPoint, CGRect, CGSize};
        let mut floats = EngineFloats::new();
        let workspace: crate::model::VirtualWorkspaceId = slotmap::KeyData::from_ffi(1).into();
        let window = WindowId::new(4, 1);
        let at = |x| CGRect::new(CGPoint::new(x, 0.0), CGSize::new(100.0, 100.0));
        floats.store(workspace, window, at(1.0));
        floats.store_if_absent(workspace, window, at(2.0));
        assert_eq!(floats.get(workspace, window), Some(at(1.0)));
    }

    #[test]
    fn identity_transfer_replaces_provisional_target_state() {
        let mut floating = FloatingManager::new();
        let restored_tiled = WindowId::new(1, 1);
        let provisional_live = WindowId::new(2, 2);
        let space = SpaceId::new(3);

        floating.add_floating(provisional_live);
        floating.set_fullscreen(provisional_live, Some(FloatingFullscreenKind::Full));
        floating.add_active(space, provisional_live.pid, provisional_live);

        floating.transfer_window_identity(restored_tiled, provisional_live);

        assert!(!floating.is_floating(provisional_live));
        assert_eq!(floating.fullscreen_kind(provisional_live), None);
        assert!(!floating.is_active(space, provisional_live));
    }
}

/// The core machine as the engine holds it: windows, Spaces, workspaces and frames in rift's
/// types (engine slice E2).
pub(crate) type FloatsCore = rift_core::floats::Floats<
    WindowId,
    SpaceId,
    crate::model::VirtualWorkspaceId,
    objc2_core_foundation::CGRect,
>;

type FloatsEvent = rift_core::floats::FloatsEvent<
    WindowId,
    pid_t,
    SpaceId,
    crate::model::VirtualWorkspaceId,
    objc2_core_foundation::CGRect,
>;

impl rift_core::floats::OfApp for WindowId {
    type App = pid_t;

    fn app(&self) -> pid_t { self.pid }
}

fn core_kind(kind: FloatingFullscreenKind) -> rift_core::floats::FullscreenKind {
    match kind {
        FloatingFullscreenKind::Full => rift_core::floats::FullscreenKind::Full,
        FloatingFullscreenKind::WithinGaps => rift_core::floats::FullscreenKind::WithinGaps,
    }
}

/// The engine's floating state: the floating windows and their fullscreen, each Space's active
/// floating windows, the floating focus, each workspace's floating frames and the focused window
/// (engine slice E2). While the core machine shadows them it is stepped beside the engine's
/// `FloatingManager`, `FloatingPositionStore` and focused window, and in tests checked against
/// them after every change.
pub(crate) struct EngineFloats {
    floating: FloatingManager,
    positions: crate::model::FloatingPositionStore,
    focused: Option<WindowId>,
    core: FloatsCore,
}

impl EngineFloats {
    pub(crate) fn new() -> Self {
        EngineFloats {
            floating: FloatingManager::new(),
            positions: Default::default(),
            focused: None,
            core: Default::default(),
        }
    }

    /// The floating state a saved layout brings: its floating windows and floating focus.
    pub(crate) fn loaded(floating: FloatingManager) -> Self {
        let core = FloatsCore::from_parts(
            floating.floating_windows.iter().copied(),
            floating.fullscreen_windows.iter().map(|(&w, &k)| (w, core_kind(k))),
            floating
                .active_floating_windows
                .iter()
                .flat_map(|(&s, apps)| apps.values().flatten().map(move |&w| (s, w))),
            floating.last_floating_focus,
            None,
            [],
        );
        EngineFloats {
            floating,
            positions: Default::default(),
            focused: None,
            core,
        }
    }

    fn step(&mut self, event: FloatsEvent) {
        #[cfg(test)]
        let shown = format!("{event:?}");
        self.core.step(event);
        #[cfg(test)]
        self.check(&shown);
    }

    /// The core against the engine's own state, every part of it.
    #[cfg(test)]
    fn check(&self, event: &str) {
        let f = &self.floating;
        let floating: Vec<_> = f.floating_windows.iter().copied().collect();
        assert_eq!(
            self.core.floating().collect::<Vec<_>>(),
            floating,
            "floating, {event}"
        );
        let mut fullscreen: Vec<_> =
            f.fullscreen_windows.iter().map(|(&w, &k)| (w, core_kind(k))).collect();
        fullscreen.sort_by_key(|&(w, _)| w);
        assert_eq!(
            self.core.fullscreens().collect::<Vec<_>>(),
            fullscreen,
            "fullscreen, {event}"
        );
        let mut active: Vec<_> = f
            .active_floating_windows
            .iter()
            .flat_map(|(&s, apps)| apps.values().flatten().map(move |&w| (s, w)))
            .collect();
        active.sort();
        assert_eq!(self.core.active().collect::<Vec<_>>(), active, "active, {event}");
        assert_eq!(
            self.core.floating_focus(),
            f.last_floating_focus,
            "floating focus, {event}"
        );
        assert_eq!(self.core.focused(), self.focused, "focused, {event}");
        let frames: Vec<_> = self.positions.clone_sorted();
        assert_eq!(self.core.frames().collect::<Vec<_>>(), frames, "frames, {event}");
    }

    pub(crate) fn is_floating(&self, window: WindowId) -> bool { self.floating.is_floating(window) }

    pub(crate) fn add_floating(&mut self, window: WindowId) {
        self.floating.add_floating(window);
        self.step(FloatsEvent::Floated(window));
    }

    pub(crate) fn remove_floating(&mut self, window: WindowId) {
        self.floating.remove_floating(window);
        self.step(FloatsEvent::Unfloated(window));
    }

    pub(crate) fn set_fullscreen(
        &mut self,
        window: WindowId,
        kind: Option<FloatingFullscreenKind>,
    ) {
        self.floating.set_fullscreen(window, kind);
        self.step(FloatsEvent::Fullscreen(window, kind.map(core_kind)));
    }

    pub(crate) fn fullscreen_kind(&self, window: WindowId) -> Option<FloatingFullscreenKind> {
        self.floating.fullscreen_kind(window)
    }

    pub(crate) fn add_active(&mut self, space: SpaceId, pid: pid_t, window: WindowId) {
        self.floating.add_active(space, pid, window);
        self.step(FloatsEvent::Activated { space, window });
    }

    pub(crate) fn remove_active(&mut self, space: SpaceId, pid: pid_t, window: WindowId) {
        self.floating.remove_active(space, pid, window);
        self.step(FloatsEvent::Deactivated { space, window });
    }

    pub(crate) fn remove_active_for_window(&mut self, window: WindowId) {
        self.floating.remove_active_for_window(window);
        self.step(FloatsEvent::DeactivatedEverywhere(window));
    }

    pub(crate) fn active_windows(&self, space: SpaceId) -> impl Iterator<Item = WindowId> + '_ {
        self.floating.active_windows(space)
    }

    pub(crate) fn is_active(&self, space: SpaceId, window: WindowId) -> bool {
        self.floating.is_active(space, window)
    }

    pub(crate) fn set_last_focus(&mut self, window: Option<WindowId>) {
        self.floating.set_last_focus(window);
        self.step(FloatsEvent::FloatingFocus(window));
    }

    pub(crate) fn last_focus(&self) -> Option<WindowId> { self.floating.last_focus() }

    pub(crate) fn rebuild_active_for_workspace(&mut self, space: SpaceId, windows: Vec<WindowId>) {
        self.floating.rebuild_active_for_workspace(space, windows.clone());
        self.step(FloatsEvent::ActiveRebuilt { space, windows });
    }

    pub(crate) fn remap_space(&mut self, from: SpaceId, to: SpaceId) {
        self.floating.remap_space(from, to);
        self.step(FloatsEvent::SpaceRemapped { from, to });
    }

    /// The engine's focused window.
    pub(crate) fn focused(&self) -> Option<WindowId> { self.focused }

    pub(crate) fn set_focused(&mut self, window: Option<WindowId>) {
        self.focused = window;
        self.step(FloatsEvent::Focused(window));
    }

    /// `pid` quit: its windows' floating state and frames go.
    pub(crate) fn app_quit(&mut self, pid: pid_t) {
        self.floating.remove_all_for_pid(pid);
        self.positions.remove_app(pid);
        self.step(FloatsEvent::AppQuit(pid));
    }

    /// `from` is now `to`: its floating state, the floating focus and its frames follow it.
    pub(crate) fn replace_identity(&mut self, from: WindowId, to: WindowId) {
        self.positions.transfer_window_identity(from, to);
        self.floating.transfer_window_identity(from, to);
        self.step(FloatsEvent::Replaced { from, to });
    }

    pub(crate) fn store(
        &mut self,
        workspace: crate::model::VirtualWorkspaceId,
        window: WindowId,
        frame: objc2_core_foundation::CGRect,
    ) {
        self.positions.store(workspace, window, frame);
        self.step(FloatsEvent::FrameStored {
            workspace,
            window,
            frame,
            if_absent: false,
        });
    }

    pub(crate) fn store_if_absent(
        &mut self,
        workspace: crate::model::VirtualWorkspaceId,
        window: WindowId,
        frame: objc2_core_foundation::CGRect,
    ) {
        self.positions.store_if_absent(workspace, window, frame);
        self.step(FloatsEvent::FrameStored {
            workspace,
            window,
            frame,
            if_absent: true,
        });
    }

    pub(crate) fn get(
        &self,
        workspace: crate::model::VirtualWorkspaceId,
        window: WindowId,
    ) -> Option<objc2_core_foundation::CGRect> {
        self.positions.get(workspace, window)
    }

    pub(crate) fn workspace_positions(
        &self,
        workspace: crate::model::VirtualWorkspaceId,
    ) -> Vec<(WindowId, objc2_core_foundation::CGRect)> {
        self.positions.workspace_positions(workspace)
    }

    pub(crate) fn remove_window(&mut self, window: WindowId) {
        self.positions.remove_window(window);
        self.step(FloatsEvent::FramesForgotten(window));
    }

    pub(crate) fn remove_workspace_window(
        &mut self,
        workspace: crate::model::VirtualWorkspaceId,
        window: WindowId,
    ) {
        self.positions.remove_workspace_window(workspace, window);
        self.step(FloatsEvent::FrameForgotten { workspace, window });
    }

    pub(crate) fn remove_workspace(&mut self, workspace: crate::model::VirtualWorkspaceId) {
        self.positions.remove_workspace(workspace);
        self.step(FloatsEvent::WorkspaceForgotten(workspace));
    }

    /// The saved form of the floating windows among `windows` and the floating focus.
    pub(crate) fn persisted_view(
        &self,
        windows: impl Iterator<Item = WindowId>,
    ) -> FloatingManager {
        self.floating.persisted_view(windows)
    }
}
