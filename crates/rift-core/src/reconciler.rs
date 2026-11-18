//! Desired frames, and the one place that decides to write them.
//!
//! Layout says where each window should be. The [`Reconciler`] compares that with what each
//! window's app has acknowledged and mints a [`Write`] only where they differ. A `Write` cannot
//! be built anywhere else, and the platform's senders take nothing but a `Write`, so a frame
//! reaches an app only because the reconciler decided it should.
//!
//! Time is never read here: every call that needs it is handed `now`.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use crate::Kinds;
use crate::pointer::{Holder, Owner};

/// How far an app's answer may be from the frame Rift asked for and still be that frame
/// (yabai's `AX_DIFF`).
pub const ACK_TOLERANCE: f64 = 1.5;

/// How long an app has to answer a write before Rift stops waiting for it.
pub const WRITE_DEADLINE: Duration = Duration::from_millis(300);

/// A frame the reconciler decided to send. Only this crate can make one.
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(bound(
        serialize = "K::Window: serde::Serialize, K::Frame: serde::Serialize, K::Time: serde::Serialize",
        deserialize = "K::Window: serde::Deserialize<'de>, K::Frame: serde::Deserialize<'de>, K::Time: serde::Deserialize<'de>"
    ))
)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Write<K: Kinds> {
    window: K::Window,
    frame: K::Frame,
}

impl<K: Kinds> Write<K> {
    pub fn window(&self) -> K::Window { self.window }

    pub fn frame(&self) -> K::Frame { self.frame }

    /// A write decided by a core implemented outside this crate (the Roc core's adapter). The
    /// platform's senders still take only what came out of a [`Core`](crate::Core).
    pub fn from_foreign_core(window: K::Window, frame: K::Frame) -> Self { Write { window, frame } }

    /// The same write on another platform's types (a recorded trace replayed on plain data).
    pub fn translate<J: Kinds>(self, to: &impl Translate<K, J>) -> Write<J> {
        Write {
            window: to.window(self.window),
            frame: to.frame(self.frame),
        }
    }
}

/// A way from one platform's types to another's.
pub trait Translate<K: Kinds, J: Kinds> {
    fn window(&self, window: K::Window) -> J::Window;
    fn frame(&self, frame: K::Frame) -> J::Frame;
    fn time(&self, time: K::Time) -> J::Time;
}

/// Where one window stands between what Rift asked of its app and what the app answered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sync<K: Kinds> {
    /// Nothing is outstanding: the window is where the model last saw it.
    Settled,
    /// One write is with the app. A newer desire waits in `queued` (one, the latest) until the
    /// app answers or `deadline` passes.
    InFlight {
        target: K::Frame,
        deadline: K::Time,
        queued: Option<K::Frame>,
    },
    /// The app answered `wanted` with `got`, or did not answer in time (`got` is then where
    /// the model last saw it). Rift does not ask for `wanted` again while the window stays at
    /// `got`.
    Refused { wanted: K::Frame, got: K::Frame },
    /// The user's hand moved it. Where the model last saw it is not to be trusted (reports
    /// lag a drag), so the next desire is sent whatever the model believes.
    Adrift,
}

/// How strongly Rift wants a frame.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Want {
    /// Layout: nothing to send when the window is already known to be there.
    Arrange,
    /// Layout during a workspace switch: as [`Want::Arrange`], and a window that keeps its size
    /// is only moved (a position-only request).
    ArrangePositions,
    /// A placement, or a layout after the cached frames stopped being trustworthy: send it
    /// whatever the model believes.
    Place,
}

/// What the reconciler made of a desire.
#[derive(Debug, PartialEq)]
pub enum Verdict<K: Kinds> {
    /// Send this now.
    Send(Write<K>),
    /// A write is with the app; this desire goes after it.
    Queued,
    /// The window is there, on its way there, or refused to go there.
    Nothing,
}

/// An unprompted report of a window's frame, classified against what Rift asked for.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Observation {
    /// The frame Rift asked for, or the one it already knew: no news.
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
    Unasked,
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
    /// Windows with something outstanding. A window not in here is [`Sync::Settled`].
    unsettled: HashMap<K::Window, Sync<K>>,
    /// Every write sent, with its deadline, in the order sent. A deadline is the send time
    /// plus [`WRITE_DEADLINE`], so this is also deadline order. An entry whose window has since
    /// been answered or written again no longer matches its sync state and is dropped when it
    /// reaches the front.
    deadlines: VecDeque<(K::Time, K::Window)>,
    /// What was decided along the way, for the caller to drain ([`Reconciler::notes`]).
    notes: Vec<(K::Window, Note)>,
}

