//! Desired frames, and the one place that decides to write them.
//!
//! Layout says where each window should be. The [`Reconciler`] compares that with what each
//! window's app has acknowledged and mints a [`Write`] only where they differ. A `Write` cannot
//! be built anywhere else, and the platform's senders take nothing but a `Write`, so a frame
//! reaches an app only because the reconciler decided it should.
//!
//! Time is never read here: every call that needs it is handed `now`. Nor are request ids
//! minted here: every call that may write is handed the `request` its writes go out under.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use crate::Kinds;
use crate::pointer::{Holder, Owner};

/// How far an app's answer may be from the frame Rift asked for and still be that frame
/// (yabai's `AX_DIFF`).
pub const ACK_TOLERANCE: f64 = 1.5;

/// How long an app has to answer a write before Rift stops waiting for it.
pub const WRITE_DEADLINE: Duration = Duration::from_millis(300);

/// A frame the reconciler decided to send, and the request it goes out under. Only a core can
/// make one.
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(bound(
        serialize = "K::Window: serde::Serialize, K::Frame: serde::Serialize, K::Time: serde::Serialize, K::Request: serde::Serialize",
        deserialize = "K::Window: serde::Deserialize<'de>, K::Frame: serde::Deserialize<'de>, K::Time: serde::Deserialize<'de>, K::Request: serde::Deserialize<'de>"
    ))
)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Write<K: Kinds> {
    window: K::Window,
    frame: K::Frame,
    request: K::Request,
}

impl<K: Kinds> Write<K> {
    pub fn window(&self) -> K::Window { self.window }

    pub fn frame(&self) -> K::Frame { self.frame }

    /// The id to send it under; the app's answer names it.
    pub fn request(&self) -> K::Request { self.request }

    /// A write decided by a core implemented outside this crate (the Roc core's adapter). The
    /// platform's senders still take only what came out of a [`Core`](crate::Core).
    pub fn from_foreign_core(window: K::Window, frame: K::Frame, request: K::Request) -> Self {
        Write { window, frame, request }
    }

    /// The same write on another platform's types (a recorded trace replayed on plain data).
    pub fn translate<J: Kinds>(self, to: &impl Translate<K, J>) -> Write<J> {
        Write {
            window: to.window(self.window),
            frame: to.frame(self.frame),
            request: to.request(self.request),
        }
    }
}

/// A way from one platform's types to another's.
pub trait Translate<K: Kinds, J: Kinds> {
    fn window(&self, window: K::Window) -> J::Window;
    fn frame(&self, frame: K::Frame) -> J::Frame;
    fn time(&self, time: K::Time) -> J::Time;
    fn request(&self, request: K::Request) -> J::Request;
}

/// Where one window stands between what Rift asked of its app and what the app answered.
/// Every window the model knows has one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sync<K: Kinds> {
    /// The app has the window at `acked`: it answered so, or reported it there. A desire equal
    /// to `acked` is not written.
    Settled { acked: K::Frame },
    /// Rift does not know where the app has the window: the user just dragged it (reports
    /// during a drag acknowledge nothing Rift asked for), or the displays changed (macOS
    /// resizes windows silently). The next desire is written whatever it is; a report or an
    /// answer settles it.
    Unknown,
    /// One write is with the app, sent under `request`, from `before`. A newer desire waits in
    /// `queued` (one, the latest) until the app answers or `deadline` passes.
    InFlight {
        target: K::Frame,
        deadline: K::Time,
        queued: Option<K::Frame>,
        request: K::Request,
        before: Before<K>,
    },
    /// The platform is animating the window to `target`, its frames sent under `request`. Until
    /// an answer lands on the target (it is then settled) or the animation ends (if it never
    /// landed, where the app has it is unknown), a desire treats the window as at `target`.
    Animating {
        target: K::Frame,
        request: K::Request,
    },
    /// The app answered `wanted` with `got`, or did not answer in time (`got` is then what it
    /// had acknowledged before). Rift does not ask for `wanted` again while the window stays at
    /// `got`.
    Refused { wanted: K::Frame, got: K::Frame },
}

/// What the app had acknowledged when a write was sent: where a write it never answers leaves
/// the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Before<K: Kinds> {
    Acked(K::Frame),
    Unknown,
}

/// How a layout pass wants its frames.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Want {
    /// Layout: nothing to send when the app already has the window there.
    Arrange,
    /// Layout during a workspace switch: as [`Want::Arrange`], and a window that keeps its size
    /// is only moved (a position-only request).
    ArrangePositions,
}

/// What the reconciler made of a desire.
#[derive(Debug, PartialEq)]
pub enum Verdict<K: Kinds> {
    /// Send this now.
    Send(Write<K>),
    /// A write is with the app; this desire goes after it.
    Queued,
    /// The window is there, on its way there, refused to go there, or is the user's.
    Nothing,
}

