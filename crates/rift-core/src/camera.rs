//! The scrolling layouts' cameras (engine slice E3-cam): for each layout, where its viewport is
//! on the strip and how it comes to rest, the gesture's recent motion, the views it remembers,
//! and the view it owes a fullscreen window.
//!
//! The camera keeps nothing of the geometry or the arrangement. Each event that reads them
//! carries them: [`Spans`], the strip's laid-out columns, bounds, view width and resting points
//! (a projection of the host's prepared-frame cache), and [`Facts`], the columns core's
//! arrangement (its columns, active column, selection and fullscreen windows). The clock is an
//! input too: every event that samples, aims or springs carries a [`Time`], and a spring keeps
//! the times it was given, never a duration.
//!
//! The camera's choices are answers ([`Answer`]): a column it chooses (a gesture's release, a
//! snap, a fullscreen's return), a scroll's excess past the strip's ends. The host has the
//! columns core make a chosen column active; the camera never writes the arrangement.
//!
//! Three invariants (upstream #718, kept from the slice that shaped these types):
//! - every geometry change settles the viewport at one point ([`Camera::set_geometry`]);
//! - a presented position never overwrites a target the layout set ([`Viewport::presented`]);
//! - [`Showing`] changes only on the columns core's answers (a fullscreen toggle's outcome, a
//!   removal or replacement naming the window), and is reconciled at the settle point.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// A moment on the host's monotonic clock, in nanoseconds: it crosses exactly and compares
/// exactly; it becomes seconds (`f64`) only inside the spring and velocity arithmetic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time(pub u64);

impl Time {
    /// Seconds from `earlier` to this moment; none before it.
    pub fn seconds_since(self, earlier: Time) -> f64 {
        self.0.saturating_sub(earlier.0) as f64 / 1_000_000_000.0
    }

    /// `nanos` later.
    pub fn plus_nanos(self, nanos: u64) -> Time { Time(self.0.saturating_add(nanos)) }
}

/// A view remembered by the column it showed, relative to that column so edits to preceding
/// columns reconcile naturally.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bookmark<C> {
    pub column: C,
    pub relative_offset: f64,
}

/// How a viewport that is not being dragged comes to rest. `Centred` is the view
/// "center the selected column" asks for: it may lie past the strip bounds on purpose, so an
/// edit re-centres it on the selected column instead of clamping it (upstream #718).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rest {
    At,
    Centred,
}

/// √800, the spring's natural frequency (stiffness 800, mass 1).
const OMEGA: f64 = 28.284271247461902;

/// ln 0.997, niri's touchpad deceleration per millisecond.
pub const DECELERATION_LN: f64 = -0.0030045090202987243;

/// e^`x`, from `+`, `*` and floor only, so the Rust and Roc cores compute it bit for bit alike
/// (their libraries' `exp` differ in the last bits, and Roc has none). Range reduction by ln 2
/// (fdlibm's split constants), a degree-13 Taylor polynomial on |r| ≤ ln 2 / 2 (error below
/// 1e-17 relative), and 2^k built from its bits; below 2^-1020 it is 0, above 2^1023 infinite.
pub fn exp(x: f64) -> f64 {
    const LN2_HI: f64 = 6.931_471_803_691_238e-1;
    const LN2_LO: f64 = 1.908_214_929_270_587_7e-10;
    // log2(e), 1.4426950408889634: the literal the Roc core writes.
    const INV_LN2: f64 = std::f64::consts::LOG2_E;
    const C: [f64; 12] = [
        1.6059043836821613e-10,
        2.08767569878681e-9,
        2.505210838544172e-8,
        2.755731922398589e-7,
        2.7557319223985893e-6,
        2.48015873015873e-5,
        1.984126984126984e-4,
        1.388888888888889e-3,
        8.333333333333333e-3,
        4.1666666666666664e-2,
        1.6666666666666666e-1,
        0.5,
    ];
    if x.is_nan() {
        return x;
    }
    let k = (x * INV_LN2 + 0.5).floor();
    if k < -1020.0 {
        return 0.0;
    }
    if k > 1023.0 {
        return f64::INFINITY;
    }
    let r = (x - k * LN2_HI) - k * LN2_LO;
    let mut p = C[0];
    for c in &C[1..] {
        p = p * r + c;
    }
    p = p * r + 1.0;
    p = p * r + 1.0;
    p * f64::from_bits(((k as i64 + 1023) as u64) << 52)
}

/// Niri's default horizontal-view-movement: mass 1, critical damping, stiffness 800.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub from: f64,
    pub target: f64,
    pub velocity: f64,
    pub current: f64,
    pub current_velocity: f64,
    pub started: Time,
    pub sampled: Time,
}

impl Spring {
    pub fn new(from: f64, target: f64, velocity: f64, started: Time) -> Self {
        Spring {
            from,
            target,
            velocity,
            current: from,
            current_velocity: velocity,
            started,
            sampled: started,
        }
    }

    /// Where the spring is at `now`, and how fast it moves.
    pub fn position_velocity(&self, now: Time) -> (f64, f64) {
        let t = now.seconds_since(self.started);
        let omega = OMEGA;
        let x0 = self.from - self.target;
        let b = omega * x0 + self.velocity;
        let envelope = exp(-omega * t);
        (
            self.target + envelope * (x0 + b * t),
            envelope * (self.velocity - omega * b * t),
        )
    }

    /// Advanced to `now` (never back); whether it has come to rest, on a display at `scale`.
    pub fn sample(&mut self, now: Time, scale: f64) -> bool {
        let now = now.max(self.sampled);
        let (position, velocity) = self.position_velocity(now);
        self.current = position;
        self.current_velocity = velocity;
        self.sampled = now;
        // Check both position and speed so crossing the target cannot finish early.
        (self.current - self.target).abs() <= 0.25 / scale && velocity.abs() / OMEGA <= 0.25 / scale
    }
}

/// How a resting viewport gets to where it rests.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Motion {
    Static(f64),
    /// The previous position kept until the presenter starts the new target.
    Pending {
        from: f64,
        target: f64,
    },
    Animation(Spring),
}

impl Motion {
    fn towards(from: f64, target: f64) -> Self {
        if from == target {
            Self::Static(target)
        } else {
            Self::Pending { from, target }
        }
    }

