//! The Roc core on rift's path for the pure decisions that take no model (the `roc-core`
//! feature): each call site asks the Roc core the question it just answered in Rust, on plain
//! data, and the two answers must be the same words. A checked decision ([`check`]): equal, the
//! Rust answer stands, the Roc core's decision bit for bit; different (or no answer), tests
//! fail, and a running Rift logs `roc-divergence:` and goes on with the Rust answer. A decided
//! family ([`decided`]: focus) flips that: the Roc answer is rift's, the Rust answer its shadow,
//! and a running Rift counts each difference ([`Family`]) and goes on with the Roc answer.
//!
//! A question carries only the facts the Rust decision asked: a call site asks each fact
//! through an [`Asked`] (or, about a key, an [`AskedAbout`]), which remembers the answer under
//! `roc-core` and is only a call without it. A Roc decision reading any other fact diverges.

#[cfg(feature = "roc-core")]
use std::cell::{OnceCell, RefCell};
#[cfg(not(feature = "roc-core"))]
use std::marker::PhantomData;

#[cfg(feature = "roc-core")]
use rift_core::plain::{Answers, Fact};

/// A fact a decision may ask for: remembered when asked (under `roc-core`), so the Roc core
/// is asked the same question on what the Rust decision saw.
pub(crate) struct Asked<T> {
    #[cfg(feature = "roc-core")]
    answer: OnceCell<T>,
    #[cfg(not(feature = "roc-core"))]
    answer: PhantomData<fn() -> T>,
}

impl<T: Clone> Asked<T> {
    pub(crate) fn new() -> Self {
        Asked {
            #[cfg(feature = "roc-core")]
            answer: OnceCell::new(),
            #[cfg(not(feature = "roc-core"))]
            answer: PhantomData,
        }
    }

    /// Asks the fact. A second ask asks again and keeps the first answer.
    pub(crate) fn ask(&self, fact: impl FnOnce() -> T) -> T {
        let answer = fact();
        #[cfg(feature = "roc-core")]
        let _ = self.answer.set(answer.clone());
        answer
    }

    /// The fact on plain data, as asked or not.
    #[cfg(feature = "roc-core")]
    pub(crate) fn fact<U>(self, plain: impl FnOnce(T) -> U) -> Fact<U> {
        match self.answer.into_inner() {
            Some(answer) => Fact::Asked(plain(answer)),
            None => Fact::Unasked,
        }
    }
}

/// A fact a decision may ask about keys (a window's visibility): each key asked and its first
/// answer, remembered (under `roc-core`).
pub(crate) struct AskedAbout<K, V> {
    #[cfg(feature = "roc-core")]
    answers: RefCell<Vec<(K, V)>>,
    #[cfg(not(feature = "roc-core"))]
    answers: PhantomData<fn(K) -> V>,
}

impl<K: Copy + PartialEq, V: Clone> AskedAbout<K, V> {
    pub(crate) fn new() -> Self {
        AskedAbout {
            #[cfg(feature = "roc-core")]
            answers: RefCell::new(Vec::new()),
            #[cfg(not(feature = "roc-core"))]
            answers: PhantomData,
        }
    }

    /// Asks the fact about `key`. A second ask about a key asks again and keeps the first
    /// answer.
    pub(crate) fn ask(&self, key: K, fact: impl FnOnce(K) -> V) -> V {
        let answer = fact(key);
        #[cfg(feature = "roc-core")]
        {
            let mut answers = self.answers.borrow_mut();
            if !answers.iter().any(|(asked, _)| *asked == key) {
                answers.push((key, answer.clone()));
            }
        }
        answer
    }

    /// The answers on plain data.
    #[cfg(feature = "roc-core")]
    pub(crate) fn answers<PK, PV>(
        self,
        key: impl Fn(K) -> PK,
        answer: impl Fn(V) -> PV,
    ) -> Answers<PK, PV> {
        Answers(
            self.answers
                .into_inner()
                .into_iter()
                .map(|(asked, answered)| (key(asked), answer(answered)))
                .collect(),
        )
    }
}

/// Rift's ids as the plain questions' words, one each.
#[cfg(feature = "roc-core")]
pub(crate) trait Word {
    fn word(self) -> u64;
}

#[cfg(feature = "roc-core")]
impl Word for crate::actor::app::WindowId {
    fn word(self) -> u64 { (u64::from(self.pid as u32) << 32) | u64::from(self.idx.get()) }
}

