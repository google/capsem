use super::*;
use axum::body::{to_bytes, Body};
use capsem_assets::asset_manager::{
    hash_filename, host_manifest_arch, AssetEntry, AssetRelease, AssetsSection, BinariesSection, ManifestV2,
};
use std::sync::atomic::AtomicU64;
use tower::ServiceExt;

mod asset_status;
mod asset_wait;
mod instance_reaper;
mod policy_push;

pub(crate) static SETTINGS_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(target_os = "linux")]
#[test]
fn overlay_prewarm_creates_the_default_size_and_reuses_it() {
    let dir = tempfile::tempdir().unwrap();
    let expected = capsem_core::system_overlay_template_path(dir.path(), DEFAULT_SCRATCH_DISK_GB);
    prewarm_system_overlay_template(dir.path());
    let found: HashSet<PathBuf> = std::fs::read_dir(expected.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(
        found,
        HashSet::from([expected.clone()]),
        "prewarm must not allocate unrequested disk sizes"
    );
    let modified = std::fs::metadata(&expected).unwrap().modified().unwrap();
    prewarm_system_overlay_template(dir.path());
    assert_eq!(std::fs::metadata(&expected).unwrap().modified().unwrap(), modified);
}

/// A test state's home is its run directory's parent, so its host ledger stays in
/// the test's own temporary directory.
fn test_host_ledger(run_dir: &StdPath) -> Arc<capsem_logger::DbHandle> {
    host_ledger::open_host_ledger(&run_dir.parent().unwrap().join("sessions")).unwrap()
}

pub(crate) fn test_session_ledger(db_path: PathBuf) -> Result<session_db_handles::SessionLedger, String> {
    capsem_logger::DbHandle::open_external_reader(&db_path)
        .map(|db| session_db_handles::SessionLedger::embedded(db_path, db))
        .map_err(|error| error.to_string())
}

/// The one test `ServiceState` constructor. The manifest is whatever
/// `assets_dir` holds, as at service startup.
fn test_state(run_dir: PathBuf, assets_dir: PathBuf, test_tempdir: Option<tempfile::TempDir>) -> ServiceState {
    std::fs::create_dir_all(&run_dir).unwrap();
    let manifest = capsem_assets::asset_manager::load_manifest_for_assets(&assets_dir).map(Arc::new);
    ServiceState {
        instances: Mutex::new(HashMap::new()),
        proxy_workers: Mutex::new(HashMap::new()),
        ledger_workers: Arc::new(ledger_worker::LedgerWorkers::new(PathBuf::from(
            "/nonexistent/capsem-ledger",
        ))),
        retirements: Default::default(),
        session_db_handles: Mutex::new(HashMap::new()),
        persistent_registry: SharedRegistry::new(
            PersistentRegistry::load(run_dir.join("persistent_registry.json")).expect("registry loads"),
        ),
        networks: tokio::sync::Mutex::new(capsem_core::net::network_registry::NetworkRegistry::new(
            run_dir.join("networks"),
        )),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        proxy_binary: PathBuf::from("/nonexistent/capsem-proxy"),
        assets_dir,
        service_socket: run_dir.join("service.sock"),
        switches: switches::Switches::in_process(),
        job_counter: AtomicU64::new(1),
        manifest: RwLock::new(manifest),
        current_version: "0.0.0".into(),
        asset_reconcile: Mutex::new(AssetReconcileState::default()),
        asset_reconcile_inflight: AtomicBool::new(false),
        asset_status_path: asset_status_path_for_run_dir(&run_dir),
        mcp_tool_cache: Mutex::new(capsem_core::mcp::load_tool_cache()),
        plugin_policy_cache: Mutex::new(Default::default()),
        host_ledger: test_host_ledger(&run_dir),
        host_stats: Mutex::new(Default::default()),
        last_defunct_reconcile_ms: AtomicU64::new(0),
        stats_detail_response_cache: Mutex::new(HashMap::new()),
        containers: Default::default(),
        storage_diagnostics_cache: Mutex::new(HashMap::new()),
        persistent_resume_state_cache: Mutex::new(HashMap::new()),
        asset_manifest_cache: Mutex::new(None),
        list_response_cache: Mutex::new(None),
        lifecycle: capsem_service::lifecycle::VmLifecycle::default(),
        shutdown_lock: tokio::sync::Mutex::new(()),
        update_lock: tokio::sync::Mutex::new(()),
        policy_mutation: policy_mutation::PolicyMutationLock::default(),
        update_restart: tokio::sync::Notify::new(),
        run_dir,
        _test_tempdir: test_tempdir,
    }
}

pub(crate) fn make_test_state() -> Arc<ServiceState> {
    Arc::new(make_test_state_owned())
}

/// The test state before it is shared, for tests that replace an owner.
pub(crate) fn make_test_state_owned() -> ServiceState {
    let test_tempdir = tempfile::tempdir().unwrap();
    let root = test_tempdir.path().to_path_buf();
    test_state(root.join("run"), root.join("assets"), Some(test_tempdir))
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

pub(super) fn make_asset_state(assets_dir: PathBuf) -> Arc<ServiceState> {
    Arc::new(test_state(assets_dir.join("run"), assets_dir, None))
}

/// The fields every fake instance shares; a test names only what it varies.
pub(crate) fn test_instance() -> InstanceInfo {
    InstanceInfo {
        generation: uuid::Uuid::new_v4(),
        authority: Default::default(),
        upstream_policy: upstream_broker::test_policy_publisher(),
        id: String::new(),
        name: String::new(),
        asset_pins: test_asset_pins(),
        pid: std::process::id(),
        uds_path: PathBuf::new(),
        session_dir: PathBuf::new(),
        ram_mb: 2048,
        cpus: 2,
        start_time: std::time::Instant::now(),
        base_version: "0.0.0".into(),
        persistent: false,
        env: None,
        forked_from: None,
    }
}

fn insert_fake_instance(state: &ServiceState, id: &str, pid: u32) {
    insert_fake_instance_with_session_dir(state, id, pid, state.run_dir.join("sessions").join(id));
}

/// The reply a fake process produces for one received message.
pub(crate) type FakeProcessReply =
    std::pin::Pin<Box<dyn std::future::Future<Output = Option<ProcessToService>> + Send>>;

const FAKE_PROCESS_ACCEPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// A stand-in capsem-process listening on `uds_path`: accepts `expected`
/// service connections one at a time, answers each message through
/// `handler` (no reply closes the connection), and returns everything it
/// received. The one fixture for every test that needs the process side
/// of the IPC; a fake instance's `uds_path` is where to bind it.
pub(crate) fn spawn_fake_process(
    uds_path: &StdPath,
    expected: usize,
    handler: impl Fn(&ServiceToProcess) -> FakeProcessReply + Send + Sync + 'static,
) -> tokio::task::JoinHandle<Vec<ServiceToProcess>> {
    let _ = std::fs::remove_file(uds_path);
    let listener = tokio::net::UnixListener::bind(uds_path).unwrap();
    std::fs::write(uds_path.with_extension("ready"), b"ready").unwrap();
    tokio::spawn(async move {
        let mut messages = Vec::new();
        for received in 0..expected {
            // A route that stops talking to its VM used to hang the whole
            // binary here: tests serialized on SETTINGS_ENV_LOCK stalled
            // behind the one waiting forever. Fail with what was missing.
            let (stream, _) = tokio::time::timeout(FAKE_PROCESS_ACCEPT_TIMEOUT, listener.accept())
                .await
                .unwrap_or_else(|_| panic!("fake capsem-process got {received} of {expected} expected IPC connections"))
                .unwrap();
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
                capsem_foundation::ipc_channel::Sender<ProcessToService>,
                capsem_foundation::ipc_channel::Receiver<ServiceToProcess>,
            ) = capsem_foundation::ipc_channel::channel_from_std(std_stream).unwrap();
            let message = rx.recv().await.unwrap();
            if let Some(reply) = handler(&message).await {
                tx.send(reply).await.unwrap();
            }
            messages.push(message);
        }
        messages
    })
}

/// True when a fake process received exactly `count` reload requests.
pub(crate) fn only_reloads(received: &[ServiceToProcess], count: usize) -> bool {
    received.len() == count
        && received
            .iter()
            .all(|message| matches!(message, ServiceToProcess::ReloadConfig { .. }))
}

/// A VM owner that freezes, lets the service clone, and then reports thaw.
pub(crate) fn spawn_fake_fork_owner(
    uds_path: &StdPath,
    _source: PathBuf,
    refusal: Option<&'static str>,
) -> tokio::task::JoinHandle<Vec<ServiceToProcess>> {
    let _ = std::fs::remove_file(uds_path);
    let listener = tokio::net::UnixListener::bind(uds_path).unwrap();
    std::fs::write(uds_path.with_extension("ready"), b"ready").unwrap();
    tokio::spawn(async move {
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
            capsem_foundation::ipc_channel::Sender<ProcessToService>,
            capsem_foundation::ipc_channel::Receiver<ServiceToProcess>,
        ) = capsem_foundation::ipc_channel::channel_from_std(std_stream).unwrap();
        let request = rx.recv().await.unwrap();
        let ServiceToProcess::CloneState { id } = request else {
            panic!("fork sent an unexpected owner message: {request:?}")
        };
        let mut messages = vec![request];
        if let Some(reason) = refusal {
            tx.send(ProcessToService::CloneStateResult {
                id,
                size_bytes: None,
                error: Some(reason.to_string()),
            })
            .await
            .unwrap();
            return messages;
        }
        tx.send(ProcessToService::CloneStateReady { id }).await.unwrap();
        let completion = rx.recv().await.unwrap();
        let ServiceToProcess::CloneStateComplete {
            id: completed_id,
            size_bytes,
            error,
        } = &completion
        else {
            panic!("fork sent an unexpected completion: {completion:?}")
        };
        assert_eq!(*completed_id, id);
        tx.send(ProcessToService::CloneStateResult {
            id,
            size_bytes: *size_bytes,
            error: error.clone(),
        })
        .await
        .unwrap();
        messages.push(completion);
        messages
    })
}