    fn offset(&self) -> f64 {
        match self {
            Self::Static(offset) => *offset,
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

    pub fn target(&self) -> f64 {
        match self {
            Self::Animation(spring) => spring.target,
            _ => self.offset(),
        }
    }

    fn sample(&mut self, now: Time, scale: f64) -> Option<bool> {
        match self {
            Self::Static(_) => None,
            Self::Pending { target, .. } => {
                *self = Self::Static(*target);
                Some(false)
            }
            // Bounds constrain the destination. After a column closes, the previous position
            // can lie outside them and must remain visible while returning.
            Self::Animation(spring) => {
                if spring.sample(now, scale) {
                    *self = Self::Static(spring.target);
                    Some(false)
                } else {
                    Some(true)
                }
            }
        }
    }

    /// Move where the motion comes to rest, whether it is at rest or still on its way.
    fn move_target(&mut self, target: f64, now: Time) {
        match self {
            Self::Static(_) | Self::Pending { .. } => {
                if target != self.target() {
                    *self = Self::towards(self.start_offset(), target);
                }
            }
            Self::Animation(spring) => {
                if target != spring.target {
                    let now = now.max(spring.sampled);
                    let (_, velocity) = spring.position_velocity(now);
                    *spring = Spring::new(spring.current, target, velocity, now);
                }
            }
        }
    }

    fn rebase(&mut self, delta: f64) {
        match self {
            Self::Static(offset) => *offset += delta,
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

/// Where a layout's camera is on the strip.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Viewport {
    #[default]
    Uninitialized,
    Gesture(f64),
    Resting(Rest, Motion),
}

impl Viewport {
    /// At rest at `offset`, not centred.
    pub fn at(offset: f64) -> Self { Self::Resting(Rest::At, Motion::Static(offset)) }

    /// A saved offset: at rest there, or at 0 for one that is not finite.
    pub fn saved(offset: f64) -> Self { Self::at(if offset.is_finite() { offset } else { 0.0 }) }

    /// On a spring from `from` to `target`, coming to rest as `rest` says.
    pub fn springing(rest: Rest, from: f64, target: f64, velocity: f64, started: Time) -> Self {
        Self::Resting(
            rest,
            Motion::Animation(Spring::new(from, target, velocity, started)),
        )
    }

    /// The requested offset (layout queries use it).
    pub fn offset(&self) -> f64 {
        match self {
            Self::Uninitialized => 0.0,
            Self::Gesture(offset) => *offset,
            Self::Resting(_, motion) => motion.offset(),
        }
    }

    /// The offset the presenter still shows until it starts the animation.
    pub fn start_offset(&self) -> f64 {
        match self {
            Self::Resting(_, motion) => motion.start_offset(),
            Self::Uninitialized | Self::Gesture(_) => self.offset(),
        }
    }

    pub fn target(&self) -> f64 {
        match self {
            Self::Resting(_, motion) => motion.target(),
            Self::Uninitialized | Self::Gesture(_) => self.offset(),
        }
    }

    /// Being dragged or on a spring: the offset is a moment, not where it rests.
    pub fn moving(&self) -> bool {
        matches!(self, Self::Gesture(_) | Self::Resting(_, Motion::Animation(_)))
    }

    pub fn uncentre(&mut self) {
        if let Self::Resting(rest, _) = self {
            *rest = Rest::At;
        }
    }

    /// At rest at its target.
    pub fn finish(&mut self) {
        if let Self::Resting(_, motion) = self {
            *motion = Motion::Static(motion.target());
        }
    }

    /// Rest at `target`, no longer centred.
    pub fn set_target(&mut self, target: f64) { self.aim(target, Rest::At); }

    pub fn aim(&mut self, target: f64, rest: Rest) {
        *self = Self::Resting(rest, Motion::towards(self.start_offset(), target));
    }

    /// Advanced to `now`: whether it is still moving, or none for one that has nothing to
    /// advance.
    pub fn sample(&mut self, now: Time, scale: f64) -> Option<bool> {
        match self {
            Self::Resting(_, motion) => motion.sample(now, scale),
            Self::Uninitialized | Self::Gesture(_) => None,
        }
    }

    fn rebase(&mut self, delta: f64) {
        match self {
            Self::Uninitialized => {}
            Self::Gesture(offset) => *offset += delta,
            Self::Resting(_, motion) => motion.rebase(delta),
        }
    }

    /// The presenter's camera taken back, `ended` when nothing presents it any more. The
    /// layout owns where the camera rests; the presenter only moves it there. So its position is
    /// taken only out of a gesture, or out of a resting viewport on its way to the very target
    /// the presenter shows. A viewport the layout has aimed elsewhere since (a reveal, a settle
    /// into bounds after an edit) keeps its own target: a presented position can never
    /// overwrite one the layout set.
    pub fn presented(self, presented: &Presented, ended: Ended) -> Self {
        match (self, presented) {
            (Self::Gesture(_), Presented::Gesture(offset)) => Self::Gesture(*offset),
            // A released gesture rests where the presenter has it.
            (Self::Gesture(_), Presented::Still(offset)) => Self::at(*offset),
            (Self::Gesture(_), Presented::Moving(spring)) => match ended {
                Ended::No => Self::Resting(Rest::At, Motion::Animation(*spring)),
                Ended::Yes => Self::at(spring.target),
            },
            (Self::Resting(rest, motion), Presented::Still(offset))
                if motion.target() == *offset =>
            {
                Self::Resting(rest, Motion::Static(*offset))
            }
            (Self::Resting(rest, motion), Presented::Moving(spring))
                if motion.target() == spring.target =>
            {
                match ended {
                    Ended::No => Self::Resting(rest, Motion::Animation(*spring)),
                    Ended::Yes => Self::Resting(rest, Motion::Static(spring.target)),
                }
            }
            (viewport, _) => viewport,
        }
    }
}

/// Where the presenter has the camera. How it comes to rest stays the layout's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Presented {
    Gesture(f64),
    Still(f64),
    Moving(Spring),
}

/// Whether a presented camera is still presented (`No`) or has ended (`Yes`): an ended one
/// leaves its viewport at the target, nothing advancing it any more.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    No,
    Yes,
}

/// Recent camera deltas in pixels, independent of viewport rebasing, each at the time the
/// gesture reported it; and how far the hand has pulled past the strip's ends.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MotionHistory {
    samples: VecDeque<(Time, f64)>,
    overscroll: f64,
}

impl MotionHistory {
    /// The recorded deltas, oldest first.
    pub fn samples(&self) -> impl Iterator<Item = (Time, f64)> + '_ { self.samples.iter().copied() }

    /// How far the hand has pulled past the strip's ends, in pixels.
    pub fn overscroll(&self) -> f64 { self.overscroll }

    /// A history holding `samples` and `overscroll`.
    pub fn from_parts(samples: impl IntoIterator<Item = (Time, f64)>, overscroll: f64) -> Self {
        MotionHistory {
            samples: samples.into_iter().collect(),
            overscroll,
        }
    }

    /// One delta at `time`; refused (false) when older than the last.
    pub fn push(&mut self, delta: f64, time: Time) -> bool {
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
            .is_some_and(|(first, _)| time.0.saturating_sub(first.0) > 150_000_000)
        {
            self.samples.pop_front();
        }
        true
    }

