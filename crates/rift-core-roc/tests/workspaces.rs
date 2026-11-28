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
    ClaimFlags, Claimant, EligibleQuestion, EmitsQuestion, FocusFollowsMouse, PreviousQuestion,
    ResolveQuestion, StepQuestion, WorkspacesQuestion,
};
use rift_core::workspaces::{
    Activation, Destination, Owned, Ownership, Selector, SkipEmpty, Step, Successor,
    WorkspacesEvent, Wrapping,
};
use rift_core_roc::AllocationCount;
use rift_core_roc::workspaces::JoinedView;

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

    /// A claim's holder: one of three owners, either flag, and the id it claimed.
    fn claimant(&mut self) -> Claimant {
        Claimant {
            owner: 100 + self.below(3),
            flags: ClaimFlags {
                focus_follows_mouse: if self.chance(50) {
                    FocusFollowsMouse::Suppressed
                } else {
                    FocusFollowsMouse::Followed
                },
            },
            original: 1 + self.below(8),
        }
    }

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
                            owner: rng.claimant(),
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

#[test]
fn the_roc_core_emits_a_discovered_window_as_the_rust_core_does() {
    typed(
        "workspaces",
        "emits_window",
        Rng(0xE417_0000_0000_0037),
        |rng, blank| {
            // Few workspaces, so the assigned one is often the active one.
            let active = rng.chance(85).then(|| 1 + rng.below(3));
            let question = EmitsQuestion {
                assigned: rng.chance(80).then(|| 1 + rng.below(3)),
                active: blank.fact(rng, active),
            };
            let roc = rust(
                rift_core_roc::workspaces::emits_window(question),
                wire::encode_verdict,
            );
            let expected = rust(question.answer(), wire::encode_verdict);
            (format!("{question:?}"), roc, expected)
        },
    );
}

