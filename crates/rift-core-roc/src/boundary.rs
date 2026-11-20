//! The one place the Rust core's values become the platform's boundary types (`Boundary.roc`,
//! glued into [`abi`]) and back. Every unsafe operation on Roc memory is here: building a tag
//! union's payload, allocating a Roc box, reading a payload after matching its tag, and
//! releasing what Roc answered. Values going in are owned by Roc once handed over (the Roc
//! function consumes its argument); values coming back are read, then released here.

use std::mem::ManuallyDrop;

use rift_core::focus::{self, Follow, Removal, RemovalFocus, Stay, UntrackedFocus};
use rift_core::layout::bsp::{self, Bsp};
use rift_core::layout::floating::{Floating, Newcomer, Seen};
use rift_core::layout::scrolling::{
    self, Alignment, Column, Navigation, Placed, Row, Settle, Settled, Snap, Spot, Strip, Width,
};
use rift_core::layout::traditional::{self, Traditional};
use rift_core::layout::{Gaps, Size, Sizing, WindowConstraints};
use rift_core::plain::{Answers, Fact, Rect, Unasked, WindowId};

use crate::abi;

/// A Roc box's header, which the payload's type decides: a type holding refcounted values
/// (lists, boxes) gets a two-word header, any other one word. Each boxed boundary type's
/// helper names its own.
#[derive(Clone, Copy)]
enum Header {
    /// The payload holds refcounted values.
    Refcounted,
}

/// A Roc box holding `value`, allocated as Roc allocates one with `header`.
fn boxed<T>(value: T, header: Header, host: &abi::RocHost) -> *mut T {
    let refcounted = match header {
        Header::Refcounted => true,
    };
    // SAFETY: allocate_box returns room for a `T` behind a box header with its refcount set
    // to one; writing the value initializes it, and Roc takes the box over.
    unsafe {
        let ptr = abi::allocate_box(size_of::<T>(), align_of::<T>(), refcounted, host) as *mut T;
        ptr.write(value);
        ptr
    }
}

/// A boxed BSP node (`BspNode` holds boxes).
fn box_bsp_node(node: abi::BoundaryBspNode, host: &abi::RocHost) -> *mut abi::BoundaryBspNode {
    boxed(node, Header::Refcounted, host)
}

/// A split's boxed children (`BspChildren` holds boxes).
fn box_bsp_children(
    children: abi::BoundaryBspChildren,
    host: &abi::RocHost,
) -> *mut abi::BoundaryBspChildren {
    boxed(children, Header::Refcounted, host)
}

/// A boxed traditional child (`Child` holds a box).
fn box_child(child: abi::BoundaryChild, host: &abi::RocHost) -> *mut abi::BoundaryChild {
    boxed(child, Header::Refcounted, host)
}

/// A boxed traditional node (`TraditionalNode` holds lists and boxes).
fn box_traditional_node(
    node: abi::BoundaryTraditionalNode,
    host: &abi::RocHost,
) -> *mut abi::BoundaryTraditionalNode {
    boxed(node, Header::Refcounted, host)
}

/// A Roc list of `items` (Roc takes it over with the value it is in).
fn list<T: Copy, const REFCOUNTED: bool>(
    items: &[T],
    host: &abi::RocHost,
) -> abi::RocListWith<T, REFCOUNTED> {
    // SAFETY: the list is allocated through the host and its elements copied in; it is handed
    // to Roc inside the argument, which Roc consumes.
    unsafe { abi::RocListWith::from_slice(items, host) }
}

/// A tag union value: the union's type, its payload union's type and field, and its tag.
macro_rules! union {
    ($ty:ident, $payload:ident . $field:ident ($value:expr), $tag:expr) => {
        abi::$ty {
            payload: abi::$payload {
                $field: ManuallyDrop::new($value),
            },
            tag: $tag,
        }
    };
    ($ty:ident, $payload:ident . $field:ident, $tag:expr) => {
        abi::$ty {
            payload: abi::$payload { $field: [] },
            tag: $tag,
        }
    };
}

// ---------------------------------------------------------------------------------------------
// Shared geometry.

fn window(window: WindowId) -> abi::BoundaryWindowId {
    abi::BoundaryWindowId {
        pid: window.pid,
        idx: window.idx,
    }
}

