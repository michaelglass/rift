## Decisions about a window's life (`rift_core::lifecycle`): app rules, placement, transients,
## what an inventory says about the windows it omits. Questions on plain data: a decision's
## facts are what the platform's run of it asked (`Asking.Fact`), read in the same order;
## windows, Spaces, window-server ids and apps are one word each.

import Asking exposing [ask, ask_word]

## A Space, or none.
MaybeSpace : [Space(U64), NoSpace]

## Why app rules run: windows being admitted, or the rules re-applied.
RuleRun : [Admission, Reapply]

## Where a transient entry's evidence came from.
Origin : [Native, Heuristic]

## A fact the platform's run of the decision answered, or one it never asked.
Fact(a) : [Asked(a), Unasked]

## A fact asked about keys: each key asked and its answer.
Answers(k, v) : List({ key : k, answer : v })

## What a decision decided, or the first fact it read that its question does not carry.
Read(a) : Try(a, [Unasked(U64)])

## What admission reads of a window (the platform's `Boundary.AdmissionFacts`).
Facts : { manageability : [Manageable, Unmanageable], rule : [NoRule, Manage, Ignore], ax : [Ordinary, DialogWindow, Transient], native : [Ordinary, Transient], minimize : [Shown, Minimized] }

## Why a window is not admitted.
Veto : [NativeTransient, Minimized, AxTransient, RuleIgnores, Unmanageable]

## One admission fact, changed.
FactChange : [Manageability([Manageable, Unmanageable]), Rule([NoRule, Manage, Ignore]), Ax([Ordinary, DialogWindow, Transient]), Native([Ordinary, Transient]), Minimize([Shown, Minimized])]

## A reading of a window: its facts afresh, one fact changed, or its AX state gone.
Reading : [Live(Facts), Changed(FactChange), Gone]

## A change in whether rift manages a window.
Change : [NoChange, Admitted, Suspended(Veto)]

## A window's presence: none, live, or kept with its AX element invalidated.
Presence : [NoPresence, Live(Facts), Invalidated(Facts)]

## A window with AX state: valid, or invalidated (kept, with its facts, but not managed).
Present : [Live(Facts), Invalidated(Facts)]

## What an inventory knows of a window it is reconciled against.
InventoryFacts : { report : [Reported, Omitted], view : [InView, OutOfView], minimize : [Shown, Minimized, NoAxState], fullscreen : [Recorded, NotRecorded], identity : [Known(U64), Unknown], validity : [Valid, Invalidated] }

## What the window server says of an omitted window.
NativeEvidence : { suitability : [Suitable, Unsuitable, SuitabilityUnknown], ordering : [OrderedIn, OrderedOut, OrderingUnknown], look : [AWindow, NotAWindow, LookUnknown] }

## Why an inventory leaves a window alone.
Keep : [Reported, OutOfView, NoAxState, Minimized, Fullscreen, NoNativeIdentity, NativeOrdering, NoNegativeEvidence]

