//! Upstream's floating layout system (#688's `FloatingLayoutSystem`), kept as the shadow of the
//! core's floating system that decides since the flip (layout trees, floating's step 3): every
//! operation steps it, its answers and whole state held to the core's.

use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use rift_protocol::DirectionalDistance;
use serde::{Deserialize, Serialize};

use crate::actor::app::{WindowId, pid_t};
use crate::common::collections::HashMap;
use crate::common::config::WindowInsertionPoint;
use crate::layout_engine::systems::groups::GroupLayouts;
use crate::layout_engine::systems::{
    EditResult, Focus, FocusResult, GroupEdit, LayoutEdit, LayoutSystem, WindowChange,
    WindowLayoutConstraints, WindowQuery,
};
use crate::layout_engine::{Direction, LayoutId, LayoutKind};
use crate::model::tree::{NodeId, Placement};

/// Geometry is shared by all arrangements of a floating window.
#[serde_with::serde_as]
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct FloatingGeometry {
    #[serde_as(as = "Option<crate::sys::geometry::CGRectDef>")]
    pub(crate) frame: Option<CGRect>,
    #[serde_as(as = "Option<crate::sys::geometry::CGRectDef>")]
    pub(crate) restore: Option<CGRect>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(from = "StoredFloating")]
pub(crate) struct Upstream {
    #[serde(rename = "inner")]
    pub(crate) groups: GroupLayouts,
    #[serde(skip)]
    pub(crate) insertion: WindowInsertionPoint,
    pub(crate) geometry: HashMap<WindowId, FloatingGeometry>,
    pub(crate) spread_on_initialization: bool,
}

#[serde_with::serde_as]
#[derive(Deserialize)]
struct StoredFloating {
    #[serde(rename = "inner")]
    groups: GroupLayouts,
    #[serde(default)]
    geometry: HashMap<WindowId, FloatingGeometry>,
    #[serde(default)]
    #[serde_as(as = "HashMap<_, crate::sys::geometry::CGRectDef>")]
    frames: HashMap<WindowId, CGRect>,
    #[serde(default)]
    #[serde_as(as = "HashMap<_, crate::sys::geometry::CGRectDef>")]
    independent_frames: HashMap<WindowId, CGRect>,
    #[serde(default)]
    spread_on_initialization: bool,
}

impl From<StoredFloating> for Upstream {
    fn from(stored: StoredFloating) -> Self {
        let mut geometry = stored.geometry;
        for (window, frame) in stored.frames {
            geometry.entry(window).or_default().frame = Some(frame);
        }
        for (window, frame) in stored.independent_frames {
            geometry.entry(window).or_default().restore = Some(frame);
        }
        Self {
            groups: stored.groups,
            geometry,
            spread_on_initialization: stored.spread_on_initialization,
            insertion: WindowInsertionPoint::default(),
        }
    }
}

impl Default for Upstream {
    fn default() -> Self { Self::new(WindowInsertionPoint::default()) }
}

impl Upstream {
    pub(crate) fn new(point: WindowInsertionPoint) -> Self {
        Self {
            groups: GroupLayouts::default(),
            insertion: point,
            geometry: HashMap::default(),
            spread_on_initialization: false,
        }
    }

    /// Each window's pair of frames, as the shadow compares them.
    pub(crate) fn frame_pairs(&self) -> Vec<(WindowId, Option<CGRect>, Option<CGRect>)> {
        self.geometry
            .iter()
            .map(|(&window, geometry)| (window, geometry.frame, geometry.restore))
            .collect()
    }

    pub(crate) fn set_window_insertion_point(&mut self, point: WindowInsertionPoint) {
        self.insertion = point;
    }

    pub(crate) fn collect_group_containers(
        &self,
        layout: LayoutId,
        screen: CGRect,
        gaps: &crate::common::config::GapSettings,
        selection_path_only: bool,
    ) -> Vec<crate::layout_engine::engine::GroupContainerInfo> {
        let selection = self.groups.selection(layout);
        let root = self.groups.root(layout);
        root.traverse_preorder(self.groups.map())
            .filter(|node| self.groups.layout(*node).is_stacked())
            .filter(|node| {
                !selection_path_only
                    || selection.ancestors(self.groups.map()).any(|ancestor| ancestor == *node)
            })
            .filter_map(|node| {
                let children: Vec<_> = node.children(self.groups.map()).collect();
                let selected = self.groups.visible_stack_child(node)?;
                let index = children.iter().position(|child| *child == selected)?;
                let (window, wid) = self.groups.first_visible_window(selected)?;
                if self.groups.fullscreen_frame(window, screen, gaps).is_some() {
                    return None;
                }
                let frame = self.frame(wid).unwrap_or_else(|| {
                    Self::default_frame(screen, WindowLayoutConstraints::default())
                });
                Some(self.groups.stack_group_container_info(
                    node,
                    self.groups.layout(node),
                    frame,
                    &children,
                    index,
                ))
            })
            .collect()
    }

