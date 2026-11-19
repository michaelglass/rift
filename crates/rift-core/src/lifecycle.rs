//! Decisions about a window's life (admission, app rules, placement), as pure functions of
//! what the platform observed. The platform asks and acts; these decide, asking for a fact
//! only when the decision needs it.

/// Why app rules run over some windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleRun {
    /// Windows are being admitted: only those whose admission can still be reconciled, and
    /// the layout hears of a window only if a rule changed something for it.
    Admission,
    /// The rules are re-applied (a title changed): every window, and the layout hears of each.
    Reapply,
}

/// The windows app rules run over, by Space in Space order: each window's Space is its
/// assignment's, else its best guess; a window with neither, or on an inactive Space, is left.
pub fn rule_targets<W: Copy, S: Copy + Ord>(
    windows: &[W],
    run: RuleRun,
    reconcilable: impl Fn(W) -> bool,
    space: impl Fn(W) -> Option<S>,
    active: impl Fn(S) -> bool,
) -> Vec<(S, Vec<W>)> {
    let mut by_space: std::collections::BTreeMap<S, Vec<W>> = Default::default();
    for &window in windows {
        let considered = match run {
            RuleRun::Reapply => true,
            RuleRun::Admission => reconcilable(window),
        };
        if !considered {
            continue;
        }
        if let Some(space) = space(window) {
            by_space.entry(space).or_default().push(window);
        }
    }
    by_space.into_iter().filter(|(space, _)| active(*space)).collect()
}

/// A window before and after its app rules ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleOutcome<K> {
    pub workspace_before: Option<K>,
    pub workspace: K,
    pub floating_before: bool,
    pub floating: bool,
    /// A rule had told rift to leave it alone.
    pub was_ignored: bool,
    /// It is temporarily unavailable (hidden, minimized, on its way somewhere).
    pub unavailable: bool,
}

/// Whether the layout hears of a window after its app rules ran: always on a re-apply;
/// otherwise when the rules moved it, floated or tiled it, or it had been ignored or was away.
pub fn rule_refreshes_layout<K: PartialEq>(run: RuleRun, outcome: &RuleOutcome<K>) -> bool {
    match run {
        RuleRun::Reapply => true,
        RuleRun::Admission => {
            outcome.unavailable
                || outcome.workspace_before.as_ref() != Some(&outcome.workspace)
                || outcome.floating_before != outcome.floating
                || outcome.was_ignored
        }
    }
}

/// How workspaces map onto displays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceMode {
    /// Each display's desktops have their own workspaces.
    PerDisplay,
    /// One set of workspaces across displays.
    Global,
}

/// A window that just opened, as placement sees it; a standard one with what the app rules
/// read of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewWindow<T> {
    /// The platform has no state for it.
    Untracked,
    /// Not a standard window (a panel, a sheet, ...).
    NotStandard,
    Standard(T),
}

/// The Space a new window moves to, opened on `opened_on`: in global mode, the pointer's
/// display's, if that is another active one, unless an app rule names the window's workspace
/// (launchers can activate an app on another display). The pointer is asked only in global
/// mode, the rules only for a standard window that would move.
pub fn pointer_display_target<S: Copy + PartialEq, T>(
    mode: WorkspaceMode,
    opened_on: S,
    pointer_space: impl FnOnce() -> Option<S>,
    active: impl Fn(S) -> bool,
    window: NewWindow<T>,
    rule_names_workspace: impl FnOnce(T) -> bool,
) -> Option<S> {
    match mode {
        WorkspaceMode::PerDisplay => None,
        WorkspaceMode::Global => {
            let target = pointer_space().filter(|&space| space != opened_on && active(space))?;
            match window {
                NewWindow::Untracked | NewWindow::NotStandard => None,
                NewWindow::Standard(window) => (!rule_names_workspace(window)).then_some(target),
            }
        }
    }
}

/// Where the evidence that a window is transient (a popover, a sheet, a panel) came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransientOrigin {
    /// The window server's own tags: final.
    Native,
    /// The accessibility heuristic, which a rule can override.
    Heuristic,
}

/// Whether `owner`'s window is a native transient, from its transient entry (none for a window
/// with no window-server id or no entry): native evidence for the same app, unless the window
/// is exempt (asked only then).
pub fn is_native_transient<P: PartialEq>(
    entry: Option<(TransientOrigin, P)>,
    owner: P,
    exempt: impl FnOnce() -> bool,
) -> bool {
    matches!(entry, Some((TransientOrigin::Native, pid)) if pid == owner) && !exempt()
}

/// A window lifecycle event, as the transient entries hear of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Departure<Id, P> {
    /// An AX window was destroyed, with its window-server id if it had one: its heuristic
    /// entry goes; native evidence stays until the window server says so.
    Destroyed(Option<Id>),
    /// The window server closed the window.
    Closed(Id),
    /// The app is gone, and every entry of its windows.
    AppGone(P),
}

/// Whether the transient entry for window `id`, of `owner` and from `origin`, survives
/// `departure`.
pub fn keeps_transient<Id: PartialEq, P: PartialEq>(
    departure: Departure<Id, P>,
    id: Id,
    origin: TransientOrigin,
    owner: P,
) -> bool {
    match departure {
        Departure::Destroyed(gone) => origin == TransientOrigin::Native || gone != Some(id),
        Departure::Closed(gone) => gone != id,
        Departure::AppGone(pid) => pid != owner,
    }
}

/// Whether a window's assignment and the Space macOS has it on disagree (both known).
pub fn misplaced<S: PartialEq>(assigned: Option<S>, native: Option<S>) -> bool {
    matches!((assigned, native), (Some(assigned), Some(native)) if assigned != native)
}

