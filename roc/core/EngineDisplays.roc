## The layout engine's display and departure choices (`rift_core::display_memory::choose_display`):
## which workspaces take a display as their home, whether a Space's widths resync, and which moves
## of a reconcile batch are departures. Workspaces and Spaces are one word each.

import pf.Boundary exposing [DisplayQuestion, DisplayChoice]

EngineDisplays := [].{

	choose : DisplayQuestion -> DisplayChoice
	choose = |question|
		match question {
			Rehomed(on_space) =>
				Rehomed(
					on_space.keep_if(|w| match w.homed {
						Homeless => True
						Homed => False
					}).map(|w| w.workspace),
				)
			Resync(q) => if q.shown or q.rehomed > 0 Resync(Resync) else Resync(Unchanged)
			Move(q) =>
				match q.origin {
					Unassigned => Move(Unassigned)
					On(space) => if space == q.target Move(Within) else Move(Departure)
				}
		}
}
