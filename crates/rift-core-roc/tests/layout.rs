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
use rift_core_roc::{
    AllocationCount, first_frames, place_strip, settle, tile_bsp, tile_traditional,
};

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
            size: match rng.below(14) {
                0 => 0.0,
                1 => 0.01,
                2 => f32::NAN,
                3 => 1.0,
                4 => -(rng.float(0.0, 1.0) as f32),
                5 => f32::INFINITY,
                6 => rng.float(0.0, 0.05) as f32,
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
        // Compared as words: bit for bit, floats included.
        let roc = tile_traditional(shape.clone()).expect("frames");
        if wire::encode_frames(roc.clone()) != wire::encode_frames(rust.clone()) {
            panic!("case {case}: {shape:#?}\n  roc:  {roc:?}\n  rust: {rust:?}");
        }
    }
    let after = AllocationCount::now();
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    eprintln!("layout: {cases} traditional shapes, {windows} frames identical");
}

/// A bound as an app or a container may give it: mostly plain, sometimes 0, negative, infinite
/// or NaN.
fn axis_bound(rng: &mut Rng, high: f64) -> f64 {
    match rng.below(12) {
        0 => 0.,
        1 => -rng.length(1., high),
        2 => f64::INFINITY,
        3 => f64::NAN,
        _ => rng.length(0., high),
    }
}

fn axis_weight(rng: &mut Rng) -> f64 {
    match rng.below(10) {
        0 => 0.,
        1 => rng.float(0., 0.05),
        2 => -rng.float(0., 1.),
        3 => f64::NAN,
        4..=6 => 1.,
        _ => rng.length(0., 3.),
    }
}

