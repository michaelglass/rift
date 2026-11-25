//! A stack layout system's stacks as the core holds them: upstream's (#688) `StackLayoutSystem`,
//! its state and its operations, ported operation by operation. One value per system, as
//! upstream keeps it: every layout's stack in one slot table (ids equal upstream's slotmap ids
//! for the same operations), and member ids minted from one saved counter (`next_node`, the
//! lead's ruling: byte-compatible, so ids are upstream's words).
//!
//! Upstream's rules are the spec; where this module's types differ from upstream's (the
//! selection as one [`Selection`] instead of `active` and `root_selected`, each pair of
//! fullscreen flags as one [`Fill`], a stack's orientation as an [`Orientation`] instead of a
//! layout kind), they hold the states upstream's operations reach.

use std::collections::BTreeSet;

pub use super::bsp_tree::{
    Direction, Edited, Focus, Focused, InsertionPoint, Orientation, Participation, Query, Resize,
};
pub use super::traditional::Fill;
use super::traditional::{self, StackLine, Traditional};
use super::{Gaps, WindowConstraints};
use crate::plain::Rect;
use crate::slot_table::{SlotKey, SlotTable};

/// A stack's or a member's id: a word as upstream's `NodeId` writes it (`version << 32 |
/// index`). Minted ids are version 1 over the counter; a stack loaded from the old forest keeps
/// the forest's ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(u64);

impl Id {
    pub fn from_word(word: u64) -> Self { Id(word) }

    pub fn word(self) -> u64 { self.0 }

    fn minted(index: u32) -> Self { Id((1 << 32) | u64::from(index)) }
}

/// The orientation a new stack takes (upstream's `StackDefaultOrientation`; `Same` and
/// `Horizontal` make a horizontal stack, the others a vertical one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultOrientation {
    Perpendicular,
    Same,
    Horizontal,
    Vertical,
}

/// What a stack's selection is: nothing (an empty stack), a member, or the stack itself, which
/// remembers the member it was selected from (that member is still the selected window, and
/// descending returns to it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    Nothing,
    Member(Id),
    Root(Id),
}

/// A window in a stack: its id, its weight (a saved share; no operation changes it), and how it
/// fills the screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Member<W> {
    pub id: Id,
    pub window: W,
    pub weight: f64,
    pub fill: Fill,
}

/// One layout's stack. The selection is [`Selection::Nothing`] exactly when there are no
/// members, and otherwise names one of them.
#[derive(Debug, Clone, PartialEq)]
pub struct Stack<W> {
    pub id: Id,
    pub members: Vec<Member<W>>,
    pub selection: Selection,
    pub orientation: Orientation,
    pub fill: Fill,
}

/// Where a move past the stack's end would go, beyond the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beyond {
    AnotherDisplay,
    Nothing,
}

/// The edits a stack has a case for (upstream's `LayoutEdit`, the others unsupported).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Edit<W> {
    Move {
        direction: Direction,
        beyond: Beyond,
    },
    Fullscreen {
        within_gaps: bool,
    },
    Swap(W, W),
    ReorderTab {
        window: W,
        target: W,
    },
    Drop {
        source: W,
        target: W,
    },
    /// Stacking the selection: a stack turns its orientation and raises the shown window.
    Restack,
    ToggleOrientation,
}

/// A stack as the group bar shows it: its windows taking part, and the shown one's position
/// among them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupContainer<W> {
    pub stack: Id,
    pub orientation: Orientation,
    pub windows: Vec<W>,
    pub selected: usize,
}

/// A stack layout system: every layout's stack, the member id counter, the windows left out of
/// layout, and its settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Stacks<L, W> {
    layouts: SlotTable<L, Stack<W>>,
    next_node: u32,
    orientation: DefaultOrientation,
    left_out: BTreeSet<W>,
    insertion: InsertionPoint,
}

