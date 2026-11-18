//! Rough measurements of the Roc core against the Rust core. Run in release, explicitly:
//! `cargo test --release --test measure -- --ignored --nocapture`.
//!
//! Latency is per `step`, adapter included (the Roc side converts the event into the glue's
//! layout and copies the effects out). The scaling run grows the number of windows the model
//! tracks: if Roc lost the model's uniqueness it would copy the whole model on every step and
//! the per-step time would grow with the window count.

use std::time::{Duration, Instant};

use rift_core::plain::{Nanos, Plain, Rect, RequestId, WindowId};
use rift_core::{Core, Effect, Event, Initiative, InputClass, Pointer, RustCore, Want};
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
        "  {label:<5} mean {:>7.0?}  p50 {:>7.0?}  p99 {:>7.0?}  max {:>8.0?}",
        s.mean, s.p50, s.p99, s.max
    );
}

fn timed(core: &mut impl Core<Plain>, events: &[Event<Plain>]) -> Vec<Duration> {
    events
        .iter()
        .map(|event| {
            let event = event.clone();
            let start = Instant::now();
            let effects = core.step(event);
            let took = start.elapsed();
            std::hint::black_box(effects);
            took
        })
        .collect()
}

fn rect(x: f64) -> Rect { Rect { x, y: 0., w: 400., h: 300. } }

/// A steady workload over `windows` windows: each round asks every window somewhere new, its
/// app reports a step on the way and answers (half of them refusing), and the mode gate is asked; a press and release
/// and a timer tick (with every window's known frame) come round now and then.
fn workload(windows: u32, steps: usize) -> Vec<Event<Plain>> {
    let ids: Vec<WindowId> =
        (1..=windows).map(|idx| WindowId { pid: 7 + (idx % 5) as i32, idx }).collect();
    let mut events = Vec::with_capacity(steps);
    let mut now = 0u64;
    let mut round = 0u64;
    while events.len() < steps {
        round += 1;
        for (i, &window) in ids.iter().enumerate() {
            let x = (round * 10 + i as u64) as f64;
            now += 1_000_000;
            events.push(Event::Want {
                now: Nanos(now),
                request: RequestId(round as u32),
                window,
                desired: rect(x),
                known: rect(x - 10.),
                want: Want::Arrange,
            });
            events.push(Event::Report {
                window,
                frame: rect(x - 5.),
                known: rect(x),
            });
            // Every other app adjusts the frame: a refusal, which the model keeps until the
            // next desire, so it tracks about half the windows all the time.
            let answered = if i % 2 == 0 { rect(x + 7.) } else { rect(x) };
            events.push(Event::Answer {
                now: Nanos(now),
                request: RequestId(round as u32),
                seen: Some(RequestId(round as u32)),
                window,
                frame: answered,
            });
            events.push(Event::Gate {
                initiative: Initiative::UserInput(InputClass::Steering),
            });
            if i % 50 == 0 {
                events.push(Event::Pointer(Pointer::Pressing(window)));
                events.push(Event::Pointer(Pointer::Idle));
            }
        }
        now += 400_000_000;
        let known = ids.iter().map(|&window| (window, rect(0.))).collect();
        events.push(Event::Tick {
            now: Nanos(now),
            request: RequestId(round as u32),
            known,
        });
    }
    events.truncate(steps);
    events
}

#[test]
#[ignore]
fn measure() {
    let dir = std::env::var("RIFT_TRACE_DIR").ok();
    if let Some(dir) = dir {
        let mut events = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let trace: Vec<(Event<Plain>, Vec<Effect<Plain>>)> =
                serde_json::from_slice(&std::fs::read(entry.unwrap().path()).unwrap()).unwrap();
            events.extend(trace.into_iter().map(|(event, _)| event));
        }
        // One long-lived core per side over the whole corpus, as in a long rift session.
        eprintln!("fuzzer corpus: {} steps", events.len());
        show("rust", &stats(timed(&mut RustCore::<Plain>::default(), &events)));
        let before = AllocationCount::now();
        show("roc", &stats(timed(&mut RocCore::default(), &events)));
        let after = AllocationCount::now();
        eprintln!(
            "  roc allocations: {:.2} per step, live after drop {}",
            (after.allocated - before.allocated) as f64 / events.len() as f64,
            after.live() - before.live()
        );
    }

    for windows in [10, 100, 500, 2000] {
        let events = workload(windows, 400_000);
        eprintln!("synthetic, {windows} windows, {} steps:", events.len());
        show("rust", &stats(timed(&mut RustCore::<Plain>::default(), &events)));
        let before = AllocationCount::now();
        let mut roc = RocCore::default();
        let samples = timed(&mut roc, &events);
        let during = AllocationCount::now();
        drop(roc);
        let after = AllocationCount::now();
        show("roc", &stats(samples));
        eprintln!(
            "  roc allocations: {:.2} per step, live before drop {}, after drop {}",
            (during.allocated - before.allocated) as f64 / events.len() as f64,
            during.live() - before.live(),
            after.live() - before.live()
        );
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
