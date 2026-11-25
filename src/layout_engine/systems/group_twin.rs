//! What the traditional and master-stack twins share (layout trees, the group forest's slice 1):
//! the core's forest seeded from upstream's `GroupLayouts`, the view the twins compare, a
//! difference's report, and the conversions of rift's edits, focus and answers.
//!
//! Upstream's system decides; the core is stepped on every operation, in every build, and held
//! to it: each answer on every operation, the whole forest (the view) on every operation in tests
//! and `roc-core` builds and every [`VIEW_EVERY`]th in a plain release build, each pass's shape on
//! every pass. A difference is counted, fails a test, or in a running Rift is warned
//! (`group-forest-divergence:`) and the core re-seeded from upstream's system.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

use objc2_core_foundation::CGRect;
use rift_core::layout::group_forest::{
    self as core, Body, Container, DropAction, Fill, Forest, Kind, Members, Memory, Node,
    Participation, Pass, Place, Resize, Side,
};

use super::groups::{GroupLayouts, LayoutRoot};
use super::{EditResult, Focus, FocusResult, GroupEdit, LayoutEdit, WindowQuery, core_direction};
use crate::actor::app::WindowId;
use crate::common::collections::HashMap;
use crate::common::config::{GapSettings, StackDefaultOrientation, StackLineReserve};
use crate::layout_engine::{LayoutId, LayoutKind, ResizeOrientation, WindowDropAction};
use crate::model::selection::SelectionInfo;
use crate::model::slot_table::{SlotTable, of_slotmap};
use crate::model::tree::{self, Held, NodeId};

/// The core's forest over rift's ids.
pub(super) type CoreForest = Forest<LayoutId, NodeId, WindowId>;

/// How often a plain release build compares the whole forest (the twins' sampling).
pub(super) const VIEW_EVERY: u64 = 32;

/// Whether every operation compares the whole forest.
pub(super) const VIEW_ALWAYS: bool = cfg!(any(test, feature = "roc-core"));

static GROUP_FOREST_DIVERGENCES: AtomicU64 = AtomicU64::new(0);

/// Differences between the core's forest and upstream's systems, counted process-wide.
pub(crate) fn group_forest_divergences() -> u64 { GROUP_FOREST_DIVERGENCES.load(Ordering::Relaxed) }

/// Counts a difference, then fails the test, or in a running Rift warns it.
pub(super) fn report(system: &str, what: &str, difference: &str) {
    let divergences = GROUP_FOREST_DIVERGENCES.fetch_add(1, Ordering::Relaxed) + 1;
    #[cfg(test)]
    panic!("group-forest-divergence ({divergences} so far) in {system} after {what}: {difference}");
    #[cfg(not(test))]
    tracing::warn!(
        system,
        what,
        divergences,
        difference,
        "group-forest-divergence: the core's forest differs from upstream's; re-seeded"
    );
}

pub(super) fn core_kind(kind: LayoutKind) -> Kind {
    match kind {
        LayoutKind::Horizontal => Kind::Split(core::Orientation::Horizontal),
        LayoutKind::Vertical => Kind::Split(core::Orientation::Vertical),
        LayoutKind::HorizontalStack => Kind::Stack(core::Orientation::Horizontal),
        LayoutKind::VerticalStack => Kind::Stack(core::Orientation::Vertical),
    }
}

/// Upstream's two fullscreen flags as one fill (`fullscreen` first, as its projection reads
/// them; the load leaves no saved state holding both).
fn core_fill(fullscreen: bool, within_gaps: bool) -> Fill {
    match (fullscreen, within_gaps) {
        (true, _) => Fill::Screen,
        (false, true) => Fill::WithinGaps,
        (false, false) => Fill::Tiled,
    }
}

fn core_memory(selection: Option<SelectionInfo>) -> Memory<NodeId> {
    match selection.map(SelectionInfo::parts) {
        None => Memory::Unset,
        Some((child, false)) => Memory::Toward(child),
        Some((child, true)) => Memory::Stops(child),
    }
}

/// Each indexed leaf's stamp, in the index's order: a window's leaves by when they entered.
fn stamps(tiles: &GroupLayouts) -> HashMap<NodeId, u64> {
    let mut stamps = HashMap::default();
    let entries = tiles.tree.window.window_nodes.values().flat_map(|nodes| nodes.0.iter());
    for (stamp, info) in (0u64..).zip(entries) {
        stamps.insert(info.node, stamp);
    }
    stamps
}

/// An upstream node as the core holds it (a leaf no layout reaches is not indexed: the load
/// leaves every other one indexed; it takes the stamp after every indexed one).
fn core_node(
    tiles: &GroupLayouts,
    stamps: &HashMap<NodeId, u64>,
    key: NodeId,
    node: &tree::Node,
) -> Node<NodeId, WindowId> {
    let (parent, held) = tiles.tree.map.held(key);
    let info = node.layout;
    let place = match parent {
        None => Place::Root,
        Some(parent) => Place::Child { parent, size: info.size },
    };
    let body = match held {
        Held::Window(window) => Body::Window {
            window,
            stamp: stamps.get(&key).copied().unwrap_or(stamps.len() as u64),
        },
        Held::Container { children, selection } => Body::Container(Container {
            kind: core_kind(info.kind),
            members: Members::Children(children),
            memory: core_memory(selection),
        }),
        Held::RegionPair {
            regions,
            primary_first,
            selection,
        } => Body::Container(Container {
            kind: core_kind(info.kind),
            members: Members::Regions {
                order: regions,
                primary: if primary_first {
                    Side::First
                } else {
                    Side::Second
                },
            },
            memory: core_memory(selection),
        }),
    };
    Node {
        place,
        fill: core_fill(info.is_fullscreen, info.is_fullscreen_within_gaps),
        body,
    }
}

