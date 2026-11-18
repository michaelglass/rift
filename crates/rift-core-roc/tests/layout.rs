//! The Roc core's layout geometry against the Rust core's, on random BSP and traditional shapes
//! and scrolling strips:
//! the frames must be identical word for word (so float bits must match). The shapes lean on
//! the edges: maxima under minima, locked axes, zero and full ratios, one-child splits, tiny and
//! non-finite weights, stacks with bars, fractional screens.

use rift_core::layout::bsp::{Bsp, Children, Fill, Node, Slot};
use rift_core::layout::floating::{Floating, Newcomer, Seen};
use rift_core::layout::scrolling::{self, Strip};
use rift_core::layout::traditional::{self, StackLine, Traditional};
use rift_core::layout::{Gaps, Size, Sizing, WindowConstraints};
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
    let maybe = |rng: &mut Rng, high: f64| {
        if rng.chance(50) {
            0.
        } else {
            rng.length(1., high)
        }
    };
    let is_resizable = rng.chance(70);
    let locked_width = maybe(rng, 1500.);
    let locked_height = maybe(rng, 1000.);
    let min = Size::reported(maybe(rng, 900.), maybe(rng, 700.));
    let mut c = WindowConstraints {
        sizing: Sizing::new(is_resizable, rng.chance(50), min),
        locked_width,
        locked_height,
        max_width: maybe(rng, 1800.),
        max_height: maybe(rng, 1200.),
    };
    if rng.chance(10) {
        c.max_width = min.map_or(0., Size::width);
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
        root: node(rng, 6, &mut next_window),
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
    let mut children: Vec<traditional::Child<WindowId>> = (0..count)
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
    if rng.chance(30) {
        let on_top = rng.below(count as u64) as usize;
        let after = children.split_off(on_top + 1);
        let focused = Box::new(children.pop().expect("count >= 1"));
        traditional::Node::Stack {
            horizontal,
            fill,
            before: children,
            focused,
            after,
        }
    } else {
        let rest = children.split_off(1);
        let first = Box::new(children.pop().expect("count >= 1"));
        traditional::Node::Axis { horizontal, fill, first, rest }
    }
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
        root: tnode(rng, 5, &mut next_window),
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
        if roc != Ok(wire::encode_frames(rust.clone())) {
            panic!(
                "case {case}: {shape:#?}\n  roc:  {:?}\n  rust: {rust:?}",
                roc.map(|words| wire::decode_frames(&words))
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
        if roc != Ok(wire::encode_frames(rust.clone())) {
            panic!(
                "case {case}: {shape:#?}\n  roc:  {:?}\n  rust: {rust:?}",
                roc.map(|words| wire::decode_frames(&words))
            );
        }
    }
    let after = AllocationCount::now();
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    eprintln!("layout: {cases} BSP shapes, {windows} frames identical");
}

fn strip(rng: &mut Rng) -> Strip<WindowId, u64> {
    let mut next_window = 1;
    let mut column = |rng: &mut Rng, id: u64| {
        let mut row = |rng: &mut Rng| {
            next_window += 1;
            scrolling::Row {
                window: WindowId { pid: 3, idx: next_window },
                constraints: constraints(rng),
                weight: match rng.below(4) {
                    0 => 1.,
                    1 => rng.float(0., 3.),
                    2 => rng.length(0., 900.),
                    _ => rng.float(-1., 0.01),
                },
            }
        };
        let width = match rng.below(4) {
            0 | 1 => scrolling::Width::Default,
            2 => scrolling::Width::Proportion(if rng.chance(10) {
                f64::NAN
            } else {
                rng.float(-0.2, 1.5)
            }),
            _ => scrolling::Width::Fixed(rng.length(-10., 2600.)),
        };
        let first = row(rng);
        let rest = (0..rng.below(4)).map(|_| row(rng)).collect();
        scrolling::Column { id, width, first, rest }
    };
    let id = rng.next();
    let first = column(rng, id);
    let rest = (0..rng.below(7))
        .map(|_| {
            let id = rng.next();
            column(rng, id)
        })
        .collect();
    Strip {
        tiling: Rect {
            x: rng.length(-500., 500.),
            y: rng.length(-50., 100.),
            w: rng.length(0., 2600.),
            h: rng.length(0., 1600.),
        },
        column_gap: rng.length(0., 30.),
        row_gap: rng.length(0., 30.),
        widths: scrolling::Widths {
            ratio: rng.float(0., 1.2),
            min_ratio: rng.float(-0.1, 0.6),
            max_ratio: rng.float(0., 1.2),
            preserve_window_sizes: rng.chance(50),
            expand_single_column: rng.chance(50),
        },
        alignment: match rng.below(3) {
            0 => scrolling::Alignment::Left,
            1 => scrolling::Alignment::Center,
            _ => scrolling::Alignment::Right,
        },
        navigation: if rng.chance(50) {
            scrolling::Navigation::Anchored
        } else {
            scrolling::Navigation::Free
        },
        first,
        rest,
    }
}

#[test]
fn the_roc_core_places_scrolling_strips_as_the_rust_core_does() {
    let cases: u64 = std::env::var("RIFT_LAYOUT_CASES").map_or(20_000, |n| n.parse().unwrap());
    let mut rng = Rng(0x5EED_57A1_C0DE_F00D);
    let before = AllocationCount::now();
    let (mut windows, mut snaps) = (0, 0);
    for case in 0..cases {
        let strip = strip(&mut rng);
        let rust = strip.place();
        windows += rust.frames.len();
        snaps += 1 + rust.snaps.1.len();
        let roc = tile_words(&wire::encode_strip(strip.clone()));
        if roc != Ok(wire::encode_placed(rust.clone())) {
            panic!(
                "case {case}: {strip:#?}\n  roc:  {:?}\n  rust: {rust:?}",
                roc.map(|words| wire::decode_placed(&words))
            );
        }
    }
    let after = AllocationCount::now();
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    eprintln!("layout: {cases} scrolling strips, {windows} frames and {snaps} snaps identical");
}

#[test]
fn the_roc_core_settles_the_camera_where_the_rust_core_does() {
    let cases: u64 = std::env::var("RIFT_LAYOUT_CASES").map_or(20_000, |n| n.parse().unwrap());
    let mut rng = Rng(0x0FF5_E7C4_11B8_A7E5);
    let before = AllocationCount::now();
    for case in 0..cases {
        let strip = strip(&mut rng);
        let placed = strip.place();
        let spots: Vec<u64> =
            std::iter::once(&placed.first).chain(&placed.rest).map(|s| s.id).collect();
        let snaps: Vec<f64> = std::iter::once(&placed.snaps.0)
            .chain(&placed.snaps.1)
            .map(|s| s.offset)
            .collect();
        // Often exactly on a resting point (ties), or halfway between two, or anywhere.
        let offset = |rng: &mut Rng| match rng.below(4) {
            0 => snaps[rng.below(snaps.len() as u64) as usize],
            1 => {
                let (a, b) = (rng.below(snaps.len() as u64), rng.below(snaps.len() as u64));
                (snaps[a as usize] + snaps[b as usize]) / 2.
            }
            2 => -0.0,
            _ => rng.length(-3000., 6000.),
        };
        let projected = offset(&mut rng);
        let from = offset(&mut rng);
        let how = match rng.below(3) {
            0 => scrolling::Settle::Niri,
            1 => scrolling::Settle::Anchored,
            _ => scrolling::Settle::AnchoredTowards(if rng.chance(90) {
                spots[rng.below(spots.len() as u64) as usize]
            } else {
                rng.next()
            }),
        };
        let view = strip.tiling.w;
        let rust = placed.settle(view, projected, from, how);
        let roc = tile_words(&wire::encode_settle(placed.clone(), view, projected, from, how));
        if roc != Ok(wire::encode_settled(rust)) {
            panic!(
                "case {case}: {how:?} from {from} to {projected}, view {view}, {placed:#?}\n  roc:  {:?}\n  rust: {rust:?}",
                roc.map(|words| wire::decode_settled(&words))
            );
        }
    }
    let after = AllocationCount::now();
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    eprintln!("layout: {cases} settle questions answered identically");
}

#[test]
fn the_roc_core_places_floating_newcomers_as_the_rust_core_does() {
    let cases: u64 = std::env::var("RIFT_LAYOUT_CASES").map_or(20_000, |n| n.parse().unwrap());
    let mut rng = Rng(0xF10A_7E5C_A5CA_DE11);
    let before = AllocationCount::now();
    let mut placed = 0;
    for case in 0..cases {
        let floating = Floating {
            screen: Rect {
                x: rng.length(-2000., 2000.),
                y: rng.length(-100., 1000.),
                w: rng.length(0., 3000.),
                h: rng.length(0., 2000.),
            },
            spread: rng.chance(50),
        };
        let windows: Vec<Newcomer> = (0..rng.below(7))
            .map(|_| Newcomer {
                constraints: constraints(&mut rng),
                seen: match rng.below(4) {
                    0 => Seen::Unseen,
                    1 => Seen::Live(Rect {
                        x: 0.,
                        y: 0.,
                        w: f64::NAN,
                        h: rng.length(0., 900.),
                    }),
                    _ => Seen::Live(Rect {
                        x: rng.length(-500., 2500.),
                        y: rng.length(-500., 1500.),
                        w: rng.length(0., 3000.),
                        h: rng.length(0., 2000.),
                    }),
                },
            })
            .collect();
        let rust = floating.first_frames(&windows);
        placed += rust.len();
        let roc = tile_words(&wire::encode_floating(floating, windows.clone()));
        if roc != Ok(wire::encode_rects(rust.clone())) {
            panic!(
                "case {case}: {floating:?} {windows:#?}\n  roc:  {:?}\n  rust: {rust:?}",
                roc.map(|words| wire::decode_rects(&words))
            );
        }
    }
    let after = AllocationCount::now();
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    eprintln!("layout: {cases} floating layouts, {placed} first frames identical");
}

#[test]
#[ignore]
fn tile_timing() {
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let bsp: Vec<Vec<u64>> = (0..200_000).map(|_| wire::encode_bsp(shape(&mut rng))).collect();
    let trad: Vec<Vec<u64>> =
        (0..200_000).map(|_| wire::encode_traditional(tshape(&mut rng))).collect();
    let strips: Vec<Vec<u64>> = (0..200_000).map(|_| wire::encode_strip(strip(&mut rng))).collect();
    let floating: Vec<Vec<u64>> = (0..200_000)
        .map(|_| {
            let screen = Rect {
                x: 0.,
                y: 0.,
                w: rng.length(400., 3000.),
                h: rng.length(300., 2000.),
            };
            let windows = (0..1 + rng.below(6))
                .map(|_| Newcomer {
                    constraints: constraints(&mut rng),
                    seen: Seen::Unseen,
                })
                .collect();
            wire::encode_floating(Floating { screen, spread: rng.chance(50) }, windows)
        })
        .collect();
    for (name, shapes) in [
        ("bsp", &bsp),
        ("traditional", &trad),
        ("scrolling", &strips),
        ("floating", &floating),
    ] {
        let mut best = std::time::Duration::MAX;
        for _ in 0..5 {
            let start = std::time::Instant::now();
            for words in shapes {
                std::hint::black_box(tile_words(words).expect("a shape"));
            }
            best = best.min(start.elapsed());
        }
        eprintln!(
            "TILE {name}: {:?} per shape (best of 5)",
            best / shapes.len() as u32
        );
    }
}