fn rect(rect: Rect) -> abi::BoundaryRect {
    abi::BoundaryRect {
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: rect.h,
    }
}

fn size(size: Size) -> abi::BoundarySize {
    abi::BoundarySize {
        width: size.width(),
        height: size.height(),
    }
}

fn window_constraints(constraints: WindowConstraints) -> abi::BoundaryWindowConstraints {
    use abi::BoundarySizingTag as Tag;
    abi::BoundaryWindowConstraints {
        sizing: match constraints.sizing {
            Sizing::Fixed { min } => {
                union!(
                    BoundarySizing,
                    BoundarySizingPayload.fixed(size(min)),
                    Tag::Fixed
                )
            }
            Sizing::Resizable { advisory_min } => union!(
                BoundarySizing,
                BoundarySizingPayload.resizable(size(advisory_min)),
                Tag::Resizable
            ),
            Sizing::ResizableAtLeastMin { min } => union!(
                BoundarySizing,
                BoundarySizingPayload.resizable_at_least_min(size(min)),
                Tag::ResizableAtLeastMin
            ),
        },
        locked_width: constraints.locked_width,
        locked_height: constraints.locked_height,
        max_width: constraints.max_width,
        max_height: constraints.max_height,
    }
}

fn constraints(constraints: Option<WindowConstraints>) -> abi::BoundaryConstraints {
    use abi::BoundaryConstraintsTag as Tag;
    match constraints {
        Some(known) => union!(
            BoundaryConstraints,
            BoundaryConstraintsPayload.constrained(window_constraints(known)),
            Tag::Constrained
        ),
        None => union!(
            BoundaryConstraints,
            BoundaryConstraintsPayload.unconstrained,
            Tag::Unconstrained
        ),
    }
}

fn gaps(gaps: Gaps) -> abi::BoundaryGaps {
    abi::BoundaryGaps {
        top: gaps.top,
        left: gaps.left,
        bottom: gaps.bottom,
        right: gaps.right,
        horizontal: gaps.horizontal,
        vertical: gaps.vertical,
    }
}

fn from_window(window: abi::BoundaryWindowId) -> WindowId {
    WindowId {
        pid: window.pid,
        idx: window.idx,
    }
}

fn from_rect(rect: abi::BoundaryRect) -> Rect {
    Rect {
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: rect.h,
    }
}

/// The frames Roc answered, read out and released.
pub(crate) fn frames(
    answer: abi::RocListWith<abi::BoundaryFrame, false>,
    host: &abi::RocHost,
) -> Vec<(WindowId, Rect)> {
    let frames = answer
        .as_slice()
        .iter()
        .map(|frame| (from_window(frame.window), from_rect(frame.frame)))
        .collect();
    // SAFETY: the list is Roc's answer, owned here and read in full above.
    unsafe { answer.decref(host) };
    frames
}

/// The rects Roc answered, read out and released.
pub(crate) fn rects(
    answer: abi::RocListWith<abi::BoundaryRect, false>,
    host: &abi::RocHost,
) -> Vec<Rect> {
    let rects = answer.as_slice().iter().copied().map(from_rect).collect();
    // SAFETY: the list is Roc's answer, owned here and read in full above.
    unsafe { answer.decref(host) };
    rects
}

// ---------------------------------------------------------------------------------------------
// BSP.

fn bsp_fill(fill: bsp::Fill) -> abi::BoundaryFill {
    match fill {
        bsp::Fill::Tiled => abi::BoundaryFill::Tiled,
        bsp::Fill::Screen => abi::BoundaryFill::Screen,
        bsp::Fill::WithinGaps => abi::BoundaryFill::WithinGaps,
    }
}

