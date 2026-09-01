use super::*;
use capsem_service::fs_utils::sanitize_file_path;

// Settings handler tests
// -----------------------------------------------------------------------

pub(super) struct SettingsEnvGuard {
    // Holds the path redirect for the guard's lifetime; restores on drop.
    _capsem_paths: capsem_foundation::paths::CapsemPathsGuard,
    previous_corp: Option<std::ffi::OsString>,
}

pub(super) struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
    #[allow(clippy::option_option, reason = "outer is captured-ness, inner is the value")]
    previous_test_profile_dir_override: Option<Option<PathBuf>>,
}

pub(super) struct TestBuiltinMcpBinaryGuard {
    path: PathBuf,
    remove_on_drop: bool,
}

pub(super) fn ensure_test_builtin_mcp_binary() -> TestBuiltinMcpBinaryGuard {
    let path = std::env::current_exe()
        .expect("test binary path")
        .parent()
        .expect("test binary parent")
        .join("capsem-mcp-builtin");
    let remove_on_drop = !path.exists();
    if remove_on_drop {
        std::fs::write(&path, "#!/bin/sh\n").expect("write test builtin MCP binary placeholder");
    }
    TestBuiltinMcpBinaryGuard { path, remove_on_drop }
}

impl EnvVarGuard {
    pub(super) fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        let previous_test_profile_dir_override = if key == "CAPSEM_PROFILES_DIR" {
            Some(super::set_test_profile_dir_override(Some(PathBuf::from(
                value.as_ref(),
            ))))
        } else {
            None
        };
        std::env::set_var(key, value);
        Self {
            key,
            previous,
            previous_test_profile_dir_override,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
        if let Some(previous) = self.previous_test_profile_dir_override.take() {
            super::set_test_profile_dir_override(previous);
        }
    }
}

impl Drop for TestBuiltinMcpBinaryGuard {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl Drop for SettingsEnvGuard {
    fn drop(&mut self) {
        if let Some(previous_corp) = self.previous_corp.take() {
            std::env::set_var("CAPSEM_CORP_CONFIG", previous_corp);
        } else {
            std::env::remove_var("CAPSEM_CORP_CONFIG");
        }
    }
}

pub(super) fn install_empty_settings_env(dir: &tempfile::TempDir) -> (SettingsEnvGuard, PathBuf, PathBuf) {
    let settings_path = dir.path().join("settings.toml");
    let corp_path = dir.path().join("corp.toml");
    capsem_core::net::policy_config::write_settings_file(
        &settings_path,
        &capsem_core::net::policy_config::SettingsFile::default(),
    )
    .unwrap();
    capsem_core::net::policy_config::write_settings_file(
        &corp_path,
        &capsem_core::net::policy_config::SettingsFile::default(),
    )
    .unwrap();

    let guard = SettingsEnvGuard {
        _capsem_paths: capsem_foundation::paths::CapsemPathsGuard::redirect(dir.path()),
        previous_corp: std::env::var_os("CAPSEM_CORP_CONFIG"),
    };
    std::env::set_var("CAPSEM_CORP_CONFIG", &corp_path);
    (guard, settings_path, corp_path)
}

#[tokio::test]
async fn handle_get_settings_returns_tree() {
    let Json(val) = handle_get_settings().await;
    assert!(val.get("tree").is_some(), "response must have 'tree'");
    assert!(val.get("issues").is_some(), "response must have 'issues'");
    assert!(val.get("presets").is_none(), "settings must not expose presets");
    assert!(
        val.get("policy").is_none(),
        "retired policy compatibility payload must not be emitted"
    );
    assert!(
        val.get("providers").is_none(),
        "settings response must not expose provider status"
    );
    assert!(val["tree"].is_array());
    assert!(val["issues"].is_array());
}

#[tokio::test]
async fn handle_save_settings_rejects_unknown_key() {
    let mut changes = HashMap::new();
    changes.insert("nonexistent.setting.xyz".into(), serde_json::json!("value"));
    let result = handle_save_settings(Json(changes)).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn handle_save_settings_rejects_retired_policy_rule_keys_atomically() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;

    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, user_path, _) = install_empty_settings_env(&dir);

    let mut changes = HashMap::new();
    let retired_key = "policy".to_string() + ".http.block_openai_github";
    changes.insert(
        retired_key.clone(),
        serde_json::json!({
            "on": "http.request",
            "if": "http.host == 'github.com'",
            "decision": "block",
            "priority": 10
        }),
    );

    let err = handle_save_settings(Json(changes))
        .await
        .expect_err("retired policy rule key should be rejected by settings handler");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(
        err.1.contains(&format!("unknown setting: {retired_key}")),
        "error should point to the retired policy key, got: {}",
        err.1
    );
    let loaded = capsem_core::net::policy_config::load_settings_file(&user_path).unwrap();
    assert!(
        loaded.settings.is_empty(),
        "rejected retired policy update must not mutate user config"
    );
}

