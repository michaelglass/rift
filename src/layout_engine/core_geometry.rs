//! The conversions between rift's geometry and the core's plain geometry, the decision that
//! gives a layout the core's frames (policy (a): the core decides, the layout system's frames are
//! its shadow), and the checks that hold the core's frames to the systems' not yet flipped.

use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use rift_core::layout::Gaps;
use rift_core::layout::traditional::{StackLine, Traditional};
use rift_core::plain::Rect;

use crate::actor::app::WindowId;

pub(crate) fn core_rect(rect: CGRect) -> Rect {
    Rect {
        x: rect.origin.x,
        y: rect.origin.y,
        w: rect.size.width,
        h: rect.size.height,
    }
}

pub(crate) fn cg_rect(rect: Rect) -> CGRect {
    CGRect::new(CGPoint::new(rect.x, rect.y), CGSize::new(rect.w, rect.h))
}

/// The gaps one screen uses (any per-display override already applied by the caller).
pub(crate) fn core_gaps(gaps: &crate::common::config::GapSettings) -> Gaps {
    Gaps {
        top: gaps.outer.top,
        left: gaps.outer.left,
        bottom: gaps.outer.bottom,
        right: gaps.outer.right,
        horizontal: gaps.inner.horizontal,
        vertical: gaps.inner.vertical,
    }
}

/// What an app allows of a window's size, as the core takes it (the minimum it reports goes
/// into the `Sizing`, parsed as the core parses a reported size).
pub(crate) fn core_constraints(
    c: crate::layout_engine::systems::WindowLayoutConstraints,
) -> rift_core::layout::WindowConstraints {
    use crate::layout_engine::systems::Sizing;
    let min = rift_core::layout::Size::reported(c.min_width, c.min_height);
    let (is_resizable, respect_min_size) = match c.sizing {
        Sizing::Fixed => (false, false),
        Sizing::Resizable => (true, false),
        Sizing::ResizableAtLeastMin => (true, true),
    };
    rift_core::layout::WindowConstraints {
        sizing: rift_core::layout::Sizing::new(is_resizable, respect_min_size, min),
        locked_width: c.locked_width,
        locked_height: c.locked_height,
        max_width: c.max_width,
        max_height: c.max_height,
    }
}

/// The stack bar's reservation, as the core takes it.
pub(crate) fn core_stack_line(line: crate::common::config::StackLineReserve) -> StackLine {
    use crate::common::config::{HorizontalPlacement, VerticalPlacement};
    StackLine {
        thickness: line.thickness,
        spacing: line.spacing,
        top: line.horiz == HorizontalPlacement::Top,
        left: line.vert == VerticalPlacement::Left,
    }
}

/// How many times a shadow's frames differed from the deciding core's, process-wide: each one a
/// `layout-divergence:` (a layout system's) or `roc-divergence:` (under `roc-core`, the Rust
/// core's, or the Roc core failing to answer) warned.
static LAYOUT_DIVERGENCES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many times a layout's shadow has differed from the deciding core (see
/// [`LAYOUT_DIVERGENCES`]).
pub(crate) fn layout_divergences() -> u64 {
    LAYOUT_DIVERGENCES.load(std::sync::atomic::Ordering::Relaxed)
}

/// How many times a layout system's shadow ran beside the deciding core, process-wide.
static LAYOUT_SHADOWS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many times a layout system's shadow has run (see [`LAYOUT_SHADOWS`]).
pub(crate) fn layout_shadows() -> u64 { LAYOUT_SHADOWS.load(std::sync::atomic::Ordering::Relaxed) }

/// Whether a layout pass runs the shadow beside the deciding core (the core decides either way).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shadow {
    Run,
    Skip,
}

/// How often a Space's layout passes run the shadow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sampling {
    EveryPass,
    /// Every this many passes (the first pass among them).
    Every(std::num::NonZeroU64),
}

/// Tests and `roc-core` builds run the shadow on every pass; a plain release build on every 32nd
/// pass per Space (the lead's ruling, 2026-11-24, after the twins' `VIEW_EVERY`): the shadow
/// costs a large share of a pass (see roc-fork.md, "Measured"), and a divergence is still
/// counted within 32 passes of a Space.
const SAMPLING: Sampling = if cfg!(any(test, feature = "roc-core")) {
    Sampling::EveryPass
} else {
    Sampling::Every(std::num::NonZeroU64::new(32).unwrap())
};

