//! The stack layout system: the core's stacks decide (layout trees, stack slices 3 and 5; under
//! `roc-core` the Roc stacks decide, the Rust stacks their shadow, slice 4) and are saved in
//! upstream's form, byte for byte. Loaded through the saved form's parse (`stored`), which reads
//! the legacy forest and normalizes the states only a file holds.

use std::collections::BTreeSet;

use objc2_core_foundation::CGRect;
use rift_core::layout::stack_tree::{
    self as core, Beyond, DefaultOrientation, Fill, Id, Member as CoreMember, Members, NonEmpty,
    Orientation, Participation, Query, Resize, Selection, Stack as CoreStack, Stacks,
};
#[cfg(feature = "roc-core")]
use rift_core_roc::stack_tree::{Answer, Event, Scope};
use serde::{Deserialize, Serialize};
use slotmap::Key;

#[cfg(feature = "roc-core")]
use super::roc_stacks::{self as roc, RocStacks};
use super::stored::{Loaded, Member, Stack, StoredStackSystem};
use crate::actor::app::{WindowId, pid_t};
use crate::common::collections::HashMap;
use crate::common::config::{
    GapSettings, StackDefaultOrientation, StackLineReserve, WindowInsertionPoint,
};
use crate::layout_engine::engine::GroupContainerInfo;
use crate::layout_engine::systems::{
    EditResult, Focus, FocusResult, GroupEdit, LayoutEdit, LayoutSystem, WindowChange,
    WindowLayoutConstraints, WindowQuery, core_direction, insertion_point,
};
use crate::layout_engine::{Direction, LayoutId, LayoutKind};
use crate::model::slot_table::{SavedSlot, of_slotmap, saved};
use crate::model::tree::NodeId;

/// The core's stacks over rift's ids.
pub(crate) type CoreStacks = Stacks<LayoutId, WindowId>;

/// The stack layout system: the core's stacks decide (under `roc-core` the Roc stacks, the Rust
/// stacks their shadow), saved from the Rust stacks in upstream's form.
#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "StoredStackSystem")]
pub struct StackLayoutSystem {
    /// Under `roc-core` the Roc stacks' shadow, else the deciding stacks; saved either way (under
    /// `roc-core` it is re-seeded from the Roc stacks on any difference, compared on every
    /// operation).
    core: CoreStacks,
    /// Under `roc-core`, the deciding stacks.
    #[cfg(feature = "roc-core")]
    roc: RocStacks,
}

/// The core's stacks as upstream saves its system (the field names and order of upstream's
/// derived form, under its original name), byte for byte.
#[derive(Serialize)]
#[serde(rename = "StackLayoutSystem")]
struct Saved {
    layouts: Vec<SavedSlot<Stack>>,
    next_node: u32,
    default_orientation: StackDefaultOrientation,
}

impl Serialize for StackLayoutSystem {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Saved {
            layouts: saved(self.core.layouts(), |_, stack| saved_stack(stack)).collect(),
            next_node: self.core.next_node(),
            default_orientation: stack_default_orientation(self.core.orientation()),
        }
        .serialize(serializer)
    }
}

impl TryFrom<StoredStackSystem> for StackLayoutSystem {
    type Error = String;

    /// A saved system, parsed (`Loaded`), made into the core's stacks.
    fn try_from(stored: StoredStackSystem) -> Result<Self, String> {
        let loaded = Loaded::try_from(stored)?;
        Ok(Self::holding(core_of_loaded(&loaded)))
    }
}

impl StackLayoutSystem {
    /// The system deciding with `core` (under `roc-core` the Roc stacks seeded from it).
    fn holding(core: CoreStacks) -> Self {
        Self {
            #[cfg(feature = "roc-core")]
            roc: RocStacks::seeded(&core),
            core,
        }
    }
}

impl Default for StackLayoutSystem {
    fn default() -> Self { Self::new(crate::common::config::default_stack_orientation()) }
}

impl StackLayoutSystem {
    pub fn new(orientation: StackDefaultOrientation) -> Self {
        Self::new_with_insertion_point(orientation, WindowInsertionPoint::default())
    }

    pub fn new_with_insertion_point(
        orientation: StackDefaultOrientation,
        insertion: WindowInsertionPoint,
    ) -> Self {
        Self::holding(CoreStacks::new(
            default_orientation(orientation),
            insertion_point(insertion),
        ))
    }

