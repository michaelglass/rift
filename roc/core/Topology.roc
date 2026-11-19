## Decisions about Spaces and the windows on them (`rift_core::topology`), as questions on plain
## data: every fact a decision may ask for is in its question. Spaces, window-server ids and
## displays are one word each.

import Wire exposing [finished, put_list, put_window, take, take_flag, take_list, take_window]

WindowId : { pid : I32, idx : U32 }

## Words being read, and where.
Words : { words : List(U64), at : U64 }

## A value read and where the rest starts, or the words were malformed.
Taken(a) : Try((a, Words), [Malformed])

## A Space, or none.
MaybeSpace : [Space(U64), NoSpace]

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

Admission : [Hold([MissionControl, MembershipIncomplete]), Commit({ invalidated : List(U64), lost : List(U64) })]

Topology := [].{

	## A tracked window resolves as usual. An untracked or recovering one is where it is live,
	## if that is exactly one Space: several memberships are transitional. Failing that, an
	## untracked window falls back to its recorded Space and a recovering one to none.
	discovered_space : Standing, List(U64), List(U64), MaybeSpace, MaybeSpace -> DiscoveredSpace
	discovered_space = |standing, live, observable, recorded, resolved| {
		one_live =
			match live {
				[space] => if (observable.contains(space)) Observed(space) else Live(space)
				_ => Fallback(NoSpace)
			}
		match standing {
			Tracked => Fallback(resolved)
			Untracked =>
				match one_live {
					Fallback(_) => Fallback(recorded)
					found => found
				}
			RecoveringFullscreen => one_live
		}
	}

	## A reported window's authoritative Space (native, else assigned) and the Space discovery
	## may admit it on: only an admissible window's, preferring its assignment, then its native
	## Space, then where its frame is.
	discovery_spaces : DiscoveryStanding, MaybeSpace, MaybeSpace, MaybeSpace -> (MaybeSpace, MaybeSpace)
	discovery_spaces = |standing, native, assigned, by_frame| {
		authoritative = or_space(native, assigned)
		match standing {
			FullscreenRecorded => (authoritative, NoSpace)
			Placeholder => (authoritative, NoSpace)
			Admissible => (authoritative, or_space(or_space(assigned, authoritative), by_frame))
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
	## be reconciled.
	candidates : List({ server : U64, space : U64 }), List(U64), List({ server : U64, window : WindowId }), List(WindowId) -> List({ pid : I32, windows : List(WindowId) })
	candidates = |window_spaces, activated, tracked, reconcilable| {
		var $by_app = []
		for entry in window_spaces {
			if activated.contains(entry.space) {
				match tracked_window(tracked, entry.server) {
					Tracked(window) =>
						if reconcilable.contains(window) {
							$by_app = add_to_app($by_app, window)
						} else {
							{}
						}
					NotTracked => {}
				}
			} else {
				{}
			}
		}
		$by_app
	}

	tracked_window : List({ server : U64, window : WindowId }), U64 -> [Tracked(WindowId), NotTracked]
	tracked_window = |tracked, server| {
		var $found = NotTracked
		for entry in tracked {
			match $found {
				NotTracked => if (entry.server == server) { $found = Tracked(entry.window) } else {}
				Tracked(_) => {}
			}
		}
		$found
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

	# The wire.

	take_space : Words -> Taken(MaybeSpace)
	take_space = |c0| {
		(present, c1) = take_flag(c0)?
		if present {
			(space, c2) = take(c1)?
			Ok((Space(space), c2))
		} else {
			Ok((NoSpace, c1))
		}
	}

	put_space : List(U64), MaybeSpace -> List(U64)
	put_space = |words, space|
		match space {
			Space(id) => words.append(1).append(id)
			NoSpace => words.append(0)
		}

	take_displays : Words -> Taken(Displays)
	take_displays = |c0| {
		take_screen = |c| {
			(named, after) = take_flag(c)?
			if named {
				(display, rest) = take(after)?
				Ok((Display(display), rest))
			} else {
				Ok((Unnamed, after))
			}
		}
		take_entry = |c| {
			(display, after) = take(c)?
			(spaces, rest) = take_list(after, take)?
			Ok(({ display, spaces }, rest))
		}
		(screens, c1) = take_list(c0, take_screen)?
		(spaces, c2) = take_list(c1, take_entry)?
		Ok(({ screens, spaces }, c2))
	}

	## Tag 5: where an app inventory finds a window.
	discovered_words : Words -> Try(List(U64), [Malformed])
	discovered_words = |c0| {
		(standing_word, c1) = take(c0)?
		standing =
			match standing_word {
				0 => Ok(RecoveringFullscreen)
				1 => Ok(Untracked)
				2 => Ok(Tracked)
				_ => Err(Malformed)
			}?
		(live, c2) = take_list(c1, take)?
		(observable, c3) = take_list(c2, take)?
		(recorded, c4) = take_space(c3)?
		(resolved, c5) = take_space(c4)?
		finished(c5)?
		Ok(
			match discovered_space(standing, live, observable, recorded, resolved) {
				Observed(space) => [0, space]
				Live(space) => [1, space]
				Fallback(space) => put_space([2], space)
			},
		)
	}

	## Tag 6: a reported window's authoritative and discovery Spaces.
	discovery_words : Words -> Try(List(U64), [Malformed])
	discovery_words = |c0| {
		(standing_word, c1) = take(c0)?
		standing =
			match standing_word {
				0 => Ok(FullscreenRecorded)
				1 => Ok(Placeholder)
				2 => Ok(Admissible)
				_ => Err(Malformed)
			}?
		(native, c2) = take_space(c1)?
		(assigned, c3) = take_space(c2)?
		(by_frame, c4) = take_space(c3)?
		finished(c4)?
		(authoritative, discovery) = discovery_spaces(standing, native, assigned, by_frame)
		Ok(put_space(put_space([], authoritative), discovery))
	}

	## Tag 7: a space snapshot's admission.
	admit_words : Words -> Try(List(U64), [Malformed])
	admit_words = |c0| {
		(mission_control_word, c1) = take(c0)?
		mission_control_open =
			match mission_control_word {
				0 => Ok(False)
				1 => Ok(True)
				_ => Err(Malformed)
			}?
		(current, c2) = take_displays(c1)?
		(displays, c3) = take_displays(c2)?
		take_remap = |c| {
			(old, after) = take(c)?
			(new, rest) = take(after)?
			Ok(({ old, new }, rest))
		}
		(remaps, c4) = take_list(c3, take_remap)?
		(display_set_changed, c5) = take_flag(c4)?
		(force_refresh, c6) = take_flag(c5)?
		(membership_complete, c7) = take_flag(c6)?
		finished(c7)?
		incoming = { displays, remaps, display_set_changed, force_refresh, membership_complete }
		Ok(
			match admit(mission_control_open, current, incoming) {
				Hold(MissionControl) => [0, 0]
				Hold(MembershipIncomplete) => [0, 1]
				Commit({ invalidated, lost }) => put_list(put_list([1], invalidated, |w, s| w.append(s)), lost, |w, s| w.append(s))
			},
		)
	}

	## Tag 8: the windows an activation re-applies app rules to, by app.
	candidates_words : Words -> Try(List(U64), [Malformed])
	candidates_words = |c0| {
		take_pair = |c| {
			(server, after) = take(c)?
			(space, rest) = take(after)?
			Ok(({ server, space }, rest))
		}
		take_tracked = |c| {
			(server, after) = take(c)?
			(window, rest) = take_window(after)?
			Ok(({ server, window }, rest))
		}
		(window_spaces, c1) = take_list(c0, take_pair)?
		(activated, c2) = take_list(c1, take)?
		(tracked, c3) = take_list(c2, take_tracked)?
		(reconcilable, c4) = take_list(c3, take_window)?
		finished(c4)?
		Ok(
			put_list(
				[],
				candidates(window_spaces, activated, tracked, reconcilable),
				|w, group| put_list(w.append(group.pid.to_i64().to_u64_wrap()), group.windows, put_window),
			),
		)
	}
}