/// An unprompted report of a window's frame, classified against what Rift asked for.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Observation {
    /// The frame Rift asked for, or the one the model already had: no news.
    Ack,
    /// A write is with the app and nobody holds the window: a step on the way there (size
    /// lands before position). The app's answer settles it.
    InTransit,
    /// Someone else put the window here: the user if their hand is on it, else its app.
    Foreign,
}

/// The app's answer to a write, classified against the write it answers.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Answer {
    Acked,
    /// The app put the window somewhere else ([`Sync::Refused`] says where).
    Refused,
    /// Nothing tracked was waiting for it: the answer to one sent outside the reconciler (an
    /// animation frame, a drag's interactive frame), or to a write the user's hand overtook.
    /// It still says where the window is.
    Unasked,
}

/// What an app's reply to a write turned out to be.
#[derive(Debug, PartialEq)]
pub enum Reply<K: Kinds> {
    /// The answer to the write in flight (or to none tracked), and the write that waited
    /// behind it.
    Answered(Answer, Option<Write<K>>),
    /// Rift had stopped waiting: not an answer but news of where the window is, to be
    /// [observed](Reconciler::observe) as unprompted.
    Late,
    /// The answer to a write Rift has since replaced: it says nothing about the one with the
    /// app now, and is dropped.
    Stale,
}

/// What the reconciler decided along the way that is not a write: the platform logs these.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Note {
    /// A desire for a window that is the user's was dropped.
    Discarded,
    /// An app did not answer a write in time.
    TimedOut,
    /// A window gone from the model was forgotten when its write's deadline passed.
    Forgotten,
}

fn answers<K: Kinds>(target: K::Frame, frame: K::Frame) -> bool {
    K::is_within(frame, ACK_TOLERANCE, target)
}

#[derive(Debug)]
pub struct Reconciler<K: Kinds> {
    /// Every window the model knows, and where it stands.
    syncs: HashMap<K::Window, Sync<K>>,
    /// Every write sent, with its deadline, in the order sent. A deadline is the send time
    /// plus [`WRITE_DEADLINE`], so this is also deadline order. An entry whose window has since
    /// been answered or written again no longer matches its sync state and is dropped when it
    /// reaches the front.
    deadlines: VecDeque<(K::Time, K::Window)>,
    /// What was decided along the way, for the caller to drain ([`Reconciler::notes`]).
    notes: Vec<(K::Window, Note)>,
    /// Windows whose state changed since the last call to [`Reconciler::touched`].
    touched: Vec<K::Window>,
}

impl<K: Kinds> Default for Reconciler<K> {
    fn default() -> Self {
        Reconciler {
            syncs: HashMap::new(),
            deadlines: VecDeque::new(),
            notes: Vec::new(),
            touched: Vec::new(),
        }
    }
}

impl<K: Kinds> Reconciler<K> {
    /// Where `window` stands; `None` for a window the model does not know.
    pub fn sync(&self, window: K::Window) -> Option<Sync<K>> { self.syncs.get(&window).copied() }

    /// What was decided since the last call that is not a write.
    pub fn notes(&mut self) -> Vec<(K::Window, Note)> { std::mem::take(&mut self.notes) }

    fn set(&mut self, window: K::Window, sync: Sync<K>) {
        self.syncs.insert(window, sync);
        self.touched.push(window);
    }

    /// The windows whose state changed since the last call.
    pub fn touched(&mut self) -> Vec<K::Window> {
        let mut touched = std::mem::take(&mut self.touched);
        touched.dedup();
        touched
    }

    /// The frame Rift has pending for `window`, if any: the target of a write in flight or of
    /// an animation, or the frame its app refused (it still says where Rift means the window to
    /// be).
    pub fn pending(&self, window: K::Window) -> Option<K::Frame> {
        match self.sync(window)? {
            Sync::InFlight { target, .. } | Sync::Animating { target, .. } => Some(target),
            Sync::Refused { wanted, .. } => Some(wanted),
            Sync::Settled { .. } | Sync::Unknown => None,
        }
    }

    /// A window the model did not know appeared at `frame`: its app has it there.
    pub fn appear(&mut self, window: K::Window, frame: K::Frame) {
        self.syncs.entry(window).or_insert(Sync::Settled { acked: frame });
    }

    /// The app's window was read at `frame` (an inventory, the window server): its app has it
    /// there. A write in flight is still waited for (its answer decides), and a refusal stands
    /// (the model's frame, which this read moved, decides whether it is asked again).
    pub fn found(&mut self, window: K::Window, frame: K::Frame) {
        match self.sync(window) {
            None | Some(Sync::InFlight { .. } | Sync::Refused { .. } | Sync::Animating { .. }) => {}
            Some(Sync::Settled { .. } | Sync::Unknown) => {
                self.set(window, Sync::Settled { acked: frame })
            }
        }
    }

    /// `window` is gone: nothing is tracked for it any more.
    pub fn remove(&mut self, window: K::Window) {
        self.syncs.remove(&window);
        self.touched.push(window);
    }