    pub fn update_settings(
        &mut self,
        orientation: StackDefaultOrientation,
        insertion: WindowInsertionPoint,
    ) {
        let (orientation, insertion) =
            (default_orientation(orientation), insertion_point(insertion));
        self.core.update_settings(orientation, insertion);
        #[cfg(feature = "roc-core")]
        self.roc_stepped(
            "update_settings",
            Event::Settings(orientation, insertion),
            Answer::Done,
        );
    }

    pub(crate) fn collect_group_containers(
        &self,
        layout: LayoutId,
        screen: CGRect,
        _offset: f64,
        gaps: &GapSettings,
        _line: StackLineReserve,
        selection_path_only: bool,
    ) -> Vec<GroupContainerInfo> {
        let core = self.core.group_container(layout, selection_path_only);
        #[cfg(feature = "roc-core")]
        let core = roc::decided(
            self.roc_asked(
                "collect_group_containers",
                Event::GroupContainer(
                    roc::layout_key(layout),
                    if selection_path_only {
                        Scope::SelectionPath
                    } else {
                        Scope::Whole
                    },
                ),
                Answer::Group(roc::group(&core)),
            ),
            roc::as_group,
            core,
        );
        let core = core.map(|c| GroupContainerInfo {
            node_id: node_id(c.stack),
            container_kind: layout_kind(c.orientation),
            frame: crate::layout_engine::utils::compute_tiling_area(screen, gaps),
            total_count: c.windows.len(),
            selected_index: c.selected,
            window_ids: c.windows,
        });
        core.into_iter().collect()
    }

    /// rift-cli's drawing of the layout: the core's.
    pub(crate) fn draw_tree(&self, layout: LayoutId) -> String { self.core.draw(layout) }

    /// rift-cli's container view of the layout: the core's.
    pub(crate) fn container_tree(&self, layout: LayoutId) -> rift_protocol::ContainerTreeNode {
        container_node(self.core.container(layout))
    }

    pub(crate) fn window_slot(&self, layout: LayoutId, window: WindowId) -> Option<Vec<usize>> {
        let core = self.core.window_slot(layout, window);
        #[cfg(feature = "roc-core")]
        let core = roc::decided(
            self.roc_asked(
                "window_slot",
                Event::WindowSlot(roc::layout_key(layout), roc::window(window)),
                Answer::WindowSlot(core),
            ),
            roc::as_slot,
            core,
        );
        core.map(|i| vec![i])
    }

    /// An edit the core decides (under `roc-core` the Roc stacks, the Rust stacks' answer held to
    /// theirs).
    fn core_edit(
        &mut self,
        layout: LayoutId,
        edit: core::Edit<WindowId>,
    ) -> core::Edited<WindowId> {
        let edited = self.core.edit(layout, edit);
        #[cfg(feature = "roc-core")]
        let edited = roc::decided(
            self.roc_stepped(
                "edit_layout",
                Event::Edit(roc::layout_key(layout), roc::edit(edit)),
                Answer::Edited(roc::edited(&edited)),
            ),
            roc::as_edited,
            edited,
        );
        edited
    }

    /// Under `roc-core`, an operation the Rust stacks took (answering `answer`): the Roc stacks
    /// take it and decide (a difference re-seeds the Rust stacks from them); their answer is
    /// rift's.
    #[cfg(feature = "roc-core")]
    fn roc_stepped(&mut self, what: &str, event: Event, answer: Answer) -> Answer {
        self.roc.decided(&mut self.core, what, event, answer)
    }

    /// Under `roc-core`, a query the Rust stacks answered `answer`: the Roc stacks' answer is
    /// rift's.
    #[cfg(feature = "roc-core")]
    fn roc_asked(&self, what: &str, event: Event, answer: Answer) -> Answer {
        self.roc.asked(what, event, answer)
    }

    #[cfg(test)]
    pub(crate) fn shape(
        &self,
        layout: LayoutId,
        screen: CGRect,
        offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &GapSettings,
        line: StackLineReserve,
    ) -> Option<rift_core::layout::traditional::Traditional<WindowId>> {
        use crate::layout_engine::core_geometry::{core_constraints, core_stack_line};
        use crate::layout_engine::{core_gaps, core_rect};
        self.core.shape(
            layout,
            core_rect(screen),
            core_gaps(gaps),
            offset,
            core_stack_line(line),
            |window| constraints.get(&window).copied().map(core_constraints),
        )
    }