pub(super) fn make_test_state_with_tempdir_at(dir: tempfile::TempDir) -> (Arc<ServiceState>, tempfile::TempDir) {
    let run_dir = dir.path().join("run");
    let registry_path = run_dir.join("persistent_registry.json");
    let asset_status_path = asset_status_path_for_run_dir(&run_dir);
    let state = Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        session_db_handles: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: run_dir.join("assets"),
        run_dir: run_dir.clone(),
        job_counter: AtomicU64::new(1),
        manifest: RwLock::new(None),
        current_version: "0.0.0".into(),
        asset_reconcile: Mutex::new(AssetReconcileState::default()),
        asset_reconcile_inflight: AtomicBool::new(false),
        asset_status_path,
        magika: test_magika(),
        plugin_policy_by_profile: Mutex::new(HashMap::new()),
        profile_summary_cache: Mutex::new(test_profile_summary_cache()),
        profile_cache: Mutex::new(test_profile_cache()),
        profile_status_cache: Mutex::new(None),
        profile_rule_cache: test_profile_rule_cache(),
        profile_mcp_default_cache: test_profile_mcp_default_cache(),
        profile_plugin_policy_cache: test_profile_plugin_policy_cache(),
        mcp_tool_cache: Mutex::new(capsem_core::mcp::load_tool_cache()),
        profile_mutation_db: test_profile_mutation_db(&run_dir),
        last_defunct_reconcile_ms: AtomicU64::new(0),
        stats_response_cache: Mutex::new(None),
        stats_detail_response_cache: Mutex::new(HashMap::new()),
        storage_diagnostics_cache: Mutex::new(HashMap::new()),
        persistent_resume_state_cache: Mutex::new(HashMap::new()),
        evaluate_rule_cache: Mutex::new(HashMap::new()),
        profile_rule_response_cache: Mutex::new(HashMap::new()),
        profile_plugin_response_cache: Mutex::new(HashMap::new()),
        evaluate_response_cache: Mutex::new(HashMap::new()),
        list_response_cache: Mutex::new(None),
        evaluate_last_response_cache: Mutex::new(None),
        save_restore_lock: tokio::sync::RwLock::new(()),
        shutdown_lock: tokio::sync::Mutex::new(()),
        update_lock: tokio::sync::Mutex::new(()),
        update_restart: tokio::sync::Notify::new(),
        _test_tempdir: None,
    });
    (state, dir)
}

// -----------------------------------------------------------------------
// resolve_workspace_path
// -----------------------------------------------------------------------

#[test]
fn resolve_rejects_unknown_vm() {
    let state = make_test_state();
    let r = resolve_workspace_path(&state, "nonexistent", "src/main.rs");
    assert!(r.is_err());
}

#[test]
fn resolve_rejects_symlink_escape() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("session");
    let workspace = session_dir.join("guest/workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    // Create a symlink that points outside workspace
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "secret").unwrap();
    std::os::unix::fs::symlink(&outside, workspace.join("escape")).unwrap();

    let (state, _dir2) = make_test_state_with_tempdir();
    state.instances.lock().unwrap().insert(
        "test-vm".into(),
        InstanceInfo {
            id: "test-vm".into(),
            name: "test-vm".into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: test_asset_pins(),
            pid: 1,
            uds_path: PathBuf::from("/tmp/test.sock"),
            session_dir,
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
        },
    );

    let r = resolve_workspace_path(&state, "test-vm", "escape/secret.txt");
    assert!(r.is_err());
}

#[test]
fn resolve_valid_path_inside_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("session");
    let workspace = session_dir.join("guest/workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("hello.txt"), "world").unwrap();

    let (state, _dir2) = make_test_state_with_tempdir();
    state.instances.lock().unwrap().insert(
        "test-vm".into(),
        InstanceInfo {
            id: "test-vm".into(),
            name: "test-vm".into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: test_asset_pins(),
            pid: 1,
            uds_path: PathBuf::from("/tmp/test.sock"),
            session_dir,
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
        },
    );

    let r = resolve_workspace_path(&state, "test-vm", "hello.txt");
    assert!(r.is_ok());
    let (ws_root, resolved) = r.unwrap();
    assert!(resolved.starts_with(ws_root.canonicalize().unwrap()));
}

// -----------------------------------------------------------------------
// list_dir_recursive
// -----------------------------------------------------------------------

#[test]
fn list_dir_returns_correct_structure() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::create_dir_all(ws.join("src")).unwrap();
    std::fs::write(ws.join("src/main.rs"), "fn main() {}").unwrap();
    std::fs::write(ws.join("README.md"), "# Hello").unwrap();

    let magika = test_magika();
    let entries = list_dir_recursive(ws, "", 1, 2, &magika);

    // Should have src/ dir and README.md file
    assert!(entries.len() >= 2);
    let dir_entry = entries.iter().find(|e| e.name == "src").unwrap();
    assert_eq!(dir_entry.entry_type, "directory");
    assert!(dir_entry.children.is_some());
    let children = dir_entry.children.as_ref().unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].name, "main.rs");
    assert_eq!(children[0].entry_type, "file");

    let file_entry = entries.iter().find(|e| e.name == "README.md").unwrap();
    assert_eq!(file_entry.entry_type, "file");
    assert!(file_entry.size > 0);
}

