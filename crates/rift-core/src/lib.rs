//! Rift's pure core. No I/O, no clocks, no threads: every fact arrives as an [`Event`], every
//! decision leaves as an [`Effect`], and [`Core::step`] is the only way in.
//!
//! The platform (event taps, AX, SkyLight, timers, the app actors) owns the event loop and
//! the effects; the core owns the decisions. The core is generic over the platform's types
//! ([`Kinds`]) so rift can run it on its own `WindowId`/`CGRect`/`Instant` with no conversion,
//! and a foreign core (Roc) can run it on [`plain`] data.

pub mod core;
pub mod plain;
pub mod pointer;
pub mod reconciler;

use std::fmt::Debug;
use std::hash::Hash;
use std::time::Duration;

pub use crate::core::{Core, Effect, Event, RustCore};
pub use crate::pointer::{Holder, Initiative, InputClass, Owner, Pointer, may};
pub use crate::reconciler::{
    Answer, Note, Observation, Reconciler, Sync, Translate, Verdict, Want, Write,
};

/// The platform's types, and the little geometry and time the core needs of them.
pub trait Kinds: Copy + Debug + PartialEq + 'static {
    type Window: Copy + Eq + Hash + Debug;
    type Frame: Copy + PartialEq + Debug;
    type Time: Copy + Ord + Debug;

    /// Whether `a` is within `tolerance` of `b` on every edge, origin and size alike.
    fn is_within(a: Self::Frame, tolerance: f64, b: Self::Frame) -> bool;
    /// `duration` after `time`.
    fn after(time: Self::Time, duration: Duration) -> Self::Time;

    /// The same frame, give or take a tenth of a point.
    fn same_as(a: Self::Frame, b: Self::Frame) -> bool { Self::is_within(a, 0.1, b) }
}