    /// The Rust stacks, for the Roc stacks' tests.
    #[cfg(all(test, feature = "roc-core"))]
    pub(crate) fn core_for_tests(&self) -> &CoreStacks { &self.core }

    /// The Rust stacks re-seeded from the deciding Roc stacks' view, as after a divergence.
    #[cfg(all(test, feature = "roc-core"))]
    pub(crate) fn reseed_rust_from_roc(&mut self) { self.core = roc::core_of(&self.roc.view()); }
}

impl LayoutSystem for StackLayoutSystem {
    fn has_windows_for_app(&self, layout: LayoutId, pid: pid_t) -> bool {
        let core = self.core.has_windows_for_app(layout, |window| window.pid == pid);
        #[cfg(feature = "roc-core")]
        let core = roc::decided(
            self.roc_asked(
                "has_windows_for_app",
                Event::HasWindowsForApp(
                    roc::layout_key(layout),
                    roc::app_windows(&self.core, |window| window.pid == pid),
                ),
                Answer::Verdict(core),
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
                Event::WindowParticipates(roc::window(window)),
                Answer::Verdict(core),
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
                Event::Participation(roc::window(window), participation),
                Answer::Participated(core),
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
                Event::CreateLayout,
                Answer::Layout(roc::layout_key(core)),
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
                Event::ContainsLayout(roc::layout_key(layout)),
                Answer::Verdict(core),
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
                Event::CloneLayout(roc::layout_key(layout)),
                Answer::Layout(roc::layout_key(core)),
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
            Event::RemoveLayout(roc::layout_key(layout)),
            Answer::Done,
        );
    }

    /// The core's frames: under `roc-core` the Roc stacks' own shape through the Roc geometry,
    /// with the Rust geometry's frames for the Rust stacks' shape the shadow; else the Rust
    /// stacks' shape through the Rust geometry.
    fn calculate_layout(
        &self,
        layout: LayoutId,
        screen: CGRect,
        offset: f64,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        gaps: &GapSettings,
        line: StackLineReserve,
        _shadow: crate::layout_engine::Shadow,
    ) -> Vec<(WindowId, CGRect)> {
        use crate::layout_engine::core_geometry::{core_constraints, core_stack_line};
        use crate::layout_engine::{core_gaps, core_rect};
        let core = self.core.shape(
            layout,
            core_rect(screen),
            core_gaps(gaps),
            offset,
            core_stack_line(line),
            |window| constraints.get(&window).copied().map(core_constraints),
        );
        // Under `roc-core` the Roc stacks' own shape through the Roc geometry decides.
        #[cfg(feature = "roc-core")]
        return crate::layout_engine::core_geometry::decide_roc_tree("stack", core, || {
            use crate::roc_path::Word;
            let known: Vec<_> = constraints
                .iter()
                .map(|(&window, &held)| (window.word(), core_constraints(held)))
                .collect();
            let pass = rift_core_roc::stack_tree::Pass {
                screen: core_rect(screen),
                gaps: core_gaps(gaps),
                stack_offset: offset,
                stack_line: core_stack_line(line),
            };
            self.roc.frames(layout, pass, &known)
        });
        #[cfg(not(feature = "roc-core"))]
        crate::layout_engine::core_geometry::decide_tree("stack", core)
    }

    fn selected_window(&self, layout: LayoutId) -> Option<WindowId> {
        let core = self.core.selected_window(layout);
        #[cfg(feature = "roc-core")]
        let core = roc::decided(
            self.roc_asked(
                "selected_window",
                Event::SelectedWindow(roc::layout_key(layout)),
                Answer::Window(core.map(roc::window)),
            ),
            roc::as_window,
            core,
        );
        core
    }

    fn selection_contains_window(&self, layout: LayoutId, window: WindowId) -> bool {
        let core = self.core.selection_contains_window(layout, window);
        #[cfg(feature = "roc-core")]
        let core = roc::decided(
            self.roc_asked(
                "selection_contains_window",
                Event::SelectionContainsWindow(roc::layout_key(layout), roc::window(window)),
                Answer::Verdict(core),
            ),
            roc::as_verdict,
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
                Event::WindowInDirection(roc::layout_key(layout), core_direction(direction)),
                Answer::Window(core.map(roc::window)),
            ),
            roc::as_window,
            core,
        );
        core
    }

