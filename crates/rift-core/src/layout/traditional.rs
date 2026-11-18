//! A traditional (i3-style) layout's frames: containers lay their children out along an axis
//! by weight, or stack them with an offset (one shown on top), as far as the windows'
//! constraints allow. The stack and master-stack systems are traditional trees too.

use super::axis::{AxisConstraints, AxisSummary, container_axis_constraints, solve_axis_lengths};
use super::{Gaps, WindowConstraints, round, tiling_area};
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
    /// Empty when nothing in the layout takes space.
    pub root: Option<Node<W>>,
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
            root: self.root.map(|root| root.map(window)),
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

/// A leaf's constraints along one axis. A resizable window's minimum and maximum bounds are
/// not honoured when splitting (apps report transient or conservative bounds), only a fixed
/// length is.
fn leaf_summary(c: WindowConstraints, horizontal: bool) -> AxisSummary {
    let c = c.normalized();
    let min = c.min_for_axis(horizontal);
    let max = (c.max_for_axis(horizontal) > 0.0).then(|| c.max_for_axis(horizontal));
    if c.is_resizable {
        if let Some(fixed) = c.fixed_for_axis(horizontal) {
            return (min, Some(fixed), Some(fixed), false);
        }
        return (0.0, None, max, true);
    }
    let fixed = c.fixed_for_axis(horizontal).filter(|v| *v > 0.0);
    (min, fixed, max.or(fixed), fixed.is_none())
}

impl<W: Copy> Traditional<W> {
    /// Each window's frame, in tree order.
    pub fn frames(&self) -> Vec<(W, Rect)> {
        let mut out = Vec::new();
        if let Some(root) = &self.root {
            self.place(root, self.area, &mut out);
        }
        out
    }

    fn summary(&self, node: &Node<W>, horizontal: bool) -> AxisSummary {
        match node {
            Node::Window { constraints: Some(c), .. } => leaf_summary(*c, horizontal),
            Node::Window { constraints: None, .. } => (0.0, None, None, true),
            Node::Axis {
                horizontal: along, first, rest, ..
            } => self.container(
                std::iter::once(&**first).chain(rest),
                *along == horizontal,
                false,
                horizontal,
                0.0,
            ),
            Node::Stack {
                horizontal: along,
                before,
                focused,
                after,
                ..
            } => self.container(
                before.iter().chain(std::iter::once(&**focused)).chain(after),
                *along == horizontal,
                true,
                horizontal,
                self.stack_line.on_axis(*along, horizontal),
            ),
        }
    }

    fn container<'a>(
        &self,
        children: impl Iterator<Item = &'a Child<W>>,
        aligned: bool,
        stacked: bool,
        horizontal: bool,
        reserve: f64,
    ) -> AxisSummary
    where
        W: 'a,
    {
        container_axis_constraints(
            children.map(|child| self.summary(&child.node, horizontal)),
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

    fn place(&self, node: &Node<W>, rect: Rect, out: &mut Vec<(W, Rect)>) {
        match node {
            Node::Window { window, constraints, fill } => {
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
            Node::Stack {
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
            Node::Axis { horizontal, fill, first, rest } => {
                self.axis(first, rest, *horizontal, self.filled(*fill, rect), out)
            }
        }
    }

    fn stack(
        &self,
        before: &[Child<W>],
        focused: &Child<W>,
        after: &[Child<W>],
        horizontal: bool,
        rect: Rect,
        out: &mut Vec<(W, Rect)>,
    ) {
        let count = before.len() + 1 + after.len();
        let axis_len = if horizontal { rect.w } else { rect.h };
        // The offset is capped so the window on top keeps its required length (a maximum is no
        // requirement).
        let offset = if count > 1 {
            let (min, fixed, _, _) = self.summary(&focused.node, horizontal);
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
        first: &Child<W>,
        rest: &[Child<W>],
        horizontal: bool,
        rect: Rect,
        out: &mut Vec<(W, Rect)>,
    ) {
        let children: Vec<&Child<W>> = std::iter::once(first).chain(rest).collect();
        let min_size = 0.05_f32;
        let mut needs_normalization = false;
        let mut actual_total = 0.0_f32;
        for child in &children {
            actual_total += child.size;
            if !child.size.is_finite() || child.size < min_size - f32::EPSILON {
                needs_normalization = true;
            }
        }
        let normalize =
            !actual_total.is_finite() || actual_total <= f32::EPSILON || needs_normalization;
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
                let (min, fixed, max, can_grow) = self.summary(&child.node, horizontal);
                let weight = f64::from(if normalize { 1.0 } else { child.size.max(0.0) });
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
