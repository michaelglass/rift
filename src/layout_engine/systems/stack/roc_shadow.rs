//! Under `roc-core`, the Roc stacks ([`RocStackTree`]) are a second shadow of the core's (layout
//! trees, stack slice 3): seeded from the core at load, stepped with every operation (queries
//! too, through a cell: the layout system's queries take `&self`), their answer and whole view
//! compared with the core's on every operation. A difference is counted
//! ([`stack_tree_reseeds`]), fails a test, or in a running Rift warns `roc-divergence:` and
//! re-seeds the Roc stacks from the core (the deciding side stands; a query's difference is only
//! reported, and the next operation compares the views).
//!
//! The Roc stacks hold the core's ids as words: layouts by slotmap's ffi form, stack and member
//! ids as their words, windows as [`Word`] makes them, which order as `WindowId` does (pid, then
//! index), so the windows left out and an app's windows agree in order.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

use rift_core::layout::stack_tree::{
    self as core, Edit, Edited, Focus, Focused, GroupContainer, Member, Members, Stack,
};
use rift_core_roc::RocStackTree;
use rift_core_roc::stack_tree::{Answer, Event, LayoutKey, StacksView};

use super::twinned::CoreStacks;
use crate::actor::app::WindowId;
use crate::layout_engine::LayoutId;
use crate::roc_path::Word;

static RESEEDS: AtomicU64 = AtomicU64::new(0);

/// How many times the Roc stacks differed from the core's, process-wide (each one a
/// `roc-divergence:`; an operation's re-seeds the Roc stacks from the core).
pub(crate) fn stack_tree_reseeds() -> u64 { RESEEDS.load(Ordering::Relaxed) }

/// The Roc stacks, the core's second shadow. Queries step them too, so they sit in a cell;
/// nothing else reaches them.
pub(super) struct RocShadow {
    stacks: RefCell<RocStackTree>,
}

impl RocShadow {
    /// The Roc stacks holding what `core` holds (a load, or a re-seed).
    pub(super) fn seeded(core: &CoreStacks) -> Self {
        Self {
            stacks: RefCell::new(RocStackTree::seeded(&view(core))),
        }
    }

    /// An operation the core decided (answering `rust`): the Roc stacks take it too; a
    /// different answer or view re-seeds them from the core.
    pub(super) fn stepped(&mut self, core: &CoreStacks, what: &str, event: Event, rust: Answer) {
        let stacks = self.stacks.get_mut();
        let answer = stacks.step(event);
        let roc = stacks.view();
        let difference = if answer != rust {
            Some(format!("answers: rust {rust:?}, roc {answer:?}"))
        } else {
            let held = view(core);
            (roc != held).then(|| format!("views: rust {held:?}, roc {roc:?}"))
        };
        if let Some(difference) = difference {
            report(what, &difference);
            *self = Self::seeded(core);
        }
    }

    /// A query the core answered (`rust`): the Roc stacks' answer is held to it.
    pub(super) fn asked(&self, what: &str, event: Event, rust: Answer) {
        let answer = self.stacks.borrow_mut().step(event);
        if answer != rust {
            report(what, &format!("answers: rust {rust:?}, roc {answer:?}"));
        }
    }
}

/// Counts a difference, then fails the test, or in a running Rift warns it.
fn report(what: &str, difference: &str) {
    let reseeds = RESEEDS.fetch_add(1, Ordering::Relaxed) + 1;
    #[cfg(test)]
    panic!("roc-divergence ({reseeds} so far): the stacks after {what}: {difference}");
    #[cfg(not(test))]
    tracing::warn!(
        what,
        reseeds,
        difference,
        "roc-divergence: the Roc stacks differ from the core's; re-seeded from the core"
    );
}

impl Clone for RocShadow {
    /// Second Roc stacks holding what these hold.
    fn clone(&self) -> Self {
        let held = self.stacks.borrow_mut().view();
        Self {
            stacks: RefCell::new(RocStackTree::seeded(&held)),
        }
    }
}

impl std::fmt::Debug for RocShadow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("RocShadow") }
}

pub(super) fn layout_key(layout: LayoutId) -> LayoutKey {
    LayoutKey(slotmap::Key::data(&layout).as_ffi())
}

pub(super) fn window(window: WindowId) -> u64 { window.word() }

pub(super) fn windows(windows: &[WindowId]) -> Vec<u64> {
    windows.iter().copied().map(window).collect()
}

/// The windows the core holds (in its stacks or left out) that `of_app` tells, in the core's
/// order: an app closing, or a has-windows-for-app question, as Roc has no pid.
pub(super) fn app_windows(core: &CoreStacks, of_app: impl Fn(WindowId) -> bool) -> Vec<u64> {
    let mut held: Vec<WindowId> = core
        .layouts()
        .values()
        .flat_map(|stack| stack.list().iter().map(|member| member.window))
        .chain(core.left_out().iter().copied())
        .filter(|&held| of_app(held))
        .collect();
    held.sort();
    held.dedup();
    windows(&held)
}

pub(super) fn focus(focus: Focus<WindowId>) -> Focus<u64> {
    match focus {
        Focus::Select(held) => Focus::Select(window(held)),
        Focus::Ascend => Focus::Ascend,
        Focus::Descend => Focus::Descend,
        Focus::Direction(direction) => Focus::Direction(direction),
    }
}

