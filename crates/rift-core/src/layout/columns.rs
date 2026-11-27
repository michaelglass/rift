//! A scrolling layout's arrangement as the core holds it: the strip of columns, each a stack of
//! rows (a window, its height, its fill), the active column and each column's active row, and
//! the windows left out, ported operation by operation from rift's `ScrollingLayoutSystem`
//! (`Arrangement` and the system's structural operations). The camera (its offset, spring,
//! gesture, bookmarks and reveal) is not here: it reads world geometry, and stays rift's.
//!
//! Rift's rules are the spec. Where this module's types differ from rift's they hold only the
//! states rift's operations reach: a strip is empty or has an active column ([`Strip`]); a column
//! is never empty and always has an active row ([`Column`]); a window's fullscreen mode is on its
//! row ([`Row::fill`]), so it moves, swaps and is replaced with the window. The operations run
//! on an editing form with rift's indices ([`Editing`]), which is parsed back into these types
//! after each, clamping the active indices as rift does.

use std::collections::BTreeSet;

pub use super::bsp_tree::{
    Direction, Edited, Focus, Focused, InsertionPoint, Participation, Query,
};
pub use super::traditional::Fill;
use crate::plain::Rect;
use crate::slot_table::{SlotKey, SlotTable};

/// A column's id: even, from 2, so it never collides with a window's odd one in rift's views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ColumnId(pub u64);

/// How wide a column is asked to be: the settings' default (or a preserved window width), a
/// proportion of the view, or a fixed width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Width {
    Default,
    Proportion(f64),
    Fixed(f64),
}

/// A window in a column: its height (a weight among the column's rows) and its fill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Row<W> {
    pub window: W,
    pub height: f64,
    pub fill: Fill,
}

/// A column: its rows around the active one, which always exists.
#[derive(Debug, Clone, PartialEq)]
pub struct Column<W> {
    pub id: ColumnId,
    pub width: Width,
    pub before: Vec<Row<W>>,
    pub active: Row<W>,
    pub after: Vec<Row<W>>,
}

/// A layout's columns: none, or those around the active one.
#[derive(Debug, Clone, PartialEq)]
pub enum Strip<W> {
    Empty,
    Columns {
        before: Vec<Column<W>>,
        active: Column<W>,
        after: Vec<Column<W>>,
    },
}

/// A layout's strip and the id its next column takes.
#[derive(Debug, Clone, PartialEq)]
pub struct Arrangement<W> {
    pub strip: Strip<W>,
    pub next_column: u64,
}

impl<W> Default for Arrangement<W> {
    fn default() -> Self {
        Arrangement {
            strip: Strip::Empty,
            next_column: 0,
        }
    }
}

/// The settings the arrangement reads: where a new window goes, and the column widths.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub insertion: InsertionPoint,
    pub column_width_ratio: f64,
    pub min_column_width_ratio: f64,
    pub max_column_width_ratio: f64,
    pub preset_column_widths: Vec<f64>,
    /// Whether a window's locked width fixes its column's (rift's `preserve_window_sizes`).
    pub preserve_window_sizes: bool,
}

impl Settings {
    /// A ratio within the settings' bounds (rift's `clamp_ratio`).
    pub fn clamp_ratio(&self, ratio: f64) -> f64 {
        let min = self.min_column_width_ratio.max(0.05);
        let max = self.max_column_width_ratio.max(min);
        if ratio.is_finite() {
            ratio.clamp(min, max)
        } else {
            min
        }
    }
}

/// The width a ratio gives in a view, gaps between columns (rift's `proportional_width`).
pub fn proportional_width(view: f64, gap: f64, ratio: f64) -> f64 {
    ((view + gap) * ratio - gap).max(1.0)
}

/// The ratio a width is of a view (rift's `width_ratio`).
pub fn width_ratio(view: f64, gap: f64, width: f64) -> f64 { (width + gap) / (view + gap).max(1.0) }

/// Where the selection moves to (rift's `MoveDestination`): a new column of its own at this
/// index, the place of another column, or of another row in its column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moved {
    NewColumn(usize),
    Column(usize),
    Row(usize),
}

/// What a window dropped onto another does, of the actions that reach the arrangement (rift
/// handles a drop's move and swap first).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transfer {
    Stack,
    Insert(Direction),
}

/// A drop as the system takes it (upstream's `WindowDropAction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropAction {
    Swap,
    Move(Direction),
    Transfer(Transfer),
}

/// What removing a window did to a layout it was in, for the camera: whether it was the
/// selection, its column went with it, and it was fullscreen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Removal {
    pub active_removed: bool,
    pub column_removed: bool,
    pub was_fullscreen: bool,
}

/// What a fullscreen toggle did to the selected window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fullscreened<W> {
    Nothing,
    Entered(W),
    Left(W),
}

/// The selected column as the current layout pass lays it out: the view's width, the gap
/// between columns, and the column's width (a fact only the geometry knows).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub view: f64,
    pub gap: f64,
    pub width: f64,
}

/// A window's frame changed natively (rift's `WindowChange::Resize`): its old and new frame, the
/// tiling area and the gaps between columns and rows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameResize {
    pub old: Rect,
    pub new: Rect,
    pub tiling: Rect,
    pub horizontal_gap: f64,
    pub vertical_gap: f64,
}

/// Which way a resize goes (rift's `ResizeOrientation`).
pub use super::bsp_tree::ResizeAlong;

