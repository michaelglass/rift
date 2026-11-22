//! Rough measurements of the Roc core against the Rust core. Run in release, explicitly:
//! `cargo test --release --features roc --test measure -- --ignored --nocapture`.
//!
//! Latency is per step, for Roc as `RocCore::step` (the event's and effects' conversion to and
//! from the typed boundary included). The scaling run grows the number of windows the model
//! tracks: if
//! Roc lost the model's uniqueness it would copy the whole model every step, and the per-step
//! time would grow with the window count.

use std::time::{Duration, Instant};

use rift_core::plain::{Nanos, Plain, Rect, RequestId, WindowId, wire};
use rift_core::{Button, Desire, Event, Release, RustCore, Want};
use rift_core_roc::{AllocationCount, RocCore};

struct Stats {
    mean: Duration,
    p50: Duration,
    p99: Duration,
    max: Duration,
}

fn stats(mut samples: Vec<Duration>) -> Stats {
    samples.sort();
    let total: Duration = samples.iter().sum();
    let at = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize];
    Stats {
        mean: total / samples.len() as u32,
        p50: at(0.5),
        p99: at(0.99),
        max: *samples.last().unwrap(),
    }
}

fn show(label: &str, s: &Stats) {
    eprintln!(
        "  {label:<9} mean {:>7.0?}  p50 {:>7.0?}  p99 {:>7.0?}  max {:>8.0?}",
        s.mean, s.p50, s.p99, s.max
    );
}

fn time<T>(items: &[T], mut step: impl FnMut(&T)) -> Vec<Duration> {
    items
        .iter()
        .map(|item| {
            let start = Instant::now();
            step(item);
            start.elapsed()
        })
        .collect()
}

/// Both cores over the same sessions, each session on a fresh core (its clock starts at 0).
/// The Roc step includes the event's and effects' conversion to and from the typed boundary.
fn compare(sessions: &[Vec<Event<Plain>>]) {
    let steps: usize = sessions.iter().map(Vec::len).sum();
    let mut rust_samples = Vec::with_capacity(steps);
    let mut step_samples = Vec::with_capacity(steps);
    let (mut allocated, mut live_during) = (0, 0);
    let before = AllocationCount::now();
    for events in sessions {
        let mut rust = RustCore::<Plain>::default();
        rust_samples.extend(time(events, |event| {
            std::hint::black_box(rust.step(event.clone()));
        }));
        let start = AllocationCount::now();
        let mut roc = RocCore::default();
        step_samples.extend(time(events, |event| {
            std::hint::black_box(roc.step(event.clone()));
        }));
        let during = AllocationCount::now();
        allocated += during.allocated - start.allocated;
        live_during = live_during.max(during.live() - start.live());
    }
    let after = AllocationCount::now();
    show("rust", &stats(rust_samples));
    show("roc step", &stats(step_samples));
    eprintln!(
        "  roc allocations: {:.2} per step, at most {live_during} live in a session, {} after \
         every core was dropped",
        allocated as f64 / steps as f64,
        after.live() - before.live()
    );
}

fn rect(x: f64) -> Rect { Rect { x, y: 0., w: 400., h: 300. } }

