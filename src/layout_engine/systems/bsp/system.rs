//! rift's BSP layout system: the core's [`Tree`] decides and is saved (layout trees: the core's
//! tree since slice 4, upstream's system deleted in slice 5). Under `roc-core` the Roc tree
//! decides instead, with the Rust tree its shadow (`roc_tree`). Loaded through the saved form's
//! load (`stored`), saved in the same form, byte for byte.

use std::collections::BTreeSet;

use objc2_core_foundation::CGRect;
use rift_core::layout::bsp_tree::{
    self as core, Content, InsertionPoint, Kind, Layout, Leaf, Members, Memory, Parent,
    Participation, Preselect, Query, Resize, Side, Split, Tabs, Tree,
};
#[cfg(feature = "roc-core")]
use rift_core_roc::bsp_tree::{Answer as RocAnswer, Event as RocEvent};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use slotmap::SlotMap;

#[cfg(feature = "roc-core")]
use super::roc_tree::{self as roc, RocTree};
use super::stored::{LayoutState, LeafMembers, Loaded, Node, NodeKind, StoredBspSystem};
use crate::actor::app::{WindowId, pid_t};
use crate::common::collections::HashMap;
use crate::common::config::WindowInsertionPoint;
use crate::layout_engine::systems::{
    EditResult, Focus, FocusResult, LayoutEdit, LayoutSystem, WindowChange,
    WindowLayoutConstraints, WindowQuery,
};
use crate::layout_engine::{Direction, LayoutId, Orientation};
use crate::model::slot_table::{SavedSlot, SlotKey, SlotTable, saved};
use crate::model::tree::NodeId;

/// The core's tree over rift's ids.
pub(super) type CoreTree = Tree<LayoutId, NodeId, WindowId>;

/// The BSP layout system: the core's tree, and under `roc-core` the deciding Roc tree.
#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "StoredBspSystem")]
pub struct BspLayoutSystem {
    /// The deciding tree, or under `roc-core` the Roc tree's shadow (re-seeded from it on any
    /// difference, compared on every operation); saved either way.
    pub(super) core: CoreTree,
    /// Under `roc-core`, the deciding tree.
    #[cfg(feature = "roc-core")]
    pub(super) roc: RocTree,
}

/// The core's tree in the saved form (the field names and order upstream's system wrote), byte
/// for byte.
#[derive(Serialize)]
#[serde(rename = "Upstream")]
struct Saved {
    layouts: Vec<SavedSlot<LayoutState>>,
    nodes: Vec<SavedSlot<Node>>,
}

impl Serialize for BspLayoutSystem {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Saved {
            layouts: saved(self.core.layouts(), |_, layout| stored_layout(layout)).collect(),
            nodes: saved(self.core.nodes(), |_, node| stored_node(node)).collect(),
        }
        .serialize(serializer)
    }
}

impl TryFrom<StoredBspSystem> for BspLayoutSystem {
    type Error = String;

    /// A saved system loaded into the core's tree (the settings are not saved: rift sets them
    /// again).
    fn try_from(stored: StoredBspSystem) -> Result<Self, String> {
        let loaded = Loaded::try_from(stored)?;
        Ok(Self::holding(core_of_tables(
            &loaded.layouts,
            &loaded.nodes,
            insertion_point(WindowInsertionPoint::default()),
        )))
    }
}

impl Default for BspLayoutSystem {
    fn default() -> Self { Self::new(WindowInsertionPoint::default()) }
}

impl BspLayoutSystem {
    pub fn new(window_insertion_point: WindowInsertionPoint) -> Self {
        Self::holding(CoreTree::new(insertion_point(window_insertion_point)))
    }

    /// The system holding `core` (under `roc-core`, the Roc tree seeded from it).
    fn holding(core: CoreTree) -> Self {
        Self {
            #[cfg(feature = "roc-core")]
            roc: RocTree::seeded(&core),
            core,
        }
    }

    pub fn set_window_insertion_point(&mut self, value: WindowInsertionPoint) {
        self.core.set_window_insertion_point(insertion_point(value));
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "set_window_insertion_point",
            RocEvent::Insertion(insertion_point(value)),
            RocAnswer::Done,
        );
    }

    pub fn set_single_window_aspect_ratio(&mut self, ratio: Option<f64>) {
        self.core.set_single_window_aspect_ratio(ratio);
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "set_single_window_aspect_ratio",
            RocEvent::AspectRatio(ratio),
            RocAnswer::Done,
        );
    }

    pub(in crate::layout_engine) fn can_move_window(
        &self,
        layout: LayoutId,
        window: WindowId,
        direction: Direction,
    ) -> bool {
        let core = self.core.can_move_window(layout, window, core_direction(direction));
        #[cfg(feature = "roc-core")]
        let core = roc::decided(
            self.roc_asked(
                "can_move_window",
                RocEvent::CanMoveWindow(
                    roc::layout_key(layout),
                    roc::window(window),
                    core_direction(direction),
                ),
                RocAnswer::Verdict(core),
            ),
            roc::as_verdict,
            core,
        );
        core
    }

    /// The layout drawn as text (rift-cli's debug view), from the core's tree.
    pub(crate) fn draw_tree(&self, layout: LayoutId) -> String { self.core.draw(layout) }

    /// The layout's nodes as rift-cli's container query shows them, from the core's tree.
    pub(crate) fn container_tree(&self, layout: LayoutId) -> rift_protocol::ContainerTreeNode {
        self.core
            .container(layout)
            .map_or_else(Default::default, |root| container_node(&root))
    }

    pub(crate) fn window_slot(&self, layout: LayoutId, window: WindowId) -> Option<Vec<usize>> {
        let core = self.core.window_slot(layout, window);
        #[cfg(feature = "roc-core")]
        let core = roc::decided(
            self.roc_asked(
                "window_slot",
                RocEvent::WindowSlot(roc::layout_key(layout), roc::window(window)),
                RocAnswer::WindowSlot(core.clone()),
            ),
            roc::as_slot,
            core,
        );
        core
    }

    /// Under `roc-core`, an operation the Rust tree took (answering `answer`): the Roc tree
    /// takes it and decides (a difference re-seeds the Rust tree from it); its answer is rift's.
    #[cfg(feature = "roc-core")]
    fn roc_stepped(&mut self, what: &str, event: RocEvent, answer: RocAnswer) -> RocAnswer {
        self.roc.decided(&mut self.core, what, event, answer)
    }

    /// Under `roc-core`, a query the Rust tree answered `answer`: the Roc tree's answer is
    /// rift's.
    #[cfg(feature = "roc-core")]
    fn roc_asked(&self, what: &str, event: RocEvent, answer: RocAnswer) -> RocAnswer {
        self.roc.asked(what, event, answer)
    }
}

