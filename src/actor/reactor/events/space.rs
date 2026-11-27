use rift_core::topology::{Displays, Incoming};
use tracing::{debug, trace};

use crate::actor::reactor::events::EventOutcome;
use crate::actor::reactor::managers::MissionControlManager;
use crate::actor::reactor::{MissionControlState, SpaceEventKind};
use crate::actor::spaces::ForwardedSpaceState;
use crate::actor::wm_controller::WmEvent;
use crate::common::collections::HashSet;
use crate::model::RiftState;
use crate::model::space_activation::{SpaceActivationConfig, SpaceActivationPolicy};
use crate::sys::app::AppInfo;
use crate::sys::screen::SpaceId;
use crate::sys::window_server::WindowServerId;

#[derive(Debug)]
pub(crate) struct SpaceSnapshotAnalysis {
    pub(crate) spaces: Vec<Option<SpaceId>>,
    pub(crate) authoritative_spaces: Vec<Option<SpaceId>>,
    pub(crate) command_space_only_update: bool,
    pub(crate) invalidates_pending_targets: bool,
}

pub(crate) fn analyze_space_snapshot(
    current: &ForwardedSpaceState,
    current_effective_active_spaces: &HashSet<SpaceId>,
    activation_policy: &SpaceActivationPolicy,
    activation_config: SpaceActivationConfig,
    incoming: &ForwardedSpaceState,
) -> SpaceSnapshotAnalysis {
    let active_window_membership_changed =
        current.active_window_spaces != incoming.active_window_spaces;
    let spaces = incoming.screens.iter().map(|screen| screen.space).collect();
    let display_uuids: Vec<Option<String>> =
        incoming.screens.iter().map(|screen| screen.display_uuid_owned()).collect();
    let authoritative_spaces: Vec<Option<SpaceId>> = incoming
        .screens
        .iter()
        .map(|screen| screen.space.filter(|space| incoming.active_spaces.contains(space)))
        .collect();
    let effective_active_spaces = activation_policy
        .compute_active_spaces(activation_config, &authoritative_spaces, &display_uuids)
        .into_iter()
        .flatten()
        .collect();
    let command_space_only_update = !incoming.display_set_changed
        && !incoming.should_force_refresh_layout
        && incoming.space_remaps.is_empty()
        && incoming.resized_spaces.is_empty()
        && current.screens == incoming.screens
        && current.fullscreen_spaces == incoming.fullscreen_spaces
        && current_effective_active_spaces == &effective_active_spaces
        && current.display_space_ids == incoming.display_space_ids
        && current.membership_complete == incoming.membership_complete
        && incoming.destroyed_spaces.is_empty()
        && !active_window_membership_changed;
    let invalidates_pending_targets = incoming.display_set_changed
        || incoming.should_force_refresh_layout
        || !incoming.space_remaps.is_empty()
        || !incoming.resized_spaces.is_empty();
    SpaceSnapshotAnalysis {
        spaces,
        authoritative_spaces,
        command_space_only_update,
        invalidates_pending_targets,
    }
}

/// A snapshot's displays as admission reads them: each screen's display id (none when
/// empty), each display's Spaces.
pub(crate) fn displays(state: &ForwardedSpaceState) -> Displays<&str, SpaceId> {
    Displays {
        screens: state.screens.iter().map(|screen| screen.display_uuid_opt()).collect(),
        spaces: state
            .display_space_ids
            .iter()
            .map(|(display, spaces)| (display.as_str(), spaces.clone()))
            .collect(),
    }
}

/// A snapshot being admitted, as admission reads it.
pub(crate) fn incoming(state: &ForwardedSpaceState) -> Incoming<&str, SpaceId> {
    Incoming {
        displays: displays(state),
        remaps: state.space_remaps.clone(),
        display_set_changed: state.display_set_changed,
        force_refresh: state.should_force_refresh_layout,
        membership_complete: state.membership_complete,
    }
}

