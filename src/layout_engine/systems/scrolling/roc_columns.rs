//! Under `roc-core`, the Roc columns (`RocColumns`, `Columns.roc`) decide (scrolling slice 4, as
//! floating's step 4): every operation and query the scrolling system makes of its arrangements
//! goes to [`ColumnsDecider`], whose answers are the Roc system's. The Rust core takes every
//! operation too, as the Roc system's shadow: its answers are held to the Roc system's on every
//! call, and the whole system on every operation in tests and debug builds and every
//! [`VIEW_EVERY`]th in a `roc-core` release build. A difference is counted
//! ([`columns_roc_reseeds`]), fails a test, or in a running Rift warns `roc-divergence:`; an
//! operation's re-seeds the Rust core from the Roc system's view (a query's is only reported:
//! the next operation compares the systems). An answer of another kind than the operation's, or
//! naming a window rift never gave the Roc system, reads as nothing rift holds: the Rust answer
//! stands (after the report).
//!
//! The decider's methods are the Rust core's, name for name, so the scrolling system calls the
//! same methods in either build; without `roc-core` the decider is the Rust core itself.

use std::cell::RefCell;
use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};

use rift_core::layout::columns::{
    Arrangement, CameraMemory, Column, ColumnId, Direction, DropAction, Edited, Focus, Focused,
    FrameResize, Fullscreened, Moved, Participation, Query, Removal, ResizeAlong, Row, Settings,
    Span, Strip,
};
use rift_core::slot_table::SlotTable;
use rift_core_roc::RocColumns;
use rift_core_roc::columns::{Answer, ColumnsView, Event, LayoutKey};

use super::projection::CoreColumns;
use crate::actor::app::WindowId;
use crate::layout_engine::LayoutId;
use crate::layout_engine::systems::group_roc::{ON_DIVERGENCE, OnDivergence};
use crate::roc_path::{Word as _, window_of_word};

static RESEEDS: AtomicU64 = AtomicU64::new(0);

/// How often a `roc-core` release build compares the whole systems after an operation.
const VIEW_EVERY: u64 = 32;

/// Whether every operation compares the whole systems: in tests and debug builds.
const VIEW_ALWAYS: bool = cfg!(any(test, debug_assertions));

/// How many times the Rust core's arrangements differed from the deciding Roc system's,
/// process-wide (each one a `roc-divergence:`, the Rust core then re-seeded from the Roc system).
pub(crate) fn columns_roc_reseeds() -> u64 { RESEEDS.load(Ordering::Relaxed) }

fn report_under(policy: OnDivergence, what: &str, difference: &str) {
    let reseeds = RESEEDS.fetch_add(1, Ordering::Relaxed) + 1;
    if policy == OnDivergence::Fail {
        panic!(
            "roc-divergence ({reseeds} so far): the scrolling columns after {what}: {difference}"
        );
    }
    tracing::warn!(
        what,
        reseeds,
        difference,
        "roc-divergence: the Rust core's scrolling arrangements differ from the deciding Roc \
         columns'; re-seeded from them"
    );
}

