//! The mode table. Every mode Rift can be in sits on one independent axis, and each axis states,
//! for every initiative (Rift's own, and the user's input by class), whether it admits it. An
//! initiative goes ahead only when every axis admits it ("deny if any active mode denies").
//! What is refused is discarded, never queued: when a mode ends, Rift acts on the facts as they
//! are then, it does not replay what it refused. The grid is in `docs/modes.md`.
//!
//! Axes are a product, not a set of flags: each axis is one enum, so a combination that makes no
//! sense within an axis (a menu open for nobody, a gesture both held and released) cannot be
//! written, and state that only one mode needs lives inside that mode's variant.

use super::{Initiative, InputClass};
use crate::model::reactor::{MenuState, MissionControlState};
use crate::model::window_store::Pointer;

/// One axis of the table. `admits` states its rule for every initiative and every variant: there
/// is no wildcard, so a new initiative or a new mode does not compile until its row is written.
pub(super) trait Mode: std::fmt::Debug {
    fn admits(&self, initiative: Initiative) -> bool;
}

/// The modes Rift is in right now, one per axis. Built fresh from the facts for each question
/// (`Reactor::modes`), never stored, so it cannot drift from them.
#[derive(Debug)]
pub(super) struct Modes {
    pub(super) pointer: Pointer,
    pub(super) menu: MenuState,
    pub(super) native_mission_control: MissionControlState,
    pub(super) modal: Modal,
    pub(super) viewport: Viewport,
    pub(super) focus_claim: FocusClaim,
    pub(super) user_spaces: UserSpaces,
    pub(super) wake: Wake,
    pub(super) workspace_switch: WorkspaceSwitch,
    pub(super) topology: Topology,
}

impl Modes {
    /// The first axis that refuses `initiative`, `None` when every axis admits it.
    pub(super) fn refusal(&self, initiative: Initiative) -> Option<&dyn Mode> {
        // Destructured so that a new axis does not compile until it is listed here.
        let Modes {
            pointer,
            menu,
            native_mission_control,
            modal,
            viewport,
            focus_claim,
            user_spaces,
            wake,
            workspace_switch,
            topology,
        } = self;
        [
            pointer as &dyn Mode,
            menu,
            native_mission_control,
            modal,
            viewport,
            focus_claim,
            user_spaces,
            wake,
            workspace_switch,
            topology,
        ]
        .into_iter()
        .find(|mode| !mode.admits(initiative))
    }
}

/// A sheet or popover of the frontmost app is showing on an active Space.
#[derive(Debug, Clone, Copy)]
pub(super) enum Modal {
    Clear,
    Frontmost,
}

/// A viewport (scrolling layout) gesture is under the user's fingers.
#[derive(Debug, Clone, Copy)]
pub(super) enum Viewport {
    Free,
    Gesturing,
}

/// The focused window is claimed by an external manager that asked for focus follows mouse to
/// pause while it is focused (`WindowClaimFlags::SUPPRESS_FOCUS_FOLLOWS_MOUSE`).
#[derive(Debug, Clone, Copy)]
pub(super) enum FocusClaim {
    Free,
    PausesHover,
}

/// Whether any screen shows a user Space. `None` with no displays at all (asleep, lid closed)
/// and while every screen shows a system or fullscreen Space.
#[derive(Debug, Clone, Copy)]
pub(super) enum UserSpaces {
    Shown,
    None,
}

/// Whether the user has given input since the machine woke or the session became active.
/// Until they have, app activations are loginwindow handing focus back, not requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Wake {
    UserPresent,
    AwaitingInput,
}

/// A workspace switch is in progress: Rift's own (still settling) or the user's.
#[derive(Debug, Clone, Copy)]
pub(super) enum WorkspaceSwitch {
    Idle,
    Switching,
}

/// Whether Rift's model of Spaces is authoritative: `Unsettled` while no authoritative sample
/// is accepted or a topology invalidation is newer than it (the revision guard with the spaces
/// actor). Window reports are quarantined before they reach the table
/// (`Reactor::should_quarantine_unstable_topology`); this axis covers the user's input.
#[derive(Debug, Clone, Copy)]
pub(super) enum Topology {
    Settled,
    Unsettled,
}

use Initiative::*;
use InputClass::{Arranging, Hover, Inert, Steering};

impl Mode for Pointer {
    /// The pointer's row is the core's (`rift_core::may`): the core owns what the hand is doing,
    /// and the rule that follows from it.
    fn admits(&self, initiative: Initiative) -> bool { rift_core::may(initiative, *self) }
}

impl Mode for MenuState {
    fn admits(&self, initiative: Initiative) -> bool {
        match (initiative, self) {
            // An open menu has the pointer: passing over windows must not focus or raise them.
            (UserInput(Hover), MenuState::Open(_)) => false,
            (UserInput(Hover), MenuState::Closed) => true,
            (
                UserInput(Steering | Arranging | Inert)
                | Arrange
                | RetireWindows
                | SwitchWorkspaceFocus
                | WarpFocus
                | FollowActivation,
                MenuState::Open(_) | MenuState::Closed,
            ) => true,
        }
    }
}

