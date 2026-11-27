//! A scrolling layout's arrangements (`rift_core::layout::columns`) as the platform's boundary
//! types and back, and the box holding them in Roc. Unsafe operations on Roc memory as in the
//! rest of [`super`].

use std::mem::ManuallyDrop;

use rift_core::layout::bsp_tree::Focused;
use rift_core::layout::columns::{
    Arrangement, Column, ColumnId, DropAction, Edited, Fill, FrameResize, Fullscreened, Moved,
    Participation, Removal, ResizeAlong, Row, Settings, Span, Strip, Transfer, Width,
};

use super::{
    from_insertion, from_maybe_word, insertion, list, rect, traditional_fill, tree_direction,
    tree_focus, tree_query,
};
use crate::abi;
use crate::bsp_tree::LayoutKey;
use crate::columns::{Answer, ColumnsView, Event};

fn width(width: Width) -> abi::BoundaryColumnsWidth {
    use abi::BoundaryColumnsWidthTag as Tag;
    match width {
        Width::Default => union!(
            BoundaryColumnsWidth,
            BoundaryColumnsWidthPayload.default,
            Tag::Default
        ),
        Width::Proportion(ratio) => union!(
            BoundaryColumnsWidth,
            BoundaryColumnsWidthPayload.proportion(ratio),
            Tag::Proportion
        ),
        Width::Fixed(width) => {
            union!(
                BoundaryColumnsWidth,
                BoundaryColumnsWidthPayload.fixed(width),
                Tag::Fixed
            )
        }
    }
}

fn from_width(width: &abi::BoundaryColumnsWidth) -> Width {
    use abi::BoundaryColumnsWidthTag as Tag;
    match width.tag {
        Tag::Default => Width::Default,
        // SAFETY (each arm): the tag says which payload it is.
        Tag::Proportion => {
            Width::Proportion(unsafe { *width.borrow_payload_proportion_unchecked() })
        }
        Tag::Fixed => Width::Fixed(unsafe { *width.borrow_payload_fixed_unchecked() }),
    }
}

fn from_fill(fill: abi::BoundaryFill) -> Fill {
    match fill {
        abi::BoundaryFill::Tiled => Fill::Tiled,
        abi::BoundaryFill::Screen => Fill::Screen,
        abi::BoundaryFill::WithinGaps => Fill::WithinGaps,
    }
}

fn row(row: &Row<u64>) -> abi::BoundaryColumnsRow {
    abi::BoundaryColumnsRow {
        window: row.window,
        height: row.height,
        fill: traditional_fill(row.fill),
    }
}

fn from_row(row: &abi::BoundaryColumnsRow) -> Row<u64> {
    Row {
        window: row.window,
        height: row.height,
        fill: from_fill(row.fill),
    }
}

fn column(column: &Column<u64>, host: &abi::RocHost) -> abi::BoundaryColumnsColumn {
    let before: Vec<abi::BoundaryColumnsRow> = column.before.iter().map(row).collect();
    let after: Vec<abi::BoundaryColumnsRow> = column.after.iter().map(row).collect();
    abi::BoundaryColumnsColumn {
        id: column.id.0,
        width: width(column.width),
        before: list(&before, host),
        active: row(&column.active),
        after: list(&after, host),
    }
}

fn from_column(column: &abi::BoundaryColumnsColumn) -> Column<u64> {
    Column {
        id: ColumnId(column.id),
        width: from_width(&column.width),
        before: column.before.as_slice().iter().map(from_row).collect(),
        active: from_row(&column.active),
        after: column.after.as_slice().iter().map(from_row).collect(),
    }
}

fn arrangement(
    arrangement: &Arrangement<u64>,
    host: &abi::RocHost,
) -> abi::BoundaryColumnsArrangement {
    use abi::BoundaryColumnsStripTag as Tag;
    let strip = match &arrangement.strip {
        Strip::Empty => union!(
            BoundaryColumnsStrip,
            BoundaryColumnsStripPayload.empty,
            Tag::Empty
        ),
        Strip::Columns { before, active, after } => {
            let before: Vec<abi::BoundaryColumnsColumn> =
                before.iter().map(|c| column(c, host)).collect();
            let after: Vec<abi::BoundaryColumnsColumn> =
                after.iter().map(|c| column(c, host)).collect();
            union!(
                BoundaryColumnsStrip,
                BoundaryColumnsStripPayload.columns(abi::BoundaryColumnsAround {
                    before: list(&before, host),
                    active: column(active, host),
                    after: list(&after, host),
                }),
                Tag::Columns
            )
        }
    };
    abi::BoundaryColumnsArrangement {
        strip,
        next_column: arrangement.next_column,
    }
}

