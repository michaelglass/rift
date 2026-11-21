//! The Roc core's lifecycle decisions against the Rust core's (`rift_core::lifecycle`), on
//! random questions: the answers must be identical word for word. Ids are few and collide on
//! purpose, so windows share Spaces, entries share apps, and departures hit and miss.

use rift_core::lifecycle::plain::{
    Inventory, KeepsTransient, Misplaced, NativeTransient, OutOfView, PointerTarget, RuleRefresh,
    RuleTargets, TitleChange,
};
use rift_core::lifecycle::{
    Departure, NewWindow, OnTitleChange, RuleOutcome, RuleRun, TransientOrigin, WorkspaceMode,
};
mod facts;

use facts::{Coin, rust, typed};
use rift_core::plain::{Fact, wire};
use rift_core_roc::AllocationCount;

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

impl Coin for Rng {
    fn chance(&mut self, percent: u64) -> bool { Rng::chance(self, percent) }
}

#[test]
fn the_roc_core_picks_app_rule_targets_as_the_rust_core_does() {
    typed(
        "lifecycle",
        "rule-targets",
        Rng(0x7A26_E750_0000_0009),
        |rng, blank| {
            let windows = rng.ids(6);
            let run = rng.run();
            let reconcilable = rng.ids(4);
            // A window's Space is the first listed for it, if any.
            let listed: Vec<(u64, u64)> = (0..rng.below(6)).map(|_| (rng.id(), rng.id())).collect();
            let space = |window: u64| listed.iter().find(|(w, _)| *w == window).map(|(_, s)| *s);
            let active = rng.ids(3);
            let spaces: Vec<u64> = listed.iter().map(|(_, space)| *space).collect();
            let question = RuleTargets {
                reconcilable: blank
                    .answers(rng, windows.iter().copied(), |w| reconcilable.contains(&w)),
                spaces: blank.answers(rng, windows.iter().copied(), space),
                active: blank.answers(rng, spaces, |space| active.contains(&space)),
                windows,
                run,
            };
            let roc = rust(
                rift_core_roc::lifecycle::rule_targets(question.clone()),
                wire::encode_rule_targets,
            );
            let rust = rust(question.answer(), wire::encode_rule_targets);
            (format!("{question:?}"), roc, rust)
        },
    );
}

#[test]
fn the_roc_core_decides_whether_the_layout_hears_of_a_window_as_the_rust_core_does() {
    typed(
        "lifecycle",
        "rule-refresh",
        Rng(0x2EF2_E540_0000_0010),
        |rng, blank| {
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
            let _ = blank;
            let roc = Ok(wire::encode_verdict(
                rift_core_roc::lifecycle::rule_refreshes_layout(question),
            ));
            let rust = Ok(wire::encode_verdict(question.answer()));
            (format!("{question:?}"), roc, rust)
        },
    );
}

#[test]
fn the_roc_core_places_a_new_window_on_the_pointers_display_as_the_rust_core_does() {
    typed(
        "lifecycle",
        "pointer-target",
        Rng(0x9014_7E2D_0000_0011),
        |rng, blank| {
            let mode = if rng.chance(70) {
                WorkspaceMode::Global
            } else {
                WorkspaceMode::PerDisplay
            };
            let opened_on = rng.id();
            let pointer = rng.maybe();
            let active = rng.ids(3);
            let window = match rng.below(4) {
                0 => NewWindow::Untracked,
                1 => NewWindow::NotStandard,
                _ => NewWindow::Standard(()),
            };
            let rule_names_workspace = rng.chance(30);
            let question = PointerTarget {
                mode,
                opened_on,
                pointer: blank.fact(rng, pointer),
                active: blank.answers(rng, pointer, |space| active.contains(&space)),
                window,
                rule_names_workspace: blank.fact(rng, rule_names_workspace),
            };
            let roc = rust(
                rift_core_roc::lifecycle::pointer_display_target(question.clone()),
                wire::encode_maybe_space,
            );
            let rust = rust(question.answer(), wire::encode_maybe_space);
            (format!("{question:?}"), roc, rust)
        },
    );
}

