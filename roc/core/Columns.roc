## A scrolling layout's arrangements (`rift_core::layout::columns::ColumnSystem`), ported
## operation by operation: every layout's strip of columns in one slot table (keys as slotmap
## allocates them), the windows left out, the settings. A strip is empty or has an active
## column; a column is never empty and has an active row. The operations run on an editing form
## with rift's indices (`Editing`), parsed back after each with the active indices clamped.
## Keys and windows are one word each.

import Geometry exposing [clamp, max]
import Slots
import pf.Boundary exposing [BspTakesPart, BspDirection, BspEdited, BspFocusIn, BspFocused, BspMoveQuery, BspResizeAlong, BspSlot, BspWindowsQuery, ColumnsAnswer, ColumnsArrangement, ColumnsClosing, ColumnsColumn, ColumnsEvent, ColumnsFrameResize, ColumnsFullscreened, ColumnsLayoutSlot, ColumnsMaybeLocation, ColumnsMaybeMoved, ColumnsMaybeRemoval, ColumnsMaybeSpan, ColumnsMoved, ColumnsPreserve, ColumnsRemoval, ColumnsRow, ColumnsSeed, ColumnsSettings, ColumnsStrip, ColumnsTransfer, ColumnsWidth, Fill, MaybeWord]

## A scrolling layout system's arrangements, the windows left out, the settings.
Cols : {
	layouts : Slots(ColumnsArrangement),
	left_out : Set(U64),
	settings : ColumnsSettings,
}

## A column as an edit holds it: its rows in order and its active row's index.
EditColumn : { id : U64, rows : List(ColumnsRow), active : U64, width : ColumnsWidth }

## An arrangement as an edit holds it: its columns, the active one's index, the next id.
Editing : { columns : List(EditColumn), active : U64, next : U64 }

## The system as a view gives it.
View : { layouts : List(ColumnsLayoutSlot), layout_free : List(U32), left_out : List(U64), settings : ColumnsSettings }

