//! The core on plain data: what a foreign core (Roc) sees, and what a recorded trace holds.
//! Every type here is a fixed-width integer, a float, or a record of them. [`Translate`] takes
//! the core's events and effects from the platform's types to these; [`wire`] puts them into
//! words and back (the Roc core's boundary, and the trace file format).

use std::ops::Add;
use std::time::Duration;

use crate::Kinds;
use crate::core::{Desire, Effect, Event, Plan};
use crate::reconciler::Write;

pub mod wire;

/// A window: its app's pid and its index within the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId {
    pub pid: i32,
    pub idx: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The id a write goes out under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(pub u32);

/// Monotonic nanoseconds since the platform started its clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Nanos(pub u64);

impl Add<Duration> for Nanos {
    type Output = Nanos;

    fn add(self, by: Duration) -> Nanos { Nanos(self.0 + by.as_nanos() as u64) }
}

/// A fact a decision may ask for, as a question on plain data carries it: what the platform
/// answered, or that the decision the platform ran never asked it. A question built from what
/// the Rust decision asked leaves the rest `Unasked`, and a decision reading one of those reads
/// a fact the Rust decision did not: it decides in a different order, and says so
/// ([`Unasked`]) instead of answering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fact<T> {
    Asked(T),
    Unasked,
}

/// A fact asked about a key (a window's visibility, a Space's activity): the keys asked and
/// what each answered. Any other key was not asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answers<K, V>(pub Vec<(K, V)>);

impl<K, V> Default for Answers<K, V> {
    fn default() -> Self { Answers(Vec::new()) }
}

/// A decision read a fact its question does not carry: `fact` is the fact's place among the
/// question's facts (its `FACTS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unasked {
    pub fact: u64,
}

/// What a decision has read of a question's facts so far.
#[derive(Debug, Clone, Copy)]
enum Read {
    Given,
    Missing(Unasked),
}

/// Reads a question's facts for a decision, remembering the first fact the question does not
/// carry. A decision reading it is given `stand_in` and runs on, and its answer is replaced by
/// the [`Unasked`] (the decision functions take plain closures, which cannot stop it).
pub(crate) struct Reading(std::cell::Cell<Read>);

impl Reading {
    pub(crate) fn new() -> Self { Reading(std::cell::Cell::new(Read::Given)) }

    fn missing(&self, fact: u64) {
        if let Read::Given = self.0.get() {
            self.0.set(Read::Missing(Unasked { fact }));
        }
    }

    /// The `fact`-th fact's answer.
    pub(crate) fn fact<T: Clone>(&self, given: &Fact<T>, fact: u64, stand_in: T) -> T {
        match given {
            Fact::Asked(answer) => answer.clone(),
            Fact::Unasked => {
                self.missing(fact);
                stand_in
            }
        }
    }

    /// The `fact`-th fact's answer for `key`.
    pub(crate) fn about<K: PartialEq, V: Clone>(
        &self,
        given: &Answers<K, V>,
        key: &K,
        fact: u64,
        stand_in: V,
    ) -> V {
        match given.0.iter().find(|(asked, _)| asked == key) {
            Some((_, answer)) => answer.clone(),
            None => {
                self.missing(fact);
                stand_in
            }
        }
    }

    /// The decision's answer, unless it read a fact it was not given.
    pub(crate) fn answer<A>(self, answer: A) -> Result<A, Unasked> {
        match self.0.get() {
            Read::Given => Ok(answer),
            Read::Missing(unasked) => Err(unasked),
        }
    }
}

impl Write<Plain> {
    /// A write a foreign core (the Roc one) decided, as it said it: the parse of its effect.
    /// The platform's own writes are only ever the core's.
    pub fn foreign(window: WindowId, frame: Rect, request: RequestId) -> Self {
        Write::new(window, frame, request)
    }
}

impl Plan<Plain> {
    /// A plan a foreign core (the Roc one) decided, as it said it: the parse of its effect.
    pub fn foreign(
        request: RequestId,
        positions: Vec<Write<Plain>>,
        frames: Vec<Write<Plain>>,
    ) -> Self {
        Plan { request, positions, frames }
    }
}

fn within(a: f64, tolerance: f64, b: f64) -> bool { (a - b).abs() < tolerance }

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plain;

impl Kinds for Plain {
    type Frame = Rect;
    type Request = RequestId;
    type Time = Nanos;
    type Window = WindowId;

    fn is_within(a: Rect, tolerance: f64, b: Rect) -> bool {
        within(a.x, tolerance, b.x)
            && within(a.y, tolerance, b.y)
            && within(a.w, tolerance, b.w)
            && within(a.h, tolerance, b.h)
    }

    fn after(time: Nanos, duration: Duration) -> Nanos { time + duration }

    fn same_size(a: Rect, b: Rect) -> bool { within(a.w, 0.1, b.w) && within(a.h, 0.1, b.h) }
}

