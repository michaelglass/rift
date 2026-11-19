//! The Roc core's workspace decisions against the Rust core's (`rift_core::workspaces`), on
//! random questions: the answers must be identical word for word. Catalogs are small and their
//! Spaces few, the current workspace is usually in them, names repeat, and visible sets overlap,
//! so steps cross, wrap and stop. Topology event runs reuse a few workspaces and Spaces, so
//! moves and removals mostly find their workspace, and Spaces are activated again, cleared and
//! activated anew.

use rift_core::lifecycle::WorkspaceMode;
use rift_core::plain::wire;
use rift_core::workspaces::plain::{
    PreviousQuestion, ResolveQuestion, StepQuestion, TopologyQuestion,
};
use rift_core::workspaces::{Activation, Selector, SkipEmpty, Step, TopologyEvent, Wrapping};
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

    fn mode(&mut self) -> WorkspaceMode {
        if self.chance(70) {
            WorkspaceMode::Global
        } else {
            WorkspaceMode::PerDisplay
        }
    }

    fn space(&mut self) -> u64 { [10, 20, 30][self.below(3) as usize] }

    fn visible(&mut self) -> Vec<u64> {
        [10, 20, 30].into_iter().filter(|_| self.chance(60)).collect()
    }

    /// Distinct workspace ids, each on a Space, in a random catalog order.
    fn catalog(&mut self) -> Vec<(u64, u64)> {
        let mut ids: Vec<u64> = (1..=6).filter(|_| self.chance(70)).collect();
        for i in (1..ids.len()).rev() {
            let j = self.below(i as u64 + 1) as usize;
            ids.swap(i, j);
        }
        ids.into_iter().map(|id| (id, self.space())).collect()
    }
}

fn differential(name: &str, seed: u64, mut ask: impl FnMut(&mut Rng) -> (Vec<u64>, Vec<u64>)) {
    let cases: u64 = std::env::var("RIFT_WORKSPACES_CASES").map_or(20_000, |n| n.parse().unwrap());
    let mut rng = Rng(seed);
    let before = AllocationCount::now();
    for case in 0..cases {
        let (question, rust) = ask(&mut rng);
        assert_eq!(
            tile_words(&question),
            Ok(rust),
            "case {case}: {name} {question:?}"
        );
    }
    let after = AllocationCount::now();
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    eprintln!("workspaces: {cases} {name} questions answered identically");
}

#[test]
fn the_roc_core_steps_through_workspaces_as_the_rust_core_does() {
    differential("step", 0x57E9_0000_0000_0029, |rng| {
        let catalog = rng.catalog();
        let current = match (rng.chance(85), catalog.len()) {
            (true, n) if n > 0 => catalog[rng.below(n as u64) as usize].0,
            _ => 1 + rng.below(7),
        };
        let question = StepQuestion {
            mode: rng.mode(),
            space: rng.space(),
            visible: if rng.chance(15) {
                vec![]
            } else {
                rng.visible()
            },
            current,
            way: if rng.chance(50) {
                Step::Next
            } else {
                Step::Previous
            },
            skip_empty: if rng.chance(40) {
                SkipEmpty::Skip
            } else {
                SkipEmpty::Keep
            },
            wrapping: if rng.chance(30) {
                Wrapping::Stop
            } else {
                Wrapping::Wrap
            },
            with_windows: (1..=6).filter(|_| rng.chance(60)).collect(),
            catalog,
        };
        // A step answers an option of one workspace.
        let rust = wire::encode_maybe_space(question.answer());
        (wire::encode_step(question), rust)
    });
}

#[test]
fn the_roc_core_resolves_selectors_as_the_rust_core_does() {
    differential("resolve", 0x2E50_1000_0000_0030, |rng| {
        let scope: Vec<(u64, u64, u64)> = rng
            .catalog()
            .into_iter()
            .map(|(id, space)| (id, space, 100 + rng.below(4)))
            .collect();
        let question = ResolveQuestion {
            mode: rng.mode(),
            space: rng.space(),
            visible: if rng.chance(15) {
                vec![]
            } else {
                rng.visible()
            },
            selector: if rng.chance(50) {
                Selector::Index(rng.below(8) as usize)
            } else {
                Selector::Name(100 + rng.below(5))
            },
            scope,
        };
        let rust = wire::encode_target(question.answer());
        (wire::encode_resolve(question), rust)
    });
}

#[test]
fn the_roc_core_follows_navigation_history_as_the_rust_core_does() {
    differential("previous", 0x9E21_0000_0000_0031, |rng| {
        let question = PreviousQuestion {
            mode: rng.mode(),
            space: rng.space(),
            visible: if rng.chance(15) {
                vec![]
            } else {
                rng.visible()
            },
            last: rng.chance(80).then(|| 1 + rng.below(6)),
            on: rng.chance(85).then(|| rng.space()),
        };
        let rust = wire::encode_target(question.answer());
        (wire::encode_previous(question), rust)
    });
}

#[test]
fn the_roc_core_steps_the_topology_as_the_rust_core_does() {
    differential("topology", 0x7090_0000_0000_0032, |rng| {
        let mut created: Vec<u64> = Vec::new();
        let mut events = Vec::new();
        for _ in 0..rng.below(25) {
            // A known workspace usually; one never created sometimes.
            let known = |rng: &mut Rng, created: &[u64]| match created.len() {
                0 => 1 + rng.below(6),
                n if rng.chance(85) => created[rng.below(n as u64) as usize],
                _ => 1 + rng.below(6),
            };
            let maybe = |rng: &mut Rng| rng.chance(60).then(|| 1 + rng.below(6));
            let event = match rng.below(100) {
                0..25 => {
                    // A fresh id usually; one already in the catalog rarely.
                    let workspace = match (1..=6).find(|id| !created.contains(id)) {
                        Some(fresh) if rng.chance(95) => fresh,
                        _ => 1 + rng.below(6),
                    };
                    created.push(workspace);
                    TopologyEvent::Created { workspace, space: rng.space() }
                }
                25..35 => TopologyEvent::Removed(known(rng, &created)),
                35..55 => TopologyEvent::Moved {
                    workspace: known(rng, &created),
                    space: rng.space(),
                },
                55..90 => {
                    let activation = if rng.chance(75) {
                        Activation::Showing {
                            current: known(rng, &created),
                            previous: maybe(rng),
                            remembered: maybe(rng),
                        }
                    } else {
                        Activation::Vacant { remembered: maybe(rng) }
                    };
                    TopologyEvent::Activated { space: rng.space(), activation }
                }
                _ => TopologyEvent::Cleared(rng.space()),
            };
            events.push(event);
        }
        let question = TopologyQuestion { events };
        let rust = wire::encode_topology(&question.answer());
        (wire::encode_topology_steps(question), rust)
    });
}