#[cfg(test)]
thread_local! {
    /// Whether a test makes the Rust core differ on purpose ([`tolerating`]).
    static TOLERATING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Rift's policy, or a warning while a test makes the systems differ on purpose.
fn policy() -> OnDivergence {
    #[cfg(test)]
    if TOLERATING.with(|t| t.get()) {
        return OnDivergence::Warn;
    }
    ON_DIVERGENCE
}

/// Runs `f` with a divergence warned and re-seeded instead of failing the test; how many
/// re-seeds there were (process-wide, so a test reads its own only while no other tolerates).
#[cfg(test)]
fn tolerating(f: impl FnOnce()) -> u64 {
    let before = columns_roc_reseeds();
    TOLERATING.with(|t| t.set(true));
    f();
    TOLERATING.with(|t| t.set(false));
    columns_roc_reseeds() - before
}

fn key(layout: LayoutId) -> LayoutKey { LayoutKey(slotmap::Key::data(&layout).as_ffi()) }

fn layout_id(key: LayoutKey) -> LayoutId { slotmap::KeyData::from_ffi(key.0).into() }

fn word(window: WindowId) -> u64 { window.word() }

fn words(windows: &[WindowId]) -> Vec<u64> { windows.iter().copied().map(word).collect() }

/// The windows Roc words name; none when one names a window rift never gave it.
fn window_ids(words: &[u64]) -> Option<Vec<WindowId>> {
    words.iter().map(|&w| window_of_word(w)).collect()
}

fn arrangement_on<V: Copy, W: Copy>(
    arrangement: &Arrangement<V>,
    window: impl Fn(V) -> W,
) -> Arrangement<W> {
    let row = |r: &Row<V>| Row {
        window: window(r.window),
        height: r.height,
        fill: r.fill,
    };
    let column = |c: &Column<V>| Column {
        id: c.id,
        width: c.width,
        before: c.before.iter().map(row).collect(),
        active: row(&c.active),
        after: c.after.iter().map(row).collect(),
    };
    Arrangement {
        strip: match &arrangement.strip {
            Strip::Empty => Strip::Empty,
            Strip::Columns { before, active, after } => Strip::Columns {
                before: before.iter().map(column).collect(),
                active: column(active),
                after: after.iter().map(column).collect(),
            },
        },
        next_column: arrangement.next_column,
    }
}

/// The Rust core's arrangements as the Roc system's view gives them.
fn view_of(core: &CoreColumns) -> ColumnsView {
    ColumnsView {
        layouts: core
            .layouts()
            .saved()
            .map(|(held, version)| (held.map(|(_, a)| arrangement_on(a, word)), version))
            .collect(),
        layout_free: core.layouts().free().to_vec(),
        left_out: core.left_out().iter().copied().map(word).collect(),
        settings: core.settings().clone(),
    }
}

/// The Rust core holding what a Roc system's view holds (a re-seed).
fn core_of(view: &ColumnsView) -> CoreColumns {
    let window =
        |w: u64| window_of_word(w).expect("the Roc columns hold the windows rift gave them");
    let taking: Vec<u32> = view.layout_free.iter().rev().copied().collect();
    let layouts = SlotTable::<LayoutId, _>::parse(
        view.layouts
            .iter()
            .map(|(held, version)| (held.as_ref().map(|a| arrangement_on(a, window)), *version))
            .collect(),
    )
    .and_then(|table| table.resumed(&taking))
    .expect("the Roc columns' table is a slot table");
    CoreColumns::from_parts(
        layouts,
        view.left_out.iter().map(|&w| window(w)).collect(),
        view.settings.clone(),
    )
}

/// The scrolling system's arrangements: the deciding Roc columns, and the Rust core their
/// shadow. Queries step the Roc system too (`&self` in the layout system's interface), so it
/// sits in a cell; nothing else reaches it.
pub(in crate::layout_engine::systems) struct ColumnsDecider {
    rust: CoreColumns,
    roc: RefCell<RocColumns>,
    steps: u64,
}

impl Debug for ColumnsDecider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColumnsDecider")
            .field("rust", &self.rust)
            .finish_non_exhaustive()
    }
}

impl Clone for ColumnsDecider {
    fn clone(&self) -> Self { ColumnsDecider::seeded(self.rust.clone()) }
}

impl ColumnsDecider {
    /// The Roc system holding what `rust` holds, `rust` its shadow.
    pub(super) fn seeded(rust: CoreColumns) -> Self {
        ColumnsDecider {
            roc: RefCell::new(RocColumns::seeded(&view_of(&rust))),
            rust,
            steps: 0,
        }
    }

    pub(super) fn new(settings: Settings) -> Self { Self::seeded(CoreColumns::new(settings)) }