#[test]
fn list_dir_respects_depth_limit() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::create_dir_all(ws.join("a/b/c")).unwrap();
    std::fs::write(ws.join("a/b/c/deep.txt"), "deep").unwrap();

    let magika = test_magika();
    // depth 1: should list "a" but not recurse into "a/b"
    let entries = list_dir_recursive(ws, "", 1, 1, &magika);
    let a = entries.iter().find(|e| e.name == "a").unwrap();
    assert!(a.children.is_none());
}

#[test]
fn list_dir_skips_system_but_shows_hidden() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::create_dir_all(ws.join(".hidden")).unwrap();
    std::fs::create_dir_all(ws.join("system")).unwrap();
    std::fs::write(ws.join("visible.txt"), "yes").unwrap();

    let magika = test_magika();
    let entries = list_dir_recursive(ws, "", 1, 1, &magika);
    // .hidden + visible.txt shown; system/ filtered out
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|e| e.name == ".hidden"));
    assert!(entries.iter().any(|e| e.name == "visible.txt"));
    assert!(!entries.iter().any(|e| e.name == "system"));
}

#[test]
fn list_dir_sorts_dirs_first_then_alphabetical() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::write(ws.join("zebra.txt"), "z").unwrap();
    std::fs::create_dir_all(ws.join("alpha")).unwrap();
    std::fs::write(ws.join("apple.txt"), "a").unwrap();
    std::fs::create_dir_all(ws.join("beta")).unwrap();

    let magika = test_magika();
    let entries = list_dir_recursive(ws, "", 1, 1, &magika);
    // Dirs first (alpha, beta), then files (apple.txt, zebra.txt)
    assert_eq!(entries[0].name, "alpha");
    assert_eq!(entries[1].name, "beta");
    assert_eq!(entries[2].name, "apple.txt");
    assert_eq!(entries[3].name, "zebra.txt");
}

// -----------------------------------------------------------------------
// Download / Upload via resolve_workspace_path
// -----------------------------------------------------------------------

fn setup_vm_with_workspace(state: &ServiceState, dir: &std::path::Path, vm_id: &str) {
    setup_vm_with_workspace_and_uds(state, dir, vm_id, PathBuf::from("/tmp/test.sock"));
}

