use rift_core::main_window::{Initiator, MainWindowEvent, Standing};

use super::Event;
use crate::actor::app::{Quiet, WindowId, pid_t};

/// The main window, as the core's machine (`rift_core::main_window`, twinned in Roc under
/// `roc-core`) decides it from what the reactor observes: this only translates events.
#[derive(Default)]
pub(crate) struct MainWindowTracker {
    windows: crate::roc_twin::TwinnedMainWindows,
}

fn initiator(quiet: Quiet) -> Initiator {
    match quiet {
        Quiet::Yes => Initiator::Rift,
        Quiet::No => Initiator::Other,
    }
}

impl MainWindowTracker {
    /// The window to focus, if `event` makes one known (only before the window server's first
    /// focus report).
    #[must_use]
    pub fn handle_event(&mut self, event: &Event) -> Option<WindowId> {
        let event = match *event {
            Event::ApplicationLaunched {
                pid, is_frontmost, main_window, ..
            } => MainWindowEvent::Launched {
                pid,
                standing: if is_frontmost {
                    Standing::Frontmost
                } else {
                    Standing::Background
                },
                main_window,
            },
            Event::ApplicationThreadTerminated(pid) => MainWindowEvent::Terminated(pid),
            Event::ApplicationActivated(pid, quiet) => {
                MainWindowEvent::Activated(pid, initiator(quiet))
            }
            Event::ApplicationDeactivated(pid) => MainWindowEvent::Deactivated(pid),
            Event::ApplicationGloballyActivated(pid) => MainWindowEvent::GloballyActivated(pid),
            Event::ApplicationGloballyDeactivated(pid) => MainWindowEvent::GloballyDeactivated(pid),
            Event::ApplicationMainWindowChanged(pid, window, quiet) => {
                MainWindowEvent::MainWindowChanged(pid, window, initiator(quiet))
            }
            Event::WindowServerFocusChanged(window, _) => {
                MainWindowEvent::NativeFocused { pid: window.pid, window }
            }
            _ => return None,
        };
        self.windows.step(event)
    }

    /// `wid` is retired (no replacement) or has a new identity (upstream #702).
    pub(super) fn replace_window(&mut self, wid: WindowId, replacement: Option<WindowId>) {
        let _ = self.windows.step(MainWindowEvent::Replaced {
            pid: wid.pid,
            window: wid,
            replacement,
        });
    }

    pub fn main_window(&self) -> Option<WindowId> { self.windows.main_window() }

    pub fn window_server_focus(&self) -> Option<WindowId> { self.windows.native_focus() }

    pub fn is_globally_frontmost(&self, pid: pid_t) -> bool {
        self.windows.is_globally_frontmost(pid)
    }

    pub fn app_main_window(&self, pid: pid_t) -> Option<WindowId> {
        self.windows.app_main_window(pid)
    }
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use test_log::test;

    use super::super::testing::{Apps, make_windows, space_state_event};
    use super::super::{Event, Quiet, Reactor, SpaceId, WindowId};
    use super::MainWindowTracker;
    use crate::layout_engine::LayoutEngine;

    #[test]
    fn window_server_focus_supersedes_ax_focus_events() {
        let ax_window = WindowId::new(7, 1);
        let server_window = WindowId::new(7, 2);
        let stale_window = WindowId::new(7, 3);
        let mut tracker = MainWindowTracker::default();
        let _ = tracker.windows.step(rift_core::main_window::MainWindowEvent::Launched {
            pid: 7,
            standing: rift_core::main_window::Standing::Frontmost,
            main_window: Some(ax_window),
        });
        let _ = tracker
            .windows
            .step(rift_core::main_window::MainWindowEvent::GloballyActivated(7));

        assert_eq!(tracker.main_window(), Some(ax_window));
        assert_eq!(
            tracker.handle_event(&Event::WindowServerFocusChanged(server_window, SpaceId::new(1),)),
            None
        );
        assert_eq!(tracker.main_window(), Some(server_window));

        assert_eq!(
            tracker.handle_event(&Event::ApplicationMainWindowChanged(
                7,
                Some(stale_window),
                Quiet::No,
            )),
            None,
            "AX must not drive focus after native authority is initialized"
        );
        assert_eq!(tracker.main_window(), Some(server_window));

        let _ = tracker.handle_event(&Event::ApplicationMainWindowChanged(
            7,
            Some(ax_window),
            Quiet::No,
        ));

        tracker.replace_window(server_window, None);
        assert_eq!(tracker.main_window(), Some(ax_window));
    }

