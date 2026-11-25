//! The stack layout system's stacks (`rift_core::layout::stack_tree`) as the platform's boundary
//! types and back, and the box holding them in Roc. Unsafe operations on Roc memory as in the
//! rest of [`super`].

use std::mem::ManuallyDrop;

use rift_core::layout::WindowConstraints;
use rift_core::layout::stack_tree::{
    Beyond, DefaultOrientation, Edit, Edited, Fill, Focused, GroupContainer, Id, InsertionPoint,
    Member, Members, NonEmpty, Participation, Resize, Selection, Stack,
};
use rift_core::plain::{Rect, WindowId};

use super::{
    frames, from_maybe_word, from_orientation, gaps, insertion, list, rect, stack_line,
    traditional_fill, tree_direction, tree_focus, tree_orientation, tree_query, window_constraints,
};
use crate::abi;
use crate::bsp_tree::LayoutKey;
use crate::stack_tree::{Answer, Event, Pass, Scope, StacksView};

fn default_orientation(orientation: DefaultOrientation) -> abi::BoundaryStackDefault {
    match orientation {
        DefaultOrientation::Perpendicular => abi::BoundaryStackDefault::Perpendicular,
        DefaultOrientation::Same => abi::BoundaryStackDefault::Same,
        DefaultOrientation::Horizontal => abi::BoundaryStackDefault::Horizontal,
        DefaultOrientation::Vertical => abi::BoundaryStackDefault::Vertical,
    }
}

fn from_default_orientation(orientation: abi::BoundaryStackDefault) -> DefaultOrientation {
    match orientation {
        abi::BoundaryStackDefault::Perpendicular => DefaultOrientation::Perpendicular,
        abi::BoundaryStackDefault::Same => DefaultOrientation::Same,
        abi::BoundaryStackDefault::Horizontal => DefaultOrientation::Horizontal,
        abi::BoundaryStackDefault::Vertical => DefaultOrientation::Vertical,
    }
}

fn settings(orientation: DefaultOrientation, point: InsertionPoint) -> abi::BoundaryStackSettings {
    abi::BoundaryStackSettings {
        orientation: default_orientation(orientation),
        insertion: insertion(point),
    }
}

fn from_fill(fill: abi::BoundaryFill) -> Fill {
    match fill {
        abi::BoundaryFill::Tiled => Fill::Tiled,
        abi::BoundaryFill::Screen => Fill::Screen,
        abi::BoundaryFill::WithinGaps => Fill::WithinGaps,
    }
}

fn member(member: &Member<u64>) -> abi::BoundaryStackMember {
    abi::BoundaryStackMember {
        id: member.id.word(),
        window: member.window,
        weight: member.weight,
        fill: traditional_fill(member.fill),
    }
}

fn from_member(member: &abi::BoundaryStackMember) -> Member<u64> {
    Member {
        id: Id::from_word(member.id),
        window: member.window,
        weight: member.weight,
        fill: from_fill(member.fill),
    }
}

fn selection(selection: Selection) -> abi::BoundaryStackSelection {
    use abi::BoundaryStackSelectionTag as Tag;
    match selection {
        Selection::Member(id) => union!(
            BoundaryStackSelection,
            BoundaryStackSelectionPayload.member(id.word()),
            Tag::Member
        ),
        Selection::Root(id) => union!(
            BoundaryStackSelection,
            BoundaryStackSelectionPayload.root(id.word()),
            Tag::Root
        ),
    }
}

fn from_selection(selection: &abi::BoundaryStackSelection) -> Selection {
    use abi::BoundaryStackSelectionTag as Tag;
    match selection.tag {
        // SAFETY (each arm): the tag says the payload is the member's id.
        Tag::Member => Selection::Member(Id::from_word(unsafe {
            *selection.borrow_payload_member_unchecked()
        })),
        Tag::Root => Selection::Root(Id::from_word(unsafe {
            *selection.borrow_payload_root_unchecked()
        })),
    }
}

