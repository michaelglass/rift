use objc2_core_foundation::CGSize;
use rift_core::workspaces::Successor;
use serde::{Deserialize, Serialize};
use slotmap::new_key_type;
use tracing::{error, warn};

use crate::actor::app::WindowId;
use crate::common::collections::{HashMap, HashSet};
#[cfg(test)]
use crate::common::config::AppWorkspaceRule;
use crate::common::config::{
    LayoutMode, LayoutSettings, MAX_WORKSPACES, VirtualWorkspaceSettings, WorkspaceLayoutRule,
    WorkspaceMode, WorkspaceSelector,
};
use crate::common::log::trace_misc;
use crate::layout_engine::systems::{self, LayoutSystemKind};
use crate::layout_engine::workspaces::WorkspaceLayoutState;
use crate::layout_engine::{Direction, LayoutId, LayoutSystem};
use crate::model::app_rules::{AppRuleDecision, AppRuleEffects, AppRuleRejection, AppRuleResult};
use crate::model::slot_table::{SavedSlot, SlotTable};
use crate::model::window_store::{Claimant, Windows, WindowsMut};
use crate::sys::screen::SpaceId;

new_key_type! {
    pub struct VirtualWorkspaceId;
}

/// Resolution consumes accepted Reactor topology, never the full native inventory.
#[derive(Clone, Copy)]
pub(crate) struct WorkspaceContext<'a> {
    pub space: SpaceId,
    pub visible: &'a [SpaceId],
}

/// A workspace context as the core reads it.
#[cfg(feature = "roc-core")]
use crate::roc_path::Word;
use crate::roc_path::{Asked, AskedAbout};

fn core_context(context: WorkspaceContext<'_>) -> rift_core::workspaces::Context<'_, SpaceId> {
    rift_core::workspaces::Context {
        space: context.space,
        visible: context.visible,
    }
}

impl From<SpaceId> for WorkspaceContext<'_> {
    fn from(space: SpaceId) -> Self { Self { space, visible: &[] } }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorkspaceTarget {
    pub id: VirtualWorkspaceId,
    pub space: SpaceId,
}

/// Start with contiguous blocks, then balance any uneven loads the pins forced
/// into separate blocks. Pins are hard constraints throughout.
fn distribute(displays: usize, pins: Vec<Option<usize>>) -> Vec<usize> {
    if pins.iter().all(Option::is_none) {
        let (share, extra) = (pins.len() / displays, pins.len() % displays);
        return (0..displays)
            .flat_map(|owner| std::iter::repeat_n(owner, share + usize::from(owner < extra)))
            .collect();
    }
    let mut pinned = vec![0; displays];
    pins.iter().flatten().for_each(|&d| pinned[d] += 1);
    let bare: Vec<_> = (0..displays)
        .filter(|&d| pinned[d] == 0)
        .take(pins.iter().filter(|pin| pin.is_none()).count())
        .collect();

    // (number of runs, balance penalty, owners); tuple ordering supplies a stable
    // physical left-to-right tie break without reordering workspace identities.
    type Allocation = (usize, usize, Vec<usize>);
    struct Allocator<'a> {
        pins: &'a [Option<usize>],
        pinned: &'a [usize],
        bare: &'a [usize],
        memo: HashMap<(usize, usize, usize), Option<Allocation>>,
    }
    impl Allocator<'_> {
        fn solve(&mut self, start: usize, used: usize, previous: usize) -> Option<Allocation> {
            if start == self.pins.len() {
                return (used == self.bare.len()).then(|| (0, 0, Vec::new()));
            }
            let key = (start, used, previous);
            if let Some(result) = self.memo.get(&key) {
                return result.clone();
            }
            let mut best = None;
            for owner in 0..self.pinned.len() {
                if owner == previous {
                    continue;
                }
                let unpinned = self.pinned[owner] == 0;
                if unpinned && self.bare.get(used) != Some(&owner) {
                    continue;
                }
                let mut pins_in_run = 0;
                for end in start..self.pins.len() {
                    if self.pins[end].is_some_and(|pin| pin != owner) {
                        break;
                    }
                    pins_in_run += usize::from(self.pins[end].is_some());
                    let Some((runs, balance, tail)) =
                        self.solve(end + 1, used + usize::from(unpinned), owner)
                    else {
                        continue;
                    };
                    let len = end + 1 - start;
                    let mut owners = Vec::with_capacity(len + tail.len());
                    owners.resize(len, owner);
                    owners.extend(tail);
                    let candidate = (
                        runs + 1,
                        balance + len * len + 2 * len * (self.pinned[owner] - pins_in_run),
                        owners,
                    );
                    if best.as_ref().is_none_or(|best| &candidate < best) {
                        best = Some(candidate);
                    }
                }
            }
            self.memo.insert(key, best.clone());
            best
        }
    }
    let owners = Allocator {
        pins: &pins,
        pinned: &pinned,
        bare: &bare,
        memo: HashMap::default(),
    }
    .solve(0, 0, displays)
    .expect("pins and available unassigned workspaces admit a distribution")
    .2;

    balance_owners(displays, &pins, owners)
}

/// Each move strictly reduces squared load and moves only an unpinned workspace.
fn balance_owners(displays: usize, pins: &[Option<usize>], mut owners: Vec<usize>) -> Vec<usize> {
    let mut counts = vec![0; displays];
    for &owner in &owners {
        counts[owner] += 1;
    }
    let mut movable = vec![0; displays];
    for (&owner, pin) in owners.iter().zip(pins) {
        movable[owner] += usize::from(pin.is_none());
    }
    loop {
        let Some(target) = (0..displays).min_by_key(|&display| counts[display]) else {
            break;
        };
        let Some(source) = (0..displays)
            .filter(|&display| movable[display] > 0 && counts[display] > counts[target] + 1)
            .max_by_key(|&display| counts[display])
        else {
            break;
        };
        let mut eligible =
            (0..owners.len()).filter(|&index| owners[index] == source && pins[index].is_none());
        let index = if source < target {
            eligible.next_back()
        } else {
            eligible.next()
        };
        owners[index.expect("source has a movable workspace")] = target;
        counts[source] -= 1;
        counts[target] += 1;
        movable[source] -= 1;
        movable[target] += 1;
    }
    owners
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceTransferReason {
    Explicit,
    DisplayArrival,
    DisplayDeparture,
    Restoration,
}

impl std::fmt::Display for VirtualWorkspaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let dbg = format!("{:?}", self);
        let digits: String = dbg.chars().filter(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse::<u64>() {
            write!(f, "{:08}", n)
        } else {
            write!(f, "{}", dbg)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceError {
    NoWorkspacesAvailable,
    AssignmentFailed,
    InvalidWorkspaceId(VirtualWorkspaceId),
    InvalidWorkspaceIndex(usize),
    InconsistentState(String),
}

/// Workspace-local configuration and layout state.
///
/// This intentionally does not own the set of member windows. Window-to-
/// workspace assignment is authoritative in `WindowStore`; the layout system
/// here is only the arrangement for the windows currently projected into this
/// workspace.
#[derive(Debug)]
pub struct VirtualWorkspace {
    pub name: String,
    /// Where the workspace belongs when topology is not moving it.
    pub home: Home,
    /// Whether the workspace follows its display rule, if one matches.
    pub rule_standing: RuleStanding,
    last_focused: Option<WindowId>,
    pub layout_system: LayoutSystemKind,
    pub layout_mode: LayoutMode,
    pub(crate) layout_state: WorkspaceLayoutState,
}

/// Where a workspace belongs when topology is not moving it: the Space (and the display it was
/// on) it was created on or moved to, and who chose it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Home {
    /// Never placed: saved before homes were recorded.
    Unplaced,
    /// A Space, with no display preference (created where no workspace had one).
    Space { space: SpaceId, by: PlacedBy },
    /// A Space on a display: created beside a workspace with that display preference, moved
    /// there explicitly, or handed to a display as it arrived.
    OnDisplay {
        space: SpaceId,
        display: String,
        by: PlacedBy,
    },
    /// A display preference with no Space, as older versions saved it.
    DisplayOnly(String),
}

/// Who chose a placed workspace's home. Automatic display balancing moves only the workspaces
/// rift placed itself: a home the user chose (an explicit display move) is kept.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PlacedBy {
    /// Rift: created there, or handed to an arriving display.
    #[default]
    Automatic,
    /// The user, by an explicit display move.
    User,
}

impl Home {
    /// The home of a workspace placed on `space`, on `display` if it has one.
    pub fn placed(space: SpaceId, display: Option<String>, by: PlacedBy) -> Home {
        match display {
            Some(display) => Home::OnDisplay { space, display, by },
            None => Home::Space { space, by },
        }
    }

    /// From the saved form's fields. `display_manually_selected` belongs to a placed home; on a
    /// home with no Space (never written so, only by hand) it is dropped, with one log line.
    fn from_saved(
        space: Option<SpaceId>,
        display: Option<String>,
        display_manually_selected: bool,
    ) -> Home {
        let by = if display_manually_selected {
            PlacedBy::User
        } else {
            PlacedBy::Automatic
        };
        match (space, display) {
            (Some(space), display) => Home::placed(space, display, by),
            (None, saved_display) => {
                if display_manually_selected {
                    warn!(
                        "workspace-load: normalized: dropped display_manually_selected on a workspace with no home Space (display {:?})",
                        saved_display.as_deref()
                    );
                }
                match saved_display {
                    None => Home::Unplaced,
                    Some(display) => Home::DisplayOnly(display),
                }
            }
        }
    }

    pub fn space(&self) -> Option<SpaceId> {
        match self {
            Home::Space { space, .. } | Home::OnDisplay { space, .. } => Some(*space),
            Home::Unplaced | Home::DisplayOnly(_) => None,
        }
    }

    pub fn display(&self) -> Option<&str> {
        match self {
            Home::OnDisplay { display, .. } | Home::DisplayOnly(display) => Some(display),
            Home::Unplaced | Home::Space { .. } => None,
        }
    }

    /// Who chose a placed home; `None` for a home with no Space.
    pub fn placed_by(&self) -> Option<PlacedBy> {
        match self {
            Home::Space { by, .. } | Home::OnDisplay { by, .. } => Some(*by),
            Home::Unplaced | Home::DisplayOnly(_) => None,
        }
    }

    /// The Space and display of a home on a display.
    pub fn on_display(&self) -> Option<(SpaceId, &str)> {
        match self {
            Home::OnDisplay { space, display, .. } => Some((*space, display)),
            Home::Unplaced | Home::Space { .. } | Home::DisplayOnly(_) => None,
        }
    }

    /// The same home with no display preference.
    #[cfg(test)]
    pub fn without_display(&self) -> Home {
        match *self {
            Home::Space { space, by } | Home::OnDisplay { space, by, .. } => {
                Home::Space { space, by }
            }
            Home::Unplaced | Home::DisplayOnly(_) => Home::Unplaced,
        }
    }

    /// The same home on another Space (the display preference and who chose it, if any, kept).
    /// The same home, as rift's own placement: automatic balancing may move it again.
    pub fn automatic(&self) -> Home {
        match self.space() {
            Some(space) => {
                Home::placed(space, self.display().map(str::to_owned), PlacedBy::Automatic)
            }
            None => self.clone(),
        }
    }

    pub fn with_space(&self, space: SpaceId) -> Home {
        Home::placed(
            space,
            self.display().map(str::to_owned),
            self.placed_by().unwrap_or_default(),
        )
    }
}

/// Whether a workspace follows the display rule that matches it, if any. An explicit move away
/// from the rule's display overrides it until the workspace returns there or the rule is
/// (re)activated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RuleStanding {
    #[default]
    Followed,
    Overridden,
}

/// A workspace as it is saved: the form every version has written, parsed into the workspace's
/// types on load (`From<SavedWorkspace>`) and written back field for field.
#[derive(Deserialize)]
#[serde(rename = "VirtualWorkspace")]
struct SavedWorkspace {
    name: String,
    space: SpaceId,
    #[serde(default)]
    preferred_display: Option<String>,
    #[serde(default)]
    home_space: Option<SpaceId>,
    /// Explicit display moves keep their destination during automatic balancing.
    #[serde(default)]
    display_manually_selected: bool,
    #[serde(default)]
    rule_overridden: bool,
    last_focused: Option<WindowId>,
    #[serde(default = "default_layout_system_kind")]
    layout_system: LayoutSystemKind,
    #[serde(default)]
    layout_mode: LayoutMode,
    #[serde(default)]
    layout_state: WorkspaceLayoutState,
}

/// [`SavedWorkspace`] borrowed from a workspace, for writing.
#[derive(Serialize)]
#[serde(rename = "VirtualWorkspace")]
pub(crate) struct SavedWorkspaceRef<'a> {
    name: &'a String,
    space: SpaceId,
    preferred_display: Option<&'a str>,
    home_space: Option<SpaceId>,
    display_manually_selected: bool,
    rule_overridden: bool,
    last_focused: Option<WindowId>,
    layout_system: &'a LayoutSystemKind,
    layout_mode: LayoutMode,
    layout_state: &'a WorkspaceLayoutState,
}

impl SavedWorkspace {
    /// The saved record split into the workspace and the Space it is on (the core's).
    fn split(self) -> (VirtualWorkspace, SpaceId) {
        let space = self.space;
        (VirtualWorkspace::from(self), space)
    }
}

impl From<SavedWorkspace> for VirtualWorkspace {
    fn from(saved: SavedWorkspace) -> Self {
        VirtualWorkspace {
            name: saved.name,
            home: Home::from_saved(
                saved.home_space,
                saved.preferred_display,
                saved.display_manually_selected,
            ),
            rule_standing: if saved.rule_overridden {
                RuleStanding::Overridden
            } else {
                RuleStanding::Followed
            },
            last_focused: saved.last_focused,
            layout_system: saved.layout_system,
            layout_mode: saved.layout_mode,
            layout_state: saved.layout_state,
        }
    }
}

impl VirtualWorkspace {
    /// The workspace as it is saved, on `space` (the core's).
    pub(crate) fn saved(&self, space: SpaceId) -> SavedWorkspaceRef<'_> {
        SavedWorkspaceRef {
            name: &self.name,
            space,
            preferred_display: self.home.display(),
            home_space: self.home.space(),
            display_manually_selected: self.home.placed_by() == Some(PlacedBy::User),
            rule_overridden: self.rule_standing == RuleStanding::Overridden,
            last_focused: self.last_focused,
            layout_system: &self.layout_system,
            layout_mode: self.layout_mode,
            layout_state: &self.layout_state,
        }
    }
}

impl<'de> Deserialize<'de> for VirtualWorkspace {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        SavedWorkspace::deserialize(deserializer).map(VirtualWorkspace::from)
    }
}

fn default_layout_system_kind() -> LayoutSystemKind {
    VirtualWorkspace::create_layout_system(LayoutMode::default(), &LayoutSettings::default())
}

impl VirtualWorkspace {
    fn new(name: String, mode: LayoutMode, settings: &LayoutSettings) -> Self {
        let layout_system = Self::create_layout_system(mode, settings);
        Self {
            name,
            home: Home::Unplaced,
            rule_standing: RuleStanding::Followed,
            last_focused: None,
            layout_system,
            layout_mode: mode,
            layout_state: WorkspaceLayoutState::default(),
        }
    }

    pub(crate) fn active_layout(&self) -> Option<LayoutId> { self.layout_state.active() }

    pub(crate) fn ensure_layout_for_size(&mut self, size: CGSize) {
        self.layout_state.ensure_active(size, &mut self.layout_system);
    }

    pub(crate) fn replace_layout_mode(
        &mut self,
        mode: LayoutMode,
        settings: &LayoutSettings,
    ) -> LayoutId {
        self.layout_mode = mode;
        self.layout_system = Self::create_layout_system(mode, settings);
        let layout = self.layout_system.create_layout();
        self.layout_state.replace(layout);
        layout
    }

    pub fn layout_mode(&self) -> LayoutMode { self.layout_mode }

    pub fn create_layout_system(mode: LayoutMode, settings: &LayoutSettings) -> LayoutSystemKind {
        let mut system = match mode {
            LayoutMode::Traditional => systems::TraditionalLayoutSystem::default().into(),
            LayoutMode::Floating => systems::FloatingLayoutSystem::default().into(),
            LayoutMode::Bsp => systems::BspLayoutSystem::default().into(),
            LayoutMode::Stack => systems::StackLayoutSystem::default().into(),
            LayoutMode::MasterStack => systems::MasterStackLayoutSystem::default().into(),
            LayoutMode::Scrolling => systems::ScrollingLayoutSystem::default().into(),
        };
        Self::configure_layout_system(&mut system, mode, settings, None);
        system
    }

    pub(crate) fn apply_layout_settings(
        &mut self,
        settings: &LayoutSettings,
        display: Option<&str>,
    ) {
        Self::configure_layout_system(&mut self.layout_system, self.layout_mode, settings, display);
    }

    fn configure_layout_system(
        system: &mut LayoutSystemKind,
        mode: LayoutMode,
        settings: &LayoutSettings,
        display: Option<&str>,
    ) {
        let insertion_point = settings.window_insertion_point_for(mode);
        match system {
            LayoutSystemKind::Traditional(system) => {
                system.set_window_insertion_point(insertion_point);
                system.set_equalize_nodes(settings.traditional.equalize_nodes);
            }
            LayoutSystemKind::Floating(system) => {
                system.set_window_insertion_point(insertion_point);
            }
            LayoutSystemKind::Bsp(system) => {
                system.set_window_insertion_point(insertion_point);
                system.set_single_window_aspect_ratio(settings.bsp.single_window_aspect_ratio);
            }
            LayoutSystemKind::Stack(system) => {
                system.update_settings(settings.stack.default_orientation, insertion_point);
            }
            LayoutSystemKind::MasterStack(system) => {
                let mut mode_settings = settings.master_stack.clone();
                mode_settings.base = settings.resolved_base_for(mode);
                system.update_settings(mode_settings);
            }
            LayoutSystemKind::Scrolling(system) => {
                let mut mode_settings = settings.scrolling.effective_for_display(display);
                mode_settings.base = settings.resolved_base_for(mode);
                system.update_settings(&mode_settings);
            }
        }
    }

    /// A restore replaces layout contents, not the workspace's identity: name, native owner and
    /// home are current-session state that an old snapshot must not overwrite.
    pub(crate) fn restore_contents(&mut self, saved: Self) {
        self.last_focused = saved.last_focused;
        self.layout_system = saved.layout_system;
        self.layout_mode = saved.layout_mode;
        self.layout_state = saved.layout_state;
    }

    pub fn set_last_focused(&mut self, window_id: Option<WindowId>) {
        self.last_focused = window_id;
    }

    pub fn last_focused(&self) -> Option<WindowId> { self.last_focused }
}

/// What a native Space shows. Current ownership is never inferred from home intent: a
/// departed display can remember its presentation without keeping a foreign workspace in its
/// active slot, and show a fallback meanwhile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceActivation {
    /// Showing `current`. `previous` is navigation history (global mode can return to a
    /// workspace on another display), none yet if `None`.
    Showing {
        current: VirtualWorkspaceId,
        previous: Option<VirtualWorkspaceId>,
        remembered: Option<VirtualWorkspaceId>,
    },
    /// Showing nothing.
    Vacant {
        remembered: Option<VirtualWorkspaceId>,
    },
}
// In both, `remembered` is the presentation a departed display takes back when that
// workspace's ownership returns; none if `None`.

impl SpaceActivation {
    /// The same activation in the core's terms.
    pub(crate) fn to_core(self) -> rift_core::workspaces::Activation<VirtualWorkspaceId> {
        use rift_core::workspaces::Activation;
        match self {
            SpaceActivation::Showing { current, previous, remembered } => {
                Activation::Showing { current, previous, remembered }
            }
            SpaceActivation::Vacant { remembered } => Activation::Vacant { remembered },
        }
    }
}

impl SpaceActivation {
    /// The core's activation in the platform's terms.
    pub(crate) fn from_core(
        activation: rift_core::workspaces::Activation<VirtualWorkspaceId>,
    ) -> Self {
        use rift_core::workspaces::Activation;
        match activation {
            Activation::Showing { current, previous, remembered } => {
                SpaceActivation::Showing { current, previous, remembered }
            }
            Activation::Vacant { remembered } => SpaceActivation::Vacant { remembered },
        }
    }
}

impl Default for SpaceActivation {
    fn default() -> Self { SpaceActivation::Vacant { remembered: None } }
}

impl SpaceActivation {
    fn showing(id: VirtualWorkspaceId) -> Self {
        SpaceActivation::Showing {
            current: id,
            previous: None,
            remembered: None,
        }
    }

    pub fn current(&self) -> Option<VirtualWorkspaceId> {
        match *self {
            SpaceActivation::Showing { current, .. } => Some(current),
            SpaceActivation::Vacant { .. } => None,
        }
    }

    pub fn previous(&self) -> Option<VirtualWorkspaceId> {
        match *self {
            SpaceActivation::Showing { previous, .. } => previous,
            SpaceActivation::Vacant { .. } => None,
        }
    }

    pub fn remembered(&self) -> Option<VirtualWorkspaceId> {
        match *self {
            SpaceActivation::Showing { remembered, .. }
            | SpaceActivation::Vacant { remembered } => remembered,
        }
    }

    fn remembered_mut(&mut self) -> &mut Option<VirtualWorkspaceId> {
        match self {
            SpaceActivation::Showing { remembered, .. }
            | SpaceActivation::Vacant { remembered } => remembered,
        }
    }

    /// Shows `id`, the workspace shown until now becoming the previous one.
    fn activate(&mut self, id: VirtualWorkspaceId) {
        *self = match *self {
            SpaceActivation::Showing { current, previous, remembered } if current == id => {
                SpaceActivation::Showing { current, previous, remembered }
            }
            SpaceActivation::Showing { current, remembered, .. } => SpaceActivation::Showing {
                current: id,
                previous: Some(current),
                remembered,
            },
            SpaceActivation::Vacant { remembered } => SpaceActivation::Showing {
                current: id,
                previous: None,
                remembered,
            },
        };
    }

