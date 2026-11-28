//! The Roc core's state machines on rift's path (the `roc-core` feature): a machine rift keeps
//! between events has a Roc twin, stepped beside it with every change the owner makes. Each
//! step must come to the same thing in both (checked on every step), and the twin must keep
//! the same state (its whole view checked on every step in tests, every [`VIEW_EVERY`] steps
//! in a running Rift). A difference is a `roc-divergence:` (tests panic, a running Rift warns).
//! Without `roc-core` a twinned machine is only its Rust machine.
//!
//! Which machine stands differs by machine. The lifecycle is flipped (the lead's policy (a)): the
//! Roc core decides, its answers are rift's, and on a difference the Rust machine, now its shadow,
//! is re-seeded from the Roc core's state (counted: [`lifecycle_reseeds`]). The main window is
//! flipped the same way ([`main_window_reseeds`]), and the joined workspaces
//! ([`workspaces_reseeds`]).
//!
//! A twinned machine reads as its Rust machine (`Deref`), and only its own methods change it:
//! the Rust machine's changing methods take `&mut`, which `Deref` does not give, so no change
//! can reach the Rust machine without reaching the twin.

use std::ops::Deref;

use rift_core::constraints::{Constraints, ConstraintsEvent};
use rift_core::lifecycle::{Invalidation, Lifecycle, LifecycleEvent, Revalidation, Transition};
use rift_core::main_window::{MainWindowEvent, MainWindows};
#[cfg(feature = "roc-core")]
use rift_core::workspaces::Refused;
use rift_core::workspaces::{Workspaces, WorkspacesEvent};

use crate::actor::app::{WindowId, pid_t};
use crate::model::VirtualWorkspaceId;
use crate::model::window_store::Claimant;
use crate::sys::screen::SpaceId;

/// How often a running Rift compares a twin's whole state with its Rust machine's, in steps.
/// A view costs time in the number of windows (measured in release, lifecycle: 0.15 µs at 10
/// windows, 0.36 µs at 100, 3 µs at 1000, against 75 ns for a step at any count), so every
/// 32nd step keeps the view's share of a step below 15 ns at 100 windows and about 100 ns at
/// 1000, and a divergence in state that no outcome showed is still reported within 32 steps.
/// Tests compare every step.
#[cfg(feature = "roc-core")]
const VIEW_EVERY: u64 = 32;

/// How many times a lifecycle's Rust shadow was re-seeded from the Roc core, process-wide.
#[cfg(feature = "roc-core")]
static LIFECYCLE_RESEEDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many times the main window's Rust shadow was re-seeded from the Roc core, process-wide.
#[cfg(feature = "roc-core")]
static MAIN_WINDOW_RESEEDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many times the floating state's Rust shadow was re-seeded from the Roc core.
#[cfg(feature = "roc-core")]
static FLOATS_RESEEDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many times the floating state's Rust shadow has been re-seeded from the Roc core (each one
/// a `roc-divergence:` warned).
#[cfg(feature = "roc-core")]
pub(crate) fn floats_reseeds() -> u64 { FLOATS_RESEEDS.load(std::sync::atomic::Ordering::Relaxed) }

/// How many times the window constraints' Rust shadow was re-seeded from the Roc core.
#[cfg(feature = "roc-core")]
static CONSTRAINTS_RESEEDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many times the window constraints' Rust shadow has been re-seeded from the Roc core (each
/// one a `roc-divergence:` warned).
#[cfg(feature = "roc-core")]
pub(crate) fn constraints_reseeds() -> u64 {
    CONSTRAINTS_RESEEDS.load(std::sync::atomic::Ordering::Relaxed)
}

/// How many times the display memory's Rust shadow was re-seeded from the Roc core.
#[cfg(feature = "roc-core")]
static DISPLAYS_RESEEDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many times the display memory's Rust shadow has been re-seeded from the Roc core (each one
/// a `roc-divergence:` warned).
#[cfg(feature = "roc-core")]
pub(crate) fn displays_reseeds() -> u64 {
    DISPLAYS_RESEEDS.load(std::sync::atomic::Ordering::Relaxed)
}

/// How many times the native memory's Rust shadow was re-seeded from the Roc core.
#[cfg(feature = "roc-core")]
static NATIVE_RESEEDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many times the native memory's Rust shadow has been re-seeded from the Roc core (each one
/// a `roc-divergence:` warned).
#[cfg(feature = "roc-core")]
pub(crate) fn native_reseeds() -> u64 { NATIVE_RESEEDS.load(std::sync::atomic::Ordering::Relaxed) }

/// How many times the evictions' Rust shadow was re-seeded from the Roc core.
#[cfg(feature = "roc-core")]
static EVICTIONS_RESEEDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many times the evictions' Rust shadow has been re-seeded from the Roc core (each one a
/// `roc-divergence:` warned).
#[cfg(feature = "roc-core")]
pub(crate) fn evictions_reseeds() -> u64 {
    EVICTIONS_RESEEDS.load(std::sync::atomic::Ordering::Relaxed)
}

/// How many times the joined workspaces' Rust shadow was re-seeded from the Roc core.
#[cfg(feature = "roc-core")]
static WORKSPACES_RESEEDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many times the joined workspaces' Rust shadow has been re-seeded from the Roc core (each
/// one a `roc-divergence:` warned).
#[cfg(feature = "roc-core")]
pub(crate) fn workspaces_reseeds() -> u64 {
    WORKSPACES_RESEEDS.load(std::sync::atomic::Ordering::Relaxed)
}

/// How many times the main window's Rust shadow has been re-seeded from the Roc core (each one a
/// `roc-divergence:` warned).
#[cfg(feature = "roc-core")]
pub(crate) fn main_window_reseeds() -> u64 {
    MAIN_WINDOW_RESEEDS.load(std::sync::atomic::Ordering::Relaxed)
}

/// How many times a lifecycle's Rust shadow has been re-seeded from the Roc core (each one a
/// `roc-divergence:` warned).
#[cfg(feature = "roc-core")]
pub(crate) fn lifecycle_reseeds() -> u64 {
    LIFECYCLE_RESEEDS.load(std::sync::atomic::Ordering::Relaxed)
}

/// The lifecycle (every window with AX state and its presence), with its Roc twin under
/// `roc-core`. Under `roc-core` the Roc core decides and the Rust machine is its shadow, read
/// through `Deref` and re-seeded on any difference.
#[derive(Default)]
pub(crate) struct TwinnedLifecycle {
    rust: Lifecycle<WindowId>,
    #[cfg(feature = "roc-core")]
    twin: LifecycleTwin,
}

#[cfg(feature = "roc-core")]
#[derive(Default)]
struct LifecycleTwin {
    roc: rift_core_roc::RocLifecycle,
    steps: u64,
}

impl std::fmt::Debug for TwinnedLifecycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.rust.fmt(f) }
}

impl Deref for TwinnedLifecycle {
    type Target = Lifecycle<WindowId>;

    fn deref(&self) -> &Lifecycle<WindowId> { &self.rust }
}

impl TwinnedLifecycle {
    /// As [`Lifecycle::step`].
    pub(crate) fn step(&mut self, event: LifecycleEvent<WindowId>) -> Option<Transition<WindowId>> {
        let transition = self.rust.step(event);
        #[cfg(feature = "roc-core")]
        let window = match event {
            LifecycleEvent::Live { window, .. }
            | LifecycleEvent::Changed { window, .. }
            | LifecycleEvent::Gone(window) => window,
        };
        #[cfg(feature = "roc-core")]
        let decided = {
            use crate::roc_path::Word;
            let event = match event {
                LifecycleEvent::Live { window, facts } => {
                    LifecycleEvent::Live { window: window.word(), facts }
                }
                LifecycleEvent::Changed { window, change } => {
                    LifecycleEvent::Changed { window: window.word(), change }
                }
                LifecycleEvent::Gone(window) => LifecycleEvent::Gone(window.word()),
            };
            let roc = self.twin.roc.step(event);
            let rust = transition.map(|transition| match transition {
                Transition::Admitted(_) => rift_core::lifecycle::Change::Admitted,
                Transition::Suspended(_, why) => rift_core::lifecycle::Change::Suspended(why),
            });
            self.stepped("lifecycle step", &event, &rust, &roc);
            // The Roc core decides.
            roc.map(|change| match change {
                rift_core::lifecycle::Change::Admitted => Transition::Admitted(window),
                rift_core::lifecycle::Change::Suspended(why) => Transition::Suspended(window, why),
            })
        };
        #[cfg(not(feature = "roc-core"))]
        let decided = transition;
        decided
    }

    /// As [`Lifecycle::invalidate`].
    pub(crate) fn invalidate(&mut self, window: WindowId) -> Invalidation {
        let invalidated = self.rust.invalidate(window);
        #[cfg(feature = "roc-core")]
        let decided = {
            use crate::roc_path::Word;
            let roc = self.twin.roc.invalidate(window.word());
            self.stepped("lifecycle invalidate", &window, &invalidated, &roc);
            roc
        };
        #[cfg(not(feature = "roc-core"))]
        let decided = invalidated;
        decided
    }

    /// As [`Lifecycle::revalidate`].
    pub(crate) fn revalidate(&mut self, window: WindowId) -> Revalidation {
        let revalidated = self.rust.revalidate(window);
        #[cfg(feature = "roc-core")]
        let decided = {
            use crate::roc_path::Word;
            let roc = self.twin.roc.revalidate(window.word());
            self.stepped("lifecycle revalidate", &window, &revalidated, &roc);
            roc
        };
        #[cfg(not(feature = "roc-core"))]
        let decided = revalidated;
        decided
    }

