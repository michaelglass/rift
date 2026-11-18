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
use rift_core::reconciler::Note;
pub(crate) use rift_core::{Answer, Observation, Owner, Want};
use rift_core::{Core, Kinds};
use tracing::{debug, warn};

use super::animation::scrolling_bounds;
use super::managers::ScrollingDisplayBounds;
use super::{Reactor, TransactionId};
use crate::actor::app::{AppThreadHandle, FrameMode, FrameSource, Request, WindowId, pid_t};
use crate::common::collections::{HashMap, HashSet};
use crate::model::window_store::Pointer;
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

/// The core the reactor steps: the Rust core, or with the `roc-core` feature the Roc core
/// (shadowed by the Rust core, which must agree at every step).
#[cfg(not(feature = "roc-core"))]
type Engine = rift_core::RustCore<Rift>;
#[cfg(feature = "roc-core")]
type Engine = super::roc_bridge::RocInRift;

/// One step through the core, as the reactor took it: what went in and what came out.
#[cfg(test)]
pub(crate) type Step = (Event, Vec<Effect>);

/// The reactor's handle on the core. The model stays inside: the reactor reads nothing of it
/// but what effects say (`armed`), except the two Rust-only reads marked below.
#[derive(Debug)]
pub(crate) struct Reconciler {
    /// Shared so the mode gate, asked from `&self`, can step it too.
    core: Shared<Engine>,
    /// The pointer as the core was last told it. The reactor's window store owns the press
    /// and drag model; the core is told each change before it decides anything.
    told: Cell<Pointer>,
    /// The deadline the core last asked the run loop's timer to be armed for.
    armed: Cell<Option<Instant>>,
    pub(crate) clock: Clock,
    /// When `Some`, every step is recorded (the differential test replays them).
    #[cfg(test)]
    pub(crate) trace: RefCell<Option<Vec<Step>>>,
}

