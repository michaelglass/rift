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

impl<D> WindowDropAction<D> {
    /// The action with its direction renamed.
    pub fn map<D2>(self, d: impl FnOnce(D) -> D2) -> WindowDropAction<D2> {
        match self {
            WindowDropAction::Swap => WindowDropAction::Swap,
            WindowDropAction::Stack => WindowDropAction::Stack,
            WindowDropAction::Insert(x) => WindowDropAction::Insert(d(x)),
            WindowDropAction::Move(x) => WindowDropAction::Move(d(x)),
            WindowDropAction::ConsumeOrExpel(x) => WindowDropAction::ConsumeOrExpel(d(x)),
        }
    }
}

impl<W, K, D, P> DropFacts<W, K, D, P> {
    /// The facts with the place replaced by a token: a twin decides on whether there is one,
    /// never on the place itself.
    pub fn without_place(self) -> DropFacts<W, K, D, ()> {
        DropFacts {
            window: self.window,
            source: self.source,
            destination: self.destination,
            laid: match self.laid {
                Laid::NoSpace => Laid::NoSpace,
                Laid::NoLayout => Laid::NoLayout,
                Laid::Laid(_) => Laid::Laid(()),
            },
            floating: self.floating,
            at: self.at,
            frame: self.frame,
            target: self.target,
        }
    }

    /// The facts with their windows, workspaces and directions renamed.
    pub fn map<W2, K2, D2>(
        self,
        w: impl Fn(W) -> W2,
        k: impl Fn(K) -> K2,
        d: impl Fn(D) -> D2,
    ) -> DropFacts<W2, K2, D2, P> {
        DropFacts {
            window: w(self.window),
            source: self.source.map(&k),
            destination: k(self.destination),
            laid: self.laid,
            floating: self.floating,
            at: self.at,
            frame: self.frame,
            target: self.target.map(|t| DropTarget {
                window: w(t.window),
                action: t.action.map(&d),
                in_destination: t.in_destination,
                floating: t.floating,
            }),
        }
    }
}

impl<W, K, D, P> DropPlan<W, K, D, P> {
    /// The plan without its place (the wire form).
    pub fn without_place(self) -> DropPlan<W, K, D, ()> {
        match self {
            DropPlan::Refused(r) => DropPlan::Refused(r),
            DropPlan::Float { frame, across, .. } => DropPlan::Float { frame, across, place: () },
            DropPlan::Relocate { from, .. } => DropPlan::Relocate { from, place: () },
            DropPlan::Target { target, action, across, .. } => DropPlan::Target {
                target,
                action,
                across,
                place: (),
            },
        }
    }
}

impl<W, K, D> DropPlan<W, K, D, ()> {
    /// A wire plan with the engine's place put back; none if it accepts a drop the facts give
    /// no place for (a twin naming a payload the engine does not hold).
    pub fn with_place<P>(self, laid: Laid<P>) -> Option<DropPlan<W, K, D, P>> {
        match (self, laid) {
            (DropPlan::Refused(r), _) => Some(DropPlan::Refused(r)),
            (_, Laid::NoSpace | Laid::NoLayout) => None,
            (DropPlan::Float { frame, across, .. }, Laid::Laid(place)) => {
                Some(DropPlan::Float { frame, across, place })
            }
            (DropPlan::Relocate { from, .. }, Laid::Laid(place)) => {
                Some(DropPlan::Relocate { from, place })
            }
            (DropPlan::Target { target, action, across, .. }, Laid::Laid(place)) => {
                Some(DropPlan::Target { target, action, across, place })
            }
        }
    }

    /// The plan with its windows, workspaces and directions renamed; none where a word names
    /// nothing.
    pub fn map<W2, K2, D2>(
        self,
        w: impl Fn(W) -> Option<W2>,
        k: impl Fn(K) -> Option<K2>,
        d: impl Fn(D) -> D2,
    ) -> Option<DropPlan<W2, K2, D2, ()>> {
        let across = |a: Across<K>| -> Option<Across<K2>> {
            Some(match a {
                Across::Same => Across::Same,
                Across::Into { from } => Across::Into { from: k(from)? },
            })
        };
        Some(match self {
            DropPlan::Refused(r) => DropPlan::Refused(r),
            DropPlan::Float { frame, across: a, .. } => DropPlan::Float {
                frame,
                across: across(a)?,
                place: (),
            },
            DropPlan::Relocate { from, .. } => DropPlan::Relocate { from: k(from)?, place: () },
            DropPlan::Target { target, action, across: a, .. } => DropPlan::Target {
                target: w(target)?,
                action: action.map(d),
                across: across(a)?,
                place: (),
            },
        })
    }
}

/// The layout engine's drop choices, as a question (the wire form a Roc core answers):
/// payloads the engine holds (the place, the trees) are tokens.
#[derive(Debug, Clone, PartialEq)]
pub enum DropQuestion<W, K, D> {
    Plan(DropFacts<W, K, D, ()>),
    Kept(KeptAsk),
    Action(ActionAsk<D>),
    Override(OverrideAsk),
    Edit(Arrangement),
}

/// A drop choice's answer, in its wire form.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DropChoice<W, K, D> {
    Plan(DropPlan<W, K, D, ()>),
    Kept(Kept),
    Action(WindowDropAction<D>),
    Override(Override),
    Edit(DropEdit),
}

/// The answer to `question` (the wire form's), from the functions above.
pub fn choose_drop<W: Copy + PartialEq, K: Copy + PartialEq, D: Copy>(
    question: DropQuestion<W, K, D>,
) -> DropChoice<W, K, D> {
    match question {
        DropQuestion::Plan(facts) => DropChoice::Plan(drop_plan(facts)),
        DropQuestion::Kept(ask) => DropChoice::Kept(ask.answer_kept()),
        DropQuestion::Action(ask) => {
            DropChoice::Action(drop_action(ask.same_slot, ask.action, ask.center))
        }
        DropQuestion::Override(ask) => DropChoice::Override(drop_override(ask.0)),
        DropQuestion::Edit(arrangement) => DropChoice::Edit(drop_edit(arrangement)),
    }
}

