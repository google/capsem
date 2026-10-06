use super::*;

#[tokio::test]
async fn catalog_root_failure_refuses_create_before_staging_or_launch() {
    let root_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let arch = stage::catalog_architecture().unwrap();
    let catalog = json!({
        "schema_version": 1,
        "channel": "stable",
        "generated_at": "2026-10-01T00:00:00Z",
        "entries": {"published": {"description": "Published root", "versions": [{
            "image": format!("registry.example/app@sha256:{}", "f".repeat(64)),
            "platforms": [format!("linux/{}", arch.as_str())],
            "contract": 1,
            "erofs": {(arch.as_str()): format!("sha256:{}", "8".repeat(64))}
        }]}}
    });
    let fx = fixture(FixtureImages {
        catalog: Some(catalog),
        root_calls: Arc::clone(&root_calls),
        ..images()
    });
    let owner = owner_accepting_stage_and_launch(&fx.uds_path, 4);
    let mut request = spec(None);
    request.image = "published".into();
    let generation = fx.state.containers.begin("box", &request.image);
    let result = run(&fx.state, "box", generation, request).await;
    owner.abort();
    assert!(
        result.unwrap_err().contains("fixture artifact unavailable"),
        "a catalog-promised root must not silently fall back to layer extraction"
    );
    assert_eq!(root_calls.load(Ordering::Relaxed), 1);
    assert!(!fx.workspace.join(capsem_core::container::STAGE).exists());
    assert!(!fx.workspace.join(".capsem-container-running").exists());
}
