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
use crate::pointer::{Button, Grip, Pointer, Release, Ungrip};
use crate::reconciler::{Note, Planned, Reconciler, Reply, Verdict, Want, Write};

/// One window's place in an arrange pass, and the request its write would go out under (one
/// per batch the platform sends: an app, and whether its writes are held to scrolling bounds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Desire<K: Kinds> {
    pub window: K::Window,
    pub frame: K::Frame,
    pub request: K::Request,
}

/// What one batch of an arrange pass sends: the windows that only move, and those that also
/// resize, all under `request`.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan<K: Kinds> {
    pub request: K::Request,
    pub positions: Vec<Write<K>>,
    pub frames: Vec<Write<K>>,
}

/// A fact for the core.
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
    /// The displays changed: macOS may have resized these windows without telling anyone, so
    /// where their apps have them is unknown until they say.
    Distrust { windows: Vec<K::Window> },
    /// Layout wants `window` at `desired`.
    Want {
        now: K::Time,
        request: K::Request,
        window: K::Window,
        desired: K::Frame,
        want: Want,
    },
    /// Layout's pass over one space: where it wants each window, in layout order.
    /// `ArrangePositions` sends a window that keeps its size as a move only.
    Arrange {
        now: K::Time,
        want: Want,
        desires: Vec<Desire<K>>,
    },
    /// An animated layout pass over the active workspace's windows (in layout order): which of
    /// them to animate, each from where the model has it to its desire. Each desire names the
    /// request the animation's frames go out under.
    Animate {
        now: K::Time,
        desires: Vec<Desire<K>>,
    },
    /// The animation under `request` ended.
    AnimationEnded { now: K::Time, request: K::Request },
    /// The platform stopped presenting `window` (its frames went out under `request`), where
    /// the model now sees it; frames of `request` the app had not applied are dropped.
    AnimationStopped {
        window: K::Window,
        request: K::Request,
    },
    /// A modifier drag moved the dragged window's interactive frame to `frame`.
    Follow {
        request: K::Request,
        window: K::Window,
        frame: K::Frame,
    },
    /// `window` was read at `frame`. `as_of` is the newest request the app had applied to it
    /// when it read the frame, for a read Rift asked for (an answer, a re-read); a notification
    /// carries none. What waited behind an answered write goes out under `request`.
    Observe {
        now: K::Time,
        request: K::Request,
        window: K::Window,
        frame: K::Frame,
        as_of: Option<K::Request>,
    },
    /// The deadline timer fired.
    Tick { now: K::Time, request: K::Request },
    /// Something outside the core (an animation, a topology change) took `window`'s frame, or
    /// the window is gone: nothing is outstanding.
    Forget { window: K::Window },
}

/// A decision for the platform.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect<K: Kinds> {
    /// Send this frame to the window's app under its request.
    Write(Write<K>),
    /// Animate `window` from `from` to `to`, its frames under `request`.
    Animate {
        window: K::Window,
        from: K::Frame,
        to: K::Frame,
        request: K::Request,
    },
    /// Rift's pending frame for `window` is `target` now (a write in flight, an animation, a
    /// refused frame): the platform's pending store is its projection.
    Pending { window: K::Window, target: K::Frame },
    /// Nothing is pending for `window` any more.
    Unpending { window: K::Window },
    /// Send one batch of an arrange pass.
    Plan(Plan<K>),
    /// A write is with the app; the desire waits behind it.
    Queued { window: K::Window },
    /// The model takes `window` to be at `frame` from now on (its projection, `frame_monotonic`
    /// in rift's window store).
    Seen { window: K::Window, frame: K::Frame },
    /// What a read of `window` meant.
    Replied { window: K::Window, reply: Reply },
    /// Something decided along the way, for the log.
    Note { window: K::Window, note: Note },
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

