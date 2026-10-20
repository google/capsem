use super::*;

#[test]
fn suspend_confirm_timeout_matches_public_api_budget() {
    assert_eq!(
        SUSPEND_CONFIRM_TIMEOUT_SECS, 45,
        "service must not kill an in-progress KVM checkpoint before clients' documented suspend timeout"
    );
}

#[test]
fn system_status_documents_preserve_exact_installed_manifest_and_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let manifest_path = dir.path().join("manifest.json");
    let metadata_path = dir.path().join("manifest-metadata.json");
    let manifest = serde_json::json!({
        "channel": "stable",
        "version": "1.0.142",
        "packages": [{"name": "Capsem.pkg", "binaries": [{"name": "capsem"}]}],
        "profiles": {"code": {"name": "Code", "description": "Coding", "revision": "2026.0703.2"}}
    });
    let metadata = serde_json::json!({
        "schema": "capsem.manifest_metadata.v1",
        "manifest_url": "https://release.capsem.org/assets/stable/manifest.json",
        "installed_at": 1000,
        "refreshed_at": 1100,
        "checked_at": 1200,
        "update_available": false
    });
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    std::fs::write(&metadata_path, serde_json::to_vec(&metadata).unwrap()).unwrap();

    assert_eq!(read_installed_status_document(&manifest_path).unwrap(), manifest);
    assert_eq!(
        read_manifest_metadata_status_document(&metadata_path).unwrap(),
        metadata
    );
}

#[test]
fn system_status_rejects_noncanonical_manifest_metadata_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("manifest-metadata.json");
    std::fs::write(&path, r#"{"schema":"capsem.other.v1"}"#).unwrap();

    let error = read_manifest_metadata_status_document(&path).unwrap_err();

    assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(error.body.error.contains("capsem.manifest_metadata.v1"));
}

