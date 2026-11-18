//! The boundary: one step function from an [`Event`] to the [`Effect`]s it decides.
//!
//! Events are facts the platform parsed (a report, an answer, the pointer's state, a desire
//! layout computed, a timer firing). Effects are data the platform carries out (a write, a
//! timer to arm) or records (what a report turned out to be). The model stays behind
//! [`Core`]: the platform never reads it, so whatever it needs comes out as an effect.

use crate::Kinds;
use crate::pointer::{Initiative, Pointer, may};
use crate::reconciler::{Answer, Note, Observation, Reconciler, Translate, Verdict, Want, Write};

/// A fact for the core.
#[derive(Debug, Clone, PartialEq)]
pub enum Event<K: Kinds> {
    /// The pointer is now doing this. (The platform still owns the press and drag model; this
    /// is its fact, sent when it changes.)
    Pointer(Pointer<K::Window>),
    /// Layout wants `window` at `desired`; the model last saw it at `known`.
    Want {
        now: K::Time,
        window: K::Window,
        desired: K::Frame,
        known: K::Frame,
        want: Want,
    },
    /// A modifier drag moved the dragged window's interactive frame to `frame`.
    Follow { window: K::Window, frame: K::Frame },
    /// The app answered its latest write to `window`: it is at `frame`.
    Answer {
        now: K::Time,
        window: K::Window,
        frame: K::Frame,
    },
    /// `window` reported `frame` unprompted; the model last saw it at `known`.
    Report {
        window: K::Window,
        frame: K::Frame,
        known: K::Frame,
    },
    /// The deadline timer fired. `known` is where the model last saw each window it knows
    /// (at least those whose writes are due); a window not listed is gone.
    Tick {
        now: K::Time,
        known: Vec<(K::Window, K::Frame)>,
    },
    /// Something outside the core (an animation, a topology change) took `window`'s frame, or
    /// the window is gone: nothing is outstanding.
    Forget { window: K::Window },
    /// May Rift take `initiative` now?
    Gate { initiative: Initiative },
}

/// A decision for the platform.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect<K: Kinds> {
    /// Send this frame to the window's app. The model takes the window to be there from now on.
    Write(Write<K>),
    /// A write is with the app; the desire waits behind it. The model takes the window to be
    /// at the desire from now on.
    Queued { window: K::Window },
    /// What the app's answer was.
    Answered { window: K::Window, answer: Answer },
    /// The answer came after Rift stopped waiting: it is news, and comes back as a
    /// [`Event::Report`].
    Late { window: K::Window },
    /// What an unprompted report was.
    Observed {
        window: K::Window,
        observation: Observation,
    },
    /// Something decided along the way, for the log.
    Note { window: K::Window, note: Note },
    /// Whether Rift may take `initiative`. Refused is discarded, never queued.
    Gate {
        initiative: Initiative,
        allowed: bool,
    },
    /// Arm the deadline timer for this time, or disarm it (`None`).
    Timer(Option<K::Time>),
}

impl<K: Kinds> Event<K> {
    /// The same event on another platform's types.
    pub fn translate<J: Kinds>(self, to: &impl Translate<K, J>) -> Event<J> {
        match self {
            Event::Pointer(pointer) => Event::Pointer(match pointer {
                Pointer::Idle => Pointer::Idle,
                Pointer::Pressing(window) => Pointer::Pressing(to.window(window)),
                Pointer::Dragging(window) => Pointer::Dragging(to.window(window)),
            }),
            Event::Want {
                now,
                window,
                desired,
                known,
                want,
            } => Event::Want {
                now: to.time(now),
                window: to.window(window),
                desired: to.frame(desired),
                known: to.frame(known),
                want,
            },
            Event::Follow { window, frame } => Event::Follow {
                window: to.window(window),
                frame: to.frame(frame),
            },
            Event::Answer { now, window, frame } => Event::Answer {
                now: to.time(now),
                window: to.window(window),
                frame: to.frame(frame),
            },
            Event::Report { window, frame, known } => Event::Report {
                window: to.window(window),
                frame: to.frame(frame),
                known: to.frame(known),
            },
            Event::Tick { now, known } => Event::Tick {
                now: to.time(now),
                known: known.into_iter().map(|(w, f)| (to.window(w), to.frame(f))).collect(),
            },
            Event::Forget { window } => Event::Forget { window: to.window(window) },
            Event::Gate { initiative } => Event::Gate { initiative },
        }
    }
}

impl<K: Kinds> Effect<K> {
    /// The same effect on another platform's types.
    pub fn translate<J: Kinds>(self, to: &impl Translate<K, J>) -> Effect<J> {
        match self {
            Effect::Write(write) => Effect::Write(write.translate(to)),
            Effect::Queued { window } => Effect::Queued { window: to.window(window) },
            Effect::Answered { window, answer } => Effect::Answered {
                window: to.window(window),
                answer,
            },
            Effect::Late { window } => Effect::Late { window: to.window(window) },
            Effect::Observed { window, observation } => Effect::Observed {
                window: to.window(window),
                observation,
            },
            Effect::Note { window, note } => Effect::Note {
                window: to.window(window),
                note,
            },
            Effect::Gate { initiative, allowed } => Effect::Gate { initiative, allowed },
            Effect::Timer(at) => Effect::Timer(at.map(|at| to.time(at))),
        }
    }
}

