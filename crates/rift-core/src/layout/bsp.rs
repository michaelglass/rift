//! A BSP layout's frames: every split divides its area in two along one axis, by a ratio, as
//! far as its windows' constraints allow.

use super::axis::{
    Along, AxisConstraints, AxisSummary, container_axis_constraints, solve_axis_lengths,
};
use super::{Gaps, WindowConstraints, tiling_area};
use crate::plain::Rect;

/// A BSP layout as the platform has it, pruned to the nodes that take space (a layout where
/// nothing does has no shape).
#[derive(Debug, Clone, PartialEq)]
pub struct Bsp<W> {
    pub screen: Rect,
    pub gaps: Gaps,
    pub root: Node<W>,
    /// The width/height a layout's only window is fitted to, if set.
    pub aspect: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node<W> {
    Split {
        horizontal: bool,
        /// The first child's share of the split.
        ratio: f32,
        children: Children<W>,
    },
    Leaf {
        slot: Slot<W>,
        fill: Fill,
        /// The leaf holds a stack of windows (one shown).
        stacked: bool,
    },
}

/// A split's children that take space.
#[derive(Debug, Clone, PartialEq)]
pub enum Children<W> {
    /// The other is empty: the one left takes the whole area.
    One(Box<Node<W>>),
    Two(Box<Node<W>>, Box<Node<W>>),
}

/// What a leaf shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Slot<W> {
    /// An empty slot, kept so a window can be put there.
    Empty,
    Window {
        window: W,
        constraints: Option<WindowConstraints>,
    },
}

/// How much of the screen a leaf's window takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fill {
    Tiled,
    Screen,
    WithinGaps,
}

impl<W> Bsp<W> {
    /// The same shape on other window ids (another platform's types).
    pub fn map<V>(self, window: &impl Fn(W) -> V) -> Bsp<V> {
        Bsp {
            screen: self.screen,
            gaps: self.gaps,
            root: self.root.map(window),
            aspect: self.aspect,
        }
    }
}

impl<W> Node<W> {
    fn map<V>(self, window: &impl Fn(W) -> V) -> Node<V> {
        let child = |node: Box<Node<W>>| Box::new(node.map(window));
        match self {
            Node::Split { horizontal, ratio, children } => Node::Split {
                horizontal,
                ratio,
                children: match children {
                    Children::One(only) => Children::One(child(only)),
                    Children::Two(first, second) => Children::Two(child(first), child(second)),
                },
            },
            Node::Leaf { slot, fill, stacked } => Node::Leaf {
                slot: match slot {
                    Slot::Empty => Slot::Empty,
                    Slot::Window { window: id, constraints } => Slot::Window {
                        window: window(id),
                        constraints,
                    },
                },
                fill,
                stacked,
            },
        }
    }
}

impl<W: Copy> Bsp<W> {
    /// Each window's frame, in tree order.
    pub fn frames(&self) -> Vec<(W, Rect)> {
        let root = &self.root;
        let area = tiling_area(self.screen, self.gaps);
        let mut out = Vec::new();
        self.place(&self.summarized(root), area, &mut out);
        if let (Some(ratio), [(_, frame)]) = (self.aspect, out.as_mut_slice())
            && let Node::Leaf {
                fill: Fill::Tiled,
                stacked: false,
                ..
            } = root
        {
            let (w, h) = if frame.w > frame.h * ratio {
                (frame.h * ratio, frame.h)
            } else {
                (frame.w, frame.w / ratio)
            };
            *frame = Rect {
                x: area.x + (area.w - w) / 2.0,
                y: area.y + (area.h - h) / 2.0,
                w,
                h,
            };
        }
        out
    }