impl LayoutSystem for BspLayoutSystem {
    fn has_windows_for_app(&self, layout: LayoutId, pid: pid_t) -> bool {
        let core = self.core.has_windows_for_app(layout, |window| window.pid == pid);
        #[cfg(feature = "roc-core")]
        let core = roc::decided(
            self.roc_asked(
                "has_windows_for_app",
                RocEvent::HasWindowsForApp(
                    roc::layout_key(layout),
                    roc::app_windows(&self.core, |window| window.pid == pid),
                ),
                RocAnswer::Verdict(core),
            ),
            roc::as_verdict,
            core,
        );
        core
    }

    fn window_participates(&self, window: WindowId) -> bool {
        let core = self.core.window_participates(window);
        #[cfg(feature = "roc-core")]
        let core = roc::decided(
            self.roc_asked(
                "window_participates",
                RocEvent::WindowParticipates(roc::window(window)),
                RocAnswer::Verdict(core),
            ),
            roc::as_verdict,
            core,
        );
        core
    }

    fn set_window_participation(&mut self, window: WindowId, participates: bool) -> bool {
        let participation = if participates {
            Participation::TakesPart
        } else {
            Participation::LeftOut
        };
        let core = self.core.set_window_participation(window, participation);
        #[cfg(feature = "roc-core")]
        let core = roc::decided(
            self.roc_stepped(
                "set_window_participation",
                RocEvent::Participation(roc::window(window), participation),
                RocAnswer::Participated(core),
            ),
            roc::as_participated,
            core,
        );
        core
    }

    fn create_layout(&mut self) -> LayoutId {
        let core = self.core.create_layout();
        #[cfg(feature = "roc-core")]
        let core = roc::decided(
            self.roc_stepped(
                "create_layout",
                RocEvent::CreateLayout,
                RocAnswer::Layout(roc::layout_key(core)),
            ),
            roc::as_layout,
            core,
        );
        core
    }

    fn contains_layout(&self, layout: LayoutId) -> bool {
        let core = self.core.contains_layout(layout);
        #[cfg(feature = "roc-core")]
        let core = roc::decided(
            self.roc_asked(
                "contains_layout",
                RocEvent::ContainsLayout(roc::layout_key(layout)),
                RocAnswer::Verdict(core),
            ),
            roc::as_verdict,
            core,
        );
        core
    }

    fn clone_layout(&mut self, layout: LayoutId) -> LayoutId {
        let core = self.core.clone_layout(layout);
        #[cfg(feature = "roc-core")]
        let core = roc::decided(
            self.roc_stepped(
                "clone_layout",
                RocEvent::CloneLayout(roc::layout_key(layout)),
                RocAnswer::Layout(roc::layout_key(core)),
            ),
            roc::as_layout,
            core,
        );
        core
    }

