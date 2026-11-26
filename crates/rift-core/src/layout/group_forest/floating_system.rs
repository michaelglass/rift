//! The floating layout system on the group forest (upstream's `FloatingLayoutSystem`): the
//! forest, which holds the windows and the groups the user joins or stacks them into, and the
//! frame the layout keeps for each window. The frames the layout places come from
//! [`crate::layout::floating`]; this is the bookkeeping that feeds it.
//!
//! The settings (where a new window goes, whether a layout's first frames spread) are passed to
//! the operations that read them, not held here.

use std::collections::BTreeMap;
use std::ops::ControlFlow;

use super::{
    Direction, Edit, Edited, Focus, Focused, Forest, GroupBar, InsertionPoint, Kind, Orientation,
    Participation, Placement, Query,
};
use crate::layout::floating::{self, Newcomer, Seen, Slot};
use crate::layout::{Gaps, WindowConstraints};
use crate::plain::Rect;
use crate::slot_table::SlotKey;

/// The frame a floating layout keeps for a window. A window with no entry has none yet (its
/// first frame is computed when the layout places it).
///
/// The states upstream's pair of optional frames (`frame`, `restore`) reaches:
/// - [`Geometry::Kept`]: a frame, the window on its own.
/// - [`Geometry::Aligned`]: a frame shared with the group it was aligned to (a stack, a join into
///   a stack), and the window's own frame from before, to return to when it leaves.
/// - [`Geometry::Restorable`]: no frame yet, and a frame to return to. Upstream reaches it only by
///   loading a file of its older form (`independent_frames` without `frames`); every operation
///   keeps it closed (it becomes `Aligned` or `Kept`, or goes).
///
/// Not a state: neither frame (upstream's entry with both `None`, which every reader takes as no
/// entry); it is no entry here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Geometry {
    Kept(Rect),
    Aligned { frame: Rect, restore: Rect },
    Restorable(Rect),
}

impl Geometry {
    /// The frame the layout keeps, if any.
    pub fn frame(self) -> Option<Rect> {
        match self {
            Geometry::Kept(frame) | Geometry::Aligned { frame, .. } => Some(frame),
            Geometry::Restorable(_) => None,
        }
    }

    /// The frame to return to, if any.
    pub fn restore(self) -> Option<Rect> {
        match self {
            Geometry::Aligned { restore, .. } | Geometry::Restorable(restore) => Some(restore),
            Geometry::Kept(_) => None,
        }
    }

    /// Upstream's pair as it would hold it (its saved form; `None` for no entry).
    pub fn from_pair(frame: Option<Rect>, restore: Option<Rect>) -> Option<Geometry> {
        match (frame, restore) {
            (Some(frame), Some(restore)) => Some(Geometry::Aligned { frame, restore }),
            (Some(frame), None) => Some(Geometry::Kept(frame)),
            (None, Some(restore)) => Some(Geometry::Restorable(restore)),
            (None, None) => None,
        }
    }

    /// `frame` kept, the frame to return to kept with it.
    fn with_frame(geometry: Option<Geometry>, frame: Rect) -> Geometry {
        match geometry {
            Some(Geometry::Aligned { restore, .. } | Geometry::Restorable(restore)) => {
                Geometry::Aligned { frame, restore }
            }
            Some(Geometry::Kept(_)) | None => Geometry::Kept(frame),
        }
    }

    /// The frame to return to, back as the frame (upstream's `restore_frame`).
    fn restored(self) -> Geometry {
        match self {
            Geometry::Aligned { restore, .. } | Geometry::Restorable(restore) => {
                Geometry::Kept(restore)
            }
            Geometry::Kept(frame) => Geometry::Kept(frame),
        }
    }

    /// The frame to return to forgotten (upstream's `restore = None`).
    fn forgetting(self) -> Option<Geometry> {
        match self {
            Geometry::Kept(frame) | Geometry::Aligned { frame, .. } => Some(Geometry::Kept(frame)),
            Geometry::Restorable(_) => None,
        }
    }

    /// Aligned to `anchor`'s frame (upstream's `align_members` for one window): the frame
    /// becomes the anchor's (kept when the anchor has none), and with `save` the window's own
    /// frame is remembered unless one is already.
    fn aligned(geometry: Option<Geometry>, anchor: Option<Rect>, save: bool) -> Option<Geometry> {
        match (geometry, anchor) {
            (None, None) => None,
            (None, Some(anchor)) => Some(Geometry::Kept(anchor)),
            (Some(Geometry::Kept(own)), _) => {
                let frame = anchor.unwrap_or(own);
                Some(if save {
                    Geometry::Aligned { frame, restore: own }
                } else {
                    Geometry::Kept(frame)
                })
            }
            (Some(Geometry::Aligned { frame, restore }), _) => Some(Geometry::Aligned {
                frame: anchor.unwrap_or(frame),
                restore,
            }),
            (Some(Geometry::Restorable(restore)), Some(frame)) => {
                Some(Geometry::Aligned { frame, restore })
            }
            (Some(Geometry::Restorable(restore)), None) => Some(Geometry::Restorable(restore)),
        }
    }

