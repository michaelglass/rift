//! Desired frames, and the reactor's side of the core's reconciler.
//!
//! Layout says where each window should be ([`Desired`]). The core (`rift_core`) compares that
//! with what each window's app has acknowledged and decides the writes; this module is the thin
//! applier between it and the reactor: it turns reactor calls into core [`Event`]s, steps the
//! core, and turns the [`Effect`]s back into what the reactor acts on. A `Write` can only come
//! out of the core, and the senders below take nothing but a `Write`, so a frame reaches an
//! app only because the core decided it should.

use std::time::{Duration, Instant};

use objc2_core_foundation::CGRect;
use rift_core::{Kinds, Release};
pub(crate) use rift_core::{Owner, Want};
use tracing::{debug, warn};

use super::animation::scrolling_bounds;
use super::managers::ScrollingDisplayBounds;
use super::{Reactor, TransactionId};
use crate::actor::app::{AppThreadHandle, FrameMode, FrameSource, Request, WindowId, pid_t};
use crate::common::collections::{HashMap, HashSet};
use crate::model::RiftState;
use crate::model::drag::Session;
use crate::model::window_store::WindowRecord;
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
pub(crate) use rift_core::Reply;
pub(crate) type Event = rift_core::Event<Rift>;
/// The core's events, as the reactor (which has its own `Event`) names them.
pub(crate) type CoreEvent = Event;
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
#[cfg(not(feature = "roc-core"))]
type Engine = rift_core::RustCore<Rift>;
#[cfg(feature = "roc-core")]
type Engine = Shadowed;

/// Rift's types as plain data and back (the Roc core's, and a recorded trace's): times are
/// nanoseconds since `epoch`.
#[cfg(any(test, feature = "roc-core"))]
#[derive(Debug, Clone, Copy)]
pub(crate) struct Plainly {
    pub(crate) epoch: Instant,
}

#[cfg(any(test, feature = "roc-core"))]
impl rift_core::plain::Translate<Rift, rift_core::plain::Plain> for Plainly {
    fn window(&self, window: WindowId) -> rift_core::plain::WindowId {
        rift_core::plain::WindowId {
            pid: window.pid,
            idx: window.idx.get(),
        }
    }

    fn frame(&self, frame: CGRect) -> rift_core::plain::Rect {
        rift_core::plain::Rect {
            x: frame.origin.x,
            y: frame.origin.y,
            w: frame.size.width,
            h: frame.size.height,
        }
    }

    fn time(&self, time: Instant) -> rift_core::plain::Nanos {
        rift_core::plain::Nanos(time.saturating_duration_since(self.epoch).as_nanos() as u64)
    }

    fn request(&self, request: TransactionId) -> rift_core::plain::RequestId {
        rift_core::plain::RequestId(request.number())
    }
}

#[cfg(feature = "roc-core")]
impl rift_core::plain::Translate<rift_core::plain::Plain, Rift> for Plainly {
    fn window(&self, window: rift_core::plain::WindowId) -> WindowId {
        let idx = std::num::NonZeroU32::new(window.idx).expect("the core names only real windows");
        WindowId { pid: window.pid, idx }
    }

    fn frame(&self, frame: rift_core::plain::Rect) -> CGRect {
        use objc2_core_foundation::{CGPoint, CGSize};
        CGRect::new(CGPoint::new(frame.x, frame.y), CGSize::new(frame.w, frame.h))
    }

    fn time(&self, time: rift_core::plain::Nanos) -> Instant {
        self.epoch + Duration::from_nanos(time.0)
    }

    fn request(&self, request: rift_core::plain::RequestId) -> TransactionId {
        TransactionId::from_number(request.0)
    }
}

/// The Roc core deciding, with the Rust core stepped beside it on the same events: every step,
/// the two must decide the same effects, or Rift stops (a disagreement is a bug in one of them,
/// and acting on either would hide it).
#[cfg(feature = "roc-core")]
pub(crate) struct Shadowed {
    roc: rift_core_roc::RocCore,
    rust: rift_core::RustCore<Rift>,
    plain: Plainly,
}