/// A way from one platform's types to another's.
pub trait Translate<K: Kinds, J: Kinds> {
    fn window(&self, window: K::Window) -> J::Window;
    fn frame(&self, frame: K::Frame) -> J::Frame;
    fn time(&self, time: K::Time) -> J::Time;
    fn request(&self, request: K::Request) -> J::Request;
}

fn desires<K: Kinds, J: Kinds>(
    desires: Vec<Desire<K>>,
    to: &impl Translate<K, J>,
) -> Vec<Desire<J>> {
    desires
        .into_iter()
        .map(|Desire { window, frame, request }| Desire {
            window: to.window(window),
            frame: to.frame(frame),
            request: to.request(request),
        })
        .collect()
}

impl<K: Kinds> Event<K> {
    /// The same event on another platform's types.
    pub fn translate<J: Kinds>(self, to: &impl Translate<K, J>) -> Event<J> {
        let w = |window| to.window(window);
        match self {
            Event::Press { window, button } => Event::Press { window: window.map(w), button },
            Event::Drag { window } => Event::Drag { window: w(window) },
            Event::Release(release) => Event::Release(release),
            Event::LetGo => Event::LetGo,
            Event::Gone { window } => Event::Gone { window: w(window) },
            Event::Rekeyed { from, to: into } => Event::Rekeyed { from: w(from), to: w(into) },
            Event::Seen { window, frame } => Event::Seen {
                window: w(window),
                frame: to.frame(frame),
            },
            Event::Ceded { window } => Event::Ceded { window: w(window) },
            Event::Unseen { window } => Event::Unseen { window: w(window) },
            Event::Distrust { windows } => Event::Distrust {
                windows: windows.into_iter().map(w).collect(),
            },
            Event::Arrange { now, want, desires: wanted } => Event::Arrange {
                now: to.time(now),
                want,
                desires: desires(wanted, to),
            },
            Event::Animate { now, desires: wanted } => Event::Animate {
                now: to.time(now),
                desires: desires(wanted, to),
            },
            Event::AnimationEnded { now, request } => Event::AnimationEnded {
                now: to.time(now),
                request: to.request(request),
            },
            Event::AnimationStopped { window, request, at } => Event::AnimationStopped {
                window: w(window),
                request: to.request(request),
                at: at.map(|frame| to.frame(frame)),
            },
            Event::Follow { request, window, frame } => Event::Follow {
                request: to.request(request),
                window: w(window),
                frame: to.frame(frame),
            },
            Event::Observe {
                now,
                request,
                window,
                frame,
                as_of,
            } => Event::Observe {
                now: to.time(now),
                request: to.request(request),
                window: w(window),
                frame: to.frame(frame),
                as_of: as_of.map(|request| to.request(request)),
            },
            Event::Tick { now, request } => Event::Tick {
                now: to.time(now),
                request: to.request(request),
            },
            Event::Forget { window } => Event::Forget { window: w(window) },
            Event::SentOutside { window, request } => Event::SentOutside {
                window: w(window),
                request: to.request(request),
            },
        }
    }
}

impl<K: Kinds> Effect<K> {
    /// The same effect on another platform's types.
    pub fn translate<J: Kinds>(self, to: &impl Translate<K, J>) -> Effect<J> {
        let w = |window| to.window(window);
        match self {
            Effect::Write(write) => Effect::Write(write.translate(to)),
            Effect::Animate {
                window,
                from,
                to: target,
                request,
            } => Effect::Animate {
                window: w(window),
                from: to.frame(from),
                to: to.frame(target),
                request: to.request(request),
            },
            Effect::Pending { window, target } => Effect::Pending {
                window: w(window),
                target: target.map(|frame| to.frame(frame)),
            },
            Effect::Plan(Plan { request, positions, frames }) => Effect::Plan(Plan {
                request: to.request(request),
                positions: positions.into_iter().map(|write| write.translate(to)).collect(),
                frames: frames.into_iter().map(|write| write.translate(to)).collect(),
            }),
            Effect::Queued { window } => Effect::Queued { window: w(window) },
            Effect::Seen { window, frame } => Effect::Seen {
                window: w(window),
                frame: to.frame(frame),
            },
            Effect::Replied { window, reply } => Effect::Replied { window: w(window), reply },
            Effect::Note { window, note } => Effect::Note { window: w(window), note },
            Effect::Timer(at) => Effect::Timer(at.map(|time| to.time(time))),
            Effect::Grip { window } => Effect::Grip { window: w(window) },
            Effect::DragBegan { window } => Effect::DragBegan { window: w(window) },
            Effect::Ungrip { window, why, changed } => Effect::Ungrip {
                window: w(window),
                why,
                changed: changed.into_iter().map(w).collect(),
            },
            Effect::Regrip { from, to: into } => Effect::Regrip { from: w(from), to: w(into) },
        }
    }
}

impl<K: Kinds> Write<K> {
    /// The same write on another platform's types.
    pub fn translate<J: Kinds>(self, to: &impl Translate<K, J>) -> Write<J> {
        Write::new(
            to.window(self.window()),
            to.frame(self.frame()),
            to.request(self.request()),
        )
    }
}