/// The window a word names (the inverse of its [`Word`]).
#[cfg(feature = "roc-core")]
pub(crate) fn window_of_word(word: u64) -> Option<crate::actor::app::WindowId> {
    let idx = std::num::NonZeroU32::new(word as u32)?;
    Some(crate::actor::app::WindowId {
        pid: (word >> 32) as u32 as i32,
        idx,
    })
}

#[cfg(feature = "roc-core")]
impl Word for crate::sys::screen::SpaceId {
    fn word(self) -> u64 { self.get() }
}

#[cfg(feature = "roc-core")]
impl Word for crate::model::VirtualWorkspaceId {
    fn word(self) -> u64 { slotmap::Key::data(&self).as_ffi() }
}

#[cfg(feature = "roc-core")]
impl Word for crate::model::window_store::ClaimOwner {
    fn word(self) -> u64 { u64::from(self.0) }
}

#[cfg(feature = "roc-core")]
impl Word for crate::sys::window_server::WindowServerId {
    fn word(self) -> u64 { u64::from(self.0) }
}

/// An app's pid as a word, sign-extended.
#[cfg(feature = "roc-core")]
impl Word for crate::actor::app::pid_t {
    fn word(self) -> u64 { i64::from(self) as u64 }
}

/// A rift value from the word the Roc core answered with: the inverse of [`Word`] (none for a
/// word no value of the type has).
#[cfg(feature = "roc-core")]
pub(crate) trait FromWord: Sized {
    fn from_word(word: u64) -> Option<Self>;
}

#[cfg(feature = "roc-core")]
impl FromWord for crate::actor::app::WindowId {
    fn from_word(word: u64) -> Option<Self> { window_of_word(word) }
}

#[cfg(feature = "roc-core")]
impl FromWord for crate::sys::screen::SpaceId {
    fn from_word(word: u64) -> Option<Self> { Some(crate::sys::screen::SpaceId::new(word)) }
}

#[cfg(feature = "roc-core")]
impl FromWord for crate::model::VirtualWorkspaceId {
    fn from_word(word: u64) -> Option<Self> { Some(slotmap::KeyData::from_ffi(word).into()) }
}

#[cfg(feature = "roc-core")]
impl FromWord for crate::sys::window_server::WindowServerId {
    fn from_word(word: u64) -> Option<Self> {
        u32::try_from(word).ok().map(crate::sys::window_server::WindowServerId)
    }
}

#[cfg(feature = "roc-core")]
impl FromWord for crate::actor::app::pid_t {
    fn from_word(word: u64) -> Option<Self> { Self::try_from(word as i64).ok() }
}

/// The window a plain window names (the inverse of [`window`]).
#[cfg(feature = "roc-core")]
pub(crate) fn window_of_plain(
    window: rift_core::plain::WindowId,
) -> Option<crate::actor::app::WindowId> {
    let idx = std::num::NonZeroU32::new(window.idx)?;
    Some(crate::actor::app::WindowId { pid: window.pid, idx })
}

/// A family of decisions the Roc core decides (under `roc-core`), with how many times its
/// Rust shadow disagreed (each a `roc-divergence:`).
#[cfg(feature = "roc-core")]
pub(crate) struct Family {
    divergences: std::sync::atomic::AtomicU64,
}

