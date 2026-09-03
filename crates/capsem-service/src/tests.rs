use super::*;
use axum::body::{to_bytes, Body};
use capsem_core::net::policy_config::{ProfileObomConfig, ProfileObomDescriptor};
use std::sync::atomic::AtomicU64;
use tower::ServiceExt;

mod asset_wait;
mod instance_reaper;
mod profile_asset_status;

static SETTINGS_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_magika() -> Mutex<magika::Session> {
    Mutex::new(
        magika::Session::builder()
            .with_inter_threads(1)
            .with_intra_threads(1)
            .build()
            .expect("magika init"),
    )
}

fn test_profile_summary_cache() -> Vec<api::ProfileSummary> {
    build_profile_summary_cache().expect("test profile summary cache should build")
}

fn test_profile_cache() -> BTreeMap<String, Profile> {
    build_profile_cache().expect("test profile cache should build")
}

fn test_profile_rule_cache() -> Mutex<BTreeMap<String, Vec<api::EnforcementRuleInfo>>> {
    Mutex::new(build_profile_rule_cache(None).expect("test profile rule cache should build"))
}

fn test_profile_mcp_default_cache() -> Mutex<BTreeMap<String, Result<api::McpDefaultPermissionResponse, String>>> {
    Mutex::new(build_profile_mcp_default_cache(None).expect("test profile MCP default cache should build"))
}

fn test_profile_plugin_policy_cache() -> Mutex<BTreeMap<String, BTreeMap<String, SecurityPluginConfig>>> {
    Mutex::new(build_profile_plugin_policy_cache(None).expect("test profile plugin policy cache should build"))
}

fn test_profile_mutation_db(run_dir: &StdPath) -> Arc<capsem_logger::DbHandle> {
    ServiceState::open_profile_mutation_db_handle(run_dir).unwrap()
}

fn make_test_state() -> Arc<ServiceState> {
    let test_tempdir = tempfile::tempdir().unwrap();
    let run_dir = test_tempdir.path().join("run");
    std::fs::create_dir_all(&run_dir).unwrap();
    let registry_path = run_dir.join("persistent_registry.json");
    let asset_status_path = asset_status_path_for_run_dir(&run_dir);
    Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        session_db_handles: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path).expect("registry loads")),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: PathBuf::from("/nonexistent/assets"),
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
        _test_tempdir: Some(test_tempdir),
    })
}

pub(crate) async fn route_request(
    app: axum::Router,
    method: axum::http::Method,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    let request_body = if let Some(body) = body {
        builder = builder.header(axum::http::header::CONTENT_TYPE, "application/json");
        Body::from(serde_json::to_vec(&body).unwrap())
    } else {
        Body::empty()
    };
    let response = app
        .oneshot(builder.body(request_body).unwrap())
        .await
        .expect("route should respond");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(&bytes).to_string() }))
    };
    (status, json)
}

fn enforcement_evaluate_body(request: &EnforcementEvaluateRequest) -> Bytes {
    Bytes::from(serde_json::to_vec(request).unwrap())
}

pub(super) fn make_asset_state(assets_dir: PathBuf) -> Arc<ServiceState> {
    let run_dir = assets_dir.join("run");
    let asset_status_path = asset_status_path_for_run_dir(&run_dir);
    let manifest = capsem_assets::asset_manager::load_manifest_for_assets(&assets_dir).map(Arc::new);
    Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        session_db_handles: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(
            PersistentRegistry::load(assets_dir.join("persistent_registry.json")).expect("registry loads"),
        ),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir,
        run_dir: run_dir.clone(),
        job_counter: AtomicU64::new(1),
        manifest: RwLock::new(manifest),
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
    })
}

fn insert_fake_instance(state: &ServiceState, id: &str, pid: u32) {
    insert_fake_instance_with_session_dir(state, id, pid, PathBuf::from(format!("/tmp/sessions/{}", id)));
}

pub(crate) fn insert_fake_instance_with_session_dir(state: &ServiceState, id: &str, pid: u32, session_dir: PathBuf) {
    insert_fake_instance_with_session_dir_and_pins(
        state,
        id,
        pid,
        session_dir,
        test_profile_revision(),
        test_profile_payload_hash(),
        test_asset_pins(),
    );
}

