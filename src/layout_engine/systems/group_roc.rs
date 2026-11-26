//! Under `roc-core`, the Roc forest decides (layout trees, the group forest's slice 4), one
//! machine per system ([`RocTraditional`], [`RocMasterStack`]): it takes every operation, queries
//! included, and its answers are rift's, as are a layout's frames (its own shape through the Roc
//! traditional geometry), a pass's group bars and rift-cli's views. The Rust core's forest, the
//! Roc forest's shadow, takes every operation too; its answer and whole view are compared with
//! the Roc forest's on every operation, and its frames, bars and views with the Roc forest's on
//! every call. A difference is counted ([`group_forest_reseeds`]), fails a test, or in a running
//! Rift warns `roc-divergence:`; an operation's re-seeds the Rust forest from the Roc forest's
//! view (a query's or a view's is only reported: the Rust forest is not changed by one, and the
//! next operation compares the views).
//!
//! The Roc forest holds the core's ids as words: layouts and nodes by slotmap's ffi form,
//! windows as [`Word`] makes them, which order as `WindowId` does (pid, then index), so the
//! windows left out and an app's windows agree in order.

use std::cell::RefCell;
use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};

use rift_core::layout::WindowConstraints;
use rift_core::layout::group_forest::{
    Bars, Body, ContainerView, Drawn, Edit, Edited, Focus, Focused, GroupBar, Held, Pass, Query,
    Row,
};
use rift_core::plain::Rect;
use rift_core_roc::group_forest::{self as plain, Answer, ForestView, LayoutKey, NodeKey};
use rift_core_roc::{RocMasterStack, RocTraditional};

use super::group_system::CoreForest;
use crate::actor::app::WindowId;
use crate::common::collections::HashMap;
use crate::layout_engine::LayoutId;
use crate::layout_engine::systems::WindowLayoutConstraints;
use crate::model::tree::NodeId;
use crate::roc_path::{Word, window_of_word};

static RESEEDS: AtomicU64 = AtomicU64::new(0);

/// How many times the Rust core's forest differed from the deciding Roc forest, process-wide
/// (each one a `roc-divergence:`; an operation's re-seeds the Rust forest from the Roc forest).
pub(crate) fn group_forest_reseeds() -> u64 { RESEEDS.load(Ordering::Relaxed) }

/// A system's Roc machine: seeded from a view, stepped by its events, read back as a view, and
/// asked its layouts' questions; and the Rust system it decides for.
pub(super) trait Machine: Sized {
    type Event: Debug;
    type View: PartialEq + Debug;
    /// The Rust system, the machine's shadow.
    type Core;
    fn seeded(view: &Self::View) -> Self;
    fn step(&mut self, event: Self::Event) -> Answer;
    fn view(&mut self) -> Self::View;
    fn frames(
        &mut self,
        layout: LayoutKey,
        pass: Pass,
        known: &[(u64, WindowConstraints)],
    ) -> Vec<(rift_core::plain::WindowId, Rect)>;
    fn project(
        &mut self,
        layout: LayoutKey,
        pass: Pass,
        known: &[(u64, WindowConstraints)],
        bars: Bars,
    ) -> plain::Projected;
    fn container_view(&mut self, layout: LayoutKey) -> plain::ContainerView;
    fn rows(&mut self, layout: LayoutKey) -> Vec<plain::Row>;
    /// The Rust system as the machine's view gives it.
    fn view_of(core: &Self::Core) -> Self::View;
    /// The Rust system holding what a machine's view holds (a re-seed).
    fn core_of(view: &Self::View) -> Self::Core;
}

macro_rules! machine_asks {
    () => {
        fn seeded(view: &Self::View) -> Self { Self::seeded(view) }

        fn step(&mut self, event: Self::Event) -> Answer { Self::step(self, event) }

        fn view(&mut self) -> Self::View { Self::view(self) }

        fn frames(
            &mut self,
            layout: LayoutKey,
            pass: Pass,
            known: &[(u64, WindowConstraints)],
        ) -> Vec<(rift_core::plain::WindowId, Rect)> {
            Self::frames(self, layout, pass, known)
        }

        fn project(
            &mut self,
            layout: LayoutKey,
            pass: Pass,
            known: &[(u64, WindowConstraints)],
            bars: Bars,
        ) -> plain::Projected {
            Self::project(self, layout, pass, known, bars)
        }

        fn container_view(&mut self, layout: LayoutKey) -> plain::ContainerView {
            Self::container_view(self, layout)
        }

        fn rows(&mut self, layout: LayoutKey) -> Vec<plain::Row> { Self::rows(self, layout) }
    };
}

