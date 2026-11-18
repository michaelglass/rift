//! Desired frames, and the reactor's side of the core's reconciler.
//!
//! Layout says where each window should be ([`Desired`]). The core (`rift_core`) compares that
//! with what each window's app has acknowledged and decides the writes; this module is the thin
//! applier between it and the reactor: it turns reactor calls into core [`Event`]s, steps the
//! core, and turns the [`Effect`]s back into what the reactor acts on. A `Write` can only come
//! out of the core, and the senders below take nothing but a `Write`, so a frame reaches an
//! app only because the core decided it should.

#[cfg(test)]
use std::cell::RefCell;
use std::cell::{Cell, RefCell as Shared};
use std::time::{Duration, Instant};

use objc2_core_foundation::CGRect;
use rift_core::{Kinds, Release};
pub(crate) use rift_core::{Observation, Owner, Want};
use tracing::{debug, warn};

use super::animation::scrolling_bounds;
use super::managers::ScrollingDisplayBounds;
use super::{Reactor, TransactionId};
use crate::actor::app::{AppThreadHandle, FrameMode, FrameSource, Request, WindowId, pid_t};
use crate::common::collections::{HashMap, HashSet};
use crate::model::RiftState;
use crate::model::drag::{MouseButton, Session};
use crate::model::window_store::{Pointer, WindowRecord};
use crate::sys::geometry::{IsWithin, Round, SameAs};
use crate::sys::screen::SpaceId;
use crate::sys::window_server::WindowServerId;

/// The core on Rift's own types: no conversion at the boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rift;

impl Kinds for Rift {
    type Frame = CGRect;
    type Request = TransactionId;
    type Time = Instant;
    type Window = WindowId;

    fn is_within(a: CGRect, tolerance: f64, b: CGRect) -> bool { a.is_within(tolerance, b) }

    fn after(time: Instant, duration: Duration) -> Instant { time + duration }

    fn same_size(a: CGRect, b: CGRect) -> bool { a.size.same_as(b.size) }
}

#[cfg(test)]
pub(crate) type Sync = rift_core::Sync<Rift>;
pub(crate) type Write = rift_core::Write<Rift>;
pub(crate) type Verdict = rift_core::Verdict<Rift>;
pub(crate) type Reply = rift_core::Reply<Rift>;
pub(crate) type Event = rift_core::Event<Rift>;
pub(crate) type Effect = rift_core::Effect<Rift>;

/// Where Rift wants the windows of one space: layout as data. A drag does not change the
/// layout, so the window the user holds is in here too, at its slot; the reconciler is what
/// leaves it alone until they let go ([`Owner::User`]).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Desired(Vec<(WindowId, CGRect)>);

