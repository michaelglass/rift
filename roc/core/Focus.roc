## Decisions about focus and raising (`rift_core::focus`): what a removal does to focus, the
## window to focus in a workspace, hidden focus, following an activation, and the fallbacks.
## Questions on plain data; Spaces, workspaces, window-server ids, modes and apps are one word
## each, and a window with an app is a WindowId (its app is its pid).

import pf.Boundary exposing [WindowId, MaybeWord, MaybeRefusal, FactRefusal, RefusingAxis, MaybeWindow, Removal, RemovalFocus, FactBool, FactWord, FactWindow, FactWords, FactKnown, FactListing, WordBool, WindowBool, WindowWindow, WordFrame, WordLevels, ActivationQuestion, Follow, FollowTo, UntrackedFocus, Surface, KeyWindow, RaiseQuestion, MouseRaise, Rect]
import Asking exposing [ask, ask_window, ask_word]

## What a decision decided, or the first fact it read that its question does not carry (the
## fact's place among the question's facts): it decides in another order than the platform's.
Read(a) : Try(a, [Unasked(U64)])

Focus := [].{

	## What a removal does to focus. Facts: 0 the removed window's Space, 1 macOS's key window, 2
	## the successor a native window is tracked as, 3 whether a successor is in an inactive
	## workspace.
	removal_focus : Removal, MaybeWindow, FactWord, FactWindow, List(WindowWindow), List(WindowBool) -> Read(RemovalFocus)
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
	focus_candidate : MaybeWord, FactWord, FactWords, List(WordBool) -> Read(MaybeWord)
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
	hides_focus : U64, MaybeWord, MaybeWord, FactBool -> Read(Bool)
	hides_focus = |window, main, focused, in_inactive_workspace| {
		is = |candidate|
			match candidate {
				Some(id) => id == window
				Nothing => False
			}
		if is(main) or is(focused) ask(in_inactive_workspace, 0) else Ok(False)
	}

	## Whether an activation follows its window to its Space and workspace, or why it stays.
	## Facts 0 to 9, in the order they are asked.
	activation_follow : ActivationQuestion -> Read(Follow)
	activation_follow = |facts| {
		standard = ask(facts.standard, 0)?
		if !standard {
			Ok(Stay(NotStandard))
		} else {
			unavailable = ask(facts.unavailable, 1)?
			if unavailable {
				Ok(Stay(Unavailable))
			} else {
				space_of = ask(facts.space, 2)?
				match space_of {
					Nothing => Ok(Stay(NoSpace))
					Some(space) => {
						workspace_of = ask(facts.workspace, 3)?
						match workspace_of {
							Nothing => Ok(Stay(NoWorkspace))
							Some(workspace) => {
								active_of = ask(facts.active_workspace, 4)?
								already =
									match active_of {
										Some(active) => active == workspace
										Nothing => True
									}
								if already {
									Ok(Stay(AlreadyActive))
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
	after_active : ActivationQuestion, U64, U64 -> Read(Follow)
	after_active = |facts, space, workspace| {
		pending = ask(facts.pending_window, 5)?
		if pending {
			Ok(Stay(PendingWindow))
		} else {
			mode_of = ask(facts.mode_refusal, 6)?
			match mode_of {
				Refusing(axis) => Ok(Stay(Mode(axis)))
				NotRefusing => {
					fullscreen = ask(facts.active_space_fullscreen, 7)?
					if fullscreen {
						Ok(Stay(FullscreenActive))
					} else {
						source_of = ask(facts.unmanageable_source, 8)?
						match source_of {
							Some(server) => Ok(Stay(Unmanageable(server)))
							Nothing => {
								listed = ask(facts.blacklisted, 9)?
								match listed {
									NoBundle => Ok(Stay(NoBundleId))
									Listed(True) => Ok(Stay(Blacklisted))
									Listed(False) => Ok(To({ space, workspace }))
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
	display_to_follow : MaybeWord, MaybeWord, List(WordBool), List(WordBool) -> Read(MaybeWord)
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

	## Whether to make the untracked window under the cursor key. Facts: 0 whether it is
	## tracked, 1 what is known of it (its layer and its app): only an application window (layer 0)
	## is made key.
	untracked_focus : MaybeWord, FactBool, FactKnown -> Read(UntrackedFocus)
	untracked_focus = |under_cursor, tracked, known|
		match under_cursor {
			Nothing => Ok(Nothing)
			Some(window) => {
				is_tracked = ask(tracked, 0)?
				if is_tracked {
					Ok(Nothing)
				} else {
					known_of = ask(known, 1)?
					match known_of {
						Unknown => Ok(Nothing)
						Known({ layer, owner }) => if (layer == 0) Ok(MakeKey({ owner, window })) else Ok(SystemSurface({ layer, window }))
					}
				}
			}
		}

	## Whether focus-follows-mouse raises the window under the pointer: a tracked window that is
	## unclaimed and admitted or floating, on an active Space in its active workspace, unless
	## raising it would bury a floating window above it. Facts: 0 claimed, 1 admitted, 2 floating,
	## 3 whether its Space is active, 4 whether it is in the active workspace, 5 whether it could
	## cover a floating window, 6 the Space's stacking order, 7 a floating window's frame, 8 a
	## window's levels.
	mouse_raise : RaiseQuestion -> Read(MouseRaise)
	mouse_raise = |q|
		match q.window {
			Untracked => Ok(Skip(Unknown))
			Hovered(window) => {
				claimed = ask(q.claimed, 0)?
				outside =
					if claimed {
						True
					} else {
						admitted = ask(q.admitted, 1)?
						if admitted {
							False
						} else {
							floating = ask(q.floating, 2)?
							if floating False else True
						}
					}
				if outside {
					Ok(Skip(OutsideAdmission))
				} else {
					match q.space {
						Nothing => Ok(Skip(NoSpace))
						Some(_) => {
							is_active = ask(q.active, 3)?
							if is_active {
								inside = ask(q.in_active_workspace, 4)?
								if inside {
									match window.server {
										Nothing => Ok(Raise)
										Some(candidate) => {
											occludes = ask(q.could_occlude, 5)?
											if occludes {
												order = ask(q.order, 6)?
												buries = raise_buries_a_floater(candidate, window.frame, order, q.floaters, q.levels)?
												if buries Ok(Skip(BuriesFloater)) else Ok(Raise)
											} else {
												Ok(Raise)
											}
										}
									}
								} else {
									Ok(Skip(OutsideActiveWorkspace))
								}
							} else {
								Ok(Skip(InactiveSpace))
							}
						}
					}
				}
			}
		}

	## Whether raising `candidate` (at `frame`) would bury a floating window it covers entirely
	## that is above it at the same level: the windows in front of it in `order`, each a floating
	## window's frame (fact 7), and the candidate's and its levels (fact 8, the candidate's
	## first).
	raise_buries_a_floater : U64, Rect, List(U64), List(WordFrame), List(WordLevels) -> Read(Bool)
	raise_buries_a_floater = |candidate, frame, order, floaters, levels| {
		var $buries = Ok(False)
		var $reached = False
		for above in order {
			if above == candidate {
				$reached = True
			} else {}
			match $buries {
				Ok(False) =>
					if $reached {} else {
						match ask_word(floaters, above, 7) {
							Err(unasked) => { $buries = Err(unasked) }
							Ok(NoFrame) => {}
							Ok(Frame(above_frame)) =>
								if contains(frame, above_frame) {
									match ask_word(levels, candidate, 8) {
										Err(unasked) => { $buries = Err(unasked) }
										Ok(candidate_levels) =>
											match ask_word(levels, above, 8) {
												Err(unasked) => { $buries = Err(unasked) }
												Ok(above_levels) =>
													match candidate_levels {
														Levels(mine) =>
															match above_levels {
																Levels(theirs) => if mine.level == theirs.level and mine.sub_level == theirs.sub_level { $buries = Ok(True) } else {}
																NoLevels => {}
															}
														NoLevels => {}
													}
											}
									}
								} else {}
						}
					}
				_ => {}
			}
		}
		$buries
	}

	## Whether `outer` covers `inner` entirely (edges included).
	contains : Rect, Rect -> Bool
	contains = |outer, inner| outer.x <= inner.x and outer.y <= inner.y and outer.x + outer.w >= inner.x + inner.w and outer.y + outer.h >= inner.y + inner.h

	## Whether to focus a Space's desktop. Fact: 0 whether its workspace is empty.
	focuses_desktop : Bool, FactBool -> Read(Bool)
	focuses_desktop = |active, workspace_empty| if active ask(workspace_empty, 0) else Ok(False)
}
