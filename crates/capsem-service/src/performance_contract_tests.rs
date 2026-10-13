use super::asset_routes::asset_manifest_status;
use super::{tests, ShutdownMode};
use std::sync::Arc;

#[test]
fn asset_manifest_status_shares_warm_bytes_and_invalidates_same_size_edits() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("manifest.json");
    std::fs::write(&manifest, b"{}").unwrap();
    let state = tests::make_asset_state(dir.path().to_path_buf());

    let first = asset_manifest_status(&state).unwrap();
    let warm = asset_manifest_status(&state).unwrap();
    assert!(
        Arc::ptr_eq(&first, &warm),
        "a warm status read must share one cached allocation"
    );

    std::fs::write(&manifest, b"[]").unwrap();
    let changed = asset_manifest_status(&state).unwrap();
    assert!(
        !Arc::ptr_eq(&first, &changed),
        "same-size manifest byte edits must invalidate the cache"
    );
}

#[test]
fn shutdown_mode_keeps_retained_and_discarded_teardown_distinct() {
    assert!(ShutdownMode::Retain.retains_state());
    assert!(!ShutdownMode::Discard.retains_state());
    assert!(ShutdownMode::Retain.exit_timeout() > ShutdownMode::Discard.exit_timeout());
}
