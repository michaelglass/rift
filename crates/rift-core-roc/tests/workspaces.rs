//! The Roc core's workspace decisions against the Rust core's (`rift_core::workspaces`), on
//! random questions: the answers must be identical word for word. Catalogs are small and their
//! Spaces few, the current workspace is usually in them, names repeat, and visible sets overlap,
//! so steps cross, wrap and stop. Topology event runs reuse a few workspaces and Spaces, so
//! moves and removals mostly find their workspace, and Spaces are activated again, cleared and
//! activated anew. Ownership runs reuse a few windows, so ownership is replaced and dropped,
//! and their catalogs miss some workspaces, so orphans appear. Joined workspace runs name
//! unknown workspaces, succeed workspaces into themselves and remove shown ones often enough
//! that every refusal is met.

mod facts;

use facts::{Coin, rust, typed};
use rift_core::lifecycle::WorkspaceMode;
use rift_core::plain::{Answers, wire};
use rift_core::workspaces::plain::{
    EligibleQuestion, OwnershipQuestion, PreviousQuestion, ResolveQuestion, StepQuestion,
    TopologyQuestion, WorkspacesQuestion,
};
use rift_core::workspaces::{
    Activation, Destination, Owned, Ownership, OwnershipEvent, Selector, SkipEmpty, Step,
    Successor, TopologyEvent, WorkspacesEvent, Wrapping,
};
use rift_core_roc::AllocationCount;
use rift_core_roc::workspaces::{JoinedView, OwnershipView, TopologyView};

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

impl Coin for Rng {
    fn chance(&mut self, percent: u64) -> bool { Rng::chance(self, percent) }
}

/// Runs random questions through both cores; `ask` makes one and returns it (for the failure
/// message), the Roc core's answer and the Rust core's.
fn differential<A: PartialEq + std::fmt::Debug>(
    name: &str,
    seed: u64,
    mut ask: impl FnMut(&mut Rng) -> (String, A, A),
) {
    let cases: u64 = std::env::var("RIFT_WORKSPACES_CASES").map_or(20_000, |n| n.parse().unwrap());
    let mut rng = Rng(seed);
    let before = AllocationCount::now();
    for case in 0..cases {
        let (question, roc, rust) = ask(&mut rng);
        assert_eq!(roc, rust, "case {case}: {name} {question}");
    }
    let after = AllocationCount::now();
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    eprintln!("workspaces: {cases} {name} questions answered identically");
}

#[test]
fn the_roc_core_steps_through_workspaces_as_the_rust_core_does() {
    typed("workspaces", "step", Rng(0x57E9_0000_0000_0029), |rng, blank| {
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
            has_windows: Answers(Vec::new()),
            catalog,
        };
        // Every catalogued workspace answered whether it has windows (each key once).
        let with_windows: Vec<u64> = (1..=6).filter(|_| rng.chance(60)).collect();
        let ids: Vec<u64> = question.catalog.iter().map(|(id, _)| *id).collect();
        let question = StepQuestion {
            has_windows: blank.answers(rng, ids, |id| with_windows.contains(&id)),
            ..question
        };
        // A step answers an option of one workspace.
        let roc = rust(
            rift_core_roc::workspaces::step(question.clone()),
            wire::encode_maybe_space,
        );
        let expected = rust(question.answer(), wire::encode_maybe_space);
        (format!("{question:?}"), roc, expected)
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
        let roc = rift_core_roc::workspaces::resolve(question.clone());
        (format!("{question:?}"), roc, question.answer())
    });
}

#[test]
fn the_roc_core_follows_navigation_history_as_the_rust_core_does() {
    typed(
        "workspaces",
        "previous",
        Rng(0x9E21_0000_0000_0031),
        |rng, blank| {
            let on = rng.chance(85).then(|| rng.space());
            let question = PreviousQuestion {
                mode: rng.mode(),
                space: rng.space(),
                visible: if rng.chance(15) {
                    vec![]
                } else {
                    rng.visible()
                },
                last: rng.chance(80).then(|| 1 + rng.below(6)),
                on: blank.fact(rng, on),
            };
            let roc = rust(rift_core_roc::workspaces::previous(question.clone()), target);
            let expected = rust(question.answer(), target);
            (format!("{question:?}"), roc, expected)
        },
    );
}

#[test]
fn the_roc_core_finds_a_workspace_reachable_as_the_rust_core_does() {
    differential("eligible", 0xE119_0000_0000_0035, |rng| {
        let question = EligibleQuestion {
            mode: rng.mode(),
            space: rng.space(),
            visible: if rng.chance(15) {
                vec![]
            } else {
                rng.visible()
            },
            on: rng.space(),
        };
        let roc = rift_core_roc::workspaces::eligible(question.clone());
        (format!("{question:?}"), roc, question.answer())
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
        let topology = question.answer();
        let rust = TopologyView {
            catalog: topology.catalog().to_vec(),
            activations: topology.activations().to_vec(),
        };
        let roc = rift_core_roc::workspaces::topology(question.clone());
        (format!("{question:?}"), roc, rust)
    });
}

