//! The floating system, kept between operations: the Roc twin (`RocFloating`) against the Rust
//! system (`rift_core::layout::group_forest::FloatingSystem`), on drawn runs of operations. Runs
//! reuse a few windows and the layouts the run has made, so slots are reused. Every operation
//! must answer alike and leave the same whole system: both tables' slots, versions and free
//! lists, every node, the windows left out, and every window's kept frame (its `Geometry`, bit
//! for bit). On 3% of steps a reload (both seeded from the Rust system's view); on 40% a layout
//! question of a live layout (the shape the floating geometry takes, the group bars, rift-cli's
//! container view and drawing rows). A tripwire holds every step of a 400-window system under an
//! allocation bound: no node table copied, and no frame map copied.

use std::fmt::Debug;

use rift_core::layout::floating::Floating;
use rift_core::layout::group_forest::{
    DefaultOrientation, Direction, DropAction, Edit, Focus, InsertionPoint, Kind, Orientation,
    Participation, Query, ResizeAlong,
};
use rift_core::layout::{Gaps, Size, Sizing, WindowConstraints};
use rift_core::plain::Rect;
use rift_core_roc::group_forest::{
    self, Answer, Floating as RustFloating, FloatingAnswer, FloatingEvent, FloatingView, LayoutKey,
};
use rift_core_roc::{AllocationCount, RocFloating};

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

    fn window(&mut self) -> u64 { WINDOWS[self.below(WINDOWS.len() as u64) as usize] }

    /// A window the layout holds (most of the time), else any.
    fn held(&mut self, held: &[u64]) -> u64 {
        match self.pick(held) {
            Some(window) if self.chance(80) => window,
            _ => self.window(),
        }
    }

    fn length(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * (self.below(1 << 20) as f64 / f64::from(1 << 20))
    }

    /// A frame: small and near (so directional moves find neighbours), or anywhere.
    fn frame(&mut self) -> Rect {
        if self.chance(70) {
            Rect {
                x: 50.0 * self.below(12) as f64,
                y: 50.0 * self.below(10) as f64,
                w: 100.0 + 50.0 * self.below(6) as f64,
                h: 100.0 + 50.0 * self.below(6) as f64,
            }
        } else {
            Rect {
                x: self.length(-500.0, 2500.0),
                y: self.length(-300.0, 1500.0),
                w: self.length(0.0, 2000.0),
                h: self.length(0.0, 1500.0),
            }
        }
    }

    fn floating(&mut self) -> Floating {
        Floating {
            screen: if self.chance(50) {
                SCREEN
            } else {
                self.frame()
            },
            spread: self.chance(70),
        }
    }

    fn gaps(&mut self) -> Gaps {
        let mut side = || {
            if self.chance(50) {
                0.0
            } else {
                self.length(0.0, 40.0)
            }
        };
        Gaps {
            top: side(),
            left: side(),
            bottom: side(),
            right: side(),
            horizontal: side(),
            vertical: side(),
        }
    }

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

    fn default_orientation(&mut self) -> DefaultOrientation {
        [
            DefaultOrientation::Perpendicular,
            DefaultOrientation::Same,
            DefaultOrientation::Horizontal,
            DefaultOrientation::Vertical,
        ][self.below(4) as usize]
    }

    /// Constraints known for some of `windows`, one entry a window.
    fn known(&mut self, windows: &[u64]) -> Vec<(u64, WindowConstraints)> {
        // One entry a window, as the host's map holds them (a window in two leaves is listed
        // twice).
        let mut known: Vec<(u64, WindowConstraints)> = Vec::new();
        for &window in windows {
            if !known.iter().any(|&(held, _)| held == window) && self.chance(60) {
                known.push((window, self.constraints()));
            }
        }
        known
    }
}

const WINDOWS: [u64; 10] = [
    (1 << 32) | 1,
    (1 << 32) | 2,
    (1 << 32) | 3,
    (1 << 32) | 4,
    (2 << 32) | 1,
    (2 << 32) | 2,
    (2 << 32) | 3,
    (3 << 32) | 1,
    (3 << 32) | 2,
    (3 << 32) | 3,
];

const SCREEN: Rect = Rect {
    x: 0.0,
    y: 0.0,
    w: 1000.0,
    h: 800.0,
};