Columns := [].{

	empty : ColumnsSettings -> Cols
	empty = |settings| { layouts: Slots.empty({}), left_out: Set.empty(), settings }

	## A system as the platform holds it, which the host has parsed.
	seeded : ColumnsSeed -> Cols
	seeded = |seed| {
		layouts: Slots.from_parts(
			seed.layouts.map(
				|slot|
					match slot {
						Occupied({ version, arrangement }) => Occupied({ version, value: arrangement })
						Vacant(version) => Vacant(version)
					},
			),
			seed.layout_free,
		),
		left_out: Set.from_list(seed.left_out),
		settings: seed.settings,
	}

	view : Cols -> View
	view = |cols| {
		layouts: Slots.slot_list(cols.layouts).map(
			|slot|
				match slot {
					Occupied({ version, value }) => Occupied({ version, arrangement: value })
					Vacant(version) => Vacant(version)
				},
		),
		layout_free: Slots.free_indices(cols.layouts),
		left_out: cols.left_out.to_list().sort_with(|a, b| a.order_relative_to(b)),
		settings: cols.settings,
	}

	## One operation, and what it answers.
	step : Cols, ColumnsEvent -> (Cols, ColumnsAnswer)
	step = |cols, event|
		match event {
			CreateLayout => {
				(layouts, layout) = Slots.insert(cols.layouts, empty_arrangement)
				({ ..cols, layouts }, Layout(layout))
			}
			CloneLayout(layout) => {
				copy =
					match Slots.get(cols.layouts, layout) {
						Held(held) => held
						NotHeld => empty_arrangement
					}
				(layouts, clone) = Slots.insert(cols.layouts, copy)
				({ ..cols, layouts }, Layout(clone))
			}
			RemoveLayout(layout) => {
				(layouts, _) = Slots.remove(cols.layouts, layout)
				({ ..cols, layouts }, Done)
			}
			Participation({ window, participation }) => {
				(after, changed) = set_window_participation(cols, window, participation)
				(after, Verdict(changed))
			}
			Settings(settings) => ({ ..cols, settings }, Done)
			Insert({ layout, window }) => {
				(after, inserted) = insert(cols, layout, window)
				(after, Verdict(inserted))
			}
			ForgetWindow(window) => {
				(after, layouts) = forget_window(cols, window)
				(after, Layouts(layouts))
			}
			RemoveFrom({ layout, window }) => {
				(after, removal) = edited_layout(cols, layout, NotHeld, |e| removed_from(e, cols.left_out, window))
				(after, Removal(removal))
			}
			ReplaceWindow({ from, to }) => (replace_window(cols, from, to), Done)
			ForgetApp(windows) => {
				(after, closing) = forget_app(cols, windows)
				(after, Closing(closing))
			}
			ResizeFromFrame(resize) => {
				(after, changed) = resize_from_frame(cols, resize)
				(after, Verdict(changed))
			}
			ActivateColumn({ layout, column }) => {
				(after, held) = edited_layout(cols, layout, False, |e| activate_column(e, column))
				(after, Verdict(held))
			}
			PreserveWindowSizes(asked) => {
				(after, fixed) = preserve_window_sizes(cols, asked)
				(after, Fixed(fixed))
			}
			ContainsLayout(layout) =>
				match Slots.get(cols.layouts, layout) {
					Held(_) => (cols, Verdict(True))
					NotHeld => (cols, Verdict(False))
				}
			WindowParticipates(window) => (cols, Verdict(takes_part(cols, window)))
			SelectedWindow(layout) => (cols, Window(selected_window(cols, layout)))
			SelectedLocation(layout) => (cols, Location(selected_location_of(cols, layout)))
			WindowInDirection({ layout, direction }) => (cols, Window(window_in_direction(cols, layout, direction)))
			ContainsWindow({ layout, window }) => (cols, Verdict(contains_window(cols, layout, window)))
			HasAnyFullscreenNode(layout) => (cols, Verdict(has_any_fullscreen_node(cols, layout)))
			ParentOfSelectionIsStacked(layout) => (cols, Verdict(parent_of_selection_is_stacked(cols, layout)))
			HasWindowsForApp({ layout, windows }) => (cols, Verdict(windows_of(cols, layout).any(|w| windows.contains(w))))
			Windows(asked) => (cols, Windows(listed_windows(cols, asked)))
			WindowSlot({ layout, window }) => (cols, Slot(window_slot(cols, layout, window)))
			CanMoveWindow(asked) => (cols, Verdict(can_move_window(cols, asked)))
			Focus(asked) => {
				(after, focused) = focus(cols, asked)
				(after, Focused(focused))
			}
			Swap({ layout, a, b }) => {
				(after, swapped) = swap_windows(cols, layout, a, b)
				(after, Verdict(swapped))
			}
			MoveSelection({ layout, direction }) => {
				(after, moved) = move_selection(cols, layout, direction)
				(after, Moved(moved))
			}
			Drop({ layout, source, target, action }) => {
				(after, dropped) =
					if !takes_part(cols, source) or !takes_part(cols, target) {
						(cols, False)
					} else {
						match action {
							Move(direction) => {
								(chosen_cols, chose) = select_window(cols, layout, source)
								if chose {
									(moved_cols, moved) = move_selection(chosen_cols, layout, direction)
									match moved {
										Moved(_) => (moved_cols, True)
										Stayed => (moved_cols, False)
									}
								} else {
									(chosen_cols, False)
								}
							}
							Swap => swap_windows(cols, layout, source, target)
							Transfer(transfer) => edited_layout(cols, layout, False, |e| transfer_window(e, cols.left_out, source, target, transfer))
						}
					}
				(after, Verdict(dropped))
			}
			Join({ layout, direction }) => {
				(after, joined) = join_selection(cols, layout, direction)
				(after, Verdict(joined))
			}
			ConsumeOrExpel({ layout, direction }) => {
				(after, edited) = consume_or_expel(cols, layout, direction)
				(after, Edited(edited))
			}
			Stack(layout) => {
				(after, moved) = stack(cols, layout)
				(after, Windows(moved))
			}
			Unstack(layout) => {
				(after, moved) = unstack(cols, layout)
				(after, Windows(moved))
			}
			Unjoin(layout) =>
				if parent_of_selection_is_stacked(cols, layout) {
					(after, moved) = move_selection(cols, layout, Right)
					match moved {
						Moved(_) => (after, Verdict(True))
						Stayed => (after, Verdict(False))
					}
				} else {
					(cols, Verdict(False))
				}
			Resize({ layout, amount, along, span }) => {
				(after, edited) = resize_selection(cols, layout, amount, along, span)
				(after, Edited(edited))
			}
			SwitchPreset({ layout, backwards, span }) => {
				(after, changed) = switch_preset(cols, layout, backwards, span)
				(after, Verdict(changed))
			}
			ToggleFullscreen({ layout, within_gaps }) => {
				(after, toggled) = toggle_fullscreen(cols, layout, within_gaps)
				(after, Fullscreened(toggled))
			}
		}

	empty_arrangement : ColumnsArrangement
	empty_arrangement = { strip: Empty, next_column: 0 }

	takes_part : Cols, U64 -> Bool
	takes_part = |cols, window| !cols.left_out.contains(window)

	# The editing form.

	## A column's rows in order, and its active row's index.
	edit_column : ColumnsColumn -> EditColumn
	edit_column = |column| {
		id: column.id,
		rows: column.before.append(column.active).concat(column.after),
		active: column.before.len(),
		width: column.width,
	}

	editing : ColumnsArrangement -> Editing
	editing = |arrangement|
		match arrangement.strip {
			Empty => { columns: [], active: 0, next: arrangement.next_column }
			Columns({ before, active, after }) => {
				columns: before.append(active).concat(after).map(edit_column),
				active: before.len(),
				next: arrangement.next_column,
			}
		}

	## The column an edit leaves: none for no rows, the active row clamped to the last.
	column_of : EditColumn -> [Some(ColumnsColumn), None]
	column_of = |edit| {
		count = edit.rows.len()
		if count == 0 {
			None
		} else {
			at = if edit.active < count edit.active else count - 1
			match edit.rows.get(at) {
				Ok(active) => Some({ id: edit.id, width: edit.width, before: edit.rows.take_first(at), active, after: edit.rows.drop_first(at + 1) })
				Err(_) => None
			}
		}
	}

	## The arrangement an edit leaves: columns with no rows dropped, the active index clamped.
	arrangement_of : Editing -> ColumnsArrangement
	arrangement_of = |edit| {
		columns = edit.columns.fold(
			[],
			|acc, column|
				match column_of(column) {
					Some(made) => acc.append(made)
					None => acc
				},
		)
		count = columns.len()
		strip : ColumnsStrip
		strip =
			if count == 0 {
				Empty
			} else {
				at = if edit.active < count edit.active else count - 1
				match columns.get(at) {
					Ok(active) => Columns({ before: columns.take_first(at), active, after: columns.drop_first(at + 1) })
					Err(_) => Empty
				}
			}
		{ strip, next_column: edit.next }
	}

	## `layout`'s arrangement edited, the edit's answer beside; `missing` for a layout the
	## system does not hold.
	edited_layout : Cols, U64, a, (Editing -> (Editing, a)) -> (Cols, a)
	edited_layout = |cols, layout, missing, edit|
		match Slots.get(cols.layouts, layout) {
			NotHeld => (cols, missing)
			Held(arrangement) => {
				(after, answer) = edit(editing(arrangement))
				match Slots.replace(cols.layouts, layout, arrangement_of(after)) {
					Held(layouts) => ({ ..cols, layouts }, answer)
					NotHeld => (cols, missing)
				}
			}
		}

	## `layout`'s arrangement as an edit holds it, when the system holds it.
	editing_of : Cols, U64 -> [Some(Editing), None]
	editing_of = |cols, layout|
		match Slots.get(cols.layouts, layout) {
			Held(arrangement) => Some(editing(arrangement))
			NotHeld => None
		}

	# Reading an edit.

	row_window : EditColumn, U64 -> MaybeWord
	row_window = |column, row|
		match column.rows.get(row) {
			Ok(found) => Some(found.window)
			Err(_) => Nothing
		}

	column_at : Editing, U64 -> [Some(EditColumn), None]
	column_at = |e, col|
		match e.columns.get(col) {
			Ok(column) => Some(column)
			Err(_) => None
		}

	## The first row of `column` taking part.
	first_taking_part : EditColumn, Set(U64) -> [Some(U64), None]
	first_taking_part = |column, left_out|
		match column.rows.find_first_index(|r| !left_out.contains(r.window)) {
			Ok(row) => Some(row)
			Err(_) => None
		}

	selected_location : Editing, Set(U64) -> [At(U64, U64), Nowhere]
	selected_location = |e, left_out|
		match column_at(e, e.active) {
			None => Nowhere
			Some(column) =>
				match row_window(column, column.active) {
					Some(window) if !left_out.contains(window) => At(e.active, column.active)
					_ =>
						match first_taking_part(column, left_out) {
							Some(row) => At(e.active, row)
							None => Nowhere
						}
				}
		}

	selected : Editing, Set(U64) -> MaybeWord
	selected = |e, left_out|
		match selected_location(e, left_out) {
			At(col, row) =>
				match column_at(e, col) {
					Some(column) => row_window(column, row)
					None => Nothing
				}
			Nowhere => Nothing
		}

	locate : Editing, U64 -> [At(U64, U64), Nowhere]
	locate = |e, window| {
		(_, found) = e.columns.fold_until(
			(0, Nowhere),
			|(col, _), column|
				match column.rows.find_first_index(|r| r.window == window) {
					Ok(row) => Break((col, At(col, row)))
					Err(_) => Continue((col + 1, Nowhere)),
				},
		)
		found
	}

	set_column : Editing, U64, EditColumn -> Editing
	set_column = |e, col, column| { ..e, columns: set_at(e.columns, col, column) }

	## `list` with `value` at `index` (unchanged past its end: every index here was just read
	## from the same list).
	set_at : List(a), U64, a -> List(a)
	set_at = |list, index, value|
		match list.set(index, value) {
			Ok(set) => set
			Err(_) => list
		}

	activate : Editing, U64 -> (Editing, Bool)
	activate = |e, window|
		match locate(e, window) {
			At(col, row) =>
				match column_at(e, col) {
					Some(column) => (set_column({ ..e, active: col }, col, { ..column, active: row }), True)
					None => (e, False)
				}
			Nowhere => (e, False)
		}

	activate_column : Editing, U64 -> (Editing, Bool)
	activate_column = |e, id|
		match e.columns.find_first_index(|c| c.id == id) {
			Ok(index) => ({ ..e, active: index }, True)
			Err(_) => (e, False)
		}

	new_column : Editing, U64, ColumnsRow, ColumnsWidth -> Editing
	new_column = |e, index, row, width| {
		next = max_u64(e.next, 2)
		at = if index < e.columns.len() index else e.columns.len()
		column = { id: next, rows: [row], active: 0, width }
		inserted =
			match e.columns.insert(at, column) {
				Ok(list) => list
				Err(_) => e.columns.append(column)
			}
		(activated, _) = activate({ ..e, columns: inserted, next: next + 2 }, row.window)
		activated
	}

	max_u64 : U64, U64 -> U64
	max_u64 = |a, b| if a > b a else b

	## `window` taken out, its column's width and its row kept (rift's `detach`).
	detach : Editing, Set(U64), U64 -> (Editing, [Detached(ColumnsWidth, ColumnsRow), NotThere])
	detach = |e, left_out, window|
		match locate(e, window) {
			Nowhere => (e, NotThere)
			At(col, row) =>
				match column_at(e, col) {
					None => (e, NotThere)
					Some(column) =>
						match column.rows.get(row) {
							Err(_) => (e, NotThere)
							Ok(taken) => {
								was_selected = selected(e, left_out)
								rows = column.rows.drop_at(row)
								shifted = if row < column.active column.active - 1 else column.active
								last = if rows.is_empty() 0 else rows.len() - 1
								kept = { ..column, rows, active: if shifted < last shifted else last }
								removed =
									if rows.is_empty() {
										columns = e.columns.drop_at(col)
										active = if col < e.active e.active - 1 else e.active
										{ ..e, columns, active }
									} else {
										set_column(e, col, kept)
									}
								count = removed.columns.len()
								last_column = if count == 0 0 else count - 1
								clamped = { ..removed, active: if removed.active < last_column removed.active else last_column }
								reselected =
									match was_selected {
										Some(chosen) if chosen != window => {
											(activated, _) = activate(clamped, chosen)
											activated
										}
										_ => clamped
									}
								(reselected, Detached(column.width, taken))
							}
						}
				}
		}

	transfer_window : Editing, Set(U64), U64, U64, ColumnsTransfer -> (Editing, Bool)
	transfer_window = |e, left_out, source, target, transfer|
		if source == target {
			(e, False)
		} else {
			match (locate(e, source), locate(e, target)) {
				(At(_, _), At(_, _)) =>
					match detach(e, left_out, source) {
						(_, NotThere) => (e, False)
						(detached, Detached(width, row)) =>
							match locate(detached, target) {
								Nowhere => (e, False)
								At(col, at) => {
									placed =
										match transfer {
											Insert(Left) => new_column(detached, col, row, width)
											Insert(Right) => new_column(detached, col + 1, row, width)
											_ => {
												after =
													match transfer {
														Insert(Up) => False
														_ => True
													}
												index = if after at + 1 else at
												match column_at(detached, col) {
													Some(column) => {
														active = if index <= column.active column.active + 1 else column.active
														rows =
															match column.rows.insert(index, row) {
																Ok(list) => list
																Err(_) => column.rows.append(row)
															}
														set_column(detached, col, { ..column, rows, active })
													}
													None => detached
												}
											}
										}
									(activated, _) = activate(placed, source)
									(activated, True)
								}
							}
					}
				_ => (e, False)
			}
		}

	## Where the selection at (`col`, `row`) moves in `direction`.
	move_destination : Editing, U64, U64, BspDirection, Set(U64) -> ColumnsMaybeMoved
	move_destination = |e, col, row, direction, left_out|
		match column_at(e, col) {
			None => Stayed
			Some(column) => {
				horizontal =
					match direction {
						Left | Right => True
						Up | Down => False
					}
				if horizontal and column.rows.len() > 1 {
					match direction {
						Right => Moved(NewColumn(col + 1))
						_ => Moved(NewColumn(col))
					}
				} else {
					(index, len) = if horizontal (col, e.columns.len()) else (row, column.rows.len())
					participates = |candidate|
						if horizontal {
							match column_at(e, candidate) {
								Some(other) => other.rows.any(|r| !left_out.contains(r.window))
								None => False
							}
						} else {
							match row_window(column, candidate) {
								Some(window) => !left_out.contains(window)
								Nothing => False
							}
						}
					found =
						match direction {
							Left | Up => search_down(index, participates)
							Right | Down => search_up(index + 1, len, participates)
						}
					match found {
						Some(target) => if horizontal Moved(Column(target)) else Moved(Row(target))
						None => Stayed
					}
				}
			}
		}

	## The nearest index below `index` that `ok` admits.
	search_down : U64, (U64 -> Bool) -> [Some(U64), None]
	search_down = |index, ok|
		if index == 0 {
			None
		} else if ok(index - 1) {
			Some(index - 1)
		} else {
			search_down(index - 1, ok)
		}

	## The first index from `from` below `len` that `ok` admits.
	search_up : U64, U64, (U64 -> Bool) -> [Some(U64), None]
	search_up = |from, len, ok|
		if from >= len {
			None
		} else if ok(from) {
			Some(from)
		} else {
			search_up(from + 1, len, ok)
		}

	## The row at `row` given `share` of `scale`, the others the rest in their proportions.
	set_height_share : EditColumn, U64, F64, F64 -> EditColumn
	set_height_share = |column, row, share, scale| {
		(_, other) = column.rows.fold((0, 0.0), |(i, sum), r| (i + 1, if i == row sum else sum + r.height))
		others = if column.rows.len() > 1 (column.rows.len() - 1).to_f64() else 1.0
		(_, rows) = column.rows.fold(
			(0, []),
			|(i, acc), r| {
				height =
					if i == row {
						share * scale
					} else if other > 0.0 {
						(1.0 - share) * scale * r.height / other
					} else {
						(1.0 - share) * scale / others
					}
				(i + 1, acc.append({ ..r, height }))
			},
		)
		{ ..column, rows }
	}

	# Settings.

	clamp_ratio : ColumnsSettings, F64 -> F64
	clamp_ratio = |settings, ratio| {
		low = max(settings.min_column_width_ratio, 0.05)
		high = max(settings.max_column_width_ratio, low)
		if F64.is_finite(ratio) clamp(ratio, low, high) else low
	}

	proportional_width : F64, F64, F64 -> F64
	proportional_width = |across, gap, ratio| max((across + gap) * ratio - gap, 1.0)

	width_ratio : F64, F64, F64 -> F64
	width_ratio = |across, gap, width| (width + gap) / max(across + gap, 1.0)

	# The system's operations.

	set_window_participation : Cols, U64, BspTakesPart -> (Cols, Bool)
	set_window_participation = |cols, window, participation| {
		(left_out, changed) =
			match participation {
				TakesPart => (cols.left_out.remove(window), cols.left_out.contains(window))
				LeftOut => (cols.left_out.insert(window), !cols.left_out.contains(window))
			}
		if !changed {
			(cols, False)
		} else {
			marked = { ..cols, left_out }
			repaired = layout_keys(marked).fold(
				marked,
				|acc, layout|
					if contains_window(acc, layout, window) {
						(after, _) = edited_layout(acc, layout, {}, |e| (repair_selection(e, left_out), {}))
						after
					} else {
						acc
					},
			)
			(repaired, True)
		}
	}

	## With no selection taking part, the first row taking part made the selection.
	repair_selection : Editing, Set(U64) -> Editing
	repair_selection = |e, left_out|
		match selected(e, left_out) {
			Some(_) => e
			Nothing => {
				(_, found) = e.columns.fold_until(
					(0, None),
					|(col, _), column|
						match first_taking_part(column, left_out) {
							Some(row) => Break((col, Some((col, row))))
							None => Continue((col + 1, None)),
						},
				)
				match found {
					Some((col, row)) =>
						match column_at(e, col) {
							Some(column) => set_column({ ..e, active: col }, col, { ..column, active: row })
							None => e
						}
					None => e
				}
			}
		}

	## Every layout's key, in table order.
	layout_keys : Cols -> List(U64)
	layout_keys = |cols|
		Slots.slot_list(cols.layouts).map_with_index(
			|slot, index|
				match slot {
					Occupied({ version, .. }) => [Slots.key_of(index, version)]
					Vacant(_) => []
				},
		).join_map(|keys| keys)

	insert : Cols, U64, U64 -> (Cols, Bool)
	insert = |cols, layout, window|
		match editing_of(cols, layout) {
			None => (cols, False)
			Some(e) =>
				match locate(e, window) {
					At(_, _) => (cols, False)
					Nowhere =>
						edited_layout(
							cols,
							layout,
							False,
							|edit| {
								index =
									match cols.settings.insertion {
										EndOfTree => edit.columns.len()
										NextToSelection =>
											match selected_location(edit, cols.left_out) {
												At(col, _) => col + 1
												Nowhere => 0
											}
									}
								(new_column(edit, index, { window, height: 1.0, fill: Tiled }, Default), True)
							},
						)
				}
		}

	## What removing `window` from `e` did (for the camera), and the edit after; the camera's
	## answer comes as its own events (`ActivateColumn`), never asked here.
	removed_from : Editing, Set(U64), U64 -> (Editing, ColumnsMaybeRemoval)
	removed_from = |e, left_out, window|
		match locate(e, window) {
			Nowhere => (e, NotHeld)
			At(col, row) =>
				match column_at(e, col) {
					None => (e, NotHeld)
					Some(column) => {
						removal = {
							active_removed: selected(e, left_out) == Some(window),
							column_removed: column.rows.len() == 1,
							was_fullscreen:
								match column.rows.get(row) {
									Ok(r) => !is_tiled(r.fill)
									Err(_) => False
								},
						}
						(after, _) = detach(e, left_out, window)
						(after, Removed(removal))
					}
				}
		}

	## `window` going: no longer left out, and the layouts holding it, in layout order.
	forget_window : Cols, U64 -> (Cols, List(U64))
	forget_window = |cols, window| {
		without = { ..cols, left_out: cols.left_out.remove(window) }
		(without, layout_keys(without).keep_if(|layout| contains_window(without, layout, window)))
	}

	replace_window : Cols, U64, U64 -> Cols
	replace_window = |cols, from, to| {
		left_out = if cols.left_out.contains(from) cols.left_out.remove(from).insert(to) else cols.left_out
		marked = { ..cols, left_out }
		layout_keys(marked).fold(
			marked,
			|acc, layout|
				if contains_window(acc, layout, from) {
					(after, _) = edited_layout(
						acc,
						layout,
						{},
						|e| ({ ..e, columns: e.columns.map(|c| { ..c, rows: c.rows.map(|r| if r.window == from { ..r, window: to } else r) }) }, {}),
					)
					after
				} else {
					acc
				},
		)
	}

	## An app's windows going: none left out any more, and each layout's windows of the app, in
	## layout order and its own (layouts holding none left out).
	forget_app : Cols, List(U64) -> (Cols, List(ColumnsClosing))
	forget_app = |cols, windows| {
		marked = { ..cols, left_out: cols.left_out.drop_if(|w| windows.contains(w)) }
		closing = layout_keys(marked)
			.map(|layout| { layout, windows: windows_of(marked, layout).keep_if(|w| windows.contains(w)) })
			.keep_if(|c| !c.windows.is_empty())
		(marked, closing)
	}

	windows_of : Cols, U64 -> List(U64)
	windows_of = |cols, layout|
		match editing_of(cols, layout) {
			Some(e) => e.columns.join_map(|c| c.rows.map(|r| r.window))
			None => []
		}

	contains_window : Cols, U64, U64 -> Bool
	contains_window = |cols, layout, window|
		match editing_of(cols, layout) {
			Some(e) =>
				match locate(e, window) {
					At(_, _) => True
					Nowhere => False
				}
			None => False
		}

	selected_window : Cols, U64 -> MaybeWord
	selected_window = |cols, layout|
		match editing_of(cols, layout) {
			Some(e) => selected(e, cols.left_out)
			None => Nothing
		}

	selected_location_of : Cols, U64 -> ColumnsMaybeLocation
	selected_location_of = |cols, layout|
		match editing_of(cols, layout) {
			Some(e) =>
				match selected_location(e, cols.left_out) {
					At(column, row) => Location({ column, row })
					Nowhere => NoLocation
				}
			None => NoLocation
		}

	## A column's selection: its active row's window taking part, else its first taking part.
	column_selection : EditColumn, Set(U64) -> MaybeWord
	column_selection = |column, left_out|
		match row_window(column, column.active) {
			Some(window) if !left_out.contains(window) => Some(window)
			_ =>
				match column.rows.find_first(|r| !left_out.contains(r.window)) {
					Ok(r) => Some(r.window)
					Err(_) => Nothing
				}
		}

	window_in_direction : Cols, U64, BspDirection -> MaybeWord
	window_in_direction = |cols, layout, direction|
		match editing_of(cols, layout) {
			None => Nothing
			Some(e) =>
				match selected_location(e, cols.left_out) {
					Nowhere => Nothing
					At(col, row) => {
						taking = |r| if cols.left_out.contains(r.window) Nothing else Some(r.window)
						match direction {
							Left => last_some(e.columns.take_first(col), |c| column_selection(c, cols.left_out))
							Right => first_some(e.columns.drop_first(col + 1), |c| column_selection(c, cols.left_out))
							Up =>
								match column_at(e, col) {
									Some(column) => last_some(column.rows.take_first(row), taking)
									None => Nothing
								}
							Down =>
								match column_at(e, col) {
									Some(column) => first_some(column.rows.drop_first(row + 1), taking)
									None => Nothing
								}
						}
					}
				}
		}

	has_any_fullscreen_node : Cols, U64 -> Bool
	has_any_fullscreen_node = |cols, layout|
		match editing_of(cols, layout) {
			Some(e) => e.columns.any(|c| c.rows.any(|r| !is_tiled(r.fill) and takes_part(cols, r.window)))
			None => False
		}

	parent_of_selection_is_stacked : Cols, U64 -> Bool
	parent_of_selection_is_stacked = |cols, layout|
		match Slots.get(cols.layouts, layout) {
			Held({ strip: Columns({ active, .. }), .. }) => !active.before.is_empty() or !active.after.is_empty()
			_ => False
		}

	visible_under_selection : Cols, U64 -> List(U64)
	visible_under_selection = |cols, layout|
		match Slots.get(cols.layouts, layout) {
			Held({ strip: Columns({ active, .. }), .. }) =>
				active.before.append(active.active).concat(active.after).map(|r| r.window).keep_if(|w| takes_part(cols, w))
			_ => []
		}

	listed_windows : Cols, BspWindowsQuery -> List(U64)
	listed_windows = |cols, { layout, query }|
		match query {
			All => windows_of(cols, layout)
			Visible => windows_of(cols, layout).keep_if(|w| takes_part(cols, w))
			Selection => visible_under_selection(cols, layout)
			Stack(_) => []
		}

	window_slot : Cols, U64, U64 -> BspSlot
	window_slot = |cols, layout, window|
		match editing_of(cols, layout) {
			Some(e) =>
				match locate(e, window) {
					At(col, row) => Slot([col, row])
					Nowhere => NoSlot
				}
			None => NoSlot
		}

	can_move_window : Cols, BspMoveQuery -> Bool
	can_move_window = |cols, { layout, window, direction }|
		if !takes_part(cols, window) {
			False
		} else {
			match editing_of(cols, layout) {
				None => False
				Some(e) =>
					match locate(e, window) {
						Nowhere => False
						At(col, row) =>
							match move_destination(e, col, row, direction, cols.left_out) {
								Moved(_) => True
								Stayed => False
							}
					}
			}
		}

	select_window : Cols, U64, U64 -> (Cols, Bool)
	select_window = |cols, layout, window|
		if !takes_part(cols, window) {
			(cols, False)
		} else {
			match editing_of(cols, layout) {
				None => (cols, False)
				Some(e) =>
					if selected(e, cols.left_out) == Some(window) {
						(cols, True)
					} else {
						edited_layout(cols, layout, False, |edit| activate(edit, window))
					}
			}
		}

	focus : Cols, BspFocusIn -> (Cols, BspFocused)
	focus = |cols, { layout, focus: asked }|
		match asked {
			Select(window) => {
				(after, handled) = select_window(cols, layout, window)
				(after, Selection(handled))
			}
			Ascend => {
				(after, moved, _) = move_focus(cols, layout, Up)
				(after, Selection(moved != Nothing))
			}
			Descend => {
				(after, moved, _) = move_focus(cols, layout, Down)
				(after, Selection(moved != Nothing))
			}
			Direction(direction) => {
				(after, window, raise) = move_focus(cols, layout, direction)
				(after, Movement({ window, raise }))
			}
		}

	move_focus : Cols, U64, BspDirection -> (Cols, MaybeWord, List(U64))
	move_focus = |cols, layout, direction|
		match window_in_direction(cols, layout, direction) {
			Nothing => (cols, Nothing, [])
			Some(window) => {
				(after, _) = select_window(cols, layout, window)
				(after, Some(window), visible_under_selection(after, layout))
			}
		}

	swap_windows : Cols, U64, U64, U64 -> (Cols, Bool)
	swap_windows = |cols, layout, a, b|
		if !takes_part(cols, a) or !takes_part(cols, b) {
			(cols, False)
		} else {
			match editing_of(cols, layout) {
				None => (cols, False)
				Some(e) =>
					match (locate(e, a), locate(e, b)) {
						(At(ac, ar), At(bc, br)) =>
							edited_layout(
								cols,
								layout,
								False,
								|edit| {
									was_selected = selected(edit, cols.left_out)
									first = row_at(edit, ac, ar)
									second = row_at(edit, bc, br)
									swapped = put_row(put_row(edit, ac, ar, second), bc, br, first)
									reselected =
										match was_selected {
											Some(window) => activate(swapped, window).0
											Nothing => swapped
										}
									(reselected, True)
								},
							)
						_ => (cols, False)
					}
			}
		}

	row_at : Editing, U64, U64 -> [Some(ColumnsRow), None]
	row_at = |e, col, row|
		match column_at(e, col) {
			Some(column) =>
				match column.rows.get(row) {
					Ok(found) => Some(found)
					Err(_) => None
				}
			None => None
		}

	put_row : Editing, U64, U64, [Some(ColumnsRow), None] -> Editing
	put_row = |e, col, row, placed|
		match (placed, column_at(e, col)) {
			(Some(r), Some(column)) => set_column(e, col, { ..column, rows: set_at(column.rows, row, r) })
			_ => e
		}

	move_selection : Cols, U64, BspDirection -> (Cols, ColumnsMaybeMoved)
	move_selection = |cols, layout, direction|
		edited_layout(
			cols,
			layout,
			Stayed,
			|e|
				match selected_location(e, cols.left_out) {
					Nowhere => (e, Stayed)
					At(col, row) => {
						destination = move_destination(e, col, row, direction, cols.left_out)
						match destination {
							Stayed => (e, Stayed)
							Moved(NewColumn(target)) =>
								match column_at(e, col) {
									None => (e, Stayed)
									Some(column) =>
										match row_window(column, row) {
											Nothing => (e, Stayed)
											Some(window) =>
												match detach(e, cols.left_out, window) {
													(detached, Detached(width, taken)) => (new_column(detached, target, taken, width), destination)
													(_, NotThere) => (e, Stayed)
												}
										}
								}
							Moved(Column(target)) =>
								match (column_at(e, col), column_at(e, target)) {
									(Some(here), Some(there)) => ({ ..e, columns: set_at(set_at(e.columns, col, there), target, here), active: target }, destination)
									_ => (e, Stayed)
								}
							Moved(Row(target)) =>
								match column_at(e, col) {
									Some(column) =>
										match (column.rows.get(row), column.rows.get(target)) {
											(Ok(here), Ok(there)) => (set_column(e, col, { ..column, rows: set_at(set_at(column.rows, row, there), target, here), active: target }), destination)
											_ => (e, Stayed)
										}
									None => (e, Stayed)
								}
						}
					}
				},
		)

	join_selection : Cols, U64, BspDirection -> (Cols, Bool)
	join_selection = |cols, layout, direction|
		match direction {
			Up | Down => (cols, False)
			Left | Right =>
				match (window_in_direction(cols, layout, direction), selected_window(cols, layout)) {
					(Some(target), Some(source)) =>
						if !takes_part(cols, source) or !takes_part(cols, target) {
							(cols, False)
						} else {
							edited_layout(cols, layout, False, |e| transfer_window(e, cols.left_out, source, target, Stack))
						}
					_ => (cols, False)
				}
		}

	consume_or_expel : Cols, U64, BspDirection -> (Cols, BspEdited)
	consume_or_expel = |cols, layout, direction|
		match direction {
			Up | Down => (cols, Unchanged)
			Left | Right => {
				(after, changed) =
					if parent_of_selection_is_stacked(cols, layout) {
						(moved_cols, moved) = move_selection(cols, layout, direction)
						match moved {
							Moved(_) => (moved_cols, True)
							Stayed => (moved_cols, False)
						}
					} else {
						join_selection(cols, layout, direction)
					}
				(after, if changed Changed else Unchanged)
			}
		}

	stack : Cols, U64 -> (Cols, List(U64))
	stack = |cols, layout|
		edited_layout(
			cols,
			layout,
			[],
			|e|
				match (selected_location(e, cols.left_out), selected(e, cols.left_out)) {
					(At(col, _), Some(chosen)) => {
						count = e.columns.len()
						neighbour =
							if col + 1 < count {
								Some(col + 1)
							} else if col > 0 {
								Some(col - 1)
							} else {
								None
							}
						match neighbour {
							None => (e, [])
							Some(target) =>
								match (column_at(e, target), column_at(e, col)) {
									(Some(taken), Some(_)) => {
										moved = taken.rows.map(|r| r.window)
										without = { ..e, columns: e.columns.drop_at(target) }
										at = if target < col col - 1 else col
										merged =
											match column_at(without, at) {
												Some(column) => set_column(without, at, { ..column, rows: column.rows.concat(taken.rows) })
												None => without
											}
										(activate(merged, chosen).0, moved)
									}
									_ => (e, [])
								}
						}
					}
					_ => (e, [])
				},
		)

	unstack : Cols, U64 -> (Cols, List(U64))
	unstack = |cols, layout|
		edited_layout(
			cols,
			layout,
			[],
			|e|
				match (selected_location(e, cols.left_out), selected(e, cols.left_out)) {
					(At(col, _), Some(chosen)) =>
						match column_at(e, col) {
							None => (e, [])
							Some(column) => {
								moved = column.rows.map(|r| r.window).keep_if(|w| w != chosen)
								(_, spread) = moved.fold(
									(0, e),
									|(index, acc), window|
										match detach(acc, cols.left_out, window) {
											(detached, Detached(width, row)) => (index + 1, new_column(detached, col + 1 + index, row, width))
											(unchanged, NotThere) => (index + 1, unchanged)
										},
								)
								(activate(spread, chosen).0, moved)
							}
						}
					_ => (e, [])
				},
		)

	resize_selection : Cols, U64, F64, BspResizeAlong, ColumnsMaybeSpan -> (Cols, BspEdited)
	resize_selection = |cols, layout, amount, along, span|
		if !F64.is_finite(amount) {
			(cols, Unchanged)
		} else {
			edited_layout(
				cols,
				layout,
				Unchanged,
				|e|
					match selected_location(e, cols.left_out) {
						Nowhere => (e, Unchanged)
						At(col, row) =>
							match column_at(e, col) {
								None => (e, Unchanged)
								Some(column) => {
									vertical =
										match along {
											Vertical => True
											Smart => column.rows.len() > 1
											Horizontal => False
										}
									resized =
										if vertical {
											if column.rows.len() > 1 {
												total = column.rows.fold(0.0, |sum, r| sum + r.height)
												height =
													match column.rows.get(row) {
														Ok(r) => r.height
														Err(_) => 0.0
													}
												share = clamp(height / total + amount, 0.05, 0.95)
												set_height_share(column, row, share, max(total, 10_000.0))
											} else {
												column
											}
										} else {
											ratio =
												match span {
													Span(s) => width_ratio(s.view, s.gap, s.width)
													NoSpan =>
														match column.width {
															Proportion(r) => r
															_ => cols.settings.column_width_ratio
														}
												}
											{ ..column, width: Proportion(clamp_ratio(cols.settings, ratio + amount)) }
										}
									(set_column(e, col, resized), Changed)
								}
							}
					},
			)
		}

	resize_from_frame : Cols, ColumnsFrameResize -> (Cols, Bool)
	resize_from_frame = |cols, resize|
		if !takes_part(cols, resize.window) {
			(cols, False)
		} else {
			match editing_of(cols, resize.layout) {
				None => (cols, False)
				Some(e) =>
					match locate(e, resize.window) {
						Nowhere => (cols, False)
						At(col, row) => {
							width_changed = abs(resize.new.w - resize.old.w) > 0.5
							height_changed = abs(resize.new.h - resize.old.h) > 0.5
							if !width_changed and !height_changed {
								(cols, False)
							} else {
								edited_layout(
									cols,
									resize.layout,
									False,
									|edit|
										match column_at(edit, col) {
											None => (edit, True)
											Some(column) => {
												widened =
													if width_changed {
														{ ..column, width: Proportion(clamp_ratio(cols.settings, width_ratio(resize.tiling.w, resize.horizontal_gap, resize.new.w))) }
													} else {
														column
													}
												heightened =
													if height_changed and widened.rows.len() > 1 {
														total = max(resize.tiling.h - resize.vertical_gap * (widened.rows.len() - 1).to_f64(), 1.0)
														set_height_share(widened, row, clamp(resize.new.h / total, 0.05, 0.95), total)
													} else {
														widened
													}
												(set_column(edit, col, heightened), True)
											}
										},
								)
							}
						}
					}
			}
		}

	## The first item `pick` names a window for.
	first_some : List(a), (a -> MaybeWord) -> MaybeWord
	first_some = |list, pick|
		list.fold_until(
			Nothing,
			|_, item|
				match pick(item) {
					Some(found) => Break(Some(found))
					Nothing => Continue(Nothing)
				},
		)

	## The last item `pick` names a window for (Rust's `.rev().find_map`).
	last_some : List(a), (a -> MaybeWord) -> MaybeWord
	last_some = |list, pick|
		list.fold(
			Nothing,
			|found, item|
				match pick(item) {
					Some(named) => Some(named)
					Nothing => found
				},
		)

	is_tiled : Fill -> Bool
	is_tiled = |fill|
		match fill {
			Tiled => True
			Screen | WithinGaps => False
		}

	same_fill : Fill, Fill -> Bool
	same_fill = |a, b|
		match (a, b) {
			(Tiled, Tiled) | (Screen, Screen) | (WithinGaps, WithinGaps) => True
			_ => False
		}

	abs : F64 -> F64
	abs = |x| if x < 0.0 -x else x

	switch_preset : Cols, U64, Bool, ColumnsMaybeSpan -> (Cols, Bool)
	switch_preset = |cols, layout, backwards, span|
		match editing_of(cols, layout) {
			None => (cols, False)
			Some(e) =>
				match selected_location(e, cols.left_out) {
					Nowhere => (cols, False)
					At(col, _) =>
						match column_at(e, col) {
							None => (cols, False)
							Some(column) => {
								presets = cols.settings.preset_column_widths.keep_if(|r| F64.is_finite(r) and r > 0.0)
								match presets.first() {
									Err(_) => (cols, False)
									Ok(first) => {
										known =
											match column.width {
												Proportion(ratio) =>
													match presets.find_first_index(|p| abs(ratio - p) <= 0.000001) {
														Ok(index) => Some(index)
														Err(_) => None
													}
												_ => None
											}
										target =
											match known {
												Some(index) => {
													len = presets.len()
													next = (index + if backwards len - 1 else 1) % len
													presets.get(next) ?? first
												}
												None =>
													match span {
														NoSpan => -1.0
														Span(s) => {
															resolved = |r| proportional_width(s.view, s.gap, clamp_ratio(cols.settings, r))
															if backwards {
																match presets.fold(NoPreset, |found, r| if resolved(r) + 1.0 < s.width Preset(r) else found) {
																	Preset(r) => r
																	NoPreset => presets.last() ?? first
																}
															} else {
																match presets.find_first(|r| s.width + 1.0 < resolved(r)) {
																	Ok(r) => r
																	Err(_) => first
																}
															}
														}
													}
											}
										unchanged =
											match column.width {
												Proportion(r) => r == target
												_ => False
											}
										if target < 0.0 or unchanged {
											(cols, False)
										} else {
											edited_layout(cols, layout, False, |edit| (set_column(edit, col, { ..column, width: Proportion(target) }), True))
										}
									}
								}
							}
						}
				}
		}

	toggle_fullscreen : Cols, U64, Bool -> (Cols, ColumnsFullscreened)
	toggle_fullscreen = |cols, layout, within_gaps| {
		fill : Fill
		fill = if within_gaps WithinGaps else Screen
		edited_layout(
			cols,
			layout,
			Nothing,
			|e|
				match selected_location(e, cols.left_out) {
					Nowhere => (e, Nothing)
					At(col, row) =>
						match column_at(e, col) {
							None => (e, Nothing)
							Some(column) =>
								match column.rows.get(row) {
									Err(_) => (e, Nothing)
									Ok(r) =>
										if same_fill(r.fill, fill) {
											(set_column(e, col, { ..column, rows: set_at(column.rows, row, { ..r, fill: Tiled }) }), Left(r.window))
										} else {
											(set_column(e, col, { ..column, rows: set_at(column.rows, row, { ..r, fill }) }), Entered(r.window))
										}
								}
						}
				},
		)
	}

	preserve_window_sizes : Cols, ColumnsPreserve -> (Cols, List(U64))
	preserve_window_sizes = |cols, { layout, view: across, gap, locked }|
		if !cols.settings.preserve_window_sizes {
			(cols, [])
		} else {
			edited_layout(
				cols,
				layout,
				[],
				|e| {
					(columns, fixed) = e.columns.fold(
						([], []),
						|(acc, ids), column|
							match (column.rows, column.width) {
								([row], Default) if !cols.left_out.contains(row.window) =>
									match locked.find_first(|l| l.window == row.window) {
										Ok(held) if F64.is_finite(held.width) and held.width > 0.0 => {
											ratio = clamp_ratio(cols.settings, width_ratio(across, gap, held.width))
											(acc.append({ ..column, width: Fixed(proportional_width(across, gap, ratio)) }), ids.append(column.id))
										}
										_ => (acc.append(column), ids)
									}
								_ => (acc.append(column), ids)
							},
					)
					({ ..e, columns }, fixed)
				},
			)
		}
}