/// Whether an app inventory that omits this window says nothing about it: it is on an inactive
/// Space (its native Space, else its assignment's), or parked just offscreen on a non-active
/// workspace. Such a window is not retired for being missing.
pub fn out_of_inventory_view<S: Copy>(
    native: Option<S>,
    assigned: impl FnOnce() -> Option<S>,
    active: impl Fn(S) -> bool,
    parked: impl FnOnce() -> bool,
) -> bool {
    native.or_else(assigned).is_some_and(|space| !active(space)) || parked()
}

/// What a window's title change does to its app rules, as configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnTitleChange {
    Reapply,
    Ignore,
}

/// Whether a title change re-applies a window's app rules: if configured to, for a window on
/// an active Space (its best-known one, asked only then) whose admission can still be
/// reconciled (asked last).
pub fn reapplies_on_title_change<S: Copy>(
    configured: OnTitleChange,
    space: impl FnOnce() -> Option<S>,
    active: impl Fn(S) -> bool,
    reconcilable: impl FnOnce() -> bool,
) -> bool {
    match configured {
        OnTitleChange::Ignore => false,
        OnTitleChange::Reapply => space().is_some_and(active) && reconcilable(),
    }
}

/// The platform's manageability heuristic, as it judged the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manageability {
    Manageable,
    Unmanageable,
}

/// What an app rule says about managing the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    NoRule,
    Manage,
    Ignore,
}

/// What the accessibility roles say of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxKind {
    Ordinary,
    /// An AXWindow whose only transient marker is its AXDialog subrole: some apps use it for a
    /// real main window, so a Manage rule admits it.
    DialogWindow,
    /// Any other transient (a non-window role, a sheet, a system dialog).
    Transient,
}

impl AxKind {
    /// From the platform's two readings: whether the roles mark the window transient, and
    /// whether it is an AXWindow with the AXDialog subrole.
    pub fn classify(ax_transient: bool, dialog_window: bool) -> AxKind {
        match (ax_transient, dialog_window) {
            (false, _) => AxKind::Ordinary,
            (true, true) => AxKind::DialogWindow,
            (true, false) => AxKind::Transient,
        }
    }
}

/// Whether the window server marks the window transient (for its own app).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeKind {
    Ordinary,
    Transient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Minimize {
    Shown,
    Minimized,
}

/// What admission reads of a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmissionFacts {
    pub manageability: Manageability,
    pub rule: Rule,
    pub ax: AxKind,
    pub native: NativeKind,
    pub minimize: Minimize,
}

/// Why a window is not admitted, the first that applies in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Veto {
    NativeTransient,
    Minimized,
    AxTransient,
    RuleIgnores,
    Unmanageable,
}

/// Whether rift manages a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Admitted,
    Vetoed(Veto),
}

/// A window is admitted unless the window server marks it transient, it is minimized, its
/// roles mark it transient (a Manage rule outranks only the AXDialog marker), a rule ignores
/// it, or, with no rule, the heuristic finds it unmanageable.
pub fn admission(facts: AdmissionFacts) -> Admission {
    let veto = match (facts.native, facts.minimize, facts.ax, facts.rule) {
        (NativeKind::Transient, ..) => Some(Veto::NativeTransient),
        (_, Minimize::Minimized, ..) => Some(Veto::Minimized),
        (_, _, AxKind::Transient, _) => Some(Veto::AxTransient),
        (_, _, AxKind::DialogWindow, Rule::NoRule | Rule::Ignore) => Some(Veto::AxTransient),
        (_, _, _, Rule::Ignore) => Some(Veto::RuleIgnores),
        (_, _, _, Rule::Manage) => None,
        (_, _, _, Rule::NoRule) => match facts.manageability {
            Manageability::Manageable => None,
            Manageability::Unmanageable => Some(Veto::Unmanageable),
        },
    };
    match veto {
        None => Admission::Admitted,
        Some(why) => Admission::Vetoed(why),
    }
}

/// One admission fact, changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactChange {
    Manageability(Manageability),
    Rule(Rule),
    Ax(AxKind),
    Native(NativeKind),
    Minimize(Minimize),
}

/// What the platform tells the lifecycle about a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleEvent<W> {
    /// The window has AX state, read afresh: these are its facts now.
    Live { window: W, facts: AdmissionFacts },
    /// One of a live window's facts changed. A window with no AX state has no facts to change.
    Changed { window: W, change: FactChange },
    /// The window's AX state is gone (destroyed, retired, replaced by a new identity).
    Gone(W),
}

/// A change in whether rift manages a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition<W> {
    Admitted(W),
    Suspended(W, Veto),
}

/// Every window with AX state: its admission facts, and whether that state is still valid.
/// Admission is not stored: it is read off the facts, so it can never disagree with them.
#[derive(Debug, Clone)]
pub struct Lifecycle<W> {
    windows: std::collections::BTreeMap<W, Presence>,
}

/// A window with AX state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Live(AdmissionFacts),
    /// Its AX element was invalidated (displays and sessions change under AX): rift keeps the
    /// window, its facts and its place, but does not manage it until fresh AX evidence.
    Invalidated(AdmissionFacts),
}

/// What happens to a window's presence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceEvent {
    /// A reading of its facts (see [`read`]).
    Reading(Reading),
    /// Its AX element was invalidated.
    Invalidate,
    /// A positive AX observation.
    Revalidate,
}

/// What a presence event came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceOutcome {
    /// The change in its admission by its facts.
    Read(Option<Change>),
    /// Whether there was AX state to invalidate.
    Invalidated(bool),
    /// Whether it had been invalidated.
    Revalidated(bool),
}

