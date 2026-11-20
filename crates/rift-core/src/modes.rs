//! The mode table. Every mode Rift can be in sits on one independent axis, and each axis states,
//! for every initiative (Rift's own, and the user's input by class), whether it admits it. An
//! initiative goes ahead only when every axis admits it ("deny if any active mode denies").
//! What is refused is discarded, never queued: when a mode ends, Rift acts on the facts as they
//! are then, it does not replay what it refused. The grid is in rift's `docs/modes.md`.
//!
//! Axes are a product, not a set of flags: each axis is one enum, so a combination that makes no
//! sense within an axis (a menu open for nobody, a gesture both held and released) cannot be
//! written. The platform builds [`Modes`] fresh from its facts for each question, never stored,
//! so it cannot drift from them.

use crate::pointer::{Initiative, InputClass, Pointer, may};

/// An app's menu is open (it has the pointer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    Closed,
    Open,
}

/// macOS's Mission Control is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissionControl {
    Inactive,
    Active,
}

/// A sheet or popover of the frontmost app is showing on an active Space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modal {
    Clear,
    Frontmost,
}

/// A viewport (scrolling layout) gesture is under the user's fingers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Viewport {
    Free,
    Gesturing,
}

/// The focused window is claimed by an external manager that asked for focus follows mouse to
/// pause while it is focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusClaim {
    Free,
    PausesHover,
}

/// Whether any screen shows a user Space. `None` with no displays at all (asleep, lid closed)
/// and while every screen shows a system or fullscreen Space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserSpaces {
    Shown,
    None,
}

/// Whether the user has given input since the machine woke or the session became active.
/// Until they have, app activations are loginwindow handing focus back, not requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    UserPresent,
    AwaitingInput,
}

/// A workspace switch is in progress: Rift's own (still settling) or the user's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceSwitch {
    Idle,
    Switching,
}

/// Whether Rift's model of Spaces is authoritative: `Unsettled` while no authoritative sample
/// is accepted or a topology invalidation is newer than it. Window reports are quarantined
/// before they reach the table; this axis covers the user's input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topology {
    Settled,
    Unsettled,
}

/// The modes Rift is in right now, one per axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modes<W> {
    pub pointer: Pointer<W>,
    pub menu: Menu,
    pub mission_control: MissionControl,
    pub modal: Modal,
    pub viewport: Viewport,
    pub focus_claim: FocusClaim,
    pub user_spaces: UserSpaces,
    pub wake: Wake,
    pub workspace_switch: WorkspaceSwitch,
    pub topology: Topology,
}

/// The axis that refused an initiative. Every axis but the pointer refuses in one state only
/// (its mode is on), so naming the axis names the state; the pointer's says what the hand does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis<W> {
    Pointer(Pointer<W>),
    Menu,
    MissionControl,
    Modal,
    Viewport,
    FocusClaim,
    UserSpaces,
    Wake,
    WorkspaceSwitch,
    Topology,
}

/// What the table says of an initiative: every axis admits it, or the first that refuses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict<W> {
    Admitted,
    Refused(Axis<W>),
}

impl<W> Modes<W> {
    /// The same modes, the pointer's window named by `f`.
    pub fn map<V>(self, f: impl FnOnce(W) -> V) -> Modes<V> {
        Modes {
            pointer: self.pointer.map(f),
            menu: self.menu,
            mission_control: self.mission_control,
            modal: self.modal,
            viewport: self.viewport,
            focus_claim: self.focus_claim,
            user_spaces: self.user_spaces,
            wake: self.wake,
            workspace_switch: self.workspace_switch,
            topology: self.topology,
        }
    }
}