fn insert_fake_instance_with_session_dir_and_pins(
    state: &ServiceState,
    id: &str,
    pid: u32,
    session_dir: PathBuf,
    profile_revision: String,
    profile_payload_hash: String,
    asset_pins: BootAssetPins,
) {
    state.instances.lock().unwrap().insert(
        id.to_string(),
        InstanceInfo {
            id: id.to_string(),
            name: id.to_string(),
            profile_id: "code".into(),
            profile_revision,
            profile_payload_hash,
            asset_pins,
            pid,
            uds_path: PathBuf::from(format!("/tmp/{}.sock", id)),
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

fn test_profile_revision() -> String {
    ProfileConfigFile::builtin_primary().revision
}

fn materialized_test_profile() -> ProfileConfigFile {
    materialized_test_profile_for("code")
}

fn materialized_test_profile_for(profile_id: &str) -> ProfileConfigFile {
    let profile_path = checked_in_profile_dir(profile_id).join("profile.toml");
    let mut profile: ProfileConfigFile = toml::from_str(&std::fs::read_to_string(profile_path).unwrap()).unwrap();
    let hash = format!("blake3:{}", blake3::hash(b"test-asset").to_hex());
    let size = b"test-asset".len() as u64;
    for arch_assets in profile.assets.arch.values_mut() {
        for asset in [
            &mut arch_assets.kernel,
            &mut arch_assets.initrd,
            &mut arch_assets.rootfs,
        ] {
            asset.hash = Some(hash.clone());
            asset.size = Some(size);
        }
    }
    pin_checked_in_profile_files(&mut profile);
    profile
}

fn test_profile_payload_hash() -> String {
    profile_payload_hash(&materialized_test_profile()).unwrap()
}

fn test_asset_pins() -> BootAssetPins {
    profile_asset_pins(&materialized_test_profile()).unwrap()
}

fn install_test_profile_assets(state: &ServiceState) {
    let profile = materialized_test_profile();
    install_test_profile_catalog(state, &profile);

    let arch = capsem_core::net::policy_config::current_profile_arch();
    let arch_dir = state.assets_dir.join(arch);
    std::fs::create_dir_all(&arch_dir).unwrap();
    let assets = profile.assets.current_arch_assets().unwrap();
    for asset in [&assets.kernel, &assets.initrd, &assets.rootfs] {
        std::fs::write(
            arch_dir.join(profile_asset_hash_name(asset).expect("profile asset hash name")),
            b"test-asset",
        )
        .unwrap();
    }
    refresh_profile_route_caches(state).expect("test profile route caches should refresh");
}

fn install_test_profile_catalog(state: &ServiceState, profile: &ProfileConfigFile) {
    let config_root = state.run_dir.join("config");
    let profile_dir = config_root.join("profiles").join(&profile.id);
    copy_dir_all(checked_in_profile_dir(&profile.id).as_path(), &profile_dir);
    std::fs::write(
        profile_dir.join("profile.toml"),
        toml::to_string_pretty(&profile).unwrap(),
    )
    .unwrap();
    super::set_test_profile_dir_override(Some(config_root.join("profiles")));
}

fn test_persistent_entry(name: &str, session_dir: PathBuf) -> PersistentVmEntry {
    PersistentVmEntry {
        id: new_persistent_vm_id(),
        name: name.into(),
        profile_id: "code".into(),
        profile_revision: test_profile_revision(),
        profile_payload_hash: test_profile_payload_hash(),
        asset_pins: test_asset_pins(),
        ram_mb: 2048,
        cpus: 2,
        base_version: "0.0.0".into(),
        created_at: "0".into(),
        session_dir,
        forked_from: None,
        description: None,
        suspended: false,
        defunct: false,
        last_error: None,
        checkpoint_path: None,
        env: None,
    }
}

/// Copy a checked-in profile tree into a test's scratch directory.
///
/// Two details that are not incidental.
///
/// `std::fs::copy` gives the destination the *source's* permissions, and then
/// fails with `EACCES` if it is asked to write a destination that already
/// exists without write permission. Copying a tree twice into one place is
/// therefore self-blocking, so an existing target is removed first.
///
/// And every failure names the file. This panicked as a bare
/// `Os { code: 13, kind: PermissionDenied }` with no path, which under
/// parallel `nextest` inside the Linux container produced an intermittent
/// failure nobody could place -- the message identified neither which file
/// nor which side of the copy.
fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).unwrap_or_else(|e| panic!("create {}: {e}", dst.display()));
    let entries = std::fs::read_dir(src).unwrap_or_else(|e| panic!("read dir {}: {e}", src.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| panic!("read entry under {}: {e}", src.display()));
        let ty = entry
            .file_type()
            .unwrap_or_else(|e| panic!("stat {}: {e}", entry.path().display()));
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &target);
        } else {
            if target.exists() {
                std::fs::remove_file(&target).unwrap_or_else(|e| panic!("replace {}: {e}", target.display()));
            }
            std::fs::copy(entry.path(), &target)
                .unwrap_or_else(|e| panic!("copy {} -> {}: {e}", entry.path().display(), target.display()));
        }
    }
}

