//! The BSP layout system: the core's [`Tree`] decides and is saved (layout trees, slice 4, the
//! flip), with upstream's system ([`Upstream`]) the shadow. The shadow is stepped on every
//! operation, in every build, and held to the core: each answer is compared on every operation,
//! the whole tree (the view) on every operation in tests and `roc-core` builds and every
//! [`VIEW_EVERY`]th in a plain release build, and each pass's shape (what the core's geometry is
//! handed) on every pass. A difference is counted, fails a test, or in a running Rift is warned
//! and the shadow re-seeded from the core.

use std::collections::BTreeSet;
use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};

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
use super::{LayoutState, LeafMembers, Node, NodeKind, Upstream};
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

/// How often a plain release build compares the whole tree (the twins' sampling).
const VIEW_EVERY: u64 = 32;

/// Whether every operation compares the whole tree.
const VIEW_ALWAYS: bool = cfg!(any(test, feature = "roc-core"));

static BSP_TREE_DIVERGENCES: AtomicU64 = AtomicU64::new(0);

/// Differences between the core's BSP tree and upstream's system, counted process-wide.
pub(crate) fn bsp_tree_divergences() -> u64 { BSP_TREE_DIVERGENCES.load(Ordering::Relaxed) }

/// The BSP layout system: the core's tree decides, and upstream's system is stepped beside it.
/// Loaded through upstream's load (which parses old forms and normalizes what only a file
/// holds), saved from the core in the same form.
#[derive(Clone, Debug, Deserialize)]
#[serde(from = "Upstream")]
pub struct BspLayoutSystem {
    /// Under `roc-core` the Roc tree's shadow, else the deciding tree; saved either way (under
    /// `roc-core` it is re-seeded from the Roc tree on any difference, compared on every
    /// operation).
    pub(super) core: CoreTree,
    /// Under `roc-core`, the deciding tree.
    #[cfg(feature = "roc-core")]
    pub(super) roc: RocTree,
    /// Boxed: the system is one variant of `LayoutSystemKind`.
    pub(super) shadow: Box<Shadow>,
}

/// Upstream's system and how many operations stepped it.
#[derive(Clone, Debug)]
pub(super) struct Shadow {
    pub(super) upstream: Upstream,
    steps: u64,
}

/// The core's tree as upstream saves its system (the field names and order of upstream's
/// derived form), byte for byte.
#[derive(Serialize)]
#[serde(rename = "Upstream")]
struct Saved {
    layouts: Vec<SavedSlot<LayoutState>>,
    nodes: Vec<SavedSlot<Node>>,
}

impl Serialize for BspLayoutSystem {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Saved {
            layouts: saved(self.core.layouts(), |_, layout| upstream_layout(layout)).collect(),
            nodes: saved(self.core.nodes(), |_, node| upstream_node(node)).collect(),
        }
        .serialize(serializer)
    }
}

impl From<Upstream> for BspLayoutSystem {
    fn from(upstream: Upstream) -> Self {
        let core = seed(&upstream);
        Self {
            #[cfg(feature = "roc-core")]
            roc: RocTree::seeded(&core),
            shadow: Box::new(Shadow { upstream, steps: 0 }),
            core,
        }
    }
}

impl Default for BspLayoutSystem {
    fn default() -> Self { Upstream::default().into() }
}

impl BspLayoutSystem {
    pub fn new(window_insertion_point: WindowInsertionPoint) -> Self {
        Upstream::new(window_insertion_point).into()
    }