/// A steady workload over `windows` windows: each round arranges every window somewhere new
/// (one plan), each app reports a frame on the way and then answers (every other one
/// refusing), a press and release come round now and then, and a tick ends the round after the
/// write deadline.
fn workload(windows: u32, steps: usize) -> Vec<Event<Plain>> {
    let ids: Vec<WindowId> =
        (1..=windows).map(|idx| WindowId { pid: 7 + (idx % 5) as i32, idx }).collect();
    let mut events: Vec<Event<Plain>> =
        ids.iter().map(|&window| Event::Seen { window, frame: rect(0.) }).collect();
    let mut now = 0u64;
    let mut round = 0u32;
    while events.len() < steps {
        round += 1;
        now += 1_000_000;
        let request = RequestId(round);
        let x = |i: usize| f64::from(round) * 10. + i as f64;
        events.push(Event::Arrange {
            now: Nanos(now),
            want: Want::Arrange,
            desires: ids
                .iter()
                .enumerate()
                .map(|(i, &window)| Desire {
                    window,
                    frame: rect(x(i)),
                    request,
                })
                .collect(),
        });
        for (i, &window) in ids.iter().enumerate() {
            now += 1_000_000;
            events.push(Event::Observe {
                now: Nanos(now),
                request,
                window,
                frame: rect(x(i) - 5.),
                as_of: None,
            });
            let answered = if i % 2 == 0 {
                rect(x(i) + 7.)
            } else {
                rect(x(i))
            };
            events.push(Event::Observe {
                now: Nanos(now),
                request,
                window,
                frame: answered,
                as_of: Some(request),
            });
            if i % 50 == 0 {
                events.push(Event::Press {
                    window: Some(window),
                    button: Button::Left,
                });
                events.push(Event::Release(Release::Button(Button::Left)));
            }
        }
        now += 400_000_000;
        events.push(Event::Tick { now: Nanos(now), request });
    }
    events.truncate(steps);
    events
}

fn traces(dir: &str) -> Vec<Vec<Event<Plain>>> {
    let mut sessions = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        let words: Vec<u64> = bytes
            .chunks_exact(8)
            .map(|word| u64::from_le_bytes(word.try_into().unwrap()))
            .collect();
        sessions.push(
            wire::decode_trace(&words)
                .unwrap()
                .into_iter()
                .map(|(event, _)| event)
                .collect(),
        );
    }
    sessions
}

#[test]
#[ignore]
fn measure() {
    if let Ok(dir) = std::env::var("RIFT_TRACE_DIR") {
        // A fresh core per seed: each seed's clock starts at 0, and a core carried across seeds
        // would see time run backwards (a write's deadline then never comes).
        let sessions = traces(&dir);
        let steps: usize = sessions.iter().map(Vec::len).sum();
        eprintln!("fuzzer corpus: {} seeds, {steps} steps", sessions.len());
        compare(&sessions);
    }
    for windows in [10, 100, 500, 2000] {
        let events = workload(windows, 400_000);
        eprintln!("synthetic, {windows} windows, {} steps:", events.len());
        compare(&[events]);
    }
}

#[test]
#[ignore]
fn a_million_steps_leave_nothing_behind() {
    let events = workload(500, 1_000_000);
    let before = AllocationCount::now();
    let mut roc = RocCore::default();
    let mut live = Vec::new();
    for chunk in events.chunks(100_000) {
        for event in chunk {
            std::hint::black_box(roc.step(event.clone()));
        }
        live.push(AllocationCount::now().live() - before.live());
    }
    drop(roc);
    let after = AllocationCount::now();
    eprintln!("live Roc allocations every 100k steps: {live:?}");
    assert_eq!(after.live(), before.live(), "leaked");
    assert!(
        live.windows(2).all(|pair| pair[1] <= pair[0] + 16),
        "live allocations grow: {live:?}"
    );
}

/// Times one model-free question two ways: the Rust answer alone, and the Roc core's answer
/// (`roc`, asked through its typed entry), each as
/// words; the Roc time is what `roc_path::check` adds on rift's path, the question's
/// conversion included.
fn question<Q: Clone>(
    label: &str,
    questions: &[Q],
    rust: impl Fn(&Q) -> Vec<u64>,
    roc: impl Fn(Q) -> Vec<u64>,
) {
    for question in questions {
        assert_eq!(
            roc(question.clone()),
            rust(question),
            "{label}: the cores disagree"
        );
    }
    let rounds = 200_000 / questions.len();
    let all: Vec<Q> = questions.iter().cycle().take(rounds * questions.len()).cloned().collect();
    let rust_only = time(&all, |question| {
        std::hint::black_box(rust(question));
    });
    let roc_only = time(&all, |question| {
        std::hint::black_box(roc(question.clone()));
    });
    eprintln!("{label}:");
    show("rust", &stats(rust_only));
    show("roc", &stats(roc_only));
}

