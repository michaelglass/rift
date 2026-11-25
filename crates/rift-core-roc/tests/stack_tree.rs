//! A stack layout system's stacks, kept between operations: the Roc twin (`RocStackTree`) against
//! the Rust system (`rift_core::layout::stack_tree::Stacks`), on 8000 drawn runs of operations.
//! Runs reuse a few windows and the layouts the run has made (removed ones too, where an
//! operation takes a key it may not hold: removing a layout, asking whether one is held; every
//! other operation indexes the layout, which upstream's `Index` refuses with a panic), so slots
//! are reused and stale keys meet the slot versions. Every operation must answer alike and leave
//! the same whole system: the table's slots, versions and free list, the id counter, the
//! settings, the windows left out.

use rift_core::layout::stack_tree::{
    Beyond, DefaultOrientation, Direction, Edit, Fill, Focus, Focused, InsertionPoint, Members,
    Participation, Query, Resize, Selection,
};
use rift_core::plain::Rect;
use rift_core_roc::stack_tree::{self, Answer, Event, LayoutKey, Scope, Stacks, StacksView};
use rift_core_roc::{AllocationCount, RocStackTree};

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

    /// A frame a native resize starts or ends at: the screen, the tiling area, or another.
    fn frame(&mut self) -> Rect {
        match self.below(4) {
            0 => SCREEN,
            1 => TILING,
            2 => Rect {
                x: 100.0,
                y: 100.0,
                w: 400.0,
                h: 300.0,
            },
            _ => Rect {
                x: 10.0,
                y: 10.0,
                w: 980.4,
                h: 780.0,
            },
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

    fn orientation(&mut self) -> DefaultOrientation {
        [
            DefaultOrientation::Perpendicular,
            DefaultOrientation::Same,
            DefaultOrientation::Horizontal,
            DefaultOrientation::Vertical,
        ][self.below(4) as usize]
    }

    fn insertion(&mut self) -> InsertionPoint {
        if self.chance(50) {
            InsertionPoint::NextToSelection
        } else {
            InsertionPoint::EndOfTree
        }
    }

    fn edit(&mut self, held: &[u64]) -> Edit<u64> {
        match self.below(10) {
            0..=2 => Edit::Move {
                direction: self.direction(),
                beyond: if self.chance(30) {
                    Beyond::AnotherDisplay
                } else {
                    Beyond::Nothing
                },
            },
            3 => Edit::Fullscreen { within_gaps: self.chance(50) },
            4 => Edit::Swap(self.held(held), self.held(held)),
            5 => Edit::ReorderTab {
                window: self.held(held),
                target: self.held(held),
            },
            6 | 7 => Edit::Drop {
                source: self.held(held),
                target: self.held(held),
            },
            8 => Edit::Restack,
            _ => Edit::ToggleOrientation,
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
    fn any_layout(&self, rng: &mut Rng) -> LayoutKey {
        match rng.pick(&self.layouts) {
            Some(layout) if rng.chance(90) => layout,
            _ => LayoutKey((7 << 32) | 40),
        }
    }

    /// The app's windows, as the predicates tell them.
    fn app(window: u64) -> Vec<u64> {
        WINDOWS.into_iter().filter(|held| held >> 32 == window >> 32).collect()
    }

    fn event(&self, rng: &mut Rng, stacks: &Stacks) -> Event {
        let live: Vec<LayoutKey> = stacks.layouts().keys().collect();
        let Some(layout) = rng.pick(&live) else {
            return Event::CreateLayout;
        };
        let held = stacks.windows(layout, Query::All);
        match rng.below(40) {
            30..=39 => Event::Edit(layout, rng.edit(&held)),
            24..=29 => Event::Focus(layout, match rng.below(5) {
                0 => Focus::Select(rng.held(&held)),
                1 => Focus::Ascend,
                2 => Focus::Descend,
                _ => Focus::Direction(rng.direction()),
            }),
            0 | 1 => Event::CreateLayout,
            2 | 3 => Event::CloneLayout(layout),
            4 => Event::RemoveLayout(self.any_layout(rng)),
            5..=7 => Event::Participation(
                rng.window(),
                if rng.chance(60) {
                    Participation::TakesPart
                } else {
                    Participation::LeftOut
                },
            ),
            8 => Event::Settings(rng.orientation(), rng.insertion()),
            9..=16 => Event::AddWindow(layout, rng.window()),
            17 | 18 => Event::RemoveWindow(rng.held(&held)),
            19 => Event::ReplaceWindow {
                from: rng.held(&held),
                to: rng.window(),
            },
            20 => Event::CloseApp(Self::app(rng.window())),
            _ => Event::ResizeWindow(layout, rng.held(&held), Resize {
                old: rng.frame(),
                new: rng.frame(),
                screen: SCREEN,
                tiling: TILING,
            }),
        }
    }

    /// A query, of a live layout (any layout for whether one is held) and a window it may hold.
    fn query(&self, rng: &mut Rng, stacks: &Stacks) -> Option<Event> {
        let live: Vec<LayoutKey> = stacks.layouts().keys().collect();
        let layout = rng.pick(&live)?;
        let window = rng.held(&stacks.windows(layout, Query::All));
        Some(match rng.below(12) {
            0 => Event::ContainsLayout(self.any_layout(rng)),
            1 => Event::WindowParticipates(window),
            2 => Event::SelectedWindow(layout),
            3 => Event::SelectionContainsWindow(layout, window),
            4 => Event::WindowInDirection(layout, rng.direction()),
            5 => Event::ContainsWindow(layout, window),
            6 => Event::HasAnyFullscreenNode(layout),
            7 => Event::HasWindowsForApp(layout, Self::app(window)),
            8 => Event::Windows(
                layout,
                [
                    Query::All,
                    Query::Visible,
                    Query::Selection,
                    Query::Stack(window),
                ][rng.below(4) as usize],
            ),
            9 => Event::WindowSlot(layout, window),
            _ => Event::GroupContainer(
                layout,
                if rng.chance(50) {
                    Scope::Whole
                } else {
                    Scope::SelectionPath
                },
            ),
        })
    }

    fn answered(&mut self, answer: &Answer) {
        if let Answer::Layout(layout) = answer {
            self.layouts.push(*layout);
        }
    }
}

#[test]
fn the_roc_stack_tree_answers_and_keeps_the_system_as_the_rust_stacks_do() {
    let runs: u64 = std::env::var("RIFT_STACK_TREE_RUNS").map_or(8_000, |n| n.parse().unwrap());
    let mut rng = Rng(0x57AC_7EE0_0000_0001);
    let before = AllocationCount::now();
    let (mut steps, mut reused, mut resized, mut largest) = (0, 0, 0, 0);
    let (mut queries, mut told, mut reached, mut rooted, mut moved) = (0, 0, 0, 0, 0);
    for run in 0..runs {
        let (orientation, insertion) = (rng.orientation(), rng.insertion());
        let mut rust = Stacks::new(orientation, insertion);
        let mut roc = RocStackTree::new(orientation, insertion);
        let mut made = Made::default();
        let mut history = Vec::new();
        let mut reloads = Vec::new();
        for step in 0..60 {
            let event = made.event(&mut rng, &rust);
            history.push(event.clone());
            let before_step = StacksView::from(&rust);
            let expected = stack_tree::step(&mut rust, event.clone());
            if matches!(event, Event::ResizeWindow(..)) && StacksView::from(&rust) != before_step {
                resized += 1;
            }
            if matches!(event, Event::Edit(_, Edit::Move { .. }))
                && expected == Answer::Edited(rift_core::layout::stack_tree::Edited::Changed)
            {
                moved += 1;
            }
            assert_eq!(
                roc.step(event),
                expected,
                "run {run} step {step}: the answer after {history:#?}"
            );
            assert_eq!(
                roc.view(),
                StacksView::from(&rust),
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
            rooted += usize::from(
                rust.layouts()
                    .values()
                    .any(|stack| matches!(stack.selection(), Some(Selection::Root(_)))),
            );
            largest =
                largest.max(rust.layouts().values().map(|s| s.list().len()).max().unwrap_or(0));
            if rng.chance(3) {
                // A reload: both systems seeded from the same one, with states only a saved file
                // holds (weights other than 1, from the old forest; a stack's fill on an empty
                // stack).
                let mut view = StacksView::from(&rust);
                for stack in view.layouts.iter_mut().filter_map(|(held, _)| held.as_mut()) {
                    if let Members::Populated { members, .. } = &mut stack.members {
                        for member in members.as_mut_slice() {
                            if rng.chance(30) {
                                member.weight = [0.25, 2.0, 3.5, 0.0][rng.below(4) as usize];
                            }
                        }
                    } else if rng.chance(30) {
                        stack.fill = Fill::WithinGaps;
                    }
                }
                rust = view.stacks().expect("a Rust system's view parses");
                roc = RocStackTree::seeded(&view);
                reloads.push(step);
                assert_eq!(
                    roc.view(),
                    view,
                    "run {run} step {step}: the seed (reloads at {reloads:?}) after {history:#?}"
                );
            }
            if rng.chance(50)
                && let Some(query) = made.query(&mut rng, &rust)
            {
                let expected = stack_tree::step(&mut rust, query.clone());
                assert_eq!(
                    roc.step(query.clone()),
                    expected,
                    "run {run} step {step}: {query:?} after {history:#?}"
                );
                queries += 1;
                told += usize::from(match &expected {
                    Answer::Verdict(yes) => *yes,
                    Answer::Window(window) => window.is_some(),
                    Answer::Windows(windows) => !windows.is_empty(),
                    Answer::WindowSlot(slot) => slot.is_some(),
                    Answer::Group(group) => group.is_some(),
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
    assert!(
        reused > 0,
        "no layout reused a slot: the free list went unexercised"
    );
    assert!(resized > 0, "no native resize changed a system");
    assert!(
        told * 4 > queries,
        "only {told} of {queries} queries told anything"
    );
    assert!(largest >= 6, "no stack grew past {largest} members");
    assert!(
        rooted > 0 && moved > 0,
        "no stack selected ({rooted}) or member moved ({moved})"
    );
    eprintln!(
        "stack_tree: {steps} operations identical ({reused} reused a slot, {resized} native resizes changed a system, stacks of up to {largest} members; {queries} queries, {told} telling something; {reached} focus moves reaching a window; {moved} moves; {rooted} operations leaving a stack selected), the system compared after each"
    );
}