impl<K: Kinds> Default for Reconciler<K> {
    fn default() -> Self {
        Reconciler {
            unsettled: HashMap::new(),
            deadlines: VecDeque::new(),
            notes: Vec::new(),
        }
    }
}

impl<K: Kinds> Reconciler<K> {
    pub fn sync(&self, window: K::Window) -> Sync<K> {
        self.unsettled.get(&window).copied().unwrap_or(Sync::Settled)
    }

    /// What was decided since the last call that is not a write.
    pub fn notes(&mut self) -> Vec<(K::Window, Note)> { std::mem::take(&mut self.notes) }

    fn set(&mut self, window: K::Window, sync: Sync<K>) {
        match sync {
            Sync::Settled => _ = self.unsettled.remove(&window),
            Sync::InFlight { .. } | Sync::Refused { .. } | Sync::Adrift => {
                _ = self.unsettled.insert(window, sync)
            }
        }
    }

    fn send(
        &mut self,
        now: K::Time,
        window: K::Window,
        frame: K::Frame,
        queued: Option<K::Frame>,
    ) -> Write<K> {
        let deadline = K::after(now, WRITE_DEADLINE);
        self.deadlines.push_back((deadline, window));
        self.set(window, Sync::InFlight {
            target: frame,
            deadline,
            queued,
        });
        Write { window, frame }
    }

    /// Rift wants `window` at `desired`; the model last saw it at `known`.
    pub fn want(
        &mut self,
        now: K::Time,
        window: K::Window,
        desired: K::Frame,
        known: K::Frame,
        want: Want,
        owner: Owner,
    ) -> Verdict<K> {
        let mut sync = self.sync(window);
        // A write nobody answered in time no longer holds newer desires back. Whatever was
        // queued behind it is older than `desired`, so it is dropped.
        if let Sync::InFlight { target, deadline, queued: _ } = sync
            && now >= deadline
        {
            sync = self.time_out(window, target, known);
        }
        match owner {
            Owner::Rift => {}
            Owner::User => {
                self.notes.push((window, Note::Discarded));
                // A transfer's placement has already moved the model to `desired`, so what the
                // model believes may no longer be what the app was told. An answer still due
                // brings the truth; otherwise the next desire is sent regardless.
                match (want, sync) {
                    (
                        Want::Arrange | Want::ArrangePositions,
                        Sync::Settled | Sync::InFlight { .. } | Sync::Refused { .. } | Sync::Adrift,
                    )
                    | (Want::Place, Sync::InFlight { .. }) => {}
                    (Want::Place, Sync::Settled | Sync::Refused { .. } | Sync::Adrift) => {
                        self.set(window, Sync::Adrift)
                    }
                }
                return Verdict::Nothing;
            }
        }
        match (sync, want) {
            (
                Sync::InFlight { target, deadline, queued },
                Want::Arrange | Want::ArrangePositions | Want::Place,
            ) => {
                if K::same_as(desired, target) {
                    if queued.is_some() {
                        self.set(window, Sync::InFlight { target, deadline, queued: None });
                    }
                    return Verdict::Nothing;
                }
                let queued = Some(desired);
                self.set(window, Sync::InFlight { target, deadline, queued });
                Verdict::Queued
            }
            // Asking again for what was refused would be a retry loop. A window that has
            // since been seen somewhere else is no longer the one that refused.
            (Sync::Refused { wanted, got }, Want::Arrange | Want::ArrangePositions)
                if K::same_as(desired, wanted) && K::same_as(known, got) =>
            {
                Verdict::Nothing
            }
            // Nothing to write. A refusal is kept: its answer may still come, late, and is then
            // observed like any unprompted report.
            (Sync::Settled | Sync::Refused { .. }, Want::Arrange | Want::ArrangePositions)
                if K::same_as(desired, known) =>
            {
                Verdict::Nothing
            }
            (
                Sync::Settled | Sync::Refused { .. } | Sync::Adrift,
                Want::Arrange | Want::ArrangePositions | Want::Place,
            ) => Verdict::Send(self.send(now, window, desired, None)),
        }
    }

