//! rift's BSP layout system: the core's tree (`rift_core::layout::bsp_tree`) and, under
//! `roc-core`, the Roc tree that decides with the Rust tree its shadow (`system`, `roc_tree`); the
//! saved form and its load (`stored`).

#[cfg(feature = "roc-core")]
pub(super) mod roc_tree;
mod stored;
mod system;
#[cfg(feature = "roc-core")]
pub(crate) use roc_tree::bsp_tree_reseeds;
pub use system::BspLayoutSystem;

#[cfg(test)]
mod tests {
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};

    use super::*;
    use crate::actor::app::WindowId;
    use crate::common::collections::HashMap;
    use crate::layout_engine::systems::{
        Focus, GroupEdit, LayoutEdit, LayoutSystem, WindowChange, WindowLayoutConstraints,
        WindowQuery,
    };
    use crate::layout_engine::{Direction, LayoutId, LayoutKind, ResizeOrientation};

    fn w(idx: u32) -> WindowId { WindowId::new(1, idx) }

    fn tiled(windows: &[u32]) -> (BspLayoutSystem, LayoutId) {
        let mut system = BspLayoutSystem::default();
        let layout = system.create_layout();
        for &window in windows {
            system.update(WindowChange::Add(layout, w(window), None));
        }
        (system, layout)
    }

    fn fullscreen(system: &mut BspLayoutSystem, layout: LayoutId, within_gaps: bool) {
        system.edit(layout, LayoutEdit::Fullscreen { within_gaps }).discard();
    }

    fn calculate_frames(
        system: &BspLayoutSystem,
        layout: LayoutId,
        size: CGSize,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
    ) -> HashMap<WindowId, CGRect> {
        system
            .calculate_layout(
                layout,
                CGRect::new(CGPoint::new(0.0, 0.0), size),
                0.0,
                constraints,
                &Default::default(),
                Default::default(),
                crate::layout_engine::Shadow::Run,
            )
            .into_iter()
            .collect()
    }

    /// A split resized towards its first window gives that window the larger share (what rift
    /// does with a BSP layout, whichever core decides it).
    #[test]
    fn a_resized_split_gives_its_first_window_the_larger_share() {
        let (mut system, layout) = tiled(&[1, 2]);
        let _ = system.focus(layout, Focus::Select(w(1)));
        system
            .edit(layout, LayoutEdit::Resize {
                amount: 0.2,
                orientation: ResizeOrientation::Horizontal,
            })
            .discard();
        let frames =
            calculate_frames(&system, layout, CGSize::new(1000.0, 800.0), &Default::default());
        let rect = |x, w| CGRect::new(CGPoint::new(x, 0.0), CGSize::new(w, 800.0));
        // The ratio is kept as an f32 (0.6), so the first window is 0.6f32 of the width.
        let first = 1000.0 * f64::from(0.6_f32);
        assert_eq!(
            frames,
            HashMap::from_iter([
                (w(1), rect(0.0, first)),
                (w(2), rect(first, 1000.0 - first))
            ])
        );
    }

    #[test]
    fn clone_preserves_arrangement_and_selection() {
        let (mut system, original) = tiled(&[1, 2, 3, 4]);
        system.focus(original, Focus::Select(w(1))).handled;
        system
            .edit(original, LayoutEdit::Resize {
                amount: 0.2,
                orientation: ResizeOrientation::Horizontal,
            })
            .discard();
        system
            .edit(original, LayoutEdit::Resize {
                amount: -0.1,
                orientation: ResizeOrientation::Vertical,
            })
            .discard();
        let size = CGSize::new(1400.0, 900.0);
        let frames = calculate_frames(&system, original, size, &Default::default());
        let cloned = system.clone_layout(original);
        assert_eq!(system.selected_window(cloned), Some(w(1)));
        assert_eq!(
            calculate_frames(&system, cloned, size, &Default::default()),
            frames
        );
        fullscreen(&mut system, original, true);
        let fullscreen_clone = system.clone_layout(original);
        assert_eq!(
            calculate_frames(&system, fullscreen_clone, size, &Default::default()),
            calculate_frames(&system, original, size, &Default::default())
        );
        assert_eq!(
            calculate_frames(&system, cloned, size, &Default::default()),
            frames
        );
    }

    #[test]
    fn moving_into_a_placeholder_promotes_the_old_sibling() {
        let (mut system, layout) = tiled(&[1, 2, 3]);
        assert!(system.focus(layout, Focus::Select(w(1))).handled);
        system.edit(layout, LayoutEdit::Split(LayoutKind::Horizontal)).discard();
        assert!(system.focus(layout, Focus::Select(w(3))).handled);

        assert!(
            system
                .edit(layout, LayoutEdit::Move {
                    direction: Direction::Left,
                    has_display_neighbor: false,
                })
                .changed()
        );

        let frames =
            calculate_frames(&system, layout, CGSize::new(1000.0, 1000.0), &Default::default());
        assert_eq!(system.selected_window(layout), Some(w(3)));
        assert_eq!(frames.len(), 3);
        for (window, x, width) in [
            (w(1), 0.0, 250.0),
            (w(3), 250.0, 250.0),
            (w(2), 500.0, 500.0),
        ] {
            assert_eq!(
                frames[&window],
                CGRect::new(CGPoint::new(x, 0.0), CGSize::new(width, 1000.0))
            );
        }
    }

    #[test]
    fn cloned_stacks_replace_and_remove_all_members() {
        use crate::layout_engine::WindowDropAction::Stack;

        let (mut system, original) = tiled(&[1, 2, 3]);
        assert!(
            system
                .edit(original, LayoutEdit::Drop {
                    source: w(2),
                    target: w(1),
                    action: Stack
                })
                .changed()
        );
        // Snapshot of the former separate topology and stack maps.
        let expected = system.container_tree(original);
        let saved = include_str!("bsp_legacy.ron");
        system = ron::from_str(saved).unwrap();
        assert_eq!(system.container_tree(original), expected);
        assert!(!system.edit(original, LayoutEdit::Swap(w(2), w(2))).changed());
        assert_eq!(system.container_tree(original), expected);
        let cloned = system.clone_layout(original);
        system.update(WindowChange::Replace(w(1), w(4)));
        system.update(WindowChange::Replace(w(2), w(5)));
        for layout in [original, cloned] {
            assert_eq!(system.windows(layout, WindowQuery::Stack(w(4))), vec![
                w(4),
                w(5)
            ]);
            assert_eq!(system.windows(layout, WindowQuery::Visible), vec![w(5), w(3)]);
        }
        system.update(WindowChange::Remove(w(5)));
        for layout in [original, cloned] {
            assert_eq!(system.windows(layout, WindowQuery::Stack(w(4))), vec![w(4)]);
            assert_eq!(system.windows(layout, WindowQuery::Visible), vec![w(4), w(3)]);
        }
        system.update(WindowChange::Replace(w(4), w(3)));
        for layout in [original, cloned] {
            assert_eq!(system.windows(layout, WindowQuery::All), vec![w(3)]);
        }
        system.update(WindowChange::CloseApp(1));
        for layout in [original, cloned] {
            assert!(system.windows(layout, WindowQuery::All).is_empty());
            assert!(system.windows(layout, WindowQuery::Visible).is_empty());
            assert_eq!(system.selected_window(layout), None);
        }
    }

    #[test]
    fn closing_the_last_fullscreen_window_clears_the_tile() {
        let (mut system, layout) = tiled(&[1]);
        fullscreen(&mut system, layout, false);
        assert!(system.container_tree(layout).is_fullscreen);
        system.update(WindowChange::Remove(w(1)));
        assert!(!system.container_tree(layout).is_fullscreen);
        system.update(WindowChange::Add(layout, w(2), None));
        assert!(!system.container_tree(layout).is_fullscreen);
    }

    /// Rebalancing counts the windows along each split's own axis: across alternating splits
    /// every share is even again after a resize.
    #[test]
    fn a_rebalance_evens_the_shares_after_a_resize() {
        let (mut system, layout) = tiled(&[1, 2, 3]);
        let _ = system.focus(layout, Focus::Select(w(1)));
        system
            .edit(layout, LayoutEdit::Resize {
                amount: 0.2,
                orientation: ResizeOrientation::Horizontal,
            })
            .discard();
        system.edit(layout, LayoutEdit::Rebalance).discard();
        let frames =
            calculate_frames(&system, layout, CGSize::new(1200.0, 900.0), &Default::default());
        for (window, x, y, width, height) in [
            (w(1), 0.0, 0.0, 600.0, 900.0),
            (w(2), 600.0, 0.0, 600.0, 450.0),
            (w(3), 600.0, 450.0, 600.0, 450.0),
        ] {
            assert_eq!(
                frames[&window],
                CGRect::new(CGPoint::new(x, y), CGSize::new(width, height))
            );
        }
    }

    /// rift-cli's container view of a BSP layout: a split with each child's share, a window leaf
    /// with its fill, selection and preselection, a stack as a horizontal stack of its windows.
    #[test]
    fn the_container_view_shows_the_trees_nodes() {
        use rift_protocol::{ContainerNodeType, ContainerTreeNode, LayoutKind as Kind};
        let (mut system, layout) = tiled(&[1, 2, 3]);
        system
            .edit(layout, LayoutEdit::Drop {
                source: w(3),
                target: w(2),
                action: crate::layout_engine::WindowDropAction::Stack,
            })
            .discard();
        let _ = system.focus(layout, Focus::Select(w(1)));
        fullscreen(&mut system, layout, true);
        // A preselection only a saved file holds, on the first leaf.
        let file = ron::to_string(&system).unwrap().replacen(
            "preselected:None",
            "preselected:Some(left)",
            1,
        );
        let system: BspLayoutSystem = ron::from_str(&file).unwrap();
        let window = |idx: u32, selected: bool| ContainerTreeNode {
            node_id: (1 << 32) | u64::from(idx),
            node_type: ContainerNodeType::Window,
            window_id: Some(w(idx).into()),
            is_selected: selected,
            ..Default::default()
        };
        let expected = ContainerTreeNode {
            node_id: (1 << 32) | 1,
            layout_kind: Some(Kind::Horizontal),
            children: vec![
                ContainerTreeNode {
                    node_id: (1 << 32) | 2,
                    weight: Some(0.5),
                    is_fullscreen_within_gaps: true,
                    pending_split: Some(rift_protocol::Direction::Left),
                    ..window(1, true)
                },
                ContainerTreeNode {
                    node_id: (1 << 32) | 3,
                    weight: Some(0.5),
                    layout_kind: Some(Kind::HorizontalStack),
                    children: vec![window(2, false), window(3, false)],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert_eq!(system.container_tree(layout), expected);
        // rift-cli's text drawing: each node a line, a leaf's window on top, a split's share.
        assert_eq!(
            system.draw_tree(layout),
            "Split Horizontal 0.50\n  Leaf Some(WindowId { pid: 1, idx: 1 })\n  Leaf \
             Some(WindowId { pid: 1, idx: 3 })\n"
        );
        assert_eq!(system.draw_tree(LayoutId::default()), "<empty bsp>");
        // A share off the middle: each child's weight is its own side's.
        let mut system = system;
        system
            .edit(layout, LayoutEdit::Resize {
                amount: 0.2,
                orientation: ResizeOrientation::Horizontal,
            })
            .discard();
        // The stack selected: its window on top is shown selected, the other not.
        assert!(system.focus(layout, Focus::Select(w(3))).handled);
        let tree = system.container_tree(layout);
        let weights: Vec<f64> = tree.children.iter().map(|child| child.weight.unwrap()).collect();
        assert!(weights[0] > weights[1], "{weights:?}");
        assert_eq!(weights[0] + weights[1], 1.0);
        let stack = &tree.children[1];
        assert!(stack.is_selected);
        let selected: Vec<bool> = stack.children.iter().map(|tab| tab.is_selected).collect();
        assert_eq!(selected, vec![false, true]);
    }

    #[test]
    fn join_adds_the_selected_window_to_the_neighboring_stack() {
        let (mut system, layout) = tiled(&[1, 2, 3]);
        assert!(
            system
                .edit(layout, LayoutEdit::Drop {
                    source: w(2),
                    target: w(1),
                    action: crate::layout_engine::WindowDropAction::Stack,
                })
                .changed()
        );
        assert!(system.focus(layout, Focus::Select(w(3))).handled);
        system
            .edit(layout, LayoutEdit::Group(GroupEdit::Join(Direction::Left)))
            .discard();
        assert_eq!(system.windows(layout, WindowQuery::All), vec![w(1), w(2), w(3)]);
        assert_eq!(system.windows(layout, WindowQuery::Visible), vec![w(3)]);
    }
}

#[cfg(test)]
mod core_geometry_tests {
    use objc2_core_foundation::CGRect;

    use super::*;
    use crate::actor::app::WindowId;
    use crate::common::config::WindowInsertionPoint;
    use crate::layout_engine::systems::drawn::*;
    use crate::layout_engine::systems::{
        Focus, GroupEdit, LayoutEdit, LayoutSystem, WindowChange, WindowQuery,
    };
    use crate::layout_engine::{
        Direction, LayoutId, LayoutKind, ResizeOrientation, WindowDropAction,
    };

    /// A BSP system after random edits: windows added and removed, closed apps, every focus,
    /// splits of every kind, joins, unjoins, consume-or-expel, moves, drops of every kind (stacks
    /// among them), resizes, swaps, rebalances, orientation toggles, fullscreen of a leaf or a
    /// split, participation, a single window's aspect ratio; layouts created, cloned and removed,
    /// windows replaced, native resizes into and out of fullscreen, the settings changed, every
    /// query asked, and saves and loads (under `roc-core` every operation also held to the Roc
    /// tree, and the Rust tree re-seeded from it now and then).
    pub(super) fn drawn_system(rng: &mut Rng) -> (BspLayoutSystem, LayoutId, Vec<WindowId>) {
        let mut insertion = drawn_insertion(rng);
        let mut system = BspLayoutSystem::new(insertion);
        let mut aspect = None;
        if rng.chance(30) {
            aspect = Some(rng.length(0.3, 3.0));
            system.set_single_window_aspect_ratio(aspect);
        }
        let layout = system.create_layout();
        let mut others: Vec<LayoutId> = Vec::new();
        let mut windows: Vec<WindowId> = Vec::new();
        let mut next = 0;
        for _ in 0..rng.below(40) {
            let pick = |rng: &mut Rng, windows: &[WindowId]| {
                windows[rng.below(windows.len() as u64) as usize]
            };
            // Most edits go to the layout drawn for frames, some to the others.
            let layout = if !others.is_empty() && rng.chance(15) {
                others[rng.below(others.len() as u64) as usize]
            } else {
                layout
            };
            if rng.chance(35) {
                asked(rng, &system, layout, &windows);
            }
            if rng.chance(12) {
                match rng.below(6) {
                    0 => others.push(system.create_layout()),
                    1 | 2 => others.push(system.clone_layout(layout)),
                    3 if !others.is_empty() => {
                        let removed = others.swap_remove(rng.below(others.len() as u64) as usize);
                        system.remove_layout(removed);
                    }
                    3 => system.remove_layout(LayoutId::default()),
                    4 => {
                        insertion = drawn_insertion(rng);
                        system.set_window_insertion_point(insertion);
                    }
                    _ => {
                        aspect = [None, Some(rng.length(0.3, 3.0)), Some(0.0), Some(f64::NAN)]
                            [rng.below(4) as usize];
                        system.set_single_window_aspect_ratio(aspect);
                    }
                }
                continue;
            }
            #[cfg(feature = "roc-core")]
            if rng.chance(4) {
                // The Rust tree re-seeded from the deciding Roc tree (as after a divergence):
                // the trees go on alike.
                system.core = roc_tree::core_of(&system.roc.view());
            }
            if rng.chance(4) {
                // Saved and loaded (the settings are not saved: rift sets them again), some
                // leaf maybe given a preselected direction in the file, which only a saved file
                // holds.
                let mut saved = ron::to_string(&system).unwrap();
                let leaves = saved.matches("preselected:None").count();
                if leaves > 0 && rng.chance(70) {
                    let leaf = rng.below(leaves as u64) as usize;
                    let at = saved.match_indices("preselected:None").nth(leaf).unwrap().0;
                    let direction = match drawn_direction(rng) {
                        Direction::Left => "left",
                        Direction::Right => "right",
                        Direction::Up => "up",
                        Direction::Down => "down",
                    };
                    saved.replace_range(
                        at..at + "preselected:None".len(),
                        &format!("preselected:Some({direction})"),
                    );
                }
                system = ron::from_str(&saved).unwrap();
                system.set_window_insertion_point(insertion);
                system.set_single_window_aspect_ratio(aspect);
                continue;
            }
            if !windows.is_empty() && rng.chance(8) {
                let from = pick(rng, &windows);
                let to = if rng.chance(50) {
                    pick(rng, &windows)
                } else {
                    next += 1;
                    WindowId::new(from.pid, next)
                };
                windows.retain(|w| *w != from && *w != to);
                windows.push(to);
                system.update(WindowChange::Replace(from, to));
                continue;
            }
            if !windows.is_empty() && rng.chance(8) {
                let window = pick(rng, &windows);
                let screen = drawn_screen(rng);
                let gaps = drawn_gaps(rng);
                let tiling = crate::layout_engine::utils::compute_tiling_area(screen, &gaps);
                let frame = |rng: &mut Rng| match rng.below(4) {
                    0 => screen,
                    1 => tiling,
                    _ => drawn_screen(rng),
                };
                let old = frame(rng);
                let new = if rng.chance(25) {
                    // A nudge either side of the half-point a resize must exceed.
                    let mut new = old;
                    new.size.width += [0.3, 0.5, 0.7, -0.4][rng.below(4) as usize];
                    new.size.height += [0.0, 0.5, 0.6][rng.below(3) as usize];
                    new
                } else {
                    frame(rng)
                };
                system.update(WindowChange::Resize {
                    layout,
                    window,
                    old,
                    new,
                    screen,
                    gaps: &gaps,
                });
                continue;
            }
            match rng.below(17) {
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
                    let window = if rng.chance(15) {
                        // A window in no layout (participation is the system's, not a layout's).
                        next += 1;
                        WindowId::new(1 + rng.below(3) as i32, next)
                    } else {
                        pick(rng, &windows)
                    };
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
                        1 | 2 => WindowDropAction::Stack,
                        3 => WindowDropAction::Insert(drawn_direction(rng)),
                        _ => WindowDropAction::Move(drawn_direction(rng)),
                    };
                    let drop = LayoutEdit::Drop { source, target, action };
                    if rng.chance(30) {
                        // Past `edit`'s participation check, which the system makes again.
                        system.edit_layout(layout, drop);
                    } else {
                        system.edit(layout, drop);
                    }
                }
                10 => {
                    let amount = rng.length(-0.6, 0.6);
                    let orientation = match rng.below(3) {
                        0 => ResizeOrientation::Horizontal,
                        1 => ResizeOrientation::Vertical,
                        _ => ResizeOrientation::Smart,
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
                    let edit = match rng.below(8) {
                        0 | 1 => GroupEdit::Join(drawn_direction(rng)),
                        2 | 3 => GroupEdit::ConsumeOrExpel(drawn_direction(rng)),
                        4 | 5 => GroupEdit::Unjoin,
                        // Edits upstream's BSP system has no case for.
                        6 => GroupEdit::Stack(crate::common::config::StackDefaultOrientation::Same),
                        _ => {
                            let (window, target) = (pick(rng, &windows), pick(rng, &windows));
                            system.edit(layout, LayoutEdit::ReorderTab { window, target });
                            continue;
                        }
                    };
                    system.edit(layout, LayoutEdit::Group(edit));
                }
                14 => {
                    system.edit(layout, LayoutEdit::ToggleOrientation);
                }
                15 => {
                    system.edit(layout, LayoutEdit::Rebalance);
                }
                _ if rng.chance(15) => {
                    // Every window stacked onto one: the layout is a single stacked leaf.
                    let target = windows[0];
                    for &source in &windows[1..] {
                        system.edit(layout, LayoutEdit::Drop {
                            source,
                            target,
                            action: WindowDropAction::Stack,
                        });
                    }
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

    fn drawn_insertion(rng: &mut Rng) -> WindowInsertionPoint {
        if rng.chance(50) {
            WindowInsertionPoint::NextToSelection
        } else {
            WindowInsertionPoint::EndOfTree
        }
    }

    /// Every query, about drawn windows (some not in the layout).
    fn asked(rng: &mut Rng, system: &BspLayoutSystem, layout: LayoutId, windows: &[WindowId]) {
        let window = match windows {
            [] => WindowId::new(1, 9999),
            _ => windows[rng.below(windows.len() as u64) as usize],
        };
        let _ = system.selected_window(layout);
        let _ = system.contains_window(layout, window);
        let _ = system.has_any_fullscreen_node(layout);
        let _ = system.has_windows_for_app(layout, window.pid);
        let _ = system.window_participates(window);
        let _ = system.contains_layout(layout);
        for query in [
            WindowQuery::All,
            WindowQuery::Visible,
            WindowQuery::Selection,
            WindowQuery::Stack(window),
        ] {
            let _ = system.windows(layout, query);
        }
        for direction in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ] {
            let _ = system.window_in_direction(layout, direction);
            for &window in windows {
                let _ = system.can_move_window(layout, window, direction);
            }
        }
        let _ = system.window_slot(layout, window);
        // rift-cli's views of the layout (and of a layout removed or never made).
        for layout in [layout, LayoutId::default()] {
            let _ = system.draw_tree(layout);
            let _ = system.container_tree(layout);
        }
    }

    /// rift's frames for a BSP layout are the core tree's shape through the core's geometry, bit
    /// for bit, over drawn systems, screens, gaps and window constraints (under `roc-core` the
    /// Roc tree's frames decide, held to the Rust tree's by the pass itself).
    #[test]
    fn the_bsp_system_tiles_its_trees_shape() {
        use crate::layout_engine::core_geometry::core_constraints;
        use crate::layout_engine::{cg_rect, core_gaps, core_rect};
        let mut rng = Rng(0x5eed_b5b5);
        let (mut cases, mut frames_seen) = (0, 0);
        for _ in 0..3000 {
            let (system, layout, windows) = drawn_system(&mut rng);
            let screen = drawn_screen(&mut rng);
            let constraints = drawn_constraints_for(&mut rng, &windows);
            let gaps = drawn_gaps(&mut rng);
            // A BSP layout has no bar: the line is drawn only for the pass's arguments.
            let line = drawn_line(&mut rng);
            let given = system.calculate_layout(
                layout,
                screen,
                0.0,
                &constraints,
                &gaps,
                line,
                crate::layout_engine::Shadow::Run,
            );
            let shaped: Vec<(WindowId, CGRect)> = system
                .core
                .shape(layout, core_rect(screen), core_gaps(&gaps), |window| {
                    constraints.get(&window).copied().map(core_constraints)
                })
                .map_or_else(Vec::new, |shape| shape.frames())
                .into_iter()
                .map(|(window, frame)| (window, cg_rect(frame)))
                .collect();
            assert_eq!(bits(&given), bits(&shaped));
            cases += 1;
            frames_seen += given.len();
        }
        eprintln!("bsp: {cases} layouts, {frames_seen} frames identical");
        assert!(frames_seen > 3000, "the draws reach frames ({frames_seen})");
    }
}
