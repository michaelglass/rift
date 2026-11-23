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
    let upstream: Vec<(WindowId, Rect)> =
        upstream.iter().map(|&(window, frame)| (window, core_rect(frame))).collect();
    let rust = shape.as_ref().map_or_else(Vec::new, Traditional::frames);
    if bits(&rust) != bits(&upstream) {
        diverged("layout-divergence", what, &shape, &upstream, &rust);
    }
    #[cfg(feature = "roc-core")]
    {
        let roc = match &shape {
            None => Vec::new(),
            Some(shape) => match rift_core_roc::tile_traditional(shape.clone().map(&to_plain)) {
                Ok(frames) => {
                    frames.into_iter().map(|(window, frame)| (of_plain(window), frame)).collect()
                }
                Err(error) => {
                    diverged("roc-divergence", what, &shape, &upstream, &error);
                    return;
                }
            },
        };
        if bits(&roc) != bits(&upstream) {
            diverged("roc-divergence", what, &shape, &upstream, &roc);
        }
    }
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
mod tests {
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