impl<W> Verdict<W> {
    /// The same verdict, a refusing pointer's window named by `f`.
    pub fn map<V>(self, f: impl FnOnce(W) -> V) -> Verdict<V> {
        match self {
            Verdict::Admitted => Verdict::Admitted,
            Verdict::Refused(axis) => Verdict::Refused(match axis {
                Axis::Pointer(pointer) => Axis::Pointer(pointer.map(f)),
                Axis::Menu => Axis::Menu,
                Axis::MissionControl => Axis::MissionControl,
                Axis::Modal => Axis::Modal,
                Axis::Viewport => Axis::Viewport,
                Axis::FocusClaim => Axis::FocusClaim,
                Axis::UserSpaces => Axis::UserSpaces,
                Axis::Wake => Axis::Wake,
                Axis::WorkspaceSwitch => Axis::WorkspaceSwitch,
                Axis::Topology => Axis::Topology,
            }),
        }
    }
}

impl<W: Copy> Modes<W> {
    /// The table's verdict on `initiative`: the first axis, in the table's order, that refuses
    /// it.
    pub fn verdict(&self, initiative: Initiative) -> Verdict<W> {
        // Destructured so that a new axis does not compile until it is listed here.
        let Modes {
            pointer,
            menu,
            mission_control,
            modal,
            viewport,
            focus_claim,
            user_spaces,
            wake,
            workspace_switch,
            topology,
        } = *self;
        let refusals = [
            (!may(initiative, pointer)).then_some(Axis::Pointer(pointer)),
            refuses_menu(menu, initiative).then_some(Axis::Menu),
            refuses_mission_control(mission_control, initiative).then_some(Axis::MissionControl),
            refuses_modal(modal, initiative).then_some(Axis::Modal),
            refuses_viewport(viewport, initiative).then_some(Axis::Viewport),
            refuses_focus_claim(focus_claim, initiative).then_some(Axis::FocusClaim),
            refuses_user_spaces(user_spaces, initiative).then_some(Axis::UserSpaces),
            refuses_wake(wake, initiative).then_some(Axis::Wake),
            refuses_workspace_switch(workspace_switch, initiative).then_some(Axis::WorkspaceSwitch),
            refuses_topology(topology, initiative).then_some(Axis::Topology),
        ];
        match refusals.into_iter().flatten().next() {
            Some(axis) => Verdict::Refused(axis),
            None => Verdict::Admitted,
        }
    }
}

use Initiative::*;
use InputClass::{Arranging, Hover, Inert, Steering};

// The refusal sets: each says, for every initiative, whether an axis that refuses that set
// refuses it. No wildcard, so a new initiative does not compile until each set decides.

/// Passing over windows (focus follows mouse, raise on hover).
fn hover(initiative: Initiative) -> bool {
    match initiative {
        UserInput(Hover) => true,
        UserInput(Steering | Arranging | Inert)
        | Arrange
        | RetireWindows
        | SwitchWorkspaceFocus
        | WarpFocus
        | FollowActivation => false,
    }
}

/// Retiring windows an inventory no longer reports.
fn retire(initiative: Initiative) -> bool {
    match initiative {
        RetireWindows => true,
        UserInput(Steering | Arranging | Inert | Hover)
        | Arrange
        | SwitchWorkspaceFocus
        | WarpFocus
        | FollowActivation => false,
    }
}

/// Following an app's activation with focus.
fn follow_activation(initiative: Initiative) -> bool {
    match initiative {
        FollowActivation => true,
        UserInput(Steering | Arranging | Inert | Hover)
        | Arrange
        | RetireWindows
        | SwitchWorkspaceFocus
        | WarpFocus => false,
    }
}

/// The user's layout commands and hover, which act on Space assignments.
fn layout_input(initiative: Initiative) -> bool {
    match initiative {
        UserInput(Arranging | Hover) => true,
        UserInput(Steering | Inert)
        | Arrange
        | RetireWindows
        | SwitchWorkspaceFocus
        | WarpFocus
        | FollowActivation => false,
    }
}

/// An open menu has the pointer: passing over windows must not focus or raise them.
fn refuses_menu(menu: Menu, initiative: Initiative) -> bool {
    match menu {
        Menu::Open => hover(initiative),
        Menu::Closed => false,
    }
}

