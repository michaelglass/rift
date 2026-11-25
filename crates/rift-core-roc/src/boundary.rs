//! The one place the Rust core's values become the platform's boundary types (`Boundary.roc`,
//! glued into [`abi`]) and back. Every unsafe operation on Roc memory is here: building a tag
//! union's payload, allocating a Roc box, reading a payload after matching its tag, and
//! releasing what Roc answered. Values going in are owned by Roc once handed over (the Roc
//! function consumes its argument); values coming back are read, then released here.

use std::mem::ManuallyDrop;

use rift_core::focus::{
    self, Follow, MouseRaise, Removal, RemovalFocus, Stay, Unraised, UntrackedFocus,
};
use rift_core::layout::bsp::{self, Bsp};
use rift_core::layout::bsp_tree::{
    Content, Direction as TreeDirection, DropAction, Edit, Edited, Focus as TreeFocus, Focused,
    InsertionPoint, Kind, Layout, Leaf, Members, Memory, Node as TreeNode, Orientation, Parent,
    Participation, Preselect, Query, Resize, ResizeAlong, Side, Split, SplitKind, Tabs,
};
use rift_core::layout::floating::{Floating, Newcomer, Seen};
use rift_core::layout::scrolling::{
    self, Alignment, Column, Navigation, Placed, Row, Settle, Settled, Snap, Spot, Strip, Width,
};
use rift_core::layout::traditional::{self, Traditional};
use rift_core::layout::{Gaps, Size, Sizing, WindowConstraints};
use rift_core::lifecycle::{
    self, Admission as WindowAdmission, AdmissionFacts, AxKind, AxValidity, Change, Departure,
    FactChange, FullscreenRecord, Invalidation, InventoryView, Keep, Manageability, Minimize,
    NativeIdentity, NativeKind, NativeLook, NewWindow, OnTitleChange, Ordering, Presence,
    PresenceEvent, PresenceOutcome, Reading, Report, Retirement, Revalidation, Rule, RuleRun,
    Suitability, TransientOrigin, Veto, WorkspaceMode,
};
use rift_core::main_window::{
    App, Initiator, MainWindowEvent, NativeFocus, Standing as MainStanding,
};
use rift_core::plain::{Answers, Fact, Nanos, Plain, Point, Rect, RequestId, Unasked, WindowId};
use rift_core::topology::{
    self, Admission, DiscoveredSpace, DiscoveryStanding, Displays, Hold, MissionControl, Standing,
};
use rift_core::workspaces::{
    self, Destination, Owned, Ownership, Refused, Selector, SkipEmpty, Step, Successor,
    WorkspacesEvent, Wrapping,
};
use rift_core::{
    Answer, Button, Desire, Effect, Event, Note, Plan, Release, Reply, Ungrip, Want, Write,
};

use crate::abi;
use crate::bsp_tree::{Answer as TreeAnswer, Event as TreeEvent, LayoutKey, NodeKey, TreeView};

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

pub(crate) fn floating_layout(
    layout: rift_core::layout::floating::Layout<WindowId>,
    host: &abi::RocHost,
) -> abi::BoundaryFloatingLayout {
    use abi::{BoundaryFloatingPlaceTag as PlaceTag, BoundaryFloatingSlotTag as SlotTag};
    use rift_core::layout::floating::{Place, Slot};
    let windows: Vec<abi::BoundaryFloatingSlot> = layout
        .windows
        .into_iter()
        .map(|slot| match slot {
            Slot::LeftOut => union!(
                BoundaryFloatingSlot,
                BoundaryFloatingSlotPayload.left_out,
                SlotTag::LeftOut
            ),
            Slot::In { window: id, place } => {
                let place = match place {
                    Place::Screen => union!(
                        BoundaryFloatingPlace,
                        BoundaryFloatingPlacePayload.screen,
                        PlaceTag::Screen
                    ),
                    Place::WithinGaps => union!(
                        BoundaryFloatingPlace,
                        BoundaryFloatingPlacePayload.within_gaps,
                        PlaceTag::WithinGaps
                    ),
                    Place::Kept(frame) => union!(
                        BoundaryFloatingPlace,
                        BoundaryFloatingPlacePayload.kept(rect(frame)),
                        PlaceTag::Kept
                    ),
                    Place::First(constraints) => union!(
                        BoundaryFloatingPlace,
                        BoundaryFloatingPlacePayload.first(window_constraints(constraints)),
                        PlaceTag::First
                    ),
                };
                union!(
                    BoundaryFloatingSlot,
                    BoundaryFloatingSlotPayload
                        .placed(abi::BoundaryFloatingWindow { window: window(id), place }),
                    SlotTag::Placed
                )
            }
        })
        .collect();
    abi::BoundaryFloatingLayout {
        screen: rect(layout.floating.screen),
        spread: layout.floating.spread,
        gaps: gaps(layout.gaps),
        windows: list(&windows, host),
    }
}

pub(crate) fn translate_question(
    frames: &[(WindowId, Rect)],
    camera: rift_core::layout::scrolling::Camera,
    fullscreen: &[(WindowId, rift_core::layout::scrolling::Fullscreen)],
    host: &abi::RocHost,
) -> abi::BoundaryTranslateQuestion {
    use rift_core::layout::scrolling::{Fullscreen, Parking};
    let frames: Vec<abi::BoundaryFrame> = frames
        .iter()
        .map(|&(id, frame)| abi::BoundaryFrame {
            window: window(id),
            frame: rect(frame),
        })
        .collect();
    let fullscreen: Vec<abi::BoundaryFullscreenWindow> = fullscreen
        .iter()
        .map(|&(id, mode)| abi::BoundaryFullscreenWindow {
            window: window(id),
            mode: match mode {
                Fullscreen::Screen => abi::BoundaryFullscreenMode::Screen,
                Fullscreen::WithinGaps => abi::BoundaryFullscreenMode::WithinGaps,
            },
        })
        .collect();
    abi::BoundaryTranslateQuestion {
        frames: list(&frames, host),
        screen: rect(camera.screen),
        tiling: rect(camera.tiling),
        offset: camera.offset,
        parking: match camera.parking {
            Parking::Park => abi::BoundaryParking::Park,
            Parking::InPlace => abi::BoundaryParking::InPlace,
        },
        fullscreen: list(&fullscreen, host),
    }
}

// ---------------------------------------------------------------------------------------------
// One axis.

pub(crate) fn axis_question(
    items: &[rift_core::layout::axis::AxisConstraints],
    usable: f64,
    host: &abi::RocHost,
) -> abi::BoundaryAxisQuestion {
    use abi::{BoundaryAxisFixedTag as FixedTag, BoundaryAxisMaxTag as MaxTag};
    let items: Vec<abi::BoundaryAxisItem> = items
        .iter()
        .map(|item| abi::BoundaryAxisItem {
            min: item.min,
            fixed: match item.fixed {
                Some(length) => {
                    union!(
                        BoundaryAxisFixed,
                        BoundaryAxisFixedPayload.fixed(length),
                        FixedTag::Fixed
                    )
                }
                None => union!(BoundaryAxisFixed, BoundaryAxisFixedPayload.free, FixedTag::Free),
            },
            max: match item.max {
                Some(length) => {
                    union!(BoundaryAxisMax, BoundaryAxisMaxPayload.max(length), MaxTag::Max)
                }
                None => union!(BoundaryAxisMax, BoundaryAxisMaxPayload.no_max, MaxTag::NoMax),
            },
            weight: item.weight,
            can_grow: item.can_grow,
        })
        .collect();
    abi::BoundaryAxisQuestion {
        items: list(&items, host),
        usable,
    }
}