/// A column as a list of rows with its active row's index (rift's saved form).
#[derive(Debug, Clone, PartialEq)]
pub struct Listed<W> {
    pub id: ColumnId,
    pub width: Width,
    pub rows: Vec<Row<W>>,
    pub active_row: usize,
}

/// The arrangement with rift's indices, as its operations edit it.
#[derive(Debug, Clone)]
struct Editing<W> {
    columns: Vec<EditColumn<W>>,
    active_column: usize,
    next_column_id: u64,
}

#[derive(Debug, Clone)]
struct EditColumn<W> {
    id: ColumnId,
    rows: Vec<Row<W>>,
    active_window: usize,
    width: Width,
}

impl<W: Copy> Column<W> {
    fn rows(&self) -> impl Iterator<Item = &Row<W>> {
        self.before.iter().chain(std::iter::once(&self.active)).chain(&self.after)
    }

    fn editing(&self) -> EditColumn<W> {
        EditColumn {
            id: self.id,
            rows: self.rows().copied().collect(),
            active_window: self.before.len(),
            width: self.width,
        }
    }

    /// The column a list of rows makes, the active index clamped as rift clamps it; none for no
    /// rows.
    fn of(edit: EditColumn<W>) -> Option<Column<W>> {
        let EditColumn {
            id,
            mut rows,
            active_window,
            width,
        } = edit;
        if rows.is_empty() {
            return None;
        }
        let at = active_window.min(rows.len() - 1);
        let after = rows.split_off(at + 1);
        let active = rows.pop()?;
        Some(Column {
            id,
            width,
            before: rows,
            active,
            after,
        })
    }

    /// Its rows in order.
    pub fn listed(&self) -> Vec<Row<W>> { self.rows().copied().collect() }

    /// Its active row's index.
    pub fn active_index(&self) -> usize { self.before.len() }
}

impl<W: Copy> Strip<W> {
    /// Its columns in order.
    pub fn columns(&self) -> Vec<&Column<W>> {
        match self {
            Strip::Empty => Vec::new(),
            Strip::Columns { before, active, after } => {
                before.iter().chain(std::iter::once(active)).chain(after).collect()
            }
        }
    }

    /// The active column's index (none for an empty strip).
    pub fn active_index(&self) -> Option<usize> {
        match self {
            Strip::Empty => None,
            Strip::Columns { before, .. } => Some(before.len()),
        }
    }
}

impl<W: Copy + Eq> Arrangement<W> {
    fn editing(&self) -> Editing<W> {
        Editing {
            columns: self.strip.columns().into_iter().map(Column::editing).collect(),
            active_column: self.strip.active_index().unwrap_or(0),
            next_column_id: self.next_column,
        }
    }

    /// The arrangement an edit leaves: columns with no rows dropped, the active indices clamped.
    fn of(edit: Editing<W>) -> Arrangement<W> {
        let Editing {
            columns,
            active_column,
            next_column_id,
        } = edit;
        let mut columns: Vec<Column<W>> = columns.into_iter().filter_map(Column::of).collect();
        let strip = if columns.is_empty() {
            Strip::Empty
        } else {
            let at = active_column.min(columns.len() - 1);
            let after = columns.split_off(at + 1);
            let active = columns.pop().expect("a column at the active index");
            Strip::Columns { before: columns, active, after }
        };
        Arrangement {
            strip,
            next_column: next_column_id,
        }
    }

    /// The arrangement listed columns make (rift's form): a column with no rows is none, and the
    /// active indices are clamped as rift's edits clamp them.
    pub fn from_listed(columns: Vec<Listed<W>>, active_column: usize, next_column: u64) -> Self {
        Arrangement::of(Editing {
            columns: columns
                .into_iter()
                .map(|c| EditColumn {
                    id: c.id,
                    rows: c.rows,
                    active_window: c.active_row,
                    width: c.width,
                })
                .collect(),
            active_column,
            next_column_id: next_column,
        })
    }

    /// The arrangement with an edit made to it, the edit's answer beside.
    fn edited<R>(&mut self, edit: impl FnOnce(&mut Editing<W>) -> R) -> R {
        let mut editing = self.editing();
        let answer = edit(&mut editing);
        *self = Arrangement::of(editing);
        answer
    }

    /// Every window, column by column.
    pub fn windows(&self) -> impl Iterator<Item = W> + '_ {
        self.strip
            .columns()
            .into_iter()
            .flat_map(|c| c.rows().map(|r| r.window).collect::<Vec<_>>())
    }

    /// The column and row holding `window`.
    pub fn locate(&self, window: W) -> Option<(usize, usize)> { self.editing().locate(window) }
}

impl<W: Copy + Eq> Editing<W> {
    fn selected(&self, left_out: &impl Fn(W) -> bool) -> Option<W> {
        let (col, row) = self.selected_location(left_out)?;
        Some(self.columns[col].rows[row].window)
    }

    fn selected_location(&self, left_out: &impl Fn(W) -> bool) -> Option<(usize, usize)> {
        let column = self.columns.get(self.active_column)?;
        let row = if column.selected().is_some_and(|&w| !left_out(w)) {
            column.active_window
        } else {
            column.rows.iter().position(|r| !left_out(r.window))?
        };
        Some((self.active_column, row))
    }

    fn locate(&self, window: W) -> Option<(usize, usize)> {
        self.columns.iter().enumerate().find_map(|(col, column)| {
            column.rows.iter().position(|r| r.window == window).map(|row| (col, row))
        })
    }

