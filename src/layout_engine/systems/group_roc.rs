//! Under `roc-core`, the Roc forest is a second shadow of the core's (layout trees, the group
//! forest's slice 3), one machine per system ([`RocTraditional`], [`RocMasterStack`]): seeded
//! from the core at load, stepped with every operation (queries too, through a cell: the layout
//! system's queries take `&self`), its answer and whole view compared with the core's on every
//! operation. A difference is counted ([`group_forest_reseeds`]), fails a test, or in a running
//! Rift warns `roc-divergence:` and re-seeds the Roc forest from the core (the deciding side
//! stands; a query's difference is only reported, and the next operation compares the views).
//!
//! The Roc forest holds the core's ids as words: layouts and nodes by slotmap's ffi form,
//! windows as [`Word`] makes them, which order as `WindowId` does (pid, then index), so the
//! windows left out and an app's windows agree in order.

use std::cell::RefCell;
use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};

use rift_core::layout::group_forest::{
    Body, Container, Edit, Edited, Focus, Focused, Members, Memory, Node, Place, Query,
};
use rift_core_roc::group_forest::{
    Answer, ForestView, LayoutKey, MasterStackView, NodeKey, TraditionalView,
};
use rift_core_roc::{RocMasterStack, RocTraditional};

use super::group_twin::CoreForest;
use crate::actor::app::WindowId;
use crate::layout_engine::LayoutId;
use crate::model::tree::NodeId;
use crate::roc_path::Word;

static RESEEDS: AtomicU64 = AtomicU64::new(0);

/// How many times the Roc forest differed from the core's, process-wide (each one a
/// `roc-divergence:`; an operation's re-seeds the Roc forest from the core).
pub(crate) fn group_forest_reseeds() -> u64 { RESEEDS.load(Ordering::Relaxed) }

/// A system's Roc machine: seeded from a view, stepped by its events, read back as a view.
pub(super) trait Machine: Sized {
    type Event: Debug;
    type View: PartialEq + Debug + Clone;
    fn seeded(view: &Self::View) -> Self;
    fn step(&mut self, event: Self::Event) -> Answer;
    fn view(&mut self) -> Self::View;
    /// A view's forest.
    fn forest(view: &mut Self::View) -> &mut ForestView;
}

impl Machine for RocTraditional {
    type Event = rift_core_roc::group_forest::TraditionalEvent;
    type View = TraditionalView;

    fn seeded(view: &TraditionalView) -> Self { RocTraditional::seeded(view) }

    fn step(&mut self, event: Self::Event) -> Answer { RocTraditional::step(self, event) }

    fn view(&mut self) -> TraditionalView { RocTraditional::view(self) }

    fn forest(view: &mut TraditionalView) -> &mut ForestView { &mut view.forest }
}

impl Machine for RocMasterStack {
    type Event = rift_core_roc::group_forest::MasterStackEvent;
    type View = MasterStackView;

    fn seeded(view: &MasterStackView) -> Self { RocMasterStack::seeded(view) }

    fn step(&mut self, event: Self::Event) -> Answer { RocMasterStack::step(self, event) }

    fn view(&mut self) -> MasterStackView { RocMasterStack::view(self) }

    fn forest(view: &mut MasterStackView) -> &mut ForestView { &mut view.forest }
}

/// The Roc forest of one system, the core's second shadow. Queries step it too, so it sits in a
/// cell; nothing else reaches it.
pub(super) struct RocShadow<M> {
    machine: RefCell<M>,
}

impl<M: Machine> RocShadow<M> {
    /// The Roc forest holding what the core holds (`held`, the core's view: a load, or a
    /// re-seed).
    pub(super) fn seeded(held: &M::View) -> Self {
        Self {
            machine: RefCell::new(M::seeded(held)),
        }
    }

