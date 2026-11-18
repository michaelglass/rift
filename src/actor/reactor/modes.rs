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
        } = self;
        [
            pointer as &dyn Mode,
            menu,
            native_mission_control,
            modal,
            viewport,
            focus_claim,
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

use Initiative::*;
use InputClass::{Hover, Inert, Steering};

impl Mode for Pointer {
    fn admits(&self, initiative: Initiative) -> bool {
        use Pointer::*;
        match (initiative, self) {
            (UserInput(Inert), Idle | Pressing(_) | Dragging(_)) => true,
            (UserInput(Steering | Hover), Idle) => true,
            // The window under the user's hand is theirs (`Owner::User`), pressed or dragged:
            // input's effect on it would wait for the release, splitting it from the rest of
            // the effect. A hover sample from before the press would focus another window under
            // the held one.
            (UserInput(Steering | Hover), Pressing(_) | Dragging(_)) => false,
            // A press alone changes nothing: the window hasn't moved.
            (Arrange | RetireWindows | SwitchWorkspaceFocus | WarpFocus, Idle | Pressing(_)) => {
                true
            }
            // While the user drags a window, Rift keeps its hands off the layout and the
            // pointer until the drop.
            (Arrange | RetireWindows | SwitchWorkspaceFocus | WarpFocus, Dragging(_)) => false,
        }
    }
}

impl Mode for MenuState {
    fn admits(&self, initiative: Initiative) -> bool {
        match (initiative, self) {
            // An open menu has the pointer: passing over windows must not focus or raise them.
            (UserInput(Hover), MenuState::Open(_)) => false,
            (UserInput(Hover), MenuState::Closed) => true,
            (
                UserInput(Steering | Inert)
                | Arrange
                | RetireWindows
                | SwitchWorkspaceFocus
                | WarpFocus,
                MenuState::Open(_) | MenuState::Closed,
            ) => true,
        }
    }
}

impl Mode for MissionControlState {
    fn admits(&self, initiative: Initiative) -> bool {
        match (initiative, self) {
            // macOS's Mission Control is on screen: the windows under the pointer are its
            // thumbnails, not windows to focus.
            (UserInput(Hover), MissionControlState::Active) => false,
            (UserInput(Hover), MissionControlState::Inactive) => true,
            (
                UserInput(Steering | Inert)
                | Arrange
                | RetireWindows
                | SwitchWorkspaceFocus
                | WarpFocus,
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
                UserInput(Steering | Inert)
                | Arrange
                | RetireWindows
                | SwitchWorkspaceFocus
                | WarpFocus,
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
                UserInput(Steering | Inert)
                | Arrange
                | RetireWindows
                | SwitchWorkspaceFocus
                | WarpFocus,
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
                UserInput(Steering | Inert)
                | Arrange
                | RetireWindows
                | SwitchWorkspaceFocus
                | WarpFocus,
                FocusClaim::PausesHover | FocusClaim::Free,
            ) => true,
        }
    }
}
