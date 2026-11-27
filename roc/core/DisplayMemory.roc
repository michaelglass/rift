## What the layout engine remembers of displays (`rift_core::display_memory`): the display each
## Space shows on, and the Space each display last showed. Spaces and displays are one word each.

import pf.Boundary exposing [DisplayEvent, SpaceDisplay]

## Each Space's display and each display's last Space.
Kept : { display_of : Dict(U64, U64), last_space : Dict(U64, U64) }

DisplayMemory := [].{

	empty : {} -> Kept
	empty = |{}| { display_of: Dict.empty(), last_space: Dict.empty() }

	## Applies `event`; whether either map changed.
	step : Kept, DisplayEvent -> (Kept, Bool)
	step = |kept, event|
		match event {
			Shown(seen) => {
				# Every read before the updates, so each table is updated in place.
				shown = kept.display_of.get(seen.space) != Ok(seen.display)
				followed =
					match seen.last {
						Follow => kept.last_space.get(seen.display) != Ok(seen.space)
						Hold => False
					}
				last_space =
					match seen.last {
						Follow => kept.last_space.insert(seen.display, seen.space)
						Hold => kept.last_space
					}
				({ display_of: kept.display_of.insert(seen.space, seen.display), last_space }, shown or followed)
			}
			Hidden(space) =>
				match kept.display_of.get(space) {
					Ok(_) => ({ ..kept, display_of: kept.display_of.remove(space) }, True)
					Err(_) => (kept, False)
				}
			Remapped(remap) =>
				if remap.from == remap.to {
					(kept, False)
				} else {
					(display_of, moved) =
						match kept.display_of.get(remap.from) {
							Ok(display) => (kept.display_of.remove(remap.from).insert(remap.to, display), True)
							Err(_) => (kept.display_of, False)
						}
					followed = kept.last_space.to_list().any(|(_, space)| space == remap.from)
					last_space = if followed kept.last_space.map(|_, space| if space == remap.from remap.to else space) else kept.last_space
					({ display_of, last_space }, moved or followed)
				}
			Forgotten(space) => {
				shown = kept.display_of.contains(space)
				remembered = kept.last_space.to_list().any(|(_, last)| last == space)
				display_of = if shown kept.display_of.remove(space) else kept.display_of
				last_space = if remembered kept.last_space.keep_if(|(_, last)| last != space) else kept.last_space
				({ display_of, last_space }, shown or remembered)
			}
		}

	ascending : (U64, U64), (U64, U64) -> [Before, Same, After]
	ascending = |(a, _), (b, _)| if a < b Before else if a > b After else Same

	## Every Space's display, in Space order.
	displays : Kept -> List(SpaceDisplay)
	displays = |kept| kept.display_of.to_list().sort_with(ascending).map(|(space, display)| SpaceDisplay.{ space, display })

	## Every display's last Space, in display order.
	last_spaces : Kept -> List(SpaceDisplay)
	last_spaces = |kept| kept.last_space.to_list().sort_with(ascending).map(|(display, space)| SpaceDisplay.{ space, display })
}