    /// As [`Lifecycle::carry_invalidation`], step by step so the twin takes each.
    pub(crate) fn carry_invalidation(&mut self, from: WindowId, to: WindowId) {
        if self.revalidate(from) == Revalidation::Revalidated {
            self.invalidate(to);
        }
    }

    /// One step taken by both: its outcomes must agree, and, when due, their windows. On a
    /// difference the Roc core stands: the Rust shadow is re-seeded from its windows.
    #[cfg(feature = "roc-core")]
    fn stepped<A: PartialEq + std::fmt::Debug>(
        &mut self,
        what: &str,
        question: &impl std::fmt::Debug,
        rust: &A,
        roc: &A,
    ) {
        use crate::roc_path::diverged;
        self.twin.steps += 1;
        let outcome_differs = rust != roc;
        if outcome_differs {
            diverged(what, question, rust, roc);
        }
        if outcome_differs || cfg!(test) || self.twin.steps.is_multiple_of(VIEW_EVERY) {
            let roc = self.twin.roc.view();
            let rust = self.rust_view();
            if rust != roc {
                diverged("lifecycle windows", question, &rust, &roc);
            }
            if outcome_differs || rust != roc {
                self.reseed(roc);
            }
        }
    }

    /// The Rust shadow's windows as words, in window order (as the Roc core's view).
    #[cfg(feature = "roc-core")]
    fn rust_view(&self) -> Vec<(u64, rift_core::lifecycle::Presence)> {
        use crate::roc_path::Word;
        let mut rust: Vec<_> = self
            .rust
            .presences()
            .map(|(window, presence)| (window.word(), presence))
            .collect();
        rust.sort_by_key(|&(window, _)| window);
        rust
    }

    /// Replace the Rust shadow with the Roc core's windows, and count it.
    #[cfg(feature = "roc-core")]
    fn reseed(&mut self, roc: Vec<(u64, rift_core::lifecycle::Presence)>) {
        self.rust = Lifecycle::from_presences(roc.into_iter().filter_map(|(word, presence)| {
            Some((crate::roc_path::window_of_word(word)?, presence))
        }));
        let reseeds = LIFECYCLE_RESEEDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        tracing::warn!(
            reseeds,
            "roc-divergence: the lifecycle's Rust shadow was re-seeded from the Roc core"
        );
    }
}

/// The workspaces joined: the catalog (each workspace with its Space), what each Space shows and
/// who places each window (a workspace, a claim, or the user's hand), with its Roc twin under
/// `roc-core`. A refused event is a bug in the caller (`workspace-refused:`, a panic in tests, a
/// warning in a running Rift) and changes nothing. Flipped (policy (a)): under `roc-core` the Roc
/// core decides (its outcome is the one reported) and the Rust machine is its shadow, read
/// through `Deref`: both must meet every event the same way and keep the same workspaces,
/// compared after every topology change (they are rare, and the view is small without many
/// windows), after every [`VIEW_EVERY`]th ownership change in a running Rift (every one in
/// tests), and after any outcome that differs; on a difference the shadow is re-seeded from the
/// Roc core's view.
#[derive(Default)]
pub(crate) struct TwinnedWorkspaces {
    rust: Workspaces<WindowId, VirtualWorkspaceId, SpaceId, Claimant>,
    #[cfg(feature = "roc-core")]
    twin: WorkspacesTwin,
}

#[cfg(feature = "roc-core")]
#[derive(Default)]
struct WorkspacesTwin {
    roc: rift_core_roc::RocWorkspaces,
    steps: u64,
}

impl std::fmt::Debug for TwinnedWorkspaces {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.rust.fmt(f) }
}

impl Deref for TwinnedWorkspaces {
    type Target = Workspaces<WindowId, VirtualWorkspaceId, SpaceId, Claimant>;

    fn deref(&self) -> &Self::Target { &self.rust }
}

/// A change to the joined workspaces, as the platform's types make it.
pub(crate) type JoinedEvent = WorkspacesEvent<WindowId, VirtualWorkspaceId, SpaceId, Claimant>;

impl TwinnedWorkspaces {
    /// As [`Workspaces::step`]; a refusal is reported here.
    pub(crate) fn step(&mut self, event: JoinedEvent) {
        let met = self.rust.step(event);
        #[cfg(feature = "roc-core")]
        let met = self.stepped(event, met);
        if let Err(refused) = met {
            #[cfg(test)]
            panic!("workspace-refused: {refused:?} by {event:?}");
            #[cfg(not(test))]
            tracing::warn!(
                ?refused,
                ?event,
                "workspace-refused: the workspaces refused a change (a bug in its caller)"
            );
        }
    }

    /// The Roc core takes the event the Rust shadow took: its outcome stands, and on any
    /// difference (the outcome, or the workspaces when due) the shadow is re-seeded from it.
    #[cfg(feature = "roc-core")]
    fn stepped(
        &mut self,
        event: JoinedEvent,
        met: Result<(), Refused<VirtualWorkspaceId, SpaceId>>,
    ) -> Result<(), Refused<VirtualWorkspaceId, SpaceId>> {
        use crate::roc_path::{Word, diverged};
        let plain = plain_joined_event(event);
        let roc = self.twin.roc.step(plain);
        let rust = met.map_err(|refused| match refused {
            Refused::Exists(workspace) => Refused::Exists(workspace.word()),
            Refused::Unknown(workspace) => Refused::Unknown(workspace.word()),
            Refused::OwnSuccessor(workspace) => Refused::OwnSuccessor(workspace.word()),
            Refused::Shown { workspace, space } => Refused::Shown {
                workspace: workspace.word(),
                space: space.word(),
            },
        });
        let outcome_differs = rust != roc;
        if outcome_differs {
            diverged("workspaces step", &plain, &rust, &roc);
        }
        self.twin.steps += 1;
        let topology = !matches!(event, WorkspacesEvent::Owned { .. });
        if outcome_differs || topology || cfg!(test) || self.twin.steps.is_multiple_of(VIEW_EVERY) {
            let roc_view = self.twin.roc.view();
            let rust_view = self.rust_view();
            if rust_view != roc_view {
                diverged("workspaces view", &plain, &rust_view, &roc_view);
            }
            if outcome_differs || rust_view != roc_view {
                self.reseed(roc_view);
            }
        }
        // The Roc core decides.
        roc.map_err(|refused| match refused {
            Refused::Exists(workspace) => Refused::Exists(workspace_of_word(workspace)),
            Refused::Unknown(workspace) => Refused::Unknown(workspace_of_word(workspace)),
            Refused::OwnSuccessor(workspace) => Refused::OwnSuccessor(workspace_of_word(workspace)),
            Refused::Shown { workspace, space } => Refused::Shown {
                workspace: workspace_of_word(workspace),
                space: SpaceId::new(space),
            },
        })
    }

    /// Replace the Rust shadow with the Roc core's workspaces, and count it. A view the shadow
    /// cannot hold (a word naming no window or claimant, or an event the step refuses) leaves
    /// the shadow as it was, said so beside the divergence already warned.
    #[cfg(feature = "roc-core")]
    fn reseed(&mut self, roc: rift_core_roc::workspaces::WorkspacesView) {
        use rift_core::workspaces::{Activation, Destination, Ownership};

        use crate::roc_path::window_of_word;
        let activation = |activation: Activation<u64>| match activation {
            Activation::Showing { current, previous, remembered } => Activation::Showing {
                current: workspace_of_word(current),
                previous: previous.map(workspace_of_word),
                remembered: remembered.map(workspace_of_word),
            },
            Activation::Vacant { remembered } => Activation::Vacant {
                remembered: remembered.map(workspace_of_word),
            },
        };
        let owners: Option<Vec<_>> = roc
            .owners
            .into_iter()
            .map(|(window, ownership)| {
                let ownership = match ownership {
                    Ownership::Assigned(workspace) => {
                        Ownership::Assigned(workspace_of_word(workspace))
                    }
                    Ownership::Held => Ownership::Held,
                    Ownership::Claimed { owner, destination } => Ownership::Claimed {
                        owner: Claimant::from_plain(owner)?,
                        destination: match destination {
                            Destination::Returns(workspace) => {
                                Destination::Returns(workspace_of_word(workspace))
                            }
                            Destination::Nowhere => Destination::Nowhere,
                        },
                    },
                };
                Some((window_of_word(window)?, ownership))
            })
            .collect();
        let rebuilt = owners.ok_or(None).and_then(|owners| {
            Workspaces::from_view(
                roc.catalog
                    .into_iter()
                    .map(|(workspace, space)| (workspace_of_word(workspace), SpaceId::new(space))),
                roc.activations
                    .into_iter()
                    .map(|(space, shown)| (SpaceId::new(space), activation(shown))),
                owners,
            )
            .map_err(Some)
        });
        match rebuilt {
            Ok(rebuilt) => {
                self.rust = rebuilt;
                let reseeds =
                    WORKSPACES_RESEEDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                tracing::warn!(
                    reseeds,
                    "roc-divergence: the joined workspaces' Rust shadow was re-seeded from the Roc core"
                );
            }
            Err(refused) => tracing::warn!(
                ?refused,
                "roc-divergence: the Roc core's workspaces could not be held by the Rust shadow; it stays as it was"
            ),
        }
    }

