//! A BSP layout system's trees, kept between operations: the Roc twin (`RocBspTree`) against the
//! Rust tree (`rift_core::layout::bsp_tree::Tree`), on drawn runs of operations. Runs reuse a few
//! windows and the keys the run has made (stale ones too: removed layouts), so ids are reused
//! and stale keys meet the slot versions. Every operation must answer alike and leave the same
//! whole system: each table's slots, versions and free list, the windows left out, the settings.

use rift_core::layout::bsp_tree::{InsertionPoint, Participation};
use rift_core_roc::bsp_tree::{self, Answer, Event, LayoutKey, Tree, TreeView};
use rift_core_roc::{AllocationCount, RocBspTree};

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
    fn window(&mut self) -> u64 {
        [
            (1 << 32) | 1,
            (1 << 32) | 2,
            (2 << 32) | 1,
            (2 << 32) | 2,
            7,
            9,
        ][self.below(6) as usize]
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

    fn event(&self, rng: &mut Rng) -> Event {
        match rng.below(20) {
            0..=3 => Event::CreateLayout,
            4..=7 => Event::CloneLayout(self.layout(rng)),
            8..=11 => Event::RemoveLayout(self.layout(rng)),
            12..=15 => Event::Participation(
                rng.window(),
                if rng.chance(50) {
                    Participation::TakesPart
                } else {
                    Participation::LeftOut
                },
            ),
            16 | 17 => Event::Insertion(rng.insertion()),
            _ => Event::AspectRatio(rng.aspect()),
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
    let runs: u64 = std::env::var("RIFT_BSP_TREE_RUNS").map_or(1_000, |n| n.parse().unwrap());
    let mut rng = Rng(0xB5B7_7EE0_0000_0001);
    let before = AllocationCount::now();
    let (mut steps, mut reused) = (0, 0);
    for run in 0..runs {
        let insertion = rng.insertion();
        let mut rust = Tree::new(insertion);
        let mut roc = RocBspTree::new(insertion);
        let mut made = Made::default();
        let mut history = Vec::new();
        for step in 0..60 {
            let event = made.event(&mut rng);
            history.push(event);
            let expected = bsp_tree::step(&mut rust, event);
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
    eprintln!(
        "bsp_tree: {steps} operations identical ({reused} reused a slot), the system compared after each"
    );
}
