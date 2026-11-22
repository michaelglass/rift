//! The main window, kept between events: the Roc twin (`RocMainWindows`) against the Rust
//! machine (`rift_core::main_window::MainWindows`), on random runs of events. Runs reuse a few
//! pids (one of them a negative pid, sign-extended, as rift's words carry it) and a few windows,
//! so apps launch again, terminate while focused, and windows are replaced while main or focused.
//! Every step must answer the same window to focus, and leave the same whole state (main window,
//! native focus, every app, the globally frontmost app).

use rift_core::main_window::{Initiator, MainWindowEvent, MainWindows, Standing};
use rift_core_roc::main_window::MainWindowsView;
use rift_core_roc::{AllocationCount, RocMainWindows};

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 { self.next() % n }

    fn chance(&mut self, percent: u64) -> bool { self.below(100) < percent }

    /// One of a few pids, as words: small ones and a negative one, sign-extended.
    fn pid(&mut self) -> u64 { [1, 2, 3, (-1i64) as u64][self.below(4) as usize] }

    /// One of a few windows, as words (a pid in the high half, an index in the low one).
    fn window(&mut self) -> u64 {
        [(1 << 32) | 1, (1 << 32) | 2, (2 << 32) | 1, 7][self.below(4) as usize]
    }

    fn maybe_window(&mut self) -> Option<u64> { self.chance(75).then(|| self.window()) }

    fn initiator(&mut self) -> Initiator {
        if self.chance(30) {
            Initiator::Rift
        } else {
            Initiator::Other
        }
    }

    fn standing(&mut self) -> Standing {
        if self.chance(50) {
            Standing::Frontmost
        } else {
            Standing::Background
        }
    }

    /// A run starts cold: the window server's first report comes rarely, so the cold-start
    /// answers get exercised before it.
    fn event(&mut self) -> MainWindowEvent<u64, u64> {
        let pid = self.pid();
        match self.below(20) {
            0..=2 => MainWindowEvent::Launched {
                pid,
                standing: self.standing(),
                main_window: self.maybe_window(),
            },
            3 => MainWindowEvent::Terminated(pid),
            4..=6 => MainWindowEvent::Activated(pid, self.initiator()),
            7 => MainWindowEvent::Deactivated(pid),
            8..=10 => MainWindowEvent::GloballyActivated(pid),
            11 => MainWindowEvent::GloballyDeactivated(pid),
            12..=14 => {
                MainWindowEvent::MainWindowChanged(pid, self.maybe_window(), self.initiator())
            }
            15 => MainWindowEvent::NativeFocused { pid, window: self.window() },
            16 | 17 => MainWindowEvent::Replaced {
                pid,
                window: self.window(),
                replacement: None,
            },
            _ => MainWindowEvent::Replaced {
                pid,
                window: self.window(),
                replacement: Some(self.window()),
            },
        }
    }
}

#[test]
fn the_roc_main_windows_answer_and_keep_state_as_the_rust_ones_do() {
    let runs: u64 = std::env::var("RIFT_MAIN_WINDOW_RUNS").map_or(1_000, |n| n.parse().unwrap());
    let mut rng = Rng(0x3A1D_0F0C_0500_0001);
    let before = AllocationCount::now();
    let (mut steps, mut answered) = (0, 0);
    for run in 0..runs {
        let mut rust = MainWindows::<u64, u64>::default();
        let mut roc = RocMainWindows::default();
        let mut history = Vec::new();
        for step in 0..60 {
            let event = rng.event();
            history.push(event);
            let expected = rust.step(event);
            assert_eq!(
                roc.step(event),
                expected,
                "run {run} step {step}: the window to focus after {history:#?}"
            );
            assert_eq!(
                roc.view(),
                MainWindowsView::from(&rust),
                "run {run} step {step}: the state after {history:#?}"
            );
            steps += 1;
            answered += u64::from(expected.is_some());
        }
    }
    assert_eq!(
        AllocationCount::now().live(),
        before.live(),
        "Roc allocations left live"
    );
    assert!(
        answered > 0,
        "no step answered a window: the cold start went unexercised"
    );
    eprintln!(
        "main_window: {steps} steps identical ({answered} answered a window), state compared after each"
    );
}