    /// The Rust workspaces as the Roc core's view: owners and each workspace's windows in word
    /// order.
    #[cfg(feature = "roc-core")]
    fn rust_view(&self) -> rift_core_roc::workspaces::WorkspacesView {
        use crate::roc_path::Word;
        let mut owners: Vec<_> = self
            .rust
            .owners()
            .map(|(window, ownership)| (window.word(), plain_ownership(ownership)))
            .collect();
        owners.sort_by_key(|&(window, _)| window);
        rift_core_roc::workspaces::WorkspacesView {
            catalog: self
                .rust
                .catalog()
                .iter()
                .map(|&(workspace, space)| (workspace.word(), space.word()))
                .collect(),
            activations: self
                .rust
                .activations()
                .iter()
                .map(|&(space, shown)| (space.word(), plain_activation(shown)))
                .collect(),
            owners,
            windows: self
                .rust
                .catalog()
                .iter()
                .map(|&(workspace, _)| {
                    let mut windows: Vec<u64> =
                        self.rust.windows_on(workspace).map(Word::word).collect();
                    windows.sort_unstable();
                    windows
                })
                .collect(),
        }
    }
}

/// The workspace a word names (the inverse of its `Word`).
#[cfg(feature = "roc-core")]
fn workspace_of_word(word: u64) -> VirtualWorkspaceId { slotmap::KeyData::from_ffi(word).into() }

/// A joined event with its windows, workspaces, Spaces and owners as words.
#[cfg(feature = "roc-core")]
fn plain_joined_event(
    event: JoinedEvent,
) -> WorkspacesEvent<u64, u64, u64, rift_core::workspaces::plain::Claimant> {
    use rift_core::workspaces::{Owned, Successor};

    use crate::roc_path::Word;
    match event {
        WorkspacesEvent::Created { workspace, space } => WorkspacesEvent::Created {
            workspace: workspace.word(),
            space: space.word(),
        },
        WorkspacesEvent::Removed { workspace, successor } => WorkspacesEvent::Removed {
            workspace: workspace.word(),
            successor: match successor {
                Successor::Into(heir) => Successor::Into(heir.word()),
                Successor::Unassign => Successor::Unassign,
            },
        },
        WorkspacesEvent::Moved { workspace, space } => WorkspacesEvent::Moved {
            workspace: workspace.word(),
            space: space.word(),
        },
        WorkspacesEvent::Activated { space, activation } => WorkspacesEvent::Activated {
            space: space.word(),
            activation: plain_activation(activation),
        },
        WorkspacesEvent::Cleared(space) => WorkspacesEvent::Cleared(space.word()),
        WorkspacesEvent::Owned { window, owned } => WorkspacesEvent::Owned {
            window: window.word(),
            owned: match owned {
                Owned::Unowned => Owned::Unowned,
                Owned::Owner(ownership) => Owned::Owner(plain_ownership(ownership)),
            },
        },
    }
}

/// An activation with its workspaces as words.
#[cfg(feature = "roc-core")]
fn plain_activation(
    activation: rift_core::workspaces::Activation<VirtualWorkspaceId>,
) -> rift_core::workspaces::Activation<u64> {
    use rift_core::workspaces::Activation;

    use crate::roc_path::Word;
    let word = |workspace: Option<VirtualWorkspaceId>| workspace.map(Word::word);
    match activation {
        Activation::Showing { current, previous, remembered } => Activation::Showing {
            current: current.word(),
            previous: word(previous),
            remembered: word(remembered),
        },
        Activation::Vacant { remembered } => Activation::Vacant { remembered: word(remembered) },
    }
}

/// An ownership with its workspaces and owner as words.
#[cfg(feature = "roc-core")]
fn plain_ownership(
    ownership: rift_core::workspaces::Ownership<VirtualWorkspaceId, Claimant>,
) -> rift_core::workspaces::Ownership<u64, rift_core::workspaces::plain::Claimant> {
    use rift_core::workspaces::{Destination, Ownership};

    use crate::roc_path::Word;
    match ownership {
        Ownership::Assigned(workspace) => Ownership::Assigned(workspace.word()),
        Ownership::Claimed { owner, destination } => Ownership::Claimed {
            owner: owner.plain(),
            destination: match destination {
                Destination::Returns(workspace) => Destination::Returns(workspace.word()),
                Destination::Nowhere => Destination::Nowhere,
            },
        },
        Ownership::Held => Ownership::Held,
    }
}

/// The main window (each app's standing and main window, the globally frontmost app and the
/// native focus), with its Roc twin under `roc-core`. Flipped (policy (a)): under `roc-core` the
/// Roc core decides and the Rust machine is its shadow, read through `Deref` and re-seeded on any
/// difference.
#[derive(Default)]
pub(crate) struct TwinnedMainWindows {
    rust: MainWindows<pid_t, WindowId>,
    #[cfg(feature = "roc-core")]
    twin: MainWindowsTwin,
}

#[cfg(feature = "roc-core")]
#[derive(Default)]
struct MainWindowsTwin {
    roc: rift_core_roc::RocMainWindows,
    steps: u64,
}

impl Deref for TwinnedMainWindows {
    type Target = MainWindows<pid_t, WindowId>;

    fn deref(&self) -> &Self::Target { &self.rust }
}

impl TwinnedMainWindows {
    /// As [`MainWindows::step`]: both must answer the same window, and, when due, keep the same
    /// state.
    pub(crate) fn step(&mut self, event: MainWindowEvent<pid_t, WindowId>) -> Option<WindowId> {
        let answer = self.rust.step(event);
        #[cfg(feature = "roc-core")]
        let decided = {
            use crate::roc_path::{Word, diverged};
            let plain = match event {
                MainWindowEvent::Launched { pid, standing, main_window } => {
                    MainWindowEvent::Launched {
                        pid: pid.word(),
                        standing,
                        main_window: main_window.map(Word::word),
                    }
                }
                MainWindowEvent::Terminated(pid) => MainWindowEvent::Terminated(pid.word()),
                MainWindowEvent::Activated(pid, initiator) => {
                    MainWindowEvent::Activated(pid.word(), initiator)
                }
                MainWindowEvent::Deactivated(pid) => MainWindowEvent::Deactivated(pid.word()),
                MainWindowEvent::GloballyActivated(pid) => {
                    MainWindowEvent::GloballyActivated(pid.word())
                }
                MainWindowEvent::GloballyDeactivated(pid) => {
                    MainWindowEvent::GloballyDeactivated(pid.word())
                }
                MainWindowEvent::MainWindowChanged(pid, window, initiator) => {
                    MainWindowEvent::MainWindowChanged(
                        pid.word(),
                        window.map(Word::word),
                        initiator,
                    )
                }
                MainWindowEvent::NativeFocused { pid, window } => MainWindowEvent::NativeFocused {
                    pid: pid.word(),
                    window: window.word(),
                },
                MainWindowEvent::Replaced { pid, window, replacement } => {
                    MainWindowEvent::Replaced {
                        pid: pid.word(),
                        window: window.word(),
                        replacement: replacement.map(Word::word),
                    }
                }
            };
            let roc = self.twin.roc.step(plain);
            let rust = answer.map(Word::word);
            let answer_differs = rust != roc;
            if answer_differs {
                diverged("main window step", &plain, &rust, &roc);
            }
            self.twin.steps += 1;
            if answer_differs || cfg!(test) || self.twin.steps.is_multiple_of(VIEW_EVERY) {
                let rust = self.rust_view();
                let view = self.twin.roc.view();
                if rust != view {
                    diverged("main windows", &plain, &rust, &view);
                }
                if answer_differs || rust != view {
                    self.reseed(view);
                }
            }
            // The Roc core decides.
            roc.and_then(crate::roc_path::window_of_word)
        };
        #[cfg(not(feature = "roc-core"))]
        let decided = answer;
        decided
    }

    /// Replace the Rust shadow with the Roc core's state, and count it.
    #[cfg(feature = "roc-core")]
    fn reseed(&mut self, view: rift_core_roc::main_window::MainWindowsView) {
        use rift_core::main_window::{App, NativeFocus};

        use crate::roc_path::window_of_word;
        let pid = |word: u64| word as i64 as pid_t;
        let apps = view.apps.into_iter().map(|(word, app)| {
            (pid(word), App {
                standing: app.standing,
                last_activation: app.last_activation,
                main_window: app.main_window.and_then(window_of_word),
            })
        });
        let native_focus = match view.native_focus {
            NativeFocus::NotYetReported => NativeFocus::NotYetReported,
            NativeFocus::Reported(focused) => NativeFocus::Reported(
                focused.and_then(|(owner, window)| Some((pid(owner), window_of_word(window)?))),
            ),
        };
        self.rust = MainWindows::from_parts(apps, view.global_frontmost.map(pid), native_focus);
        let reseeds = MAIN_WINDOW_RESEEDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        tracing::warn!(
            reseeds,
            "roc-divergence: the main window's Rust shadow was re-seeded from the Roc core"
        );
    }