/// A fake process that answers ping, and reloads by reporting the digest of
/// the active policy it finds in its session, as capsem-process does.
pub(crate) fn spawn_fake_process_reload_ack(
    uds_path: &StdPath,
    expected: usize,
) -> tokio::task::JoinHandle<Vec<ServiceToProcess>> {
    let active_policy = uds_path
        .parent()
        .unwrap()
        .join(ACTIVE_POLICY_DIR)
        .join(ACTIVE_POLICY_FILE);
    spawn_fake_process(uds_path, expected, move |message| {
        let reply = match message {
            ServiceToProcess::Ping => Some(ProcessToService::Pong),
            ServiceToProcess::ReloadConfig { id } => Some(ProcessToService::ConfigReloadResult {
                id: *id,
                active_policy_digest: Some(capsem_core::net::policy_config::active_policy_digest(
                    &std::fs::read(&active_policy).unwrap(),
                )),
                error: None,
            }),
            _ => None,
        };
        Box::pin(async move { reply })
    })
}

pub(crate) fn insert_fake_instance_with_session_dir(state: &ServiceState, id: &str, pid: u32, session_dir: PathBuf) {
    state.instances.lock().unwrap().insert(
        id.to_string(),
        InstanceInfo {
            id: id.to_string(),
            name: id.to_string(),
            pid,
            // Inside the session, never a global path: a test that binds a
            // fake process here cannot collide with another run's leftovers.
            uds_path: session_dir.join("process.sock"),
            session_dir,
            ..test_instance()
        },
    );
}

