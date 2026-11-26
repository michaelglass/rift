//! What the traditional and master-stack systems share: the core's forest over rift's ids, the
//! group bar's pass, and the conversions of rift's edits, focus, queries and answers to and from
//! the core's.

use objc2_core_foundation::CGRect;
use rift_core::layout::group_forest::{
    self as core, Bars, DropAction, Fill, Forest, Kind, Participation, Pass, Resize,
};

use super::{
    EditResult, Focus, FocusResult, GroupEdit, LayoutEdit, WindowLayoutConstraints, WindowQuery,
    core_direction,
};
use crate::actor::app::WindowId;
use crate::common::collections::HashMap;
use crate::common::config::{GapSettings, StackDefaultOrientation, StackLineReserve};
use crate::layout_engine::engine::GroupContainerInfo;
use crate::layout_engine::{LayoutId, LayoutKind, ResizeOrientation, WindowDropAction};
use crate::model::tree::NodeId;

/// The core's forest over rift's ids.
pub(super) type CoreForest = Forest<LayoutId, NodeId, WindowId>;

/// A forest with these nodes' sizes among their siblings set (for tests: a state operations
/// reach only by many resizes, or a saved file holds).
#[cfg(test)]
pub(super) fn with_sizes(forest: &CoreForest, sizes: &[(NodeId, f32)]) -> CoreForest {
    let nodes = forest
        .nodes()
        .clone()
        .map(|key, mut node| {
            if let core::Place::Child { size, .. } = &mut node.place
                && let Some(&(_, set)) = sizes.iter().find(|(sized, _)| *sized == key)
            {
                *size = set;
            }
            Ok::<_, std::convert::Infallible>(node)
        })
        .unwrap_or_else(|never| match never {});
    Forest::from_parts(
        forest.layout_table().clone(),
        nodes,
        forest.left_out().clone(),
        forest.next_stamp(),
    )
}

pub(super) fn core_kind(kind: LayoutKind) -> Kind {
    match kind {
        LayoutKind::Horizontal => Kind::Split(core::Orientation::Horizontal),
        LayoutKind::Vertical => Kind::Split(core::Orientation::Vertical),
        LayoutKind::HorizontalStack => Kind::Stack(core::Orientation::Horizontal),
        LayoutKind::VerticalStack => Kind::Stack(core::Orientation::Vertical),
    }
}

/// A core kind as upstream's layout kind.
pub(super) fn layout_kind(kind: Kind) -> LayoutKind {
    match kind {
        Kind::Split(core::Orientation::Horizontal) => LayoutKind::Horizontal,
        Kind::Split(core::Orientation::Vertical) => LayoutKind::Vertical,
        Kind::Stack(core::Orientation::Horizontal) => LayoutKind::HorizontalStack,
        Kind::Stack(core::Orientation::Vertical) => LayoutKind::VerticalStack,
    }
}

/// The core's container view as rift-cli's protocol carries it (upstream's
/// `container_tree_with_roles`: ids as slotmap's ffi words, no frames, no pending split).
pub(super) fn container_node(
    view: core::ContainerView<NodeId, WindowId>,
) -> rift_protocol::ContainerTreeNode {
    use rift_protocol::{ContainerNodeType, ContainerTreeNode};
    use slotmap::Key;
    let (node_type, layout_kind, window_id, children) = match view.held {
        core::Held::Window(window) => {
            (ContainerNodeType::Window, None, Some(window.into()), vec![])
        }
        core::Held::Container { kind, children } => (
            ContainerNodeType::Container,
            Some(layout_kind(kind)),
            None,
            children.into_iter().map(container_node).collect(),
        ),
    };
    ContainerTreeNode {
        node_id: view.node.data().as_ffi(),
        node_type,
        frame: Default::default(),
        layout_kind,
        weight: view.weight.map(f64::from),
        window_id,
        is_selected: view.selected,
        is_fullscreen: view.fill == Fill::Screen,
        is_fullscreen_within_gaps: view.fill == Fill::WithinGaps,
        role: view.role.name().map(str::to_owned),
        pending_split: None,
        children,
    }
}

