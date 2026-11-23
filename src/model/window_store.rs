use rift_core::lifecycle::{
    Admission, AdmissionFacts, FactChange, LifecycleEvent, Manageability, NativeKind, Rule,
    Transition,
};
use rift_protocol::WindowClaimFlags;

use crate::actor::app::WindowId;
use crate::common::collections::{HashMap, HashSet};
use crate::model::VirtualWorkspaceId;
use crate::model::drag::Session;
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

/// How many times a transfer is retried on new topology revisions before Rift stops asking.
const TRANSFER_RETRIES: u8 = 2;

/// Where a move Rift owes a window stands: retry bookkeeping only. The destination is never
/// stored (it is the Space of the window's workspace) and neither is the frame (it is derived
/// from the layout, or the window's floating position, each time it is written).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transfer {
    /// The destination can't be placed now, so nothing has been written. Becoming available
    /// writes it and starts the retry budget.
    Waiting,
    /// Rift wrote the window onto its destination; `retries` more writes followed on new
    /// topology revisions, the last on `revision`.
    Moving { retries: u8, revision: u64 },
    /// macOS kept the window on `on` (the Space last seen, if any) through every retry. Rift
    /// asks no more until the displays or Spaces change; seen anywhere else, the window is
    /// where the user put it.
    Refused { on: Option<SpaceId> },
}

impl Transfer {
    /// Whether the window keeps its assignment though seen on `observed` (`None`: not seen).
    /// While Rift is moving it, yes; once macOS refused, only while it stays where macOS left
    /// it: the user's latest placement wins over a refused move.
    pub fn keeps_assignment(self, observed: Option<SpaceId>) -> bool {
        match self {
            Transfer::Waiting | Transfer::Moving { .. } => true,
            Transfer::Refused { on } => observed.is_none() || observed == on,
        }
    }
}

/// What the user's hand is doing with the one window it is on: the projection of the core's
/// grip (`rift_core`), changed only by applying the core's effects (`RiftState`'s grip
/// methods). The button and the rules (one hand, a drag only from a press, a release only on
/// the grip's button) are the core's.
#[derive(Debug)]
enum Grip {
    /// The window is pressed. A transfer it had was discarded at the press: the release
    /// re-derives one from where the window is and where it is assigned.
    Pressed,
    Dragging(Box<Session>),
}

/// What the pointer is doing with windows: the core's type, on Rift's window ids.
pub(crate) type Pointer = rift_core::Pointer<WindowId>;

/// The user's one hand. [`Records::default`] makes the only one a store has, and it is neither
/// `Clone` nor `Copy`: the entry holding it is the gripped one, so a second gripped entry would
/// need a second hand, which can't exist.
#[derive(Debug)]
struct Hand(());

/// A window's record and, by what is happening to it, what goes with it.
#[derive(Debug)]
enum Entry {
    /// Rift places this window: its data and the transfer in flight, if any.
    Loose {
        record: WindowRecord,
        transfer: Option<Transfer>,
    },
    /// The user's hand is on this window. It has no transfer to request or advance: the user
    /// places this window.
    Gripped {
        record: WindowRecord,
        grip: Grip,
        hand: Hand,
    },
}

impl Entry {
    fn loose(record: WindowRecord) -> Self { Entry::Loose { record, transfer: None } }

    fn record(&self) -> &WindowRecord {
        match self {
            Entry::Loose { record, .. } | Entry::Gripped { record, .. } => record,
        }
    }

    fn record_mut(&mut self) -> &mut WindowRecord {
        match self {
            Entry::Loose { record, .. } | Entry::Gripped { record, .. } => record,
        }
    }

    #[cfg(test)]
    fn is_gripped(&self) -> bool { matches!(self, Entry::Gripped { .. }) }
}

/// Why the store would not take hold of a window: the core grips only after it ungripped, so
/// a refusal means the store and the core disagree, and the core's next effect corrects it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GripRefused {
    /// The hand is already on this window.
    Holding(WindowId),
    /// A re-key, with nothing held.
    NothingHeld,
    /// No entry holds the hand and it is not free: an entry holding it was dropped.
    HandLost,
    /// The store has no record of this window: it does not make one up to hold.
    Unknown(WindowId),
}

/// Every window's record, each in exactly one place: one map, one entry per window. The hand
/// is in the gripped entry or, while nothing is held, in `free`; so one press or drag at a
/// time, a pressed or dragged window can't be pulled back to its assignment (its entry has no
/// transfer), and a drag starts only from a [`Grip::Pressed`].
#[derive(Debug)]
struct Records {
    entries: HashMap<WindowId, Entry>,
    /// The hand while no entry holds it.
    free: Option<Hand>,
}

impl Default for Records {
    fn default() -> Self {
        Records {
            entries: HashMap::default(),
            free: Some(Hand(())),
        }
    }
}

impl Records {
    fn get(&self, window: &WindowId) -> Option<&WindowRecord> {
        self.entries.get(window).map(Entry::record)
    }

    fn get_mut(&mut self, window: &WindowId) -> Option<&mut WindowRecord> {
        self.entries.get_mut(window).map(Entry::record_mut)
    }

    /// The record for `window`, tracked loose if it is new.
    fn entry_or_default(&mut self, window: WindowId) -> &mut WindowRecord {
        self.entries
            .entry(window)
            .or_insert_with(|| Entry::loose(Default::default()))
            .record_mut()
    }

    /// `window`'s transfer slot, tracked loose if it is new; `None` for the gripped window.
    fn loose_or_new(&mut self, window: WindowId) -> Option<&mut Option<Transfer>> {
        match self.entries.entry(window).or_insert_with(|| Entry::loose(Default::default())) {
            Entry::Loose { transfer, .. } => Some(transfer),
            Entry::Gripped { .. } => None,
        }
    }

    /// Removes `window`'s entry: the one place an entry leaves the map (the only
    /// `HashMap::remove`), so a gripped entry's hand always goes back to `free` and can't be
    /// dropped with it. A transfer goes with the entry; a grip is returned.
    fn take(&mut self, window: &WindowId) -> Option<(WindowRecord, Option<Grip>)> {
        match self.entries.remove(window)? {
            Entry::Loose { record, .. } => Some((record, None)),
            Entry::Gripped { record, grip, hand } => {
                self.free = Some(hand);
                Some((record, Some(grip)))
            }
        }
    }

    /// Removes a record. The core ungrips a window that goes away (`Gone`) before its record is
    /// removed; were it not to, the hand still comes back (`take`).
    fn remove(&mut self, window: &WindowId) -> Option<WindowRecord> {
        debug_assert_ne!(
            self.gripped_window(),
            Some(*window),
            "removing the gripped record"
        );
        self.take(window).map(|(record, _)| record)
    }

    fn iter(&self) -> impl Iterator<Item = (&WindowId, &WindowRecord)> {
        self.entries.iter().map(|(window, entry)| (window, entry.record()))
    }

    fn values(&self) -> impl Iterator<Item = &WindowRecord> {
        self.iter().map(|(_, record)| record)
    }

    fn transfer(&self, window: &WindowId) -> Option<Transfer> {
        match self.entries.get(window)? {
            Entry::Loose { transfer, .. } => *transfer,
            Entry::Gripped { .. } => None,
        }
    }

    fn transfers(&self) -> impl Iterator<Item = (WindowId, Transfer)> {
        self.entries.iter().filter_map(|(window, entry)| match entry {
            Entry::Loose { transfer, .. } => Some((*window, (*transfer)?)),
            Entry::Gripped { .. } => None,
        })
    }

    fn transfers_mut(&mut self) -> impl Iterator<Item = &mut Transfer> {
        self.entries.values_mut().filter_map(|entry| match entry {
            Entry::Loose { transfer, .. } => transfer.as_mut(),
            Entry::Gripped { .. } => None,
        })
    }

    /// Sets a loose window's transfer; the gripped window has none.
    fn set_transfer(&mut self, window: &WindowId, to: Option<Transfer>) {
        if let Some(Entry::Loose { transfer, .. }) = self.entries.get_mut(window) {
            *transfer = to;
        }
    }

    /// Forgets a transfer. Only a loose window has one.
    fn clear_transfer(&mut self, window: &WindowId) { self.set_transfer(window, None); }

    /// The window the hand is on, and its grip.
    fn held(&self) -> Option<(WindowId, &Grip)> {
        self.entries.iter().find_map(|(window, entry)| match entry {
            Entry::Gripped { grip, .. } => Some((*window, grip)),
            Entry::Loose { .. } => None,
        })
    }

    fn held_mut(&mut self) -> Option<(WindowId, &mut Grip)> {
        self.entries.iter_mut().find_map(|(window, entry)| match entry {
            Entry::Gripped { grip, .. } => Some((*window, grip)),
            Entry::Loose { .. } => None,
        })
    }

    fn gripped_window(&self) -> Option<WindowId> { self.held().map(|(window, _)| window) }

    /// How many hands the store has: always one, in an entry or free.
    #[cfg(test)]
    fn hands(&self) -> usize {
        usize::from(self.free.is_some())
            + self.entries.values().filter(|entry| entry.is_gripped()).count()
    }

    /// The hand takes `window` with `grip`, its record being `record`; refused while it is on
    /// a window.
    fn hold(
        &mut self,
        window: WindowId,
        record: WindowRecord,
        grip: Grip,
    ) -> Result<(), GripRefused> {
        let Some(hand) = self.free.take() else {
            return Err(self.busy());
        };
        self.entries.insert(window, Entry::Gripped { record, grip, hand });
        Ok(())
    }

    /// Why the hand can't take a window now.
    fn busy(&self) -> GripRefused {
        self.gripped_window().map_or(GripRefused::HandLost, GripRefused::Holding)
    }

    /// The core gripped `window`: its entry becomes the gripped one, pressed. Its transfer is
    /// discarded, not set aside: the user places this window now, and the release re-derives.
    /// A window with no record is refused, not made up.
    fn grip(&mut self, window: WindowId) -> Result<(), GripRefused> {
        // Refused before the entry is touched: a loose window's record stays where it is.
        if self.free.is_none() {
            return Err(self.busy());
        }
        let (record, _) = self.take(&window).ok_or(GripRefused::Unknown(window))?;
        self.hold(window, record, Grip::Pressed)
    }

    /// The user's hand comes off: the record goes back among the loose ones, with no transfer.
    /// A drag's session is returned.
    fn let_go(&mut self) -> Option<(WindowId, Option<Session>)> {
        let window = self.gripped_window()?;
        let (record, grip) = self.take(&window)?;
        self.entries.insert(window, Entry::loose(record));
        let session = grip.and_then(|grip| match grip {
            Grip::Pressed => None,
            Grip::Dragging(session) => Some(*session),
        });
        Some((window, session))
    }

    /// macOS rekeyed the gripped window: the grip moves to its new identity. The old identity's
    /// record stays, loose; the new one's record is the one held.
    fn regrip(&mut self, from: WindowId, to: WindowId) -> Result<(), GripRefused> {
        let held = self.gripped_window().ok_or(GripRefused::NothingHeld)?;
        debug_assert_eq!(held, from, "the core re-keys its grip");
        let (record, grip) = self.take(&held).ok_or(GripRefused::NothingHeld)?;
        self.entries.insert(held, Entry::loose(record));
        let grip = grip.ok_or(GripRefused::NothingHeld)?;
        let replacement = self.take(&to).map(|(record, _)| record).unwrap_or_default();
        self.hold(to, replacement, grip)
    }
}

impl std::ops::Index<&WindowId> for Records {
    type Output = WindowRecord;

    fn index(&self, window: &WindowId) -> &WindowRecord {
        self.get(window).expect("tracked window")
    }
}

/// A window as Rift may place it: its transfer slot. Only a window the user hasn't pressed
/// has one, so transfers are begun and retried here and nowhere else.
pub(crate) struct Unheld<'a> {
    transfer: &'a mut Option<Transfer>,
    placeable: bool,
    topology_revision: u64,
}

impl Unheld<'_> {
    /// Rift moves the window to its workspace's Space now. True when that is to be written
    /// at once (the destination is placeable); otherwise the transfer waits for it.
    pub(crate) fn begin_transfer(self, available: bool) -> bool {
        let available = available && self.placeable;
        *self.transfer = Some(if available {
            Transfer::Moving {
                retries: 0,
                revision: self.topology_revision,
            }
        } else {
            Transfer::Waiting
        });
        available
    }

    /// An accepted, complete topology `revision` still shows the window off its destination
    /// (on `observed`). True when it is to be written again: only once per revision, with a
    /// fixed budget. A frame acknowledgement never confirms Space membership.
    pub(crate) fn retry_transfer(
        self,
        revision: u64,
        available: bool,
        observed: Option<SpaceId>,
    ) -> bool {
        let Some(transfer) = self.transfer.as_mut() else {
            return false;
        };
        let next = if !(available && self.placeable) {
            Transfer::Waiting
        } else {
            match *transfer {
                Transfer::Refused { .. } => return false,
                Transfer::Moving { revision: last, .. } if revision <= last => return false,
                Transfer::Moving { retries, .. } if retries >= TRANSFER_RETRIES => {
                    Transfer::Refused { on: observed }
                }
                Transfer::Moving { retries, .. } => {
                    Transfer::Moving { retries: retries + 1, revision }
                }
                Transfer::Waiting => Transfer::Moving { retries: 1, revision },
            }
        };
        *transfer = next;
        matches!(next, Transfer::Moving { .. })
    }
}

/// The complete reactor-owned state for one AX window identity.
#[derive(Debug, Default)]
pub struct WindowRecord {
    state: Option<WindowState>,
    window_server_id: Option<WindowServerId>,
    visibility: WindowVisibility,
    rule_floating: bool,
    floating_display_geometry: Option<FloatingDisplayGeometry>,
}

/// Who places a window, in the store's terms: the core's answer (`Windows::ownership`),
/// with a claim read as a [`WindowClaim`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Ownership {
    #[default]
    Unassigned,
    Assigned(VirtualWorkspaceId),
    /// Claimed by an external owner; the claim holds the workspace it is to return to.
    Claimed(WindowClaim),
    /// Under the user's hand and placed by no workspace: a workspace transfer would have carried
    /// it off while it was held. Nothing assigns it until the hand lets go: the drop places it.
    Held,
}

/// The window the user holds, as one of a workspace's windows: what a transfer of that
/// workspace must leave behind ([`Windows::held_member`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HeldMember(WindowId);

impl HeldMember {
    pub(crate) fn window(self) -> WindowId { self.0 }
}

/// Whether a window the hand let go of is placed by a workspace, or by nothing (a workspace
/// transfer left it while it was held): then whoever let go places it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placed {
    ByWorkspace,
    Unplaced,
}

impl Ownership {
    /// The workspace the window is assigned to; `None` unassigned, held or claimed (a claim
    /// holds the workspace it returns to).
    fn assignment(self) -> Option<VirtualWorkspaceId> {
        match self {
            Ownership::Assigned(workspace) => Some(workspace),
            Ownership::Unassigned | Ownership::Claimed(_) | Ownership::Held => None,
        }
    }

    fn claim(self) -> Option<WindowClaim> {
        match self {
            Ownership::Claimed(claim) => Some(claim),
            Ownership::Unassigned | Ownership::Assigned(_) | Ownership::Held => None,
        }
    }

