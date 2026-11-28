//! The stack layout system (upstream's, #688; a stack owns an ordered list): the core's stacks
//! decide ([`StackLayoutSystem`], `system`), loaded through the saved form (`stored`); under
//! `roc-core` the Roc stacks decide (`roc_stacks`).

#[cfg(feature = "roc-core")]
mod roc_stacks;
mod stored;
mod system;
#[cfg(feature = "roc-core")]
pub(crate) use roc_stacks::stack_tree_reseeds;
pub(crate) use system::Snapshot;
pub use system::StackLayoutSystem;

#[cfg(test)]
mod tests {
    use objc2_core_foundation::CGRect;
    use serde::Serialize;
    use slotmap::{Key, KeyData};

    use super::*;
    use crate::actor::app::WindowId;
    use crate::common::collections::HashMap;
    use crate::common::config::{
        GapSettings, StackDefaultOrientation, StackLineReserve, WindowInsertionPoint,
    };
    use crate::layout_engine::engine::GroupContainerInfo;
    use crate::layout_engine::systems::drawn::*;
    use crate::layout_engine::systems::{
        Focus, GroupEdit, LayoutEdit, LayoutSystem, TraditionalLayoutSystem, WindowChange,
        WindowLayoutConstraints, WindowQuery,
    };
    use crate::layout_engine::utils::compute_tiling_area;
    use crate::layout_engine::{Direction, LayoutId, LayoutKind};
    use crate::model::tree::NodeId;

    fn w(idx: u32) -> WindowId { WindowId::new(1, idx) }

    #[test]
    fn moving_past_the_stack_edge_leaves_the_move_to_the_engine() {
        let mut system = StackLayoutSystem::new(StackDefaultOrientation::Horizontal);
        let layout = system.create_layout();
        let first = WindowId::new(1, 1);
        let last = WindowId::new(1, 2);
        for window in [first, last] {
            system.update(WindowChange::Add(layout, window, None));
        }
        for (direction, has_display_neighbor) in [(Direction::Right, false), (Direction::Up, true)]
        {
            assert!(
                !system
                    .edit(layout, LayoutEdit::Move {
                        direction,
                        has_display_neighbor
                    })
                    .changed()
            );
        }
        assert_eq!(system.windows(layout, WindowQuery::All), [first, last]);
        assert_eq!(system.selected_window(layout), Some(last));
    }