pub(crate) fn lengths(answer: abi::RocListWith<f64, false>, host: &abi::RocHost) -> Vec<f64> {
    let lengths = answer.as_slice().to_vec();
    // SAFETY: the list is Roc's answer, owned here and read in full above.
    unsafe { answer.decref(host) };
    lengths
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
    fact_event_class,
    topology::EventClass,
    BoundaryFactEventClass,
    BoundaryFactEventClassPayload,
    BoundaryFactEventClassTag,
    |v, host| match v {
        topology::EventClass::WindowReport => abi::BoundaryEventClass::WindowReport,
        topology::EventClass::WindowInput => abi::BoundaryEventClass::WindowInput,
        topology::EventClass::Other => abi::BoundaryEventClass::Other,
    }
);
fact!(
    fact_origin,
    Option<Point>,
    BoundaryFactOrigin,
    BoundaryFactOriginPayload,
    BoundaryFactOriginTag,
    |v, host| lookup_origin(v)
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

pub(crate) fn raise_question(
    question: focus::plain::Raise,
    host: &abi::RocHost,
) -> abi::BoundaryRaiseQuestion {
    use abi::{
        BoundaryMaybeFrameTag as FrameTag, BoundaryMaybeLevelsTag as LevelsTag,
        BoundaryPointedTag as Tag,
    };
    let floaters: Vec<_> = question
        .floaters
        .0
        .into_iter()
        .map(|(key, frame)| abi::BoundaryWordFrame {
            key,
            answer: match frame {
                Some(frame) => union!(
                    BoundaryMaybeFrame,
                    BoundaryMaybeFramePayload.frame(rect(frame)),
                    FrameTag::Frame
                ),
                None => union!(
                    BoundaryMaybeFrame,
                    BoundaryMaybeFramePayload.no_frame,
                    FrameTag::NoFrame
                ),
            },
        })
        .collect();
    let levels: Vec<_> = question
        .levels
        .0
        .into_iter()
        .map(|(key, levels)| abi::BoundaryWordLevels {
            key,
            answer: match levels {
                Some((level, sub_level)) => union!(
                    BoundaryMaybeLevels,
                    BoundaryMaybeLevelsPayload
                        .levels(abi::BoundaryWindowLevels { level, sub_level }),
                    LevelsTag::Levels
                ),
                None => union!(
                    BoundaryMaybeLevels,
                    BoundaryMaybeLevelsPayload.no_levels,
                    LevelsTag::NoLevels
                ),
            },
        })
        .collect();
    abi::BoundaryRaiseQuestion {
        window: match question.window {
            Some(window) => union!(
                BoundaryPointed,
                BoundaryPointedPayload.hovered(abi::BoundaryHoveredWindow {
                    frame: rect(window.frame),
                    server: maybe_word(window.server),
                }),
                Tag::Hovered
            ),
            None => union!(BoundaryPointed, BoundaryPointedPayload.untracked, Tag::Untracked),
        },
        space: maybe_word(question.space),
        claimed: fact_bool(question.claimed, host),
        admitted: fact_bool(question.admitted, host),
        floating: fact_bool(question.floating, host),
        active: fact_bool(question.active, host),
        in_active_workspace: fact_bool(question.in_active_workspace, host),
        could_occlude: fact_bool(question.could_occlude, host),
        order: fact_words(question.order, host),
        floaters: list(&floaters, host),
        levels: list(&levels, host),
    }
}

pub(crate) fn raise_answer(answer: abi::BoundaryRaiseAnswer) -> Result<MouseRaise, Unasked> {
    use abi::{BoundaryMouseRaiseTag as Tag, BoundaryUnraised as Why};
    decision!(answer, BoundaryRaiseAnswerTag, |raise| match raise.tag {
        Tag::Raise => MouseRaise::Raise,
        Tag::Skip => MouseRaise::Skip(
            // SAFETY: the tag says the payload is the reason.
            match unsafe { *raise.borrow_payload_skip_unchecked() } {
                Why::Unknown => Unraised::Unknown,
                Why::OutsideAdmission => Unraised::OutsideAdmission,
                Why::NoSpace => Unraised::NoSpace,
                Why::InactiveSpace => Unraised::InactiveSpace,
                Why::OutsideActiveWorkspace => Unraised::OutsideActiveWorkspace,
                Why::BuriesFloater => Unraised::BuriesFloater,
            }
        ),
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

// Topology.

pub(crate) fn quarantine_question(
    question: topology::plain::Quarantine,
    host: &abi::RocHost,
) -> abi::BoundaryQuarantineQuestion {
    abi::BoundaryQuarantineQuestion {
        topology: topology_now(question.topology),
        class: fact_event_class(question.class, host),
    }
}

fn lookup_origin(origin: Option<Point>) -> abi::BoundaryLookupOrigin {
    use abi::BoundaryLookupOriginTag as Tag;
    match origin {
        Some(Point { x, y }) => union!(
            BoundaryLookupOrigin,
            BoundaryLookupOriginPayload.at(abi::BoundaryPoint { x, y }),
            Tag::At
        ),
        None => union!(
            BoundaryLookupOrigin,
            BoundaryLookupOriginPayload.nowhere,
            Tag::Nowhere
        ),
    }
}

pub(crate) fn target_question(
    question: topology::plain::Target,
    host: &abi::RocHost,
) -> abi::BoundaryTargetQuestion {
    use abi::BoundaryScreenFoundTag as Tag;
    let screens: Vec<_> = question
        .screen
        .0
        .into_iter()
        .map(|(origin, found)| abi::BoundaryOriginScreen {
            key: lookup_origin(origin),
            answer: match found {
                Some((display, space)) => union!(
                    BoundaryScreenFound,
                    BoundaryScreenFoundPayload.found(abi::BoundaryFoundScreen {
                        display,
                        space: maybe_word(space),
                    }),
                    Tag::Found
                ),
                None => union!(
                    BoundaryScreenFound,
                    BoundaryScreenFoundPayload.not_found,
                    Tag::NotFound
                ),
            },
        })
        .collect();
    abi::BoundaryTargetQuestion {
        source_center: lookup_origin(question.source_center),
        current_center: fact_origin(question.current_center, host),
        screens: list(&screens, host),
        active: word_bools(question.active, host),
    }
}

pub(crate) fn target_answer(
    answer: abi::BoundaryTargetAnswer,
) -> Result<Result<(u64, u64), topology::NoTarget>, Unasked> {
    use abi::BoundaryTargetTag as Tag;
    decision!(answer, BoundaryTargetAnswerTag, |target| match target.tag {
        Tag::Display => {
            // SAFETY: the tag says the payload is the display and its Space.
            let found = unsafe { target.borrow_payload_display_unchecked() };
            Ok((found.display, found.space))
        }
        Tag::DisplayNotFound => Err(topology::NoTarget::DisplayNotFound),
        Tag::SpaceUnavailable => Err(topology::NoTarget::SpaceUnavailable),
    })
}

fn maybe_space(space: Option<u64>) -> abi::BoundaryMaybeSpace {
    use abi::BoundaryMaybeSpaceTag as Tag;
    match space {
        Some(space) => union!(
            BoundaryMaybeSpace,
            BoundaryMaybeSpacePayload.space(space),
            Tag::Space
        ),
        None => union!(
            BoundaryMaybeSpace,
            BoundaryMaybeSpacePayload.no_space,
            Tag::NoSpace
        ),
    }
}

fn from_maybe_space(space: abi::BoundaryMaybeSpace) -> Option<u64> {
    use abi::BoundaryMaybeSpaceTag as Tag;
    match space.tag {
        // SAFETY: the tag says the payload is the Space.
        Tag::Space => Some(unsafe { *space.borrow_payload_space_unchecked() }),
        Tag::NoSpace => None,
    }
}

fact!(
    fact_space,
    Option<u64>,
    BoundaryFactSpace,
    BoundaryFactSpacePayload,
    BoundaryFactSpaceTag,
    |v, host| maybe_space(v)
);

pub(crate) fn discovered_question(
    question: topology::plain::Discovered,
    host: &abi::RocHost,
) -> abi::BoundaryDiscoveredQuestion {
    abi::BoundaryDiscoveredQuestion {
        standing: match question.standing {
            Standing::RecoveringFullscreen => abi::BoundaryStanding::RecoveringFullscreen,
            Standing::Untracked => abi::BoundaryStanding::Untracked,
            Standing::Tracked => abi::BoundaryStanding::Tracked,
        },
        live: fact_words(question.live, host),
        observable: word_bools(question.observable, host),
        recorded: fact_space(question.recorded, host),
        resolved: fact_space(question.resolved, host),
    }
}

pub(crate) fn discovered_answer(
    answer: abi::BoundaryDiscoveredAnswer,
) -> Result<DiscoveredSpace<u64>, Unasked> {
    use abi::BoundaryDiscoveredSpaceTag as Tag;
    decision!(answer, BoundaryDiscoveredAnswerTag, |found| match found.tag {
        // SAFETY: the tag says the payload is the Space.
        Tag::Observed =>
            DiscoveredSpace::Observed(unsafe { *found.borrow_payload_observed_unchecked() }),
        // SAFETY: the tag says the payload is the Space.
        Tag::Live => DiscoveredSpace::Live(unsafe { *found.borrow_payload_live_unchecked() }),
        // SAFETY: the tag says the payload is the fallback.
        Tag::Fallback => DiscoveredSpace::Fallback(from_maybe_space(unsafe {
            *found.borrow_payload_fallback_unchecked()
        })),
    })
}

pub(crate) fn discovery_question(
    question: topology::plain::Discovery,
    host: &abi::RocHost,
) -> abi::BoundaryDiscoveryQuestion {
    abi::BoundaryDiscoveryQuestion {
        standing: match question.standing {
            DiscoveryStanding::FullscreenRecorded => {
                abi::BoundaryDiscoveryStanding::FullscreenRecorded
            }
            DiscoveryStanding::Placeholder => abi::BoundaryDiscoveryStanding::Placeholder,
            DiscoveryStanding::Admissible => abi::BoundaryDiscoveryStanding::Admissible,
        },
        native: maybe_space(question.native),
        assigned: maybe_space(question.assigned),
        by_frame: fact_space(question.by_frame, host),
    }
}

pub(crate) fn discovery_answer(
    answer: abi::BoundaryDiscoveryAnswer,
) -> Result<(Option<u64>, Option<u64>), Unasked> {
    decision!(answer, BoundaryDiscoveryAnswerTag, |spaces| (
        from_maybe_space(spaces.authoritative),
        from_maybe_space(spaces.discovery)
    ))
}

fn displays(displays: Displays<u64, u64>, host: &abi::RocHost) -> abi::BoundaryDisplays {
    use abi::BoundaryScreenTag as Tag;
    let screens: Vec<_> = displays
        .screens
        .into_iter()
        .map(|screen| match screen {
            Some(display) => union!(
                BoundaryScreen,
                BoundaryScreenPayload.display(display),
                Tag::Display
            ),
            None => union!(BoundaryScreen, BoundaryScreenPayload.unnamed, Tag::Unnamed),
        })
        .collect();
    let spaces: Vec<_> = displays
        .spaces
        .into_iter()
        .map(|(display, spaces)| abi::BoundaryDisplaySpaces {
            display,
            spaces: list(&spaces, host),
        })
        .collect();
    abi::BoundaryDisplays {
        screens: list(&screens, host),
        spaces: list(&spaces, host),
    }
}

pub(crate) fn admit_question(
    question: topology::plain::Admit,
    host: &abi::RocHost,
) -> abi::BoundaryAdmitQuestion {
    let incoming = question.incoming;
    let remaps: Vec<_> = incoming
        .remaps
        .into_iter()
        .map(|(old, new)| abi::BoundaryRemap { old, new })
        .collect();
    abi::BoundaryAdmitQuestion {
        mission_control: match question.mission_control {
            MissionControl::Open => abi::BoundaryMissionControl::Open,
            MissionControl::Closed => abi::BoundaryMissionControl::Closed,
        },
        current: displays(question.current, host),
        incoming: abi::BoundaryIncoming {
            displays: displays(incoming.displays, host),
            remaps: list(&remaps, host),
            display_set_changed: incoming.display_set_changed,
            force_refresh: incoming.force_refresh,
            membership_complete: incoming.membership_complete,
        },
    }
}

/// The admission Roc answered, read out and released.
pub(crate) fn from_admission(
    answer: abi::BoundaryAdmission,
    host: &abi::RocHost,
) -> Admission<u64> {
    use abi::BoundaryAdmissionTag as Tag;
    let admission = match answer.tag {
        // SAFETY: the tag says the payload is why it holds.
        Tag::Hold => Admission::Hold(match unsafe { *answer.borrow_payload_hold_unchecked() } {
            abi::BoundaryHold::MissionControl => Hold::MissionControl,
            abi::BoundaryHold::MembershipIncomplete => Hold::MembershipIncomplete,
        }),
        Tag::Commit => {
            // SAFETY: the tag says the payload is the commit.
            let commit = unsafe { answer.borrow_payload_commit_unchecked() };
            Admission::Commit {
                invalidated: commit.invalidated.as_slice().to_vec(),
                lost_local_desktops: commit.lost.as_slice().to_vec(),
            }
        }
    };
    // SAFETY: the admission is Roc's answer, owned here and read in full above.
    unsafe { answer.decref(host) };
    admission
}

pub(crate) fn candidates_question(
    question: topology::plain::Candidates,
    host: &abi::RocHost,
) -> abi::BoundaryCandidatesQuestion {
    use abi::BoundaryTrackedWindowTag as Tag;
    let window_spaces: Vec<_> = question
        .window_spaces
        .into_iter()
        .map(|(server, space)| abi::BoundaryServerSpace { server, space })
        .collect();
    let tracked: Vec<_> = question
        .tracked
        .0
        .into_iter()
        .map(|(key, answer)| abi::BoundaryWordTracked {
            key,
            answer: match answer {
                Some(id) => union!(
                    BoundaryTrackedWindow,
                    BoundaryTrackedWindowPayload.tracked(window(id)),
                    Tag::Tracked
                ),
                None => union!(
                    BoundaryTrackedWindow,
                    BoundaryTrackedWindowPayload.not_tracked,
                    Tag::NotTracked
                ),
            },
        })
        .collect();
    abi::BoundaryCandidatesQuestion {
        window_spaces: list(&window_spaces, host),
        activated: list(&question.activated, host),
        tracked: list(&tracked, host),
        reconcilable: window_bools(question.reconcilable, host),
    }
}

/// The candidates Roc answered, read out and released.
pub(crate) fn candidates_answer(
    answer: abi::BoundaryCandidatesAnswer,
    host: &abi::RocHost,
) -> Result<Vec<(i32, Vec<WindowId>)>, Unasked> {
    let candidates = decision!(answer, BoundaryCandidatesAnswerTag, |apps| apps
        .as_slice()
        .iter()
        .map(|app| (
            app.pid,
            app.windows.as_slice().iter().copied().map(from_window).collect()
        ))
        .collect());
    // SAFETY: the answer is Roc's, owned here and read in full above.
    unsafe { answer.decref(host) };
    candidates
}

// Lifecycle.

fn rule_run(run: RuleRun) -> abi::BoundaryRuleRun {
    match run {
        RuleRun::Admission => abi::BoundaryRuleRun::Admission,
        RuleRun::Reapply => abi::BoundaryRuleRun::Reapply,
    }
}

fn word_spaces(
    answers: Answers<u64, Option<u64>>,
    host: &abi::RocHost,
) -> abi::RocListWith<abi::BoundaryWordSpace, false> {
    let answers: Vec<_> = answers
        .0
        .into_iter()
        .map(|(key, answer)| abi::BoundaryWordSpace {
            key,
            answer: maybe_space(answer),
        })
        .collect();
    list(&answers, host)
}

pub(crate) fn rule_targets_question(
    question: lifecycle::plain::RuleTargets,
    host: &abi::RocHost,
) -> abi::BoundaryRuleTargetsQuestion {
    abi::BoundaryRuleTargetsQuestion {
        windows: list(&question.windows, host),
        run: rule_run(question.run),
        reconcilable: word_bools(question.reconcilable, host),
        spaces: word_spaces(question.spaces, host),
        active: word_bools(question.active, host),
    }
}

/// The rule targets Roc answered, read out and released.
pub(crate) fn rule_targets_answer(
    answer: abi::BoundaryRuleTargetsAnswer,
    host: &abi::RocHost,
) -> Result<Vec<(u64, Vec<u64>)>, Unasked> {
    let targets = decision!(answer, BoundaryRuleTargetsAnswerTag, |groups| groups
        .as_slice()
        .iter()
        .map(|group| (group.space, group.windows.as_slice().to_vec()))
        .collect());
    // SAFETY: the answer is Roc's, owned here and read in full above.
    unsafe { answer.decref(host) };
    targets
}

pub(crate) fn rule_refresh_question(
    question: lifecycle::plain::RuleRefresh,
) -> abi::BoundaryRuleRefreshQuestion {
    let outcome = question.outcome;
    abi::BoundaryRuleRefreshQuestion {
        run: rule_run(question.run),
        outcome: abi::BoundaryRuleOutcome {
            workspace_before: maybe_space(outcome.workspace_before),
            workspace: outcome.workspace,
            floating_before: outcome.floating_before,
            floating: outcome.floating,
            was_ignored: outcome.was_ignored,
            unavailable: outcome.unavailable,
        },
    }
}

pub(crate) fn pointer_target_question(
    question: lifecycle::plain::PointerTarget,
    host: &abi::RocHost,
) -> abi::BoundaryPointerTargetQuestion {
    abi::BoundaryPointerTargetQuestion {
        mode: match question.mode {
            WorkspaceMode::PerDisplay => abi::BoundaryWorkspaceMode::PerDisplay,
            WorkspaceMode::Global => abi::BoundaryWorkspaceMode::Global,
        },
        opened_on: question.opened_on,
        pointer: fact_space(question.pointer, host),
        active: word_bools(question.active, host),
        window: match question.window {
            NewWindow::Untracked => abi::BoundaryNewWindow::Untracked,
            NewWindow::NotStandard => abi::BoundaryNewWindow::NotStandard,
            NewWindow::Standard(()) => abi::BoundaryNewWindow::Standard,
        },
        rule_names_workspace: fact_bool(question.rule_names_workspace, host),
    }
}

pub(crate) fn space_answer(answer: abi::BoundarySpaceAnswer) -> Result<Option<u64>, Unasked> {
    decision!(answer, BoundarySpaceAnswerTag, |space| from_maybe_space(*space))
}

fn origin(origin: TransientOrigin) -> abi::BoundaryOrigin {
    match origin {
        TransientOrigin::Native => abi::BoundaryOrigin::Native,
        TransientOrigin::Heuristic => abi::BoundaryOrigin::Heuristic,
    }
}

pub(crate) fn native_transient_question(
    question: lifecycle::plain::NativeTransient,
    host: &abi::RocHost,
) -> abi::BoundaryNativeTransientQuestion {
    use abi::BoundaryTransientEntryTag as Tag;
    abi::BoundaryNativeTransientQuestion {
        entry: match question.entry {
            Some((from, pid)) => union!(
                BoundaryTransientEntry,
                BoundaryTransientEntryPayload
                    .entry(abi::BoundaryEntryOf { origin: origin(from), pid }),
                Tag::Entry
            ),
            None => union!(
                BoundaryTransientEntry,
                BoundaryTransientEntryPayload.no_entry,
                Tag::NoEntry
            ),
        },
        owner: question.owner,
        exempt: fact_bool(question.exempt, host),
    }
}

pub(crate) fn keeps_transient_question(
    question: lifecycle::plain::KeepsTransient,
) -> abi::BoundaryKeepsTransientQuestion {
    use abi::BoundaryDepartureTag as Tag;
    abi::BoundaryKeepsTransientQuestion {
        departure: match question.departure {
            Departure::Destroyed(gone) => union!(
                BoundaryDeparture,
                BoundaryDeparturePayload.destroyed(maybe_space(gone)),
                Tag::Destroyed
            ),
            Departure::Closed(gone) => union!(
                BoundaryDeparture,
                BoundaryDeparturePayload.closed(gone),
                Tag::Closed
            ),
            Departure::AppGone(pid) => union!(
                BoundaryDeparture,
                BoundaryDeparturePayload.app_gone(pid),
                Tag::AppGone
            ),
        },
        id: question.id,
        origin: origin(question.origin),
        owner: question.owner,
    }
}

pub(crate) fn misplaced_question(
    question: lifecycle::plain::Misplaced,
) -> abi::BoundaryMisplacedQuestion {
    abi::BoundaryMisplacedQuestion {
        assigned: maybe_space(question.assigned),
        native: maybe_space(question.native),
    }
}

pub(crate) fn out_of_view_question(
    question: lifecycle::plain::OutOfView,
    host: &abi::RocHost,
) -> abi::BoundaryOutOfViewQuestion {
    abi::BoundaryOutOfViewQuestion {
        native: maybe_space(question.native),
        assigned: fact_space(question.assigned, host),
        active: word_bools(question.active, host),
        parked: fact_bool(question.parked, host),
    }
}

pub(crate) fn title_change_question(
    question: lifecycle::plain::TitleChange,
    host: &abi::RocHost,
) -> abi::BoundaryTitleChangeQuestion {
    abi::BoundaryTitleChangeQuestion {
        configured: match question.configured {
            OnTitleChange::Reapply => abi::BoundaryOnTitleChange::Reapply,
            OnTitleChange::Ignore => abi::BoundaryOnTitleChange::Ignore,
        },
        space: fact_space(question.space, host),
        active: word_bools(question.active, host),
        reconcilable: fact_bool(question.reconcilable, host),
    }
}

fn manageability(manageability: Manageability) -> abi::BoundaryManageability {
    match manageability {
        Manageability::Manageable => abi::BoundaryManageability::Manageable,
        Manageability::Unmanageable => abi::BoundaryManageability::Unmanageable,
    }
}

fn rule(rule: Rule) -> abi::BoundaryRule {
    match rule {
        Rule::NoRule => abi::BoundaryRule::NoRule,
        Rule::Manage => abi::BoundaryRule::Manage,
        Rule::Ignore => abi::BoundaryRule::Ignore,
    }
}

fn ax_kind(ax: AxKind) -> abi::BoundaryAxKind {
    match ax {
        AxKind::Ordinary => abi::BoundaryAxKind::Ordinary,
        AxKind::DialogWindow => abi::BoundaryAxKind::DialogWindow,
        AxKind::Transient => abi::BoundaryAxKind::Transient,
    }
}

fn native_kind(native: NativeKind) -> abi::BoundaryNativeKind {
    match native {
        NativeKind::Ordinary => abi::BoundaryNativeKind::Ordinary,
        NativeKind::Transient => abi::BoundaryNativeKind::Transient,
    }
}

fn minimize(minimize: Minimize) -> abi::BoundaryMinimize {
    match minimize {
        Minimize::Shown => abi::BoundaryMinimize::Shown,
        Minimize::Minimized => abi::BoundaryMinimize::Minimized,
    }
}

pub(crate) fn admission_facts(facts: AdmissionFacts) -> abi::BoundaryAdmissionFacts {
    abi::BoundaryAdmissionFacts {
        manageability: manageability(facts.manageability),
        rule: rule(facts.rule),
        ax: ax_kind(facts.ax),
        native: native_kind(facts.native),
        minimize: minimize(facts.minimize),
    }
}

fn from_admission_facts(facts: abi::BoundaryAdmissionFacts) -> AdmissionFacts {
    AdmissionFacts {
        manageability: match facts.manageability {
            abi::BoundaryManageability::Manageable => Manageability::Manageable,
            abi::BoundaryManageability::Unmanageable => Manageability::Unmanageable,
        },
        rule: match facts.rule {
            abi::BoundaryRule::NoRule => Rule::NoRule,
            abi::BoundaryRule::Manage => Rule::Manage,
            abi::BoundaryRule::Ignore => Rule::Ignore,
        },
        ax: match facts.ax {
            abi::BoundaryAxKind::Ordinary => AxKind::Ordinary,
            abi::BoundaryAxKind::DialogWindow => AxKind::DialogWindow,
            abi::BoundaryAxKind::Transient => AxKind::Transient,
        },
        native: match facts.native {
            abi::BoundaryNativeKind::Ordinary => NativeKind::Ordinary,
            abi::BoundaryNativeKind::Transient => NativeKind::Transient,
        },
        minimize: match facts.minimize {
            abi::BoundaryMinimize::Shown => Minimize::Shown,
            abi::BoundaryMinimize::Minimized => Minimize::Minimized,
        },
    }
}

fn veto(veto: Veto) -> abi::BoundaryVeto {
    match veto {
        Veto::NativeTransient => abi::BoundaryVeto::NativeTransient,
        Veto::Minimized => abi::BoundaryVeto::Minimized,
        Veto::AxTransient => abi::BoundaryVeto::AxTransient,
        Veto::RuleIgnores => abi::BoundaryVeto::RuleIgnores,
        Veto::Unmanageable => abi::BoundaryVeto::Unmanageable,
    }
}

fn from_veto(veto: abi::BoundaryVeto) -> Veto {
    match veto {
        abi::BoundaryVeto::NativeTransient => Veto::NativeTransient,
        abi::BoundaryVeto::Minimized => Veto::Minimized,
        abi::BoundaryVeto::AxTransient => Veto::AxTransient,
        abi::BoundaryVeto::RuleIgnores => Veto::RuleIgnores,
        abi::BoundaryVeto::Unmanageable => Veto::Unmanageable,
    }
}

pub(crate) fn from_window_admission(admission: abi::BoundaryWindowAdmission) -> WindowAdmission {
    use abi::BoundaryWindowAdmissionTag as Tag;
    match admission.tag {
        Tag::Admitted => WindowAdmission::Admitted,
        // SAFETY: the tag says the payload is the veto.
        Tag::Vetoed => WindowAdmission::Vetoed(from_veto(unsafe {
            *admission.borrow_payload_vetoed_unchecked()
        })),
    }
}

fn reading(reading: Reading) -> abi::BoundaryReading {
    use abi::{BoundaryFactChangeTag as Fact, BoundaryReadingTag as Tag};
    match reading {
        Reading::Live(facts) => union!(
            BoundaryReading,
            BoundaryReadingPayload.live(admission_facts(facts)),
            Tag::Live
        ),
        Reading::Changed(change) => {
            let change = match change {
                FactChange::Manageability(m) => union!(
                    BoundaryFactChange,
                    BoundaryFactChangePayload.manageability(manageability(m)),
                    Fact::Manageability
                ),
                FactChange::Rule(r) => union!(
                    BoundaryFactChange,
                    BoundaryFactChangePayload.rule(rule(r)),
                    Fact::Rule
                ),
                FactChange::Ax(a) => union!(
                    BoundaryFactChange,
                    BoundaryFactChangePayload.ax(ax_kind(a)),
                    Fact::Ax
                ),
                FactChange::Native(n) => union!(
                    BoundaryFactChange,
                    BoundaryFactChangePayload.native(native_kind(n)),
                    Fact::Native
                ),
                FactChange::Minimize(m) => union!(
                    BoundaryFactChange,
                    BoundaryFactChangePayload.minimize(minimize(m)),
                    Fact::Minimize
                ),
            };
            union!(
                BoundaryReading,
                BoundaryReadingPayload.changed(change),
                Tag::Changed
            )
        }
        Reading::Gone => union!(BoundaryReading, BoundaryReadingPayload.gone, Tag::Gone),
    }
}

fn known_facts(facts: Option<AdmissionFacts>) -> abi::BoundaryKnownFacts {
    use abi::BoundaryKnownFactsTag as Tag;
    match facts {
        Some(facts) => union!(
            BoundaryKnownFacts,
            BoundaryKnownFactsPayload.known(admission_facts(facts)),
            Tag::Known
        ),
        None => union!(
            BoundaryKnownFacts,
            BoundaryKnownFactsPayload.no_facts,
            Tag::NoFacts
        ),
    }
}

fn from_known_facts(facts: abi::BoundaryKnownFacts) -> Option<AdmissionFacts> {
    use abi::BoundaryKnownFactsTag as Tag;
    match facts.tag {
        // SAFETY: the tag says the payload is the facts.
        Tag::Known => Some(from_admission_facts(unsafe {
            *facts.borrow_payload_known_unchecked()
        })),
        Tag::NoFacts => None,
    }
}

pub(crate) fn change(change: Option<Change>) -> abi::BoundaryChange {
    use abi::BoundaryChangeTag as Tag;
    match change {
        None => union!(BoundaryChange, BoundaryChangePayload.no_change, Tag::NoChange),
        Some(Change::Admitted) => {
            union!(BoundaryChange, BoundaryChangePayload.admitted, Tag::Admitted)
        }
        Some(Change::Suspended(why)) => union!(
            BoundaryChange,
            BoundaryChangePayload.suspended(veto(why)),
            Tag::Suspended
        ),
    }
}

pub(crate) fn from_change(change: abi::BoundaryChange) -> Option<Change> {
    use abi::BoundaryChangeTag as Tag;
    match change.tag {
        Tag::NoChange => None,
        Tag::Admitted => Some(Change::Admitted),
        // SAFETY: the tag says the payload is the veto.
        Tag::Suspended => Some(Change::Suspended(from_veto(unsafe {
            *change.borrow_payload_suspended_unchecked()
        }))),
    }
}

pub(crate) fn read_question(
    before: Option<AdmissionFacts>,
    read: Reading,
) -> abi::BoundaryReadQuestion {
    abi::BoundaryReadQuestion {
        before: known_facts(before),
        reading: reading(read),
    }
}

pub(crate) fn from_read(
    answer: abi::BoundaryReadAnswer,
) -> (Option<AdmissionFacts>, Option<Change>) {
    (from_known_facts(answer.after), from_change(answer.change))
}

fn presence(presence: Option<Presence>) -> abi::BoundaryPresence {
    use abi::BoundaryPresenceTag as Tag;
    match presence {
        None => union!(
            BoundaryPresence,
            BoundaryPresencePayload.no_presence,
            Tag::NoPresence
        ),
        Some(Presence::Live(facts)) => union!(
            BoundaryPresence,
            BoundaryPresencePayload.live(admission_facts(facts)),
            Tag::Live
        ),
        Some(Presence::Invalidated(facts)) => union!(
            BoundaryPresence,
            BoundaryPresencePayload.invalidated(admission_facts(facts)),
            Tag::Invalidated
        ),
    }
}

fn from_presence(presence: abi::BoundaryPresence) -> Option<Presence> {
    use abi::BoundaryPresenceTag as Tag;
    match presence.tag {
        Tag::NoPresence => None,
        // SAFETY: the tag says the payload is the facts.
        Tag::Live => Some(Presence::Live(from_admission_facts(unsafe {
            *presence.borrow_payload_live_unchecked()
        }))),
        // SAFETY: the tag says the payload is the facts.
        Tag::Invalidated => Some(Presence::Invalidated(from_admission_facts(unsafe {
            *presence.borrow_payload_invalidated_unchecked()
        }))),
    }
}

pub(crate) fn presence_question(
    before: Option<Presence>,
    event: PresenceEvent,
) -> abi::BoundaryPresenceQuestion {
    use abi::BoundaryPresenceEventTag as Tag;
    abi::BoundaryPresenceQuestion {
        before: presence(before),
        event: match event {
            PresenceEvent::Reading(read) => union!(
                BoundaryPresenceEvent,
                BoundaryPresenceEventPayload.reading(reading(read)),
                Tag::Reading
            ),
            PresenceEvent::Invalidate => union!(
                BoundaryPresenceEvent,
                BoundaryPresenceEventPayload.invalidate,
                Tag::Invalidate
            ),
            PresenceEvent::Revalidate => union!(
                BoundaryPresenceEvent,
                BoundaryPresenceEventPayload.revalidate,
                Tag::Revalidate
            ),
        },
    }
}

/// A presence event's answer: the presence after, what the event came to, whether rift manages
/// the window then.
pub(crate) fn from_presence_answer(
    answer: abi::BoundaryPresenceAnswer,
) -> (Option<Presence>, PresenceOutcome, bool) {
    use abi::BoundaryPresenceOutcomeTag as Tag;
    let outcome = answer.outcome;
    let outcome = match outcome.tag {
        // SAFETY: the tag says the payload is the change.
        Tag::Read => {
            PresenceOutcome::Read(from_change(unsafe { *outcome.borrow_payload_read_unchecked() }))
        }
        // SAFETY: the tag says the payload is whether it invalidated.
        Tag::Invalidated => PresenceOutcome::Invalidated(
            if unsafe { *outcome.borrow_payload_invalidated_unchecked() } {
                Invalidation::Invalidated
            } else {
                Invalidation::NoAxState
            },
        ),
        // SAFETY: the tag says the payload is whether it revalidated.
        Tag::Revalidated => PresenceOutcome::Revalidated(
            if unsafe { *outcome.borrow_payload_revalidated_unchecked() } {
                Revalidation::Revalidated
            } else {
                Revalidation::NotInvalidated
            },
        ),
    };
    (from_presence(answer.after), outcome, answer.manages)
}

pub(crate) fn inventory_question(
    question: lifecycle::plain::Inventory,
) -> abi::BoundaryInventoryQuestion {
    use abi::{BoundaryFactEvidenceTag as Evidence, BoundaryNativeIdentityTag as Identity};
    let facts = question.facts;
    abi::BoundaryInventoryQuestion {
        facts: abi::BoundaryInventoryFacts {
            report: match facts.report {
                Report::Reported => abi::BoundaryReport::Reported,
                Report::Omitted => abi::BoundaryReport::Omitted,
            },
            view: match facts.view {
                InventoryView::InView => abi::BoundaryInventoryView::InView,
                InventoryView::OutOfView => abi::BoundaryInventoryView::OutOfView,
            },
            minimize: match facts.minimize {
                Some(Minimize::Shown) => abi::BoundaryInventoryMinimize::Shown,
                Some(Minimize::Minimized) => abi::BoundaryInventoryMinimize::Minimized,
                None => abi::BoundaryInventoryMinimize::NoAxState,
            },
            fullscreen: match facts.fullscreen {
                FullscreenRecord::Recorded => abi::BoundaryFullscreenRecord::Recorded,
                FullscreenRecord::NotRecorded => abi::BoundaryFullscreenRecord::NotRecorded,
            },
            identity: match facts.identity {
                NativeIdentity::Known(id) => union!(
                    BoundaryNativeIdentity,
                    BoundaryNativeIdentityPayload.known(id),
                    Identity::Known
                ),
                NativeIdentity::Unknown => union!(
                    BoundaryNativeIdentity,
                    BoundaryNativeIdentityPayload.unknown,
                    Identity::Unknown
                ),
            },
            validity: match facts.validity {
                AxValidity::Valid => abi::BoundaryAxValidity::Valid,
                AxValidity::Invalidated => abi::BoundaryAxValidity::Invalidated,
            },
        },
        evidence: match question.evidence {
            Fact::Asked(evidence) => union!(
                BoundaryFactEvidence,
                BoundaryFactEvidencePayload.asked(abi::BoundaryNativeEvidence {
                    suitability: match evidence.suitability {
                        Some(Suitability::Suitable) => abi::BoundarySuitability::Suitable,
                        Some(Suitability::Unsuitable) => abi::BoundarySuitability::Unsuitable,
                        None => abi::BoundarySuitability::SuitabilityUnknown,
                    },
                    ordering: match evidence.ordering {
                        Some(Ordering::OrderedIn) => abi::BoundaryOrdering::OrderedIn,
                        Some(Ordering::OrderedOut) => abi::BoundaryOrdering::OrderedOut,
                        None => abi::BoundaryOrdering::OrderingUnknown,
                    },
                    look: match evidence.look {
                        Some(NativeLook::Window) => abi::BoundaryLook::AWindow,
                        Some(NativeLook::NotAWindow) => abi::BoundaryLook::NotAWindow,
                        None => abi::BoundaryLook::LookUnknown,
                    },
                }),
                Evidence::Asked
            ),
            Fact::Unasked => union!(
                BoundaryFactEvidence,
                BoundaryFactEvidencePayload.unasked,
                Evidence::Unasked
            ),
        },
    }
}

pub(crate) fn retirement_answer(
    answer: abi::BoundaryRetirementAnswer,
) -> Result<Retirement, Unasked> {
    use abi::BoundaryRetirementTag as Tag;
    decision!(
        answer,
        BoundaryRetirementAnswerTag,
        |retirement| match retirement.tag {
            Tag::Retire => Retirement::Retire,
            // SAFETY: the tag says the payload is why it keeps the window.
            Tag::Keep =>
                Retirement::Keep(match unsafe { *retirement.borrow_payload_keep_unchecked() } {
                    abi::BoundaryKeep::Reported => Keep::Reported,
                    abi::BoundaryKeep::OutOfView => Keep::OutOfView,
                    abi::BoundaryKeep::NoAxState => Keep::NoAxState,
                    abi::BoundaryKeep::Minimized => Keep::Minimized,
                    abi::BoundaryKeep::Fullscreen => Keep::Fullscreen,
                    abi::BoundaryKeep::NoNativeIdentity => Keep::NoNativeIdentity,
                    abi::BoundaryKeep::NativeOrdering => Keep::NativeOrdering,
                    abi::BoundaryKeep::NoNegativeEvidence => Keep::NoNegativeEvidence,
                }),
        }
    )
}

// Workspaces.

fn workspace_mode(mode: WorkspaceMode) -> abi::BoundaryWorkspaceMode {
    match mode {
        WorkspaceMode::PerDisplay => abi::BoundaryWorkspaceMode::PerDisplay,
        WorkspaceMode::Global => abi::BoundaryWorkspaceMode::Global,
    }
}

fn context(space: u64, visible: Vec<u64>, host: &abi::RocHost) -> abi::BoundaryContext {
    abi::BoundaryContext {
        space,
        visible: list(&visible, host),
    }
}

pub(crate) fn step_question(
    question: workspaces::plain::StepQuestion,
    host: &abi::RocHost,
) -> abi::BoundaryStepQuestion {
    let catalog: Vec<_> = question
        .catalog
        .into_iter()
        .map(|(id, space)| abi::BoundaryCatalogEntry { id, space })
        .collect();
    abi::BoundaryStepQuestion {
        mode: workspace_mode(question.mode),
        catalog: list(&catalog, host),
        context: context(question.space, question.visible, host),
        current: question.current,
        way: match question.way {
            Step::Next => abi::BoundaryWay::Next,
            Step::Previous => abi::BoundaryWay::Previous,
        },
        skip_empty: match question.skip_empty {
            SkipEmpty::Skip => abi::BoundarySkipEmpty::Skip,
            SkipEmpty::Keep => abi::BoundarySkipEmpty::Keep,
        },
        wrapping: match question.wrapping {
            Wrapping::Wrap => abi::BoundaryWrapping::Wrap,
            Wrapping::Stop => abi::BoundaryWrapping::Stop,
        },
        has_windows: word_bools(question.has_windows, host),
    }
}

pub(crate) fn resolve_question(
    question: workspaces::plain::ResolveQuestion,
    host: &abi::RocHost,
) -> abi::BoundaryResolveQuestion {
    use abi::BoundarySelectorTag as Tag;
    let scope: Vec<_> = question
        .scope
        .into_iter()
        .map(|(id, space, name)| abi::BoundaryScopeEntry { id, space, name })
        .collect();
    abi::BoundaryResolveQuestion {
        mode: workspace_mode(question.mode),
        scope: list(&scope, host),
        context: context(question.space, question.visible, host),
        selector: match question.selector {
            Selector::Index(index) => {
                union!(
                    BoundarySelector,
                    BoundarySelectorPayload.index(index as u64),
                    Tag::Index
                )
            }
            Selector::Name(name) => {
                union!(BoundarySelector, BoundarySelectorPayload.name(name), Tag::Name)
            }
        },
    }
}

pub(crate) fn previous_question(
    question: workspaces::plain::PreviousQuestion,
    host: &abi::RocHost,
) -> abi::BoundaryPreviousQuestion {
    abi::BoundaryPreviousQuestion {
        mode: workspace_mode(question.mode),
        context: context(question.space, question.visible, host),
        last: maybe_word(question.last),
        on: fact_word(question.on, host),
    }
}

pub(crate) fn eligible_question(
    question: workspaces::plain::EligibleQuestion,
    host: &abi::RocHost,
) -> abi::BoundaryEligibleQuestion {
    abi::BoundaryEligibleQuestion {
        mode: workspace_mode(question.mode),
        context: context(question.space, question.visible, host),
        on: question.on,
    }
}

pub(crate) fn emits_question(
    question: workspaces::plain::EmitsQuestion,
    host: &abi::RocHost,
) -> abi::BoundaryEmitsQuestion {
    abi::BoundaryEmitsQuestion {
        assigned: maybe_word(question.assigned),
        active: fact_word(question.active, host),
    }
}

pub(crate) fn reached_answer(
    answer: abi::BoundaryReachedAnswer,
) -> Result<Option<(u64, u64)>, Unasked> {
    decision!(answer, BoundaryReachedAnswerTag, |reached| from_reached(
        *reached
    ))
}

pub(crate) fn from_reached(reached: abi::BoundaryReached) -> Option<(u64, u64)> {
    use abi::BoundaryReachedTag as Tag;
    match reached.tag {
        Tag::Reached => {
            // SAFETY: the tag says the payload is the workspace and its Space.
            let at = unsafe { reached.borrow_payload_reached_unchecked() };
            Some((at.workspace, at.space))
        }
        Tag::Unreached => None,
    }
}

fn activation(activation: workspaces::Activation<u64>) -> abi::BoundaryActivation {
    use abi::BoundaryActivationTag as Tag;
    match activation {
        workspaces::Activation::Showing { current, previous, remembered } => union!(
            BoundaryActivation,
            BoundaryActivationPayload.showing(abi::BoundaryShowing {
                current,
                previous: maybe_word(previous),
                remembered: maybe_word(remembered),
            }),
            Tag::Showing
        ),
        workspaces::Activation::Vacant { remembered } => {
            union!(
                BoundaryActivation,
                BoundaryActivationPayload.vacant(maybe_word(remembered)),
                Tag::Vacant
            )
        }
    }
}

fn from_activation(activation: abi::BoundaryActivation) -> workspaces::Activation<u64> {
    use abi::BoundaryActivationTag as Tag;
    match activation.tag {
        Tag::Showing => {
            // SAFETY: the tag says the payload is what the Space shows.
            let showing = unsafe { activation.borrow_payload_showing_unchecked() };
            workspaces::Activation::Showing {
                current: showing.current,
                previous: from_maybe_word(showing.previous),
                remembered: from_maybe_word(showing.remembered),
            }
        }
        Tag::Vacant => workspaces::Activation::Vacant {
            // SAFETY: the tag says the payload is the remembered workspace.
            remembered: from_maybe_word(unsafe { *activation.borrow_payload_vacant_unchecked() }),
        },
    }
}

fn workspace_space(workspace: u64, space: u64) -> abi::BoundaryWorkspaceSpace {
    abi::BoundaryWorkspaceSpace { workspace, space }
}

fn space_activation(space: u64, at: workspaces::Activation<u64>) -> abi::BoundarySpaceActivation {
    abi::BoundarySpaceActivation {
        space,
        activation: activation(at),
    }
}

fn from_catalog(catalog: &abi::RocListWith<abi::BoundaryWorkspaceSpace, false>) -> Vec<(u64, u64)> {
    catalog.as_slice().iter().map(|entry| (entry.workspace, entry.space)).collect()
}

fn from_activations(
    activations: &abi::RocListWith<abi::BoundarySpaceActivation, false>,
) -> Vec<(u64, workspaces::Activation<u64>)> {
    activations
        .as_slice()
        .iter()
        .map(|entry| (entry.space, from_activation(entry.activation)))
        .collect()
}

use rift_core::workspaces::plain::{ClaimFlags, Claimant, FocusFollowsMouse};

fn ownership(ownership: Ownership<u64, Claimant>) -> abi::BoundaryOwnership {
    use abi::{BoundaryDestinationTag as Destination_, BoundaryOwnershipTag as Tag};
    match ownership {
        Ownership::Held => union!(BoundaryOwnership, BoundaryOwnershipPayload.held, Tag::Held),
        Ownership::Assigned(workspace) => {
            union!(
                BoundaryOwnership,
                BoundaryOwnershipPayload.assigned(workspace),
                Tag::Assigned
            )
        }
        Ownership::Claimed { owner, destination } => union!(
            BoundaryOwnership,
            BoundaryOwnershipPayload.claimed(abi::BoundaryClaim {
                owner: owner.owner,
                flags: abi::BoundaryClaimFlags {
                    focus_follows_mouse: match owner.flags.focus_follows_mouse {
                        FocusFollowsMouse::Followed => abi::BoundaryFocusFollowsMouse::Followed,
                        FocusFollowsMouse::Suppressed => abi::BoundaryFocusFollowsMouse::Suppressed,
                    },
                },
                original: owner.original,
                destination: match destination {
                    Destination::Nowhere => union!(
                        BoundaryDestination,
                        BoundaryDestinationPayload.nowhere,
                        Destination_::Nowhere
                    ),
                    Destination::Returns(workspace) => union!(
                        BoundaryDestination,
                        BoundaryDestinationPayload.returns(workspace),
                        Destination_::Returns
                    ),
                },
            }),
            Tag::Claimed
        ),
    }
}

fn from_ownership(ownership: abi::BoundaryOwnership) -> Ownership<u64, Claimant> {
    use abi::{BoundaryDestinationTag as Destination_, BoundaryOwnershipTag as Tag};
    match ownership.tag {
        Tag::Held => Ownership::Held,
        // SAFETY: the tag says the payload is the workspace.
        Tag::Assigned => {
            Ownership::Assigned(unsafe { *ownership.borrow_payload_assigned_unchecked() })
        }
        Tag::Claimed => {
            // SAFETY: the tag says the payload is the claim.
            let claim = unsafe { ownership.borrow_payload_claimed_unchecked() };
            Ownership::Claimed {
                owner: Claimant {
                    owner: claim.owner,
                    flags: ClaimFlags {
                        focus_follows_mouse: match claim.flags.focus_follows_mouse {
                            abi::BoundaryFocusFollowsMouse::Followed => FocusFollowsMouse::Followed,
                            abi::BoundaryFocusFollowsMouse::Suppressed => {
                                FocusFollowsMouse::Suppressed
                            }
                        },
                    },
                    original: claim.original,
                },
                destination: match claim.destination.tag {
                    Destination_::Nowhere => Destination::Nowhere,
                    // SAFETY: the tag says the payload is the workspace.
                    Destination_::Returns => Destination::Returns(unsafe {
                        *claim.destination.borrow_payload_returns_unchecked()
                    }),
                },
            }
        }
    }
}

fn owned(window: u64, owned: Owned<u64, Claimant>) -> abi::BoundaryOwnershipEvent {
    use abi::BoundaryOwnedTag as Tag;
    abi::BoundaryOwnershipEvent {
        window,
        owned: match owned {
            Owned::Unowned => union!(BoundaryOwned, BoundaryOwnedPayload.unowned, Tag::Unowned),
            Owned::Owner(owner) => union!(
                BoundaryOwned,
                BoundaryOwnedPayload.owner(ownership(owner)),
                Tag::Owner
            ),
        },
    }
}

fn from_owners(
    owners: &abi::RocListWith<abi::BoundaryWindowOwnership, false>,
) -> Vec<(u64, Ownership<u64, Claimant>)> {
    owners
        .as_slice()
        .iter()
        .map(|entry| (entry.window, from_ownership(entry.ownership)))
        .collect()
}

pub(crate) fn joined_question(
    question: workspaces::plain::WorkspacesQuestion,
    host: &abi::RocHost,
) -> abi::BoundaryJoinedQuestion {
    let events: Vec<_> = question.events.into_iter().map(joined_event).collect();
    abi::BoundaryJoinedQuestion { events: list(&events, host) }
}

/// A change to the joined workspaces.
pub(crate) fn joined_event(
    event: WorkspacesEvent<u64, u64, u64, Claimant>,
) -> abi::BoundaryJoinedEvent {
    use abi::{BoundaryJoinedEventTag as Tag, BoundarySuccessorTag as Heir};
    match event {
        WorkspacesEvent::Created { workspace, space } => union!(
            BoundaryJoinedEvent,
            BoundaryJoinedEventPayload.created(workspace_space(workspace, space)),
            Tag::Created
        ),
        WorkspacesEvent::Removed { workspace, successor } => union!(
            BoundaryJoinedEvent,
            BoundaryJoinedEventPayload.removed(abi::BoundaryWorkspaceRemoved {
                workspace,
                successor: match successor {
                    Successor::Unassign => union!(
                        BoundarySuccessor,
                        BoundarySuccessorPayload.unassign,
                        Heir::Unassign
                    ),
                    Successor::Into(heir) => union!(
                        BoundarySuccessor,
                        BoundarySuccessorPayload.into(heir),
                        Heir::Into
                    ),
                },
            }),
            Tag::Removed
        ),
        WorkspacesEvent::Moved { workspace, space } => union!(
            BoundaryJoinedEvent,
            BoundaryJoinedEventPayload.moved(workspace_space(workspace, space)),
            Tag::Moved
        ),
        WorkspacesEvent::Activated { space, activation: at } => union!(
            BoundaryJoinedEvent,
            BoundaryJoinedEventPayload.activated(space_activation(space, at)),
            Tag::Activated
        ),
        WorkspacesEvent::Cleared(space) => {
            union!(
                BoundaryJoinedEvent,
                BoundaryJoinedEventPayload.cleared(space),
                Tag::Cleared
            )
        }
        WorkspacesEvent::Owned { window, owned: who } => {
            union!(
                BoundaryJoinedEvent,
                BoundaryJoinedEventPayload.owned(owned(window, who)),
                Tag::Owned
            )
        }
    }
}

/// The joined workspaces Roc answered, read out and released.
pub(crate) fn from_joined(
    answer: abi::BoundaryJoinedView,
    host: &abi::RocHost,
) -> crate::workspaces::JoinedView {
    let view = crate::workspaces::JoinedView {
        outcomes: answer.outcomes.as_slice().iter().map(from_outcome).collect(),
        catalog: from_catalog(&answer.catalog),
        activations: from_activations(&answer.activations),
        owners: from_owners(&answer.owners),
        windows: answer
            .windows
            .as_slice()
            .iter()
            .map(|entry| entry.windows.as_slice().to_vec())
            .collect(),
    };
    // SAFETY: the view is Roc's answer, owned here and read in full above.
    unsafe { answer.decref(host) };
    view
}

/// What an event met: accepted, or why the workspaces refused it.
fn from_outcome(outcome: &abi::BoundaryOutcome) -> Result<(), Refused<u64, u64>> {
    use abi::BoundaryOutcomeTag as Tag;
    match outcome.tag {
        Tag::Accepted => Ok(()),
        // SAFETY (each arm): the tag says what the payload is.
        Tag::Exists => Err(Refused::Exists(unsafe {
            *outcome.borrow_payload_exists_unchecked()
        })),
        Tag::Unknown => Err(Refused::Unknown(unsafe {
            *outcome.borrow_payload_unknown_unchecked()
        })),
        Tag::OwnSuccessor => Err(Refused::OwnSuccessor(unsafe {
            *outcome.borrow_payload_own_successor_unchecked()
        })),
        Tag::Shown => {
            let at = unsafe { outcome.borrow_payload_shown_unchecked() };
            Err(Refused::Shown {
                workspace: at.workspace,
                space: at.space,
            })
        }
    }
}

// The core's events and effects.

fn button(button: Button) -> abi::BoundaryButton {
    match button {
        Button::Left => abi::BoundaryButton::Left,
        Button::Right => abi::BoundaryButton::Right,
    }
}

fn desires(
    desires: Vec<Desire<Plain>>,
    host: &abi::RocHost,
) -> abi::RocListWith<abi::BoundaryDesire, false> {
    let desires: Vec<_> = desires
        .into_iter()
        .map(|desire| abi::BoundaryDesire {
            window: window(desire.window),
            frame: rect(desire.frame),
            request: desire.request.0,
        })
        .collect();
    list(&desires, host)
}

fn sighting(id: WindowId, frame: Rect) -> abi::BoundarySighting {
    abi::BoundarySighting {
        window: window(id),
        frame: rect(frame),
    }
}

fn stamp(now: Nanos, request: RequestId) -> abi::BoundaryStamp {
    abi::BoundaryStamp { now: now.0, request: request.0 }
}

pub(crate) fn event(event: Event<Plain>, host: &abi::RocHost) -> abi::BoundaryEvent {
    use abi::{
        BoundaryAsOfTag as AsOf, BoundaryEventTag as Tag, BoundaryPressOnTag as On,
        BoundaryReleaseOfTag as Up, BoundaryStoppedAtTag as At,
    };
    match event {
        Event::Press { window: on, button: pressed } => union!(
            BoundaryEvent,
            BoundaryEventPayload.press(abi::BoundaryPress {
                on: match on {
                    Some(id) => union!(
                        BoundaryPressOn,
                        BoundaryPressOnPayload.on_window(window(id)),
                        On::OnWindow
                    ),
                    None =>
                        union!(BoundaryPressOn, BoundaryPressOnPayload.on_nothing, On::OnNothing),
                },
                button: button(pressed),
            }),
            Tag::Press
        ),
        Event::Drag { window: id } => {
            union!(BoundaryEvent, BoundaryEventPayload.drag(window(id)), Tag::Drag)
        }
        Event::Release(up) => union!(
            BoundaryEvent,
            BoundaryEventPayload.released(match up {
                Release::Button(released) => union!(
                    BoundaryReleaseOf,
                    BoundaryReleaseOfPayload.button_up(button(released)),
                    Up::ButtonUp
                ),
                Release::Missed =>
                    union!(BoundaryReleaseOf, BoundaryReleaseOfPayload.missed, Up::Missed),
            }),
            Tag::Released
        ),
        Event::LetGo => union!(BoundaryEvent, BoundaryEventPayload.let_go, Tag::LetGo),
        Event::Gone { window: id } => {
            union!(BoundaryEvent, BoundaryEventPayload.gone(window(id)), Tag::Gone)
        }
        Event::Rekeyed { from, to } => union!(
            BoundaryEvent,
            BoundaryEventPayload.rekeyed(abi::BoundaryRekey {
                from: window(from),
                to: window(to)
            }),
            Tag::Rekeyed
        ),
        Event::Seen { window: id, frame } => {
            union!(
                BoundaryEvent,
                BoundaryEventPayload.seen(sighting(id, frame)),
                Tag::Seen
            )
        }
        Event::Ceded { window: id } => {
            union!(BoundaryEvent, BoundaryEventPayload.ceded(window(id)), Tag::Ceded)
        }
        Event::Unseen { window: id } => union!(
            BoundaryEvent,
            BoundaryEventPayload.unseen(window(id)),
            Tag::Unseen
        ),
        Event::Distrust { windows } => {
            let windows: Vec<_> = windows.into_iter().map(window).collect();
            union!(
                BoundaryEvent,
                BoundaryEventPayload.distrust(list(&windows, host)),
                Tag::Distrust
            )
        }
        Event::Arrange { now, want, desires: wanted } => union!(
            BoundaryEvent,
            BoundaryEventPayload.arrange(abi::BoundaryArrangeOf {
                now: now.0,
                want: match want {
                    Want::Arrange => abi::BoundaryWant::Arrange,
                    Want::ArrangePositions => abi::BoundaryWant::ArrangePositions,
                },
                desires: desires(wanted, host),
            }),
            Tag::Arrange
        ),
        Event::Animate { now, desires: wanted } => union!(
            BoundaryEvent,
            BoundaryEventPayload.animate(abi::BoundaryAnimateOf {
                now: now.0,
                desires: desires(wanted, host)
            }),
            Tag::Animate
        ),
        Event::AnimationEnded { now, request } => union!(
            BoundaryEvent,
            BoundaryEventPayload.animation_ended(stamp(now, request)),
            Tag::AnimationEnded
        ),
        Event::AnimationStopped { window: id, request, at } => union!(
            BoundaryEvent,
            BoundaryEventPayload.animation_stopped(abi::BoundaryAnimationStop {
                window: window(id),
                request: request.0,
                at: match at {
                    Some(frame) => union!(
                        BoundaryStoppedAt,
                        BoundaryStoppedAtPayload.stopped_at(rect(frame)),
                        At::StoppedAt
                    ),
                    None => union!(
                        BoundaryStoppedAt,
                        BoundaryStoppedAtPayload.not_published,
                        At::NotPublished
                    ),
                },
            }),
            Tag::AnimationStopped
        ),
        Event::Follow { request, window: id, frame } => union!(
            BoundaryEvent,
            BoundaryEventPayload.follow(abi::BoundaryFollowOf {
                request: request.0,
                window: window(id),
                frame: rect(frame)
            }),
            Tag::Follow
        ),
        Event::Observe {
            now,
            request,
            window: id,
            frame,
            as_of,
        } => union!(
            BoundaryEvent,
            BoundaryEventPayload.observe(abi::BoundaryObservation {
                now: now.0,
                request: request.0,
                window: window(id),
                frame: rect(frame),
                as_of: match as_of {
                    Some(stamped) =>
                        union!(BoundaryAsOf, BoundaryAsOfPayload.as_of(stamped.0), AsOf::AsOf),
                    None => union!(BoundaryAsOf, BoundaryAsOfPayload.unstamped, AsOf::Unstamped),
                },
            }),
            Tag::Observe
        ),
        Event::Tick { now, request } => union!(
            BoundaryEvent,
            BoundaryEventPayload.tick(stamp(now, request)),
            Tag::Tick
        ),
        Event::Forget { window: id } => union!(
            BoundaryEvent,
            BoundaryEventPayload.forget(window(id)),
            Tag::Forget
        ),
        Event::SentOutside { window: id, request } => union!(
            BoundaryEvent,
            BoundaryEventPayload.sent_outside(abi::BoundarySentOutsideOf {
                window: window(id),
                request: request.0
            }),
            Tag::SentOutside
        ),
        Event::WriteFailed {
            now,
            request,
            window: id,
            failed,
        } => union!(
            BoundaryEvent,
            BoundaryEventPayload.write_failed(abi::BoundaryWriteFailure {
                now: now.0,
                request: request.0,
                window: window(id),
                failed: failed.0
            }),
            Tag::WriteFailed
        ),
    }
}

fn from_write(write: &abi::BoundaryWriteEffect) -> Write<Plain> {
    Write::foreign(
        from_window(write.window),
        from_rect(write.frame),
        RequestId(write.request),
    )
}

fn from_effect(effect: &abi::BoundaryEffect) -> Effect<Plain> {
    use abi::BoundaryEffectTag as Tag;
    // SAFETY (each arm): the tag says what the payload is.
    unsafe {
        match effect.tag {
            Tag::Write => Effect::Write(from_write(effect.borrow_payload_write_unchecked())),
            Tag::Animate => {
                let animate = effect.borrow_payload_animate_unchecked();
                Effect::Animate {
                    window: from_window(animate.window),
                    from: from_rect(animate.from),
                    to: from_rect(animate.to),
                    request: RequestId(animate.request),
                }
            }
            Tag::Pending => {
                let pending = effect.borrow_payload_pending_unchecked();
                Effect::Pending {
                    window: from_window(pending.window),
                    target: match pending.target.tag {
                        abi::BoundaryPendingTargetTag::Target => {
                            Some(from_rect(*pending.target.borrow_payload_target_unchecked()))
                        }
                        abi::BoundaryPendingTargetTag::NoTarget => None,
                    },
                }
            }
            Tag::Plan => {
                let plan = effect.borrow_payload_plan_unchecked();
                Effect::Plan(Plan::foreign(
                    RequestId(plan.request),
                    plan.positions.as_slice().iter().map(from_write).collect(),
                    plan.frames.as_slice().iter().map(from_write).collect(),
                ))
            }
            Tag::Queued => Effect::Queued {
                window: from_window(*effect.borrow_payload_queued_unchecked()),
            },
            Tag::Seen => {
                let seen = effect.borrow_payload_seen_unchecked();
                Effect::Seen {
                    window: from_window(seen.window),
                    frame: from_rect(seen.frame),
                }
            }
            Tag::Replied => {
                let replied = effect.borrow_payload_replied_unchecked();
                Effect::Replied {
                    window: from_window(replied.window),
                    reply: match replied.reply {
                        abi::BoundaryReply::NoNews => Reply::NoNews,
                        abi::BoundaryReply::InTransit => Reply::InTransit,
                        abi::BoundaryReply::Acked => Reply::Answered(Answer::Acked),
                        abi::BoundaryReply::Refused => Reply::Answered(Answer::Refused),
                        abi::BoundaryReply::Foreign => Reply::Foreign,
                    },
                }
            }
            Tag::Note => {
                let noted = effect.borrow_payload_note_unchecked();
                Effect::Note {
                    window: from_window(noted.window),
                    note: match noted.note {
                        abi::BoundaryNoteKind::Discarded => Note::Discarded,
                        abi::BoundaryNoteKind::TimedOut => Note::TimedOut,
                        abi::BoundaryNoteKind::GaveUp => Note::GaveUp,
                    },
                }
            }
            Tag::Timer => {
                let armed = effect.borrow_payload_timer_unchecked();
                Effect::Timer(match armed.tag {
                    abi::BoundaryArmedTag::Armed => {
                        Some(Nanos(*armed.borrow_payload_armed_unchecked()))
                    }
                    abi::BoundaryArmedTag::Disarmed => None,
                })
            }
            Tag::Grip => Effect::Grip {
                window: from_window(*effect.borrow_payload_grip_unchecked()),
            },
            Tag::DragBegan => Effect::DragBegan {
                window: from_window(*effect.borrow_payload_drag_began_unchecked()),
            },
            Tag::Ungrip => {
                let ungrip = effect.borrow_payload_ungrip_unchecked();
                Effect::Ungrip {
                    window: from_window(ungrip.window),
                    why: match ungrip.why {
                        abi::BoundaryUngripWhy::Released => Ungrip::Released,
                        abi::BoundaryUngripWhy::LetGo => Ungrip::LetGo,
                        abi::BoundaryUngripWhy::Gone => Ungrip::Gone,
                    },
                    changed: ungrip.changed.as_slice().iter().copied().map(from_window).collect(),
                }
            }
            Tag::Regrip => {
                let regrip = effect.borrow_payload_regrip_unchecked();
                Effect::Regrip {
                    from: from_window(regrip.from),
                    to: from_window(regrip.to),
                }
            }
        }
    }
}

/// The effects Roc decided, read out and released.
pub(crate) fn effects(
    effects: abi::RocList<abi::BoundaryEffect>,
    host: &abi::RocHost,
) -> Vec<Effect<Plain>> {
    let decided = effects.as_slice().iter().map(from_effect).collect();
    // SAFETY: the list is Roc's answer, owned here and read in full above.
    unsafe { abi::decref_list_of_boundary_effect(effects, host) };
    decided
}

// ---------------------------------------------------------------------------------------------
// The machines kept between events. Each machine's box is a type of its own (the glue types
// every box alike), which only that machine's entries make and take; a step takes the box by
// value and gives back its successor, so a box that has been stepped cannot be stepped again.

/// Steps the machine in `slot` by value, putting its successor back: `step` takes the box and
/// returns the next one with the answer. A panic while the slot is moved out aborts, so the
/// moved-out box is never seen (nor dropped twice).
pub(crate) fn in_place<B, A>(slot: &mut B, step: impl FnOnce(B) -> (B, A)) -> A {
    struct AbortOnUnwind;
    impl Drop for AbortOnUnwind {
        fn drop(&mut self) { std::process::abort() }
    }
    let guard = AbortOnUnwind;
    // SAFETY: the box is read out of the slot, stepped, and its successor written back before
    // anything else can reach the slot; a panic in between aborts (the guard), so the slot is
    // never used, nor dropped, while it is moved out.
    let (next, answer) = step(unsafe { std::ptr::read(slot) });
    unsafe { std::ptr::write(slot, next) };
    std::mem::forget(guard);
    answer
}

/// The lifecycle's box, which only Roc can read or free; only the `roc_lifecycle_*` entries
/// make and take one. Dropping it releases the lifecycle.
pub(crate) struct LifecycleBox(abi::RocBox);

impl LifecycleBox {
    pub(crate) fn empty() -> LifecycleBox { LifecycleBox(unsafe { abi::roc_lifecycle_init() }) }

    /// The raw box, handed to Roc: this owner is gone, so its drop does not release it.
    fn into_raw(self) -> abi::RocBox {
        let raw = self.0;
        std::mem::forget(self);
        raw
    }

    /// A reading of `window`'s facts: the lifecycle after it, and the change in its admission.
    pub(crate) fn read(self, window: u64, reading: Reading) -> (LifecycleBox, Option<Change>) {
        let event = abi::BoundaryLifecycleEvent {
            window,
            reading: self::reading(reading),
        };
        // SAFETY: the box is this lifecycle's, given to the step; its successor comes back.
        let stepped = unsafe { abi::roc_lifecycle_step(self.into_raw(), event) };
        (LifecycleBox(stepped.model), from_change(stepped.change))
    }

    /// `window`'s AX element invalidated: whether it had AX state to invalidate.
    pub(crate) fn invalidate(self, window: u64) -> (LifecycleBox, crate::Invalidation) {
        // SAFETY: as in `read`.
        let stepped = unsafe { abi::roc_lifecycle_invalidate(self.into_raw(), window) };
        let invalidation = match stepped.invalidation {
            abi::BoundaryInvalidation::Invalidated => crate::Invalidation::Invalidated,
            abi::BoundaryInvalidation::NoAxState => crate::Invalidation::NoAxState,
        };
        (LifecycleBox(stepped.model), invalidation)
    }

    /// A positive AX observation of `window`: whether it had been invalidated.
    pub(crate) fn revalidate(self, window: u64) -> (LifecycleBox, crate::Revalidation) {
        // SAFETY: as in `read`.
        let stepped = unsafe { abi::roc_lifecycle_revalidate(self.into_raw(), window) };
        let revalidation = match stepped.revalidation {
            abi::BoundaryRevalidation::Revalidated => crate::Revalidation::Revalidated,
            abi::BoundaryRevalidation::NotInvalidated => crate::Revalidation::NotInvalidated,
        };
        (LifecycleBox(stepped.model), revalidation)
    }

    /// Every window with AX state and its presence, in window order.
    pub(crate) fn view(self, host: &abi::RocHost) -> (LifecycleBox, Vec<(u64, Presence)>) {
        // SAFETY: as in `read`; the list is Roc's answer, owned here, read in full, released.
        let viewed = unsafe { abi::roc_lifecycle_view(self.into_raw()) };
        let mut windows: Vec<(u64, Presence)> = viewed
            .windows
            .as_slice()
            .iter()
            .map(|entry| (entry.window, from_present(entry.presence)))
            .collect();
        unsafe { viewed.windows.decref(host) };
        windows.sort_by_key(|&(window, _)| window);
        (LifecycleBox(viewed.model), windows)
    }
}

impl Drop for LifecycleBox {
    fn drop(&mut self) {
        // SAFETY: the box is this lifecycle's and is never used again.
        unsafe { abi::roc_lifecycle_release(self.0) }
    }
}

fn from_present(presence: abi::BoundaryPresent) -> Presence {
    use abi::BoundaryPresentTag as Tag;
    match presence.tag {
        // SAFETY: the tag says the payload is the facts.
        Tag::Live => Presence::Live(from_admission_facts(unsafe {
            *presence.borrow_payload_live_unchecked()
        })),
        // SAFETY: the tag says the payload is the facts.
        Tag::Invalidated => Presence::Invalidated(from_admission_facts(unsafe {
            *presence.borrow_payload_invalidated_unchecked()
        })),
    }
}

// ---------------------------------------------------------------------------------------------
// The workspaces joined, kept between events.

/// The joined workspaces' box, which only Roc can read or free; only the `roc_workspaces_*`
/// entries make and take one. Dropping it releases the workspaces.
pub(crate) struct WorkspacesBox(abi::RocBox);

impl WorkspacesBox {
    pub(crate) fn empty() -> WorkspacesBox { WorkspacesBox(unsafe { abi::roc_workspaces_init() }) }

    /// The raw box, handed to Roc (see [`LifecycleBox::into_raw`]).
    fn into_raw(self) -> abi::RocBox {
        let raw = self.0;
        std::mem::forget(self);
        raw
    }

    /// A change to the workspaces: the workspaces after it (unchanged if refused), and what the
    /// event met.
    pub(crate) fn step(
        self,
        event: WorkspacesEvent<u64, u64, u64, Claimant>,
    ) -> (WorkspacesBox, Result<(), Refused<u64, u64>>) {
        // SAFETY: the box is these workspaces', given to the step; its successor comes back.
        let stepped = unsafe { abi::roc_workspaces_step(self.into_raw(), joined_event(event)) };
        (WorkspacesBox(stepped.model), from_outcome(&stepped.outcome))
    }

    /// The catalog, every Space's activation, every owned window and each catalogued
    /// workspace's assigned windows.
    pub(crate) fn view(
        self,
        host: &abi::RocHost,
    ) -> (WorkspacesBox, crate::workspaces::WorkspacesView) {
        // SAFETY: as in `step`; the lists are Roc's answer, owned here, read in full, released.
        let viewed = unsafe { abi::roc_workspaces_view(self.into_raw()) };
        let view = crate::workspaces::WorkspacesView {
            catalog: from_catalog(&viewed.catalog),
            activations: from_activations(&viewed.activations),
            owners: from_owners(&viewed.owners),
            windows: viewed
                .windows
                .as_slice()
                .iter()
                .map(|entry| entry.windows.as_slice().to_vec())
                .collect(),
        };
        unsafe {
            viewed.catalog.decref(host);
            viewed.activations.decref(host);
            viewed.owners.decref(host);
            abi::decref_list_of_boundary_workspace_windows(viewed.windows, host);
        }
        (WorkspacesBox(viewed.model), view)
    }
}

impl Drop for WorkspacesBox {
    fn drop(&mut self) {
        // SAFETY: the box is these workspaces' and is never used again.
        unsafe { abi::roc_workspaces_release(self.0) }
    }
}

// ---------------------------------------------------------------------------------------------
// Which window is the main one, kept between events.

fn initiator(initiator: Initiator) -> abi::BoundaryInitiator {
    match initiator {
        Initiator::Rift => abi::BoundaryInitiator::Rift,
        Initiator::Other => abi::BoundaryInitiator::Other,
    }
}

fn from_initiator(initiator: abi::BoundaryInitiator) -> Initiator {
    match initiator {
        abi::BoundaryInitiator::Rift => Initiator::Rift,
        abi::BoundaryInitiator::Other => Initiator::Other,
    }
}

fn app_standing(standing: MainStanding) -> abi::BoundaryAppStanding {
    match standing {
        MainStanding::Frontmost => abi::BoundaryAppStanding::Frontmost,
        MainStanding::Background => abi::BoundaryAppStanding::Background,
    }
}

fn from_app_standing(standing: abi::BoundaryAppStanding) -> MainStanding {
    match standing {
        abi::BoundaryAppStanding::Frontmost => MainStanding::Frontmost,
        abi::BoundaryAppStanding::Background => MainStanding::Background,
    }
}

fn main_window_event(event: MainWindowEvent<u64, u64>) -> abi::BoundaryMainWindowEvent {
    use abi::BoundaryMainWindowEventTag as Tag;
    match event {
        MainWindowEvent::Launched { pid, standing, main_window } => union!(
            BoundaryMainWindowEvent,
            BoundaryMainWindowEventPayload.launched(abi::BoundaryMainLaunch {
                pid,
                standing: app_standing(standing),
                main_window: maybe_word(main_window),
            }),
            Tag::Launched
        ),
        MainWindowEvent::Terminated(pid) => union!(
            BoundaryMainWindowEvent,
            BoundaryMainWindowEventPayload.terminated(pid),
            Tag::Terminated
        ),
        MainWindowEvent::Activated(pid, by) => union!(
            BoundaryMainWindowEvent,
            BoundaryMainWindowEventPayload
                .activated(abi::BoundaryMainActivation { pid, initiator: initiator(by) }),
            Tag::Activated
        ),
        MainWindowEvent::Deactivated(pid) => union!(
            BoundaryMainWindowEvent,
            BoundaryMainWindowEventPayload.deactivated(pid),
            Tag::Deactivated
        ),
        MainWindowEvent::GloballyActivated(pid) => union!(
            BoundaryMainWindowEvent,
            BoundaryMainWindowEventPayload.globally_activated(pid),
            Tag::GloballyActivated
        ),
        MainWindowEvent::GloballyDeactivated(pid) => union!(
            BoundaryMainWindowEvent,
            BoundaryMainWindowEventPayload.globally_deactivated(pid),
            Tag::GloballyDeactivated
        ),
        MainWindowEvent::MainWindowChanged(pid, window, by) => union!(
            BoundaryMainWindowEvent,
            BoundaryMainWindowEventPayload.main_window_changed(abi::BoundaryMainWindowChange {
                pid,
                window: maybe_word(window),
                initiator: initiator(by),
            }),
            Tag::MainWindowChanged
        ),
        MainWindowEvent::NativeFocused { pid, window } => union!(
            BoundaryMainWindowEvent,
            BoundaryMainWindowEventPayload
                .native_focused(abi::BoundaryFocusedWindow { pid, window }),
            Tag::NativeFocused
        ),
        MainWindowEvent::Replaced { pid, window, replacement } => union!(
            BoundaryMainWindowEvent,
            BoundaryMainWindowEventPayload.replaced(abi::BoundaryMainReplacement {
                pid,
                window,
                replacement: maybe_word(replacement),
            }),
            Tag::Replaced
        ),
    }
}

fn from_native_focus(focus: abi::BoundaryNativeFocus) -> NativeFocus<u64, u64> {
    use abi::{BoundaryNativeFocusTag as Tag, BoundaryReportedFocusTag as Reported};
    match focus.tag {
        Tag::NotYetReported => NativeFocus::NotYetReported,
        Tag::Reported => {
            // SAFETY: the tag says the payload is the reported focus.
            let reported = unsafe { *focus.borrow_payload_reported_unchecked() };
            NativeFocus::Reported(match reported.tag {
                Reported::NoWindow => None,
                Reported::Window => {
                    // SAFETY: the tag says the payload is the window and its app.
                    let focused = unsafe { *reported.borrow_payload_window_unchecked() };
                    Some((focused.pid, focused.window))
                }
            })
        }
    }
}

/// The main windows' box, which only Roc can read or free; only the `roc_main_window_*`
/// entries make and take one. Dropping it releases the main windows.
pub(crate) struct MainWindowsBox(abi::RocBox);

impl MainWindowsBox {
    pub(crate) fn empty() -> MainWindowsBox {
        MainWindowsBox(unsafe { abi::roc_main_window_init() })
    }

    /// The raw box, handed to Roc (see [`LifecycleBox::into_raw`]).
    fn into_raw(self) -> abi::RocBox {
        let raw = self.0;
        std::mem::forget(self);
        raw
    }

    /// An observed event: the main windows after it, and the window to focus, if it makes one
    /// known.
    pub(crate) fn step(self, event: MainWindowEvent<u64, u64>) -> (MainWindowsBox, Option<u64>) {
        // SAFETY: the box is these main windows', given to the step; its successor comes back.
        let stepped =
            unsafe { abi::roc_main_window_step(self.into_raw(), main_window_event(event)) };
        (MainWindowsBox(stepped.model), from_maybe_word(stepped.focus))
    }

    /// The main window, the native focus, every app in pid order and the globally frontmost
    /// app.
    pub(crate) fn view(
        self,
        host: &abi::RocHost,
    ) -> (MainWindowsBox, crate::main_window::MainWindowsView) {
        // SAFETY: as in `step`; the list is Roc's answer, owned here, read in full, released.
        let viewed = unsafe { abi::roc_main_window_view(self.into_raw()) };
        let apps = viewed
            .apps
            .as_slice()
            .iter()
            .map(|app| {
                (app.pid, App {
                    standing: from_app_standing(app.standing),
                    last_activation: from_initiator(app.last_activation),
                    main_window: from_maybe_word(app.main_window),
                })
            })
            .collect();
        unsafe { viewed.apps.decref(host) };
        let view = crate::main_window::MainWindowsView {
            main_window: from_maybe_word(viewed.main_window),
            native_focus: from_native_focus(viewed.native_focus),
            apps,
            global_frontmost: from_maybe_word(viewed.global_frontmost),
        };
        (MainWindowsBox(viewed.model), view)
    }
}

impl Drop for MainWindowsBox {
    fn drop(&mut self) {
        // SAFETY: the box is these main windows' and is never used again.
        unsafe { abi::roc_main_window_release(self.0) }
    }
}

// ---------------------------------------------------------------------------------------------
// The mode table.

fn pointer_now(pointer: rift_core::Pointer<u64>) -> abi::BoundaryPointerNow {
    use abi::BoundaryPointerNowTag as Tag;
    match pointer {
        rift_core::Pointer::Idle => {
            union!(BoundaryPointerNow, BoundaryPointerNowPayload.idle, Tag::Idle)
        }
        rift_core::Pointer::Pressing(window) => union!(
            BoundaryPointerNow,
            BoundaryPointerNowPayload.pressing(window),
            Tag::Pressing
        ),
        rift_core::Pointer::Dragging(window) => union!(
            BoundaryPointerNow,
            BoundaryPointerNowPayload.dragging(window),
            Tag::Dragging
        ),
    }
}

fn from_pointer_now(pointer: abi::BoundaryPointerNow) -> rift_core::Pointer<u64> {
    use abi::BoundaryPointerNowTag as Tag;
    match pointer.tag {
        Tag::Idle => rift_core::Pointer::Idle,
        // SAFETY: the tag says the payload is the window.
        Tag::Pressing => {
            rift_core::Pointer::Pressing(unsafe { *pointer.borrow_payload_pressing_unchecked() })
        }
        // SAFETY: the tag says the payload is the window.
        Tag::Dragging => {
            rift_core::Pointer::Dragging(unsafe { *pointer.borrow_payload_dragging_unchecked() })
        }
    }
}

fn initiative(initiative: rift_core::Initiative) -> abi::BoundaryInitiative {
    use abi::BoundaryInitiativeTag as Tag;
    use rift_core::{Initiative, InputClass};
    match initiative {
        Initiative::UserInput(class) => union!(
            BoundaryInitiative,
            BoundaryInitiativePayload.user_input(match class {
                InputClass::Steering => abi::BoundaryInputClass::Steering,
                InputClass::Arranging => abi::BoundaryInputClass::Arranging,
                InputClass::Inert => abi::BoundaryInputClass::Inert,
                InputClass::Hover => abi::BoundaryInputClass::Hover,
            }),
            Tag::UserInput
        ),
        Initiative::Arrange => {
            union!(
                BoundaryInitiative,
                BoundaryInitiativePayload.arrange,
                Tag::Arrange
            )
        }
        Initiative::RetireWindows => union!(
            BoundaryInitiative,
            BoundaryInitiativePayload.retire_windows,
            Tag::RetireWindows
        ),
        Initiative::SwitchWorkspaceFocus => union!(
            BoundaryInitiative,
            BoundaryInitiativePayload.switch_workspace_focus,
            Tag::SwitchWorkspaceFocus
        ),
        Initiative::WarpFocus => {
            union!(
                BoundaryInitiative,
                BoundaryInitiativePayload.warp_focus,
                Tag::WarpFocus
            )
        }
        Initiative::FollowActivation => union!(
            BoundaryInitiative,
            BoundaryInitiativePayload.follow_activation,
            Tag::FollowActivation
        ),
    }
}

pub(crate) fn modes_question(
    modes: rift_core::modes::Modes<u64>,
    asked: rift_core::Initiative,
) -> abi::BoundaryModesQuestion {
    use rift_core::modes::{
        FocusClaim, Menu, MissionControl, Modal, UserSpaces, Viewport, Wake, WorkspaceSwitch,
    };
    abi::BoundaryModesQuestion {
        initiative: initiative(asked),
        pointer: pointer_now(modes.pointer),
        menu: match modes.menu {
            Menu::Closed => abi::BoundaryMenuNow::Closed,
            Menu::Open => abi::BoundaryMenuNow::Open,
        },
        mission_control: match modes.mission_control {
            MissionControl::Inactive => abi::BoundaryMissionControlNow::Inactive,
            MissionControl::Active => abi::BoundaryMissionControlNow::Active,
        },
        modal: match modes.modal {
            Modal::Clear => abi::BoundaryModalNow::Clear,
            Modal::Frontmost => abi::BoundaryModalNow::Frontmost,
        },
        viewport: match modes.viewport {
            Viewport::Free => abi::BoundaryViewportNow::Free,
            Viewport::Gesturing => abi::BoundaryViewportNow::Gesturing,
        },
        focus_claim: match modes.focus_claim {
            FocusClaim::Free => abi::BoundaryFocusClaimNow::Free,
            FocusClaim::PausesHover => abi::BoundaryFocusClaimNow::PausesHover,
        },
        user_spaces: match modes.user_spaces {
            UserSpaces::Shown => abi::BoundaryUserSpacesNow::Shown,
            UserSpaces::None => abi::BoundaryUserSpacesNow::None,
        },
        wake: match modes.wake {
            Wake::UserPresent => abi::BoundaryWakeNow::UserPresent,
            Wake::AwaitingInput => abi::BoundaryWakeNow::AwaitingInput,
        },
        workspace_switch: match modes.workspace_switch {
            WorkspaceSwitch::Idle => abi::BoundaryWorkspaceSwitchNow::Idle,
            WorkspaceSwitch::Switching => abi::BoundaryWorkspaceSwitchNow::Switching,
        },
        topology: topology_now(modes.topology),
    }
}

fn topology_now(topology: rift_core::modes::Topology) -> abi::BoundaryTopologyNow {
    use rift_core::modes::Topology;
    match topology {
        Topology::Settled => abi::BoundaryTopologyNow::Settled,
        Topology::Unsettled => abi::BoundaryTopologyNow::Unsettled,
    }
}

pub(crate) fn from_mode_verdict(
    verdict: abi::BoundaryModeVerdict,
) -> rift_core::modes::Verdict<u64> {
    use abi::{BoundaryModeVerdictTag as Tag, BoundaryRefusingAxisTag as AxisTag};
    use rift_core::modes::{Axis, Verdict};
    match verdict.tag {
        Tag::Admitted => Verdict::Admitted,
        Tag::Refused => {
            // SAFETY: the tag says the payload is the refusing axis.
            let axis = unsafe { *verdict.borrow_payload_refused_unchecked() };
            Verdict::Refused(match axis.tag {
                // SAFETY: the tag says the payload is the pointer.
                AxisTag::Pointer => Axis::Pointer(from_pointer_now(unsafe {
                    *axis.borrow_payload_pointer_unchecked()
                })),
                AxisTag::Menu => Axis::Menu,
                AxisTag::MissionControl => Axis::MissionControl,
                AxisTag::Modal => Axis::Modal,
                AxisTag::Viewport => Axis::Viewport,
                AxisTag::FocusClaim => Axis::FocusClaim,
                AxisTag::UserSpaces => Axis::UserSpaces,
                AxisTag::Wake => Axis::Wake,
                AxisTag::WorkspaceSwitch => Axis::WorkspaceSwitch,
                AxisTag::Topology => Axis::Topology,
            })
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The BSP layout system's trees.

fn insertion(insertion: InsertionPoint) -> abi::BoundaryBspInsertion {
    match insertion {
        InsertionPoint::NextToSelection => abi::BoundaryBspInsertion::NextToSelection,
        InsertionPoint::EndOfTree => abi::BoundaryBspInsertion::EndOfTree,
    }
}

fn from_insertion(insertion: abi::BoundaryBspInsertion) -> InsertionPoint {
    match insertion {
        abi::BoundaryBspInsertion::NextToSelection => InsertionPoint::NextToSelection,
        abi::BoundaryBspInsertion::EndOfTree => InsertionPoint::EndOfTree,
    }
}

fn aspect(ratio: Option<f64>) -> abi::BoundaryAspect {
    use abi::BoundaryAspectTag as Tag;
    match ratio {
        Some(ratio) => union!(BoundaryAspect, BoundaryAspectPayload.aspect(ratio), Tag::Aspect),
        None => union!(BoundaryAspect, BoundaryAspectPayload.free, Tag::Free),
    }
}

fn from_aspect(aspect: abi::BoundaryAspect) -> Option<f64> {
    match aspect.tag {
        // SAFETY: the tag says the payload is the ratio.
        abi::BoundaryAspectTag::Aspect => {
            Some(unsafe { *aspect.borrow_payload_aspect_unchecked() })
        }
        abi::BoundaryAspectTag::Free => None,
    }
}

fn tree_event(event: TreeEvent, host: &abi::RocHost) -> abi::BoundaryBspTreeEvent {
    use abi::BoundaryBspTreeEventTag as Tag;
    match event {
        TreeEvent::CreateLayout => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.create_layout,
            Tag::CreateLayout
        ),
        TreeEvent::CloneLayout(layout) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.clone_layout(layout.0),
            Tag::CloneLayout
        ),
        TreeEvent::RemoveLayout(layout) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.remove_layout(layout.0),
            Tag::RemoveLayout
        ),
        TreeEvent::Participation(window, participation) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.participation(abi::BoundaryBspParticipation {
                window,
                participation: match participation {
                    Participation::TakesPart => abi::BoundaryBspTakesPart::TakesPart,
                    Participation::LeftOut => abi::BoundaryBspTakesPart::LeftOut,
                },
            }),
            Tag::Participation
        ),
        TreeEvent::Insertion(point) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.insertion(insertion(point)),
            Tag::Insertion
        ),
        TreeEvent::AspectRatio(ratio) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.aspect_ratio(aspect(ratio)),
            Tag::AspectRatio
        ),
        TreeEvent::AddWindow(layout, window) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload
                .add_window(abi::BoundaryBspWindowIn { layout: layout.0, window }),
            Tag::AddWindow
        ),
        TreeEvent::RemoveWindow(window) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.remove_window(window),
            Tag::RemoveWindow
        ),
        TreeEvent::ReplaceWindow { from, to } => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.replace_window(abi::BoundaryBspReplacement { from, to }),
            Tag::ReplaceWindow
        ),
        TreeEvent::CloseApp(windows) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.close_app(list(&windows, host)),
            Tag::CloseApp
        ),
        TreeEvent::Edit(layout, edit) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.edit(abi::BoundaryBspEditIn {
                layout: layout.0,
                edit: tree_edit(edit),
            }),
            Tag::Edit
        ),
        TreeEvent::Focus(layout, focus) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.focus(abi::BoundaryBspFocusIn {
                layout: layout.0,
                focus: tree_focus(focus),
            }),
            Tag::Focus
        ),
        TreeEvent::ContainsLayout(layout) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.contains_layout(layout.0),
            Tag::ContainsLayout
        ),
        TreeEvent::WindowParticipates(window) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.window_participates(window),
            Tag::WindowParticipates
        ),
        TreeEvent::SelectedWindow(layout) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.selected_window(layout.0),
            Tag::SelectedWindow
        ),
        TreeEvent::WindowInDirection(layout, direction) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.window_in_direction(abi::BoundaryBspLayoutDirection {
                layout: layout.0,
                direction: tree_direction(direction),
            }),
            Tag::WindowInDirection
        ),
        TreeEvent::ContainsWindow(layout, window) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload
                .contains_window(abi::BoundaryBspWindowIn { layout: layout.0, window }),
            Tag::ContainsWindow
        ),
        TreeEvent::HasAnyFullscreenNode(layout) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.has_any_fullscreen_node(layout.0),
            Tag::HasAnyFullscreenNode
        ),
        TreeEvent::HasWindowsForApp(layout, windows) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.has_windows_for_app(abi::BoundaryBspAppIn {
                layout: layout.0,
                windows: list(&windows, host),
            }),
            Tag::HasWindowsForApp
        ),
        TreeEvent::Windows(layout, query) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.windows(abi::BoundaryBspWindowsQuery {
                layout: layout.0,
                query: tree_query(query),
            }),
            Tag::Windows
        ),
        TreeEvent::CanMoveWindow(layout, window, direction) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.can_move_window(abi::BoundaryBspMoveQuery {
                layout: layout.0,
                window,
                direction: tree_direction(direction),
            }),
            Tag::CanMoveWindow
        ),
        TreeEvent::WindowSlot(layout, window) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload
                .window_slot(abi::BoundaryBspWindowIn { layout: layout.0, window }),
            Tag::WindowSlot
        ),
        TreeEvent::ResizeWindow(layout, window, Resize { old, new, screen, tiling }) => union!(
            BoundaryBspTreeEvent,
            BoundaryBspTreeEventPayload.resize_window(abi::BoundaryBspNativeResize {
                layout: layout.0,
                window,
                old: rect(old),
                new: rect(new),
                screen: rect(screen),
                tiling: rect(tiling),
            }),
            Tag::ResizeWindow
        ),
    }
}