    fn members(&self, container: NodeId) -> Vec<WindowId> {
        container
            .traverse_preorder(self.groups.map())
            .filter_map(|node| self.groups.window_at(node))
            .collect()
    }

    fn selected_container(&self, layout: LayoutId) -> Option<NodeId> {
        let selection = self.groups.selection(layout);
        let container = if self.groups.window_at(selection).is_some() {
            selection.parent(self.groups.map())?
        } else {
            selection
        };
        (container != self.groups.root(layout)).then_some(container)
    }

    fn remaining_leaf(&self, moving: NodeId) -> Option<NodeId> {
        let map = self.groups.map();
        let mut siblings = moving.parent(map)?.children(map).filter(|&node| node != moving);
        siblings
            .next()
            .filter(|&node| siblings.next().is_none() && self.groups.window_at(node).is_some())
    }

    fn restore_if_independent(&mut self, layout: LayoutId, remaining: Option<NodeId>) {
        if let Some(node) = remaining
            && node.parent(self.groups.map()) == Some(self.groups.root(layout))
            && let Some(window) = self.groups.window_at(node)
        {
            self.restore_frame(window);
        }
    }

    pub(crate) fn cycle_windows(&self, layout: LayoutId) -> Vec<WindowId> {
        if let Some((node, _)) = self.groups.focused_leaf(layout)
            && let Some(parent) = node.parent(self.groups.map())
            && self.groups.layout(parent).is_stacked()
        {
            return self
                .groups
                .windows_from(parent, Some(parent), true)
                .filter(|&window| self.window_participates(window))
                .collect();
        }
        self.groups.windows(layout, WindowQuery::Visible)
    }

    fn directional_window(
        &self,
        layout: LayoutId,
        focused: WindowId,
        direction: Direction,
        exclude: Option<NodeId>,
    ) -> Option<(NodeId, WindowId)> {
        let source = self.frame(focused)?;
        let origin = source.mid();
        let horizontal = matches!(direction, Direction::Left | Direction::Right);
        let mut best: Option<((NodeId, WindowId), bool, f64)> = None;
        let _ = self.groups.visit_visible_windows(self.groups.root(layout), &mut |node, wid| {
            if wid != focused
                && !exclude.is_some_and(|parent| {
                    node.ancestors(self.groups.map()).any(|ancestor| ancestor == parent)
                })
                && let Some(frame) = self.frame(wid)
            {
                let candidate = frame.mid();
                let (across, overlaps) = if horizontal {
                    (
                        candidate.y - origin.y,
                        source.origin.y < frame.max().y && frame.origin.y < source.max().y,
                    )
                } else {
                    (
                        candidate.x - origin.x,
                        source.origin.x < frame.max().x && frame.origin.x < source.max().x,
                    )
                };
                if let Some(along) = (origin.x, origin.y)
                    .distance_in_direction((candidate.x, candidate.y), direction)
                    && along > 0.0
                {
                    let distance = along * along + across * across;
                    if best.is_none_or(|(_, outside, previous)| {
                        (!overlaps).cmp(&outside).then(distance.total_cmp(&previous)).is_lt()
                    }) {
                        best = Some(((node, wid), !overlaps, distance));
                    }
                }
            }
            std::ops::ControlFlow::<()>::Continue(())
        });
        best.map(|(target, _, _)| target)
    }