    /// The interactive frame of the window the user is dragging with the modifier held: the
    /// drag's own desire, and the only write a dragged window gets. It is not tracked: the
    /// next one replaces it in the app's queue, and the drop arranges the window afresh.
    pub fn follow(&mut self, window: K::Window, frame: K::Frame) -> Write<K> {
        self.set(window, Sync::Adrift);
        Write { window, frame }
    }

    /// What waited behind a write that has been answered or given up on: sent if the window is
    /// Rift's. If it is the user's now, the desire is discarded; the model already takes the
    /// window to be at that frame, so it is `Adrift` and the next desire is sent regardless.
    fn release(
        &mut self,
        now: K::Time,
        window: K::Window,
        queued: Option<K::Frame>,
        owner: Owner,
    ) -> Option<Write<K>> {
        match (owner, queued) {
            (Owner::Rift, Some(next)) => Some(self.send(now, window, next, None)),
            (Owner::Rift | Owner::User, None) => None,
            (Owner::User, Some(_)) => {
                self.notes.push((window, Note::Discarded));
                self.set(window, Sync::Adrift);
                None
            }
        }
    }

    /// The app answered its latest write to `window` with `frame`. Returns the queued write,
    /// if one was waiting behind it and the window is still Rift's. `None` when Rift had
    /// stopped waiting (the write timed out, or was refused already): a late answer is not an
    /// answer but news of where the window is, to be [observed](Self::observe) as unprompted.
    pub fn answer(
        &mut self,
        now: K::Time,
        window: K::Window,
        frame: K::Frame,
        owner: Owner,
    ) -> Option<(Answer, Option<Write<K>>)> {
        Some(match self.sync(window) {
            Sync::Settled | Sync::Adrift => (Answer::Unasked, None),
            Sync::InFlight { target, deadline: _, queued } => {
                let answer = if answers::<K>(target, frame) {
                    self.set(window, Sync::Settled);
                    Answer::Acked
                } else {
                    self.set(window, Sync::Refused { wanted: target, got: frame });
                    Answer::Refused
                };
                (answer, self.release(now, window, queued, owner))
            }
            Sync::Refused { .. } => return None,
        })
    }

    /// `window` reported `frame` unprompted; the model last saw it at `known`. A report of the
    /// frame in flight is no news, and does not end the wait: the app's answer does, and it can
    /// still differ.
    pub fn observe(
        &mut self,
        window: K::Window,
        frame: K::Frame,
        known: K::Frame,
        holder: Holder,
    ) -> Observation {
        match (self.sync(window), holder) {
            (Sync::InFlight { target, .. }, Holder::User | Holder::Nobody)
                if answers::<K>(target, frame) =>
            {
                Observation::Ack
            }
            (Sync::InFlight { .. }, Holder::Nobody) => Observation::InTransit,
            (
                Sync::InFlight { .. } | Sync::Settled | Sync::Refused { .. } | Sync::Adrift,
                Holder::User | Holder::Nobody,
            ) => {
                if K::same_as(frame, known) {
                    return Observation::Ack;
                }
                // Whatever Rift was waiting for, the window is someone else's now.
                self.set(window, match holder {
                    Holder::User => Sync::Adrift,
                    Holder::Nobody => Sync::Settled,
                });
                Observation::Foreign
            }
        }
    }

    /// The app did not answer the write of `target` in time: a refusal, with the window where
    /// the model last saw it.
    fn time_out(&mut self, window: K::Window, target: K::Frame, known: K::Frame) -> Sync<K> {
        self.notes.push((window, Note::TimedOut));
        let refused = Sync::Refused { wanted: target, got: known };
        self.set(window, refused);
        refused
    }

    /// The write a `deadlines` entry stands for, if it is still waiting: its target and what
    /// was queued behind it.
    fn waiting(
        &self,
        (deadline, window): (K::Time, K::Window),
    ) -> Option<(K::Frame, Option<K::Frame>)> {
        match self.sync(window) {
            Sync::InFlight { target, deadline: due, queued } if due == deadline => {
                Some((target, queued))
            }
            Sync::InFlight { .. } | Sync::Settled | Sync::Refused { .. } | Sync::Adrift => None,
        }
    }