fn from_tree_answer(answer: abi::BoundaryBspTreeAnswer) -> TreeAnswer {
    use abi::BoundaryBspTreeAnswerTag as Tag;
    match answer.tag {
        // SAFETY (each arm): the tag says which payload it is.
        Tag::Layout => {
            TreeAnswer::Layout(LayoutKey(unsafe { *answer.borrow_payload_layout_unchecked() }))
        }
        Tag::Participated => {
            TreeAnswer::Participated(unsafe { *answer.borrow_payload_participated_unchecked() })
        }
        Tag::Done => TreeAnswer::Done,
        Tag::Edited => {
            let edited = unsafe { answer.borrow_payload_edited_unchecked() };
            TreeAnswer::Edited(match edited.tag {
                abi::BoundaryBspEditedTag::Unsupported => Edited::Unsupported,
                abi::BoundaryBspEditedTag::Unchanged => Edited::Unchanged,
                abi::BoundaryBspEditedTag::Changed => Edited::Changed,
                abi::BoundaryBspEditedTag::Raise => Edited::Raise(
                    unsafe { edited.borrow_payload_raise_unchecked() }.as_slice().to_vec(),
                ),
            })
        }
        Tag::Focused => {
            let focused = unsafe { answer.borrow_payload_focused_unchecked() };
            TreeAnswer::Focused(match focused.tag {
                abi::BoundaryBspFocusedTag::Selection => Focused::Selection {
                    handled: unsafe { *focused.borrow_payload_selection_unchecked() },
                },
                abi::BoundaryBspFocusedTag::Movement => {
                    let movement = unsafe { focused.borrow_payload_movement_unchecked() };
                    Focused::Movement {
                        window: from_maybe_word(movement.window),
                        raise: movement.raise.as_slice().to_vec(),
                    }
                }
            })
        }
        Tag::Verdict => TreeAnswer::Verdict(unsafe { *answer.borrow_payload_verdict_unchecked() }),
        Tag::Window => TreeAnswer::Window(from_maybe_word(unsafe {
            *answer.borrow_payload_window_unchecked()
        })),
        Tag::Windows => TreeAnswer::Windows(unsafe {
            answer.borrow_payload_windows_unchecked().as_slice().to_vec()
        }),
        Tag::WindowSlot => {
            let slot = unsafe { answer.borrow_payload_window_slot_unchecked() };
            TreeAnswer::WindowSlot(match slot.tag {
                abi::BoundaryBspSlotTag::NoSlot => None,
                abi::BoundaryBspSlotTag::Slot => Some(
                    unsafe { slot.borrow_payload_slot_unchecked() }
                        .as_slice()
                        .iter()
                        .map(|&side| side as usize)
                        .collect(),
                ),
            })
        }
    }
}