impl Desired {
    pub(crate) fn of(mut layout: Vec<(WindowId, CGRect)>) -> Self {
        for (_, frame) in &mut layout {
            *frame = frame.round();
        }
        Self(layout)
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

/// The core the reactor steps.
type Engine = rift_core::RustCore<Rift>;

/// One step through the core, as the reactor took it: what went in and what came out.
#[cfg(test)]
pub(crate) type Step = (Event, Vec<Effect>);

/// The reactor's handle on the core. The model stays inside: the reactor reads nothing of it
/// but what effects say (`armed`), except the two Rust-only reads marked below.
#[derive(Debug)]
pub(crate) struct Reconciler {
    /// Shared so the mode gate, asked from `&self`, can step it too.
    core: Shared<Engine>,
    /// The deadline the core last asked the run loop's timer to be armed for.
    armed: Cell<Option<Instant>>,
    pub(crate) clock: Clock,
    /// When `Some`, every step is recorded (the fuzzer's invariants read why Rift wrote).
    #[cfg(test)]
    pub(crate) trace: RefCell<Option<Vec<Step>>>,
}

impl Default for Reconciler {
    fn default() -> Self {
        let clock = Clock::default();
        let core = Shared::new(Engine::default());
        Reconciler {
            core,
            armed: Cell::new(None),
            clock,
            #[cfg(test)]
            trace: RefCell::default(),
        }
    }
}

impl Reconciler {
    fn step(&self, event: Event) -> Vec<Effect> {
        #[cfg(test)]
        let recorded = self.trace.borrow().is_some().then(|| event.clone());
        let effects = self.core.borrow_mut().step(event);
        for effect in &effects {
            match effect {
                Effect::Timer(at) => self.armed.set(*at),
                effect => debug!(?effect, "reconcile"),
            }
        }
        #[cfg(test)]
        if let (Some(event), Some(trace)) = (recorded, self.trace.borrow_mut().as_mut()) {
            trace.push((event, effects.clone()));
        }
        effects
    }

    fn writes(effects: Vec<Effect>) -> impl Iterator<Item = Write> {
        effects.into_iter().filter_map(|effect| match effect {
            Effect::Write(write) => Some(write),
            _ => None,
        })
    }

    /// Rust-only read, not part of the boundary: tests, and the fuzzer's invariants.
    #[cfg(test)]
    pub(crate) fn sync(&self, window: WindowId) -> Option<Sync> {
        self.core.borrow().reconciler().sync(window)
    }

    /// Rust-only read, not part of the boundary: the frame Rift has pending for `window`.
    #[cfg(test)]
    pub(crate) fn pending(&self, window: WindowId) -> Option<CGRect> {
        self.core.borrow().reconciler().pending(window)
    }

    /// Steps the core; the caller (`RiftState`) applies what it projects onto the store.
    fn decide(&self, event: Event) -> Vec<Effect> { self.step(event) }

    /// Rust-only read, not part of the boundary: where the core sees `window`, which the
    /// window store's `frame_monotonic` must agree with (checked in debug builds).
    pub(crate) fn seen(&self, window: WindowId) -> Option<CGRect> {
        self.core.borrow().seen(window)
    }

    /// Rust-only read, not part of the boundary: the core's grip, which the window store's
    /// slot must agree with (checked after each applied grip step in debug builds).
    fn pointer(&self) -> Pointer { self.core.borrow().pointer() }

    /// When the run loop's timer should fire: the deadline the core last asked for.
    pub(crate) fn next_deadline(&self) -> Option<Instant> { self.armed.get() }
}

fn frames_of(writes: Vec<Write>) -> Vec<(WindowId, CGRect)> {
    writes.into_iter().map(|write| (write.window(), write.frame())).collect()
}

/// Whose windows to re-read when the event queue drains (R5): after a drop, a display or Space
/// change, an app's launch or quit, a wake, or the end of a topology quarantine, what Rift
/// believes about windows may be wrong in ways no report will tell it. Requests coalesce, so a
/// burst of such events costs one read per window.
#[derive(Debug, Default)]
pub(crate) enum Reread {
    #[default]
    Nothing,
    /// Only these apps' windows: a launched app's are the only news.
    Apps(HashSet<pid_t>),
    /// Every app's windows but those of apps that are quitting.
    All { quitting: HashSet<pid_t> },
}

impl Reread {
    pub(crate) fn everything(&mut self) {
        if !matches!(self, Reread::All { .. }) {
            *self = Reread::All { quitting: HashSet::default() };
        }
    }

    /// `pid` launched: its windows are read.
    pub(crate) fn app(&mut self, pid: pid_t) {
        match self {
            Reread::Nothing => *self = Reread::Apps([pid].into_iter().collect()),
            Reread::Apps(pids) => _ = pids.insert(pid),
            Reread::All { quitting } => _ = quitting.remove(&pid),
        }
    }

    /// `pid` is quitting: every other app's windows are read, and none of its own.
    pub(crate) fn everything_but(&mut self, pid: pid_t) {
        self.everything();
        if let Reread::All { quitting } = self {
            quitting.insert(pid);
        }
    }
}

/// The full reconcile.
impl Reactor {
    /// Sends what [`Reread`] gathered, once per drained event queue. Each read comes back as an
    /// unprompted report and is classified like any other: no news, or someone else's move,
    /// which re-derives where the window belongs and writes the difference. A write that timed
    /// out (the app was busy) is asked again once its window is seen where it really is.
    pub(crate) fn reread_windows(&mut self) {
        let reread = std::mem::take(&mut self.reread);
        if matches!(reread, Reread::Nothing) {
            return;
        }
        if self.refreshes_blocked() {
            // Discarded: the topology is in flux, and the snapshot that ends the quarantine
            // asks again.
            debug!(
                ?reread,
                "reconcile: re-read discarded while topology is unstable"
            );
            return;
        }
        debug!(?reread, "reconcile: re-read");
        let wanted = |pid: &pid_t| match &reread {
            Reread::Nothing => false,
            Reread::Apps(pids) => pids.contains(pid),
            Reread::All { quitting } => !quitting.contains(pid),
        };
        for (pid, app) in self.app_manager.apps.iter().filter(|(pid, _)| wanted(pid)) {
            if let Err(error) = app.handle.send(Request::ReadFrames) {
                debug!(pid, %error, "reconcile: app gone before its frames were read");
            }
        }
    }
}

/// The senders. Each takes a [`Write`], so each sends only what the reconciler decided.
impl Reactor {
    pub(super) fn owner(&self, window: WindowId) -> Owner {
        self.state.windows.pointer().owner(window)
    }

    /// Asks the reconciler for `window` at `desired`. A desire it sends or queues is the model's
    /// from then on: the window is taken to be there until a report says otherwise.
    /// A window the model does not know is `Nothing`.
    fn arrange_one(
        &mut self,
        window: WindowId,
        desired: CGRect,
        want: Want,
        request: TransactionId,
    ) -> Verdict {
        self.state.want(request, window, desired, want)
    }

    fn send_one(&mut self, write: Write, handle: AppThreadHandle) {
        if let Err(error) = handle.send(Request::set_window_frame(
            write.window(),
            write.frame(),
            write.request(),
            true,
        )) {
            warn!(window = ?write.window(), %error, "failed to write window frame");
            self.state.forget(write.window());
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
        let request = self.state.tx.next_id();
        match self.arrange_one(window, frame, Want::Arrange, request) {
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
        let window = write.window();
        let Some(app) = self.app_manager.apps.get(&window.pid) else {
            self.state.forget(window);
            return;
        };
        let bounds = self
            .assigned_space_for_window_id(window)
            .filter(|&space| self.in_scrolling_bounds(window, space))
            .and_then(|space| scrolling_bounds(self, space));
        let handle = app.handle.with_scrolling_bounds(bounds);
        self.send_one(write, handle);
    }

    /// Sends the interactive frame of the window the user is dragging.
    pub(super) fn send_interactive(&mut self, write: Write) {
        if !self.may_write(write.window()) {
            return;
        }
        if let Some(app) = self.app_manager.apps.get(&write.window().pid) {
            app.handle.send_interactive_frame(
                write.window(),
                write.frame(),
                false,
                write.request(),
                FrameSource::Drag,
            );
        }
    }

    /// Stops waiting for writes whose apps did not answer in time, and sends what waited
    /// behind them.
    pub(super) fn expire_writes(&mut self) {
        let request = self.state.tx.next_id();
        let queued = self.state.expire(request);
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
        rift_core::Plan { request, positions, frames }: rift_core::Plan<Rift>,
    ) {
        let Some(app) = self.app_manager.apps.get(&pid) else {
            debug!(?pid, "Skipping layout update for app - app no longer exists");
            for write in positions.iter().chain(&frames) {
                self.state.forget(write.window());
            }
            return;
        };
        let handle = app.handle.with_scrolling_bounds(bounds);
        let requests = [
            (!positions.is_empty()).then(|| (frames_of(positions), FrameMode::Position)),
            (!frames.is_empty()).then(|| (frames_of(frames), FrameMode::Full)),
        ];
        for (frames, mode) in requests.into_iter().flatten() {
            let request = Request::SetWindowFrames(frames, request, mode, true, Default::default());
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
        // One request per batch (an app, and whether its writes are held to the scrolling
        // bounds), minted before the core decides: its writes carry it.
        let mut batches: HashMap<(pid_t, bool), TransactionId> = HashMap::default();
        let mut keys: HashMap<TransactionId, (pid_t, bool)> = HashMap::default();
        let desires = frames
            .iter()
            .map(|&(window, frame)| {
                let constrained = bounds.is_some() && self.in_scrolling_bounds(window, space);
                let key = (window.pid, constrained);
                let request = *batches.entry(key).or_insert_with(|| self.state.tx.next_id());
                keys.insert(request, key);
                rift_core::Desire { window, frame, request }
            })
            .collect();
        let (plans, any_desire_new) = self.state.arrange(want, desires);
        for plan in plans {
            let (pid, constrained) = keys[&plan.request];
            self.send_batch(pid, bounds.filter(|_| constrained), plan);
        }
        any_desire_new
    }
}

/// What a grip step let go of: the window, and the drag's session if it was a drag.
pub(crate) struct Ungripped {
    pub(crate) window: WindowId,
    pub(crate) session: Option<Session>,
}

/// What an animated layout pass decided: the animations, and the writes in their place.
#[derive(Default)]
pub(crate) struct Animated {
    pub animations: Vec<(WindowId, CGRect, CGRect, TransactionId)>,
    pub writes: Vec<Write>,
}

/// The grip: the core decides, the window store's slot is its projection. These are the only
/// ways the slot changes, each a core step whose effects are applied here.
impl RiftState {
    fn sent(&self, write: &Write) {
        if let Some(server_id) = self.windows.server_id(write.window()) {
            self.tx.sent(server_id, write.request());
        }
    }

    /// Applies one grip step's effects to the store. `session` is what a `DragBegan` attaches.
    fn project(&mut self, effects: &[Effect], mut session: Option<Session>) -> Option<Ungripped> {
        let mut ungripped = None;
        for effect in effects {
            match *effect {
                Effect::Ungrip { window, why: _ } => {
                    let session = self.windows.project_ungrip(window);
                    ungripped = Some(Ungripped { window, session });
                }
                Effect::Grip { window } => self.windows.project_grip(window),
                Effect::DragBegan { window } => self.windows.project_drag(
                    window,
                    session.take().expect("a drag is asked for with its session"),
                ),
                Effect::Regrip { from, to } => self.windows.project_regrip(from, to),
                // The pending store the app actors read is the core's pending frames.
                Effect::Pending { window, target } => {
                    if let Some(server_id) = self.windows.server_id(window) {
                        self.tx.project_target(server_id, target);
                    }
                }
                Effect::Unpending { window } => {
                    if let Some(server_id) = self.windows.server_id(window) {
                        self.tx.clear_target(&server_id);
                    }
                }
                // The latest request sent each window, which the app actors read.
                Effect::Write(ref write) => self.sent(write),
                Effect::Plan(ref plan) => {
                    for write in plan.positions.iter().chain(&plan.frames) {
                        self.sent(write);
                    }
                }
                Effect::Animate { window, request, .. } => {
                    if let Some(server_id) = self.windows.server_id(window) {
                        self.tx.sent(server_id, request);
                    }
                }
                Effect::Seen { window, frame } => {
                    let state = self.windows.window_mut(window);
                    debug_assert!(state.is_some(), "the core sees a window the store lacks");
                    if let Some(state) = state {
                        state.frame_monotonic = frame;
                    }
                }
                _ => {}
            }
        }
        debug_assert_eq!(
            self.windows.pointer(),
            self.core.pointer(),
            "store and core disagree"
        );
        ungripped
    }

    /// Rift wants `window` at `desired`; a write goes out under `request`. A desire the core
    /// sends or queues is where the model takes the window to be from then on.
    pub(crate) fn want(
        &mut self,
        request: TransactionId,
        window: WindowId,
        desired: CGRect,
        want: Want,
    ) -> Verdict {
        let now = self.core.clock.now();
        let effects = self.core.decide(Event::Want {
            now,
            request,
            window,
            desired,
            want,
        });
        self.project(&effects, None);
        let mut verdict = Verdict::Nothing;
        for effect in effects {
            match effect {
                Effect::Write(write) => verdict = Verdict::Send(write),
                Effect::Queued { .. } => verdict = Verdict::Queued,
                _ => {}
            }
        }
        verdict
    }

    /// One arrange pass (`desires` in layout order): the core's plans, one per batch that writes,
    /// and whether any desire was new (written or queued).
    pub(crate) fn arrange(
        &mut self,
        want: Want,
        desires: Vec<rift_core::Desire<Rift>>,
    ) -> (Vec<rift_core::Plan<Rift>>, bool) {
        let now = self.core.clock.now();
        let effects = self.core.decide(Event::Arrange { now, want, desires });
        self.project(&effects, None);
        let mut plans = Vec::new();
        let mut any_new = false;
        for effect in effects {
            match effect {
                Effect::Plan(plan) => {
                    any_new = true;
                    plans.push(plan);
                }
                Effect::Queued { .. } => any_new = true,
                _ => {}
            }
        }
        (plans, any_new)
    }

    /// The modifier drag's interactive frame: the only write a dragged window gets.
    pub(crate) fn follow(
        &mut self,
        request: TransactionId,
        window: WindowId,
        frame: CGRect,
    ) -> Write {
        let effects = self.core.decide(Event::Follow { request, window, frame });
        self.project(&effects, None);
        Reconciler::writes(effects)
            .next()
            .expect("the core writes the interactive frame it is given")
    }

    /// The app answered a write to `window` with `frame`, naming the request `seen` (if it
    /// named one). What waited behind the write goes out under `request`.
    pub(crate) fn answer(
        &mut self,
        request: TransactionId,
        seen: Option<TransactionId>,
        window: WindowId,
        frame: CGRect,
    ) -> Reply {
        let now = self.core.clock.now();
        let effects = self.core.decide(Event::Answer {
            now,
            request,
            seen,
            window,
            frame,
        });
        self.project(&effects, None);
        let mut reply = Reply::Late;
        let mut queued = None;
        for effect in effects {
            match effect {
                Effect::Answered { answer, .. } => reply = Reply::Answered(answer, None),
                Effect::Stale { .. } => reply = Reply::Stale,
                Effect::Write(write) => queued = Some(write),
                _ => {}
            }
        }
        match reply {
            Reply::Answered(answer, None) => Reply::Answered(answer, queued),
            reply => reply,
        }
    }

    /// `window` reported `frame` unprompted.
    pub(crate) fn observe(&mut self, window: WindowId, frame: CGRect) -> Observation {
        let effects = self.core.decide(Event::Report { window, frame });
        self.project(&effects, None);
        effects
            .into_iter()
            .find_map(|effect| match effect {
                Effect::Observed { observation, .. } => Some(observation),
                _ => None,
            })
            .expect("the core classifies every report on a window it sees")
    }

    /// Stops waiting for writes past their deadline; returns what waited behind them, which
    /// goes out under `request`.
    pub(crate) fn expire(&mut self, request: TransactionId) -> Vec<Write> {
        let now = self.core.clock.now();
        let effects = self.core.decide(Event::Tick { now, request });
        self.project(&effects, None);
        Reconciler::writes(effects).collect()
    }

    /// `window` is gone, or something outside the reconciler (an animation) now owns its
    /// frame: nothing is outstanding.
    pub(crate) fn forget(&mut self, window: WindowId) {
        let effects = self.core.decide(Event::Forget { window });
        self.project(&effects, None);
    }

    /// The model takes `window` to be at `frame`: Rift accepted a report of it there, or put it
    /// there outside the reconciler.
    pub(crate) fn seen(&mut self, window: WindowId, frame: CGRect) {
        let effects = self.core.decide(Event::Seen { window, frame });
        self.project(&effects, None);
    }

    /// An accessibility destruction during native fullscreen may be an identity replacement:
    /// the window's state is suspended (the model no longer sees it), its record kept.
    pub(crate) fn suspend_fullscreen_ax_identity(&mut self, window: WindowId) -> bool {
        let suspended = self.windows.suspend_fullscreen_ax_identity(window);
        if suspended {
            let effects = self.core.decide(Event::Unseen { window });
            self.project(&effects, None);
        }
        suspended
    }

    /// An animated layout pass over `desires` (the active workspace's windows): which to
    /// animate, each `(window, from, to, request)`, and which to write instead (where the app
    /// has them is unknown). The model sees each at its target from now on.
    pub(crate) fn animate(&mut self, desires: Vec<rift_core::Desire<Rift>>) -> Animated {
        let now = self.core.clock.now();
        let effects = self.core.decide(Event::Animate { now, desires });
        self.project(&effects, None);
        let mut animated = Animated::default();
        for effect in effects {
            match effect {
                Effect::Animate { window, from, to, request } => {
                    animated.animations.push((window, from, to, request))
                }
                Effect::Write(write) => animated.writes.push(write),
                _ => {}
            }
        }
        animated
    }

    /// The animation under `request` ended.
    pub(crate) fn animation_ended(&mut self, request: TransactionId) {
        let now = self.core.clock.now();
        let effects = self.core.decide(Event::AnimationEnded { now, request });
        self.project(&effects, None);
    }

    /// The platform stopped `window`'s animation where the model now sees it.
    pub(crate) fn animation_stopped(&mut self, window: WindowId, request: TransactionId) {
        let effects = self.core.decide(Event::AnimationStopped { window, request });
        self.project(&effects, None);
    }

    /// The displays changed: where the apps have these windows is unknown until they say
    /// (macOS resizes windows silently on a display change).
    pub(crate) fn distrust(&mut self, windows: Vec<WindowId>) {
        let effects = self.core.decide(Event::Distrust { windows });
        self.project(&effects, None);
    }

    /// The app's window was read at `frame` (an inventory, the window server): the model sees it
    /// there and its app has it there.
    pub(crate) fn found(&mut self, window: WindowId, frame: CGRect) {
        let effects = self.core.decide(Event::Found { window, frame });
        self.project(&effects, None);
    }

    /// Tracks a new window's state; the model sees it where its state says it is.
    pub(crate) fn insert_window(
        &mut self,
        window: WindowId,
        state: crate::model::reactor::WindowState,
    ) {
        let frame = state.frame_monotonic;
        self.windows.insert_window(window, state);
        self.seen(window, frame);
    }

    /// The window server reports a window; a normal-layer one is seen where it reports it.
    pub(crate) fn observe_native_window(
        &mut self,
        info: crate::sys::window_server::WindowServerInfo,
    ) -> Option<WindowId> {
        let window = self.windows.observe_native_window(info)?;
        if info.layer == 0 {
            self.found(window, info.frame);
        }
        Some(window)
    }

    /// `button` went down on `window` (tracked and managed), or on none.
    pub(crate) fn press(&mut self, window: Option<WindowId>, button: MouseButton) {
        let effects = self.core.decide(Event::Press { window, button: button.into() });
        self.project(&effects, None);
    }

    /// The pressed `window`'s movement is the user's drag, with `session`. Refused (and the
    /// session dropped) unless `window` is the one pressed.
    pub(crate) fn begin_drag(&mut self, window: WindowId, session: Session) {
        let effects = self.core.decide(Event::Drag { window });
        self.project(&effects, Some(session));
    }

    /// Tests start a drag the only way there is: a press on the window, then its drag.
    #[cfg(test)]
    pub(crate) fn press_and_drag(
        &mut self,
        window: WindowId,
        button: MouseButton,
        session: Session,
    ) {
        self.press(Some(window), button);
        self.begin_drag(window, session);
    }

    /// The pointer was released: what the release ended, if anything.
    pub(crate) fn release(&mut self, release: Release) -> Option<Ungripped> {
        let effects = self.core.decide(Event::Release(release));
        self.project(&effects, None)
    }

    /// The hand comes off without a drop (a cancel, Mission Control).
    pub(crate) fn let_go(&mut self) -> Option<Ungripped> {
        let effects = self.core.decide(Event::LetGo);
        self.project(&effects, None)
    }

    /// `window` left Rift's control: a grip on it ends.
    pub(crate) fn gone(&mut self, window: WindowId) {
        let effects = self.core.decide(Event::Gone { window });
        self.project(&effects, None);
    }

    /// macOS replaced `from` by `to`: a grip moves with it.
    pub(crate) fn rekeyed(&mut self, from: WindowId, to: WindowId) {
        let effects = self.core.decide(Event::Rekeyed { from, to });
        self.project(&effects, None);
    }

    /// Removes `window`'s record; a grip on it ends first, through the core.
    pub(crate) fn remove_window(&mut self, window: WindowId) -> Option<WindowRecord> {
        self.gone(window);
        self.windows.remove_window(window)
    }

    /// Removes every window of `pid`; a grip on one ends first, through the core.
    pub(crate) fn remove_windows_for_app(&mut self, pid: pid_t) {
        let windows: Vec<_> = self.windows.window_ids_for_pid(pid).collect();
        for window in windows {
            self.gone(window);
        }
        self.windows.remove_windows_for_app(pid);
    }

    /// An outside owner took a window (a claim): a grip on it ends, and it is no longer a drop
    /// target.
    pub(crate) fn forget_drag_window(&mut self, window: WindowId) {
        let effects = self.core.decide(Event::Ceded { window });
        self.project(&effects, None);
        if let Some((_, session)) = self.windows.drag_mut() {
            session.forget_target(window);
        }
    }

    /// Binds an AX observation to its native identity. When that identity belonged to another
    /// window id (macOS replaced it), a grip moves to the new id first, through the core.
    /// Retires the windows of `pid` that an inventory no longer reports (see
    /// `WindowStore::inventory_retirements`); a grip on one ends first, through the core.
    pub(crate) fn reconcile_app_inventory(
        &mut self,
        pid: pid_t,
        observed: &[WindowId],
        inactive: &HashSet<WindowId>,
        observe: impl FnMut(
            WindowServerId,
            Option<crate::sys::window_server::WindowServerInfo>,
        ) -> crate::model::window_store::InventoryWindowObservation,
    ) -> Vec<(WindowId, Option<WindowServerId>)> {
        let retired = self.windows.inventory_retirements(pid, observed, inactive, observe);
        for &(window, _) in &retired {
            self.remove_window(window);
        }
        retired
    }

    pub(crate) fn reconcile_ax_identity(
        &mut self,
        window: WindowId,
        server_id: Option<WindowServerId>,
    ) -> Option<WindowId> {
        if let Some(previous) = server_id
            .and_then(|server_id| self.windows.tracked_window_id(server_id))
            .filter(|previous| *previous != window)
        {
            self.rekeyed(previous, window);
        }
        let previous = self.windows.reconcile_ax_identity(window, server_id);
        debug_assert_eq!(
            self.windows.window(window).map(|state| state.frame_monotonic),
            self.core.seen(window),
            "the store moved the window's state the way the core moved where it sees it"
        );
        previous
    }
}
