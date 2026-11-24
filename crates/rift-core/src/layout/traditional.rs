//! A traditional (i3-style) layout's frames: containers lay their children out along an axis
//! by weight, or stack them with an offset (one shown on top), as far as the windows'
//! constraints allow. The stack and master-stack systems are traditional trees too.

use super::axis::{
    Along, AxisConstraints, AxisSummary, container_axis_constraints, solve_axis_lengths,
};
use super::{Gaps, Sizing, WindowConstraints, round, tiling_area};
use crate::plain::Rect;

/// A traditional layout (or one subtree of it) as the platform has it, pruned to the nodes
/// that take space.
#[derive(Debug, Clone, PartialEq)]
pub struct Traditional<W> {
    pub screen: Rect,
    /// The area the root is laid out in (the tiling area, unless a system lays out a subtree).
    pub area: Rect,
    pub gaps: Gaps,
    /// How far each window of a stack sits from the one before it.
    pub stack_offset: f64,
    pub stack_line: StackLine,
    /// The subtree laid out (a layout where nothing takes space has no shape).
    pub root: Node<W>,
}

/// The bar a stack reserves room for along one edge (none when `thickness` is 0).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StackLine {
    pub thickness: f64,
    pub spacing: f64,
    /// A horizontal stack's bar is above it (else below).
    pub top: bool,
    /// A vertical stack's bar is left of it (else right).
    pub left: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node<W> {
    Window {
        window: W,
        constraints: Option<WindowConstraints>,
        fill: Fill,
    },
    /// Children laid out along an axis by weight (a container with nothing that takes space is
    /// pruned, so there is always a first).
    Axis {
        horizontal: bool,
        fill: Fill,
        first: Box<Child<W>>,
        rest: Vec<Child<W>>,
    },
    /// A stack: `focused` is shown on top, the others behind it, each offset.
    Stack {
        horizontal: bool,
        fill: Fill,
        before: Vec<Child<W>>,
        focused: Box<Child<W>>,
        after: Vec<Child<W>>,
    },
}

/// A container's child and its weight (the share of an axis it asks for).
#[derive(Debug, Clone, PartialEq)]
pub struct Child<W> {
    pub size: f32,
    pub node: Node<W>,
}

/// How much of the screen a node takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fill {
    Tiled,
    Screen,
    WithinGaps,
}

impl StackLine {
    fn total(self) -> f64 {
        if self.thickness > 0.0 {
            self.thickness + self.spacing
        } else {
            0.0
        }
    }

    /// The reservation along one axis of a stack: a horizontal stack gives up height for its
    /// bar, a vertical one width.
    fn on_axis(self, stack_horizontal: bool, horizontal: bool) -> f64 {
        if stack_horizontal != horizontal {
            self.total()
        } else {
            0.0
        }
    }

    /// A stack's frame without its bar.
    fn inset(self, mut rect: Rect, horizontal_stack: bool) -> Rect {
        let reserve = self.total();
        if horizontal_stack {
            let reserve = reserve.min(rect.h);
            if self.top {
                rect.y += reserve;
            }
            rect.h -= reserve;
        } else {
            let reserve = reserve.min(rect.w);
            if self.left {
                rect.x += reserve;
            }
            rect.w -= reserve;
        }
        rect
    }
}

impl<W> Traditional<W> {
    /// The same shape on other window ids (another platform's types).
    pub fn map<V>(self, window: &impl Fn(W) -> V) -> Traditional<V> {
        Traditional {
            screen: self.screen,
            area: self.area,
            gaps: self.gaps,
            stack_offset: self.stack_offset,
            stack_line: self.stack_line,
            root: self.root.map(window),
        }
    }
}

impl<W> Node<W> {
    fn map<V>(self, window: &impl Fn(W) -> V) -> Node<V> {
        match self {
            Node::Window { window: id, constraints, fill } => Node::Window {
                window: window(id),
                constraints,
                fill,
            },
            Node::Axis { horizontal, fill, first, rest } => Node::Axis {
                horizontal,
                fill,
                first: Box::new(first.map(window)),
                rest: rest.into_iter().map(|child| child.map(window)).collect(),
            },
            Node::Stack {
                horizontal,
                fill,
                before,
                focused,
                after,
            } => Node::Stack {
                horizontal,
                fill,
                before: before.into_iter().map(|child| child.map(window)).collect(),
                focused: Box::new(focused.map(window)),
                after: after.into_iter().map(|child| child.map(window)).collect(),
            },
        }
    }
}