    fn contains_window(&self, layout: LayoutId, window: WindowId) -> bool {
        let core = self.core.contains_window(layout, window);
        #[cfg(feature = "roc-core")]
        let core = roc::decided(
            self.roc_asked(
                "contains_window",
                Event::ContainsWindow(roc::layout_key(layout), roc::window(window)),
                Answer::Verdict(core),
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
                Event::HasAnyFullscreenNode(roc::layout_key(layout)),
                Answer::Verdict(core),
            ),
            roc::as_verdict,
            core,
        );
        core
    }

    /// A stack layout's selection is always in its stack (upstream's answer).
    fn parent_of_selection_is_stacked(&self, _layout: LayoutId) -> bool { true }

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
                Event::Windows(roc::layout_key(layout), roc::query(core_query)),
                Answer::Windows(roc::windows(&core)),
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
                Event::Focus(roc::layout_key(layout), roc::focus(core_focus)),
                Answer::Focused(roc::focused(&focused)),
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
        let applied = Applied::of(&change);
        // Read before the core applies it: an app's windows are the ones the core holds.
        #[cfg(feature = "roc-core")]
        let event = applied.event(&self.core);
        applied.apply(&mut self.core);
        #[cfg(feature = "roc-core")]
        self.roc_stepped("update", event, Answer::Done);
    }

    fn edit_layout(&mut self, layout: LayoutId, edit: LayoutEdit) -> EditResult {
        match core_edit(edit) {
            Some(edit) => match self.core_edit(layout, edit) {
                core::Edited::Unsupported => EditResult::Unsupported,
                core::Edited::Unchanged => EditResult::Unchanged,
                core::Edited::Changed => EditResult::Changed,
                core::Edited::Raise(windows) => EditResult::Raise(windows),
            },
            // An edit upstream's stack system has no case for.
            None => EditResult::Unsupported,
        }
    }
}

/// A window change as the core takes it.
enum Applied {
    Add(LayoutId, WindowId),
    Remove(WindowId),
    Replace(WindowId, WindowId),
    CloseApp(pid_t),
    Resize(LayoutId, WindowId, Resize),
}

impl Applied {
    fn of(change: &WindowChange<'_>) -> Self {
        use crate::layout_engine::core_rect;
        match *change {
            WindowChange::Add(layout, window, _) => Applied::Add(layout, window),
            WindowChange::Remove(window) | WindowChange::RemoveAndRebalance(window) => {
                Applied::Remove(window)
            }
            WindowChange::Replace(from, to) => Applied::Replace(from, to),
            WindowChange::CloseApp(pid) => Applied::CloseApp(pid),
            WindowChange::Resize {
                layout,
                window,
                old,
                new,
                screen,
                gaps,
            } => Applied::Resize(layout, window, Resize {
                old: core_rect(old),
                new: core_rect(new),
                screen: core_rect(screen),
                tiling: core_rect(crate::layout_engine::utils::compute_tiling_area(screen, gaps)),
            }),
        }
    }

    fn apply(&self, core: &mut CoreStacks) {
        match *self {
            Applied::Add(layout, window) => core.add_window(layout, window),
            Applied::Remove(window) => core.remove_window(window),
            Applied::Replace(from, to) => core.replace_window(from, to),
            Applied::CloseApp(pid) => core.close_app(|window| window.pid == pid),
            Applied::Resize(layout, window, resize) => core.resize_window(layout, window, resize),
        }
    }

    /// The change as the Roc stacks take it (an app closing lists the app's windows the core
    /// holds, as Roc has no pid).
    #[cfg(feature = "roc-core")]
    fn event(&self, core: &CoreStacks) -> Event {
        match *self {
            Applied::Add(layout, window) => {
                Event::AddWindow(roc::layout_key(layout), roc::window(window))
            }
            Applied::Remove(window) => Event::RemoveWindow(roc::window(window)),
            Applied::Replace(from, to) => Event::ReplaceWindow {
                from: roc::window(from),
                to: roc::window(to),
            },
            Applied::CloseApp(pid) => {
                Event::CloseApp(roc::app_windows(core, |window| window.pid == pid))
            }
            Applied::Resize(layout, window, resize) => {
                Event::ResizeWindow(roc::layout_key(layout), roc::window(window), resize)
            }
        }
    }
}

