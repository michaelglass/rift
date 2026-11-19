pub use rift_protocol::{DisplaySelector, ReactorCommand};
use serde::{Deserialize, Serialize};

use crate::actor::app::{AppInfo, AppThreadHandle, WindowId, pid_t};
use crate::common::log::MetricsCommand;
use crate::layout_engine::{LayoutCommand, WindowLayoutInfo};
use crate::model::WindowStore;
use crate::sys::app::WindowInfo;
use crate::sys::screen::SpaceId;

/// All mutable domain state is owned by the reactor thread.
///
/// Workspace topology is still carried by the layout coordinator during this
/// migration, but window identity, native-space observations, and workspace
/// assignments have one explicit owner here. Cross-store operations receive
/// this store by reference instead of retaining an alias to it.
#[derive(Debug, Default)]
pub struct RiftState {
    pub windows: WindowStore,
    /// The decisions: the core, stepped through its applier. Beside the window store so a
    /// workflow that changes windows can tell the core in the same breath.
    pub(crate) core: crate::actor::reactor::reconciler::Reconciler,
    /// The pending store the app actors read across threads: its targets are the projection of
    /// the core's pending frames (`Effect::Pending`), shared with the transaction manager.
    pub(crate) tx: crate::model::tx_store::WindowTxStore,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Requested(pub bool);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum Command {
    Layout(LayoutCommand),
    Metrics(MetricsCommand),
    Reactor(ReactorCommand),
}

#[derive(Debug, Clone)]
pub enum MissionControlState {
    Inactive,
    Active,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuState {
    Closed,
    Open(pid_t),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceSwitchState {
    Inactive,
    /// A switch is in progress; the origin only exists while it is.
    Active(WorkspaceSwitchOrigin),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceSwitchOrigin {
    Manual,
    Auto,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RefocusState {
    None,
    Pending(SpaceId),
}

#[derive(Debug)]
pub(crate) struct AppState {
    #[allow(unused)]
    pub(crate) info: AppInfo,
    pub(crate) handle: AppThreadHandle,
}

#[derive(Debug, Clone)]
pub(crate) struct WindowState {
    pub(crate) info: WindowInfo,
    /// The last known frame of the window. Always includes the last write.
    ///
    /// This value only updates monotonically with respect to writes; in other
    /// words, we only accept reads when we know they come after the last write.
    /// It is the core's model frame, projected: written only by [`Self::project_seen`].
    frame_monotonic: CGRect,
    // Admission's own facts (the heuristic's verdict, the rule, native transience) are the
    // core's lifecycle's, kept by the window store; minimize and the AX roles are read off
    // `info`, the AX snapshot, and the store tells the core each change to them.
}

impl WindowState {
    /// Where the model takes the window to be.
    pub(crate) fn frame_monotonic(&self) -> CGRect { self.frame_monotonic }

    /// The core's `Effect::Seen`, applied by the reactor's projection and nothing else.
    pub(crate) fn project_seen(&mut self, frame: CGRect) { self.frame_monotonic = frame; }
}

impl From<WindowInfo> for WindowState {
    fn from(info: WindowInfo) -> WindowState {
        WindowState {
            frame_monotonic: info.frame,
            info,
        }
    }
}

impl WindowState {
    pub(crate) fn layout_info(&self, wid: WindowId) -> WindowLayoutInfo {
        WindowLayoutInfo {
            window_id: wid,
            title: Some(self.info.title.clone()),
            ax_role: self.info.ax_role.clone(),
            ax_subrole: self.info.ax_subrole.clone(),
            is_resizable: self.info.is_resizable,
            current_size: self.frame_monotonic.size,
            min_size: self.info.min_size,
            max_size: self.info.max_size,
        }
    }

    /// What the AX roles say of the window, parsed at the boundary.
    pub(crate) fn ax_kind(&self) -> rift_core::lifecycle::AxKind {
        rift_core::lifecycle::AxKind::classify(
            crate::sys::app::is_ax_transient(&self.info),
            self.info.ax_role.as_deref() == Some("AXWindow")
                && self.info.ax_subrole.as_deref() == Some("AXDialog"),
        )
    }

    /// Whether the AX snapshot has the window minimized.
    pub(crate) fn minimize(&self) -> rift_core::lifecycle::Minimize {
        if self.info.is_minimized {
            rift_core::lifecycle::Minimize::Minimized
        } else {
            rift_core::lifecycle::Minimize::Shown
        }
    }

    pub(crate) fn can_reconcile_admission(&self) -> bool { !self.info.is_minimized }
}

/// An app rule's override as the rule it is.
pub(crate) fn rule_of(manage_override: Option<bool>) -> rift_core::lifecycle::Rule {
    use rift_core::lifecycle::Rule;
    match manage_override {
        None => Rule::NoRule,
        Some(true) => Rule::Manage,
        Some(false) => Rule::Ignore,
    }
}

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReactorError {
    #[error("App communication failed: {0}")]
    AppCommunicationFailed(#[from] tokio::sync::mpsc::error::SendError<crate::actor::app::Request>),
    #[error("Stack line communication failed: {0}")]
    StackLineCommunicationFailed(
        #[from] tokio::sync::mpsc::error::TrySendError<crate::actor::stack_line::Event>,
    ),
    #[error("Raise manager communication failed: {0}")]
    RaiseManagerCommunicationFailed(
        #[from] tokio::sync::mpsc::error::SendError<crate::actor::raise_manager::Event>,
    ),
}

#[cfg(test)]
mod tests {
    use rift_protocol::{RestoreScope, RestoreSource};

    use super::*;

    #[test]
    fn legacy_restore_command_defaults_to_portable_source_policy() {
        let mut serialized = serde_json::to_value(ReactorCommand::RestoreLayout {
            path: "layout.ron".into(),
            scope: RestoreScope::Workspace,
            source: RestoreSource::CurrentSpace,
        })
        .unwrap();
        serialized
            .get_mut("restore_layout")
            .and_then(serde_json::Value::as_object_mut)
            .unwrap()
            .remove("source");

        let restored: ReactorCommand = serde_json::from_value(serialized).unwrap();

        assert_eq!(restored, ReactorCommand::RestoreLayout {
            path: "layout.ron".into(),
            scope: RestoreScope::Workspace,
            source: RestoreSource::SavedActiveSpace,
        });
    }
}
use objc2_core_foundation::CGRect;
