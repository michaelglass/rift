use enum_dispatch::enum_dispatch;
use objc2_core_foundation::CGRect;
use serde::{Deserialize, Serialize};

use crate::actor::app::{WindowId, pid_t};
use crate::common::collections::{HashMap, HashSet};
use crate::common::config::{GapSettings, StackDefaultOrientation};
use crate::layout_engine::{Direction, LayoutKind, ResizeOrientation, WindowDropAction};

#[derive(Serialize, Deserialize)]
struct SavedSlot<T> {
    value: Option<T>,
    version: u32,
}

fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

slotmap::new_key_type! { pub struct LayoutId; }

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WindowLayoutConstraints {
    pub is_resizable: bool,
    pub locked_width: f64,
    pub locked_height: f64,
    pub min_width: f64,
    pub min_height: f64,
    pub max_width: f64,
    pub max_height: f64,
}

impl WindowLayoutConstraints {
    pub fn normalized(self) -> Self {
        let clean = |v: f64| if v.is_finite() { v.max(0.0) } else { 0.0 };
        let min_width = clean(self.min_width);
        let min_height = clean(self.min_height);
        let mut max_width = clean(self.max_width);
        let mut max_height = clean(self.max_height);
        if max_width > 0.0 && max_width < min_width {
            max_width = min_width;
        }
        if max_height > 0.0 && max_height < min_height {
            max_height = min_height;
        }
        Self {
            is_resizable: self.is_resizable,
            locked_width: clean(self.locked_width),
            locked_height: clean(self.locked_height),
            min_width,
            min_height,
            max_width,
            max_height,
        }
    }

    pub fn min_for_axis(self, horizontal: bool) -> f64 {
        if horizontal {
            self.min_width
        } else {
            self.min_height
        }
    }

    pub fn max_for_axis(self, horizontal: bool) -> f64 {
        if horizontal {
            self.max_width
        } else {
            self.max_height
        }
    }

    pub fn fixed_for_axis(self, horizontal: bool) -> Option<f64> {
        let locked = if horizontal {
            self.locked_width
        } else {
            self.locked_height
        };
        let min = self.min_for_axis(horizontal);
        let max = self.max_for_axis(horizontal);
        // Axis-specific lock: when min/max collapse to the same positive value,
        // treat that axis as fixed even if the window is generally resizable.
        if min > 0.0 && max > 0.0 && (min - max).abs() <= f64::EPSILON {
            return Some(max);
        }
        if !self.is_resizable {
            return (locked > 0.0).then_some(locked);
        }
        None
    }

    pub fn resizable_for_axis(self, horizontal: bool) -> bool {
        self.fixed_for_axis(horizontal).is_none()
    }

    pub fn resizable_any_axis(self) -> bool {
        self.resizable_for_axis(true) || self.resizable_for_axis(false)
    }
}

/// Which windows are away: they keep their slots but take no space, selection or
/// focus. A layout system reads it from the window store each time it is called and
/// keeps no copy, so a read can't see a stale or missing set.
#[derive(Clone, Copy)]
pub struct Away<'a>(AwaySource<'a>);