/// A layout pass with its group bars (upstream's `project` with a group output, which the
/// group bar reads): the bars the core's forest gives, and the frames (when `emit_frames`) as
/// `calculate_layout` decides them. Under `roc-core` the Roc forest's pass decides, bars and
/// frames (`roc`, given the Rust forest's bars to hold its own to).
#[allow(clippy::too_many_arguments)]
pub(super) fn project_groups(
    system: &str,
    forest: &CoreForest,
    layout: LayoutId,
    screen: CGRect,
    offset: f64,
    constraints: &HashMap<WindowId, WindowLayoutConstraints>,
    gaps: &GapSettings,
    line: StackLineReserve,
    emit_frames: bool,
    selection_path_only: bool,
    #[cfg(feature = "roc-core")] roc: impl FnOnce(
        Pass,
        Bars,
        &[core::GroupBar<NodeId, WindowId>],
    ) -> super::group_roc::Projected,
) -> (Vec<(WindowId, CGRect)>, Vec<GroupContainerInfo>) {
    use crate::layout_engine::core_geometry::{cg_rect, core_constraints};
    let constrained = |window: WindowId| constraints.get(&window).copied().map(core_constraints);
    let bars = if selection_path_only {
        Bars::SelectionPath
    } else {
        Bars::Every
    };
    let pass = pass(screen, gaps, offset, line);
    let rust = forest.project(layout, pass, &constrained, bars).bars;
    let info = |bars: &[core::GroupBar<NodeId, WindowId>]| -> Vec<GroupContainerInfo> {
        bars.iter()
            .map(|bar| GroupContainerInfo {
                node_id: bar.stack,
                container_kind: layout_kind(Kind::Stack(bar.orientation)),
                frame: cg_rect(bar.frame),
                total_count: bar.windows.len(),
                selected_index: bar.selected,
                window_ids: bar.windows.clone(),
            })
            .collect()
    };
    let shape = || forest.shape(layout, pass, &constrained);
    #[cfg(feature = "roc-core")]
    {
        let (frames, decided) = roc(pass, bars, &rust);
        let frames = if emit_frames {
            crate::layout_engine::core_geometry::decide_roc_tree(system, shape(), || frames)
        } else {
            Vec::new()
        };
        (frames, info(&decided))
    }
    #[cfg(not(feature = "roc-core"))]
    {
        let frames = if emit_frames {
            crate::layout_engine::core_geometry::decide_tree(system, shape())
        } else {
            Vec::new()
        };
        (frames, info(&rust))
    }
}

pub(super) fn default_orientation(
    orientation: StackDefaultOrientation,
) -> core::DefaultOrientation {
    match orientation {
        StackDefaultOrientation::Perpendicular => core::DefaultOrientation::Perpendicular,
        StackDefaultOrientation::Same => core::DefaultOrientation::Same,
        StackDefaultOrientation::Horizontal => core::DefaultOrientation::Horizontal,
        StackDefaultOrientation::Vertical => core::DefaultOrientation::Vertical,
    }
}

/// An edit as the core takes it (a drop that moves or consumes arrives as its edit, through
/// `LayoutSystem::edit`; one reaching `edit_layout` directly does nothing, as upstream's
/// systems leave it).
pub(super) fn core_edit(edit: LayoutEdit) -> Option<core::Edit<WindowId>> {
    Some(match edit {
        LayoutEdit::Move {
            direction,
            has_display_neighbor,
        } => core::Edit::Move {
            direction: core_direction(direction),
            beyond: if has_display_neighbor {
                core::Beyond::AnotherDisplay
            } else {
                core::Beyond::Nothing
            },
        },
        LayoutEdit::Split(kind) => core::Edit::Split(core_kind(kind)),
        LayoutEdit::Fullscreen { within_gaps } => core::Edit::Fullscreen { within_gaps },
        LayoutEdit::Swap(a, b) => core::Edit::Swap(a, b),
        LayoutEdit::ReorderTab { window, target } => core::Edit::ReorderTab { window, target },
        LayoutEdit::Drop { source, target, action } => core::Edit::Drop {
            source,
            target,
            action: match action {
                WindowDropAction::Swap => DropAction::Swap,
                WindowDropAction::Stack => DropAction::Stack,
                WindowDropAction::Insert(direction) => {
                    DropAction::Insert(core_direction(direction))
                }
                WindowDropAction::Move(_) | WindowDropAction::ConsumeOrExpel(_) => return None,
            },
        },
        LayoutEdit::Group(GroupEdit::Join(direction)) => {
            core::Edit::Join(core_direction(direction))
        }
        LayoutEdit::Group(GroupEdit::ConsumeOrExpel(direction)) => {
            core::Edit::ConsumeOrExpel(core_direction(direction))
        }
        LayoutEdit::Group(GroupEdit::Stack(default)) => {
            core::Edit::Stack(default_orientation(default))
        }
        LayoutEdit::Group(GroupEdit::Unstack(default)) => {
            core::Edit::Unstack(default_orientation(default))
        }
        LayoutEdit::Group(GroupEdit::Unjoin) => core::Edit::Unjoin,
        LayoutEdit::Resize { amount, orientation } => core::Edit::Resize {
            amount,
            along: match orientation {
                ResizeOrientation::Horizontal => core::ResizeAlong::Horizontal,
                ResizeOrientation::Vertical => core::ResizeAlong::Vertical,
                ResizeOrientation::Smart => core::ResizeAlong::Smart,
            },
        },
        LayoutEdit::Rebalance => core::Edit::Rebalance,
        LayoutEdit::ToggleOrientation => core::Edit::ToggleOrientation,
    })
}