pub(super) fn focused(focused: &Focused<WindowId>) -> Focused<u64> {
    match focused {
        Focused::Selection { handled } => Focused::Selection { handled: *handled },
        Focused::Movement { window: held, raise } => Focused::Movement {
            window: held.map(window),
            raise: windows(raise),
        },
    }
}

pub(super) fn edit(edit: Edit<WindowId>) -> Edit<u64> {
    match edit {
        Edit::Move { direction, beyond } => Edit::Move { direction, beyond },
        Edit::Fullscreen { within_gaps } => Edit::Fullscreen { within_gaps },
        Edit::Swap(a, b) => Edit::Swap(window(a), window(b)),
        Edit::ReorderTab { window: held, target } => Edit::ReorderTab {
            window: window(held),
            target: window(target),
        },
        Edit::Drop { source, target } => Edit::Drop {
            source: window(source),
            target: window(target),
        },
        Edit::Restack => Edit::Restack,
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
        core::Query::Stack(held) => core::Query::Stack(window(held)),
    }
}

pub(super) fn group(group: &Option<GroupContainer<WindowId>>) -> Option<GroupContainer<u64>> {
    group.as_ref().map(|held| GroupContainer {
        stack: held.stack,
        orientation: held.orientation,
        windows: windows(&held.windows),
        selected: held.selected,
    })
}

fn member(member: &Member<WindowId>) -> Member<u64> {
    Member {
        id: member.id,
        window: window(member.window),
        weight: member.weight,
        fill: member.fill,
    }
}

fn plain_stack(stack: &Stack<WindowId>) -> Stack<u64> {
    Stack {
        id: stack.id,
        members: match &stack.members {
            Members::Empty => Members::Empty,
            Members::Populated { members, selection } => Members::Populated {
                members: members.map(member),
                selection: *selection,
            },
        },
        orientation: stack.orientation,
        fill: stack.fill,
    }
}

/// The core's stacks as the Roc stacks' view gives them.
fn view(core: &CoreStacks) -> StacksView {
    StacksView {
        layouts: core
            .layouts()
            .saved()
            .map(|(held, version)| (held.map(|(_, stack)| plain_stack(stack)), version))
            .collect(),
        layout_free: core.layouts().free().to_vec(),
        next_node: core.next_node(),
        orientation: core.orientation(),
        insertion: core.insertion(),
        left_out: windows(&core.left_out().iter().copied().collect::<Vec<_>>()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout_engine::systems::{LayoutSystem, StackLayoutSystem, WindowChange};

    fn w(idx: u32) -> WindowId { WindowId::new(1, idx) }

    /// Roc stacks that differ from the core's are counted and fail a test (a running Rift warns
    /// and re-seeds them from the core).
    #[test]
    fn a_roc_stack_divergence_is_counted_and_fails_tests() {
        let mut system = StackLayoutSystem::default();
        let layout = system.create_layout();
        let mut roc = RocShadow::seeded(system.core_for_tests());
        roc.stacks.get_mut().step(Event::CreateLayout);
        let before = stack_tree_reseeds();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            system.update(WindowChange::Add(layout, w(1), None));
            roc.stepped(
                system.core_for_tests(),
                "update",
                Event::AddWindow(layout_key(layout), window(w(1))),
                Answer::Done,
            );
        }));
        assert!(failed.is_err());
        assert!(stack_tree_reseeds() > before);
    }

    /// The Roc shadow's cost per operation at 10, 100 and 1000 windows (release, roc-core:
    /// `cargo test --release --lib --features roc-core measure_stack_roc_shadow -- --ignored
    /// --nocapture`): the Roc step, the view comparison, a re-seed.
    #[test]
    #[ignore]
    fn measure_stack_roc_shadow() {
        use std::hint::black_box;
        use std::time::Instant;

        use rift_core::layout::stack_tree::Direction;
        for count in [10_u32, 100, 1000] {
            let mut system = StackLayoutSystem::default();
            let layout = system.create_layout();
            for idx in 1..=count {
                system.update(WindowChange::Add(layout, w(idx), None));
            }
            let core = system.core_for_tests().clone();
            let mut roc = RocStackTree::seeded(&view(&core));
            let ops = 2000_u32;
            let key = layout_key(layout);
            let started = Instant::now();
            for i in 0..ops {
                let event = match i % 3 {
                    0 => Event::Focus(
                        key,
                        Focus::Direction([Direction::Left, Direction::Right][(i / 3 % 2) as usize]),
                    ),
                    1 => Event::Edit(
                        key,
                        Edit::Swap(window(w(1 + i % count)), window(w(1 + (i * 7) % count))),
                    ),
                    _ => Event::SelectedWindow(key),
                };
                black_box(roc.step(event));
            }
            let step_each = started.elapsed() / ops;
            let rounds = 200;
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(roc.view() == view(&core));
            }
            let view_each = started.elapsed() / rounds;
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(RocShadow::seeded(&core));
            }
            let seed_each = started.elapsed() / rounds;
            eprintln!(
                "{count} windows: roc step {step_each:?}/op, view comparison {view_each:?}, re-seed {seed_each:?}"
            );
        }
    }
}
