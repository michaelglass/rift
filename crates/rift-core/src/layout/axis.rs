//! One axis of a container: summarizing children's constraints, and solving segment lengths.

/// One subtree's constraints along an axis: minimum, fixed length, maximum, and whether it can
/// grow.
pub type AxisSummary = (f64, Option<f64>, Option<f64>, bool);

/// A subtree's constraints along both axes: its parent asks along its own axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Along {
    pub horizontal: AxisSummary,
    pub vertical: AxisSummary,
}

impl Along {
    /// The constraints along each axis, as `summary` gives them.
    pub fn both(summary: impl Fn(bool) -> AxisSummary) -> Self {
        Self {
            horizontal: summary(true),
            vertical: summary(false),
        }
    }

    /// The constraints along one axis.
    pub fn on(self, horizontal: bool) -> AxisSummary {
        if horizontal {
            self.horizontal
        } else {
            self.vertical
        }
    }
}

// Shared by Traditional containers and BSP splits. A sole child contributes
// exactly its own constraints; gaps and stack decorations belong to projections.
//
// `aligned`: the container lays its children out along the axis being summarized; `stacked`:
// its children are stacked (one shown, offsets between them).
pub fn container_axis_constraints(
    children: impl Iterator<Item = AxisSummary>,
    aligned: bool,
    stacked: bool,
    gap: f64,
    stack_offset: f64,
    reserve: f64,
) -> AxisSummary {
    let mut count = 0;
    let mut axis = AxisConstraints {
        fixed: Some(0.0),
        max: Some(0.0),
        ..Default::default()
    };
    let mut focused: f64 = 0.0;
    let combine = |a: f64, b: f64| {
        if aligned && !stacked {
            a + b.max(0.0)
        } else {
            a.max(b.max(0.0))
        }
    };
    for (min, fixed, max, grow) in children {
        count += 1;
        axis.min = combine(axis.min, min);
        axis.fixed = axis.fixed.zip(fixed).map(|(a, b)| combine(a, b));
        axis.max = axis.max.zip(max).map(|(a, b)| combine(a, b));
        axis.can_grow |= grow;
        focused = focused.max(fixed.unwrap_or(min));
    }
    if count == 0 {
        return (0.0, None, None, true);
    }
    let extra = if count == 1 {
        0.0
    } else if !aligned {
        reserve
    } else if stacked {
        stack_offset.max(0.0) * (count - 1) as f64 + reserve
    } else {
        gap * (count - 1) as f64
    };
    if count > 1 && stacked && aligned {
        axis.min = focused;
        axis.fixed = None;
    }
    (
        axis.min + extra,
        axis.fixed.map(|v| v + extra),
        axis.max.map(|v| v + extra),
        axis.can_grow,
    )
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AxisConstraints {
    pub min: f64,
    pub fixed: Option<f64>,
    pub max: Option<f64>,
    pub weight: f64,
    pub can_grow: bool,
}

fn sanitize(v: f64) -> f64 { if v.is_finite() { v.max(0.0) } else { 0.0 } }

/// Solve 1D segment lengths for a container axis (upstream's solver since #688, step for step,
/// so the lengths are its bit for bit).
///
/// Rules:
/// - never negative
/// - `min` is a lower bound unless physically infeasible (then minima are scaled down proportionally)
/// - `fixed` values are enforced before distributing remainder
/// - `max` caps growth when positive
/// - growable nodes preserve their final weighted proportions whenever their bounds allow it
/// - zero-weight growable nodes share equally when no positive weights are present
/// - if nothing can grow, remainder becomes blank space
pub fn solve_axis_lengths(items: &[AxisConstraints], usable: f64) -> Vec<f64> {
    let usable = sanitize(usable);
    let segments: Vec<Segment> = items.iter().map(Segment::of).collect();
    let fixed_sum: f64 = segments.iter().filter_map(|segment| segment.fixed).sum();
    if fixed_sum > usable {
        let scale = usable / fixed_sum;
        return segments.iter().map(|segment| segment.fixed.unwrap_or(0.0) * scale).collect();
    }

    let min_sum: f64 = segments.iter().filter(|s| s.fixed.is_none()).map(|s| s.min).sum();
    let remaining_for_mins = (usable - fixed_sum).max(0.0);
    let segments: Vec<Segment> = if min_sum > remaining_for_mins {
        let scale = remaining_for_mins / min_sum;
        segments
            .into_iter()
            .map(|segment| match segment.fixed {
                None => Segment {
                    min: segment.min * scale,
                    ..segment
                },
                Some(_) => segment,
            })
            .collect()
    } else {
        segments
    };

    // Fixed segments first, in order; then each non-growable segment gets its minimum, in order.
    let mut remaining = usable;
    let mut states: Vec<State> = segments
        .iter()
        .map(|segment| match segment.fixed {
            Some(v) => {
                let length = v.min(remaining);
                remaining = (remaining - length).max(0.0);
                State::Set(length)
            }
            None => State::Growing,
        })
        .collect();
    for (segment, state) in segments.iter().zip(&mut states) {
        if let (State::Growing, false) = (*state, segment.grows) {
            let length = segment.min.min(remaining);
            remaining = (remaining - length).max(0.0);
            *state = State::Set(length);
        }
    }

    // Clamp one side at a time. A binding maximum raises the remaining shares, which can make a
    // tentative minimum unnecessary, and vice versa.
    loop {
        let growing: Vec<&Segment> = segments
            .iter()
            .zip(&states)
            .filter(|(_, state)| **state == State::Growing)
            .map(|(segment, _)| segment)
            .collect();
        if growing.is_empty() || remaining <= f64::EPSILON {
            break;
        }
        let count = growing.len();
        let total_weight: f64 = growing.iter().map(|segment| segment.weight).sum();
        let proposed = |segment: &Segment| {
            if total_weight == 0.0 {
                remaining / count as f64
            } else {
                remaining * (segment.weight / total_weight)
            }
        };
        let bounded = |segment: &Segment| {
            proposed(segment).clamp(
                segment.min,
                segment.max.unwrap_or(f64::INFINITY).max(segment.min),
            )
        };
        let total: f64 = growing.iter().map(|segment| bounded(segment)).sum();
        let tolerance = remaining * f64::EPSILON * count as f64;
        let settle_all = |states: &mut Vec<State>| {
            for (segment, state) in segments.iter().zip(states.iter_mut()) {
                if *state == State::Growing {
                    *state = State::Set(bounded(segment));
                }
            }
        };
        if (total - remaining).abs() <= tolerance {
            settle_all(&mut states);
            break;
        }
        let mut assigned = 0.0;
        let mut clamped_any = false;
        for (segment, state) in segments.iter().zip(states.iter_mut()) {
            if *state != State::Growing {
                continue;
            }
            let (share, bound) = (proposed(segment), bounded(segment));
            if (total > remaining && bound > share) || (total < remaining && bound < share) {
                *state = State::Set(bound);
                assigned += bound;
                clamped_any = true;
            }
        }
        if !clamped_any {
            settle_all(&mut states);
            break;
        }
        remaining = (remaining - assigned).max(0.0);
    }

    states
        .into_iter()
        .map(|state| match state {
            State::Set(length) => length,
            State::Growing => 0.0,
        })
        .collect()
}

/// One segment's bounds, sanitized as the solver reads them.
#[derive(Debug, Clone, Copy)]
struct Segment {
    min: f64,
    fixed: Option<f64>,
    /// A positive maximum (none: no limit).
    max: Option<f64>,
    weight: f64,
    grows: bool,
}

/// How far solving a segment has got.
#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Set(f64),
    Growing,
}

