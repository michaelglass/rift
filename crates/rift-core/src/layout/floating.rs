//! A floating layout's frames: where a window goes the first time the layout places it, and,
//! for a whole layout, each window's frame (fullscreen, the frame the layout keeps for it, or its
//! first frame). The frames the layout keeps are the platform's: the user moves them.

use super::{Gaps, WindowConstraints, tiling_area};
use crate::plain::Rect;

/// The screen a floating layout places windows on, and whether new windows spread out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Floating {
    pub screen: Rect,
    /// Cascade the windows placed together diagonally, rather than stacking them in the centre.
    pub spread: bool,
}

/// A window the layout places for the first time: its constraints as reported, and the frame
/// it already has on screen, if it has been seen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Newcomer {
    pub constraints: WindowConstraints,
    pub seen: Seen,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seen {
    Live(Rect),
    Unseen,
}

/// A floating layout's windows, in the layout's order, and how each is placed.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout<W> {
    pub floating: Floating,
    pub gaps: Gaps,
    pub windows: Vec<Slot<W>>,
}

/// One of a layout's windows. A window left out of the layout (suspended) has no frame, but
/// holds its position in a cascade.
#[derive(Debug, Clone, PartialEq)]
pub enum Slot<W> {
    LeftOut,
    In { window: W, place: Place },
}

/// Where a window of a floating layout goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place {
    /// Fullscreen: the screen.
    Screen,
    /// Fullscreen within the gaps: the tiling area.
    WithinGaps,
    /// The frame the layout keeps for it.
    Kept(Rect),
    /// Its first frame (it has no kept frame yet), cascaded by its position when spreading.
    First(WindowConstraints),
}

impl<W> Layout<W> {
    /// The same layout on other window ids (another platform's types).
    pub fn map<V>(self, window: &impl Fn(W) -> V) -> Layout<V> {
        Layout {
            floating: self.floating,
            gaps: self.gaps,
            windows: self
                .windows
                .into_iter()
                .map(|slot| match slot {
                    Slot::LeftOut => Slot::LeftOut,
                    Slot::In { window: id, place } => Slot::In { window: window(id), place },
                })
                .collect(),
        }
    }
}

impl<W: Copy> Layout<W> {
    /// Each window's frame, in the layout's order (none for a window left out). A first frame
    /// cascades by the window's position among all the layout's windows.
    pub fn frames(&self) -> Vec<(W, Rect)> {
        let floating = self.floating;
        let count = self.windows.len();
        let slots = count.saturating_sub(1) as f64;
        self.windows
            .iter()
            .enumerate()
            .filter_map(|(position, slot)| match *slot {
                Slot::LeftOut => None,
                Slot::In { window, place } => Some((window, match place {
                    Place::Screen => floating.screen,
                    Place::WithinGaps => tiling_area(floating.screen, self.gaps),
                    Place::Kept(frame) => frame,
                    Place::First(constraints) => {
                        let frame = floating.initial_frame(Newcomer {
                            constraints,
                            seen: Seen::Unseen,
                        });
                        if floating.spread && count > 1 {
                            floating.cascade(frame, position as f64 / slots, slots)
                        } else {
                            frame
                        }
                    }
                })),
            })
            .collect()
    }
}

/// The step between cascaded windows, in points on each axis.
const CASCADE_STEP: f64 = 36.0;

impl Floating {
    /// A window's frame when nothing else is known: 70% of the screen on each axis, within the
    /// window's limits (a locked axis keeps its length), centred.
    pub fn default_frame(&self, constraints: WindowConstraints) -> Rect {
        let limits = constraints.normalized();
        let dimension = |value: f64, horizontal| {
            if let Some(fixed) = limits.fixed_for_axis(horizontal) {
                return fixed;
            }
            let value = value.max(limits.reported_min(horizontal));
            let max = limits.max_for_axis(horizontal);
            if max > 0.0 { value.min(max) } else { value }
        };
        self.centred(
            dimension(self.screen.w * 0.7, true),
            dimension(self.screen.h * 0.7, false),
        )
    }

    /// The first frame of each window, in the layout's order: its live frame, no larger than
    /// the default (recentred if it had to shrink), or the default; cascaded when spreading.
    pub fn first_frames(&self, windows: &[Newcomer]) -> Vec<Rect> {
        windows
            .iter()
            .enumerate()
            .map(|(position, window)| self.first_frame(*window, position, windows.len()))
            .collect()
    }

