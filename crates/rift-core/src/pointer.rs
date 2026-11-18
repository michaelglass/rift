//! What the pointer is doing with windows, and the rules that follow from it: whose a
//! window's frame is, and which of Rift's initiatives may run (the mode gate: what a mode
//! refuses is discarded, never queued).

/// What the pointer is doing with windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pointer<W> {
    Idle,
    /// Down on this window, which hasn't moved.
    Pressing(W),
    /// Dragging this window.
    Dragging(W),
}

/// Whose a window's frame is right now.
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputClass {
    /// Changes which windows are where, shown, focused or raised, or where the pointer is.
    Steering,
    /// Touches no window and not the pointer: settings, binding modes, diagnostics, saving.
    Inert,
}

/// Things Rift would do that have a rule about the pointer: its own initiatives, and the
/// user's commands by class.
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
}

/// Whether Rift may take `initiative` with the pointer doing what it does. Stated per pointer
/// state for every initiative, with no wildcard.
pub fn may<W>(initiative: Initiative, pointer: Pointer<W>) -> bool {
    use Initiative::*;
    use Pointer::*;
    match (initiative, pointer) {
        (UserInput(InputClass::Inert), Idle | Pressing(_) | Dragging(_)) => true,
        (UserInput(InputClass::Steering), Idle) => true,
        // The window under the user's hand is theirs (`Owner::User`), pressed or dragged:
        // input's effect on it would wait for the release, splitting it from the rest of
        // the effect. Dropped instead, not deferred.
        (UserInput(InputClass::Steering), Pressing(_) | Dragging(_)) => false,
        // A press alone changes nothing: the window hasn't moved.
        (Arrange, Idle | Pressing(_)) => true,
        (RetireWindows, Idle | Pressing(_)) => true,
        (SwitchWorkspaceFocus, Idle | Pressing(_)) => true,
        (WarpFocus, Idle | Pressing(_)) => true,
        // While the user drags a window, Rift keeps its hands off the layout and the
        // pointer until the drop.
        (Arrange, Dragging(_)) => false,
        (RetireWindows, Dragging(_)) => false,
        (SwitchWorkspaceFocus, Dragging(_)) => false,
        (WarpFocus, Dragging(_)) => false,
    }
}