    /// A gesture's `delta` at `time`, applied to `offset` within `bounds` (the excess kept as
    /// overscroll); how far the offset moved, or none for a delta refused.
    pub fn update(
        &mut self,
        offset: &mut f64,
        bounds: (f64, f64),
        delta: f64,
        time: Time,
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

    /// Pixels per second over the recorded deltas.
    pub fn velocity(&self) -> f64 {
        let (Some(&(first, _)), Some(&(last, _))) = (self.samples.front(), self.samples.back())
        else {
            return 0.0;
        };
        let dt = last.seconds_since(first);
        if dt > 0.0 {
            self.samples.iter().map(|(_, delta)| delta).sum::<f64>() / dt
        } else {
            0.0
        }
    }
}

/// The view the camera owes a fullscreen window: none (`Strip`), or the strip's view to go back
/// to once `window` is no longer fullscreen. Not a second fullscreen fact: whether `window` is
/// fullscreen is the columns core's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Showing<W, C> {
    Strip,
    Fullscreen { window: W, return_to: Bookmark<C> },
}

impl<W: PartialEq + Copy, C: Copy> Showing<W, C> {
    /// The return owed to `window`, taken (the camera back to owing nothing); none if it is owed
    /// to another window, or to none.
    fn take_owed_to(&mut self, window: W) -> Option<Bookmark<C>> {
        match *self {
            Self::Fullscreen { window: owed, return_to } if owed == window => {
                *self = Self::Strip;
                Some(return_to)
            }
            _ => None,
        }
    }
}

/// A laid-out column's place on the strip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span<C> {
    pub id: C,
    pub world_x: f64,
    pub width: f64,
}

/// A camera offset the camera may rest at, and the column it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snap<C> {
    pub offset: f64,
    pub column: C,
}

/// The strip as laid out, as the camera reads it: the laid-out columns in strip order (a
/// dormant column has none), the resting bounds, the view's width and the resting points.
/// A projection of the host's prepared geometry, carried by each event that reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct Spans<C> {
    pub columns: Vec<Span<C>>,
    pub bounds: (f64, f64),
    pub view: f64,
    pub snaps: Vec<Snap<C>>,
}

impl<C: Copy + PartialEq> Spans<C> {
    fn column(&self, id: C) -> Option<Span<C>> { self.columns.iter().find(|c| c.id == id).copied() }

    /// The nearest resting position within the strip bounds.
    pub fn clamp(&self, offset: f64) -> f64 { offset.clamp(self.bounds.0, self.bounds.1) }

    /// Where the column at `index` rests under `alignment`.
    pub fn anchor_offset(&self, index: usize, alignment: Alignment) -> f64 {
        let column = self.columns[index];
        let view = self.view;
        if column.width >= view {
            return column.world_x;
        }
        // Rift's anchored first/last-column policy.
        if self.columns.len() > 1 {
            if index == 0 {
                return column.world_x;
            }
            if index + 1 == self.columns.len() {
                return column.world_x + column.width - view;
            }
        }
        match alignment {
            Alignment::Left => column.world_x,
            Alignment::Center => centered_offset(view, column),
            Alignment::Right => column.world_x + column.width - view,
        }
    }
}

/// A column of the arrangement: its id and its windows.
#[derive(Clone, Debug, PartialEq)]
pub struct FactColumn<C, W> {
    pub id: C,
    pub windows: Vec<W>,
}

/// The columns core's arrangement, as the camera reads it at each entry point.
#[derive(Clone, Debug, PartialEq)]
pub struct Facts<C, W> {
    pub columns: Vec<FactColumn<C, W>>,
    pub active_column: usize,
    pub selected: Option<W>,
    pub fullscreen: BTreeSet<W>,
}

impl<C: Copy + PartialEq, W: Copy + Ord> Facts<C, W> {
    fn active_id(&self) -> Option<C> { self.columns.get(self.active_column).map(|c| c.id) }

    fn holds_column(&self, id: C) -> bool { self.columns.iter().any(|c| c.id == id) }

    fn column_of(&self, window: W) -> Option<C> {
        self.columns.iter().find(|c| c.windows.contains(&window)).map(|c| c.id)
    }

    /// Whether the active column shows a fullscreen window.
    fn active_column_fullscreen(&self) -> bool {
        self.columns
            .get(self.active_column)
            .is_some_and(|c| c.windows.iter().any(|w| self.fullscreen.contains(w)))
    }
}

/// Where a revealed column comes to rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Center,
    Right,
}

/// How the camera follows focus: niri's (just enough to show the column) or anchored (each
/// column at its alignment).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Navigation {
    Niri,
    Anchored,
}

/// The settings the camera reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraSettings {
    pub alignment: Alignment,
    pub navigation: Navigation,
}

/// The selection an edit holds still on screen, as it was before the edit: the selected window
/// (if any), its column, and where that column was laid out on the strip. After the edit the
/// camera holds the window's column, or else that column if it is still there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeldAt<C, W> {
    pub window: Option<W>,
    pub column: C,
    pub x: f64,
}

/// Which way a scroll ran past the strip's ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
}

/// Where a release or a snap leaves the camera: the column it chose (which the host has the
/// columns core make active), where it rests, where it was, and the release's speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Release<C> {
    pub column: C,
    pub offset: f64,
    pub from_offset: f64,
    pub velocity: f64,
}

/// What a layout's camera learns, each event with the geometry and arrangement it reads.
#[derive(Clone, Debug, PartialEq)]
pub enum CameraEvent<C, W> {
    /// A layout pass with new geometry `spans` (`before` the geometry it replaces, if any): the
    /// single settle point. A first view starts at the strip's origin and jumps, unanimated, to
    /// where the selection is shown.
    Prepared {
        before: Option<Spans<C>>,
        spans: Spans<C>,
        facts: Facts<C, W>,
        settings: CameraSettings,
        now: Time,
    },
    /// The geometry rebuilt after an edit the columns core made (`spans`; none before the
    /// layout's first pass), the camera holding `hold` still on screen, or else its place on the
    /// strip. Settles.
    Rebuilt {
        spans: Option<Spans<C>>,
        facts: Facts<C, W>,
        hold: Option<HeldAt<C, W>>,
        now: Time,
    },
    /// The camera brought round to show the selected column.
    Revealed {
        spans: Option<Spans<C>>,
        facts: Facts<C, W>,
        settings: CameraSettings,
    },
    /// The camera aimed to centre the selected column, and to stay centred on it.
    Centred {
        spans: Option<Spans<C>>,
        facts: Facts<C, W>,
    },
    /// A window is about to come: the camera remembers its view for when it goes, and stops
    /// restoring a saved one.
    Inserting {
        spans: Option<Spans<C>>,
        facts: Facts<C, W>,
    },
    /// The columns core removed a window (`active_removed`: it was the selection;
    /// `column_removed`: its column went): back to the view remembered from before it came if
    /// the core shows that column, or else the new selection revealed.
    WindowRemoved {
        active_removed: bool,
        column_removed: bool,
        spans: Option<Spans<C>>,
        facts: Facts<C, W>,
        settings: CameraSettings,
    },
    /// The columns core selected another window (`column_changed`: in another column).
    Selected {
        column_changed: bool,
        spans: Option<Spans<C>>,
        facts: Facts<C, W>,
        settings: CameraSettings,
    },
    /// The columns core moved to another column on its own (a participation change): back to
    /// the view remembered, if that is the column, or else revealing it.
    Followed {
        spans: Option<Spans<C>>,
        facts: Facts<C, W>,
        settings: CameraSettings,
    },
    /// The view remembered for a window's going is forgotten (an edit decided otherwise).
    TransientForgotten,
    /// The saved view being restored is forgotten.
    ViewForgotten,
    /// `from` has a new identity, `to`: a return owed to it follows.
    Replaced { from: W, to: W },
    /// The columns core took `window` out of fullscreen: the return owed to it, if the core's
    /// active column shows no fullscreen window; answers the column it shows.
    FullscreenLeft {
        window: W,
        spans: Option<Spans<C>>,
        facts: Facts<C, W>,
    },
    /// The columns core put `window` fullscreen (`before` and `after` its arrangement): the
    /// view before is owed back, unless a return is owed already; the camera moves to the
    /// fullscreen column.
    FullscreenEntered {
        window: W,
        spans: Option<Spans<C>>,
        before: Facts<C, W>,
        after: Facts<C, W>,
    },
    /// At rest at its target.
    Finished,
    /// The presenter's camera and motion taken back.
    Presented {
        presented: Presented,
        motion: MotionHistory,
        ended: Ended,
    },
    /// A scroll by `delta` columns of the selected column's width; answers the excess past the
    /// strip's ends.
    ScrollBy {
        delta: f64,
        spans: Option<Spans<C>>,
        facts: Facts<C, W>,
        gap: f64,
    },
    /// A gesture begins at `now`, if one is available (a laid-out strip whose selection is not
    /// fullscreen); answers whether it began.
    GestureBegan {
        now: Time,
        spans: Option<Spans<C>>,
        facts: Facts<C, W>,
    },
    /// A gesture's `delta` pixels at `at`; answers how far the camera moved.
    GestureMoved {
        delta: f64,
        at: Time,
        spans: Option<Spans<C>>,
    },
    /// A gesture released at `at`, projected by niri's touchpad deceleration and settled on a
    /// resting point; on a spring from `started` when `animate`. Answers the release.
    GestureEnded {
        at: Time,
        animate: bool,
        started: Time,
        spans: Option<Spans<C>>,
        facts: Facts<C, W>,
        settings: CameraSettings,
    },
    /// A gesture cancelled: the camera rests where it is, within bounds.
    GestureCancelled { spans: Option<Spans<C>> },
    /// The camera settled on the nearest resting point; answers the release.
    Snapped {
        spans: Option<Spans<C>>,
        facts: Facts<C, W>,
        settings: CameraSettings,
    },
}

