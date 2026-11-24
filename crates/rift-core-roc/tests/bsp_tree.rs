//! A BSP layout system's trees, kept between operations: the Roc twin (`RocBspTree`) against the
//! Rust tree (`rift_core::layout::bsp_tree::Tree`), on 8000 drawn runs of operations. Runs reuse a
//! few windows and the keys the run has made (stale ones too: removed layouts), so ids are reused
//! and stale keys meet the slot versions. Every operation must answer alike and leave the same
//! whole system: each table's slots, versions and free list, the windows left out, the settings.

use rift_core::layout::bsp_tree::{
    Content, Direction, DropAction, Edit, Focus, Focused, InsertionPoint, Kind, Members, Memory,
    Orientation, Participation, Preselect, Query, Resize, ResizeAlong, SplitKind,
};
use rift_core::plain::Rect;
use rift_core_roc::bsp_tree::{self, Answer, Event, LayoutKey, Tree, TreeView};
use rift_core_roc::{AllocationCount, RocBspTree, narrow};

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

    /// A frame a native resize starts or ends at: the screen, the tiling area, or a window's
    /// frame nudged under and over half a point and changed along either axis or both.
    fn frame(&mut self) -> Rect {
        let at = |x, y, w, h| Rect { x, y, w, h };
        match self.below(10) {
            0 => SCREEN,
            1 => TILING,
            2 => at(100.0, 100.0, 400.0, 300.0),
            3 => at(100.0, 100.0, 400.4, 300.0),
            4 => at(100.0, 100.0, 400.6, 300.0),
            5 => at(100.0, 100.0, 400.0, 300.6),
            6 => at(100.0, 100.0, 550.0, 300.0),
            7 => at(100.0, 100.0, 400.0, 180.0),
            8 => at(50.0, 60.0, 620.0, 410.0),
            _ => at(100.0, 100.0, 0.0, 300.0),
        }
    }

    /// A window the layout holds (most of the time), else any: biased toward the windows a
    /// move, swap, drop or query decides on.
    fn held(&mut self, held: &[u64]) -> u64 {
        match self.pick(held) {
            Some(window) if self.chance(80) => window,
            _ => self.window(),
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

    /// A resize amount: small steps either way, large ones, and magnitudes an `f32` rounds,
    /// overflows or loses.
    fn amount(&mut self) -> f64 {
        match self.below(4) {
            0 => [
                0.05, -0.05, 0.1, -0.3, 1.0, -1.0, 10.0, 1e-9, 1e30, -7.25, 1e-40,
            ][self.below(11) as usize],
            _ => (self.next() >> 11) as f64 / (1u64 << 53) as f64 * 4.0 - 2.0,
        }
    }

    /// A shape edit (upstream's `LayoutEdit`, the ones that reshape a layout).
    fn shape_edit(&mut self) -> Edit<u64> {
        match self.below(6) {
            0 => Edit::Split(if self.chance(85) {
                SplitKind::Tiled(self.orientation())
            } else {
                SplitKind::Stacked
            }),
            1 => Edit::Fullscreen { within_gaps: self.chance(50) },
            2 | 3 => Edit::Resize {
                amount: self.amount(),
                along: [
                    ResizeAlong::Horizontal,
                    ResizeAlong::Vertical,
                    ResizeAlong::Smart,
                ][self.below(3) as usize],
            },
            4 => Edit::Rebalance,
            _ => Edit::ToggleOrientation,
        }
    }

    /// A move, swap, drop or join (the edits that move windows between leaves).
    fn moving_edit(&mut self, held: &[u64]) -> Edit<u64> {
        match self.below(8) {
            5 => Edit::Join(self.direction()),
            6 => Edit::ConsumeOrExpel(self.direction()),
            7 => Edit::Unjoin,
            0 | 1 => Edit::Move(self.direction()),
            2 => Edit::Swap(self.held(held), self.held(held)),
            _ => Edit::Drop {
                source: self.held(held),
                target: self.held(held),
                action: match self.below(6) {
                    0 => DropAction::Swap,
                    1 | 2 => DropAction::Stack,
                    3 => DropAction::Insert(self.direction()),
                    4 => DropAction::Move(self.direction()),
                    _ => DropAction::ConsumeOrExpel(self.direction()),
                },
            },
        }
    }

    fn insertion(&mut self) -> InsertionPoint {
        if self.chance(50) {
            InsertionPoint::NextToSelection
        } else {
            InsertionPoint::EndOfTree
        }
    }

    /// A ratio the setting keeps (finite and positive) or drops.
    fn aspect(&mut self) -> Option<f64> {
        match self.below(8) {
            0 => None,
            1 => Some(0.0),
            2 => Some(-1.5),
            3 => Some(f64::NAN),
            4 => Some(f64::INFINITY),
            _ => Some([0.5, 1.0, 16.0 / 9.0, 2.35][self.below(4) as usize]),
        }
    }
}

/// The windows runs draw from: two apps of three windows each, and two of one.
const WINDOWS: [u64; 8] = [
    (1 << 32) | 1,
    (1 << 32) | 2,
    (1 << 32) | 3,
    (2 << 32) | 1,
    (2 << 32) | 2,
    (2 << 32) | 3,
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

/// What a run has made: every layout key answered (removed ones stay, as stale keys).
#[derive(Default)]
struct Made {
    layouts: Vec<LayoutKey>,
}

impl Made {
    /// A layout the run made (live or removed), or one it never made.
    fn layout(&self, rng: &mut Rng) -> LayoutKey {
        match rng.pick(&self.layouts) {
            Some(layout) if rng.chance(90) => layout,
            _ => LayoutKey((7 << 32) | 40),
        }
    }

    fn event(&self, rng: &mut Rng, tree: &Tree) -> Event {
        match rng.below(52) {
            44..=51 => {
                let layout = self.layout(rng);
                Event::Edit(layout, rng.moving_edit(&tree.windows(layout, Query::All)))
            }
            36..=43 => Event::Edit(self.layout(rng), rng.shape_edit()),
            30..=35 => Event::Focus(self.layout(rng), match rng.below(5) {
                0 => Focus::Select(rng.window()),
                1 => Focus::Ascend,
                2 => Focus::Descend,
                _ => Focus::Direction(rng.direction()),
            }),
            0..=2 => Event::CreateLayout,
            3 | 4 => Event::CloneLayout(self.layout(rng)),
            5 => Event::RemoveLayout(self.layout(rng)),
            6..=8 => Event::Participation(
                rng.window(),
                if rng.chance(60) {
                    Participation::TakesPart
                } else {
                    Participation::LeftOut
                },
            ),
            9 => Event::Insertion(rng.insertion()),
            10 => Event::AspectRatio(rng.aspect()),
            11..=19 => Event::AddWindow(self.layout(rng), rng.window()),
            20 | 21 => Event::RemoveWindow(rng.window()),
            22 => Event::ReplaceWindow {
                from: rng.window(),
                to: rng.window(),
            },
            23 => {
                // The app's windows, as `close_app`'s predicate tells them.
                let pid = rng.window() >> 32;
                Event::CloseApp(WINDOWS.into_iter().filter(|window| window >> 32 == pid).collect())
            }
            _ => Event::ResizeWindow(self.layout(rng), rng.window(), Resize {
                old: rng.frame(),
                new: rng.frame(),
                screen: SCREEN,
                tiling: TILING,
            }),
        }
    }

    /// A query, of a layout the run made and a window it may hold.
    fn query(&self, rng: &mut Rng, tree: &Tree) -> Event {
        let layout = self.layout(rng);
        let window = rng.held(&tree.windows(layout, Query::All));
        match rng.below(10) {
            0 => Event::ContainsLayout(layout),
            1 => Event::WindowParticipates(window),
            2 => Event::SelectedWindow(layout),
            3 => Event::WindowInDirection(layout, rng.direction()),
            4 => Event::ContainsWindow(layout, window),
            5 => Event::HasAnyFullscreenNode(layout),
            6 => {
                let pid = window >> 32;
                Event::HasWindowsForApp(
                    layout,
                    WINDOWS.into_iter().filter(|window| window >> 32 == pid).collect(),
                )
            }
            7 => Event::Windows(
                layout,
                [
                    Query::All,
                    Query::Visible,
                    Query::Selection,
                    Query::Stack(window),
                ][rng.below(4) as usize],
            ),
            8 => Event::CanMoveWindow(layout, window, rng.direction()),
            _ => Event::WindowSlot(layout, window),
        }
    }

    fn answered(&mut self, answer: &Answer) {
        if let Answer::Layout(layout) = answer {
            self.layouts.push(*layout);
        }
    }
}

#[test]
fn the_roc_bsp_tree_answers_and_keeps_the_system_as_the_rust_tree_does() {
    let runs: u64 = std::env::var("RIFT_BSP_TREE_RUNS").map_or(8_000, |n| n.parse().unwrap());
    let mut rng = Rng(0xB5B7_7EE0_0000_0001);
    let before = AllocationCount::now();
    let (mut steps, mut reused, mut deepest, mut resized) = (0, 0, 0, 0);
    let (mut queries, mut told, mut reached, mut stacked) = (0, 0, 0, 0);
    let mut descended = 0;
    for run in 0..runs {
        let insertion = rng.insertion();
        let mut rust = Tree::new(insertion);
        let mut roc = RocBspTree::new(insertion);
        let mut made = Made::default();
        let mut history = Vec::new();
        let mut reloads = Vec::new();
        for step in 0..60 {
            let event = made.event(&mut rng, &rust);
            history.push(event.clone());
            let before_step = TreeView::from(&rust);
            let expected = bsp_tree::step(&mut rust, event.clone());
            let event_was = event.clone();
            if matches!(event, Event::ResizeWindow(..)) && TreeView::from(&rust) != before_step {
                resized += 1;
            }
            assert_eq!(
                roc.step(event),
                expected,
                "run {run} step {step}: the answer after {history:#?}"
            );
            assert_eq!(
                roc.view(),
                TreeView::from(&rust),
                "run {run} step {step}: the system after {history:#?}"
            );
            if let Answer::Layout(layout) = expected
                && layout.0 >> 32 > 1
            {
                reused += 1;
            }
            made.answered(&expected);
            steps += 1;
            if let Answer::Focused(Focused::Movement { window: Some(_), .. }) = expected {
                reached += 1;
            }
            if let (
                Event::Focus(_, Focus::Descend),
                Answer::Focused(Focused::Selection { handled: true }),
            ) = (&event_was, &expected)
            {
                descended += 1;
            }
            if rng.chance(3) {
                // A reload: both trees seeded from the same system, with states only a saved
                // file holds (preselected leaves, splits whose memory stops).
                let mut view = TreeView::from(&rust);
                for node in view.nodes.iter_mut().filter_map(|(held, _)| held.as_mut()) {
                    match &mut node.kind {
                        Kind::Leaf(leaf) if rng.chance(30) => {
                            leaf.preselect = Preselect::Toward(rng.direction())
                        }
                        Kind::Split(split) if rng.chance(30) => split.memory = Memory::Stops,
                        Kind::Leaf(_) | Kind::Split(_) => {}
                    }
                }
                rust = view.tree().expect("a Rust tree's view parses");
                roc = RocBspTree::seeded(&view);
                reloads.push(step);
                assert_eq!(
                    roc.view(),
                    view,
                    "run {run} step {step}: the seed (reloads at {reloads:?}) after {history:#?}"
                );
            }
            if rng.chance(50) {
                let query = made.query(&mut rng, &rust);
                let expected = bsp_tree::step(&mut rust, query.clone());
                assert_eq!(
                    roc.step(query.clone()),
                    expected,
                    "run {run} step {step}: {query:?} after {history:#?}"
                );
                queries += 1;
                // An answer that tells something: a window, a yes, a list, a place below a root.
                told += usize::from(match &expected {
                    Answer::Verdict(yes) => *yes,
                    Answer::Window(window) => window.is_some(),
                    Answer::Windows(windows) => !windows.is_empty(),
                    Answer::WindowSlot(slot) => slot.as_ref().is_some_and(|slot| !slot.is_empty()),
                    _ => false,
                });
            }
            deepest = deepest.max(rust.nodes().len());
            stacked += usize::from(rust.nodes().values().any(|node| {
                matches!(&node.kind, Kind::Leaf(leaf) if matches!(&leaf.content, Content::Held {
                    members: Members::Stack(_),
                    ..
                }))
            }));
        }
    }
    assert_eq!(
        AllocationCount::now().live(),
        before.live(),
        "Roc allocations left live"
    );
    assert!(
        reused > 0,
        "no layout reused a slot: the free list went unexercised"
    );
    assert!(resized > 0, "no native resize changed a system");
    assert!(
        told * 4 > queries,
        "only {told} of {queries} queries told anything"
    );
    assert!(
        deepest > 8,
        "no system grew past {deepest} nodes: the trees went unexercised"
    );
    eprintln!(
        "bsp_tree: {steps} operations identical ({reused} reused a slot, {resized} native resizes changed a system, systems of up to {deepest} nodes; {queries} queries, {told} telling something; {reached} focus moves reaching a window; {stacked} operations leaving a stack; {descended} descents moving the selection), the system compared after each"
    );
}

/// Roc has no `F64 -> F32`, so the core writes Rust's `as f32` out (`Geometry.narrow`): held to
/// it bit for bit on drawn bit patterns of every class, the midpoints between neighbouring
/// `f32`s (ties, which round to even), and the edges: just under each power of two (a carry
/// into the next exponent), the largest `f32` and past it, the smallest normal and subnormal,
/// and `f64` subnormals.
#[test]
fn the_roc_core_narrows_an_f64_as_rust_does() {
    let mut rng = Rng(0x0F32_0F64_0000_0001);
    let mut values = vec![
        0.0,
        -0.0,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::from(f32::MAX),
        f64::MAX,
        f64::MIN_POSITIVE,
        f64::from(f32::MIN_POSITIVE),
        f64::from(f32::from_bits(1)),
        f64::from(f32::from_bits(1)) / 2.0,
    ];
    // Just under each power of two: rounding up carries into the next exponent.
    for exponent in 1..255u32 {
        let below = f32::from_bits((exponent << 23) | 0x7F_FFFF);
        let midpoint = (f64::from(below) + f64::from(f32::from_bits(below.to_bits() + 1))) / 2.0;
        values.extend([midpoint, midpoint.next_up(), midpoint.next_down()]);
    }
    let (mut ties, mut odd_ties) = (0, 0);
    for _ in 0..200_000 {
        let bits = rng.next();
        values.push(f64::from_bits(bits));
        // Every f32 class, and the midpoint to its neighbour (exact in an f64).
        let low = f32::from_bits(bits as u32 & 0x7F7F_FFFF);
        let high = f32::from_bits(low.to_bits() + 1);
        let midpoint = (f64::from(low) + f64::from(high)) / 2.0;
        ties += 1;
        odd_ties += usize::from(low.to_bits() % 2 == 1);
        values.push(if bits >> 63 == 1 { -midpoint } else { midpoint });
        values.push(f64::from(low) * (1.0 + f64::EPSILON * (rng.below(5) as f64 - 2.0)));
    }
    for value in values {
        let (expected, got) = (value as f32, narrow(value));
        if expected.is_nan() {
            assert!(
                got.is_nan(),
                "{value:e} ({:#x}) narrowed to {got:e}",
                value.to_bits()
            );
        } else {
            assert_eq!(
                got.to_bits(),
                expected.to_bits(),
                "{value:e} ({:#x}) narrowed to {got:e}, Rust gives {expected:e}",
                value.to_bits()
            );
        }
    }
    assert!(odd_ties > 0 && odd_ties < ties, "ties all of one parity");
}
