use crate::layout_engine::LayoutKind;

// Both axes are prepared bottom-up; absent nodes do not participate.
pub(super) type ConstraintCache =
    crate::common::collections::HashMap<crate::model::tree::NodeId, [AxisSummary; 2]>;

pub(crate) use rift_core::layout::axis::{AxisConstraints, AxisSummary, solve_axis_lengths};

/// [`rift_core::layout::axis::container_axis_constraints`] for a container of `kind`.
pub(super) fn container_axis_constraints(
    children: impl Iterator<Item = AxisSummary>,
    kind: LayoutKind,
    horizontal: bool,
    gap: f64,
    stack_offset: f64,
    reserve: f64,
) -> AxisSummary {
    let aligned =
        (kind.orientation() == crate::layout_engine::Orientation::Horizontal) == horizontal;
    rift_core::layout::axis::container_axis_constraints(
        children,
        aligned,
        kind.is_stacked(),
        gap,
        stack_offset,
        reserve,
    )
}