/// The windows of `window`'s app (Roc has no pid: the host lists them).
fn app(window: u64) -> Vec<u64> {
    WINDOWS.iter().copied().filter(|&w| w >> 32 == window >> 32).collect()
}

fn live(system: &RustFloating) -> Vec<LayoutKey> {
    system.forest().layouts().map(|(layout, _)| layout).collect()
}

/// The layouts a run has made, removed ones too.
#[derive(Default)]
struct Made(Vec<LayoutKey>);

impl Made {
    fn answered(&mut self, answer: &FloatingAnswer) {
        if let FloatingAnswer::Tree(Answer::Layout(layout)) = answer {
            self.0.push(*layout);
        }
    }
}

fn edit(rng: &mut Rng, held: &[u64]) -> Edit<u64> {
    match rng.below(22) {
        0..=4 => Edit::Join(rng.direction()),
        5 => Edit::ConsumeOrExpel(rng.direction()),
        6..=9 => Edit::Stack(rng.default_orientation()),
        10 => Edit::Unstack(rng.default_orientation()),
        11 => Edit::Unjoin,
        12 => Edit::Fullscreen { within_gaps: rng.chance(50) },
        13 | 14 => Edit::Swap(rng.held(held), rng.held(held)),
        15 => Edit::ReorderTab {
            window: rng.held(held),
            target: rng.held(held),
        },
        16 => Edit::ToggleOrientation,
        17 => Edit::Split(Kind::Split(Orientation::Horizontal)),
        18 => Edit::Drop {
            source: rng.held(held),
            target: rng.held(held),
            action: DropAction::Stack,
        },
        19 => Edit::Resize {
            amount: 0.1,
            along: ResizeAlong::Smart,
        },
        20 => Edit::Move {
            direction: rng.direction(),
            beyond: rift_core::layout::group_forest::Beyond::Nothing,
        },
        _ => Edit::Rebalance,
    }
}

/// One of the system's operations, given a live layout (none: the system makes one).
fn change(rng: &mut Rng, system: &RustFloating, layout: Option<LayoutKey>) -> FloatingEvent {
    let Some(layout) = layout else {
        return FloatingEvent::CreateLayout;
    };
    let held = system.forest().windows(layout, Query::All);
    match rng.below(36) {
        0 => FloatingEvent::CreateLayout,
        1 => FloatingEvent::CloneLayout(layout),
        2 => FloatingEvent::RemoveLayout(layout),
        3 => FloatingEvent::Participation(
            rng.held(&held),
            if rng.chance(50) {
                Participation::LeftOut
            } else {
                Participation::TakesPart
            },
        ),
        4..=7 => FloatingEvent::AddWindow(
            layout,
            rng.window(),
            if rng.chance(70) {
                InsertionPoint::NextToSelection
            } else {
                InsertionPoint::EndOfTree
            },
        ),
        8 => FloatingEvent::RemoveWindow(rng.held(&held)),
        9 => FloatingEvent::ReplaceWindow {
            from: rng.held(&held),
            to: rng.window(),
        },
        10 => FloatingEvent::CloseApp(app(rng.window())),
        11..=14 => FloatingEvent::StoreFrame(rng.held(&held), rng.frame()),
        15 | 16 => {
            let windows: Vec<u64> = WINDOWS.to_vec();
            let known = rng.known(&windows);
            let mut live = Vec::new();
            for &window in &windows {
                if rng.chance(40) {
                    live.push((window, rng.frame()));
                }
            }
            FloatingEvent::InitializeFrames {
                layout,
                floating: rng.floating(),
                known,
                live,
            }
        }
        17..=22 => FloatingEvent::Focus(layout, match rng.below(8) {
            0 => Focus::Select(rng.held(&held)),
            1 => Focus::Ascend,
            2 => Focus::Descend,
            _ => Focus::Direction(rng.direction()),
        }),
        _ => FloatingEvent::Edit(layout, edit(rng, &held)),
    }
}