impl<W: Copy, K: Copy, D: Copy> DropQuestion<W, K, D> {
    /// The question with its windows, workspaces and directions renamed.
    pub fn map<W2, K2, D2>(
        self,
        w: impl Fn(W) -> W2,
        k: impl Fn(K) -> K2,
        d: impl Fn(D) -> D2,
    ) -> DropQuestion<W2, K2, D2> {
        match self {
            DropQuestion::Plan(facts) => DropQuestion::Plan(facts.map(w, k, d)),
            DropQuestion::Kept(ask) => DropQuestion::Kept(ask),
            DropQuestion::Action(ask) => DropQuestion::Action(ActionAsk {
                same_slot: ask.same_slot,
                action: ask.action.map(&d),
                center: ask.center.map(&d),
            }),
            DropQuestion::Override(ask) => DropQuestion::Override(ask),
            DropQuestion::Edit(arrangement) => DropQuestion::Edit(arrangement),
        }
    }
}

impl<W, K, D> DropChoice<W, K, D> {
    /// The answer with its words renamed; none where a word names nothing.
    pub fn map<W2, K2, D2>(
        self,
        w: impl Fn(W) -> Option<W2>,
        k: impl Fn(K) -> Option<K2>,
        d: impl Fn(D) -> D2,
    ) -> Option<DropChoice<W2, K2, D2>> {
        Some(match self {
            DropChoice::Plan(plan) => DropChoice::Plan(plan.map(w, k, d)?),
            DropChoice::Kept(kept) => DropChoice::Kept(kept),
            DropChoice::Action(action) => DropChoice::Action(action.map(d)),
            DropChoice::Override(o) => DropChoice::Override(o),
            DropChoice::Edit(e) => DropChoice::Edit(e),
        })
    }
}

/// [`target_kept`]'s facts: the window's slot (a path in the tree) before and after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptAsk {
    pub before: Option<Vec<usize>>,
    pub after: Option<Vec<usize>>,
}

impl KeptAsk {
    fn answer_kept(&self) -> Kept { target_kept(self.before.as_ref(), self.after.as_ref()) }
}

/// [`drop_action`]'s facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionAsk<D> {
    pub same_slot: bool,
    pub action: WindowDropAction<D>,
    pub center: WindowDropAction<D>,
}

/// [`drop_override`]'s fact: the shown workspace's arrangement (none: no workspace).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverrideAsk(pub Option<Arrangement>);

/// One kind of drop question with its own answer: callers see `Answer`, never a
/// [`DropChoice`]. `of` reads only a choice of its own kind, and puts back the payloads the
/// engine holds (none: a twin's bug, its answer of another kind or naming a payload the engine
/// does not hold).
pub trait DropAsk<W, K, D> {
    type Answer: PartialEq + std::fmt::Debug;
    /// The Rust core's answer.
    fn answer(&self) -> Self::Answer;
    /// The question in its wire form.
    fn question(&self) -> DropQuestion<W, K, D>;
    /// An answer in its wire form.
    fn choice(answer: &Self::Answer) -> DropChoice<W, K, D>;
    /// A wire answer as this kind's answer, its payloads put back.
    fn of(&self, choice: DropChoice<W, K, D>) -> Option<Self::Answer>;
}

impl<
    W: Copy + PartialEq + std::fmt::Debug,
    K: Copy + PartialEq + std::fmt::Debug,
    D: Copy + PartialEq + std::fmt::Debug,
    P: Copy + PartialEq + std::fmt::Debug,
> DropAsk<W, K, D> for DropFacts<W, K, D, P>
{
    type Answer = DropPlan<W, K, D, P>;

    fn answer(&self) -> Self::Answer { drop_plan(*self) }

    fn question(&self) -> DropQuestion<W, K, D> { DropQuestion::Plan(self.without_place()) }

    fn choice(answer: &Self::Answer) -> DropChoice<W, K, D> {
        DropChoice::Plan(answer.without_place())
    }

    fn of(&self, choice: DropChoice<W, K, D>) -> Option<Self::Answer> {
        match choice {
            DropChoice::Plan(plan) => plan.with_place(self.laid),
            DropChoice::Kept(_)
            | DropChoice::Action(_)
            | DropChoice::Override(_)
            | DropChoice::Edit(_) => None,
        }
    }
}

impl<W, K, D> DropAsk<W, K, D> for KeptAsk {
    type Answer = Kept;

    fn answer(&self) -> Kept { self.answer_kept() }

    fn question(&self) -> DropQuestion<W, K, D> { DropQuestion::Kept(self.clone()) }

    fn choice(answer: &Kept) -> DropChoice<W, K, D> { DropChoice::Kept(*answer) }

    fn of(&self, choice: DropChoice<W, K, D>) -> Option<Kept> {
        match choice {
            DropChoice::Kept(kept) => Some(kept),
            DropChoice::Plan(_)
            | DropChoice::Action(_)
            | DropChoice::Override(_)
            | DropChoice::Edit(_) => None,
        }
    }
}

impl<W, K, D: Copy + PartialEq + std::fmt::Debug> DropAsk<W, K, D> for ActionAsk<D> {
    type Answer = WindowDropAction<D>;

    fn answer(&self) -> WindowDropAction<D> {
        drop_action(self.same_slot, self.action, self.center)
    }

    fn question(&self) -> DropQuestion<W, K, D> { DropQuestion::Action(*self) }

    fn choice(answer: &WindowDropAction<D>) -> DropChoice<W, K, D> { DropChoice::Action(*answer) }