fn bsp_node(node: bsp::Node<WindowId>, host: &abi::RocHost) -> abi::BoundaryBspNode {
    use abi::{BoundaryBspChildrenTag as Children, BoundaryBspNodeTag as Node, BoundarySlotTag};
    match node {
        bsp::Node::Split { horizontal, ratio, children } => {
            let children = match children {
                bsp::Children::One(only) => union!(
                    BoundaryBspChildren,
                    BoundaryBspChildrenPayload.one(box_bsp_node(bsp_node(*only, host), host)),
                    Children::One
                ),
                bsp::Children::Two(first, second) => union!(
                    BoundaryBspChildren,
                    BoundaryBspChildrenPayload.two(abi::BoundaryBspChildrenTwoPayload {
                        _0: box_bsp_node(bsp_node(*first, host), host),
                        _1: box_bsp_node(bsp_node(*second, host), host),
                    }),
                    Children::Two
                ),
            };
            union!(
                BoundaryBspNode,
                BoundaryBspNodePayload.split(abi::BoundaryBspSplit {
                    horizontal,
                    ratio,
                    children: box_bsp_children(children, host),
                }),
                Node::Split
            )
        }
        bsp::Node::Leaf { slot, fill, stacked } => {
            let slot = match slot {
                bsp::Slot::Empty => {
                    union!(BoundarySlot, BoundarySlotPayload.empty, BoundarySlotTag::Empty)
                }
                bsp::Slot::Window { window: id, constraints: known } => union!(
                    BoundarySlot,
                    BoundarySlotPayload.window(abi::BoundaryBspWindow {
                        window: window(id),
                        constraints: constraints(known),
                    }),
                    BoundarySlotTag::Window
                ),
            };
            union!(
                BoundaryBspNode,
                BoundaryBspNodePayload.leaf(abi::BoundaryBspLeaf {
                    slot,
                    fill: bsp_fill(fill),
                    stacked
                }),
                Node::Leaf
            )
        }
    }
}

pub(crate) fn bsp_shape(shape: Bsp<WindowId>, host: &abi::RocHost) -> abi::BoundaryBspShape {
    use abi::BoundaryAspectTag as Tag;
    abi::BoundaryBspShape {
        screen: rect(shape.screen),
        gaps: gaps(shape.gaps),
        aspect: match shape.aspect {
            Some(ratio) => union!(BoundaryAspect, BoundaryAspectPayload.aspect(ratio), Tag::Aspect),
            None => union!(BoundaryAspect, BoundaryAspectPayload.free, Tag::Free),
        },
        root: bsp_node(shape.root, host),
    }
}

// ---------------------------------------------------------------------------------------------
// Traditional.

fn traditional_fill(fill: traditional::Fill) -> abi::BoundaryFill {
    match fill {
        traditional::Fill::Tiled => abi::BoundaryFill::Tiled,
        traditional::Fill::Screen => abi::BoundaryFill::Screen,
        traditional::Fill::WithinGaps => abi::BoundaryFill::WithinGaps,
    }
}

fn child(child: traditional::Child<WindowId>, host: &abi::RocHost) -> abi::BoundaryChild {
    abi::BoundaryChild {
        size: child.size,
        node: box_traditional_node(traditional_node(child.node, host), host),
    }
}

fn children(
    children: Vec<traditional::Child<WindowId>>,
    host: &abi::RocHost,
) -> abi::RocList<abi::BoundaryChild> {
    let children: Vec<abi::BoundaryChild> = children.into_iter().map(|c| child(c, host)).collect();
    list(&children, host)
}

fn traditional_node(
    node: traditional::Node<WindowId>,
    host: &abi::RocHost,
) -> abi::BoundaryTraditionalNode {
    use abi::BoundaryTraditionalNodeTag as Tag;
    match node {
        traditional::Node::Window {
            window: id,
            constraints: known,
            fill,
        } => union!(
            BoundaryTraditionalNode,
            BoundaryTraditionalNodePayload.window(abi::BoundaryTraditionalWindow {
                window: window(id),
                constraints: constraints(known),
                fill: traditional_fill(fill),
            }),
            Tag::Window
        ),
        traditional::Node::Axis { horizontal, fill, first, rest } => union!(
            BoundaryTraditionalNode,
            BoundaryTraditionalNodePayload.axis(abi::BoundaryTraditionalAxis {
                horizontal,
                fill: traditional_fill(fill),
                first: box_child(child(*first, host), host),
                rest: children(rest, host),
            }),
            Tag::Axis
        ),
        traditional::Node::Stack {
            horizontal,
            fill,
            before,
            focused,
            after,
        } => union!(
            BoundaryTraditionalNode,
            BoundaryTraditionalNodePayload.stack(abi::BoundaryTraditionalStack {
                horizontal,
                fill: traditional_fill(fill),
                before: children(before, host),
                focused: box_child(child(*focused, host), host),
                after: children(after, host),
            }),
            Tag::Stack
        ),
    }
}

