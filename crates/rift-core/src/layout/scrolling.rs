//! A scrolling layout's strip: columns side by side in world space (x from 0, the tiling area's
//! y), each column's windows stacked in rows, and the camera's resting positions. The platform
//! owns the columns and the camera; for each arrange it describes the strip it has (its active
//! windows only) and the core computes where everything goes.

use std::cmp::Ordering;

use super::WindowConstraints;
use super::axis::{AxisConstraints, solve_axis_lengths};
use crate::plain::Rect;

/// A strip with at least one column, each with at least one window: a strip with no active
/// window has no shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Strip<W, C> {
    /// The tiling area: its width is the camera's view, its top and height the rows'.
    pub tiling: Rect,
    /// Between columns.
    pub column_gap: f64,
    /// Between a column's rows.
    pub row_gap: f64,
    pub widths: Widths,
    pub alignment: Alignment,
    pub navigation: Navigation,
    pub first: Column<W, C>,
    pub rest: Vec<Column<W, C>>,
}

/// How a column's width is resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Widths {
    /// A default column's share of the view.
    pub ratio: f64,
    pub min_ratio: f64,
    pub max_ratio: f64,
    /// A default column takes its first window's locked width, if it has one.
    pub preserve_window_sizes: bool,
    /// A strip's only column fills the view.
    pub expand_single_column: bool,
}

/// Where a column the camera stops at sits in the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Center,
    Right,
}

/// Where the camera may rest: at each column's anchor, or at any column edge between the
/// strip's two ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Navigation {
    Anchored,
    Free,
}

/// The width a column asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Width {
    Default,
    Proportion(f64),
    Fixed(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Column<W, C> {
    pub id: C,
    pub width: Width,
    pub first: Row<W>,
    pub rest: Vec<Row<W>>,
}

/// A window in a column, with its constraints as the app reported them and its share of the
/// column's height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Row<W> {
    pub window: W,
    pub constraints: WindowConstraints,
    pub weight: f64,
}

/// Where a strip's columns and windows go, and where its camera may rest.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed<W, C> {
    pub first: Spot<C>,
    pub rest: Vec<Spot<C>>,
    /// Each window's frame in world space, in strip order.
    pub frames: Vec<(W, Rect)>,
    /// The camera's resting positions, in the order a nearest-point search breaks ties by.
    pub snaps: (Snap<C>, Vec<Snap<C>>),
    /// The leftmost and rightmost resting positions.
    pub bounds: (f64, f64),
}

/// A column in world space, and the camera offset its anchor is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spot<C> {
    pub id: C,
    pub world_x: f64,
    pub width: f64,
    /// Where the camera stops to show this column under the strip's alignment.
    pub anchor: f64,
}

/// A camera offset the camera may rest at, and the column it shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snap<C> {
    pub offset: f64,
    pub column: C,
}

/// How the camera settles on a resting point after a release, and which wins a tie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settle<C> {
    /// Free navigation (niri): a tie goes to the leftmost point, and the column shown is the
    /// furthest wholly in view in the direction of travel.
    Niri,
    /// Anchored: a tie goes to the earlier point.
    Anchored,
    /// Anchored, after a gesture: a tie goes to a point showing the active column.
    AnchoredTowards(C),
}

/// Where the camera settles, and the column it shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settled<C> {
    pub column: C,
    pub offset: f64,
}

/// A column's place among the strip's columns, which decides its anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    Only,
    First,
    Middle,
    Last,
}

/// A column `ratio` of the view wide, its gap included.
pub fn proportional_width(view: f64, gap: f64, ratio: f64) -> f64 {
    ((view + gap) * ratio - gap).max(1.0)
}

/// The share of the view a column `width` wide takes, its gap included.
pub fn width_ratio(view: f64, gap: f64, width: f64) -> f64 { (width + gap) / (view + gap).max(1.0) }

impl Widths {
    /// The width a column asks for in a `view` wide view: fixed, a share of the view, or by
    /// default its first window's locked width (if the strip preserves window sizes) or the
    /// default share.
    pub fn resolve(self, width: Width, locked_width: f64, view: f64, gap: f64) -> f64 {
        let ratio = match width {
            Width::Fixed(width) => return width.max(1.0),
            Width::Proportion(ratio) => ratio,
            Width::Default => {
                if self.preserve_window_sizes && locked_width.is_finite() && locked_width > 0.0 {
                    width_ratio(view, gap, locked_width)
                } else {
                    self.ratio
                }
            }
        };
        proportional_width(view, gap, self.clamp(ratio))
    }

