//! A strip of persistent columns in world coordinates, translated by one camera.
//! View targets follow niri's fit/center and gesture snap semantics. Ordinary layout
//! animation remains in the reactor; touchpad release belongs to the camera.
use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use super::{EditResult, Focus, FocusResult, GroupEdit, LayoutEdit, WindowChange, WindowQuery};
use crate::actor::app::{WindowId, pid_t};
use crate::common::collections::{HashMap, HashSet};
use crate::common::config::{
    GapSettings, ScrollingAlignment, ScrollingFocusNavigationStyle, ScrollingLayoutSettings,
};
use crate::layout_engine::systems::constraints::{AxisConstraints, solve_axis_lengths};
use crate::layout_engine::systems::{LayoutSystem, WindowLayoutConstraints};
use crate::layout_engine::utils::compute_tiling_area;
use crate::layout_engine::{Direction, LayoutId, ResizeOrientation, WindowDropAction};
use crate::model::hidden_window_placement::HiddenWindowPlacement;
use crate::sys::geometry::Round;

mod projection;
#[cfg(feature = "roc-core")]
mod roc_columns;
#[cfg(feature = "roc-core")]
pub(crate) use roc_columns::columns_roc_reseeds;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(transparent)]
struct ColumnId(u64);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
enum ColumnWidth {
    #[default]
    Default,
    Proportion(f64),
    Fixed(f64),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
struct Row {
    window: WindowId,
    height: f64,
}

#[derive(Serialize, Clone, Debug)]
struct Column {
    id: ColumnId,
    rows: Vec<Row>,
    active_window: usize,
    width: ColumnWidth,
}

impl Column {
    fn windows(&self) -> impl DoubleEndedIterator<Item = &WindowId> + ExactSizeIterator + Clone {
        self.rows.iter().map(|row| &row.window)
    }

    fn resolved_width(
        &self,
        view: f64,
        gap: f64,
        settings: &ScrollingLayoutSettings,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        inactive: &BTreeSet<WindowId>,
    ) -> f64 {
        let ratio = match self.width {
            ColumnWidth::Fixed(width) => return width.max(1.0),
            ColumnWidth::Proportion(ratio) => ratio,
            ColumnWidth::Default => self
                .windows()
                .find(|wid| !inactive.contains(*wid))
                .and_then(|wid| constraints.get(wid))
                .map(|c| c.locked_width)
                .filter(|w| settings.preserve_window_sizes && w.is_finite() && *w > 0.0)
                .map_or(settings.column_width_ratio, |w| width_ratio(view, gap, w)),
        };
        proportional_width(view, gap, clamp_ratio(ratio, settings))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ViewBookmark {
    column: ColumnId,
    // Relative to the bookmarked column so edits to preceding columns reconcile naturally.
    relative_offset: f64,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(from = "f64")]
enum Viewport {
    #[default]
    Uninitialized,
    Static(f64),
    // Keep the previous position until the presenter starts the new target.
    Pending {
        from: f64,
        target: f64,
    },
    Gesture(f64),
    Animation(CameraSpring),
}

impl Viewport {
    // Layout queries use the requested offset; the presenter also needs the
    // previous offset until it starts the animation.
    fn offset(&self) -> f64 {
        match self {
            Self::Uninitialized => 0.0,
            Self::Static(offset) | Self::Gesture(offset) => *offset,
            Self::Pending { target, .. } => *target,
            Self::Animation(spring) => spring.current,
        }
    }

    fn start_offset(&self) -> f64 {
        match self {
            Self::Pending { from, .. } => *from,
            _ => self.offset(),
        }
    }

    fn target(&self) -> f64 {
        match self {
            Self::Animation(spring) => spring.target,
            _ => self.offset(),
        }
    }

    fn finish(&mut self) {
        if !matches!(self, Self::Gesture(_)) {
            *self = Self::Static(self.target());
        }
    }

    fn set_target(&mut self, target: f64) {
        let from = self.start_offset();
        *self = if from == target {
            Self::Static(target)
        } else {
            Self::Pending { from, target }
        };
    }

    fn sample(&mut self, now: Instant, scale: f64) -> Option<bool> {
        if let Self::Pending { target, .. } = *self {
            *self = Self::Static(target);
            return Some(false);
        }
        let Self::Animation(spring) = self else {
            return None;
        };
        // Bounds constrain the destination. After a column closes, the previous
        // position can lie outside them and must remain visible while returning.
        if spring.sample(now, scale) {
            *self = Self::Static(spring.target);
            Some(false)
        } else {
            Some(true)
        }
    }

    fn rebase(&mut self, delta: f64) {
        match self {
            Self::Uninitialized => {}
            Self::Static(offset) | Self::Gesture(offset) => *offset += delta,
            Self::Pending { from, target } => {
                *from += delta;
                *target += delta;
            }
            Self::Animation(spring) => {
                spring.from += delta;
                spring.target += delta;
                spring.current += delta;
            }
        }
    }
}

/// Niri's default horizontal-view-movement: mass 1, critical damping, stiffness 800.
#[derive(Clone, Debug)]
pub(crate) struct CameraSpring {
    from: f64,
    target: f64,
    velocity: f64,
    current: f64,
    current_velocity: f64,
    started: Instant,
    sampled: Instant,
}
impl CameraSpring {
    pub(crate) fn new(from: f64, target: f64, velocity: f64, started: Instant) -> Self {
        Self {
            from,
            target,
            velocity,
            current: from,
            current_velocity: velocity,
            started,
            sampled: started,
        }
    }

    pub(crate) fn current(&self) -> f64 { self.current }

    pub(crate) fn position_velocity(&self, now: Instant) -> (f64, f64) {
        let t = now.saturating_duration_since(self.started).as_secs_f64();
        let omega = 800.0_f64.sqrt();
        let x0 = self.from - self.target;
        let b = omega * x0 + self.velocity;
        let envelope = (-omega * t).exp();
        (
            self.target + envelope * (x0 + b * t),
            envelope * (self.velocity - omega * b * t),
        )
    }

    pub(crate) fn sample(&mut self, now: Instant, scale: f64) -> bool {
        let now = now.max(self.sampled);
        let (position, velocity) = self.position_velocity(now);
        self.current = position;
        self.current_velocity = velocity;
        self.sampled = now;
        // Check both position and speed so crossing the target cannot finish early.
        (self.current - self.target).abs() <= 0.25 / scale
            && velocity.abs() / 800.0_f64.sqrt() <= 0.25 / scale
    }
}

impl From<f64> for Viewport {
    fn from(offset: f64) -> Self { Self::Static(if offset.is_finite() { offset } else { 0.0 }) }
}

/// Semantic release result; animation, when enabled, belongs to the viewport.
#[derive(Clone, Copy, Debug)]
pub struct ViewportRelease {
    pub window: WindowId,
    pub offset: f64,
    pub from_offset: f64,
    pub velocity: f64,
}

#[derive(Clone, Copy, Debug)]
struct ColumnGeometry {
    id: ColumnId,
    world_x: f64,
    width: f64,
}

/// A camera offset the camera may rest at, and the column it shows.
#[derive(Clone, Copy, Debug)]
struct SnapPoint {
    offset: f64,
    column: ColumnId,
}

#[derive(Clone, Debug)]
struct Geometry {
    screen: CGRect,
    tiling: CGRect,
    gaps: GapSettings,
    constraints: HashMap<WindowId, WindowLayoutConstraints>,
    constraint_revision: Option<u64>,
    columns: Vec<ColumnGeometry>,
    frames: Vec<(WindowId, CGRect)>,
    bounds: (f64, f64),
    snaps: Vec<SnapPoint>,
}

/// Prepared world frames and temporary camera motion. Built only at semantic
/// boundaries; sampling does not enter the layout engine.
#[derive(Debug)]
pub(crate) struct ViewportPresentation {
    viewport: Viewport,
    motion: Arc<Mutex<MotionHistory>>,
    bounds: (f64, f64),
    width: f64,
    tiling: CGRect,
    screen: CGRect,
}

/// Rendered scalar state, read only at semantic boundaries. Motion samples stay
/// shared so release keeps the exact touchpad velocity history without cloning
/// a camera or copying history on each display refresh.
#[derive(Clone, Debug)]
pub(crate) struct PresentedViewport {
    pub offset: f64,
    pub velocity: f64,
    pub target: f64,
    pub gesturing: bool,
    pub timestamp: Instant,
    motion: Arc<Mutex<MotionHistory>>,
}

impl ViewportPresentation {
    pub fn offset(&self) -> f64 { self.viewport.start_offset() }

    pub fn gesturing(&self) -> bool { matches!(self.viewport, Viewport::Gesture(_)) }

    pub fn animated(&self) -> bool { matches!(self.viewport, Viewport::Animation(_)) }

    pub fn finish(&mut self) { self.viewport.finish(); }

    pub fn target(&self) -> f64 { self.viewport.target() }

    pub fn retarget(&mut self, from: f64, velocity: f64, now: Instant) {
        let target = self.target();
        self.viewport = Viewport::Animation(CameraSpring::new(from, target, velocity, now));
    }

    pub fn position_velocity(&self, now: Instant) -> (f64, f64) {
        match &self.viewport {
            Viewport::Animation(s) => s.position_velocity(s.sampled.max(now)),
            _ => (self.offset(), 0.0),
        }
    }

    pub fn update(&mut self, delta: f64, time: Duration) {
        if let Viewport::Gesture(offset) = &mut self.viewport {
            self.motion.lock().update(offset, self.bounds, delta * self.width, time);
        }
    }

    pub fn sample(&mut self, now: Instant, scale: f64) -> bool {
        self.viewport.sample(now, scale).unwrap_or(self.gesturing())
    }

    pub fn snapshot(&self, now: Instant) -> PresentedViewport {
        PresentedViewport {
            offset: self.offset(),
            velocity: match &self.viewport {
                Viewport::Animation(s) => s.current_velocity,
                _ => 0.0,
            },
            target: self.target(),
            gesturing: self.gesturing(),
            timestamp: now,
            motion: self.motion.clone(),
        }
    }

    pub fn target_frame(&self, frame: CGRect, fixed: bool, scale: f64) -> CGRect {
        self.frame_at_offset(frame, fixed, scale, self.target())
    }

    pub fn frame_at_offset(
        &self,
        mut frame: CGRect,
        fixed: bool,
        scale: f64,
        offset: f64,
    ) -> CGRect {
        if !fixed {
            frame = translate_frame(frame, offset, self.tiling, self.screen, true);
        }
        frame.origin.x = (frame.origin.x * scale).round() / scale;
        frame.origin.y = (frame.origin.y * scale).round() / scale;
        frame
    }
}

fn translate_frame(
    mut frame: CGRect,
    offset: f64,
    tiling: CGRect,
    screen: CGRect,
    park: bool,
) -> CGRect {
    frame.origin.x += tiling.origin.x - offset;
    if park {
        if frame.max().x <= tiling.origin.x {
            frame.origin.x = screen.origin.x - frame.size.width + HiddenWindowPlacement::REVEAL_PX;
        } else if frame.origin.x >= tiling.max().x {
            frame.origin.x = screen.max().x - HiddenWindowPlacement::REVEAL_PX;
        }
    }
    frame
}

fn proportional_width(view: f64, gap: f64, ratio: f64) -> f64 {
    ((view + gap) * ratio - gap).max(1.0)
}

fn width_ratio(view: f64, gap: f64, width: f64) -> f64 { (width + gap) / (view + gap).max(1.0) }

fn clamp_ratio(ratio: f64, settings: &ScrollingLayoutSettings) -> f64 {
    let min = settings.min_column_width_ratio.max(0.05);
    let max = settings.max_column_width_ratio.max(min);
    if ratio.is_finite() {
        ratio.clamp(min, max)
    } else {
        min
    }
}

// Adapted from niri's compute_new_view_offset, using an absolute camera.
// Rift's outer gaps already reserve padding, including at both strip boundaries.
fn fit_offset(current: f64, view: f64, column: ColumnGeometry) -> f64 {
    let left = column.world_x;
    if column.width >= view {
        return left;
    }
    current.clamp(left + column.width - view, left)
}

fn centered_offset(view: f64, column: ColumnGeometry) -> f64 {
    column.world_x - (view - column.width).max(0.0) / 2.0
}

/// What the core's `translate` takes to show a placed strip: its world frames, the camera (the
/// geometry's screen and tiling area, the camera's `offset`, parking or not) and the fullscreen
/// windows, as `translate_frames` reads them.
fn shown(
    view: &ArrangementView,
    g: &Geometry,
    offset: f64,
    park: bool,
) -> (
    Vec<(WindowId, rift_core::plain::Rect)>,
    rift_core::layout::scrolling::Camera,
    Vec<(WindowId, rift_core::layout::scrolling::Fullscreen)>,
) {
    use rift_core::layout::scrolling::{Camera, Fullscreen, Parking};

    use crate::layout_engine::core_rect;
    let world = g.frames.iter().map(|&(window, frame)| (window, core_rect(frame))).collect();
    let camera = Camera {
        screen: core_rect(g.screen),
        tiling: core_rect(g.tiling),
        offset,
        parking: if park {
            Parking::Park
        } else {
            Parking::InPlace
        },
    };
    let fullscreen = view
        .fullscreen
        .iter()
        .map(|(&window, mode)| {
            (window, match mode {
                FullscreenMode::Screen => Fullscreen::Screen,
                FullscreenMode::WithinGaps => Fullscreen::WithinGaps,
            })
        })
        .collect();
    (world, camera, fullscreen)
}

/// A scrolling layout's strip as the core's geometry takes it: the columns with a window that
/// takes part, each with those windows (a window with no constraints is resizable), as
/// `Geometry::build` reads them (none: no window takes part). Arguments as `build` takes them.
fn strip(
    view: &ArrangementView,
    settings: &ScrollingLayoutSettings,
    inactive: &BTreeSet<WindowId>,
    screen: CGRect,
    constraints: &HashMap<WindowId, WindowLayoutConstraints>,
    gaps: &GapSettings,
) -> Option<rift_core::layout::scrolling::Strip<WindowId, u64>> {
    use rift_core::layout::scrolling::{self as core, Alignment, Navigation, Width, Widths};

    use crate::layout_engine::core_geometry::core_constraints;
    use crate::layout_engine::{core_gaps, core_rect};
    let row = |row: &Row| core::Row {
        window: row.window,
        constraints: core_constraints(constraints.get(&row.window).copied().unwrap_or(
            WindowLayoutConstraints {
                sizing: super::Sizing::Resizable,
                ..Default::default()
            },
        )),
        weight: row.height,
    };
    let mut columns = view.columns.iter().filter_map(|column| {
        let mut rows = column.rows.iter().filter(|row| !inactive.contains(&row.window)).map(row);
        let first = rows.next()?;
        Some(core::Column {
            id: column.id.0,
            width: match column.width {
                ColumnWidth::Default => Width::Default,
                ColumnWidth::Proportion(ratio) => Width::Proportion(ratio),
                ColumnWidth::Fixed(width) => Width::Fixed(width),
            },
            first,
            rest: rows.collect(),
        })
    });
    let first = columns.next()?;
    Some(core::Strip {
        tiling: rift_core::layout::tiling_area(core_rect(screen), core_gaps(gaps)),
        column_gap: gaps.inner.horizontal,
        row_gap: gaps.inner.vertical,
        widths: Widths {
            ratio: settings.column_width_ratio,
            min_ratio: settings.min_column_width_ratio,
            max_ratio: settings.max_column_width_ratio,
            preserve_window_sizes: settings.preserve_window_sizes,
            expand_single_column: settings.expand_single_column,
        },
        alignment: match settings.alignment {
            ScrollingAlignment::Left => Alignment::Left,
            ScrollingAlignment::Center => Alignment::Center,
            ScrollingAlignment::Right => Alignment::Right,
        },
        navigation: match settings.focus_navigation_style {
            ScrollingFocusNavigationStyle::Anchored => Navigation::Anchored,
            ScrollingFocusNavigationStyle::Niri => Navigation::Free,
        },
        first,
        rest: columns.collect(),
    })
}

impl Geometry {
    /// The strip's geometry: placed by the core (policy (a)), with the system's own placement
    /// ([`shadow_placement`]) its shadow.
    fn build(
        view: &ArrangementView,
        settings: &ScrollingLayoutSettings,
        inactive: &BTreeSet<WindowId>,
        screen: CGRect,
        constraints: HashMap<WindowId, WindowLayoutConstraints>,
        constraint_revision: Option<u64>,
        gaps: GapSettings,
        shadow: crate::layout_engine::Shadow,
    ) -> Self {
        use crate::layout_engine::cg_rect;
        let tiling = compute_tiling_area(screen, &gaps);
        let placed = crate::layout_engine::core_geometry::decide_strip(
            strip(view, settings, inactive, screen, &constraints, &gaps),
            shadow,
            || shadow_placement(view, settings, inactive, tiling, &constraints, &gaps),
        );
        Self {
            screen,
            tiling,
            gaps,
            constraints,
            constraint_revision,
            columns: placed
                .columns
                .into_iter()
                .map(|(id, world_x, width)| ColumnGeometry {
                    id: ColumnId(id),
                    world_x,
                    width,
                })
                .collect(),
            frames: placed.frames.into_iter().map(|(w, frame)| (w, cg_rect(frame))).collect(),
            bounds: placed.bounds,
            snaps: placed
                .snaps
                .into_iter()
                .map(|(offset, column)| SnapPoint {
                    offset,
                    column: ColumnId(column),
                })
                .collect(),
        }
    }

    fn same_projected_constraints(
        &self,
        current: &HashMap<WindowId, WindowLayoutConstraints>,
    ) -> bool {
        self.frames
            .iter()
            .all(|(window, _)| self.constraints.get(window) == current.get(window))
    }

    fn has_constraints(
        &self,
        current: &HashMap<WindowId, WindowLayoutConstraints>,
        revision: Option<u64>,
    ) -> bool {
        match (self.constraint_revision, revision) {
            (Some(saved), Some(current)) => saved == current,
            _ => self.constraints == *current,
        }
    }

    fn column(&self, id: ColumnId) -> Option<ColumnGeometry> {
        self.columns.iter().find(|c| c.id == id).copied()
    }

    fn anchor_offset(&self, index: usize, alignment: ScrollingAlignment) -> f64 {
        anchor_offset(&self.columns, index, self.tiling.size.width, alignment)
    }
}

/// Where the scrolling system places a strip itself (`tiling` the screen's tiling area): since
/// the core decides (policy (a)), the shadow `Geometry::build` holds the core's placement to.
/// Arguments as `build` takes them.
fn shadow_placement(
    view: &ArrangementView,
    settings: &ScrollingLayoutSettings,
    inactive: &BTreeSet<WindowId>,
    tiling: CGRect,
    constraints: &HashMap<WindowId, WindowLayoutConstraints>,
    gaps: &GapSettings,
) -> crate::layout_engine::core_geometry::StripPlacement {
    use crate::layout_engine::core_rect;
    let mut columns = Vec::with_capacity(view.columns.len());
    let mut frames = Vec::with_capacity(view.columns.iter().map(|c| c.rows.len()).sum());
    let constraint = |wid: &WindowId| {
        constraints
            .get(wid)
            .copied()
            .unwrap_or(WindowLayoutConstraints {
                sizing: super::Sizing::Resizable,
                ..Default::default()
            })
            .normalized()
    };
    let mut x = 0.0;
    let active_columns = if inactive.is_empty() {
        view.columns.len()
    } else {
        view.columns
            .iter()
            .filter(|c| c.windows().any(|w| !inactive.contains(w)))
            .count()
    };
    for column in &view.columns {
        let active_rows = column.rows.iter().filter(|row| !inactive.contains(&row.window));
        let mut rows = Vec::with_capacity(column.rows.len());
        let mut min: f64 = 1.0;
        let mut fixed: f64 = 0.0;
        let mut max = f64::INFINITY;
        let mut flexible = false;
        for row in active_rows.clone() {
            let c = constraint(&row.window);
            let fixed_width = c.fixed_for_axis(true);
            min = min.max(c.min_width);
            fixed = fixed.max(fixed_width.unwrap_or(0.0));
            flexible |= fixed_width.is_none();
            if c.max_width > 0.0 {
                max = max.min(c.max_width);
            }
            let fixed_height = c.fixed_for_axis(false);
            rows.push(AxisConstraints {
                min: c.min_height,
                fixed: fixed_height,
                max: (c.max_height > 0.0).then_some(c.max_height),
                weight: (row.height - c.min_height).max(0.001),
                can_grow: fixed_height.is_none(),
            });
        }
        if rows.is_empty() {
            continue;
        }
        let base = if settings.expand_single_column && active_columns == 1 {
            tiling.size.width
        } else {
            column.resolved_width(
                tiling.size.width,
                gaps.inner.horizontal,
                settings,
                constraints,
                inactive,
            )
        };
        let width = if flexible {
            base.min(max).max(min).max(fixed)
        } else {
            min.max(fixed)
        };
        columns.push(ColumnGeometry {
            id: column.id,
            world_x: x,
            width,
        });
        let available =
            (tiling.size.height - gaps.inner.vertical * (rows.len() - 1) as f64).max(0.0);
        let heights = solve_axis_lengths(rows, available);
        let mut y = tiling.origin.y;
        for (row, height) in active_rows.zip(heights) {
            let wid = row.window;
            let mut size = CGSize::new(width, height);
            let c = constraint(&wid);
            size.width = c.fixed_for_axis(true).unwrap_or(width).max(c.min_width).min(width);
            size.height = c.fixed_for_axis(false).unwrap_or(height).max(c.min_height).min(height);
            if c.max_width > 0.0 {
                size.width = size.width.min(c.max_width);
            }
            if c.max_height > 0.0 {
                size.height = size.height.min(c.max_height);
            }
            frames.push((wid, core_rect(CGRect::new(CGPoint::new(x, y), size))));
            y += height + gaps.inner.vertical;
        }
        x += width + gaps.inner.horizontal;
    }
    // The bounds are resting positions, not topology-derived column indices.
    let snaps = snap_points(&columns, tiling.size.width, settings);
    let bounds = if snaps.is_empty() {
        (0.0, 0.0)
    } else {
        snaps.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), snap| {
            (min.min(snap.offset), max.max(snap.offset))
        })
    };
    crate::layout_engine::core_geometry::StripPlacement {
        columns: columns.iter().map(|c| (c.id.0, c.world_x, c.width)).collect(),
        frames,
        snaps: snaps.iter().map(|snap| (snap.offset, snap.column.0)).collect(),
        bounds,
    }
}

/// The shadow's resting points over its `columns` in a view `view` wide.
fn snap_points(
    columns: &[ColumnGeometry],
    view: f64,
    settings: &ScrollingLayoutSettings,
) -> Vec<SnapPoint> {
    let Some(first) = columns.first() else {
        return Vec::new();
    };
    let anchored = settings.focus_navigation_style == ScrollingFocusNavigationStyle::Anchored;
    if anchored {
        return columns
            .iter()
            .enumerate()
            .map(|(index, column)| SnapPoint {
                offset: anchor_offset(columns, index, view, settings.alignment),
                column: column.id,
            })
            .collect();
    }
    let last = columns.last().unwrap();
    let left = first.world_x;
    let right = last.world_x + last.width - view;
    let mut points = vec![SnapPoint { offset: left, column: first.id }, SnapPoint {
        offset: right,
        column: last.id,
    }];
    for column in columns {
        for offset in [column.world_x, column.world_x + column.width - view] {
            if left < offset && offset < right {
                points.push(SnapPoint { offset, column: column.id });
            }
        }
    }
    points
}

fn anchor_offset(
    columns: &[ColumnGeometry],
    index: usize,
    view: f64,
    alignment: ScrollingAlignment,
) -> f64 {
    let column = columns[index];
    if column.width >= view {
        return column.world_x;
    }
    // Preserve Rift's anchored first/last-column policy.
    if columns.len() > 1 {
        if index == 0 {
            return column.world_x;
        }
        if index + 1 == columns.len() {
            return column.world_x + column.width - view;
        }
    }
    match alignment {
        ScrollingAlignment::Left => column.world_x,
        ScrollingAlignment::Center => centered_offset(view, column),
        ScrollingAlignment::Right => column.world_x + column.width - view,
    }
}

/// Recent camera deltas in pixels, independent of viewport rebasing.
#[derive(Clone, Debug, Default)]
struct MotionHistory {
    samples: VecDeque<(Duration, f64)>,
    overscroll: f64,
}
impl MotionHistory {
    fn push(&mut self, delta: f64, time: Duration) -> bool {
        if self.samples.back().is_some_and(|(last, _)| time < *last) {
            return false;
        }
        // macOS reports stationary contacts continuously; retain one fresh idle
        // anchor so that a pause before a new flick does not dilute that flick.
        if delta == 0.0
            && let Some((last, 0.0)) = self.samples.back_mut()
        {
            *last = time;
        } else {
            if self.samples.len() == 64 {
                self.samples.pop_front();
            }
            self.samples.push_back((time, delta));
        }
        while self
            .samples
            .front()
            .is_some_and(|(first, _)| time.saturating_sub(*first) > Duration::from_millis(150))
        {
            self.samples.pop_front();
        }
        true
    }