#[test]
fn the_roc_core_finds_native_transients_as_the_rust_core_does() {
    typed(
        "lifecycle",
        "native-transient",
        Rng(0x7EA2_5157_0000_0012),
        |rng, blank| {
            let entry = rng.chance(70).then(|| (rng.origin(), rng.id()));
            let owner = rng.id();
            let exempt = rng.chance(20);
            let question = NativeTransient {
                entry,
                owner,
                exempt: blank.fact(rng, exempt),
            };
            let roc = rust(
                rift_core_roc::lifecycle::is_native_transient(question.clone()),
                wire::encode_verdict,
            );
            let rust = rust(question.answer(), wire::encode_verdict);
            (format!("{question:?}"), roc, rust)
        },
    );
}

#[test]
fn the_roc_core_keeps_transient_entries_as_the_rust_core_does() {
    typed(
        "lifecycle",
        "keeps-transient",
        Rng(0x6EE9_5000_0000_0013),
        |rng, blank| {
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
            let _ = blank;
            let roc = Ok(wire::encode_verdict(rift_core_roc::lifecycle::keeps_transient(
                question,
            )));
            let rust = Ok(wire::encode_verdict(question.answer()));
            (format!("{question:?}"), roc, rust)
        },
    );
}

#[test]
fn the_roc_core_finds_misplaced_windows_as_the_rust_core_does() {
    typed(
        "lifecycle",
        "misplaced",
        Rng(0x0315_91AC_ED00_0014),
        |rng, blank| {
            let question = Misplaced {
                assigned: rng.maybe(),
                native: rng.maybe(),
            };
            let _ = blank;
            let roc = Ok(wire::encode_verdict(rift_core_roc::lifecycle::misplaced(
                question,
            )));
            let rust = Ok(wire::encode_verdict(question.answer()));
            (format!("{question:?}"), roc, rust)
        },
    );
}

#[test]
fn the_roc_core_knows_what_an_inventory_cannot_see_as_the_rust_core_does() {
    typed(
        "lifecycle",
        "out-of-view",
        Rng(0x0070_F713_0000_0015),
        |rng, blank| {
            let native = rng.maybe();
            let assigned = rng.maybe();
            let active = rng.ids(3);
            let parked = rng.chance(20);
            let question = OutOfView {
                native,
                assigned: blank.fact(rng, assigned),
                active: blank.answers(rng, native.or(assigned), |space| active.contains(&space)),
                parked: blank.fact(rng, parked),
            };
            let roc = rust(
                rift_core_roc::lifecycle::out_of_inventory_view(question.clone()),
                wire::encode_verdict,
            );
            let rust = rust(question.answer(), wire::encode_verdict);
            (format!("{question:?}"), roc, rust)
        },
    );
}