/// The test runtime asset set: each boot image's logical name and bytes.
const TEST_RUNTIME_ASSETS: [(&str, &[u8]); 3] = [
    ("vmlinuz", b"test-kernel"),
    ("initrd.img", b"test-initrd"),
    ("rootfs.erofs", b"test-rootfs"),
];
const TEST_ASSET_VERSION: &str = "2030.0101.1";

fn test_asset_hex(body: &[u8]) -> String {
    blake3::hash(body).to_hex().to_string()
}

/// A format-2 manifest whose one release, for this host's architecture, is
/// the test runtime asset set.
fn test_runtime_manifest() -> ManifestV2 {
    let entries = TEST_RUNTIME_ASSETS
        .iter()
        .map(|(name, body)| {
            (
                name.to_string(),
                AssetEntry {
                    hash: test_asset_hex(body),
                    sha256: String::new(),
                    size: body.len() as u64,
                },
            )
        })
        .collect();
    ManifestV2 {
        format: 2,
        refresh_policy: "24h".into(),
        asset_base: None,
        assets: AssetsSection {
            current: TEST_ASSET_VERSION.into(),
            releases: HashMap::from([(
                TEST_ASSET_VERSION.into(),
                AssetRelease {
                    date: "2030-01-01".into(),
                    deprecated: false,
                    deprecated_date: None,
                    min_binary: String::new(),
                    arches: HashMap::from([(host_manifest_arch().to_string(), entries)]),
                },
            )]),
        },
        binaries: BinariesSection {
            current: "0.0.0".into(),
            releases: HashMap::new(),
        },
    }
}