/// The hot focus questions S1 puts on rift's path (`focus_candidate` on every focus pass,
/// `untracked_focus` on layout responses), at the sizes a workspace reaches.
#[test]
#[ignore]
fn measure_focus_questions() {
    use rift_core::focus::plain::{Candidate, Untracked};
    use rift_core::plain::{Answers, Fact};

    for windows in [1u64, 10, 50] {
        let in_workspace: Vec<u64> = (1..=windows).collect();
        // Every window's visibility, as a question on full facts carries it.
        let visible = |shown: fn(u64) -> bool| {
            Answers((1..=windows + 1).map(|window| (window, shown(window))).collect())
        };
        let candidates = vec![
            // The preferred window is visible: answered at once.
            Candidate {
                preferred: Some(1),
                last_focused: Fact::Asked(Some(2)),
                in_workspace: Fact::Asked(in_workspace.clone()),
                visible: visible(|window| window % 3 != 0),
            },
            // Neither preferred nor last focused is visible: the workspace is scanned.
            Candidate {
                preferred: Some(windows + 1),
                last_focused: Fact::Asked(None),
                in_workspace: Fact::Asked(in_workspace.iter().rev().copied().collect()),
                visible: visible(|window| window % 3 != 0),
            },
            // Nothing visible: the whole workspace is scanned for nothing.
            Candidate {
                preferred: None,
                last_focused: Fact::Asked(None),
                in_workspace: Fact::Asked(in_workspace.clone()),
                visible: visible(|_| false),
            },
        ];
        question(
            &format!("focus_candidate, {windows} windows"),
            &candidates,
            |q| wire::encode_maybe_space(q.answer().expect("full facts")),
            |q| {
                wire::encode_maybe_space(
                    rift_core_roc::focus::focus_candidate(q).expect("the facts it asks"),
                )
            },
        );
    }
    let untracked = vec![
        Untracked {
            under_cursor: None,
            tracked: Fact::Asked(false),
            known: Fact::Asked(None),
        },
        Untracked {
            under_cursor: Some(40),
            tracked: Fact::Asked(true),
            known: Fact::Asked(None),
        },
        Untracked {
            under_cursor: Some(41),
            tracked: Fact::Asked(false),
            known: Fact::Asked(Some((0, 700))),
        },
        Untracked {
            under_cursor: Some(42),
            tracked: Fact::Asked(false),
            known: Fact::Asked(Some((25, 0))),
        },
    ];
    question(
        "untracked_focus",
        &untracked,
        |q| wire::encode_untracked_focus(q.answer().expect("full facts")),
        |q| {
            wire::encode_untracked_focus(
                rift_core_roc::focus::untracked_focus(q).expect("the facts it asks"),
            )
        },
    );
}

