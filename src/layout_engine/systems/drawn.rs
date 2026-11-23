//! Drawn inputs for the differentials that hold the core's layout geometry to the layout
//! systems' frames: screens, gaps, offsets, stack bars and window constraints, leaning on the
//! edges (locked axes, maxima under minima, NaN minima, fractional lengths).

use objc2_core_foundation::{CGPoint, CGRect, CGSize};

use super::WindowLayoutConstraints;
use crate::actor::app::WindowId;
use crate::common::config::{GapSettings, StackLineReserve};

/// xorshift64*: deterministic, no dependency.
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub(crate) fn below(&mut self, n: u64) -> u64 { self.next() % n }

    pub(crate) fn chance(&mut self, percent: u64) -> bool { self.below(100) < percent }

    /// Mostly round numbers, sometimes halves, sometimes anything in `low..high`.
    pub(crate) fn length(&mut self, low: f64, high: f64) -> f64 {
        let value = low + (high - low) * (self.next() >> 11) as f64 / (1u64 << 53) as f64;
        match self.below(3) {
            0 => value.round(),
            1 => (value * 2.).round() / 2.,
            _ => value,
        }
    }

    pub(crate) fn maybe(&mut self, high: f64) -> f64 {
        if self.chance(50) {
            0.
        } else {
            self.length(1., high)
        }
    }
}

pub(crate) fn drawn_constraints(rng: &mut Rng) -> WindowLayoutConstraints {
    let sizing = match rng.below(3) {
        0 => super::Sizing::Fixed,
        1 => super::Sizing::Resizable,
        _ => super::Sizing::ResizableAtLeastMin,
    };
    let mut c = WindowLayoutConstraints {
        sizing,
        locked_width: rng.maybe(1500.),
        locked_height: rng.maybe(1000.),
        min_width: rng.maybe(900.),
        min_height: rng.maybe(700.),
        max_width: rng.maybe(1800.),
        max_height: rng.maybe(1200.),
    };
    if rng.chance(10) {
        // An axis the app locks (min = max).
        c.max_width = c.min_width;
    }
    if rng.chance(5) {
        // An axis locked within f64::EPSILON (a small minimum, the maximum one ulp above it).
        c.min_height = rng.length(0.05, 1.0);
        c.max_height = f64::from_bits(c.min_height.to_bits() + 1);
    }
    if rng.chance(5) {
        c.min_height = f64::NAN;
    }
    c
}

pub(crate) fn drawn_gaps(rng: &mut Rng) -> GapSettings {
    let mut gaps = GapSettings::default();
    if rng.chance(80) {
        gaps.outer.top = rng.length(0., 40.);
        gaps.outer.left = rng.length(0., 40.);
        gaps.outer.bottom = rng.length(0., 40.);
        gaps.outer.right = rng.length(0., 40.);
        gaps.inner.horizontal = rng.length(0., 30.);
        gaps.inner.vertical = rng.length(0., 30.);
    }
    gaps
}

pub(crate) fn drawn_line(rng: &mut Rng) -> StackLineReserve {
    use crate::common::config::{HorizontalPlacement, VerticalPlacement};
    StackLineReserve {
        thickness: if rng.chance(40) {
            0.
        } else {
            rng.length(1., 12.)
        },
        spacing: rng.maybe(10.),
        horiz: if rng.chance(50) {
            HorizontalPlacement::Top
        } else {
            HorizontalPlacement::Bottom
        },
        vert: if rng.chance(50) {
            VerticalPlacement::Left
        } else {
            VerticalPlacement::Right
        },
    }
}

pub(crate) fn drawn_screen(rng: &mut Rng) -> CGRect {
    CGRect::new(
        CGPoint::new(rng.length(-2000., 2000.), rng.length(-50., 1000.)),
        CGSize::new(rng.length(40., 3000.), rng.length(40., 2000.)),
    )
}

pub(crate) fn drawn_offset(rng: &mut Rng) -> f64 {
    if rng.chance(20) {
        rng.length(0., 900.)
    } else {
        rng.length(0., 60.)
    }
}

/// Constraints for some of `windows`.
pub(crate) fn drawn_constraints_for(
    rng: &mut Rng,
    windows: &[WindowId],
) -> crate::common::collections::HashMap<WindowId, WindowLayoutConstraints> {
    let mut constraints = crate::common::collections::HashMap::default();
    for &window in windows {
        if rng.chance(60) {
            constraints.insert(window, drawn_constraints(rng));
        }
    }
    constraints
}

/// Frames as their float bits, so `-0.0`, `0.0` and NaNs are told apart.
pub(crate) fn bits(frames: &[(WindowId, CGRect)]) -> Vec<(WindowId, [u64; 4])> {
    frames
        .iter()
        .map(|&(window, r)| {
            (
                window,
                [r.origin.x, r.origin.y, r.size.width, r.size.height].map(f64::to_bits),
            )
        })
        .collect()
}

/// The core's frames as rift's.
pub(crate) fn core_frames(
    shape: Option<rift_core::layout::traditional::Traditional<WindowId>>,
) -> Vec<(WindowId, CGRect)> {
    shape
        .map_or_else(Vec::new, |shape| shape.frames())
        .into_iter()
        .map(|(window, frame)| (window, crate::layout_engine::cg_rect(frame)))
        .collect()
}
