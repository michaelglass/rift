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
