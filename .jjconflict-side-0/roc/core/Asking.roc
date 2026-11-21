## A fact the platform's run of a decision answered, or one it never asked, and reading one in
## the order the decision asks: the decisions' questions carry exactly the facts the platform's
## run of each decision asked (`rift_core::plain::Fact`).

WindowId : { pid : I32, idx : U32 }

## A fact the platform's run of a decision answered, or one it never asked.
Fact(a) : [Asked(a), Unasked]

## A fact asked about keys: each key asked and its answer. Any other key was not asked.
Answers(k, v) : List({ key : k, answer : v })

## What a decision decided, or the first fact it read that its question does not carry (the
## fact's place among the question's facts): it decides in another order than the platform's.
Read(a) : Try(a, [Unasked(U64)])

Asking := [].{
	## The `fact`-th fact's answer.
	ask : Fact(a), U64 -> Read(a)
	ask = |given, fact|
		match given {
			Asked(answer) => Ok(answer)
			Unasked => Err(Unasked(fact))
		}

	## The `fact`-th fact's answer for a window.
	ask_window : Answers(WindowId, v), WindowId, U64 -> Read(v)
	ask_window = |answers, key, fact| {
		var $found = Err(Unasked(fact))
		for entry in answers {
			match $found {
				Err(Unasked(_)) => if (entry.key == key) { $found = Ok(entry.answer) } else {}
				Ok(_) => {}
			}
		}
		$found
	}

	## The `fact`-th fact's answer for a word.
	ask_word : Answers(U64, v), U64, U64 -> Read(v)
	ask_word = |answers, key, fact| {
		var $found = Err(Unasked(fact))
		for entry in answers {
			match $found {
				Err(Unasked(_)) => if (entry.key == key) { $found = Ok(entry.answer) } else {}
				Ok(_) => {}
			}
		}
		$found
	}
}
