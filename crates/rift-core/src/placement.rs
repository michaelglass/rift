//! Decisions about where a window goes when it is dropped, moved or placed: plans the layout
//! engine carries out, one variant per kind of outcome (a refusal names its reason). Generic over
//! the platform's window `W`, workspace `K`, layout direction `D` and place `P` (where an accepted
//! plan lands: the destination's Space and layout).

use crate::plain::Rect;

/// What a dropped window does to its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowDropAction<D> {
    Swap,
    Stack,
    Insert(D),
    Move(D),
    ConsumeOrExpel(D),
}

/// The action a drop takes: an insert into the slot the window holds already becomes the center
/// action (`center`), since inserting a window beside itself would change nothing.
pub fn drop_action<D>(
    same_slot: bool,
    action: WindowDropAction<D>,
    center: WindowDropAction<D>,
) -> WindowDropAction<D> {
    match (same_slot, action) {
        (true, WindowDropAction::Insert(_)) => center,
        (_, action) => action,
    }
}

/// The layout family a drop reads from the shown workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrangement {
    Floating,
    Stack,
    Tiled,
}

/// Whether a drag's zones decide the action, or a layout overrides them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Override {
    /// The zone under the pointer decides (a scene before its layout is known).
    #[default]
    ByZone,
    /// Every zone swaps (a stack has no sides to insert at).
    Swap,
}

/// The drag override on a Space showing `shown` (none: no workspace, so the zones decide).
pub fn drop_override(shown: Option<Arrangement>) -> Override {
    match shown {
        Some(Arrangement::Stack) => Override::Swap,
        Some(Arrangement::Floating | Arrangement::Tiled) | None => Override::ByZone,
    }
}

/// Whether a drop onto a tile edits the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropEdit {
    /// A floating layout has no tiles to drop onto.
    Refused,
    Edit,
}

/// Whether a drop onto a tile edits a layout of `arrangement`.
pub fn drop_edit(arrangement: Arrangement) -> DropEdit {
    match arrangement {
        Arrangement::Floating => DropEdit::Refused,
        Arrangement::Stack | Arrangement::Tiled => DropEdit::Edit,
    }
}

/// The destination of a drop, as far as it is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Laid<P> {
    /// The destination workspace is on no Space.
    NoSpace,
    /// Its Space has no layout for it.
    NoLayout,
    Laid(P),
}

/// A drop's target: the window under the pointer and what the drop does to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DropTarget<W, D> {
    pub window: W,
    pub action: WindowDropAction<D>,
    /// It is a member of the destination workspace.
    pub in_destination: bool,
    pub floating: bool,
}

/// What a drop request is checked against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropFacts<W, K, D, P> {
    pub window: W,
    /// The window's workspace, if rift holds it.
    pub source: Option<K>,
    pub destination: K,
    pub laid: Laid<P>,
    pub floating: bool,
    /// The window's floating position in the destination, if it has one.
    pub at: Option<Rect>,
    /// The frame the request drops a floating window at.
    pub frame: Option<Rect>,
    pub target: Option<DropTarget<W, D>>,
}

/// Why a drop is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropRefusal {
    /// The window has no workspace rift holds.
    NotAMember,
    /// The destination workspace is on no Space.
    NoDestination,
    /// The destination has no layout on its Space.
    NoLayout,
    /// A floating window dropped without a frame.
    FloatWithoutFrame,
    /// A floating window's frame is not finite, or not of positive size.
    BadFrame,
    /// A floating window dropped onto a tile.
    FloatOntoTarget,
    /// A floating window dropped where it is, in its own workspace.
    FloatUnmoved,
    /// A tiled window dropped with a floating frame.
    TiledWithFrame,
    /// A tiled window inserted beside itself (only moving or consuming it into itself means
    /// something).
    SelfInsert,
    /// The target is not a member of the destination workspace.
    ForeignTarget,
    /// The target floats: there is no tile to drop onto.
    FloatingTarget,
    /// A tiled window dropped on no tile, in its own workspace.
    InPlace,
}

/// Whether a drop crosses workspaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Across<K> {
    /// Within the window's own workspace.
    Same,
    /// From workspace `from` into the destination (a relocation).
    Into { from: K },
}

/// What a drop does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DropPlan<W, K, D, P> {
    Refused(DropRefusal),
    /// A floating window dropped at a frame.
    Float {
        frame: Rect,
        across: Across<K>,
        place: P,
    },
    /// A tiled window dropped on no tile, into another workspace.
    Relocate {
        from: K,
        place: P,
    },
    /// A tiled window dropped onto a tile: the layout edits a prospective tree, then
    /// [`target_kept`] says whether it moved the window.
    Target {
        target: W,
        action: WindowDropAction<D>,
        across: Across<K>,
        place: P,
    },
}

