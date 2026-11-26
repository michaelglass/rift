#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AxisConstraints {
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
pub(crate) fn solve_axis_lengths(
    mut items: impl AsMut<[AxisConstraints]>,
    usable: f64,
) -> Vec<f64> {
    let items = items.as_mut();
    if items.is_empty() {
        return Vec::new();
    }
    let usable = sanitize(usable);
    let n = items.len();
    for item in items.iter_mut() {
        item.min = sanitize(item.min);
        item.max = item.max.map(sanitize).filter(|&v| v > 0.0);
        item.fixed = item.fixed.map(|v| {
            let v = sanitize(v).max(item.min);
            item.max.map_or(v, |max| v.min(max))
        });
        item.weight = sanitize(item.weight);
    }
    let fixed_sum: f64 = items.iter().filter_map(|item| item.fixed).sum();
    if fixed_sum > usable {
        let scale = usable / fixed_sum;
        return items.iter().map(|item| item.fixed.unwrap_or(0.0) * scale).collect();
    }

    let min_sum: f64 = items.iter().filter(|item| item.fixed.is_none()).map(|item| item.min).sum();
    let remaining_for_mins = (usable - fixed_sum).max(0.0);
    if min_sum > remaining_for_mins {
        let scale = remaining_for_mins / min_sum;
        for item in items.iter_mut().filter(|item| item.fixed.is_none()) {
            item.min *= scale;
        }
    }

    let mut lengths = vec![0.0; n];
    let mut remaining = usable;
    // Give fixed segments priority, then assign non-growable segments their minima.
    for (idx, item) in items.iter().enumerate() {
        if let Some(v) = item.fixed {
            lengths[idx] = v.min(remaining);
            remaining = (remaining - lengths[idx]).max(0.0);
        }
    }
    let mut growable = Vec::new();
    for (idx, item) in items.iter().enumerate().filter(|(_, item)| item.fixed.is_none()) {
        if item.can_grow {
            growable.push(idx);
        } else {
            lengths[idx] = item.min.min(remaining);
            remaining = (remaining - lengths[idx]).max(0.0);
        }
    }

    // Clamp one side at a time. A binding maximum raises the remaining
    // shares, which can make a tentative minimum unnecessary, and vice versa.
    while !growable.is_empty() && remaining > f64::EPSILON {
        let growable_count = growable.len();
        let total_weight: f64 = growable.iter().map(|&idx| items[idx].weight).sum();
        let proposed = |item: AxisConstraints| {
            if total_weight == 0.0 {
                remaining / growable_count as f64
            } else {
                remaining * (item.weight / total_weight)
            }
        };
        let bounded = |item: AxisConstraints| {
            proposed(item).clamp(item.min, item.max.unwrap_or(f64::INFINITY).max(item.min))
        };
        let total: f64 = growable.iter().map(|&idx| bounded(items[idx])).sum();
        let tolerance = remaining * f64::EPSILON * growable_count as f64;
        if (total - remaining).abs() <= tolerance {
            for idx in growable {
                lengths[idx] = bounded(items[idx]);
            }
            break;
        }

        let before = growable.len();
        let mut assigned = 0.0;
        growable.retain(|&idx| {
            let item = items[idx];
            let share = proposed(item);
            let bound = bounded(item);
            if (total > remaining && bound > share) || (total < remaining && bound < share) {
                lengths[idx] = bound;
                assigned += bound;
                false
            } else {
                true
            }
        });
        if growable.len() == before {
            for idx in growable {
                lengths[idx] = bounded(items[idx]);
            }
            break;
        }
        remaining = (remaining - assigned).max(0.0);
    }

    lengths
}

#[cfg(test)]
mod tests {
    use super::{AxisConstraints, solve_axis_lengths};

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
                    [
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
            [
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
}
