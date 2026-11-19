//! The Roc core's lifecycle decisions against the Rust core's (`rift_core::lifecycle`), on
//! random questions: the answers must be identical word for word. Ids are few and collide on
//! purpose, so windows share Spaces, entries share apps, and departures hit and miss.

use rift_core::lifecycle::plain::{
    KeepsTransient, Misplaced, NativeTransient, OutOfView, PointerTarget, RuleRefresh, RuleTargets,
    TitleChange,
};
use rift_core::lifecycle::{
    Departure, NewWindow, OnTitleChange, RuleOutcome, RuleRun, TransientOrigin, WorkspaceMode,
};
use rift_core::plain::wire;
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
            _ => 1 + self.below(5),
        }
    }

    fn ids(&mut self, most: u64) -> Vec<u64> {
        (0..self.below(most + 1)).map(|_| self.id()).collect()
    }

    fn maybe(&mut self) -> Option<u64> { self.chance(50).then(|| self.id()) }

    fn run(&mut self) -> RuleRun {
        if self.chance(50) {
            RuleRun::Admission
        } else {
            RuleRun::Reapply
        }
    }

    fn origin(&mut self) -> TransientOrigin {
        if self.chance(50) {
            TransientOrigin::Native
        } else {
            TransientOrigin::Heuristic
        }
    }
}

/// Runs random questions through both cores; `ask` makes one and returns its words and the
/// Rust core's answer as words.
fn differential(name: &str, seed: u64, mut ask: impl FnMut(&mut Rng) -> (Vec<u64>, Vec<u64>)) {
    let cases: u64 = std::env::var("RIFT_LIFECYCLE_CASES").map_or(20_000, |n| n.parse().unwrap());
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
    eprintln!("lifecycle: {cases} {name} questions answered identically");
}

#[test]
fn the_roc_core_picks_app_rule_targets_as_the_rust_core_does() {
    differential("rule-targets", 0x7A26_E750_0000_0009, |rng| {
        let question = RuleTargets {
            windows: rng.ids(6),
            run: rng.run(),
            reconcilable: rng.ids(4),
            spaces: (0..rng.below(6)).map(|_| (rng.id(), rng.id())).collect(),
            active: rng.ids(3),
        };
        let rust = wire::encode_rule_targets(question.answer());
        (wire::encode_rule_targets_question(question), rust)
    });
}

#[test]
fn the_roc_core_decides_whether_the_layout_hears_of_a_window_as_the_rust_core_does() {
    differential("rule-refresh", 0x2EF2_E540_0000_0010, |rng| {
        let question = RuleRefresh {
            run: rng.run(),
            outcome: RuleOutcome {
                workspace_before: rng.maybe(),
                workspace: rng.id(),
                floating_before: rng.chance(50),
                floating: rng.chance(50),
                was_ignored: rng.chance(20),
                unavailable: rng.chance(20),
            },
        };
        let rust = wire::encode_verdict(question.answer());
        (wire::encode_rule_refresh(question), rust)
    });
}

#[test]
fn the_roc_core_places_a_new_window_on_the_pointers_display_as_the_rust_core_does() {
    differential("pointer-target", 0x9014_7E2D_0000_0011, |rng| {
        let question = PointerTarget {
            mode: if rng.chance(70) {
                WorkspaceMode::Global
            } else {
                WorkspaceMode::PerDisplay
            },
            opened_on: rng.id(),
            pointer: rng.maybe(),
            active: rng.ids(3),
            window: match rng.below(4) {
                0 => NewWindow::Untracked,
                1 => NewWindow::NotStandard,
                _ => NewWindow::Standard(()),
            },
            rule_names_workspace: rng.chance(30),
        };
        let rust = wire::encode_maybe_space(question.answer());
        (wire::encode_pointer_target(question), rust)
    });
}

#[test]
fn the_roc_core_finds_native_transients_as_the_rust_core_does() {
    differential("native-transient", 0x7EA2_5157_0000_0012, |rng| {
        let question = NativeTransient {
            entry: rng.chance(70).then(|| (rng.origin(), rng.id())),
            owner: rng.id(),
            exempt: rng.chance(20),
        };
        let rust = wire::encode_verdict(question.answer());
        (wire::encode_native_transient(question), rust)
    });
}

