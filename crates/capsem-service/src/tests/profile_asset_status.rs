use super::*;

#[test]
fn reconcile_claim_keeps_routes_unsettled_until_cache_publication() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    state
        .asset_reconcile_inflight
        .store(true, Ordering::Release);
    state.asset_reconcile.lock().unwrap().last_downloaded = Some(3);

    assert!(asset_reconcile_has_route_fields(&state));
    let status = refresh_reconcile_fields(&state, json!({ "downloading": false }));
    assert_eq!(status["downloading"], true);
    assert_eq!(status["downloaded"], 3);
}

#[cfg(unix)]
#[tokio::test]
async fn does_not_read_asset_contents_on_hot_path() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (config_root, profile) = install_file_asset_profile_fixture(&dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let assets_dir = dir.path().join("assets");
    let state = make_asset_state(assets_dir.clone());
    let app = build_service_router(state);

    let (status, ensured) = route_request(
        app.clone(),
        axum::http::Method::POST,
        "/profiles/code/assets/ensure",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ensured}");
    assert_eq!(ensured["started"], true);
    let completed = asset_wait::wait_for_profile_assets(&app).await;
    assert_eq!(completed["ready"], true, "{completed}");

    let arch = capsem_core::net::policy_config::current_profile_arch();
    let rootfs = &profile.assets.current_arch_assets().unwrap().rootfs;
    let rootfs_path = assets_dir
        .join(arch)
        .join(capsem_core::asset_manager::hash_filename(
            &rootfs.name,
            rootfs
                .hash
                .as_deref()
                .expect("rootfs hash")
                .strip_prefix("blake3:")
                .unwrap(),
        ));

    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&rootfs_path, std::fs::Permissions::from_mode(0o000)).unwrap();
    let (status, hot_status) =
        route_request(app, axum::http::Method::GET, "/profiles/status", None).await;
    assert_eq!(status, StatusCode::OK, "{hot_status}");
    assert_eq!(
        hot_status["profiles"][0]["ready"], true,
        "profile status is a hot readiness route and must not hash/read asset contents"
    );

    std::fs::set_permissions(&rootfs_path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let loaded =
        capsem_core::net::policy_config::Profile::load_from_dir(config_root.join("profiles/code"))
            .unwrap();
    std::fs::set_permissions(&rootfs_path, std::fs::Permissions::from_mode(0o000)).unwrap();
    let error = loaded
        .check(&assets_dir, arch)
        .expect_err("explicit profile verification still reads and rejects unreadable assets");
    assert!(error.contains("rootfs"), "{error}");
    std::fs::set_permissions(&rootfs_path, std::fs::Permissions::from_mode(0o644)).unwrap();
}