    /// The Rust machine's whole state as words, as the Roc core views it (apps in pid order).
    #[cfg(feature = "roc-core")]
    fn rust_view(&self) -> rift_core_roc::main_window::MainWindowsView {
        use rift_core::main_window::{App, NativeFocus};

        use crate::roc_path::Word;
        let mut apps: Vec<_> = self
            .rust
            .apps()
            .map(|(pid, app)| {
                (pid.word(), App {
                    standing: app.standing,
                    last_activation: app.last_activation,
                    main_window: app.main_window.map(Word::word),
                })
            })
            .collect();
        apps.sort_by_key(|&(pid, _)| pid);
        rift_core_roc::main_window::MainWindowsView {
            main_window: self.rust.main_window().map(Word::word),
            native_focus: match self.rust.native_focus_state() {
                NativeFocus::NotYetReported => NativeFocus::NotYetReported,
                NativeFocus::Reported(focused) => {
                    NativeFocus::Reported(focused.map(|(pid, window)| (pid.word(), window.word())))
                }
            },
            apps,
            global_frontmost: self.rust.global_frontmost().map(Word::word),
        }
    }
}

/// Every window's size constraints and each app's last minimum (engine slice E4), with its Roc
/// twin under `roc-core`. Flipped (policy (a)): under `roc-core` the Roc core decides and the
/// Rust machine is its shadow, read through `Deref` and re-seeded on any difference.
#[derive(Default)]
pub(crate) struct TwinnedConstraints {
    rust: Constraints<WindowId, pid_t>,
    #[cfg(feature = "roc-core")]
    twin: ConstraintsTwin,
}

#[cfg(feature = "roc-core")]
#[derive(Default)]
struct ConstraintsTwin {
    roc: rift_core_roc::RocSizes,
    steps: u64,
}

impl Deref for TwinnedConstraints {
    type Target = Constraints<WindowId, pid_t>;

    fn deref(&self) -> &Self::Target { &self.rust }
}

impl std::fmt::Debug for TwinnedConstraints {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.rust.fmt(f) }
}

impl TwinnedConstraints {
    /// As [`Constraints::step`]: both must answer the same change, and, when due, keep the same
    /// state.
    pub(crate) fn step(&mut self, event: ConstraintsEvent<WindowId, pid_t>) -> bool {
        let answer = self.rust.step(event);
        #[cfg(feature = "roc-core")]
        let decided = {
            use crate::roc_path::{Word, diverged};
            let plain = match event {
                ConstraintsEvent::Observed {
                    window,
                    app,
                    respect_min_size,
                    observed,
                } => ConstraintsEvent::Observed {
                    window: window.word(),
                    app,
                    respect_min_size,
                    observed,
                },
                ConstraintsEvent::WidthSet { window, width } => {
                    ConstraintsEvent::WidthSet { window: window.word(), width }
                }
                ConstraintsEvent::WidthPreserved { window, observed, width } => {
                    ConstraintsEvent::WidthPreserved {
                        window: window.word(),
                        observed,
                        width,
                    }
                }
                ConstraintsEvent::Retired(window) => ConstraintsEvent::Retired(window.word()),
                ConstraintsEvent::Replaced { from, to } => ConstraintsEvent::Replaced {
                    from: from.word(),
                    to: to.word(),
                },
                ConstraintsEvent::AppQuit(app) => ConstraintsEvent::AppQuit(app),
            };
            let roc = self.twin.roc.step(plain);
            let answer_differs = roc != answer;
            if answer_differs {
                diverged("constraints step", &plain, &answer, &roc);
            }
            self.twin.steps += 1;
            if answer_differs || cfg!(test) || self.twin.steps.is_multiple_of(VIEW_EVERY) {
                let rust = self.rust_view();
                let view = self.twin.roc.view();
                if rust != view {
                    diverged("constraints", &plain, &rust, &view);
                }
                if answer_differs || rust != view {
                    self.reseed(view);
                }
            }
            // The Roc core decides.
            roc
        };
        #[cfg(not(feature = "roc-core"))]
        let decided = answer;
        decided
    }

    /// The Rust machine as words, as the Roc core views it (records and minimums in word order).
    #[cfg(feature = "roc-core")]
    fn rust_view(&self) -> Constraints<u64, pid_t> {
        use crate::roc_path::Word;
        Constraints::from_parts(
            self.rust.windows().map(|(window, c)| (window.word(), c)),
            self.rust.app_minimums(),
            self.rust.revision(),
        )
    }

    /// Replace the Rust shadow with the Roc core's state, and count it.
    #[cfg(feature = "roc-core")]
    fn reseed(&mut self, view: Constraints<u64, pid_t>) {
        use crate::roc_path::window_of_word;
        self.rust = Constraints::from_parts(
            view.windows().filter_map(|(word, c)| Some((window_of_word(word)?, c))),
            view.app_minimums(),
            view.revision(),
        );
        let reseeds = CONSTRAINTS_RESEEDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        tracing::warn!(
            reseeds,
            "roc-divergence: the window constraints' Rust shadow was re-seeded from the Roc core"
        );
    }
}

/// What the layout engine remembers of displays as it holds them (engine slice E3).
pub(crate) type RiftDisplays = rift_core::display_memory::DisplayMemory<SpaceId, String>;

/// An event of the display memory as the engine steps it.
pub(crate) type RiftDisplayEvent = rift_core::display_memory::DisplayEvent<SpaceId, String>;

/// Each Space's display and each display's last Space (engine slice E3), with its Roc twin under
/// `roc-core`. Flipped (policy (a)): under `roc-core` the Roc core decides and the Rust machine is
/// its shadow, read through `Deref` and re-seeded on any difference. A display crosses as a word:
/// its place in the twin's table of display names (`DisplayNames`).
#[derive(Default)]
pub(crate) struct TwinnedDisplays {
    rust: RiftDisplays,
    #[cfg(feature = "roc-core")]
    twin: DisplaysTwin,
}

#[cfg(feature = "roc-core")]
#[derive(Default)]
struct DisplaysTwin {
    roc: rift_core_roc::RocDisplayMemory,
    names: DisplayNames,
    steps: u64,
}

/// Display names as the Roc core's words: each name's place in the table. The table only grows,
/// and that is fine: it is bounded by the displays this process has ever seen, a handful.
#[cfg(feature = "roc-core")]
#[derive(Default)]
struct DisplayNames(Vec<String>);

#[cfg(feature = "roc-core")]
impl DisplayNames {
    /// `display`'s word: its place in the table, added if new.
    fn word(&mut self, display: &str) -> u64 {
        let place = match self.0.iter().position(|named| named == display) {
            Some(place) => place,
            None => {
                self.0.push(display.to_owned());
                self.0.len() - 1
            }
        };
        place as u64
    }

    /// The name of `word`; every word the Roc core holds came from this table.
    fn name(&self, word: u64) -> String { self.0[word as usize].clone() }
}

#[cfg(feature = "roc-core")]
impl DisplaysTwin {
    fn word(&mut self, display: &str) -> u64 { self.names.word(display) }

    fn event(
        &mut self,
        event: &RiftDisplayEvent,
    ) -> rift_core::display_memory::DisplayEvent<u64, u64> {
        use rift_core::display_memory::DisplayEvent as E;

        use crate::roc_path::Word;
        match event {
            E::Shown { space, display, last } => E::Shown {
                space: space.word(),
                display: self.word(display),
                last: *last,
            },
            E::Hidden(space) => E::Hidden(space.word()),
            E::Remapped { from, to } => E::Remapped {
                from: from.word(),
                to: to.word(),
            },
            E::Forgotten(space) => E::Forgotten(space.word()),
        }
    }
}

impl Deref for TwinnedDisplays {
    type Target = RiftDisplays;

    fn deref(&self) -> &Self::Target { &self.rust }
}

impl std::fmt::Debug for TwinnedDisplays {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.rust.fmt(f) }
}

impl TwinnedDisplays {
    /// A loaded memory: the Roc core is stepped to the same state (each last Space shown on its
    /// display and hidden again, then each Space shown without moving a last Space).
    pub(crate) fn seeded(rust: RiftDisplays) -> Self {
        #[cfg(feature = "roc-core")]
        let twin = {
            use rift_core::display_memory::{DisplayEvent, LastSpace};
            let mut twin = DisplaysTwin::default();
            let mut seed = |event: RiftDisplayEvent| {
                let plain = twin.event(&event);
                twin.roc.step(plain);
            };
            for (display, space) in rust.last_spaces() {
                let display = display.clone();
                seed(DisplayEvent::Shown {
                    space,
                    display,
                    last: LastSpace::Follow,
                });
                seed(DisplayEvent::Hidden(space));
            }
            for (space, display) in rust.displays() {
                let display = display.clone();
                seed(DisplayEvent::Shown {
                    space,
                    display,
                    last: LastSpace::Hold,
                });
            }
            twin
        };
        let seeded = TwinnedDisplays {
            rust,
            #[cfg(feature = "roc-core")]
            twin,
        };
        // Tests compare the seed at once: a load is often not followed by a step.
        #[cfg(all(test, feature = "roc-core"))]
        {
            let mut seeded = seeded;
            let (rust, view) = (seeded.rust_view(), seeded.twin.roc.view());
            if rust != view {
                crate::roc_path::diverged("display memory seed", &"a saved layout", &rust, &view);
            }
            return seeded;
        }
        #[allow(unreachable_code)]
        seeded
    }

