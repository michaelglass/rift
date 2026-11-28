use objc2_core_foundation::{CGPoint, CGRect};

use crate::actor::app::WindowId;
use crate::layout_engine::WindowDropAction;
use crate::sys::screen::SpaceId;

/// Who moves the window during a drag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DragKind {
    /// The user moves or resizes it (title bar, edges); Rift follows its frame.
    Native(NativeOp),
    /// Rift moves it by interactive frames, the pointer `anchor` away from where it was
    /// pressed.
    Modifier { anchor: CGPoint },
}

/// What a native drag does to the window, decided once by the frame change that started it.
/// An app that changes its own size mid-move (WhatsApp does) does not turn the move into a
/// resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeOp {
    /// The title bar: the window keeps its size and goes somewhere to drop.
    Move,
    /// An edge or corner: the window changes size in place, and there is nowhere to drop.
    Resize,
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
    pub action_override: rift_core::placement::Override,
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

impl From<MouseButton> for rift_core::Button {
    fn from(button: MouseButton) -> Self {
        match button {
            MouseButton::Left => rift_core::Button::Left,
            MouseButton::Right => rift_core::Button::Right,
        }
    }
}

/// A user drag: where the pointer is and where a drop would land. It lives in the held
/// window's placement in the window store, which is how it knows its window.
#[derive(Debug, Clone)]
pub struct Session {
    pub source: DragSource,
    pub pointer: CGPoint,
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