    fn update(
        &mut self,
        offset: &mut f64,
        bounds: (f64, f64),
        delta: f64,
        time: Duration,
    ) -> Option<f64> {
        if !delta.is_finite() {
            return None;
        }
        let raw = *offset + self.overscroll + delta;
        let bounded = raw.clamp(bounds.0, bounds.1);
        let moved = bounded - *offset;
        if !self.push(moved, time) {
            return None;
        }
        self.overscroll = raw - bounded;
        *offset = bounded;
        Some(moved)
    }

    fn velocity(&self) -> f64 {
        let (Some(&(first, _)), Some(&(last, _))) = (self.samples.front(), self.samples.back())
        else {
            return 0.0;
        };
        let dt = last.saturating_sub(first).as_secs_f64();
        if dt > 0.0 {
            self.samples.iter().map(|(_, delta)| delta).sum::<f64>() / dt
        } else {
            0.0
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum FullscreenMode {
    Screen,
    WithinGaps,
}

/// A layout's arrangement in rift's shape, as the camera reads it: computed from the core's at
/// each entry point ([`projection::view`]) and passed down, never kept; also the shape a saved
/// layout is read in, before the core is seeded from it.
#[derive(Clone, Debug, Default)]
struct ArrangementView {
    columns: Vec<Column>,
    active_column: usize,
    next_column_id: u64,
    fullscreen: HashMap<WindowId, FullscreenMode>,
}

#[derive(Clone, Debug, Default)]
struct Camera {
    viewport: Viewport,
    motion: MotionHistory,
    transient_restore: Option<ViewBookmark>,
    fullscreen_restore: Option<ViewBookmark>,
    restored_view: Option<ViewBookmark>,
}

#[derive(Clone, Debug, Default)]
struct LayoutState {
    camera: Camera,
    geometry: Option<Geometry>,
}

// Store the camera relative to its active column as well as its absolute
// position. On another display, the first preparation rebases it using the new
// geometry. Serialize borrowed fields so saving does not clone geometry/maps.
/// What the camera's half of an edit keeps from before it (`ScrollingLayoutSystem::edited`).
struct BeforeEdit {
    view: ArrangementView,
    window: Option<WindowId>,
    id: Option<ColumnId>,
    old_x: Option<f64>,
}

/// A layout as it is saved: its arrangement (the core's, in rift's shape) beside its camera.
struct SavedLayout<'a> {
    view: ArrangementView,
    state: &'a LayoutState,
}

impl Serialize for SavedLayout<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let SavedLayout { view, state } = self;
        let mut stored = serializer.serialize_struct("LayoutState", 8)?;
        stored.serialize_field("columns", &view.columns)?;
        stored.serialize_field("active_column", &view.active_column)?;
        stored.serialize_field("next_column_id", &view.next_column_id)?;
        let current = state.camera.viewport.offset();
        let offset = if matches!(
            state.camera.viewport,
            Viewport::Gesture(_) | Viewport::Animation(_)
        ) {
            state
                .geometry
                .as_ref()
                .map_or(current, |g| current.clamp(g.bounds.0, g.bounds.1))
        } else {
            current
        };
        stored.serialize_field("viewport", &offset)?;
        let anchor = state.bookmark(view).or_else(|| state.camera.restored_view.clone()).map(
            |mut anchor| {
                anchor.relative_offset += offset - current;
                anchor
            },
        );
        stored.serialize_field("view_anchor", &anchor)?;
        stored.serialize_field("transient_restore", &state.camera.transient_restore)?;
        stored.serialize_field("fullscreen_restore", &state.camera.fullscreen_restore)?;
        stored.serialize_field("fullscreen_modes", &view.fullscreen)?;
        stored.end()
    }
}

// Released snapshots on main store selected, column_width_ratio, node_id,
// width_offset and width_overridden. Keep their migration at this boundary;
// current snapshots use the same flat representation.
#[derive(Deserialize, Default)]
#[serde(default)]
struct StoredLayoutState {
    columns: Vec<StoredColumn>,
    active_column: usize,
    next_column_id: u64,
    viewport: Viewport,
    view_anchor: Option<ViewBookmark>,
    transient_restore: Option<ViewBookmark>,
    fullscreen_restore: Option<ViewBookmark>,
    fullscreen_modes: HashMap<WindowId, FullscreenMode>,
    fullscreen: HashSet<WindowId>,
    fullscreen_within_gaps: HashSet<WindowId>,
    selected: Option<WindowId>,
    column_width_ratio: f64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct StoredColumn {
    #[serde(alias = "node_id")]
    id: u64,
    windows: Vec<WindowId>,
    rows: Vec<Row>,
    active_window: usize,
    width: ColumnWidth,
    height_weights: Vec<f64>,
    width_offset: f64,
    width_overridden: bool,
}

/// A layout as it is read: the arrangement saved (which seeds the core) and the camera.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(from = "StoredLayoutState")]
struct LoadedLayout {
    view: ArrangementView,
    state: LayoutState,
}

// The core is seeded from a saved slotmap through its own written form (`of_slotmap`).
impl Serialize for LoadedLayout {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        SavedLayout {
            view: self.view.clone(),
            state: &self.state,
        }
        .serialize(serializer)
    }
}

impl From<StoredLayoutState> for LoadedLayout {
    fn from(stored: StoredLayoutState) -> Self {
        let mut next = stored.next_column_id.max(2);
        for column in &stored.columns {
            next = next.max(column.id.saturating_add(2));
        }
        let base = if stored.column_width_ratio > 0.0 {
            stored.column_width_ratio
        } else {
            0.7
        };
        let columns = stored
            .columns
            .into_iter()
            .filter_map(|mut c| {
                if c.rows.is_empty() {
                    c.rows = c
                        .windows
                        .into_iter()
                        .enumerate()
                        .map(|(i, window)| Row {
                            window,
                            height: c.height_weights.get(i).copied().unwrap_or(1.0),
                        })
                        .collect();
                }
                if c.rows.is_empty() {
                    return None;
                }
                if c.id == 0 {
                    c.id = next;
                    next += 2;
                }
                let width = if c.width_overridden || c.width_offset != 0.0 {
                    ColumnWidth::Proportion(base + c.width_offset)
                } else {
                    c.width
                };
                Some(Column {
                    id: ColumnId(c.id),
                    active_window: c.active_window.min(c.rows.len() - 1),
                    rows: c.rows,
                    width,
                })
            })
            .collect::<Vec<_>>();
        let active_column = stored.active_column.min(columns.len().saturating_sub(1));
        let mut fullscreen = stored.fullscreen_modes;
        fullscreen.extend(
            stored
                .fullscreen_within_gaps
                .into_iter()
                .map(|w| (w, FullscreenMode::WithinGaps)),
        );
        fullscreen.extend(stored.fullscreen.into_iter().map(|w| (w, FullscreenMode::Screen)));
        // A mode for a window no column holds is a state only a saved file holds (the core keeps
        // a window's mode on its row): it would leave the layout with a fullscreen node forever.
        let held: HashSet<WindowId> =
            columns.iter().flat_map(|c| c.rows.iter().map(|r| r.window)).collect();
        fullscreen.retain(|window, _| {
            let kept = held.contains(window);
            if !kept {
                tracing::warn!(
                    ?window,
                    "scrolling-load: a fullscreen mode for a window in no column; dropped"
                );
            }
            kept
        });
        let mut loaded = Self {
            view: ArrangementView {
                columns,
                active_column,
                next_column_id: next,
                fullscreen,
            },
            state: LayoutState {
                camera: Camera {
                    viewport: stored.viewport,
                    transient_restore: stored.transient_restore,
                    fullscreen_restore: stored.fullscreen_restore,
                    restored_view: stored.view_anchor,
                    ..Camera::default()
                },
                geometry: None,
            },
        };
        if let Some(window) = stored.selected {
            loaded.view.activate(window);
        }
        loaded
    }
}

impl ArrangementView {
    fn locate(&self, wid: WindowId) -> Option<(usize, usize)> {
        self.columns.iter().enumerate().find_map(|(col, column)| {
            column.windows().position(|&w| w == wid).map(|row| (col, row))
        })
    }

