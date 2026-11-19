//! The Roc core's focus decisions against the Rust core's (`rift_core::focus`), on random
//! questions: the answers must be identical word for word. Ids are few and collide on purpose,
//! so removals hit the focused window, siblings share apps, and candidates overlap.

use rift_core::focus::Removal;
use rift_core::focus::plain::{
    Activation, Candidate, Desktop, DisplayFollow, Hides, RemovalQuestion, Untracked,
};
use rift_core::plain::{Answers, Fact, Unasked, WindowId, wire};
use rift_core_roc::{AllocationCount, RocError, tile_words};

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

/// The Rust core's answer, or the fact it read that its question does not carry.
type Answered = Result<Vec<u64>, u64>;

fn rust<A>(answer: Result<A, Unasked>, encode: impl FnOnce(A) -> Vec<u64>) -> Answered {
    answer.map(encode).map_err(|unasked| unasked.fact)
}

fn roc(question: &[u64]) -> Answered {
    match tile_words(question) {
        Ok(words) => Ok(words),
        Err(RocError::Unasked { fact }) => Err(fact),
        Err(error) => panic!("no answer to {question:?}: {error:?}"),
    }
}

/// Runs random questions through both cores, twice: on full facts (every fact asked, as the
/// decision's own differential has always run), then with facts left unasked at random, where
/// both cores must read the same facts in the same order: the same answer, or the same first
/// fact read that the question does not carry. `ask` makes a question with `Blank`'s facts and
/// returns its words and the Rust core's answer.
fn differential(
    name: &str,
    seed: u64,
    mut ask: impl FnMut(&mut Rng, Blank) -> (Vec<u64>, Answered),
) {
    let cases: u64 = std::env::var("RIFT_FOCUS_CASES").map_or(20_000, |n| n.parse().unwrap());
    let mut rng = Rng(seed);
    let before = AllocationCount::now();
    let mut unasked = 0;
    for blank in [Blank::Full, Blank::Some] {
        for case in 0..cases {
            let (question, rust) = ask(&mut rng, blank);
            if let Blank::Full = blank {
                assert!(
                    rust.is_ok(),
                    "case {case}: {name} on full facts read a missing one"
                );
            }
            unasked += u64::from(rust.is_err());
            assert_eq!(
                roc(&question),
                rust,
                "case {case} ({blank:?}): {name} {question:?}"
            );
        }
    }
    let after = AllocationCount::now();
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    eprintln!(
        "focus: {cases} {name} questions on full facts and {cases} with facts unasked answered \
         identically ({unasked} read an unasked fact)"
    );
}

/// Which facts a question carries.
#[derive(Debug, Clone, Copy)]
enum Blank {
    /// All of them.
    Full,
    /// Each left unasked (each key's answer dropped) one time in four.
    Some,
}

impl Blank {
    fn fact<T>(self, rng: &mut Rng, answer: T) -> Fact<T> {
        match self {
            Blank::Some if rng.chance(25) => Fact::Unasked,
            Blank::Full | Blank::Some => Fact::Asked(answer),
        }
    }

    /// Answers for `keys` (each once), as `answer` gives them.
    fn answers<K: Copy + PartialEq, V>(
        self,
        rng: &mut Rng,
        keys: impl IntoIterator<Item = K>,
        answer: impl Fn(K) -> V,
    ) -> Answers<K, V> {
        let mut asked: Vec<K> = Vec::new();
        for key in keys {
            if !asked.contains(&key) {
                asked.push(key);
            }
        }
        Answers(
            asked
                .into_iter()
                .filter(|_| match self {
                    Blank::Full => true,
                    Blank::Some => !rng.chance(25),
                })
                .map(|key| (key, answer(key)))
                .collect(),
        )
    }
}

#[test]
fn the_roc_core_decides_what_a_removal_does_to_focus_as_the_rust_core_does() {
    differential("removal", 0x2E40_7A10_0000_0017, |rng, blank| {
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
        let rust = rust(question.answer(), wire::encode_removal_focus);
        (wire::encode_removal_question(question), rust)
    });
}

#[test]
fn the_roc_core_picks_the_window_to_focus_as_the_rust_core_does() {
    differential("candidate", 0xCA4D_1DA7_E000_0018, |rng, blank| {
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
        let rust = rust(question.answer(), wire::encode_maybe_space);
        (wire::encode_candidate(question), rust)
    });
}

#[test]
fn the_roc_core_finds_hidden_focus_as_the_rust_core_does() {
    differential("hides", 0x41DE_5F0C_0000_0019, |rng, blank| {
        let in_inactive_workspace = rng.chance(50);
        let question = Hides {
            window: rng.id(),
            main: rng.maybe(),
            focused: rng.maybe(),
            in_inactive_workspace: blank.fact(rng, in_inactive_workspace),
        };
        let rust = rust(question.answer(), wire::encode_verdict);
        (wire::encode_hides(question), rust)
    });
}

#[test]
fn the_roc_core_follows_activations_as_the_rust_core_does() {
    differential("activation", 0xAC71_FA7E_0000_0020, |rng, blank| {
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
        let rust = rust(question.answer(), wire::encode_follow);
        (wire::encode_activation(question), rust)
    });
}

#[test]
fn the_roc_core_picks_the_display_to_follow_as_the_rust_core_does() {
    differential("display-follow", 0xD15F_0110_0000_0021, |rng, blank| {
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
        let rust = rust(question.answer(), wire::encode_maybe_space);
        (wire::encode_display_follow(question), rust)
    });
}

#[test]
fn the_roc_core_decides_about_an_untracked_window_under_the_cursor_as_the_rust_core_does() {
    differential("untracked", 0x0174_ACED_0000_0022, |rng, blank| {
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
        let rust = rust(question.answer(), wire::encode_untracked_focus);
        (wire::encode_untracked(question), rust)
    });
}

#[test]
fn the_roc_core_focuses_a_desktop_as_the_rust_core_does() {
    differential("desktop", 0xDE5C_7000_0000_0023, |rng, blank| {
        let workspace_empty = rng.chance(50);
        let question = Desktop {
            active: rng.chance(50),
            workspace_empty: blank.fact(rng, workspace_empty),
        };
        let rust = rust(question.answer(), wire::encode_verdict);
        (wire::encode_desktop(question), rust)
    });
}