fn from_arrangement(arrangement: &abi::BoundaryColumnsArrangement) -> Arrangement<u64> {
    use abi::BoundaryColumnsStripTag as Tag;
    let strip = match arrangement.strip.tag {
        Tag::Empty => Strip::Empty,
        Tag::Columns => {
            // SAFETY: the tag says the payload is the columns around the active one.
            let around = unsafe { arrangement.strip.borrow_payload_columns_unchecked() };
            Strip::Columns {
                before: around.before.as_slice().iter().map(from_column).collect(),
                active: from_column(&around.active),
                after: around.after.as_slice().iter().map(from_column).collect(),
            }
        }
    };
    Arrangement {
        strip,
        next_column: arrangement.next_column,
    }
}

fn settings(settings: &Settings, host: &abi::RocHost) -> abi::BoundaryColumnsSettings {
    abi::BoundaryColumnsSettings {
        insertion: insertion(settings.insertion),
        column_width_ratio: settings.column_width_ratio,
        min_column_width_ratio: settings.min_column_width_ratio,
        max_column_width_ratio: settings.max_column_width_ratio,
        preset_column_widths: list(&settings.preset_column_widths, host),
        preserve_window_sizes: settings.preserve_window_sizes,
    }
}

fn from_settings(settings: &abi::BoundaryColumnsSettings) -> Settings {
    Settings {
        insertion: from_insertion(settings.insertion),
        column_width_ratio: settings.column_width_ratio,
        min_column_width_ratio: settings.min_column_width_ratio,
        max_column_width_ratio: settings.max_column_width_ratio,
        preset_column_widths: settings.preset_column_widths.as_slice().to_vec(),
        preserve_window_sizes: settings.preserve_window_sizes,
    }
}

/// A system as the platform holds it, for `roc_columns_seed`.
fn seed(view: &ColumnsView, host: &abi::RocHost) -> abi::BoundaryColumnsSeed {
    let layouts: Vec<abi::BoundaryColumnsLayoutSlot> = view
        .layouts
        .iter()
        .map(|(held, version)| match held {
            Some(held) => union!(
                BoundaryColumnsLayoutSlot,
                BoundaryColumnsLayoutSlotPayload.occupied(abi::BoundaryColumnsLayoutHeld {
                    version: *version,
                    arrangement: arrangement(held, host),
                }),
                abi::BoundaryColumnsLayoutSlotTag::Occupied
            ),
            None => union!(
                BoundaryColumnsLayoutSlot,
                BoundaryColumnsLayoutSlotPayload.vacant(*version),
                abi::BoundaryColumnsLayoutSlotTag::Vacant
            ),
        })
        .collect();
    abi::BoundaryColumnsSeed {
        layouts: list(&layouts, host),
        layout_free: list(&view.layout_free, host),
        left_out: list(&view.left_out, host),
        settings: settings(&view.settings, host),
    }
}

fn span(span: Option<Span>) -> abi::BoundaryColumnsMaybeSpan {
    use abi::BoundaryColumnsMaybeSpanTag as Tag;
    match span {
        Some(s) => union!(
            BoundaryColumnsMaybeSpan,
            BoundaryColumnsMaybeSpanPayload.span(abi::BoundaryColumnsSpan {
                view: s.view,
                gap: s.gap,
                width: s.width,
            }),
            Tag::Span
        ),
        None => union!(
            BoundaryColumnsMaybeSpan,
            BoundaryColumnsMaybeSpanPayload.no_span,
            Tag::NoSpan
        ),
    }
}

fn window_in(layout: LayoutKey, window: u64) -> abi::BoundaryBspWindowIn {
    abi::BoundaryBspWindowIn { layout: layout.0, window }
}

