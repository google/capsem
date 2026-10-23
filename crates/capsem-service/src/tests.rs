use super::*;

pub(crate) fn isolated_std_test_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    let coverage_profile = std::env::var_os("LLVM_PROFILE_FILE");
    let mut command = std::process::Command::new(program);
    command.env_clear();
    if let Some(profile) = coverage_profile {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    command
}

pub(crate) fn isolated_test_command(program: impl AsRef<std::ffi::OsStr>) -> tokio::process::Command {
    tokio::process::Command::from(isolated_std_test_command(program))
}

pub(crate) fn retain_coverage_profile(command: &mut tokio::process::Command) {
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
}

#[test]
fn isolated_test_children_keep_only_the_coverage_output_path() {
    let mut command = isolated_test_command("true");
    command.env_clear();
    retain_coverage_profile(&mut command);
    let environment: Vec<_> = command.as_std().get_envs().collect();
    match std::env::var_os("LLVM_PROFILE_FILE") {
        Some(profile) => assert_eq!(
            environment,
            [(std::ffi::OsStr::new("LLVM_PROFILE_FILE"), Some(profile.as_os_str()))]
        ),
        None => assert!(environment.is_empty()),
    }
}
use axum::body::{to_bytes, Body};
use capsem_assets::asset_manager::{
    hash_filename, host_manifest_arch, AssetEntry, AssetRelease, AssetsSection, BinariesSection, ManifestV2,
};
use std::sync::atomic::{AtomicU64, AtomicUsize};
use tower::ServiceExt;