#[cfg(feature = "roc-core")]
impl Family {
    const fn new() -> Self {
        Family {
            divergences: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub(crate) fn divergences(&self) -> u64 {
        self.divergences.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn count(&self) { self.divergences.fetch_add(1, std::sync::atomic::Ordering::Relaxed); }
}

/// The focus decisions (`rift_core::focus`, `Focus.roc`).
#[cfg(feature = "roc-core")]
pub(crate) static FOCUS: Family = Family::new();

/// The Roc core decides (policy (a)): its answer to `question` is rift's, and the Rust answer
/// (`rust`, as words: `words`) is its shadow. Equal, the Rust value is the Roc answer's own.
/// Different, `family` counts it, tests fail (`roc-divergence:`), and a running Rift warns and
/// takes the Roc answer, read back by `back`; a Roc answer naming nothing `back` can rebuild
/// (an id no value of rift's has, or a mode the question carried only as a word) leaves the Rust
/// answer, with a warn. A Roc decision that read a fact Rust did not ask has no answer on the
/// facts asked: counted, a test failure, and the Rust answer stands.
#[cfg(feature = "roc-core")]
#[expect(
    clippy::too_many_arguments,
    reason = "one decision's question, answers and reading"
)]
pub(crate) fn decided<Q: std::fmt::Debug, A: PartialEq + std::fmt::Debug, T>(
    family: &Family,
    what: &str,
    facts: &[&str],
    question: &Q,
    rust: T,
    words: &A,
    roc: Result<A, rift_core::plain::Unasked>,
    back: impl FnOnce(A) -> Option<T>,
) -> T {
    decided_under(
        Report::Divergence,
        family,
        what,
        facts,
        question,
        rust,
        words,
        roc,
        back,
    )
}

/// How [`decided_under`] reports a difference: as a divergence (a test failure, a warn in a
/// running Rift), or only counted (the tests of the running Rift's path).
#[cfg(feature = "roc-core")]
#[derive(Clone, Copy)]
enum Report {
    Divergence,
    #[cfg(test)]
    Counted,
}

#[cfg(feature = "roc-core")]
#[expect(
    clippy::too_many_arguments,
    reason = "one decision's question, answers and reading"
)]
fn decided_under<Q: std::fmt::Debug, A: PartialEq + std::fmt::Debug, T>(
    report: Report,
    family: &Family,
    what: &str,
    facts: &[&str],
    question: &Q,
    rust: T,
    words: &A,
    roc: Result<A, rift_core::plain::Unasked>,
    back: impl FnOnce(A) -> Option<T>,
) -> T {
    match roc {
        Ok(roc) if roc == *words => rust,
        Ok(roc) => {
            family.count();
            match report {
                Report::Divergence => diverged(what, question, words, &roc),
                #[cfg(test)]
                Report::Counted => {}
            }
            match back(roc) {
                Some(answer) => answer,
                None => {
                    tracing::warn!(
                        what,
                        "roc-divergence: the Roc answer names nothing rift holds; the Rust answer \
                         stands"
                    );
                    rust
                }
            }
        }
        Err(rift_core::plain::Unasked { fact }) => {
            family.count();
            match report {
                Report::Divergence => {
                    let fact = usize::try_from(fact).ok().and_then(|fact| facts.get(fact));
                    unasked(what, fact.copied().unwrap_or("(not one of its facts)"), question);
                }
                #[cfg(test)]
                Report::Counted => {}
            }
            rust
        }
    }
}

/// An admission change without its window, as `net`'s question carries it (`net` does not
/// read the window).
#[cfg(feature = "roc-core")]
pub(crate) fn window_free(
    change: Option<rift_core::lifecycle::Transition<crate::actor::app::WindowId>>,
) -> Option<rift_core::lifecycle::Change> {
    use rift_core::lifecycle::{Change, Transition};
    change.map(|change| match change {
        Transition::Admitted(_) => Change::Admitted,
        Transition::Suspended(_, why) => Change::Suspended(why),
    })
}

/// A point, as the plain questions carry it.
#[cfg(feature = "roc-core")]
pub(crate) fn point(point: objc2_core_foundation::CGPoint) -> rift_core::plain::Point {
    rift_core::plain::Point { x: point.x, y: point.y }
}

/// A window with its app, as the plain questions carry it.
#[cfg(feature = "roc-core")]
pub(crate) fn window(window: crate::actor::app::WindowId) -> rift_core::plain::WindowId {
    rift_core::plain::WindowId {
        pid: window.pid,
        idx: window.idx.get(),
    }
}

/// Holds the Roc core's answer to `question`, asked through its typed entry, to the Rust
/// answer, and reports a disagreement. `what` names the decision and `facts` its question's
/// facts (the question type's `FACTS`): the question carries only the facts the Rust decision
/// asked, so a Roc decision reading any other is a divergence too.
#[cfg(feature = "roc-core")]
pub(crate) fn check<Q: std::fmt::Debug, A: PartialEq + std::fmt::Debug>(
    what: &str,
    facts: &[&str],
    question: &Q,
    rust: &A,
    roc: Result<A, rift_core::plain::Unasked>,
) {
    match roc {
        Ok(roc) if roc == *rust => {}
        Err(rift_core::plain::Unasked { fact }) => {
            let fact = usize::try_from(fact).ok().and_then(|fact| facts.get(fact));
            unasked(what, fact.copied().unwrap_or("(not one of its facts)"), question)
        }
        Ok(roc) => diverged(what, question, rust, &roc),
    }
}

/// The Roc core read a fact the Rust decision did not ask: they decide in different orders.
#[cfg(feature = "roc-core")]
fn unasked(what: &str, fact: &str, question: &impl std::fmt::Debug) {
    #[cfg(test)]
    panic!("roc-divergence: {what} read a fact Rust didn't ask: {fact} {question:?}");
    #[cfg(not(test))]
    tracing::warn!(
        what,
        fact,
        ?question,
        "roc-divergence: the Roc core read a fact Rust didn't ask"
    );
}

