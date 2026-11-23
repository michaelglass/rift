## Decisions about Spaces and the windows on them (`rift_core::topology`), as questions on plain
## data: a decision's facts are what the platform's run of it asked (`Asking.Fact`), read in the
## same order. Spaces, window-server ids and displays are one word each.

import Asking exposing [ask, ask_window, ask_word]

WindowId : { pid : I32, idx : U32 }

## A Space, or none.
MaybeSpace : [Space(U64), NoSpace]

## A point on the screens, or none.
Origin : [At({ x : F64, y : F64 }), Nowhere]

## Where an app inventory finds a window, by what the platform already knows of it.
Standing : [RecoveringFullscreen, Untracked, Tracked]

## The Space an app inventory takes a window to be on: live on exactly one Space, a
## fullscreen one or a display's (also evidence of a fullscreen entry or exit); live on exactly
## one other Space; or not live on exactly one, and what the platform resolves for it.
DiscoveredSpace : [Observed(U64), Live(U64), Fallback(MaybeSpace)]

## What an app inventory may do with a window it reports, for its discovery Space.
DiscoveryStanding : [FullscreenRecorded, Placeholder, Admissible]

## The displays of a snapshot: each screen's display (none without an id), each display's
## Spaces in the platform's order.
Displays : { screens : List([Display(U64), Unnamed]), spaces : List({ display : U64, spaces : List(U64) }) }

## A snapshot being admitted.
Incoming : {
	displays : Displays,
	remaps : List({ old : U64, new : U64 }),
	display_set_changed : Bool,
	force_refresh : Bool,
	membership_complete : Bool,
}

## A fact the platform's run of the decision answered, or one it never asked.
Fact(a) : [Asked(a), Unasked]

## A fact asked about keys: each key asked and its answer.
Answers(k, v) : List({ key : k, answer : v })

## What a decision decided, or the first fact it read that its question does not carry.
Read(a) : Try(a, [Unasked(U64)])

Admission : [Hold([MissionControl, MembershipIncomplete]), Commit({ invalidated : List(U64), lost : List(U64) })]