    fn activate(&mut self, wid: WindowId) -> bool {
        let Some((col, row)) = self.locate(wid) else {
            return false;
        };
        self.active_column = col;
        self.columns[col].active_window = row;
        true
    }
}

impl LayoutState {
    fn fullscreen_frame(
        &self,
        view: &ArrangementView,
        geometry: &Geometry,
        window: WindowId,
    ) -> Option<CGRect> {
        Some(match view.fullscreen.get(&window)? {
            FullscreenMode::Screen => geometry.screen,
            FullscreenMode::WithinGaps => geometry.tiling,
        })
    }

    /// After a fullscreen ends: the view remembered before it, unless the active column still
    /// shows a fullscreen window; the column it shows, for the core to make active.
    fn restore_fullscreen_view(&mut self, view: &ArrangementView) -> Option<ColumnId> {
        let fullscreen = view
            .columns
            .get(view.active_column)
            .is_some_and(|column| column.windows().any(|wid| view.fullscreen.contains_key(wid)));
        if fullscreen {
            return None;
        }
        self.camera
            .fullscreen_restore
            .take()
            .and_then(|bookmark| self.restore(view, bookmark))
    }

    fn prepare_geometry(
        &mut self,
        settings: &ScrollingLayoutSettings,
        inactive: &BTreeSet<WindowId>,
        screen: CGRect,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        revision: Option<u64>,
        gaps: &GapSettings,
        shadow: crate::layout_engine::Shadow,
        view: impl FnOnce(bool) -> ArrangementView,
    ) {
        let same_constraints =
            self.geometry.as_ref().is_some_and(|g| g.has_constraints(constraints, revision));
        if let Some(g) = self.geometry.as_mut()
            && g.screen == screen
            && g.gaps == *gaps
            && (same_constraints
                || (!settings.preserve_window_sizes && g.same_projected_constraints(constraints)))
        {
            if !same_constraints {
                // Edits may introduce a member before the next engine projection.
                g.constraints.clone_from(constraints);
            }
            g.constraint_revision = revision;
            return;
        }
        // The arrangement this pass lays out: the core's, after it fixed preserved widths (a
        // window's locked width fixing its column's, with this pass's constraints).
        let view = view(settings.preserve_window_sizes);
        let bookmark = self.camera.restored_view.take().or_else(|| self.bookmark(&view));
        let initial = matches!(self.camera.viewport, Viewport::Uninitialized);
        let mut snapshot = self.geometry.take().map(|g| g.constraints).unwrap_or_default();
        if !same_constraints {
            snapshot.clone_from(constraints);
        }
        self.geometry = Some(Geometry::build(
            &view,
            settings,
            inactive,
            screen,
            snapshot,
            revision,
            gaps.clone(),
            shadow,
        ));
        if let Some(bookmark) = bookmark
            && let Some(column) = self.geometry.as_ref().unwrap().column(bookmark.column)
        {
            let old = self.camera.viewport.offset();
            self.camera.viewport.rebase(column.world_x + bookmark.relative_offset - old);
        }
        self.reconcile_camera_bounds();
        if initial {
            self.camera.viewport = Viewport::Static(0.0);
            if self.geometry.as_ref().unwrap().columns.len() == 1 {
                let g = self.geometry.as_ref().unwrap();
                self.camera.viewport = Viewport::Static(g.anchor_offset(0, settings.alignment));
            } else {
                self.reveal(&view, settings);
            }
            self.camera.viewport = Viewport::Static(self.camera.viewport.offset());
        }
    }

    fn reconcile_camera_bounds(&mut self) {
        if let (Viewport::Animation(spring), Some(g)) = (&mut self.camera.viewport, &self.geometry)
        {
            let target = spring.target.clamp(g.bounds.0, g.bounds.1);
            if target != spring.target {
                let now = Instant::now().max(spring.sampled);
                let (_, velocity) = spring.position_velocity(now);
                spring.from = spring.current;
                spring.velocity = velocity;
                spring.target = target;
                spring.started = now;
                spring.sampled = now;
            }
        }
    }

    fn bookmark(&self, view: &ArrangementView) -> Option<ViewBookmark> {
        let id = view.columns.get(view.active_column)?.id;
        let x = self.geometry.as_ref()?.column(id)?.world_x;
        Some(ViewBookmark {
            column: id,
            relative_offset: self.camera.viewport.offset() - x,
        })
    }

    /// The camera aimed back at a remembered view, if the strip still holds its column; that
    /// column, which the caller has the core make active (the camera never writes the
    /// arrangement).
    fn restore(&mut self, view: &ArrangementView, bookmark: ViewBookmark) -> Option<ColumnId> {
        if !view.columns.iter().any(|c| c.id == bookmark.column) {
            return None;
        }
        if let Some(g) = &self.geometry
            && let Some(column) = g.column(bookmark.column)
        {
            let view = g.tiling.size.width;
            self.camera.viewport.set_target(fit_offset(
                column.world_x + bookmark.relative_offset,
                view,
                column,
            ));
        }
        Some(bookmark.column)
    }

    fn reveal(&mut self, view: &ArrangementView, settings: &ScrollingLayoutSettings) {
        let Some(g) = &self.geometry else {
            return;
        };
        let Some(column) = view.columns.get(view.active_column).and_then(|c| g.column(c.id)) else {
            return;
        };
        let offset = if settings.focus_navigation_style == ScrollingFocusNavigationStyle::Anchored {
            g.anchor_offset(
                g.columns.iter().position(|c| c.id == column.id).unwrap(),
                settings.alignment,
            )
        } else {
            let target = match &self.camera.viewport {
                Viewport::Animation(s) => s.target,
                _ => self.camera.viewport.offset(),
            };
            fit_offset(target, g.tiling.size.width, column)
        };
        if matches!(&self.camera.viewport, Viewport::Animation(s) if s.target == offset) {
            return;
        }
        self.camera.viewport.set_target(offset);
    }

    /// The camera's half of an edit the core made: the geometry rebuilt from the arrangement
    /// after it (`view`), the camera rebased so the selection (`window`, or else the column it was
    /// in, `id`, at `old_x`) stays where it was on screen.
    fn rebuilt(
        &mut self,
        view: &ArrangementView,
        settings: &ScrollingLayoutSettings,
        inactive: &BTreeSet<WindowId>,
        window: Option<WindowId>,
        id: Option<ColumnId>,
        old_x: Option<f64>,
    ) {
        if let Some(old) = self.geometry.take() {
            self.geometry = Some(Geometry::build(
                view,
                settings,
                inactive,
                old.screen,
                old.constraints,
                old.constraint_revision,
                old.gaps,
                // An edit's rebuild is not a Space's layout pass.
                crate::layout_engine::Shadow::outside_a_pass(),
            ));
            let anchor = window
                .and_then(|wid| view.locate(wid))
                .map(|(col, _)| view.columns[col].id)
                .or_else(|| id.filter(|id| view.columns.iter().any(|c| c.id == *id)));
            if let (Some(old_x), Some(new)) =
                (old_x, anchor.and_then(|id| self.geometry.as_ref()?.column(id)))
            {
                self.camera.viewport.rebase(new.world_x - old_x);
            }
        }
        self.reconcile_camera_bounds();
        if view.columns.is_empty() {
            self.camera.viewport = Viewport::Static(0.0);
        }
    }
}

/// The scrolling layouts. The core's arrangements (`core`, `projection::Decider`) decide and
/// are saved; `layouts` holds each layout's camera and geometry, and the camera's projection of
/// the core's arrangement (refreshed from it after every change, never stepped on its own);
/// `inactive` is the projection of the core's windows left out.
#[derive(Deserialize, Clone, Debug)]
#[serde(from = "SavedSystem")]
pub struct ScrollingLayoutSystem {
    layouts: slotmap::SecondaryMap<LayoutId, LayoutState>,
    pub(super) settings: ScrollingLayoutSettings,
    core: projection::Decider,
}

impl Default for ScrollingLayoutSystem {
    fn default() -> Self {
        let settings = ScrollingLayoutSettings::default();
        ScrollingLayoutSystem {
            layouts: slotmap::SecondaryMap::default(),
            core: projection::decider(projection::CoreColumns::new(projection::settings(
                &settings,
            ))),
            settings,
        }
    }
}

/// A saved system: its layouts, read as rift reads them; the core is seeded from them.
#[derive(Deserialize, Default)]
#[serde(default)]
struct SavedSystem {
    layouts: slotmap::SlotMap<LayoutId, LoadedLayout>,
}

impl From<SavedSystem> for ScrollingLayoutSystem {
    fn from(saved: SavedSystem) -> Self {
        let core = projection::seed(&saved.layouts);
        ScrollingLayoutSystem {
            layouts: saved
                .layouts
                .into_iter()
                .map(|(layout, loaded)| (layout, loaded.state))
                .collect(),
            core: projection::decider(core),
            ..ScrollingLayoutSystem::default()
        }
    }
}

// Saving is the core's: each layout's arrangement as the core holds it, in rift's saved form
// (byte-compatible by construction), beside the layout's camera.
impl Serialize for ScrollingLayoutSystem {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        // The core's table, slot for slot (slotmap's saved form), each layout's arrangement
        // beside its camera.
        let layouts: Vec<_> =
            crate::model::slot_table::saved(self.core.layouts(), |layout, a| SavedLayout {
                view: projection::view(a),
                state: &self.layouts[layout],
            })
            .collect();
        let mut saved = serializer.serialize_struct("ScrollingLayoutSystem", 1)?;
        saved.serialize_field("layouts", &layouts)?;
        saved.end()
    }
}

impl ScrollingLayoutSystem {
    /// `layout`'s arrangement as the camera reads it, computed from the core's.
    fn view(&self, layout: LayoutId) -> Option<ArrangementView> {
        self.core.arrangement(layout).map(projection::view)
    }

    /// The camera brought round to show `layout`'s selected column.
    fn reveal(&mut self, layout: LayoutId) {
        if let (Some(view), Some(state)) = (self.view(layout), self.layouts.get_mut(layout)) {
            state.reveal(&view, &self.settings);
        }
    }

    /// The camera's choice made on the core: `column` made `layout`'s active one.
    fn activate_chosen(core: &mut projection::Decider, layout: LayoutId, column: ColumnId) {
        core.activate_column(layout, rift_core::layout::columns::ColumnId(column.0));
    }

    /// What the camera's half of an edit needs from before it: the arrangement, the core's
    /// selection, its column and where that column was.
    fn before_edit(&self, layout: LayoutId) -> Option<BeforeEdit> {
        let view = self.view(layout)?;
        let window = self.core.selected_window(layout);
        let id = view.columns.get(view.active_column).map(|c| c.id);
        let old_x = id
            .and_then(|id| self.layouts.get(layout)?.geometry.as_ref()?.column(id))
            .map(|c| c.world_x);
        Some(BeforeEdit { view, window, id, old_x })
    }

    /// The camera's half of an edit the core made: the geometry rebuilt from the arrangement
    /// after it, the camera holding the selection still; the arrangement after it.
    fn after_edit(&mut self, layout: LayoutId, before: &BeforeEdit) -> Option<ArrangementView> {
        let after = self.view(layout)?;
        let state = self.layouts.get_mut(layout)?;
        state.rebuilt(
            &after,
            &self.settings,
            self.core.left_out(),
            before.window,
            before.id,
            before.old_x,
        );
        Some(after)
    }

    /// An edit of `layout` the core makes (`edit`), then the camera's half of it. None for a
    /// layout the system does not hold.
    fn edited<R>(
        &mut self,
        layout: LayoutId,
        edit: impl FnOnce(&mut projection::Decider) -> R,
    ) -> Option<R> {
        let before = self.before_edit(layout)?;
        let answer = edit(&mut self.core);
        self.after_edit(layout, &before)?;
        Some(answer)
    }

    pub fn new(settings: &ScrollingLayoutSettings) -> Self {
        let mut system = Self::default();
        system.update_settings(settings);
        system
    }

    pub fn update_settings(&mut self, settings: &ScrollingLayoutSettings) {
        let mut settings = settings.clone();
        settings.per_display.clear();
        if self.settings == settings {
            return;
        }
        self.settings = settings;
        self.core.set_settings(projection::settings(&self.settings));
        let layouts: Vec<LayoutId> = self.layouts.keys().collect();
        for layout in layouts {
            self.edited(layout, |_| ());
        }
    }

    pub fn update_width_settings(&mut self, (ratio, min, max): (f64, f64, f64)) {
        if (ratio, min, max) == self.configured_widths() {
            return;
        }
        self.settings.column_width_ratio = ratio;
        self.settings.min_column_width_ratio = min;
        self.settings.max_column_width_ratio = max;
        self.core.set_settings(projection::settings(&self.settings));
        let layouts: Vec<LayoutId> = self.layouts.keys().collect();
        for layout in layouts {
            self.edited(layout, |_| ());
        }
    }

    pub(crate) fn configured_widths(&self) -> (f64, f64, f64) {
        (
            self.settings.column_width_ratio,
            self.settings.min_column_width_ratio,
            self.settings.max_column_width_ratio,
        )
    }

    fn insert(&mut self, layout: LayoutId, wid: WindowId) {
        let (Some(state), Some(view)) = (self.layouts.get(layout), self.view(layout)) else {
            return;
        };
        if self.core.contains_window(layout, wid) {
            return;
        }
        let bookmark = state.bookmark(&view);
        self.edited(layout, |core| core.insert(layout, wid));
        let view = self.view(layout).expect("the layout the edit was made in");
        let state = &mut self.layouts[layout];
        state.camera.transient_restore = bookmark;
        state.reveal(&view, &self.settings);
    }

    /// `wid` removed from `layout` by the core, then the camera's answer: the view remembered
    /// before the window came (when the selection's column went with it), the view before a
    /// fullscreen (when the window was fullscreen), or else the new selection revealed. Each
    /// column the camera chooses is made active on the core before the next removal.
    fn remove_from_layout(&mut self, layout: LayoutId, wid: WindowId) {
        // The view the camera remembers from before the window came: the core shows its column
        // if the selection's column goes (upstream's unified fallback, bafcc444).
        let remembered = self.layouts.get(layout).and_then(|state| {
            let bookmark = state.camera.transient_restore.as_ref()?;
            Some(rift_core::layout::columns::ColumnId(bookmark.column.0))
        });
        let Some(removal) =
            self.edited(layout, |core| core.remove_from(layout, wid, remembered)).flatten()
        else {
            return;
        };
        let Self { core, layouts, settings } = self;
        let state = &mut layouts[layout];
        let view = |core: &projection::Decider| {
            projection::view(core.arrangement(layout).expect("the layout the edit was made in"))
        };
        let restore = if removal.active_removed && removal.column_removed {
            state.camera.transient_restore.take()
        } else {
            None
        };
        // Restored only when it is the column the core showed.
        let shown = {
            let now = view(core);
            now.columns.get(now.active_column).map(|c| c.id)
        };
        let restored = restore
            .filter(|bookmark| Some(bookmark.column) == shown)
            .and_then(|bookmark| state.restore(&view(core), bookmark));
        let restored_fullscreen = removal
            .was_fullscreen
            .then(|| state.restore_fullscreen_view(&view(core)))
            .flatten();
        if let Some(column) = restored_fullscreen {
            Self::activate_chosen(core, layout, column);
        }
        if removal.active_removed && restored.is_none() && restored_fullscreen.is_none() {
            state.reveal(&view(core), settings);
        }
        let after = view(core);
        if state
            .camera
            .transient_restore
            .as_ref()
            .is_some_and(|b| !after.columns.iter().any(|c| c.id == b.column))
        {
            state.camera.transient_restore = None;
        }
    }