fn stack(stack: &Stack<u64>, host: &abi::RocHost) -> abi::BoundaryStackTreeStack {
    use abi::BoundaryStackMembersTag as Tag;
    abi::BoundaryStackTreeStack {
        id: stack.id.word(),
        members: match &stack.members {
            Members::Empty => {
                union!(
                    BoundaryStackMembers,
                    BoundaryStackMembersPayload.empty,
                    Tag::Empty
                )
            }
            Members::Populated { members, selection: chosen } => {
                let (first, rest) =
                    members.as_slice().split_first().expect("a non-empty list has a first element");
                let rest: Vec<abi::BoundaryStackMember> = rest.iter().map(member).collect();
                union!(
                    BoundaryStackMembers,
                    BoundaryStackMembersPayload.populated(abi::BoundaryStackPopulated {
                        first: member(first),
                        rest: list(&rest, host),
                        selection: selection(*chosen),
                    }),
                    Tag::Populated
                )
            }
        },
        orientation: tree_orientation(stack.orientation),
        fill: traditional_fill(stack.fill),
    }
}

fn from_stack(stack: &abi::BoundaryStackTreeStack) -> Stack<u64> {
    let members = match stack.members.tag {
        abi::BoundaryStackMembersTag::Empty => Members::Empty,
        abi::BoundaryStackMembersTag::Populated => {
            // SAFETY: the tag says the payload is the populated stack; its list is only read.
            let populated = unsafe { stack.members.borrow_payload_populated_unchecked() };
            let mut all = vec![from_member(&populated.first)];
            all.extend(populated.rest.as_slice().iter().map(from_member));
            Members::Populated {
                members: NonEmpty::new(all).expect("the first member is there"),
                selection: from_selection(&populated.selection),
            }
        }
    };
    Stack {
        id: Id::from_word(stack.id),
        members,
        orientation: from_orientation(stack.orientation),
        fill: from_fill(stack.fill),
    }
}

/// A system as the platform holds it, for `roc_stack_tree_seed`.
fn seed(view: &StacksView, host: &abi::RocHost) -> abi::BoundaryStackTreeSeed {
    let layouts: Vec<abi::BoundaryStackLayoutSlot> = view
        .layouts
        .iter()
        .map(|(held, version)| match held {
            Some(held) => union!(
                BoundaryStackLayoutSlot,
                BoundaryStackLayoutSlotPayload.occupied(abi::BoundaryStackLayoutHeld {
                    version: *version,
                    stack: stack(held, host),
                }),
                abi::BoundaryStackLayoutSlotTag::Occupied
            ),
            None => union!(
                BoundaryStackLayoutSlot,
                BoundaryStackLayoutSlotPayload.vacant(*version),
                abi::BoundaryStackLayoutSlotTag::Vacant
            ),
        })
        .collect();
    abi::BoundaryStackTreeSeed {
        layouts: list(&layouts, host),
        layout_free: list(&view.layout_free, host),
        next_node: view.next_node,
        settings: settings(view.orientation, view.insertion),
        left_out: list(&view.left_out, host),
    }
}

fn edit(edit: Edit<u64>) -> abi::BoundaryStackEdit {
    use abi::BoundaryStackEditTag as Tag;
    match edit {
        Edit::Move { direction, beyond } => union!(
            BoundaryStackEdit,
            BoundaryStackEditPayload.moved(abi::BoundaryStackMove {
                direction: tree_direction(direction),
                beyond: match beyond {
                    Beyond::AnotherDisplay => abi::BoundaryStackBeyond::AnotherDisplay,
                    Beyond::Nothing => abi::BoundaryStackBeyond::Nothing,
                },
            }),
            Tag::Moved
        ),
        Edit::Fullscreen { within_gaps } => union!(
            BoundaryStackEdit,
            BoundaryStackEditPayload.fullscreen(if within_gaps {
                abi::BoundaryBspFullscreen::WithinGaps
            } else {
                abi::BoundaryBspFullscreen::Screen
            }),
            Tag::Fullscreen
        ),
        Edit::Swap(a, b) => union!(
            BoundaryStackEdit,
            BoundaryStackEditPayload.swap(abi::BoundaryBspPair { a, b }),
            Tag::Swap
        ),
        Edit::ReorderTab { window, target } => union!(
            BoundaryStackEdit,
            BoundaryStackEditPayload.reorder_tab(abi::BoundaryStackTab { window, target }),
            Tag::ReorderTab
        ),
        Edit::Drop { source, target } => union!(
            BoundaryStackEdit,
            BoundaryStackEditPayload.drop(abi::BoundaryBspPair { a: source, b: target }),
            Tag::Drop
        ),
        Edit::Restack => union!(BoundaryStackEdit, BoundaryStackEditPayload.restack, Tag::Restack),
        Edit::ToggleOrientation => union!(
            BoundaryStackEdit,
            BoundaryStackEditPayload.toggle_orientation,
            Tag::ToggleOrientation
        ),
    }
}