    fn move_destination(
        &self,
        (col, row): (usize, usize),
        direction: Direction,
        left_out: &impl Fn(W) -> bool,
    ) -> Option<Moved> {
        let horizontal = matches!(direction, Direction::Left | Direction::Right);
        let rows = &self.columns[col].rows;
        if horizontal && rows.len() > 1 {
            return Some(Moved::NewColumn(
                col + usize::from(direction == Direction::Right),
            ));
        }
        let (index, len) = if horizontal {
            (col, self.columns.len())
        } else {
            (row, rows.len())
        };
        let participates = |candidate: &usize| {
            if horizontal {
                self.columns[*candidate].rows.iter().any(|r| !left_out(r.window))
            } else {
                !left_out(rows[*candidate].window)
            }
        };
        let target = match direction {
            Direction::Left | Direction::Up => (0..index).rev().find(participates),
            Direction::Right | Direction::Down => (index + 1..len).find(participates),
        }?;
        Some(if horizontal {
            Moved::Column(target)
        } else {
            Moved::Row(target)
        })
    }

    fn activate(&mut self, window: W) -> bool {
        let Some((col, row)) = self.locate(window) else {
            return false;
        };
        self.active_column = col;
        self.columns[col].active_window = row;
        true
    }

    fn new_column(&mut self, index: usize, row: Row<W>, width: Width) {
        self.next_column_id = self.next_column_id.max(2);
        let id = ColumnId(self.next_column_id);
        self.next_column_id += 2;
        let window = row.window;
        self.columns.insert(index.min(self.columns.len()), EditColumn {
            id,
            rows: vec![row],
            active_window: 0,
            width,
        });
        self.activate(window);
    }

    /// `window` taken out, its column's width and its row kept (rift's `detach`).
    fn detach(&mut self, window: W, left_out: &impl Fn(W) -> bool) -> Option<(Width, Row<W>)> {
        let (col, row) = self.locate(window)?;
        let selected = self.selected(left_out);
        let column = &mut self.columns[col];
        let width = column.width;
        let taken = column.rows.remove(row);
        if row < column.active_window {
            column.active_window -= 1;
        }
        column.active_window = column.active_window.min(column.rows.len().saturating_sub(1));
        if column.rows.is_empty() {
            self.columns.remove(col);
            if col < self.active_column {
                self.active_column -= 1;
            }
        }
        self.active_column = self.active_column.min(self.columns.len().saturating_sub(1));
        if let Some(selected) = selected.filter(|&w| w != window) {
            self.activate(selected);
        }
        Some((width, taken))
    }

    fn transfer(
        &mut self,
        source: W,
        target: W,
        action: Transfer,
        left_out: &impl Fn(W) -> bool,
    ) -> bool {
        if source == target || self.locate(source).is_none() || self.locate(target).is_none() {
            return false;
        }
        let Some((width, row)) = self.detach(source, left_out) else {
            return false;
        };
        let Some((col, at)) = self.locate(target) else {
            return false;
        };
        match action {
            Transfer::Stack | Transfer::Insert(Direction::Up | Direction::Down) => {
                let at = at + usize::from(action != Transfer::Insert(Direction::Up));
                let column = &mut self.columns[col];
                if at <= column.active_window {
                    column.active_window += 1;
                }
                column.rows.insert(at, row);
            }
            Transfer::Insert(direction @ (Direction::Left | Direction::Right)) => {
                self.new_column(col + usize::from(direction == Direction::Right), row, width);
            }
        }
        self.activate(source);
        true
    }
}

/// The column `id` made `arrangement`'s active one, keeping its active row; whether it holds it.
fn activate<W: Copy + Eq>(arrangement: &mut Arrangement<W>, id: ColumnId) -> bool {
    arrangement.edited(|e| {
        let Some(index) = e.columns.iter().position(|c| c.id == id) else {
            return false;
        };
        e.active_column = index;
        true
    })
}

impl<W> EditColumn<W> {
    fn selected(&self) -> Option<&W> { self.rows.get(self.active_window).map(|r| &r.window) }
}

impl<W: Copy> EditColumn<W> {
    /// The row at `row` given `share` of `scale`, the others the rest in their proportions
    /// (rift's `set_height_share`).
    fn set_height_share(&mut self, row: usize, share: f64, scale: f64) {
        let other: f64 = self
            .rows
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != row)
            .map(|(_, r)| r.height)
            .sum();
        let others = self.rows.len().saturating_sub(1).max(1) as f64;
        for (index, r) in self.rows.iter_mut().enumerate() {
            r.height = if index == row {
                share * scale
            } else if other > 0.0 {
                (1.0 - share) * scale * r.height / other
            } else {
                (1.0 - share) * scale / others
            };
        }
    }
}

/// A scrolling layout system's arrangements: every layout's, the windows left out of layout,
/// and the settings its operations read.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnSystem<L, W> {
    layouts: SlotTable<L, Arrangement<W>>,
    left_out: BTreeSet<W>,
    settings: Settings,
}

impl<L: SlotKey + Eq, W: Copy + Ord> ColumnSystem<L, W> {
    pub fn new(settings: Settings) -> Self {
        ColumnSystem {
            layouts: SlotTable::default(),
            left_out: BTreeSet::new(),
            settings,
        }
    }

