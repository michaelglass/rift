use objc2_core_foundation::{CGPoint, CGRect};

use crate::actor::app::WindowId;
use crate::layout_engine::WindowDropAction;
use crate::sys::screen::SpaceId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragKind {
    NativeMove,
    NativeResize,
    ModifierMove,
}

#[derive(Debug, Clone, Copy)]
pub struct DragSceneTarget {
    pub window: WindowId,
    pub space: SpaceId,
    pub frame: CGRect,
}

#[derive(Debug, Clone, Default)]
pub struct DragScene {
    /// Layout-wide semantic override. Stack layouts use Swap for every zone.
    pub action_override: Option<WindowDropAction>,
    /// Whether each source-tile edge points toward a logical tiled neighbor.
    pub source_neighbors: [bool; 4],
    pub targets: Vec<DragSceneTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropZone {
    Center,
    West,
    East,
    North,
    South,
}

#[derive(Debug, Clone, Copy)]
pub struct DropIntent {
    pub window: WindowId,
    pub space: SpaceId,
    pub frame: CGRect,
    pub zone: DropZone,
    pub action: WindowDropAction,
}

#[derive(Debug, Clone, Copy)]
pub struct DropTarget {
    pub intent: DropIntent,
    pub preview_area: CGRect,
}

/// Where a dragged window started and where it is now. The window itself is whichever one
/// the drag holds, so it is not stored here.
#[derive(Debug, Clone, Copy)]
pub struct DragSource {
    pub origin_frame: CGRect,
    pub last_frame: CGRect,
    pub origin_space: Option<SpaceId>,
    pub current_space: Option<SpaceId>,
    pub tiled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MouseButton {
    Left,
    Right,
}

/// A user drag: where the pointer is and where a drop would land. It lives in the held
/// window's placement in the window store, which is how it knows its window.
#[derive(Debug, Clone)]
pub struct Session {
    pub source: DragSource,
    pub(crate) button: MouseButton,
    pub pointer: CGPoint,
    pub anchor_point: CGPoint,
    pub scene: DragScene,
    pub(crate) target: TargetState,
    pub(crate) unavailable: Vec<(WindowId, DropZone, WindowDropAction)>,
    pub kind: DragKind,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) enum TargetState {
    #[default]
    None,
    Candidate(DropIntent),
    Validated(DropTarget),
}

impl TargetState {
    pub(crate) fn intent(self) -> Option<DropIntent> {
        match self {
            Self::Candidate(intent) => Some(intent),
            Self::Validated(target) => Some(target.intent),
            Self::None => None,
        }
    }

    pub(crate) fn validated(self) -> Option<DropTarget> {
        match self {
            Self::Validated(target) => Some(target),
            Self::None | Self::Candidate(_) => None,
        }
    }
}

/// The mouse button as macOS reported it with an observation, read once where the
/// observation arrives. Trackpad tap-to-drag and three-finger drag may move windows with no
/// button reported; see [`Mover::of`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Down,
    Up,
    Unknown,
}

impl Button {
    pub fn reported(state: Option<crate::sys::event::MouseState>) -> Self {
        match state {
            Some(crate::sys::event::MouseState::Down) => Button::Down,
            Some(crate::sys::event::MouseState::Up) => Button::Up,
            None => Button::Unknown,
        }
    }
}

/// What the user's drag says about one window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    /// The user is dragging this window.
    This,
    /// The user is dragging another window.
    Other,
    Nobody,
}

/// The pointer let go of `.0`. Only this ends a drag by release: it comes from the input tap,
/// which reports every release, or from a fresh read of the button during a native drag (a
/// release the tap missed). The button a window's frame report carries is never one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerReleased(pub MouseButton);

/// Who moved a window whose frame changed without Rift asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mover {
    /// The user, dragging this window: its drag starts or continues.
    User,
    /// The app (or anything but the pointer): a programmatic move.
    App,
}

impl Mover {
    pub fn of(hold: Hold, button: Button) -> Self {
        match (hold, button) {
            // Its own drag continues whatever the button reads: only a release ends it.
            (Hold::This, _) => Mover::User,
            // One pointer, one drag: while it holds another window, this one isn't the user's.
            (Hold::Other, _) => Mover::App,
            (Hold::Nobody, Button::Down) => Mover::User,
            // Without a button there's no telling a trackpad drag from an app moving itself;
            // treat it as the app's (conservative: nothing starts a drag on no evidence).
            (Hold::Nobody, Button::Up | Button::Unknown) => Mover::App,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_pointer_drags_a_window() {
        use Button::*;
        use Hold::*;
        for button in [Down, Up, Unknown] {
            assert_eq!(Mover::of(This, button), Mover::User, "{button:?}");
            assert_eq!(Mover::of(Other, button), Mover::App, "{button:?}");
        }
        assert_eq!(Mover::of(Nobody, Down), Mover::User);
        assert_eq!(Mover::of(Nobody, Up), Mover::App);
        assert_eq!(Mover::of(Nobody, Unknown), Mover::App);
    }
}
