//! One latest browser request per live owner. It is deliberately neither
//! checkpointed nor logged: an auth URL must not outlive this session owner.
use capsem_proto::browser::{browser_url, BrowserRequest};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::watch;

pub(crate) struct SessionBrowser {
    pending: watch::Sender<Option<BrowserRequest>>,
    sequence: AtomicU64,
    handled: AtomicU64,
}

impl Default for SessionBrowser {
    fn default() -> Self {
        Self {
            pending: watch::channel(None).0,
            sequence: AtomicU64::new(0),
            handled: AtomicU64::new(0),
        }
    }
}

impl SessionBrowser {
    pub(crate) fn request(&self, url: &str) -> Result<(), &'static str> {
        let url = browser_url(url)?;
        let accepted = self.pending.send_if_modified(|current| {
            let Some(id) = self.sequence.load(Ordering::Relaxed).checked_add(1) else {
                return false;
            };
            self.sequence.store(id, Ordering::Relaxed);
            *current = Some(BrowserRequest {
                id,
                url,
                owner_generation: None,
            });
            true
        });
        if accepted {
            Ok(())
        } else {
            Err("browser request identity exhausted")
        }
    }

    pub(crate) fn acknowledge(&self, expected: u64) -> bool {
        let mut acknowledged = false;
        self.pending.send_if_modified(|current| {
            if expected > 0 && current.as_ref().is_some_and(|request| request.id == expected) {
                self.handled.store(expected, Ordering::Relaxed);
                *current = None;
                acknowledged = true;
                return true;
            }
            acknowledged = expected > 0 && self.handled.load(Ordering::Relaxed) == expected;
            false
        });
        acknowledged
    }

    /// Read while holding a watch borrow so clear state and revision stay paired.
    pub(crate) fn revision(&self) -> u64 {
        self.sequence.load(Ordering::Relaxed)
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<Option<BrowserRequest>> {
        self.pending.subscribe()
    }
}

#[cfg(test)]
mod tests;