    fn remove_layout(&mut self, layout: LayoutId) {
        self.core.remove_layout(layout);
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "remove_layout",
            RocEvent::RemoveLayout(roc::layout_key(layout)),
            RocAnswer::Done,
        );
    }

    /// The core's frames: under `roc-core` the Roc tree's own shape through the Roc geometry,
    /// with the Rust geometry's frames for the Rust tree's shape the shadow; else the Rust tree's
    /// shape through the Rust geometry. A BSP layout has no system geometry of its own to
    /// shadow, so `shadow` is not read.
    fn calculate_layout(
        &self,
        layout: LayoutId,
        screen: CGRect,
        _stack_offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &crate::common::config::GapSettings,
        _stack_line: crate::common::config::StackLineReserve,
        _shadow: crate::layout_engine::Shadow,
    ) -> Vec<(WindowId, CGRect)> {
        use crate::layout_engine::core_geometry::core_constraints;
        use crate::layout_engine::{core_gaps, core_rect};
        let core = self.core.shape(layout, core_rect(screen), core_gaps(gaps), |window| {
            constraints.get(&window).copied().map(core_constraints)
        });
        // Under `roc-core` the Roc tree's own shape through the Roc geometry decides.
        #[cfg(feature = "roc-core")]
        return crate::layout_engine::core_geometry::decide_bsp_tree(core, || {
            use crate::roc_path::Word;
            let known: Vec<_> = constraints
                .iter()
                .map(|(&window, &held)| (window.word(), core_constraints(held)))
                .collect();
            self.roc.frames(layout, core_rect(screen), core_gaps(gaps), &known)
        });
        #[cfg(not(feature = "roc-core"))]
        crate::layout_engine::core_geometry::decide_bsp(core)
    }

    fn selected_window(&self, layout: LayoutId) -> Option<WindowId> {
        let core = self.core.selected_window(layout);
        #[cfg(feature = "roc-core")]
        let core = roc::decided(
            self.roc_asked(
                "selected_window",
                RocEvent::SelectedWindow(roc::layout_key(layout)),
                RocAnswer::Window(core.map(roc::window)),
            ),
            roc::as_window,
            core,
        );
        core
    }

    fn window_in_direction(&self, layout: LayoutId, direction: Direction) -> Option<WindowId> {
        let core = self.core.window_in_direction(layout, core_direction(direction));
        #[cfg(feature = "roc-core")]
        let core = roc::decided(
            self.roc_asked(
                "window_in_direction",
                RocEvent::WindowInDirection(roc::layout_key(layout), core_direction(direction)),
                RocAnswer::Window(core.map(roc::window)),
            ),
            roc::as_window,
            core,
        );
        core
    }

    fn contains_window(&self, layout: LayoutId, wid: WindowId) -> bool {
        let core = self.core.contains_window(layout, wid);
        #[cfg(feature = "roc-core")]
        let core = roc::decided(
            self.roc_asked(
                "contains_window",
                RocEvent::ContainsWindow(roc::layout_key(layout), roc::window(wid)),
                RocAnswer::Verdict(core),
            ),
            roc::as_verdict,
            core,
        );
        core
    }

    fn has_any_fullscreen_node(&self, layout: LayoutId) -> bool {
        let core = self.core.has_any_fullscreen_node(layout);
        #[cfg(feature = "roc-core")]
        let core = roc::decided(
            self.roc_asked(
                "has_any_fullscreen_node",
                RocEvent::HasAnyFullscreenNode(roc::layout_key(layout)),
                RocAnswer::Verdict(core),
            ),
            roc::as_verdict,
            core,
        );
        core
    }

    fn windows(&self, layout: LayoutId, query: WindowQuery) -> Vec<WindowId> {
        let core_query = match query {
            WindowQuery::All => Query::All,
            WindowQuery::Visible => Query::Visible,
            WindowQuery::Selection => Query::Selection,
            WindowQuery::Stack(window) => Query::Stack(window),
        };
        let core = self.core.windows(layout, core_query);
        #[cfg(feature = "roc-core")]
        let core = roc::decided(
            self.roc_asked(
                "windows",
                RocEvent::Windows(roc::layout_key(layout), roc::query(core_query)),
                RocAnswer::Windows(roc::windows(&core)),
            ),
            roc::as_windows,
            core,
        );
        core
    }

    fn focus(&mut self, layout: LayoutId, focus: Focus) -> FocusResult {
        let core_focus = match focus {
            Focus::Select(window) => core::Focus::Select(window),
            Focus::Ascend => core::Focus::Ascend,
            Focus::Descend => core::Focus::Descend,
            Focus::Direction(direction) => core::Focus::Direction(core_direction(direction)),
        };
        let focused = self.core.focus(layout, core_focus);
        #[cfg(feature = "roc-core")]
        let focused = roc::decided(
            self.roc_stepped(
                "focus",
                RocEvent::Focus(roc::layout_key(layout), roc::focus(core_focus)),
                RocAnswer::Focused(roc::focused(&focused)),
            ),
            roc::as_focused,
            focused,
        );
        match focused {
            core::Focused::Selection { handled } => FocusResult::selection(handled),
            core::Focused::Movement { window, raise } => FocusResult::movement((window, raise)),
        }
    }

    fn update(&mut self, change: WindowChange<'_>) {
        use crate::layout_engine::core_rect;
        let resize = |old, new, screen, gaps| Resize {
            old: core_rect(old),
            new: core_rect(new),
            screen: core_rect(screen),
            tiling: core_rect(crate::layout_engine::utils::compute_tiling_area(screen, gaps)),
        };
        // The Roc tree's event, read before the change (an app's windows are gone after).
        #[cfg(feature = "roc-core")]
        let roc_event = match change {
            WindowChange::Add(layout, window, _) => {
                RocEvent::AddWindow(roc::layout_key(layout), roc::window(window))
            }
            WindowChange::Remove(window) | WindowChange::RemoveAndRebalance(window) => {
                RocEvent::RemoveWindow(roc::window(window))
            }
            WindowChange::Replace(from, to) => RocEvent::ReplaceWindow {
                from: roc::window(from),
                to: roc::window(to),
            },
            WindowChange::CloseApp(pid) => {
                RocEvent::CloseApp(roc::app_windows(&self.core, |window| window.pid == pid))
            }
            WindowChange::Resize {
                layout,
                window,
                old,
                new,
                screen,
                gaps,
            } => RocEvent::ResizeWindow(
                roc::layout_key(layout),
                roc::window(window),
                resize(old, new, screen, gaps),
            ),
        };
        let core = &mut self.core;
        match change {
            WindowChange::Add(layout, window, _) => core.add_window(layout, window),
            WindowChange::Remove(window) | WindowChange::RemoveAndRebalance(window) => {
                core.remove_window(window)
            }
            WindowChange::Replace(from, to) => core.replace_window(from, to),
            WindowChange::CloseApp(pid) => core.close_app(|window| window.pid == pid),
            WindowChange::Resize {
                layout,
                window,
                old,
                new,
                screen,
                gaps,
            } => core.resize_window(layout, window, resize(old, new, screen, gaps)),
        }
        #[cfg(feature = "roc-core")]
        self.roc_stepped("update", roc_event, RocAnswer::Done);
    }

    fn edit_layout(&mut self, layout: LayoutId, edit: LayoutEdit) -> EditResult {
        match core_edit(edit) {
            Some(edit) => {
                let edited = self.core.edit(layout, edit);
                #[cfg(feature = "roc-core")]
                let edited = roc::decided(
                    self.roc_stepped(
                        "edit_layout",
                        RocEvent::Edit(roc::layout_key(layout), roc::edit(edit)),
                        RocAnswer::Edited(roc::edited(&edited)),
                    ),
                    roc::as_edited,
                    edited,
                );
                match edited {
                    core::Edited::Unsupported => EditResult::Unsupported,
                    core::Edited::Unchanged => EditResult::Unchanged,
                    core::Edited::Changed => EditResult::Changed,
                    core::Edited::Raise(windows) => EditResult::Raise(windows),
                }
            }
            // An edit the BSP trees have no case for (upstream's system had none).
            None => EditResult::Unsupported,
        }
    }
}