    /// `ratio` within the configured range (a non-finite ratio is the smallest).
    pub fn clamp(self, ratio: f64) -> f64 {
        let min = self.min_ratio.max(0.05);
        let max = self.max_ratio.max(min);
        if ratio.is_finite() {
            ratio.clamp(min, max)
        } else {
            min
        }
    }
}

impl<W: Copy, C: Copy> Strip<W, C> {
    /// Where the columns and windows go, and where the camera may rest.
    pub fn place(&self) -> Placed<W, C> {
        let only = self.rest.is_empty();
        let (first, mut x, mut frames) = self.column(&self.first, Position::first(only), 0.0);
        let mut rest = Vec::with_capacity(self.rest.len());
        if let Some((last, middle)) = self.rest.split_last() {
            for column in middle {
                let (spot, next, placed) = self.column(column, Position::Middle, x);
                rest.push(spot);
                frames.extend(placed);
                x = next;
            }
            let (spot, _, placed) = self.column(last, Position::Last, x);
            rest.push(spot);
            frames.extend(placed);
        }
        let snaps = self.snaps(first, &rest);
        let bounds = std::iter::once(&snaps.0)
            .chain(&snaps.1)
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), snap| {
                (min.min(snap.offset), max.max(snap.offset))
            });
        Placed {
            first,
            rest,
            frames,
            snaps,
            bounds,
        }
    }

    /// The width a column asks for, before its windows' constraints: a strip's only column
    /// fills the view if the strip expands it.
    fn requested_width(&self, column: &Column<W, C>) -> f64 {
        if self.widths.expand_single_column && self.rest.is_empty() {
            return self.tiling.w;
        }
        self.widths.resolve(
            column.width,
            column.first.constraints.locked_width,
            self.tiling.w,
            self.column_gap,
        )
    }

    /// A column's spot, where the next column starts, and its windows' frames.
    fn column(
        &self,
        column: &Column<W, C>,
        position: Position,
        x: f64,
    ) -> (Spot<C>, f64, Vec<(W, Rect)>) {
        let rows: Vec<(Row<W>, WindowConstraints)> = std::iter::once(&column.first)
            .chain(&column.rest)
            .map(|row| (*row, row.constraints.normalized()))
            .collect();
        let width = self.column_width(column, &rows);
        let available = (self.tiling.h - self.row_gap * (rows.len() - 1) as f64).max(0.0);
        let axis: Vec<AxisConstraints> = rows
            .iter()
            .map(|(row, c)| AxisConstraints {
                min: c.reported_min(false),
                fixed: c.fixed_for_axis(false),
                max: (c.max_height > 0.0).then_some(c.max_height),
                weight: (row.weight - c.reported_min(false)).max(0.001),
                can_grow: c.resizable_for_axis(false),
            })
            .collect();
        let mut y = self.tiling.y;
        let frames = rows
            .iter()
            .zip(solve_axis_lengths(&axis, available))
            .map(|((row, c), height)| {
                let mut w =
                    c.fixed_for_axis(true).unwrap_or(width).max(c.reported_min(true)).min(width);
                let mut h = c
                    .fixed_for_axis(false)
                    .unwrap_or(height)
                    .max(c.reported_min(false))
                    .min(height);
                if c.max_width > 0.0 {
                    w = w.min(c.max_width);
                }
                if c.max_height > 0.0 {
                    h = h.min(c.max_height);
                }
                let frame = Rect { x, y, w, h };
                y += height + self.row_gap;
                (row.window, frame)
            })
            .collect();
        let spot = Spot {
            id: column.id,
            world_x: x,
            width,
            anchor: self.anchor(x, width, position),
        };
        // Upstream's order (`x += width + gap`: the gap joins the width first); it must match
        // upstream's `Geometry::build` bit for bit.
        (spot, x + (width + self.column_gap), frames)
    }

    /// The column's width: what it asks for, within what its windows allow.
    fn column_width(&self, column: &Column<W, C>, rows: &[(Row<W>, WindowConstraints)]) -> f64 {
        let base = self.requested_width(column);
        let mut min: f64 = 1.0;
        let mut fixed: f64 = 0.0;
        let mut max = f64::INFINITY;
        let mut flexible = false;
        for (_, c) in rows {
            min = min.max(c.reported_min(true));
            fixed = fixed.max(c.fixed_for_axis(true).unwrap_or(0.0));
            flexible |= c.resizable_for_axis(true);
            if c.max_width > 0.0 {
                max = max.min(c.max_width);
            }
        }
        if flexible {
            base.min(max).max(min).max(fixed)
        } else {
            min.max(fixed)
        }
    }

    /// Where the camera stops to show a column: a column as wide as the view at its left edge;
    /// the first column at its left edge and the last at its right (of several); otherwise as
    /// the strip aligns it.
    fn anchor(&self, world_x: f64, width: f64, position: Position) -> f64 {
        let view = self.tiling.w;
        let left = world_x;
        let right = world_x + width - view;
        if width >= view {
            return left;
        }
        match position {
            Position::First => left,
            Position::Last => right,
            Position::Only | Position::Middle => match self.alignment {
                Alignment::Left => left,
                Alignment::Center => centered(world_x, width, view),
                Alignment::Right => right,
            },
        }
    }

    /// Anchored: each column's anchor. Free: the strip's two ends, then every column edge
    /// strictly between them.
    fn snaps(&self, first: Spot<C>, rest: &[Spot<C>]) -> (Snap<C>, Vec<Snap<C>>) {
        let spots = || std::iter::once(&first).chain(rest);
        let at = |offset, spot: &Spot<C>| Snap { offset, column: spot.id };
        match self.navigation {
            Navigation::Anchored => (
                at(first.anchor, &first),
                rest.iter().map(|s| at(s.anchor, s)).collect(),
            ),
            Navigation::Free => {
                let view = self.tiling.w;
                let last = rest.last().unwrap_or(&first);
                let left = first.world_x;
                let right = last.world_x + last.width - view;
                let mut points = vec![at(right, last)];
                for spot in spots() {
                    for offset in [spot.world_x, spot.world_x + spot.width - view] {
                        if left < offset && offset < right {
                            points.push(at(offset, spot));
                        }
                    }
                }
                (at(left, &first), points)
            }
        }
    }
}

