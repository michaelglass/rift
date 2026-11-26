mod system;
#[cfg(feature = "roc-core")]
pub(super) use system::CoreMasterStack;
pub use system::MasterStackLayoutSystem;

#[cfg(test)]
mod tests {
    use objc2_core_foundation::CGRect;

    use super::*;
    use crate::actor::app::WindowId;
    use crate::common::config::{
        MasterStackNewWindowPlacement, MasterStackSettings, MasterStackSide, WindowInsertionPoint,
    };
    use crate::layout_engine::systems::drawn::*;
    use crate::layout_engine::systems::{
        Focus, LayoutEdit, LayoutSystem, WindowChange, WindowQuery,
    };
    use crate::layout_engine::{Direction, LayoutId, Orientation, ResizeOrientation};

    fn w(idx: u32) -> WindowId { WindowId::new(1, idx) }

    /// A master and two stacked windows: the master on the left, the stack's windows sharing
    /// the right, the inner gaps between them (what rift does with a master-stack layout,
    /// whichever core decides it).
    #[test]
    fn a_master_stack_layout_keeps_its_inner_gaps() {
        use objc2_core_foundation::{CGPoint, CGSize};
        let mut system = MasterStackLayoutSystem::default();
        let layout = system.create_layout();
        for window in [w(1), w(2), w(3)] {
            system.update(WindowChange::Add(layout, window, None));
        }
        let mut gaps = crate::common::config::GapSettings::default();
        gaps.inner.horizontal = 10.0;
        gaps.inner.vertical = 6.0;
        let frames = system.calculate_layout(
            layout,
            CGRect::new(CGPoint::ZERO, CGSize::new(1000.0, 800.0)),
            0.0,
            &Default::default(),
            &gaps,
            Default::default(),
            crate::layout_engine::Shadow::Run,
        );
        let rect = |x, y, w, h| CGRect::new(CGPoint::new(x, y), CGSize::new(w, h));
        // The newest window is the master: 60% of the width less the gap, then the 10-point
        // gap, then the stack, its two windows 6 points apart.
        assert_eq!(frames, vec![
            (w(3), rect(0.0, 0.0, 594.0, 800.0)),
            (w(2), rect(604.0, 0.0, 396.0, 397.0)),
            (w(1), rect(604.0, 403.0, 396.0, 397.0)),
        ]);
    }

    #[test]
    fn region_identity_and_grouping_survive_removal_and_settings_reload() {
        use crate::layout_engine::WindowDropAction;
        let mut system = MasterStackLayoutSystem::default();
        let layout = system.create_layout();
        for window in [w(1), w(2), w(3)] {
            system.update(WindowChange::Add(layout, window, None));
        }
        assert!(
            system
                .edit(layout, LayoutEdit::Drop {
                    source: w(3),
                    target: w(2),
                    action: WindowDropAction::Stack
                })
                .changed()
        );
        system.focus(layout, Focus::Select(w(1))).handled;
        system
            .edit(layout, LayoutEdit::Resize {
                amount: 0.1,
                orientation: ResizeOrientation::Horizontal,
            })
            .discard();
        let before = system.container_tree(layout);
        let master_weight = before.children[0].weight;
        let master = before
            .children
            .iter()
            .find(|node| node.role.as_deref() == Some("master"))
            .unwrap()
            .node_id;
        let stack = before
            .children
            .iter()
            .find(|node| node.role.as_deref() == Some("stack"))
            .unwrap()
            .node_id;
        let group =
            before.children.iter().find(|node| node.node_id == stack).unwrap().children[0].node_id;
        system.update_settings(MasterStackSettings {
            master_side: MasterStackSide::Right,
            ..MasterStackSettings::default()
        });
        let after = system.container_tree(layout);
        assert_eq!(after.children[0].node_id, stack);
        assert_eq!(after.children[1].node_id, master);
        assert_eq!(after.children[1].weight, master_weight);
        assert!(after.children[1].children[0].is_selected);
        assert_eq!(after.children[0].children[0].node_id, group);
        system.update(WindowChange::Remove(w(3)));
        system.update(WindowChange::Remove(w(2)));
        let after = system.container_tree(layout);
        assert_eq!(after.children[0].node_id, stack);
        assert_eq!(after.children[1].node_id, master);
        let serialized = ron::to_string(&system).unwrap();
        let restored: MasterStackLayoutSystem = ron::from_str(&serialized).unwrap();
        assert_eq!(restored.container_tree(layout), after);
    }

