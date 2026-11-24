use tracing::{debug, warn};

use super::window;
use crate::actor::app::{AppInfo, WindowId, WindowInfo, pid_t};
use crate::actor::reactor::{LayoutEvent, WindowState, utils};
use crate::common::collections::{HashMap, HashSet};
use crate::layout_engine::ResolvedWindow;
use crate::model::virtual_workspace::WorkspaceError;
use crate::model::{AppRuleEffects, AppRuleResult, WindowRuleContext};
use crate::sys::screen::SpaceId;

/// Handler for window discovery events, responsible for processing newly discovered windows
/// and managing the lifecycle of window state in the reactor.
fn sync_existing_window_state(
    state: &mut crate::model::RiftState,
    wid: WindowId,
    mut info: WindowInfo,
    as_of: Option<crate::actor::reactor::transaction_manager::TransactionId>,
) -> anyhow::Result<(crate::actor::reactor::events::EventOutcome, bool)> {
    let revalidation = state.windows.validate_ax_identity(wid);
    let ax_returned = revalidation == rift_core::lifecycle::Revalidation::Revalidated;
    let was_minimized = state.windows.window(wid).is_some_and(|window| window.info.is_minimized);

    let is_minimized = info.is_minimized;
    let claimed = state.windows().is_claimed(wid);
    let frame = info.frame;
    let Some(existing) = state.windows.window(wid) else {
        return Ok((crate::actor::reactor::events::EventOutcome::default(), false));
    };
    let constraints_changed = existing.info.is_resizable != info.is_resizable
        || existing.info.min_size != info.min_size
        || existing.info.max_size != info.max_size;
    // Preserve the observed frame and minimize transition until their handlers run.
    info.frame = existing.info.frame;
    info.is_minimized = was_minimized;
    let reading = state.windows.replace_info(wid, info);
    // Where the inventory read it: a read like any other, news only if the core says so (a
    // read from before the app applied Rift's latest word is not).
    let valid_frame = frame.size.width != 0.0 || frame.size.height != 0.0;
    let (geometry_changed, released) = if valid_frame {
        let (reply, released) = state.read(wid, frame, as_of);
        (
            reply == crate::actor::reactor::reconciler::Reply::Foreign,
            released,
        )
    } else {
        (false, None)
    };
    // A claimed window's owner has geometry authority.
    let needs_arrange = (geometry_changed || constraints_changed) && !claimed;

    let mut admission_changed = false;
    let mut admitted_now = false;
    let mut outcome = match (was_minimized, is_minimized) {
        (_, true) | (true, false) => {
            let mut outcome = window::handle_window_visibility_changed(state, wid, !is_minimized)?;
            // Discovery admits a returning window itself (#702): its observation, not the
            // minimization handler, puts it back.
            outcome.finalize_created_windows.retain(|window| *window != wid);
            outcome
        }
        _ => {
            // The fresh reading's roles and the heuristic, as one change to its admission.
            let heuristic = utils::refresh_heuristic(state, wid);
            let change = rift_core::lifecycle::net(reading, heuristic);
            #[cfg(feature = "roc-core")]
            {
                use crate::roc_path::window_free;
                let question = (window_free(reading), window_free(heuristic));
                let roc = Ok(rift_core_roc::lifecycle::net(question.0, question.1));
                crate::roc_path::check("net", &[], &question, &window_free(change), roc);
            }
            admission_changed = change.is_some();
            admitted_now = matches!(change, Some(rift_core::lifecycle::Transition::Admitted(_)));
            if let Some(rift_core::lifecycle::Transition::Suspended(..)) = change {
                crate::actor::reactor::events::EventOutcome::default()
                    .with_layout_event(utils::layout_departure(state, wid))
            } else {
                // A window that came back (admitted, restored or revalidated) is admitted by
                // discovery's observation, even on an inactive workspace (#702's resumed).
                crate::actor::reactor::events::EventOutcome::default()
            }
        }
    };
    if (needs_arrange || admission_changed || ax_returned)
        && state.windows().is_admitted(wid)
        && was_minimized == is_minimized
    {
        outcome.absorb(crate::actor::reactor::events::EventOutcome::layout_changed(
            false,
        ));
    }
    outcome.released.extend(released);

    if was_minimized != is_minimized {
        debug!(
            ?wid,
            was_minimized,
            is_minimized = is_minimized,
            "Window minimize state reconciled from discovery"
        );
    }
    let restored = was_minimized && !is_minimized;
    let resumed = !is_minimized && !claimed && (restored || ax_returned || admitted_now);
    Ok((outcome, resumed))
}