    #[test]
    fn it_tracks_frontmost_app_and_main_window_correctly() {
        use Event::*;
        let mut apps = Apps::new();
        let mut reactor = Reactor::new_for_test(LayoutEngine::new(
            &crate::common::config::VirtualWorkspaceSettings::default(),
            &crate::common::config::LayoutSettings::default(),
        ));
        let space = SpaceId::new(1);
        let screen_frame = CGRect::new(CGPoint::new(0., 0.), CGSize::new(1920., 1080.));
        reactor.handle_event(space_state_event(vec![screen_frame], vec![Some(space)]));
        assert_eq!(None, reactor.main_window());

        reactor.handle_event(ApplicationGloballyActivated(1));
        reactor.handle_events(apps.make_app_with_opts(
            1,
            make_windows(2),
            Some(WindowId::new(1, 1)),
            true,
            true,
        ));
        reactor.handle_events(apps.make_app_with_opts(2, make_windows(2), None, false, true));
        assert_eq!(Some(WindowId::new(1, 1)), reactor.main_window());
        assert_eq!(
            reactor.layout_manager.layout_engine.selected_window(space),
            Some(WindowId::new(1, 1))
        );

        reactor.handle_event(ApplicationGloballyDeactivated(1));
        assert_eq!(None, reactor.main_window());
        reactor.handle_event(ApplicationActivated(2, Quiet::No));
        reactor.handle_event(ApplicationGloballyActivated(2));
        assert_eq!(None, reactor.main_window());
        reactor.handle_event(ApplicationMainWindowChanged(
            2,
            Some(WindowId::new(2, 2)),
            Quiet::No,
        ));
        assert_eq!(Some(WindowId::new(2, 2)), reactor.main_window());
        assert_eq!(
            reactor.layout_manager.layout_engine.selected_window(space),
            Some(WindowId::new(2, 2))
        );
        reactor.handle_event(ApplicationMainWindowChanged(
            1,
            Some(WindowId::new(1, 2)),
            Quiet::No,
        ));
        assert_eq!(Some(WindowId::new(2, 2)), reactor.main_window());
        reactor.handle_event(ApplicationDeactivated(1));
        assert_eq!(Some(WindowId::new(2, 2)), reactor.main_window());
        reactor.handle_event(ApplicationDeactivated(2));
        assert_eq!(None, reactor.main_window());

        reactor.handle_event(ApplicationGloballyActivated(3));
        assert_eq!(None, reactor.main_window());

        reactor.handle_events(apps.make_app_with_opts(
            3,
            make_windows(2),
            Some(WindowId::new(3, 1)),
            true,
            true,
        ));
        assert_eq!(Some(WindowId::new(3, 1)), reactor.main_window());
        assert_eq!(
            reactor.layout_manager.layout_engine.selected_window(space),
            Some(WindowId::new(3, 1))
        );
    }

