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

/// Relocation (`rift_core::placement::relocation`) on the facts' token form: every policy, a few
/// workspaces and Spaces (so owners are the source, the destination or a third), every pair of
/// families, layouts and trees present or not.
#[test]
fn the_roc_core_relocates_as_the_rust_core_does() {
    use rift_core::placement::{Edited, Family, Policy, RelocationFacts, relocation};
    let mut rng = Rng(0x2E10_CA7E_0E1C);
    let runs: u64 =
        std::env::var("RIFT_ENGINE_RELOCATION_CASES").map_or(20_000, |n| n.parse().unwrap());
    let family =
        |r: &mut Rng| [Family::Floating, Family::Scrolling, Family::Tiled][r.below(3) as usize];
    let mut moved = 0;
    for case in 0..runs {
        let policy = match rng.below(5) {
            0 => Policy::Workspace,
            1 => Policy::Observed,
            2 => Policy::Drop(Edited::Tree(())),
            3 => Policy::Drop(Edited::NoTree),
            _ => Policy::Directional(rng.direction()),
        };
        let families = if rng.below(8) == 0 {
            None
        } else {
            Some((family(&mut rng), family(&mut rng)))
        };
        let facts: RelocationFacts<u64, u64, Direction, (), ()> = RelocationFacts {
            policy,
            source: 10,
            destination: 11,
            families,
            space: 1,
            destination_space: match rng.below(6) {
                0 => None,
                1 => Some(2),
                _ => Some(1),
            },
            owner: rng.maybe(|r| 10 + r.below(3)),
            projected_floating: rng.below(2) == 0,
            in_floats: rng.below(2) == 0,
            layout: if rng.below(4) == 0 { None } else { Some(()) },
            preserve_widths: rng.below(2) == 0,
            unavailable: rng.below(5) == 0,
            placeable: rng.below(5) > 0,
        };
        let rust = relocation(facts);
        let roc = rift_core_roc::placement::engine_relocation(facts);
        assert_eq!(roc, rust, "case {case}: {facts:?}");
        moved += u64::from(matches!(rust, rift_core::placement::Relocation::Move { .. }));
    }
    assert!(moved > 0);
    eprintln!("engine relocation: {runs} questions answered alike ({moved} moves)");
}

/// The remaining placement choices (`rift_core::placement::choose_place`): every kind, drawn at
/// random, with few workspaces and windows so moved windows include the preferred one about half
/// the time, and frames on a coarse grid.
#[test]
fn the_roc_core_makes_the_remaining_placement_choices_as_the_rust_core_does() {
    use rift_core::floats::{Centering, Sizing};
    use rift_core::placement::{
        AdditionFacts, AssignmentAsk, ClaimAsk, Family, FrameAsk, LandingAsk, MembersAsk,
        MovedFocusAsk, PlaceQuestion as Q, RuleFloatAsk, RuleFrameAsk, Tree, WindowMoveAsk,
        WorkspaceMoveAsk, choose_place,
    };
    let mut rng = Rng(0x91AC_E0E1_C4C0);
    let runs: u64 = std::env::var("RIFT_ENGINE_PLACE_CASES").map_or(20_000, |n| n.parse().unwrap());
    let coin = |r: &mut Rng| r.below(2) == 0;
    let family =
        |r: &mut Rng| [Family::Floating, Family::Scrolling, Family::Tiled][r.below(3) as usize];
    for case in 0..runs {
        let question: Q<u64, u64> = match rng.below(11) {
            0 => Q::Addition(AdditionFacts {
                admitted: rng.below(6) > 0,
                fullscreen_suspended: rng.below(6) == 0,
                workspace: rng.maybe(|r| (10 + r.below(2), family(r))),
                in_floats: coin(&mut rng),
                tree: rng.maybe(|r| Tree {
                    layout: (),
                    holds_window: r.below(2) == 0,
                }),
                preserve_widths: coin(&mut rng),
            }),
            1 => Q::WindowMove(WindowMoveAsk {
                from: rng.maybe(|r| 10 + r.below(2)),
                to: rng.maybe(|r| 10 + r.below(2)),
            }),
            2 => Q::Landing(LandingAsk { placeable: coin(&mut rng) }),
            3 => Q::WorkspaceMove(WorkspaceMoveAsk {
                shown: rng.maybe(|r| 10 + r.below(2)),
                per_display: coin(&mut rng),
                same_space: coin(&mut rng),
                index: rng.maybe(|r| r.below(4) as usize),
            }),
            4 => Q::Members(MembersAsk {
                found: rng.maybe(|r| 10 + r.below(2)),
            }),
            5 => {
                let n = rng.below(4);
                Q::MovedFocus(MovedFocusAsk {
                    preferred: rng.maybe(|r| 1 + r.below(4)),
                    moved: (0..n).map(|_| 1 + rng.below(4)).collect(),
                })
            }
            6 => Q::Claim(ClaimAsk {
                claimed: coin(&mut rng),
                management_override: rng.maybe(|r| r.below(2) == 0),
            }),
            7 => Q::Assignment(AssignmentAsk {
                restored: coin(&mut rng),
                reapply: coin(&mut rng),
                rule_names_workspace: coin(&mut rng),
            }),
            8 => Q::RuleFloat(RuleFloatAsk {
                should_float: coin(&mut rng),
                was_floating: coin(&mut rng),
            }),
            9 => Q::RuleFrame(RuleFrameAsk {
                place: rng.maybe(|_| ()),
                resize: rng.maybe(|_| ()),
            }),
            _ => {
                let mut grid = |r: &mut Rng| Rect {
                    x: 100.0 * r.below(10) as f64,
                    y: 50.0 * r.below(10) as f64,
                    w: 100.0 + 100.0 * r.below(10) as f64,
                    h: 80.0 + 40.0 * r.below(10) as f64,
                };
                let (frame, screen) = (grid(&mut rng), grid(&mut rng));
                let sizing = match rng.below(3) {
                    0 => Sizing::Keep,
                    1 => Sizing::Smart,
                    _ => Sizing::Dimensions {
                        w: 300.0 + rng.below(5) as f64 * 37.5,
                        h: 200.0,
                    },
                };
                let centering = if coin(&mut rng) {
                    Centering::Center
                } else {
                    Centering::Keep
                };
                Q::Frame(FrameAsk {
                    frame,
                    screen,
                    centering,
                    sizing,
                })
            }
        };
        let rust = choose_place(question.clone());
        let roc = rift_core_roc::placement::engine_place(question.clone());
        assert_eq!(roc, rust, "case {case}: {question:?}");
    }
    eprintln!("engine place: {runs} questions answered alike");
}