impl<W: Copy + Ord> Stack<W> {
    fn active(&self) -> Option<Id> {
        match self.selection {
            Selection::Nothing => None,
            Selection::Member(id) | Selection::Root(id) => Some(id),
        }
    }

    fn root_selected(&self) -> bool { matches!(self.selection, Selection::Root(_)) }

    fn position(&self, id: Id) -> Option<usize> { self.members.iter().position(|m| m.id == id) }

    fn window_position(&self, window: W) -> Option<usize> {
        self.members.iter().position(|m| m.window == window)
    }

    /// The selected member, or the first taking part when it does not.
    fn active_index(&self, left_out: &BTreeSet<W>) -> Option<usize> {
        let active = self.active();
        self.members
            .iter()
            .position(|m| Some(m.id) == active && !left_out.contains(&m.window))
            .or_else(|| self.members.iter().position(|m| !left_out.contains(&m.window)))
    }

    /// The member shown: the first taking part while the stack itself is selected.
    fn visible_index(&self, left_out: &BTreeSet<W>) -> Option<usize> {
        if self.root_selected() {
            self.members.iter().position(|m| !left_out.contains(&m.window))
        } else {
            self.active_index(left_out)
        }
    }

    /// Keeps the members `keep` keeps; a selected member that goes passes the selection to the
    /// next kept member, else the previous one (the stack itself is no longer selected).
    fn retain(&mut self, keep: impl Fn(&Member<W>) -> bool) {
        if let Some(index) = self.active().and_then(|id| self.position(id))
            && !keep(&self.members[index])
        {
            self.selection = self.members[index + 1..]
                .iter()
                .find(|m| keep(m))
                .or_else(|| self.members[..index].iter().rev().find(|m| keep(m)))
                .map_or(Selection::Nothing, |m| Selection::Member(m.id));
        }
        self.members.retain(keep);
    }

    fn insert(&mut self, member: Member<W>, insertion: InsertionPoint) {
        let index = match insertion {
            InsertionPoint::NextToSelection => {
                self.active().and_then(|id| self.position(id)).map(|i| i + 1)
            }
            InsertionPoint::EndOfTree => None,
        }
        .unwrap_or(self.members.len());
        self.selection = Selection::Member(member.id);
        self.members.insert(index, member);
    }

    /// The member taking part next to the selected one along `direction`, if the stack runs
    /// that way and a member (not the stack) is selected.
    fn neighbor(&self, direction: Direction, left_out: &BTreeSet<W>) -> Option<usize> {
        if self.root_selected() || self.orientation != direction.orientation() {
            return None;
        }
        let index = self.active_index(left_out)?;
        let takes_part = |i: &usize| !left_out.contains(&self.members[*i].window);
        match direction {
            Direction::Left | Direction::Up => (0..index).rev().find(takes_part),
            Direction::Right | Direction::Down => (index + 1..self.members.len()).find(takes_part),
        }
    }
}

impl<L: SlotKey + Eq, W: Copy + Ord> Stacks<L, W> {
    pub fn new(orientation: DefaultOrientation, insertion: InsertionPoint) -> Self {
        Self::from_parts(SlotTable::default(), 1, orientation, BTreeSet::new(), insertion)
    }

    pub fn from_parts(
        layouts: SlotTable<L, Stack<W>>,
        next_node: u32,
        orientation: DefaultOrientation,
        left_out: BTreeSet<W>,
        insertion: InsertionPoint,
    ) -> Self {
        Self {
            layouts,
            next_node,
            orientation,
            left_out,
            insertion,
        }
    }

    pub fn layouts(&self) -> &SlotTable<L, Stack<W>> { &self.layouts }

    pub fn next_node(&self) -> u32 { self.next_node }

    pub fn orientation(&self) -> DefaultOrientation { self.orientation }

    pub fn left_out(&self) -> &BTreeSet<W> { &self.left_out }

    pub fn insertion(&self) -> InsertionPoint { self.insertion }

