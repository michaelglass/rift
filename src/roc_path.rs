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
/// Rust answer encodes to `rust`, and reports a disagreement. `what` names the decision.
pub(crate) fn check(what: &str, question: &[u64], rust: &[u64]) {
    match rift_core_roc::tile_words(question) {
        Ok(roc) if roc == rust => {}
        roc => diverged(what, question, rust, &roc),
    }
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
    use rift_core::lifecycle::WorkspaceMode;
    use rift_core::plain::wire;
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
        check("previous", &wire::encode_previous(question), &rust);
    }

    #[test]
    #[should_panic(expected = "roc-divergence: previous")]
    fn a_different_answer_is_a_divergence() {
        let question = question();
        let wrong = wire::encode_target(None);
        check("previous", &wire::encode_previous(question), &wrong);
    }

    #[test]
    #[should_panic(expected = "roc-divergence: malformed")]
    fn no_answer_is_a_divergence() { check("malformed", &[29], &[0]); }
}