impl Machine for RocTraditional {
    type Core = super::traditional::CoreTraditional;
    type Event = plain::TraditionalEvent;
    type View = plain::TraditionalView;

    machine_asks!();

    fn view_of(core: &Self::Core) -> Self::View {
        plain::TraditionalView {
            forest: forest_view(core.forest()),
            insertion: core.insertion(),
            new_sibling: core.new_sibling(),
        }
    }

    fn core_of(view: &Self::View) -> Self::Core {
        Self::Core::from_parts(core_forest(&view.forest), view.insertion, view.new_sibling)
    }
}

impl Machine for RocMasterStack {
    type Core = super::master_stack::CoreMasterStack;
    type Event = plain::MasterStackEvent;
    type View = plain::MasterStackView;

    machine_asks!();

    fn view_of(core: &Self::Core) -> Self::View {
        plain::MasterStackView {
            forest: forest_view(core.forest()),
            settings: core.settings(),
        }
    }

    fn core_of(view: &Self::View) -> Self::Core {
        Self::Core::from_parts(core_forest(&view.forest), view.settings)
    }
}

/// A layout pass as the Roc forest decides it: each window's frame (the geometry's windows) and
/// each group bar, in tree order.
pub(super) type Projected = (
    Vec<(rift_core::plain::WindowId, Rect)>,
    Vec<GroupBar<NodeId, WindowId>>,
);

/// The deciding Roc forest of one system. Queries step it too (`&self` in the layout system's
/// interface), so it sits in a cell; nothing else reaches it.
pub(super) struct RocForest<M> {
    machine: RefCell<M>,
}

impl<M: Machine> RocForest<M> {
    /// The Roc forest holding what `core` holds (a load).
    pub(super) fn seeded(core: &M::Core) -> Self {
        Self {
            machine: RefCell::new(M::seeded(&M::view_of(core))),
        }
    }

    /// An operation: the Roc forest takes `event` and decides, and its answer is rift's. The
    /// Rust forest took it too (answering `rust`); a different answer or view re-seeds the Rust
    /// forest from the Roc forest's view.
    pub(super) fn decided(
        &mut self,
        core: &mut M::Core,
        what: &str,
        event: M::Event,
        rust: Answer,
    ) -> Answer {
        let machine = self.machine.get_mut();
        let answer = machine.step(event);
        let roc = machine.view();
        let difference = if answer != rust {
            Some(format!("answers: rust {rust:?}, roc {answer:?}"))
        } else {
            let held = M::view_of(core);
            (roc != held).then(|| format!("views: rust {held:?}, roc {roc:?}"))
        };
        if let Some(difference) = difference {
            report(what, &difference);
            *core = M::core_of(&roc);
        }
        answer
    }

    /// A query: the Roc forest answers, and its answer is rift's; the Rust forest's (`rust`) is
    /// held to it.
    pub(super) fn asked(&self, what: &str, event: M::Event, rust: Answer) -> Answer {
        let answer = self.machine.borrow_mut().step(event);
        if answer != rust {
            report(what, &format!("answers: rust {rust:?}, roc {answer:?}"));
        }
        answer
    }

    /// A layout's frames: the Roc forest's own shape through the Roc traditional geometry, with
    /// the constraints rift knows.
    pub(super) fn frames(
        &self,
        layout: LayoutId,
        pass: Pass,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
    ) -> Vec<(rift_core::plain::WindowId, Rect)> {
        self.machine.borrow_mut().frames(layout_key(layout), pass, &known(constraints))
    }

    /// A layout pass with its group bars, the Roc forest's; the Rust forest's bars (`rust`) are
    /// held to its bars.
    pub(super) fn project(
        &self,
        layout: LayoutId,
        pass: Pass,
        constraints: &HashMap<WindowId, WindowLayoutConstraints>,
        bars: Bars,
        rust: &[GroupBar<NodeId, WindowId>],
    ) -> Projected {
        let projected =
            self.machine
                .borrow_mut()
                .project(layout_key(layout), pass, &known(constraints), bars);
        let roc: Vec<GroupBar<NodeId, WindowId>> = projected
            .bars
            .into_iter()
            .map(|bar| GroupBar {
                stack: node_id(bar.stack),
                orientation: bar.orientation,
                frame: bar.frame,
                windows: window_ids(&bar.windows),
                selected: bar.selected,
            })
            .collect();
        (projected.frames, viewed("project_groups", rust, roc))
    }

    /// rift-cli's container view of a layout, the Roc forest's; the Rust forest's (`rust`) is held
    /// to it.
    pub(super) fn container_view(
        &self,
        layout: LayoutId,
        rust: ContainerView<NodeId, WindowId>,
    ) -> ContainerView<NodeId, WindowId> {
        let roc = rift_view(self.machine.borrow_mut().container_view(layout_key(layout)));
        viewed("container_tree", rust, roc)
    }