    pub fn update_settings(&mut self, orientation: DefaultOrientation, insertion: InsertionPoint) {
        self.orientation = orientation;
        self.insertion = insertion;
    }

    fn node_id(&mut self) -> Id {
        let index = self.next_node;
        self.next_node = index.checked_add(1).expect("stack node identity exhausted");
        Id::minted(index)
    }

    pub fn window_participates(&self, window: W) -> bool { !self.left_out.contains(&window) }

    /// Whether the window's participation changed.
    pub fn set_window_participation(&mut self, window: W, participation: Participation) -> bool {
        match participation {
            Participation::TakesPart => self.left_out.remove(&window),
            Participation::LeftOut => self.left_out.insert(window),
        }
    }

    pub fn create_layout(&mut self) -> L {
        let id = self.node_id();
        let orientation = match self.orientation {
            DefaultOrientation::Horizontal | DefaultOrientation::Same => Orientation::Horizontal,
            DefaultOrientation::Perpendicular | DefaultOrientation::Vertical => {
                Orientation::Vertical
            }
        };
        self.layouts.insert(Stack {
            id,
            members: Vec::new(),
            selection: Selection::Nothing,
            orientation,
            fill: Fill::Tiled,
        })
    }

    pub fn contains_layout(&self, layout: L) -> bool { self.layouts.contains_key(layout) }

    /// A copy of the layout under fresh ids (the stack's first, then each member's in order).
    pub fn clone_layout(&mut self, layout: L) -> L {
        let mut stack = self.layouts[layout].clone();
        stack.id = self.node_id();
        for i in 0..stack.members.len() {
            let id = self.node_id();
            stack.selection = match stack.selection {
                Selection::Member(old) if old == stack.members[i].id => Selection::Member(id),
                Selection::Root(old) if old == stack.members[i].id => Selection::Root(id),
                selection => selection,
            };
            stack.members[i].id = id;
        }
        self.layouts.insert(stack)
    }

    pub fn remove_layout(&mut self, layout: L) { self.layouts.remove(layout); }

    pub fn has_windows_for_app(&self, layout: L, of_app: impl Fn(W) -> bool) -> bool {
        self.layouts[layout].members.iter().any(|m| of_app(m.window))
    }

    pub fn selected_window(&self, layout: L) -> Option<W> {
        let stack = &self.layouts[layout];
        stack.active_index(&self.left_out).map(|i| stack.members[i].window)
    }

    pub fn selection_contains_window(&self, layout: L, window: W) -> bool {
        let stack = &self.layouts[layout];
        match stack.selection {
            Selection::Nothing => false,
            Selection::Root(_) => stack.members.iter().any(|m| m.window == window),
            Selection::Member(id) => stack.members.iter().any(|m| m.id == id && m.window == window),
        }
    }

    pub fn window_in_direction(&self, layout: L, direction: Direction) -> Option<W> {
        let stack = &self.layouts[layout];
        if let Selection::Member(id) = stack.selection
            && let Some(member) =
                stack.members.iter().find(|m| m.id == id && self.window_participates(m.window))
        {
            return Some(member.window);
        }
        let mut taking_part = stack.members.iter().filter(|m| self.window_participates(m.window));
        match direction {
            Direction::Left | Direction::Up => taking_part.next_back(),
            Direction::Right | Direction::Down => taking_part.next(),
        }
        .map(|m| m.window)
    }

    pub fn contains_window(&self, layout: L, window: W) -> bool {
        self.layouts[layout].members.iter().any(|m| m.window == window)
    }

    pub fn has_any_fullscreen_node(&self, layout: L) -> bool {
        let stack = &self.layouts[layout];
        self.selected_window(layout).is_some()
            && (stack.fill != Fill::Tiled
                || stack
                    .members
                    .iter()
                    .any(|m| self.window_participates(m.window) && m.fill != Fill::Tiled))
    }

