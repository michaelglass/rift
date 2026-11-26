use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use rift_protocol::DirectionalDistance;
use serde::{Deserialize, Serialize};

use super::floating_twin::{Twin, held, seed};
use super::group_system::{
    core_edit, core_focus, core_query, edit_result, focus_result, layout_kind, participation,
};
use super::groups::GroupLayouts;
use super::{
    EditResult, Focus, FocusResult, GroupEdit, LayoutEdit, LayoutSystem, WindowChange,
    WindowLayoutConstraints, WindowQuery, core_direction, insertion_point,
};
use crate::actor::app::{WindowId, pid_t};
use crate::common::collections::HashMap;
use crate::common::config::WindowInsertionPoint;
use crate::layout_engine::{Direction, LayoutId, LayoutKind};
use crate::model::tree::{NodeId, Placement};

/// Geometry is shared by all arrangements of a floating window.
#[serde_with::serde_as]
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct FloatingGeometry {
    #[serde_as(as = "Option<crate::sys::geometry::CGRectDef>")]
    frame: Option<CGRect>,
    #[serde_as(as = "Option<crate::sys::geometry::CGRectDef>")]
    restore: Option<CGRect>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(from = "StoredFloating")]
pub struct FloatingLayoutSystem {
    #[serde(rename = "inner")]
    pub(super) groups: GroupLayouts,
    #[serde(skip)]
    insertion: WindowInsertionPoint,
    geometry: HashMap<WindowId, FloatingGeometry>,
    spread_on_initialization: bool,
    /// The core's floating system, held to this one on every operation (step 2).
    #[serde(skip)]
    twin: Twin,
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

impl From<StoredFloating> for FloatingLayoutSystem {
    fn from(stored: StoredFloating) -> Self {
        let mut geometry = stored.geometry;
        for (window, frame) in stored.frames {
            geometry.entry(window).or_default().frame = Some(frame);
        }
        for (window, frame) in stored.independent_frames {
            geometry.entry(window).or_default().restore = Some(frame);
        }
        let mut system = Self {
            groups: stored.groups,
            geometry,
            spread_on_initialization: stored.spread_on_initialization,
            insertion: WindowInsertionPoint::default(),
            twin: Twin::default(),
        };
        system.reseed();
        system
    }
}

impl Default for FloatingLayoutSystem {
    fn default() -> Self { Self::new(WindowInsertionPoint::default()) }
}

impl FloatingLayoutSystem {
    pub fn new(point: WindowInsertionPoint) -> Self {
        let mut system = Self {
            groups: GroupLayouts::default(),
            insertion: point,
            geometry: HashMap::default(),
            spread_on_initialization: false,
            twin: Twin::default(),
        };
        system.reseed();
        system
    }

    /// Each window's pair of frames, as the twin compares them.
    fn frame_pairs(&self) -> Vec<(WindowId, Option<CGRect>, Option<CGRect>)> {
        self.geometry
            .iter()
            .map(|(&window, geometry)| (window, geometry.frame, geometry.restore))
            .collect()
    }

    /// The core's system made this one again.
    fn reseed(&mut self) { self.twin = Twin::seeded(seed(&self.groups, self.frame_pairs())); }

    /// After an operation both systems took: the whole systems compared when due.
    fn settle(&mut self, what: &str) {
        let pairs = self.frame_pairs();
        self.twin.settle(what, &self.groups, || pairs.clone());
    }

    /// The screen and spread as the core's floating geometry takes them.
    fn core_floating(&self, screen: CGRect) -> rift_core::layout::floating::Floating {
        rift_core::layout::floating::Floating {
            screen: crate::layout_engine::core_rect(screen),
            spread: self.spread_on_initialization,
        }
    }

    /// rift-cli's drawing of the layout (upstream's, the core's held to it).
    pub(crate) fn draw_tree(&self, layout: LayoutId) -> String {
        held(
            "draw_tree",
            self.groups.draw_tree(layout, &[]),
            self.twin.core().forest().draw(layout),
        )
    }

    /// rift-cli's container view (upstream's, the core's held to it).
    pub(crate) fn container_tree(&self, layout: LayoutId) -> rift_protocol::ContainerTreeNode {
        held(
            "container_tree",
            self.groups.container_tree(layout),
            super::group_system::container_node(self.twin.core().forest().container_view(layout)),
        )
    }

    pub(crate) fn window_slot(&self, layout: LayoutId, window: WindowId) -> Option<Vec<usize>> {
        held(
            "window_slot",
            self.groups.window_slot(layout, window),
            self.twin.core().forest().window_slot(layout, window),
        )
    }

    pub fn set_window_insertion_point(&mut self, point: WindowInsertionPoint) {
        self.insertion = point;
    }