impl Sampling {
    /// Whether the pass numbered `pass` (from 0) runs the shadow.
    fn shadow(self, pass: u64) -> Shadow {
        match self {
            Self::EveryPass => Shadow::Run,
            Self::Every(every) if pass % every.get() == 0 => Shadow::Run,
            Self::Every(_) => Shadow::Skip,
        }
    }

    /// Whether layout work outside a Space's pass (a query, a drag preview, an edit's rebuild of a
    /// scrolling strip) runs the shadow: only where every pass does.
    fn outside_a_pass(self) -> Shadow {
        match self {
            Self::EveryPass => Shadow::Run,
            Self::Every(_) => Shadow::Skip,
        }
    }
}

impl Shadow {
    /// For layout work outside a Space's pass (see [`Sampling::outside_a_pass`]).
    pub(crate) fn outside_a_pass() -> Self { SAMPLING.outside_a_pass() }
}

/// Each Space's layout passes, counted, deciding which run the shadow.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct ShadowPasses(std::collections::HashMap<crate::sys::screen::SpaceId, u64>);

impl ShadowPasses {
    /// Counts a layout pass of `space`, answering whether it runs the shadow.
    pub(crate) fn pass(&mut self, space: crate::sys::screen::SpaceId) -> Shadow {
        let passes = self.0.entry(space).or_default();
        let shadow = SAMPLING.shadow(*passes);
        *passes += 1;
        shadow
    }
}

/// The frames rift gives a traditional layout (stack, master-stack, traditional): the core's for
/// `shape` (none: nothing takes space), the Roc core's under `roc-core`, else the Rust core's. The
/// layout system's frames (`upstream`) are the shadow: a difference fails tests, and a running
/// Rift warns it, counts it and uses the core's frames.
pub(crate) fn decide_traditional(
    what: &str,
    shape: Option<Traditional<WindowId>>,
    shadow: Shadow,
    upstream: impl FnOnce() -> Vec<(WindowId, CGRect)>,
) -> Vec<(WindowId, CGRect)> {
    let rust = shape.as_ref().map_or_else(Vec::new, Traditional::frames);
    #[cfg(feature = "roc-core")]
    let roc = || match &shape {
        None => Ok(Vec::new()),
        Some(shape) => rift_core_roc::tile_traditional(shape.clone().map(&to_plain)),
    };
    #[cfg(not(feature = "roc-core"))]
    let roc = no_roc;
    decide(what, &shape, shadow, upstream, rust, roc)
}

/// A shadow (frames, or a placement) that differed from the deciding core, or the Roc core
/// failing to answer.
#[derive(Debug, PartialEq)]
enum Divergence<T, E> {
    /// The layout system's.
    System(T),
    /// Under `roc-core`, the Rust core's.
    #[cfg_attr(not(feature = "roc-core"), allow(dead_code))]
    RustCore(T),
    /// Under `roc-core`, the Roc core could not answer: the Rust core decides.
    #[cfg_attr(not(feature = "roc-core"), allow(dead_code))]
    RocFailed(E),
}

/// The deciding core's answer and every shadow that differs from it (compared by `key`): the
/// Roc core's (`roc`, `Some` only under `roc-core`) when it answers, else the Rust core's
/// (`rust`); the Rust core's is a shadow when the Roc core decides, the system's (`upstream`)
/// when the pass runs it.
fn decided<T, E, K: PartialEq>(
    upstream: Shadowed<T>,
    rust: T,
    roc: Option<Result<T, E>>,
    key: impl Fn(&T) -> K,
) -> (T, Vec<Divergence<T, E>>) {
    let mut divergences = Vec::new();
    let core = match roc {
        None => rust,
        Some(Ok(roc)) => {
            if key(&rust) != key(&roc) {
                divergences.push(Divergence::RustCore(rust));
            }
            roc
        }
        Some(Err(error)) => {
            divergences.push(Divergence::RocFailed(error));
            rust
        }
    };
    match upstream {
        Shadowed::Ran(upstream) if key(&upstream) != key(&core) => {
            divergences.push(Divergence::System(upstream))
        }
        Shadowed::Ran(_) | Shadowed::Skipped => {}
    }
    (core, divergences)
}