/// The workspaces joined, kept between events: the Roc twin and the Rust workspaces take the
/// same runs of changes, each run on one persistent model. The runs lean on what the step turns
/// on: workspaces created twice, unknown ones named, a workspace succeeding itself, a shown one
/// removed, and removals with windows and claims to hand on. After every change both must meet it
/// the same way (accepted, or refused for the same reason) and keep the same catalog,
/// activations, owners and windows per workspace.
#[test]
fn the_roc_workspaces_keep_the_joined_workspaces_between_events_as_the_rust_ones_do() {
    use rift_core::workspaces::Workspaces;
    use rift_core_roc::RocWorkspaces;
    use rift_core_roc::workspaces::WorkspacesView;
    let mut rng = Rng(0x0701_4ED0_0000_0039);
    let before = AllocationCount::now();
    let (mut steps, mut refused, mut handed) = (0, 0, 0);
    for run in 0..400 {
        let mut rust = Workspaces::<u64, u64, u64, Claimant>::default();
        let mut roc = RocWorkspaces::default();
        for step in 0..60 {
            // Workspaces 1 to 6, mostly ones in the catalog; windows 1 to 8.
            let catalog: Vec<u64> = rust.catalog().iter().map(|&(id, _)| id).collect();
            let known = |rng: &mut Rng| match catalog.len() {
                n if n > 0 && rng.chance(85) => catalog[rng.below(n as u64) as usize],
                _ => 1 + rng.below(6),
            };
            let maybe = |rng: &mut Rng| rng.chance(60).then(|| known(rng));
            let event = match rng.below(100) {
                0..15 => WorkspacesEvent::Created {
                    workspace: 1 + rng.below(6),
                    space: rng.space(),
                },
                15..27 => {
                    let workspace = known(&mut rng);
                    WorkspacesEvent::Removed {
                        workspace,
                        successor: match rng.below(10) {
                            0 => Successor::Into(workspace),
                            1..=6 => Successor::Into(known(&mut rng)),
                            _ => Successor::Unassign,
                        },
                    }
                }
                27..33 => WorkspacesEvent::Moved {
                    workspace: known(&mut rng),
                    space: rng.space(),
                },
                33..53 => WorkspacesEvent::Activated {
                    space: rng.space(),
                    activation: if rng.chance(70) {
                        Activation::Showing {
                            current: known(&mut rng),
                            previous: maybe(&mut rng),
                            remembered: maybe(&mut rng),
                        }
                    } else {
                        Activation::Vacant { remembered: maybe(&mut rng) }
                    },
                },
                53..60 => WorkspacesEvent::Cleared(rng.space()),
                _ => WorkspacesEvent::Owned {
                    window: 1 + rng.below(8),
                    owned: match rng.below(100) {
                        0..10 => Owned::Unowned,
                        10..20 => Owned::Owner(Ownership::Held),
                        20..70 => Owned::Owner(Ownership::Assigned(known(&mut rng))),
                        _ => Owned::Owner(Ownership::Claimed {
                            owner: rng.claimant(),
                            destination: if rng.chance(75) {
                                Destination::Returns(known(&mut rng))
                            } else {
                                Destination::Nowhere
                            },
                        }),
                    },
                },
            };
            let owners_before = rust.owners().count();
            let met = rust.step(event);
            if met.is_err() {
                refused += 1;
            } else if matches!(event, WorkspacesEvent::Removed { .. }) && owners_before > 0 {
                handed += 1;
            }
            assert_eq!(roc.step(event), met, "run {run} step {step}: {event:?}");
            let expected = WorkspacesView {
                catalog: rust.catalog().to_vec(),
                activations: rust.activations().to_vec(),
                owners: rust.owners().collect(),
                windows: rust
                    .catalog()
                    .iter()
                    .map(|&(workspace, _)| rust.windows_on(workspace).collect())
                    .collect(),
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
        "workspaces: {steps} kept-joined steps identical ({refused} refused, {handed} removals with owners), views compared after each"
    );
}

/// The layout engine's switching choices (`rift_core::workspaces::choose_switching`): every kind
/// of question, drawn at random, with a few workspaces and Spaces so the shown workspace is the
/// target about half the time and the asking Space is the workspace's own about as often.
#[test]
fn the_roc_core_makes_the_engines_switching_choices_as_the_rust_core_does() {
    use rift_core::workspaces::{
        ArrivalAsk, MovedAsk, SwitchAsk, SwitchQuestion as Q, TransferReason, choose_switching,
    };
    let mut rng = Rng(0x5717_C4E5_0E1B);
    let runs: u64 =
        std::env::var("RIFT_ENGINE_SWITCHING_CASES").map_or(20_000, |n| n.parse().unwrap());
    for case in 0..runs {
        let mut word = |n: u64| 1 + rng.below(n);
        let question: Q<u64, u64> = match word(4) {
            1 => {
                let shown = word(3);
                Q::Switch(SwitchAsk {
                    shown_here: (shown < 3).then_some(shown),
                    target: word(2),
                    back_and_forth: word(2) == 1,
                })
            }
            2 => {
                let shown = word(3);
                Q::Arrival(ArrivalAsk {
                    shown_there: (shown < 3).then_some(shown),
                    target: word(2),
                    asked_from: 10 + word(2),
                    its_space: 10 + word(2),
                })
            }
            3 => Q::Source(
                [
                    TransferReason::Explicit,
                    TransferReason::DisplayArrival,
                    TransferReason::DisplayDeparture,
                    TransferReason::Restoration,
                ][word(4) as usize - 1],
            ),
            _ => Q::Moved(MovedAsk {
                source: word(2),
                target: word(2),
            }),
        };
        let rust = choose_switching(question);
        let roc = rift_core_roc::workspaces::engine_switching(question);
        assert_eq!(roc, rust, "case {case}: {question:?}");
    }
    eprintln!("engine switching: {runs} questions answered alike");
}
