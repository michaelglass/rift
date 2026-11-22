//! The Roc core's state machines on rift's path (the `roc-core` feature): a machine rift keeps
//! between events has a Roc twin, stepped beside it with every change the owner makes. Each
//! step must come to the same thing in both (checked on every step), and the twin must keep
//! the same state (its whole view checked on every step in tests, every [`VIEW_EVERY`] steps
//! in a running Rift). The Rust machine stands; a difference is a `roc-divergence:` (tests
//! panic, a running Rift warns). Without `roc-core` a twinned machine is only its Rust machine.
//!
//! A twinned machine reads as its Rust machine (`Deref`), and only its own methods change it:
//! the Rust machine's changing methods take `&mut`, which `Deref` does not give, so no change
//! can reach the Rust machine without reaching the twin.

use std::ops::Deref;

use rift_core::lifecycle::{Invalidation, Lifecycle, LifecycleEvent, Revalidation, Transition};
use rift_core::workspaces::{OwnershipEvent, Ownerships, Topology, TopologyEvent};

use crate::actor::app::WindowId;
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

/// The lifecycle (every window with AX state and its presence), with its Roc twin under
/// `roc-core`.
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
        }
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
        }
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
        }
        revalidated
    }

    /// As [`Lifecycle::carry_invalidation`], step by step so the twin takes each.
    pub(crate) fn carry_invalidation(&mut self, from: WindowId, to: WindowId) {
        if self.revalidate(from) == Revalidation::Revalidated {
            self.invalidate(to);
        }
    }

    /// One step taken by both: its outcomes must agree, and, when due, their windows.
    #[cfg(feature = "roc-core")]
    fn stepped<A: PartialEq + std::fmt::Debug>(
        &mut self,
        what: &str,
        question: &impl std::fmt::Debug,
        rust: &A,
        roc: &A,
    ) {
        use crate::roc_path::{Word, diverged};
        if rust != roc {
            diverged(what, question, rust, roc);
        }
        self.twin.steps += 1;
        if cfg!(test) || self.twin.steps % VIEW_EVERY == 0 {
            let mut rust: Vec<_> = self
                .rust
                .presences()
                .map(|(window, presence)| (window.word(), presence))
                .collect();
            rust.sort_by_key(|&(window, _)| window);
            let roc = self.twin.roc.view();
            if rust != roc {
                diverged("lifecycle windows", question, &rust, &roc);
            }
        }
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