    pub(crate) fn prepare(
        &mut self,
        layout: LayoutId,
        screen: CGRect,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        revision: Option<u64>,
        gaps: &GapSettings,
        shadow: crate::layout_engine::Shadow,
    ) {
        let Self { core, layouts, settings } = self;
        // The windows left out as the pass starts (preserving widths leaves them as they are).
        let left_out = core.left_out().clone();
        if let Some(state) = layouts.get_mut(layout) {
            state.prepare_geometry(
                settings,
                &left_out,
                screen,
                constraints,
                revision,
                gaps,
                shadow,
                |preserve| {
                    if preserve {
                        projection::preserve(core, layout, screen, gaps, |window| {
                            constraints.get(&window).map(|c| c.locked_width)
                        });
                    }
                    projection::view(core.arrangement(layout).expect("a layout the core holds"))
                },
            );
        }
    }

    pub(crate) fn calculate_frames(
        &self,
        layout: LayoutId,
        screen: CGRect,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        revision: Option<u64>,
        gaps: &GapSettings,
        park: bool,
        shadow: crate::layout_engine::Shadow,
    ) -> Vec<(WindowId, CGRect)> {
        let (Some(state), Some(view)) = (self.layouts.get(layout), self.view(layout)) else {
            return Vec::new();
        };
        if let Some(g) = &state.geometry
            && g.screen == screen
            && g.gaps == *gaps
            && (g.has_constraints(constraints, revision)
                || (!self.settings.preserve_window_sizes
                    && g.same_projected_constraints(constraints)))
        {
            return Self::translate_frames(&view, state, g, park, shadow);
        }
        let mut camera = state.camera.clone();
        camera.restored_view = camera.restored_view.or_else(|| state.bookmark(&view));
        let mut fallback = LayoutState { camera, geometry: None };
        // This pass is not committed: the widths it preserves are a copy's.
        let mut preserved = None;
        fallback.prepare_geometry(
            &self.settings,
            self.core.left_out(),
            screen,
            constraints,
            revision,
            gaps,
            shadow,
            |preserve| {
                if !preserve {
                    return view.clone();
                }
                let mut core = self.core.clone();
                projection::preserve(&mut core, layout, screen, gaps, |window| {
                    constraints.get(&window).map(|c| c.locked_width)
                });
                let preserved_view =
                    projection::view(core.arrangement(layout).expect("a layout the core holds"));
                preserved = Some(preserved_view.clone());
                preserved_view
            },
        );
        let view = preserved.unwrap_or(view);
        Self::translate_frames(
            &view,
            &fallback,
            fallback.geometry.as_ref().unwrap(),
            park,
            shadow,
        )
    }

    /// The strip's frames on screen: the core's (policy (a)), with the system's own
    /// ([`Self::shadow_translation`]) their shadow.
    fn translate_frames(
        view: &ArrangementView,
        state: &LayoutState,
        g: &Geometry,
        park: bool,
        shadow: crate::layout_engine::Shadow,
    ) -> Vec<(WindowId, CGRect)> {
        let (world, camera, fullscreen) = shown(view, g, state.camera.viewport.offset(), park);
        crate::layout_engine::core_geometry::decide_translation(
            &world,
            camera,
            &fullscreen,
            shadow,
            || Self::shadow_translation(view, state, g, park),
        )
    }

    /// The strip's frames on screen as the scrolling system computes them itself: since the core
    /// decides (policy (a)), the shadow `translate_frames` holds the core's to.
    fn shadow_translation(
        view: &ArrangementView,
        state: &LayoutState,
        g: &Geometry,
        park: bool,
    ) -> Vec<(WindowId, CGRect)> {
        let offset = state.camera.viewport.offset();
        let frames: Vec<(WindowId, CGRect)> = g
            .frames
            .iter()
            .map(move |&(wid, frame)| {
                let frame = state.fullscreen_frame(view, g, wid).unwrap_or_else(|| {
                    translate_frame(frame, offset, g.tiling, g.screen, park).round()
                });
                (wid, frame)
            })
            .collect();
        frames
    }

    pub(crate) fn finish_viewport(&mut self, layout: LayoutId) {
        if let Some(state) = self.layouts.get_mut(layout)
            && state.geometry.is_some()
        {
            state.camera.viewport.finish();
        }
    }

    pub(crate) fn presentation(
        &self,
        layout: LayoutId,
    ) -> Option<(ViewportPresentation, Vec<(WindowId, CGRect, bool)>)> {
        let state = self.layouts.get(layout)?;
        let view = self.view(layout)?;
        let g = state.geometry.as_ref()?;
        let mut motion = state.camera.motion.clone();
        motion.samples.reserve(64_usize.saturating_sub(motion.samples.len()));
        let camera = ViewportPresentation {
            viewport: state.camera.viewport.clone(),
            motion: Arc::new(Mutex::new(motion)),
            bounds: g.bounds,
            width: g.tiling.size.width,
            tiling: g.tiling,
            screen: g.screen,
        };
        let frames = g
            .frames
            .iter()
            .map(|&(wid, frame)| {
                state
                    .fullscreen_frame(&view, g, wid)
                    .map_or_else(|| (wid, frame.round(), false), |frame| (wid, frame, true))
            })
            .collect();
        Some((camera, frames))
    }

    /// A camera ended: nothing presents its viewport any more, so it does not stay animating
    /// (no one would advance it, and every later layout would place windows at an offset of
    /// the moment). It ends at its target, where the camera sent the windows it presented and
    /// where the core wrote the ones it left; a gesture's camera ends where the hand left it.
    pub(crate) fn commit_ended_viewport(&mut self, layout: LayoutId, p: &PresentedViewport) {
        if let Some(state) = self.layouts.get_mut(layout) {
            state.camera.viewport = if p.gesturing {
                Viewport::Gesture(p.offset)
            } else {
                Viewport::Static(p.target)
            };
            state.camera.motion = p.motion.lock().clone();
        }
    }

    pub(crate) fn commit_presented_viewport(&mut self, layout: LayoutId, p: &PresentedViewport) {
        if let Some(state) = self.layouts.get_mut(layout) {
            state.camera.viewport = if p.gesturing {
                Viewport::Gesture(p.offset)
            } else if p.offset != p.target {
                Viewport::Animation(CameraSpring::new(p.offset, p.target, p.velocity, p.timestamp))
            } else {
                Viewport::Static(p.offset)
            };
            state.camera.motion = p.motion.lock().clone();
        }
    }

    /// Legacy normalized strip delta; boundary recognition is owned by the caller.
    pub fn scroll_by_delta(&mut self, layout: LayoutId, delta: f64) -> Option<(Direction, f64)> {
        if !delta.is_finite() {
            return None;
        }
        let view = self.view(layout)?;
        let state = self.layouts.get_mut(layout)?;
        let g = state.geometry.as_ref()?;
        let column = view.columns.get(view.active_column).and_then(|c| g.column(c.id))?;
        let step = column.width + g.gaps.inner.horizontal;
        let raw = state.camera.viewport.offset() + delta * step;
        state.camera.viewport.set_target(raw.clamp(g.bounds.0, g.bounds.1));
        if raw < g.bounds.0 {
            Some((Direction::Left, (g.bounds.0 - raw) / step))
        } else if raw > g.bounds.1 {
            Some((Direction::Right, (raw - g.bounds.1) / step))
        } else {
            None
        }
    }

    pub fn viewport_gesture_available(&self, layout: LayoutId) -> bool {
        let (Some(state), Some(view)) = (self.layouts.get(layout), self.view(layout)) else {
            return false;
        };
        state.geometry.is_some()
            && self
                .core
                .selected_window(layout)
                .is_some_and(|wid| !view.fullscreen.contains_key(&wid))
    }

    pub fn begin_viewport_gesture(&mut self, layout: LayoutId, now: Instant) -> bool {
        if !self.viewport_gesture_available(layout) {
            return false;
        }
        let state = &mut self.layouts[layout];
        state.camera.transient_restore = None;
        if matches!(state.camera.viewport, Viewport::Gesture(_)) {
            return false;
        }
        state.camera.motion.overscroll = 0.0;
        state.camera.motion.samples.clear();
        state.camera.motion.samples.reserve(64);
        state.camera.viewport.sample(now, 1.0);
        state.camera.viewport = Viewport::Gesture(state.camera.viewport.offset());
        true
    }

    /// Pixel deltas, independent of any native input source; focus stays fixed.
    pub fn update_viewport_gesture(
        &mut self,
        layout: LayoutId,
        delta: f64,
        timestamp: Duration,
    ) -> Option<f64> {
        if !delta.is_finite() {
            return None;
        }
        let state = self.layouts.get_mut(layout)?;
        let Viewport::Gesture(offset) = &mut state.camera.viewport else {
            return None;
        };
        let g = state.geometry.as_ref()?;
        state.camera.motion.update(offset, g.bounds, delta, timestamp)
    }

    /// Normalize only at the layout boundary, using cached working geometry.
    pub fn update_viewport_gesture_normalized(
        &mut self,
        layout: LayoutId,
        delta: f64,
        timestamp: Duration,
    ) -> Option<f64> {
        let width = self.layouts.get(layout)?.geometry.as_ref()?.tiling.size.width;
        self.update_viewport_gesture(layout, delta * width, timestamp)
    }

    /// Release projection is niri's exponential touchpad deceleration, in camera pixels.
    pub fn end_viewport_gesture(
        &mut self,
        layout: LayoutId,
        timestamp: Duration,
        animate: bool,
        started: Instant,
    ) -> Option<ViewportRelease> {
        // The camera chooses the column it settles on, which the core makes active.
        self.end_viewport_gesture_system(layout, timestamp, animate, started)
    }

    fn end_viewport_gesture_system(
        &mut self,
        layout: LayoutId,
        timestamp: Duration,
        animate: bool,
        started: Instant,
    ) -> Option<ViewportRelease> {
        let state = self.layouts.get_mut(layout)?;
        let Viewport::Gesture(offset) = state.camera.viewport else {
            return None;
        };
        if !state.camera.motion.push(0.0, timestamp) {
            return None;
        }
        let velocity = state.camera.motion.velocity();
        let projected = offset - velocity / (1000.0 * 0.997_f64.ln());
        let release = self.settle(layout, projected, velocity, true)?;
        if animate {
            self.layouts[layout].camera.viewport = Viewport::Animation(CameraSpring::new(
                release.from_offset,
                release.offset,
                release.velocity,
                started,
            ));
        }
        Some(release)
    }

    /// Settle where cancellation occurred, with no artificial fling or focus change.
    pub fn cancel_viewport_gesture(&mut self, layout: LayoutId) {
        if let Some(state) = self.layouts.get_mut(layout)
            && matches!(
                state.camera.viewport,
                Viewport::Gesture(_) | Viewport::Animation(_)
            )
        {
            let offset = state.camera.viewport.offset();
            let offset =
                state.geometry.as_ref().map_or(offset, |g| offset.clamp(g.bounds.0, g.bounds.1));
            state.camera.viewport = Viewport::Static(offset);
            state.camera.motion = MotionHistory::default();
        }
    }

    /// Signed travel beyond resting bounds, in working-area widths.
    pub fn gesture_overscroll(&self, layout: LayoutId) -> f64 {
        self.layouts
            .get(layout)
            .and_then(|state| {
                let g = state.geometry.as_ref()?;
                Some(state.camera.motion.overscroll / g.tiling.size.width)
            })
            .unwrap_or(0.0)
    }

    fn settle(
        &mut self,
        layout: LayoutId,
        projected: f64,
        velocity: f64,
        gesture_release: bool,
    ) -> Option<ViewportRelease> {
        let view = self.view(layout)?;
        let state = self.layouts.get_mut(layout)?;
        let g = state.geometry.as_ref()?;
        let from_offset = state.camera.viewport.offset();
        let niri = self.settings.focus_navigation_style == ScrollingFocusNavigationStyle::Niri;
        // After a gesture, a tie goes to a point showing the active column (asked only of a
        // strip with resting points, so with columns).
        let away = |snap: &SnapPoint| {
            gesture_release && snap.column != view.columns[view.active_column].id
        };
        let snap = g.snaps.iter().min_by(|a, b| {
            (a.offset - projected)
                .abs()
                .total_cmp(&(b.offset - projected).abs())
                .then_with(|| {
                    if niri {
                        // Niri sorts snaps by position before nearest-point selection.
                        a.offset.total_cmp(&b.offset)
                    } else {
                        away(a).cmp(&away(b))
                    }
                })
        })?;
        let mut column = snap.column;
        if niri {
            // Choose the furthest fully visible column
            // in the direction of travel.
            let past = |c: &&ColumnGeometry| c.id != snap.column;
            column = if projected >= from_offset {
                g.columns
                    .iter()
                    .skip_while(past)
                    .skip(1)
                    .take_while(|next| {
                        next.world_x + next.width <= snap.offset + g.tiling.size.width
                    })
                    .last()
                    .map_or(column, |next| next.id)
            } else {
                g.columns
                    .iter()
                    .rev()
                    .skip_while(past)
                    .skip(1)
                    .take_while(|next| next.world_x >= snap.offset)
                    .last()
                    .map_or(column, |next| next.id)
            };
        }
        let offset = snap.offset;
        let canonical_index = view.columns.iter().position(|c| c.id == column)?;
        if canonical_index != view.active_column {
            state.camera.fullscreen_restore = None;
        }
        state.camera.transient_restore = None;
        state.camera.viewport.set_target(offset);
        // The camera chose the column; the core makes it active.
        Self::activate_chosen(&mut self.core, layout, column);
        Some(ViewportRelease {
            window: self.core.selected_window(layout)?,
            offset,
            from_offset,
            velocity,
        })
    }

    pub fn snap_to_nearest_column(&mut self, layout: LayoutId) -> Option<WindowId> {
        // The camera chooses the column, which the core makes active.
        self.snap_to_nearest_column_system(layout)
    }

    pub fn center_selected_column(&mut self, layout: LayoutId) {
        let Some(view) = self.view(layout) else {
            return;
        };
        let Some(state) = self.layouts.get_mut(layout) else {
            return;
        };
        if let Some(g) = &state.geometry
            && let Some(column) = view.columns.get(view.active_column).and_then(|c| g.column(c.id))
        {
            state.camera.viewport.set_target(centered_offset(g.tiling.size.width, column));
        }
    }

    pub fn switch_preset_column_width(&mut self, layout: LayoutId, backwards: bool) -> bool {
        self.switch_preset_column_width_system(layout, backwards)
    }

