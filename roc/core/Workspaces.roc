## Decisions about virtual workspaces (`rift_core::workspaces`): which workspace a command
## reaches and where stepping goes. Workspaces, Spaces and names are one word each.

import Wire exposing [finished, put_list, take, take_flag, take_list]

## Words being read, and where.
Words : { words : List(U64), at : U64 }

## A value read and where the rest starts, or the words were malformed.
Taken(a) : Try((a, Words), [Malformed])

## Where a command is given: the acting Space, and the Spaces visible now.
Context : { space : U64, visible : List(U64) }

## A workspace, when there is one.
Maybe : [Some(U64), Nothing]

## What a native Space shows.
Activation : [Showing({ current : U64, previous : Maybe, remembered : Maybe }), Vacant(Maybe)]

## A change to the workspaces' topology, as the platform makes it.
TopologyEvent : [
	Created({ workspace : U64, space : U64 }),
	Removed(U64),
	Moved({ workspace : U64, space : U64 }),
	Activated({ space : U64, activation : Activation }),
	Cleared(U64),
]

## The catalog (each workspace with its Space, in catalog order) and what each Space shows (in
## the order the Spaces first got an activation).
Topology : { catalog : List({ workspace : U64, space : U64 }), activations : List({ space : U64, activation : Activation }) }

## Where a claimed window returns to when its claim ends.
Destination : [Returns(U64), Nowhere]

## Who places a window, when somebody does: a workspace, or an external owner.
Ownership : [Assigned(U64), Claimed({ owner : U64, destination : Destination })]

## Who places each window, in window order; a window without an entry is unowned.
Ownerships : List({ window : U64, ownership : Ownership })