fn tree_direction(direction: TreeDirection) -> abi::BoundaryBspDirection {
    match direction {
        TreeDirection::Left => abi::BoundaryBspDirection::Left,
        TreeDirection::Right => abi::BoundaryBspDirection::Right,
        TreeDirection::Up => abi::BoundaryBspDirection::Up,
        TreeDirection::Down => abi::BoundaryBspDirection::Down,
    }
}

fn tree_orientation(orientation: Orientation) -> abi::BoundaryBspOrientation {
    match orientation {
        Orientation::Horizontal => abi::BoundaryBspOrientation::Horizontal,
        Orientation::Vertical => abi::BoundaryBspOrientation::Vertical,
    }
}

fn tree_edit(edit: Edit<u64>) -> abi::BoundaryBspEdit {
    use abi::BoundaryBspEditTag as Tag;
    match edit {
        Edit::Split(kind) => union!(
            BoundaryBspEdit,
            BoundaryBspEditPayload.split(match kind {
                SplitKind::Tiled(orientation) => union!(
                    BoundaryBspSplitKind,
                    BoundaryBspSplitKindPayload.tiled(tree_orientation(orientation)),
                    abi::BoundaryBspSplitKindTag::Tiled
                ),
                SplitKind::Stacked => union!(
                    BoundaryBspSplitKind,
                    BoundaryBspSplitKindPayload.stacked,
                    abi::BoundaryBspSplitKindTag::Stacked
                ),
            }),
            Tag::Split
        ),
        Edit::Fullscreen { within_gaps } => union!(
            BoundaryBspEdit,
            BoundaryBspEditPayload.fullscreen(if within_gaps {
                abi::BoundaryBspFullscreen::WithinGaps
            } else {
                abi::BoundaryBspFullscreen::Screen
            }),
            Tag::Fullscreen
        ),
        Edit::Resize { amount, along } => union!(
            BoundaryBspEdit,
            BoundaryBspEditPayload.resize(abi::BoundaryBspResize {
                amount,
                along: match along {
                    ResizeAlong::Horizontal => abi::BoundaryBspResizeAlong::Horizontal,
                    ResizeAlong::Vertical => abi::BoundaryBspResizeAlong::Vertical,
                    ResizeAlong::Smart => abi::BoundaryBspResizeAlong::Smart,
                },
            }),
            Tag::Resize
        ),
        Edit::Rebalance => {
            union!(BoundaryBspEdit, BoundaryBspEditPayload.rebalance, Tag::Rebalance)
        }
        Edit::ToggleOrientation => union!(
            BoundaryBspEdit,
            BoundaryBspEditPayload.toggle_orientation,
            Tag::ToggleOrientation
        ),
        Edit::Move(direction) => union!(
            BoundaryBspEdit,
            BoundaryBspEditPayload.moved(tree_direction(direction)),
            Tag::Moved
        ),
        Edit::Swap(a, b) => union!(
            BoundaryBspEdit,
            BoundaryBspEditPayload.swap(abi::BoundaryBspPair { a, b }),
            Tag::Swap
        ),
        Edit::Drop { source, target, action } => union!(
            BoundaryBspEdit,
            BoundaryBspEditPayload.drop(abi::BoundaryBspDrop {
                source,
                target,
                action: drop_action(action),
            }),
            Tag::Drop
        ),
        Edit::Join(direction) => union!(
            BoundaryBspEdit,
            BoundaryBspEditPayload.join(tree_direction(direction)),
            Tag::Join
        ),
        Edit::ConsumeOrExpel(direction) => union!(
            BoundaryBspEdit,
            BoundaryBspEditPayload.consume_or_expel(tree_direction(direction)),
            Tag::ConsumeOrExpel
        ),
        Edit::Unjoin => union!(BoundaryBspEdit, BoundaryBspEditPayload.unjoin, Tag::Unjoin),
    }
}

