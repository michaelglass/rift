//! Under `roc-core`, the Roc floating system (`RocFloating`) beside the core's (layout trees,
//! floating's step 4): it takes every operation and query the core's takes, its answers held to
//! the core's on every call, the shape, group bars and rift-cli's views on every call, and the
//! whole system (the forest's view and every window's kept frame) on every operation in tests
//! and debug builds and every [`VIEW_EVERY`]th in a `roc-core` release build. The core decides;
//! a difference is counted ([`floating_roc_reseeds`]), fails a test, or in a running Rift warns
//! `roc-divergence:` and re-seeds the Roc system from the core's.

use std::cell::RefCell;
use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};

use rift_core::layout::floating::{Floating, Layout};
use rift_core::layout::group_forest::{Focus, Query};
use rift_core::layout::{Gaps, WindowConstraints};
use rift_core_roc::RocFloating;
use rift_core_roc::group_forest::{Answer, FloatingAnswer, FloatingEvent, FloatingView};

use super::floating_twin::CoreFloating;
use super::group_roc::{forest_view, layout_key, rift_view, window, window_id, windows};
use crate::actor::app::WindowId;
use crate::layout_engine::LayoutId;
use crate::model::tree::NodeId;

static RESEEDS: AtomicU64 = AtomicU64::new(0);

/// How often a `roc-core` release build compares the whole systems after an operation.
const VIEW_EVERY: u64 = 32;

/// Whether every operation compares the whole systems: in tests and debug builds.
const VIEW_ALWAYS: bool = cfg!(any(test, debug_assertions));

/// How many times the Roc floating system differed from the core's, process-wide (each one a
/// `roc-divergence:`, the Roc system then re-seeded from the core's).
pub(crate) fn floating_roc_reseeds() -> u64 { RESEEDS.load(Ordering::Relaxed) }

fn report(what: &str, difference: &str) {
    let reseeds = RESEEDS.fetch_add(1, Ordering::Relaxed) + 1;
    #[cfg(test)]
    panic!("roc-divergence ({reseeds} so far): the floating system after {what}: {difference}");
    #[cfg(not(test))]
    tracing::warn!(
        what,
        reseeds,
        difference,
        "roc-divergence: the Roc floating system differs from the core's; re-seeded from it"
    );
}

/// The core's floating system as the Roc system's view gives it.
pub(super) fn view_of(core: &CoreFloating) -> FloatingView {
    FloatingView {
        forest: forest_view(core.forest()),
        geometry: core
            .geometry()
            .iter()
            .map(|(&held, &geometry)| (window(held), geometry))
            .collect(),
    }
}

/// The Roc floating system beside the core's. Queries step it too (`&self` in the layout
/// system's interface), so it sits in a cell; nothing else reaches it.
pub(super) struct FloatingRoc {
    machine: RefCell<RocFloating>,
    steps: u64,
}

impl Debug for FloatingRoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FloatingRoc")
    }
}

impl FloatingRoc {
    /// The Roc system holding what `core` holds.
    pub(super) fn seeded(core: &CoreFloating) -> Self {
        Self {
            machine: RefCell::new(RocFloating::seeded(&view_of(core))),
            steps: 0,
        }
    }

    fn reseed(&mut self, core: &CoreFloating) {
        *self.machine.get_mut() = RocFloating::seeded(&view_of(core));
    }

    /// An operation the core took (answering `rust`): the Roc system takes it too; its answer,
    /// and the whole system when due, held to the core's.
    pub(super) fn stepped(
        &mut self,
        core: &CoreFloating,
        what: &str,
        event: FloatingEvent,
        rust: FloatingAnswer,
    ) {
        self.steps += 1;
        let answer = self.machine.get_mut().step(event);
        if answer != rust {
            report(what, &format!("answers: core {rust:?}, roc {answer:?}"));
            self.reseed(core);
        } else if VIEW_ALWAYS || self.steps.is_multiple_of(VIEW_EVERY) {
            let (roc, held) = (self.machine.get_mut().view(), view_of(core));
            if roc != held {
                report(what, &format!("views: core {held:?}, roc {roc:?}"));
                self.reseed(core);
            }
        }
    }