fn should_emit_window_for_space(
    state: &crate::model::RiftState,
    layout: &crate::actor::reactor::managers::LayoutManager,
    space: SpaceId,
    wid: WindowId,
) -> bool {
    let workspaces = layout.layout_engine.workspaces();
    let assigned = workspaces.reading(state.windows()).workspace_for_window(space, wid);
    let active = crate::roc_path::Asked::new();
    let emits = rift_core::workspaces::emits_window(assigned, || {
        active.ask(|| workspaces.reading(state.windows()).active_workspace(space))
    });
    #[cfg(feature = "roc-core")]
    {
        use rift_core::workspaces::plain::EmitsQuestion;

        use crate::roc_path::Word;
        let question = EmitsQuestion {
            assigned: assigned.map(Word::word),
            active: active.fact(|active| active.map(Word::word)),
        };
        let roc = rift_core_roc::workspaces::emits_window(question);
        crate::roc_path::check("emits_window", EmitsQuestion::FACTS, &question, &emits, roc);
    }
    emits
}

/// Process new and updated windows, returning lists of new and updated windows.
#[derive(Debug)]
pub(crate) struct ObservedWindow {
    pub(crate) wid: WindowId,
    pub(crate) info: WindowInfo,
    /// The newest request the app had applied to the window when it read it.
    pub(crate) as_of: Option<crate::actor::reactor::transaction_manager::TransactionId>,
}

pub(crate) fn process_window_list(
    state: &mut crate::model::RiftState,
    layout: &mut crate::actor::reactor::managers::LayoutManager,
    observed: Vec<ObservedWindow>,
) -> (
    Vec<(WindowId, WindowInfo)>,
    HashSet<WindowId>,
    crate::actor::reactor::events::EventOutcome,
) {
    let mut new_windows = Vec::new();
    let mut resumed = HashSet::default();
    let mut outcome = crate::actor::reactor::events::EventOutcome::default();

    for window in observed {
        let ObservedWindow { wid, info, as_of } = window;
        let previous = state.reconcile_ax_identity(wid, info.sys_id);
        if let Some(previous) = previous {
            layout.layout_engine.transfer_persistent_window_identity(previous, wid);
            outcome =
                outcome.with_layout_event(LayoutEvent::WindowRemovedPreserveFloating(previous));
            // Anything pending for the old identity died with it: arrange, so the core writes
            // the desire again for the new one (an outside owner's window is never arranged).
            if state.windows().claim(wid).is_none() {
                let arrange = crate::actor::reactor::events::EventOutcome::layout_changed(false);
                outcome.absorb(arrange);
            }
        }
        if state.windows.contains_window(wid) {
            let minimized = info.is_minimized;
            if let Ok((existing_outcome, came_back)) =
                sync_existing_window_state(state, wid, info, as_of)
            {
                outcome.absorb(existing_outcome);
                if came_back || (previous.is_some() && !minimized) {
                    resumed.insert(wid);
                }
            }
        } else {
            new_windows.push((wid, info));
        }
    }

    (new_windows, resumed, outcome)
}

/// Inserts the newly discovered window snapshots into domain state.
pub(crate) fn update_window_states(
    rift_state: &mut crate::model::RiftState,
    new_windows: Vec<(WindowId, WindowInfo)>,
) {
    // Update or insert window states
    for (wid, info) in new_windows {
        let state: WindowState = info.into();
        rift_state.insert_window(wid, state);
        let _ = utils::refresh_heuristic(rift_state, wid);
    }
}

/// Assignment and rejection handling shared by discovery and rule reapplication.
/// Callers decide whether successful, unchanged membership needs a layout refresh.
pub(crate) fn assign_window<'a>(
    state: &mut crate::model::RiftState,
    layout: &mut crate::actor::reactor::managers::LayoutManager,
    wid: WindowId,
    context: impl Into<crate::model::virtual_workspace::WorkspaceContext<'a>>,
    app_info: Option<&AppInfo>,
    reapply: bool,
) -> (Option<AppRuleEffects>, Option<LayoutEvent>) {
    let context = context.into();
    let space = context.space;
    let result = if let Some(window) = state.windows.window(wid) {
        let (title, role, subrole) = (
            window.info.title.clone(),
            window.info.ax_role.clone(),
            window.info.ax_subrole.clone(),
        );
        layout.layout_engine.assign_window_with_app_info(
            &mut state.windows_mut(),
            wid,
            context,
            WindowRuleContext {
                app_bundle_id: app_info.and_then(|app| app.bundle_id.as_deref()),
                app_name: app_info.and_then(|app| app.localized_name.as_deref()),
                window_title: Some(&title),
                ax_role: role.as_deref(),
                ax_subrole: subrole.as_deref(),
                window_tags: None,
            },
            reapply,
        )
    } else {
        Err(WorkspaceError::AssignmentFailed)
    };
    match result {
        Ok(None) => (None, None),
        Ok(Some(AppRuleResult::Managed(effects))) => (Some(effects), None),
        Ok(Some(AppRuleResult::Rejected(_))) => (
            None,
            utils::rejection_needs_removal(state, layout, wid, space)
                .then_some(LayoutEvent::WindowRemoved(wid)),
        ),
        Err(error) => {
            warn!(?wid, ?error, "Failed to assign window to workspace");
            utils::clear_rule_admission(state, wid);
            (None, None)
        }
    }
}