    /// Out of a stack being unstacked (upstream's `unstack_parent_of_selection`): the frame to
    /// return to forgotten, and the frame back at it unless the window is the focused one.
    fn unstacked(self, focused: bool) -> Option<Geometry> {
        match (self, focused) {
            (Geometry::Kept(frame), _) => Some(Geometry::Kept(frame)),
            (Geometry::Aligned { frame, .. }, true) => Some(Geometry::Kept(frame)),
            (Geometry::Aligned { restore, .. } | Geometry::Restorable(restore), false) => {
                Some(Geometry::Kept(restore))
            }
            (Geometry::Restorable(_), true) => None,
        }
    }
}

/// The order a floating layout's visible windows are compared in when two are equally near
/// ([`FloatingSystem::directional_window`]): the forest's visible windows in tree order, a stack
/// contributing its chosen child (`visit_visible_windows`), the first of equals winning.
fn visible_order<L: SlotKey + Eq, N: SlotKey + Eq + Ord, W: Copy + Ord>(
    forest: &Forest<L, N, W>,
    root: N,
) -> Vec<(N, W)> {
    let mut windows = Vec::new();
    let _ = forest.visit_visible_windows(root, &mut |node, window| {
        windows.push((node, window));
        ControlFlow::<()>::Continue(())
    });
    windows
}

/// How near a candidate frame is from a source frame toward `direction`: whether it lies
/// outside the source's band across the direction, and its squared distance between centres.
/// None when it is not ahead (upstream's `distance_in_direction` on the centres).
fn nearness(source: Rect, candidate: Rect, direction: Direction) -> Option<(bool, f64)> {
    let mid = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let (origin, target) = (mid(source), mid(candidate));
    let horizontal = matches!(direction, Direction::Left | Direction::Right);
    let (across, overlaps) = if horizontal {
        (
            target.1 - origin.1,
            source.y < candidate.y + candidate.h && candidate.y < source.y + source.h,
        )
    } else {
        (
            target.0 - origin.0,
            source.x < candidate.x + candidate.w && candidate.x < source.x + source.w,
        )
    };
    let along = match direction {
        Direction::Left => origin.0 - target.0,
        Direction::Right => target.0 - origin.0,
        Direction::Up => origin.1 - target.1,
        Direction::Down => target.1 - origin.1,
    };
    (along > 0.0).then_some((!overlaps, along * along + across * across))
}

/// The floating layout system (upstream's `FloatingLayoutSystem` without its settings).
#[derive(Debug, Clone, PartialEq)]
pub struct FloatingSystem<L, N, W> {
    forest: Forest<L, N, W>,
    geometry: BTreeMap<W, Geometry>,
}

impl<L: SlotKey + Eq, N: SlotKey + Eq + Ord, W: Copy + Ord> Default for FloatingSystem<L, N, W> {
    fn default() -> Self { Self::from_parts(Forest::default(), BTreeMap::new()) }
}

impl<L: SlotKey + Eq, N: SlotKey + Eq + Ord, W: Copy + Ord> FloatingSystem<L, N, W> {
    pub fn from_parts(forest: Forest<L, N, W>, geometry: BTreeMap<W, Geometry>) -> Self {
        Self { forest, geometry }
    }

    pub fn forest(&self) -> &Forest<L, N, W> { &self.forest }

    pub fn geometry(&self) -> &BTreeMap<W, Geometry> { &self.geometry }

    /// The frame the layout keeps for `window`, if any.
    pub fn frame(&self, window: W) -> Option<Rect> { self.geometry.get(&window)?.frame() }

    pub fn set_window_participation(&mut self, window: W, participation: Participation) -> bool {
        self.forest.set_window_participation(window, participation)
    }

    pub fn create_layout(&mut self) -> L { self.forest.create_layout() }

    pub fn clone_layout(&mut self, layout: L) -> L { self.forest.clone_layout(layout) }

    /// The layout gone, and the frames of the windows no other layout holds.
    pub fn remove_layout(&mut self, layout: L) {
        let windows = self.forest.windows(layout, Query::All);
        self.forest.remove_layout(layout);
        let held: Vec<W> = self.forest.index().into_iter().map(|(window, ..)| window).collect();
        for window in windows {
            if !held.contains(&window) {
                self.geometry.remove(&window);
            }
        }
    }

    /// A new window: after the selection's branch of the root when inserting next to the
    /// selection, else at the root's back; selected. New windows stay on their own until joined.
    pub fn add_window(&mut self, layout: L, window: W, insertion: InsertionPoint) {
        let f = &mut self.forest;
        if f.contains_window(layout, window) {
            return;
        }
        let root = f.root(layout);
        let anchor = match insertion {
            InsertionPoint::NextToSelection => {
                let selection = f.selection(layout);
                f.ancestors(selection).find(|&node| f.parent(node) == Some(root))
            }
            InsertionPoint::EndOfTree => None,
        };
        let node = match anchor {
            Some(anchor) => {
                let node = f.insert(Placement::After(anchor));
                f.set_window(node, window);
                node
            }
            None => f.add_window_under(root, window),
        };
        f.select_node(node);
    }

