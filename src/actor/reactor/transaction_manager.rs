use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};

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