fn drop_action(action: DropAction) -> abi::BoundaryBspDropAction {
    use abi::BoundaryBspDropActionTag as Tag;
    match action {
        DropAction::Swap => {
            union!(
                BoundaryBspDropAction,
                BoundaryBspDropActionPayload.swap,
                Tag::Swap
            )
        }
        DropAction::Stack => {
            union!(
                BoundaryBspDropAction,
                BoundaryBspDropActionPayload.stack,
                Tag::Stack
            )
        }
        DropAction::Insert(direction) => union!(
            BoundaryBspDropAction,
            BoundaryBspDropActionPayload.insert(tree_direction(direction)),
            Tag::Insert
        ),
        DropAction::Move(direction) => union!(
            BoundaryBspDropAction,
            BoundaryBspDropActionPayload.moved(tree_direction(direction)),
            Tag::Moved
        ),
        DropAction::ConsumeOrExpel(direction) => union!(
            BoundaryBspDropAction,
            BoundaryBspDropActionPayload.consume_or_expel(tree_direction(direction)),
            Tag::ConsumeOrExpel
        ),
    }
}

fn tree_focus(focus: TreeFocus<u64>) -> abi::BoundaryBspFocus {
    use abi::BoundaryBspFocusTag as Tag;
    match focus {
        TreeFocus::Select(window) => {
            union!(
                BoundaryBspFocus,
                BoundaryBspFocusPayload.select(window),
                Tag::Select
            )
        }
        TreeFocus::Ascend => union!(BoundaryBspFocus, BoundaryBspFocusPayload.ascend, Tag::Ascend),
        TreeFocus::Descend => {
            union!(BoundaryBspFocus, BoundaryBspFocusPayload.descend, Tag::Descend)
        }
        TreeFocus::Direction(direction) => union!(
            BoundaryBspFocus,
            BoundaryBspFocusPayload.direction(tree_direction(direction)),
            Tag::Direction
        ),
    }
}