    /// Rift no longer knows where the app has `window` (a drag ended on it, the displays
    /// changed). A write in flight still has its answer to come; what it falls back to if that
    /// never comes is unknown too.
    pub fn distrust(&mut self, window: K::Window) {
        let unknown = match self.sync(window) {
            None => return,
            Some(Sync::InFlight {
                target,
                deadline,
                queued,
                request,
                before: _,
            }) => Sync::InFlight {
                target,
                deadline,
                queued,
                request,
                before: Before::Unknown,
            },
            Some(
                Sync::Settled { .. }
                | Sync::Refused { .. }
                | Sync::Unknown
                | Sync::Animating { .. },
            ) => Sync::Unknown,
        };
        self.set(window, unknown);
    }

    /// What a write sent now would fall back to.
    fn before(sync: Sync<K>) -> Before<K> {
        match sync {
            Sync::Settled { acked } | Sync::Animating { target: acked, .. } => Before::Acked(acked),
            Sync::Refused { got, .. } => Before::Acked(got),
            Sync::Unknown => Before::Unknown,
            Sync::InFlight { before, .. } => before,
        }
    }

    fn send(
        &mut self,
        now: K::Time,
        request: K::Request,
        window: K::Window,
        frame: K::Frame,
        before: Before<K>,
    ) -> Write<K> {
        let deadline = K::after(now, WRITE_DEADLINE);
        self.deadlines.push_back((deadline, window));
        self.set(window, Sync::InFlight {
            target: frame,
            deadline,
            queued: None,
            request,
            before,
        });
        Write { window, frame, request }
    }

    /// Rift wants `window` at `desired`; the model last saw it at `known`. A write goes out
    /// under `request`.
    #[allow(clippy::too_many_arguments)]
    pub fn want(
        &mut self,
        now: K::Time,
        request: K::Request,
        window: K::Window,
        desired: K::Frame,
        known: K::Frame,
        want: Want,
        owner: Owner,
    ) -> Verdict<K> {
        let Some(mut sync) = self.sync(window) else {
            return Verdict::Nothing;
        };
        // A write nobody answered in time no longer holds newer desires back. Whatever was
        // queued behind it is older than `desired`, so it is dropped.
        if let Sync::InFlight { target, deadline, before, .. } = sync
            && now >= deadline
        {
            sync = self.time_out(window, target, before);
        }
        match owner {
            Owner::Rift => {}
            Owner::User => {
                // Discarded, not queued: the release re-derives the window's frame.
                self.notes.push((window, Note::Discarded));
                return Verdict::Nothing;
            }
        }
        match (sync, want) {
            (
                Sync::InFlight {
                    target,
                    deadline,
                    queued,
                    request: sent,
                    before,
                },
                Want::Arrange | Want::ArrangePositions,
            ) => {
                if K::same_as(desired, target) {
                    if queued.is_some() {
                        self.set(window, Sync::InFlight {
                            target,
                            deadline,
                            queued: None,
                            request: sent,
                            before,
                        });
                    }
                    return Verdict::Nothing;
                }
                let queued = Some(desired);
                self.set(window, Sync::InFlight {
                    target,
                    deadline,
                    queued,
                    request: sent,
                    before,
                });
                Verdict::Queued
            }
            // Asking again for what was refused would be a retry loop. A window that has
            // since been seen somewhere else is no longer the one that refused.
            (Sync::Refused { wanted, got }, Want::Arrange | Want::ArrangePositions)
                if K::same_as(desired, wanted) && K::same_as(known, got) =>
            {
                Verdict::Nothing
            }
            // The app has the window there already (or an animation is taking it there).
            (
                Sync::Settled { acked } | Sync::Animating { target: acked, .. },
                Want::Arrange | Want::ArrangePositions,
            ) if K::same_as(desired, acked) => Verdict::Nothing,
            // A refused window is where the model has it (a timed-out write's `got` is only what
            // was acknowledged before it: the write may still have landed).
            (Sync::Refused { .. }, Want::Arrange | Want::ArrangePositions)
                if K::same_as(desired, known) =>
            {
                Verdict::Nothing
            }
            (
                Sync::Settled { .. }
                | Sync::Refused { .. }
                | Sync::Unknown
                | Sync::Animating { .. },
                Want::Arrange | Want::ArrangePositions,
            ) => Verdict::Send(self.send(now, request, window, desired, Self::before(sync))),
        }
    }

    /// The interactive frame of the window the user is dragging with the modifier held: the
    /// drag's own desire, and the only write a dragged window gets. It is not tracked: the
    /// next one replaces it in the app's queue, and the drop arranges the window afresh.
    pub fn follow(&mut self, request: K::Request, window: K::Window, frame: K::Frame) -> Write<K> {
        if self.syncs.contains_key(&window) {
            self.set(window, Sync::Settled { acked: frame });
        }
        Write { window, frame, request }
    }