/// An edit as the core takes it; none for those upstream's stack system has no case for.
fn core_edit(edit: LayoutEdit) -> Option<core::Edit<WindowId>> {
    Some(match edit {
        LayoutEdit::Move {
            direction,
            has_display_neighbor,
        } => core::Edit::Move {
            direction: core_direction(direction),
            beyond: if has_display_neighbor {
                Beyond::AnotherDisplay
            } else {
                Beyond::Nothing
            },
        },
        LayoutEdit::Fullscreen { within_gaps } => core::Edit::Fullscreen { within_gaps },
        LayoutEdit::Swap(a, b) => core::Edit::Swap(a, b),
        LayoutEdit::ReorderTab { window, target } => core::Edit::ReorderTab { window, target },
        LayoutEdit::Drop { source, target, .. } => core::Edit::Drop { source, target },
        LayoutEdit::Group(GroupEdit::Stack(_)) => core::Edit::Restack,
        LayoutEdit::ToggleOrientation => core::Edit::ToggleOrientation,
        LayoutEdit::Split(_)
        | LayoutEdit::Resize { .. }
        | LayoutEdit::Rebalance
        | LayoutEdit::Group(
            GroupEdit::Join(_)
            | GroupEdit::ConsumeOrExpel(_)
            | GroupEdit::Unstack(_)
            | GroupEdit::Unjoin,
        ) => return None,
    })
}

/// The core's container view as rift-cli's node: the stack, then each member.
fn container_node(container: core::Container<WindowId>) -> rift_protocol::ContainerTreeNode {
    use rift_protocol::{ContainerNodeType, ContainerTreeNode as Node};
    let flags = |fill: Fill| (fill == Fill::Screen, fill == Fill::WithinGaps);
    let (is_fullscreen, is_fullscreen_within_gaps) = flags(container.fill);
    Node {
        node_id: container.stack.word(),
        layout_kind: Some(layout_kind(container.orientation)),
        is_selected: container.selected,
        is_fullscreen,
        is_fullscreen_within_gaps,
        children: container
            .members
            .into_iter()
            .map(|held| {
                let (is_fullscreen, is_fullscreen_within_gaps) = flags(held.member.fill);
                Node {
                    node_id: held.member.id.word(),
                    node_type: ContainerNodeType::Window,
                    window_id: Some(held.member.window.into()),
                    weight: Some(held.member.weight),
                    is_selected: held.selected,
                    is_fullscreen,
                    is_fullscreen_within_gaps,
                    ..Default::default()
                }
            })
            .collect(),
        ..Default::default()
    }
}

fn default_orientation(orientation: StackDefaultOrientation) -> DefaultOrientation {
    match orientation {
        StackDefaultOrientation::Perpendicular => DefaultOrientation::Perpendicular,
        StackDefaultOrientation::Same => DefaultOrientation::Same,
        StackDefaultOrientation::Horizontal => DefaultOrientation::Horizontal,
        StackDefaultOrientation::Vertical => DefaultOrientation::Vertical,
    }
}

fn node_id(id: Id) -> NodeId { NodeId::from(slotmap::KeyData::from_ffi(id.word())) }

fn core_id(id: NodeId) -> Id { Id::from_word(id.data().as_ffi()) }

fn layout_kind(orientation: Orientation) -> LayoutKind {
    match orientation {
        Orientation::Horizontal => LayoutKind::HorizontalStack,
        Orientation::Vertical => LayoutKind::VerticalStack,
    }
}

/// The saved form's two fullscreen flags as one fill (`fullscreen` first, as upstream's
/// projection read them; the load warns of a stack or member holding both).
fn core_fill(fullscreen: bool, within_gaps: bool) -> Fill {
    match (fullscreen, within_gaps) {
        (true, _) => Fill::Screen,
        (false, true) => Fill::WithinGaps,
        (false, false) => Fill::Tiled,
    }
}