impl<W> Child<W> {
    fn map<V>(self, window: &impl Fn(W) -> V) -> Child<V> {
        Child {
            size: self.size,
            node: self.node.map(window),
        }
    }
}

/// A leaf's constraints along one axis. A resizable window's minimum is honoured when splitting
/// only under `respect_min_size` (`tiled_min`); a fixed length always is.
fn leaf_summary(c: WindowConstraints, horizontal: bool) -> AxisSummary {
    let c = c.normalized();
    let min = c.tiled_min(horizontal);
    let max = (c.max_for_axis(horizontal) > 0.0).then(|| c.max_for_axis(horizontal));
    match c.sizing {
        Sizing::Resizable { .. } | Sizing::ResizableAtLeastMin { .. } => {
            match c.fixed_for_axis(horizontal) {
                Some(fixed) => (fixed, Some(fixed), Some(fixed), false),
                None => (min, None, max, true),
            }
        }
        Sizing::Fixed { .. } => {
            let fixed = c.fixed_for_axis(horizontal).filter(|v| *v > 0.0);
            (min, fixed, max.or(fixed), fixed.is_none())
        }
    }
}

impl<W: Copy> Traditional<W> {
    /// Each window's frame, in tree order.
    pub fn frames(&self) -> Vec<(W, Rect)> {
        let mut out = Vec::new();
        self.place(&self.summarized(&self.root), self.area, &mut out);
        out
    }

    /// The tree with each subtree's constraints along both axes, folded once bottom-up (a
    /// container asks each child's along its own axis; recomputing them at every level made a
    /// deep tree's frames quadratic).
    fn summarized(&self, node: &Node<W>) -> Summarized<W> {
        let child = |child: &Child<W>| SummarizedChild {
            size: child.size,
            node: self.summarized(&child.node),
        };
        match node {
            Node::Window { window, constraints, fill } => Summarized {
                along: Along::both(|horizontal| match constraints {
                    Some(c) => leaf_summary(*c, horizontal),
                    None => (0.0, None, None, true),
                }),
                node: Annotated::Window {
                    window: *window,
                    constraints: *constraints,
                    fill: *fill,
                },
            },
            Node::Axis {
                horizontal: along,
                fill,
                first,
                rest,
            } => {
                let first = Box::new(child(first));
                let rest: Vec<SummarizedChild<W>> = rest.iter().map(child).collect();
                Summarized {
                    along: Along::both(|horizontal| {
                        self.container(
                            std::iter::once(&*first).chain(&rest),
                            *along == horizontal,
                            false,
                            horizontal,
                            0.0,
                        )
                    }),
                    node: Annotated::Axis {
                        horizontal: *along,
                        fill: *fill,
                        first,
                        rest,
                    },
                }
            }
            Node::Stack {
                horizontal: along,
                fill,
                before,
                focused,
                after,
            } => {
                let before: Vec<SummarizedChild<W>> = before.iter().map(child).collect();
                let focused = Box::new(child(focused));
                let after: Vec<SummarizedChild<W>> = after.iter().map(child).collect();
                Summarized {
                    along: Along::both(|horizontal| {
                        self.container(
                            before.iter().chain(std::iter::once(&*focused)).chain(&after),
                            *along == horizontal,
                            true,
                            horizontal,
                            self.stack_line.on_axis(*along, horizontal),
                        )
                    }),
                    node: Annotated::Stack {
                        horizontal: *along,
                        fill: *fill,
                        before,
                        focused,
                        after,
                    },
                }
            }
        }
    }