/// The Roc core's axis solver against the Rust core's, length for length (frames would round
/// most differences away): minima, fixed lengths and maxima that fit or overcommit, weights
/// small, zero, negative and NaN.
#[test]
fn the_roc_core_solves_axis_lengths_as_the_rust_core_does() {
    use rift_core::layout::axis::{AxisConstraints, solve_axis_lengths};
    let cases: u64 = std::env::var("RIFT_LAYOUT_CASES").map_or(20_000, |n| n.parse().unwrap());
    let mut rng = Rng(0x5EED_A415_0C0F_FEE5);
    let before = AllocationCount::now();
    let mut lengths = 0;
    for case in 0..cases {
        let count = 1 + rng.below(6);
        let items: Vec<AxisConstraints> = (0..count)
            .map(|_| AxisConstraints {
                min: if rng.chance(50) {
                    0.
                } else {
                    axis_bound(&mut rng, 900.)
                },
                fixed: rng.chance(25).then(|| axis_bound(&mut rng, 900.)),
                max: rng.chance(40).then(|| axis_bound(&mut rng, 900.)),
                weight: axis_weight(&mut rng),
                can_grow: rng.chance(80),
            })
            .collect();
        let usable = if rng.chance(5) {
            axis_bound(&mut rng, 3000.)
        } else {
            rng.length(0., 3000.)
        };
        let rust = solve_axis_lengths(&items, usable);
        let roc = rift_core_roc::solve_axis_lengths(&items, usable).expect("lengths");
        let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        if bits(&roc) != bits(&rust) {
            panic!("case {case}: {items:?} in {usable}\n  roc:  {roc:?}\n  rust: {rust:?}");
        }
        lengths += rust.len();
    }
    let after = AllocationCount::now();
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    eprintln!("layout: {cases} axes, {lengths} lengths identical");
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
        let roc = tile_bsp(shape.clone()).expect("frames");
        if wire::encode_frames(roc.clone()) != wire::encode_frames(rust.clone()) {
            panic!("case {case}: {shape:#?}\n  roc:  {roc:?}\n  rust: {rust:?}");
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
        let roc = place_strip(strip.clone()).expect("a placement");
        if wire::encode_placed(roc.clone()) != wire::encode_placed(rust.clone()) {
            panic!("case {case}: {strip:#?}\n  roc:  {roc:?}\n  rust: {rust:?}");
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
        let roc = settle(placed.clone(), view, projected, from, how).expect("a settling");
        if wire::encode_settled(roc) != wire::encode_settled(rust) {
            panic!(
                "case {case}: {how:?} from {from} to {projected}, view {view}, {placed:#?}\n  roc:  {roc:?}\n  rust: {rust:?}"
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
        let roc = first_frames(floating, windows.clone()).expect("first frames");
        if wire::encode_rects(roc.clone()) != wire::encode_rects(rust.clone()) {
            panic!("case {case}: {floating:?} {windows:#?}\n  roc:  {roc:?}\n  rust: {rust:?}");
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
    let bsp: Vec<_> = (0..200_000).map(|_| shape(&mut rng)).collect();
    let trad: Vec<_> = (0..200_000).map(|_| tshape(&mut rng)).collect();
    let strips: Vec<_> = (0..200_000).map(|_| strip(&mut rng)).collect();
    let floating: Vec<_> = (0..200_000)
        .map(|_| {
            let screen = Rect {
                x: 0.,
                y: 0.,
                w: rng.length(400., 3000.),
                h: rng.length(300., 2000.),
            };
            let windows: Vec<Newcomer> = (0..1 + rng.below(6))
                .map(|_| Newcomer {
                    constraints: constraints(&mut rng),
                    seen: Seen::Unseen,
                })
                .collect();
            (Floating { screen, spread: rng.chance(50) }, windows)
        })
        .collect();
    fn best<T: Clone>(name: &str, items: &[T], mut run: impl FnMut(T)) {
        let mut best = std::time::Duration::MAX;
        for _ in 0..5 {
            let owned: Vec<T> = items.to_vec();
            let start = std::time::Instant::now();
            for item in owned {
                run(item);
            }
            best = best.min(start.elapsed());
        }
        eprintln!(
            "TILE {name}: {:?} per shape (best of 5)",
            best / items.len() as u32
        );
    }
    best("bsp", &bsp, |shape| {
        std::hint::black_box(tile_bsp(shape).expect("frames"));
    });
    best("traditional", &trad, |shape| {
        std::hint::black_box(tile_traditional(shape).expect("frames"));
    });
    best("scrolling", &strips, |strip| {
        std::hint::black_box(place_strip(strip).expect("a placement"));
    });
    best("floating", &floating, |(floating, windows)| {
        std::hint::black_box(first_frames(floating, windows).expect("first frames"));
    });
}

#[test]
fn the_roc_core_places_whole_floating_layouts_as_the_rust_core_does() {
    use rift_core::layout::floating::{Layout, Place, Slot};
    let cases: u64 = std::env::var("RIFT_LAYOUT_CASES").map_or(20_000, |n| n.parse().unwrap());
    let mut rng = Rng(0xF10A_7E5C_1A70_0E77);
    let before = AllocationCount::now();
    let mut placed = 0;
    for case in 0..cases {
        let gap = |rng: &mut Rng| {
            if rng.chance(40) {
                0.
            } else {
                rng.length(0., 40.)
            }
        };
        let layout = Layout {
            floating: Floating {
                screen: Rect {
                    x: rng.length(-2000., 2000.),
                    y: rng.length(-100., 1000.),
                    w: rng.length(0., 3000.),
                    h: rng.length(0., 2000.),
                },
                spread: rng.chance(50),
            },
            gaps: Gaps {
                top: gap(&mut rng),
                left: gap(&mut rng),
                bottom: gap(&mut rng),
                right: gap(&mut rng),
                horizontal: gap(&mut rng),
                vertical: gap(&mut rng),
            },
            windows: (0..rng.below(8) as u32)
                .map(|idx| {
                    if rng.chance(15) {
                        return Slot::LeftOut;
                    }
                    let place = match rng.below(6) {
                        0 => Place::Screen,
                        1 => Place::WithinGaps,
                        2 | 3 => Place::Kept(Rect {
                            x: rng.length(-500., 2500.),
                            y: rng.length(-500., 1500.),
                            w: rng.length(0., 3000.),
                            h: rng.length(0., 2000.),
                        }),
                        _ => Place::First(constraints(&mut rng)),
                    };
                    Slot::In {
                        window: WindowId { pid: 9, idx: idx + 1 },
                        place,
                    }
                })
                .collect(),
        };
        let rust = layout.frames();
        placed += rust.len();
        let roc = rift_core_roc::floating_frames(layout.clone()).expect("frames");
        if wire::encode_frames(roc.clone()) != wire::encode_frames(rust.clone()) {
            panic!("case {case}: {layout:#?}\n  roc:  {roc:?}\n  rust: {rust:?}");
        }
    }
    let after = AllocationCount::now();
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    eprintln!("layout: {cases} whole floating layouts, {placed} frames identical");
}

/// An offset that puts `frame` at a parking edge, just past it, or anywhere (fractional).
fn edge_offset(rng: &mut Rng, frame: Rect, tiling: Rect) -> f64 {
    let left_edge = frame.x + frame.w;
    let right_edge = frame.x - tiling.w;
    let nudge = |rng: &mut Rng| match rng.below(4) {
        0 => 0.,
        1 => 1e-9,
        2 => -1e-9,
        _ => rng.float(-1., 1.),
    };
    match rng.below(4) {
        0 => left_edge + nudge(rng),
        1 => right_edge + nudge(rng),
        2 => rng.float(-3000., 6000.),
        _ => rng.length(-3000., 6000.) + 0.5,
    }
}

#[test]
fn the_roc_core_shows_a_strip_as_the_rust_core_does() {
    use rift_core::layout::scrolling::{Camera, Fullscreen, Parking, translate};
    let cases: u64 = std::env::var("RIFT_LAYOUT_CASES").map_or(20_000, |n| n.parse().unwrap());
    let mut rng = Rng(0x7A45_1A7E_0FF5_E7ED);
    let before = AllocationCount::now();
    let mut shown = 0;
    for case in 0..cases {
        let screen = Rect {
            x: rng.length(-2000., 2000.),
            y: rng.length(-100., 1000.),
            w: rng.length(0., 3000.),
            h: rng.length(0., 2000.),
        };
        let inset = rng.length(0., 40.);
        let tiling = Rect {
            x: screen.x + inset,
            y: screen.y + inset,
            w: (screen.w - 2. * inset).max(0.),
            h: (screen.h - 2. * inset).max(0.),
        };
        let frames: Vec<(WindowId, Rect)> = (0..rng.below(7) as u32)
            .map(|idx| {
                (WindowId { pid: 5, idx: idx + 1 }, Rect {
                    x: rng.float(-500., 8000.),
                    y: tiling.y + rng.float(0., 500.),
                    w: rng.float(1., 2500.),
                    h: rng.float(1., 1500.),
                })
            })
            .collect();
        let offset = match frames.first() {
            Some(&(_, frame)) => edge_offset(&mut rng, frame, tiling),
            None => rng.float(-3000., 6000.),
        };
        let mut fullscreen: Vec<(WindowId, Fullscreen)> = Vec::new();
        for &(window, _) in &frames {
            if rng.chance(15) {
                let mode = if rng.chance(50) {
                    Fullscreen::Screen
                } else {
                    Fullscreen::WithinGaps
                };
                fullscreen.push((window, mode));
            }
        }
        let camera = Camera {
            screen,
            tiling,
            offset,
            parking: if rng.chance(70) {
                Parking::Park
            } else {
                Parking::InPlace
            },
        };
        let rust = translate(&frames, camera, &fullscreen);
        shown += rust.len();
        let roc = rift_core_roc::translate(&frames, camera, &fullscreen).expect("frames");
        if wire::encode_frames(roc.clone()) != wire::encode_frames(rust.clone()) {
            panic!(
                "case {case}: {camera:?} {frames:?} {fullscreen:?}\n  roc:  {roc:?}\n  rust: {rust:?}"
            );
        }
    }
    let after = AllocationCount::now();
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    eprintln!("layout: {cases} strips shown, {shown} frames identical");
}
