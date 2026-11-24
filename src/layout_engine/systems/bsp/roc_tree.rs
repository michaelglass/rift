//! Under `roc-core`, the Roc BSP tree ([`RocBspTree`]) decides (layout trees, 4b): it takes
//! every operation, queries included, and its answers are rift's, as are a layout's frames (its
//! own shape through the Roc geometry). The Rust core's tree, the Roc tree's shadow, takes every
//! operation too; its answer and whole view are compared with the Roc tree's on every operation.
//! A difference is counted ([`bsp_tree_reseeds`]), fails a test, or in a running Rift warns
//! `roc-divergence:` and re-seeds the Rust tree from the Roc tree's view (a query's difference is
//! only reported: the Rust tree is not changed by one, and the next operation compares the
//! views). Upstream's system stays the Rust tree's shadow.
//!
//! The Roc tree holds the core's ids as words: layouts and nodes by slotmap's ffi form, windows as
//! [`Word`] makes them, which order as `WindowId` does (pid, then index; pids are not negative),
//! so the trees' orderings (the windows left out, an app's windows closed in order) agree.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

use rift_core::layout::bsp_tree::{
    self as core, Content, Edit, Edited, Focus, Focused, Kind, Layout, Leaf, Members, Node, Parent,
    Split, Tabs,
};
use rift_core::layout::{Gaps, WindowConstraints};
use rift_core::plain::Rect;
use rift_core::slot_table::SlotTable;
use rift_core_roc::RocBspTree;
use rift_core_roc::bsp_tree::{Answer, Event, LayoutKey, NodeKey, TreeView};

use super::system::CoreTree;
use crate::actor::app::WindowId;
use crate::layout_engine::LayoutId;
use crate::model::tree::NodeId;
use crate::roc_path::{Word, window_of_word};

static RESEEDS: AtomicU64 = AtomicU64::new(0);

/// How many times the Rust core's BSP tree differed from the deciding Roc tree, process-wide
/// (each one a `roc-divergence:`; an operation's re-seeds the Rust tree from the Roc tree).
pub(crate) fn bsp_tree_reseeds() -> u64 { RESEEDS.load(Ordering::Relaxed) }

/// The deciding Roc tree. Queries step it too (`&self` in the layout system's interface), so it
/// sits in a cell; nothing else reaches it.
pub(super) struct RocTree {
    tree: RefCell<RocBspTree>,
}

impl RocTree {
    /// The Roc tree holding what `core` holds (a load).
    pub(super) fn seeded(core: &CoreTree) -> Self {
        Self {
            tree: RefCell::new(RocBspTree::seeded(&view(core))),
        }
    }

    /// An operation: the Roc tree takes `event` and decides, and its answer is rift's. The Rust
    /// tree took it too (answering `rust`); a different answer or view re-seeds the Rust tree
    /// from the Roc tree's view.
    pub(super) fn decided(
        &mut self,
        core: &mut CoreTree,
        what: &str,
        event: Event,
        rust: Answer,
    ) -> Answer {
        let tree = self.tree.get_mut();
        let answer = tree.step(event);
        let roc = tree.view();
        let difference = if answer != rust {
            Some(format!("answers: rust {rust:?}, roc {answer:?}"))
        } else {
            let held = view(core);
            (roc != held).then(|| format!("views: rust {held:?}, roc {roc:?}"))
        };
        if let Some(difference) = difference {
            report(what, &difference);
            *core = core_of(&roc);
        }
        answer
    }

    /// A query: the Roc tree answers, and its answer is rift's; the Rust tree's (`rust`) is held
    /// to it.
    pub(super) fn asked(&self, what: &str, event: Event, rust: Answer) -> Answer {
        let answer = self.tree.borrow_mut().step(event);
        if answer != rust {
            report(what, &format!("answers: rust {rust:?}, roc {answer:?}"));
        }
        answer
    }

    /// The whole system the Roc tree holds.
    #[cfg(test)]
    pub(super) fn view(&self) -> TreeView { self.tree.borrow_mut().view() }