    fn toggle_fullscreen(&mut self, layout: LayoutId, within_gaps: bool) -> Vec<WindowId> {
        use rift_core::layout::columns::Fullscreened;
        let Some(before) = self.view(layout) else {
            return Vec::new();
        };
        let Self { core, layouts, .. } = self;
        let Some(state) = layouts.get_mut(layout) else {
            return Vec::new();
        };
        let view = |core: &projection::Decider| {
            projection::view(core.arrangement(layout).expect("a layout the core holds"))
        };
        // The view before a fullscreen is the one the camera shows now.
        let bookmark = state.bookmark(&before);
        match core.toggle_fullscreen(layout, within_gaps) {
            Fullscreened::Nothing => Vec::new(),
            Fullscreened::Left(wid) => {
                if let Some(column) = state.restore_fullscreen_view(&view(core)) {
                    Self::activate_chosen(core, layout, column);
                }
                vec![wid]
            }
            Fullscreened::Entered(wid) => {
                let after = view(core);
                if state.camera.fullscreen_restore.is_none() {
                    state.camera.fullscreen_restore = bookmark;
                }
                if let Some(column) = state.geometry.as_ref().and_then(|g| {
                    after.columns.get(after.active_column).and_then(|c| g.column(c.id))
                }) {
                    state.camera.viewport.set_target(column.world_x);
                }
                vec![wid]
            }
        }
    }
}

impl LayoutSystem for ScrollingLayoutSystem {
    fn has_windows_for_app(&self, layout: LayoutId, pid: pid_t) -> bool {
        self.core.has_windows_for_app(layout, |window| window.pid == pid)
    }

    fn window_participates(&self, window: WindowId) -> bool {
        self.core.window_participates(window)
    }

    fn set_window_participation(&mut self, window: WindowId, participates: bool) -> bool {
        self.set_window_participation_system(window, participates)
    }

    fn create_layout(&mut self) -> LayoutId { self.create_layout_system() }

    fn contains_layout(&self, layout: LayoutId) -> bool { self.core.contains_layout(layout) }

    fn clone_layout(&mut self, layout: LayoutId) -> LayoutId { self.clone_layout_system(layout) }

    fn remove_layout(&mut self, layout: LayoutId) { self.remove_layout_system(layout); }

    fn prepare_layout(
        &mut self,
        layout: LayoutId,
        screen: CGRect,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &GapSettings,
        shadow: crate::layout_engine::Shadow,
    ) {
        self.prepare(layout, screen, constraints, None, gaps, shadow);
    }

    fn calculate_layout(
        &self,
        layout: LayoutId,
        screen: CGRect,
        _stack_offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &GapSettings,
        _stack_line: crate::common::config::StackLineReserve,
        shadow: crate::layout_engine::Shadow,
    ) -> Vec<(WindowId, CGRect)> {
        self.calculate_frames(layout, screen, constraints, None, gaps, true, shadow)
    }

    fn selected_window(&self, layout: LayoutId) -> Option<WindowId> {
        self.core.selected_window(layout)
    }

    fn window_in_direction(&self, layout: LayoutId, direction: Direction) -> Option<WindowId> {
        self.core.window_in_direction(layout, super::core_direction(direction))
    }

    fn contains_window(&self, layout: LayoutId, wid: WindowId) -> bool {
        self.core.contains_window(layout, wid)
    }

    fn has_any_fullscreen_node(&self, layout: LayoutId) -> bool {
        self.core.has_any_fullscreen_node(layout)
    }

    fn parent_of_selection_is_stacked(&self, layout: LayoutId) -> bool {
        self.core.parent_of_selection_is_stacked(layout)
    }

    fn windows(&self, layout: LayoutId, query: WindowQuery) -> Vec<WindowId> {
        self.core.windows(layout, projection::query(query))
    }

    fn focus(&mut self, layout: LayoutId, focus: Focus) -> FocusResult {
        self.focus_system(layout, focus)
    }

    fn update(&mut self, change: WindowChange<'_>) { self.update_system(change); }

    fn edit_layout(&mut self, layout: LayoutId, edit: LayoutEdit) -> EditResult {
        self.edit_layout_system(layout, edit)
    }
}

impl ScrollingLayoutSystem {
    fn visible_windows_under_selection(&self, layout: LayoutId) -> Vec<WindowId> {
        self.view(layout)
            .and_then(|view| view.columns.get(view.active_column).cloned())
            .map(|c| c.windows().copied().filter(|w| self.window_participates(*w)).collect())
            .unwrap_or_default()
    }

    fn move_focus(
        &mut self,
        layout: LayoutId,
        direction: Direction,
    ) -> (Option<WindowId>, Vec<WindowId>) {
        let Some(wid) = self.window_in_direction(layout, direction) else {
            return (None, Vec::new());
        };
        self.select_window(layout, wid);
        (Some(wid), self.visible_windows_under_selection(layout))
    }

    fn select_window(&mut self, layout: LayoutId, wid: WindowId) -> bool {
        let already = self.core.selected_window(layout) == Some(wid);
        let selected = self.core.select_window(layout, wid);
        if !selected || already {
            return selected;
        }
        let (Some(view), Some(state)) = (self.view(layout), self.layouts.get_mut(layout)) else {
            return selected;
        };
        state.camera.transient_restore = None;
        if state
            .camera
            .fullscreen_restore
            .as_ref()
            .is_some_and(|b| b.column != view.columns[view.active_column].id)
        {
            state.camera.fullscreen_restore = None;
        }
        state.reveal(&view, &self.settings);
        true
    }

    fn swap_windows(&mut self, layout: LayoutId, a: WindowId, b: WindowId) -> bool {
        self.edited(layout, |core| core.swap_windows(layout, a, b)).unwrap_or(false)
    }

    fn apply_target_drop(
        &mut self,
        layout: LayoutId,
        source: WindowId,
        target: WindowId,
        action: WindowDropAction,
    ) -> bool {
        if !self.core.window_participates(source) || !self.core.window_participates(target) {
            return false;
        }
        let transfer = match action {
            WindowDropAction::Move(direction) => {
                return self.select_window(layout, source)
                    && self.move_selection(layout, direction);
            }
            WindowDropAction::Swap => return self.swap_windows(layout, source, target),
            WindowDropAction::Stack => rift_core::layout::columns::Transfer::Stack,
            WindowDropAction::Insert(direction) => {
                rift_core::layout::columns::Transfer::Insert(super::core_direction(direction))
            }
            WindowDropAction::ConsumeOrExpel(_) => return false,
        };
        let Some(state) = self.layouts.get_mut(layout) else {
            return false;
        };
        state.camera.transient_restore = None;
        self.edited(layout, |core| {
            core.drop_window(
                layout,
                source,
                target,
                rift_core::layout::columns::DropAction::Transfer(transfer),
            )
        })
        .unwrap_or(false)
    }

    pub(in crate::layout_engine) fn can_move_window(
        &self,
        layout: LayoutId,
        window: WindowId,
        direction: Direction,
    ) -> bool {
        self.core.can_move_window(layout, window, super::core_direction(direction))
    }

    fn move_selection(&mut self, layout: LayoutId, direction: Direction) -> bool {
        let Some(state) = self.layouts.get_mut(layout) else {
            return false;
        };
        let viewport = state.camera.viewport.clone();
        let Some(moved) = self
            .edited(layout, |core| {
                core.move_selection(layout, super::core_direction(direction))
            })
            .flatten()
        else {
            return false;
        };
        let state = &mut self.layouts[layout];
        state.camera.transient_restore = None;
        if !matches!(moved, rift_core::layout::columns::Moved::Row(_)) {
            state.camera.viewport = viewport;
            state.reconcile_camera_bounds();
            self.reveal(layout);
        }
        true
    }

    fn join_selection_with_direction(&mut self, layout: LayoutId, direction: Direction) -> bool {
        if !matches!(direction, Direction::Left | Direction::Right) {
            return false;
        }
        let Some(target) = self.window_in_direction(layout, direction) else {
            return false;
        };
        let Some(source) = self.selected_window(layout) else {
            return false;
        };
        self.apply_target_drop(layout, source, target, WindowDropAction::Stack)
    }
}

/// The system's operations: each made by the core, then the camera's answer to it.
impl ScrollingLayoutSystem {
    fn set_window_participation_system(&mut self, window: WindowId, participates: bool) -> bool {
        use rift_core::layout::columns::Participation;
        let participation = if participates {
            Participation::TakesPart
        } else {
            Participation::LeftOut
        };
        let holding: Vec<(LayoutId, BeforeEdit)> = self
            .layouts
            .keys()
            .filter(|&layout| self.core.contains_window(layout, window))
            .filter_map(|layout| Some((layout, self.before_edit(layout)?)))
            .collect();
        // The core shows an available column where the selection stops taking part: the one each
        // camera remembers, else the nearest (upstream's unified fallback, bafcc444).
        let remembered: Vec<_> = self
            .layouts
            .iter()
            .filter_map(|(layout, state)| {
                let bookmark = state.camera.transient_restore.as_ref()?;
                Some((layout, rift_core::layout::columns::ColumnId(bookmark.column.0)))
            })
            .collect();
        if !self.core.set_window_participation(window, participation, &remembered) {
            return false;
        }
        for (layout, before) in holding {
            let Some(after) = self.after_edit(layout, &before) else {
                continue;
            };
            // The camera follows a column the core moved to: back to the view it remembers, if
            // that is the column, or else revealing it (upstream #705, f168b6c6, and bafcc444).
            if after.active_column != before.view.active_column {
                let shown = after.columns[after.active_column].id;
                let state = &mut self.layouts[layout];
                let restored = state
                    .camera
                    .transient_restore
                    .take()
                    .filter(|bookmark| bookmark.column == shown)
                    .and_then(|bookmark| state.restore(&after, bookmark));
                if restored.is_none() {
                    state.reveal(&after, &self.settings);
                }
            }
        }
        true
    }

    fn create_layout_system(&mut self) -> LayoutId {
        // The core allocates the layout's id; the camera's state is kept under it.
        let layout = self.core.create_layout();
        self.layouts.insert(layout, LayoutState::default());
        layout
    }

    fn clone_layout_system(&mut self, layout: LayoutId) -> LayoutId {
        let mut state = self.layouts.get(layout).cloned().unwrap_or_default();
        if let Viewport::Pending { target, .. } = state.camera.viewport {
            state.camera.viewport = Viewport::Static(target);
        } else if matches!(state.camera.viewport, Viewport::Animation(_)) {
            let offset = state.camera.viewport.offset();
            state.camera.viewport = Viewport::Static(
                state.geometry.as_ref().map_or(offset, |g| offset.clamp(g.bounds.0, g.bounds.1)),
            );
        }
        let clone = self.core.clone_layout(layout);
        self.layouts.insert(clone, state);
        clone
    }

    fn remove_layout_system(&mut self, layout: LayoutId) {
        self.core.remove_layout(layout);
        self.layouts.remove(layout);
    }

    fn update_system(&mut self, change: WindowChange<'_>) {
        match change {
            WindowChange::Add(layout, window, _) => self.insert(layout, window),
            WindowChange::Remove(window) | WindowChange::RemoveAndRebalance(window) => {
                // Removal per layout, the camera answering each before the next (the core never
                // asks it).
                let holding = self.core.forget_window(window);
                for layout in holding {
                    self.remove_from_layout(layout, window);
                }
            }
            WindowChange::Replace(from, to) => {
                let holding: Vec<LayoutId> = self
                    .layouts
                    .keys()
                    .filter(|&layout| self.core.contains_window(layout, from))
                    .collect();
                self.core.replace_window(from, to);
                for layout in holding {
                    self.edited(layout, |_| ());
                }
            }
            WindowChange::CloseApp(pid) => {
                let closing = self.core.forget_app(|window| window.pid == pid);
                for (layout, windows) in closing {
                    for window in windows {
                        self.remove_from_layout(layout, window);
                    }
                }
            }
            WindowChange::Resize {
                layout,
                window,
                old,
                new,
                screen,
                gaps,
            } => {
                let resize = projection::frame_resize(old, new, screen, gaps);
                self.edited(layout, |core| core.resize_from_frame(layout, window, resize));
            }
        }
    }

    fn edit_layout_system(&mut self, layout: LayoutId, edit: LayoutEdit) -> EditResult {
        match edit {
            LayoutEdit::Move {
                direction,
                has_display_neighbor: _,
            } => EditResult::from(self.move_selection(layout, direction)),
            LayoutEdit::Fullscreen { within_gaps } => {
                EditResult::Raise(self.toggle_fullscreen(layout, within_gaps))
            }
            LayoutEdit::Swap(a, b) => EditResult::from(self.swap_windows(layout, a, b)),
            LayoutEdit::Drop { source, target, action } => {
                EditResult::from(self.apply_target_drop(layout, source, target, action))
            }
            LayoutEdit::Group(GroupEdit::Join(direction)) => {
                EditResult::from(self.join_selection_with_direction(layout, direction))
            }
            LayoutEdit::Group(GroupEdit::ConsumeOrExpel(direction)) => {
                if !matches!(direction, Direction::Left | Direction::Right) {
                    return EditResult::Unchanged;
                }
                EditResult::from(if self.parent_of_selection_is_stacked(layout) {
                    self.move_selection(layout, direction)
                } else {
                    self.join_selection_with_direction(layout, direction)
                })
            }
            LayoutEdit::Group(GroupEdit::Stack(_)) => {
                let moved = self.edited(layout, |core| core.stack(layout)).unwrap_or_default();
                if !moved.is_empty() {
                    self.layouts[layout].camera.transient_restore = None;
                }
                EditResult::Raise(moved)
            }
            LayoutEdit::Group(GroupEdit::Unstack(_)) => {
                let moved = self.edited(layout, |core| core.unstack(layout)).unwrap_or_default();
                if !moved.is_empty() {
                    self.layouts[layout].camera.transient_restore = None;
                }
                EditResult::Raise(moved)
            }
            LayoutEdit::Group(GroupEdit::Unjoin) => EditResult::from(
                self.parent_of_selection_is_stacked(layout)
                    && self.move_selection(layout, Direction::Right),
            ),
            LayoutEdit::Resize { amount, orientation } => {
                use rift_core::layout::columns::{Edited, ResizeAlong};
                let along = match orientation {
                    ResizeOrientation::Horizontal => ResizeAlong::Horizontal,
                    ResizeOrientation::Vertical => ResizeAlong::Vertical,
                    ResizeOrientation::Smart => ResizeAlong::Smart,
                };
                let Some(view) = self.view(layout) else {
                    return EditResult::Unchanged;
                };
                let vertical = orientation == ResizeOrientation::Vertical
                    || (orientation == ResizeOrientation::Smart
                        && view.columns.get(view.active_column).is_some_and(|c| c.rows.len() > 1));
                let span = projection::span(self, layout);
                let edited = self
                    .edited(layout, |core| core.resize_selection(layout, amount, along, span))
                    .unwrap_or(Edited::Unchanged);
                if !vertical && edited == Edited::Changed {
                    self.reveal(layout);
                }
                projection::edit_result(edited)
            }
            _ => EditResult::Unsupported,
        }
    }

    fn focus_system(&mut self, layout: LayoutId, focus: Focus) -> FocusResult {
        match focus {
            Focus::Select(window) => FocusResult::selection(self.select_window(layout, window)),
            Focus::Ascend => {
                FocusResult::selection(self.move_focus(layout, Direction::Up).0.is_some())
            }
            Focus::Descend => {
                FocusResult::selection(self.move_focus(layout, Direction::Down).0.is_some())
            }
            Focus::Direction(direction) => {
                FocusResult::movement(self.move_focus(layout, direction))
            }
        }
    }

    fn switch_preset_column_width_system(&mut self, layout: LayoutId, backwards: bool) -> bool {
        let span = projection::span(self, layout);
        let changed = self
            .edited(layout, |core| {
                core.switch_preset_column_width(layout, backwards, span)
            })
            .unwrap_or(false);
        if changed {
            self.reveal(layout);
        }
        changed
    }

    fn snap_to_nearest_column_system(&mut self, layout: LayoutId) -> Option<WindowId> {
        let offset = self.layouts.get(layout)?.camera.viewport.offset();
        self.settle(layout, offset, 0.0, false).map(|release| release.window)
    }
}

impl ScrollingLayoutSystem {
    pub(crate) fn draw_tree(&self, layout: LayoutId) -> String {
        let Some(view) = self.view(layout) else {
            return String::new();
        };
        view.columns
            .iter()
            .enumerate()
            .map(|(index, col)| {
                format!("Column {index}: {:?} active={}\n", col.rows, col.active_window)
            })
            .collect()
    }