#[cfg(feature = "roc-core")]
impl Default for Shadowed {
    fn default() -> Self {
        // Before any time the reactor will hand the core (a test clock starts at `now`).
        let now = Instant::now();
        let epoch = now.checked_sub(Duration::from_secs(3600)).unwrap_or(now);
        Shadowed {
            roc: rift_core_roc::RocCore::default(),
            rust: rift_core::RustCore::default(),
            plain: Plainly { epoch },
        }
    }
}

#[cfg(feature = "roc-core")]
impl std::fmt::Debug for Shadowed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shadowed").field("rust", &self.rust).finish_non_exhaustive()
    }
}

#[cfg(feature = "roc-core")]
impl Shadowed {
    fn step(&mut self, event: Event) -> Vec<Effect> {
        // The Roc core rejecting an event rift encoded, or answering in words rift cannot read,
        // is a bug in one of the two codecs: rift stops and says which.
        let decided: Vec<Effect> = match self.roc.step(event.clone().translate(&self.plain)) {
            Ok(effects) => {
                effects.into_iter().map(|effect| effect.translate(&self.plain)).collect()
            }
            Err(error) => panic!("the Roc core gave no answer ({error:?}) to {event:?}"),
        };
        let shadow = self.rust.step(event.clone());
        assert_eq!(
            decided, shadow,
            "the Roc core and its Rust shadow disagree on {event:?}"
        );
        decided
    }

    /// The shadow's pointer: the Roc core's, which decides the same effects.
    fn pointer(&self) -> rift_core::Pointer<WindowId> { self.rust.pointer() }

    #[cfg(test)]
    fn reconciler(&self) -> &rift_core::Reconciler<Rift> { self.rust.reconciler() }

    #[cfg(test)]
    fn seen(&self, window: WindowId) -> Option<CGRect> { self.rust.seen(window) }
}

/// One step through the core, as the reactor took it: what went in and what came out.
#[cfg(test)]
pub(crate) type Step = (Event, Vec<Effect>);

/// The store refused a grip effect: it and the core disagree about the hand. Logged in a
/// running Rift (the core's next effect corrects the store); a failure in tests, so the
/// fuzzer finds what led there.
fn refused(result: Result<(), crate::model::window_store::GripRefused>, effect: &Effect) {
    if let Err(refused) = result {
        #[cfg(test)]
        panic!("the store refused {effect:?}: {refused:?}");
        #[cfg(not(test))]
        tracing::error!(?effect, ?refused, "the store refused a grip effect");
    }
}

/// The reactor's handle on the core. The model stays inside: the reactor reads nothing of it
/// but what effects say (`armed`), except the two Rust-only reads marked below.
#[derive(Debug, Default)]
pub(crate) struct Reconciler {
    core: Engine,
    /// The deadline the core last asked the run loop's timer to be armed for.
    armed: Option<Instant>,
    pub(crate) clock: Clock,
    /// When `Some`, every step is recorded (the fuzzer's invariants read why Rift wrote).
    #[cfg(test)]
    pub(crate) trace: Option<Vec<Step>>,
}

impl Reconciler {
    fn step(&mut self, event: Event) -> Vec<Effect> {
        #[cfg(test)]
        let recorded = self.trace.is_some().then(|| event.clone());
        let effects = self.core.step(event);
        for effect in &effects {
            match effect {
                Effect::Timer(at) => self.armed = *at,
                effect => debug!(?effect, "reconcile"),
            }
        }
        #[cfg(test)]
        if let (Some(event), Some(trace)) = (recorded, self.trace.as_mut()) {
            trace.push((event, effects.clone()));
        }
        effects
    }

    /// Rust-only read, not part of the boundary: tests, and the fuzzer's invariants.
    #[cfg(test)]
    pub(crate) fn sync(&self, window: WindowId) -> Option<Sync> {
        self.core.reconciler().sync(window)
    }