fn setup_vm_with_workspace_and_uds(state: &ServiceState, dir: &std::path::Path, vm_id: &str, uds_path: PathBuf) {
    let session_dir = dir.join("session");
    let workspace = session_dir.join("guest/workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    state.instances.lock().unwrap().insert(
        vm_id.into(),
        InstanceInfo {
            id: vm_id.into(),
            name: vm_id.into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: test_asset_pins(),
            pid: 1,
            uds_path,
            session_dir,
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
        },
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WriteFileIpcReply {
    Success,
    Disconnect,
}

async fn spawn_file_boundary_ipc(
    expected_messages: usize,
    write_reply: WriteFileIpcReply,
) -> (
    tempfile::TempDir,
    PathBuf,
    tokio::task::JoinHandle<Vec<ServiceToProcess>>,
) {
    let dir = tempfile::tempdir().unwrap();
    let uds_path = dir.path().join("process.sock");
    let listener = tokio::net::UnixListener::bind(&uds_path).unwrap();
    std::fs::write(uds_path.with_extension("ready"), b"ready").unwrap();
    let handle = tokio::spawn(async move {
        let mut messages = Vec::new();
        for _ in 0..expected_messages {
            let (stream, _) = listener.accept().await.unwrap();
            let std_stream = stream.into_std().unwrap();
            let std_stream = tokio::task::spawn_blocking(move || {
                let mut std_stream = std_stream;
                capsem_foundation::ipc_handshake::negotiate_responder(&mut std_stream, "capsem-process-test", "")?;
                Ok::<_, capsem_proto::handshake::HandshakeError>(std_stream)
            })
            .await
            .unwrap()
            .unwrap();
            let (tx, rx): (
                tokio_unix_ipc::Sender<ProcessToService>,
                tokio_unix_ipc::Receiver<ServiceToProcess>,
            ) = tokio_unix_ipc::channel_from_std(std_stream).unwrap();
            let msg = rx.recv().await.unwrap();
            match &msg {
                ServiceToProcess::LogFileBoundary { id, .. } => {
                    tx.send(ProcessToService::LogFileBoundaryResult {
                        id: *id,
                        success: true,
                        data: None,
                        error: None,
                    })
                    .await
                    .unwrap();
                }
                ServiceToProcess::WriteFile { id, .. } => {
                    if write_reply == WriteFileIpcReply::Disconnect {
                        drop(tx);
                    } else {
                        tx.send(ProcessToService::WriteFileResult {
                            id: *id,
                            success: true,
                            error: None,
                        })
                        .await
                        .unwrap();
                    }
                }
                ServiceToProcess::ReadFile { id, .. } => {
                    tx.send(ProcessToService::ReadFileResult {
                        id: *id,
                        data: Some(b"guest export".to_vec()),
                        error: None,
                    })
                    .await
                    .unwrap();
                }
                other => panic!("unexpected IPC message in file boundary test: {other:?}"),
            }
            messages.push(msg);
        }
        messages
    });
    (dir, uds_path, handle)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_logs_file_import_before_writing_workspace_file() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(1, WriteFileIpcReply::Success).await;
    setup_vm_with_workspace_and_uds(&state, dir.path(), "up-ledger-vm", uds_path);

    let result = handle_upload_file(
        State(state),
        Path("up-ledger-vm".to_string()),
        Query(FileContentQuery {
            path: "new.txt".to_string(),
        }),
        axum::body::Bytes::from_static(b"uploaded through ledger"),
    )
    .await
    .expect("upload should succeed after boundary log");

    assert_eq!(result.size, b"uploaded through ledger".len() as u64);
    let messages = ipc.await.unwrap();
    assert_eq!(messages.len(), 1);
    match &messages[0] {
        ServiceToProcess::LogFileBoundary {
            action,
            path,
            data,
            size,
            ..
        } => {
            assert_eq!(*action, FileBoundaryAction::Import);
            assert_eq!(path, "new.txt");
            assert_eq!(data, b"uploaded through ledger");
            assert_eq!(*size, b"uploaded through ledger".len() as u64);
        }
        other => panic!("upload must log file import before write, got {other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("session/guest/workspace/new.txt")).unwrap(),
        "uploaded through ledger"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_logs_file_export_before_returning_response() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(1, WriteFileIpcReply::Success).await;
    setup_vm_with_workspace_and_uds(&state, dir.path(), "dl-ledger-vm", uds_path);
    let workspace_file = dir.path().join("session/guest/workspace/report.txt");
    std::fs::write(&workspace_file, b"export through ledger").unwrap();

    let response = handle_download_file(
        State(state),
        Path("dl-ledger-vm".to_string()),
        Query(FileContentQuery {
            path: "report.txt".to_string(),
        }),
    )
    .await
    .expect("download should succeed after boundary log");

    assert_eq!(response.status(), StatusCode::OK);
    let messages = ipc.await.unwrap();
    assert_eq!(messages.len(), 1);
    match &messages[0] {
        ServiceToProcess::LogFileBoundary {
            action,
            path,
            data,
            size,
            ..
        } => {
            assert_eq!(*action, FileBoundaryAction::Export);
            assert_eq!(path, "report.txt");
            assert_eq!(data, b"export through ledger");
            assert_eq!(*size, b"export through ledger".len() as u64);
        }
        other => panic!("download must log file export before response, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_file_content_does_not_wait_on_stats_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(1, WriteFileIpcReply::Success).await;
    setup_vm_with_workspace_and_uds(&state, dir.path(), "fast-file-vm", uds_path);
    std::fs::write(
        dir.path().join("session/guest/workspace/latency.txt"),
        b"file content must not wait on ledger rebuild",
    )
    .unwrap();

    // Liveness, not performance: this guards the route blocking on the logger
    // DB, a wait measured in seconds or never. At 250ms it measured the runner
    // instead and failed a release.
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        handle_download_file(
            State(state),
            Path("fast-file-vm".to_string()),
            Query(FileContentQuery {
                path: "latency.txt".to_string(),
            }),
        ),
    )
    .await
    .expect("file content route waited for stats rebuild")
    .expect("download should succeed after boundary log");

    assert_eq!(response.status(), StatusCode::OK);
    let messages = ipc.await.unwrap();
    assert_eq!(messages.len(), 1);
    assert!(matches!(
        &messages[0],
        ServiceToProcess::LogFileBoundary {
            action: FileBoundaryAction::Export,
            path,
            ..
        } if path == "latency.txt"
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mounted_file_import_export_routes_log_boundary_events() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(2, WriteFileIpcReply::Success).await;
    setup_vm_with_workspace_and_uds(&state, dir.path(), "file-route-vm", uds_path);
    let app = build_service_router(state);

    let upload_response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::POST)
                .uri("/vms/file-route-vm/files/content?path=new.txt")
                .body(Body::from("uploaded over mounted route"))
                .unwrap(),
        )
        .await
        .expect("upload route should respond");
    assert_eq!(upload_response.status(), StatusCode::OK);
    let upload_body = to_bytes(upload_response.into_body(), usize::MAX).await.unwrap();
    let upload_json: serde_json::Value = serde_json::from_slice(&upload_body).unwrap();
    assert_eq!(upload_json["success"], true);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("session/guest/workspace/new.txt")).unwrap(),
        "uploaded over mounted route"
    );

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::GET)
                .uri("/vms/file-route-vm/files/content?path=new.txt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("download route should respond");
    assert_eq!(response.status(), StatusCode::OK);
    let downloaded = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&downloaded[..], b"uploaded over mounted route");

    let messages = ipc.await.unwrap();
    assert_eq!(messages.len(), 2);
    match &messages[0] {
        ServiceToProcess::LogFileBoundary {
            action,
            path,
            data,
            size,
            ..
        } => {
            assert_eq!(*action, FileBoundaryAction::Import);
            assert_eq!(path, "new.txt");
            assert_eq!(data, b"uploaded over mounted route");
            assert_eq!(*size, b"uploaded over mounted route".len() as u64);
        }
        other => panic!("upload route must log import first, got {other:?}"),
    }
    match &messages[1] {
        ServiceToProcess::LogFileBoundary {
            action,
            path,
            data,
            size,
            ..
        } => {
            assert_eq!(*action, FileBoundaryAction::Export);
            assert_eq!(path, "new.txt");
            assert_eq!(data, b"uploaded over mounted route");
            assert_eq!(*size, b"uploaded over mounted route".len() as u64);
        }
        other => panic!("download route must log export first, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_does_not_write_workspace_file_when_import_ledger_fails() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let ipc_dir = tempfile::tempdir().unwrap();
    let uds_path = ipc_dir.path().join("process.sock");
    let listener = tokio::net::UnixListener::bind(&uds_path).unwrap();
    std::fs::write(uds_path.with_extension("ready"), b"ready").unwrap();
    let ipc = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let std_stream = stream.into_std().unwrap();
        let std_stream = tokio::task::spawn_blocking(move || {
            let mut std_stream = std_stream;
            capsem_foundation::ipc_handshake::negotiate_responder(&mut std_stream, "capsem-process-test", "")?;
            Ok::<_, capsem_proto::handshake::HandshakeError>(std_stream)
        })
        .await
        .unwrap()
        .unwrap();
        let (tx, rx): (
            tokio_unix_ipc::Sender<ProcessToService>,
            tokio_unix_ipc::Receiver<ServiceToProcess>,
        ) = tokio_unix_ipc::channel_from_std(std_stream).unwrap();
        let msg = rx.recv().await.unwrap();
        match &msg {
            ServiceToProcess::LogFileBoundary { id, .. } => {
                tx.send(ProcessToService::LogFileBoundaryResult {
                    id: *id,
                    success: false,
                    data: None,
                    error: Some("security ledger rejected import".to_string()),
                })
                .await
                .unwrap();
            }
            other => panic!("unexpected IPC message in import denial test: {other:?}"),
        }
        msg
    });
    setup_vm_with_workspace_and_uds(&state, dir.path(), "deny-ledger-vm", uds_path);

    let err = handle_upload_file(
        State(state),
        Path("deny-ledger-vm".to_string()),
        Query(FileContentQuery {
            path: "blocked.txt".to_string(),
        }),
        axum::body::Bytes::from_static(b"must not land"),
    )
    .await
    .expect_err("failed import ledger write must fail closed");

    assert_eq!(err.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(err.1.contains("security ledger rejected import"));
    let msg = ipc.await.unwrap();
    assert!(matches!(msg, ServiceToProcess::LogFileBoundary { .. }));
    assert!(
        !dir.path().join("session/guest/workspace/blocked.txt").exists(),
        "upload must not write bytes when import ledger fails"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_file_logs_import_before_guest_write() {
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(2, WriteFileIpcReply::Success).await;
    state.instances.lock().unwrap().insert(
        "write-ledger-vm".into(),
        InstanceInfo {
            id: "write-ledger-vm".into(),
            name: "write-ledger-vm".into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: test_asset_pins(),
            pid: 1,
            uds_path,
            session_dir: state.run_dir.join("sessions/write-ledger-vm"),
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
        },
    );

    let _ = handle_write_file(
        State(state),
        Path("write-ledger-vm".to_string()),
        Json(WriteFileRequest {
            path: "/workspace/from-api.txt".to_string(),
            content: "guest write".to_string(),
        }),
    )
    .await
    .expect("write_file should succeed after import ledger");

    let messages = ipc.await.unwrap();
    assert_eq!(messages.len(), 2);
    match &messages[0] {
        ServiceToProcess::LogFileBoundary {
            action,
            path,
            data,
            size,
            ..
        } => {
            assert_eq!(*action, FileBoundaryAction::Import);
            assert_eq!(path, "/workspace/from-api.txt");
            assert_eq!(data, b"guest write");
            assert_eq!(*size, b"guest write".len() as u64);
        }
        other => panic!("write_file first IPC must be import ledger, got {other:?}"),
    }
    assert!(matches!(
        messages[1],
        ServiceToProcess::WriteFile { ref path, .. } if path == "/workspace/from-api.txt"
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_file_ipc_failure_names_vm_and_completion_stage() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(2, WriteFileIpcReply::Disconnect).await;
    setup_vm_with_workspace_and_uds(&state, dir.path(), "diagnostic-vm", uds_path);

    let error = handle_write_file(
        State(state),
        Path("diagnostic-vm".to_string()),
        Json(WriteFileRequest {
            path: "/workspace/failure.txt".to_string(),
            content: "diagnose me".to_string(),
        }),
    )
    .await
    .expect_err("a disconnected process must fail the write");

    let messages = ipc.await.unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(error.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(error.1.contains("VM diagnostic-vm write_file"), "{}", error.1);
    assert!(
        error.1.contains("awaiting the guest completion response"),
        "{}",
        error.1
    );
}

#[test]
fn download_reads_correct_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "dl-vm");

    let ws = dir.path().join("session/guest/workspace");
    let content = b"hello world\nline 2\n";
    std::fs::write(ws.join("test.txt"), content).unwrap();

    let (_, resolved) = resolve_workspace_path(&state, "dl-vm", "test.txt").unwrap();
    let data = std::fs::read(&resolved).unwrap();
    assert_eq!(data, content);
}

#[test]
fn download_binary_preserves_content() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "bin-vm");

    let ws = dir.path().join("session/guest/workspace");
    let binary: Vec<u8> = (0..256).map(|i| i as u8).collect();
    std::fs::write(ws.join("data.bin"), &binary).unwrap();

    let (_, resolved) = resolve_workspace_path(&state, "bin-vm", "data.bin").unwrap();
    let data = std::fs::read(&resolved).unwrap();
    assert_eq!(data, binary);
}

#[test]
fn upload_creates_file_with_content() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "up-vm");

    let ws = dir.path().join("session/guest/workspace");
    let (_, target) = resolve_workspace_path(&state, "up-vm", "new.txt").unwrap();
    std::fs::write(&target, b"uploaded").unwrap();

    assert_eq!(std::fs::read_to_string(ws.join("new.txt")).unwrap(), "uploaded");
}

#[test]
fn upload_creates_parent_directories() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "mkdir-vm");

    let ws = dir.path().join("session/guest/workspace");
    // resolve_workspace_path should succeed even for non-existing nested paths
    let (_, target) = resolve_workspace_path(&state, "mkdir-vm", "deep/nested/file.txt").unwrap();
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"deep content").unwrap();

    assert_eq!(
        std::fs::read_to_string(ws.join("deep/nested/file.txt")).unwrap(),
        "deep content"
    );
}