    pub(crate) fn container_tree(&self, layout: LayoutId) -> rift_protocol::ContainerTreeNode {
        let view = self.view(layout).expect("unknown scrolling layout");
        let selected = self.core.selected_window(layout);
        let children = view
            .columns
            .iter()
            .map(|column| {
                let windows = column
                    .rows
                    .iter()
                    .map(|&Row { window, height }| {
                        let fullscreen = view.fullscreen.get(&window);
                        rift_protocol::ContainerTreeNode {
                            node_id: (((window.pid as u32 as u64) << 32)
                                | u64::from(window.idx.get()))
                            .rotate_left(1)
                                | 1,
                            node_type: rift_protocol::ContainerNodeType::Window,
                            weight: Some(height),
                            window_id: Some(window.into()),
                            is_selected: selected == Some(window),
                            is_fullscreen: fullscreen == Some(&FullscreenMode::Screen),
                            is_fullscreen_within_gaps: fullscreen
                                == Some(&FullscreenMode::WithinGaps),
                            ..Default::default()
                        }
                    })
                    .collect();
                rift_protocol::ContainerTreeNode {
                    node_id: column.id.0,
                    layout_kind: Some(rift_protocol::LayoutKind::Vertical),
                    role: Some("column".to_owned()),
                    children: windows,
                    ..Default::default()
                }
            })
            .collect();

        rift_protocol::ContainerTreeNode {
            layout_kind: Some(rift_protocol::LayoutKind::Horizontal),
            children,
            ..Default::default()
        }
    }