    #[test]
    fn it_does_not_update_layout_for_quiet_raises() {
        use Event::*;
        let mut apps = Apps::new();
        let mut reactor = Reactor::new_for_test(LayoutEngine::new(
            &crate::common::config::VirtualWorkspaceSettings::default(),
            &crate::common::config::LayoutSettings::default(),
        ));
        let space = SpaceId::new(1);
        let screen_frame = CGRect::new(CGPoint::new(0., 0.), CGSize::new(1920., 1080.));
        reactor.handle_event(space_state_event(vec![screen_frame], vec![Some(space)]));

        reactor.handle_event(ApplicationGloballyActivated(1));
        reactor.handle_events(apps.make_app_with_opts(
            1,
            make_windows(2),
            Some(WindowId::new(1, 1)),
            true,
            true,
        ));
        reactor.handle_events(apps.make_app_with_opts(2, make_windows(2), None, false, true));
        assert_eq!(Some(WindowId::new(1, 1)), reactor.main_window());
        assert_eq!(
            reactor.layout_manager.layout_engine.selected_window(space),
            Some(WindowId::new(1, 1))
        );

        reactor.handle_event(ApplicationGloballyDeactivated(1));
        assert_eq!(None, reactor.main_window());
        reactor.handle_event(ApplicationGloballyActivated(2));
        reactor.handle_event(ApplicationActivated(2, Quiet::Yes));
        assert_eq!(None, reactor.main_window());
        reactor.handle_event(ApplicationMainWindowChanged(
            2,
            Some(WindowId::new(2, 2)),
            Quiet::Yes,
        ));
        assert_eq!(Some(WindowId::new(2, 2)), reactor.main_window());
        assert_eq!(
            reactor.layout_manager.layout_engine.selected_window(space),
            Some(WindowId::new(1, 1))
        );

        reactor.handle_event(ApplicationActivated(2, Quiet::No));
        assert_eq!(
            reactor.layout_manager.layout_engine.selected_window(space),
            Some(WindowId::new(2, 2))
        );

        reactor.handle_event(ApplicationMainWindowChanged(
            2,
            Some(WindowId::new(2, 1)),
            Quiet::Yes,
        ));
        assert_eq!(Some(WindowId::new(2, 1)), reactor.main_window());
        assert_eq!(
            reactor.layout_manager.layout_engine.selected_window(space),
            Some(WindowId::new(2, 2))
        );

        reactor.handle_event(ApplicationActivated(1, Quiet::Yes));
        reactor.handle_event(ApplicationGloballyActivated(1));
        assert_eq!(Some(WindowId::new(1, 1)), reactor.main_window());
        assert_eq!(
            reactor.layout_manager.layout_engine.selected_window(space),
            Some(WindowId::new(2, 2))
        );

        reactor.handle_event(ApplicationMainWindowChanged(
            1,
            Some(WindowId::new(1, 2)),
            Quiet::No,
        ));
        assert_eq!(Some(WindowId::new(1, 2)), reactor.main_window());
        assert_eq!(
            reactor.layout_manager.layout_engine.selected_window(space),
            Some(WindowId::new(1, 2))
        );
    }

    #[test]
    fn it_selects_main_window_when_space_is_enabled() {
        use Event::*;
        let mut apps = Apps::new();
        let mut reactor = Reactor::new_for_test(LayoutEngine::new(
            &crate::common::config::VirtualWorkspaceSettings::default(),
            &crate::common::config::LayoutSettings::default(),
        ));
        let pid = 3;
        let windows = make_windows(2);
        let space = SpaceId::new(1);
        let screen_frame = CGRect::new(CGPoint::new(0., 0.), CGSize::new(1920., 1080.));
        reactor.handle_event(space_state_event(vec![screen_frame], vec![Some(space)]));

        reactor.handle_events(apps.make_app_with_opts(
            pid,
            windows,
            Some(WindowId::new(3, 1)),
            false,
            true,
        ));

        reactor.handle_event(space_state_event(vec![screen_frame], vec![None]));
        reactor.handle_event(ApplicationActivated(3, Quiet::No));
        reactor.handle_event(ApplicationGloballyActivated(3));
        reactor.discover_test_windows(pid, vec![], vec![WindowId::new(3, 1), WindowId::new(3, 2)]);
        assert_eq!(Some(WindowId::new(3, 1)), reactor.main_window());

        reactor.handle_event(space_state_event(vec![screen_frame], vec![Some(space)]));
        assert_eq!(
            reactor.layout_manager.layout_engine.selected_window(space),
            Some(WindowId::new(3, 1))
        );
    }
}
