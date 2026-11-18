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
use crate::actor::reactor::reconciler::{Desired, Rift, Sync};
use crate::actor::reactor::testing::TestSizing;

/// Signatures (invariant, step kind) this branch is known to fail. Each has a regression test.
const KNOWN: &[(&str, &str)] = &[];

const SCREEN_W: f64 = 1000.;
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
    reactor.state.press(Some(wid), crate::actor::drag::MouseButton::Left);
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
    /// Time passes: every unanswered write is past its deadline.
    Tick,
    /// macOS asks for a full refresh (a wake): the display topology is re-sent as it is, with
    /// the refresh flag up. Pending targets of parked windows no longer hold.
    Wake,
    /// Window `w`'s app starts treating the sizes it is asked for this way.
    Sizing { w: usize, sizing: TestSizing },
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
            Step::Tick => "tick",
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
        .map(|_| match rng.below(100) {
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
            30..40 => Step::CrossDisplay { to: rng.below(3) },
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
            _ => Step::MoveFocusedToDisplay { d: rng.below(3) },
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
struct Held {
    window: WindowId,
    /// rift registered the drag when it started. Invariants about the drag apply only then;
    /// otherwise rift had no way to know.
    rift_saw_it: bool,
    assigned_at_start: Option<SpaceId>,
    /// A command ran during the drag: whether a command may move a held window is policy.
    command_during_drag: bool,
    /// Where the pointer is, for a modifier drag (rift follows the pointer, not the window).
    modifier_pointer: Option<CGPoint>,
}

impl Held {
    /// The user's hand goes down on `window`, assigned to `assigned_at_start`.
    fn new(window: WindowId, rift_saw_it: bool, assigned_at_start: Option<SpaceId>) -> Self {
        Held {
            window,
            rift_saw_it,
            assigned_at_start,
            command_during_drag: false,
            modifier_pointer: None,
        }
    }
}

struct Sim {
    apps: Apps,
    reactor: Reactor,
    screens: Vec<CGRect>,
    spaces: Vec<SpaceId>,
    pids: Vec<pid_t>,
    windows: Vec<WindowId>,
    live: BTreeSet<WindowId>,
    previous_frame: BTreeMap<WindowId, CGRect>,
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
}

fn display_count(seed: u64) -> usize { 2 + (seed % 2) as usize }

impl Sim {
    /// Every step the reactor takes through the core is recorded from the start
    /// (`reactor.state.core.trace`): the invariants read why rift wrote what it wrote, and the
    /// differential test replays it.
    fn new(displays: usize) -> Self {
        let (mut apps, mut reactor) = test_context();
        *reactor.state.core.trace.borrow_mut() = Some(Vec::new());
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
        Sim {
            apps,
            reactor,
            screens,
            spaces,
            pids,
            windows,
            live,
            previous_frame: BTreeMap::new(),
            held: None,
            pending: Vec::new(),
            moved: None,
            settled_before: BTreeMap::new(),
            dragged_before: None,
            gave_up: BTreeSet::new(),
            distrusted: BTreeSet::new(),
        }
    }

    fn pick(&self, w: usize) -> WindowId { self.windows[w % self.windows.len()] }

    fn os_frame(&self, wid: WindowId) -> CGRect { self.apps.windows[&wid].frame }

    fn set_os_frame(&mut self, wid: WindowId, frame: CGRect) {
        let old = self.os_frame(wid);
        self.previous_frame.insert(wid, old);
        self.apps.windows.get_mut(&wid).unwrap().frame = frame;
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

    fn held_window(&self) -> Option<WindowId> { self.held.as_ref().map(|held| held.window) }

    fn true_mouse(&self) -> MouseState {
        if self.held.is_some() {
            MouseState::Down
        } else {
            MouseState::Up
        }
    }

    /// Runs one step. Returns the requests rift emitted while handling it.
    fn run(&mut self, step: Step) -> Vec<Request> {
        self.moved = None;
        self.distrusted.clear();
        let traced = self.core_steps();
        self.dragged_before = dragged(&self.reactor);
        self.settled_before = self
            .live
            .iter()
            .filter_map(|&wid| match self.reactor.state.core.sync(wid)? {
                Sync::Settled { acked } => Some((wid, (acked, self.os_frame(wid)))),
                Sync::Unknown
                | Sync::InFlight { .. }
                | Sync::Refused { .. }
                | Sync::Animating { .. } => None,
            })
            .collect();
        match step {
            Step::DragStart { w, dx, dy } => {
                let wid = self.pick(w);
                if self.held.is_none() && self.live.contains(&wid) {
                    let assigned_at_start = self.reactor.assigned_space_for_window_id(wid);
                    let frame = offset(self.os_frame(wid), dx, dy);
                    press(&mut self.reactor, wid);
                    self.report_frame(wid, frame, MouseState::Down);
                    let rift_saw_it = dragged(&self.reactor) == Some(wid);
                    self.held = Some(Held::new(wid, rift_saw_it, assigned_at_start));
                }
            }
            Step::Press { w } => {
                let wid = self.pick(w);
                if self.held.is_none() && self.live.contains(&wid) {
                    press(&mut self.reactor, wid);
                    // Nothing has moved yet, so there is no drag for rift to have seen.
                    let assigned_at_start = self.reactor.assigned_space_for_window_id(wid);
                    self.held = Some(Held::new(wid, false, assigned_at_start));
                }
            }
            Step::ModifierDrag { w, dx, dy } => {
                let wid = self.pick(w);
                if self.held.is_none() && self.live.contains(&wid) {
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
                        let mut held = Held::new(gripped, rift_saw_it, assigned_at_start);
                        let point = CGPoint::new(point.x + f64::from(dx), point.y + f64::from(dy));
                        held.modifier_pointer = Some(point);
                        self.held = Some(held);
                        self.reactor.handle_event(Event::DragMotion(
                            crate::actor::drag::DragMotion { point },
                        ));
                    }
                }
            }
            Step::ModifierMove { dx, dy } => {
                if let Some(held) = &mut self.held
                    && let Some(point) = &mut held.modifier_pointer
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
                let mine: Vec<WindowId> =
                    self.live.iter().copied().filter(|wid| wid.pid == pid).collect();
                let new = mine
                    .iter()
                    .filter_map(|&wid| {
                        let mut info = self.reactor.state.windows.window(wid)?.info.clone();
                        info.frame = if stale {
                            self.previous_frame.get(&wid).copied().unwrap_or(self.os_frame(wid))
                        } else {
                            self.os_frame(wid)
                        };
                        if zero_min {
                            info.min_size = Some(CGSize::ZERO);
                        }
                        Some((wid, info))
                    })
                    .collect();
                self.reactor.discover_test_windows(pid, new, mine);
            }
            Step::ForeignMove { w, to, mouse } => {
                let wid = self.pick(w);
                if self.live.contains(&wid) && self.held_window() != Some(wid) {
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
            Step::Destroy { w } => {
                let wid = self.pick(w);
                if self.live.remove(&wid) {
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
        self.read_core_steps(traced);
        self.apps.requests()
    }

    fn core_steps(&self) -> usize {
        self.reactor.state.core.trace.borrow().as_ref().map_or(0, Vec::len)
    }

    /// What the core decided since step `from` that the invariants need: a modifier drag's
    /// interactive frame, or the core forgetting a window, ends rift's wait for it; and a
    /// window whose acknowledged frame rift stopped trusting (a display change, a drag's end) is
    /// written whatever it was.
    fn read_core_steps(&mut self, from: usize) {
        let trace = self.reactor.state.core.trace.borrow();
        for (event, effects) in &trace.as_ref().expect("recording")[from..] {
            match event {
                // A modifier drag took the window, or something outside the core (a topology
                // change, an animation) did: either way rift stopped waiting for its write.
                rift_core::Event::Follow { window, .. } | rift_core::Event::Forget { window } => {
                    _ = self.gave_up.insert(*window)
                }
                rift_core::Event::Distrust { windows } => self.distrusted.extend(windows),
                _ if effects
                    .iter()
                    .any(|effect| matches!(effect, rift_core::Effect::Ungrip { .. })) =>
                {
                    self.distrusted.extend(effects.iter().filter_map(|effect| match effect {
                        rift_core::Effect::Ungrip { window, .. } => Some(*window),
                        _ => None,
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
        let writes = frames_written(emitted);
        // 1. Liveness: a pressed, live window that moved under the pointer is the drag. Without
        // this, a drag that fails to start would quietly skip every invariant below.
        // Exceptions: a step that leaves the frame where it was moves nothing; and a move that
        // lands on the very frame rift has in flight for the window is, by geometry, that
        // write's acknowledgement (the next move starts the drag).
        if matches!(
            step,
            Step::DragStart { .. } | Step::DragMove { .. } | Step::CrossDisplay { .. }
        ) && let Some(held) = &self.held
            && self.live.contains(&held.window)
            && self.moved == Some(held.window)
            && !frames_written(&self.pending).iter().any(|(wid, frame)| {
                *wid == held.window && frame.same_as(self.os_frame(held.window))
            })
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
            // A modifier drag moves its own window by interactive frames, and only that window.
            let modifier_drag = self.held.as_ref().filter(|held| held.modifier_pointer.is_some());
            if emitted.iter().any(|request| match request {
                Request::InteractiveFramesPending(frames) => modifier_drag
                    .is_none_or(|held| frames.windows().into_iter().any(|w| w != held.window)),
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
                    if !frame.same_as(target) {
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
                    | Sync::Unknown
                    | Sync::Animating { .. },
                ) => {
                    if self.moved != Some(wid) && !matches!(step, Step::Tick) {
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
            // it believes.)
            let placed = matches!(
                step,
                Step::MoveFocusedToDisplay { .. }
                    | Step::MoveToWorkspace { .. }
                    | Step::Refresh { .. }
            );
            if !placed
                && !self.distrusted.contains(&wid)
                && self.moved != Some(wid)
                && let Some((known, real)) = self.settled_before.get(&wid)
                && known.same_as(frame)
                && real.same_as(frame)
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
            let projected = self.reactor.state.windows.window(wid).map(|w| w.frame_monotonic);
            let seen = self.reactor.state.core.seen(wid);
            if projected != seen {
                return fail(
                    "frames-are-the-cores",
                    format!("{wid:?}: the store has {projected:?}, the core sees {seen:?}"),
                );
            }
        }
        // 9. Every live window stays assigned to some space.
        for &wid in &self.live {
            if self.reactor.assigned_space_for_window_id(wid).is_none() {
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
            if self.pending.is_empty() {
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
                        | Sync::Unknown
                        | Sync::Animating { .. },
                    ) => false,
                };
                if !refused && !observed.same_as(frame) {
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
fn execute(displays: usize, steps: &[Step], past: &[Signature]) -> Result<(), Violation> {
    let mut step_index = 0;
    let mut current: Option<Step> = None;
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let mut sim = Sim::new(displays);
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
    displays: usize,
    mut steps: Vec<Step>,
    target: Signature,
    past: &[Signature],
) -> (Vec<Step>, Violation) {
    let mut violation = execute(displays, &steps, past).unwrap_err();
    steps.truncate(violation.step + 1);
    loop {
        let mut progressed = false;
        let mut i = 0;
        while i < steps.len() {
            let mut candidate = steps.clone();
            candidate.remove(i);
            match execute(displays, &candidate, past) {
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
            let displays = display_count(seed);
            let steps = generate(seed, len);
            if let Err(violation) = execute(displays, &steps, &past) {
                let signature = violation.signature();
                if !found.contains_key(&signature) {
                    let (steps, violation) = shrink(displays, steps, signature, &past);
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
                "\n[{} after {}] seed {seed}, {} displays: {}\n  execute({}, &{steps:?}, &{past:?})",
                violation.invariant,
                violation.step_kind,
                display_count(*seed),
                violation.detail,
                display_count(*seed),
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

/// Rift's types to plain data, with times counted from `epoch`.
struct ToPlain {
    epoch: std::time::Instant,
}

impl rift_core::Translate<Rift, rift_core::plain::Plain> for ToPlain {
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

    fn time(&self, time: std::time::Instant) -> rift_core::plain::Nanos {
        rift_core::plain::Nanos((time - self.epoch).as_nanos() as u64)
    }

    fn request(&self, request: TransactionId) -> rift_core::plain::RequestId {
        rift_core::plain::RequestId(request.number())
    }
}

type PlainStep = (
    rift_core::Event<rift_core::plain::Plain>,
    Vec<rift_core::Effect<rift_core::plain::Plain>>,
);

/// Runs `steps` (and the settling at the end) and returns every step the reactor took through
/// the core, on plain data. Invariant violations are not this function's business.
fn trace(displays: usize, steps: &[Step]) -> Vec<PlainStep> {
    let mut sim = Sim::new(displays);
    let epoch = sim.reactor.state.core.clock.now();
    for &step in steps {
        let emitted = sim.run(step);
        sim.pending.extend(emitted);
    }
    _ = sim.settle(steps.len());
    let to = ToPlain { epoch };
    let recorded = sim.reactor.state.core.trace.borrow_mut().take().unwrap_or_default();
    recorded
        .into_iter()
        .map(|(event, effects)| {
            (
                event.translate(&to),
                effects.into_iter().map(|effect| effect.translate(&to)).collect(),
            )
        })
        .collect()
}

#[test]
fn the_core_on_plain_data_decides_what_the_reactor_decided() {
    // The reactor runs the core on its own types; replayed on plain data (what a foreign core
    // sees), every step must decide exactly the same effects.
    use rift_core::Core;
    let cases = budget("RIFT_FUZZ_CASES", 200) as u64;
    let len = budget("RIFT_FUZZ_STEPS", 30);
    let mut total = 0;
    for seed in 0..cases {
        let recorded = trace(display_count(seed), &generate(seed, len));
        let mut core = rift_core::RustCore::<rift_core::plain::Plain>::default();
        for (index, (event, effects)) in recorded.iter().enumerate() {
            assert_eq!(
                &core.step(event.clone()),
                effects,
                "seed {seed}, core step {index}: {event:?}"
            );
        }
        total += recorded.len();
    }
    assert!(total > 0, "the fuzzer took no steps through the core");
}

/// Writes every seed's trace to `$RIFT_TRACE_DIR/seed-<n>.json`, for the Roc core's
/// differential test (`crates/rift-core-roc`). Run explicitly:
/// `RIFT_TRACE_DIR=… cargo test dump_core_traces -- --ignored`.
#[test]
#[ignore]
fn dump_core_traces() {
    let dir = std::path::PathBuf::from(std::env::var("RIFT_TRACE_DIR").expect("RIFT_TRACE_DIR"));
    std::fs::create_dir_all(&dir).unwrap();
    let cases = budget("RIFT_FUZZ_CASES", 200) as u64;
    let len = budget("RIFT_FUZZ_STEPS", 30);
    for seed in 0..cases {
        let recorded = trace(display_count(seed), &generate(seed, len));
        let json = serde_json::to_string(&recorded).unwrap();
        std::fs::write(dir.join(format!("seed-{seed}.json")), json).unwrap();
    }
}

// The failures the fuzzer found on earlier revisions, shrunk; each now passes.

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
    assert_eq!(execute(2, &steps, &[]), Ok(()));
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
    assert_eq!(execute(2, &steps, &[]), Ok(()));
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
    assert_eq!(execute(3, &steps, &[]), Ok(()));
}

#[test]
fn a_window_dropped_on_another_display_leaves_the_old_displays_layout() {
    // Found by the quiescence check, and present at checkpoint/a-before-reconciler: after a
    // native drag onto another display the window was in both displays' layouts, both frames
    // were written, and it ended on the old display's tile while assigned to the new display.
    let mut sim = Sim::new(2);
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
