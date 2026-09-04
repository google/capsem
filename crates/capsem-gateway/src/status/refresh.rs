//! Coalescing of concurrent `/status` reads without ever serving a stale one.
//!
//! The rule the gateway must keep: a `/status` answer comes from a service
//! read that *began after the request arrived*. VM mutations go over the
//! service socket, outside the gateway, so a read that started earlier may
//! predate a `capsem create` the caller just made (the TUI would then reject
//! a session that already exists). The old handler kept that rule with a
//! mutex around every fetch, so N concurrent polls (browser, tray, TUI)
//! cost N sequential round trips and queued behind each other.
//!
//! This gate keeps the rule and bounds the cost: a poll that finds no fetch
//! in flight leads one; a poll that arrives while one is in flight waits for
//! it, then joins (or leads) the *next* fetch, which by construction began
//! after it arrived. Any number of concurrent polls cost at most two fetches.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::{watch, Mutex};

pub(super) struct RefreshGate<T> {
    /// Held by the poll currently fetching.
    fetching: Mutex<()>,
    /// Count of fetches started so far; a poll arriving when this reads `n`
    /// needs a fetch numbered `n + 1` or later.
    started: AtomicU64,
    /// The latest completed fetch, numbered.
    completed: watch::Sender<(u64, Option<Arc<T>>)>,
}

/// What a poll got back: the read, and whether this poll performed it
/// (only the fetcher reports lifecycle transitions, so each is reported once).
pub(super) struct Read<T> {
    pub(super) value: Arc<T>,
    pub(super) led: bool,
}

impl<T> RefreshGate<T> {
    pub(super) fn new() -> Self {
        Self {
            fetching: Mutex::new(()),
            started: AtomicU64::new(0),
            completed: watch::channel((0, None)).0,
        }
    }

    /// Fetches this gate has started.
    #[cfg(test)]
    pub(super) fn fetches_started(&self) -> u64 {
        self.started.load(Ordering::SeqCst)
    }

    /// Return a value from a `fetch` that began after this call did,
    /// performing the fetch only when nobody else's will do.
    pub(super) async fn read<F, Fut>(&self, fetch: F) -> Read<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        let needed = self.started.load(Ordering::SeqCst) + 1;
        let mut completed = self.completed.subscribe();
        let mut fetch = Some(fetch);
        loop {
            {
                let latest = completed.borrow_and_update();
                if latest.0 >= needed {
                    if let Some(value) = latest.1.as_ref() {
                        return Read {
                            value: Arc::clone(value),
                            led: false,
                        };
                    }
                }
            }
            if let Ok(_guard) = self.fetching.try_lock() {
                let number = self.started.fetch_add(1, Ordering::SeqCst) + 1;
                debug_assert!(number >= needed);
                let fetch = fetch.take().expect("a poll fetches at most once");
                let value = Arc::new(fetch().await);
                // channel-closed-ok: no receiver means no other poll waited.
                let _ = self.completed.send((number, Some(Arc::clone(&value))));
                return Read { value, led: true };
            }
            if completed.changed().await.is_err() {
                // The sender lives as long as the gate; unreachable in practice.
                let value = Arc::new(fetch.take().expect("a poll fetches at most once")().await);
                return Read { value, led: true };
            }
        }
    }
}

#[cfg(test)]
mod tests;