/// The pins a VM created from the test runtime asset set records.
pub(crate) fn test_asset_pins() -> BootAssetPins {
    let pin = |(name, body): (&str, &[u8])| BootAssetPin {
        name: name.to_string(),
        hash: format!("blake3:{}", test_asset_hex(body)),
    };
    BootAssetPins {
        kernel: pin(TEST_RUNTIME_ASSETS[0]),
        initrd: pin(TEST_RUNTIME_ASSETS[1]),
        rootfs: pin(TEST_RUNTIME_ASSETS[2]),
    }
}

/// Install the test manifest, on disk and as the state's, without its images.
fn install_test_runtime_manifest(state: &ServiceState) {
    let manifest = test_runtime_manifest();
    std::fs::create_dir_all(&state.assets_dir).unwrap();
    std::fs::write(
        state.assets_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    *state.manifest.write().unwrap() = Some(Arc::new(manifest));
}

/// Install the test runtime asset set: the manifest and every image under
/// its hash name, so a new VM's assets are ready.
fn install_test_runtime_assets(state: &ServiceState) {
    install_test_runtime_manifest(state);
    let arch_dir = state.assets_dir.join(host_manifest_arch());
    std::fs::create_dir_all(&arch_dir).unwrap();
    for (name, body) in TEST_RUNTIME_ASSETS {
        std::fs::write(arch_dir.join(hash_filename(name, &test_asset_hex(body))), body).unwrap();
    }
}

pub(crate) fn test_persistent_entry(name: &str, session_dir: PathBuf) -> PersistentVmEntry {
    PersistentVmEntry {
        id: new_persistent_vm_id(),
        name: name.into(),
        legacy_profile_id: None,
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

// Image handler tests (service-level unit tests)
// -----------------------------------------------------------------------

fn make_test_state_with_tempdir() -> (Arc<ServiceState>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state(dir.path().join("run"), dir.path().join("assets"), None);
    (Arc::new(state), dir)
}

mod assets_registry;
mod async_io_contract;
mod db_handle_ownership;
mod exec_target;
mod files_api;
mod files_paths;
mod fork;
mod host_ledger_events;
mod inspection;
mod interactions;
mod ipc_command;
mod ledger_routes;
mod lifecycle;
mod logs_api;
mod mcp_routes;
mod network_routes;
mod persist_purge;
mod plugin_routes;
mod polled_route_cost;
mod restart;
mod route_query_plans;
mod session_identity;
mod settings_files;
mod stop_cleanup;
mod system_contracts;
mod telemetry_export;
mod transcript;
mod update_routes;
mod vm_info;

pub(crate) use assets_registry::make_state_in;
use settings_files::{
    ensure_test_builtin_mcp_binary, install_empty_settings_env, make_test_state_with_tempdir_at, EnvVarGuard,
};
use update_routes::decode_response_json;
