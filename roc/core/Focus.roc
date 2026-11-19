## Decisions about focus and raising (`rift_core::focus`): what a removal does to focus, the
## window to focus in a workspace, hidden focus, following an activation, and the fallbacks.
## Questions on plain data; Spaces, workspaces, window-server ids, modes and apps are one word
## each, and a window with an app is a WindowId (its app is its pid).

import Wire exposing [finished, put_window, take, take_flag, take_list, take_window]

WindowId : { pid : I32, idx : U32 }

## Words being read, and where.
Words : { words : List(U64), at : U64 }

## A value read and where the rest starts, or the words were malformed.
Taken(a) : Try((a, Words), [Malformed])

## A word, or none.
Maybe : [Some(U64), Nothing]

## A window, or none.
MaybeWindow : [SomeWindow(WindowId), NoWindow]

## A fact the platform's run of the decision answered, or one it never asked.
Fact(a) : [Asked(a), Unasked]

## A fact asked about keys: each key asked and its answer. Any other key was not asked.
Answers(k, v) : List({ key : k, answer : v })

## What a decision decided, or the first fact it read that its question does not carry (the
## fact's place among the question's facts): it decides in another order than the platform's.
Read(a) : Try(a, [Unasked(U64)])

Focus := [].{

	## The `fact`-th fact's answer.
	ask : Fact(a), U64 -> Read(a)
	ask = |given, fact|
		match given {
			Asked(answer) => Ok(answer)
			Unasked => Err(Unasked(fact))
		}

	## The `fact`-th fact's answer for a window.
	ask_window : Answers(WindowId, v), WindowId, U64 -> Read(v)
	ask_window = |answers, key, fact| {
		var $found = Err(Unasked(fact))
		for entry in answers {
			match $found {
				Err(Unasked(_)) => if (entry.key == key) { $found = Ok(entry.answer) } else {}
				Ok(_) => {}
			}
		}
		$found
	}

	## The `fact`-th fact's answer for a word.
	ask_word : Answers(U64, v), U64, U64 -> Read(v)
	ask_word = |answers, key, fact| {
		var $found = Err(Unasked(fact))
		for entry in answers {
			match $found {
				Err(Unasked(_)) => if (entry.key == key) { $found = Ok(entry.answer) } else {}
				Ok(_) => {}
			}
		}
		$found
	}

	## What a removal does to focus. Facts: 0 the removed window's Space, 1 macOS's key window, 2
	## the successor a native window is tracked as, 3 whether a successor is in an inactive
	## workspace.
	removal_focus : [AppClosed(I32), WindowRemoved(WindowId)], MaybeWindow, Fact(Maybe), Fact(MaybeWindow), Answers(WindowId, WindowId), Answers(WindowId, Bool) -> Read([Unaffected, KeepNative(WindowId), Refocus(U64)])
	removal_focus = |removal, focused, removed_space, native_focus, successors, inactive| {
		removed =
			match focused {
				SomeWindow(window) =>
					match removal {
						AppClosed(pid) => if (window.pid == pid) SomeWindow(window) else NoWindow
						WindowRemoved(gone) => if (window == gone) SomeWindow(window) else NoWindow
					}
				NoWindow => NoWindow
			}
		match removed {
			NoWindow => Ok(Unaffected)
			SomeWindow(window) => {
				space_of = ask(removed_space, 0)?
				match space_of {
					Nothing => Ok(Unaffected)
					Some(space) =>
						match removal {
							AppClosed(_) => Ok(Refocus(space))
							WindowRemoved(_) => {
								native_of = ask(native_focus, 1)?
								match native_of {
									SomeWindow(native) =>
										if native.pid == window.pid and native != window {
											successor = ask_window(successors, native, 2)?
											if successor == window {
												Ok(Refocus(space))
											} else {
												hidden = ask_window(inactive, successor, 3)?
												if hidden Ok(Refocus(space)) else Ok(KeepNative(successor))
											}
										} else {
											Ok(Refocus(space))
										}
									NoWindow => Ok(Refocus(space))
								}
							}
						}
				}
			}
		}
	}

	## The window to focus: preferred, else last focused, else the first of the workspace's,
	## whichever is visible first. Facts: 0 the last focused window, 1 the workspace's windows, 2
	## whether a window is visible.
	focus_candidate : Maybe, Fact(Maybe), Fact(List(U64)), Answers(U64, Bool) -> Read(Maybe)
	focus_candidate = |preferred, last_focused, in_workspace, visible| {
		visible_one = |candidate|
			match candidate {
				Some(window) => {
					shown = ask_word(visible, window, 2)?
					if shown Ok(Some(window)) else Ok(Nothing)
				}
				Nothing => Ok(Nothing)
			}
		first = visible_one(preferred)?
		match first {
			Some(window) => Ok(Some(window))
			Nothing => {
				last = ask(last_focused, 0)?
				second = visible_one(last)?
				match second {
					Some(window) => Ok(Some(window))
					Nothing => {
						windows = ask(in_workspace, 1)?
						var $found = Ok(Nothing)
						for window in windows {
							match $found {
								Ok(Nothing) =>
									match ask_word(visible, window, 2) {
										Ok(True) => { $found = Ok(Some(window)) }
										Ok(False) => {}
										Err(unasked) => { $found = Err(unasked) }
									}
								Ok(Some(_)) => {}
								Err(_) => {}
							}
						}
						$found
					}
				}
			}
		}
	}

	## Whether `window`, the main or the focused one, is hidden in an inactive workspace. Fact: 0
	## whether it is in an inactive workspace.
	hides_focus : U64, Maybe, Maybe, Fact(Bool) -> Read(Bool)
	hides_focus = |window, main, focused, in_inactive_workspace| {
		is = |candidate|
			match candidate {
				Some(id) => id == window
				Nothing => False
			}
		if is(main) or is(focused) ask(in_inactive_workspace, 0) else Ok(False)
	}

	## Whether an activation follows its window: 0 with the Space and workspace, or 1 with the
	## reason (and its word, for a mode or an unmanageable source). Facts 0 to 9, in the order
	## they are asked.
	activation_follow : { standard : Fact(Bool), unavailable : Fact(Bool), space : Fact(Maybe), workspace : Fact(Maybe), active_workspace : Fact(Maybe), pending_window : Fact(Bool), mode_refusal : Fact(Maybe), active_space_fullscreen : Fact(Bool), unmanageable_source : Fact(Maybe), blacklisted : Fact([Listed(Bool), NoBundle]) } -> Read(List(U64))
	activation_follow = |facts| {
		standard = ask(facts.standard, 0)?
		if !standard {
			Ok([1, 0])
		} else {
			unavailable = ask(facts.unavailable, 1)?
			if unavailable {
				Ok([1, 1])
			} else {
				space_of = ask(facts.space, 2)?
				match space_of {
					Nothing => Ok([1, 2])
					Some(space) => {
						workspace_of = ask(facts.workspace, 3)?
						match workspace_of {
							Nothing => Ok([1, 3])
							Some(workspace) => {
								active_of = ask(facts.active_workspace, 4)?
								already =
									match active_of {
										Some(active) => active == workspace
										Nothing => True
									}
								if already {
									Ok([1, 4])
								} else {
									after_active(facts, space, workspace)
								}
							}
						}
					}
				}
			}
		}
	}

	## The rest of an activation follow, once its window's workspace is not the active one.
	after_active : { standard : Fact(Bool), unavailable : Fact(Bool), space : Fact(Maybe), workspace : Fact(Maybe), active_workspace : Fact(Maybe), pending_window : Fact(Bool), mode_refusal : Fact(Maybe), active_space_fullscreen : Fact(Bool), unmanageable_source : Fact(Maybe), blacklisted : Fact([Listed(Bool), NoBundle]) }, U64, U64 -> Read(List(U64))
	after_active = |facts, space, workspace| {
		pending = ask(facts.pending_window, 5)?
		if pending {
			Ok([1, 5])
		} else {
			mode_of = ask(facts.mode_refusal, 6)?
			match mode_of {
				Some(mode) => Ok([1, 6, mode])
				Nothing => {
					fullscreen = ask(facts.active_space_fullscreen, 7)?
					if fullscreen {
						Ok([1, 7])
					} else {
						source_of = ask(facts.unmanageable_source, 8)?
						match source_of {
							Some(server) => Ok([1, 8, server])
							Nothing => {
								listed = ask(facts.blacklisted, 9)?
								match listed {
									NoBundle => Ok([1, 9])
									Listed(True) => Ok([1, 10])
									Listed(False) => Ok([0, space, workspace])
								}
							}
						}
					}
				}
			}
		}
	}

	## The display to activate after a command moved the focused window. Facts: 0 whether a Space
	## is active, 1 whether it is on a known display.
	display_to_follow : Maybe, Maybe, Answers(U64, Bool), Answers(U64, Bool) -> Read(Maybe)
	display_to_follow = |focused_space, previous, active, on_known_display|
		match focused_space {
			Some(space) => {
				changed =
					match previous {
						Some(before) => before != space
						Nothing => True
					}
				if changed {
					is_active = ask_word(active, space, 0)?
					if is_active {
						known = ask_word(on_known_display, space, 1)?
						if known Ok(Some(space)) else Ok(Nothing)
					} else {
						Ok(Nothing)
					}
				} else {
					Ok(Nothing)
				}
			}
			Nothing => Ok(Nothing)
		}

	# The wire.

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

	take_maybe_window : Words -> Taken(MaybeWindow)
	take_maybe_window = |c0| {
		(present, c1) = take_flag(c0)?
		if present {
			(window, c2) = take_window(c1)?
			Ok((SomeWindow(window), c2))
		} else {
			Ok((NoWindow, c1))
		}
	}

	## A pid: its word sign-extended, within I32.
	take_pid : Words -> Taken(I32)
	take_pid = |c0| {
		(word, c1) = take(c0)?
		signed = word.to_i64_wrap()
		if signed < -2147483648 or signed > 2147483647 {
			Err(Malformed)
		} else {
			Ok((word.to_i32_wrap(), c1))
		}
	}

	## A fact: a flag (0 unasked, 1 asked) and then its answer.
	take_fact : Words, (Words -> Taken(a)) -> Taken(Fact(a))
	take_fact = |c0, take_answer| {
		(asked, c1) = take_flag(c0)?
		if asked {
			(answer, c2) = take_answer(c1)?
			Ok((Asked(answer), c2))
		} else {
			Ok((Unasked, c1))
		}
	}

	## Answers: a list of (key, answer).
	take_answers : Words, (Words -> Taken(k)), (Words -> Taken(v)) -> Taken(Answers(k, v))
	take_answers = |c0, take_key, take_answer|
		take_list(
			c0,
			|c| {
				(key, after) = take_key(c)?
				(answer, rest) = take_answer(after)?
				Ok(({ key, answer }, rest))
			},
		)

	put_maybe : List(U64), Maybe -> List(U64)
	put_maybe = |words, maybe|
		match maybe {
			Some(word) => words.append(1).append(word)
			Nothing => words.append(0)
		}

	put_verdict : Bool -> List(U64)
	put_verdict = |verdict| if (verdict) [1] else [0]

	## Tag 17.
	removal_words : Words -> Try(List(U64), [Malformed, Unasked(U64)])
	removal_words = |c0| {
		(kind, c1) = take(c0)?
		(removal, c2) =
			match kind {
				0 => take_pid(c1).map_ok(|(pid, rest)| (AppClosed(pid), rest))
				1 => take_window(c1).map_ok(|(window, rest)| (WindowRemoved(window), rest))
				_ => Err(Malformed)
			}?
		(focused, c3) = take_maybe_window(c2)?
		(removed_space, c4) = take_fact(c3, take_maybe)?
		(native_focus, c5) = take_fact(c4, take_maybe_window)?
		(successors, c6) = take_answers(c5, take_window, take_window)?
		(inactive, c7) = take_answers(c6, take_window, take_flag)?
		finished(c7)?
		answer = removal_focus(removal, focused, removed_space, native_focus, successors, inactive)?
		Ok(
			match answer {
				Unaffected => [0]
				KeepNative(window) => put_window([1], window)
				Refocus(space) => [2, space]
			},
		)
	}

	## Tag 18.
	candidate_words : Words -> Try(List(U64), [Malformed, Unasked(U64)])
	candidate_words = |c0| {
		(preferred, c1) = take_maybe(c0)?
		(last_focused, c2) = take_fact(c1, take_maybe)?
		(in_workspace, c3) = take_fact(c2, |c| take_list(c, take))?
		(visible, c4) = take_answers(c3, take, take_flag)?
		finished(c4)?
		answer = focus_candidate(preferred, last_focused, in_workspace, visible)?
		Ok(put_maybe([], answer))
	}

	## Tag 19.
	hides_words : Words -> Try(List(U64), [Malformed, Unasked(U64)])
	hides_words = |c0| {
		(window, c1) = take(c0)?
		(main, c2) = take_maybe(c1)?
		(focused, c3) = take_maybe(c2)?
		(in_inactive_workspace, c4) = take_fact(c3, take_flag)?
		finished(c4)?
		answer = hides_focus(window, main, focused, in_inactive_workspace)?
		Ok(put_verdict(answer))
	}

	## A bundle's blacklisting: a flag (0 no bundle id, 1 one) and then whether it is listed.
	take_listing : Words -> Taken([Listed(Bool), NoBundle])
	take_listing = |c0| {
		(has_bundle, c1) = take_flag(c0)?
		if has_bundle {
			(listed, c2) = take_flag(c1)?
			Ok((Listed(listed), c2))
		} else {
			Ok((NoBundle, c1))
		}
	}

	## Tag 20.
	activation_words : Words -> Try(List(U64), [Malformed, Unasked(U64)])
	activation_words = |c0| {
		(standard, c1) = take_fact(c0, take_flag)?
		(unavailable, c2) = take_fact(c1, take_flag)?
		(space, c3) = take_fact(c2, take_maybe)?
		(workspace, c4) = take_fact(c3, take_maybe)?
		(active_workspace, c5) = take_fact(c4, take_maybe)?
		(pending_window, c6) = take_fact(c5, take_flag)?
		(mode_refusal, c7) = take_fact(c6, take_maybe)?
		(active_space_fullscreen, c8) = take_fact(c7, take_flag)?
		(unmanageable_source, c9) = take_fact(c8, take_maybe)?
		(blacklisted, c10) = take_fact(c9, take_listing)?
		finished(c10)?
		answer = activation_follow({ standard, unavailable, space, workspace, active_workspace, pending_window, mode_refusal, active_space_fullscreen, unmanageable_source, blacklisted })?
		Ok(answer)
	}

	## Tag 21.
	display_follow_words : Words -> Try(List(U64), [Malformed, Unasked(U64)])
	display_follow_words = |c0| {
		(focused_space, c1) = take_maybe(c0)?
		(previous, c2) = take_maybe(c1)?
		(active, c3) = take_answers(c2, take, take_flag)?
		(on_known_display, c4) = take_answers(c3, take, take_flag)?
		finished(c4)?
		answer = display_to_follow(focused_space, previous, active, on_known_display)?
		Ok(put_maybe([], answer))
	}

	## What is known of a window: a flag and then its layer (as its word) and app.
	take_known : Words -> Taken([Known({ layer : U64, owner : U64 }), Unknown])
	take_known = |c0| {
		(has_known, c1) = take_flag(c0)?
		if has_known {
			(layer, c2) = take(c1)?
			(owner, c3) = take(c2)?
			Ok((Known({ layer, owner }), c3))
		} else {
			Ok((Unknown, c1))
		}
	}

	## Whether to make the untracked window under the cursor key. Facts: 0 whether it is
	## tracked, 1 what is known of it. A layer is echoed as its word: 0 is an application
	## window.
	untracked_focus : Maybe, Fact(Bool), Fact([Known({ layer : U64, owner : U64 }), Unknown]) -> Read(List(U64))
	untracked_focus = |under_cursor, tracked, known|
		match under_cursor {
			Nothing => Ok([0])
			Some(window) => {
				is_tracked = ask(tracked, 0)?
				if is_tracked {
					Ok([0])
				} else {
					known_of = ask(known, 1)?
					match known_of {
						Unknown => Ok([0])
						Known({ layer, owner }) => if (layer == 0) Ok([2, owner, window]) else Ok([1, layer, window])
					}
				}
			}
		}

	## Tag 22.
	untracked_words : Words -> Try(List(U64), [Malformed, Unasked(U64)])
	untracked_words = |c0| {
		(under_cursor, c1) = take_maybe(c0)?
		(tracked, c2) = take_fact(c1, take_flag)?
		(known, c3) = take_fact(c2, take_known)?
		finished(c3)?
		answer = untracked_focus(under_cursor, tracked, known)?
		Ok(answer)
	}

	## Whether to focus a Space's desktop. Fact: 0 whether its workspace is empty.
	focuses_desktop : Bool, Fact(Bool) -> Read(Bool)
	focuses_desktop = |active, workspace_empty| if active ask(workspace_empty, 0) else Ok(False)

	## Tag 23.
	desktop_words : Words -> Try(List(U64), [Malformed, Unasked(U64)])
	desktop_words = |c0| {
		(active, c1) = take_flag(c0)?
		(workspace_empty, c2) = take_fact(c1, take_flag)?
		finished(c2)?
		answer = focuses_desktop(active, workspace_empty)?
		Ok(put_verdict(answer))
	}
}