    fn container<'a>(
        &self,
        children: impl Iterator<Item = &'a SummarizedChild<W>>,
        aligned: bool,
        stacked: bool,
        horizontal: bool,
        reserve: f64,
    ) -> AxisSummary
    where
        W: 'a,
    {
        container_axis_constraints(
            children.map(|child| child.node.along.on(horizontal)),
            aligned,
            stacked,
            if horizontal {
                self.gaps.horizontal
            } else {
                self.gaps.vertical
            },
            self.stack_offset,
            reserve,
        )
    }

    fn filled(&self, fill: Fill, rect: Rect) -> Rect {
        match fill {
            Fill::Tiled => rect,
            Fill::Screen => self.screen,
            Fill::WithinGaps => tiling_area(self.screen, self.gaps),
        }
    }

    fn place(&self, node: &Summarized<W>, rect: Rect, out: &mut Vec<(W, Rect)>) {
        match &node.node {
            Annotated::Window { window, constraints, fill } => {
                let rect = self.filled(*fill, rect);
                let frame = match constraints {
                    Some(c) => {
                        let (min_w, fixed_w, max_w, _) = leaf_summary(*c, true);
                        let (min_h, fixed_h, max_h, _) = leaf_summary(*c, false);
                        let desired_w = fixed_w.unwrap_or(rect.w).max(min_w);
                        let desired_h = fixed_h.unwrap_or(rect.h).max(min_h);
                        let desired_w = max_w.map_or(desired_w, |m| desired_w.min(m));
                        let desired_h = max_h.map_or(desired_h, |m| desired_h.min(m));
                        Rect {
                            w: desired_w.min(rect.w).max(0.0),
                            h: desired_h.min(rect.h).max(0.0),
                            ..rect
                        }
                    }
                    None => rect,
                };
                out.push((*window, frame));
            }
            Annotated::Stack {
                horizontal,
                fill,
                before,
                focused,
                after,
            } => self.stack(
                before,
                focused,
                after,
                *horizontal,
                self.filled(*fill, rect),
                out,
            ),
            Annotated::Axis { horizontal, fill, first, rest } => {
                self.axis(first, rest, *horizontal, self.filled(*fill, rect), out)
            }
        }
    }

    fn stack(
        &self,
        before: &[SummarizedChild<W>],
        focused: &SummarizedChild<W>,
        after: &[SummarizedChild<W>],
        horizontal: bool,
        rect: Rect,
        out: &mut Vec<(W, Rect)>,
    ) {
        let count = before.len() + 1 + after.len();
        let axis_len = if horizontal { rect.w } else { rect.h };
        // The offset is capped so the window on top keeps its required length (a maximum is no
        // requirement).
        let offset = if count > 1 {
            let (min, fixed, _, _) = focused.node.along.on(horizontal);
            let desired = fixed.unwrap_or(min).clamp(0.0, axis_len.max(0.0));
            let max_offset = (axis_len - desired).max(0.0) / (count - 1) as f64;
            self.stack_offset.min(max_offset)
        } else {
            self.stack_offset
        };
        let frames = stack_frames(rect, count, offset, horizontal, self.stack_line);
        let children = before.iter().chain(std::iter::once(focused)).chain(after);
        for (child, frame) in children.zip(frames) {
            self.place(&child.node, frame, out);
        }
    }

    fn axis(
        &self,
        first: &SummarizedChild<W>,
        rest: &[SummarizedChild<W>],
        horizontal: bool,
        rect: Rect,
        out: &mut Vec<(W, Rect)>,
    ) {
        let children: Vec<&SummarizedChild<W>> = std::iter::once(first).chain(rest).collect();
        // Weights as upstream's `layout_axis` reads them (since #688): each as an f64, and
        // equal weights only when one is negative or not finite, or they add up to nothing (a
        // small positive weight, which splitting and promotion can leave, stands).
        let mut total = 0.0;
        let mut invalid = false;
        for child in &children {
            let weight = f64::from(child.size);
            total += weight;
            invalid |= !weight.is_finite() || weight < 0.0;
        }
        let equal = invalid || !total.is_finite() || total <= 0.0;
        let gap = if horizontal {
            self.gaps.horizontal
        } else {
            self.gaps.vertical
        };
        let axis_len = if horizontal { rect.w } else { rect.h };
        let total_gap = rest.len() as f64 * gap;
        let usable = if gap == 0.0 {
            axis_len
        } else {
            (axis_len - total_gap).max(0.0)
        };
        let items: Vec<AxisConstraints> = children
            .iter()
            .map(|child| {
                let (min, fixed, max, can_grow) = child.node.along.on(horizontal);
                let weight = if equal { 1.0 } else { f64::from(child.size) };
                AxisConstraints {
                    min,
                    fixed,
                    max,
                    weight,
                    can_grow,
                }
            })
            .collect();
        let lengths = solve_axis_lengths(&items, usable);
        let mut offset = if horizontal { rect.x } else { rect.y };
        for (index, (child, length)) in children.iter().zip(lengths).enumerate() {
            let frame = round(if horizontal {
                Rect { x: offset, w: length, ..rect }
            } else {
                Rect { y: offset, h: length, ..rect }
            });
            self.place(&child.node, frame, out);
            offset += length;
            if index < rest.len() {
                offset += gap;
            }
        }
    }
}

