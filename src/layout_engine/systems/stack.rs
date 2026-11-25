use nix::libc::pid_t;
use objc2_core_foundation::CGRect;
use rift_core::layout::traditional::{self, Fill, Traditional};
use serde::{Deserialize, Serialize};
use slotmap::{Key, KeyData, SlotMap};

use super::constraints::{constrain_frame, traditional_leaf_axis_constraints};
use super::{EditResult, Focus, FocusResult, GroupEdit, LayoutEdit, WindowChange, WindowQuery};
use crate::actor::app::WindowId;
use crate::common::collections::{HashMap, HashSet};
use crate::common::config::{
    GapSettings, StackDefaultOrientation, StackLineReserve, WindowInsertionPoint,
    default_stack_orientation,
};
use crate::layout_engine::core_geometry::{core_constraints, core_stack_line};
use crate::layout_engine::systems::{LayoutSystem, WindowLayoutConstraints};
use crate::layout_engine::utils::compute_tiling_area;
use crate::layout_engine::{Direction, LayoutId, LayoutKind, core_gaps, core_rect};
use crate::model::tree::NodeId;

impl Stack {
    fn active_index(&self, inactive: &HashSet<WindowId>) -> Option<usize> {
        self.members
            .iter()
            .position(|m| Some(m.id) == self.active && !inactive.contains(&m.window))
            .or_else(|| self.members.iter().position(|m| !inactive.contains(&m.window)))
    }

    fn visible_index(&self, inactive: &HashSet<WindowId>) -> Option<usize> {
        if self.root_selected {
            self.members.iter().position(|m| !inactive.contains(&m.window))
        } else {
            self.active_index(inactive)
        }
    }

    fn retain(&mut self, keep: impl Fn(&Member) -> bool) {
        if let Some(index) = self.members.iter().position(|m| Some(m.id) == self.active)
            && !keep(&self.members[index])
        {
            self.active = self.members[index + 1..]
                .iter()
                .find(|m| keep(m))
                .or_else(|| self.members[..index].iter().rev().find(|m| keep(m)))
                .map(|m| m.id);
            self.root_selected = false;
        }
        self.members.retain(keep);
    }

    fn remove(&mut self, window: WindowId) { self.retain(|member| member.window != window); }

    fn insert(&mut self, member: Member, after_selection: bool) {
        let index = if after_selection {
            self.members.iter().position(|m| Some(m.id) == self.active).map(|i| i + 1)
        } else {
            None
        }
        .unwrap_or(self.members.len());
        self.active = Some(member.id);
        self.root_selected = false;
        self.members.insert(index, member);
    }

    fn neighbor(&self, direction: Direction, inactive: &HashSet<WindowId>) -> Option<usize> {
        if self.root_selected || self.kind.orientation() != direction.orientation() {
            return None;
        }
        let index = self.active_index(inactive)?;
        match direction {
            Direction::Left | Direction::Up => {
                (0..index).rev().find(|&i| !inactive.contains(&self.members[i].window))
            }
            Direction::Right | Direction::Down => (index + 1..self.members.len())
                .find(|&i| !inactive.contains(&self.members[i].window)),
        }
    }
}

#[cfg(feature = "roc-core")]
mod roc_stacks;
mod stored;
mod twinned;
#[cfg(feature = "roc-core")]
pub(crate) use roc_stacks::stack_tree_reseeds;
use stored::{Loaded, Member, Stack, StoredStackSystem, stack_kind};
pub use twinned::StackLayoutSystem;
pub(crate) use twinned::stack_tree_divergences;

/// Upstream's stack system (#688): a stack owns an ordered list; selection and fullscreen never
/// require rebuilding it. Since layout trees' stack slice 1 the core's stacks are stepped beside
/// it ([`StackLayoutSystem`]).
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(try_from = "StoredStackSystem")]
pub struct Upstream {
    layouts: SlotMap<LayoutId, Stack>,
    next_node: u32,
    default_orientation: StackDefaultOrientation,
    #[serde(skip)]
    inactive: HashSet<WindowId>,
    #[serde(skip)]
    window_insertion_point: WindowInsertionPoint,
}

impl TryFrom<StoredStackSystem> for Upstream {
    type Error = String;

    fn try_from(stored: StoredStackSystem) -> Result<Self, String> {
        let loaded = Loaded::try_from(stored)?;
        Ok(Self {
            layouts: loaded.layouts,
            next_node: loaded.next_node,
            default_orientation: loaded.default_orientation,
            inactive: Default::default(),
            window_insertion_point: Default::default(),
        })
    }
}

impl Default for Upstream {
    fn default() -> Self { Self::new(default_stack_orientation()) }
}

impl Upstream {
    pub fn new(orientation: StackDefaultOrientation) -> Self {
        Self::new_with_insertion_point(orientation, WindowInsertionPoint::default())
    }

    pub fn new_with_insertion_point(
        default_orientation: StackDefaultOrientation,
        window_insertion_point: WindowInsertionPoint,
    ) -> Self {
        Self {
            layouts: SlotMap::with_key(),
            next_node: 1,
            default_orientation,
            window_insertion_point,
            inactive: Default::default(),
        }
    }

    pub fn update_settings(
        &mut self,
        orientation: StackDefaultOrientation,
        insertion: WindowInsertionPoint,
    ) {
        self.default_orientation = orientation;
        self.window_insertion_point = insertion;
    }

    fn node_id(&mut self) -> NodeId {
        let index = self.next_node;
        self.next_node = index.checked_add(1).expect("stack node identity exhausted");
        NodeId::from(KeyData::from_ffi((1u64 << 32) | u64::from(index)))
    }

    fn toggle_orientation(&mut self, layout: LayoutId) {
        let stack = &mut self.layouts[layout];
        stack.kind = if stack.kind == LayoutKind::HorizontalStack {
            LayoutKind::VerticalStack
        } else {
            LayoutKind::HorizontalStack
        };
    }

    fn toggle_fullscreen(&mut self, layout: LayoutId, within: bool) -> Vec<WindowId> {
        if self.selected_window(layout).is_none() {
            return Vec::new();
        }
        let stack = &mut self.layouts[layout];
        let (full, gaps) = if stack.root_selected {
            (&mut stack.fullscreen, &mut stack.within_gaps)
        } else {
            let index = stack.active_index(&self.inactive).unwrap();
            let m = &mut stack.members[index];
            (&mut m.fullscreen, &mut m.within_gaps)
        };
        if super::toggle_fullscreen_mode(full, gaps, within) {
            self.visible_windows_in_layout(layout)
        } else {
            Vec::new()
        }
    }