    /// An operation: the Roc system takes `event` and decides; the Rust core took it too
    /// (answering `rust`). A different answer, or a different whole system when one is compared,
    /// re-seeds the Rust core from the Roc system's view.
    fn decided(&mut self, policy: OnDivergence, what: &str, event: Event, rust: Answer) -> Answer {
        self.steps += 1;
        let roc = self.roc.get_mut();
        let answer = roc.step(event);
        let reseed = if answer != rust {
            report_under(policy, what, &format!("answers: rust {rust:?}, roc {answer:?}"));
            true
        } else if VIEW_ALWAYS || self.steps.is_multiple_of(VIEW_EVERY) {
            let (held, rust_view) = (roc.view(), view_of(&self.rust));
            // Compared as written, bit for bit: a ratio or height that is not a number is the
            // same on both sides, which `!=` would say it is not.
            let differs = format!("{held:?}") != format!("{rust_view:?}");
            if differs {
                report_under(policy, what, &format!("views: rust {rust_view:?}, roc {held:?}"));
            }
            differs
        } else {
            false
        };
        if reseed {
            self.rust = core_of(&roc.view());
        }
        answer
    }

    fn operation(&mut self, what: &str, event: Event, rust: Answer) -> Answer {
        self.decided(policy(), what, event, rust)
    }

    /// A query: the Roc system answers; the Rust core's answer (`rust`) is held to it.
    fn asked(&self, what: &str, event: Event, rust: Answer) -> Answer {
        let answer = self.roc.borrow_mut().step(event);
        if answer != rust {
            report_under(
                policy(),
                what,
                &format!("answers: rust {rust:?}, roc {answer:?}"),
            );
        }
        answer
    }

    // What the scrolling system reads of the arrangements (the Rust core's, held to the Roc
    // system's).

    pub(super) fn layouts(&self) -> &SlotTable<LayoutId, Arrangement<WindowId>> {
        self.rust.layouts()
    }

    pub(super) fn left_out(&self) -> &std::collections::BTreeSet<WindowId> { self.rust.left_out() }

    pub(super) fn arrangement(&self, layout: LayoutId) -> Option<&Arrangement<WindowId>> {
        self.rust.arrangement(layout)
    }

    // The operations, name for name the Rust core's.

    pub(super) fn set_settings(&mut self, settings: Settings) {
        self.rust.set_settings(settings.clone());
        self.operation("settings", Event::Settings(settings), Answer::Done);
    }

    pub(super) fn create_layout(&mut self) -> LayoutId {
        let rust = self.rust.create_layout();
        match self.operation("create_layout", Event::CreateLayout, Answer::Layout(key(rust))) {
            Answer::Layout(roc) => layout_id(roc),
            _ => rust,
        }
    }

    pub(super) fn clone_layout(&mut self, layout: LayoutId) -> LayoutId {
        let rust = self.rust.clone_layout(layout);
        match self.operation(
            "clone_layout",
            Event::CloneLayout(key(layout)),
            Answer::Layout(key(rust)),
        ) {
            Answer::Layout(roc) => layout_id(roc),
            _ => rust,
        }
    }

    pub(super) fn remove_layout(&mut self, layout: LayoutId) {
        self.rust.remove_layout(layout);
        self.operation("remove_layout", Event::RemoveLayout(key(layout)), Answer::Done);
    }

    pub(super) fn set_window_participation(
        &mut self,
        window: WindowId,
        participation: Participation,
        cameras: &[(LayoutId, CameraMemory)],
    ) -> bool {
        let rust = self.rust.set_window_participation(window, participation, cameras);
        let event = Event::Participation(
            word(window),
            participation,
            cameras.iter().map(|&(layout, memory)| (key(layout), memory)).collect(),
        );
        verdict(
            self.operation("participation", event, Answer::Verdict(rust)),
            rust,
        )
    }