#[test]
fn the_roc_core_steps_ownership_and_finds_orphans_as_the_rust_core_does() {
    differential("ownership", 0x0E4E_0000_0000_0033, |rng| {
        let events = (0..rng.below(25))
            .map(|_| {
                let window = 1 + rng.below(8);
                let owned = match rng.below(100) {
                    0..20 => Owned::Unowned,
                    20..30 => Owned::Owner(Ownership::Held),
                    30..65 => Owned::Owner(Ownership::Assigned(1 + rng.below(5))),
                    _ => Owned::Owner(Ownership::Claimed {
                        owner: 100 + rng.below(3),
                        destination: if rng.chance(70) {
                            Destination::Returns(1 + rng.below(5))
                        } else {
                            Destination::Nowhere
                        },
                    }),
                };
                OwnershipEvent::Owned { window, owned }
            })
            .collect();
        let question = OwnershipQuestion {
            events,
            catalog: (1..=5).filter(|_| rng.chance(70)).collect(),
        };
        let (ownerships, orphans) = question.answer();
        let rust = OwnershipView {
            owners: ownerships.owners().collect(),
            orphans,
        };
        let roc = rift_core_roc::workspaces::ownership(question.clone());
        (format!("{question:?}"), roc, rust)
    });
}

#[test]
fn the_roc_core_steps_the_joined_workspaces_as_the_rust_core_does() {
    differential("joined", 0x7015_0000_0000_0034, |rng| {
        // Workspaces 1 to 5, usually ones created already; Spaces 10, 20, 30; windows 1 to 8.
        let mut created: Vec<u64> = Vec::new();
        let mut events = Vec::new();
        for _ in 0..rng.below(30) {
            let known = |rng: &mut Rng, created: &[u64]| match created.len() {
                n if n > 0 && rng.chance(85) => created[rng.below(n as u64) as usize],
                _ => 1 + rng.below(5),
            };
            let maybe =
                |rng: &mut Rng, created: &[u64]| rng.chance(60).then(|| known(rng, created));
            let event = match rng.below(100) {
                0..20 => {
                    let workspace = 1 + rng.below(5);
                    created.push(workspace);
                    WorkspacesEvent::Created { workspace, space: rng.space() }
                }
                20..30 => WorkspacesEvent::Removed {
                    workspace: known(rng, &created),
                    successor: if rng.chance(70) {
                        Successor::Into(known(rng, &created))
                    } else {
                        Successor::Unassign
                    },
                },
                30..40 => WorkspacesEvent::Moved {
                    workspace: known(rng, &created),
                    space: rng.space(),
                },
                40..60 => WorkspacesEvent::Activated {
                    space: rng.space(),
                    activation: if rng.chance(70) {
                        Activation::Showing {
                            current: known(rng, &created),
                            previous: maybe(rng, &created),
                            remembered: maybe(rng, &created),
                        }
                    } else {
                        Activation::Vacant {
                            remembered: maybe(rng, &created),
                        }
                    },
                },
                60..68 => WorkspacesEvent::Cleared(rng.space()),
                _ => WorkspacesEvent::Owned {
                    window: 1 + rng.below(8),
                    owned: match rng.below(100) {
                        0..15 => Owned::Unowned,
                        15..25 => Owned::Owner(Ownership::Held),
                        25..65 => Owned::Owner(Ownership::Assigned(known(rng, &created))),
                        _ => Owned::Owner(Ownership::Claimed {
                            owner: 100 + rng.below(3),
                            destination: if rng.chance(70) {
                                Destination::Returns(known(rng, &created))
                            } else {
                                Destination::Nowhere
                            },
                        }),
                    },
                },
            };
            events.push(event);
        }
        let question = WorkspacesQuestion { events };
        let (outcomes, workspaces) = question.answer();
        let rust = JoinedView {
            outcomes,
            catalog: workspaces.catalog().to_vec(),
            activations: workspaces.activations().to_vec(),
            owners: workspaces.owners().collect(),
            windows: workspaces
                .catalog()
                .iter()
                .map(|&(workspace, _)| workspaces.windows_on(workspace).collect())
                .collect(),
        };
        let roc = rift_core_roc::workspaces::joined(question.clone());
        (format!("{question:?}"), roc, rust)
    });
}

