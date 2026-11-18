use std::sync::Arc;

use dashmap::DashMap;
use objc2_core_foundation::CGRect;

use crate::actor::reactor::transaction_manager::{TransactionCounter, TransactionId};
use crate::sys::window_server::WindowServerId;

#[derive(Clone, Copy, Debug, Default)]
pub struct TxRecord {
    pub txid: TransactionId,
    pub target: Option<CGRect>,
}

/// Thread-safe cache mapping window server IDs to their last known transaction, and the one
/// counter every transaction id comes from.
#[derive(Clone, Default, Debug)]
pub struct WindowTxStore {
    records: Arc<DashMap<WindowServerId, TxRecord>>,
    ids: Arc<TransactionCounter>,
}

impl WindowTxStore {
    pub fn new() -> Self { Self::default() }

    /// The core's pending frame for `id` (its projection): the record's id is kept.
    pub fn project_target(&self, id: WindowServerId, target: CGRect) {
        self.records.entry(id).or_default().target = Some(target);
    }

    /// The id of the latest request sent to `id`: its pending frame (the core's) is kept.
    pub fn sent(&self, id: WindowServerId, txid: TransactionId) {
        self.records.entry(id).or_default().txid = txid;
    }

    pub fn get(&self, id: &WindowServerId) -> Option<TxRecord> {
        self.records.get(id).map(|entry| *entry)
    }

    pub fn remove(&self, id: &WindowServerId) { self.records.remove(id); }

    pub fn clear_target(&self, id: &WindowServerId) {
        if let Some(mut record) = self.records.get_mut(id) {
            record.target = None;
        }
    }

    /// The id of a new request.
    pub fn next_id(&self) -> TransactionId { self.ids.next() }

    pub fn last_txid(&self, id: &WindowServerId) -> TransactionId {
        self.get(id).map(|record| record.txid).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};

    use super::*;

    #[test]
    fn the_projected_target_and_the_last_request_are_kept_apart() {
        let store = WindowTxStore::new();
        let wsid = WindowServerId::new(1);
        let target = CGRect::new(CGPoint::new(10.0, 20.0), CGSize::new(30.0, 40.0));
        let txid = store.next_id();
        store.sent(wsid, txid);
        store.project_target(wsid, target);
        assert_eq!(store.get(&wsid).unwrap().target, Some(target));

        let newer = store.next_id();
        store.sent(wsid, newer);
        assert_eq!(
            store.get(&wsid).unwrap().target,
            Some(target),
            "a send keeps the target"
        );
        store.clear_target(&wsid);
        let record = store.get(&wsid).unwrap();
        assert_eq!((record.txid, record.target), (newer, None));
        assert_ne!(txid, newer);
    }
}
