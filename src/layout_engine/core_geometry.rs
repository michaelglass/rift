//! Where the layout engine asks the core for geometry, and the conversions on the way.

use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use rift_core::layout::traditional::{self, StackLine};
use rift_core::layout::{Gaps, Size, bsp};
use rift_core::plain::Rect;

use crate::actor::app::WindowId;

pub(crate) fn core_rect(rect: CGRect) -> Rect {
    Rect {
        x: rect.origin.x,
        y: rect.origin.y,
        w: rect.size.width,
        h: rect.size.height,
    }
}

pub(crate) fn cg_rect(rect: Rect) -> CGRect {
    CGRect::new(CGPoint::new(rect.x, rect.y), CGSize::new(rect.w, rect.h))
}

/// A size an app reported, as the core parses it (no size, or 0x0, is none).
pub(crate) fn reported_size(size: Option<CGSize>) -> Option<Size> {
    size.and_then(|size| Size::reported(size.width, size.height))
}

/// The gaps one screen uses (any per-display override already applied by the caller).
pub(crate) fn core_gaps(gaps: &crate::common::config::GapSettings) -> Gaps {
    Gaps {
        top: gaps.outer.top,
        left: gaps.outer.left,
        bottom: gaps.outer.bottom,
        right: gaps.outer.right,
        horizontal: gaps.inner.horizontal,
        vertical: gaps.inner.vertical,
    }
}

/// The stack bar's reservation, as the core takes it.
pub(crate) fn core_stack_line(line: crate::common::config::StackLineReserve) -> StackLine {
    use crate::common::config::{HorizontalPlacement, VerticalPlacement};
    StackLine {
        thickness: line.thickness,
        spacing: line.spacing,
        top: line.horiz == HorizontalPlacement::Top,
        left: line.vert == VerticalPlacement::Left,
    }
}

/// A traditional layout's frames, as the core computes them.
pub(crate) fn tile_traditional(
    shape: &traditional::Traditional<WindowId>,
) -> Vec<(WindowId, CGRect)> {
    let frames = shape.frames();
    #[cfg(feature = "roc-core")]
    let frames = shadowed(
        shape,
        frames,
        rift_core_roc::tile_traditional(shape.clone().map(&to_plain)).unwrap_or_else(|error| {
            panic!("the Roc core gave no frames ({error:?}) for {shape:?}")
        }),
    );
    frames.into_iter().map(|(window, frame)| (window, cg_rect(frame))).collect()
}

/// A BSP layout's frames, as the core computes them.
pub(crate) fn tile_bsp(shape: &bsp::Bsp<WindowId>) -> Vec<(WindowId, CGRect)> {
    let frames = shape.frames();
    #[cfg(feature = "roc-core")]
    let frames = shadowed(
        shape,
        frames,
        rift_core_roc::tile_bsp(shape.clone().map(&to_plain)).unwrap_or_else(|error| {
            panic!("the Roc core gave no frames ({error:?}) for {shape:?}")
        }),
    );
    frames.into_iter().map(|(window, frame)| (window, cg_rect(frame))).collect()
}

#[cfg(feature = "roc-core")]
fn to_plain(window: WindowId) -> rift_core::plain::WindowId {
    rift_core::plain::WindowId {
        pid: window.pid,
        idx: window.idx.get(),
    }
}

/// The Roc core's frames for `shape` (`roc`, on plain window ids), which must be the Rust
/// core's (`shadow`): a difference is a bug in one of them, so Rift stops.
#[cfg(feature = "roc-core")]
fn shadowed(
    shape: &impl std::fmt::Debug,
    shadow: Vec<(WindowId, Rect)>,
    roc: Vec<(rift_core::plain::WindowId, Rect)>,
) -> Vec<(WindowId, Rect)> {
    let frames: Vec<(WindowId, Rect)> = roc
        .into_iter()
        .map(|(window, frame)| {
            let idx =
                std::num::NonZeroU32::new(window.idx).expect("the core names only real windows");
            (WindowId { pid: window.pid, idx }, frame)
        })
        .collect();
    assert_eq!(
        frames, shadow,
        "the Roc core and its Rust shadow tile {shape:?} differently"
    );
    frames
}
