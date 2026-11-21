use objc2_core_foundation::CGRect;
use tracing::debug;

use crate::actor::app::WindowId;
use crate::actor::drag::{DragSource, Session};
use crate::actor::reactor::events::EventOutcome;
use crate::actor::reactor::reconciler::CoreEvent;
use crate::actor::reactor::{Quiet, WindowState, utils};
use crate::layout_engine::LayoutEvent;
use crate::model::tx_store::WindowTxStore;
use crate::model::window_store::Pointer;
use crate::sys::app::WindowInfo as Window;
use crate::sys::geometry::SameAs;
use crate::sys::screen::SpaceId;
use crate::sys::window_server::WindowServerInfo;

#[derive(Debug)]
pub struct WindowCreatedPayload {
    pub window_id: WindowId,
    pub window: Window,
    pub window_server_info: Option<WindowServerInfo>,
}

pub fn handle_window_created(
    state: &mut crate::model::RiftState,
    layout: &mut crate::actor::reactor::managers::LayoutManager,
    payload: WindowCreatedPayload,
) -> anyhow::Result<EventOutcome> {
    let WindowCreatedPayload {
        window_id: wid,
        window,
        window_server_info: ws_info,
    } = payload;
    let rekeyed = state.reconcile_ax_identity(wid, window.sys_id);
    if let Some(previous) = rekeyed {
        layout.layout_engine.transfer_persistent_window_identity(previous, wid);
    }
    if let Some(info) = ws_info {
        state.windows.clear_window_server_observed(info.id);
        state.windows.track_window_server_info(info);
    }

    // A fresh AX reading keeps the rule the window was under.
    let rule = state.windows.rule(wid).unwrap_or(rift_core::lifecycle::Rule::NoRule);
    state.insert_window_as(
        wid,
        WindowState::from(window),
        rift_core::lifecycle::Manageability::Unmanageable,
        rule,
    );
    let _ = utils::refresh_heuristic(state, wid);

    // Excluded and claimed windows only need lifecycle notifications; app-rule admission
    // arranges itself.
    // A re-keyed window is arranged whatever its admission (unless an outside owner holds it):
    // anything pending for the old identity died with it, and the core writes the desire again
    // for the new one.
    let arrange =
        state.windows.is_admitted(wid) || (rekeyed.is_some() && state.windows.claim(wid).is_none());
    let outcome =
        EventOutcome::window_membership_changed(false, true).with_arrange_passes(arrange.into());
    Ok(if state.windows.can_reconcile_admission(wid) {
        outcome.with_created_window_finalization(wid)
    } else {
        outcome
    })
}

#[derive(Debug, Clone, Copy)]
pub struct WindowDestroyedPayload {
    pub window: WindowId,
}

pub fn handle_window_destroyed(
    state: &mut crate::model::RiftState,
    payload: WindowDestroyedPayload,
) -> EventOutcome {
    let wid = payload.window;
    if state.suspend_fullscreen_ax_identity(wid) {
        return EventOutcome::no_change().with_layout_event(LayoutEvent::WindowSuspended(wid));
    }
    // Retired at the reactor's edge (#702's retire_window): the store, the layout and every
    // tracker naming the window (the main window, a pending refocus) let go of it together.
    let mut outcome = EventOutcome::no_change();
    if state.windows.record(wid).is_some() {
        outcome.retirements.push(wid);
    }
    outcome
}

pub(crate) fn apply_window_retirement(
    tx: &WindowTxStore,
    (wid, wsid): (WindowId, Option<crate::sys::window_server::WindowServerId>),
) -> EventOutcome {
    if let Some(wsid) = wsid {
        tx.remove(&wsid);
    }
    EventOutcome::window_membership_changed(true, false)
        .with_arrange_passes(0)
        .with_layout_event(LayoutEvent::WindowRemoved(wid))
}

