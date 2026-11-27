//! The flip (scrolling slice 3, policy (a)): the core's arrangements (`rift_core::layout::columns`)
//! decide, rift's own are their shadow until slice 5. The system holds the core
//! ([`super::ScrollingLayoutSystem::core`]) and steps it with every operation in every build;
//! every answer is the core's, and saving writes the core's arrangements in rift's saved form.
//! Rift's arrangements step beside it, because the camera (offset, spring, gesture, bookmarks,
//! reveal) still reads them; in tests and debug builds after every operation, in a release build
//! after every [`EVERY`]th, they are held to the core's, and a difference is counted
//! ([`scrolling_divergences`]), fails a test or warns `layout-divergence:`, and re-seeds rift's
//! arrangements and windows left out from the core's.
//!
//! The camera's own writes to the arrangement (the column a gesture settles on, a remembered
//! view's column, the columns it makes active after a removal) are its decisions, and a layout
//! pass's preserved widths read the pass's constraints: each is logged ([`super::Logged`]) while
//! rift's side runs, and made on the core in order, a removal's answer after that removal.

use std::collections::{BTreeSet, VecDeque};
use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};

use rift_core::layout::columns::{self as core, Fill};
use rift_core::plain::Rect;

use super::{
    Arrangement, Column, ColumnId, ColumnWidth, FullscreenMode, LayoutState, Logged, Row,
    ScrollingLayoutSettings, ScrollingLayoutSystem,
};
use crate::actor::app::WindowId;
use crate::common::collections::{HashMap, HashSet};
use crate::common::config::{GapSettings, WindowInsertionPoint};
use crate::layout_engine::systems::{
    EditResult, FocusResult, GroupEdit, LayoutEdit, WindowChange, WindowQuery, core_direction,
};
use crate::layout_engine::utils::compute_tiling_area;
use crate::layout_engine::{LayoutId, ResizeOrientation, WindowDropAction};
use crate::model::slot_table::of_slotmap;

/// The core's arrangements over rift's ids.
pub(super) type CoreColumns = core::ColumnSystem<LayoutId, WindowId>;

/// What decides the arrangements: under `roc-core` the Roc columns, the Rust core their shadow
/// (`roc_columns`, scrolling slice 4); otherwise the Rust core. Both answer the Rust core's
/// methods, name for name.
#[cfg(feature = "roc-core")]
pub(super) type Decider = super::roc_columns::ColumnsDecider;
#[cfg(not(feature = "roc-core"))]
pub(super) type Decider = CoreColumns;

/// The decider holding what `core` holds.
pub(super) fn decider(core: CoreColumns) -> Decider {
    #[cfg(feature = "roc-core")]
    return super::roc_columns::ColumnsDecider::seeded(core);
    #[cfg(not(feature = "roc-core"))]
    core
}

/// A release build holds rift's arrangements to the core's after every this many operations.
const EVERY: u64 = 32;

static STEPS: AtomicU64 = AtomicU64::new(0);
static DIVERGENCES: AtomicU64 = AtomicU64::new(0);
static RUNS: AtomicU64 = AtomicU64::new(0);

/// How many times rift's arrangements have differed from the core's (each re-seeded).
pub(crate) fn scrolling_divergences() -> u64 { DIVERGENCES.load(Ordering::Relaxed) }

/// How many times rift's arrangements have been held to the core's.
pub(crate) fn scrolling_shadows() -> u64 { RUNS.load(Ordering::Relaxed) }

/// Whether this operation's result is held to rift's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Due {
    Compared,
    Skipped,
}

fn due() -> Due {
    if cfg!(debug_assertions) || STEPS.fetch_add(1, Ordering::Relaxed).is_multiple_of(EVERY) {
        Due::Compared
    } else {
        Due::Skipped
    }
}

/// The core's settings, read from rift's.
pub(super) fn settings(settings: &ScrollingLayoutSettings) -> core::Settings {
    core::Settings {
        insertion: match settings.base.window_insertion_point.unwrap_or_default() {
            WindowInsertionPoint::EndOfTree => core::InsertionPoint::EndOfTree,
            WindowInsertionPoint::NextToSelection => core::InsertionPoint::NextToSelection,
        },
        column_width_ratio: settings.column_width_ratio,
        min_column_width_ratio: settings.min_column_width_ratio,
        max_column_width_ratio: settings.max_column_width_ratio,
        preset_column_widths: settings.preset_column_widths.clone(),
        preserve_window_sizes: settings.preserve_window_sizes,
    }
}