/// A window's presence after `event` (none without AX state), and what the event came to.
/// A fresh reading is valid AX state; a change keeps the presence; an invalidation needs AX
/// state; a revalidation brings an invalidated window back.
pub fn present(
    before: Option<Presence>,
    event: PresenceEvent,
) -> (Option<Presence>, PresenceOutcome) {
    match event {
        PresenceEvent::Reading(reading) => {
            let (after, change) = read(before.map(Presence::facts), reading);
            let presence = match (after, reading, before) {
                (Some(facts), Reading::Live(_), _) | (Some(facts), _, None) => {
                    Some(Presence::Live(facts))
                }
                (Some(facts), _, Some(presence)) => Some(presence.with(facts)),
                (None, ..) => None,
            };
            (presence, PresenceOutcome::Read(change))
        }
        PresenceEvent::Invalidate => match before {
            Some(presence) => (
                Some(Presence::Invalidated(presence.facts())),
                PresenceOutcome::Invalidated(true),
            ),
            None => (None, PresenceOutcome::Invalidated(false)),
        },
        PresenceEvent::Revalidate => match before {
            Some(Presence::Invalidated(facts)) => {
                (Some(Presence::Live(facts)), PresenceOutcome::Revalidated(true))
            }
            live_or_none => (live_or_none, PresenceOutcome::Revalidated(false)),
        },
    }
}

/// Whether rift manages a window by its presence: valid AX state and admitted.
pub fn manages(presence: Option<Presence>) -> bool {
    matches!(presence, Some(Presence::Live(facts)) if admission(facts) == Admission::Admitted)
}

impl Presence {
    fn facts(self) -> AdmissionFacts {
        match self {
            Presence::Live(facts) | Presence::Invalidated(facts) => facts,
        }
    }

    /// The same presence with new facts.
    fn with(self, facts: AdmissionFacts) -> Presence {
        match self {
            Presence::Live(_) => Presence::Live(facts),
            Presence::Invalidated(_) => Presence::Invalidated(facts),
        }
    }
}

impl<W> Default for Lifecycle<W> {
    fn default() -> Self { Lifecycle { windows: Default::default() } }
}

/// What the platform read of one window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    Live(AdmissionFacts),
    Changed(FactChange),
    Gone,
}

/// A change in whether rift manages one window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Admitted,
    Suspended(Veto),
}

/// A window's facts after `reading` (none without AX state), and the change in its admission.
/// Only a change between admitted and vetoed is one: a new window that is admitted counts, a
/// window going away does not, and a change to a window with no AX state is ignored.
pub fn read(
    before: Option<AdmissionFacts>,
    reading: Reading,
) -> (Option<AdmissionFacts>, Option<Change>) {
    let after = match (before, reading) {
        (_, Reading::Live(facts)) => facts,
        (None, Reading::Changed(_)) | (_, Reading::Gone) => return (None, None),
        (Some(mut facts), Reading::Changed(change)) => {
            match change {
                FactChange::Manageability(manageability) => facts.manageability = manageability,
                FactChange::Rule(rule) => facts.rule = rule,
                FactChange::Ax(ax) => facts.ax = ax,
                FactChange::Native(native) => facts.native = native,
                FactChange::Minimize(minimize) => facts.minimize = minimize,
            }
            facts
        }
    };
    let change = match (before.map(admission), admission(after)) {
        (Some(Admission::Admitted), Admission::Vetoed(why)) => Some(Change::Suspended(why)),
        (None | Some(Admission::Vetoed(_)), Admission::Admitted) => Some(Change::Admitted),
        (Some(Admission::Admitted), Admission::Admitted)
        | (None | Some(Admission::Vetoed(_)), Admission::Vetoed(_)) => None,
    };
    (Some(after), change)
}

impl<W: Copy + Ord> Lifecycle<W> {
    /// Applies `event`; the transition it makes, if any (see [`read`]).
    pub fn step(&mut self, event: LifecycleEvent<W>) -> Option<Transition<W>> {
        let (window, reading) = match event {
            LifecycleEvent::Live { window, facts } => (window, Reading::Live(facts)),
            LifecycleEvent::Changed { window, change } => (window, Reading::Changed(change)),
            LifecycleEvent::Gone(window) => (window, Reading::Gone),
        };
        let change = match self.apply(window, PresenceEvent::Reading(reading)) {
            PresenceOutcome::Read(change) => change,
            PresenceOutcome::Invalidated(_) | PresenceOutcome::Revalidated(_) => None,
        };
        change.map(|change| match change {
            Change::Admitted => Transition::Admitted(window),
            Change::Suspended(why) => Transition::Suspended(window, why),
        })
    }

    /// A window's admission by its facts, valid AX state or not; `None` for a window with no
    /// AX state.
    pub fn admission(&self, window: W) -> Option<Admission> { self.facts(window).map(admission) }

    /// A window's facts; `None` for a window with no AX state.
    pub fn facts(&self, window: W) -> Option<AdmissionFacts> {
        self.windows.get(&window).copied().map(Presence::facts)
    }

    fn apply(&mut self, window: W, event: PresenceEvent) -> PresenceOutcome {
        let (after, outcome) = present(self.windows.get(&window).copied(), event);
        match after {
            Some(presence) => self.windows.insert(window, presence),
            None => self.windows.remove(&window),
        };
        outcome
    }

    /// Whether rift manages the window by its lifecycle: valid AX state and admitted.
    pub fn manages(&self, window: W) -> bool { manages(self.windows.get(&window).copied()) }

    /// The window's AX element was invalidated; false for a window with no AX state.
    pub fn invalidate(&mut self, window: W) -> bool {
        self.apply(window, PresenceEvent::Invalidate) == PresenceOutcome::Invalidated(true)
    }

    /// A positive AX observation: whether the window had been invalidated.
    pub fn revalidate(&mut self, window: W) -> bool {
        self.apply(window, PresenceEvent::Revalidate) == PresenceOutcome::Revalidated(true)
    }

    pub fn is_invalidated(&self, window: W) -> bool {
        matches!(self.windows.get(&window), Some(Presence::Invalidated(_)))
    }

    /// `from`'s identity moved to `to`: an invalidation moves with it, to a window with AX
    /// state.
    pub fn carry_invalidation(&mut self, from: W, to: W) {
        if self.revalidate(from) {
            self.invalidate(to);
        }
    }