    /// A layout's frames: the Roc tree's own shape through the Roc geometry, with the
    /// constraints rift knows.
    pub(super) fn frames(
        &self,
        layout: LayoutId,
        screen: Rect,
        gaps: Gaps,
        known: &[(u64, WindowConstraints)],
    ) -> Vec<(rift_core::plain::WindowId, Rect)> {
        self.tree.borrow_mut().frames(layout_key(layout), screen, gaps, known)
    }
}

/// Counts a difference, then fails the test, or in a running Rift warns it.
fn report(what: &str, difference: &str) {
    let reseeds = RESEEDS.fetch_add(1, Ordering::Relaxed) + 1;
    #[cfg(test)]
    panic!("roc-divergence ({reseeds} so far): the BSP trees after {what}: {difference}");
    #[cfg(not(test))]
    tracing::warn!(
        what,
        reseeds,
        difference,
        "roc-divergence: the Rust BSP tree differs from the deciding Roc tree"
    );
}

impl Clone for RocTree {
    /// A second Roc tree holding what this one holds.
    fn clone(&self) -> Self {
        let held = self.tree.borrow_mut().view();
        Self {
            tree: RefCell::new(RocBspTree::seeded(&held)),
        }
    }
}

impl std::fmt::Debug for RocTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("RocTree") }
}

/// What the Roc tree answered, as rift takes it (`parse`); an answer of another kind than
/// the operation's was a difference already reported (the Rust tree's answered the operation's
/// kind), and rift takes the Rust tree's answer.
pub(super) fn decided<T>(answer: Answer, parse: impl FnOnce(Answer) -> Option<T>, rust: T) -> T {
    parse(answer).unwrap_or(rust)
}

pub(super) fn as_verdict(answer: Answer) -> Option<bool> {
    match answer {
        Answer::Verdict(yes) => Some(yes),
        _ => None,
    }
}

pub(super) fn as_participated(answer: Answer) -> Option<bool> {
    match answer {
        Answer::Participated(changed) => Some(changed),
        _ => None,
    }
}

pub(super) fn as_layout(answer: Answer) -> Option<LayoutId> {
    match answer {
        Answer::Layout(key) => Some(layout_id(key)),
        _ => None,
    }
}

pub(super) fn as_slot(answer: Answer) -> Option<Option<Vec<usize>>> {
    match answer {
        Answer::WindowSlot(slot) => Some(slot),
        _ => None,
    }
}

pub(super) fn as_window(answer: Answer) -> Option<Option<WindowId>> {
    match answer {
        Answer::Window(word) => Some(word.map(window_id)),
        _ => None,
    }
}

pub(super) fn as_windows(answer: Answer) -> Option<Vec<WindowId>> {
    match answer {
        Answer::Windows(words) => Some(window_ids(&words)),
        _ => None,
    }
}

pub(super) fn as_focused(answer: Answer) -> Option<Focused<WindowId>> {
    match answer {
        Answer::Focused(focused) => Some(rift_focused(focused)),
        _ => None,
    }
}

pub(super) fn as_edited(answer: Answer) -> Option<Edited<WindowId>> {
    match answer {
        Answer::Edited(edited) => Some(rift_edited(edited)),
        _ => None,
    }
}

pub(super) fn layout_id(key: LayoutKey) -> LayoutId { slotmap::KeyData::from_ffi(key.0).into() }

fn node_id(key: NodeKey) -> NodeId { slotmap::KeyData::from_ffi(key.0).into() }

/// The window a Roc word names: the Roc tree holds only words rift gave it.
pub(super) fn window_id(word: u64) -> WindowId {
    window_of_word(word).expect("the Roc tree holds the windows rift gave it")
}

pub(super) fn window_ids(words: &[u64]) -> Vec<WindowId> {
    words.iter().map(|&word| window_id(word)).collect()
}

pub(super) fn rift_focused(focused: Focused<u64>) -> Focused<WindowId> {
    match focused {
        Focused::Selection { handled } => Focused::Selection { handled },
        Focused::Movement { window, raise } => Focused::Movement {
            window: window.map(window_id),
            raise: window_ids(&raise),
        },
    }
}

