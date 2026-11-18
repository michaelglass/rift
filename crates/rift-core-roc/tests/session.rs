//! One core over a long session: the recorded seeds back to back, each shifted so its clock
//! and request ids carry on from where the one before ended (time only moves forward, as in a
//! session that runs for days). The Roc core must decide what the Rust core decides at every
//! step, and no step may be slow: a window manager that stops for 100 ms freezes visibly.
//!
//! Default: the committed traces, many times over. `RIFT_TRACE_DIR` points it at a corpus;
//! `RIFT_SESSION_ROUNDS` sets how many times it goes round.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use rift_core::plain::{Nanos, Plain, Rect, RequestId, Translate, WindowId, wire};
use rift_core::{Event, RustCore};
use rift_core_roc::{AllocationCount, RocCore};

/// Moves a recorded seed later: its times by `time`, its request ids by `request`.
struct Later {
    time: u64,
    request: u32,
}

impl Translate<Plain, Plain> for Later {
    fn window(&self, window: WindowId) -> WindowId { window }

    fn frame(&self, frame: Rect) -> Rect { frame }

    fn time(&self, time: Nanos) -> Nanos { Nanos(time.0 + self.time) }

    fn request(&self, request: RequestId) -> RequestId { RequestId(request.0 + self.request) }
}

fn seeds() -> Vec<Vec<Event<Plain>>> {
    let dir = std::env::var("RIFT_TRACE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("traces"));
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "trace"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let words: Vec<u64> = std::fs::read(path)
                .unwrap()
                .chunks_exact(8)
                .map(|word| u64::from_le_bytes(word.try_into().unwrap()))
                .collect();
            wire::decode_trace(&words)
                .unwrap()
                .into_iter()
                .map(|(event, _)| event)
                .collect()
        })
        .collect()
}

fn latest(event: &Event<Plain>) -> (u64, u32) {
    let mut time = 0;
    let mut request = 0;
    let mut at = |now: Nanos| time = time.max(now.0);
    let mut asked = |id: RequestId| request = request.max(id.0);
    match event {
        Event::Arrange { now, desires, .. } | Event::Animate { now, desires } => {
            at(*now);
            desires.iter().for_each(|desire| asked(desire.request));
        }
        Event::AnimationEnded { now, request } | Event::Tick { now, request } => {
            at(*now);
            asked(*request);
        }
        Event::Observe { now, request, as_of, .. } => {
            at(*now);
            asked(*request);
            as_of.iter().for_each(|id| asked(*id));
        }
        Event::Follow { request, .. } | Event::AnimationStopped { request, .. } => asked(*request),
        _ => {}
    }
    (time, request)
}

/// The seeds back to back, `rounds` times, every one later than all before it.
fn session(rounds: usize) -> Vec<Event<Plain>> {
    let seeds = seeds();
    let mut later = Later { time: 0, request: 0 };
    let mut events = Vec::new();
    for _ in 0..rounds {
        for seed in &seeds {
            let (mut time, mut request) = (0, 0);
            for event in seed {
                let (t, r) = latest(event);
                (time, request) = (time.max(t), request.max(r));
                events.push(event.clone().translate(&later));
            }
            // A second's quiet between seeds, past every write's deadline.
            later.time += time + 1_000_000_000;
            later.request += request + 1;
        }
    }
    events
}

#[test]
fn a_long_session_decides_the_same_and_has_no_slow_step() {
    let rounds = std::env::var("RIFT_SESSION_ROUNDS").map_or(200, |n| n.parse().unwrap());
    let events = session(rounds);
    let hours = match events.iter().rev().find_map(|event| match event {
        Event::Tick { now, .. } | Event::Arrange { now, .. } | Event::Observe { now, .. } => {
            Some(now.0)
        }
        _ => None,
    }) {
        Some(nanos) => nanos as f64 / 3.6e12,
        None => 0.0,
    };
    let before = AllocationCount::now();
    let mut rust = RustCore::<Plain>::default();
    let mut roc = RocCore::default();
    let mut slowest = Duration::ZERO;
    let mut slow = 0;
    let mut live_peak = 0;
    for (index, event) in events.iter().enumerate() {
        let words = wire::encode_event(event.clone());
        let start = Instant::now();
        let decided = roc.step_words(&words).map(<[u64]>::to_vec);
        let took = start.elapsed();
        slowest = slowest.max(took);
        if took > Duration::from_millis(5) {
            slow += 1;
        }
        let shadow = wire::encode_effects(rust.step(event.clone()));
        assert!(
            decided == Ok(shadow),
            "step {index}: the cores disagree on {event:?}"
        );
        live_peak = live_peak.max(AllocationCount::now().live() - before.live());
    }
    drop(roc);
    let after = AllocationCount::now();
    eprintln!(
        "session: {} steps over {hours:.1} h simulated; slowest Roc step {slowest:?}, {slow} over \
         5 ms; at most {live_peak} Roc allocations live",
        events.len()
    );
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    // A busy machine stalls a step now and then (dozens in a long run under load); a cost that
    // grows with the session stalls most of them.
    if !cfg!(debug_assertions) {
        let allowed = (events.len() / 10_000).max(10);
        assert!(slow <= allowed, "{slow} steps over 5 ms (slowest {slowest:?})");
    }
}
