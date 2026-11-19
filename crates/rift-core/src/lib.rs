//! Rift's pure core. No I/O, no clocks, no threads: every fact arrives as an [`Event`], every
//! decision leaves as an [`Effect`], and [`RustCore::step`] is the only way in.
//!
//! The platform (event taps, AX, SkyLight, timers, the app actors) owns the event loop and
//! the effects; the core owns the decisions. The core is generic over the platform's types
//! ([`Kinds`]) so rift can run it on its own `WindowId`/`CGRect`/`Instant` with no conversion,
//! and a foreign core (Roc) runs it on [`plain`] data, which crosses as words ([`plain::wire`]).

pub mod core;
pub mod focus;
pub mod layout;
pub mod lifecycle;
pub mod plain;
pub mod pointer;
pub mod reconciler;
pub mod topology;
pub mod workspaces;

use std::fmt::Debug;
use std::hash::Hash;
use std::time::Duration;

pub use crate::core::{Desire, Effect, Event, Plan, RustCore};
pub use crate::pointer::{
    Button, Holder, Initiative, InputClass, Owner, Pointer, Release, Ungrip, may,
};
pub use crate::reconciler::{
    Answer, Note, Reconciler, Reply, Sent, Stamp, Sync, Verdict, Want, Write,
};

/// The platform's types, and the little geometry and time the core needs of them.
pub trait Kinds: Copy + Debug + PartialEq + 'static {
    /// Ordered so that what the core decides about several windows at once comes out in the
    /// same order whatever the platform's types.
    type Window: Copy + Ord + Hash + Debug;
    type Frame: Copy + PartialEq + Debug;
    type Time: Copy + Ord + Debug;
    /// The id a write goes out under, which the app echoes in its answer. The platform mints
    /// it from one counter shared with writers outside the core, so ids are ordered as issued:
    /// a read stamped with an older one was made before the newer request was applied.
    type Request: Copy + Ord + Debug;

    /// Whether `a` is within `tolerance` of `b` on every edge, origin and size alike.
    fn is_within(a: Self::Frame, tolerance: f64, b: Self::Frame) -> bool;
    /// `duration` after `time`.
    fn after(time: Self::Time, duration: Duration) -> Self::Time;

    /// Whether `a` and `b` have the same size, give or take a tenth of a point.
    fn same_size(a: Self::Frame, b: Self::Frame) -> bool;

    /// The same frame, give or take a tenth of a point.
    fn same_as(a: Self::Frame, b: Self::Frame) -> bool { Self::is_within(a, 0.1, b) }
}
