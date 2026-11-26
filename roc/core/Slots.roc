## Values by key, in slots, with slotmap's (1.1) rules (`rift_core::slot_table`): the layout
## trees' tables. Slot 0 is the sentinel: always vacant, never handed out. Each slot holds a
## value under an odd version, or is vacant under an even one. `free` holds the vacant slots in
## the order they are taken: the last first. Keys are slotmap's ffi form, one word each
## (`version << 32 | index`).
## One slot: a value under an odd version, or vacant under an even one.
Slot(v) : [Occupied({ version : U32, value : v }), Vacant(U32)]

Slots(v) := { slots : List(Slot(v)), free : List(U32) }.{

	empty : {} -> Slots(v)
	empty = |{}| { slots: [Vacant(0)], free: [] }

	## The key of slot `index` under `version`.
	key_of : U64, U32 -> U64
	key_of = |index, version| version.to_u64().shl_wrap(32).bitwise_or(index)

	## A key's slot index and version.
	parts : U64 -> (U64, U32)
	parts = |key| (key.bitwise_and(0xFFFF_FFFF), key.shr_zf_wrap(32).to_u32_wrap())

	## `value` stored in the most recently vacated slot (at its version `| 1`), or a new one
	## (at version 1).
	insert : Slots(v), v -> (Slots(v), U64)
	insert = |table, value|
		match table.free.last() {
			Ok(index) => {
				at = index.to_u64()
				version =
					match table.slots.get(at) {
						Ok(Vacant(vacant)) => vacant.bitwise_or(1)
						_ => {
							crash "Slots.insert: free slot ${at.to_str()} is not vacant"
						}
					}
				slots = put(table.slots, at, Occupied({ version, value }))
				({ slots, free: table.free.drop_last(1) }, key_of(at, version))
			}
			Err(_) => {
				at = table.slots.len()
				({ ..table, slots: table.slots.append(Occupied({ version: 1, value })) }, key_of(at, 1))
			}
		}

	## `key`'s value taken out; its slot (at its version plus one) is the next one taken.
	remove : Slots(v), U64 -> (Slots(v), [Took(v), NotHeld])
	remove = |table, key| {
		(index, version) = parts(key)
		match table.slots.get(index) {
			Ok(Occupied(held)) if held.version == version => {
				slots = put(table.slots, index, Vacant(version.plus_wrap(1)))
				({ slots, free: table.free.append(index.to_u32_wrap()) }, Took(held.value))
			}
			_ => (table, NotHeld)
		}
	}

	get : Slots(v), U64 -> [Held(v), NotHeld]
	get = |table, key| {
		(index, version) = parts(key)
		match table.slots.get(index) {
			Ok(Occupied(held)) if held.version == version => Held(held.value)
			_ => NotHeld
		}
	}

	## The table with `key`'s value replaced, if it holds one.
	replace : Slots(v), U64, v -> [Held(Slots(v)), NotHeld]
	replace = |table, key, value| {
		(index, version) = parts(key)
		match table.slots.get(index) {
			Ok(Occupied(held)) if held.version == version =>
				Held({ ..table, slots: put(table.slots, index, Occupied({ version, value })) })
			_ => NotHeld
		}
	}

	## The slots with slot `index`, one just read, set.
	put : List(Slot(v)), U64, Slot(v) -> List(Slot(v))
	put = |slots, index, slot|
		match slots.set(index, slot) {
			Ok(set) => set
			Err(OutOfBounds) => {
				crash "Slots.put: no slot ${index.to_str()}"
			}
		}

	## One mark per slot, none set: the slots a pass picks out, by slot index (a key's version is
	## not looked at; the marks are only read against the table they were made for).
	unmarked : Slots(v) -> List(Bool)
	unmarked = |table| List.repeat(False, table.slots.len())

	## `key`'s slot marked (a key past the table's slots names no slot: nothing to mark).
	mark : List(Bool), U64 -> List(Bool)
	mark = |marks, key| {
		(index, _) = parts(key)
		match marks.set(index, True) {
			Ok(set) => set
			Err(OutOfBounds) => marks
		}
	}

	## Whether `key`'s slot is marked.
	marked : List(Bool), U64 -> Bool
	marked = |marks, key| {
		(index, _) = parts(key)
		match marks.get(index) {
			Ok(mark_) => mark_
			Err(OutOfBounds) => False
		}
	}

	## Every held value changed by `change`, the slots and versions as they were.
	map_values : Slots(v), (v -> v) -> Slots(v)
	map_values = |table, change| {
		..table,
		slots: table.slots.map(
			|slot|
				match slot {
					Occupied({ version, value }) => Occupied({ version, value: change(value) })
					Vacant(version) => Vacant(version)
				},
		),
	}
}