impl<W, C> Strip<W, C> {
    /// The same strip on other window and column ids (another platform's types).
    pub fn map<V, D>(self, window: &impl Fn(W) -> V, column: &impl Fn(C) -> D) -> Strip<V, D> {
        Strip {
            tiling: self.tiling,
            column_gap: self.column_gap,
            row_gap: self.row_gap,
            widths: self.widths,
            alignment: self.alignment,
            navigation: self.navigation,
            first: self.first.map(window, column),
            rest: self.rest.into_iter().map(|c| c.map(window, column)).collect(),
        }
    }
}

impl<W, C> Column<W, C> {
    fn map<V, D>(self, window: &impl Fn(W) -> V, column: &impl Fn(C) -> D) -> Column<V, D> {
        let row = |row: Row<W>| Row {
            window: window(row.window),
            constraints: row.constraints,
            weight: row.weight,
        };
        Column {
            id: column(self.id),
            width: self.width,
            first: row(self.first),
            rest: self.rest.into_iter().map(row).collect(),
        }
    }
}

impl<W, C> Placed<W, C> {
    /// The same placement on other window and column ids.
    pub fn map<V, D>(self, window: &impl Fn(W) -> V, column: &impl Fn(C) -> D) -> Placed<V, D> {
        let spot = |s: Spot<C>| Spot {
            id: column(s.id),
            world_x: s.world_x,
            width: s.width,
            anchor: s.anchor,
        };
        let snap = |s: Snap<C>| Snap {
            offset: s.offset,
            column: column(s.column),
        };
        Placed {
            first: spot(self.first),
            rest: self.rest.into_iter().map(spot).collect(),
            frames: self.frames.into_iter().map(|(w, frame)| (window(w), frame)).collect(),
            snaps: (snap(self.snaps.0), self.snaps.1.into_iter().map(snap).collect()),
            bounds: self.bounds,
        }
    }
}

