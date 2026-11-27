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
//! On 40% of steps a layout pass of a live layout: the Roc forest's own shape through the Roc
//! geometry against the Rust forest's shape through the Rust geometry (frames bit for bit), the
//! pass with its group bars (`Forest::project`, every stack's bar or the selection path's), and
//! rift-cli's container view and drawing rows.

use std::fmt::Debug;

use rift_core::layout::group_forest::{
    Arrangement, Bars, Beyond, Body, Container, DefaultOrientation, Direction, DropAction, Edit,
    Edited, Fill, Focus, Focused, InsertionPoint, Kind, MasterSettings, MasterSide, Memory,
    NewSibling, NewWindowPlacement, Orientation, Participation, Pass, Place, Query,
    RegionInsertion, Resize, ResizeAlong,
};
use rift_core::layout::traditional::StackLine;
use rift_core::layout::{Gaps, Size, Sizing, WindowConstraints};
use rift_core::plain::{Rect, WindowId};
use rift_core_roc::group_forest::{
    self, Answer, ContainerView, Forest, ForestEvent, ForestView, LayoutKey, MasterStack,
    MasterStackEvent, MasterStackView, Projected, Row, Traditional, TraditionalEvent,
    TraditionalView,
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

    /// A layout pass's surroundings: the run's screen or one drawn anywhere (narrow and empty
    /// too), gaps, a stack offset and a stack line (often none).
    fn pass(&mut self) -> Pass {
        let screen = if self.chance(50) {
            SCREEN
        } else {
            Rect {
                x: self.length(-2000.0, 3000.0),
                y: self.length(-500.0, 1500.0),
                w: self.length(0.0, 3500.0),
                h: self.length(0.0, 2200.0),
            }
        };
        let gap = |rng: &mut Self| {
            if rng.chance(40) {
                0.0
            } else {
                rng.length(0.0, 40.0)
            }
        };
        let gaps = Gaps {
            top: gap(self),
            left: gap(self),
            bottom: gap(self),
            right: gap(self),
            horizontal: gap(self),
            vertical: gap(self),
        };
        let stack_offset = if self.chance(30) {
            0.0
        } else {
            self.length(0.0, 60.0)
        };
        let stack_line = StackLine {
            thickness: if self.chance(50) {
                0.0
            } else {
                self.length(0.0, 12.0)
            },
            spacing: self.length(0.0, 8.0),
            top: self.chance(50),
            left: self.chance(50),
        };
        Pass {
            screen,
            gaps,
            stack_offset,
            stack_line,
        }
    }

    /// A window's constraints, as the layout differential draws them.
    fn constraints(&mut self) -> WindowConstraints {
        let mut maybe = |high: f64| {
            if self.chance(50) {
                0.0
            } else {
                self.length(1.0, high)
            }
        };
        let (locked_width, locked_height) = (maybe(1500.0), maybe(1000.0));
        let min = Size::reported(maybe(900.0), maybe(700.0));
        let (max_width, max_height) = (maybe(1800.0), maybe(1200.0));
        WindowConstraints {
            sizing: Sizing::new(self.chance(70), self.chance(50), min),
            locked_width,
            locked_height,
            max_width,
            max_height,
        }
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
/// The stamp counter is sometimes ahead of the leaves by more than the operations here leave it
/// (leaves removed after many entered).
fn with_saved_only_states(rng: &mut Rng, view: &mut ForestView) {
    if rng.chance(30) {
        view.next_stamp += 1 + rng.below(40);
    }
    for (node, _) in &mut view.nodes {
        let Some(node) = node.as_mut() else { continue };
        if let Some(at) = node.child_mut()
            && rng.chance(15)
        {
            at.size = [0.25, 2.0, 3.5, 0.0][rng.below(4) as usize];
        }
        if let Body::Container {
            container: Container { memory, members, .. },
            ..
        } = &mut node.body
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
    /// `window` added to `layout`.
    fn add(layout: LayoutKey, window: u64) -> Self::Event;
    /// Whether the Roc system still copies a node table on `event` (see the Handoff: Roc
    /// uniqueness): held by its own ignored test, stepped but not checked by the main one.
    fn known_copy(event: &Self::Event) -> bool;
    /// A query, the forest's or the system's own.
    fn query(rng: &mut Rng, made: &Made, layout: LayoutKey, held: &[u64]) -> Self::Event;
    /// The Rust system's answer, then the Roc twin's.
    fn step(&mut self, event: Self::Event) -> (Answer, Answer);
    /// The Rust system's view, then the Roc twin's.
    fn views(&mut self) -> (Self::View, Self::View);
    /// Both reseeded from the Rust system's view, sometimes with saved-only states; the view.
    fn reload(&mut self, rng: &mut Rng) -> Self::View;
    /// The Rust system's forest and the Roc twin.
    fn parts(&mut self) -> (&Forest, &mut dyn RocLayouts);
}

/// A Roc twin's layout questions.
trait RocLayouts {
    fn frames(
        &mut self,
        layout: LayoutKey,
        pass: Pass,
        known: &[(u64, WindowConstraints)],
    ) -> Vec<(WindowId, Rect)>;
    fn project(
        &mut self,
        layout: LayoutKey,
        pass: Pass,
        known: &[(u64, WindowConstraints)],
        bars: Bars,
    ) -> Projected;
    fn container_view(&mut self, layout: LayoutKey) -> ContainerView;
    fn rows(&mut self, layout: LayoutKey) -> Vec<Row>;
}

macro_rules! roc_layouts {
    ($roc:ty) => {
        impl RocLayouts for $roc {
            fn frames(
                &mut self,
                layout: LayoutKey,
                pass: Pass,
                known: &[(u64, WindowConstraints)],
            ) -> Vec<(WindowId, Rect)> {
                <$roc>::frames(self, layout, pass, known)
            }

            fn project(
                &mut self,
                layout: LayoutKey,
                pass: Pass,
                known: &[(u64, WindowConstraints)],
                bars: Bars,
            ) -> Projected {
                <$roc>::project(self, layout, pass, known, bars)
            }

            fn container_view(&mut self, layout: LayoutKey) -> ContainerView {
                <$roc>::container_view(self, layout)
            }

            fn rows(&mut self, layout: LayoutKey) -> Vec<Row> { <$roc>::rows(self, layout) }
        }
    };
}

roc_layouts!(RocTraditional);
roc_layouts!(RocMasterStack);

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

    fn add(layout: LayoutKey, window: u64) -> Self::Event {
        TraditionalEvent::AddWindow(layout, window, None)
    }

    fn known_copy(event: &Self::Event) -> bool {
        match event {
            TraditionalEvent::AddWindow(_, _, toward) => toward.is_some(),
            TraditionalEvent::Forest(forest) => known_forest_copy(forest),
            TraditionalEvent::Edit(_, edit) => known_edit_copy(edit),
            _ => false,
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

    fn parts(&mut self) -> (&Forest, &mut dyn RocLayouts) { (self.rust.forest(), &mut self.roc) }
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
            32 | 33 => MasterStackEvent::Rotate(layout, rng.chance(50)),
            20..=31 => MasterStackEvent::Edit(layout, rng.edit(&held)),
            _ => MasterStackEvent::Forest(forest_change(rng, layout, &held)),
        }
    }

    fn add(layout: LayoutKey, window: u64) -> Self::Event {
        MasterStackEvent::AddWindow(layout, window)
    }

    fn known_copy(event: &Self::Event) -> bool {
        match event {
            MasterStackEvent::CreateLayout => true,
            // The first swap after the rotation order is listed copies the node table, though a
            // lone swap (SwapMasterStack, Edit::Swap) does not; not found within the time box
            // (the order listed without closures holding the forest, the swaps by recursion and
            // inline through the record all still copy). Open, docs/roc-fork.md.
            MasterStackEvent::Rotate(..) => true,
            MasterStackEvent::Forest(forest) => known_forest_copy(forest),
            MasterStackEvent::Edit(_, edit) => known_edit_copy(edit),
            _ => false,
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

    fn parts(&mut self) -> (&Forest, &mut dyn RocLayouts) { (self.rust.forest(), &mut self.roc) }
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
    passes: usize,
    framed: usize,
    several: usize,
    constrained: usize,
    bars: usize,
    bar_past_first: usize,
    master_start: usize,
    filled_root: usize,
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
                matches!(node.place(), Place::Child(at)
                    if matches!(forest.node(at.parent).place(), Place::Child(_)))
                    && matches!(node.body, Body::Container { .. })
            }));
            reached.stacked += usize::from(forest.nodes().values().any(|node| {
                matches!(node.body, Body::Container {
                    container: Container { kind: Kind::Stack(_), .. },
                    ..
                }) && node.fill == Fill::Tiled
            }));
            if rng.chance(40)
                && let Some(layout) = rng.pick(&live(twin.forest()))
            {
                let at = format!("run {run} step {step}: {layout:?} after {history:#?}");
                layout_pass(&mut rng, twin.parts(), layout, &mut reached, &at);
            }
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

fn window_id(word: u64) -> WindowId {
    WindowId {
        pid: (word >> 32) as u32 as i32,
        idx: word as u32,
    }
}

/// A layout pass of a live layout, every layout question asked of both: the Rust forest's
/// answers (its shape through the Rust geometry) against the Roc twin's.
fn layout_pass(
    rng: &mut Rng,
    (forest, roc): (&Forest, &mut dyn RocLayouts),
    layout: LayoutKey,
    reached: &mut Reached,
    at: &str,
) {
    let pass = rng.pass();
    let mut known: Vec<(u64, WindowConstraints)> = Vec::new();
    for window in WINDOWS {
        if rng.chance(40) {
            known.push((window, rng.constraints()));
        }
    }
    let constraints = |window: u64| known.iter().find(|(held, _)| *held == window).map(|&(_, c)| c);
    let expected = forest
        .shape(layout, pass, &constraints)
        .map_or_else(Vec::new, |shape| shape.map(&window_id).frames());
    let got = roc.frames(layout, pass, &known);
    assert!(
        same(&got, &expected),
        "{at}\nframes in {pass:?}, {known:?}: rust {expected:?}, roc {got:?}"
    );
    let bars = if rng.chance(50) {
        Bars::Every
    } else {
        Bars::SelectionPath
    };
    let projected = forest.project(layout, pass, &constraints, bars);
    let expected_projected = Projected {
        frames: projected
            .frames
            .iter()
            .map(|&(window, frame)| (window_id(window), frame))
            .collect(),
        bars: projected.bars,
    };
    let got_projected = roc.project(layout, pass, &known, bars);
    assert!(
        same(&got_projected, &expected_projected),
        "{at}\n{bars:?} pass in {pass:?}, {known:?}: rust {expected_projected:?}, roc {got_projected:?}"
    );
    let (view, got_view) = (forest.container_view(layout), roc.container_view(layout));
    assert!(
        same(&got_view, &view),
        "{at}\ncontainer view: rust {view:?}, roc {got_view:?}"
    );
    let (rows, got_rows) = (forest.rows(layout), roc.rows(layout));
    assert!(
        same(&got_rows, &rows),
        "{at}\nrows: rust {rows:?}, roc {got_rows:?}"
    );
    reached.passes += 1;
    reached.framed += expected.len();
    reached.several += usize::from(expected.len() >= 2);
    reached.constrained += expected
        .iter()
        .filter(|(window, _)| known.iter().any(|(held, _)| window_id(*held) == *window))
        .count();
    reached.bars += expected_projected.bars.len();
    reached.bar_past_first += expected_projected.bars.iter().filter(|bar| bar.selected > 0).count();
    let root = forest.root(layout);
    reached.filled_root += usize::from(forest.node(root).fill != Fill::Tiled);
    reached.master_start += usize::from(
        forest.node(root).fill == Fill::Tiled
            && forest.regions(root).is_some_and(|(master, stack)| {
                !forest.windows(layout, Query::All).is_empty()
                    && forest.children(stack).is_empty()
                    && !forest.children(master).is_empty()
            }),
    );
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
    assert!(
        reached.passes > 0
            && reached.several * 10 > reached.passes
            && reached.constrained * 10 > reached.framed
            && reached.bars > 0
            && reached.bar_past_first > 0
            && reached.filled_root > 0,
        "layout passes reached little: {reached:?}"
    );
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

/// The shared operations the Roc forest still copies a node table on (see the Handoff: Roc
/// uniqueness).
fn known_forest_copy(event: &ForestEvent) -> bool {
    matches!(
        event,
        ForestEvent::RemoveLayout(_) | ForestEvent::ReplaceWindow { .. }
    )
}

/// The edits the Roc forest still copies a node table on (see the Handoff: Roc uniqueness).
fn known_edit_copy(edit: &Edit<u64>) -> bool {
    matches!(
        edit,
        Edit::Drop { .. } | Edit::Move { .. } | Edit::Join(_) | Edit::ConsumeOrExpel(_)
    )
}

/// A forest of several hundred windows stepped through drawn operations and queries: no step
/// asks Roc for an allocation anywhere near a node table's size. A table copied because the
/// forest holding it was still live (its uniqueness lost) makes every window event pay O(n).
/// The bound, 64 bytes a slot (vacant ones too: a copy copies them), sits under a table copy
/// (about 112 bytes a slot) and over every list a step builds (a container's children, a
/// window list: 8 bytes an entry). The steps still known to copy (`Twin::known_copy`) are
/// stepped but exempt here; the ignored tests below check them.
fn steps_without_copying_a_table<T: Twin>(seed: u64, known: Known) {
    let mut rng = Rng(seed);
    let mut twin = T::new(&mut rng);
    let (layout, _) = twin.step(twin.change(&mut rng, None));
    let Answer::Layout(layout) = layout else {
        panic!("a layout")
    };
    for idx in 1..=400 {
        twin.step(T::add(layout, (7 << 32) | idx));
    }
    let mut made = Made::default();
    for step in 0..3000 {
        let Some(layout) = rng.pick(&live(twin.forest())) else {
            let created = twin.change(&mut rng, None);
            twin.step(created);
            continue;
        };
        let event = if rng.chance(30) {
            let held = twin.forest().windows(layout, Query::All);
            T::query(&mut rng, &made, layout, &held)
        } else {
            twin.change(&mut rng, Some(layout))
        };
        let bound = 64 * twin.forest().nodes().saved().count();
        rift_core_roc::take_largest_allocation();
        let (expected, _) = twin.step(event.clone());
        made.answered(&expected);
        let largest = rift_core_roc::take_largest_allocation();
        assert!(
            (known == Known::Exempt && T::known_copy(&event)) || largest < bound,
            "step {step}: {event:?} allocated {largest} bytes (bound {bound})"
        );
    }
}

/// Two of the steps the Roc systems still copy a node table on, scripted (a traditional window
/// added toward a direction, a master-stack layout created): each, on a 400-window forest,
/// held to the same bound. Fails until the cause (see the Handoff: Roc uniqueness) is found.
fn known_copies_stay_under_the_bound<T: Twin>(
    seed: u64,
    known: impl Fn(LayoutKey) -> Vec<T::Event>,
) {
    let mut rng = Rng(seed);
    let mut twin = T::new(&mut rng);
    let (layout, _) = twin.step(twin.change(&mut rng, None));
    let Answer::Layout(layout) = layout else {
        panic!("a layout")
    };
    for idx in 1..=400 {
        twin.step(T::add(layout, (7 << 32) | idx));
    }
    for event in known(layout) {
        assert!(T::known_copy(&event));
        let bound = 64 * twin.forest().nodes().saved().count();
        rift_core_roc::take_largest_allocation();
        twin.step(event.clone());
        let largest = rift_core_roc::take_largest_allocation();
        assert!(
            largest < bound,
            "{event:?} allocated {largest} bytes (bound {bound})"
        );
    }
}

#[test]
#[ignore = "known copy, see Handoff: Roc uniqueness"]
fn the_roc_traditional_system_adds_toward_a_direction_without_copying_a_table() {
    known_copies_stay_under_the_bound::<TraditionalTwin>(0x7AB1_E000_0000_0002, |layout| {
        [
            Direction::Up,
            Direction::Left,
            Direction::Right,
            Direction::Down,
        ]
        .into_iter()
        .zip(1..)
        .map(|(direction, idx)| {
            TraditionalEvent::AddWindow(layout, (8 << 32) | idx, Some(direction))
        })
        .collect()
    });
}

#[test]
#[ignore = "known copy, see Handoff: Roc uniqueness"]
fn the_roc_master_stack_system_creates_a_layout_without_copying_a_table() {
    known_copies_stay_under_the_bound::<MasterStackTwin>(0x3AB1_E000_0000_0002, |_| {
        vec![MasterStackEvent::CreateLayout; 3]
    });
}

/// Whether the steps a system still copies a table on are exempt from the bound.
#[derive(Clone, Copy, PartialEq)]
enum Known {
    Exempt,
    Checked,
}

#[test]
fn the_roc_traditional_system_steps_without_copying_a_table() {
    steps_without_copying_a_table::<TraditionalTwin>(0x7AB1_E000_0000_0001, Known::Exempt);
}

#[test]
fn the_roc_master_stack_system_steps_without_copying_a_table() {
    steps_without_copying_a_table::<MasterStackTwin>(0x3AB1_E000_0000_0001, Known::Exempt);
}

#[test]
#[ignore = "known copy, see Handoff: Roc uniqueness"]
fn the_roc_traditional_system_steps_without_copying_a_table_even_on_known_copies() {
    steps_without_copying_a_table::<TraditionalTwin>(0x7AB1_E000_0000_0001, Known::Checked);
}

#[test]
#[ignore = "known copy, see Handoff: Roc uniqueness"]
fn the_roc_master_stack_system_steps_without_copying_a_table_even_on_known_copies() {
    steps_without_copying_a_table::<MasterStackTwin>(0x3AB1_E000_0000_0001, Known::Checked);
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

/// A join moving Down from the last window of a horizontal container (`[w1, w3]`) whose next
/// sibling is a vertical one (`[w2, w4]`): no sibling, no lift and no traversal reach a target,
/// so the hierarchical join takes the vertical container's child nearest the move (its first,
/// w2: the move's opposite, Up, runs along it). Drawn runs never reach this; scripted here.
#[test]
fn a_hierarchical_join_down_into_a_vertical_container_takes_its_first_child() {
    let (w1, w2, w3, w4) = (WINDOWS[0], WINDOWS[1], WINDOWS[2], WINDOWS[3]);
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
        select(w1),
        TraditionalEvent::Edit(layout, Edit::Split(Kind::Split(Orientation::Horizontal))),
        TraditionalEvent::AddWindow(layout, w3, None),
        select(w2),
        TraditionalEvent::Edit(layout, Edit::Split(Kind::Split(Orientation::Vertical))),
        TraditionalEvent::AddWindow(layout, w4, None),
        select(w3),
        TraditionalEvent::Edit(layout, Edit::Join(Direction::Down)),
    ];
    for (step, event) in script.into_iter().enumerate() {
        let (expected, got) = twin.step(event);
        assert_eq!(got, expected, "step {step}");
        let (rust, roc) = twin.views();
        assert!(same(&roc, &rust), "step {step}: Rust {rust:#?}\nRoc {roc:#?}");
    }
    let drawn: Vec<String> = twin
        .roc
        .rows(layout)
        .iter()
        .map(|row| format!("{} {:?}", row.branches.len(), row.held))
        .collect();
    assert_eq!(
        drawn,
        [
            "0 Container { kind: Split(Horizontal), total: 3.0 }".to_string(),
            format!("1 Window({w1})"),
            format!("1 Window({w4})"),
            "1 Container { kind: Split(Vertical), total: 2.0 }".to_string(),
            format!("2 Window({w3})"),
            format!("2 Window({w2})"),
        ],
        "w3 joined with w2, the vertical container's first child"
    );
}
