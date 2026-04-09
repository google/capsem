//! Deadline-based polling with exponential backoff.
//!
//! Single utility for all "wait until ready" patterns. Replaces ad-hoc
//! sleep loops scattered across the codebase.

use std::future::Future;
use std::time::Duration;
use tokio::time::Instant;
use tracing::{debug, warn};

/// Configuration for a poll loop with exponential backoff.
pub struct PollOpts {
    /// Human-readable label for log messages (e.g. "vm-ready", "service-socket").
    pub label: &'static str,
    /// Total time budget before giving up.
    pub timeout: Duration,
    /// Initial delay between attempts.
    pub initial_delay: Duration,
    /// Maximum delay between attempts (backoff caps here).
    pub max_delay: Duration,
}

impl PollOpts {
    /// Quick constructor with a label and timeout; uses default backoff.
    pub fn new(label: &'static str, timeout: Duration) -> Self {
        Self {
            timeout,
            ..Self::default_with_label(label)
        }
    }

    fn default_with_label(label: &'static str) -> Self {
        Self {
            label,
            timeout: Duration::from_secs(30),
            initial_delay: Duration::from_millis(50),
            max_delay: Duration::from_millis(500),
        }
    }
}

impl Default for PollOpts {
    fn default() -> Self {
        Self::default_with_label("poll")
    }
}

/// Poll an async predicate with exponential backoff until it returns `Some(T)`
/// or the deadline expires.
///
/// Returns `Ok(T)` on success, `Err(())` on timeout.
///
/// ```ignore
/// let result = poll_until(
///     PollOpts::new("vm-ready", Duration::from_secs(30)),
///     || async {
///         if socket_path.exists() { Some(()) } else { None }
///     },
/// ).await;
/// ```
pub async fn poll_until<T, F, Fut>(opts: PollOpts, mut f: F) -> Result<T, ()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    let deadline = Instant::now() + opts.timeout;
    let mut delay = opts.initial_delay;
    let mut attempts: u32 = 0;

    loop {
        attempts += 1;
        if let Some(val) = f().await {
            debug!(
                label = opts.label,
                attempts,
                elapsed_ms = (Instant::now() - (deadline - opts.timeout)).as_millis() as u64,
                "poll succeeded"
            );
            return Ok(val);
        }
        if Instant::now() >= deadline {
            warn!(
                label = opts.label,
                attempts,
                timeout_ms = opts.timeout.as_millis() as u64,
                "poll timed out"
            );
            return Err(());
        }
        debug!(
            label = opts.label,
            attempts,
            next_delay_ms = delay.as_millis() as u64,
            "poll attempt failed, retrying"
        );
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(opts.max_delay);
    }
}