    pub(crate) fn collect_group_containers(
        &self,
        layout: LayoutId,
        screen: CGRect,
        _offset: f64,
        gaps: &GapSettings,
        _line: StackLineReserve,
        selection_path_only: bool,
    ) -> Vec<crate::layout_engine::engine::GroupContainerInfo> {
        let stack = &self.layouts[layout];
        let Some(active) = stack.visible_index(&self.inactive) else {
            return Vec::new();
        };
        if stack.fullscreen
            || stack.within_gaps
            || (selection_path_only
                && (stack.members[active].fullscreen || stack.members[active].within_gaps))
        {
            return Vec::new();
        }
        let mut selected_index = 0;
        let window_ids: Vec<_> = stack
            .members
            .iter()
            .enumerate()
            .filter(|(_, m)| !self.inactive.contains(&m.window))
            .map(|(i, m)| {
                if i < active {
                    selected_index += 1;
                }
                m.window
            })
            .collect();
        vec![crate::layout_engine::engine::GroupContainerInfo {
            node_id: stack.id,
            container_kind: stack.kind,
            frame: compute_tiling_area(screen, gaps),
            total_count: window_ids.len(),
            selected_index,
            window_ids,
        }]
    }
}

impl LayoutSystem for Upstream {
    fn has_windows_for_app(&self, layout: LayoutId, pid: pid_t) -> bool {
        self.layouts[layout].members.iter().any(|member| member.window.pid == pid)
    }

    fn window_participates(&self, window: WindowId) -> bool { !self.inactive.contains(&window) }

    fn set_window_participation(&mut self, window: WindowId, participates: bool) -> bool {
        super::update_participation(&mut self.inactive, window, participates)
    }

    fn create_layout(&mut self) -> LayoutId {
        let id = self.node_id();
        let kind = match self.default_orientation {
            StackDefaultOrientation::Horizontal | StackDefaultOrientation::Same => {
                LayoutKind::HorizontalStack
            }
            _ => LayoutKind::VerticalStack,
        };
        self.layouts.insert(Stack {
            id,
            members: Vec::new(),
            active: None,
            root_selected: false,
            kind,
            fullscreen: false,
            within_gaps: false,
        })
    }

    fn contains_layout(&self, layout: LayoutId) -> bool { self.layouts.contains_key(layout) }

    fn clone_layout(&mut self, layout: LayoutId) -> LayoutId {
        let mut stack = self.layouts[layout].clone();
        stack.id = self.node_id();
        for member in &mut stack.members {
            let id = self.node_id();
            if stack.active == Some(member.id) {
                stack.active = Some(id);
            }
            member.id = id;
        }
        self.layouts.insert(stack)
    }

    fn remove_layout(&mut self, layout: LayoutId) { self.layouts.remove(layout); }

    /// The core's frames (policy (a)): the system's own ([`Self::shadow_frames`]) are their
    /// shadow.
    fn calculate_layout(
        &self,
        layout: LayoutId,
        screen: CGRect,
        offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &GapSettings,
        line: StackLineReserve,
        shadow: crate::layout_engine::Shadow,
    ) -> Vec<(WindowId, CGRect)> {
        crate::layout_engine::core_geometry::decide_traditional(
            "stack",
            self.shape(layout, screen, offset, constraints, gaps, line),
            shadow,
            || self.shadow_frames(layout, screen, offset, constraints, gaps, line),
        )
    }

    fn selected_window(&self, layout: LayoutId) -> Option<WindowId> {
        let s = &self.layouts[layout];
        s.active_index(&self.inactive).map(|i| s.members[i].window)
    }

    fn selection_contains_window(&self, layout: LayoutId, window: WindowId) -> bool {
        let s = &self.layouts[layout];
        if s.root_selected {
            s.members.iter().any(|m| m.window == window)
        } else {
            s.members.iter().any(|m| Some(m.id) == s.active && m.window == window)
        }
    }

    fn window_in_direction(&self, layout: LayoutId, direction: Direction) -> Option<WindowId> {
        let stack = &self.layouts[layout];
        if !stack.root_selected
            && let Some(member) = stack
                .members
                .iter()
                .find(|m| Some(m.id) == stack.active && self.window_participates(m.window))
        {
            return Some(member.window);
        }
        match direction {
            Direction::Left | Direction::Up => {
                stack.members.iter().rev().find(|m| self.window_participates(m.window))
            }
            _ => stack.members.iter().find(|m| self.window_participates(m.window)),
        }
        .map(|m| m.window)
    }

    fn contains_window(&self, layout: LayoutId, window: WindowId) -> bool {
        self.layouts[layout].members.iter().any(|m| m.window == window)
    }

    fn has_any_fullscreen_node(&self, layout: LayoutId) -> bool {
        let s = &self.layouts[layout];
        self.selected_window(layout).is_some()
            && (s.fullscreen
                || s.within_gaps
                || s.members
                    .iter()
                    .any(|m| self.window_participates(m.window) && (m.fullscreen || m.within_gaps)))
    }

    fn parent_of_selection_is_stacked(&self, _layout: LayoutId) -> bool { true }

    fn windows(&self, layout: LayoutId, query: WindowQuery) -> Vec<WindowId> {
        match query {
            WindowQuery::All => self.all_windows_in_layout(layout),
            WindowQuery::Visible | WindowQuery::Selection => self.visible_windows_in_layout(layout),
            WindowQuery::Stack(window) => {
                if self.contains_window(layout, window) {
                    self.all_windows_in_layout(layout)
                } else {
                    Vec::new()
                }
            }
        }
    }

    fn focus(&mut self, layout: LayoutId, focus: Focus) -> FocusResult {
        match focus {
            Focus::Select(window) => FocusResult::selection(self.select_window(layout, window)),
            Focus::Ascend | Focus::Descend => FocusResult::selection({
                if self.selected_window(layout).is_none() {
                    return FocusResult::selection(false);
                }
                let select_root = matches!(focus, Focus::Ascend);
                std::mem::replace(&mut self.layouts[layout].root_selected, select_root)
                    != select_root
            }),
            Focus::Direction(direction) => {
                FocusResult::movement(self.move_focus(layout, direction))
            }
        }
    }

