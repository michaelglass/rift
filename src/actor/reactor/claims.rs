//! External window claims: a Mach session temporarily excludes windows from
//! Rift management. Ownership lives in [`WindowStore`](crate::model::WindowStore);
//! this module only applies the layout, presentation and focus effects.

use rift_protocol::WindowClaimFlags;

use super::{LayoutEvent, Reactor};
use crate::actor::app::WindowId;
use crate::actor::reactor::events::HeldCameras;
use crate::common::collections::HashSet;
use crate::layout_engine::systems::LayoutSystem;
use crate::model::window_store::ClaimOwner;
use crate::sys::screen::SpaceId;

const OWNED_ELSEWHERE: &str = "Window owned by another external manager";

impl Reactor {
    pub(crate) fn claim_window(
        &mut self,
        wid: WindowId,
        owner: ClaimOwner,
        flags: WindowClaimFlags,
    ) -> Result<(), &'static str> {
        if !flags.is_supported() {
            return Err("Unsupported window claim flags");
        }
        let wid = self
            .stores
            .state
            .windows()
            .resolve_claim_target(owner, wid)
            .ok_or("Window not found")?;
        match self.stores.state.windows().claim(wid) {
            Some(claim) if claim.owner != owner => return Err(OWNED_ELSEWHERE),
            // Flags only affect focus policy (the mode table reads them), never layout.
            Some(_) => {
                self.stores.state.windows_mut().set_claim_flags(wid, flags);
                return Ok(());
            }
            None => {}
        }
        {
            let reactor = &mut *self;
            // The owner takes over like a user drag: stop Rift's motion and pending writes.
            reactor.cancel_window_presentations(vec![wid]);
            reactor.stores.state.forget_drag_window(wid);
            reactor.drag_manager.sync_preview(reactor.stores.state.windows.drag());
            // Leave the projection (keeping floating state and the layout slot) before the
            // claim takes the assignment, but arrange only after: no write may target it.
            let space = reactor.assigned_space_for_window_id(wid);
            let removal = LayoutEvent::WindowSuspended(wid);
            let removed = reactor
                .stores
                .layout
                .layout_engine
                .handle_event(&mut reactor.stores.state.windows_mut(), removal)
                .changed;
            reactor.stores.state.windows_mut().insert_claim(wid, owner, flags);
            if removed {
                reactor.update_layout_or_warn(&HeldCameras::none(), false, space);
            }
            reactor.maybe_send_menu_update();
        }
        Ok(())
    }

    pub(crate) fn release_window(
        &mut self,
        wid: WindowId,
        owner: ClaimOwner,
    ) -> Result<(), &'static str> {
        let wid = self
            .stores
            .state
            .windows()
            .resolve_claim_target(owner, wid)
            .ok_or("Window not found")?;
        match self.stores.state.windows().claim(wid) {
            None => Ok(()),
            Some(claim) if claim.owner != owner => Err(OWNED_ELSEWHERE),
            Some(_) => {
                self.release_claims_of([wid]);
                Ok(())
            }
        }
    }

    /// The owner's session ended: return all of its windows, arranging each space once.
    pub(crate) fn release_claims(&mut self, owner: ClaimOwner) {
        let windows = self.stores.state.windows.owner_claims(owner).to_vec();
        if !windows.is_empty() {
            self.release_claims_of(windows);
        }
    }

    fn release_claims_of(&mut self, windows: impl IntoIterator<Item = WindowId>) {
        {
            let reactor = &mut *self;
            let spaces: HashSet<_> =
                windows.into_iter().filter_map(|wid| reactor.readmit(wid)).collect();
            for space in spaces {
                reactor.update_layout_or_warn(&HeldCameras::none(), false, Some(space));
            }
            reactor.maybe_send_menu_update();
        }
    }

    /// Replays normal reconciliation for a released window; returns an active space to arrange.
    fn readmit(&mut self, wid: WindowId) -> Option<SpaceId> {
        let claim = self.stores.state.windows_mut().remove_claim(wid)?;
        let engine = &mut self.stores.layout.layout_engine;
        if let Some(workspace) =
            claim.workspace().filter(|id| engine.workspaces().workspaces.contains_key(*id))
        {
            self.stores.state.windows_mut().assign_window_to_workspace(wid, workspace);
            // The owner may have changed the claim's destination. Reconcile only
            // at release; an ordinary return keeps its canonical slot untouched.
            for (id, current) in engine.workspaces_mut().workspaces.iter_mut() {
                if id != workspace {
                    current
                        .layout_system
                        .update(crate::layout_engine::systems::WindowChange::Remove(wid));
                }
            }
            // Like any external move of a floating window, the owner's geometry is authoritative.
            if engine.is_window_floating(wid)
                && let Some(window) = self.stores.state.windows.window(wid)
            {
                engine.store_floating_position(workspace, wid, window.frame_monotonic());
            }
        }
        if self.stores.state.windows.native_fullscreen_record_for_window(wid).is_some() {
            // Suspended windows return through native fullscreen recovery.
            let outcome = self.restore_native_fullscreen(None, wid, None);
            self.apply_event_outcome(outcome);
            return None;
        }
        let space = self.best_space_for_window_id(wid)?;
        self.reassign_window_to_authoritative_space(wid, space, false);
        self.is_space_active(space).then_some(space)
    }
}
