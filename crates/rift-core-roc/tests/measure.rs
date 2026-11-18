//! Rough measurements of the Roc core against the Rust core. Run in release, explicitly:
//! `cargo test --release --features roc --test measure -- --ignored --nocapture`.
//!
//! Latency is per step. For Roc it is shown twice: on words (what crosses the boundary: the Roc
//! app parses the event and writes the effects) and as `RocCore::step` (the Rust side's wire
//! encode and decode added). The scaling run grows the number of windows the model tracks: if
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

/// Every side over the same sessions, each session on a fresh core (its clock starts at 0).
fn compare(sessions: &[Vec<Event<Plain>>]) {
    let steps: usize = sessions.iter().map(Vec::len).sum();
    let mut rust_samples = Vec::with_capacity(steps);
    let mut word_samples = Vec::with_capacity(steps);
    let mut step_samples = Vec::with_capacity(steps);
    let (mut allocated, mut live_during) = (0, 0);
    let before = AllocationCount::now();
    for events in sessions {
        let mut rust = RustCore::<Plain>::default();
        rust_samples.extend(time(events, |event| {
            std::hint::black_box(rust.step(event.clone()));
        }));
        let words: Vec<Vec<u64>> =
            events.iter().map(|event| wire::encode_event(event.clone())).collect();
        let start = AllocationCount::now();
        let mut roc = RocCore::default();
        word_samples.extend(time(&words, |words| {
            std::hint::black_box(roc.step_words(words));
        }));
        let during = AllocationCount::now();
        allocated += during.allocated - start.allocated;
        live_during = live_during.max(during.live() - start.live());
        drop(roc);
        let mut roc = RocCore::default();
        step_samples.extend(time(events, |event| {
            std::hint::black_box(roc.step(event.clone()));
        }));
    }
    let after = AllocationCount::now();
    show("rust", &stats(rust_samples));
    show("roc words", &stats(word_samples));
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