/// A saved stack as the core holds it (the load leaves a stack kind, and a selection naming a
/// member whenever there are members).
fn core_stack(stack: &Stack) -> CoreStack<WindowId> {
    let members = stack
        .members
        .iter()
        .map(|m: &Member| CoreMember {
            id: core_id(m.id),
            window: m.window,
            weight: m.weight,
            fill: core_fill(m.fullscreen, m.within_gaps),
        })
        .collect();
    CoreStack {
        id: core_id(stack.id),
        members: match (NonEmpty::new(members), stack.active) {
            (None, _) => Members::Empty,
            (Some(members), Some(id)) => Members::Populated {
                members,
                selection: if stack.root_selected {
                    Selection::Root(core_id(id))
                } else {
                    Selection::Member(core_id(id))
                },
            },
            (Some(_), None) => {
                unreachable!("the load selects a member of a populated stack")
            }
        },
        orientation: if stack.kind == LayoutKind::HorizontalStack {
            Orientation::Horizontal
        } else {
            Orientation::Vertical
        },
        fill: core_fill(stack.fullscreen, stack.within_gaps),
    }
}

/// The saved stack a vacant slot holds while a table is read (never read itself).
fn vacant_stack() -> Stack {
    Stack {
        id: NodeId::default(),
        members: Vec::new(),
        active: None,
        root_selected: false,
        kind: LayoutKind::VerticalStack,
        fullscreen: false,
        within_gaps: false,
    }
}

/// A saved system, loaded, as the core holds it (no window left out and the default insertion
/// point: neither is saved, and rift sets the insertion point again).
fn core_of_loaded(loaded: &Loaded) -> CoreStacks {
    CoreStacks::from_parts(
        of_slotmap(&loaded.layouts, vacant_stack(), core_stack),
        loaded.next_node,
        default_orientation(loaded.default_orientation),
        BTreeSet::new(),
        insertion_point(WindowInsertionPoint::default()),
    )
}

/// A core stack as upstream saved one: the selection as `active` and `root_selected`, each fill as
/// two flags, the orientation as a stack layout kind.
fn saved_stack(stack: &CoreStack<WindowId>) -> Stack {
    let flags = |fill: Fill| (fill == Fill::Screen, fill == Fill::WithinGaps);
    let (fullscreen, within_gaps) = flags(stack.fill);
    let selection = stack.selection();
    Stack {
        id: node_id(stack.id),
        members: stack
            .list()
            .iter()
            .map(|member| {
                let (fullscreen, within_gaps) = flags(member.fill);
                Member {
                    id: node_id(member.id),
                    window: member.window,
                    weight: member.weight,
                    fullscreen,
                    within_gaps,
                }
            })
            .collect(),
        active: selection.map(|chosen| node_id(chosen.member())),
        root_selected: matches!(selection, Some(Selection::Root(_))),
        kind: layout_kind(stack.orientation),
        fullscreen,
        within_gaps,
    }
}