    fn update(&mut self, change: WindowChange<'_>) {
        match change {
            WindowChange::Add(layout, window, _) => self.add_window_after_selection(layout, window),
            WindowChange::Remove(window) | WindowChange::RemoveAndRebalance(window) => {
                self.inactive.remove(&window);
                for s in self.layouts.values_mut() {
                    s.remove(window);
                }
            }
            WindowChange::Replace(from, to) => {
                super::replace_participation(&mut self.inactive, from, to);
                for s in self.layouts.values_mut() {
                    for m in &mut s.members {
                        if m.window == from {
                            m.window = to;
                        }
                    }
                }
            }
            WindowChange::CloseApp(pid) => {
                self.inactive.retain(|w| w.pid != pid);
                for stack in self.layouts.values_mut() {
                    stack.retain(|member| member.window.pid != pid);
                }
            }
            WindowChange::Resize {
                layout,
                window,
                old,
                new,
                screen,
                gaps,
            } => self.on_window_resized(layout, window, old, new, screen, gaps),
        }
    }

    fn edit_layout(&mut self, layout: LayoutId, edit: LayoutEdit) -> EditResult {
        match edit {
            LayoutEdit::Move {
                direction,
                has_display_neighbor,
            } => EditResult::from({
                let s = &mut self.layouts[layout];
                if s.root_selected {
                    return EditResult::Unchanged;
                }
                let Some(i) = s.active_index(&self.inactive) else {
                    return EditResult::Unchanged;
                };
                let target = if let Some(target) = s.neighbor(direction, &self.inactive) {
                    target
                } else if has_display_neighbor || s.kind.orientation() == direction.orientation() {
                    return EditResult::Unchanged;
                } else {
                    // Leaving the stack axis places the member at the corresponding outer
                    // edge and turns the whole stack onto that axis.
                    s.kind = stack_kind(LayoutKind::from(direction.orientation()));
                    match direction {
                        Direction::Left | Direction::Up => 0,
                        _ => s.members.len() - 1,
                    }
                };
                let member = s.members.remove(i);
                s.active = Some(member.id);
                s.members.insert(target, member);
                true
            }),

            LayoutEdit::Fullscreen { within_gaps } => {
                EditResult::Raise(self.toggle_fullscreen(layout, within_gaps))
            }
            LayoutEdit::Swap(a, b) => EditResult::from(self.swap_windows(layout, a, b)),
            LayoutEdit::ReorderTab { window, target } => {
                EditResult::from(self.move_stack_tab(layout, window, target))
            }
            LayoutEdit::Drop { source, target, .. } => EditResult::from({
                source != target
                    && self.swap_windows(layout, source, target)
                    && self.select_window(layout, source)
            }),
            LayoutEdit::Group(GroupEdit::Stack(_)) => EditResult::Raise({
                self.toggle_orientation(layout);
                self.visible_windows_in_layout(layout)
            }),
            LayoutEdit::ToggleOrientation => {
                self.toggle_orientation(layout);
                EditResult::Changed
            }
            _ => EditResult::Unsupported,
        }
    }
}

impl Upstream {
    /// The frames the stack system computes itself: since the core decides (policy (a)), the
    /// shadow `calculate_layout` holds the core's to.
    pub(crate) fn shadow_frames(
        &self,
        layout: LayoutId,
        screen: CGRect,
        offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &GapSettings,
        line: StackLineReserve,
    ) -> Vec<(WindowId, CGRect)> {
        let stack = &self.layouts[layout];
        let count = stack.members.iter().filter(|m| self.window_participates(m.window)).count();
        let Some(active) = stack.visible_index(&self.inactive) else {
            return Vec::new();
        };
        let horizontal = stack.kind == LayoutKind::HorizontalStack;
        let tiling = compute_tiling_area(screen, gaps);
        let rect = if stack.fullscreen { screen } else { tiling };
        let required = constraints
            .get(&stack.members[active].window)
            .copied()
            .map(|c| {
                let (min, fixed, _, _) = traditional_leaf_axis_constraints(c, horizontal);
                fixed.unwrap_or(min)
            })
            .unwrap_or(0.0);
        let offset = super::tiles::stack_offset_with_required_size(
            rect, horizontal, count, offset, required,
        );
        let geometry = super::tiles::stack_layout_result(rect, count, offset, horizontal, line);
        let frames: Vec<(WindowId, CGRect)> = stack
            .members
            .iter()
            .filter(|m| self.window_participates(m.window))
            .enumerate()
            .map(|(i, m)| {
                let frame = if m.fullscreen {
                    screen
                } else if m.within_gaps {
                    tiling
                } else {
                    geometry.get_frame_for_index(i)
                };
                let frame = constraints.get(&m.window).copied().map_or(frame, |c| {
                    constrain_frame(frame, |horizontal| {
                        traditional_leaf_axis_constraints(c, horizontal)
                    })
                });
                (m.window, frame)
            })
            .collect();
        frames
    }

    /// A stack layout as the core's geometry takes it: one stack of the participating members,
    /// the visible one on top (none when no member participates). `offset` is the configured
    /// stack offset, as `calculate_layout` takes it.
    pub(crate) fn shape(
        &self,
        layout: LayoutId,
        screen: CGRect,
        offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &GapSettings,
        line: StackLineReserve,
    ) -> Option<Traditional<WindowId>> {
        let stack = &self.layouts[layout];
        let visible = stack.visible_index(&self.inactive)?;
        let child = |member: &Member| traditional::Child {
            size: member.weight as f32,
            node: traditional::Node::Window {
                window: member.window,
                constraints: constraints.get(&member.window).copied().map(core_constraints),
                fill: if member.fullscreen {
                    Fill::Screen
                } else if member.within_gaps {
                    Fill::WithinGaps
                } else {
                    Fill::Tiled
                },
            },
        };
        let participating = |members: &[Member]| -> Vec<traditional::Child<WindowId>> {
            members
                .iter()
                .filter(|m| self.window_participates(m.window))
                .map(child)
                .collect()
        };
        let screen_rect = core_rect(screen);
        let gaps = core_gaps(gaps);
        Some(Traditional {
            screen: screen_rect,
            area: rift_core::layout::tiling_area(screen_rect, gaps),
            gaps,
            stack_offset: offset,
            stack_line: core_stack_line(line),
            root: traditional::Node::Stack {
                horizontal: stack.kind == LayoutKind::HorizontalStack,
                fill: if stack.fullscreen {
                    Fill::Screen
                } else {
                    Fill::Tiled
                },
                before: participating(&stack.members[..visible]),
                focused: Box::new(child(&stack.members[visible])),
                after: participating(&stack.members[visible + 1..]),
            },
        })
    }

    fn all_windows_in_layout(&self, layout: LayoutId) -> Vec<WindowId> {
        self.layouts[layout].members.iter().map(|m| m.window).collect()
    }