fn usable(frame: Rect) -> bool {
    [frame.x, frame.y, frame.w, frame.h].iter().all(|v| v.is_finite())
        && frame.w > 0.0
        && frame.h > 0.0
}

/// What a drop does, from its facts.
pub fn drop_plan<W: PartialEq, K: Copy + PartialEq, D, P>(
    facts: DropFacts<W, K, D, P>,
) -> DropPlan<W, K, D, P> {
    let Some(source) = facts.source else {
        return DropPlan::Refused(DropRefusal::NotAMember);
    };
    let place = match facts.laid {
        Laid::NoSpace => return DropPlan::Refused(DropRefusal::NoDestination),
        Laid::NoLayout => return DropPlan::Refused(DropRefusal::NoLayout),
        Laid::Laid(place) => place,
    };
    let across = if source == facts.destination {
        Across::Same
    } else {
        Across::Into { from: source }
    };
    if facts.floating {
        let Some(frame) = facts.frame else {
            return DropPlan::Refused(DropRefusal::FloatWithoutFrame);
        };
        if !usable(frame) {
            return DropPlan::Refused(DropRefusal::BadFrame);
        }
        if facts.target.is_some() {
            return DropPlan::Refused(DropRefusal::FloatOntoTarget);
        }
        if across == Across::Same && facts.at == Some(frame) {
            return DropPlan::Refused(DropRefusal::FloatUnmoved);
        }
        return DropPlan::Float { frame, across, place };
    }
    if facts.frame.is_some() {
        return DropPlan::Refused(DropRefusal::TiledWithFrame);
    }
    match (facts.target, across) {
        (Some(target), _) => {
            let into_itself = matches!(
                target.action,
                WindowDropAction::Move(_) | WindowDropAction::ConsumeOrExpel(_)
            );
            if target.window == facts.window && !into_itself {
                DropPlan::Refused(DropRefusal::SelfInsert)
            } else if !target.in_destination {
                DropPlan::Refused(DropRefusal::ForeignTarget)
            } else if target.floating {
                DropPlan::Refused(DropRefusal::FloatingTarget)
            } else {
                DropPlan::Target {
                    target: target.window,
                    action: target.action,
                    across,
                    place,
                }
            }
        }
        (None, Across::Into { from }) => DropPlan::Relocate { from, place },
        (None, Across::Same) => DropPlan::Refused(DropRefusal::InPlace),
    }
}

/// Whether a target drop's prospective tree moved the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    /// It holds the slot it held: the drop is refused.
    Unchanged,
    Moved,
}

