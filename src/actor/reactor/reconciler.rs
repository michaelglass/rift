//! Desired frames, and the one place that decides to write them.
//!
//! Layout says where each window should be ([`Desired`]). The [`Reconciler`] compares that with
//! what each window's app has acknowledged and mints a [`Write`] only where they differ. A
//! `Write` cannot be built anywhere else, and the reactor's senders take nothing but a `Write`,
//! so a frame reaches an app only because the reconciler decided it should.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use objc2_core_foundation::CGRect;
use tracing::{debug, warn};

use super::animation::scrolling_bounds;
use super::managers::ScrollingDisplayBounds;
use super::{Reactor, TransactionId};
use crate::actor::app::{AppThreadHandle, FrameMode, FrameSource, Request, WindowId, pid_t};
use crate::actor::drag::Session;
use crate::common::collections::HashMap;
use crate::model::window_store::{Pointer, WindowStore};
use crate::sys::geometry::{IsWithin, Round, SameAs};
use crate::sys::screen::SpaceId;
use crate::sys::window_server::WindowServerId;

/// How far an app's answer may be from the frame Rift asked for and still be that frame
/// (yabai's `AX_DIFF`).
const ACK_TOLERANCE: f64 = 1.5;

/// How long an app has to answer a write before Rift stops waiting for it.
const WRITE_DEADLINE: Duration = Duration::from_millis(300);

/// Where Rift wants the windows of one space: layout as data. A drag does not change the
/// layout, so the window the user holds is in here too, at its slot; the reconciler is what
/// leaves it alone until they let go ([`Owner::User`]).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Desired(Vec<(WindowId, CGRect)>);

impl Desired {
    pub(crate) fn of(layout: &[(WindowId, CGRect)]) -> Self {
        Self(layout.iter().map(|&(wid, frame)| (wid, frame.round())).collect())
    }

    pub(crate) fn frames(&self) -> &[(WindowId, CGRect)] { &self.0 }

    pub(crate) fn windows(&self) -> Vec<WindowId> { self.0.iter().map(|(wid, _)| *wid).collect() }
}

/// The time the reconciler measures deadlines against. Tests drive it by hand, so no test
/// depends on how fast the machine is.
#[derive(Debug)]
pub(crate) enum Clock {
    #[cfg(not(test))]
    System,
    #[cfg(test)]
    Manual(Instant),
}

impl Default for Clock {
    #[cfg(not(test))]
    fn default() -> Self { Clock::System }

    #[cfg(test)]
    fn default() -> Self { Clock::Manual(Instant::now()) }
}

impl Clock {
    pub(crate) fn now(&self) -> Instant {
        match self {
            #[cfg(not(test))]
            Clock::System => Instant::now(),
            #[cfg(test)]
            Clock::Manual(now) => *now,
        }
    }

    #[cfg(test)]
    pub(crate) fn advance(&mut self, by: Duration) {
        let Clock::Manual(now) = self;
        *now += by;
    }
}

/// A frame the reconciler decided to send. Only this module can make one.
#[derive(Debug, PartialEq)]
pub(crate) struct Write {
    window: WindowId,
    frame: CGRect,
}

/// Where one window stands between what Rift asked of its app and what the app answered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Sync {
    /// Nothing is outstanding: the window is where the model last saw it.
    Settled,
    /// One write is with the app. A newer desire waits in `queued` (one, the latest) until the
    /// app answers or `deadline` passes.
    InFlight {
        target: CGRect,
        deadline: Instant,
        queued: Option<CGRect>,
    },
    /// The app answered `wanted` with `got`, or did not answer in time (`got` is then where
    /// the model last saw it). Rift does not ask for `wanted` again while the window stays at
    /// `got`.
    Refused { wanted: CGRect, got: CGRect },
    /// The user's hand moved it. Where the model last saw it is not to be trusted (reports
    /// lag a drag), so the next desire is sent whatever the model believes.
    Adrift,
}