    pub(super) fn insert(&mut self, layout: LayoutId, window: WindowId) -> bool {
        let rust = self.rust.insert(layout, window);
        let event = Event::Insert(key(layout), word(window));
        verdict(self.operation("insert", event, Answer::Verdict(rust)), rust)
    }

    pub(super) fn forget_window(&mut self, window: WindowId) -> Vec<LayoutId> {
        let rust = self.rust.forget_window(window);
        let answer = Answer::Layouts(rust.iter().copied().map(key).collect());
        match self.operation("forget_window", Event::ForgetWindow(word(window)), answer) {
            Answer::Layouts(roc) => roc.into_iter().map(layout_id).collect(),
            _ => rust,
        }
    }

    pub(super) fn remove_from(
        &mut self,
        layout: LayoutId,
        window: WindowId,
        remembered: Option<ColumnId>,
    ) -> Option<Removal> {
        let rust = self.rust.remove_from(layout, window, remembered);
        let event = Event::RemoveFrom(key(layout), word(window), remembered);
        match self.operation("remove_from", event, Answer::Removal(rust)) {
            Answer::Removal(roc) => roc,
            _ => rust,
        }
    }

    pub(super) fn leave_out_dormant(&mut self, windows: Vec<WindowId>) {
        self.rust.leave_out_dormant(windows.iter().copied());
        self.operation(
            "leave_out_dormant",
            Event::LeaveOutDormant(words(&windows)),
            Answer::Done,
        );
    }

    pub(super) fn replace_window(&mut self, from: WindowId, to: WindowId) {
        self.rust.replace_window(from, to);
        let event = Event::ReplaceWindow { from: word(from), to: word(to) };
        self.operation("replace_window", event, Answer::Done);
    }

    /// The app's windows are the ones the Rust core holds (`is_app` over them), so the Roc
    /// system is told them as the differential tells them: listed.
    pub(super) fn forget_app(
        &mut self,
        is_app: impl Fn(WindowId) -> bool,
    ) -> Vec<(LayoutId, Vec<WindowId>)> {
        let app: Vec<u64> = self
            .rust
            .layouts()
            .values()
            .flat_map(|a| a.windows().collect::<Vec<_>>())
            .chain(self.rust.left_out().iter().copied())
            .filter(|&w| is_app(w))
            .map(word)
            .collect();
        let rust = self.rust.forget_app(is_app);
        let answer = Answer::Closing(rust.iter().map(|(l, ws)| (key(*l), words(ws))).collect());
        match self.operation("forget_app", Event::ForgetApp(app), answer) {
            Answer::Closing(roc) => roc
                .into_iter()
                .map(|(l, ws)| window_ids(&ws).map(|ws| (layout_id(l), ws)))
                .collect::<Option<Vec<_>>>()
                .unwrap_or(rust),
            _ => rust,
        }
    }

    pub(super) fn resize_from_frame(
        &mut self,
        layout: LayoutId,
        window: WindowId,
        resize: FrameResize,
    ) -> bool {
        let rust = self.rust.resize_from_frame(layout, window, resize);
        let event = Event::ResizeFromFrame(key(layout), word(window), resize);
        verdict(
            self.operation("resize_from_frame", event, Answer::Verdict(rust)),
            rust,
        )
    }

    pub(super) fn activate_column(&mut self, layout: LayoutId, id: ColumnId) -> bool {
        let rust = self.rust.activate_column(layout, id);
        let event = Event::ActivateColumn(key(layout), id);
        verdict(
            self.operation("activate_column", event, Answer::Verdict(rust)),
            rust,
        )
    }

    pub(super) fn preserve_window_sizes(
        &mut self,
        layout: LayoutId,
        view: f64,
        gap: f64,
        locked: impl Fn(WindowId) -> Option<f64>,
    ) -> Vec<ColumnId> {
        let known: Vec<(u64, f64)> = self
            .rust
            .arrangement(layout)
            .map(|a| a.windows().filter_map(|w| locked(w).map(|l| (word(w), l))).collect())
            .unwrap_or_default();
        let rust = self.rust.preserve_window_sizes(layout, view, gap, locked);
        let event = Event::PreserveWindowSizes {
            layout: key(layout),
            view,
            gap,
            locked: known,
        };
        match self.operation("preserve_window_sizes", event, Answer::Fixed(rust.clone())) {
            Answer::Fixed(roc) => roc,
            _ => rust,
        }
    }

