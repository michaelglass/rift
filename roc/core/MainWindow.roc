## Which window is the main one (`rift_core::main_window`): the frontmost app's, as the window
## server reports focus or, before its first report, as AX activation and main-window events say
## (a cold start). A step answers the window to focus, if the event makes one known. Pids and
## windows are one word each (a pid sign-extended).

import pf.Boundary exposing [MaybeWord, MainActivation, Initiator, AppStanding, NativeFocus, MainWindowEvent, MainApp]

## One app as the machine knows it: its standing, who started its latest activation, and its
## main window.
App : { standing : AppStanding, last_activation : Initiator, main_window : MaybeWord }

## Every app (by pid), the globally frontmost app and the native focus: what the window server
## has said about focus (nothing yet, or the window it last reported focused, and its app, while
## that window exists; before its first answer, AX activation and main-window events stand in for
## it, after it they are metadata only).
Mains : { apps : Dict(U64, App), global_frontmost : MaybeWord, native_focus : NativeFocus }

## What the platform observes that the machine reads.
Event : MainWindowEvent

## Whether an event can answer a window to focus: an AX event of an app the machine knows, and
## who started it; every other event is quiet.
Asking : [Quiet, Asks(MainActivation)]

MainWindow := [].{

	empty : {} -> Mains
	empty = |{}| { apps: Dict.empty(), global_frontmost: Nothing, native_focus: NotYetReported }

	## Applies `event`; the window to focus, if the event makes one known. Only a cold start
	## answers one (before the window server's first focus report): an AX event of the
	## globally frontmost app that Rift did not start names its main window.
	step : Mains, Event -> (Mains, MaybeWord)
	step = |mains, event| {
		(after, asking) = apply(mains, event)
		match asking {
			Quiet => (after, Nothing)
			Asks(asked) => (after, cold_start(after, asked.pid, asked.initiator))
		}
	}

	## What `event` leaves, and whether it can answer.
	apply : Mains, Event -> (Mains, Asking)
	apply = |mains, event|
		match event {
			Launched(launch) => {
				known = { standing: launch.standing, last_activation: Other, main_window: launch.main_window }
				({ ..mains, apps: mains.apps.insert(launch.pid, known) }, Asks({ pid: launch.pid, initiator: Other }))
			}
			Terminated(pid) => {
				native_focus =
					match mains.native_focus {
						Reported(Window(focused)) => if (focused.pid == pid) Reported(NoWindow) else mains.native_focus
						_ => mains.native_focus
					}
				({ ..mains, apps: mains.apps.remove(pid), native_focus }, Quiet)
			}
			Activated(activation) =>
				match mains.apps.get(activation.pid) {
					Ok(known) => {
						activated = { ..known, standing: Frontmost, last_activation: activation.initiator }
						({ ..mains, apps: mains.apps.insert(activation.pid, activated) }, Asks(activation))
					}
					Err(_) => (mains, Quiet)
				}
			Deactivated(pid) => ({ ..mains, apps: with_standing(mains.apps, pid, Background) }, Quiet)
			GloballyActivated(pid) => {
				fronted = { ..mains, global_frontmost: Some(pid) }
				match mains.apps.get(pid) {
					Ok(known) => ({ ..fronted, apps: fronted.apps.insert(pid, { ..known, standing: Frontmost }) }, Asks({ pid, initiator: known.last_activation }))
					Err(_) => (fronted, Quiet)
				}
			}
			GloballyDeactivated(pid) => {
				global_frontmost = if (mains.global_frontmost == Some(pid)) Nothing else mains.global_frontmost
				({ ..mains, global_frontmost, apps: with_standing(mains.apps, pid, Background) }, Quiet)
			}
			MainWindowChanged(changed) =>
				match mains.apps.get(changed.pid) {
					Ok(known) => ({ ..mains, apps: mains.apps.insert(changed.pid, { ..known, main_window: changed.window }) }, Asks({ pid: changed.pid, initiator: changed.initiator }))
					Err(_) => (mains, Quiet)
				}
			NativeFocused(focused) => ({ ..mains, native_focus: Reported(Window(focused)) }, Quiet)
			Replaced(replaced) => {
				focused_on =
					match mains.native_focus {
						Reported(Window(focused)) => focused.pid == replaced.pid and focused.window == replaced.window
						_ => False
					}
				native_focus =
					if focused_on {
						match replaced.replacement {
							Some(window) => Reported(Window({ pid: replaced.pid, window }))
							Nothing => Reported(NoWindow)
						}
					} else {
						mains.native_focus
					}
				apps =
					match mains.apps.get(replaced.pid) {
						Ok(known) => if (known.main_window == Some(replaced.window)) mains.apps.insert(replaced.pid, { ..known, main_window: replaced.replacement }) else mains.apps
						Err(_) => mains.apps
					}
				({ ..mains, native_focus, apps }, Quiet)
			}
		}

	## The apps with `pid`'s standing set, if it is known.
	with_standing : Dict(U64, App), U64, AppStanding -> Dict(U64, App)
	with_standing = |apps, pid, standing|
		match apps.get(pid) {
			Ok(known) => apps.insert(pid, { ..known, standing })
			Err(_) => apps
		}

	## Once the window server has reported focus, AX activation and main-window events are
	## metadata and the cold-start fallback only: answering from them could replay the previous
	## native focus while the new one is still being resolved.
	cold_start : Mains, U64, Initiator -> MaybeWord
	cold_start = |mains, pid, initiator|
		match mains.native_focus {
			Reported(_) => Nothing
			NotYetReported => {
				others =
					match initiator {
						Other => True
						Rift => False
					}
				if (mains.global_frontmost == Some(pid) and others) main_window(mains) else Nothing
			}
		}

	## The main window: the window server's focus if it belongs to the globally frontmost app,
	## else that app's main window while it is frontmost.
	main_window : Mains -> MaybeWord
	main_window = |mains|
		match mains.global_frontmost {
			Nothing => Nothing
			Some(pid) => {
				native =
					match mains.native_focus {
						Reported(Window(focused)) => if (focused.pid == pid) Some(focused.window) else Nothing
						_ => Nothing
					}
				match native {
					Some(window) => Some(window)
					Nothing =>
						match mains.apps.get(pid) {
							Ok(known) =>
								match known.standing {
									Frontmost => known.main_window
									Background => Nothing
								}
							Err(_) => Nothing
						}
				}
			}
		}

	## Every app and its state, in pid order.
	apps_in_order : Mains -> List(MainApp)
	apps_in_order = |mains|
		mains.apps
			.to_list()
			.sort_with(|(a, _), (b, _)| if a < b Before else if a > b After else Same)
			.map(|(pid, known)| { pid, standing: known.standing, last_activation: known.last_activation, main_window: known.main_window })
}