/// Upstream's forest as the core holds it: both slot tables as upstream's slotmaps hold them
/// (free lists included), each leaf stamped in the index's order.
pub(super) fn seed(tiles: &GroupLayouts) -> CoreForest {
    let layouts = of_slotmap(&tiles.layout_roots, LayoutRoot(NodeId::default()), |root| root.0);
    let stamps = stamps(tiles);
    let nodes: SlotTable<NodeId, tree::Node> =
        of_slotmap(tiles.tree.map.slots(), tree::Node::default(), Clone::clone);
    let nodes = nodes
        .map(|key, node| Ok::<_, std::convert::Infallible>(core_node(tiles, &stamps, key, &node)))
        .unwrap_or_else(|never| match never {});
    CoreForest::from_parts(layouts, nodes, tiles.inactive.iter().copied().collect())
}

/// A forest as the twins compare it: layouts and nodes by id (a leaf's stamp left out: only its
/// order among the window's leaves is behaviour, which the index holds), the windows left out,
/// and the index. Vacant slots and free lists are not compared: a difference there shows as the
/// next id allocated.
type View = (
    Vec<(LayoutId, NodeId)>,
    Vec<(NodeId, Node<NodeId, WindowId>)>,
    BTreeSet<WindowId>,
    Vec<(WindowId, LayoutId, NodeId)>,
);

fn unstamped(mut node: Node<NodeId, WindowId>) -> Node<NodeId, WindowId> {
    if let Body::Window { stamp, .. } = &mut node.body {
        *stamp = 0;
    }
    node
}

fn core_view(forest: &CoreForest) -> View {
    (
        forest.layouts().collect(),
        forest
            .nodes()
            .iter()
            .map(|(key, node)| (key, unstamped(node.clone())))
            .collect(),
        forest.left_out().clone(),
        forest.index(),
    )
}

/// Upstream's index as (window, layout, leaf), in its order.
fn upstream_index(tiles: &GroupLayouts) -> Vec<(WindowId, LayoutId, NodeId)> {
    tiles
        .tree
        .window
        .window_nodes
        .iter()
        .flat_map(|(&window, nodes)| {
            nodes.0.iter().map(move |info| (window, info.layout, info.node))
        })
        .collect()
}

fn upstream_view(tiles: &GroupLayouts) -> View {
    let stamps = HashMap::default();
    (
        tiles.layout_roots.iter().map(|(layout, root)| (layout, root.0)).collect(),
        tiles
            .tree
            .map
            .slots()
            .iter()
            .map(|(key, node)| (key, unstamped(core_node(tiles, &stamps, key, node))))
            .collect(),
        tiles.inactive.iter().copied().collect(),
        upstream_index(tiles),
    )
}

/// Where the core's forest differs from upstream's, if it does.
pub(super) fn view_difference(core: &CoreForest, tiles: &GroupLayouts) -> Option<String> {
    let (ours, theirs) = (core_view(core), upstream_view(tiles));
    if ours.0 != theirs.0 {
        return Some(format!("layouts: upstream {:?}, core {:?}", theirs.0, ours.0));
    }
    if ours.1 != theirs.1 {
        return Some(format!("nodes: upstream {:?}, core {:?}", theirs.1, ours.1));
    }
    if ours.2 != theirs.2 {
        return Some(format!("left out: upstream {:?}, core {:?}", theirs.2, ours.2));
    }
    if ours.3 != theirs.3 {
        return Some(format!("index: upstream {:?}, core {:?}", theirs.3, ours.3));
    }
    None
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

/// A focus's answer, comparable.
pub(super) fn focus_words(result: &FocusResult) -> (bool, Option<WindowId>, Vec<WindowId>) {
    (result.handled, result.window, result.raise.clone())
}

/// An edit's answer, comparable.
pub(super) fn edit_words(result: &EditResult) -> (&'static str, Vec<WindowId>) {
    match result {
        EditResult::Unsupported => ("unsupported", Vec::new()),
        EditResult::Unchanged => ("unchanged", Vec::new()),
        EditResult::Changed => ("changed", Vec::new()),
        EditResult::Raise(windows) => ("raise", windows.clone()),
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
    /// selection memory cleared while it has children (no operation clears one so).
    pub(in crate::layout_engine::systems) fn with_saved_only_states(
        rng: &mut Rng,
        text: &str,
    ) -> String {
        const HELD: &str = "selection:Some((";
        let held: Vec<usize> = text.match_indices(HELD).map(|(at, _)| at).collect();
        if held.is_empty() || !rng.chance(30) {
            return text.to_owned();
        }
        let at = held[rng.below(held.len() as u64) as usize];
        let end = at + text[at..].find("))").expect("a saved selection closes") + 2;
        format!("{}selection:None{}", &text[..at], &text[end..])
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

    /// Every query, answered by both sides (the twin compares them).
    fn query<S: LayoutSystem>(rng: &mut Rng, system: &S, drawn: &mut Drawn) {
        let layout = drawn.layout(rng);
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
                query(rng, system, &mut drawn);
            }
        }
        drawn
    }
}