    /// As the Rust core's: a `Focus::Select`, whose answer is whether it was handled.
    pub(super) fn select_window(&mut self, layout: LayoutId, window: WindowId) -> bool {
        let rust = self.rust.select_window(layout, window);
        let event = Event::Focus(key(layout), Focus::Select(word(window)));
        match self.operation(
            "select_window",
            event,
            Answer::Focused(Focused::Selection { handled: rust }),
        ) {
            Answer::Focused(Focused::Selection { handled }) => handled,
            _ => rust,
        }
    }

    pub(super) fn focus(&mut self, layout: LayoutId, focus: Focus<WindowId>) -> Focused<WindowId> {
        let rust = self.rust.focus(layout, focus);
        let event = Event::Focus(key(layout), focus_words(focus));
        match self.operation("focus", event, Answer::Focused(focused_words(&rust))) {
            Answer::Focused(roc) => focused_ids(roc).unwrap_or(rust),
            _ => rust,
        }
    }

    pub(super) fn swap_windows(&mut self, layout: LayoutId, a: WindowId, b: WindowId) -> bool {
        let rust = self.rust.swap_windows(layout, a, b);
        let event = Event::Swap(key(layout), word(a), word(b));
        verdict(self.operation("swap", event, Answer::Verdict(rust)), rust)
    }

    pub(super) fn move_selection(
        &mut self,
        layout: LayoutId,
        direction: Direction,
    ) -> Option<Moved> {
        let rust = self.rust.move_selection(layout, direction);
        let event = Event::MoveSelection(key(layout), direction);
        match self.operation("move_selection", event, Answer::Moved(rust)) {
            Answer::Moved(roc) => roc,
            _ => rust,
        }
    }

    pub(super) fn drop_window(
        &mut self,
        layout: LayoutId,
        source: WindowId,
        target: WindowId,
        action: DropAction,
    ) -> bool {
        let rust = self.rust.drop_window(layout, source, target, action);
        let event = Event::Drop {
            layout: key(layout),
            source: word(source),
            target: word(target),
            action,
        };
        verdict(self.operation("drop", event, Answer::Verdict(rust)), rust)
    }

    pub(super) fn join_selection(&mut self, layout: LayoutId, direction: Direction) -> bool {
        let rust = self.rust.join_selection(layout, direction);
        let event = Event::Join(key(layout), direction);
        verdict(self.operation("join", event, Answer::Verdict(rust)), rust)
    }

    pub(super) fn consume_or_expel(
        &mut self,
        layout: LayoutId,
        direction: Direction,
    ) -> Edited<WindowId> {
        let rust = self.rust.consume_or_expel(layout, direction);
        let event = Event::ConsumeOrExpel(key(layout), direction);
        edited(
            self.operation("consume_or_expel", event, Answer::Edited(edited_words(&rust))),
            rust,
        )
    }

    pub(super) fn stack(&mut self, layout: LayoutId) -> Vec<WindowId> {
        let rust = self.rust.stack(layout);
        windows(
            self.operation("stack", Event::Stack(key(layout)), Answer::Windows(words(&rust))),
            rust,
        )
    }

    pub(super) fn unstack(&mut self, layout: LayoutId) -> Vec<WindowId> {
        let rust = self.rust.unstack(layout);
        let answer = Answer::Windows(words(&rust));
        windows(
            self.operation("unstack", Event::Unstack(key(layout)), answer),
            rust,
        )
    }