    /// Remembers `id`, to take it back when its ownership returns.
    fn remember(&mut self, id: VirtualWorkspaceId) { *self.remembered_mut() = Some(id); }

    /// Forgets the remembered presentation, whichever it is.
    pub(crate) fn forget_memory(&mut self) { *self.remembered_mut() = None; }

    /// Keeps only history and memory that `keep` accepts.
    fn retain(&mut self, keep: impl Fn(VirtualWorkspaceId) -> bool) {
        if let SpaceActivation::Showing { previous, .. } = self {
            *previous = previous.filter(|&id| keep(id));
        }
        let remembered = self.remembered_mut();
        *remembered = remembered.filter(|&id| keep(id));
    }
}

/// A Space's activation as it is saved: three independent fields (and, older still, a pair).
#[derive(Deserialize)]
#[serde(rename = "SpaceActivation")]
struct SavedActivation {
    current: Option<VirtualWorkspaceId>,
    previous: Option<VirtualWorkspaceId>,
    remembered: Option<VirtualWorkspaceId>,
}

/// [`SavedActivation`] from an activation, for writing.
#[derive(Serialize)]
#[serde(rename = "SpaceActivation")]
struct SavedActivationRef {
    current: Option<VirtualWorkspaceId>,
    previous: Option<VirtualWorkspaceId>,
    remembered: Option<VirtualWorkspaceId>,
}

impl Serialize for SpaceActivation {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        SavedActivationRef {
            current: self.current(),
            previous: self.previous(),
            remembered: self.remembered(),
        }
        .serialize(serializer)
    }
}

impl SpaceActivation {
    /// A saved activation, as a Space can hold it. History with nothing shown is dropped, with
    /// one log line: it means a mid-operation state was saved.
    fn from_saved(space: SpaceId, saved: SavedActivation) -> SpaceActivation {
        match saved {
            SavedActivation {
                current: Some(current),
                previous,
                remembered,
            } => SpaceActivation::Showing { current, previous, remembered },
            SavedActivation {
                current: None,
                previous,
                remembered,
            } => {
                if let Some(previous) = previous {
                    warn!(
                        space = space.get(),
                        ?previous,
                        "workspace-load: normalized: dropped navigation history on a Space showing nothing"
                    );
                }
                SpaceActivation::Vacant { remembered }
            }
        }
    }
}

fn deserialize_activations<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<HashMap<SpaceId, SpaceActivation>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Snapshot {
        Current(SavedActivation),
        Legacy(Option<VirtualWorkspaceId>, VirtualWorkspaceId),
    }
    Ok(HashMap::<SpaceId, Snapshot>::deserialize(deserializer)?
        .into_iter()
        .map(|(space, saved)| {
            let activation = match saved {
                Snapshot::Current(saved) => SpaceActivation::from_saved(space, saved),
                Snapshot::Legacy(previous, current) => SpaceActivation::Showing {
                    current,
                    previous,
                    remembered: None,
                },
            };
            (space, activation)
        })
        .collect())
}

pub(crate) struct WorkspaceSettingsPlan {
    names: Vec<(VirtualWorkspaceId, String)>,
    additions: Vec<(SpaceId, String)>,
}

/// Owns virtual workspace topology and workspace-scoped layout configurations.
///
/// Membership is single-source-of-truth in `WindowStore`. Any code that
/// needs to answer "which workspace owns this window?" or "which windows belong
/// to this workspace?" must go through the store-backed helpers on this
/// manager. Keeping the mapping out of `VirtualWorkspace` prevents stale
/// duplicated membership from surviving topology churn or discovery refreshes.
#[derive(Debug)]
pub struct WorkspaceStore {
    /// Every workspace, in catalog order.
    pub(crate) workspaces: Catalog,
    /// Last accepted physical topology also scopes configuration during restore.
    single_display: bool,
    pub(crate) settings: VirtualWorkspaceSettings,
    pub layout_settings: LayoutSettings,
}

/// A Space's workspaces as a query sees them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listing {
    /// The Space is initialised: its workspaces, with their names.
    Known(Vec<(VirtualWorkspaceId, String)>),
    /// No event has initialised the Space yet (it has never been exposed, nor had a window
    /// placed on it).
    NotYetKnown,
}

/// Every workspace's own data, by id (its place in the catalog order, and what Space it is on,
/// are the core's). Iteration is in slot order.
#[derive(Debug, Default)]
pub struct Catalog {
    workspaces: SlotTable<VirtualWorkspaceId, VirtualWorkspace>,
}

/// Why a saved catalog can't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    /// The order names a workspace the catalog doesn't have.
    Unknown(VirtualWorkspaceId),
    /// The order names a workspace twice.
    Duplicate(VirtualWorkspaceId),
    /// The order leaves a workspace out.
    Unlisted {
        id: VirtualWorkspaceId,
        space: SpaceId,
    },
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CatalogError::Unknown(id) => {
                write!(f, "invalid or duplicate workspace {id:?} in catalog")
            }
            CatalogError::Duplicate(id) => {
                write!(f, "invalid or duplicate workspace {id:?} in catalog")
            }
            CatalogError::Unlisted { id, space } => {
                write!(
                    f,
                    "workspace {id:?} for native space {} is not indexed",
                    space.get()
                )
            }
        }
    }
}

impl Catalog {
    /// Saved records and order as one catalog and its order, each workspace with its Space: the
    /// order must name every workspace, each once. Each record splits into the workspace and its
    /// Space, which only the order (the core's catalog) keeps.
    fn parse(
        records: SlotTable<VirtualWorkspaceId, SavedWorkspace>,
        order: Vec<VirtualWorkspaceId>,
    ) -> Result<(Catalog, Vec<(VirtualWorkspaceId, SpaceId)>), CatalogError> {
        let mut listed = HashSet::default();
        for &id in &order {
            if !records.contains_key(id) {
                return Err(CatalogError::Unknown(id));
            }
            if !listed.insert(id) {
                return Err(CatalogError::Duplicate(id));
            }
        }
        if let Some((id, record)) = records.iter().find(|(id, _)| !listed.contains(id)) {
            return Err(CatalogError::Unlisted { id, space: record.space });
        }
        let order = order.into_iter().map(|id| (id, records[id].space)).collect();
        let workspaces = records
            .map(|_, record| Ok::<_, std::convert::Infallible>(record.split().0))
            .unwrap_or_else(|never| match never {});
        Ok((Catalog { workspaces }, order))
    }

    /// Adds a workspace (the core puts it at the end of the catalog order).
    pub fn insert(&mut self, workspace: VirtualWorkspace) -> VirtualWorkspaceId {
        self.workspaces.insert(workspace)
    }

    /// Takes a workspace out.
    pub fn remove(&mut self, id: VirtualWorkspaceId) -> Option<VirtualWorkspace> {
        self.workspaces.remove(id)
    }

    pub fn get(&self, id: VirtualWorkspaceId) -> Option<&VirtualWorkspace> {
        self.workspaces.get(id)
    }

    pub fn get_mut(&mut self, id: VirtualWorkspaceId) -> Option<&mut VirtualWorkspace> {
        self.workspaces.get_mut(id)
    }

    pub fn contains_key(&self, id: VirtualWorkspaceId) -> bool { self.workspaces.contains_key(id) }

    pub fn len(&self) -> usize { self.workspaces.len() }

    pub fn is_empty(&self) -> bool { self.len() == 0 }

    pub fn iter(&self) -> impl Iterator<Item = (VirtualWorkspaceId, &VirtualWorkspace)> {
        self.workspaces.iter()
    }

    pub fn iter_mut(
        &mut self,
    ) -> impl Iterator<Item = (VirtualWorkspaceId, &mut VirtualWorkspace)> {
        self.workspaces.iter_mut()
    }

    pub fn values(&self) -> impl Iterator<Item = &VirtualWorkspace> { self.workspaces.values() }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut VirtualWorkspace> {
        self.workspaces.values_mut()
    }

    pub fn keys(&self) -> impl Iterator<Item = VirtualWorkspaceId> + '_ { self.workspaces.keys() }
}

impl std::ops::Index<VirtualWorkspaceId> for Catalog {
    type Output = VirtualWorkspace;

    fn index(&self, id: VirtualWorkspaceId) -> &VirtualWorkspace { &self.workspaces[id] }
}

impl std::ops::IndexMut<VirtualWorkspaceId> for Catalog {
    fn index_mut(&mut self, id: VirtualWorkspaceId) -> &mut VirtualWorkspace {
        &mut self.workspaces[id]
    }
}

/// The store as it is saved: the workspace records (slotmap's slot sequence) and the catalog
/// order as two fields (older files group the order by Space).
#[derive(Deserialize)]
#[serde(rename = "WorkspaceStore")]
struct SavedStore {
    workspaces: Vec<SavedSlot<SavedWorkspace>>,
    #[serde(
        alias = "workspaces_by_space",
        deserialize_with = "deserialize_workspace_order"
    )]
    order: Vec<VirtualWorkspaceId>,
    #[serde(default)]
    single_display: bool,
    #[serde(deserialize_with = "deserialize_activations")]
    active_workspace_per_space: HashMap<SpaceId, SpaceActivation>,
}

/// [`SavedStore`] written from a store and the core: the store's workspaces, each with its Space
/// as the core holds it, and the topology (the catalog order and the activations) as the core
/// holds it.
#[derive(Serialize)]
#[serde(rename = "WorkspaceStore")]
pub(crate) struct SavedStoreRef<'a> {
    workspaces: SavedRecords<'a>,
    order: Vec<VirtualWorkspaceId>,
    single_display: bool,
    active_workspace_per_space: HashMap<SpaceId, SpaceActivation>,
}

/// The catalog's slots, each workspace written with the Space `spaces` names for it.
pub(crate) struct SavedRecords<'a> {
    catalog: &'a Catalog,
    spaces: HashMap<VirtualWorkspaceId, SpaceId>,
}

impl Serialize for SavedRecords<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // One holder: the catalog holds every workspace the core does (its writers tell the core
        // in the same step), so each record has a Space.
        serializer.collect_seq(crate::model::slot_table::saved(
            &self.catalog.workspaces,
            |id, workspace| workspace.saved(self.spaces[&id]),
        ))
    }
}

/// The core's joined workspaces, as the platform's types hold them.
pub(crate) type CoreWorkspaces =
    rift_core::workspaces::Workspaces<WindowId, VirtualWorkspaceId, SpaceId, Claimant>;

impl WorkspaceStore {
    /// The store as it is saved: its workspaces, with the topology written from `core`.
    pub(crate) fn saved<'a>(&'a self, core: &CoreWorkspaces) -> SavedStoreRef<'a> {
        SavedStoreRef {
            workspaces: SavedRecords {
                catalog: &self.workspaces,
                spaces: core.catalog().iter().copied().collect(),
            },
            order: core.catalog().iter().map(|&(id, _)| id).collect(),
            single_display: self.single_display,
            active_workspace_per_space: core
                .activations()
                .iter()
                .map(|&(space, activation)| (space, SpaceActivation::from_core(activation)))
                .collect(),
        }
    }
}

/// The workspaces' topology as a saved file holds it, parsed at the load boundary: every
/// workspace with its Space, in catalog order, and what each initialised Space shows. The core is
/// seeded from it when the stores join ([`crate::model::Stores::joining`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SavedTopology {
    catalog: Vec<(VirtualWorkspaceId, SpaceId)>,
    activations: HashMap<SpaceId, SpaceActivation>,
}

impl SavedTopology {
    /// Every workspace with its Space, in catalog order.
    pub(crate) fn catalog(&self) -> &[(VirtualWorkspaceId, SpaceId)] { &self.catalog }

    /// What each initialised Space shows.
    pub(crate) fn activations(&self) -> &HashMap<SpaceId, SpaceActivation> { &self.activations }

    /// Every workspace, in catalog order.
    pub(crate) fn ids(&self) -> impl Iterator<Item = VirtualWorkspaceId> + '_ {
        self.catalog.iter().map(|&(id, _)| id)
    }

    /// `space`'s workspaces, in catalog order.
    pub(crate) fn workspace_ids(&self, space: SpaceId) -> Vec<VirtualWorkspaceId> {
        self.catalog.iter().filter(|&&(_, on)| on == space).map(|&(id, _)| id).collect()
    }

    /// The workspace `space` shows.
    pub(crate) fn active_workspace(&self, space: SpaceId) -> Option<VirtualWorkspaceId> {
        self.activations.get(&space)?.current()
    }

    /// The Space `id` is on.
    pub(crate) fn space(&self, id: VirtualWorkspaceId) -> Option<SpaceId> {
        self.catalog.iter().find(|&&(listed, _)| listed == id).map(|&(_, space)| space)
    }

    fn contains(&self, id: VirtualWorkspaceId) -> bool { self.space(id).is_some() }

    /// Normalizes the loaded activations, at load: a Space showing a workspace that moved away
    /// (`home` says where each workspace's home is) remembers it and shows its own first one, if
    /// it has one; a Space's history never names what it shows.
    pub(crate) fn migrate_legacy_activation(
        &mut self,
        home: impl Fn(VirtualWorkspaceId) -> Option<SpaceId>,
    ) {
        let spaces: Vec<_> = self.activations.keys().copied().collect();
        for space in spaces {
            let mut activation = self.activations[&space];
            if let Some(id) = activation.current()
                && self.space(id).is_some_and(|on| on != space && home(id) == Some(space))
            {
                let previous = activation.previous();
                let first = self.catalog.iter().find(|&&(_, on)| on == space).map(|&(id, _)| id);
                activation = match first {
                    Some(current) => SpaceActivation::Showing {
                        current,
                        previous,
                        remembered: Some(id),
                    },
                    None => {
                        if let Some(previous) = previous {
                            warn!(
                                space = space.get(),
                                ?previous,
                                "workspace-load: normalized: dropped navigation history on a Space showing nothing"
                            );
                        }
                        SpaceActivation::Vacant { remembered: Some(id) }
                    }
                };
            }
            if let SpaceActivation::Showing { current, previous, .. } = &mut activation
                && *previous == Some(*current)
            {
                *previous = None;
            }
            self.activations.insert(space, activation);
        }
    }

    /// Validate the saved topology before layout code indexes into slotmaps.
    ///
    /// Persistence files are user-visible and may be old, truncated, or manually edited. Loading
    /// malformed topology must return a useful error instead of panicking later through indexing.
    pub(crate) fn validate(&self) -> Result<(), String> {
        for (&space, activation) in &self.activations {
            for (kind, id) in [
                ("active", activation.current()),
                ("previous", activation.previous()),
            ] {
                if id.is_some_and(|id| {
                    !self.contains(id)
                        || (kind == "active" && self.space(id) != Some(space))
                        || (kind == "previous" && Some(id) == activation.current())
                }) {
                    return Err(format!(
                        "native space {} has an invalid {kind} workspace",
                        space.get()
                    ));
                }
            }
            if activation.remembered().is_some_and(|id| !self.contains(id)) {
                return Err(format!(
                    "native space {} remembers a missing workspace",
                    space.get()
                ));
            }
        }

        for &(workspace, space) in &self.catalog {
            if !self.activations.contains_key(&space) {
                return Err(format!(
                    "workspace {workspace:?} for native space {} is not indexed",
                    space.get()
                ));
            }
        }
        Ok(())
    }
}

/// A saved store, parsed: the workspace store, and the topology the core is seeded from.
pub(crate) struct LoadedWorkspaces {
    pub(crate) store: WorkspaceStore,
    pub(crate) topology: SavedTopology,
}

impl<'de> Deserialize<'de> for LoadedWorkspaces {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let saved = SavedStore::deserialize(deserializer)?;
        let records =
            crate::model::slot_table::parse(saved.workspaces).map_err(serde::de::Error::custom)?;
        let (workspaces, catalog) =
            Catalog::parse(records, saved.order).map_err(serde::de::Error::custom)?;
        let topology = SavedTopology {
            catalog,
            activations: saved.active_workspace_per_space,
        };
        let store = WorkspaceStore {
            workspaces,
            single_display: saved.single_display,
            settings: VirtualWorkspaceSettings::default(),
            layout_settings: LayoutSettings::default(),
        };
        Ok(LoadedWorkspaces { store, topology })
    }
}

impl LoadedWorkspaces {
    /// Runs the load-time checks on the saved topology (normalizing, then validating it).
    pub(crate) fn checked(mut self) -> Result<LoadedWorkspaces, String> {
        let store = &self.store;
        self.topology.migrate_legacy_activation(|id| {
            store.workspaces.get(id).and_then(|workspace| workspace.home.space())
        });
        self.topology.validate()?;
        Ok(self)
    }
}

/// A rule's `display` only applies in global mode; say so rather than ignore it silently.
fn warn_on_ignored_display_rules(config: &VirtualWorkspaceSettings) {
    if config.mode == WorkspaceMode::Global {
        return;
    }
    for (index, rule) in config.workspace_rules.iter().enumerate() {
        if rule.display.as_deref().is_some_and(|display| !display.is_empty()) {
            warn!(
                index,
                "Workspace rule display only applies with mode = \"global\""
            );
        }
    }
}

fn selector_matches(selector: &WorkspaceSelector, index: usize, name: &str) -> bool {
    match selector {
        WorkspaceSelector::Index(i) => *i == index,
        WorkspaceSelector::Name(n) => n == name,
    }
}

// Schema 4 uses one ordered catalog. Older snapshots group IDs by native Space;
// migrate deterministically, preserving the legacy ordinal contract.
fn deserialize_workspace_order<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<VirtualWorkspaceId>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = Vec<VirtualWorkspaceId>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("workspace catalog or legacy Space index")
        }

        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
            Vec::deserialize(serde::de::value::SeqAccessDeserializer::new(seq))
        }

        fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
            let spaces =
                std::collections::BTreeMap::<SpaceId, Vec<VirtualWorkspaceId>>::deserialize(
                    serde::de::value::MapAccessDeserializer::new(map),
                )?;
            Ok(spaces.into_values().flatten().collect())
        }
    }
    deserializer.deserialize_any(Visitor)
}

impl std::ops::Index<VirtualWorkspaceId> for WorkspaceStore {
    type Output = VirtualWorkspace;

    fn index(&self, id: VirtualWorkspaceId) -> &Self::Output { &self.workspaces[id] }
}

impl std::ops::IndexMut<VirtualWorkspaceId> for WorkspaceStore {
    fn index_mut(&mut self, id: VirtualWorkspaceId) -> &mut Self::Output {
        &mut self.workspaces[id]
    }
}

impl Default for WorkspaceStore {
    fn default() -> Self { Self::new() }
}

/// The workspace store with the window store and the core's workspaces (the catalog order, each
/// workspace's Space, what each Space shows), lent together for reading: the store answers its
/// topology reads from the core. The view derefs to the store, so the store's own data stays
/// reachable.
#[derive(Clone, Copy)]
pub(crate) struct Workspaces<'a> {
    pub(crate) store: &'a WorkspaceStore,
    pub(crate) windows: Windows<'a>,
}

impl std::ops::Deref for Workspaces<'_> {
    type Target = WorkspaceStore;

    fn deref(&self) -> &WorkspaceStore { self.store }
}

impl WorkspaceStore {
    /// The store with `windows` (the window store and the core's workspaces), for reading.
    pub(crate) fn reading<'a>(&'a self, windows: Windows<'a>) -> Workspaces<'a> {
        Workspaces { store: self, windows }
    }
}

impl<'a> Workspaces<'a> {
    /// The core's joined workspaces.
    fn core(&self) -> &'a CoreWorkspaces { self.windows.core }

    /// What `space` shows, if it has an activation.
    pub(crate) fn activation(&self, space: SpaceId) -> Option<SpaceActivation> {
        self.core().activation(space).map(SpaceActivation::from_core)
    }