/// Whether the window's slot `after` the prospective edit is the one it held `before` (a window
/// with no slot before, joining the tree, always moved).
pub fn target_kept<T: PartialEq>(before: Option<T>, after: Option<T>) -> Kept {
    match before {
        Some(before) if after.as_ref() == Some(&before) => Kept::Unchanged,
        Some(_) | None => Kept::Moved,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type A = WindowDropAction<u8>;

    fn rect(x: f64) -> Rect { Rect { x, y: 0.0, w: 100.0, h: 100.0 } }

    fn tiled(target: Option<DropTarget<u32, u8>>) -> DropFacts<u32, u32, u8, &'static str> {
        DropFacts {
            window: 1,
            source: Some(10),
            destination: 10,
            laid: Laid::Laid("here"),
            floating: false,
            at: None,
            frame: None,
            target,
        }
    }

    fn onto(window: u32, action: A) -> Option<DropTarget<u32, u8>> {
        Some(DropTarget {
            window,
            action,
            in_destination: true,
            floating: false,
        })
    }

    #[test]
    fn an_insert_into_its_own_slot_takes_the_center_action() {
        assert_eq!(drop_action(true, A::Insert(1), A::Stack), A::Stack);
        assert_eq!(drop_action(false, A::Insert(1), A::Stack), A::Insert(1));
        assert_eq!(drop_action(true, A::Move(1), A::Stack), A::Move(1));
    }

    #[test]
    fn only_a_stack_overrides_the_zones_and_only_a_floating_layout_refuses_an_edit() {
        assert_eq!(drop_override(Some(Arrangement::Stack)), Override::Swap);
        assert_eq!(drop_override(Some(Arrangement::Tiled)), Override::ByZone);
        assert_eq!(drop_override(Some(Arrangement::Floating)), Override::ByZone);
        assert_eq!(drop_override(None), Override::ByZone);
        assert_eq!(drop_edit(Arrangement::Floating), DropEdit::Refused);
        assert_eq!(drop_edit(Arrangement::Stack), DropEdit::Edit);
        assert_eq!(drop_edit(Arrangement::Tiled), DropEdit::Edit);
    }

    #[test]
    fn a_drop_without_a_member_or_a_laid_destination_is_refused() {
        let facts = tiled(onto(2, A::Swap));
        let r = |facts| drop_plan(facts);
        assert_eq!(
            r(DropFacts { source: None, ..facts }),
            DropPlan::Refused(DropRefusal::NotAMember)
        );
        assert_eq!(
            r(DropFacts { laid: Laid::NoSpace, ..facts }),
            DropPlan::Refused(DropRefusal::NoDestination)
        );
        assert_eq!(
            r(DropFacts { laid: Laid::NoLayout, ..facts }),
            DropPlan::Refused(DropRefusal::NoLayout)
        );
    }

    #[test]
    fn a_floating_drop_needs_a_usable_new_frame_and_no_target() {
        let floating = DropFacts {
            floating: true,
            frame: Some(rect(0.0)),
            ..tiled(None)
        };
        let r = |facts| drop_plan(facts);
        assert_eq!(
            r(DropFacts { frame: None, ..floating }),
            DropPlan::Refused(DropRefusal::FloatWithoutFrame)
        );
        for bad in [
            Rect { w: 0.0, ..rect(0.0) },
            Rect { h: -1.0, ..rect(0.0) },
            Rect { x: f64::NAN, ..rect(0.0) },
            Rect { y: f64::INFINITY, ..rect(0.0) },
        ] {
            assert_eq!(
                r(DropFacts { frame: Some(bad), ..floating }),
                DropPlan::Refused(DropRefusal::BadFrame)
            );
        }
        assert_eq!(
            r(DropFacts {
                target: onto(2, A::Swap),
                ..floating
            }),
            DropPlan::Refused(DropRefusal::FloatOntoTarget)
        );
        assert_eq!(
            r(DropFacts {
                at: Some(rect(0.0)),
                ..floating
            }),
            DropPlan::Refused(DropRefusal::FloatUnmoved)
        );
        assert_eq!(
            r(DropFacts {
                at: Some(rect(5.0)),
                ..floating
            }),
            DropPlan::Float {
                frame: rect(0.0),
                across: Across::Same,
                place: "here"
            }
        );
        // Into another workspace, the same frame is a move.
        assert_eq!(
            r(DropFacts {
                at: Some(rect(0.0)),
                source: Some(11),
                ..floating
            }),
            DropPlan::Float {
                frame: rect(0.0),
                across: Across::Into { from: 11 },
                place: "here"
            }
        );
    }

    #[test]
    fn a_tiled_drop_needs_no_frame_and_a_tiled_target_of_the_destination() {
        let r = |facts| drop_plan(facts);
        assert_eq!(
            r(DropFacts {
                frame: Some(rect(0.0)),
                ..tiled(None)
            }),
            DropPlan::Refused(DropRefusal::TiledWithFrame)
        );
        for action in [A::Swap, A::Stack, A::Insert(1)] {
            assert_eq!(
                r(tiled(onto(1, action))),
                DropPlan::Refused(DropRefusal::SelfInsert)
            );
        }
        for action in [A::Move(1), A::ConsumeOrExpel(1)] {
            assert_eq!(r(tiled(onto(1, action))), DropPlan::Target {
                target: 1,
                action,
                across: Across::Same,
                place: "here"
            });
        }
        let foreign = Some(DropTarget {
            window: 2,
            action: A::Swap,
            in_destination: false,
            floating: false,
        });
        assert_eq!(r(tiled(foreign)), DropPlan::Refused(DropRefusal::ForeignTarget));
        let floating = Some(DropTarget {
            window: 2,
            action: A::Swap,
            in_destination: true,
            floating: true,
        });
        assert_eq!(
            r(tiled(floating)),
            DropPlan::Refused(DropRefusal::FloatingTarget)
        );
        assert_eq!(r(tiled(None)), DropPlan::Refused(DropRefusal::InPlace));
        assert_eq!(
            r(DropFacts {
                source: Some(11),
                ..tiled(None)
            }),
            DropPlan::Relocate { from: 11, place: "here" }
        );
        assert_eq!(
            r(DropFacts {
                source: Some(11),
                ..tiled(onto(2, A::Swap))
            }),
            DropPlan::Target {
                target: 2,
                action: A::Swap,
                across: Across::Into { from: 11 },
                place: "here"
            }
        );
    }

    #[test]
    fn a_target_drop_that_leaves_the_window_in_its_slot_is_kept() {
        assert_eq!(target_kept(Some(3), Some(3)), Kept::Unchanged);
        assert_eq!(target_kept(Some(3), Some(4)), Kept::Moved);
        assert_eq!(target_kept(Some(3), None), Kept::Moved);
        assert_eq!(target_kept(None::<u8>, None), Kept::Moved);
        assert_eq!(target_kept(None, Some(4)), Kept::Moved);
    }
}
