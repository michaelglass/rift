## Decisions about virtual workspaces (`rift_core::workspaces`): which workspace a command
## reaches and where stepping goes. Workspaces, Spaces and names are one word each.

import Wire exposing [finished, take, take_flag, take_list]

## Words being read, and where.
Words : { words : List(U64), at : U64 }

## A value read and where the rest starts, or the words were malformed.
Taken(a) : Try((a, Words), [Malformed])

## Where a command is given: the acting Space, and the Spaces visible now.
Context : { space : U64, visible : List(U64) }

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
		take_maybe = |c| {
			(present, after) = take_flag(c)?
			if present {
				(word, rest) = take(after)?
				Ok((Some(word), rest))
			} else {
				Ok((Nothing, after))
			}
		}
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
}
