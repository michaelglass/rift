//! The Roc core on rift's path for the pure decisions that take no model (the `roc-core`
//! feature): each call site asks the Roc core the question it just answered in Rust, on plain
//! data, and the two answers must be the same words. Equal, the Rust answer stands: it is the
//! Roc core's decision, bit for bit. Different (or no answer), tests fail, and a running Rift
//! logs `roc-divergence:` and goes on with the Rust answer.
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "its first call sites come with the focus decisions"
    )
)]

/// Asks the Roc core `question` (a tagged question's words, `rift_core::plain::wire`), whose
/// Rust answer encodes to `rust`, and reports a disagreement. `what` names the decision and
/// `facts` its question's facts (the question type's `FACTS`): the question carries only the
/// facts the Rust decision asked, so a Roc decision reading any other is a divergence too.
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

#[cfg(test)]
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
