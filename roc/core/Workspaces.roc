## Decisions about virtual workspaces (`rift_core::workspaces`): which workspace a command
## reaches and where stepping goes. Workspaces, Spaces and names are one word each.

import pf.Boundary exposing [Context, MaybeWord, Activation, SpaceActivation, WorkspaceSpace, Ownership, WindowOwnership, WorkspaceWindows, JoinedEvent, JoinedView, Outcome, Reached, CatalogEntry, ScopeEntry, Selector, FactWord, WordBool]
import Asking exposing [ask, ask_word]

## A change to the workspaces' topology, as the platform makes it.
TopologyEvent : [
	Created(WorkspaceSpace),
	Removed(U64),
	Moved(WorkspaceSpace),
	Activated(SpaceActivation),
	Cleared(U64),
]

## The catalog (each workspace with its Space, in catalog order) and what each Space shows (in
## the order the Spaces first got an activation).
Topology : { catalog : List(WorkspaceSpace), activations : List(SpaceActivation) }

## Who places each window, keyed by window, so no window has two owners; a window without
## an entry is unowned. Views give them in window order (`in_order`). A claim is kept whole: its
## owner (the one part a decision reads), how it claimed, the id it claimed, and where the
## window returns when it ends.
Ownerships : Dict(U64, Ownership)

## The workspaces joined: the catalog, what each Space shows, and who places each window.
## Every workspace an activation or an ownership names is in the catalog.
Joined : {
	catalog : List(WorkspaceSpace),
	activations : List(SpaceActivation),
	owners : Ownerships,
}

