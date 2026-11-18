//! Plain types for the core's unit tests.

use std::ops::Add;
use std::time::Duration;

use crate::Kinds;

/// A window: its app's pid and its index within the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId {
    pub pid: i32,
    pub idx: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The id a write goes out under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(pub u32);

/// Monotonic nanoseconds since the platform started its clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Nanos(pub u64);

impl Add<Duration> for Nanos {
    type Output = Nanos;

    fn add(self, by: Duration) -> Nanos { Nanos(self.0 + by.as_nanos() as u64) }
}

fn within(a: f64, tolerance: f64, b: f64) -> bool { (a - b).abs() < tolerance }

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plain;

impl Kinds for Plain {
    type Frame = Rect;
    type Request = RequestId;
    type Time = Nanos;
    type Window = WindowId;

    fn is_within(a: Rect, tolerance: f64, b: Rect) -> bool {
        within(a.x, tolerance, b.x)
            && within(a.y, tolerance, b.y)
            && within(a.w, tolerance, b.w)
            && within(a.h, tolerance, b.h)
    }

    fn after(time: Nanos, duration: Duration) -> Nanos { time + duration }

    fn same_size(a: Rect, b: Rect) -> bool { within(a.w, 0.1, b.w) && within(a.h, 0.1, b.h) }
}