pub(crate) fn traditional_shape(
    shape: Traditional<WindowId>,
    host: &abi::RocHost,
) -> abi::BoundaryTraditionalShape {
    let line = shape.stack_line;
    abi::BoundaryTraditionalShape {
        screen: rect(shape.screen),
        area: rect(shape.area),
        gaps: gaps(shape.gaps),
        stack_offset: shape.stack_offset,
        stack_line: abi::BoundaryStackLine {
            thickness: line.thickness,
            spacing: line.spacing,
            top: line.top,
            left: line.left,
        },
        root: traditional_node(shape.root, host),
    }
}

// ---------------------------------------------------------------------------------------------
// Scrolling.

fn row(row: Row<WindowId>) -> abi::BoundaryRow {
    abi::BoundaryRow {
        window: window(row.window),
        constraints: window_constraints(row.constraints),
        weight: row.weight,
    }
}

fn column(column: Column<WindowId, u64>, host: &abi::RocHost) -> abi::BoundaryColumn {
    use abi::BoundaryWidthTag as Tag;
    let rest: Vec<abi::BoundaryRow> = column.rest.into_iter().map(row).collect();
    abi::BoundaryColumn {
        id: column.id,
        width: match column.width {
            Width::Default => union!(BoundaryWidth, BoundaryWidthPayload.default, Tag::Default),
            Width::Proportion(ratio) => {
                union!(
                    BoundaryWidth,
                    BoundaryWidthPayload.proportion(ratio),
                    Tag::Proportion
                )
            }
            Width::Fixed(width) => {
                union!(BoundaryWidth, BoundaryWidthPayload.exactly(width), Tag::Exactly)
            }
        },
        first: row(column.first),
        rest: list(&rest, host),
    }
}

pub(crate) fn strip(strip: Strip<WindowId, u64>, host: &abi::RocHost) -> abi::BoundaryStrip {
    let widths = strip.widths;
    let rest: Vec<abi::BoundaryColumn> = strip.rest.into_iter().map(|c| column(c, host)).collect();
    abi::BoundaryStrip {
        tiling: rect(strip.tiling),
        column_gap: strip.column_gap,
        row_gap: strip.row_gap,
        widths: abi::BoundaryWidths {
            ratio: widths.ratio,
            min_ratio: widths.min_ratio,
            max_ratio: widths.max_ratio,
            preserve_window_sizes: widths.preserve_window_sizes,
            expand_single_column: widths.expand_single_column,
        },
        alignment: match strip.alignment {
            Alignment::Left => abi::BoundaryAlignment::Left,
            Alignment::Center => abi::BoundaryAlignment::Center,
            Alignment::Right => abi::BoundaryAlignment::Right,
        },
        navigation: match strip.navigation {
            Navigation::Anchored => abi::BoundaryNavigation::Anchored,
            Navigation::Free => abi::BoundaryNavigation::Free,
        },
        first: column(strip.first, host),
        rest: list(&rest, host),
    }
}

fn spot(spot: Spot<u64>) -> abi::BoundarySpot {
    abi::BoundarySpot {
        id: spot.id,
        world_x: spot.world_x,
        width: spot.width,
        anchor: spot.anchor,
    }
}

fn snap(snap: Snap<u64>) -> abi::BoundarySnap {
    abi::BoundarySnap {
        offset: snap.offset,
        column: snap.column,
    }
}

fn placed(placed: Placed<WindowId, u64>, host: &abi::RocHost) -> abi::BoundaryPlaced {
    let rest: Vec<abi::BoundarySpot> = placed.rest.into_iter().map(spot).collect();
    let frames: Vec<abi::BoundaryFrame> = placed
        .frames
        .into_iter()
        .map(|(id, frame)| abi::BoundaryFrame {
            window: window(id),
            frame: rect(frame),
        })
        .collect();
    let (first_snap, rest_snaps) = placed.snaps;
    let rest_snaps: Vec<abi::BoundarySnap> = rest_snaps.into_iter().map(snap).collect();
    let (low, high) = placed.bounds;
    abi::BoundaryPlaced {
        first: spot(placed.first),
        rest: list(&rest, host),
        frames: list(&frames, host),
        first_snap: snap(first_snap),
        rest_snaps: list(&rest_snaps, host),
        low,
        high,
    }
}