    /// The windows whose writes are past their deadline at `now` and still waited for: the
    /// ones [`expire`](Self::expire) will ask about.
    pub fn due(&self, now: K::Time) -> Vec<K::Window> {
        self.deadlines
            .iter()
            .take_while(|(deadline, _)| *deadline <= now)
            .filter(|&&entry| self.waiting(entry).is_some())
            .map(|&(_, window)| window)
            .collect()
    }

    /// Stops waiting for every write whose deadline has passed at `now`. `known` is where the
    /// model last saw a window, and whose it is; a window it no longer knows is forgotten.
    /// Returns the writes that were queued behind the late ones.
    pub fn expire(
        &mut self,
        now: K::Time,
        known: impl Fn(K::Window) -> Option<(K::Frame, Owner)>,
    ) -> Vec<Write<K>> {
        let mut released = Vec::new();
        while let Some(&(deadline, window)) = self.deadlines.front()
            && deadline <= now
        {
            self.deadlines.pop_front();
            let Some((target, queued)) = self.waiting((deadline, window)) else {
                continue;
            };
            match known(window) {
                Some((known, owner)) => {
                    self.time_out(window, target, known);
                    released.extend(self.release(now, window, queued, owner));
                }
                None => {
                    self.notes.push((window, Note::Forgotten));
                    self.forget(window)
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

    /// Whether a write to `window` is with its app.
    pub fn in_flight(&self, window: K::Window) -> bool {
        match self.sync(window) {
            Sync::InFlight { .. } => true,
            Sync::Settled | Sync::Refused { .. } | Sync::Adrift => false,
        }
    }

    /// `window` is gone, or something outside the reconciler (an animation) now owns its
    /// frame: nothing is outstanding.
    pub fn forget(&mut self, window: K::Window) { self.set(window, Sync::Settled); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plain::{Nanos, Plain, Rect, WindowId};

    type Reconciler = super::Reconciler<Plain>;
    type Sync = super::Sync<Plain>;
    type Verdict = super::Verdict<Plain>;

    fn at(x: f64) -> Rect { Rect { x, y: 0., w: 400., h: 300. } }

    fn window() -> WindowId { WindowId { pid: 1, idx: 1 } }

    const T0: Nanos = Nanos(1_000);

    fn later(by: Duration) -> Nanos { T0 + by }

    fn sent(verdict: Verdict) -> Rect {
        match verdict {
            Verdict::Send(write) => write.frame,
            Verdict::Queued | Verdict::Nothing => panic!("expected a write, got {verdict:?}"),
        }
    }

    #[test]
    fn a_window_already_where_it_should_be_is_not_written() {
        let mut reconciler = Reconciler::default();
        assert_eq!(
            reconciler.want(T0, window(), at(10.), at(10.), Want::Arrange, Owner::Rift),
            Verdict::Nothing
        );
        // A placement is sent whatever the model believes.
        assert_eq!(
            sent(reconciler.want(T0, window(), at(10.), at(10.), Want::Place, Owner::Rift)),
            at(10.)
        );
    }

    #[test]
    fn one_write_is_in_flight_and_only_the_latest_desire_waits_behind_it() {
        let mut reconciler = Reconciler::default();
        assert_eq!(
            sent(reconciler.want(T0, window(), at(10.), at(0.), Want::Arrange, Owner::Rift)),
            at(10.)
        );
        // The same desire again: nothing. Newer ones: queued, the latest replacing the last.
        assert_eq!(
            reconciler.want(T0, window(), at(10.), at(10.), Want::Arrange, Owner::Rift),
            Verdict::Nothing
        );
        assert_eq!(
            reconciler.want(T0, window(), at(20.), at(10.), Want::Arrange, Owner::Rift),
            Verdict::Queued
        );
        assert_eq!(
            reconciler.want(T0, window(), at(30.), at(20.), Want::Place, Owner::Rift),
            Verdict::Queued
        );

        let (answer, next) = reconciler.answer(T0, window(), at(10.5), Owner::Rift).unwrap();
        assert_eq!(answer, Answer::Acked);
        assert_eq!(next.map(|write| write.frame), Some(at(30.)));
        assert!(reconciler.in_flight(window()));

        assert_eq!(
            reconciler.answer(T0, window(), at(30.), Owner::Rift),
            Some((Answer::Acked, None))
        );
        assert_eq!(reconciler.sync(window()), Sync::Settled);
    }

    #[test]
    fn a_desire_withdrawn_while_queued_is_not_sent() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(T0, window(), at(10.), at(0.), Want::Arrange, Owner::Rift);
        assert_eq!(
            reconciler.want(T0, window(), at(20.), at(10.), Want::Arrange, Owner::Rift),
            Verdict::Queued
        );
        // Layout changes its mind back to what is in flight.
        assert_eq!(
            reconciler.want(T0, window(), at(10.), at(20.), Want::Arrange, Owner::Rift),
            Verdict::Nothing
        );
        assert_eq!(
            reconciler.answer(T0, window(), at(10.), Owner::Rift),
            Some((Answer::Acked, None))
        );
    }

    #[test]
    fn an_adjusted_answer_is_a_refusal_that_is_not_asked_again() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(T0, window(), at(10.), at(0.), Want::Arrange, Owner::Rift);
        assert_eq!(
            reconciler.answer(T0, window(), at(14.), Owner::Rift),
            Some((Answer::Refused, None))
        );
        assert_eq!(reconciler.sync(window()), Sync::Refused {
            wanted: at(10.),
            got: at(14.)
        });
        // Layout keeps wanting the same frame; the window stays where the app put it.
        for _ in 0..3 {
            assert_eq!(
                reconciler.want(T0, window(), at(10.), at(14.), Want::Arrange, Owner::Rift),
                Verdict::Nothing
            );
        }
        // A different desire is a new question.
        assert_eq!(
            sent(reconciler.want(T0, window(), at(50.), at(14.), Want::Arrange, Owner::Rift)),
            at(50.)
        );
    }

    #[test]
    fn a_refused_window_seen_somewhere_else_is_asked_again() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(T0, window(), at(10.), at(0.), Want::Arrange, Owner::Rift);
        _ = reconciler.answer(T0, window(), at(14.), Owner::Rift);
        // Its app moved it: no longer the frame that was refused.
        assert_eq!(
            reconciler.observe(window(), at(90.), at(14.), Holder::Nobody),
            Observation::Foreign
        );
        assert_eq!(
            sent(reconciler.want(T0, window(), at(10.), at(90.), Want::Arrange, Owner::Rift)),
            at(10.)
        );
    }

    #[test]
    fn an_unanswered_write_times_out_into_a_refusal_and_releases_what_waited() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(T0, window(), at(10.), at(0.), Want::Arrange, Owner::Rift);
        _ = reconciler.want(T0, window(), at(20.), at(10.), Want::Arrange, Owner::Rift);
        assert!(
            reconciler.expire(T0, |_| Some((at(0.), Owner::Rift))).is_empty(),
            "the deadline has not passed"
        );

        let t1 = later(WRITE_DEADLINE);
        let released = reconciler.expire(t1, |_| Some((at(0.), Owner::Rift)));
        assert_eq!(released.iter().map(|write| write.frame).collect::<Vec<_>>(), [
            at(20.)
        ]);

        // Without anything queued, the late write is a refusal, and its answer, when it does
        // come, settles it.
        let t2 = t1 + WRITE_DEADLINE;
        assert!(reconciler.expire(t2, |_| Some((at(0.), Owner::Rift))).is_empty());
        assert_eq!(reconciler.sync(window()), Sync::Refused {
            wanted: at(20.),
            got: at(0.)
        });
        assert_eq!(reconciler.answer(t2, window(), at(20.), Owner::Rift), None);
        assert_eq!(
            reconciler.observe(window(), at(20.), at(0.), Holder::Nobody),
            Observation::Foreign
        );
        assert_eq!(reconciler.sync(window()), Sync::Settled);
    }

    #[test]
    fn a_late_answer_after_layout_moved_on_is_news() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(T0, window(), at(20.), at(0.), Want::Arrange, Owner::Rift);
        let t1 = later(WRITE_DEADLINE);
        // The app is busy. Layout moves the window back to where it is: nothing to write, and
        // the refusal stays on record, since its answer can still come.
        assert_eq!(
            reconciler.want(t1, window(), at(0.), at(0.), Want::Arrange, Owner::Rift),
            Verdict::Nothing
        );
        assert!(matches!(reconciler.sync(window()), Sync::Refused { .. }));
        // It comes: the window is at the old target, which layout no longer wants. Not an
        // answer any more: news, observed as someone else's move.
        assert_eq!(reconciler.answer(t1, window(), at(20.), Owner::Rift), None);
        assert_eq!(
            reconciler.observe(window(), at(20.), at(0.), Holder::Nobody),
            Observation::Foreign
        );
        // Re-derived, the old desire is written.
        assert_eq!(
            sent(reconciler.want(t1, window(), at(0.), at(20.), Want::Arrange, Owner::Rift)),
            at(0.)
        );
    }

