## The word codec's primitives (`rift_core::plain::wire`): every field is one word; an option
## is a flag word and then the value; a list is its length and then its items. Reading is a
## `Try`: words that are not a well-formed value are `Malformed`, which the entry points turn
## into one explicit rejection for the host (never a crash inside a decode).

WindowId : { pid : I32, idx : U32 }

Rect : { x : F64, y : F64, w : F64, h : F64 }

## Words being read, and where.
Words : { words : List(U64), at : U64 }

## What reading gives back: a value and where the rest starts, or the words were malformed.
Taken(a) : Try((a, Words), [Malformed])

## A fact the platform's run of a decision answered, or one it never asked.
Fact(a) : [Asked(a), Unasked]

## A fact asked about keys: each key asked and its answer. Any other key was not asked.
Answers(k, v) : List({ key : k, answer : v })

## What a decision decided, or the first fact it read that its question does not carry (the
## fact's place among the question's facts): it decides in another order than the platform's.
Read(a) : Try(a, [Unasked(U64)])

Wire := [].{
	take : Words -> Taken(U64)
	take = |cursor| {
		word = cursor.words.get(cursor.at) ? |_| Malformed
		Ok((word, { ..cursor, at: cursor.at + 1 }))
	}

	## Every word was read: anything left over is malformed.
	finished : Words -> Try({}, [Malformed])
	finished = |cursor| if (cursor.at == cursor.words.len()) Ok({}) else Err(Malformed)

	take_u32 : Words -> Taken(U32)
	take_u32 = |cursor| {
		(word, rest) = take(cursor)?
		if (word > 4294967295) Err(Malformed) else Ok((word.to_u32_wrap(), rest))
	}

	take_window : Words -> Taken(WindowId)
	take_window = |cursor| {
		(pid, after_pid) = take(cursor)?
		(idx, rest) = take_u32(after_pid)?
		signed = pid.to_i64_wrap()
		if signed < -2147483648 or signed > 2147483647 {
			Err(Malformed)
		} else {
			Ok(({ pid: pid.to_i32_wrap(), idx }, rest))
		}
	}

	take_float : Words -> Taken(F64)
	take_float = |cursor| {
		(word, rest) = take(cursor)?
		Ok((F64.from_bits(word), rest))
	}

	take_rect : Words -> Taken(Rect)
	take_rect = |cursor| {
		(x, c1) = take_float(cursor)?
		(y, c2) = take_float(c1)?
		(w, c3) = take_float(c2)?
		(h, c4) = take_float(c3)?
		Ok(({ x, y, w, h }, c4))
	}

	take_flag : Words -> Taken(Bool)
	take_flag = |cursor| {
		(word, rest) = take(cursor)?
		match word {
			0 => Ok((False, rest))
			1 => Ok((True, rest))
			_ => Err(Malformed)
		}
	}

	## A list: its length, then its items. A length longer than the words left is malformed
	## (each item is at least a word).
	take_list : Words, (Words -> Taken(item)) -> Taken(List(item))
	take_list = |cursor0, take_item| {
		(count, after_count) = take(cursor0)?
		if count > after_count.words.len() - after_count.at {
			Err(Malformed)
		} else {
			var $cursor = after_count
			var $items = []
			var $left = count
			while $left > 0 {
				(item, next) = take_item($cursor)?
				$cursor = next
				$items = $items.append(item)
				$left = $left - 1
			}
			Ok(($items, $cursor))
		}
	}

	put_window : List(U64), WindowId -> List(U64)
	put_window = |words, window| words.append(window.pid.to_i64().to_u64_wrap()).append(window.idx.to_u64())

	put_float : List(U64), F64 -> List(U64)
	put_float = |words, value| words.append(F64.to_bits(value))

	put_rect : List(U64), Rect -> List(U64)
	put_rect = |words, rect| put_float(put_float(put_float(put_float(words, rect.x), rect.y), rect.w), rect.h)

	put_flag : List(U64), Bool -> List(U64)
	put_flag = |words, flag| words.append(if (flag) 1 else 0)

	put_list : List(U64), List(item), (List(U64), item -> List(U64)) -> List(U64)
	put_list = |words0, items, put_item| {
		var $words = words0.append(items.len())
		for item in items {
			$words = put_item($words, item)
		}
		$words
	}

	## A fact: a flag (0 unasked, 1 asked) and then its answer.
	take_fact : Words, (Words -> Taken(a)) -> Taken(Fact(a))
	take_fact = |c0, take_answer| {
		(asked, c1) = take_flag(c0)?
		if asked {
			(answer, c2) = take_answer(c1)?
			Ok((Asked(answer), c2))
		} else {
			Ok((Unasked, c1))
		}
	}

	## Answers: a list of (key, answer).
	take_answers : Words, (Words -> Taken(k)), (Words -> Taken(v)) -> Taken(Answers(k, v))
	take_answers = |c0, take_key, take_answer|
		take_list(
			c0,
			|c| {
				(key, after) = take_key(c)?
				(answer, rest) = take_answer(after)?
				Ok(({ key, answer }, rest))
			},
		)

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