impl Default for Reconciler {
    fn default() -> Self {
        let clock = Clock::default();
        #[cfg(not(feature = "roc-core"))]
        let core = Shared::new(Engine::default());
        #[cfg(feature = "roc-core")]
        let core = Shared::new(Engine::new(clock.now()));
        Reconciler {
            core,
            told: Cell::new(Pointer::Idle),
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
                Effect::Write(write) => {
                    debug!(window = ?write.window(), frame = ?write.frame(), "reconcile: write")
                }
                Effect::Queued { window } => debug!(?window, "reconcile: queued"),
                Effect::Answered { window, answer } => {
                    debug!(?window, ?answer, "reconcile: answer")
                }
                Effect::Stale { window } => {
                    debug!(?window, "reconcile: answer to a replaced write, dropped")
                }
                Effect::Late { window } => {
                    debug!(?window, "reconcile: late answer, observed as unprompted")
                }
                Effect::Note { window, note: Note::Discarded } => {
                    debug!(?window, "reconcile: discarded (user's)")
                }
                Effect::Note { window, note: Note::TimedOut } => {
                    debug!(?window, "reconcile: timeout")
                }
                Effect::Note { window, note: Note::Forgotten } => {
                    debug!(?window, "reconcile: forgotten (gone)")
                }
                Effect::Observed { .. } | Effect::Gate { .. } => {}
            }
        }
        #[cfg(test)]
        if let (Some(event), Some(trace)) = (recorded, self.trace.borrow_mut().as_mut()) {
            trace.push((event, effects.clone()));
        }
        effects
    }

    /// Tells the core what the pointer is doing, if that changed since it was last told.
    fn tell(&self, pointer: Pointer) {
        if self.told.get() != pointer {
            self.told.set(pointer);
            self.step(Event::Pointer(pointer));
        }
    }

    fn writes(effects: Vec<Effect>) -> impl Iterator<Item = Write> {
        effects.into_iter().filter_map(|effect| match effect {
            Effect::Write(write) => Some(write),
            _ => None,
        })
    }

    /// Rust-only read, not part of the boundary: tests, and the fuzzer's invariants.
    #[cfg(test)]
    pub(crate) fn sync(&self, window: WindowId) -> Sync {
        self.core.borrow().reconciler().sync(window)
    }

    /// Rift wants `window` at `desired`; the model last saw it at `known`. A write goes out
    /// under `request`.
    fn want(
        &mut self,
        pointer: Pointer,
        request: TransactionId,
        window: WindowId,
        desired: CGRect,
        known: CGRect,
        want: Want,
    ) -> Verdict {
        self.tell(pointer);
        let now = self.clock.now();
        let effects = self.step(Event::Want {
            now,
            request,
            window,
            desired,
            known,
            want,
        });
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

    /// The modifier drag's interactive frame: the only write a dragged window gets.
    pub(crate) fn follow(
        &mut self,
        request: TransactionId,
        window: WindowId,
        frame: CGRect,
    ) -> Write {
        let effects = self.step(Event::Follow { request, window, frame });
        Self::writes(effects)
            .next()
            .expect("the core writes the interactive frame it is given")
    }

    /// The app answered a write to `window` with `frame`, naming the request `seen` (if it
    /// named one). What waited behind the write goes out under `request`.
    pub(crate) fn answer(
        &mut self,
        pointer: Pointer,
        request: TransactionId,
        seen: Option<TransactionId>,
        window: WindowId,
        frame: CGRect,
    ) -> Reply {
        self.tell(pointer);
        let now = self.clock.now();
        let mut reply = Reply::Late;
        let mut queued = None;
        for effect in self.step(Event::Answer {
            now,
            request,
            seen,
            window,
            frame,
        }) {
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

    /// `window` reported `frame` unprompted; the model last saw it at `known`.
    pub(crate) fn observe(
        &mut self,
        pointer: Pointer,
        window: WindowId,
        frame: CGRect,
        known: CGRect,
    ) -> Observation {
        self.tell(pointer);
        self.step(Event::Report { window, frame, known })
            .into_iter()
            .find_map(|effect| match effect {
                Effect::Observed { observation, .. } => Some(observation),
                _ => None,
            })
            .expect("the core classifies every report")
    }

    /// Stops waiting for writes past their deadline. `known` is where the model last saw a
    /// window, `None` once it is gone. Returns what waited behind the late writes.
    fn expire(
        &mut self,
        pointer: Pointer,
        request: TransactionId,
        known: impl Fn(WindowId) -> Option<CGRect>,
    ) -> Vec<Write> {
        self.tell(pointer);
        let now = self.clock.now();
        let due = self.core.borrow().reconciler().due(now);
        let known = due.into_iter().filter_map(|window| Some((window, known(window)?))).collect();
        Self::writes(self.step(Event::Tick { now, request, known })).collect()
    }

    /// When the run loop's timer should fire: the deadline the core last asked for.
    pub(crate) fn next_deadline(&self) -> Option<Instant> { self.armed.get() }

    /// `window` is gone, or something outside the reconciler (an animation) now owns its
    /// frame: nothing is outstanding.
    pub(crate) fn forget(&mut self, window: WindowId) { self.step(Event::Forget { window }); }
}

/// One app's share of an arrange pass: the request it goes out under, its windows that only
/// move, and those that also resize, and the targets to record under the request.
struct Batch {
    request: TransactionId,
    positions: Vec<Write>,
    frames: Vec<Write>,
    targets: Vec<(WindowServerId, CGRect)>,
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
        let pointer = self.state.windows.pointer();
        let Some(state) = self.state.windows.window_mut(window) else {
            return Verdict::Nothing;
        };
        let verdict =
            self.reconciler
                .want(pointer, request, window, desired, state.frame_monotonic, want);
        match verdict {
            Verdict::Send(_) | Verdict::Queued => state.frame_monotonic = desired,
            Verdict::Nothing => {}
        }
        verdict
    }

    /// Records a write's request, with its target, for the app protocol and the space
    /// bookkeeping (`TransactionManager`).
    fn record(&self, write: &Write) {
        let server_id =
            self.state.windows.window(write.window()).and_then(|state| state.info.sys_id);
        self.transaction_manager.record(
            write.request(),
            server_id.map(|server_id| (server_id, write.frame())),
        );
    }

    fn send_one(&mut self, write: Write, handle: AppThreadHandle) {
        self.record(&write);
        if let Err(error) = handle.send(Request::set_window_frame(
            write.window(),
            write.frame(),
            write.request(),
            true,
        )) {
            warn!(window = ?write.window(), %error, "failed to write window frame");
            self.reconciler.forget(write.window());
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
        let request = self.transaction_manager.mint();
        match self.arrange_one(window, frame, Want::Place, request) {
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
            self.reconciler.forget(window);
            return;
        };
        let bounds = self
            .assigned_space_for_window_id(window)
            .filter(|&space| self.in_scrolling_bounds(window, space))
            .and_then(|space| scrolling_bounds(self, space));
        let handle = app.handle.with_scrolling_bounds(bounds);
        if let Some(state) = self.state.windows.window_mut(window) {
            state.frame_monotonic = write.frame();
        }
        self.send_one(write, handle);
    }

    /// Sends the interactive frame of the window the user is dragging.
    pub(super) fn send_interactive(&mut self, write: Write) {
        if !self.may_write(write.window()) {
            return;
        }
        self.record(&write);
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
        let request = self.transaction_manager.mint();
        let windows = &self.state.windows;
        let queued = self.reconciler.expire(windows.pointer(), request, |window| {
            Some(windows.window(window)?.frame_monotonic)
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
        Batch {
            request,
            positions,
            frames,
            targets,
        }: Batch,
    ) {
        let Some(app) = self.app_manager.apps.get(&pid) else {
            debug!(?pid, "Skipping layout update for app - app no longer exists");
            for write in positions.iter().chain(&frames) {
                self.reconciler.forget(write.window());
            }
            return;
        };
        let handle = app.handle.with_scrolling_bounds(bounds);
        self.transaction_manager.record(request, targets);
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
        let mut per_app: HashMap<(pid_t, bool), Batch> = HashMap::default();
        // One request per app batch, minted before the core decides (its writes carry it).
        let mut requests: HashMap<(pid_t, bool), TransactionId> = HashMap::default();
        let mut any_desire_new = false;

        for &(wid, target) in frames {
            let constrained = bounds.is_some() && self.in_scrolling_bounds(wid, space);
            let Some(window) = self.state.windows.window(wid) else {
                debug!(?wid, "Skipping layout - window no longer exists");
                continue;
            };
            let (current, server_id) = (window.frame_monotonic, window.info.sys_id);
            let key = (wid.pid, constrained);
            let request = *requests.entry(key).or_insert_with(|| self.transaction_manager.mint());
            let write = match self.arrange_one(wid, target, want, request) {
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
            let batch = per_app.entry(key).or_insert_with(|| Batch {
                request,
                positions: Vec::new(),
                frames: Vec::new(),
                targets: Vec::new(),
            });
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