    pub(super) fn unjoin(&mut self, layout: LayoutId) -> bool {
        let rust = self.rust.unjoin(layout);
        verdict(
            self.operation("unjoin", Event::Unjoin(key(layout)), Answer::Verdict(rust)),
            rust,
        )
    }

    pub(super) fn resize_selection(
        &mut self,
        layout: LayoutId,
        amount: f64,
        along: ResizeAlong,
        span: Option<Span>,
    ) -> Edited<WindowId> {
        let rust = self.rust.resize_selection(layout, amount, along, span);
        let event = Event::Resize {
            layout: key(layout),
            amount,
            along,
            span,
        };
        edited(
            self.operation("resize", event, Answer::Edited(edited_words(&rust))),
            rust,
        )
    }

    pub(super) fn switch_preset_column_width(
        &mut self,
        layout: LayoutId,
        backwards: bool,
        span: Option<Span>,
    ) -> bool {
        let rust = self.rust.switch_preset_column_width(layout, backwards, span);
        let event = Event::SwitchPreset {
            layout: key(layout),
            backwards,
            span,
        };
        verdict(
            self.operation("switch_preset", event, Answer::Verdict(rust)),
            rust,
        )
    }

    pub(super) fn toggle_fullscreen(
        &mut self,
        layout: LayoutId,
        within_gaps: bool,
    ) -> Fullscreened<WindowId> {
        let rust = self.rust.toggle_fullscreen(layout, within_gaps);
        let event = Event::ToggleFullscreen(key(layout), within_gaps);
        let answer = Answer::Fullscreened(match rust {
            Fullscreened::Nothing => Fullscreened::Nothing,
            Fullscreened::Entered(w) => Fullscreened::Entered(word(w)),
            Fullscreened::Left(w) => Fullscreened::Left(word(w)),
        });
        match self.operation("toggle_fullscreen", event, answer) {
            Answer::Fullscreened(Fullscreened::Nothing) => Fullscreened::Nothing,
            Answer::Fullscreened(Fullscreened::Entered(w)) => {
                window_of_word(w).map_or(rust, Fullscreened::Entered)
            }
            Answer::Fullscreened(Fullscreened::Left(w)) => {
                window_of_word(w).map_or(rust, Fullscreened::Left)
            }
            _ => rust,
        }
    }

    // The queries.

    pub(super) fn contains_layout(&self, layout: LayoutId) -> bool {
        let rust = self.rust.contains_layout(layout);
        verdict(
            self.asked(
                "contains_layout",
                Event::ContainsLayout(key(layout)),
                Answer::Verdict(rust),
            ),
            rust,
        )
    }

    pub(super) fn window_participates(&self, window: WindowId) -> bool {
        let rust = self.rust.window_participates(window);
        let event = Event::WindowParticipates(word(window));
        verdict(
            self.asked("window_participates", event, Answer::Verdict(rust)),
            rust,
        )
    }

    pub(super) fn selected_window(&self, layout: LayoutId) -> Option<WindowId> {
        let rust = self.rust.selected_window(layout);
        let event = Event::SelectedWindow(key(layout));
        window(
            self.asked("selected_window", event, Answer::Window(rust.map(word))),
            rust,
        )
    }

    pub(super) fn window_in_direction(
        &self,
        layout: LayoutId,
        direction: Direction,
    ) -> Option<WindowId> {
        let rust = self.rust.window_in_direction(layout, direction);
        let event = Event::WindowInDirection(key(layout), direction);
        window(
            self.asked("window_in_direction", event, Answer::Window(rust.map(word))),
            rust,
        )
    }

    pub(super) fn contains_window(&self, layout: LayoutId, w: WindowId) -> bool {
        let rust = self.rust.contains_window(layout, w);
        let event = Event::ContainsWindow(key(layout), word(w));
        verdict(self.asked("contains_window", event, Answer::Verdict(rust)), rust)
    }