    /// As [`rift_core::display_memory::DisplayMemory::step`]: both must answer the same change,
    /// and, when due, keep the same state.
    pub(crate) fn step(&mut self, event: RiftDisplayEvent) -> bool {
        #[cfg(feature = "roc-core")]
        let plain = self.twin.event(&event);
        let answer = self.rust.step(event);
        #[cfg(feature = "roc-core")]
        let decided = {
            use crate::roc_path::diverged;
            let roc = self.twin.roc.step(plain);
            let answer_differs = roc != answer;
            if answer_differs {
                diverged("display memory step", &plain, &answer, &roc);
            }
            self.twin.steps += 1;
            if answer_differs || cfg!(test) || self.twin.steps.is_multiple_of(VIEW_EVERY) {
                let rust = self.rust_view();
                let view = self.twin.roc.view();
                if rust != view {
                    diverged("display memory", &plain, &rust, &view);
                }
                if answer_differs || rust != view {
                    self.reseed(view);
                }
            }
            // The Roc core decides.
            roc
        };
        #[cfg(not(feature = "roc-core"))]
        let decided = answer;
        decided
    }

    /// The Rust machine as words, as the Roc core views it.
    #[cfg(feature = "roc-core")]
    fn rust_view(&mut self) -> rift_core::display_memory::DisplayMemory<u64, u64> {
        use crate::roc_path::Word;
        let displays: Vec<_> = (self.rust.displays())
            .map(|(space, display)| (space.word(), display.clone()))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|(space, display)| (space, self.twin.word(&display)))
            .collect();
        let lasts: Vec<_> = (self.rust.last_spaces())
            .map(|(display, space)| (display.clone(), space.word()))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|(display, space)| (self.twin.word(&display), space))
            .collect();
        rift_core::display_memory::DisplayMemory::from_parts(displays, lasts)
    }

    /// Replace the Rust shadow with the Roc core's state, and count it.
    #[cfg(feature = "roc-core")]
    fn reseed(&mut self, view: rift_core::display_memory::DisplayMemory<u64, u64>) {
        let names = &self.twin.names;
        let name = |word: u64| names.name(word);
        self.rust = RiftDisplays::from_parts(
            view.displays().map(|(space, &display)| (SpaceId::new(space), name(display))),
            view.last_spaces().map(|(&display, space)| (name(display), SpaceId::new(space))),
        );
        let reseeds = DISPLAYS_RESEEDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        tracing::warn!(
            reseeds,
            "roc-divergence: the display memory's Rust shadow was re-seeded from the Roc core"
        );
    }
}

/// Where a display departure moved windows from, as the engine holds it (engine slice E3b).
pub(crate) type RiftEvictions =
    rift_core::evictions::Evictions<WindowId, String, VirtualWorkspaceId>;

/// An event of the evictions machine as the engine steps it.
pub(crate) type RiftEvictionEvent =
    rift_core::evictions::EvictionEvent<WindowId, pid_t, String, VirtualWorkspaceId>;

/// Where a display departure moved windows from (engine slice E3b), with its Roc twin under
/// `roc-core`. Flipped (policy (a)): under `roc-core` the Roc core holds the records and the
/// Rust machine is its shadow, read through `Deref` (the queries) and re-seeded on any
/// difference. A display crosses as its word in the twin's `DisplayNames`.
#[derive(Default)]
pub(crate) struct TwinnedEvictions {
    rust: RiftEvictions,
    #[cfg(feature = "roc-core")]
    twin: EvictionsTwin,
}

#[cfg(feature = "roc-core")]
#[derive(Default)]
struct EvictionsTwin {
    roc: rift_core_roc::RocEvictions,
    names: DisplayNames,
    steps: u64,
}

impl Deref for TwinnedEvictions {
    type Target = RiftEvictions;

    fn deref(&self) -> &Self::Target { &self.rust }
}

impl std::fmt::Debug for TwinnedEvictions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.rust.fmt(f) }
}

impl TwinnedEvictions {
    /// As [`rift_core::evictions::Evictions::step`]: both must hold the same records when due.
    pub(crate) fn step(&mut self, event: RiftEvictionEvent) {
        #[cfg(feature = "roc-core")]
        let plain = {
            use rift_core::evictions::EvictionEvent as E;

            use crate::roc_path::Word;
            let names = &mut self.twin.names;
            match &event {
                E::Evicted {
                    window,
                    from,
                    ordinal,
                    landed,
                    landed_on,
                } => E::Evicted {
                    window: window.word(),
                    from: names.word(from),
                    ordinal: *ordinal,
                    landed: landed.word(),
                    landed_on: landed_on.as_deref().map(|display| names.word(display)),
                },
                E::Forgotten(window) => E::Forgotten(window.word()),
                E::Replaced { from, to } => E::Replaced {
                    from: from.word(),
                    to: to.word(),
                },
                E::AppQuit(pid) => E::AppQuit(*pid),
            }
        };
        self.rust.step(event);
        #[cfg(feature = "roc-core")]
        {
            use crate::roc_path::diverged;
            self.twin.roc.step(plain);
            self.twin.steps += 1;
            if cfg!(test) || self.twin.steps.is_multiple_of(VIEW_EVERY) {
                let rust = self.rust_view();
                let view = self.twin.roc.view();
                if rust != view {
                    diverged("evictions", &plain, &rust, &view);
                    self.reseed(view);
                }
            }
        }
    }

    /// The Rust machine as words, as the Roc core views it.
    #[cfg(feature = "roc-core")]
    fn rust_view(&mut self) -> rift_core::evictions::Evictions<u64, u64, u64> {
        use rift_core::evictions::Eviction;

        use crate::roc_path::Word;
        let records: Vec<_> = (self.rust.records())
            .map(|(window, r)| (window.word(), r.display.clone(), r.ordinal, r.landed.word()))
            .collect();
        let names = &mut self.twin.names;
        rift_core::evictions::Evictions::from_parts(records.into_iter().map(
            |(window, display, ordinal, landed)| {
                let display = names.word(&display);
                (window, Eviction { display, ordinal, landed })
            },
        ))
    }

    /// Replace the Rust shadow with the Roc core's records, and count it.
    #[cfg(feature = "roc-core")]
    fn reseed(&mut self, view: rift_core::evictions::Evictions<u64, u64, u64>) {
        use rift_core::evictions::Eviction;

        use crate::roc_path::window_of_word;
        let names = &self.twin.names;
        self.rust = RiftEvictions::from_parts(view.records().filter_map(|(word, r)| {
            let record = Eviction {
                display: names.name(r.display),
                ordinal: r.ordinal,
                landed: workspace_of_word(r.landed),
            };
            Some((window_of_word(word)?, record))
        }));
        let reseeds = EVICTIONS_RESEEDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        tracing::warn!(
            reseeds,
            "roc-divergence: the evictions' Rust shadow was re-seeded from the Roc core"
        );
    }
}

/// The native memory as the engine holds it (engine slice E3c), over its snapshot values `S`.
pub(crate) type RiftNative<S> = rift_core::native_memory::NativeMemory<VirtualWorkspaceId, S>;

/// The native memory (engine slice E3c), with its Roc twin under `roc-core`. Flipped (policy (a)):
/// the Roc core decides and owns which workspaces hold a snapshot and the reconcile batch; the
/// Rust machine holds the snapshot values, its decisions the Roc core's shadow. Each event crosses
/// with the windows it reads, computed from the values (`NativeMemory::crossing`). A Roc restore
/// decision is applied to the Rust machine's value of that snapshot. On a disagreement both are
/// brought to what they can share (the shadow keeps only the snapshots the Roc core holds, and its
/// batch; the Roc core forgets a workspace the shadow has no value for), counted.
pub(crate) struct TwinnedNative<S> {
    rust: RiftNative<S>,
    #[cfg(feature = "roc-core")]
    twin: Box<NativeTwin>,
}

#[cfg(feature = "roc-core")]
#[derive(Default)]
struct NativeTwin {
    roc: rift_core_roc::RocNative,
    steps: u64,
}

impl<S> Default for TwinnedNative<S> {
    fn default() -> Self {
        TwinnedNative {
            rust: Default::default(),
            #[cfg(feature = "roc-core")]
            twin: Default::default(),
        }
    }
}

impl<S> Deref for TwinnedNative<S> {
    type Target = RiftNative<S>;

    fn deref(&self) -> &Self::Target { &self.rust }
}

impl<S: std::fmt::Debug> std::fmt::Debug for TwinnedNative<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.rust.fmt(f) }
}

