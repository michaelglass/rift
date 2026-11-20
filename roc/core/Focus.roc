## Decisions about focus and raising (`rift_core::focus`): what a removal does to focus, the
## window to focus in a workspace, hidden focus, following an activation, and the fallbacks.
## Questions on plain data; Spaces, workspaces, window-server ids, modes and apps are one word
## each, and a window with an app is a WindowId (its app is its pid).

import Asking exposing [ask, ask_window, ask_word]

WindowId : { pid : I32, idx : U32 }

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

## Whether an activation follows its window, and where, or why not.
Follow : [To({ space : U64, workspace : U64 }), Stay([NotStandard, Unavailable, NoSpace, NoWorkspace, AlreadyActive, PendingWindow, Mode(U64), FullscreenActive, Unmanageable(U64), NoBundleId, Blacklisted])]

Focus := [].{

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

	## Whether an activation follows its window to its Space and workspace, or why it stays.
	## Facts 0 to 9, in the order they are asked.
	activation_follow : { standard : Fact(Bool), unavailable : Fact(Bool), space : Fact(Maybe), workspace : Fact(Maybe), active_workspace : Fact(Maybe), pending_window : Fact(Bool), mode_refusal : Fact(Maybe), active_space_fullscreen : Fact(Bool), unmanageable_source : Fact(Maybe), blacklisted : Fact([Listed(Bool), NoBundle]) } -> Read(Follow)
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
	after_active : { standard : Fact(Bool), unavailable : Fact(Bool), space : Fact(Maybe), workspace : Fact(Maybe), active_workspace : Fact(Maybe), pending_window : Fact(Bool), mode_refusal : Fact(Maybe), active_space_fullscreen : Fact(Bool), unmanageable_source : Fact(Maybe), blacklisted : Fact([Listed(Bool), NoBundle]) }, U64, U64 -> Read(Follow)
	after_active = |facts, space, workspace| {
		pending = ask(facts.pending_window, 5)?
		if pending {
			Ok(Stay(PendingWindow))
		} else {
			mode_of = ask(facts.mode_refusal, 6)?
			match mode_of {
				Some(mode) => Ok(Stay(Mode(mode)))
				Nothing => {
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

	## Whether to make the untracked window under the cursor key. Facts: 0 whether it is
	## tracked, 1 what is known of it (its layer and its app): only an application window (layer 0)
	## is made key.
	untracked_focus : Maybe, Fact(Bool), Fact([Known({ layer : I32, owner : U64 }), Unknown]) -> Read([Nothing, SystemSurface({ layer : I32, window : U64 }), MakeKey({ owner : U64, window : U64 })])
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

	## Whether to focus a Space's desktop. Fact: 0 whether its workspace is empty.
	focuses_desktop : Bool, Fact(Bool) -> Read(Bool)
	focuses_desktop = |active, workspace_empty| if active ask(workspace_empty, 0) else Ok(False)
}