fn layout_direction(
    layout: LayoutKey,
    direction: rift_core::layout::bsp_tree::Direction,
) -> abi::BoundaryBspLayoutDirection {
    abi::BoundaryBspLayoutDirection {
        layout: layout.0,
        direction: tree_direction(direction),
    }
}

fn event(event: Event, host: &abi::RocHost) -> abi::BoundaryColumnsEvent {
    use abi::BoundaryColumnsEventTag as Tag;
    match event {
        Event::CreateLayout => {
            union!(
                BoundaryColumnsEvent,
                BoundaryColumnsEventPayload.create_layout,
                Tag::CreateLayout
            )
        }
        Event::CloneLayout(layout) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.clone_layout(layout.0),
            Tag::CloneLayout
        ),
        Event::RemoveLayout(layout) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.remove_layout(layout.0),
            Tag::RemoveLayout
        ),
        Event::Participation(window, participation) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.participation(abi::BoundaryBspParticipation {
                window,
                participation: match participation {
                    Participation::TakesPart => abi::BoundaryBspTakesPart::TakesPart,
                    Participation::LeftOut => abi::BoundaryBspTakesPart::LeftOut,
                },
            }),
            Tag::Participation
        ),
        Event::Settings(asked) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.settings(settings(&asked, host)),
            Tag::Settings
        ),
        Event::Insert(layout, window) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.insert(window_in(layout, window)),
            Tag::Insert
        ),
        Event::ForgetWindow(window) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.forget_window(window),
            Tag::ForgetWindow
        ),
        Event::RemoveFrom(layout, window) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.remove_from(window_in(layout, window)),
            Tag::RemoveFrom
        ),
        Event::ReplaceWindow { from, to } => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.replace_window(abi::BoundaryBspReplacement { from, to }),
            Tag::ReplaceWindow
        ),
        Event::ForgetApp(windows) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.forget_app(list(&windows, host)),
            Tag::ForgetApp
        ),
        Event::ResizeFromFrame(
            layout,
            window,
            FrameResize {
                old,
                new,
                tiling,
                horizontal_gap,
                vertical_gap,
            },
        ) => {
            union!(
                BoundaryColumnsEvent,
                BoundaryColumnsEventPayload.resize_from_frame(abi::BoundaryColumnsFrameResize {
                    layout: layout.0,
                    window,
                    old: rect(old),
                    new: rect(new),
                    tiling: rect(tiling),
                    horizontal_gap,
                    vertical_gap,
                }),
                Tag::ResizeFromFrame
            )
        }
        Event::ActivateColumn(layout, id) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload
                .activate_column(abi::BoundaryColumnsActivate { layout: layout.0, column: id.0 }),
            Tag::ActivateColumn
        ),
        Event::PreserveWindowSizes { layout, view, gap, locked } => {
            let locked: Vec<abi::BoundaryColumnsLocked> = locked
                .iter()
                .map(|&(window, width)| abi::BoundaryColumnsLocked { window, width })
                .collect();
            union!(
                BoundaryColumnsEvent,
                BoundaryColumnsEventPayload.preserve_window_sizes(abi::BoundaryColumnsPreserve {
                    layout: layout.0,
                    view,
                    gap,
                    locked: list(&locked, host),
                }),
                Tag::PreserveWindowSizes
            )
        }
        Event::ContainsLayout(layout) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.contains_layout(layout.0),
            Tag::ContainsLayout
        ),
        Event::WindowParticipates(window) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.window_participates(window),
            Tag::WindowParticipates
        ),
        Event::SelectedWindow(layout) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.selected_window(layout.0),
            Tag::SelectedWindow
        ),
        Event::SelectedLocation(layout) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.selected_location(layout.0),
            Tag::SelectedLocation
        ),
        Event::WindowInDirection(layout, direction) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.window_in_direction(layout_direction(layout, direction)),
            Tag::WindowInDirection
        ),
        Event::ContainsWindow(layout, window) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.contains_window(window_in(layout, window)),
            Tag::ContainsWindow
        ),
        Event::HasAnyFullscreenNode(layout) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.has_any_fullscreen_node(layout.0),
            Tag::HasAnyFullscreenNode
        ),
        Event::ParentOfSelectionIsStacked(layout) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.parent_of_selection_is_stacked(layout.0),
            Tag::ParentOfSelectionIsStacked
        ),
        Event::HasWindowsForApp(layout, windows) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.has_windows_for_app(abi::BoundaryBspAppIn {
                layout: layout.0,
                windows: list(&windows, host),
            }),
            Tag::HasWindowsForApp
        ),
        Event::Windows(layout, query) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.windows(abi::BoundaryBspWindowsQuery {
                layout: layout.0,
                query: tree_query(query),
            }),
            Tag::Windows
        ),
        Event::WindowSlot(layout, window) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.window_slot(window_in(layout, window)),
            Tag::WindowSlot
        ),
        Event::CanMoveWindow(layout, window, direction) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.can_move_window(abi::BoundaryBspMoveQuery {
                layout: layout.0,
                window,
                direction: tree_direction(direction),
            }),
            Tag::CanMoveWindow
        ),
        Event::Focus(layout, focus) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.focus(abi::BoundaryBspFocusIn {
                layout: layout.0,
                focus: tree_focus(focus),
            }),
            Tag::Focus
        ),
        Event::Swap(layout, a, b) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.swap(abi::BoundaryColumnsSwap { layout: layout.0, a, b }),
            Tag::Swap
        ),
        Event::MoveSelection(layout, direction) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.move_selection(layout_direction(layout, direction)),
            Tag::MoveSelection
        ),
        Event::Drop { layout, source, target, action } => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.drop(abi::BoundaryColumnsDrop {
                layout: layout.0,
                source,
                target,
                action: drop_action(action),
            }),
            Tag::Drop
        ),
        Event::Join(layout, direction) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.join(layout_direction(layout, direction)),
            Tag::Join
        ),
        Event::ConsumeOrExpel(layout, direction) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.consume_or_expel(layout_direction(layout, direction)),
            Tag::ConsumeOrExpel
        ),
        Event::Stack(layout) => {
            union!(
                BoundaryColumnsEvent,
                BoundaryColumnsEventPayload.stack(layout.0),
                Tag::Stack
            )
        }
        Event::Unstack(layout) => {
            union!(
                BoundaryColumnsEvent,
                BoundaryColumnsEventPayload.unstack(layout.0),
                Tag::Unstack
            )
        }
        Event::Unjoin(layout) => {
            union!(
                BoundaryColumnsEvent,
                BoundaryColumnsEventPayload.unjoin(layout.0),
                Tag::Unjoin
            )
        }
        Event::Resize {
            layout,
            amount,
            along,
            span: laid,
        } => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.resize(abi::BoundaryColumnsResize {
                layout: layout.0,
                amount,
                along: match along {
                    ResizeAlong::Horizontal => abi::BoundaryBspResizeAlong::Horizontal,
                    ResizeAlong::Vertical => abi::BoundaryBspResizeAlong::Vertical,
                    ResizeAlong::Smart => abi::BoundaryBspResizeAlong::Smart,
                },
                span: span(laid),
            }),
            Tag::Resize
        ),
        Event::SwitchPreset { layout, backwards, span: laid } => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.switch_preset(abi::BoundaryColumnsPreset {
                layout: layout.0,
                backwards,
                span: span(laid),
            }),
            Tag::SwitchPreset
        ),
        Event::ToggleFullscreen(layout, within_gaps) => union!(
            BoundaryColumnsEvent,
            BoundaryColumnsEventPayload.toggle_fullscreen(abi::BoundaryColumnsFullscreen {
                layout: layout.0,
                within_gaps,
            }),
            Tag::ToggleFullscreen
        ),
    }
}