    /// The live windows.
    pub fn windows(&self) -> impl Iterator<Item = W> + '_ { self.windows.keys().copied() }
}

/// Whether an app inventory reported a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Report {
    Reported,
    Omitted,
}

/// Whether an inventory can see the window at all (see [`out_of_inventory_view`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryView {
    InView,
    OutOfView,
}

/// Whether the window has a native fullscreen record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullscreenRecord {
    Recorded,
    NotRecorded,
}

/// The window's window-server id, if known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeIdentity<V> {
    Known(V),
    Unknown,
}

/// Whether the window's AX element is still valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxValidity {
    Valid,
    Invalidated,
}

/// What the platform knows of a window an inventory is reconciled against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InventoryFacts<V> {
    pub report: Report,
    pub view: InventoryView,
    /// `None` without AX state.
    pub minimize: Option<Minimize>,
    pub fullscreen: FullscreenRecord,
    pub identity: NativeIdentity<V>,
    pub validity: AxValidity,
}

/// The window server's type for a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suitability {
    Suitable,
    Unsuitable,
}

/// Whether the window server has the window ordered in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ordering {
    OrderedIn,
    OrderedOut,
}

/// What the window server's record of a window looks like: a window, or not one (a nonzero
/// layer, or a frame under two points on an axis).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeLook {
    Window,
    NotAWindow,
}

/// What the window server says of an omitted window; each `None` is unknown, which carries no
/// negative authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeEvidence {
    pub suitability: Option<Suitability>,
    pub ordering: Option<Ordering>,
    pub look: Option<NativeLook>,
}

/// Why an inventory leaves a window alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    Reported,
    OutOfView,
    NoAxState,
    Minimized,
    Fullscreen,
    NoNativeIdentity,
    /// Ordered out while its AX identity is valid: native ordering explains the omission.
    /// After an invalidation, omission plus ordering out confirms retirement.
    NativeOrdering,
    /// Nothing native says it is gone.
    NoNegativeEvidence,
}

/// Whether an inventory retires a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retirement {
    Retire,
    Keep(Keep),
}

/// A current, successful inventory omission retires a window only with native negative
/// evidence: unsuitable, ordered out, or not a window. The evidence is asked only for an
/// omitted window in view with AX state, shown, not in fullscreen and with a native identity.
pub fn retirement<V: Copy>(
    facts: InventoryFacts<V>,
    evidence: impl FnOnce(V) -> NativeEvidence,
) -> Retirement {
    let identity = match facts {
        InventoryFacts { report: Report::Reported, .. } => Err(Keep::Reported),
        InventoryFacts {
            view: InventoryView::OutOfView, ..
        } => Err(Keep::OutOfView),
        InventoryFacts { minimize: None, .. } => Err(Keep::NoAxState),
        InventoryFacts {
            minimize: Some(Minimize::Minimized),
            ..
        } => Err(Keep::Minimized),
        InventoryFacts {
            fullscreen: FullscreenRecord::Recorded,
            ..
        } => Err(Keep::Fullscreen),
        InventoryFacts {
            identity: NativeIdentity::Unknown,
            ..
        } => Err(Keep::NoNativeIdentity),
        InventoryFacts {
            identity: NativeIdentity::Known(identity),
            ..
        } => Ok(identity),
    };
    let evidence = match identity {
        Ok(identity) => evidence(identity),
        Err(why) => return Retirement::Keep(why),
    };
    match (evidence, facts.validity) {
        (
            NativeEvidence {
                suitability: Some(Suitability::Suitable),
                ordering: Some(Ordering::OrderedOut),
                ..
            },
            AxValidity::Valid,
        ) => Retirement::Keep(Keep::NativeOrdering),
        (
            NativeEvidence {
                suitability: Some(Suitability::Unsuitable),
                ..
            }
            | NativeEvidence {
                ordering: Some(Ordering::OrderedOut),
                ..
            }
            | NativeEvidence {
                look: Some(NativeLook::NotAWindow),
                ..
            },
            _,
        ) => Retirement::Retire,
        (_, _) => Retirement::Keep(Keep::NoNegativeEvidence),
    }
}

/// Each decision as a question on plain data: every fact it may ask for is in the question.
/// Windows, Spaces, window-server ids and apps are one word each.
pub mod plain {
    use super::{
        Departure, NewWindow, OnTitleChange, RuleOutcome, RuleRun, TransientOrigin, WorkspaceMode,
    };

    fn lookup(pairs: &[(u64, u64)], key: u64) -> Option<u64> {
        pairs.iter().find(|(k, _)| *k == key).map(|(_, value)| *value)
    }

    /// [`super::rule_targets`]: each window's Space, where it has one.
    #[derive(Debug, Clone, PartialEq)]
    pub struct RuleTargets {
        pub windows: Vec<u64>,
        pub run: RuleRun,
        pub reconcilable: Vec<u64>,
        pub spaces: Vec<(u64, u64)>,
        pub active: Vec<u64>,
    }

    impl RuleTargets {
        pub fn answer(&self) -> Vec<(u64, Vec<u64>)> {
            super::rule_targets(
                &self.windows,
                self.run,
                |window| self.reconcilable.contains(&window),
                |window| lookup(&self.spaces, window),
                |space| self.active.contains(&space),
            )
        }
    }

    /// [`super::rule_refreshes_layout`].
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct RuleRefresh {
        pub run: RuleRun,
        pub outcome: RuleOutcome<u64>,
    }

    impl RuleRefresh {
        pub fn answer(&self) -> bool { super::rule_refreshes_layout(self.run, &self.outcome) }
    }

    /// [`super::pointer_display_target`]: what the rules say of a standard window.
    #[derive(Debug, Clone, PartialEq)]
    pub struct PointerTarget {
        pub mode: WorkspaceMode,
        pub opened_on: u64,
        pub pointer: Option<u64>,
        pub active: Vec<u64>,
        pub window: NewWindow<()>,
        pub rule_names_workspace: bool,
    }