    /// The ownership as the core holds it.
    fn core(self) -> CoreOwned {
        use rift_core::workspaces::{Destination, Owned};
        match self {
            Ownership::Unassigned => Owned::Unowned,
            Ownership::Held => Owned::Owner(rift_core::workspaces::Ownership::Held),
            Ownership::Assigned(workspace) => {
                Owned::Owner(rift_core::workspaces::Ownership::Assigned(workspace))
            }
            Ownership::Claimed(claim) => Owned::Owner(rift_core::workspaces::Ownership::Claimed {
                owner: Claimant {
                    owner: claim.owner,
                    flags: claim.flags,
                    original: claim.original,
                },
                destination: match claim.workspace {
                    Some(workspace) => Destination::Returns(workspace),
                    None => Destination::Nowhere,
                },
            }),
        }
    }
}

/// Who holds a claimed window, as the core keeps it: the owner, the claim's flags and the id
/// the owner claimed. Only the owner takes part in the core's decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Claimant {
    pub(crate) owner: ClaimOwner,
    flags: WindowClaimFlags,
    original: WindowId,
}

#[cfg(feature = "roc-core")]
impl Claimant {
    /// The claimant whole, as the Roc core holds it: the owner, the flags and the id claimed.
    pub(crate) fn plain(self) -> rift_core::workspaces::plain::Claimant {
        use rift_core::workspaces::plain::{ClaimFlags, Claimant as Plain, FocusFollowsMouse};

        use crate::roc_path::Word;
        Plain {
            owner: self.owner.word(),
            flags: ClaimFlags {
                // A claim holds only supported flags (`claim_window` refuses the rest), and focus
                // follows mouse is the one there is.
                focus_follows_mouse: match self
                    .flags
                    .contains(WindowClaimFlags::SUPPRESS_FOCUS_FOLLOWS_MOUSE)
                {
                    true => FocusFollowsMouse::Suppressed,
                    false => FocusFollowsMouse::Followed,
                },
            },
            original: self.original.word(),
        }
    }

    /// The claimant a Roc core's plain one names (the inverse of [`Claimant::plain`]); `None`
    /// for an owner or an original id no claimant has.
    pub(crate) fn from_plain(plain: rift_core::workspaces::plain::Claimant) -> Option<Claimant> {
        use rift_core::workspaces::plain::FocusFollowsMouse;
        Some(Claimant {
            owner: ClaimOwner(u32::try_from(plain.owner).ok()?),
            flags: match plain.flags.focus_follows_mouse {
                FocusFollowsMouse::Suppressed => WindowClaimFlags::SUPPRESS_FOCUS_FOLLOWS_MOUSE,
                FocusFollowsMouse::Followed => WindowClaimFlags::empty(),
            },
            original: crate::roc_path::window_of_word(plain.original)?,
        })
    }
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
        self.state.as_ref().map(|state| state.frame_monotonic())
    }

    pub fn window_server_id(&self) -> Option<WindowServerId> { self.window_server_id }

    pub fn visibility(&self) -> WindowVisibility { self.visibility }
}

/// A window's record together with its transfer in flight, if it has one.
#[derive(Debug, Clone, Copy)]
pub struct RecordRef<'a> {
    record: &'a WindowRecord,
    transfer: Option<Transfer>,
    /// Who places the window, as the core answers.
    ownership: Ownership,
}

impl std::ops::Deref for RecordRef<'_> {
    type Target = WindowRecord;

    fn deref(&self) -> &WindowRecord { self.record }
}

impl RecordRef<'_> {
    pub fn transfer(&self) -> Option<Transfer> { self.transfer }

    fn assignment(&self) -> Option<VirtualWorkspaceId> { self.ownership.assignment() }

    fn claim(&self) -> Option<WindowClaim> { self.ownership.claim() }
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
    windows: Records,
    app_windows: HashMap<i32, HashSet<WindowId>>,
    window_servers: HashMap<WindowServerId, WindowServerRecord>,
    native_fullscreen_records_by_original_window: HashMap<WindowId, NativeFullscreenRecord>,
    native_fullscreen_original_window_by_current_window: HashMap<WindowId, WindowId>,
    native_fullscreen_original_window_by_window_server: HashMap<WindowServerId, WindowId>,
    claims: HashMap<ClaimOwner, Vec<WindowId>>,
    /// The core's lifecycle, in shadow: every live window's admission facts, told each change
    /// the store makes. [`Self::lifecycle_divergences`] holds the two to agreeing.
    lifecycle: crate::roc_twin::TwinnedLifecycle,
}

/// The window store's records and the core's workspaces (the joined shadow: who places each
/// window, told every ownership change, and the workspace store's catalog and activations, told
/// through [`WindowsMut::topology`] by the workspace store's topology writers), lent together
/// for reading. The owner holds the two side by side; a view holds exactly those two.
/// [`Windows::ownership_divergences`] holds the two to agreeing on ownership.
#[derive(Clone, Copy)]
pub(crate) struct Windows<'a> {
    pub(crate) records: &'a WindowStore,
    pub(crate) core: &'a crate::roc_twin::TwinnedWorkspaces,
}

/// [`Windows`], lent for writing: only [`WindowsMut::set_ownership`] writes an ownership.
pub(crate) struct WindowsMut<'a> {
    pub(crate) records: &'a mut WindowStore,
    pub(crate) core: &'a mut crate::roc_twin::TwinnedWorkspaces,
}

impl std::ops::Deref for Windows<'_> {
    type Target = WindowStore;

    fn deref(&self) -> &WindowStore { self.records }
}

impl std::ops::Deref for WindowsMut<'_> {
    type Target = WindowStore;

    fn deref(&self) -> &WindowStore { self.records }
}

impl std::ops::DerefMut for WindowsMut<'_> {
    fn deref_mut(&mut self) -> &mut WindowStore { self.records }
}

impl WindowsMut<'_> {
    /// The same two, lent for reading.
    pub(crate) fn read(&self) -> Windows<'_> {
        Windows {
            records: self.records,
            core: self.core,
        }
    }
}

/// The core's workspaces seeded from `saved`, the topology the stores start with: told its whole
/// catalog, in order, and every Space's activation.
pub(crate) fn joined_core(
    saved: &crate::model::virtual_workspace::SavedTopology,
) -> crate::roc_twin::TwinnedWorkspaces {
    let mut core = crate::roc_twin::TwinnedWorkspaces::default();
    let mut topology = TopologyChanges(&mut core);
    for &(id, space) in saved.catalog() {
        topology.created(id, space);
    }
    for (&space, activation) in saved.activations() {
        topology.activation(space, Some(activation.to_core()));
    }
    core
}

/// A test's owner of the two halves, side by side, as [`crate::model::reactor::RiftState`]
/// holds them.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct OwnedWindows {
    pub(crate) records: WindowStore,
    pub(crate) core: crate::roc_twin::TwinnedWorkspaces,
}

#[cfg(test)]
impl OwnedWindows {
    /// The two halves, the core seeded from `saved`.
    pub(crate) fn joining(saved: &crate::model::virtual_workspace::SavedTopology) -> OwnedWindows {
        OwnedWindows {
            records: WindowStore::default(),
            core: joined_core(saved),
        }
    }

    pub(crate) fn view(&self) -> Windows<'_> {
        Windows {
            records: &self.records,
            core: &self.core,
        }
    }

    pub(crate) fn view_mut(&mut self) -> WindowsMut<'_> {
        WindowsMut {
            records: &mut self.records,
            core: &mut self.core,
        }
    }
}

#[cfg(test)]
impl std::ops::Deref for OwnedWindows {
    type Target = WindowStore;

    fn deref(&self) -> &WindowStore { &self.records }
}

#[cfg(test)]
impl std::ops::DerefMut for OwnedWindows {
    fn deref_mut(&mut self) -> &mut WindowStore { &mut self.records }
}

/// The joined shadow, lent to the workspace store's topology writers: it takes the catalog's and
/// the activations' changes only (a removal, which hands windows on, is
/// [`WindowsMut::remove_workspace`]).
pub(crate) struct TopologyChanges<'a>(&'a mut crate::roc_twin::TwinnedWorkspaces);

impl TopologyChanges<'_> {
    /// A workspace created on a Space, at the end of the catalog.
    pub(crate) fn created(&mut self, workspace: VirtualWorkspaceId, space: SpaceId) {
        self.0
            .step(rift_core::workspaces::WorkspacesEvent::Created { workspace, space });
    }

    pub(crate) fn moved(&mut self, workspace: VirtualWorkspaceId, space: SpaceId) {
        self.0.step(rift_core::workspaces::WorkspacesEvent::Moved { workspace, space });
    }

    /// What `space` shows now, or that it has no activation any more.
    pub(crate) fn activation(
        &mut self,
        space: SpaceId,
        activation: Option<rift_core::workspaces::Activation<VirtualWorkspaceId>>,
    ) {
        self.0.step(match activation {
            Some(activation) => {
                rift_core::workspaces::WorkspacesEvent::Activated { space, activation }
            }
            None => rift_core::workspaces::WorkspacesEvent::Cleared(space),
        });
    }
}

/// A removal's hand-off the store cannot follow: the core decided something a removal never
/// makes of a window the removed workspace placed (or of a claim returning to it). A bug:
/// tests panic, a running Rift says so (`workspace-divergence:`) and leaves the record.
fn removal_unfollowed(window: WindowId, removed: VirtualWorkspaceId, decided: CoreOwned) {
    #[cfg(test)]
    panic!("workspace-divergence: removing {removed:?} left {window:?} {decided:?}");
    #[cfg(not(test))]
    tracing::debug!(
        ?window,
        ?removed,
        ?decided,
        "workspace-divergence: the store cannot follow the core's hand-off of a removed workspace"
    );
}

/// Whether a window that leaves the layout is away for now or for good.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Availability {
    /// It comes back (unminimized, revalidated, out of fullscreen): its slot is suspended.
    TemporarilyUnavailable,
    /// Nothing will bring it back: it is removed from the layout.
    Available,
}

/// Where the store's AX snapshot of a window and the core's lifecycle disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LifecycleDivergence {
    pub(crate) window: WindowId,
    /// The AX snapshot's roles and minimized flag; `None` without AX state.
    pub(crate) snapshot: Option<(rift_core::lifecycle::AxKind, rift_core::lifecycle::Minimize)>,
    /// The core's; `None` when it holds no facts.
    pub(crate) core: Option<(rift_core::lifecycle::AxKind, rift_core::lifecycle::Minimize)>,
}

/// A window's ownership in the core's terms.
type CoreOwned = rift_core::workspaces::Owned<VirtualWorkspaceId, Claimant>;

/// Where the store's claims index (or its records) and the core's ownership of a window
/// disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OwnershipDivergence {
    pub(crate) window: WindowId,
    /// The owner whose claims list the window, if one does.
    pub(crate) indexed: Option<ClaimOwner>,
    /// Whether the store has a record of the window.
    pub(crate) recorded: bool,
    pub(crate) core: CoreOwned,
}

/// Restored native lifecycle facts; placement and actor effects remain external.
pub(crate) struct NativeFullscreenRestoration {
    pub(crate) record: NativeFullscreenRecord,
    pub(crate) window: Option<WindowId>,
    pub(crate) previous_assignment: Option<WindowId>,
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
    Hidden(Option<WindowId>),
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

