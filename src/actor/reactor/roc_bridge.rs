//! The reactor on the Roc core (feature `roc-core`).
//!
//! Every decision the reactor applies comes from the Roc core: each event is translated to
//! plain data, stepped through [`RocCore`], and its effects translated back. A Rust core is
//! stepped beside it on the same event and must decide the same effects, so every reactor test
//! run with the feature is also a differential test. The shadow also answers the Rust-only
//! reads the Roc core has no door for (`sync` in tests, `due` for the timer's coeffect).

use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use rift_core::plain::{Nanos, Plain, Rect, RequestId};
use rift_core::{Core, Effect, Event, Reconciler, RustCore, Translate};
use rift_core_roc::RocCore;

use super::TransactionId;
use super::reconciler::Rift;
use crate::actor::app::WindowId;

/// Times are counted from `epoch`, the bridge's creation; the clock only moves forward.
struct Plainly {
    epoch: Instant,
}

impl Translate<Rift, Plain> for Plainly {
    fn window(&self, window: WindowId) -> rift_core::plain::WindowId {
        rift_core::plain::WindowId {
            pid: window.pid,
            idx: window.idx.get(),
        }
    }

    fn frame(&self, frame: CGRect) -> Rect {
        Rect {
            x: frame.origin.x,
            y: frame.origin.y,
            w: frame.size.width,
            h: frame.size.height,
        }
    }

    fn time(&self, time: Instant) -> Nanos { Nanos((time - self.epoch).as_nanos() as u64) }

    fn request(&self, request: TransactionId) -> RequestId { RequestId(request.number()) }
}

impl Translate<Plain, Rift> for Plainly {
    fn window(&self, window: rift_core::plain::WindowId) -> WindowId {
        WindowId {
            pid: window.pid,
            idx: NonZeroU32::new(window.idx).expect("the Roc core only names windows it was given"),
        }
    }

    fn frame(&self, rect: Rect) -> CGRect {
        CGRect::new(CGPoint::new(rect.x, rect.y), CGSize::new(rect.w, rect.h))
    }

    fn time(&self, time: Nanos) -> Instant { self.epoch + Duration::from_nanos(time.0) }

    fn request(&self, request: RequestId) -> TransactionId { TransactionId::from_number(request.0) }
}

pub(crate) struct RocInRift {
    roc: RocCore,
    shadow: RustCore<Rift>,
    plainly: Plainly,
}

impl std::fmt::Debug for RocInRift {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RocInRift")
            .field("shadow", &self.shadow)
            .finish_non_exhaustive()
    }
}

impl RocInRift {
    /// `epoch` must be no later than any time the bridge is handed.
    pub(crate) fn new(epoch: Instant) -> Self {
        RocInRift {
            roc: RocCore::default(),
            shadow: RustCore::default(),
            plainly: Plainly { epoch },
        }
    }

    /// The shadow's reconciler, for the Rust-only reads.
    pub(crate) fn reconciler(&self) -> &Reconciler<Rift> { self.shadow.reconciler() }
}

impl Core<Rift> for RocInRift {
    fn step(&mut self, event: Event<Rift>) -> Vec<Effect<Rift>> {
        let decided: Vec<Effect<Rift>> = self
            .roc
            .step(event.clone().translate(&self.plainly))
            .into_iter()
            .map(|effect| effect.translate(&self.plainly))
            .collect();
        let shadowed = self.shadow.step(event.clone());
        assert_eq!(
            decided, shadowed,
            "the Roc core and the Rust core disagree on {event:?}"
        );
        decided
    }
}