/// How strongly Rift wants a frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Want {
    /// Layout: nothing to send when the window is already known to be there.
    Arrange,
    /// Layout during a workspace switch: as [`Want::Arrange`], and a window that keeps its size
    /// is only moved (a position-only request).
    ArrangePositions,
    /// A placement, or a layout after the cached frames stopped being trustworthy: send it
    /// whatever the model believes.
    Place,
}

/// Whose a window's frame is right now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Owner {
    /// Rift's: layout and placements decide where it goes.
    Rift,
    /// The user's: their hand is on it (pressed or dragging), or they are dragging another
    /// window (a drag does not change the layout; only the drop does). Rift writes nothing to
    /// it but a modifier drag's interactive frame. A desire for it is discarded, not queued:
    /// the release re-derives its frame from what is true then.
    User,
}

/// What the reconciler made of a desire.
#[derive(Debug, PartialEq)]
pub(crate) enum Verdict {
    /// Send this now.
    Send(Write),
    /// A write is with the app; this desire goes after it.
    Queued,
    /// The window is there, on its way there, or refused to go there.
    Nothing,
}

/// Whether the user's hand is on a window a report is about.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Holder {
    User,
    Nobody,
}

/// An unprompted report of a window's frame, classified against what Rift asked for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Observation {
    /// The frame Rift asked for, or the one it already knew: no news.
    Ack,
    /// A write is with the app and nobody holds the window: a step on the way there (size
    /// lands before position). The app's answer settles it.
    InTransit,
    /// Someone else put the window here: the user if their hand is on it, else its app.
    Foreign,
}

/// The app's answer to a write, classified against the write it answers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Answer {
    Acked,
    /// The app put the window somewhere else ([`Sync::Refused`] says where).
    Refused,
    /// Nothing tracked was waiting for it: the answer to a write Rift had stopped waiting for,
    /// or to one sent outside the reconciler (an animation frame, a drag's interactive frame).
    Unasked,
}

fn answers(target: CGRect, frame: CGRect) -> bool { frame.is_within(ACK_TOLERANCE, target) }

#[derive(Debug, Default)]
pub(crate) struct Reconciler {
    /// Windows with something outstanding. A window not in here is [`Sync::Settled`].
    unsettled: HashMap<WindowId, Sync>,
    /// Every write sent, with its deadline, in the order sent. A deadline is the send time
    /// plus [`WRITE_DEADLINE`], so this is also deadline order. An entry whose window has since
    /// been answered or written again no longer matches its sync state and is dropped when it
    /// reaches the front.
    deadlines: VecDeque<(Instant, WindowId)>,
    pub(crate) clock: Clock,
}

impl Reconciler {
    pub(crate) fn sync(&self, window: WindowId) -> Sync {
        self.unsettled.get(&window).copied().unwrap_or(Sync::Settled)
    }

    fn set(&mut self, window: WindowId, sync: Sync) {
        match sync {
            Sync::Settled => _ = self.unsettled.remove(&window),
            Sync::InFlight { .. } | Sync::Refused { .. } | Sync::Adrift => {
                _ = self.unsettled.insert(window, sync)
            }
        }
    }

    fn send(&mut self, window: WindowId, frame: CGRect, queued: Option<CGRect>) -> Write {
        let deadline = self.clock.now() + WRITE_DEADLINE;
        debug!(?window, ?frame, "reconcile: write");
        self.deadlines.push_back((deadline, window));
        self.set(window, Sync::InFlight {
            target: frame,
            deadline,
            queued,
        });
        Write { window, frame }
    }