    fn of(&self, choice: DropChoice<W, K, D>) -> Option<WindowDropAction<D>> {
        match choice {
            DropChoice::Action(action) => Some(action),
            DropChoice::Plan(_)
            | DropChoice::Kept(_)
            | DropChoice::Override(_)
            | DropChoice::Edit(_) => None,
        }
    }
}

impl<W, K, D> DropAsk<W, K, D> for OverrideAsk {
    type Answer = Override;

    fn answer(&self) -> Override { drop_override(self.0) }

    fn question(&self) -> DropQuestion<W, K, D> { DropQuestion::Override(*self) }

    fn choice(answer: &Override) -> DropChoice<W, K, D> { DropChoice::Override(*answer) }

    fn of(&self, choice: DropChoice<W, K, D>) -> Option<Override> {
        match choice {
            DropChoice::Override(o) => Some(o),
            DropChoice::Plan(_)
            | DropChoice::Kept(_)
            | DropChoice::Action(_)
            | DropChoice::Edit(_) => None,
        }
    }
}

impl<W, K, D> DropAsk<W, K, D> for Arrangement {
    type Answer = DropEdit;

    fn answer(&self) -> DropEdit { drop_edit(*self) }

    fn question(&self) -> DropQuestion<W, K, D> { DropQuestion::Edit(*self) }

    fn choice(answer: &DropEdit) -> DropChoice<W, K, D> { DropChoice::Edit(*answer) }

    fn of(&self, choice: DropChoice<W, K, D>) -> Option<DropEdit> {
        match choice {
            DropChoice::Edit(e) => Some(e),
            DropChoice::Plan(_)
            | DropChoice::Kept(_)
            | DropChoice::Action(_)
            | DropChoice::Override(_) => None,
        }
    }
}

/// Why a window is relocated, which decides how its floating state travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy<D, T> {
    /// A command moved it to a workspace.
    Workspace,
    /// Observation found it on its new workspace (an app rule may have assigned it already).
    Observed,
    /// A drop moved it, with or without a prospective tree `T` for the destination.
    Drop(Edited<T>),
    /// A directional move: it is inserted toward `D` and its floating state is left alone.
    Directional(D),
}

/// Whether a drop brought a prospective tree for the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edited<T> {
    Tree(T),
    NoTree,
}

/// A workspace's layout family, as relocation reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// A floating layout system: windows in it are tiled into it, never floated over it.
    Floating,
    Scrolling,
    Tiled,
}

/// What a relocation is decided from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelocationFacts<K, S, D, T, L> {
    pub policy: Policy<D, T>,
    pub source: K,
    pub destination: K,
    /// The source's and the destination's families, if both are workspaces rift holds.
    pub families: Option<(Family, Family)>,
    /// The Space the window goes to, and the Space the destination is on.
    pub space: S,
    pub destination_space: Option<S>,
    /// The window's workspace now.
    pub owner: Option<K>,
    /// The window floats, as the engine's projection says (`is_window_floating`).
    pub projected_floating: bool,
    /// The window is in the floating set.
    pub in_floats: bool,
    /// The destination's layout on the Space, if it has one.
    pub layout: Option<L>,
    pub preserve_widths: bool,
    pub unavailable: bool,
    pub placeable: bool,
}

/// Why a relocation is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocationRefusal {
    /// The source or the destination is not a workspace rift holds.
    UnknownWorkspace,
    /// The destination is not on the Space the window goes to.
    OtherSpace,
    /// The window belongs to a third workspace (only observation may find it on the destination
    /// already, assigned by an app rule).
    OwnedElsewhere,
    /// A directional move into a destination with no layout to insert into.
    DirectionalWithoutLayout,
}

/// Whether the window must be assigned to the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assign {
    /// It belongs there already.
    Already,
    Assign,
}

/// Whether a scrolling destination keeps the window's width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Preserve,
    AsIs,
}

/// How a window goes into the destination's tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placed<Wd, T, L> {
    /// The drop's prospective tree becomes the destination's.
    Edited { tree: T, width: Wd },
    /// It is added to the destination's layout.
    Add { layout: L, width: Wd },
    /// The destination has no layout on the Space: nothing is inserted.
    NoLayout,
}

/// Whether a window floated in the destination was in the destination's tree already.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observed {
    /// Observation found it a member: it leaves the destination's tree.
    WasMember,
    Moved,
}

/// What the window becomes in the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection<D, T, L> {
    /// It floats over the destination (it floated, and the destination tiles).
    Float { observed: Observed },
    /// It floated, and the destination is a floating layout: it leaves the floating set (its
    /// last focus kept) and is tiled into that layout (never scrolling, so no width to keep).
    Unfloat { placed: Placed<(), T, L> },
    /// It is tiled, its width kept or not.
    Tile { placed: Placed<Width, T, L> },
    /// A directional move: inserted toward `D` into the destination's layout (there is one: a
    /// directional move without one is refused), its floating state untouched.
    Toward {
        direction: D,
        width: Width,
        layout: L,
    },
}

/// The window's state after it lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Afterwards {
    /// It is unavailable, or a command moved it where it cannot be placed.
    Suspended,
    /// Observation found it: it takes part in the destination's layout.
    Resumed,
    AsIs,
}

/// What relocating a window does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relocation<D, T, L> {
    Refused(RelocationRefusal),
    Move {
        assign: Assign,
        projection: Projection<D, T, L>,
        afterwards: Afterwards,
    },
}

/// How a window goes into the destination's tree: the drop's tree, else an addition if there is
/// a layout.
fn placed<Wd, T, L>(edited: Edited<T>, layout: Option<L>, width: Wd) -> Placed<Wd, T, L> {
    match (edited, layout) {
        (Edited::Tree(tree), _) => Placed::Edited { tree, width },
        (Edited::NoTree, Some(layout)) => Placed::Add { layout, width },
        (Edited::NoTree, None) => Placed::NoLayout,
    }
}