impl<S> TwinnedNative<S>
where S: rift_core::native_memory::Held<WindowId> + Clone + std::fmt::Debug
{
    /// As [`rift_core::native_memory::NativeMemory::step`]: under `roc-core`, the Roc core's
    /// decision, applied to the Rust machine's snapshot value.
    pub(crate) fn step(
        &mut self,
        event: rift_core::native_memory::MemoryEvent<WindowId, VirtualWorkspaceId, S>,
    ) -> rift_core::native_memory::MemoryAnswer<WindowId, VirtualWorkspaceId, S> {
        #[cfg(feature = "roc-core")]
        let plain = native_words(self.rust.crossing(&event));
        let answer = self.rust.step(event);
        #[cfg(feature = "roc-core")]
        let decided = {
            use rift_core::native_memory::{Decided, MemoryAnswer, Plan};

            use crate::roc_path::diverged;
            let roc = self.twin.roc.step(plain.clone());
            let rust_decided = decided_words(answer.decided());
            let answer_differs = roc != rust_decided;
            if answer_differs {
                diverged("native memory step", &plain, &rust_decided, &roc);
            }
            self.twin.steps += 1;
            if answer_differs || cfg!(test) || self.twin.steps.is_multiple_of(VIEW_EVERY) {
                let rust_owned = owned_words(self.rust.owned());
                let view = self.twin.roc.view();
                if rust_owned != view {
                    diverged("native memory", &plain, &rust_owned, &view);
                }
                if answer_differs || rust_owned != view {
                    self.reseed(view);
                }
            }
            // The Roc core decides; the snapshot value is the Rust machine's.
            match (roc, answer) {
                (_, answer) if !answer_differs => answer,
                (Decided::Nothing, _) => MemoryAnswer::Nothing,
                (Decided::Deferred, _) => MemoryAnswer::Deferred,
                (Decided::Waited(waited), _) => {
                    MemoryAnswer::Waited(waited.into_iter().map(workspace_of_word).collect())
                }
                (Decided::Restore(decision), MemoryAnswer::Restore(plan)) => {
                    let windows = |words: Vec<u64>| -> Vec<WindowId> {
                        words.into_iter().filter_map(crate::roc_path::window_of_word).collect()
                    };
                    MemoryAnswer::Restore(Plan {
                        snapshot: plan.snapshot,
                        remove: windows(decision.remove),
                        ordered: windows(decision.ordered),
                        add: windows(decision.add),
                        spent: decision.spent,
                    })
                }
                // A restore the Rust machine has no value for: nothing to restore from.
                (Decided::Restore(_), _) => MemoryAnswer::Nothing,
            }
        };
        #[cfg(not(feature = "roc-core"))]
        let decided = answer;
        decided
    }

    /// Both brought to what they can share, and counted.
    #[cfg(feature = "roc-core")]
    fn reseed(&mut self, view: rift_core::native_memory::Owned<u64>) {
        use rift_core::native_memory::{Batch, NativeEvent};

        use crate::roc_path::Word;
        let shared: Vec<_> = (self.rust.snapshots())
            .filter(|(workspace, _)| view.held.contains(&workspace.word()))
            .map(|(workspace, snapshot)| (workspace, snapshot.clone()))
            .collect();
        let rust_held: std::collections::BTreeSet<u64> =
            shared.iter().map(|(workspace, _)| workspace.word()).collect();
        for &workspace in view.held.difference(&rust_held) {
            self.twin.roc.step(NativeEvent::Forgotten(workspace));
        }
        let batch = match view.batch {
            Batch::Idle => Batch::Idle,
            Batch::Collecting(waiting) => {
                Batch::Collecting(waiting.into_iter().map(workspace_of_word).collect())
            }
        };
        self.rust = RiftNative::from_parts(shared, batch);
        let reseeds = NATIVE_RESEEDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        tracing::warn!(
            reseeds,
            "roc-divergence: the native memory's Rust shadow was re-seeded from the Roc core"
        );
    }
}

#[cfg(feature = "roc-core")]
fn native_words(
    event: rift_core::native_memory::NativeEvent<WindowId, VirtualWorkspaceId>,
) -> rift_core::native_memory::NativeEvent<u64, u64> {
    use rift_core::native_memory::{Candidate, Current, NativeEvent as E};

    use crate::roc_path::Word;
    let words = |windows: Vec<WindowId>| windows.into_iter().map(|w| w.word()).collect();
    match event {
        E::Kept { workspace, windows } => E::Kept {
            workspace: workspace.word(),
            windows: words(windows),
        },
        E::Edited { workspace, windows } => E::Edited {
            workspace: workspace.word(),
            windows: words(windows),
        },
        E::Forgotten(workspace) => E::Forgotten(workspace.word()),
        E::Cleared => E::Cleared,
        E::ReconcileBegan => E::ReconcileBegan,
        E::ReconcileEnded => E::ReconcileEnded,
        E::RestoreAsked {
            workspace,
            returning,
            current,
            held,
        } => E::RestoreAsked {
            workspace: workspace.word(),
            returning: returning.map(|w| w.word()),
            current: match current {
                Current::Unlaid => Current::Unlaid,
                Current::Laid(candidates) => Current::Laid(
                    (candidates.into_iter())
                        .map(|c| Candidate {
                            window: c.window.word(),
                            in_workspace: c.in_workspace,
                            present: c.present,
                            floating: c.floating,
                        })
                        .collect(),
                ),
            },
            held: words(held),
        },
    }
}

#[cfg(feature = "roc-core")]
fn decided_words(
    decided: rift_core::native_memory::Decided<WindowId, VirtualWorkspaceId>,
) -> rift_core::native_memory::Decided<u64, u64> {
    use rift_core::native_memory::{Decided, Decision};

    use crate::roc_path::Word;
    let words = |windows: Vec<WindowId>| windows.into_iter().map(|w| w.word()).collect();
    match decided {
        Decided::Nothing => Decided::Nothing,
        Decided::Deferred => Decided::Deferred,
        Decided::Restore(d) => Decided::Restore(Decision {
            remove: words(d.remove),
            ordered: words(d.ordered),
            add: words(d.add),
            spent: d.spent,
        }),
        Decided::Waited(waited) => Decided::Waited(waited.into_iter().map(|w| w.word()).collect()),
    }
}

#[cfg(feature = "roc-core")]
fn owned_words(
    owned: rift_core::native_memory::Owned<VirtualWorkspaceId>,
) -> rift_core::native_memory::Owned<u64> {
    use rift_core::native_memory::{Batch, Owned};

    use crate::roc_path::Word;
    Owned {
        held: owned.held.into_iter().map(|w| w.word()).collect(),
        batch: match owned.batch {
            Batch::Idle => Batch::Idle,
            Batch::Collecting(waiting) => {
                Batch::Collecting(waiting.into_iter().map(|w| w.word()).collect())
            }
        },
    }
}

/// The floating state as the engine holds it (engine slice E2).
pub(crate) type RiftFloats =
    rift_core::floats::Floats<WindowId, VirtualWorkspaceId, objc2_core_foundation::CGRect>;

/// An event of the floating state as the engine steps it.
pub(crate) type RiftFloatsEvent = rift_core::floats::FloatsEvent<
    WindowId,
    pid_t,
    VirtualWorkspaceId,
    objc2_core_foundation::CGRect,
>;

/// The floating state (engine slice E2), with its Roc twin under `roc-core`. Flipped (policy
/// (a)): under `roc-core` the Roc core holds the state and the Rust machine is its shadow, read
/// through `Deref` and re-seeded on any difference.
#[derive(Default)]
pub(crate) struct TwinnedFloats {
    rust: RiftFloats,
    #[cfg(feature = "roc-core")]
    twin: FloatsTwin,
}

#[cfg(feature = "roc-core")]
#[derive(Default)]
struct FloatsTwin {
    roc: rift_core_roc::RocFloats,
    steps: u64,
}

impl Deref for TwinnedFloats {
    type Target = RiftFloats;

    fn deref(&self) -> &Self::Target { &self.rust }
}

impl TwinnedFloats {
    /// A machine holding a saved layout's floating state (the Roc twin seeded with the same).
    pub(crate) fn seeded(rust: RiftFloats) -> Self {
        #[cfg(feature = "roc-core")]
        let twin = {
            use rift_core::floats::FloatsEvent;
            let mut twin = FloatsTwin::default();
            let mut seed = |event: RiftFloatsEvent| twin.roc.step(words(event));
            rust.floating().for_each(|w| seed(FloatsEvent::Floated(w)));
            seed(FloatsEvent::FloatingFocus(rust.floating_focus()));
            twin
        };
        let seeded = TwinnedFloats {
            rust,
            #[cfg(feature = "roc-core")]
            twin,
        };
        // Tests compare the seed at once: a load is often not followed by a step.
        #[cfg(all(test, feature = "roc-core"))]
        {
            let mut seeded = seeded;
            let (rust, view) = (seeded.rust_view(), seeded.twin.roc.view());
            if rust != view {
                crate::roc_path::diverged("floats seed", &"a saved layout", &rust, &view);
            }
            return seeded;
        }
        #[allow(unreachable_code)]
        seeded
    }

    /// As [`rift_core::floats::Floats::step`]: both must hold the same state when due.
    pub(crate) fn step(&mut self, event: RiftFloatsEvent) {
        #[cfg(feature = "roc-core")]
        let plain = words(event.clone());
        self.rust.step(event);
        #[cfg(feature = "roc-core")]
        {
            use crate::roc_path::diverged;
            self.twin.roc.step(plain.clone());
            self.twin.steps += 1;
            if cfg!(test) || self.twin.steps.is_multiple_of(VIEW_EVERY) {
                let rust = self.rust_view();
                let view = self.twin.roc.view();
                if rust != view {
                    diverged("floats", &plain, &rust, &view);
                    self.reseed(view);
                }
            }
        }
    }

    /// The Rust machine as words, as the Roc core views it (every part in word order).
    #[cfg(feature = "roc-core")]
    fn rust_view(&self) -> rift_core::floats::Floats<u64, u64, rift_core::plain::Rect> {
        use crate::roc_path::Word;
        let r = &self.rust;
        rift_core::floats::Floats::from_parts(
            r.floating().map(Word::word),
            r.fullscreens().map(|(w, k)| (w.word(), k)),
            r.floating_focus().map(Word::word),
            r.focused().map(Word::word),
            r.frames().map(|(ws, w, f)| (ws.word(), w.word(), plain_rect(f))),
        )
    }

