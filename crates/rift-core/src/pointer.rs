//! What the pointer is doing with windows, and the rules that follow from it: whose a
//! window's frame is, and which of Rift's initiatives may run (the mode gate: what a mode
//! refuses is discarded, never queued).

/// What the pointer is doing with windows.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pointer<W> {
    Idle,
    /// Down on this window, which hasn't moved.
    Pressing(W),
    /// Dragging this window.
    Dragging(W),
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

/// A release the platform saw.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Release {
    /// The input tap reported this button going up.
    Button(Button),
    /// A fresh read found the button up although the tap reported no release (it missed one).
    Missed,
}

/// Why the user's hand came off a window.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ungrip {
    /// The button that pressed it went up: a press ends, a drag drops.
    Released,
    /// Without a drop: a press elsewhere or on nothing, a cancel, Mission Control.
    LetGo,
    /// The window left Rift's control (destroyed, retired, replaced).
    Gone,
}

/// What the user's hand is doing: the core's own state, of which the platform's window store
/// holds the projection. One hand, so one grip; a drag starts only from a press of its window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Grip<W> {
    Idle,
    Pressed { window: W, button: Button },
    Dragging { window: W, button: Button },
}

impl<W: Copy> Grip<W> {
    pub(crate) fn pointer(self) -> Pointer<W> {
        match self {
            Grip::Idle => Pointer::Idle,
            Grip::Pressed { window, .. } => Pointer::Pressing(window),
            Grip::Dragging { window, .. } => Pointer::Dragging(window),
        }
    }
}

/// Whose a window's frame is right now.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Owner {
    /// Rift's: layout and placements decide where it goes.
    Rift,
    /// The user's: their hand is on it (pressed or dragging), or they are dragging another
    /// window (a drag does not change the layout; only the drop does). Rift writes nothing to
    /// it but a modifier drag's interactive frame. A desire for it is discarded, not queued:
    /// the release re-derives its frame from what is true then.
    User,
}

/// Whether the user's hand is on a window a report is about.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Holder {
    User,
    Nobody,
}

impl<W: PartialEq + Copy> Pointer<W> {
    /// Whose `window`'s frame is: the user's while their hand is on it, and every window's
    /// while they drag one.
    pub fn owner(self, window: W) -> Owner {
        match self {
            Pointer::Dragging(_) => Owner::User,
            Pointer::Pressing(pressed) if pressed == window => Owner::User,
            Pointer::Pressing(_) | Pointer::Idle => Owner::Rift,
        }
    }

    /// Whether the user's hand is on `window`.
    pub fn holder(self, window: W) -> Holder {
        match self {
            Pointer::Pressing(held) | Pointer::Dragging(held) if held == window => Holder::User,
            Pointer::Pressing(_) | Pointer::Dragging(_) | Pointer::Idle => Holder::Nobody,
        }
    }
}

/// What user input does, as far as the pointer rule in [`may`] needs.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputClass {
    /// Changes which windows are where, shown, focused or raised, or where the pointer is.
    Steering,
    /// Steering that rearranges by Space assignment: layout commands and moves between
    /// displays. Refused while the topology is unsettled as well as by everything that refuses
    /// steering.
    Arranging,
    /// Touches no window and not the pointer: settings, binding modes, diagnostics, saving.
    Inert,
    /// The pointer resting over a window: focus follows mouse and autoraise.
    Hover,
}

/// Things Rift would do that have a rule about the pointer: its own initiatives, and the
/// user's commands by class.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Initiative {
    /// User input: a command (hotkey, CLI, menu) or a gesture. Input the pointer rule refuses
    /// is discarded, never queued: after the drag nothing is replayed.
    UserInput(InputClass),
    /// Arrangement and geometry layout passes.
    Arrange,
    /// Retiring windows an inventory refresh no longer reports.
    RetireWindows,
    /// Following a workspace switch with focus.
    SwitchWorkspaceFocus,
    /// Warping the pointer or focus to a window.
    WarpFocus,
    /// Following an app's activation to the workspace holding its window.
    FollowActivation,
}

/// Whether Rift may take `initiative` with the pointer doing what it does: the pointer's row of
/// the mode table (rift's `Mode` for the pointer axis delegates here). Stated per pointer state
/// for every initiative, with no wildcard.
pub fn may<W>(initiative: Initiative, pointer: Pointer<W>) -> bool {
    use Initiative::*;
    use InputClass::{Arranging, Hover, Inert, Steering};
    use Pointer::*;
    match (initiative, pointer) {
        (UserInput(Inert), Idle | Pressing(_) | Dragging(_)) => true,
        (UserInput(Steering | Arranging | Hover), Idle) => true,
        // The window under the user's hand is theirs (`Owner::User`), pressed or dragged:
        // input's effect on it would wait for the release, splitting it from the rest of the
        // effect. A hover sample from before the press would focus another window under the
        // held one.
        (UserInput(Steering | Arranging | Hover), Pressing(_) | Dragging(_)) => false,
        // A press alone changes nothing: the window hasn't moved.
        (Arrange | RetireWindows | SwitchWorkspaceFocus | WarpFocus, Idle | Pressing(_)) => true,
        // While the user drags a window, Rift keeps its hands off the layout and the pointer
        // until the drop.
        (Arrange | RetireWindows | SwitchWorkspaceFocus | WarpFocus, Dragging(_)) => false,
        // Following an activation switches workspace, which moves every window; nothing moves
        // existing windows until the drop, and a press may become a drag.
        (FollowActivation, Idle) => true,
        (FollowActivation, Pressing(_) | Dragging(_)) => false,
    }
}
