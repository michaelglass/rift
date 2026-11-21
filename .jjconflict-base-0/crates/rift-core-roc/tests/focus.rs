//! The Roc core's focus decisions against the Rust core's (`rift_core::focus`), on random
//! questions: the answers must be identical word for word. Ids are few and collide on purpose,
//! so removals hit the focused window, siblings share apps, and candidates overlap.

use rift_core::focus::Removal;
use rift_core::focus::plain::{
    Activation, Candidate, Desktop, DisplayFollow, Hides, RemovalQuestion, Untracked,
};
mod facts;

use facts::{Coin, rust, typed};
use rift_core::plain::{WindowId, wire};

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
}

impl Coin for Rng {
    fn chance(&mut self, percent: u64) -> bool { Rng::chance(self, percent) }
}

#[test]
fn the_roc_core_decides_what_a_removal_does_to_focus_as_the_rust_core_does() {
    typed("focus", "removal", Rng(0x2E40_7A10_0000_0017), |rng, blank| {
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
        let pick = |rng: &mut Rng| match rng.below(3) {
            0 => focused,
            1 => sibling,
            _ => rng.window(),
        };
        let removal = match rng.below(4) {
            0 => Removal::AppClosed(pick(rng).pid),
            _ => Removal::WindowRemoved(pick(rng)),
        };
        let removed_space = rng.chance(85).then(|| rng.id());
        let native_focus = rng.chance(80).then_some(native);
        // A native window is tracked as a listed successor, or as itself.
        let listed: Vec<(WindowId, WindowId)> = (0..rng.below(3))
            .map(|_| (if rng.chance(60) { native } else { pick(rng) }, pick(rng)))
            .collect();
        let successor = |from: WindowId| {
            listed.iter().find(|(listed, _)| *listed == from).map_or(from, |(_, to)| *to)
        };
        let hidden: Vec<WindowId> = (0..rng.below(3)).map(|_| pick(rng)).collect();
        let successors = blank.answers(
            rng,
            [native].into_iter().chain(listed.iter().map(|(from, _)| *from)),
            successor,
        );
        let candidates: Vec<WindowId> = [native, successor(native)]
            .into_iter()
            .chain(listed.iter().map(|(_, to)| *to))
            .collect();
        let question = RemovalQuestion {
            removal,
            focused: rng.chance(85).then_some(focused),
            removed_space: blank.fact(rng, removed_space),
            native_focus: blank.fact(rng, native_focus),
            successors,
            inactive: blank.answers(rng, candidates, |window| hidden.contains(&window)),
        };
        let roc = rust(
            rift_core_roc::focus::removal_focus(question.clone()),
            wire::encode_removal_focus,
        );
        let rust = rust(question.answer(), wire::encode_removal_focus);
        (format!("{question:?}"), roc, rust)
    });
}

#[test]
fn the_roc_core_picks_the_window_to_focus_as_the_rust_core_does() {
    typed("focus", "candidate", Rng(0xCA4D_1DA7_E000_0018), |rng, blank| {
        let preferred = rng.maybe();
        let last_focused = rng.maybe();
        let in_workspace = rng.ids(5);
        let shown = rng.ids(4);
        let windows: Vec<u64> = preferred
            .into_iter()
            .chain(last_focused)
            .chain(in_workspace.iter().copied())
            .collect();
        let question = Candidate {
            preferred,
            last_focused: blank.fact(rng, last_focused),
            in_workspace: blank.fact(rng, in_workspace),
            visible: blank.answers(rng, windows, |window| shown.contains(&window)),
        };
        let roc = rust(
            rift_core_roc::focus::focus_candidate(question.clone()),
            wire::encode_maybe_space,
        );
        let rust = rust(question.answer(), wire::encode_maybe_space);
        (format!("{question:?}"), roc, rust)
    });
}

#[test]
fn the_roc_core_finds_hidden_focus_as_the_rust_core_does() {
    typed("focus", "hides", Rng(0x41DE_5F0C_0000_0019), |rng, blank| {
        let in_inactive_workspace = rng.chance(50);
        let question = Hides {
            window: rng.id(),
            main: rng.maybe(),
            focused: rng.maybe(),
            in_inactive_workspace: blank.fact(rng, in_inactive_workspace),
        };
        let roc = rust(
            rift_core_roc::focus::hides_focus(question.clone()),
            wire::encode_verdict,
        );
        let rust = rust(question.answer(), wire::encode_verdict);
        (format!("{question:?}"), roc, rust)
    });
}

