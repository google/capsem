use super::{process_exit_poll_options, profile_status_cache, tests, PollOpts, ShutdownMode};
use std::sync::Arc;

#[test]
fn profile_status_cache_shares_warm_bytes_and_invalidates_same_size_edits() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("manifest.json");
    std::fs::write(&manifest, b"{}").unwrap();
    let state = tests::make_asset_state(dir.path().to_path_buf());

    let first = profile_status_cache(&state).unwrap();
    let warm = profile_status_cache(&state).unwrap();
    assert!(
        Arc::ptr_eq(&first, &warm),
        "a warm status read must share one cached allocation"
    );

    std::fs::write(&manifest, b"[]").unwrap();
    let changed = profile_status_cache(&state).unwrap();
    assert!(
        !Arc::ptr_eq(&first, &changed),
        "same-size manifest byte edits must invalidate the cache"
    );
}

#[test]
fn process_exit_poll_does_not_use_the_generic_exponential_backoff() {
    let timeout = std::time::Duration::from_secs(1);
    let exit = process_exit_poll_options(timeout);
    let generic = PollOpts::new("generic", timeout);

    assert_eq!(exit.timeout, timeout);
    assert_eq!(exit.initial_delay, exit.max_delay);
    assert!(exit.max_delay < generic.max_delay);
}

#[test]
fn shutdown_mode_keeps_retained_and_discarded_teardown_distinct() {
    assert!(ShutdownMode::Retain.retains_state());
    assert!(!ShutdownMode::Discard.retains_state());
    assert!(ShutdownMode::Retain.exit_timeout() > ShutdownMode::Discard.exit_timeout());
}
