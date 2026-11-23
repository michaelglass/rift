pub mod app_rules;
pub mod floating_position_store;
pub mod hidden_window_placement;
pub mod selection;
pub mod server;
// Test-only until the workspace catalog moves onto it (slice 2b part 2).
#[cfg(test)]
pub(crate) mod slot_table;
pub mod tree;
pub mod tx_store;
pub mod virtual_workspace;
pub mod window_store;
pub use app_rules::{
    AppRuleDecision, AppRuleEffects, AppRuleEngine, AppRuleRejection, AppRuleResult,
    WindowRuleContext,
};
pub use floating_position_store::FloatingPositionStore;
pub use hidden_window_placement::{HiddenWindowPlacement, HideCorner};
pub use virtual_workspace::{VirtualWorkspace, VirtualWorkspaceId, WorkspaceStore};
pub use window_store::{Transfer, WindowRecord, WindowStore, WindowVisibility};
pub mod broadcast;
pub mod drag;
pub mod reactor;
pub mod space_activation;
pub use reactor::{RiftState, Stores};
