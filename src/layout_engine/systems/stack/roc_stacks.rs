//! Under `roc-core`, the Roc stacks ([`RocStackTree`]) decide (layout trees, stack slice 4):
//! they take every operation, queries included, and their answers are rift's, as are a layout's
//! frames (their own shape through the Roc traditional geometry). The Rust core's stacks, the Roc
//! stacks' shadow, take every operation too; their answer and whole view are compared with the
//! Roc stacks' on every operation. A difference is counted ([`stack_tree_reseeds`]), fails a
//! test, or in a running Rift warns `roc-divergence:` and re-seeds the Rust stacks from the Roc
//! stacks' view (a query's difference is only reported: the Rust stacks are not changed by one,
//! and the next operation compares the views).
//!
//! The Roc stacks hold the core's ids as words: layouts by slotmap's ffi form, stack and member
//! ids as their words, windows as [`Word`] makes them, which order as `WindowId` does (pid, then
//! index), so the windows left out and an app's windows agree in order.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

use rift_core::layout::WindowConstraints;
use rift_core::layout::stack_tree::{
    self as core, Edit, Edited, Focus, Focused, GroupContainer, Member, Members, Stack,
};
use rift_core::plain::Rect;
use rift_core::slot_table::SlotTable;
use rift_core_roc::RocStackTree;
use rift_core_roc::stack_tree::{Answer, Event, LayoutKey, Pass, StacksView};

use super::system::CoreStacks;
use crate::actor::app::WindowId;
use crate::layout_engine::LayoutId;
use crate::roc_path::{Word, window_of_word};

static RESEEDS: AtomicU64 = AtomicU64::new(0);

/// How many times the Rust core's stacks differed from the deciding Roc stacks, process-wide
/// (each one a `roc-divergence:`; an operation's re-seeds the Rust stacks from the Roc stacks).
pub(crate) fn stack_tree_reseeds() -> u64 { RESEEDS.load(Ordering::Relaxed) }

/// The deciding Roc stacks. Queries step them too (`&self` in the layout system's interface), so
/// they sit in a cell; nothing else reaches them.
pub(super) struct RocStacks {
    stacks: RefCell<RocStackTree>,
}

impl RocStacks {
    /// The Roc stacks holding what `core` holds (a load).
    pub(super) fn seeded(core: &CoreStacks) -> Self {
        Self {
            stacks: RefCell::new(RocStackTree::seeded(&view(core))),
        }
    }

    /// An operation: the Roc stacks take `event` and decide, and their answer is rift's. The
    /// Rust stacks took it too (answering `rust`); a different answer or view re-seeds the Rust
    /// stacks from the Roc stacks' view.
    pub(super) fn decided(
        &mut self,
        core: &mut CoreStacks,
        what: &str,
        event: Event,
        rust: Answer,
    ) -> Answer {
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
            *core = core_of(&roc);
        }
        answer
    }

    /// A query: the Roc stacks answer, and their answer is rift's; the Rust stacks' (`rust`) is
    /// held to it.
    pub(super) fn asked(&self, what: &str, event: Event, rust: Answer) -> Answer {
        let answer = self.stacks.borrow_mut().step(event);
        if answer != rust {
            report(what, &format!("answers: rust {rust:?}, roc {answer:?}"));
        }
        answer
    }

    /// The whole system the Roc stacks hold.
    #[cfg(test)]
    pub(super) fn view(&self) -> StacksView { self.stacks.borrow_mut().view() }

    /// A layout's frames: the Roc stacks' own shape through the Roc traditional geometry, with
    /// the constraints rift knows.
    pub(super) fn frames(
        &self,
        layout: LayoutId,
        pass: Pass,
        known: &[(u64, WindowConstraints)],
    ) -> Vec<(rift_core::plain::WindowId, Rect)> {
        self.stacks.borrow_mut().frames(layout_key(layout), pass, known)
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
        "roc-divergence: the Rust stacks differ from the deciding Roc stacks; re-seeded from them"
    );
}

impl Clone for RocStacks {
    /// Second Roc stacks holding what these hold.
    fn clone(&self) -> Self {
        let held = self.stacks.borrow_mut().view();
        Self {
            stacks: RefCell::new(RocStackTree::seeded(&held)),
        }
    }
}

impl std::fmt::Debug for RocStacks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("RocStacks") }
}

/// What the Roc stacks answered, as rift takes it (`parse`); an answer of another kind than the
/// operation's was a difference already reported (the Rust stacks answered the operation's
/// kind), and rift takes the Rust stacks' answer.
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

