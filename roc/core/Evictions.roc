## Where a display's departure moved windows from (`rift_core::evictions`): for each evicted
## window, the display it was moved off, its workspace's position there and the workspace the
## departure gave it. Windows, displays and workspaces are one word each, a pid an I32.

import pf.Boundary exposing [EvictionEvent, EvictionRecord, MaybeWord]
import Floats

## A window's record, without its window.
Record : { display : U64, ordinal : U64, landed : U64 }

## Every evicted window's record.
Kept : { records : Dict(U64, Record) }

Evictions := [].{

	empty : {} -> Kept
	empty = |{}| { records: Dict.empty() }

	step : Kept, EvictionEvent -> Kept
	step = |kept, event|
		match event {
			# A window that stayed on its display was not evicted.
			Evicted(moved) =>
				if moved.landed_on == Some(moved.from) {
					kept
				} else {
					{ records: kept.records.insert(moved.window, { display: moved.from, ordinal: moved.ordinal, landed: moved.landed }) }
				}
			Forgotten(window) => { records: kept.records.remove(window) }
			Replaced(pair) =>
				if pair.from == pair.to {
					kept
				} else {
					match kept.records.get(pair.from) {
						Ok(record) => { records: kept.records.remove(pair.from).remove(pair.to).insert(pair.to, record) }
						Err(_) => { records: kept.records.remove(pair.to) }
					}
				}
			AppQuit(pid) => { records: kept.records.keep_if(|(window, _)| !Floats.of_app(window, pid)) }
		}

	## Every record, in window order.
	records : Kept -> List(EvictionRecord)
	records = |kept|
		kept.records
			.to_list()
			.sort_with(|(a, _), (b, _)| if a < b Before else if a > b After else Same)
			.map(|(window, record)| EvictionRecord.{ window, display: record.display, ordinal: record.ordinal, landed: record.landed })
}
