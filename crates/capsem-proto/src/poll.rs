//! Deadline-based retry with exponential backoff.
//!
//! Single shared primitive for all "wait until ready" patterns across host
//! and guest code. Lives in capsem-proto so both capsem-core (async, host)
//! and capsem-agent (sync, guest) can use the same configuration and logic.

use std::fmt;
use std::time::{Duration, Instant};

/// Error returned when [`retry_with_backoff`] exceeds its deadline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedOut {
    pub label: &'static str,
    pub attempts: u32,
    pub timeout: Duration,
}

impl fmt::Display for TimedOut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: timed out after {} attempt(s) ({:.0?})",
            self.label, self.attempts, self.timeout,
        )
    }
}

/// Configuration for retry with exponential backoff.
///
/// Used directly for sync retries via [`retry_with_backoff`], and re-exported
/// as `PollOpts` in `capsem-core::poll` for the async variant.
pub struct RetryOpts {
    /// Human-readable label for log messages (e.g. "vm-ready", "vsock-connect").
    pub label: &'static str,
    /// Total time budget before giving up.
    pub timeout: Duration,
    /// Initial delay between attempts.
    pub initial_delay: Duration,
    /// Maximum delay between attempts (backoff caps here).
    pub max_delay: Duration,
}

impl RetryOpts {
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

impl Default for RetryOpts {
    fn default() -> Self {
        Self::default_with_label("poll")
    }
}

/// Synchronous retry with exponential backoff.
///
/// Calls `f()` repeatedly until it returns `Some(T)` or the deadline expires.
/// Returns `Ok(T)` on success, `Err(TimedOut)` on timeout.
///
/// ```ignore
/// let fd = retry_with_backoff(
///     &RetryOpts::new("vsock-connect", Duration::from_secs(30)),
///     || vsock_connect(cid, port).ok(),
/// );
/// ```
pub fn retry_with_backoff<T, F>(opts: &RetryOpts, mut f: F) -> Result<T, TimedOut>
where
    F: FnMut() -> Option<T>,
{
    let deadline = Instant::now() + opts.timeout;
    let mut delay = opts.initial_delay;
    let mut attempts: u32 = 0;

    loop {
        attempts += 1;
        if let Some(val) = f() {
            eprintln!(
                "[{}] succeeded after {} attempt(s) ({:.0?})",
                opts.label,
                attempts,
                Instant::now() + opts.timeout - deadline,
            );
            return Ok(val);
        }
        if Instant::now() >= deadline {
            eprintln!(
                "[{}] timed out after {} attempt(s) ({:.0?})",
                opts.label,
                attempts,
                opts.timeout,
            );
            return Err(TimedOut {
                label: opts.label,
                attempts,
                timeout: opts.timeout,
            });
        }
        std::thread::sleep(delay);
        delay = (delay * 2).min(opts.max_delay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn succeeds_on_first_attempt() {
        let result = retry_with_backoff(
            &RetryOpts::new("test", Duration::from_secs(1)),
            || Some(42),
        );
        assert_eq!(result, Ok(42));
    }

    #[test]
    fn succeeds_after_retries() {
        let mut count = 0;
        let result = retry_with_backoff(
            &RetryOpts::new("test", Duration::from_secs(5)),
            || {
                count += 1;
                if count >= 3 { Some(count) } else { None }
            },
        );
        assert_eq!(result, Ok(3));
    }

    #[test]
    fn times_out() {
        let result = retry_with_backoff(
            &RetryOpts {
                label: "test",
                timeout: Duration::from_millis(100),
                initial_delay: Duration::from_millis(30),
                max_delay: Duration::from_millis(50),
            },
            || None::<()>,
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.label, "test");
        assert_eq!(err.timeout, Duration::from_millis(100));
    }

    #[test]
    fn backoff_doubles_then_caps() {
        // Verify the progression: 50 -> 100 -> 200 -> 400 -> 500 (capped)
        let opts = RetryOpts::default();
        let mut delay = opts.initial_delay;
        let expected = [
            Duration::from_millis(100),
            Duration::from_millis(200),
            Duration::from_millis(400),
            Duration::from_millis(500),
            Duration::from_millis(500),
        ];
        for exp in &expected {
            delay = (delay * 2).min(opts.max_delay);
            assert_eq!(delay, *exp);
        }
    }

    #[test]
    fn default_opts() {
        let opts = RetryOpts::default();
        assert_eq!(opts.label, "poll");
        assert_eq!(opts.timeout, Duration::from_secs(30));
        assert_eq!(opts.initial_delay, Duration::from_millis(50));
        assert_eq!(opts.max_delay, Duration::from_millis(500));
    }
}