/// An admission as the Roc core's question: Spaces as their words, each display as the place
/// it first appears in this question (the answer names only Spaces, so a display needs only
/// to be told apart from the others).
#[cfg(feature = "roc-core")]
pub(crate) fn plain_admit(
    current: &Displays<&str, SpaceId>,
    incoming: &Incoming<&str, SpaceId>,
    mission_control: rift_core::topology::MissionControl,
) -> rift_core::topology::plain::Admit {
    use crate::roc_path::Word;
    /// The place `display` first appears among `names`.
    fn name(names: &mut Vec<String>, display: &str) -> u64 {
        match names.iter().position(|named| named == display) {
            Some(place) => place as u64,
            None => {
                names.push(display.to_owned());
                (names.len() - 1) as u64
            }
        }
    }
    let mut names = Vec::new();
    let mut plain = |displays: &Displays<&str, SpaceId>| Displays {
        screens: displays
            .screens
            .iter()
            .map(|screen| screen.map(|display| name(&mut names, display)))
            .collect(),
        spaces: displays
            .spaces
            .iter()
            .map(|(display, spaces)| {
                (
                    name(&mut names, display),
                    spaces.iter().map(|&space| space.word()).collect(),
                )
            })
            .collect(),
    };
    let current = plain(current);
    let displays = plain(&incoming.displays);
    rift_core::topology::plain::Admit {
        mission_control,
        current,
        incoming: Incoming {
            displays,
            remaps: incoming.remaps.iter().map(|&(old, new)| (old.word(), new.word())).collect(),
            display_set_changed: incoming.display_set_changed,
            force_refresh: incoming.force_refresh,
            membership_complete: incoming.membership_complete,
        },
    }
}

/// Prepends an older sample's deltas to a newer one; geometry and membership stay the newer's.
pub(crate) fn merge_space_deltas(
    mut older: ForwardedSpaceState,
    mut newer: ForwardedSpaceState,
) -> ForwardedSpaceState {
    older.space_remaps.retain(|remap| !newer.space_remaps.contains(remap));
    older.space_remaps.append(&mut newer.space_remaps);
    newer.space_remaps = older.space_remaps;
    older
        .resized_spaces
        .retain(|(space, _)| !newer.resized_spaces.iter().any(|(next, _)| space == next));
    older.resized_spaces.append(&mut newer.resized_spaces);
    newer.resized_spaces = older.resized_spaces;
    newer.destroyed_spaces.extend(older.destroyed_spaces);
    newer.should_force_refresh_layout |= older.should_force_refresh_layout;
    newer.display_set_changed |= older.display_set_changed;
    newer
}

// spacewindowappeared/destroyed happen a lot when a display is connected/disconnected
// since they are literally when a window enters or leaves a space and each display has its own space(s)
// this is functionally a connection dropping to the window server
#[derive(Debug, Clone, Copy)]
pub struct WindowServerLifecyclePayload {
    pub window_server_id: WindowServerId,
    pub space: SpaceId,
    pub kind: SpaceEventKind,
}

#[derive(Debug)]
pub struct WindowServerDestroyedObservations {
    pub resolved_space: Option<SpaceId>,
    pub active_spaces: HashSet<SpaceId>,
    pub ordered_in: Option<bool>,
    pub suitable: Option<bool>,
}

#[derive(Debug)]
pub struct WindowServerAppearedObservations {
    pub resolved_space: Option<SpaceId>,
    pub active_spaces: HashSet<SpaceId>,
    pub mission_control_active: bool,
    pub last_known_user_space: Option<SpaceId>,
    pub window_server_info: Option<crate::sys::window_server::WindowServerInfo>,
    pub app_known: bool,
    pub running_app_info: Option<AppInfo>,
}

pub fn handle_window_server_destroyed(
    state: &mut RiftState,
    payload: WindowServerLifecyclePayload,
    observations: WindowServerDestroyedObservations,
) -> anyhow::Result<EventOutcome> {
    let WindowServerLifecyclePayload {
        window_server_id: wsid,
        space: sid,
        kind,
    } = payload;
    let WindowServerDestroyedObservations {
        resolved_space,
        active_spaces,
        ordered_in,
        suitable,
    } = observations;
    let mut outcome = EventOutcome::default();
    if matches!(kind, SpaceEventKind::Fullscreen) {
        // Departure alone cannot establish entry. Startup recovery uses positive
        // fullscreen appearance/discovery, even if this notification arrives late.
        if let Some(wid) = state.windows.tracked_window_id(wsid) {
            outcome = outcome.with_window_inventory_request(wid.pid);
        }

        return Ok(outcome);
    } else if matches!(kind, SpaceEventKind::User) {
        match state.windows.observe_native_departure(
            wsid,
            sid,
            resolved_space,
            &active_spaces,
            ordered_in,
            suitable,
        ) {
            crate::model::window_store::NativeDeparture::Moved(window, space) => {
                if let Some(wid) = window {
                    outcome = outcome.with_topology_reassignment(wid, space, false);
                }
            }
            crate::model::window_store::NativeDeparture::Closed(wid) => {
                outcome.retirements.push(wid);
            }
            crate::model::window_store::NativeDeparture::Hidden(window) => {
                if let Some(wid) = window {
                    outcome.memberships.push((wid, None));
                    outcome = outcome.with_window_inventory_request(wid.pid);
                }
            }
        }
        return Ok(outcome);
    }
    Ok(outcome)
}