fn from_spot(spot: abi::BoundarySpot) -> Spot<u64> {
    Spot {
        id: spot.id,
        world_x: spot.world_x,
        width: spot.width,
        anchor: spot.anchor,
    }
}

fn from_snap(snap: abi::BoundarySnap) -> Snap<u64> {
    Snap {
        offset: snap.offset,
        column: snap.column,
    }
}

/// The placement Roc answered, read out and released.
pub(crate) fn from_placed(
    answer: abi::BoundaryPlaced,
    host: &abi::RocHost,
) -> Placed<WindowId, u64> {
    let placed = Placed {
        first: from_spot(answer.first),
        rest: answer.rest.as_slice().iter().copied().map(from_spot).collect(),
        frames: answer
            .frames
            .as_slice()
            .iter()
            .map(|frame| (from_window(frame.window), from_rect(frame.frame)))
            .collect(),
        snaps: (
            from_snap(answer.first_snap),
            answer.rest_snaps.as_slice().iter().copied().map(from_snap).collect(),
        ),
        bounds: (answer.low, answer.high),
    };
    // SAFETY: the placement is Roc's answer, owned here and read in full above.
    unsafe { answer.decref(host) };
    placed
}

pub(crate) fn settle_question(
    placement: Placed<WindowId, u64>,
    view: f64,
    projected: f64,
    from: f64,
    how: Settle<u64>,
    host: &abi::RocHost,
) -> abi::BoundarySettleQuestion {
    use abi::BoundarySettleTag as Tag;
    abi::BoundarySettleQuestion {
        placed: placed(placement, host),
        view,
        projected,
        from,
        how: match how {
            Settle::Niri => union!(BoundarySettle, BoundarySettlePayload.niri, Tag::Niri),
            Settle::Anchored => {
                union!(BoundarySettle, BoundarySettlePayload.anchored, Tag::Anchored)
            }
            Settle::AnchoredTowards(column) => union!(
                BoundarySettle,
                BoundarySettlePayload.anchored_towards(column),
                Tag::AnchoredTowards
            ),
        },
    }
}

pub(crate) fn from_settled(answer: abi::BoundarySettled) -> Settled<u64> {
    scrolling::Settled {
        column: answer.column,
        offset: answer.offset,
    }
}

// ---------------------------------------------------------------------------------------------
// Floating.

pub(crate) fn floating_question(
    floating: Floating,
    windows: Vec<Newcomer>,
    host: &abi::RocHost,
) -> abi::BoundaryFloatingQuestion {
    use abi::BoundarySeenTag as Tag;
    let windows: Vec<abi::BoundaryNewcomer> = windows
        .into_iter()
        .map(|newcomer| abi::BoundaryNewcomer {
            constraints: window_constraints(newcomer.constraints),
            seen: match newcomer.seen {
                Seen::Live(frame) => {
                    union!(BoundarySeen, BoundarySeenPayload.live(rect(frame)), Tag::Live)
                }
                Seen::Unseen => union!(BoundarySeen, BoundarySeenPayload.unseen, Tag::Unseen),
            },
        })
        .collect();
    abi::BoundaryFloatingQuestion {
        screen: rect(floating.screen),
        spread: floating.spread,
        windows: list(&windows, host),
    }
}

// ---------------------------------------------------------------------------------------------
// Model-free questions: facts, answers about keys, and decisions.

fn maybe_word(word: Option<u64>) -> abi::BoundaryMaybeWord {
    use abi::BoundaryMaybeWordTag as Tag;
    match word {
        Some(word) => union!(BoundaryMaybeWord, BoundaryMaybeWordPayload.some(word), Tag::Some),
        None => union!(BoundaryMaybeWord, BoundaryMaybeWordPayload.nothing, Tag::Nothing),
    }
}

