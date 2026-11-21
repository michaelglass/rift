//! What macOS says about windows and Spaces: the one way the reactor asks the window server.
//! Decisions take these answers as inputs; `System` asks the window server, and tests answer
//! from a `Fixture`, the one place the two differ.

use objc2_core_foundation::{CGPoint, CGRect};

use crate::actor::app::WindowId;
use crate::model::window_store::WindowStore;
use crate::sys::screen::SpaceId;
use crate::sys::window_server::{self, WindowServerId};

pub(crate) trait Native {
    /// The Space a window is on, preferring a user Space when it is on several.
    fn window_space(&self, wsid: WindowServerId) -> Option<SpaceId>;
    /// Every Space a window is on.
    fn window_spaces(&self, wsid: WindowServerId) -> Vec<SpaceId>;
    /// Whether a Space is a user desktop (not fullscreen, not a system Space).
    fn space_is_user(&self, space: SpaceId) -> bool;
    /// The window with key focus on a Space's display, as the window server has it.
    fn key_focused_window(&self, space: SpaceId) -> Option<WindowId>;
    /// A window's frame, as the window server has it.
    fn window_frame(&self, wsid: WindowServerId) -> Option<CGRect>;
    /// The window under the cursor (Rift's own windows aside).
    fn window_under_cursor(&self) -> Option<WindowServerId> { window_server::window_under_cursor() }
    /// The tracked window at a point.
    fn tracked_window_at(&self, point: CGPoint, windows: &WindowStore) -> Option<WindowId>;
}

/// The window server.
#[cfg_attr(test, allow(dead_code))]
#[derive(Default)]
pub(crate) struct System;

impl Native for System {
    fn window_space(&self, wsid: WindowServerId) -> Option<SpaceId> {
        window_server::window_space(wsid)
    }

    fn window_spaces(&self, wsid: WindowServerId) -> Vec<SpaceId> {
        window_server::window_spaces(wsid)
    }

    fn space_is_user(&self, space: SpaceId) -> bool { window_server::space_is_user(space.get()) }

    fn key_focused_window(&self, space: SpaceId) -> Option<WindowId> {
        window_server::key_focused_window(space)
    }

    fn window_frame(&self, wsid: WindowServerId) -> Option<CGRect> {
        window_server::get_window(wsid).map(|window| window.frame)
    }

    fn tracked_window_at(&self, point: CGPoint, windows: &WindowStore) -> Option<WindowId> {
        windows.tracked_window_id(window_server::get_window_at_point(point)?)
    }
}

/// Answers for tests. Space membership and the cursor come from the window server's test
/// fixtures (`set_window_spaces_override`, `set_cursor_location_override`); every Space is a user desktop; key focus and window-server
/// frames are whatever the test sets.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Fixture {
    pub key_focused: Option<WindowId>,
    pub frames: crate::common::collections::HashMap<WindowServerId, CGRect>,
}

#[cfg(test)]
impl Native for Fixture {
    fn window_space(&self, wsid: WindowServerId) -> Option<SpaceId> {
        window_server::window_space(wsid)
    }

    fn window_spaces(&self, wsid: WindowServerId) -> Vec<SpaceId> {
        window_server::window_spaces(wsid)
    }

    fn space_is_user(&self, _space: SpaceId) -> bool { true }

    fn key_focused_window(&self, _space: SpaceId) -> Option<WindowId> { self.key_focused }

    fn window_frame(&self, wsid: WindowServerId) -> Option<CGRect> {
        self.frames.get(&wsid).copied()
    }

    /// The window whose known frame contains the point.
    fn tracked_window_at(&self, point: CGPoint, windows: &WindowStore) -> Option<WindowId> {
        windows.window_at(point)
    }
}

/// What the reactor asks.
#[cfg(not(test))]
pub(crate) type Source = System;
#[cfg(test)]
pub(crate) type Source = Fixture;
