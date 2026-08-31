use super::*;

#[test]
fn statvfs_bytes_accepts_platform_block_widths() {
    assert_eq!(statvfs_bytes(7_u32, 4096), 28_672);
    assert_eq!(statvfs_bytes(7_u64, 4096), 28_672);
}

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

    assert_eq!(error.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(error.1.contains("capsem.manifest_metadata.v1"));
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
        "profiles": {"code": {
            "name": "Code",
            "description": "Optimized for coding.",
            "revision": "2026.0703.2",
            "status": "current",
            "architectures": [{
                "architecture": arch,
                "image_revision": "2026.0714.18",
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
        }}
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
    let code = body["profiles"]["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|profile| profile["id"] == "code")
        .unwrap();
    assert_eq!(code["description"], "Optimized for coding and long-running agents.");
    assert_eq!(body["updates"]["checked_at"], 1100);
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
fn snapshot_status_from_session_dir_reads_snapshot_metadata_without_db() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path();
    std::fs::create_dir_all(session.join("workspace")).unwrap();
    std::fs::create_dir_all(session.join("system")).unwrap();
    std::fs::create_dir_all(session.join("auto_snapshots")).unwrap();
    std::fs::write(session.join("workspace/hello.txt"), "hello").unwrap();

    let mut scheduler = capsem_core::auto_snapshot::AutoSnapshotScheduler::new(
        session.to_path_buf(),
        10,
        12,
        std::time::Duration::from_secs(300),
    );
    scheduler.take_snapshot().unwrap();
    scheduler.take_named_snapshot("manual_check").unwrap();

    let status = snapshot_status_from_session_dir(session);
    assert_eq!(status.total, 2);
    assert_eq!(status.auto_count, 1);
    assert_eq!(status.manual_count, 1);
    assert_eq!(status.manual_available, 11);
    assert!(status
        .snapshots
        .iter()
        .any(|snapshot| snapshot.origin == "manual" && snapshot.name.as_deref() == Some("manual_check")));

    let db_path = session.join("session.db");
    assert!(!db_path.exists(), "snapshot route backing must not require session.db");
}

#[test]
fn find_orphan_capsem_pids_matches_capsem_process_under_run_dir() {
    let run_dir = PathBuf::from("/var/folders/XY/T/capsem-test-abc");
    let ps = "\
  1502 /path/to/target/debug/capsem-process --env CAPSEM_VM_ID=orphan --id orphan --session-dir /var/folders/XY/T/capsem-test-abc/sessions/orphan --uds-path /tmp/capsem/abc.sock
  1742 /path/to/target/debug/capsem-process --id victim --session-dir /var/folders/XY/T/capsem-test-abc/persistent/victim --uds-path /tmp/capsem/def.sock
";
    let pids = find_orphan_capsem_pids(ps, &run_dir);
    assert_eq!(pids, vec![1502, 1742]);
}

#[test]
fn find_orphan_capsem_pids_skips_processes_for_other_run_dirs() {
    let run_dir = PathBuf::from("/var/folders/XY/T/capsem-test-mine");
    let ps = "\
  1502 /path/to/target/debug/capsem-process --session-dir /var/folders/XY/T/capsem-test-other/sessions/foo
  1742 /path/to/target/debug/capsem-process --session-dir /var/folders/XY/T/capsem-test-mine/sessions/bar
";
    let pids = find_orphan_capsem_pids(ps, &run_dir);
    assert_eq!(pids, vec![1742], "must not match neighbouring test run dirs");
}

#[test]
fn find_orphan_capsem_pids_skips_non_capsem_process_binaries() {
    let run_dir = PathBuf::from("/var/folders/XY/T/capsem-test-abc");
    // A stray cargo invocation that happens to mention the run_dir path.
    let ps = "\
  99 /bin/cargo build --manifest-path /var/folders/XY/T/capsem-test-abc/Cargo.toml
  1502 /path/to/target/debug/capsem-process --session-dir /var/folders/XY/T/capsem-test-abc/sessions/orphan
";
    let pids = find_orphan_capsem_pids(ps, &run_dir);
    assert_eq!(pids, vec![1502], "match must require 'capsem-process' in the line");
}

#[test]
fn find_orphan_capsem_pids_returns_empty_on_no_match() {
    let run_dir = PathBuf::from("/var/folders/XY/T/capsem-test-empty");
    let ps = "\
  1 /sbin/launchd
  42 /usr/bin/bash
";
    let pids = find_orphan_capsem_pids(ps, &run_dir);
    assert!(pids.is_empty());
}