fn stack_default_orientation(orientation: DefaultOrientation) -> StackDefaultOrientation {
    match orientation {
        DefaultOrientation::Perpendicular => StackDefaultOrientation::Perpendicular,
        DefaultOrientation::Same => StackDefaultOrientation::Same,
        DefaultOrientation::Horizontal => StackDefaultOrientation::Horizontal,
        DefaultOrientation::Vertical => StackDefaultOrientation::Vertical,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(idx: u32) -> WindowId { WindowId::new(1, idx) }

    /// A system as the operations leave it, saved in `stack_saved.ron`: vacant slots of several
    /// versions (the free list), a stack of three with a member filling the tiling area, a stack
    /// selected itself (remembering its member), a turned stack, an empty stack, a window left
    /// out (not saved).
    fn saved_fixture_system() -> StackLayoutSystem {
        let mut system = StackLayoutSystem::new(StackDefaultOrientation::Horizontal);
        let first = system.create_layout();
        let gone = system.create_layout();
        let selected = system.create_layout();
        let _empty = system.create_layout();
        system.remove_layout(gone);
        let again = system.create_layout();
        system.remove_layout(again);
        let twice = system.create_layout();
        system.remove_layout(twice);
        let (x, y) = (system.create_layout(), system.create_layout());
        system.remove_layout(x);
        system.remove_layout(y);
        for window in [w(1), w(2), w(3)] {
            system.update(WindowChange::Add(first, window, None));
        }
        system.focus(first, Focus::Select(w(2)));
        system.edit_layout(first, LayoutEdit::Fullscreen { within_gaps: true });
        for window in [w(4), w(5)] {
            system.update(WindowChange::Add(selected, window, None));
        }
        system.focus(selected, Focus::Ascend);
        system.edit_layout(selected, LayoutEdit::ToggleOrientation);
        system.set_window_participation(w(6), false);
        system
    }

    /// A system as rift saves it today (`stack_saved.ron`, written while upstream's system still
    /// stood beside the core and the core's bytes were held to upstream's): the operations save
    /// to exactly those bytes, and the file loads and saves back byte for byte.
    #[test]
    fn a_saved_system_loads_and_saves_byte_for_byte() {
        let file = include_str!("stack_saved.ron").trim_end();
        assert_eq!(ron::to_string(&saved_fixture_system()).unwrap(), file);
        let mut loaded: StackLayoutSystem = ron::from_str(file).unwrap();
        assert_eq!(ron::to_string(&loaded).unwrap(), file);
        // A weight other than 1 (only the legacy forest writes one) loads and saves back too.
        let weighted = file.replacen("weight:1.0", "weight:2.5", 1);
        let reloaded: StackLayoutSystem = ron::from_str(&weighted).unwrap();
        assert_eq!(ron::to_string(&reloaded).unwrap(), weighted);
        // The loaded stacks take their vacant slots as the saved form's slotmap does once loaded
        // (the highest first), and mint the next member from the saved counter.
        let stored: StoredStackSystem = ron::from_str(file).unwrap();
        let mut loaded_form = Loaded::try_from(stored).unwrap();
        let made = loaded.create_layout();
        let expected = loaded_form.layouts.insert(vacant_stack());
        assert_eq!(made, expected);
        loaded.update(WindowChange::Add(made, w(7), None));
        let member = loaded.container_tree(made).children[0].node_id;
        // The stack took the counter's next id, its member the one after.
        assert_eq!(member, (1 << 32) | u64::from(loaded_form.next_node + 1));
    }

    /// What rift pays per stack operation and layout pass at 10, 100 and 1000 windows (release:
    /// `cargo test --release --lib measure_stack_system -- --ignored --nocapture`, and again with
    /// `--features roc-core`): everyday operations (focus moves, swaps, fullscreen, a window added
    /// and removed) through the system, and a layout pass. A plain build pays the core's stacks
    /// alone; a `roc-core` build pays the deciding Roc stacks, the Rust stacks as their shadow and
    /// the view comparison on every operation, and both geometries on a pass.
    #[test]
    #[ignore]
    fn measure_stack_system() {
        use std::hint::black_box;
        use std::time::Instant;

        let directions = [
            Direction::Left,
            Direction::Up,
            Direction::Right,
            Direction::Down,
        ];
        let ops = 4000_u32;
        for count in [10_u32, 100, 1000] {
            let mut system = StackLayoutSystem::new(StackDefaultOrientation::Horizontal);
            let layout = system.create_layout();
            for idx in 1..=count {
                system.update(WindowChange::Add(layout, w(idx), None));
            }
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
                    2 => {
                        drop(black_box(system.edit_layout(layout, LayoutEdit::Fullscreen {
                            within_gaps: false,
                        })))
                    }
                    _ => {
                        system.update(WindowChange::Add(layout, w(count + 1), None));
                        system.update(WindowChange::Remove(w(count + 1)));
                    }
                }
            }
            let op_each = started.elapsed() / ops;
            let screen = CGRect::new(
                objc2_core_foundation::CGPoint::ZERO,
                objc2_core_foundation::CGSize::new(1600.0, 1000.0),
            );
            let (gaps, constraints, line) = (
                GapSettings::default(),
                HashMap::default(),
                StackLineReserve::default(),
            );
            let rounds = 200;
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(system.calculate_layout(
                    layout,
                    screen,
                    12.0,
                    &constraints,
                    &gaps,
                    line,
                    crate::layout_engine::Shadow::Run,
                ));
            }
            let pass_each = started.elapsed() / rounds;
            eprintln!("{count} windows: {op_each:?} an operation, {pass_each:?} a layout pass");
        }
    }
}
