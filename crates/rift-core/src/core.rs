//! The boundary: one step function from an [`Event`] to the [`Effect`]s it decides.
//!
//! Events are facts the platform parsed (a report, an answer, the pointer's state, a desire
//! layout computed, a timer firing). Effects are data the platform carries out (a write, a
//! timer to arm) or records (what a report turned out to be). The model stays behind
//! [`Core`]: the platform never reads it, so whatever it needs comes out as an effect.
//!
//! Every event that may lead to a write carries the `request` the platform will send its
//! writes under; each [`Write`] says which, and the app's answer names it back.

use std::collections::HashMap;

use crate::Kinds;
use crate::pointer::{Button, Grip, Initiative, Pointer, Release, Ungrip, may};
use crate::reconciler::{
    Answer, Note, Observation, Reconciler, Reply, Translate, Verdict, Want, Write,
};

/// A fact for the core.
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(bound(
        serialize = "K::Window: serde::Serialize, K::Frame: serde::Serialize, K::Time: serde::Serialize, K::Request: serde::Serialize",
        deserialize = "K::Window: serde::Deserialize<'de>, K::Frame: serde::Deserialize<'de>, K::Time: serde::Deserialize<'de>, K::Request: serde::Deserialize<'de>"
    ))
)]
#[derive(Debug, Clone, PartialEq)]
pub enum Event<K: Kinds> {
    /// `button` went down on `window` (a tracked window Rift manages), or on none. One hand:
    /// whatever it held is let go first.
    Press {
        window: Option<K::Window>,
        button: Button,
    },
    /// The pressed window's movement is the user's drag (its first move under the pointer, or
    /// a modifier press). A drag starts only from a press of that window.
    Drag { window: K::Window },
    /// The pointer was released.
    Release(Release),
    /// The hand comes off without a drop: a cancel, Mission Control.
    LetGo,
    /// `window` left Rift's control: a grip on it ends, and the model no longer sees it.
    Gone { window: K::Window },
    /// macOS replaced window `from` by `to` (same window, new identity): a grip moves with it,
    /// and so does where the model sees it, unless `to` is already seen.
    Rekeyed { from: K::Window, to: K::Window },
    /// The model takes `window` to be at `frame`: a window appeared there, an inventory or a
    /// notification Rift accepted found it there, an app answered with it, or Rift put it there
    /// outside the reconciler (an animation, a placement it prepares).
    Seen { window: K::Window, frame: K::Frame },
    /// An outside owner took `window` (a claim): a grip on it ends; Rift keeps the window.
    Ceded { window: K::Window },
    /// The model no longer sees `window`, though Rift keeps it (its accessibility identity is
    /// suspended during native fullscreen); it is seen again when it comes back.
    Unseen { window: K::Window },
    /// Layout wants `window` at `desired`.
    Want {
        now: K::Time,
        request: K::Request,
        window: K::Window,
        desired: K::Frame,
        want: Want,
    },
    /// A modifier drag moved the dragged window's interactive frame to `frame`.
    Follow {
        request: K::Request,
        window: K::Window,
        frame: K::Frame,
    },
    /// The app answered a write to `window`: it is at `frame`. `seen` is the request the answer
    /// names, if it names one.
    Answer {
        now: K::Time,
        request: K::Request,
        seen: Option<K::Request>,
        window: K::Window,
        frame: K::Frame,
    },
    /// `window` reported `frame` unprompted.
    Report { window: K::Window, frame: K::Frame },
    /// The deadline timer fired.
    Tick { now: K::Time, request: K::Request },
    /// Something outside the core (an animation, a topology change) took `window`'s frame, or
    /// the window is gone: nothing is outstanding.
    Forget { window: K::Window },
    /// May Rift take `initiative` now?
    Gate { initiative: Initiative },
}

/// A decision for the platform.
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(bound(
        serialize = "K::Window: serde::Serialize, K::Frame: serde::Serialize, K::Time: serde::Serialize, K::Request: serde::Serialize",
        deserialize = "K::Window: serde::Deserialize<'de>, K::Frame: serde::Deserialize<'de>, K::Time: serde::Deserialize<'de>, K::Request: serde::Deserialize<'de>"
    ))
)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Effect<K: Kinds> {
    /// Send this frame to the window's app under its request.
    Write(Write<K>),
    /// A write is with the app; the desire waits behind it.
    Queued { window: K::Window },
    /// The model takes `window` to be at `frame` from now on (its projection, `frame_monotonic`
    /// in rift's window store).
    Seen { window: K::Window, frame: K::Frame },
    /// What the app's answer was.
    Answered { window: K::Window, answer: Answer },
    /// The answer names a write Rift has since replaced: dropped, it says nothing about the
    /// write with the app now.
    Stale { window: K::Window },
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
    /// The hand is on `window` now (pressed): its record goes into the grip slot.
    Grip { window: K::Window },
    /// The press on `window` became a drag: the platform attaches the drag's session.
    DragBegan { window: K::Window },
    /// The hand came off `window`, for `why`: its record leaves the slot.
    Ungrip { window: K::Window, why: Ungrip },
    /// The grip moved with its window's new identity.
    Regrip { from: K::Window, to: K::Window },
}