    /// Replace the Rust shadow with the Roc core's state, and count it.
    #[cfg(feature = "roc-core")]
    fn reseed(&mut self, view: rift_core::floats::Floats<u64, u64, rift_core::plain::Rect>) {
        use crate::roc_path::window_of_word;
        self.rust = RiftFloats::from_parts(
            view.floating().filter_map(window_of_word),
            view.fullscreens().filter_map(|(w, k)| Some((window_of_word(w)?, k))),
            view.floating_focus().and_then(window_of_word),
            view.focused().and_then(window_of_word),
            view.frames().filter_map(|(ws, w, f)| {
                Some((workspace_of_word(ws), window_of_word(w)?, cg_rect(f)))
            }),
        );
        let reseeds = FLOATS_RESEEDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        tracing::warn!(
            reseeds,
            "roc-divergence: the floating state's Rust shadow was re-seeded from the Roc core"
        );
    }
}

#[cfg(feature = "roc-core")]
fn plain_rect(f: objc2_core_foundation::CGRect) -> rift_core::plain::Rect {
    rift_core::plain::Rect {
        x: f.origin.x,
        y: f.origin.y,
        w: f.size.width,
        h: f.size.height,
    }
}

#[cfg(feature = "roc-core")]
fn cg_rect(r: rift_core::plain::Rect) -> objc2_core_foundation::CGRect {
    objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(r.x, r.y),
        objc2_core_foundation::CGSize::new(r.w, r.h),
    )
}

/// An event of the floating state as words, as the Roc core takes it.
#[cfg(feature = "roc-core")]
fn words(
    event: RiftFloatsEvent,
) -> rift_core::floats::FloatsEvent<u64, i32, u64, rift_core::plain::Rect> {
    use rift_core::floats::FloatsEvent as E;

    use crate::roc_path::Word;
    match event {
        E::Floated(w) => E::Floated(w.word()),
        E::Unfloated(w) => E::Unfloated(w.word()),
        E::Fullscreen(w, k) => E::Fullscreen(w.word(), k),
        E::FloatingFocus(w) => E::FloatingFocus(w.map(Word::word)),
        E::Focused(w) => E::Focused(w.map(Word::word)),
        E::Replaced { from, to } => E::Replaced {
            from: from.word(),
            to: to.word(),
        },
        E::AppQuit(pid) => E::AppQuit(pid),
        E::FloatingTaken => E::FloatingTaken,
        E::FrameStored {
            workspace,
            window,
            frame,
            if_absent,
        } => E::FrameStored {
            workspace: workspace.word(),
            window: window.word(),
            frame: plain_rect(frame),
            if_absent,
        },
        E::FramesForgotten(w) => E::FramesForgotten(w.word()),
        E::FrameForgotten { workspace, window } => E::FrameForgotten {
            workspace: workspace.word(),
            window: window.word(),
        },
        E::WorkspaceForgotten(ws) => E::WorkspaceForgotten(ws.word()),
    }
}

#[cfg(all(test, feature = "roc-core"))]
mod tests {
    use rift_core::lifecycle::{
        AdmissionFacts, AxKind, LifecycleEvent, Manageability, Minimize, NativeKind, Rule,
    };

    use super::{LIFECYCLE_RESEEDS, TwinnedLifecycle};
    use crate::actor::app::WindowId;
    use crate::model::window_store::Claimant;
    use crate::sys::screen::SpaceId;

    const FACTS: AdmissionFacts = AdmissionFacts {
        manageability: Manageability::Manageable,
        rule: Rule::NoRule,
        ax: AxKind::Ordinary,
        native: NativeKind::Ordinary,
        minimize: Minimize::Shown,
    };

    /// A lifecycle whose Rust shadow lost a window the Roc core still has.
    fn knocked_out_of_step() -> (TwinnedLifecycle, WindowId) {
        let mut lifecycle = TwinnedLifecycle::default();
        let window = WindowId::new(3, 1);
        lifecycle.step(LifecycleEvent::Live { window, facts: FACTS });
        lifecycle.rust.step(LifecycleEvent::Gone(window));
        (lifecycle, window)
    }

    /// Re-seeding gives the Rust shadow exactly the Roc core's windows, and is counted.
    #[test]
    fn a_reseed_makes_the_shadow_the_roc_cores_and_is_counted() {
        let (mut lifecycle, window) = knocked_out_of_step();
        assert!(!lifecycle.manages(window), "the shadow lost it");
        let before = LIFECYCLE_RESEEDS.load(std::sync::atomic::Ordering::Relaxed);
        let roc = lifecycle.twin.roc.view();
        lifecycle.reseed(roc.clone());
        assert_eq!(lifecycle.rust_view(), roc);
        assert!(lifecycle.manages(window), "reads follow the Roc core");
        assert!(LIFECYCLE_RESEEDS.load(std::sync::atomic::Ordering::Relaxed) > before);
    }

    /// Re-seeding gives the constraints' Rust shadow exactly the Roc core's records and app
    /// minimums (a negative pid among them, back from its word), and is counted.
    #[test]
    fn a_constraints_reseed_makes_the_shadow_the_roc_cores_and_is_counted() {
        use rift_core::constraints::{ConstraintsEvent, Observed};
        let mut constraints = super::TwinnedConstraints::default();
        let window = WindowId::new(-4, 1);
        constraints.step(ConstraintsEvent::Observed {
            window,
            app: -4,
            respect_min_size: true,
            observed: Observed {
                resizable: true,
                width: 600.0,
                height: 400.0,
                min: rift_core::layout::Size::reported(200.0, 100.0),
                max_width: 0.0,
                max_height: 0.0,
            },
        });
        // The shadow loses the window the Roc core still has.
        constraints.rust.step(ConstraintsEvent::Retired(window));
        assert_eq!(constraints.get(window), None, "the shadow lost it");
        let before = super::CONSTRAINTS_RESEEDS.load(std::sync::atomic::Ordering::Relaxed);
        let view = constraints.twin.roc.view();
        constraints.reseed(view.clone());
        assert_eq!(constraints.rust_view(), view);
        assert!(constraints.get(window).is_some(), "reads follow the Roc core");
        assert_eq!(
            constraints.app_minimums().map(|(app, _)| app).collect::<Vec<_>>(),
            [-4]
        );
        assert!(super::CONSTRAINTS_RESEEDS.load(std::sync::atomic::Ordering::Relaxed) > before);
    }

    /// Re-seeding gives the display memory's Rust shadow exactly the Roc core's maps, each display
    /// back from its word, and is counted; a seeded memory holds the loaded maps in both.
    #[test]
    fn a_displays_reseed_makes_the_shadow_the_roc_cores_and_is_counted() {
        use rift_core::display_memory::{DisplayEvent, LastSpace};
        let mut displays = super::TwinnedDisplays::seeded(super::RiftDisplays::from_parts(
            [
                (SpaceId::new(1), "a".to_owned()),
                (SpaceId::new(2), "b".to_owned()),
            ],
            [
                ("a".to_owned(), SpaceId::new(3)),
                ("c".to_owned(), SpaceId::new(2)),
            ],
        ));
        assert_eq!(displays.last_space("a"), Some(SpaceId::new(3)));
        displays.step(DisplayEvent::Shown {
            space: SpaceId::new(4),
            display: "d".to_owned(),
            last: LastSpace::Follow,
        });
        // The shadow loses the Space the Roc core still has.
        displays.rust.step(DisplayEvent::Forgotten(SpaceId::new(4)));
        assert_eq!(displays.display_of(SpaceId::new(4)), None, "the shadow lost it");
        let before = super::DISPLAYS_RESEEDS.load(std::sync::atomic::Ordering::Relaxed);
        let view = displays.twin.roc.view();
        displays.reseed(view.clone());
        assert_eq!(displays.rust_view(), view);
        assert_eq!(
            displays.display_of(SpaceId::new(4)).map(String::as_str),
            Some("d")
        );
        assert_eq!(displays.last_space("c"), Some(SpaceId::new(2)));
        assert!(super::DISPLAYS_RESEEDS.load(std::sync::atomic::Ordering::Relaxed) > before);
    }

    /// Re-seeding gives the evictions' Rust shadow exactly the Roc core's records, each display
    /// and workspace back from its word, and is counted.
    #[test]
    fn an_evictions_reseed_makes_the_shadow_the_roc_cores_and_is_counted() {
        use rift_core::evictions::EvictionEvent;
        let mut evictions = super::TwinnedEvictions::default();
        let window = WindowId::new(-4, 1);
        let mut workspaces = slotmap::SlotMap::<super::VirtualWorkspaceId, ()>::with_key();
        let landed = workspaces.insert(());
        evictions.step(EvictionEvent::Evicted {
            window,
            from: "a".to_owned(),
            ordinal: 2,
            landed,
            landed_on: Some("b".to_owned()),
        });
        // The shadow loses the record the Roc core still has.
        evictions.rust.step(EvictionEvent::Forgotten(window));
        assert_eq!(evictions.get(window), None, "the shadow lost it");
        let before = super::EVICTIONS_RESEEDS.load(std::sync::atomic::Ordering::Relaxed);
        let view = evictions.twin.roc.view();
        evictions.reseed(view.clone());
        assert_eq!(evictions.rust_view(), view);
        assert_eq!(
            evictions.ordinal(window, Some(landed), Some(&"a".to_owned())),
            Some(2)
        );
        assert!(super::EVICTIONS_RESEEDS.load(std::sync::atomic::Ordering::Relaxed) > before);
    }