#[test]
fn upload_path_traversal_blocked() {
    let r = sanitize_file_path("../../etc/passwd");
    assert!(r.is_err());
}

#[test]
fn download_nonexistent_file_resolve_ok_but_not_exists() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "404-vm");

    // Resolving a non-existent file path still works (for upload target)
    let result = resolve_workspace_path(&state, "404-vm", "nonexistent.txt");
    assert!(result.is_ok());
    let (_, resolved) = result.unwrap();
    assert!(!resolved.exists());
}

// is_launchd_cleanup_transient identifies the misleading "missing
// entitlement" NSError that VZ emits when launchd's PETRIFIED-cleanup
// queue is saturated under rapid VM churn. The error string is
// stable across VZ releases (Apple's localizedDescription); pattern-
// match conservatively so a real codesign regression doesn't get
// silently retried.
#[test]
fn launchd_transient_matches_actual_vz_entitlement_error() {
    let tail = "Error: failed to boot VM\n\nCaused by:\n    \
        VM config validation failed: NSError { code: 2, \
        localizedDescription: \"Invalid virtual machine configuration. \
        The process doesn't have the \u{201c}com.apple.security.\
        virtualization\u{201d} entitlement.\", domain: \"VZErrorDomain\", \
        userInfo: {} }";
    assert!(is_launchd_cleanup_transient(tail));
}

