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

use tokio::sync::Mutex;

pub(super) struct RefreshGate<T> {
    /// Cancellation releases a waiter through the mutex itself; there is no
    /// separate notification that can be missed or abandoned by its sender.
    completed: Mutex<Option<(u64, Arc<T>)>>,
    /// Count of fetches started so far; a poll arriving when this reads `n`
    /// needs a fetch numbered `n + 1` or later.
    started: AtomicU64,
}

impl<T> RefreshGate<T> {
    pub(super) fn new() -> Self {
        Self {
            completed: Mutex::new(None),
            started: AtomicU64::new(0),
        }
    }

    /// Fetches this gate has started.
    #[cfg(test)]
    pub(super) fn fetches_started(&self) -> u64 {
        self.started.load(Ordering::SeqCst)
    }

    /// Return a value from a `fetch` that began after this call did,
    /// performing the fetch only when nobody else's will do.
    pub(super) async fn read<F, Fut>(&self, fetch: F) -> Arc<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        let needed = self.started.load(Ordering::SeqCst) + 1;
        let mut completed = self.completed.lock().await;
        if let Some((number, value)) = completed.as_ref() {
            if *number >= needed {
                return Arc::clone(value);
            }
        }
        let number = self.started.fetch_add(1, Ordering::SeqCst) + 1;
        let value = Arc::new(fetch().await);
        *completed = Some((number, Arc::clone(&value)));
        value
    }
}

#[cfg(test)]
mod tests;