    pub fn windows(&self, layout: L, query: Query<W>) -> Vec<W> {
        match query {
            Query::All => self.all_windows(layout),
            Query::Visible | Query::Selection => self.visible_windows(layout),
            Query::Stack(window) => {
                if self.contains_window(layout, window) {
                    self.all_windows(layout)
                } else {
                    Vec::new()
                }
            }
        }
    }

    fn all_windows(&self, layout: L) -> Vec<W> {
        self.layouts[layout].members.iter().map(|m| m.window).collect()
    }

    fn visible_windows(&self, layout: L) -> Vec<W> {
        let stack = &self.layouts[layout];
        stack
            .visible_index(&self.left_out)
            .map(|i| stack.members[i].window)
            .into_iter()
            .collect()
    }

    /// The position of the window's member in the stack.
    pub fn window_slot(&self, layout: L, window: W) -> Option<usize> {
        self.layouts[layout].window_position(window)
    }

    pub fn focus(&mut self, layout: L, focus: Focus<W>) -> Focused<W> {
        match focus {
            Focus::Select(window) => Focused::Selection {
                handled: self.select_window(layout, window),
            },
            Focus::Ascend | Focus::Descend => {
                if self.selected_window(layout).is_none() {
                    return Focused::Selection { handled: false };
                }
                let stack = &mut self.layouts[layout];
                let (Selection::Member(active) | Selection::Root(active)) = stack.selection else {
                    unreachable!("a stack with a selected window has a selection")
                };
                let selection = match focus {
                    Focus::Ascend => Selection::Root(active),
                    _ => Selection::Member(active),
                };
                let handled = stack.selection != selection;
                stack.selection = selection;
                Focused::Selection { handled }
            }
            Focus::Direction(direction) => {
                let stack = &mut self.layouts[layout];
                match stack.neighbor(direction, &self.left_out) {
                    Some(i) => {
                        let member = &stack.members[i];
                        stack.selection = Selection::Member(member.id);
                        Focused::Movement {
                            window: Some(member.window),
                            raise: vec![member.window],
                        }
                    }
                    None => Focused::Movement {
                        window: None,
                        raise: Vec::new(),
                    },
                }
            }
        }
    }

    fn select_window(&mut self, layout: L, window: W) -> bool {
        if !self.window_participates(window) {
            return false;
        }
        let stack = &mut self.layouts[layout];
        let Some(member) = stack.members.iter().find(|m| m.window == window) else {
            return false;
        };
        stack.selection = Selection::Member(member.id);
        true
    }

    /// `window` put into the layout's stack, selected (nothing if the stack holds it already).
    pub fn add_window(&mut self, layout: L, window: W) {
        if self.contains_window(layout, window) {
            return;
        }
        let id = self.node_id();
        let insertion = self.insertion;
        self.layouts[layout].insert(
            Member {
                id,
                window,
                weight: 1.0,
                fill: Fill::Tiled,
            },
            insertion,
        );
    }

    pub fn remove_window(&mut self, window: W) {
        self.left_out.remove(&window);
        for (_, stack) in self.layouts.iter_mut() {
            stack.retain(|m| m.window != window);
        }
    }

    pub fn replace_window(&mut self, from: W, to: W) {
        if self.left_out.remove(&from) {
            self.left_out.insert(to);
        }
        for (_, stack) in self.layouts.iter_mut() {
            for member in &mut stack.members {
                if member.window == from {
                    member.window = to;
                }
            }
        }
    }

    pub fn close_app(&mut self, of_app: impl Fn(W) -> bool) {
        self.left_out.retain(|&window| !of_app(window));
        for (_, stack) in self.layouts.iter_mut() {
            stack.retain(|m| !of_app(m.window));
        }
    }

