use objc2_core_foundation::CGRect;

/// The part of `screen` inside the outer gaps (the core's `tiling_area`).
pub fn compute_tiling_area(screen: CGRect, gaps: &crate::common::config::GapSettings) -> CGRect {
    use crate::layout_engine::{cg_rect, core_gaps, core_rect};
    cg_rect(rift_core::layout::tiling_area(
        core_rect(screen),
        core_gaps(gaps),
    ))
}