    /// Rust-only read, not part of the boundary: the frame Rift has pending for `window`.
    #[cfg(test)]
    pub(crate) fn pending(&self, window: WindowId) -> Option<CGRect> {
        self.core.reconciler().pending(window)
    }

    /// Rust-only read, not part of the boundary: where the core sees `window` (the fuzzer
    /// checks the store's `frame_monotonic` agrees).
    #[cfg(test)]
    pub(crate) fn seen(&self, window: WindowId) -> Option<CGRect> { self.core.seen(window) }

    /// When the run loop's timer should fire: the deadline the core last asked for.
    pub(crate) fn next_deadline(&self) -> Option<Instant> { self.armed }
}

fn writes(effects: Vec<Effect>) -> impl Iterator<Item = Write> {
    effects.into_iter().filter_map(|effect| match effect {
        Effect::Write(write) => Some(write),
        _ => None,
    })
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

    fn send_one(&mut self, write: Write, handle: AppThreadHandle) {
        if let Err(error) = handle.send(Request::set_window_frame(
            write.window(),
            write.frame(),
            write.request(),
            true,
        )) {
            warn!(window = ?write.window(), %error, "failed to write window frame");
            self.state.step(CoreEvent::Forget { window: write.window() });
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
        let desire = rift_core::Desire { window, frame, request };
        let (plans, _) = self.state.arrange(Want::Arrange, vec![desire]);
        for write in plans.into_iter().flat_map(|plan| {
            let (_, positions, frames) = plan.into_parts();
            positions.into_iter().chain(frames)
        }) {
            self.send_one(write, handle.clone());
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
            self.state.step(CoreEvent::Forget { window });
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
        plan: rift_core::Plan<Rift>,
    ) {
        let (request, positions, frames) = plan.into_parts();
        let Some(app) = self.app_manager.apps.get(&pid) else {
            debug!(?pid, "Skipping layout update for app - app no longer exists");
            for write in positions.iter().chain(&frames) {
                self.state.step(CoreEvent::Forget { window: write.window() });
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
            let (pid, constrained) = keys[&plan.request()];
            self.send_batch(pid, bounds.filter(|_| constrained), plan);
        }
        any_desire_new
    }
}

/// What a grip step let go of: the window, and the drag's session if it was a drag.
pub(crate) struct Ungripped {
    pub(crate) window: WindowId,
    pub(crate) session: Option<Session>,
    /// Windows whose model frame changed during the drag, to re-derive.
    pub(crate) changed: Vec<WindowId>,
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

    /// Steps the core and applies what it projects onto the store.
    pub(crate) fn step(&mut self, event: Event) -> Vec<Effect> { self.apply(event).0 }

    /// Steps the core and applies its effects; what an ungrip let go of, if anything.
    fn apply(&mut self, event: Event) -> (Vec<Effect>, Option<Ungripped>) {
        let effects = self.core.step(event);
        let ungripped = self.project(&effects);
        (effects, ungripped)
    }

    /// Applies one step's effects to the store.
    fn project(&mut self, effects: &[Effect]) -> Option<Ungripped> {
        let mut ungripped = None;
        for effect in effects {
            match *effect {
                Effect::Ungrip { window, ref changed, .. } => {
                    let session = self.windows.project_ungrip();
                    ungripped = Some(Ungripped {
                        window,
                        session,
                        changed: changed.clone(),
                    });
                }
                Effect::Grip { window } => refused(self.windows.project_grip(window), effect),
                Effect::Regrip { from, to } => {
                    refused(self.windows.project_regrip(from, to), effect)
                }
                // The pending store the app actors read is the core's pending frames. It is keyed
                // by window-server id, taken from the record: a suspended identity has no AX
                // state, but its record (and its server id) remains.
                Effect::Pending { window, target } => {
                    let record = self.windows.record(window);
                    if let Some(server_id) = record.and_then(|record| record.window_server_id()) {
                        self.tx.project_target(server_id, target);
                    }
                }
                // The latest request sent each window, which the app actors read.
                Effect::Write(ref write) => self.sent(write),
                Effect::Plan(ref plan) => {
                    for write in plan.positions().iter().chain(plan.frames()) {
                        self.sent(write);
                    }
                }
                Effect::Animate { window, request, .. } => {
                    if let Some(server_id) = self.windows.server_id(window) {
                        self.tx.sent(server_id, request);
                    }
                }
                Effect::Seen { window, frame } => {
                    if let Some(state) = self.windows.window_mut(window) {
                        state.project_seen(frame);
                    }
                }
                _ => {}
            }
        }
        if let Some(Ungripped { window, .. }) = &ungripped {
            self.windows.prune_ungripped(*window);
        }
        debug_assert_eq!(
            self.windows.pointer(),
            self.core.core.pointer(),
            "store and core disagree"
        );
        ungripped
    }

    /// One arrange pass (`desires` in layout order): the core's plans, one per batch that writes,
    /// and whether any desire was new (written or queued).
    pub(crate) fn arrange(
        &mut self,
        want: Want,
        desires: Vec<rift_core::Desire<Rift>>,
    ) -> (Vec<rift_core::Plan<Rift>>, bool) {
        let now = self.core.clock.now();
        let effects = self.step(Event::Arrange { now, want, desires });
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
        let effects = self.step(Event::Follow { request, window, frame });
        writes(effects)
            .next()
            .expect("the core writes the interactive frame it is given")
    }

    /// `window` was read at `frame`, as of the request `as_of` (a read Rift asked for) or
    /// unstamped (a notification). What it meant, and the write that waited behind an answered
    /// one, which goes out under `request`.
    pub(crate) fn observe(
        &mut self,
        request: TransactionId,
        window: WindowId,
        frame: CGRect,
        as_of: Option<TransactionId>,
    ) -> (Reply, Option<Write>) {
        let now = self.core.clock.now();
        let effects = self.step(Event::Observe {
            now,
            request,
            window,
            frame,
            as_of,
        });
        let mut reply = Reply::NoNews;
        let mut released = None;
        for effect in effects {
            match effect {
                Effect::Replied { reply: replied, .. } => reply = replied,
                Effect::Write(write) => released = Some(write),
                _ => {}
            }
        }
        (reply, released)
    }

    /// Stops waiting for writes past their deadline; returns what waited behind them, which
    /// goes out under `request`.
    pub(crate) fn expire(&mut self, request: TransactionId) -> Vec<Write> {
        let now = self.core.clock.now();
        let effects = self.step(Event::Tick { now, request });
        writes(effects).collect()
    }

    /// An accessibility destruction during native fullscreen may be an identity replacement:
    /// the window's state is suspended (the model no longer sees it), its record kept.
    pub(crate) fn suspend_fullscreen_ax_identity(&mut self, window: WindowId) -> bool {
        if self.windows.native_fullscreen_record_for_window(window).is_none() {
            return false;
        }
        // The core forgets the window while its state is still there, then the state goes.
        self.step(Event::Unseen { window });
        self.windows.suspend_fullscreen_ax_identity(window)
    }

    /// An animated layout pass over `desires` (the active workspace's windows): which to
    /// animate, each `(window, from, to, request)`, and which to write instead (where the app
    /// has them is unknown). The model sees each at its target from now on.
    pub(crate) fn animate(&mut self, desires: Vec<rift_core::Desire<Rift>>) -> Animated {
        let now = self.core.clock.now();
        let effects = self.step(Event::Animate { now, desires });
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
        self.step(Event::AnimationEnded { now, request });
    }

    /// The app's window was read at `frame` (an inventory, the window server), as of the
    /// request the app had applied (`None` unstamped). Unless that is no news, the model takes
    /// the window to be there.
    pub(crate) fn read(
        &mut self,
        window: WindowId,
        frame: CGRect,
        as_of: Option<TransactionId>,
    ) -> (Reply, Option<Write>) {
        let request = self.tx.next_id();
        let (reply, released) = self.observe(request, window, frame, as_of);
        if reply != Reply::NoNews {
            self.step(Event::Seen { window, frame });
        }
        (reply, released)
    }

    /// Tracks a new window's state; the model sees it where its state says it is.
    pub(crate) fn insert_window(
        &mut self,
        window: WindowId,
        state: crate::model::reactor::WindowState,
    ) {
        let frame = state.frame_monotonic();
        self.windows.insert_window(window, state);
        self.step(Event::Seen { window, frame });
    }

    /// Tracks a new window's state with the heuristic's verdict and a rule already known.
    pub(crate) fn insert_window_as(
        &mut self,
        window: WindowId,
        state: crate::model::reactor::WindowState,
        manageability: rift_core::lifecycle::Manageability,
        rule: rift_core::lifecycle::Rule,
    ) {
        let frame = state.frame_monotonic();
        self.windows.insert_window_as(window, state, manageability, rule);
        self.step(Event::Seen { window, frame });
    }

    /// The window server reports a window; a normal-layer one is seen where it reports it.
    pub(crate) fn observe_native_window(
        &mut self,
        info: crate::sys::window_server::WindowServerInfo,
    ) -> Option<WindowId> {
        let window = self.windows.observe_native_window(info)?;
        if info.layer == 0 {
            // Unstamped: the window server's read says nothing of what the app applied.
            let (_, released) = self.read(window, info.frame, None);
            debug_assert!(released.is_none(), "an unstamped read answers nothing");
        }
        Some(window)
    }

    /// The pressed `window`'s movement is the user's drag, with `session`. Refused (and the
    /// session dropped) unless `window` is the one pressed.
    pub(crate) fn begin_drag(&mut self, window: WindowId, session: Session) {
        let effects = self.core.step(Event::Drag { window });
        if effects.contains(&Effect::DragBegan { window }) {
            self.windows.project_drag(session);
        }
        self.project(&effects);
    }

    /// Tests start a drag the only way there is: a press on the window, then its drag.
    #[cfg(test)]
    pub(crate) fn press_and_drag(
        &mut self,
        window: WindowId,
        button: crate::model::drag::MouseButton,
        session: Session,
    ) {
        self.step(Event::Press {
            window: Some(window),
            button: button.into(),
        });
        self.begin_drag(window, session);
    }

    /// The pointer was released: what the release ended, if anything.
    pub(crate) fn release(&mut self, release: Release) -> Option<Ungripped> {
        self.apply(Event::Release(release)).1
    }

    /// The hand comes off without a drop (a cancel, Mission Control).
    pub(crate) fn let_go(&mut self) -> Option<Ungripped> { self.apply(Event::LetGo).1 }

    /// Removes `window`'s record; a grip on it ends first, through the core.
    pub(crate) fn remove_window(&mut self, window: WindowId) -> Option<WindowRecord> {
        self.step(Event::Gone { window });
        self.windows.remove_window(window)
    }

    /// Removes every window of `pid`; a grip on one ends first, through the core.
    pub(crate) fn remove_windows_for_app(&mut self, pid: pid_t) {
        let windows: Vec<_> = self.windows.window_ids_for_pid(pid).collect();
        for window in windows {
            self.step(Event::Gone { window });
        }
        self.windows.remove_windows_for_app(pid);
    }

    /// An outside owner took a window (a claim): a grip on it ends, and it is no longer a drop
    /// target.
    pub(crate) fn forget_drag_window(&mut self, window: WindowId) {
        self.step(Event::Ceded { window });
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
            self.step(Event::Rekeyed { from: previous, to: window });
        }
        self.windows.reconcile_ax_identity(window, server_id)
    }
}