    pub fn remove_window(&mut self, window: W) {
        self.geometry.remove(&window);
        self.forest.remove_window(window);
    }

    /// `from` becomes `to`: its frames go with it (`to`'s own are dropped).
    pub fn replace_window(&mut self, from: W, to: W) {
        if from == to {
            return;
        }
        let geometry = self.geometry.remove(&from);
        self.geometry.remove(&to);
        if let Some(geometry) = geometry {
            self.geometry.insert(to, geometry);
        }
        self.forest.replace_window(from, to);
    }

    pub fn close_app(&mut self, is_app: impl Fn(W) -> bool) {
        self.geometry.retain(|&window, _| !is_app(window));
        self.forest.close_app(is_app);
    }

    /// The user moved or resized `window` to `frame`: kept when one of its leaves is visible,
    /// and shared by every member of the outermost stack around each of its leaves.
    pub fn store_frame(&mut self, window: W, frame: Rect) {
        let leaves: Vec<N> =
            self.forest.occurrences(window).into_iter().map(|(_, node)| node).collect();
        if !leaves.iter().any(|&node| self.node_is_visible(node)) {
            return;
        }
        self.keep_frame(window, frame);
        for node in leaves {
            let f = &self.forest;
            let outermost = f.ancestors(node).filter(|&above| f.is_stacked(above)).last();
            if let Some(stack) = outermost {
                let members: Vec<W> = f.windows_from(stack, Some(stack), true).collect();
                for member in members {
                    self.keep_frame(member, frame);
                }
            }
        }
    }

    fn keep_frame(&mut self, window: W, frame: Rect) {
        let geometry = Geometry::with_frame(self.geometry.get(&window).copied(), frame);
        self.geometry.insert(window, geometry);
    }

    fn set_geometry(&mut self, window: W, geometry: Option<Geometry>) {
        match geometry {
            Some(geometry) => self.geometry.insert(window, geometry),
            None => self.geometry.remove(&window),
        };
    }

    /// The first frame of each of the layout's windows without one (its live frame, no larger
    /// than the default, or the default), cascaded by its position among all the layout's
    /// windows when `floating.spread`.
    pub fn initialize_frames(
        &mut self,
        layout: L,
        floating: floating::Floating,
        constraints: impl Fn(W) -> WindowConstraints,
        live: impl Fn(W) -> Option<Rect>,
    ) {
        let windows: Vec<W> = self.forest.windows_with_nodes(layout).map(|(_, w)| w).collect();
        let count = windows.len();
        for (position, &window) in windows.iter().enumerate() {
            if self.frame(window).is_some() {
                continue;
            }
            let newcomer = Newcomer {
                constraints: constraints(window),
                seen: live(window).map_or(Seen::Unseen, Seen::Live),
            };
            self.keep_frame(window, floating.first_frame(newcomer, position, count));
        }
    }

    /// A layout as the floating geometry takes it: every window in the layout's order (one left
    /// out holds its position), on the screen or in the tiling area when it or an ancestor
    /// fills, else at its kept frame, else at a first frame.
    pub fn shape(
        &self,
        layout: L,
        floating: floating::Floating,
        gaps: Gaps,
        constraints: impl Fn(W) -> WindowConstraints,
    ) -> floating::Layout<W> {
        let f = &self.forest;
        let windows = f
            .windows_with_nodes(layout)
            .map(|(node, window)| {
                if !f.window_participates(window) {
                    return Slot::LeftOut;
                }
                let place = self
                    .fill_place(node)
                    .or_else(|| self.frame(window).map(floating::Place::Kept))
                    .unwrap_or_else(|| floating::Place::First(constraints(window)));
                Slot::In { window, place }
            })
            .collect();
        floating::Layout { floating, gaps, windows }
    }

    /// The screen or the tiling area, when `node` or an ancestor fills.
    fn fill_place(&self, node: N) -> Option<floating::Place> {
        let f = &self.forest;
        f.ancestors(node).find_map(|above| match f.nodes[above].fill {
            super::Fill::Screen => Some(floating::Place::Screen),
            super::Fill::WithinGaps => Some(floating::Place::WithinGaps),
            super::Fill::Tiled => None,
        })
    }

    /// The group bar of each stack (along the selection path, with `selection_path_only`) whose
    /// shown window does not fill: at the shown window's frame, or the default frame.
    pub fn group_bars(
        &self,
        layout: L,
        floating: floating::Floating,
        selection_path_only: bool,
    ) -> Vec<GroupBar<N, W>> {
        let f = &self.forest;
        let selection = f.selection(layout);
        f.preorder(f.root(layout))
            .filter_map(|node| {
                let Kind::Stack(orientation) = f.kind(node)? else {
                    return None;
                };
                if selection_path_only && !f.ancestors(selection).any(|above| above == node) {
                    return None;
                }
                let shown = self.visible_stack_child(node)?;
                let (leaf, window) = self.first_visible_window(shown)?;
                if self.fill_place(leaf).is_some() {
                    return None;
                }
                let frame = self
                    .frame(window)
                    .unwrap_or_else(|| floating.default_frame(WindowConstraints::default()));
                let children = f.children(node).to_vec();
                Some(f.group_bar(node, orientation, frame, &children, shown))
            })
            .collect()
    }

