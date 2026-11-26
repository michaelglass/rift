//! Under `roc-core`, the Roc floating system (`RocFloating`) decides (layout trees, floating's
//! step 4, as the group forest's slice 4): it takes every operation and query, and its answers
//! are rift's, as are a layout's shape, its group bars and rift-cli's views. The core's floating
//! system, the Roc system's shadow, takes every operation too; its answers are held to the Roc
//! system's on every call, and the whole system (the forest's view and every window's kept frame)
//! on every operation in tests and debug builds and every [`VIEW_EVERY`]th in a `roc-core`
//! release build. A difference is counted ([`floating_roc_reseeds`]), fails a test, or in a
//! running Rift warns `roc-divergence:`; an operation's re-seeds the core's system from the Roc
//! system's view (a query's or a view's is only reported: the next operation compares the
//! systems). Upstream's system stays the core's own shadow until it is deleted.

use std::cell::RefCell;
use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};

use rift_core::layout::floating::{Floating, Layout};
use rift_core::layout::group_forest::{Focus, Query};
use rift_core::layout::{Gaps, WindowConstraints};
use rift_core_roc::RocFloating;
use rift_core_roc::group_forest::{Answer, FloatingAnswer, FloatingEvent, FloatingView};

use super::floating_twin::CoreFloating;
use super::group_roc::{forest_view, layout_key, node_id, rift_view, window, window_id, windows};
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

use super::group_roc::{ON_DIVERGENCE, OnDivergence};

fn report(what: &str, difference: &str) { report_under(ON_DIVERGENCE, what, difference) }

