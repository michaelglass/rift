## What the layout passes know of each window's size (`rift_core::constraints`): its
## constraints, kept for as long as the window lives (upstream #707), and each app's last
## reported minimum, which a `respect_min_size` window reporting none takes. `revision` moves on
## every change to the records. Windows and apps are one word each (a pid sign-extended).

import pf.Boundary exposing [SizesEvent, SizeObserved, ReportedMin, WindowConstraints, Sizing, Size, SizeRecord, AppMinimum]
import Geometry exposing [normalized]

## Every window's constraints, each app's last reported minimum, and how often the records
## changed.
Kept : { windows : Dict(U64, WindowConstraints), app_minimums : Dict(U64, Size), revision : U64 }

Sizes := [].{

	empty : {} -> Kept
	empty = |{}| { windows: Dict.empty(), app_minimums: Dict.empty(), revision: 0 }

	## Applies `event`; whether the records or the app minimums changed (`revision` moved).
	step : Kept, SizesEvent -> (Kept, Bool)
	step = |sizes, event| {
		(after, changed) = apply(sizes, event)
		(if changed { ..after, revision: after.revision + 1 } else after, changed)
	}

	apply : Kept, SizesEvent -> (Kept, Bool)
	apply = |sizes, event|
		match event {
			Observed(seen) => {
				(app_minimums, min) =
					if seen.respect_min_size {
						match seen.observed.min {
							Min(reported) => (sizes.app_minimums.insert(seen.pid, reported), Min(reported))
							NoMin =>
								match sizes.app_minimums.get(seen.pid) {
									Ok(last) => (sizes.app_minimums, Min(last))
									Err(_) => (sizes.app_minimums, NoMin)
								}
						}
					} else {
						(sizes.app_minimums, seen.observed.min)
					}
				made = record(seen.observed, seen.respect_min_size, min, seen.observed.width)
				recorded({ ..sizes, app_minimums }, seen.window, made)
			}
			WidthSet(set) =>
				match sizes.windows.get(set.window) {
					Ok(known) => recorded(sizes, set.window, normalized(locked_at(known, set.width)))
					Err(_) => (sizes, False)
				}
			WidthPreserved(kept) => {
				base =
					match sizes.windows.get(kept.window) {
						Ok(known) => known
						Err(_) => record(kept.observed, False, kept.observed.min, 0.0)
					}
				recorded(sizes, kept.window, normalized(locked_at(base, kept.width)))
			}
			Retired(window) =>
				match sizes.windows.get(window) {
					Ok(_) => ({ ..sizes, windows: sizes.windows.remove(window) }, True)
					Err(_) => (sizes, False)
				}
			Replaced(replaced) =>
				match sizes.windows.get(replaced.from) {
					Ok(known) => ({ ..sizes, windows: sizes.windows.remove(replaced.from).insert(replaced.to, known) }, True)
					Err(_) => (sizes, False)
				}
			# Not a window's record: a layout pass reads none of it.
			AppQuit(pid) => ({ ..sizes, app_minimums: sizes.app_minimums.remove(pid) }, False)
		}

	## `window` recorded as `made`; whether that changed its record.
	recorded : Kept, U64, WindowConstraints -> (Kept, Bool)
	recorded = |sizes, window, made| {
		same =
			match sizes.windows.get(window) {
				Ok(known) => same_constraints(known, made)
				Err(_) => False
			}
		({ ..sizes, windows: sizes.windows.insert(window, made) }, !same)
	}

	## A window's record from what is observed of it, locked at `locked_width`.
	record : SizeObserved, Bool, ReportedMin, F64 -> WindowConstraints
	record = |observed, respect_min_size, min, locked_width| {
		low =
			match min {
				Min(size) => size
				NoMin => Size.{ width: 0.0, height: 0.0 }
			}
		sizing =
			if !observed.resizable {
				Fixed(low)
			} else if respect_min_size {
				ResizableAtLeastMin(low)
			} else {
				Resizable(low)
			}
		normalized(WindowConstraints.{ sizing, locked_width, locked_height: observed.height, max_width: observed.max_width, max_height: observed.max_height })
	}

	## `c` locked at `width`.
	locked_at : WindowConstraints, F64 -> WindowConstraints
	locked_at = |c, width| WindowConstraints.{ sizing: c.sizing, locked_width: width, locked_height: c.locked_height, max_width: c.max_width, max_height: c.max_height }

	## Platform values are compared by their fields.
	same_constraints : WindowConstraints, WindowConstraints -> Bool
	same_constraints = |a, b|
		same_sizing(a.sizing, b.sizing)
		and a.locked_width == b.locked_width
		and a.locked_height == b.locked_height
		and a.max_width == b.max_width
		and a.max_height == b.max_height

	same_sizing : Sizing, Sizing -> Bool
	same_sizing = |a, b|
		match (a, b) {
			(Fixed(x), Fixed(y)) => same_size(x, y)
			(Resizable(x), Resizable(y)) => same_size(x, y)
			(ResizableAtLeastMin(x), ResizableAtLeastMin(y)) => same_size(x, y)
			_ => False
		}

	same_size : Size, Size -> Bool
	same_size = |a, b| a.width == b.width and a.height == b.height

	## Every window's record, in window order.
	records : Kept -> List(SizeRecord)
	records = |sizes|
		sizes.windows
			.to_list()
			.sort_with(|(a, _), (b, _)| if a < b Before else if a > b After else Same)
			.map(|(window, constraints)| SizeRecord.{ window, constraints })

	## Each app's last reported minimum, in app order.
	minimums : Kept -> List(AppMinimum)
	minimums = |sizes|
		sizes.app_minimums
			.to_list()
			.sort_with(|(a, _), (b, _)| if a < b Before else if a > b After else Same)
			.map(|(pid, min)| AppMinimum.{ pid, min })
}
