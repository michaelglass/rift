//! The conversions between rift's geometry and the core's plain geometry, and the check that
//! holds the core's frames to the layout systems'.

use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use rift_core::layout::Gaps;
#[cfg(any(test, feature = "roc-core"))]
use rift_core::layout::traditional::{StackLine, Traditional};
use rift_core::plain::Rect;

#[cfg(any(test, feature = "roc-core"))]
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
#[cfg(any(test, feature = "roc-core"))]
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
#[cfg(any(test, feature = "roc-core"))]
pub(crate) fn core_stack_line(line: crate::common::config::StackLineReserve) -> StackLine {
    use crate::common::config::{HorizontalPlacement, VerticalPlacement};
    StackLine {
        thickness: line.thickness,
        spacing: line.spacing,
        top: line.horiz == HorizontalPlacement::Top,
        left: line.vert == VerticalPlacement::Left,
    }
}

/// Holds the core's frames for a traditional `shape` (none: nothing takes space) to the frames
/// rift's layout system computed (`upstream`), bit for bit; under `roc-core` the Roc core's too.
/// rift's frames stand either way: a difference fails tests and a running Rift warns
/// `layout-divergence:` (the Rust core) or `roc-divergence:` (the Roc core).
#[cfg(any(test, feature = "roc-core"))]
pub(crate) fn check_traditional(
    what: &str,
    shape: Option<Traditional<WindowId>>,
    upstream: &[(WindowId, CGRect)],
) {
    let rust = shape.as_ref().map_or_else(Vec::new, Traditional::frames);
    #[cfg(feature = "roc-core")]
    let roc = || match &shape {
        None => Ok(Vec::new()),
        Some(shape) => rift_core_roc::tile_traditional(shape.clone().map(&to_plain)),
    };
    #[cfg(not(feature = "roc-core"))]
    let roc = no_roc;
    check(what, &shape, upstream, rust, roc);
}

/// Holds the core's frames for a BSP `shape` (none: nothing takes space) to the BSP system's, as
/// [`check_traditional`] does.
#[cfg(any(test, feature = "roc-core"))]
pub(crate) fn check_bsp(
    shape: Option<rift_core::layout::bsp::Bsp<WindowId>>,
    upstream: &[(WindowId, CGRect)],
) {
    let rust = shape.as_ref().map_or_else(Vec::new, rift_core::layout::bsp::Bsp::frames);
    #[cfg(feature = "roc-core")]
    let roc = || match &shape {
        None => Ok(Vec::new()),
        Some(shape) => rift_core_roc::tile_bsp(shape.clone().map(&to_plain)),
    };
    #[cfg(not(feature = "roc-core"))]
    let roc = no_roc;
    check("bsp", &shape, upstream, rust, roc);
}

/// Where a scrolling layout's system placed its strip in world space: each column (id, x,
/// width), each window's frame, the camera's resting points (offset, the column's id) and its
/// bounds.
#[cfg(any(test, feature = "roc-core"))]
#[derive(Debug)]
pub(crate) struct StripPlacement {
    pub columns: Vec<(u64, f64, f64)>,
    pub frames: Vec<(WindowId, CGRect)>,
    pub snaps: Vec<(f64, u64)>,
    pub bounds: (f64, f64),
}

/// The placement as words: ids as they are, floats as their bits.
#[cfg(any(test, feature = "roc-core"))]
fn placement_words<W: Copy>(
    columns: impl Iterator<Item = (u64, f64, f64)>,
    frames: &[(W, Rect)],
    snaps: impl Iterator<Item = (f64, u64)>,
    bounds: (f64, f64),
    window: impl Fn(W) -> u64,
) -> Vec<u64> {
    let mut words = Vec::new();
    for (id, x, width) in columns {
        words.extend([id, x.to_bits(), width.to_bits()]);
    }
    words.push(u64::MAX);
    for &(w, r) in frames {
        words.push(window(w));
        words.extend([r.x, r.y, r.w, r.h].map(f64::to_bits));
    }
    words.push(u64::MAX);
    for (offset, column) in snaps {
        words.extend([offset.to_bits(), column]);
    }
    words.extend([bounds.0.to_bits(), bounds.1.to_bits()]);
    words
}

/// The core's placement of a scrolling `strip` (none: no window takes part, so no column, frame
/// or resting point, and bounds at 0) as words.
#[cfg(any(test, feature = "roc-core"))]
fn placed_words<W: Copy>(
    placed: Option<rift_core::layout::scrolling::Placed<W, u64>>,
    window: impl Fn(W) -> u64,
) -> Vec<u64> {
    match placed {
        None => placement_words(std::iter::empty(), &[], std::iter::empty(), (0.0, 0.0), window),
        Some(placed) => placement_words(
            std::iter::once(&placed.first)
                .chain(&placed.rest)
                .map(|spot| (spot.id, spot.world_x, spot.width)),
            &placed.frames,
            std::iter::once(&placed.snaps.0)
                .chain(&placed.snaps.1)
                .map(|snap| (snap.offset, snap.column)),
            placed.bounds,
            window,
        ),
    }
}