#[test]
fn launchd_transient_matches_straight_quote_variant() {
    // Same content with ASCII quotes around the entitlement key.
    let tail = "VM config validation failed: NSError { code: 2, \
        localizedDescription: \"...The process doesn't have the \
        \\\"com.apple.security.virtualization\\\" entitlement.\" }";
    assert!(is_launchd_cleanup_transient(tail));
}

#[test]
fn launchd_transient_rejects_other_failures() {
    let unrelated = "Error: failed to build VmConfig\n\nCaused by:\n    \
        hash mismatch for ...img: expected abc, got def";
    assert!(!is_launchd_cleanup_transient(unrelated));

    let no_log = "(no preserved log found)";
    assert!(!is_launchd_cleanup_transient(no_log));

    let empty = "";
    assert!(!is_launchd_cleanup_transient(empty));
}

#[test]
fn launchd_transient_rejects_partial_match() {
    // The word "entitlement" alone in some unrelated error must not match;
    // the matcher requires the full VZ-specific phrase.
    let mention_only = "warn: this command may need an entitlement";
    assert!(!is_launchd_cleanup_transient(mention_only));
}

// classify_attempt_decision is the pure routing function the
// poll_until-based retry loop in handle_provision delegates to.
// Testing it directly lets us prove the retry path engages on the
// LaunchdTransient outcome (the actual fix for Bug A) without
// spawning a real VM. If a future refactor breaks the routing
// (e.g., maps LaunchdTransient to BailWithError), these fail.