    /// Rift wants `window` at `desired`; the model last saw it at `known`.
    pub(crate) fn want(
        &mut self,
        window: WindowId,
        desired: CGRect,
        known: CGRect,
        want: Want,
        owner: Owner,
    ) -> Verdict {
        let mut sync = self.sync(window);
        // A write nobody answered in time no longer holds newer desires back. Whatever was
        // queued behind it is older than `desired`, so it is dropped.
        if let Sync::InFlight { target, deadline, queued: _ } = sync
            && self.clock.now() >= deadline
        {
            sync = self.time_out(window, target, known);
        }
        match owner {
            Owner::Rift => {}
            Owner::User => {
                debug!(?window, frame = ?desired, "reconcile: discarded (user's)");
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
                if desired.same_as(target) {
                    if queued.is_some() {
                        self.set(window, Sync::InFlight { target, deadline, queued: None });
                    }
                    return Verdict::Nothing;
                }
                debug!(?window, frame = ?desired, in_flight = ?target, "reconcile: queued");
                let queued = Some(desired);
                self.set(window, Sync::InFlight { target, deadline, queued });
                Verdict::Queued
            }
            // Asking again for what was refused would be a retry loop. A window that has
            // since been seen somewhere else is no longer the one that refused.
            (Sync::Refused { wanted, got }, Want::Arrange | Want::ArrangePositions)
                if desired.same_as(wanted) && known.same_as(got) =>
            {
                Verdict::Nothing
            }
            (Sync::Settled, Want::Arrange | Want::ArrangePositions) if desired.same_as(known) => {
                Verdict::Nothing
            }
            (Sync::Refused { .. }, Want::Arrange | Want::ArrangePositions)
                if desired.same_as(known) =>
            {
                self.set(window, Sync::Settled);
                Verdict::Nothing
            }
            (
                Sync::Settled | Sync::Refused { .. } | Sync::Adrift,
                Want::Arrange | Want::ArrangePositions | Want::Place,
            ) => Verdict::Send(self.send(window, desired, None)),
        }
    }

    /// The interactive frame of the window the user is dragging with the modifier held: the
    /// drag's own desire, and the only write a dragged window gets. It is not tracked: the
    /// next one replaces it in the app's queue, and the drop arranges the window afresh.
    pub(crate) fn follow(&mut self, window: WindowId, session: &Session) -> Option<Write> {
        let frame = session.interactive_frame()?;
        self.set(window, Sync::Adrift);
        Some(Write { window, frame })
    }

    /// What waited behind a write that has been answered or given up on: sent if the window is
    /// Rift's. If it is the user's now, the desire is discarded; the model already takes the
    /// window to be at that frame, so it is `Adrift` and the next desire is sent regardless.
    fn release(&mut self, window: WindowId, queued: Option<CGRect>, owner: Owner) -> Option<Write> {
        match (owner, queued) {
            (Owner::Rift, Some(next)) => Some(self.send(window, next, None)),
            (Owner::Rift | Owner::User, None) => None,
            (Owner::User, Some(frame)) => {
                debug!(?window, ?frame, "reconcile: discarded (user's)");
                self.set(window, Sync::Adrift);
                None
            }
        }
    }

    /// The app answered its latest write to `window` with `frame`. Returns the queued write,
    /// if one was waiting behind it and the window is still Rift's.
    pub(crate) fn answer(
        &mut self,
        window: WindowId,
        frame: CGRect,
        owner: Owner,
    ) -> (Answer, Option<Write>) {
        match self.sync(window) {
            Sync::Settled | Sync::Adrift => (Answer::Unasked, None),
            Sync::InFlight { target, deadline: _, queued } => {
                let answer = if answers(target, frame) {
                    debug!(?window, ?target, "reconcile: ack");
                    self.set(window, Sync::Settled);
                    Answer::Acked
                } else {
                    debug!(?window, wanted = ?target, got = ?frame, "reconcile: refused");
                    self.set(window, Sync::Refused { wanted: target, got: frame });
                    Answer::Refused
                };
                (answer, self.release(window, queued, owner))
            }
            Sync::Refused { wanted, got: _ } => {
                // The answer Rift stopped waiting for, or a second look at a refusal.
                if answers(wanted, frame) {
                    debug!(?window, target = ?wanted, "reconcile: ack (late)");
                    self.set(window, Sync::Settled);
                } else {
                    self.set(window, Sync::Refused { wanted, got: frame });
                }
                (Answer::Unasked, None)
            }
        }
    }

