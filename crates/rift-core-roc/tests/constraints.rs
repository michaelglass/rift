//! Window size constraints, kept between events: the Roc twin (`RocSizes`) against the Rust
//! machine (`rift_core::constraints::Constraints`), on random runs of events. Runs reuse a few
//! windows and a few pids (one of them negative),
//! report sizes plain, zero, negative, infinite or NaN, and observe rule windows with and without
//! a minimum, so an app's last minimum is taken, replaced and forgotten. Every step must answer
//! the same change, and leave the same whole state (every record, every app minimum, the
//! revision).

use rift_core::constraints::{Constraints, ConstraintsEvent, Observed};
use rift_core::layout::Size;
use rift_core_roc::{AllocationCount, RocSizes};

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

    /// One of a few pids: small ones and a negative one.
    fn pid(&mut self) -> i32 { [1, 2, 3, -1][self.below(4) as usize] }

    /// One of a few windows, as words.
    fn window(&mut self) -> u64 {
        [(1 << 32) | 1, (1 << 32) | 2, (2 << 32) | 1, 7, 9][self.below(5) as usize]
    }

    /// A length as an app may report it: mostly plain, sometimes 0, negative, infinite or NaN.
    fn length(&mut self) -> f64 {
        match self.below(12) {
            0 => 0.0,
            1 => -((self.below(500) + 1) as f64),
            2 => f64::INFINITY,
            3 => f64::NAN,
            _ => (self.below(1600) as f64) / 2.0,
        }
    }

    fn observed(&mut self) -> Observed {
        Observed {
            resizable: self.chance(70),
            width: self.length(),
            height: self.length(),
            min: if self.chance(50) {
                Size::reported(self.length(), self.length())
            } else {
                None
            },
            max_width: if self.chance(60) { 0.0 } else { self.length() },
            max_height: if self.chance(60) { 0.0 } else { self.length() },
        }
    }

    fn event(&mut self) -> ConstraintsEvent<u64, i32> {
        match self.below(20) {
            0..=7 => ConstraintsEvent::Observed {
                window: self.window(),
                app: self.pid(),
                respect_min_size: self.chance(50),
                observed: self.observed(),
            },
            8..=10 => ConstraintsEvent::WidthSet {
                window: self.window(),
                width: if self.chance(30) {
                    400.0
                } else {
                    self.length()
                },
            },
            11..=13 => ConstraintsEvent::WidthPreserved {
                window: self.window(),
                observed: self.observed(),
                width: self.length(),
            },
            14 | 15 => ConstraintsEvent::Retired(self.window()),
            16 | 17 => ConstraintsEvent::Replaced {
                from: self.window(),
                to: self.window(),
            },
            _ => ConstraintsEvent::AppQuit(self.pid()),
        }
    }
}

/// The Rust machine as the Roc twin views it: records and minimums in order.
fn word_ordered(rust: &Constraints<u64, i32>) -> Constraints<u64, i32> {
    Constraints::from_parts(rust.windows(), rust.app_minimums(), rust.revision())
}

#[test]
fn the_roc_constraints_answer_and_keep_state_as_the_rust_ones_do() {
    let runs: u64 = std::env::var("RIFT_CONSTRAINTS_RUNS").map_or(2_000, |n| n.parse().unwrap());
    let mut rng = Rng(0xC0_5741_0000_0E44);
    let before = AllocationCount::now();
    let (mut steps, mut changed, mut minimums) = (0, 0, 0);
    for run in 0..runs {
        let mut rust = Constraints::<u64, i32>::default();
        let mut roc = RocSizes::default();
        let mut history = Vec::new();
        for step in 0..40 {
            let event = rng.event();
            history.push(event);
            let expected = rust.step(event);
            assert_eq!(
                roc.step(event),
                expected,
                "run {run} step {step}: whether the records changed after {history:#?}"
            );
            assert_eq!(
                roc.view(),
                word_ordered(&rust),
                "run {run} step {step}: the state after {history:#?}"
            );
            steps += 1;
            changed += u64::from(expected);
            minimums += rust.app_minimums().count() as u64;
        }
    }
    assert_eq!(
        AllocationCount::now().live(),
        before.live(),
        "Roc allocations left live"
    );
    assert!(
        changed > 0 && minimums > 0,
        "no change or no app minimum exercised"
    );
    eprintln!(
        "constraints: {steps} steps identical ({changed} changed the records), state compared after each"
    );
}

/// A step updates the records in place: with 256 windows recorded (a table of some 20 KB), no
/// step allocates more than a sliver of it.
#[test]
fn the_roc_constraints_step_without_copying_a_table() {
    let mut rng = Rng(0xC0_5741_0000_0E45);
    let windows: Vec<u64> = (0..256).map(|i| (3 << 32) | i).collect();
    let mut rust = Constraints::<u64, i32>::default();
    let mut roc = RocSizes::default();
    for &window in &windows {
        let event = ConstraintsEvent::Observed {
            window,
            app: 1,
            respect_min_size: false,
            observed: rng.observed(),
        };
        rust.step(event);
        roc.step(event);
    }
    let bound = 2048;
    for step in 0..3000 {
        let window = windows[rng.below(256) as usize];
        let event = match rng.event() {
            ConstraintsEvent::Observed {
                app,
                respect_min_size,
                observed,
                ..
            } => ConstraintsEvent::Observed {
                window,
                app,
                respect_min_size,
                observed,
            },
            ConstraintsEvent::WidthSet { width, .. } => {
                ConstraintsEvent::WidthSet { window, width }
            }
            ConstraintsEvent::WidthPreserved { observed, width, .. } => {
                ConstraintsEvent::WidthPreserved { window, observed, width }
            }
            // Keep the table full: a retired window comes back observed.
            ConstraintsEvent::Retired(_) | ConstraintsEvent::Replaced { .. } => {
                ConstraintsEvent::Observed {
                    window,
                    app: 2,
                    respect_min_size: true,
                    observed: rng.observed(),
                }
            }
            quit @ ConstraintsEvent::AppQuit(_) => quit,
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
    assert_eq!(roc.view(), word_ordered(&rust));
}