fn tree_query(query: Query<u64>) -> abi::BoundaryBspQuery {
    use abi::BoundaryBspQueryTag as Tag;
    match query {
        Query::All => union!(BoundaryBspQuery, BoundaryBspQueryPayload.all, Tag::All),
        Query::Visible => union!(BoundaryBspQuery, BoundaryBspQueryPayload.visible, Tag::Visible),
        Query::Selection => {
            union!(
                BoundaryBspQuery,
                BoundaryBspQueryPayload.selection,
                Tag::Selection
            )
        }
        Query::Stack(window) => {
            union!(
                BoundaryBspQuery,
                BoundaryBspQueryPayload.stack(window),
                Tag::Stack
            )
        }
    }
}

fn from_orientation(orientation: abi::BoundaryBspOrientation) -> Orientation {
    match orientation {
        abi::BoundaryBspOrientation::Horizontal => Orientation::Horizontal,
        abi::BoundaryBspOrientation::Vertical => Orientation::Vertical,
    }
}

fn from_side(side: abi::BoundaryBspSide) -> Side {
    match side {
        abi::BoundaryBspSide::First => Side::First,
        abi::BoundaryBspSide::Second => Side::Second,
    }
}

fn from_direction(direction: abi::BoundaryBspDirection) -> TreeDirection {
    match direction {
        abi::BoundaryBspDirection::Left => TreeDirection::Left,
        abi::BoundaryBspDirection::Right => TreeDirection::Right,
        abi::BoundaryBspDirection::Up => TreeDirection::Up,
        abi::BoundaryBspDirection::Down => TreeDirection::Down,
    }
}