/// Mission Control is on screen: the windows under the pointer are its thumbnails, not windows
/// to focus, and app inventories omit windows it shows.
fn refuses_mission_control(mission_control: MissionControl, initiative: Initiative) -> bool {
    match mission_control {
        MissionControl::Active => hover(initiative) || retire(initiative),
        MissionControl::Inactive => false,
    }
}

/// Focusing another window would hide the sheet the frontmost app is asking about.
fn refuses_modal(modal: Modal, initiative: Initiative) -> bool {
    match modal {
        Modal::Frontmost => hover(initiative),
        Modal::Clear => false,
    }
}

/// Windows slide under a resting pointer while the strip scrolls.
fn refuses_viewport(viewport: Viewport, initiative: Initiative) -> bool {
    match viewport {
        Viewport::Gesturing => hover(initiative),
        Viewport::Free => false,
    }
}

fn refuses_focus_claim(focus_claim: FocusClaim, initiative: Initiative) -> bool {
    match focus_claim {
        FocusClaim::PausesHover => hover(initiative),
        FocusClaim::Free => false,
    }
}

/// With no user Space on screen, an inventory that omits a window is no evidence that it is
/// gone.
fn refuses_user_spaces(user_spaces: UserSpaces, initiative: Initiative) -> bool {
    match user_spaces {
        UserSpaces::None => retire(initiative),
        UserSpaces::Shown => false,
    }
}

/// Until the user gives input, an app activation is loginwindow handing focus back.
fn refuses_wake(wake: Wake, initiative: Initiative) -> bool {
    match wake {
        Wake::AwaitingInput => follow_activation(initiative),
        Wake::UserPresent => false,
    }
}

/// The activation is most likely the switch's own focus landing.
fn refuses_workspace_switch(workspace_switch: WorkspaceSwitch, initiative: Initiative) -> bool {
    match workspace_switch {
        WorkspaceSwitch::Switching => follow_activation(initiative),
        WorkspaceSwitch::Idle => false,
    }
}

/// Layout commands and hover act on Space assignments that are about to change (each one
/// refused logs). Other commands pass; layout passes and retirement have their own topology
/// checks.
fn refuses_topology(topology: Topology, initiative: Initiative) -> bool {
    match topology {
        Topology::Unsettled => layout_input(initiative),
        Topology::Settled => false,
    }
}

/// Every initiative, for tables and tests.
pub const INITIATIVES: [Initiative; 9] = [
    UserInput(Steering),
    UserInput(Arranging),
    UserInput(Inert),
    UserInput(Hover),
    Arrange,
    RetireWindows,
    SwitchWorkspaceFocus,
    WarpFocus,
    FollowActivation,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn calm() -> Modes<u64> {
        Modes {
            pointer: Pointer::Idle,
            menu: Menu::Closed,
            mission_control: MissionControl::Inactive,
            modal: Modal::Clear,
            viewport: Viewport::Free,
            focus_claim: FocusClaim::Free,
            user_spaces: UserSpaces::Shown,
            wake: Wake::UserPresent,
            workspace_switch: WorkspaceSwitch::Idle,
            topology: Topology::Settled,
        }
    }

    #[test]
    fn with_every_mode_off_everything_is_admitted() {
        for initiative in INITIATIVES {
            assert_eq!(calm().verdict(initiative), Verdict::Admitted, "{initiative:?}");
        }
    }

    #[test]
    fn the_first_refusing_axis_is_named() {
        let modes = Modes {
            pointer: Pointer::Dragging(4),
            menu: Menu::Open,
            ..calm()
        };
        assert_eq!(
            modes.verdict(UserInput(Hover)),
            Verdict::Refused(Axis::Pointer(Pointer::Dragging(4)))
        );
        let modes = Modes {
            menu: Menu::Open,
            mission_control: MissionControl::Active,
            ..calm()
        };
        assert_eq!(modes.verdict(UserInput(Hover)), Verdict::Refused(Axis::Menu));
        assert_eq!(
            modes.verdict(RetireWindows),
            Verdict::Refused(Axis::MissionControl)
        );
    }
}