    /// `window` reported `frame` unprompted; the model last saw it at `known`. A report of the
    /// frame in flight is no news, and does not end the wait: the app's answer does, and it can
    /// still differ.
    pub(crate) fn observe(
        &mut self,
        window: WindowId,
        frame: CGRect,
        known: CGRect,
        holder: Holder,
    ) -> Observation {
        match (self.sync(window), holder) {
            (Sync::InFlight { target, .. }, Holder::User | Holder::Nobody)
                if answers(target, frame) =>
            {
                Observation::Ack
            }
            (Sync::InFlight { .. }, Holder::Nobody) => Observation::InTransit,
            (
                Sync::InFlight { .. } | Sync::Settled | Sync::Refused { .. } | Sync::Adrift,
                Holder::User | Holder::Nobody,
            ) => {
                if frame.same_as(known) {
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
    fn time_out(&mut self, window: WindowId, target: CGRect, known: CGRect) -> Sync {
        debug!(?window, wanted = ?target, "reconcile: timeout");
        let refused = Sync::Refused { wanted: target, got: known };
        self.set(window, refused);
        refused
    }

    /// The write a `deadlines` entry stands for, if it is still waiting: its target and what
    /// was queued behind it.
    fn waiting(&self, (deadline, window): (Instant, WindowId)) -> Option<(CGRect, Option<CGRect>)> {
        match self.sync(window) {
            Sync::InFlight { target, deadline: due, queued } if due == deadline => {
                Some((target, queued))
            }
            Sync::InFlight { .. } | Sync::Settled | Sync::Refused { .. } | Sync::Adrift => None,
        }
    }

    /// Stops waiting for every write whose deadline has passed. `known` is where the model last
    /// saw a window, and whose it is; a window it no longer knows is forgotten. Returns the
    /// writes that were queued behind the late ones.
    pub(crate) fn expire(
        &mut self,
        known: impl Fn(WindowId) -> Option<(CGRect, Owner)>,
    ) -> Vec<Write> {
        let now = self.clock.now();
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
                    released.extend(self.release(window, queued, owner));
                }
                None => self.forget(window),
            }
        }
        released
    }

    /// When the earliest unanswered write stops being waited for.
    pub(crate) fn next_deadline(&mut self) -> Option<Instant> {
        while let Some(&entry) = self.deadlines.front() {
            if self.waiting(entry).is_some() {
                return Some(entry.0);
            }
            self.deadlines.pop_front();
        }
        None
    }

    /// Whether a write to `window` is with its app.
    pub(crate) fn in_flight(&self, window: WindowId) -> bool {
        match self.sync(window) {
            Sync::InFlight { .. } => true,
            Sync::Settled | Sync::Refused { .. } | Sync::Adrift => false,
        }
    }

    /// `window` is gone, or something outside the reconciler (an animation) now owns its
    /// frame: nothing is outstanding.
    pub(crate) fn forget(&mut self, window: WindowId) { self.set(window, Sync::Settled); }
}

/// One app's share of an arrange pass: its windows that only move, and those that also resize,
/// and the targets to record under the batch's transaction id.
#[derive(Default)]
struct Batch {
    positions: Vec<Write>,
    frames: Vec<Write>,
    targets: Vec<(WindowServerId, CGRect)>,
}

fn frames_of(writes: Vec<Write>) -> Vec<(WindowId, CGRect)> {
    writes.into_iter().map(|write| (write.window, write.frame)).collect()
}

/// Whose `window`'s frame is: the user's while their hand is on it, and every window's while
/// they drag one.
fn owner(windows: &WindowStore, window: WindowId) -> Owner {
    match (windows.pointer(), windows.is_gripped(window)) {
        (Pointer::Dragging, _) | (Pointer::Pressing | Pointer::Idle, true) => Owner::User,
        (Pointer::Pressing | Pointer::Idle, false) => Owner::Rift,
    }
}

/// The senders. Each takes a [`Write`], so each sends only what the reconciler decided.
impl Reactor {
    pub(super) fn owner(&self, window: WindowId) -> Owner { owner(&self.state.windows, window) }

