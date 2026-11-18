use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};

/// Names one request that changes window frames. Ids come from one counter
/// ([`TransactionCounter`]), so no two requests share one and they are ordered as issued: a
/// read stamped with an older id was made before the newer request was applied. (The counter
/// wraps after 2^32 requests; a session does not get near that.)
#[derive(
    Default, Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct TransactionId(u32);

impl TransactionId {
    /// The id as a number, for plain data (a recorded trace, the Roc core).
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