fn maybe_window(id: Option<WindowId>) -> abi::BoundaryMaybeWindow {
    use abi::BoundaryMaybeWindowTag as Tag;
    match id {
        Some(id) => union!(
            BoundaryMaybeWindow,
            BoundaryMaybeWindowPayload.some_window(window(id)),
            Tag::SomeWindow
        ),
        None => union!(
            BoundaryMaybeWindow,
            BoundaryMaybeWindowPayload.no_window,
            Tag::NoWindow
        ),
    }
}

/// A fact of a given payload type, built by its own function from the payload's conversion.
macro_rules! fact {
    ($name:ident, $from:ty, $ty:ident, $payload:ident, $tag:ident, |$value:ident, $host:ident| $convert:expr) => {
        fn $name(fact: Fact<$from>, $host: &abi::RocHost) -> abi::$ty {
            let _ = $host;
            match fact {
                Fact::Asked($value) => union!($ty, $payload.asked($convert), abi::$tag::Asked),
                Fact::Unasked => union!($ty, $payload.unasked, abi::$tag::Unasked),
            }
        }
    };
}

fact!(
    fact_bool,
    bool,
    BoundaryFactBool,
    BoundaryFactBoolPayload,
    BoundaryFactBoolTag,
    |v, host| v
);
fact!(
    fact_word,
    Option<u64>,
    BoundaryFactWord,
    BoundaryFactWordPayload,
    BoundaryFactWordTag,
    |v, host| maybe_word(v)
);
fact!(
    fact_window,
    Option<WindowId>,
    BoundaryFactWindow,
    BoundaryFactWindowPayload,
    BoundaryFactWindowTag,
    |v, host| maybe_window(v)
);
fact!(
    fact_words,
    Vec<u64>,
    BoundaryFactWords,
    BoundaryFactWordsPayload,
    BoundaryFactWordsTag,
    |v, host| list(&v, host)
);
fact!(
    fact_listing,
    Option<bool>,
    BoundaryFactListing,
    BoundaryFactListingPayload,
    BoundaryFactListingTag,
    |v, host| listing(v)
);
fact!(
    fact_known,
    Option<(i32, u64)>,
    BoundaryFactKnown,
    BoundaryFactKnownPayload,
    BoundaryFactKnownTag,
    |v, host| known(v)
);

fn word_bools(
    answers: Answers<u64, bool>,
    host: &abi::RocHost,
) -> abi::RocListWith<abi::BoundaryWordBool, false> {
    let answers: Vec<_> = answers
        .0
        .into_iter()
        .map(|(key, answer)| abi::BoundaryWordBool { key, answer })
        .collect();
    list(&answers, host)
}

fn window_bools(
    answers: Answers<WindowId, bool>,
    host: &abi::RocHost,
) -> abi::RocListWith<abi::BoundaryWindowBool, false> {
    let answers: Vec<_> = answers
        .0
        .into_iter()
        .map(|(key, answer)| abi::BoundaryWindowBool { key: window(key), answer })
        .collect();
    list(&answers, host)
}

fn window_windows(
    answers: Answers<WindowId, WindowId>,
    host: &abi::RocHost,
) -> abi::RocListWith<abi::BoundaryWindowWindow, false> {
    let answers: Vec<_> = answers
        .0
        .into_iter()
        .map(|(key, answer)| abi::BoundaryWindowWindow {
            key: window(key),
            answer: window(answer),
        })
        .collect();
    list(&answers, host)
}

fn from_maybe_word(word: abi::BoundaryMaybeWord) -> Option<u64> {
    use abi::BoundaryMaybeWordTag as Tag;
    match word.tag {
        // SAFETY: the tag says the payload is a word.
        Tag::Some => Some(unsafe { *word.borrow_payload_some_unchecked() }),
        Tag::Nothing => None,
    }
}

/// A decision, or the fact it read that its question does not carry: each answer union's tag
/// matched, its payload read.
macro_rules! decision {
    ($answer:expr, $tag:ident, |$decided:ident| $convert:expr) => {{
        let answer = $answer;
        match answer.tag {
            abi::$tag::Decided => {
                // SAFETY: the tag says the payload is the decision.
                let $decided = unsafe { answer.borrow_payload_decided_unchecked() };
                Ok($convert)
            }
            // SAFETY: the tag says the payload is the fact's place.
            abi::$tag::Unasked => Err(Unasked {
                fact: unsafe { *answer.borrow_payload_unasked_unchecked() },
            }),
        }
    }};
}