## What a decision decided, or the first fact it read that its question does not carry.
Read(a) : Try(a, [Unasked(U64)])

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

	## Where a step from `current` lands, if anywhere. Fact: 0 whether a workspace has windows
	## (asked only when skipping empty ones, in the order the step meets them).
	step : Bool, List(CatalogEntry), Context, U64, Bool, Bool, Bool, List(WordBool) -> Read(MaybeWord)
	step = |global, catalog, context, current, backwards, skip, stop, has_windows| {
		reachable = |entry| eligible(global, context, entry.space)
		if !catalog.any(|entry| entry.id == current and reachable(entry)) {
			Ok(Nothing)
		} else {
			counts = |id| if (skip) ask_word(has_windows, id, 0) else Ok(True)
			unset = |maybe|
				match maybe {
					Nothing => True
					Some(_) => False
				}
			ordered = if (backwards) reversed(catalog) else catalog
			var $crossed = False
			var $wrapped = Nothing
			var $found = Nothing
			var $state = Going
			for entry in ordered {
				match $state {
					Going =>
						if !reachable(entry) {
							{}
						} else if entry.id == current {
							$crossed = True
							if unset($wrapped) {
								match counts(entry.id) {
									Ok(True) => {
										$wrapped = Some(entry.id)
									}
									Ok(False) => {}
									Err(unasked) => {
										$state = Stopped(unasked)
									}
								}
							} else {
								{}
							}
						} else {
							match counts(entry.id) {
								Ok(True) =>
									if $crossed {
										$found = Some(entry.id)
										$state = Done
									} else if unset($wrapped) {
										$wrapped = Some(entry.id)
									} else {
										{}
									}
								Ok(False) => {}
								Err(unasked) => {
									$state = Stopped(unasked)
								}
							}
						}
					Done => {}
					Stopped(_) => {}
				}
			}
			match $state {
				Stopped(unasked) => Err(unasked)
				Done => Ok($found)
				Going => if (stop) Ok(Nothing) else Ok($wrapped)
			}
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

	## The workspace a selector names in scope and its Space, if reachable.
	resolve : Bool, List(ScopeEntry), Context, Selector -> Reached
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
			Found(entry) => if (eligible(global, context, entry.space)) Reached({ workspace: entry.id, space: entry.space }) else Unreached
			Missing => Unreached
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

	## Every owned window and its ownership, in window order: the order every view and answer
	## gives them in.
	in_order : Ownerships -> List(WindowOwnership)
	in_order = |ownerships|
		ownerships
			.to_list()
			.sort_with(|(a, _), (b, _)| if a < b Before else if a > b After else Same)
			.map(|(window, ownership)| { window, ownership })

	known : Joined, U64 -> Bool
	known = |joined, workspace| joined.catalog.any(|entry| entry.workspace == workspace)

	## The first of `workspaces` the catalog lacks, as a refusal, or accepted.
	first_unknown : Joined, List(U64) -> Outcome
	first_unknown = |joined, workspaces| {
		var $outcome = Accepted
		for workspace in workspaces {
			match $outcome {
				Accepted if !known(joined, workspace) => {
					$outcome = Unknown(workspace)
				}
				_ => {}
			}
		}
		$outcome
	}

	maybe_list : MaybeWord -> List(U64)
	maybe_list = |maybe|
		match maybe {
			Some(workspace) => [workspace]
			Nothing => []
		}

	## Whether the joined workspaces take the event, or why not.
	admit : Joined, JoinedEvent -> Outcome
	admit = |joined, event|
		match event {
			Created(created) => if (known(joined, created.workspace)) Exists(created.workspace) else Accepted
			Removed(removed) =>
				if !known(joined, removed.workspace) {
					Unknown(removed.workspace)
				} else {
					match removed.successor {
						Into(heir) if heir == removed.workspace => OwnSuccessor(heir)
						Into(heir) if !known(joined, heir) => Unknown(heir)
						_ => {
							var $shown = Accepted
							for entry in joined.activations {
								match ($shown, entry.activation) {
									(Accepted, Showing(shown)) if shown.current == removed.workspace => {
										$shown = Shown({ workspace: removed.workspace, space: entry.space })
									}
									_ => {}
								}
							}
							$shown
						}
					}
				}
			Moved(moved) => first_unknown(joined, [moved.workspace])
			Activated(activated) =>
				match activated.activation {
					Showing(shown) => first_unknown(joined, [shown.current].concat(maybe_list(shown.previous)).concat(maybe_list(shown.remembered)))
					Vacant(remembered) => first_unknown(joined, maybe_list(remembered))
				}
			Cleared(_) => Accepted
			Owned(owned) =>
				match owned.owned {
					Unowned => Accepted
					Owner(Assigned(workspace)) => first_unknown(joined, [workspace])
					Owner(Claimed({ destination: Returns(workspace), .. })) => first_unknown(joined, [workspace])
					Owner(Claimed({ destination: Nowhere, .. })) => Accepted
					Owner(Held) => Accepted
				}
		}

	## The joined workspaces after an event they take.
	apply : Joined, JoinedEvent -> Joined
	apply = |joined, event| {
		topology = |topology_event| {
			stepped = topology_step({ catalog: joined.catalog, activations: joined.activations }, topology_event)
			{ ..joined, catalog: stepped.catalog, activations: stepped.activations }
		}
		match event {
			Created(created) => topology(Created(created))
			Moved(moved) => topology(Moved(moved))
			Activated(activated) => topology(Activated(activated))
			Cleared(space) => topology(Cleared(space))
			Owned(owned) =>
				match owned.owned {
					Unowned => { ..joined, owners: joined.owners.remove(owned.window) }
					Owner(ownership) => { ..joined, owners: joined.owners.insert(owned.window, ownership) }
				}
			Removed(removed) => {
				gone = removed.workspace
				var $owners = joined.owners
				for (window, ownership) in joined.owners.to_list() {
					match (ownership, removed.successor) {
						(Assigned(id), Into(heir)) if id == gone => {
							$owners = $owners.insert(window, Assigned(heir))
						}
						(Assigned(id), Unassign) if id == gone => {
							$owners = $owners.remove(window)
						}
						(Claimed(claim), Into(heir)) if claim.destination == Returns(gone) => {
							$owners = $owners.insert(window, Claimed({ ..claim, destination: Returns(heir) }))
						}
						(Claimed(claim), Unassign) if claim.destination == Returns(gone) => {
							$owners = $owners.insert(window, Claimed({ ..claim, destination: Nowhere }))
						}
						_ => {}
					}
				}
				forget = |maybe|
					match maybe {
						Some(id) if id == gone => Nothing
						_ => maybe
					}
				activations = joined.activations.map(
					|entry| {
						space: entry.space,
						activation: match entry.activation {
							Showing(shown) => Showing({ current: shown.current, previous: forget(shown.previous), remembered: forget(shown.remembered) })
							Vacant(remembered) => Vacant(forget(remembered))
						},
					},
				)
				{ catalog: joined.catalog.keep_if(|entry| entry.workspace != gone), activations, owners: $owners }
			}
		}
	}

	## The windows assigned to a workspace, in window order.
	windows_on : Joined, U64 -> List(U64)
	windows_on = |joined, workspace| {
		var $windows = []
		for entry in in_order(joined.owners) {
			match entry.ownership {
				Assigned(id) if id == workspace => {
					$windows = $windows.append(entry.window)
				}
				_ => {}
			}
		}
		$windows
	}

	# The wire.

	## The workspace navigation history points at and its Space, if it is on a Space and
	## reachable. Fact: 0 the Space it is on (asked only with a history).
	previous : Bool, Context, MaybeWord, FactWord -> Read(Reached)
	previous = |global, context, last, on|
		match last {
			Some(id) => {
				on_space = ask(on, 0)?
				match on_space {
					Some(space) => if (eligible(global, context, space)) Ok(Reached({ workspace: id, space })) else Ok(Unreached)
					Nothing => Ok(Unreached)
				}
			}
			Nothing => Ok(Unreached)
		}

	## Whether window discovery emits a window it found on a Space: unless the window is
	## assigned to a workspace and the Space's active workspace is another one. Fact: 0 the
	## Space's active workspace (asked only for an assigned window).
	emits_window : MaybeWord, FactWord -> Read(Bool)
	emits_window = |assigned, active|
		match assigned {
			Nothing => Ok(True)
			Some(workspace) => {
				active_workspace = ask(active, 0)?
				match active_workspace {
					Some(shown) => Ok(shown == workspace)
					Nothing => Ok(True)
				}
			}
		}

	## What each event of a run met from empty workspaces, and the workspaces it leaves.
	joined_after : List(JoinedEvent) -> JoinedView
	joined_after = |events| {
		(joined, outcomes) = events.fold(
			({ catalog: [], activations: [], owners: Dict.empty() }, []),
			|(state, met), event| {
				outcome = admit(state, event)
				match outcome {
					Accepted => (apply(state, event), met.append(outcome))
					Exists(_) | Unknown(_) | OwnSuccessor(_) | Shown(_) => (state, met.append(outcome))
				}
			},
		)
		{
			outcomes,
			catalog: joined.catalog,
			activations: joined.activations,
			owners: in_order(joined.owners),
			windows: joined.catalog.map(|entry| { workspace: entry.workspace, windows: windows_on(joined, entry.workspace) }),
		}
	}
}