pub(super) fn core_focus(focus: Focus) -> core::Focus<WindowId> {
    match focus {
        Focus::Select(window) => core::Focus::Select(window),
        Focus::Ascend => core::Focus::Ascend,
        Focus::Descend => core::Focus::Descend,
        Focus::Direction(direction) => core::Focus::Direction(core_direction(direction)),
    }
}

pub(super) fn focus_result(focused: core::Focused<WindowId>) -> FocusResult {
    match focused {
        core::Focused::Selection { handled } => FocusResult::selection(handled),
        core::Focused::Movement { window, raise } => FocusResult::movement((window, raise)),
    }
}

pub(super) fn edit_result(edited: core::Edited<WindowId>) -> EditResult {
    match edited {
        core::Edited::Unsupported => EditResult::Unsupported,
        core::Edited::Unchanged => EditResult::Unchanged,
        core::Edited::Changed => EditResult::Changed,
        core::Edited::Raise(windows) => EditResult::Raise(windows),
    }
}

pub(super) fn participation(participates: bool) -> Participation {
    if participates {
        Participation::TakesPart
    } else {
        Participation::LeftOut
    }
}

pub(super) fn core_query(query: WindowQuery) -> rift_core::layout::group_forest::Query<WindowId> {
    use rift_core::layout::group_forest::Query;
    match query {
        WindowQuery::All => Query::All,
        WindowQuery::Visible => Query::Visible,
        WindowQuery::Selection => Query::Selection,
        WindowQuery::Stack(window) => Query::Stack(window),
    }
}

pub(super) fn pass(
    screen: CGRect,
    gaps: &GapSettings,
    offset: f64,
    line: StackLineReserve,
) -> Pass {
    use crate::layout_engine::core_geometry::{core_gaps, core_rect, core_stack_line};
    Pass {
        screen: core_rect(screen),
        gaps: core_gaps(gaps),
        stack_offset: offset,
        stack_line: core_stack_line(line),
    }
}

pub(super) fn resize(old: CGRect, new: CGRect, screen: CGRect, gaps: &GapSettings) -> Resize {
    use crate::layout_engine::core_geometry::core_rect;
    Resize {
        old: core_rect(old),
        new: core_rect(new),
        screen: core_rect(screen),
        tiling: core_rect(crate::layout_engine::utils::compute_tiling_area(screen, gaps)),
    }
}

