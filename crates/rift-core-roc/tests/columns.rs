//! A scrolling layout's arrangements, kept between operations: the Roc twin (`RocColumns`)
//! against the Rust system (`rift_core::layout::columns::ColumnSystem`), on drawn runs of
//! operations. Runs reuse a few windows, the layouts the run has made (removed ones too: every
//! operation takes a key it may not hold) and the columns' ids (a column made active, the camera's
//! answer to a removal among them, names live columns and gone ones). A removal is drawn as its
//! steps (a window or an app forgotten, a window removed from a layout), so every interleaving
//! with other operations is reached. Every operation must answer alike and
//! leave the same whole system, compared as written (a height that is not a number is the same
//! on both sides): the table's slots, versions and free list, every strip, the windows left
//! out, the settings. Reloads seed both from the same system, with states only a saved file
//! holds planted (heights and widths that are zero, negative or not a number, fills).

use rift_core::layout::columns::{
    ColumnId, Direction, DropAction, Fill, Focus, FrameResize, InsertionPoint, Participation,
    Query, ResizeAlong, Settings, Span, Strip, Transfer, Width,
};
use rift_core::plain::Rect;
use rift_core_roc::columns::{self, Answer, Columns, ColumnsView, Event, LayoutKey};
use rift_core_roc::{AllocationCount, RocColumns};

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);

/// A few windows of three apps (a pid in the high half, an index in the low one).
const WINDOWS: [u64; 8] = [
    (1 << 32) | 1,
    (1 << 32) | 2,
    (1 << 32) | 3,
    (2 << 32) | 1,
    (2 << 32) | 2,
    (2 << 32) | 3,
    (3 << 32) | 1,
    (3 << 32) | 2,
];

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 { self.next() % n }

    fn chance(&mut self, percent: u64) -> bool { self.below(100) < percent }

    fn pick<T: Copy>(&mut self, items: &[T]) -> Option<T> {
        (!items.is_empty()).then(|| items[self.below(items.len() as u64) as usize])
    }

    fn window(&mut self) -> u64 { WINDOWS[self.below(WINDOWS.len() as u64) as usize] }

    /// A window the layout holds (most of the time), else any.
    fn held(&mut self, held: &[u64]) -> u64 {
        match self.pick(held) {
            Some(window) if self.chance(85) => window,
            _ => self.window(),
        }
    }

    fn length(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * (self.below(1 << 20) as f64 / f64::from(1 << 20))
    }

    fn direction(&mut self) -> Direction {
        [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ][self.below(4) as usize]
    }

    /// A ratio setting: mostly sensible, sometimes out of range or not a number.
    fn ratio(&mut self) -> f64 {
        match self.below(8) {
            0 => f64::NAN,
            1 => self.length(0.0, 0.05),
            2 => self.length(1.0, 1.6),
            _ => self.length(0.1, 1.0),
        }
    }

    fn settings(&mut self) -> Settings {
        let presets = (0..self.below(4))
            .map(|_| match self.below(6) {
                0 => f64::NAN,
                1 => -0.5,
                _ => [0.33, 0.5, 0.667, 1.0][self.below(4) as usize],
            })
            .collect();
        Settings {
            insertion: if self.chance(50) {
                InsertionPoint::NextToSelection
            } else {
                InsertionPoint::EndOfTree
            },
            column_width_ratio: self.ratio(),
            min_column_width_ratio: self.ratio(),
            max_column_width_ratio: self.ratio(),
            preset_column_widths: presets,
            preserve_window_sizes: self.chance(50),
        }
    }

    fn span(&mut self) -> Option<Span> {
        self.chance(70).then(|| Span {
            view: self.length(300.0, 2000.0),
            gap: self.length(0.0, 30.0),
            width: self.length(1.0, 1500.0),
        })
    }

    fn frame(&mut self) -> Rect {
        Rect {
            x: self.length(0.0, 400.0),
            y: self.length(0.0, 300.0),
            w: self.length(50.0, 1400.0),
            h: self.length(50.0, 900.0),
        }
    }
}

