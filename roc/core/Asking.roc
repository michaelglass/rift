## A fact the platform's run of a decision answered, or one it never asked, and reading one in
## the order the decision asks: the decisions' questions carry exactly the facts the platform's
## run of each decision asked (`rift_core::plain::Fact`).

import pf.Boundary exposing [WindowId]

## What a decision decided, or the first fact it read that its question does not carry (the
## fact's place among the question's facts): it decides in another order than the platform's.
Read(a) : Try(a, [Unasked(U64)])

Asking := [].{
	## The `fact`-th fact's answer. A fact is one of the platform's `Fact…` types, each of which
	## answers what the platform asked, if it asked.
	ask : f, U64 -> Read(a) where [f.answer : f -> Try(a, [Unasked])]
	ask = |given, fact|
		match given.answer() {
			Ok(answer) => Ok(answer)
			Err(Unasked) => Err(Unasked(fact))
		}

	## The `fact`-th fact's answer for a window. Each answer is one of the platform's keyed types
	## (a key asked and its answer); any other key was not asked.
	ask_window : List(e), WindowId, U64 -> Read(v) where [e.entry : e -> { key : WindowId, answer : v }]
	ask_window = |answers, key, fact| {
		var $found = Err(Unasked(fact))
		for given in answers {
			entry = given.entry()
			match $found {
				Err(Unasked(_)) => if (entry.key == key) { $found = Ok(entry.answer) } else {}
				Ok(_) => {}
			}
		}
		$found
	}

	## The `fact`-th fact's answer for a word.
	ask_word : List(e), U64, U64 -> Read(v) where [e.entry : e -> { key : U64, answer : v }]
	ask_word = |answers, key, fact| {
		var $found = Err(Unasked(fact))
		for given in answers {
			entry = given.entry()
			match $found {
				Err(Unasked(_)) => if (entry.key == key) { $found = Ok(entry.answer) } else {}
				Ok(_) => {}
			}
		}
		$found
	}
}