    /// The tree with each subtree's constraints along both axes, folded once bottom-up (a
    /// split asks each child's along its own axis; recomputing them at every level made a deep
    /// tree's frames quadratic).
    fn summarized<'a>(&self, node: &'a Node<W>) -> Summarized<'a, W> {
        match node {
            Node::Leaf { slot, fill, .. } => Summarized {
                along: Along::both(|horizontal| leaf_summary(slot, horizontal)),
                node: Annotated::Leaf { slot, fill: *fill },
            },
            Node::Split {
                horizontal: split,
                ratio,
                children,
            } => {
                let children = match children {
                    Children::One(child) => {
                        SummarizedChildren::One(Box::new(self.summarized(child)))
                    }
                    Children::Two(first, second) => SummarizedChildren::Two(
                        Box::new(self.summarized(first)),
                        Box::new(self.summarized(second)),
                    ),
                };
                Summarized {
                    along: Along::both(|horizontal| {
                        self.split_summary(*split, children.alongs(), horizontal)
                    }),
                    node: Annotated::Split {
                        horizontal: *split,
                        ratio: *ratio,
                        children,
                    },
                }
            }
        }
    }

    fn place(&self, node: &Summarized<'_, W>, rect: Rect, out: &mut Vec<(W, Rect)>) {
        match &node.node {
            Annotated::Leaf { slot: Slot::Empty, .. } => {}
            Annotated::Leaf {
                slot: Slot::Window { window, constraints },
                fill,
            } => {
                let target = match fill {
                    Fill::Screen => self.screen,
                    Fill::WithinGaps => tiling_area(self.screen, self.gaps),
                    Fill::Tiled => match constraints {
                        Some(constraints) => fit(rect, constraints.normalized()),
                        None => rect,
                    },
                };
                out.push((*window, target));
            }
            Annotated::Split {
                children: SummarizedChildren::One(child),
                ..
            } => self.place(child, rect, out),
            Annotated::Split {
                horizontal,
                ratio,
                children: SummarizedChildren::Two(first, second),
            } => {
                let horizontal = *horizontal;
                let (total, gap) = if horizontal {
                    (rect.w, self.gaps.horizontal)
                } else {
                    (rect.h, self.gaps.vertical)
                };
                let available = (total - gap).max(0.0);
                let ratio = f64::from(*ratio);
                let axis = |node: &Summarized<'_, W>, weight: f64| {
                    let (min, fixed, max, can_grow) = node.along.on(horizontal);
                    AxisConstraints {
                        min,
                        fixed,
                        max,
                        weight: weight.max(0.0),
                        can_grow,
                    }
                };
                let solved =
                    solve_axis_lengths(&[axis(first, ratio), axis(second, 1.0 - ratio)], available);
                let (first_len, second_len) = (solved[0], solved[1]);
                let (mut r1, mut r2) = (rect, rect);
                if horizontal {
                    r1.w = first_len;
                    r2.x += first_len + gap;
                    r2.w = second_len;
                } else {
                    r1.h = first_len;
                    r2.y += first_len + gap;
                    r2.h = second_len;
                }
                self.place(first, r1, out);
                self.place(second, r2, out);
            }
        }
    }

    /// A split's constraints along one axis, from its children's.
    fn split_summary(
        &self,
        split: bool,
        children: impl Iterator<Item = Along>,
        horizontal: bool,
    ) -> AxisSummary {
        let gap = if horizontal {
            self.gaps.horizontal
        } else {
            self.gaps.vertical
        };
        let mut summary = container_axis_constraints(
            children.map(|along| along.on(horizontal)),
            split == horizontal,
            false,
            gap,
            0.0,
            0.0,
        );
        // BSP has always left the perpendicular maximum unconstrained.
        if split != horizontal {
            summary.2 = None;
        }
        summary
    }
}

/// A leaf's constraints along one axis.
fn leaf_summary<W>(slot: &Slot<W>, horizontal: bool) -> AxisSummary {
    match slot {
        Slot::Window {
            constraints: Some(constraints), ..
        } => {
            let c = constraints.normalized();
            (
                c.reported_min(horizontal),
                c.fixed_for_axis(horizontal),
                (c.max_for_axis(horizontal) > 0.0).then(|| c.max_for_axis(horizontal)),
                c.resizable_for_axis(horizontal),
            )
        }
        Slot::Window { constraints: None, .. } | Slot::Empty => (0.0, None, None, true),
    }
}

/// A node of a [`Bsp`] with its subtree's constraints along both axes, made for one `frames`
/// and dropped with it.
struct Summarized<'a, W> {
    along: Along,
    node: Annotated<'a, W>,
}

enum Annotated<'a, W> {
    Leaf {
        slot: &'a Slot<W>,
        fill: Fill,
    },
    Split {
        horizontal: bool,
        ratio: f32,
        children: SummarizedChildren<'a, W>,
    },
}

enum SummarizedChildren<'a, W> {
    One(Box<Summarized<'a, W>>),
    Two(Box<Summarized<'a, W>>, Box<Summarized<'a, W>>),
}

impl<W> SummarizedChildren<'_, W> {
    fn alongs(&self) -> impl Iterator<Item = Along> + '_ {
        let (first, second) = match self {
            Self::One(only) => (only, None),
            Self::Two(first, second) => (first, Some(second)),
        };
        std::iter::once(first.along).chain(second.map(|s| s.along))
    }
}

/// A tiled window's frame inside its area: as large as the area, within its constraints.
fn fit(mut target: Rect, c: WindowConstraints) -> Rect {
    let desired_w = c.fixed_for_axis(true).unwrap_or(target.w).max(c.reported_min(true));
    let desired_h = c.fixed_for_axis(false).unwrap_or(target.h).max(c.reported_min(false));
    let desired_w = if c.max_for_axis(true) > 0.0 {
        desired_w.min(c.max_for_axis(true))
    } else {
        desired_w
    };
    let desired_h = if c.max_for_axis(false) > 0.0 {
        desired_h.min(c.max_for_axis(false))
    } else {
        desired_h
    };
    target.w = desired_w.min(target.w).max(0.0);
    target.h = desired_h.min(target.h).max(0.0);
    target
}