    /// An operation the core decided (answering `rust`, holding `held` after it): the Roc forest
    /// takes it too; a different answer or view re-seeds it from the core.
    pub(super) fn stepped(
        &mut self,
        what: &str,
        event: M::Event,
        rust: Answer,
        held: impl FnOnce() -> M::View,
    ) {
        let machine = self.machine.get_mut();
        let answer = machine.step(event);
        let roc = ranked::<M>(machine.view());
        let held = held();
        let difference = if answer != rust {
            Some(format!("answers: rust {rust:?}, roc {answer:?}"))
        } else {
            let ranked_held = ranked::<M>(held.clone());
            (roc != ranked_held).then(|| format!("views: rust {ranked_held:?}, roc {roc:?}"))
        };
        if let Some(difference) = difference {
            report(what, &difference);
            *self = Self::seeded(&held);
        }
    }

    /// A query the core answered (`rust`): the Roc forest's answer is held to it.
    pub(super) fn asked(&self, what: &str, event: M::Event, rust: Answer) {
        let answer = self.machine.borrow_mut().step(event);
        if answer != rust {
            report(what, &format!("answers: rust {rust:?}, roc {answer:?}"));
        }
    }

    #[cfg(test)]
    pub(super) fn machine(&mut self) -> &mut M { self.machine.get_mut() }
}

/// A view with each leaf's stamp written as its rank among the leaves' stamps. Only the stamps'
/// order is behaviour (it orders each window's leaves); their values are not: a forest seeded
/// from a view derives its next stamp from the leaves, while a live one keeps counting past
/// leaves since removed, so a Roc forest seeded from the core stamps lower from then on.
fn ranked<M: Machine>(mut view: M::View) -> M::View {
    let forest = M::forest(&mut view);
    let stamp = |node: &Node<NodeKey, u64>| match node.body {
        Body::Window { stamp, .. } => Some(stamp),
        Body::Container(_) => None,
    };
    let mut stamps: Vec<u64> = forest
        .nodes
        .iter()
        .filter_map(|(node, _)| node.as_ref().and_then(stamp))
        .collect();
    stamps.sort_unstable();
    for (node, _) in &mut forest.nodes {
        if let Some(Node {
            body: Body::Window { stamp, .. },
            ..
        }) = node
        {
            *stamp = stamps.partition_point(|&held| held < *stamp) as u64;
        }
    }
    view
}

/// Counts a difference, then fails the test, or in a running Rift warns it.
fn report(what: &str, difference: &str) {
    let reseeds = RESEEDS.fetch_add(1, Ordering::Relaxed) + 1;
    #[cfg(test)]
    panic!("roc-divergence ({reseeds} so far): the group forest after {what}: {difference}");
    #[cfg(not(test))]
    tracing::warn!(
        what,
        reseeds,
        difference,
        "roc-divergence: the Roc group forest differs from the core's; re-seeded from the core"
    );
}

impl<M: Machine> Clone for RocShadow<M> {
    /// A second Roc forest holding what this one holds.
    fn clone(&self) -> Self { Self::seeded(&self.machine.borrow_mut().view()) }
}

impl<M> Debug for RocShadow<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("RocShadow") }
}

pub(super) fn layout_key(layout: LayoutId) -> LayoutKey {
    LayoutKey(slotmap::Key::data(&layout).as_ffi())
}

fn node_key(node: NodeId) -> NodeKey { NodeKey(slotmap::Key::data(&node).as_ffi()) }

pub(super) fn window(window: WindowId) -> u64 { window.word() }

pub(super) fn windows(windows: &[WindowId]) -> Vec<u64> {
    windows.iter().copied().map(window).collect()
}

/// The windows the core's forest holds (in its layouts or left out) that `of_app` tells, in
/// order: an app closing, or a has-windows-for-app question, as Roc has no pid.
pub(super) fn app_windows(forest: &CoreForest, of_app: impl Fn(WindowId) -> bool) -> Vec<u64> {
    let held = forest.nodes().values().filter_map(|node| match node.body {
        Body::Window { window, .. } => Some(window),
        Body::Container(_) => None,
    });
    let mut words: Vec<u64> = held
        .chain(forest.left_out().iter().copied())
        .filter(|&held| of_app(held))
        .map(window)
        .collect();
    words.sort_unstable();
    words.dedup();
    words
}