/// What a camera answered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Answer<C> {
    Nothing,
    /// The column the camera chose, for the columns core to make active.
    Activate(C),
    /// Whether a gesture began.
    Began(bool),
    /// How far a gesture moved the camera.
    Moved(f64),
    /// How far a scroll ran past an end, in columns.
    Excess(Edge, f64),
    /// Where a release or snap left the camera.
    Released(Release<C>),
}

/// A layout's camera.
#[derive(Clone, Debug, PartialEq)]
pub struct Camera<C, W> {
    viewport: Viewport,
    motion: MotionHistory,
    transient_restore: Option<Bookmark<C>>,
    showing: Showing<W, C>,
    restored_view: Option<Bookmark<C>>,
}

impl<C, W> Default for Camera<C, W> {
    fn default() -> Self {
        Camera {
            viewport: Viewport::Uninitialized,
            motion: MotionHistory::default(),
            transient_restore: None,
            showing: Showing::Strip,
            restored_view: None,
        }
    }
}

fn fit_offset<C>(current: f64, view: f64, column: Span<C>) -> f64 {
    // Adapted from niri's compute_new_view_offset, using an absolute camera.
    let left = column.world_x;
    if column.width >= view {
        return left;
    }
    current.clamp(left + column.width - view, left)
}

fn centered_offset<C>(view: f64, column: Span<C>) -> f64 {
    column.world_x - (view - column.width).max(0.0) / 2.0
}

impl<C: Copy + PartialEq, W: Copy + Ord> Camera<C, W> {
    /// A saved camera: at rest at `offset`, remembering `transient_restore`, owing `showing`,
    /// restoring `restored_view` on its first pass.
    pub fn saved(
        offset: f64,
        transient_restore: Option<Bookmark<C>>,
        showing: Showing<W, C>,
        restored_view: Option<Bookmark<C>>,
    ) -> Self {
        Camera {
            viewport: Viewport::saved(offset),
            motion: MotionHistory::default(),
            transient_restore,
            showing,
            restored_view,
        }
    }

    /// A camera holding exactly these parts (a twin's reseed, a test's setup).
    pub fn from_parts(
        viewport: Viewport,
        motion: MotionHistory,
        transient_restore: Option<Bookmark<C>>,
        showing: Showing<W, C>,
        restored_view: Option<Bookmark<C>>,
    ) -> Self {
        Camera {
            viewport,
            motion,
            transient_restore,
            showing,
            restored_view,
        }
    }

    pub fn viewport(&self) -> &Viewport { &self.viewport }

    pub fn motion(&self) -> &MotionHistory { &self.motion }

    pub fn transient_restore(&self) -> Option<Bookmark<C>> { self.transient_restore }

    pub fn showing(&self) -> &Showing<W, C> { &self.showing }

    pub fn restored_view(&self) -> Option<Bookmark<C>> { self.restored_view }

    /// The view shown now, by the selected column (none without one laid out).
    pub fn bookmark(&self, spans: &Spans<C>, facts: &Facts<C, W>) -> Option<Bookmark<C>> {
        let column = spans.column(facts.active_id()?)?;
        Some(Bookmark {
            column: column.id,
            relative_offset: self.viewport.offset() - column.world_x,
        })
    }

    /// A copy for a layout clone: a pending move finished, a moving one stopped where it is
    /// (so it no longer rests centred).
    pub fn cloned(&self, spans: Option<&Spans<C>>) -> Self {
        let mut clone = self.clone();
        match clone.viewport {
            Viewport::Resting(_, Motion::Pending { .. }) => clone.viewport.finish(),
            Viewport::Resting(_, Motion::Animation(_)) => {
                let offset = clone.viewport.offset();
                clone.viewport = Viewport::at(spans.map_or(offset, |s| s.clamp(offset)));
            }
            Viewport::Uninitialized
            | Viewport::Gesture(_)
            | Viewport::Resting(_, Motion::Static(_)) => {}
        }
        clone
    }