    #[test]
    fn a_desire_after_the_deadline_is_sent_not_held_behind_what_was_queued() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(T0, window(), at(10.), at(0.), Want::Arrange, Owner::Rift);
        _ = reconciler.want(T0, window(), at(20.), at(0.), Want::Arrange, Owner::Rift);
        let t1 = later(WRITE_DEADLINE);
        // The app never answered; the queued frame is older than this desire.
        assert_eq!(
            sent(reconciler.want(t1, window(), at(30.), at(0.), Want::Arrange, Owner::Rift)),
            at(30.)
        );
        assert!(matches!(
            reconciler.sync(window()),
            Sync::InFlight { target, queued: None, .. } if target == at(30.)
        ));
    }

    #[test]
    fn a_report_during_a_write_is_the_users_only_if_their_hand_is_on_the_window() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(T0, window(), at(10.), at(0.), Want::Arrange, Owner::Rift);
        // Nobody holds it: a step on the way.
        assert_eq!(
            reconciler.observe(window(), at(5.), at(10.), Holder::Nobody),
            Observation::InTransit
        );
        assert!(reconciler.in_flight(window()));
        // The user holds it: theirs, and Rift stops waiting.
        assert_eq!(
            reconciler.observe(window(), at(70.), at(10.), Holder::User),
            Observation::Foreign
        );
        assert_eq!(reconciler.sync(window()), Sync::Adrift);
        // The answer to the abandoned write is nobody's.
        assert_eq!(
            reconciler.answer(T0, window(), at(10.), Owner::Rift),
            Some((Answer::Unasked, None))
        );
        // After the drop the window is written even where the model believes it already is.
        assert_eq!(
            sent(reconciler.want(T0, window(), at(70.), at(70.), Want::Arrange, Owner::Rift)),
            at(70.)
        );
    }

    #[test]
    fn a_report_of_the_target_is_no_news_and_the_answer_still_decides() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(T0, window(), at(10.), at(0.), Want::Arrange, Owner::Rift);
        // Even with the user's hand on it: by geometry this is the write landing.
        for holder in [Holder::Nobody, Holder::User] {
            assert_eq!(
                reconciler.observe(window(), at(10.), at(10.), holder),
                Observation::Ack
            );
            assert!(reconciler.in_flight(window()));
        }
        // The app then answers with something else (it adjusted the size when it applied it).
        assert_eq!(
            reconciler.answer(T0, window(), at(13.), Owner::Rift),
            Some((Answer::Refused, None))
        );
        assert_eq!(reconciler.sync(window()), Sync::Refused {
            wanted: at(10.),
            got: at(13.)
        });
    }

    #[test]
    fn the_next_deadline_is_the_earliest_unanswered_write() {
        let mut reconciler = Reconciler::default();
        assert_eq!(reconciler.next_deadline(), None);
        _ = reconciler.want(T0, window(), at(10.), at(0.), Want::Arrange, Owner::Rift);
        let t1 = later(Duration::from_millis(100));
        let other = WindowId { pid: 1, idx: 2 };
        _ = reconciler.want(t1, other, at(10.), at(0.), Want::Arrange, Owner::Rift);
        assert_eq!(reconciler.next_deadline(), Some(T0 + WRITE_DEADLINE));
        _ = reconciler.answer(t1, window(), at(10.), Owner::Rift);
        assert_eq!(reconciler.next_deadline(), Some(t1 + WRITE_DEADLINE));
    }

    #[test]
    fn forgetting_a_window_drops_what_was_outstanding() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(T0, window(), at(10.), at(0.), Want::Arrange, Owner::Rift);
        // The window is gone from the model by the time its write's deadline passes.
        assert!(reconciler.expire(later(WRITE_DEADLINE), |_| None).is_empty());
        assert_eq!(reconciler.sync(window()), Sync::Settled);
    }
}