/// Holds the core's placement of a scrolling strip (none: no window takes part) to the
/// scrolling system's (`Geometry::build`), word for word; under `roc-core` the Roc core's too.
/// The system's placement stands either way, as [`check_traditional`]'s frames do.
#[cfg(any(test, feature = "roc-core"))]
pub(crate) fn check_strip(
    strip: Option<rift_core::layout::scrolling::Strip<WindowId, u64>>,
    upstream: &StripPlacement,
) {
    let frames: Vec<(WindowId, Rect)> = upstream
        .frames
        .iter()
        .map(|&(window, frame)| (window, core_rect(frame)))
        .collect();
    let theirs = placement_words(
        upstream.columns.iter().copied(),
        &frames,
        upstream.snaps.iter().copied(),
        upstream.bounds,
        window_word,
    );
    let rust = placed_words(
        strip.as_ref().map(rift_core::layout::scrolling::Strip::place),
        window_word,
    );
    if rust != theirs {
        diverged("layout-divergence", "scrolling", &strip, upstream, &rust);
    }
    #[cfg(feature = "roc-core")]
    {
        let plain = |w: rift_core::plain::WindowId| window_word(of_plain(w));
        let roc = match &strip {
            None => Ok(placed_words(None, plain)),
            Some(strip) => rift_core_roc::place_strip(strip.clone().map(&to_plain, &|id| id))
                .map(|placed| placed_words(Some(placed), plain)),
        };
        match roc {
            Ok(roc) if roc == theirs => {}
            Ok(roc) => diverged("roc-divergence", "scrolling", &strip, upstream, &roc),
            Err(error) => diverged("roc-divergence", "scrolling", &strip, upstream, &error),
        }
    }
}

/// Holds the core's frames for a placed strip shown by a camera (`translate`) to the scrolling
/// system's (`translate_frames`), as [`check_traditional`] does.
#[cfg(any(test, feature = "roc-core"))]
pub(crate) fn check_translation(
    world: &[(WindowId, Rect)],
    camera: rift_core::layout::scrolling::Camera,
    fullscreen: &[(WindowId, rift_core::layout::scrolling::Fullscreen)],
    upstream: &[(WindowId, CGRect)],
) {
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
    check("scrolling translation", &question, upstream, rust, roc);
}

/// Holds the core's frames for a floating layout to the floating system's, as
/// [`check_traditional`] does.
#[cfg(any(test, feature = "roc-core"))]
pub(crate) fn check_floating(
    layout: rift_core::layout::floating::Layout<WindowId>,
    upstream: &[(WindowId, CGRect)],
) {
    let rust = layout.frames();
    #[cfg(feature = "roc-core")]
    let roc = || rift_core_roc::floating_frames(layout.clone().map(&to_plain));
    #[cfg(not(feature = "roc-core"))]
    let roc = no_roc;
    check("floating", &layout, upstream, rust, roc);
}

/// Without `roc-core` there is no Roc core to ask.
#[cfg(all(test, not(feature = "roc-core")))]
fn no_roc() -> Result<Vec<(rift_core::plain::WindowId, Rect)>, std::convert::Infallible> {
    Ok(Vec::new())
}

/// The core's frames (`rust`, and under `roc-core` what `roc` answers) against rift's.
#[cfg(any(test, feature = "roc-core"))]
fn check<E: std::fmt::Debug>(
    what: &str,
    shape: &impl std::fmt::Debug,
    upstream: &[(WindowId, CGRect)],
    rust: Vec<(WindowId, Rect)>,
    #[cfg_attr(not(feature = "roc-core"), allow(unused_variables))] roc: impl FnOnce() -> Result<
        Vec<(rift_core::plain::WindowId, Rect)>,
        E,
    >,
) {
    let upstream: Vec<(WindowId, Rect)> =
        upstream.iter().map(|&(window, frame)| (window, core_rect(frame))).collect();
    if bits(&rust) != bits(&upstream) {
        diverged("layout-divergence", what, shape, &upstream, &rust);
    }
    #[cfg(feature = "roc-core")]
    match roc() {
        Ok(frames) => {
            let roc: Vec<(WindowId, Rect)> =
                frames.into_iter().map(|(window, frame)| (of_plain(window), frame)).collect();
            if bits(&roc) != bits(&upstream) {
                diverged("roc-divergence", what, shape, &upstream, &roc);
            }
        }
        Err(error) => diverged("roc-divergence", what, shape, &upstream, &error),
    }
}

/// A window as one word (its pid's bits above its index).
#[cfg(any(test, feature = "roc-core"))]
fn window_word(window: WindowId) -> u64 {
    (u64::from(window.pid as u32) << 32) | u64::from(window.idx.get())
}

#[cfg(any(test, feature = "roc-core"))]
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

#[cfg(any(test, feature = "roc-core"))]
fn diverged(
    label: &str,
    what: &str,
    shape: &impl std::fmt::Debug,
    upstream: &impl std::fmt::Debug,
    core: &impl std::fmt::Debug,
) {
    #[cfg(test)]
    panic!("{label}: {what} {shape:?}: upstream {upstream:?}, core {core:?}");
    #[cfg(not(test))]
    tracing::warn!(
        what,
        ?shape,
        ?upstream,
        ?core,
        "{label}: the core's frames differ from the layout system's"
    );
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The core's placement as the check compares it.
    pub(crate) fn placed_words_for_test(
        placed: Option<rift_core::layout::scrolling::Placed<WindowId, u64>>,
    ) -> Vec<u64> {
        placed_words(placed, window_word)
    }

    /// A system's placement as the check compares it.
    pub(crate) fn placement_words_for_test(upstream: &StripPlacement) -> Vec<u64> {
        let frames: Vec<(WindowId, Rect)> = upstream
            .frames
            .iter()
            .map(|&(window, frame)| (window, core_rect(frame)))
            .collect();
        placement_words(
            upstream.columns.iter().copied(),
            &frames,
            upstream.snaps.iter().copied(),
            upstream.bounds,
            window_word,
        )
    }

    use crate::layout_engine::systems::constraints as upstream;
    use crate::layout_engine::systems::drawn::Rng;

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
