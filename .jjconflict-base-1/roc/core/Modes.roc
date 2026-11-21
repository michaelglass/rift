## The mode table (`rift_core::modes`): every mode Rift can be in sits on one independent axis,
## and an initiative goes ahead only when every axis admits it. The verdict names the first axis,
## in the table's order, that refuses. Each axis's rule is an exhaustive match, no wildcard.

## What user input does, as far as the table needs.
InputClass : [Steering, Arranging, Inert, Hover]

## Things Rift would do that a mode can refuse.
Initiative : [UserInput(InputClass), Arrange, RetireWindows, SwitchWorkspaceFocus, WarpFocus, FollowActivation]

## What the pointer is doing with windows (a window is one word).
Pointer : [Idle, Pressing(U64), Dragging(U64)]

## The axis that refused: every axis but the pointer refuses in one state only.
Axis : [Pointer(Pointer), Menu, MissionControl, Modal, Viewport, FocusClaim, UserSpaces, Wake, WorkspaceSwitch, Topology]

## The modes Rift is in, one per axis.
ModesNow : {
	pointer : Pointer,
	menu : [Closed, Open],
	mission_control : [Inactive, Active],
	modal : [Clear, Frontmost],
	viewport : [Free, Gesturing],
	focus_claim : [Free, PausesHover],
	user_spaces : [Shown, None],
	wake : [UserPresent, AwaitingInput],
	workspace_switch : [Idle, Switching],
	topology : [Settled, Unsettled],
}