/// What the run has made: its layouts (removed ones kept, to name gone ones too).
#[derive(Default)]
struct Made {
    layouts: Vec<LayoutKey>,
}

impl Made {
    fn answered(&mut self, answer: &Answer) {
        if let Answer::Layout(layout) = answer
            && !self.layouts.contains(layout)
        {
            self.layouts.push(*layout);
        }
    }

    /// Column ids: the system's own, and some no column holds.
    fn columns(rust: &Columns) -> Vec<ColumnId> {
        let mut ids: Vec<ColumnId> = rust
            .layouts()
            .values()
            .flat_map(|a| a.strip.columns().into_iter().map(|c| c.id).collect::<Vec<_>>())
            .collect();
        ids.push(ColumnId(2));
        ids.push(ColumnId(999));
        ids
    }

    fn event(&mut self, rng: &mut Rng, rust: &Columns) -> Event {
        let Some(layout) = rng.pick(&self.layouts).filter(|_| rng.chance(95)) else {
            return Event::CreateLayout;
        };
        let held = rust.windows(layout, Query::All);
        let window = rng.held(&held);
        match rng.below(40) {
            0..=7 => Event::Insert(layout, rng.window()),
            8 => Event::CreateLayout,
            9 => Event::CloneLayout(layout),
            10 => Event::RemoveLayout(layout),
            11 => Event::ForgetWindow(window),
            12 => Event::RemoveFrom(layout, if rng.chance(90) { window } else { rng.window() }),
            13 => {
                let pid = 1 + rng.below(3);
                Event::ForgetApp(WINDOWS.iter().copied().filter(|w| w >> 32 == pid).collect())
            }
            14 => Event::ReplaceWindow { from: window, to: rng.window() },
            15 => Event::Participation(
                window,
                if rng.chance(50) {
                    Participation::TakesPart
                } else {
                    Participation::LeftOut
                },
            ),
            16 => Event::Settings(rng.settings()),
            17 => {
                let old = rng.frame();
                let new = if rng.chance(30) { old } else { rng.frame() };
                Event::ResizeFromFrame(layout, window, FrameResize {
                    old,
                    new,
                    tiling: rng.frame(),
                    horizontal_gap: rng.length(0.0, 30.0),
                    vertical_gap: rng.length(0.0, 30.0),
                })
            }
            18 => {
                let ids = Made::columns(rust);
                Event::ActivateColumn(layout, rng.pick(&ids).unwrap_or(ColumnId(2)))
            }
            19 => Event::PreserveWindowSizes {
                layout,
                view: rng.length(300.0, 2000.0),
                gap: rng.length(0.0, 30.0),
                locked: {
                    let mut locked = Vec::new();
                    for w in WINDOWS {
                        if rng.chance(50) {
                            let width = [rng.length(50.0, 1500.0), -3.0, f64::NAN];
                            locked.push((w, width[rng.below(3) as usize]));
                        }
                    }
                    locked
                },
            },
            20 => match rng.below(12) {
                0 => Event::ContainsLayout(layout),
                1 => Event::WindowParticipates(window),
                2 => Event::SelectedWindow(layout),
                3 => Event::SelectedLocation(layout),
                4 => Event::WindowInDirection(layout, rng.direction()),
                5 => Event::ContainsWindow(layout, window),
                6 => Event::HasAnyFullscreenNode(layout),
                7 => Event::ParentOfSelectionIsStacked(layout),
                8 => Event::HasWindowsForApp(
                    layout,
                    held.iter().copied().filter(|_| rng.chance(50)).collect(),
                ),
                9 => Event::Windows(
                    layout,
                    [
                        Query::All,
                        Query::Visible,
                        Query::Selection,
                        Query::Stack(window),
                    ][rng.below(4) as usize],
                ),
                10 => Event::WindowSlot(layout, window),
                _ => Event::CanMoveWindow(layout, window, rng.direction()),
            },
            21..=23 => Event::Focus(layout, match rng.below(4) {
                0 => Focus::Select(window),
                1 => Focus::Ascend,
                2 => Focus::Descend,
                _ => Focus::Direction(rng.direction()),
            }),
            24 => Event::Swap(layout, window, rng.held(&held)),
            25 | 26 => Event::MoveSelection(layout, rng.direction()),
            27 | 28 => Event::Drop {
                layout,
                source: window,
                target: rng.held(&held),
                action: match rng.below(4) {
                    0 => DropAction::Swap,
                    1 => DropAction::Move(rng.direction()),
                    2 => DropAction::Transfer(Transfer::Stack),
                    _ => DropAction::Transfer(Transfer::Insert(rng.direction())),
                },
            },
            29 => Event::Join(layout, rng.direction()),
            30 | 31 => Event::ConsumeOrExpel(layout, rng.direction()),
            32 => Event::Stack(layout),
            33 => Event::Unstack(layout),
            34 => Event::Unjoin(layout),
            35 | 36 => Event::Resize {
                layout,
                amount: if rng.chance(5) {
                    f64::NAN
                } else {
                    rng.length(-0.6, 0.6)
                },
                along: [
                    ResizeAlong::Horizontal,
                    ResizeAlong::Vertical,
                    ResizeAlong::Smart,
                ][rng.below(3) as usize],
                span: rng.span(),
            },
            37 => Event::SwitchPreset {
                layout,
                backwards: rng.chance(50),
                span: rng.span(),
            },
            _ => Event::ToggleFullscreen(layout, rng.chance(50)),
        }
    }
}