    /// The windows a cycle goes through: the focused stack's members taking part, else the
    /// layout's visible windows.
    pub fn cycle_windows(&self, layout: L) -> Vec<W> {
        let f = &self.forest;
        if let Some((node, _)) = f.focused_leaf(layout)
            && let Some(parent) = f.parent(node)
            && f.is_stacked(parent)
        {
            return f
                .windows_from(parent, Some(parent), true)
                .filter(|&window| f.window_participates(window))
                .collect();
        }
        f.windows(layout, Query::Visible)
    }

    fn first_visible_window(&self, node: N) -> Option<(N, W)> {
        self.forest
            .visit_visible_windows(node, &mut |node, window| ControlFlow::Break((node, window)))
            .break_value()
    }

    fn visible_stack_child(&self, node: N) -> Option<N> {
        self.forest
            .participating_child(node, false, |child| self.first_visible_window(child).is_some())
    }

    fn node_is_visible(&self, node: N) -> bool {
        let f = &self.forest;
        if !f.window(node).is_some_and(|window| f.window_participates(window)) {
            return false;
        }
        let mut child = node;
        while let Some(parent) = f.parent(child) {
            if f.is_stacked(parent) && self.visible_stack_child(parent) != Some(child) {
                return false;
            }
            child = parent;
        }
        true
    }

    /// The members of a group: its windows in preorder.
    fn members(&self, container: N) -> Vec<W> {
        let f = &self.forest;
        f.preorder(container).filter_map(|node| f.window(node)).collect()
    }

    /// The group the selection is in (or is), when it is not the root.
    fn selected_container(&self, layout: L) -> Option<N> {
        let f = &self.forest;
        let selection = f.selection(layout);
        let container = if f.window(selection).is_some() {
            f.parent(selection)?
        } else {
            selection
        };
        (container != f.root(layout)).then_some(container)
    }

    /// The leaf left alone in `moving`'s group once `moving` leaves it, if that is what stays.
    fn remaining_leaf(&self, moving: N) -> Option<N> {
        let f = &self.forest;
        let mut siblings =
            f.children(f.parent(moving)?).iter().copied().filter(|&node| node != moving);
        siblings
            .next()
            .filter(|&node| siblings.next().is_none() && f.window(node).is_some())
    }

    /// A leaf left on its own at the root takes back its own frame.
    fn restore_if_independent(&mut self, layout: L, remaining: Option<N>) {
        let f = &self.forest;
        if let Some(node) = remaining
            && f.parent(node) == Some(f.root(layout))
            && let Some(window) = f.window(node)
        {
            self.restore_frame(window);
        }
    }

    fn restore_frame(&mut self, window: W) {
        if let Some(geometry) = self.geometry.get(&window).copied() {
            self.geometry.insert(window, geometry.restored());
        }
    }

    fn align_members(&mut self, members: &[W], anchor: W, save: bool) {
        let frame = self.frame(anchor);
        for &window in members {
            let aligned = Geometry::aligned(self.geometry.get(&window).copied(), frame, save);
            self.set_geometry(window, aligned);
        }
    }

    /// The nearest visible window toward `direction` from `focused`'s frame, not inside
    /// `exclude`: one overlapping the focused frame's band across the direction before one
    /// outside it, then the nearest centre, then the first in [`visible_order`].
    fn directional_window(
        &self,
        layout: L,
        focused: W,
        direction: Direction,
        exclude: Option<N>,
    ) -> Option<(N, W)> {
        let source = self.frame(focused)?;
        let f = &self.forest;
        let mut best: Option<((N, W), (bool, f64))> = None;
        for (node, window) in visible_order(f, f.root(layout)) {
            if window == focused || exclude.is_some_and(|group| f.is_within(node, group)) {
                continue;
            }
            let Some(frame) = self.frame(window) else { continue };
            let Some((outside, distance)) = nearness(source, frame, direction) else {
                continue;
            };
            let nearer = best.is_none_or(|(_, (best_outside, best_distance))| {
                outside.cmp(&best_outside).then(distance.total_cmp(&best_distance)).is_lt()
            });
            if nearer {
                best = Some(((node, window), (outside, distance)));
            }
        }
        best.map(|(target, _)| target)
    }

    /// Where a directional focus move goes: along a stack's axis, its next member; else the
    /// nearest window that way; else the next member, then around the stack.
    pub fn window_in_direction(&self, layout: L, direction: Direction) -> Option<W> {
        let f = &self.forest;
        let (node, focused) = f.focused_leaf(layout)?;
        let parent = f.parent(node)?;
        let kind = f.container_kind(parent);
        let forward = matches!(direction, Direction::Right | Direction::Down);
        let stack_target = if kind.is_stacked() {
            let next = f.step(node, parent, forward);
            f.windows_from(parent, next, forward).next()
        } else {
            None
        };
        if kind.orientation() == orientation_of(direction) && stack_target.is_some() {
            return stack_target;
        }
        self.directional_window(layout, focused, direction, None)
            .map(|(_, window)| window)
            .or(stack_target)
            .or_else(|| {
                if !kind.is_stacked() {
                    return None;
                }
                let start = if forward {
                    parent
                } else {
                    f.last_descendant(parent)
                };
                f.windows_from(parent, Some(start), forward).next()
            })
    }