    /// A system as the platform holds it.
    pub fn from_parts(
        layouts: SlotTable<L, Arrangement<W>>,
        left_out: BTreeSet<W>,
        settings: Settings,
    ) -> Self {
        ColumnSystem { layouts, left_out, settings }
    }

    pub fn layouts(&self) -> &SlotTable<L, Arrangement<W>> { &self.layouts }

    pub fn left_out(&self) -> &BTreeSet<W> { &self.left_out }

    pub fn settings(&self) -> &Settings { &self.settings }

    pub fn set_settings(&mut self, settings: Settings) { self.settings = settings; }

    pub fn arrangement(&self, layout: L) -> Option<&Arrangement<W>> { self.layouts.get(layout) }

    fn is_left_out(&self) -> impl Fn(W) -> bool + '_ { |window| self.left_out.contains(&window) }

    pub fn window_participates(&self, window: W) -> bool { !self.left_out.contains(&window) }

    /// A window taking part or left out; whether that changed. Every layout holding it gets a
    /// selection again if it had none taking part.
    pub fn set_window_participation(&mut self, window: W, participation: Participation) -> bool {
        let changed = match participation {
            Participation::TakesPart => self.left_out.remove(&window),
            Participation::LeftOut => self.left_out.insert(window),
        };
        if !changed {
            return false;
        }
        let left_out = self.left_out.clone();
        let is_left_out = |w: W| left_out.contains(&w);
        for arrangement in self.layouts.values_mut() {
            if arrangement.locate(window).is_none() {
                continue;
            }
            arrangement.edited(|e| {
                if e.selected(&is_left_out).is_none()
                    && let Some((col, row)) = e.columns.iter().enumerate().find_map(|(col, c)| {
                        c.rows.iter().position(|r| !is_left_out(r.window)).map(|row| (col, row))
                    })
                {
                    e.active_column = col;
                    e.columns[col].active_window = row;
                }
            });
        }
        true
    }

    pub fn create_layout(&mut self) -> L { self.layouts.insert(Arrangement::default()) }

    pub fn contains_layout(&self, layout: L) -> bool { self.layouts.contains_key(layout) }

    pub fn clone_layout(&mut self, layout: L) -> L {
        let arrangement = self.layouts.get(layout).cloned().unwrap_or_default();
        self.layouts.insert(arrangement)
    }

    pub fn remove_layout(&mut self, layout: L) { self.layouts.remove(layout); }

    /// A new window in `layout`: a column of its own after the selection's, or at the end; false
    /// when the layout is unknown or already holds it.
    pub fn insert(&mut self, layout: L, window: W) -> bool {
        let insertion = self.settings.insertion;
        let left_out = self.left_out.clone();
        let Some(arrangement) = self.layouts.get_mut(layout) else {
            return false;
        };
        if arrangement.locate(window).is_some() {
            return false;
        }
        arrangement.edited(|e| {
            let index = match insertion {
                InsertionPoint::EndOfTree => e.columns.len(),
                InsertionPoint::NextToSelection => {
                    e.selected_location(&|w| left_out.contains(&w)).map_or(0, |(col, _)| col + 1)
                }
            };
            let row = Row {
                window,
                height: 1.0,
                fill: Fill::Tiled,
            };
            e.new_column(index, row, Width::Default);
        });
        true
    }

    /// `window` going: it is no longer left out, and the layouts holding it, in layout order. The
    /// caller removes it from each ([`Self::remove_from`]) and, after each removal, makes active
    /// the columns the camera chooses ([`Self::activate_column`]): the core never asks the camera.
    pub fn forget_window(&mut self, window: W) -> Vec<L> {
        self.left_out.remove(&window);
        self.layouts
            .iter()
            .filter(|(_, arrangement)| arrangement.locate(window).is_some())
            .map(|(layout, _)| layout)
            .collect()
    }

    /// `window` removed from `layout`: what that did, for the camera; none when the layout does
    /// not hold it.
    pub fn remove_from(&mut self, layout: L, window: W) -> Option<Removal> {
        let left_out = self.left_out.clone();
        let is_left_out = |w: W| left_out.contains(&w);
        let arrangement = self.layouts.get_mut(layout)?;
        let (col, row) = arrangement.locate(window)?;
        Some(arrangement.edited(|e| {
            let removal = Removal {
                active_removed: e.selected(&is_left_out) == Some(window),
                column_removed: e.columns[col].rows.len() == 1,
                was_fullscreen: e.columns[col].rows[row].fill != Fill::Tiled,
            };
            e.detach(window, &is_left_out);
            removal
        }))
    }

    /// `from` replaced by `to` wherever it is, its row and its participation kept.
    pub fn replace_window(&mut self, from: W, to: W) {
        if self.left_out.remove(&from) {
            self.left_out.insert(to);
        }
        for arrangement in self.layouts.values_mut() {
            if arrangement.locate(from).is_none() {
                continue;
            }
            arrangement.edited(|e| {
                for row in e.columns.iter_mut().flat_map(|c| c.rows.iter_mut()) {
                    if row.window == from {
                        row.window = to;
                    }
                }
            });
        }
    }