    /// What waited behind a write that has been answered or given up on: sent if the window is
    /// Rift's, discarded if it is the user's now (the release re-derives it).
    fn release(
        &mut self,
        now: K::Time,
        request: K::Request,
        window: K::Window,
        queued: Option<K::Frame>,
        owner: Owner,
    ) -> Option<Write<K>> {
        match (owner, queued) {
            (Owner::Rift, Some(next)) => {
                let before = self.sync(window).map_or(Before::Unknown, Self::before);
                Some(self.send(now, request, window, next, before))
            }
            (Owner::Rift | Owner::User, None) => None,
            (Owner::User, Some(_)) => {
                self.notes.push((window, Note::Discarded));
                None
            }
        }
    }

    /// The app answered a write to `window` with `frame`; `seen` is the request its answer
    /// names, if it names one. What waited behind the write goes out under `request`.
    #[allow(clippy::too_many_arguments)]
    pub fn answer(
        &mut self,
        now: K::Time,
        request: K::Request,
        seen: Option<K::Request>,
        window: K::Window,
        frame: K::Frame,
        owner: Owner,
    ) -> Reply<K> {
        match self.sync(window) {
            None => Reply::Answered(Answer::Unasked, None),
            // Not an answer to anything Rift tracks, but where the app has the window.
            Some(Sync::Settled { .. } | Sync::Unknown) => {
                self.set(window, Sync::Settled { acked: frame });
                Reply::Answered(Answer::Unasked, None)
            }
            // A frame of the animation, or one that landed on its target (which settles it).
            Some(Sync::Animating { target, .. }) => {
                if answers::<K>(target, frame) {
                    self.set(window, Sync::Settled { acked: frame });
                }
                Reply::Answered(Answer::Unasked, None)
            }
            Some(Sync::InFlight { request: sent, .. }) if seen.is_some_and(|seen| seen != sent) => {
                Reply::Stale
            }
            Some(Sync::InFlight { target, queued, .. }) => {
                let answer = if answers::<K>(target, frame) {
                    self.set(window, Sync::Settled { acked: frame });
                    Answer::Acked
                } else {
                    self.set(window, Sync::Refused { wanted: target, got: frame });
                    Answer::Refused
                };
                Reply::Answered(answer, self.release(now, request, window, queued, owner))
            }
            Some(Sync::Refused { .. }) => Reply::Late,
        }
    }

    /// `window` reported `frame` unprompted; the model last saw it at `known`. A report of the
    /// frame in flight is no news, and does not end the wait: the app's answer does, and it can
    /// still differ. Any other report is where the app has the window.
    pub fn observe(
        &mut self,
        window: K::Window,
        frame: K::Frame,
        known: K::Frame,
        holder: Holder,
    ) -> Observation {
        let Some(sync) = self.sync(window) else {
            return Observation::Foreign;
        };
        match (sync, holder) {
            (Sync::InFlight { target, .. }, Holder::User | Holder::Nobody)
                if answers::<K>(target, frame) =>
            {
                Observation::Ack
            }
            (Sync::InFlight { .. }, Holder::Nobody) => Observation::InTransit,
            (
                Sync::InFlight { .. }
                | Sync::Settled { .. }
                | Sync::Refused { .. }
                | Sync::Unknown
                | Sync::Animating { .. },
                Holder::User | Holder::Nobody,
            ) => {
                let observation = if K::same_as(frame, known) {
                    Observation::Ack
                } else {
                    Observation::Foreign
                };
                match (sync, observation) {
                    // A refusal stands while the window stays where the model has it.
                    (Sync::Refused { .. }, Observation::Ack) => {}
                    _ => self.set(window, Sync::Settled { acked: frame }),
                }
                observation
            }
        }
    }

    /// The app did not answer the write of `target` in time: a refusal, with the window where
    /// it was acknowledged before the write, or unknown if that was unknown.
    fn time_out(&mut self, window: K::Window, target: K::Frame, before: Before<K>) -> Sync<K> {
        self.notes.push((window, Note::TimedOut));
        let late = match before {
            Before::Acked(got) => Sync::Refused { wanted: target, got },
            Before::Unknown => Sync::Unknown,
        };
        self.set(window, late);
        late
    }

    /// The write a `deadlines` entry stands for, if it is still waiting: its target, what was
    /// queued behind it, and what it falls back to.
    #[allow(clippy::type_complexity)]
    fn waiting(
        &self,
        (deadline, window): (K::Time, K::Window),
    ) -> Option<(K::Frame, Option<K::Frame>, Before<K>)> {
        match self.sync(window)? {
            Sync::InFlight {
                target,
                deadline: due,
                queued,
                before,
                ..
            } if due == deadline => Some((target, queued, before)),
            Sync::InFlight { .. }
            | Sync::Settled { .. }
            | Sync::Refused { .. }
            | Sync::Unknown
            | Sync::Animating { .. } => None,
        }
    }