    /// A disagreement brings both sides to what they share: the shadow keeps only the snapshots
    /// the Roc core holds, the Roc core forgets one the shadow has no value for; counted.
    #[test]
    fn a_native_reseed_brings_both_to_what_they_share_and_is_counted() {
        use rift_core::native_memory::{Batch, MemoryEvent};
        let mut native = super::TwinnedNative::<Vec<WindowId>>::default();
        let mut workspaces = slotmap::SlotMap::<super::VirtualWorkspaceId, ()>::with_key();
        let (both, roc_only, rust_only) = (
            workspaces.insert(()),
            workspaces.insert(()),
            workspaces.insert(()),
        );
        let w = WindowId::new(3, 1);
        for workspace in [both, roc_only] {
            native.step(MemoryEvent::Kept { workspace, snapshot: vec![w] });
        }
        // The shadow loses one and gains one the Roc core never saw.
        native.rust.step(MemoryEvent::Forgotten(roc_only));
        native.rust.step(MemoryEvent::Kept {
            workspace: rust_only,
            snapshot: vec![w],
        });
        let before = super::NATIVE_RESEEDS.load(std::sync::atomic::Ordering::Relaxed);
        let view = native.twin.roc.view();
        native.reseed(view);
        let held: Vec<_> = native.snapshots().map(|(workspace, _)| workspace).collect();
        assert_eq!(held, [both]);
        assert_eq!(super::owned_words(native.owned()), native.twin.roc.view());
        assert_eq!(*native.batch(), Batch::Idle);
        assert!(super::NATIVE_RESEEDS.load(std::sync::atomic::Ordering::Relaxed) > before);
    }

    /// An app quitting reaches the Roc core as the same app: its minimum goes in both.
    #[test]
    fn a_constraints_twin_forgets_a_quitting_apps_minimum() {
        use rift_core::constraints::{ConstraintsEvent, Observed};
        let mut constraints = super::TwinnedConstraints::default();
        for app in [4, -4] {
            constraints.step(ConstraintsEvent::Observed {
                window: WindowId::new(app, 1),
                app,
                respect_min_size: true,
                observed: Observed {
                    resizable: true,
                    width: 600.0,
                    height: 400.0,
                    min: rift_core::layout::Size::reported(200.0, 100.0),
                    max_width: 0.0,
                    max_height: 0.0,
                },
            });
        }
        // Every step compares the whole state in tests: a difference panics.
        constraints.step(ConstraintsEvent::AppQuit(4));
        constraints.step(ConstraintsEvent::AppQuit(-4));
        assert_eq!(constraints.app_minimums().count(), 0);
    }

    /// Re-seeding gives the floating state's Rust shadow exactly the Roc core's (a negative pid's
    /// window, a workspace key and a frame back from their words), and is counted.
    #[test]
    fn a_floats_reseed_makes_the_shadow_the_roc_cores_and_is_counted() {
        use objc2_core_foundation::{CGPoint, CGRect, CGSize};
        use rift_core::floats::FloatsEvent;
        let mut floats = super::TwinnedFloats::default();
        let window = WindowId::new(-6, 2);
        let workspace: crate::model::VirtualWorkspaceId =
            slotmap::KeyData::from_ffi((3 << 32) | 5).into();
        let frame = CGRect::new(CGPoint::new(10.0, 20.0), CGSize::new(300.0, 200.0));
        floats.step(FloatsEvent::Floated(window));
        floats.step(FloatsEvent::FrameStored {
            workspace,
            window,
            frame,
            if_absent: false,
        });
        // The shadow loses the window the Roc core still has.
        floats.rust.step(FloatsEvent::Unfloated(window));
        assert!(!floats.is_floating(window), "the shadow lost it");
        let before = super::FLOATS_RESEEDS.load(std::sync::atomic::Ordering::Relaxed);
        let view = floats.twin.roc.view();
        floats.reseed(view.clone());
        assert_eq!(floats.rust_view(), view);
        assert!(floats.is_floating(window));
        assert_eq!(floats.frame(workspace, window), Some(frame));
        assert!(super::FLOATS_RESEEDS.load(std::sync::atomic::Ordering::Relaxed) > before);
    }

    /// A main window machine whose Rust shadow lost the app the Roc core still has.
    fn main_windows_knocked_out_of_step() -> super::TwinnedMainWindows {
        use rift_core::main_window::{MainWindowEvent, Standing};
        let mut mains = super::TwinnedMainWindows::default();
        let window = WindowId::new(5, 1);
        mains.step(MainWindowEvent::Launched {
            pid: 5,
            standing: Standing::Frontmost,
            main_window: Some(window),
        });
        mains.step(MainWindowEvent::GloballyActivated(5));
        mains.rust.step(MainWindowEvent::Terminated(5));
        mains
    }

    /// Re-seeding gives the main window's Rust shadow exactly the Roc core's state, and is
    /// counted.
    #[test]
    fn a_main_window_reseed_makes_the_shadow_the_roc_cores_and_is_counted() {
        let mut mains = main_windows_knocked_out_of_step();
        assert_eq!(mains.main_window(), None, "the shadow lost it");
        let before = super::MAIN_WINDOW_RESEEDS.load(std::sync::atomic::Ordering::Relaxed);
        let view = mains.twin.roc.view();
        mains.reseed(view.clone());
        assert_eq!(mains.rust_view(), view);
        assert_eq!(
            mains.main_window(),
            Some(WindowId::new(5, 1)),
            "reads follow the Roc core"
        );
        assert!(super::MAIN_WINDOW_RESEEDS.load(std::sync::atomic::Ordering::Relaxed) > before);
    }

    /// Tests still panic on a main window difference.
    #[test]
    #[should_panic(expected = "roc-divergence")]
    fn a_main_window_shadow_out_of_step_panics_in_tests() {
        let mut mains = main_windows_knocked_out_of_step();
        mains.step(rift_core::main_window::MainWindowEvent::Deactivated(5));
    }

    /// Joined workspaces whose Rust shadow lost a claim (suppressing focus follows mouse) the Roc
    /// core still holds.
    fn workspaces_knocked_out_of_step() -> (super::TwinnedWorkspaces, WindowId) {
        use rift_core::workspaces::plain::{ClaimFlags, Claimant as Plain, FocusFollowsMouse};
        use rift_core::workspaces::{Destination, Owned, Ownership, WorkspacesEvent};
        let mut workspaces = super::TwinnedWorkspaces::default();
        let workspace = super::workspace_of_word((1 << 32) | 1);
        let window = WindowId::new(6, 2);
        let owner = Claimant::from_plain(Plain {
            owner: 40,
            flags: ClaimFlags {
                focus_follows_mouse: FocusFollowsMouse::Suppressed,
            },
            original: crate::roc_path::Word::word(WindowId::new(6, 1)),
        })
        .unwrap();
        workspaces.step(WorkspacesEvent::Created {
            workspace,
            space: SpaceId::new(9),
        });
        workspaces.step(WorkspacesEvent::Owned {
            window,
            owned: Owned::Owner(Ownership::Claimed {
                owner,
                destination: Destination::Returns(workspace),
            }),
        });
        workspaces
            .rust
            .step(WorkspacesEvent::Owned { window, owned: Owned::Unowned })
            .unwrap();
        (workspaces, window)
    }

    /// Re-seeding gives the joined workspaces' Rust shadow exactly the Roc core's, the claim
    /// whole (flags and original id), and is counted.
    #[test]
    fn a_workspaces_reseed_makes_the_shadow_the_roc_cores_and_is_counted() {
        use rift_core::workspaces::{Owned, Ownership};
        let (mut workspaces, window) = workspaces_knocked_out_of_step();
        assert_eq!(workspaces.owned(window), Owned::Unowned, "the shadow lost it");
        let before = super::WORKSPACES_RESEEDS.load(std::sync::atomic::Ordering::Relaxed);
        let view = workspaces.twin.roc.view();
        workspaces.reseed(view.clone());
        assert_eq!(workspaces.rust_view(), view);
        let Owned::Owner(Ownership::Claimed { owner, .. }) = workspaces.owned(window) else {
            panic!("reads follow the Roc core");
        };
        let Ownership::Claimed { owner: held, .. } = view.owners[0].1 else {
            panic!("the Roc core holds the claim");
        };
        assert_eq!(owner.plain(), held, "the claim whole: owner, flags, original id");
        assert!(super::WORKSPACES_RESEEDS.load(std::sync::atomic::Ordering::Relaxed) > before);
    }

    /// Tests still panic on a workspaces difference.
    #[test]
    #[should_panic(expected = "roc-divergence")]
    fn a_workspaces_shadow_out_of_step_panics_in_tests() {
        use rift_core::workspaces::{Owned, Ownership, WorkspacesEvent};
        let (mut workspaces, _) = workspaces_knocked_out_of_step();
        workspaces.step(WorkspacesEvent::Owned {
            window: WindowId::new(6, 3),
            owned: Owned::Owner(Ownership::Held),
        });
    }

    /// Tests still panic on a difference (a running Rift warns and re-seeds).
    #[test]
    #[should_panic(expected = "roc-divergence")]
    fn a_shadow_out_of_step_panics_in_tests() {
        let (mut lifecycle, window) = knocked_out_of_step();
        lifecycle.step(LifecycleEvent::Changed {
            window,
            change: rift_core::lifecycle::FactChange::Rule(Rule::Ignore),
        });
    }
}