pub fn handle_window_server_appeared(
    state: &mut RiftState,
    payload: WindowServerLifecyclePayload,
    observations: WindowServerAppearedObservations,
) -> anyhow::Result<EventOutcome> {
    let WindowServerLifecyclePayload {
        window_server_id: wsid,
        space: sid,
        kind,
    } = payload;
    let WindowServerAppearedObservations {
        resolved_space,
        active_spaces,
        mission_control_active,
        last_known_user_space,
        window_server_info,
        app_known,
        running_app_info,
    } = observations;
    let mut outcome = EventOutcome::default();
    if matches!(kind, SpaceEventKind::User) {
        if let Some(resolved_space) = resolved_space {
            if resolved_space != sid {
                state.windows.observe_native_space(
                    wsid,
                    resolved_space,
                    active_spaces.contains(&resolved_space),
                );
                if let Some(wid) = state.windows.tracked_window_id(wsid) {
                    outcome = outcome.with_topology_reassignment(wid, resolved_space, false);
                }
                debug!(
                    ?wsid,
                    reported_space = ?sid,
                    resolved_space = ?resolved_space,
                    "Resolved user-space appearance to stronger native membership"
                );
                return Ok(outcome);
            }

            state.windows.observe_native_space(wsid, resolved_space, true);
            state.windows.note_window_server_fullscreen_exit(wsid, resolved_space);
            outcome.confirmed_window_spaces.push((wsid, resolved_space));
        }
    }

    let tracked = state.windows.tracked_window_id(wsid);
    let known = tracked.is_some()
        || state.windows.knows_window_server_id(wsid)
        || state.windows.is_window_server_observed(wsid);
    if known && mission_control_active {
        return Ok(outcome);
    }
    if !known {
        state.windows.mark_window_server_observed(wsid);
        // Layered or tiny windows (tooltips and similar chrome) are never managed.
        const MIN_MANAGEABLE_WINDOW_SIZE: f64 = 50.0;
        if let Some(info) = window_server_info
            && (info.layer != 0
                || info.frame.size.width < MIN_MANAGEABLE_WINDOW_SIZE
                || info.frame.size.height < MIN_MANAGEABLE_WINDOW_SIZE)
        {
            state.windows.clear_window_server_observed(wsid);
            trace!(?wsid, layer = info.layer, "Ignoring non-normal or tiny window");
            return Ok(outcome);
        }
    }

    if matches!(kind, SpaceEventKind::Fullscreen) {
        let pid = tracked.map(|wid| wid.pid).or_else(|| {
            state
                .windows
                .get_window_server_info(wsid)
                .or(window_server_info)
                .map(|info| info.pid)
        });
        if pid.is_none() {
            return Ok(outcome);
        }
        if let Some(wid) =
            state
                .windows_mut()
                .observe_native_fullscreen(wsid, sid, last_known_user_space, pid)
        {
            outcome.memberships.push((wid, None));
        }
        if tracked.is_none()
            && let Some(pid) = pid
        {
            outcome = outcome.with_window_inventory_request(pid);
        }
        return Ok(outcome);
    }
    if let Some(wid) = tracked {
        if state.windows.native_fullscreen_record_for_window(wid).is_some() {
            outcome = outcome.with_window_inventory_request(wid.pid);
        }
        outcome.fullscreen_restorations.push((wsid, sid, wid));
    } else if known {
        if let Some(pid) = state.windows.pending_native_fullscreen_pid_for_window_server_id(wsid) {
            outcome = outcome.with_window_inventory_request(pid);
        }
    } else if let Some(window_server_info) = window_server_info {
        outcome = outcome.with_window_server_updates(vec![window_server_info]);

        if !app_known {
            if let Some(app_info) = running_app_info {
                outcome.wm_events.push(WmEvent::AppLaunch(
                    window_server_info.pid,
                    app_info,
                    crate::actor::wm_controller::AppDiscoverySource::WindowServer,
                ));
            }
        } else {
            outcome = outcome.with_window_inventory_request(window_server_info.pid);
        }
    }
    Ok(outcome)
}

pub fn handle_mission_control_native_entered(
    mission_control: &mut MissionControlManager,
    state: &mut crate::model::RiftState,
) -> anyhow::Result<EventOutcome> {
    state.let_go();
    mission_control.mission_control_state = MissionControlState::Active;
    Ok(EventOutcome::focus_changed(None, false))
}

pub fn handle_mission_control_native_exited(
    mission_control: &mut MissionControlManager,
) -> anyhow::Result<EventOutcome> {
    mission_control.mission_control_state = MissionControlState::Inactive;
    let mut outcome = EventOutcome::layout_changed();
    outcome.recover_after_mission_control = true;
    Ok(outcome)
}