fn drop_action(action: DropAction) -> abi::BoundaryColumnsDropAction {
    use abi::BoundaryColumnsDropActionTag as Tag;
    match action {
        DropAction::Swap => union!(
            BoundaryColumnsDropAction,
            BoundaryColumnsDropActionPayload.swap,
            Tag::Swap
        ),
        DropAction::Move(direction) => union!(
            BoundaryColumnsDropAction,
            BoundaryColumnsDropActionPayload.r#move(tree_direction(direction)),
            Tag::Move
        ),
        DropAction::Transfer(transfer) => union!(
            BoundaryColumnsDropAction,
            BoundaryColumnsDropActionPayload.transfer(match transfer {
                Transfer::Stack => union!(
                    BoundaryColumnsTransfer,
                    BoundaryColumnsTransferPayload.stack,
                    abi::BoundaryColumnsTransferTag::Stack
                ),
                Transfer::Insert(direction) => union!(
                    BoundaryColumnsTransfer,
                    BoundaryColumnsTransferPayload.insert(tree_direction(direction)),
                    abi::BoundaryColumnsTransferTag::Insert
                ),
            }),
            Tag::Transfer
        ),
    }
}

fn from_removal(removal: &abi::BoundaryColumnsRemoval) -> Removal {
    Removal {
        active_removed: removal.active_removed,
        column_removed: removal.column_removed,
        was_fullscreen: removal.was_fullscreen,
    }
}

