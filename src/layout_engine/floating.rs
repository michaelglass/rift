use objc2_core_foundation::CGRect;
use serde::{Deserialize, Serialize};

use crate::actor::app::{WindowId, pid_t};
use crate::common::collections::BTreeSet;
use crate::model::VirtualWorkspaceId;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FloatingFullscreenKind {
    Full,
    WithinGaps,
}

/// The floating windows and the floating focus as a layout is saved (the live state is the
/// engine's [`EngineFloats`]).
#[derive(Serialize, Deserialize, Default)]
pub(crate) struct FloatingManager {
    floating_windows: BTreeSet<WindowId>,
    last_floating_focus: Option<WindowId>,
}

impl FloatingManager {
    /// Saved floating windows leave the live marker set: they exist as restoration candidates
    /// until they match a window.
    pub(crate) fn take_persisted_windows(&mut self) -> BTreeSet<WindowId> {
        std::mem::take(&mut self.floating_windows)
    }

    pub(crate) fn set_last_focus(&mut self, wid: Option<WindowId>) {
        self.last_floating_focus = wid;
    }

    pub(crate) fn last_focus(&self) -> Option<WindowId> { self.last_floating_focus }

    /// A saved form marking `windows` floating (a test's file, written as older versions did).
    #[cfg(test)]
    pub(crate) fn marking(windows: &[WindowId]) -> Self {
        FloatingManager {
            floating_windows: windows.iter().copied().collect(),
            last_floating_focus: None,
        }
    }
}

type FloatsCore = crate::roc_twin::RiftFloats;

type FloatsEvent = crate::roc_twin::RiftFloatsEvent;

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

fn engine_kind(kind: rift_core::floats::FullscreenKind) -> FloatingFullscreenKind {
    match kind {
        rift_core::floats::FullscreenKind::Full => FloatingFullscreenKind::Full,
        rift_core::floats::FullscreenKind::WithinGaps => FloatingFullscreenKind::WithinGaps,
    }
}

/// The engine's floating state: the floating windows and their fullscreen, the floating focus, each workspace's floating frames and the focused window
/// (engine slice E2), all of it the core machine's ([`rift_core::floats`]): the engine steps it
/// and reads it back.
pub(crate) struct EngineFloats {
    core: crate::roc_twin::TwinnedFloats,
}

impl EngineFloats {
    pub(crate) fn new() -> Self { EngineFloats { core: Default::default() } }

    /// The floating state a saved layout brings: its floating windows and floating focus.
    pub(crate) fn loaded(floating: FloatingManager) -> Self {
        EngineFloats {
            core: crate::roc_twin::TwinnedFloats::seeded(FloatsCore::from_parts(
                floating.floating_windows,
                [],
                floating.last_floating_focus,
                None,
                [],
            )),
        }
    }

    fn step(&mut self, event: FloatsEvent) { self.core.step(event); }

    pub(crate) fn is_floating(&self, window: WindowId) -> bool { self.core.is_floating(window) }

    pub(crate) fn add_floating(&mut self, window: WindowId) {
        self.step(FloatsEvent::Floated(window));
    }

    pub(crate) fn remove_floating(&mut self, window: WindowId) {
        self.step(FloatsEvent::Unfloated(window));
    }

    pub(crate) fn set_fullscreen(
        &mut self,
        window: WindowId,
        kind: Option<FloatingFullscreenKind>,
    ) {
        self.step(FloatsEvent::Fullscreen(window, kind.map(core_kind)));
    }

    pub(crate) fn fullscreen_kind(&self, window: WindowId) -> Option<FloatingFullscreenKind> {
        self.core.fullscreen_kind(window).map(engine_kind)
    }

    pub(crate) fn set_last_focus(&mut self, window: Option<WindowId>) {
        self.step(FloatsEvent::FloatingFocus(window));
    }

    pub(crate) fn last_focus(&self) -> Option<WindowId> { self.core.floating_focus() }