/// A node of a [`Traditional`] with its subtree's constraints along both axes, made for one
/// `frames` and dropped with it.
struct Summarized<W> {
    along: Along,
    node: Annotated<W>,
}

enum Annotated<W> {
    Window {
        window: W,
        constraints: Option<WindowConstraints>,
        fill: Fill,
    },
    Axis {
        horizontal: bool,
        fill: Fill,
        first: Box<SummarizedChild<W>>,
        rest: Vec<SummarizedChild<W>>,
    },
    Stack {
        horizontal: bool,
        fill: Fill,
        before: Vec<SummarizedChild<W>>,
        focused: Box<SummarizedChild<W>>,
        after: Vec<SummarizedChild<W>>,
    },
}

struct SummarizedChild<W> {
    size: f32,
    node: Summarized<W>,
}

/// The frames of a stack of `count` windows in `rect`, each `offset` further along than the one
/// before (leaving room for the stack's bar when there is more than one).
pub fn stack_frames(
    rect: Rect,
    count: usize,
    offset: f64,
    horizontal: bool,
    line: StackLine,
) -> impl Iterator<Item = Rect> {
    let container = if count > 1 {
        line.inset(rect, horizontal)
    } else {
        rect
    };
    let spread = count.saturating_sub(1) as f64 * offset;
    let (window_w, window_h) = if horizontal {
        ((container.w - spread).max(100.0), container.h.max(100.0))
    } else {
        (container.w.max(100.0), (container.h - spread).max(100.0))
    };
    (0..count).map(move |index| {
        let shift = index as f64 * offset;
        let (dx, dy) = if horizontal {
            (shift, 0.0)
        } else {
            (0.0, shift)
        };
        let w = window_w.min(container.w);
        let h = window_h.min(container.h);
        let max_x = (container.x + container.w - w).max(container.x);
        let max_y = (container.y + container.h - h).max(container.y);
        round(Rect {
            x: (container.x + dx).clamp(container.x, max_x),
            y: (container.y + dy).clamp(container.y, max_y),
            w,
            h,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Size;

    fn constraints(sizing: Sizing) -> WindowConstraints {
        WindowConstraints { sizing, ..Default::default() }
    }

    #[test]
    fn a_resizable_windows_reported_minimum_holds_a_split_only_under_respect_min_size() {
        let min = Size::reported(400.0, 600.0);
        // Advisory: traditional tiling may shrink it below its reported minimum.
        let advisory = leaf_summary(constraints(Sizing::new(true, false, min)), false);
        assert_eq!(advisory, (0.0, None, None, true));
        // Respected: its minimum holds, and it can still grow.
        let respected = leaf_summary(constraints(Sizing::new(true, true, min)), false);
        assert_eq!(respected, (600.0, None, None, true));
        // Not resizable: its minimum holds too.
        let fixed = leaf_summary(constraints(Sizing::new(false, false, min)), false);
        assert_eq!(fixed.0, 600.0);
    }

    #[test]
    fn an_axis_locked_by_equal_bounds_is_fixed_whatever_the_rule() {
        for respect in [false, true] {
            let c = WindowConstraints {
                max_width: 500.0,
                ..constraints(Sizing::new(true, respect, Size::reported(500.0, 0.0)))
            };
            assert_eq!(leaf_summary(c, true), (500.0, Some(500.0), Some(500.0), false));
        }
    }
}
