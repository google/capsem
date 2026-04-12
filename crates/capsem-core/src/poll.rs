//! Deadline-based polling with exponential backoff.
//!
//! Async variant of the shared retry primitive in `capsem_proto::poll`.
//! Uses the same `RetryOpts` configuration (re-exported as `PollOpts`).

use std::future::Future;
use tracing::{debug, warn};

/// Re-export as PollOpts to preserve existing call sites.
pub type PollOpts = capsem_proto::poll::RetryOpts;

/// Poll an async predicate with exponential backoff until it returns `Some(T)`
/// or the deadline expires.
///
/// Returns `Ok(T)` on success, `Err(TimedOut)` on timeout.
///
/// ```ignore
/// let result = poll_until(
///     PollOpts::new("vm-ready", Duration::from_secs(30)),
///     || async {
///         if socket_path.exists() { Some(()) } else { None }
///     },
/// ).await;
/// ```
pub async fn poll_until<T, F, Fut>(opts: PollOpts, mut f: F) -> Result<T, capsem_proto::poll::TimedOut>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    let deadline = tokio::time::Instant::now() + opts.timeout;
    let mut delay = opts.initial_delay;
    let mut attempts: u32 = 0;

    loop {
        attempts += 1;
        if let Some(val) = f().await {
            debug!(
                label = opts.label,
                attempts,
                elapsed_ms = (tokio::time::Instant::now() - (deadline - opts.timeout)).as_millis() as u64,
                "poll succeeded"
            );
            return Ok(val);
        }
        if tokio::time::Instant::now() >= deadline {
            warn!(
                label = opts.label,
                attempts,
                timeout_ms = opts.timeout.as_millis() as u64,
                "poll timed out"
            );
            return Err(capsem_proto::poll::TimedOut {
                label: opts.label,
                attempts,
                timeout: opts.timeout,
            });
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