    pub(crate) fn collect_group_containers(
        &self,
        layout: LayoutId,
        screen: CGRect,
        gaps: &crate::common::config::GapSettings,
        selection_path_only: bool,
    ) -> Vec<crate::layout_engine::engine::GroupContainerInfo> {
        let upstream = self.upstream_group_containers(layout, screen, gaps, selection_path_only);
        let core =
            self.twin
                .core()
                .group_bars(layout, self.core_floating(screen), selection_path_only);
        held(
            "collect_group_containers",
            upstream.iter().map(bar_words).collect::<Vec<BarWords>>(),
            core.into_iter()
                .map(|bar| {
                    let frame = crate::layout_engine::cg_rect(bar.frame);
                    (
                        bar.stack,
                        layout_kind(rift_core::layout::group_forest::Kind::Stack(bar.orientation)),
                        rect_bits(frame),
                        bar.windows.len(),
                        bar.selected,
                        bar.windows,
                    )
                })
                .collect(),
        );
        upstream
    }

    fn upstream_group_containers(
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
                let frame = self.kept_frame(wid).unwrap_or_else(|| {
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
        held(
            "cycle_windows",
            self.upstream_cycle_windows(layout),
            self.twin.core().cycle_windows(layout),
        )
    }

    fn upstream_cycle_windows(&self, layout: LayoutId) -> Vec<WindowId> {
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
        let source = self.kept_frame(focused)?;
        let origin = source.mid();
        let horizontal = matches!(direction, Direction::Left | Direction::Right);
        let mut best: Option<((NodeId, WindowId), bool, f64)> = None;
        let _ = self.groups.visit_visible_windows(self.groups.root(layout), &mut |node, wid| {
            if wid != focused
                && !exclude.is_some_and(|parent| {
                    node.ancestors(self.groups.map()).any(|ancestor| ancestor == parent)
                })
                && let Some(frame) = self.kept_frame(wid)
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
        held(
            "window_in_direction",
            self.upstream_focus_target(layout, direction),
            self.twin.core().window_in_direction(layout, core_direction(direction)),
        )
    }

    fn upstream_focus_target(&self, layout: LayoutId, direction: Direction) -> Option<WindowId> {
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
                    .or_else(|| self.kept_frame(wid))
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
                    .or_else(|| self.kept_frame(window).map(|frame| Place::Kept(core_rect(frame))))
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
        let floating = self.core_floating(screen);
        self.twin.core_mut().initialize_frames(
            layout,
            floating,
            |window| {
                crate::layout_engine::core_geometry::core_constraints(
                    constraints.get(&window).copied().unwrap_or_default(),
                )
            },
            |window| get_frame(window).map(crate::layout_engine::core_rect),
        );
        let count = if self.spread_on_initialization {
            self.groups.windows_with_nodes(layout).count()
        } else {
            0
        };
        for (index, (_, wid)) in self.groups.windows_with_nodes(layout).enumerate() {
            if self.kept_frame(wid).is_some() {
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
        self.settle("initialize_frames");
    }

    /// Upstream's kept frame, as its own operations read it.
    fn kept_frame(&self, wid: WindowId) -> Option<CGRect> { self.geometry.get(&wid)?.frame }

    pub fn frame(&self, wid: WindowId) -> Option<CGRect> {
        let upstream = self.geometry.get(&wid).and_then(|geometry| geometry.frame);
        held(
            "frame",
            upstream.map(crate::layout_engine::core_rect),
            self.twin.core().frame(wid),
        );
        upstream
    }

    fn restore_frame(&mut self, window: WindowId) {
        if let Some(geometry) = self.geometry.get_mut(&window)
            && let Some(frame) = geometry.restore.take()
        {
            geometry.frame = Some(frame);
        }
    }

    pub fn store_frame(&mut self, wid: WindowId, frame: CGRect) {
        self.upstream_store_frame(wid, frame);
        self.twin.core_mut().store_frame(wid, crate::layout_engine::core_rect(frame));
        self.settle("store_frame");
    }

    fn upstream_store_frame(&mut self, wid: WindowId, frame: CGRect) {
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
        let frame = self.kept_frame(anchor);
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

    pub fn contains_floating_window(&self, wid: WindowId) -> bool {
        held(
            "contains_floating_window",
            self.groups.contains_any_window(wid),
            self.twin.core().forest().index().iter().any(|&(window, ..)| window == wid),
        )
    }
}
impl LayoutSystem for FloatingLayoutSystem {
    fn window_participates(&self, window: WindowId) -> bool {
        held(
            "window_participates",
            self.groups.window_participates(window),
            self.twin.core().forest().window_participates(window),
        )
    }

    fn set_window_participation(&mut self, window: WindowId, participates: bool) -> bool {
        let upstream = self.groups.set_window_participation(window, participates);
        let core = self
            .twin
            .core_mut()
            .set_window_participation(window, participation(participates));
        held("set_window_participation", upstream, core);
        self.settle("set_window_participation");
        upstream
    }

    fn contains_layout(&self, layout: LayoutId) -> bool {
        held(
            "contains_layout",
            self.groups.contains_layout(layout),
            self.twin.core().forest().contains_layout(layout),
        )
    }

    fn selected_window(&self, layout: LayoutId) -> Option<WindowId> {
        held(
            "selected_window",
            self.groups.selected_window(layout),
            self.twin.core().forest().selected_window(layout),
        )
    }

    fn selection_contains_window(&self, layout: LayoutId, window: WindowId) -> bool {
        held(
            "selection_contains_window",
            self.groups.selection_contains_window(layout, window),
            self.twin.core().forest().selection_contains_window(layout, window),
        )
    }

    fn contains_window(&self, layout: LayoutId, wid: WindowId) -> bool {
        held(
            "contains_window",
            self.groups.contains_window(layout, wid),
            self.twin.core().forest().contains_window(layout, wid),
        )
    }

    fn has_any_fullscreen_node(&self, layout: LayoutId) -> bool {
        held(
            "has_any_fullscreen_node",
            self.groups.has_any_fullscreen_node(layout),
            self.twin.core().forest().has_any_fullscreen_node(layout),
        )
    }

    fn create_layout(&mut self) -> LayoutId {
        let upstream = self.groups.create_layout();
        let core = self.twin.core_mut().create_layout();
        held("create_layout", upstream, core);
        self.settle("create_layout");
        upstream
    }

    fn clone_layout(&mut self, layout: LayoutId) -> LayoutId {
        let upstream = self.groups.clone_layout(layout);
        let core = self.twin.core_mut().clone_layout(layout);
        held("clone_layout", upstream, core);
        self.settle("clone_layout");
        upstream
    }

    fn remove_layout(&mut self, layout: LayoutId) {
        let windows = self.groups.windows(layout, WindowQuery::All);
        self.groups.remove_layout(layout);
        for wid in windows {
            if !self.groups.contains_any_window(wid) {
                self.geometry.remove(&wid);
            }
        }
        self.twin.core_mut().remove_layout(layout);
        self.settle("remove_layout");
    }

    /// The core's frames (policy (a)): the system's own ([`Self::shadow_frames`]) are their
    /// shadow; the core's floating system's shape is held to this system's.
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
        let shape = held(
            "shape",
            self.shape(layout, screen, constraints, gaps),
            self.twin.core().shape(
                layout,
                self.core_floating(screen),
                crate::layout_engine::core_gaps(gaps),
                |window| {
                    crate::layout_engine::core_geometry::core_constraints(
                        constraints.get(&window).copied().unwrap_or_default(),
                    )
                },
            ),
        );
        crate::layout_engine::core_geometry::decide_floating(shape, shadow, || {
            self.shadow_frames(layout, screen, constraints, gaps)
        })
    }

    fn window_in_direction(&self, layout: LayoutId, direction: Direction) -> Option<WindowId> {
        self.focus_target(layout, direction)
    }

    fn has_windows_for_app(&self, layout: LayoutId, pid: pid_t) -> bool {
        held(
            "has_windows_for_app",
            self.groups.has_windows_for_app(layout, pid),
            self.twin
                .core()
                .forest()
                .has_windows_for_app(layout, |window| window.pid == pid),
        )
    }

    fn parent_of_selection_is_stacked(&self, layout: LayoutId) -> bool {
        held(
            "parent_of_selection_is_stacked",
            self.groups.parent_of_selection_is_stacked(layout),
            self.twin.core().forest().parent_of_selection_is_stacked(layout),
        )
    }

    fn windows(&self, layout: LayoutId, query: WindowQuery) -> Vec<WindowId> {
        held(
            "windows",
            self.groups.windows(layout, query),
            self.twin.core().forest().windows(layout, core_query(query)),
        )
    }

    fn focus(&mut self, layout: LayoutId, focus: Focus) -> FocusResult {
        let core = focus_result(self.twin.core_mut().focus(layout, core_focus(focus)));
        let upstream = self.upstream_focus(layout, focus);
        held("focus", focus_words(&upstream), focus_words(&core));
        self.settle("focus");
        upstream
    }

    fn update(&mut self, change: WindowChange<'_>) {
        let core = self.twin.core_mut();
        match &change {
            WindowChange::Add(layout, window, _) => {
                core.add_window(*layout, *window, insertion_point(self.insertion))
            }
            WindowChange::Remove(window) | WindowChange::RemoveAndRebalance(window) => {
                core.remove_window(*window)
            }
            WindowChange::Replace(from, to) => core.replace_window(*from, *to),
            WindowChange::CloseApp(pid) => core.close_app(|window| window.pid == *pid),
            WindowChange::Resize { window, new, .. } => {
                core.store_frame(*window, crate::layout_engine::core_rect(*new))
            }
        }
        self.upstream_update(change);
        self.settle("update");
    }

    fn edit_layout(&mut self, layout: LayoutId, edit: LayoutEdit) -> EditResult {
        let core = edit_result(match core_edit(edit) {
            Some(edit) => self.twin.core_mut().edit(layout, edit),
            None => rift_core::layout::group_forest::Edited::Unsupported,
        });
        let upstream = self.upstream_edit_layout(layout, edit);
        held("edit_layout", edit_words(&upstream), edit_words(&core));
        self.settle("edit_layout");
        upstream
    }
}

/// A focus answer, comparable.
fn focus_words(result: &FocusResult) -> (bool, Option<WindowId>, Vec<WindowId>) {
    (result.handled, result.window, result.raise.clone())
}

/// An edit answer, comparable.
fn edit_words(result: &EditResult) -> (&'static str, Vec<WindowId>) {
    match result {
        EditResult::Unsupported => ("unsupported", Vec::new()),
        EditResult::Unchanged => ("unchanged", Vec::new()),
        EditResult::Changed => ("changed", Vec::new()),
        EditResult::Raise(windows) => ("raise", windows.clone()),
    }
}

/// A group bar, comparable (its frame by bits).
type BarWords = (NodeId, LayoutKind, [u64; 4], usize, usize, Vec<WindowId>);

fn bar_words(bar: &crate::layout_engine::engine::GroupContainerInfo) -> BarWords {
    (
        bar.node_id,
        bar.container_kind,
        rect_bits(bar.frame),
        bar.total_count,
        bar.selected_index,
        bar.window_ids.clone(),
    )
}

fn rect_bits(frame: CGRect) -> [u64; 4] {
    [
        frame.origin.x,
        frame.origin.y,
        frame.size.width,
        frame.size.height,
    ]
    .map(f64::to_bits)
}

impl FloatingLayoutSystem {
    fn upstream_focus(&mut self, layout: LayoutId, focus: Focus) -> FocusResult {
        match focus {
            Focus::Direction(direction) => {
                FocusResult::movement(self.move_focus(layout, direction))
            }
            _ => self.groups.focus(layout, focus),
        }
    }

    fn upstream_update(&mut self, change: WindowChange<'_>) {
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
            WindowChange::Resize { window, new, .. } => self.upstream_store_frame(window, new),
        }
    }

    fn upstream_edit_layout(&mut self, layout: LayoutId, edit: LayoutEdit) -> EditResult {
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

impl FloatingLayoutSystem {
    fn move_focus(
        &mut self,
        layout: LayoutId,
        direction: Direction,
    ) -> (Option<WindowId>, Vec<WindowId>) {
        let target = self.upstream_focus_target(layout, direction);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn w(index: u32) -> WindowId { WindowId::new(1, index) }
    fn rect(x: f64, y: f64) -> CGRect { CGRect::new(CGPoint::new(x, y), CGSize::new(200., 150.)) }
    fn fixture() -> (FloatingLayoutSystem, LayoutId) {
        let mut system = FloatingLayoutSystem::default();
        let layout = system.create_layout();
        for index in 1..=4 {
            system.update(WindowChange::Add(layout, w(index), None));
            system.store_frame(w(index), rect(20. + (index - 1) as f64 * 250., 40.));
        }
        system.focus(layout, Focus::Select(w(1))).handled;
        (system, layout)
    }
    fn stack(system: &mut FloatingLayoutSystem, layout: LayoutId) {
        system
            .edit(
                layout,
                LayoutEdit::Group(GroupEdit::Stack(
                    crate::common::config::default_stack_orientation(),
                )),
            )
            .into_windows();
    }

    /// A floating window rift has no frame for yet takes 70% of the screen, centred, and one
    /// with a frame keeps it (what rift does with a floating layout, whichever core decides it).
    #[test]
    fn a_new_floating_window_takes_most_of_the_screen_centred() {
        let mut system = FloatingLayoutSystem::default();
        let layout = system.create_layout();
        system.update(WindowChange::Add(layout, w(1), None));
        system.update(WindowChange::Add(layout, w(2), None));
        system.store_frame(w(2), rect(20., 40.));
        let frames = system.calculate_layout(
            layout,
            CGRect::new(CGPoint::ZERO, CGSize::new(1000., 800.)),
            0.0,
            &HashMap::default(),
            &crate::common::config::GapSettings::default(),
            Default::default(),
            crate::layout_engine::Shadow::Run,
        );
        assert_eq!(frames, vec![
            (
                w(1),
                CGRect::new(CGPoint::new(150., 120.), CGSize::new(700., 560.))
            ),
            (w(2), rect(20., 40.)),
        ]);
    }

    #[test]
    fn suspended_stack_selection_leaves_the_visible_tab_interactive() {
        let (mut system, layout) = fixture();
        system.edit(layout, LayoutEdit::Group(GroupEdit::Join(Direction::Right)));
        stack(&mut system, layout);
        system.set_window_participation(w(1), false);
        assert!(system.windows(layout, WindowQuery::Visible).contains(&w(2)));

        let moved = rect(100., 80.);
        system.store_frame(w(2), moved);
        assert_eq!(system.frame(w(2)), Some(moved));
        let screen = CGRect::new(CGPoint::ZERO, CGSize::new(1000., 800.));
        let groups = system.collect_group_containers(layout, screen, &Default::default(), false);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].window_ids, [w(2)]);
        assert_eq!(groups[0].frame, moved);
    }

    #[test]
    fn explicit_stacks_share_geometry_and_restore_only_their_members() {
        let (mut system, layout) = fixture();
        stack(&mut system, layout);
        assert_eq!(system.windows(layout, WindowQuery::Visible).len(), 4);
        system.edit(layout, LayoutEdit::Group(GroupEdit::Join(Direction::Right)));
        assert_ne!(system.frame(w(1)), system.frame(w(2)));
        stack(&mut system, layout);
        system.edit(layout, LayoutEdit::Group(GroupEdit::Join(Direction::Right)));
        assert_eq!(system.cycle_windows(layout), vec![w(1), w(2), w(3)]);
        system.update(WindowChange::Add(layout, w(5), None));
        system.update(WindowChange::Add(layout, w(6), None));
        assert_eq!(system.windows(layout, WindowQuery::Visible).len(), 4);
        system.focus(layout, Focus::Select(w(2))).handled;
        system.store_frame(w(2), rect(100., 80.));
        system.store_frame(w(1), rect(999., 999.));
        assert_eq!(system.frame(w(1)), Some(rect(100., 80.)));
        system.geometry.get_mut(&w(4)).unwrap().restore = Some(rect(999., 999.));
        // A stale frame to return to, set by hand: the core's system takes it too.
        system.reseed();
        system.edit(layout, LayoutEdit::Group(GroupEdit::Unjoin));
        assert_eq!(system.frame(w(2)), Some(rect(270., 40.)));
        assert_eq!(system.frame(w(4)), Some(rect(770., 40.)));
        system.focus(layout, Focus::Select(w(1))).handled;
        system.edit(
            layout,
            LayoutEdit::Group(GroupEdit::Unstack(
                crate::common::config::default_stack_orientation(),
            )),
        );
        assert_eq!(system.frame(w(1)), Some(rect(100., 80.)));
        assert_eq!(system.frame(w(3)), Some(rect(520., 40.)));
    }

    #[test]
    fn directional_focus_reaches_hidden_members_and_neighboring_windows() {
        let (mut system, layout) = fixture();
        system.edit(layout, LayoutEdit::Group(GroupEdit::Join(Direction::Right)));
        assert!(system.focus(layout, Focus::Ascend).handled);
        stack(&mut system, layout);
        assert_eq!(system.selected_window(layout), Some(w(1)));
        system.store_frame(w(3), rect(20., 400.));
        for (direction, expected) in [
            (Direction::Down, w(2)),
            (Direction::Down, w(3)),
            (Direction::Up, w(2)),
            (Direction::Up, w(1)),
        ] {
            assert_eq!(
                system.focus(layout, Focus::Direction(direction)).window,
                Some(expected)
            );
        }
        system.update(WindowChange::Remove(w(3)));
        system.update(WindowChange::Remove(w(4)));
        assert_eq!(
            system.focus(layout, Focus::Direction(Direction::Right)).window,
            Some(w(2))
        );
        assert_eq!(
            system.focus(layout, Focus::Direction(Direction::Right)).window,
            Some(w(1))
        );
        system.edit(layout, LayoutEdit::Group(GroupEdit::Unjoin));
        assert_eq!(system.windows(layout, WindowQuery::All).len(), 2);
        assert!(!system.parent_of_selection_is_stacked(layout));
    }

    #[test]
    fn saved_stack_frames_survive_identity_changes_and_layout_clones() {
        let (mut system, layout) = fixture();
        system.edit(layout, LayoutEdit::Group(GroupEdit::Join(Direction::Right)));
        stack(&mut system, layout);
        system.focus(layout, Focus::Select(w(3))).handled;
        system.edit(layout, LayoutEdit::Group(GroupEdit::Join(Direction::Right)));
        stack(&mut system, layout);
        system.focus(layout, Focus::Select(w(1))).handled;
        system.edit(layout, LayoutEdit::Group(GroupEdit::Join(Direction::Right)));
        assert_eq!(system.cycle_windows(layout), vec![w(1), w(2), w(3)]);
        assert_eq!(system.frame(w(4)), Some(rect(770., 40.)));
        let bytes = ron::to_string(&system).unwrap();
        let mut restored: FloatingLayoutSystem = ron::from_str(&bytes).unwrap();
        assert_eq!(restored.geometry, system.geometry);
        restored.update(WindowChange::Replace(w(2), w(9)));
        assert_eq!(restored.frame(w(9)), system.frame(w(2)));
        assert_eq!(restored.geometry[&w(9)].restore, Some(rect(270., 40.)));
        let clone = restored.clone_layout(layout);
        restored.remove_layout(layout);
        assert!(restored.frame(w(9)).is_some());
        restored.remove_layout(clone);
        assert!(restored.geometry.is_empty());
    }

    /// A floating system after random edits: windows added and removed, frames kept (the user
    /// moving them, or the first pass placing them, spread or not), every focus, fullscreen of a
    /// window or a group, participation, joins, stacks, unstacks, swaps, tab reorders.
    fn drawn_system(
        rng: &mut crate::layout_engine::systems::drawn::Rng,
    ) -> (FloatingLayoutSystem, LayoutId, Vec<WindowId>) {
        use crate::common::config::StackDefaultOrientation;
        use crate::layout_engine::systems::drawn::{drawn_direction, drawn_screen};
        let mut system = FloatingLayoutSystem::default();
        let layout = system.create_layout();
        let mut windows: Vec<WindowId> = Vec::new();
        let mut next = 0;
        if rng.chance(30) {
            system.spread_initial_windows();
        }
        let orientation = |rng: &mut crate::layout_engine::systems::drawn::Rng| match rng.below(4) {
            0 => StackDefaultOrientation::Perpendicular,
            1 => StackDefaultOrientation::Same,
            2 => StackDefaultOrientation::Horizontal,
            _ => StackDefaultOrientation::Vertical,
        };
        for _ in 0..rng.below(30) {
            let pick = |rng: &mut crate::layout_engine::systems::drawn::Rng,
                        windows: &[WindowId]| {
                windows[rng.below(windows.len() as u64) as usize]
            };
            match rng.below(14) {
                0..=3 => {
                    next += 1;
                    let window = WindowId::new(1 + rng.below(3) as i32, next);
                    windows.push(window);
                    system.update(WindowChange::Add(layout, window, None));
                }
                _ if windows.is_empty() => {}
                4 => {
                    let window = pick(rng, &windows);
                    windows.retain(|w| *w != window);
                    system.update(WindowChange::Remove(window));
                }
                5 => {
                    let window = pick(rng, &windows);
                    let frame = drawn_screen(rng);
                    system.store_frame(window, frame);
                }
                6 => {
                    let focus = match rng.below(4) {
                        0 => Focus::Select(pick(rng, &windows)),
                        1 => Focus::Ascend,
                        2 => Focus::Descend,
                        _ => Focus::Direction(drawn_direction(rng)),
                    };
                    system.focus(layout, focus);
                }
                7 => {
                    if rng.chance(30) {
                        system.focus(layout, Focus::Ascend);
                    }
                    let within_gaps = rng.chance(50);
                    system.edit(layout, LayoutEdit::Fullscreen { within_gaps });
                }
                8 => {
                    let window = pick(rng, &windows);
                    system.set_window_participation(window, rng.chance(40));
                }
                9 => {
                    let edit = match rng.below(5) {
                        0 => GroupEdit::Join(drawn_direction(rng)),
                        1 => GroupEdit::ConsumeOrExpel(drawn_direction(rng)),
                        2 => GroupEdit::Stack(orientation(rng)),
                        3 => GroupEdit::Unstack(orientation(rng)),
                        _ => GroupEdit::Unjoin,
                    };
                    system.edit(layout, LayoutEdit::Group(edit));
                }
                10 => {
                    let (a, b) = (pick(rng, &windows), pick(rng, &windows));
                    system.edit(layout, LayoutEdit::Swap(a, b));
                }
                11 => {
                    let (window, target) = (pick(rng, &windows), pick(rng, &windows));
                    system.edit(layout, LayoutEdit::ReorderTab { window, target });
                }
                12 => {
                    // The first pass places the windows that have no frame (seen or not).
                    let screen = drawn_screen(rng);
                    let mut seen: HashMap<WindowId, CGRect> = HashMap::default();
                    for &window in &windows {
                        if rng.chance(50) {
                            seen.insert(window, drawn_screen(rng));
                        }
                    }
                    system.initialize_frames(layout, screen, &HashMap::default(), &|window| {
                        seen.get(&window).copied()
                    });
                    if rng.chance(30) {
                        system.spread_initial_windows();
                    }
                }
                _ => {
                    system.edit(layout, LayoutEdit::ToggleOrientation);
                }
            }
        }
        (system, layout, windows)
    }

    /// The core's frames for a floating layout are the system's own (`shadow_frames`), bit for bit,
    /// over drawn systems, screens, gaps and window constraints.
    #[test]
    fn the_core_places_floating_layouts_as_the_floating_system_does() {
        use crate::layout_engine::systems::drawn::*;
        let mut rng = Rng(0x5eed_f10a);
        let (mut cases, mut frames_seen) = (0, 0);
        for _ in 0..3000 {
            let (system, layout, windows) = drawn_system(&mut rng);
            let screen = drawn_screen(&mut rng);
            let constraints = drawn_constraints_for(&mut rng, &windows);
            let gaps = drawn_gaps(&mut rng);
            // Drawn but unused (a floating layout has no bar): the draws stay as they were.
            let _ = drawn_line(&mut rng);
            let upstream = system.shadow_frames(layout, screen, &constraints, &gaps);
            let core: Vec<(WindowId, CGRect)> = system
                .shape(layout, screen, &constraints, &gaps)
                .frames()
                .into_iter()
                .map(|(window, frame)| (window, crate::layout_engine::cg_rect(frame)))
                .collect();
            assert_eq!(bits(&core), bits(&upstream));
            cases += 1;
            frames_seen += upstream.len();
        }
        eprintln!("floating: {cases} layouts, {frames_seen} frames identical");
        assert!(frames_seen > 3000, "the draws reach frames ({frames_seen})");
    }

    /// Every question the floating system answers, asked of `system` (each held to the core's).
    fn ask_everything(system: &FloatingLayoutSystem, layouts: &[LayoutId], windows: &[WindowId]) {
        use crate::layout_engine::systems::drawn::*;
        let screen = CGRect::new(CGPoint::new(10., 20.), CGSize::new(1200., 900.));
        let gaps = crate::common::config::GapSettings::default();
        for &layout in layouts {
            if !system.contains_layout(layout) {
                continue;
            }
            system.selected_window(layout);
            system.has_any_fullscreen_node(layout);
            system.parent_of_selection_is_stacked(layout);
            for query in [
                WindowQuery::All,
                WindowQuery::Visible,
                WindowQuery::Selection,
            ] {
                system.windows(layout, query);
            }
            for direction in [
                Direction::Left,
                Direction::Right,
                Direction::Up,
                Direction::Down,
            ] {
                system.window_in_direction(layout, direction);
            }
            system.cycle_windows(layout);
            system.draw_tree(layout);
            system.container_tree(layout);
            for only_path in [false, true] {
                system.collect_group_containers(layout, screen, &gaps, only_path);
            }
            let _ = system.calculate_layout(
                layout,
                screen,
                0.0,
                &HashMap::default(),
                &gaps,
                Default::default(),
                crate::layout_engine::Shadow::Run,
            );
            for &window in windows {
                system.contains_window(layout, window);
                system.selection_contains_window(layout, window);
                system.window_slot(layout, window);
                system.windows(layout, WindowQuery::Stack(window));
                system.has_windows_for_app(layout, window.pid);
            }
        }
        for &window in windows {
            system.window_participates(window);
            system.frame(window);
            system.contains_floating_window(window);
        }
        let _ = bits(&[]);
    }

    /// The core's floating system beside upstream's (step 2), over drawn systems: each drawn as
    /// the frames test draws it, then layouts cloned and removed, windows replaced, closed by
    /// app, removed (rebalancing or not) and resized through `update`, in two layouts; every
    /// question asked along the way, and the system reloaded through serde. Any difference fails
    /// the test (the twin panics in tests).
    #[test]
    fn the_core_floating_system_takes_every_operation_as_upstreams_does() {
        use crate::layout_engine::systems::drawn::*;
        let mut rng = Rng(0xf_10a7_7713);
        let mut steps = 0;
        for _ in 0..1500 {
            let (mut system, first, mut windows) = drawn_system(&mut rng);
            let mut layouts = vec![first];
            ask_everything(&system, &layouts, &windows);
            let mut next = 1000;
            for _ in 0..rng.below(25) {
                let layout = layouts[rng.below(layouts.len() as u64) as usize];
                match rng.below(9) {
                    0 => layouts.push(system.clone_layout(layout)),
                    1 if layouts.len() > 1 => {
                        system.remove_layout(layout);
                        layouts.retain(|&l| l != layout);
                    }
                    2 => {
                        next += 1;
                        let window = WindowId::new(1 + rng.below(3) as i32, next);
                        windows.push(window);
                        let second = layouts[rng.below(layouts.len() as u64) as usize];
                        system.update(WindowChange::Add(second, window, None));
                    }
                    3 if !windows.is_empty() => {
                        let from = windows[rng.below(windows.len() as u64) as usize];
                        next += 1;
                        let to = if rng.chance(20) {
                            windows[rng.below(windows.len() as u64) as usize]
                        } else {
                            WindowId::new(from.pid, next)
                        };
                        system.update(WindowChange::Replace(from, to));
                        if from != to {
                            windows.retain(|&w| w != from);
                            if !windows.contains(&to) {
                                windows.push(to);
                            }
                        }
                    }
                    4 => {
                        let pid = 1 + rng.below(3) as i32;
                        system.update(WindowChange::CloseApp(pid));
                        windows.retain(|w| w.pid != pid);
                    }
                    5 if !windows.is_empty() => {
                        let window = windows[rng.below(windows.len() as u64) as usize];
                        windows.retain(|&w| w != window);
                        system.update(if rng.chance(50) {
                            WindowChange::RemoveAndRebalance(window)
                        } else {
                            WindowChange::Remove(window)
                        });
                    }
                    6 if !windows.is_empty() => {
                        let window = windows[rng.below(windows.len() as u64) as usize];
                        let gaps = crate::common::config::GapSettings::default();
                        system.update(WindowChange::Resize {
                            layout,
                            window,
                            old: drawn_screen(&mut rng),
                            new: drawn_screen(&mut rng),
                            screen: drawn_screen(&mut rng),
                            gaps: &gaps,
                        });
                    }
                    7 => {
                        let bytes = ron::to_string(&system).unwrap();
                        let mut restored: FloatingLayoutSystem = ron::from_str(&bytes).unwrap();
                        restored.set_window_insertion_point(system.insertion);
                        system = restored;
                    }
                    _ if !windows.is_empty() => {
                        let window = windows[rng.below(windows.len() as u64) as usize];
                        system.focus(layout, Focus::Select(window));
                        system.edit(
                            layout,
                            LayoutEdit::Group(GroupEdit::Join(drawn_direction(&mut rng))),
                        );
                    }
                    _ => {}
                }
                steps += 1;
                if rng.chance(30) {
                    ask_everything(&system, &layouts, &windows);
                }
            }
            ask_everything(&system, &layouts, &windows);
        }
        eprintln!("floating twin: {steps} steps past the drawn systems, no difference");
        assert!(steps > 5000, "the draws reach steps ({steps})");
    }

    /// States only a saved file builds (floating's own operations never make them), loaded and
    /// stepped through the twin, so the core is held to upstream from each: a stack inside a
    /// stack (a stored frame shared through the outermost), an aligned window in a group that is
    /// not a stack (stacking it keeps its remembered frame), and a window at the root with a stale
    /// frame to return to (joining it into a split forgets it). Written once by upstream's own
    /// save from hand-built states; the follow-ups reach each state's rule.
    #[test]
    fn states_only_a_file_builds_step_through_the_twin() {
        let load = |text: &str| -> (FloatingLayoutSystem, LayoutId) {
            let system: FloatingLayoutSystem = ron::from_str(text).unwrap();
            let layout = system.groups.layout_roots.keys().next().unwrap();
            (system, layout)
        };
        let all = [w(1), w(2), w(3), w(4)];

        let (mut system, layout) = load(include_str!("floating/stack_in_stack.ron"));
        for (i, window) in all.into_iter().enumerate() {
            system.store_frame(window, rect(5. + i as f64, 5.));
            ask_everything(&system, &[layout], &all);
        }
        assert_eq!(
            system.frame(w(3)),
            system.frame(w(1)),
            "the outer stack shares the frame"
        );

        let (mut system, layout) = load(include_str!("floating/aligned_in_split.ron"));
        system.focus(layout, Focus::Select(w(1)));
        stack(&mut system, layout);
        ask_everything(&system, &[layout], &all);
        assert_eq!(
            system.geometry[&w(2)].restore,
            Some(rect(900., 600.)),
            "the remembered frame kept"
        );
        system.edit(
            layout,
            LayoutEdit::Group(GroupEdit::Unstack(
                crate::common::config::default_stack_orientation(),
            )),
        );
        ask_everything(&system, &[layout], &all);

        let (mut system, layout) = load(include_str!("floating/stale_restore_at_root.ron"));
        system.focus(layout, Focus::Select(w(2)));
        system.edit(layout, LayoutEdit::Group(GroupEdit::Join(Direction::Right)));
        ask_everything(&system, &[layout], &all);
        assert_eq!(
            system.geometry[&w(3)].restore,
            None,
            "joined into a split, the stale frame is forgotten"
        );
        system.edit(layout, LayoutEdit::Group(GroupEdit::Unjoin));
        ask_everything(&system, &[layout], &all);
    }
}