    pub(super) fn has_any_fullscreen_node(&self, layout: LayoutId) -> bool {
        let rust = self.rust.has_any_fullscreen_node(layout);
        let event = Event::HasAnyFullscreenNode(key(layout));
        verdict(
            self.asked("has_any_fullscreen_node", event, Answer::Verdict(rust)),
            rust,
        )
    }

    pub(super) fn parent_of_selection_is_stacked(&self, layout: LayoutId) -> bool {
        let rust = self.rust.parent_of_selection_is_stacked(layout);
        let event = Event::ParentOfSelectionIsStacked(key(layout));
        verdict(
            self.asked("parent_of_selection_is_stacked", event, Answer::Verdict(rust)),
            rust,
        )
    }

    /// The app's windows are those of the layout `is_app` tells, listed for the Roc system.
    pub(super) fn has_windows_for_app(
        &self,
        layout: LayoutId,
        is_app: impl Fn(WindowId) -> bool,
    ) -> bool {
        let app: Vec<u64> = self
            .rust
            .arrangement(layout)
            .map(|a| a.windows().filter(|&w| is_app(w)).map(word).collect())
            .unwrap_or_default();
        let rust = self.rust.has_windows_for_app(layout, is_app);
        let event = Event::HasWindowsForApp(key(layout), app);
        verdict(
            self.asked("has_windows_for_app", event, Answer::Verdict(rust)),
            rust,
        )
    }

    pub(super) fn windows(&self, layout: LayoutId, query: Query<WindowId>) -> Vec<WindowId> {
        let rust = self.rust.windows(layout, query);
        let query = match query {
            Query::All => Query::All,
            Query::Visible => Query::Visible,
            Query::Selection => Query::Selection,
            Query::Stack(w) => Query::Stack(word(w)),
        };
        let event = Event::Windows(key(layout), query);
        windows(self.asked("windows", event, Answer::Windows(words(&rust))), rust)
    }

    pub(super) fn window_slot(&self, layout: LayoutId, w: WindowId) -> Option<Vec<usize>> {
        let rust = self.rust.window_slot(layout, w);
        let event = Event::WindowSlot(key(layout), word(w));
        match self.asked("window_slot", event, Answer::Slot(rust.clone())) {
            Answer::Slot(roc) => roc,
            _ => rust,
        }
    }

    pub(super) fn can_move_window(
        &self,
        layout: LayoutId,
        w: WindowId,
        direction: Direction,
    ) -> bool {
        let rust = self.rust.can_move_window(layout, w, direction);
        let event = Event::CanMoveWindow(key(layout), word(w), direction);
        verdict(self.asked("can_move_window", event, Answer::Verdict(rust)), rust)
    }
}

// The Roc system's answers read back as rift's values; an answer of another kind, or naming a
// window rift never gave it, is the Rust answer (already reported: the answers differed).

fn verdict(roc: Answer, rust: bool) -> bool {
    match roc {
        Answer::Verdict(roc) => roc,
        _ => rust,
    }
}

fn window(roc: Answer, rust: Option<WindowId>) -> Option<WindowId> {
    match roc {
        Answer::Window(None) => None,
        Answer::Window(Some(w)) => window_of_word(w).map_or(rust, Some),
        _ => rust,
    }
}

fn windows(roc: Answer, rust: Vec<WindowId>) -> Vec<WindowId> {
    match roc {
        Answer::Windows(roc) => window_ids(&roc).unwrap_or(rust),
        _ => rust,
    }
}

fn edited_words(edited: &Edited<WindowId>) -> Edited<u64> {
    match edited {
        Edited::Unsupported => Edited::Unsupported,
        Edited::Unchanged => Edited::Unchanged,
        Edited::Changed => Edited::Changed,
        Edited::Raise(ws) => Edited::Raise(words(ws)),
    }
}

fn edited(roc: Answer, rust: Edited<WindowId>) -> Edited<WindowId> {
    match roc {
        Answer::Edited(Edited::Unsupported) => Edited::Unsupported,
        Answer::Edited(Edited::Unchanged) => Edited::Unchanged,
        Answer::Edited(Edited::Changed) => Edited::Changed,
        Answer::Edited(Edited::Raise(ws)) => window_ids(&ws).map_or(rust, Edited::Raise),
        _ => rust,
    }
}