    /// The engine's focused window.
    pub(crate) fn focused(&self) -> Option<WindowId> { self.core.focused() }

    pub(crate) fn set_focused(&mut self, window: Option<WindowId>) {
        self.step(FloatsEvent::Focused(window));
    }

    /// `pid` quit: its windows' floating state and frames go.
    pub(crate) fn app_quit(&mut self, pid: pid_t) { self.step(FloatsEvent::AppQuit(pid)); }

    /// `from` is now `to`: its floating state, the floating focus and its frames follow it.
    pub(crate) fn replace_identity(&mut self, from: WindowId, to: WindowId) {
        self.step(FloatsEvent::Replaced { from, to });
    }

    pub(crate) fn store(&mut self, workspace: VirtualWorkspaceId, window: WindowId, frame: CGRect) {
        self.step(FloatsEvent::FrameStored {
            workspace,
            window,
            frame,
            if_absent: false,
        });
    }

    pub(crate) fn store_if_absent(
        &mut self,
        workspace: VirtualWorkspaceId,
        window: WindowId,
        frame: CGRect,
    ) {
        self.step(FloatsEvent::FrameStored {
            workspace,
            window,
            frame,
            if_absent: true,
        });
    }

    pub(crate) fn get(&self, workspace: VirtualWorkspaceId, window: WindowId) -> Option<CGRect> {
        self.core.frame(workspace, window)
    }

    pub(crate) fn workspace_positions(
        &self,
        workspace: VirtualWorkspaceId,
    ) -> Vec<(WindowId, CGRect)> {
        self.core.workspace_frames(workspace)
    }

    pub(crate) fn remove_window(&mut self, window: WindowId) {
        self.step(FloatsEvent::FramesForgotten(window));
    }

    pub(crate) fn remove_workspace_window(
        &mut self,
        workspace: VirtualWorkspaceId,
        window: WindowId,
    ) {
        self.step(FloatsEvent::FrameForgotten { workspace, window });
    }

    pub(crate) fn remove_workspace(&mut self, workspace: VirtualWorkspaceId) {
        self.step(FloatsEvent::WorkspaceForgotten(workspace));
    }

    /// The saved form of the floating windows among `windows` and the floating focus on one.
    pub(crate) fn persisted_view(
        &self,
        windows: impl Iterator<Item = WindowId>,
    ) -> FloatingManager {
        let floating_windows: BTreeSet<_> = windows.collect();
        FloatingManager {
            last_floating_focus: self
                .core
                .floating_focus()
                .filter(|focus| floating_windows.contains(focus)),
            floating_windows,
        }
    }
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::{CGPoint, CGSize};

    use super::*;

    /// A frame stored only if absent leaves one already there.
    #[test]
    fn a_frame_stored_if_absent_keeps_the_one_there() {
        let mut floats = EngineFloats::new();
        let workspace: VirtualWorkspaceId = slotmap::KeyData::from_ffi(1).into();
        let window = WindowId::new(4, 1);
        let at = |x| CGRect::new(CGPoint::new(x, 0.0), CGSize::new(100.0, 100.0));
        floats.store(workspace, window, at(1.0));
        floats.store_if_absent(workspace, window, at(2.0));
        assert_eq!(floats.get(workspace, window), Some(at(1.0)));
    }

    /// A saved layout's floating windows and floating focus come back.
    #[test]
    fn a_saved_layout_brings_its_floating_windows_and_floating_focus() {
        let (a, b) = (WindowId::new(4, 1), WindowId::new(5, 1));
        let mut live = EngineFloats::new();
        live.add_floating(a);
        live.set_last_focus(Some(a));
        let saved = live.persisted_view([a, b].into_iter());
        let loaded = EngineFloats::loaded(saved);
        assert!(loaded.is_floating(a) && loaded.is_floating(b));
        assert_eq!(loaded.last_focus(), Some(a));
        // A floating focus on a window the saved form leaves out is not saved.
        assert_eq!(live.persisted_view([b].into_iter()).last_focus(), None);
    }
}
