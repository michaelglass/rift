use objc2_app_kit::NSNormalWindowLevel;

use crate::actor::app::WindowId;
use crate::actor::reactor::LayoutEvent;
use crate::actor::reactor::managers::LayoutManager;
use crate::model::RiftState;
use crate::sys::app::{WindowInfo, is_ax_transient, is_ibkr_desktop_dialog};
use crate::sys::screen::SpaceId;
use crate::sys::window_server::{WindowServerId, WindowServerInfo, window_is_sticky, window_level};

/// Re-runs the manageability heuristic for a window with AX state: the change it made to the
/// window's admission, if any.
pub(crate) fn refresh_heuristic(
    state: &mut RiftState,
    wid: WindowId,
) -> Option<rift_core::lifecycle::Transition<WindowId>> {
    let window = state.windows.window(wid)?;
    let manageable = compute_window_info_manageability(&window.info, |wsid| {
        state.windows.get_window_server_info(wsid)
    });
    state.windows.set_manageable(wid, manageable)
}

/// How a window that lost admission leaves the layout: suspended while it is away for now,
/// removed when nothing will bring it back.
pub(crate) fn layout_departure(state: &RiftState, wid: WindowId) -> LayoutEvent {
    match state.windows.availability(wid) {
        crate::model::window_store::Availability::TemporarilyUnavailable => {
            LayoutEvent::WindowSuspended(wid)
        }
        crate::model::window_store::Availability::Available => LayoutEvent::WindowRemoved(wid),
    }
}

pub(crate) fn rejection_needs_removal(
    state: &RiftState,
    layout: &LayoutManager,
    wid: WindowId,
    space: SpaceId,
) -> bool {
    let engine = &layout.layout_engine;
    if state.windows.is_temporarily_unavailable(wid)
        && state.windows.manage_override(wid) != Some(false)
    {
        return false;
    }
    engine.workspaces().workspace_for_window(&state.windows, space, wid).is_some()
        || engine.is_window_floating(wid)
}

pub(crate) fn clear_rule_admission(state: &mut RiftState, wid: WindowId) {
    state.windows.set_manage_override(wid, None);
}

pub(super) fn compute_window_info_manageability(
    info: &WindowInfo,
    mut window_server_info: impl FnMut(WindowServerId) -> Option<WindowServerInfo>,
) -> bool {
    if is_ax_transient(info) || info.is_minimized || !info.is_root {
        return false;
    }

    let allow_nonstandard_window = is_ibkr_desktop_dialog(info);
    if !info.is_standard && !allow_nonstandard_window {
        return false;
    }

    if let Some(wsid) = info.sys_id {
        if window_is_sticky(wsid) {
            return false;
        }
        if !allow_nonstandard_window {
            if window_server_info(wsid).is_some_and(|server| server.layer != 0)
                || window_level(wsid.0).is_some_and(|level| level != NSNormalWindowLevel)
            {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use objc2_core_foundation::{CGPoint, CGRect, CGSize};

    use super::compute_window_info_manageability;
    use crate::sys::app::WindowInfo;

    fn ax_window(subrole: &str, is_standard: bool) -> WindowInfo {
        WindowInfo {
            has_native_tabs: false,
            is_standard,
            is_root: true,
            is_minimized: false,
            is_resizable: true,
            title: "Window".into(),
            frame: CGRect::new(CGPoint::ZERO, CGSize::new(800.0, 600.0)),
            min_size: None,
            max_size: None,
            sys_id: None,
            bundle_id: None,
            path: None,
            ax_role: Some("AXWindow".into()),
            ax_subrole: Some(subrole.into()),
        }
    }

    #[test]
    fn standard_ax_window_without_window_server_metadata_is_manageable() {
        let info = ax_window("AXStandardWindow", true);

        assert!(compute_window_info_manageability(&info, |_| None));
    }

    #[test]
    fn standard_dialogs_and_sheets_are_not_documents() {
        for subrole in ["AXDialog", "AXSystemDialog", "AXSheet"] {
            assert!(!compute_window_info_manageability(
                &ax_window(subrole, true),
                |_| None
            ));
        }
    }

    #[test]
    fn non_window_ax_role_is_not_manageable() {
        let mut info = ax_window("AXStandardWindow", true);
        info.ax_role = Some("AXTextField".into());

        assert!(!compute_window_info_manageability(&info, |_| None));
    }

    #[test]
    fn synthesized_standard_window_without_ax_role_remains_manageable() {
        let mut info = ax_window("AXStandardWindow", true);
        info.ax_role = None;

        assert!(compute_window_info_manageability(&info, |_| None));
    }

    #[test]
    fn nonstandard_qt_tooltip_without_window_server_metadata_is_not_manageable() {
        let mut info = ax_window("AXUnknown", false);
        info.is_resizable = false;
        info.frame.size = CGSize::new(240.0, 32.0);
        info.bundle_id = Some("org.qgis.qgis3".into());

        assert!(!compute_window_info_manageability(&info, |_| None));
    }

    #[test]
    fn known_ibkr_dialog_remains_manageable() {
        let mut info = ax_window("AXDialog", false);
        info.frame.size = CGSize::new(1400.0, 900.0);
        info.bundle_id = Some("com.install4j.runtime.123".into());
        info.path = Some(PathBuf::from("file:///Applications/IBKR%20Desktop.app"));

        assert!(compute_window_info_manageability(&info, |_| None));
        // Preserve the original compatibility case based on maximum size,
        // without imposing a new resizability requirement.
        info.frame.size = CGSize::new(800.0, 600.0);
        info.max_size = Some(CGSize::new(1400.0, 900.0));
        info.is_resizable = false;
        assert!(compute_window_info_manageability(&info, |_| None));
    }
}
