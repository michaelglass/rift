//! Questions whose facts are what the platform's run of a decision asked (`rift_core::plain::Fact`),
//! run through both cores: on full facts, and with facts left unasked, where both cores must
//! read the same facts in the same order.

use rift_core::plain::{Answers, Fact, Unasked};
use rift_core_roc::{AllocationCount, RocError, tile_words};

/// A source of chances, the test's random generator.
pub trait Coin {
    fn chance(&mut self, percent: u64) -> bool;
}

/// The Rust core's answer, or the fact it read that its question does not carry.
pub type Answered = Result<Vec<u64>, u64>;

pub fn rust<A>(answer: Result<A, Unasked>, encode: impl FnOnce(A) -> Vec<u64>) -> Answered {
    answer.map(encode).map_err(|unasked| unasked.fact)
}

fn roc(question: &[u64]) -> Answered {
    match tile_words(question) {
        Ok(words) => Ok(words),
        Err(RocError::Unasked { fact }) => Err(fact),
        Err(error) => panic!("no answer to {question:?}: {error:?}"),
    }
}

/// Runs random questions through both cores, twice: on full facts (every fact asked, as the
/// decision's own differential has always run), then with facts left unasked at random, where
/// both cores must read the same facts in the same order: the same answer, or the same first
/// fact read that the question does not carry. `ask` makes a question with `Blank`'s facts and
/// returns its words and the Rust core's answer. `RIFT_<GROUP>_CASES` sets the cases per pass.
pub fn differential<R: Coin>(
    group: &str,
    name: &str,
    mut rng: R,
    mut ask: impl FnMut(&mut R, Blank) -> (Vec<u64>, Answered),
) {
    let cases: u64 = std::env::var(format!("RIFT_{}_CASES", group.to_uppercase()))
        .map_or(20_000, |n| n.parse().unwrap());
    let before = AllocationCount::now();
    let mut unasked = 0;
    for blank in [Blank::Full, Blank::Some] {
        for case in 0..cases {
            let (question, rust) = ask(&mut rng, blank);
            if let Blank::Full = blank {
                assert!(
                    rust.is_ok(),
                    "case {case}: {name} on full facts read a missing one"
                );
            }
            unasked += u64::from(rust.is_err());
            assert_eq!(
                roc(&question),
                rust,
                "case {case} ({blank:?}): {name} {question:?}"
            );
        }
    }
    let after = AllocationCount::now();
    assert_eq!(after.live(), before.live(), "Roc allocations left live");
    eprintln!(
        "{group}: {cases} {name} questions on full facts and {cases} with facts unasked answered \
         identically ({unasked} read an unasked fact)"
    );
}

/// Which facts a question carries.
#[derive(Debug, Clone, Copy)]
pub enum Blank {
    /// All of them.
    Full,
    /// Each left unasked (each key's answer dropped) one time in four.
    Some,
}

impl Blank {
    pub fn fact<T>(self, coin: &mut impl Coin, answer: T) -> Fact<T> {
        match self {
            Blank::Some if coin.chance(25) => Fact::Unasked,
            Blank::Full | Blank::Some => Fact::Asked(answer),
        }
    }

    /// Answers for `keys` (each once), as `answer` gives them.
    pub fn answers<K: Copy + PartialEq, V>(
        self,
        coin: &mut impl Coin,
        keys: impl IntoIterator<Item = K>,
        answer: impl Fn(K) -> V,
    ) -> Answers<K, V> {
        let mut asked: Vec<K> = Vec::new();
        for key in keys {
            if !asked.contains(&key) {
                asked.push(key);
            }
        }
        Answers(
            asked
                .into_iter()
                .filter(|_| match self {
                    Blank::Full => true,
                    Blank::Some => !coin.chance(25),
                })
                .map(|key| (key, answer(key)))
                .collect(),
        )
    }
}