/// A view as written: bit for bit, a height that is not a number the same on both sides.
fn written(view: &ColumnsView) -> String { format!("{view:?}") }

#[test]
fn the_roc_columns_answer_and_keep_the_arrangements_as_the_rust_system_does() {
    let runs: u64 = std::env::var("RIFT_COLUMNS_RUNS").map_or(4_000, |n| n.parse().unwrap());
    let mut rng = Rng(0xC011_0000_0000_0001);
    let before = AllocationCount::now();
    let mut reached = [0u64; 10];
    for run in 0..runs {
        let settings = rng.settings();
        let mut rust = Columns::new(settings.clone());
        let mut roc = RocColumns::new(&settings);
        let mut made = Made::default();
        let mut history = Vec::new();
        for step in 0..60 {
            let event = made.event(&mut rng, &rust);
            history.push(event.clone());
            let expected = columns::step(&mut rust, event.clone());
            let answered = roc.step(event.clone());
            assert_eq!(
                format!("{answered:?}"),
                format!("{expected:?}"),
                "run {run} step {step}: the answer after {history:#?}"
            );
            assert_eq!(
                written(&roc.view()),
                written(&ColumnsView::from(&rust)),
                "run {run} step {step}: the system after {history:#?}"
            );
            made.answered(&expected);
            match (&event, &expected) {
                (Event::RemoveFrom(..), Answer::Removal(Some(_))) => reached[0] += 1,
                (Event::ForgetApp(_), Answer::Closing(c)) if !c.is_empty() => reached[1] += 1,
                (Event::MoveSelection(..), Answer::Moved(Some(_))) => reached[2] += 1,
                (Event::Drop { .. }, Answer::Verdict(true)) => reached[3] += 1,
                (Event::Stack(_) | Event::Unstack(_), Answer::Windows(w)) if !w.is_empty() => {
                    reached[4] += 1
                }
                (Event::SwitchPreset { .. }, Answer::Verdict(true)) => reached[5] += 1,
                (Event::PreserveWindowSizes { .. }, Answer::Fixed(f)) if !f.is_empty() => {
                    reached[6] += 1
                }
                (Event::ResizeFromFrame(..), Answer::Verdict(true)) => reached[7] += 1,
                _ => {}
            }
            if rust.layouts().values().any(|a| {
                a.strip.columns().iter().any(|c| !c.before.is_empty() || !c.after.is_empty())
            }) {
                reached[8] += 1;
            }
            if rng.chance(3) {
                // A reload: both seeded from the same system, with states only a saved file
                // holds planted.
                let mut view = ColumnsView::from(&rust);
                for arrangement in view.layouts.iter_mut().filter_map(|(held, _)| held.as_mut()) {
                    if let Strip::Columns { before, active, after } = &mut arrangement.strip {
                        for column in
                            before.iter_mut().chain(std::iter::once(active)).chain(after.iter_mut())
                        {
                            if rng.chance(20) {
                                column.width = [
                                    Width::Fixed(-5.0),
                                    Width::Proportion(f64::NAN),
                                    Width::Fixed(0.0),
                                ][rng.below(3) as usize];
                            }
                            for row in column
                                .before
                                .iter_mut()
                                .chain(std::iter::once(&mut column.active))
                                .chain(column.after.iter_mut())
                            {
                                if rng.chance(15) {
                                    row.height = [0.0, -2.0, f64::NAN, 3.5][rng.below(4) as usize];
                                }
                                if rng.chance(10) {
                                    row.fill = Fill::WithinGaps;
                                }
                            }
                        }
                    }
                }
                rust = view.columns().expect("a Rust system's view parses");
                roc = RocColumns::seeded(&view);
                reached[9] += 1;
                assert_eq!(
                    written(&roc.view()),
                    written(&view),
                    "run {run} step {step}: the seed"
                );
            }
        }
    }
    assert_eq!(
        AllocationCount::now().live(),
        before.live(),
        "Roc allocations left live"
    );
    // Each kind is reached more than 50 times in 4000 runs, proportionally in fewer.
    let needed = (50 * runs / 4000).max(1);
    assert!(reached.iter().all(|&n| n > needed), "reached {reached:?}");
    eprintln!("columns: reached {reached:?}");
}