    fn visible_windows_in_layout(&self, layout: LayoutId) -> Vec<WindowId> {
        let stack = &self.layouts[layout];
        stack
            .visible_index(&self.inactive)
            .map(|i| stack.members[i].window)
            .into_iter()
            .collect()
    }

    fn move_focus(
        &mut self,
        layout: LayoutId,
        direction: Direction,
    ) -> (Option<WindowId>, Vec<WindowId>) {
        let s = &mut self.layouts[layout];
        let Some(i) = s.neighbor(direction, &self.inactive) else {
            return (None, Vec::new());
        };
        s.active = Some(s.members[i].id);
        (Some(s.members[i].window), vec![s.members[i].window])
    }

    fn add_window_after_selection(&mut self, layout: LayoutId, window: WindowId) {
        if self.contains_window(layout, window) {
            return;
        }
        let id = self.node_id();
        self.layouts[layout].insert(
            Member {
                id,
                window,
                weight: 1.0,
                fullscreen: false,
                within_gaps: false,
            },
            self.window_insertion_point == WindowInsertionPoint::NextToSelection,
        );
    }

    fn select_window(&mut self, layout: LayoutId, window: WindowId) -> bool {
        if !self.window_participates(window) {
            return false;
        }
        let s = &mut self.layouts[layout];
        let Some(m) = s.members.iter().find(|m| m.window == window) else {
            return false;
        };
        s.active = Some(m.id);
        s.root_selected = false;
        true
    }

    fn on_window_resized(
        &mut self,
        layout: LayoutId,
        window: WindowId,
        old: CGRect,
        new: CGRect,
        screen: CGRect,
        gaps: &GapSettings,
    ) {
        if !self.window_participates(window) {
            return;
        }
        let Some(m) = self.layouts[layout].members.iter_mut().find(|m| m.window == window) else {
            return;
        };
        super::track_fullscreen_resize(
            &mut m.fullscreen,
            &mut m.within_gaps,
            old,
            new,
            screen,
            gaps,
        );
    }

    fn swap_windows(&mut self, layout: LayoutId, a: WindowId, b: WindowId) -> bool {
        if !self.window_participates(a) || !self.window_participates(b) {
            return false;
        }
        let s = &mut self.layouts[layout];
        let Some(i) = s.members.iter().position(|m| m.window == a) else {
            return false;
        };
        let Some(j) = s.members.iter().position(|m| m.window == b) else {
            return false;
        };
        if i == j {
            return false;
        }
        if !s.root_selected {
            if s.active == Some(s.members[i].id) {
                s.active = Some(s.members[j].id);
            } else if s.active == Some(s.members[j].id) {
                s.active = Some(s.members[i].id);
            }
        }
        s.members[i].window = b;
        s.members[j].window = a;
        true
    }

    fn move_stack_tab(&mut self, layout: LayoutId, window: WindowId, target: WindowId) -> bool {
        if !self.window_participates(window) || !self.window_participates(target) {
            return false;
        }
        let s = &mut self.layouts[layout];
        let Some(i) = s.members.iter().position(|m| m.window == window) else {
            return false;
        };
        let Some(j) = s.members.iter().position(|m| m.window == target) else {
            return false;
        };
        if i == j {
            return false;
        }
        let m = s.members.remove(i);
        s.members.insert(j, m);
        true
    }
}

impl Upstream {
    pub(crate) fn draw_tree(&self, layout: LayoutId) -> String {
        format!("{:#?}", self.layouts[layout])
    }