impl Mode for MissionControlState {
    fn admits(&self, initiative: Initiative) -> bool {
        match (initiative, self) {
            // macOS's Mission Control is on screen: the windows under the pointer are its
            // thumbnails, not windows to focus, and app inventories omit windows it shows.
            (UserInput(Hover) | RetireWindows, MissionControlState::Active) => false,
            (UserInput(Hover) | RetireWindows, MissionControlState::Inactive) => true,
            (
                UserInput(Steering | Arranging | Inert)
                | Arrange
                | SwitchWorkspaceFocus
                | WarpFocus
                | FollowActivation,
                MissionControlState::Active | MissionControlState::Inactive,
            ) => true,
        }
    }
}

impl Mode for Modal {
    fn admits(&self, initiative: Initiative) -> bool {
        match (initiative, self) {
            // Focusing another window would hide the sheet the frontmost app is asking about.
            (UserInput(Hover), Modal::Frontmost) => false,
            (UserInput(Hover), Modal::Clear) => true,
            (
                UserInput(Steering | Arranging | Inert)
                | Arrange
                | RetireWindows
                | SwitchWorkspaceFocus
                | WarpFocus
                | FollowActivation,
                Modal::Frontmost | Modal::Clear,
            ) => true,
        }
    }
}

impl Mode for Viewport {
    fn admits(&self, initiative: Initiative) -> bool {
        match (initiative, self) {
            // Windows slide under a resting pointer while the strip scrolls.
            (UserInput(Hover), Viewport::Gesturing) => false,
            (UserInput(Hover), Viewport::Free) => true,
            (
                UserInput(Steering | Arranging | Inert)
                | Arrange
                | RetireWindows
                | SwitchWorkspaceFocus
                | WarpFocus
                | FollowActivation,
                Viewport::Gesturing | Viewport::Free,
            ) => true,
        }
    }
}

impl Mode for FocusClaim {
    fn admits(&self, initiative: Initiative) -> bool {
        match (initiative, self) {
            (UserInput(Hover), FocusClaim::PausesHover) => false,
            (UserInput(Hover), FocusClaim::Free) => true,
            (
                UserInput(Steering | Arranging | Inert)
                | Arrange
                | RetireWindows
                | SwitchWorkspaceFocus
                | WarpFocus
                | FollowActivation,
                FocusClaim::PausesHover | FocusClaim::Free,
            ) => true,
        }
    }
}

impl Mode for UserSpaces {
    fn admits(&self, initiative: Initiative) -> bool {
        match (initiative, self) {
            // With no user Space on screen, an inventory that omits a window is no evidence
            // that it is gone.
            (RetireWindows, UserSpaces::None) => false,
            (RetireWindows, UserSpaces::Shown) => true,
            (
                UserInput(Steering | Arranging | Inert | Hover)
                | Arrange
                | SwitchWorkspaceFocus
                | WarpFocus
                | FollowActivation,
                UserSpaces::None | UserSpaces::Shown,
            ) => true,
        }
    }
}

impl Mode for Wake {
    fn admits(&self, initiative: Initiative) -> bool {
        match (initiative, self) {
            (FollowActivation, Wake::AwaitingInput) => false,
            (FollowActivation, Wake::UserPresent) => true,
            (
                UserInput(Steering | Arranging | Inert | Hover)
                | Arrange
                | RetireWindows
                | SwitchWorkspaceFocus
                | WarpFocus,
                Wake::AwaitingInput | Wake::UserPresent,
            ) => true,
        }
    }
}

impl Mode for WorkspaceSwitch {
    fn admits(&self, initiative: Initiative) -> bool {
        match (initiative, self) {
            // The activation is most likely the switch's own focus landing.
            (FollowActivation, WorkspaceSwitch::Switching) => false,
            (FollowActivation, WorkspaceSwitch::Idle) => true,
            (
                UserInput(Steering | Arranging | Inert | Hover)
                | Arrange
                | RetireWindows
                | SwitchWorkspaceFocus
                | WarpFocus,
                WorkspaceSwitch::Switching | WorkspaceSwitch::Idle,
            ) => true,
        }
    }
}

impl Mode for Topology {
    fn admits(&self, initiative: Initiative) -> bool {
        match (initiative, self) {
            // Layout commands and hover act on Space assignments that are about to change.
            // They used to be dropped silently by the quarantine; now each one logs.
            (UserInput(Arranging | Hover), Topology::Unsettled) => false,
            (UserInput(Arranging | Hover), Topology::Settled) => true,
            // Unchanged from the quarantine: other commands pass; layout passes and retirement
            // have their own topology checks.
            (
                UserInput(Steering | Inert)
                | Arrange
                | RetireWindows
                | SwitchWorkspaceFocus
                | WarpFocus
                | FollowActivation,
                Topology::Unsettled | Topology::Settled,
            ) => true,
        }
    }
}