pub(crate) struct EmitLayoutPayload<'a> {
    pub(crate) pid: pid_t,
    pub(crate) known_visible: &'a [WindowId],
    /// Windows this inventory found coming back: observed even on an inactive workspace.
    pub(crate) resumed: &'a HashSet<WindowId>,
    pub(crate) app_info: &'a Option<AppInfo>,
    // Authoritative ownership and location allowing geometry fallback, respectively.
    pub(crate) window_spaces: HashMap<WindowId, (Option<SpaceId>, Option<SpaceId>)>,
    pub(crate) active_spaces: Vec<SpaceId>,
    pub(crate) focused_window: Option<(SpaceId, WindowId)>,
}

pub(crate) fn emit_layout_events(
    state: &mut crate::model::RiftState,
    layout: &mut crate::actor::reactor::managers::LayoutManager,
    payload: EmitLayoutPayload<'_>,
) -> crate::actor::reactor::events::EventOutcome {
    let EmitLayoutPayload {
        pid,
        known_visible,
        resumed,
        app_info,
        window_spaces,
        active_spaces,
        focused_window,
    } = payload;
    let mut outcome = crate::actor::reactor::events::EventOutcome::default();
    if state.windows.window_ids_for_pid(pid).next().is_none() {
        return outcome;
    }

    // Which windows this inventory observes (#702): a minimized window still matches its saved
    // slot; otherwise one rift may reconcile, natively visible (or reported by AX when nothing of
    // the app is natively visible, or on an active Space), on its workspace's active slot. A
    // window that came back is observed regardless.
    let native_visible = |wid: WindowId| {
        state
            .windows()
            .record(wid)
            .and_then(|record| record.window_server_id())
            .is_some_and(|wsid| state.windows.is_window_visible(wsid))
    };
    let has_visible = state
        .windows
        .window_ids_for_pid(pid)
        .any(|wid| native_visible(wid) && state.windows().can_reconcile_admission(wid));
    let known_visible: HashSet<_> = known_visible.iter().copied().collect();
    let mut observations: Vec<(SpaceId, WindowId)> = state
        .windows
        .window_ids_for_pid(pid)
        .filter_map(|wid| {
            let (native, discovery) = *window_spaces.get(&wid)?;
            let window = state.windows.window(wid)?;
            if window.info.is_minimized && !state.windows().is_claimed(wid) {
                return Some((native.or(discovery)?, wid));
            }
            if !state.windows().can_reconcile_admission(wid) {
                return None;
            }
            let space = discovery?;
            if !resumed.contains(&wid)
                && (!should_emit_window_for_space(state, layout, space, wid)
                    || !(native_visible(wid)
                        || (known_visible.contains(&wid)
                            && (!has_visible
                                || native.is_some_and(|s| active_spaces.contains(&s))))))
            {
                return None;
            }
            Some((space, wid))
        })
        .collect();
    observations.sort_unstable();
    for (space, wid) in observations {
        let active = active_spaces.contains(&space);
        let assigned_space = state.windows().workspace_for_window(wid).and_then(|workspace| {
            layout
                .layout_engine
                .workspaces()
                .reading(state.windows())
                .workspace_space(workspace)
        });
        if !active && resumed.contains(&wid) && assigned_space == Some(space) {
            // Back on its own, inactive workspace: it rejoins its slot there.
            outcome.memberships.push((wid, Some(space)));
            continue;
        }
        let (effects, removal) = assign_window(
            state,
            layout,
            wid,
            crate::model::virtual_workspace::WorkspaceContext { space, visible: &active_spaces },
            app_info.as_ref(),
            false,
        );
        if removal.is_some() {
            // A rule rejects it: its membership follows its record (#702).
            outcome.memberships.push((wid, None));
        }
        let Some(effects) = effects else {
            continue;
        };
        if active && let Some(window) = state.windows.window(wid) {
            outcome =
                outcome.with_layout_event(LayoutEvent::WindowObserved(space, ResolvedWindow {
                    info: window.layout_info(wid),
                    effects,
                }));
        }
    }

    // D4 (pending the lead): upstream #700 no longer discards unmatched persisted windows when
    // an app's discovery completes (LayoutEvent::WindowDiscoveryCompleted is gone); provisionally
    // upstream's.
    let _ = (&app_info, &active_spaces);

    if let Some((space, main_window)) = focused_window {
        outcome = outcome.with_layout_event(LayoutEvent::WindowFocused(space, main_window));
    }
    outcome
}