#[test]
fn the_roc_core_follows_activations_as_the_rust_core_does() {
    typed(
        "focus",
        "activation",
        Rng(0xAC71_FA7E_0000_0020),
        |rng, blank| {
            let standard = rng.chance(90);
            let unavailable = rng.chance(10);
            let space = rng.chance(90).then(|| rng.id());
            let workspace = rng.chance(90).then(|| rng.id());
            let active_workspace = rng.chance(80).then(|| rng.id());
            let pending_window = rng.chance(10);
            let mode_refusal = rng.chance(10).then(|| rng.id());
            let active_space_fullscreen = rng.chance(10);
            let unmanageable_source = rng.chance(10).then(|| rng.id());
            let blacklisted = rng.chance(85).then(|| rng.chance(10));
            let question = Activation {
                standard: blank.fact(rng, standard),
                unavailable: blank.fact(rng, unavailable),
                space: blank.fact(rng, space),
                workspace: blank.fact(rng, workspace),
                active_workspace: blank.fact(rng, active_workspace),
                pending_window: blank.fact(rng, pending_window),
                mode_refusal: blank.fact(rng, mode_refusal),
                active_space_fullscreen: blank.fact(rng, active_space_fullscreen),
                unmanageable_source: blank.fact(rng, unmanageable_source),
                blacklisted: blank.fact(rng, blacklisted),
            };
            let roc = rust(
                rift_core_roc::focus::activation_follow(question.clone()),
                wire::encode_follow,
            );
            let rust = rust(question.answer(), wire::encode_follow);
            (format!("{question:?}"), roc, rust)
        },
    );
}

#[test]
fn the_roc_core_picks_the_display_to_follow_as_the_rust_core_does() {
    typed(
        "focus",
        "display-follow",
        Rng(0xD15F_0110_0000_0021),
        |rng, blank| {
            let focused_space = rng.maybe();
            let active = rng.ids(3);
            let on_known_display = rng.ids(3);
            let question = DisplayFollow {
                focused_space,
                previous: rng.maybe(),
                active: blank.answers(rng, focused_space, |space| active.contains(&space)),
                on_known_display: blank
                    .answers(rng, focused_space, |space| on_known_display.contains(&space)),
            };
            let roc = rust(
                rift_core_roc::focus::display_to_follow(question.clone()),
                wire::encode_maybe_space,
            );
            let rust = rust(question.answer(), wire::encode_maybe_space);
            (format!("{question:?}"), roc, rust)
        },
    );
}

#[test]
fn the_roc_core_decides_about_an_untracked_window_under_the_cursor_as_the_rust_core_does() {
    typed("focus", "untracked", Rng(0x0174_ACED_0000_0022), |rng, blank| {
        let under_cursor = rng.maybe();
        let tracked = rng.chance(30);
        let known = rng.chance(80).then(|| {
            let layer = [0, 0, 0, 25, -20, i32::MAX, i32::MIN][rng.below(7) as usize];
            (layer, rng.id())
        });
        let question = Untracked {
            under_cursor,
            tracked: blank.fact(rng, tracked),
            known: blank.fact(rng, known),
        };
        let roc = rust(
            rift_core_roc::focus::untracked_focus(question.clone()),
            wire::encode_untracked_focus,
        );
        let rust = rust(question.answer(), wire::encode_untracked_focus);
        (format!("{question:?}"), roc, rust)
    });
}

#[test]
fn the_roc_core_focuses_a_desktop_as_the_rust_core_does() {
    typed("focus", "desktop", Rng(0xDE5C_7000_0000_0023), |rng, blank| {
        let workspace_empty = rng.chance(50);
        let question = Desktop {
            active: rng.chance(50),
            workspace_empty: blank.fact(rng, workspace_empty),
        };
        let roc = rust(
            rift_core_roc::focus::focuses_desktop(question.clone()),
            wire::encode_verdict,
        );
        let rust = rust(question.answer(), wire::encode_verdict);
        (format!("{question:?}"), roc, rust)
    });
}