/// A drawn run over a group forest system, shared by the traditional and master-stack drawn
/// fuzzers: layouts made, cloned and removed; windows added (in a layout or two, with a
/// direction or not), removed, rebalanced away, replaced and closed by app; native resizes into
/// and out of the screen and tiling frames and nudged; participation; every focus and edit;
/// every query on a third of steps; layout passes; and reloads through serde. The system's own
/// operations come through `own`, its reload through `reload`.
#[cfg(test)]
pub(super) mod drawn {
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};

    use crate::actor::app::WindowId;
    use crate::common::config::StackDefaultOrientation;
    use crate::layout_engine::systems::drawn::*;
    use crate::layout_engine::systems::{
        Focus, GroupEdit, LayoutEdit, LayoutSystem, WindowChange, WindowQuery,
    };
    use crate::layout_engine::{
        Direction, LayoutId, LayoutKind, ResizeOrientation, Shadow, WindowDropAction,
    };

    /// How many systems a drawn fuzzer draws (`RIFT_FOREST_CASES`, for plant runs).
    pub(in crate::layout_engine::systems) fn cases(default: u64) -> u64 {
        std::env::var("RIFT_FOREST_CASES")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(default)
    }

    /// A saved system's text, sometimes with a state only a saved file holds: a container's
    /// selection memory cleared while it has children (no operation clears one so), or a
    /// child's size negative (no operation writes one).
    pub(in crate::layout_engine::systems) fn with_saved_only_states(
        rng: &mut Rng,
        text: &str,
    ) -> String {
        let text = &with_a_negative_size(rng, text);
        const HELD: &str = "selection:Some((";
        let held: Vec<usize> = text.match_indices(HELD).map(|(at, _)| at).collect();
        if held.is_empty() || !rng.chance(30) {
            return text.to_owned();
        }
        let at = held[rng.below(held.len() as u64) as usize];
        let end = at + text[at..].find("))").expect("a saved selection closes") + 2;
        format!("{}selection:None{}", &text[..at], &text[end..])
    }

    /// `text` with one saved size other than 0 (a root's is 0) negated, on a tenth of calls.
    fn with_a_negative_size(rng: &mut Rng, text: &str) -> String {
        const SIZE: &str = "size:";
        let sizes: Vec<usize> = text
            .match_indices(SIZE)
            .map(|(at, _)| at + SIZE.len())
            .filter(|&at| !text[at..].starts_with("0.0,") && !text[at..].starts_with('-'))
            .collect();
        if sizes.is_empty() || !rng.chance(10) {
            return text.to_owned();
        }
        let at = sizes[rng.below(sizes.len() as u64) as usize];
        format!("{}-{}", &text[..at], &text[at..])
    }

    /// A drawn group bar pass's arguments: a screen, constraints for `windows`, gaps, a stack
    /// offset and line, whether frames are asked for and whether only the selection path is.
    pub(in crate::layout_engine::systems) fn bar_pass(
        rng: &mut Rng,
        windows: &[WindowId],
    ) -> (
        CGRect,
        crate::common::collections::HashMap<
            WindowId,
            crate::layout_engine::systems::WindowLayoutConstraints,
        >,
        crate::common::config::GapSettings,
        f64,
        crate::common::config::StackLineReserve,
        bool,
        bool,
    ) {
        (
            drawn_screen(rng),
            drawn_constraints_for(rng, windows),
            drawn_gaps(rng),
            drawn_offset(rng),
            drawn_line(rng),
            rng.chance(50),
            rng.chance(50),
        )
    }

    /// What a run has made so far.
    pub(in crate::layout_engine::systems) struct Drawn {
        pub(in crate::layout_engine::systems) layouts: Vec<LayoutId>,
        pub(in crate::layout_engine::systems) windows: Vec<WindowId>,
        next: u32,
    }

    impl Drawn {
        pub(in crate::layout_engine::systems) fn layout(&self, rng: &mut Rng) -> LayoutId {
            self.layouts[rng.below(self.layouts.len() as u64) as usize]
        }

        /// One of the windows, or (rarely) one no layout has seen.
        pub(in crate::layout_engine::systems) fn window(&mut self, rng: &mut Rng) -> WindowId {
            if self.windows.is_empty() || rng.chance(4) {
                self.next += 1;
                WindowId::new(1 + rng.below(3) as i32, self.next)
            } else {
                self.windows[rng.below(self.windows.len() as u64) as usize]
            }
        }

        fn fresh(&mut self, rng: &mut Rng) -> WindowId {
            self.next += 1;
            let window = WindowId::new(1 + rng.below(3) as i32, self.next);
            self.windows.push(window);
            window
        }
    }

    fn stack_orientation(rng: &mut Rng) -> StackDefaultOrientation {
        match rng.below(4) {
            0 => StackDefaultOrientation::Perpendicular,
            1 => StackDefaultOrientation::Same,
            2 => StackDefaultOrientation::Horizontal,
            _ => StackDefaultOrientation::Vertical,
        }
    }

    fn kind(rng: &mut Rng) -> LayoutKind {
        match rng.below(4) {
            0 => LayoutKind::Horizontal,
            1 => LayoutKind::Vertical,
            2 => LayoutKind::HorizontalStack,
            _ => LayoutKind::VerticalStack,
        }
    }

    fn edit(rng: &mut Rng, drawn: &mut Drawn) -> LayoutEdit {
        match rng.below(15) {
            0 => LayoutEdit::Move {
                direction: drawn_direction(rng),
                has_display_neighbor: rng.chance(30),
            },
            1 => LayoutEdit::Split(kind(rng)),
            2 => LayoutEdit::Fullscreen { within_gaps: rng.chance(50) },
            3 => LayoutEdit::Swap(drawn.window(rng), drawn.window(rng)),
            4 => LayoutEdit::ReorderTab {
                window: drawn.window(rng),
                target: drawn.window(rng),
            },
            5 | 6 => LayoutEdit::Drop {
                source: drawn.window(rng),
                target: drawn.window(rng),
                action: match rng.below(5) {
                    0 => WindowDropAction::Swap,
                    1 => WindowDropAction::Stack,
                    2 => WindowDropAction::Insert(drawn_direction(rng)),
                    3 => WindowDropAction::Move(drawn_direction(rng)),
                    _ => WindowDropAction::ConsumeOrExpel(drawn_direction(rng)),
                },
            },
            7 | 8 | 13 => LayoutEdit::Group(match rng.below(5) {
                0 => GroupEdit::Join(drawn_direction(rng)),
                1 => GroupEdit::ConsumeOrExpel(drawn_direction(rng)),
                2 => GroupEdit::Stack(stack_orientation(rng)),
                3 => GroupEdit::Unstack(stack_orientation(rng)),
                _ => GroupEdit::Unjoin,
            }),
            9 | 10 => LayoutEdit::Resize {
                amount: if rng.chance(10) {
                    0.0
                } else {
                    rng.length(-0.6, 0.6)
                },
                orientation: match rng.below(3) {
                    0 => ResizeOrientation::Horizontal,
                    1 => ResizeOrientation::Vertical,
                    _ => ResizeOrientation::Smart,
                },
            },
            11 => LayoutEdit::Rebalance,
            _ => LayoutEdit::ToggleOrientation,
        }
    }

    fn focus(rng: &mut Rng, drawn: &mut Drawn) -> Focus {
        match rng.below(7) {
            0 | 1 => Focus::Select(drawn.window(rng)),
            2 => Focus::Ascend,
            3 => Focus::Descend,
            _ => Focus::Direction(drawn_direction(rng)),
        }
    }

    /// A native resize of `window`: from its frame (or the screen or tiling area) into the
    /// screen, the tiling area, or its frame nudged on an edge or two.
    fn resize<S: LayoutSystem>(
        rng: &mut Rng,
        system: &S,
        layout: LayoutId,
        window: WindowId,
    ) -> (CGRect, CGRect, CGRect, crate::common::config::GapSettings) {
        let screen = CGRect::new(CGPoint::new(0., 0.), CGSize::new(1600., 1000.));
        let gaps = drawn_gaps(rng);
        let tiling = crate::layout_engine::utils::compute_tiling_area(screen, &gaps);
        let frame = system
            .calculate_layout(
                layout,
                screen,
                20.,
                &Default::default(),
                &gaps,
                Default::default(),
                Shadow::Skip,
            )
            .into_iter()
            .find_map(|(w, frame)| (w == window).then_some(frame))
            .unwrap_or(tiling);
        let old = match rng.below(6) {
            0 => screen,
            1 => tiling,
            _ => frame,
        };
        let mut nudged = old;
        for _ in 0..1 + rng.below(2) {
            let delta = if rng.chance(20) {
                rng.length(-0.9, 0.9)
            } else {
                rng.length(-300., 300.)
            };
            match rng.below(4) {
                0 => {
                    nudged.origin.x += delta;
                    nudged.size.width -= delta;
                }
                1 => nudged.size.width += delta,
                2 => {
                    nudged.origin.y += delta;
                    nudged.size.height -= delta;
                }
                _ => nudged.size.height += delta,
            }
        }
        let new = match rng.below(6) {
            0 => screen,
            1 => tiling,
            _ => nudged,
        };
        (old, new, screen, gaps)
    }

    /// Every query (under `roc-core` the Roc forest's answer held to the Rust forest's), the
    /// system's own views
    /// (rift-cli's, the group bar's) through `views`.
    fn query<S: LayoutSystem>(
        rng: &mut Rng,
        system: &S,
        drawn: &mut Drawn,
        views: &impl Fn(&mut Rng, &S, LayoutId, &[WindowId]),
    ) {
        let layout = drawn.layout(rng);
        let windows = drawn.windows.clone();
        views(rng, system, layout, &windows);
        let window = drawn.window(rng);
        system.contains_layout(layout);
        system.selected_window(layout);
        system.selection_contains_window(layout, window);
        system.contains_window(layout, window);
        system.has_any_fullscreen_node(layout);
        system.parent_of_selection_is_stacked(layout);
        system.window_participates(window);
        system.has_windows_for_app(layout, window.pid);
        for direction in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ] {
            system.window_in_direction(layout, direction);
        }
        for query in [
            WindowQuery::All,
            WindowQuery::Visible,
            WindowQuery::Selection,
            WindowQuery::Stack(window),
        ] {
            system.windows(layout, query);
        }
        let screen = drawn_screen(rng);
        let windows = drawn.windows.clone();
        let constraints = drawn_constraints_for(rng, &windows);
        let (gaps, offset, line) = (drawn_gaps(rng), drawn_offset(rng), drawn_line(rng));
        system.calculate_layout(layout, screen, offset, &constraints, &gaps, line, Shadow::Skip);
    }

    /// A run of `steps` drawn operations on `system` (holding one layout to start).
    pub(in crate::layout_engine::systems) fn run<S: LayoutSystem>(
        rng: &mut Rng,
        system: &mut S,
        steps: u64,
        own: &mut impl FnMut(&mut Rng, &mut S, &mut Drawn),
        reload: &impl Fn(&mut Rng, &S) -> S,
        views: &impl Fn(&mut Rng, &S, LayoutId, &[WindowId]),
    ) -> Drawn {
        let mut drawn = Drawn {
            layouts: vec![system.create_layout()],
            windows: Vec::new(),
            next: 0,
        };
        for _ in 0..steps {
            let layout = drawn.layout(rng);
            match rng.below(30) {
                0..=5 => {
                    let window = drawn.fresh(rng);
                    let toward = rng.chance(30).then(|| drawn_direction(rng));
                    system.update(WindowChange::Add(layout, window, toward));
                    if rng.chance(20) {
                        let other = drawn.layout(rng);
                        system.update(WindowChange::Add(other, window, None));
                    }
                }
                6 => {
                    let window = drawn.window(rng);
                    drawn.windows.retain(|&w| w != window);
                    let change = if rng.chance(50) {
                        WindowChange::Remove(window)
                    } else {
                        WindowChange::RemoveAndRebalance(window)
                    };
                    system.update(change);
                }
                7 => {
                    let (from, to) = (drawn.window(rng), drawn.window(rng));
                    system.update(WindowChange::Replace(from, to));
                    drawn.windows.retain(|&w| w != from);
                    if !drawn.windows.contains(&to) {
                        drawn.windows.push(to);
                    }
                }
                8 if rng.chance(30) => {
                    let pid = drawn.window(rng).pid;
                    system.update(WindowChange::CloseApp(pid));
                    drawn.windows.retain(|w| w.pid != pid);
                }
                9 => {
                    let window = drawn.window(rng);
                    let (old, new, screen, gaps) = resize(rng, system, layout, window);
                    system.update(WindowChange::Resize {
                        layout,
                        window,
                        old,
                        new,
                        screen,
                        gaps: &gaps,
                    });
                }
                10 => {
                    let window = drawn.window(rng);
                    system.set_window_participation(window, rng.chance(55));
                }
                11..=14 => {
                    system.focus(layout, focus(rng, &mut drawn));
                }
                15..=22 => {
                    if rng.chance(30) {
                        for _ in 0..1 + rng.below(2) {
                            system.focus(layout, Focus::Ascend);
                        }
                    }
                    let edit = edit(rng, &mut drawn);
                    // Straight to `edit_layout` (past `edit`'s participation check), as rift never
                    // sends a drop that moves or consumes (upstream's systems panic on one).
                    let translated = matches!(edit, LayoutEdit::Drop {
                        action: WindowDropAction::Move(_) | WindowDropAction::ConsumeOrExpel(_),
                        ..
                    });
                    if !translated && rng.chance(15) {
                        system.edit_layout(layout, edit).discard();
                    } else {
                        system.edit(layout, edit).discard();
                    }
                }
                23 => {
                    if drawn.layouts.len() < 4 && rng.chance(50) {
                        drawn.layouts.push(system.create_layout());
                    } else if drawn.layouts.len() < 4 {
                        drawn.layouts.push(system.clone_layout(layout));
                    } else {
                        system.remove_layout(layout);
                        drawn.layouts.retain(|&l| l != layout);
                    }
                }
                24 if drawn.layouts.len() > 1 && rng.chance(30) => {
                    system.remove_layout(layout);
                    drawn.layouts.retain(|&l| l != layout);
                }
                25 if rng.chance(15) => *system = reload(rng, system),
                26..=27 => own(rng, system, &mut drawn),
                _ => {}
            }
            if rng.chance(33) {
                query(rng, system, &mut drawn, views);
            }
        }
        drawn
    }
}