    /// A query the core answered (`rust`): the Roc system's answer held to it.
    pub(super) fn asked(&self, what: &str, event: FloatingEvent, rust: FloatingAnswer) {
        let answer = self.machine.borrow_mut().step(event);
        if answer != rust {
            report(what, &format!("answers: core {rust:?}, roc {answer:?}"));
        }
    }

    /// The shape the core built (`rust`), the Roc system's held to it.
    pub(super) fn shape(
        &self,
        layout: LayoutId,
        floating: Floating,
        gaps: Gaps,
        known: &[(u64, WindowConstraints)],
        rust: &Layout<WindowId>,
    ) {
        let roc = self.machine.borrow_mut().shape(layout_key(layout), floating, gaps, known);
        let held = rust.clone().map(&|held: WindowId| rift_core::plain::WindowId {
            pid: held.pid,
            idx: held.idx.get(),
        });
        if roc != held {
            report("shape", &format!("core {held:?}, roc {roc:?}"));
        }
    }

    /// The group bars the core gave (`rust`), the Roc system's held to them.
    pub(super) fn group_bars(
        &self,
        layout: LayoutId,
        floating: Floating,
        selection_path_only: bool,
        rust: &[rift_core::layout::group_forest::GroupBar<NodeId, WindowId>],
    ) {
        let roc: Vec<_> = self
            .machine
            .borrow_mut()
            .group_bars(layout_key(layout), floating, selection_path_only)
            .into_iter()
            .map(|bar| rift_core::layout::group_forest::GroupBar {
                stack: super::group_roc::node_id(bar.stack),
                orientation: bar.orientation,
                frame: bar.frame,
                windows: bar.windows.into_iter().map(window_id).collect(),
                selected: bar.selected,
            })
            .collect();
        if roc != rust {
            report("group_bars", &format!("core {rust:?}, roc {roc:?}"));
        }
    }

    /// rift-cli's container view the core gave, the Roc system's held to it.
    pub(super) fn container_view(
        &self,
        layout: LayoutId,
        rust: &rift_core::layout::group_forest::ContainerView<NodeId, WindowId>,
    ) {
        let roc = rift_view(self.machine.borrow_mut().container_view(layout_key(layout)));
        if &roc != rust {
            report("container_view", &format!("core {rust:?}, roc {roc:?}"));
        }
    }

    /// rift-cli's drawing rows the core gave, the Roc system's held to them.
    pub(super) fn rows(
        &self,
        layout: LayoutId,
        rust: &[rift_core::layout::group_forest::Row<NodeId, WindowId>],
    ) {
        let roc: Vec<_> = self
            .machine
            .borrow_mut()
            .rows(layout_key(layout))
            .into_iter()
            .map(|row| rift_core::layout::group_forest::Row {
                branches: row.branches,
                mark: row.mark,
                node: super::group_roc::node_id(row.node),
                held: match row.held {
                    rift_core::layout::group_forest::Drawn::Window(word) => {
                        rift_core::layout::group_forest::Drawn::Window(window_id(word))
                    }
                    rift_core::layout::group_forest::Drawn::Container { kind, total } => {
                        rift_core::layout::group_forest::Drawn::Container { kind, total }
                    }
                },
                size: row.size,
                role: row.role,
            })
            .collect();
        if roc != rust {
            report("rows", &format!("core {rust:?}, roc {roc:?}"));
        }
    }
}

impl Clone for FloatingRoc {
    fn clone(&self) -> Self {
        let held = self.machine.borrow_mut().view();
        Self {
            machine: RefCell::new(RocFloating::seeded(&held)),
            steps: self.steps,
        }
    }
}

/// A tree answer.
pub(super) fn tree(answer: Answer) -> FloatingAnswer { FloatingAnswer::Tree(answer) }

pub(super) fn plain_focus(focus: Focus<WindowId>) -> Focus<u64> { super::group_roc::focus(focus) }

pub(super) fn plain_query(query: Query<WindowId>) -> Query<u64> { super::group_roc::query(query) }

pub(super) fn plain_windows(held: &[WindowId]) -> Vec<u64> { windows(held) }