    /// Asks the reconciler for `window` at `desired`. A desire it sends or queues is the model's
    /// from then on: the window is taken to be there until a report says otherwise.
    /// A window the model does not know is `Nothing`.
    fn arrange_one(&mut self, window: WindowId, desired: CGRect, want: Want) -> Verdict {
        let owner = self.owner(window);
        let Some(state) = self.state.windows.window_mut(window) else {
            return Verdict::Nothing;
        };
        let verdict = self.reconciler.want(window, desired, state.frame_monotonic, want, owner);
        match verdict {
            Verdict::Send(_) | Verdict::Queued => state.frame_monotonic = desired,
            Verdict::Nothing => {}
        }
        verdict
    }

    /// The transaction id the app protocol wants on a write, recorded with its target.
    fn stamp(&self, window: WindowId, frame: CGRect) -> TransactionId {
        let server_id = self.state.windows.window(window).and_then(|state| state.info.sys_id);
        self.transaction_manager.stamp(server_id.map(|server_id| (server_id, frame)))
    }

    fn send_one(&mut self, write: Write, handle: AppThreadHandle) {
        let transaction = self.stamp(write.window, write.frame);
        if let Err(error) = handle.send(Request::set_window_frame(
            write.window,
            write.frame,
            transaction,
            true,
        )) {
            warn!(window = ?write.window, %error, "failed to write window frame");
            self.reconciler.forget(write.window);
        }
    }

    /// Whether a frame may be written to `window` at all: not while the display topology is
    /// being re-read, and not to a hidden, minimized or native-fullscreen window.
    fn may_write(&self, window: WindowId) -> bool {
        !self.refreshes_blocked() && self.state.windows.can_place_window(window)
    }

    /// Rift places `window` at `frame` (a transfer, a cancelled drag's way back, an app rule):
    /// sent whatever the model believes, unless a write is with the app, which it then follows.
    pub(super) fn place_window(&mut self, window: WindowId, frame: CGRect) {
        if !self.may_write(window) {
            return;
        }
        let Some(app) = self.app_manager.apps.get(&window.pid) else {
            return;
        };
        let handle = app.handle.clone();
        match self.arrange_one(window, frame, Want::Place) {
            Verdict::Send(write) => self.send_one(write, handle),
            Verdict::Queued | Verdict::Nothing => {}
        }
    }

    /// Whether a write to `window` is held to `space`'s scrolling bounds (when its layout has
    /// them): a tiled window of the space's active workspace.
    fn in_scrolling_bounds(&self, window: WindowId, space: SpaceId) -> bool {
        self.state.windows.workspace_for_window(window)
            == self.layout_manager.layout_engine.workspaces().active_workspace(space)
            && !self.layout_manager.layout_engine.is_window_floating(window)
    }

    /// Sends a write layout asked for: the model takes the window to be there from now on.
    pub(super) fn send_arranged(&mut self, write: Write) {
        let window = write.window;
        let Some(app) = self.app_manager.apps.get(&window.pid) else {
            self.reconciler.forget(window);
            return;
        };
        let bounds = self
            .assigned_space_for_window_id(window)
            .filter(|&space| self.in_scrolling_bounds(window, space))
            .and_then(|space| scrolling_bounds(self, space));
        let handle = app.handle.with_scrolling_bounds(bounds);
        if let Some(state) = self.state.windows.window_mut(window) {
            state.frame_monotonic = write.frame;
        }
        self.send_one(write, handle);
    }