/// The layout system's own answer, if the pass ran it.
#[derive(Debug)]
enum Shadowed<T> {
    Ran(T),
    Skipped,
}

impl<T> Shadowed<T> {
    /// The system's answer (`upstream`), asked only when the pass runs the shadow.
    fn of(shadow: Shadow, upstream: impl FnOnce() -> T) -> Self {
        match shadow {
            Shadow::Run => {
                LAYOUT_SHADOWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Self::Ran(upstream())
            }
            Shadow::Skip => Self::Skipped,
        }
    }
}

/// The core's frames (`rust`, and under `roc-core` what `roc` answers) decided over rift's
/// system's (`upstream`), every divergence reported.
fn decide<E: std::fmt::Debug>(
    what: &str,
    shape: &impl std::fmt::Debug,
    shadow: Shadow,
    upstream: impl FnOnce() -> Vec<(WindowId, CGRect)>,
    rust: Vec<(WindowId, Rect)>,
    #[cfg_attr(not(feature = "roc-core"), allow(unused_variables))] roc: impl FnOnce() -> Result<
        Vec<(rift_core::plain::WindowId, Rect)>,
        E,
    >,
) -> Vec<(WindowId, CGRect)> {
    let upstream = Shadowed::of(shadow, || {
        upstream()
            .into_iter()
            .map(|(window, frame)| (window, core_rect(frame)))
            .collect()
    });
    #[cfg(feature = "roc-core")]
    let roc = Some(roc().map(|frames| {
        frames.into_iter().map(|(window, frame)| (of_plain(window), frame)).collect()
    }));
    #[cfg(not(feature = "roc-core"))]
    let roc: Option<Result<Vec<(WindowId, Rect)>, E>> = None;
    let (core, divergences) = decided(upstream, rust, roc, |frames| bits(frames));
    report_all(what, shape, divergences, &core);
    core.into_iter().map(|(window, frame)| (window, cg_rect(frame))).collect()
}

/// Reports each divergence from the deciding core's answer (`core`).
fn report_all<T: std::fmt::Debug, E: std::fmt::Debug>(
    what: &str,
    shape: &impl std::fmt::Debug,
    divergences: Vec<Divergence<T, E>>,
    core: &T,
) {
    for divergence in divergences {
        match divergence {
            Divergence::System(theirs) => {
                report_divergence("layout-divergence", what, shape, &theirs, core)
            }
            Divergence::RustCore(rust) => {
                report_divergence("roc-divergence", what, shape, &rust, core)
            }
            Divergence::RocFailed(error) => {
                report_divergence("roc-divergence", what, shape, &error, core)
            }
        }
    }
}

/// Counts a divergence, then fails the test, or in a running Rift warns it (the core's frames
/// are used either way).
fn report_divergence(
    label: &str,
    what: &str,
    shape: &impl std::fmt::Debug,
    shadow: &impl std::fmt::Debug,
    core: &impl std::fmt::Debug,
) {
    let divergences = LAYOUT_DIVERGENCES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    #[cfg(test)]
    panic!("{label} ({divergences} so far): {what} {shape:?}: shadow {shadow:?}, core {core:?}");
    #[cfg(not(test))]
    tracing::warn!(
        what,
        divergences,
        ?shape,
        ?shadow,
        ?core,
        "{label}: a shadow's frames differ from the deciding core's; the core's are used"
    );
}