    /// One event; what the camera answers.
    pub fn step(&mut self, event: CameraEvent<C, W>) -> Answer<C> {
        match event {
            CameraEvent::Prepared {
                before,
                spans,
                facts,
                settings,
                now,
            } => {
                self.prepared(before, spans, &facts, settings, now);
                Answer::Nothing
            }
            CameraEvent::Rebuilt { spans, facts, hold, now } => {
                if let Some(spans) = spans {
                    let offset = self.viewport.offset();
                    let hold = hold.and_then(|held| {
                        let column = (held.window.and_then(|w| facts.column_of(w)))
                            .or_else(|| facts.holds_column(held.column).then_some(held.column))?;
                        Some(Bookmark {
                            column,
                            relative_offset: offset - held.x,
                        })
                    });
                    self.set_geometry(&spans, &facts, hold, now);
                }
                if facts.columns.is_empty() {
                    self.viewport = Viewport::at(0.0);
                }
                Answer::Nothing
            }
            CameraEvent::Revealed { spans, facts, settings } => {
                if let Some(spans) = spans {
                    self.reveal(&spans, &facts, settings);
                }
                Answer::Nothing
            }
            CameraEvent::Centred { spans, facts } => {
                if let Some(spans) = spans
                    && let Some(offset) = Self::centred_offset(&spans, &facts)
                {
                    self.viewport.aim(offset, Rest::Centred);
                }
                Answer::Nothing
            }
            CameraEvent::Inserting { spans, facts } => {
                self.transient_restore = spans.and_then(|spans| self.bookmark(&spans, &facts));
                self.restored_view = None;
                Answer::Nothing
            }
            CameraEvent::WindowRemoved {
                active_removed,
                column_removed,
                spans,
                facts,
                settings,
            } => {
                let restore = if active_removed && column_removed {
                    self.transient_restore.take()
                } else {
                    None
                };
                // Restored only when it is the column the core showed.
                let restored = restore
                    .filter(|bookmark| Some(bookmark.column) == facts.active_id())
                    .and_then(|bookmark| self.restore(spans.as_ref(), &facts, bookmark));
                // A return owed to the window removed was reconciled by the edit's settle.
                if active_removed
                    && restored.is_none()
                    && let Some(spans) = &spans
                {
                    self.reveal(spans, &facts, settings);
                }
                if self.transient_restore.is_some_and(|b| !facts.holds_column(b.column)) {
                    self.transient_restore = None;
                }
                Answer::Nothing
            }
            CameraEvent::Selected {
                column_changed,
                spans,
                facts,
                settings,
            } => {
                // Focusing another column ends a centred view (upstream #718).
                if column_changed {
                    self.viewport.uncentre();
                }
                self.restored_view = None;
                self.transient_restore = None;
                // Selecting another column than the return's owes no return (gate 3).
                if let Showing::Fullscreen { return_to, .. } = &self.showing
                    && Some(return_to.column) != facts.active_id()
                {
                    self.showing = Showing::Strip;
                }
                if let Some(spans) = &spans {
                    self.reveal(spans, &facts, settings);
                }
                Answer::Nothing
            }
            CameraEvent::Followed { spans, facts, settings } => {
                let shown = facts.active_id();
                let restored = self
                    .transient_restore
                    .take()
                    .filter(|bookmark| Some(bookmark.column) == shown)
                    .and_then(|bookmark| self.restore(spans.as_ref(), &facts, bookmark));
                if restored.is_none()
                    && let Some(spans) = &spans
                {
                    self.reveal(spans, &facts, settings);
                }
                Answer::Nothing
            }
            CameraEvent::TransientForgotten => {
                self.transient_restore = None;
                Answer::Nothing
            }
            CameraEvent::ViewForgotten => {
                self.restored_view = None;
                Answer::Nothing
            }
            CameraEvent::Replaced { from, to } => {
                if let Some(return_to) = self.showing.take_owed_to(from) {
                    self.showing = Showing::Fullscreen { window: to, return_to };
                }
                Answer::Nothing
            }
            CameraEvent::FullscreenLeft { window, spans, facts } => {
                match self
                    .showing
                    .take_owed_to(window)
                    .and_then(|return_to| self.consume_return(spans.as_ref(), &facts, return_to))
                {
                    Some(column) => Answer::Activate(column),
                    None => Answer::Nothing,
                }
            }
            CameraEvent::FullscreenEntered { window, spans, before, after } => {
                let bookmark = spans.as_ref().and_then(|spans| self.bookmark(spans, &before));
                // A return already owed is kept: the view before the first fullscreen (gate 1).
                if let (Showing::Strip, Some(return_to)) = (&self.showing, bookmark) {
                    self.showing = Showing::Fullscreen { window, return_to };
                }
                if let Some(column) =
                    spans.as_ref().and_then(|spans| spans.column(after.active_id()?))
                {
                    self.viewport.set_target(column.world_x);
                }
                Answer::Nothing
            }
            CameraEvent::Finished => {
                self.viewport.finish();
                Answer::Nothing
            }
            CameraEvent::Presented { presented, motion, ended } => {
                self.viewport = self.viewport.presented(&presented, ended);
                self.motion = motion;
                Answer::Nothing
            }
            CameraEvent::ScrollBy { delta, spans, facts, gap } => {
                if !delta.is_finite() {
                    return Answer::Nothing;
                }
                let Some(spans) = spans else {
                    return Answer::Nothing;
                };
                let Some(column) = facts.active_id().and_then(|id| spans.column(id)) else {
                    return Answer::Nothing;
                };
                let step = column.width + gap;
                let raw = self.viewport.offset() + delta * step;
                self.viewport.set_target(spans.clamp(raw));
                if raw < spans.bounds.0 {
                    Answer::Excess(Edge::Left, (spans.bounds.0 - raw) / step)
                } else if raw > spans.bounds.1 {
                    Answer::Excess(Edge::Right, (raw - spans.bounds.1) / step)
                } else {
                    Answer::Nothing
                }
            }
            CameraEvent::GestureBegan { now, spans, facts } => {
                Answer::Began(self.gesture_began(now, spans.is_some(), &facts))
            }
            CameraEvent::GestureMoved { delta, at, spans } => {
                if !delta.is_finite() {
                    return Answer::Nothing;
                }
                let (Viewport::Gesture(offset), Some(spans)) = (&mut self.viewport, spans) else {
                    return Answer::Nothing;
                };
                match self.motion.update(offset, spans.bounds, delta, at) {
                    Some(moved) => Answer::Moved(moved),
                    None => Answer::Nothing,
                }
            }
            CameraEvent::GestureEnded {
                at,
                animate,
                started,
                spans,
                facts,
                settings,
            } => {
                let Viewport::Gesture(offset) = self.viewport else {
                    return Answer::Nothing;
                };
                if !self.motion.push(0.0, at) {
                    return Answer::Nothing;
                }
                let velocity = self.motion.velocity();
                let projected = offset - velocity / (1000.0 * DECELERATION_LN);
                let Some(spans) = spans else {
                    return Answer::Nothing;
                };
                let Some(release) =
                    self.settle(&spans, &facts, settings, projected, velocity, true)
                else {
                    return Answer::Nothing;
                };
                if animate {
                    self.viewport = Viewport::springing(
                        Rest::At,
                        release.from_offset,
                        release.offset,
                        release.velocity,
                        started,
                    );
                }
                Answer::Released(release)
            }
            CameraEvent::GestureCancelled { spans } => {
                if self.viewport.moving() {
                    let offset = self.viewport.offset();
                    self.viewport = Viewport::at(spans.map_or(offset, |s| s.clamp(offset)));
                    self.motion = MotionHistory::default();
                }
                Answer::Nothing
            }
            CameraEvent::Snapped { spans, facts, settings } => {
                let offset = self.viewport.offset();
                match spans
                    .and_then(|spans| self.settle(&spans, &facts, settings, offset, 0.0, false))
                {
                    Some(release) => Answer::Released(release),
                    None => Answer::Nothing,
                }
            }
        }
    }