    /// Sends the interactive frame of the window the user is dragging.
    pub(super) fn send_interactive(&mut self, write: Write) {
        if !self.may_write(write.window) {
            return;
        }
        let transaction = self.stamp(write.window, write.frame);
        if let Some(app) = self.app_manager.apps.get(&write.window.pid) {
            app.handle.send_interactive_frame(
                write.window,
                write.frame,
                false,
                transaction,
                FrameSource::Drag,
            );
        }
    }

    /// Stops waiting for writes whose apps did not answer in time, and sends what waited
    /// behind them.
    pub(super) fn expire_writes(&mut self) {
        let windows = &self.state.windows;
        let queued = self.reconciler.expire(|window| {
            Some((windows.window(window)?.frame_monotonic, owner(windows, window)))
        });
        for write in queued {
            self.send_arranged(write);
        }
    }

    /// Sends one app's share of an arrange pass, under one transaction id as its protocol
    /// expects: a position-only request and a full one, as needed.
    fn send_batch(
        &mut self,
        pid: pid_t,
        bounds: Option<ScrollingDisplayBounds>,
        Batch { positions, frames, targets }: Batch,
    ) {
        let Some(app) = self.app_manager.apps.get(&pid) else {
            debug!(?pid, "Skipping layout update for app - app no longer exists");
            for write in positions.iter().chain(&frames) {
                self.reconciler.forget(write.window);
            }
            return;
        };
        let handle = app.handle.with_scrolling_bounds(bounds);
        let transaction = self.transaction_manager.stamp(targets);
        let requests = [
            (!positions.is_empty()).then(|| (frames_of(positions), FrameMode::Position)),
            (!frames.is_empty()).then(|| (frames_of(frames), FrameMode::Full)),
        ];
        for (frames, mode) in requests.into_iter().flatten() {
            let request =
                Request::SetWindowFrames(frames, transaction, mode, true, Default::default());
            if let Err(error) = handle.send(request) {
                debug!(?pid, ?error, "Failed to send layout request - app may have quit");
                break;
            }
        }
    }

    /// Reconciles one space's frames with its apps, one request per app and kind of change.
    /// Returns whether any desire was new.
    pub(super) fn arrange_frames(
        &mut self,
        space: SpaceId,
        frames: &[(WindowId, CGRect)],
        want: Want,
    ) -> bool {
        let bounds = scrolling_bounds(self, space);
        let mut per_app: HashMap<(pid_t, bool), Batch> = HashMap::default();
        let mut any_desire_new = false;

        for &(wid, target) in frames {
            let constrained = bounds.is_some() && self.in_scrolling_bounds(wid, space);
            let Some(window) = self.state.windows.window(wid) else {
                debug!(?wid, "Skipping layout - window no longer exists");
                continue;
            };
            let (current, server_id) = (window.frame_monotonic, window.info.sys_id);
            let write = match self.arrange_one(wid, target, want) {
                Verdict::Send(write) => write,
                Verdict::Queued => {
                    any_desire_new = true;
                    continue;
                }
                Verdict::Nothing => continue,
            };
            any_desire_new = true;
            let moves_only = match want {
                Want::ArrangePositions => current.size.same_as(target.size),
                Want::Arrange | Want::Place => false,
            };
            let batch = per_app.entry((wid.pid, constrained)).or_default();
            batch.targets.extend(server_id.map(|server_id| (server_id, target)));
            if moves_only {
                batch.positions.push(write);
            } else {
                batch.frames.push(write);
            }
        }

        for ((pid, constrained), batch) in per_app {
            self.send_batch(pid, bounds.filter(|_| constrained), batch);
        }

        any_desire_new
    }
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::{CGPoint, CGSize};

    use super::*;

    fn at(x: f64) -> CGRect { CGRect::new(CGPoint::new(x, 0.), CGSize::new(400., 300.)) }

