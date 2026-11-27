//! What the layout engine remembers of displays, kept between events: the Roc twin
//! (`RocDisplayMemory`) against the Rust machine (`rift_core::display_memory::DisplayMemory`),
//! on random runs of events. Runs reuse a few Spaces and displays, so Spaces show, hide, move
//! between displays, are remapped onto each other and forgotten, with last Spaces followed and
//! held. Every step must answer the same change and leave the same whole state.

use rift_core::display_memory::{DisplayEvent, DisplayMemory, LastSpace};
use rift_core_roc::RocDisplayMemory;

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

    fn space(&mut self) -> u64 { 100 + self.below(6) }

    fn display(&mut self) -> u64 { self.below(3) }

    fn event(&mut self) -> DisplayEvent<u64, u64> {
        match self.below(12) {
            0..=5 => DisplayEvent::Shown {
                space: self.space(),
                display: self.display(),
                last: if self.below(4) == 0 {
                    LastSpace::Hold
                } else {
                    LastSpace::Follow
                },
            },
            6 | 7 => DisplayEvent::Hidden(self.space()),
            8..=10 => DisplayEvent::Remapped {
                from: self.space(),
                to: self.space(),
            },
            _ => DisplayEvent::Forgotten(self.space()),
        }
    }
}

#[test]
fn the_roc_display_memory_answers_and_keeps_state_as_the_rust_one_does() {
    let runs: u64 = std::env::var("RIFT_DISPLAYS_RUNS").map_or(2_000, |n| n.parse().unwrap());
    let mut rng = Rng(0xD15_0000_0E3A);
    let (mut steps, mut changed, mut remembered) = (0, 0, 0);
    for run in 0..runs {
        let mut rust = DisplayMemory::<u64, u64>::default();
        let mut roc = RocDisplayMemory::default();
        let mut history = Vec::new();
        for step in 0..40 {
            let event = rng.event();
            history.push(event);
            let expected = rust.step(event);
            assert_eq!(
                roc.step(event),
                expected,
                "run {run} step {step}: whether the memory changed after {history:#?}"
            );
            assert_eq!(
                roc.view(),
                rust,
                "run {run} step {step}: the state after {history:#?}"
            );
            steps += 1;
            changed += u64::from(expected);
            remembered += rust.last_spaces().count() as u64;
        }
    }
    assert!(
        changed > 0 && remembered > 0,
        "no change or no last Space exercised"
    );
    eprintln!(
        "display memory: {steps} steps identical ({changed} changed it), state compared after each"
    );
}

/// A step updates the maps in place: with 300 Spaces shown on 300 displays, no step allocates
/// more than a sliver of the tables.
#[test]
fn the_roc_display_memory_steps_without_copying_a_table() {
    let mut rng = Rng(0xD15_0000_0E3B);
    let mut rust = DisplayMemory::<u64, u64>::default();
    let mut roc = RocDisplayMemory::default();
    for i in 0..300 {
        let event = DisplayEvent::Shown {
            space: 1000 + i,
            display: i,
            last: LastSpace::Follow,
        };
        rust.step(event);
        roc.step(event);
    }
    let bound = 2048;
    for step in 0..3000 {
        let space = 1000 + rng.below(300);
        let display = rng.below(300);
        // Keep the tables full: only shows, holds and no-op remaps.
        let event = match rng.below(3) {
            0 => DisplayEvent::Shown {
                space,
                display,
                last: LastSpace::Follow,
            },
            1 => DisplayEvent::Shown {
                space,
                display,
                last: LastSpace::Hold,
            },
            _ => DisplayEvent::Remapped { from: space, to: space },
        };
        let expected = rust.step(event);
        rift_core_roc::take_largest_allocation();
        let answered = roc.step(event);
        let largest = rift_core_roc::take_largest_allocation();
        assert_eq!(answered, expected);
        assert!(
            largest < bound,
            "step {step}: {event:?} allocated {largest} bytes (bound {bound})"
        );
    }
    assert_eq!(roc.view(), rust);
}