/// A core container as rift-cli's container node: a split with its orientation and children; a
/// leaf as its window, a horizontal stack of its windows (each selected when the leaf is and it
/// is on top), or a placeholder, with its fill and preselection.
fn container_node(
    container: &core::Container<NodeId, WindowId>,
) -> rift_protocol::ContainerTreeNode {
    use rift_protocol::{ContainerNodeType, ContainerTreeNode, LayoutKind};
    let mut node = ContainerTreeNode {
        node_id: slotmap::Key::data(&container.node).as_ffi(),
        is_selected: container.selected,
        weight: container.share,
        ..Default::default()
    };
    match &container.kind {
        core::ContainerKind::Split { orientation, children } => {
            node.layout_kind = Some(LayoutKind::from(engine_orientation(*orientation)));
            node.children = children.iter().map(|child| container_node(child)).collect();
        }
        core::ContainerKind::Leaf(leaf) => {
            node.pending_split = match leaf.preselect {
                Preselect::Unset => None,
                Preselect::Toward(direction) => Some(engine_direction(direction)),
            };
            match &leaf.content {
                Content::Empty => node.node_type = ContainerNodeType::Placeholder,
                Content::Held { members, fill } => {
                    (node.is_fullscreen, node.is_fullscreen_within_gaps) = match fill {
                        core::Fill::Tiled => (false, false),
                        core::Fill::Screen => (true, false),
                        core::Fill::WithinGaps => (false, true),
                    };
                    match members {
                        Members::Window(window) => {
                            node.node_type = ContainerNodeType::Window;
                            node.window_id = Some((*window).into());
                        }
                        Members::Stack(tabs) => {
                            node.layout_kind = Some(LayoutKind::HorizontalStack);
                            node.children = tabs
                                .windows()
                                .map(|member| ContainerTreeNode {
                                    node_id: (u64::from(member.pid as u32) << 32)
                                        | u64::from(member.idx.get()),
                                    node_type: ContainerNodeType::Window,
                                    window_id: Some(member.into()),
                                    is_selected: container.selected && member == tabs.top,
                                    is_fullscreen: node.is_fullscreen,
                                    is_fullscreen_within_gaps: node.is_fullscreen_within_gaps,
                                    ..Default::default()
                                })
                                .collect();
                        }
                    }
                }
            }
        }
    }
    node
}

