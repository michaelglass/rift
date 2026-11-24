//! Under `roc-core`, the Roc BSP tree ([`RocBspTree`]) as a shadow of the core's deciding tree
//! (layout trees, slice 4): it takes every operation the core takes, queries included, and its
//! answer and whole view are compared with the core's on every operation. A difference is counted
//! ([`bsp_tree_reseeds`]), fails a test, or in a running Rift warns `roc-divergence:` and re-seeds
//! the Roc tree from the core through `bsp_tree_seed`.
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
use rift_core_roc::RocBspTree;
use rift_core_roc::bsp_tree::{Answer, Event, LayoutKey, NodeKey, TreeView};

use super::twinned::CoreTree;
use crate::actor::app::WindowId;
use crate::layout_engine::LayoutId;
use crate::model::tree::NodeId;
use crate::roc_path::Word;

static RESEEDS: AtomicU64 = AtomicU64::new(0);

/// How many times the Roc BSP tree has been re-seeded from the core's tree, process-wide (each
/// one a `roc-divergence:`).
pub(crate) fn bsp_tree_reseeds() -> u64 { RESEEDS.load(Ordering::Relaxed) }

/// The Roc tree, stepped beside the core's. Queries step it too (`&self` in the layout system's
/// interface), so it sits in a cell; nothing else reaches it.
pub(super) struct RocShadow {
    tree: RefCell<RocBspTree>,
}

impl RocShadow {
    /// The Roc tree holding what `core` holds.
    pub(super) fn seeded(core: &CoreTree) -> Self {
        Self {
            tree: RefCell::new(RocBspTree::seeded(&view(core))),
        }
    }

    /// After an operation the core took (answering `expected`): the Roc tree takes `event`, and
    /// its answer and whole view are compared with the core's.
    pub(super) fn stepped(&mut self, core: &CoreTree, what: &str, event: Event, expected: Answer) {
        let tree = self.tree.get_mut();
        let answer = tree.step(event);
        let difference = if answer != expected {
            Some(format!("answers: core {expected:?}, roc {answer:?}"))
        } else {
            let (held, roc) = (view(core), tree.view());
            (roc != held).then(|| format!("views: core {held:?}, roc {roc:?}"))
        };
        if let Some(difference) = difference {
            report(what, &difference);
            *tree = RocBspTree::seeded(&view(core));
        }
    }

    /// A query the core answered `expected`: the Roc tree answers it too.
    pub(super) fn asked(&self, core: &CoreTree, what: &str, event: Event, expected: Answer) {
        let mut tree = self.tree.borrow_mut();
        let answer = tree.step(event);
        if answer != expected {
            report(what, &format!("answers: core {expected:?}, roc {answer:?}"));
            *tree = RocBspTree::seeded(&view(core));
        }
    }
}

/// Counts a difference, then fails the test, or in a running Rift warns it (the caller re-seeds
/// the Roc tree from the core).
fn report(what: &str, difference: &str) {
    let reseeds = RESEEDS.fetch_add(1, Ordering::Relaxed) + 1;
    #[cfg(test)]
    panic!("roc-divergence ({reseeds} so far): the Roc BSP tree after {what}: {difference}");
    #[cfg(not(test))]
    tracing::warn!(
        what,
        reseeds,
        difference,
        "roc-divergence: the Roc BSP tree differs from the core's; re-seeded from the core"
    );
}

impl Clone for RocShadow {
    /// A second Roc tree holding what this one holds.
    fn clone(&self) -> Self {
        let held = self.tree.borrow_mut().view();
        Self {
            tree: RefCell::new(RocBspTree::seeded(&held)),
        }
    }
}

impl std::fmt::Debug for RocShadow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("RocShadow") }
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

    /// A Roc tree that differs from the core's (here: a layout the core lacks) is counted and
    /// fails a test (a running Rift warns and re-seeds it from the core), on an operation and on
    /// a query alike.
    #[test]
    fn a_roc_tree_that_differs_is_counted_and_fails_tests() {
        for query in [false, true] {
            let mut system = BspLayoutSystem::default();
            let layout = system.create_layout();
            system.update(WindowChange::Add(layout, WindowId::new(1, 1), None));
            system
                .shadow
                .roc
                .tree
                .get_mut()
                .step(Event::RemoveWindow(window(WindowId::new(1, 1))));
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
    /// --features roc-core measure_bsp_roc_shadow -- --ignored --nocapture`): the Roc tree's
    /// step and a query alone, the view comparison (the core's view, the Roc tree's, compared),
    /// and a re-seed, each on its own. Under `roc-core` every operation pays a step and a view.
    #[test]
    #[ignore]
    fn measure_bsp_roc_shadow() {
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
            eprintln!(
                "{count} windows: roc step {step:?}/op, view comparison {compared:?}/op, re-seed \
                 {reseed:?}"
            );
        }
    }
}