    /// rift-cli's drawing of a layout, from the Roc forest's rows; the Rust forest's rows
    /// (`rust`) are held to them.
    pub(super) fn draw(&self, layout: LayoutId, rust: Vec<Row<NodeId, WindowId>>) -> String {
        let roc: Vec<Row<NodeId, WindowId>> = self
            .machine
            .borrow_mut()
            .rows(layout_key(layout))
            .into_iter()
            .map(|row| Row {
                branches: row.branches,
                mark: row.mark,
                node: node_id(row.node),
                held: match row.held {
                    Drawn::Window(word) => Drawn::Window(window_id(word)),
                    Drawn::Container { kind, total } => Drawn::Container { kind, total },
                },
                size: row.size,
                role: row.role,
            })
            .collect();
        rift_core::layout::group_forest::render(&viewed("draw_tree", rust, roc))
    }

    /// The whole system the Roc forest holds.
    #[cfg(test)]
    pub(super) fn view(&self) -> M::View { self.machine.borrow_mut().view() }

    #[cfg(test)]
    pub(super) fn machine(&mut self) -> &mut M { self.machine.get_mut() }
}

/// A view or bars the Roc forest gave (`roc`, rift's), the Rust forest's (`rust`) held to it: a
/// difference is reported (the next operation compares the systems).
fn viewed<A: Debug, R: PartialEq<A> + Debug>(what: &str, rust: R, roc: A) -> A {
    if rust != roc {
        report(what, &format!("views: rust {rust:?}, roc {roc:?}"));
    }
    roc
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
        "roc-divergence: the Rust group forest differs from the deciding Roc forest; re-seeded \
         from it"
    );
}

impl<M: Machine> Clone for RocForest<M> {
    /// A second Roc forest holding what this one holds.
    fn clone(&self) -> Self {
        let held = self.machine.borrow_mut().view();
        Self {
            machine: RefCell::new(M::seeded(&held)),
        }
    }
}

impl<M> Debug for RocForest<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("RocForest") }
}

/// Every window's constraints rift knows, by the window's word.
fn known(
    constraints: &HashMap<WindowId, WindowLayoutConstraints>,
) -> Vec<(u64, WindowConstraints)> {
    constraints
        .iter()
        .map(|(&held, &c)| {
            (
                held.word(),
                crate::layout_engine::core_geometry::core_constraints(c),
            )
        })
        .collect()
}

pub(super) fn layout_key(layout: LayoutId) -> LayoutKey {
    LayoutKey(slotmap::Key::data(&layout).as_ffi())
}

fn node_key(node: NodeId) -> NodeKey { NodeKey(slotmap::Key::data(&node).as_ffi()) }

fn node_id(key: NodeKey) -> NodeId { slotmap::KeyData::from_ffi(key.0).into() }

pub(super) fn window(window: WindowId) -> u64 { window.word() }

pub(super) fn windows(windows: &[WindowId]) -> Vec<u64> {
    windows.iter().copied().map(window).collect()
}

/// The window a Roc word names: the Roc forest holds only words rift gave it.
fn window_id(word: u64) -> WindowId {
    window_of_word(word).expect("the Roc forest holds the windows rift gave it")
}

fn window_ids(words: &[u64]) -> Vec<WindowId> {
    words.iter().map(|&word| window_id(word)).collect()
}

/// A container view on rift's ids.
fn rift_view(view: plain::ContainerView) -> ContainerView<NodeId, WindowId> {
    ContainerView {
        node: node_id(view.node),
        selected: view.selected,
        weight: view.weight,
        fill: view.fill,
        role: view.role,
        held: match view.held {
            Held::Window(word) => Held::Window(window_id(word)),
            Held::Container { kind, children } => Held::Container {
                kind,
                children: children.into_iter().map(rift_view).collect(),
            },
        },
    }
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

/// The core's forest as the Roc forest's view gives it.
fn forest_view(forest: &CoreForest) -> ForestView {
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
            .map(|(held, version)| (held.map(|(_, node)| node.map(node_key, window)), version))
            .collect(),
        node_free: forest.nodes().free().to_vec(),
        left_out: windows(&forest.left_out().iter().copied().collect::<Vec<_>>()),
        next_stamp: forest.next_stamp(),
    }
}

/// The core's forest holding what a Roc forest's view holds (a re-seed): both tables' slots,
/// versions and free lists, the windows left out, the stamp counter.
fn core_forest(view: &ForestView) -> CoreForest {
    view.forest_on(node_id, window_id)
        .expect("the Roc forest's tables are slot tables")
}
