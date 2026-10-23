use super::*;

#[test]
fn missing_requested_collector_fails_before_any_child_or_store() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("ran");
    std::fs::write(
        root.path().join("routes"),
        format!(
            "touch '{}'; echo '{{\"metrics\":{{\"value\":{{\"unit\":\"count\",\"samples\":[1]}}}}}}'",
            marker.display()
        ),
    )
    .unwrap();
    let store = root.path().join("measurements.db");
    let result = run_dimensions(
        &[schema::Dimension::Routes, schema::Dimension::Stream],
        root.path(),
        &store,
        Duration::from_secs(2),
        Some("/bin/sh"),
        false,
        "test",
        "head",
        Vec::new(),
    );
    let error = result.unwrap_err().to_string();
    assert!(error.contains("stream"), "{error}");
    assert!(!marker.exists(), "partial workloads must not run before admission");
    assert!(!store.exists(), "failed admission must not produce a partial record");
}

#[test]
fn default_dimension_inventory_names_missing_collectors() {
    let root = tempfile::tempdir().unwrap();
    let error = run_dimensions(
        schema::Dimension::ALL,
        root.path(),
        &root.path().join("store.db"),
        Duration::from_secs(2),
        None,
        false,
        "test",
        "head",
        Vec::new(),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("routes") && error.contains("vsock"), "{error}");
}