    /// An app's windows (`is_app`) going: none is left out any more, and each layout's windows of
    /// the app, in layout order and each layout's own order (layouts holding none left out). The
    /// caller removes them one by one ([`Self::remove_from`]), the camera answering each removal
    /// before the next, as for [`Self::forget_window`].
    pub fn forget_app(&mut self, is_app: impl Fn(W) -> bool) -> Vec<(L, Vec<W>)> {
        self.left_out.retain(|&w| !is_app(w));
        self.layouts
            .iter()
            .map(|(layout, arrangement)| {
                (
                    layout,
                    arrangement.windows().filter(|&w| is_app(w)).collect::<Vec<_>>(),
                )
            })
            .filter(|(_, windows)| !windows.is_empty())
            .collect()
    }

    pub fn has_windows_for_app(&self, layout: L, is_app: impl Fn(W) -> bool) -> bool {
        self.layouts.get(layout).is_some_and(|a| a.windows().any(is_app))
    }

    pub fn contains_window(&self, layout: L, window: W) -> bool {
        self.layouts.get(layout).is_some_and(|a| a.locate(window).is_some())
    }

    pub fn selected_window(&self, layout: L) -> Option<W> {
        self.layouts.get(layout)?.editing().selected(&self.is_left_out())
    }

    /// The selection's column and row.
    pub fn selected_location(&self, layout: L) -> Option<(usize, usize)> {
        self.layouts.get(layout)?.editing().selected_location(&self.is_left_out())
    }

    /// The window a focus move in `direction` reaches.
    pub fn window_in_direction(&self, layout: L, direction: Direction) -> Option<W> {
        let e = self.layouts.get(layout)?.editing();
        let left_out = self.is_left_out();
        let (col, row) = e.selected_location(&left_out)?;
        let selected = |column: &EditColumn<W>| {
            column
                .selected()
                .copied()
                .filter(|&w| !left_out(w))
                .or_else(|| column.rows.iter().map(|r| r.window).find(|&w| !left_out(w)))
        };
        match direction {
            Direction::Left => e.columns[..col].iter().rev().find_map(selected),
            Direction::Right => e.columns[col + 1..].iter().find_map(selected),
            Direction::Up => e.columns[col].rows[..row]
                .iter()
                .rev()
                .map(|r| r.window)
                .find(|&w| !left_out(w)),
            Direction::Down => {
                e.columns[col].rows[row + 1..].iter().map(|r| r.window).find(|&w| !left_out(w))
            }
        }
    }

    pub fn has_any_fullscreen_node(&self, layout: L) -> bool {
        self.layouts.get(layout).is_some_and(|a| {
            a.strip
                .columns()
                .into_iter()
                .flat_map(|c| c.listed())
                .any(|r| r.fill != Fill::Tiled && self.window_participates(r.window))
        })
    }

    pub fn parent_of_selection_is_stacked(&self, layout: L) -> bool {
        self.layouts.get(layout).is_some_and(|a| match &a.strip {
            Strip::Empty => false,
            Strip::Columns { active, .. } => !active.before.is_empty() || !active.after.is_empty(),
        })
    }

    /// The windows taking part in the active column.
    fn visible_windows_under_selection(&self, layout: L) -> Vec<W> {
        match self.layouts.get(layout).map(|a| &a.strip) {
            Some(Strip::Columns { active, .. }) => active
                .listed()
                .into_iter()
                .map(|r| r.window)
                .filter(|&w| self.window_participates(w))
                .collect(),
            Some(Strip::Empty) | None => Vec::new(),
        }
    }

    pub fn windows(&self, layout: L, query: Query<W>) -> Vec<W> {
        match query {
            Query::All | Query::Visible => self
                .layouts
                .get(layout)
                .into_iter()
                .flat_map(|a| a.windows().collect::<Vec<_>>())
                .filter(|&w| matches!(query, Query::All) || self.window_participates(w))
                .collect(),
            Query::Selection => self.visible_windows_under_selection(layout),
            Query::Stack(_) => Vec::new(),
        }
    }

    /// A window's column and row.
    pub fn window_slot(&self, layout: L, window: W) -> Option<Vec<usize>> {
        let (col, row) = self.layouts.get(layout)?.locate(window)?;
        Some(vec![col, row])
    }

    /// Whether `window` can move in `direction`.
    pub fn can_move_window(&self, layout: L, window: W, direction: Direction) -> bool {
        self.window_participates(window)
            && self.layouts.get(layout).is_some_and(|a| {
                let e = a.editing();
                e.locate(window).is_some_and(|slot| {
                    e.move_destination(slot, direction, &self.is_left_out()).is_some()
                })
            })
    }

    /// The selection made `window`: true when it is (already, or now).
    pub fn select_window(&mut self, layout: L, window: W) -> bool {
        if !self.window_participates(window) {
            return false;
        }
        let left_out = self.left_out.clone();
        let Some(arrangement) = self.layouts.get_mut(layout) else {
            return false;
        };
        if arrangement.editing().selected(&|w| left_out.contains(&w)) == Some(window) {
            return true;
        }
        arrangement.edited(|e| e.activate(window))
    }

    pub fn focus(&mut self, layout: L, focus: Focus<W>) -> Focused<W> {
        match focus {
            Focus::Select(window) => Focused::Selection {
                handled: self.select_window(layout, window),
            },
            Focus::Ascend => Focused::Selection {
                handled: self.move_focus(layout, Direction::Up).0.is_some(),
            },
            Focus::Descend => Focused::Selection {
                handled: self.move_focus(layout, Direction::Down).0.is_some(),
            },
            Focus::Direction(direction) => {
                let (window, raise) = self.move_focus(layout, direction);
                Focused::Movement { window, raise }
            }
        }
    }