Topology := [].{

	## A tracked window resolves as usual. An untracked or recovering one is where it is live,
	## if that is exactly one Space: several memberships are transitional. Failing that, an
	## untracked window falls back to its recorded Space and a recovering one to none. Facts: 0
	## its live Spaces, 1 whether a Space is observable, 2 its recorded Space, 3 its resolved
	## Space.
	discovered_space : Standing, Fact(List(U64)), Answers(U64, Bool), Fact(MaybeSpace), Fact(MaybeSpace) -> Read(DiscoveredSpace)
	discovered_space = |standing, live, observable, recorded, resolved| {
		one_live = |{}| {
			spaces = ask(live, 0)?
			match spaces {
				[space] => {
					seen = ask_word(observable, space, 1)?
					if seen Ok(Observed(space)) else Ok(Live(space))
				}
				_ => Ok(Fallback(NoSpace))
			}
		}
		match standing {
			Tracked => {
				space = ask(resolved, 3)?
				Ok(Fallback(space))
			}
			Untracked => {
				found = one_live({})?
				match found {
					Fallback(_) => {
						space = ask(recorded, 2)?
						Ok(Fallback(space))
					}
					Observed(space) => Ok(Observed(space))
					Live(space) => Ok(Live(space))
				}
			}
			RecoveringFullscreen => one_live({})
		}
	}

	## A reported window's authoritative Space (native, else assigned) and the Space discovery
	## may admit it on: only an admissible window's, preferring its assignment, then its native
	## Space, then where its frame is. Fact: 0 where its frame is.
	discovery_spaces : DiscoveryStanding, MaybeSpace, MaybeSpace, Fact(MaybeSpace) -> Read((MaybeSpace, MaybeSpace))
	discovery_spaces = |standing, native, assigned, by_frame| {
		authoritative = or_space(native, assigned)
		match standing {
			FullscreenRecorded => Ok((authoritative, NoSpace))
			Placeholder => Ok((authoritative, NoSpace))
			Admissible =>
				match or_space(assigned, authoritative) {
					Space(space) => Ok((authoritative, Space(space)))
					NoSpace => {
						framed = ask(by_frame, 0)?
						Ok((authoritative, framed))
					}
				}
		}
	}

	## Whether to quarantine an event: while the topology is unsettled, window reports and the
	## input that acts on windows are held back. Fact: 0 the event's class (asked only while
	## unsettled).
	quarantines : [Settled, Unsettled], Fact([WindowReport, WindowInput, Other]) -> Read(Bool)
	quarantines = |topology, class|
		match topology {
			Settled => Ok(False)
			Unsettled => {
				event = ask(class, 0)?
				match event {
					WindowReport => Ok(True)
					WindowInput => Ok(True)
					Other => Ok(False)
				}
			}
		}

	## The display a selector names, and its active Space. Facts: 0 the current display's
	## center (asked only without the source's), 1 the display found from an origin and its
	## Space, 2 whether a Space is active (asked only for a display with a Space).
	target_display : Origin, Fact(Origin), List({ key : Origin, answer : [Found({ display : U64, space : [Some(U64), Nothing] }), NotFound] }), Answers(U64, Bool) -> Read([Display({ display : U64, space : U64 }), DisplayNotFound, SpaceUnavailable])
	target_display = |source_center, current_center, screens, active| {
		origin =
			match source_center {
				At(center) => At(center)
				Nowhere => ask(current_center, 0)?
			}
		found = ask_origin(screens, origin, 1)?
		match found {
			NotFound => Ok(DisplayNotFound)
			Found(screen) =>
				match screen.space {
					Nothing => Ok(SpaceUnavailable)
					Some(space) => {
						is_active = ask_word(active, space, 2)?
						if is_active Ok(Display({ display: screen.display, space })) else Ok(SpaceUnavailable)
					}
				}
		}
	}

	## The `fact`-th fact's answer for an origin.
	ask_origin : List({ key : Origin, answer : v }), Origin, U64 -> Read(v)
	ask_origin = |answers, key, fact| {
		var $found = Err(Unasked(fact))
		for entry in answers {
			match $found {
				Err(Unasked(_)) => if (same_origin(entry.key, key)) { $found = Ok(entry.answer) } else {}
				Ok(_) => {}
			}
		}
		$found
	}

	## Whether two origins are the same point (or both none).
	same_origin : Origin, Origin -> Bool
	same_origin = |a, b|
		match a {
			At(p) =>
				match b {
					At(q) => p.x == q.x and p.y == q.y
					Nowhere => False
				}
			Nowhere =>
				match b {
					At(_) => False
					Nowhere => True
				}
		}

	or_space : MaybeSpace, MaybeSpace -> MaybeSpace
	or_space = |first, second|
		match first {
			Space(space) => Space(space)
			NoSpace => second
		}

	## The Spaces `display` has in `displays`, if it is there.
	spaces_on : Displays, U64 -> [On(List(U64)), Absent]
	spaces_on = |displays, display| {
		var $found = Absent
		for entry in displays.spaces {
			match $found {
				Absent => if (entry.display == display) { $found = On(entry.spaces) } else {}
				On(_) => {}
			}
		}
		$found
	}

	## Admits `incoming` over the `current` displays. Only a display change or a forced refresh
	## invalidates Spaces; a remapped Space is not lost.
	admit : Bool, Displays, Incoming -> Admission
	admit = |mission_control_open, current, incoming| {
		if mission_control_open {
			Hold(MissionControl)
		} else {
			sole =
				match current.screens {
					[previous] =>
						match incoming.displays.screens {
							[now] => if (previous == now) now else Unnamed
							_ => Unnamed
						}
					_ => Unnamed
				}
			var $invalidated = []
			var $lost = []
			if incoming.display_set_changed or incoming.force_refresh {
				for entry in current.spaces {
					now_spaces = spaces_on(incoming.displays, entry.display)
					for space in entry.spaces {
						still_there =
							match now_spaces {
								On(ids) => ids.contains(space)
								Absent => False
							}
						remapped = incoming.remaps.any(|remap| remap.old == space)
						if still_there or remapped {
							{}
						} else {
							$invalidated = $invalidated.append(space)
							on_sole =
								match sole {
									Display(display) => display == entry.display
									Unnamed => False
								}
							anywhere = incoming.displays.spaces.any(|other| other.spaces.contains(space))
							if on_sole and !anywhere {
								$lost = $lost.append(space)
							} else {
								{}
							}
						}
					}
				}
			} else {
				{}
			}
			if !incoming.membership_complete and !$invalidated.is_empty() {
				Hold(MembershipIncomplete)
			} else {
				Commit({ invalidated: $invalidated, lost: $lost })
			}
		}
	}

	## The windows whose app rules an activation re-applies, by app in the order its first
	## window is seen: the tracked windows live on an activated Space whose admission can still
	## be reconciled. Facts: 0 the window a window-server id is tracked as, 1 whether a
	## window's admission can be reconciled.
	candidates : List({ server : U64, space : U64 }), List(U64), Answers(U64, [Tracked(WindowId), NotTracked]), Answers(WindowId, Bool) -> Read(List({ pid : I32, windows : List(WindowId) }))
	candidates = |window_spaces, activated, tracked, reconcilable| {
		var $by_app = Ok([])
		for entry in window_spaces {
			match $by_app {
				Ok(by_app) =>
					if activated.contains(entry.space) {
						$by_app = add_candidate(by_app, entry.server, tracked, reconcilable)
					} else {
						{}
					}
				Err(_) => {}
			}
		}
		$by_app
	}

	## Adds the window a window-server id is tracked as, if its admission can be reconciled.
	add_candidate : List({ pid : I32, windows : List(WindowId) }), U64, Answers(U64, [Tracked(WindowId), NotTracked]), Answers(WindowId, Bool) -> Read(List({ pid : I32, windows : List(WindowId) }))
	add_candidate = |by_app, server, tracked, reconcilable| {
		window_of = ask_word(tracked, server, 0)?
		match window_of {
			Tracked(window) => {
				open = ask_window(reconcilable, window, 1)?
				if open Ok(add_to_app(by_app, window)) else Ok(by_app)
			}
			NotTracked => Ok(by_app)
		}
	}

	add_to_app : List({ pid : I32, windows : List(WindowId) }), WindowId -> List({ pid : I32, windows : List(WindowId) })
	add_to_app = |by_app, window| {
		var $out = []
		var $added = False
		for group in by_app {
			if group.pid == window.pid {
				$out = $out.append({ ..group, windows: group.windows.append(window) })
				$added = True
			} else {
				$out = $out.append(group)
			}
		}
		if $added $out else $out.append({ pid: window.pid, windows: [window] })
	}
}