#[test]
fn classify_ready_outcome_succeeds() {
    let uds = PathBuf::from("/tmp/x.sock");
    match classify_attempt_decision(ProvisionAttemptOutcome::Ready { uds_path: uds.clone() }, "vm-1") {
        AttemptDecision::Succeed(p) => assert_eq!(p, uds),
        other => panic!("expected Succeed, got {other:?}"),
    }
}

#[test]
fn classify_still_booting_timeout_succeeds_with_uds() {
    let uds = PathBuf::from("/tmp/y.sock");
    match classify_attempt_decision(
        ProvisionAttemptOutcome::StillBootingTimedOut { uds_path: uds.clone() },
        "vm-2",
    ) {
        AttemptDecision::Succeed(p) => assert_eq!(p, uds),
        other => panic!("expected Succeed for still-booting envelope, got {other:?}"),
    }
}

#[test]
fn classify_launchd_transient_routes_to_retry() {
    // The core of the Bug A fix: LaunchdTransient must trigger a retry,
    // not bail with the misleading entitlement error.
    match classify_attempt_decision(ProvisionAttemptOutcome::LaunchdTransient, "vm-3") {
        AttemptDecision::RetryAfterCleanup => {}
        other => panic!("expected RetryAfterCleanup for LaunchdTransient, got {other:?}"),
    }
}

#[test]
fn classify_boot_crash_bails_with_500_and_tail() {
    let tail = "Error: failed to boot VM\n\nCaused by:\n    bogus".to_string();
    match classify_attempt_decision(ProvisionAttemptOutcome::BootCrash { tail: tail.clone() }, "vm-4") {
        AttemptDecision::BailWithError(AppError(status, msg)) => {
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert!(msg.contains("vm-4"), "msg should embed the id: {msg}");
            assert!(msg.contains(&tail), "msg should embed the log tail: {msg}");
            assert!(msg.contains("capsem logs vm-4"), "msg should hint at follow-up cmd");
        }
        other => panic!("expected BailWithError(500), got {other:?}"),
    }
}

#[test]
fn classify_provision_error_already_exists_returns_409() {
    let err = anyhow::anyhow!("persistent VM \"vm-5\" already exists. Use `capsem resume vm-5`.");
    match classify_attempt_decision(ProvisionAttemptOutcome::ProvisionError(err), "vm-5") {
        AttemptDecision::BailWithError(AppError(status, _)) => {
            assert_eq!(
                status,
                StatusCode::CONFLICT,
                "duplicate-name errors must return 409 so clients can distinguish from server failures"
            );
        }
        other => panic!("expected BailWithError(409) for already-exists, got {other:?}"),
    }
}

#[test]
fn classify_provision_error_other_returns_500() {
    let err = anyhow::anyhow!("rootfs not found at /missing/path");
    match classify_attempt_decision(ProvisionAttemptOutcome::ProvisionError(err), "vm-6") {
        AttemptDecision::BailWithError(AppError(status, msg)) => {
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert!(msg.contains("rootfs not found"), "underlying error preserved: {msg}");
        }
        other => panic!("expected BailWithError(500), got {other:?}"),
    }
}

// wait_for_vm_ready polls a cheap local sentinel file. Lock the production
// backoff directly instead of asserting host wall-clock timing, which measures
// test-runner starvation under parallel LLVM coverage rather than poll delay.
#[tokio::test]
async fn wait_for_vm_ready_uses_tight_poll_contract_and_detects_ready() {
    let dir = tempfile::tempdir().unwrap();
    let uds_path = dir.path().join("vm.sock");
    let ready_path = uds_path.with_extension("ready");

    let opts = vm_ready_poll_opts(30);
    assert_eq!(opts.initial_delay, std::time::Duration::from_millis(5));
    assert_eq!(opts.max_delay, std::time::Duration::from_millis(50));
    assert_eq!(opts.timeout, std::time::Duration::from_secs(30));

    // Simulate a VM that becomes ready ~200ms after provision. Real VM
    // boots land in the 400-700ms range, so 200ms is a conservative stand-in.
    let ready_clone = ready_path.clone();
    let creator = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        std::fs::write(&ready_clone, b"").unwrap();
    });

    wait_for_vm_ready(&uds_path, 30, None, None)
        .await
        .expect("ready should be detected");
    creator.join().unwrap();
}