fn from_answer(answer: &abi::BoundaryColumnsAnswer) -> Answer {
    use abi::BoundaryColumnsAnswerTag as Tag;
    match answer.tag {
        // SAFETY (each arm): the tag says which payload it is; lists are only read.
        Tag::Layout => {
            Answer::Layout(LayoutKey(unsafe { *answer.borrow_payload_layout_unchecked() }))
        }
        Tag::Done => Answer::Done,
        Tag::Verdict => Answer::Verdict(unsafe { *answer.borrow_payload_verdict_unchecked() }),
        Tag::Window => Answer::Window(from_maybe_word(unsafe {
            *answer.borrow_payload_window_unchecked()
        })),
        Tag::Location => {
            let location = unsafe { answer.borrow_payload_location_unchecked() };
            Answer::Location(match location.tag {
                abi::BoundaryColumnsMaybeLocationTag::NoLocation => None,
                abi::BoundaryColumnsMaybeLocationTag::Location => {
                    let at = unsafe { location.borrow_payload_location_unchecked() };
                    Some((at.column as usize, at.row as usize))
                }
            })
        }
        Tag::Windows => Answer::Windows(
            unsafe { answer.borrow_payload_windows_unchecked() }.as_slice().to_vec(),
        ),
        Tag::Slot => {
            let slot = unsafe { answer.borrow_payload_slot_unchecked() };
            Answer::Slot(match slot.tag {
                abi::BoundaryBspSlotTag::NoSlot => None,
                abi::BoundaryBspSlotTag::Slot => Some(
                    unsafe { slot.borrow_payload_slot_unchecked() }
                        .as_slice()
                        .iter()
                        .map(|&at| at as usize)
                        .collect(),
                ),
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
        Tag::Moved => {
            let moved = unsafe { answer.borrow_payload_moved_unchecked() };
            Answer::Moved(match moved.tag {
                abi::BoundaryColumnsMaybeMovedTag::Stayed => None,
                abi::BoundaryColumnsMaybeMovedTag::Moved => {
                    let to = unsafe { moved.borrow_payload_moved_unchecked() };
                    Some(match to.tag {
                        abi::BoundaryColumnsMovedTag::NewColumn => {
                            Moved::NewColumn(
                                unsafe { *to.borrow_payload_new_column_unchecked() } as usize
                            )
                        }
                        abi::BoundaryColumnsMovedTag::Column => {
                            Moved::Column(unsafe { *to.borrow_payload_column_unchecked() } as usize)
                        }
                        abi::BoundaryColumnsMovedTag::Row => {
                            Moved::Row(unsafe { *to.borrow_payload_row_unchecked() } as usize)
                        }
                    })
                }
            })
        }
        Tag::Layouts => Answer::Layouts(
            unsafe { answer.borrow_payload_layouts_unchecked() }
                .as_slice()
                .iter()
                .map(|&layout| LayoutKey(layout))
                .collect(),
        ),
        Tag::Closing => Answer::Closing(
            unsafe { answer.borrow_payload_closing_unchecked() }
                .as_slice()
                .iter()
                .map(|c| (LayoutKey(c.layout), c.windows.as_slice().to_vec()))
                .collect(),
        ),
        Tag::Removal => {
            let removal = unsafe { answer.borrow_payload_removal_unchecked() };
            Answer::Removal(match removal.tag {
                abi::BoundaryColumnsMaybeRemovalTag::NotHeld => None,
                abi::BoundaryColumnsMaybeRemovalTag::Removed => Some(from_removal(unsafe {
                    removal.borrow_payload_removed_unchecked()
                })),
            })
        }
        Tag::Fullscreened => {
            let toggled = unsafe { answer.borrow_payload_fullscreened_unchecked() };
            Answer::Fullscreened(match toggled.tag {
                abi::BoundaryColumnsFullscreenedTag::Nothing => Fullscreened::Nothing,
                abi::BoundaryColumnsFullscreenedTag::Entered => {
                    Fullscreened::Entered(unsafe { *toggled.borrow_payload_entered_unchecked() })
                }
                abi::BoundaryColumnsFullscreenedTag::Left => {
                    Fullscreened::Left(unsafe { *toggled.borrow_payload_left_unchecked() })
                }
            })
        }
        Tag::Fixed => Answer::Fixed(
            unsafe { answer.borrow_payload_fixed_unchecked() }
                .as_slice()
                .iter()
                .map(|&id| ColumnId(id))
                .collect(),
        ),
    }
}

/// The arrangements' box, which only Roc can read or free; only the `roc_columns_*` entries
/// make and take one. Dropping it releases the arrangements.
pub(crate) struct ColumnsBox(abi::RocBox);

impl ColumnsBox {
    pub(crate) fn new(asked: &Settings, host: &abi::RocHost) -> ColumnsBox {
        // SAFETY: the settings' list is allocated through the host and consumed by Roc.
        ColumnsBox(unsafe { abi::roc_columns_init(settings(asked, host)) })
    }

    /// The system `view` holds, which the caller has parsed (`ColumnsView::columns`).
    pub(crate) fn seeded(view: &ColumnsView, host: &abi::RocHost) -> ColumnsBox {
        // SAFETY: the seed's lists are allocated through the host and consumed by Roc.
        ColumnsBox(unsafe { abi::roc_columns_seed(seed(view, host)) })
    }

    fn into_raw(self) -> abi::RocBox {
        let raw = self.0;
        std::mem::forget(self);
        raw
    }

    /// One operation: the arrangements after it, and what it answered.
    pub(crate) fn step(self, asked: Event, host: &abi::RocHost) -> (ColumnsBox, Answer) {
        // SAFETY: the box is these arrangements', given to the step; its successor comes back.
        // The answer is read, then released.
        let stepped = unsafe { abi::roc_columns_step(self.into_raw(), event(asked, host)) };
        let answer = from_answer(&stepped.answer);
        unsafe { stepped.answer.decref(host) };
        (ColumnsBox(stepped.model), answer)
    }

    /// The table's slots and free list, the windows left out, the settings.
    pub(crate) fn view(self, host: &abi::RocHost) -> (ColumnsBox, ColumnsView) {
        // SAFETY: as in `step`; the lists are Roc's answer, owned here, read in full, released.
        let viewed = unsafe { abi::roc_columns_view(self.into_raw()) };
        let seed = &viewed.view;
        let layouts = seed
            .layouts
            .as_slice()
            .iter()
            .map(|slot| match slot.tag {
                abi::BoundaryColumnsLayoutSlotTag::Occupied => {
                    // SAFETY: the tag says the payload is the held arrangement.
                    let held = unsafe { slot.borrow_payload_occupied_unchecked() };
                    (Some(from_arrangement(&held.arrangement)), held.version)
                }
                // SAFETY: the tag says the payload is the vacant version.
                abi::BoundaryColumnsLayoutSlotTag::Vacant => {
                    (None, unsafe { *slot.borrow_payload_vacant_unchecked() })
                }
            })
            .collect();
        let view = ColumnsView {
            layouts,
            layout_free: seed.layout_free.as_slice().to_vec(),
            left_out: seed.left_out.as_slice().to_vec(),
            settings: from_settings(&seed.settings),
        };
        unsafe {
            abi::decref_list_of_boundary_columns_layout_slot(seed.layouts, host);
            seed.layout_free.decref(host);
            seed.left_out.decref(host);
            seed.settings.preset_column_widths.decref(host);
        }
        (ColumnsBox(viewed.model), view)
    }
}

impl Drop for ColumnsBox {
    fn drop(&mut self) {
        // SAFETY: the box is these arrangements' and is never used again.
        unsafe { abi::roc_columns_release(self.0) }
    }
}