impl<Wd, T, L> Placed<Wd, T, L> {
    /// The same placement with its width answer replaced.
    pub fn with_width<W2>(self, width: impl FnOnce(Wd) -> W2) -> Placed<W2, T, L> {
        match self {
            Placed::Edited { tree, width: w } => Placed::Edited { tree, width: width(w) },
            Placed::Add { layout, width: w } => Placed::Add { layout, width: width(w) },
            Placed::NoLayout => Placed::NoLayout,
        }
    }
}

/// A move that is not directional: the window floats, unfloats into a floating layout, or tiles.
fn projected<D, T, L>(
    floating: bool,
    observed: Observed,
    into_floating_layout: bool,
    edited: Edited<T>,
    width: Width,
    layout: Option<L>,
) -> Projection<D, T, L> {
    match (floating, into_floating_layout) {
        (true, false) => Projection::Float { observed },
        (true, true) => Projection::Unfloat {
            placed: placed(edited, layout, ()),
        },
        (false, _) => Projection::Tile {
            placed: placed(edited, layout, width),
        },
    }
}

/// What relocating a window does, from its facts.
pub fn relocation<K: Copy + PartialEq, S: PartialEq, D, T, L>(
    facts: RelocationFacts<K, S, D, T, L>,
) -> Relocation<D, T, L> {
    let RelocationFacts {
        policy,
        source,
        destination,
        families,
        space,
        destination_space,
        owner,
        projected_floating,
        in_floats,
        layout,
        preserve_widths,
        unavailable,
        placeable,
    } = facts;
    let Some((source_family, destination_family)) = families else {
        return Relocation::Refused(RelocationRefusal::UnknownWorkspace);
    };
    let observed = matches!(policy, Policy::Observed);
    let commanded = matches!(policy, Policy::Workspace);
    if destination_space.as_ref() != Some(&space) {
        return Relocation::Refused(RelocationRefusal::OtherSpace);
    }
    if let Some(owner) = owner
        && owner != source
        && !(observed && owner == destination)
    {
        return Relocation::Refused(RelocationRefusal::OwnedElsewhere);
    }
    let width = |has_layout: bool| match (destination_family, preserve_widths && has_layout) {
        (Family::Scrolling, true) => Width::Preserve,
        _ => Width::AsIs,
    };
    let floating_layouts =
        source_family == Family::Floating || destination_family == Family::Floating;
    let into_floating_layout = destination_family == Family::Floating;
    let laid = layout.is_some();
    let projection = match policy {
        Policy::Directional(direction) => match layout {
            None => return Relocation::Refused(RelocationRefusal::DirectionalWithoutLayout),
            Some(layout) => Projection::Toward {
                direction,
                width: width(true),
                layout,
            },
        },
        // A command does not float a window into or out of a floating layout.
        Policy::Workspace => projected(
            in_floats && !floating_layouts,
            Observed::Moved,
            into_floating_layout,
            Edited::NoTree,
            width(laid),
            layout,
        ),
        Policy::Observed => projected(
            projected_floating,
            Observed::WasMember,
            into_floating_layout,
            Edited::NoTree,
            width(laid),
            layout,
        ),
        Policy::Drop(edited) => projected(
            projected_floating,
            Observed::Moved,
            into_floating_layout,
            edited,
            width(laid),
            layout,
        ),
    };
    let assign = if owner == Some(destination) {
        Assign::Already
    } else {
        Assign::Assign
    };
    let afterwards = if unavailable || (commanded && !placeable) {
        Afterwards::Suspended
    } else if observed {
        Afterwards::Resumed
    } else {
        Afterwards::AsIs
    };
    Relocation::Move { assign, projection, afterwards }
}

/// Why a window observed on a Space is not added to a layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skipped {
    /// It is not admitted for management.
    Unadmitted,
    /// It is in native fullscreen, suspended from the layout.
    FullscreenSuspended,
    /// The Space's workspaces assign it to none.
    Unassigned,
}

/// The destination's tree for an added window: its layout on the Space, and whether it holds
/// the window already.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tree<L> {
    pub layout: L,
    pub holds_window: bool,
}

/// What adding a window is decided from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdditionFacts<K, L> {
    pub admitted: bool,
    pub fullscreen_suspended: bool,
    /// The workspace the window is assigned to on the Space, and its family.
    pub workspace: Option<(K, Family)>,
    pub in_floats: bool,
    /// The workspace's layout on the Space, if it has one.
    pub tree: Option<Tree<L>>,
    pub preserve_widths: bool,
}

/// How an added window goes into its workspace's tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Insert<L, Wd> {
    /// The tree holds it already.
    Present,
    Add {
        layout: L,
        width: Wd,
    },
    /// The workspace has no layout on the Space (a warning: it is not added).
    NoLayout,
}

/// What adding an observed window to its workspace's layout does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Addition<K, L> {
    Skip(Skipped),
    /// It floats: nothing is inserted.
    Float {
        workspace: K,
    },
    /// It is tiled, its width kept or not.
    Tile {
        workspace: K,
        insert: Insert<L, Width>,
    },
    /// Its workspace is a floating layout: it leaves the floating set and joins that layout
    /// (never scrolling, so no width to keep).
    IntoFloatingLayout {
        workspace: K,
        insert: Insert<L, ()>,
    },
}

fn insert<L, Wd>(tree: Option<Tree<L>>, width: Wd) -> Insert<L, Wd> {
    match tree {
        None => Insert::NoLayout,
        Some(Tree { holds_window: true, .. }) => Insert::Present,
        Some(Tree { layout, holds_window: false }) => Insert::Add { layout, width },
    }
}