Workspaces := [].{

	## Whether a workspace on `on` is reachable: per display, only the acting Space's;
	## globally, a visible Space's (the acting Space's when none is known).
	eligible : Bool, Context, U64 -> Bool
	eligible = |global, context, on|
		if global {
			context.visible.contains(on) or (context.visible.is_empty() and on == context.space)
		} else {
			on == context.space
		}

	## Where a step from `current` lands, as an option's words.
	step : Bool, List({ id : U64, space : U64 }), Context, U64, Bool, Bool, Bool, List(U64) -> List(U64)
	step = |global, catalog, context, current, backwards, skip, stop, with_windows| {
		reachable = |entry| eligible(global, context, entry.space)
		if !catalog.any(|entry| entry.id == current and reachable(entry)) {
			[0]
		} else {
			counts = |id| if (skip) with_windows.contains(id) else True
			ordered = if (backwards) reversed(catalog) else catalog
			var $crossed = False
			var $wrapped = [0]
			var $found = [0]
			var $done = False
			for entry in ordered {
				if $done or !reachable(entry) {
					{}
				} else if entry.id == current {
					$crossed = True
					if $wrapped == [0] and counts(entry.id) {
						$wrapped = [1, entry.id]
					} else {
						{}
					}
				} else if counts(entry.id) {
					if $crossed {
						$found = [1, entry.id]
						$done = True
					} else if $wrapped == [0] {
						$wrapped = [1, entry.id]
					} else {
						{}
					}
				} else {
					{}
				}
			}
			if $done $found else if (stop) [0] else $wrapped
		}
	}

	## `items` from the last to the first.
	reversed : List(a) -> List(a)
	reversed = |items| {
		var $out = []
		for item in items {
			$out = [item].concat($out)
		}
		$out
	}

	## The workspace a selector names in scope, if reachable, as an option of (id, Space).
	resolve : Bool, List({ id : U64, space : U64, name : U64 }), Context, [Index(U64), Name(U64)] -> List(U64)
	resolve = |global, scope, context, selector| {
		named =
			match selector {
				Index(index) =>
					match scope.get(index) {
						Ok(entry) => Found(entry)
						Err(_) => Missing
					}
				Name(name) => {
					var $hit = Missing
					for entry in scope {
						match $hit {
							Missing => if (entry.name == name) { $hit = Found(entry) } else {}
							Found(_) => {}
						}
					}
					$hit
				}
			}
		match named {
			Found(entry) => if (eligible(global, context, entry.space)) [1, entry.id, entry.space] else [0]
			Missing => [0]
		}
	}

	## The topology after one event. A move or removal of an unknown workspace, or a clear of
	## a Space without an activation, changes nothing; a move moves the first entry for the
	## workspace; an activation replaces the Space's in place, or comes last.
	topology_step : Topology, TopologyEvent -> Topology
	topology_step = |topology, event|
		match event {
			Created(entry) => { ..topology, catalog: topology.catalog.append(entry) }
			Removed(workspace) => { ..topology, catalog: topology.catalog.keep_if(|entry| entry.workspace != workspace) }
			Moved(moved) => {
				var $catalog = []
				var $moving = Moving
				for entry in topology.catalog {
					match $moving {
						Moving if entry.workspace == moved.workspace => {
							$catalog = $catalog.append({ workspace: entry.workspace, space: moved.space })
							$moving = Moved
						}
						_ => {
							$catalog = $catalog.append(entry)
						}
					}
				}
				{ ..topology, catalog: $catalog }
			}
			Activated(shown) => {
				var $activations = []
				var $placing = Placing
				for entry in topology.activations {
					match $placing {
						Placing if entry.space == shown.space => {
							$activations = $activations.append(shown)
							$placing = Placed
						}
						_ => {
							$activations = $activations.append(entry)
						}
					}
				}
				match $placing {
					Placing => { ..topology, activations: $activations.append(shown) }
					Placed => { ..topology, activations: $activations }
				}
			}
			Cleared(space) => { ..topology, activations: topology.activations.keep_if(|entry| entry.space != space) }
		}

	## The ownerships after a window's new owner: replaced in place, or put in window order.
	own : Ownerships, U64, Ownership -> Ownerships
	own = |ownerships, window, ownership| {
		var $out = []
		var $placing = Placing
		for entry in ownerships {
			match $placing {
				Placing if entry.window == window => {
					$out = $out.append({ window, ownership })
					$placing = Placed
				}
				Placing if entry.window > window => {
					$out = $out.append({ window, ownership }).append(entry)
					$placing = Placed
				}
				_ => {
					$out = $out.append(entry)
				}
			}
		}
		match $placing {
			Placing => $out.append({ window, ownership })
			Placed => $out
		}
	}

	## Each owned window naming a workspace the catalog lacks (assigned, or a claim's
	## destination), with that workspace, in window order.
	orphans : Ownerships, List(U64) -> List({ window : U64, workspace : U64 })
	orphans = |ownerships, catalog| {
		var $out = []
		for entry in ownerships {
			named =
				match entry.ownership {
					Assigned(workspace) => Names(workspace)
					Claimed({ destination: Returns(workspace), .. }) => Names(workspace)
					Claimed({ destination: Nowhere, .. }) => NamesNone
				}
			match named {
				Names(workspace) if !catalog.contains(workspace) => {
					$out = $out.append({ window: entry.window, workspace })
				}
				_ => {}
			}
		}
		$out
	}

	# The wire.

	take_mode : Words -> Taken(Bool)
	take_mode = |c0| {
		(word, c1) = take(c0)?
		match word {
			0 => Ok((False, c1))
			1 => Ok((True, c1))
			_ => Err(Malformed)
		}
	}

	take_binary : Words -> Taken(Bool)
	take_binary = |c0| take_mode(c0)

	take_context : Words -> Taken(Context)
	take_context = |c0| {
		(space, c1) = take(c0)?
		(visible, c2) = take_list(c1, take)?
		Ok(({ space, visible }, c2))
	}

	## Tag 29.
	step_words : Words -> Try(List(U64), [Malformed])
	step_words = |c0| {
		take_entry = |c| {
			(id, after) = take(c)?
			(space, rest) = take(after)?
			Ok(({ id, space }, rest))
		}
		(global, c1) = take_mode(c0)?
		(catalog, c2) = take_list(c1, take_entry)?
		(context, c3) = take_context(c2)?
		(current, c4) = take(c3)?
		(backwards, c5) = take_binary(c4)?
		(skip, c6) = take_binary(c5)?
		(stop, c7) = take_binary(c6)?
		(with_windows, c8) = take_list(c7, take)?
		finished(c8)?
		Ok(step(global, catalog, context, current, backwards, skip, stop, with_windows))
	}

	## Tag 30.
	resolve_words : Words -> Try(List(U64), [Malformed])
	resolve_words = |c0| {
		take_entry = |c| {
			(id, a) = take(c)?
			(space, b) = take(a)?
			(name, rest) = take(b)?
			Ok(({ id, space, name }, rest))
		}
		(global, c1) = take_mode(c0)?
		(scope, c2) = take_list(c1, take_entry)?
		(context, c3) = take_context(c2)?
		(kind, c4) = take(c3)?
		(value, c5) = take(c4)?
		selector =
			match kind {
				0 => Ok(Index(value))
				1 => Ok(Name(value))
				_ => Err(Malformed)
			}?
		finished(c5)?
		Ok(resolve(global, scope, context, selector))
	}

	## Tag 31.
	previous_words : Words -> Try(List(U64), [Malformed])
	previous_words = |c0| {
		(global, c1) = take_mode(c0)?
		(context, c2) = take_context(c1)?
		(last, c3) = take_maybe(c2)?
		(on, c4) = take_maybe(c3)?
		finished(c4)?
		Ok(
			match last {
				Some(id) =>
					match on {
						Some(space) => if (eligible(global, context, space)) [1, id, space] else [0]
						Nothing => [0]
					}
				Nothing => [0]
			},
		)
	}

	take_maybe : Words -> Taken(Maybe)
	take_maybe = |c0| {
		(present, c1) = take_flag(c0)?
		if present {
			(word, c2) = take(c1)?
			Ok((Some(word), c2))
		} else {
			Ok((Nothing, c1))
		}
	}

	take_activation : Words -> Taken(Activation)
	take_activation = |c0| {
		(kind, c1) = take(c0)?
		match kind {
			0 => {
				(current, c2) = take(c1)?
				(previous, c3) = take_maybe(c2)?
				(remembered, c4) = take_maybe(c3)?
				Ok((Showing({ current, previous, remembered }), c4))
			}
			1 => {
				(remembered, c2) = take_maybe(c1)?
				Ok((Vacant(remembered), c2))
			}
			_ => Err(Malformed)
		}
	}

	take_topology_event : Words -> Taken(TopologyEvent)
	take_topology_event = |c0| {
		(kind, c1) = take(c0)?
		match kind {
			0 => {
				(workspace, c2) = take(c1)?
				(space, c3) = take(c2)?
				Ok((Created({ workspace, space }), c3))
			}
			1 => {
				(workspace, c2) = take(c1)?
				Ok((Removed(workspace), c2))
			}
			2 => {
				(workspace, c2) = take(c1)?
				(space, c3) = take(c2)?
				Ok((Moved({ workspace, space }), c3))
			}
			3 => {
				(space, c2) = take(c1)?
				(activation, c3) = take_activation(c2)?
				Ok((Activated({ space, activation }), c3))
			}
			4 => {
				(space, c2) = take(c1)?
				Ok((Cleared(space), c2))
			}
			_ => Err(Malformed)
		}
	}

	put_maybe : List(U64), Maybe -> List(U64)
	put_maybe = |words, maybe|
		match maybe {
			Some(word) => words.append(1).append(word)
			Nothing => words.append(0)
		}

	put_activation : List(U64), Activation -> List(U64)
	put_activation = |words, activation|
		match activation {
			Showing(shown) => put_maybe(put_maybe(words.append(0).append(shown.current), shown.previous), shown.remembered)
			Vacant(remembered) => put_maybe(words.append(1), remembered)
		}

	## Tag 32: the topology an empty one becomes after the events.
	topology_words : Words -> Try(List(U64), [Malformed])
	topology_words = |c0| {
		(events, c1) = take_list(c0, take_topology_event)?
		finished(c1)?
		topology = events.fold({ catalog: [], activations: [] }, topology_step)
		with_catalog = put_list([], topology.catalog, |words, entry| words.append(entry.workspace).append(entry.space))
		Ok(put_list(with_catalog, topology.activations, |words, entry| put_activation(words.append(entry.space), entry.activation)))
	}

	take_ownership : Words -> Taken(Ownership)
	take_ownership = |c0| {
		(kind, c1) = take(c0)?
		match kind {
			0 => {
				(workspace, c2) = take(c1)?
				Ok((Assigned(workspace), c2))
			}
			1 => {
				(owner, c2) = take(c1)?
				(returns, c3) = take_flag(c2)?
				if returns {
					(workspace, c4) = take(c3)?
					Ok((Claimed({ owner, destination: Returns(workspace) }), c4))
				} else {
					Ok((Claimed({ owner, destination: Nowhere }), c3))
				}
			}
			_ => Err(Malformed)
		}
	}

	put_ownership : List(U64), Ownership -> List(U64)
	put_ownership = |words, ownership|
		match ownership {
			Assigned(workspace) => words.append(0).append(workspace)
			Claimed({ owner, destination: Returns(workspace) }) => words.append(1).append(owner).append(1).append(workspace)
			Claimed({ owner, destination: Nowhere }) => words.append(1).append(owner).append(0)
		}

	## Tag 33: the ownerships an empty one becomes after the events, and their orphans
	## against the catalog.
	ownership_words : Words -> Try(List(U64), [Malformed])
	ownership_words = |c0| {
		take_event = |c| {
			(window, after) = take(c)?
			(owned, rest) = take_flag(after)?
			if owned {
				(ownership, end) = take_ownership(rest)?
				Ok(({ window, owned: Owner(ownership) }, end))
			} else {
				Ok(({ window, owned: Unowned }, rest))
			}
		}
		(events, c1) = take_list(c0, take_event)?
		(catalog, c2) = take_list(c1, take)?
		finished(c2)?
		ownerships = events.fold(
			[],
			|acc, event|
				match event.owned {
					Unowned => acc.keep_if(|entry| entry.window != event.window)
					Owner(ownership) => own(acc, event.window, ownership)
				},
		)
		with_owners = put_list([], ownerships, |words, entry| put_ownership(words.append(entry.window), entry.ownership))
		Ok(put_list(with_owners, orphans(ownerships, catalog), |words, orphan| words.append(orphan.window).append(orphan.workspace)))
	}
}