impl<K: Kinds> Event<K> {
    /// The same event on another platform's types.
    pub fn translate<J: Kinds>(self, to: &impl Translate<K, J>) -> Event<J> {
        match self {
            Event::Press { window, button } => Event::Press {
                window: window.map(|window| to.window(window)),
                button,
            },
            Event::Drag { window } => Event::Drag { window: to.window(window) },
            Event::Release(release) => Event::Release(release),
            Event::LetGo => Event::LetGo,
            Event::Gone { window } => Event::Gone { window: to.window(window) },
            Event::Rekeyed { from, to: into } => Event::Rekeyed {
                from: to.window(from),
                to: to.window(into),
            },
            Event::Want {
                now,
                request,
                window,
                desired,
                want,
            } => Event::Want {
                now: to.time(now),
                request: to.request(request),
                window: to.window(window),
                desired: to.frame(desired),
                want,
            },
            Event::Follow { request, window, frame } => Event::Follow {
                request: to.request(request),
                window: to.window(window),
                frame: to.frame(frame),
            },
            Event::Answer {
                now,
                request,
                seen,
                window,
                frame,
            } => Event::Answer {
                now: to.time(now),
                request: to.request(request),
                seen: seen.map(|seen| to.request(seen)),
                window: to.window(window),
                frame: to.frame(frame),
            },
            Event::Report { window, frame } => Event::Report {
                window: to.window(window),
                frame: to.frame(frame),
            },
            Event::Seen { window, frame } => Event::Seen {
                window: to.window(window),
                frame: to.frame(frame),
            },
            Event::Unseen { window } => Event::Unseen { window: to.window(window) },
            Event::Ceded { window } => Event::Ceded { window: to.window(window) },
            Event::Tick { now, request } => Event::Tick {
                now: to.time(now),
                request: to.request(request),
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
            Effect::Seen { window, frame } => Effect::Seen {
                window: to.window(window),
                frame: to.frame(frame),
            },
            Effect::Answered { window, answer } => Effect::Answered {
                window: to.window(window),
                answer,
            },
            Effect::Stale { window } => Effect::Stale { window: to.window(window) },
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
            Effect::Grip { window } => Effect::Grip { window: to.window(window) },
            Effect::DragBegan { window } => Effect::DragBegan { window: to.window(window) },
            Effect::Ungrip { window, why } => Effect::Ungrip { window: to.window(window), why },
            Effect::Regrip { from, to: into } => Effect::Regrip {
                from: to.window(from),
                to: to.window(into),
            },
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
    grip: Grip<K::Window>,
    /// Where the model takes each window it knows to be: its last-seen frame.
    seen: HashMap<K::Window, K::Frame>,
    /// The deadline the platform's timer is armed for.
    armed: Option<K::Time>,
}

impl<K: Kinds> Default for RustCore<K> {
    fn default() -> Self {
        RustCore {
            reconciler: Reconciler::default(),
            grip: Grip::Idle,
            seen: HashMap::new(),
            armed: None,
        }
    }
}

impl<K: Kinds> RustCore<K> {
    /// Read access for Rust callers that have not moved behind the boundary yet (tests). Not
    /// part of the boundary: a foreign core has no such door.
    pub fn reconciler(&self) -> &Reconciler<K> { &self.reconciler }

    /// Read access to what the hand is doing, for the platform's check (in debug builds) that
    /// its projection of the grip agrees. Not part of the boundary.
    pub fn pointer(&self) -> Pointer<K::Window> { self.grip.pointer() }

    /// Where the model sees `window`, for the platform's check (in debug builds) that its
    /// projection agrees. Not part of the boundary.
    pub fn seen(&self, window: K::Window) -> Option<K::Frame> { self.seen.get(&window).copied() }

    /// The model takes `window` to be at `frame` from now on.
    fn see(&mut self, window: K::Window, frame: K::Frame) -> Effect<K> {
        self.seen.insert(window, frame);
        Effect::Seen { window, frame }
    }

    /// A write the reconciler released goes out, and the model takes the window to be at it.
    fn written(&mut self, write: Write<K>) -> [Effect<K>; 2] {
        let seen = self.see(write.window(), write.frame());
        [Effect::Write(write), seen]
    }

    /// The hand comes off whatever it holds, for `why`.
    fn ungrip(&mut self, why: Ungrip) -> Option<Effect<K>> {
        let window = match self.grip {
            Grip::Idle => return None,
            Grip::Pressed { window, .. } | Grip::Dragging { window, .. } => window,
        };
        self.grip = Grip::Idle;
        Some(Effect::Ungrip { window, why })
    }

    fn decide(&mut self, event: Event<K>) -> Vec<Effect<K>> {
        let pointer = self.grip.pointer();
        match event {
            Event::Press { window, button } => {
                let let_go = self.ungrip(Ungrip::LetGo);
                let pressed = window.map(|window| {
                    self.grip = Grip::Pressed { window, button };
                    Effect::Grip { window }
                });
                let_go.into_iter().chain(pressed).collect()
            }
            Event::Drag { window } => match self.grip {
                Grip::Pressed { window: pressed, button } if pressed == window => {
                    self.grip = Grip::Dragging { window, button };
                    vec![Effect::DragBegan { window }]
                }
                Grip::Pressed { .. } | Grip::Dragging { .. } | Grip::Idle => vec![],
            },
            Event::Release(release) => {
                let ends = match (release, self.grip) {
                    (Release::Missed, Grip::Pressed { .. } | Grip::Dragging { .. }) => true,
                    (
                        Release::Button(released),
                        Grip::Pressed { button, .. } | Grip::Dragging { button, .. },
                    ) => released == button,
                    (Release::Missed | Release::Button(_), Grip::Idle) => false,
                };
                if ends {
                    self.ungrip(Ungrip::Released).into_iter().collect()
                } else {
                    vec![]
                }
            }
            Event::LetGo => self.ungrip(Ungrip::LetGo).into_iter().collect(),
            Event::Gone { window } => {
                self.seen.remove(&window);
                self.ungrip_window(window)
            }
            Event::Ceded { window } => self.ungrip_window(window),
            Event::Seen { window, frame } => vec![self.see(window, frame)],
            Event::Unseen { window } => {
                self.seen.remove(&window);
                vec![]
            }
            Event::Rekeyed { from, to } => {
                // The new identity inherits where the old one was seen unless it is seen itself
                // (the platform moves the window's state the same way).
                if let Some(frame) = self.seen.remove(&from) {
                    self.seen.entry(to).or_insert(frame);
                }
                self.rekey_grip(from, to)
            }
            Event::Want {
                now,
                request,
                window,
                desired,
                want,
            } => {
                // A window the model does not know is not written.
                let Some(&known) = self.seen.get(&window) else {
                    return vec![];
                };
                let owner = pointer.owner(window);
                match self.reconciler.want(now, request, window, desired, known, want, owner) {
                    Verdict::Send(write) => self.written(write).into(),
                    Verdict::Queued => vec![Effect::Queued { window }, self.see(window, desired)],
                    Verdict::Nothing => vec![],
                }
            }
            Event::Follow { request, window, frame } => {
                vec![Effect::Write(
                    self.reconciler.follow(request, window, frame),
                )]
            }
            Event::Answer {
                now,
                request,
                seen,
                window,
                frame,
            } => {
                let owner = pointer.owner(window);
                match self.reconciler.answer(now, request, seen, window, frame, owner) {
                    Reply::Answered(answer, queued) => {
                        let mut effects = vec![Effect::Answered { window, answer }];
                        effects.extend(queued.into_iter().flat_map(|write| self.written(write)));
                        effects
                    }
                    Reply::Late => vec![Effect::Late { window }],
                    Reply::Stale => vec![Effect::Stale { window }],
                }
            }
            Event::Report { window, frame } => {
                let Some(&known) = self.seen.get(&window) else {
                    return vec![];
                };
                let holder = pointer.holder(window);
                let observation = self.reconciler.observe(window, frame, known, holder);
                vec![Effect::Observed { window, observation }]
            }
            Event::Tick { now, request } => {
                let seen = &self.seen;
                let lookup = |window: K::Window| Some((*seen.get(&window)?, pointer.owner(window)));
                let released = self.reconciler.expire(now, request, lookup);
                released.into_iter().flat_map(|write| self.written(write)).collect()
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

    /// A grip on `window` ends: it left Rift's hands.
    fn ungrip_window(&mut self, window: K::Window) -> Vec<Effect<K>> {
        match self.grip {
            Grip::Pressed { window: held, .. } | Grip::Dragging { window: held, .. }
                if held == window =>
            {
                self.ungrip(Ungrip::Gone).into_iter().collect()
            }
            Grip::Pressed { .. } | Grip::Dragging { .. } | Grip::Idle => vec![],
        }
    }

    fn rekey_grip(&mut self, from: K::Window, to: K::Window) -> Vec<Effect<K>> {
        match self.grip {
            Grip::Pressed { window, button } if window == from => {
                self.grip = Grip::Pressed { window: to, button };
                vec![Effect::Regrip { from, to }]
            }
            Grip::Dragging { window, button } if window == from => {
                self.grip = Grip::Dragging { window: to, button };
                vec![Effect::Regrip { from, to }]
            }
            Grip::Pressed { .. } | Grip::Dragging { .. } | Grip::Idle => vec![],
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
    use crate::plain::{Nanos, Plain, Rect, RequestId, WindowId};
    use crate::pointer::Ungrip;
    use crate::reconciler::WRITE_DEADLINE;

    fn at(x: f64) -> Rect { Rect { x, y: 0., w: 400., h: 300. } }

    const W: WindowId = WindowId { pid: 7, idx: 1 };

    #[test]
    fn a_write_arms_the_timer_and_its_answer_disarms_it() {
        let mut core = RustCore::<Plain>::default();
        core.step(Event::Seen { window: W, frame: at(0.) });
        let effects = core.step(Event::Want {
            now: Nanos(0),
            request: RequestId(1),
            window: W,
            desired: at(10.),
            want: Want::Arrange,
        });
        let [
            Effect::Write(write),
            Effect::Seen { .. },
            Effect::Timer(timer),
        ] = effects.as_slice()
        else {
            panic!("expected a write, the model's new frame and a timer, got {effects:?}");
        };
        assert_eq!((write.window(), write.frame()), (W, at(10.)));
        assert_eq!(*timer, Some(Nanos(0) + WRITE_DEADLINE));
        let effects = core.step(Event::Answer {
            now: Nanos(5),
            request: RequestId(2),
            seen: Some(RequestId(1)),
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
    fn the_model_sees_what_the_core_writes_and_forgets_a_window_that_goes() {
        let mut core = RustCore::<Plain>::default();
        let want = |x| Event::Want {
            now: Nanos(0),
            request: RequestId(1),
            window: W,
            desired: at(x),
            want: Want::Arrange,
        };
        assert_eq!(core.step(want(10.)), vec![], "a window the model does not know");
        core.step(Event::Seen { window: W, frame: at(0.) });
        core.step(want(10.));
        assert_eq!(core.seen(W), Some(at(10.)));
        let renamed = WindowId { pid: 7, idx: 9 };
        core.step(Event::Rekeyed { from: W, to: renamed });
        assert_eq!((core.seen(W), core.seen(renamed)), (None, Some(at(10.))));
        core.step(Event::Gone { window: renamed });
        assert_eq!(core.seen(renamed), None);
    }

    #[test]
    fn a_grip_ends_only_on_its_own_button_and_a_drag_starts_only_from_its_press() {
        let other = WindowId { pid: 7, idx: 2 };
        let mut core = RustCore::<Plain>::default();
        assert_eq!(core.step(Event::Drag { window: W }), vec![], "no press, no drag");
        core.step(Event::Press {
            window: Some(W),
            button: Button::Left,
        });
        assert_eq!(core.step(Event::Drag { window: other }), vec![]);
        assert_eq!(core.step(Event::Drag { window: W }), vec![Effect::DragBegan {
            window: W
        }]);
        assert_eq!(core.step(Event::Release(Release::Button(Button::Right))), vec![]);
        assert_eq!(core.step(Event::Release(Release::Button(Button::Left))), vec![
            Effect::Ungrip {
                window: W,
                why: Ungrip::Released
            }
        ]);
        // One hand: a press elsewhere lets go of what it held.
        core.step(Event::Press {
            window: Some(W),
            button: Button::Left,
        });
        assert_eq!(
            core.step(Event::Press {
                window: Some(other),
                button: Button::Left
            }),
            vec![
                Effect::Ungrip { window: W, why: Ungrip::LetGo },
                Effect::Grip { window: other },
            ]
        );
        let renamed = WindowId { pid: 7, idx: 3 };
        assert_eq!(core.step(Event::Rekeyed { from: other, to: renamed }), vec![
            Effect::Regrip { from: other, to: renamed }
        ]);
        assert_eq!(core.step(Event::Gone { window: other }), vec![]);
        assert_eq!(core.step(Event::Gone { window: renamed }), vec![
            Effect::Ungrip {
                window: renamed,
                why: Ungrip::Gone
            }
        ]);
    }

    #[test]
    fn a_desire_for_a_pressed_window_is_discarded_and_steering_input_refused() {
        let mut core = RustCore::<Plain>::default();
        core.step(Event::Seen { window: W, frame: at(0.) });
        assert_eq!(
            core.step(Event::Press {
                window: Some(W),
                button: Button::Left
            }),
            vec![Effect::Grip { window: W }]
        );
        let effects = core.step(Event::Want {
            now: Nanos(0),
            request: RequestId(1),
            window: W,
            desired: at(10.),
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
