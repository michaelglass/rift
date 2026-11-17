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

/// The pointer let go of `.0`. Only this ends a press or a drag by release: it comes from the
/// input tap, which reports every release, or from a fresh read of the button finding it up
/// (a release the tap missed). The button a window's frame report carries is never one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerReleased(pub MouseButton);