/// Each operation edits one layout's arrangement in place in the table: no step copies the
/// layout table (64 layouts, so a copy would be the largest allocation by far). The edited
/// layout's own columns are rebuilt (the editing form), which is O(its columns and rows).
#[test]
fn the_roc_columns_step_without_copying_the_layout_table() {
    let mut rng = Rng(0xC011_0000_0000_0002);
    let settings = Settings {
        insertion: InsertionPoint::NextToSelection,
        column_width_ratio: 0.5,
        min_column_width_ratio: 0.1,
        max_column_width_ratio: 1.0,
        preset_column_widths: vec![0.33, 0.5, 1.0],
        preserve_window_sizes: true,
    };
    let mut rust = Columns::new(settings.clone());
    let mut roc = RocColumns::new(&settings);
    let mut made = Made::default();
    for _ in 0..64 {
        let answer = columns::step(&mut rust, Event::CreateLayout);
        roc.step(Event::CreateLayout);
        made.answered(&answer);
    }
    for &layout in &made.layouts {
        for &window in &WINDOWS[..3] {
            columns::step(&mut rust, Event::Insert(layout, window));
            roc.step(Event::Insert(layout, window));
        }
    }
    // A slot holds an arrangement record (strip, three lists, ids): far above 64 bytes.
    let bound = 64 * 64;
    let mut largest_seen = 0;
    for step in 0..3000 {
        let event = made.event(&mut rng, &rust);
        if matches!(
            event,
            Event::CreateLayout | Event::CloneLayout(_) | Event::RemoveLayout(_)
        ) {
            continue;
        }
        let expected = columns::step(&mut rust, event.clone());
        rift_core_roc::take_largest_allocation();
        let answered = roc.step(event.clone());
        let largest = rift_core_roc::take_largest_allocation();
        assert_eq!(format!("{answered:?}"), format!("{expected:?}"));
        largest_seen = largest_seen.max(largest);
        assert!(
            largest < bound,
            "step {step}: {event:?} allocated {largest} bytes (bound {bound})"
        );
    }
    eprintln!("columns: the largest allocation of a step: {largest_seen} bytes (bound {bound})");
}