/// Counts a difference, then fails or warns as `policy` says.
fn report_under(policy: OnDivergence, what: &str, difference: &str) {
    let reseeds = RESEEDS.fetch_add(1, Ordering::Relaxed) + 1;
    if policy == OnDivergence::Fail {
        panic!("roc-divergence ({reseeds} so far): the floating system after {what}: {difference}");
    }
    tracing::warn!(
        what,
        reseeds,
        difference,
        "roc-divergence: the core's floating system differs from the deciding Roc system's; \
         re-seeded from it"
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

/// The core's floating system holding what a Roc system's view holds (a re-seed).
fn core_of(view: &FloatingView) -> CoreFloating {
    CoreFloating::from_parts(
        view.forest
            .forest_on(node_id, window_id)
            .expect("the Roc system's tables are slot tables"),
        view.geometry
            .iter()
            .map(|&(word, geometry)| (window_id(word), geometry))
            .collect(),
    )
}

/// The deciding Roc floating system. Queries step it too (`&self` in the layout
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

    /// An operation: the Roc system takes `event` and decides, and its answer is rift's. The
    /// core's took it too (answering `rust`); a different answer, or a different whole system
    /// when one is compared, re-seeds the core's from the Roc system's view (the second value:
    /// whether it did).
    pub(super) fn decided(
        &mut self,
        core: &mut CoreFloating,
        what: &str,
        event: FloatingEvent,
        rust: FloatingAnswer,
    ) -> (FloatingAnswer, bool) {
        self.decided_under(ON_DIVERGENCE, core, what, event, rust)
    }

    /// [`Self::decided`], a divergence failing or warned as `policy` says (a test of the re-seed
    /// asks for a warning, so the re-seed is reached).
    pub(super) fn decided_under(
        &mut self,
        policy: OnDivergence,
        core: &mut CoreFloating,
        what: &str,
        event: FloatingEvent,
        rust: FloatingAnswer,
    ) -> (FloatingAnswer, bool) {
        self.steps += 1;
        let machine = self.machine.get_mut();
        let answer = machine.step(event);
        let reseeded = if answer != rust {
            report_under(policy, what, &format!("answers: core {rust:?}, roc {answer:?}"));
            true
        } else if VIEW_ALWAYS || self.steps.is_multiple_of(VIEW_EVERY) {
            let (roc, held) = (machine.view(), view_of(core));
            let differs = roc != held;
            if differs {
                report_under(policy, what, &format!("views: core {held:?}, roc {roc:?}"));
            }
            differs
        } else {
            false
        };
        if reseeded {
            *core = core_of(&machine.view());
        }
        (answer, reseeded)
    }

    /// A query: the Roc system answers, and its answer is rift's; the core's (`rust`) is held to
    /// it.
    pub(super) fn asked(
        &self,
        what: &str,
        event: FloatingEvent,
        rust: FloatingAnswer,
    ) -> FloatingAnswer {
        let answer = self.machine.borrow_mut().step(event);
        if answer != rust {
            report(what, &format!("answers: core {rust:?}, roc {answer:?}"));
        }
        answer
    }

    /// The shape the core built (`rust`), the Roc system's held to it.
    pub(super) fn shape(
        &self,
        layout: LayoutId,
        floating: Floating,
        gaps: Gaps,
        known: &[(u64, WindowConstraints)],
        rust: &Layout<WindowId>,
    ) -> Layout<WindowId> {
        let roc = self.machine.borrow_mut().shape(layout_key(layout), floating, gaps, known);
        let held = rust.clone().map(&|held: WindowId| rift_core::plain::WindowId {
            pid: held.pid,
            idx: held.idx.get(),
        });
        if roc != held {
            report("shape", &format!("core {held:?}, roc {roc:?}"));
        }
        roc.map(&|plain: rift_core::plain::WindowId| {
            window_id((u64::from(plain.pid as u32) << 32) | u64::from(plain.idx))
        })
    }

    /// The group bars the core gave (`rust`), the Roc system's held to them.
    pub(super) fn group_bars(
        &self,
        layout: LayoutId,
        floating: Floating,
        selection_path_only: bool,
        rust: &[rift_core::layout::group_forest::GroupBar<NodeId, WindowId>],
    ) -> Vec<rift_core::layout::group_forest::GroupBar<NodeId, WindowId>> {
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
        roc
    }

    /// rift-cli's container view the core gave, the Roc system's held to it.
    pub(super) fn container_view(
        &self,
        layout: LayoutId,
        rust: &rift_core::layout::group_forest::ContainerView<NodeId, WindowId>,
    ) -> rift_core::layout::group_forest::ContainerView<NodeId, WindowId> {
        let roc = rift_view(self.machine.borrow_mut().container_view(layout_key(layout)));
        if &roc != rust {
            report("container_view", &format!("core {rust:?}, roc {roc:?}"));
        }
        roc
    }

    /// rift-cli's drawing rows the core gave, the Roc system's held to them.
    pub(super) fn rows(
        &self,
        layout: LayoutId,
        rust: &[rift_core::layout::group_forest::Row<NodeId, WindowId>],
    ) -> Vec<rift_core::layout::group_forest::Row<NodeId, WindowId>> {
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
        roc
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

/// The tree answer a floating answer holds (a frame answer has none).
pub(super) fn as_tree(answer: FloatingAnswer) -> Option<Answer> {
    match answer {
        FloatingAnswer::Tree(answer) => Some(answer),
        FloatingAnswer::Frame(_) => None,
    }
}

/// The kept frame a frame answer holds.
pub(super) fn as_frame(answer: FloatingAnswer) -> Option<Option<rift_core::plain::Rect>> {
    match answer {
        FloatingAnswer::Frame(frame) => Some(frame),
        FloatingAnswer::Tree(_) => None,
    }
}

/// The Roc system's answer as `parse` reads it, or the core's (`rust`) when it reads none (a
/// divergence, already reported).
pub(super) fn decided<A, T>(answer: A, parse: impl FnOnce(A) -> Option<T>, rust: T) -> T {
    parse(answer).unwrap_or(rust)
}

#[cfg(test)]
mod tests {
    use rift_core::layout::group_forest::{Geometry, InsertionPoint};
    use rift_core::plain::Rect;

    use super::*;

    /// A core that differs from the deciding Roc system (a frame the Roc system does not hold)
    /// is re-seeded from it on the next operation: answer and view are the Roc system's.
    #[test]
    fn a_core_that_differs_is_re_seeded_from_the_roc_system() {
        let mut core = CoreFloating::default();
        let mut roc = FloatingRoc::seeded(&core);
        let layout = core.create_layout();
        roc.decided(
            &mut core,
            "create_layout",
            FloatingEvent::CreateLayout,
            tree(Answer::Layout(layout_key(layout))),
        );
        // A window with a frame both hold.
        let held = WindowId::new(2, 2);
        let frame = Rect {
            x: 10.0,
            y: 20.0,
            w: 30.0,
            h: 40.0,
        };
        core.add_window(layout, held, InsertionPoint::EndOfTree);
        roc.decided(
            &mut core,
            "update",
            FloatingEvent::AddWindow(
                layout_key(layout),
                super::window(held),
                InsertionPoint::EndOfTree,
            ),
            tree(Answer::Done),
        );
        core.store_frame(held, frame);
        roc.decided(
            &mut core,
            "store_frame",
            FloatingEvent::StoreFrame(super::window(held), frame),
            tree(Answer::Done),
        );
        let mut geometry = core.geometry().clone();
        let stray = WindowId::new(4, 4);
        geometry.insert(stray, Geometry::Kept(Rect { x: 1.0, y: 2.0, w: 3.0, h: 4.0 }));
        core = CoreFloating::from_parts(core.forest().clone(), geometry);
        let window = WindowId::new(1, 1);
        core.add_window(layout, window, InsertionPoint::EndOfTree);
        let (answer, reseeded) = roc.decided_under(
            OnDivergence::Warn,
            &mut core,
            "update",
            FloatingEvent::AddWindow(
                layout_key(layout),
                super::window(window),
                InsertionPoint::EndOfTree,
            ),
            tree(Answer::Done),
        );
        assert_eq!(answer, tree(Answer::Done));
        assert!(reseeded);
        assert_eq!(core.frame(stray), None, "the stray frame is gone");
        assert_eq!(core.frame(held), Some(frame), "the frames held stay");
        assert_eq!(view_of(&core), roc.machine.get_mut().view());
    }
}