    pub fn resize_window(&mut self, layout: L, window: W, resize: Resize) {
        if !self.window_participates(window) {
            return;
        }
        let Some(member) = self.layouts[layout].members.iter_mut().find(|m| m.window == window)
        else {
            return;
        };
        member.fill = resized(member.fill, resize);
    }

    pub fn edit(&mut self, layout: L, edit: Edit<W>) -> Edited<W> {
        let of = |changed: bool| {
            if changed {
                Edited::Changed
            } else {
                Edited::Unchanged
            }
        };
        match edit {
            Edit::Move { direction, beyond } => of(self.move_selection(layout, direction, beyond)),
            Edit::Fullscreen { within_gaps } => {
                Edited::Raise(self.toggle_fullscreen(layout, within_gaps))
            }
            Edit::Swap(a, b) => of(self.swap_windows(layout, a, b)),
            Edit::ReorderTab { window, target } => of(self.move_tab(layout, window, target)),
            Edit::Drop { source, target } => of(source != target
                && self.swap_windows(layout, source, target)
                && self.select_window(layout, source)),
            Edit::Restack => {
                self.toggle_orientation(layout);
                Edited::Raise(self.visible_windows(layout))
            }
            Edit::ToggleOrientation => {
                self.toggle_orientation(layout);
                Edited::Changed
            }
        }
    }

    fn move_selection(&mut self, layout: L, direction: Direction, beyond: Beyond) -> bool {
        let stack = &mut self.layouts[layout];
        if stack.root_selected() {
            return false;
        }
        let Some(i) = stack.active_index(&self.left_out) else {
            return false;
        };
        let target = if let Some(target) = stack.neighbor(direction, &self.left_out) {
            target
        } else if beyond == Beyond::AnotherDisplay || stack.orientation == direction.orientation() {
            return false;
        } else {
            // Leaving the stack's axis places the member at the matching outer edge and turns
            // the whole stack onto that axis.
            stack.orientation = direction.orientation();
            match direction {
                Direction::Left | Direction::Up => 0,
                Direction::Right | Direction::Down => stack.members.len() - 1,
            }
        };
        let member = stack.members.remove(i);
        stack.selection = Selection::Member(member.id);
        stack.members.insert(target, member);
        true
    }

    fn toggle_orientation(&mut self, layout: L) {
        let stack = &mut self.layouts[layout];
        stack.orientation = match stack.orientation {
            Orientation::Horizontal => Orientation::Vertical,
            Orientation::Vertical => Orientation::Horizontal,
        };
    }

    /// The selection's fill toggled (the stack's while it is selected); the shown window is
    /// raised when a fullscreen mode turned on.
    fn toggle_fullscreen(&mut self, layout: L, within_gaps: bool) -> Vec<W> {
        if self.selected_window(layout).is_none() {
            return Vec::new();
        }
        let stack = &mut self.layouts[layout];
        let fill = if stack.root_selected() {
            &mut stack.fill
        } else {
            let index = stack.active_index(&self.left_out).expect("a selected window has a member");
            &mut stack.members[index].fill
        };
        *fill = toggled(*fill, within_gaps);
        if *fill == Fill::Tiled {
            Vec::new()
        } else {
            self.visible_windows(layout)
        }
    }

    fn swap_windows(&mut self, layout: L, a: W, b: W) -> bool {
        if !self.window_participates(a) || !self.window_participates(b) {
            return false;
        }
        let stack = &mut self.layouts[layout];
        let (Some(i), Some(j)) = (stack.window_position(a), stack.window_position(b)) else {
            return false;
        };
        if i == j {
            return false;
        }
        if let Selection::Member(id) = stack.selection {
            if id == stack.members[i].id {
                stack.selection = Selection::Member(stack.members[j].id);
            } else if id == stack.members[j].id {
                stack.selection = Selection::Member(stack.members[i].id);
            }
        }
        stack.members[i].window = b;
        stack.members[j].window = a;
        true
    }