fn width(width: ColumnWidth) -> core::Width {
    match width {
        ColumnWidth::Default => core::Width::Default,
        ColumnWidth::Proportion(ratio) => core::Width::Proportion(ratio),
        ColumnWidth::Fixed(width) => core::Width::Fixed(width),
    }
}

fn rift_width(width: core::Width) -> ColumnWidth {
    match width {
        core::Width::Default => ColumnWidth::Default,
        core::Width::Proportion(ratio) => ColumnWidth::Proportion(ratio),
        core::Width::Fixed(width) => ColumnWidth::Fixed(width),
    }
}

/// Rift's arrangement as the core holds it (a window's fullscreen mode on its row).
pub(super) fn arrangement(arrangement: &Arrangement) -> core::Arrangement<WindowId> {
    let fill = |window: WindowId| match arrangement.fullscreen.get(&window) {
        None => Fill::Tiled,
        Some(FullscreenMode::Screen) => Fill::Screen,
        Some(FullscreenMode::WithinGaps) => Fill::WithinGaps,
    };
    core::Arrangement::from_listed(
        arrangement
            .columns
            .iter()
            .map(|column| core::Listed {
                id: core::ColumnId(column.id.0),
                width: width(column.width),
                rows: column
                    .rows
                    .iter()
                    .map(|row| core::Row {
                        window: row.window,
                        height: row.height,
                        fill: fill(row.window),
                    })
                    .collect(),
                active_row: column.active_window,
            })
            .collect(),
        arrangement.active_column,
        arrangement.next_column_id,
    )
}

/// The core's arrangement in rift's form: what saving writes, and what a difference re-seeds.
pub(super) fn rift_arrangement(arrangement: &core::Arrangement<WindowId>) -> Arrangement {
    let columns = arrangement.strip.columns();
    Arrangement {
        columns: columns
            .iter()
            .map(|column| Column {
                id: ColumnId(column.id.0),
                rows: column
                    .listed()
                    .into_iter()
                    .map(|row| Row {
                        window: row.window,
                        height: row.height,
                    })
                    .collect(),
                active_window: column.active_index(),
                width: rift_width(column.width),
            })
            .collect(),
        active_column: arrangement.strip.active_index().unwrap_or(0),
        next_column_id: arrangement.next_column,
        fullscreen: columns
            .iter()
            .flat_map(|column| column.listed())
            .filter_map(|row| match row.fill {
                Fill::Tiled => None,
                Fill::Screen => Some((row.window, FullscreenMode::Screen)),
                Fill::WithinGaps => Some((row.window, FullscreenMode::WithinGaps)),
            })
            .collect::<HashMap<_, _>>(),
    }
}

/// The core seeded from rift's system as it is: at a load, and in tests that build a system's
/// state by hand.
pub(super) fn seed(system: &ScrollingLayoutSystem) -> CoreColumns {
    // The slot structure (versions, free list) from the slotmap's saved form; each value from
    // the live state (a saved one is normalized on its way back in).
    let slots = of_slotmap(&system.layouts, LayoutState::default(), |_| ());
    let layouts = slots
        .map(|layout, ()| {
            Ok::<_, std::convert::Infallible>(arrangement(&system.layouts[layout].arrangement))
        })
        .unwrap_or_else(|never| match never {});
    CoreColumns::from_parts(
        layouts,
        system.inactive.iter().copied().collect::<BTreeSet<_>>(),
        settings(&system.settings),
    )
}

/// Rift's arrangements and windows left out, made the core's.
fn reseed(system: &mut ScrollingLayoutSystem) {
    let core = &system.core;
    for (layout, state) in system.layouts.iter_mut() {
        state.arrangement = rift_arrangement(
            core.arrangement(layout)
                .expect("the core holds every layout rift's system does"),
        );
    }
    system.inactive = core.left_out().iter().copied().collect::<HashSet<_>>();
}

