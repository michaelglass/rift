//! The boundary: one step function from an [`Event`] to the [`Effect`]s it decides.
//!
//! Events are facts the platform parsed (a report, an answer, the pointer's state, a desire
//! layout computed, a timer firing). Effects are data the platform carries out (a write, a
//! timer to arm) or records (what a report turned out to be). The model stays behind
//! [`Core`]: the platform never reads it, so whatever it needs comes out as an effect.
//!
//! Every event that may lead to a write carries the `request` the platform will send its
//! writes under; each [`Write`] says which, and the app's answer names it back.

use crate::Kinds;
use crate::pointer::{Button, Grip, Phase, Pointer, Release, Ungrip};
use crate::reconciler::{Answer, Note, Planned, Reconciler, Reply, Stamp, Verdict, Want, Write};

/// One window's place in an arrange pass, and the request its write would go out under (one
/// per batch the platform sends: an app, and whether its writes are held to scrolling bounds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Desire<K: Kinds> {
    pub window: K::Window,
    pub frame: K::Frame,
    pub request: K::Request,
}

/// What one batch of an arrange pass sends: the windows that only move, and those that also
/// resize, all under `request`. Only the core makes one, starting with its first write, so a
/// plan is never empty.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan<K: Kinds> {
    pub(crate) request: K::Request,
    pub(crate) positions: Vec<Write<K>>,
    pub(crate) frames: Vec<Write<K>>,
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
    /// An outside owner took `window` (a claim): a grip on it ends, and nothing more is asked of
    /// its app; a write already sent is still waited for (only what was queued behind it is
    /// dropped), anything else is forgotten (as [`Event::Forget`]). Rift keeps the window.
    Ceded { window: K::Window },
    /// The model no longer sees `window`, though Rift keeps it (its accessibility identity is
    /// suspended during native fullscreen); it is seen again when it comes back.
    Unseen { window: K::Window },
    /// The displays changed: macOS may have resized these windows without telling anyone, so
    /// where their apps have them is unknown until they say.
    Distrust { windows: Vec<K::Window> },
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
    /// The platform stopped presenting `window` (its frames went out under `request`), at the
    /// last frame it published (`at`), where the model now takes it to be; frames of `request`
    /// the app had not applied are dropped.
    AnimationStopped {
        window: K::Window,
        request: K::Request,
        at: Option<K::Frame>,
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
    /// The platform sends `window` frames outside the core under `request` (a gesture's
    /// camera): reads stamped with it are Rift's own.
    SentOutside {
        window: K::Window,
        request: K::Request,
    },
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
    /// refused frame), or nothing: the platform's pending store is its projection.
    Pending {
        window: K::Window,
        target: Option<K::Frame>,
    },
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
    /// `changed`: the windows whose model frame changed during a drag that ended, now
    /// unknown, to re-derive.
    Ungrip {
        window: K::Window,
        why: Ungrip,
        changed: Vec<K::Window>,
    },
    /// The grip moved with its window's new identity.
    Regrip { from: K::Window, to: K::Window },
}

/// The core written in Rust.
#[derive(Debug)]
pub struct RustCore<K: Kinds> {
    reconciler: Reconciler<K>,
    grip: Grip<K::Window>,
    /// The deadline the platform's timer is armed for.
    armed: Option<K::Time>,
}

impl<K: Kinds> Default for RustCore<K> {
    fn default() -> Self {
        RustCore {
            reconciler: Reconciler::default(),
            grip: Grip::Idle,
            armed: None,
        }
    }
}

impl<K: Kinds> Plan<K> {
    pub fn request(&self) -> K::Request { self.request }

    /// The writes that only move their windows.
    pub fn positions(&self) -> &[Write<K>] { &self.positions }

    /// The writes that also resize.
    pub fn frames(&self) -> &[Write<K>] { &self.frames }

    /// The request, the moves and the resizes, taken apart.
    pub fn into_parts(self) -> (K::Request, Vec<Write<K>>, Vec<Write<K>>) {
        (self.request, self.positions, self.frames)
    }

    /// A plan begun by its first write (a plan is never empty).
    fn starting(write: Write<K>, moves_only: bool) -> Self {
        let mut plan = Plan {
            request: write.request(),
            positions: Vec::new(),
            frames: Vec::new(),
        };
        plan.add(write, moves_only);
        plan
    }

