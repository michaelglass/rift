## Values by key, in slots, with slotmap's (1.1) rules (`rift_core::slot_table`): the layout
## trees' tables. Slot 0 is the sentinel: always vacant, never handed out. Each slot holds a
## value under an odd version, or is vacant under an even one. `free` holds the vacant slots in
## the order they are taken: the last first. Keys are slotmap's ffi form, one word each
## (`version << 32 | index`).
## One slot: a value under an odd version, or vacant under an even one.
Slot(v) : [Occupied({ version : U32, value : v }), Vacant(U32)]

## A vacant slot on the free list, with the version its next value takes (odd, one past the
## slot's vacant version): inserting never reads the slot.
Free : { index : U32, version : U32 }

Slots(v) := { slots : List(Slot(v)), free : List(Free) }.{

	empty : {} -> Slots(v)
	empty = |{}| { slots: [Vacant(0)], free: [] }

	## A table as the host holds it, parsed once: each free slot's next version read from its
	## vacant version. The host's table is slotmap's, whose free list names only vacant slots; a
	## seed naming a held slot is not one, and is refused here, at the boundary.
	from_parts : List(Slot(v)), List(U32) -> Slots(v)
	from_parts = |slots, vacated| {
		freed = |index|
			match slots.get(index.to_u64()) {
				Ok(Vacant(vacant)) => { index, version: vacant.bitwise_or(1) }
				_ => {
					crash "Slots.from_parts: free slot ${index.to_str()} is not vacant"
				}
			}
		{ slots, free: vacated.map(freed) }
	}

	## The slots, as the host reads them back.
	slot_list : Slots(v) -> List(Slot(v))
	slot_list = |table| table.slots

	## The free list as the host holds it: the vacant slots' indices, the next taken last.
	free_indices : Slots(v) -> List(U32)
	free_indices = |table| table.free.map(|freed| freed.index)

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
			Ok({ index, version }) => {
				at = index.to_u64()
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
				vacant = version.plus_wrap(1)
				slots = put(table.slots, index, Vacant(vacant))
				({ slots, free: table.free.append({ index: index.to_u32_wrap(), version: vacant.bitwise_or(1) }) }, Took(held.value))
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

	## The slots with slot `index`, one just read, set. The out-of-bounds branch stays: every
	## caller has just read `index` from these slots, but the builtins have no list update that
	## cannot miss, so the type cannot say so.
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
