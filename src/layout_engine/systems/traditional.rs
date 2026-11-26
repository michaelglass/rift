mod system;
#[cfg(feature = "roc-core")]
pub(super) use system::CoreTraditional;
pub use system::TraditionalLayoutSystem;

#[cfg(test)]
mod tests {
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};

    use super::*;
    use crate::actor::app::WindowId;
    use crate::common::collections::HashMap;
    use crate::common::config::WindowInsertionPoint;
    use crate::layout_engine::systems::{Focus, GroupEdit, LayoutEdit, LayoutSystem, WindowChange};
    use crate::layout_engine::{Direction, LayoutId, LayoutKind};

    fn w(idx: u32) -> WindowId { WindowId::new(1, idx) }

    /// A layout's root's children: each one's id (as rift-cli writes it) and weight.
    fn children(system: &TraditionalLayoutSystem, layout: LayoutId) -> Vec<(u64, f64)> {
        let tree = system.container_tree(layout);
        tree.children
            .iter()
            .map(|child| (child.node_id, child.weight.unwrap()))
            .collect()
    }

    #[test]
    fn splitting_a_narrow_tile_preserves_the_other_tiles_width() {
        let mut system = TraditionalLayoutSystem::new(WindowInsertionPoint::NextToSelection, false);
        let layout = system.create_layout();
        for window in [w(1), w(2)] {
            system.update(WindowChange::Add(layout, window, None));
        }
        for amount in [-0.25, -0.9] {
            assert!(
                system
                    .edit(layout, LayoutEdit::Resize {
                        amount,
                        orientation: crate::layout_engine::ResizeOrientation::Horizontal,
                    })
                    .changed()
            );
        }

        let widths = |system: &TraditionalLayoutSystem| {
            system
                .calculate_layout(
                    layout,
                    CGRect::new(CGPoint::ZERO, CGSize::new(4000., 800.)),
                    0.0,
                    &Default::default(),
                    &Default::default(),
                    Default::default(),
                    crate::layout_engine::Shadow::Run,
                )
                .into_iter()
                .map(|(window, frame)| (window, frame.size.width))
                .collect::<HashMap<_, _>>()
        };
        let before = widths(&system);
        assert_eq!(before[&w(2)], 100.);
        system.update(WindowChange::Add(layout, w(3), None));
        let after = widths(&system);
        assert_eq!(after[&w(1)], before[&w(1)]);
        assert_eq!(after[&w(2)], 50.);
        assert_eq!(after[&w(3)], 50.);
    }

    #[test]
    fn joining_resized_neighbors_preserves_the_unrelated_tiles_width() {
        let mut system = TraditionalLayoutSystem::default();
        let layout = system.create_layout();
        for window in [w(1), w(2), w(3)] {
            system.update(WindowChange::Add(layout, window, None));
        }
        assert!(
            system
                .edit(layout, LayoutEdit::Resize {
                    amount: 0.2,
                    orientation: crate::layout_engine::ResizeOrientation::Horizontal,
                })
                .changed()
        );
        let frame = |system: &TraditionalLayoutSystem| {
            system
                .calculate_layout(
                    layout,
                    CGRect::new(CGPoint::ZERO, CGSize::new(1200., 800.)),
                    0.0,
                    &Default::default(),
                    &Default::default(),
                    Default::default(),
                    crate::layout_engine::Shadow::Run,
                )
                .into_iter()
                .find(|(window, _)| *window == w(3))
                .unwrap()
                .1
        };
        let before = frame(&system);
        assert!(system.focus(layout, Focus::Select(w(1))).handled);
        assert!(
            system
                .edit(layout, LayoutEdit::Group(GroupEdit::Join(Direction::Right)))
                .changed()
        );
        assert_eq!(frame(&system), before);
    }

    #[test]
    fn unjoin_preserves_nested_and_outer_resize_ratios() {
        let mut system = TraditionalLayoutSystem::default();
        let layout = system.create_layout();
        let (w1, w2, w3) = (w(163), w(164), w(165));
        for window in [w1, w2, w3] {
            system.update(WindowChange::Add(layout, window, None));
        }
        assert!(system.focus(layout, Focus::Select(w1)).handled);
        assert!(
            system
                .edit(layout, LayoutEdit::Group(GroupEdit::Join(Direction::Right)))
                .changed()
        );
        let (n1, n2, n3) = (
            system.leaf(layout, w1),
            system.leaf(layout, w2),
            system.leaf(layout, w3),
        );
        let group = system.parent(n1);
        system.with_sizes(&[(group, 6.0), (n3, 4.0), (n1, 3.0), (n2, 1.0)]);
        assert!(system.focus(layout, Focus::Select(w1)).handled);
        assert!(system.edit(layout, LayoutEdit::Group(GroupEdit::Unjoin)).changed());
        let sizes: Vec<f64> = children(&system, layout).into_iter().map(|(_, size)| size).collect();
        let expected = [4.5, 1.5, 4.0];
        assert!(
            sizes
                .iter()
                .zip(expected)
                .all(|(size, expected)| (size - expected).abs() < 0.0001),
            "{sizes:?}"
        );
        assert!((sizes.iter().sum::<f64>() - 10.0).abs() < 0.0001);
    }

    #[test]
    fn reordering_siblings_preserves_user_resize_ratios() {
        let mut system = TraditionalLayoutSystem::default();
        let layout = system.create_layout();
        for window in [w(178), w(179), w(180)] {
            system.update(WindowChange::Add(layout, window, None));
        }
        let [n1, n2, n3] = [w(178), w(179), w(180)].map(|window| system.leaf(layout, window));
        system.with_sizes(&[(n1, 5.0), (n2, 2.0), (n3, 1.0)]);
        assert!(system.focus(layout, Focus::Select(w(179))).handled);
        assert!(
            system
                .edit(layout, LayoutEdit::Move {
                    direction: Direction::Right,
                    has_display_neighbor: false,
                })
                .changed()
        );
        use slotmap::Key;
        let [n1, n2, n3] = [n1, n2, n3].map(|node| node.data().as_ffi());
        assert_eq!(children(&system, layout), [(n1, 5.0), (n3, 1.0), (n2, 2.0)]);
    }

    #[test]
    fn native_tabs_reserve_the_configured_edge_without_changing_stack_direction() {
        use crate::common::config::{
            HorizontalPlacement, StackDefaultOrientation, StackLineReserve, VerticalPlacement,
        };
        let rect = |x, y, w, h| CGRect::new(CGPoint::new(x, y), CGSize::new(w, h));
        let cases = [
            (
                LayoutKind::HorizontalStack,
                HorizontalPlacement::Top,
                VerticalPlacement::Left,
                rect(100.0, 229.0, 400.0, 271.0),
            ),
            (
                LayoutKind::HorizontalStack,
                HorizontalPlacement::Bottom,
                VerticalPlacement::Left,
                rect(100.0, 200.0, 400.0, 271.0),
            ),
            (
                LayoutKind::VerticalStack,
                HorizontalPlacement::Top,
                VerticalPlacement::Left,
                rect(129.0, 200.0, 371.0, 300.0),
            ),
            (
                LayoutKind::VerticalStack,
                HorizontalPlacement::Top,
                VerticalPlacement::Right,
                rect(100.0, 200.0, 371.0, 300.0),
            ),
        ];
        for (kind, horiz, vert, expected) in cases {
            let mut system = TraditionalLayoutSystem::default();
            let layout = system.create_layout();
            let (a, b) = (w(170), w(171));
            for window in [a, b] {
                system.update(WindowChange::Add(layout, window, None));
            }
            let orientation = match kind {
                LayoutKind::HorizontalStack => StackDefaultOrientation::Horizontal,
                _ => StackDefaultOrientation::Vertical,
            };
            assert!(
                system.edit(layout, LayoutEdit::Group(GroupEdit::Stack(orientation))).changed()
            );
            assert!(system.focus(layout, Focus::Select(b)).handled);
            let screen = rect(100.0, 200.0, 400.0, 300.0);
            let reserve = StackLineReserve {
                thickness: 28.0,
                spacing: 1.0,
                horiz,
                vert,
            };
            let frames = system.calculate_layout(
                layout,
                screen,
                0.0,
                &HashMap::default(),
                &Default::default(),
                reserve,
                crate::layout_engine::Shadow::Run,
            );
            assert_eq!(frames.len(), 2);
            for (_, frame) in frames {
                assert_eq!(frame, expected, "{kind:?} {horiz:?} {vert:?}");
            }
            // The bar sits on the reserved edge, flush with the stack's outer frame.
            let bar = reserve.bar(screen, kind == LayoutKind::HorizontalStack);
            let expected_bar = match (kind, horiz, vert) {
                (LayoutKind::HorizontalStack, HorizontalPlacement::Top, _) => {
                    rect(100.0, 200.0, 400.0, 28.0)
                }
                (LayoutKind::HorizontalStack, HorizontalPlacement::Bottom, _) => {
                    rect(100.0, 472.0, 400.0, 28.0)
                }
                (_, _, VerticalPlacement::Left) => rect(100.0, 200.0, 28.0, 300.0),
                (_, _, VerticalPlacement::Right) => rect(472.0, 200.0, 28.0, 300.0),
            };
            assert_eq!(bar, expected_bar);
            assert_eq!(system.container_tree(layout).layout_kind, Some(kind));
        }
    }

    /// A traditional system after random edits: windows added and removed, every focus, splits,
    /// joins, stacks and unstacks, consume-or-expel, moves, drops of every kind, resizes, swaps,
    /// rebalances, orientation toggles, fullscreen of a window or a container, participation.
    fn drawn_system(
        rng: &mut crate::layout_engine::systems::drawn::Rng,
    ) -> (TraditionalLayoutSystem, LayoutId, Vec<WindowId>) {
        use crate::common::config::StackDefaultOrientation;
        use crate::layout_engine::systems::drawn::drawn_direction;
        use crate::layout_engine::systems::{GroupEdit, LayoutEdit};
        use crate::layout_engine::{ResizeOrientation, WindowDropAction};
        let insertion = if rng.chance(50) {
            WindowInsertionPoint::NextToSelection
        } else {
            WindowInsertionPoint::EndOfTree
        };
        let mut system = TraditionalLayoutSystem::new(insertion, rng.chance(50));
        let layout = system.create_layout();
        let mut windows: Vec<WindowId> = Vec::new();
        let mut next = 0;
        let stack_orientation =
            |rng: &mut crate::layout_engine::systems::drawn::Rng| match rng.below(4) {
                0 => StackDefaultOrientation::Perpendicular,
                1 => StackDefaultOrientation::Same,
                2 => StackDefaultOrientation::Horizontal,
                _ => StackDefaultOrientation::Vertical,
            };
        for _ in 0..rng.below(40) {
            let pick = |rng: &mut crate::layout_engine::systems::drawn::Rng,
                        windows: &[WindowId]| {
                windows[rng.below(windows.len() as u64) as usize]
            };
            match rng.below(18) {
                0..=3 => {
                    next += 1;
                    let window = WindowId::new(1 + rng.below(3) as i32, next);
                    windows.push(window);
                    system.update(WindowChange::Add(layout, window, None));
                }
                _ if windows.is_empty() => {}
                4 => {
                    let window = pick(rng, &windows);
                    windows.retain(|w| *w != window);
                    system.update(WindowChange::Remove(window));
                }
                5 => {
                    let focus = match rng.below(4) {
                        0 => Focus::Select(pick(rng, &windows)),
                        1 => Focus::Ascend,
                        2 => Focus::Descend,
                        _ => Focus::Direction(drawn_direction(rng)),
                    };
                    system.focus(layout, focus);
                }
                6 => {
                    for _ in 0..rng.below(3) {
                        system.focus(layout, Focus::Ascend);
                    }
                    let within_gaps = rng.chance(50);
                    system.edit(layout, LayoutEdit::Fullscreen { within_gaps });
                }
                7 => {
                    let window = pick(rng, &windows);
                    system.set_window_participation(window, rng.chance(40));
                }
                8 => {
                    let direction = drawn_direction(rng);
                    let has_display_neighbor = rng.chance(30);
                    system.edit(layout, LayoutEdit::Move {
                        direction,
                        has_display_neighbor,
                    });
                }
                9 => {
                    let (source, target) = (pick(rng, &windows), pick(rng, &windows));
                    let action = match rng.below(5) {
                        0 => WindowDropAction::Swap,
                        1 => WindowDropAction::Stack,
                        2 => WindowDropAction::Insert(drawn_direction(rng)),
                        3 => WindowDropAction::Move(drawn_direction(rng)),
                        _ => WindowDropAction::ConsumeOrExpel(drawn_direction(rng)),
                    };
                    system.edit(layout, LayoutEdit::Drop { source, target, action });
                }
                10 => {
                    let amount = rng.length(-0.4, 0.4);
                    let orientation = if rng.chance(50) {
                        ResizeOrientation::Horizontal
                    } else {
                        ResizeOrientation::Vertical
                    };
                    system.edit(layout, LayoutEdit::Resize { amount, orientation }).discard();
                }
                11 => {
                    let (a, b) = (pick(rng, &windows), pick(rng, &windows));
                    system.edit(layout, LayoutEdit::Swap(a, b));
                }
                12 => {
                    let kind = match rng.below(4) {
                        0 => LayoutKind::Horizontal,
                        1 => LayoutKind::Vertical,
                        2 => LayoutKind::HorizontalStack,
                        _ => LayoutKind::VerticalStack,
                    };
                    system.edit(layout, LayoutEdit::Split(kind));
                }
                13 => {
                    let edit = match rng.below(5) {
                        0 => GroupEdit::Join(drawn_direction(rng)),
                        1 => GroupEdit::ConsumeOrExpel(drawn_direction(rng)),
                        2 => GroupEdit::Stack(stack_orientation(rng)),
                        3 => GroupEdit::Unstack(stack_orientation(rng)),
                        _ => GroupEdit::Unjoin,
                    };
                    system.edit(layout, LayoutEdit::Group(edit));
                }
                14 => {
                    system.edit(layout, LayoutEdit::ToggleOrientation);
                }
                15 => {
                    system.edit(layout, LayoutEdit::Rebalance);
                }
                16 => {
                    let (window, target) = (pick(rng, &windows), pick(rng, &windows));
                    system.edit(layout, LayoutEdit::ReorderTab { window, target });
                }
                _ if rng.chance(25) => {
                    let window = pick(rng, &windows);
                    system.update(WindowChange::CloseApp(window.pid));
                    windows.retain(|w| w.pid != window.pid);
                }
                _ => {
                    next += 1;
                    let window = WindowId::new(1 + rng.below(3) as i32, next);
                    windows.push(window);
                    system.update(WindowChange::Add(layout, window, None));
                }
            }
        }
        (system, layout, windows)
    }

    /// rift's frames for a traditional layout are the core forest's shape through the core
    /// geometry, bit for bit (under `roc-core` the Roc forest's, held to them), over drawn
    /// systems, screens, gaps, offsets, bars and window constraints.
    #[test]
    fn the_traditional_system_tiles_its_forests_shape() {
        use crate::layout_engine::systems::drawn::*;
        let mut rng = Rng(0x5eed_7a0d);
        let (mut cases, mut frames_seen) = (0, 0);
        for _ in 0..3000 {
            let (system, layout, windows) = drawn_system(&mut rng);
            let screen = drawn_screen(&mut rng);
            let constraints = drawn_constraints_for(&mut rng, &windows);
            let gaps = drawn_gaps(&mut rng);
            let offset = drawn_offset(&mut rng);
            let line = drawn_line(&mut rng);
            let rift = system.calculate_layout(
                layout,
                screen,
                offset,
                &constraints,
                &gaps,
                line,
                crate::layout_engine::Shadow::Run,
            );
            let core = core_frames(system.shape(layout, screen, offset, &constraints, &gaps, line));
            assert_eq!(bits(&rift), bits(&core));
            cases += 1;
            frames_seen += core.len();
        }
        eprintln!("traditional system frames: {cases} cases, {frames_seen} frames identical");
        assert!(frames_seen > 3000, "the draws reach frames ({frames_seen})");
    }
}