fn focus_words(focus: Focus<WindowId>) -> Focus<u64> {
    match focus {
        Focus::Select(w) => Focus::Select(word(w)),
        Focus::Ascend => Focus::Ascend,
        Focus::Descend => Focus::Descend,
        Focus::Direction(d) => Focus::Direction(d),
    }
}

fn focused_words(focused: &Focused<WindowId>) -> Focused<u64> {
    match focused {
        Focused::Selection { handled } => Focused::Selection { handled: *handled },
        Focused::Movement { window, raise } => Focused::Movement {
            window: window.map(word),
            raise: words(raise),
        },
    }
}

fn focused_ids(focused: Focused<u64>) -> Option<Focused<WindowId>> {
    Some(match focused {
        Focused::Selection { handled } => Focused::Selection { handled },
        Focused::Movement { window, raise } => Focused::Movement {
            window: match window {
                None => None,
                Some(w) => Some(window_of_word(w)?),
            },
            raise: window_ids(&raise)?,
        },
    })
}

#[cfg(test)]
mod tests {
    use rift_core::layout::columns::InsertionPoint;

    use super::*;

    fn wid(idx: u32) -> WindowId { WindowId::new(1, idx) }

    /// A decider with one layout of three windows, the first selected.
    fn three() -> (ColumnsDecider, LayoutId) {
        let mut decider = ColumnsDecider::new(Settings {
            insertion: InsertionPoint::NextToSelection,
            column_width_ratio: 0.5,
            min_column_width_ratio: 0.1,
            max_column_width_ratio: 1.0,
            preset_column_widths: vec![0.5],
            preserve_window_sizes: false,
        });
        let layout = decider.create_layout();
        for idx in 1..=3 {
            decider.insert(layout, wid(idx));
        }
        assert!(decider.select_window(layout, wid(1)));
        (decider, layout)
    }

    /// Under `roc-core` the Roc columns decide: an operation's answer and a query's are theirs,
    /// not the Rust core's.
    #[test]
    fn the_roc_columns_answer() {
        let (mut decider, layout) = three();
        decider.rust.set_window_participation(wid(2), Participation::LeftOut, &[]);
        let mut changed = false;
        tolerating(|| {
            changed = decider.set_window_participation(wid(2), Participation::LeftOut, &[])
        });
        assert!(changed, "window 2 took part in the Roc columns");
        decider.rust.select_window(layout, wid(3));
        let mut selected = None;
        tolerating(|| selected = decider.selected_window(layout));
        assert_eq!(selected, Some(wid(1)));
    }

    /// An app closing forgets its windows left out of every layout too, in both systems (a
    /// difference fails the test).
    #[test]
    fn an_app_closing_forgets_its_windows_left_out_of_every_layout() {
        let (mut decider, _) = three();
        decider.set_window_participation(wid(9), Participation::LeftOut, &[]);
        decider.forget_app(|window| window.pid == 1);
        assert!(decider.window_participates(wid(9)));
    }

    /// A Rust core that differs is re-seeded from the Roc columns' view.
    #[test]
    fn a_differing_rust_core_is_reseeded_from_the_roc_columns() {
        let (mut decider, layout) = three();
        decider.rust.select_window(layout, wid(3));
        let reseeds = tolerating(|| {
            decider.insert(layout, wid(4));
        });
        assert!(reseeds >= 1);
        assert_eq!(
            format!("{:?}", view_of(&decider.rust)),
            format!("{:?}", decider.roc.get_mut().view())
        );
        assert_eq!(
            decider.rust.arrangement(layout).unwrap().windows().collect::<Vec<_>>(),
            vec![wid(1), wid(4), wid(2), wid(3)]
        );
    }
}