    fn add(&mut self, write: Write<K>, moves_only: bool) {
        if moves_only {
            self.positions.push(write)
        } else {
            self.frames.push(write)
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
    pub fn seen(&self, window: K::Window) -> Option<K::Frame> { self.reconciler.seen(window) }

    /// The model takes `window` to be at `frame` from now on. A window it did not know has
    /// just appeared, and its app has it there.
    fn see(&mut self, window: K::Window, frame: K::Frame) -> Effect<K> {
        self.reconciler.see(window, frame);
        Effect::Seen { window, frame }
    }

    /// A write the reconciler released goes out, and the model takes the window to be at it.
    fn written(&mut self, write: Write<K>) -> [Effect<K>; 2] {
        let seen = self.see(write.window(), write.frame());
        [Effect::Write(write), seen]
    }

    /// A desire waits behind the write in flight; the model takes the window to be there.
    fn queued(&mut self, window: K::Window, desired: K::Frame) -> [Effect<K>; 2] {
        [Effect::Queued { window }, self.see(window, desired)]
    }

    /// The hand comes off whatever it holds, for `why`. A window the user dragged is where
    /// they left it, which no report during the drag acknowledged: unknown; so is any window
    /// whose model frame changed during the drag (its desires were the user's), and those are
    /// handed back for re-derivation. (A press that never moved the window leaves it as it was.)
    fn ungrip(&mut self, why: Ungrip) -> Option<Effect<K>> {
        let (window, changed) = match std::mem::replace(&mut self.grip, Grip::Idle) {
            Grip::Idle => return None,
            Grip::Held {
                window, phase: Phase::Pressed, ..
            } => (window, vec![]),
            Grip::Held {
                window,
                phase: Phase::Dragging { changed },
                ..
            } => {
                for &window in std::iter::once(&window).chain(&changed) {
                    self.reconciler.distrust(window);
                }
                (window, changed)
            }
        };
        Some(Effect::Ungrip { window, why, changed })
    }

    fn decide(&mut self, event: Event<K>) -> Vec<Effect<K>> {
        let pointer = self.grip.pointer();
        match event {
            Event::Press { window, button } => {
                let let_go = self.ungrip(Ungrip::LetGo);
                let pressed = window.map(|window| {
                    self.grip = Grip::Held {
                        window,
                        button,
                        phase: Phase::Pressed,
                    };
                    Effect::Grip { window }
                });
                let_go.into_iter().chain(pressed).collect()
            }
            Event::Drag { window } => match &mut self.grip {
                Grip::Held {
                    window: pressed,
                    phase: phase @ Phase::Pressed,
                    ..
                } if *pressed == window => {
                    *phase = Phase::Dragging { changed: vec![] };
                    vec![Effect::DragBegan { window }]
                }
                Grip::Held { .. } | Grip::Idle => vec![],
            },
            Event::Release(release) => {
                let ends = match (release, &self.grip) {
                    (Release::Missed, Grip::Held { .. }) => true,
                    (Release::Button(released), Grip::Held { button, .. }) => released == *button,
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
            Event::Ceded { window } => {
                let mut effects = self.ungrip_window(window);
                effects.extend(self.cede(window));
                effects
            }
            Event::Seen { window, frame } => vec![self.see(window, frame)],
            Event::Unseen { window } => {
                self.reconciler.remove(window);
                vec![]
            }
            Event::Rekeyed { from, to } => {
                // The new identity inherits where the old one was seen unless it is seen itself
                // (the platform moves the window's state the same way).
                self.reconciler.rekey(from, to);
                self.rekey_grip(from, to)
            }
            Event::Arrange { now, want, desires } => self.arrange(now, want, desires),
            Event::Animate { now, desires } => {
                let mut effects = Vec::new();
                for Desire { window, frame: to, request } in desires {
                    let Some(from) = self.reconciler.seen(window) else {
                        continue;
                    };
                    let owner = pointer.owner(window);
                    match self.reconciler.animate(now, request, window, to, owner) {
                        Planned::Animate => {
                            effects.push(Effect::Animate { window, from, to, request });
                            effects.push(self.see(window, to));
                        }
                        Planned::Verdict(Verdict::Send(write)) => {
                            effects.extend(self.written(write))
                        }
                        Planned::Verdict(Verdict::Queued) => {
                            effects.extend(self.queued(window, to))
                        }
                        Planned::Verdict(Verdict::Nothing) => {}
                    }
                }
                effects
            }
            Event::AnimationEnded { now, request } => {
                self.reconciler.animation_ended(now, Stamp::new(request));
                vec![]
            }
            Event::AnimationStopped { window, request, at } => {
                let seen = at.map(|frame| self.see(window, frame));
                self.reconciler.animation_stopped(window, Stamp::new(request));
                seen.into_iter().collect()
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
                let (owner, holder) = (pointer.owner(window), pointer.holder(window));
                let (reply, released) = self.reconciler.observe(
                    now,
                    request,
                    window,
                    frame,
                    as_of.map(Stamp::new),
                    owner,
                    holder,
                );
                // News of another window during a drag (someone else moved it): its desires are
                // the user's until the drag ends, so it is re-derived then.
                if reply == Reply::Foreign
                    && let Grip::Held {
                        window: held,
                        phase: Phase::Dragging { changed },
                        ..
                    } = &mut self.grip
                    && *held != window
                    && !changed.contains(&window)
                {
                    changed.push(window);
                }
                let mut effects = vec![Effect::Replied { window, reply }];
                // The app answered with a frame of its own: the model takes the window to be
                // where the app has it (neighbours and the projection build on it). The desire
                // stays refused, so it is not asked for again.
                if reply == Reply::Answered(Answer::Refused) {
                    effects.push(self.see(window, frame));
                }
                effects.extend(released.into_iter().flat_map(|write| self.written(write)));
                effects
            }
            Event::Tick { now, request } => {
                let released = self.reconciler.expire(now, request, pointer);
                released.into_iter().flat_map(|write| self.written(write)).collect()
            }
            Event::Forget { window } => self.forget(window).into_iter().collect(),
            Event::SentOutside { window, request } => {
                self.reconciler.sent_outside(window, request);
                vec![]
            }
        }
    }

    /// Nothing is outstanding for `window`: the model takes it to be where the app
    /// acknowledged it, if that is known.
    /// An outside owner took `window`: nothing more is asked of its app, but a write already
    /// sent is still Rift's to hear back about, so it keeps its request and deadline (only what
    /// was queued behind it is dropped). Anything else is forgotten.
    fn cede(&mut self, window: K::Window) -> Option<Effect<K>> {
        if self.reconciler.drop_queued(window) {
            return None;
        }
        self.forget(window)
    }

    fn forget(&mut self, window: K::Window) -> Option<Effect<K>> {
        let acked = self.reconciler.forget(window)?;
        let moved = self.reconciler.seen(window).is_some_and(|seen| seen != acked);
        moved.then(|| self.see(window, acked))
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
            let Some(known) = self.reconciler.seen(window) else {
                continue;
            };
            let owner = pointer.owner(window);
            match self.reconciler.want(now, request, window, desired, owner) {
                Verdict::Send(write) => {
                    let moves_only = match want {
                        Want::ArrangePositions => K::same_size(known, desired),
                        Want::Arrange => false,
                    };
                    effects.push(self.see(window, desired));
                    // The write joins its request's plan, or starts one.
                    match plans.iter_mut().find(|plan| plan.request == request) {
                        Some(plan) => plan.add(write, moves_only),
                        None => plans.push(Plan::starting(write, moves_only)),
                    }
                }
                Verdict::Queued => effects.extend(self.queued(window, desired)),
                Verdict::Nothing => {}
            }
        }
        effects.extend(plans.into_iter().map(Effect::Plan));
        effects
    }

    /// A grip on `window` ends: it left Rift's hands.
    fn ungrip_window(&mut self, window: K::Window) -> Vec<Effect<K>> {
        match self.grip {
            Grip::Held { window: held, .. } if held == window => {
                self.ungrip(Ungrip::Gone).into_iter().collect()
            }
            Grip::Held { .. } | Grip::Idle => vec![],
        }
    }

    fn rekey_grip(&mut self, from: K::Window, to: K::Window) -> Vec<Effect<K>> {
        match &mut self.grip {
            Grip::Held { window, .. } if *window == from => {
                *window = to;
                vec![Effect::Regrip { from, to }]
            }
            Grip::Held { .. } | Grip::Idle => vec![],
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
                .drain_notes()
                .map(|(window, note)| Effect::Note { window, note }),
        );
        effects.extend(
            self.reconciler
                .drain_pending()
                .map(|(window, target)| Effect::Pending { window, target }),
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

    /// Layout's pass wanting only `W` at `desired`, its write under request 1.
    fn want(desired: Rect) -> Event<Plain> {
        Event::Arrange {
            now: Nanos(0),
            want: Want::Arrange,
            desires: vec![Desire {
                window: W,
                frame: desired,
                request: RequestId(1),
            }],
        }
    }

    #[test]
    fn a_refused_write_leaves_the_model_where_the_app_put_the_window() {
        // The app answers Rift's write with a frame of its own (it would not take that size):
        // the model takes the window to be where the app has it, not where Rift asked, so
        // neighbours and the projection build on the real frame. The desire stays refused:
        // asking for it again writes nothing.
        let mut core = RustCore::<Plain>::default();
        core.step(Event::Seen { window: W, frame: at(0.) });
        let target = Rect {
            x: 20.,
            y: 20.,
            w: 620.,
            h: 500.,
        };
        let got = Rect { w: 700., ..target };
        core.step(Event::Arrange {
            now: Nanos(1),
            want: Want::Arrange,
            desires: vec![Desire {
                window: W,
                frame: target,
                request: RequestId(1),
            }],
        });
        let effects = core.step(Event::Observe {
            now: Nanos(2),
            request: RequestId(2),
            window: W,
            frame: got,
            as_of: Some(RequestId(1)),
        });
        assert!(
            effects.contains(&Effect::Replied {
                window: W,
                reply: crate::Reply::Answered(crate::Answer::Refused)
            }),
            "{effects:?}"
        );
        assert!(
            effects.contains(&Effect::Seen { window: W, frame: got }),
            "{effects:?}"
        );
        assert_eq!(core.seen(W), Some(got));
        let again = core.step(Event::Arrange {
            now: Nanos(3),
            want: Want::Arrange,
            desires: vec![Desire {
                window: W,
                frame: target,
                request: RequestId(3),
            }],
        });
        assert!(
            !again.iter().any(|effect| matches!(effect, Effect::Plan(_) | Effect::Write(_))),
            "the refused desire was written again: {again:?}"
        );
    }

    #[test]
    fn a_write_arms_the_timer_and_its_answer_disarms_it() {
        let mut core = RustCore::<Plain>::default();
        core.step(Event::Seen { window: W, frame: at(0.) });
        let effects = core.step(want(at(10.)));
        let [
            Effect::Seen { .. },
            Effect::Plan(plan),
            Effect::Pending { target, .. },
            Effect::Timer(timer),
        ] = effects.as_slice()
        else {
            panic!(
                "expected the model's new frame, a plan, the pending target and a timer, got {effects:?}"
            );
        };
        assert_eq!(*target, Some(at(10.)));
        let [write] = plan.frames.as_slice() else {
            panic!("one write, got {plan:?}")
        };
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
            Effect::Pending { window: W, target: None },
            Effect::Timer(None),
        ]);
    }

    #[test]
    fn a_window_a_read_moved_during_a_drag_is_handed_back_unknown() {
        let mut core = RustCore::<Plain>::default();
        let other = WindowId { pid: 8, idx: 1 };
        core.step(Event::Seen { window: W, frame: at(0.) });
        core.step(Event::Seen { window: other, frame: at(500.) });
        core.step(Event::Press {
            window: Some(W),
            button: Button::Left,
        });
        core.step(Event::Drag { window: W });
        // A read moves the other window's model frame; its desires are the user's now.
        core.step(Event::Observe {
            now: Nanos(0),
            request: RequestId(1),
            window: other,
            frame: at(520.),
            as_of: None,
        });
        let effects = core.step(Event::Release(Release::Button(Button::Left)));
        assert!(effects.contains(&Effect::Ungrip {
            window: W,
            why: Ungrip::Released,
            changed: vec![other]
        }));
        assert!(matches!(
            core.reconciler.sync(other),
            Some(crate::Sync::Unknown { .. })
        ));
    }

    #[test]
    fn the_model_sees_what_the_core_writes_and_forgets_a_window_that_goes() {
        let mut core = RustCore::<Plain>::default();
        assert_eq!(
            core.step(want(at(10.))),
            vec![],
            "a window the model does not know"
        );
        core.step(Event::Seen { window: W, frame: at(0.) });
        core.step(want(at(10.)));
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
                why: Ungrip::Released,
                changed: vec![]
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
                Effect::Ungrip {
                    window: W,
                    why: Ungrip::LetGo,
                    changed: vec![]
                },
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
                why: Ungrip::Gone,
                changed: vec![]
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
        let effects = core.step(want(at(10.)));
        assert_eq!(effects, vec![Effect::Note {
            window: W,
            note: Note::Discarded
        }]);
        let steering = crate::Initiative::UserInput(crate::pointer::InputClass::Steering);
        assert!(!crate::may(steering, core.pointer()));
    }
}