    /// Every Space with an activation, and what it shows.
    pub(crate) fn activations(&self) -> impl Iterator<Item = (SpaceId, SpaceActivation)> + use<'a> {
        self.core()
            .activations()
            .iter()
            .map(|&(space, activation)| (space, SpaceActivation::from_core(activation)))
    }

    /// Every workspace, in catalog order.
    pub(crate) fn catalog(&self) -> impl Iterator<Item = VirtualWorkspaceId> + use<'a> {
        self.core().catalog().iter().map(|&(id, _)| id)
    }

    /// The Space `id` is on.
    pub(crate) fn workspace_space(&self, id: VirtualWorkspaceId) -> Option<SpaceId> {
        self.core().space_of(id)
    }

    /// The assigned window with `index`, as the core resolves it (the command Space, then other
    /// visible Spaces, then retained workspaces); `None` when none has it or two tie.
    pub(crate) fn resolve_window_index(
        &self,
        command_space: Option<SpaceId>,
        is_visible: impl Fn(SpaceId) -> bool,
        index: u32,
    ) -> Option<WindowId> {
        match self.core().resolve_named(
            |window| window.idx.get() == index,
            command_space,
            is_visible,
        ) {
            rift_core::workspaces::Resolved::Window(window) => Some(window),
            rift_core::workspaces::Resolved::Ambiguous
            | rift_core::workspaces::Resolved::NotFound => None,
        }
    }

    pub fn last_focused_window(
        &self,
        space: SpaceId,
        workspace_id: VirtualWorkspaceId,
    ) -> Option<WindowId> {
        match self.core().is_on(workspace_id, space) {
            true => self.store.workspaces.get(workspace_id)?.last_focused(),
            false => None,
        }
    }

    pub fn workspace_info(
        &self,
        space: SpaceId,
        workspace_id: VirtualWorkspaceId,
    ) -> Option<&'a VirtualWorkspace> {
        let store = self.store;
        store
            .workspaces
            .get(workspace_id)
            .filter(|_| self.core().is_on(workspace_id, space))
    }

    pub(crate) fn workspace_for_window(
        &self,
        space: SpaceId,
        window: WindowId,
    ) -> Option<VirtualWorkspaceId> {
        // A window rift has no record of is in no workspace, whatever the core holds.
        self.windows.record(window)?;
        self.core().assigned_on(window, space)
    }

    pub(crate) fn active_layout(
        &self,
        space: SpaceId,
        workspace: VirtualWorkspaceId,
    ) -> Option<LayoutId> {
        self.workspace_info(space, workspace)?.active_layout()
    }

    /// Every Space with a workspace that has an active layout.
    pub(crate) fn layout_spaces(&self) -> crate::common::collections::BTreeSet<SpaceId> {
        self.core()
            .catalog()
            .iter()
            .filter(|&&(id, _)| self[id].active_layout().is_some())
            .map(|&(_, space)| space)
            .collect()
    }

    pub(crate) fn active_layout_for_space(
        &self,
        space: SpaceId,
    ) -> Option<(VirtualWorkspaceId, LayoutId)> {
        self.store.active_layout_in(self.core(), space)
    }

    /// The workspace at `index` among `space`'s, or the one it shows. A Space not initialised
    /// yet has none: a writer that needs one initialises it first.
    pub(crate) fn workspace_id_at(
        &self,
        space: SpaceId,
        index: Option<usize>,
    ) -> Option<VirtualWorkspaceId> {
        if let Some(index) = index {
            self.workspaces_on(space).nth(index)
        } else {
            self.active_workspace(space)
        }
    }

    /// The workspaces in `space`'s namespace, in catalog order.
    fn scope_ids(&self, space: SpaceId) -> impl Iterator<Item = VirtualWorkspaceId> + use<'a> {
        self.store.scope_in(self.core().catalog(), space)
    }

    /// Navigation history, validated like any target. Unreachable history is kept.
    pub(crate) fn previous_target(&self, context: WorkspaceContext<'_>) -> Option<WorkspaceTarget> {
        let last = self.last_workspace(context.space);
        let on = Asked::new();
        let previous =
            rift_core::workspaces::previous(self.core_mode(), core_context(context), last, |id| {
                on.ask(|| self.workspace_space(id))
            });
        #[cfg(feature = "roc-core")]
        {
            use rift_core::workspaces::plain::PreviousQuestion;
            let question = PreviousQuestion {
                mode: self.core_mode(),
                space: context.space.word(),
                visible: context.visible.iter().map(|&space| space.word()).collect(),
                last: last.map(Word::word),
                on: on.fact(|space| space.map(Word::word)),
            };
            let roc = rift_core_roc::workspaces::previous(question.clone());
            let rust = previous.map(|(id, space)| (id.word(), space.word()));
            crate::roc_path::check("previous", PreviousQuestion::FACTS, &question, &rust, roc);
        }
        previous.map(|(id, space)| WorkspaceTarget { id, space })
    }

    pub(crate) fn resolve_workspace<'c>(
        &self,
        context: impl Into<WorkspaceContext<'c>>,
        selector: &WorkspaceSelector,
    ) -> Option<WorkspaceTarget> {
        let context = context.into();
        let namespace = self.namespace(self.settings.mode, context.space);
        let scope: Vec<_> = self
            .core()
            .catalog()
            .iter()
            .filter(|&&(_, on)| self.namespace(self.settings.mode, on) == namespace)
            .map(|&(id, on)| (id, on, self.store[id].name.as_str()))
            .collect();
        let selector = match selector {
            WorkspaceSelector::Index(index) => rift_core::workspaces::Selector::Index(*index),
            WorkspaceSelector::Name(name) => rift_core::workspaces::Selector::Name(name.as_str()),
        };
        let resolved = rift_core::workspaces::resolve(
            self.core_mode(),
            &scope,
            core_context(context),
            &selector,
        );
        #[cfg(feature = "roc-core")]
        {
            use rift_core::workspaces::Selector;
            use rift_core::workspaces::plain::ResolveQuestion;
            // A name is the place it first appears in this question: the answer names no
            // name, so a name need only be told apart from the others.
            fn word(names: &mut Vec<String>, name: &str) -> u64 {
                match names.iter().position(|named| named == name) {
                    Some(place) => place as u64,
                    None => {
                        names.push(name.to_owned());
                        (names.len() - 1) as u64
                    }
                }
            }
            let mut names = Vec::new();
            let question = ResolveQuestion {
                mode: self.core_mode(),
                scope: scope
                    .iter()
                    .map(|&(id, space, name)| (id.word(), space.word(), word(&mut names, name)))
                    .collect(),
                space: context.space.word(),
                visible: context.visible.iter().map(|&space| space.word()).collect(),
                selector: match selector {
                    Selector::Index(index) => Selector::Index(index),
                    Selector::Name(name) => Selector::Name(word(&mut names, name)),
                },
            };
            let roc = Ok(rift_core_roc::workspaces::resolve(question.clone()));
            let rust = resolved.map(|(id, space)| (id.word(), space.word()));
            crate::roc_path::check("resolve", ResolveQuestion::FACTS, &question, &rust, roc);
        }
        resolved.map(|(id, space)| WorkspaceTarget { id, space })
    }

    fn eligible(&self, context: WorkspaceContext<'_>, id: VirtualWorkspaceId) -> bool {
        let Some(on) = self.workspace_space(id) else {
            return false;
        };
        let eligible = rift_core::workspaces::eligible(self.core_mode(), core_context(context), on);
        #[cfg(feature = "roc-core")]
        {
            use rift_core::workspaces::plain::EligibleQuestion;
            let question = EligibleQuestion {
                mode: self.core_mode(),
                space: context.space.word(),
                visible: context.visible.iter().map(|&space| space.word()).collect(),
                on: on.word(),
            };
            let roc = Ok(rift_core_roc::workspaces::eligible(question.clone()));
            crate::roc_path::check("eligible", EligibleQuestion::FACTS, &question, &eligible, roc);
        }
        eligible
    }

    pub(crate) fn workspace_index(&self, space: SpaceId, id: VirtualWorkspaceId) -> Option<usize> {
        self.scope_ids(space).position(|candidate| candidate == id)
    }

    pub(crate) fn workspace_view(
        self,
        context: WorkspaceContext<'a>,
    ) -> impl Iterator<Item = (usize, VirtualWorkspaceId)> + 'a {
        self.scope_ids(context.space)
            .enumerate()
            .filter(move |(_, id)| self.eligible(context, *id))
    }

    /// Balance visible workspaces around explicit placements and display rules.
    pub(crate) fn plan_display_balance(
        &self,
        displays: &[SpaceId],
        restored: &HashSet<VirtualWorkspaceId>,
    ) -> Vec<(VirtualWorkspaceId, SpaceId)> {
        if displays.len() < 2 {
            return Vec::new();
        }
        let mut visible = Vec::new();
        let pins: Vec<_> = self
            .core()
            .catalog()
            .iter()
            .enumerate()
            .filter_map(|(index, &(id, on))| {
                let workspace = &self[id];
                let owner = displays.iter().position(|&space| space == on)?;
                visible.push((id, owner));
                let fixed = restored.contains(&id)
                    || workspace.home.placed_by() == Some(PlacedBy::User)
                    || workspace.rule_standing == RuleStanding::Overridden
                    || workspace.home.space().is_some_and(|home| home != on)
                    || self
                        .matching_rules(index, &workspace.name)
                        .any(|rule| rule.display.is_some());
                Some(fixed.then_some(owner))
            })
            .collect();
        // Start from current ownership so an improved load does not reshuffle
        // workspaces just to recreate the initial contiguous distribution.
        let owners = visible.iter().map(|&(_, owner)| owner).collect();
        let allocation = balance_owners(displays.len(), &pins, owners);
        visible
            .into_iter()
            .zip(allocation)
            .filter_map(|((id, source), target)| {
                (source != target).then_some((id, displays[target]))
            })
            .collect()
    }

    /// Validate the projected namespace before mutating configuration or layout trees.
    pub(crate) fn plan_settings(
        &self,
        config: &VirtualWorkspaceSettings,
    ) -> Result<WorkspaceSettingsPlan, String> {
        self.plan_settings_names(config, false)
    }

    pub(crate) fn plan_settings_names(
        &self,
        config: &VirtualWorkspaceSettings,
        restoring: bool,
    ) -> Result<WorkspaceSettingsPlan, String> {
        self.validate_persisted_topology()?;
        let issues = config.validate();
        if !issues.is_empty() {
            return Err(issues.join("; "));
        }
        let mode_changed = !restoring && self.settings.mode != config.mode;
        let mut plan = WorkspaceSettingsPlan {
            names: Vec::new(),
            additions: Vec::new(),
        };
        let mut seen = HashSet::default();
        let namespace = |space| self.namespace(config.mode, space);
        let catalog = self.core().catalog();
        let scope_ids = |space| {
            catalog
                .iter()
                .filter(move |&&(_, on)| namespace(on) == namespace(space))
                .map(|&(id, _)| id)
        };
        for space in self.initialized_spaces() {
            // Legacy catalogs may exceed the limit because earlier versions appended
            // workspaces for empty displays. Keep those catalogs valid on reload.
            if scope_ids(space).count() > MAX_WORKSPACES
                && (mode_changed || config.mode != WorkspaceMode::Global)
            {
                return Err(format!("workspace scope exceeds {MAX_WORKSPACES} workspaces"));
            }
        }
        // Configuration owns the initial names on restore. Reserve them before
        // repairing legacy/custom extras so saved names cannot shadow them.
        if restoring {
            for space in self.initialized_spaces() {
                let count = scope_ids(space).count();
                for index in 0..config.default_workspace_count {
                    if index < config.workspace_names.len() || index >= count {
                        seen.insert((namespace(space), config.workspace_name(index)));
                    }
                }
            }
        }
        for &(id, on) in catalog {
            let workspace = &self[id];
            let index = scope_ids(on).position(|candidate| candidate == id).unwrap();
            let configured = restoring && index < config.workspace_names.len();
            let mut name = if mode_changed {
                workspace.name.clone()
            } else {
                config
                    .workspace_names
                    .get(index)
                    .map(|_| config.workspace_name(index))
                    .unwrap_or_else(|| workspace.name.clone())
            };
            if (restoring && !configured) || (mode_changed && config.mode == WorkspaceMode::Global)
            {
                let base = name.clone();
                let mut suffix = 2;
                while seen.contains(&(namespace(on), name.clone())) {
                    name = format!("{base} ({suffix})");
                    suffix += 1;
                }
            }
            if !configured && !seen.insert((namespace(on), name.clone())) {
                return Err(format!(
                    "Duplicate workspace name {name:?}. Workspace names must be unique."
                ));
            }
            if name != workspace.name {
                plan.names.push((id, name));
            }
        }
        let mut planned = HashMap::default();
        for space in self.initialized_spaces() {
            let count = self.workspaces_on(space).count();
            if count == 0 {
                continue;
            } // Empty global contexts remain empty.
            let total = planned.entry(namespace(space)).or_insert_with(|| scope_ids(space).count());
            let missing = config.default_workspace_count.saturating_sub(*total);
            for index in *total..*total + missing {
                let mut name = config.workspace_name(index);
                let scope = namespace(space);
                if !restoring && config.workspace_names.get(index).is_none_or(String::is_empty) {
                    let base = name.clone();
                    let mut suffix = 2;
                    while seen.contains(&(scope, name.clone())) {
                        name = format!("{base} ({suffix})");
                        suffix += 1;
                    }
                }
                if !restoring && !seen.insert((scope, name.clone())) {
                    return Err(format!(
                        "Duplicate workspace name {name:?}. Workspace names must be unique."
                    ));
                }
                plan.additions.push((space, name));
                *total += 1;
            }
        }
        Ok(plan)
    }

    fn name_exists(&self, space: SpaceId, name: &str) -> bool {
        self.store.name_exists_in(self.core().catalog(), space, name)
    }

    /// Global-mode display assignments from workspace rules (last matching rule wins).
    /// Empty unless global mode is active and some rule names a display.
    pub(crate) fn rule_displays(
        self,
    ) -> impl Iterator<Item = (VirtualWorkspaceId, &'a [String])> + use<'a> {
        let order = if self.effective_mode() == WorkspaceMode::Global
            && self.settings.workspace_rules.iter().any(|rule| rule.display.is_some())
        {
            self.core().catalog()
        } else {
            &[]
        };
        let store = self.store;
        order.iter().filter_map(move |&(id, on)| {
            let index = self.workspace_index(on, id)?;
            let display = store
                .matching_rules(index, &store[id].name)
                .find_map(|rule| rule.display.as_deref())?;
            Some((id, display))
        })
    }

    pub fn initialized_spaces(&self) -> Vec<SpaceId> {
        let mut spaces = self.activations().map(|(space, _)| space).collect::<Vec<_>>();
        spaces.sort_unstable();
        spaces
    }

    /// The topology in its saved shape, as the core holds it.
    pub(crate) fn topology(&self) -> SavedTopology {
        SavedTopology {
            catalog: self.core().catalog().to_vec(),
            activations: self.activations().collect(),
        }
    }

    /// Validate the workspace graph before layout code indexes into slotmaps (as a loaded file's
    /// is, [`SavedTopology::validate`]).
    pub(crate) fn validate_persisted_topology(&self) -> Result<(), String> {
        self.topology().validate()
    }

    /// What each native Space shows.
    pub fn active_workspace_per_space(&self) -> HashMap<SpaceId, SpaceActivation> {
        self.activations().collect()
    }

    pub fn last_workspace(&self, space: SpaceId) -> Option<VirtualWorkspaceId> {
        self.activation(space)?.previous()
    }

    pub fn active_workspace(&self, space: SpaceId) -> Option<VirtualWorkspaceId> {
        self.activation(space)?.current()
    }

    pub fn active_workspace_idx(&self, space: SpaceId) -> Option<u64> {
        self.workspace_index(space, self.active_workspace(space)?)
            .map(|index| index as u64)
    }

    /// Per-display mode steps through `space`'s workspaces. Global mode steps through
    /// the whole visible catalog, so the target may live on another display.
    pub(crate) fn step_workspace<'c>(
        &self,
        context: impl Into<WorkspaceContext<'c>>,
        current: VirtualWorkspaceId,
        skip_empty: Option<bool>,
        dir: Direction,
    ) -> Option<VirtualWorkspaceId> {
        use rift_core::workspaces::{SkipEmpty, Step, Wrapping};
        let way = match dir {
            Direction::Right => Step::Next,
            Direction::Left => Step::Previous,
            // Workspaces are a line: up and down go nowhere.
            Direction::Up | Direction::Down => return None,
        };
        let catalog = self.core().catalog();
        let window_store = self.windows;
        let context = context.into();
        let skip_empty = if skip_empty == Some(true) {
            SkipEmpty::Skip
        } else {
            SkipEmpty::Keep
        };
        let wrapping = if self.settings.prevent_wrapping {
            Wrapping::Stop
        } else {
            Wrapping::Wrap
        };
        let has_windows = AskedAbout::new();
        let stepped = rift_core::workspaces::step(
            self.core_mode(),
            &catalog,
            core_context(context),
            current,
            way,
            skip_empty,
            wrapping,
            |id| has_windows.ask(id, |id| window_store.workspace_window_count(id) != 0),
        );
        #[cfg(feature = "roc-core")]
        {
            use rift_core::workspaces::plain::StepQuestion;
            let question = StepQuestion {
                mode: self.core_mode(),
                catalog: catalog.iter().map(|&(id, space)| (id.word(), space.word())).collect(),
                space: context.space.word(),
                visible: context.visible.iter().map(|&space| space.word()).collect(),
                current: current.word(),
                way,
                skip_empty,
                wrapping,
                has_windows: has_windows.answers(Word::word, |has| has),
            };
            let roc = rift_core_roc::workspaces::step(question.clone());
            let rust = stepped.map(Word::word);
            crate::roc_path::check("step", StepQuestion::FACTS, &question, &rust, roc);
        }
        stepped
    }

    /// The workspace on `space` at the ordinal of the window's current workspace,
    /// for native-space identity churn: WindowServer can briefly report another
    /// space without a user moving the window to that space's active workspace.
    pub(crate) fn workspace_at_window_ordinal(
        &self,
        window_store: Windows<'_>,
        space: SpaceId,
        window_id: WindowId,
    ) -> Option<VirtualWorkspaceId> {
        let existing = window_store.workspace_for_window(window_id)?;
        let source = self.workspace_space(existing)?;
        let ordinal = self.workspaces_on(source).position(|id| id == existing)?;
        self.workspaces_on(space).nth(ordinal)
    }

    /// Gets all windows in the active virtual workspace for a given native space.
    pub(crate) fn windows_in_active_workspace(
        &self,
        window_store: Windows<'_>,
        space: SpaceId,
    ) -> Vec<WindowId> {
        if let Some(workspace_id) = self.active_workspace(space) {
            return window_store.workspace_windows(workspace_id);
        }
        Vec::new()
    }

    pub(crate) fn is_window_in_active_workspace(
        &self,
        space: SpaceId,
        window_id: WindowId,
    ) -> bool {
        if let Some(active_workspace_id) = self.active_workspace(space) {
            if let Some(window_workspace_id) = self.workspace_for_window(space, window_id) {
                return window_workspace_id == active_workspace_id;
            }
        }
        true
    }

    pub(crate) fn windows_in_inactive_workspaces(
        &self,
        window_store: Windows<'_>,
        space: SpaceId,
    ) -> Vec<WindowId> {
        let active_workspace_id = self.active_workspace(space);

        self.workspaces
            .iter()
            .filter(|&(id, _)| {
                self.workspace_space(id) == Some(space) && Some(id) != active_workspace_id
            })
            .flat_map(|(id, _)| window_store.workspace_windows(id))
            .collect()
    }

    /// `space`'s workspaces, with their names, if the Space is initialised; a query never
    /// initialises one (the events that bring a Space do, through
    /// [`Self::ensure_space_initialized`]).
    pub fn list_workspaces(&self, space: SpaceId) -> Listing {
        match self.initialized(space) {
            true => Listing::Known(self.existing_workspaces(space)),
            false => Listing::NotYetKnown,
        }
    }

    /// Whether [`Self::ensure_space_initialized`] would leave `space` as it is: it shows a
    /// workspace, or (in global mode, with workspaces elsewhere to show) it shows nothing and
    /// owns none.
    pub(crate) fn initialized(&self, space: SpaceId) -> bool {
        match self.activation(space) {
            Some(SpaceActivation::Showing { .. }) => true,
            // Repair keeps a vacant Space's memory as it is.
            Some(SpaceActivation::Vacant { .. }) => {
                self.workspaces_on(space).next().is_none()
                    && self.effective_mode() == WorkspaceMode::Global
                    && self.scope_ids(space).next().is_some()
            }
            None => false,
        }
    }

    /// Read workspace topology without creating missing state. Validation and restore planning
    /// must use this accessor so a failed transaction cannot initialize part of the live engine.
    pub(crate) fn existing_workspaces(&self, space: SpaceId) -> Vec<(VirtualWorkspaceId, String)> {
        self.workspaces_on(space)
            .filter_map(|id| self.workspaces.get(id).map(|ws| (id, ws.name.clone())))
            .collect()
    }

    /// Workspace IDs are kept in canonical ordinal order, including when loading older files.
    pub(crate) fn workspace_ids(&self, space: SpaceId) -> Vec<VirtualWorkspaceId> {
        self.workspaces_on(space).collect()
    }

    pub(crate) fn workspaces_on(
        &self,
        space: SpaceId,
    ) -> impl Iterator<Item = VirtualWorkspaceId> + use<'a> {
        self.core()
            .catalog()
            .iter()
            .filter(move |&&(_, on)| on == space)
            .map(|&(id, _)| id)
    }

    /// The workspace a window placed on `context`'s Space goes to without a rule. The caller
    /// (placement) initialised the Space.
    fn get_default_workspace<'c>(
        &self,
        context: impl Into<WorkspaceContext<'c>>,
    ) -> Result<VirtualWorkspaceId, WorkspaceError> {
        let context = context.into();
        let space = context.space;
        self.active_workspace(space)
            .or_else(|| {
                (self.effective_mode() == WorkspaceMode::Global)
                    .then(|| self.scope_ids(space).find(|id| self.eligible(context, *id)))
                    .flatten()
            })
            .ok_or(WorkspaceError::NoWorkspacesAvailable)
    }

    pub(crate) fn window_counts(
        &self,
    ) -> impl Iterator<Item = (VirtualWorkspaceId, usize)> + use<'a> {
        let windows = self.windows;
        self.catalog().map(move |id| (id, windows.workspace_window_count(id)))
    }
}

impl WorkspaceStore {
    /// The workspace `space` shows (as `core` holds it) and its active layout. Takes only the
    /// core, for a reader that holds the window records mutably (a drag's preview).
    pub(crate) fn active_layout_in(
        &self,
        core: &CoreWorkspaces,
        space: SpaceId,
    ) -> Option<(VirtualWorkspaceId, LayoutId)> {
        let id = core.activation(space).and_then(|activation| match activation {
            rift_core::workspaces::Activation::Showing { current, .. } => Some(current),
            rift_core::workspaces::Activation::Vacant { .. } => None,
        })?;
        Some((id, self.workspaces.get(id)?.active_layout()?))
    }

