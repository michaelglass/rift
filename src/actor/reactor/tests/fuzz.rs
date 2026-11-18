//! Event-interleaving fuzzer over the reactor test harness.
//!
//! Each case builds two or three displays with a few apps, then replays a seeded sequence of
//! user input, window-server reports, inventory refreshes, app-driven moves, rift's own echoes
//! (delivered late, out of step with the writes that caused them), commands and known macOS lies.
//! Some apps adjust the size they are asked to take, and none answers until a `Deliver` step.
//! After every step it checks invariants about drags and about rift's frame writes; at the end
//! of a case it lets everything settle and checks that the windows are where layout wants them.
//! A failure prints the seed and the shrunk
//! step list, which `execute` turns back into a deterministic test.
//!
//! Every failure is identified by a signature: the invariant that broke and the kind of step that
//! broke it. `KNOWN` lists the signatures this branch is known to have; the default test fails on
//! any other. `RIFT_FUZZ_CASES` and `RIFT_FUZZ_STEPS` raise the budget for long runs.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};

use test_log::test;

use super::*;
use crate::actor::reactor::managers::LayoutManager;
use crate::actor::reactor::reconciler::{Desired, Plainly, Sync};
use crate::actor::reactor::testing::TestSizing;
use crate::sys::geometry::IsWithin;

/// Signatures (invariant, step kind) this branch is known to fail. Each has a regression test.
const KNOWN: &[(&str, &str)] = &[];

const SCREEN_W: f64 = 1000.;

/// The outside owner the fuzzer claims windows for.
const FUZZ_OWNER: crate::model::window_store::ClaimOwner =
    crate::model::window_store::ClaimOwner(303);
const SCREEN_H: f64 = 800.;

// ---------------------------------------------------------------------------------------------
// Adapter: the only place that knows how this branch exposes the drag.

fn dragged(reactor: &Reactor) -> Option<WindowId> {
    reactor.state.windows.drag().map(|(window, _)| window)
}

fn user_dragging(reactor: &Reactor) -> bool { reactor.user_is_dragging() }

/// The pointer goes down on `wid`. (The simulated windows can overlap, so the press names the
/// window instead of a point for the reactor's test double to resolve.)
fn press(reactor: &mut Reactor, wid: WindowId) {
    reactor.state.step(CoreEvent::Press {
        window: Some(wid),
        button: crate::actor::drag::MouseButton::Left.into(),
    });
}

// ---------------------------------------------------------------------------------------------
// Steps

/// The button state an app-driven report carries.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Mouse {
    /// What the OS really reports: down while the user holds a window, up otherwise.
    Truth,
    /// Reported up regardless: a lie while the user is dragging (e.g. a notification sampled
    /// after an input tap swallowed the press).
    ReportsUp,
    /// Reported down regardless: the button really is down (the user is pressing somewhere).
    ReportsDown,
}

#[derive(Debug, Clone, Copy)]
enum Step {
    /// The user presses on window `w` and starts dragging it (a native title-bar drag).
    DragStart { w: usize, dx: i16, dy: i16 },
    /// The user presses on window `w` and holds still; a later move drags it.
    Press { w: usize },
    /// The user presses on window `w` with the drag modifier held (rift takes the press and
    /// moves the window itself, by interactive frames) and moves the pointer.
    ModifierDrag { w: usize, dx: i16, dy: i16 },
    /// The pointer of a modifier drag moves.
    ModifierMove { dx: i16, dy: i16 },
    /// The held window moves under the pointer.
    DragMove { dx: i16, dy: i16 },
    /// The held window crosses onto display `to`; the window server reports it there.
    CrossDisplay { to: usize },
    /// The pointer of a native drag crosses onto display `to` while most of the window's frame
    /// is still on the display it left (the user holds it near the leading end of its title
    /// bar); the window server puts the window on the pointer's display.
    CrossDisplayLagging { to: usize },
    /// The user releases the button.
    MouseUp,
    /// An inventory refresh re-observes app `app`'s windows. `stale` reports each window's
    /// previous frame (a lagging read); `zero_min` reports a 0x0 minimum (a lie WhatsApp tells).
    Refresh {
        app: usize,
        stale: bool,
        zero_min: bool,
    },
    /// An app moves window `w` itself onto display `to`, reporting the given button state.
    ForeignMove { w: usize, to: usize, mouse: Mouse },
    /// The held window moves but the report says the button is up (tap-to-drag, three-finger
    /// drag: macOS moves the window with no button down).
    HeldMovesButtonUp { dx: i16 },
    /// rift's outstanding requests reach the app threads and their echoes come back.
    Deliver,
    /// Window `w` is destroyed.
    Destroy { w: usize },
    /// The user asks rift to move window `w` to workspace `k` (by its index, as the CLI does;
    /// the sim has no focus to lean on).
    MoveToWorkspace { w: usize, k: usize },
    /// The user asks rift to move the focused window to display `d`.
    MoveFocusedToDisplay { d: usize },
    /// The user moves focus left or right (in a scrolling layout, the viewport follows).
    Focus { right: bool },
    /// Time passes: every unanswered write is past its deadline.
    Tick,
    /// macOS asks for a full refresh (a wake): the display topology is re-sent as it is, with
    /// the refresh flag up. Pending targets of parked windows no longer hold.
    Wake,
    /// Window `w`'s app starts treating the sizes it is asked for this way.
    Sizing { w: usize, sizing: TestSizing },
    /// The user cancels a drag (escape) and lets go: rift lets go of the window.
    Cancel,
    /// An outside owner claims window `w`: rift stops managing it.
    Claim { w: usize },
    /// The outside owner gives window `w` back.
    Unclaim { w: usize },
    /// The displays are reported again in the other order: a display change, after which no
    /// cached frame is trusted.
    ReorderDisplays,
    /// macOS announces a new accessibility identity for window `w`: the same window-server
    /// window under a new window id (its old identity may already be dead, in fullscreen).
    Rekey { w: usize },
    /// Window `w` goes native fullscreen: the window server shows it in its fullscreen space.
    EnterFullscreen { w: usize },
    /// Window `w`'s accessibility identity is destroyed while it is in native fullscreen (macOS
    /// replaces it; a new identity is announced later, by `Rekey`).
    DestroyIdentity { w: usize },
    /// Window `w` leaves native fullscreen: the window server shows it on its space again.
    LeaveFullscreen { w: usize },
}

impl Step {
    fn kind(&self) -> &'static str {
        match self {
            Step::DragStart { .. } => "drag-start",
            Step::Press { .. } => "press",
            Step::ModifierDrag { .. } => "modifier-drag",
            Step::ModifierMove { .. } => "modifier-move",
            Step::Wake => "wake",
            Step::DragMove { .. } => "drag-move",
            Step::CrossDisplay { .. } => "cross-display",
            Step::CrossDisplayLagging { .. } => "cross-display-lagging",
            Step::MouseUp => "mouse-up",
            Step::Refresh { .. } => "refresh",
            Step::ForeignMove { mouse: Mouse::Truth, .. } => "foreign-move",
            Step::ForeignMove { mouse: Mouse::ReportsUp, .. } => "foreign-move-reports-up",
            Step::ForeignMove { mouse: Mouse::ReportsDown, .. } => "foreign-move-reports-down",
            Step::HeldMovesButtonUp { .. } => "lie-tap-drag",
            Step::Deliver => "deliver-echoes",
            Step::Destroy { .. } => "destroy",
            Step::MoveToWorkspace { .. } => "command-move-to-workspace",
            Step::MoveFocusedToDisplay { .. } => "command-move-to-display",
            Step::Focus { .. } => "command-focus",
            Step::Tick => "tick",
            Step::Cancel => "cancel",
            Step::Claim { .. } => "claim",
            Step::Unclaim { .. } => "unclaim",
            Step::ReorderDisplays => "reorder-displays",
            Step::Rekey { .. } => "rekey",
            Step::EnterFullscreen { .. } => "enter-fullscreen",
            Step::DestroyIdentity { .. } => "destroy-identity",
            Step::LeaveFullscreen { .. } => "leave-fullscreen",
            Step::Sizing { .. } => "sizing",
        }
    }
}

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self { Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1) }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize { (self.next() % n as u64) as usize }

    fn delta(&mut self) -> i16 { (self.below(301) as i16) - 150 }
}

