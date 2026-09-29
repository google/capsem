//! The counter snapshot a polled route reads, held in memory by the DB object.
//!
//! `/info`, `stats/summary`, `security/status` and the list are polled by the
//! UI, the TUI and the tray, per VM, on a timer. Answering each poll from the
//! file -- even as one cached primary-key lookup -- cost a round trip to the
//! reader worker and a SQLite read per poll, so the service's CPU scaled with
//! how often clients asked rather than with what the session did.
//!
//! The reader worker keeps the snapshot here instead. It reads it from the
//! file when SQLite's `data_version` says another connection committed:
//! when it starts, on every `ready()`, and on its own every
//! [`HOT_REFRESH_INTERVAL`] while the handle is open. A poll takes the last
//! published snapshot under a lock and touches neither the worker nor
//! SQLite. The snapshot is never older than the interval, and `ready()` is the
//! read-after-write barrier: it returns only after the snapshot caught up.

use super::*;
use crate::counters::LedgerCounters;

/// How stale a published snapshot may get while nobody calls `ready()`.
///
/// Every open handle pays one `PRAGMA data_version` per interval, which is
/// what keeps a poll free. Clients poll on the order of a second, so a quarter
/// of one is not something a person watching the UI can see.
pub(super) const HOT_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// The snapshot the worker last published, and the `data_version` it was
/// read at.
#[derive(Default)]
struct Published {
    data_version: Option<i64>,
    snapshot: Option<Result<Arc<LedgerCounters>, String>>,
}

/// Shared between a handle and its reader worker.
#[derive(Default)]
pub(super) struct HotCounters {
    published: Mutex<Published>,
    /// Moves every time a different snapshot, or a different failure, is
    /// published: the freshness answer for [`ReadCacheDomain::LedgerCounters`].
    epoch: AtomicU64,
}

impl HotCounters {
    /// The last published snapshot, or `None` before the worker's first
    /// look at the file.
    pub(super) fn current(&self) -> Option<Result<Arc<LedgerCounters>, String>> {
        self.published
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot
            .clone()
    }

    pub(super) fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// Worker side: bring the published snapshot up to the file.
    ///
    /// Reads nothing but `data_version` when the ledger has not moved since
    /// the last snapshot. `data_version` is read before the snapshot, so a
    /// commit landing between the two leaves an older version beside a newer
    /// snapshot, which the next refresh reads again -- never the reverse,
    /// which would pin a stale snapshot until the writer committed again.
    /// A failure is published too, and retried on the next refresh: a ledger
    /// that is not ready says so rather than reporting zeros.
    pub(super) fn refresh(&self, reader: &DbReader) {
        let data_version = match reader.data_version() {
            Ok(data_version) => data_version,
            Err(error) => return self.publish(None, Err(format!("session db data version unreadable: {error}"))),
        };
        let current = {
            let published = self.published.lock().unwrap_or_else(|e| e.into_inner());
            published.data_version == Some(data_version)
        };
        if current {
            return;
        }
        match reader
            .ready()
            .and_then(|()| crate::counters::load(reader.connection()).map_err(|error| error.to_string()))
        {
            Ok(counters) => self.publish(Some(data_version), Ok(Arc::new(counters))),
            Err(error) => self.publish(None, Err(error)),
        }
    }

    fn publish(&self, data_version: Option<i64>, snapshot: Result<Arc<LedgerCounters>, String>) {
        let mut published = self.published.lock().unwrap_or_else(|e| e.into_inner());
        let changed = match (&published.snapshot, &snapshot) {
            (Some(Ok(old)), Ok(new)) => old != new,
            (Some(Err(old)), Err(new)) => old != new,
            _ => true,
        };
        published.data_version = data_version;
        if changed {
            published.snapshot = Some(snapshot);
            // Under the lock, so a reader never sees the new epoch beside the
            // old snapshot.
            self.epoch.fetch_add(1, Ordering::AcqRel);
        }
        drop(published);
    }
}