fn event(event: Event, host: &abi::RocHost) -> abi::BoundaryStackTreeEvent {
    use abi::BoundaryStackTreeEventTag as Tag;
    let window_in =
        |layout: LayoutKey, window| abi::BoundaryBspWindowIn { layout: layout.0, window };
    match event {
        Event::CreateLayout => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.create_layout,
            Tag::CreateLayout
        ),
        Event::CloneLayout(layout) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.clone_layout(layout.0),
            Tag::CloneLayout
        ),
        Event::RemoveLayout(layout) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.remove_layout(layout.0),
            Tag::RemoveLayout
        ),
        Event::Participation(window, participation) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.participation(abi::BoundaryBspParticipation {
                window,
                participation: match participation {
                    Participation::TakesPart => abi::BoundaryBspTakesPart::TakesPart,
                    Participation::LeftOut => abi::BoundaryBspTakesPart::LeftOut,
                },
            }),
            Tag::Participation
        ),
        Event::Settings(orientation, point) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.settings(settings(orientation, point)),
            Tag::Settings
        ),
        Event::AddWindow(layout, window) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.add_window(window_in(layout, window)),
            Tag::AddWindow
        ),
        Event::RemoveWindow(window) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.remove_window(window),
            Tag::RemoveWindow
        ),
        Event::ReplaceWindow { from, to } => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.replace_window(abi::BoundaryBspReplacement { from, to }),
            Tag::ReplaceWindow
        ),
        Event::CloseApp(windows) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.close_app(list(&windows, host)),
            Tag::CloseApp
        ),
        Event::ResizeWindow(layout, window, Resize { old, new, screen, tiling }) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.resize_window(abi::BoundaryBspNativeResize {
                layout: layout.0,
                window,
                old: rect(old),
                new: rect(new),
                screen: rect(screen),
                tiling: rect(tiling),
            }),
            Tag::ResizeWindow
        ),
        Event::ContainsLayout(layout) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.contains_layout(layout.0),
            Tag::ContainsLayout
        ),
        Event::WindowParticipates(window) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.window_participates(window),
            Tag::WindowParticipates
        ),
        Event::SelectedWindow(layout) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.selected_window(layout.0),
            Tag::SelectedWindow
        ),
        Event::SelectionContainsWindow(layout, window) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.selection_contains_window(window_in(layout, window)),
            Tag::SelectionContainsWindow
        ),
        Event::WindowInDirection(layout, direction) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.window_in_direction(abi::BoundaryBspLayoutDirection {
                layout: layout.0,
                direction: tree_direction(direction),
            }),
            Tag::WindowInDirection
        ),
        Event::ContainsWindow(layout, window) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.contains_window(window_in(layout, window)),
            Tag::ContainsWindow
        ),
        Event::HasAnyFullscreenNode(layout) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.has_any_fullscreen_node(layout.0),
            Tag::HasAnyFullscreenNode
        ),
        Event::HasWindowsForApp(layout, windows) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.has_windows_for_app(abi::BoundaryBspAppIn {
                layout: layout.0,
                windows: list(&windows, host),
            }),
            Tag::HasWindowsForApp
        ),
        Event::Windows(layout, query) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.windows(abi::BoundaryBspWindowsQuery {
                layout: layout.0,
                query: tree_query(query),
            }),
            Tag::Windows
        ),
        Event::WindowSlot(layout, window) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.window_slot(window_in(layout, window)),
            Tag::WindowSlot
        ),
        Event::GroupContainer(layout, scope) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.group_container(abi::BoundaryStackGroupIn {
                layout: layout.0,
                scope: match scope {
                    Scope::Whole => abi::BoundaryStackScope::Whole,
                    Scope::SelectionPath => abi::BoundaryStackScope::SelectionPath,
                },
            }),
            Tag::GroupContainer
        ),
        Event::Focus(layout, focus) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.focus(abi::BoundaryBspFocusIn {
                layout: layout.0,
                focus: tree_focus(focus),
            }),
            Tag::Focus
        ),
        Event::Edit(layout, asked) => union!(
            BoundaryStackTreeEvent,
            BoundaryStackTreeEventPayload.edit(abi::BoundaryStackEditIn {
                layout: layout.0,
                edit: edit(asked),
            }),
            Tag::Edit
        ),
    }
}