Lifecycle := [].{

	## The windows app rules run over, by Space in Space order. Facts: 0 whether a window is
	## reconcilable, 1 its Space, 2 whether a Space is active (asked once every window is placed,
	## in Space order).
	rule_targets : List(U64), RuleRun, Answers(U64, Bool), Answers(U64, MaybeSpace), Answers(U64, Bool) -> Read(List({ space : U64, windows : List(U64) }))
	rule_targets = |windows, run, reconcilable, spaces, active| {
		var $groups = Ok([])
		for window in windows {
			match $groups {
				Ok(groups) => {
					$groups = add_target(groups, window, run, reconcilable, spaces)
				}
				Err(_) => {}
			}
		}
		groups = $groups?
		var $kept = Ok([])
		for group in groups {
			match $kept {
				Ok(kept) =>
					match ask_word(active, group.space, 2) {
						Ok(True) => {
							$kept = Ok(kept.append(group))
						}
						Ok(False) => {}
						Err(unasked) => {
							$kept = Err(unasked)
						}
					}
				Err(_) => {}
			}
		}
		$kept
	}

	## Adds `window` to its Space's group, if the run considers it and it has a Space.
	add_target : List({ space : U64, windows : List(U64) }), U64, RuleRun, Answers(U64, Bool), Answers(U64, MaybeSpace) -> Read(List({ space : U64, windows : List(U64) }))
	add_target = |groups, window, run, reconcilable, spaces| {
		considered =
			match run {
				Reapply => Ok(True)
				Admission => ask_word(reconcilable, window, 0)
			}?
		if considered {
			space_of = ask_word(spaces, window, 1)?
			match space_of {
				Space(space) => Ok(add_to_space(groups, space, window))
				NoSpace => Ok(groups)
			}
		} else {
			Ok(groups)
		}
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
	## for a standard window no app rule places. Facts: 0 the pointer's Space, 1 whether a Space
	## is active, 2 whether a rule names the window's workspace.
	pointer_display_target : Bool, U64, Fact(MaybeSpace), Answers(U64, Bool), [Untracked, NotStandard, Standard], Fact(Bool) -> Read(MaybeSpace)
	pointer_display_target = |global, opened_on, pointer, active, window, rule_names_workspace|
		if global {
			pointer_of = ask(pointer, 0)?
			match pointer_of {
				Space(target) =>
					if target != opened_on {
						is_active = ask_word(active, target, 1)?
						if is_active {
							match window {
								Standard => {
									names = ask(rule_names_workspace, 2)?
									if names Ok(NoSpace) else Ok(Space(target))
								}
								Untracked => Ok(NoSpace)
								NotStandard => Ok(NoSpace)
							}
						} else {
							Ok(NoSpace)
						}
					} else {
						Ok(NoSpace)
					}
				NoSpace => Ok(NoSpace)
			}
		} else {
			Ok(NoSpace)
		}

	## Whether `owner`'s window is a native transient. Fact: 0 whether it is exempt.
	is_native_transient : [Entry({ origin : Origin, pid : U64 }), NoEntry], U64, Fact(Bool) -> Read(Bool)
	is_native_transient = |entry, owner, exempt|
		match entry {
			Entry({ origin: Native, pid }) =>
				if pid == owner {
					is_exempt = ask(exempt, 0)?
					Ok(!is_exempt)
				} else {
					Ok(False)
				}
			Entry({ origin: Heuristic, .. }) => Ok(False)
			NoEntry => Ok(False)
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

	## Whether an inventory that omits a window says nothing about it. Facts: 0 its assignment,
	## 1 whether a Space is active, 2 whether it is parked.
	out_of_inventory_view : MaybeSpace, Fact(MaybeSpace), Answers(U64, Bool), Fact(Bool) -> Read(Bool)
	out_of_inventory_view = |native, assigned, active, parked| {
		known =
			match native {
				Space(space) => Ok(Space(space))
				NoSpace => ask(assigned, 0)
			}?
		inactive =
			match known {
				Space(space) => ask_word(active, space, 1).map_ok(|is_active| !is_active)
				NoSpace => Ok(False)
			}?
		if inactive Ok(True) else ask(parked, 2)
	}

	## Whether a title change re-applies a window's app rules. Facts: 0 its Space, 1 whether a
	## Space is active, 2 whether it is reconcilable.
	reapplies_on_title_change : Bool, Fact(MaybeSpace), Answers(U64, Bool), Fact(Bool) -> Read(Bool)
	reapplies_on_title_change = |configured, space, active, reconcilable|
		if configured {
			space_of = ask(space, 0)?
			match space_of {
				Space(id) => {
					is_active = ask_word(active, id, 1)?
					if is_active ask(reconcilable, 2) else Ok(False)
				}
				NoSpace => Ok(False)
			}
		} else {
			Ok(False)
		}

	## Whether rift manages a window, and if not the first veto that applies.
	admits : Facts -> [Admitted, Vetoed(Veto)]
	admits = |facts|
		match facts.native {
			Transient => Vetoed(NativeTransient)
			Ordinary =>
				match facts.minimize {
					Minimized => Vetoed(Minimized)
					Shown => {
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
							Vetoed(AxTransient)
						} else {
							match facts.rule {
								Ignore => Vetoed(RuleIgnores)
								Manage => Admitted
								NoRule =>
									match facts.manageability {
										Manageable => Admitted
										Unmanageable => Vetoed(Unmanageable)
									}
							}
						}
					}
				}
		}

	## A window's facts with one changed.
	changed : Facts, FactChange -> Facts
	changed = |facts, change|
		match change {
			Manageability(manageability) => { ..facts, manageability }
			Rule(rule) => { ..facts, rule }
			Ax(ax) => { ..facts, ax }
			Native(native) => { ..facts, native }
			Minimize(minimize) => { ..facts, minimize }
		}

	## A window's facts after a reading (none without AX state), and the change in its
	## admission: only a change between admitted and vetoed is one.
	read : [Known(Facts), NoFacts], Reading -> ([Known(Facts), NoFacts], Change)
	read = |before, reading| {
		after =
			match reading {
				Live(facts) => Known(facts)
				Gone => NoFacts
				Changed(change) =>
					match before {
						NoFacts => NoFacts
						Known(facts) => Known(changed(facts, change))
					}
			}
		change =
			match after {
				NoFacts => NoChange
				Known(now) =>
					match admits(now) {
						Admitted =>
							match before {
								Known(was) =>
									match admits(was) {
										Admitted => NoChange
										Vetoed(_) => Admitted
									}
								NoFacts => Admitted
							}
						Vetoed(why) =>
							match before {
								Known(was) =>
									match admits(was) {
										Admitted => Suspended(why)
										Vetoed(_) => NoChange
									}
								NoFacts => NoChange
							}
					}
			}
		(after, change)
	}

	## A window's presence after an event, what the event came to (a read and its change, or
	## whether it invalidated or revalidated), and whether rift manages it then.
	presence_step : Presence, [Reading(Reading), Invalidate, Revalidate] -> { after : Presence, outcome : [Read(Change), Invalidated(Bool), Revalidated(Bool)], manages : Bool }
	presence_step = |before, event| {
		(after, outcome) =
			match event {
				Reading(reading) => {
					(presence, change) = presence_read(before, reading)
					(presence, Read(change))
				}
				Invalidate => {
					(presence, happened) = presence_invalidate(before)
					(presence, Invalidated(happened))
				}
				Revalidate => {
					(presence, happened) = presence_revalidate(before)
					(presence, Revalidated(happened))
				}
			}
		manages =
			match after {
				Live(facts) =>
					match admits(facts) {
						Admitted => True
						Vetoed(_) => False
					}
				Invalidated(_) => False
				NoPresence => False
			}
		{ after, outcome, manages }
	}

	## A window's presence after a reading of its facts, and the change in its admission: a
	## fresh reading is valid AX state, a change keeps the presence.
	presence_read : Presence, Reading -> (Presence, Change)
	presence_read = |before, reading| {
		known =
			match before {
				NoPresence => NoFacts
				Live(facts) => Known(facts)
				Invalidated(facts) => Known(facts)
			}
		(read_after, change) = read(known, reading)
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
		(presence, change)
	}

	## A window's presence after its AX element was invalidated, and whether it had AX state
	## to invalidate.
	presence_invalidate : Presence -> (Presence, Bool)
	presence_invalidate = |before|
		match before {
			NoPresence => (NoPresence, False)
			Live(facts) => (Invalidated(facts), True)
			Invalidated(facts) => (Invalidated(facts), True)
		}

	## A window's presence after a positive AX observation, and whether it had been
	## invalidated.
	presence_revalidate : Presence -> (Presence, Bool)
	presence_revalidate = |before|
		match before {
			Invalidated(facts) => (Live(facts), True)
			Live(facts) => (Live(facts), False)
			NoPresence => (NoPresence, False)
		}

	## The lifecycle of every window (`rift_core::lifecycle::Lifecycle`), kept between events:
	## each window with AX state and its presence. A window without AX state has no entry.
	windows_empty : {} -> Dict(U64, Present)
	windows_empty = |{}| Dict.empty()

	## A reading of `window`'s facts: the windows after it, and the change in its admission.
	windows_read : Dict(U64, Present), U64, Reading -> (Dict(U64, Present), Change)
	windows_read = |windows, window, reading| {
		(after, change) = presence_read(presence_in(windows, window), reading)
		(with_presence(windows, window, after), change)
	}

	## `window`'s AX element was invalidated: the windows after, and whether it had AX state.
	windows_invalidate : Dict(U64, Present), U64 -> (Dict(U64, Present), Bool)
	windows_invalidate = |windows, window| {
		(after, happened) = presence_invalidate(presence_in(windows, window))
		(with_presence(windows, window, after), happened)
	}

	## A positive AX observation of `window`: the windows after, and whether it had been
	## invalidated.
	windows_revalidate : Dict(U64, Present), U64 -> (Dict(U64, Present), Bool)
	windows_revalidate = |windows, window| {
		(after, happened) = presence_revalidate(presence_in(windows, window))
		(with_presence(windows, window, after), happened)
	}

	## Every window with AX state and its presence.
	windows_view : Dict(U64, Present) -> List({ window : U64, presence : Present })
	windows_view = |windows| windows.to_list().map(|(window, presence)| { window, presence })

	## `window`'s presence, none without an entry.
	presence_in : Dict(U64, Present), U64 -> Presence
	presence_in = |windows, window|
		match windows.get(window) {
			Ok(Live(facts)) => Live(facts)
			Ok(Invalidated(facts)) => Invalidated(facts)
			Err(_) => NoPresence
		}

	## The windows with `window` at `presence`: no presence is no entry.
	with_presence : Dict(U64, Present), U64, Presence -> Dict(U64, Present)
	with_presence = |windows, window, presence|
		match presence {
			NoPresence => windows.remove(window)
			Live(facts) => windows.insert(window, Live(facts))
			Invalidated(facts) => windows.insert(window, Invalidated(facts))
		}

	## Two admission changes in a row, as one: from where the first started to where the
	## second ended; changes that cancel out are none.
	net : Change, Change -> Change
	net = |first, later|
		match (first, later) {
			(NoChange, change) => change
			(change, NoChange) => change
			(Admitted, Suspended(_)) => NoChange
			(Suspended(_), Admitted) => NoChange
			(Admitted, Admitted) => Admitted
			(Suspended(_), Suspended(why)) => Suspended(why)
		}

	## Whether an inventory retires a window, or why it keeps it. Fact: 0 what the window server
	## says of it, asked only for an omitted window in view with AX state, shown, not in
	## fullscreen and with a native identity.
	retirement : InventoryFacts, Fact(NativeEvidence) -> Read([Retire, Keep(Keep)])
	retirement = |facts, evidence|
		match facts.report {
			Reported => Ok(Keep(Reported))
			Omitted =>
				match facts.view {
					OutOfView => Ok(Keep(OutOfView))
					InView =>
						match facts.minimize {
							NoAxState => Ok(Keep(NoAxState))
							Minimized => Ok(Keep(Minimized))
							Shown =>
								match facts.fullscreen {
									Recorded => Ok(Keep(Fullscreen))
									NotRecorded =>
										match facts.identity {
											Unknown => Ok(Keep(NoNativeIdentity))
											Known(_) => {
												native = ask(evidence, 0)?
												Ok(judged(native, facts.validity))
											}
										}
								}
						}
				}
		}

	## Retirement on native evidence: kept while suitable and ordered out with a valid AX
	## identity (ordering explains the omission), retired on any negative evidence.
	judged : NativeEvidence, [Valid, Invalidated] -> [Retire, Keep(Keep)]
	judged = |native, validity| {
		(suitable, unsuitable) =
			match native.suitability {
				Suitable => (True, False)
				Unsuitable => (False, True)
				SuitabilityUnknown => (False, False)
			}
		ordered_out =
			match native.ordering {
				OrderedOut => True
				OrderedIn => False
				OrderingUnknown => False
			}
		not_a_window =
			match native.look {
				NotAWindow => True
				AWindow => False
				LookUnknown => False
			}
		valid =
			match validity {
				Valid => True
				Invalidated => False
			}
		if suitable and ordered_out and valid {
			Keep(NativeOrdering)
		} else if unsuitable or ordered_out or not_a_window {
			Retire
		} else {
			Keep(NoNegativeEvidence)
		}
	}
}
