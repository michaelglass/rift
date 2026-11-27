## Which windows float, and what the layout engine remembers of them (`rift_core::floats`): the
## floating windows, a floating window's fullscreen, each Space's active floating windows, the
## last floating window focused, the engine's focused window, and each workspace's floating
## frames. Windows, pids, Spaces and workspaces are one word each (a window's pid is its word's
## high half; a pid word is sign-extended).

import pf.Boundary exposing [FloatsEvent, FloatFullscreen, FloatFullscreenEntry, FloatSpaceWindow, FloatFrameEntry, MaybeWord, Rect]

## The floating state, all of it.
Kept : { floating : Set(U64), fullscreen : Dict(U64, FloatFullscreen), active : Dict(U64, Set(U64)), floating_focus : MaybeWord, focused : MaybeWord, frames : Dict(U64, Dict(U64, Rect)) }

Floats := [].{

	empty : {} -> Kept
	empty = |{}| { floating: Set.empty(), fullscreen: Dict.empty(), active: Dict.empty(), floating_focus: Nothing, focused: Nothing, frames: Dict.empty() }

	step : Kept, FloatsEvent -> Kept
	step = |kept, event|
		match event {
			Floated(window) => { ..kept, floating: kept.floating.insert(window) }
			Unfloated(window) => {
				deactivated = deactivate_everywhere(kept, window)
				{ ..deactivated, floating: kept.floating.remove(window), fullscreen: kept.fullscreen.remove(window), floating_focus: if (kept.floating_focus == Some(window)) Nothing else kept.floating_focus }
			}
			Fullscreen(set) =>
				match set.kind {
					Fullscreen(kind) => { ..kept, fullscreen: kept.fullscreen.insert(set.window, kind) }
					NotFullscreen => { ..kept, fullscreen: kept.fullscreen.remove(set.window) }
				}
			Activated(at) => { ..kept, active: with_active(kept.active, at.space, |windows| windows.insert(at.window)) }
			Deactivated(at) => { ..kept, active: with_active(kept.active, at.space, |windows| windows.remove(at.window)) }
			DeactivatedEverywhere(window) => deactivate_everywhere(kept, window)
			ActiveRebuilt(rebuilt) => {
				active = rebuilt.windows.fold(Set.empty(), |set, window| if kept.floating.contains(window) set.insert(window) else set)
				{ ..kept, active: kept.active.insert(rebuilt.space, active) }
			}
			SpaceRemapped(pair) =>
				if pair.from == pair.to {
					kept
				} else {
					merged = windows_on(kept, pair.to).union(windows_on(kept, pair.from))
					removed = kept.active.remove(pair.from).remove(pair.to)
					{ ..kept, active: if merged.is_empty() removed else removed.insert(pair.to, merged) }
				}
			FloatingFocus(window) => { ..kept, floating_focus: window }
			Focused(window) => { ..kept, focused: window }
			Replaced(pair) => if pair.from == pair.to kept else replaced(kept, pair.from, pair.to)
			AppQuit(pid) => app_quit(kept, pid)
			FloatingTaken => { ..kept, floating: Set.empty() }
			FrameStored(stored) => {
				store = |frames| if stored.if_absent and frames.contains(stored.window) frames else frames.insert(stored.window, stored.frame)
				{ ..kept, frames: with_frames(kept.frames, stored.workspace, store) }
			}
			FramesForgotten(window) => { ..kept, frames: kept.frames.map(|_, frames| frames.remove(window)) }
			FrameForgotten(at) => { ..kept, frames: with_frames(kept.frames, at.workspace, |frames| frames.remove(at.window)) }
			WorkspaceForgotten(workspace) => { ..kept, frames: kept.frames.remove(workspace) }
		}

	windows_on : Kept, U64 -> Set(U64)
	windows_on = |kept, space|
		match kept.active.get(space) {
			Ok(windows) => windows
			Err(_) => Set.empty()
		}

	## `space`'s active windows changed by `change`, in place: taken out of the table first, so
	## the table no longer shares them (a field bound while its record stays live is copied).
	with_active : Dict(U64, Set(U64)), U64, (Set(U64) -> Set(U64)) -> Dict(U64, Set(U64))
	with_active = |active, space, change|
		match active.get(space) {
			Ok(windows) => {
				rest = active.remove(space)
				rest.insert(space, change(windows))
			}
			Err(_) => active.insert(space, change(Set.empty()))
		}

	## `workspace`'s frames changed by `change`, in place, as `with_active`.
	with_frames : Dict(U64, Dict(U64, Rect)), U64, (Dict(U64, Rect) -> Dict(U64, Rect)) -> Dict(U64, Dict(U64, Rect))
	with_frames = |frames, workspace, change|
		match frames.get(workspace) {
			Ok(in_ws) => {
				rest = frames.remove(workspace)
				rest.insert(workspace, change(in_ws))
			}
			Err(_) => frames.insert(workspace, change(Dict.empty()))
		}

	deactivate_everywhere : Kept, U64 -> Kept
	deactivate_everywhere = |kept, window| { ..kept, active: kept.active.map(|_, windows| windows.remove(window)) }

	replaced : Kept, U64, U64 -> Kept
	replaced = |kept, from, to| {
		was_floating = kept.floating.contains(from)
		floating0 = kept.floating.remove(from).remove(to)
		floating = if was_floating floating0.insert(to) else floating0
		fullscreen0 = kept.fullscreen.remove(from).remove(to)
		fullscreen =
			match kept.fullscreen.get(from) {
				Ok(kind) => fullscreen0.insert(to, kind)
				Err(_) => fullscreen0
			}
		active = kept.active.map(|_, windows| if windows.contains(from) windows.remove(from).remove(to).insert(to) else windows.remove(to))
		floating_focus = if (kept.floating_focus == Some(from)) Some(to) else kept.floating_focus
		frames = kept.frames.map(
			|_, in_ws|
				match in_ws.get(from) {
					Ok(frame) => in_ws.remove(from).remove(to).insert(to, frame)
					Err(_) => in_ws.remove(to)
				},
		)
		{ ..kept, floating, fullscreen, active, floating_focus, frames }
	}

	## `window`'s app is `pid`: its word's high half is the pid's low 32 bits.
	of_app : U64, U64 -> Bool
	of_app = |window, pid| U64.div_trunc_by(window, 4294967296) == pid % 4294967296

	app_quit : Kept, U64 -> Kept
	app_quit = |kept, pid| {
		other = |window| !of_app(window, pid)
		{
			..kept,
			floating: kept.floating.keep_if(other),
			fullscreen: kept.fullscreen.keep_if(|(window, _)| other(window)),
			active: kept.active.map(|_, windows| windows.keep_if(other)),
			floating_focus:
				match kept.floating_focus {
					Some(window) => if other(window) kept.floating_focus else Nothing
					Nothing => Nothing
				},
			frames: kept.frames.map(|_, in_ws| in_ws.keep_if(|(window, _)| other(window))),
		}
	}

	ascending : (U64, a), (U64, a) -> [Before, Same, After]
	ascending = |(a, _), (b, _)| if a < b Before else if a > b After else Same

	## The floating windows, in word order.
	floating_list : Kept -> List(U64)
	floating_list = |kept| kept.floating.to_list().sort_with(|a, b| if a < b Before else if a > b After else Same)

	fullscreens : Kept -> List(FloatFullscreenEntry)
	fullscreens = |kept| kept.fullscreen.to_list().sort_with(ascending).map(|(window, kind)| FloatFullscreenEntry.{ window, kind })

	## Every active floating window, by Space then window.
	active_list : Kept -> List(FloatSpaceWindow)
	active_list = |kept|
		kept.active
			.to_list()
			.sort_with(ascending)
			.join_map(|(space, windows)| windows.to_list().sort_with(|a, b| if a < b Before else if a > b After else Same).map(|window| FloatSpaceWindow.{ space, window }))

	## Every floating frame, by workspace then window.
	frames_list : Kept -> List(FloatFrameEntry)
	frames_list = |kept|
		kept.frames
			.to_list()
			.sort_with(ascending)
			.join_map(|(workspace, in_ws)| in_ws.to_list().sort_with(ascending).map(|(window, frame)| FloatFrameEntry.{ workspace, window, frame }))
}