pub(super) fn rift_edited(edited: Edited<u64>) -> Edited<WindowId> {
    match edited {
        Edited::Unsupported => Edited::Unsupported,
        Edited::Unchanged => Edited::Unchanged,
        Edited::Changed => Edited::Changed,
        Edited::Raise(raised) => Edited::Raise(window_ids(&raised)),
    }
}

/// The Rust tree holding what a Roc tree's view holds (a re-seed): each table's slots, versions
/// and free list, the windows left out, the settings.
pub(super) fn core_of(view: &TreeView) -> CoreTree {
    let taking = |free: &[u32]| free.iter().rev().copied().collect::<Vec<u32>>();
    let layouts = SlotTable::parse(
        view.layouts
            .iter()
            .map(|(held, version)| {
                (
                    held.map(|layout| Layout { root: node_id(layout.root) }),
                    *version,
                )
            })
            .collect(),
    )
    .and_then(|table| table.resumed(&taking(&view.layout_free)))
    .expect("a Roc tree's layouts are a slot table");
    let nodes = SlotTable::parse(
        view.nodes
            .iter()
            .map(|(held, version)| (held.as_ref().map(rust_node), *version))
            .collect(),
    )
    .and_then(|table| table.resumed(&taking(&view.node_free)))
    .expect("a Roc tree's nodes are a slot table");
    CoreTree::from_parts(
        layouts,
        nodes,
        view.left_out.iter().map(|&word| window_id(word)).collect(),
        view.insertion,
        view.aspect,
    )
}

fn rust_node(node: &Node<NodeKey, u64>) -> Node<NodeId, WindowId> {
    Node {
        parent: match node.parent {
            Parent::Root => Parent::Root,
            Parent::Split(parent) => Parent::Split(node_id(parent)),
        },
        kind: match &node.kind {
            Kind::Split(split) => Kind::Split(Split {
                orientation: split.orientation,
                ratio: split.ratio,
                children: split.children.map(node_id),
                memory: split.memory,
            }),
            Kind::Leaf(leaf) => Kind::Leaf(Leaf {
                content: match &leaf.content {
                    Content::Empty => Content::Empty,
                    Content::Held { members, fill } => Content::Held {
                        members: match members {
                            Members::Window(word) => Members::Window(window_id(*word)),
                            Members::Stack(tabs) => Members::Stack(Tabs {
                                before: window_ids(&tabs.before),
                                top: window_id(tabs.top),
                                after: window_ids(&tabs.after),
                            }),
                        },
                        fill: *fill,
                    },
                },
                preselect: leaf.preselect,
            }),
        },
    }
}

pub(super) fn layout_key(layout: LayoutId) -> LayoutKey {
    LayoutKey(slotmap::Key::data(&layout).as_ffi())
}

fn node_key(node: NodeId) -> NodeKey { NodeKey(slotmap::Key::data(&node).as_ffi()) }

pub(super) fn window(window: WindowId) -> u64 { window.word() }

pub(super) fn windows(windows: &[WindowId]) -> Vec<u64> {
    windows.iter().map(|&w| window(w)).collect()
}

/// The windows the core's tree holds (in its layouts or left out) that `of_app` names: what
/// the core's predicate tells, listed for the Roc tree, which has no pid.
pub(super) fn app_windows(core: &CoreTree, of_app: impl Fn(WindowId) -> bool) -> Vec<u64> {
    let held = core.nodes().values().flat_map(|node| match &node.kind {
        Kind::Leaf(Leaf {
            content: Content::Held { members, .. },
            ..
        }) => members.windows().collect::<Vec<_>>(),
        Kind::Leaf(_) | Kind::Split(_) => Vec::new(),
    });
    let mut words: Vec<u64> = held
        .chain(core.left_out().iter().copied())
        .filter(|&w| of_app(w))
        .map(window)
        .collect();
    words.sort_unstable();
    words.dedup();
    words
}

pub(super) fn focus(focus: Focus<WindowId>) -> Focus<u64> {
    match focus {
        Focus::Select(w) => Focus::Select(window(w)),
        Focus::Ascend => Focus::Ascend,
        Focus::Descend => Focus::Descend,
        Focus::Direction(direction) => Focus::Direction(direction),
    }
}