/// The frames rift gives a BSP layout: the core's for `shape` (none: nothing takes space), the
/// Roc core's under `roc-core`, else the Rust core's, with the BSP system's (`upstream`) the
/// shadow, as [`decide_traditional`].
pub(crate) fn decide_bsp(
    shape: Option<rift_core::layout::bsp::Bsp<WindowId>>,
    shadow: Shadow,
    upstream: impl FnOnce() -> Vec<(WindowId, CGRect)>,
) -> Vec<(WindowId, CGRect)> {
    let rust = shape.as_ref().map_or_else(Vec::new, rift_core::layout::bsp::Bsp::frames);
    #[cfg(feature = "roc-core")]
    let roc = || match &shape {
        None => Ok(Vec::new()),
        Some(shape) => rift_core_roc::tile_bsp(shape.clone().map(&to_plain)),
    };
    #[cfg(not(feature = "roc-core"))]
    let roc = no_roc;
    decide("bsp", &shape, shadow, upstream, rust, roc)
}

/// The frames rift gives a BSP layout whose tree the Roc core decides (`roc-core`): the Roc
/// tree's own (`roc`: its shape through the Roc geometry), with the Rust core's frames for the
/// Rust tree's `shape` and the BSP system's (`upstream`) the shadows.
#[cfg(feature = "roc-core")]
pub(crate) fn decide_bsp_tree(
    shape: Option<rift_core::layout::bsp::Bsp<WindowId>>,
    shadow: Shadow,
    upstream: impl FnOnce() -> Vec<(WindowId, CGRect)>,
    roc: impl FnOnce() -> Vec<(rift_core::plain::WindowId, Rect)>,
) -> Vec<(WindowId, CGRect)> {
    let rust = shape.as_ref().map_or_else(Vec::new, rift_core::layout::bsp::Bsp::frames);
    decide("bsp", &shape, shadow, upstream, rust, || {
        Ok::<_, std::convert::Infallible>(roc())
    })
}

/// A scrolling strip placed in world space: each column (id, x, width), each window's frame,
/// the camera's resting points (offset, the column's id) and its bounds. Nothing placed (no
/// window takes part) is no column, frame or resting point, and bounds at 0.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StripPlacement {
    pub columns: Vec<(u64, f64, f64)>,
    pub frames: Vec<(WindowId, Rect)>,
    pub snaps: Vec<(f64, u64)>,
    pub bounds: (f64, f64),
}

impl StripPlacement {
    /// The core's placement (none: nothing placed), as the platform holds it: the columns'
    /// anchors are the core's own and are dropped.
    fn of_placed<W: Copy>(
        placed: Option<rift_core::layout::scrolling::Placed<W, u64>>,
        window: impl Fn(W) -> WindowId,
    ) -> Self {
        match placed {
            None => Self {
                columns: Vec::new(),
                frames: Vec::new(),
                snaps: Vec::new(),
                bounds: (0.0, 0.0),
            },
            Some(placed) => Self {
                columns: placed.spots().map(|spot| (spot.id, spot.world_x, spot.width)).collect(),
                frames: placed.frames.iter().map(|&(w, frame)| (window(w), frame)).collect(),
                snaps: std::iter::once(&placed.snaps.0)
                    .chain(&placed.snaps.1)
                    .map(|snap| (snap.offset, snap.column))
                    .collect(),
                bounds: placed.bounds,
            },
        }
    }

    /// The placement as words: ids as they are, floats as their bits.
    fn words(&self) -> Vec<u64> {
        let mut words = Vec::new();
        for &(id, x, width) in &self.columns {
            words.extend([id, x.to_bits(), width.to_bits()]);
        }
        words.push(u64::MAX);
        for &(w, r) in &self.frames {
            words.push(window_word(w));
            words.extend([r.x, r.y, r.w, r.h].map(f64::to_bits));
        }
        words.push(u64::MAX);
        for &(offset, column) in &self.snaps {
            words.extend([offset.to_bits(), column]);
        }
        words.extend([self.bounds.0.to_bits(), self.bounds.1.to_bits()]);
        words
    }
}