impl<L, Wd> Insert<L, Wd> {
    /// The same insertion with its width answer replaced.
    pub fn with_width<W2>(self, width: impl FnOnce(Wd) -> W2) -> Insert<L, W2> {
        match self {
            Insert::Present => Insert::Present,
            Insert::Add { layout, width: w } => Insert::Add { layout, width: width(w) },
            Insert::NoLayout => Insert::NoLayout,
        }
    }
}

/// What adding an observed window does, from its facts.
pub fn addition<K, L>(facts: AdditionFacts<K, L>) -> Addition<K, L> {
    if !facts.admitted {
        return Addition::Skip(Skipped::Unadmitted);
    }
    if facts.fullscreen_suspended {
        return Addition::Skip(Skipped::FullscreenSuspended);
    }
    let Some((workspace, family)) = facts.workspace else {
        return Addition::Skip(Skipped::Unassigned);
    };
    match (family, facts.in_floats) {
        (Family::Floating, _) => Addition::IntoFloatingLayout {
            workspace,
            insert: insert(facts.tree, ()),
        },
        (Family::Scrolling | Family::Tiled, true) => Addition::Float { workspace },
        (Family::Scrolling, false) if facts.preserve_widths => Addition::Tile {
            workspace,
            insert: insert(facts.tree, Width::Preserve),
        },
        (Family::Scrolling | Family::Tiled, false) => Addition::Tile {
            workspace,
            insert: insert(facts.tree, Width::AsIs),
        },
    }
}

/// Why moving a window to another Space is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowMoveRefusal {
    /// The window is in no workspace of its Space.
    NotAMember,
    /// The destination Space shows no workspace.
    NoWorkspace,
}

/// What moving a window to another Space does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowMove<K> {
    Refused(WindowMoveRefusal),
    /// From its workspace into the destination's shown one.
    Move {
        from: K,
        to: K,
    },
}

/// What moving a window from workspace `from` to a Space showing `to` does.
pub fn window_move<K>(from: Option<K>, to: Option<K>) -> WindowMove<K> {
    match (from, to) {
        (None, _) => WindowMove::Refused(WindowMoveRefusal::NotAMember),
        (Some(_), None) => WindowMove::Refused(WindowMoveRefusal::NoWorkspace),
        (Some(from), Some(to)) => WindowMove::Move { from, to },
    }
}

/// How a moved window lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landing {
    /// It cannot be placed there (hidden, minimized, claimed): the move is a change, nothing more.
    Hidden,
    /// It is focused and raised there.
    Focused,
}

/// How a moved window lands, by whether it can be placed.
pub fn landing(placeable: bool) -> Landing {
    if placeable {
        Landing::Focused
    } else {
        Landing::Hidden
    }
}

/// Why moving the shown workspace to another Space is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceMoveRefusal {
    /// The Space shows no workspace.
    Nothing,
    /// Per display, onto the Space it is on.
    SameSpace,
    /// Per display, the shown workspace has no index on its Space.
    NoIndex,
    /// Per display, the destination has no workspace at that index.
    NoDestination,
}

/// What moving the shown workspace to another Space does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceMove<K> {
    Refused(WorkspaceMoveRefusal),
    /// Per display (each display has its own workspaces): the members move to the workspace at
    /// the same index on the destination.
    Members {
        from: K,
        index: usize,
    },
    /// The workspace itself moves (its identity goes with it).
    Transfer(K),
}

/// What moving the workspace `shown` to another Space does.
pub fn workspace_move<K>(
    shown: Option<K>,
    per_display: bool,
    same_space: bool,
    index: Option<usize>,
) -> WorkspaceMove<K> {
    let Some(shown) = shown else {
        return WorkspaceMove::Refused(WorkspaceMoveRefusal::Nothing);
    };
    match (per_display, same_space, index) {
        (false, _, _) => WorkspaceMove::Transfer(shown),
        (true, true, _) => WorkspaceMove::Refused(WorkspaceMoveRefusal::SameSpace),
        (true, false, None) => WorkspaceMove::Refused(WorkspaceMoveRefusal::NoIndex),
        (true, false, Some(index)) => WorkspaceMove::Members { from: shown, index },
    }
}

/// The workspace the members go to, once the destination is initialised.
pub fn members_destination<K>(found: Option<K>) -> Result<K, WorkspaceMoveRefusal> {
    found.ok_or(WorkspaceMoveRefusal::NoDestination)
}

/// The window focused after moving members.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovedFocus<W> {
    /// The workspace's focus moved with it.
    Preferred(W),
    /// Its focus stayed: the first window moved.
    First(W),
    /// Nothing moved.
    Nothing,
}

/// The window focused after moving `moved`, the workspace's focus having been `preferred`.
pub fn moved_focus<W: Copy + PartialEq>(preferred: Option<W>, moved: &[W]) -> MovedFocus<W> {
    match (preferred.filter(|window| moved.contains(window)), moved.first()) {
        (Some(window), _) => MovedFocus::Preferred(window),
        (None, Some(&first)) => MovedFocus::First(first),
        (None, None) => MovedFocus::Nothing,
    }
}

/// Who manages a claimed window: its rule, or the rule's override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manage<O> {
    ByRule,
    Override(O),
}

/// Whether a window an app rule evaluated is claimed by another owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim<O> {
    /// Claimed: only its management override is recorded; it is not assigned.
    Claimed {
        manage: Manage<O>,
    },
    Free,
}

/// Whether a window is claimed, and its rule's management override (none: the rule manages).
pub fn claim<O>(claimed: bool, management_override: Option<O>) -> Claim<O> {
    match (claimed, management_override) {
        (false, _) => Claim::Free,
        (true, None) => Claim::Claimed { manage: Manage::ByRule },
        (true, Some(o)) => Claim::Claimed { manage: Manage::Override(o) },
    }
}

/// Whether an app rule's workspace stands for an assigned window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleWorkspace {
    Kept,
    /// A persistence match restores the user's workspace: the rule's default placement must
    /// not relocate the window.
    DroppedByRestore,
}