pub(super) fn focused(focused: &Focused<WindowId>) -> Focused<u64> {
    match focused {
        Focused::Selection { handled } => Focused::Selection { handled: *handled },
        Focused::Movement { window: w, raise } => Focused::Movement {
            window: w.map(window),
            raise: windows(raise),
        },
    }
}

pub(super) fn edit(edit: Edit<WindowId>) -> Edit<u64> {
    match edit {
        Edit::Move(direction) => Edit::Move(direction),
        Edit::Split(kind) => Edit::Split(kind),
        Edit::Fullscreen { within_gaps } => Edit::Fullscreen { within_gaps },
        Edit::Swap(a, b) => Edit::Swap(window(a), window(b)),
        Edit::Drop { source, target, action } => Edit::Drop {
            source: window(source),
            target: window(target),
            action,
        },
        Edit::Join(direction) => Edit::Join(direction),
        Edit::ConsumeOrExpel(direction) => Edit::ConsumeOrExpel(direction),
        Edit::Unjoin => Edit::Unjoin,
        Edit::Resize { amount, along } => Edit::Resize { amount, along },
        Edit::Rebalance => Edit::Rebalance,
        Edit::ToggleOrientation => Edit::ToggleOrientation,
    }
}

pub(super) fn edited(edited: &Edited<WindowId>) -> Edited<u64> {
    match edited {
        Edited::Unsupported => Edited::Unsupported,
        Edited::Unchanged => Edited::Unchanged,
        Edited::Changed => Edited::Changed,
        Edited::Raise(raised) => Edited::Raise(windows(raised)),
    }
}

pub(super) fn query(query: core::Query<WindowId>) -> core::Query<u64> {
    match query {
        core::Query::All => core::Query::All,
        core::Query::Visible => core::Query::Visible,
        core::Query::Selection => core::Query::Selection,
        core::Query::Stack(w) => core::Query::Stack(window(w)),
    }
}

/// The core's tree as the Roc tree's view reads it (what `bsp_tree_seed` takes).
fn view(core: &CoreTree) -> TreeView {
    let mut left_out: Vec<u64> = core.left_out().iter().map(|&w| window(w)).collect();
    left_out.sort_unstable();
    TreeView {
        layouts: core
            .layouts()
            .saved()
            .map(|(held, version)| {
                (
                    held.map(|(_, layout)| Layout { root: node_key(layout.root) }),
                    version,
                )
            })
            .collect(),
        layout_free: core.layouts().free().to_vec(),
        nodes: core
            .nodes()
            .saved()
            .map(|(held, version)| (held.map(|(_, node)| roc_node(node)), version))
            .collect(),
        node_free: core.nodes().free().to_vec(),
        left_out,
        insertion: core.insertion(),
        aspect: core.aspect(),
    }
}