/// The offset that centres a column `width` wide at `world_x` in the view (its left edge if
/// it is wider).
fn centered(world_x: f64, width: f64, view: f64) -> f64 { world_x - (view - width).max(0.0) / 2.0 }

impl<C> Spot<C> {
    /// The offset nearest `current` that shows the whole column (its left edge if it is wider
    /// than the view). Adapted from niri's compute_new_view_offset, on an absolute camera.
    pub fn fit(&self, current: f64, view: f64) -> f64 {
        let left = self.world_x;
        if self.width >= view {
            return left;
        }
        current.clamp(left + self.width - view, left)
    }

    /// The offset that centres the column in the view.
    pub fn centered(&self, view: f64) -> f64 { centered(self.world_x, self.width, view) }
}

impl<W, C: Copy + PartialEq> Placed<W, C> {
    /// Every column's spot, in strip order.
    pub fn spots(&self) -> impl DoubleEndedIterator<Item = &Spot<C>> {
        std::iter::once(&self.first).chain(&self.rest)
    }

    /// The spot of the column `id`, if the strip has it.
    pub fn spot(&self, id: C) -> Option<Spot<C>> { self.spots().find(|s| s.id == id).copied() }

    /// Where the camera settles when a release would carry it to `projected` from `from`: the
    /// resting point nearest `projected` (ties as `how` says), showing its column or, for niri,
    /// the furthest column wholly in view in the direction of travel.
    pub fn settle(&self, view: f64, projected: f64, from: f64, how: Settle<C>) -> Settled<C> {
        let compare = |a: &Snap<C>, b: &Snap<C>| {
            (a.offset - projected)
                .abs()
                .total_cmp(&(b.offset - projected).abs())
                .then_with(|| {
                    match how {
                        // Niri sorts its points by position before choosing the nearest.
                        Settle::Niri => a.offset.total_cmp(&b.offset),
                        Settle::Anchored => Ordering::Equal,
                        Settle::AnchoredTowards(active) => {
                            (a.column != active).cmp(&(b.column != active))
                        }
                    }
                })
        };
        // The first nearest point (as Iterator::min_by): a later one wins only if nearer.
        let snap = self.snaps.1.iter().fold(&self.snaps.0, |best, snap| {
            if compare(best, snap) == Ordering::Greater {
                snap
            } else {
                best
            }
        });
        let column = match how {
            Settle::Anchored | Settle::AnchoredTowards(_) => snap.column,
            Settle::Niri if projected >= from => past(self.spots(), snap.column)
                .take_while(|spot| spot.world_x + spot.width <= snap.offset + view)
                .last()
                .map_or(snap.column, |spot| spot.id),
            Settle::Niri => past(self.spots().rev(), snap.column)
                .take_while(|spot| spot.world_x >= snap.offset)
                .last()
                .map_or(snap.column, |spot| spot.id),
        };
        Settled { column, offset: snap.offset }
    }
}

/// The spots after the column `id`, in the order `spots` walks them.
fn past<'a, C: Copy + PartialEq + 'a>(
    spots: impl Iterator<Item = &'a Spot<C>>,
    id: C,
) -> impl Iterator<Item = &'a Spot<C>> {
    spots.skip_while(move |spot| spot.id != id).skip(1)
}

impl Position {
    fn first(only: bool) -> Self { if only { Self::Only } else { Self::First } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Size, Sizing};

    const VIEW: Rect = Rect {
        x: 0.,
        y: 50.,
        w: 1000.,
        h: 800.,
    };

    fn row(window: u32) -> Row<u32> {
        Row {
            window,
            constraints: WindowConstraints {
                sizing: Sizing::Resizable { advisory_min: Size::ZERO },
                ..Default::default()
            },
            weight: 1.0,
        }
    }

    fn column(id: u32, width: Width, rows: &[u32]) -> Column<u32, u32> {
        Column {
            id,
            width,
            first: row(rows[0]),
            rest: rows[1..].iter().map(|&w| row(w)).collect(),
        }
    }

