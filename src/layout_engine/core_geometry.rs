//! Where the layout engine asks the core for geometry, and the conversions on the way.

use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use rift_core::layout::{Gaps, bsp};
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

/// A BSP layout's frames, as the core computes them.
pub(crate) fn tile_bsp(shape: &bsp::Bsp<WindowId>) -> Vec<(WindowId, CGRect)> {
    let frames = shape.frames();
    #[cfg(feature = "roc-core")]
    let frames = roc_frames(shape, frames);
    frames.into_iter().map(|(window, frame)| (window, cg_rect(frame))).collect()
}

/// The Roc core's frames for `shape`, which must be the Rust core's (`shadow`): a difference
/// is a bug in one of them, so Rift stops.
#[cfg(feature = "roc-core")]
fn roc_frames(shape: &bsp::Bsp<WindowId>, shadow: Vec<(WindowId, Rect)>) -> Vec<(WindowId, Rect)> {
    use rift_core::plain::WindowId as PlainWindow;
    let plain = shape.clone().map(&|window: WindowId| PlainWindow {
        pid: window.pid,
        idx: window.idx.get(),
    });
    let frames: Vec<(WindowId, Rect)> = rift_core_roc::tile_bsp(plain)
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