    /// Stops waiting for every write whose deadline has passed at `now`. `owner` says whose a
    /// window is, `None` once the model no longer has it (it is then forgotten). Returns the
    /// writes that were queued behind the late ones.
    pub fn expire(
        &mut self,
        now: K::Time,
        request: K::Request,
        owner: impl Fn(K::Window) -> Option<Owner>,
    ) -> Vec<Write<K>> {
        let mut released = Vec::new();
        while let Some(&(deadline, window)) = self.deadlines.front()
            && deadline <= now
        {
            self.deadlines.pop_front();
            let Some((target, queued, before)) = self.waiting((deadline, window)) else {
                continue;
            };
            match owner(window) {
                Some(owner) => {
                    self.time_out(window, target, before);
                    released.extend(self.release(now, request, window, queued, owner));
                }
                None => {
                    self.notes.push((window, Note::Forgotten));
                    self.remove(window);
                }
            }
        }
        released
    }

    /// When the earliest unanswered write stops being waited for.
    pub fn next_deadline(&mut self) -> Option<K::Time> {
        while let Some(&entry) = self.deadlines.front() {
            if self.waiting(entry).is_some() {
                return Some(entry.0);
            }
            self.deadlines.pop_front();
        }
        None
    }

    /// Should `window` be animated from where the model has it (`known`) to `desired`? Not when
    /// what is pending for it (a write in flight, a refused frame, an animation) is already
    /// `desired`, nor, with nothing pending, when the model has it there; never the user's.
    /// Animated, it is held as [`Sync::Animating`] under `request`.
    pub fn animate(
        &mut self,
        request: K::Request,
        window: K::Window,
        desired: K::Frame,
        known: K::Frame,
        owner: Owner,
    ) -> bool {
        let Some(sync) = self.sync(window) else {
            return false;
        };
        let pending = match sync {
            Sync::InFlight { target, .. } | Sync::Animating { target, .. } => Some(target),
            Sync::Refused { wanted, .. } => Some(wanted),
            Sync::Settled { .. } | Sync::Unknown => None,
        };
        let already = match pending {
            Some(pending) => K::same_as(pending, desired),
            None => K::same_as(known, desired),
        };
        match (owner, already) {
            (Owner::User, _) | (Owner::Rift, true) => false,
            (Owner::Rift, false) => {
                self.set(window, Sync::Animating { target: desired, request });
                true
            }
        }
    }

    /// The animation under `request` ended. A window it never landed on its target is where the
    /// app left it, which nobody reported: unknown.
    pub fn animation_ended(&mut self, request: K::Request) {
        let ended: Vec<K::Window> = self
            .syncs
            .iter()
            .filter(|(_, sync)| {
                matches!(sync, Sync::Animating { request: sent, .. } if *sent == request)
            })
            .map(|(window, _)| *window)
            .collect();
        for window in ended {
            self.set(window, Sync::Unknown);
        }
    }

    /// `window`'s animation was stopped where the model has it (`known`): the app has it there.
    /// Only an animating window is touched.
    pub fn animation_stopped(&mut self, window: K::Window, known: K::Frame) {
        if let Some(Sync::Animating { .. }) = self.sync(window) {
            self.set(window, Sync::Settled { acked: known });
        }
    }

