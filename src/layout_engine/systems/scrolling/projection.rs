//! The scrolling system's arrangements are the core's (scrolling slices 3 to 5): the system
//! holds a [`Decider`] (the Rust core, or under `roc-core` the Roc columns deciding with the
//! Rust core their shadow) and every operation and query goes to it. The camera reads each
//! layout's arrangement as a projection in rift's shape ([`rift_arrangement`]), refreshed from
//! the core after every change and never stepped on its own; saving writes it, so the bytes are
//! rift's saved form of the core's arrangement. This module converts between the two shapes,
//! seeds the core at a load, and reads a layout pass's facts the core's operations take.

use std::collections::BTreeSet;

use rift_core::layout::columns::{self as core, Fill};
use rift_core::plain::Rect;
use slotmap::SlotMap;

use super::{
    ArrangementView, Column, ColumnId, ColumnWidth, FullscreenMode, LoadedLayout, Row,
    ScrollingLayoutSettings, ScrollingLayoutSystem,
};
use crate::actor::app::WindowId;
use crate::common::collections::HashMap;
use crate::common::config::{GapSettings, WindowInsertionPoint};
use crate::layout_engine::LayoutId;
use crate::layout_engine::systems::{EditResult, WindowQuery};
use crate::layout_engine::utils::compute_tiling_area;
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

/// The Rust core the decider holds (under `roc-core`, the Roc columns' shadow, equal to them).
pub(super) fn core_of(decider: &Decider) -> &CoreColumns {
    #[cfg(feature = "roc-core")]
    return decider.rust();
    #[cfg(not(feature = "roc-core"))]
    decider
}

/// The decider holding what `core` holds.
pub(super) fn decider(core: CoreColumns) -> Decider {
    #[cfg(feature = "roc-core")]
    return super::roc_columns::ColumnsDecider::seeded(core);
    #[cfg(not(feature = "roc-core"))]
    core
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
pub(super) fn arrangement(arrangement: &ArrangementView) -> core::Arrangement<WindowId> {
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

/// The core's arrangement in rift's shape: what the camera reads and saving writes, computed at
/// each use.
pub(super) fn view(arrangement: &core::Arrangement<WindowId>) -> ArrangementView {
    let columns = arrangement.strip.columns();
    ArrangementView {
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

/// The core seeded from a loaded system's layouts (each read as rift reads its saved form): the
/// slot structure (versions, free list) from the slotmap's saved form, each arrangement as the
/// core parses rift's; nothing left out, the default settings (both set after a load).
pub(super) fn seed(layouts: &SlotMap<LayoutId, LoadedLayout>) -> CoreColumns {
    let table = of_slotmap(layouts, LoadedLayout::default(), |loaded| {
        arrangement(&loaded.view)
    });
    CoreColumns::from_parts(
        table,
        BTreeSet::new(),
        settings(&ScrollingLayoutSettings::default()),
    )
}

/// The selected column as the layout's last pass laid it out.
pub(super) fn span(system: &ScrollingLayoutSystem, layout: LayoutId) -> Option<core::Span> {
    let state = system.layouts.get(layout)?;
    let g = state.geometry.as_ref()?;
    let arrangement = system.core.arrangement(layout)?;
    let core::Strip::Columns { active, .. } = &arrangement.strip else {
        return None;
    };
    let column = g.column(ColumnId(active.id.0))?;
    Some(core::Span {
        view: g.tiling.size.width,
        gap: g.gaps.inner.horizontal,
        width: column.width,
    })
}

fn rect(frame: objc2_core_foundation::CGRect) -> Rect {
    Rect {
        x: frame.origin.x,
        y: frame.origin.y,
        w: frame.size.width,
        h: frame.size.height,
    }
}

pub(super) fn frame_resize(
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

pub(super) fn edit_result(edited: core::Edited<WindowId>) -> EditResult {
    match edited {
        core::Edited::Unsupported => EditResult::Unsupported,
        core::Edited::Unchanged => EditResult::Unchanged,
        core::Edited::Changed => EditResult::Changed,
        core::Edited::Raise(windows) => EditResult::Raise(windows),
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
