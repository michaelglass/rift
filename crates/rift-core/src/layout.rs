//! Layout geometry: where windows go, as pure functions of a layout's shape. The platform still
//! owns the layout trees; for each arrange it describes the shape it has (pruned to what takes
//! space) and the core computes the frames.

pub mod axis;
pub mod bsp;
pub mod traditional;

use crate::plain::Rect;

/// A size an app reported, parsed once: both axes finite and not negative.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Size {
    width: f64,
    height: f64,
}

impl Size {
    pub const ZERO: Self = Self { width: 0.0, height: 0.0 };

    /// What an app reported: a non-finite or negative axis is 0, and 0x0 is no size at all.
    pub fn reported(width: f64, height: f64) -> Option<Self> {
        let clean = |v: f64| if v.is_finite() { v.max(0.0) } else { 0.0 };
        let size = Self {
            width: clean(width),
            height: clean(height),
        };
        (size != Self::ZERO).then_some(size)
    }

    pub fn width(self) -> f64 { self.width }

    pub fn height(self) -> f64 { self.height }

    pub fn axis(self, horizontal: bool) -> f64 { if horizontal { self.width } else { self.height } }
}

/// How tiling may size a window, with the minimum size the window reports.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sizing {
    /// Not resizable: kept at its own size.
    Fixed { min: Size },
    /// Resizable; traditional tiling may shrink it below `advisory_min`.
    Resizable { advisory_min: Size },
    /// Resizable, but never below `min` (an app rule's `respect_min_size`).
    ResizableAtLeastMin { min: Size },
}

impl Default for Sizing {
    fn default() -> Self { Self::Fixed { min: Size::ZERO } }
}

impl Sizing {
    /// What the app says of a window (whether it is resizable, the minimum it reports, if
    /// any), and whether a rule asks to respect that minimum.
    pub fn new(is_resizable: bool, respect_min_size: bool, min: Option<Size>) -> Self {
        let min = min.unwrap_or(Size::ZERO);
        match (is_resizable, respect_min_size) {
            (false, _) => Self::Fixed { min },
            (true, false) => Self::Resizable { advisory_min: min },
            (true, true) => Self::ResizableAtLeastMin { min },
        }
    }

    /// The minimum the window reports, advisory or not.
    pub fn reported_min(self) -> Size {
        match self {
            Self::Fixed { min } | Self::Resizable { advisory_min: min } => min,
            Self::ResizableAtLeastMin { min } => min,
        }
    }
}

/// What an app allows of a window's size (0 means no limit, as the app reported it).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WindowConstraints {
    pub sizing: Sizing,
    pub locked_width: f64,
    pub locked_height: f64,
    pub max_width: f64,
    pub max_height: f64,
}

impl WindowConstraints {
    pub fn normalized(self) -> Self {
        let clean = |v: f64| if v.is_finite() { v.max(0.0) } else { 0.0 };
        let min = self.sizing.reported_min();
        let mut max_width = clean(self.max_width);
        let mut max_height = clean(self.max_height);
        if max_width > 0.0 && max_width < min.width() {
            max_width = min.width();
        }
        if max_height > 0.0 && max_height < min.height() {
            max_height = min.height();
        }
        Self {
            sizing: self.sizing,
            locked_width: clean(self.locked_width),
            locked_height: clean(self.locked_height),
            max_width,
            max_height,
        }
    }

    /// The window's reported minimum on an axis, advisory or not.
    pub fn reported_min(self, horizontal: bool) -> f64 {
        self.sizing.reported_min().axis(horizontal)
    }

    pub fn max_for_axis(self, horizontal: bool) -> f64 {
        if horizontal {
            self.max_width
        } else {
            self.max_height
        }
    }

    pub fn fixed_for_axis(self, horizontal: bool) -> Option<f64> {
        let locked = if horizontal {
            self.locked_width
        } else {
            self.locked_height
        };
        let min = self.reported_min(horizontal);
        let max = self.max_for_axis(horizontal);
        // Axis-specific lock: when min/max collapse to the same positive value, treat that
        // axis as fixed even if the window is generally resizable. An advisory minimum counts
        // here: min == max is a lock, not a floor.
        if min > 0.0 && max > 0.0 && (min - max).abs() <= f64::EPSILON {
            return Some(max);
        }
        match self.sizing {
            Sizing::Fixed { .. } => (locked > 0.0).then_some(locked),
            Sizing::Resizable { .. } | Sizing::ResizableAtLeastMin { .. } => None,
        }
    }

    pub fn resizable_for_axis(self, horizontal: bool) -> bool {
        self.fixed_for_axis(horizontal).is_none()
    }

    pub fn resizable_any_axis(self) -> bool {
        self.resizable_for_axis(true) || self.resizable_for_axis(false)
    }
}

/// The gaps around a screen's tiling area and between tiled windows.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Gaps {
    pub top: f64,
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
    pub horizontal: f64,
    pub vertical: f64,
}

/// `rect` with its edges on whole points (each corner rounded, half away from zero).
pub fn round(rect: Rect) -> Rect {
    let (x0, y0) = (rect.x.round(), rect.y.round());
    let (x1, y1) = ((rect.x + rect.w).round(), (rect.y + rect.h).round());
    Rect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    }
}

/// The part of `screen` inside the outer gaps.
pub fn tiling_area(screen: Rect, gaps: Gaps) -> Rect {
    if gaps.top == 0.0 && gaps.left == 0.0 && gaps.bottom == 0.0 && gaps.right == 0.0 {
        screen
    } else {
        round(Rect {
            x: screen.x + gaps.left,
            y: screen.y + gaps.top,
            w: (screen.w - gaps.left - gaps.right).max(0.0),
            h: (screen.h - gaps.top - gaps.bottom).max(0.0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reported_size_is_parsed_once_and_none_means_no_size() {
        assert_eq!(Size::reported(0.0, 0.0), None);
        assert_eq!(Size::reported(f64::NAN, -3.0), None);
        let size = Size::reported(400.0, f64::INFINITY).unwrap();
        assert_eq!((size.width(), size.height()), (400.0, 0.0));
        assert_eq!((size.axis(true), size.axis(false)), (400.0, 0.0));
    }

    #[test]
    fn sizing_says_how_tiling_may_size_a_window() {
        let min = Size::reported(400.0, 600.0);
        assert_eq!(Sizing::new(false, true, min), Sizing::Fixed {
            min: min.unwrap()
        });
        assert_eq!(Sizing::new(true, false, min), Sizing::Resizable {
            advisory_min: min.unwrap()
        });
        assert_eq!(Sizing::new(true, true, min), Sizing::ResizableAtLeastMin {
            min: min.unwrap()
        });
        assert_eq!(Sizing::new(true, true, None).reported_min(), Size::ZERO);
    }

    #[test]
    fn a_maximum_below_the_reported_minimum_is_raised_to_it_and_equal_bounds_lock_an_axis() {
        let c = WindowConstraints {
            sizing: Sizing::new(true, false, Size::reported(500.0, 0.0)),
            max_width: 300.0,
            ..Default::default()
        }
        .normalized();
        assert_eq!(c.max_for_axis(true), 500.0);
        // min == max is a lock, even for an advisory minimum.
        assert_eq!(c.fixed_for_axis(true), Some(500.0));
        assert_eq!(c.fixed_for_axis(false), None);
    }
}