    impl PointerTarget {
        pub fn answer(&self) -> Option<u64> {
            super::pointer_display_target(
                self.mode,
                self.opened_on,
                || self.pointer,
                |space| self.active.contains(&space),
                self.window,
                |()| self.rule_names_workspace,
            )
        }
    }

    /// [`super::is_native_transient`].
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct NativeTransient {
        pub entry: Option<(TransientOrigin, u64)>,
        pub owner: u64,
        pub exempt: bool,
    }

    impl NativeTransient {
        pub fn answer(&self) -> bool {
            super::is_native_transient(self.entry, self.owner, || self.exempt)
        }
    }

    /// [`super::keeps_transient`].
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct KeepsTransient {
        pub departure: Departure<u64, u64>,
        pub id: u64,
        pub origin: TransientOrigin,
        pub owner: u64,
    }

    impl KeepsTransient {
        pub fn answer(&self) -> bool {
            super::keeps_transient(self.departure, self.id, self.origin, self.owner)
        }
    }

    /// [`super::misplaced`].
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Misplaced {
        pub assigned: Option<u64>,
        pub native: Option<u64>,
    }

    impl Misplaced {
        pub fn answer(&self) -> bool { super::misplaced(self.assigned, self.native) }
    }

    /// [`super::out_of_inventory_view`].
    #[derive(Debug, Clone, PartialEq)]
    pub struct OutOfView {
        pub native: Option<u64>,
        pub assigned: Option<u64>,
        pub active: Vec<u64>,
        pub parked: bool,
    }

    impl OutOfView {
        pub fn answer(&self) -> bool {
            super::out_of_inventory_view(
                self.native,
                || self.assigned,
                |space| self.active.contains(&space),
                || self.parked,
            )
        }
    }

    /// [`super::reapplies_on_title_change`].
    #[derive(Debug, Clone, PartialEq)]
    pub struct TitleChange {
        pub configured: OnTitleChange,
        pub space: Option<u64>,
        pub active: Vec<u64>,
        pub reconcilable: bool,
    }

