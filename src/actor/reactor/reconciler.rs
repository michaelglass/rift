//! Desired frames, and the one place that decides to write them.
//!
//! Layout says where each window should be ([`Desired`]). The [`Reconciler`] compares that with
//! what each window's app has acknowledged and mints a [`Write`] only where they differ. A
//! `Write` cannot be built anywhere else, and the reactor's senders take nothing but a `Write`,
//! so a frame reaches an app only because the reconciler decided it should.

use std::time::{Duration, Instant};

use objc2_core_foundation::CGRect;
use tracing::{debug, warn};

use super::animation::scrolling_bounds;
use super::{Reactor, TransactionId};
use crate::actor::app::{AppThreadHandle, FrameMode, FrameSource, Request, WindowId, pid_t};
use crate::actor::drag::Session;
use crate::common::collections::HashMap;
use crate::model::window_store::WindowStore;
use crate::sys::geometry::{IsWithin, Round, SameAs};
use crate::sys::screen::SpaceId;
use crate::sys::window_server::WindowServerId;

/// How far an app's answer may be from the frame Rift asked for and still be that frame
/// (yabai's `AX_DIFF`).
const ACK_TOLERANCE: f64 = 1.5;

/// How long an app has to answer a write before Rift stops waiting for it.
const WRITE_DEADLINE: Duration = Duration::from_millis(300);

/// Where Rift wants the windows of one space: layout as data. The window the user is dragging
/// is theirs to place until the drop, so it has no desired frame and is not in here.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Desired(Vec<(WindowId, CGRect)>);

impl Desired {
    pub(crate) fn of(layout: &[(WindowId, CGRect)], windows: &WindowStore) -> Self {
        let dragged = windows.drag().map(|(window, _)| window);
        Self(
            layout
                .iter()
                .filter(|(wid, _)| Some(*wid) != dragged)
                .map(|&(wid, frame)| (wid, frame.round()))
                .collect(),
        )
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
}

/// How strongly Rift wants a frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Want {
    /// Layout: nothing to send when the window is already known to be there.
    Arrange,
    /// A placement, or a layout after the cached frames stopped being trustworthy: send it
    /// whatever the model believes.
    Place,
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

/// The app's answer to a write, classified against the write it answers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Answer {
    Acked,
    Refused {
        wanted: CGRect,
        got: CGRect,
    },
    /// Nothing tracked was waiting for it: the answer to a write Rift had stopped waiting for,
    /// or to one sent outside the reconciler (an animation frame, a drag's interactive frame).
    Unasked,
}

fn answers(target: CGRect, frame: CGRect) -> bool { frame.is_within(ACK_TOLERANCE, target) }

#[derive(Debug, Default)]
pub(crate) struct Reconciler {
    /// Windows with something outstanding. A window not in here is [`Sync::Settled`].
    unsettled: HashMap<WindowId, Sync>,
    pub(crate) clock: Clock,
}

impl Reconciler {
    pub(crate) fn sync(&self, window: WindowId) -> Sync {
        self.unsettled.get(&window).copied().unwrap_or(Sync::Settled)
    }

    fn set(&mut self, window: WindowId, sync: Sync) {
        match sync {
            Sync::Settled => _ = self.unsettled.remove(&window),
            Sync::InFlight { .. } | Sync::Refused { .. } => _ = self.unsettled.insert(window, sync),
        }
    }

    fn send(&mut self, window: WindowId, frame: CGRect, queued: Option<CGRect>) -> Write {
        let deadline = self.clock.now() + WRITE_DEADLINE;
        debug!(?window, ?frame, "reconcile: write");
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
    ) -> Verdict {
        // A write nobody answered in time no longer holds newer desires back. Whatever was
        // queued behind it is older than `desired`.
        _ = self.expire_one(window, known);
        match (self.sync(window), want) {
            (Sync::InFlight { target, deadline, queued: _ }, Want::Arrange | Want::Place) => {
                if desired.same_as(target) {
                    self.set(window, Sync::InFlight { target, deadline, queued: None });
                    return Verdict::Nothing;
                }
                debug!(?window, frame = ?desired, in_flight = ?target, "reconcile: queued");
                let queued = Some(desired);
                self.set(window, Sync::InFlight { target, deadline, queued });
                Verdict::Queued
            }
            // Asking again for what was refused would be a retry loop. A window that has
            // since been seen somewhere else is no longer the one that refused.
            (Sync::Refused { wanted, got }, Want::Arrange)
                if desired.same_as(wanted) && known.same_as(got) =>
            {
                Verdict::Nothing
            }
            (Sync::Settled | Sync::Refused { .. }, Want::Arrange) if desired.same_as(known) => {
                self.set(window, Sync::Settled);
                Verdict::Nothing
            }
            (Sync::Settled | Sync::Refused { .. }, Want::Arrange | Want::Place) => {
                Verdict::Send(self.send(window, desired, None))
            }
        }
    }

