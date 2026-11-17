//! Desired frames, and (from R2) the one place that writes them.

use objc2_core_foundation::CGRect;

use crate::actor::app::WindowId;
use crate::model::window_store::WindowStore;
use crate::sys::geometry::Round;

/// Where Rift wants the windows of one space: layout as data. The window the user is dragging
/// is theirs to place until the drop, so it has no desired frame and is not in here.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Desired(Vec<(WindowId, CGRect)>);

impl Desired {
    pub(crate) fn of(layout: &[(WindowId, CGRect)], windows: &WindowStore) -> Self {
        let dragged = windows.drag().map(|(window, _)| window);
        Self(
            layout
                .iter()
                .filter(|(wid, _)| Some(*wid) != dragged)
                .map(|&(wid, frame)| (wid, frame.round()))
                .collect(),
        )
    }

    pub(crate) fn frames(&self) -> &[(WindowId, CGRect)] { &self.0 }

    pub(crate) fn windows(&self) -> Vec<WindowId> { self.0.iter().map(|(wid, _)| *wid).collect() }
}