pub(super) fn as_slot(answer: Answer) -> Option<Option<usize>> {
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

pub(super) fn as_group(answer: Answer) -> Option<Option<GroupContainer<WindowId>>> {
    match answer {
        Answer::Group(group) => Some(group.map(|held| GroupContainer {
            stack: held.stack,
            orientation: held.orientation,
            windows: window_ids(&held.windows),
            selected: held.selected,
        })),
        _ => None,
    }
}

pub(super) fn as_focused(answer: Answer) -> Option<Focused<WindowId>> {
    match answer {
        Answer::Focused(focused) => Some(match focused {
            Focused::Selection { handled } => Focused::Selection { handled },
            Focused::Movement { window, raise } => Focused::Movement {
                window: window.map(window_id),
                raise: window_ids(&raise),
            },
        }),
        _ => None,
    }
}

pub(super) fn as_edited(answer: Answer) -> Option<Edited<WindowId>> {
    match answer {
        Answer::Edited(edited) => Some(match edited {
            Edited::Unsupported => Edited::Unsupported,
            Edited::Unchanged => Edited::Unchanged,
            Edited::Changed => Edited::Changed,
            Edited::Raise(raised) => Edited::Raise(window_ids(&raised)),
        }),
        _ => None,
    }
}

fn layout_id(key: LayoutKey) -> LayoutId { slotmap::KeyData::from_ffi(key.0).into() }

/// The window a Roc word names: the Roc stacks hold only words rift gave them.
fn window_id(word: u64) -> WindowId {
    window_of_word(word).expect("the Roc stacks hold the windows rift gave them")
}

fn window_ids(words: &[u64]) -> Vec<WindowId> {
    words.iter().map(|&word| window_id(word)).collect()
}

/// The Rust stacks holding what a Roc stacks' view holds (a re-seed): the table's slots,
/// versions and free list, the counter, the settings, the windows left out.
pub(super) fn core_of(view: &StacksView) -> CoreStacks {
    let taking: Vec<u32> = view.layout_free.iter().rev().copied().collect();
    let layouts = SlotTable::parse(
        view.layouts
            .iter()
            .map(|(held, version)| (held.as_ref().map(rift_stack), *version))
            .collect(),
    )
    .and_then(|table| table.resumed(&taking))
    .expect("the Roc stacks' layouts are a slot table");
    CoreStacks::from_parts(
        layouts,
        view.next_node,
        view.orientation,
        view.left_out.iter().map(|&word| window_id(word)).collect(),
        view.insertion,
    )
}

fn rift_stack(stack: &Stack<u64>) -> Stack<WindowId> {
    let member = |member: &Member<u64>| Member {
        id: member.id,
        window: window_id(member.window),
        weight: member.weight,
        fill: member.fill,
    };
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

    /// Rust stacks that differ from the deciding Roc stacks are counted and fail a test (a
    /// running Rift warns and re-seeds them from the Roc stacks).
    #[test]
    fn stacks_that_differ_are_counted_and_fail_tests() {
        let mut system = StackLayoutSystem::default();
        let layout = system.create_layout();
        let mut roc = RocStacks::seeded(system.core_for_tests());
        roc.stacks.get_mut().step(Event::CreateLayout);
        let mut core = system.core_for_tests().clone();
        let before = stack_tree_reseeds();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            system.update(WindowChange::Add(layout, w(1), None));
            roc.decided(
                &mut core,
                "update",
                Event::AddWindow(layout_key(layout), window(w(1))),
                Answer::Done,
            );
        }));
        assert!(failed.is_err());
        assert!(stack_tree_reseeds() > before);
    }

    /// The Rust stacks re-seeded from the Roc stacks' view hold what they held, and allocate
    /// what they would have.
    #[test]
    fn the_rust_stacks_reseeded_from_the_roc_view_hold_and_allocate_alike() {
        let mut system = StackLayoutSystem::default();
        let layout = system.create_layout();
        let removed = system.create_layout();
        for idx in 1..=4 {
            system.update(WindowChange::Add(layout, w(idx), None));
        }
        system.set_window_participation(w(9), false);
        system.remove_layout(removed);
        let mut core = system.core_for_tests().clone();
        let mut reseeded = core_of(&RocStacks::seeded(&core).view());
        assert_eq!(view(&reseeded), view(&core));
        assert_eq!(reseeded.create_layout(), core.create_layout());
        assert_eq!(view(&reseeded), view(&core));
    }

    /// The Roc stacks' cost at 10, 100 and 1000 windows (release, roc-core: `cargo test
    /// --release --lib --features roc-core measure_stack_roc_stacks -- --ignored --nocapture`):
    /// the Roc step, the view comparison, a layout pass (the Roc stacks' against the Rust
    /// stacks' shape and geometry), a re-seed of the Rust stacks from the Roc view.
    #[test]
    #[ignore]
    fn measure_stack_roc_stacks() {
        use std::hint::black_box;
        use std::time::Instant;

        use rift_core::layout::Gaps;
        use rift_core::layout::stack_tree::Direction;
        use rift_core::layout::traditional::StackLine;
        for count in [10_u32, 100, 1000] {
            let mut system = StackLayoutSystem::default();
            let layout = system.create_layout();
            for idx in 1..=count {
                system.update(WindowChange::Add(layout, w(idx), None));
            }
            let core = system.core_for_tests().clone();
            let mut roc = RocStacks::seeded(&core);
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
                black_box(roc.stacks.get_mut().step(event));
            }
            let step_each = started.elapsed() / ops;
            let rounds = 200;
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(roc.view() == view(&core));
            }
            let view_each = started.elapsed() / rounds;
            let pass = Pass {
                screen: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1440.0,
                    h: 900.0,
                },
                gaps: Gaps {
                    top: 8.0,
                    left: 8.0,
                    bottom: 8.0,
                    right: 8.0,
                    horizontal: 8.0,
                    vertical: 8.0,
                },
                stack_offset: 40.0,
                stack_line: StackLine {
                    thickness: 4.0,
                    spacing: 2.0,
                    top: true,
                    left: true,
                },
            };
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(roc.frames(layout, pass, &[]));
            }
            let roc_pass = started.elapsed() / rounds;
            let started = Instant::now();
            for _ in 0..rounds {
                let shape = core.shape(
                    layout,
                    pass.screen,
                    pass.gaps,
                    pass.stack_offset,
                    pass.stack_line,
                    |_| None,
                );
                black_box(shape.map(|shape| shape.frames()));
            }
            let rust_pass = started.elapsed() / rounds;
            let held = roc.view();
            let started = Instant::now();
            for _ in 0..rounds {
                black_box(core_of(&held));
            }
            let reseed_each = started.elapsed() / rounds;
            eprintln!(
                "{count} windows: roc step {step_each:?}/op, view comparison {view_each:?}, roc pass {roc_pass:?}, rust shape and geometry {rust_pass:?}, rust re-seed from the roc view {reseed_each:?}"
            );
        }
    }
}
