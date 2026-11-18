use std::sync::Arc;

use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
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

    pub fn insert(&self, id: WindowServerId, txid: TransactionId, target: CGRect) {
        match self.records.entry(id) {
            Entry::Occupied(mut entry) => {
                *entry.get_mut() = TxRecord { txid, target: Some(target) }
            }
            Entry::Vacant(entry) => {
                entry.insert(TxRecord { txid, target: Some(target) });
            }
        }
    }

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

    /// A retiring presentation cannot clear a newer semantic transaction.
    pub fn clear_target_if_current(&self, id: &WindowServerId, txid: TransactionId) {
        if let Some(mut record) = self.records.get_mut(id)
            && record.txid == txid
        {
            record.target = None;
        }
    }

    /// The id of a new request.
    pub fn next_id(&self) -> TransactionId { self.ids.next() }

    /// The id of a new request to `id`, recorded as its latest with no target yet.
    pub fn next_txid(&self, id: WindowServerId) -> TransactionId {
        let txid = self.next_id();
        self.set_last_txid(id, txid);
        txid
    }

    pub fn set_last_txid(&self, id: WindowServerId, txid: TransactionId) {
        match self.records.entry(id) {
            Entry::Occupied(mut entry) => {
                let record = entry.get_mut();
                record.txid = txid;
                record.target = None;
            }
            Entry::Vacant(entry) => {
                entry.insert(TxRecord { txid, target: None });
            }
        }
    }

    pub fn last_txid(&self, id: &WindowServerId) -> TransactionId {
        self.get(id).map(|record| record.txid).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};

    use super::*;

    #[test]
    fn clear_target_keeps_last_txid_and_fences_retired_presentations() {
        let store = WindowTxStore::new();
        let wsid = WindowServerId::new(1);
        let target = CGRect::new(CGPoint::new(10.0, 20.0), CGSize::new(30.0, 40.0));
        let txid = store.next_txid(wsid);
        store.insert(wsid, txid, target);
        assert_eq!(store.get(&wsid).unwrap().target, Some(target));

        store.clear_target(&wsid);

        let record = store.get(&wsid).expect("tx record should exist");
        assert_eq!(record.txid, txid);
        assert_eq!(record.target, None);

        let newer = store.next_txid(wsid);
        store.insert(wsid, newer, target);
        store.clear_target_if_current(&wsid, txid);
        assert_eq!(
            store.get(&wsid).unwrap().target,
            Some(target),
            "late retirement must preserve the new target"
        );
        store.clear_target_if_current(&wsid, newer);
        assert_eq!(store.get(&wsid).unwrap().target, None);
        assert_eq!(store.last_txid(&wsid), newer);
    }

    #[test]
    fn every_request_gets_its_own_id_whichever_window_it_writes() {
        let store = WindowTxStore::new();
        let (first, second) = (WindowServerId::new(2), WindowServerId::new(4));

        let txid_1 = store.next_txid(first);
        let target = CGRect::new(CGPoint::new(1.0, 2.0), CGSize::new(3.0, 4.0));
        store.insert(first, txid_1, target);
        store.clear_target(&first);
        let txid_2 = store.next_txid(second);
        let txid_3 = store.next_txid(first);

        assert_ne!(txid_1, txid_2);
        assert_ne!(txid_2, txid_3);
        assert_ne!(txid_1, txid_3);
        assert_eq!(store.last_txid(&first), txid_3);
        assert_eq!(store.last_txid(&second), txid_2);
    }

    #[test]
    fn set_last_txid_clears_any_stale_target() {
        let store = WindowTxStore::new();
        let wsid = WindowServerId::new(3);
        let txid_1 = store.next_txid(wsid);
        let target = CGRect::new(CGPoint::new(5.0, 6.0), CGSize::new(7.0, 8.0));
        store.insert(wsid, txid_1, target);

        let txid_2 = store.next_id();
        store.set_last_txid(wsid, txid_2);

        let record = store.get(&wsid).expect("tx record should exist");
        assert_eq!(record.txid, txid_2);
        assert_eq!(record.target, None);
    }
}
