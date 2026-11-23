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
//! flipped the same way ([`main_window_reseeds`]). The ownerships and the topology are not
//! flipped yet: their Rust machines stand.
//!
//! A twinned machine reads as its Rust machine (`Deref`), and only its own methods change it:
//! the Rust machine's changing methods take `&mut`, which `Deref` does not give, so no change
//! can reach the Rust machine without reaching the twin.

use std::ops::Deref;

use rift_core::lifecycle::{Invalidation, Lifecycle, LifecycleEvent, Revalidation, Transition};
use rift_core::main_window::{MainWindowEvent, MainWindows};
use rift_core::workspaces::{OwnershipEvent, Ownerships, Topology, TopologyEvent};

use crate::actor::app::{WindowId, pid_t};
use crate::model::VirtualWorkspaceId;
use crate::model::window_store::ClaimOwner;
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
        {
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
            return roc.map(|change| match change {
                rift_core::lifecycle::Change::Admitted => Transition::Admitted(window),
                rift_core::lifecycle::Change::Suspended(why) => Transition::Suspended(window, why),
            });
        }
        #[cfg(not(feature = "roc-core"))]
        transition
    }

    /// As [`Lifecycle::invalidate`].
    pub(crate) fn invalidate(&mut self, window: WindowId) -> Invalidation {
        let invalidated = self.rust.invalidate(window);
        #[cfg(feature = "roc-core")]
        {
            use crate::roc_path::Word;
            let roc = self.twin.roc.invalidate(window.word());
            self.stepped("lifecycle invalidate", &window, &invalidated, &roc);
            return roc;
        }
        #[cfg(not(feature = "roc-core"))]
        invalidated
    }

    /// As [`Lifecycle::revalidate`].
    pub(crate) fn revalidate(&mut self, window: WindowId) -> Revalidation {
        let revalidated = self.rust.revalidate(window);
        #[cfg(feature = "roc-core")]
        {
            use crate::roc_path::Word;
            let roc = self.twin.roc.revalidate(window.word());
            self.stepped("lifecycle revalidate", &window, &revalidated, &roc);
            return roc;
        }
        #[cfg(not(feature = "roc-core"))]
        revalidated
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
        if outcome_differs || cfg!(test) || self.twin.steps % VIEW_EVERY == 0 {
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

/// Who places each window (a workspace, a claim, or the user's hand), with its Roc twin under
/// `roc-core`.
#[derive(Default)]
pub(crate) struct TwinnedOwnerships {
    rust: Ownerships<WindowId, VirtualWorkspaceId, ClaimOwner>,
    #[cfg(feature = "roc-core")]
    twin: OwnershipsTwin,
}

#[cfg(feature = "roc-core")]
#[derive(Default)]
struct OwnershipsTwin {
    roc: rift_core_roc::RocOwnerships,
    steps: u64,
}

impl std::fmt::Debug for TwinnedOwnerships {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.rust.fmt(f) }
}

impl Deref for TwinnedOwnerships {
    type Target = Ownerships<WindowId, VirtualWorkspaceId, ClaimOwner>;

    fn deref(&self) -> &Self::Target { &self.rust }
}

impl TwinnedOwnerships {
    /// As [`Ownerships::step`]. The step answers nothing, so what both must agree on after it
    /// is who places the window now; and, when due, every owner.
    pub(crate) fn step(&mut self, event: OwnershipEvent<WindowId, VirtualWorkspaceId, ClaimOwner>) {
        self.rust.step(event);
        #[cfg(feature = "roc-core")]
        {
            use rift_core::workspaces::Owned;

            use crate::roc_path::{Word, diverged};
            let OwnershipEvent::Owned { window, owned } = event;
            let plain = OwnershipEvent::Owned {
                window: window.word(),
                owned: match owned {
                    Owned::Unowned => Owned::Unowned,
                    Owned::Owner(ownership) => Owned::Owner(plain_ownership(ownership)),
                },
            };
            let roc = self.twin.roc.step(plain);
            let rust = match self.rust.owned(window) {
                Owned::Unowned => Owned::Unowned,
                Owned::Owner(ownership) => Owned::Owner(plain_ownership(ownership)),
            };
            if rust != roc {
                diverged("ownerships step", &plain, &rust, &roc);
            }
            self.twin.steps += 1;
            if cfg!(test) || self.twin.steps % VIEW_EVERY == 0 {
                let mut rust: Vec<_> = self
                    .rust
                    .owners()
                    .map(|(window, ownership)| (window.word(), plain_ownership(ownership)))
                    .collect();
                rust.sort_by_key(|&(window, _)| window);
                let roc = self.twin.roc.view();
                if rust != roc {
                    diverged("ownerships owners", &plain, &rust, &roc);
                }
            }
        }
    }
}

/// An ownership with its workspaces and owner as words.
#[cfg(feature = "roc-core")]
fn plain_ownership(
    ownership: rift_core::workspaces::Ownership<VirtualWorkspaceId, ClaimOwner>,
) -> rift_core::workspaces::Ownership<u64, u64> {
    use rift_core::workspaces::{Destination, Ownership};

    use crate::roc_path::Word;
    match ownership {
        Ownership::Assigned(workspace) => Ownership::Assigned(workspace.word()),
        Ownership::Claimed { owner, destination } => Ownership::Claimed {
            owner: owner.word(),
            destination: match destination {
                Destination::Returns(workspace) => Destination::Returns(workspace.word()),
                Destination::Nowhere => Destination::Nowhere,
            },
        },
        Ownership::Held => Ownership::Held,
    }
}

/// The workspaces' topology (the catalog and what each Space shows), with its Roc twin under
/// `roc-core`. Its step answers nothing, so what it comes to is the topology itself, compared
/// after every step, in a running Rift too: it changes only as workspaces are created, moved,
/// removed and switched, and both views together cost about 0.5 µs (measured in release, at
/// 5 to 100 workspaces).
#[derive(Default)]
pub(crate) struct TwinnedTopology {
    rust: Topology<VirtualWorkspaceId, SpaceId>,
    #[cfg(feature = "roc-core")]
    roc: rift_core_roc::RocTopology,
}

impl std::fmt::Debug for TwinnedTopology {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.rust.fmt(f) }
}