pub(super) fn focus(focus: Focus<WindowId>) -> Focus<u64> {
    match focus {
        Focus::Select(held) => Focus::Select(window(held)),
        Focus::Ascend => Focus::Ascend,
        Focus::Descend => Focus::Descend,
        Focus::Direction(direction) => Focus::Direction(direction),
    }
}

pub(super) fn focused(focused: &Focused<WindowId>) -> Answer {
    Answer::Focused(match focused {
        Focused::Selection { handled } => Focused::Selection { handled: *handled },
        Focused::Movement { window: held, raise } => Focused::Movement {
            window: held.map(window),
            raise: windows(raise),
        },
    })
}

pub(super) fn edit(edit: Edit<WindowId>) -> Edit<u64> {
    match edit {
        Edit::Move { direction, beyond } => Edit::Move { direction, beyond },
        Edit::Split(kind) => Edit::Split(kind),
        Edit::Fullscreen { within_gaps } => Edit::Fullscreen { within_gaps },
        Edit::Swap(a, b) => Edit::Swap(window(a), window(b)),
        Edit::ReorderTab { window: held, target } => Edit::ReorderTab {
            window: window(held),
            target: window(target),
        },
        Edit::Drop { source, target, action } => Edit::Drop {
            source: window(source),
            target: window(target),
            action,
        },
        Edit::Join(direction) => Edit::Join(direction),
        Edit::ConsumeOrExpel(direction) => Edit::ConsumeOrExpel(direction),
        Edit::Stack(default) => Edit::Stack(default),
        Edit::Unstack(default) => Edit::Unstack(default),
        Edit::Unjoin => Edit::Unjoin,
        Edit::Resize { amount, along } => Edit::Resize { amount, along },
        Edit::Rebalance => Edit::Rebalance,
        Edit::ToggleOrientation => Edit::ToggleOrientation,
    }
}

pub(super) fn edited(edited: &Edited<WindowId>) -> Answer {
    Answer::Edited(match edited {
        Edited::Unsupported => Edited::Unsupported,
        Edited::Unchanged => Edited::Unchanged,
        Edited::Changed => Edited::Changed,
        Edited::Raise(raised) => Edited::Raise(windows(raised)),
    })
}

pub(super) fn query(query: Query<WindowId>) -> Query<u64> {
    match query {
        Query::All => Query::All,
        Query::Visible => Query::Visible,
        Query::Selection => Query::Selection,
        Query::Stack(held) => Query::Stack(window(held)),
    }
}

/// A node on the Roc forest's keys and words.
fn roc_node(node: &Node<NodeId, WindowId>) -> Node<NodeKey, u64> {
    let memory = |memory: Memory<NodeId>| match memory {
        Memory::Unset => Memory::Unset,
        Memory::Toward(child) => Memory::Toward(node_key(child)),
        Memory::Stops(child) => Memory::Stops(node_key(child)),
    };
    Node {
        place: match node.place {
            Place::Root => Place::Root,
            Place::Child { parent, size } => Place::Child { parent: node_key(parent), size },
        },
        fill: node.fill,
        body: match &node.body {
            Body::Window { window: held, stamp } => Body::Window {
                window: window(*held),
                stamp: *stamp,
            },
            Body::Container(container) => Body::Container(Container {
                kind: container.kind,
                members: match &container.members {
                    Members::Children(children) => {
                        Members::Children(children.iter().copied().map(node_key).collect())
                    }
                    Members::Regions { order, primary } => Members::Regions {
                        order: order.map(node_key),
                        primary: *primary,
                    },
                },
                memory: memory(container.memory),
            }),
        },
    }
}

/// The core's forest as the Roc forest's view gives it.
pub(super) fn forest_view(forest: &CoreForest) -> ForestView {
    ForestView {
        layouts: forest
            .layout_table()
            .saved()
            .map(|(held, version)| (held.map(|(_, &root)| node_key(root)), version))
            .collect(),
        layout_free: forest.layout_table().free().to_vec(),
        nodes: forest
            .nodes()
            .saved()
            .map(|(held, version)| (held.map(|(_, node)| roc_node(node)), version))
            .collect(),
        node_free: forest.nodes().free().to_vec(),
        left_out: windows(&forest.left_out().iter().copied().collect::<Vec<_>>()),
    }
}