/// Application hiding and AX minimization share one suspension/return lifecycle (#702's
/// observe_window_minimization). Minimized is the core's admission fact; minimizing never
/// writes manageability.
pub fn handle_window_visibility_changed(
    state: &mut crate::model::RiftState,
    wid: WindowId,
    visible: bool,
) -> anyhow::Result<crate::actor::reactor::events::EventOutcome> {
    let minimized = !visible;
    // Application hiding can report minimization for a cached AX key.
    let ax_returned = !minimized && state.windows.validate_ax_identity(wid);
    let Some(window) = state.windows.window(wid) else {
        debug!(
            ?wid,
            minimized, "Minimization change for unknown window - ignoring"
        );
        return Ok(EventOutcome::no_change());
    };
    if !minimized && !window.info.is_minimized && !ax_returned {
        return Ok(EventOutcome::no_change());
    }
    state.windows.set_minimized(wid, minimized);
    // Minimizing orders the window out and restoring orders it back in: without native
    // visibility, inventory treats it as an AX-only fallback and skips it while other windows
    // of the app are visible.
    state.windows.set_minimized_visibility(wid, minimized);
    let _ = utils::refresh_heuristic(state, wid);
    if minimized {
        let mut outcome =
            EventOutcome::window_membership_changed(false, false).with_arrange_passes(0);
        outcome.memberships.push((wid, None));
        return Ok(outcome);
    }
    // Minimized AX snapshots can have stale admission metadata: inventory refreshes them.
    let mut outcome = EventOutcome::no_change().with_window_inventory_request(wid.pid);
    if state.windows.can_reconcile_admission(wid) {
        // Admission keeps the existing workspace and layout slot; a claimed window returns on
        // release. The restore transition owns the arrange pass.
        outcome = outcome
            .with_created_window_finalization(wid)
            .with_arrange_passes(state.windows.is_admitted(wid).into());
    }
    Ok(outcome)
}

#[derive(Debug)]
pub struct WindowFrameChangedPayload {
    pub window: WindowId,
    pub new_frame: CGRect,
    pub old_space: Option<SpaceId>,
    pub new_space: Option<SpaceId>,
    pub old_space_active: bool,
    pub new_space_active: bool,
    pub active_resize_space: Option<SpaceId>,
    pub pending_target_space: Option<SpaceId>,
    pub assigned_space: Option<SpaceId>,
    pub keep_assigned_for_scrolling: bool,
    pub screens: Vec<(SpaceId, CGRect, Option<String>)>,
}

pub fn handle_window_frame_changed(
    state: &mut crate::model::RiftState,
    layout: &mut crate::actor::reactor::managers::LayoutManager,
    payload: WindowFrameChangedPayload,
) -> anyhow::Result<EventOutcome> {
    let WindowFrameChangedPayload {
        window: wid,
        new_frame,
        old_space,
        new_space,
        old_space_active,
        new_space_active,
        active_resize_space,
        pending_target_space,
        assigned_space,
        keep_assigned_for_scrolling,
        screens,
    } = payload;
    let mut outcome = EventOutcome::default();
    let Some(window) = state.windows.window(wid) else {
        return Ok(outcome);
    };
    let server_id = window.info.sys_id;
    let old_frame = window.frame_monotonic();

    if !old_space_active && !new_space_active {
        return Ok(outcome);
    }
    if old_frame.same_as(new_frame) {
        return Ok(outcome);
    }
    state.step(CoreEvent::Seen { window: wid, frame: new_frame });
    // The external owner has geometry authority; keep only native facts current.
    if state.windows.is_claimed(wid) {
        if old_space != new_space
            && let Some(server) = server_id
        {
            match new_space {
                Some(space) => _ = state.windows.observe_native_space(server, space, true),
                None => state.windows.set_window_server_space(server, None),
            }
        }
        return Ok(EventOutcome::no_change());
    }
    // An adjusted acknowledgement of our own frame write is not a new native drag.
    if state.windows.drag_on(wid).is_some_and(|session| session.is_modifier()) {
        return Ok(EventOutcome::no_change());
    }
    outcome = EventOutcome::layout_changed(false);

    state.windows.clear_floating_display_geometry(wid);
    // External moves as well as resizes are authoritative for floating layouts.
    // Requested frame acknowledgements have already been filtered by the classifier.
    if let Some(space) = assigned_space.or(old_space)
        && Some(space) == new_space
        && let Some(workspace) =
            layout
                .layout_engine
                .workspaces()
                .workspace_for_window(&state.windows, space, wid)
        && layout.layout_engine.is_window_floating(wid)
    {
        layout.layout_engine.store_floating_position(workspace, wid, new_frame);
    }

    // This change isn't Rift's echo (the classifier filtered those). Whose it is depends on the
    // user's hand: on this window and not yet moved, it starts the drag; dragging it, the drag
    // follows; anywhere else, the app moved the window.
    let admitted = state.windows.is_admitted(wid);
    let tiled = !layout.layout_engine.is_window_floating(wid);
    let native_resize = !old_frame.size.same_as(new_frame.size);
    // A drag that starts here needs the drop scene, built before the hand is borrowed.
    let scene =
        (admitted && state.windows.pointer() == Pointer::Pressing(wid) && tiled && !native_resize)
            .then_some(new_space)
            .flatten()
            .map(|space| {
                crate::actor::reactor::events::drag::build_drag_scene(state, layout, wid, space)
            });
    let moved_by_user = admitted
        && match state.windows.pointer() {
            Pointer::Dragging(held) if held == wid => {
                if let Some(session) = state.windows.drag_on_mut(wid) {
                    session.update_native(new_frame);
                }
                true
            }
            Pointer::Pressing(held) if held == wid => {
                tracing::debug!(?wid, "drag-evidence: drag started (pressed window moved)");
                state.begin_drag(
                    wid,
                    Session::native(
                        DragSource {
                            origin_frame: old_frame,
                            last_frame: new_frame,
                            origin_space: old_space,
                            current_space: new_space,
                            tiled,
                        },
                        scene.unwrap_or_default(),
                    ),
                );
                // The start of a user drag is intent, even when no workspace changes.
                layout.layout_engine.forget_native_layout_for_window(&state.windows, wid);
                true
            }
            Pointer::Pressing(_) | Pointer::Dragging(_) | Pointer::Idle => false,
        };
    if moved_by_user {
        // The drag's start decided whether the user is resizing: an app changing its own size
        // mid-move resizes no tile.
        if state.windows.drag_on(wid).is_some_and(Session::resizing)
            && active_resize_space.is_some()
        {
            outcome = outcome.with_layout_event(LayoutEvent::WindowResized {
                wid,
                old_frame,
                new_frame,
                screens: screens.into(),
            });
        }
    } else {
        if old_space == new_space
            || keep_assigned_for_scrolling
            || (pending_target_space.is_some()
                && assigned_space == pending_target_space
                && new_space != pending_target_space)
        {
            return Ok(outcome);
        }
        if let Some(server) = server_id {
            if let Some(space) = new_space {
                state.windows.observe_native_space(server, space, true);
            } else {
                state.windows.set_window_server_space(server, None);
            }
        }
        // The Space the window is now on owns it, through the one reassignment path (#702).
        if let Some(space) = new_space.filter(|_| new_space_active) {
            outcome.topology_reassignments.push(
                crate::actor::reactor::events::TopologyReassignment {
                    window: wid,
                    space,
                    preserve_workspace_ordinal: false,
                },
            );
        }
    }

    Ok(outcome)
}

