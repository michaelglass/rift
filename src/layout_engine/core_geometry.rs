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
    shape
        .frames()
        .into_iter()
        .map(|(window, frame)| (window, cg_rect(frame)))
        .collect()
}

/// A BSP layout's frames, as the core computes them.
pub(crate) fn tile_bsp(shape: &bsp::Bsp<WindowId>) -> Vec<(WindowId, CGRect)> {
    shape
        .frames()
        .into_iter()
        .map(|(window, frame)| (window, cg_rect(frame)))
        .collect()
}
