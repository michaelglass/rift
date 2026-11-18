//! The Roc core's layout geometry against the Rust core's, on random BSP and traditional shapes:
//! the frames must be identical word for word (so float bits must match). The shapes lean on
//! the edges: maxima under minima, locked axes, zero and full ratios, one-child splits, tiny and
//! non-finite weights, stacks with bars, fractional screens.

use rift_core::layout::bsp::{Bsp, Children, Fill, Node, Slot};
use rift_core::layout::traditional::{self, StackLine, Traditional};
use rift_core::layout::{Gaps, WindowConstraints};
use rift_core::plain::{Rect, WindowId, wire};
use rift_core_roc::{AllocationCount, tile_words};

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

    fn float(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Mostly round numbers, sometimes halves, sometimes anything.
    fn length(&mut self, low: f64, high: f64) -> f64 {
        let value = self.float(low, high);
        match self.below(3) {
            0 => value.round(),
            1 => (value * 2.).round() / 2.,
            _ => value,
        }
    }
}

fn constraints(rng: &mut Rng) -> WindowConstraints {
    let mut maybe = |rng: &mut Rng, high: f64| {
        if rng.chance(50) {
            0.
        } else {
            rng.length(1., high)
        }
    };
    let mut c = WindowConstraints {
        is_resizable: rng.chance(70),
        locked_width: maybe(rng, 1500.),
        locked_height: maybe(rng, 1000.),
        min_width: maybe(rng, 900.),
        min_height: maybe(rng, 700.),
        max_width: maybe(rng, 1800.),
        max_height: maybe(rng, 1200.),
    };
    if rng.chance(10) {
        c.max_width = c.min_width;
    }
    c
}

fn node(rng: &mut Rng, depth: u32, next_window: &mut u32) -> Node<WindowId> {
    if depth == 0 || rng.chance(35) {
        let slot = if rng.chance(15) {
            Slot::Empty
        } else {
            *next_window += 1;
            Slot::Window {
                window: WindowId {
                    pid: 100 + (*next_window % 3) as i32,
                    idx: *next_window,
                },
                constraints: rng.chance(50).then(|| constraints(rng)),
            }
        };
        let fill = match rng.below(10) {
            0 => Fill::Screen,
            1 => Fill::WithinGaps,
            _ => Fill::Tiled,
        };
        return Node::Leaf {
            slot,
            fill,
            stacked: rng.chance(10),
        };
    }
    let ratio = match rng.below(10) {
        0 => 0.0,
        1 => 1.0,
        2 => 0.5,
        _ => rng.float(0.05, 0.95) as f32,
    };
    let children = if rng.chance(15) {
        Children::One(Box::new(node(rng, depth - 1, next_window)))
    } else {
        Children::Two(
            Box::new(node(rng, depth - 1, next_window)),
            Box::new(node(rng, depth - 1, next_window)),
        )
    };
    Node::Split {
        horizontal: rng.chance(50),
        ratio,
        children,
    }
}

fn shape(rng: &mut Rng) -> Bsp<WindowId> {
    let gap = |rng: &mut Rng| {
        if rng.chance(40) {
            0.
        } else {
            rng.length(0., 40.)
        }
    };
    let mut next_window = 0;
    Bsp {
        screen: Rect {
            x: rng.length(-2000., 3000.),
            y: rng.length(-500., 1500.),
            w: rng.length(0., 3500.),
            h: rng.length(0., 2200.),
        },
        gaps: Gaps {
            top: gap(rng),
            left: gap(rng),
            bottom: gap(rng),
            right: gap(rng),
            horizontal: gap(rng),
            vertical: gap(rng),
        },
        root: (!rng.chance(5)).then(|| node(rng, 6, &mut next_window)),
        aspect: rng.chance(20).then(|| rng.float(0.3, 3.)),
    }
}

fn tnode(rng: &mut Rng, depth: u32, next_window: &mut u32) -> traditional::Node<WindowId> {
    let fill = match rng.below(12) {
        0 => traditional::Fill::Screen,
        1 => traditional::Fill::WithinGaps,
        _ => traditional::Fill::Tiled,
    };
    if depth == 0 || rng.chance(40) {
        *next_window += 1;
        return traditional::Node::Window {
            window: WindowId {
                pid: 200 + (*next_window % 4) as i32,
                idx: *next_window,
            },
            constraints: rng.chance(50).then(|| constraints(rng)),
            fill,
        };
    }
    let count = 1 + rng.below(4) as usize;
    let children = (0..count)
        .map(|_| traditional::Child {
            size: match rng.below(12) {
                0 => 0.0,
                1 => 0.01,
                2 => f32::NAN,
                3 => 1.0,
                _ => rng.float(0.05, 3.0) as f32,
            },
            node: tnode(rng, depth - 1, next_window),
        })
        .collect();
    let horizontal = rng.chance(50);
    let kind = if rng.chance(30) {
        traditional::Kind::Stack {
            horizontal,
            focused: rng.below(count as u64) as usize,
        }
    } else {
        traditional::Kind::Axis { horizontal }
    };
    traditional::Node::Container { kind, fill, children }
}

fn tshape(rng: &mut Rng) -> Traditional<WindowId> {
    let bsp = shape(rng);
    let mut next_window = 0;
    Traditional {
        screen: bsp.screen,
        area: rift_core::layout::tiling_area(bsp.screen, bsp.gaps),
        gaps: bsp.gaps,
        stack_offset: if rng.chance(30) {
            0.
        } else {
            rng.length(0., 60.)
        },
        stack_line: StackLine {
            thickness: if rng.chance(50) {
                0.
            } else {
                rng.length(1., 12.)
            },
            spacing: rng.length(0., 8.),
            top: rng.chance(50),
            left: rng.chance(50),
        },
        root: (!rng.chance(5)).then(|| tnode(rng, 5, &mut next_window)),
    }
}

#[test]
fn the_roc_core_tiles_traditional_layouts_as_the_rust_core_does() {
    let cases: u64 = std::env::var("RIFT_LAYOUT_CASES").map_or(20_000, |n| n.parse().unwrap());
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let before = AllocationCount::now();
    let mut windows = 0;
    for case in 0..cases {
        let shape = tshape(&mut rng);
        let rust = shape.frames();
        windows += rust.len();
        let roc = tile_words(&wire::encode_traditional(shape.clone()));
        if roc != wire::encode_frames(rust.clone()) {
            panic!(
                "case {case}: {shape:#?}\n  roc:  {:?}\n  rust: {rust:?}",
                wire::decode_frames(&roc)
            );
        }
    }
    let after = AllocationCount::now();
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    eprintln!("layout: {cases} traditional shapes, {windows} frames identical");
}

#[test]
fn the_roc_core_tiles_what_the_rust_core_tiles() {
    let cases: u64 = std::env::var("RIFT_LAYOUT_CASES").map_or(20_000, |n| n.parse().unwrap());
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let before = AllocationCount::now();
    let mut windows = 0;
    for case in 0..cases {
        let shape = shape(&mut rng);
        let rust = shape.frames();
        windows += rust.len();
        let roc = tile_words(&wire::encode_bsp(shape.clone()));
        if roc != wire::encode_frames(rust.clone()) {
            panic!(
                "case {case}: {shape:#?}\n  roc:  {:?}\n  rust: {rust:?}",
                wire::decode_frames(&roc)
            );
        }
    }
    let after = AllocationCount::now();
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    eprintln!("layout: {cases} BSP shapes, {windows} frames identical");
}
