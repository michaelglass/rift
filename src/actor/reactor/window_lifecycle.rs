//! A window's admission and membership as the reactor acts on them: upstream's module
//! (#702), with its names, over our state. Admission facts, presence and ownership are the
//! core's (`rift_core::lifecycle`, `rift_core::workspaces`, through the window store); these
//! methods are the reactor's edge, applying what the store decided to the layout and to the
//! other trackers. Handlers that only read events (events/*) describe this work in their
//! outcome (`memberships`, `retirements`, `topology_reassignments`), and
//! `Reactor::apply_event_outcome` calls in here.

use tracing::debug;

use super::{EventOutcome, Reactor, RefocusState};
use crate::actor::app::{WindowId, pid_t};
use crate::layout_engine::LayoutEvent;
use crate::model::window_store::NativeFullscreenTransition;
use crate::sys::screen::SpaceId;
use crate::sys::window_server::WindowServerId;

impl Reactor {
    /// Reconcile layout participation from the canonical window record. A placement is
    /// supplied only by evidence that can put a window in layout.
    pub(super) fn reconcile_window_membership(
        &mut self,
        wid: WindowId,
        placement: Option<SpaceId>,
    ) {
        let windows = self.stores.state.windows();
        let event = if windows.is_claimed(wid) {
            return;
        } else if windows.rule(wid) == Some(rift_core::lifecycle::Rule::Ignore) {
            if windows.workspace_for_window(wid).is_none()
                && !self.stores.layout.layout_engine.is_window_floating(wid)
            {
                return;
            }
            LayoutEvent::WindowRemoved(wid)
        } else if windows.is_temporarily_unavailable(wid) {
            LayoutEvent::WindowSuspended(wid)
        } else if let Some(space) = placement.filter(|_| windows.is_admitted(wid)) {
            LayoutEvent::WindowAdded(space, wid)
        } else {
            return;
        };
        self.send_layout_event(event);
    }

    /// The window is gone for good: out of every tracker, then out of the store (through the
    /// state, so a grip on it ends through the core).
    pub(super) fn retire_window(&mut self, wid: WindowId) {
        if self.stores.state.windows().record(wid).is_none() {
            return;
        }
        self.stores.state.windows.restore_window_from_native_fullscreen(wid);
        let Some(record) = self.stores.state.remove_window(wid) else {
            return;
        };
        let wsid = record.window_server_id();
        self.discard_window_activity(wid, wsid);
        self.replace_window_references(wid, None);
        let retired =
            super::events::window::apply_window_retirement(&self.stores.state.tx, (wid, wsid));
        self.apply_event_outcome(retired);
    }

    fn discard_window_activity(&mut self, _wid: WindowId, obsolete_native: Option<WindowServerId>) {
        if let Some(wsid) = obsolete_native {
            self.transient_focus.windows.remove(&wsid);
            self.stores.state.tx.remove(&wsid);
        }
    }

    /// The same window under a new identity: the store carries its record (ownership, rule,
    /// invalidation) and the layout its slot; every other tracker follows here.
    pub(super) fn replace_window_identity(
        &mut self,
        old: WindowId,
        replacement: WindowId,
        obsolete_native: Option<WindowServerId>,
    ) {
        self.discard_window_activity(old, obsolete_native);
        self.stores
            .layout
            .layout_engine
            .transfer_persistent_window_identity(old, replacement);
        self.replace_window_references(old, Some(replacement));
    }

    fn replace_window_references(&mut self, old: WindowId, replacement: Option<WindowId>) {
        self.main_window_tracker.replace_window(old, replacement);
        if let Some(focus) = self.overview_focus.take() {
            self.overview_focus = if focus.peek().1 != old {
                Some(focus)
            } else {
                replacement.map(|window| focus.map(|(space, _)| (space, window)))
            };
        }
        let pending = &mut self.window_inventory_manager.refocus_after_refresh;
        if pending.get(&old.pid) == Some(&old) {
            match replacement {
                Some(window) => {
                    pending.insert(window.pid, window);
                }
                None => {
                    pending.remove(&old.pid);
                }
            }
        }
    }

