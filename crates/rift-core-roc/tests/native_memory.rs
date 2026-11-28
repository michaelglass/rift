//! The native memory, kept between events: the Roc twin (`RocNative`), which owns only which
//! workspaces hold a snapshot and the reconcile batch, against the Rust machine
//! (`rift_core::native_memory::NativeMemory`) holding each snapshot as its list of windows. Each
//! event crosses as the Rust machine reads it before stepping (`crossing`: the windows it reads,
//! computed from the values). Runs reuse a few workspaces and windows, so snapshots are kept,
//! edited, emptied, forgotten and restored, partly or wholly, inside and outside a batch. Every
//! step must answer the same and leave the same owned state.

use rift_core::native_memory::{Candidate, Current, MemoryEvent, NativeMemory};
use rift_core_roc::{AllocationCount, RocNative};

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 { self.next() % n }

    fn chance(&mut self, percent: u64) -> bool { self.below(100) < percent }

    fn workspace(&mut self) -> u64 { 1 + self.below(4) }

    fn window(&mut self) -> u64 { 10 + self.below(6) }

    /// Some windows, each at most once, possibly none.
    fn windows(&mut self) -> Vec<u64> {
        let mut windows: Vec<u64> = (0..self.below(5)).map(|_| self.window()).collect();
        windows.dedup();
        windows.sort();
        windows.dedup();
        windows
    }

    fn current(&mut self) -> Current<u64> {
        if self.chance(10) {
            return Current::Unlaid;
        }
        Current::Laid(
            (0..self.below(6))
                .map(|_| Candidate {
                    window: self.window(),
                    in_workspace: self.chance(80),
                    present: self.chance(85),
                    floating: self.chance(10),
                })
                .collect(),
        )
    }

    fn event(&mut self) -> MemoryEvent<u64, u64, Vec<u64>> {
        match self.below(20) {
            0..=4 => MemoryEvent::Kept {
                workspace: self.workspace(),
                snapshot: self.windows(),
            },
            5..=7 => MemoryEvent::Edited {
                workspace: self.workspace(),
                snapshot: self.windows(),
            },
            8 => MemoryEvent::Forgotten(self.workspace()),
            9 if self.chance(30) => MemoryEvent::Cleared,
            9 | 10 => MemoryEvent::ReconcileBegan,
            11 | 12 => MemoryEvent::ReconcileEnded,
            _ => MemoryEvent::RestoreAsked {
                workspace: self.workspace(),
                returning: if self.chance(30) {
                    Some(self.window())
                } else {
                    None
                },
                current: self.current(),
            },
        }
    }
}

#[test]
fn the_roc_native_memory_answers_and_owns_what_the_rust_one_does() {
    let runs: u64 = std::env::var("RIFT_NATIVE_RUNS").map_or(2_000, |n| n.parse().unwrap());
    let mut rng = Rng(0x0A71_0000_0E3C);
    let before = AllocationCount::now();
    let (mut steps, mut restored, mut spent) = (0, 0, 0);
    for run in 0..runs {
        let mut rust = NativeMemory::<u64, Vec<u64>>::default();
        let mut roc = RocNative::default();
        let mut history = Vec::new();
        for step in 0..40 {
            let event = rng.event();
            history.push(event.clone());
            let crossing = rust.crossing(&event);
            let expected = rust.step(event).decided();
            assert_eq!(
                roc.step(crossing),
                expected,
                "run {run} step {step}: the answer after {history:#?}"
            );
            assert_eq!(
                roc.view(),
                rust.owned(),
                "run {run} step {step}: what it owns after {history:#?}"
            );
            steps += 1;
            if let rift_core::native_memory::Decided::Restore(plan) = &expected {
                restored += 1;
                spent += u64::from(plan.spent);
            }
        }
    }
    assert_eq!(
        AllocationCount::now().live(),
        before.live(),
        "Roc allocations left live"
    );
    assert!(
        restored > 0 && spent > 0,
        "no restore or no spent snapshot exercised"
    );
    eprintln!("native memory: {steps} steps identical ({restored} restores, {spent} spent)");
}

/// A step updates the held set in place: with 300 workspaces holding a snapshot, no step
/// allocates more than a sliver of it.
#[test]
fn the_roc_native_memory_steps_without_copying_its_set() {
    let mut rust = NativeMemory::<u64, Vec<u64>>::default();
    let mut roc = RocNative::default();
    for workspace in 0..300 {
        let event = MemoryEvent::Kept {
            workspace,
            snapshot: vec![1, 2],
        };
        roc.step(rust.crossing(&event));
        rust.step(event);
    }
    let mut rng = Rng(0x0A71_0000_0E3D);
    let bound = 2048;
    for step in 0..2000 {
        let workspace = rng.below(300);
        // Keep the set full: partial restores and non-empty edits only.
        let event = if rng.chance(50) {
            MemoryEvent::Edited {
                workspace,
                snapshot: vec![1, 2],
            }
        } else {
            MemoryEvent::RestoreAsked {
                workspace,
                returning: None,
                current: Current::Laid(vec![Candidate {
                    window: 1,
                    in_workspace: true,
                    present: true,
                    floating: false,
                }]),
            }
        };
        let crossing = rust.crossing(&event);
        let expected = rust.step(event.clone()).decided();
        rift_core_roc::take_largest_allocation();
        let got = roc.step(crossing);
        let largest = rift_core_roc::take_largest_allocation();
        assert_eq!(got, expected);
        assert!(
            largest < bound,
            "step {step}: {event:?} allocated {largest} bytes (bound {bound})"
        );
    }
    assert_eq!(roc.view(), rust.owned());
}