/// An edit as the core takes it; none for those the BSP trees have no case for (as upstream's
/// system had none).
fn core_edit(edit: LayoutEdit) -> Option<core::Edit<WindowId>> {
    use crate::layout_engine::systems::GroupEdit;
    use crate::layout_engine::{LayoutKind, ResizeOrientation, WindowDropAction};
    Some(match edit {
        LayoutEdit::Move { direction, .. } => core::Edit::Move(core_direction(direction)),
        LayoutEdit::Swap(a, b) => core::Edit::Swap(a, b),
        LayoutEdit::Drop { source, target, action } => core::Edit::Drop {
            source,
            target,
            action: match action {
                WindowDropAction::Swap => core::DropAction::Swap,
                WindowDropAction::Stack => core::DropAction::Stack,
                WindowDropAction::Insert(direction) => {
                    core::DropAction::Insert(core_direction(direction))
                }
                WindowDropAction::Move(direction) => {
                    core::DropAction::Move(core_direction(direction))
                }
                WindowDropAction::ConsumeOrExpel(direction) => {
                    core::DropAction::ConsumeOrExpel(core_direction(direction))
                }
            },
        },
        LayoutEdit::Split(kind) => core::Edit::Split(match kind {
            LayoutKind::Horizontal => core::SplitKind::Tiled(core::Orientation::Horizontal),
            LayoutKind::Vertical => core::SplitKind::Tiled(core::Orientation::Vertical),
            LayoutKind::HorizontalStack | LayoutKind::VerticalStack => core::SplitKind::Stacked,
        }),
        LayoutEdit::Fullscreen { within_gaps } => core::Edit::Fullscreen { within_gaps },
        LayoutEdit::Resize { amount, orientation } => core::Edit::Resize {
            amount,
            along: match orientation {
                ResizeOrientation::Horizontal => core::ResizeAlong::Horizontal,
                ResizeOrientation::Vertical => core::ResizeAlong::Vertical,
                ResizeOrientation::Smart => core::ResizeAlong::Smart,
            },
        },
        LayoutEdit::Rebalance => core::Edit::Rebalance,
        LayoutEdit::ToggleOrientation => core::Edit::ToggleOrientation,
        LayoutEdit::Group(GroupEdit::Join(direction)) => {
            core::Edit::Join(core_direction(direction))
        }
        LayoutEdit::Group(GroupEdit::ConsumeOrExpel(direction)) => {
            core::Edit::ConsumeOrExpel(core_direction(direction))
        }
        LayoutEdit::Group(GroupEdit::Unjoin) => core::Edit::Unjoin,
        LayoutEdit::ReorderTab { .. }
        | LayoutEdit::Group(GroupEdit::Stack(_) | GroupEdit::Unstack(_)) => return None,
    })
}

/// A loaded system's tables as the core's tree: each slotmap's slots, versions and free list
/// (no window left out, no aspect ratio: those are not saved).
fn core_of_tables(
    layouts: &SlotMap<LayoutId, LayoutState>,
    nodes: &SlotMap<NodeId, Node>,
    insertion: InsertionPoint,
) -> CoreTree {
    let dummy_node = Node {
        parent: None,
        kind: NodeKind::Leaf {
            members: LeafMembers::Empty,
            fullscreen: false,
            fullscreen_within_gaps: false,
            preselected: None,
        },
    };
    let dummy_layout = LayoutState { root: NodeId::default() };
    CoreTree::from_parts(
        table(layouts, dummy_layout, |state| Layout { root: state.root }),
        table(nodes, dummy_node, core_node),
        BTreeSet::new(),
        insertion,
        None,
    )
}

/// A live slotmap as the core's table: its slots, vacant ones and their versions included (read
/// from its saved form), and its free list (the order a copy hands its vacant slots out; a load
/// rebuilds it otherwise).
fn table<K: slotmap::Key + SlotKey, V: Clone + Serialize + DeserializeOwned, C>(
    map: &SlotMap<K, V>,
    dummy: V,
    convert: impl Fn(&V) -> C,
) -> SlotTable<K, C> {
    let written = ron::to_string(map).expect("a slotmap is written");
    let saved: Vec<SavedSlot<V>> = ron::from_str(&written).expect("a written slotmap is read");
    let vacant = saved.len() - 1 - map.len();
    let mut copy = map.clone();
    let order: Vec<u32> = (0..vacant).map(|_| copy.insert(dummy.clone()).parts().0).collect();
    crate::model::slot_table::parse::<K, V>(saved)
        .expect("a written slotmap is a slot sequence")
        .map(|_, value| Ok::<_, std::convert::Infallible>(convert(&value)))
        .unwrap_or_else(|never| match never {})
        .resumed(&order)
        .expect("a slotmap's free list is its vacant slots")
}

fn insertion_point(value: WindowInsertionPoint) -> InsertionPoint {
    match value {
        WindowInsertionPoint::NextToSelection => InsertionPoint::NextToSelection,
        WindowInsertionPoint::EndOfTree => InsertionPoint::EndOfTree,
    }
}

fn core_orientation(orientation: Orientation) -> core::Orientation {
    match orientation {
        Orientation::Horizontal => core::Orientation::Horizontal,
        Orientation::Vertical => core::Orientation::Vertical,
    }
}

fn core_direction(direction: Direction) -> core::Direction {
    match direction {
        Direction::Left => core::Direction::Left,
        Direction::Right => core::Direction::Right,
        Direction::Up => core::Direction::Up,
        Direction::Down => core::Direction::Down,
    }
}

/// A saved leaf's two fullscreen flags as one fill (`fullscreen` first, as upstream's projection
/// read them; the load warns of a file holding both).
fn core_fill(fullscreen: bool, within_gaps: bool) -> core::Fill {
    match (fullscreen, within_gaps) {
        (true, _) => core::Fill::Screen,
        (false, true) => core::Fill::WithinGaps,
        (false, false) => core::Fill::Tiled,
    }
}

/// A saved node as the core holds it.
fn core_node(node: &Node) -> core::Node<NodeId, WindowId> {
    let kind = match &node.kind {
        NodeKind::Split {
            orientation,
            ratio,
            children,
            selected,
        } => Kind::Split(Split {
            orientation: core_orientation(*orientation),
            ratio: *ratio,
            children: *children,
            memory: match selected {
                None => Memory::Stops,
                Some(false) => Memory::Toward(Side::First),
                Some(true) => Memory::Toward(Side::Second),
            },
        }),
        NodeKind::Leaf {
            members,
            fullscreen,
            fullscreen_within_gaps,
            preselected,
        } => Kind::Leaf(Leaf {
            content: core_content(members, core_fill(*fullscreen, *fullscreen_within_gaps)),
            preselect: preselected.map_or(Preselect::Unset, |direction| {
                Preselect::Toward(core_direction(direction))
            }),
        }),
    };
    core::Node {
        parent: node.parent.map_or(Parent::Root, Parent::Split),
        kind,
    }
}