fn checked_in_profile_dir(profile_id: &str) -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../config/profiles")
        .join(profile_id)
}

fn install_code_profile_fixture(dir: &tempfile::TempDir) -> PathBuf {
    let config_root = dir.path().join("config");
    let profile_dir = config_root.join("profiles/code");
    copy_dir_all(checked_in_profile_dir("code").as_path(), &profile_dir);
    config_root
}

fn profile_file_descriptor(
    config_root: &std::path::Path,
    path: &std::path::Path,
) -> capsem_core::net::policy_config::ProfileFileDescriptor {
    let bytes = std::fs::metadata(path).unwrap().len();
    let hash = capsem_assets::asset_manager::hash_file(path).unwrap();
    let relative = path
        .strip_prefix(config_root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();
    capsem_core::net::policy_config::ProfileFileDescriptor {
        path: relative,
        hash: Some(format!("blake3:{hash}")),
        size: Some(bytes),
    }
}

fn assign_file_descriptor_profile(
    profile: &mut ProfileConfigFile,
    descriptor: capsem_core::net::policy_config::ProfileFileDescriptor,
) {
    match std::path::Path::new(&descriptor.path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap()
    {
        "enforcement.toml" => {
            profile.files.enforcement = Some(descriptor);
        }
        "detection.yaml" => {
            profile.files.detection = Some(descriptor);
        }
        "mcp.json" => {
            profile.files.mcp = Some(descriptor);
        }
        "apt-packages.txt" => {
            profile.files.apt_packages = Some(descriptor);
        }
        "python-requirements.txt" => {
            profile.files.python_requirements = Some(descriptor);
        }
        "python-requirements.lock" => {
            profile.files.python_requirements_lock = Some(descriptor);
        }
        "npm-packages.txt" => {
            profile.files.npm_packages = Some(descriptor);
        }
        "npm-package-lock.json" => {
            profile.files.npm_package_lock = Some(descriptor);
        }
        "build.sh" => {
            profile.files.build = Some(descriptor);
        }
        "tips.txt" => {
            profile.files.tips = Some(descriptor);
        }
        "root.manifest.json" => {
            profile.files.root_manifest = Some(descriptor);
        }
        other => panic!("unsupported profile fixture descriptor {other}"),
    }
}

fn write_file_descriptor_profile(
    profile: &mut ProfileConfigFile,
    config_root: &std::path::Path,
    path: &std::path::Path,
) {
    assign_file_descriptor_profile(profile, profile_file_descriptor(config_root, path));
}

fn pin_checked_in_profile_files(profile: &mut ProfileConfigFile) {
    let repo_config_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
    let profile_dir = repo_config_root.join("profiles").join(&profile.id);
    for filename in [
        "enforcement.toml",
        "detection.yaml",
        "mcp.json",
        "apt-packages.txt",
        "python-requirements.txt",
        "python-requirements.lock",
        "npm-packages.txt",
        "npm-package-lock.json",
        "build.sh",
        "tips.txt",
        "root.manifest.json",
    ] {
        write_file_descriptor_profile(profile, &repo_config_root, &profile_dir.join(filename));
    }
}

fn install_file_asset_profile_fixture(dir: &tempfile::TempDir) -> (PathBuf, ProfileConfigFile) {
    let config_root = install_code_profile_fixture(dir);
    let profile_dir = config_root.join("profiles/code");
    let arch = capsem_core::net::policy_config::current_profile_arch();
    let source_dir = dir.path().join("asset-source").join(arch);
    std::fs::create_dir_all(&source_dir).unwrap();

    let mut profile = ProfileConfigFile::builtin_primary();
    for (name, body) in [
        ("vmlinuz", b"fixture-kernel".as_slice()),
        ("initrd.img", b"fixture-initrd".as_slice()),
        ("rootfs.erofs", b"fixture-rootfs".as_slice()),
    ] {
        std::fs::write(source_dir.join(name), body).unwrap();
    }
    let arch_assets = profile.assets.arch.get_mut(arch).unwrap();
    for asset in [
        &mut arch_assets.kernel,
        &mut arch_assets.initrd,
        &mut arch_assets.rootfs,
    ] {
        let source = source_dir.join(&asset.name);
        let hash = capsem_assets::asset_manager::hash_file(&source).unwrap();
        asset.url = format!("file://{}", source.display());
        asset.hash = Some(format!("blake3:{hash}"));
        asset.size = Some(std::fs::metadata(&source).unwrap().len());
    }
    for filename in [
        "enforcement.toml",
        "detection.yaml",
        "mcp.json",
        "apt-packages.txt",
        "python-requirements.txt",
        "python-requirements.lock",
        "npm-packages.txt",
        "npm-package-lock.json",
        "build.sh",
        "tips.txt",
        "root.manifest.json",
    ] {
        write_file_descriptor_profile(&mut profile, &config_root, &profile_dir.join(filename));
    }
    std::fs::write(
        profile_dir.join("profile.toml"),
        toml::to_string_pretty(&profile).unwrap(),
    )
    .unwrap();
    (config_root, profile)
}

fn add_profile_enforcement_rule(
    config_root: &std::path::Path,
    rule_id: &str,
    rule: capsem_core::net::policy_config::SecurityRule,
) {
    let profile_dir = config_root.join("profiles/code");
    let enforcement_path = profile_dir.join("enforcement.toml");
    let content = std::fs::read_to_string(&enforcement_path).unwrap();
    let mut rule_profile = SecurityRuleProfile::parse_toml(&content).unwrap();
    rule_profile.profiles.rules.insert(rule_id.to_string(), rule);
    std::fs::write(&enforcement_path, toml::to_string_pretty(&rule_profile).unwrap()).unwrap();
    let mut profile: ProfileConfigFile =
        toml::from_str(&std::fs::read_to_string(profile_dir.join("profile.toml")).unwrap()).unwrap();
    write_file_descriptor_profile(&mut profile, config_root, &enforcement_path);
    std::fs::write(
        profile_dir.join("profile.toml"),
        toml::to_string_pretty(&profile).unwrap(),
    )
    .unwrap();
}

// Image handler tests (service-level unit tests)
// -----------------------------------------------------------------------

fn make_test_state_with_tempdir() -> (Arc<ServiceState>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("run");
    std::fs::create_dir_all(&run_dir).unwrap();
    let registry_path = run_dir.join("persistent_registry.json");
    let asset_status_path = asset_status_path_for_run_dir(&run_dir);
    let state = Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        session_db_handles: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path).expect("registry loads")),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: dir.path().join("assets"),
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

mod assets_registry;
mod files_api;
mod ledger_routes;
mod lifecycle;
mod persist_purge;
mod profile_mutations;
mod profile_routes;
mod session_identity;
mod settings_files;
mod system_contracts;
mod update_routes;

pub(crate) use assets_registry::make_state_in;
use settings_files::{
    ensure_test_builtin_mcp_binary, install_empty_settings_env, make_test_state_with_tempdir_at, EnvVarGuard,
};
use update_routes::decode_response_json;