    impl TitleChange {
        pub fn answer(&self) -> bool {
            super::reapplies_on_title_change(
                self.configured,
                || self.space,
                |space| self.active.contains(&space),
                || self.reconcilable,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_rules_run_over_the_windows_on_active_spaces_by_space() {
        // Window n is on Space n % 3 (Space 0: none); Space 2 is inactive; even windows are
        // reconcilable.
        let windows = [1_u32, 2, 3, 4, 5, 6, 7];
        let space = |window: u32| (window % 3 != 0).then_some(window % 3);
        let active = |space: u32| space != 2;
        let reconcilable = |window: u32| window % 2 == 0;
        assert_eq!(
            rule_targets(&windows, RuleRun::Reapply, reconcilable, space, active),
            vec![(1, vec![1, 4, 7])]
        );
        assert_eq!(
            rule_targets(&windows, RuleRun::Admission, reconcilable, space, active),
            vec![(1, vec![4])]
        );
        let all = |_| true;
        assert_eq!(
            rule_targets(&windows, RuleRun::Reapply, reconcilable, space, all),
            vec![(1, vec![1, 4, 7]), (2, vec![2, 5])]
        );
        // An unconsidered window's Space is not asked.
        let asked = std::cell::Cell::new(0);
        let counted = |window: u32| {
            asked.set(asked.get() + 1);
            space(window)
        };
        rule_targets(&windows, RuleRun::Admission, reconcilable, counted, active);
        assert_eq!(asked.get(), 3);
    }

    #[test]
    fn a_new_window_moves_to_the_pointers_other_active_display_in_global_mode() {
        use NewWindow::{NotStandard, Untracked};
        let standard = NewWindow::Standard(());
        use WorkspaceMode::{Global, PerDisplay};
        let (opened_on, pointer, inactive) = (1_u64, 2, 3);
        let active = |space| space != inactive;
        let no_rule = |()| false;
        let place = |pointer_space: Option<u64>, window: NewWindow<()>| {
            pointer_display_target(Global, opened_on, || pointer_space, active, window, no_rule)
        };
        assert_eq!(place(Some(pointer), standard), Some(pointer));
        // Already there, an inactive or unknown display, or not a standard window: it stays.
        assert_eq!(place(Some(opened_on), standard), None);
        assert_eq!(place(Some(inactive), standard), None);
        assert_eq!(place(None, standard), None);
        assert_eq!(place(Some(pointer), NotStandard), None);
        assert_eq!(place(Some(pointer), Untracked), None);
        // An app rule naming its workspace keeps it where the rule puts it.
        assert_eq!(
            pointer_display_target(
                Global,
                opened_on,
                || Some(pointer),
                active,
                standard,
                |()| true
            ),
            None
        );
        // Per display: the pointer is not asked.
        assert_eq!(
            pointer_display_target(
                PerDisplay,
                opened_on,
                || panic!("the pointer is not asked"),
                active,
                standard,
                |()| panic!("nor the rules"),
            ),
            None
        );
        // The rules are asked only for a standard window that would move.
        assert_eq!(
            pointer_display_target(
                Global,
                opened_on,
                || Some(opened_on),
                active,
                standard,
                |()| panic!("the rules are not asked")
            ),
            None
        );
    }

    #[test]
    fn a_window_is_a_native_transient_on_native_evidence_for_its_own_app_unless_exempt() {
        use TransientOrigin::{Heuristic, Native};
        let not_exempt = || false;
        assert!(is_native_transient(Some((Native, 7)), 7, not_exempt));
        assert!(!is_native_transient(Some((Native, 8)), 7, || panic!(
            "not asked"
        )));
        assert!(!is_native_transient(Some((Heuristic, 7)), 7, || panic!(
            "not asked"
        )));
        assert!(!is_native_transient(None, 7, || panic!("not asked")));
        assert!(!is_native_transient(Some((Native, 7)), 7, || true));
    }

    #[test]
    fn a_transient_entry_goes_with_its_window_or_its_app() {
        use Departure::{AppGone, Closed, Destroyed};
        use TransientOrigin::{Heuristic, Native};
        // An AX destruction drops the heuristic entry for that id only.
        assert!(!keeps_transient(Destroyed(Some(1)), 1, Heuristic, 7));
        assert!(keeps_transient(Destroyed(Some(1)), 1, Native, 7));
        assert!(keeps_transient(Destroyed(Some(1)), 2, Heuristic, 7));
        assert!(keeps_transient(Destroyed::<u32, i32>(None), 1, Heuristic, 7));
        // A window-server close drops its entry, native or not.
        assert!(!keeps_transient(Closed::<u32, i32>(1), 1, Native, 7));
        assert!(keeps_transient(Closed::<u32, i32>(1), 2, Native, 7));
        // An app's end drops every entry of its windows.
        assert!(!keeps_transient(AppGone::<u32, i32>(7), 1, Native, 7));
        assert!(keeps_transient(AppGone::<u32, i32>(8), 1, Native, 7));
    }

    #[test]
    fn a_window_is_misplaced_when_its_assignment_and_its_native_space_disagree() {
        assert!(misplaced(Some(1), Some(2)));
        assert!(!misplaced(Some(1), Some(1)));
        assert!(!misplaced(None, Some(1)));
        assert!(!misplaced(Some(1), None::<u64>));
    }

    #[test]
    fn an_inventory_cannot_see_a_window_on_an_inactive_space_or_parked_offscreen() {
        let active = |space: u64| space == 1;
        let unparked = || false;
        // Its native Space decides, else its assignment.
        assert!(out_of_inventory_view(
            Some(2),
            || panic!("not asked"),
            active,
            unparked
        ));
        assert!(!out_of_inventory_view(Some(1), || Some(2), active, unparked));
        assert!(out_of_inventory_view(None, || Some(2), active, unparked));
        assert!(!out_of_inventory_view(None, || None, active, unparked));
        // Parked offscreen on a non-active workspace, wherever it is.
        assert!(out_of_inventory_view(Some(1), || None, active, || true));
        assert!(out_of_inventory_view(None, || None, active, || true));
    }

    #[test]
    fn a_title_change_reapplies_app_rules_to_a_reconcilable_window_on_an_active_space() {
        use OnTitleChange::{Ignore, Reapply};
        let active = |space: u64| space == 1;
        assert!(reapplies_on_title_change(Reapply, || Some(1), active, || true));
        assert!(!reapplies_on_title_change(
            Reapply,
            || Some(2),
            active,
            || panic!("not asked")
        ));
        assert!(!reapplies_on_title_change(
            Reapply,
            || None,
            active,
            || panic!("not asked")
        ));
        assert!(!reapplies_on_title_change(Reapply, || Some(1), active, || false));
        assert!(!reapplies_on_title_change(
            Ignore,
            || panic!("the Space is not asked"),
            active,
            || panic!("nor the window"),
        ));
    }

    #[test]
    fn admission_is_todays_boolean_rule_over_every_combination_of_facts() {
        // The rule as rift wrote it on loose facts: a Manage rule outranks the AX veto only for
        // an AXWindow/AXDialog; native transients and minimized windows stay vetoed.
        let reference = |manageable: bool,
                         rule: Option<bool>,
                         ax_transient: bool,
                         dialog_window: bool,
                         native_transient: bool,
                         minimized: bool| {
            let overrides_dialog_veto = rule == Some(true) && dialog_window;
            !native_transient
                && !minimized
                && (overrides_dialog_veto || !ax_transient)
                && rule.unwrap_or(manageable)
        };
        let mut checked = 0;
        for manageable in [false, true] {
            for rule in [None, Some(false), Some(true)] {
                for (ax_transient, dialog_window) in
                    [(false, false), (false, true), (true, false), (true, true)]
                {
                    for native_transient in [false, true] {
                        for minimized in [false, true] {
                            let facts = AdmissionFacts {
                                manageability: if manageable {
                                    Manageability::Manageable
                                } else {
                                    Manageability::Unmanageable
                                },
                                rule: match rule {
                                    None => Rule::NoRule,
                                    Some(true) => Rule::Manage,
                                    Some(false) => Rule::Ignore,
                                },
                                ax: AxKind::classify(ax_transient, dialog_window),
                                native: if native_transient {
                                    NativeKind::Transient
                                } else {
                                    NativeKind::Ordinary
                                },
                                minimize: if minimized {
                                    Minimize::Minimized
                                } else {
                                    Minimize::Shown
                                },
                            };
                            let expected = reference(
                                manageable,
                                rule,
                                ax_transient,
                                dialog_window,
                                native_transient,
                                minimized,
                            );
                            assert_eq!(
                                admission(facts) == Admission::Admitted,
                                expected,
                                "{facts:?}"
                            );
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(checked, 96);
        // The first veto that applies is the one given.
        let vetoed = |facts| match admission(facts) {
            Admission::Vetoed(why) => why,
            Admission::Admitted => panic!("admitted: {facts:?}"),
        };
        let worst = AdmissionFacts {
            manageability: Manageability::Unmanageable,
            rule: Rule::Ignore,
            ax: AxKind::Transient,
            native: NativeKind::Transient,
            minimize: Minimize::Minimized,
        };
        assert_eq!(vetoed(worst), Veto::NativeTransient);
        assert_eq!(
            vetoed(AdmissionFacts {
                native: NativeKind::Ordinary,
                ..worst
            }),
            Veto::Minimized
        );
        let shown = AdmissionFacts {
            native: NativeKind::Ordinary,
            minimize: Minimize::Shown,
            ..worst
        };
        assert_eq!(vetoed(shown), Veto::AxTransient);
        assert_eq!(
            vetoed(AdmissionFacts { ax: AxKind::Ordinary, ..shown }),
            Veto::RuleIgnores
        );
        assert_eq!(
            vetoed(AdmissionFacts {
                ax: AxKind::Ordinary,
                rule: Rule::NoRule,
                ..shown
            }),
            Veto::Unmanageable
        );
    }

    #[test]
    fn the_lifecycle_tracks_live_windows_facts_and_reports_admission_transitions() {
        let facts = AdmissionFacts {
            manageability: Manageability::Manageable,
            rule: Rule::NoRule,
            ax: AxKind::Ordinary,
            native: NativeKind::Ordinary,
            minimize: Minimize::Shown,
        };
        let mut lifecycle = Lifecycle::default();
        assert_eq!(lifecycle.admission(1_u32), None);
        assert_eq!(
            lifecycle.step(LifecycleEvent::Live { window: 1, facts }),
            Some(Transition::Admitted(1))
        );
        assert_eq!(lifecycle.admission(1), Some(Admission::Admitted));
        // A change that keeps it admitted is no transition.
        assert_eq!(
            lifecycle.step(LifecycleEvent::Changed {
                window: 1,
                change: FactChange::Rule(Rule::Manage)
            }),
            None
        );
        assert_eq!(
            lifecycle.step(LifecycleEvent::Changed {
                window: 1,
                change: FactChange::Minimize(Minimize::Minimized)
            }),
            Some(Transition::Suspended(1, Veto::Minimized))
        );
        // Vetoed to vetoed for another reason: no transition.
        assert_eq!(
            lifecycle.step(LifecycleEvent::Changed {
                window: 1,
                change: FactChange::Native(NativeKind::Transient)
            }),
            None
        );
        assert_eq!(
            lifecycle.admission(1),
            Some(Admission::Vetoed(Veto::NativeTransient))
        );
        assert_eq!(
            lifecycle.step(LifecycleEvent::Changed {
                window: 1,
                change: FactChange::Native(NativeKind::Ordinary)
            }),
            None
        );
        assert_eq!(
            lifecycle.step(LifecycleEvent::Changed {
                window: 1,
                change: FactChange::Minimize(Minimize::Shown)
            }),
            Some(Transition::Admitted(1))
        );
        // A new live reading replaces the facts, the earlier Manage rule with them.
        let unmanageable = AdmissionFacts {
            manageability: Manageability::Unmanageable,
            ..facts
        };
        assert_eq!(
            lifecycle.step(LifecycleEvent::Live { window: 1, facts: unmanageable }),
            Some(Transition::Suspended(1, Veto::Unmanageable))
        );
        let _ = lifecycle.step(LifecycleEvent::Changed {
            window: 1,
            change: FactChange::Ax(AxKind::Transient),
        });
        assert_eq!(
            lifecycle.admission(1),
            Some(Admission::Vetoed(Veto::AxTransient))
        );
        let _ = lifecycle.step(LifecycleEvent::Changed {
            window: 1,
            change: FactChange::Manageability(Manageability::Manageable),
        });
        // Gone: no facts, no transition; a change for a window with no AX state is ignored.
        assert_eq!(lifecycle.step(LifecycleEvent::Gone(1)), None);
        assert_eq!(lifecycle.admission(1), None);
        assert_eq!(
            lifecycle.step(LifecycleEvent::Changed {
                window: 1,
                change: FactChange::Rule(Rule::Manage)
            }),
            None
        );
        assert_eq!(lifecycle.admission(1), None);
        assert_eq!(lifecycle.windows().count(), 0);
    }

    #[test]
    fn an_invalidated_window_keeps_its_facts_and_comes_back_on_fresh_ax() {
        let facts = AdmissionFacts {
            manageability: Manageability::Manageable,
            rule: Rule::NoRule,
            ax: AxKind::Ordinary,
            native: NativeKind::Ordinary,
            minimize: Minimize::Shown,
        };
        let mut lifecycle = Lifecycle::default();
        // Only a window with AX state can lose it.
        assert!(!lifecycle.invalidate(1_u32));
        assert!(!lifecycle.is_invalidated(1));
        lifecycle.step(LifecycleEvent::Live { window: 1, facts });
        assert!(lifecycle.manages(1));
        assert!(lifecycle.invalidate(1));
        assert!(lifecycle.is_invalidated(1) && !lifecycle.manages(1));
        // Its facts stay, and still change, while it is invalidated.
        assert_eq!(lifecycle.admission(1), Some(Admission::Admitted));
        lifecycle.step(LifecycleEvent::Changed {
            window: 1,
            change: FactChange::Rule(Rule::Manage),
        });
        assert_eq!(lifecycle.facts(1).map(|f| f.rule), Some(Rule::Manage));
        // A positive AX observation brings it back, once.
        assert!(lifecycle.revalidate(1));
        assert!(!lifecycle.revalidate(1));
        assert!(lifecycle.manages(1));
        // So does a fresh AX reading.
        lifecycle.invalidate(1);
        lifecycle.step(LifecycleEvent::Live { window: 1, facts });
        assert!(!lifecycle.is_invalidated(1));
        // Moving to a new identity carries invalidation to a window with AX state.
        lifecycle.invalidate(1);
        lifecycle.step(LifecycleEvent::Live { window: 2, facts });
        lifecycle.carry_invalidation(1, 2);
        assert!(lifecycle.is_invalidated(2) && !lifecycle.is_invalidated(1));
        lifecycle.carry_invalidation(2, 3);
        assert!(!lifecycle.is_invalidated(3) && lifecycle.facts(3).is_none());
        // Gone takes the invalidation with it.
        lifecycle.step(LifecycleEvent::Gone(2));
        assert!(!lifecycle.is_invalidated(2));
    }

    #[test]
    fn retirement_is_todays_rule_over_every_combination_of_inventory_facts() {
        // The rule as rift wrote it: an omission retires only with native negative evidence;
        // native ordering out explains an omission while the AX identity is valid.
        let reference = |reported: bool,
                         out_of_view: bool,
                         minimized: Option<bool>,
                         fullscreen: bool,
                         identified: bool,
                         invalidated: bool,
                         suitable: Option<bool>,
                         ordered_in: Option<bool>,
                         not_a_window: Option<bool>| {
            if reported || out_of_view {
                return false;
            }
            let Some(minimized) = minimized else { return false };
            if minimized || fullscreen || !identified {
                return false;
            }
            if suitable == Some(true) && ordered_in == Some(false) && !invalidated {
                return false;
            }
            suitable == Some(false) || ordered_in == Some(false) || not_a_window == Some(true)
        };
        let options = [None, Some(false), Some(true)];
        let mut checked = 0;
        for reported in [false, true] {
            for out_of_view in [false, true] {
                for minimized in options {
                    for fullscreen in [false, true] {
                        for identified in [false, true] {
                            for invalidated in [false, true] {
                                for suitable in options {
                                    for ordered_in in options {
                                        for not_a_window in options {
                                            let evidence = NativeEvidence {
                                                suitability: suitable.map(|s| {
                                                    if s {
                                                        Suitability::Suitable
                                                    } else {
                                                        Suitability::Unsuitable
                                                    }
                                                }),
                                                ordering: ordered_in.map(|o| {
                                                    if o {
                                                        Ordering::OrderedIn
                                                    } else {
                                                        Ordering::OrderedOut
                                                    }
                                                }),
                                                look: not_a_window.map(|n| {
                                                    if n {
                                                        NativeLook::NotAWindow
                                                    } else {
                                                        NativeLook::Window
                                                    }
                                                }),
                                            };
                                            let verdict = retirement(
                                                InventoryFacts {
                                                    report: if reported {
                                                        Report::Reported
                                                    } else {
                                                        Report::Omitted
                                                    },
                                                    view: if out_of_view {
                                                        InventoryView::OutOfView
                                                    } else {
                                                        InventoryView::InView
                                                    },
                                                    minimize: minimized.map(|m| {
                                                        if m {
                                                            Minimize::Minimized
                                                        } else {
                                                            Minimize::Shown
                                                        }
                                                    }),
                                                    fullscreen: if fullscreen {
                                                        FullscreenRecord::Recorded
                                                    } else {
                                                        FullscreenRecord::NotRecorded
                                                    },
                                                    identity: if identified {
                                                        NativeIdentity::Known(9_u32)
                                                    } else {
                                                        NativeIdentity::Unknown
                                                    },
                                                    validity: if invalidated {
                                                        AxValidity::Invalidated
                                                    } else {
                                                        AxValidity::Valid
                                                    },
                                                },
                                                |_| evidence,
                                            );
                                            let expected = reference(
                                                reported,
                                                out_of_view,
                                                minimized,
                                                fullscreen,
                                                identified,
                                                invalidated,
                                                suitable,
                                                ordered_in,
                                                not_a_window,
                                            );
                                            assert_eq!(
                                                verdict == Retirement::Retire,
                                                expected,
                                                "{verdict:?}"
                                            );
                                            checked += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(checked, 2 * 2 * 3 * 2 * 2 * 2 * 27);
        // The native evidence is asked only for an identified, omitted, shown, unrecorded
        // window in view.
        let kept = |facts| retirement(facts, |_| -> NativeEvidence { panic!("not asked") });
        let omitted = InventoryFacts {
            report: Report::Omitted,
            view: InventoryView::InView,
            minimize: Some(Minimize::Shown),
            fullscreen: FullscreenRecord::NotRecorded,
            identity: NativeIdentity::Known(9_u32),
            validity: AxValidity::Valid,
        };
        assert_eq!(
            kept(InventoryFacts {
                report: Report::Reported,
                ..omitted
            }),
            Retirement::Keep(Keep::Reported)
        );
        assert_eq!(
            kept(InventoryFacts {
                view: InventoryView::OutOfView,
                ..omitted
            }),
            Retirement::Keep(Keep::OutOfView)
        );
        assert_eq!(
            kept(InventoryFacts { minimize: None, ..omitted }),
            Retirement::Keep(Keep::NoAxState)
        );
        assert_eq!(
            kept(InventoryFacts {
                minimize: Some(Minimize::Minimized),
                ..omitted
            }),
            Retirement::Keep(Keep::Minimized)
        );
        assert_eq!(
            kept(InventoryFacts {
                fullscreen: FullscreenRecord::Recorded,
                ..omitted
            }),
            Retirement::Keep(Keep::Fullscreen)
        );
        assert_eq!(
            kept(InventoryFacts {
                identity: NativeIdentity::Unknown,
                ..omitted
            }),
            Retirement::Keep(Keep::NoNativeIdentity)
        );
    }

    #[test]
    fn the_layout_hears_of_a_window_whose_rules_changed_something_or_on_a_reapply() {
        let still = RuleOutcome {
            workspace_before: Some(1),
            workspace: 1,
            floating_before: false,
            floating: false,
            was_ignored: false,
            unavailable: false,
        };
        assert!(!rule_refreshes_layout(RuleRun::Admission, &still));
        assert!(rule_refreshes_layout(RuleRun::Reapply, &still));
        for changed in [
            RuleOutcome { workspace: 2, ..still },
            RuleOutcome {
                workspace_before: None,
                ..still
            },
            RuleOutcome { floating: true, ..still },
            RuleOutcome { floating_before: true, ..still },
            RuleOutcome { was_ignored: true, ..still },
            RuleOutcome { unavailable: true, ..still },
        ] {
            assert!(
                rule_refreshes_layout(RuleRun::Admission, &changed),
                "{changed:?}"
            );
        }
    }
}