    pub fn focus(&mut self, layout: L, focus: Focus<W>) -> Focused<W> {
        match focus {
            Focus::Direction(direction) => {
                let target = self.window_in_direction(layout, direction);
                if let Some(window) = target {
                    self.forest.focus(layout, Focus::Select(window));
                }
                Focused::Movement {
                    window: target,
                    raise: target.into_iter().collect(),
                }
            }
            other => self.forest.focus(layout, other),
        }
    }

    pub fn edit(&mut self, layout: L, edit: Edit<W>) -> Edited<W> {
        let edited = |changed| {
            if changed {
                Edited::Changed
            } else {
                Edited::Unchanged
            }
        };
        match edit {
            Edit::Move { .. } => Edited::Unchanged,
            Edit::Fullscreen { within_gaps } => {
                Edited::Raise(self.forest.toggle_fullscreen(layout, within_gaps))
            }
            Edit::Swap(a, b) => edited(self.forest.swap_windows(layout, a, b)),
            Edit::ReorderTab { window, target } => {
                edited(self.forest.move_stack_tab(layout, window, target))
            }
            Edit::ToggleOrientation => self.forest.toggle_orientation(layout),
            Edit::Join(direction) => edited(self.join_selection_with_direction(layout, direction)),
            Edit::ConsumeOrExpel(direction) => {
                edited(if self.selected_container(layout).is_some() {
                    self.unjoin_selection(layout)
                } else {
                    self.join_selection_with_direction(layout, direction)
                })
            }
            Edit::Stack(default) => Edited::Raise(self.stack(layout, default)),
            Edit::Unstack(default) => Edited::Raise(self.unstack(layout, default)),
            Edit::Unjoin => edited(self.unjoin_selection(layout)),
            Edit::Split(_) | Edit::Drop { .. } | Edit::Resize { .. } | Edit::Rebalance => {
                Edited::Unsupported
            }
        }
    }

    /// The selected group made a stack, its members sharing the focused window's frame (each
    /// remembering its own unless the group was a stack already).
    fn stack(&mut self, layout: L, default: super::DefaultOrientation) -> Vec<W> {
        let Some(focused) = self.forest.selected_window(layout) else {
            return Vec::new();
        };
        // Only a group joined explicitly can become a floating stack.
        let Some(container) = self.selected_container(layout) else {
            return Vec::new();
        };
        let members = self.members(container);
        let save = !self.forest.is_stacked(container);
        self.align_members(&members, focused, save);
        self.forest.apply_stacking(layout, default);
        self.forest.focus(layout, Focus::Select(focused));
        self.forest.windows(layout, Query::Selection)
    }

    /// The stack around the selection made a split: its members take back their own frames,
    /// but for the focused one, which keeps the stack's.
    fn unstack(&mut self, layout: L, default: super::DefaultOrientation) -> Vec<W> {
        let f = &self.forest;
        let focused = f.selected_window(layout);
        let selection = f.selection(layout);
        let Some(container) = f.ancestors(selection).find(|&node| f.is_stacked(node)) else {
            return Vec::new();
        };
        let visible = self.forest.unstack(layout, default);
        let members: Vec<W> = self.forest.windows_from(container, Some(container), true).collect();
        for window in members {
            if let Some(geometry) = self.geometry.get(&window).copied() {
                self.set_geometry(window, geometry.unstacked(Some(window) == focused));
            }
        }
        if let Some(window) = focused {
            self.forest.focus(layout, Focus::Select(window));
        }
        visible
    }

    /// The focused window joined with the nearest window toward `direction` (one outside its
    /// own group): a window at the root gets a new group along the direction; the joined window
    /// takes the group's frame when the group is a stack, else forgets its own.
    fn join_selection_with_direction(&mut self, layout: L, direction: Direction) -> bool {
        let Some((selection, focused)) = self.forest.focused_leaf(layout) else {
            return false;
        };
        let root = self.forest.root(layout);
        let parent = self.forest.parent(selection).unwrap_or(root);
        let exclude = (parent != root).then_some(parent);
        let Some((target_node, target)) =
            self.directional_window(layout, focused, direction, exclude)
        else {
            return false;
        };
        let container = if parent == root {
            let kind = Kind::Split(orientation_of(direction));
            self.forest.nest_in_container(layout, selection, kind)
        } else {
            parent
        };
        let remaining = self.remaining_leaf(target_node);
        if self.forest.is_stacked(container) {
            self.align_members(&[target], focused, true);
        } else if let Some(geometry) = self.geometry.get(&target).copied() {
            self.set_geometry(target, geometry.forgetting());
        }
        self.forest.move_node(target_node, Placement::Back(container));
        self.restore_if_independent(layout, remaining);
        self.forest.select_node(selection);
        true
    }