    fn move_focus(&mut self, layout: L, direction: Direction) -> (Option<W>, Vec<W>) {
        let Some(window) = self.window_in_direction(layout, direction) else {
            return (None, Vec::new());
        };
        self.select_window(layout, window);
        (Some(window), self.visible_windows_under_selection(layout))
    }

    pub fn swap_windows(&mut self, layout: L, a: W, b: W) -> bool {
        if !self.window_participates(a) || !self.window_participates(b) {
            return false;
        }
        let left_out = self.left_out.clone();
        let Some(arrangement) = self.layouts.get_mut(layout) else {
            return false;
        };
        let (Some((ac, ar)), Some((bc, br))) = (arrangement.locate(a), arrangement.locate(b))
        else {
            return false;
        };
        arrangement.edited(|e| {
            let selected = e.selected(&|w| left_out.contains(&w));
            let first = e.columns[ac].rows[ar];
            let second = e.columns[bc].rows[br];
            e.columns[ac].rows[ar] = second;
            e.columns[bc].rows[br] = first;
            if let Some(window) = selected {
                e.activate(window);
            }
        });
        true
    }

    /// The selection moved in `direction`: where it went, if anywhere.
    pub fn move_selection(&mut self, layout: L, direction: Direction) -> Option<Moved> {
        let left_out = self.left_out.clone();
        let is_left_out = |w: W| left_out.contains(&w);
        let arrangement = self.layouts.get_mut(layout)?;
        arrangement.edited(|e| {
            let (col, row) = e.selected_location(&is_left_out)?;
            let destination = e.move_destination((col, row), direction, &is_left_out)?;
            match destination {
                Moved::NewColumn(target) => {
                    let window = e.columns[col].rows[row].window;
                    let (width, taken) = e.detach(window, &is_left_out)?;
                    e.new_column(target, taken, width);
                }
                Moved::Column(target) => {
                    e.columns.swap(col, target);
                    e.active_column = target;
                }
                Moved::Row(target) => {
                    let column = &mut e.columns[col];
                    column.rows.swap(row, target);
                    column.active_window = target;
                }
            }
            Some(destination)
        })
    }

    /// A drop of `source` onto `target`: whether it changed anything.
    pub fn drop_window(&mut self, layout: L, source: W, target: W, action: DropAction) -> bool {
        if !self.window_participates(source) || !self.window_participates(target) {
            return false;
        }
        match action {
            DropAction::Move(direction) => {
                self.select_window(layout, source)
                    && self.move_selection(layout, direction).is_some()
            }
            DropAction::Swap => self.swap_windows(layout, source, target),
            DropAction::Transfer(transfer) => {
                let left_out = self.left_out.clone();
                let Some(arrangement) = self.layouts.get_mut(layout) else {
                    return false;
                };
                arrangement
                    .edited(|e| e.transfer(source, target, transfer, &|w| left_out.contains(&w)))
            }
        }
    }

    /// The selection stacked into its neighbour's column on `direction`'s side (rift's
    /// `join_selection_with_direction`).
    pub fn join_selection(&mut self, layout: L, direction: Direction) -> bool {
        if !matches!(direction, Direction::Left | Direction::Right) {
            return false;
        }
        let Some(target) = self.window_in_direction(layout, direction) else {
            return false;
        };
        let Some(source) = self.selected_window(layout) else {
            return false;
        };
        self.drop_window(layout, source, target, DropAction::Transfer(Transfer::Stack))
    }

    /// Out of its column if it shares one, else into its neighbour's (rift's consume or expel).
    pub fn consume_or_expel(&mut self, layout: L, direction: Direction) -> Edited<W> {
        if !matches!(direction, Direction::Left | Direction::Right) {
            return Edited::Unchanged;
        }
        let changed = if self.parent_of_selection_is_stacked(layout) {
            self.move_selection(layout, direction).is_some()
        } else {
            self.join_selection(layout, direction)
        };
        if changed {
            Edited::Changed
        } else {
            Edited::Unchanged
        }
    }

    /// The selection's column takes its next neighbour's rows (its previous one's at the end):
    /// the windows that moved.
    pub fn stack(&mut self, layout: L) -> Vec<W> {
        let left_out = self.left_out.clone();
        let is_left_out = |w: W| left_out.contains(&w);
        let Some(arrangement) = self.layouts.get_mut(layout) else {
            return Vec::new();
        };
        arrangement.edited(|e| {
            let Some((col, _)) = e.selected_location(&is_left_out) else {
                return Vec::new();
            };
            let target = if col + 1 < e.columns.len() {
                col + 1
            } else if col > 0 {
                col - 1
            } else {
                return Vec::new();
            };
            let moved: Vec<W> = e.columns[target].rows.iter().map(|r| r.window).collect();
            let Some(selected) = e.selected(&is_left_out) else {
                return Vec::new();
            };
            let mut neighbour = e.columns.remove(target);
            let column = &mut e.columns[col - usize::from(target < col)];
            column.rows.append(&mut neighbour.rows);
            e.activate(selected);
            moved
        })
    }