    fn prepared(
        &mut self,
        before: Option<Spans<C>>,
        spans: Spans<C>,
        facts: &Facts<C, W>,
        settings: CameraSettings,
        now: Time,
    ) {
        if self.restored_view.is_none() {
            self.restored_view = before.and_then(|before| self.bookmark(&before, facts));
        }
        let initial = matches!(self.viewport, Viewport::Uninitialized);
        self.set_geometry(&spans, facts, None, now);
        if initial {
            // The first view starts at the strip's origin and jumps, unanimated, to where the
            // selection is shown.
            self.viewport = Viewport::at(0.0);
            if spans.columns.len() == 1 {
                let offset = spans.anchor_offset(0, settings.alignment);
                self.viewport.set_target(offset);
            } else {
                self.reveal(&spans, facts, settings);
            }
            self.viewport.finish();
        }
    }

    /// Every geometry change goes through here, so none can skip settling the viewport
    /// (upstream #718). One bookmarked column keeps its place on screen: the saved view when
    /// its column is laid out (upstream #663), otherwise `hold`. A saved view whose column is
    /// dormant waits for readmission; it is forgotten once its column is gone.
    fn set_geometry(
        &mut self,
        spans: &Spans<C>,
        facts: &Facts<C, W>,
        hold: Option<Bookmark<C>>,
        now: Time,
    ) {
        let (restored, dormant) = match self.restored_view.take() {
            Some(b) if spans.column(b.column).is_some() => (Some(b), None),
            Some(b) if facts.holds_column(b.column) => (None, Some(b)),
            _ => (None, None),
        };
        if let Some(bookmark) = restored.or(hold)
            && let Some(column) = spans.column(bookmark.column)
        {
            let offset = self.viewport.offset();
            self.viewport.rebase(column.world_x + bookmark.relative_offset - offset);
        }
        self.restored_view = dormant;
        self.settle_viewport(spans, facts, now);
    }

    /// After the geometry changes the viewport comes to rest within the strip bounds, or, when
    /// centred, centred on whichever column is selected now. A gesture keeps its overscroll
    /// until it ends.
    fn settle_viewport(&mut self, spans: &Spans<C>, facts: &Facts<C, W>, now: Time) {
        self.reconcile_return(spans, facts);
        let centred = Self::centred_offset(spans, facts);
        let Viewport::Resting(rest, motion) = &mut self.viewport else {
            return;
        };
        let clamped = spans.clamp(motion.target());
        let target = match rest {
            Rest::At => clamped,
            Rest::Centred => centred.unwrap_or(clamped),
        };
        motion.move_target(target, now);
    }

    /// The return owed reconciled with the columns core: if it no longer shows its window
    /// fullscreen, by whatever path, the return is consumed here. From the settle point the
    /// camera cannot make a column active, so it restores only a return to the active column
    /// and drops any other.
    fn reconcile_return(&mut self, spans: &Spans<C>, facts: &Facts<C, W>) {
        let Showing::Fullscreen { window, .. } = self.showing else {
            return;
        };
        if facts.fullscreen.contains(&window) {
            return;
        }
        let Some(return_to) = self.showing.take_owed_to(window) else {
            return;
        };
        if facts.active_id() == Some(return_to.column) {
            self.consume_return(Some(spans), facts, return_to);
        }
    }

    /// A return owed to a window no longer fullscreen, consumed: restored, unless the active
    /// column still shows a fullscreen window (dropped then); the column it shows.
    fn consume_return(
        &mut self,
        spans: Option<&Spans<C>>,
        facts: &Facts<C, W>,
        return_to: Bookmark<C>,
    ) -> Option<C> {
        if facts.active_column_fullscreen() {
            return None;
        }
        self.restore(spans, facts, return_to)
    }

    fn centred_offset(spans: &Spans<C>, facts: &Facts<C, W>) -> Option<f64> {
        let column = spans.column(facts.active_id()?)?;
        Some(centered_offset(spans.view, column))
    }

    /// The camera aimed back at a remembered view, if the strip still holds its column; that
    /// column, which the host has the columns core make active.
    fn restore(
        &mut self,
        spans: Option<&Spans<C>>,
        facts: &Facts<C, W>,
        bookmark: Bookmark<C>,
    ) -> Option<C> {
        if !facts.holds_column(bookmark.column) {
            return None;
        }
        if let Some(spans) = spans
            && let Some(column) = spans.column(bookmark.column)
        {
            self.viewport.set_target(fit_offset(
                column.world_x + bookmark.relative_offset,
                spans.view,
                column,
            ));
        }
        Some(bookmark.column)
    }

    fn reveal(&mut self, spans: &Spans<C>, facts: &Facts<C, W>, settings: CameraSettings) {
        let Some(column) = facts.active_id().and_then(|id| spans.column(id)) else {
            return;
        };
        let offset = match settings.navigation {
            Navigation::Anchored => spans.anchor_offset(
                spans.columns.iter().position(|c| c.id == column.id).unwrap(),
                settings.alignment,
            ),
            Navigation::Niri => fit_offset(self.viewport.target(), spans.view, column),
        };
        // A reveal that leaves the view where it is keeps it centred.
        if self.viewport.target() != offset {
            self.viewport.set_target(offset);
        }
    }

    fn gesture_began(&mut self, now: Time, laid_out: bool, facts: &Facts<C, W>) -> bool {
        let available = laid_out && facts.selected.is_some_and(|w| !facts.fullscreen.contains(&w));
        if !available {
            return false;
        }
        self.transient_restore = None;
        if matches!(self.viewport, Viewport::Gesture(_)) {
            return false;
        }
        self.motion = MotionHistory::default();
        self.viewport.sample(now, 1.0);
        self.viewport = Viewport::Gesture(self.viewport.offset());
        true
    }

