## What the layout engine keeps of a workspace's arrangement for a departed display's return
## (`rift_core::native_memory`), as the Roc core owns it: which workspaces hold a snapshot, and the
## reconcile batch. The snapshots' values stay with the host; each event that reads a snapshot's
## windows carries them (computed from the value), never kept here. Workspaces and windows are one
## word each.

import pf.Boundary exposing [NativeEvent, NativeAnswer, NativePlan, NativeCurrent, NativeBatchView, MaybeWord]

## The workspaces holding a snapshot, and the batch: idle, or the workspaces waiting for its end.
Kept : { held : Set(U64), batch : [Idle, Collecting(Set(U64))] }

NativeMemory := [].{

	empty : {} -> Kept
	empty = |{}| { held: Set.empty(), batch: Idle }

	step : Kept, NativeEvent -> (Kept, NativeAnswer)
	step = |kept, event|
		match event {
			Kept(k) =>
				if kept.held.contains(k.workspace) or k.windows.is_empty() {
					(kept, Nothing)
				} else {
					({ ..kept, held: kept.held.insert(k.workspace) }, Nothing)
				}
			Edited(e) =>
				if e.windows.is_empty() {
					({ ..kept, held: kept.held.remove(e.workspace) }, Nothing)
				} else {
					(kept, Nothing)
				}
			Forgotten(workspace) => {
				batch =
					match kept.batch {
						Collecting(waiting) => Collecting(waiting.remove(workspace))
						Idle => Idle
					}
				({ held: kept.held.remove(workspace), batch }, Nothing)
			}
			Cleared => ({ ..kept, held: Set.empty() }, Nothing)
			ReconcileBegan => ({ ..kept, batch: Collecting(Set.empty()) }, Nothing)
			ReconcileEnded => {
				waited =
					match kept.batch {
						Collecting(waiting) => ascending(waiting.to_list())
						Idle => []
					}
				({ ..kept, batch: Idle }, Waited(waited))
			}
			RestoreAsked(ask) => restore(kept, ask.workspace, ask.returning, ask.current, ask.held)
		}

	ascending : List(U64) -> List(U64)
	ascending = |words| words.sort_with(|a, b| if a < b Before else if a > b After else Same)

	restore : Kept, U64, MaybeWord, NativeCurrent, List(U64) -> (Kept, NativeAnswer)
	restore = |kept, workspace, returning, current, original|
		if !kept.held.contains(workspace) {
			(kept, Nothing)
		} else {
			not_held =
				match returning {
					Some(window) => !original.contains(window)
					Nothing => False
				}
			if not_held {
				(kept, Nothing)
			} else {
				match kept.batch {
					Collecting(waiting) => ({ ..kept, batch: Collecting(waiting.insert(workspace)) }, Deferred)
					Idle =>
						match current {
							Unlaid => (kept, Nothing)
							Laid(candidates) => {
								# The workspace's own present, tiled windows, each once, in its order.
								(ordered, desired) = candidates.fold(
									([], Set.empty()),
									|(list, seen), c|
										if c.in_workspace and c.present and !c.floating and !seen.contains(c.window) {
											(list.append(c.window), seen.insert(c.window))
										} else {
											(list, seen)
										},
								)
								if !original.any(|w| desired.contains(w)) {
									(kept, Nothing)
								} else {
									spent = original.all(|w| desired.contains(w))
									plan = NativePlan.{
										remove: original.keep_if(|w| !desired.contains(w)),
										add: ordered.keep_if(|w| !original.contains(w)),
										ordered,
										spent,
									}
									after = if spent { ..kept, held: kept.held.remove(workspace) } else kept
									(after, Restore(plan))
								}
							}
						}
				}
			}
		}

	held_list : Kept -> List(U64)
	held_list = |kept| ascending(kept.held.to_list())

	batch_view : Kept -> NativeBatchView
	batch_view = |kept|
		match kept.batch {
			Collecting(waiting) => Collecting(ascending(waiting.to_list()))
			Idle => Idle
		}
}
