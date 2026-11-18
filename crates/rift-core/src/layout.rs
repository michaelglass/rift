//! Layout geometry: where windows go, as pure functions of a layout's shape. The platform still
//! owns the layout trees; for each arrange it describes the shape it has (pruned to what takes
//! space) and the core computes the frames.

pub mod axis;
pub mod bsp;
pub mod traditional;

use crate::plain::Rect;

/// What an app allows of a window's size (0 means no limit, as the app reported it).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WindowConstraints {
    pub is_resizable: bool,
    pub locked_width: f64,
    pub locked_height: f64,
    pub min_width: f64,
    pub min_height: f64,
    pub max_width: f64,
    pub max_height: f64,
}

impl WindowConstraints {
    pub fn normalized(self) -> Self {
        let clean = |v: f64| if v.is_finite() { v.max(0.0) } else { 0.0 };
        let min_width = clean(self.min_width);
        let min_height = clean(self.min_height);
        let mut max_width = clean(self.max_width);
        let mut max_height = clean(self.max_height);
        if max_width > 0.0 && max_width < min_width {
            max_width = min_width;
        }
        if max_height > 0.0 && max_height < min_height {
            max_height = min_height;
        }
        Self {
            is_resizable: self.is_resizable,
            locked_width: clean(self.locked_width),
            locked_height: clean(self.locked_height),
            min_width,
            min_height,
            max_width,
            max_height,
        }
    }

    pub fn min_for_axis(self, horizontal: bool) -> f64 {
        if horizontal {
            self.min_width
        } else {
            self.min_height
        }
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
        let min = self.min_for_axis(horizontal);
        let max = self.max_for_axis(horizontal);
        // Axis-specific lock: when min/max collapse to the same positive value,
        // treat that axis as fixed even if the window is generally resizable.
        if min > 0.0 && max > 0.0 && (min - max).abs() <= f64::EPSILON {
            return Some(max);
        }
        if !self.is_resizable {
            return (locked > 0.0).then_some(locked);
        }
        None
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
