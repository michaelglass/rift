//! The group forest's systems, kept between operations: the Roc twins (`RocTraditional`,
//! `RocMasterStack`) against the Rust systems (`rift_core::layout::group_forest::Traditional` and
//! `MasterStack`), on drawn runs of operations. Runs reuse a few windows and the layouts the run
//! has made (removed ones too, where an operation takes a key it may not hold: asking whether
//! one is held; every other operation indexes the layout, which the Rust forest's
//! `Index` refuses with a panic), so slots are reused and stale keys meet the slot versions. Every
//! operation must answer alike and leave the same whole system: both tables' slots, versions and
//! free lists, every node (place, size bit for bit, fill, window and its entry stamp, kind,
//! children, memory), the windows left out, the settings. On 3% of steps a reload: both seeded
//! from the Rust system's view, sometimes with states only a saved file holds (a container's
//! memory cleared while it has children, children's weights other than the operations write).

use std::fmt::Debug;

use rift_core::layout::group_forest::{
    Arrangement, Beyond, Body, Container, DefaultOrientation, Direction, DropAction, Edit, Edited,
    Fill, Focus, Focused, InsertionPoint, Kind, MasterSettings, MasterSide, Memory, NewSibling,
    NewWindowPlacement, Orientation, Participation, Place, Query, RegionInsertion, Resize,
    ResizeAlong,
};
use rift_core::plain::Rect;
use rift_core_roc::group_forest::{
    self, Answer, Forest, ForestEvent, ForestView, LayoutKey, MasterStack, MasterStackEvent,
    MasterStackView, Traditional, TraditionalEvent, TraditionalView,
};
use rift_core_roc::{AllocationCount, RocMasterStack, RocTraditional};

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);

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

    /// One of a few windows, as words (a pid in the high half, an index in the low one).
    fn window(&mut self) -> u64 { WINDOWS[self.below(WINDOWS.len() as u64) as usize] }

    /// A window the layout holds (most of the time), else any: biased toward the windows a
    /// move, swap, drop or query decides on.
    fn held(&mut self, held: &[u64]) -> u64 {
        match self.pick(held) {
            Some(window) if self.chance(80) => window,
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

    fn orientation(&mut self) -> Orientation {
        if self.chance(50) {
            Orientation::Horizontal
        } else {
            Orientation::Vertical
        }
    }

    fn default_orientation(&mut self) -> DefaultOrientation {
        [
            DefaultOrientation::Perpendicular,
            DefaultOrientation::Same,
            DefaultOrientation::Horizontal,
            DefaultOrientation::Vertical,
        ][self.below(4) as usize]
    }

    fn insertion(&mut self) -> InsertionPoint {
        if self.chance(70) {
            InsertionPoint::NextToSelection
        } else {
            InsertionPoint::EndOfTree
        }
    }

    fn new_sibling(&mut self) -> NewSibling {
        if self.chance(50) {
            NewSibling::AverageWeight
        } else {
            NewSibling::HalvesSelection
        }
    }

    fn arrangement(&mut self) -> Arrangement {
        if self.chance(60) {
            Arrangement::Derived
        } else {
            Arrangement::Set(self.orientation())
        }
    }

    fn master_settings(&mut self) -> MasterSettings {
        MasterSettings {
            insertion: [
                RegionInsertion::NextToSelection,
                RegionInsertion::EndOfTree,
                RegionInsertion::Front,
            ][self.below(3) as usize],
            ratio: match self.below(6) {
                0 => 0.5,
                1 => 0.3,
                2 => 0.95,
                3 => 0.0,
                4 => 1.2,
                _ => self.length(0.0, 1.0),
            },
            count: [0, 1, 1, 2, 3, 4, 5][self.below(7) as usize],
            side: [
                MasterSide::Left,
                MasterSide::Right,
                MasterSide::Top,
                MasterSide::Bottom,
            ][self.below(4) as usize],
            placement: [
                NewWindowPlacement::Master,
                NewWindowPlacement::Stack,
                NewWindowPlacement::Focused,
            ][self.below(3) as usize],
            master: self.arrangement(),
            stack: self.arrangement(),
        }
    }

    /// New settings: all drawn afresh, or the current ones with one field drawn (so a change of
    /// the insertion alone, which `update_settings` applies without rearranging, is reached).
    fn master_settings_near(&mut self, current: MasterSettings) -> MasterSettings {
        let drawn = self.master_settings();
        match self.below(9) {
            0 => MasterSettings {
                insertion: drawn.insertion,
                ..current
            },
            1 => MasterSettings { ratio: drawn.ratio, ..current },
            2 => MasterSettings { count: drawn.count, ..current },
            3 => MasterSettings { side: drawn.side, ..current },
            4 => MasterSettings {
                placement: drawn.placement,
                ..current
            },
            5 => MasterSettings {
                master: drawn.master,
                ..current
            },
            6 => MasterSettings { stack: drawn.stack, ..current },
            _ => drawn,
        }
    }

    /// A native resize: from a frame (or the screen or tiling area) into the screen, the tiling
    /// area, or the frame nudged on an edge or two by under a point to many points.
    fn resize(&mut self) -> Resize {
        let frame = Rect {
            x: self.length(0.0, 500.0),
            y: self.length(0.0, 400.0),
            w: self.length(50.0, 500.0),
            h: self.length(50.0, 400.0),
        };
        let old = match self.below(5) {
            0 => SCREEN,
            1 => TILING,
            _ => frame,
        };
        let nudge = |rng: &mut Self| match rng.below(4) {
            0 => 0.0,
            1 => rng.length(-0.9, 0.9),
            _ => rng.length(-120.0, 120.0),
        };
        let new = match self.below(5) {
            0 => SCREEN,
            1 => TILING,
            _ => Rect {
                x: old.x + nudge(self),
                y: old.y + nudge(self),
                w: old.w + nudge(self),
                h: old.h + nudge(self),
            },
        };
        Resize {
            old,
            new,
            screen: SCREEN,
            tiling: TILING,
        }
    }

    fn edit(&mut self, held: &[u64]) -> Edit<u64> {
        match self.below(16) {
            0 | 1 => Edit::Move {
                direction: self.direction(),
                beyond: if self.chance(30) {
                    Beyond::AnotherDisplay
                } else {
                    Beyond::Nothing
                },
            },
            2 => Edit::Split(if self.chance(50) {
                Kind::Split(self.orientation())
            } else {
                Kind::Stack(self.orientation())
            }),
            3 => Edit::Fullscreen { within_gaps: self.chance(50) },
            4 => Edit::Swap(self.held(held), self.held(held)),
            5 => Edit::ReorderTab {
                window: self.held(held),
                target: self.held(held),
            },
            6 | 7 => Edit::Drop {
                source: self.held(held),
                target: self.held(held),
                action: match self.below(3) {
                    0 => DropAction::Swap,
                    1 => DropAction::Stack,
                    _ => DropAction::Insert(self.direction()),
                },
            },
            8 => Edit::Join(self.direction()),
            9 => Edit::ConsumeOrExpel(self.direction()),
            10 => Edit::Stack(self.default_orientation()),
            11 => Edit::Unstack(self.default_orientation()),
            12 => Edit::Unjoin,
            13 => Edit::Resize {
                amount: match self.below(6) {
                    0 => 0.0,
                    1 => 1e-9,
                    2 => 5.0,
                    3 => -5.0,
                    _ => self.length(-0.6, 0.6),
                },
                along: [
                    ResizeAlong::Horizontal,
                    ResizeAlong::Vertical,
                    ResizeAlong::Smart,
                ][self.below(3) as usize],
            },
            14 => Edit::Rebalance,
            _ => Edit::ToggleOrientation,
        }
    }
}

/// The windows runs draw from: two apps of four windows each, and two of one.
const WINDOWS: [u64; 10] = [
    (1 << 32) | 1,
    (1 << 32) | 2,
    (1 << 32) | 3,
    (1 << 32) | 4,
    (2 << 32) | 1,
    (2 << 32) | 2,
    (2 << 32) | 3,
    (2 << 32) | 4,
    7,
    9,
];

const SCREEN: Rect = Rect {
    x: 0.0,
    y: 0.0,
    w: 1000.0,
    h: 800.0,
};

const TILING: Rect = Rect {
    x: 10.0,
    y: 10.0,
    w: 980.0,
    h: 780.0,
};

/// The app's windows, as the predicates tell them.
fn app(window: u64) -> Vec<u64> {
    WINDOWS.into_iter().filter(|held| held >> 32 == window >> 32).collect()
}

/// What a run has made: every layout key answered (removed ones stay, as stale keys).
#[derive(Default)]
struct Made {
    layouts: Vec<LayoutKey>,
}

impl Made {
    /// A layout the run made (live or removed), or one it never made.
    fn any_layout(&self, rng: &mut Rng) -> LayoutKey {
        match rng.pick(&self.layouts) {
            Some(layout) if rng.chance(90) => layout,
            _ => LayoutKey((7 << 32) | 40),
        }
    }

    fn answered(&mut self, answer: &Answer) {
        if let Answer::Layout(layout) = answer {
            self.layouts.push(*layout);
        }
    }
}

fn live(forest: &Forest) -> Vec<LayoutKey> { forest.layouts().map(|(layout, _)| layout).collect() }

/// One of the forest's own changes, of a live layout and a window it may hold.
fn forest_change(rng: &mut Rng, layout: LayoutKey, held: &[u64]) -> ForestEvent {
    match rng.below(16) {
        0 | 1 => ForestEvent::CloneLayout(layout),
        2 => ForestEvent::RemoveLayout(layout),
        3 | 4 => ForestEvent::Participation(
            rng.held(held),
            if rng.chance(60) {
                Participation::TakesPart
            } else {
                Participation::LeftOut
            },
        ),
        5 => ForestEvent::ReplaceWindow {
            from: rng.held(held),
            to: rng.window(),
        },
        6 | 7 => ForestEvent::ResizeWindow(layout, rng.held(held), rng.resize()),
        _ => ForestEvent::Focus(layout, match rng.below(8) {
            0 | 1 => Focus::Select(rng.held(held)),
            2 => Focus::Ascend,
            3 => Focus::Descend,
            _ => Focus::Direction(rng.direction()),
        }),
    }
}

/// A query of a live layout (any layout for whether one is held) and a window it may hold.
fn forest_query(rng: &mut Rng, made: &Made, layout: LayoutKey, held: &[u64]) -> ForestEvent {
    let window = rng.held(held);
    match rng.below(12) {
        0 => ForestEvent::ContainsLayout(made.any_layout(rng)),
        1 => ForestEvent::WindowParticipates(window),
        2 => ForestEvent::SelectedWindow(layout),
        3 => ForestEvent::SelectionContainsWindow(layout, window),
        4 => ForestEvent::WindowInDirection(layout, rng.direction()),
        5 => ForestEvent::ContainsWindow(layout, window),
        6 => ForestEvent::HasAnyFullscreenNode(layout),
        7 => ForestEvent::HasWindowsForApp(layout, app(window)),
        8 => ForestEvent::ParentOfSelectionIsStacked(layout),
        9 | 10 => ForestEvent::Windows(
            layout,
            [
                Query::All,
                Query::Visible,
                Query::Selection,
                Query::Stack(window),
            ][rng.below(4) as usize],
        ),
        _ => ForestEvent::WindowSlot(layout, window),
    }
}

/// A view's states only a saved file holds: some containers' memory cleared while they have
/// children, some children's weights other than the operations write; and some containers'
/// selection stopping at them (reachable, but rarely where a walk into the container decides).
fn with_saved_only_states(rng: &mut Rng, view: &mut ForestView) {
    for (node, _) in &mut view.nodes {
        let Some(node) = node.as_mut() else { continue };
        if let Place::Child { size, .. } = &mut node.place
            && rng.chance(15)
        {
            *size = [0.25, 2.0, 3.5, 0.0][rng.below(4) as usize];
        }
        if let Body::Container(Container { memory, members, .. }) = &mut node.body
            && let Some(&child) = members.list().first()
        {
            match rng.below(10) {
                0..=2 => *memory = Memory::Unset,
                // Reachable too (a container selected), but rarely where it decides.
                3 | 4 => *memory = Memory::Stops(memory.last().unwrap_or(child)),
                _ => {}
            }
        }
    }
}

/// A Rust system and its Roc twin.
trait Twin {
    type Event: Clone + Debug;
    type View: Clone + PartialEq + Debug;
    fn new(rng: &mut Rng) -> Self;
    fn forest(&self) -> &Forest;
    /// One of the system's operations, given a live layout (none: the system makes one).
    fn change(&self, rng: &mut Rng, layout: Option<LayoutKey>) -> Self::Event;
    /// A query, the forest's or the system's own.
    fn query(rng: &mut Rng, made: &Made, layout: LayoutKey, held: &[u64]) -> Self::Event;
    /// The Rust system's answer, then the Roc twin's.
    fn step(&mut self, event: Self::Event) -> (Answer, Answer);
    /// The Rust system's view, then the Roc twin's.
    fn views(&mut self) -> (Self::View, Self::View);
    /// Both reseeded from the Rust system's view, sometimes with saved-only states; the view.
    fn reload(&mut self, rng: &mut Rng) -> Self::View;
}

struct TraditionalTwin {
    rust: Traditional,
    roc: RocTraditional,
}

impl Twin for TraditionalTwin {
    type Event = TraditionalEvent;
    type View = TraditionalView;

    fn new(rng: &mut Rng) -> Self {
        let (insertion, new_sibling) = (rng.insertion(), rng.new_sibling());
        TraditionalTwin {
            rust: Traditional::new(insertion, new_sibling),
            roc: RocTraditional::new(insertion, new_sibling),
        }
    }

    fn forest(&self) -> &Forest { self.rust.forest() }

    fn change(&self, rng: &mut Rng, layout: Option<LayoutKey>) -> Self::Event {
        let forest = self.forest();
        let Some(layout) = layout else {
            return TraditionalEvent::CreateLayout;
        };
        let held = forest.windows(layout, Query::All);
        match rng.below(40) {
            0 => TraditionalEvent::CreateLayout,
            1 => TraditionalEvent::Settings(rng.insertion(), rng.new_sibling()),
            2..=9 => TraditionalEvent::AddWindow(
                layout,
                rng.window(),
                rng.chance(30).then(|| rng.direction()),
            ),
            10 | 11 => TraditionalEvent::RemoveWindow(rng.held(&held)),
            12 => TraditionalEvent::RemoveAndRebalance(rng.held(&held)),
            13 => TraditionalEvent::CloseApp(app(rng.window())),
            14..=25 => TraditionalEvent::Edit(layout, rng.edit(&held)),
            _ => TraditionalEvent::Forest(forest_change(rng, layout, &held)),
        }
    }

    fn query(rng: &mut Rng, made: &Made, layout: LayoutKey, held: &[u64]) -> Self::Event {
        if rng.chance(15) {
            TraditionalEvent::CanMoveWindow(layout, rng.held(held), rng.direction())
        } else {
            TraditionalEvent::Forest(forest_query(rng, made, layout, held))
        }
    }

    fn step(&mut self, event: Self::Event) -> (Answer, Answer) {
        let expected = group_forest::step_traditional(&mut self.rust, event.clone());
        (expected, self.roc.step(event))
    }

    fn views(&mut self) -> (Self::View, Self::View) {
        (TraditionalView::from(&self.rust), self.roc.view())
    }

    fn reload(&mut self, rng: &mut Rng) -> Self::View {
        let mut view = TraditionalView::from(&self.rust);
        with_saved_only_states(rng, &mut view.forest);
        self.rust = view.traditional().expect("a Rust system's view parses");
        self.roc = RocTraditional::seeded(&view);
        view
    }
}

struct MasterStackTwin {
    rust: MasterStack,
    roc: RocMasterStack,
}

impl Twin for MasterStackTwin {
    type Event = MasterStackEvent;
    type View = MasterStackView;

    fn new(rng: &mut Rng) -> Self {
        let settings = rng.master_settings();
        MasterStackTwin {
            rust: MasterStack::new(settings),
            roc: RocMasterStack::new(settings),
        }
    }

    fn forest(&self) -> &Forest { self.rust.forest() }

    fn change(&self, rng: &mut Rng, layout: Option<LayoutKey>) -> Self::Event {
        let forest = self.forest();
        let Some(layout) = layout else {
            return MasterStackEvent::CreateLayout;
        };
        let held = forest.windows(layout, Query::All);
        match rng.below(44) {
            0 => MasterStackEvent::CreateLayout,
            1 | 2 => MasterStackEvent::Settings(rng.master_settings_near(self.rust.settings())),
            3..=11 => MasterStackEvent::AddWindow(layout, rng.window()),
            12 | 13 => MasterStackEvent::RemoveWindow(rng.held(&held)),
            14 => MasterStackEvent::CloseApp(app(rng.window())),
            15 => MasterStackEvent::AdjustRatio(match rng.below(4) {
                0 => 0.0,
                1 => 1e-17,
                _ => rng.length(-0.5, 0.5),
            }),
            16 => MasterStackEvent::AdjustCount([-3, -1, 1, 3][rng.below(4) as usize]),
            17 | 18 => MasterStackEvent::Promote(layout),
            19 => MasterStackEvent::SwapMasterStack(layout),
            20..=31 => MasterStackEvent::Edit(layout, rng.edit(&held)),
            _ => MasterStackEvent::Forest(forest_change(rng, layout, &held)),
        }
    }

    fn query(rng: &mut Rng, made: &Made, layout: LayoutKey, held: &[u64]) -> Self::Event {
        if rng.chance(15) {
            MasterStackEvent::CanMoveWindow(layout, rng.held(held), rng.direction())
        } else {
            MasterStackEvent::Forest(forest_query(rng, made, layout, held))
        }
    }

    fn step(&mut self, event: Self::Event) -> (Answer, Answer) {
        let expected = group_forest::step_master_stack(&mut self.rust, event.clone());
        (expected, self.roc.step(event))
    }

    fn views(&mut self) -> (Self::View, Self::View) {
        (MasterStackView::from(&self.rust), self.roc.view())
    }

    fn reload(&mut self, rng: &mut Rng) -> Self::View {
        let mut view = MasterStackView::from(&self.rust);
        with_saved_only_states(rng, &mut view.forest);
        self.rust = view.master_stack().expect("a Rust system's view parses");
        self.roc = RocMasterStack::seeded(&view);
        view
    }
}

/// What the runs reached, so a generator that stops reaching a case fails.
#[derive(Debug, Default)]
struct Reached {
    steps: usize,
    queries: usize,
    told: usize,
    reused: usize,
    changed_edits: usize,
    raised: usize,
    moved_focus: usize,
    reloads: usize,
    nested: usize,
    stacked: usize,
    largest: usize,
}

/// Views compared by their written form too, so a size's sign of zero counts.
fn same<V: PartialEq + Debug>(a: &V, b: &V) -> bool {
    a == b && format!("{a:?}") == format!("{b:?}")
}

fn run<T: Twin>(seed: u64, runs: u64, steps: usize) -> Reached {
    let mut rng = Rng(seed);
    let before = AllocationCount::now();
    let mut reached = Reached::default();
    for run in 0..runs {
        let mut twin = T::new(&mut rng);
        let mut made = Made::default();
        let mut history: Vec<T::Event> = Vec::new();
        for step in 0..steps {
            let layout = rng.pick(&live(twin.forest()));
            let event = twin.change(&mut rng, layout);
            history.push(event.clone());
            let (expected, got) = twin.step(event);
            assert_eq!(
                got, expected,
                "run {run} step {step}: the answer after {history:#?}"
            );
            let (rust, roc) = twin.views();
            assert!(
                same(&roc, &rust),
                "run {run} step {step}: the system after {history:#?}\nRust: {rust:#?}\nRoc: {roc:#?}"
            );
            made.answered(&expected);
            reached.steps += 1;
            match &expected {
                Answer::Layout(layout) if layout.0 >> 32 > 1 => reached.reused += 1,
                Answer::Edited(Edited::Changed) => reached.changed_edits += 1,
                Answer::Edited(Edited::Raise(raised)) if !raised.is_empty() => reached.raised += 1,
                Answer::Focused(Focused::Movement { window: Some(_), .. }) => {
                    reached.moved_focus += 1
                }
                _ => {}
            }
            let forest = twin.forest();
            for (layout, _) in forest.layouts() {
                reached.largest = reached.largest.max(forest.windows(layout, Query::All).len());
            }
            reached.nested += usize::from(forest.nodes().values().any(|node| {
                matches!(node.place, Place::Child { parent, .. }
                    if matches!(forest.node(parent).place, Place::Child { .. }))
                    && matches!(node.body, Body::Container(_))
            }));
            reached.stacked += usize::from(forest.nodes().values().any(|node| {
                matches!(
                    node.body,
                    Body::Container(Container { kind: Kind::Stack(_), .. })
                ) && node.fill == Fill::Tiled
            }));
            if rng.chance(3) {
                let view = twin.reload(&mut rng);
                reached.reloads += 1;
                let (_, roc) = twin.views();
                assert!(
                    same(&roc, &view),
                    "run {run} step {step}: the seed after {history:#?}"
                );
            }
            if rng.chance(50)
                && let Some(layout) = rng.pick(&live(twin.forest()))
            {
                let held = twin.forest().windows(layout, Query::All);
                let query = T::query(&mut rng, &made, layout, &held);
                let (expected, got) = twin.step(query.clone());
                assert_eq!(
                    got, expected,
                    "run {run} step {step}: {query:?} after {history:#?}"
                );
                reached.queries += 1;
                reached.told += usize::from(match &expected {
                    Answer::Verdict(yes) => *yes,
                    Answer::Window(window) => window.is_some(),
                    Answer::Windows(windows) => !windows.is_empty(),
                    Answer::WindowSlot(slot) => slot.as_ref().is_some_and(|s| s.len() >= 2),
                    _ => false,
                });
            }
        }
    }
    assert_eq!(
        AllocationCount::now().live(),
        before.live(),
        "Roc allocations left live"
    );
    reached
}

fn runs(default: u64) -> u64 {
    std::env::var("RIFT_GROUP_FOREST_RUNS").map_or(default, |n| n.parse().unwrap())
}

fn assert_reached(reached: &Reached) {
    assert!(reached.reused > 0, "no layout reused a slot: {reached:?}");
    assert!(
        reached.changed_edits > 0 && reached.raised > 0,
        "edits did nothing: {reached:?}"
    );
    assert!(reached.moved_focus > 0, "focus never moved: {reached:?}");
    assert!(reached.reloads > 0 && reached.told > 0, "{reached:?}");
    assert!(
        reached.nested > 0 && reached.stacked > 0,
        "no nesting or stacking: {reached:?}"
    );
    assert!(reached.largest >= 5, "layouts stayed small: {reached:?}");
}

#[test]
fn the_roc_traditional_system_answers_and_keeps_the_forest_as_the_rust_one_does() {
    let reached = run::<TraditionalTwin>(0x7AD1_7104_0000_0001, runs(3_000), 80);
    println!("traditional: {reached:?}");
    assert_reached(&reached);
}

#[test]
fn the_roc_master_stack_system_answers_and_keeps_the_forest_as_the_rust_one_does() {
    let reached = run::<MasterStackTwin>(0x3A57_E125_0000_0001, runs(3_000), 80);
    println!("master-stack: {reached:?}");
    assert_reached(&reached);
}

/// A focus move entering a stack whose selection stops at it, through a container across the
/// move (no direct sibling): the walk into the stack takes its children in order. Drawn runs
/// reach this rarely; scripted here.
#[test]
fn a_focus_move_into_a_stack_whose_selection_stops_there_takes_its_first_window() {
    let (w1, w2, w3, w4, w5) = (WINDOWS[0], WINDOWS[1], WINDOWS[2], WINDOWS[3], WINDOWS[4]);
    let mut twin = TraditionalTwin {
        rust: Traditional::new(InsertionPoint::NextToSelection, NewSibling::AverageWeight),
        roc: RocTraditional::new(InsertionPoint::NextToSelection, NewSibling::AverageWeight),
    };
    let (layout, _) = twin.step(TraditionalEvent::CreateLayout);
    let Answer::Layout(layout) = layout else {
        panic!("a layout")
    };
    let select =
        |window| TraditionalEvent::Forest(ForestEvent::Focus(layout, Focus::Select(window)));
    let script = [
        TraditionalEvent::AddWindow(layout, w1, None),
        TraditionalEvent::AddWindow(layout, w2, None),
        TraditionalEvent::AddWindow(layout, w3, None),
        select(w2),
        TraditionalEvent::Edit(layout, Edit::Split(Kind::Stack(Orientation::Horizontal))),
        TraditionalEvent::AddWindow(layout, w4, None),
        select(w1),
        TraditionalEvent::Edit(layout, Edit::Split(Kind::Split(Orientation::Vertical))),
        TraditionalEvent::AddWindow(layout, w5, None),
        select(w2),
        TraditionalEvent::Forest(ForestEvent::Focus(layout, Focus::Ascend)),
        select(w1),
        TraditionalEvent::Forest(ForestEvent::Focus(layout, Focus::Direction(Direction::Right))),
    ];
    let mut last = None;
    for (step, event) in script.into_iter().enumerate() {
        let (expected, got) = twin.step(event);
        assert_eq!(got, expected, "step {step}");
        let (rust, roc) = twin.views();
        assert!(same(&roc, &rust), "step {step}: Rust {rust:#?}\nRoc {roc:#?}");
        last = Some(expected);
    }
    assert_eq!(
        last,
        Some(Answer::Focused(Focused::Movement {
            window: Some(w2),
            raise: vec![w2]
        })),
        "the stack's first window, revealed"
    );
}
