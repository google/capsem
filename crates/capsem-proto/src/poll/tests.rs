//! Tests for `poll` (extracted from inline `mod tests`).

use super::*;

#[test]
fn succeeds_on_first_attempt() {
    let result = retry_with_backoff(&RetryOpts::new("test", Duration::from_secs(1)), || Some(42));
    assert_eq!(result, Ok(42));
}

#[test]
fn succeeds_after_retries() {
    let mut count = 0;
    let result = retry_with_backoff(&RetryOpts::new("test", Duration::from_secs(5)), || {
        count += 1;
        if count >= 3 {
            Some(count)
        } else {
            None
        }
    });
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
