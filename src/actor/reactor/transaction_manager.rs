use std::sync::atomic::{AtomicU32, Ordering};

use objc2_core_foundation::CGRect;
use serde::{Deserialize, Serialize};

use crate::model::tx_store::WindowTxStore;
use crate::sys::window_server::WindowServerId;

/// Names one request that changes window frames. Ids come from one counter
/// ([`TransactionCounter`]), so no two requests share one; only equality means anything.
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TransactionId(u32);

/// The one source of transaction ids, shared by every thread that sends frames.
#[derive(Debug, Default)]
pub struct TransactionCounter(AtomicU32);

impl TransactionCounter {
    pub fn next(&self) -> TransactionId {
        TransactionId(self.0.fetch_add(1, Ordering::Relaxed).wrapping_add(1))
    }
}

/// Manages window transaction IDs and their associated target frames.
#[derive(Debug)]
pub struct TransactionManager {
    pub store: WindowTxStore,
}

impl TransactionManager {
    pub fn new(store: WindowTxStore) -> Self { Self { store } }

    /// Removes the transaction ID entry for a window.
    pub fn remove_for_window(&self, wsid: WindowServerId) { self.store.remove(&wsid); }

    /// The id of a new request, recorded nowhere yet.
    pub fn mint(&self) -> TransactionId { self.store.next_id() }

    /// Records `txid` as the latest request sent to each window in `windows`. Their pending
    /// frames are the core's (`RiftState` projects them).
    pub fn record(&self, txid: TransactionId, windows: impl IntoIterator<Item = WindowServerId>) {
        for wsid in windows {
            self.store.sent(wsid, txid);
        }
    }

    /// Gets the last sent transaction ID for a window.
    pub fn get_last_sent_txid(&self, wsid: WindowServerId) -> TransactionId {
        self.store.last_txid(&wsid)
    }

    /// Gets the target frame for a window's transaction, if it exists.
    pub fn get_target_frame(&self, wsid: WindowServerId) -> Option<CGRect> {
        self.store.get(&wsid)?.target
    }
}