/// The hot lifecycle questions S3 puts on rift's path: `is_native_transient` and
/// `keeps_transient` (every event touching a window's transient entry) and `retirement` (every
/// window an inventory is reconciled against).
#[test]
#[ignore]
fn measure_lifecycle_questions() {
    use rift_core::lifecycle::plain::{Inventory, KeepsTransient, NativeTransient};
    use rift_core::lifecycle::{
        AxValidity, Departure, FullscreenRecord, InventoryFacts, InventoryView, Minimize,
        NativeEvidence, NativeIdentity, NativeLook, Ordering, Report, Suitability, TransientOrigin,
    };
    use rift_core::plain::Fact;

    let native = vec![
        NativeTransient {
            entry: None,
            owner: 7,
            exempt: Fact::Unasked,
        },
        NativeTransient {
            entry: Some((TransientOrigin::Native, 7)),
            owner: 7,
            exempt: Fact::Asked(false),
        },
        NativeTransient {
            entry: Some((TransientOrigin::Heuristic, 7)),
            owner: 7,
            exempt: Fact::Unasked,
        },
    ];
    question(
        "is_native_transient",
        &native,
        |q| wire::encode_verdict(q.answer().expect("the facts it asks")),
        |q| {
            wire::encode_verdict(
                rift_core_roc::lifecycle::is_native_transient(q).expect("the facts it asks"),
            )
        },
    );
    let keeps = vec![
        KeepsTransient {
            departure: Departure::Destroyed(Some(40)),
            id: 40,
            origin: TransientOrigin::Heuristic,
            owner: 7,
        },
        KeepsTransient {
            departure: Departure::Closed(41),
            id: 40,
            origin: TransientOrigin::Native,
            owner: 7,
        },
        KeepsTransient {
            departure: Departure::AppGone(7),
            id: 40,
            origin: TransientOrigin::Native,
            owner: 7,
        },
    ];
    question(
        "keeps_transient",
        &keeps,
        |q| wire::encode_verdict(q.answer()),
        |q| wire::encode_verdict(rift_core_roc::lifecycle::keeps_transient(q)),
    );
    let facts = |report| InventoryFacts {
        report,
        view: InventoryView::InView,
        minimize: Some(Minimize::Shown),
        fullscreen: FullscreenRecord::NotRecorded,
        identity: NativeIdentity::Known(40),
        validity: AxValidity::Valid,
    };
    let evidence = NativeEvidence {
        suitability: Some(Suitability::Suitable),
        ordering: Some(Ordering::OrderedIn),
        look: Some(NativeLook::Window),
    };
    // Reported (kept at once), and omitted with the evidence asked.
    let inventories = vec![
        Inventory {
            facts: facts(Report::Reported),
            evidence: Fact::Unasked,
        },
        Inventory {
            facts: facts(Report::Omitted),
            evidence: Fact::Asked(evidence),
        },
    ];
    question(
        "retirement",
        &inventories,
        |q| wire::encode_retirement(q.answer().expect("the facts it asks")),
        |q| {
            wire::encode_retirement(
                rift_core_roc::lifecycle::retirement(q).expect("the facts it asks"),
            )
        },
    );
}

/// The lifecycle kept between events, Rust against the Roc twin: per step on AX-style events
/// (fact changes, fresh readings, invalidations and revalidations of tracked windows), after
/// filling it with `windows` windows; and the cost of reading every window back (the view the
/// twin is compared by).
#[test]
#[ignore]
fn measure_lifecycle_twin() {
    use rift_core::lifecycle::{
        AdmissionFacts, AxKind, FactChange, Lifecycle, LifecycleEvent, Manageability, Minimize,
        NativeKind, Rule,
    };
    use rift_core_roc::RocLifecycle;
    #[derive(Clone, Copy)]
    enum Op {
        Step(LifecycleEvent<u64>),
        Invalidate(u64),
        Revalidate(u64),
    }
    let facts = AdmissionFacts {
        manageability: Manageability::Manageable,
        rule: Rule::NoRule,
        ax: AxKind::Ordinary,
        native: NativeKind::Ordinary,
        minimize: Minimize::Shown,
    };
    for windows in [10u64, 100, 1000] {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64 ^ windows;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let ops: Vec<Op> = (0..20_000)
            .map(|_| {
                let window = 1 + next() % windows;
                match next() % 8 {
                    0 | 1 => Op::Step(LifecycleEvent::Changed {
                        window,
                        change: FactChange::Minimize(if next() % 2 == 0 {
                            Minimize::Shown
                        } else {
                            Minimize::Minimized
                        }),
                    }),
                    2 | 3 => Op::Step(LifecycleEvent::Changed {
                        window,
                        change: FactChange::Ax(AxKind::Ordinary),
                    }),
                    4 | 5 => Op::Step(LifecycleEvent::Live { window, facts }),
                    6 => Op::Invalidate(window),
                    _ => Op::Revalidate(window),
                }
            })
            .collect();
        let mut rust = Lifecycle::<u64>::default();
        let mut roc = RocLifecycle::default();
        for window in 1..=windows {
            rust.step(LifecycleEvent::Live { window, facts });
            roc.step(LifecycleEvent::Live { window, facts });
        }
        let rust_times = time(&ops, |&op| {
            std::hint::black_box(match op {
                Op::Step(event) => rust.step(event).is_some(),
                Op::Invalidate(window) => {
                    rust.invalidate(window) == rift_core_roc::Invalidation::Invalidated
                }
                Op::Revalidate(window) => {
                    rust.revalidate(window) == rift_core_roc::Revalidation::Revalidated
                }
            });
        });
        let before = AllocationCount::now();
        let roc_times = time(&ops, |&op| {
            std::hint::black_box(match op {
                Op::Step(event) => roc.step(event).is_some(),
                Op::Invalidate(window) => {
                    roc.invalidate(window) == rift_core_roc::Invalidation::Invalidated
                }
                Op::Revalidate(window) => {
                    roc.revalidate(window) == rift_core_roc::Revalidation::Revalidated
                }
            });
        });
        let allocated = AllocationCount::now().allocated - before.allocated;
        let views: Vec<()> = vec![(); 200];
        let rust_views = time(&views, |_| {
            std::hint::black_box(rust.presences().collect::<Vec<_>>());
        });
        let roc_views = time(&views, |_| {
            std::hint::black_box(roc.view());
        });
        eprintln!(
            "lifecycle twin, {windows} windows ({:.2} Roc allocations per step):",
            allocated as f64 / ops.len() as f64
        );
        show("rust", &stats(rust_times));
        show("roc", &stats(roc_times));
        show("rust view", &stats(rust_views));
        show("roc view", &stats(roc_views));
    }
}

