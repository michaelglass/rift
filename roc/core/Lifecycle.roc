## Decisions about a window's life (`rift_core::lifecycle`): app rules, placement, transients,
## what an inventory says about the windows it omits. Questions on plain data: every fact a
## decision may ask for is in its question; windows, Spaces, window-server ids and apps are one
## word each.

import Wire exposing [finished, put_list, take, take_flag, take_list]

## Words being read, and where.
Words : { words : List(U64), at : U64 }

## A value read and where the rest starts, or the words were malformed.
Taken(a) : Try((a, Words), [Malformed])

## A Space, or none.
MaybeSpace : [Space(U64), NoSpace]

## Why app rules run: windows being admitted, or the rules re-applied.
RuleRun : [Admission, Reapply]

## Where a transient entry's evidence came from.
Origin : [Native, Heuristic]

## What admission reads of a window.
Facts : { manageable : Bool, rule : [NoRule, Manage, Ignore], ax : [Ordinary, DialogWindow, Transient], native_transient : Bool, minimized : Bool }

Lifecycle := [].{

	## The windows app rules run over, by Space in Space order.
	rule_targets : List(U64), RuleRun, List(U64), List({ window : U64, space : U64 }), List(U64) -> List({ space : U64, windows : List(U64) })
	rule_targets = |windows, run, reconcilable, spaces, active| {
		var $groups = []
		for window in windows {
			considered =
				match run {
					Reapply => True
					Admission => reconcilable.contains(window)
				}
			if considered {
				match space_of(spaces, window) {
					Space(space) => {
						$groups = add_to_space($groups, space, window)
					}
					NoSpace => {}
				}
			} else {
				{}
			}
		}
		$groups.keep_if(|group| active.contains(group.space))
	}

	space_of : List({ window : U64, space : U64 }), U64 -> MaybeSpace
	space_of = |spaces, window| {
		var $found = NoSpace
		for entry in spaces {
			match $found {
				NoSpace => if (entry.window == window) { $found = Space(entry.space) } else {}
				Space(_) => {}
			}
		}
		$found
	}

	## Adds `window` to its Space's group, keeping the groups in Space order (a BTreeMap's).
	add_to_space : List({ space : U64, windows : List(U64) }), U64, U64 -> List({ space : U64, windows : List(U64) })
	add_to_space = |groups, space, window| {
		var $out = []
		var $placed = False
		for group in groups {
			if $placed {
				$out = $out.append(group)
			} else if group.space == space {
				$out = $out.append({ space, windows: group.windows.append(window) })
				$placed = True
			} else if group.space > space {
				$out = $out.append({ space, windows: [window] }).append(group)
				$placed = True
			} else {
				$out = $out.append(group)
			}
		}
		if $placed $out else $out.append({ space, windows: [window] })
	}

	## Whether the layout hears of a window after its app rules ran.
	rule_refreshes_layout : RuleRun, { workspace_before : MaybeSpace, workspace : U64, floating_before : Bool, floating : Bool, was_ignored : Bool, unavailable : Bool } -> Bool
	rule_refreshes_layout = |run, outcome|
		match run {
			Reapply => True
			Admission => {
				moved =
					match outcome.workspace_before {
						Space(before) => before != outcome.workspace
						NoSpace => True
					}
				outcome.unavailable or moved or outcome.floating_before != outcome.floating or outcome.was_ignored
			}
		}

	## The Space a new window moves to: in global mode, the pointer's other active display's,
	## for a standard window no app rule places.
	pointer_display_target : Bool, U64, MaybeSpace, List(U64), [Untracked, NotStandard, Standard], Bool -> MaybeSpace
	pointer_display_target = |global, opened_on, pointer, active, window, rule_names_workspace|
		if global {
			match pointer {
				Space(target) =>
					if target != opened_on and active.contains(target) {
						match window {
							Standard => if (rule_names_workspace) NoSpace else Space(target)
							Untracked => NoSpace
							NotStandard => NoSpace
						}
					} else {
						NoSpace
					}
				NoSpace => NoSpace
			}
		} else {
			NoSpace
		}

	## Whether `owner`'s window is a native transient.
	is_native_transient : [Entry({ origin : Origin, pid : U64 }), NoEntry], U64, Bool -> Bool
	is_native_transient = |entry, owner, exempt|
		match entry {
			Entry({ origin: Native, pid }) => pid == owner and !exempt
			Entry({ origin: Heuristic, .. }) => False
			NoEntry => False
		}

	## Whether a transient entry survives a departure.
	keeps_transient : [Destroyed(MaybeSpace), Closed(U64), AppGone(U64)], U64, Origin, U64 -> Bool
	keeps_transient = |departure, id, origin, owner|
		match departure {
			Destroyed(gone) =>
				match origin {
					Native => True
					Heuristic =>
						match gone {
							Space(gone_id) => gone_id != id
							NoSpace => True
						}
				}
			Closed(gone) => gone != id
			AppGone(pid) => pid != owner
		}

	## Whether a window's assignment and its native Space disagree.
	misplaced : MaybeSpace, MaybeSpace -> Bool
	misplaced = |assigned, native|
		match assigned {
			Space(a) =>
				match native {
					Space(n) => a != n
					NoSpace => False
				}
			NoSpace => False
		}

	## Whether an inventory that omits a window says nothing about it.
	out_of_inventory_view : MaybeSpace, MaybeSpace, List(U64), Bool -> Bool
	out_of_inventory_view = |native, assigned, active, parked| {
		known =
			match native {
				Space(space) => Space(space)
				NoSpace => assigned
			}
		inactive =
			match known {
				Space(space) => !active.contains(space)
				NoSpace => False
			}
		inactive or parked
	}

	## Whether a title change re-applies a window's app rules.
	reapplies_on_title_change : Bool, MaybeSpace, List(U64), Bool -> Bool
	reapplies_on_title_change = |configured, space, active, reconcilable|
		if configured {
			match space {
				Space(id) => active.contains(id) and reconcilable
				NoSpace => False
			}
		} else {
			False
		}

	## Whether rift manages a window, and if not the first veto that applies: 0 admitted, or 1
	## and the veto (0 native transient, 1 minimized, 2 AX transient, 3 rule ignores, 4
	## unmanageable).
	admission : Facts -> List(U64)
	admission = |facts|
		match admits(facts) {
			Admitted => [0]
			Vetoed(why) => [1, why]
		}

	## Admitted, or vetoed with the veto's word.
	admits : Facts -> [Admitted, Vetoed(U64)]
	admits = |facts|
		if facts.native_transient {
			Vetoed(0)
		} else if facts.minimized {
			Vetoed(1)
		} else {
			ax_vetoes =
				match facts.ax {
					Transient => True
					DialogWindow =>
						match facts.rule {
							Manage => False
							NoRule => True
							Ignore => True
						}
					Ordinary => False
				}
			if ax_vetoes {
				Vetoed(2)
			} else {
				match facts.rule {
					Ignore => Vetoed(3)
					Manage => Admitted
					NoRule => if (facts.manageable) Admitted else Vetoed(4)
				}
			}
		}

	## A window's facts after a reading (none without AX state), and the change in its
	## admission: 0 none, 1 admitted, 2 suspended (and the veto).
	read : [Known(Facts), NoFacts], [Live(Facts), Changed({ fact : U64, value : U64 }), Gone] -> Try(([Known(Facts), NoFacts], List(U64)), [Malformed])
	read = |before, reading| {
		after_try =
			match reading {
				Live(facts) => Ok(Known(facts))
				Gone => Ok(NoFacts)
				Changed(change) =>
					match before {
						NoFacts => Ok(NoFacts)
						Known(facts) => changed(facts, change).map_ok(|now| Known(now))
					}
			}
		after = after_try?
		change =
			match after {
				NoFacts => [0]
				Known(now) =>
					match admits(now) {
						Admitted =>
							match before {
								Known(was) =>
									match admits(was) {
										Admitted => [0]
										Vetoed(_) => [1]
									}
								NoFacts => [1]
							}
						Vetoed(why) =>
							match before {
								Known(was) =>
									match admits(was) {
										Admitted => [2, why]
										Vetoed(_) => [0]
									}
								NoFacts => [0]
							}
					}
			}
		Ok((after, change))
	}

	changed : Facts, { fact : U64, value : U64 } -> Try(Facts, [Malformed])
	changed = |facts, change|
		match change.fact {
			0 => binary(change.value).map_ok(|unmanageable| { ..facts, manageable: !unmanageable })
			1 => rule_of(change.value).map_ok(|rule| { ..facts, rule })
			2 => ax_of(change.value).map_ok(|ax| { ..facts, ax })
			3 => binary(change.value).map_ok(|native_transient| { ..facts, native_transient })
			4 => binary(change.value).map_ok(|minimized| { ..facts, minimized })
			_ => Err(Malformed)
		}

	binary : U64 -> Try(Bool, [Malformed])
	binary = |word|
		match word {
			0 => Ok(False)
			1 => Ok(True)
			_ => Err(Malformed)
		}

	rule_of : U64 -> Try([NoRule, Manage, Ignore], [Malformed])
	rule_of = |word|
		match word {
			0 => Ok(NoRule)
			1 => Ok(Manage)
			2 => Ok(Ignore)
			_ => Err(Malformed)
		}

	ax_of : U64 -> Try([Ordinary, DialogWindow, Transient], [Malformed])
	ax_of = |word|
		match word {
			0 => Ok(Ordinary)
			1 => Ok(DialogWindow)
			2 => Ok(Transient)
			_ => Err(Malformed)
		}

	take_facts : Words -> Taken(Facts)
	take_facts = |c0| {
		(manageability, c1) = take(c0)?
		(rule_word, c2) = take(c1)?
		(ax_word, c3) = take(c2)?
		(native_word, c4) = take(c3)?
		(minimize_word, c5) = take(c4)?
		unmanageable = binary(manageability)?
		rule = rule_of(rule_word)?
		ax = ax_of(ax_word)?
		native_transient = binary(native_word)?
		minimized = binary(minimize_word)?
		Ok(({ manageable: !unmanageable, rule, ax, native_transient, minimized }, c5))
	}

	put_facts : List(U64), Facts -> List(U64)
	put_facts = |words, facts| {
		rule =
			match facts.rule {
				NoRule => 0
				Manage => 1
				Ignore => 2
			}
		ax =
			match facts.ax {
				Ordinary => 0
				DialogWindow => 1
				Transient => 2
			}
		words
			.append(if (facts.manageable) 0 else 1)
			.append(rule)
			.append(ax)
			.append(if (facts.native_transient) 1 else 0)
			.append(if (facts.minimized) 1 else 0)
	}

	## A window's presence after an event, and what the event came to (a read and its
	## change, or whether it invalidated or revalidated), then whether rift manages it.
	presence_step : [NoPresence, Live(Facts), Invalidated(Facts)], [Reading([Live(Facts), Changed({ fact : U64, value : U64 }), Gone]), Invalidate, Revalidate] -> Try(List(U64), [Malformed])
	presence_step = |before, event| {
		(after, outcome) =
			match event {
				Reading(reading) => {
					known =
						match before {
							NoPresence => NoFacts
							Live(facts) => Known(facts)
							Invalidated(facts) => Known(facts)
						}
					(read_after, change) = read(known, reading)?
					presence =
						match read_after {
							NoFacts => NoPresence
							Known(facts) =>
								match reading {
									Live(_) => Live(facts)
									Changed(_) =>
										match before {
											Invalidated(_) => Invalidated(facts)
											Live(_) => Live(facts)
											NoPresence => Live(facts)
										}
									Gone => Live(facts)
								}
						}
					(presence, [0].concat(change))
				}
				Invalidate =>
					match before {
						NoPresence => (NoPresence, [1, 0])
						Live(facts) => (Invalidated(facts), [1, 1])
						Invalidated(facts) => (Invalidated(facts), [1, 1])
					}
				Revalidate =>
					match before {
						Invalidated(facts) => (Live(facts), [2, 1])
						Live(facts) => (Live(facts), [2, 0])
						NoPresence => (NoPresence, [2, 0])
					}
			}
		manages_it =
			match after {
				Live(facts) =>
					match admits(facts) {
						Admitted => 1
						Vetoed(_) => 0
					}
				Invalidated(_) => 0
				NoPresence => 0
			}
		Ok(put_presence([], after).concat(outcome).append(manages_it))
	}

	put_presence : List(U64), [NoPresence, Live(Facts), Invalidated(Facts)] -> List(U64)
	put_presence = |words, presence|
		match presence {
			NoPresence => words.append(0)
			Live(facts) => put_facts(words.append(1), facts)
			Invalidated(facts) => put_facts(words.append(2), facts)
		}

	take_reading : Words -> Taken([Live(Facts), Changed({ fact : U64, value : U64 }), Gone])
	take_reading = |c0| {
		(kind, c1) = take(c0)?
		match kind {
			0 => take_facts(c1).map_ok(|(facts, rest)| (Live(facts), rest))
			1 => {
				(fact, after) = take(c1)?
				(value, rest) = take(after)?
				Ok((Changed({ fact, value }), rest))
			}
			2 => Ok((Gone, c1))
			_ => Err(Malformed)
		}
	}

	## Tag 26.
	presence_words : Words -> Try(List(U64), [Malformed])
	presence_words = |c0| {
		(presence_kind, c1) = take(c0)?
		(before, c2) =
			match presence_kind {
				0 => Ok((NoPresence, c1))
				1 => take_facts(c1).map_ok(|(facts, rest)| (Live(facts), rest))
				2 => take_facts(c1).map_ok(|(facts, rest)| (Invalidated(facts), rest))
				_ => Err(Malformed)
			}?
		(event_kind, c3) = take(c2)?
		(event, c4) =
			match event_kind {
				0 => take_reading(c3).map_ok(|(reading, rest)| (Reading(reading), rest))
				1 => Ok((Invalidate, c3))
				2 => Ok((Revalidate, c3))
				_ => Err(Malformed)
			}?
		finished(c4)?
		presence_step(before, event)
	}

	## Whether an inventory retires a window: 0 retire, or 1 and why it keeps it.
	retirement : { reported : Bool, out_of_view : Bool, minimize : [Shown, Minimized, NoAxState], fullscreen : Bool, identified : Bool, invalidated : Bool, suitable : [Yes, No, Unknown], ordered_in : [Yes, No, Unknown], not_a_window : [Yes, No, Unknown] } -> List(U64)
	retirement = |facts|
		if facts.reported {
			[1, 0]
		} else if facts.out_of_view {
			[1, 1]
		} else {
			match facts.minimize {
				NoAxState => [1, 2]
				Minimized => [1, 3]
				Shown =>
					if facts.fullscreen {
						[1, 4]
					} else if !facts.identified {
						[1, 5]
					} else {
						suitable = facts.suitable == Yes
						unsuitable = facts.suitable == No
						ordered_out = facts.ordered_in == No
						if suitable and ordered_out and !facts.invalidated {
							[1, 6]
						} else if unsuitable or ordered_out or facts.not_a_window == Yes {
							[0]
						} else {
							[1, 7]
						}
					}
			}
		}

	take_tri : Words -> Taken([Yes, No, Unknown])
	take_tri = |c0| {
		(known, c1) = take_flag(c0)?
		if known {
			(word, c2) = take(c1)?
			match word {
				0 => Ok((Yes, c2))
				1 => Ok((No, c2))
				_ => Err(Malformed)
			}
		} else {
			Ok((Unknown, c1))
		}
	}

	## Tag 27. (Suitability and the window look are written 0 for yes; ordering 0 for in.)
	inventory_words : Words -> Try(List(U64), [Malformed])
	inventory_words = |c0| {
		(report, c1) = take(c0)?
		(view, c2) = take(c1)?
		(has_minimize, c3) = take_flag(c2)?
		(minimize, c4) =
			if has_minimize {
				(word, rest) = take(c3)?
				minimized = binary(word)?
				(if (minimized) Minimized else Shown, rest)
			} else {
				(NoAxState, c3)
			}
		(fullscreen_word, c5) = take(c4)?
		(has_identity, after_flag) = take_flag(c5)?
		(identified, c6) =
			if has_identity {
				(_, rest) = take(after_flag)?
				(True, rest)
			} else {
				(False, after_flag)
			}
		(validity, c7) = take(c6)?
		(suitable, c8) = take_tri(c7)?
		(ordered_in, c9) = take_tri(c8)?
		(look, c10) = take_tri(c9)?
		finished(c10)?
		reported = !(binary(report)?)
		out_of_view = binary(view)?
		fullscreen = !(binary(fullscreen_word)?)
		invalidated = binary(validity)?
		Ok(retirement({ reported, out_of_view, minimize, fullscreen, identified, invalidated, suitable, ordered_in, not_a_window: invert(look) }))
	}

	## A window look read as "a window?" (0 yes) becomes "not a window?".
	invert : [Yes, No, Unknown] -> [Yes, No, Unknown]
	invert = |answer|
		match answer {
			Yes => No
			No => Yes
			Unknown => Unknown
		}

	## Two admission changes in a row as one: from the first's start to the second's end.
	net : [NoChange, Admitted, Suspended(U64)], [NoChange, Admitted, Suspended(U64)] -> [NoChange, Admitted, Suspended(U64)]
	net = |first, later|
		match (first, later) {
			(NoChange, change) => change
			(change, NoChange) => change
			(Admitted, Suspended(_)) => NoChange
			(Suspended(_), Admitted) => NoChange
			(Admitted, Admitted) => Admitted
			(Suspended(_), Suspended(why)) => Suspended(why)
		}

	take_change : Words -> Taken([NoChange, Admitted, Suspended(U64)])
	take_change = |c0| {
		(kind, c1) = take(c0)?
		match kind {
			0 => Ok((NoChange, c1))
			1 => Ok((Admitted, c1))
			2 => take(c1).map_ok(|(why, rest)| (Suspended(why), rest))
			_ => Err(Malformed)
		}
	}

	## Tag 28.
	net_words : Words -> Try(List(U64), [Malformed])
	net_words = |c0| {
		(first, c1) = take_change(c0)?
		(later, c2) = take_change(c1)?
		finished(c2)?
		Ok(
			match net(first, later) {
				NoChange => [0]
				Admitted => [1]
				Suspended(why) => [2, why]
			},
		)
	}

	## Tag 25.
	reading_words : Words -> Try(List(U64), [Malformed])
	reading_words = |c0| {
		(has_before, c1) = take_flag(c0)?
		(before, c2) =
			if has_before {
				(facts, rest) = take_facts(c1)?
				(Known(facts), rest)
			} else {
				(NoFacts, c1)
			}
		(kind, c3) = take(c2)?
		(reading, c4) =
			match kind {
				0 => take_facts(c3).map_ok(|(facts, rest)| (Live(facts), rest))
				1 => {
					(fact, after) = take(c3)?
					(value, rest) = take(after)?
					Ok((Changed({ fact, value }), rest))
				}
				2 => Ok((Gone, c3))
				_ => Err(Malformed)
			}?
		finished(c4)?
		(after, change) = read(before, reading)?
		words =
			match after {
				Known(facts) => put_facts([1], facts)
				NoFacts => [0]
			}
		Ok(words.concat(change))
	}

	## Tag 24.
	admission_words : Words -> Try(List(U64), [Malformed])
	admission_words = |c0| {
		(manageability, c1) = take(c0)?
		(rule_word, c2) = take(c1)?
		(ax_word, c3) = take(c2)?
		(native_word, c4) = take(c3)?
		(minimize_word, c5) = take(c4)?
		finished(c5)?
		manageable =
			match manageability {
				0 => Ok(True)
				1 => Ok(False)
				_ => Err(Malformed)
			}?
		rule =
			match rule_word {
				0 => Ok(NoRule)
				1 => Ok(Manage)
				2 => Ok(Ignore)
				_ => Err(Malformed)
			}?
		ax =
			match ax_word {
				0 => Ok(Ordinary)
				1 => Ok(DialogWindow)
				2 => Ok(Transient)
				_ => Err(Malformed)
			}?
		native_transient =
			match native_word {
				0 => Ok(False)
				1 => Ok(True)
				_ => Err(Malformed)
			}?
		minimized =
			match minimize_word {
				0 => Ok(False)
				1 => Ok(True)
				_ => Err(Malformed)
			}?
		Ok(admission({ manageable, rule, ax, native_transient, minimized }))
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

	put_verdict : Bool -> List(U64)
	put_verdict = |verdict| if (verdict) [1] else [0]

	take_run : Words -> Taken(RuleRun)
	take_run = |c0| {
		(word, c1) = take(c0)?
		match word {
			0 => Ok((Admission, c1))
			1 => Ok((Reapply, c1))
			_ => Err(Malformed)
		}
	}

	take_origin : Words -> Taken(Origin)
	take_origin = |c0| {
		(word, c1) = take(c0)?
		match word {
			0 => Ok((Native, c1))
			1 => Ok((Heuristic, c1))
			_ => Err(Malformed)
		}
	}

	take_binary : Words -> Taken(Bool)
	take_binary = |c0| {
		(word, c1) = take(c0)?
		match word {
			0 => Ok((False, c1))
			1 => Ok((True, c1))
			_ => Err(Malformed)
		}
	}

	## Tag 9.
	rule_targets_words : Words -> Try(List(U64), [Malformed])
	rule_targets_words = |c0| {
		take_pair = |c| {
			(window, after) = take(c)?
			(space, rest) = take(after)?
			Ok(({ window, space }, rest))
		}
		(windows, c1) = take_list(c0, take)?
		(run, c2) = take_run(c1)?
		(reconcilable, c3) = take_list(c2, take)?
		(spaces, c4) = take_list(c3, take_pair)?
		(active, c5) = take_list(c4, take)?
		finished(c5)?
		Ok(put_list([], rule_targets(windows, run, reconcilable, spaces, active), |w, group| put_list(w.append(group.space), group.windows, |ws, id| ws.append(id))))
	}

	## Tag 10.
	rule_refresh_words : Words -> Try(List(U64), [Malformed])
	rule_refresh_words = |c0| {
		(run, c1) = take_run(c0)?
		(workspace_before, c2) = take_space(c1)?
		(workspace, c3) = take(c2)?
		(floating_before, c4) = take_flag(c3)?
		(floating, c5) = take_flag(c4)?
		(was_ignored, c6) = take_flag(c5)?
		(unavailable, c7) = take_flag(c6)?
		finished(c7)?
		Ok(put_verdict(rule_refreshes_layout(run, { workspace_before, workspace, floating_before, floating, was_ignored, unavailable })))
	}

	## Tag 11.
	pointer_target_words : Words -> Try(List(U64), [Malformed])
	pointer_target_words = |c0| {
		(global, c1) = take_binary(c0)?
		(opened_on, c2) = take(c1)?
		(pointer, c3) = take_space(c2)?
		(active, c4) = take_list(c3, take)?
		(window_word, c5) = take(c4)?
		window =
			match window_word {
				0 => Ok(Untracked)
				1 => Ok(NotStandard)
				2 => Ok(Standard)
				_ => Err(Malformed)
			}?
		(rule_names_workspace, c6) = take_flag(c5)?
		finished(c6)?
		Ok(put_space([], pointer_display_target(global, opened_on, pointer, active, window, rule_names_workspace)))
	}

	## Tag 12.
	native_transient_words : Words -> Try(List(U64), [Malformed])
	native_transient_words = |c0| {
		(has_entry, c1) = take_flag(c0)?
		(entry, c2) =
			if has_entry {
				(origin, after) = take_origin(c1)?
				(pid, rest) = take(after)?
				(Entry({ origin, pid }), rest)
			} else {
				(NoEntry, c1)
			}
		(owner, c3) = take(c2)?
		(exempt, c4) = take_flag(c3)?
		finished(c4)?
		Ok(put_verdict(is_native_transient(entry, owner, exempt)))
	}

	## Tag 13.
	keeps_transient_words : Words -> Try(List(U64), [Malformed])
	keeps_transient_words = |c0| {
		(kind, c1) = take(c0)?
		(departure, c2) =
			match kind {
				0 => take_space(c1).map_ok(|(gone, rest)| (Destroyed(gone), rest))
				1 => take(c1).map_ok(|(gone, rest)| (Closed(gone), rest))
				2 => take(c1).map_ok(|(pid, rest)| (AppGone(pid), rest))
				_ => Err(Malformed)
			}?
		(id, c3) = take(c2)?
		(origin, c4) = take_origin(c3)?
		(owner, c5) = take(c4)?
		finished(c5)?
		Ok(put_verdict(keeps_transient(departure, id, origin, owner)))
	}

	## Tag 14.
	misplaced_words : Words -> Try(List(U64), [Malformed])
	misplaced_words = |c0| {
		(assigned, c1) = take_space(c0)?
		(native, c2) = take_space(c1)?
		finished(c2)?
		Ok(put_verdict(misplaced(assigned, native)))
	}

	## Tag 15.
	out_of_view_words : Words -> Try(List(U64), [Malformed])
	out_of_view_words = |c0| {
		(native, c1) = take_space(c0)?
		(assigned, c2) = take_space(c1)?
		(active, c3) = take_list(c2, take)?
		(parked, c4) = take_flag(c3)?
		finished(c4)?
		Ok(put_verdict(out_of_inventory_view(native, assigned, active, parked)))
	}

	## Tag 16.
	title_change_words : Words -> Try(List(U64), [Malformed])
	title_change_words = |c0| {
		(configured, c1) = take_binary(c0)?
		(space, c2) = take_space(c1)?
		(active, c3) = take_list(c2, take)?
		(reconcilable, c4) = take_flag(c3)?
		finished(c4)?
		Ok(put_verdict(reapplies_on_title_change(configured, space, active, reconcilable)))
	}
}