/// The core written in Rust.
#[derive(Debug)]
pub struct RustCore<K: Kinds> {
    reconciler: Reconciler<K>,
    grip: Grip<K::Window>,
    /// The pending frames as last told to the platform (`Effect::Pending`).
    projected: HashMap<K::Window, K::Frame>,
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
            projected: HashMap::new(),
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

    /// The model takes `window` to be at `frame` from now on. A window it did not know has
    /// just appeared, and its app has it there.
    fn see(&mut self, window: K::Window, frame: K::Frame) -> Effect<K> {
        if self.seen.insert(window, frame).is_none() {
            self.reconciler.appear(window, frame);
        }
        Effect::Seen { window, frame }
    }

    /// A write the reconciler released goes out, and the model takes the window to be at it.
    fn written(&mut self, write: Write<K>) -> [Effect<K>; 2] {
        let seen = self.see(write.window(), write.frame());
        [Effect::Write(write), seen]
    }

    /// The hand comes off whatever it holds, for `why`. A window the user dragged is where
    /// they left it, which no report during the drag acknowledged: unknown. (A press that never
    /// moved the window leaves it as it was.)
    fn ungrip(&mut self, why: Ungrip) -> Option<Effect<K>> {
        let window = match self.grip {
            Grip::Idle => return None,
            Grip::Pressed { window, .. } => window,
            Grip::Dragging { window, .. } => {
                self.reconciler.distrust(window);
                window
            }
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
                let effects = self.ungrip_window(window);
                self.reconciler.remove(window);
                effects
            }
            Event::Distrust { windows } => {
                for window in windows {
                    self.reconciler.distrust(window);
                }
                vec![]
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
                if let Some(frame) = self.seen.remove(&from)
                    && !self.seen.contains_key(&to)
                {
                    self.seen.insert(to, frame);
                    self.reconciler.appear(to, frame);
                }
                self.reconciler.remove(from);
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
            Event::Arrange { now, want, desires } => self.arrange(now, want, desires),
            Event::Animate { now, desires } => {
                let mut effects = Vec::new();
                for Desire { window, frame: to, request } in desires {
                    let Some(&from) = self.seen.get(&window) else { continue };
                    let owner = pointer.owner(window);
                    match self.reconciler.animate(now, request, window, to, from, owner) {
                        Planned::Animate => {
                            effects.push(Effect::Animate { window, from, to, request });
                            effects.push(self.see(window, to));
                        }
                        Planned::Write(write) => effects.extend(self.written(write)),
                        Planned::Queued => {
                            effects.push(Effect::Queued { window });
                            effects.push(self.see(window, to));
                        }
                        Planned::Nothing => {}
                    }
                }
                effects
            }
            Event::AnimationEnded { now, request } => {
                self.reconciler.animation_ended(now, request);
                vec![]
            }
            Event::AnimationStopped { window, request } => {
                if let Some(&known) = self.seen.get(&window) {
                    self.reconciler.animation_stopped(window, request, known);
                }
                vec![]
            }
            Event::Follow { request, window, frame } => {
                vec![Effect::Write(
                    self.reconciler.follow(request, window, frame),
                )]
            }
            Event::Observe {
                now,
                request,
                window,
                frame,
                as_of,
            } => {
                let Some(&known) = self.seen.get(&window) else {
                    return vec![];
                };
                let (owner, holder) = (pointer.owner(window), pointer.holder(window));
                let (reply, released) = self
                    .reconciler
                    .observe(now, request, window, frame, known, as_of, owner, holder);
                let mut effects = vec![Effect::Replied { window, reply }];
                effects.extend(released.into_iter().flat_map(|write| self.written(write)));
                effects
            }
            Event::Tick { now, request } => {
                let seen = &self.seen;
                let lookup =
                    |window: K::Window| seen.contains_key(&window).then(|| pointer.owner(window));
                let released = self.reconciler.expire(now, request, lookup);
                released.into_iter().flat_map(|write| self.written(write)).collect()
            }
            Event::Forget { window } => match self.reconciler.forget(window) {
                // The model takes the window to be where the app acknowledged it.
                Some(acked) if self.seen.get(&window).is_some_and(|&seen| seen != acked) => {
                    vec![self.see(window, acked)]
                }
                Some(_) | None => vec![],
            },
        }
    }