    /// Every window of an app that quit.
    pub(super) fn retire_app_windows(&mut self, pid: pid_t) {
        let windows: Vec<_> = self.stores.state.windows.window_ids_for_pid(pid).collect();
        for &wid in &windows {
            let wsid = self.stores.state.windows().record(wid).and_then(|r| r.window_server_id());
            self.discard_window_activity(wid, wsid);
            self.replace_window_references(wid, None);
        }
        self.transient_focus.windows.retain(|_, transient| transient.pid != pid);
        self.stores.state.remove_windows_for_app(pid);
        self.send_layout_event(LayoutEvent::AppClosed(pid));
    }

    /// The window's AX element was invalidated: it keeps its record and slot until fresh AX
    /// evidence or confirmed retirement, and sits out of layout meanwhile.
    pub(super) fn invalidate_window_identity(&mut self, wid: WindowId) -> EventOutcome {
        let mut outcome = EventOutcome::no_change().with_window_inventory_request(wid.pid);
        match self.stores.state.windows.invalidate_ax_identity(wid) {
            rift_core::lifecycle::Invalidation::Invalidated => {
                outcome.memberships.push((wid, None));
                outcome.refresh_window_notifications = true;
            }
            // No AX state: the window is in no layout and has no notifications to refresh.
            rift_core::lifecycle::Invalidation::NoAxState => {}
        }
        outcome
    }

    /// Leave fullscreen with the logical workspace intact; placement follows its current owner
    /// through the transfer lifecycle (deferred while dormant), never the Space macOS returned
    /// the window to.
    pub(super) fn restore_native_fullscreen(
        &mut self,
        wsid: Option<WindowServerId>,
        window: WindowId,
        observed: Option<SpaceId>,
    ) -> EventOutcome {
        if let Some(space) = observed {
            self.stores.state.windows.note_fullscreen_exit(window, space);
        }
        let Some(restored) = self.stores.state.windows().restore_native_identity(wsid, window)
        else {
            return EventOutcome::no_change();
        };
        let mut outcome = EventOutcome::no_change();
        if let Some(previous) = restored.previous_assignment {
            outcome = outcome.with_layout_event(LayoutEvent::WindowRemoved(previous));
        }
        let Some(window) = restored.window else {
            outcome.memberships.push((restored.record.current_window_id, None));
            return outcome;
        };
        // Readiness is checked without rejecting logical ownership. A temporary admission
        // failure keeps both the lifecycle and its preserved layout slot.
        let native = observed.or(match restored.record.transition {
            NativeFullscreenTransition::ExitObserved(space) => space,
            _ => None,
        });
        let engine = &mut self.stores.layout.layout_engine;
        let Some((workspace, target)) = (self.stores.state.windows().workspace_for_window(window))
            .or(restored.record.workspace)
            .and_then(|id| {
                Some((
                    id,
                    engine.workspaces().reading(self.stores.state.windows()).workspace_space(id)?,
                ))
            })
        else {
            let Some(space) = native else { return outcome };
            self.stores.state.windows.restore_window_from_native_fullscreen(window);
            self.reassign_window_to_authoritative_space(window, space, false);
            return outcome;
        };
        engine.workspaces_mut().assign_window_to_workspace(
            &mut self.stores.state.windows_mut(),
            target,
            window,
            workspace,
        );
        // Ownership is recovered before releasing suspension to the existing placement and
        // projection paths. History never confirms physical placement.
        self.stores.state.windows.restore_window_from_native_fullscreen(window);
        let available = self.is_space_active(target);
        if native != Some(target) {
            outcome.absorb(self.request_window_transfer(window, target, None));
        }
        // Rejoin the tree even while the owner is dormant; arrange only a visible one.
        outcome.memberships.push((window, Some(target)));
        if available {
            self.refocus_manager.refocus_state = RefocusState::Pending(target);
            outcome
                .absorb(EventOutcome::layout_changed(false).with_arrange_space_scope(Some(target)));
        }
        outcome
    }