// Focus.

pub(crate) fn removal_question(
    question: focus::plain::RemovalQuestion,
    host: &abi::RocHost,
) -> abi::BoundaryRemovalQuestion {
    use abi::BoundaryRemovalTag as Tag;
    abi::BoundaryRemovalQuestion {
        removal: match question.removal {
            Removal::AppClosed(pid) => union!(
                BoundaryRemoval,
                BoundaryRemovalPayload.app_closed(pid),
                Tag::AppClosed
            ),
            Removal::WindowRemoved(id) => union!(
                BoundaryRemoval,
                BoundaryRemovalPayload.window_removed(window(id)),
                Tag::WindowRemoved
            ),
        },
        focused: maybe_window(question.focused),
        removed_space: fact_word(question.removed_space, host),
        native_focus: fact_window(question.native_focus, host),
        successors: window_windows(question.successors, host),
        inactive: window_bools(question.inactive, host),
    }
}

pub(crate) fn removal_answer(
    answer: abi::BoundaryRemovalAnswer,
) -> Result<RemovalFocus<WindowId, u64>, Unasked> {
    use abi::BoundaryRemovalFocusTag as Tag;
    decision!(answer, BoundaryRemovalAnswerTag, |focus| match focus.tag {
        Tag::Unaffected => RemovalFocus::Unaffected,
        // SAFETY: the tag says the payload is the successor.
        Tag::KeepNative => RemovalFocus::KeepNative(from_window(unsafe {
            *focus.borrow_payload_keep_native_unchecked()
        })),
        // SAFETY: the tag says the payload is the Space.
        Tag::Refocus => RemovalFocus::Refocus(unsafe { *focus.borrow_payload_refocus_unchecked() }),
    })
}

pub(crate) fn candidate_question(
    question: focus::plain::Candidate,
    host: &abi::RocHost,
) -> abi::BoundaryCandidateQuestion {
    abi::BoundaryCandidateQuestion {
        preferred: maybe_word(question.preferred),
        last_focused: fact_word(question.last_focused, host),
        in_workspace: fact_words(question.in_workspace, host),
        visible: word_bools(question.visible, host),
    }
}

pub(crate) fn word_answer(answer: abi::BoundaryWordAnswer) -> Result<Option<u64>, Unasked> {
    decision!(answer, BoundaryWordAnswerTag, |word| from_maybe_word(*word))
}

pub(crate) fn verdict_answer(answer: abi::BoundaryVerdictAnswer) -> Result<bool, Unasked> {
    decision!(answer, BoundaryVerdictAnswerTag, |verdict| *verdict)
}

pub(crate) fn hides_question(
    question: focus::plain::Hides,
    host: &abi::RocHost,
) -> abi::BoundaryHidesQuestion {
    abi::BoundaryHidesQuestion {
        window: question.window,
        main: maybe_word(question.main),
        focused: maybe_word(question.focused),
        in_inactive_workspace: fact_bool(question.in_inactive_workspace, host),
    }
}

fn listing(listed: Option<bool>) -> abi::BoundaryListing {
    use abi::BoundaryListingTag as Tag;
    match listed {
        Some(listed) => union!(
            BoundaryListing,
            BoundaryListingPayload.listed(listed),
            Tag::Listed
        ),
        None => union!(BoundaryListing, BoundaryListingPayload.no_bundle, Tag::NoBundle),
    }
}

pub(crate) fn activation_question(
    question: focus::plain::Activation,
    host: &abi::RocHost,
) -> abi::BoundaryActivationQuestion {
    abi::BoundaryActivationQuestion {
        standard: fact_bool(question.standard, host),
        unavailable: fact_bool(question.unavailable, host),
        space: fact_word(question.space, host),
        workspace: fact_word(question.workspace, host),
        active_workspace: fact_word(question.active_workspace, host),
        pending_window: fact_bool(question.pending_window, host),
        mode_refusal: fact_word(question.mode_refusal, host),
        active_space_fullscreen: fact_bool(question.active_space_fullscreen, host),
        unmanageable_source: fact_word(question.unmanageable_source, host),
        blacklisted: fact_listing(question.blacklisted, host),
    }
}