/// The Roc core and its Rust shadow disagree on `what`.
#[cfg(feature = "roc-core")]
pub(crate) fn diverged(
    what: &str,
    question: &impl std::fmt::Debug,
    rust: &impl std::fmt::Debug,
    roc: &impl std::fmt::Debug,
) {
    #[cfg(test)]
    panic!("roc-divergence: {what} {question:?}: rust {rust:?}, roc {roc:?}");
    #[cfg(not(test))]
    tracing::warn!(
        what,
        ?question,
        ?rust,
        ?roc,
        "roc-divergence: the Roc core and its Rust shadow disagree"
    );
}

#[cfg(all(test, feature = "roc-core"))]
mod tests {
    use super::{Family, FromWord, Report, Word, decided_under, window, window_of_plain};

    /// Every id a decided answer names reads back as the value it was asked as.
    #[test]
    fn an_answer_word_reads_back_as_its_value() {
        use crate::actor::app::WindowId;
        use crate::model::VirtualWorkspaceId;
        use crate::sys::screen::SpaceId;
        use crate::sys::window_server::WindowServerId;
        fn round<T: Word + FromWord + Copy + PartialEq + std::fmt::Debug>(value: T) {
            assert_eq!(T::from_word(value.word()), Some(value));
        }
        round(WindowId::new(7, 3));
        round(WindowId::new(-1, 1));
        round(SpaceId::new(1 << 40));
        let mut workspaces = slotmap::SlotMap::<VirtualWorkspaceId, ()>::with_key();
        let first = workspaces.insert(());
        workspaces.remove(first);
        round(workspaces.insert(()));
        round(WindowServerId(4242));
        round::<crate::actor::app::pid_t>(-5);
        let held = WindowId::new(12, 9);
        assert_eq!(window_of_plain(window(held)), Some(held));
        assert_eq!(WindowServerId::from_word(1 << 33), None);
    }

    /// In a running Rift the Roc answer is rift's: a different one is taken (read back) and
    /// counted; one naming nothing rift holds, or a decision that read a fact Rust did not ask,
    /// leaves the Rust answer, counted too.
    #[test]
    fn a_decided_answer_is_the_roc_cores_and_each_difference_is_counted() {
        let family = Family::new();
        let decide = |roc: Result<u64, rift_core::plain::Unasked>| {
            decided_under(
                Report::Counted,
                &family,
                "test",
                &["fact"],
                &(),
                1_u32,
                &1_u64,
                roc,
                |word| u32::try_from(word).ok(),
            )
        };
        assert_eq!(decide(Ok(1)), 1);
        assert_eq!(family.divergences(), 0);
        assert_eq!(decide(Ok(2)), 2, "the Roc answer, read back");
        assert_eq!(family.divergences(), 1);
        assert_eq!(decide(Ok(1 << 40)), 1, "nothing to read back: the Rust answer");
        assert_eq!(family.divergences(), 2);
        assert_eq!(decide(Err(rift_core::plain::Unasked { fact: 0 })), 1);
        assert_eq!(family.divergences(), 3);
    }

    use rift_core::focus::plain::Desktop;
    use rift_core::lifecycle::WorkspaceMode;
    use rift_core::plain::Fact;
    use rift_core::workspaces::plain::PreviousQuestion;

    use super::check;

    fn question() -> PreviousQuestion {
        PreviousQuestion {
            mode: WorkspaceMode::Global,
            space: 10,
            visible: vec![10, 20],
            last: Some(3),
            on: Fact::Asked(Some(20)),
        }
    }

    #[test]
    fn an_answer_both_cores_give_passes() {
        let question = question();
        let roc = rift_core_roc::workspaces::previous(question.clone());
        let rust = question.answer().expect("the facts it asks");
        check("previous", PreviousQuestion::FACTS, &question, &rust, roc);
    }

    #[test]
    #[should_panic(expected = "roc-divergence: previous")]
    fn a_different_answer_is_a_divergence() {
        let question = question();
        let roc = rift_core_roc::workspaces::previous(question.clone());
        check("previous", PreviousQuestion::FACTS, &question, &None, roc);
    }

    #[test]
    #[should_panic(
        expected = "roc-divergence: focuses_desktop read a fact Rust didn't ask: workspace_empty"
    )]
    fn reading_a_fact_rust_did_not_ask_is_a_divergence() {
        // An active Space: the decision asks whether its workspace is empty, which this
        // question says the Rust decision never asked.
        let question = Desktop {
            active: true,
            workspace_empty: Fact::Unasked,
        };
        let roc = rift_core_roc::focus::focuses_desktop(question);
        check("focuses_desktop", Desktop::FACTS, &question, &false, roc);
    }
}
