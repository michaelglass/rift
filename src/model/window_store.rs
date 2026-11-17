use rift_protocol::WindowClaimFlags;

use crate::actor::app::WindowId;
use crate::common::collections::{HashMap, HashSet};
use crate::model::VirtualWorkspaceId;
use crate::model::drag::{MouseButton, PointerReleased, Session};
use crate::model::reactor::WindowState;
use crate::sys::screen::SpaceId;
use crate::sys::window_server::{WindowServerId, WindowServerInfo};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WindowVisibility {
    #[default]
    Unknown,
    Visible,
    Hidden,
    Minimized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferStatus {
    Pending,
    Deferred,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PendingWindowTransfer {
    pub generation: u64,
    pub requested_frame: Option<objc2_core_foundation::CGRect>,
    pub requested_space: SpaceId,
    pub(crate) last_revision: Option<u64>,
    started_revision: u64,
    pub(crate) attempts: u8,
    pub status: TransferStatus,
}

/// What the user's hand is doing with the one window it is on.
#[derive(Debug)]
enum Grip {
    /// `button` is down on the window. Its transfer, if any, waits: a release without a
    /// drag puts it back, a drag supersedes it.
    Pressed {
        button: MouseButton,
        waiting: Option<PendingWindowTransfer>,
    },
    Dragging(Box<Session>),
}

/// What the pointer is doing with windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pointer {
    Idle,
    /// Down on a window that hasn't moved.
    Pressing,
    /// Dragging a window.
    Dragging,
}

/// What the user's hand is doing with a window, for whoever observed that window change.
pub(crate) enum Hand<'a> {
    /// The pointer is down on it and it hasn't moved yet: a change Rift didn't make starts
    /// its drag.
    Pressed(Pressed<'a>),
    /// The user is dragging it: a change follows the drag.
    Dragging(&'a mut Session),
}

/// The pointer is down on a window that isn't dragged yet: the only thing a drag starts from.
pub(crate) struct Pressed<'a>(&'a mut Grip);

impl Pressed<'_> {
    /// The pressed window moved under the pointer, or a modifier press took it: the drag begins,
    /// superseding any transfer that was waiting.
    pub(crate) fn start(self, session: Session) { *self.0 = Grip::Dragging(Box::new(session)); }
}

/// A record Rift places: the window's data and the transfer in flight, if any.
#[derive(Debug, Default)]
struct Loose {
    record: WindowRecord,
    transfer: Option<PendingWindowTransfer>,
}

/// The record of the one window the user's hand is on. It has no transfer to request or
/// advance: the user places this window.
#[derive(Debug)]
struct Gripped {
    window: WindowId,
    record: WindowRecord,
    grip: Grip,
}

/// Every window's record, each in exactly one place: with the windows Rift places, or in the
/// single slot for the window under the user's hand. One slot, so one press or drag at a time;
/// no transfer in the slot, so a pressed or dragged window can't be pulled back to its
/// assignment; and a drag starts only from the slot's [`Pressed`].
#[derive(Debug, Default)]
struct Records {
    loose: HashMap<WindowId, Loose>,
    gripped: Option<Gripped>,
}

impl Records {
    fn get(&self, window: &WindowId) -> Option<&WindowRecord> {
        match &self.gripped {
            Some(gripped) if gripped.window == *window => Some(&gripped.record),
            _ => self.loose.get(window).map(|loose| &loose.record),
        }
    }

    fn get_mut(&mut self, window: &WindowId) -> Option<&mut WindowRecord> {
        match &mut self.gripped {
            Some(gripped) if gripped.window == *window => Some(&mut gripped.record),
            _ => self.loose.get_mut(window).map(|loose| &mut loose.record),
        }
    }

    fn contains_key(&self, window: &WindowId) -> bool { self.get(window).is_some() }

    /// The record for `window`, tracked loose if it is new.
    fn entry_or_default(&mut self, window: WindowId) -> &mut WindowRecord {
        match &mut self.gripped {
            Some(gripped) if gripped.window == window => &mut gripped.record,
            _ => &mut self.loose.entry(window).or_default().record,
        }
    }

    /// `window`'s loose record, tracked if it is new; `None` for the gripped window.
    fn loose_or_new(&mut self, window: WindowId) -> Option<&mut Loose> {
        match &self.gripped {
            Some(gripped) if gripped.window == window => None,
            _ => Some(self.loose.entry(window).or_default()),
        }
    }

    /// Removes the record wherever it is; a press or drag on it ends with it.
    fn remove(&mut self, window: &WindowId) -> Option<WindowRecord> {
        if self.gripped_window() == Some(*window) {
            return self.gripped.take().map(|gripped| gripped.record);
        }
        self.loose.remove(window).map(|loose| loose.record)
    }

    fn iter(&self) -> impl Iterator<Item = (&WindowId, &WindowRecord)> {
        let gripped = self.gripped.iter().map(|gripped| (&gripped.window, &gripped.record));
        self.loose.iter().map(|(window, loose)| (window, &loose.record)).chain(gripped)
    }

    fn values(&self) -> impl Iterator<Item = &WindowRecord> {
        self.iter().map(|(_, record)| record)
    }

    fn transfer(&self, window: &WindowId) -> Option<PendingWindowTransfer> {
        self.loose.get(window)?.transfer
    }

    fn transfers_mut(&mut self) -> impl Iterator<Item = &mut PendingWindowTransfer> {
        self.loose.values_mut().filter_map(|loose| loose.transfer.as_mut())
    }

    /// Forgets a transfer, in flight or waiting behind a press.
    fn clear_transfer(&mut self, window: &WindowId) {
        match &mut self.gripped {
            Some(Gripped {
                window: pressed,
                grip: Grip::Pressed { waiting, .. },
                ..
            }) if pressed == window => *waiting = None,
            _ => {
                if let Some(loose) = self.loose.get_mut(window) {
                    loose.transfer = None;
                }
            }
        }
    }

    fn gripped_window(&self) -> Option<WindowId> {
        self.gripped.as_ref().map(|gripped| gripped.window)
    }

    /// The pointer goes down on `window`: its record moves into the slot, its transfer waits.
    fn press(&mut self, window: WindowId, button: MouseButton) {
        self.let_go();
        if let Some(loose) = self.loose.remove(&window) {
            self.gripped = Some(Gripped {
                window,
                record: loose.record,
                grip: Grip::Pressed {
                    button,
                    waiting: loose.transfer,
                },
            });
        }
    }

    /// The user's hand comes off: the record goes back among the loose ones, with the transfer
    /// that waited behind a press. A drag's session is returned.
    fn let_go(&mut self) -> Option<(WindowId, Option<Session>)> {
        let Gripped { window, record, grip } = self.gripped.take()?;
        let (transfer, session) = match grip {
            Grip::Pressed { waiting, .. } => (waiting, None),
            Grip::Dragging(session) => (None, Some(*session)),
        };
        self.loose.insert(window, Loose { record, transfer });
        Some((window, session))
    }

    /// macOS rekeyed the gripped window: the grip moves to its new identity.
    fn regrip(&mut self, from: WindowId, to: WindowId) {
        let Some(Gripped { record, grip, .. }) = self.gripped.take() else {
            return;
        };
        let replacement = self.loose.remove(&to).unwrap_or_default();
        self.loose.insert(from, Loose { record, transfer: None });
        self.gripped = Some(Gripped {
            window: to,
            record: replacement.record,
            grip,
        });
    }
}

impl std::ops::Index<&WindowId> for Records {
    type Output = WindowRecord;

    fn index(&self, window: &WindowId) -> &WindowRecord {
        self.get(window).expect("tracked window")
    }
}

/// A window as Rift may place it: its transfer slot. Only a window the user hasn't pressed
/// has one, so transfers are requested and advanced here and nowhere else.
pub(crate) struct Unheld<'a> {
    transfer: &'a mut Option<PendingWindowTransfer>,
    frame: Option<objc2_core_foundation::CGRect>,
    placeable: bool,
    topology_revision: u64,
    generation: &'a mut u64,
}

impl Unheld<'_> {
    /// Coalesce repeated intent; only a new destination or newly available
    /// lifecycle context can start physical work. Exhaustion preserves intent.
    pub(crate) fn request_transfer(
        self,
        frame: Option<objc2_core_foundation::CGRect>,
        target: SpaceId,
        available: bool,
    ) -> bool {
        let available = available && self.placeable;
        if let Some(operation) = self.transfer.as_mut()
            && operation.requested_space == target
        {
            use crate::sys::geometry::SameAs;
            let resumed = available && operation.status == TransferStatus::Unavailable;
            // Newer geometry supersedes stale frames without resetting the budget.
            let new_frame = frame.is_some_and(|frame| {
                operation.requested_frame.is_none_or(|requested| !requested.same_as(frame))
            });
            if new_frame {
                operation.requested_frame = frame;
            }
            if !available {
                operation.status = TransferStatus::Unavailable;
            } else if resumed {
                operation.status = TransferStatus::Pending;
                operation.attempts = 0;
            }
            let write = available && operation.requested_frame.is_some() && (resumed || new_frame);
            if write {
                operation.last_revision = Some(self.topology_revision);
            }
            return write;
        }
        *self.generation = self.generation.wrapping_add(1);
        let write = available && frame.is_some();
        *self.transfer = Some(PendingWindowTransfer {
            generation: *self.generation,
            requested_frame: frame,
            requested_space: target,
            started_revision: self.topology_revision,
            last_revision: write.then_some(self.topology_revision),
            attempts: 0,
            status: if available {
                TransferStatus::Pending
            } else {
                TransferStatus::Unavailable
            },
        });
        write
    }

    /// Retry only on accepted, complete topology revisions, with a fixed budget.
    /// Exhaustion keeps the logical intent; a frame acknowledgement never confirms Space membership.
    pub(crate) fn advance_transfer(
        self,
        revision: u64,
        available: bool,
    ) -> Option<PendingWindowTransfer> {
        let operation = self.transfer.as_mut()?;
        if !(available && self.placeable) {
            operation.status = TransferStatus::Unavailable;
            return None;
        }
        if operation.status == TransferStatus::Unavailable {
            operation.status = TransferStatus::Pending;
            operation.attempts = 0;
            operation.last_revision = None;
        }
        operation.requested_frame = operation.requested_frame.or(self.frame);
        if operation.status != TransferStatus::Pending
            || operation.requested_frame.is_none()
            || operation.last_revision.is_some_and(|previous| revision <= previous)
        {
            return None;
        }
        operation.last_revision = Some(revision);
        if operation.attempts >= 2 {
            // Exhaustion changes retry authority, never accepted frame authority.
            operation.status = TransferStatus::Deferred;
            return None;
        }
        operation.attempts += 1;
        Some(*operation)
    }
}

/// The complete reactor-owned state for one AX window identity.
#[derive(Debug, Default)]
pub struct WindowRecord {
    state: Option<WindowState>,
    window_server_id: Option<WindowServerId>,
    workspace: Option<VirtualWorkspaceId>,
    visibility: WindowVisibility,
    // AX invalidation retains ownership until fresh AX evidence or confirmed retirement.
    ax_invalidated: bool,
    rule_floating: bool,
    floating_display_geometry: Option<FloatingDisplayGeometry>,
    claim: Option<WindowClaim>,
}

/// The Mach client port (session) holding external claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClaimOwner(pub u32);

/// Temporary exclusion of a tracked window from Rift management.
///
/// The claim holds the window's workspace assignment outside the workspace
/// index, so enumeration, hiding, rebuilds and projection never see a claimed
/// window while native identity, visibility and fullscreen tracking continue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowClaim {
    pub owner: ClaimOwner,
    pub flags: WindowClaimFlags,
    /// The id the owner claimed; release accepts it across AX identity changes.
    original: WindowId,
    workspace: Option<VirtualWorkspaceId>,
}

impl WindowClaim {
    pub fn workspace(&self) -> Option<VirtualWorkspaceId> { self.workspace }
}

/// Only retained while display projection differs from the user's geometry.
/// Lives with the window identity, never in the persisted layout.
#[derive(Debug, Clone, Copy)]
struct FloatingDisplayGeometry {
    intended: objc2_core_foundation::CGRect,
    viewport: objc2_core_foundation::CGRect,
}

impl WindowRecord {
    /// Latest frame accepted by the reactor, including its own completed writes.
    pub fn frame(&self) -> Option<objc2_core_foundation::CGRect> {
        self.state.as_ref().map(|state| state.frame_monotonic)
    }

    pub fn window_server_id(&self) -> Option<WindowServerId> { self.window_server_id }

    pub fn visibility(&self) -> WindowVisibility { self.visibility }
}

/// A window's record together with its transfer in flight, if it has one.
#[derive(Debug, Clone, Copy)]
pub struct RecordRef<'a> {
    record: &'a WindowRecord,
    transfer: Option<PendingWindowTransfer>,
}

