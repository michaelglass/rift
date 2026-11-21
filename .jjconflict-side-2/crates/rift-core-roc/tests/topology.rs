//! The Roc core's topology decisions against the Rust core's (`rift_core::topology`), on random
//! questions: the answers must be identical word for word. The questions lean on the edges:
//! empty and repeated Spaces, Spaces that are both live and observable, displays with no id,
//! remaps of Spaces that stayed, windows tracked twice or not at all.

mod facts;

use facts::{Coin, rust, typed};
use rift_core::plain::{WindowId, wire};
use rift_core::topology::plain::{Admit, Candidates, Discovered, Discovery};
use rift_core::topology::{DiscoveryStanding, Displays, Incoming, MissionControl, Standing};

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

impl Coin for Rng {
    fn chance(&mut self, percent: u64) -> bool { Rng::chance(self, percent) }
}

#[test]
fn the_roc_core_finds_a_discovered_windows_space_as_the_rust_core_does() {
    typed(
        "topology",
        "discovered-space",
        Rng(0xD15C_0BE7_ED5A_ACE5),
        |rng, blank| {
            let standing = [
                Standing::RecoveringFullscreen,
                Standing::Untracked,
                Standing::Tracked,
            ][rng.below(3) as usize];
            let live = rng.ids(3);
            let observable = rng.ids(3);
            let (recorded, resolved) = (rng.maybe(), rng.maybe());
            let question = Discovered {
                standing,
                observable: blank
                    .answers(rng, live.iter().copied(), |space| observable.contains(&space)),
                live: blank.fact(rng, live),
                recorded: blank.fact(rng, recorded),
                resolved: blank.fact(rng, resolved),
            };
            let roc = rust(
                rift_core_roc::topology::discovered_space(question.clone()),
                wire::encode_discovered_space,
            );
            let rust = rust(question.answer(), wire::encode_discovered_space);
            (format!("{question:?}"), roc, rust)
        },
    );
}

#[test]
fn the_roc_core_gives_a_reported_window_its_discovery_space_as_the_rust_core_does() {
    typed(
        "topology",
        "discovery-spaces",
        Rng(0x0D15_C0FE_5A5A_1234),
        |rng, blank| {
            let by_frame = rng.maybe();
            let question = Discovery {
                standing: [
                    DiscoveryStanding::FullscreenRecorded,
                    DiscoveryStanding::Placeholder,
                    DiscoveryStanding::Admissible,
                ][rng.below(3) as usize],
                native: rng.maybe(),
                assigned: rng.maybe(),
                by_frame: blank.fact(rng, by_frame),
            };
            let roc = rust(
                rift_core_roc::topology::discovery_spaces(question.clone()),
                wire::encode_discovery_spaces,
            );
            let rust = rust(question.answer(), wire::encode_discovery_spaces);
            (format!("{question:?}"), roc, rust)
        },
    );
}

#[test]
fn the_roc_core_admits_a_space_snapshot_as_the_rust_core_does() {
    // It asks for no fact: both passes run on the whole question.
    typed("topology", "admission", Rng(0xAD31_75A9_5407_0001), |rng, _| {
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
        let roc = Ok(wire::encode_admission(
            rift_core_roc::topology::admit_space_snapshot(question.clone()),
        ));
        let rust = Ok(wire::encode_admission(question.answer()));
        (format!("{question:?}"), roc, rust)
    });
}

#[test]
fn the_roc_core_picks_app_rule_candidates_as_the_rust_core_does() {
    typed(
        "topology",
        "app-rule-candidates",
        Rng(0xCA4D_1DA7_E500_0042),
        |rng, blank| {
            let window_spaces: Vec<(u64, u64)> =
                (0..rng.below(7)).map(|_| (rng.id(), rng.id())).collect();
            // A window-server id is tracked as the first listed window, or not at all.
            let listed: Vec<(u64, WindowId)> =
                (0..rng.below(7)).map(|_| (rng.id(), rng.window())).collect();
            let tracked = |server: u64| {
                listed.iter().find(|(id, _)| *id == server).map(|(_, window)| *window)
            };
            let reconcilable: Vec<WindowId> = (0..rng.below(5)).map(|_| rng.window()).collect();
            let windows: Vec<WindowId> = listed.iter().map(|(_, window)| *window).collect();
            let question = Candidates {
                activated: rng.ids(3),
                tracked: blank.answers(
                    rng,
                    window_spaces.iter().map(|(server, _)| *server),
                    tracked,
                ),
                reconcilable: blank.answers(rng, windows, |window| reconcilable.contains(&window)),
                window_spaces,
            };
            let roc = rust(
                rift_core_roc::topology::app_rule_candidates(question.clone()),
                wire::encode_app_windows,
            );
            let rust = rust(question.answer(), wire::encode_app_windows);
            (format!("{question:?}"), roc, rust)
        },
    );
}