mod asset_status;
mod asset_wait;
mod instance_reaper;
mod policy_push;
#[path = "tests/session_housekeeping.rs"]
mod session_housekeeping_tests;

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
pub(crate) fn test_state(
    run_dir: PathBuf,
    assets_dir: PathBuf,
    test_tempdir: Option<tempfile::TempDir>,
) -> ServiceState {
    std::fs::create_dir_all(&run_dir).unwrap();
    let manifest = capsem_assets::asset_manager::load_manifest_for_assets(&assets_dir).map(Arc::new);
    let managed = managed_routes::Runtime::open(&run_dir, Default::default()).unwrap();
    let connections =
        connection_routes::Runtime::new(run_dir.join("credentials/broker.json"), managed.grant_authority());
    ServiceState {
        managed,
        connections,
        approvals: Default::default(),
        instances: Mutex::new(HashMap::new()),
        proxy_workers: Mutex::new(HashMap::new()),
        standalone_proxies: tokio::sync::Mutex::new(HashMap::new()),
        ledger_workers: Arc::new(ledger_worker::LedgerWorkers::new(
            PathBuf::from("/nonexistent/capsem-ledger"),
            None,
        )),
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
        plugin_policy_cache: GenerationCache::new("service.plugin_policy"),
        host_ledger: test_host_ledger(&run_dir),
        host_stats: Mutex::new(Default::default()),
        last_defunct_reconcile_ms: AtomicU64::new(0),
        containers: Default::default(),
        storage_diagnostics_cache: BoundedCache::new(
            "service.storage_diagnostics",
            capsem_proto::CACHE_SERVICE_STORAGE_DIAGNOSTICS_ENTRIES,
        ),
        persistent_resume_state_cache: KeyedIdentityCache::new(
            "service.persistent_resume",
            capsem_proto::CACHE_SERVICE_PERSISTENT_RESUME_ENTRIES,
        ),
        asset_manifest_cache: IdentityCache::new("service.asset_manifest"),
        list_response_cache: IdentityCache::new("service.vm_list"),
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

pub(crate) fn install_test_ledger_worker(state: &mut ServiceState, directory: &StdPath) {
    state.ledger_workers = Arc::new(crate::ledger_worker::LedgerWorkers::new(
        crate::ledger_worker::tests::fake_binary(directory, "normal"),
        None,
    ));
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
        terminal: None,
        browser_request: None,
        browser_revision: 0,
        generation: capsem_crypto::uuid_v4(),
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
        labels: None,
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

type FakeProcessHandler = dyn Fn(&ServiceToProcess) -> FakeProcessReply + Send + Sync;

/// A stand-in capsem-process: independent connections, like the real owner.
/// Return exactly `expected` commands in receive order after replying. Idle
/// observer channels and credential admission do not consume that budget;
/// every outstanding connection is closed before the fixture returns.
pub(crate) fn spawn_fake_process(
    uds_path: &StdPath,
    expected: usize,
    handler: impl Fn(&ServiceToProcess) -> FakeProcessReply + Send + Sync + 'static,
) -> tokio::task::JoinHandle<Vec<ServiceToProcess>> {
    let _ = std::fs::remove_file(uds_path);
    let listener = tokio::net::UnixListener::bind(uds_path).unwrap();
    std::fs::write(uds_path.with_extension("ready"), b"ready").unwrap();
    let handler: Arc<FakeProcessHandler> = Arc::new(handler);
    tokio::spawn(async move {
        let mut messages = Vec::new();
        let sequence = Arc::new(AtomicUsize::new(0));
        let mut peers = tokio::task::JoinSet::new();
        let mut deadline = tokio::time::Instant::now() + FAKE_PROCESS_ACCEPT_TIMEOUT;
        while messages.len() < expected {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => {
                    panic!("fake capsem-process got {} of {expected} expected commands", messages.len());
                }
                accepted = listener.accept() => {
                    let (stream, _) = accepted.unwrap();
                    peers.spawn(answer_fake_process_connection(stream, Arc::clone(&handler), Arc::clone(&sequence)));
                }
                completed = peers.join_next(), if !peers.is_empty() => {
                    if let Some(message) = completed.unwrap().unwrap() {
                        messages.push(message);
                        deadline = tokio::time::Instant::now() + FAKE_PROCESS_ACCEPT_TIMEOUT;
                    }
                }
            }
        }
        drop(listener);
        peers.abort_all();
        while let Some(completed) = peers.join_next().await {
            if let Err(error) = completed {
                assert!(error.is_cancelled(), "fake owner connection panicked: {error}");
            }
        }
        assert_eq!(
            sequence.load(std::sync::atomic::Ordering::Relaxed),
            expected,
            "extra owner commands"
        );
        messages.sort_by_key(|(sequence, _)| *sequence);
        messages.into_iter().map(|(_, message)| message).collect()
    })
}

async fn answer_fake_process_connection(
    stream: tokio::net::UnixStream,
    handler: Arc<FakeProcessHandler>,
    sequence: Arc<AtomicUsize>,
) -> Option<(usize, ServiceToProcess)> {
    let mut stream = stream.into_std().unwrap();
    let negotiated = tokio::task::spawn_blocking(move || {
        let peer = capsem_foundation::ipc_handshake::negotiate_responder(&mut stream, "capsem-process-test", "")?;
        Ok::<_, capsem_proto::handshake::HandshakeError>((stream, peer))
    })
    .await
    .unwrap();
    let (stream, peer) = match negotiated {
        Ok(pair) => pair,
        Err(error) => {
            eprintln!("fake owner rejected handshake: {error}");
            return None;
        }
    };
    let (tx, rx): (Sender<ProcessToService>, Receiver<ServiceToProcess>) = channel_from_std(stream).unwrap();
    if peer.peer == "capsem-service-lifecycle" {
        let _tx = tx;
        let _ = rx.recv().await;
        return None;
    }
    let message = match rx.recv().await {
        Ok(message) => message,
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return None,
        Err(error) => panic!("fake owner request failed: {error}"),
    };
    if let ServiceToProcess::InjectCredentials { id, .. } = &message {
        if let Err(error) = tx
            .send(ProcessToService::CredentialsInjected { id: *id, error: None })
            .await
        {
            eprintln!("fake owner credential peer closed: {error}");
        }
        return None;
    }
    let sequence = sequence.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if let Some(reply) = handler(&message).await {
        if let Err(error) = tx.send(reply).await {
            eprintln!("fake owner reply peer closed: {error}");
        }
    }
    Some((sequence, message))
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
/// the exact active policy the service delivered, as capsem-process does.
pub(crate) fn spawn_fake_process_reload_ack(
    uds_path: &StdPath,
    expected: usize,
) -> tokio::task::JoinHandle<Vec<ServiceToProcess>> {
    spawn_fake_process(uds_path, expected, |message| {
        let reply = match message {
            ServiceToProcess::Ping => Some(ProcessToService::Pong),
            ServiceToProcess::ReloadConfig { id, active_policy } => Some(ProcessToService::ConfigReloadResult {
                id: *id,
                active_policy_digest: Some(capsem_core::net::policy_config::active_policy_digest(active_policy)),
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
            terminal: None,
            browser_request: None,
            browser_revision: 0,
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
    capsem_crypto::blake3::hash(body).to_hex().to_string()
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
pub(crate) fn install_test_runtime_assets(state: &ServiceState) {
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
        labels: None,
    }
}

pub(crate) fn test_provision_options<'a>(id: &'a str, name: &'a str) -> ProvisionOptions<'a> {
    ProvisionOptions {
        id,
        name,
        ram_mb: 2048,
        cpus: 2,
        scratch_disk_size_gb: 16,
        version_override: None,
        persistent: false,
        env: None,
        labels: None,
        from: None,
        description: None,
    }
}

// Image handler tests (service-level unit tests)
// -----------------------------------------------------------------------

pub(crate) fn make_test_state_with_tempdir() -> (Arc<ServiceState>, tempfile::TempDir) {
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
mod suspend;
mod system_contracts;
mod telemetry_export;
mod transcript;
mod update_routes;
mod vm_info;

pub(crate) use assets_registry::make_state_in;
use settings_files::make_test_state_with_tempdir_at;
pub(crate) use settings_files::{install_empty_settings_env, EnvVarGuard};
use update_routes::decode_response_json;

#[test]
fn browser_request_is_in_inventory_and_invalidates_its_runtime_snapshot() {
    let (state, _dir) = make_test_state_with_tempdir();
    let instance = test_instance();
    let id = instance.id.clone();
    state.instances.lock().unwrap().insert(id.clone(), instance);
    let before = list_response_fingerprint(&state);
    state.instances.lock().unwrap().get_mut(&id).unwrap().browser_request = Some(capsem_api::BrowserRequest {
        owner_generation: None,
        id: 1,
        url: "https://example.org/login?state=opaque".into(),
    });
    let (response, _) = sandbox_info::build_list_response(&state);
    assert_eq!(response.sandboxes[0].browser_request.as_ref().unwrap().id, 1);
    assert_ne!(list_response_fingerprint(&state), before);
}

#[test]
fn terminal_metadata_is_in_inventory_and_invalidates_its_runtime_snapshot() {
    let (state, _dir) = make_test_state_with_tempdir();
    let mut instance = test_instance();
    instance.terminal = Some(capsem_api::TerminalSummary {
        revision: 1,
        title: "Agent".into(),
        bell_count: 1,
        last_notification: Some(capsem_api::TerminalNotification {
            id: "notice-1".into(),
            title: "Review".into(),
            body: "Ready".into(),
        }),
    });
    let id = instance.id.clone();
    let expected = instance.terminal.clone();
    state.instances.lock().unwrap().insert(id.clone(), instance);
    let before = list_response_fingerprint(&state);
    let (response, _) = sandbox_info::build_list_response(&state);
    assert_eq!(response.sandboxes[0].terminal, expected);
    state
        .instances
        .lock()
        .unwrap()
        .get_mut(&id)
        .unwrap()
        .terminal
        .as_mut()
        .unwrap()
        .revision += 1;
    assert_ne!(list_response_fingerprint(&state), before);
}