    /// The focused window out of its group, after it, with its own frame back.
    fn unjoin_selection(&mut self, layout: L) -> bool {
        let Some((selection, window)) = self.forest.focused_leaf(layout) else {
            return false;
        };
        let Some(parent) = self.forest.parent(selection) else {
            return false;
        };
        if parent == self.forest.root(layout) {
            return false;
        }
        let remaining = self.remaining_leaf(selection);
        // Only the focused member leaves; the rest of the group stays.
        self.forest.move_node(selection, Placement::After(parent));
        self.restore_frame(window);
        self.restore_if_independent(layout, remaining);
        self.forest.select_node(selection);
        true
    }
}

fn orientation_of(direction: Direction) -> Orientation {
    match direction {
        Direction::Left | Direction::Right => Orientation::Horizontal,
        Direction::Up | Direction::Down => Orientation::Vertical,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::group_forest::DefaultOrientation;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    struct Key(u32, u32);

    impl SlotKey for Key {
        fn from_parts(index: u32, version: u32) -> Self { Key(index, version) }

        fn parts(self) -> (u32, u32) { (self.0, self.1) }
    }

    type System = FloatingSystem<Key, Key, u32>;

    fn rect(x: f64, y: f64) -> Rect { Rect { x, y, w: 200., h: 150. } }

    const SCREEN: Rect = Rect {
        x: 0.,
        y: 0.,
        w: 1000.,
        h: 800.,
    };

    /// Four windows in a row, 250 apart, each on its own; the first selected.
    fn fixture() -> (System, Key) {
        let mut system = System::default();
        let layout = system.create_layout();
        for index in 1..=4 {
            system.add_window(layout, index, InsertionPoint::NextToSelection);
            system.store_frame(index, rect(20. + f64::from(index - 1) * 250., 40.));
        }
        system.focus(layout, Focus::Select(1));
        (system, layout)
    }

    fn stack(system: &mut System, layout: Key) {
        system.edit(layout, Edit::Stack(DefaultOrientation::Perpendicular));
    }

    fn join(system: &mut System, layout: Key, direction: Direction) {
        system.edit(layout, Edit::Join(direction));
    }

    fn moved(system: &mut System, layout: Key, direction: Direction) -> Option<u32> {
        match system.focus(layout, Focus::Direction(direction)) {
            Focused::Movement { window, .. } => window,
            Focused::Selection { .. } => panic!("a directional focus is a movement"),
        }
    }

    /// Upstream's `a_new_floating_window_takes_most_of_the_screen_centred`: a window with no
    /// frame yet takes a first frame, one with a frame keeps it.
    #[test]
    fn a_new_floating_window_takes_most_of_the_screen_centred() {
        let mut system = System::default();
        let layout = system.create_layout();
        system.add_window(layout, 1, InsertionPoint::NextToSelection);
        system.add_window(layout, 2, InsertionPoint::NextToSelection);
        system.store_frame(2, rect(20., 40.));
        let floating = floating::Floating { screen: SCREEN, spread: false };
        let shape = system.shape(layout, floating, Gaps::default(), |_| Default::default());
        assert_eq!(shape.frames(), vec![
            (1, Rect {
                x: 150.,
                y: 120.,
                w: 700.,
                h: 560.
            }),
            (2, rect(20., 40.)),
        ]);
    }

    /// Upstream's `suspended_stack_selection_leaves_the_visible_tab_interactive`.
    #[test]
    fn a_suspended_stack_selection_leaves_the_visible_tab_interactive() {
        let (mut system, layout) = fixture();
        join(&mut system, layout, Direction::Right);
        stack(&mut system, layout);
        system.set_window_participation(1, Participation::LeftOut);
        assert!(system.forest.windows(layout, Query::Visible).contains(&2));
        let moved_to = rect(100., 80.);
        system.store_frame(2, moved_to);
        assert_eq!(system.frame(2), Some(moved_to));
        let floating = floating::Floating { screen: SCREEN, spread: false };
        let bars = system.group_bars(layout, floating, false);
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].windows, [2]);
        assert_eq!(bars[0].frame, moved_to);
    }

    /// Upstream's `explicit_stacks_share_geometry_and_restore_only_their_members`.
    #[test]
    fn explicit_stacks_share_geometry_and_restore_only_their_members() {
        let (mut system, layout) = fixture();
        stack(&mut system, layout);
        assert_eq!(system.forest.windows(layout, Query::Visible).len(), 4);
        join(&mut system, layout, Direction::Right);
        assert_ne!(system.frame(1), system.frame(2));
        stack(&mut system, layout);
        join(&mut system, layout, Direction::Right);
        assert_eq!(system.cycle_windows(layout), vec![1, 2, 3]);
        system.add_window(layout, 5, InsertionPoint::NextToSelection);
        system.add_window(layout, 6, InsertionPoint::NextToSelection);
        assert_eq!(system.forest.windows(layout, Query::Visible).len(), 4);
        system.focus(layout, Focus::Select(2));
        system.store_frame(2, rect(100., 80.));
        system.store_frame(1, rect(999., 999.));
        assert_eq!(system.frame(1), Some(rect(100., 80.)));
        // A frame to return to on a window outside the stack: only the stack's members restore.
        system.geometry.insert(4, Geometry::Aligned {
            frame: rect(770., 40.),
            restore: rect(999., 999.),
        });
        system.edit(layout, Edit::Unjoin);
        assert_eq!(system.frame(2), Some(rect(270., 40.)));
        assert_eq!(system.frame(4), Some(rect(770., 40.)));
        system.focus(layout, Focus::Select(1));
        system.edit(layout, Edit::Unstack(DefaultOrientation::Perpendicular));
        assert_eq!(system.frame(1), Some(rect(100., 80.)));
        assert_eq!(system.frame(3), Some(rect(520., 40.)));
    }

    /// Upstream's `directional_focus_reaches_hidden_members_and_neighboring_windows`.
    #[test]
    fn directional_focus_reaches_hidden_members_and_neighbouring_windows() {
        let (mut system, layout) = fixture();
        join(&mut system, layout, Direction::Right);
        assert_eq!(system.focus(layout, Focus::Ascend), Focused::Selection {
            handled: true
        });
        stack(&mut system, layout);
        assert_eq!(system.forest.selected_window(layout), Some(1));
        system.store_frame(3, rect(20., 400.));
        for (direction, expected) in [
            (Direction::Down, 2),
            (Direction::Down, 3),
            (Direction::Up, 2),
            (Direction::Up, 1),
        ] {
            assert_eq!(moved(&mut system, layout, direction), Some(expected));
        }
        system.remove_window(3);
        system.remove_window(4);
        assert_eq!(moved(&mut system, layout, Direction::Right), Some(2));
        assert_eq!(moved(&mut system, layout, Direction::Right), Some(1));
        system.edit(layout, Edit::Unjoin);
        assert_eq!(system.forest.windows(layout, Query::All).len(), 2);
        assert!(!system.forest.parent_of_selection_is_stacked(layout));
    }

    /// Upstream's `saved_stack_frames_survive_identity_changes_and_layout_clones` (its save and
    /// load are the platform's; the frames survive a replace and a clone here).
    #[test]
    fn stack_frames_survive_identity_changes_and_layout_clones() {
        let (mut system, layout) = fixture();
        join(&mut system, layout, Direction::Right);
        stack(&mut system, layout);
        system.focus(layout, Focus::Select(3));
        join(&mut system, layout, Direction::Right);
        stack(&mut system, layout);
        system.focus(layout, Focus::Select(1));
        join(&mut system, layout, Direction::Right);
        assert_eq!(system.cycle_windows(layout), vec![1, 2, 3]);
        assert_eq!(system.frame(4), Some(rect(770., 40.)));
        let before = system.frame(2);
        system.replace_window(2, 9);
        assert_eq!(system.frame(9), before);
        assert_eq!(system.geometry[&9].restore(), Some(rect(270., 40.)));
        let clone = system.clone_layout(layout);
        system.remove_layout(layout);
        assert!(system.frame(9).is_some());
        system.remove_layout(clone);
        assert!(system.geometry.is_empty());
    }

    /// Two windows equally near, both ahead and both overlapping the focused frame's band: the
    /// first in visible order wins.
    #[test]
    fn of_two_equally_near_windows_the_first_in_visible_order_wins() {
        let mut system = System::default();
        let layout = system.create_layout();
        for window in 1..=3 {
            system.add_window(layout, window, InsertionPoint::EndOfTree);
        }
        // 2 above and 3 below the band's centre line, the same distance to the right.
        system.store_frame(1, Rect {
            x: 0.,
            y: 100.,
            w: 100.,
            h: 100.,
        });
        system.store_frame(2, Rect {
            x: 300.,
            y: 90.,
            w: 100.,
            h: 100.,
        });
        system.store_frame(3, Rect {
            x: 300.,
            y: 110.,
            w: 100.,
            h: 100.,
        });
        system.focus(layout, Focus::Select(1));
        assert_eq!(system.window_in_direction(layout, Direction::Right), Some(2));
        let order: Vec<u32> = visible_order(&system.forest, system.forest.root(layout))
            .into_iter()
            .map(|(_, w)| w)
            .collect();
        assert_eq!(order, vec![1, 2, 3]);
    }

    /// A window overlapping the focused frame's band wins over a nearer one outside it.
    #[test]
    fn a_window_in_the_focused_band_wins_over_a_nearer_one_outside_it() {
        let mut system = System::default();
        let layout = system.create_layout();
        for window in 1..=3 {
            system.add_window(layout, window, InsertionPoint::EndOfTree);
        }
        system.store_frame(1, Rect { x: 0., y: 0., w: 100., h: 100. });
        // 2: just ahead but below the band; 3: far ahead, inside it.
        system.store_frame(2, Rect {
            x: 150.,
            y: 150.,
            w: 100.,
            h: 100.,
        });
        system.store_frame(3, Rect {
            x: 600.,
            y: 0.,
            w: 100.,
            h: 100.,
        });
        system.focus(layout, Focus::Select(1));
        assert_eq!(system.window_in_direction(layout, Direction::Right), Some(3));
    }

    /// A frame stored for a window in a stack inside a stack is shared by the outer stack's
    /// members.
    #[test]
    fn a_frame_is_shared_through_the_outermost_stack() {
        let (mut system, layout) = fixture();
        join(&mut system, layout, Direction::Right);
        stack(&mut system, layout);
        let f = &mut system.forest;
        let leaf = |f: &Forest<Key, Key, u32>, window| f.window_node(layout, window).unwrap();
        let inner = f.parent(leaf(f, 1)).unwrap();
        let outer = f.nest_in_container(layout, inner, Kind::Stack(Orientation::Horizontal));
        let three = leaf(f, 3);
        f.move_node(three, Placement::Back(outer));
        system.store_frame(1, rect(5., 5.));
        assert_eq!(system.frame(3), Some(rect(5., 5.)));
    }

    /// A window joined into a group that is not a stack forgets the frame it had to return to.
    #[test]
    fn a_window_joined_into_a_split_forgets_its_frame_to_return_to() {
        let (mut system, layout) = fixture();
        system.geometry.insert(2, Geometry::Aligned {
            frame: rect(270., 40.),
            restore: rect(9., 9.),
        });
        join(&mut system, layout, Direction::Right);
        assert_eq!(system.geometry[&2], Geometry::Kept(rect(270., 40.)));
    }

    /// Inserting next to the selection puts a new window after the selection's branch of the
    /// root; at the end of the tree, at the root's back.
    #[test]
    fn a_new_window_goes_after_the_selections_branch_or_at_the_end() {
        let (mut system, layout) = fixture();
        system.focus(layout, Focus::Select(2));
        system.add_window(layout, 5, InsertionPoint::NextToSelection);
        assert_eq!(system.forest.windows(layout, Query::All), vec![1, 2, 5, 3, 4]);
        system.focus(layout, Focus::Select(1));
        system.add_window(layout, 6, InsertionPoint::EndOfTree);
        assert_eq!(system.forest.windows(layout, Query::All), vec![1, 2, 5, 3, 4, 6]);
    }

    /// Each state the geometry reaches, through each change.
    #[test]
    fn the_geometry_moves_between_its_states_as_upstreams_pair_does() {
        let (a, b, c) = (rect(1., 1.), rect(2., 2.), rect(3., 3.));
        let kept = Geometry::Kept(a);
        let aligned = Geometry::Aligned { frame: a, restore: b };
        let restorable = Geometry::Restorable(b);
        // The pair upstream saves, both ways.
        for geometry in [kept, aligned, restorable] {
            assert_eq!(
                Geometry::from_pair(geometry.frame(), geometry.restore()),
                Some(geometry)
            );
        }
        assert_eq!(Geometry::from_pair(None, None), None);
        // A frame kept.
        assert_eq!(Geometry::with_frame(None, c), Geometry::Kept(c));
        assert_eq!(Geometry::with_frame(Some(kept), c), Geometry::Kept(c));
        assert_eq!(Geometry::with_frame(Some(aligned), c), Geometry::Aligned {
            frame: c,
            restore: b
        });
        assert_eq!(Geometry::with_frame(Some(restorable), c), Geometry::Aligned {
            frame: c,
            restore: b
        });
        // Restored.
        assert_eq!(kept.restored(), kept);
        assert_eq!(aligned.restored(), Geometry::Kept(b));
        assert_eq!(restorable.restored(), Geometry::Kept(b));
        // Forgetting the frame to return to.
        assert_eq!(kept.forgetting(), Some(kept));
        assert_eq!(aligned.forgetting(), Some(Geometry::Kept(a)));
        assert_eq!(restorable.forgetting(), None);
        // Aligned to an anchor, saving or not.
        assert_eq!(Geometry::aligned(None, None, true), None);
        assert_eq!(Geometry::aligned(None, Some(c), true), Some(Geometry::Kept(c)));
        assert_eq!(
            Geometry::aligned(Some(kept), Some(c), true),
            Some(Geometry::Aligned { frame: c, restore: a })
        );
        assert_eq!(
            Geometry::aligned(Some(kept), Some(c), false),
            Some(Geometry::Kept(c))
        );
        assert_eq!(
            Geometry::aligned(Some(kept), None, true),
            Some(Geometry::Aligned { frame: a, restore: a })
        );
        assert_eq!(
            Geometry::aligned(Some(aligned), Some(c), true),
            Some(Geometry::Aligned { frame: c, restore: b })
        );
        assert_eq!(Geometry::aligned(Some(aligned), None, false), Some(aligned));
        assert_eq!(
            Geometry::aligned(Some(restorable), Some(c), false),
            Some(Geometry::Aligned { frame: c, restore: b })
        );
        assert_eq!(Geometry::aligned(Some(restorable), None, true), Some(restorable));
        // Unstacked, focused or not.
        assert_eq!(kept.unstacked(false), Some(kept));
        assert_eq!(aligned.unstacked(true), Some(Geometry::Kept(a)));
        assert_eq!(aligned.unstacked(false), Some(Geometry::Kept(b)));
        assert_eq!(restorable.unstacked(false), Some(Geometry::Kept(b)));
        assert_eq!(restorable.unstacked(true), None);
    }
}