    /// `catalog`'s workspaces in `space`'s namespace, in catalog order.
    fn scope_in<'c>(
        &'c self,
        catalog: &'c [(VirtualWorkspaceId, SpaceId)],
        space: SpaceId,
    ) -> impl Iterator<Item = VirtualWorkspaceId> + 'c {
        let namespace = self.namespace(self.settings.mode, space);
        catalog
            .iter()
            .filter(move |&&(_, on)| self.namespace(self.settings.mode, on) == namespace)
            .map(|&(id, _)| id)
    }

    fn name_exists_in(
        &self,
        catalog: &[(VirtualWorkspaceId, SpaceId)],
        space: SpaceId,
        name: &str,
    ) -> bool {
        self.scope_in(catalog, space).any(|id| self[id].name == name)
    }

    /// `base`, or the first `base (n)` no workspace in `space`'s namespace of `catalog` has.
    fn available_name_in(
        &self,
        catalog: &[(VirtualWorkspaceId, SpaceId)],
        space: SpaceId,
        base: String,
    ) -> String {
        if !self.name_exists_in(catalog, space, &base) {
            return base;
        }
        let mut suffix = 2;
        loop {
            let name = format!("{base} ({suffix})");
            if !self.name_exists_in(catalog, space, &name) {
                return name;
            }
            suffix += 1;
        }
    }

    pub(crate) fn effective_mode(&self) -> WorkspaceMode {
        if self.single_display {
            WorkspaceMode::PerDisplay
        } else {
            self.settings.mode
        }
    }

    fn namespace(&self, mode: WorkspaceMode, space: SpaceId) -> Option<SpaceId> {
        (mode != WorkspaceMode::Global || self.single_display).then_some(space)
    }

    /// The workspace mode, as the core decides by it.
    fn core_mode(&self) -> rift_core::lifecycle::WorkspaceMode {
        match self.effective_mode() {
            WorkspaceMode::PerDisplay => rift_core::lifecycle::WorkspaceMode::PerDisplay,
            WorkspaceMode::Global => rift_core::lifecycle::WorkspaceMode::Global,
        }
    }

    /// Ownership changes leave the workspace object, layout and focus history intact.
    pub(crate) fn transfer_workspace(
        &mut self,
        windows: &mut WindowsMut<'_>,
        id: VirtualWorkspaceId,
        target: SpaceId,
        display: Option<&str>,
        reason: WorkspaceTransferReason,
    ) -> bool {
        let (Some(workspace), Some(source)) = (
            self.workspaces.get(id),
            self.reading(windows.read()).workspace_space(id),
        ) else {
            return false;
        };
        if reason == WorkspaceTransferReason::Explicit
            && let Some(home) = workspace.home.space()
            && self
                .reading(windows.read())
                .activation(home)
                .is_some_and(|activation| activation.remembered() == Some(id))
        {
            self.update_activation(windows, home, SpaceActivation::forget_memory);
        }
        if source == target {
            if reason != WorkspaceTransferReason::Explicit {
                return false;
            }
            let workspace = &mut self[id];
            // `by` is part of the home: re-choosing the current display still marks it chosen.
            let home = Home::placed(target, display.map(str::to_owned), PlacedBy::User);
            let changed = workspace.home != home;
            workspace.home = home;
            return changed;
        }
        if reason == WorkspaceTransferReason::DisplayDeparture {
            self.update_activation_or_default(windows, source, |activation| {
                if activation.current() == Some(id) && activation.remembered().is_none() {
                    activation.remember(id);
                }
            });
        }
        let workspace = &mut self[id];
        match reason {
            WorkspaceTransferReason::Explicit => {
                workspace.home = Home::placed(target, display.map(str::to_owned), PlacedBy::User);
            }
            // An arriving display takes the workspace as its home; who chose the old home
            // still stands (a home the user chose stays out of automatic balancing).
            WorkspaceTransferReason::DisplayArrival => {
                let by = workspace.home.placed_by().unwrap_or_default();
                workspace.home = Home::placed(target, display.map(str::to_owned), by);
            }
            WorkspaceTransferReason::DisplayDeparture | WorkspaceTransferReason::Restoration => {}
        }
        self.move_workspace(windows, id, target);
        // Remembered presentation becomes active again only when ownership returns.
        if reason == WorkspaceTransferReason::Explicit {
            self.set_active_workspace(windows, target, id);
        } else if self.reading(windows.read()).active_workspace(target).is_none()
            || self
                .reading(windows.read())
                .activation(target)
                .is_some_and(|activation| activation.remembered() == Some(id))
        {
            self.set_active_workspace(windows, target, id);
            self.update_activation(windows, target, SpaceActivation::forget_memory);
        }
        self.repair_activation(windows, source);
        self.repair_activation(windows, target);
        true
    }

    /// Old snapshots may contain duplicate local names. Keep every identity and
    /// disambiguate only at load; live configuration conflicts are rejected.
    pub(crate) fn normalize_loaded_names(&mut self, topology: &SavedTopology) {
        self.normalize_names(topology.catalog(), false);
    }

    /// Names every workspace of `catalog` (in its order) apart from those before it in its
    /// namespace (`shared`: one namespace).
    fn normalize_names(&mut self, catalog: &[(VirtualWorkspaceId, SpaceId)], shared: bool) {
        for (index, &(id, space)) in catalog.iter().enumerate() {
            let workspace = &self[id];
            let duplicate = catalog[..index]
                .iter()
                .any(|&(other, on)| (shared || on == space) && self[other].name == workspace.name);
            if workspace.name.is_empty() || duplicate {
                let base = if workspace.name.is_empty() {
                    format!("Workspace {}", index + 1)
                } else {
                    workspace.name.clone()
                };
                let name = self.available_name_in(catalog, space, base);
                self.workspaces[id].name = name;
            }
        }
    }

    fn repair_activation(&mut self, windows: &mut WindowsMut<'_>, space: SpaceId) {
        let view = self.reading(windows.read());
        let owned = |id| view.workspace_space(id) == Some(space);
        let activation = view.activation(space).unwrap_or_default();
        let (previous, remembered) = (activation.previous(), activation.remembered());
        let current = activation
            .current()
            .filter(|&id| owned(id))
            .or_else(|| previous.filter(|&id| owned(id)))
            .or_else(|| view.workspaces_on(space).next());
        let previous = previous.filter(|&id| {
            self.workspaces.contains_key(id)
                && Some(id) != current
                && (self.effective_mode() == WorkspaceMode::Global || owned(id))
        });
        let repaired = match current {
            Some(current) => SpaceActivation::Showing { current, previous, remembered },
            // A Space that shows nothing keeps no history: showing a workspace again makes the
            // one shown until then, none, the previous one, so the history would never be read.
            None => SpaceActivation::Vacant { remembered },
        };
        self.set_activation(windows, space, repaired);
    }

    pub(crate) fn ensure_layouts_for_size(
        &mut self,
        windows: &mut WindowsMut<'_>,
        space: SpaceId,
        size: CGSize,
    ) {
        self.ensure_space_initialized(windows, space);
        for id in self.reading(windows.read()).workspace_ids(space) {
            self[id].ensure_layout_for_size(size);
        }
    }

    pub(crate) fn validate_layouts(&self) -> Result<(), String> {
        for (id, workspace) in self.workspaces.iter() {
            workspace
                .layout_state
                .validate(&workspace.layout_system)
                .map_err(|error| format!("workspace {id:?} {error}"))?;
        }
        Ok(())
    }

    pub fn new() -> Self {
        Self::new_with_config(&VirtualWorkspaceSettings::default(), &LayoutSettings::default())
    }

    pub fn new_with_config(
        config: &VirtualWorkspaceSettings,
        layout_settings: &LayoutSettings,
    ) -> Self {
        warn_on_ignored_display_rules(config);
        Self {
            workspaces: Catalog::default(),
            single_display: false,
            settings: config.clone(),
            layout_settings: layout_settings.clone(),
        }
    }

    pub(crate) fn apply_settings(
        &mut self,
        windows: &mut WindowsMut<'_>,
        config: &VirtualWorkspaceSettings,
        layout_settings: &LayoutSettings,
        plan: WorkspaceSettingsPlan,
    ) {
        warn_on_ignored_display_rules(config);
        self.settings = config.clone();
        self.layout_settings = layout_settings.clone();
        for (id, name) in plan.names {
            self[id].name = name;
        }
        for (space, name) in plan.additions {
            let index = self.reading(windows.read()).scope_ids(space).count();
            let mode = self.resolve_layout_mode_for_workspace(index, &name);
            self.insert_workspace(windows, space, name, mode);
        }
        for space in self.reading(windows.read()).initialized_spaces() {
            self.repair_activation(windows, space);
        }
    }

    /// Adds a workspace to an already-initialized space. If the space has been
    /// shown, the new workspace gets a layout for the space's current size so
    /// it can accept windows immediately instead of waiting for the next
    /// `SpaceExposed`.
    fn insert_workspace(
        &mut self,
        windows: &mut WindowsMut<'_>,
        space: SpaceId,
        name: String,
        mode: LayoutMode,
    ) -> VirtualWorkspaceId {
        let size = self
            .reading(windows.read())
            .workspaces_on(space)
            .find_map(|id| self.workspaces[id].layout_state.active_size());
        let mut workspace = VirtualWorkspace::new(name, mode, &self.layout_settings);
        // Beside a workspace homed on this Space with a display preference, it shares it.
        let display = self.workspaces.values().find_map(|workspace| {
            workspace
                .home
                .on_display()
                .filter(|(home, _)| *home == space)
                .map(|(_, display)| display.to_owned())
        });
        workspace.home = Home::placed(space, display, PlacedBy::Automatic);
        if let Some(size) = size {
            workspace.ensure_layout_for_size(size);
        }
        self.catalog_insert(windows, space, workspace)
    }

    fn append_default_workspace(
        &mut self,
        windows: &mut WindowsMut<'_>,
        space: SpaceId,
    ) -> VirtualWorkspaceId {
        let index = self.reading(windows.read()).scope_ids(space).count();
        let name = self.available_name_in(
            windows.core.catalog(),
            space,
            self.settings.workspace_name(index),
        );
        let mode = self.resolve_layout_mode_for_workspace(index, &name);
        self.insert_workspace(windows, space, name, mode)
    }

    /// Update scope before display placement rules run, without creating workspaces.
    /// `catalog` is the core's (every workspace with its Space, in catalog order).
    pub(crate) fn update_display_count(
        &mut self,
        catalog: &[(VirtualWorkspaceId, SpaceId)],
        display_count: usize,
    ) {
        let was_single = self.single_display;
        self.single_display = display_count == 1;
        if was_single && self.effective_mode() == WorkspaceMode::Global {
            // Independent native desktops may have used the same names.
            self.normalize_names(catalog, true);
        }
    }

    /// Allocate the configured global catalog in contiguous blocks across visible managed
    /// desktops. Existing catalogs retain their placement as desktops are exposed.
    /// `rule_space` resolves a workspace rule's display to one of `spaces`.
    pub(crate) fn ensure_visible_spaces(
        &mut self,
        windows: &mut WindowsMut<'_>,
        spaces: &[SpaceId],
        display_count: usize,
        rule_space: impl Fn(&[String]) -> Option<SpaceId>,
    ) {
        if spaces.is_empty() {
            return;
        }
        self.update_display_count(windows.core.catalog(), display_count);
        if self.effective_mode() == WorkspaceMode::Global
            && self.reading(windows.read()).scope_ids(spaces[0]).next().is_none()
        {
            let count = self.settings.default_workspace_count.clamp(1, MAX_WORKSPACES);
            let pins: Vec<_> = (0..count)
                .map(|index| {
                    let name = self.settings.workspace_name(index);
                    let space = self
                        .matching_rules(index, &name)
                        .find_map(|rule| rule.display.as_deref())
                        .and_then(&rule_space)?;
                    spaces.iter().position(|candidate| *candidate == space)
                })
                .collect();
            for (index, owner) in distribute(spaces.len(), pins).into_iter().enumerate() {
                let space = spaces[owner];
                let id = self.append_default_workspace(windows, space);
                if index == self.settings.default_workspace
                    || self.reading(windows.read()).activation(space).is_none()
                {
                    self.set_activation(windows, space, SpaceActivation::showing(id));
                }
            }
            // A display left without a workspace (more displays than workspaces) shows nothing:
            // it is initialised here, at its exposure, not by whichever query comes first.
            for &space in spaces {
                self.ensure_space_initialized(windows, space);
            }
        } else {
            for &space in spaces {
                self.ensure_space_initialized(windows, space);
            }
        }
    }

    pub(crate) fn ensure_space_initialized(
        &mut self,
        windows: &mut WindowsMut<'_>,
        space: SpaceId,
    ) {
        if self.reading(windows.read()).active_workspace(space).is_some() {
            return;
        }
        let first = self.reading(windows.read()).workspaces_on(space).next();
        if let Some(id) = first {
            self.set_active_workspace(windows, space, id);
            return;
        }
        if self.effective_mode() == WorkspaceMode::Global
            && self.reading(windows.read()).scope_ids(space).next().is_some()
        {
            self.repair_activation(windows, space);
            return;
        }
        let count = self.settings.default_workspace_count.clamp(1, MAX_WORKSPACES);
        let selected = self.settings.default_workspace.min(count - 1);
        for index in 0..count {
            let id = self.append_default_workspace(windows, space);
            if index == selected {
                self.set_activation(windows, space, SpaceActivation::showing(id));
            }
        }
    }

    /// Resolve each property independently, with the last matching rule winning.
    pub(crate) fn matching_rules<'a>(
        &'a self,
        index: usize,
        name: &'a str,
    ) -> impl Iterator<Item = &'a WorkspaceLayoutRule> {
        self.settings
            .workspace_rules
            .iter()
            .rev()
            .filter(move |rule| selector_matches(&rule.workspace, index, name))
    }

    pub(crate) fn resolve_layout_mode_for_workspace(&self, index: usize, name: &str) -> LayoutMode {
        self.matching_rules(index, name)
            .find_map(|rule| rule.layout)
            .unwrap_or(self.layout_settings.mode)
    }

    /// Moves `old_space`'s workspaces onto `new_space` and returns the workspaces that were
    /// auto-created there and deleted to make room, so callers can drop state keyed by them.
    pub(crate) fn remap_space(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        old_space: SpaceId,
        new_space: SpaceId,
    ) -> Vec<VirtualWorkspaceId> {
        for workspace in self.workspaces.values_mut() {
            if workspace.home.space() == Some(old_space) {
                workspace.home = workspace.home.with_space(new_space);
            }
        }
        if old_space == new_space
            || self.reading(window_store.read()).activation(old_space).is_none()
        {
            return Vec::new();
        }

        // The window server puts windows on a desktop before rift is told it replaced
        // another, so some are already assigned to the workspaces about to be deleted. Dropping
        // those assignments leaves the window on no workspace, which also loses which
        // workspace of the desktop it was on. Each deleted workspace hands its windows and
        // claims to the migrating workspace in the same position (the last one past the end),
        // decided before anything is removed.
        let heirs = self.reading(window_store.read()).workspace_ids(old_space);
        let successor = |ordinal: usize| {
            heirs
                .get(ordinal)
                .or_else(|| heirs.last())
                .map_or(Successor::Unassign, |&heir| Successor::Into(heir))
        };
        let deleted_target_workspace_ids =
            if self.effective_mode() == WorkspaceMode::PerDisplay && !heirs.is_empty() {
                self.remove_space(window_store, new_space, successor)
            } else {
                Vec::new()
            };
        for &id in &heirs {
            self.move_workspace(window_store, id, new_space);
        }
        if let Some(activation) = self.clear_activation(window_store, old_space) {
            self.set_activation(window_store, new_space, activation);
        }
        window_store.remap_space(old_space, new_space);

        deleted_target_workspace_ids
    }

    /// Delete a native Space that no longer exists, returning the workspaces it took with it;
    /// the `ordinal`th of them hands its windows and claims to `successor(ordinal)`. Nothing
    /// shows or remembers a workspace as it is removed: the Space's activation is cleared, and
    /// every other Space's history and memory of them dropped, first.
    pub(crate) fn remove_space(
        &mut self,
        windows: &mut WindowsMut<'_>,
        space: SpaceId,
        successor: impl Fn(usize) -> Successor<VirtualWorkspaceId>,
    ) -> Vec<VirtualWorkspaceId> {
        let removed = self.reading(windows.read()).workspace_ids(space);
        self.clear_activation(windows, space);
        self.update_all_activations(windows, |activation| {
            activation.retain(|id| !removed.contains(&id))
        });
        for (ordinal, &id) in removed.iter().enumerate() {
            self.catalog_remove(windows, id, successor(ordinal));
        }
        removed
    }

    pub(crate) fn create_workspace(
        &mut self,
        windows: &mut WindowsMut<'_>,
        space: SpaceId,
        name: Option<String>,
    ) -> Result<VirtualWorkspaceId, WorkspaceError> {
        if self.effective_mode() == WorkspaceMode::PerDisplay {
            self.ensure_space_initialized(windows, space);
        }
        let count = self.reading(windows.read()).scope_ids(space).count();
        if count >= MAX_WORKSPACES {
            return Err(WorkspaceError::InconsistentState(format!(
                "Maximum workspace limit ({}) reached for space {:?}",
                MAX_WORKSPACES, space
            )));
        }

        let name = if let Some(name) = name.filter(|name| !name.is_empty()) {
            if self.reading(windows.read()).name_exists(space, &name) {
                return Err(WorkspaceError::InconsistentState(format!(
                    "Workspace name {name:?} already exists. Workspace names must be unique."
                )));
            }
            name
        } else {
            (1..=MAX_WORKSPACES + 1)
                .map(|index| format!("Workspace {index}"))
                .find(|name| !self.reading(windows.read()).name_exists(space, name))
                .expect("a bounded catalog always has an available generated name")
        };

        let mode = self.resolve_layout_mode_for_workspace(count, &name);

        let id = self.insert_workspace(windows, space, name, mode);
        self.repair_activation(windows, space);
        Ok(id)
    }

    /// `space` remembers `id`, to take it back when its ownership returns.
    pub(crate) fn remember_on(
        &mut self,
        windows: &mut WindowsMut<'_>,
        space: SpaceId,
        id: VirtualWorkspaceId,
    ) {
        self.update_activation_or_default(windows, space, |activation| activation.remember(id));
    }

    // The topology's only writers: each tells the core, which the owner holds beside the window
    // store and lends them (`WindowsMut::topology`). The core holds what each Space shows.

    fn set_activation(
        &mut self,
        windows: &mut WindowsMut<'_>,
        space: SpaceId,
        activation: SpaceActivation,
    ) {
        windows.topology().activation(space, Some(activation.to_core()));
    }

    /// Changes what `space` shows, if it has an activation.
    pub(crate) fn update_activation(
        &mut self,
        windows: &mut WindowsMut<'_>,
        space: SpaceId,
        change: impl FnOnce(&mut SpaceActivation),
    ) {
        if let Some(mut activation) = self.reading(windows.read()).activation(space) {
            change(&mut activation);
            self.set_activation(windows, space, activation);
        }
    }

    fn update_activation_or_default(
        &mut self,
        windows: &mut WindowsMut<'_>,
        space: SpaceId,
        change: impl FnOnce(&mut SpaceActivation),
    ) {
        let mut activation = self.reading(windows.read()).activation(space).unwrap_or_default();
        change(&mut activation);
        self.set_activation(windows, space, activation);
    }

    fn clear_activation(
        &mut self,
        windows: &mut WindowsMut<'_>,
        space: SpaceId,
    ) -> Option<SpaceActivation> {
        let activation = self.reading(windows.read()).activation(space);
        windows.topology().activation(space, None);
        activation
    }

    fn update_all_activations(
        &mut self,
        windows: &mut WindowsMut<'_>,
        change: impl Fn(&mut SpaceActivation),
    ) {
        let spaces: Vec<_> =
            self.reading(windows.read()).activations().map(|(space, _)| space).collect();
        for space in spaces {
            self.update_activation(windows, space, &change);
        }
    }

    /// Adds `workspace` on `space`: the catalog holds the workspace, the core its Space.
    fn catalog_insert(
        &mut self,
        windows: &mut WindowsMut<'_>,
        space: SpaceId,
        workspace: VirtualWorkspace,
    ) -> VirtualWorkspaceId {
        let id = self.workspaces.insert(workspace);
        windows.topology().created(id, space);
        id
    }

    /// Takes a workspace out of the catalog; the core's shadow hands its windows and claims to
    /// `successor` in the same step, and the window store's records follow.
    fn catalog_remove(
        &mut self,
        windows: &mut WindowsMut<'_>,
        id: VirtualWorkspaceId,
        successor: rift_core::workspaces::Successor<VirtualWorkspaceId>,
    ) -> Option<VirtualWorkspace> {
        let workspace = self.workspaces.remove(id)?;
        windows.remove_workspace(id, successor);
        Some(workspace)
    }

    /// Moves a workspace to another Space.
    fn move_workspace(
        &mut self,
        windows: &mut WindowsMut<'_>,
        id: VirtualWorkspaceId,
        space: SpaceId,
    ) {
        windows.topology().moved(id, space);
    }

    /// Select a workspace and remember where navigation started, even when the
    /// destination is already visible on another display.
    pub(crate) fn set_active_workspace_from(
        &mut self,
        windows: &mut WindowsMut<'_>,
        source: SpaceId,
        workspace_id: VirtualWorkspaceId,
    ) -> bool {
        let Some(target) = self.reading(windows.read()).workspace_space(workspace_id) else {
            return false;
        };
        let previous = self.reading(windows.read()).active_workspace(source);
        if !self.set_active_workspace(windows, target, workspace_id) {
            return false;
        }
        if self.effective_mode() == WorkspaceMode::Global && source != target {
            self.update_activation(windows, target, |activation| {
                if let SpaceActivation::Showing { previous: history, .. } = activation {
                    *history = previous;
                }
            });
        }
        true
    }

    pub(crate) fn set_active_workspace(
        &mut self,
        windows: &mut WindowsMut<'_>,
        space: SpaceId,
        workspace_id: VirtualWorkspaceId,
    ) -> bool {
        trace_misc("set_active_workspace", || {
            if self.reading(windows.read()).workspace_space(workspace_id) != Some(space) {
                error!(
                    ?workspace_id,
                    ?space,
                    "Attempted to activate a missing or foreign workspace"
                );
                return false;
            }
            self.update_activation_or_default(windows, space, |activation| {
                activation.activate(workspace_id)
            });
            true
        })
    }

    pub(crate) fn assign_window_to_workspace(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        space: SpaceId,
        window_id: WindowId,
        workspace_id: VirtualWorkspaceId,
    ) -> bool {
        trace_misc("assign_window_to_workspace", || {
            if !self.workspaces.contains_key(workspace_id)
                || self.reading(window_store.read()).workspace_space(workspace_id) != Some(space)
            {
                error!(
                    "Attempted to assign window to non-existent/foreign workspace {:?} for space {:?}",
                    workspace_id, space
                );
                return false;
            }

            let previous_assignment = window_store.read().workspace_for_window(window_id);
            window_store.assign_window_to_workspace(window_id, workspace_id);
            if let Some(previous_assignment) = previous_assignment
                && previous_assignment != workspace_id
                && let Some(previous_workspace) = self.workspaces.get_mut(previous_assignment)
            {
                // Membership is single-valued: the window leaves the old workspace's tree here,
                // where its assignment changes, not later from wherever removal looks.
                previous_workspace.layout_system.update(
                    crate::layout_engine::systems::WindowChange::RemoveAndRebalance(window_id),
                );
                if previous_workspace.last_focused() == Some(window_id) {
                    previous_workspace.set_last_focused(None);
                }
            }
            true
        })
    }

    pub(crate) fn remove_window(&mut self, window_store: &mut WindowsMut<'_>, window_id: WindowId) {
        let _ = window_store.remove_window_assignment(window_id);
        window_store.clear_rule_floating(window_id);
    }

    pub(crate) fn set_last_focused_window(
        &mut self,
        core: &CoreWorkspaces,
        space: SpaceId,
        workspace_id: VirtualWorkspaceId,
        window_id: Option<WindowId>,
    ) {
        if core.is_on(workspace_id, space) {
            if let Some(workspace) = self.workspaces.get_mut(workspace_id) {
                workspace.set_last_focused(window_id);
            }
        }
    }

    pub fn transfer_window_identity(&mut self, from: WindowId, to: WindowId) {
        if from == to {
            return;
        }
        for workspace in self.workspaces.values_mut() {
            let contained_from = workspace.last_focused() == Some(from);
            if workspace.last_focused() == Some(to) {
                workspace.set_last_focused(None);
            }
            if contained_from {
                workspace.set_last_focused(Some(to));
            }
        }
    }

    /// Tests: `space`'s workspaces once the events that bring a Space have initialised it (as
    /// its exposure or a window placed on it does).
    #[cfg(test)]
    pub(crate) fn listed_after_init(
        &mut self,
        windows: &mut WindowsMut<'_>,
        space: SpaceId,
    ) -> Vec<(VirtualWorkspaceId, String)> {
        self.ensure_space_initialized(windows, space);
        match self.reading(windows.read()).list_workspaces(space) {
            Listing::Known(workspaces) => workspaces,
            Listing::NotYetKnown => panic!("{space:?} is initialised, so it is known"),
        }
    }

    /// Tests only: nothing in rift renames a workspace by id (names come from settings).
    #[cfg(test)]
    pub(crate) fn rename_workspace(
        &mut self,
        core: &CoreWorkspaces,
        space: SpaceId,
        workspace_id: VirtualWorkspaceId,
        new_name: String,
    ) -> bool {
        if !core.is_on(workspace_id, space) {
            return false;
        }
        if new_name.is_empty()
            || self
                .scope_in(core.catalog(), space)
                .any(|id| id != workspace_id && self[id].name == new_name)
        {
            return false;
        }
        self[workspace_id].name = new_name;
        true
    }

    pub(crate) fn apply_app_rule_decision<'a>(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        window_id: WindowId,
        context: impl Into<WorkspaceContext<'a>>,
        rule_decision: Option<AppRuleDecision>,
        preserve_existing: bool,
    ) -> Result<AppRuleResult, WorkspaceError> {
        let context = context.into();
        let space = context.space;
        self.ensure_space_initialized(window_store, space);

        let existing_assignment = if self.effective_mode() == WorkspaceMode::Global {
            window_store
                .read()
                .workspace_for_window(window_id)
                .filter(|id| self.workspaces.contains_key(*id))
        } else {
            self.reading(window_store.read()).workspace_for_window(space, window_id)
        };

        let rule_override = rule_decision.as_ref().and_then(AppRuleDecision::management_override);
        let admitted = window_store
            .admitted_with_override(window_id, rule_override)
            // Assignment tests and restore paths may not yet have an AX
            // snapshot. In that case only an explicit rejection can deny it.
            .unwrap_or(rule_override != Some(false));
        window_store.set_manage_override(window_id, rule_override);
        if !admitted {
            if !window_store.is_temporarily_unavailable(window_id) || rule_override == Some(false) {
                window_store.clear_rule_floating(window_id);
            }
            return Ok(AppRuleResult::Rejected(if rule_override == Some(false) {
                AppRuleRejection::ExplicitRule
            } else {
                AppRuleRejection::Heuristic
            }));
        }

        let (workspace, floating, position, size, respect_min_size, focus) =
            rule_decision.map_or((None, false, None, None, false, false), |decision| {
                (
                    decision.workspace,
                    decision.floating,
                    decision.position,
                    decision.size,
                    decision.respect_min_size,
                    decision.focus,
                )
            });
        let selected = workspace
            .as_ref()
            .and_then(|selector| {
                self.reading(window_store.read()).resolve_workspace(context, selector)
            })
            .map(|target| target.id);
        if workspace.is_some() && selected.is_none() {
            warn!(
                ?space,
                ?workspace,
                "App rule workspace was not found; preserving assignment"
            );
        }
        let workspace_id = if preserve_existing {
            existing_assignment.or(selected)
        } else {
            selected.or(existing_assignment)
        }
        .map(Ok)
        .unwrap_or_else(|| self.reading(window_store.read()).get_default_workspace(context))?;
        let prior_assignment = window_store.read().workspace_for_window(window_id);
        let workspace_changed = prior_assignment != Some(workspace_id);
        let Some(on) = self.reading(window_store.read()).workspace_space(workspace_id) else {
            error!("Failed to apply window workspace assignment");
            return Err(WorkspaceError::AssignmentFailed);
        };
        if !self.assign_window_to_workspace(window_store, on, window_id, workspace_id) {
            error!("Failed to apply window workspace assignment");
            return Err(WorkspaceError::AssignmentFailed);
        }
        let was_rule_floating = window_store.replace_rule_floating(window_id, floating);
        // Focus is a placement side effect, not persistent state. Rediscovery may
        // reapply the same rule repeatedly, so only emit it for a real transition.
        let focus = focus && (workspace_changed || was_rule_floating != floating);
        Ok(AppRuleResult::Managed(AppRuleEffects {
            source_workspace: prior_assignment,
            workspace_id,
            floating,
            position,
            size,
            respect_min_size,
            focus,
            was_rule_floating,
        }))
    }

    #[cfg(test)]
    fn assign_window_with_app_info(
        &mut self,
        window_store: &mut WindowsMut<'_>,
        window_id: WindowId,
        space: SpaceId,
        app_bundle_id: Option<&str>,
        app_name: Option<&str>,
        window_title: Option<&str>,
        ax_role: Option<&str>,
        ax_subrole: Option<&str>,
    ) -> Result<AppRuleResult, WorkspaceError> {
        let decision = crate::model::AppRuleEngine::new(&self.settings.app_rules).evaluate(
            crate::model::WindowRuleContext {
                app_bundle_id,
                app_name,
                window_title,
                ax_role,
                ax_subrole,
                window_tags: None,
            },
        );
        self.apply_app_rule_decision(window_store, window_id, space, decision, false)
    }
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use slotmap::SlotMap;

    use super::*;

    /// `saved`, a saved store, loaded and checked as a file's is.
    fn load_store(saved: &str) -> LoadedWorkspaces {
        ron::from_str::<LoadedWorkspaces>(saved).unwrap().checked().unwrap()
    }

    /// `store` as it is saved, its topology written from `windows`' core.
    fn save_store(store: &WorkspaceStore, windows: &OwnedWindows) -> String {
        ron::ser::to_string(&store.saved(windows.view().core_workspaces())).unwrap()
    }
    use crate::actor::app::WindowId;
    use crate::model::window_store::OwnedWindows;
    use crate::model::{HiddenWindowPlacement, HideCorner};
    use crate::sys::screen::SpaceId;

    fn expect_managed(result: Result<AppRuleResult, WorkspaceError>) -> AppRuleEffects {
        match result {
            Ok(AppRuleResult::Managed(decision)) => decision,
            Ok(AppRuleResult::Rejected(reason)) => {
                panic!("Window was unexpectedly rejected: {reason:?}")
            }
            Err(e) => panic!("assign_window_with_app_info failed: {:?}", e),
        }
    }

    fn assign(
        manager: &mut WorkspaceStore,
        window_store: &mut WindowsMut<'_>,
        window_id: WindowId,
        space: SpaceId,
        app_id: Option<&str>,
        app_name: Option<&str>,
        window_title: Option<&str>,
        ax_role: Option<&str>,
        ax_subrole: Option<&str>,
    ) -> AppRuleEffects {
        expect_managed(manager.assign_window_with_app_info(
            window_store,
            window_id,
            space,
            app_id,
            app_name,
            window_title,
            ax_role,
            ax_subrole,
        ))
    }

    #[test]
    fn global_single_display_isolates_native_spaces_and_rejoins_displays() {
        let (a, b, other) = (SpaceId::new(801), SpaceId::new(802), SpaceId::new(803));
        let settings = VirtualWorkspaceSettings {
            mode: WorkspaceMode::Global,
            default_workspace_count: 2,
            workspace_names: vec!["Code".into(), "Web".into()],
            ..Default::default()
        };
        let mut store = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());
        let mut windows = OwnedWindows::default();
        store.ensure_visible_spaces(&mut windows.view_mut(), &[a], 1, |_| None);
        let original = store.reading(windows.view()).workspace_ids(a);
        store.set_active_workspace(&mut windows.view_mut(), a, original[1]);
        store.ensure_visible_spaces(&mut windows.view_mut(), &[b], 1, |_| None);
        let independent = store.reading(windows.view()).workspace_ids(b);
        assert_eq!(independent.len(), 2);
        let context = WorkspaceContext { space: b, visible: &[b] };
        let window = WindowId::new(80, 1);
        let assigned = expect_managed(store.apply_app_rule_decision(
            &mut windows.view_mut(),
            window,
            context,
            None,
            false,
        ))
        .workspace_id;
        assert_eq!(store.reading(windows.view()).workspace_space(assigned), Some(b));
        for (index, &id) in independent.iter().enumerate() {
            assert_eq!(store[id].name, settings.workspace_names[index]);
            assert_eq!(
                store
                    .reading(windows.view())
                    .resolve_workspace(context, &WorkspaceSelector::Index(index))
                    .unwrap()
                    .id,
                id
            );
            assert_eq!(
                store
                    .reading(windows.view())
                    .resolve_workspace(
                        context,
                        &WorkspaceSelector::Name(settings.workspace_names[index].clone())
                    )
                    .unwrap()
                    .id,
                id
            );
        }
        let plan = store.reading(windows.view()).plan_settings(&settings).unwrap();
        store.apply_settings(
            &mut windows.view_mut(),
            &settings,
            &LayoutSettings::default(),
            plan,
        );
        // Loading before the first screen snapshot must preserve independent names.
        let saved = serde_json::to_string(&store.saved(windows.view().core_workspaces())).unwrap();
        let mut loaded = serde_json::from_str::<LoadedWorkspaces>(&saved)
            .unwrap()
            .checked()
            .unwrap()
            .store;
        let plan = loaded.reading(windows.view()).plan_settings_names(&settings, true).unwrap();
        loaded.apply_settings(
            &mut windows.view_mut(),
            &settings,
            &LayoutSettings::default(),
            plan,
        );
        assert_eq!(loaded[independent[0]].name, "Code");
        assert_eq!(loaded[original[0]].name, "Code");
        store.ensure_visible_spaces(&mut windows.view_mut(), &[a], 1, |_| None);
        assert_eq!(store.reading(windows.view()).workspace_ids(a), original);
        assert_eq!(
            store.reading(windows.view()).active_workspace(a),
            Some(original[1])
        );
        assert_eq!(windows.view().workspace_for_window(window), Some(assigned));
        // Connecting a display restores shared navigation without replacing
        // either native desktop's workspace identities.
        store.ensure_visible_spaces(&mut windows.view_mut(), &[a, b], 2, |_| None);
        let shared = WorkspaceContext { space: a, visible: &[a, b] };
        assert_eq!(store.reading(windows.view()).workspace_view(shared).count(), 4);
        assert_eq!(
            store
                .reading(windows.view())
                .resolve_workspace(shared, &WorkspaceSelector::Index(2))
                .unwrap()
                .id,
            independent[0]
        );
        store.reading(windows.view()).plan_settings(&settings).unwrap();
        store.ensure_visible_spaces(&mut windows.view_mut(), &[a, other], 2, |_| None);
        assert_eq!(
            store
                .reading(windows.view())
                .workspace_view(WorkspaceContext { space: a, visible: &[a, other] })
                .count(),
            2
        );
        store.ensure_visible_spaces(&mut windows.view_mut(), &[b], 1, |_| None);
        assert_eq!(store.reading(windows.view()).workspace_ids(b), independent);
        assert_eq!(
            store.reading(windows.view()).workspace_index(b, assigned),
            Some(0)
        );
        store.reading(windows.view()).validate_persisted_topology().unwrap();
    }

    #[test]
    fn moving_the_last_global_workspace_preserves_the_catalog() {
        let mut windows = OwnedWindows::default();
        let (a, b) = (SpaceId::new(901), SpaceId::new(902));
        let settings = VirtualWorkspaceSettings {
            mode: WorkspaceMode::Global,
            default_workspace_count: 1,
            workspace_names: vec![],
            ..Default::default()
        };
        let mut store = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());
        let original = store.create_workspace(&mut windows.view_mut(), a, None).unwrap();
        store.ensure_visible_spaces(&mut windows.view_mut(), &[a, b], 2, |_| None);
        assert_eq!(store.reading(windows.view()).active_workspace(b), None);
        while store.reading(windows.view()).catalog().count() < MAX_WORKSPACES {
            store.create_workspace(&mut windows.view_mut(), b, None).unwrap();
        }
        let catalog: Vec<_> = store.reading(windows.view()).catalog().collect();
        assert!(store.transfer_workspace(
            &mut windows.view_mut(),
            original,
            b,
            None,
            WorkspaceTransferReason::Explicit
        ));
        assert_eq!(store.reading(windows.view()).active_workspace(a), None);
        assert_eq!(store.reading(windows.view()).active_workspace(b), Some(original));
        // Rediscovery must retain a non-default assignment even on another display.
        let assigned = catalog[1];
        let mut windows = OwnedWindows::joining(&store.reading(windows.view()).topology());
        let window = WindowId::new(90, 1);
        store.assign_window_to_workspace(&mut windows.view_mut(), b, window, assigned);
        assert_eq!(
            expect_managed(store.apply_app_rule_decision(
                &mut windows.view_mut(),
                window,
                a,
                None,
                true
            ))
            .workspace_id,
            assigned
        );
        assert_eq!(
            store.reading(windows.view()).catalog().collect::<Vec<_>>(),
            catalog
        );
        store.reading(windows.view()).validate_persisted_topology().unwrap();
    }

    #[test]
    fn workspace_mode_wrap_empty_and_dormant_policy_matrix() {
        let (a, b, dormant) = (SpaceId::new(101), SpaceId::new(201), SpaceId::new(301));
        // mode, visible displays, current index, direction, skip empty, no wrap, target
        use Direction::{Left, Right};
        use WorkspaceMode::{Global, PerDisplay};
        #[rustfmt::skip]
        let cases = [
            (PerDisplay,  true,  0, Right, false, false, Some(1)),
            (PerDisplay,  true,  1, Left,  false, false, Some(0)),
            (PerDisplay,  true,  1, Right, false, false, Some(0)),
            (Global,      false, 1, Right, false, false, Some(0)),
            (Global,      true,  1, Right, false, false, Some(2)),
            (Global,      true,  0, Left,  false, false, Some(3)),
            (Global,      true,  3, Right, false, false, Some(0)),
            (Global,      true,  3, Right, false, true,  None),
            (Global,      true,  0, Left,  false, true,  None),
            (Global,      true,  0, Right, true,  false, Some(3)),
            (Global,      true,  0, Right, true,  true,  Some(3)),
            (Global,      true,  3, Right, true,  true,  None),
            (Global,      false, 0, Right, true,  false, None),
        ];
        for (mode, both, current, direction, skip, prevent_wrapping, expected) in cases {
            let settings = VirtualWorkspaceSettings {
                mode,
                default_workspace_count: 2,
                prevent_wrapping,
                ..Default::default()
            };
            let mut store = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());
            let mut windows = OwnedWindows::default();
            // Initialize separately so both modes have two workspaces per native Space.
            for space in [a, b, dormant] {
                store.ensure_space_initialized(&mut windows.view_mut(), space);
                while store.reading(windows.view()).workspace_ids(space).len() < 2 {
                    store.create_workspace(&mut windows.view_mut(), space, None).unwrap();
                }
            }
            let ids: Vec<_> = store.reading(windows.view()).catalog().collect();
            let mut windows = OwnedWindows::joining(&store.reading(windows.view()).topology());
            windows.view_mut().assign_window_to_workspace(WindowId::new(90, 1), ids[3]);
            windows.view_mut().assign_window_to_workspace(WindowId::new(90, 2), ids[4]);
            let context = WorkspaceContext {
                space: a,
                visible: if both { &[a, b] } else { &[a] },
            };
            assert_eq!(
                store.reading(windows.view()).step_workspace(
                    context,
                    ids[current],
                    Some(skip),
                    direction
                ),
                expected.map(|index| ids[index]),
                "{mode:?} {current} {direction:?}"
            );
        }
    }

    #[test]
    fn shared_initial_count_stays_fixed_across_topology_changes() {
        let (a, b, c) = (SpaceId::new(51), SpaceId::new(52), SpaceId::new(53));
        for (count, pin_all) in [
            (1, false),
            (6, false),
            (MAX_WORKSPACES, false),
            (MAX_WORKSPACES, true),
        ] {
            let settings = VirtualWorkspaceSettings {
                mode: WorkspaceMode::Global,
                default_workspace_count: count,
                workspace_names: vec![],
                workspace_rules: if pin_all {
                    (0..count)
                        .map(|index| WorkspaceLayoutRule {
                            workspace: WorkspaceSelector::Index(index),
                            layout: None,
                            display: Some(vec!["a".into()]),
                        })
                        .collect()
                } else {
                    vec![]
                },
                ..Default::default()
            };
            let mut store = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());
            let mut windows = OwnedWindows::default();
            store.ensure_visible_spaces(&mut windows.view_mut(), &[a, b], 2, |_| {
                pin_all.then_some(a)
            });
            let catalog: Vec<_> = store.reading(windows.view()).catalog().collect();
            assert_eq!(catalog.len(), count);
            for (index, &id) in catalog.iter().enumerate() {
                let boundary = if pin_all || count == 1 {
                    count
                } else {
                    count / 2
                };
                assert_eq!(
                    store.reading(windows.view()).workspace_space(id).unwrap(),
                    if index < boundary { a } else { b }
                );
            }
            store.ensure_visible_spaces(&mut windows.view_mut(), &[a, b, c], 3, |_| None);
            assert_eq!(store.reading(windows.view()).active_workspace(c), None);
            assert_eq!(
                store.reading(windows.view()).catalog().collect::<Vec<_>>(),
                catalog
            );
            let placement: Vec<_> = store
                .reading(windows.view())
                .catalog()
                .map(|id| (id, store.reading(windows.view()).workspace_space(id).unwrap()))
                .collect();
            let active =
                [a, b, c].map(|space| store.reading(windows.view()).active_workspace(space));
            // Display loss, return, and physical reordering must not run the initial
            // allocator again. Departure fallback/restoration is owned by the Reactor.
            for (visible, display_count) in [
                (vec![a], 1),
                (vec![a, b, c], 3),
                (vec![c, b, a], 3),
                (vec![a, b, c], 3),
            ] {
                store.ensure_visible_spaces(
                    &mut windows.view_mut(),
                    &visible,
                    display_count,
                    |_| pin_all.then_some(a),
                );
                assert_eq!(
                    store
                        .reading(windows.view())
                        .catalog()
                        .map(|id| (id, store.reading(windows.view()).workspace_space(id).unwrap()))
                        .collect::<Vec<_>>(),
                    placement,
                    "exposure must preserve catalog order and ownership"
                );
                assert_eq!(
                    [a, b, c].map(|space| store.reading(windows.view()).active_workspace(space)),
                    active
                );
            }
            store.reading(windows.view()).validate_persisted_topology().unwrap();
            store
                .reading(windows.view())
                .plan_settings(&settings)
                .expect("the existing catalog must allow configuration reload");
        }
    }

    #[test]
    fn initial_distribution_is_contiguous_balanced_and_pins_win() {
        // (displays, workspaces, 1-based pins (workspace, display)) -> owner per workspace.
        let cases: &[(usize, usize, &[(usize, usize)], &[usize])] = &[
            (2, 6, &[], &[0, 0, 0, 1, 1, 1]),
            (2, 10, &[(3, 0), (8, 1)], &[0, 0, 0, 0, 0, 1, 1, 1, 1, 1]),
            (3, 9, &[], &[0, 0, 0, 1, 1, 1, 2, 2, 2]),
            (2, 6, &[(2, 1)], &[1, 1, 1, 0, 0, 0]),
            (3, 6, &[(1, 2)], &[2, 2, 0, 0, 1, 1]),
            (2, 6, &[(1, 0), (2, 0), (3, 0), (4, 0)], &[0, 0, 0, 0, 1, 1]),
            (2, 4, &[(1, 0), (2, 1), (3, 0), (4, 1)], &[0, 1, 0, 1]),
            (3, 2, &[(1, 0), (2, 0)], &[0, 0]),
        ];
        for &(displays, count, pinned, expected) in cases {
            let mut pins = vec![None; count];
            for &(workspace, display) in pinned {
                pins[workspace - 1] = Some(display);
            }
            assert_eq!(
                distribute(displays, pins),
                expected,
                "{displays} displays {pinned:?}"
            );
        }
    }

    #[test]
    fn native_remap_preserves_shared_catalog_and_remaps_a_temporarily_away_home() {
        let settings = VirtualWorkspaceSettings {
            default_workspace_count: 1,
            mode: WorkspaceMode::Global,
            ..Default::default()
        };
        let mut store = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());
        let mut windows = OwnedWindows::default();
        let (old, fallback, new) = (SpaceId::new(41), SpaceId::new(42), SpaceId::new(43));
        let id = store.listed_after_init(&mut windows.view_mut(), old)[0].0;
        store[id].home = store[id].home.with_space(old);
        store.listed_after_init(&mut windows.view_mut(), fallback);
        store.transfer_workspace(
            &mut windows.view_mut(),
            id,
            fallback,
            Some("fallback"),
            WorkspaceTransferReason::DisplayDeparture,
        );
        let catalog: Vec<_> = store.reading(windows.view()).catalog().collect();
        store.remap_space(&mut windows.view_mut(), old, new);
        assert_eq!(store[id].home.space(), Some(new));
        assert_eq!(
            store.reading(windows.view()).workspace_space(id).unwrap(),
            fallback
        );
        store.transfer_workspace(
            &mut windows.view_mut(),
            id,
            new,
            Some("home"),
            WorkspaceTransferReason::Restoration,
        );
        assert_eq!(store.reading(windows.view()).active_workspace(new), Some(id));
        assert_eq!(
            store.reading(windows.view()).catalog().collect::<Vec<_>>(),
            catalog
        );
    }

    #[test]
    fn shared_catalog_resolution_retains_dormant_positions_and_unique_names() {
        let mut windows = OwnedWindows::default();
        let (a, b, dormant) = (SpaceId::new(11), SpaceId::new(21), SpaceId::new(12));
        let settings = VirtualWorkspaceSettings {
            default_workspace_count: 2,
            mode: WorkspaceMode::Global,
            ..Default::default()
        };
        let mut store = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());
        store.ensure_visible_spaces(&mut windows.view_mut(), &[a, b], 2, |_| None);
        let ids: Vec<_> = store.reading(windows.view()).catalog().collect();
        assert!(!store.rename_workspace(
            windows.view().core,
            b,
            ids[1],
            store[ids[0]].name.clone()
        ));
        let context = WorkspaceContext { space: a, visible: &[a, b] };
        assert_eq!(
            store
                .reading(windows.view())
                .resolve_workspace(context, &WorkspaceSelector::Name(store[ids[0]].name.clone())),
            Some(WorkspaceTarget { id: ids[0], space: a })
        );
        store.ensure_space_initialized(&mut windows.view_mut(), dormant);
        let switched = WorkspaceContext {
            space: dormant,
            visible: &[dormant, b],
            ..context
        };
        assert_eq!(
            store
                .reading(windows.view())
                .resolve_workspace(switched, &WorkspaceSelector::Index(0)),
            None
        );
        assert_eq!(
            store
                .reading(windows.view())
                .resolve_workspace(switched, &WorkspaceSelector::Index(1)),
            Some(WorkspaceTarget { id: ids[1], space: b })
        );
        assert_eq!(
            store
                .reading(windows.view())
                .workspace_view(switched)
                .map(|(index, _)| index)
                .collect::<Vec<_>>(),
            [1]
        );
        assert_eq!(store.reading(windows.view()).active_workspace(a), Some(ids[0]));
    }

    #[test]
    fn workspace_transfer_preserves_identity_membership_layout_and_explicit_home() {
        let source = SpaceId::new(31);
        let target = SpaceId::new(32);
        let mut store = WorkspaceStore::new();
        let mut windows = OwnedWindows::default();
        let id = store.listed_after_init(&mut windows.view_mut(), source)[0].0;
        store[id].ensure_layout_for_size(CGSize::new(1200.0, 800.0));
        let window = WindowId::new(5, 1);
        store.assign_window_to_workspace(&mut windows.view_mut(), source, window, id);
        store[id].set_last_focused(Some(window));
        store[id].home = Home::placed(source, Some("external".into()), PlacedBy::Automatic);
        let layout = store[id].active_layout();
        assert!(store.transfer_workspace(
            &mut windows.view_mut(),
            id,
            target,
            Some("builtin"),
            WorkspaceTransferReason::DisplayDeparture
        ));
        assert_eq!(windows.view().workspace_for_window(window).unwrap(), id);
        assert_eq!(store[id].active_layout(), layout);
        assert_eq!(store[id].last_focused(), Some(window));
        assert_eq!(store[id].home.display(), Some("external"));
        assert_eq!(store[id].home.space(), Some(source));
        assert_eq!(store[id].home.placed_by(), Some(PlacedBy::Automatic));
        assert!(store.transfer_workspace(
            &mut windows.view_mut(),
            id,
            target,
            Some("builtin"),
            WorkspaceTransferReason::Explicit
        ));
        assert_eq!(store[id].home.space(), Some(target));
        assert_eq!(store[id].home.display(), Some("builtin"));
        assert_eq!(store[id].home.placed_by(), Some(PlacedBy::User));
        assert_eq!(store.reading(windows.view()).active_workspace(target), Some(id));
        store.reading(windows.view()).validate_persisted_topology().unwrap();
    }

    #[test]
    fn test_virtual_workspace_creation() {
        let mut windows = OwnedWindows::default();
        let mut manager = WorkspaceStore::new();

        let space = SpaceId::new(1);
        assert_eq!(
            manager.listed_after_init(&mut windows.view_mut(), space).len(),
            manager.reading(windows.view()).workspace_ids(space).len()
        );

        let ws_id = manager
            .create_workspace(&mut windows.view_mut(), space, Some("Workspace 1".to_string()))
            .unwrap();
        assert!(
            manager
                .listed_after_init(&mut windows.view_mut(), space)
                .iter()
                .any(|(id, name)| *id == ws_id && name == "Workspace 1")
        );

        let workspace = manager.reading(windows.view()).workspace_info(space, ws_id).unwrap();
        assert_eq!(workspace.name, "Workspace 1");
        assert!(
            manager
                .create_workspace(&mut windows.view_mut(), space, Some("Workspace 1".into()))
                .is_err()
        );
        let generated = manager.create_workspace(&mut windows.view_mut(), space, None).unwrap();
        let name = manager[generated].name.clone();
        assert_ne!(name, "Workspace 1");
        assert!(!manager.rename_workspace(
            windows.view().core,
            space,
            generated,
            "Workspace 1".into()
        ));
        assert_eq!(manager[generated].name, name);
    }

    #[test]
    fn test_window_assignment() {
        let mut window_store = OwnedWindows::default();
        let mut manager = WorkspaceStore::new();
        let space = SpaceId::new(1);
        let ws1_id = manager
            .create_workspace(&mut window_store.view_mut(), space, Some("WS1".to_string()))
            .unwrap();
        let ws2_id = manager
            .create_workspace(&mut window_store.view_mut(), space, Some("WS2".to_string()))
            .unwrap();

        let window1 = WindowId::new(1, 1);
        let window2 = WindowId::new(1, 2);

        assert!(manager.assign_window_to_workspace(
            &mut window_store.view_mut(),
            space,
            window1,
            ws1_id
        ));
        assert!(manager.assign_window_to_workspace(
            &mut window_store.view_mut(),
            space,
            window2,
            ws2_id
        ));

        assert_eq!(
            manager.reading(window_store.view()).workspace_for_window(space, window1),
            Some(ws1_id)
        );
        assert_eq!(
            manager.reading(window_store.view()).workspace_for_window(space, window2),
            Some(ws2_id)
        );

        assert_eq!(window_store.view().workspace_windows(ws1_id), vec![window1]);
        assert_eq!(window_store.view().workspace_windows(ws2_id), vec![window2]);
    }

    #[test]
    fn reassignment_updates_authoritative_workspace_index() {
        let mut window_store = OwnedWindows::default();
        let mut manager = WorkspaceStore::new();
        let space = SpaceId::new(1);
        let ws1_id = manager
            .create_workspace(&mut window_store.view_mut(), space, Some("WS1".to_string()))
            .unwrap();
        let ws2_id = manager
            .create_workspace(&mut window_store.view_mut(), space, Some("WS2".to_string()))
            .unwrap();
        let window = WindowId::new(9, 1);

        assert!(manager.assign_window_to_workspace(
            &mut window_store.view_mut(),
            space,
            window,
            ws1_id
        ));
        assert_eq!(
            manager.reading(window_store.view()).workspace_for_window(space, window),
            Some(ws1_id)
        );
        assert_eq!(window_store.view().workspace_windows(ws1_id), vec![window]);

        assert!(manager.assign_window_to_workspace(
            &mut window_store.view_mut(),
            space,
            window,
            ws2_id
        ));
        assert_eq!(
            manager.reading(window_store.view()).workspace_for_window(space, window),
            Some(ws2_id)
        );
        assert!(window_store.view().workspace_windows(ws1_id).is_empty());
        assert_eq!(window_store.view().workspace_windows(ws2_id), vec![window]);
    }

    /// A workspace answers for, and is changed through, only the Space the core lists it on.
    #[test]
    fn a_workspace_answers_only_on_the_space_it_is_listed_on() {
        let mut windows = OwnedWindows::default();
        let mut store = WorkspaceStore::new();
        let (on, other) = (SpaceId::new(1), SpaceId::new(2));
        store.ensure_space_initialized(&mut windows.view_mut(), other);
        let id = store
            .create_workspace(&mut windows.view_mut(), on, Some("Here".to_string()))
            .unwrap();
        let window = WindowId::new(9, 1);

        store.set_last_focused_window(windows.view().core, other, id, Some(window));
        assert_eq!(
            store[id].last_focused(),
            None,
            "focus told on another Space is ignored"
        );
        store.set_last_focused_window(windows.view().core, on, id, Some(window));
        assert_eq!(
            store.reading(windows.view()).last_focused_window(on, id),
            Some(window)
        );
        assert_eq!(
            store.reading(windows.view()).last_focused_window(other, id),
            None
        );

        assert!(store.reading(windows.view()).workspace_info(on, id).is_some());
        assert!(store.reading(windows.view()).workspace_info(other, id).is_none());

        assert!(!store.rename_workspace(windows.view().core, other, id, "Elsewhere".into()));
        assert_eq!(store[id].name, "Here");
        assert!(store.rename_workspace(windows.view().core, on, id, "There".into()));
        assert_eq!(store[id].name, "There");
    }

    #[test]
    fn reassignment_clears_stale_last_focused_on_source_workspace() {
        let mut window_store = OwnedWindows::default();
        let mut manager = WorkspaceStore::new();
        let space = SpaceId::new(1);
        let ws1_id = manager
            .create_workspace(&mut window_store.view_mut(), space, Some("WS1".to_string()))
            .unwrap();
        let ws2_id = manager
            .create_workspace(&mut window_store.view_mut(), space, Some("WS2".to_string()))
            .unwrap();
        let window = WindowId::new(9, 1);

        assert!(manager.assign_window_to_workspace(
            &mut window_store.view_mut(),
            space,
            window,
            ws1_id
        ));
        manager.set_last_focused_window(window_store.view().core, space, ws1_id, Some(window));

        assert!(manager.assign_window_to_workspace(
            &mut window_store.view_mut(),
            space,
            window,
            ws2_id
        ));

        assert_eq!(
            manager.reading(window_store.view()).last_focused_window(space, ws1_id),
            None
        );
        assert_eq!(
            manager.reading(window_store.view()).workspace_for_window(space, window),
            Some(ws2_id)
        );
    }

    #[test]
    fn remap_space_migrates_workspaces_and_rehomes_deleted_target_windows() {
        let mut window_store = OwnedWindows::default();
        let mut manager = WorkspaceStore::new();
        let old_space = SpaceId::new(1);
        let new_space = SpaceId::new(2);
        let migrated_ws = manager
            .create_workspace(&mut window_store.view_mut(), old_space, Some("Old".to_string()))
            .unwrap();
        let transient_ws = manager
            .create_workspace(
                &mut window_store.view_mut(),
                new_space,
                Some("Transient".to_string()),
            )
            .unwrap();
        let migrated_window = WindowId::new(10, 1);
        let transient_window = WindowId::new(11, 1);

        assert!(manager.assign_window_to_workspace(
            &mut window_store.view_mut(),
            old_space,
            migrated_window,
            migrated_ws
        ));
        assert!(manager.assign_window_to_workspace(
            &mut window_store.view_mut(),
            new_space,
            transient_window,
            transient_ws
        ));

        manager[migrated_ws].replace_layout_mode(LayoutMode::Stack, &LayoutSettings::default());
        manager[migrated_ws].ensure_layout_for_size(CGSize::new(1200.0, 800.0));
        let first = manager[migrated_ws].active_layout();
        manager[migrated_ws].ensure_layout_for_size(CGSize::new(1600.0, 1000.0));
        manager[migrated_ws].set_last_focused(Some(migrated_window));
        let configurations = ron::ser::to_string(&manager[migrated_ws].layout_state).unwrap();
        manager.remap_space(&mut window_store.view_mut(), old_space, new_space);

        assert_eq!(
            manager.reading(window_store.view()).workspace_space(migrated_ws).unwrap(),
            new_space
        );
        assert_eq!(manager[migrated_ws].layout_mode, LayoutMode::Stack);
        assert_eq!(manager[migrated_ws].last_focused(), Some(migrated_window));
        assert_eq!(
            ron::ser::to_string(&manager[migrated_ws].layout_state).unwrap(),
            configurations
        );
        manager[migrated_ws].ensure_layout_for_size(CGSize::new(1200.0, 800.0));
        assert_eq!(manager[migrated_ws].active_layout(), first);
        assert!(!manager.reading(window_store.view()).layout_spaces().contains(&old_space));

        assert_eq!(
            manager
                .reading(window_store.view())
                .workspace_for_window(new_space, migrated_window),
            Some(migrated_ws)
        );
        let mut on_migrated = window_store.view().workspace_windows(migrated_ws);
        on_migrated.sort();
        assert_eq!(on_migrated, vec![migrated_window, transient_window]);
        // The window the window server had already placed keeps a workspace.
        assert_eq!(
            manager
                .reading(window_store.view())
                .workspace_for_window(new_space, transient_window),
            Some(migrated_ws)
        );
        assert!(window_store.view().workspace_windows(transient_ws).is_empty());
        assert!(
            manager
                .reading(window_store.view())
                .workspace_info(new_space, transient_ws)
                .is_none()
        );
    }

    #[test]
    fn remap_space_rehomes_windows_of_deleted_target_workspaces_by_ordinal() {
        let mut window_store = OwnedWindows::default();
        let mut manager = WorkspaceStore::new();
        let old_space = SpaceId::new(1);
        let new_space = SpaceId::new(2);
        let migrated: Vec<_> = manager
            .listed_after_init(&mut window_store.view_mut(), old_space)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        // One more than the migrated space has, so its last position has no counterpart.
        manager
            .create_workspace(
                &mut window_store.view_mut(),
                new_space,
                Some("Extra".to_string()),
            )
            .unwrap();
        let transient: Vec<_> = manager
            .listed_after_init(&mut window_store.view_mut(), new_space)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(transient.len(), migrated.len() + 1);
        let positions = [0, 1, transient.len() - 1];
        let windows: Vec<_> = (1..=3).map(|idx| WindowId::new(20, idx)).collect();
        for (window, &position) in windows.iter().zip(&positions) {
            assert!(manager.assign_window_to_workspace(
                &mut window_store.view_mut(),
                new_space,
                *window,
                transient[position]
            ));
        }

        let fullscreen = WindowId::new(21, 1);
        manager.assign_window_to_workspace(
            &mut window_store.view_mut(),
            new_space,
            fullscreen,
            transient[1],
        );
        window_store.view_mut().suspend_window_to_native_fullscreen(
            fullscreen,
            None,
            Some(new_space),
            SpaceId::new(99),
            crate::model::window_store::NativeFullscreenTransition::Suspended,
        );
        window_store.view_mut().remove_window(fullscreen);
        let deleted = manager.remap_space(&mut window_store.view_mut(), old_space, new_space);

        assert_eq!(
            window_store.native_fullscreen_record_for_window(fullscreen).unwrap().workspace,
            Some(migrated[1])
        );
        assert_eq!(deleted, transient, "callers are told every workspace that went");
        let landed: Vec<_> = windows
            .iter()
            .map(|w| manager.reading(window_store.view()).workspace_for_window(new_space, *w))
            .collect();
        assert_eq!(landed, vec![
            Some(migrated[0]),
            Some(migrated[1]),
            Some(*migrated.last().unwrap())
        ]);
    }

    #[test]
    fn generic_assignment_does_not_infer_space_id_churn_from_empty_target() {
        let mut window_store = OwnedWindows::default();
        let mut settings = VirtualWorkspaceSettings::default();
        settings.default_workspace_count = 3;
        let mut manager = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());
        let old_space = SpaceId::new(1);
        let new_space = SpaceId::new(2);
        let window = WindowId::new(12, 1);

        let old_workspaces = manager.listed_after_init(&mut window_store.view_mut(), old_space);
        let new_workspaces = manager.listed_after_init(&mut window_store.view_mut(), new_space);
        let preserved_workspace = old_workspaces[2].0;
        let expected_target_workspace = new_workspaces[0].0;

        assert!(manager.assign_window_to_workspace(
            &mut window_store.view_mut(),
            old_space,
            window,
            preserved_workspace
        ));

        let assignment = assign(
            &mut manager,
            &mut window_store.view_mut(),
            window,
            new_space,
            None,
            None,
            None,
            None,
            None,
        );

        assert_eq!(assignment.workspace_id, expected_target_workspace);
        assert_eq!(
            window_store.view().workspace_for_window(window),
            Some(expected_target_workspace)
        );
    }

    #[test]
    fn does_not_preserve_workspace_ordinal_when_target_space_already_has_assignments() {
        let mut window_store = OwnedWindows::default();
        let mut settings = VirtualWorkspaceSettings::default();
        settings.default_workspace_count = 3;
        let mut manager = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());
        let old_space = SpaceId::new(1);
        let new_space = SpaceId::new(2);
        let moved_window = WindowId::new(13, 1);
        let existing_window = WindowId::new(14, 1);

        let old_workspaces = manager.listed_after_init(&mut window_store.view_mut(), old_space);
        let new_workspaces = manager.listed_after_init(&mut window_store.view_mut(), new_space);

        assert!(manager.assign_window_to_workspace(
            &mut window_store.view_mut(),
            old_space,
            moved_window,
            old_workspaces[2].0
        ));
        assert!(manager.assign_window_to_workspace(
            &mut window_store.view_mut(),
            new_space,
            existing_window,
            new_workspaces[1].0
        ));

        let assignment = assign(
            &mut manager,
            &mut window_store.view_mut(),
            moved_window,
            new_space,
            None,
            None,
            None,
            None,
            None,
        );

        assert_eq!(assignment.workspace_id, new_workspaces[0].0);
        assert_eq!(
            window_store.view().workspace_for_window(moved_window),
            Some(new_workspaces[0].0)
        );
    }

    #[test]
    fn topology_reassignment_preserves_workspace_ordinal_with_destination_assignments() {
        let mut window_store = OwnedWindows::default();
        let mut settings = VirtualWorkspaceSettings::default();
        settings.default_workspace_count = 3;
        let mut manager = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());
        let source_space = SpaceId::new(1);
        let destination_space = SpaceId::new(2);
        let moved_window = WindowId::new(15, 1);
        let destination_window = WindowId::new(16, 1);

        let source_workspaces =
            manager.listed_after_init(&mut window_store.view_mut(), source_space);
        let destination_workspaces =
            manager.listed_after_init(&mut window_store.view_mut(), destination_space);
        assert!(manager.assign_window_to_workspace(
            &mut window_store.view_mut(),
            source_space,
            moved_window,
            source_workspaces[2].0
        ));
        assert!(manager.assign_window_to_workspace(
            &mut window_store.view_mut(),
            destination_space,
            destination_window,
            destination_workspaces[0].0
        ));

        assert_eq!(
            manager.reading(window_store.view()).workspace_at_window_ordinal(
                window_store.view(),
                destination_space,
                moved_window
            ),
            Some(destination_workspaces[2].0)
        );
    }

    #[test]
    fn unmanaged_rule_does_not_reassign_window_during_transient_space_id_churn() {
        let mut window_store = OwnedWindows::default();
        let mut settings = VirtualWorkspaceSettings::default();
        settings.default_workspace_count = 3;
        settings.app_rules = vec![AppWorkspaceRule {
            app_id: Some("com.example.unmanaged".into()),
            workspace: None,
            floating: false,
            position: None,
            size: None,
            respect_min_size: false,
            focus: false,
            manage: Some(false),
            app_name: None,
            title_regex: None,
            title_substring: None,
            ax_role: None,
            ax_subrole: None,
            window_tags: None,
        }];
        let mut manager = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());
        let old_space = SpaceId::new(1);
        let new_space = SpaceId::new(2);
        let window = WindowId::new(15, 1);

        let old_workspaces = manager.listed_after_init(&mut window_store.view_mut(), old_space);
        let old_assignment = old_workspaces[2].0;
        assert!(manager.assign_window_to_workspace(
            &mut window_store.view_mut(),
            old_space,
            window,
            old_assignment
        ));

        let result = manager.assign_window_with_app_info(
            &mut window_store.view_mut(),
            window,
            new_space,
            Some("com.example.unmanaged"),
            None,
            None,
            None,
            None,
        );

        assert!(matches!(
            result,
            Ok(AppRuleResult::Rejected(AppRuleRejection::ExplicitRule))
        ));
        assert_eq!(
            manager.reading(window_store.view()).workspace_for_window(new_space, window),
            None
        );
        assert_eq!(
            window_store.view().workspace_for_window(window),
            Some(old_assignment)
        );
    }

    #[test]
    fn test_active_workspace_switching() {
        let mut windows = OwnedWindows::default();
        let mut manager = WorkspaceStore::new();
        let space = SpaceId::new(1);
        let ws1_id = manager
            .create_workspace(&mut windows.view_mut(), space, Some("WS1".to_string()))
            .unwrap();
        let ws2_id = manager
            .create_workspace(&mut windows.view_mut(), space, Some("WS2".to_string()))
            .unwrap();

        assert!(manager.set_active_workspace(&mut windows.view_mut(), space, ws1_id));
        assert_eq!(
            manager.reading(windows.view()).active_workspace(space),
            Some(ws1_id)
        );

        assert!(manager.set_active_workspace(&mut windows.view_mut(), space, ws2_id));
        assert_eq!(
            manager.reading(windows.view()).active_workspace(space),
            Some(ws2_id)
        );
    }

    #[test]
    fn test_window_visibility() {
        let mut window_store = OwnedWindows::default();
        fn is_window_visible(
            wm: &WorkspaceStore,
            window_store: Windows<'_>,
            window_id: WindowId,
            space: SpaceId,
        ) -> bool {
            let window_workspace = wm.reading(window_store).workspace_for_window(space, window_id);
            let active_workspace = wm.reading(window_store).active_workspace(space);

            match (window_workspace, active_workspace) {
                (Some(window_ws), Some(active_ws)) => window_ws == active_ws,
                _ => true,
            }
        }
        let mut manager = WorkspaceStore::new();
        let space = SpaceId::new(1);
        let ws1_id = manager
            .create_workspace(&mut window_store.view_mut(), space, Some("WS1".to_string()))
            .unwrap();
        let ws2_id = manager
            .create_workspace(&mut window_store.view_mut(), space, Some("WS2".to_string()))
            .unwrap();
        let window1 = WindowId::new(1, 1);
        let window2 = WindowId::new(1, 2);

        manager.set_active_workspace(&mut window_store.view_mut(), space, ws1_id);
        manager.assign_window_to_workspace(&mut window_store.view_mut(), space, window1, ws1_id);
        manager.assign_window_to_workspace(&mut window_store.view_mut(), space, window2, ws2_id);

        assert!(is_window_visible(&manager, window_store.view(), window1, space));
        assert!(!is_window_visible(&manager, window_store.view(), window2, space));

        manager.set_active_workspace(&mut window_store.view_mut(), space, ws2_id);
        assert!(!is_window_visible(&manager, window_store.view(), window1, space));
        assert!(is_window_visible(&manager, window_store.view(), window2, space));
    }

    #[test]
    fn default_workspace_setting_applied() {
        let mut windows = OwnedWindows::default();
        let mut settings = VirtualWorkspaceSettings::default();
        settings.default_workspace_count = 5;
        settings.default_workspace = 3;

        let mut manager = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());

        let space = SpaceId::new(42);
        let workspaces = manager.listed_after_init(&mut windows.view_mut(), space);
        let expected_ws = workspaces.get(settings.default_workspace).unwrap().0;

        assert_eq!(
            manager.reading(windows.view()).active_workspace(space),
            Some(expected_ws)
        );
    }

    #[test]
    fn workspaces_saved_before_typed_homes_load_into_them_and_save_unchanged() {
        // Written by the code before Home and RuleStanding: every combination of the two home
        // fields, and a rule override; the last as upstream writes an explicit display move.
        let saved = [
            r#"(name:"Main",space:(2),preferred_display:None,home_space:None,rule_overridden:false,last_focused:None,layout_system:traditional((tree:(map:(map:[(value:None,version:0)]),window:(window_nodes:{})),layout_roots:[(value:None,version:0)])),layout_mode:traditional,layout_state:(layout:None,active_size:(width:0,height:0)))"#,
            r#"(name:"Development",space:(2),preferred_display:None,home_space:Some((2)),rule_overridden:false,last_focused:None,layout_system:traditional((tree:(map:(map:[(value:None,version:0)]),window:(window_nodes:{})),layout_roots:[(value:None,version:0)])),layout_mode:traditional,layout_state:(layout:None,active_size:(width:0,height:0)))"#,
            r#"(name:"Communication",space:(2),preferred_display:Some("display-uuid"),home_space:Some((3)),rule_overridden:false,last_focused:None,layout_system:traditional((tree:(map:(map:[(value:None,version:0)]),window:(window_nodes:{})),layout_roots:[(value:None,version:0)])),layout_mode:traditional,layout_state:(layout:None,active_size:(width:0,height:0)))"#,
            r#"(name:"Utilities",space:(2),preferred_display:Some("legacy-display"),home_space:None,rule_overridden:true,last_focused:None,layout_system:traditional((tree:(map:(map:[(value:None,version:0)]),window:(window_nodes:{})),layout_roots:[(value:None,version:0)])),layout_mode:traditional,layout_state:(layout:None,active_size:(width:0,height:0)))"#,
            r#"(name:"Chosen",space:(2),preferred_display:Some("display-uuid"),home_space:Some((3)),display_manually_selected:true,rule_overridden:false,last_focused:None,layout_system:traditional((tree:(map:(map:[(value:None,version:0)]),window:(window_nodes:{})),layout_roots:[(value:None,version:0)])),layout_mode:traditional,layout_state:(layout:None,active_size:(width:0,height:0)))"#,
        ];
        let expected = [
            (Home::Unplaced, RuleStanding::Followed),
            (
                Home::Space {
                    space: SpaceId::new(2),
                    by: PlacedBy::Automatic,
                },
                RuleStanding::Followed,
            ),
            (
                Home::OnDisplay {
                    space: SpaceId::new(3),
                    display: "display-uuid".into(),
                    by: PlacedBy::Automatic,
                },
                RuleStanding::Followed,
            ),
            (
                Home::DisplayOnly("legacy-display".into()),
                RuleStanding::Overridden,
            ),
            (
                Home::OnDisplay {
                    space: SpaceId::new(3),
                    display: "display-uuid".into(),
                    by: PlacedBy::User,
                },
                RuleStanding::Followed,
            ),
        ];
        for (saved, (home, standing)) in saved.into_iter().zip(expected) {
            let (workspace, space) = ron::from_str::<SavedWorkspace>(saved).unwrap().split();
            assert_eq!((&workspace.home, workspace.rule_standing), (&home, standing));
            // Written back field for field, with upstream's `display_manually_selected` added
            // where the file predates it.
            let written = if saved.contains("display_manually_selected") {
                saved.to_owned()
            } else {
                saved.replace(
                    ",rule_overridden:",
                    ",display_manually_selected:false,rule_overridden:",
                )
            };
            assert_eq!(
                ron::ser::to_string(&workspace.saved(space)).unwrap(),
                written,
                "the saved form is unchanged"
            );
        }
    }

    #[test]
    fn display_balance_moves_only_workspaces_rift_placed_and_arrival_keeps_who_chose() {
        let mut windows = OwnedWindows::default();
        let (a, b) = (SpaceId::new(71), SpaceId::new(72));
        let settings = VirtualWorkspaceSettings {
            mode: WorkspaceMode::Global,
            default_workspace_count: 4,
            ..Default::default()
        };
        let mut store = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());
        store.ensure_visible_spaces(&mut windows.view_mut(), &[a], 2, |_| None);
        let ids: Vec<_> = store.reading(windows.view()).catalog().collect();
        assert_eq!(store.reading(windows.view()).workspace_ids(a), ids);
        assert!(ids.iter().all(|&id| store[id].home.placed_by() == Some(PlacedBy::Automatic)));
        let none = HashSet::default();
        assert_eq!(
            store.reading(windows.view()).plan_display_balance(&[a, b], &none),
            [(ids[2], b), (ids[3], b)]
        );
        // Choosing the display a workspace is already on marks the home chosen, and balancing
        // then leaves it where it is.
        assert!(store.transfer_workspace(
            &mut windows.view_mut(),
            ids[3],
            a,
            None,
            WorkspaceTransferReason::Explicit
        ));
        assert_eq!(store[ids[3]].home, Home::Space { space: a, by: PlacedBy::User });
        assert!(!store.transfer_workspace(
            &mut windows.view_mut(),
            ids[3],
            a,
            None,
            WorkspaceTransferReason::Explicit
        ));
        assert_eq!(
            store.reading(windows.view()).plan_display_balance(&[a, b], &none),
            [(ids[1], b), (ids[2], b)]
        );
        assert!(store.reading(windows.view()).plan_display_balance(&[a], &none).is_empty());
        // An arriving display becomes the home; who chose the old home still stands.
        assert!(store.transfer_workspace(
            &mut windows.view_mut(),
            ids[2],
            b,
            Some("external"),
            WorkspaceTransferReason::DisplayArrival
        ));
        assert_eq!(store[ids[2]].home, Home::OnDisplay {
            space: b,
            display: "external".into(),
            by: PlacedBy::Automatic,
        });
        assert_eq!(store.reading(windows.view()).active_workspace(b), Some(ids[2]));
        assert!(store.transfer_workspace(
            &mut windows.view_mut(),
            ids[3],
            b,
            Some("external"),
            WorkspaceTransferReason::DisplayArrival
        ));
        assert_eq!(store[ids[3]].home.placed_by(), Some(PlacedBy::User));
        assert_eq!(store.reading(windows.view()).active_workspace(b), Some(ids[2]));
        store.reading(windows.view()).validate_persisted_topology().unwrap();
    }

    #[test]
    fn a_saved_manual_selection_without_a_home_space_is_dropped() {
        // Upstream sets `display_manually_selected` only with `home_space`; anything else was
        // edited by hand, and the flag has no placed home to belong to.
        let saved = r#"(name:"Utilities",space:(2),preferred_display:Some("legacy-display"),home_space:None,rule_overridden:true,last_focused:None,layout_system:traditional((tree:(map:(map:[(value:None,version:0)]),window:(window_nodes:{})),layout_roots:[(value:None,version:0)])),layout_mode:traditional,layout_state:(layout:None,active_size:(width:0,height:0)))"#
            .replace(
                ",rule_overridden:true",
                ",display_manually_selected:true,rule_overridden:true",
            );
        let (workspace, space) = ron::from_str::<SavedWorkspace>(&saved).unwrap().split();
        assert_eq!(workspace.home, Home::DisplayOnly("legacy-display".into()));
        assert_eq!(workspace.home.placed_by(), None);
        assert!(
            ron::ser::to_string(&workspace.saved(space))
                .unwrap()
                .contains(",display_manually_selected:false,")
        );
    }

    #[test]
    fn a_space_left_with_history_and_nothing_shown_drops_the_history() {
        let mut windows = OwnedWindows::default();
        // Global mode: a switch gives the Space history; a departure takes both workspaces.
        let settings = VirtualWorkspaceSettings {
            default_workspace_count: 2,
            mode: WorkspaceMode::Global,
            ..Default::default()
        };
        let mut store = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());
        let (source, target) = (SpaceId::new(61), SpaceId::new(62));
        let on_source: Vec<_> = store
            .listed_after_init(&mut windows.view_mut(), source)
            .iter()
            .map(|(id, _)| *id)
            .collect();
        store.listed_after_init(&mut windows.view_mut(), target);
        let [first, second] = [on_source[0], on_source[1]];
        store.set_active_workspace(&mut windows.view_mut(), source, first);
        store.set_active_workspace(&mut windows.view_mut(), source, second);
        assert_eq!(store.reading(windows.view()).last_workspace(source), Some(first));
        for id in [second, first] {
            store.transfer_workspace(
                &mut windows.view_mut(),
                id,
                target,
                Some("other"),
                WorkspaceTransferReason::DisplayDeparture,
            );
        }
        // Nothing shown, the departed workspace remembered, no history.
        assert_eq!(
            store.reading(windows.view()).active_workspace_per_space()[&source],
            SpaceActivation::Vacant { remembered: Some(second) }
        );
        assert_eq!(store.reading(windows.view()).last_workspace(source), None);
        // The history would not have survived the return: showing a workspace again makes the
        // one shown until then (none) the previous one.
        store.transfer_workspace(
            &mut windows.view_mut(),
            second,
            source,
            Some("home"),
            WorkspaceTransferReason::Restoration,
        );
        assert_eq!(
            store.reading(windows.view()).active_workspace_per_space()[&source],
            SpaceActivation::Showing {
                current: second,
                previous: None,
                remembered: None
            }
        );
    }

    #[test]
    fn a_departed_display_shows_a_fallback_remembers_its_workspace_and_takes_it_back() {
        let mut windows = OwnedWindows::default();
        let settings = VirtualWorkspaceSettings {
            default_workspace_count: 2,
            mode: WorkspaceMode::PerDisplay,
            ..Default::default()
        };
        let mut store = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());
        let (source, target) = (SpaceId::new(51), SpaceId::new(52));
        let [shown, fallback] =
            [0, 1].map(|i| store.listed_after_init(&mut windows.view_mut(), source)[i].0);
        store.listed_after_init(&mut windows.view_mut(), target);
        store.set_active_workspace(&mut windows.view_mut(), source, shown);
        // The display leaves; another workspace stays on its Space.
        assert!(store.transfer_workspace(
            &mut windows.view_mut(),
            shown,
            target,
            Some("other"),
            WorkspaceTransferReason::DisplayDeparture
        ));
        assert_eq!(
            store.reading(windows.view()).active_workspace_per_space()[&source],
            SpaceActivation::Showing {
                current: fallback,
                previous: None,
                remembered: Some(shown)
            }
        );
        // It comes back and takes its workspace back.
        assert!(store.transfer_workspace(
            &mut windows.view_mut(),
            shown,
            source,
            Some("home"),
            WorkspaceTransferReason::Restoration
        ));
        assert_eq!(
            store.reading(windows.view()).active_workspace(source),
            Some(shown)
        );
        assert_eq!(
            store.reading(windows.view()).active_workspace_per_space()[&source].remembered(),
            None
        );
        store.reading(windows.view()).validate_persisted_topology().unwrap();
    }

    /// Collects the messages of the events logged while `run` runs on this thread.
    fn logged(run: impl FnOnce()) -> Vec<String> {
        use std::sync::{Arc, Mutex};
        struct Collect(Arc<Mutex<Vec<String>>>);
        struct Message<'a>(&'a mut String);
        impl tracing::field::Visit for Message<'_> {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    *self.0 = format!("{value:?}");
                }
            }
        }
        impl tracing::Subscriber for Collect {
            fn enabled(&self, _: &tracing::Metadata<'_>) -> bool { true }

            fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
                tracing::span::Id::from_u64(1)
            }

            fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

            fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

            fn event(&self, event: &tracing::Event<'_>) {
                let mut message = String::new();
                event.record(&mut Message(&mut message));
                self.0.lock().unwrap().push(message);
            }

            fn enter(&self, _: &tracing::span::Id) {}

            fn exit(&self, _: &tracing::span::Id) {}
        }
        let lines = Arc::new(Mutex::new(Vec::new()));
        tracing::subscriber::with_default(Collect(lines.clone()), run);
        lines.lock().unwrap().clone()
    }

    #[test]
    fn a_saved_activation_with_history_and_nothing_shown_loads_vacant_and_says_so() {
        let mut ids = SlotMap::<VirtualWorkspaceId, ()>::with_key();
        let (previous, remembered) = (ids.insert(()), ids.insert(()));
        let space = SpaceId::new(7);
        let saved = |previous: Option<VirtualWorkspaceId>,
                     remembered: Option<VirtualWorkspaceId>| {
            SavedActivation {
                current: None,
                previous,
                remembered,
            }
        };
        let mut loaded = None;
        let lines = logged(|| {
            loaded = Some(SpaceActivation::from_saved(
                space,
                saved(Some(previous), Some(remembered)),
            ))
        });
        assert_eq!(
            loaded,
            Some(SpaceActivation::Vacant { remembered: Some(remembered) })
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("workspace-load: normalized"), "{lines:?}");
        // Without history, nothing to normalize and nothing said.
        let lines = logged(|| {
            loaded = Some(SpaceActivation::from_saved(space, saved(None, Some(remembered))))
        });
        assert_eq!(
            loaded,
            Some(SpaceActivation::Vacant { remembered: Some(remembered) })
        );
        assert!(lines.is_empty(), "{lines:?}");
    }

    #[test]
    fn a_saved_activation_showing_and_remembering_loads_as_it_was() {
        let mut ids = SlotMap::<VirtualWorkspaceId, ()>::with_key();
        let (current, previous, remembered) = (ids.insert(()), ids.insert(()), ids.insert(()));
        let space = SpaceId::new(7);
        let mut loaded = None;
        let lines = logged(|| {
            loaded = Some(SpaceActivation::from_saved(space, SavedActivation {
                current: Some(current),
                previous: Some(previous),
                remembered: Some(remembered),
            }))
        });
        assert_eq!(
            loaded,
            Some(SpaceActivation::Showing {
                current,
                previous: Some(previous),
                remembered: Some(remembered)
            })
        );
        assert!(lines.is_empty(), "{lines:?}");
    }

    #[test]
    fn a_store_saved_before_the_catalog_type_loads_and_saves_unchanged() {
        // Written by the code before Catalog: two Spaces, two workspaces each.
        let saved = r#"(workspaces:[(value:None,version:0),(value:Some((name:"One",space:(3),preferred_display:None,home_space:Some((3)),rule_overridden:false,last_focused:None,layout_system:traditional((tree:(map:(map:[(value:None,version:0)]),window:(window_nodes:{})),layout_roots:[(value:None,version:0)])),layout_mode:traditional,layout_state:(layout:None,active_size:(width:0,height:0)))),version:1),(value:Some((name:"Two",space:(3),preferred_display:None,home_space:Some((3)),rule_overridden:false,last_focused:None,layout_system:traditional((tree:(map:(map:[(value:None,version:0)]),window:(window_nodes:{})),layout_roots:[(value:None,version:0)])),layout_mode:traditional,layout_state:(layout:None,active_size:(width:0,height:0)))),version:1),(value:Some((name:"One",space:(4),preferred_display:None,home_space:Some((4)),rule_overridden:false,last_focused:None,layout_system:traditional((tree:(map:(map:[(value:None,version:0)]),window:(window_nodes:{})),layout_roots:[(value:None,version:0)])),layout_mode:traditional,layout_state:(layout:None,active_size:(width:0,height:0)))),version:1),(value:Some((name:"Two",space:(4),preferred_display:None,home_space:Some((4)),rule_overridden:false,last_focused:None,layout_system:traditional((tree:(map:(map:[(value:None,version:0)]),window:(window_nodes:{})),layout_roots:[(value:None,version:0)])),layout_mode:traditional,layout_state:(layout:None,active_size:(width:0,height:0)))),version:1)],order:[(idx:1,version:1),(idx:2,version:1),(idx:3,version:1),(idx:4,version:1)],single_display:false,active_workspace_per_space:{(3):(current:Some((idx:1,version:1)),previous:None,remembered:None),(4):(current:Some((idx:3,version:1)),previous:None,remembered:None)})"#;
        let LoadedWorkspaces { store, topology } = load_store(saved);
        let windows = OwnedWindows::joining(&topology);
        assert_eq!(store.reading(windows.view()).catalog().count(), 4);
        assert_eq!(store.workspaces.len(), 4);
        // Unchanged but for upstream's `display_manually_selected`, written on every workspace.
        assert_eq!(
            save_store(&store, &windows),
            saved.replace(
                ",rule_overridden:",
                ",display_manually_selected:false,rule_overridden:"
            ),
            "the saved form is unchanged"
        );
    }

    #[test]
    fn a_catalog_lists_every_workspace_once_and_a_saved_one_that_does_not_is_refused() {
        let settings = LayoutSettings::default();
        let workspace =
            |name: &str| VirtualWorkspace::new(name.into(), LayoutMode::default(), &settings);
        // A workspace as a file holds it, on Space 1.
        let record = |name: &str| {
            let saved = ron::ser::to_string(&workspace(name).saved(SpaceId::new(1))).unwrap();
            ron::from_str::<SavedWorkspace>(&saved).unwrap()
        };
        let mut map = SlotTable::<VirtualWorkspaceId, SavedWorkspace>::default();
        let (a, b) = (map.insert(record("a")), map.insert(record("b")));
        let mut gone = SlotMap::<VirtualWorkspaceId, ()>::with_key();
        let _ = gone.insert(());
        let _ = gone.insert(());
        let unknown = gone.insert(());
        let parse = |order: Vec<VirtualWorkspaceId>| {
            let map = map.iter().map(|(_, w)| w.name.clone()).collect::<Vec<_>>();
            let mut copy = SlotTable::<VirtualWorkspaceId, SavedWorkspace>::default();
            for name in map {
                copy.insert(record(&name));
            }
            Catalog::parse(copy, order)
                .map(|(_, order)| order.into_iter().map(|(id, _)| id).collect::<Vec<_>>())
        };
        assert_eq!(parse(vec![b, a]), Ok(vec![b, a]));
        assert_eq!(parse(vec![a, unknown, b]), Err(CatalogError::Unknown(unknown)));
        assert_eq!(parse(vec![a, b, a]), Err(CatalogError::Duplicate(a)));
        assert_eq!(
            parse(vec![a]),
            Err(CatalogError::Unlisted { id: b, space: SpaceId::new(1) })
        );
        // The order is the core's now; the catalog keeps each workspace by id.
        let (mut catalog, _) = Catalog::parse(map, vec![a, b]).unwrap();
        let c = catalog.insert(workspace("c"));
        assert_eq!(catalog.remove(b).map(|w| w.name), Some("b".into()));
        assert!(catalog.remove(b).is_none());
        assert_eq!(catalog.keys().collect::<Vec<_>>(), vec![a, c]);
    }

    #[test]
    fn a_saved_store_whose_order_leaves_a_workspace_out_does_not_load() {
        let saved = r#"(workspaces:[(value:None,version:0),(value:Some((name:"One",space:(3),preferred_display:None,home_space:Some((3)),rule_overridden:false,last_focused:None,layout_system:traditional((tree:(map:(map:[(value:None,version:0)]),window:(window_nodes:{})),layout_roots:[(value:None,version:0)])),layout_mode:traditional,layout_state:(layout:None,active_size:(width:0,height:0)))),version:1),(value:Some((name:"Two",space:(3),preferred_display:None,home_space:Some((3)),rule_overridden:false,last_focused:None,layout_system:traditional((tree:(map:(map:[(value:None,version:0)]),window:(window_nodes:{})),layout_roots:[(value:None,version:0)])),layout_mode:traditional,layout_state:(layout:None,active_size:(width:0,height:0)))),version:1),(value:Some((name:"One",space:(4),preferred_display:None,home_space:Some((4)),rule_overridden:false,last_focused:None,layout_system:traditional((tree:(map:(map:[(value:None,version:0)]),window:(window_nodes:{})),layout_roots:[(value:None,version:0)])),layout_mode:traditional,layout_state:(layout:None,active_size:(width:0,height:0)))),version:1),(value:Some((name:"Two",space:(4),preferred_display:None,home_space:Some((4)),rule_overridden:false,last_focused:None,layout_system:traditional((tree:(map:(map:[(value:None,version:0)]),window:(window_nodes:{})),layout_roots:[(value:None,version:0)])),layout_mode:traditional,layout_state:(layout:None,active_size:(width:0,height:0)))),version:1)],order:[(idx:1,version:1),(idx:2,version:1),(idx:3,version:1),(idx:4,version:1)],single_display:false,active_workspace_per_space:{(3):(current:Some((idx:1,version:1)),previous:None,remembered:None),(4):(current:Some((idx:3,version:1)),previous:None,remembered:None)})"#;
        let broken = saved.replace(",(idx:4,version:1)]", "]");
        assert_ne!(broken, saved);
        let error = ron::from_str::<LoadedWorkspaces>(&broken).err().unwrap().to_string();
        assert!(error.contains("is not indexed"), "{error}");
    }

    /// Duplicate names in a saved store are told apart in catalog order (the saved `order`, not
    /// the slots'): the first of them keeps its name.
    #[test]
    fn duplicate_saved_names_are_told_apart_in_catalog_order() {
        let saved = r#"(workspaces:[(value:None,version:0),(value:Some((name:"One",space:(3),preferred_display:None,home_space:Some((3)),rule_overridden:false,last_focused:None,layout_system:traditional((tree:(map:(map:[(value:None,version:0)]),window:(window_nodes:{})),layout_roots:[(value:None,version:0)])),layout_mode:traditional,layout_state:(layout:None,active_size:(width:0,height:0)))),version:1),(value:Some((name:"Two",space:(3),preferred_display:None,home_space:Some((3)),rule_overridden:false,last_focused:None,layout_system:traditional((tree:(map:(map:[(value:None,version:0)]),window:(window_nodes:{})),layout_roots:[(value:None,version:0)])),layout_mode:traditional,layout_state:(layout:None,active_size:(width:0,height:0)))),version:1),(value:Some((name:"One",space:(4),preferred_display:None,home_space:Some((4)),rule_overridden:false,last_focused:None,layout_system:traditional((tree:(map:(map:[(value:None,version:0)]),window:(window_nodes:{})),layout_roots:[(value:None,version:0)])),layout_mode:traditional,layout_state:(layout:None,active_size:(width:0,height:0)))),version:1),(value:Some((name:"Two",space:(4),preferred_display:None,home_space:Some((4)),rule_overridden:false,last_focused:None,layout_system:traditional((tree:(map:(map:[(value:None,version:0)]),window:(window_nodes:{})),layout_roots:[(value:None,version:0)])),layout_mode:traditional,layout_state:(layout:None,active_size:(width:0,height:0)))),version:1)],order:[(idx:1,version:1),(idx:2,version:1),(idx:3,version:1),(idx:4,version:1)],single_display:false,active_workspace_per_space:{(3):(current:Some((idx:1,version:1)),previous:None,remembered:None),(4):(current:Some((idx:3,version:1)),previous:None,remembered:None)})"#;
        let saved = saved.replace(r#""Two",space:(3)"#, r#""One",space:(3)"#).replace(
            "order:[(idx:1,version:1),(idx:2,version:1)",
            "order:[(idx:2,version:1),(idx:1,version:1)",
        );
        let LoadedWorkspaces { mut store, topology } = load_store(&saved);
        let (second, first) = (topology.catalog()[1].0, topology.catalog()[0].0);
        store.normalize_loaded_names(&topology);
        assert_eq!(store[first].name, "One");
        assert_eq!(store[second].name, "One (2)");
    }

    /// A per-display store on Spaces 71, 72, 73 drawn from `seed`: one to four workspaces on
    /// each, every Space showing one of its own (or nothing), with history and memory pointing
    /// anywhere; and the window store joined to it, holding a window on each workspace.
    fn drawn_store(seed: u64) -> (WorkspaceStore, OwnedWindows, [SpaceId; 3]) {
        let mut state = seed | 1;
        let mut draw = |n: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % n
        };
        let settings = VirtualWorkspaceSettings {
            default_workspace_count: 1,
            mode: WorkspaceMode::PerDisplay,
            ..Default::default()
        };
        let mut store = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());
        let mut windows = OwnedWindows::default();
        let spaces = [SpaceId::new(71), SpaceId::new(72), SpaceId::new(73)];
        for space in spaces {
            store.ensure_space_initialized(&mut windows.view_mut(), space);
            for _ in 0..draw(4) {
                store.create_workspace(&mut windows.view_mut(), space, None).unwrap();
            }
        }
        let all: Vec<_> = store.reading(windows.view()).catalog().collect();
        let any = |draw: &mut dyn FnMut(u64) -> u64| {
            (draw(3) > 0).then(|| all[draw(all.len() as u64) as usize])
        };
        for space in spaces {
            let own = store.reading(windows.view()).workspace_ids(space);
            let activation = if draw(5) == 0 {
                SpaceActivation::Vacant { remembered: any(&mut draw) }
            } else {
                let current = own[draw(own.len() as u64) as usize];
                SpaceActivation::Showing {
                    current,
                    previous: any(&mut draw).filter(|&id| id != current),
                    remembered: any(&mut draw),
                }
            };
            store.set_activation(&mut windows.view_mut(), space, activation);
        }
        for (index, &id) in all.iter().enumerate() {
            let window = WindowId::new(70, index as u32 + 1);
            windows.view_mut().insert_test_ax_state(window);
            windows.view_mut().assign_window_to_workspace(window, id);
        }
        (store, windows, spaces)
    }

    /// The catalog (each workspace with its Space, in order) and every Space's activation.
    fn topology_of(
        topology: &SavedTopology,
    ) -> (
        Vec<(VirtualWorkspaceId, SpaceId)>,
        Vec<(SpaceId, SpaceActivation)>,
    ) {
        let mut activations: Vec<_> = topology
            .activations()
            .iter()
            .map(|(space, activation)| (*space, *activation))
            .collect();
        activations.sort_by_key(|(space, _)| *space);
        (topology.catalog().to_vec(), activations)
    }

    /// `remove_space` now clears what the Space shows and drops every other Space's history and
    /// memory of the removed workspaces before removing them (the joined core refuses to remove a
    /// workspace a Space still shows); before W4b it removed them first. Over drawn stores both
    /// orders leave the same catalog and activations. And the heirs `remap_space` now reads off
    /// the old Space before anything moves are the workspaces it read off the new Space after
    /// the move, so each deleted workspace's windows go where they went before.
    #[test]
    fn removing_a_space_clears_it_first_and_ends_where_removing_first_did() {
        for seed in 1..=500_u64 {
            let (mut store, mut windows, spaces) = drawn_store(seed);
            let space = spaces[(seed % 3) as usize];
            let mut before = store.reading(windows.view()).topology();

            let removed =
                store.remove_space(&mut windows.view_mut(), space, |_| Successor::Unassign);

            // The order before W4b: the workspaces removed first, then the activations.
            let removed_before = before.workspace_ids(space);
            before.catalog.retain(|(id, _)| !removed_before.contains(id));
            before.activations.remove(&space);
            for activation in before.activations.values_mut() {
                activation.retain(|id| !removed_before.contains(&id));
            }

            assert_eq!(removed, removed_before, "seed {seed}");
            assert_eq!(
                topology_of(&store.reading(windows.view()).topology()),
                topology_of(&before),
                "seed {seed}"
            );
            assert_eq!(windows.view().ownership_divergences(), vec![]);
        }
        for seed in 1..=500_u64 {
            let (mut store, mut windows, spaces) = drawn_store(seed);
            let (old, new) = (spaces[0], spaces[1 + (seed % 2) as usize]);
            let heirs = store.reading(windows.view()).workspace_ids(old);
            let deleted = store.reading(windows.view()).workspace_ids(new);
            let placed: Vec<_> =
                deleted.iter().map(|&id| (id, windows.view().workspace_windows(id))).collect();

            store.remap_space(&mut windows.view_mut(), old, new);

            // Before W4b the heirs were read off the new Space after the move.
            assert_eq!(
                store.reading(windows.view()).workspace_ids(new),
                heirs,
                "seed {seed}"
            );
            for (ordinal, (_, placed)) in placed.iter().enumerate() {
                let heir = heirs.get(ordinal).or_else(|| heirs.last()).copied();
                for &window in placed {
                    assert_eq!(windows.view().workspace_for_window(window), heir, "seed {seed}");
                }
            }
            assert_eq!(windows.view().ownership_divergences(), vec![]);
        }
    }

    /// A query never initialises a Space, so every Space a query can list must be initialised by
    /// the event that brought it: whatever the mode, the workspace count and the displays coming
    /// and going, every Space `ensure_visible_spaces` exposes is initialised when it returns.
    #[test]
    fn every_space_ensure_visible_spaces_exposes_is_initialised() {
        let spaces = [
            SpaceId::new(81),
            SpaceId::new(82),
            SpaceId::new(83),
            SpaceId::new(84),
        ];
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut draw = |n: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % n
        };
        let mut exposures = 0;
        for case in 0..400 {
            let settings = VirtualWorkspaceSettings {
                mode: [WorkspaceMode::PerDisplay, WorkspaceMode::Global][(case % 2) as usize],
                default_workspace_count: 1 + draw(6) as usize,
                ..Default::default()
            };
            let mut store = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());
            let mut windows = OwnedWindows::default();
            for _ in 0..8 {
                let visible: Vec<SpaceId> =
                    spaces.iter().copied().filter(|_| draw(2) == 0).collect();
                if visible.is_empty() {
                    continue;
                }
                let pinned = (draw(3) == 0).then(|| visible[0]);
                store.ensure_visible_spaces(
                    &mut windows.view_mut(),
                    &visible,
                    visible.len(),
                    |_| pinned,
                );
                for &space in &visible {
                    assert!(
                        store.reading(windows.view()).initialized(space),
                        "case {case}: {space:?} exposed among {visible:?} is not initialised: {:?}",
                        store.reading(windows.view()).activation(space),
                    );
                    assert!(matches!(
                        store.reading(windows.view()).list_workspaces(space),
                        Listing::Known(_)
                    ));
                    exposures += 1;
                }
            }
        }
        assert!(exposures > 1000);
    }

    /// Global mode with more displays than workspaces: a display left without one used to get its
    /// activation (`Vacant`) only when a query first listed it, so whether a remap carried the
    /// Space along and what the saved file held depended on whether a query had run. Now the
    /// exposure initialises it: with or without a query first, the remap moves it and the saved
    /// store is the same.
    #[test]
    fn remap_and_the_saved_store_do_not_depend_on_a_query_having_run() {
        let (shown, vacant, renumbered) = (SpaceId::new(91), SpaceId::new(92), SpaceId::new(93));
        let exposed = || {
            let settings = VirtualWorkspaceSettings {
                mode: WorkspaceMode::Global,
                default_workspace_count: 1,
                ..Default::default()
            };
            let mut store = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());
            let mut windows = OwnedWindows::default();
            store.ensure_visible_spaces(&mut windows.view_mut(), &[shown, vacant], 2, |_| None);
            (store, windows)
        };
        let saved_after_remap = |queried: bool| {
            let (mut store, mut windows) = exposed();
            if queried {
                assert_eq!(
                    store.reading(windows.view()).list_workspaces(vacant),
                    Listing::Known(vec![])
                );
            }
            store.remap_space(&mut windows.view_mut(), vacant, renumbered);
            assert_eq!(
                store.reading(windows.view()).active_workspace_per_space().get(&renumbered),
                Some(&SpaceActivation::Vacant { remembered: None }),
                "queried first: {queried}"
            );
            assert!(
                !store.reading(windows.view()).active_workspace_per_space().contains_key(&vacant)
            );
            save_store(&store, &windows)
        };
        assert_eq!(saved_after_remap(false), saved_after_remap(true));
    }

    #[test]
    fn legacy_catalog_preserves_custom_ordinal_order() {
        let mut windows = OwnedWindows::default();
        let settings = VirtualWorkspaceSettings {
            default_workspace_count: 3,
            workspace_names: vec!["A".into(), "B".into(), "C".into()],
            ..Default::default()
        };
        let mut store = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());
        let space = SpaceId::new(2);
        let ids = store.listed_after_init(&mut windows.view_mut(), space);
        let mut reversed = store.reading(windows.view()).workspace_ids(space);
        reversed.reverse();
        let serialized = save_store(&store, &windows);
        let current = format!(
            "order:{}",
            ron::ser::to_string(&store.reading(windows.view()).catalog().collect::<Vec<_>>())
                .unwrap()
        );
        let legacy = format!(
            "workspaces_by_space:{}",
            ron::ser::to_string(&HashMap::from_iter([(space, reversed.clone())])).unwrap()
        );
        let LoadedWorkspaces { store: restored, topology } =
            load_store(&serialized.replace(&current, &legacy));
        let restored_windows = OwnedWindows::joining(&topology);
        assert_eq!(
            restored.reading(restored_windows.view()).workspace_ids(space),
            reversed
        );
        assert_eq!(
            restored.reading(restored_windows.view()).catalog().collect::<Vec<_>>(),
            reversed
        );
        assert_eq!(restored[ids[0].0].name, ids[0].1);
    }

    #[test]
    fn prevent_wrapping_updates_on_config_reload() {
        let mut windows = OwnedWindows::default();
        let mut settings = VirtualWorkspaceSettings {
            default_workspace_count: 2,
            ..VirtualWorkspaceSettings::default()
        };
        let mut manager = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());
        let space = SpaceId::new(1);
        let workspaces = manager.listed_after_init(&mut windows.view_mut(), space).to_vec();

        assert_eq!(
            manager.reading(windows.view()).step_workspace(
                space,
                workspaces[1].0,
                None,
                Direction::Right
            ),
            Some(workspaces[0].0)
        );

        settings.resize(2);
        settings.prevent_wrapping = true;
        let plan = manager.reading(windows.view()).plan_settings(&settings).unwrap();
        manager.apply_settings(
            &mut windows.view_mut(),
            &settings,
            &LayoutSettings::default(),
            plan,
        );
        assert_eq!(
            manager.reading(windows.view()).step_workspace(
                space,
                workspaces[1].0,
                None,
                Direction::Right
            ),
            None
        );
    }

    #[test]
    fn app_rules() {
        let mut window_store = OwnedWindows::default();
        let space1 = SpaceId::new(1);
        let space2 = SpaceId::new(2);

        let mut settings = VirtualWorkspaceSettings::default();

        if settings.workspace_names.len() < 4 {
            while settings.workspace_names.len() < 4 {
                settings
                    .workspace_names
                    .push(format!("Workspace {}", settings.workspace_names.len() + 1));
            }
        }
        settings.workspace_names[1] = "coding".to_string();

        settings.app_rules = vec![
            // Floating by app_id
            AppWorkspaceRule {
                app_id: Some("com.example.test".into()),
                floating: true,
                manage: Some(true),
                ..Default::default()
            },
            // Match by app_name -> workspace 1
            AppWorkspaceRule {
                workspace: Some(WorkspaceSelector::Index(1)),
                manage: Some(true),
                app_name: Some("Calendar".into()),
                ..Default::default()
            },
            // Title substring -> workspace 0
            AppWorkspaceRule {
                app_id: Some("com.example.foo".into()),
                workspace: Some(WorkspaceSelector::Index(0)),
                manage: Some(true),
                title_substring: Some("Preferences".into()),
                ..Default::default()
            },
            // Title regex -> workspace 2
            AppWorkspaceRule {
                app_id: Some("com.example.foo".into()),
                workspace: Some(WorkspaceSelector::Index(2)),
                manage: Some(true),
                title_regex: Some(r"Dialog\s+\d+".into()),
                ..Default::default()
            },
            // AX role + subrole floating
            AppWorkspaceRule {
                app_id: Some("com.example.special".into()),
                floating: true,
                manage: Some(true),
                ax_role: Some("AXWindow".into()),
                ax_subrole: Some("AXDialog".into()),
                ..Default::default()
            },
            // Workspace by name
            AppWorkspaceRule {
                app_id: Some("com.example.name".into()),
                workspace: Some(WorkspaceSelector::Name("coding".into())),
                manage: Some(true),
                ..Default::default()
            },
            // A title-specific rule can replace an existing assignment.
            AppWorkspaceRule {
                app_id: Some("app.zen-browser.zen".into()),
                workspace: Some(WorkspaceSelector::Index(2)),
                manage: Some(true),
                ..Default::default()
            },
            AppWorkspaceRule {
                app_id: Some("app.zen-browser.zen".into()),
                workspace: Some(WorkspaceSelector::Index(3)),
                floating: true,
                manage: Some(true),
                title_substring: Some("bitwarden".into()),
                ..Default::default()
            },
        ];

        let mut manager = WorkspaceStore::new_with_config(&settings, &LayoutSettings::default());

        // 1. Floating persistence via app_id (case-insensitive)
        let w_float = WindowId::new(10, 1);
        let assignment = assign(
            &mut manager,
            &mut window_store.view_mut(),
            w_float,
            space1,
            Some("COM.EXAMPLE.Test"),
            None,
            None,
            None,
            None,
        );
        assert!(assignment.floating);

        manager.remove_window(&mut window_store.view_mut(), w_float);

        // After removal, reassign should still float.
        let assignment_again = assign(
            &mut manager,
            &mut window_store.view_mut(),
            w_float,
            space1,
            Some("com.example.test"),
            None,
            None,
            None,
            None,
        );
        assert!(assignment_again.floating);

        // 2. Match by app_name
        let w_name = WindowId::new(20, 2);
        let ws_name = assign(
            &mut manager,
            &mut window_store.view_mut(),
            w_name,
            space1,
            None,
            Some("MyCalendarApp"),
            None,
            None,
            None,
        )
        .workspace_id;
        let coding_idx = 1; // Calendar rule points to workspace index 1
        let expected_ws_name = manager
            .listed_after_init(&mut window_store.view_mut(), space1)
            .get(coding_idx)
            .unwrap()
            .0;
        assert_eq!(ws_name, expected_ws_name);

        // 3. Title substring and regex for same app
        let w_pref = WindowId::new(30, 3);
        let w_dialog = WindowId::new(30, 4);
        let ws_pref = assign(
            &mut manager,
            &mut window_store.view_mut(),
            w_pref,
            space1,
            Some("com.example.foo"),
            None,
            Some("App Preferences"),
            None,
            None,
        )
        .workspace_id;
        let ws_dialog = assign(
            &mut manager,
            &mut window_store.view_mut(),
            w_dialog,
            space1,
            Some("com.example.foo"),
            None,
            Some("Dialog 42"),
            None,
            None,
        )
        .workspace_id;
        let expected_pref = manager
            .listed_after_init(&mut window_store.view_mut(), space1)
            .get(0)
            .unwrap()
            .0;
        let expected_dialog = manager
            .listed_after_init(&mut window_store.view_mut(), space1)
            .get(2)
            .unwrap()
            .0;
        assert_eq!(ws_pref, expected_pref);
        assert_eq!(ws_dialog, expected_dialog);

        // 4. AX role + subrole floating
        let w_ax = WindowId::new(40, 5);
        let ax_assignment = assign(
            &mut manager,
            &mut window_store.view_mut(),
            w_ax,
            space1,
            Some("com.example.special"),
            None,
            None,
            Some("AXWindow"),
            Some("AXDialog"),
        );
        assert!(ax_assignment.floating);

        // 5. Workspace name resolution
        let w_named = WindowId::new(50, 6);
        let ws_named = assign(
            &mut manager,
            &mut window_store.view_mut(),
            w_named,
            space1,
            Some("com.example.name"),
            None,
            None,
            None,
            None,
        )
        .workspace_id;
        let coding_ws = manager
            .listed_after_init(&mut window_store.view_mut(), space1)
            .iter()
            .find(|(_, n)| n == "coding")
            .unwrap()
            .0;
        assert_eq!(ws_named, coding_ws);

        // 6. Reapplication updates both workspace and floating state.
        let w_bw2 = WindowId::new(80, 9);
        let bw2_initial_assignment = assign(
            &mut manager,
            &mut window_store.view_mut(),
            w_bw2,
            space2,
            Some("app.zen-browser.zen"),
            None,
            None,
            None,
            None,
        );
        assert!(!bw2_initial_assignment.floating);
        let bw2_updated_assignment = assign(
            &mut manager,
            &mut window_store.view_mut(),
            w_bw2,
            space2,
            Some("app.zen-browser.zen"),
            None,
            Some("Bitwarden Vault"),
            None,
            None,
        );
        let expected_initial = manager
            .listed_after_init(&mut window_store.view_mut(), space2)
            .get(2)
            .unwrap()
            .0; // workspace index 1
        let expected_updated = manager
            .listed_after_init(&mut window_store.view_mut(), space2)
            .get(3)
            .unwrap()
            .0;
        assert_eq!(bw2_initial_assignment.workspace_id, expected_initial);
        assert_eq!(bw2_updated_assignment.workspace_id, expected_updated);
        assert!(bw2_updated_assignment.floating);
    }

    #[test]
    fn hidden_position_uses_corner_anchor_while_hiding_offscreen() {
        let screen = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(100.0, 100.0));
        let frame = CGRect::new(CGPoint::new(20.0, 37.0), CGSize::new(30.0, 20.0));

        let hidden = HiddenWindowPlacement::calculate(screen, frame, HideCorner::BottomRight, &[]);

        assert_eq!(hidden.origin.y, screen.max().y - 1.0);
        assert_eq!(hidden.origin.x, screen.max().x - 1.0);
    }

    #[test]
    fn hidden_position_flips_sides_to_avoid_neighboring_monitor_overlap() {
        let primary = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(100.0, 100.0));
        let right_neighbor = CGRect::new(CGPoint::new(100.0, 0.0), CGSize::new(100.0, 100.0));
        let frame = CGRect::new(CGPoint::new(20.0, 25.0), CGSize::new(30.0, 20.0));

        let hidden = HiddenWindowPlacement::calculate(primary, frame, HideCorner::BottomRight, &[
            right_neighbor,
        ]);

        assert_eq!(hidden.origin.y, primary.max().y - 1.0);
        assert_eq!(hidden.origin.x, primary.origin.x - frame.size.width + 1.0);
    }
}
