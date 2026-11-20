//! The Roc core on rift's path for the pure decisions that take no model (the `roc-core`
//! feature): each call site asks the Roc core the question it just answered in Rust, on plain
//! data, and the two answers must be the same words. Equal, the Rust answer stands: it is the
//! Roc core's decision, bit for bit. Different (or no answer), tests fail, and a running Rift
//! logs `roc-divergence:` and goes on with the Rust answer.
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

#[cfg(feature = "roc-core")]
impl Word for crate::sys::screen::SpaceId {
    fn word(self) -> u64 { self.get() }
}

#[cfg(feature = "roc-core")]
impl Word for crate::model::VirtualWorkspaceId {
    fn word(self) -> u64 { slotmap::Key::data(&self).as_ffi() }
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

/// An admission change as the plain questions carry it (its window as a word).
#[cfg(feature = "roc-core")]
pub(crate) fn transition(
    change: Option<rift_core::lifecycle::Transition<crate::actor::app::WindowId>>,
) -> Option<rift_core::lifecycle::Transition<u64>> {
    use rift_core::lifecycle::Transition;
    change.map(|change| match change {
        Transition::Admitted(window) => Transition::Admitted(window.word()),
        Transition::Suspended(window, why) => Transition::Suspended(window.word(), why),
    })
}

/// A window with its app, as the plain questions carry it.
#[cfg(feature = "roc-core")]
pub(crate) fn window(window: crate::actor::app::WindowId) -> rift_core::plain::WindowId {
    rift_core::plain::WindowId {
        pid: window.pid,
        idx: window.idx.get(),
    }
}

/// Asks the Roc core `question` (a tagged question's words, `rift_core::plain::wire`), whose
/// Rust answer encodes to `rust`, and reports a disagreement. `what` names the decision and
/// `facts` its question's facts (the question type's `FACTS`): the question carries only the
/// facts the Rust decision asked, so a Roc decision reading any other is a divergence too.
#[cfg(feature = "roc-core")]
pub(crate) fn check(what: &str, facts: &[&str], question: &[u64], rust: &[u64]) {
    match rift_core_roc::tile_words(question) {
        Ok(roc) if roc == rust => {}
        Err(rift_core_roc::RocError::Unasked { fact }) => {
            let fact = usize::try_from(fact).ok().and_then(|fact| facts.get(fact));
            unasked(what, fact.copied().unwrap_or("(not one of its facts)"), question)
        }
        roc => diverged(what, question, rust, &roc),
    }
}

/// The Roc core read a fact the Rust decision did not ask: they decide in different orders.
#[cfg(feature = "roc-core")]
fn unasked(what: &str, fact: &str, question: &[u64]) {
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
fn diverged(
    what: &str,
    question: &[u64],
    rust: &[u64],
    roc: &Result<Vec<u64>, rift_core_roc::RocError>,
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
    use rift_core::focus::plain::Desktop;
    use rift_core::lifecycle::WorkspaceMode;
    use rift_core::plain::{Fact, wire};
    use rift_core::workspaces::plain::PreviousQuestion;

    use super::check;

    fn question() -> PreviousQuestion {
        PreviousQuestion {
            mode: WorkspaceMode::Global,
            space: 10,
            visible: vec![10, 20],
            last: Some(3),
            on: Some(20),
        }
    }

    #[test]
    fn an_answer_both_cores_give_passes() {
        let question = question();
        let rust = wire::encode_target(question.answer());
        check("previous", &[], &wire::encode_previous(question), &rust);
    }

    #[test]
    #[should_panic(expected = "roc-divergence: previous")]
    fn a_different_answer_is_a_divergence() {
        let question = question();
        let wrong = wire::encode_target(None);
        check("previous", &[], &wire::encode_previous(question), &wrong);
    }

    #[test]
    #[should_panic(expected = "roc-divergence: malformed")]
    fn no_answer_is_a_divergence() { check("malformed", &[], &[29], &[0]); }

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
        check(
            "focuses_desktop",
            Desktop::FACTS,
            &wire::encode_desktop(question),
            &wire::encode_verdict(false),
        );
    }
}