    /// Every other window of the selection's column a column of its own after it: the windows
    /// that moved.
    pub fn unstack(&mut self, layout: L) -> Vec<W> {
        let left_out = self.left_out.clone();
        let is_left_out = |w: W| left_out.contains(&w);
        let Some(arrangement) = self.layouts.get_mut(layout) else {
            return Vec::new();
        };
        arrangement.edited(|e| {
            let Some((col, _)) = e.selected_location(&is_left_out) else {
                return Vec::new();
            };
            let Some(selected) = e.selected(&is_left_out) else {
                return Vec::new();
            };
            let moved: Vec<W> = e.columns[col]
                .rows
                .iter()
                .map(|r| r.window)
                .filter(|&w| w != selected)
                .collect();
            for (index, &window) in moved.iter().enumerate() {
                if let Some((width, row)) = e.detach(window, &is_left_out) {
                    e.new_column(col + 1 + index, row, width);
                }
            }
            e.activate(selected);
            moved
        })
    }

    /// The selection's column out of its neighbour's (rift's unjoin: a move right of a stacked
    /// selection).
    pub fn unjoin(&mut self, layout: L) -> bool {
        self.parent_of_selection_is_stacked(layout)
            && self.move_selection(layout, Direction::Right).is_some()
    }

    /// The selection resized by `amount` (a share of its column, or of the view's width);
    /// `span` the selected column as laid out, when it is.
    pub fn resize_selection(
        &mut self,
        layout: L,
        amount: f64,
        along: ResizeAlong,
        span: Option<Span>,
    ) -> Edited<W> {
        if !amount.is_finite() {
            return Edited::Unchanged;
        }
        let settings = self.settings.clone();
        let left_out = self.left_out.clone();
        let Some(arrangement) = self.layouts.get_mut(layout) else {
            return Edited::Unchanged;
        };
        arrangement.edited(|e| {
            let Some((col, row)) = e.selected_location(&|w| left_out.contains(&w)) else {
                return Edited::Unchanged;
            };
            let column = &mut e.columns[col];
            let vertical = along == ResizeAlong::Vertical
                || (along == ResizeAlong::Smart && column.rows.len() > 1);
            if vertical {
                if column.rows.len() > 1 {
                    let total: f64 = column.rows.iter().map(|r| r.height).sum();
                    let share = (column.rows[row].height / total + amount).clamp(0.05, 0.95);
                    column.set_height_share(row, share, total.max(10_000.0));
                }
            } else {
                let ratio = span.map_or(
                    match column.width {
                        Width::Proportion(r) => r,
                        Width::Default | Width::Fixed(_) => settings.column_width_ratio,
                    },
                    |s| width_ratio(s.view, s.gap, s.width),
                );
                column.width = Width::Proportion(settings.clamp_ratio(ratio + amount));
            }
            Edited::Changed
        })
    }

    /// A window's frame changed natively: its column takes the new width, and its row the new
    /// height among a column's several rows; whether anything changed.
    pub fn resize_from_frame(&mut self, layout: L, window: W, resize: FrameResize) -> bool {
        if !self.window_participates(window) {
            return false;
        }
        let settings = self.settings.clone();
        let Some(arrangement) = self.layouts.get_mut(layout) else {
            return false;
        };
        let Some((col, row)) = arrangement.locate(window) else {
            return false;
        };
        let FrameResize {
            old,
            new,
            tiling,
            horizontal_gap,
            vertical_gap,
        } = resize;
        let width_changed = (new.w - old.w).abs() > 0.5;
        let height_changed = (new.h - old.h).abs() > 0.5;
        if !width_changed && !height_changed {
            return false;
        }
        arrangement.edited(|e| {
            if width_changed {
                e.columns[col].width = Width::Proportion(settings.clamp_ratio(width_ratio(
                    tiling.w,
                    horizontal_gap,
                    new.w,
                )));
            }
            let column = &mut e.columns[col];
            if height_changed && column.rows.len() > 1 {
                let total =
                    (tiling.h - vertical_gap * column.rows.len().saturating_sub(1) as f64).max(1.0);
                column.set_height_share(row, (new.h / total).clamp(0.05, 0.95), total);
            }
        });
        true
    }

    /// The selection's column the next (or previous) preset width; `span` the column as laid
    /// out, read when its width is no preset. Whether its width changed.
    pub fn switch_preset_column_width(
        &mut self,
        layout: L,
        backwards: bool,
        span: Option<Span>,
    ) -> bool {
        let settings = self.settings.clone();
        let left_out = self.left_out.clone();
        let Some(arrangement) = self.layouts.get_mut(layout) else {
            return false;
        };
        let Some((col, _)) = arrangement.editing().selected_location(&|w| left_out.contains(&w))
        else {
            return false;
        };
        let presets: Vec<f64> = settings
            .preset_column_widths
            .iter()
            .copied()
            .filter(|r| r.is_finite() && *r > 0.0)
            .collect();
        let Some(&first) = presets.first() else {
            return false;
        };
        let current = arrangement.editing().columns[col].width;
        let known = match current {
            Width::Proportion(ratio) => presets.iter().position(|p| (ratio - p).abs() <= 1e-6),
            Width::Default | Width::Fixed(_) => None,
        };
        let target = if let Some(index) = known {
            let len = presets.len();
            presets[(index + if backwards { len - 1 } else { 1 }) % len]
        } else {
            let Some(span) = span else {
                return false;
            };
            let resolved =
                |ratio| proportional_width(span.view, span.gap, settings.clamp_ratio(ratio));
            if backwards {
                presets
                    .iter()
                    .rev()
                    .copied()
                    .find(|&r| resolved(r) + 1.0 < span.width)
                    .unwrap_or(presets[presets.len() - 1])
            } else {
                presets
                    .iter()
                    .copied()
                    .find(|&r| span.width + 1.0 < resolved(r))
                    .unwrap_or(first)
            }
        };
        if matches!(current, Width::Proportion(r) if r == target) {
            return false;
        }
        arrangement.edited(|e| e.columns[col].width = Width::Proportion(target));
        true
    }