    fn window() -> WindowId { WindowId::new(1, 1) }

    fn sent(verdict: Verdict) -> CGRect {
        match verdict {
            Verdict::Send(write) => write.frame,
            Verdict::Queued | Verdict::Nothing => panic!("expected a write, got {verdict:?}"),
        }
    }

    #[test]
    fn a_window_already_where_it_should_be_is_not_written() {
        let mut reconciler = Reconciler::default();
        assert_eq!(
            reconciler.want(window(), at(10.), at(10.), Want::Arrange, Owner::Rift),
            Verdict::Nothing
        );
        // A placement is sent whatever the model believes.
        assert_eq!(
            sent(reconciler.want(window(), at(10.), at(10.), Want::Place, Owner::Rift)),
            at(10.)
        );
    }

    #[test]
    fn one_write_is_in_flight_and_only_the_latest_desire_waits_behind_it() {
        let mut reconciler = Reconciler::default();
        assert_eq!(
            sent(reconciler.want(window(), at(10.), at(0.), Want::Arrange, Owner::Rift)),
            at(10.)
        );
        // The same desire again: nothing. Newer ones: queued, the latest replacing the last.
        assert_eq!(
            reconciler.want(window(), at(10.), at(10.), Want::Arrange, Owner::Rift),
            Verdict::Nothing
        );
        assert_eq!(
            reconciler.want(window(), at(20.), at(10.), Want::Arrange, Owner::Rift),
            Verdict::Queued
        );
        assert_eq!(
            reconciler.want(window(), at(30.), at(20.), Want::Place, Owner::Rift),
            Verdict::Queued
        );

        let (answer, next) = reconciler.answer(window(), at(10.5), Owner::Rift);
        assert_eq!(answer, Answer::Acked);
        assert_eq!(next.map(|write| write.frame), Some(at(30.)));
        assert!(reconciler.in_flight(window()));

        assert_eq!(
            reconciler.answer(window(), at(30.), Owner::Rift),
            (Answer::Acked, None)
        );
        assert_eq!(reconciler.sync(window()), Sync::Settled);
    }

    #[test]
    fn a_desire_withdrawn_while_queued_is_not_sent() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(window(), at(10.), at(0.), Want::Arrange, Owner::Rift);
        assert_eq!(
            reconciler.want(window(), at(20.), at(10.), Want::Arrange, Owner::Rift),
            Verdict::Queued
        );
        // Layout changes its mind back to what is in flight.
        assert_eq!(
            reconciler.want(window(), at(10.), at(20.), Want::Arrange, Owner::Rift),
            Verdict::Nothing
        );
        assert_eq!(
            reconciler.answer(window(), at(10.), Owner::Rift),
            (Answer::Acked, None)
        );
    }

