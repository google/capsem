use super::*;

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
        persistent_registry: SharedRegistry::new(PersistentRegistry::load(registry_path).expect("registry loads")),
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
// resolve_workspace_target
// -----------------------------------------------------------------------

#[test]
fn resolve_rejects_unknown_vm() {
    let state = make_test_state();
    let r = resolve_workspace_target(&state, "nonexistent", "src/main.rs", false);
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

    let r = resolve_workspace_target(&state, "test-vm", "escape/secret.txt", false);
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

    let (parent, name) = resolve_workspace_target(&state, "test-vm", "hello.txt", false).unwrap();
    assert_eq!(parent.path(), workspace.canonicalize().unwrap());
    assert_eq!(name, "hello.txt");
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
    let entries = list_dir_recursive(
        &capsem_foundation::unix::contained::ContainedDir::open_root(ws).unwrap(),
        "",
        1,
        2,
        &magika,
    );

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
    let entries = list_dir_recursive(
        &capsem_foundation::unix::contained::ContainedDir::open_root(ws).unwrap(),
        "",
        1,
        1,
        &magika,
    );
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
    let entries = list_dir_recursive(
        &capsem_foundation::unix::contained::ContainedDir::open_root(ws).unwrap(),
        "",
        1,
        1,
        &magika,
    );
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
    let entries = list_dir_recursive(
        &capsem_foundation::unix::contained::ContainedDir::open_root(ws).unwrap(),
        "",
        1,
        1,
        &magika,
    );
    // Dirs first (alpha, beta), then files (apple.txt, zebra.txt)
    assert_eq!(entries[0].name, "alpha");
    assert_eq!(entries[1].name, "beta");
    assert_eq!(entries[2].name, "apple.txt");
    assert_eq!(entries[3].name, "zebra.txt");
}

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