/// A query, the forest's or the system's own.
fn query(rng: &mut Rng, made: &Made, layout: LayoutKey, held: &[u64]) -> FloatingEvent {
    let window = rng.held(held);
    match rng.below(16) {
        14 | 15 => FloatingEvent::Cycle(layout),
        0 => FloatingEvent::ContainsLayout(rng.pick(&made.0).unwrap_or(layout)),
        1 => FloatingEvent::WindowParticipates(window),
        2 => FloatingEvent::SelectedWindow(layout),
        3 => FloatingEvent::SelectionContainsWindow(layout, window),
        4 => FloatingEvent::ContainsWindow(layout, window),
        5 => FloatingEvent::HasAnyFullscreenNode(layout),
        6 => FloatingEvent::HasWindowsForApp(layout, app(window)),
        7 => FloatingEvent::ParentOfSelectionIsStacked(layout),
        8 => FloatingEvent::Windows(layout, match rng.below(4) {
            0 => Query::All,
            1 => Query::Visible,
            2 => Query::Selection,
            _ => Query::Stack(window),
        }),
        9 => FloatingEvent::WindowSlot(layout, window),
        10 => FloatingEvent::WindowInDirection(layout, rng.direction()),
        11 => FloatingEvent::Cycle(layout),
        12 => FloatingEvent::FrameOf(window),
        _ => FloatingEvent::Floats(window),
    }
}

fn same<V: PartialEq + Debug>(a: &V, b: &V) -> bool {
    a == b && format!("{a:?}") == format!("{b:?}")
}

/// Every layout question of a live layout, asked of both.
fn layout_question(
    rng: &mut Rng,
    rust: &RustFloating,
    roc: &mut RocFloating,
    layout: LayoutKey,
    at: &str,
) {
    let floating = rng.floating();
    let gaps = rng.gaps();
    let windows = rust.forest().windows(layout, Query::All);
    let known = rng.known(&windows);
    let shape = rust.shape(layout, floating, gaps, |window| {
        known.iter().find(|&&(w, _)| w == window).map(|&(_, c)| c).unwrap_or_default()
    });
    let rust_shape = shape.clone().map(&|word: u64| rift_core::plain::WindowId {
        pid: (word >> 32) as u32 as i32,
        idx: word as u32,
    });
    let roc_shape = roc.shape(layout, floating, gaps, &known);
    assert!(
        same(&roc_shape, &rust_shape),
        "{at}: the shape\nRust: {rust_shape:?}\nRoc: {roc_shape:?}"
    );
    let only = rng.chance(50);
    assert!(
        same(
            &roc.group_bars(layout, floating, only),
            &rust.group_bars(layout, floating, only)
        ),
        "{at}: the bars"
    );
    assert!(
        same(
            &roc.container_view(layout),
            &rust.forest().container_view(layout)
        ),
        "{at}: the container view"
    );
    assert!(
        same(&roc.rows(layout), &rust.forest().rows(layout)),
        "{at}: the rows"
    );
}

#[derive(Default, Debug)]
struct Reached {
    steps: usize,
    queries: usize,
    told: usize,
    reloads: usize,
    questions: usize,
    joined: usize,
    stacked: usize,
    aligned: usize,
    moved_focus: usize,
}