fn roc_node(node: &Node<NodeId, WindowId>) -> Node<NodeKey, u64> {
    Node {
        parent: match node.parent {
            Parent::Root => Parent::Root,
            Parent::Split(parent) => Parent::Split(node_key(parent)),
        },
        kind: match &node.kind {
            Kind::Split(split) => Kind::Split(Split {
                orientation: split.orientation,
                ratio: split.ratio,
                children: split.children.map(node_key),
                memory: split.memory,
            }),
            Kind::Leaf(leaf) => Kind::Leaf(Leaf {
                content: match &leaf.content {
                    Content::Empty => Content::Empty,
                    Content::Held { members, fill } => Content::Held {
                        members: match members {
                            Members::Window(w) => Members::Window(window(*w)),
                            Members::Stack(tabs) => Members::Stack(Tabs {
                                before: windows(&tabs.before),
                                top: window(tabs.top),
                                after: windows(&tabs.after),
                            }),
                        },
                        fill: *fill,
                    },
                },
                preselect: leaf.preselect,
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout_engine::systems::bsp::BspLayoutSystem;
    use crate::layout_engine::systems::{LayoutSystem, WindowChange};

    /// A Rust tree that differs from the deciding Roc tree (here: the Roc tree lost a window) is
    /// counted and fails a test (a running Rift warns and re-seeds the Rust tree from the Roc
    /// tree), on an operation and on a query alike.
    #[test]
    fn trees_that_differ_are_counted_and_fail_tests() {
        for query in [false, true] {
            let mut system = BspLayoutSystem::default();
            let layout = system.create_layout();
            system.update(WindowChange::Add(layout, WindowId::new(1, 1), None));
            system.roc.tree.get_mut().step(Event::RemoveWindow(window(WindowId::new(1, 1))));
            let before = bsp_tree_reseeds();
            let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if query {
                    system.selected_window(layout);
                } else {
                    system.update(WindowChange::Add(layout, WindowId::new(1, 2), None));
                }
            }));
            assert!(failed.is_err());
            assert!(bsp_tree_reseeds() > before);
        }
    }

    /// The Roc shadow's cost at 10, 100 and 1000 windows (release: `cargo test --release --lib
    /// --features roc-core measure_bsp_roc_tree -- --ignored --nocapture`): the Roc tree's
    /// step and a query alone, the view comparison (the core's view, the Roc tree's, compared),
    /// and a re-seed, each on its own. Under `roc-core` every operation pays a step and a view.
    #[test]
    #[ignore]
    fn measure_bsp_roc_tree() {
        use std::hint::black_box;
        use std::time::Instant;

        use rift_core::layout::bsp_tree::{Direction, ResizeAlong};

        for count in [10_u32, 100, 1000] {
            let mut system = BspLayoutSystem::default();
            let layout = system.create_layout();
            for idx in 1..=count {
                system.update(WindowChange::Add(layout, WindowId::new(1, idx), None));
            }
            let core = system.core.clone();
            let key = layout_key(layout);
            let mut roc = RocBspTree::seeded(&view(&core));
            let directions = [
                Direction::Left,
                Direction::Up,
                Direction::Right,
                Direction::Down,
            ];
            let w = |idx: u32| window(WindowId::new(1, idx));
            let ops = 4000_u32;
            let started = Instant::now();
            for i in 0..ops {
                black_box(roc.step(match i % 4 {
                    0 => Event::Focus(key, Focus::Direction(directions[(i / 4 % 4) as usize])),
                    1 => Event::Edit(key, Edit::Swap(w(1 + i % count), w(1 + (i * 7) % count))),
                    2 => Event::Edit(key, Edit::Resize {
                        amount: 0.05,
                        along: ResizeAlong::Smart,
                    }),
                    _ => Event::SelectedWindow(key),
                }));
            }
            let step = started.elapsed() / ops;
            let rounds = 200;
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(view(&core) == roc.view());
            }
            let compared = started.elapsed() / rounds;
            let started = Instant::now();
            for _ in 0..rounds / 10 {
                black_box(RocBspTree::seeded(&view(&core)));
            }
            let reseed = started.elapsed() / (rounds / 10);
            let started = Instant::now();
            for _ in 0..rounds / 10 {
                black_box(core_of(&roc.view()));
            }
            let rust_reseed = started.elapsed() / (rounds / 10);
            // A layout pass: the Roc tree's own shape through the Roc geometry, against the
            // Rust tree's shape through the Rust geometry.
            let screen = Rect {
                x: 0.0,
                y: 0.0,
                w: 3000.0,
                h: 2000.0,
            };
            let gaps = Gaps {
                top: 0.0,
                left: 0.0,
                bottom: 0.0,
                right: 0.0,
                horizontal: 0.0,
                vertical: 0.0,
            };
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(roc.frames(key, screen, gaps, &[]));
            }
            let roc_pass = started.elapsed() / rounds;
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(core.shape(layout, screen, gaps, |_| None).map(|shape| shape.frames()));
            }
            let rust_pass = started.elapsed() / rounds;
            eprintln!(
                "{count} windows: roc step {step:?}/op, view comparison {compared:?}/op, re-seed \
                 of the Roc tree {reseed:?}, of the Rust tree {rust_reseed:?}; a pass: roc \
                 {roc_pass:?}, rust {rust_pass:?}"
            );
        }
    }
}
