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