#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn non_macos_lifecycle_does_not_take_the_apple_vz_host_lock() {
    let guard = super::acquire_vz_host_lock(super::startup::VzHostLockMode::Exclusive)
        .await
        .expect("non-macOS lifecycle lock acquisition should succeed");

    assert!(
        guard.is_none(),
        "KVM lifecycle operations are independent and must not contend on the Apple VZ host lock"
    );
}

#[test]
fn apple_vz_host_lock_is_required_only_on_macos() {
    assert_eq!(
        super::requires_vz_host_lock(),
        cfg!(target_os = "macos"),
        "the host-wide save/restore lock protects Apple VZ, not independent KVM VMs"
    );
}

// ── Spawn environment leak boundary ────────────────────────────────
//
// Both provision and resume call `child_cmd.env_clear()` and then re-add only
// PROCESS_ENV_ALLOWLIST. That allowlist is the entire boundary between the
// service's own environment -- which on a developer or CI machine routinely
// holds ANTHROPIC_API_KEY, AWS_SECRET_ACCESS_KEY, GITHUB_TOKEN -- and the
// per-VM process that talks to the guest. Nothing else enforces it, so these
// tests fail the build the moment a secret-shaped name is added.

/// Substrings that mark a variable as likely secret-bearing.
const SECRET_MARKERS: &[&str] = &[
    "KEY",
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "CREDENTIAL",
    "AUTH",
    "SESSION",
    "COOKIE",
];

/// The one allowlisted name that trips the marker scan without carrying a
/// secret: it is a filesystem path to the broker's store, deliberately
/// redirected by the hermetic integration and Ironbank rails.
const SECRET_MARKER_EXCEPTIONS: &[&str] = &["CAPSEM_CREDENTIAL_STORE_PATH"];

#[test]
fn spawn_env_allowlist_carries_no_secret_bearing_names() {
    let offenders: Vec<&str> = PROCESS_ENV_ALLOWLIST
        .iter()
        .copied()
        .filter(|key| !SECRET_MARKER_EXCEPTIONS.contains(key))
        .filter(|key| {
            let upper = key.to_ascii_uppercase();
            SECRET_MARKERS.iter().any(|marker| upper.contains(marker))
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "these keys would forward host secrets into the per-VM process: {offenders:?}. \
         If one is genuinely not a secret, add it to SECRET_MARKER_EXCEPTIONS with a reason."
    );
}

#[test]
fn spawn_env_allowlist_forwards_only_capsem_vars_and_a_minimal_os_set() {
    // Anything outside this set is third-party environment the guest-facing
    // process has no reason to inherit.
    const OS_BASELINE: &[&str] = &["HOME", "PATH", "USER", "TMPDIR"];

    let unexpected: Vec<&str> = PROCESS_ENV_ALLOWLIST
        .iter()
        .copied()
        .filter(|key| !key.starts_with("CAPSEM_") && !OS_BASELINE.contains(key))
        .collect();

    assert!(
        unexpected.is_empty(),
        "only CAPSEM_-prefixed vars and the minimal OS baseline may cross into \
         the per-VM process: {unexpected:?}"
    );
}

#[test]
fn spawn_env_allowlist_is_deduplicated() {
    let unique: std::collections::HashSet<&str> = PROCESS_ENV_ALLOWLIST.iter().copied().collect();

    assert_eq!(
        unique.len(),
        PROCESS_ENV_ALLOWLIST.len(),
        "duplicate entries hide review churn in the leak boundary"
    );
}

#[test]
fn spawn_env_allowlist_keeps_the_vars_the_child_actually_needs() {
    for required in ["HOME", "PATH", "CAPSEM_HOME"] {
        assert!(
            PROCESS_ENV_ALLOWLIST.contains(&required),
            "{required} is required for the per-VM process to start"
        );
    }
}

// ---------------------------------------------------------------------------
// Service pidfile ownership
// ---------------------------------------------------------------------------
//
// The pidfile is the only handle a harness has on a detached service: the
// asset gate, `_ensure-service`, and every abort path reap by
// `$run_dir/service.pid`. A guard that removes that file when it no longer
// records us erases the pid of whichever service is now serving, and every
// later `stop_gate_pidfile` reads as a silent success while the real service
// runs on under launchd.

#[test]
fn service_pidfile_removes_its_own_record_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("service.pid");

    let guard = ServicePidfile::claim(path.clone());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap().trim(),
        std::process::id().to_string(),
        "claim must record our own pid for the reaper to find"
    );

    drop(guard);
    assert!(
        !path.exists(),
        "a dead service must not leave a stale pid for the reaper to kill"
    );
}

#[test]
fn service_pidfile_leaves_a_successors_record_intact() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("service.pid");

    let guard = ServicePidfile::claim(path.clone());
    // A successor service claims the same run directory while we shut down.
    std::fs::write(&path, "424242").unwrap();
    drop(guard);

    assert_eq!(
        std::fs::read_to_string(&path).unwrap().trim(),
        "424242",
        "erasing a successor's pid strands it: every later reap finds no \
         pidfile and reports success while the service keeps running"
    );
}