/// The selected column as the layout's last pass laid it out.
pub(super) fn span(system: &ScrollingLayoutSystem, layout: LayoutId) -> Option<core::Span> {
    let state = system.layouts.get(layout)?;
    let g = state.geometry.as_ref()?;
    let column = state.arrangement.columns.get(state.arrangement.active_column)?;
    let laid = g.column(column.id)?;
    Some(core::Span {
        view: g.tiling.size.width,
        gap: g.gaps.inner.horizontal,
        width: laid.width,
    })
}

/// An operation begins: the camera's logged writes are cleared, so only this operation's are
/// made on the core; and whether its result is held to rift's.
pub(super) fn begin(system: &mut ScrollingLayoutSystem) -> Due {
    for state in system.layouts.values_mut() {
        state.camera.logged.clear();
    }
    due()
}

/// One operation's logged writes, per layout, in order.
pub(super) struct Replays(HashMap<LayoutId, VecDeque<Logged>>);

impl Replays {
    /// The camera's answer to the next removal from `layout`: the columns it made active after
    /// it, up to the removal's marker.
    pub(super) fn removal(&mut self, layout: LayoutId) -> Vec<core::ColumnId> {
        let mut activated = Vec::new();
        if let Some(logged) = self.0.get_mut(&layout) {
            while let Some(entry) = logged.pop_front() {
                match entry {
                    Logged::Removed => break,
                    Logged::Activated(id) => activated.push(core::ColumnId(id.0)),
                    Logged::Preserved => {}
                }
            }
        }
        activated
    }
}

/// The operation decided by the core (`core_op`, its answer as rift's; the logged writes at
/// hand for a removal's), the writes it did not take made after it; held to rift's (`rift`,
/// the arrangements rift's side left) when `due`. The core's answer.
pub(super) fn decide<A: PartialEq + Debug>(
    due: Due,
    what: &str,
    system: &mut ScrollingLayoutSystem,
    rift: A,
    core_op: impl FnOnce(&mut Decider, &mut Replays) -> A,
) -> A {
    decide_pass(due, what, system, rift, core_op, |_, _| {})
}

/// [`decide`] for an operation that may run a layout pass: `preserved` makes a logged pass's
/// preserved widths on the core, with the pass's facts.
pub(super) fn decide_pass<A: PartialEq + Debug>(
    due: Due,
    what: &str,
    system: &mut ScrollingLayoutSystem,
    rift: A,
    core_op: impl FnOnce(&mut Decider, &mut Replays) -> A,
    preserved: impl Fn(&mut Decider, LayoutId),
) -> A {
    let mut replays = Replays(
        system
            .layouts
            .iter_mut()
            .map(|(layout, state)| (layout, state.camera.logged.drain(..).collect()))
            .collect(),
    );
    let answer = core_op(&mut system.core, &mut replays);
    for (layout, logged) in replays.0 {
        for entry in logged {
            match entry {
                Logged::Activated(id) => {
                    system.core.activate_column(layout, core::ColumnId(id.0));
                }
                Logged::Preserved => preserved(&mut system.core, layout),
                Logged::Removed => {}
            }
        }
    }
    if due == Due::Compared {
        RUNS.fetch_add(1, Ordering::Relaxed);
        let shadow = seed(system);
        if answer != rift {
            report(what, &format!("answer {answer:?}, rift's {rift:?}"));
            reseed(system);
        } else if format!("{:?}", system.core.layouts()) != format!("{:?}", shadow.layouts())
            || system.core.left_out() != shadow.left_out()
        {
            // Compared as written, bit for bit: a height or width that is not a number is the same
            // on both sides, which `==` would say it is not.
            report(
                what,
                &format!(
                    "arrangements {:?}, rift's {:?}",
                    system.core.layouts(),
                    shadow.layouts()
                ),
            );
            reseed(system);
        }
    }
    answer
}

/// A query answered by the core, held to rift's answer when due.
pub(super) fn decide_query<A: PartialEq + Debug>(
    system: &ScrollingLayoutSystem,
    what: &str,
    rift: A,
    query: impl FnOnce(&Decider) -> A,
) -> A {
    let answer = query(&system.core);
    if due() == Due::Compared {
        RUNS.fetch_add(1, Ordering::Relaxed);
        if answer != rift {
            report(what, &format!("answer {answer:?}, rift's {rift:?}"));
        }
    }
    answer
}