fn from_fill(fill: abi::BoundaryFill) -> bsp::Fill {
    match fill {
        abi::BoundaryFill::Tiled => bsp::Fill::Tiled,
        abi::BoundaryFill::Screen => bsp::Fill::Screen,
        abi::BoundaryFill::WithinGaps => bsp::Fill::WithinGaps,
    }
}

fn from_members(members: &abi::BoundaryBspMembers) -> Members<u64> {
    use abi::BoundaryBspMembersTag as Tag;
    match members.tag {
        // SAFETY (each arm): the tag says which payload it is; the lists are only read.
        Tag::Window => Members::Window(unsafe { *members.borrow_payload_window_unchecked() }),
        Tag::Stack => {
            let tabs = unsafe { members.borrow_payload_stack_unchecked() };
            Members::Stack(Tabs {
                before: tabs.before.as_slice().to_vec(),
                top: tabs.top,
                after: tabs.after.as_slice().to_vec(),
            })
        }
    }
}

fn from_leaf(leaf: &abi::BoundaryBspTreeLeaf) -> Leaf<u64> {
    let content = match leaf.content.tag {
        abi::BoundaryBspContentTag::Empty => Content::Empty,
        abi::BoundaryBspContentTag::Held => {
            // SAFETY: the tag says the payload is the held windows.
            let held = unsafe { leaf.content.borrow_payload_held_unchecked() };
            Content::Held {
                members: from_members(&held.members),
                fill: from_fill(held.fill),
            }
        }
    };
    let preselect = match leaf.preselect.tag {
        abi::BoundaryBspPreselectTag::Unset => Preselect::Unset,
        abi::BoundaryBspPreselectTag::Toward => {
            // SAFETY: the tag says the payload is the direction.
            Preselect::Toward(from_direction(unsafe {
                *leaf.preselect.borrow_payload_toward_unchecked()
            }))
        }
    };
    Leaf { content, preselect }
}

