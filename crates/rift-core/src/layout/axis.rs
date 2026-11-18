//! One axis of a container: summarizing children's constraints, and solving segment lengths.

/// One subtree's constraints along an axis: minimum, fixed length, maximum, and whether it can
/// grow.
pub type AxisSummary = (f64, Option<f64>, Option<f64>, bool);

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

/// Solve 1D segment lengths for a container axis.
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
    if fixed_sum > usable && fixed_sum > 0.0 {
        let scale = usable / fixed_sum;
        return segments
            .iter()
            .map(|segment| segment.fixed.map_or(0.0, |v| v * scale))
            .collect();
    }

    let min_sum: f64 = segments.iter().filter(|s| s.fixed.is_none()).map(|s| s.low).sum();
    let remaining_for_mins = (usable - fixed_sum).max(0.0);
    let scale =
        (min_sum > remaining_for_mins && min_sum > 0.0).then(|| remaining_for_mins / min_sum);
    let segments = segments.into_iter().map(|segment| match (segment.fixed, scale) {
        (None, Some(scale)) => Segment {
            low: segment.low * scale,
            ..segment
        },
        (Some(_), _) | (None, None) => segment,
    });

    // Fixed lengths first, in order.
    let mut remaining = usable;
    let mut placed: Vec<(Segment, State)> = segments
        .map(|segment| match segment.fixed {
            Some(v) => {
                let assigned = v.min(remaining);
                remaining = (remaining - assigned).max(0.0);
                (segment, State::Set(assigned))
            }
            None => (segment, State::Growing),
        })
        .collect();
    // Non-growable segments stay at their minimum, in order. Growable segments are solved from
    // their *final* weighted sizes, clamped to their bounds. Seeding every segment with its
    // minimum and distributing only the remainder would skew equal-weight splits whenever the
    // minima differ.
    for (segment, state) in &mut placed {
        if let (State::Growing, None, false) = (*state, segment.fixed, segment.grows) {
            let assigned = segment.low.min(remaining);
            remaining = (remaining - assigned).max(0.0);
            *state = State::Set(assigned);
        }
    }

    while remaining > f64::EPSILON && placed.iter().any(|(_, state)| *state == State::Growing) {
        let growing = || placed.iter().filter(|(_, state)| *state == State::Growing);
        let total_weight: f64 = growing().map(|(segment, _)| segment.weight).sum();
        let use_equal_weights = total_weight <= f64::EPSILON;
        let divisor = if use_equal_weights {
            growing().count() as f64
        } else {
            total_weight
        };
        let share = |segment: &Segment| {
            let weight = if use_equal_weights {
                1.0
            } else {
                segment.weight
            };
            remaining * weight / divisor
        };
        let clamps: Vec<Option<f64>> = placed
            .iter()
            .map(|(segment, state)| match state {
                State::Growing => segment.clamp(share(segment), remaining),
                State::Set(_) => None,
            })
            .collect();
        if clamps.iter().all(Option::is_none) {
            for (segment, state) in &mut placed {
                if *state == State::Growing {
                    *state = State::Set(share(segment));
                }
            }
            remaining = 0.0;
            break;
        }
        let clamped_sum: f64 = clamps.iter().flatten().sum();
        remaining = (remaining - clamped_sum).max(0.0);
        for ((_, state), clamp) in placed.iter_mut().zip(clamps) {
            if let Some(length) = clamp {
                *state = State::Set(length);
            }
        }
    }

    let mut lengths: Vec<f64> = placed
        .iter()
        .map(|(_, state)| match state {
            State::Set(length) => *length,
            State::Growing => 0.0,
        })
        .collect();
    if remaining <= f64::EPSILON {
        let used: f64 = lengths.iter().sum();
        let drift = usable - used;
        if drift.abs() > f64::EPSILON
            && let Some(last) = lengths.iter_mut().rev().find(|length| **length > 0.0)
        {
            *last = (*last + drift).max(0.0);
        }
    }
    lengths
}

/// One segment's bounds, sanitized.
#[derive(Debug, Clone, Copy)]
struct Segment {
    low: f64,
    fixed: Option<f64>,
    high: Option<f64>,
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
        let low = sanitize(item.min);
        let high = item.max.map(sanitize).filter(|v| *v > 0.0 && v.is_finite());
        // A fixed length within the segment's bounds.
        let fixed = item.fixed.map(sanitize).filter(|v| v.is_finite()).map(|v| {
            let v = if v < low { low } else { v };
            match high {
                Some(high) if v > high => high,
                Some(_) | None => v,
            }
        });
        Segment {
            low,
            fixed,
            high,
            weight: sanitize(item.weight),
            grows: item.can_grow,
        }
    }

    /// The length a bound holds a growing segment to, if its share breaks one.
    fn clamp(&self, proposed: f64, remaining: f64) -> Option<f64> {
        if proposed + f64::EPSILON < self.low {
            return Some(self.low.min(remaining));
        }
        let high = self.high.map(|value| value.max(self.low))?;
        (proposed > high + f64::EPSILON).then(|| high.min(remaining))
    }
}

#[cfg(test)]
mod tests {
    use super::{AxisConstraints, solve_axis_lengths};

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