/// A saved leaf's members as the core holds them (an empty leaf holds no fill; the load warns of
/// a file whose empty leaf has a flag).
fn core_content(members: &LeafMembers, fill: core::Fill) -> Content<WindowId> {
    match members {
        LeafMembers::Empty => Content::Empty,
        LeafMembers::Single(window) => Content::Held {
            members: Members::Window(*window),
            fill,
        },
        LeafMembers::Stack { windows, active } => Content::Held {
            members: Members::Stack(Tabs {
                before: windows[..*active].to_vec(),
                top: windows[*active],
                after: windows[*active + 1..].to_vec(),
            }),
            fill,
        },
    }
}

fn stored_layout(layout: &Layout<NodeId>) -> LayoutState { LayoutState { root: layout.root } }

fn engine_orientation(orientation: core::Orientation) -> Orientation {
    match orientation {
        core::Orientation::Horizontal => Orientation::Horizontal,
        core::Orientation::Vertical => Orientation::Vertical,
    }
}

fn engine_direction(direction: core::Direction) -> Direction {
    match direction {
        core::Direction::Left => Direction::Left,
        core::Direction::Right => Direction::Right,
        core::Direction::Up => Direction::Up,
        core::Direction::Down => Direction::Down,
    }
}

/// A core node as rift saves it: the inverse of [`core_node`].
fn stored_node(node: &core::Node<NodeId, WindowId>) -> Node {
    let kind = match &node.kind {
        Kind::Split(split) => NodeKind::Split {
            orientation: engine_orientation(split.orientation),
            ratio: split.ratio,
            children: split.children,
            selected: match split.memory {
                Memory::Stops => None,
                Memory::Toward(Side::First) => Some(false),
                Memory::Toward(Side::Second) => Some(true),
            },
        },
        Kind::Leaf(leaf) => {
            let (members, (fullscreen, fullscreen_within_gaps)) = match &leaf.content {
                Content::Empty => (LeafMembers::Empty, (false, false)),
                Content::Held { members, fill } => (
                    match members {
                        Members::Window(window) => LeafMembers::Single(*window),
                        Members::Stack(tabs) => LeafMembers::Stack {
                            windows: tabs.windows().collect(),
                            active: tabs.before.len(),
                        },
                    },
                    match fill {
                        core::Fill::Tiled => (false, false),
                        core::Fill::Screen => (true, false),
                        core::Fill::WithinGaps => (false, true),
                    },
                ),
            };
            NodeKind::Leaf {
                members,
                fullscreen,
                fullscreen_within_gaps,
                preselected: match leaf.preselect {
                    Preselect::Unset => None,
                    Preselect::Toward(direction) => Some(engine_direction(direction)),
                },
            }
        }
    };
    Node {
        parent: match node.parent {
            Parent::Root => None,
            Parent::Split(parent) => Some(parent),
        },
        kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(idx: u32) -> WindowId { WindowId::new(1, idx) }

    /// A leaf with a preselected direction (only a saved file holds one) is split that way by
    /// the next window, which goes on that side and is selected; a window after it goes next to
    /// the selection as usual.
    #[test]
    fn a_preselected_direction_puts_the_next_window_on_that_side() {
        let mut system = BspLayoutSystem::new(WindowInsertionPoint::NextToSelection);
        let layout = system.create_layout();
        system.update(WindowChange::Add(layout, w(1), None));
        let file = written_as(
            &ron::to_string(&system).unwrap(),
            "preselected:None",
            "preselected:Some(left)",
        );
        let mut system: BspLayoutSystem = ron::from_str(&file).unwrap();
        system.update(WindowChange::Add(layout, w(2), None));
        assert_eq!(system.window_slot(layout, w(2)), Some(vec![0]));
        assert_eq!(system.window_slot(layout, w(1)), Some(vec![1]));
        assert_eq!(system.selected_window(layout), Some(w(2)));
    }

    /// Unjoining a stack the selection reached past a left-out window puts the stack's windows
    /// back where the stack was, not beside the left-out window.
    #[test]
    fn an_unjoin_reached_past_a_left_out_window_stays_in_the_stacks_place() {
        use crate::layout_engine::WindowDropAction;
        use crate::layout_engine::systems::GroupEdit;
        let mut system = BspLayoutSystem::default();
        let layout = system.create_layout();
        for idx in 1..=3 {
            system.update(WindowChange::Add(layout, w(idx), None));
        }
        system.edit(layout, LayoutEdit::Drop {
            source: w(3),
            target: w(2),
            action: WindowDropAction::Stack,
        });
        assert_eq!(system.window_slot(layout, w(2)), Some(vec![1, 0]));
        assert!(system.focus(layout, Focus::Select(w(1))).handled);
        system.set_window_participation(w(1), false);
        assert!(system.edit(layout, LayoutEdit::Group(GroupEdit::Unjoin)).changed());
        assert_eq!(system.window_slot(layout, w(1)), Some(vec![0]));
        assert_eq!(system.window_slot(layout, w(2)), Some(vec![1, 0]));
        assert_eq!(system.window_slot(layout, w(3)), Some(vec![1, 1]));
    }

    /// A file holding `corrupt` (a state only a saved file holds) loads as `normalized` holds
    /// it, and is saved as `normalized`: the bytes rift writes.
    fn loads_normalized(corrupt: &str, normalized: &str) {
        assert_ne!(corrupt, normalized);
        let loaded: BspLayoutSystem = ron::from_str(corrupt).unwrap();
        assert_eq!(ron::to_string(&loaded).unwrap(), normalized);
    }

    /// `file` with `from` (which it holds once) written as `to`.
    fn written_as(file: &str, from: &str, to: &str) -> String {
        assert_eq!(file.matches(from).count(), 1, "{from} in {file}");
        file.replace(from, to)
    }

    /// A saved leaf with both fullscreen flags loads fullscreen (as the projection reads it).
    #[test]
    fn a_saved_leaf_with_both_fullscreen_flags_loads_fullscreen() {
        let mut system = BspLayoutSystem::default();
        let layout = system.create_layout();
        system.update(WindowChange::Add(layout, w(1), None));
        system.edit_layout(layout, LayoutEdit::Fullscreen { within_gaps: false });
        let normalized = ron::to_string(&system).unwrap();
        let corrupt = written_as(
            &normalized,
            "fullscreen:true,fullscreen_within_gaps:false",
            "fullscreen:true,fullscreen_within_gaps:true",
        );
        loads_normalized(&corrupt, &normalized);
    }

    /// A saved empty leaf with a fullscreen flag loads without it.
    #[test]
    fn a_saved_empty_leaf_loads_without_fullscreen_flags() {
        let mut system = BspLayoutSystem::default();
        system.create_layout();
        let normalized = ron::to_string(&system).unwrap();
        for flags in [
            "true,fullscreen_within_gaps:false",
            "false,fullscreen_within_gaps:true",
            "true,fullscreen_within_gaps:true",
        ] {
            let corrupt = written_as(
                &normalized,
                "members:Empty,fullscreen:false,fullscreen_within_gaps:false",
                &format!("members:Empty,fullscreen:{flags}"),
            );
            loads_normalized(&corrupt, &normalized);
        }
    }

    /// A saved split whose share is not a finite number loads with a fresh split's share; a
    /// finite share stands as saved, even outside the bounds edits keep.
    #[test]
    fn a_saved_split_with_a_share_not_a_number_loads_even() {
        let mut system = BspLayoutSystem::default();
        let layout = system.create_layout();
        system.update(WindowChange::Add(layout, w(1), None));
        system.update(WindowChange::Add(layout, w(2), None));
        let normalized = ron::to_string(&system).unwrap();
        for saved in ["NaN", "inf", "-inf"] {
            let corrupt = written_as(&normalized, "ratio:0.5", &format!("ratio:{saved}"));
            loads_normalized(&corrupt, &normalized);
        }
        let wide = written_as(&normalized, "ratio:0.5", "ratio:0.99");
        let loaded: BspLayoutSystem = ron::from_str(&wide).unwrap();
        assert_eq!(ron::to_string(&loaded).unwrap(), wide);
    }

    /// The older topology-and-components form (`bsp_legacy.ron`: a split of a stack of two and
    /// a window) with `from` (which it holds once) written as `to`.
    fn legacy_as(from: &str, to: &str) -> String {
        written_as(include_str!("../bsp_legacy.ron"), from, to)
    }

    /// A legacy stack whose saved window is not one of its windows shows its last window.
    #[test]
    fn a_legacy_stack_without_its_window_shows_its_last() {
        let file = legacy_as(
            "stacks: {(idx: 2, version: 1): [(pid: 1, idx: 1), (pid: 1, idx: 2)]}",
            "stacks: {(idx: 2, version: 1): [(pid: 1, idx: 2), (pid: 1, idx: 1)]}",
        );
        let file = written_as(
            &file,
            "window: Some((pid: 1, idx: 2))",
            "window: Some((pid: 1, idx: 7))",
        );
        let system: BspLayoutSystem = ron::from_str(&file).unwrap();
        let layout = system.core.layouts().keys().next().unwrap();
        assert_eq!(system.windows(layout, WindowQuery::Visible), vec![w(1), w(3)]);
    }

    /// A legacy node no layout reaches is dropped at the load: its slot is vacated (its version
    /// bumped, as upstream's retain did) and nothing of it is saved.
    #[test]
    fn a_legacy_node_no_layout_reaches_is_dropped() {
        let file = legacy_as(
            "            (value: None, version: 2),\n            (value: None, version: 2),\n        ]),",
            "            (value: Some((\n                parent: None, prev_sibling: None, next_sibling: None,\n                first_child: None, last_child: None,\n            )), version: 1),\n            (value: None, version: 2),\n        ]),",
        );
        let file = written_as(
            &file,
            "            preselected: None,\n        )), version: 1),\n    ],\n    stacks",
            "            preselected: None,\n        )), version: 1),\n        (value: Some(Leaf(\n            window: Some((pid: 1, idx: 8)), fullscreen: false, fullscreen_within_gaps: false,\n            preselected: None,\n        )), version: 1),\n    ],\n    stacks",
        );
        let system: BspLayoutSystem = ron::from_str(&file).unwrap();
        let saved = ron::to_string(&system).unwrap();
        assert!(!saved.contains("idx:8"), "{saved}");
        assert!(
            saved.contains("(value:None,version:2),(value:None,version:2)]"),
            "{saved}"
        );
    }

    /// The system the saved-system fixture holds, made by operations: stacks, a fill, a split's
    /// memory, a share off the middle, vacant slots and a cloned layout.
    fn saved_fixture_system() -> BspLayoutSystem {
        use crate::layout_engine::{ResizeOrientation, WindowDropAction};
        let mut system = BspLayoutSystem::default();
        let main = system.create_layout();
        let spare = system.create_layout();
        let other = system.create_layout();
        for idx in 1..=5 {
            system.update(WindowChange::Add(main, w(idx), None));
        }
        system.update(WindowChange::Add(spare, w(9), None));
        system.edit_layout(main, LayoutEdit::Drop {
            source: w(5),
            target: w(4),
            action: WindowDropAction::Stack,
        });
        system.focus(main, Focus::Select(w(1)));
        system.edit_layout(main, LayoutEdit::Resize {
            amount: 0.15,
            orientation: ResizeOrientation::Horizontal,
        });
        system.edit_layout(main, LayoutEdit::Fullscreen { within_gaps: true });
        system.remove_layout(spare);
        let cloned = system.clone_layout(main);
        system.update(WindowChange::Remove(w(2)));
        system.update(WindowChange::Add(other, w(6), None));
        system.focus(cloned, Focus::Select(w(4)));
        system
    }

    /// A system as rift saves it today (`bsp_saved.ron`, written while upstream's system still
    /// stood beside the core and the core's bytes were held to upstream's): the operations save
    /// to exactly those bytes, and the file loads and saves back byte for byte.
    #[test]
    fn a_saved_system_loads_and_saves_byte_for_byte() {
        let file = include_str!("bsp_saved.ron").trim_end();
        assert_eq!(ron::to_string(&saved_fixture_system()).unwrap(), file);
        let mut loaded: BspLayoutSystem = ron::from_str(file).unwrap();
        assert_eq!(ron::to_string(&loaded).unwrap(), file);
        // The loaded tree takes its vacant slots as the saved form's slotmaps do once loaded
        // (the highest first, after the abandoned nodes went): the next layout and its root
        // land where the slotmaps put them.
        let stored: StoredBspSystem = ron::from_str(file).unwrap();
        let mut tables = Loaded::try_from(stored).unwrap();
        let made = loaded.create_layout();
        let root = loaded.core.layouts().get(made).unwrap().root;
        let expected_root = tables.nodes.insert(tables.nodes.values().next().unwrap().clone());
        assert_eq!(made, tables.layouts.insert(LayoutState { root: expected_root }));
        assert_eq!(root, expected_root);
    }

    /// What rift pays per BSP operation and layout pass at 10, 100 and 1000 windows (release:
    /// `cargo test --release --lib measure_bsp_system -- --ignored --nocapture`, and again with
    /// `--features roc-core`): everyday operations (focus moves, swaps, resizes, a window added
    /// and removed) through the system, and a layout pass. A plain build pays the core's tree
    /// alone; a `roc-core` build pays the deciding Roc tree, the Rust tree as its shadow and the
    /// view comparison on every operation, and both geometries on a pass.
    #[test]
    #[ignore]
    fn measure_bsp_system() {
        use std::hint::black_box;
        use std::time::Instant;

        use crate::layout_engine::ResizeOrientation;

        for count in [10_u32, 100, 1000] {
            let mut system = BspLayoutSystem::default();
            let layout = system.create_layout();
            for idx in 1..=count {
                system.update(WindowChange::Add(layout, w(idx), None));
            }
            let directions = [
                Direction::Left,
                Direction::Up,
                Direction::Right,
                Direction::Down,
            ];
            let ops = 4000_u32;
            let started = Instant::now();
            for i in 0..ops {
                match i % 4 {
                    0 => drop(black_box(
                        system.focus(layout, Focus::Direction(directions[(i / 4 % 4) as usize])),
                    )),
                    1 => drop(black_box(system.edit_layout(
                        layout,
                        LayoutEdit::Swap(w(1 + i % count), w(1 + (i * 7) % count)),
                    ))),
                    2 => drop(black_box(system.edit_layout(layout, LayoutEdit::Resize {
                        amount: 0.05,
                        orientation: ResizeOrientation::Smart,
                    }))),
                    _ => {
                        system.update(WindowChange::Add(layout, w(count + 1), None));
                        system.update(WindowChange::Remove(w(count + 1)));
                    }
                }
            }
            let each = started.elapsed() / ops;
            let screen = CGRect::new(
                objc2_core_foundation::CGPoint::new(0.0, 0.0),
                objc2_core_foundation::CGSize::new(3000.0, 2000.0),
            );
            let gaps = crate::common::config::GapSettings::default();
            let rounds = 200;
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(system.calculate_layout(
                    layout,
                    screen,
                    0.0,
                    &HashMap::default(),
                    &gaps,
                    Default::default(),
                    crate::layout_engine::Shadow::Run,
                ));
            }
            let pass = started.elapsed() / rounds;
            eprintln!(
                "{count} windows (roc-core: {}): {each:?}/op, a layout pass {pass:?}",
                cfg!(feature = "roc-core")
            );
        }
    }
}
