use std::sync::atomic::{AtomicU32, Ordering};

use objc2_core_foundation::CGRect;
use serde::{Deserialize, Serialize};

use crate::model::tx_store::WindowTxStore;
use crate::sys::window_server::WindowServerId;

/// Names one request that changes window frames. Ids come from one counter
/// ([`TransactionCounter`]), so no two requests share one; only equality means anything.
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionId(u32);

impl TransactionId {
    /// The id as a number, for a trace on plain data.
    #[cfg(test)]
    pub(crate) fn number(self) -> u32 { self.0 }
}

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

    /// Stores a transaction ID for a window with its target frame.
    pub fn store_txid(&self, wsid: WindowServerId, txid: TransactionId, target: CGRect) {
        self.store.insert(wsid, txid, target);
    }

    /// Updates multiple transaction ID entries.
    pub fn update_txid_entries<I>(&self, entries: I)
    where I: IntoIterator<Item = (WindowServerId, TransactionId, CGRect)> {
        for (wsid, txid, target) in entries {
            self.store.insert(wsid, txid, target);
        }
    }

    /// Removes the transaction ID entry for a window.
    pub fn remove_for_window(&self, wsid: WindowServerId) { self.store.remove(&wsid); }

    /// Clears the pending target for a window while preserving its last txid.
    pub fn clear_target_for_window(&self, wsid: WindowServerId) { self.store.clear_target(&wsid); }

    /// The id of a new request, recorded nowhere yet.
    pub fn mint(&self) -> TransactionId { self.store.next_id() }

    /// Records `txid`, with its target, for each window a request sent under it writes.
    pub fn record(
        &self,
        txid: TransactionId,
        targets: impl IntoIterator<Item = (WindowServerId, CGRect)>,
    ) {
        self.update_txid_entries(targets.into_iter().map(|(wsid, target)| (wsid, txid, target)));
    }

    /// Mints the id of one request and records it, with its target, for each window the
    /// request writes.
    pub fn stamp(
        &self,
        targets: impl IntoIterator<Item = (WindowServerId, CGRect)>,
    ) -> TransactionId {
        let txid = self.store.next_id();
        self.update_txid_entries(targets.into_iter().map(|(wsid, target)| (wsid, txid, target)));
        txid
    }

    /// Sets the last sent transaction ID for a window.
    pub fn set_last_sent_txid(&self, wsid: WindowServerId, txid: TransactionId) {
        self.store.set_last_txid(wsid, txid);
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