    /// One arrange pass: each desire asked of the reconciler in layout order; the writes it
    /// decides, gathered per request into plans (in the order their batches first wrote).
    fn arrange(&mut self, now: K::Time, want: Want, desires: Vec<Desire<K>>) -> Vec<Effect<K>> {
        let pointer = self.grip.pointer();
        let mut effects = Vec::new();
        let mut plans: Vec<Plan<K>> = Vec::new();
        for Desire {
            window,
            frame: desired,
            request,
        } in desires
        {
            // A window the model does not know is not written.
            let Some(&known) = self.seen.get(&window) else { continue };
            let owner = pointer.owner(window);
            match self.reconciler.want(now, request, window, desired, known, want, owner) {
                Verdict::Send(write) => {
                    let moves_only = match want {
                        Want::ArrangePositions => K::same_size(known, desired),
                        Want::Arrange => false,
                    };
                    effects.push(self.see(window, desired));
                    let index = match plans.iter().position(|plan| plan.request == request) {
                        Some(index) => index,
                        None => {
                            plans.push(Plan {
                                request,
                                positions: Vec::new(),
                                frames: Vec::new(),
                            });
                            plans.len() - 1
                        }
                    };
                    let plan = &mut plans[index];
                    if moves_only {
                        plan.positions.push(write)
                    } else {
                        plan.frames.push(write)
                    }
                }
                Verdict::Queued => {
                    effects.push(Effect::Queued { window });
                    effects.push(self.see(window, desired));
                }
                Verdict::Nothing => {}
            }
        }
        effects.extend(plans.into_iter().map(Effect::Plan));
        effects
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

impl<K: Kinds> RustCore<K> {
    /// Decisions first, then the notes taken while deciding, then the pending frames that
    /// changed, then the timer if its deadline moved.
    pub fn step(&mut self, event: Event<K>) -> Vec<Effect<K>> {
        let mut effects = self.decide(event);
        effects.extend(
            self.reconciler
                .notes()
                .into_iter()
                .map(|(window, note)| Effect::Note { window, note }),
        );
        for window in self.reconciler.touched() {
            match (
                self.reconciler.pending(window),
                self.projected.get(&window).copied(),
            ) {
                (Some(target), Some(told)) if K::same_as(target, told) => {}
                (Some(target), Some(_) | None) => {
                    self.projected.insert(window, target);
                    effects.push(Effect::Pending { window, target });
                }
                (None, Some(_)) => {
                    self.projected.remove(&window);
                    effects.push(Effect::Unpending { window });
                }
                (None, None) => {}
            }
        }
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
            Effect::Pending { target, .. },
            Effect::Timer(timer),
        ] = effects.as_slice()
        else {
            panic!(
                "expected a write, the model's new frame, the pending target and a timer, got {effects:?}"
            );
        };
        assert_eq!(*target, at(10.));
        assert_eq!((write.window(), write.frame()), (W, at(10.)));
        assert_eq!(*timer, Some(Nanos(0) + WRITE_DEADLINE));
        let effects = core.step(Event::Observe {
            now: Nanos(5),
            request: RequestId(2),
            window: W,
            frame: at(10.),
            as_of: Some(RequestId(1)),
        });
        assert_eq!(effects, vec![
            Effect::Replied {
                window: W,
                reply: crate::Reply::Answered(crate::Answer::Acked)
            },
            Effect::Unpending { window: W },
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
        let steering = crate::Initiative::UserInput(crate::pointer::InputClass::Steering);
        assert!(!crate::may(steering, core.pointer()));
    }
}
