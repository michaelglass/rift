//! The Roc core's focus decisions against the Rust core's (`rift_core::focus`), on random
//! questions: the answers must be identical word for word. Ids are few and collide on purpose,
//! so removals hit the focused window, siblings share apps, and candidates overlap.

use rift_core::focus::Removal;
use rift_core::focus::plain::{
    Activation, Candidate, Desktop, DisplayFollow, Hides, RemovalQuestion, Untracked,
};
use rift_core::plain::{WindowId, wire};
use rift_core_roc::{AllocationCount, tile_words};

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

    /// One of a few small ids, so they collide, and now and then an extreme one.
    fn id(&mut self) -> u64 {
        match self.below(20) {
            0 => u64::MAX,
            1 => 0,
            _ => 1 + self.below(5),
        }
    }

    fn ids(&mut self, most: u64) -> Vec<u64> {
        (0..self.below(most + 1)).map(|_| self.id()).collect()
    }

    fn maybe(&mut self) -> Option<u64> { self.chance(50).then(|| self.id()) }

    fn window(&mut self) -> WindowId {
        WindowId {
            pid: [1, 2, -1, i32::MAX, i32::MIN][self.below(5) as usize],
            idx: [1, 2, 3, u32::MAX][self.below(4) as usize],
        }
    }

    fn windows(&mut self, most: u64) -> Vec<WindowId> {
        (0..self.below(most + 1)).map(|_| self.window()).collect()
    }
}

/// Runs random questions through both cores; `ask` makes one and returns its words and the
/// Rust core's answer as words.
fn differential(name: &str, seed: u64, mut ask: impl FnMut(&mut Rng) -> (Vec<u64>, Vec<u64>)) {
    let cases: u64 = std::env::var("RIFT_FOCUS_CASES").map_or(20_000, |n| n.parse().unwrap());
    let mut rng = Rng(seed);
    let before = AllocationCount::now();
    for case in 0..cases {
        let (question, rust) = ask(&mut rng);
        assert_eq!(
            tile_words(&question),
            Ok(rust),
            "case {case}: {name} {question:?}"
        );
    }
    let after = AllocationCount::now();
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    eprintln!("focus: {cases} {name} questions answered identically");
}

#[test]
fn the_roc_core_decides_what_a_removal_does_to_focus_as_the_rust_core_does() {
    differential("removal", 0x2E40_7A10_0000_0017, |rng| {
        // Lean on the cases that decide: the removal takes the focused window, macOS's key
        // window is that window or a sibling of the same app, and successors start from it.
        let focused = rng.window();
        let sibling = WindowId {
            idx: rng.window().idx,
            ..focused
        };
        let native = match rng.below(4) {
            0 => focused,
            1 | 2 => sibling,
            _ => rng.window(),
        };
        let mut pick = |rng: &mut Rng| match rng.below(3) {
            0 => focused,
            1 => sibling,
            _ => rng.window(),
        };
        let question = RemovalQuestion {
            removal: match rng.below(4) {
                0 => Removal::AppClosed(pick(rng).pid),
                _ => Removal::WindowRemoved(pick(rng)),
            },
            focused: rng.chance(85).then_some(focused),
            removed_space: rng.chance(85).then(|| rng.id()),
            native_focus: rng.chance(80).then_some(native),
            successors: (0..rng.below(3))
                .map(|_| (if rng.chance(60) { native } else { pick(rng) }, pick(rng)))
                .collect(),
            inactive: (0..rng.below(3)).map(|_| pick(rng)).collect(),
        };
        let rust = wire::encode_removal_focus(question.answer());
        (wire::encode_removal_question(question), rust)
    });
}

#[test]
fn the_roc_core_picks_the_window_to_focus_as_the_rust_core_does() {
    differential("candidate", 0xCA4D_1DA7_E000_0018, |rng| {
        let question = Candidate {
            preferred: rng.maybe(),
            last_focused: rng.maybe(),
            in_workspace: rng.ids(5),
            visible: rng.ids(4),
        };
        let rust = wire::encode_maybe_space(question.answer());
        (wire::encode_candidate(question), rust)
    });
}

#[test]
fn the_roc_core_finds_hidden_focus_as_the_rust_core_does() {
    differential("hides", 0x41DE_5F0C_0000_0019, |rng| {
        let question = Hides {
            window: rng.id(),
            main: rng.maybe(),
            focused: rng.maybe(),
            in_inactive_workspace: rng.chance(50),
        };
        let rust = wire::encode_verdict(question.answer());
        (wire::encode_hides(question), rust)
    });
}

#[test]
fn the_roc_core_follows_activations_as_the_rust_core_does() {
    differential("activation", 0xAC71_FA7E_0000_0020, |rng| {
        let question = Activation {
            standard: rng.chance(90),
            unavailable: rng.chance(10),
            space: rng.chance(90).then(|| rng.id()),
            workspace: rng.chance(90).then(|| rng.id()),
            active_workspace: rng.chance(80).then(|| rng.id()),
            pending_window: rng.chance(10),
            mode_refusal: rng.chance(10).then(|| rng.id()),
            active_space_fullscreen: rng.chance(10),
            unmanageable_source: rng.chance(10).then(|| rng.id()),
            blacklisted: rng.chance(85).then(|| rng.chance(10)),
        };
        let rust = wire::encode_follow(question.answer());
        (wire::encode_activation(question), rust)
    });
}

#[test]
fn the_roc_core_picks_the_display_to_follow_as_the_rust_core_does() {
    differential("display-follow", 0xD15F_0110_0000_0021, |rng| {
        let question = DisplayFollow {
            focused_space: rng.maybe(),
            previous: rng.maybe(),
            active: rng.ids(3),
            on_known_display: rng.ids(3),
        };
        let rust = wire::encode_maybe_space(question.answer());
        (wire::encode_display_follow(question), rust)
    });
}

#[test]
fn the_roc_core_decides_about_an_untracked_window_under_the_cursor_as_the_rust_core_does() {
    differential("untracked", 0x0174_ACED_0000_0022, |rng| {
        let question = Untracked {
            under_cursor: rng.maybe(),
            tracked: rng.chance(30),
            known: rng.chance(80).then(|| {
                let layer = [0, 0, 0, 25, -20, i32::MAX, i32::MIN][rng.below(7) as usize];
                (layer, rng.id())
            }),
        };
        let rust = wire::encode_untracked_focus(question.answer());
        (wire::encode_untracked(question), rust)
    });
}

#[test]
fn the_roc_core_focuses_a_desktop_as_the_rust_core_does() {
    differential("desktop", 0xDE5C_7000_0000_0023, |rng| {
        let question = Desktop {
            active: rng.chance(50),
            workspace_empty: rng.chance(50),
        };
        let rust = wire::encode_verdict(question.answer());
        (wire::encode_desktop(question), rust)
    });
}