Modes := [].{
	## The table's verdict on `initiative`: admitted, or the first axis that refuses it.
	verdict : ModesNow, Initiative -> [Admitted, Refused(Axis)]
	verdict = |m, initiative| {
		refusals = [
			if (may(initiative, m.pointer)) Admitted else Refused(Pointer(m.pointer)),
			if (refuses_menu(m.menu, initiative)) Refused(Menu) else Admitted,
			if (refuses_mission_control(m.mission_control, initiative)) Refused(MissionControl) else Admitted,
			if (refuses_modal(m.modal, initiative)) Refused(Modal) else Admitted,
			if (refuses_viewport(m.viewport, initiative)) Refused(Viewport) else Admitted,
			if (refuses_focus_claim(m.focus_claim, initiative)) Refused(FocusClaim) else Admitted,
			if (refuses_user_spaces(m.user_spaces, initiative)) Refused(UserSpaces) else Admitted,
			if (refuses_wake(m.wake, initiative)) Refused(Wake) else Admitted,
			if (refuses_workspace_switch(m.workspace_switch, initiative)) Refused(WorkspaceSwitch) else Admitted,
			if (refuses_topology(m.topology, initiative)) Refused(Topology) else Admitted,
		]
		var $verdict = Admitted
		for refusal in refusals {
			match ($verdict, refusal) {
				(Admitted, Refused(axis)) => {
					$verdict = Refused(axis)
				}
				_ => {}
			}
		}
		$verdict
	}

	## The pointer's row (`rift_core::may`).
	may : Initiative, Pointer -> Bool
	may = |initiative, pointer|
		match (initiative, pointer) {
			(UserInput(Inert), Idle) => True
			(UserInput(Inert), Pressing(_)) => True
			(UserInput(Inert), Dragging(_)) => True
			(UserInput(Steering), Idle) => True
			(UserInput(Arranging), Idle) => True
			(UserInput(Hover), Idle) => True
			(UserInput(Steering), Pressing(_)) => False
			(UserInput(Steering), Dragging(_)) => False
			(UserInput(Arranging), Pressing(_)) => False
			(UserInput(Arranging), Dragging(_)) => False
			(UserInput(Hover), Pressing(_)) => False
			(UserInput(Hover), Dragging(_)) => False
			(Arrange, Idle) => True
			(Arrange, Pressing(_)) => True
			(Arrange, Dragging(_)) => False
			(RetireWindows, Idle) => True
			(RetireWindows, Pressing(_)) => True
			(RetireWindows, Dragging(_)) => False
			(SwitchWorkspaceFocus, Idle) => True
			(SwitchWorkspaceFocus, Pressing(_)) => True
			(SwitchWorkspaceFocus, Dragging(_)) => False
			(WarpFocus, Idle) => True
			(WarpFocus, Pressing(_)) => True
			(WarpFocus, Dragging(_)) => False
			(FollowActivation, Idle) => True
			(FollowActivation, Pressing(_)) => False
			(FollowActivation, Dragging(_)) => False
		}

	## Passing over windows (focus follows mouse, raise on hover).
	hover : Initiative -> Bool
	hover = |initiative|
		match initiative {
			UserInput(Hover) => True
			UserInput(Steering) => False
			UserInput(Arranging) => False
			UserInput(Inert) => False
			Arrange => False
			RetireWindows => False
			SwitchWorkspaceFocus => False
			WarpFocus => False
			FollowActivation => False
		}

	## Retiring windows an inventory no longer reports.
	retire : Initiative -> Bool
	retire = |initiative|
		match initiative {
			RetireWindows => True
			UserInput(Steering) => False
			UserInput(Arranging) => False
			UserInput(Inert) => False
			UserInput(Hover) => False
			Arrange => False
			SwitchWorkspaceFocus => False
			WarpFocus => False
			FollowActivation => False
		}

	## Following an app's activation with focus.
	follow_activation : Initiative -> Bool
	follow_activation = |initiative|
		match initiative {
			FollowActivation => True
			UserInput(Steering) => False
			UserInput(Arranging) => False
			UserInput(Inert) => False
			UserInput(Hover) => False
			Arrange => False
			RetireWindows => False
			SwitchWorkspaceFocus => False
			WarpFocus => False
		}

	## The user's layout commands and hover, which act on Space assignments.
	layout_input : Initiative -> Bool
	layout_input = |initiative|
		match initiative {
			UserInput(Arranging) => True
			UserInput(Hover) => True
			UserInput(Steering) => False
			UserInput(Inert) => False
			Arrange => False
			RetireWindows => False
			SwitchWorkspaceFocus => False
			WarpFocus => False
			FollowActivation => False
		}

	refuses_menu : [Closed, Open], Initiative -> Bool
	refuses_menu = |menu, initiative|
		match menu {
			Open => hover(initiative)
			Closed => False
		}

	refuses_mission_control : [Inactive, Active], Initiative -> Bool
	refuses_mission_control = |mission_control, initiative|
		match mission_control {
			Active => hover(initiative) or retire(initiative)
			Inactive => False
		}

	refuses_modal : [Clear, Frontmost], Initiative -> Bool
	refuses_modal = |modal, initiative|
		match modal {
			Frontmost => hover(initiative)
			Clear => False
		}

	refuses_viewport : [Free, Gesturing], Initiative -> Bool
	refuses_viewport = |viewport, initiative|
		match viewport {
			Gesturing => hover(initiative)
			Free => False
		}

	refuses_focus_claim : [Free, PausesHover], Initiative -> Bool
	refuses_focus_claim = |focus_claim, initiative|
		match focus_claim {
			PausesHover => hover(initiative)
			Free => False
		}

	refuses_user_spaces : [Shown, None], Initiative -> Bool
	refuses_user_spaces = |user_spaces, initiative|
		match user_spaces {
			None => retire(initiative)
			Shown => False
		}

	refuses_wake : [UserPresent, AwaitingInput], Initiative -> Bool
	refuses_wake = |wake, initiative|
		match wake {
			AwaitingInput => follow_activation(initiative)
			UserPresent => False
		}

	refuses_workspace_switch : [Idle, Switching], Initiative -> Bool
	refuses_workspace_switch = |workspace_switch, initiative|
		match workspace_switch {
			Switching => follow_activation(initiative)
			Idle => False
		}

	refuses_topology : [Settled, Unsettled], Initiative -> Bool
	refuses_topology = |topology, initiative|
		match topology {
			Unsettled => layout_input(initiative)
			Settled => False
		}
}
