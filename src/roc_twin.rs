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

use rift_core::lifecycle::{Lifecycle, LifecycleEvent, Transition};

use crate::actor::app::WindowId;

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
    pub(crate) fn invalidate(&mut self, window: WindowId) -> bool {
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
    pub(crate) fn revalidate(&mut self, window: WindowId) -> bool {
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
        if self.revalidate(from) {
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
