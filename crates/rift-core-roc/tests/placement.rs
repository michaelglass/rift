//! The Roc core's placement choices against the Rust core's (`rift_core::placement`), on random
//! questions: the answers must be identical. Windows and workspaces are few, so sources are the
//! destination about half the time and targets are the dropped window often enough; frames are
//! drawn from a few, some unusable (zero, negative, not finite), so a floating window is dropped
//! where it is as often as elsewhere; slots are short paths that often match.

use rift_core::layout::bsp_tree::Direction;
use rift_core::placement::{
    ActionAsk, Arrangement, DropFacts, DropQuestion as Q, DropTarget, KeptAsk, Laid, OverrideAsk,
    WindowDropAction as A, choose_drop,
};
use rift_core::plain::Rect;

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

    fn maybe<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> Option<T> {
        if self.below(3) == 0 {
            None
        } else {
            Some(f(self))
        }
    }

    fn direction(&mut self) -> Direction {
        [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ][self.below(4) as usize]
    }

    fn action(&mut self) -> A<Direction> {
        match self.below(5) {
            0 => A::Swap,
            1 => A::Stack,
            2 => A::Insert(self.direction()),
            3 => A::Move(self.direction()),
            _ => A::ConsumeOrExpel(self.direction()),
        }
    }

    fn rect(&mut self) -> Rect {
        let base = Rect {
            x: 10.0 * self.below(2) as f64,
            y: 0.0,
            w: 400.0,
            h: 300.0,
        };
        match self.below(16) {
            0 => Rect { w: 0.0, ..base },
            1 => Rect { h: -5.0, ..base },
            2 => Rect { x: f64::NAN, ..base },
            3 => Rect { y: f64::INFINITY, ..base },
            _ => base,
        }
    }

    fn arrangement(&mut self) -> Arrangement {
        [
            Arrangement::Floating,
            Arrangement::Stack,
            Arrangement::Tiled,
        ][self.below(3) as usize]
    }

    fn slot(&mut self) -> Vec<usize> {
        (0..self.below(3)).map(|_| self.below(2) as usize).collect()
    }
}

#[test]
fn the_roc_core_makes_the_engines_drop_choices_as_the_rust_core_does() {
    let mut rng = Rng(0xD20F_0E1C_4A00);
    let runs: u64 = std::env::var("RIFT_ENGINE_DROP_CASES").map_or(20_000, |n| n.parse().unwrap());
    let mut accepted = 0;
    for case in 0..runs {
        let question: Q<u64, u64, Direction> = match rng.below(5) {
            0 | 1 => {
                let laid = match rng.below(6) {
                    0 => Laid::NoSpace,
                    1 => Laid::NoLayout,
                    _ => Laid::Laid(()),
                };
                Q::Plan(DropFacts {
                    window: 1,
                    source: rng.maybe(|r| 10 + r.below(2)),
                    destination: 10,
                    laid,
                    floating: rng.below(2) == 0,
                    at: rng.maybe(Rng::rect),
                    frame: rng.maybe(Rng::rect),
                    target: rng.maybe(|r| DropTarget {
                        window: 1 + r.below(2),
                        action: r.action(),
                        in_destination: r.below(4) > 0,
                        floating: r.below(4) == 0,
                    }),
                })
            }
            2 => Q::Kept(KeptAsk {
                before: rng.maybe(Rng::slot),
                after: rng.maybe(Rng::slot),
            }),
            3 => Q::Action(ActionAsk {
                same_slot: rng.below(2) == 0,
                action: rng.action(),
                center: [A::Swap, A::Stack][rng.below(2) as usize],
            }),
            _ => match rng.below(2) {
                0 => Q::Override(OverrideAsk(rng.maybe(Rng::arrangement))),
                _ => Q::Edit(rng.arrangement()),
            },
        };
        let rust = choose_drop(question.clone());
        let roc = rift_core_roc::placement::engine_drop(question.clone());
        // Frames carry NaN, which equals nothing: compare the answers as written.
        assert_eq!(
            format!("{roc:?}"),
            format!("{rust:?}"),
            "case {case}: {question:?}"
        );
        accepted += u64::from(matches!(
            rust,
            rift_core::placement::DropChoice::Plan(
                rift_core::placement::DropPlan::Float { .. }
                    | rift_core::placement::DropPlan::Relocate { .. }
                    | rift_core::placement::DropPlan::Target { .. }
            )
        ));
    }
    assert!(accepted > 0);
    eprintln!("engine drop: {runs} questions answered alike ({accepted} drops accepted)");
}
