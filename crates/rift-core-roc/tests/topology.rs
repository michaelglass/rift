//! The Roc core's topology decisions against the Rust core's (`rift_core::topology`), on random
//! questions: the answers must be identical word for word. The questions lean on the edges:
//! empty and repeated Spaces, Spaces that are both live and observable, displays with no id,
//! remaps of Spaces that stayed, windows tracked twice or not at all.

use rift_core::plain::{WindowId, wire};
use rift_core::topology::plain::{Admit, Candidates, Discovered, Discovery};
use rift_core::topology::{DiscoveryStanding, Displays, Incoming, MissionControl, Standing};
use rift_core_roc::{AllocationCount, tile_words};

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

    /// One of a few small ids, so they collide, and now and then an extreme one.
    fn id(&mut self) -> u64 {
        match self.below(20) {
            0 => u64::MAX,
            1 => 0,
            _ => 1 + self.below(6),
        }
    }

    fn ids(&mut self, most: u64) -> Vec<u64> {
        (0..self.below(most + 1)).map(|_| self.id()).collect()
    }

    fn maybe(&mut self) -> Option<u64> { self.chance(50).then(|| self.id()) }

    fn window(&mut self) -> WindowId {
        WindowId {
            pid: [1, 2, 3, -1, i32::MAX, i32::MIN][self.below(6) as usize],
            idx: [1, 2, 3, u32::MAX][self.below(4) as usize],
        }
    }

    fn displays(&mut self) -> Displays<u64, u64> {
        Displays {
            screens: (0..self.below(3)).map(|_| self.chance(80).then(|| self.below(3))).collect(),
            spaces: (0..self.below(4)).map(|_| (self.below(3), self.ids(4))).collect(),
        }
    }
}

/// Runs `cases` random questions through both cores; `ask` makes one and returns its words
/// and the Rust core's answer as words.
fn differential(name: &str, seed: u64, mut ask: impl FnMut(&mut Rng) -> (Vec<u64>, Vec<u64>)) {
    let cases: u64 = std::env::var("RIFT_TOPOLOGY_CASES").map_or(20_000, |n| n.parse().unwrap());
    let mut rng = Rng(seed);
    let before = AllocationCount::now();
    for case in 0..cases {
        let (question, rust) = ask(&mut rng);
        let roc = tile_words(&question);
        assert_eq!(roc, Ok(rust), "case {case}: {name} {question:?}");
    }
    let after = AllocationCount::now();
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    eprintln!("topology: {cases} {name} questions answered identically");
}

#[test]
fn the_roc_core_finds_a_discovered_windows_space_as_the_rust_core_does() {
    differential("discovered-space", 0xD15C_0BE7_ED5A_ACE5, |rng| {
        let question = Discovered {
            standing: [
                Standing::RecoveringFullscreen,
                Standing::Untracked,
                Standing::Tracked,
            ][rng.below(3) as usize],
            live: rng.ids(3),
            observable: rng.ids(3),
            recorded: rng.maybe(),
            resolved: rng.maybe(),
        };
        let rust = wire::encode_discovered_space(question.answer());
        (wire::encode_discovered(question), rust)
    });
}

#[test]
fn the_roc_core_gives_a_reported_window_its_discovery_space_as_the_rust_core_does() {
    differential("discovery-spaces", 0x0D15_C0FE_5A5A_1234, |rng| {
        let question = Discovery {
            standing: [
                DiscoveryStanding::FullscreenRecorded,
                DiscoveryStanding::Placeholder,
                DiscoveryStanding::Admissible,
            ][rng.below(3) as usize],
            native: rng.maybe(),
            assigned: rng.maybe(),
            by_frame: rng.maybe(),
        };
        let rust = wire::encode_discovery_spaces(question.answer());
        (wire::encode_discovery(question), rust)
    });
}

#[test]
fn the_roc_core_admits_a_space_snapshot_as_the_rust_core_does() {
    differential("admission", 0xAD31_75A9_5407_0001, |rng| {
        let question = Admit {
            mission_control: if rng.chance(10) {
                MissionControl::Open
            } else {
                MissionControl::Closed
            },
            current: rng.displays(),
            incoming: Incoming {
                displays: rng.displays(),
                remaps: (0..rng.below(3)).map(|_| (rng.id(), rng.id())).collect(),
                display_set_changed: rng.chance(50),
                force_refresh: rng.chance(30),
                membership_complete: rng.chance(60),
            },
        };
        let rust = wire::encode_admission(question.answer());
        (wire::encode_admit(question), rust)
    });
}

#[test]
fn the_roc_core_picks_app_rule_candidates_as_the_rust_core_does() {
    differential("app-rule-candidates", 0xCA4D_1DA7_E500_0042, |rng| {
        let question = Candidates {
            window_spaces: (0..rng.below(7)).map(|_| (rng.id(), rng.id())).collect(),
            activated: rng.ids(3),
            tracked: (0..rng.below(7)).map(|_| (rng.id(), rng.window())).collect(),
            reconcilable: (0..rng.below(5)).map(|_| rng.window()).collect(),
        };
        let rust = wire::encode_app_windows(question.answer());
        (wire::encode_candidates(question), rust)
    });
}