fn run(seed: u64, runs: u64, steps: usize) -> Reached {
    let mut rng = Rng(seed);
    let before = AllocationCount::now();
    let mut reached = Reached::default();
    for run in 0..runs {
        let mut rust = RustFloating::default();
        let mut roc = RocFloating::new();
        let mut made = Made::default();
        let mut history: Vec<FloatingEvent> = Vec::new();
        for step in 0..steps {
            let layout = rng.pick(&live(&rust));
            let event = change(&mut rng, &rust, layout);
            history.push(event.clone());
            let expected = group_forest::step_floating(&mut rust, event.clone());
            let got = roc.step(event);
            assert_eq!(
                got, expected,
                "run {run} step {step}: the answer after {history:#?}"
            );
            let (view, roc_view) = (FloatingView::from(&rust), roc.view());
            assert!(
                same(&roc_view, &view),
                "run {run} step {step}: the system after {history:#?}\nRust: {view:#?}\nRoc: {roc_view:#?}"
            );
            made.answered(&expected);
            reached.steps += 1;
            match &expected {
                FloatingAnswer::Tree(Answer::Edited(
                    rift_core::layout::group_forest::Edited::Changed,
                )) => reached.joined += 1,
                FloatingAnswer::Tree(Answer::Focused(
                    rift_core::layout::group_forest::Focused::Movement { window: Some(_), .. },
                )) => reached.moved_focus += 1,
                _ => {}
            }
            reached.stacked += usize::from(rust.forest().nodes().values().any(|node| {
                matches!(node.body, rift_core::layout::group_forest::Body::Container(ref c) if c.kind.is_stacked())
            }));
            reached.aligned +=
                usize::from(rust.geometry().values().any(|g| {
                    matches!(g, rift_core::layout::group_forest::Geometry::Aligned { .. })
                }));
            if rng.chance(40)
                && let Some(layout) = rng.pick(&live(&rust))
            {
                layout_question(
                    &mut rng,
                    &rust,
                    &mut roc,
                    layout,
                    &format!("run {run} step {step}: {layout:?} after {history:#?}"),
                );
                reached.questions += 1;
            }
            if rng.chance(3) {
                // Sometimes with a state only an older file holds: a window with only a frame
                // to return to.
                let mut view = view;
                if rng.chance(50)
                    && let Some(at) = rng.pick(&(0..view.geometry.len()).collect::<Vec<_>>())
                {
                    let (window, geometry) = view.geometry[at];
                    if let Some(restore) = geometry.frame() {
                        view.geometry[at] = (
                            window,
                            rift_core::layout::group_forest::Geometry::Restorable(restore),
                        );
                    }
                }
                roc = RocFloating::seeded(&view);
                rust = view.floating().expect("a Rust system's view parses");
                reached.reloads += 1;
                assert!(same(&roc.view(), &view), "run {run} step {step}: the seed");
            }
            if rng.chance(50)
                && let Some(layout) = rng.pick(&live(&rust))
            {
                let held = rust.forest().windows(layout, Query::All);
                let asked = query(&mut rng, &made, layout, &held);
                let expected = group_forest::step_floating(&mut rust, asked.clone());
                assert_eq!(
                    roc.step(asked.clone()),
                    expected,
                    "run {run} step {step}: {asked:?} after {history:#?}"
                );
                reached.queries += 1;
                reached.told += usize::from(match &expected {
                    FloatingAnswer::Tree(Answer::Verdict(yes)) => *yes,
                    FloatingAnswer::Tree(Answer::Window(window)) => window.is_some(),
                    FloatingAnswer::Tree(Answer::Windows(windows)) => !windows.is_empty(),
                    FloatingAnswer::Frame(frame) => frame.is_some(),
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
    std::env::var("FLOATING_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[test]
fn the_roc_floating_system_answers_and_keeps_its_frames_as_the_rust_one_does() {
    let reached = run(0xf10a_7175, runs(300), 120);
    eprintln!("floating differential: {reached:?}");
    assert!(
        reached.joined > 500,
        "joins and edits change something ({reached:?})"
    );
    assert!(reached.stacked > 200, "stacks are made ({reached:?})");
    assert!(reached.aligned > 300, "frames are aligned ({reached:?})");
    assert!(
        reached.moved_focus > 150,
        "directional focus moves ({reached:?})"
    );
    assert!(reached.told > 4000, "queries answer something ({reached:?})");
    assert!(reached.reloads > 800, "reloads happen ({reached:?})");
}

/// The floating steps the Roc system still copies its node table on (see the Handoff: Roc
/// uniqueness): every step that writes the forest but participation (which writes only the
/// windows left out). Queries and participation are held to the bound.
fn known_copy(event: &FloatingEvent) -> bool {
    !matches!(
        event,
        FloatingEvent::Participation(..)
            | FloatingEvent::ContainsLayout(_)
            | FloatingEvent::WindowParticipates(_)
            | FloatingEvent::SelectedWindow(_)
            | FloatingEvent::SelectionContainsWindow(..)
            | FloatingEvent::ContainsWindow(..)
            | FloatingEvent::HasAnyFullscreenNode(_)
            | FloatingEvent::HasWindowsForApp(..)
            | FloatingEvent::ParentOfSelectionIsStacked(_)
            | FloatingEvent::Windows(..)
            | FloatingEvent::WindowSlot(..)
            | FloatingEvent::WindowInDirection(..)
            | FloatingEvent::Cycle(_)
            | FloatingEvent::FrameOf(_)
            | FloatingEvent::Floats(_)
    )
}

/// A 400-window floating system, every window with a frame, stepped through 3000 drawn
/// operations and queries (the known copies exempt, unless `exempt` is false): no step asks Roc for an allocation near a node table's size (64
/// bytes a slot), nor near the frame map's (32 bytes a window): a table or the map copied
/// because the system holding it was still live makes every window event pay O(n).
#[test]
fn the_roc_floating_system_steps_without_copying_a_table_or_its_frames() {
    steps_without_copying(true)
}

/// The same steps, the known copies checked too: fails until the cause is found.
#[test]
#[ignore = "known copy, see Handoff: Roc uniqueness"]
fn the_roc_floating_system_steps_without_copying_a_table_even_on_known_copies() {
    steps_without_copying(false)
}

fn steps_without_copying(exempt: bool) {
    let mut rng = Rng(0xf10a_7_c0de);
    let mut rust = RustFloating::default();
    let mut roc = RocFloating::new();
    let FloatingAnswer::Tree(Answer::Layout(layout)) = roc.step(FloatingEvent::CreateLayout) else {
        panic!("a layout")
    };
    group_forest::step_floating(&mut rust, FloatingEvent::CreateLayout);
    for idx in 1..=400u64 {
        let add =
            FloatingEvent::AddWindow(layout, (7 << 32) | idx, InsertionPoint::NextToSelection);
        roc.step(add.clone());
        group_forest::step_floating(&mut rust, add);
    }
    let frames = FloatingEvent::InitializeFrames {
        layout,
        floating: Floating { screen: SCREEN, spread: true },
        known: Vec::new(),
        live: Vec::new(),
    };
    roc.step(frames.clone());
    group_forest::step_floating(&mut rust, frames);
    let mut made = Made::default();
    for step in 0..3000 {
        let Some(layout) = rng.pick(&live(&rust)) else {
            roc.step(FloatingEvent::CreateLayout);
            group_forest::step_floating(&mut rust, FloatingEvent::CreateLayout);
            continue;
        };
        let event = if rng.chance(30) {
            let held = rust.forest().windows(layout, Query::All);
            query(&mut rng, &made, layout, &held)
        } else {
            let mut drawn = change(&mut rng, &rust, Some(layout));
            // Windows of this run's own app, so the system stays large.
            if let FloatingEvent::AddWindow(at, _, point) = drawn {
                drawn = FloatingEvent::AddWindow(at, (7 << 32) | (401 + rng.below(50)), point);
            }
            drawn
        };
        let tables = 64 * rust.forest().nodes().saved().count();
        // The frame map's bound once it is large (a small map's growth is not a copy).
        let frames = if rust.geometry().len() >= 200 {
            32 * rust.geometry().len()
        } else {
            usize::MAX
        };
        rift_core_roc::take_largest_allocation();
        let expected = roc.step(event.clone());
        let largest = rift_core_roc::take_largest_allocation();
        let checked = group_forest::step_floating(&mut rust, event.clone());
        assert_eq!(expected, checked, "step {step}: {event:?}");
        made.answered(&expected);
        assert!(
            (exempt && known_copy(&event)) || largest < tables.min(frames),
            "step {step}: {event:?} allocated {largest} bytes (bounds {tables}, {frames})"
        );
    }
}

/// Both systems stepped through `events` from `view`, every answer and the whole system held
/// alike after each.
fn stepped_alike(view: &FloatingView, events: &[FloatingEvent], at: &str) {
    let mut rust = view.floating().expect("the view parses");
    let mut roc = RocFloating::seeded(view);
    assert!(same(&roc.view(), view), "{at}: the seed");
    for event in events {
        let expected = group_forest::step_floating(&mut rust, event.clone());
        assert_eq!(roc.step(event.clone()), expected, "{at}: {event:?}");
        assert!(
            same(&roc.view(), &FloatingView::from(&rust)),
            "{at}: the system after {event:?}"
        );
    }
}

/// The states only a saved file builds (step 3's fixtures, `systems/floating/*.ron` in rift),
/// seeded into both systems and stepped through the follow-ups that reach each state's rule: a
/// stack inside a stack (a stored frame shared through the outermost), an aligned window in a
/// group that is not a stack (stacking it keeps its remembered frame), a window at the root
/// with a stale frame to return to (joining it into a split forgets it).
#[test]
fn states_only_a_file_builds_step_alike() {
    use rift_core::layout::group_forest::{
        Body, Container, Fill, Geometry, Members, Memory, Node, Place,
    };
    let key = |index: u64| group_forest::NodeKey((1 << 32) | index);
    let w = |index: u64| (1 << 32) | index;
    let frame = |index: u64| Rect {
        x: 20.0 + 250.0 * (index - 1) as f64,
        y: 40.0,
        w: 200.0,
        h: 150.0,
    };
    let child = |parent: u64| Place::Child { parent: key(parent), size: 1.0 };
    let container = |place, kind, children: &[u64], chosen: u64| Node {
        place,
        fill: Fill::Tiled,
        body: Body::Container(Container {
            kind,
            members: Members::Children(children.iter().map(|&c| key(c)).collect()),
            memory: Memory::Toward(key(chosen)),
        }),
    };
    let leaf = |parent: u64, window: u64, stamp: u64| Node {
        place: child(parent),
        fill: Fill::Tiled,
        body: Body::Window { window: w(window), stamp },
    };
    let stack = Kind::Stack(Orientation::Horizontal);
    let nodes = vec![
        (None, 0),
        (
            Some(container(
                Place::Root,
                Kind::Split(Orientation::Horizontal),
                &[2, 7],
                2,
            )),
            1,
        ),
        (Some(container(child(1), stack, &[3, 6], 3)), 1),
        (Some(container(child(2), stack, &[4, 5], 4)), 1),
        (Some(leaf(3, 1, 0)), 1),
        (Some(leaf(3, 2, 1)), 1),
        (Some(leaf(2, 3, 2)), 1),
        (Some(leaf(1, 4, 3)), 1),
    ];
    let layout = LayoutKey((1 << 32) | 1);
    let nested = FloatingView {
        forest: group_forest::ForestView {
            layouts: vec![(None, 0), (Some(key(1)), 1)],
            layout_free: Vec::new(),
            nodes,
            node_free: Vec::new(),
            left_out: Vec::new(),
            next_stamp: 4,
        },
        geometry: (1..=4).map(|i| (w(i), Geometry::Kept(frame(i)))).collect(),
    };
    let stored: Vec<FloatingEvent> = (1..=4)
        .map(|i| {
            FloatingEvent::StoreFrame(w(i), Rect {
                x: 5.0 + i as f64,
                y: 5.0,
                w: 90.0,
                h: 90.0,
            })
        })
        .collect();
    stepped_alike(&nested, &stored, "stack_in_stack");

    // Four windows in a row, each with its frame, the first joined with the second (a split).
    let mut joined = RustFloating::default();
    group_forest::step_floating(&mut joined, FloatingEvent::CreateLayout);
    for i in 1..=4 {
        group_forest::step_floating(
            &mut joined,
            FloatingEvent::AddWindow(layout, w(i), InsertionPoint::NextToSelection),
        );
        group_forest::step_floating(&mut joined, FloatingEvent::StoreFrame(w(i), frame(i)));
    }
    group_forest::step_floating(&mut joined, FloatingEvent::Focus(layout, Focus::Select(w(1))));
    group_forest::step_floating(
        &mut joined,
        FloatingEvent::Edit(layout, Edit::Join(Direction::Right)),
    );
    let with_restore = |window: u64| {
        let mut view = FloatingView::from(&joined);
        for (held, geometry) in &mut view.geometry {
            if *held == w(window) {
                *geometry = Geometry::Aligned {
                    frame: frame(window),
                    restore: Rect {
                        x: 900.0,
                        y: 600.0,
                        w: 200.0,
                        h: 150.0,
                    },
                };
            }
        }
        view
    };
    stepped_alike(
        &with_restore(2),
        &[
            FloatingEvent::Focus(layout, Focus::Select(w(1))),
            FloatingEvent::Edit(layout, Edit::Stack(DefaultOrientation::Perpendicular)),
            FloatingEvent::Edit(layout, Edit::Unstack(DefaultOrientation::Perpendicular)),
        ],
        "aligned_in_split",
    );
    stepped_alike(
        &with_restore(3),
        &[
            FloatingEvent::Focus(layout, Focus::Select(w(2))),
            FloatingEvent::Edit(layout, Edit::Join(Direction::Right)),
            FloatingEvent::Edit(layout, Edit::Unjoin),
        ],
        "stale_restore_at_root",
    );
}