/// A target (workspace and Space) or none, as words.
fn target(answer: Option<(u64, u64)>) -> Vec<u64> {
    match answer {
        Some((workspace, space)) => vec![1, workspace, space],
        None => vec![0],
    }
}

/// Who places each window, kept between events: the Roc twin and the Rust ownerships take the
/// same runs of ownership changes on a few windows (assigned, claimed with or without a
/// destination, held under the user's hand, unowned), each run on one persistent model. After
/// every change both must say the same of the changed window and keep the same owners.
#[test]
fn the_roc_ownerships_keep_owners_between_events_as_the_rust_ones_do() {
    use rift_core::workspaces::{Destination, Owned, Ownership, OwnershipEvent, Ownerships};
    use rift_core_roc::RocOwnerships;
    let mut rng = Rng(0x0C4E_5000_0000_0037);
    let before = AllocationCount::now();
    let mut steps = 0;
    let mut held = 0;
    for run in 0..400 {
        let mut rust = Ownerships::<u64, u64, u64>::default();
        let mut roc = RocOwnerships::default();
        for step in 0..60 {
            let window = 1 + rng.below(6);
            let owned = match rng.below(8) {
                0 | 1 => Owned::Unowned,
                2 | 3 => Owned::Owner(Ownership::Assigned(1 + rng.below(5))),
                4 => Owned::Owner(Ownership::Claimed {
                    owner: 1 + rng.below(3),
                    destination: Destination::Returns(1 + rng.below(5)),
                }),
                5 => Owned::Owner(Ownership::Claimed {
                    owner: 1 + rng.below(3),
                    destination: Destination::Nowhere,
                }),
                _ => {
                    held += 1;
                    Owned::Owner(Ownership::Held)
                }
            };
            let event = OwnershipEvent::Owned { window, owned };
            rust.step(event);
            assert_eq!(
                roc.step(event),
                rust.owned(window),
                "run {run} step {step}: {event:?}"
            );
            let expected: Vec<_> = rust.owners().collect();
            assert_eq!(
                roc.view(),
                expected,
                "run {run} step {step}: owners after {event:?}"
            );
            steps += 1;
        }
    }
    assert_eq!(
        AllocationCount::now().live(),
        before.live(),
        "Roc allocations left live"
    );
    eprintln!(
        "workspaces: {steps} kept-ownership steps identical ({held} held), owners compared after each"
    );
}

/// The workspaces' topology, kept between events: the Roc twin and the Rust topology take the
/// same runs of workspaces created, removed and moved (known and unknown ones) and Spaces
/// activated and cleared, each run on one persistent model. After every change both must hold
/// the same catalog and activations, in the same orders.
#[test]
fn the_roc_topology_keeps_workspaces_between_events_as_the_rust_one_does() {
    use rift_core::workspaces::{Activation, Topology, TopologyEvent};
    use rift_core_roc::RocTopology;
    let mut rng = Rng(0x70B0_1000_0000_0038);
    let before = AllocationCount::now();
    let mut steps = 0;
    for run in 0..400 {
        let mut rust = Topology::<u64, u64>::default();
        let mut roc = RocTopology::default();
        let mut created = 0;
        for step in 0..60 {
            let maybe = |rng: &mut Rng| rng.chance(50).then(|| 1 + rng.below(8));
            let event = match rng.below(10) {
                0..=2 => {
                    created += 1;
                    // Now and then a workspace id the catalog already has.
                    let workspace = if rng.chance(10) {
                        1 + rng.below(created)
                    } else {
                        created
                    };
                    TopologyEvent::Created { workspace, space: rng.space() }
                }
                3 => TopologyEvent::Removed(1 + rng.below(created + 2)),
                4 | 5 => TopologyEvent::Moved {
                    workspace: 1 + rng.below(created + 2),
                    space: rng.space(),
                },
                6..=8 => TopologyEvent::Activated {
                    space: rng.space(),
                    activation: if rng.chance(75) {
                        Activation::Showing {
                            current: 1 + rng.below(8),
                            previous: maybe(&mut rng),
                            remembered: maybe(&mut rng),
                        }
                    } else {
                        Activation::Vacant { remembered: maybe(&mut rng) }
                    },
                },
                _ => TopologyEvent::Cleared(rng.space()),
            };
            rust.step(event);
            roc.step(event);
            let expected = TopologyView {
                catalog: rust.catalog().to_vec(),
                activations: rust.activations().to_vec(),
            };
            assert_eq!(roc.view(), expected, "run {run} step {step}: after {event:?}");
            steps += 1;
        }
    }
    assert_eq!(
        AllocationCount::now().live(),
        before.live(),
        "Roc allocations left live"
    );
    eprintln!(
        "workspaces: {steps} kept-topology steps identical, catalog and activations compared after each"
    );
}
