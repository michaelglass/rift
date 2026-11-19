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

Focus := [].{

	## What a removal does to focus.
	removal_focus : [AppClosed(I32), WindowRemoved(WindowId)], MaybeWindow, Maybe, MaybeWindow, List({ from : WindowId, to : WindowId }), List(WindowId) -> [Unaffected, KeepNative(WindowId), Refocus(U64)]
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
			NoWindow => Unaffected
			SomeWindow(window) =>
				match removed_space {
					Nothing => Unaffected
					Some(space) =>
						match removal {
							AppClosed(_) => Refocus(space)
							WindowRemoved(_) =>
								match native_focus {
									SomeWindow(native) =>
										if native.pid == window.pid and native != window {
											successor = successor_of(successors, native)
											if successor != window and !inactive.contains(successor) {
												KeepNative(successor)
											} else {
												Refocus(space)
											}
										} else {
											Refocus(space)
										}
									NoWindow => Refocus(space)
								}
						}
				}
		}
	}

	successor_of : List({ from : WindowId, to : WindowId }), WindowId -> WindowId
	successor_of = |successors, native| {
		var $found = NoWindow
		for entry in successors {
			match $found {
				NoWindow => if (entry.from == native) { $found = SomeWindow(entry.to) } else {}
				SomeWindow(_) => {}
			}
		}
		match $found {
			SomeWindow(to) => to
			NoWindow => native
		}
	}

	## The window to focus: preferred, else last focused, else the first of the workspace's,
	## whichever is visible first.
	focus_candidate : Maybe, Maybe, List(U64), List(U64) -> Maybe
	focus_candidate = |preferred, last_focused, in_workspace, visible| {
		visible_one = |candidate|
			match candidate {
				Some(window) => if (visible.contains(window)) Some(window) else Nothing
				Nothing => Nothing
			}
		match visible_one(preferred) {
			Some(window) => Some(window)
			Nothing =>
				match visible_one(last_focused) {
					Some(window) => Some(window)
					Nothing => {
						var $found = Nothing
						for window in in_workspace {
							match $found {
								Nothing => if (visible.contains(window)) { $found = Some(window) } else {}
								Some(_) => {}
							}
						}
						$found
					}
				}
		}
	}

	## Whether `window`, the main or the focused one, is hidden in an inactive workspace.
	hides_focus : U64, Maybe, Maybe, Bool -> Bool
	hides_focus = |window, main, focused, in_inactive_workspace| {
		is = |candidate|
			match candidate {
				Some(id) => id == window
				Nothing => False
			}
		(is(main) or is(focused)) and in_inactive_workspace
	}

	## Whether an activation follows its window: 0 with the Space and workspace, or 1 with the
	## reason (and its word, for a mode or an unmanageable source).
	activation_follow : { standard : Bool, unavailable : Bool, space : Maybe, workspace : Maybe, active_workspace : Maybe, pending_window : Bool, mode_refusal : Maybe, active_space_fullscreen : Bool, unmanageable_source : Maybe, blacklisted : [Listed(Bool), NoBundle] } -> List(U64)
	activation_follow = |facts|
		if !facts.standard {
			[1, 0]
		} else if facts.unavailable {
			[1, 1]
		} else {
			match facts.space {
				Nothing => [1, 2]
				Some(space) =>
					match facts.workspace {
						Nothing => [1, 3]
						Some(workspace) => {
							already =
								match facts.active_workspace {
									Some(active) => active == workspace
									Nothing => True
								}
							if already {
								[1, 4]
							} else if facts.pending_window {
								[1, 5]
							} else {
								match facts.mode_refusal {
									Some(mode) => [1, 6, mode]
									Nothing =>
										if facts.active_space_fullscreen {
											[1, 7]
										} else {
											match facts.unmanageable_source {
												Some(server) => [1, 8, server]
												Nothing =>
													match facts.blacklisted {
														NoBundle => [1, 9]
														Listed(True) => [1, 10]
														Listed(False) => [0, space, workspace]
													}
											}
										}
								}
							}
						}
					}
			}
		}

	## The display to activate after a command moved the focused window.
	display_to_follow : Maybe, Maybe, List(U64), List(U64) -> Maybe
	display_to_follow = |focused_space, previous, active, on_known_display|
		match focused_space {
			Some(space) => {
				changed =
					match previous {
						Some(before) => before != space
						Nothing => True
					}
				if changed and active.contains(space) and on_known_display.contains(space) Some(space) else Nothing
			}
			Nothing => Nothing
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

	put_maybe : List(U64), Maybe -> List(U64)
	put_maybe = |words, maybe|
		match maybe {
			Some(word) => words.append(1).append(word)
			Nothing => words.append(0)
		}

	put_verdict : Bool -> List(U64)
	put_verdict = |verdict| if (verdict) [1] else [0]

	## Tag 17.
	removal_words : Words -> Try(List(U64), [Malformed])
	removal_words = |c0| {
		(kind, c1) = take(c0)?
		(removal, c2) =
			match kind {
				0 => take_pid(c1).map_ok(|(pid, rest)| (AppClosed(pid), rest))
				1 => take_window(c1).map_ok(|(window, rest)| (WindowRemoved(window), rest))
				_ => Err(Malformed)
			}?
		(focused, c3) = take_maybe_window(c2)?
		(removed_space, c4) = take_maybe(c3)?
		(native_focus, c5) = take_maybe_window(c4)?
		take_pair = |c| {
			(from, after) = take_window(c)?
			(to, rest) = take_window(after)?
			Ok(({ from, to }, rest))
		}
		(successors, c6) = take_list(c5, take_pair)?
		(inactive, c7) = take_list(c6, take_window)?
		finished(c7)?
		Ok(
			match removal_focus(removal, focused, removed_space, native_focus, successors, inactive) {
				Unaffected => [0]
				KeepNative(window) => put_window([1], window)
				Refocus(space) => [2, space]
			},
		)
	}

	## Tag 18.
	candidate_words : Words -> Try(List(U64), [Malformed])
	candidate_words = |c0| {
		(preferred, c1) = take_maybe(c0)?
		(last_focused, c2) = take_maybe(c1)?
		(in_workspace, c3) = take_list(c2, take)?
		(visible, c4) = take_list(c3, take)?
		finished(c4)?
		Ok(put_maybe([], focus_candidate(preferred, last_focused, in_workspace, visible)))
	}

	## Tag 19.
	hides_words : Words -> Try(List(U64), [Malformed])
	hides_words = |c0| {
		(window, c1) = take(c0)?
		(main, c2) = take_maybe(c1)?
		(focused, c3) = take_maybe(c2)?
		(in_inactive_workspace, c4) = take_flag(c3)?
		finished(c4)?
		Ok(put_verdict(hides_focus(window, main, focused, in_inactive_workspace)))
	}

	## Tag 20.
	activation_words : Words -> Try(List(U64), [Malformed])
	activation_words = |c0| {
		(standard, c1) = take_flag(c0)?
		(unavailable, c2) = take_flag(c1)?
		(space, c3) = take_maybe(c2)?
		(workspace, c4) = take_maybe(c3)?
		(active_workspace, c5) = take_maybe(c4)?
		(pending_window, c6) = take_flag(c5)?
		(mode_refusal, c7) = take_maybe(c6)?
		(active_space_fullscreen, c8) = take_flag(c7)?
		(unmanageable_source, c9) = take_maybe(c8)?
		(has_bundle, c10) = take_flag(c9)?
		(blacklisted, c11) =
			if has_bundle {
				(listed, rest) = take_flag(c10)?
				(Listed(listed), rest)
			} else {
				(NoBundle, c10)
			}
		finished(c11)?
		Ok(activation_follow({ standard, unavailable, space, workspace, active_workspace, pending_window, mode_refusal, active_space_fullscreen, unmanageable_source, blacklisted }))
	}

	## Tag 21.
	display_follow_words : Words -> Try(List(U64), [Malformed])
	display_follow_words = |c0| {
		(focused_space, c1) = take_maybe(c0)?
		(previous, c2) = take_maybe(c1)?
		(active, c3) = take_list(c2, take)?
		(on_known_display, c4) = take_list(c3, take)?
		finished(c4)?
		Ok(put_maybe([], display_to_follow(focused_space, previous, active, on_known_display)))
	}

	## Tag 22. A layer is echoed as its word: 0 is an application window.
	untracked_words : Words -> Try(List(U64), [Malformed])
	untracked_words = |c0| {
		(under_cursor, c1) = take_maybe(c0)?
		(tracked, c2) = take_flag(c1)?
		(has_known, c3) = take_flag(c2)?
		(known, c4) =
			if has_known {
				(layer, after) = take(c3)?
				(owner, rest) = take(after)?
				(Known({ layer, owner }), rest)
			} else {
				(Unknown, c3)
			}
		finished(c4)?
		Ok(
			match under_cursor {
				Nothing => [0]
				Some(window) =>
					if tracked {
						[0]
					} else {
						match known {
							Unknown => [0]
							Known({ layer, owner }) => if (layer == 0) [2, owner, window] else [1, layer, window]
						}
					}
			},
		)
	}

	## Tag 23.
	desktop_words : Words -> Try(List(U64), [Malformed])
	desktop_words = |c0| {
		(active, c1) = take_flag(c0)?
		(workspace_empty, c2) = take_flag(c1)?
		finished(c2)?
		Ok(put_verdict(active and workspace_empty))
	}
}