#[test]
fn the_roc_core_reapplies_rules_on_a_title_change_as_the_rust_core_does() {
    typed(
        "lifecycle",
        "title-change",
        Rng(0x7171_EC4A_0000_0016),
        |rng, blank| {
            let configured = if rng.chance(80) {
                OnTitleChange::Reapply
            } else {
                OnTitleChange::Ignore
            };
            let space = rng.maybe();
            let active = rng.ids(3);
            let reconcilable = rng.chance(60);
            let question = TitleChange {
                configured,
                space: blank.fact(rng, space),
                active: blank.answers(rng, space, |space| active.contains(&space)),
                reconcilable: blank.fact(rng, reconcilable),
            };
            let roc = rust(
                rift_core_roc::lifecycle::reapplies_on_title_change(question.clone()),
                wire::encode_verdict,
            );
            let rust = rust(question.answer(), wire::encode_verdict);
            (format!("{question:?}"), roc, rust)
        },
    );
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
                            wire::encode_window_admission(rift_core_roc::lifecycle::admission(
                                facts
                            )),
                            rust,
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
                wire::encode_read(rift_core_roc::lifecycle::read(before, reading)),
                rust,
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
            let (after, outcome) = rift_core::lifecycle::present(before, event);
            let rust = (after, outcome, rift_core::lifecycle::manages(after));
            assert_eq!(
                rift_core_roc::lifecycle::present(before, event),
                rust,
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

#[test]
fn the_roc_core_retires_windows_as_the_rust_core_does_for_every_combination_of_facts() {
    use rift_core::lifecycle::{
        AxValidity, FullscreenRecord, InventoryFacts, InventoryView, Minimize, NativeEvidence,
        NativeIdentity, NativeLook, Ordering, Report, Suitability,
    };
    let before = AllocationCount::now();
    let mut checked = 0;
    for report in [Report::Reported, Report::Omitted] {
        for view in [InventoryView::InView, InventoryView::OutOfView] {
            for minimize in [None, Some(Minimize::Shown), Some(Minimize::Minimized)] {
                for fullscreen in [FullscreenRecord::Recorded, FullscreenRecord::NotRecorded] {
                    for identity in [NativeIdentity::Known(42_u64), NativeIdentity::Unknown] {
                        for validity in [AxValidity::Valid, AxValidity::Invalidated] {
                            for suitability in [
                                None,
                                Some(Suitability::Suitable),
                                Some(Suitability::Unsuitable),
                            ] {
                                for ordering in
                                    [None, Some(Ordering::OrderedIn), Some(Ordering::OrderedOut)]
                                {
                                    for look in [
                                        None,
                                        Some(NativeLook::Window),
                                        Some(NativeLook::NotAWindow),
                                    ] {
                                        let facts = InventoryFacts {
                                            report,
                                            view,
                                            minimize,
                                            fullscreen,
                                            identity,
                                            validity,
                                        };
                                        let evidence =
                                            NativeEvidence { suitability, ordering, look };
                                        // As asked, and unasked: both cores read it at the same
                                        // point, or neither does.
                                        for evidence in [Fact::Asked(evidence), Fact::Unasked] {
                                            let question = Inventory { facts, evidence };
                                            let roc = rust(
                                                rift_core_roc::lifecycle::retirement(question),
                                                wire::encode_retirement,
                                            );
                                            let expected =
                                                rust(question.answer(), wire::encode_retirement);
                                            assert_eq!(roc, expected, "{question:?}");
                                            checked += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(checked, 2 * 2 * 3 * 2 * 2 * 2 * 27 * 2);
    let after = AllocationCount::now();
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    eprintln!("lifecycle: all {checked} retirement fact combinations answered identically");
}

#[test]
fn the_roc_core_nets_every_pair_of_admission_changes_as_the_rust_core_does() {
    use rift_core::lifecycle::{Transition, Veto};
    let changes: Vec<Option<Transition<u64>>> = [None, Some(Transition::Admitted(7))]
        .into_iter()
        .chain(
            [
                Veto::NativeTransient,
                Veto::Minimized,
                Veto::AxTransient,
                Veto::RuleIgnores,
                Veto::Unmanageable,
            ]
            .map(|why| Some(Transition::Suspended(7, why))),
        )
        .collect();
    let before = AllocationCount::now();
    let mut checked = 0;
    for &first in &changes {
        for &then in &changes {
            let rust = change(rift_core::lifecycle::net(first, then));
            assert_eq!(
                rift_core_roc::lifecycle::net(change(first), change(then)),
                rust,
                "{first:?} {then:?}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 49);
    let after = AllocationCount::now();
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    eprintln!("lifecycle: all {checked} pairs of admission changes answered identically");
}

/// An admission change without its window (`net` does not read it).
fn change(
    transition: Option<rift_core::lifecycle::Transition<u64>>,
) -> Option<rift_core::lifecycle::Change> {
    use rift_core::lifecycle::{Change, Transition};
    transition.map(|transition| match transition {
        Transition::Admitted(_) => Change::Admitted,
        Transition::Suspended(_, why) => Change::Suspended(why),
    })
}

/// The lifecycle kept between events: the Roc twin and the Rust lifecycle take the same runs
/// of readings, invalidations, revalidations and carried invalidations on a few windows (so
/// they come and go and collide), each run on one persistent model. Every operation must come
/// to the same thing and leave the same windows in the same presences.
#[test]
fn the_roc_lifecycle_keeps_windows_between_events_as_the_rust_one_does() {
    use rift_core::lifecycle::{
        AdmissionFacts, AxKind, Change, FactChange, Lifecycle, LifecycleEvent, Manageability,
        Minimize, NativeKind, Rule, Transition,
    };
    use rift_core_roc::{Invalidation, Revalidation, RocLifecycle};
    fn facts(rng: &mut Rng) -> AdmissionFacts {
        AdmissionFacts {
            manageability: if rng.chance(85) {
                Manageability::Manageable
            } else {
                Manageability::Unmanageable
            },
            rule: [Rule::NoRule, Rule::NoRule, Rule::Manage, Rule::Ignore][rng.below(4) as usize],
            ax: [
                AxKind::Ordinary,
                AxKind::Ordinary,
                AxKind::DialogWindow,
                AxKind::Transient,
            ][rng.below(4) as usize],
            native: if rng.chance(85) {
                NativeKind::Ordinary
            } else {
                NativeKind::Transient
            },
            minimize: if rng.chance(80) {
                Minimize::Shown
            } else {
                Minimize::Minimized
            },
        }
    }
    fn change(rng: &mut Rng) -> FactChange {
        let facts = facts(rng);
        match rng.below(5) {
            0 => FactChange::Manageability(facts.manageability),
            1 => FactChange::Rule(facts.rule),
            2 => FactChange::Ax(facts.ax),
            3 => FactChange::Native(facts.native),
            _ => FactChange::Minimize(facts.minimize),
        }
    }
    let as_change = |transition: Option<Transition<u64>>| {
        transition.map(|transition| match transition {
            Transition::Admitted(_) => Change::Admitted,
            Transition::Suspended(_, why) => Change::Suspended(why),
        })
    };
    let mut rng = Rng(0x11FE_C7C1_E000_0036);
    let before = AllocationCount::now();
    let mut steps = 0;
    for run in 0..400 {
        let mut rust = Lifecycle::<u64>::default();
        let mut roc = RocLifecycle::default();
        for step in 0..60 {
            let window = 1 + rng.below(6);
            let what = match rng.below(10) {
                0..=2 => {
                    let event = LifecycleEvent::Live { window, facts: facts(&mut rng) };
                    let expected = as_change(rust.step(event));
                    assert_eq!(roc.step(event), expected, "run {run} step {step}: {event:?}");
                    format!("{event:?}")
                }
                3..=5 => {
                    let event = LifecycleEvent::Changed {
                        window,
                        change: change(&mut rng),
                    };
                    let expected = as_change(rust.step(event));
                    assert_eq!(roc.step(event), expected, "run {run} step {step}: {event:?}");
                    format!("{event:?}")
                }
                6 => {
                    let event = LifecycleEvent::Gone(window);
                    let expected = as_change(rust.step(event));
                    assert_eq!(roc.step(event), expected, "run {run} step {step}: {event:?}");
                    format!("{event:?}")
                }
                7 => {
                    let expected = if rust.invalidate(window) {
                        Invalidation::Invalidated
                    } else {
                        Invalidation::NoAxState
                    };
                    assert_eq!(roc.invalidate(window), expected, "run {run} step {step}");
                    format!("invalidate {window}")
                }
                8 => {
                    let expected = if rust.revalidate(window) {
                        Revalidation::Revalidated
                    } else {
                        Revalidation::NotInvalidated
                    };
                    assert_eq!(roc.revalidate(window), expected, "run {run} step {step}");
                    format!("revalidate {window}")
                }
                _ => {
                    // `carry_invalidation`, as the Rust lifecycle does it.
                    let to = 1 + rng.below(6);
                    rust.carry_invalidation(window, to);
                    if roc.revalidate(window) == Revalidation::Revalidated {
                        roc.invalidate(to);
                    }
                    format!("carry {window} to {to}")
                }
            };
            let expected: Vec<_> = rust.presences().collect();
            assert_eq!(
                roc.view(),
                expected,
                "run {run} step {step}: windows after {what}"
            );
            steps += 1;
        }
    }
    assert_eq!(
        AllocationCount::now().live(),
        before.live(),
        "Roc allocations left live"
    );
    eprintln!("lifecycle: {steps} kept-lifecycle steps identical, windows compared after each");
}