    /// The first frame of the window at `position` among `count` placed together: its live
    /// frame, no larger than the default (recentred if it had to shrink), or the default;
    /// cascaded by its position when spreading.
    pub fn first_frame(&self, window: Newcomer, position: usize, count: usize) -> Rect {
        let frame = self.initial_frame(window);
        if self.spread && count > 1 {
            let slots = (count - 1) as f64;
            self.cascade(frame, position as f64 / slots, slots)
        } else {
            frame
        }
    }

    fn initial_frame(&self, window: Newcomer) -> Rect {
        let default = self.default_frame(window.constraints);
        let frame = match window.seen {
            Seen::Live(frame) => frame,
            Seen::Unseen => default,
        };
        let (w, h) = (frame.w.min(default.w), frame.h.min(default.h));
        if (w, h) != (frame.w, frame.h) {
            self.centred(w, h)
        } else {
            frame
        }
    }

    /// `frame` moved to `fraction` of the way along a diagonal of `slots` steps, centred on the
    /// screen and no longer than the room the frame leaves.
    fn cascade(&self, frame: Rect, fraction: f64, slots: f64) -> Rect {
        let available_x = (self.screen.w - frame.w).max(0.0);
        let available_y = (self.screen.h - frame.h).max(0.0);
        let span_x = (slots * CASCADE_STEP).min(available_x);
        let span_y = (slots * CASCADE_STEP).min(available_y);
        Rect {
            x: self.screen.x + available_x / 2.0 - span_x / 2.0 + span_x * fraction,
            y: self.screen.y + available_y / 2.0 - span_y / 2.0 + span_y * fraction,
            w: frame.w,
            h: frame.h,
        }
    }

    /// A `w` by `h` frame centred on the screen.
    fn centred(&self, w: f64, h: f64) -> Rect {
        let (mid_x, mid_y) = (
            self.screen.x + self.screen.w * 0.5,
            self.screen.y + self.screen.h * 0.5,
        );
        Rect {
            x: mid_x - w / 2.,
            y: mid_y - h / 2.,
            w,
            h,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Size, Sizing};

    const SCREEN: Rect = Rect {
        x: 100.,
        y: 0.,
        w: 1000.,
        h: 800.,
    };

    fn newcomer(seen: Seen) -> Newcomer {
        Newcomer {
            constraints: WindowConstraints::default(),
            seen,
        }
    }

    #[test]
    fn a_window_starts_at_seventy_percent_centred_within_its_limits() {
        let floating = Floating { screen: SCREEN, spread: false };
        assert_eq!(floating.default_frame(WindowConstraints::default()), Rect {
            x: 250.,
            y: 120.,
            w: 700.,
            h: 560.
        });
        let wide_minimum = WindowConstraints {
            sizing: Sizing::new(true, false, Size::reported(900., 0.)),
            max_height: 300.,
            ..Default::default()
        };
        assert_eq!(floating.default_frame(wide_minimum), Rect {
            x: 150.,
            y: 250.,
            w: 900.,
            h: 300.
        });
    }

    #[test]
    fn a_live_frame_is_kept_unless_larger_than_the_default() {
        let floating = Floating { screen: SCREEN, spread: false };
        let small = Rect { x: 5., y: 6., w: 300., h: 200. };
        let large = Rect {
            x: 5.,
            y: 6.,
            w: 1000.,
            h: 200.,
        };
        assert_eq!(
            floating.first_frames(&[newcomer(Seen::Live(small)), newcomer(Seen::Live(large))]),
            vec![small, Rect {
                x: 250.,
                y: 300.,
                w: 700.,
                h: 200.
            }]
        );
    }

    #[test]
    fn windows_placed_together_cascade_when_spreading() {
        let floating = Floating { screen: SCREEN, spread: true };
        let frames = floating.first_frames(&[newcomer(Seen::Unseen); 3]);
        // Two steps of 36 around the centred default (250, 120).
        let at = |x, y| Rect { x, y, w: 700., h: 560. };
        assert_eq!(frames, vec![at(214., 84.), at(250., 120.), at(286., 156.)]);
        assert_eq!(floating.first_frames(&[newcomer(Seen::Unseen)]), vec![at(
            250., 120.
        )]);
    }
}