    fn focus_target(&self, layout: LayoutId, direction: Direction) -> Option<WindowId> {
        let (node, focused) = self.groups.focused_leaf(layout)?;
        let parent = node.parent(self.groups.map())?;
        let kind = self.groups.layout(parent);
        let forward = matches!(direction, Direction::Right | Direction::Down);
        let map = self.groups.map();
        let stack_target = if kind.is_stacked() {
            let next = node.step(parent, map, forward);
            self.groups.windows_from(parent, next, forward).next()
        } else {
            None
        };
        // Walk hidden members along the stack axis, then try physical neighbors.
        if kind.orientation() == direction.orientation() && stack_target.is_some() {
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
                    parent.last_descendant(map)
                };
                self.groups.windows_from(parent, Some(start), forward).next()
            })
    }

    fn default_frame(screen: CGRect, limits: WindowLayoutConstraints) -> CGRect {
        let limits = limits.normalized();
        let dimension = |value: f64, horizontal| {
            if let Some(fixed) = limits.fixed_for_axis(horizontal) {
                return fixed;
            }
            let value = value.max(limits.min_for_axis(horizontal));
            let max = limits.max_for_axis(horizontal);
            if max > 0.0 { value.min(max) } else { value }
        };
        let size = CGSize::new(
            dimension(screen.size.width * 0.7, true),
            dimension(screen.size.height * 0.7, false),
        );
        CGRect::new(
            CGPoint::new(
                screen.mid().x - size.width / 2.,
                screen.mid().y - size.height / 2.,
            ),
            size,
        )
    }

    pub(crate) fn spread_initial_windows(&mut self) { self.spread_on_initialization = true; }

    /// A floating layout as the core's geometry takes it: every window in the layout's order (a
    /// window left out holds its position), each on the screen or in the tiling area when it or
    /// an ancestor is fullscreen, else at the frame the layout keeps for it, else at a first
    /// frame. Arguments as `calculate_layout` takes them.
    /// The frames the floating system computes itself: since the core decides (policy (a)), the
    /// shadow `calculate_layout` holds the core's to.
    pub(crate) fn shadow_frames(
        &self,
        layout: LayoutId,
        screen: CGRect,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &crate::common::config::GapSettings,
    ) -> Vec<(WindowId, CGRect)> {
        let count = if self.spread_on_initialization {
            self.groups.windows_with_nodes(layout).count()
        } else {
            0
        };
        let frames: Vec<(WindowId, CGRect)> = self
            .groups
            .windows_with_nodes(layout)
            .enumerate()
            .filter(|(_, (_, wid))| self.groups.window_participates(*wid))
            .map(|(index, (node, wid))| {
                let frame = self
                    .groups
                    .fullscreen_frame(node, screen, gaps)
                    .or_else(|| self.frame(wid))
                    .unwrap_or_else(|| {
                        self.initial_frame(
                            screen,
                            constraints.get(&wid).copied().unwrap_or_default(),
                            None,
                            index,
                            count,
                        )
                    });
                (wid, frame)
            })
            .collect();
        frames
    }

    pub(crate) fn shape(
        &self,
        layout: LayoutId,
        screen: CGRect,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &crate::common::config::GapSettings,
    ) -> rift_core::layout::floating::Layout<WindowId> {
        use rift_core::layout::floating::{Floating, Layout, Place, Slot};

        use crate::layout_engine::core_geometry::core_constraints;
        use crate::layout_engine::{core_gaps, core_rect};
        let map = self.groups.map();
        let windows = self
            .groups
            .windows_with_nodes(layout)
            .map(|(node, window)| {
                if !self.groups.window_participates(window) {
                    return Slot::LeftOut;
                }
                let fullscreen = node.ancestors(map).find_map(|node| {
                    let info = &map[node].layout;
                    if info.is_fullscreen {
                        Some(Place::Screen)
                    } else if info.is_fullscreen_within_gaps {
                        Some(Place::WithinGaps)
                    } else {
                        None
                    }
                });
                let place = fullscreen
                    .or_else(|| self.frame(window).map(|frame| Place::Kept(core_rect(frame))))
                    .unwrap_or_else(|| {
                        Place::First(core_constraints(
                            constraints.get(&window).copied().unwrap_or_default(),
                        ))
                    });
                Slot::In { window, place }
            })
            .collect();
        Layout {
            floating: Floating {
                screen: core_rect(screen),
                spread: self.spread_on_initialization,
            },
            gaps: core_gaps(gaps),
            windows,
        }
    }

    fn cascade_frame(frame: CGRect, screen: CGRect, index: usize, count: usize) -> CGRect {
        if count <= 1 {
            return frame;
        }
        let slots = (count - 1) as f64;
        let available_x = (screen.size.width - frame.size.width).max(0.0);
        let available_y = (screen.size.height - frame.size.height).max(0.0);
        let span_x = (slots * 36.0).min(available_x);
        let span_y = (slots * 36.0).min(available_y);
        let fraction = index as f64 / slots;
        CGRect::new(
            CGPoint::new(
                screen.origin.x + available_x / 2.0 - span_x / 2.0 + span_x * fraction,
                screen.origin.y + available_y / 2.0 - span_y / 2.0 + span_y * fraction,
            ),
            frame.size,
        )
    }

    fn initial_frame(
        &self,
        screen: CGRect,
        limits: WindowLayoutConstraints,
        live: Option<CGRect>,
        index: usize,
        count: usize,
    ) -> CGRect {
        let default = Self::default_frame(screen, limits);
        let mut frame = live.unwrap_or(default);
        let size = CGSize::new(
            frame.size.width.min(default.size.width),
            frame.size.height.min(default.size.height),
        );
        if size != frame.size {
            frame = CGRect::new(
                CGPoint::new(
                    screen.mid().x - size.width / 2.,
                    screen.mid().y - size.height / 2.,
                ),
                size,
            );
        }
        if self.spread_on_initialization {
            Self::cascade_frame(frame, screen, index, count)
        } else {
            frame
        }
    }

    pub(crate) fn initialize_frames(
        &mut self,
        layout: LayoutId,
        screen: CGRect,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        get_frame: &impl Fn(WindowId) -> Option<CGRect>,
    ) {
        let count = if self.spread_on_initialization {
            self.groups.windows_with_nodes(layout).count()
        } else {
            0
        };
        for (index, (_, wid)) in self.groups.windows_with_nodes(layout).enumerate() {
            if self.frame(wid).is_some() {
                continue;
            }
            let frame = self.initial_frame(
                screen,
                constraints.get(&wid).copied().unwrap_or_default(),
                get_frame(wid),
                index,
                count,
            );
            self.geometry.entry(wid).or_default().frame = Some(frame);
        }
        self.spread_on_initialization = false;
    }

    pub(crate) fn frame(&self, wid: WindowId) -> Option<CGRect> { self.geometry.get(&wid)?.frame }

    fn restore_frame(&mut self, window: WindowId) {
        if let Some(geometry) = self.geometry.get_mut(&window)
            && let Some(frame) = geometry.restore.take()
        {
            geometry.frame = Some(frame);
        }
    }

    pub(crate) fn store_frame(&mut self, wid: WindowId, frame: CGRect) {
        let occurrences = self.groups.tree.window.occurrences(wid);
        if !occurrences.iter().any(|info| self.groups.node_is_visible(info.node)) {
            return;
        }
        self.geometry.entry(wid).or_default().frame = Some(frame);
        for info in occurrences {
            let node = info.node;
            // The outermost stack contains all members that share this frame.
            let stack = node
                .ancestors(self.groups.map())
                .filter(|parent| self.groups.layout(*parent).is_stacked())
                .last();
            if let Some(stack) = stack {
                for member in self.groups.windows_from(stack, Some(stack), true) {
                    self.geometry.entry(member).or_default().frame = Some(frame);
                }
            }
        }
    }

    fn align_members(&mut self, members: &[WindowId], anchor: WindowId, save: bool) {
        let frame = self.frame(anchor);
        for &window in members {
            if frame.is_none() && !self.geometry.contains_key(&window) {
                continue;
            }
            let geometry = self.geometry.entry(window).or_default();
            if save && geometry.restore.is_none() {
                geometry.restore = geometry.frame;
            }
            geometry.frame = frame.or(geometry.frame);
        }
    }

    pub(crate) fn contains_floating_window(&self, wid: WindowId) -> bool {
        self.groups.contains_any_window(wid)
    }
}
impl LayoutSystem for Upstream {
    fn window_participates(&self, window: WindowId) -> bool {
        self.groups.window_participates(window)
    }

