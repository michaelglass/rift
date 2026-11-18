## The word codec's primitives (`rift_core::plain::wire`): every field is one word; an option
## is a flag word and then the value; a list is its length and then its items.

WindowId : { pid : I32, idx : U32 }

Rect : { x : F64, y : F64, w : F64, h : F64 }

## Words being read, and where.
Words : { words : List(U64), at : U64 }

Wire := [].{
	take : Words -> (U64, Words)
	take = |cursor|
		match cursor.words.get(cursor.at) {
			Ok(word) => (word, { ..cursor, at: cursor.at + 1 })
			Err(_) => crash "wire: the words ended inside a value"
		}

	take_u32 : Words -> (U32, Words)
	take_u32 = |cursor| {
		(word, rest) = take(cursor)
		(word.to_u32_wrap(), rest)
	}

	take_window : Words -> (WindowId, Words)
	take_window = |cursor| {
		(pid, after_pid) = take(cursor)
		(idx, rest) = take_u32(after_pid)
		({ pid: pid.to_i32_wrap(), idx }, rest)
	}

	take_float : Words -> (F64, Words)
	take_float = |cursor| {
		(word, rest) = take(cursor)
		(F64.from_bits(word), rest)
	}

	take_rect : Words -> (Rect, Words)
	take_rect = |cursor| {
		(x, c1) = take_float(cursor)
		(y, c2) = take_float(c1)
		(w, c3) = take_float(c2)
		(h, c4) = take_float(c3)
		({ x, y, w, h }, c4)
	}

	take_flag : Words -> (Bool, Words)
	take_flag = |cursor| {
		(word, rest) = take(cursor)
		match word {
			0 => (False, rest)
			1 => (True, rest)
			_ => crash "wire: a flag is 0 or 1"
		}
	}

	take_list : Words, (Words -> (item, Words)) -> (List(item), Words)
	take_list = |cursor0, take_item| {
		(count, after_count) = take(cursor0)
		var $cursor = after_count
		var $items = []
		var $left = count
		while $left > 0 {
			(item, next) = take_item($cursor)
			$cursor = next
			$items = $items.append(item)
			$left = $left - 1
		}
		($items, $cursor)
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
}