    pub(crate) fn container_tree(&self, layout: LayoutId) -> rift_protocol::ContainerTreeNode {
        use rift_protocol::{ContainerNodeType, ContainerTreeNode as Node};
        let stack = &self.layouts[layout];
        Node {
            node_id: stack.id.data().as_ffi(),
            layout_kind: Some(stack.kind),
            is_selected: stack.root_selected || stack.members.is_empty(),
            is_fullscreen: stack.fullscreen,
            is_fullscreen_within_gaps: stack.within_gaps,
            children: stack
                .members
                .iter()
                .map(|m| Node {
                    node_id: m.id.data().as_ffi(),
                    node_type: ContainerNodeType::Window,
                    window_id: Some(m.window.into()),
                    weight: Some(m.weight),
                    is_selected: !stack.root_selected && stack.active == Some(m.id),
                    is_fullscreen: m.fullscreen,
                    is_fullscreen_within_gaps: m.within_gaps,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    pub(crate) fn window_slot(&self, layout: LayoutId, window: WindowId) -> Option<Vec<usize>> {
        self.layouts[layout]
            .members
            .iter()
            .position(|m| m.window == window)
            .map(|i| vec![i])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout_engine::TraditionalLayoutSystem;
    use crate::layout_engine::engine::GroupContainerInfo;
    use crate::layout_engine::systems::LayoutEdit;
    use crate::layout_engine::systems::drawn::*;

    fn w(idx: u32) -> WindowId { WindowId::new(1, idx) }

    #[test]
    fn moving_past_the_stack_edge_leaves_the_move_to_the_engine() {
        let mut system = StackLayoutSystem::new(StackDefaultOrientation::Horizontal);
        let layout = system.create_layout();
        let first = WindowId::new(1, 1);
        let last = WindowId::new(1, 2);
        for window in [first, last] {
            system.update(WindowChange::Add(layout, window, None));
        }
        for (direction, has_display_neighbor) in [(Direction::Right, false), (Direction::Up, true)]
        {
            assert!(
                !system
                    .edit(layout, LayoutEdit::Move {
                        direction,
                        has_display_neighbor
                    })
                    .changed()
            );
        }
        assert_eq!(system.windows(layout, WindowQuery::All), [first, last]);
        assert_eq!(system.selected_window(layout), Some(last));
    }

    #[test]
    fn legacy_stack_restoration_preserves_identity_selection_and_fullscreen() {
        #[derive(Serialize)]
        struct Legacy<'a> {
            inner: &'a TraditionalLayoutSystem,
            default_orientation: StackDefaultOrientation,
        }
        let mut inner = TraditionalLayoutSystem::default();
        let removed = inner.create_layout();
        let layout = inner.create_layout();
        inner.remove_layout(removed);
        for window in [w(1), w(2), w(3)] {
            inner.update(WindowChange::Add(layout, window, None));
        }
        inner.focus(layout, Focus::Select(w(2))).handled;
        // A share other than 1 (resized while the root was tiled), which the load keeps as the
        // member's weight.
        inner
            .edit(layout, LayoutEdit::Resize {
                amount: 0.2,
                orientation: crate::layout_engine::ResizeOrientation::Horizontal,
            })
            .discard();
        inner.tiles.set_layout(inner.tiles.root(layout), LayoutKind::HorizontalStack);
        inner.edit(layout, LayoutEdit::Fullscreen { within_gaps: true }).into_windows();
        let expected = inner.tiles.container_tree(layout);
        assert!(
            expected.children.iter().any(|child| child.weight != Some(1.0)),
            "{expected:?}"
        );
        let legacy = Legacy {
            inner: &inner,
            default_orientation: StackDefaultOrientation::Horizontal,
        };
        // The legacy form as the old tree wrote it, frozen in `stack_legacy.ron` (the live text
        // is held to it while the traditional system still writes that form).
        let file = include_str!("stack/stack_legacy.ron").trim_end();
        assert_eq!(ron::to_string(&legacy).unwrap(), file);
        let mut restored = ron::from_str::<StackLayoutSystem>(file).unwrap();
        {
            assert_eq!(restored.container_tree(layout), expected);
            // The counter resumes past every node the legacy forest saved for the layout.
            let highest = std::iter::once(expected.node_id)
                .chain(expected.children.iter().map(|child| child.node_id))
                .map(|id| id as u32)
                .max()
                .unwrap();
            let probe = restored.clone_layout(layout);
            let minted = restored.container_tree(probe).node_id;
            assert_eq!(minted, (1 << 32) | u64::from(highest + 1));
            restored.remove_layout(probe);
            assert_eq!(restored.selected_window(layout), Some(w(2)));
            let clone = restored.clone_layout(layout);
            assert_eq!(restored.selected_window(clone), Some(w(2)));
            assert!(restored.has_any_fullscreen_node(clone));
            restored.update(WindowChange::Remove(w(2)));
            assert_eq!(restored.selected_window(layout), Some(w(3)));
            let new_layout = restored.create_layout();
            assert!(!restored.contains_layout(removed));
            let reused = [clone, new_layout]
                .into_iter()
                .find(|layout| layout.data().as_ffi() as u32 == removed.data().as_ffi() as u32)
                .expect("a vacant saved slot must be reused");
            assert_ne!(reused, removed);
            assert_ne!(restored.container_tree(new_layout).node_id, expected.node_id);
        }
    }

    #[test]
    fn selecting_the_stack_retains_focus_history_and_presents_its_first_member() {
        let mut system = StackLayoutSystem::default();
        let layout = system.create_layout();
        for window in [w(1), w(2), w(3)] {
            system.update(WindowChange::Add(layout, window, None));
        }
        assert!(system.focus(layout, Focus::Ascend).handled);
        assert_eq!(system.selected_window(layout), Some(w(3)));
        assert_eq!(system.windows(layout, WindowQuery::Visible), vec![w(1)]);
        assert_eq!(system.window_in_direction(layout, Direction::Left), Some(w(3)));
        assert_eq!(system.window_in_direction(layout, Direction::Right), Some(w(1)));
        assert!(system.focus(layout, Focus::Descend).handled);
        assert_eq!(system.windows(layout, WindowQuery::Visible), vec![w(3)]);
        assert!(
            system
                .edit(layout, LayoutEdit::Move {
                    direction: Direction::Left,
                    has_display_neighbor: false
                })
                .changed()
        );
        assert_eq!(system.windows(layout, WindowQuery::All), vec![w(3), w(1), w(2)]);
        assert_eq!(
            system.container_tree(layout).layout_kind,
            Some(LayoutKind::HorizontalStack)
        );
        let survivor = WindowId::new(2, 1);
        system.update(WindowChange::Add(layout, survivor, None));
        system.focus(layout, Focus::Select(w(1)));
        system.update(WindowChange::CloseApp(1));
        assert_eq!(system.windows(layout, WindowQuery::All), vec![survivor]);
        assert_eq!(system.selected_window(layout), Some(survivor));
    }

    #[test]
    fn fixed_window_caps_match_traditional_when_minimum_exceeds_locked_size() {
        use objc2_core_foundation::{CGPoint, CGSize};
        let mut system = StackLayoutSystem::default();
        let layout = system.create_layout();
        system.update(WindowChange::Add(layout, w(1), None));
        system.update(WindowChange::Add(layout, w(2), None));
        let constraints = HashMap::from_iter([(w(1), WindowLayoutConstraints {
            locked_width: 200.0,
            min_width: 300.0,
            ..Default::default()
        })]);
        let frames = system.calculate_layout(
            layout,
            CGRect::new(CGPoint::ZERO, CGSize::new(1000.0, 800.0)),
            12.0,
            &constraints,
            &GapSettings::default(),
            StackLineReserve::default(),
            crate::layout_engine::Shadow::Run,
        );
        assert_eq!(frames[0].1.size.width, 200.0);
    }

    /// A stack of two windows leaves room for its bar and offsets each window behind the first
    /// (what rift does with a stack layout, whichever core decides it).
    #[test]
    fn a_stack_reserves_its_bar_and_offsets_each_window() {
        use objc2_core_foundation::{CGPoint, CGSize};
        let mut system = StackLayoutSystem::default();
        let layout = system.create_layout();
        system.update(WindowChange::Add(layout, w(1), None));
        system.update(WindowChange::Add(layout, w(2), None));
        let line = StackLineReserve {
            thickness: 20.0,
            spacing: 4.0,
            horiz: crate::common::config::HorizontalPlacement::Top,
            vert: crate::common::config::VerticalPlacement::Left,
        };
        let frames = system.calculate_layout(
            layout,
            CGRect::new(CGPoint::ZERO, CGSize::new(1000.0, 800.0)),
            12.0,
            &HashMap::default(),
            &GapSettings::default(),
            line,
            crate::layout_engine::Shadow::Run,
        );
        let rect = |x, y, w, h| CGRect::new(CGPoint::new(x, y), CGSize::new(w, h));
        // A vertical stack: the bar (20 + 4) on the left, each window 12 points below the one
        // before and 12 shorter than the screen.
        assert_eq!(frames, vec![
            (w(1), rect(24.0, 0.0, 976.0, 788.0)),
            (w(2), rect(24.0, 12.0, 976.0, 788.0))
        ]);
    }

    /// A horizontal stack of four windows, the second selected, the fourth left out.
    fn four_with_one_left_out() -> (StackLayoutSystem, LayoutId) {
        let mut system = StackLayoutSystem::new(StackDefaultOrientation::Horizontal);
        let layout = system.create_layout();
        for window in [w(1), w(2), w(3), w(4)] {
            system.update(WindowChange::Add(layout, window, None));
        }
        system.focus(layout, Focus::Select(w(2)));
        system.set_window_participation(w(4), false);
        (system, layout)
    }

    fn screen_and_gaps() -> (CGRect, GapSettings) {
        use objc2_core_foundation::{CGPoint, CGSize};
        let screen = CGRect::new(CGPoint::new(0.0, 25.0), CGSize::new(1000.0, 800.0));
        let mut gaps = GapSettings::default();
        gaps.outer.top = 10.0;
        gaps.outer.left = 20.0;
        gaps.outer.bottom = 30.0;
        gaps.outer.right = 40.0;
        (screen, gaps)
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect {
        use objc2_core_foundation::{CGPoint, CGSize};
        CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
    }

    fn frames(system: &StackLayoutSystem, layout: LayoutId) -> Vec<(WindowId, CGRect)> {
        let (screen, gaps) = screen_and_gaps();
        system.calculate_layout(
            layout,
            screen,
            12.0,
            &HashMap::default(),
            &gaps,
            StackLineReserve::default(),
            crate::layout_engine::Shadow::Run,
        )
    }

    fn group(
        system: &StackLayoutSystem,
        layout: LayoutId,
        path_only: bool,
    ) -> Vec<GroupContainerInfo> {
        let (screen, gaps) = screen_and_gaps();
        system.collect_group_containers(
            layout,
            screen,
            0.0,
            &gaps,
            StackLineReserve::default(),
            path_only,
        )
    }

    /// rift-cli's drawing of a stack layout (`stack_drawn.txt`: the stack as upstream's system
    /// wrote it with `{:#?}`, ids as slotmap writes a key) and its container view: the stack,
    /// each member with its weight, the selected member, the window left out still listed.
    #[test]
    fn rift_cli_draws_and_lists_a_stack_and_its_members() {
        let (system, layout) = four_with_one_left_out();
        assert_eq!(
            system.draw_tree(layout),
            include_str!("stack/stack_drawn.txt").trim_end()
        );
        let tree = system.container_tree(layout);
        assert_eq!(
            (tree.node_id, tree.layout_kind, tree.is_selected),
            ((1 << 32) | 1, Some(LayoutKind::HorizontalStack), false)
        );
        let members: Vec<_> = tree
            .children
            .iter()
            .map(|m| (m.node_id, m.window_id, m.weight, m.is_selected))
            .collect();
        assert_eq!(
            members,
            (1..=4)
                .map(|idx| (
                    (1 << 32) | u64::from(idx + 1),
                    Some(w(idx).into()),
                    Some(1.0),
                    idx == 2
                ))
                .collect::<Vec<_>>()
        );
    }

    /// The group bar shows the windows taking part (the selected one counted among them), over
    /// the tiling area; none for the selection path while the shown member fills the tiling
    /// area, and none at all while the stack fills the screen.
    #[test]
    fn the_group_bar_shows_the_members_taking_part() {
        let (mut system, layout) = four_with_one_left_out();
        let shown = vec![GroupContainerInfo {
            node_id: NodeId::from(KeyData::from_ffi((1 << 32) | 1)),
            container_kind: LayoutKind::HorizontalStack,
            frame: rect(20.0, 35.0, 940.0, 760.0),
            total_count: 3,
            selected_index: 1,
            window_ids: vec![w(1), w(2), w(3)],
        }];
        let words = |infos: Vec<GroupContainerInfo>| -> Vec<_> {
            infos
                .into_iter()
                .map(|i| {
                    (
                        i.node_id,
                        i.container_kind,
                        i.frame,
                        i.total_count,
                        i.selected_index,
                        i.window_ids,
                    )
                })
                .collect()
        };
        assert_eq!(words(group(&system, layout, false)), words(shown.clone()));
        assert_eq!(words(group(&system, layout, true)), words(shown.clone()));
        system.edit(layout, LayoutEdit::Fullscreen { within_gaps: true }).discard();
        assert_eq!(words(group(&system, layout, false)), words(shown));
        assert!(group(&system, layout, true).is_empty());
        let _ = system.focus(layout, Focus::Ascend);
        system.edit(layout, LayoutEdit::Fullscreen { within_gaps: false }).discard();
        assert!(group(&system, layout, false).is_empty());
        assert!(group(&system, layout, true).is_empty());
    }

    /// A stack's frames for each fill: tiled members offset along the stack in the tiling area,
    /// a member filling the tiling area or the screen, the whole stack on the screen (the window
    /// left out placed nowhere).
    #[test]
    fn a_stack_places_each_fill() {
        let (mut system, layout) = four_with_one_left_out();
        let tiled = |x| rect(x, 35.0, 916.0, 760.0);
        assert_eq!(frames(&system, layout), vec![
            (w(1), tiled(20.0)),
            (w(2), tiled(32.0)),
            (w(3), tiled(44.0))
        ]);
        system.edit(layout, LayoutEdit::Fullscreen { within_gaps: true }).discard();
        assert_eq!(frames(&system, layout), vec![
            (w(1), tiled(20.0)),
            (w(2), rect(20.0, 35.0, 940.0, 760.0)),
            (w(3), tiled(44.0))
        ]);
        system.edit(layout, LayoutEdit::Fullscreen { within_gaps: false }).discard();
        assert!(system.container_tree(layout).children[1].is_fullscreen);
        assert_eq!(frames(&system, layout), vec![
            (w(1), tiled(20.0)),
            (w(2), rect(0.0, 25.0, 1000.0, 800.0)),
            (w(3), tiled(44.0))
        ]);
        let _ = system.focus(layout, Focus::Ascend);
        system.edit(layout, LayoutEdit::Fullscreen { within_gaps: false }).discard();
        assert_eq!(frames(&system, layout), vec![
            (w(1), rect(0.0, 25.0, 976.0, 800.0)),
            (w(2), rect(0.0, 25.0, 1000.0, 800.0)),
            (w(3), rect(24.0, 25.0, 976.0, 800.0))
        ]);
    }

    /// A system of two windows (the second selected), saved: the text the normalization tests
    /// write a corrupt state into.
    fn saved_two() -> (String, LayoutId) {
        let mut system = StackLayoutSystem::new(StackDefaultOrientation::Horizontal);
        let layout = system.create_layout();
        for window in [w(1), w(2)] {
            system.update(WindowChange::Add(layout, window, None));
        }
        (ron::to_string(&system).unwrap(), layout)
    }

    /// `saved` with `from` written as `to` (exactly once).
    fn corrupted(saved: &str, from: &str, to: &str) -> String {
        assert_eq!(saved.matches(from).count(), 1, "{from} in {saved}");
        saved.replacen(from, to, 1)
    }

    /// Loads a corrupt file: the loaded system saves the normalized text.
    fn loads_as(file: &str, normalized: &str) -> StackLayoutSystem {
        let system: StackLayoutSystem = ron::from_str(file).unwrap();
        assert_eq!(ron::to_string(&system).unwrap(), normalized);
        system
    }

    #[test]
    fn a_saved_stack_of_a_tiled_kind_loads_as_a_stack_kind() {
        let (saved, layout) = saved_two();
        let file = corrupted(&saved, "kind:horizontal_stack", "kind:horizontal");
        let system = loads_as(&file, &saved);
        assert_eq!(
            system.container_tree(layout).layout_kind,
            Some(LayoutKind::HorizontalStack)
        );
    }

    #[test]
    fn a_saved_stack_with_both_fullscreen_flags_loads_fullscreen() {
        let (saved, _) = saved_two();
        let full = corrupted(
            &saved,
            "kind:horizontal_stack,fullscreen:false",
            "kind:horizontal_stack,fullscreen:true",
        );
        let both = corrupted(
            &full,
            "kind:horizontal_stack,fullscreen:true,within_gaps:false",
            "kind:horizontal_stack,fullscreen:true,within_gaps:true",
        );
        loads_as(&both, &full);
    }

    #[test]
    fn a_saved_member_with_both_fullscreen_flags_loads_fullscreen() {
        let (saved, layout) = saved_two();
        let at = "window:(pid:1,idx:1),weight:1.0,fullscreen:";
        let full = corrupted(&saved, &format!("{at}false"), &format!("{at}true"));
        let both = corrupted(
            &full,
            &format!("{at}true,within_gaps:false"),
            &format!("{at}true,within_gaps:true"),
        );
        let system = loads_as(&both, &full);
        assert!(system.container_tree(layout).children[0].is_fullscreen);
    }

    #[test]
    fn a_saved_selection_naming_no_member_loads_on_the_first_member() {
        let (saved, layout) = saved_two();
        let first = corrupted(
            &saved,
            "active:Some((idx:3,version:1))",
            "active:Some((idx:2,version:1))",
        );
        for active in ["None", "Some((idx:9,version:1))"] {
            let file = corrupted(
                &saved,
                "active:Some((idx:3,version:1))",
                &format!("active:{active}"),
            );
            let system = loads_as(&file, &first);
            assert_eq!(system.selected_window(layout), Some(w(1)));
        }
    }

    #[test]
    fn a_saved_empty_stack_loads_with_nothing_selected() {
        let mut system = StackLayoutSystem::default();
        system.create_layout();
        let saved = ron::to_string(&system).unwrap();
        let file = corrupted(
            &saved,
            "active:None,root_selected:false",
            "active:Some((idx:7,version:1)),root_selected:true",
        );
        loads_as(&file, &saved);
    }

    fn drawn_orientation(rng: &mut Rng) -> StackDefaultOrientation {
        match rng.below(4) {
            0 => StackDefaultOrientation::Perpendicular,
            1 => StackDefaultOrientation::Same,
            2 => StackDefaultOrientation::Horizontal,
            _ => StackDefaultOrientation::Vertical,
        }
    }

    fn drawn_insertion(rng: &mut Rng) -> WindowInsertionPoint {
        if rng.chance(50) {
            WindowInsertionPoint::NextToSelection
        } else {
            WindowInsertionPoint::EndOfTree
        }
    }

    /// A stack system after random operations, each held to the core's stacks by the twin (under
    /// `roc-core`, the Rust stacks held to the deciding Roc stacks, and upstream's to the Rust):
    /// windows added, removed, replaced and closed by app, selected, the stack selected and left,
    /// made fullscreen, left out of the layout (windows in no layout too), moved, swapped,
    /// reordered and dropped, the orientation turned, native resizes into and out of the screen
    /// and the tiling area, layouts created, cloned and removed, the settings changed, every
    /// query asked on a third of the steps, and reloads through serde.
    fn drawn_system(rng: &mut Rng) -> (StackLayoutSystem, LayoutId, Vec<WindowId>) {
        let (mut orientation, mut insertion) = (drawn_orientation(rng), drawn_insertion(rng));
        let mut system = StackLayoutSystem::new_with_insertion_point(orientation, insertion);
        let layout = system.create_layout();
        let mut others: Vec<LayoutId> = Vec::new();
        let mut windows: Vec<WindowId> = Vec::new();
        let mut next = 0;
        for _ in 0..rng.below(40) {
            let pick = |rng: &mut Rng, windows: &[WindowId]| {
                windows[rng.below(windows.len() as u64) as usize]
            };
            // Most operations go to the layout drawn for frames, some to the others.
            let layout = if !others.is_empty() && rng.chance(15) {
                others[rng.below(others.len() as u64) as usize]
            } else {
                layout
            };
            if rng.chance(35) {
                asked(rng, &system, layout, &windows);
            }
            if rng.chance(10) {
                match rng.below(5) {
                    0 => others.push(system.create_layout()),
                    1 | 2 => others.push(system.clone_layout(layout)),
                    3 if !others.is_empty() => {
                        let removed = others.swap_remove(rng.below(others.len() as u64) as usize);
                        system.remove_layout(removed);
                    }
                    3 => system.remove_layout(LayoutId::default()),
                    _ => {
                        (orientation, insertion) = (drawn_orientation(rng), drawn_insertion(rng));
                        system.update_settings(orientation, insertion);
                    }
                }
                continue;
            }
            if rng.chance(4) {
                // Upstream's system re-seeded from the core (as after a divergence): the two go
                // on alike.
                system.reseed_shadow();
            }
            #[cfg(feature = "roc-core")]
            if rng.chance(4) {
                // The Rust stacks re-seeded from the deciding Roc stacks (as after a
                // divergence): the two go on alike.
                system.reseed_rust_from_roc();
            }
            if rng.chance(4) {
                // Saved and loaded (the insertion point is not saved: rift sets it again). The
                // core's save is upstream's, byte for byte.
                let mut saved = ron::to_string(&system).unwrap();
                assert_eq!(saved, system.upstream_saved());
                // Some member given a weight other than 1, which only a file from the old
                // forest holds.
                let members = saved.matches("weight:1.0").count();
                if members > 0 && rng.chance(40) {
                    let at =
                        saved.match_indices("weight:1.0").nth(rng.below(members as u64) as usize);
                    let at = at.unwrap().0;
                    let weight = ["weight:2.5", "weight:0.25", "weight:0.0"][rng.below(3) as usize];
                    saved.replace_range(at..at + "weight:1.0".len(), weight);
                }
                system = ron::from_str(&saved).unwrap();
                system.update_settings(orientation, insertion);
                continue;
            }
            if !windows.is_empty() && rng.chance(6) {
                let from = pick(rng, &windows);
                let to = if rng.chance(50) {
                    pick(rng, &windows)
                } else {
                    next += 1;
                    WindowId::new(from.pid, next)
                };
                windows.retain(|w| *w != from && *w != to);
                windows.push(to);
                system.update(WindowChange::Replace(from, to));
                continue;
            }
            if !windows.is_empty() && rng.chance(8) {
                let window = pick(rng, &windows);
                let screen = drawn_screen(rng);
                let gaps = drawn_gaps(rng);
                let tiling = compute_tiling_area(screen, &gaps);
                let frame = |rng: &mut Rng| match rng.below(4) {
                    0 => screen,
                    1 => tiling,
                    _ => drawn_screen(rng),
                };
                let (old, new) = (frame(rng), frame(rng));
                system.update(WindowChange::Resize {
                    layout,
                    window,
                    old,
                    new,
                    screen,
                    gaps: &gaps,
                });
                continue;
            }
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
                    let focus = match rng.below(5) {
                        0 | 1 => Focus::Select(pick(rng, &windows)),
                        2 => Focus::Ascend,
                        3 => Focus::Descend,
                        _ => Focus::Direction(drawn_direction(rng)),
                    };
                    system.focus(layout, focus);
                }
                6 => {
                    if rng.chance(30) {
                        system.focus(layout, Focus::Ascend);
                    }
                    let within_gaps = rng.chance(50);
                    system.edit(layout, LayoutEdit::Fullscreen { within_gaps });
                }
                7 => {
                    let window = if rng.chance(15) {
                        // A window in no layout (participation is the system's, not a layout's).
                        next += 1;
                        WindowId::new(1 + rng.below(3) as i32, next)
                    } else {
                        pick(rng, &windows)
                    };
                    system.set_window_participation(window, rng.chance(40));
                }
                8 => {
                    let direction = drawn_direction(rng);
                    let has_display_neighbor = rng.chance(30);
                    system.edit(layout, LayoutEdit::Move {
                        direction,
                        has_display_neighbor,
                    });
                }
                9 => {
                    let (a, b) = (pick(rng, &windows), pick(rng, &windows));
                    let edit = match rng.below(3) {
                        0 => LayoutEdit::Swap(a, b),
                        1 => LayoutEdit::ReorderTab { window: a, target: b },
                        _ => LayoutEdit::Drop {
                            source: a,
                            target: b,
                            action: crate::layout_engine::WindowDropAction::Swap,
                        },
                    };
                    if rng.chance(30) {
                        // Past `edit`'s participation check, which the system makes again.
                        system.edit_layout(layout, edit);
                    } else {
                        system.edit(layout, edit);
                    }
                }
                10 => {
                    let edit = match rng.below(4) {
                        0 => LayoutEdit::ToggleOrientation,
                        1 => LayoutEdit::Group(GroupEdit::Stack(drawn_orientation(rng))),
                        // Edits upstream's stack system has no case for.
                        2 => LayoutEdit::Rebalance,
                        _ => LayoutEdit::Group(GroupEdit::Unjoin),
                    };
                    system.edit(layout, edit);
                }
                11 => {
                    let window = pick(rng, &windows);
                    system.update(WindowChange::CloseApp(window.pid));
                    windows.retain(|w| w.pid != window.pid);
                }
                _ => {
                    next += 1;
                    let window = WindowId::new(1 + rng.below(3) as i32, next);
                    windows.push(window);
                    system.update(WindowChange::Add(layout, window, None));
                }
            }
        }
        (system, layout, windows)
    }

    /// Every query, about drawn windows (some not in the layout).
    fn asked(rng: &mut Rng, system: &StackLayoutSystem, layout: LayoutId, windows: &[WindowId]) {
        let window = match windows {
            [] => WindowId::new(1, 9999),
            _ => windows[rng.below(windows.len() as u64) as usize],
        };
        let _ = system.selected_window(layout);
        let _ = system.selection_contains_window(layout, window);
        let _ = system.contains_window(layout, window);
        let _ = system.has_any_fullscreen_node(layout);
        let _ = system.has_windows_for_app(layout, window.pid);
        let _ = system.window_participates(window);
        let _ = system.contains_layout(layout);
        let _ = system.contains_layout(LayoutId::default());
        for query in [
            WindowQuery::All,
            WindowQuery::Visible,
            WindowQuery::Selection,
            WindowQuery::Stack(window),
        ] {
            let _ = system.windows(layout, query);
        }
        for direction in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ] {
            let _ = system.window_in_direction(layout, direction);
        }
        let _ = system.window_slot(layout, window);
        let gaps = drawn_gaps(rng);
        for selection_path_only in [false, true] {
            let _ = system.collect_group_containers(
                layout,
                drawn_screen(rng),
                0.0,
                &gaps,
                StackLineReserve::default(),
                selection_path_only,
            );
        }
        let _ = system.draw_tree(layout);
        let _ = system.container_tree(layout);
    }

    /// The core's frames for a stack layout are the system's own (`shadow_frames`), bit for bit,
    /// over drawn stacks, screens, gaps, offsets, bars and window constraints.
    #[test]
    fn the_core_tiles_stack_layouts_as_the_stack_system_does() {
        let mut rng = Rng(0x5eed_57ac);
        let (mut cases, mut frames_seen) = (0, 0);
        for _ in 0..3000 {
            let (system, layout, windows) = drawn_system(&mut rng);
            let screen = drawn_screen(&mut rng);
            let constraints = drawn_constraints_for(&mut rng, &windows);
            let gaps = drawn_gaps(&mut rng);
            let offset = drawn_offset(&mut rng);
            let line = drawn_line(&mut rng);
            // The pass also holds the core stacks' shape to upstream's.
            let given = system.calculate_layout(
                layout,
                screen,
                offset,
                &constraints,
                &gaps,
                line,
                crate::layout_engine::Shadow::Run,
            );
            let upstream = system.shadow_frames(layout, screen, offset, &constraints, &gaps, line);
            assert_eq!(bits(&given), bits(&upstream));
            let core = core_frames(system.shape(layout, screen, offset, &constraints, &gaps, line));
            assert_eq!(bits(&core), bits(&upstream), "{}", system.draw_tree(layout));
            cases += 1;
            frames_seen += upstream.len();
        }
        eprintln!("stack: {cases} layouts, {frames_seen} frames identical");
        assert!(frames_seen > 3000, "the draws reach frames ({frames_seen})");
    }
}