impl Deref for TwinnedTopology {
    type Target = Topology<VirtualWorkspaceId, SpaceId>;

    fn deref(&self) -> &Self::Target { &self.rust }
}

impl TwinnedTopology {
    /// As [`Topology::step`].
    pub(crate) fn step(&mut self, event: TopologyEvent<VirtualWorkspaceId, SpaceId>) {
        self.rust.step(event);
        #[cfg(feature = "roc-core")]
        {
            use rift_core::workspaces::Activation;
            use rift_core_roc::workspaces::TopologyView;

            use crate::roc_path::{Word, diverged};
            let word = |workspace: Option<VirtualWorkspaceId>| workspace.map(Word::word);
            let activation = |activation: Activation<VirtualWorkspaceId>| match activation {
                Activation::Showing { current, previous, remembered } => Activation::Showing {
                    current: current.word(),
                    previous: word(previous),
                    remembered: word(remembered),
                },
                Activation::Vacant { remembered } => {
                    Activation::Vacant { remembered: word(remembered) }
                }
            };
            let plain = match event {
                TopologyEvent::Created { workspace, space } => TopologyEvent::Created {
                    workspace: workspace.word(),
                    space: space.word(),
                },
                TopologyEvent::Removed(workspace) => TopologyEvent::Removed(workspace.word()),
                TopologyEvent::Moved { workspace, space } => TopologyEvent::Moved {
                    workspace: workspace.word(),
                    space: space.word(),
                },
                TopologyEvent::Activated { space, activation: shown } => TopologyEvent::Activated {
                    space: space.word(),
                    activation: activation(shown),
                },
                TopologyEvent::Cleared(space) => TopologyEvent::Cleared(space.word()),
            };
            self.roc.step(plain);
            let rust = TopologyView {
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
                    .map(|&(space, shown)| (space.word(), activation(shown)))
                    .collect(),
            };
            let roc = self.roc.view();
            if rust != roc {
                diverged("topology", &plain, &rust, &roc);
            }
        }
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
        {
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
            if answer_differs || cfg!(test) || self.twin.steps % VIEW_EVERY == 0 {
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
            return roc.and_then(crate::roc_path::window_of_word);
        }
        #[cfg(not(feature = "roc-core"))]
        answer
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

#[cfg(all(test, feature = "roc-core"))]
mod tests {
    use rift_core::lifecycle::{
        AdmissionFacts, AxKind, LifecycleEvent, Manageability, Minimize, NativeKind, Rule,
    };

    use super::{LIFECYCLE_RESEEDS, TwinnedLifecycle};
    use crate::actor::app::WindowId;

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