/// A core: the model behind one step function. The model is moved through `step` and never
/// lent out, so a foreign core can keep it unique.
pub trait Core<K: Kinds> {
    fn step(&mut self, event: Event<K>) -> Vec<Effect<K>>;
}

/// The core written in Rust.
#[derive(Debug)]
pub struct RustCore<K: Kinds> {
    reconciler: Reconciler<K>,
    pointer: Pointer<K::Window>,
    /// The deadline the platform's timer is armed for.
    armed: Option<K::Time>,
}

impl<K: Kinds> Default for RustCore<K> {
    fn default() -> Self {
        RustCore {
            reconciler: Reconciler::default(),
            pointer: Pointer::Idle,
            armed: None,
        }
    }
}

impl<K: Kinds> RustCore<K> {
    /// Read access for Rust callers that have not moved behind the boundary yet (tests, and the
    /// reactor's transaction-id check). Not part of the boundary: a foreign core has no such
    /// door.
    pub fn reconciler(&self) -> &Reconciler<K> { &self.reconciler }

    fn decide(&mut self, event: Event<K>) -> Vec<Effect<K>> {
        let pointer = self.pointer;
        match event {
            Event::Pointer(now) => {
                self.pointer = now;
                vec![]
            }
            Event::Want {
                now,
                window,
                desired,
                known,
                want,
            } => {
                let owner = pointer.owner(window);
                match self.reconciler.want(now, window, desired, known, want, owner) {
                    Verdict::Send(write) => vec![Effect::Write(write)],
                    Verdict::Queued => vec![Effect::Queued { window }],
                    Verdict::Nothing => vec![],
                }
            }
            Event::Follow { window, frame } => {
                vec![Effect::Write(self.reconciler.follow(window, frame))]
            }
            Event::Answer { now, window, frame } => {
                let owner = pointer.owner(window);
                match self.reconciler.answer(now, window, frame, owner) {
                    Some((answer, queued)) => std::iter::once(Effect::Answered { window, answer })
                        .chain(queued.map(Effect::Write))
                        .collect(),
                    None => vec![Effect::Late { window }],
                }
            }
            Event::Report { window, frame, known } => {
                let holder = pointer.holder(window);
                let observation = self.reconciler.observe(window, frame, known, holder);
                vec![Effect::Observed { window, observation }]
            }
            Event::Tick { now, known } => {
                let lookup = |window: K::Window| {
                    let (_, frame) = known.iter().find(|(w, _)| *w == window)?;
                    Some((*frame, pointer.owner(window)))
                };
                self.reconciler.expire(now, lookup).into_iter().map(Effect::Write).collect()
            }
            Event::Forget { window } => {
                self.reconciler.forget(window);
                vec![]
            }
            Event::Gate { initiative } => {
                vec![Effect::Gate {
                    initiative,
                    allowed: may(initiative, pointer),
                }]
            }
        }
    }
}

impl<K: Kinds> Core<K> for RustCore<K> {
    /// Decisions first, then the notes taken while deciding, then the timer if its deadline
    /// moved.
    fn step(&mut self, event: Event<K>) -> Vec<Effect<K>> {
        let mut effects = self.decide(event);
        effects.extend(
            self.reconciler
                .notes()
                .into_iter()
                .map(|(window, note)| Effect::Note { window, note }),
        );
        let next = self.reconciler.next_deadline();
        if next != self.armed {
            self.armed = next;
            effects.push(Effect::Timer(next));
        }
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plain::{Nanos, Plain, Rect, WindowId};
    use crate::reconciler::WRITE_DEADLINE;

    fn at(x: f64) -> Rect { Rect { x, y: 0., w: 400., h: 300. } }

    const W: WindowId = WindowId { pid: 7, idx: 1 };

    #[test]
    fn a_write_arms_the_timer_and_its_answer_disarms_it() {
        let mut core = RustCore::<Plain>::default();
        let effects = core.step(Event::Want {
            now: Nanos(0),
            window: W,
            desired: at(10.),
            known: at(0.),
            want: Want::Arrange,
        });
        let [Effect::Write(write), Effect::Timer(timer)] = effects.as_slice() else {
            panic!("expected a write and a timer, got {effects:?}");
        };
        assert_eq!((write.window(), write.frame()), (W, at(10.)));
        assert_eq!(*timer, Some(Nanos(0) + WRITE_DEADLINE));
        let effects = core.step(Event::Answer {
            now: Nanos(5),
            window: W,
            frame: at(10.),
        });
        assert_eq!(effects, vec![
            Effect::Answered {
                window: W,
                answer: Answer::Acked
            },
            Effect::Timer(None),
        ]);
    }

    #[test]
    fn a_desire_for_a_pressed_window_is_discarded_and_steering_input_refused() {
        let mut core = RustCore::<Plain>::default();
        assert_eq!(core.step(Event::Pointer(Pointer::Pressing(W))), vec![]);
        let effects = core.step(Event::Want {
            now: Nanos(0),
            window: W,
            desired: at(10.),
            known: at(0.),
            want: Want::Arrange,
        });
        assert_eq!(effects, vec![Effect::Note {
            window: W,
            note: Note::Discarded
        }]);
        let steering = Initiative::UserInput(crate::pointer::InputClass::Steering);
        assert_eq!(core.step(Event::Gate { initiative: steering }), vec![
            Effect::Gate {
                initiative: steering,
                allowed: false
            }
        ]);
    }
}