pub(crate) fn follow_answer(
    answer: abi::BoundaryFollowAnswer,
) -> Result<Follow<u64, u64, u64, u64>, Unasked> {
    use abi::{BoundaryFollowTag as Tag, BoundaryStayTag as Why};
    decision!(answer, BoundaryFollowAnswerTag, |follow| match follow.tag {
        Tag::To => {
            // SAFETY: the tag says the payload is the Space and workspace.
            let to = unsafe { follow.borrow_payload_to_unchecked() };
            Follow::To(to.space, to.workspace)
        }
        Tag::Stay => {
            // SAFETY: the tag says the payload is the reason.
            let why = unsafe { follow.borrow_payload_stay_unchecked() };
            Follow::Stay(match why.tag {
                Why::NotStandard => Stay::NotStandard,
                Why::Unavailable => Stay::Unavailable,
                Why::NoSpace => Stay::NoSpace,
                Why::NoWorkspace => Stay::NoWorkspace,
                Why::AlreadyActive => Stay::AlreadyActive,
                Why::PendingWindow => Stay::PendingWindow,
                // SAFETY: the tag says the payload is the mode's word.
                Why::Mode => Stay::Mode(unsafe { *why.borrow_payload_mode_unchecked() }),
                Why::FullscreenActive => Stay::FullscreenActive,
                // SAFETY: the tag says the payload is the window-server id.
                Why::Unmanageable => {
                    Stay::Unmanageable(unsafe { *why.borrow_payload_unmanageable_unchecked() })
                }
                Why::NoBundleId => Stay::NoBundleId,
                Why::Blacklisted => Stay::Blacklisted,
            })
        }
    })
}

pub(crate) fn display_follow_question(
    question: focus::plain::DisplayFollow,
    host: &abi::RocHost,
) -> abi::BoundaryDisplayFollowQuestion {
    abi::BoundaryDisplayFollowQuestion {
        focused_space: maybe_word(question.focused_space),
        previous: maybe_word(question.previous),
        active: word_bools(question.active, host),
        on_known_display: word_bools(question.on_known_display, host),
    }
}

fn known(known: Option<(i32, u64)>) -> abi::BoundaryKnown {
    use abi::BoundaryKnownTag as Tag;
    match known {
        Some((layer, owner)) => union!(
            BoundaryKnown,
            BoundaryKnownPayload.known(abi::BoundaryKnownWindow { layer, owner }),
            Tag::Known
        ),
        None => union!(BoundaryKnown, BoundaryKnownPayload.unknown, Tag::Unknown),
    }
}

pub(crate) fn untracked_question(
    question: focus::plain::Untracked,
    host: &abi::RocHost,
) -> abi::BoundaryUntrackedQuestion {
    abi::BoundaryUntrackedQuestion {
        under_cursor: maybe_word(question.under_cursor),
        tracked: fact_bool(question.tracked, host),
        known: fact_known(question.known, host),
    }
}

pub(crate) fn untracked_answer(
    answer: abi::BoundaryUntrackedAnswer,
) -> Result<UntrackedFocus<u64, u64>, Unasked> {
    use abi::BoundaryUntrackedFocusTag as Tag;
    decision!(answer, BoundaryUntrackedAnswerTag, |focus| match focus.tag {
        Tag::Nothing => UntrackedFocus::Nothing,
        Tag::SystemSurface => {
            // SAFETY: the tag says the payload is the surface.
            let surface = unsafe { focus.borrow_payload_system_surface_unchecked() };
            UntrackedFocus::SystemSurface {
                layer: surface.layer,
                window: surface.window,
            }
        }
        Tag::MakeKey => {
            // SAFETY: the tag says the payload is the window and its app.
            let key = unsafe { focus.borrow_payload_make_key_unchecked() };
            UntrackedFocus::MakeKey(key.owner, key.window)
        }
    })
}

pub(crate) fn desktop_question(
    question: focus::plain::Desktop,
    host: &abi::RocHost,
) -> abi::BoundaryDesktopQuestion {
    abi::BoundaryDesktopQuestion {
        active: question.active,
        workspace_empty: fact_bool(question.workspace_empty, host),
    }
}