    #[test]
    fn legacy_stack_restoration_preserves_identity_selection_and_fullscreen() {
        #[derive(Serialize)]
        struct Legacy<'a> {
            inner: &'a TraditionalLayoutSystem,
            default_orientation: StackDefaultOrientation,
        }
        let mut inner = TraditionalLayoutSystem::default();
        let removed = inner.create_layout();
        let layout = inner.create_layout();
        inner.remove_layout(removed);
        for window in [w(1), w(2), w(3)] {
            inner.update(WindowChange::Add(layout, window, None));
        }
        inner.focus(layout, Focus::Select(w(2))).handled;
        // A share other than 1 (resized while the root was tiled), which the load keeps as the
        // member's weight.
        inner
            .edit(layout, LayoutEdit::Resize {
                amount: 0.2,
                orientation: crate::layout_engine::ResizeOrientation::Horizontal,
            })
            .discard();
        // The root made a horizontal stack (the selection's parent is the root).
        assert!(
            inner
                .edit(
                    layout,
                    LayoutEdit::Group(GroupEdit::Stack(StackDefaultOrientation::Horizontal))
                )
                .changed()
        );
        assert!(inner.focus(layout, Focus::Select(w(2))).handled);
        inner.edit(layout, LayoutEdit::Fullscreen { within_gaps: true }).into_windows();
        let expected = inner.container_tree(layout);
        assert!(
            expected.children.iter().any(|child| child.weight != Some(1.0)),
            "{expected:?}"
        );
        let legacy = Legacy {
            inner: &inner,
            default_orientation: StackDefaultOrientation::Horizontal,
        };
        // The legacy form as the old tree wrote it, frozen in `stack_legacy.ron` (the live text
        // is held to it while the traditional system still writes that form).
        let file = include_str!("stack/stack_legacy.ron").trim_end();
        assert_eq!(ron::to_string(&legacy).unwrap(), file);
        let mut restored = ron::from_str::<StackLayoutSystem>(file).unwrap();
        {
            assert_eq!(restored.container_tree(layout), expected);
            // The counter resumes past every node the legacy forest saved for the layout.
            let highest = std::iter::once(expected.node_id)
                .chain(expected.children.iter().map(|child| child.node_id))
                .map(|id| id as u32)
                .max()
                .unwrap();
            let probe = restored.clone_layout(layout);
            let minted = restored.container_tree(probe).node_id;
            assert_eq!(minted, (1 << 32) | u64::from(highest + 1));
            restored.remove_layout(probe);
            assert_eq!(restored.selected_window(layout), Some(w(2)));
            let clone = restored.clone_layout(layout);
            assert_eq!(restored.selected_window(clone), Some(w(2)));
            assert!(restored.has_any_fullscreen_node(clone));
            restored.update(WindowChange::Remove(w(2)));
            assert_eq!(restored.selected_window(layout), Some(w(3)));
            let new_layout = restored.create_layout();
            assert!(!restored.contains_layout(removed));
            let reused = [clone, new_layout]
                .into_iter()
                .find(|layout| layout.data().as_ffi() as u32 == removed.data().as_ffi() as u32)
                .expect("a vacant saved slot must be reused");
            assert_ne!(reused, removed);
            assert_ne!(restored.container_tree(new_layout).node_id, expected.node_id);
        }
    }

    #[test]
    fn selecting_the_stack_retains_focus_history_and_presents_its_first_member() {
        let mut system = StackLayoutSystem::default();
        let layout = system.create_layout();
        for window in [w(1), w(2), w(3)] {
            system.update(WindowChange::Add(layout, window, None));
        }
        assert!(system.focus(layout, Focus::Ascend).handled);
        assert_eq!(system.selected_window(layout), Some(w(3)));
        assert_eq!(system.windows(layout, WindowQuery::Visible), vec![w(1)]);
        assert_eq!(system.window_in_direction(layout, Direction::Left), Some(w(3)));
        assert_eq!(system.window_in_direction(layout, Direction::Right), Some(w(1)));
        assert!(system.focus(layout, Focus::Descend).handled);
        assert_eq!(system.windows(layout, WindowQuery::Visible), vec![w(3)]);
        assert!(
            system
                .edit(layout, LayoutEdit::Move {
                    direction: Direction::Left,
                    has_display_neighbor: false
                })
                .changed()
        );
        assert_eq!(system.windows(layout, WindowQuery::All), vec![w(3), w(1), w(2)]);
        assert_eq!(
            system.container_tree(layout).layout_kind,
            Some(LayoutKind::HorizontalStack)
        );
        let survivor = WindowId::new(2, 1);
        system.update(WindowChange::Add(layout, survivor, None));
        system.focus(layout, Focus::Select(w(1)));
        system.update(WindowChange::CloseApp(1));
        assert_eq!(system.windows(layout, WindowQuery::All), vec![survivor]);
        assert_eq!(system.selected_window(layout), Some(survivor));
    }

    #[test]
    fn fixed_window_caps_match_traditional_when_minimum_exceeds_locked_size() {
        use objc2_core_foundation::{CGPoint, CGSize};
        let mut system = StackLayoutSystem::default();
        let layout = system.create_layout();
        system.update(WindowChange::Add(layout, w(1), None));
        system.update(WindowChange::Add(layout, w(2), None));
        let constraints = HashMap::from_iter([(w(1), WindowLayoutConstraints {
            locked_width: 200.0,
            min_width: 300.0,
            ..Default::default()
        })]);
        let frames = system.calculate_layout(
            layout,
            CGRect::new(CGPoint::ZERO, CGSize::new(1000.0, 800.0)),
            12.0,
            &constraints,
            &GapSettings::default(),
            StackLineReserve::default(),
            crate::layout_engine::Shadow::Run,
        );
        assert_eq!(frames[0].1.size.width, 200.0);
    }

    /// A stack of two windows leaves room for its bar and offsets each window behind the first
    /// (what rift does with a stack layout, whichever core decides it).
    #[test]
    fn a_stack_reserves_its_bar_and_offsets_each_window() {
        use objc2_core_foundation::{CGPoint, CGSize};
        let mut system = StackLayoutSystem::default();
        let layout = system.create_layout();
        system.update(WindowChange::Add(layout, w(1), None));
        system.update(WindowChange::Add(layout, w(2), None));
        let line = StackLineReserve {
            thickness: 20.0,
            spacing: 4.0,
            horiz: crate::common::config::HorizontalPlacement::Top,
            vert: crate::common::config::VerticalPlacement::Left,
        };
        let frames = system.calculate_layout(
            layout,
            CGRect::new(CGPoint::ZERO, CGSize::new(1000.0, 800.0)),
            12.0,
            &HashMap::default(),
            &GapSettings::default(),
            line,
            crate::layout_engine::Shadow::Run,
        );
        let rect = |x, y, w, h| CGRect::new(CGPoint::new(x, y), CGSize::new(w, h));
        // A vertical stack: the bar (20 + 4) on the left, each window 12 points below the one
        // before and 12 shorter than the screen.
        assert_eq!(frames, vec![
            (w(1), rect(24.0, 0.0, 976.0, 788.0)),
            (w(2), rect(24.0, 12.0, 976.0, 788.0))
        ]);
    }

    /// A horizontal stack of four windows, the second selected, the fourth left out.
    fn four_with_one_left_out() -> (StackLayoutSystem, LayoutId) {
        let mut system = StackLayoutSystem::new(StackDefaultOrientation::Horizontal);
        let layout = system.create_layout();
        for window in [w(1), w(2), w(3), w(4)] {
            system.update(WindowChange::Add(layout, window, None));
        }
        system.focus(layout, Focus::Select(w(2)));
        system.set_window_participation(w(4), false);
        (system, layout)
    }

    fn screen_and_gaps() -> (CGRect, GapSettings) {
        use objc2_core_foundation::{CGPoint, CGSize};
        let screen = CGRect::new(CGPoint::new(0.0, 25.0), CGSize::new(1000.0, 800.0));
        let mut gaps = GapSettings::default();
        gaps.outer.top = 10.0;
        gaps.outer.left = 20.0;
        gaps.outer.bottom = 30.0;
        gaps.outer.right = 40.0;
        (screen, gaps)
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect {
        use objc2_core_foundation::{CGPoint, CGSize};
        CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
    }

    fn frames(system: &StackLayoutSystem, layout: LayoutId) -> Vec<(WindowId, CGRect)> {
        let (screen, gaps) = screen_and_gaps();
        system.calculate_layout(
            layout,
            screen,
            12.0,
            &HashMap::default(),
            &gaps,
            StackLineReserve::default(),
            crate::layout_engine::Shadow::Run,
        )
    }

    fn group(
        system: &StackLayoutSystem,
        layout: LayoutId,
        path_only: bool,
    ) -> Vec<GroupContainerInfo> {
        let (screen, gaps) = screen_and_gaps();
        system.collect_group_containers(
            layout,
            screen,
            0.0,
            &gaps,
            StackLineReserve::default(),
            path_only,
        )
    }

    /// rift-cli's drawing of a stack layout (`stack_drawn.txt`: the stack as upstream's system
    /// wrote it with `{:#?}`, ids as slotmap writes a key) and its container view: the stack,
    /// each member with its weight, the selected member, the window left out still listed.
    #[test]
    fn rift_cli_draws_and_lists_a_stack_and_its_members() {
        let (system, layout) = four_with_one_left_out();
        assert_eq!(
            system.draw_tree(layout),
            include_str!("stack/stack_drawn.txt").trim_end()
        );
        let tree = system.container_tree(layout);
        assert_eq!(
            (tree.node_id, tree.layout_kind, tree.is_selected),
            ((1 << 32) | 1, Some(LayoutKind::HorizontalStack), false)
        );
        let members: Vec<_> = tree
            .children
            .iter()
            .map(|m| (m.node_id, m.window_id, m.weight, m.is_selected))
            .collect();
        assert_eq!(
            members,
            (1..=4)
                .map(|idx| (
                    (1 << 32) | u64::from(idx + 1),
                    Some(w(idx).into()),
                    Some(1.0),
                    idx == 2
                ))
                .collect::<Vec<_>>()
        );
    }

    /// The group bar shows the windows taking part (the selected one counted among them), over
    /// the tiling area; none for the selection path while the shown member fills the tiling
    /// area, and none at all while the stack fills the screen.
    #[test]
    fn the_group_bar_shows_the_members_taking_part() {
        let (mut system, layout) = four_with_one_left_out();
        let shown = vec![GroupContainerInfo {
            node_id: NodeId::from(KeyData::from_ffi((1 << 32) | 1)),
            container_kind: LayoutKind::HorizontalStack,
            frame: rect(20.0, 35.0, 940.0, 760.0),
            total_count: 3,
            selected_index: 1,
            window_ids: vec![w(1), w(2), w(3)],
        }];
        let words = |infos: Vec<GroupContainerInfo>| -> Vec<_> {
            infos
                .into_iter()
                .map(|i| {
                    (
                        i.node_id,
                        i.container_kind,
                        i.frame,
                        i.total_count,
                        i.selected_index,
                        i.window_ids,
                    )
                })
                .collect()
        };
        assert_eq!(words(group(&system, layout, false)), words(shown.clone()));
        assert_eq!(words(group(&system, layout, true)), words(shown.clone()));
        // A window left out before the selected one is not counted before it.
        let mut fewer = system.clone();
        fewer.set_window_participation(w(1), false);
        let counted: Vec<_> = group(&fewer, layout, false)
            .into_iter()
            .map(|i| (i.total_count, i.selected_index, i.window_ids))
            .collect();
        assert_eq!(counted, vec![(2, 0, vec![w(2), w(3)])]);
        system.edit(layout, LayoutEdit::Fullscreen { within_gaps: true }).discard();
        assert_eq!(words(group(&system, layout, false)), words(shown));
        assert!(group(&system, layout, true).is_empty());
        let _ = system.focus(layout, Focus::Ascend);
        system.edit(layout, LayoutEdit::Fullscreen { within_gaps: false }).discard();
        assert!(group(&system, layout, false).is_empty());
        assert!(group(&system, layout, true).is_empty());
    }

    /// A stack's frames for each fill: tiled members offset along the stack in the tiling area,
    /// a member filling the tiling area or the screen, the whole stack on the screen (the window
    /// left out placed nowhere).
    #[test]
    fn a_stack_places_each_fill() {
        let (mut system, layout) = four_with_one_left_out();
        let tiled = |x| rect(x, 35.0, 916.0, 760.0);
        assert_eq!(frames(&system, layout), vec![
            (w(1), tiled(20.0)),
            (w(2), tiled(32.0)),
            (w(3), tiled(44.0))
        ]);
        system.edit(layout, LayoutEdit::Fullscreen { within_gaps: true }).discard();
        assert_eq!(frames(&system, layout), vec![
            (w(1), tiled(20.0)),
            (w(2), rect(20.0, 35.0, 940.0, 760.0)),
            (w(3), tiled(44.0))
        ]);
        system.edit(layout, LayoutEdit::Fullscreen { within_gaps: false }).discard();
        assert!(system.container_tree(layout).children[1].is_fullscreen);
        assert_eq!(frames(&system, layout), vec![
            (w(1), tiled(20.0)),
            (w(2), rect(0.0, 25.0, 1000.0, 800.0)),
            (w(3), tiled(44.0))
        ]);
        let _ = system.focus(layout, Focus::Ascend);
        system.edit(layout, LayoutEdit::Fullscreen { within_gaps: false }).discard();
        assert_eq!(frames(&system, layout), vec![
            (w(1), rect(0.0, 25.0, 976.0, 800.0)),
            (w(2), rect(0.0, 25.0, 1000.0, 800.0)),
            (w(3), rect(24.0, 25.0, 976.0, 800.0))
        ]);
    }

    /// A system of two windows (the second selected), saved: the text the normalization tests
    /// write a corrupt state into.
    fn saved_two() -> (String, LayoutId) {
        let mut system = StackLayoutSystem::new(StackDefaultOrientation::Horizontal);
        let layout = system.create_layout();
        for window in [w(1), w(2)] {
            system.update(WindowChange::Add(layout, window, None));
        }
        (ron::to_string(&system).unwrap(), layout)
    }

    /// `saved` with `from` written as `to` (exactly once).
    fn corrupted(saved: &str, from: &str, to: &str) -> String {
        assert_eq!(saved.matches(from).count(), 1, "{from} in {saved}");
        saved.replacen(from, to, 1)
    }

    /// Loads a corrupt file: the loaded system saves the normalized text.
    fn loads_as(file: &str, normalized: &str) -> StackLayoutSystem {
        let system: StackLayoutSystem = ron::from_str(file).unwrap();
        assert_eq!(ron::to_string(&system).unwrap(), normalized);
        system
    }

    #[test]
    fn a_saved_stack_of_a_tiled_kind_loads_as_a_stack_kind() {
        let (saved, layout) = saved_two();
        let file = corrupted(&saved, "kind:horizontal_stack", "kind:horizontal");
        let system = loads_as(&file, &saved);
        assert_eq!(
            system.container_tree(layout).layout_kind,
            Some(LayoutKind::HorizontalStack)
        );
    }

    #[test]
    fn a_saved_stack_with_both_fullscreen_flags_loads_fullscreen() {
        let (saved, _) = saved_two();
        let full = corrupted(
            &saved,
            "kind:horizontal_stack,fullscreen:false",
            "kind:horizontal_stack,fullscreen:true",
        );
        let both = corrupted(
            &full,
            "kind:horizontal_stack,fullscreen:true,within_gaps:false",
            "kind:horizontal_stack,fullscreen:true,within_gaps:true",
        );
        loads_as(&both, &full);
    }

    #[test]
    fn a_saved_member_with_both_fullscreen_flags_loads_fullscreen() {
        let (saved, layout) = saved_two();
        let at = "window:(pid:1,idx:1),weight:1.0,fullscreen:";
        let full = corrupted(&saved, &format!("{at}false"), &format!("{at}true"));
        let both = corrupted(
            &full,
            &format!("{at}true,within_gaps:false"),
            &format!("{at}true,within_gaps:true"),
        );
        let system = loads_as(&both, &full);
        assert!(system.container_tree(layout).children[0].is_fullscreen);
    }

    #[test]
    fn a_saved_selection_naming_no_member_loads_on_the_first_member() {
        let (saved, layout) = saved_two();
        let first = corrupted(
            &saved,
            "active:Some((idx:3,version:1))",
            "active:Some((idx:2,version:1))",
        );
        for active in ["None", "Some((idx:9,version:1))"] {
            let file = corrupted(
                &saved,
                "active:Some((idx:3,version:1))",
                &format!("active:{active}"),
            );
            let system = loads_as(&file, &first);
            assert_eq!(system.selected_window(layout), Some(w(1)));
        }
    }

    #[test]
    fn a_saved_empty_stack_loads_with_nothing_selected() {
        let mut system = StackLayoutSystem::default();
        system.create_layout();
        let saved = ron::to_string(&system).unwrap();
        let file = corrupted(
            &saved,
            "active:None,root_selected:false",
            "active:Some((idx:7,version:1)),root_selected:true",
        );
        loads_as(&file, &saved);
    }

    fn drawn_orientation(rng: &mut Rng) -> StackDefaultOrientation {
        match rng.below(4) {
            0 => StackDefaultOrientation::Perpendicular,
            1 => StackDefaultOrientation::Same,
            2 => StackDefaultOrientation::Horizontal,
            _ => StackDefaultOrientation::Vertical,
        }
    }

    fn drawn_insertion(rng: &mut Rng) -> WindowInsertionPoint {
        if rng.chance(50) {
            WindowInsertionPoint::NextToSelection
        } else {
            WindowInsertionPoint::EndOfTree
        }
    }

    /// A stack system after random operations, each held to the core's stacks by the twin (under
    /// `roc-core`, the Rust stacks held to the deciding Roc stacks, and upstream's to the Rust):
    /// windows added, removed, replaced and closed by app, selected, the stack selected and left,
    /// made fullscreen, left out of the layout (windows in no layout too), moved, swapped,
    /// reordered and dropped, the orientation turned, native resizes into and out of the screen
    /// and the tiling area, layouts created, cloned and removed, the settings changed, every
    /// query asked on a third of the steps, and reloads through serde.
    fn drawn_system(rng: &mut Rng) -> (StackLayoutSystem, LayoutId, Vec<WindowId>) {
        let (mut orientation, mut insertion) = (drawn_orientation(rng), drawn_insertion(rng));
        let mut system = StackLayoutSystem::new_with_insertion_point(orientation, insertion);
        let layout = system.create_layout();
        let mut others: Vec<LayoutId> = Vec::new();
        let mut windows: Vec<WindowId> = Vec::new();
        let mut next = 0;
        for _ in 0..rng.below(40) {
            let pick = |rng: &mut Rng, windows: &[WindowId]| {
                windows[rng.below(windows.len() as u64) as usize]
            };
            // Most operations go to the layout drawn for frames, some to the others.
            let layout = if !others.is_empty() && rng.chance(15) {
                others[rng.below(others.len() as u64) as usize]
            } else {
                layout
            };
            if rng.chance(35) {
                asked(rng, &system, layout, &windows);
            }
            if rng.chance(10) {
                match rng.below(5) {
                    0 => others.push(system.create_layout()),
                    1 | 2 => others.push(system.clone_layout(layout)),
                    3 if !others.is_empty() => {
                        let removed = others.swap_remove(rng.below(others.len() as u64) as usize);
                        system.remove_layout(removed);
                    }
                    3 => system.remove_layout(LayoutId::default()),
                    _ => {
                        (orientation, insertion) = (drawn_orientation(rng), drawn_insertion(rng));
                        system.update_settings(orientation, insertion);
                    }
                }
                continue;
            }
            #[cfg(feature = "roc-core")]
            if rng.chance(4) {
                // The Rust stacks re-seeded from the deciding Roc stacks (as after a
                // divergence): the two go on alike.
                system.reseed_rust_from_roc();
            }
            if rng.chance(4) {
                // Saved and loaded (the insertion point is not saved: rift sets it again).
                let mut saved = ron::to_string(&system).unwrap();
                // Some member given a weight other than 1, which only a file from the old
                // forest holds.
                let members = saved.matches("weight:1.0").count();
                if members > 0 && rng.chance(40) {
                    let at =
                        saved.match_indices("weight:1.0").nth(rng.below(members as u64) as usize);
                    let at = at.unwrap().0;
                    let weight = ["weight:2.5", "weight:0.25", "weight:0.0"][rng.below(3) as usize];
                    saved.replace_range(at..at + "weight:1.0".len(), weight);
                }
                system = ron::from_str(&saved).unwrap();
                system.update_settings(orientation, insertion);
                continue;
            }
            if !windows.is_empty() && rng.chance(6) {
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
                let tiling = compute_tiling_area(screen, &gaps);
                let frame = |rng: &mut Rng| match rng.below(4) {
                    0 => screen,
                    1 => tiling,
                    _ => drawn_screen(rng),
                };
                let (old, new) = (frame(rng), frame(rng));
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
                    let focus = match rng.below(5) {
                        0 | 1 => Focus::Select(pick(rng, &windows)),
                        2 => Focus::Ascend,
                        3 => Focus::Descend,
                        _ => Focus::Direction(drawn_direction(rng)),
                    };
                    system.focus(layout, focus);
                }
                6 => {
                    if rng.chance(30) {
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
                    let (a, b) = (pick(rng, &windows), pick(rng, &windows));
                    let edit = match rng.below(3) {
                        0 => LayoutEdit::Swap(a, b),
                        1 => LayoutEdit::ReorderTab { window: a, target: b },
                        _ => LayoutEdit::Drop {
                            source: a,
                            target: b,
                            action: crate::layout_engine::WindowDropAction::Swap,
                        },
                    };
                    if rng.chance(30) {
                        // Past `edit`'s participation check, which the system makes again.
                        system.edit_layout(layout, edit);
                    } else {
                        system.edit(layout, edit);
                    }
                }
                10 => {
                    let edit = match rng.below(4) {
                        0 => LayoutEdit::ToggleOrientation,
                        1 => LayoutEdit::Group(GroupEdit::Stack(drawn_orientation(rng))),
                        // Edits upstream's stack system has no case for.
                        2 => LayoutEdit::Rebalance,
                        _ => LayoutEdit::Group(GroupEdit::Unjoin),
                    };
                    system.edit(layout, edit);
                }
                11 => {
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

    /// Every query, about drawn windows (some not in the layout).
    fn asked(rng: &mut Rng, system: &StackLayoutSystem, layout: LayoutId, windows: &[WindowId]) {
        let window = match windows {
            [] => WindowId::new(1, 9999),
            _ => windows[rng.below(windows.len() as u64) as usize],
        };
        let _ = system.selected_window(layout);
        let _ = system.selection_contains_window(layout, window);
        let _ = system.contains_window(layout, window);
        let _ = system.has_any_fullscreen_node(layout);
        let _ = system.has_windows_for_app(layout, window.pid);
        let _ = system.window_participates(window);
        let _ = system.contains_layout(layout);
        let _ = system.contains_layout(LayoutId::default());
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
        }
        let _ = system.window_slot(layout, window);
        let gaps = drawn_gaps(rng);
        for selection_path_only in [false, true] {
            let _ = system.collect_group_containers(
                layout,
                drawn_screen(rng),
                0.0,
                &gaps,
                StackLineReserve::default(),
                selection_path_only,
            );
        }
        let _ = system.draw_tree(layout);
        let _ = system.container_tree(layout);
    }

    /// rift's frames for a stack layout are the core stacks' shape through the core's geometry,
    /// bit for bit, over drawn stacks, screens, gaps, offsets, bars and window constraints (under
    /// `roc-core` the Roc stacks' frames decide, held to the Rust stacks' by the pass itself).
    #[test]
    fn the_stack_system_tiles_its_stacks_shape() {
        let mut rng = Rng(0x5eed_57ac);
        let (mut cases, mut frames_seen) = (0, 0);
        for _ in 0..3000 {
            let (system, layout, windows) = drawn_system(&mut rng);
            let screen = drawn_screen(&mut rng);
            let constraints = drawn_constraints_for(&mut rng, &windows);
            let gaps = drawn_gaps(&mut rng);
            let offset = drawn_offset(&mut rng);
            let line = drawn_line(&mut rng);
            let given = system.calculate_layout(
                layout,
                screen,
                offset,
                &constraints,
                &gaps,
                line,
                crate::layout_engine::Shadow::Run,
            );
            let core = core_frames(system.shape(layout, screen, offset, &constraints, &gaps, line));
            assert_eq!(bits(&given), bits(&core), "{}", system.draw_tree(layout));
            cases += 1;
            frames_seen += core.len();
        }
        eprintln!("stack: {cases} layouts, {frames_seen} frames identical");
        assert!(frames_seen > 3000, "the draws reach frames ({frames_seen})");
    }
}
