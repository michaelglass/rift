use objc2_core_foundation::CGRect;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::actor::app::WindowId;
use crate::common::collections::HashMap;
use crate::model::{VirtualWorkspaceId, WorkspaceStore};
use crate::sys::geometry::CGRectDef;

/// Viewport placement belongs to workspace identity, independent of its native owner.
#[serde_as]
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct FloatingPositionStore {
    #[serde_as(as = "HashMap<_, CGRectDef>")]
    positions: HashMap<(VirtualWorkspaceId, WindowId), CGRect>,
}

impl FloatingPositionStore {
    pub(crate) fn validate_persisted(&self, workspaces: &WorkspaceStore) -> Result<(), String> {
        for (&(workspace, window), frame) in &self.positions {
            if !workspaces.workspaces.contains_key(workspace) {
                return Err(format!(
                    "floating frame for window {window:?} references missing workspace {workspace:?}"
                ));
            }
            if !frame.origin.x.is_finite()
                || !frame.origin.y.is_finite()
                || !frame.size.width.is_finite()
                || !frame.size.height.is_finite()
                || frame.size.width < 0.0
                || frame.size.height < 0.0
            {
                return Err(format!("window {window:?} has an invalid floating frame"));
            }
        }
        Ok(())
    }

    /// Every saved frame in a stable order, so a choice between duplicates is deterministic.
    pub(crate) fn into_sorted(self) -> Vec<(VirtualWorkspaceId, WindowId, CGRect)> {
        let mut positions: Vec<_> = self
            .positions
            .into_iter()
            .map(|((workspace, window), frame)| (workspace, window, frame))
            .collect();
        positions.sort_unstable_by_key(|&(workspace, window, _)| (workspace, window));
        positions
    }

    pub(crate) fn positioned_windows(&self) -> impl Iterator<Item = WindowId> + '_ {
        self.positions.keys().map(|(_, window)| *window)
    }

    pub fn store(&mut self, workspace: VirtualWorkspaceId, window: WindowId, frame: CGRect) {
        self.positions.insert((workspace, window), frame);
    }

    pub fn get(&self, workspace: VirtualWorkspaceId, window: WindowId) -> Option<CGRect> {
        self.positions.get(&(workspace, window)).copied()
    }
}