/// Whether the window's native layout memory is forgotten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeLayout {
    /// The rule is re-applied with a workspace of its own: the old native layout is stale.
    Forgotten,
    Kept,
}

/// What assigning a free window does besides the assignment itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assignment {
    pub rule_workspace: RuleWorkspace,
    pub native: NativeLayout,
}

/// What assigning a free window does: `restored` (a persistence match), `reapply` (the rule's
/// workspace is applied again), and whether the rule names a workspace.
pub fn assignment(restored: bool, reapply: bool, rule_names_workspace: bool) -> Assignment {
    let rule_workspace = if restored {
        RuleWorkspace::DroppedByRestore
    } else {
        RuleWorkspace::Kept
    };
    let native = if reapply && rule_names_workspace && !restored {
        NativeLayout::Forgotten
    } else {
        NativeLayout::Kept
    };
    Assignment { rule_workspace, native }
}

/// What an app rule does to a window's floating state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleFloat {
    Float,
    Unfloat,
    Keep,
}

/// What an app rule that says the window `should_float` does to one that `was_floating`.
pub fn rule_float(should_float: bool, was_floating: bool) -> RuleFloat {
    match (should_float, was_floating) {
        (true, _) => RuleFloat::Float,
        (false, true) => RuleFloat::Unfloat,
        (false, false) => RuleFloat::Keep,
    }
}

/// The frame request an app rule makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RuleFrame<P, R> {
    /// A floating placement.
    Place(P),
    /// A tiled resize.
    Resize(R),
    /// No frame request.
    #[default]
    Keep,
}