    fn set_window_participation(&mut self, window: WindowId, participates: bool) -> bool {
        self.groups.set_window_participation(window, participates)
    }

    fn contains_layout(&self, layout: LayoutId) -> bool { self.groups.contains_layout(layout) }

    fn selected_window(&self, layout: LayoutId) -> Option<WindowId> {
        self.groups.selected_window(layout)
    }

    fn selection_contains_window(&self, layout: LayoutId, window: WindowId) -> bool {
        self.groups.selection_contains_window(layout, window)
    }

    fn contains_window(&self, layout: LayoutId, wid: WindowId) -> bool {
        self.groups.contains_window(layout, wid)
    }

    fn has_any_fullscreen_node(&self, layout: LayoutId) -> bool {
        self.groups.has_any_fullscreen_node(layout)
    }

    fn create_layout(&mut self) -> LayoutId { self.groups.create_layout() }

    fn clone_layout(&mut self, layout: LayoutId) -> LayoutId { self.groups.clone_layout(layout) }

    fn remove_layout(&mut self, layout: LayoutId) {
        let windows = self.groups.windows(layout, WindowQuery::All);
        self.groups.remove_layout(layout);
        for wid in windows {
            if !self.groups.contains_any_window(wid) {
                self.geometry.remove(&wid);
            }
        }
    }