    pub(super) fn reconcile_native_presence(
        &mut self,
        wsid: WindowServerId,
        space: SpaceId,
        window: WindowId,
    ) -> EventOutcome {
        if self
            .stores
            .state
            .windows
            .native_fullscreen_record_for_window_server_id(wsid)
            .is_some()
            || self.stores.state.windows.native_fullscreen_record_for_window(window).is_some()
        {
            return self.restore_native_fullscreen(Some(wsid), window, Some(space));
        }
        self.admit_native_presence(window, space)
    }

    fn admit_native_presence(&mut self, window: WindowId, space: SpaceId) -> EventOutcome {
        // Appearance can restore participation, but is not an authoritative move.
        let owner = self.assigned_space_for_window_id(window).unwrap_or(space);
        let mut outcome = EventOutcome::no_change();
        if self.is_space_active(owner) && self.stores.state.windows().is_visible_admitted(window) {
            if self.stores.state.windows().workspace_for_window(window).is_some() {
                outcome.memberships.push((window, Some(owner)));
            } else {
                outcome = outcome.with_created_window_finalization(window);
            }
        }
        outcome
    }

    pub(super) fn reassign_window_to_authoritative_space(
        &mut self,
        wid: WindowId,
        space: SpaceId,
        preserve_workspace_ordinal: bool,
    ) {
        // A claim holds the destination; release replays this reconciliation. So does a hand
        // on the window, and a transfer that keeps its assignment.
        if self.stores.state.windows().is_claimed(wid)
            || self.stores.state.windows.drag_on(wid).is_some()
            || self
                .stores
                .state
                .windows()
                .record(wid)
                .and_then(|record| record.transfer())
                .is_some_and(|transfer| transfer.keeps_assignment(Some(space)))
        {
            return;
        }
        if !self.stores.state.windows().is_admitted(wid)
            || self.stores.state.windows.is_temporarily_unavailable(wid)
        {
            self.reconcile_window_membership(wid, None);
            return;
        }
        let owner = self.assigned_space_for_window_id(wid);
        if owner.is_none() {
            if self.is_space_active(space) {
                let admitted = EventOutcome::no_change().with_created_window_finalization(wid);
                self.apply_event_outcome(admitted);
            }
            return;
        }
        if owner != Some(space) {
            let departed_from = self.stores.state.windows().workspace_for_window(wid);
            let engine = &mut self.stores.layout.layout_engine;
            // A window a departure moved off this display goes back where it was.
            let returning = engine.eviction_ordinal(self.stores.state.windows(), wid, space);
            // Resolve the destination before touching the source: a global-mode Space can own
            // no workspace, and then the current owner is kept.
            engine
                .workspaces_mut()
                .ensure_space_initialized(&mut self.stores.state.windows_mut(), space);
            let workspaces = engine.workspaces();
            let remembered = returning.and_then(|ordinal| {
                workspaces
                    .reading(self.stores.state.windows())
                    .workspaces_on(space)
                    .nth(ordinal)
            });
            let destination = if remembered.is_some() {
                remembered
            } else if preserve_workspace_ordinal {
                workspaces.reading(self.stores.state.windows()).workspace_at_window_ordinal(
                    self.stores.state.windows(),
                    space,
                    wid,
                )
            } else {
                engine
                    .ensure_active_workspace_info(&mut self.stores.state.windows_mut(), space)
                    .map(|(id, _)| id)
            };
            let Some(workspace) = destination else {
                debug!(
                    ?wid,
                    ?space,
                    "Native Space has no destination workspace; keeping owner"
                );
                return;
            };
            self.send_layout_event(LayoutEvent::WindowRemovedPreserveFloating(wid));
            let engine = &mut self.stores.layout.layout_engine;
            if !engine.workspaces_mut().assign_window_to_workspace(
                &mut self.stores.state.windows_mut(),
                space,
                wid,
                workspace,
            ) {
                return;
            }
            if returning.is_some() {
                engine.take_eviction(wid);
            }
            if remembered.is_none()
                && preserve_workspace_ordinal
                && let Some(from) = departed_from
            {
                engine.note_eviction(self.stores.state.windows(), wid, from);
            }
        }
        if self.is_space_active(space) {
            self.reconcile_window_membership(wid, Some(space));
        }
    }
}