impl Segment {
    fn of(item: &AxisConstraints) -> Self {
        let min = sanitize(item.min);
        let max = item.max.map(sanitize).filter(|&v| v > 0.0);
        // A fixed length within the segment's bounds.
        let fixed = item.fixed.map(|v| {
            let v = sanitize(v).max(min);
            max.map_or(v, |max| v.min(max))
        });
        Segment {
            min,
            fixed,
            max,
            weight: sanitize(item.weight),
            grows: item.can_grow,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AxisConstraints, solve_axis_lengths};

    /// The case that showed the core's solver predated upstream's #688: a minimum that fits
    /// holds, and the rest is shared under the maxima (the pre-#688 solver gave 105, 0, 814).
    #[test]
    fn a_minimum_that_fits_holds_while_maxima_share_the_rest() {
        let grows = |min, max| AxisConstraints {
            min,
            fixed: None,
            max,
            weight: 1.0,
            can_grow: true,
        };
        let solved = solve_axis_lengths(
            &[
                grows(0.0, Some(105.0)),
                grows(0.0, Some(1746.0)),
                grows(869.0, None),
            ],
            919.0,
        );
        assert_eq!(solved, [25.0, 25.0, 869.0]);
    }

    #[test]
    fn mixed_bounds_only_clamp_segments_that_need_it() {
        let flexible = AxisConstraints {
            min: 0.0,
            fixed: None,
            max: None,
            weight: 1.0,
            can_grow: true,
        };
        for (min, expected) in [(40.0, [45.0, 10.0, 45.0]), (60.0, [60.0, 10.0, 30.0])] {
            assert_eq!(
                solve_axis_lengths(
                    &[
                        AxisConstraints { min, ..flexible },
                        AxisConstraints { max: Some(10.0), ..flexible },
                        flexible,
                    ],
                    100.0,
                ),
                expected,
            );
        }
    }

    #[test]
    fn scales_non_fixed_minima_after_reserving_fixed_segments() {
        let solved = solve_axis_lengths(
            &[
                AxisConstraints {
                    min: 0.0,
                    fixed: Some(600.0),
                    max: None,
                    weight: 1.0,
                    can_grow: false,
                },
                AxisConstraints {
                    min: 300.0,
                    fixed: None,
                    max: None,
                    weight: 1.0,
                    can_grow: true,
                },
                AxisConstraints {
                    min: 300.0,
                    fixed: None,
                    max: None,
                    weight: 1.0,
                    can_grow: true,
                },
            ],
            1000.0,
        );

        assert_eq!(solved.len(), 3);
        assert!((solved[0] - 600.0).abs() < 0.001);
        assert!((solved[1] - 200.0).abs() < 0.001);
        assert!((solved[2] - 200.0).abs() < 0.001);
    }

    #[test]
    fn scales_overcommitted_fixed_segments_symmetrically() {
        let solved = solve_axis_lengths(
            &[
                AxisConstraints {
                    min: 0.0,
                    fixed: Some(900.0),
                    max: None,
                    weight: 1.0,
                    can_grow: false,
                },
                AxisConstraints {
                    min: 0.0,
                    fixed: Some(900.0),
                    max: None,
                    weight: 1.0,
                    can_grow: false,
                },
            ],
            1400.0,
        );

        assert_eq!(solved.len(), 2);
        assert!((solved[0] - 700.0).abs() < 0.001);
        assert!((solved[1] - 700.0).abs() < 0.001);
    }

    #[test]
    fn max_caps_participate_in_growth_distribution() {
        let solved = solve_axis_lengths(
            &[
                AxisConstraints {
                    min: 0.0,
                    fixed: None,
                    max: Some(600.0),
                    weight: 1.0,
                    can_grow: true,
                },
                AxisConstraints {
                    min: 0.0,
                    fixed: None,
                    max: None,
                    weight: 1.0,
                    can_grow: true,
                },
            ],
            1600.0,
        );

        assert_eq!(solved.len(), 2);
        assert!((solved[0] - 600.0).abs() < 0.001);
        assert!((solved[1] - 1000.0).abs() < 0.001);
    }

    #[test]
    fn non_binding_minima_do_not_skew_equal_weight_segments() {
        let solved = solve_axis_lengths(
            &[
                AxisConstraints {
                    min: 0.0,
                    fixed: None,
                    max: None,
                    weight: 1.0,
                    can_grow: true,
                },
                AxisConstraints {
                    min: 400.0,
                    fixed: None,
                    max: None,
                    weight: 1.0,
                    can_grow: true,
                },
            ],
            1200.0,
        );

        assert_eq!(solved.len(), 2);
        assert!((solved[0] - 600.0).abs() < 0.001);
        assert!((solved[1] - 600.0).abs() < 0.001);
    }
}