    /// The core's frames (policy (a)): the system's own ([`Self::shadow_frames`]) are their
    /// shadow.
    fn calculate_layout(
        &self,
        layout: LayoutId,
        screen: CGRect,
        _stack_offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &crate::common::config::GapSettings,
        _stack_line: crate::common::config::StackLineReserve,
        shadow: crate::layout_engine::Shadow,
    ) -> Vec<(WindowId, CGRect)> {
        crate::layout_engine::core_geometry::decide_floating(
            self.shape(layout, screen, constraints, gaps),
            shadow,
            || self.shadow_frames(layout, screen, constraints, gaps),
        )
    }

    fn window_in_direction(&self, layout: LayoutId, direction: Direction) -> Option<WindowId> {
        self.focus_target(layout, direction)
    }

    fn has_windows_for_app(&self, layout: LayoutId, pid: pid_t) -> bool {
        self.groups.has_windows_for_app(layout, pid)
    }

    fn parent_of_selection_is_stacked(&self, layout: LayoutId) -> bool {
        self.groups.parent_of_selection_is_stacked(layout)
    }

    fn windows(&self, layout: LayoutId, query: WindowQuery) -> Vec<WindowId> {
        self.groups.windows(layout, query)
    }

    fn focus(&mut self, layout: LayoutId, focus: Focus) -> FocusResult {
        match focus {
            Focus::Direction(direction) => {
                FocusResult::movement(self.move_focus(layout, direction))
            }
            _ => self.groups.focus(layout, focus),
        }
    }

    fn update(&mut self, change: WindowChange<'_>) {
        match change {
            WindowChange::Add(layout, window, _) => self.add_window_after_selection(layout, window),
            WindowChange::Remove(window) | WindowChange::RemoveAndRebalance(window) => {
                self.remove_window(window)
            }
            WindowChange::Replace(from, to) => {
                if from == to {
                    return;
                }
                let geometry = self.geometry.remove(&from);
                self.geometry.remove(&to);
                if let Some(geometry) = geometry {
                    self.geometry.insert(to, geometry);
                }
                self.groups.replace_window(from, to);
            }
            WindowChange::CloseApp(pid) => {
                self.geometry.retain(|wid, _| wid.pid != pid);
                self.groups.close_app(pid)
            }
            WindowChange::Resize { window, new, .. } => self.store_frame(window, new),
        }
    }

    fn edit_layout(&mut self, layout: LayoutId, edit: LayoutEdit) -> EditResult {
        match edit {
            LayoutEdit::Move { .. } => EditResult::Unchanged,

            LayoutEdit::Fullscreen { within_gaps } => {
                EditResult::Raise(self.groups.toggle_fullscreen(layout, within_gaps))
            }
            LayoutEdit::Swap(a, b) => EditResult::from(self.groups.swap_windows(layout, a, b)),
            LayoutEdit::ReorderTab { window, target } => {
                EditResult::from(self.groups.move_stack_tab(layout, window, target))
            }
            LayoutEdit::ToggleOrientation => self.groups.toggle_orientation(layout),
            LayoutEdit::Group(GroupEdit::Join(direction)) => {
                EditResult::from(self.join_selection_with_direction(layout, direction))
            }
            LayoutEdit::Group(GroupEdit::ConsumeOrExpel(direction)) => {
                if self.selected_container(layout).is_some() {
                    EditResult::from(self.unjoin_selection(layout))
                } else {
                    EditResult::from(self.join_selection_with_direction(layout, direction))
                }
            }
            LayoutEdit::Group(GroupEdit::Stack(orientation)) => {
                EditResult::Raise({
                    let Some(focused) = self.groups.selected_window(layout) else {
                        return EditResult::Raise(Vec::new());
                    };
                    // Only an explicitly joined container can become a floating stack.
                    let Some(container) = self.selected_container(layout) else {
                        return EditResult::Raise(Vec::new());
                    };
                    let members = self.members(container);
                    let save = !self.groups.layout(container).is_stacked();
                    self.align_members(&members, focused, save);
                    self.groups.apply_stacking_to_parent_of_selection(layout, orientation);
                    self.groups.focus(layout, Focus::Select(focused)).handled;
                    self.groups.windows(layout, WindowQuery::Selection)
                })
            }
            LayoutEdit::Group(GroupEdit::Unstack(orientation)) => {
                EditResult::Raise(self.unstack_parent_of_selection(layout, orientation))
            }
            LayoutEdit::Group(GroupEdit::Unjoin) => EditResult::from(self.unjoin_selection(layout)),
            _ => EditResult::Unsupported,
        }
    }
}