/// Where rift places a scrolling strip (none: no window takes part): the core's placement (the
/// Roc core's under `roc-core`, else the Rust core's), with the scrolling system's own
/// (`upstream`) the shadow, held word for word, as [`decide_traditional`].
pub(crate) fn decide_strip(
    strip: Option<rift_core::layout::scrolling::Strip<WindowId, u64>>,
    shadow: Shadow,
    upstream: impl FnOnce() -> StripPlacement,
) -> StripPlacement {
    let rust = StripPlacement::of_placed(
        strip.as_ref().map(rift_core::layout::scrolling::Strip::place),
        |w| w,
    );
    #[cfg(feature = "roc-core")]
    let roc = Some(match &strip {
        None => Ok(StripPlacement::of_placed::<WindowId>(None, |w| w)),
        Some(strip) => rift_core_roc::place_strip(strip.clone().map(&to_plain, &|id| id))
            .map(|placed| StripPlacement::of_placed(Some(placed), of_plain)),
    });
    #[cfg(not(feature = "roc-core"))]
    let roc: Option<Result<StripPlacement, std::convert::Infallible>> = None;
    let (core, divergences) =
        decided(Shadowed::of(shadow, upstream), rust, roc, StripPlacement::words);
    report_all("scrolling", &strip, divergences, &core);
    core
}

/// The frames rift gives a placed strip shown by a camera: the core's (`translate`; the Roc
/// core's under `roc-core`, else the Rust core's), with the scrolling system's
/// (`translate_frames`'s own, `upstream`) the shadow, as [`decide_traditional`].
pub(crate) fn decide_translation(
    world: &[(WindowId, Rect)],
    camera: rift_core::layout::scrolling::Camera,
    fullscreen: &[(WindowId, rift_core::layout::scrolling::Fullscreen)],
    shadow: Shadow,
    upstream: impl FnOnce() -> Vec<(WindowId, CGRect)>,
) -> Vec<(WindowId, CGRect)> {
    let rust = rift_core::layout::scrolling::translate(world, camera, fullscreen);
    let question = (world, camera, fullscreen);
    #[cfg(feature = "roc-core")]
    let roc = || {
        let world: Vec<_> = world.iter().map(|&(w, r)| (to_plain(w), r)).collect();
        let fullscreen: Vec<_> = fullscreen.iter().map(|&(w, m)| (to_plain(w), m)).collect();
        rift_core_roc::translate(&world, camera, &fullscreen)
    };
    #[cfg(not(feature = "roc-core"))]
    let roc = no_roc;
    decide("scrolling translation", &question, shadow, upstream, rust, roc)
}

/// The frames rift gives a floating layout: the core's (the Roc core's under `roc-core`, else the
/// Rust core's), with the floating system's (`upstream`) the shadow, as [`decide_traditional`].
pub(crate) fn decide_floating(
    layout: rift_core::layout::floating::Layout<WindowId>,
    shadow: Shadow,
    upstream: impl FnOnce() -> Vec<(WindowId, CGRect)>,
) -> Vec<(WindowId, CGRect)> {
    let rust = layout.frames();
    #[cfg(feature = "roc-core")]
    let roc = || rift_core_roc::floating_frames(layout.clone().map(&to_plain));
    #[cfg(not(feature = "roc-core"))]
    let roc = no_roc;
    decide("floating", &layout, shadow, upstream, rust, roc)
}

/// Without `roc-core` there is no Roc core to ask.
#[cfg(not(feature = "roc-core"))]
fn no_roc() -> Result<Vec<(rift_core::plain::WindowId, Rect)>, std::convert::Infallible> {
    Ok(Vec::new())
}

/// A window as one word (its pid's bits above its index).
fn window_word(window: WindowId) -> u64 {
    (u64::from(window.pid as u32) << 32) | u64::from(window.idx.get())
}

fn bits(frames: &[(WindowId, Rect)]) -> Vec<(WindowId, [u64; 4])> {
    frames
        .iter()
        .map(|&(window, r)| (window, [r.x, r.y, r.w, r.h].map(f64::to_bits)))
        .collect()
}

#[cfg(feature = "roc-core")]
fn to_plain(window: WindowId) -> rift_core::plain::WindowId {
    rift_core::plain::WindowId {
        pid: window.pid,
        idx: window.idx.get(),
    }
}