    /// The camera settled on the resting point nearest `projected`; the column it chose. The
    /// decision `GestureEnded` and `Snapped` make, public for the host's tests of it.
    pub fn settle(
        &mut self,
        spans: &Spans<C>,
        facts: &Facts<C, W>,
        settings: CameraSettings,
        projected: f64,
        velocity: f64,
        gesture_release: bool,
    ) -> Option<Release<C>> {
        let from_offset = self.viewport.offset();
        let niri = settings.navigation == Navigation::Niri;
        let active = facts.active_id();
        // After a gesture, a tie goes to a point showing the active column.
        let away = |snap: &Snap<C>| gesture_release && Some(snap.column) != active;
        let snap = spans.snaps.iter().min_by(|a, b| {
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
            // The furthest fully visible column in the direction of travel.
            let past = |c: &&Span<C>| c.id != snap.column;
            column = if projected >= from_offset {
                spans
                    .columns
                    .iter()
                    .skip_while(past)
                    .skip(1)
                    .take_while(|next| next.world_x + next.width <= snap.offset + spans.view)
                    .last()
                    .map_or(column, |next| next.id)
            } else {
                spans
                    .columns
                    .iter()
                    .rev()
                    .skip_while(past)
                    .skip(1)
                    .take_while(|next| next.world_x >= snap.offset)
                    .last()
                    .map_or(column, |next| next.id)
            };
        }
        if !facts.holds_column(column) {
            return None;
        }
        // A snap to another column owes no return (gate 4).
        if Some(column) != active {
            self.showing = Showing::Strip;
        }
        self.transient_restore = None;
        self.viewport.set_target(snap.offset);
        Some(Release {
            column,
            offset: snap.offset,
            from_offset,
            velocity,
        })
    }
}

/// What the scrolling layouts' cameras learn: a layout made, cloned or removed, or an event of
/// one layout's camera.
#[derive(Clone, Debug, PartialEq)]
pub enum CamerasEvent<L, C, W> {
    Created(L),
    Cloned {
        from: L,
        to: L,
        spans: Option<Spans<C>>,
    },
    Removed(L),
    Camera(L, CameraEvent<C, W>),
}

/// Every scrolling layout's camera.
#[derive(Clone, Debug, PartialEq)]
pub struct Cameras<L, C, W> {
    cameras: BTreeMap<L, Camera<C, W>>,
}

impl<L, C, W> Default for Cameras<L, C, W> {
    fn default() -> Self { Cameras { cameras: BTreeMap::new() } }
}

impl<L: Ord + Copy, C: Copy + PartialEq, W: Copy + Ord> Cameras<L, C, W> {
    pub fn from_parts(cameras: impl IntoIterator<Item = (L, Camera<C, W>)>) -> Self {
        Cameras {
            cameras: cameras.into_iter().collect(),
        }
    }

    /// One event; what the camera answered (`Nothing` for a layout it does not hold).
    pub fn step(&mut self, event: CamerasEvent<L, C, W>) -> Answer<C> {
        match event {
            CamerasEvent::Created(layout) => {
                self.cameras.insert(layout, Camera::default());
            }
            CamerasEvent::Cloned { from, to, spans } => {
                let clone =
                    self.cameras.get(&from).map(|c| c.cloned(spans.as_ref())).unwrap_or_default();
                self.cameras.insert(to, clone);
            }
            CamerasEvent::Removed(layout) => {
                self.cameras.remove(&layout);
            }
            CamerasEvent::Camera(layout, event) => {
                if let Some(camera) = self.cameras.get_mut(&layout) {
                    return camera.step(event);
                }
            }
        }
        Answer::Nothing
    }

    pub fn get(&self, layout: L) -> Option<&Camera<C, W>> { self.cameras.get(&layout) }

    /// `layout`'s camera replaced by `camera` (a twin's reseed, a test's setup).
    pub fn seed(&mut self, layout: L, camera: Camera<C, W>) { self.cameras.insert(layout, camera); }

    /// Every layout's camera, in layout order.
    pub fn cameras(&self) -> impl Iterator<Item = (L, &Camera<C, W>)> + '_ {
        self.cameras.iter().map(|(layout, camera)| (*layout, camera))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Cam = Camera<u64, u32>;

    const SETTINGS: CameraSettings = CameraSettings {
        alignment: Alignment::Center,
        navigation: Navigation::Niri,
    };

    /// `n` columns 400 wide with 10 gaps, a 1000 view, snaps at each column's edges.
    fn spans(n: u64) -> Spans<u64> {
        let columns: Vec<_> = (0..n)
            .map(|i| Span {
                id: i + 1,
                world_x: i as f64 * 410.0,
                width: 400.0,
            })
            .collect();
        let right = columns.last().map_or(0.0, |c| c.world_x + c.width - 1000.0).max(0.0);
        let snaps = columns
            .iter()
            .flat_map(|c| {
                [c.world_x, c.world_x + c.width - 1000.0]
                    .into_iter()
                    .filter(move |o| (0.0..=right).contains(o))
                    .map(move |offset| Snap { offset, column: c.id })
            })
            .collect();
        Spans {
            columns,
            bounds: (0.0, right),
            view: 1000.0,
            snaps,
        }
    }

    fn facts(n: u64, active: usize) -> Facts<u64, u32> {
        Facts {
            columns: (0..n)
                .map(|i| FactColumn {
                    id: i + 1,
                    windows: vec![i as u32 + 1],
                })
                .collect(),
            active_column: active,
            selected: (n > 0).then_some(active as u32 + 1),
            fullscreen: BTreeSet::new(),
        }
    }

    fn prepared(n: u64, active: usize) -> Cam {
        let mut cam = Cam::default();
        cam.step(CameraEvent::Prepared {
            before: None,
            spans: spans(n),
            facts: facts(n, active),
            settings: SETTINGS,
            now: Time(0),
        });
        cam
    }

    #[test]
    fn a_first_view_jumps_unanimated_to_show_the_selection() {
        let cam = prepared(5, 3);
        assert!(matches!(
            cam.viewport(),
            Viewport::Resting(Rest::At, Motion::Static(_))
        ));
        // Column 4 (x 1230..1630) fits: the camera's right edge at its right.
        assert_eq!(cam.viewport().offset(), 630.0);
    }

    #[test]
    fn a_presented_position_never_overwrites_a_target_the_layout_set() {
        let mut cam = prepared(5, 0);
        cam.step(CameraEvent::Revealed {
            spans: Some(spans(5)),
            facts: facts(5, 4),
            settings: SETTINGS,
        });
        let target = cam.viewport().target();
        let elsewhere = Spring::new(0.0, target + 50.0, 0.0, Time(0));
        cam.step(CameraEvent::Presented {
            presented: Presented::Moving(elsewhere),
            motion: MotionHistory::default(),
            ended: Ended::No,
        });
        assert_eq!(cam.viewport().target(), target);
        let on_way = Spring::new(0.0, target, 0.0, Time(0));
        cam.step(CameraEvent::Presented {
            presented: Presented::Moving(on_way),
            motion: MotionHistory::default(),
            ended: Ended::No,
        });
        assert!(matches!(
            cam.viewport(),
            Viewport::Resting(_, Motion::Animation(_))
        ));
    }

    #[test]
    fn a_spring_keeps_the_times_it_was_given_and_never_samples_back() {
        let mut spring = Spring::new(0.0, 100.0, 0.0, Time(1_000));
        assert!(!spring.sample(Time(1_000 + 10_000_000), 1.0));
        let at = spring.current;
        // A stale moment does not move it back.
        spring.sample(Time(1_000), 1.0);
        assert_eq!((spring.current, spring.sampled), (at, Time(1_000 + 10_000_000)));
        assert!(spring.sample(Time(2_000_000_000), 1.0));
    }

