//! Where a display's departure moved windows from, kept between events: the Roc twin
//! (`RocEvictions`) against the Rust machine (`rift_core::evictions::Evictions`), on random runs
//! of events. Runs reuse a few windows of a few apps (one a negative pid), displays and
//! workspaces, so windows are evicted (some landing on the display they left), forgotten, take
//! new identities, and apps quit. Every step must leave the same records.

use rift_core::evictions::{EvictionEvent, Evictions};
use rift_core_roc::RocEvictions;

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

    fn pid(&mut self) -> i32 { [1, 2, -1][self.below(3) as usize] }

    /// A window of one of the apps, as a word: its pid in the high half.
    fn window(&mut self) -> u64 {
        let pid = self.pid();
        (u64::from(pid as u32) << 32) | (1 + self.below(3))
    }

    fn display(&mut self) -> u64 { self.below(3) }

    fn event(&mut self) -> EvictionEvent<u64, i32, u64, u64> {
        match self.below(10) {
            0..=4 => EvictionEvent::Evicted {
                window: self.window(),
                from: self.display(),
                ordinal: self.below(4),
                landed: 50 + self.below(3),
                landed_on: if self.below(4) == 0 {
                    None
                } else {
                    Some(self.display())
                },
            },
            5 | 6 => EvictionEvent::Forgotten(self.window()),
            7 | 8 => EvictionEvent::Replaced {
                from: self.window(),
                to: self.window(),
            },
            _ => EvictionEvent::AppQuit(self.pid()),
        }
    }
}

#[test]
fn the_roc_evictions_keep_records_as_the_rust_ones_do() {
    let runs: u64 = std::env::var("RIFT_EVICTIONS_RUNS").map_or(2_000, |n| n.parse().unwrap());
    let mut rng = Rng(0xE71C_0000_0E3B);
    let (mut steps, mut held) = (0, 0);
    for run in 0..runs {
        let mut rust = Evictions::<u64, u64, u64>::default();
        let mut roc = RocEvictions::default();
        let mut history = Vec::new();
        for step in 0..40 {
            let event = rng.event();
            history.push(event);
            rust.step(event);
            roc.step(event);
            assert_eq!(
                roc.view(),
                rust,
                "run {run} step {step}: the records after {history:#?}"
            );
            steps += 1;
            held += rust.records().count() as u64;
        }
    }
    assert!(held > 0, "no record exercised");
    eprintln!("evictions: {steps} steps identical, records compared after each");
}

/// A step updates the records in place: with 300 windows evicted, no step allocates more than a
/// sliver of the table.
#[test]
fn the_roc_evictions_step_without_copying_a_table() {
    let mut rng = Rng(0xE71C_0000_0E3C);
    let mut rust = Evictions::<u64, u64, u64>::default();
    let mut roc = RocEvictions::default();
    let window = |i: u64| (5 << 32) | i;
    let evicted = |i: u64, ordinal: u64| EvictionEvent::Evicted {
        window: window(i),
        from: 1,
        ordinal,
        landed: 50,
        landed_on: None,
    };
    for i in 0..300 {
        rust.step(evicted(i, 0));
        roc.step(evicted(i, 0));
    }
    let bound = 2048;
    for step in 0..3000 {
        let i = rng.below(300);
        // Keep the table full: re-evictions and self-replacements only.
        let event = if rng.below(2) == 0 {
            evicted(i, rng.below(4))
        } else {
            EvictionEvent::Replaced { from: window(i), to: window(i) }
        };
        rust.step(event);
        rift_core_roc::take_largest_allocation();
        roc.step(event);
        let largest = rift_core_roc::take_largest_allocation();
        assert!(
            largest < bound,
            "step {step}: {event:?} allocated {largest} bytes (bound {bound})"
        );
    }
    assert_eq!(roc.view(), rust);
}