    /// Something outside the reconciler (an animation, a topology change, a failed send) took
    /// `window`'s frame: nothing is outstanding, and the app has it where the model has it.
    pub fn forget(&mut self, window: K::Window, known: K::Frame) {
        if self.syncs.contains_key(&window) {
            self.set(window, Sync::Settled { acked: known });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plain::{Nanos, Plain, Rect, RequestId, WindowId};

    type Reconciler = super::Reconciler<Plain>;
    type Sync = super::Sync<Plain>;
    type Verdict = super::Verdict<Plain>;
    type Reply = super::Reply<Plain>;

    /// The request every write in these tests goes out under, and an older one.
    const R: RequestId = RequestId(2);
    const OLD: RequestId = RequestId(1);

    fn at(x: f64) -> Rect { Rect { x, y: 0., w: 400., h: 300. } }

    fn window() -> WindowId { WindowId { pid: 1, idx: 1 } }

    const T0: Nanos = Nanos(1_000);

    fn later(by: Duration) -> Nanos { T0 + by }

    /// A reconciler that knows `window()`, its app having it at `acked`.
    fn with(acked: Rect) -> Reconciler {
        let mut reconciler = Reconciler::default();
        reconciler.appear(window(), acked);
        reconciler
    }

    fn want(reconciler: &mut Reconciler, now: Nanos, desired: Rect, known: Rect) -> Verdict {
        reconciler.want(now, R, window(), desired, known, Want::Arrange, Owner::Rift)
    }

    fn sent(verdict: Verdict) -> Rect {
        match verdict {
            Verdict::Send(write) => write.frame,
            Verdict::Queued | Verdict::Nothing => panic!("expected a write, got {verdict:?}"),
        }
    }

    fn sync(reconciler: &Reconciler) -> Sync { reconciler.sync(window()).expect("known window") }

    #[test]
    fn a_window_its_app_has_where_layout_wants_it_is_not_written() {
        let mut reconciler = with(at(10.));
        // Whatever the model believes (a prepared placement moved it): the app's frame decides.
        assert_eq!(want(&mut reconciler, T0, at(10.), at(50.)), Verdict::Nothing);
        assert_eq!(sent(want(&mut reconciler, T0, at(50.), at(50.))), at(50.));
    }

    #[test]
    fn a_window_the_model_does_not_know_is_not_written() {
        let mut reconciler = Reconciler::default();
        assert_eq!(want(&mut reconciler, T0, at(10.), at(0.)), Verdict::Nothing);
    }

    #[test]
    fn one_write_is_in_flight_and_only_the_latest_desire_waits_behind_it() {
        let mut reconciler = with(at(0.));
        assert_eq!(sent(want(&mut reconciler, T0, at(10.), at(0.))), at(10.));
        assert_eq!(want(&mut reconciler, T0, at(10.), at(10.)), Verdict::Nothing);
        assert_eq!(want(&mut reconciler, T0, at(20.), at(10.)), Verdict::Queued);
        assert_eq!(want(&mut reconciler, T0, at(30.), at(20.)), Verdict::Queued);

        let Reply::Answered(answer, next) =
            reconciler.answer(T0, R, None, window(), at(10.5), Owner::Rift)
        else {
            panic!("expected an answer");
        };
        assert_eq!(answer, Answer::Acked);
        assert_eq!(next.map(|write| write.frame), Some(at(30.)));
        assert!(matches!(sync(&reconciler), Sync::InFlight { .. }));

        assert_eq!(
            reconciler.answer(T0, R, None, window(), at(30.), Owner::Rift),
            Reply::Answered(Answer::Acked, None)
        );
        assert_eq!(sync(&reconciler), Sync::Settled { acked: at(30.) });
    }

    #[test]
    fn a_desire_withdrawn_while_queued_is_not_sent() {
        let mut reconciler = with(at(0.));
        _ = want(&mut reconciler, T0, at(10.), at(0.));
        assert_eq!(want(&mut reconciler, T0, at(20.), at(10.)), Verdict::Queued);
        assert_eq!(want(&mut reconciler, T0, at(10.), at(20.)), Verdict::Nothing);
        assert_eq!(
            reconciler.answer(T0, R, None, window(), at(10.), Owner::Rift),
            Reply::Answered(Answer::Acked, None)
        );
    }

    #[test]
    fn an_adjusted_answer_is_a_refusal_that_is_not_asked_again() {
        let mut reconciler = with(at(0.));
        _ = want(&mut reconciler, T0, at(10.), at(0.));
        assert_eq!(
            reconciler.answer(T0, R, None, window(), at(14.), Owner::Rift),
            Reply::Answered(Answer::Refused, None)
        );
        assert_eq!(sync(&reconciler), Sync::Refused {
            wanted: at(10.),
            got: at(14.)
        });
        for _ in 0..3 {
            assert_eq!(want(&mut reconciler, T0, at(10.), at(14.)), Verdict::Nothing);
        }
        assert_eq!(sent(want(&mut reconciler, T0, at(50.), at(14.))), at(50.));
    }

    #[test]
    fn a_refused_window_seen_somewhere_else_is_asked_again() {
        let mut reconciler = with(at(0.));
        _ = want(&mut reconciler, T0, at(10.), at(0.));
        _ = reconciler.answer(T0, R, None, window(), at(14.), Owner::Rift);
        assert_eq!(
            reconciler.observe(window(), at(90.), at(14.), Holder::Nobody),
            Observation::Foreign
        );
        assert_eq!(sync(&reconciler), Sync::Settled { acked: at(90.) });
        assert_eq!(sent(want(&mut reconciler, T0, at(10.), at(90.))), at(10.));
    }

    #[test]
    fn an_unanswered_write_times_out_where_it_was_acknowledged_and_releases_what_waited() {
        let mut reconciler = with(at(0.));
        _ = want(&mut reconciler, T0, at(10.), at(0.));
        _ = want(&mut reconciler, T0, at(20.), at(10.));
        assert!(reconciler.expire(T0, R, |_| Some(Owner::Rift)).is_empty());

        let t1 = later(WRITE_DEADLINE);
        let released = reconciler.expire(t1, R, |_| Some(Owner::Rift));
        assert_eq!(released.iter().map(|write| write.frame).collect::<Vec<_>>(), [
            at(20.)
        ]);

        // The released write times out too: the app had acknowledged the window at 0 before
        // either write.
        let t2 = t1 + WRITE_DEADLINE;
        assert!(reconciler.expire(t2, R, |_| Some(Owner::Rift)).is_empty());
        assert_eq!(sync(&reconciler), Sync::Refused { wanted: at(20.), got: at(0.) });
        // Its answer, when it comes, is news.
        assert_eq!(
            reconciler.answer(t2, R, None, window(), at(20.), Owner::Rift),
            Reply::Late
        );
        assert_eq!(
            reconciler.observe(window(), at(20.), at(0.), Holder::Nobody),
            Observation::Foreign
        );
        assert_eq!(sync(&reconciler), Sync::Settled { acked: at(20.) });
    }

    #[test]
    fn a_write_from_unknown_that_times_out_is_unknown_again() {
        let mut reconciler = with(at(0.));
        reconciler.distrust(window());
        _ = want(&mut reconciler, T0, at(10.), at(0.));
        assert!(reconciler.expire(later(WRITE_DEADLINE), R, |_| Some(Owner::Rift)).is_empty());
        assert_eq!(sync(&reconciler), Sync::Unknown);
    }

    #[test]
    fn a_desire_after_the_deadline_is_sent_not_held_behind_what_was_queued() {
        let mut reconciler = with(at(0.));
        _ = want(&mut reconciler, T0, at(10.), at(0.));
        _ = want(&mut reconciler, T0, at(20.), at(0.));
        let t1 = later(WRITE_DEADLINE);
        assert_eq!(sent(want(&mut reconciler, t1, at(30.), at(0.))), at(30.));
        assert!(matches!(
            sync(&reconciler),
            Sync::InFlight { target, queued: None, .. } if target == at(30.)
        ));
    }

    #[test]
    fn unknown_is_written_at_once_even_where_the_app_had_it() {
        let mut reconciler = with(at(10.));
        // The displays changed, or the user dragged it: where the app has it is unknown.
        reconciler.distrust(window());
        assert_eq!(sent(want(&mut reconciler, T0, at(10.), at(10.))), at(10.));
    }

    #[test]
    fn a_report_settles_unknown() {
        let mut reconciler = with(at(10.));
        reconciler.distrust(window());
        assert_eq!(
            reconciler.observe(window(), at(10.), at(10.), Holder::Nobody),
            Observation::Ack
        );
        assert_eq!(sync(&reconciler), Sync::Settled { acked: at(10.) });
        assert_eq!(want(&mut reconciler, T0, at(10.), at(10.)), Verdict::Nothing);
    }

    #[test]
    fn a_desire_for_the_users_window_is_discarded_and_changes_nothing() {
        let mut reconciler = with(at(10.));
        assert_eq!(
            reconciler.want(T0, R, window(), at(50.), at(50.), Want::Arrange, Owner::User),
            Verdict::Nothing
        );
        assert_eq!(sync(&reconciler), Sync::Settled { acked: at(10.) });
        assert_eq!(sent(want(&mut reconciler, T0, at(50.), at(50.))), at(50.));
    }

    #[test]
    fn a_report_during_a_write_is_the_users_only_if_their_hand_is_on_the_window() {
        let mut reconciler = with(at(0.));
        _ = want(&mut reconciler, T0, at(10.), at(0.));
        assert_eq!(
            reconciler.observe(window(), at(5.), at(10.), Holder::Nobody),
            Observation::InTransit
        );
        assert!(matches!(sync(&reconciler), Sync::InFlight { .. }));
        // The user holds it: theirs, and Rift stops waiting.
        assert_eq!(
            reconciler.observe(window(), at(70.), at(10.), Holder::User),
            Observation::Foreign
        );
        assert_eq!(sync(&reconciler), Sync::Settled { acked: at(70.) });
        // The answer to the abandoned write is not Rift's to wait for, but says where it is.
        assert_eq!(
            reconciler.answer(T0, R, None, window(), at(10.), Owner::Rift),
            Reply::Answered(Answer::Unasked, None)
        );
        assert_eq!(sync(&reconciler), Sync::Settled { acked: at(10.) });
    }

    #[test]
    fn a_report_of_the_target_is_no_news_and_the_answer_still_decides() {
        let mut reconciler = with(at(0.));
        _ = want(&mut reconciler, T0, at(10.), at(0.));
        for holder in [Holder::Nobody, Holder::User] {
            assert_eq!(
                reconciler.observe(window(), at(10.), at(10.), holder),
                Observation::Ack
            );
            assert!(matches!(sync(&reconciler), Sync::InFlight { .. }));
        }
        assert_eq!(
            reconciler.answer(T0, R, None, window(), at(13.), Owner::Rift),
            Reply::Answered(Answer::Refused, None)
        );
        assert_eq!(sync(&reconciler), Sync::Refused {
            wanted: at(10.),
            got: at(13.)
        });
    }

    #[test]
    fn an_answer_naming_a_replaced_write_is_stale() {
        let mut reconciler = with(at(0.));
        _ = reconciler.want(T0, OLD, window(), at(10.), at(0.), Want::Arrange, Owner::Rift);
        let t1 = later(WRITE_DEADLINE);
        _ = reconciler.want(t1, R, window(), at(20.), at(0.), Want::Arrange, Owner::Rift);
        assert_eq!(
            reconciler.answer(t1, R, Some(OLD), window(), at(10.), Owner::Rift),
            Reply::Stale
        );
        assert!(matches!(sync(&reconciler), Sync::InFlight { request: R, .. }));
        assert_eq!(
            reconciler.answer(t1, R, Some(R), window(), at(20.), Owner::Rift),
            Reply::Answered(Answer::Acked, None)
        );
    }

    #[test]
    fn the_next_deadline_is_the_earliest_unanswered_write() {
        let mut reconciler = with(at(0.));
        let other = WindowId { pid: 1, idx: 2 };
        reconciler.appear(other, at(0.));
        assert_eq!(reconciler.next_deadline(), None);
        _ = want(&mut reconciler, T0, at(10.), at(0.));
        let t1 = later(Duration::from_millis(100));
        _ = reconciler.want(t1, R, other, at(10.), at(0.), Want::Arrange, Owner::Rift);
        assert_eq!(reconciler.next_deadline(), Some(T0 + WRITE_DEADLINE));
        _ = reconciler.answer(t1, R, None, window(), at(10.), Owner::Rift);
        assert_eq!(reconciler.next_deadline(), Some(t1 + WRITE_DEADLINE));
    }

    #[test]
    fn an_animation_is_planned_only_where_nothing_already_takes_the_window_there() {
        let mut reconciler = with(at(0.));
        assert!(
            !reconciler.animate(R, window(), at(0.), at(0.), Owner::Rift),
            "already there"
        );
        assert!(
            !reconciler.animate(R, window(), at(10.), at(0.), Owner::User),
            "the user's"
        );
        assert!(reconciler.animate(R, window(), at(10.), at(0.), Owner::Rift));
        assert_eq!(sync(&reconciler), Sync::Animating {
            target: at(10.),
            request: R
        });
        // Pending already: the same target again is nothing.
        assert!(!reconciler.animate(OLD, window(), at(10.), at(10.), Owner::Rift));
    }

    #[test]
    fn an_animation_settles_where_it_lands_and_its_frames_on_the_way_refuse_nothing() {
        let mut reconciler = with(at(0.));
        assert!(reconciler.animate(R, window(), at(100.), at(0.), Owner::Rift));
        assert_eq!(
            reconciler.answer(T0, OLD, Some(R), window(), at(40.), Owner::Rift),
            Reply::Answered(Answer::Unasked, None)
        );
        assert!(matches!(sync(&reconciler), Sync::Animating { .. }));
        _ = reconciler.answer(T0, OLD, Some(R), window(), at(100.), Owner::Rift);
        assert_eq!(sync(&reconciler), Sync::Settled { acked: at(100.) });
        // Its end changes nothing any more.
        reconciler.animation_ended(R);
        assert_eq!(sync(&reconciler), Sync::Settled { acked: at(100.) });
    }

    #[test]
    fn an_animation_whose_app_never_lands_on_the_target_ends_unknown_and_is_not_held_forever() {
        let mut reconciler = with(at(0.));
        assert!(reconciler.animate(R, window(), at(100.), at(0.), Owner::Rift));
        // The app keeps the window short of the target (a minimum size, a rounding).
        _ = reconciler.answer(T0, OLD, Some(R), window(), at(96.), Owner::Rift);
        assert!(matches!(sync(&reconciler), Sync::Animating { .. }));
        reconciler.animation_ended(R);
        assert_eq!(sync(&reconciler), Sync::Unknown);
        // The next desire is written at once; the app's answer is a refusal, asked once.
        assert_eq!(sent(want(&mut reconciler, T0, at(100.), at(100.))), at(100.));
        _ = reconciler.answer(T0, R, None, window(), at(96.), Owner::Rift);
        assert_eq!(sync(&reconciler), Sync::Refused {
            wanted: at(100.),
            got: at(96.)
        });
        assert_eq!(want(&mut reconciler, T0, at(100.), at(96.)), Verdict::Nothing);
    }

    #[test]
    fn a_desire_during_an_animation_treats_the_window_as_at_its_target() {
        let mut reconciler = with(at(0.));
        assert!(reconciler.animate(OLD, window(), at(100.), at(0.), Owner::Rift));
        assert_eq!(want(&mut reconciler, T0, at(100.), at(100.)), Verdict::Nothing);
        assert_eq!(sent(want(&mut reconciler, T0, at(50.), at(100.))), at(50.));
        // The superseded animation's end does not touch the write in flight.
        reconciler.animation_ended(OLD);
        assert!(matches!(sync(&reconciler), Sync::InFlight { .. }));
    }

    #[test]
    fn a_window_gone_by_its_deadline_is_forgotten() {
        let mut reconciler = with(at(0.));
        _ = want(&mut reconciler, T0, at(10.), at(0.));
        assert!(reconciler.expire(later(WRITE_DEADLINE), R, |_| None).is_empty());
        assert_eq!(reconciler.sync(window()), None);
    }
}