    fn move_tab(&mut self, layout: L, window: W, target: W) -> bool {
        if !self.window_participates(window) || !self.window_participates(target) {
            return false;
        }
        let stack = &mut self.layouts[layout];
        let (Some(i), Some(j)) = (stack.window_position(window), stack.window_position(target))
        else {
            return false;
        };
        if i == j {
            return false;
        }
        let member = stack.members.remove(i);
        stack.members.insert(j, member);
        true
    }

    /// The layout as the core's geometry takes it: one stack of the members taking part, the
    /// shown one on top (none when no member takes part). The stack fills the screen only when
    /// fullscreen (upstream's projection reads that flag alone).
    pub fn shape(
        &self,
        layout: L,
        screen: Rect,
        gaps: Gaps,
        stack_offset: f64,
        stack_line: StackLine,
        constraints: impl Fn(W) -> Option<WindowConstraints>,
    ) -> Option<Traditional<W>> {
        let stack = &self.layouts[layout];
        let visible = stack.visible_index(&self.left_out)?;
        let child = |member: &Member<W>| traditional::Child {
            size: member.weight as f32,
            node: traditional::Node::Window {
                window: member.window,
                constraints: constraints(member.window),
                fill: member.fill,
            },
        };
        let taking_part = |members: &[Member<W>]| -> Vec<traditional::Child<W>> {
            members
                .iter()
                .filter(|m| self.window_participates(m.window))
                .map(child)
                .collect()
        };
        Some(Traditional {
            screen,
            area: super::tiling_area(screen, gaps),
            gaps,
            stack_offset,
            stack_line,
            root: traditional::Node::Stack {
                horizontal: stack.orientation == Orientation::Horizontal,
                fill: if stack.fill == Fill::Screen {
                    Fill::Screen
                } else {
                    Fill::Tiled
                },
                before: taking_part(&stack.members[..visible]),
                focused: Box::new(child(&stack.members[visible])),
                after: taking_part(&stack.members[visible + 1..]),
            },
        })
    }

    /// The stack as the group bar shows it; none when nothing is shown, the stack fills the
    /// screen, or (`selection_path_only`) the shown member does.
    pub fn group_container(
        &self,
        layout: L,
        selection_path_only: bool,
    ) -> Option<GroupContainer<W>> {
        let stack = &self.layouts[layout];
        let shown = stack.visible_index(&self.left_out)?;
        if stack.fill != Fill::Tiled
            || (selection_path_only && stack.members[shown].fill != Fill::Tiled)
        {
            return None;
        }
        let taking_part = |m: &&Member<W>| self.window_participates(m.window);
        Some(GroupContainer {
            stack: stack.id,
            orientation: stack.orientation,
            windows: stack.members.iter().filter(taking_part).map(|m| m.window).collect(),
            selected: stack.members[..shown].iter().filter(taking_part).count(),
        })
    }
}

/// A fill toggled (upstream's `toggle_fullscreen_mode`): the mode asked for is turned off if it
/// is on, else on (replacing the other).
fn toggled(fill: Fill, within_gaps: bool) -> Fill {
    match (fill, within_gaps) {
        (Fill::WithinGaps, true) | (Fill::Screen, false) => Fill::Tiled,
        (_, true) => Fill::WithinGaps,
        (_, false) => Fill::Screen,
    }
}

/// A fill after a native resize (upstream's `track_fullscreen_resize`): onto the screen or the
/// tiling area takes that fill, off it leaves that fill.
fn resized(fill: Fill, Resize { old, new, screen, tiling }: Resize) -> Fill {
    if new == screen {
        Fill::Screen
    } else if old == screen {
        if fill == Fill::Screen {
            Fill::Tiled
        } else {
            fill
        }
    } else if new == tiling {
        Fill::WithinGaps
    } else if old == tiling {
        if fill == Fill::WithinGaps {
            Fill::Tiled
        } else {
            fill
        }
    } else {
        fill
    }
}