#[cfg(feature = "roc-core")]
fn of_plain(window: rift_core::plain::WindowId) -> WindowId {
    let idx = std::num::NonZeroU32::new(window.idx).expect("the core names only real windows");
    WindowId { pid: window.pid, idx }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A placement as the decision compares it.
    pub(crate) fn placement_words_for_test(placement: &StripPlacement) -> Vec<u64> {
        placement.words()
    }

    /// The Rust core's placement of `strip`, as the decision holds it.
    pub(crate) fn rust_placement_for_test(
        strip: Option<rift_core::layout::scrolling::Strip<WindowId, u64>>,
    ) -> StripPlacement {
        StripPlacement::of_placed(
            strip.as_ref().map(rift_core::layout::scrolling::Strip::place),
            |w| w,
        )
    }

    use crate::layout_engine::systems::constraints as upstream;
    use crate::layout_engine::systems::drawn::Rng;

    fn frame(window: u32, x: f64) -> (WindowId, Rect) {
        (WindowId::new(1, window), Rect { x, y: 0.0, w: 100.0, h: 50.0 })
    }

    /// The Rust core decides without a Roc answer: its frames stand over the system's, and a
    /// system that differs is the one divergence.
    #[test]
    fn the_rust_core_decides_over_the_systems_frames() {
        let core = vec![frame(1, 0.0), frame(2, 100.0)];
        let system = vec![frame(1, 0.0), frame(2, 101.0)];
        let (frames, divergences) =
            decided::<_, (), _>(Shadowed::Ran(system.clone()), core.clone(), None, |f| bits(f));
        assert_eq!(frames, core);
        assert_eq!(divergences, vec![Divergence::System(system)]);
        let (frames, divergences) =
            decided::<_, (), _>(Shadowed::Ran(core.clone()), core.clone(), None, |f| bits(f));
        assert_eq!(frames, core);
        assert_eq!(divergences, vec![]);
    }

    /// The Roc core decides when it answers, with the Rust core's frames a shadow too; when it
    /// cannot answer, the Rust core decides and the failure is a divergence.
    #[test]
    fn the_roc_core_decides_when_it_answers() {
        let roc = vec![frame(1, 0.0)];
        let rust = vec![frame(1, 1.0)];
        let (frames, divergences) = decided::<_, (), _>(
            Shadowed::Ran(rust.clone()),
            rust.clone(),
            Some(Ok(roc.clone())),
            |f| bits(f),
        );
        assert_eq!(frames, roc);
        assert_eq!(divergences, vec![
            Divergence::RustCore(rust.clone()),
            Divergence::System(rust.clone())
        ]);
        let (frames, divergences) = decided(
            Shadowed::Ran(roc.clone()),
            rust.clone(),
            Some(Err("no answer")),
            |f| bits(f),
        );
        assert_eq!(frames, rust);
        assert_eq!(divergences, vec![
            Divergence::RocFailed("no answer"),
            Divergence::System(roc)
        ]);
    }

    /// A pass that skips the shadow never asks the system, and the core decides alone.
    #[test]
    fn a_skipped_shadow_is_not_asked_and_the_core_decides() {
        let core = vec![frame(1, 0.0)];
        let (frames, divergences) =
            decided::<_, (), _>(Shadowed::Skipped, core.clone(), None, |f| bits(f));
        assert_eq!(frames, core);
        assert_eq!(divergences, vec![]);
        let asked = std::cell::Cell::new(false);
        let skipped = Shadowed::of(Shadow::Skip, || asked.set(true));
        assert!(matches!(skipped, Shadowed::Skipped) && !asked.get());
    }

    /// Sampled every 32nd pass, a Space's first pass and each 32nd after it run the shadow, and
    /// layout work outside a pass does not; sampling every pass runs it everywhere. Each Space
    /// counts its own passes.
    #[test]
    fn a_sampled_shadow_runs_on_every_32nd_pass_of_each_space() {
        let every = Sampling::Every(std::num::NonZeroU64::new(32).unwrap());
        let ran: Vec<u64> = (0..100).filter(|&p| every.shadow(p) == Shadow::Run).collect();
        assert_eq!(ran, vec![0, 32, 64, 96]);
        assert_eq!(every.outside_a_pass(), Shadow::Skip);
        assert!((0..100).all(|p| Sampling::EveryPass.shadow(p) == Shadow::Run));
        assert_eq!(Sampling::EveryPass.outside_a_pass(), Shadow::Run);
        assert_eq!(
            SAMPLING,
            Sampling::EveryPass,
            "tests run the shadow on every pass"
        );
        let mut passes = ShadowPasses::default();
        let (a, b) = (
            crate::sys::screen::SpaceId::new(1),
            crate::sys::screen::SpaceId::new(2),
        );
        passes.pass(a);
        passes.pass(a);
        passes.pass(b);
        assert_eq!(passes.0.get(&a), Some(&2));
        assert_eq!(passes.0.get(&b), Some(&1));
    }

    /// A system's frames that differ from the core's are counted, then fail the test.
    #[test]
    fn a_layout_divergence_is_counted_and_fails_tests() {
        let screen = Rect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        };
        let window = WindowId::new(1, 1);
        let shape = Traditional {
            screen,
            area: screen,
            gaps: Gaps::default(),
            stack_offset: 0.0,
            stack_line: StackLine {
                thickness: 0.0,
                spacing: 0.0,
                top: true,
                left: false,
            },
            root: rift_core::layout::traditional::Node::Window {
                window,
                constraints: None,
                fill: rift_core::layout::traditional::Fill::Tiled,
            },
        };
        let system = [(
            window,
            CGRect::new(CGPoint::new(1.0, 0.0), CGSize::new(800.0, 600.0)),
        )];
        let before = layout_divergences();
        let failed = std::panic::catch_unwind(|| {
            decide_traditional("test", Some(shape), Shadow::Run, || system.to_vec())
        })
        .expect_err("a divergence fails tests");
        let message = failed.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(message.starts_with("layout-divergence"), "{message}");
        assert!(layout_divergences() > before);
    }

    /// A bound as an app or a container may give it: mostly plain, sometimes 0, negative,
    /// infinite or NaN.
    fn bound(rng: &mut Rng, high: f64) -> f64 {
        match rng.below(12) {
            0 => 0.0,
            1 => -rng.length(1., high),
            2 => f64::INFINITY,
            3 => f64::NAN,
            _ => rng.length(0., high),
        }
    }

    fn weight(rng: &mut Rng) -> f64 {
        match rng.below(10) {
            0 => 0.0,
            1 => rng.length(0., 0.05),
            2 => -rng.length(0., 1.),
            3 => f64::NAN,
            4..=6 => 1.0,
            _ => rng.length(0., 3.),
        }
    }

    /// The core's axis solver is upstream's, bit for bit, over drawn segments: minima, fixed
    /// lengths and maxima that fit or overcommit, weights small, zero, negative and NaN.
    #[test]
    fn the_cores_axis_solver_is_upstreams() {
        let mut rng = Rng(0x5eed_a415);
        let mut lengths = 0;
        for _ in 0..20000 {
            let count = 1 + rng.below(6) as usize;
            let items: Vec<upstream::AxisConstraints> = (0..count)
                .map(|_| upstream::AxisConstraints {
                    min: if rng.chance(50) {
                        0.0
                    } else {
                        bound(&mut rng, 900.)
                    },
                    fixed: rng.chance(25).then(|| bound(&mut rng, 900.)),
                    max: rng.chance(40).then(|| bound(&mut rng, 900.)),
                    weight: weight(&mut rng),
                    can_grow: rng.chance(80),
                })
                .collect();
            let usable = if rng.chance(5) {
                bound(&mut rng, 3000.)
            } else {
                rng.length(0., 3000.)
            };
            let core: Vec<rift_core::layout::axis::AxisConstraints> = items
                .iter()
                .map(|item| rift_core::layout::axis::AxisConstraints {
                    min: item.min,
                    fixed: item.fixed,
                    max: item.max,
                    weight: item.weight,
                    can_grow: item.can_grow,
                })
                .collect();
            let theirs = upstream::solve_axis_lengths(items.clone(), usable);
            let ours = rift_core::layout::axis::solve_axis_lengths(&core, usable);
            let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
            assert_eq!(
                bits(&ours),
                bits(&theirs),
                "{items:?} in {usable}: {ours:?} vs {theirs:?}"
            );
            lengths += count;
        }
        eprintln!("axis solver: {lengths} lengths identical");
    }
}