    pub fn set_window_insertion_point(&mut self, value: WindowInsertionPoint) {
        self.shadow.upstream.set_window_insertion_point(value);
        self.core.set_window_insertion_point(insertion_point(value));
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "set_window_insertion_point",
            RocEvent::Insertion(insertion_point(value)),
            RocAnswer::Done,
        );
        self.stepped("set_window_insertion_point", &(), &());
    }

    pub fn set_single_window_aspect_ratio(&mut self, ratio: Option<f64>) {
        self.shadow.upstream.set_single_window_aspect_ratio(ratio);
        self.core.set_single_window_aspect_ratio(ratio);
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "set_single_window_aspect_ratio",
            RocEvent::AspectRatio(ratio),
            RocAnswer::Done,
        );
        self.stepped("set_single_window_aspect_ratio", &(), &());
    }

    pub(in crate::layout_engine) fn can_move_window(
        &self,
        layout: LayoutId,
        window: WindowId,
        direction: Direction,
    ) -> bool {
        let core = self.answered(
            "can_move_window",
            self.shadow.upstream.can_move_window(layout, window, direction),
            self.core.can_move_window(layout, window, core_direction(direction)),
        );
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

    /// The layout drawn as text (rift-cli's debug view), from the core's tree; upstream's
    /// drawing is held to it.
    pub(crate) fn draw_tree(&self, layout: LayoutId) -> String {
        self.answered(
            "draw_tree",
            self.shadow.upstream.draw_tree(layout),
            self.core.draw(layout),
        )
    }

    /// The layout's nodes as rift-cli's container query shows them, from the core's tree;
    /// upstream's view is held to it.
    pub(crate) fn container_tree(&self, layout: LayoutId) -> rift_protocol::ContainerTreeNode {
        self.answered(
            "container_tree",
            self.shadow.upstream.container_tree(layout),
            self.core
                .container(layout)
                .map_or_else(Default::default, |root| container_node(&root)),
        )
    }

    pub(crate) fn window_slot(&self, layout: LayoutId, window: WindowId) -> Option<Vec<usize>> {
        let core = self.answered(
            "window_slot",
            self.shadow.upstream.window_slot(layout, window),
            self.core.window_slot(layout, window),
        );
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

    /// After an operation the core and upstream's system both took: their answers compared,
    /// and the trees when due; a difference is reported and the shadow re-seeded from the core.
    fn stepped<A: PartialEq + Debug>(&mut self, what: &str, upstream: &A, core: &A) {
        self.shadow.steps += 1;
        let answers_differ = upstream != core;
        if answers_differ {
            report(what, &format!("answers: upstream {upstream:?}, core {core:?}"));
        }
        let view_due = VIEW_ALWAYS || self.shadow.steps.is_multiple_of(VIEW_EVERY);
        let view = (answers_differ || view_due)
            .then(|| view_difference(&self.core, &self.shadow.upstream));
        if let Some(Some(difference)) = &view {
            report(what, difference);
        }
        if answers_differ || matches!(view, Some(Some(_))) {
            self.shadow.upstream = upstream_of(&self.core);
        }
    }

    /// A query both answered, the core's answer given: a difference is reported (the next
    /// step's view re-seeds the shadow).
    fn answered<A: PartialEq + Debug>(&self, what: &str, upstream: A, core: A) -> A {
        if upstream != core {
            report(what, &format!("answers: upstream {upstream:?}, core {core:?}"));
        }
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

/// Counts a difference, then fails the test, or in a running Rift warns it.
fn report(what: &str, difference: &str) {
    let divergences = BSP_TREE_DIVERGENCES.fetch_add(1, Ordering::Relaxed) + 1;
    #[cfg(test)]
    panic!("bsp-tree-divergence ({divergences} so far) after {what}: {difference}");
    #[cfg(not(test))]
    tracing::warn!(
        what,
        divergences,
        difference,
        "bsp-tree-divergence: upstream's BSP system differs from the core's tree; re-seeded from the core"
    );
}

impl LayoutSystem for BspLayoutSystem {
    fn has_windows_for_app(&self, layout: LayoutId, pid: pid_t) -> bool {
        let core = self.answered(
            "has_windows_for_app",
            self.shadow.upstream.has_windows_for_app(layout, pid),
            self.core.has_windows_for_app(layout, |window| window.pid == pid),
        );
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
        let core = self.answered(
            "window_participates",
            self.shadow.upstream.window_participates(window),
            self.core.window_participates(window),
        );
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
        let upstream = self.shadow.upstream.set_window_participation(window, participates);
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
        self.stepped("set_window_participation", &upstream, &core);
        core
    }

    fn create_layout(&mut self) -> LayoutId {
        let upstream = self.shadow.upstream.create_layout();
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
        self.stepped("create_layout", &upstream, &core);
        core
    }

    fn contains_layout(&self, layout: LayoutId) -> bool {
        let core = self.answered(
            "contains_layout",
            self.shadow.upstream.contains_layout(layout),
            self.core.contains_layout(layout),
        );
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
        let upstream = self.shadow.upstream.clone_layout(layout);
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
        self.stepped("clone_layout", &upstream, &core);
        core
    }

    fn remove_layout(&mut self, layout: LayoutId) {
        self.shadow.upstream.remove_layout(layout);
        self.core.remove_layout(layout);
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "remove_layout",
            RocEvent::RemoveLayout(roc::layout_key(layout)),
            RocAnswer::Done,
        );
        self.stepped("remove_layout", &(), &());
    }

    /// The core's frames (policy (a)): under `roc-core` the Roc tree's own shape through the Roc
    /// geometry, else the Rust tree's shape through the Rust geometry, with the Rust geometry's
    /// frames for the Rust tree's shape and upstream's geometry the shadows; upstream's system's
    /// shape is held to the Rust tree's on every pass.
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
        use crate::layout_engine::core_geometry::core_constraints;
        use crate::layout_engine::{core_gaps, core_rect};
        let upstream = &self.shadow.upstream;
        let core = self.core.shape(layout, core_rect(screen), core_gaps(gaps), |window| {
            constraints.get(&window).copied().map(core_constraints)
        });
        let upstream_shape = upstream
            .layouts
            .get(layout)
            .and_then(|state| upstream.shape(state.root, screen, constraints, gaps));
        if upstream_shape != core {
            report(
                "calculate_layout",
                &format!("shapes: upstream {upstream_shape:?}, core {core:?}"),
            );
        }
        let upstream_frames = || upstream.shadow_frames(layout, screen, constraints, gaps);
        // Under `roc-core` the Roc tree's own shape through the Roc geometry decides.
        #[cfg(feature = "roc-core")]
        return crate::layout_engine::core_geometry::decide_bsp_tree(
            core,
            shadow,
            upstream_frames,
            || {
                use crate::roc_path::Word;
                let known: Vec<_> = constraints
                    .iter()
                    .map(|(&window, &held)| (window.word(), core_constraints(held)))
                    .collect();
                self.roc.frames(layout, core_rect(screen), core_gaps(gaps), &known)
            },
        );
        #[cfg(not(feature = "roc-core"))]
        crate::layout_engine::core_geometry::decide_bsp(core, shadow, upstream_frames)
    }

    fn selected_window(&self, layout: LayoutId) -> Option<WindowId> {
        let core = self.answered(
            "selected_window",
            self.shadow.upstream.selected_window(layout),
            self.core.selected_window(layout),
        );
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
        let core = self.answered(
            "window_in_direction",
            self.shadow.upstream.window_in_direction(layout, direction),
            self.core.window_in_direction(layout, core_direction(direction)),
        );
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
        let core = self.answered(
            "contains_window",
            self.shadow.upstream.contains_window(layout, wid),
            self.core.contains_window(layout, wid),
        );
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
        let core = self.answered(
            "has_any_fullscreen_node",
            self.shadow.upstream.has_any_fullscreen_node(layout),
            self.core.has_any_fullscreen_node(layout),
        );
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
        let core = self.answered(
            "windows",
            self.shadow.upstream.windows(layout, query),
            self.core.windows(layout, core_query),
        );
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
        let upstream = self.shadow.upstream.focus(layout, focus);
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
        let core = match focused {
            core::Focused::Selection { handled } => FocusResult::selection(handled),
            core::Focused::Movement { window, raise } => FocusResult::movement((window, raise)),
        };
        self.stepped("focus", &focus_words(&upstream), &focus_words(&core));
        core
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
        self.shadow.upstream.update(change);
        #[cfg(feature = "roc-core")]
        self.roc_stepped("update", roc_event, RocAnswer::Done);
        self.stepped("update", &(), &());
    }

    fn edit_layout(&mut self, layout: LayoutId, edit: LayoutEdit) -> EditResult {
        let upstream = self.shadow.upstream.edit_layout(layout, edit);
        let core = match core_edit(edit) {
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
            // An edit upstream's BSP system has no case for (nor the trees).
            None => EditResult::Unsupported,
        };
        self.stepped("edit_layout", &edit_words(&upstream), &edit_words(&core));
        core
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
            node.layout_kind = Some(LayoutKind::from(upstream_orientation(*orientation)));
            node.children = children.iter().map(|child| container_node(child)).collect();
        }
        core::ContainerKind::Leaf(leaf) => {
            node.pending_split = match leaf.preselect {
                Preselect::Unset => None,
                Preselect::Toward(direction) => Some(upstream_direction(direction)),
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

/// A focus's answer, comparable.
fn focus_words(result: &FocusResult) -> (bool, Option<WindowId>, Vec<WindowId>) {
    (result.handled, result.window, result.raise.clone())
}

/// An edit's answer, comparable.
fn edit_words(result: &EditResult) -> (&'static str, Vec<WindowId>) {
    match result {
        EditResult::Unsupported => ("unsupported", Vec::new()),
        EditResult::Unchanged => ("unchanged", Vec::new()),
        EditResult::Changed => ("changed", Vec::new()),
        EditResult::Raise(windows) => ("raise", windows.clone()),
    }
}

/// An edit as the core takes it; none for those upstream's BSP system has no case for.
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

/// Upstream's system as the core holds it.
fn seed(upstream: &Upstream) -> CoreTree {
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
        table(&upstream.layouts, dummy_layout, |state| Layout {
            root: state.root,
        }),
        table(&upstream.nodes, dummy_node, core_node),
        upstream.inactive.iter().copied().collect::<BTreeSet<_>>(),
        insertion_point(upstream.window_insertion_point),
        upstream.single_window_aspect_ratio,
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

/// Upstream's two fullscreen flags as one fill (`fullscreen` first, as its projection reads
/// them; its operations never set both).
fn core_fill(fullscreen: bool, within_gaps: bool) -> core::Fill {
    match (fullscreen, within_gaps) {
        (true, _) => core::Fill::Screen,
        (false, true) => core::Fill::WithinGaps,
        (false, false) => core::Fill::Tiled,
    }
}

/// An upstream node as the core holds it.
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

/// A leaf's members as the core holds them (an empty leaf keeps no fill: upstream's operations
/// clear it as the leaf empties).
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

/// The core's tree as upstream's system: the shadow re-seeded from the core. Each table keeps
/// the core's slots and versions and takes its vacant slots in the core's order, so both
/// allocate the same ids next.
pub(super) fn upstream_of(core: &CoreTree) -> Upstream {
    let dummy_node = upstream_node(&core::Node {
        parent: Parent::Root,
        kind: Kind::Leaf(Leaf {
            content: Content::Empty,
            preselect: Preselect::Unset,
        }),
    });
    Upstream {
        inactive: core.left_out().iter().copied().collect(),
        layouts: slot_map(core.layouts(), upstream_layout, LayoutState {
            root: NodeId::default(),
        }),
        nodes: slot_map(core.nodes(), upstream_node, dummy_node),
        window_insertion_point: window_insertion_point(core.insertion()),
        single_window_aspect_ratio: core.aspect(),
    }
}

/// A core table as a slotmap with the same slots, versions and free list. A slotmap is only
/// built through its saved form, whose load rebuilds the free list (the highest vacant slot
/// first), so each vacant slot is written two versions back, then taken (`| 1`) and freed (`+
/// 1`) again in the core's order: the last freed is taken first, as the core's last.
fn slot_map<K: slotmap::Key + SlotKey, C, V: Clone + Serialize + DeserializeOwned>(
    table: &SlotTable<K, C>,
    convert: impl Fn(&C) -> V,
    dummy: V,
) -> SlotMap<K, V> {
    let slots: Vec<SavedSlot<V>> = table
        .saved()
        .enumerate()
        .map(|(index, (occupant, version))| SavedSlot {
            version: match occupant {
                Some(_) => version,
                // The sentinel, never taken.
                None if index == 0 => version,
                None => version.wrapping_sub(2),
            },
            value: occupant.map(|(_, value)| convert(value)),
        })
        .collect();
    let written = ron::to_string(&slots).expect("a slot sequence is written");
    let mut map: SlotMap<K, V> = ron::from_str(&written).expect("a core table is a slot sequence");
    let taken: HashMap<u32, K> = table
        .free()
        .iter()
        .map(|_| {
            let key = map.insert(dummy.clone());
            (key.parts().0, key)
        })
        .collect();
    for index in table.free() {
        map.remove(taken[index]);
    }
    map
}

fn upstream_layout(layout: &Layout<NodeId>) -> LayoutState { LayoutState { root: layout.root } }

fn window_insertion_point(value: InsertionPoint) -> WindowInsertionPoint {
    match value {
        InsertionPoint::NextToSelection => WindowInsertionPoint::NextToSelection,
        InsertionPoint::EndOfTree => WindowInsertionPoint::EndOfTree,
    }
}

fn upstream_orientation(orientation: core::Orientation) -> Orientation {
    match orientation {
        core::Orientation::Horizontal => Orientation::Horizontal,
        core::Orientation::Vertical => Orientation::Vertical,
    }
}

fn upstream_direction(direction: core::Direction) -> Direction {
    match direction {
        core::Direction::Left => Direction::Left,
        core::Direction::Right => Direction::Right,
        core::Direction::Up => Direction::Up,
        core::Direction::Down => Direction::Down,
    }
}

/// A core node as upstream holds it (and saves it): the inverse of [`core_node`].
fn upstream_node(node: &core::Node<NodeId, WindowId>) -> Node {
    let kind = match &node.kind {
        Kind::Split(split) => NodeKind::Split {
            orientation: upstream_orientation(split.orientation),
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
                    Preselect::Toward(direction) => Some(upstream_direction(direction)),
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

/// Where the core's tree differs from upstream's system, if it does: layouts, nodes (ids
/// compared directly), the windows left out, the settings.
fn view_difference(core: &CoreTree, upstream: &Upstream) -> Option<String> {
    let layouts: Vec<_> = upstream
        .layouts
        .iter()
        .map(|(id, state)| (id, Layout { root: state.root }))
        .collect();
    let core_layouts: Vec<_> = core.layouts().iter().map(|(id, layout)| (id, *layout)).collect();
    if layouts != core_layouts {
        return Some(format!("layouts: upstream {layouts:?}, core {core_layouts:?}"));
    }
    let mut core_nodes = core.nodes().iter();
    for (id, node) in upstream.nodes.iter() {
        let expected = core_node(node);
        match core_nodes.next() {
            Some((core_id, core_node)) if core_id == id && *core_node == expected => {}
            other => {
                return Some(format!("node {id:?}: upstream {expected:?}, core {other:?}"));
            }
        }
    }
    if let Some(extra) = core_nodes.next() {
        return Some(format!("a node upstream lacks: {extra:?}"));
    }
    let left_out: BTreeSet<_> = upstream.inactive.iter().copied().collect();
    if &left_out != core.left_out() {
        return Some(format!(
            "left out: upstream {left_out:?}, core {:?}",
            core.left_out()
        ));
    }
    let settings = (
        insertion_point(upstream.window_insertion_point),
        upstream.single_window_aspect_ratio,
    );
    if settings != (core.insertion(), core.aspect()) {
        return Some(format!(
            "settings: upstream {settings:?}, core {:?}",
            (core.insertion(), core.aspect())
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(idx: u32) -> WindowId { WindowId::new(1, idx) }

    /// The core's tree seeded from a system takes its free list as the system's slotmap holds
    /// it, not as a load would rebuild it (the highest vacant slot first): the next ids match.
    #[test]
    fn a_seeded_core_allocates_the_ids_upstream_would() {
        let mut system = BspLayoutSystem::default();
        let layouts: Vec<_> = (0..4).map(|_| system.create_layout()).collect();
        // Vacate slots 1 and 3 (nodes and layouts), the lower last: upstream hands slot 1 out
        // next, a load would hand out slot 3.
        system.remove_layout(layouts[2]);
        system.remove_layout(layouts[0]);
        system.core = seed(&system.shadow.upstream);
        let upstream = system.shadow.upstream.create_layout();
        let core = system.core.create_layout();
        assert_eq!(core, upstream);
        assert_eq!(
            system.core.layouts()[core].root,
            system.shadow.upstream.layouts[upstream].root
        );
    }

    /// Upstream's system re-seeded from the core (after a divergence) keeps the core's vacant
    /// slots' versions and takes them in the core's order, not as a load would (the highest
    /// first): the next ids match, and the save is the core's, byte for byte.
    #[test]
    fn a_shadow_reseeded_from_the_core_allocates_the_ids_the_core_would() {
        let mut system = BspLayoutSystem::default();
        let layouts: Vec<_> = (0..4).map(|_| system.create_layout()).collect();
        system.remove_layout(layouts[2]);
        system.remove_layout(layouts[0]);
        system.shadow.upstream = upstream_of(&system.core);
        assert_eq!(
            ron::to_string(&system).unwrap(),
            ron::to_string(&system.shadow.upstream).unwrap()
        );
        for _ in 0..3 {
            let core = system.core.create_layout();
            let upstream = system.shadow.upstream.create_layout();
            assert_eq!(upstream, core);
            assert_eq!(
                system.shadow.upstream.layouts[upstream].root,
                system.core.layouts()[core].root
            );
        }
    }

    /// A query upstream's system answers otherwise than the core is counted and fails a test.
    #[test]
    fn a_query_answered_otherwise_is_counted_and_fails_tests() {
        let mut system = BspLayoutSystem::default();
        system.create_layout();
        system.shadow.upstream.set_window_participation(w(1), false);
        let before = bsp_tree_divergences();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            system.window_participates(w(1));
        }));
        assert!(failed.is_err());
        assert!(bsp_tree_divergences() > before);
    }

    /// A leaf with a preselected direction (only a saved file holds one) is split that way by
    /// the next window, which goes on that side and is selected; a window after it goes next to
    /// the selection as usual.
    #[test]
    fn a_preselected_direction_puts_the_next_window_on_that_side() {
        let mut upstream = Upstream::new(WindowInsertionPoint::NextToSelection);
        let layout = upstream.create_layout();
        upstream.update(WindowChange::Add(layout, w(1), None));
        let root = upstream.layouts[layout].root;
        let NodeKind::Leaf { preselected, .. } = &mut upstream.nodes[root].kind else {
            unreachable!("a layout of one window is a leaf");
        };
        *preselected = Some(Direction::Left);
        let mut system = BspLayoutSystem::from(upstream);
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

    /// A file holding `corrupt` loads as `normalized` holds it, upstream's copy and the core
    /// alike, and is saved as `normalized` is: the bytes rift writes.
    fn loads_normalized(corrupt: &Upstream, normalized: &Upstream) {
        let file = ron::to_string(corrupt).unwrap();
        let loaded: BspLayoutSystem = ron::from_str(&file).unwrap();
        assert_eq!(view_difference(&loaded.core, &loaded.shadow.upstream), None);
        assert_eq!(view_difference(&seed(normalized), &loaded.shadow.upstream), None);
        let expected = ron::to_string(normalized).unwrap();
        assert_ne!(file, expected);
        assert_eq!(ron::to_string(&loaded).unwrap(), expected);
    }

    fn leaf_flags(system: &mut Upstream, node: NodeId) -> (&mut bool, &mut bool) {
        let NodeKind::Leaf {
            fullscreen,
            fullscreen_within_gaps,
            ..
        } = &mut system.nodes[node].kind
        else {
            unreachable!("the node is a leaf");
        };
        (fullscreen, fullscreen_within_gaps)
    }

    /// A saved leaf with both fullscreen flags loads fullscreen (as the projection reads it).
    #[test]
    fn a_saved_leaf_with_both_fullscreen_flags_loads_fullscreen() {
        let mut normalized = Upstream::default();
        let layout = normalized.create_layout();
        normalized.update(WindowChange::Add(layout, w(1), None));
        let root = normalized.layouts[layout].root;
        *leaf_flags(&mut normalized, root).0 = true;
        let mut corrupt = normalized.clone();
        *leaf_flags(&mut corrupt, root).1 = true;
        loads_normalized(&corrupt, &normalized);
    }

    /// A saved empty leaf with a fullscreen flag loads without it.
    #[test]
    fn a_saved_empty_leaf_loads_without_fullscreen_flags() {
        let mut normalized = Upstream::default();
        let layout = normalized.create_layout();
        let root = normalized.layouts[layout].root;
        for flags in [(true, false), (false, true), (true, true)] {
            let mut corrupt = normalized.clone();
            let (fullscreen, within_gaps) = leaf_flags(&mut corrupt, root);
            (*fullscreen, *within_gaps) = flags;
            loads_normalized(&corrupt, &normalized);
        }
    }

    /// A saved split whose share is not a finite number loads with a fresh split's share; a
    /// finite share stands as saved, even outside the bounds edits keep.
    #[test]
    fn a_saved_split_with_a_share_not_a_number_loads_even() {
        let mut normalized = Upstream::default();
        let layout = normalized.create_layout();
        normalized.update(WindowChange::Add(layout, w(1), None));
        normalized.update(WindowChange::Add(layout, w(2), None));
        let root = normalized.layouts[layout].root;
        let set_share = |system: &mut Upstream, share: f32| match &mut system.nodes[root].kind {
            NodeKind::Split { ratio, .. } => *ratio = share,
            NodeKind::Leaf { .. } => unreachable!("two windows split the root"),
        };
        for saved in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut corrupt = normalized.clone();
            set_share(&mut corrupt, saved);
            loads_normalized(&corrupt, &normalized);
        }
        let mut wide = normalized.clone();
        set_share(&mut wide, 0.99);
        let loaded: Upstream = ron::from_str(&ron::to_string(&wide).unwrap()).unwrap();
        assert_eq!(ron::to_string(&loaded).unwrap(), ron::to_string(&wide).unwrap());
    }

    /// Upstream's system differing from the core is counted and fails a test (a running Rift
    /// warns and re-seeds it from the core).
    #[test]
    fn a_bsp_tree_divergence_is_counted_and_fails_tests() {
        let mut system = BspLayoutSystem::default();
        let layout = system.create_layout();
        system.shadow.upstream.set_window_participation(w(1), false);
        let before = bsp_tree_divergences();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            system.clone_layout(layout);
        }));
        assert!(failed.is_err());
        assert!(bsp_tree_divergences() > before);
    }

    /// The twin's cost per operation at 10, 100 and 1000 windows (release:
    /// `cargo test --release --lib measure_bsp_twin -- --ignored --nocapture`): upstream's
    /// operation alone (the shadow's step), the core's step alone, and the whole-tree comparison
    /// and a re-seed of the shadow from the core, each on its own. A plain release build pays
    /// both steps on every operation and the comparison on every 32nd.
    #[test]
    #[ignore]
    fn measure_bsp_twin() {
        use std::hint::black_box;
        use std::time::Instant;

        for count in [10_u32, 100, 1000] {
            let mut system = BspLayoutSystem::default();
            let layout = system.create_layout();
            for idx in 1..=count {
                system.update(WindowChange::Add(layout, w(idx), None));
            }
            // A mix of everyday operations: focus moves, swaps, resizes, a window coming and
            // going.
            let ops = 4000_u32;
            let directions = [
                Direction::Left,
                Direction::Up,
                Direction::Right,
                Direction::Down,
            ];
            let mut upstream = system.shadow.upstream.clone();
            let started = Instant::now();
            for i in 0..ops {
                match i % 4 {
                    0 => drop(black_box(
                        upstream.focus(layout, Focus::Direction(directions[(i / 4 % 4) as usize])),
                    )),
                    1 => drop(black_box(upstream.edit_layout(
                        layout,
                        LayoutEdit::Swap(w(1 + i % count), w(1 + (i * 7) % count)),
                    ))),
                    2 => drop(black_box(upstream.edit_layout(layout, LayoutEdit::Resize {
                        amount: 0.05,
                        orientation: crate::layout_engine::ResizeOrientation::Smart,
                    }))),
                    _ => {
                        upstream.update(WindowChange::Add(layout, w(count + 1), None));
                        upstream.update(WindowChange::Remove(w(count + 1)));
                    }
                }
            }
            let upstream_each = started.elapsed() / ops;
            let mut core = system.core.clone();
            let core_directions = directions.map(core_direction);
            let started = Instant::now();
            for i in 0..ops {
                match i % 4 {
                    0 => drop(black_box(core.focus(
                        layout,
                        core::Focus::Direction(core_directions[(i / 4 % 4) as usize]),
                    ))),
                    1 => drop(black_box(core.edit(
                        layout,
                        core::Edit::Swap(w(1 + i % count), w(1 + (i * 7) % count)),
                    ))),
                    2 => drop(black_box(core.edit(layout, core::Edit::Resize {
                        amount: 0.05,
                        along: core::ResizeAlong::Smart,
                    }))),
                    _ => {
                        core.add_window(layout, w(count + 1));
                        core.remove_window(w(count + 1));
                    }
                }
            }
            let core_each = started.elapsed() / ops;
            assert!(view_difference(&core, &upstream).is_none());
            let rounds = 200;
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(view_difference(&core, &upstream));
            }
            let view = started.elapsed() / rounds;
            let started = Instant::now();
            for _ in 0..rounds / 10 {
                black_box(upstream_of(&core));
            }
            let reseed = started.elapsed() / (rounds / 10);
            // Each pass builds the core's tree's shape beside upstream's system's.
            let screen = objc2_core_foundation::CGRect::new(
                objc2_core_foundation::CGPoint::new(0.0, 0.0),
                objc2_core_foundation::CGSize::new(3000.0, 2000.0),
            );
            let gaps = crate::common::config::GapSettings::default();
            let root = upstream.layouts[layout].root;
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(upstream.shape(root, screen, &HashMap::default(), &gaps));
            }
            let upstream_shape = started.elapsed() / rounds;
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(core.shape(
                    layout,
                    crate::layout_engine::core_rect(screen),
                    crate::layout_engine::core_gaps(&gaps),
                    |_| None,
                ));
            }
            let core_shape = started.elapsed() / rounds;
            eprintln!(
                "{count} windows: upstream {upstream_each:?}/op, core step {core_each:?}/op \
                 ({:.0}%), view {view:?} ({:?}/op every 32nd), re-seed {reseed:?}; a pass's \
                 shape: upstream {upstream_shape:?}, core {core_shape:?}",
                100.0 * core_each.as_secs_f64() / upstream_each.as_secs_f64(),
                view / 32,
            );
        }
    }
}