fn from_answer(answer: &abi::BoundaryStackTreeAnswer) -> Answer {
    use abi::BoundaryStackTreeAnswerTag as Tag;
    match answer.tag {
        // SAFETY (each arm): the tag says which payload it is; lists are only read.
        Tag::Layout => {
            Answer::Layout(LayoutKey(unsafe { *answer.borrow_payload_layout_unchecked() }))
        }
        Tag::Participated => {
            Answer::Participated(unsafe { *answer.borrow_payload_participated_unchecked() })
        }
        Tag::Done => Answer::Done,
        Tag::Verdict => Answer::Verdict(unsafe { *answer.borrow_payload_verdict_unchecked() }),
        Tag::Window => Answer::Window(from_maybe_word(unsafe {
            *answer.borrow_payload_window_unchecked()
        })),
        Tag::Windows => Answer::Windows(unsafe {
            answer.borrow_payload_windows_unchecked().as_slice().to_vec()
        }),
        Tag::WindowSlot => {
            let slot = unsafe { answer.borrow_payload_window_slot_unchecked() };
            Answer::WindowSlot(match slot.tag {
                abi::BoundaryStackSlotTag::NoSlot => None,
                abi::BoundaryStackSlotTag::Slot => {
                    Some(unsafe { *slot.borrow_payload_slot_unchecked() } as usize)
                }
            })
        }
        Tag::Group => {
            let group = unsafe { answer.borrow_payload_group_unchecked() };
            Answer::Group(match group.tag {
                abi::BoundaryStackGroupAnswerTag::NoGroup => None,
                abi::BoundaryStackGroupAnswerTag::Group => {
                    let held = unsafe { group.borrow_payload_group_unchecked() };
                    Some(GroupContainer {
                        stack: Id::from_word(held.stack),
                        orientation: from_orientation(held.orientation),
                        windows: held.windows.as_slice().to_vec(),
                        selected: held.selected as usize,
                    })
                }
            })
        }
        Tag::Focused => {
            let focused = unsafe { answer.borrow_payload_focused_unchecked() };
            Answer::Focused(match focused.tag {
                abi::BoundaryBspFocusedTag::Selection => Focused::Selection {
                    handled: unsafe { *focused.borrow_payload_selection_unchecked() },
                },
                abi::BoundaryBspFocusedTag::Movement => {
                    let movement = unsafe { focused.borrow_payload_movement_unchecked() };
                    Focused::Movement {
                        window: from_maybe_word(movement.window),
                        raise: movement.raise.as_slice().to_vec(),
                    }
                }
            })
        }
        Tag::Edited => {
            let edited = unsafe { answer.borrow_payload_edited_unchecked() };
            Answer::Edited(match edited.tag {
                abi::BoundaryBspEditedTag::Unsupported => Edited::Unsupported,
                abi::BoundaryBspEditedTag::Unchanged => Edited::Unchanged,
                abi::BoundaryBspEditedTag::Changed => Edited::Changed,
                abi::BoundaryBspEditedTag::Raise => Edited::Raise(
                    unsafe { edited.borrow_payload_raise_unchecked() }.as_slice().to_vec(),
                ),
            })
        }
    }
}

/// The stacks' box, which only Roc can read or free; only the `roc_stack_tree_*` entries make
/// and take one. Dropping it releases the stacks.
pub(crate) struct StackTreeBox(abi::RocBox);

impl StackTreeBox {
    pub(crate) fn new(orientation: DefaultOrientation, point: InsertionPoint) -> StackTreeBox {
        StackTreeBox(unsafe { abi::roc_stack_tree_init(settings(orientation, point)) })
    }