#[test]
fn the_roc_core_keeps_transient_entries_as_the_rust_core_does() {
    differential("keeps-transient", 0x6EE9_5000_0000_0013, |rng| {
        let question = KeepsTransient {
            departure: match rng.below(3) {
                0 => Departure::Destroyed(rng.maybe()),
                1 => Departure::Closed(rng.id()),
                _ => Departure::AppGone(rng.id()),
            },
            id: rng.id(),
            origin: rng.origin(),
            owner: rng.id(),
        };
        let rust = wire::encode_verdict(question.answer());
        (wire::encode_keeps_transient(question), rust)
    });
}

#[test]
fn the_roc_core_finds_misplaced_windows_as_the_rust_core_does() {
    differential("misplaced", 0x0315_91AC_ED00_0014, |rng| {
        let question = Misplaced {
            assigned: rng.maybe(),
            native: rng.maybe(),
        };
        let rust = wire::encode_verdict(question.answer());
        (wire::encode_misplaced(question), rust)
    });
}

#[test]
fn the_roc_core_knows_what_an_inventory_cannot_see_as_the_rust_core_does() {
    differential("out-of-view", 0x0070_F713_0000_0015, |rng| {
        let question = OutOfView {
            native: rng.maybe(),
            assigned: rng.maybe(),
            active: rng.ids(3),
            parked: rng.chance(20),
        };
        let rust = wire::encode_verdict(question.answer());
        (wire::encode_out_of_view(question), rust)
    });
}

#[test]
fn the_roc_core_reapplies_rules_on_a_title_change_as_the_rust_core_does() {
    differential("title-change", 0x7171_EC4A_0000_0016, |rng| {
        let question = TitleChange {
            configured: if rng.chance(80) {
                OnTitleChange::Reapply
            } else {
                OnTitleChange::Ignore
            },
            space: rng.maybe(),
            active: rng.ids(3),
            reconcilable: rng.chance(60),
        };
        let rust = wire::encode_verdict(question.answer());
        (wire::encode_title_change(question), rust)
    });
}

#[test]
fn the_roc_core_admits_windows_as_the_rust_core_does_for_every_combination_of_facts() {
    use rift_core::lifecycle::{AdmissionFacts, AxKind, Manageability, Minimize, NativeKind, Rule};
    let before = AllocationCount::now();
    let mut checked = 0;
    for manageability in [Manageability::Manageable, Manageability::Unmanageable] {
        for rule in [Rule::NoRule, Rule::Manage, Rule::Ignore] {
            for ax in [AxKind::Ordinary, AxKind::DialogWindow, AxKind::Transient] {
                for native in [NativeKind::Ordinary, NativeKind::Transient] {
                    for minimize in [Minimize::Shown, Minimize::Minimized] {
                        let facts = AdmissionFacts {
                            manageability,
                            rule,
                            ax,
                            native,
                            minimize,
                        };
                        let rust =
                            wire::encode_window_admission(rift_core::lifecycle::admission(facts));
                        assert_eq!(
                            tile_words(&wire::encode_admission_facts(facts)),
                            Ok(rust),
                            "{facts:?}"
                        );
                        checked += 1;
                    }
                }
            }
        }
    }
    assert_eq!(checked, 72);
    let after = AllocationCount::now();
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    eprintln!("lifecycle: all {checked} admission fact combinations answered identically");
}