    /// The interactive frame of the window the user is dragging with the modifier held: the
    /// drag's own desire, and the only write a dragged window gets. It is not tracked: the
    /// next one replaces it in the app's queue, and the drop arranges the window afresh.
    pub(crate) fn follow(&mut self, window: WindowId, session: &Session) -> Option<Write> {
        let frame = session.interactive_frame()?;
        self.set(window, Sync::Settled);
        Some(Write { window, frame })
    }

    /// The app answered its latest write to `window` with `frame`. Returns the queued write,
    /// if one was waiting behind it.
    pub(crate) fn answer(&mut self, window: WindowId, frame: CGRect) -> (Answer, Option<Write>) {
        match self.sync(window) {
            Sync::Settled => (Answer::Unasked, None),
            Sync::InFlight { target, deadline: _, queued } => {
                let answer = if answers(target, frame) {
                    debug!(?window, ?target, "reconcile: ack");
                    self.set(window, Sync::Settled);
                    Answer::Acked
                } else {
                    debug!(?window, wanted = ?target, got = ?frame, "reconcile: refused");
                    self.set(window, Sync::Refused { wanted: target, got: frame });
                    Answer::Refused { wanted: target, got: frame }
                };
                (answer, queued.map(|next| self.send(window, next, None)))
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

    fn expire_one(&mut self, window: WindowId, known: CGRect) -> Option<Write> {
        let Sync::InFlight { target, deadline, queued } = self.sync(window) else {
            return None;
        };
        if self.clock.now() < deadline {
            return None;
        }
        debug!(?window, wanted = ?target, "reconcile: timeout");
        self.set(window, Sync::Refused { wanted: target, got: known });
        queued.map(|next| self.send(window, next, None))
    }

    /// Stops waiting for every write whose deadline has passed, and forgets windows that are
    /// gone. `known` is where the model last saw a window. Returns the writes that were queued
    /// behind the late ones.
    pub(crate) fn expire(&mut self, known: impl Fn(WindowId) -> Option<CGRect>) -> Vec<Write> {
        let unsettled: Vec<WindowId> = self.unsettled.keys().copied().collect();
        unsettled
            .into_iter()
            .filter_map(|window| match known(window) {
                Some(known) => self.expire_one(window, known),
                None => {
                    self.forget(window);
                    None
                }
            })
            .collect()
    }

    /// Whether a write to `window` is with its app.
    pub(crate) fn in_flight(&self, window: WindowId) -> bool {
        match self.sync(window) {
            Sync::InFlight { .. } => true,
            Sync::Settled | Sync::Refused { .. } => false,
        }
    }

    /// `window` is gone, or something outside the reconciler (an animation) now owns its
    /// frame: nothing is outstanding.
    pub(crate) fn forget(&mut self, window: WindowId) { self.set(window, Sync::Settled); }
}

/// One app's share of an arrange pass: its windows that only move, and those that also resize.
#[derive(Default)]
struct Batch {
    positions: Vec<Write>,
    frames: Vec<Write>,
    first: Option<WindowServerId>,
}

fn frames_of(writes: Vec<Write>) -> Vec<(WindowId, CGRect)> {
    writes.into_iter().map(|write| (write.window, write.frame)).collect()
}

/// The senders. Each takes a [`Write`], so each sends only what the reconciler decided.
impl Reactor {
    /// Where the model last saw `window`.
    fn known_frame(&self, window: WindowId) -> Option<CGRect> {
        self.state.windows.window(window).map(|state| state.frame_monotonic)
    }

    /// The transaction id the app protocol wants on a write, recorded with its target.
    fn stamp(&self, window: WindowId, frame: CGRect) -> TransactionId {
        let server_id = self.state.windows.window(window).and_then(|state| state.info.sys_id);
        server_id
            .map(|server_id| {
                let transaction = self.transaction_manager.generate_next_txid(server_id);
                self.transaction_manager.store_txid(server_id, transaction, frame);
                transaction
            })
            .unwrap_or_default()
    }

    fn send_one(&mut self, write: Write, handle: AppThreadHandle, eui: bool) {
        let transaction = self.stamp(write.window, write.frame);
        if let Err(error) = handle.send(Request::set_window_frame(
            write.window,
            write.frame,
            transaction,
            eui,
        )) {
            warn!(window = ?write.window, %error, "failed to write window frame");
            self.reconciler.forget(write.window);
        }
    }

    /// Rift places `window` at `frame` (a transfer, a cancelled drag's way back, an app rule):
    /// sent whatever the model believes, unless a write is with the app, which it then follows.
    pub(super) fn place_window(&mut self, window: WindowId, frame: CGRect, eui: bool) {
        let Some(known) = self.known_frame(window) else { return };
        let Some(app) = self.app_manager.apps.get(&window.pid) else {
            return;
        };
        let handle = app.handle.clone();
        match self.reconciler.want(window, frame, known, Want::Place) {
            Verdict::Send(write) => self.send_one(write, handle, eui),
            Verdict::Queued | Verdict::Nothing => {}
        }
    }

    /// Sends a write layout asked for: the model takes the window to be there from now on.
    pub(super) fn send_arranged(&mut self, write: Write) {
        let window = write.window;
        let Some(app) = self.app_manager.apps.get(&window.pid) else {
            self.reconciler.forget(window);
            return;
        };
        let space = self.assigned_space_for_window_id(window);
        let bounds = space
            .filter(|space| {
                self.state.windows.workspace_for_window(window)
                    == self.layout_manager.layout_engine.workspaces().active_workspace(*space)
                    && !self.layout_manager.layout_engine.is_window_floating(window)
            })
            .and_then(|space| scrolling_bounds(self, space));
        let handle = app.handle.with_scrolling_bounds(bounds);
        if let Some(state) = self.state.windows.window_mut(window) {
            state.frame_monotonic = write.frame;
        }
        self.send_one(write, handle, true);
    }

    /// Sends the interactive frame of the window the user is dragging.
    pub(super) fn send_interactive(&mut self, write: Write) {
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
        let queued = self
            .reconciler
            .expire(|window| windows.window(window).map(|state| state.frame_monotonic));
        for write in queued {
            self.send_arranged(write);
        }
    }

    /// Reconciles one space's frames with its apps, one request per app and kind of change.
    /// Returns whether any desire was new.
    pub(super) fn arrange_frames(
        &mut self,
        space: SpaceId,
        frames: &[(WindowId, CGRect)],
        position_only: bool,
        want: Want,
    ) -> bool {
        let bounds = scrolling_bounds(self, space);
        let active = self.layout_manager.layout_engine.workspaces().active_workspace(space);
        let mut per_app: HashMap<(pid_t, bool), Batch> = HashMap::default();
        let mut any_desire_new = false;

        for &(wid, target) in frames {
            let constrained = bounds.is_some()
                && self.state.windows.workspace_for_window(wid) == active
                && !self.layout_manager.layout_engine.is_window_floating(wid);
            let Some(window) = self.state.windows.window_mut(wid) else {
                debug!(?wid, "Skipping layout - window no longer exists");
                continue;
            };
            let current = window.frame_monotonic;
            match self.reconciler.want(wid, target, current, want) {
                Verdict::Nothing => {}
                Verdict::Queued => {
                    any_desire_new = true;
                    window.frame_monotonic = target;
                }
                Verdict::Send(write) => {
                    any_desire_new = true;
                    window.frame_monotonic = target;
                    let size_unchanged = match want {
                        Want::Arrange => current.size.same_as(target.size),
                        Want::Place => false,
                    };
                    let batch = per_app.entry((wid.pid, constrained)).or_default();
                    batch.first = batch.first.or(window.info.sys_id);
                    if position_only && size_unchanged {
                        batch.positions.push(write);
                    } else {
                        batch.frames.push(write);
                    }
                }
            }
        }

        for ((pid, constrained), Batch { positions, frames, first }) in per_app {
            let sent = positions.iter().chain(&frames).map(|write| write.window);
            let Some(app) = self.app_manager.apps.get(&pid) else {
                debug!(?pid, "Skipping layout update for app - app no longer exists");
                for window in sent.collect::<Vec<_>>() {
                    self.reconciler.forget(window);
                }
                continue;
            };
            let handle = app.handle.with_scrolling_bounds(bounds.filter(|_| constrained));
            // One transaction id for the app's whole batch, as its protocol expects.
            let transaction = first
                .map(|server_id| self.transaction_manager.generate_next_txid(server_id))
                .unwrap_or_default();
            for write in positions.iter().chain(&frames) {
                if let Some(server_id) =
                    self.state.windows.window(write.window).and_then(|state| state.info.sys_id)
                {
                    self.transaction_manager.store_txid(server_id, transaction, write.frame);
                }
            }
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
            reconciler.want(window(), at(10.), at(10.), Want::Arrange),
            Verdict::Nothing
        );
        // A placement is sent whatever the model believes.
        assert_eq!(
            sent(reconciler.want(window(), at(10.), at(10.), Want::Place)),
            at(10.)
        );
    }

    #[test]
    fn one_write_is_in_flight_and_only_the_latest_desire_waits_behind_it() {
        let mut reconciler = Reconciler::default();
        assert_eq!(
            sent(reconciler.want(window(), at(10.), at(0.), Want::Arrange)),
            at(10.)
        );
        // The same desire again: nothing. Newer ones: queued, the latest replacing the last.
        assert_eq!(
            reconciler.want(window(), at(10.), at(10.), Want::Arrange),
            Verdict::Nothing
        );
        assert_eq!(
            reconciler.want(window(), at(20.), at(10.), Want::Arrange),
            Verdict::Queued
        );
        assert_eq!(
            reconciler.want(window(), at(30.), at(20.), Want::Place),
            Verdict::Queued
        );

        let (answer, next) = reconciler.answer(window(), at(10.5));
        assert_eq!(answer, Answer::Acked);
        assert_eq!(next.map(|write| write.frame), Some(at(30.)));
        assert!(reconciler.in_flight(window()));

        assert_eq!(reconciler.answer(window(), at(30.)), (Answer::Acked, None));
        assert_eq!(reconciler.sync(window()), Sync::Settled);
    }

    #[test]
    fn a_desire_withdrawn_while_queued_is_not_sent() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(window(), at(10.), at(0.), Want::Arrange);
        assert_eq!(
            reconciler.want(window(), at(20.), at(10.), Want::Arrange),
            Verdict::Queued
        );
        // Layout changes its mind back to what is in flight.
        assert_eq!(
            reconciler.want(window(), at(10.), at(20.), Want::Arrange),
            Verdict::Nothing
        );
        assert_eq!(reconciler.answer(window(), at(10.)), (Answer::Acked, None));
    }

    #[test]
    fn an_adjusted_answer_is_a_refusal_that_is_not_asked_again() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(window(), at(10.), at(0.), Want::Arrange);
        let refused = Answer::Refused { wanted: at(10.), got: at(14.) };
        assert_eq!(reconciler.answer(window(), at(14.)), (refused, None));
        // Layout keeps wanting the same frame; the window stays where the app put it.
        for _ in 0..3 {
            assert_eq!(
                reconciler.want(window(), at(10.), at(14.), Want::Arrange),
                Verdict::Nothing
            );
        }
        // A different desire is a new question.
        assert_eq!(
            sent(reconciler.want(window(), at(50.), at(14.), Want::Arrange)),
            at(50.)
        );
    }

    #[test]
    fn an_unanswered_write_times_out_into_a_refusal_and_releases_what_waited() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(window(), at(10.), at(0.), Want::Arrange);
        _ = reconciler.want(window(), at(20.), at(10.), Want::Arrange);
        assert!(
            reconciler.expire(|_| Some(at(0.))).is_empty(),
            "the deadline has not passed"
        );

        reconciler.clock.advance(WRITE_DEADLINE);
        let released = reconciler.expire(|_| Some(at(0.)));
        assert_eq!(released.iter().map(|write| write.frame).collect::<Vec<_>>(), [
            at(20.)
        ]);

        // Without anything queued, the late write is a refusal, and its answer, when it does
        // come, settles it.
        reconciler.clock.advance(WRITE_DEADLINE);
        assert!(reconciler.expire(|_| Some(at(0.))).is_empty());
        assert_eq!(reconciler.sync(window()), Sync::Refused {
            wanted: at(20.),
            got: at(0.)
        });
        assert_eq!(reconciler.answer(window(), at(20.)), (Answer::Unasked, None));
        assert_eq!(reconciler.sync(window()), Sync::Settled);
    }

    #[test]
    fn forgetting_a_window_drops_what_was_outstanding() {
        let mut reconciler = Reconciler::default();
        _ = reconciler.want(window(), at(10.), at(0.), Want::Arrange);
        // The window is gone from the model.
        assert!(reconciler.expire(|_| None).is_empty());
        assert_eq!(reconciler.sync(window()), Sync::Settled);
    }
}