    fn strip(navigation: Navigation, columns: Vec<Column<u32, u32>>) -> Strip<u32, u32> {
        let mut columns = columns.into_iter();
        Strip {
            tiling: VIEW,
            column_gap: 10.,
            row_gap: 20.,
            widths: Widths {
                ratio: 0.5,
                min_ratio: 0.1,
                max_ratio: 1.0,
                preserve_window_sizes: false,
                expand_single_column: false,
            },
            alignment: Alignment::Center,
            navigation,
            first: columns.next().unwrap(),
            rest: columns.collect(),
        }
    }

    #[test]
    fn columns_sit_side_by_side_and_rows_share_the_height() {
        let placed = strip(Navigation::Free, vec![
            column(1, Width::Fixed(400.), &[10, 11]),
            column(2, Width::Default, &[20]),
        ])
        .place();
        assert_eq!((placed.first.world_x, placed.first.width), (0., 400.));
        // Half the view, its gap counted: (1000 + 10) * 0.5 - 10.
        assert_eq!((placed.rest[0].world_x, placed.rest[0].width), (410., 495.));
        assert_eq!(placed.frames, vec![
            (10, Rect {
                x: 0.,
                y: 50.,
                w: 400.,
                h: 390.
            }),
            (11, Rect {
                x: 0.,
                y: 460.,
                w: 400.,
                h: 390.
            }),
            (20, Rect {
                x: 410.,
                y: 50.,
                w: 495.,
                h: 800.
            }),
        ]);
    }

    #[test]
    fn the_first_column_anchors_left_the_last_right_and_the_rest_as_aligned() {
        let columns = || {
            vec![
                column(1, Width::Fixed(300.), &[1]),
                column(2, Width::Fixed(300.), &[2]),
                column(3, Width::Fixed(300.), &[3]),
            ]
        };
        let placed = strip(Navigation::Anchored, columns()).place();
        // Middle: centred, 310 - (1000 - 300) / 2; last: its right edge, 620 + 300 - 1000.
        assert_eq!(
            [
                placed.first.anchor,
                placed.rest[0].anchor,
                placed.rest[1].anchor
            ],
            [0., -40., -80.]
        );
        let offsets: Vec<_> = std::iter::once(placed.snaps.0)
            .chain(placed.snaps.1)
            .map(|snap| (snap.offset, snap.column))
            .collect();
        assert_eq!(offsets, vec![(0., 1), (-40., 2), (-80., 3)]);
        assert_eq!(placed.bounds, (-80., 0.));
        // An only column is aligned like a middle one.
        let only = strip(Navigation::Anchored, vec![column(1, Width::Fixed(300.), &[1])]).place();
        assert_eq!(only.first.anchor, -350.);
    }

    #[test]
    fn free_snaps_are_the_two_ends_then_every_edge_strictly_between() {
        let placed = strip(Navigation::Free, vec![
            column(1, Width::Fixed(600.), &[1]),
            column(2, Width::Fixed(600.), &[2]),
            column(3, Width::Fixed(600.), &[3]),
        ])
        .place();
        let offsets: Vec<_> = std::iter::once(placed.snaps.0)
            .chain(placed.snaps.1)
            .map(|snap| (snap.offset, snap.column))
            .collect();
        // Ends 0 and 1220 + 600 - 1000; between: column 2's left 610 and right 210, column 3's
        // left 1220 is past the end, column 1's right -400 before the start.
        assert_eq!(offsets, vec![(0., 1), (820., 3), (610., 2), (210., 2)]);
        assert_eq!(placed.bounds, (0., 820.));
    }

    #[test]
    fn a_default_column_preserves_a_locked_width_and_an_only_column_may_fill_the_view() {
        let mut locked = strip(Navigation::Free, vec![column(1, Width::Default, &[1])]);
        locked.first.first.constraints.locked_width = 290.;
        assert_eq!(locked.place().first.width, 495.);
        locked.widths.preserve_window_sizes = true;
        assert_eq!(locked.place().first.width, 290.);
        locked.widths.expand_single_column = true;
        assert_eq!(locked.place().first.width, 1000.);
    }

    #[test]
    fn a_reported_minimum_holds_in_a_scrolling_column_even_when_advisory() {
        let mut s = strip(Navigation::Free, vec![column(1, Width::Fixed(300.), &[1])]);
        s.first.first.constraints.sizing = Sizing::new(true, false, Size::reported(450., 0.));
        assert_eq!(s.place().first.width, 450.);
    }