    #[test]
    fn master_count_moves_a_complete_nested_group_without_flattening_it() {
        use crate::layout_engine::WindowDropAction;
        let mut system = MasterStackLayoutSystem::default();
        let layout = system.create_layout();
        for window in [w(1), w(2), w(3), w(4)] {
            system.update(WindowChange::Add(layout, window, None));
        }
        for (source, action) in [
            (w(3), WindowDropAction::Stack),
            (w(4), WindowDropAction::Insert(Direction::Right)),
        ] {
            assert!(
                system.edit(layout, LayoutEdit::Drop { source, target: w(2), action }).changed()
            );
        }
        system.focus(layout, Focus::Select(w(2))).handled;
        let before = system.container_tree(layout);
        let group = &before.children[1].children[0];
        assert!(group.children.iter().any(|child| child.children.len() == 2));

        system.adjust_master_count(layout, 3);

        let after = system.container_tree(layout);
        assert_eq!(after.children[0].node_id, before.children[0].node_id);
        assert_eq!(after.children[1].node_id, before.children[1].node_id);
        assert!(after.children[1].children.is_empty());
        let moved = after.children[0]
            .children
            .iter()
            .find(|child| child.node_id == group.node_id)
            .expect("the complete group moved into the master region");
        assert_eq!(moved.children, group.children);
        assert_eq!(system.selected_window(layout), Some(w(2)));
    }

    #[test]
    fn promotion_demotes_a_window_from_the_last_nested_master_group() {
        use crate::layout_engine::WindowDropAction;
        let settings = MasterStackSettings {
            master_count: 3,
            new_window_placement: MasterStackNewWindowPlacement::Stack,
            ..Default::default()
        };
        let mut system = MasterStackLayoutSystem::new(settings);
        let layout = system.create_layout();
        for index in 1..=5 {
            system.update(WindowChange::Add(layout, w(index), None));
        }
        for source in [w(2), w(1)] {
            assert!(
                system
                    .edit(layout, LayoutEdit::Drop {
                        source,
                        target: w(3),
                        action: WindowDropAction::Stack,
                    })
                    .changed()
            );
        }
        let before = system.container_tree(layout);
        let [group] = &before.children[0].children[..] else {
            panic!("expected one grouped master child");
        };
        assert_eq!(group.children.len(), 3);

        assert!(system.focus(layout, Focus::Select(w(4))).handled);
        system.promote_to_master(layout);

        let in_master = |window| system.window_slot(layout, window).unwrap().first() == Some(&0);
        assert_eq!((1..=5).map(w).filter(|&window| in_master(window)).count(), 3);
        assert_eq!(system.window_slot(layout, w(4)), Some(vec![0, 0]));
        assert_eq!(system.selected_window(layout), Some(w(4)));
        let after = system.container_tree(layout);
        assert_eq!(after.children[0].children[1].node_id, group.node_id);
        assert_eq!(after.children[0].children[1].children.len(), 2);
    }

    #[test]
    fn keyboard_move_across_regions_skips_suspended_edge_windows() {
        for (master_side, direction, edge, target) in [
            (MasterStackSide::Left, Direction::Right, w(2), w(3)),
            (MasterStackSide::Right, Direction::Left, w(3), w(2)),
        ] {
            let mut settings = MasterStackSettings {
                master_side,
                new_window_placement: MasterStackNewWindowPlacement::Stack,
                ..Default::default()
            };
            settings.base.window_insertion_point = Some(WindowInsertionPoint::EndOfTree);
            let mut system = MasterStackLayoutSystem::new(settings);
            let layout = system.create_layout();
            for window in [w(1), w(2), w(3)] {
                system.update(WindowChange::Add(layout, window, None));
            }
            assert!(system.focus(layout, Focus::Select(w(1))).handled);
            let target_slot = system.window_slot(layout, target);
            let edge_slot = system.window_slot(layout, edge);
            system.set_window_participation(edge, false);
            assert!(
                system
                    .edit(layout, LayoutEdit::Move {
                        direction,
                        has_display_neighbor: false
                    })
                    .changed(),
                "{master_side:?}"
            );
            assert_eq!(system.window_slot(layout, w(1)), target_slot);
            assert_eq!(system.window_slot(layout, edge), edge_slot);
        }
    }