#[cfg(test)]
thread_local! {
    /// This thread's divergences while a test makes one on purpose ([`tolerating`]).
    static TOLERATED: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// Runs `f` with this thread's divergences counted instead of failing the test (a test that
/// makes rift's arrangement differ on purpose); how many there were.
#[cfg(test)]
pub(super) fn tolerating(f: impl FnOnce()) -> u64 {
    TOLERATED.with(|t| t.set(Some(0)));
    f();
    TOLERATED.with(|t| t.take()).expect("set above")
}

fn report(what: &str, difference: &str) {
    let divergences = DIVERGENCES.fetch_add(1, Ordering::Relaxed) + 1;
    #[cfg(test)]
    {
        let tolerated = TOLERATED.with(|t| t.get().map(|n| t.set(Some(n + 1))).is_some());
        assert!(
            tolerated,
            "layout-divergence ({divergences} so far): scrolling {what}: core {difference}"
        );
    }
    #[cfg(not(test))]
    tracing::warn!(
        what,
        divergences,
        difference,
        "layout-divergence: rift's scrolling arrangement differs from the core's; the core's \
         stands, rift's is re-seeded from it"
    );
}

/// A window change as the core takes it, the frame resize's facts read from rift's change, and
/// the camera's answer to each removal from the log, made active before the next removal.
pub(super) fn change(core: &mut Decider, change: &WindowChange<'_>, replays: &mut Replays) {
    match *change {
        WindowChange::Add(layout, window, _) => {
            core.insert(layout, window);
        }
        WindowChange::Remove(window) | WindowChange::RemoveAndRebalance(window) => {
            for layout in core.forget_window(window) {
                removed(core, layout, window, replays);
            }
        }
        WindowChange::Replace(from, to) => core.replace_window(from, to),
        WindowChange::CloseApp(pid) => {
            for (layout, windows) in core.forget_app(|window| window.pid == pid) {
                for window in windows {
                    removed(core, layout, window, replays);
                }
            }
        }
        WindowChange::Resize {
            layout,
            window,
            old,
            new,
            screen,
            gaps,
        } => {
            core.resize_from_frame(layout, window, frame_resize(old, new, screen, gaps));
        }
    }
}

/// `window` removed from `layout`, then the camera's answer to that removal made active.
fn removed(core: &mut Decider, layout: LayoutId, window: WindowId, replays: &mut Replays) {
    core.remove_from(layout, window);
    for id in replays.removal(layout) {
        core.activate_column(layout, id);
    }
}

fn rect(frame: objc2_core_foundation::CGRect) -> Rect {
    Rect {
        x: frame.origin.x,
        y: frame.origin.y,
        w: frame.size.width,
        h: frame.size.height,
    }
}

fn frame_resize(
    old: objc2_core_foundation::CGRect,
    new: objc2_core_foundation::CGRect,
    screen: objc2_core_foundation::CGRect,
    gaps: &GapSettings,
) -> core::FrameResize {
    core::FrameResize {
        old: rect(old),
        new: rect(new),
        tiling: rect(compute_tiling_area(screen, gaps)),
        horizontal_gap: gaps.inner.horizontal,
        vertical_gap: gaps.inner.vertical,
    }
}

fn changed(changed: bool) -> EditResult { EditResult::from(changed) }

fn edited(edited: core::Edited<WindowId>) -> EditResult {
    match edited {
        core::Edited::Unsupported => EditResult::Unsupported,
        core::Edited::Unchanged => EditResult::Unchanged,
        core::Edited::Changed => EditResult::Changed,
        core::Edited::Raise(windows) => EditResult::Raise(windows),
    }
}

/// A drop's action as the core takes it; none for consume-or-expel, which a drop never brings
/// here (the engine makes it an edit first).
fn drop_action(action: WindowDropAction) -> Option<core::DropAction> {
    Some(match action {
        WindowDropAction::Swap => core::DropAction::Swap,
        WindowDropAction::Move(direction) => core::DropAction::Move(core_direction(direction)),
        WindowDropAction::Stack => core::DropAction::Transfer(core::Transfer::Stack),
        WindowDropAction::Insert(direction) => {
            core::DropAction::Transfer(core::Transfer::Insert(core_direction(direction)))
        }
        WindowDropAction::ConsumeOrExpel(_) => return None,
    })
}

/// An edit as the core makes it, its answer as rift's; `span` the selected column as laid out
/// before the edit.
pub(super) fn edit(
    core: &mut Decider,
    layout: LayoutId,
    edit: LayoutEdit,
    span: Option<core::Span>,
) -> EditResult {
    match edit {
        LayoutEdit::Move { direction, .. } => {
            changed(core.move_selection(layout, core_direction(direction)).is_some())
        }
        LayoutEdit::Fullscreen { within_gaps } => {
            EditResult::Raise(match core.toggle_fullscreen(layout, within_gaps) {
                core::Fullscreened::Nothing => Vec::new(),
                core::Fullscreened::Entered(window) | core::Fullscreened::Left(window) => {
                    vec![window]
                }
            })
        }
        LayoutEdit::Swap(a, b) => changed(core.swap_windows(layout, a, b)),
        LayoutEdit::Drop { source, target, action } => match drop_action(action) {
            Some(action) => changed(core.drop_window(layout, source, target, action)),
            None => EditResult::Unsupported,
        },
        LayoutEdit::Group(GroupEdit::Join(direction)) => {
            changed(core.join_selection(layout, core_direction(direction)))
        }
        LayoutEdit::Group(GroupEdit::ConsumeOrExpel(direction)) => {
            edited(core.consume_or_expel(layout, core_direction(direction)))
        }
        LayoutEdit::Group(GroupEdit::Stack(_)) => EditResult::Raise(core.stack(layout)),
        LayoutEdit::Group(GroupEdit::Unstack(_)) => EditResult::Raise(core.unstack(layout)),
        LayoutEdit::Group(GroupEdit::Unjoin) => changed(core.unjoin(layout)),
        LayoutEdit::Resize { amount, orientation } => edited(core.resize_selection(
            layout,
            amount,
            match orientation {
                ResizeOrientation::Horizontal => core::ResizeAlong::Horizontal,
                ResizeOrientation::Vertical => core::ResizeAlong::Vertical,
                ResizeOrientation::Smart => core::ResizeAlong::Smart,
            },
            span,
        )),
        _ => EditResult::Unsupported,
    }
}

/// A focus as the core takes it, its answer as rift's.
pub(super) fn focus(
    core: &mut Decider,
    layout: LayoutId,
    focus: crate::layout_engine::systems::Focus,
) -> FocusResult {
    use crate::layout_engine::systems::Focus as Rift;
    let focus = match focus {
        Rift::Select(window) => core::Focus::Select(window),
        Rift::Ascend => core::Focus::Ascend,
        Rift::Descend => core::Focus::Descend,
        Rift::Direction(direction) => core::Focus::Direction(core_direction(direction)),
    };
    match core.focus(layout, focus) {
        core::Focused::Selection { handled } => FocusResult::selection(handled),
        core::Focused::Movement { window, raise } => FocusResult::movement((window, raise)),
    }
}

/// A window query as the core lists it.
pub(super) fn query(query: WindowQuery) -> core::Query<WindowId> {
    match query {
        WindowQuery::All => core::Query::All,
        WindowQuery::Visible => core::Query::Visible,
        WindowQuery::Selection => core::Query::Selection,
        WindowQuery::Stack(window) => core::Query::Stack(window),
    }
}

/// A layout pass's preserved widths as the core fixes them: the tiling area's width and the
/// gap between columns, and each window's locked width.
pub(super) fn preserve(
    core: &mut Decider,
    layout: LayoutId,
    screen: objc2_core_foundation::CGRect,
    gaps: &GapSettings,
    locked: impl Fn(WindowId) -> Option<f64>,
) {
    let tiling = compute_tiling_area(screen, gaps);
    core.preserve_window_sizes(layout, tiling.size.width, gaps.inner.horizontal, locked);
}