#[test]
fn the_roc_core_reads_every_lifecycle_reading_as_the_rust_core_does() {
    use rift_core::lifecycle::{
        AdmissionFacts, AxKind, FactChange, Manageability, Minimize, NativeKind, Reading, Rule,
    };
    let mut all_facts = Vec::new();
    for manageability in [Manageability::Manageable, Manageability::Unmanageable] {
        for rule in [Rule::NoRule, Rule::Manage, Rule::Ignore] {
            for ax in [AxKind::Ordinary, AxKind::DialogWindow, AxKind::Transient] {
                for native in [NativeKind::Ordinary, NativeKind::Transient] {
                    for minimize in [Minimize::Shown, Minimize::Minimized] {
                        all_facts.push(AdmissionFacts {
                            manageability,
                            rule,
                            ax,
                            native,
                            minimize,
                        });
                    }
                }
            }
        }
    }
    let changes = [
        FactChange::Manageability(Manageability::Manageable),
        FactChange::Manageability(Manageability::Unmanageable),
        FactChange::Rule(Rule::NoRule),
        FactChange::Rule(Rule::Manage),
        FactChange::Rule(Rule::Ignore),
        FactChange::Ax(AxKind::Ordinary),
        FactChange::Ax(AxKind::DialogWindow),
        FactChange::Ax(AxKind::Transient),
        FactChange::Native(NativeKind::Ordinary),
        FactChange::Native(NativeKind::Transient),
        FactChange::Minimize(Minimize::Shown),
        FactChange::Minimize(Minimize::Minimized),
    ];
    let readings: Vec<Reading> = all_facts
        .iter()
        .copied()
        .map(Reading::Live)
        .chain(changes.into_iter().map(Reading::Changed))
        .chain([Reading::Gone])
        .collect();
    let befores: Vec<Option<AdmissionFacts>> =
        std::iter::once(None).chain(all_facts.iter().copied().map(Some)).collect();
    let before_count = AllocationCount::now();
    let mut checked = 0;
    for &before in &befores {
        for &reading in &readings {
            let rust = wire::encode_read(rift_core::lifecycle::read(before, reading));
            assert_eq!(
                tile_words(&wire::encode_reading(before, reading)),
                Ok(rust),
                "{before:?} {reading:?}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 73 * 85);
    let after = AllocationCount::now();
    assert_eq!(after.live(), before_count.live(), "Roc allocations left live");
    eprintln!("lifecycle: all {checked} lifecycle readings answered identically");
}

#[test]
fn the_roc_core_steps_every_presence_event_as_the_rust_core_does() {
    use rift_core::lifecycle::{
        AdmissionFacts, AxKind, FactChange, Manageability, Minimize, NativeKind, Presence,
        PresenceEvent, Reading, Rule,
    };
    let mut all_facts = Vec::new();
    for manageability in [Manageability::Manageable, Manageability::Unmanageable] {
        for rule in [Rule::NoRule, Rule::Manage, Rule::Ignore] {
            for ax in [AxKind::Ordinary, AxKind::DialogWindow, AxKind::Transient] {
                for native in [NativeKind::Ordinary, NativeKind::Transient] {
                    for minimize in [Minimize::Shown, Minimize::Minimized] {
                        all_facts.push(AdmissionFacts {
                            manageability,
                            rule,
                            ax,
                            native,
                            minimize,
                        });
                    }
                }
            }
        }
    }
    let changes = [
        FactChange::Manageability(Manageability::Manageable),
        FactChange::Manageability(Manageability::Unmanageable),
        FactChange::Rule(Rule::NoRule),
        FactChange::Rule(Rule::Manage),
        FactChange::Rule(Rule::Ignore),
        FactChange::Ax(AxKind::Ordinary),
        FactChange::Ax(AxKind::DialogWindow),
        FactChange::Ax(AxKind::Transient),
        FactChange::Native(NativeKind::Ordinary),
        FactChange::Native(NativeKind::Transient),
        FactChange::Minimize(Minimize::Shown),
        FactChange::Minimize(Minimize::Minimized),
    ];
    let events: Vec<PresenceEvent> = all_facts
        .iter()
        .copied()
        .map(|facts| PresenceEvent::Reading(Reading::Live(facts)))
        .chain(
            changes
                .into_iter()
                .map(|change| PresenceEvent::Reading(Reading::Changed(change))),
        )
        .chain([
            PresenceEvent::Reading(Reading::Gone),
            PresenceEvent::Invalidate,
            PresenceEvent::Revalidate,
        ])
        .collect();
    let befores: Vec<Option<Presence>> = std::iter::once(None)
        .chain(all_facts.iter().copied().map(|facts| Some(Presence::Live(facts))))
        .chain(all_facts.iter().copied().map(|facts| Some(Presence::Invalidated(facts))))
        .collect();
    let before_count = AllocationCount::now();
    let mut checked = 0;
    for &before in &befores {
        for &event in &events {
            let rust = wire::encode_presence(rift_core::lifecycle::present(before, event));
            assert_eq!(
                tile_words(&wire::encode_presence_event(before, event)),
                Ok(rust),
                "{before:?} {event:?}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 145 * 87);
    let after = AllocationCount::now();
    assert_eq!(after.live(), before_count.live(), "Roc allocations left live");
    eprintln!("lifecycle: all {checked} presence events answered identically");
}