#[derive(Debug)]
pub struct WindowTitleChangedPayload {
    pub window: WindowId,
    pub title: String,
}

pub fn handle_window_title_changed(
    state: &mut crate::model::RiftState,
    payload: WindowTitleChangedPayload,
) -> anyhow::Result<crate::actor::reactor::events::EventOutcome> {
    let WindowTitleChangedPayload { window: wid, title: new_title } = payload;
    if let Some(window) = state.windows.window_mut(wid) {
        let previous_title = window.info.title.clone();
        if previous_title == new_title {
            return Ok(crate::actor::reactor::events::EventOutcome::no_change());
        }
        window.info.title = new_title.clone();
        let mut outcome = crate::actor::reactor::events::EventOutcome::no_change()
            .with_window_title_broadcast(wid, previous_title, new_title);
        outcome.reapply_app_rules.push(wid);
        return Ok(outcome);
    }
    Ok(crate::actor::reactor::events::EventOutcome::no_change())
}

#[derive(Debug, Clone, Copy)]
pub struct MouseMovedPayload {
    pub window: Option<WindowId>,
    pub should_sync: bool,
    pub is_main: bool,
    pub needs_layout_sync: bool,
    pub active_space: Option<SpaceId>,
}

pub fn handle_mouse_moved_over_window(
    apps: &crate::actor::reactor::managers::AppManager,
    payload: MouseMovedPayload,
    mouse_token: &tokio_util::sync::CancellationToken,
) -> anyhow::Result<crate::actor::reactor::events::EventOutcome> {
    let Some(window) = payload.window else {
        return Ok(crate::actor::reactor::events::EventOutcome::default());
    };
    if !payload.should_sync || (payload.is_main && !payload.needs_layout_sync) {
        return Ok(crate::actor::reactor::events::EventOutcome::default());
    }

    let mut outcome = crate::actor::reactor::events::EventOutcome::default();
    if !payload.is_main {
        let mut app_handles = crate::common::collections::HashMap::default();
        if let Some(app) = apps.apps.get(&window.pid) {
            app_handles.insert(window.pid, app.handle.clone());
        }
        outcome =
            outcome.with_raise_request(crate::actor::raise_manager::Event::MouseRaiseRequest(
                crate::actor::raise_manager::RaiseRequest {
                    raise_windows: vec![vec![window]],
                    focus_window: Some((window, None)),
                    app_handles,
                    focus_quiet: Quiet::Yes,
                },
                mouse_token.clone(),
            ));
    }
    if let Some(space) = payload.active_space {
        outcome = outcome.with_layout_event(LayoutEvent::WindowFocused(space, window));
    }
    Ok(outcome)
}