#[tokio::test]
async fn system_status_route_returns_exact_installed_documents_in_one_response() {
    let (state, _dir) = make_test_state_with_tempdir();
    std::fs::create_dir_all(&state.assets_dir).unwrap();
    let arch = capsem_assets::asset_manager::host_manifest_arch();
    let digest = |ch: char| ch.to_string().repeat(64);
    let manifest = serde_json::json!({
        "channel": "stable",
        "version": "1.0.142",
        "status": "current",
        "packages": [{
            "name": "Capsem.pkg",
            "version": "0.0.0",
            "status": "current",
            "binaries": [{"name": "capsem", "installed_path": "/usr/local/bin/capsem"}],
            "evidence": [{"kind": "sbom", "url": "https://example.test/capsem.spdx.json"}]
        }],
        "runtime": {
            "revision": "0.7.0-0123456789ab",
            "status": "current",
            "architectures": [{
                "architecture": arch,
                "image_revision": "0.7.0-0123456789ab",
                "images": [
                    {"kind":"kernel","name":"vmlinuz","bytes":10,"status":"current","digest":{"blake3":digest('a'),"sha256":digest('1')}},
                    {"kind":"initrd","name":"initrd.img","bytes":20,"status":"current","digest":{"blake3":digest('b'),"sha256":digest('2')}},
                    {"kind":"rootfs","name":"rootfs.erofs","bytes":30,"status":"current","digest":{"blake3":digest('c'),"sha256":digest('3')}}
                ],
                "evidence": [{
                    "kind":"obom",
                    "url":"https://example.test/obom.json",
                    "bytes":40,
                    "digest":{"blake3":digest('d'),"sha256":digest('4')}
                }]
            }]
        }
    });
    let metadata = serde_json::json!({
        "schema": "capsem.manifest_metadata.v1",
        "manifest_url": "https://release.capsem.org/assets/stable/manifest.json",
        "installed_at": 1000,
        "checked_at": 1100,
        "checked_url": "https://release.capsem.org/assets/stable/manifest.json",
        "validation_status": "valid",
        "update_available": false
    });
    std::fs::write(
        state.assets_dir.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(
        state.assets_dir.join("manifest-metadata.json"),
        serde_json::to_vec(&metadata).unwrap(),
    )
    .unwrap();

    let (status, body) = route_request(
        build_service_router(state),
        axum::http::Method::GET,
        "/system/status",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["manifest"], manifest);
    assert_eq!(body["manifest_metadata"], metadata);
    let assets: capsem_api::AssetStatus = serde_json::from_value(body["assets"].clone())
        .expect("the real asset status response must satisfy the generated SDK contract");
    assert_eq!(assets.current_arch, arch);
    assert!(body.get("profiles").is_none(), "{body}");
    assert_eq!(body["updates"]["checked_at"], 1100);
}

#[tokio::test]
async fn gateway_status_route_returns_vms_assets_and_updates_in_one_response() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    install_test_runtime_assets(&state);
    insert_fake_instance(&state, "gateway-status-vm", std::process::id());

    let (status, body) = route_request(
        build_service_router(state),
        axum::http::Method::GET,
        "/internal/gateway/status",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sandboxes"][0]["id"], "gateway-status-vm");
    assert_eq!(body["assets"]["ready"], true);
    assert!(body["updates"].is_object());
}

#[test]
fn process_env_allowlist_forwards_mcp_timeout_knobs() {
    assert!(
        PROCESS_ENV_ALLOWLIST.contains(&"CAPSEM_HOME"),
        "CAPSEM_HOME must reach capsem-process so tests and custom installs use the same config root as capsem-service"
    );

    for key in [
        "CAPSEM_CORP_CONFIG",
        "CAPSEM_CREDENTIAL_STORE_PATH",
        "CAPSEM_MCP_DEFAULT_TIMEOUT_SECS",
        "CAPSEM_MCP_TOOL_CALL_TIMEOUT_SECS",
        "CAPSEM_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS",
        "CAPSEM_EXPERIMENTAL_EROFS_DAX",
    ] {
        assert!(
            PROCESS_ENV_ALLOWLIST.contains(&key),
            "{key} must reach capsem-process because child-only boot/runtime config is read there"
        );
    }
}

#[test]
fn find_orphan_capsem_pids_matches_capsem_process_under_run_dir() {
    let run_dir = PathBuf::from("/var/folders/XY/T/capsem-test-abc");
    let ps = "\
  1502 /path/to/cache/target/cargo/debug/capsem-process --env CAPSEM_VM_ID=orphan --id orphan --session-dir /var/folders/XY/T/capsem-test-abc/sessions/orphan --uds-path /tmp/capsem/abc.sock
  1742 /path/to/cache/target/cargo/debug/capsem-process --id victim --session-dir /var/folders/XY/T/capsem-test-abc/persistent/victim --uds-path /tmp/capsem/def.sock
";
    let pids = find_orphan_capsem_pids(&process_fixture(ps), &run_dir);
    assert_eq!(pids, vec![1502, 1742]);
}

#[test]
fn find_orphan_capsem_pids_skips_processes_for_other_run_dirs() {
    let run_dir = PathBuf::from("/var/folders/XY/T/capsem-test-mine");
    let ps = "\
  1502 /path/to/cache/target/cargo/debug/capsem-process --session-dir /var/folders/XY/T/capsem-test-other/sessions/foo
  1742 /path/to/cache/target/cargo/debug/capsem-process --session-dir /var/folders/XY/T/capsem-test-mine/sessions/bar
";
    let pids = find_orphan_capsem_pids(&process_fixture(ps), &run_dir);
    assert_eq!(pids, vec![1742], "must not match neighbouring test run dirs");
}

#[test]
fn find_orphan_capsem_pids_skips_non_capsem_process_binaries() {
    let run_dir = PathBuf::from("/var/folders/XY/T/capsem-test-abc");
    // A stray cargo invocation that happens to mention the run_dir path.
    let ps = "\
  99 /bin/cargo build --manifest-path /var/folders/XY/T/capsem-test-abc/Cargo.toml
  1502 /path/to/cache/target/cargo/debug/capsem-process --session-dir /var/folders/XY/T/capsem-test-abc/sessions/orphan
";
    let pids = find_orphan_capsem_pids(&process_fixture(ps), &run_dir);
    assert_eq!(
        pids,
        vec![1502],
        "match must require the exact capsem-process executable argument"
    );
}

#[test]
fn find_orphan_capsem_pids_returns_empty_on_no_match() {
    let run_dir = PathBuf::from("/var/folders/XY/T/capsem-test-empty");
    let ps = "\
  1 /sbin/launchd
  42 /usr/bin/bash
";
    let pids = find_orphan_capsem_pids(&process_fixture(ps), &run_dir);
    assert!(pids.is_empty());
}

#[test]
fn orphan_selection_cannot_target_neighbouring_roots_or_incidental_command_text() {
    let run_dir = PathBuf::from("/tmp/capsem-owned");
    let table = "\
  41 /usr/bin/capsem-process --session-dir /tmp/capsem-owned-neighbour/sessions/other
  42 /bin/echo capsem-process --session-dir /tmp/capsem-owned/sessions/other
  43 /usr/bin/capsem-process-helper --session-dir /tmp/capsem-owned/sessions/other
  44 /usr/bin/capsem-process --env FAKE=--session-dir /tmp/capsem-owned/sessions/other
  45 /usr/bin/capsem-process --session-dir /tmp/capsem-owned/sessions/../other
  46 /usr/bin/capsem-process --session-dir /tmp/capsem-owned/sessions/mine
";
    assert_eq!(
        find_orphan_capsem_pids(&process_fixture(table), &run_dir),
        vec![46],
        "startup must retain neighbouring and unrelated workloads"
    );
}

#[test]
fn orphan_selection_preserves_argument_boundaries_and_spaceful_paths() {
    use capsem_foundation::proctable::Process;
    let run_dir = PathBuf::from("/tmp/capsem owned");
    let process = |pid, argv: &[&str]| Process::from_argv(pid, 1, argv.iter().map(Into::into).collect());
    let table = [
        process(
            51,
            &[
                "/tmp/installed bin/capsem-process",
                "--session-dir",
                "/tmp/capsem owned/sessions/mine",
            ],
        ),
        process(
            52,
            &[
                "capsem-process",
                "--env",
                "FAKE=--session-dir /tmp/capsem owned/sessions/mine",
            ],
        ),
        process(
            53,
            &[
                "capsem-process",
                "--session-dir",
                "/tmp/capsem owned/sessions/mine",
                "--session-dir",
                "/tmp/other/persistent/vm",
            ],
        ),
        process(54, &["capsem-process", "--session-dir", "/tmp/capsem owned/sessions"]),
        process(
            55,
            &[
                "capsem-process",
                "--session-dir",
                "/tmp/capsem owned/persistent/mine/deeper",
            ],
        ),
    ];
    let incomplete = process(
        56,
        &[
            "capsem-process",
            "--session-dir",
            "/tmp/capsem owned/sessions/mine",
            "--session-dir",
        ],
    );
    assert!(find_orphan_capsem_pids(&[incomplete], &run_dir).is_empty());
    assert_eq!(find_orphan_capsem_pids(&table, &run_dir), vec![51]);
}

#[tokio::test]
async fn service_wide_ledger_routes_are_db_backed_and_empty_without_session_dbs() {
    let state = make_test_state();

    let Json(latest) = handle_service_security_latest(
        State(Arc::clone(&state)),
        Query(SecurityLedgerQuery { limit: Some(10) }),
    )
    .await
    .expect("service security latest should return an empty ledger");
    assert!(latest.is_empty());

    let Json(status) = handle_service_security_status(State(Arc::clone(&state)))
        .await
        .expect("service security status should return empty DB aggregate");
    assert_eq!(status["total"], 0);
    assert!(status["sessions"].as_array().unwrap().is_empty());

    let Json(detections) = handle_service_detection_latest(
        State(Arc::clone(&state)),
        Query(SecurityLedgerQuery { limit: Some(10) }),
    )
    .await
    .expect("service detection latest should return an empty ledger");
    assert!(detections.is_empty());

    let Json(detection_status) = handle_service_detection_status(State(state))
        .await
        .expect("service detection status should return empty DB aggregate");
    assert_eq!(detection_status["total"], 0);
}

#[tokio::test]
async fn fake_vm_mutation_routes_are_not_mounted() {
    let state = make_test_state();
    insert_fake_instance(&state, "ops-vm", std::process::id());
    let app = build_service_router(state);

    for (method, uri, body) in [
        (
            axum::http::Method::PATCH,
            "/vms/ops-vm/edit",
            Some(json!({ "ram_mb": 8192 })),
        ),
        (axum::http::Method::POST, "/vms/ops-vm/restart", None),
        (axum::http::Method::POST, "/vms/ops-vm/reload-profile", None),
    ] {
        let (status, _) = route_request(app.clone(), method, uri, body).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{uri} must stay unmounted until the VM mutation persists or performs a real operation"
        );
    }
}

fn process_fixture(table: &str) -> Vec<capsem_foundation::proctable::Process> {
    table
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            Some(capsem_foundation::proctable::Process::from_argv(
                pid,
                1,
                fields.map(Into::into).collect(),
            ))
        })
        .collect()
}