/// The frame request: a floating placement if the rule makes one, else a tiled resize.
pub fn rule_frame<P, R>(place: Option<P>, resize: Option<R>) -> RuleFrame<P, R> {
    match (place, resize) {
        (Some(place), _) => RuleFrame::Place(place),
        (None, Some(resize)) => RuleFrame::Resize(resize),
        (None, None) => RuleFrame::Keep,
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

    fn moving(
        policy: Policy<u8, &'static str>,
    ) -> RelocationFacts<u32, u32, u8, &'static str, char> {
        RelocationFacts {
            policy,
            source: 10,
            destination: 11,
            families: Some((Family::Tiled, Family::Tiled)),
            space: 1,
            destination_space: Some(1),
            owner: Some(10),
            projected_floating: false,
            in_floats: false,
            layout: Some('L'),
            preserve_widths: true,
            unavailable: false,
            placeable: true,
        }
    }

    fn projection(
        facts: RelocationFacts<u32, u32, u8, &'static str, char>,
    ) -> Projection<u8, &'static str, char> {
        match relocation(facts) {
            Relocation::Move { projection, .. } => projection,
            refused => panic!("{refused:?}"),
        }
    }

    #[test]
    fn a_relocation_is_refused_for_an_unknown_workspace_another_space_or_owner_or_no_layout() {
        let r = |facts| relocation(facts);
        let refused = |reason| Relocation::<u8, &str, char>::Refused(reason);
        let facts = moving(Policy::Workspace);
        assert_eq!(
            r(RelocationFacts { families: None, ..facts }),
            refused(RelocationRefusal::UnknownWorkspace)
        );
        for destination_space in [None, Some(2)] {
            assert_eq!(
                r(RelocationFacts { destination_space, ..facts }),
                refused(RelocationRefusal::OtherSpace)
            );
        }
        assert_eq!(
            r(RelocationFacts { owner: Some(12), ..facts }),
            refused(RelocationRefusal::OwnedElsewhere)
        );
        // Only observation may find it on the destination already.
        assert_eq!(
            r(RelocationFacts { owner: Some(11), ..facts }),
            refused(RelocationRefusal::OwnedElsewhere)
        );
        assert!(matches!(
            r(RelocationFacts {
                owner: Some(11),
                ..moving(Policy::Observed)
            }),
            Relocation::Move { assign: Assign::Already, .. }
        ));
        assert!(matches!(
            r(RelocationFacts { owner: None, ..facts }),
            Relocation::Move { assign: Assign::Assign, .. }
        ));
        assert_eq!(
            r(RelocationFacts {
                layout: None,
                ..moving(Policy::Directional(3))
            }),
            refused(RelocationRefusal::DirectionalWithoutLayout)
        );
    }

    #[test]
    fn a_relocated_window_floats_unfloats_tiles_or_goes_toward_a_direction() {
        let floating = |policy| RelocationFacts {
            projected_floating: true,
            in_floats: true,
            ..moving(policy)
        };
        assert_eq!(projection(floating(Policy::Workspace)), Projection::Float {
            observed: Observed::Moved
        });
        assert_eq!(projection(floating(Policy::Observed)), Projection::Float {
            observed: Observed::WasMember
        });
        // A command into or out of a floating layout tiles a floating window.
        for families in [
            (Family::Floating, Family::Tiled),
            (Family::Tiled, Family::Floating),
        ] {
            assert_eq!(
                projection(RelocationFacts {
                    families: Some(families),
                    ..floating(Policy::Workspace)
                }),
                Projection::Tile {
                    placed: Placed::Add {
                        layout: 'L',
                        width: Width::AsIs
                    }
                }
            );
        }
        // Observation and drops read the projection, into a floating layout: unfloat.
        for policy in [Policy::Observed, Policy::Drop(Edited::NoTree)] {
            assert_eq!(
                projection(RelocationFacts {
                    families: Some((Family::Tiled, Family::Floating)),
                    ..floating(policy)
                }),
                Projection::Unfloat {
                    placed: Placed::Add { layout: 'L', width: () }
                }
            );
        }
        // The floating set alone does not make an observed window float.
        assert_eq!(
            projection(RelocationFacts {
                projected_floating: false,
                ..floating(Policy::Observed)
            }),
            Projection::Tile {
                placed: Placed::Add {
                    layout: 'L',
                    width: Width::AsIs
                }
            }
        );
        assert_eq!(
            projection(moving(Policy::Drop(Edited::Tree("tree")))),
            Projection::Tile {
                placed: Placed::Edited {
                    tree: "tree",
                    width: Width::AsIs
                }
            }
        );
        assert_eq!(
            projection(RelocationFacts {
                layout: None,
                ..moving(Policy::Workspace)
            }),
            Projection::Tile { placed: Placed::NoLayout }
        );
        assert_eq!(
            projection(floating(Policy::Directional(3))),
            Projection::Toward {
                direction: 3,
                width: Width::AsIs,
                layout: 'L'
            }
        );
    }

    #[test]
    fn a_scrolling_destination_keeps_the_width_when_the_setting_asks_and_it_has_a_layout() {
        let scrolling = |policy| RelocationFacts {
            families: Some((Family::Tiled, Family::Scrolling)),
            ..moving(policy)
        };
        assert_eq!(projection(scrolling(Policy::Workspace)), Projection::Tile {
            placed: Placed::Add {
                layout: 'L',
                width: Width::Preserve
            }
        });
        assert_eq!(
            projection(RelocationFacts {
                preserve_widths: false,
                ..scrolling(Policy::Workspace)
            }),
            Projection::Tile {
                placed: Placed::Add {
                    layout: 'L',
                    width: Width::AsIs
                }
            }
        );
        assert_eq!(
            projection(RelocationFacts {
                layout: None,
                ..scrolling(Policy::Drop(Edited::Tree("tree")))
            }),
            Projection::Tile {
                placed: Placed::Edited {
                    tree: "tree",
                    width: Width::AsIs
                }
            }
        );
        let floating = RelocationFacts {
            in_floats: true,
            ..scrolling(Policy::Directional(3))
        };
        assert_eq!(projection(floating), Projection::Toward {
            direction: 3,
            width: Width::Preserve,
            layout: 'L'
        });
    }

    #[test]
    fn a_relocated_window_is_suspended_resumed_or_left() {
        let afterwards = |facts| match relocation(facts) {
            Relocation::Move { afterwards, .. } => afterwards,
            refused => panic!("{refused:?}"),
        };
        assert_eq!(
            afterwards(RelocationFacts {
                unavailable: true,
                ..moving(Policy::Observed)
            }),
            Afterwards::Suspended
        );
        assert_eq!(
            afterwards(RelocationFacts {
                placeable: false,
                ..moving(Policy::Workspace)
            }),
            Afterwards::Suspended
        );
        // Only a command's move suspends a window it cannot place.
        assert_eq!(
            afterwards(RelocationFacts {
                placeable: false,
                ..moving(Policy::Observed)
            }),
            Afterwards::Resumed
        );
        assert_eq!(
            afterwards(RelocationFacts {
                placeable: false,
                ..moving(Policy::Directional(3))
            }),
            Afterwards::AsIs
        );
        assert_eq!(afterwards(moving(Policy::Workspace)), Afterwards::AsIs);
    }

    fn adding(workspace: Option<(u32, Family)>) -> AdditionFacts<u32, char> {
        AdditionFacts {
            admitted: true,
            fullscreen_suspended: false,
            workspace,
            in_floats: false,
            tree: Some(Tree {
                layout: 'L',
                holds_window: false,
            }),
            preserve_widths: true,
        }
    }

    #[test]
    fn an_added_window_is_skipped_floats_tiles_or_joins_a_floating_layout() {
        let tiled = adding(Some((1, Family::Tiled)));
        assert_eq!(
            addition(AdditionFacts { admitted: false, ..tiled }),
            Addition::Skip(Skipped::Unadmitted)
        );
        assert_eq!(
            addition(AdditionFacts {
                fullscreen_suspended: true,
                ..tiled
            }),
            Addition::Skip(Skipped::FullscreenSuspended)
        );
        assert_eq!(addition(adding(None)), Addition::Skip(Skipped::Unassigned));
        assert_eq!(
            addition(AdditionFacts { in_floats: true, ..tiled }),
            Addition::Float { workspace: 1 }
        );
        assert_eq!(addition(tiled), Addition::Tile {
            workspace: 1,
            insert: Insert::Add {
                layout: 'L',
                width: Width::AsIs
            }
        });
        let scrolling = adding(Some((1, Family::Scrolling)));
        assert_eq!(addition(scrolling), Addition::Tile {
            workspace: 1,
            insert: Insert::Add {
                layout: 'L',
                width: Width::Preserve
            }
        });
        assert_eq!(
            addition(AdditionFacts {
                preserve_widths: false,
                ..scrolling
            }),
            Addition::Tile {
                workspace: 1,
                insert: Insert::Add {
                    layout: 'L',
                    width: Width::AsIs
                }
            }
        );
        assert_eq!(
            addition(AdditionFacts {
                tree: Some(Tree {
                    layout: 'L',
                    holds_window: true
                }),
                ..tiled
            }),
            Addition::Tile {
                workspace: 1,
                insert: Insert::Present
            }
        );
        assert_eq!(addition(AdditionFacts { tree: None, ..tiled }), Addition::Tile {
            workspace: 1,
            insert: Insert::NoLayout
        });
        // A floating layout takes it in, floating or not.
        for in_floats in [true, false] {
            assert_eq!(
                addition(AdditionFacts {
                    in_floats,
                    ..adding(Some((1, Family::Floating)))
                }),
                Addition::IntoFloatingLayout {
                    workspace: 1,
                    insert: Insert::Add { layout: 'L', width: () }
                }
            );
        }
    }

    #[test]
    fn a_window_moves_to_a_space_showing_a_workspace_and_lands_focused_if_placeable() {
        assert_eq!(
            window_move(None, Some(2)),
            WindowMove::Refused(WindowMoveRefusal::NotAMember)
        );
        assert_eq!(
            window_move(Some(1), None::<u32>),
            WindowMove::Refused(WindowMoveRefusal::NoWorkspace)
        );
        assert_eq!(window_move(Some(1), Some(2)), WindowMove::Move {
            from: 1,
            to: 2
        });
        assert_eq!(landing(true), Landing::Focused);
        assert_eq!(landing(false), Landing::Hidden);
    }

    #[test]
    fn a_shown_workspace_moves_its_members_per_display_and_transfers_otherwise() {
        use WorkspaceMoveRefusal as R;
        assert_eq!(
            workspace_move(None::<u32>, true, false, Some(0)),
            WorkspaceMove::Refused(R::Nothing)
        );
        assert_eq!(
            workspace_move(None::<u32>, false, false, Some(0)),
            WorkspaceMove::Refused(R::Nothing)
        );
        assert_eq!(
            workspace_move(Some(1), true, true, Some(0)),
            WorkspaceMove::Refused(R::SameSpace)
        );
        assert_eq!(
            workspace_move(Some(1), true, false, None),
            WorkspaceMove::Refused(R::NoIndex)
        );
        assert_eq!(
            workspace_move(Some(1), true, false, Some(2)),
            WorkspaceMove::Members { from: 1, index: 2 }
        );
        assert_eq!(
            workspace_move(Some(1), false, false, Some(0)),
            WorkspaceMove::Transfer(1)
        );
        // A transfer onto its own Space or without an index is still a transfer.
        assert_eq!(
            workspace_move(Some(1), false, true, None),
            WorkspaceMove::Transfer(1)
        );
        assert_eq!(members_destination(Some(3)), Ok(3));
        assert_eq!(members_destination(None::<u32>), Err(R::NoDestination));
        assert_eq!(moved_focus(Some(2), &[1, 2]), MovedFocus::Preferred(2));
        assert_eq!(moved_focus(Some(9), &[1, 2]), MovedFocus::First(1));
        assert_eq!(moved_focus(None, &[1, 2]), MovedFocus::First(1));
        assert_eq!(moved_focus(Some(9), &[]), MovedFocus::Nothing);
    }

    #[test]
    fn a_claimed_window_records_only_its_management_and_a_restore_drops_the_rules_workspace() {
        assert_eq!(claim(false, Some('o')), Claim::Free);
        assert_eq!(claim(true, None::<char>), Claim::Claimed {
            manage: Manage::ByRule
        });
        assert_eq!(claim(true, Some('o')), Claim::Claimed {
            manage: Manage::Override('o')
        });
        let kept = |native| Assignment {
            rule_workspace: RuleWorkspace::Kept,
            native,
        };
        assert_eq!(assignment(false, true, true), kept(NativeLayout::Forgotten));
        assert_eq!(assignment(false, false, true), kept(NativeLayout::Kept));
        assert_eq!(assignment(false, true, false), kept(NativeLayout::Kept));
        assert_eq!(assignment(true, true, true), Assignment {
            rule_workspace: RuleWorkspace::DroppedByRestore,
            native: NativeLayout::Kept
        });
    }

    #[test]
    fn an_app_rule_floats_unfloats_or_keeps_and_requests_a_placement_before_a_resize() {
        assert_eq!(rule_float(true, false), RuleFloat::Float);
        assert_eq!(rule_float(true, true), RuleFloat::Float);
        assert_eq!(rule_float(false, true), RuleFloat::Unfloat);
        assert_eq!(rule_float(false, false), RuleFloat::Keep);
        assert_eq!(rule_frame(Some('p'), Some('r')), RuleFrame::Place('p'));
        assert_eq!(rule_frame(None::<char>, Some('r')), RuleFrame::Resize('r'));
        assert_eq!(rule_frame(None::<char>, None::<char>), RuleFrame::Keep);
    }

    /// Every drop kind reads only a choice of its own kind, and a plan accepting a drop the
    /// facts give no place for reads as nothing (a payload the engine does not hold).
    #[test]
    fn a_drop_choice_of_another_kind_or_without_a_place_reads_as_nothing() {
        type C = DropChoice<u32, u32, u8>;
        let plan = C::Plan(DropPlan::Relocate { from: 11, place: () });
        let all = [
            plan,
            C::Kept(Kept::Moved),
            C::Action(A::Swap),
            C::Override(Override::Swap),
            C::Edit(DropEdit::Edit),
        ];
        fn reads<X: DropAsk<u32, u32, u8>>(ask: &X, choices: &[C]) -> Vec<bool> {
            choices.iter().map(|&c| ask.of(c).is_some()).collect()
        }
        let only = |i: usize| (0..5).map(|j| j == i).collect::<Vec<_>>();
        let facts = tiled(None);
        assert_eq!(reads(&facts, &all), only(0));
        assert_eq!(
            facts.of(plan),
            Some(DropPlan::Relocate { from: 11, place: "here" })
        );
        let unlaid = DropFacts { laid: Laid::NoLayout, ..facts };
        assert_eq!(
            unlaid.of(plan),
            None,
            "an accepted plan the facts give no place for"
        );
        let refused = C::Plan(DropPlan::Refused(DropRefusal::NoLayout));
        assert_eq!(
            unlaid.of(refused),
            Some(DropPlan::Refused(DropRefusal::NoLayout))
        );
        assert_eq!(reads(&KeptAsk { before: None, after: None }, &all), only(1));
        let action = ActionAsk {
            same_slot: false,
            action: A::Swap,
            center: A::Stack,
        };
        assert_eq!(reads(&action, &all), only(2));
        assert_eq!(reads(&OverrideAsk(None), &all), only(3));
        assert_eq!(reads(&Arrangement::Tiled, &all), only(4));
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