impl std::ops::Deref for RecordRef<'_> {
    type Target = WindowRecord;

    fn deref(&self) -> &WindowRecord { self.record }
}

impl RecordRef<'_> {
    pub fn pending_transfer(&self) -> Option<PendingWindowTransfer> { self.transfer }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeFullscreenTransition {
    EnterRequested,
    Suspended,
    ExitRequested,
    /// Native exit is confirmed; identity/admission recovery still owns suspension.
    ExitObserved(Option<SpaceId>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeFullscreenRecord {
    pub original_window_id: WindowId,
    pub current_window_id: WindowId,
    pub window_server_id: Option<WindowServerId>,
    pub workspace: Option<VirtualWorkspaceId>,
    pub last_known_user_space: Option<SpaceId>,
    pub fullscreen_space: SpaceId,
    pub transition: NativeFullscreenTransition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingNativeFullscreenRecord {
    pub pid: i32,
    pub window_server_id: WindowServerId,
    pub last_known_user_space: Option<SpaceId>,
    pub fullscreen_space: SpaceId,
    pub transition: NativeFullscreenTransition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingNativeFullscreenState {
    pid: i32,
    last_known_user_space: Option<SpaceId>,
    fullscreen_space: SpaceId,
    transition: NativeFullscreenTransition,
}

#[derive(Debug, Default)]
struct WindowServerRecord {
    window_id: Option<WindowId>,
    visible: bool,
    observed: bool,
    space: Option<SpaceId>,
    info: Option<WindowServerInfo>,
    pending_native_fullscreen: Option<PendingNativeFullscreenState>,
}

impl WindowServerRecord {
    fn is_empty(&self) -> bool {
        self.window_id.is_none()
            && !self.visible
            && !self.observed
            && self.space.is_none()
            && self.info.is_none()
            && self.pending_native_fullscreen.is_none()
    }
}

/// Authoritative per-window metadata tracked by Rift.
///
/// Workspace ownership lives here, not inside `VirtualWorkspace`. Layout trees
/// retain canonical topology; geometry projects their participating members. Keeping the
/// assignment index here avoids the old class of bugs where a window could be
/// present in multiple workspace-owned sets after sleep/wake or same-space
/// workspace moves, which then leaked into queries and layout recovery.
#[derive(Debug, Default)]
pub struct WindowStore {
    topology_revision: u64,
    operation_generation: u64,
    windows: Records,
    app_windows: HashMap<i32, HashSet<WindowId>>,
    window_servers: HashMap<WindowServerId, WindowServerRecord>,
    workspace_windows: HashMap<VirtualWorkspaceId, HashSet<WindowId>>,
    native_fullscreen_records_by_original_window: HashMap<WindowId, NativeFullscreenRecord>,
    native_fullscreen_original_window_by_current_window: HashMap<WindowId, WindowId>,
    native_fullscreen_original_window_by_window_server: HashMap<WindowServerId, WindowId>,
    claims: HashMap<ClaimOwner, Vec<WindowId>>,
}

/// Restored native lifecycle facts; placement and actor effects remain external.
pub(crate) struct NativeFullscreenRestoration {
    pub(crate) record: NativeFullscreenRecord,
    pub(crate) window: Option<WindowId>,
    pub(crate) removed_window: Option<WindowId>,
}

/// Native facts supplementing a successful AX inventory omission. Unknown
/// fields carry no negative authority; an omission alone never retires identity.
#[derive(Debug)]
pub(crate) struct InventoryWindowObservation {
    pub(crate) info: Option<WindowServerInfo>,
    pub(crate) suitable: Option<bool>,
    pub(crate) ordered_in: Option<bool>,
}

/// A user-Space departure can be a move, a close, or temporary ordering out.
#[derive(Debug)]
pub(crate) enum NativeDeparture {
    Moved(Option<WindowId>, SpaceId),
    Closed(WindowId),
    Hidden {
        window: Option<WindowId>,
        remove_projection: bool,
    },
}

impl WindowStore {
    pub(crate) fn accept_topology_revision(&mut self, revision: u64) {
        self.topology_revision = self.topology_revision.max(revision);
    }

    /// Direct topology, live membership, and pending moves have different authority.
    /// A live conflict can supersede a pending move; otherwise its target wins
    /// over a stale direct observation. Unknown live membership retains prior facts.
    pub(crate) fn resolve_native_space(
        &self,
        wsid: WindowServerId,
        observation: Option<SpaceId>,
        pending: Option<SpaceId>,
        live: Option<SpaceId>,
    ) -> Option<SpaceId> {
        match (observation, pending) {
            (Some(observed), Some(target)) if observed != target => {
                Some(if live == Some(observed) {
                    observed
                } else {
                    target
                })
            }
            (Some(observed), _) => Some(observed),
            (None, _) => live.or(pending).or(self.window_server_space(wsid)),
        }
    }

    /// Rift manages a valid AX window when admission passes and no external owner holds it.
    pub(crate) fn is_admitted(&self, wid: WindowId) -> bool {
        self.record(wid).is_some_and(|record| {
            !record.ax_invalidated
                && record.claim.is_none()
                && record.state.as_ref().is_some_and(WindowState::is_admitted)
        })
    }

    /// Rules and admission are reconciled only for windows Rift may manage.
    pub(crate) fn can_reconcile_admission(&self, wid: WindowId) -> bool {
        self.record(wid).is_some_and(|record| {
            !record.ax_invalidated
                && record.claim.is_none()
                && record.state.as_ref().is_some_and(WindowState::can_reconcile_admission)
        })
    }

    pub fn claim(&self, wid: WindowId) -> Option<WindowClaim> {
        self.record(wid).and_then(|record| record.claim)
    }

    pub fn is_claimed(&self, wid: WindowId) -> bool {
        self.record(wid).is_some_and(|record| record.claim.is_some())
    }

    pub fn claimed_windows(&self) -> impl Iterator<Item = WindowId> + '_ {
        self.claims.values().flatten().copied()
    }

    pub fn owner_claims(&self, owner: ClaimOwner) -> &[WindowId] {
        self.claims.get(&owner).map_or(&[], Vec::as_slice)
    }

    pub fn suppresses_focus_follows_mouse(&self, wid: Option<WindowId>) -> bool {
        !self.claims.is_empty()
            && wid.and_then(|wid| self.claim(wid)).is_some_and(|claim| {
                claim.flags.contains(WindowClaimFlags::SUPPRESS_FOCUS_FOLLOWS_MOUSE)
            })
    }

    /// The window `owner` means by `wid`: its own claim by current, then original, id (an
    /// id reused after a rekey still reaches the claim), otherwise the window tracked as `wid`.
    pub fn resolve_claim_target(&self, owner: ClaimOwner, wid: WindowId) -> Option<WindowId> {
        let owned = self.owner_claims(owner);
        if owned.contains(&wid) {
            return Some(wid);
        }
        let original = owned
            .iter()
            .copied()
            .find(|current| self.claim(*current).is_some_and(|claim| claim.original == wid));
        original.or_else(|| self.contains_window(wid).then_some(wid))
    }

    /// Claim a tracked window, holding its assignment until release.
    pub fn insert_claim(&mut self, wid: WindowId, owner: ClaimOwner, flags: WindowClaimFlags) {
        debug_assert!(self.contains_window(wid) && !self.is_claimed(wid));
        let workspace = self.take_assignment(wid);
        self.windows.get_mut(&wid).unwrap().claim = Some(WindowClaim {
            owner,
            flags,
            original: wid,
            workspace,
        });
        self.claims.entry(owner).or_default().push(wid);
    }

    pub fn set_claim_flags(&mut self, wid: WindowId, flags: WindowClaimFlags) {
        if let Some(claim) = self.windows.get_mut(&wid).and_then(|record| record.claim.as_mut()) {
            claim.flags = flags;
        }
    }

    /// End a claim. The caller restores the returned assignment if it is still valid.
    pub fn remove_claim(&mut self, wid: WindowId) -> Option<WindowClaim> {
        let claim = self.windows.get_mut(&wid)?.claim.take()?;
        self.unindex_claim(claim.owner, wid);
        self.prune_window_record(wid);
        Some(claim)
    }

    fn unindex_claim(&mut self, owner: ClaimOwner, wid: WindowId) {
        if let Some(windows) = self.claims.get_mut(&owner) {
            windows.retain(|window| *window != wid);
            if windows.is_empty() {
                self.claims.remove(&owner);
            }
        }
    }

    fn take_assignment(&mut self, wid: WindowId) -> Option<VirtualWorkspaceId> {
        let assignment = self.windows.get_mut(&wid)?.workspace.take()?;
        self.remove_window_from_workspace_index(wid, assignment);
        Some(assignment)
    }

    /// Temporary unavailability changes admission, not logical workspace ownership.
    pub(crate) fn reconcile_admission(&mut self, wid: WindowId) -> bool {
        let admitted = self.window(wid).is_some_and(WindowState::is_admitted);
        let explicitly_excluded =
            self.window(wid).is_some_and(|w| w.manage_override == Some(false));
        if !admitted && (explicitly_excluded || !self.is_temporarily_unavailable(wid)) {
            self.remove_window_assignment(wid);
        }
        admitted
    }

    pub(crate) fn invalidate_ax_identity(&mut self, wid: WindowId) {
        if let Some(record) = self.windows.get_mut(&wid) {
            record.ax_invalidated = true;
        }
    }

    /// Only a fresh AX observation validates an invalidated identity.
    pub(crate) fn validate_ax_identity(&mut self, wid: WindowId) -> bool {
        self.windows
            .get_mut(&wid)
            .is_some_and(|r| std::mem::take(&mut r.ax_invalidated))
    }

    /// Live windows can lose heuristic admission as their AX/native metadata
    /// changes. Suspend their layout participation without forgetting ownership;
    /// only an explicit exclusion or retirement should discard that ownership.
    pub(crate) fn is_temporarily_unavailable(&self, wid: WindowId) -> bool {
        self.record(wid).is_some_and(|r| r.ax_invalidated)
            || self.window(wid).is_some_and(|w| {
                w.info.is_minimized || (w.manage_override != Some(false) && !w.is_admitted())
            })
            || self.is_window_native_fullscreen_suspended(wid)
    }

    pub(crate) fn is_visible_admitted(&self, wid: WindowId) -> bool {
        self.is_admitted(wid)
            && self
                .record(wid)
                .and_then(|record| record.window_server_id())
                .is_some_and(|wsid| self.is_window_visible(wsid))
    }

    /// Restore identity for native presence or a return to the recorded user Space.
    pub(crate) fn restore_native_identity(
        &mut self,
        wsid: Option<WindowServerId>,
        original: WindowId,
    ) -> Option<NativeFullscreenRestoration> {
        let record = wsid
            .and_then(|wsid| self.native_fullscreen_record_for_window_server_id(wsid))
            .or_else(|| self.native_fullscreen_record_for_window(original))?;
        if !matches!(record.transition, NativeFullscreenTransition::ExitObserved(_)) {
            return None;
        }
        let bound = record.window_server_id.and_then(|id| self.tracked_window_id(id));
        // A specific appearance follows the record's current AX identity first;
        // a Space return follows its surviving native binding first.
        let window = [
            wsid.map(|_| record.current_window_id),
            bound,
            Some(record.current_window_id),
            wsid.and_then(|id| self.tracked_window_id(id)),
            wsid.map(|_| original),
        ]
        .into_iter()
        .flatten()
        .find(|wid| self.is_admitted(*wid));
        let removed_window = window
            .filter(|wid| *wid != original)
            .and_then(|_| self.remove_window_assignment(original))
            .map(|_| original);
        Some(NativeFullscreenRestoration { record, window, removed_window })
    }

    /// Native fullscreen suspends admission while retaining identity and ownership.
    /// Only positive entry/discovery evidence may establish this suspension.
    pub(crate) fn observe_native_fullscreen(
        &mut self,
        wsid: WindowServerId,
        space: SpaceId,
        fallback_user_space: Option<SpaceId>,
        pid: Option<i32>,
    ) -> Option<WindowId> {
        let lifecycle = self
            .native_fullscreen_record_for_window_server_id(wsid)
            .map(|record| (record.fullscreen_space, record.transition))
            .or_else(|| {
                self.pending_native_fullscreen_record_for_window_server_id(wsid)
                    .map(|record| (record.fullscreen_space, record.transition))
            });
        if lifecycle.is_some_and(|(fullscreen, transition)| {
            fullscreen == space && matches!(transition, NativeFullscreenTransition::ExitObserved(_))
        }) {
            return None;
        }
        self.set_window_server_space(wsid, Some(space));
        if let Some(wid) = self.tracked_window_id(wsid) {
            let assignment = self.workspace_for_window(wid);
            self.suspend_window_to_native_fullscreen(
                wid,
                Some(wsid),
                fallback_user_space,
                space,
                NativeFullscreenTransition::Suspended,
            );
            assignment.map(|_| wid)
        } else {
            if let Some(pid) =
                pid.or_else(|| self.get_window_server_info(wsid).map(|info| info.pid))
            {
                self.suspend_window_server_to_native_fullscreen(
                    pid,
                    wsid,
                    fallback_user_space,
                    space,
                    NativeFullscreenTransition::Suspended,
                );
            }
            None
        }
    }

    pub(crate) fn observe_native_departure(
        &mut self,
        wsid: WindowServerId,
        reported_space: SpaceId,
        resolved_space: Option<SpaceId>,
        active_spaces: &HashSet<SpaceId>,
        ordered_in: Option<bool>,
        suitable: Option<bool>,
    ) -> NativeDeparture {
        if let Some(space) = resolved_space.filter(|space| *space != reported_space) {
            return NativeDeparture::Moved(
                self.observe_native_space(wsid, space, active_spaces.contains(&space)),
                space,
            );
        }
        let window = self.tracked_window_id(wsid);
        // Ordering out precedes fullscreen entry too. Only retire when the
        // independent native inventory also rejects this window identity.
        if let Some(wid) = window {
            if ordered_in == Some(false)
                && suitable == Some(false)
                && self.native_fullscreen_record_for_window(wid).is_none()
            {
                return NativeDeparture::Closed(wid);
            }
        }
        let remove_projection = window.and_then(|wid| self.record(wid)).is_some_and(|record| {
            record.workspace.is_some()
                && self.is_temporarily_unavailable(window.unwrap())
                && self.native_space(window.unwrap()) == Some(reported_space)
        });
        self.observe_native_space(wsid, reported_space, false);
        NativeDeparture::Hidden { window, remove_projection }
    }

    /// Bind a positive AX observation to its stable native identity. Persistent
    /// membership survives AX replacement; only projection rekeying is external.
    pub(crate) fn reconcile_ax_identity(
        &mut self,
        wid: WindowId,
        wsid: Option<WindowServerId>,
    ) -> Option<WindowId> {
        if let Some(old) = self.record(wid).and_then(|record| record.window_server_id())
            && Some(old) != wsid
        {
            self.remove_window_server_state(old);
            self.windows.clear_transfer(&wid);
        }
        let wsid = wsid?;
        let previous = self.track_window_server_id(wsid, wid).filter(|old| *old != wid);
        if let Some(old) = previous {
            self.transfer_persistent_window_metadata(old, wid);
            if !self.contains_window(wid) {
                let state = self.windows.get_mut(&old).and_then(|record| record.state.take());
                if let Some(state) = state {
                    self.insert_window(wid, state);
                }
            }
            self.remove_window(old);
        }
        self.clear_window_server_observed(wsid);
        previous
    }

    pub(crate) fn observe_native_space(
        &mut self,
        wsid: WindowServerId,
        space: SpaceId,
        visible: bool,
    ) -> Option<WindowId> {
        self.set_window_server_space(wsid, Some(space));
        self.set_native_visibility(wsid, visible);
        let wid = self.tracked_window_id(wsid);
        if wid.is_some() {
            self.clear_window_server_observed(wsid);
        }
        wid
    }

    /// Enforce the minimized invariant even if the AX flag was already set.
    /// Every known identity requests projection repair; the projection reports
    /// whether removal actually changed geometry, so correct duplicates are cheap.
    pub(crate) fn observe_minimized(&mut self, wid: WindowId) -> bool {
        let Some(window) = self.window_mut(wid) else {
            return false;
        };
        window.info.is_minimized = true;
        window.is_manageable = false;
        if let Some(wsid) = self.record(wid).and_then(|record| record.window_server_id()) {
            self.mark_window_hidden(wsid);
        }
        self.set_visibility(wid, WindowVisibility::Minimized);
        self.validate_ax_identity(wid);
        true
    }

    /// Hidden is visibility evidence, never destruction. Return the owning PID
    /// for a targeted, coalesced AX refresh (including pending native identity).
    pub(crate) fn observe_native_visibility(
        &mut self,
        wsid: WindowServerId,
        visible: bool,
    ) -> Option<i32> {
        let pid = self
            .tracked_window_id(wsid)
            .map(|wid| wid.pid)
            .or_else(|| self.get_window_server_info(wsid).map(|info| info.pid));
        self.set_native_visibility(wsid, visible);
        pid
    }

    /// A current successful AX omission is destructive only with native negative evidence.
    pub(crate) fn reconcile_app_inventory(
        &mut self,
        pid: i32,
        observed: &[WindowId],
        inactive: &HashSet<WindowId>,
        mut observe: impl FnMut(WindowServerId, Option<WindowServerInfo>) -> InventoryWindowObservation,
    ) -> Vec<(WindowId, Option<WindowServerId>)> {
        let observed: HashSet<_> = observed.iter().copied().collect();
        let retired: Vec<_> = self
            .window_ids_for_pid(pid)
            .filter_map(|wid| {
                let record = self.record(wid)?;
                if observed.contains(&wid)
                    || inactive.contains(&wid)
                    || record.state.as_ref()?.info.is_minimized
                    || self.native_fullscreen_record_for_window(wid).is_some()
                {
                    return None;
                }
                let wsid = record.window_server_id?;
                let observation = observe(wsid, self.get_window_server_info(wsid));
                if observation.suitable == Some(true)
                    && observation.ordered_in == Some(false)
                    && !record.ax_invalidated
                {
                    // AX omission can be caused by native ordering. After AX
                    // invalidation, omission plus ordering out confirms retirement;
                    // native suitability describes type, not AX liveness.
                    return None;
                }
                (observation.suitable == Some(false)
                    || observation.ordered_in == Some(false)
                    || observation.info.is_some_and(|info| {
                        info.layer != 0
                            || info.frame.size.width.abs() < 2.0
                            || info.frame.size.height.abs() < 2.0
                    }))
                .then_some((wid, Some(wsid)))
            })
            .collect();
        for &(wid, _) in &retired {
            self.remove_window(wid);
        }
        retired
    }

    /// Apply native presence and visibility, returning only affected logical
    /// ownership requiring location confirmation. Unmapped/inactive windows
    /// can become hidden without an unnecessary native membership query.
    pub(crate) fn reconcile_native_snapshot(
        &mut self,
        workspaces: &crate::model::virtual_workspace::WorkspaceStore,
        observed: &[(WindowServerId, Option<SpaceId>)],
        active_spaces: &HashSet<SpaceId>,
    ) -> HashSet<WindowServerId> {
        let observed_ids: HashSet<_> = observed.iter().map(|(wsid, _)| *wsid).collect();
        let hidden: Vec<_> = self
            .iter_visible_window_server_ids()
            .filter(|wsid| !observed_ids.contains(wsid))
            .collect();
        for wsid in hidden {
            self.mark_window_hidden(wsid);
        }
        for &(wsid, space) in observed {
            if let Some(space) = space {
                self.observe_native_space(wsid, space, true);
            } else {
                self.mark_window_visible(wsid);
            }
        }
        self.workspace_windows
            .values()
            .flat_map(|windows| windows.iter())
            .filter_map(|wid| {
                let record = self.record(*wid)?;
                let wsid = record.window_server_id?;
                (record.workspace.is_some_and(|assignment| {
                    workspaces
                        .workspaces
                        .get(assignment)
                        .is_some_and(|workspace| active_spaces.contains(&workspace.space))
                }) && self.native_fullscreen_record_for_window(*wid).is_none()
                    && !observed_ids.contains(&wsid))
                .then_some(wsid)
            })
            .collect()
    }

    /// Native absence may reflect ordering rather than an AX lifecycle change.
    /// Confirmed moves reconcile location; a live AX window keeps its ownership.
    pub(crate) fn reconcile_native_absence(
        &mut self,
        workspaces: &crate::model::virtual_workspace::WorkspaceStore,
        wsid: WindowServerId,
        active_spaces: &HashSet<SpaceId>,
        inactive_target: Option<SpaceId>,
        preserve_assignment: bool,
    ) -> Option<(WindowId, Option<SpaceId>)> {
        self.mark_window_hidden(wsid);
        let wid = self.tracked_window_id(wsid)?;
        let record = self.record(wid)?;
        // Hiding, minimizing and fullscreen all retain ownership while suspended.
        if self.is_temporarily_unavailable(wid)
            || !record.workspace.is_some_and(|assignment| {
                workspaces
                    .workspaces
                    .get(assignment)
                    .is_some_and(|workspace| active_spaces.contains(&workspace.space))
            })
        {
            return None;
        }
        if let Some(space) = inactive_target {
            self.set_window_server_space(wsid, Some(space));
            return Some((wid, Some(space)));
        }
        if preserve_assignment || self.is_admitted(wid) {
            return None;
        }
        self.remove_window_assignment(wid);
        Some((wid, None))
    }

    fn native_fullscreen_original_window(&self, window_id: WindowId) -> Option<WindowId> {
        if self.native_fullscreen_records_by_original_window.contains_key(&window_id) {
            Some(window_id)
        } else {
            self.native_fullscreen_original_window_by_current_window
                .get(&window_id)
                .copied()
        }
    }

    fn upsert_native_fullscreen_record(
        &mut self,
        record: NativeFullscreenRecord,
    ) -> NativeFullscreenRecord {
        if let Some(previous) = self
            .native_fullscreen_records_by_original_window
            .insert(record.original_window_id, record)
        {
            self.native_fullscreen_original_window_by_current_window
                .remove(&previous.current_window_id);
            if let Some(previous_wsid) = previous.window_server_id {
                self.native_fullscreen_original_window_by_window_server.remove(&previous_wsid);
            }
        }

        self.native_fullscreen_original_window_by_current_window
            .insert(record.current_window_id, record.original_window_id);
        if let Some(wsid) = record.window_server_id {
            self.native_fullscreen_original_window_by_window_server
                .insert(wsid, record.original_window_id);
        }

        record
    }

    fn remove_native_fullscreen_record_by_original_window(
        &mut self,
        original_window_id: WindowId,
    ) -> Option<NativeFullscreenRecord> {
        let record =
            self.native_fullscreen_records_by_original_window.remove(&original_window_id)?;
        self.native_fullscreen_original_window_by_current_window
            .remove(&record.current_window_id);
        if let Some(wsid) = record.window_server_id {
            self.native_fullscreen_original_window_by_window_server.remove(&wsid);
        }
        Some(record)
    }

    fn remove_window_from_workspace_index(
        &mut self,
        window_id: WindowId,
        assignment: VirtualWorkspaceId,
    ) {
        let should_prune = if let Some(windows) = self.workspace_windows.get_mut(&assignment) {
            windows.remove(&window_id);
            windows.is_empty()
        } else {
            false
        };
        if should_prune {
            self.workspace_windows.remove(&assignment);
        }
    }

    fn add_window_to_workspace_index(
        &mut self,
        window_id: WindowId,
        assignment: VirtualWorkspaceId,
    ) {
        self.workspace_windows.entry(assignment).or_default().insert(window_id);
    }

    pub(crate) fn window(&self, window_id: WindowId) -> Option<&WindowState> {
        self.windows.get(&window_id).and_then(|record| record.state.as_ref())
    }

    pub(crate) fn window_mut(&mut self, window_id: WindowId) -> Option<&mut WindowState> {
        self.windows.get_mut(&window_id).and_then(|record| record.state.as_mut())
    }

    pub(crate) fn insert_window(&mut self, window_id: WindowId, window: WindowState) {
        let wsid = window.info.sys_id;
        let retained_workspace = {
            let record = self.windows.entry_or_default(window_id);
            record.state = Some(window);
            record.ax_invalidated = false;
            record.workspace
        };
        self.app_windows.entry(window_id.pid).or_default().insert(window_id);
        if let Some(workspace) = retained_workspace {
            self.add_window_to_workspace_index(window_id, workspace);
        }
        if let Some(wsid) = wsid {
            self.track_window_server_id(wsid, window_id);
        }
    }

    pub(crate) fn project_floating_frame(
        &mut self,
        window: WindowId,
        frame: objc2_core_foundation::CGRect,
        source: Option<objc2_core_foundation::CGRect>,
        target: objc2_core_foundation::CGRect,
    ) -> objc2_core_foundation::CGRect {
        let Some(record) = self.windows.get_mut(&window) else {
            return frame;
        };
        let retained = record.floating_display_geometry;
        let (intended, viewport) = retained.map_or((frame, source), |intent| {
            (intent.intended, Some(intent.viewport))
        });
        let minimum = record.state.as_ref().map_or_else(Default::default, |state| {
            if state.info.is_resizable {
                state.info.min_size.unwrap_or_default()
            } else {
                intended.size
            }
        });
        let projected = crate::sys::geometry::frame_on_display(intended, viewport, target, minimum);
        record.floating_display_geometry = viewport
            .filter(|_| projected != intended)
            .map(|viewport| FloatingDisplayGeometry { intended, viewport });
        projected
    }

    /// Accepted user geometry supersedes any temporary display projection.
    pub(crate) fn clear_floating_display_geometry(&mut self, window: WindowId) {
        if let Some(record) = self.windows.get_mut(&window) {
            record.floating_display_geometry = None;
        }
    }

    pub fn record(&self, window_id: WindowId) -> Option<RecordRef<'_>> {
        Some(RecordRef {
            record: self.windows.get(&window_id)?,
            transfer: self.windows.transfer(&window_id),
        })
    }

    pub fn contains_window(&self, window_id: WindowId) -> bool { self.window(window_id).is_some() }

    pub fn tracked_window_count(&self) -> usize {
        self.windows.values().filter(|record| record.state.is_some()).count()
    }

    pub(crate) fn iter_windows(&self) -> impl Iterator<Item = (WindowId, &WindowState)> + '_ {
        self.windows.iter().filter_map(|(&window_id, record)| {
            record.state.as_ref().map(|state| (window_id, state))
        })
    }

    pub fn window_ids_for_pid(&self, pid: i32) -> impl Iterator<Item = WindowId> + '_ {
        self.app_windows
            .get(&pid)
            .into_iter()
            .flat_map(|windows| windows.iter().copied())
    }

    pub fn iter_window_server_ids(&self) -> impl Iterator<Item = WindowServerId> + '_ {
        self.window_servers.keys().copied()
    }

    pub fn iter_tracked_window_server_ids(&self) -> impl Iterator<Item = WindowServerId> + '_ {
        self.window_servers
            .iter()
            .filter_map(|(&wsid, record)| record.window_id.map(|_| wsid))
    }

    pub fn iter_visible_window_server_ids(&self) -> impl Iterator<Item = WindowServerId> + '_ {
        self.window_servers
            .iter()
            .filter_map(|(&wsid, record)| record.visible.then_some(wsid))
    }

    pub fn window_server_info_count(&self) -> usize {
        self.window_servers.values().filter(|record| record.info.is_some()).count()
    }

    pub fn visible_window_server_count(&self) -> usize {
        self.window_servers.values().filter(|record| record.visible).count()
    }

    fn server_record_mut(&mut self, wsid: WindowServerId) -> &mut WindowServerRecord {
        self.window_servers.entry(wsid).or_default()
    }

    fn prune_window_record(&mut self, window_id: WindowId) {
        let should_remove = self.windows.get(&window_id).is_some_and(|record| {
            record.state.is_none()
                && record.window_server_id.is_none()
                && record.workspace.is_none()
                && !record.rule_floating
                && record.claim.is_none()
                // The user's hand keeps its window's record until it lets go.
                && self.windows.gripped_window() != Some(window_id)
        });
        if should_remove {
            self.windows.remove(&window_id);
            if let Some((_, session)) = self.drag_mut() {
                session.forget_target(window_id);
            }
            if let Some(windows) = self.app_windows.get_mut(&window_id.pid) {
                windows.remove(&window_id);
                if windows.is_empty() {
                    self.app_windows.remove(&window_id.pid);
                }
            }
        }
    }

    fn prune_window_server_record(&mut self, wsid: WindowServerId) {
        let should_remove =
            self.window_servers.get(&wsid).is_some_and(WindowServerRecord::is_empty);
        if should_remove {
            self.window_servers.remove(&wsid);
        }
    }

    pub fn tracked_window_id(&self, wsid: WindowServerId) -> Option<WindowId> {
        self.window_servers.get(&wsid).and_then(|record| record.window_id)
    }

    pub fn track_window_server_id(
        &mut self,
        wsid: WindowServerId,
        window_id: WindowId,
    ) -> Option<WindowId> {
        let (old, pending_record) = {
            let record = self.server_record_mut(wsid);
            let old = record.window_id;
            record.window_id = Some(window_id);
            let pending = record.pending_native_fullscreen.take();
            (old, pending)
        };
        if let Some(old) = old.filter(|old| *old != window_id)
            && let Some(record) = self.windows.get_mut(&old)
        {
            record.window_server_id = None;
        }
        if let Some(previous_wsid) = self
            .windows
            .get(&window_id)
            .and_then(|record| record.window_server_id)
            .filter(|previous| *previous != wsid)
            && let Some(record) = self.window_servers.get_mut(&previous_wsid)
        {
            record.window_id = None;
            self.prune_window_server_record(previous_wsid);
        }
        let window = self.windows.entry_or_default(window_id);
        window.window_server_id = Some(wsid);
        self.app_windows.entry(window_id.pid).or_default().insert(window_id);
        if let Some(pending_record) = pending_record {
            if pending_record.pid != window_id.pid {
                self.prune_window_server_record(wsid);
                return old;
            }
            let _ = self.suspend_window_to_native_fullscreen(
                window_id,
                Some(wsid),
                pending_record.last_known_user_space,
                pending_record.fullscreen_space,
                pending_record.transition,
            );
        } else if let Some(original_window_id) = self.native_fullscreen_original_window(window_id)
            && let Some(mut native_record) =
                self.remove_native_fullscreen_record_by_original_window(original_window_id)
        {
            native_record.window_server_id = Some(wsid);
            self.upsert_native_fullscreen_record(native_record);
        }
        self.prune_window_server_record(wsid);
        old
    }

    pub fn track_window_server_info(&mut self, info: WindowServerInfo) -> Option<WindowServerInfo> {
        let record = self.server_record_mut(info.id);
        let old = record.info;
        record.info = Some(info);
        old
    }

    pub fn get_window_server_info(&self, wsid: WindowServerId) -> Option<WindowServerInfo> {
        self.window_servers.get(&wsid).and_then(|record| record.info)
    }

    pub fn knows_window_server_id(&self, wsid: WindowServerId) -> bool {
        self.window_servers.get(&wsid).is_some_and(|record| record.info.is_some())
    }

    fn set_native_visibility(&mut self, wsid: WindowServerId, visible: bool) -> bool {
        let record = if visible {
            self.server_record_mut(wsid)
        } else if let Some(record) = self.window_servers.get_mut(&wsid) {
            record
        } else {
            return false;
        };
        let changed = record.visible != visible;
        record.visible = visible;
        if let Some(wid) = record.window_id {
            self.windows.entry_or_default(wid).visibility = if visible {
                WindowVisibility::Visible
            } else {
                WindowVisibility::Hidden
            };
        } else if !visible {
            record.observed = false;
        }
        self.prune_window_server_record(wsid);
        changed
    }

    pub fn mark_window_visible(&mut self, wsid: WindowServerId) -> bool {
        self.set_native_visibility(wsid, true)
    }

    pub fn mark_window_hidden(&mut self, wsid: WindowServerId) -> bool {
        self.set_native_visibility(wsid, false)
    }

    pub fn clear_visible_windows(&mut self) {
        for record in self.window_servers.values_mut() {
            record.visible = false;
            if let Some(wid) = record.window_id {
                self.windows.entry_or_default(wid).visibility = WindowVisibility::Hidden;
            }
        }
        self.window_servers.retain(|_, record| !record.is_empty());
    }

    pub(crate) fn observe_native_window(&mut self, info: WindowServerInfo) -> Option<WindowId> {
        self.track_window_server_info(info);
        self.mark_window_visible(info.id);
        let wid = self.tracked_window_id(info.id)?;
        self.clear_window_server_observed(info.id);
        let window = self.window_mut(wid)?;
        if info.layer == 0 {
            window.frame_monotonic = info.frame;
        }
        Some(wid)
    }

    pub fn is_window_visible(&self, wsid: WindowServerId) -> bool {
        self.window_servers.get(&wsid).is_some_and(|record| record.visible)
    }

    pub fn mark_window_server_observed(&mut self, wsid: WindowServerId) -> bool {
        let record = self.server_record_mut(wsid);
        let changed = !record.observed;
        record.observed = true;
        changed
    }

    pub fn clear_window_server_observed(&mut self, wsid: WindowServerId) -> bool {
        let changed = self.window_servers.get(&wsid).is_some_and(|record| record.observed);
        if let Some(record) = self.window_servers.get_mut(&wsid) {
            record.observed = false;
        }
        self.prune_window_server_record(wsid);
        changed
    }

    pub fn is_window_server_observed(&self, wsid: WindowServerId) -> bool {
        self.window_servers.get(&wsid).is_some_and(|record| record.observed)
    }

    pub fn has_pending_window_for_pid(&self, pid: i32) -> bool {
        self.window_servers.values().any(|record| {
            record.observed
                && record.visible
                && record.window_id.is_none()
                && record.info.is_some_and(|info| info.pid == pid)
        })
    }

    pub fn set_window_server_space(&mut self, wsid: WindowServerId, space: Option<SpaceId>) {
        let record = self.server_record_mut(wsid);
        record.space = space;
        self.prune_window_server_record(wsid);
    }

    pub fn window_server_space(&self, wsid: WindowServerId) -> Option<SpaceId> {
        self.window_servers.get(&wsid).and_then(|record| record.space)
    }

    pub fn remove_window_server_state(&mut self, wsid: WindowServerId) -> Option<WindowId> {
        let wid = self.tracked_window_id(wsid);
        if let Some(record) = self.window_servers.get_mut(&wsid) {
            record.window_id = None;
            record.visible = false;
            record.observed = false;
            record.space = None;
            record.info = None;
        }
        if let Some(wid) = wid
            && let Some(record) = self.windows.get_mut(&wid)
        {
            record.window_server_id = None;
        }
        self.prune_window_server_record(wsid);
        wid
    }

    pub fn suspend_window_to_native_fullscreen(
        &mut self,
        window_id: WindowId,
        window_server_id: Option<WindowServerId>,
        fallback_last_known_user_space: Option<SpaceId>,
        fullscreen_space: SpaceId,
        transition: NativeFullscreenTransition,
    ) -> NativeFullscreenRecord {
        let original_window_id =
            self.native_fullscreen_original_window(window_id).unwrap_or(window_id);
        let existing = self
            .native_fullscreen_records_by_original_window
            .get(&original_window_id)
            .copied();
        let workspace = self.workspace_for_window(window_id);
        let record = NativeFullscreenRecord {
            original_window_id,
            current_window_id: window_id,
            window_server_id: window_server_id
                .or_else(|| existing.and_then(|record| record.window_server_id)),
            workspace: workspace.or_else(|| existing.and_then(|record| record.workspace)),
            last_known_user_space: fallback_last_known_user_space
                .or_else(|| existing.and_then(|record| record.last_known_user_space)),
            fullscreen_space,
            transition,
        };
        self.windows.entry_or_default(window_id);
        self.app_windows.entry(window_id.pid).or_default().insert(window_id);
        self.upsert_native_fullscreen_record(record)
    }

    pub fn suspend_window_server_to_native_fullscreen(
        &mut self,
        pid: i32,
        window_server_id: WindowServerId,
        fallback_last_known_user_space: Option<SpaceId>,
        fullscreen_space: SpaceId,
        transition: NativeFullscreenTransition,
    ) -> PendingNativeFullscreenRecord {
        let state = {
            let existing = self
                .window_servers
                .get(&window_server_id)
                .and_then(|record| record.pending_native_fullscreen);
            PendingNativeFullscreenState {
                pid,
                last_known_user_space: fallback_last_known_user_space
                    .or_else(|| existing.and_then(|record| record.last_known_user_space)),
                fullscreen_space,
                transition,
            }
        };
        self.server_record_mut(window_server_id).pending_native_fullscreen = Some(state);
        PendingNativeFullscreenRecord {
            pid: state.pid,
            window_server_id,
            last_known_user_space: state.last_known_user_space,
            fullscreen_space: state.fullscreen_space,
            transition: state.transition,
        }
    }

    pub fn native_fullscreen_record_for_window(
        &self,
        window_id: WindowId,
    ) -> Option<NativeFullscreenRecord> {
        let original_window_id = self.native_fullscreen_original_window(window_id)?;
        self.native_fullscreen_records_by_original_window
            .get(&original_window_id)
            .copied()
    }

    pub fn native_fullscreen_record_for_window_server_id(
        &self,
        wsid: WindowServerId,
    ) -> Option<NativeFullscreenRecord> {
        let original_window_id =
            self.native_fullscreen_original_window_by_window_server.get(&wsid).copied()?;
        self.native_fullscreen_records_by_original_window
            .get(&original_window_id)
            .copied()
    }

    pub fn pending_native_fullscreen_record_for_window_server_id(
        &self,
        wsid: WindowServerId,
    ) -> Option<PendingNativeFullscreenRecord> {
        self.window_servers
            .get(&wsid)
            .and_then(|record| record.pending_native_fullscreen)
            .map(|record| PendingNativeFullscreenRecord {
                pid: record.pid,
                window_server_id: wsid,
                last_known_user_space: record.last_known_user_space,
                fullscreen_space: record.fullscreen_space,
                transition: record.transition,
            })
    }

    pub fn iter_native_fullscreen_records(
        &self,
    ) -> impl Iterator<Item = NativeFullscreenRecord> + '_ {
        self.native_fullscreen_records_by_original_window.values().copied()
    }

    /// Absence is inconclusive. Exit evidence is positive user membership or a
    /// corroborated destruction, and either counts only once the fullscreen Space
    /// is gone: a window listed beside its live fullscreen Space is transitional.
    pub(crate) fn note_fullscreen_exits(
        &mut self,
        existing: &HashSet<SpaceId>,
        destroyed: &HashSet<SpaceId>,
        membership: &HashMap<WindowServerId, SpaceId>,
    ) {
        let exited = |wsid: Option<WindowServerId>, space: SpaceId| {
            !existing.contains(&space)
                && (wsid.is_some_and(|id| membership.contains_key(&id))
                    || destroyed.contains(&space))
        };
        for record in self.native_fullscreen_records_by_original_window.values_mut() {
            if exited(record.window_server_id, record.fullscreen_space) {
                let observed = record.window_server_id.and_then(|id| membership.get(&id).copied());
                record.transition = NativeFullscreenTransition::ExitObserved(observed.or(
                    match record.transition {
                        NativeFullscreenTransition::ExitObserved(space) => space,
                        _ => None,
                    },
                ));
            }
        }
        for (&id, server) in &mut self.window_servers {
            if let Some(record) = server.pending_native_fullscreen.as_mut()
                && exited(Some(id), record.fullscreen_space)
            {
                record.transition =
                    NativeFullscreenTransition::ExitObserved(membership.get(&id).copied());
            }
        }
    }

    pub(crate) fn note_window_server_fullscreen_exit(
        &mut self,
        wsid: WindowServerId,
        space: SpaceId,
    ) {
        if let Some(record) = self.native_fullscreen_record_for_window_server_id(wsid) {
            self.note_fullscreen_exit(record.current_window_id, space);
        }
        if let Some(record) = self
            .window_servers
            .get_mut(&wsid)
            .and_then(|server| server.pending_native_fullscreen.as_mut())
        {
            record.transition = NativeFullscreenTransition::ExitObserved(Some(space));
        }
    }

    /// AX destruction during native fullscreen can be an identity replacement.
    /// Retain ownership, classification, transfer intent and the native binding.
    pub(crate) fn suspend_fullscreen_ax_identity(&mut self, window: WindowId) -> bool {
        if self.native_fullscreen_record_for_window(window).is_none() {
            return false;
        }
        if let Some(record) = self.windows.get_mut(&window) {
            record.state = None;
        }
        true
    }

    pub(crate) fn note_fullscreen_exit(&mut self, window: WindowId, space: SpaceId) {
        if let Some(original) = self.native_fullscreen_original_window(window) {
            self.native_fullscreen_records_by_original_window
                .get_mut(&original)
                .unwrap()
                .transition = NativeFullscreenTransition::ExitObserved(Some(space));
        }
    }

    pub fn restore_window_from_native_fullscreen(
        &mut self,
        window_id: WindowId,
    ) -> Option<NativeFullscreenRecord> {
        let original_window_id = self.native_fullscreen_original_window(window_id)?;
        let record = self.remove_native_fullscreen_record_by_original_window(original_window_id)?;
        Some(record)
    }

    pub fn is_window_native_fullscreen_suspended(&self, window_id: WindowId) -> bool {
        self.native_fullscreen_record_for_window(window_id).is_some_and(|record| {
            matches!(
                record.transition,
                NativeFullscreenTransition::Suspended | NativeFullscreenTransition::ExitObserved(_)
            )
        })
    }

    pub fn is_window_server_id_native_fullscreen_suspended(&self, wsid: WindowServerId) -> bool {
        self.native_fullscreen_record_for_window_server_id(wsid).is_some_and(|record| {
            matches!(
                record.transition,
                NativeFullscreenTransition::Suspended | NativeFullscreenTransition::ExitObserved(_)
            )
        })
    }

    pub fn pending_native_fullscreen_pid_for_window_server_id(
        &self,
        wsid: WindowServerId,
    ) -> Option<i32> {
        self.pending_native_fullscreen_record_for_window_server_id(wsid)
            .map(|record| record.pid)
    }

    pub fn assign_window_to_workspace(
        &mut self,
        window_id: WindowId,
        assignment: VirtualWorkspaceId,
    ) -> Option<VirtualWorkspaceId> {
        // A claimed window never enters the index; any placement updates its destination.
        if let Some(claim) = self.windows.get_mut(&window_id).and_then(|r| r.claim.as_mut()) {
            return claim.workspace.replace(assignment);
        }
        let old = self.windows.get(&window_id).and_then(|record| record.workspace);
        if old == Some(assignment) {
            return old;
        }
        if let Some(old_assignment) = old {
            self.remove_window_from_workspace_index(window_id, old_assignment);
        }
        self.windows.entry_or_default(window_id).workspace = Some(assignment);
        self.app_windows.entry(window_id.pid).or_default().insert(window_id);
        self.add_window_to_workspace_index(window_id, assignment);
        if let Some(original_window_id) = self.native_fullscreen_original_window(window_id)
            && let Some(mut record) =
                self.remove_native_fullscreen_record_by_original_window(original_window_id)
        {
            record.workspace = Some(assignment);
            self.upsert_native_fullscreen_record(record);
        }
        old
    }

    pub fn workspace_for_window(&self, window_id: WindowId) -> Option<VirtualWorkspaceId> {
        self.windows.get(&window_id).and_then(|record| record.workspace)
    }

    pub fn workspace_windows(&self, workspace_id: VirtualWorkspaceId) -> Vec<WindowId> {
        let mut windows: Vec<_> = self
            .workspace_windows
            .get(&workspace_id)
            .into_iter()
            .flat_map(|windows| windows.iter().copied())
            .collect();
        windows.sort_unstable_by_key(|wid| (wid.pid, wid.idx.get()));
        windows
    }

    pub fn workspace_window_count(&self, workspace_id: VirtualWorkspaceId) -> usize {
        self.workspace_windows.get(&workspace_id).map_or(0, HashSet::len)
    }

    pub fn remove_window_assignment(&mut self, window_id: WindowId) -> Option<VirtualWorkspaceId> {
        // A claim keeps its saved workspace until release; only workspace removal clears it.
        self.windows.clear_transfer(&window_id);
        let old = self.windows.get_mut(&window_id).and_then(|record| record.workspace.take());
        if let Some(old_assignment) = old {
            self.remove_window_from_workspace_index(window_id, old_assignment);
        }
        self.prune_window_record(window_id);
        old
    }

    /// Move workspace/rule metadata from an old AX window id to a new one when
    /// macOS rekeys the same WindowServer window across sleep/wake or similar
    /// churn. The caller remains responsible for replacing any layout/window
    /// state that still references `from`.
    pub fn transfer_persistent_window_metadata(&mut self, from: WindowId, to: WindowId) {
        if from == to {
            return;
        }

        let (workspace, rule_floating, visibility, geom) = match self.windows.get(&from) {
            Some(record) => (
                record.workspace,
                record.rule_floating,
                record.visibility,
                record.floating_display_geometry,
            ),
            None => return,
        };

        let ax_invalidated = self.windows[&from].ax_invalidated;

        let target_workspace = self.windows.get(&to).and_then(|record| record.workspace);

        if let Some(assignment) = workspace {
            self.remove_window_from_workspace_index(from, assignment);
            if let Some(target_assignment) = target_workspace {
                self.remove_window_from_workspace_index(to, target_assignment);
            }
            self.add_window_to_workspace_index(to, assignment);
        }

        let target = self.windows.entry_or_default(to);
        if workspace.is_some() {
            target.workspace = workspace;
        }
        target.ax_invalidated |= ax_invalidated;
        target.rule_floating |= rule_floating;
        target.visibility = visibility;
        target.floating_display_geometry = geom;
        self.app_windows.entry(to.pid).or_default().insert(to);
        // The user's grip moves with the identity. So does a transfer, unless the replacement
        // is the window under the user's hand.
        if self.windows.gripped_window() == Some(from) {
            self.windows.regrip(from, to);
        } else {
            let transfer = self.windows.transfer(&from);
            if let Some(replacement) = self.windows.loose.get_mut(&to) {
                replacement.transfer = transfer;
            }
        }

        if let Some(source) = self.windows.get_mut(&from) {
            source.workspace = None;
            source.ax_invalidated = false;
            source.rule_floating = false;
            source.floating_display_geometry = None;
        }
        self.windows.clear_transfer(&from);
        if let Some(mut claim) = self.windows.get_mut(&from).and_then(|r| r.claim.take()) {
            self.unindex_claim(claim.owner, from);
            // The recognized identity's claim wins. A claim taken on the provisional
            // replacement ends; its owner's requests then resolve to the surviving claim.
            if let Some(displaced) = self.windows.get_mut(&to).and_then(|r| r.claim.take()) {
                self.unindex_claim(displaced.owner, to);
                claim.workspace = claim.workspace.or(displaced.workspace);
                if displaced.owner != claim.owner {
                    tracing::warn!(
                        ?from,
                        ?to,
                        ?displaced,
                        "Identity merge ended a conflicting claim"
                    );
                }
            }
            self.windows.get_mut(&to).unwrap().claim = Some(claim);
            self.claims.entry(claim.owner).or_default().push(to);
        }
        // A replacement admitted before its predecessor was recognized yields to the claim.
        if self.is_claimed(to) {
            let replacement = self.take_assignment(to);
            let claim = self.windows.get_mut(&to).and_then(|r| r.claim.as_mut()).unwrap();
            claim.workspace = claim.workspace.or(replacement);
        }

        if let Some(original_window_id) = self.native_fullscreen_original_window(from)
            && let Some(mut record) =
                self.remove_native_fullscreen_record_by_original_window(original_window_id)
        {
            if record.current_window_id == from {
                record.current_window_id = to;
            }
            if record.workspace.is_none() {
                record.workspace = workspace;
            }
            if record.last_known_user_space.is_none() {
                record.last_known_user_space = self.native_space(to);
            }
            self.upsert_native_fullscreen_record(record);
        }

        self.prune_window_record(from);
    }

    pub fn replace_rule_floating(&mut self, window_id: WindowId, value: bool) -> bool {
        let record = self.windows.entry_or_default(window_id);
        let previous = std::mem::replace(&mut record.rule_floating, value);
        self.prune_window_record(window_id);
        previous
    }

    pub fn clear_rule_floating(&mut self, window_id: WindowId) {
        if let Some(record) = self.windows.get_mut(&window_id) {
            record.rule_floating = false;
        }
        self.prune_window_record(window_id);
    }

    pub fn rule_floating(&self, window_id: WindowId) -> bool {
        self.windows.get(&window_id).is_some_and(|record| record.rule_floating)
    }

    pub fn remove_window(&mut self, window_id: WindowId) -> Option<WindowRecord> {
        let Some(record) = self.windows.remove(&window_id) else {
            return None;
        };
        // A drag holding the window ends with it; one elsewhere can no longer drop onto it.
        if let Some((_, session)) = self.drag_mut() {
            session.forget_target(window_id);
        }
        if let Some(claim) = record.claim {
            self.unindex_claim(claim.owner, window_id);
        }
        if let Some(assignment) = record.workspace {
            self.remove_window_from_workspace_index(window_id, assignment);
        }
        if let Some(wsid) = record.window_server_id {
            self.remove_window_server_state(wsid);
        }
        if let Some(windows) = self.app_windows.get_mut(&window_id.pid) {
            windows.remove(&window_id);
            if windows.is_empty() {
                self.app_windows.remove(&window_id.pid);
            }
        }
        Some(record)
    }

    pub fn remove_windows_for_app(&mut self, pid: i32) {
        let window_ids: Vec<_> =
            self.app_windows.get(&pid).into_iter().flatten().copied().collect();
        for window_id in window_ids {
            self.remove_window(window_id);
        }

        let fullscreen_keys: Vec<_> = self
            .native_fullscreen_records_by_original_window
            .keys()
            .copied()
            .filter(|window_id| window_id.pid == pid)
            .collect();
        for original_window_id in fullscreen_keys {
            let _ = self.remove_native_fullscreen_record_by_original_window(original_window_id);
        }

        self.window_servers.retain(|_, record| {
            if record.info.is_some_and(|info| info.pid == pid) {
                return false;
            }
            if record.pending_native_fullscreen.is_some_and(|pending| pending.pid == pid) {
                record.pending_native_fullscreen = None;
            }
            !record.is_empty()
        });
    }

    pub fn iter_workspace_assignments(
        &self,
    ) -> impl Iterator<Item = (WindowId, VirtualWorkspaceId)> + '_ {
        self.windows.iter().filter_map(|(&window_id, record)| {
            record.workspace.map(|workspace| (window_id, workspace))
        })
    }

    pub fn workspace_assignment_count(&self) -> usize {
        self.windows.values().filter(|record| record.workspace.is_some()).count()
    }

    /// Native desktop replacement retires a catalog identity, including history
    /// whose AX window is currently suspended in native fullscreen.
    pub(crate) fn remap_workspace(
        &mut self,
        old: VirtualWorkspaceId,
        replacement: Option<VirtualWorkspaceId>,
    ) {
        for window in self.workspace_windows(old) {
            if let Some(id) = replacement {
                self.assign_window_to_workspace(window, id);
            } else {
                self.remove_window_assignment(window);
            }
        }
        for record in self.native_fullscreen_records_by_original_window.values_mut() {
            if record.workspace == Some(old) {
                record.workspace = replacement;
            }
        }
        for wid in self.claims.values().flatten() {
            if let Some(claim) = self.windows.get_mut(wid).and_then(|r| r.claim.as_mut())
                && claim.workspace == Some(old)
            {
                claim.workspace = replacement;
            }
        }
    }

    pub fn remap_space(&mut self, old_space: SpaceId, new_space: SpaceId) {
        if old_space == new_space {
            return;
        }

        for operation in self.windows.transfers_mut() {
            if operation.requested_space == old_space {
                operation.requested_space = new_space;
            }
        }
        for record in self.native_fullscreen_records_by_original_window.values_mut() {
            if let NativeFullscreenTransition::ExitObserved(Some(space)) = &mut record.transition
                && *space == old_space
            {
                *space = new_space;
            }
            if record.last_known_user_space == Some(old_space) {
                record.last_known_user_space = Some(new_space);
            }
        }
        for record in self.window_servers.values_mut() {
            if let Some(pending) = record.pending_native_fullscreen.as_mut() {
                if pending.last_known_user_space == Some(old_space) {
                    pending.last_known_user_space = Some(new_space);
                }
                if let NativeFullscreenTransition::ExitObserved(Some(space)) =
                    &mut pending.transition
                    && *space == old_space
                {
                    *space = new_space;
                }
            }
            if record.space == Some(old_space) {
                record.space = Some(new_space);
            }
        }
    }

    pub fn native_space(&self, window: WindowId) -> Option<SpaceId> {
        self.window_server_space(self.record(window)?.window_server_id?)
    }

    pub fn set_visibility(&mut self, window_id: WindowId, visibility: WindowVisibility) {
        self.windows.entry_or_default(window_id).visibility = visibility;
        self.app_windows.entry(window_id.pid).or_default().insert(window_id);
    }

    pub(crate) fn can_place_window(&self, window: WindowId) -> bool {
        self.native_fullscreen_record_for_window(window).is_none()
            && self.record(window).is_none_or(|record| {
                !matches!(
                    record.visibility,
                    WindowVisibility::Hidden | WindowVisibility::Minimized
                )
            })
    }

    /// The drag holding `window`, if the user is dragging it.
    pub(crate) fn drag_on(&self, window: WindowId) -> Option<&Session> {
        self.drag().filter(|(held, _)| *held == window).map(|(_, session)| session)
    }

    pub(crate) fn drag_on_mut(&mut self, window: WindowId) -> Option<&mut Session> {
        self.drag_mut().filter(|(held, _)| *held == window).map(|(_, session)| session)
    }

    /// Tests have no window server: the tracked window whose known frame contains `point`.
    #[cfg(test)]
    pub(crate) fn window_at(&self, point: objc2_core_foundation::CGPoint) -> Option<WindowId> {
        use crate::sys::geometry::CGRectExt;
        self.windows
            .iter()
            .filter(|(_, record)| record.frame().is_some_and(|frame| frame.contains(point)))
            .map(|(window, _)| *window)
            .min_by_key(|window| (window.pid, window.idx.get()))
    }

    /// What the pointer is doing, for whoever has a rule about it.
    pub(crate) fn pointer(&self) -> Pointer {
        match &self.windows.gripped {
            None => Pointer::Idle,
            Some(Gripped { grip: Grip::Pressed { .. }, .. }) => Pointer::Pressing,
            Some(Gripped { grip: Grip::Dragging(_), .. }) => Pointer::Dragging,
        }
    }

    /// Whether the user's hand is on `window`: pressed, or dragging it.
    pub(crate) fn is_gripped(&self, window: WindowId) -> bool { self.gripped() == Some(window) }

    /// The window the user's hand is on, pressed or dragged.
    pub(crate) fn gripped(&self) -> Option<WindowId> { self.windows.gripped_window() }

    /// `button` went down on `window`, which is tracked. One pointer: whatever it held before
    /// is let go.
    pub(crate) fn press(&mut self, window: WindowId, button: MouseButton) {
        self.end_drag();
        self.windows.press(window, button);
    }

    /// Tests start a drag the only way there is: a press on the window, then its drag.
    #[cfg(test)]
    pub(crate) fn press_and_drag(
        &mut self,
        window: WindowId,
        button: MouseButton,
        session: Session,
    ) {
        self.press(window, button);
        if let Some(Hand::Pressed(pressed)) = self.hand_on(window) {
            pressed.start(session);
        }
    }

    /// What the user's hand is doing with `window`; `None` when it isn't on it.
    pub(crate) fn hand_on(&mut self, window: WindowId) -> Option<Hand<'_>> {
        let gripped = self.windows.gripped.as_mut().filter(|gripped| gripped.window == window)?;
        Some(match &mut gripped.grip {
            Grip::Dragging(session) => Hand::Dragging(session),
            grip @ Grip::Pressed { .. } => Hand::Pressed(Pressed(grip)),
        })
    }

    /// Whether the pointer is down on `window` with no drag started.
    pub(crate) fn is_pressed(&self, window: WindowId) -> bool {
        self.is_gripped(window) && self.drag().is_none()
    }

    /// The user's drag and the window it holds.
    pub(crate) fn drag(&self) -> Option<(WindowId, &Session)> {
        match &self.windows.gripped {
            Some(Gripped {
                window,
                grip: Grip::Dragging(session),
                ..
            }) => Some((*window, session)),
            Some(Gripped { grip: Grip::Pressed { .. }, .. }) | None => None,
        }
    }

    pub(crate) fn drag_mut(&mut self) -> Option<(WindowId, &mut Session)> {
        match &mut self.windows.gripped {
            Some(Gripped {
                window,
                grip: Grip::Dragging(session),
                ..
            }) => Some((*window, session)),
            Some(Gripped { grip: Grip::Pressed { .. }, .. }) | None => None,
        }
    }

    /// The user's hand comes off whatever it was on, without a drop (a cancel, Mission
    /// Control, the window leaving Rift's control). A drag's session is returned.
    pub(crate) fn end_drag(&mut self) -> Option<(WindowId, Session)> {
        let (window, session) = self.windows.let_go()?;
        self.prune_window_record(window);
        Some((window, session?))
    }

    /// The button that is down on the gripped window.
    pub(crate) fn grip_button(&self) -> Option<MouseButton> {
        Some(match &self.windows.gripped.as_ref()?.grip {
            Grip::Pressed { button, .. } => *button,
            Grip::Dragging(session) => session.button,
        })
    }

    /// The pointer was released: the press or drag ends only on the button that made it.
    pub(crate) fn finish_drag(
        &mut self,
        PointerReleased(button): PointerReleased,
    ) -> Option<(WindowId, Session)> {
        if self.grip_button()? != button {
            return None;
        }
        self.end_drag()
    }

    /// A window left Rift's control: a press or drag on it ends, and it is no longer a drop
    /// target.
    pub(crate) fn forget_drag_window(&mut self, window: WindowId) {
        if self.is_gripped(window) {
            self.end_drag();
        } else if let Some((_, session)) = self.drag_mut() {
            session.forget_target(window);
        }
    }

    /// `window` as Rift may place it, tracking it if need be; `None` while the user's hand is
    /// on it: the slot it is in has no transfer.
    pub(crate) fn unheld(&mut self, window: WindowId) -> Option<Unheld<'_>> {
        let placeable = self.can_place_window(window);
        let loose = self.windows.loose_or_new(window)?;
        self.app_windows.entry(window.pid).or_default().insert(window);
        Some(Unheld {
            frame: loose.record.frame(),
            transfer: &mut loose.transfer,
            placeable,
            topology_revision: self.topology_revision,
            generation: &mut self.operation_generation,
        })
    }

    /// A new display set or Space identity is a new chance; unchanged topology never is.
    pub(crate) fn resume_deferred_transfers(&mut self) {
        for operation in self.windows.transfers_mut() {
            if operation.status == TransferStatus::Deferred {
                operation.status = TransferStatus::Pending;
                operation.attempts = 0;
            }
        }
    }

    /// Only an accepted native membership snapshot can complete a transfer.
    pub(crate) fn confirm_transfer(
        &mut self,
        window_id: WindowId,
        generation: u64,
        revision: u64,
        observed: Option<SpaceId>,
    ) -> bool {
        let confirmed = self.windows.transfer(&window_id).is_some_and(|operation| {
            operation.generation == generation
                && revision >= self.topology_revision
                && revision >= operation.started_revision
                && Some(operation.requested_space) == observed
                && operation.last_revision.is_none_or(|last| revision >= last)
        });
        if confirmed {
            self.windows.clear_transfer(&window_id);
        }
        confirmed
    }

    #[cfg(any(test, debug_assertions))]
    pub fn debug_assert_invariants(&self) {
        for (&wid, record) in self.windows.iter() {
            debug_assert!(self.app_windows.get(&wid.pid).is_some_and(|ids| ids.contains(&wid)));
            if let Some(wsid) = record.window_server_id {
                debug_assert_eq!(self.tracked_window_id(wsid), Some(wid));
            }
            if let Some(workspace) = record.workspace {
                let indexed =
                    self.workspace_windows.get(&workspace).is_some_and(|ids| ids.contains(&wid));
                debug_assert!(indexed);
            }
            if let Some(claim) = record.claim {
                debug_assert!(record.workspace.is_none());
                debug_assert!(self.owner_claims(claim.owner).contains(&wid));
            }
        }
        for (&owner, ids) in &self.claims {
            debug_assert!(!ids.is_empty());
            debug_assert!(ids.iter().all(|wid| self.claim(*wid).is_some_and(|c| c.owner == owner)));
        }
        for (&pid, ids) in &self.app_windows {
            debug_assert!(ids.iter().all(|wid| wid.pid == pid && self.windows.contains_key(wid)));
        }
        for (&wsid, server) in &self.window_servers {
            if let Some(wid) = server.window_id {
                debug_assert_eq!(
                    self.windows.get(&wid).and_then(|record| record.window_server_id),
                    Some(wsid)
                );
            }
        }
        for (workspace, ids) in &self.workspace_windows {
            debug_assert!(ids.iter().all(|wid| {
                self.windows.get(wid).is_some_and(|record| {
                    record.workspace.is_some_and(|assignment| assignment == *workspace)
                })
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::virtual_workspace::WorkspaceStore;

    #[test]
    fn transfer_budget_deferral_supersession_and_authoritative_confirmation() {
        let mut store = WindowStore::default();
        let window = WindowId::new(42, 1);
        let (a, b) = (SpaceId::new(1), SpaceId::new(2));
        let frame = Some(objc2_core_foundation::CGRect::ZERO);
        store.accept_topology_revision(10);
        assert!(store.unheld(window).unwrap().request_transfer(frame, a, true));
        let first = store.record(window).unwrap().pending_transfer().unwrap();
        assert!(!store.unheld(window).unwrap().request_transfer(frame, a, true));
        for (revision, observed) in [(9, Some(a)), (11, None), (11, Some(b))] {
            assert!(!store.confirm_transfer(window, first.generation, revision, observed));
        }
        for (revision, writes) in [
            (11, true),
            (11, false),
            (12, true),
            (13, false),
            (14, false),
        ] {
            assert_eq!(
                store.unheld(window).unwrap().advance_transfer(revision, true).is_some(),
                writes
            );
        }
        assert_eq!(
            store.record(window).unwrap().pending_transfer().unwrap().status,
            TransferStatus::Deferred
        );
        assert!(!store.unheld(window).unwrap().request_transfer(frame, a, true));
        assert!(!store.unheld(window).unwrap().request_transfer(frame, b, false));
        let second = store.record(window).unwrap().pending_transfer().unwrap();
        assert!(second.generation > first.generation);
        assert!(!store.confirm_transfer(window, first.generation, 15, Some(a)));
        assert!(store.unheld(window).unwrap().advance_transfer(15, true).is_some());
        assert!(!store.confirm_transfer(window, second.generation, 14, Some(b)));
        assert!(store.confirm_transfer(window, second.generation, 16, Some(b)));
        assert!(store.record(window).unwrap().pending_transfer().is_none());
        store.accept_topology_revision(20);
        store.unheld(window).unwrap().request_transfer(frame, a, true);
        let current = store.record(window).unwrap().pending_transfer().unwrap();
        assert!(!store.confirm_transfer(window, current.generation, 19, Some(a)));
        assert!(store.confirm_transfer(window, current.generation, 20, Some(a)));
    }

    #[test]
    fn app_termination_clears_stale_pending_fullscreen_without_erasing_another_pid() {
        let mut store = WindowStore::default();
        let wsid = WindowServerId::new(42);
        store.track_window_server_info(WindowServerInfo {
            id: wsid,
            pid: 8,
            layer: 0,
            frame: Default::default(),
            min_frame: Default::default(),
            max_frame: Default::default(),
        });
        store.suspend_window_server_to_native_fullscreen(
            7,
            wsid,
            Some(SpaceId::new(1)),
            SpaceId::new(99),
            NativeFullscreenTransition::Suspended,
        );
        store.remove_windows_for_app(7);
        assert_eq!(store.get_window_server_info(wsid).unwrap().pid, 8);
        assert!(store.pending_native_fullscreen_record_for_window_server_id(wsid).is_none());
        store.debug_assert_invariants();
    }

    #[test]
    fn pending_registration_requires_visible_unmapped_window_for_pid() {
        let mut store = WindowStore::default();
        let wsid = WindowServerId::new(77);
        store.track_window_server_info(WindowServerInfo {
            id: wsid,
            pid: 2,
            layer: 0,
            frame: Default::default(),
            min_frame: Default::default(),
            max_frame: Default::default(),
        });
        store.mark_window_server_observed(wsid);
        assert!(!store.has_pending_window_for_pid(2));
        store.mark_window_visible(wsid);
        assert!(store.has_pending_window_for_pid(2));
        assert!(!store.has_pending_window_for_pid(3));
        store.track_window_server_id(wsid, WindowId::new(2, 1));
        assert!(!store.has_pending_window_for_pid(2));
    }

    #[test]
    fn authoritative_space_only_record_is_not_pruned() {
        let mut window_store = WindowStore::default();
        let wsid = WindowServerId::new(77);
        let space = SpaceId::new(9);

        window_store.set_window_server_space(wsid, Some(space));

        assert_eq!(window_store.window_server_space(wsid), Some(space));
        assert_eq!(window_store.iter_window_server_ids().collect::<Vec<_>>(), vec![
            wsid
        ]);
    }

    #[test]
    fn authoritative_space_record_is_pruned_when_space_is_cleared() {
        let mut window_store = WindowStore::default();
        let wsid = WindowServerId::new(78);

        window_store.set_window_server_space(wsid, Some(SpaceId::new(10)));
        window_store.set_window_server_space(wsid, None);

        assert_eq!(window_store.window_server_space(wsid), None);
        assert!(window_store.iter_window_server_ids().next().is_none());
    }

    fn admitted(wid: WindowId, wsid: WindowServerId) -> WindowState {
        let mut state = WindowState::from(crate::sys::app::WindowInfo {
            has_native_tabs: false,
            is_standard: true,
            is_root: true,
            is_minimized: false,
            is_resizable: true,
            min_size: None,
            max_size: None,
            title: format!("{wid:?}"),
            frame: objc2_core_foundation::CGRect::ZERO,
            sys_id: Some(wsid),
            bundle_id: None,
            path: None,
            ax_role: None,
            ax_subrole: None,
        });
        state.is_manageable = true;
        state
    }

    #[test]
    fn minimized_membership_survives_reconciliation_until_permanent_removal() {
        let mut store = WindowStore::default();
        let mut workspaces = WorkspaceStore::new();
        let home = workspaces.create_workspace(SpaceId::new(10), None).unwrap();
        let wid = WindowId::new(1, 1);
        store.insert_window(wid, admitted(wid, WindowServerId::new(1)));
        store.assign_window_to_workspace(wid, home);

        for _ in 0..2 {
            assert!(store.observe_minimized(wid));
            assert!(!store.reconcile_admission(wid));
            assert_eq!(store.workspace_for_window(wid), Some(home));
            assert_eq!(store.workspace_windows(home), vec![wid]);
            store.debug_assert_invariants();
        }

        store.insert_window(wid, admitted(wid, WindowServerId::new(1)));
        assert!(store.reconcile_admission(wid));
        assert_eq!(store.workspace_for_window(wid), Some(home));
        store.window_mut(wid).unwrap().manage_override = Some(false);
        assert!(!store.reconcile_admission(wid));
        assert_eq!(store.workspace_for_window(wid), None);
        assert!(store.workspace_windows(home).is_empty());
    }

    #[test]
    fn claims_hold_assignment_outside_the_index_and_follow_identity() {
        let mut store = WindowStore::default();
        let space = SpaceId::new(10);
        let mut workspaces = WorkspaceStore::new();
        let [home, moved, replaced] = ["Home", "Moved", "Replaced"]
            .map(|name| workspaces.create_workspace(space, Some(name.into())).unwrap());
        let (owner, other) = (ClaimOwner(7), ClaimOwner(8));
        let (wid, rekeyed, sibling) =
            (WindowId::new(1, 1), WindowId::new(1, 2), WindowId::new(1, 3));
        store.insert_window(wid, admitted(wid, WindowServerId::new(1)));
        store.insert_window(sibling, admitted(sibling, WindowServerId::new(3)));
        store.assign_window_to_workspace(wid, home);
        store.assign_window_to_workspace(sibling, home);

        store.insert_claim(wid, owner, WindowClaimFlags::empty());
        assert!(!store.is_admitted(wid) && !store.can_reconcile_admission(wid));
        assert!(
            store.window(wid).unwrap().is_admitted(),
            "native admission is untouched"
        );
        assert_eq!(store.workspace_for_window(wid), None);
        assert_eq!(store.workspace_windows(home), vec![sibling]);
        assert_eq!(store.iter_workspace_assignments().count(), 1);
        // Placement while claimed only updates the destination.
        assert_eq!(store.assign_window_to_workspace(wid, moved), Some(home));
        assert!(store.workspace_windows(moved).is_empty());
        store.remap_workspace(moved, Some(replaced));
        assert_eq!(store.claim(wid).unwrap().workspace(), Some(replaced));
        store.debug_assert_invariants();

        // An AX rekey carries the claim; a replacement assignment folds into it.
        store.insert_window(rekeyed, admitted(rekeyed, WindowServerId::new(2)));
        store.assign_window_to_workspace(rekeyed, home);
        assert_eq!(
            store.reconcile_ax_identity(rekeyed, Some(WindowServerId::new(1))),
            Some(wid)
        );
        assert_eq!(store.owner_claims(owner), &[rekeyed]);
        assert_eq!(store.claim(rekeyed).unwrap().workspace(), Some(replaced));
        assert_eq!(store.workspace_windows(home), vec![sibling]);
        assert_eq!(store.resolve_claim_target(owner, wid), Some(rekeyed));
        assert_eq!(store.resolve_claim_target(other, wid), None);
        store.debug_assert_invariants();

        let claim = store.remove_claim(rekeyed).unwrap();
        assert_eq!((claim.owner, claim.workspace()), (owner, Some(replaced)));
        assert!(store.is_admitted(rekeyed) && store.owner_claims(owner).is_empty());
    }

    #[test]
    fn claims_survive_id_reuse_and_merged_identities() {
        let mut store = WindowStore::default();
        let space = SpaceId::new(10);
        let mut workspaces = WorkspaceStore::new();
        let [home, provisional] = ["Home", "Provisional"]
            .map(|name| workspaces.create_workspace(space, Some(name.into())).unwrap());
        let (a, b, c) = (ClaimOwner(1), ClaimOwner(2), ClaimOwner(3));
        let id = |idx| WindowId::new(1, idx);
        let wsid = WindowServerId::new;
        let add = |store: &mut WindowStore, idx: u32, server: u32, workspace| {
            store.insert_window(id(idx), admitted(id(idx), wsid(server)));
            store.assign_window_to_workspace(id(idx), workspace);
        };
        let flags = WindowClaimFlags::empty();

        // A's window is rekeyed and its old id reused by an unrelated window B claims.
        add(&mut store, 1, 1, home);
        store.insert_claim(id(1), a, flags);
        add(&mut store, 2, 2, provisional);
        store.reconcile_ax_identity(id(2), Some(wsid(1)));
        add(&mut store, 1, 9, home);
        assert_eq!(store.resolve_claim_target(a, id(1)), Some(id(2)));
        assert_eq!(store.resolve_claim_target(c, id(1)), Some(id(1)));
        store.insert_claim(id(1), b, flags);
        assert_eq!(store.resolve_claim_target(a, id(1)), Some(id(2)));
        assert_eq!(store.resolve_claim_target(b, id(1)), Some(id(1)));
        assert_eq!(store.resolve_claim_target(a, id(2)), Some(id(2)));
        assert_eq!(store.claim(id(2)).unwrap().workspace(), Some(home));
        store.debug_assert_invariants();

        // Both identities of one window claimed by different owners: the recognized
        // identity's claim survives with its original id and workspace; the other ends.
        add(&mut store, 3, 3, provisional);
        store.insert_claim(id(3), a, flags);
        store.reconcile_ax_identity(id(3), Some(wsid(9)));
        let merged = store.claim(id(3)).unwrap();
        assert_eq!((merged.owner, merged.workspace()), (b, Some(home)));
        assert_eq!(store.owner_claims(a), &[id(2)]);
        assert_eq!(store.owner_claims(b), &[id(3)]);
        assert_eq!(store.resolve_claim_target(b, id(1)), Some(id(3)));
        assert_eq!(
            store.resolve_claim_target(a, id(3)),
            Some(id(3)),
            "B's, so rejected"
        );
        store.debug_assert_invariants();

        // The same owner on both identities keeps one claim reachable by either id.
        add(&mut store, 4, 4, provisional);
        store.insert_claim(id(4), a, WindowClaimFlags::SUPPRESS_FOCUS_FOLLOWS_MOUSE);
        store.reconcile_ax_identity(id(4), Some(wsid(1)));
        assert_eq!(store.owner_claims(a), &[id(4)]);
        assert_eq!(store.claim(id(4)).unwrap().flags, flags);
        for requested in [id(1), id(4)] {
            assert_eq!(store.resolve_claim_target(a, requested), Some(id(4)));
        }
        assert_eq!(
            store.resolve_claim_target(a, id(2)),
            None,
            "retired, never claimed"
        );
        store.debug_assert_invariants();

        // A deleted workspace leaves claims to be rehomed on release.
        store.remap_workspace(home, None);
        assert_eq!(store.claim(id(3)).unwrap().workspace(), None);
        assert_eq!(store.claim(id(4)).unwrap().workspace(), None);

        // Destruction ends a claim and its original-id mapping.
        store.remove_window(id(3));
        assert!(store.owner_claims(b).is_empty());
        assert_eq!(store.resolve_claim_target(b, id(1)), None);
        store.debug_assert_invariants();
    }

    #[test]
    fn transfer_persistent_metadata_replaces_existing_target_workspace_assignment() {
        let mut window_store = WindowStore::default();
        let space = SpaceId::new(10);
        let mut workspaces = WorkspaceStore::new();
        let source_workspace = workspaces
            .create_workspace(space, Some("Source".to_string()))
            .expect("source workspace");
        let target_workspace = workspaces
            .create_workspace(space, Some("Target".to_string()))
            .expect("target workspace");
        let from = WindowId::new(1, 1);
        let to = WindowId::new(1, 2);

        window_store.assign_window_to_workspace(from, source_workspace);
        window_store.assign_window_to_workspace(to, target_workspace);

        window_store.transfer_persistent_window_metadata(from, to);

        assert_eq!(window_store.workspace_for_window(to), Some(source_workspace));
        assert!(window_store.workspace_windows(target_workspace).is_empty());
        assert_eq!(window_store.workspace_windows(source_workspace), vec![to]);
    }

    #[test]
    fn transfer_persistent_metadata_rekeys_native_fullscreen_record() {
        let mut window_store = WindowStore::default();
        let space = SpaceId::new(10);
        let fullscreen_space = SpaceId::new(0x400000000 + space.get());
        let mut workspaces = WorkspaceStore::new();
        let workspace_id = workspaces
            .create_workspace(space, Some("Fullscreen".to_string()))
            .expect("workspace");
        let from = WindowId::new(1, 1);
        let to = WindowId::new(1, 2);
        let wsid = WindowServerId::new(77);

        window_store.assign_window_to_workspace(from, workspace_id);
        let _ = window_store.suspend_window_to_native_fullscreen(
            from,
            Some(wsid),
            Some(space),
            fullscreen_space,
            NativeFullscreenTransition::Suspended,
        );

        window_store.transfer_persistent_window_metadata(from, to);

        let record = window_store
            .native_fullscreen_record_for_window(to)
            .expect("fullscreen record should follow rekey");
        assert_eq!(record.current_window_id, to);
        assert_eq!(record.window_server_id, Some(wsid));
        assert_eq!(record.workspace, Some(workspace_id));
        assert_eq!(
            window_store
                .native_fullscreen_record_for_window(from)
                .expect("original key should still resolve the lifecycle")
                .current_window_id,
            to
        );
    }

    #[test]
    fn native_fullscreen_record_preserves_explicit_fallback_user_space_without_assignment() {
        let mut window_store = WindowStore::default();
        let wid = WindowId::new(1, 1);
        let wsid = WindowServerId::new(91);
        let user_space = SpaceId::new(11);
        let fullscreen_space = SpaceId::new(0x400000000 + user_space.get());

        let record = window_store.suspend_window_to_native_fullscreen(
            wid,
            Some(wsid),
            Some(user_space),
            fullscreen_space,
            NativeFullscreenTransition::Suspended,
        );

        assert_eq!(record.workspace, None);
        assert_eq!(record.last_known_user_space, Some(user_space));
        assert_eq!(
            window_store
                .native_fullscreen_record_for_window_server_id(wsid)
                .expect("record should be discoverable by wsid")
                .last_known_user_space,
            Some(user_space)
        );
        window_store.replace_rule_floating(wid, true);
        assert!(
            !window_store.unheld(wid).unwrap().request_transfer(
                Some(objc2_core_foundation::CGRect::ZERO),
                user_space,
                true
            ),
            "rule reapplication must not override native fullscreen suspension"
        );
        window_store.restore_window_from_native_fullscreen(wid).unwrap();
        assert!(window_store.unheld(wid).unwrap().request_transfer(
            Some(objc2_core_foundation::CGRect::ZERO),
            user_space,
            true
        ));
    }

    #[test]
    fn remove_window_preserves_native_fullscreen_record_until_app_cleanup() {
        let mut window_store = WindowStore::default();
        let wid = WindowId::new(7, 1);
        let wsid = WindowServerId::new(92);
        let user_space = SpaceId::new(12);
        let fullscreen_space = SpaceId::new(0x400000000 + user_space.get());

        let frame = objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(100.0, 100.0),
        );
        window_store.insert_window(
            wid,
            WindowState::from(crate::sys::app::WindowInfo {
                has_native_tabs: false,
                is_standard: true,
                is_root: true,
                is_minimized: false,
                is_resizable: true,
                min_size: None,
                max_size: None,
                title: "Window".to_string(),
                frame,
                sys_id: Some(wsid),
                bundle_id: None,
                path: None,
                ax_role: None,
                ax_subrole: None,
            }),
        );
        let _ = window_store.suspend_window_to_native_fullscreen(
            wid,
            Some(wsid),
            Some(user_space),
            fullscreen_space,
            NativeFullscreenTransition::Suspended,
        );

        window_store.remove_window(wid);

        assert!(
            window_store.native_fullscreen_record_for_window(wid).is_some(),
            "transient AX removal should not drop the fullscreen lifecycle record"
        );

        window_store.remove_windows_for_app(wid.pid);

        assert!(
            window_store.native_fullscreen_record_for_window(wid).is_none(),
            "app cleanup should retire the fullscreen lifecycle record"
        );
    }

    #[test]
    fn track_window_server_id_binds_pending_native_fullscreen_record() {
        let mut window_store = WindowStore::default();
        let wid = WindowId::new(8, 1);
        let wsid = WindowServerId::new(93);
        let user_space = SpaceId::new(13);
        let fullscreen_space = SpaceId::new(0x400000000 + user_space.get());

        let pending = window_store.suspend_window_server_to_native_fullscreen(
            wid.pid,
            wsid,
            Some(user_space),
            fullscreen_space,
            NativeFullscreenTransition::Suspended,
        );
        assert_eq!(pending.pid, wid.pid);

        window_store.track_window_server_id(wsid, wid);

        assert!(
            window_store
                .pending_native_fullscreen_record_for_window_server_id(wsid)
                .is_none(),
            "binding AX identity should consume the pending fullscreen record"
        );
        assert_eq!(
            window_store
                .native_fullscreen_record_for_window_server_id(wsid)
                .expect("resolved record should be indexed by wsid")
                .current_window_id,
            wid
        );
    }

    #[test]
    fn track_window_server_id_discards_stale_pending_native_fullscreen_record_on_pid_mismatch() {
        let mut window_store = WindowStore::default();
        let pending_wid = WindowId::new(8, 1);
        let rebound_wid = WindowId::new(9, 1);
        let wsid = WindowServerId::new(94);
        let user_space = SpaceId::new(14);
        let fullscreen_space = SpaceId::new(0x400000000 + user_space.get());

        let pending = window_store.suspend_window_server_to_native_fullscreen(
            pending_wid.pid,
            wsid,
            Some(user_space),
            fullscreen_space,
            NativeFullscreenTransition::Suspended,
        );
        assert_eq!(pending.pid, pending_wid.pid);

        window_store.track_window_server_id(wsid, rebound_wid);

        assert!(
            window_store
                .pending_native_fullscreen_record_for_window_server_id(wsid)
                .is_none(),
            "binding a different app to the wsid should discard stale pending fullscreen state"
        );
        assert!(
            window_store.native_fullscreen_record_for_window_server_id(wsid).is_none(),
            "stale pending fullscreen state must not be rebound onto a different app"
        );
    }

    #[test]
    fn rebinding_server_identity_keeps_both_indexes_unique() {
        let mut store = WindowStore::default();
        let first = WindowId::new(1, 1);
        let second = WindowId::new(1, 2);
        let wsid = WindowServerId::new(44);

        store.track_window_server_id(wsid, first);
        assert_eq!(store.track_window_server_id(wsid, second), Some(first));

        assert_eq!(
            store.record(first).and_then(|record| record.window_server_id()),
            None
        );
        assert_eq!(
            store.record(second).and_then(|record| record.window_server_id()),
            Some(wsid)
        );
        assert_eq!(store.tracked_window_id(wsid), Some(second));
        store.debug_assert_invariants();
    }

    #[test]
    fn native_identity_change_retires_transfer_but_ax_rekey_preserves_it() {
        let mut store = WindowStore::default();
        let (old, replacement) = (WindowId::new(30, 1), WindowId::new(30, 2));
        let server = WindowServerId::new(70);
        let mut workspaces = WorkspaceStore::new();
        let workspace = workspaces.create_workspace(SpaceId::new(8), None).unwrap();
        store.assign_window_to_workspace(old, workspace);
        store.replace_rule_floating(old, true);
        store.track_window_server_id(server, old);
        store.unheld(old).unwrap().request_transfer(None, SpaceId::new(8), true);
        let pending = store.record(old).unwrap().pending_transfer().unwrap();
        assert_eq!(store.reconcile_ax_identity(replacement, Some(server)), Some(old));
        assert_eq!(
            store.record(replacement).unwrap().pending_transfer(),
            Some(pending)
        );
        assert!(store.rule_floating(replacement));
        assert_eq!(store.workspace_windows(workspace), vec![replacement]);
        store.reconcile_ax_identity(replacement, Some(WindowServerId::new(71)));
        assert!(store.record(replacement).unwrap().pending_transfer().is_none());
        store.debug_assert_invariants();
    }

    #[test]
    fn a_gripped_window_is_still_found_everywhere_it_was() {
        // The gripped window's record lives in the slot, not among the loose ones. Lookups,
        // counts, the workspace index and assignment iteration must not notice.
        let mut store = WindowStore::default();
        let (wid, other) = (WindowId::new(4, 1), WindowId::new(4, 2));
        let wsid = WindowServerId::new(55);
        let mut workspaces = WorkspaceStore::new();
        let workspace = workspaces
            .create_workspace(SpaceId::new(9), Some("Slot".to_string()))
            .expect("workspace");
        store.track_window_server_id(wsid, wid);
        store.assign_window_to_workspace(wid, workspace);
        store.assign_window_to_workspace(other, workspace);
        let assignments = |store: &WindowStore| {
            let mut all: Vec<_> = store.iter_workspace_assignments().collect();
            all.sort_by_key(|(window, _)| window.idx.get());
            all
        };
        let before = (
            store.workspace_assignment_count(),
            assignments(&store),
            store.workspace_for_window(wid),
            store.record(wid).unwrap().window_server_id(),
            store.window_ids_for_pid(4).count(),
        );

        store.press(wid, MouseButton::Left);
        assert_eq!(store.gripped(), Some(wid));
        let pressed = (
            store.workspace_assignment_count(),
            assignments(&store),
            store.workspace_for_window(wid),
            store.record(wid).unwrap().window_server_id(),
            store.window_ids_for_pid(4).count(),
        );
        assert_eq!(pressed, before);
        store.debug_assert_invariants();

        // One slot: pressing another window lets go of the first.
        store.press(other, MouseButton::Left);
        assert_eq!(store.gripped(), Some(other));
        assert!(store.unheld(wid).is_some());
        assert!(store.unheld(other).is_none());
        store.debug_assert_invariants();
    }

    #[test]
    fn the_grip_follows_a_rekeyed_window_and_ends_with_a_removed_one() {
        let mut store = WindowStore::default();
        let (old, new) = (WindowId::new(30, 1), WindowId::new(30, 2));
        let mut workspaces = WorkspaceStore::new();
        let workspace = workspaces
            .create_workspace(SpaceId::new(9), Some("Rekey".to_string()))
            .expect("workspace");
        store.assign_window_to_workspace(old, workspace);
        store.press(old, MouseButton::Left);

        store.transfer_persistent_window_metadata(old, new);
        assert_eq!(store.gripped(), Some(new));
        assert_eq!(store.workspace_for_window(new), Some(workspace));
        assert!(store.unheld(new).is_none());
        store.debug_assert_invariants();

        store.remove_window(new);
        assert_eq!(store.gripped(), None);
        assert!(store.record(new).is_none());
        store.debug_assert_invariants();
    }

    #[test]
    fn app_cleanup_removes_all_indexes_and_pending_transfers() {
        let mut store = WindowStore::default();
        let wid = WindowId::new(4, 1);
        let wsid = WindowServerId::new(55);
        let mut workspaces = WorkspaceStore::new();
        let workspace_id = workspaces
            .create_workspace(SpaceId::new(9), Some("Cleanup".to_string()))
            .expect("workspace");
        store.track_window_server_id(wsid, wid);
        store.assign_window_to_workspace(wid, workspace_id);
        store.unheld(wid).unwrap().request_transfer(None, SpaceId::new(9), true);

        store.remove_windows_for_app(wid.pid);

        assert!(store.record(wid).is_none());
        assert_eq!(store.tracked_window_id(wsid), None);
        assert!(store.window_ids_for_pid(wid.pid).next().is_none());
        store.debug_assert_invariants();
    }

    #[test]
    fn remap_space_merges_into_windows_already_on_the_new_space() {
        let mut store = WindowStore::default();
        let (old, new) = (SpaceId::new(1), SpaceId::new(2));
        let mut workspaces = WorkspaceStore::new();
        let workspace_id = workspaces.create_workspace(old, Some("Shared".to_string())).unwrap();
        let (moved, present) = (WindowId::new(5, 1), WindowId::new(5, 2));
        store.assign_window_to_workspace(moved, workspace_id);
        // The window server placed this one on the incoming id before rift heard of it.
        store.assign_window_to_workspace(present, workspace_id);

        store.suspend_window_to_native_fullscreen(
            moved,
            None,
            Some(old),
            SpaceId::new(99),
            NativeFullscreenTransition::ExitObserved(Some(old)),
        );
        store.suspend_window_server_to_native_fullscreen(
            6,
            WindowServerId::new(43),
            Some(old),
            SpaceId::new(100),
            NativeFullscreenTransition::ExitObserved(Some(old)),
        );
        store.remap_space(old, new);

        assert_eq!(store.workspace_for_window(moved), Some(workspace_id));
        assert_eq!(store.workspace_for_window(present), Some(workspace_id));
        let bound = store.native_fullscreen_record_for_window(moved).unwrap();
        let pending = store
            .pending_native_fullscreen_record_for_window_server_id(WindowServerId::new(43))
            .unwrap();
        for (home, transition) in [
            (bound.last_known_user_space, bound.transition),
            (pending.last_known_user_space, pending.transition),
        ] {
            assert_eq!(
                (home, transition),
                (Some(new), NativeFullscreenTransition::ExitObserved(Some(new)))
            );
        }
        assert_eq!(store.workspace_window_count(workspace_id), 2);
    }
}