    #[test]
    fn keyboard_move_uses_the_region_created_by_a_stack_drop() {
        use crate::layout_engine::WindowDropAction;
        let mut system = MasterStackLayoutSystem::default();
        let layout = system.create_layout();
        for window in [w(1), w(2), w(3)] {
            system.update(WindowChange::Add(layout, window, None));
        }
        assert!(
            system
                .edit(layout, LayoutEdit::Drop {
                    source: w(2),
                    target: w(3),
                    action: WindowDropAction::Stack,
                })
                .changed()
        );
        assert!(
            system
                .edit(layout, LayoutEdit::Move {
                    direction: Direction::Up,
                    has_display_neighbor: false,
                })
                .changed()
        );
        assert_eq!(system.windows(layout, WindowQuery::Stack(w(2))), vec![
            w(2),
            w(3)
        ]);
        assert_eq!(system.selected_window(layout), Some(w(2)));
    }

    fn drawn_direction(rng: &mut Rng) -> Direction {
        match rng.below(4) {
            0 => Direction::Left,
            1 => Direction::Right,
            2 => Direction::Up,
            _ => Direction::Down,
        }
    }

    fn drawn_settings(rng: &mut Rng) -> MasterStackSettings {
        let orientation = |rng: &mut Rng| match rng.below(3) {
            0 => None,
            1 => Some(Orientation::Horizontal),
            _ => Some(Orientation::Vertical),
        };
        MasterStackSettings {
            master_ratio: if rng.chance(30) {
                0.5
            } else {
                rng.length(0.05, 0.95)
            },
            master_count: 1 + rng.below(3) as usize,
            master_side: match rng.below(4) {
                0 => MasterStackSide::Left,
                1 => MasterStackSide::Right,
                2 => MasterStackSide::Top,
                _ => MasterStackSide::Bottom,
            },
            new_window_placement: match rng.below(3) {
                0 => MasterStackNewWindowPlacement::Master,
                1 => MasterStackNewWindowPlacement::Stack,
                _ => MasterStackNewWindowPlacement::Focused,
            },
            master_arrangement: orientation(rng),
            stack_arrangement: orientation(rng),
            ..MasterStackSettings::default()
        }
    }

    /// A master-stack system after random edits: windows added and removed, selection moved,
    /// windows moved, swapped, dropped on each other (stacked among them), resized, made
    /// fullscreen and left out of the layout, the settings changed.
    fn drawn_system(rng: &mut Rng) -> (MasterStackLayoutSystem, LayoutId, Vec<WindowId>) {
        use crate::layout_engine::{ResizeOrientation, WindowDropAction};
        let mut system = MasterStackLayoutSystem::new(drawn_settings(rng));
        let layout = system.create_layout();
        let mut windows: Vec<WindowId> = Vec::new();
        let mut next = 0;
        for _ in 0..rng.below(32) {
            let pick = |rng: &mut Rng, windows: &[WindowId]| {
                windows[rng.below(windows.len() as u64) as usize]
            };
            match rng.below(14) {
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
                    // Sometimes the whole layout (its root selected), sometimes a window.
                    if rng.chance(40) {
                        for _ in 0..3 {
                            system.focus(layout, Focus::Ascend);
                        }
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
                    let action = match rng.below(4) {
                        0 => WindowDropAction::Swap,
                        1 => WindowDropAction::Stack,
                        2 => WindowDropAction::Insert(drawn_direction(rng)),
                        _ => WindowDropAction::Move(drawn_direction(rng)),
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
                    system.update_settings(drawn_settings(rng));
                }
                _ => {
                    system.edit(layout, LayoutEdit::Rebalance);
                }
            }
        }
        (system, layout, windows)
    }

    /// rift's frames for a master-stack layout are the core forest's shape through the core
    /// geometry, bit for bit (under `roc-core` the Roc forest's, held to them), over drawn
    /// systems, screens, gaps, offsets, bars and window constraints.
    #[test]
    fn the_master_stack_system_tiles_its_forests_shape() {
        let mut rng = Rng(0x5eed_3a57);
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
        eprintln!("master-stack: {cases} layouts, {frames_seen} frames identical");
        assert!(frames_seen > 3000, "the draws reach frames ({frames_seen})");
    }
}