    /// The system `view` holds, which the caller has parsed (`StacksView::stacks`).
    pub(crate) fn seeded(view: &StacksView, host: &abi::RocHost) -> StackTreeBox {
        // SAFETY: the seed's lists are allocated through the host and consumed by Roc.
        StackTreeBox(unsafe { abi::roc_stack_tree_seed(seed(view, host)) })
    }

    /// The raw box, handed to Roc (see `LifecycleBox::into_raw`).
    fn into_raw(self) -> abi::RocBox {
        let raw = self.0;
        std::mem::forget(self);
        raw
    }

    /// One operation: the stacks after it, and what it answered.
    pub(crate) fn step(self, asked: Event, host: &abi::RocHost) -> (StackTreeBox, Answer) {
        // SAFETY: the box is these stacks', given to the step; its successor comes back. The
        // answer is read, then released.
        let stepped = unsafe { abi::roc_stack_tree_step(self.into_raw(), event(asked, host)) };
        let answer = from_answer(&stepped.answer);
        unsafe { stepped.answer.decref(host) };
        (StackTreeBox(stepped.model), answer)
    }

    /// A layout's frames, as the traditional geometry gives them for the stacks' own shape: the
    /// pass's screen, gaps, stack offset and line, and every window's constraints the caller
    /// knows (by the window's word).
    pub(crate) fn frames(
        self,
        layout: LayoutKey,
        pass: Pass,
        known: &[(u64, WindowConstraints)],
        host: &abi::RocHost,
    ) -> (StackTreeBox, Vec<(WindowId, Rect)>) {
        let known: Vec<abi::BoundaryBspKnown> = known
            .iter()
            .map(|&(window, held)| abi::BoundaryBspKnown {
                window,
                constraints: window_constraints(held),
            })
            .collect();
        let question = abi::BoundaryStackFramesQuestion {
            layout: layout.0,
            screen: rect(pass.screen),
            gaps: gaps(pass.gaps),
            stack_offset: pass.stack_offset,
            stack_line: stack_line(pass.stack_line),
            known: list(&known, host),
        };
        // SAFETY: as in `step`; the question's list is allocated through the host and consumed
        // by Roc; the frames are Roc's answer, read in full and released by `frames`.
        let framed = unsafe { abi::roc_stack_tree_frames(self.into_raw(), question) };
        (StackTreeBox(framed.model), frames(framed.frames, host))
    }

    /// The table's slots and free list, the counter, the settings, the windows left out.
    pub(crate) fn view(self, host: &abi::RocHost) -> (StackTreeBox, StacksView) {
        // SAFETY: as in `step`; the lists are Roc's answer, owned here, read in full, released.
        let viewed = unsafe { abi::roc_stack_tree_view(self.into_raw()) };
        let layouts = viewed
            .layouts
            .as_slice()
            .iter()
            .map(|slot| match slot.tag {
                abi::BoundaryStackLayoutSlotTag::Occupied => {
                    // SAFETY: the tag says the payload is the held stack.
                    let held = unsafe { slot.borrow_payload_occupied_unchecked() };
                    (Some(from_stack(&held.stack)), held.version)
                }
                // SAFETY: the tag says the payload is the vacant version.
                abi::BoundaryStackLayoutSlotTag::Vacant => {
                    (None, unsafe { *slot.borrow_payload_vacant_unchecked() })
                }
            })
            .collect();
        let view = StacksView {
            layouts,
            layout_free: viewed.layout_free.as_slice().to_vec(),
            next_node: viewed.next_node,
            orientation: from_default_orientation(viewed.settings.orientation),
            insertion: super::from_insertion(viewed.settings.insertion),
            left_out: viewed.left_out.as_slice().to_vec(),
        };
        unsafe {
            abi::decref_list_of_boundary_stack_layout_slot(viewed.layouts, host);
            viewed.layout_free.decref(host);
            viewed.left_out.decref(host);
        }
        (StackTreeBox(viewed.model), view)
    }
}

impl Drop for StackTreeBox {
    fn drop(&mut self) {
        // SAFETY: the box is these stacks' and is never used again.
        unsafe { abi::roc_stack_tree_release(self.0) }
    }
}