/// Who places each window, kept between events, Rust against the Roc twin: per change (drags
/// holding and placing, claims, reassignments, windows going unowned and owned again) after
/// owning `windows` windows; and reading every owner back.
#[test]
#[ignore]
fn measure_ownerships_twin() {
    use rift_core::workspaces::{Destination, Owned, Ownership, OwnershipEvent, Ownerships};
    use rift_core_roc::RocOwnerships;
    for windows in [10u64, 100, 1000] {
        let mut seed = 0x2545_F491_4F6C_DD1Du64 ^ windows;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let events: Vec<OwnershipEvent<u64, u64, u64>> = (0..20_000)
            .map(|_| {
                let window = 1 + next() % windows;
                let owned = match next() % 6 {
                    0 => Owned::Owner(Ownership::Held),
                    1 | 2 => Owned::Owner(Ownership::Assigned(1 + next() % 9)),
                    3 => Owned::Owner(Ownership::Claimed {
                        owner: 1,
                        destination: Destination::Returns(1 + next() % 9),
                    }),
                    4 => Owned::Unowned,
                    _ => Owned::Owner(Ownership::Assigned(1)),
                };
                OwnershipEvent::Owned { window, owned }
            })
            .collect();
        let mut rust = Ownerships::<u64, u64, u64>::default();
        let mut roc = RocOwnerships::default();
        for window in 1..=windows {
            let event = OwnershipEvent::Owned {
                window,
                owned: Owned::Owner(Ownership::Assigned(1)),
            };
            rust.step(event);
            roc.step(event);
        }
        let rust_times = time(&events, |&event| {
            rust.step(event);
            std::hint::black_box(rust.owned(1));
        });
        let before = AllocationCount::now();
        let roc_times = time(&events, |&event| {
            std::hint::black_box(roc.step(event));
        });
        let allocated = AllocationCount::now().allocated - before.allocated;
        let views: Vec<()> = vec![(); 200];
        let rust_views = time(&views, |_| {
            std::hint::black_box(rust.owners().collect::<Vec<_>>());
        });
        let roc_views = time(&views, |_| {
            std::hint::black_box(roc.view());
        });
        eprintln!(
            "ownerships twin, {windows} windows ({:.2} Roc allocations per step):",
            allocated as f64 / events.len() as f64
        );
        show("rust", &stats(rust_times));
        show("roc", &stats(roc_times));
        show("rust view", &stats(rust_views));
        show("roc view", &stats(roc_views));
    }
}