fn generate(seed: u64, len: usize) -> Vec<Step> {
    let mut rng = Rng::new(seed);
    (0..len)
        .map(|_| match rng.below(108) {
            0..9 => Step::DragStart {
                w: rng.below(16),
                dx: rng.delta(),
                dy: rng.delta(),
            },
            9..12 => Step::ModifierDrag {
                w: rng.below(16),
                dx: rng.delta(),
                dy: rng.delta(),
            },
            12..15 => Step::Press { w: rng.below(16) },
            15..26 => Step::DragMove {
                dx: rng.delta(),
                dy: rng.delta(),
            },
            26..30 => Step::ModifierMove {
                dx: rng.delta(),
                dy: rng.delta(),
            },
            30..35 => Step::CrossDisplay { to: rng.below(3) },
            35..40 => Step::CrossDisplayLagging { to: rng.below(3) },
            40..48 => Step::MouseUp,
            48..60 => Step::Refresh {
                app: rng.below(3),
                stale: rng.below(3) == 0,
                zero_min: rng.below(4) == 0,
            },
            60..70 => Step::ForeignMove {
                w: rng.below(16),
                to: rng.below(3),
                mouse: match rng.below(10) {
                    0..7 => Mouse::Truth,
                    7..9 => Mouse::ReportsUp,
                    _ => Mouse::ReportsDown,
                },
            },
            70..73 => Step::HeldMovesButtonUp { dx: rng.delta() },
            73..89 => Step::Deliver,
            89..90 => Step::Tick,
            90..91 => Step::Wake,
            91..94 => Step::Sizing {
                w: rng.below(16),
                sizing: match rng.below(3) {
                    0 => TestSizing::AsAsked,
                    1 => TestSizing::AtLeast(CGSize::new(450., 350.)),
                    _ => TestSizing::Cells(7.),
                },
            },
            94..96 => Step::Destroy { w: rng.below(16) },
            96..98 => Step::MoveToWorkspace {
                w: rng.below(16),
                k: rng.below(3),
            },
            98 => Step::Focus { right: rng.below(2) == 1 },
            99 => Step::MoveFocusedToDisplay { d: rng.below(3) },
            100 => Step::Cancel,
            101 => Step::Claim { w: rng.below(16) },
            102 => Step::Unclaim { w: rng.below(16) },
            103 => Step::ReorderDisplays,
            104 => Step::Rekey { w: rng.below(16) },
            105 => Step::EnterFullscreen { w: rng.below(16) },
            106 => Step::DestroyIdentity { w: rng.below(16) },
            _ => Step::LeaveFullscreen { w: rng.below(16) },
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Simulation

#[derive(Debug, Clone, PartialEq)]
struct Violation {
    invariant: &'static str,
    step_kind: &'static str,
    step: usize,
    detail: String,
}

impl Violation {
    fn signature(&self) -> (&'static str, &'static str) { (self.invariant, self.step_kind) }
}

/// What the user is really doing, independent of what rift believes.
/// A window in native fullscreen.
#[derive(Debug, Clone)]
struct Fullscreen {
    wsid: WindowServerId,
    /// The space it came from and returns to.
    home: SpaceId,
    /// Its accessibility identity was destroyed there: what the next identity reports.
    dead: Option<WindowInfo>,
}

struct Held {
    window: WindowId,
    /// rift registered the drag when it started. Invariants about the drag apply only then;
    /// otherwise rift had no way to know.
    rift_saw_it: bool,
    assigned_at_start: Option<SpaceId>,
    /// A command ran during the drag: whether a command may move a held window is policy.
    command_during_drag: bool,
    hand: Hand,
}

/// How the user holds the window, and so where the pointer is.
#[derive(Debug, Clone, Copy)]
enum Hand {
    /// By its title bar, `grab` from the frame's origin. The user's moves carry the pointer
    /// with the frame, and the tap reports it; anyone else moving the window leaves it at
    /// `pointer`.
    Native { grab: CGPoint, pointer: CGPoint },
    /// With the modifier, the pointer at `pointer`: rift follows the pointer, not the window.
    Modifier { pointer: CGPoint },
}

impl Held {
    /// The user's hand goes down on `window`, assigned to `assigned_at_start`.
    fn new(
        window: WindowId,
        rift_saw_it: bool,
        assigned_at_start: Option<SpaceId>,
        hand: Hand,
    ) -> Self {
        Held {
            window,
            rift_saw_it,
            assigned_at_start,
            command_during_drag: false,
            hand,
        }
    }
}

/// Where along the title bar of `frame` the user grabs it, `along` from its left end (0 to 1).
fn title_bar(frame: CGRect, along: f64) -> CGPoint {
    CGPoint::new(frame.size.width * along.clamp(0.05, 0.95), 10.)
}

struct Sim {
    apps: Apps,
    reactor: Reactor,
    screens: Vec<CGRect>,
    spaces: Vec<SpaceId>,
    pids: Vec<pid_t>,
    windows: Vec<WindowId>,
    live: BTreeSet<WindowId>,
    /// What a read of each window would say now (its frame, and the request its app had
    /// applied), and what one said before its last change: a stale read reports that.
    reads: BTreeMap<WindowId, Read>,
    stale_reads: BTreeMap<WindowId, Read>,
    held: Option<Held>,
    pending: Vec<Request>,
    /// The window whose frame this step reported somewhere new.
    moved: Option<WindowId>,
    /// Before this step, for each window rift took to be settled: where its app had it as far
    /// as rift knew (acknowledged), and where it really was.
    settled_before: BTreeMap<WindowId, (CGRect, CGRect)>,
    /// The window rift took to be dragged before this step.
    dragged_before: Option<WindowId>,
    /// Windows whose unanswered writes rift has stopped waiting for: someone else moved the
    /// window, a modifier drag took it, or the deadline passed.
    gave_up: BTreeSet<WindowId>,
    /// Windows whose acknowledged frame rift stopped trusting this step (a display change, or
    /// the end of the user's drag), as the core's trace shows: written whatever it was.
    distrusted: BTreeSet<WindowId>,
    /// The animation thread, on seeds with animation on.
    presenter: Option<super::animation::TestPresenter>,
    /// What the reactor sends itself (`communication_manager.events_tx`), on those seeds.
    events: Option<crate::actor::Receiver<Event>>,
    /// Events collected from `events`, not yet handled.
    inbox: Vec<Event>,
    /// The frames Rift had pending for each window before the step: a write's or an
    /// animation's target, and a desire queued behind the write.
    in_flight_before: BTreeMap<WindowId, Vec<CGRect>>,
    /// The index the next replaced identity gets (past every window the sim starts with).
    next_idx: u32,
    /// Windows in native fullscreen, by the id they have now.
    fullscreen: BTreeMap<WindowId, Fullscreen>,
    /// The requests the core planned animations and cameras under.
    animation_requests: Vec<TransactionId>,
}

/// A read of a window: its frame, and the request its app had applied.
type Read = (CGRect, TransactionId);

/// What a case is set up with: two or three displays; on some seeds animation is on (the
/// animation thread runs in step with the case, `TestPresenter`), and on some of those the
/// first space's workspace is a scrolling layout, so every arrange there presents a camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Setup {
    displays: usize,
    animate: bool,
    scrolling: bool,
}

impl Setup {
    fn of(seed: u64) -> Self {
        let animate = (seed / 2) % 2 == 1;
        Setup {
            displays: 2 + (seed % 2) as usize,
            animate,
            scrolling: animate && (seed / 4) % 2 == 1,
        }
    }

    fn plain(displays: usize) -> Self {
        Setup {
            displays,
            animate: false,
            scrolling: false,
        }
    }
}

impl Sim {
    /// Every step the reactor takes through the core is recorded from the start
    /// (`reactor.state.core.trace`): the invariants read why rift wrote what it wrote, and the
    /// differential test replays it.
    fn new(setup: Setup) -> Self {
        let displays = setup.displays;
        let (mut apps, mut reactor) = test_context();
        reactor.state.core.trace = Some(Vec::new());
        let screens: Vec<CGRect> = (0..displays)
            .map(|i| {
                CGRect::new(
                    CGPoint::new(SCREEN_W * i as f64, 0.),
                    CGSize::new(SCREEN_W, SCREEN_H),
                )
            })
            .collect();
        let spaces: Vec<SpaceId> = (1..=displays as u64).map(SpaceId::new).collect();
        reactor.handle_event(space_state_event(
            screens.clone(),
            spaces.iter().copied().map(Some).collect(),
        ));
        let mut pids = Vec::new();
        let mut windows = Vec::new();
        for display in 0..displays {
            let pid = 7 + display as pid_t;
            let infos: Vec<WindowInfo> = (1..=2 + display % 2)
                .map(|i| {
                    make_window_info(
                        CGRect::new(
                            CGPoint::new(screens[display].origin.x + 100. * i as f64, 100.),
                            CGSize::new(300., 200.),
                        ),
                        None,
                        &format!("Window{pid}-{i}"),
                        None,
                    )
                })
                .collect();
            let count = infos.len();
            apps.make_app_and_settle(&mut reactor, pid, infos);
            for idx in 1..=count as u32 {
                let wid = WindowId::new(pid, idx);
                let wsid = reactor.test_window_server_id(wid);
                reactor.mark_test_window_visible_in_space(wsid, spaces[display]);
                windows.push(wid);
            }
            pids.push(pid);
        }
        apps.simulate_until_quiet(&mut reactor);
        let live = windows.iter().copied().collect();
        let (presenter, events) = if setup.animate {
            reactor.config.settings.animate = true;
            let (tx, rx) = super::animation::AnimationSender::channel();
            reactor.animation_tx = Some(tx);
            let (events_tx, events) = crate::actor::channel();
            reactor.communication_manager.events_tx = Some(events_tx);
            (Some(super::animation::TestPresenter::new(rx)), Some(events))
        } else {
            (None, None)
        };
        let mut sim = Sim {
            apps,
            reactor,
            screens,
            spaces,
            pids,
            windows,
            live,
            reads: BTreeMap::new(),
            stale_reads: BTreeMap::new(),
            held: None,
            pending: Vec::new(),
            moved: None,
            settled_before: BTreeMap::new(),
            dragged_before: None,
            gave_up: BTreeSet::new(),
            distrusted: BTreeSet::new(),
            presenter,
            events,
            inbox: Vec::new(),
            animation_requests: Vec::new(),
            in_flight_before: BTreeMap::new(),
            next_idx: 100,
            fullscreen: BTreeMap::new(),
        };
        if setup.scrolling {
            sim.reactor.handle_test_layout_command(LayoutCommand::SetWorkspaceLayout {
                workspace: None,
                mode: LayoutMode::Scrolling,
            });
        }
        if setup.animate {
            sim.pending.extend(sim.apps.requests());
            sim.drain().expect("the animated setup goes quiet");
            sim.read_core_steps(0);
        }
        sim
    }

    /// Runs the animation thread's queued work, then hands the reactor what it sent itself
    /// (an animation's end, a camera's finish).
    fn present(&mut self) {
        for _ in 0..20 {
            if let Some(presenter) = &mut self.presenter {
                presenter.present();
            }
            self.collect_events();
            if self.inbox.is_empty() {
                return;
            }
            for event in std::mem::take(&mut self.inbox) {
                self.reactor.handle_event(event);
            }
        }
    }

    fn collect_events(&mut self) {
        if let Some(events) = &mut self.events {
            while let Ok((_, event)) = events.try_recv() {
                self.inbox.push(event);
            }
        }
    }

    /// Nothing queued for the apps, the animation thread or the reactor.
    fn quiet(&mut self) -> bool {
        self.collect_events();
        self.pending.is_empty()
            && self.inbox.is_empty()
            && self.presenter.as_ref().is_none_or(|presenter| presenter.idle())
    }

    fn pick(&self, w: usize) -> WindowId { self.windows[w % self.windows.len()] }

    /// What an inventory reports of window `wid`: rift's record of it, at the frame the window
    /// really has. None when rift has no record (it retired or never admitted the window).
    fn reported_info(&self, wid: WindowId) -> Option<WindowInfo> {
        let mut info = self.reactor.state.windows.window(wid)?.info.clone();
        info.frame = self.os_frame(wid);
        Some(info)
    }

    /// A window id no window has had, for `wid`'s app.
    fn fresh_id(&mut self, wid: WindowId) -> WindowId {
        self.next_idx += 1;
        WindowId::new(wid.pid, self.next_idx)
    }

    /// macOS gave window `old` the id `new`: the sim follows it everywhere.
    fn rekeyed(&mut self, old: WindowId, new: WindowId) {
        for window in &mut self.windows {
            if *window == old {
                *window = new;
            }
        }
        if self.live.remove(&old) {
            self.live.insert(new);
        }
        if let Some(state) = self.apps.windows.remove(&old) {
            self.apps.windows.insert(new, state);
        }
        fn rename<V>(map: &mut BTreeMap<WindowId, V>, old: WindowId, new: WindowId) {
            if let Some(value) = map.remove(&old) {
                map.insert(new, value);
            }
        }
        rename(&mut self.reads, old, new);
        rename(&mut self.stale_reads, old, new);
        rename(&mut self.settled_before, old, new);
        rename(&mut self.in_flight_before, old, new);
        for set in [&mut self.gave_up, &mut self.distrusted] {
            if set.remove(&old) {
                set.insert(new);
            }
        }
        if self.moved == Some(old) {
            self.moved = Some(new);
        }
        if let Some(mut fullscreen) = self.fullscreen.remove(&old) {
            fullscreen.dead = None;
            self.fullscreen.insert(new, fullscreen);
        }
    }

    fn os_frame(&self, wid: WindowId) -> CGRect { self.apps.windows[&wid].frame }

    fn set_os_frame(&mut self, wid: WindowId, frame: CGRect) {
        self.note_reads();
        self.apps.windows.get_mut(&wid).unwrap().frame = frame;
    }

    /// What a read of `wid` says now.
    fn read(&self, wid: WindowId) -> Read {
        let window = &self.apps.windows[&wid];
        (window.frame, window.last_seen_txid)
    }

    /// Remembers what a read of each window said before it changed.
    fn note_reads(&mut self) {
        for &wid in &self.live {
            let now = self.read(wid);
            if let Some(&before) = self.reads.get(&wid)
                && !before.0.same_as(now.0)
            {
                self.stale_reads.insert(wid, before);
            }
            self.reads.insert(wid, now);
        }
    }

    fn report_frame(&mut self, wid: WindowId, frame: CGRect, mouse: MouseState) {
        if !self.os_frame(wid).same_as(frame) {
            self.moved = Some(wid);
        }
        self.gave_up.insert(wid);
        self.set_os_frame(wid, frame);
        let last_seen = Some(self.apps.windows[&wid].last_seen_txid);
        self.reactor.handle_event(Event::WindowFrameChanged(
            wid,
            frame,
            last_seen,
            Requested(false),
        ));
    }

    /// `wid` lands on display `to` with the button reported as `mouse`, and the window server
    /// says so.
    fn move_to_display(&mut self, wid: WindowId, to: usize, mouse: MouseState) {
        let to = to % self.screens.len();
        let frame = on_screen(self.screens[to], self.os_frame(wid));
        self.report_frame(wid, frame, mouse);
        let wsid = self.reactor.test_window_server_id(wid);
        self.reactor.handle_event(Event::WindowServerAppeared(
            wsid,
            self.spaces[to],
            SpaceEventKind::User,
        ));
    }

    /// After a violation the run looks past: stop expecting what rift already broke.
    fn accept_reality(&mut self) {
        let Some(held) = &mut self.held else { return };
        held.rift_saw_it = dragged(&self.reactor) == Some(held.window);
        held.assigned_at_start = self.reactor.assigned_space_for_window_id(held.window);
    }

    /// Whether the user can press `wid`: it is live and its middle is on a screen. (Before an
    /// animation's frames reach it, a window can still be where a drag left it, off them all.)
    fn pressable(&self, wid: WindowId) -> bool {
        // A native fullscreen window is not dragged around the desktop.
        self.live.contains(&wid) && !self.fullscreen.contains_key(&wid) && {
            let mid = self.os_frame(wid).mid();
            self.screens.iter().any(|screen| screen.contains(mid))
        }
    }

    fn held_window(&self) -> Option<WindowId> { self.held.as_ref().map(|held| held.window) }

    /// Where the user's pointer is, while they hold a window.
    fn pointer(&self) -> Option<CGPoint> {
        self.held.as_ref().map(|held| match held.hand {
            Hand::Native { pointer, .. } | Hand::Modifier { pointer } => pointer,
        })
    }

    /// The user moved the window they hold natively: the pointer goes with its title bar, and
    /// the tap reports it.
    fn follow_pointer(&mut self) {
        let Some(held) = &mut self.held else { return };
        let Hand::Native { grab, pointer } = &mut held.hand else {
            return;
        };
        let frame = self.apps.windows[&held.window].frame;
        *pointer = CGPoint::new(frame.origin.x + grab.x, frame.origin.y + grab.y);
        let point = *pointer;
        self.reactor
            .handle_event(Event::DragMotion(crate::actor::drag::DragMotion { point }));
    }

    fn true_mouse(&self) -> MouseState {
        if self.held.is_some() {
            MouseState::Down
        } else {
            MouseState::Up
        }
    }

    /// Runs one step. Returns the requests rift emitted while handling it.
    fn run(&mut self, step: Step) -> Vec<Request> {
        self.note_reads();
        self.moved = None;
        self.distrusted.clear();
        let traced = self.core_steps();
        self.dragged_before = dragged(&self.reactor);
        self.in_flight_before = self
            .live
            .iter()
            .filter_map(|&wid| match self.reactor.state.core.sync(wid)? {
                Sync::Animating { target, .. } => Some((wid, vec![target])),
                Sync::InFlight { target, queued, .. } => {
                    Some((wid, std::iter::once(target).chain(queued).collect()))
                }
                Sync::Settled { .. }
                | Sync::Unknown { .. }
                | Sync::Refused { .. }
                | Sync::Outside { .. } => None,
            })
            .collect();
        self.settled_before = self
            .live
            .iter()
            .filter_map(|&wid| match self.reactor.state.core.sync(wid)? {
                Sync::Settled { acked, .. } => Some((wid, (acked, self.os_frame(wid)))),
                Sync::Unknown { .. }
                | Sync::InFlight { .. }
                | Sync::Refused { .. }
                | Sync::Animating { .. }
                | Sync::Outside { .. } => None,
            })
            .collect();
        match step {
            Step::DragStart { w, dx, dy } => {
                let wid = self.pick(w);
                if self.held.is_none() && self.pressable(wid) {
                    let assigned_at_start = self.reactor.assigned_space_for_window_id(wid);
                    let frame = offset(self.os_frame(wid), dx, dy);
                    let grab = title_bar(frame, (f64::from(dx) + 150.) / 300.);
                    press(&mut self.reactor, wid);
                    let pointer = frame.origin;
                    self.held = Some(Held::new(wid, false, assigned_at_start, Hand::Native {
                        grab,
                        pointer,
                    }));
                    self.report_frame(wid, frame, MouseState::Down);
                    self.follow_pointer();
                    if let Some(held) = &mut self.held {
                        held.rift_saw_it = dragged(&self.reactor) == Some(wid);
                    }
                }
            }
            Step::Press { w } => {
                let wid = self.pick(w);
                if self.held.is_none() && self.pressable(wid) {
                    press(&mut self.reactor, wid);
                    // Nothing has moved yet, so there is no drag for rift to have seen.
                    let assigned_at_start = self.reactor.assigned_space_for_window_id(wid);
                    let frame = self.os_frame(wid);
                    let grab = title_bar(frame, 0.5);
                    let pointer = CGPoint::new(frame.origin.x + grab.x, frame.origin.y + grab.y);
                    self.held = Some(Held::new(wid, false, assigned_at_start, Hand::Native {
                        grab,
                        pointer,
                    }));
                }
            }
            Step::ModifierDrag { w, dx, dy } => {
                let wid = self.pick(w);
                if self.held.is_none() && self.pressable(wid) {
                    let point = self.os_frame(wid).mid();
                    self.reactor.handle_event(Event::ModifierMouseDown {
                        button: crate::actor::drag::MouseButton::Left,
                        point,
                        action: crate::common::config::MouseAction::Move,
                    });
                    // The windows overlap: the user holds whichever one is on top at the point,
                    // which is the one rift took (if it manages one there).
                    if let Some(gripped) = self.reactor.state.windows.gripped() {
                        let assigned_at_start = self.reactor.assigned_space_for_window_id(gripped);
                        let rift_saw_it = dragged(&self.reactor) == Some(gripped);
                        let point = CGPoint::new(point.x + f64::from(dx), point.y + f64::from(dy));
                        self.held = Some(Held::new(
                            gripped,
                            rift_saw_it,
                            assigned_at_start,
                            Hand::Modifier { pointer: point },
                        ));
                        self.reactor.handle_event(Event::DragMotion(
                            crate::actor::drag::DragMotion { point },
                        ));
                    }
                }
            }
            Step::ModifierMove { dx, dy } => {
                if let Some(held) = &mut self.held
                    && let Hand::Modifier { pointer: point } = &mut held.hand
                {
                    *point = CGPoint::new(point.x + f64::from(dx), point.y + f64::from(dy));
                    let point = *point;
                    self.reactor
                        .handle_event(Event::DragMotion(crate::actor::drag::DragMotion { point }));
                }
            }
            Step::Wake => {
                let spaces = self.spaces.iter().copied().map(Some).collect();
                self.reactor.handle_event(space_state_event_with(
                    self.screens.clone(),
                    spaces,
                    |state| state.should_force_refresh_layout = true,
                ));
            }
            Step::DragMove { dx, dy } => {
                if let Some(wid) = self.held_window() {
                    let frame = offset(self.os_frame(wid), dx, dy);
                    self.report_frame(wid, frame, MouseState::Down);
                    self.follow_pointer();
                    // A pressed window's first move is where its drag starts.
                    if let Some(held) = &mut self.held
                        && !held.rift_saw_it
                        && dragged(&self.reactor) == Some(wid)
                    {
                        held.rift_saw_it = true;
                        held.assigned_at_start = self.reactor.assigned_space_for_window_id(wid);
                    }
                }
            }
            Step::CrossDisplay { to } => {
                if let Some(wid) = self.held_window() {
                    self.move_to_display(wid, to, MouseState::Down);
                    self.follow_pointer();
                }
            }
            Step::CrossDisplayLagging { to } => {
                if let Some(Held {
                    window,
                    hand: Hand::Native { grab, pointer: now },
                    ..
                }) = self.held
                {
                    let screen = self.screens[to % self.screens.len()];
                    let frame = self.os_frame(window);
                    // Just across the edge of `to` nearest the pointer.
                    let pointer = CGPoint::new(
                        now.x.clamp(screen.origin.x + 5., screen.max().x - 5.),
                        now.y.clamp(screen.origin.y + 5., screen.max().y - 5.),
                    );
                    let frame = CGRect::new(
                        CGPoint::new(pointer.x - grab.x, pointer.y - grab.y),
                        frame.size,
                    );
                    self.report_frame(window, frame, MouseState::Down);
                    self.follow_pointer();
                    let wsid = self.reactor.test_window_server_id(window);
                    self.reactor.handle_event(Event::WindowServerAppeared(
                        wsid,
                        self.spaces[to % self.spaces.len()],
                        SpaceEventKind::User,
                    ));
                }
            }
            Step::MouseUp => {
                self.held = None;
                self.reactor
                    .handle_event(Event::PointerReleased(crate::actor::drag::MouseButton::Left));
            }
            Step::Tick => {
                self.gave_up.extend(self.live.iter().copied());
                self.reactor.test_expire_all_writes();
            }
            Step::Sizing { w, sizing } => {
                let wid = self.pick(w);
                if let Some(window) = self.apps.windows.get_mut(&wid) {
                    window.sizing = sizing;
                }
            }
            Step::Refresh { app, stale, zero_min } => {
                self.gave_up.extend(self.held_window());
                let pid = self.pids[app % self.pids.len()];
                // A destroyed identity is not in the app's inventory.
                let dead = |wid: &WindowId| {
                    self.fullscreen.get(wid).is_some_and(|fullscreen| fullscreen.dead.is_some())
                };
                let mine: Vec<WindowId> =
                    self.live.iter().copied().filter(|wid| wid.pid == pid && !dead(wid)).collect();
                self.note_reads();
                // Each read is stamped with the request the app had applied when it was made.
                let mut as_of = HashMap::default();
                let new = mine
                    .iter()
                    .filter_map(|&wid| {
                        let mut info = self.reactor.state.windows.window(wid)?.info.clone();
                        let (frame, applied) = match self.stale_reads.get(&wid) {
                            Some(&read) if stale => read,
                            Some(_) | None => self.read(wid),
                        };
                        info.frame = frame;
                        if applied != TransactionId::default() {
                            as_of.insert(wid, applied);
                        }
                        if zero_min {
                            info.min_size = Some(CGSize::ZERO);
                        }
                        Some((wid, info))
                    })
                    .collect();
                self.reactor.discover_test_windows_as_of(pid, new, mine, as_of);
            }
            Step::ForeignMove { w, to, mouse } => {
                let wid = self.pick(w);
                if self.live.contains(&wid)
                    && self.held_window() != Some(wid)
                    && !self.fullscreen.contains_key(&wid)
                {
                    let reported = match mouse {
                        Mouse::Truth => self.true_mouse(),
                        Mouse::ReportsUp => MouseState::Up,
                        Mouse::ReportsDown => MouseState::Down,
                    };
                    self.move_to_display(wid, to, reported);
                }
            }
            Step::HeldMovesButtonUp { dx } => {
                if let Some(wid) = self.held_window() {
                    let frame = offset(self.os_frame(wid), dx, 0);
                    self.report_frame(wid, frame, MouseState::Up);
                    self.follow_pointer();
                    // rift was told the button is up, so letting go is the best it can do.
                    // The user still holds the window; rift just can't know that any more.
                    if dragged(&self.reactor) != Some(wid)
                        && let Some(held) = &mut self.held
                    {
                        held.rift_saw_it = false;
                    }
                }
            }
            Step::Deliver => {
                self.gave_up.clear();
                let requests = std::mem::take(&mut self.pending)
                    .into_iter()
                    // The simulator can't raise or close windows (todo!); neither affects drags.
                    .filter(|request| {
                        !matches!(request, Request::Raise(..) | Request::CloseWindow(..))
                    })
                    .collect();
                // Echoes carry no button state. The reactor's own button read is injected and
                // answers "unknown" in tests, so nothing here reads this machine's real mouse.
                for event in self.apps.simulate_events_for_requests(requests) {
                    self.reactor.handle_event(event);
                }
            }
            Step::Cancel => {
                // Escape while holding, then the user lets go (no new press with the button
                // still down).
                self.reactor.handle_event(Event::DragCancel);
                if self.held.take().is_some() {
                    self.reactor.handle_event(Event::PointerReleased(
                        crate::actor::drag::MouseButton::Left,
                    ));
                }
            }
            Step::Claim { w } => {
                let wid = self.pick(w);
                if self.live.contains(&wid) && self.held_window() != Some(wid) {
                    _ = self.reactor.claim_window(
                        wid,
                        FUZZ_OWNER,
                        rift_protocol::WindowClaimFlags::empty(),
                    );
                }
            }
            Step::Unclaim { w } => {
                let wid = self.pick(w);
                if self.live.contains(&wid) {
                    _ = self.reactor.release_window(wid, FUZZ_OWNER);
                }
            }
            Step::ReorderDisplays => {
                if self.screens.len() > 1 {
                    self.screens.rotate_left(1);
                    self.spaces.rotate_left(1);
                    let spaces = self.spaces.iter().copied().map(Some).collect();
                    self.reactor.handle_event(space_state_event(self.screens.clone(), spaces));
                }
            }
            Step::Rekey { w } => {
                let wid = self.pick(w);
                let dead = self.fullscreen.get(&wid).and_then(|fullscreen| fullscreen.dead.clone());
                if self.live.contains(&wid)
                    && self.held_window() != Some(wid)
                    && let Some(info) = dead.or_else(|| self.reported_info(wid))
                {
                    // macOS announces the new identity as a new window (same server window).
                    let fresh = self.fresh_id(wid);
                    self.reactor.handle_event(Event::WindowCreated(fresh, info, None));
                    self.rekeyed(wid, fresh);
                }
            }
            Step::EnterFullscreen { w } => {
                let wid = self.pick(w);
                if self.live.contains(&wid)
                    && self.held_window() != Some(wid)
                    && !self.fullscreen.contains_key(&wid)
                    && let Some(home) = self.reactor.assigned_space_for_window_id(wid)
                    && self.spaces.contains(&home)
                {
                    let wsid = self.reactor.test_window_server_id(wid);
                    let space = SpaceId::new(0x400000000 + home.get());
                    super::window_server_appeared(
                        &mut self.reactor,
                        wsid,
                        space,
                        SpaceEventKind::Fullscreen,
                    );
                    self.reactor.state.windows.set_window_server_space(wsid, Some(space));
                    self.fullscreen.insert(wid, Fullscreen { wsid, home, dead: None });
                }
            }
            Step::DestroyIdentity { w } => {
                let wid = self.pick(w);
                if let Some(fullscreen) = self.fullscreen.get(&wid)
                    && fullscreen.dead.is_none()
                    && let Some(info) = self.reported_info(wid)
                {
                    self.reactor.handle_event(Event::WindowDestroyed(wid));
                    if let Some(fullscreen) = self.fullscreen.get_mut(&wid) {
                        fullscreen.dead = Some(info);
                    }
                }
            }
            Step::LeaveFullscreen { w } => {
                let wid = self.pick(w);
                if let Some(Fullscreen { wsid, home, .. }) = self.fullscreen.remove(&wid) {
                    super::window_server_appeared(
                        &mut self.reactor,
                        wsid,
                        home,
                        SpaceEventKind::User,
                    );
                }
            }
            Step::Destroy { w } => {
                let wid = self.pick(w);
                if !self.fullscreen.contains_key(&wid) && self.live.remove(&wid) {
                    if self.held_window() == Some(wid) {
                        self.held = None;
                    }
                    self.apps.windows.remove(&wid);
                    self.reactor.handle_event(Event::WindowDestroyed(wid));
                }
            }
            Step::MoveToWorkspace { w, k } => {
                if let Some(held) = &mut self.held {
                    held.command_during_drag = true;
                }
                self.reactor.handle_test_layout_command(LayoutCommand::MoveWindowToWorkspace {
                    workspace: WorkspaceSelector::Index(k),
                    follow: false,
                    window_id: Some(self.pick(w).idx.get()),
                });
            }
            Step::Focus { right } => {
                if let Some(held) = &mut self.held {
                    held.command_during_drag = true;
                }
                self.reactor.handle_test_layout_command(LayoutCommand::MoveFocus(if right {
                    Direction::Right
                } else {
                    Direction::Left
                }));
            }
            Step::MoveFocusedToDisplay { d } => {
                if let Some(held) = &mut self.held {
                    held.command_during_drag = true;
                }
                self.reactor.handle_loop_event(Event::Command(Command::Reactor(
                    ReactorCommand::MoveWindowToDisplay {
                        selector: DisplaySelector::Index(d % self.screens.len()),
                        window_id: None,
                    },
                )));
            }
        }
        // Each step is one batch of the reactor's loop, which flushes when it drains.
        self.reactor.reread_windows();
        // The animation thread takes what the step queued within a display frame, long before
        // the next input: run it now (its frames reach the apps on a later Deliver, behind
        // what was already queued for them).
        self.present();
        self.read_core_steps(traced);
        self.apps.requests()
    }

    fn core_steps(&self) -> usize { self.reactor.state.core.trace.as_ref().map_or(0, Vec::len) }

    /// What the core decided since step `from` that the invariants need: a modifier drag's
    /// interactive frame, or the core forgetting a window, ends rift's wait for it; and a
    /// window whose acknowledged frame rift stopped trusting (a display change, a drag's end) is
    /// written whatever it was.
    fn read_core_steps(&mut self, from: usize) {
        let trace = &self.reactor.state.core.trace;
        for (event, effects) in &trace.as_ref().expect("recording")[from..] {
            match event {
                // A modifier drag took the window, or something outside the core (a topology
                // change, an animation) did: either way rift stopped waiting for its write.
                rift_core::Event::Follow { window, .. } | rift_core::Event::Forget { window } => {
                    _ = self.gave_up.insert(*window)
                }
                rift_core::Event::Distrust { windows } => self.distrusted.extend(windows),
                // An animation that takes a window over supersedes the write it had in flight.
                rift_core::Event::Animate { desires, .. } => {
                    self.animation_requests.extend(desires.iter().map(|desire| desire.request));
                    self.gave_up.extend(effects.iter().filter_map(|effect| match effect {
                        rift_core::Effect::Animate { window, .. } => Some(*window),
                        _ => None,
                    }))
                }
                _ if effects
                    .iter()
                    .any(|effect| matches!(effect, rift_core::Effect::Ungrip { .. })) =>
                {
                    self.distrusted.extend(effects.iter().flat_map(|effect| match effect {
                        rift_core::Effect::Ungrip { window, changed, .. } => {
                            std::iter::once(*window).chain(changed.iter().copied()).collect()
                        }
                        _ => vec![],
                    }))
                }
                _ => {}
            }
        }
    }

    fn check(&self, step_index: usize, step: Step, emitted: &[Request]) -> Result<(), Violation> {
        let fail = |invariant, detail: String| {
            Err(Violation {
                invariant,
                step_kind: step.kind(),
                step: step_index,
                detail,
            })
        };
        // A camera's one size write at its start goes out under the request the core planned
        // the camera under: part of that presentation, not a write of its own.
        let writes: Vec<_> = emitted
            .iter()
            .flat_map(|request| match request {
                Request::SetWindowFrames(frames, txid, ..)
                    if !self.animation_requests.contains(txid) =>
                {
                    frames.clone()
                }
                _ => Vec::new(),
            })
            .collect();
        // 1. Liveness: a pressed, live window that moved under the pointer is the drag. Without
        // this, a drag that fails to start would quietly skip every invariant below.
        // Exceptions: a step that leaves the frame where it was moves nothing; and a move that
        // lands on a frame rift has pending for the window (a write's or an animation's target,
        // a desire queued behind the write) is, by geometry, its acknowledgement (the next move
        // starts the drag).
        if matches!(
            step,
            Step::DragStart { .. }
                | Step::DragMove { .. }
                | Step::CrossDisplay { .. }
                | Step::CrossDisplayLagging { .. }
        ) && let Some(held) = &self.held
            && self.live.contains(&held.window)
            // A claimed window is its owner's: rift does not drag it.
            && self.reactor.state.windows.claim(held.window).is_none()
            && self.moved == Some(held.window)
            && !frames_written(&self.pending).iter().any(|(wid, frame)| {
                *wid == held.window && frame.same_as(self.os_frame(held.window))
            })
            && !self
                .in_flight_before
                .get(&held.window)
                .is_some_and(|frames| frames.iter().any(|f| f.same_as(self.os_frame(held.window))))
            && dragged(&self.reactor) != Some(held.window)
        {
            return fail(
                "press-and-move-starts-a-drag",
                format!(
                    "the user pressed and moved {:?} but rift's drag is {:?}",
                    held.window,
                    dragged(&self.reactor)
                ),
            );
        }
        if let Some(held) = self.held.as_ref().filter(|held| held.rift_saw_it) {
            let wid = held.window;
            // 2. While the button is down, the drag stays on the window the user holds. The
            // tap-drag lie (button reported up) clears `rift_saw_it` instead.
            let current = dragged(&self.reactor);
            if current != Some(wid) {
                return fail(
                    "drag-stays-on-held-window",
                    format!("the user drags {wid:?} but rift's drag is now {current:?}"),
                );
            }
            // 3. The dragged window keeps its assignment until the drop.
            let now = self.reactor.assigned_space_for_window_id(wid);
            if now != held.assigned_at_start {
                let invariant = if held.command_during_drag {
                    "held-window-keeps-assignment (after a command)"
                } else {
                    "held-window-keeps-assignment"
                };
                return fail(
                    invariant,
                    format!(
                        "{wid:?} moved from {:?} to {now:?} mid-drag",
                        held.assigned_at_start
                    ),
                );
            }
        }
        // 3b. A drag drops onto the display under the pointer: whatever it would drop onto is
        // there, and off every display there is nothing to drop onto.
        if let Some(held) = self.held.as_ref().filter(|held| held.rift_saw_it)
            && let Some(session) = self.reactor.state.windows.drag_on(held.window)
            && let Some(intent) = session.intent()
        {
            let under = self.pointer().and_then(|pointer| {
                self.screens
                    .iter()
                    .position(|screen| screen.contains(pointer))
                    .map(|display| self.spaces[display])
            });
            if under != Some(intent.space) {
                return fail(
                    "drop-target-on-pointer-display",
                    format!(
                        "the pointer is at {:?}, on {under:?}, but the drag of {:?} would drop \
                         onto {intent:?}",
                        self.pointer(),
                        held.window
                    ),
                );
            }
        }
        // 4. A drag does not change the layout: from press to release rift writes nothing to
        // the window the user holds, and while it takes a window to be dragged it writes no
        // frame at all, not even for a window going away or a command, and sends no
        // interactive frames (the viewport's; the fuzzer's drags are native, so they have none
        // of their own). The drop is what writes.
        if let Some(held) = &self.held
            && self.reactor.state.windows.is_gripped(held.window)
            && let Some((_, frame)) = writes.iter().find(|(w, _)| *w == held.window)
        {
            return fail(
                "no-write-to-held-window",
                format!(
                    "rift wrote {:?} to {frame:?} while the user held it",
                    held.window
                ),
            );
        }
        if self.dragged_before.is_some() && dragged(&self.reactor).is_some() {
            if let Some((wid, frame)) = writes.first() {
                return fail(
                    "no-writes-during-drag",
                    format!(
                        "rift wrote {wid:?} to {frame:?} during the drag of {:?}",
                        self.dragged_before
                    ),
                );
            }
            // A modifier drag moves its own window by interactive frames, and nothing else
            // presents the held window. Without animation those are the only interactive frames;
            // with it, a layout animation or a camera may present the other windows.
            let held = self.held.as_ref().map(|held| held.window);
            let modifier = self
                .held
                .as_ref()
                .filter(|held| matches!(held.hand, Hand::Modifier { .. }))
                .map(|held| held.window);
            let animated = self.presenter.is_some();
            let presentable = |w: WindowId| Some(w) == modifier || (animated && Some(w) != held);
            if emitted.iter().any(|request| match request {
                Request::InteractiveFramesPending(frames) => {
                    frames.windows().into_iter().any(|w| !presentable(w))
                }
                _ => false,
            }) {
                return fail(
                    "no-writes-during-drag",
                    format!(
                        "rift sent interactive frames during the drag of {:?}",
                        self.dragged_before
                    ),
                );
            }
        }
        // 5. Releasing the button ends the drag.
        if matches!(step, Step::MouseUp) && user_dragging(&self.reactor) {
            return fail(
                "mouse-up-ends-drag",
                format!("still dragging {:?} after mouse-up", dragged(&self.reactor)),
            );
        }
        let unanswered = frames_written(&self.pending);
        let mut written_once = BTreeSet::new();
        for &(wid, frame) in &writes {
            if !self.live.contains(&wid) {
                continue;
            }
            // 6. Only the reconciler writes: what rift sent is what the reconciler has in
            // flight. (A write it stopped waiting for within the same step is the exception.)
            match self.reactor.state.core.sync(wid) {
                Some(Sync::InFlight { target, .. }) => {
                    if !frame.same_as(target) && !self.gave_up.contains(&wid) {
                        return fail(
                            "only-the-reconciler-writes",
                            format!("{wid:?} was sent {frame:?}; in flight is {target:?}"),
                        );
                    }
                }
                None
                | Some(
                    Sync::Settled { .. }
                    | Sync::Refused { .. }
                    | Sync::Unknown { .. }
                    | Sync::Animating { .. }
                    | Sync::Outside { .. },
                ) => {
                    if self.moved != Some(wid)
                        && !self.gave_up.contains(&wid)
                        && !matches!(step, Step::Tick)
                    {
                        return fail(
                            "only-the-reconciler-writes",
                            format!("{wid:?} was sent {frame:?} with nothing in flight"),
                        );
                    }
                }
            }
            // 7. At most one write to a window is with its app.
            let over_unanswered = unanswered.iter().any(|(w, _)| *w == wid);
            if !written_once.insert(wid) || (over_unanswered && !self.gave_up.contains(&wid)) {
                return fail(
                    "one-in-flight-per-window",
                    format!("{wid:?} was sent {frame:?} over an unanswered write"),
                );
            }
            // 8. No write to a window that was already there, and that rift knew to be there.
            // (Commands place windows whatever rift believes; a refresh or a move changes what
            // it believes, and so does an answer showing the app moved it during the step.)
            let placed = matches!(
                step,
                Step::MoveFocusedToDisplay { .. }
                    | Step::Focus { .. }
                    | Step::MoveToWorkspace { .. }
                    | Step::Refresh { .. }
            );
            if !placed
                && !self.distrusted.contains(&wid)
                && self.moved != Some(wid)
                && let Some((known, real)) = self.settled_before.get(&wid)
                && known.same_as(frame)
                && real.same_as(frame)
                && self.os_frame(wid).same_as(frame)
            {
                return fail(
                    "no-write-when-settled",
                    format!("{wid:?} was already at {frame:?}"),
                );
            }
        }
        // 10. The window store's frames are the core's projection: every window the store has
        // a frame for is seen there by the core.
        for &wid in &self.live {
            let projected = self.reactor.state.windows.window(wid).map(|w| w.frame_monotonic());
            let seen = self.reactor.state.core.seen(wid);
            if projected != seen {
                return fail(
                    "frames-are-the-cores",
                    format!("{wid:?}: the store has {projected:?}, the core sees {seen:?}"),
                );
            }
        }
        // 11. The pending store the app actors read is the core's projection too.
        for &wid in &self.live {
            let Some(wsid) = self.reactor.state.windows.server_id(wid) else {
                continue;
            };
            let stored = self.reactor.state.tx.target(wsid);
            let pending = self.reactor.state.core.pending(wid);
            if stored != pending {
                return fail(
                    "pending-is-the-cores",
                    format!("{wid:?}: the store has {stored:?} pending, the core {pending:?}"),
                );
            }
        }
        // 9. Every live window stays assigned to some space (a claimed one is its owner's, a
        // native fullscreen one is in its fullscreen space).
        for &wid in &self.live {
            if self.reactor.state.windows.claim(wid).is_none()
                && !self.fullscreen.contains_key(&wid)
                && self.reactor.assigned_space_for_window_id(wid).is_none()
            {
                return fail(
                    "live-window-has-a-space",
                    format!("{wid:?} has no assigned space"),
                );
            }
        }
        Ok(())
    }
}

impl Sim {
    /// Delivers requests and answers until there are none left.
    fn drain(&mut self) -> Result<(), String> {
        for _ in 0..20 {
            let emitted = self.run(Step::Deliver);
            self.pending.extend(emitted);
            if self.quiet() {
                return Ok(());
            }
        }
        Err(format!("rift and its apps never went quiet: {:?}", self.pending))
    }

    /// Quiescence: the user lets go, every answer arrives, a truthful inventory of every app
    /// comes in, every deadline passes. Then every window layout places, and that did not
    /// refuse its frame, is where layout wants it. (The truthful inventory is a precondition:
    /// when the last thing rift heard about a window was a lagging read, only re-reading it
    /// can tell, which is the full reconcile rift does not do yet.)
    /// A window two spaces' layouts both place has no one desired frame and is left out
    /// (`a_window_dropped_on_another_display_leaves_the_old_displays_layout`).
    fn settle(&mut self, step: usize) -> Result<(), Violation> {
        let fail = |detail: String| Violation {
            invariant: "quiescence",
            step_kind: "settle",
            step,
            detail,
        };
        let emitted = self.run(Step::MouseUp);
        self.pending.extend(emitted);
        self.drain().map_err(fail)?;
        for app in 0..self.pids.len() {
            let emitted = self.run(Step::Refresh {
                app,
                stale: false,
                zero_min: false,
            });
            self.pending.extend(emitted);
        }
        self.drain().map_err(fail)?;
        let emitted = self.run(Step::Tick);
        self.pending.extend(emitted);
        self.drain().map_err(fail)?;
        let layouts = LayoutManager::calculate_layout(&mut self.reactor, None);
        for (_, layout) in &layouts {
            let desired = Desired::of(layout.clone());
            for &(wid, frame) in desired.frames() {
                if !self.live.contains(&wid) {
                    continue;
                }
                if claims(&layouts, wid) != 1 {
                    return Err(fail(format!("{wid:?} is in more than one layout")));
                }
                let sync = self.reactor.state.core.sync(wid);
                let observed = self.os_frame(wid);
                let refused = match sync {
                    Some(Sync::Refused { .. }) => true,
                    None
                    | Some(
                        Sync::Settled { .. }
                        | Sync::InFlight { .. }
                        | Sync::Unknown { .. }
                        | Sync::Animating { .. }
                        | Sync::Outside { .. },
                    ) => false,
                };
                // Where layout wants it as near as the core takes an answer to be (an app
                // rounding to whole cells lands a fraction short).
                if !refused && !observed.is_within(rift_core::reconciler::ACK_TOLERANCE, frame) {
                    return Err(fail(format!(
                        "{wid:?} is at {observed:?}, layout wants {frame:?} ({sync:?})"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// How many spaces' layouts place `wid`.
fn claims(layouts: &[(SpaceId, Vec<(WindowId, CGRect)>)], wid: WindowId) -> usize {
    layouts
        .iter()
        .filter(|(_, layout)| layout.iter().any(|(w, _)| *w == wid))
        .count()
}

fn offset(frame: CGRect, dx: i16, dy: i16) -> CGRect {
    CGRect::new(
        CGPoint::new(frame.origin.x + f64::from(dx), frame.origin.y + f64::from(dy)),
        frame.size,
    )
}

/// The frame moved onto `screen`, keeping its offset within a display.
fn on_screen(screen: CGRect, frame: CGRect) -> CGRect {
    let x = frame.origin.x.rem_euclid(SCREEN_W).min(SCREEN_W - 50.);
    CGRect::new(
        CGPoint::new(screen.origin.x + x, frame.origin.y.clamp(0., SCREEN_H - 50.)),
        frame.size,
    )
}

type Signature = (&'static str, &'static str);

/// One shrunk failure: the seed that found it, the steps, the violation, and the signatures it
/// had to look past to get there.
type Finding = (u64, Vec<Step>, Violation, Vec<Signature>);

/// Runs `steps` from a fresh setup and returns the first violation whose signature is not in
/// `past`. A violation in `past` is noted and the run carries on from rift's actual state, so
/// one common bug doesn't hide the ones behind it. A panic is a violation.
fn execute(setup: Setup, steps: &[Step], past: &[Signature]) -> Result<(), Violation> {
    let mut step_index = 0;
    let mut current: Option<Step> = None;
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let mut sim = Sim::new(setup);
        for (i, &step) in steps.iter().enumerate() {
            step_index = i;
            current = Some(step);
            let emitted = sim.run(step);
            if let Err(violation) = sim.check(i, step, &emitted) {
                if !past.contains(&violation.signature()) {
                    return Err(violation);
                }
                sim.accept_reality();
            }
            sim.pending.extend(emitted);
        }
        step_index = steps.len();
        current = None;
        match sim.settle(steps.len()) {
            Err(violation) if !past.contains(&violation.signature()) => Err(violation),
            Err(_) | Ok(()) => Ok(()),
        }
    }));
    outcome.unwrap_or_else(|panic| {
        let message = panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default();
        Err(Violation {
            invariant: "no-panic",
            step_kind: current.map_or("setup-or-settle", |step| step.kind()),
            step: step_index,
            detail: message,
        })
    })
}

/// Drops steps while the same signature still fails.
fn shrink(
    setup: Setup,
    mut steps: Vec<Step>,
    target: Signature,
    past: &[Signature],
) -> (Vec<Step>, Violation) {
    let mut violation = execute(setup, &steps, past).unwrap_err();
    steps.truncate(violation.step + 1);
    loop {
        let mut progressed = false;
        let mut i = 0;
        while i < steps.len() {
            let mut candidate = steps.clone();
            candidate.remove(i);
            match execute(setup, &candidate, past) {
                Err(v) if v.signature() == target => {
                    candidate.truncate(v.step + 1);
                    steps = candidate;
                    violation = v;
                    progressed = true;
                }
                _ => i += 1,
            }
        }
        if !progressed {
            return (steps, violation);
        }
    }
}

fn budget(var: &str, default: usize) -> usize {
    std::env::var(var).ok().and_then(|value| value.parse().ok()).unwrap_or(default)
}

/// Runs the seeds and returns one shrunk failure per signature, ordered by signature. Each
/// round looks past every signature found so far, until a round finds nothing new.
fn fuzz(cases: usize, len: usize) -> BTreeMap<Signature, Finding> {
    let quiet = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut found: BTreeMap<Signature, Finding> = BTreeMap::new();
    loop {
        let past: Vec<Signature> = found.keys().copied().collect();
        for seed in 0..cases as u64 {
            let setup = Setup::of(seed);
            let steps = generate(seed, len);
            if let Err(violation) = execute(setup, &steps, &past) {
                let signature = violation.signature();
                if !found.contains_key(&signature) {
                    let (steps, violation) = shrink(setup, steps, signature, &past);
                    found.insert(signature, (seed, steps, violation, past.clone()));
                }
            }
        }
        if found.len() == past.len() {
            break;
        }
    }
    std::panic::set_hook(quiet);
    found
}

fn describe(found: &BTreeMap<Signature, Finding>) -> String {
    found
        .values()
        .map(|(seed, steps, violation, past)| {
            format!(
                "\n[{} after {}] seed {seed}, {:?}: {}\n  execute({:?}, &{steps:?}, &{past:?})",
                violation.invariant,
                violation.step_kind,
                Setup::of(*seed),
                violation.detail,
                Setup::of(*seed),
            )
        })
        .collect()
}

#[test]
fn drag_invariants_hold_across_interleavings() {
    let found = fuzz(budget("RIFT_FUZZ_CASES", 200), budget("RIFT_FUZZ_STEPS", 30));
    let unknown: BTreeMap<_, _> = found
        .iter()
        .filter(|(signature, _)| !KNOWN.contains(signature))
        .map(|(k, v)| (*k, v.clone()))
        .collect();
    let fixed: Vec<_> = KNOWN.iter().filter(|signature| !found.contains_key(signature)).collect();
    eprintln!("fuzz: {} signature(s) found{}", found.len(), describe(&found));
    assert!(
        unknown.is_empty(),
        "new failure signature(s):{}",
        describe(&unknown)
    );
    // A known signature that no longer reproduces at the default budget may be fixed: say so,
    // but don't fail, since a smaller budget can simply miss it.
    if !fixed.is_empty() {
        eprintln!("fuzz: known signature(s) not reproduced this run: {fixed:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// The core boundary: every seed's run, as the steps the reactor took through the core.

type PlainStep = (
    rift_core::Event<rift_core::plain::Plain>,
    Vec<rift_core::Effect<rift_core::plain::Plain>>,
);

/// The steps the reactor took through the core on one case, on plain data.
fn trace(setup: Setup, steps: &[Step]) -> Vec<PlainStep> {
    let mut sim = Sim::new(setup);
    let epoch = sim.reactor.state.core.clock.now();
    for &step in steps {
        let emitted = sim.run(step);
        sim.pending.extend(emitted);
    }
    _ = sim.settle(steps.len());
    let to = Plainly { epoch };
    let recorded = sim.reactor.state.core.trace.take().unwrap_or_default();
    recorded
        .into_iter()
        .map(|(event, effects)| {
            (
                event.translate(&to),
                effects.into_iter().map(|e| e.translate(&to)).collect(),
            )
        })
        .collect()
}

#[test]
fn the_core_on_plain_data_decides_what_the_reactor_decided() {
    // The reactor runs the core on its own types; replayed on plain data (what the Roc core
    // sees), every step must decide exactly the same effects, and survive the wire.
    use rift_core::plain::wire;
    let (cases, len) = (
        budget("RIFT_FUZZ_CASES", 200) as u64,
        budget("RIFT_FUZZ_STEPS", 30),
    );
    for seed in 0..cases {
        let mut core = rift_core::RustCore::<rift_core::plain::Plain>::default();
        for (index, (event, recorded)) in
            trace(Setup::of(seed), &generate(seed, len)).into_iter().enumerate()
        {
            assert_eq!(
                wire::decode_event(&wire::encode_event(event.clone())).as_ref(),
                Ok(&event)
            );
            assert_eq!(
                wire::decode_effects(&wire::encode_effects(recorded.clone())).as_ref(),
                Ok(&recorded)
            );
            assert_eq!(
                core.step(event.clone()),
                recorded,
                "seed {seed} step {index}: {event:?}"
            );
        }
    }
}

/// Writes every seed's trace, in the wire format, to `$RIFT_TRACE_DIR/seed-<n>.trace`, for the
/// Roc core's differential test (`RIFT_FUZZ_CASES`/`RIFT_FUZZ_STEPS` set the corpus):
/// `RIFT_TRACE_DIR=… cargo test dump_core_traces -- --ignored`.
#[test]
#[ignore]
fn dump_core_traces() {
    let dir = std::path::PathBuf::from(std::env::var("RIFT_TRACE_DIR").expect("RIFT_TRACE_DIR"));
    std::fs::create_dir_all(&dir).unwrap();
    let (cases, len) = (
        budget("RIFT_FUZZ_CASES", 200) as u64,
        budget("RIFT_FUZZ_STEPS", 30),
    );
    for seed in 0..cases {
        let mut words = Vec::new();
        for (event, effects) in trace(Setup::of(seed), &generate(seed, len)) {
            rift_core::plain::wire::record_step(&mut words, event, effects);
        }
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        std::fs::write(dir.join(format!("seed-{seed}.trace")), bytes).unwrap();
    }
}

// ---------------------------------------------------------------------------------------------
// Regressions.

#[test]
fn a_fullscreen_identity_replaced_mid_write_leaves_no_stale_pending_target() {
    // A modifier drop writes the window; it goes native fullscreen and its identity is destroyed
    // there. The pending store kept the write's target under the window-server id, which the new
    // identity inherited.
    let steps = [
        Step::ModifierDrag { w: 0, dx: 140, dy: -111 },
        Step::MouseUp,
        Step::EnterFullscreen { w: 15 },
        Step::DestroyIdentity { w: 15 },
        Step::Rekey { w: 15 },
        Step::LeaveFullscreen { w: 15 },
        Step::Deliver,
    ];
    assert_eq!(execute(Setup::plain(2), &steps, &[]), Ok(()));
}

#[test]
fn a_window_rekeyed_mid_write_after_a_claim_reflow_reaches_its_slot() {
    // The claim reflows the display, a write goes to another window, and macOS replaces that
    // window's identity before the write lands. rift left it outside its slot.
    let steps = [Step::Claim { w: 4 }, Step::Rekey { w: 10 }];
    assert_eq!(execute(Setup::plain(3), &steps, &[]), Ok(()));
}

#[test]
fn a_window_rekeyed_mid_write_after_a_drop_reflow_reaches_its_slot() {
    let steps = [
        Step::DragStart { w: 3, dx: -26, dy: -121 },
        Step::CrossDisplay { to: 0 },
        Step::MouseUp,
        Step::Rekey { w: 4 },
    ];
    assert_eq!(execute(Setup::plain(3), &steps, &[]), Ok(()));
}

#[test]
fn a_new_identitys_stamped_reads_are_never_taken_for_rifts_own() {
    // During another window's drag, a window's identity dies in fullscreen and comes back under
    // a new id; its reads carry the old identity's stamps. rift took them for its own frames, a
    // stale one became the acknowledged frame, and the window stayed outside its slot.
    let steps = [
        Step::ForeignMove {
            w: 3,
            to: 2,
            mouse: Mouse::Truth,
        },
        Step::DragStart { w: 13, dx: 33, dy: -27 },
        Step::EnterFullscreen { w: 10 },
        Step::DestroyIdentity { w: 10 },
        Step::Rekey { w: 10 },
        Step::LeaveFullscreen { w: 10 },
        Step::Refresh {
            app: 1,
            stale: true,
            zero_min: false,
        },
    ];
    assert_eq!(execute(Setup::plain(3), &steps, &[]), Ok(()));
}

#[test]
fn a_window_claimed_mid_write_gets_no_write_over_it_when_given_back() {
    // A drop writes the window, which is claimed before the app answers and released during
    // another drag. rift sent a second write over the unanswered one.
    let steps = [
        Step::Press { w: 10 },
        Step::DragMove { dx: 149, dy: 110 },
        Step::DragMove { dx: 139, dy: -57 },
        Step::MouseUp,
        Step::Claim { w: 11 },
        Step::DragStart { w: 15, dx: -89, dy: -132 },
        Step::Unclaim { w: 11 },
        Step::MouseUp,
    ];
    assert_eq!(execute(Setup::plain(3), &steps, &[]), Ok(()));
}

#[test]
fn another_apps_window_moving_does_not_take_over_the_drag() {
    // The user holds one window (button down). Another app moves its own window. rift used to
    // start dragging that one instead.
    let steps = [
        Step::DragStart { w: 1, dx: -105, dy: 102 },
        Step::ForeignMove {
            w: 8,
            to: 0,
            mouse: Mouse::Truth,
        },
    ];
    assert_eq!(execute(Setup::plain(2), &steps, &[]), Ok(()));
}

#[test]
fn another_apps_window_moving_with_the_button_reported_up_does_not_end_the_drag() {
    // As above, but the other window's report says the button is up. rift used to end the
    // user's drag.
    let steps = [
        Step::DragStart { w: 6, dx: 141, dy: -65 },
        Step::ForeignMove {
            w: 7,
            to: 0,
            mouse: Mouse::ReportsUp,
        },
    ];
    assert_eq!(execute(Setup::plain(2), &steps, &[]), Ok(()));
}

#[test]
fn a_refresh_after_crossing_displays_snaps_the_dragged_window_back() {
    let steps = [
        Step::DragStart { w: 3, dx: 110, dy: -131 },
        Step::CrossDisplay { to: 2 },
        Step::Refresh {
            app: 1,
            stale: false,
            zero_min: false,
        },
    ];
    assert_eq!(execute(Setup::plain(3), &steps, &[]), Ok(()));
}

#[test]
fn a_window_dropped_on_another_display_leaves_the_old_displays_layout() {
    // Found by the quiescence check, and present at checkpoint/a-before-reconciler: after a
    // native drag onto another display the window was in both displays' layouts, both frames
    // were written, and it ended on the old display's tile while assigned to the new display.
    let mut sim = Sim::new(Setup::plain(2));
    let wid = sim.pick(12);
    for step in [
        Step::DragStart { w: 12, dx: -36, dy: 22 },
        Step::DragMove { dx: 138, dy: -64 },
        Step::CrossDisplay { to: 0 },
        Step::MouseUp,
    ] {
        let emitted = sim.run(step);
        sim.pending.extend(emitted);
    }
    let layouts = LayoutManager::calculate_layout(&mut sim.reactor, None);
    assert_eq!(claims(&layouts, wid), 1);
}

#[test]
fn a_wake_that_forgets_a_parked_windows_queued_move_still_parks_it() {
    // The drop's write is in flight when the window is sent to another workspace, so its
    // parking desire queues behind it; the wake then forgets both. The model must not stay at
    // the dropped parking frame (nothing would write it again once the old answer came back):
    // where the app has the window is unknown, so the next desire parks it.
    let steps = [
        Step::DragStart { w: 8, dx: -13, dy: 150 },
        Step::MouseUp,
        Step::MoveToWorkspace { w: 3, k: 1 },
        Step::Wake,
    ];
    assert_eq!(execute(Setup::plain(3), &steps, &[]), Ok(()));
}

#[test]
fn a_press_and_move_after_a_scrolling_camera_starts_the_drag() {
    // The camera the drop queues reaches the animation thread long before the next input; the
    // app's own move of the window must not be overwritten by the previous camera's frames, or
    // the user's move back looks like nothing at all.
    let steps = [
        Step::MouseUp,
        Step::ModifierDrag { w: 6, dx: 24, dy: -46 },
        Step::ForeignMove {
            w: 0,
            to: 1,
            mouse: Mouse::Truth,
        },
        Step::MouseUp,
        Step::Press { w: 0 },
        Step::CrossDisplay { to: 0 },
    ];
    let setup = Setup {
        displays: 3,
        animate: true,
        scrolling: true,
    };
    assert_eq!(execute(setup, &steps, &[]), Ok(()));
}

#[test]
fn a_lagging_inventory_read_during_another_drag_is_no_news() {
    // An inventory read made before the app applied Rift's write reports the old width while
    // another window is dragged; the model must not take it (nothing would correct it).
    let steps = [
        Step::DragStart { w: 14, dx: 128, dy: 134 },
        Step::ForeignMove {
            w: 7,
            to: 2,
            mouse: Mouse::ReportsUp,
        },
        Step::CrossDisplay { to: 0 },
        Step::MouseUp,
        Step::DragStart { w: 9, dx: -44, dy: 89 },
        Step::Deliver,
        Step::Refresh {
            app: 1,
            stale: true,
            zero_min: true,
        },
        Step::CrossDisplay { to: 1 },
    ];
    assert_eq!(execute(Setup::plain(2), &steps, &[]), Ok(()));
}

#[test]
fn a_camera_moving_windows_during_a_drag_does_not_unsettle_them_at_the_drop() {
    // Windows a scrolling camera moves while another window is dragged are Rift's own frames,
    // not news: the drop must not hand them back unknown (they would be written to a frame
    // taken mid-camera instead of riding it).
    let steps = [
        Step::Press { w: 7 },
        Step::DragMove { dx: -58, dy: 63 },
        Step::ForeignMove {
            w: 12,
            to: 1,
            mouse: Mouse::Truth,
        },
        Step::MouseUp,
        Step::ForeignMove {
            w: 5,
            to: 0,
            mouse: Mouse::Truth,
        },
        Step::ForeignMove {
            w: 14,
            to: 0,
            mouse: Mouse::Truth,
        },
        Step::ModifierDrag { w: 5, dx: -140, dy: 144 },
        Step::ForeignMove {
            w: 15,
            to: 2,
            mouse: Mouse::Truth,
        },
        Step::ForeignMove {
            w: 1,
            to: 0,
            mouse: Mouse::Truth,
        },
        Step::Deliver,
    ];
    let setup = Setup {
        displays: 3,
        animate: true,
        scrolling: true,
    };
    assert_eq!(execute(setup, &steps, &[]), Ok(()));
}

#[test]
fn a_window_dropped_into_another_displays_scrolling_layout_settles_where_layout_wants_it() {
    // Found by the fuzzer (seed 134 at 2000x40); it predates the pointer deciding a drag's
    // display (the drag here moves the whole frame across). A window destroyed mid-drag moves
    // display 1's scrolling viewport; the drop's camera then retargets it, but every window
    // was unknown after the drag, so the core wrote each at the target offset and the camera
    // was left with nothing to present. It ended at once, and its viewport was committed as a
    // live animation at the old offset that nothing would advance: layout went on placing the
    // remaining window there, away from where it had been written.
    let steps = [
        Step::DragStart { w: 7, dx: -500, dy: 81 },
        Step::ForeignMove {
            w: 0,
            to: 1,
            mouse: Mouse::Truth,
        },
        Step::ForeignMove {
            w: 0,
            to: 2,
            mouse: Mouse::Truth,
        },
        Step::Destroy { w: 1 },
    ];
    let setup = Setup {
        displays: 2,
        animate: true,
        scrolling: true,
    };
    assert_eq!(execute(setup, &steps, &[]), Ok(()));
}

#[test]
fn a_rounding_app_settled_within_the_ack_tolerance_is_where_layout_wants_it() {
    // Found by the 4000x60 sweep (seed 1280): a terminal (whole 7pt cells) dropped into a
    // 400pt-tall tile answers 399. The core takes that as the answer to its write; quiescence
    // compared with a tenth of a point and flagged it.
    let steps = [
        Step::DragStart { w: 10, dx: -95, dy: -75 },
        Step::CrossDisplayLagging { to: 1 },
        Step::Sizing {
            w: 0,
            sizing: TestSizing::Cells(7.0),
        },
        Step::MouseUp,
        Step::Destroy { w: 14 },
        Step::DragStart { w: 3, dx: -67, dy: 110 },
        Step::HeldMovesButtonUp { dx: 129 },
        Step::Deliver,
    ];
    assert_eq!(execute(Setup::plain(2), &steps, &[]), Ok(()));
}
