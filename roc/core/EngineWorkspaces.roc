## The layout engine's workspace switching choices (`rift_core::workspaces::choose_switching`):
## what switching to a workspace does, how a workspace navigated to arrives, what a transfer does
## to the Space it leaves, and whether its windows moved across Spaces. Questions on plain data;
## workspaces and Spaces are one word each.

import pf.Boundary exposing [SwitchQuestion, SwitchChoice]

EngineWorkspaces := [].{

	choose : SwitchQuestion -> SwitchChoice
	choose = |question|
		match question {
			Switch(q) => {
				shown =
					match q.shown_here {
						Some(here) => here == q.target
						Nothing => False
					}
				if !shown {
					Switch(Navigate)
				} else if q.back_and_forth {
					Switch(Back)
				} else {
					Switch(Stay)
				}
			}
			Arrival(q) => {
				shown =
					match q.shown_there {
						Some(there) => there == q.target
						Nothing => False
					}
				if shown and q.asked_from != q.its_space {
					Arrival(Focus(q.its_space))
				} else {
					Arrival(Activate)
				}
			}
			Source(reason) =>
				match reason {
					Explicit => Source(Relaid)
					DisplayArrival => Source(Untouched)
					DisplayDeparture => Source(Untouched)
					Restoration => Source(Untouched)
				}
			Moved(q) => if q.source == q.target Moved(InPlace) else Moved(Across)
		}
}