    #[test]
    fn a_gesture_moves_within_bounds_keeping_the_excess_as_overscroll_and_releases_on_a_snap() {
        let mut cam = prepared(5, 0);
        let began = cam.step(CameraEvent::GestureBegan {
            now: Time(0),
            spans: Some(spans(5)),
            facts: facts(5, 0),
        });
        assert_eq!(began, Answer::Began(true));
        assert_eq!(
            cam.step(CameraEvent::GestureMoved {
                delta: -30.0,
                at: Time(1),
                spans: Some(spans(5))
            }),
            Answer::Moved(0.0)
        );
        assert_eq!(cam.motion().overscroll(), -30.0);
        for (i, at) in [(1, 10_000_000u64), (2, 20_000_000)] {
            let moved = cam.step(CameraEvent::GestureMoved {
                delta: 200.0,
                at: Time(at),
                spans: Some(spans(5)),
            });
            assert!(matches!(moved, Answer::Moved(_)), "{i}");
        }
        let released = cam.step(CameraEvent::GestureEnded {
            at: Time(30_000_000),
            animate: true,
            started: Time(30_000_000),
            spans: Some(spans(5)),
            facts: facts(5, 0),
            settings: SETTINGS,
        });
        let Answer::Released(release) = released else {
            panic!("{released:?}")
        };
        assert!(spans(5).snaps.iter().any(|s| s.offset == release.offset));
        assert!(matches!(
            cam.viewport(),
            Viewport::Resting(_, Motion::Animation(_))
        ));
    }

    #[test]
    fn a_fullscreen_return_is_owed_once_and_consumed_when_the_core_leaves_fullscreen() {
        let mut cam = prepared(5, 2);
        let mut full = facts(5, 2);
        full.fullscreen.insert(3);
        cam.step(CameraEvent::FullscreenEntered {
            window: 3,
            spans: Some(spans(5)),
            before: facts(5, 2),
            after: full.clone(),
        });
        let Showing::Fullscreen { window: 3, return_to } = *cam.showing() else {
            panic!()
        };
        assert_eq!(return_to.column, 3);
        // A second fullscreen keeps the first return.
        cam.step(CameraEvent::FullscreenEntered {
            window: 4,
            spans: Some(spans(5)),
            before: full,
            after: facts(5, 3),
        });
        assert!(matches!(cam.showing(), Showing::Fullscreen { window: 3, .. }));
        // A settle with the core no longer showing 3 fullscreen consumes it.
        cam.step(CameraEvent::Rebuilt {
            spans: Some(spans(5)),
            facts: facts(5, 2),
            hold: None,
            now: Time(0),
        });
        assert_eq!(*cam.showing(), Showing::Strip);
    }

    #[test]
    fn a_centred_view_recentres_on_the_selected_column_at_each_settle() {
        let mut cam = prepared(5, 1);
        cam.step(CameraEvent::Centred {
            spans: Some(spans(5)),
            facts: facts(5, 1),
        });
        assert_eq!(cam.viewport().target(), 410.0 - 300.0);
        cam.step(CameraEvent::Rebuilt {
            spans: Some(spans(5)),
            facts: facts(5, 2),
            hold: None,
            now: Time(0),
        });
        assert_eq!(cam.viewport().target(), 820.0 - 300.0);
        // Selecting another column ends it.
        cam.step(CameraEvent::Selected {
            column_changed: true,
            spans: Some(spans(5)),
            facts: facts(5, 3),
            settings: SETTINGS,
        });
        assert!(matches!(cam.viewport(), Viewport::Resting(Rest::At, _)));
    }

    #[test]
    fn a_scroll_past_an_end_answers_its_excess_in_columns() {
        let mut cam = prepared(5, 0);
        let answer = cam.step(CameraEvent::ScrollBy {
            delta: -1.0,
            spans: Some(spans(5)),
            facts: facts(5, 0),
            gap: 10.0,
        });
        assert_eq!(answer, Answer::Excess(Edge::Left, 1.0));
        assert_eq!(cam.viewport().target(), 0.0);
    }

    /// A spring retargeted at a moment before its last sample restarts from that sample, never
    /// from the stale moment.
    #[test]
    fn a_retarget_at_a_stale_moment_restarts_from_the_last_sample() {
        let mut motion = Motion::Animation(Spring::new(0.0, 100.0, 0.0, Time(0)));
        motion.sample(Time(50_000_000), 1.0);
        motion.move_target(200.0, Time(10_000_000));
        let Motion::Animation(spring) = motion else {
            panic!("{motion:?}")
        };
        assert_eq!((spring.started, spring.target), (Time(50_000_000), 200.0));
    }

    /// An edit that moves the selected window to another column holds that column still on
    /// screen, where the old column was.
    #[test]
    fn an_edit_holds_the_selected_windows_new_column_still() {
        let mut cam = prepared(5, 1);
        let before = cam.viewport().offset();
        // Window 2 now sits in column 4 (x 1230); its old column 2 was at 410.
        let mut after = facts(5, 3);
        after.columns[1].windows.clear();
        after.columns[3].windows.push(2);
        cam.step(CameraEvent::Rebuilt {
            spans: Some(spans(5)),
            facts: after,
            hold: Some(HeldAt {
                window: Some(2),
                column: 2,
                x: 410.0,
            }),
            now: Time(0),
        });
        let rebased = before + (1230.0 - 410.0);
        assert_eq!(cam.viewport().offset(), spans(5).clamp(rebased));
        assert_ne!(cam.viewport().offset(), before);
    }

    /// A snap to another column than the return's owes no return; a snap to its column keeps it.
    #[test]
    fn a_snap_to_another_column_owes_no_return() {
        let mut full = facts(5, 2);
        full.fullscreen.insert(3);
        for (snap_to, owed) in [(2, true), (0, false)] {
            let mut cam = prepared(5, 2);
            cam.step(CameraEvent::FullscreenEntered {
                window: 3,
                spans: Some(spans(5)),
                before: facts(5, 2),
                after: full.clone(),
            });
            let at = spans(5).snaps.iter().find(|s| s.column == snap_to + 1).unwrap().offset;
            cam.viewport = Viewport::at(at);
            // Anchored: the nearest resting point decides, with no travel rule on top.
            let anchored = CameraSettings {
                navigation: Navigation::Anchored,
                ..SETTINGS
            };
            cam.step(CameraEvent::Snapped {
                spans: Some(spans(5)),
                facts: facts(5, 2),
                settings: anchored,
            });
            assert_eq!(
                matches!(cam.showing(), Showing::Fullscreen { .. }),
                owed,
                "snap to {snap_to}"
            );
        }
    }

    /// The portable `exp` is within an ulp or two of the library's over the spring's range.
    #[test]
    fn the_portable_exp_matches_the_librarys_closely() {
        for i in 0..20_000 {
            let x = -(i as f64) * 0.01;
            let (ours, std) = (exp(x), x.exp());
            assert!((ours - std).abs() <= std * 4e-16, "x {x}: {ours} vs {std}");
        }
        assert_eq!((exp(0.0), exp(-800.0)), (1.0, 0.0));
        assert_eq!(800.0_f64.sqrt(), OMEGA);
        assert_eq!(0.997_f64.ln(), DECELERATION_LN);
    }

    #[test]
    fn motion_history_refuses_a_stale_moment_and_ages_out_old_samples() {
        let mut motion = MotionHistory::default();
        assert!(motion.push(10.0, Time(200_000_000)));
        assert!(!motion.push(10.0, Time(100_000_000)));
        assert!(motion.push(10.0, Time(400_000_000)));
        assert_eq!(motion.samples().count(), 1);
    }
}