    pub fn claimed_windows(&self) -> impl Iterator<Item = WindowId> + '_ {
        self.claims.values().flatten().copied()
    }

    pub fn owner_claims(&self, owner: ClaimOwner) -> &[WindowId] {
        self.claims.get(&owner).map_or(&[], Vec::as_slice)
    }

    fn unindex_claim(&mut self, owner: ClaimOwner, wid: WindowId) {
        if let Some(windows) = self.claims.get_mut(&owner) {
            windows.retain(|window| *window != wid);
            if windows.is_empty() {
                self.claims.remove(&owner);
            }
        }
    }

    /// The window's AX element was invalidated: it keeps its ownership, facts and place until
    /// fresh AX evidence or confirmed retirement. A window with no AX state has none to lose.
    /// Whether this invalidated it (false if it already was, or has no AX state).
    pub(crate) fn invalidate_ax_identity(&mut self, wid: WindowId) -> bool {
        self.lifecycle.invalidate(wid) == rift_core::lifecycle::Invalidation::Invalidated
    }

    /// Only a fresh AX observation validates an invalidated identity.
    pub(crate) fn validate_ax_identity(&mut self, wid: WindowId) -> bool {
        self.lifecycle.revalidate(wid) == rift_core::lifecycle::Revalidation::Revalidated
    }

    /// Live windows can lose heuristic admission as their AX/native metadata
    /// changes. Suspend their layout participation without forgetting ownership;
    /// only an explicit exclusion or retirement should discard that ownership.
    pub(crate) fn is_temporarily_unavailable(&self, wid: WindowId) -> bool {
        self.lifecycle.is_invalidated(wid)
            || self.window(wid).is_some_and(|w| {
                w.info.is_minimized
                    || (self.rule(wid) != Some(Rule::Ignore) && !self.is_admitted_state(wid))
            })
            || self.is_window_native_fullscreen_suspended(wid)
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
        self.observe_native_space(wsid, reported_space, false);
        NativeDeparture::Hidden(window)
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

    /// Hidden is visibility evidence, never destruction. Return the owning PID
    /// for a targeted, coalesced AX refresh (including pending native identity).
    pub(crate) fn observe_native_visibility(
        &mut self,
        wsid: WindowServerId,
        visible: bool,
    ) -> Option<i32> {
        if !self.set_native_visibility(wsid, visible) {
            return None;
        }
        self.tracked_window_id(wsid)
            .map(|wid| wid.pid)
            .or_else(|| self.get_window_server_info(wsid).map(|info| info.pid))
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

    /// The window server's id for `window`, if it has one.
    pub(crate) fn server_id(&self, window: WindowId) -> Option<WindowServerId> {
        self.window(window).and_then(|state| state.info.sys_id)
    }

    pub(crate) fn window(&self, window_id: WindowId) -> Option<&WindowState> {
        self.windows.get(&window_id).and_then(|record| record.state.as_ref())
    }

    pub(crate) fn window_mut(&mut self, window_id: WindowId) -> Option<&mut WindowState> {
        self.windows.get_mut(&window_id).and_then(|record| record.state.as_mut())
    }

    // Admission's facts are the core's lifecycle's. These are their only writers; each needs
    // the window's AX state and returns the change in its admission, if any.

    /// The manageability heuristic's verdict for a window with AX state.
    pub(crate) fn set_manageable(
        &mut self,
        window: WindowId,
        manageable: bool,
    ) -> Option<Transition<WindowId>> {
        let manageability = if manageable {
            Manageability::Manageable
        } else {
            Manageability::Unmanageable
        };
        self.tell_lifecycle(window, FactChange::Manageability(manageability))
    }

    /// An app rule's override (or none) for a window with AX state.
    pub(crate) fn set_manage_override(
        &mut self,
        window: WindowId,
        manage_override: Option<bool>,
    ) -> Option<Transition<WindowId>> {
        self.tell_lifecycle(
            window,
            FactChange::Rule(crate::model::reactor::rule_of(manage_override)),
        )
    }

    /// Whether the window server marks a window with AX state transient.
    pub(crate) fn set_native_transient(
        &mut self,
        window: WindowId,
        transient: bool,
    ) -> Option<Transition<WindowId>> {
        let native = if transient {
            NativeKind::Transient
        } else {
            NativeKind::Ordinary
        };
        self.tell_lifecycle(window, FactChange::Native(native))
    }

    /// Whether a window with AX state is minimized: its AX snapshot and the core both.
    pub(crate) fn set_minimized(
        &mut self,
        window: WindowId,
        minimized: bool,
    ) -> Option<Transition<WindowId>> {
        let state = self.window_mut(window)?;
        state.info.is_minimized = minimized;
        let minimize = state.minimize();
        self.tell_lifecycle(window, FactChange::Minimize(minimize))
    }

    /// A fresh AX reading of a window with AX state: its roles and minimized flag with it.
    pub(crate) fn replace_info(
        &mut self,
        window: WindowId,
        info: crate::sys::app::WindowInfo,
    ) -> Option<Transition<WindowId>> {
        let state = self.window_mut(window)?;
        state.info = info;
        let (ax, minimize) = (state.ax_kind(), state.minimize());
        let roles = self.tell_lifecycle(window, FactChange::Ax(ax));
        let minimized = self.tell_lifecycle(window, FactChange::Minimize(minimize));
        let change = rift_core::lifecycle::net(roles, minimized);
        #[cfg(feature = "roc-core")]
        {
            use crate::roc_path::window_free;
            let question = (window_free(roles), window_free(minimized));
            let roc = Ok(rift_core_roc::lifecycle::net(question.0, question.1));
            crate::roc_path::check("net", &[], &question, &window_free(change), roc);
        }
        change
    }

    /// Whether a window is away for now (hidden, minimized, invalidated, in fullscreen, or
    /// unadmitted but not excluded by a rule) or here to stay out of the layout.
    pub(crate) fn availability(&self, wid: WindowId) -> Availability {
        if self.is_temporarily_unavailable(wid) {
            Availability::TemporarilyUnavailable
        } else {
            Availability::Available
        }
    }

    fn tell_lifecycle(
        &mut self,
        window: WindowId,
        change: FactChange,
    ) -> Option<Transition<WindowId>> {
        self.window(window)?;
        self.lifecycle.step(LifecycleEvent::Changed { window, change })
    }

    /// A window's admission by its own facts (claims and AX invalidation aside); false
    /// without AX state.
    pub(crate) fn is_admitted_state(&self, window: WindowId) -> bool {
        self.lifecycle.admission(window) == Some(Admission::Admitted)
    }

    /// Whether a window would be admitted under an app rule's override; `None` without AX
    /// state.
    pub(crate) fn admitted_with_override(
        &self,
        window: WindowId,
        manage_override: Option<bool>,
    ) -> Option<bool> {
        let facts = self.lifecycle.facts(window)?;
        let rule = crate::model::reactor::rule_of(manage_override);
        let facts = AdmissionFacts { rule, ..facts };
        let admission = rift_core::lifecycle::admission(facts);
        #[cfg(feature = "roc-core")]
        {
            let roc = Ok(rift_core_roc::lifecycle::admission(facts));
            crate::roc_path::check("admission", &[], &facts, &admission, roc);
        }
        Some(admission == Admission::Admitted)
    }

    /// The app rule a window with AX state is under.
    pub(crate) fn rule(&self, window: WindowId) -> Option<Rule> {
        self.lifecycle.facts(window).map(|facts| facts.rule)
    }

    /// The manage override a window with AX state carries, as the rule's `Option<bool>`.
    pub(crate) fn manage_override(&self, window: WindowId) -> Option<bool> {
        match self.rule(window)? {
            Rule::NoRule => None,
            Rule::Manage => Some(true),
            Rule::Ignore => Some(false),
        }
    }

    /// A window's admission facts, for tests.
    #[cfg(test)]
    pub(crate) fn admission_facts(&self, window: WindowId) -> Option<AdmissionFacts> {
        self.lifecycle.facts(window)
    }

    /// Every window whose AX snapshot and the core's lifecycle disagree: a window with AX state
    /// the core holds no facts for (or the reverse), or whose minimized flag or AX roles the
    /// core does not have. Empty unless a write to the snapshot bypassed the setters above.
    pub(crate) fn lifecycle_divergences(&self) -> Vec<LifecycleDivergence> {
        let mut windows: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, record)| record.state.is_some())
            .map(|(window, _)| *window)
            .chain(self.lifecycle.windows())
            .collect();
        windows.sort_by_key(|window| (window.pid, window.idx.get()));
        windows.dedup();
        windows
            .into_iter()
            .filter_map(|window| {
                let snapshot = self.window(window).map(|state| (state.ax_kind(), state.minimize()));
                let core = self.lifecycle.facts(window).map(|facts| (facts.ax, facts.minimize));
                (snapshot != core).then_some(LifecycleDivergence { window, snapshot, core })
            })
            .collect()
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
        let mut changed = std::mem::replace(&mut record.visible, visible) != visible;
        if let Some(wid) = record.window_id {
            let visibility = if visible {
                WindowVisibility::Visible
            } else {
                WindowVisibility::Hidden
            };
            let window = self.windows.entry_or_default(wid);
            changed |= std::mem::replace(&mut window.visibility, visibility) != visibility;
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
        // The model's frame for it is the core's: the caller tells it (`RiftState`).
        self.window(wid)?;
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
            record.pending_native_fullscreen = None;
        }
        if let Some(wid) = wid
            && let Some(record) = self.windows.get_mut(&wid)
        {
            record.window_server_id = None;
        }
        self.prune_window_server_record(wsid);
        wid
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
        if let Some(record) = self.windows.get_mut(&window)
            && record.state.take().is_some()
        {
            self.lifecycle.step(LifecycleEvent::Gone(window));
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

    pub fn rule_floating(&self, window_id: WindowId) -> bool {
        self.windows.get(&window_id).is_some_and(|record| record.rule_floating)
    }

    pub fn remap_space(&mut self, old_space: SpaceId, new_space: SpaceId) {
        if old_space == new_space {
            return;
        }

        for transfer in self.windows.transfers_mut() {
            if let Transfer::Refused { on } = transfer
                && *on == Some(old_space)
            {
                *on = Some(new_space);
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

    pub fn set_visibility(&mut self, window_id: WindowId, visibility: WindowVisibility) {
        self.windows.entry_or_default(window_id).visibility = visibility;
        self.app_windows.entry(window_id.pid).or_default().insert(window_id);
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
        match self.windows.held() {
            None => Pointer::Idle,
            Some((window, Grip::Pressed)) => Pointer::Pressing(window),
            Some((window, Grip::Dragging(_))) => Pointer::Dragging(window),
        }
    }

    /// Whether the user's hand is on `window`: pressed, or dragging it.
    pub(crate) fn is_gripped(&self, window: WindowId) -> bool {
        matches!(self.pointer(), Pointer::Pressing(held) | Pointer::Dragging(held) if held == window)
    }

    /// The window the user's hand is on, pressed or dragged.
    pub(crate) fn gripped(&self) -> Option<WindowId> { self.windows.gripped_window() }

    /// The user's drag and the window it holds.
    pub(crate) fn drag(&self) -> Option<(WindowId, &Session)> {
        match self.windows.held() {
            Some((window, Grip::Dragging(session))) => Some((window, session)),
            Some((_, Grip::Pressed)) | None => None,
        }
    }

    pub(crate) fn drag_mut(&mut self) -> Option<(WindowId, &mut Session)> {
        match self.windows.held_mut() {
            Some((window, Grip::Dragging(session))) => Some((window, session)),
            Some((_, Grip::Pressed)) | None => None,
        }
    }

    // The projection of the core's grip. Only `RiftState`'s grip methods call these, each with
    // the core's effect in hand.

    /// Effect `Grip`: `window`'s entry becomes the gripped one, pressed. Refused while the hand
    /// is on a window: the core ungrips first, so a refusal is a disagreement to log.
    pub(crate) fn project_grip(&mut self, window: WindowId) -> Result<(), GripRefused> {
        self.windows.grip(window)
    }

    /// Effect `DragBegan`: the pressed window's grip becomes the drag `session`.
    pub(crate) fn project_drag(&mut self, session: Session) {
        let (_, grip) = self.windows.held_mut().expect("the core drags only a press");
        *grip = Grip::Dragging(Box::new(session));
    }

    /// Effect `Ungrip`: the gripped record goes back among the loose ones. A drag's session is
    /// returned.
    /// Its record is pruned only once the whole step is projected ([`WindowsMut::prune_ungripped`]):
    /// a re-press of the held window is an ungrip and a grip in one step, and the record must
    /// still be there for the grip.
    pub(crate) fn project_ungrip(&mut self) -> Option<Session> {
        let (_, session) = self.windows.let_go().expect("the core ungrips only a grip");
        session
    }

    /// Effect `Regrip`: the grip moves with its window's new identity.
    pub(crate) fn project_regrip(
        &mut self,
        from: WindowId,
        to: WindowId,
    ) -> Result<(), GripRefused> {
        self.windows.regrip(from, to)
    }

    /// Every window with a transfer, and the transfer.
    pub(crate) fn transfers(&self) -> impl Iterator<Item = (WindowId, Transfer)> {
        self.windows.transfers()
    }

    /// The window is on its workspace's Space: nothing is owed, the transfer is over.
    pub(crate) fn end_transfer(&mut self, window_id: WindowId) {
        self.windows.clear_transfer(&window_id);
    }
}

impl<'a> Windows<'a> {
    /// Rift manages a valid AX window when admission passes and no external owner holds it.
    pub(crate) fn is_admitted(self, wid: WindowId) -> bool {
        self.record(wid)
            .is_some_and(|record| record.claim().is_none() && self.records.lifecycle.manages(wid))
    }

    /// An explicit move can change retained ownership independently of placement: a window
    /// with valid AX state, unclaimed and assigned, that nothing but minimizing or the
    /// manageability heuristic keeps out (#702's `allows_management`).
    pub(crate) fn can_move_assigned_window(self, wid: WindowId) -> bool {
        use rift_core::lifecycle::{Admission, AdmissionFacts, Minimize, Veto, admission};
        self.record(wid).is_some_and(|record| {
            !self.records.lifecycle.is_invalidated(wid)
                && record.claim().is_none()
                && record.assignment().is_some()
                && self.records.lifecycle.facts(wid).is_some_and(|facts| {
                    matches!(
                        admission(AdmissionFacts {
                            minimize: Minimize::Shown,
                            ..facts
                        }),
                        Admission::Admitted | Admission::Vetoed(Veto::Unmanageable)
                    )
                })
        })
    }

    /// Rules and admission are reconciled only for windows Rift may manage.
    pub(crate) fn can_reconcile_admission(self, wid: WindowId) -> bool {
        self.record(wid).is_some_and(|record| {
            !self.records.lifecycle.is_invalidated(wid)
                && record.claim().is_none()
                && record.state.as_ref().is_some_and(|window| {
                    window.minimize() == rift_core::lifecycle::Minimize::Shown
                })
        })
    }

    pub fn claim(self, wid: WindowId) -> Option<WindowClaim> {
        self.record(wid).and_then(|record| record.claim())
    }

    pub fn is_claimed(self, wid: WindowId) -> bool {
        self.record(wid).is_some_and(|record| record.claim().is_some())
    }

    pub fn suppresses_focus_follows_mouse(self, wid: Option<WindowId>) -> bool {
        !self.records.claims.is_empty()
            && wid.and_then(|wid| self.claim(wid)).is_some_and(|claim| {
                claim.flags.contains(WindowClaimFlags::SUPPRESS_FOCUS_FOLLOWS_MOUSE)
            })
    }

    /// The window `owner` means by `wid`: its own claim by current, then original, id (an
    /// id reused after a rekey still reaches the claim), otherwise the window tracked as `wid`.
    pub fn resolve_claim_target(self, owner: ClaimOwner, wid: WindowId) -> Option<WindowId> {
        let owned = self.records.owner_claims(owner);
        if owned.contains(&wid) {
            return Some(wid);
        }
        let original = owned
            .iter()
            .copied()
            .find(|current| self.claim(*current).is_some_and(|claim| claim.original == wid));
        original.or_else(|| self.records.contains_window(wid).then_some(wid))
    }

    /// The core's workspaces, joined (the shadow).
    pub(crate) fn core_workspaces(
        self,
    ) -> &'a rift_core::workspaces::Workspaces<WindowId, VirtualWorkspaceId, SpaceId, Claimant>
    {
        self.core
    }

    /// Every window the store's claims index and the core disagree on, or that the core owns
    /// with no record in the store. The core keeps who places each window; the store keeps only
    /// the claims by owner (their order is the owner's), so the two must name the same claimed
    /// windows. Empty unless a write bypassed [`WindowsMut::set_ownership`].
    pub(crate) fn ownership_divergences(self) -> Vec<OwnershipDivergence> {
        use rift_core::workspaces::{Owned, Ownership as Core};
        let indexed = |window: WindowId| {
            self.records
                .claims
                .iter()
                .find(|(_, windows)| windows.contains(&window))
                .map(|(owner, _)| *owner)
        };
        let mut windows: Vec<WindowId> = (self.records.claims.values().flatten().copied())
            .chain(self.core.owners().map(|(window, _)| window))
            .collect();
        windows.sort_unstable();
        windows.dedup();
        windows
            .into_iter()
            .filter_map(|window| {
                let core = self.core.owned(window);
                let claimed_by = match core {
                    Owned::Owner(Core::Claimed { owner, .. }) => Some(owner.owner),
                    Owned::Unowned | Owned::Owner(Core::Assigned(_) | Core::Held) => None,
                };
                let recorded = self.records.windows.get(&window).is_some();
                let owned = core != Owned::Unowned;
                (indexed(window) != claimed_by || (owned && !recorded)).then_some(
                    OwnershipDivergence {
                        window,
                        indexed: indexed(window),
                        recorded,
                        core,
                    },
                )
            })
            .collect()
    }

    pub(crate) fn is_visible_admitted(self, wid: WindowId) -> bool {
        self.is_admitted(wid)
            && self
                .record(wid)
                .and_then(|record| record.window_server_id())
                .is_some_and(|wsid| self.records.is_window_visible(wsid))
    }

    /// Restore identity for native presence or a return to the recorded user Space.
    pub(crate) fn restore_native_identity(
        self,
        wsid: Option<WindowServerId>,
        original: WindowId,
    ) -> Option<NativeFullscreenRestoration> {
        let record = wsid
            .and_then(|wsid| self.records.native_fullscreen_record_for_window_server_id(wsid))
            .or_else(|| self.records.native_fullscreen_record_for_window(original))?;
        if !matches!(record.transition, NativeFullscreenTransition::ExitObserved(_)) {
            return None;
        }
        let bound = record.window_server_id.and_then(|id| self.records.tracked_window_id(id));
        // A specific appearance follows the record's current AX identity first;
        // a Space return follows its surviving native binding first.
        let window = [
            wsid.map(|_| record.current_window_id),
            bound,
            Some(record.current_window_id),
            wsid.and_then(|id| self.records.tracked_window_id(id)),
            wsid.map(|_| original),
        ]
        .into_iter()
        .flatten()
        .find(|wid| self.is_admitted(*wid));
        let previous_assignment = window
            .is_some_and(|wid| wid != original && self.workspace_for_window(original).is_some())
            .then_some(original);
        Some(NativeFullscreenRestoration {
            record,
            window,
            previous_assignment,
        })
    }

    /// A current successful AX omission is destructive only with native negative evidence:
    /// the windows to retire. The caller removes them (`RiftState::remove_window`), so a grip
    /// on one ends through the core.
    pub(crate) fn inventory_retirements(
        self,
        pid: i32,
        observed: impl IntoIterator<Item = WindowId>,
        mut inactive: impl FnMut(WindowId) -> bool,
        mut observe: impl FnMut(WindowServerId, Option<WindowServerInfo>) -> InventoryWindowObservation,
    ) -> Vec<WindowId> {
        use rift_core::lifecycle::{
            AxValidity, FullscreenRecord, InventoryFacts, InventoryView, NativeEvidence,
            NativeIdentity, NativeLook, Ordering, Report, Retirement, Suitability,
        };
        let observed: HashSet<_> = observed.into_iter().collect();
        self.records
            .window_ids_for_pid(pid)
            .filter_map(|wid| {
                let record = self.record(wid)?;
                let wsid = record.window_server_id;
                let facts = InventoryFacts {
                    report: if observed.contains(&wid) {
                        Report::Reported
                    } else {
                        Report::Omitted
                    },
                    view: if inactive(wid) {
                        InventoryView::OutOfView
                    } else {
                        InventoryView::InView
                    },
                    minimize: record.state.as_ref().map(WindowState::minimize),
                    fullscreen: if self.records.native_fullscreen_record_for_window(wid).is_some() {
                        FullscreenRecord::Recorded
                    } else {
                        FullscreenRecord::NotRecorded
                    },
                    identity: wsid.map_or(NativeIdentity::Unknown, NativeIdentity::Known),
                    validity: if self.records.lifecycle.is_invalidated(wid) {
                        AxValidity::Invalidated
                    } else {
                        AxValidity::Valid
                    },
                };
                let asked = crate::roc_path::Asked::new();
                let verdict = rift_core::lifecycle::retirement(facts, |wsid| {
                    asked.ask(|| {
                        let observation = observe(wsid, self.records.get_window_server_info(wsid));
                        NativeEvidence {
                            suitability: observation.suitable.map(|suitable| {
                                if suitable {
                                    Suitability::Suitable
                                } else {
                                    Suitability::Unsuitable
                                }
                            }),
                            ordering: observation.ordered_in.map(|ordered_in| {
                                if ordered_in {
                                    Ordering::OrderedIn
                                } else {
                                    Ordering::OrderedOut
                                }
                            }),
                            look: observation.info.map(|info| {
                                if info.layer != 0
                                    || info.frame.size.width.abs() < 2.0
                                    || info.frame.size.height.abs() < 2.0
                                {
                                    NativeLook::NotAWindow
                                } else {
                                    NativeLook::Window
                                }
                            }),
                        }
                    })
                });
                #[cfg(feature = "roc-core")]
                {
                    use crate::roc_path::Word;
                    let question = rift_core::lifecycle::plain::Inventory {
                        facts: InventoryFacts {
                            identity: match facts.identity {
                                NativeIdentity::Known(wsid) => NativeIdentity::Known(wsid.word()),
                                NativeIdentity::Unknown => NativeIdentity::Unknown,
                            },
                            report: facts.report,
                            view: facts.view,
                            minimize: facts.minimize,
                            fullscreen: facts.fullscreen,
                            validity: facts.validity,
                        },
                        evidence: asked.fact(|evidence| evidence),
                    };
                    let roc = rift_core_roc::lifecycle::retirement(question);
                    crate::roc_path::check(
                        "retirement",
                        rift_core::lifecycle::plain::Inventory::FACTS,
                        &question,
                        &verdict,
                        roc,
                    );
                }
                (verdict == Retirement::Retire).then_some(wid)
            })
            .collect()
    }

    pub fn record(self, window_id: WindowId) -> Option<RecordRef<'a>> {
        Some(RecordRef {
            record: self.records.windows.get(&window_id)?,
            transfer: self.records.windows.transfer(&window_id),
            ownership: self.ownership(window_id),
        })
    }

    /// Who places `window`, as the core answers: the store keeps no ownership of its own.
    fn ownership(self, window: WindowId) -> Ownership {
        use rift_core::workspaces::{Destination, Owned, Ownership as Core};
        match self.core.owned(window) {
            Owned::Unowned => Ownership::Unassigned,
            Owned::Owner(Core::Assigned(workspace)) => Ownership::Assigned(workspace),
            Owned::Owner(Core::Held) => Ownership::Held,
            Owned::Owner(Core::Claimed { owner, destination }) => Ownership::Claimed(WindowClaim {
                owner: owner.owner,
                flags: owner.flags,
                original: owner.original,
                workspace: match destination {
                    Destination::Returns(workspace) => Some(workspace),
                    Destination::Nowhere => None,
                },
            }),
        }
    }

    pub fn workspace_for_window(self, window_id: WindowId) -> Option<VirtualWorkspaceId> {
        self.records.windows.get(&window_id)?;
        self.ownership(window_id).assignment()
    }

    /// The windows assigned to a workspace, by (pid, index), as the core answers.
    pub fn workspace_windows(self, workspace_id: VirtualWorkspaceId) -> Vec<WindowId> {
        self.core.windows_on(workspace_id).collect()
    }

    /// The window the user holds, if `workspace_id` has it. Once it leaves
    /// ([`WindowsMut::leave_workspace_held`]) it is in no workspace's windows: a transfer can't carry it.
    pub(crate) fn held_member(self, workspace_id: VirtualWorkspaceId) -> Option<HeldMember> {
        self.records
            .windows
            .gripped_window()
            .filter(|&window| self.workspace_for_window(window) == Some(workspace_id))
            .map(HeldMember)
    }

    /// Whether `window` is held and placed by no workspace (a transfer left it).
    #[cfg(test)]
    pub(crate) fn is_held_unplaced(self, window: WindowId) -> bool {
        self.records.windows.get(&window).is_some() && self.ownership(window) == Ownership::Held
    }

    pub fn workspace_window_count(self, workspace_id: VirtualWorkspaceId) -> usize {
        self.core.windows_on(workspace_id).count()
    }

    pub fn iter_workspace_assignments(
        self,
    ) -> impl Iterator<Item = (WindowId, VirtualWorkspaceId)> + 'a {
        self.core.owners().filter_map(|(window, ownership)| match ownership {
            rift_core::workspaces::Ownership::Assigned(workspace) => Some((window, workspace)),
            rift_core::workspaces::Ownership::Claimed { .. }
            | rift_core::workspaces::Ownership::Held => None,
        })
    }

    pub fn workspace_assignment_count(self) -> usize { self.iter_workspace_assignments().count() }

    pub fn native_space(self, window: WindowId) -> Option<SpaceId> {
        self.records.window_server_space(self.record(window)?.window_server_id?)
    }

    pub(crate) fn can_place_window(self, window: WindowId) -> bool {
        self.record(window).is_none_or(|record| {
            !self.records.lifecycle.is_invalidated(window)
                && record.claim().is_none()
                && (record.state.is_none() || self.records.is_admitted_state(window))
                && !matches!(
                    record.visibility,
                    WindowVisibility::Hidden | WindowVisibility::Minimized
                )
        }) && self.records.native_fullscreen_record_for_window(window).is_none()
    }

    #[cfg(any(test, debug_assertions))]
    pub fn debug_assert_invariants(self) {
        for (&wid, record) in self.records.windows.iter() {
            debug_assert!(
                self.records.app_windows.get(&wid.pid).is_some_and(|ids| ids.contains(&wid))
            );
            if let Some(wsid) = record.window_server_id {
                debug_assert_eq!(self.records.tracked_window_id(wsid), Some(wid));
            }
            let _ = record;
            if let Some(claim) = self.ownership(wid).claim() {
                debug_assert!(self.records.owner_claims(claim.owner).contains(&wid));
            }
        }
        for (&owner, ids) in &self.records.claims {
            debug_assert!(!ids.is_empty());
            debug_assert!(ids.iter().all(|wid| self.claim(*wid).is_some_and(|c| c.owner == owner)));
        }
        for (&pid, ids) in &self.records.app_windows {
            debug_assert!(
                ids.iter().all(|wid| wid.pid == pid && self.records.windows.get(wid).is_some())
            );
        }
        for (&wsid, server) in &self.records.window_servers {
            if let Some(wid) = server.window_id {
                debug_assert_eq!(
                    self.records.windows.get(&wid).and_then(|record| record.window_server_id),
                    Some(wsid)
                );
            }
        }
    }
}

impl WindowsMut<'_> {
    /// Claim a tracked window, holding its assignment until release.
    pub fn insert_claim(&mut self, wid: WindowId, owner: ClaimOwner, flags: WindowClaimFlags) {
        debug_assert!(self.records.contains_window(wid) && !self.read().is_claimed(wid));
        let workspace = self.take_assignment(wid);
        self.set_ownership(
            wid,
            Ownership::Claimed(WindowClaim {
                owner,
                flags,
                original: wid,
                workspace,
            }),
        );
        self.records.claims.entry(owner).or_default().push(wid);
    }

    pub fn set_claim_flags(&mut self, wid: WindowId, flags: WindowClaimFlags) {
        if let Some(claim) = self.read().claim(wid) {
            self.set_ownership(wid, Ownership::Claimed(WindowClaim { flags, ..claim }));
        }
    }

    /// End a claim. The caller restores the returned assignment if it is still valid.
    pub fn remove_claim(&mut self, wid: WindowId) -> Option<WindowClaim> {
        self.records.windows.get(&wid)?;
        let claim = self.read().ownership(wid).claim()?;
        self.set_ownership(wid, Ownership::Unassigned);
        self.records.unindex_claim(claim.owner, wid);
        self.prune_window_record(wid);
        Some(claim)
    }

    /// Takes a window's ownership out of the record and out of the indexes it is in, leaving
    /// it unassigned.
    fn take_ownership(&mut self, wid: WindowId) -> Ownership {
        if self.records.windows.get(&wid).is_none() {
            return Ownership::Unassigned;
        }
        let ownership = self.read().ownership(wid);
        self.set_ownership(wid, Ownership::Unassigned);
        match ownership {
            Ownership::Unassigned | Ownership::Held | Ownership::Assigned(_) => {}
            Ownership::Claimed(claim) => self.records.unindex_claim(claim.owner, wid),
        }
        ownership
    }

    /// Gives an unassigned window `ownership`, and puts it in the index that goes with it.
    fn give_ownership(&mut self, wid: WindowId, ownership: Ownership) {
        self.set_ownership(wid, ownership);
        match ownership {
            Ownership::Unassigned | Ownership::Held | Ownership::Assigned(_) => {}
            Ownership::Claimed(claim) => {
                self.records.claims.entry(claim.owner).or_default().push(wid)
            }
        }
    }

    fn take_assignment(&mut self, wid: WindowId) -> Option<VirtualWorkspaceId> {
        self.records.windows.get(&wid)?;
        let assignment = self.read().ownership(wid).assignment()?;
        self.set_ownership(wid, Ownership::Unassigned);
        Some(assignment)
    }

    /// The only writer of a window's ownership, which the core keeps; the window has a record
    /// while it is owned.
    fn set_ownership(&mut self, wid: WindowId, ownership: Ownership) {
        self.records.windows.entry_or_default(wid);
        self.tell_ownership(wid, ownership);
    }

    fn tell_ownership(&mut self, window: WindowId, ownership: Ownership) {
        self.core.step(rift_core::workspaces::WorkspacesEvent::Owned {
            window,
            owned: ownership.core(),
        });
    }

    /// Where the workspace store's topology writers tell the joined shadow each change: only
    /// topology changes go through it, never a window's ownership.
    pub(crate) fn topology(&mut self) -> TopologyChanges<'_> { TopologyChanges(&mut *self.core) }

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
                self.records
                    .pending_native_fullscreen_record_for_window_server_id(wsid)
                    .map(|record| (record.fullscreen_space, record.transition))
            });
        if lifecycle.is_some_and(|(fullscreen, transition)| {
            fullscreen == space && matches!(transition, NativeFullscreenTransition::ExitObserved(_))
        }) {
            return None;
        }
        self.records.set_window_server_space(wsid, Some(space));
        if let Some(wid) = self.records.tracked_window_id(wsid) {
            let assignment = self.read().workspace_for_window(wid);
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
                pid.or_else(|| self.records.get_window_server_info(wsid).map(|info| info.pid))
            {
                self.records.suspend_window_server_to_native_fullscreen(
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

    /// Bind a positive AX observation to its stable native identity. Persistent
    /// membership survives AX replacement; only projection rekeying is external.
    pub(crate) fn reconcile_ax_identity(
        &mut self,
        wid: WindowId,
        wsid: Option<WindowServerId>,
    ) -> Option<WindowId> {
        if let Some(old) = self.read().record(wid).and_then(|record| record.window_server_id())
            && Some(old) != wsid
        {
            self.records.remove_window_server_state(old);
            self.records.windows.clear_transfer(&wid);
        }
        let wsid = wsid?;
        let previous = self.track_window_server_id(wsid, wid).filter(|old| *old != wid);
        if let Some(old) = previous {
            if !self.records.contains_window(wid) {
                let state =
                    self.records.windows.get_mut(&old).and_then(|record| record.state.take());
                let facts = self.records.lifecycle.facts(old);
                if state.is_some() {
                    self.records.lifecycle.step(LifecycleEvent::Gone(old));
                }
                if let Some(state) = state {
                    // The same window under a new identity keeps its heuristic verdict, rule
                    // and native transience; its AX snapshot says the rest.
                    match facts {
                        Some(facts) => {
                            self.insert_window_as(wid, state, facts.manageability, facts.rule);
                            self.records
                                .set_native_transient(wid, facts.native == NativeKind::Transient);
                        }
                        None => self.insert_window(wid, state),
                    }
                }
            }
            self.replace_window_identity(old, wid);
        }
        self.records.clear_window_server_observed(wsid);
        previous
    }

    pub(crate) fn set_minimized_visibility(&mut self, wid: WindowId, minimized: bool) {
        if let Some(wsid) = self.read().record(wid).and_then(|record| record.window_server_id()) {
            self.records.set_native_visibility(wsid, !minimized);
        }
        if let Some(record) = self.records.windows.get_mut(&wid) {
            record.visibility = if minimized {
                WindowVisibility::Minimized
            } else {
                WindowVisibility::Visible
            };
        }
    }

    /// Apply native presence and visibility, returning only affected logical
    /// ownership requiring location confirmation. Unmapped/inactive windows
    /// can become hidden without an unnecessary native membership query.
    pub(crate) fn reconcile_native_snapshot(
        &mut self,
        observed: &[(WindowServerId, Option<SpaceId>)],
        active_spaces: &HashSet<SpaceId>,
    ) -> HashSet<WindowServerId> {
        let observed_ids: HashSet<_> = observed.iter().map(|(wsid, _)| *wsid).collect();
        let hidden: Vec<_> = self
            .iter_visible_window_server_ids()
            .filter(|wsid| !observed_ids.contains(wsid))
            .collect();
        for wsid in hidden {
            self.records.mark_window_hidden(wsid);
        }
        for &(wsid, space) in observed {
            if let Some(space) = space {
                self.records.observe_native_space(wsid, space, true);
            } else {
                self.records.mark_window_visible(wsid);
            }
        }
        self.core
            .owners()
            .filter_map(|(wid, ownership)| match ownership {
                rift_core::workspaces::Ownership::Assigned(_) => Some(wid),
                rift_core::workspaces::Ownership::Claimed { .. }
                | rift_core::workspaces::Ownership::Held => None,
            })
            .filter_map(|wid| {
                let wid = &wid;
                let record = self.read().record(*wid)?;
                let wsid = record.window_server_id?;
                (record.assignment().is_some_and(|assignment| {
                    self.core.space_of(assignment).is_some_and(|on| active_spaces.contains(&on))
                }) && self.records.native_fullscreen_record_for_window(*wid).is_none()
                    && !observed_ids.contains(&wsid))
                .then_some(wsid)
            })
            .collect()
    }

    /// Native absence may reflect ordering rather than an AX lifecycle change.
    /// Confirmed moves reconcile location; a live AX window keeps its ownership.
    pub(crate) fn reconcile_native_absence(
        &mut self,
        wsid: WindowServerId,
        active_spaces: &HashSet<SpaceId>,
        inactive_target: Option<SpaceId>,
        preserve_assignment: bool,
    ) -> Option<(WindowId, Option<SpaceId>)> {
        self.records.mark_window_hidden(wsid);
        let wid = self.records.tracked_window_id(wsid)?;
        let record = self.read().record(wid)?;
        // Hiding, minimizing and fullscreen all retain ownership while suspended.
        if self.records.is_temporarily_unavailable(wid)
            || !record.assignment().is_some_and(|assignment| {
                self.core.space_of(assignment).is_some_and(|on| active_spaces.contains(&on))
            })
        {
            return None;
        }
        if let Some(space) = inactive_target {
            self.records.set_window_server_space(wsid, Some(space));
            return Some((wid, Some(space)));
        }
        if preserve_assignment || self.read().is_admitted(wid) {
            return None;
        }
        Some((wid, None))
    }

    /// Gives a window minimal AX state, manageable and under no rule, as a real window has
    /// before macOS can invalidate its AX element (tests that build windows from layout events
    /// alone).
    #[cfg(test)]
    pub(crate) fn insert_test_ax_state(&mut self, window: WindowId) {
        let info = crate::sys::app::WindowInfo {
            has_native_tabs: false,
            is_standard: true,
            is_root: true,
            is_minimized: false,
            is_resizable: true,
            min_size: None,
            max_size: None,
            title: format!("{window:?}"),
            frame: objc2_core_foundation::CGRect::ZERO,
            sys_id: None,
            bundle_id: None,
            path: None,
            ax_role: None,
            ax_subrole: None,
        };
        self.insert_manageable_window(window, WindowState::from(info));
    }

    /// Tracks a window's AX state as the heuristic's manageable window under no rule.
    #[cfg(test)]
    pub(crate) fn insert_manageable_window(&mut self, window_id: WindowId, window: WindowState) {
        self.insert_window_as(window_id, window, Manageability::Manageable, Rule::NoRule);
    }

    /// Tracks a window's AX state, read afresh: unmanageable until the heuristic says
    /// otherwise, under no rule, not a native transient.
    pub(crate) fn insert_window(&mut self, window_id: WindowId, window: WindowState) {
        self.insert_window_as(window_id, window, Manageability::Unmanageable, Rule::NoRule);
    }

    /// Tracks a window's AX state, read afresh, with the heuristic's verdict and a rule already
    /// known (an identity replacing another keeps its rule).
    pub(crate) fn insert_window_as(
        &mut self,
        window_id: WindowId,
        window: WindowState,
        manageability: Manageability,
        rule: Rule,
    ) {
        let wsid = window.info.sys_id;
        let facts = AdmissionFacts {
            manageability,
            rule,
            ax: window.ax_kind(),
            native: NativeKind::Ordinary,
            minimize: window.minimize(),
        };
        self.records.lifecycle.step(LifecycleEvent::Live { window: window_id, facts });
        self.records.windows.entry_or_default(window_id).state = Some(window);
        self.records.app_windows.entry(window_id.pid).or_default().insert(window_id);
        if let Some(wsid) = wsid {
            self.track_window_server_id(wsid, window_id);
        }
    }

    fn prune_window_record(&mut self, window_id: WindowId) {
        let should_remove = self.records.windows.get(&window_id).is_some_and(|record| {
            record.state.is_none()
                && record.window_server_id.is_none()
                && self.read().ownership(window_id) == Ownership::Unassigned
                && !record.rule_floating
                // The user's hand keeps its window's record until it lets go.
                && self.records.windows.gripped_window() != Some(window_id)
        });
        if should_remove {
            self.records.windows.remove(&window_id);
            if let Some((_, session)) = self.records.drag_mut() {
                session.forget_target(window_id);
            }
            if let Some(windows) = self.records.app_windows.get_mut(&window_id.pid) {
                windows.remove(&window_id);
                if windows.is_empty() {
                    self.records.app_windows.remove(&window_id.pid);
                }
            }
        }
    }

    pub fn track_window_server_id(
        &mut self,
        wsid: WindowServerId,
        window_id: WindowId,
    ) -> Option<WindowId> {
        let (old, pending_record) = {
            let record = self.records.server_record_mut(wsid);
            let old = record.window_id;
            record.window_id = Some(window_id);
            let pending = record.pending_native_fullscreen.take();
            (old, pending)
        };
        if let Some(old) = old.filter(|old| *old != window_id)
            && let Some(record) = self.records.windows.get_mut(&old)
        {
            record.window_server_id = None;
        }
        if let Some(previous_wsid) = self
            .windows
            .get(&window_id)
            .and_then(|record| record.window_server_id)
            .filter(|previous| *previous != wsid)
            && let Some(record) = self.records.window_servers.get_mut(&previous_wsid)
        {
            record.window_id = None;
            self.records.prune_window_server_record(previous_wsid);
        }
        let window = self.records.windows.entry_or_default(window_id);
        window.window_server_id = Some(wsid);
        self.records.app_windows.entry(window_id.pid).or_default().insert(window_id);
        if let Some(pending_record) = pending_record {
            if pending_record.pid != window_id.pid {
                self.records.prune_window_server_record(wsid);
                return old;
            }
            let _ = self.suspend_window_to_native_fullscreen(
                window_id,
                Some(wsid),
                pending_record.last_known_user_space,
                pending_record.fullscreen_space,
                pending_record.transition,
            );
        } else if let Some(original_window_id) =
            self.records.native_fullscreen_original_window(window_id)
            && let Some(mut native_record) = self
                .records
                .remove_native_fullscreen_record_by_original_window(original_window_id)
        {
            native_record.window_server_id = Some(wsid);
            self.records.upsert_native_fullscreen_record(native_record);
        }
        self.records.prune_window_server_record(wsid);
        old
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
            self.records.native_fullscreen_original_window(window_id).unwrap_or(window_id);
        let existing = self
            .native_fullscreen_records_by_original_window
            .get(&original_window_id)
            .copied();
        let workspace = self.read().workspace_for_window(window_id);
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
        self.records.windows.entry_or_default(window_id);
        self.records.app_windows.entry(window_id.pid).or_default().insert(window_id);
        self.records.upsert_native_fullscreen_record(record)
    }

    pub fn assign_window_to_workspace(
        &mut self,
        window_id: WindowId,
        assignment: VirtualWorkspaceId,
    ) -> Option<VirtualWorkspaceId> {
        // A held window a workspace transfer left is placed only by the drop.
        if self.records.windows.get(&window_id).is_some()
            && self.read().ownership(window_id) == Ownership::Held
        {
            tracing::debug!(
                ?window_id,
                ?assignment,
                "assignment refused: the window is held, unplaced"
            );
            return None;
        }
        // A claimed window never enters the index; any placement updates its destination.
        if let Some(claim) = self.read().claim(window_id) {
            self.set_ownership(
                window_id,
                Ownership::Claimed(WindowClaim {
                    workspace: Some(assignment),
                    ..claim
                }),
            );
            return claim.workspace;
        }
        let old = self.read().workspace_for_window(window_id);
        if old == Some(assignment) {
            return old;
        }
        self.set_ownership(window_id, Ownership::Assigned(assignment));
        self.records.app_windows.entry(window_id.pid).or_default().insert(window_id);
        if let Some(original_window_id) = self.records.native_fullscreen_original_window(window_id)
            && let Some(mut record) = self
                .records
                .remove_native_fullscreen_record_by_original_window(original_window_id)
        {
            record.workspace = Some(assignment);
            self.records.upsert_native_fullscreen_record(record);
        }
        old
    }

    /// The held window leaves its workspace, placed by nothing until the hand lets go.
    pub(crate) fn leave_workspace_held(&mut self, held: HeldMember) {
        let window = held.window();
        self.records.windows.clear_transfer(&window);
        self.take_assignment(window);
        self.set_ownership(window, Ownership::Held);
    }

    /// The hand let go of `window`: placed by nothing, it is unassigned now, for whoever let go
    /// to place.
    pub(crate) fn release_held(&mut self, window: WindowId) -> Placed {
        match self.records.windows.get(&window).map(|_| self.read().ownership(window)) {
            Some(Ownership::Held) => {
                self.set_ownership(window, Ownership::Unassigned);
                Placed::Unplaced
            }
            Some(Ownership::Unassigned | Ownership::Assigned(_) | Ownership::Claimed(_)) | None => {
                Placed::ByWorkspace
            }
        }
    }

    pub fn remove_window_assignment(&mut self, window_id: WindowId) -> Option<VirtualWorkspaceId> {
        // A claim keeps its saved workspace until release; only workspace removal clears it.
        self.records.windows.clear_transfer(&window_id);
        let old = self.take_assignment(window_id);
        self.prune_window_record(window_id);
        old
    }

    /// Move workspace/rule metadata from an old AX window id to a new one when
    /// macOS rekeys the same WindowServer window across sleep/wake or similar
    /// churn. The caller remains responsible for replacing any layout/window
    /// state that still references `from`.
    pub(crate) fn replace_window_identity(&mut self, from: WindowId, to: WindowId) {
        self.transfer_persistent_window_metadata(from, to);
        self.remove_window(from);
    }

    fn transfer_persistent_window_metadata(&mut self, from: WindowId, to: WindowId) {
        if from == to {
            return;
        }

        let (rule_floating, visibility, geom) = match self.records.windows.get(&from) {
            Some(record) => (
                record.rule_floating,
                record.visibility,
                record.floating_display_geometry,
            ),
            None => return,
        };
        // The workspace an assigned predecessor had, for its fullscreen record below.
        let workspace = self.read().workspace_for_window(from);

        // Ownership moves to the new identity. The recognized identity's claim wins over one
        // taken on the provisional replacement (that one ends, and its owner's requests resolve
        // to the surviving claim); a claim keeps the other side's workspace as its destination
        // when it has none.
        let from_ownership = self.take_ownership(from);
        let to_ownership = self.take_ownership(to);
        let ownership = match (from_ownership, to_ownership) {
            // The window under the hand stays placed by nothing until it is let go.
            (Ownership::Held, _) | (_, Ownership::Held) => Ownership::Held,
            (Ownership::Unassigned, kept) => kept,
            (Ownership::Assigned(workspace), Ownership::Unassigned | Ownership::Assigned(_)) => {
                Ownership::Assigned(workspace)
            }
            (Ownership::Assigned(workspace), Ownership::Claimed(claim)) => {
                Ownership::Claimed(WindowClaim {
                    workspace: claim.workspace.or(Some(workspace)),
                    ..claim
                })
            }
            (Ownership::Claimed(claim), Ownership::Unassigned) => Ownership::Claimed(claim),
            (Ownership::Claimed(claim), Ownership::Assigned(workspace)) => {
                Ownership::Claimed(WindowClaim {
                    workspace: claim.workspace.or(Some(workspace)),
                    ..claim
                })
            }
            (Ownership::Claimed(claim), Ownership::Claimed(displaced)) => {
                if displaced.owner != claim.owner {
                    tracing::warn!(
                        ?from,
                        ?to,
                        ?displaced,
                        "Identity merge ended a conflicting claim"
                    );
                }
                Ownership::Claimed(WindowClaim {
                    workspace: claim.workspace.or(displaced.workspace),
                    ..claim
                })
            }
        };
        self.give_ownership(to, ownership);

        let target = self.records.windows.entry_or_default(to);
        target.rule_floating |= rule_floating;
        target.visibility = visibility;
        target.floating_display_geometry = geom;
        self.records.app_windows.entry(to.pid).or_default().insert(to);
        // The user's grip moved with the identity already (the core re-keyed it first). A
        // transfer moves too, unless the replacement is the window under the user's hand.
        debug_assert_ne!(
            self.records.windows.gripped_window(),
            Some(from),
            "re-key the grip first"
        );
        let transfer = self.records.windows.transfer(&from);
        self.records.windows.set_transfer(&to, transfer);

        // An invalidation moves with the identity, to a window with AX state.
        self.records.lifecycle.carry_invalidation(from, to);
        if let Some(source) = self.records.windows.get_mut(&from) {
            source.rule_floating = false;
            source.floating_display_geometry = None;
        }
        self.records.windows.clear_transfer(&from);

        if let Some(original_window_id) = self.records.native_fullscreen_original_window(from)
            && let Some(mut record) = self
                .records
                .remove_native_fullscreen_record_by_original_window(original_window_id)
        {
            if record.current_window_id == from {
                record.current_window_id = to;
            }
            if record.workspace.is_none() {
                record.workspace = workspace;
            }
            if record.last_known_user_space.is_none() {
                record.last_known_user_space = self.read().native_space(to);
            }
            self.records.upsert_native_fullscreen_record(record);
        }

        self.prune_window_record(from);
    }

    pub fn replace_rule_floating(&mut self, window_id: WindowId, value: bool) -> bool {
        let record = self.records.windows.entry_or_default(window_id);
        let previous = std::mem::replace(&mut record.rule_floating, value);
        self.prune_window_record(window_id);
        previous
    }

    pub fn clear_rule_floating(&mut self, window_id: WindowId) {
        if let Some(record) = self.records.windows.get_mut(&window_id) {
            record.rule_floating = false;
        }
        self.prune_window_record(window_id);
    }

    pub fn remove_window(&mut self, window_id: WindowId) -> Option<WindowRecord> {
        let Some(record) = self.records.windows.remove(&window_id) else {
            return None;
        };
        if record.state.is_some() {
            self.records.lifecycle.step(LifecycleEvent::Gone(window_id));
        }
        // A drag holding the window ends with it; one elsewhere can no longer drop onto it.
        if let Some((_, session)) = self.records.drag_mut() {
            session.forget_target(window_id);
        }
        if let Some(claim) = self.read().ownership(window_id).claim() {
            self.records.unindex_claim(claim.owner, window_id);
        }
        self.tell_ownership(window_id, Ownership::Unassigned);
        if let Some(wsid) = record.window_server_id {
            self.records.remove_window_server_state(wsid);
        }
        if let Some(windows) = self.records.app_windows.get_mut(&window_id.pid) {
            windows.remove(&window_id);
            if windows.is_empty() {
                self.records.app_windows.remove(&window_id.pid);
            }
        }
        Some(record)
    }

    pub fn remove_windows_for_app(&mut self, pid: i32) {
        for window_id in self.records.app_windows.remove(&pid).into_iter().flatten() {
            self.remove_window(window_id);
        }

        let fullscreen_keys: Vec<_> = self
            .native_fullscreen_records_by_original_window
            .keys()
            .copied()
            .filter(|window_id| window_id.pid == pid)
            .collect();
        for original_window_id in fullscreen_keys {
            let _ = self
                .records
                .remove_native_fullscreen_record_by_original_window(original_window_id);
        }

        self.records.window_servers.retain(|_, record| {
            if record.info.is_some_and(|info| info.pid == pid) {
                return false;
            }
            if record.pending_native_fullscreen.is_some_and(|pending| pending.pid == pid) {
                record.pending_native_fullscreen = None;
            }
            !record.is_empty()
        });
    }

    /// A workspace the workspace store removed: the core's shadow takes it out of the catalog,
    /// handing its windows and claims to `successor`, and the records follow what the core
    /// decided for each window the workspace placed and each claim returning to it. A native
    /// fullscreen record naming it takes the same successor (it is not the core's).
    pub(crate) fn remove_workspace(
        &mut self,
        removed: VirtualWorkspaceId,
        successor: rift_core::workspaces::Successor<VirtualWorkspaceId>,
    ) {
        use rift_core::workspaces::{
            Destination, Owned, Ownership as Core, Successor, WorkspacesEvent,
        };
        let assigned = self.read().workspace_windows(removed);
        let claimed: Vec<WindowId> = (self.records.claims.values().flatten().copied())
            .filter(|&wid| {
                self.read().claim(wid).is_some_and(|claim| claim.workspace == Some(removed))
            })
            .collect();
        self.core.step(WorkspacesEvent::Removed { workspace: removed, successor });
        for window in assigned {
            match self.core.owned(window) {
                Owned::Owner(Core::Assigned(heir)) => {
                    self.assign_window_to_workspace(window, heir);
                }
                Owned::Unowned => {
                    self.remove_window_assignment(window);
                }
                decided @ Owned::Owner(Core::Claimed { .. } | Core::Held) => {
                    removal_unfollowed(window, removed, decided);
                }
            }
        }
        for window in claimed {
            let Some(claim) = self.read().claim(window) else {
                continue;
            };
            let workspace = match self.core.owned(window) {
                Owned::Owner(Core::Claimed { destination, .. }) => match destination {
                    Destination::Returns(heir) => Some(heir),
                    Destination::Nowhere => None,
                },
                decided @ (Owned::Unowned | Owned::Owner(Core::Assigned(_) | Core::Held)) => {
                    removal_unfollowed(window, removed, decided);
                    continue;
                }
            };
            self.set_ownership(window, Ownership::Claimed(WindowClaim { workspace, ..claim }));
        }
        let heir = match successor {
            Successor::Into(heir) => Some(heir),
            Successor::Unassign => None,
        };
        for record in self.records.native_fullscreen_records_by_original_window.values_mut() {
            if record.workspace == Some(removed) {
                record.workspace = heir;
            }
        }
    }

    /// After a step's effects: prunes the record of a window the step ungripped, if it holds
    /// nothing and the hand is not on it again.
    pub(crate) fn prune_ungripped(&mut self, window: WindowId) { self.prune_window_record(window); }

    /// `window` as Rift may place it, tracking it if need be; `None` while the user's hand is
    /// on it: the slot it is in has no transfer.
    pub(crate) fn unheld(&mut self, window: WindowId) -> Option<Unheld<'_>> {
        let placeable = self.read().can_place_window(window);
        let transfer = self.records.windows.loose_or_new(window)?;
        self.records.app_windows.entry(window.pid).or_default().insert(window);
        Some(Unheld {
            transfer,
            placeable,
            topology_revision: self.records.topology_revision,
        })
    }
}

#[cfg(test)]
mod tests {
    use rift_core::workspaces::Successor;

    use super::*;
    use crate::model::virtual_workspace::WorkspaceStore;

    #[test]
    fn transfer_retries_once_per_revision_then_refuses_until_begun_again() {
        let mut store = OwnedWindows::default();
        let window = WindowId::new(42, 1);
        let (a, b) = (SpaceId::new(1), SpaceId::new(2));
        let transfer = |store: &OwnedWindows| store.view().record(window).unwrap().transfer();
        let retry = |store: &mut OwnedWindows, revision, available| {
            store
                .view_mut()
                .unheld(window)
                .unwrap()
                .retry_transfer(revision, available, Some(a))
        };
        store.accept_topology_revision(10);
        assert!(store.view_mut().unheld(window).unwrap().begin_transfer(true));
        assert_eq!(
            transfer(&store),
            Some(Transfer::Moving { retries: 0, revision: 10 })
        );
        for (revision, writes) in [
            (10, false),
            (11, true),
            (11, false),
            (12, true),
            (13, false),
            (14, false),
        ] {
            assert_eq!(retry(&mut store, revision, true), writes, "revision {revision}");
        }
        // macOS left it on `a` through every retry: Rift asks no more, and keeps the
        // assignment only while the window stays there.
        assert_eq!(transfer(&store), Some(Transfer::Refused { on: Some(a) }));
        assert!(Transfer::Refused { on: Some(a) }.keeps_assignment(Some(a)));
        assert!(!Transfer::Refused { on: Some(a) }.keeps_assignment(Some(b)));
        assert!(Transfer::Moving { retries: 0, revision: 0 }.keeps_assignment(Some(b)));
        // The destination going away and coming back is a fresh budget.
        assert!(!retry(&mut store, 15, false));
        assert_eq!(transfer(&store), Some(Transfer::Waiting));
        assert!(retry(&mut store, 15, true));
        assert_eq!(
            transfer(&store),
            Some(Transfer::Moving { retries: 1, revision: 15 })
        );
        // A new move always starts afresh; it waits while its destination is unavailable.
        assert!(!store.view_mut().unheld(window).unwrap().begin_transfer(false));
        assert_eq!(transfer(&store), Some(Transfer::Waiting));
        store.end_transfer(window);
        assert_eq!(transfer(&store), None);
    }

    #[test]
    fn app_termination_clears_stale_pending_fullscreen_without_erasing_another_pid() {
        let mut store = OwnedWindows::default();
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
        store.view_mut().remove_windows_for_app(7);
        assert_eq!(store.get_window_server_info(wsid).unwrap().pid, 8);
        assert!(store.pending_native_fullscreen_record_for_window_server_id(wsid).is_none());
        store.view().debug_assert_invariants();
    }

    #[test]
    fn pending_registration_requires_visible_unmapped_window_for_pid() {
        let mut store = OwnedWindows::default();
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
        store.view_mut().track_window_server_id(wsid, WindowId::new(2, 1));
        assert!(!store.has_pending_window_for_pid(2));
    }

    #[test]
    fn authoritative_space_only_record_is_not_pruned() {
        let mut window_store = OwnedWindows::default();
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
        let mut window_store = OwnedWindows::default();
        let wsid = WindowServerId::new(78);

        window_store.set_window_server_space(wsid, Some(SpaceId::new(10)));
        window_store.set_window_server_space(wsid, None);

        assert_eq!(window_store.window_server_space(wsid), None);
        assert!(window_store.iter_window_server_ids().next().is_none());
    }

    fn admitted(wid: WindowId, wsid: WindowServerId) -> WindowState {
        let state = WindowState::from(crate::sys::app::WindowInfo {
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
        state
    }

    #[test]
    fn minimized_membership_survives_reconciliation_until_permanent_removal() {
        let mut store = OwnedWindows::default();
        let mut workspaces = WorkspaceStore::new();
        let home = workspaces
            .create_workspace(&mut store.view_mut(), SpaceId::new(10), None)
            .unwrap();
        let wid = WindowId::new(1, 1);
        store
            .view_mut()
            .insert_manageable_window(wid, admitted(wid, WindowServerId::new(1)));
        store.view_mut().assign_window_to_workspace(wid, home);

        for _ in 0..2 {
            store.set_minimized(wid, true);
            store.view_mut().set_minimized_visibility(wid, true);
            assert!(!store.view().is_admitted(wid));
            assert_eq!(store.view().workspace_for_window(wid), Some(home));
            assert_eq!(store.view().workspace_windows(home), vec![wid]);
            store.view().debug_assert_invariants();
        }

        store
            .view_mut()
            .insert_manageable_window(wid, admitted(wid, WindowServerId::new(1)));
        assert!(store.view().is_admitted(wid));
        assert_eq!(store.view().workspace_for_window(wid), Some(home));
        store.view_mut().remove_window(wid);
        assert_eq!(store.view().workspace_for_window(wid), None);
        assert!(store.view().workspace_windows(home).is_empty());
    }

    #[test]
    fn claims_hold_assignment_outside_the_index_and_follow_identity() {
        let mut store = OwnedWindows::default();
        let space = SpaceId::new(10);
        let mut workspaces = WorkspaceStore::new();
        let [home, moved, replaced] = ["Home", "Moved", "Replaced"].map(|name| {
            workspaces
                .create_workspace(&mut store.view_mut(), space, Some(name.into()))
                .unwrap()
        });
        let (owner, other) = (ClaimOwner(7), ClaimOwner(8));
        let (wid, rekeyed, sibling) =
            (WindowId::new(1, 1), WindowId::new(1, 2), WindowId::new(1, 3));
        store
            .view_mut()
            .insert_manageable_window(wid, admitted(wid, WindowServerId::new(1)));
        store
            .view_mut()
            .insert_manageable_window(sibling, admitted(sibling, WindowServerId::new(3)));
        store.view_mut().assign_window_to_workspace(wid, home);
        store.view_mut().assign_window_to_workspace(sibling, home);

        store.view_mut().insert_claim(wid, owner, WindowClaimFlags::empty());
        assert!(!store.view().is_admitted(wid) && !store.view().can_reconcile_admission(wid));
        assert!(store.is_admitted_state(wid), "native admission is untouched");
        assert_eq!(store.view().workspace_for_window(wid), None);
        assert_eq!(store.view().workspace_windows(home), vec![sibling]);
        assert_eq!(store.view().iter_workspace_assignments().count(), 1);
        // Placement while claimed only updates the destination.
        assert_eq!(
            store.view_mut().assign_window_to_workspace(wid, moved),
            Some(home)
        );
        assert!(store.view().workspace_windows(moved).is_empty());
        store.view_mut().remove_workspace(moved, Successor::Into(replaced));
        assert_eq!(store.view().claim(wid).unwrap().workspace(), Some(replaced));
        store.view().debug_assert_invariants();

        // An AX rekey carries the claim; a replacement assignment folds into it.
        store
            .view_mut()
            .insert_manageable_window(rekeyed, admitted(rekeyed, WindowServerId::new(2)));
        store.view_mut().assign_window_to_workspace(rekeyed, home);
        assert_eq!(
            store.view_mut().reconcile_ax_identity(rekeyed, Some(WindowServerId::new(1))),
            Some(wid)
        );
        assert_eq!(store.owner_claims(owner), &[rekeyed]);
        assert_eq!(store.view().claim(rekeyed).unwrap().workspace(), Some(replaced));
        assert_eq!(store.view().workspace_windows(home), vec![sibling]);
        assert_eq!(store.view().resolve_claim_target(owner, wid), Some(rekeyed));
        assert_eq!(store.view().resolve_claim_target(other, wid), None);
        store.view().debug_assert_invariants();

        let claim = store.view_mut().remove_claim(rekeyed).unwrap();
        assert_eq!((claim.owner, claim.workspace()), (owner, Some(replaced)));
        assert!(store.view().is_admitted(rekeyed) && store.owner_claims(owner).is_empty());
    }

    #[test]
    fn claims_survive_id_reuse_and_merged_identities() {
        let mut store = OwnedWindows::default();
        let space = SpaceId::new(10);
        let mut workspaces = WorkspaceStore::new();
        let [home, provisional] = ["Home", "Provisional"].map(|name| {
            workspaces
                .create_workspace(&mut store.view_mut(), space, Some(name.into()))
                .unwrap()
        });
        let (a, b, c) = (ClaimOwner(1), ClaimOwner(2), ClaimOwner(3));
        let id = |idx| WindowId::new(1, idx);
        let wsid = WindowServerId::new;
        let add = |store: &mut OwnedWindows, idx: u32, server: u32, workspace| {
            store.view_mut().insert_window(id(idx), admitted(id(idx), wsid(server)));
            store.view_mut().assign_window_to_workspace(id(idx), workspace);
        };
        let flags = WindowClaimFlags::empty();

        // A's window is rekeyed and its old id reused by an unrelated window B claims.
        add(&mut store, 1, 1, home);
        store.view_mut().insert_claim(id(1), a, flags);
        add(&mut store, 2, 2, provisional);
        store.view_mut().reconcile_ax_identity(id(2), Some(wsid(1)));
        add(&mut store, 1, 9, home);
        assert_eq!(store.view().resolve_claim_target(a, id(1)), Some(id(2)));
        assert_eq!(store.view().resolve_claim_target(c, id(1)), Some(id(1)));
        store.view_mut().insert_claim(id(1), b, flags);
        assert_eq!(store.view().resolve_claim_target(a, id(1)), Some(id(2)));
        assert_eq!(store.view().resolve_claim_target(b, id(1)), Some(id(1)));
        assert_eq!(store.view().resolve_claim_target(a, id(2)), Some(id(2)));
        assert_eq!(store.view().claim(id(2)).unwrap().workspace(), Some(home));
        store.view().debug_assert_invariants();

        // Both identities of one window claimed by different owners: the recognized
        // identity's claim survives with its original id and workspace; the other ends.
        add(&mut store, 3, 3, provisional);
        store.view_mut().insert_claim(id(3), a, flags);
        store.view_mut().reconcile_ax_identity(id(3), Some(wsid(9)));
        let merged = store.view().claim(id(3)).unwrap();
        assert_eq!((merged.owner, merged.workspace()), (b, Some(home)));
        assert_eq!(store.owner_claims(a), &[id(2)]);
        assert_eq!(store.owner_claims(b), &[id(3)]);
        assert_eq!(store.view().resolve_claim_target(b, id(1)), Some(id(3)));
        assert_eq!(
            store.view().resolve_claim_target(a, id(3)),
            Some(id(3)),
            "B's, so rejected"
        );
        store.view().debug_assert_invariants();

        // The same owner on both identities keeps one claim reachable by either id.
        add(&mut store, 4, 4, provisional);
        store
            .view_mut()
            .insert_claim(id(4), a, WindowClaimFlags::SUPPRESS_FOCUS_FOLLOWS_MOUSE);
        store.view_mut().reconcile_ax_identity(id(4), Some(wsid(1)));
        assert_eq!(store.owner_claims(a), &[id(4)]);
        assert_eq!(store.view().claim(id(4)).unwrap().flags, flags);
        for requested in [id(1), id(4)] {
            assert_eq!(store.view().resolve_claim_target(a, requested), Some(id(4)));
        }
        assert_eq!(
            store.view().resolve_claim_target(a, id(2)),
            None,
            "retired, never claimed"
        );
        store.view().debug_assert_invariants();

        // A deleted workspace leaves claims to be rehomed on release.
        store.view_mut().remove_workspace(home, Successor::Unassign);
        assert_eq!(store.view().claim(id(3)).unwrap().workspace(), None);
        assert_eq!(store.view().claim(id(4)).unwrap().workspace(), None);

        // Destruction ends a claim and its original-id mapping.
        store.view_mut().remove_window(id(3));
        assert!(store.owner_claims(b).is_empty());
        assert_eq!(store.view().resolve_claim_target(b, id(1)), None);
        store.view().debug_assert_invariants();
    }

    #[test]
    fn transfer_persistent_metadata_replaces_existing_target_workspace_assignment() {
        let mut window_store = OwnedWindows::default();
        let space = SpaceId::new(10);
        let mut workspaces = WorkspaceStore::new();
        let source_workspace = workspaces
            .create_workspace(&mut window_store.view_mut(), space, Some("Source".to_string()))
            .expect("source workspace");
        let target_workspace = workspaces
            .create_workspace(&mut window_store.view_mut(), space, Some("Target".to_string()))
            .expect("target workspace");
        let from = WindowId::new(1, 1);
        let to = WindowId::new(1, 2);

        window_store.view_mut().assign_window_to_workspace(from, source_workspace);
        window_store.view_mut().assign_window_to_workspace(to, target_workspace);

        window_store.view_mut().transfer_persistent_window_metadata(from, to);

        assert_eq!(
            window_store.view().workspace_for_window(to),
            Some(source_workspace)
        );
        assert!(window_store.view().workspace_windows(target_workspace).is_empty());
        assert_eq!(window_store.view().workspace_windows(source_workspace), vec![to]);
    }

    #[test]
    fn transfer_persistent_metadata_rekeys_native_fullscreen_record() {
        let mut window_store = OwnedWindows::default();
        let space = SpaceId::new(10);
        let fullscreen_space = SpaceId::new(0x400000000 + space.get());
        let mut workspaces = WorkspaceStore::new();
        let workspace_id = workspaces
            .create_workspace(
                &mut window_store.view_mut(),
                space,
                Some("Fullscreen".to_string()),
            )
            .expect("workspace");
        let from = WindowId::new(1, 1);
        let to = WindowId::new(1, 2);
        let wsid = WindowServerId::new(77);

        window_store.view_mut().assign_window_to_workspace(from, workspace_id);
        let _ = window_store.view_mut().suspend_window_to_native_fullscreen(
            from,
            Some(wsid),
            Some(space),
            fullscreen_space,
            NativeFullscreenTransition::Suspended,
        );

        window_store.view_mut().transfer_persistent_window_metadata(from, to);

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
        let mut window_store = OwnedWindows::default();
        let wid = WindowId::new(1, 1);
        let wsid = WindowServerId::new(91);
        let user_space = SpaceId::new(11);
        let fullscreen_space = SpaceId::new(0x400000000 + user_space.get());

        let record = window_store.view_mut().suspend_window_to_native_fullscreen(
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
        window_store.view_mut().replace_rule_floating(wid, true);
        assert!(
            !window_store.view_mut().unheld(wid).unwrap().begin_transfer(true),
            "rule reapplication must not override native fullscreen suspension"
        );
        window_store.restore_window_from_native_fullscreen(wid).unwrap();
        assert!(window_store.view_mut().unheld(wid).unwrap().begin_transfer(true));
    }

    #[test]
    fn remove_window_preserves_native_fullscreen_record_until_app_cleanup() {
        let mut window_store = OwnedWindows::default();
        let wid = WindowId::new(7, 1);
        let wsid = WindowServerId::new(92);
        let user_space = SpaceId::new(12);
        let fullscreen_space = SpaceId::new(0x400000000 + user_space.get());

        let frame = objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(100.0, 100.0),
        );
        window_store.view_mut().insert_window(
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
        let _ = window_store.view_mut().suspend_window_to_native_fullscreen(
            wid,
            Some(wsid),
            Some(user_space),
            fullscreen_space,
            NativeFullscreenTransition::Suspended,
        );

        window_store.view_mut().remove_window(wid);

        assert!(
            window_store.native_fullscreen_record_for_window(wid).is_some(),
            "transient AX removal should not drop the fullscreen lifecycle record"
        );

        window_store.view_mut().remove_windows_for_app(wid.pid);

        assert!(
            window_store.native_fullscreen_record_for_window(wid).is_none(),
            "app cleanup should retire the fullscreen lifecycle record"
        );
    }

    #[test]
    fn track_window_server_id_binds_pending_native_fullscreen_record() {
        let mut window_store = OwnedWindows::default();
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

        window_store.view_mut().track_window_server_id(wsid, wid);

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
        let mut window_store = OwnedWindows::default();
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

        window_store.view_mut().track_window_server_id(wsid, rebound_wid);

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
        let mut store = OwnedWindows::default();
        let first = WindowId::new(1, 1);
        let second = WindowId::new(1, 2);
        let wsid = WindowServerId::new(44);

        store.view_mut().track_window_server_id(wsid, first);
        assert_eq!(
            store.view_mut().track_window_server_id(wsid, second),
            Some(first)
        );

        assert_eq!(
            store.view().record(first).and_then(|record| record.window_server_id()),
            None
        );
        assert_eq!(
            store.view().record(second).and_then(|record| record.window_server_id()),
            Some(wsid)
        );
        assert_eq!(store.tracked_window_id(wsid), Some(second));
        store.view().debug_assert_invariants();
    }

    #[test]
    fn native_identity_change_retires_transfer_but_ax_rekey_preserves_it() {
        let mut store = OwnedWindows::default();
        let (old, replacement) = (WindowId::new(30, 1), WindowId::new(30, 2));
        let server = WindowServerId::new(70);
        let mut workspaces = WorkspaceStore::new();
        let workspace = workspaces
            .create_workspace(&mut store.view_mut(), SpaceId::new(8), None)
            .unwrap();
        store.view_mut().assign_window_to_workspace(old, workspace);
        store.view_mut().replace_rule_floating(old, true);
        store.view_mut().track_window_server_id(server, old);
        store.view_mut().unheld(old).unwrap().begin_transfer(true);
        let pending = store.view().record(old).unwrap().transfer().unwrap();
        assert_eq!(
            store.view_mut().reconcile_ax_identity(replacement, Some(server)),
            Some(old)
        );
        assert_eq!(
            store.view().record(replacement).unwrap().transfer(),
            Some(pending)
        );
        assert!(store.rule_floating(replacement));
        assert_eq!(store.view().workspace_windows(workspace), vec![replacement]);
        store
            .view_mut()
            .reconcile_ax_identity(replacement, Some(WindowServerId::new(71)));
        assert!(store.view().record(replacement).unwrap().transfer().is_none());
        store.view().debug_assert_invariants();
    }

    #[test]
    fn one_hand_one_held_window_and_a_second_grip_is_refused() {
        let (a, b) = (WindowId::new(7, 1), WindowId::new(7, 2));
        let mut records = Records::default();
        records.entry_or_default(a);
        records.entry_or_default(b);
        assert_eq!(records.hands(), 1);
        assert_eq!(records.grip(a), Ok(()));
        assert_eq!(records.gripped_window(), Some(a));
        // Without a let-go in between, neither another window nor the same one is gripped.
        assert_eq!(records.grip(b), Err(GripRefused::Holding(a)));
        assert_eq!(records.grip(a), Err(GripRefused::Holding(a)));
        assert_eq!(records.gripped_window(), Some(a));
        assert_eq!(records.hands(), 1);
        // An entry that leaves the map gives the hand back, even the held one's.
        assert!(records.take(&a).is_some());
        assert_eq!((records.gripped_window(), records.hands()), (None, 1));
        assert_eq!(records.grip(b), Ok(()));
        assert_eq!(records.let_go().map(|(window, _)| window), Some(b));
        assert_eq!((records.gripped_window(), records.hands()), (None, 1));
        assert_eq!(records.regrip(a, b), Err(GripRefused::NothingHeld));
    }

    #[test]
    fn the_store_grips_no_window_it_has_no_record_of() {
        let (known, unknown) = (WindowId::new(7, 1), WindowId::new(7, 9));
        let mut records = Records::default();
        records.entry_or_default(known);
        assert_eq!(records.grip(unknown), Err(GripRefused::Unknown(unknown)));
        assert!(records.get(&unknown).is_none());
        assert_eq!((records.gripped_window(), records.hands()), (None, 1));
        assert_eq!(records.grip(known), Ok(()));
    }

    #[test]
    fn pressing_the_held_window_again_keeps_its_record() {
        // The core lets go and grips again in one step. A record with nothing else in it (no
        // server id, workspace, state or claim) must not be pruned between the two.
        let mut state = crate::model::RiftState::default();
        let window = WindowId::new(4, 1);
        state.windows.set_visibility(window, WindowVisibility::Visible);
        let press = crate::actor::reactor::reconciler::CoreEvent::Press {
            window: Some(window),
            button: crate::model::drag::MouseButton::Left.into(),
        };
        state.step(press.clone());
        state.step(press);
        assert_eq!(state.windows.gripped(), Some(window));
        assert_eq!(state.windows.window_ids_for_pid(4).collect::<Vec<_>>(), vec![
            window
        ]);
        state.windows().debug_assert_invariants();
        // Let go, and the empty record is pruned as before.
        state.let_go();
        assert!(state.windows().record(window).is_none());
    }

    #[test]
    fn a_window_is_in_one_place_and_the_store_holds_what_the_core_holds() {
        // Random presses, drags, releases, let-gos, re-keys, removals and transfers through the
        // core. After every step: one hand, at most one held window, no transfer on it, and the
        // store's pointer is the core's (`project` checks that last one on every step too).
        use crate::actor::reactor::reconciler::CoreEvent;
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let left = || crate::model::drag::MouseButton::Left.into();
        for _ in 0..300 {
            let mut state = crate::model::RiftState::default();
            let mut live: Vec<WindowId> = (1..=4).map(|idx| WindowId::new(9, idx)).collect();
            let mut fresh = 5;
            // Every window has a window-server id, as a tracked one does: its record is never
            // pruned as empty while it lives.
            let track = |state: &mut crate::model::RiftState, window: WindowId| {
                state
                    .windows_mut()
                    .track_window_server_id(WindowServerId::new(100 + window.idx.get()), window);
            };
            for window in &live {
                track(&mut state, *window);
            }
            for _ in 0..40 {
                let pick = |n: u64, live: &[WindowId]| live.get(n as usize).copied();
                let window = pick(next(live.len().max(1) as u64 + 1), &live);
                match next(8) {
                    // The reactor presses only windows it has a record of (or nothing).
                    0 | 1 => {
                        let window =
                            window.filter(|window| state.windows().record(*window).is_some());
                        state.step(CoreEvent::Press { window, button: left() });
                    }
                    2 => {
                        if let Some(window) = window {
                            let frame = objc2_core_foundation::CGRect::ZERO;
                            let source = crate::model::drag::DragSource {
                                origin_frame: frame,
                                last_frame: frame,
                                origin_space: None,
                                current_space: None,
                                tiled: true,
                            };
                            state.begin_drag(window, Session::native(source, Default::default()));
                        }
                    }
                    3 => {
                        state.release(rift_core::Release::Button(left()));
                    }
                    4 => {
                        state.let_go();
                    }
                    5 => {
                        if let Some(from) = window {
                            let to = WindowId::new(9, fresh);
                            fresh += 1;
                            state.step(CoreEvent::Rekeyed { from, to });
                            state.windows_mut().transfer_persistent_window_metadata(from, to);
                            track(&mut state, to);
                            live.retain(|window| *window != from);
                            live.push(to);
                        }
                    }
                    6 => {
                        if let Some(window) = window {
                            state.remove_window(window);
                            live.retain(|live| *live != window);
                        }
                    }
                    _ => {
                        let mut windows = state.windows_mut();
                        if let Some(unheld) = window.and_then(|window| windows.unheld(window)) {
                            unheld.begin_transfer(true);
                        }
                    }
                }
                let held = state.windows.gripped();
                assert_eq!(state.windows.windows.hands(), 1);
                assert_eq!(
                    state
                        .windows
                        .windows
                        .entries
                        .values()
                        .filter(|entry| entry.is_gripped())
                        .count(),
                    usize::from(held.is_some())
                );
                if let Some(held) = held {
                    assert_eq!(state.windows.windows.transfer(&held), None);
                    assert!(state.windows_mut().unheld(held).is_none());
                }
                state.windows().debug_assert_invariants();
            }
        }
    }

    #[test]
    fn a_reading_that_admits_by_its_roles_and_suspends_by_its_minimize_changes_nothing() {
        // Suspended as an AXDialog window, a fresh reading finds ordinary roles (admitting it)
        // and the window minimized (suspending it again): one reading, no change.
        let wid = WindowId::new(3, 1);
        let mut store = OwnedWindows::default();
        let info = crate::sys::app::WindowInfo {
            has_native_tabs: false,
            is_standard: true,
            is_root: true,
            is_minimized: false,
            is_resizable: true,
            min_size: None,
            max_size: None,
            title: "w".into(),
            frame: objc2_core_foundation::CGRect::ZERO,
            sys_id: None,
            bundle_id: None,
            path: None,
            ax_role: Some("AXWindow".into()),
            ax_subrole: Some("AXDialog".into()),
        };
        store.view_mut().insert_manageable_window(wid, WindowState::from(info.clone()));
        assert!(!store.is_admitted_state(wid));
        let reading = crate::sys::app::WindowInfo {
            ax_subrole: Some("AXStandardWindow".into()),
            is_minimized: true,
            ..info
        };
        assert_eq!(store.replace_info(wid, reading), None);
        assert!(!store.is_admitted_state(wid));
    }

    #[test]
    fn the_lifecycle_shadow_agrees_through_the_setters_and_catches_a_write_around_them() {
        let wid = WindowId::new(3, 1);
        let mut store = OwnedWindows::default();
        let info = crate::sys::app::WindowInfo {
            has_native_tabs: false,
            is_standard: true,
            is_root: true,
            is_minimized: false,
            is_resizable: true,
            min_size: None,
            max_size: None,
            title: "w".into(),
            frame: objc2_core_foundation::CGRect::ZERO,
            sys_id: None,
            bundle_id: None,
            path: None,
            ax_role: Some("AXWindow".into()),
            ax_subrole: Some("AXStandardWindow".into()),
        };
        store.view_mut().insert_manageable_window(wid, WindowState::from(info.clone()));
        assert_eq!(store.lifecycle_divergences(), vec![]);
        store.set_minimized(wid, true);
        store.set_minimized(wid, false);
        store.set_manage_override(wid, Some(false));
        store.set_manage_override(wid, None);
        store.set_native_transient(wid, true);
        store.set_native_transient(wid, false);
        store.set_manageable(wid, false);
        store.set_manageable(wid, true);
        let mut dialog = info.clone();
        dialog.ax_subrole = Some("AXDialog".into());
        store.replace_info(wid, dialog);
        assert_eq!(store.lifecycle_divergences(), vec![]);
        store.replace_info(wid, info);
        assert!(store.view().is_admitted(wid));
        assert_eq!(store.lifecycle_divergences(), vec![]);

        // A write around the setters is caught.
        store.window_mut(wid).unwrap().info.is_minimized = true;
        {
            use rift_core::lifecycle::{AxKind, Minimize};
            assert_eq!(store.lifecycle_divergences(), vec![LifecycleDivergence {
                window: wid,
                snapshot: Some((AxKind::Ordinary, Minimize::Minimized)),
                core: Some((AxKind::Ordinary, Minimize::Shown)),
            }]);
        }
        store.set_minimized(wid, true);
        assert_eq!(store.lifecycle_divergences(), vec![]);

        // Removing the window leaves neither side holding it.
        store.view_mut().remove_window(wid);
        assert_eq!(store.lifecycle_divergences(), vec![]);
    }

    #[test]
    fn an_invalidated_window_is_unmanaged_until_fresh_ax_and_its_invalidation_moves_with_it() {
        let (wid, tab) = (WindowId::new(5, 1), WindowId::new(5, 2));
        let mut store = OwnedWindows::default();
        // No AX state: nothing to invalidate.
        store.invalidate_ax_identity(wid);
        assert!(!store.is_temporarily_unavailable(wid));
        store.view_mut().insert_test_ax_state(wid);
        assert!(store.view().is_admitted(wid) && store.view().can_reconcile_admission(wid));
        store.invalidate_ax_identity(wid);
        assert!(!store.view().is_admitted(wid) && !store.view().can_reconcile_admission(wid));
        assert!(store.is_temporarily_unavailable(wid));
        assert!(store.is_admitted_state(wid), "its facts still admit it");
        assert!(store.validate_ax_identity(wid));
        assert!(!store.validate_ax_identity(wid));
        assert!(store.view().is_admitted(wid));
        // A native tab taking over an invalidated window inherits the invalidation.
        store.invalidate_ax_identity(wid);
        store.view_mut().insert_test_ax_state(tab);
        store.view_mut().transfer_persistent_window_metadata(wid, tab);
        assert!(store.window(wid).is_some() && !store.is_temporarily_unavailable(wid));
        assert!(store.is_temporarily_unavailable(tab) && !store.view().is_admitted(tab));
        assert!(store.validate_ax_identity(tab));
        assert!(store.view().is_admitted(tab));
        assert_eq!(store.lifecycle_divergences(), vec![]);
    }

    #[test]
    fn ownership_moves_to_a_rekeyed_identity_and_a_claim_wins_keeping_a_destination() {
        let space = SpaceId::new(10);
        let mut workspaces = WorkspaceStore::new();
        let mut joined = OwnedWindows::default();
        let [a, b, c] = ["A", "B", "C"].map(|name| {
            workspaces
                .create_workspace(&mut joined.view_mut(), space, Some(name.into()))
                .unwrap()
        });
        let (from, to) = (WindowId::new(1, 1), WindowId::new(1, 2));
        let (owner, other) = (ClaimOwner(7), ClaimOwner(8));
        #[derive(Clone, Copy)]
        enum Side {
            Unassigned,
            Assigned(VirtualWorkspaceId),
            Claimed(ClaimOwner, Option<VirtualWorkspaceId>),
        }
        let setup = |store: &mut OwnedWindows, window: WindowId, side: Side| {
            store.view_mut().insert_test_ax_state(window);
            match side {
                Side::Unassigned => {}
                Side::Assigned(workspace) => {
                    store.view_mut().assign_window_to_workspace(window, workspace);
                }
                Side::Claimed(owner, held) => {
                    if let Some(workspace) = held {
                        store.view_mut().assign_window_to_workspace(window, workspace);
                    }
                    store.view_mut().insert_claim(window, owner, WindowClaimFlags::empty());
                }
            }
        };
        // (from, to) → to's assignment, to's claim owner and destination.
        let cases = [
            (Side::Unassigned, Side::Assigned(b), Some(b), None),
            (Side::Assigned(a), Side::Unassigned, Some(a), None),
            (Side::Assigned(a), Side::Assigned(b), Some(a), None),
            (
                Side::Assigned(a),
                Side::Claimed(other, None),
                None,
                Some((other, Some(a))),
            ),
            (
                Side::Assigned(a),
                Side::Claimed(other, Some(c)),
                None,
                Some((other, Some(c))),
            ),
            (
                Side::Claimed(owner, None),
                Side::Unassigned,
                None,
                Some((owner, None)),
            ),
            (
                Side::Claimed(owner, None),
                Side::Assigned(b),
                None,
                Some((owner, Some(b))),
            ),
            (
                Side::Claimed(owner, Some(a)),
                Side::Assigned(b),
                None,
                Some((owner, Some(a))),
            ),
            (
                Side::Claimed(owner, None),
                Side::Claimed(other, Some(c)),
                None,
                Some((owner, Some(c))),
            ),
            (
                Side::Claimed(owner, Some(a)),
                Side::Claimed(other, Some(c)),
                None,
                Some((owner, Some(a))),
            ),
        ];
        for (case, (from_side, to_side, assigned, claimed)) in cases.into_iter().enumerate() {
            let mut store = OwnedWindows::joining(&workspaces.reading(joined.view()).topology());
            setup(&mut store, from, from_side);
            setup(&mut store, to, to_side);
            store.view_mut().transfer_persistent_window_metadata(from, to);
            assert_eq!(store.view().workspace_for_window(to), assigned, "case {case}");
            assert_eq!(
                store.view().claim(to).map(|claim| (claim.owner, claim.workspace())),
                claimed,
                "case {case}"
            );
            assert_eq!(store.view().workspace_for_window(from), None, "case {case}");
            assert!(!store.view().is_claimed(from), "case {case}");
            for workspace in [a, b, c] {
                assert_eq!(
                    store.view().workspace_windows(workspace),
                    if assigned == Some(workspace) {
                        vec![to]
                    } else {
                        vec![]
                    },
                    "case {case}"
                );
            }
            let claimants: Vec<_> = store.claimed_windows().collect();
            assert_eq!(
                claimants,
                if claimed.is_some() { vec![to] } else { vec![] },
                "case {case}"
            );
            store.view().debug_assert_invariants();
        }
    }

    #[test]
    fn a_gripped_window_is_still_found_everywhere_it_was() {
        // The gripped window's record lives in the slot, not among the loose ones. Lookups,
        // counts, the workspace index and assignment iteration must not notice.
        let mut state = crate::model::RiftState::default();
        let (wid, other) = (WindowId::new(4, 1), WindowId::new(4, 2));
        let wsid = WindowServerId::new(55);
        let mut workspaces = WorkspaceStore::new();
        let workspace = workspaces
            .create_workspace(
                &mut state.windows_mut(),
                SpaceId::new(9),
                Some("Slot".to_string()),
            )
            .expect("workspace");
        state.windows_mut().track_window_server_id(wsid, wid);
        state.windows_mut().assign_window_to_workspace(wid, workspace);
        state.windows_mut().assign_window_to_workspace(other, workspace);
        let assignments = |store: Windows<'_>| {
            let mut all: Vec<_> = store.iter_workspace_assignments().collect();
            all.sort_by_key(|(window, _)| window.idx.get());
            all
        };
        let before = (
            state.windows().workspace_assignment_count(),
            assignments(state.windows()),
            state.windows().workspace_for_window(wid),
            state.windows().record(wid).unwrap().window_server_id(),
            state.windows.window_ids_for_pid(4).count(),
        );

        state.step(crate::actor::reactor::reconciler::CoreEvent::Press {
            window: Some(wid),
            button: crate::model::drag::MouseButton::Left.into(),
        });
        assert_eq!(state.windows.gripped(), Some(wid));
        let pressed = (
            state.windows().workspace_assignment_count(),
            assignments(state.windows()),
            state.windows().workspace_for_window(wid),
            state.windows().record(wid).unwrap().window_server_id(),
            state.windows.window_ids_for_pid(4).count(),
        );
        assert_eq!(pressed, before);
        state.windows().debug_assert_invariants();

        // One slot: pressing another window lets go of the first.
        state.step(crate::actor::reactor::reconciler::CoreEvent::Press {
            window: Some(other),
            button: crate::model::drag::MouseButton::Left.into(),
        });
        assert_eq!(state.windows.gripped(), Some(other));
        assert!(state.windows_mut().unheld(wid).is_some());
        assert!(state.windows_mut().unheld(other).is_none());
        state.windows().debug_assert_invariants();
    }

    #[test]
    fn the_grip_follows_a_rekeyed_window_and_ends_with_a_removed_one() {
        let mut state = crate::model::RiftState::default();
        let (old, new) = (WindowId::new(30, 1), WindowId::new(30, 2));
        let mut workspaces = WorkspaceStore::new();
        let workspace = workspaces
            .create_workspace(
                &mut state.windows_mut(),
                SpaceId::new(9),
                Some("Rekey".to_string()),
            )
            .expect("workspace");
        state.windows_mut().assign_window_to_workspace(old, workspace);
        state.step(crate::actor::reactor::reconciler::CoreEvent::Press {
            window: Some(old),
            button: crate::model::drag::MouseButton::Left.into(),
        });

        // Same window, new identity: the core re-keys the grip, then the store moves the rest.
        state.step(crate::actor::reactor::reconciler::CoreEvent::Rekeyed { from: old, to: new });
        state.windows_mut().transfer_persistent_window_metadata(old, new);
        assert_eq!(state.windows.gripped(), Some(new));
        assert_eq!(state.windows().workspace_for_window(new), Some(workspace));
        assert!(state.windows_mut().unheld(new).is_none());
        state.windows().debug_assert_invariants();

        state.remove_window(new);
        assert_eq!(state.windows.gripped(), None);
        assert!(state.windows().record(new).is_none());
        state.windows().debug_assert_invariants();
    }

    #[test]
    fn the_grip_ends_when_the_gripped_windows_app_terminates() {
        // Not only when the window itself is destroyed: an app that quits mid-press or mid-drag
        // takes its windows' records with it, and the slot is one of the places they live.
        for dragging in [false, true] {
            let mut state = crate::model::RiftState::default();
            let wid = WindowId::new(4, 1);
            state.windows.set_visibility(wid, WindowVisibility::Visible);
            state.step(crate::actor::reactor::reconciler::CoreEvent::Press {
                window: Some(wid),
                button: crate::model::drag::MouseButton::Left.into(),
            });
            if dragging {
                let frame = objc2_core_foundation::CGRect::ZERO;
                state.begin_drag(
                    wid,
                    Session::native(
                        crate::model::drag::DragSource {
                            origin_frame: frame,
                            last_frame: frame,
                            origin_space: None,
                            current_space: None,
                            tiled: true,
                        },
                        Default::default(),
                    ),
                );
            }
            assert_eq!(state.windows.gripped(), Some(wid));
            assert_eq!(matches!(state.windows.pointer(), Pointer::Dragging(_)), dragging);

            state.remove_windows_for_app(wid.pid);

            assert_eq!(state.windows.gripped(), None, "dragging: {dragging}");
            assert_eq!(state.windows.pointer(), Pointer::Idle);
            assert!(state.windows().record(wid).is_none());
            state.windows().debug_assert_invariants();
        }
    }

    #[test]
    fn app_cleanup_removes_all_indexes_and_pending_transfers() {
        let mut store = OwnedWindows::default();
        let wid = WindowId::new(4, 1);
        let wsid = WindowServerId::new(55);
        let mut workspaces = WorkspaceStore::new();
        let workspace_id = workspaces
            .create_workspace(
                &mut store.view_mut(),
                SpaceId::new(9),
                Some("Cleanup".to_string()),
            )
            .expect("workspace");
        store.view_mut().track_window_server_id(wsid, wid);
        store.view_mut().assign_window_to_workspace(wid, workspace_id);
        store.view_mut().unheld(wid).unwrap().begin_transfer(true);

        store.view_mut().remove_windows_for_app(wid.pid);

        assert!(store.view().record(wid).is_none());
        assert_eq!(store.tracked_window_id(wsid), None);
        assert!(store.window_ids_for_pid(wid.pid).next().is_none());
        store.view().debug_assert_invariants();
    }

    #[test]
    fn remap_space_merges_into_windows_already_on_the_new_space() {
        let mut store = OwnedWindows::default();
        let (old, new) = (SpaceId::new(1), SpaceId::new(2));
        let mut workspaces = WorkspaceStore::new();
        let workspace_id = workspaces
            .create_workspace(&mut store.view_mut(), old, Some("Shared".to_string()))
            .unwrap();
        let (moved, present) = (WindowId::new(5, 1), WindowId::new(5, 2));
        store.view_mut().assign_window_to_workspace(moved, workspace_id);
        // The window server placed this one on the incoming id before rift heard of it.
        store.view_mut().assign_window_to_workspace(present, workspace_id);

        store.view_mut().suspend_window_to_native_fullscreen(
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

        assert_eq!(store.view().workspace_for_window(moved), Some(workspace_id));
        assert_eq!(store.view().workspace_for_window(present), Some(workspace_id));
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
        assert_eq!(store.view().workspace_window_count(workspace_id), 2);
    }

    #[test]
    fn the_core_follows_every_ownership_the_store_gives() {
        let mut store = OwnedWindows::default();
        let space = SpaceId::new(10);
        let mut workspaces = WorkspaceStore::new();
        // Removed workspaces are gone for good: each step after one uses a workspace still there.
        let [a, b, c, d, e, f] = ["A", "B", "C", "D", "E", "F"].map(|name| {
            workspaces
                .create_workspace(&mut store.view_mut(), space, Some(name.into()))
                .unwrap()
        });
        let id = |idx| WindowId::new(1, idx);
        let wsid = WindowServerId::new;
        let owner = ClaimOwner(7);
        let agree = |store: &OwnedWindows, step: &str| {
            assert_eq!(store.view().ownership_divergences(), vec![], "after {step}");
        };

        store.view_mut().insert_window(id(1), admitted(id(1), wsid(1)));
        store.view_mut().assign_window_to_workspace(id(1), a);
        agree(&store, "assign");
        store.view_mut().assign_window_to_workspace(id(1), b);
        agree(&store, "reassign");
        store.view_mut().insert_claim(id(1), owner, WindowClaimFlags::empty());
        agree(&store, "claim");
        store.view_mut().assign_window_to_workspace(id(1), c);
        agree(&store, "a claimed window's destination");
        store
            .view_mut()
            .set_claim_flags(id(1), WindowClaimFlags::SUPPRESS_FOCUS_FOLLOWS_MOUSE);
        agree(&store, "claim flags");
        store.view_mut().remove_workspace(c, Successor::Into(a));
        agree(&store, "a claim's destination handed on");
        store.view_mut().remove_workspace(a, Successor::Unassign);
        agree(&store, "a claim's destination removed");
        store.view_mut().remove_claim(id(1));
        agree(&store, "release");

        store.view_mut().insert_window(id(2), admitted(id(2), wsid(2)));
        store.view_mut().assign_window_to_workspace(id(2), b);
        store.view_mut().remove_workspace(b, Successor::Into(d));
        agree(&store, "an assignment handed on");
        store.view_mut().remove_workspace(d, Successor::Unassign);
        agree(&store, "an assignment removed");
        store.view_mut().assign_window_to_workspace(id(2), e);
        store.view_mut().remove_window_assignment(id(2));
        agree(&store, "unassign");

        // An identity merge moves ownership to the recognized identity.
        store.view_mut().assign_window_to_workspace(id(2), e);
        store.view_mut().insert_window(id(3), admitted(id(3), wsid(3)));
        store.view_mut().assign_window_to_workspace(id(3), f);
        store.view_mut().insert_claim(id(3), owner, WindowClaimFlags::empty());
        store.view_mut().reconcile_ax_identity(id(3), Some(wsid(2)));
        agree(&store, "identity merge");
        store.view_mut().remove_window(id(3));
        store.view_mut().remove_window(id(2));
        agree(&store, "removal");

        // A write around the store is caught: the claims index naming a window the core does
        // not hold as claimed.
        store.view_mut().insert_window(id(4), admitted(id(4), wsid(4)));
        store.claims.entry(ClaimOwner(9)).or_default().push(id(4));
        assert!(matches!(store.view().ownership_divergences().as_slice(), [
            OwnershipDivergence { window, .. }
        ] if *window == id(4)));
    }
}