/// The workspaces' topology kept between events, Rust against the Roc twin: per change
/// (activations as workspaces switch, moves between Spaces) on a catalog of `workspaces` over
/// 3 Spaces; and reading the catalog and activations back.
#[test]
#[ignore]
fn measure_topology_twin() {
    use rift_core::workspaces::{Activation, Topology, TopologyEvent};
    use rift_core_roc::RocTopology;
    for workspaces in [5u64, 20, 100] {
        let mut seed = 0xD1B5_4A32_D192_ED03u64 ^ workspaces;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let events: Vec<TopologyEvent<u64, u64>> = (0..20_000)
            .map(|_| match next() % 4 {
                0 => TopologyEvent::Moved {
                    workspace: 1 + next() % workspaces,
                    space: 1 + next() % 3,
                },
                _ => TopologyEvent::Activated {
                    space: 1 + next() % 3,
                    activation: Activation::Showing {
                        current: 1 + next() % workspaces,
                        previous: Some(1 + next() % workspaces),
                        remembered: None,
                    },
                },
            })
            .collect();
        let mut rust = Topology::<u64, u64>::default();
        let mut roc = RocTopology::default();
        for workspace in 1..=workspaces {
            let event = TopologyEvent::Created {
                workspace,
                space: 1 + workspace % 3,
            };
            rust.step(event);
            roc.step(event);
        }
        let rust_times = time(&events, |&event| {
            rust.step(event);
            std::hint::black_box(&rust);
        });
        let before = AllocationCount::now();
        let roc_times = time(&events, |&event| {
            roc.step(event);
        });
        let allocated = AllocationCount::now().allocated - before.allocated;
        let views: Vec<()> = vec![(); 200];
        let rust_views = time(&views, |_| {
            std::hint::black_box((rust.catalog().to_vec(), rust.activations().to_vec()));
        });
        let roc_views = time(&views, |_| {
            std::hint::black_box(roc.view());
        });
        eprintln!(
            "topology twin, {workspaces} workspaces ({:.2} Roc allocations per step):",
            allocated as f64 / events.len() as f64
        );
        show("rust", &stats(rust_times));
        show("roc", &stats(roc_times));
        show("rust view", &stats(rust_views));
        show("roc view", &stats(roc_views));
    }
}

/// The mode table's verdict, Rust against Roc, over a spread of states and initiatives (hover
/// is asked at mouse rate while the pointer moves).
#[test]
#[ignore]
fn measure_mode_verdict() {
    use rift_core::Pointer;
    use rift_core::modes::{
        FocusClaim, INITIATIVES, Menu, MissionControl, Modal, Modes, Topology, UserSpaces,
        Viewport, Wake, WorkspaceSwitch,
    };
    let questions: Vec<_> = (0u32..512)
        .flat_map(|bits| {
            let on = |i: u32| bits & (1 << i) != 0;
            let modes = Modes {
                pointer: if on(0) {
                    Pointer::Dragging(3)
                } else {
                    Pointer::Idle
                },
                menu: if on(1) { Menu::Open } else { Menu::Closed },
                mission_control: if on(2) {
                    MissionControl::Active
                } else {
                    MissionControl::Inactive
                },
                modal: if on(3) {
                    Modal::Frontmost
                } else {
                    Modal::Clear
                },
                viewport: if on(4) {
                    Viewport::Gesturing
                } else {
                    Viewport::Free
                },
                focus_claim: if on(5) {
                    FocusClaim::PausesHover
                } else {
                    FocusClaim::Free
                },
                user_spaces: UserSpaces::Shown,
                wake: if on(6) {
                    Wake::AwaitingInput
                } else {
                    Wake::UserPresent
                },
                workspace_switch: if on(7) {
                    WorkspaceSwitch::Switching
                } else {
                    WorkspaceSwitch::Idle
                },
                topology: if on(8) {
                    Topology::Unsettled
                } else {
                    Topology::Settled
                },
            };
            INITIATIVES.into_iter().map(move |initiative| (modes, initiative))
        })
        .collect();
    let rust = time(&questions, |&(modes, initiative)| {
        std::hint::black_box(modes.verdict(initiative));
    });
    let before = AllocationCount::now();
    let roc = time(&questions, |&(modes, initiative)| {
        std::hint::black_box(rift_core_roc::modes::verdict(modes, initiative));
    });
    let allocated = AllocationCount::now().allocated - before.allocated;
    eprintln!(
        "mode verdict ({} questions, {allocated} Roc allocations):",
        questions.len()
    );
    show("rust", &stats(rust));
    show("roc", &stats(roc));
}