impl Upstream {
    fn move_focus(
        &mut self,
        layout: LayoutId,
        direction: Direction,
    ) -> (Option<WindowId>, Vec<WindowId>) {
        let target = self.focus_target(layout, direction);
        if let Some(wid) = target {
            self.groups.focus(layout, Focus::Select(wid)).handled;
        }
        (target, target.into_iter().collect())
    }

    fn add_window_after_selection(&mut self, layout: LayoutId, wid: WindowId) {
        if self.groups.contains_window(layout, wid) {
            return;
        }
        // New windows remain independent until an explicit join command.
        let root = self.groups.root(layout);
        let anchor = (self.insertion == WindowInsertionPoint::NextToSelection)
            .then(|| {
                self.groups
                    .selection(layout)
                    .ancestors(self.groups.map())
                    .find(|node| node.parent(self.groups.map()) == Some(root))
            })
            .flatten();
        let node = if let Some(anchor) = anchor {
            let node = self.groups.tree.insert(Placement::After(anchor));
            self.groups.tree.window.set_window(&mut self.groups.tree.map, layout, node, wid);
            node
        } else {
            self.groups.add_window_under(layout, root, wid)
        };
        self.groups.select(node);
    }

    fn remove_window(&mut self, wid: WindowId) {
        self.geometry.remove(&wid);
        self.groups.remove_window(wid);
    }

    fn join_selection_with_direction(&mut self, layout: LayoutId, direction: Direction) -> bool {
        let Some((selection, focused)) = self.groups.focused_leaf(layout) else {
            return false;
        };
        let root = self.groups.root(layout);
        let parent = selection.parent(self.groups.map()).unwrap_or(root);
        let exclude = (parent != root).then_some(parent);
        let Some((target_node, target)) =
            self.directional_window(layout, focused, direction, exclude)
        else {
            return false;
        };
        let container = if parent == root {
            let kind = match direction {
                Direction::Left | Direction::Right => LayoutKind::Horizontal,
                _ => LayoutKind::Vertical,
            };
            self.groups.nest_in_container_internal(layout, selection, kind)
        } else {
            parent
        };
        let remaining = self.remaining_leaf(target_node);
        if self.groups.layout(container).is_stacked() {
            self.align_members(&[target], focused, true);
        } else if let Some(geometry) = self.geometry.get_mut(&target) {
            geometry.restore = None;
        }
        self.groups.tree.move_node(target_node, Placement::Back(container));
        self.restore_if_independent(layout, remaining);
        self.groups.select(selection);
        true
    }

    fn unstack_parent_of_selection(
        &mut self,
        layout: LayoutId,
        default_orientation: crate::common::config::StackDefaultOrientation,
    ) -> Vec<WindowId> {
        let focused = self.groups.selected_window(layout);
        let selection = self.groups.selection(layout);
        let container = selection
            .ancestors(self.groups.map())
            .find(|node| self.groups.layout(*node).is_stacked());
        let Some(container) = container else {
            return Vec::new();
        };
        let visible = self.groups.unstack_parent_of_selection(layout, default_orientation);
        for wid in self.groups.windows_from(container, Some(container), true) {
            if let Some(geometry) = self.geometry.get_mut(&wid) {
                let restore = geometry.restore.take();
                if Some(wid) != focused {
                    geometry.frame = restore.or(geometry.frame);
                }
            }
        }
        if let Some(wid) = focused {
            self.groups.focus(layout, Focus::Select(wid)).handled;
        }
        visible
    }

    fn unjoin_selection(&mut self, layout: LayoutId) -> bool {
        let Some((selection, wid)) = self.groups.focused_leaf(layout) else {
            return false;
        };
        let Some(parent) = selection.parent(self.groups.map()) else {
            return false;
        };
        if parent == self.groups.root(layout) {
            return false;
        }
        let remaining = self.remaining_leaf(selection);
        // Detach only the focused member. The remaining stack stays intact.
        self.groups.tree.move_node(selection, Placement::After(parent));
        self.restore_frame(wid);
        self.restore_if_independent(layout, remaining);
        self.groups.select(selection);
        true
    }
}