    #[test]
    fn an_adjusted_answer_is_a_refusal_that_is_not_asked_again() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(window(), at(10.), at(0.), Want::Arrange, Owner::Rift);
        assert_eq!(
            reconciler.answer(window(), at(14.), Owner::Rift),
            (Answer::Refused, None)
        );
        assert_eq!(reconciler.sync(window()), Sync::Refused {
            wanted: at(10.),
            got: at(14.)
        });
        // Layout keeps wanting the same frame; the window stays where the app put it.
        for _ in 0..3 {
            assert_eq!(
                reconciler.want(window(), at(10.), at(14.), Want::Arrange, Owner::Rift),
                Verdict::Nothing
            );
        }
        // A different desire is a new question.
        assert_eq!(
            sent(reconciler.want(window(), at(50.), at(14.), Want::Arrange, Owner::Rift)),
            at(50.)
        );
    }

    #[test]
    fn a_refused_window_seen_somewhere_else_is_asked_again() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(window(), at(10.), at(0.), Want::Arrange, Owner::Rift);
        _ = reconciler.answer(window(), at(14.), Owner::Rift);
        // Its app moved it: no longer the frame that was refused.
        assert_eq!(
            reconciler.observe(window(), at(90.), at(14.), Holder::Nobody),
            Observation::Foreign
        );
        assert_eq!(
            sent(reconciler.want(window(), at(10.), at(90.), Want::Arrange, Owner::Rift)),
            at(10.)
        );
    }

    #[test]
    fn an_unanswered_write_times_out_into_a_refusal_and_releases_what_waited() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(window(), at(10.), at(0.), Want::Arrange, Owner::Rift);
        _ = reconciler.want(window(), at(20.), at(10.), Want::Arrange, Owner::Rift);
        assert!(
            reconciler.expire(|_| Some((at(0.), Owner::Rift))).is_empty(),
            "the deadline has not passed"
        );

        reconciler.clock.advance(WRITE_DEADLINE);
        let released = reconciler.expire(|_| Some((at(0.), Owner::Rift)));
        assert_eq!(released.iter().map(|write| write.frame).collect::<Vec<_>>(), [
            at(20.)
        ]);

        // Without anything queued, the late write is a refusal, and its answer, when it does
        // come, settles it.
        reconciler.clock.advance(WRITE_DEADLINE);
        assert!(reconciler.expire(|_| Some((at(0.), Owner::Rift))).is_empty());
        assert_eq!(reconciler.sync(window()), Sync::Refused {
            wanted: at(20.),
            got: at(0.)
        });
        assert_eq!(
            reconciler.answer(window(), at(20.), Owner::Rift),
            (Answer::Unasked, None)
        );
        assert_eq!(reconciler.sync(window()), Sync::Settled);
    }

    #[test]
    fn a_desire_after_the_deadline_is_sent_not_held_behind_what_was_queued() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(window(), at(10.), at(0.), Want::Arrange, Owner::Rift);
        _ = reconciler.want(window(), at(20.), at(0.), Want::Arrange, Owner::Rift);
        reconciler.clock.advance(WRITE_DEADLINE);
        // The app never answered; the queued frame is older than this desire.
        assert_eq!(
            sent(reconciler.want(window(), at(30.), at(0.), Want::Arrange, Owner::Rift)),
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
        _ = reconciler.want(window(), at(10.), at(0.), Want::Arrange, Owner::Rift);
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
            reconciler.answer(window(), at(10.), Owner::Rift),
            (Answer::Unasked, None)
        );
        // After the drop the window is written even where the model believes it already is.
        assert_eq!(
            sent(reconciler.want(window(), at(70.), at(70.), Want::Arrange, Owner::Rift)),
            at(70.)
        );
    }

    #[test]
    fn a_report_of_the_target_is_no_news_and_the_answer_still_decides() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(window(), at(10.), at(0.), Want::Arrange, Owner::Rift);
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
            reconciler.answer(window(), at(13.), Owner::Rift),
            (Answer::Refused, None)
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
        let start = reconciler.clock.now();
        _ = reconciler.want(window(), at(10.), at(0.), Want::Arrange, Owner::Rift);
        reconciler.clock.advance(Duration::from_millis(100));
        _ = reconciler.want(WindowId::new(1, 2), at(10.), at(0.), Want::Arrange, Owner::Rift);
        assert_eq!(reconciler.next_deadline(), Some(start + WRITE_DEADLINE));
        _ = reconciler.answer(window(), at(10.), Owner::Rift);
        assert_eq!(
            reconciler.next_deadline(),
            Some(start + Duration::from_millis(100) + WRITE_DEADLINE)
        );
    }

    #[test]
    fn forgetting_a_window_drops_what_was_outstanding() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(window(), at(10.), at(0.), Want::Arrange, Owner::Rift);
        // The window is gone from the model by the time its write's deadline passes.
        reconciler.clock.advance(WRITE_DEADLINE);
        assert!(reconciler.expire(|_| None).is_empty());
        assert_eq!(reconciler.sync(window()), Sync::Settled);
    }
}
