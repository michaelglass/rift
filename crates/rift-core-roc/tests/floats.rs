//! The floating state, kept between events: the Roc twin (`RocFloats`) against the Rust machine
//! (`rift_core::floats::Floats`), on random runs of events. Runs reuse a few windows of a few
//! apps (one of them a negative pid), a few workspaces, so windows float and tile again, go
//! fullscreen, take new identities, and apps quit with frames saved. Every step must leave the same whole state.

use rift_core::floats::{Floats, FloatsEvent, FullscreenKind};
use rift_core::plain::Rect;
use rift_core_roc::{AllocationCount, RocFloats};

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);

/// A window word: its pid (sign-extended to 32 bits) in the high half.
fn window(pid: i32, idx: u32) -> u64 { (u64::from(pid as u32) << 32) | u64::from(idx) }

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 { self.next() % n }

    fn chance(&mut self, percent: u64) -> bool { self.below(100) < percent }

    fn pid(&mut self) -> i32 { [1, 2, -3][self.below(3) as usize] }

    fn window(&mut self) -> u64 {
        let pid = self.pid();
        window(pid, 1 + self.below(2) as u32)
    }

    fn maybe_window(&mut self) -> Option<u64> { self.chance(70).then(|| self.window()) }

    fn workspace(&mut self) -> u64 { (1 << 32) | (1 + self.below(3)) }

    fn frame(&mut self) -> Rect {
        Rect {
            x: self.below(1000) as f64,
            y: self.below(800) as f64,
            w: 100.0 + self.below(500) as f64,
            h: 100.0 + self.below(400) as f64,
        }
    }

    fn event(&mut self) -> FloatsEvent<u64, i32, u64, Rect> {
        match self.below(26) {
            0..=3 => FloatsEvent::Floated(self.window()),
            4 | 5 => FloatsEvent::Unfloated(self.window()),
            6 => FloatsEvent::Fullscreen(self.window(), match self.below(3) {
                0 => None,
                1 => Some(FullscreenKind::Full),
                _ => Some(FullscreenKind::WithinGaps),
            }),
            7..=14 => FloatsEvent::FloatingFocus(self.maybe_window()),
            15 => FloatsEvent::Focused(self.maybe_window()),
            16 | 17 => FloatsEvent::Replaced {
                from: self.window(),
                to: self.window(),
            },
            18 => FloatsEvent::AppQuit(self.pid()),
            19 if self.chance(20) => FloatsEvent::FloatingTaken,
            19..=21 => FloatsEvent::FrameStored {
                workspace: self.workspace(),
                window: self.window(),
                frame: self.frame(),
                if_absent: self.chance(40),
            },
            22 => FloatsEvent::FramesForgotten(self.window()),
            23 | 24 => FloatsEvent::FrameForgotten {
                workspace: self.workspace(),
                window: self.window(),
            },
            _ => FloatsEvent::WorkspaceForgotten(self.workspace()),
        }
    }
}

type Machine = Floats<u64, u64, Rect>;

/// The Rust machine as the Roc twin views it: every part in order.
fn canonical(rust: &Machine) -> Machine {
    Floats::from_parts(
        rust.floating(),
        rust.fullscreens(),
        rust.floating_focus(),
        rust.focused(),
        rust.frames(),
    )
}

#[test]
fn the_roc_floats_keep_state_as_the_rust_ones_do() {
    let runs: u64 = std::env::var("RIFT_FLOATS_RUNS").map_or(2_000, |n| n.parse().unwrap());
    let mut rng = Rng(0xF10A_7000_0000_0001);
    let before = AllocationCount::now();
    let (mut steps, mut quits_with_frames) = (0, 0);
    for run in 0..runs {
        let mut rust = Machine::default();
        let mut roc = RocFloats::default();
        let mut history = Vec::new();
        for step in 0..40 {
            let event = rng.event();
            history.push(event.clone());
            if matches!(event, FloatsEvent::AppQuit(_)) && rust.frames().count() > 0 {
                quits_with_frames += 1;
            }
            rust.step(event.clone());
            roc.step(event);
            assert_eq!(
                roc.view(),
                canonical(&rust),
                "run {run} step {step}: the state after {history:#?}"
            );
            steps += 1;
        }
    }
    assert_eq!(
        AllocationCount::now().live(),
        before.live(),
        "Roc allocations left live"
    );
    assert!(quits_with_frames > 0, "no app quit with frames saved");
    eprintln!("floats: {steps} steps identical, state compared after each");
}

/// A step updates the state in place: with 200 floating windows, all fullscreen and framed in one
/// workspace (the tables full, so none grows), no step allocates more than a sliver of it.
#[test]
fn the_roc_floats_step_without_copying_a_table() {
    let mut rng = Rng(0xF10A_7000_0000_0002);
    let mut rust = Machine::default();
    let mut roc = RocFloats::default();
    let windows: Vec<u64> = (0..200).map(|i| window(7, i + 1)).collect();
    for &w in &windows {
        for event in [
            FloatsEvent::Floated(w),
            FloatsEvent::Fullscreen(w, Some(FullscreenKind::Full)),
            FloatsEvent::FrameStored {
                workspace: 1,
                window: w,
                frame: rng.frame(),
                if_absent: false,
            },
        ] {
            rust.step(event.clone());
            roc.step(event);
        }
    }
    let bound = 2048;
    for step in 0..3000 {
        let w = windows[rng.below(200) as usize];
        let event = match rng.below(6) {
            0 => FloatsEvent::Fullscreen(w, Some(FullscreenKind::Full)),
            1 => FloatsEvent::Fullscreen(w, Some(FullscreenKind::WithinGaps)),
            2 => FloatsEvent::FloatingFocus(Some(w)),
            3 => FloatsEvent::Focused(Some(w)),
            4 => FloatsEvent::FrameStored {
                workspace: 1,
                window: w,
                frame: rng.frame(),
                if_absent: false,
            },
            _ => FloatsEvent::Floated(w),
        };
        rust.step(event.clone());
        rift_core_roc::take_largest_allocation();
        roc.step(event.clone());
        let largest = rift_core_roc::take_largest_allocation();
        assert!(
            largest < bound,
            "step {step}: {event:?} allocated {largest} bytes (bound {bound})"
        );
    }
    assert_eq!(roc.view(), canonical(&rust));
}