#[derive(Clone, Copy)]
enum AwaySource<'a> {
    Store(&'a crate::model::window_store::WindowStore),
    #[cfg(test)]
    Only(&'a HashSet<WindowId>),
    /// No saved window is live yet: every window is away.
    Everything,
    #[cfg(test)]
    Nothing,
}

impl<'a> Away<'a> {
    /// The store's participation, settled only by its transitions. A window the
    /// store doesn't know, such as one restored from a saved layout and not yet
    /// seen, is away.
    pub fn from_store(windows: &'a crate::model::window_store::WindowStore) -> Self {
        Self(AwaySource::Store(windows))
    }

    /// Before any saved window has matched a live one, at startup.
    pub fn everything() -> Away<'static> { Away(AwaySource::Everything) }

    #[cfg(test)]
    pub fn only(windows: &'a HashSet<WindowId>) -> Self { Self(AwaySource::Only(windows)) }

    #[cfg(test)]
    pub fn nothing() -> Away<'static> { Away(AwaySource::Nothing) }

    pub fn contains(self, window: WindowId) -> bool {
        match self.0 {
            AwaySource::Store(windows) => !windows.participates(window),
            AwaySource::Everything => true,
            #[cfg(test)]
            AwaySource::Only(windows) => windows.contains(&window),
            #[cfg(test)]
            AwaySource::Nothing => false,
        }
    }

    pub fn participates(self, window: WindowId) -> bool { !self.contains(window) }
}

/// Which windows a selection repair may land on.
#[derive(Clone, Copy)]
pub enum Candidates<'a> {
    /// Structure alone: pruning a saved layout before any of its windows is live.
    Any,
    /// Windows that aren't away.
    Shown(Away<'a>),
}

impl Candidates<'_> {
    pub fn contains_away(self, window: WindowId) -> bool {
        match self {
            Candidates::Any => false,
            Candidates::Shown(away) => away.contains(window),
        }
    }

    pub fn admits(self, window: WindowId) -> bool { !self.contains_away(window) }
}

/// Which nodes of one layout take part in it.
pub(super) enum Participating {
    /// Nothing in the layout is away: every node takes part, empty placeholders included.
    All,
    /// Something in the layout is away: only these nodes take part.
    Only(HashSet<crate::model::tree::NodeId>),
}

impl Participating {
    pub(super) fn contains(&self, node: crate::model::tree::NodeId) -> bool {
        match self {
            Self::All => true,
            Self::Only(nodes) => nodes.contains(&node),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum WindowQuery {
    Visible,
    Selection,
    Stack(WindowId),
}

pub enum WindowChange<'a> {
    Add(LayoutId, WindowId, Option<Direction>),
    Remove(WindowId),
    RemoveAndRebalance(WindowId),
    Replace(WindowId, WindowId),
    CloseApp(pid_t),
    Resize {
        layout: LayoutId,
        window: WindowId,
        old: CGRect,
        new: CGRect,
        screen: CGRect,
        gaps: &'a GapSettings,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum Focus {
    Select(WindowId),
    Ascend,
    Descend,
    Direction(Direction),
}

#[derive(Debug)]
pub struct FocusResult {
    pub handled: bool,
    pub window: Option<WindowId>,
    pub raise: Vec<WindowId>,
}

impl FocusResult {
    pub fn selection(handled: bool) -> Self {
        Self {
            handled,
            window: None,
            raise: Vec::new(),
        }
    }

    pub fn movement((window, raise): (Option<WindowId>, Vec<WindowId>)) -> Self {
        Self {
            handled: window.is_some(),
            window,
            raise,
        }
    }

    pub fn into_move(self) -> (Option<WindowId>, Vec<WindowId>) { (self.window, self.raise) }
}

#[derive(Clone, Copy, Debug)]
pub enum GroupEdit {
    Join(Direction),
    ConsumeOrExpel(Direction),
    Stack(StackDefaultOrientation),
    Unstack(StackDefaultOrientation),
    Unjoin,
}

#[derive(Clone, Copy, Debug)]
pub enum LayoutEdit {
    Move {
        direction: Direction,
        has_display_neighbor: bool,
    },
    Split(LayoutKind),
    Fullscreen {
        within_gaps: bool,
    },
    Swap(WindowId, WindowId),
    ReorderTab {
        window: WindowId,
        target: WindowId,
    },
    Drop {
        source: WindowId,
        target: WindowId,
        action: WindowDropAction,
    },
    Group(GroupEdit),
    Resize {
        amount: f64,
        orientation: ResizeOrientation,
    },
    Rebalance,
    ToggleOrientation,
}

#[derive(Debug)]
pub enum EditResult {
    Unsupported,
    Unchanged,
    Changed,
    Raise(Vec<WindowId>),
}

impl From<bool> for EditResult {
    fn from(changed: bool) -> Self {
        if changed {
            Self::Changed
        } else {
            Self::Unchanged
        }
    }
}

impl EditResult {
    pub fn changed(&self) -> bool { matches!(self, Self::Changed | Self::Raise(_)) }

    pub fn into_windows(self) -> Vec<WindowId> {
        if let Self::Raise(windows) = self {
            windows
        } else {
            Vec::new()
        }
    }

    pub fn discard(self) {}
}

/// Toggle one fullscreen mode and clear the other when enabling it.
fn toggle_fullscreen_mode(full: &mut bool, gaps: &mut bool, within_gaps: bool) -> bool {
    let (target, other) = if within_gaps {
        (gaps, full)
    } else {
        (full, gaps)
    };
    *target = !*target;
    if *target {
        *other = false;
    }
    *target
}

/// Return whether a native resize entered or left a fullscreen frame.
fn track_fullscreen_resize(
    full: &mut bool,
    within_gaps: &mut bool,
    old: CGRect,
    new: CGRect,
    screen: CGRect,
    gaps: &GapSettings,
) -> bool {
    if new == screen {
        (*full, *within_gaps) = (true, false);
    } else if old == screen {
        *full = false;
    } else {
        let tiling = super::utils::compute_tiling_area(screen, gaps);
        if new == tiling {
            (*full, *within_gaps) = (false, true);
        } else if old == tiling {
            *within_gaps = false;
        } else {
            return false;
        }
    }
    true
}

#[enum_dispatch]
pub trait LayoutSystem: Serialize + for<'de> Deserialize<'de> {
    /// A window's participation really changed, as the window store's transition
    /// reports it. Systems that react to the change itself, such as scrolling moving
    /// its selection off a column that went away, do so here.
    fn participation_changed(
        &mut self,
        _window: WindowId,
        _change: crate::model::window_store::ParticipationChange,
        _away: Away<'_>,
    ) {
    }

    fn create_layout(&mut self) -> LayoutId;

    fn contains_layout(&self, layout: LayoutId) -> bool;

    fn clone_layout(&mut self, layout: LayoutId) -> LayoutId;

    fn remove_layout(&mut self, layout: LayoutId);

    fn prepare_layout(
        &mut self,
        _layout: LayoutId,
        _screen: CGRect,
        _constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        _gaps: &crate::common::config::GapSettings,
        _away: Away<'_>,
    ) {
    }

    #[allow(clippy::too_many_arguments)]
    fn calculate_layout(
        &self,
        layout: LayoutId,
        screen: CGRect,
        stack_offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &crate::common::config::GapSettings,
        stack_line: crate::common::config::StackLineReserve,
        away: Away<'_>,
    ) -> Vec<(WindowId, CGRect)>;

    fn selected_window(&self, layout: LayoutId, away: Away<'_>) -> Option<WindowId>;

    fn selection_contains_window(
        &self,
        layout: LayoutId,
        window: WindowId,
        away: Away<'_>,
    ) -> bool {
        self.selected_window(layout, away) == Some(window)
    }

    fn window_in_direction(
        &self,
        layout: LayoutId,
        direction: Direction,
        away: Away<'_>,
    ) -> Option<WindowId>;

    fn has_windows_for_app(&self, layout: LayoutId, pid: pid_t) -> bool;

    fn contains_window(&self, layout: LayoutId, wid: WindowId) -> bool;

    fn has_any_fullscreen_node(&self, layout: LayoutId, away: Away<'_>) -> bool;

    fn parent_of_selection_is_stacked(&self, _layout: LayoutId, _away: Away<'_>) -> bool { false }

    /// Every window the layout holds, away or not.
    fn all_windows(&self, layout: LayoutId) -> Vec<WindowId>;
    fn windows(&self, layout: LayoutId, query: WindowQuery, away: Away<'_>) -> Vec<WindowId>;
    fn focus(&mut self, layout: LayoutId, focus: Focus, away: Away<'_>) -> FocusResult;
    fn update(&mut self, change: WindowChange<'_>, away: Away<'_>);
    /// Drop a window from a saved layout before any window is live: structural, so
    /// selection is repaired by structure alone.
    fn forget_window(&mut self, window: WindowId);
    fn edit_layout(&mut self, layout: LayoutId, edit: LayoutEdit, away: Away<'_>) -> EditResult;

    fn edit(&mut self, layout: LayoutId, edit: LayoutEdit, away: Away<'_>) -> EditResult {
        let LayoutEdit::Drop { source, target, action } = edit else {
            return self.edit_layout(layout, edit, away);
        };
        if away.contains(source) || away.contains(target) {
            return EditResult::Unchanged;
        }
        let translated = match action {
            WindowDropAction::Move(direction) => LayoutEdit::Move {
                direction,
                has_display_neighbor: false,
            },
            WindowDropAction::ConsumeOrExpel(direction) => {
                LayoutEdit::Group(GroupEdit::ConsumeOrExpel(direction))
            }
            _ => return self.edit_layout(layout, edit, away),
        };
        if !self.focus(layout, Focus::Select(source), away).handled {
            return EditResult::Unchanged;
        }
        self.edit_layout(layout, translated, away)
    }
}

pub(super) fn node_slot(
    mut node: crate::model::tree::NodeId,
    root: crate::model::tree::NodeId,
    map: &crate::model::tree::NodeMap,
) -> Option<Vec<usize>> {
    let mut path = Vec::new();
    while node != root {
        let parent = node.parent(map)?;
        path.push(parent.children(map).position(|child| child == node)?);
        node = parent;
    }
    path.reverse();
    Some(path)
}

mod groups;
mod tiles;
mod traditional;
pub use traditional::TraditionalLayoutSystem;
mod bsp;
pub(crate) mod constraints;
pub use bsp::BspLayoutSystem;
mod master_stack;
pub use master_stack::MasterStackLayoutSystem;
pub(crate) mod scrolling;
pub use scrolling::{ScrollingLayoutSystem, ViewportRelease};

#[cfg(test)]
mod tests {
    use super::{
        BspLayoutSystem, LayoutSystem, MasterStackLayoutSystem, ScrollingLayoutSystem,
        TraditionalLayoutSystem,
    };
    use crate::actor::app::WindowId;
    use crate::common::config::{ScrollingLayoutSettings, WindowInsertionPoint};
    use crate::layout_engine::Away;
    use crate::layout_engine::systems::{Focus, WindowChange};

    fn w(idx: u32) -> WindowId { WindowId::new(1, idx) }

    #[test]
    fn common_insertion_point_controls_tree_and_linear_layouts() {
        let mut traditional = TraditionalLayoutSystem::new(WindowInsertionPoint::EndOfTree, false);
        let traditional_layout = traditional.create_layout();
        traditional.update(
            WindowChange::Add(traditional_layout, w(1), None),
            Away::nothing(),
        );
        traditional.update(
            WindowChange::Add(traditional_layout, w(2), None),
            Away::nothing(),
        );
        traditional
            .focus(traditional_layout, Focus::Select(w(1)), Away::nothing())
            .handled;
        traditional.update(
            WindowChange::Add(traditional_layout, w(3), None),
            Away::nothing(),
        );
        assert_eq!(traditional.all_windows(traditional_layout), vec![
            w(1),
            w(2),
            w(3)
        ]);

        let mut scrolling_settings = ScrollingLayoutSettings::default();
        scrolling_settings.base.window_insertion_point = Some(WindowInsertionPoint::EndOfTree);
        let mut scrolling = ScrollingLayoutSystem::new(&scrolling_settings);
        let scrolling_layout = scrolling.create_layout();
        scrolling.update(WindowChange::Add(scrolling_layout, w(1), None), Away::nothing());
        scrolling.update(WindowChange::Add(scrolling_layout, w(2), None), Away::nothing());
        scrolling.focus(scrolling_layout, Focus::Select(w(1)), Away::nothing()).handled;
        scrolling.update(WindowChange::Add(scrolling_layout, w(3), None), Away::nothing());
        assert_eq!(scrolling.all_windows(scrolling_layout), vec![w(1), w(2), w(3)]);
    }

    fn window_nodes(
        tree: &rift_protocol::ContainerTreeNode,
    ) -> Vec<&rift_protocol::ContainerTreeNode> {
        let mut windows = Vec::new();
        if tree.node_type == rift_protocol::ContainerNodeType::Window {
            windows.push(tree);
        }
        for child in &tree.children {
            windows.extend(window_nodes(child));
        }
        windows
    }

    fn node_ids(tree: &rift_protocol::ContainerTreeNode) -> Vec<u64> {
        let mut ids = vec![tree.node_id];
        for child in &tree.children {
            ids.extend(node_ids(child));
        }
        ids
    }

    fn assert_stable_unique_ids(
        before: &rift_protocol::ContainerTreeNode,
        after: &rift_protocol::ContainerTreeNode,
    ) {
        let before_ids = node_ids(before);
        let after_ids = node_ids(after);
        assert_eq!(before_ids, after_ids);
        let unique: std::collections::HashSet<_> = after_ids.iter().copied().collect();
        assert_eq!(unique.len(), after_ids.len());
    }

    #[test]
    fn normalized_container_trees_expose_layout_topology() {
        let mut traditional = TraditionalLayoutSystem::default();
        let layout = traditional.create_layout();
        traditional.update(WindowChange::Add(layout, w(1), None), Away::nothing());
        traditional.update(WindowChange::Add(layout, w(2), None), Away::nothing());
        let tree = traditional.tiles.container_tree(layout, Away::nothing());
        assert_eq!(tree.node_type, rift_protocol::ContainerNodeType::Container);
        assert_eq!(window_nodes(&tree).len(), 2);
        assert_eq!(
            window_nodes(&tree).iter().filter(|node| node.is_selected).count(),
            1
        );
        traditional.focus(layout, Focus::Select(w(1)), Away::nothing()).handled;
        assert_stable_unique_ids(&tree, &traditional.tiles.container_tree(layout, Away::nothing()));

        let mut bsp = BspLayoutSystem::default();
        let layout = bsp.create_layout();
        bsp.update(WindowChange::Add(layout, w(1), None), Away::nothing());
        bsp.update(WindowChange::Add(layout, w(2), None), Away::nothing());
        let tree = bsp.container_tree(layout);
        assert_eq!(tree.children.len(), 2);
        let total_weight: f64 = tree.children.iter().filter_map(|node| node.weight).sum();
        assert!((total_weight - 1.0).abs() < f64::EPSILON);
        bsp.focus(layout, Focus::Select(w(1)), Away::nothing()).handled;
        assert_stable_unique_ids(&tree, &bsp.container_tree(layout));

        let mut master_stack = MasterStackLayoutSystem::default();
        let layout = master_stack.create_layout();
        master_stack.update(WindowChange::Add(layout, w(1), None), Away::nothing());
        master_stack.update(WindowChange::Add(layout, w(2), None), Away::nothing());
        let tree = master_stack.container_tree(layout, Away::nothing());
        let roles: Vec<_> = tree.children.iter().filter_map(|node| node.role.as_deref()).collect();
        assert!(roles.contains(&"master"), "{tree:#?}");
        assert!(roles.contains(&"stack"), "{tree:#?}");

        let mut scrolling = ScrollingLayoutSystem::default();
        let layout = scrolling.create_layout();
        scrolling.update(WindowChange::Add(layout, w(1), None), Away::nothing());
        scrolling.update(WindowChange::Add(layout, w(2), None), Away::nothing());
        let tree = scrolling.container_tree(layout, Away::nothing());
        assert!(tree.children.iter().all(|node| node.role.as_deref() == Some("column")));
        assert_eq!(window_nodes(&tree).len(), 2);
        scrolling.focus(layout, Focus::Select(w(1)), Away::nothing()).handled;
        assert_stable_unique_ids(&tree, &scrolling.container_tree(layout, Away::nothing()));
    }
}
mod floating;
pub use floating::FloatingLayoutSystem;
mod stack;
pub use stack::StackLayoutSystem;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Debug)]
#[enum_dispatch(LayoutSystem)]
pub enum LayoutSystemKind {
    Traditional(TraditionalLayoutSystem),
    Bsp(BspLayoutSystem),
    MasterStack(MasterStackLayoutSystem),
    Scrolling(ScrollingLayoutSystem),
    Stack(StackLayoutSystem),
    Floating(FloatingLayoutSystem),
}

impl LayoutSystemKind {
    pub(crate) fn preview_clone(&self) -> Option<Self> {
        match self {
            Self::Traditional(system) => Some(Self::Traditional(system.clone())),
            Self::Bsp(system) => Some(Self::Bsp(system.clone())),
            Self::MasterStack(system) => Some(Self::MasterStack(system.clone())),
            Self::Scrolling(system) => Some(Self::Scrolling(system.clone())),
            Self::Stack(system) => Some(Self::Stack(system.clone())),
            Self::Floating(_) => None,
        }
    }
}

#[cfg(test)]
mod participation_tests {
    use objc2_core_foundation::{CGPoint, CGSize};

    use super::*;
    use crate::layout_engine::Away;
    use crate::layout_engine::systems::{Focus, LayoutEdit, WindowChange};

    /// What the engine does when the store's participation flips: marks `window`
    /// away or back and tells the system about the change.
    fn set_away(
        system: &mut LayoutSystemKind,
        away: &mut HashSet<WindowId>,
        window: WindowId,
        is_away: bool,
    ) {
        let changed = if is_away {
            away.insert(window)
        } else {
            away.remove(&window)
        };
        if changed {
            let change = if is_away {
                crate::model::window_store::ParticipationChange::Suspended
            } else {
                crate::model::window_store::ParticipationChange::Resumed
            };
            system.participation_changed(window, change, Away::only(away));
        }
    }

    #[test]
    fn suspended_members_preserve_slots_and_geometry_in_every_backend() {
        let mut systems = [
            LayoutSystemKind::Traditional(Default::default()),
            LayoutSystemKind::Bsp(Default::default()),
            LayoutSystemKind::Stack(Default::default()),
            LayoutSystemKind::MasterStack(Default::default()),
            LayoutSystemKind::Scrolling(Default::default()),
            LayoutSystemKind::Floating(Default::default()),
        ];
        let screen = CGRect::new(CGPoint::ZERO, CGSize::new(1200.0, 800.0));
        let constraints = HashMap::default();
        let mut gaps = crate::common::config::GapSettings::default();
        gaps.inner.horizontal = 16.0;
        gaps.inner.vertical = 12.0;
        for (system, mode) in systems.iter_mut().zip([
            "traditional",
            "bsp",
            "stack",
            "master_stack",
            "scrolling",
            "floating",
        ]) {
            let layout = system.create_layout();
            let mut away = HashSet::default();
            let frames = |system: &mut LayoutSystemKind, away: &HashSet<WindowId>| {
                system.prepare_layout(layout, screen, &constraints, &gaps, Away::only(away));
                system.calculate_layout(
                    layout,
                    screen,
                    20.0,
                    &constraints,
                    &gaps,
                    Default::default(),
                    Away::only(away),
                )
            };
            let windows = [1, 2, 3, 4].map(|i| WindowId::new(1, i));
            for (index, &window) in windows.iter().enumerate() {
                if index == 2 {
                    system
                        .edit(
                            layout,
                            LayoutEdit::Split(LayoutKind::Vertical),
                            Away::only(&away),
                        )
                        .discard();
                }
                system.update(WindowChange::Add(layout, window, None), Away::only(&away));
            }
            system.focus(layout, Focus::Select(windows[1]), Away::only(&away)).handled;
            system
                .edit(
                    layout,
                    LayoutEdit::Resize {
                        amount: 0.13,
                        orientation: ResizeOrientation::Horizontal,
                    },
                    Away::only(&away),
                )
                .discard();
            system.focus(layout, Focus::Select(windows[0]), Away::only(&away)).handled;
            let slots: Vec<_> = windows.iter().map(|&w| system.window_slot(layout, w)).collect();
            let original = frames(system, &away);
            for &window in &windows[1..] {
                set_away(system, &mut away, window, true);
            }
            let projected = frames(system, &away);
            assert_eq!(
                projected.iter().map(|(w, _)| *w).collect::<Vec<_>>(),
                vec![windows[0]],
                "{mode}"
            );
            if mode != "scrolling" && mode != "floating" {
                assert_eq!(
                    projected[0].1,
                    crate::layout_engine::utils::compute_tiling_area(screen, &gaps),
                    "{mode}: an inactive subtree cannot reserve geometry"
                );
            }
            for index in [2, 1, 3] {
                set_away(system, &mut away, windows[index], false);
            }
            assert_eq!(frames(system, &away), original, "{mode}");
            assert_eq!(
                windows.iter().map(|&w| system.window_slot(layout, w)).collect::<Vec<_>>(),
                slots
            );

            for &window in &windows {
                set_away(system, &mut away, window, true);
            }
            assert!(frames(system, &away).is_empty());
            for &window in windows.iter().rev() {
                set_away(system, &mut away, window, false);
            }
            assert_eq!(
                windows.iter().map(|&w| system.window_slot(layout, w)).collect::<Vec<_>>(),
                slots
            );

            let returned = frames(system, &away);
            assert_eq!(
                returned.iter().map(|(w, frame)| (*w, frame.size)).collect::<Vec<_>>(),
                original.iter().map(|(w, frame)| (*w, frame.size)).collect::<Vec<_>>(),
                "{mode}"
            );

            set_away(system, &mut away, windows[1], true);
            assert!(!system.focus(layout, Focus::Select(windows[1]), Away::only(&away)).handled);
            assert!(
                !system
                    .edit(
                        layout,
                        LayoutEdit::Swap(windows[0], windows[1]),
                        Away::only(&away)
                    )
                    .changed()
            );
            let replacement = WindowId::new(1, 20);
            system.update(WindowChange::Replace(windows[1], replacement), Away::only(&away));
            set_away(system, &mut away, windows[1], false);
            set_away(system, &mut away, replacement, true);
            assert_eq!(system.window_slot(layout, replacement), slots[1]);
            system.update(WindowChange::Remove(windows[2]), Away::only(&away));
            let inserted = WindowId::new(1, 5);
            system.update(WindowChange::Add(layout, inserted, None), Away::only(&away));
            assert!(
                system.contains_window(layout, replacement),
                "inventory must retain suspended members"
            );
            assert!(!system.contains_window(layout, windows[2]));
            let serialized = ron::ser::to_string(&system).unwrap();
            let mut restarted: LayoutSystemKind = ron::de::from_str(&serialized).unwrap();
            assert!(restarted.contains_window(layout, replacement));
            assert!(!restarted.contains_window(layout, windows[1]));
            restarted.update(WindowChange::CloseApp(1), Away::only(&away));
            assert!(restarted.all_windows(layout).is_empty());
            set_away(system, &mut away, replacement, false);
            system.update(WindowChange::Remove(replacement), Away::only(&away));
            assert!(!system.contains_window(layout, replacement));
            assert!(system.contains_window(layout, inserted));
            let retained = system.all_windows(layout);
            for &window in &retained {
                set_away(system, &mut away, window, true);
            }
            let newcomer = WindowId::new(1, 6);
            system.update(WindowChange::Add(layout, newcomer, None), Away::only(&away));
            assert_eq!(
                frames(system, &away).iter().map(|(w, _)| *w).collect::<Vec<_>>(),
                vec![newcomer]
            );
            for &window in &retained {
                set_away(system, &mut away, window, false);
                assert!(system.contains_window(layout, window));
            }
        }
    }
}

// Inspection is deliberately outside the ordinary layout interface.
impl LayoutSystemKind {
    pub fn draw_tree(&self, layout: LayoutId) -> String {
        match self {
            Self::Traditional(system) => system.tiles.draw_tree(layout, &[]),
            Self::Bsp(system) => system.draw_tree(layout),
            Self::Stack(system) => system.draw_tree(layout),
            Self::MasterStack(system) => system.draw_tree(layout),
            Self::Scrolling(system) => system.draw_tree(layout),
            Self::Floating(system) => system.groups.draw_tree(layout, &[]),
        }
    }

    pub fn container_tree(
        &self,
        layout: LayoutId,
        away: Away<'_>,
    ) -> rift_protocol::ContainerTreeNode {
        match self {
            Self::Traditional(system) => system.tiles.container_tree(layout, away),
            Self::Bsp(system) => system.container_tree(layout),
            Self::Stack(system) => system.container_tree(layout),
            Self::MasterStack(system) => system.container_tree(layout, away),
            Self::Scrolling(system) => system.container_tree(layout, away),
            Self::Floating(system) => system.groups.container_tree(layout, away),
        }
    }

    pub fn window_slot(&self, layout: LayoutId, window: WindowId) -> Option<Vec<usize>> {
        match self {
            Self::Traditional(system) => system.tiles.window_slot(layout, window),
            Self::Bsp(system) => system.window_slot(layout, window),
            Self::Stack(system) => system.window_slot(layout, window),
            Self::MasterStack(system) => system.tiles.window_slot(layout, window),
            Self::Scrolling(system) => system.window_slot(layout, window),
            Self::Floating(system) => system.groups.window_slot(layout, window),
        }
    }
}

impl LayoutSystemKind {
    pub(crate) fn project_groups(
        &self,
        layout: LayoutId,
        screen: CGRect,
        stack_offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &GapSettings,
        stack_line: crate::common::config::StackLineReserve,
        emit_frames: bool,
        groups: &mut Vec<crate::layout_engine::engine::GroupContainerInfo>,
        selection_path_only: bool,
        away: Away<'_>,
    ) -> Option<Vec<(WindowId, CGRect)>> {
        let groups = tiles::GroupOutput { groups, selection_path_only };
        match self {
            Self::Traditional(system) => {
                let root = system.tiles.root(layout);
                Some(tiles::project(
                    &system.tiles,
                    root,
                    |_| root,
                    screen,
                    stack_offset,
                    constraints,
                    gaps,
                    stack_line,
                    emit_frames,
                    Some(groups),
                    away,
                ))
            }
            Self::MasterStack(system) => Some(system.project_groups(
                layout,
                screen,
                stack_offset,
                constraints,
                gaps,
                stack_line,
                emit_frames,
                groups,
                away,
            )),
            _ => None,
        }
    }

    pub(crate) fn collect_group_containers(
        &self,
        layout_id: LayoutId,
        screen: CGRect,
        stack_offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &GapSettings,
        stack_line: crate::common::config::StackLineReserve,
        selection_path_only: bool,
        away: Away<'_>,
    ) -> Vec<crate::layout_engine::engine::GroupContainerInfo> {
        let mut groups = Vec::new();
        if self
            .project_groups(
                layout_id,
                screen,
                stack_offset,
                constraints,
                gaps,
                stack_line,
                false,
                &mut groups,
                selection_path_only,
                away,
            )
            .is_some()
        {
            return groups;
        }
        match self {
            LayoutSystemKind::Stack(s) => s.collect_group_containers(
                layout_id,
                screen,
                stack_offset,
                gaps,
                stack_line,
                selection_path_only,
                away,
            ),
            LayoutSystemKind::Floating(s) => {
                s.collect_group_containers(layout_id, screen, gaps, selection_path_only, away)
            }
            _ => Vec::new(),
        }
    }
}