    /// The column `id` made the active one, keeping its active row (the camera's choice: the
    /// column a gesture settles on, or a remembered view's); whether `layout` holds it.
    pub fn activate_column(&mut self, layout: L, id: ColumnId) -> bool {
        self.layouts
            .get_mut(layout)
            .is_some_and(|arrangement| activate(arrangement, id))
    }

    /// A layout pass's preserved widths (with `preserve_window_sizes` set): each column of one
    /// window taking part, at the default width, whose window has a locked width (`locked`, a
    /// fact of the pass), fixed at the width the default resolves to for it in a view of `view`
    /// with `gap` between columns. The columns fixed, by id.
    pub fn preserve_window_sizes(
        &mut self,
        layout: L,
        view: f64,
        gap: f64,
        locked: impl Fn(W) -> Option<f64>,
    ) -> Vec<ColumnId> {
        let settings = self.settings.clone();
        if !settings.preserve_window_sizes {
            return Vec::new();
        }
        let left_out = self.left_out.clone();
        let Some(arrangement) = self.layouts.get_mut(layout) else {
            return Vec::new();
        };
        arrangement.edited(|e| {
            let mut fixed = Vec::new();
            for column in &mut e.columns {
                let [row] = column.rows.as_slice() else {
                    continue;
                };
                if column.width != Width::Default || left_out.contains(&row.window) {
                    continue;
                }
                let Some(locked) = locked(row.window).filter(|w| w.is_finite() && *w > 0.0) else {
                    continue;
                };
                let ratio = settings.clamp_ratio(width_ratio(view, gap, locked));
                column.width = Width::Fixed(proportional_width(view, gap, ratio));
                fixed.push(column.id);
            }
            fixed
        })
    }

    /// The selected window's fill toggled to `within_gaps`'s: entered, or left when it was that
    /// fill already.
    pub fn toggle_fullscreen(&mut self, layout: L, within_gaps: bool) -> Fullscreened<W> {
        let left_out = self.left_out.clone();
        let Some(arrangement) = self.layouts.get_mut(layout) else {
            return Fullscreened::Nothing;
        };
        let fill = if within_gaps {
            Fill::WithinGaps
        } else {
            Fill::Screen
        };
        arrangement.edited(|e| {
            let Some((col, row)) = e.selected_location(&|w| left_out.contains(&w)) else {
                return Fullscreened::Nothing;
            };
            let selected = &mut e.columns[col].rows[row];
            if selected.fill == fill {
                selected.fill = Fill::Tiled;
                Fullscreened::Left(selected.window)
            } else {
                selected.fill = fill;
                Fullscreened::Entered(selected.window)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Key(u32, u32);

    impl SlotKey for Key {
        fn from_parts(index: u32, version: u32) -> Self { Key(index, version) }

        fn parts(self) -> (u32, u32) { (self.0, self.1) }
    }

    fn row(window: u32) -> Row<u32> {
        Row {
            window,
            height: 1.0,
            fill: Fill::Tiled,
        }
    }

    fn listed(id: u64, rows: &[u32], active_row: usize) -> Listed<u32> {
        Listed {
            id: ColumnId(id),
            width: Width::Default,
            rows: rows.iter().map(|&w| row(w)).collect(),
            active_row,
        }
    }

    /// Rift's form parses into the strip: a column with no rows is none, and the active column
    /// and row clamp to the last as rift's edits clamp them.
    #[test]
    fn listed_columns_parse_with_the_active_indices_clamped_and_empty_columns_gone() {
        let parsed = Arrangement::from_listed(
            vec![listed(2, &[1, 2], 7), listed(4, &[], 0), listed(6, &[3], 0)],
            9,
            8,
        );
        let Strip::Columns { before, active, after } = &parsed.strip else {
            panic!("columns");
        };
        assert_eq!((before.len(), active.id, after.len()), (1, ColumnId(6), 0));
        assert_eq!(before[0].active.window, 2);
        assert_eq!(before[0].before.len(), 1);
        assert_eq!(
            Arrangement::<u32>::from_listed(vec![listed(2, &[], 0)], 0, 2).strip,
            Strip::Empty
        );
    }

    /// Removing the last window leaves an empty strip; the next column's id keeps counting by two.
    #[test]
    fn the_last_window_out_leaves_an_empty_strip_and_ids_keep_counting() {
        let mut system: ColumnSystem<Key, u32> = ColumnSystem::new(Settings {
            insertion: InsertionPoint::NextToSelection,
            column_width_ratio: 0.5,
            min_column_width_ratio: 0.1,
            max_column_width_ratio: 1.0,
            preset_column_widths: vec![0.5],
            preserve_window_sizes: false,
        });
        let layout = system.create_layout();
        system.insert(layout, 1);
        for layout in system.forget_window(1) {
            system.remove_from(layout, 1);
        }
        assert_eq!(system.arrangement(layout).unwrap().strip, Strip::Empty);
        system.insert(layout, 2);
        let Strip::Columns { active, .. } = &system.arrangement(layout).unwrap().strip else {
            panic!("a column");
        };
        assert_eq!(active.id, ColumnId(4));
    }
}
