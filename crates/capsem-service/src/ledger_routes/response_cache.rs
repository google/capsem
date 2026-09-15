//! Session-ledger route response bytes, cached on the logger's change generation.
use super::*;

fn session_response_cache_key(vm_id: &str, route_key: &str) -> String {
    format!("{vm_id}:{route_key}")
}

/// Outcome of looking up a cached session-ledger route response.
pub(crate) enum SessionResponseCache {
    Hit(Bytes),
    Miss(SessionResponseCacheSlot),
}

/// Where a freshly built response is stored, pinned to the ledger generation
/// observed before the route queried.
///
/// Freshness is the logger handle's answer: `ready()` syncs the external
/// reader from disk and advances `read_cache_epoch` when another connection
/// committed. Taking the epoch after that sync and before the query means a
/// commit landing mid-query leaves the bytes under an older epoch -- a miss on
/// the next read, never a stale hit.
pub(crate) struct SessionResponseCacheSlot {
    cache_key: String,
    db_epoch: u64,
}

pub(crate) async fn session_response_cache_lookup(
    state: &ServiceState,
    vm_id: &str,
    route_key: &str,
    ledger: &str,
    db_path: &StdPath,
) -> Result<SessionResponseCache, AppError> {
    let db = open_ready_session_db(state, vm_id, ledger, db_path).await?;
    let db_epoch = db.read_cache_epoch(capsem_logger::ReadCacheDomain::All);
    let cache_key = session_response_cache_key(vm_id, route_key);
    let cached = state
        .stats_detail_response_cache
        .lock()
        .unwrap()
        .get(&cache_key)
        .filter(|cached| cached.db_epoch == db_epoch)
        .map(|cached| Bytes::from(cached.bytes.clone()));
    Ok(match cached {
        Some(bytes) => SessionResponseCache::Hit(bytes),
        None => SessionResponseCache::Miss(SessionResponseCacheSlot { cache_key, db_epoch }),
    })
}

impl SessionResponseCacheSlot {
    pub(crate) fn store(self, state: &ServiceState, bytes: &[u8]) {
        state.stats_detail_response_cache.lock().unwrap().insert(
            self.cache_key,
            CachedLedgerResponse {
                db_epoch: self.db_epoch,
                bytes: bytes.to_vec(),
            },
        );
    }
}