fn from_tree_node(node: &abi::BoundaryBspTreeNode) -> TreeNode<NodeKey, u64> {
    let parent = match node.parent.tag {
        abi::BoundaryBspParentTag::Root => Parent::Root,
        // SAFETY: the tag says the payload is the parent's key.
        abi::BoundaryBspParentTag::Split => {
            Parent::Split(NodeKey(unsafe { *node.parent.borrow_payload_split_unchecked() }))
        }
    };
    let kind = match node.kind.tag {
        abi::BoundaryBspKindTag::Split => {
            // SAFETY: the tag says the payload is the split.
            let split = unsafe { node.kind.borrow_payload_split_unchecked() };
            Kind::Split(Split {
                orientation: from_orientation(split.orientation),
                ratio: split.ratio,
                children: [NodeKey(split.first), NodeKey(split.second)],
                memory: match split.memory.tag {
                    abi::BoundaryBspMemoryTag::Stops => Memory::Stops,
                    abi::BoundaryBspMemoryTag::Toward => {
                        // SAFETY: the tag says the payload is the side.
                        Memory::Toward(from_side(unsafe {
                            *split.memory.borrow_payload_toward_unchecked()
                        }))
                    }
                },
            })
        }
        // SAFETY: the tag says the payload is the leaf.
        abi::BoundaryBspKindTag::Leaf => {
            Kind::Leaf(from_leaf(unsafe { node.kind.borrow_payload_leaf_unchecked() }))
        }
    };
    TreeNode { parent, kind }
}

fn tree_side(side: Side) -> abi::BoundaryBspSide {
    match side {
        Side::First => abi::BoundaryBspSide::First,
        Side::Second => abi::BoundaryBspSide::Second,
    }
}

fn tree_members(members: &Members<u64>, host: &abi::RocHost) -> abi::BoundaryBspMembers {
    use abi::BoundaryBspMembersTag as Tag;
    match members {
        Members::Window(window) => {
            union!(
                BoundaryBspMembers,
                BoundaryBspMembersPayload.window(*window),
                Tag::Window
            )
        }
        Members::Stack(Tabs { before, top, after }) => union!(
            BoundaryBspMembers,
            BoundaryBspMembersPayload.stack(abi::BoundaryBspTabs {
                before: list(before, host),
                top: *top,
                after: list(after, host),
            }),
            Tag::Stack
        ),
    }
}

fn tree_leaf(leaf: &Leaf<u64>, host: &abi::RocHost) -> abi::BoundaryBspTreeLeaf {
    abi::BoundaryBspTreeLeaf {
        content: match &leaf.content {
            Content::Empty => union!(
                BoundaryBspContent,
                BoundaryBspContentPayload.empty,
                abi::BoundaryBspContentTag::Empty
            ),
            Content::Held { members, fill } => union!(
                BoundaryBspContent,
                BoundaryBspContentPayload.held(abi::BoundaryBspHeld {
                    members: tree_members(members, host),
                    fill: bsp_fill(*fill),
                }),
                abi::BoundaryBspContentTag::Held
            ),
        },
        preselect: match leaf.preselect {
            Preselect::Unset => union!(
                BoundaryBspPreselect,
                BoundaryBspPreselectPayload.unset,
                abi::BoundaryBspPreselectTag::Unset
            ),
            Preselect::Toward(direction) => union!(
                BoundaryBspPreselect,
                BoundaryBspPreselectPayload.toward(tree_direction(direction)),
                abi::BoundaryBspPreselectTag::Toward
            ),
        },
    }
}

fn tree_node(node: &TreeNode<NodeKey, u64>, host: &abi::RocHost) -> abi::BoundaryBspTreeNode {
    abi::BoundaryBspTreeNode {
        parent: match node.parent {
            Parent::Root => union!(
                BoundaryBspParent,
                BoundaryBspParentPayload.root,
                abi::BoundaryBspParentTag::Root
            ),
            Parent::Split(above) => union!(
                BoundaryBspParent,
                BoundaryBspParentPayload.split(above.0),
                abi::BoundaryBspParentTag::Split
            ),
        },
        kind: match &node.kind {
            Kind::Split(Split {
                orientation,
                ratio,
                children: [first, second],
                memory,
            }) => union!(
                BoundaryBspKind,
                BoundaryBspKindPayload.split(abi::BoundaryBspTreeSplit {
                    orientation: tree_orientation(*orientation),
                    ratio: *ratio,
                    first: first.0,
                    second: second.0,
                    memory: match memory {
                        Memory::Stops => union!(
                            BoundaryBspMemory,
                            BoundaryBspMemoryPayload.stops,
                            abi::BoundaryBspMemoryTag::Stops
                        ),
                        Memory::Toward(side) => union!(
                            BoundaryBspMemory,
                            BoundaryBspMemoryPayload.toward(tree_side(*side)),
                            abi::BoundaryBspMemoryTag::Toward
                        ),
                    },
                }),
                abi::BoundaryBspKindTag::Split
            ),
            Kind::Leaf(leaf) => union!(
                BoundaryBspKind,
                BoundaryBspKindPayload.leaf(tree_leaf(leaf, host)),
                abi::BoundaryBspKindTag::Leaf
            ),
        },
    }
}

/// A system as the platform holds it, for `roc_bsp_tree_seed`.
fn tree_seed(view: &TreeView, host: &abi::RocHost) -> abi::BoundaryBspTreeSeed {
    let layouts: Vec<abi::BoundaryBspLayoutSlot> = view
        .layouts
        .iter()
        .map(|(held, version)| match held {
            Some(layout) => union!(
                BoundaryBspLayoutSlot,
                BoundaryBspLayoutSlotPayload.occupied(abi::BoundaryBspLayoutHeld {
                    version: *version,
                    root: layout.root.0,
                }),
                abi::BoundaryBspLayoutSlotTag::Occupied
            ),
            None => union!(
                BoundaryBspLayoutSlot,
                BoundaryBspLayoutSlotPayload.vacant(*version),
                abi::BoundaryBspLayoutSlotTag::Vacant
            ),
        })
        .collect();
    let nodes: Vec<abi::BoundaryBspNodeSlot> = view
        .nodes
        .iter()
        .map(|(held, version)| match held {
            Some(node) => union!(
                BoundaryBspNodeSlot,
                BoundaryBspNodeSlotPayload.occupied(abi::BoundaryBspNodeHeld {
                    version: *version,
                    node: tree_node(node, host),
                }),
                abi::BoundaryBspNodeSlotTag::Occupied
            ),
            None => union!(
                BoundaryBspNodeSlot,
                BoundaryBspNodeSlotPayload.vacant(*version),
                abi::BoundaryBspNodeSlotTag::Vacant
            ),
        })
        .collect();
    abi::BoundaryBspTreeSeed {
        layouts: list(&layouts, host),
        layout_free: list(&view.layout_free, host),
        nodes: list(&nodes, host),
        node_free: list(&view.node_free, host),
        left_out: list(&view.left_out, host),
        insertion: insertion(view.insertion),
        aspect: aspect(view.aspect),
    }
}

/// The BSP trees' box, which only Roc can read or free; only the `roc_bsp_tree_*` entries make
/// and take one. Dropping it releases the trees.
pub(crate) struct BspTreeBox(abi::RocBox);

impl BspTreeBox {
    pub(crate) fn new(point: InsertionPoint) -> BspTreeBox {
        BspTreeBox(unsafe { abi::roc_bsp_tree_init(insertion(point)) })
    }

    /// The system `view` holds, which the caller has parsed (`TreeView::tree`).
    pub(crate) fn seeded(view: &TreeView, host: &abi::RocHost) -> BspTreeBox {
        // SAFETY: the seed's lists are allocated through the host and consumed by Roc.
        BspTreeBox(unsafe { abi::roc_bsp_tree_seed(tree_seed(view, host)) })
    }

    /// The raw box, handed to Roc (see [`LifecycleBox::into_raw`]).
    fn into_raw(self) -> abi::RocBox {
        let raw = self.0;
        std::mem::forget(self);
        raw
    }

    /// One operation: the trees after it, and what it answered.
    pub(crate) fn step(self, event: TreeEvent, host: &abi::RocHost) -> (BspTreeBox, TreeAnswer) {
        // SAFETY: the box is these trees', given to the step; its successor comes back. The
        // answer is read, then released.
        let stepped = unsafe { abi::roc_bsp_tree_step(self.into_raw(), tree_event(event, host)) };
        let answer = from_tree_answer(stepped.answer);
        unsafe { stepped.answer.decref(host) };
        (BspTreeBox(stepped.model), answer)
    }

    /// A layout's frames, as the BSP geometry gives them for the trees' own shape: the screen,
    /// the gaps, and every window's constraints the caller knows (by the window's word).
    pub(crate) fn frames(
        self,
        layout: LayoutKey,
        screen: Rect,
        layout_gaps: Gaps,
        known: &[(u64, WindowConstraints)],
        host: &abi::RocHost,
    ) -> (BspTreeBox, Vec<(WindowId, Rect)>) {
        let known: Vec<abi::BoundaryBspKnown> = known
            .iter()
            .map(|&(window, held)| abi::BoundaryBspKnown {
                window,
                constraints: window_constraints(held),
            })
            .collect();
        let question = abi::BoundaryBspFramesQuestion {
            layout: layout.0,
            screen: rect(screen),
            gaps: gaps(layout_gaps),
            known: list(&known, host),
        };
        // SAFETY: as in `step`; the question's list is allocated through the host and consumed
        // by Roc; the frames are Roc's answer, read in full and released by `frames`.
        let framed = unsafe { abi::roc_bsp_tree_frames(self.into_raw(), question) };
        (BspTreeBox(framed.model), frames(framed.frames, host))
    }

    /// Each table's slots and free list, the windows left out in order, the settings.
    pub(crate) fn view(self, host: &abi::RocHost) -> (BspTreeBox, TreeView) {
        // SAFETY: as in `step`; the lists are Roc's answer, owned here, read in full, released.
        let viewed = unsafe { abi::roc_bsp_tree_view(self.into_raw()) };
        let layouts = viewed
            .layouts
            .as_slice()
            .iter()
            .map(|slot| match slot.tag {
                abi::BoundaryBspLayoutSlotTag::Occupied => {
                    // SAFETY: the tag says the payload is the held layout.
                    let held = unsafe { slot.borrow_payload_occupied_unchecked() };
                    (Some(Layout { root: NodeKey(held.root) }), held.version)
                }
                // SAFETY: the tag says the payload is the vacant version.
                abi::BoundaryBspLayoutSlotTag::Vacant => {
                    (None, unsafe { *slot.borrow_payload_vacant_unchecked() })
                }
            })
            .collect();
        let nodes = viewed
            .nodes
            .as_slice()
            .iter()
            .map(|slot| match slot.tag {
                abi::BoundaryBspNodeSlotTag::Occupied => {
                    // SAFETY: the tag says the payload is the held node.
                    let held = unsafe { slot.borrow_payload_occupied_unchecked() };
                    (Some(from_tree_node(&held.node)), held.version)
                }
                // SAFETY: the tag says the payload is the vacant version.
                abi::BoundaryBspNodeSlotTag::Vacant => {
                    (None, unsafe { *slot.borrow_payload_vacant_unchecked() })
                }
            })
            .collect();
        let view = TreeView {
            layouts,
            layout_free: viewed.layout_free.as_slice().to_vec(),
            nodes,
            node_free: viewed.node_free.as_slice().to_vec(),
            left_out: viewed.left_out.as_slice().to_vec(),
            insertion: from_insertion(viewed.insertion),
            aspect: from_aspect(viewed.aspect),
        };
        unsafe {
            viewed.layouts.decref(host);
            viewed.layout_free.decref(host);
            abi::decref_list_of_boundary_bsp_node_slot(viewed.nodes, host);
            viewed.node_free.decref(host);
            viewed.left_out.decref(host);
        }
        (BspTreeBox(viewed.model), view)
    }
}

impl Drop for BspTreeBox {
    fn drop(&mut self) {
        // SAFETY: the box is these trees' and is never used again.
        unsafe { abi::roc_bsp_tree_release(self.0) }
    }
}