    pub(crate) fn window_slot(&self, layout: LayoutId, window: WindowId) -> Option<Vec<usize>> {
        self.core.window_slot(layout, window)
    }
}
#[cfg(test)]
pub(crate) mod tests {
    pub(crate) fn presented_frames(
        system: &super::ScrollingLayoutSystem,
        layout: crate::layout_engine::LayoutId,
    ) -> std::vec::IntoIter<(crate::actor::app::WindowId, objc2_core_foundation::CGRect)> {
        system
            .presentation(layout)
            .map(|(camera, frames)| {
                frames
                    .into_iter()
                    .map(|(wid, frame, fixed)| {
                        (wid, camera.frame_at_offset(frame, fixed, 1.0, camera.offset()))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
            .into_iter()
    }
    use super::*;

    fn wid(index: u32) -> WindowId { WindowId::new(1, index) }

    #[test]
    fn a_camera_that_ends_before_arriving_leaves_its_viewport_at_the_target() {
        let mut f = Fixture::new(4);
        let (mut p, _) = f.system.presentation(f.layout).unwrap();
        p.viewport = Viewport::Static(0.0);
        p.viewport.set_target(500.0);
        p.retarget(0.0, 0.0, Instant::now());
        let snapshot = p.snapshot(Instant::now());
        assert_eq!((snapshot.offset, snapshot.target), (0.0, 500.0));
        // Still presenting: the viewport keeps moving.
        f.system.commit_presented_viewport(f.layout, &snapshot);
        assert!(matches!(
            &f.system.layouts[f.layout].camera.viewport,
            Viewport::Animation(_)
        ));
        // Ended there: nothing will move it, so it is at its target, not stuck animating.
        f.system.commit_ended_viewport(f.layout, &snapshot);
        let viewport = &f.system.layouts[f.layout].camera.viewport;
        assert!(
            matches!(viewport, Viewport::Static(offset) if *offset == 500.0),
            "{viewport:?}"
        );
    }

    struct Fixture {
        system: ScrollingLayoutSystem,
        layout: LayoutId,
        screen: CGRect,
        gaps: GapSettings,
        constraints: HashMap<WindowId, WindowLayoutConstraints>,
    }

    impl Fixture {
        fn new(count: u32) -> Self {
            let settings = ScrollingLayoutSettings {
                column_width_ratio: 0.5,
                min_column_width_ratio: 0.1,
                alignment: ScrollingAlignment::Left,
                preserve_window_sizes: false,
                ..Default::default()
            };
            let mut system = ScrollingLayoutSystem::new(&settings);
            let layout = system.create_layout();
            for index in 1..=count {
                system.update(WindowChange::Add(layout, wid(index), None));
            }
            if count > 0 {
                system.select_window(layout, wid(1));
            }
            let mut fixture = Self {
                system,
                layout,
                screen: CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(1000.0, 800.0)),
                gaps: GapSettings::default(),
                constraints: HashMap::default(),
            };
            fixture.prepare();
            fixture
        }

        fn prepare(&mut self) {
            self.system.prepare_layout(
                self.layout,
                self.screen,
                &self.constraints,
                &self.gaps,
                crate::layout_engine::Shadow::Run,
            );
        }

        fn frames(&mut self) -> Vec<(WindowId, CGRect)> {
            self.prepare();
            self.system.calculate_frames(
                self.layout,
                self.screen,
                &self.constraints,
                None,
                &self.gaps,
                false,
                crate::layout_engine::Shadow::Run,
            )
        }

        fn frame(&mut self, index: u32) -> CGRect {
            self.frames().into_iter().find(|(w, _)| *w == wid(index)).unwrap().1
        }

        fn select(&mut self, index: u32) {
            assert!(self.system.select_window(self.layout, wid(index)));
        }

        fn selected(&self) -> Option<WindowId> { self.system.selected_window(self.layout) }

        fn drop(&mut self, source: u32, target: u32, action: WindowDropAction) {
            assert!(
                self.system
                    .edit(self.layout, LayoutEdit::Drop {
                        source: wid(source),
                        target: wid(target),
                        action: action
                    })
                    .changed()
            );
        }
    }

    /// Columns wholly out of view are parked at the screen's edge with one point showing (what
    /// rift does with a scrolling layout, whichever core decides it).
    #[test]
    fn columns_out_of_view_are_parked_with_a_point_showing() {
        let mut f = Fixture::new(4);
        f.prepare();
        let frames = f.system.calculate_layout(
            f.layout,
            f.screen,
            0.0,
            &f.constraints,
            &f.gaps,
            Default::default(),
            crate::layout_engine::Shadow::Run,
        );
        let xs: Vec<(WindowId, f64)> = frames.iter().map(|&(w, r)| (w, r.origin.x)).collect();
        // Two 500-point columns fill the 1000-point screen; the other two park at its right edge,
        // one point showing.
        assert_eq!(xs, vec![
            (wid(1), 0.0),
            (wid(2), 500.0),
            (wid(3), 999.0),
            (wid(4), 999.0)
        ]);
    }

    /// Columns sit a column gap apart and a column's rows a row gap apart (what rift does with a
    /// scrolling layout, whichever core places it).
    #[test]
    fn columns_and_rows_are_placed_their_gaps_apart() {
        let mut f = Fixture::new(3);
        f.gaps.inner.horizontal = 20.0;
        f.gaps.inner.vertical = 10.0;
        f.prepare();
        f.select(3);
        f.system
            .edit(
                f.layout,
                LayoutEdit::Group(GroupEdit::ConsumeOrExpel(Direction::Left)),
            )
            .discard();
        f.prepare();
        let frames = f.system.calculate_layout(
            f.layout,
            f.screen,
            0.0,
            &f.constraints,
            &f.gaps,
            Default::default(),
            crate::layout_engine::Shadow::Run,
        );
        let rect = |x, y, w, h| CGRect::new(CGPoint::new(x, y), CGSize::new(w, h));
        // Half of the 1000-point screen and one gap, less the gap: 490 points a column; the
        // second column's two rows share its 800 points less one gap.
        assert_eq!(frames, vec![
            (wid(1), rect(0.0, 0.0, 490.0, 800.0)),
            (wid(2), rect(510.0, 0.0, 490.0, 395.0)),
            (wid(3), rect(510.0, 405.0, 490.0, 395.0)),
        ]);
    }

    /// Anchored and left-aligned, the last two of four half-view columns rest at the same point:
    /// a release after a gesture goes to the active one, any other release to the earlier one.
    #[test]
    fn after_a_gesture_a_tied_resting_point_shows_the_active_column() {
        let mut f = Fixture::new(4);
        let settings = ScrollingLayoutSettings {
            focus_navigation_style: ScrollingFocusNavigationStyle::Anchored,
            ..f.system.settings.clone()
        };
        f.system.update_settings(&settings);
        f.prepare();
        for (gesture, window) in [(true, 4), (false, 3)] {
            f.select(4);
            let release = f.system.settle(f.layout, 1000.0, 0.0, gesture).unwrap();
            assert_eq!((release.window, release.offset), (wid(window), 1000.0));
        }
    }

    /// Niri: a release shows the furthest column wholly in view in the direction of travel.
    #[test]
    fn a_niri_release_shows_the_furthest_column_in_view_its_way() {
        let mut f = Fixture::new(5);
        f.prepare();
        // Five half-view columns at 0, 500, 1000, 1500 and 2000; the camera rests at 0, 500,
        // 1000 and 1500.
        for (from, projected, window, offset) in [
            (0.0, 480.0, 3, 500.0),
            (0.0, 1020.0, 4, 1000.0),
            (1400.0, 1010.0, 3, 1000.0),
            (1500.0, 520.0, 2, 500.0),
            // Midway between 1000 and the right end (1500): niri takes the leftmost of the two,
            // though the right end comes first among the resting points.
            (1000.0, 1250.0, 4, 1000.0),
        ] {
            f.system.layouts[f.layout].camera.viewport = Viewport::Static(from);
            let release = f.system.settle(f.layout, projected, 0.0, true).unwrap();
            assert_eq!(
                (release.window, release.offset),
                (wid(window), offset),
                "from {from} to {projected}"
            );
        }
    }

    /// Anchored and centred, the camera rests with a middle column in the middle of the view and
    /// the last column at the view's right edge: snapping goes to the nearest such point (what
    /// rift does with a scrolling layout, whichever core places it).
    #[test]
    fn a_centred_strip_snaps_to_the_nearest_column_centred() {
        let mut f = Fixture::new(4);
        let settings = ScrollingLayoutSettings {
            alignment: ScrollingAlignment::Center,
            focus_navigation_style: ScrollingFocusNavigationStyle::Anchored,
            ..f.system.settings.clone()
        };
        f.system.update_settings(&settings);
        f.prepare();
        // Four 500-point columns in a 1000-point view: the camera rests at 0 (the first column),
        // 250 and 750 (the middle two, centred) and 1000 (the last).
        for (offset, window, target) in [(700.0, 3, 750.0), (400.0, 2, 250.0), (900.0, 4, 1000.0)] {
            f.system.layouts[f.layout].camera.viewport = Viewport::Static(offset);
            assert_eq!(
                f.system.snap_to_nearest_column(f.layout),
                Some(wid(window)),
                "from {offset}"
            );
            assert_eq!(
                f.system.layouts[f.layout].camera.viewport.target(),
                target,
                "from {offset}"
            );
        }
    }

    #[test]
    fn consume_expel_and_horizontal_extraction_preserve_identity_sizing_and_focus() {
        for direction in [Direction::Left, Direction::Right] {
            let mut f = Fixture::new(2);
            f.select(1);
            f.system
                .edit(f.layout, LayoutEdit::Resize {
                    amount: -0.1,
                    orientation: ResizeOrientation::Horizontal,
                })
                .discard();
            let destination_id = f.system.container_tree(f.layout).children[0].node_id;
            f.select(2);
            f.system
                .edit(
                    f.layout,
                    LayoutEdit::Group(GroupEdit::ConsumeOrExpel(Direction::Left)),
                )
                .discard();
            assert_eq!(f.selected(), Some(wid(2)));
            assert_eq!(f.frame(2).size.width, 400.0);
            assert_eq!(
                f.system.container_tree(f.layout).children[0].node_id,
                destination_id
            );
            f.system
                .edit(f.layout, LayoutEdit::Resize {
                    amount: 0.2,
                    orientation: ResizeOrientation::Vertical,
                })
                .discard();
            let offset = f.system.layouts[f.layout].camera.viewport.start_offset();
            f.system
                .edit(f.layout, LayoutEdit::Group(GroupEdit::ConsumeOrExpel(direction)))
                .discard();
            assert_eq!(f.selected(), Some(wid(2)));
            assert_eq!(f.system.layouts[f.layout].camera.viewport.start_offset(), offset);
            assert_eq!(f.frame(2).size.width, 400.0);
            let tree = f.system.container_tree(f.layout);
            let expelled = tree
                .children
                .iter()
                .find(|c| c.children[0].window_id == Some(wid(2).into()))
                .unwrap();
            assert_ne!(expelled.node_id, destination_id);
            f.system.join_selection_with_direction(
                f.layout,
                if direction == Direction::Left {
                    Direction::Right
                } else {
                    Direction::Left
                },
            );
            assert_eq!(f.selected(), Some(wid(2)));
            assert_eq!(f.frame(2).size.height, 560.0);
            assert!(f.system.move_selection(f.layout, direction));
            assert_eq!(f.selected(), Some(wid(2)));
            assert_eq!(
                f.system.window_slot(f.layout, wid(2)),
                Some(vec![usize::from(direction == Direction::Right), 0])
            );
        }
    }

    /// Upstream's test (bafcc444), carried over: removing the selected column shows the
    /// nearest column with a window taking part, past a suspended neighbour.
    #[test]
    fn removing_selected_column_skips_suspended_neighbor() {
        let mut f = Fixture::new(3);
        f.select(3);
        f.system.set_window_participation(wid(2), false);

        f.system.update(WindowChange::Remove(wid(3)));

        assert_eq!(f.selected(), Some(wid(1)));
        assert_eq!(f.frame(1).origin.x, f.screen.origin.x);
    }

    /// A selection left out shows an available column: the one the camera remembers from before
    /// the window came, and the camera goes back to that view; with nothing remembered, the
    /// nearest column taking part, revealed (upstream bafcc444, #705).
    #[test]
    fn leaving_out_the_selection_shows_the_remembered_column_else_the_nearest() {
        let mut f = Fixture::new(3);
        f.select(1);
        let before = f.frame(1);
        // Window 4 comes next to 1, remembering the view on 1; left out, that view comes back
        // (the nearest column, rightwards, would be 2's).
        f.system.update(WindowChange::Add(f.layout, wid(4), None));
        assert_eq!(f.selected(), Some(wid(4)));
        f.system.set_window_participation(wid(4), false);
        f.system.finish_viewport(f.layout);
        assert_eq!(f.selected(), Some(wid(1)));
        assert_eq!(f.frame(1), before);
        // Nothing remembered: with 2 selected and taking part on both sides, 2 left out shows
        // the nearest rightwards (3), revealed, before leftwards (4, then 1).
        f.system.set_window_participation(wid(4), true);
        f.select(2);
        f.system.set_window_participation(wid(2), false);
        f.system.finish_viewport(f.layout);
        assert_eq!(f.selected(), Some(wid(3)));
        let shown = f.frame(3);
        use crate::sys::geometry::CGRectExt;
        assert!(f.screen.contains(shown.mid()), "{shown:?}");
    }

    /// A selection left out shows the nearest column taking part, and the camera reveals it
    /// whole (upstream #705, f168b6c6): a wide column only partly in view comes fully into it.
    #[test]
    fn a_column_shown_after_a_selection_is_left_out_is_revealed() {
        let mut f = Fixture::new(3);
        f.select(3);
        f.system
            .edit_layout(f.layout, LayoutEdit::Resize {
                amount: 0.5,
                orientation: ResizeOrientation::Horizontal,
            })
            .discard();
        // The camera back at the strip's start, 1 and 2 in view, 3 partly beyond the edge.
        f.select(1);
        f.system.finish_viewport(f.layout);
        f.select(2);
        f.system.finish_viewport(f.layout);
        f.system.set_window_participation(wid(2), false);
        f.system.finish_viewport(f.layout);
        assert_eq!(f.selected(), Some(wid(3)));
        let shown = f.frame(3);
        assert!(
            shown.origin.x >= f.screen.origin.x && shown.max().x <= f.screen.max().x,
            "{shown:?}"
        );
    }

    /// When the column the camera remembers takes no part any more, a removal shows the nearest
    /// one instead, and the camera reveals that one rather than going back to the remembered
    /// view (upstream bafcc444).
    #[test]
    fn a_removal_reveals_the_shown_column_when_the_remembered_one_takes_no_part() {
        let mut f = Fixture::new(3);
        f.select(3);
        f.system
            .edit_layout(f.layout, LayoutEdit::Resize {
                amount: 0.5,
                orientation: ResizeOrientation::Horizontal,
            })
            .discard();
        f.select(2);
        f.system.finish_viewport(f.layout);
        // 4 comes next to 2, remembering the view on 2; 2 then takes no part.
        f.system.update(WindowChange::Add(f.layout, wid(4), None));
        f.system.set_window_participation(wid(2), false);
        f.system.update(WindowChange::Remove(wid(4)));
        f.system.finish_viewport(f.layout);
        assert_eq!(f.selected(), Some(wid(3)));
        let shown = f.frame(3);
        assert!(
            shown.origin.x >= f.screen.origin.x && shown.max().x <= f.screen.max().x,
            "{shown:?}"
        );
    }

    #[test]
    fn structural_edits_rebase_an_ongoing_gesture_without_changing_focus() {
        let mut f = Fixture::new(4);
        f.select(3);
        f.system.begin_viewport_gesture(f.layout, Instant::now());
        f.system.update_viewport_gesture(f.layout, 30.0, Duration::from_millis(10));
        let x = f.frame(3).origin.x;
        f.system.update(WindowChange::Remove(wid(1)));
        assert_eq!(f.frame(3).origin.x, x);
        assert_eq!(f.selected(), Some(wid(3)));
        f.system.update_viewport_gesture(f.layout, 20.0, Duration::from_millis(20));
        assert_eq!(f.frame(3).origin.x, x - 20.0);
        assert_eq!(f.selected(), Some(wid(3)));
        let release = f
            .system
            .end_viewport_gesture(f.layout, Duration::from_millis(220), false, Instant::now())
            .unwrap();
        assert_eq!(release.from_offset, 50.0);
        assert_eq!(release.offset, 0.0);
        assert_eq!(release.window, wid(3));
        assert_eq!(f.frame(3).origin.x, 500.0);
    }

    /// The camera reads the core's arrangement, computed at each use: after an operation the
    /// view is the core's arrangement, which the core decided by itself.
    #[test]
    fn the_camera_reads_the_cores_arrangement() {
        let mut f = Fixture::new(3);
        f.system.update(WindowChange::Add(f.layout, wid(4), None));
        let core = f.system.core.arrangement(f.layout).unwrap();
        // The core placed the new window next to the selection, 1.
        assert_eq!(core.windows().collect::<Vec<_>>(), vec![
            wid(1),
            wid(4),
            wid(2),
            wid(3)
        ]);
        assert_eq!(projection::arrangement(&f.system.view(f.layout).unwrap()), *core);
    }

    /// Saving writes slotmap's own form of the layouts (the core's table, a vacant slot and its
    /// version included): a slotmap read from the saved text writes the same bytes.
    #[test]
    fn saving_writes_a_slotmaps_form() {
        let mut f = Fixture::new(3);
        let clone = f.system.clone_layout(f.layout);
        f.system.remove_layout(clone);
        let text = ron::to_string(&f.system).unwrap();
        #[derive(Serialize, Deserialize)]
        struct Map {
            layouts: slotmap::SlotMap<LayoutId, LoadedLayout>,
        }
        let map: Map = ron::from_str(&text).unwrap();
        assert_eq!(ron::to_string(&map).unwrap(), text);
    }

    /// Per-display width settings reach the core: a resize narrower than their minimum is
    /// clamped to it on both sides (a difference fails the test).
    #[test]
    fn width_settings_reach_the_core() {
        let mut f = Fixture::new(2);
        f.system.update_width_settings((0.5, 0.4, 0.6));
        f.prepare();
        f.system
            .edit_layout(f.layout, LayoutEdit::Resize {
                amount: -0.5,
                orientation: ResizeOrientation::Horizontal,
            })
            .discard();
        let core = f.system.core.arrangement(f.layout).unwrap();
        let rift_core::layout::columns::Strip::Columns { active, .. } = &core.strip else {
            panic!("columns")
        };
        assert_eq!(active.width, rift_core::layout::columns::Width::Proportion(0.4));
    }

    /// A window that came and goes again, still selected and alone in its column, gives back
    /// the view remembered when it came: the column the camera chooses is made active on the
    /// core (its own removal would have selected the next column).
    #[test]
    fn a_window_that_comes_and_goes_gives_back_the_view_before_it() {
        let mut f = Fixture::new(3);
        f.system.update(WindowChange::Add(f.layout, wid(4), None));
        assert_eq!(f.selected(), Some(wid(4)));
        f.system.update(WindowChange::Remove(wid(4)));
        assert_eq!(f.selected(), Some(wid(1)));
    }

    /// A window left out, removed and added again takes part: the windows left out are the
    /// core's after the removal.
    #[test]
    fn a_window_left_out_removed_and_added_again_takes_part() {
        let mut f = Fixture::new(3);
        f.system.set_window_participation(wid(2), false);
        f.system.update(WindowChange::Remove(wid(2)));
        f.system.update(WindowChange::Add(f.layout, wid(2), None));
        assert!(f.frames().iter().any(|&(w, _)| w == wid(2)));
    }

    /// Under `preserve_window_sizes` a pass fixes a lone window's column at its locked width,
    /// on the core.
    #[test]
    fn a_pass_preserves_a_locked_width_on_the_core() {
        let mut f = Fixture::new(2);
        f.system.update_settings(&ScrollingLayoutSettings {
            preserve_window_sizes: true,
            ..f.system.settings.clone()
        });
        f.constraints.insert(wid(1), WindowLayoutConstraints {
            locked_width: 300.0,
            ..Default::default()
        });
        f.prepare();
        let core = f.system.core.arrangement(f.layout).unwrap();
        let first = core.strip.columns()[0].width;
        assert!(
            matches!(first, rift_core::layout::columns::Width::Fixed(_)),
            "{first:?}"
        );
    }

    /// A fullscreen ended gives back the view the camera showed before it.
    #[test]
    fn a_fullscreen_ended_gives_back_the_view_before_it() {
        let mut f = Fixture::new(3);
        f.select(3);
        let before = f.frame(3);
        for _ in 0..2 {
            f.system.edit(f.layout, LayoutEdit::Fullscreen { within_gaps: false }).discard();
            f.system.finish_viewport(f.layout);
        }
        assert_eq!(f.frame(3), before);
    }

    #[test]
    fn persistence_and_clone_preserve_column_state() {
        let mut f = Fixture::new(3);
        f.drop(2, 1, WindowDropAction::Stack);
        f.select(2);
        f.system.center_selected_column(f.layout);
        let tree = f.system.container_tree(f.layout);
        let frames = f.frames();
        let clone = f.system.clone_layout(f.layout);
        assert_eq!(f.system.container_tree(clone), tree);
        assert_eq!(presented_frames(&f.system, clone).collect::<Vec<_>>(), frames);
        let text = ron::to_string(&f.system).unwrap();
        let mut restored: ScrollingLayoutSystem = ron::from_str(&text).unwrap();
        restored.update_settings(&f.system.settings);
        restored.prepare_layout(
            f.layout,
            f.screen,
            &f.constraints,
            &f.gaps,
            crate::layout_engine::Shadow::Run,
        );
        assert_eq!(restored.container_tree(f.layout), tree);
        assert_eq!(presented_frames(&restored, f.layout).collect::<Vec<_>>(), frames);
        f.system.select_window(clone, wid(3));
        f.system.center_selected_column(clone);
        let old_x = f
            .system
            .calculate_frames(
                clone,
                f.screen,
                &f.constraints,
                None,
                &f.gaps,
                false,
                crate::layout_engine::Shadow::Run,
            )
            .into_iter()
            .find(|(w, _)| *w == wid(3))
            .unwrap()
            .1
            .origin
            .x;
        let mut resized_restore: ScrollingLayoutSystem =
            ron::from_str(&ron::to_string(&f.system).unwrap()).unwrap();
        resized_restore.update_settings(&f.system.settings);
        let wider = CGRect::new(CGPoint::ZERO, CGSize::new(1600.0, 800.0));
        resized_restore.prepare_layout(
            clone,
            wider,
            &f.constraints,
            &f.gaps,
            crate::layout_engine::Shadow::Run,
        );
        let new_x = resized_restore
            .calculate_frames(
                clone,
                wider,
                &f.constraints,
                None,
                &f.gaps,
                false,
                crate::layout_engine::Shadow::Run,
            )
            .into_iter()
            .find(|(w, _)| *w == wid(3))
            .unwrap()
            .1
            .origin
            .x;
        assert_eq!(new_x, old_x);

        f.system.update(WindowChange::Add(f.layout, wid(4), None));
        let ids: Vec<_> =
            f.system.container_tree(f.layout).children.iter().map(|c| c.node_id).collect();
        assert_ne!(ids[0], ids[1]);
    }

    #[test]
    fn old_scrolling_snapshots_migrate_identity_sizing_and_selected_row() {
        let first = ron::to_string(&wid(1)).unwrap();
        let second = ron::to_string(&wid(2)).unwrap();
        let text = format!(
            "(columns:[(node_id:42,windows:[{first},{second}],width_offset:0.1,width_overridden:true,height_weights:[1.0,1.0])],selected:Some({second}),column_width_ratio:0.5,fullscreen:[],fullscreen_within_gaps:[])"
        );
        let loaded: LoadedLayout = ron::from_str(&text).unwrap();
        let mut layouts = slotmap::SlotMap::<LayoutId, LoadedLayout>::with_key();
        let layout = layouts.insert(loaded);
        let mut system = ScrollingLayoutSystem::from(SavedSystem { layouts });
        let screen = CGRect::new(CGPoint::ZERO, CGSize::new(1000.0, 800.0));
        system.prepare_layout(
            layout,
            screen,
            &HashMap::default(),
            &GapSettings::default(),
            crate::layout_engine::Shadow::Run,
        );
        assert_eq!(system.selected_window(layout), Some(wid(2)));
        assert_eq!(system.container_tree(layout).children[0].node_id, 42);
        assert_eq!(
            presented_frames(&system, layout).next().unwrap().1.size.width,
            600.0
        );
        system.update(WindowChange::Add(layout, wid(3), None));
        assert_ne!(system.container_tree(layout).children[1].node_id, 42);
    }
}

#[cfg(test)]
mod core_geometry_tests {
    use super::*;
    use crate::layout_engine::systems::drawn::*;
    use crate::layout_engine::systems::{GroupEdit, LayoutEdit};

    /// How many strips a drawn test here draws: `default`, or `RIFT_SCROLLING_CASES` (a cut for
    /// `mise run check-fast`, or a raise for plant runs).
    fn cases(default: usize) -> usize {
        std::env::var("RIFT_SCROLLING_CASES").map_or(default, |n| n.parse().unwrap())
    }
    fn drawn_settings(rng: &mut Rng) -> ScrollingLayoutSettings {
        let ratio = |rng: &mut Rng| match rng.below(6) {
            0 => f64::NAN,
            1 => rng.length(0.0, 0.05),
            _ => rng.length(0.05, 1.2),
        };
        ScrollingLayoutSettings {
            column_width_ratio: ratio(rng),
            min_column_width_ratio: ratio(rng),
            max_column_width_ratio: ratio(rng),
            preserve_window_sizes: rng.chance(40),
            expand_single_column: rng.chance(50),
            alignment: match rng.below(3) {
                0 => ScrollingAlignment::Left,
                1 => ScrollingAlignment::Center,
                _ => ScrollingAlignment::Right,
            },
            focus_navigation_style: if rng.chance(50) {
                ScrollingFocusNavigationStyle::Niri
            } else {
                ScrollingFocusNavigationStyle::Anchored
            },
            ..ScrollingLayoutSettings::default()
        }
    }

    /// A scrolling system after random edits: windows added and removed, every focus, moves,
    /// drops of every kind, joins, consume-or-expel, stacks, unstacks, unjoins, swaps, resizes,
    /// preset widths, fullscreen, participation, settings changes, arranges (which fix a column's
    /// width when the strip preserves window sizes).
    fn drawn_system(rng: &mut Rng) -> (ScrollingLayoutSystem, LayoutId, Vec<WindowId>) {
        use crate::common::config::StackDefaultOrientation;
        let mut system = ScrollingLayoutSystem::new(&drawn_settings(rng));
        let layout = system.create_layout();
        let mut windows: Vec<WindowId> = Vec::new();
        let mut next = 0;
        let orientation = |rng: &mut Rng| match rng.below(4) {
            0 => StackDefaultOrientation::Perpendicular,
            1 => StackDefaultOrientation::Same,
            2 => StackDefaultOrientation::Horizontal,
            _ => StackDefaultOrientation::Vertical,
        };
        for _ in 0..rng.below(36) {
            let pick = |rng: &mut Rng, windows: &[WindowId]| {
                windows[rng.below(windows.len() as u64) as usize]
            };
            match rng.below(16) {
                0..=3 => {
                    next += 1;
                    let window = WindowId::new(1 + rng.below(3) as i32, next);
                    windows.push(window);
                    system.update(WindowChange::Add(layout, window, None));
                }
                _ if windows.is_empty() => {}
                4 => {
                    let window = pick(rng, &windows);
                    windows.retain(|w| *w != window);
                    system.update(WindowChange::Remove(window));
                }
                5 => {
                    let focus = match rng.below(4) {
                        0 => Focus::Select(pick(rng, &windows)),
                        1 => Focus::Ascend,
                        2 => Focus::Descend,
                        _ => Focus::Direction(drawn_direction(rng)),
                    };
                    system.focus(layout, focus);
                }
                6 => {
                    let within_gaps = rng.chance(50);
                    system.edit(layout, LayoutEdit::Fullscreen { within_gaps });
                }
                7 => {
                    let window = pick(rng, &windows);
                    system.set_window_participation(window, rng.chance(40));
                }
                8 => {
                    let direction = drawn_direction(rng);
                    let has_display_neighbor = rng.chance(30);
                    system.edit(layout, LayoutEdit::Move {
                        direction,
                        has_display_neighbor,
                    });
                }
                9 => {
                    let (source, target) = (pick(rng, &windows), pick(rng, &windows));
                    let action = match rng.below(5) {
                        0 => WindowDropAction::Swap,
                        1 => WindowDropAction::Stack,
                        2 => WindowDropAction::Insert(drawn_direction(rng)),
                        3 => WindowDropAction::Move(drawn_direction(rng)),
                        _ => WindowDropAction::ConsumeOrExpel(drawn_direction(rng)),
                    };
                    system.edit(layout, LayoutEdit::Drop { source, target, action });
                }
                10 => {
                    let edit = match rng.below(5) {
                        0 => GroupEdit::Join(drawn_direction(rng)),
                        1 => GroupEdit::ConsumeOrExpel(drawn_direction(rng)),
                        2 => GroupEdit::Stack(orientation(rng)),
                        3 => GroupEdit::Unstack(orientation(rng)),
                        _ => GroupEdit::Unjoin,
                    };
                    system.edit(layout, LayoutEdit::Group(edit));
                }
                11 => {
                    let (a, b) = (pick(rng, &windows), pick(rng, &windows));
                    system.edit(layout, LayoutEdit::Swap(a, b));
                }
                12 => {
                    let amount = rng.length(-0.6, 0.6);
                    let orientation = match rng.below(3) {
                        0 => ResizeOrientation::Horizontal,
                        1 => ResizeOrientation::Vertical,
                        _ => ResizeOrientation::Smart,
                    };
                    system.edit(layout, LayoutEdit::Resize { amount, orientation }).discard();
                }
                13 => {
                    system.switch_preset_column_width(layout, rng.chance(50));
                }
                14 => {
                    let screen = drawn_screen(rng);
                    let constraints = drawn_constraints_for(rng, &windows);
                    let gaps = drawn_gaps(rng);
                    system.prepare(
                        layout,
                        screen,
                        &constraints,
                        None,
                        &gaps,
                        crate::layout_engine::Shadow::Run,
                    );
                }
                _ => {
                    system.update_settings(&drawn_settings(rng));
                }
            }
        }
        (system, layout, windows)
    }

    /// The core places a scrolling strip as the scrolling system's own placement does (its
    /// shadow, upstream's `Geometry::build`), bit for bit:
    /// columns, world frames, the camera's resting points and bounds, over drawn systems,
    /// settings, screens, gaps and window constraints.
    #[test]
    fn the_core_places_scrolling_strips_as_the_scrolling_system_does() {
        let mut rng = Rng(0x5eed_5c01);
        let (mut cases, mut frames_seen) = (0, 0);
        for _ in 0..self::cases(3000) {
            let (system, layout, windows) = drawn_system(&mut rng);
            let screen = drawn_screen(&mut rng);
            let constraints = drawn_constraints_for(&mut rng, &windows);
            let gaps = drawn_gaps(&mut rng);
            let view = system.view(layout).unwrap();
            let tiling = compute_tiling_area(screen, &gaps);
            let theirs = shadow_placement(
                &view,
                &system.settings,
                system.core.left_out(),
                tiling,
                &constraints,
                &gaps,
            );
            let core = crate::layout_engine::core_geometry::tests::rust_placement_for_test(strip(
                &view,
                &system.settings,
                system.core.left_out(),
                screen,
                &constraints,
                &gaps,
            ));
            let words = crate::layout_engine::core_geometry::tests::placement_words_for_test;
            assert_eq!(words(&core), words(&theirs), "{}", system.draw_tree(layout));
            cases += 1;
            frames_seen += theirs.frames.len();
        }
        eprintln!("scrolling: {cases} strips, {frames_seen} frames identical");
        assert!(
            frames_seen > self::cases(3000),
            "the draws reach frames ({frames_seen})"
        );
    }

    /// An offset that puts a world frame at a parking edge, just past it, or anywhere
    /// (fractional, so rounding is exercised).
    fn edge_offset(rng: &mut Rng, frame: CGRect, tiling: CGRect) -> f64 {
        let left_edge = frame.origin.x + frame.size.width;
        let right_edge = frame.origin.x - tiling.size.width;
        let nudge = |rng: &mut Rng| match rng.below(4) {
            0 => 0.0,
            1 => 1e-9,
            2 => -1e-9,
            _ => rng.length(-1.0, 1.0),
        };
        match rng.below(4) {
            0 => left_edge + nudge(rng),
            1 => right_edge + nudge(rng),
            2 => rng.length(-3000.0, 6000.0),
            _ => rng.length(-3000.0, 6000.0).round() + 0.5,
        }
    }

    /// The core shows a placed strip as upstream's `translate_frames` does, bit for bit: drawn
    /// systems (fullscreen windows among them), each placed, then shown by cameras at and past
    /// the parking edges and at fractional offsets, parking or not.
    #[test]
    fn the_core_shows_scrolling_strips_as_the_scrolling_system_does() {
        use rift_core::layout::scrolling::translate;
        let mut rng = Rng(0x5eed_7a45);
        let (mut cases, mut frames_seen, mut parked) = (0, 0, 0);
        for _ in 0..self::cases(3000) {
            let (mut system, layout, windows) = drawn_system(&mut rng);
            let screen = drawn_screen(&mut rng);
            let constraints = drawn_constraints_for(&mut rng, &windows);
            let gaps = drawn_gaps(&mut rng);
            system.prepare(
                layout,
                screen,
                &constraints,
                None,
                &gaps,
                crate::layout_engine::Shadow::Run,
            );
            let state = &system.layouts[layout];
            let view = system.view(layout).unwrap();
            let Some(g) = state.geometry.as_ref() else { continue };
            for _ in 0..4 {
                let offset = match g.frames.first() {
                    Some(&(_, frame)) => edge_offset(&mut rng, frame, g.tiling),
                    None => rng.length(-3000.0, 6000.0),
                };
                let park = rng.chance(70);
                let mut shown_state = state.clone();
                shown_state.camera.viewport = Viewport::Static(offset);
                let upstream =
                    ScrollingLayoutSystem::shadow_translation(&view, &shown_state, g, park);
                let (world, camera, fullscreen) = shown(&view, g, offset, park);
                let core: Vec<(WindowId, CGRect)> = translate(&world, camera, &fullscreen)
                    .into_iter()
                    .map(|(window, frame)| (window, crate::layout_engine::cg_rect(frame)))
                    .collect();
                assert_eq!(bits(&core), bits(&upstream), "offset {offset}, park {park}");
                cases += 1;
                frames_seen += upstream.len();
                parked += upstream
                    .iter()
                    .filter(|(_, f)| {
                        f.max().x <= g.screen.origin.x + 1.0 || f.origin.x >= g.screen.max().x - 1.0
                    })
                    .count();
            }
        }
        eprintln!(
            "scrolling shown: {cases} cameras, {frames_seen} frames identical, {parked} parked"
        );
        assert!(
            frames_seen > self::cases(3000) && parked > self::cases(3000) / 10,
            "the draws reach frames and parking"
        );
    }

    /// The core's arrangement takes every operation as the scrolling system does (scrolling slice
    /// 1: the system decides, the core is its shadow, and each operation is held to it in tests):
    /// drawn systems (every edit, focus, drop, participation, arrange and settings change), then
    /// replacements, an app closing, native resizes, clones and removals of layouts, gestures
    /// settling on a column, and every query, each reached many times.
    #[test]
    fn the_core_arranges_scrolling_strips_as_the_scrolling_system_does() {
        let mut rng = Rng(0x5eed_c011);
        let mut reached = [0u32; 8];
        let systems = cases(600);
        for _ in 0..systems {
            let (mut system, layout, mut windows) = drawn_system(&mut rng);
            for _ in 0..12 {
                let pick = |rng: &mut Rng, windows: &[WindowId]| {
                    windows.get(rng.below(windows.len().max(1) as u64) as usize).copied()
                };
                match rng.below(7) {
                    0 => {
                        if let Some(from) = pick(&mut rng, &windows) {
                            let to = WindowId::new(from.pid, 1000 + rng.below(100) as u32);
                            if !windows.contains(&to) {
                                system.update(WindowChange::Replace(from, to));
                                windows.iter_mut().filter(|w| **w == from).for_each(|w| *w = to);
                                reached[0] += 1;
                            }
                        }
                    }
                    1 => {
                        let pid = 1 + rng.below(3) as i32;
                        system.update(WindowChange::CloseApp(pid));
                        windows.retain(|w| w.pid != pid);
                        reached[1] += 1;
                    }
                    2 => {
                        if let Some(window) = pick(&mut rng, &windows) {
                            let screen = drawn_screen(&mut rng);
                            let gaps = drawn_gaps(&mut rng);
                            let frame = |rng: &mut Rng| {
                                CGRect::new(
                                    CGPoint::new(rng.length(0.0, 400.0), rng.length(0.0, 300.0)),
                                    CGSize::new(rng.length(50.0, 1400.0), rng.length(50.0, 900.0)),
                                )
                            };
                            let (old, new) = (frame(&mut rng), frame(&mut rng));
                            system.update(WindowChange::Resize {
                                layout,
                                window,
                                old,
                                new,
                                screen,
                                gaps: &gaps,
                            });
                            reached[2] += 1;
                        }
                    }
                    3 => {
                        let clone = system.clone_layout(layout);
                        if rng.chance(50) {
                            system.remove_layout(clone);
                        }
                        reached[3] += 1;
                    }
                    4 => {
                        let screen = drawn_screen(&mut rng);
                        let constraints = drawn_constraints_for(&mut rng, &windows);
                        let gaps = drawn_gaps(&mut rng);
                        system.prepare(
                            layout,
                            screen,
                            &constraints,
                            None,
                            &gaps,
                            crate::layout_engine::Shadow::Run,
                        );
                        let start = Duration::from_millis(rng.below(1000));
                        if system.begin_viewport_gesture(layout, Instant::now()) {
                            system.update_viewport_gesture(
                                layout,
                                rng.length(-2.0, 2.0),
                                start + Duration::from_millis(16),
                            );
                            if system
                                .end_viewport_gesture(
                                    layout,
                                    start + Duration::from_millis(32),
                                    false,
                                    Instant::now(),
                                )
                                .is_some()
                            {
                                reached[4] += 1;
                            }
                        } else if system.snap_to_nearest_column(layout).is_some() {
                            reached[5] += 1;
                        }
                    }
                    _ => {
                        let direction = drawn_direction(&mut rng);
                        system.selected_window(layout);
                        system.window_in_direction(layout, direction);
                        system.has_any_fullscreen_node(layout);
                        system.parent_of_selection_is_stacked(layout);
                        for query in [
                            WindowQuery::All,
                            WindowQuery::Visible,
                            WindowQuery::Selection,
                        ] {
                            system.windows(layout, query);
                        }
                        system.has_windows_for_app(layout, 1 + rng.below(3) as i32);
                        if let Some(window) = pick(&mut rng, &windows) {
                            system.contains_window(layout, window);
                            system.window_slot(layout, window);
                            if system.can_move_window(layout, window, direction) {
                                reached[6] += 1;
                            }
                        }
                        reached[7] += 1;
                    }
                }
            }
        }
        // Each kind is reached 20 times in 600 systems, proportionally in fewer.
        let needed = (20 * systems / 600).max(1) as u32;
        assert!(reached.iter().all(|&n| n >= needed), "reached {reached:?}");
    }

    /// A saved system holding `columns` (each `(id, rows, active_window, width)`) and these
    /// saved fields after them, one layout.
    fn saved(columns: &[(u64, &str, usize, &str)], rest: &str) -> String {
        let columns: Vec<String> = columns
            .iter()
            .map(|(id, rows, active, width)| {
                format!("(id:{id},rows:[{rows}],active_window:{active},width:{width})")
            })
            .collect();
        format!(
            "(layouts:[(value:None,version:0),(value:Some((columns:[{}],{rest})),version:1)])",
            columns.join(",")
        )
    }

    fn row(idx: u32, height: &str) -> String {
        format!("(window:(pid:1,idx:{idx}),height:{height})")
    }

    /// A fullscreen mode saved for a window no column holds is dropped at the load (a
    /// `scrolling-load:` warn), and the file saves without it.
    #[test]
    fn a_saved_fullscreen_mode_for_a_window_in_no_column_is_dropped() {
        let columns = [
            (2, row(1, "1.0"), 0, "Default"),
            (4, row(2, "1.0"), 0, "Default"),
        ];
        let columns: Vec<(u64, &str, usize, &str)> =
            columns.iter().map(|(id, r, a, w)| (*id, r.as_str(), *a, *w)).collect();
        let stale = saved(
            &columns,
            "active_column:1,next_column_id:6,viewport:0.0,fullscreen_modes:{(pid:1,idx:9):Screen,(pid:1,idx:2):WithinGaps}",
        );
        let loaded: ScrollingLayoutSystem = ron::from_str(&stale).unwrap();
        let layout = loaded.layouts.keys().next().unwrap();
        assert_eq!(
            loaded.view(layout).unwrap().fullscreen.keys().collect::<Vec<_>>(),
            vec![&WindowId::new(1, 2)]
        );
        assert!(
            ron::to_string(&loaded)
                .unwrap()
                .contains("fullscreen_modes:{(pid:1,idx:2):WithinGaps}")
        );
    }

    /// States only a saved file holds, loaded as rift loads them and stepped through every kind
    /// of operation, each held to the core's arrangement (the shadow): a fullscreen mode for a
    /// window in no row, a window in two columns, two columns with one id, heights of zero, below
    /// zero and not a number, widths not a number or below zero, active indices past the end, and
    /// the legacy fields (windows with height weights, fullscreen sets, a selected window).
    #[test]
    fn states_only_a_file_holds_step_through_the_core_as_the_system_does() {
        let rest = "active_column:0,next_column_id:6,viewport:0.0";
        let (r1, r2, r3) = (row(1, "1.0"), row(2, "1.0"), row(3, "1.0"));
        let files = [
            // A fullscreen mode for a window no row holds.
            saved(
                &[(2, &r1, 0, "Default"), (4, &r2, 0, "Default")],
                &format!("{rest},fullscreen_modes:{{(pid:1,idx:9):Screen}}"),
            ),
            // A window in two columns.
            saved(
                &[
                    (2, &format!("{r1},{r2}"), 1, "Default"),
                    (4, &r1, 0, "Default"),
                ],
                rest,
            ),
            // Two columns with one id.
            saved(
                &[
                    (2, &r1, 0, "Default"),
                    (2, &format!("{r2},{r3}"), 0, "Default"),
                ],
                rest,
            ),
            // Heights of zero, below zero, not a number.
            saved(
                &[(
                    2,
                    &format!("{},{},{}", row(1, "0.0"), row(2, "-3.0"), row(3, "NaN")),
                    2,
                    "Default",
                )],
                rest,
            ),
            // Widths not a number or below zero.
            saved(
                &[(2, &r1, 0, "Proportion(NaN)"), (4, &r2, 0, "Fixed(-5.0)")],
                rest,
            ),
            // Active indices past the end.
            saved(
                &[
                    (2, &format!("{r1},{r2}"), 9, "Default"),
                    (4, &r3, 0, "Default"),
                ],
                "active_column:7,next_column_id:6,viewport:0.0",
            ),
            // The legacy fields.
            "(layouts:[(value:None,version:0),(value:Some((columns:[(node_id:0,windows:[(pid:1,idx:1),(pid:1,idx:2)],height_weights:[2.0,1.0],active_window:1,width_offset:0.1),(node_id:0,windows:[(pid:1,idx:3)])],active_column:1,next_column_id:0,viewport:0.0,fullscreen:[(pid:1,idx:3)],fullscreen_within_gaps:[(pid:1,idx:1)],selected:Some((pid:1,idx:1)),column_width_ratio:0.5)),version:1)])"
                .to_owned(),
        ];
        let (w1, w2, w3) = (WindowId::new(1, 1), WindowId::new(1, 2), WindowId::new(1, 3));
        for file in &files {
            let mut system: ScrollingLayoutSystem = ron::from_str(file).unwrap();
            let layout = system.layouts.keys().next().unwrap();
            for direction in [
                Direction::Left,
                Direction::Right,
                Direction::Up,
                Direction::Down,
            ] {
                system.window_in_direction(layout, direction);
                system.can_move_window(layout, w1, direction);
            }
            system.selected_window(layout);
            system.has_any_fullscreen_node(layout);
            system.parent_of_selection_is_stacked(layout);
            system.windows(layout, WindowQuery::Selection);
            system.window_slot(layout, w2);
            for edit in [
                LayoutEdit::Resize {
                    amount: 0.1,
                    orientation: ResizeOrientation::Smart,
                },
                LayoutEdit::Resize {
                    amount: -0.2,
                    orientation: ResizeOrientation::Horizontal,
                },
                LayoutEdit::Fullscreen { within_gaps: true },
                LayoutEdit::Move {
                    direction: Direction::Right,
                    has_display_neighbor: false,
                },
                LayoutEdit::Group(GroupEdit::Join(Direction::Left)),
                LayoutEdit::Group(GroupEdit::Unstack(
                    crate::common::config::StackDefaultOrientation::Same,
                )),
                LayoutEdit::Group(GroupEdit::Stack(
                    crate::common::config::StackDefaultOrientation::Same,
                )),
                LayoutEdit::Drop {
                    source: w3,
                    target: w1,
                    action: WindowDropAction::Insert(Direction::Down),
                },
                LayoutEdit::Swap(w1, w2),
            ] {
                system.edit_layout(layout, edit);
            }
            system.focus(layout, Focus::Select(w2));
            system.switch_preset_column_width(layout, false);
            system.set_window_participation(w1, false);
            system.focus(layout, Focus::Direction(Direction::Left));
            system.update(WindowChange::Replace(w2, WindowId::new(1, 7)));
            system.update(WindowChange::Remove(w3));
            system.update(WindowChange::CloseApp(1));
        }
    }
}