    fn narrow(navigation: Navigation) -> Placed<u32, u32> {
        strip(navigation, vec![
            column(1, Width::Fixed(300.), &[1]),
            column(2, Width::Fixed(300.), &[2]),
            column(3, Width::Fixed(300.), &[3]),
        ])
        .place()
    }

    #[test]
    fn a_tie_goes_to_the_earlier_point_the_active_column_or_the_leftmost_as_settle_says() {
        // Anchored points 0 (column 1), -40 (2), -80 (3); -20 is as near 0 as -40.
        let placed = narrow(Navigation::Anchored);
        let settle = |how| placed.settle(VIEW.w, -20., 0., how);
        assert_eq!(settle(Settle::Anchored), Settled { column: 1, offset: 0. });
        assert_eq!(settle(Settle::AnchoredTowards(2)), Settled {
            column: 2,
            offset: -40.
        });
        assert_eq!(settle(Settle::AnchoredTowards(3)), Settled {
            column: 1,
            offset: 0.
        });
        // Niri takes the leftmost, -40, then walks back to the furthest column wholly in view.
        assert_eq!(settle(Settle::Niri), Settled { column: 1, offset: -40. });
    }

    #[test]
    fn niri_shows_the_furthest_column_wholly_in_view_in_the_direction_of_travel() {
        // Free points: 0 (column 1) and the right end -80 (column 3); all three fit the view.
        let placed = narrow(Navigation::Free);
        assert_eq!(placed.settle(VIEW.w, 0., -50., Settle::Niri), Settled {
            column: 3,
            offset: 0.
        });
        assert_eq!(placed.settle(VIEW.w, -80., 0., Settle::Niri), Settled {
            column: 1,
            offset: -80.
        });
        // Wide columns: the nearest point's column is the only one wholly in view.
        let wide = strip(Navigation::Free, vec![
            column(1, Width::Fixed(600.), &[1]),
            column(2, Width::Fixed(600.), &[2]),
            column(3, Width::Fixed(600.), &[3]),
        ])
        .place();
        assert_eq!(wide.settle(VIEW.w, 415., 0., Settle::Niri), Settled {
            column: 2,
            offset: 610.
        });
        assert_eq!(wide.settle(VIEW.w, 415., 900., Settle::Niri), Settled {
            column: 2,
            offset: 610.
        });
    }

    #[test]
    fn fitting_keeps_the_camera_where_the_whole_column_shows() {
        let placed = narrow(Navigation::Free);
        let middle = placed.spot(2).unwrap();
        // Column 2 spans 310..610: offsets -390..310 show it whole.
        assert_eq!(middle.fit(100., VIEW.w), 100.);
        assert_eq!(middle.fit(500., VIEW.w), 310.);
        assert_eq!(middle.fit(-900., VIEW.w), -390.);
        assert_eq!(middle.centered(VIEW.w), -40.);
        assert_eq!(placed.spot(9), None);
    }

    /// The case that showed the core adding a column's width and gap in another order than
    /// upstream (`(x + width) + gap`): the third column sat one ulp short of upstream's
    /// 1653.7266638402023, and every column after it, its frames, snaps and bounds with it.
    #[test]
    fn a_columns_x_adds_its_width_and_gap_in_upstreams_order() {
        let widths = [
            1494.0,
            138.2473041445788,
            359.0,
            138.2473041445788,
            138.2473041445788,
        ];
        let mut columns = widths
            .iter()
            .enumerate()
            .map(|(id, &width)| column(id as u32, Width::Fixed(width), &[id as u32]));
        let first = columns.next().unwrap();
        let placed = Strip {
            tiling: Rect {
                x: 318.,
                y: 883.,
                w: 2969.,
                h: 125.,
            },
            column_gap: 10.739679847811766,
            first,
            rest: columns.collect(),
            ..strip(Navigation::Anchored, vec![column(0, Width::Default, &[0])])
        }
        .place();
        assert_eq!(
            placed.rest[1].world_x.to_bits(),
            1653.7266638402023_f64.to_bits()
        );
        assert_eq!(
            placed.rest[3].world_x.to_bits(),
            2172.4533276804045_f64.to_bits()
        );
    }
}
