use super::*;
use crate::tests::{insert_fake_instance_with_session_dir, spawn_fake_process};
use capsem_api::RegistryAccess;
use tokio::sync::Notify;

mod published;
mod registry;
mod surface;

mod retirement;

/// An image source serving a fixed two-file layout, optionally held at the
/// pull until released, and recording the registry access it was given.
struct FixtureImages {
    fail: bool,
    gate: Option<Arc<Notify>>,
    access: Arc<Mutex<Option<RegistryAccess>>>,
    /// Image config labels; `None` serves an image with none.
    labels: Option<serde_json::Value>,
    catalog_reads: Arc<std::sync::atomic::AtomicUsize>,
    catalog: Option<serde_json::Value>,
    root_calls: Arc<std::sync::atomic::AtomicUsize>,
    fetches: Arc<Mutex<Vec<ImageFetch>>>,
    cache_key: Option<capsem_assets::oci::CacheKey>,
    root_keys: Arc<Mutex<Vec<Option<capsem_assets::oci::CacheKey>>>>,
}

const MANIFEST_BLOB: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CONFIG_BLOB: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const LAYER_BLOB: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
/// What the fixture's layer holds: no byte of it may reach the workspace.
const LAYER_BYTES: &[u8] = b"layer bytes that stay in the image share";

/// index.json -> manifest -> config carrying `labels`, and one layer, as the
/// puller lays them out. Returns the layout's verified files.
fn write_labelled_layout(root: &StdPath, labels: &serde_json::Value) -> std::io::Result<Vec<PathBuf>> {
    let blobs = root.join("blobs/sha256");
    std::fs::create_dir_all(&blobs)?;
    let index = json!({"manifests": [{"digest": format!("sha256:{MANIFEST_BLOB}")}]});
    std::fs::write(root.join("index.json"), serde_json::to_vec(&index)?)?;
    std::fs::write(root.join("oci-layout"), b"")?;
    let manifest = json!({
        "config": {"digest": format!("sha256:{CONFIG_BLOB}")},
        "layers": [{"digest": format!("sha256:{LAYER_BLOB}")}],
    });
    std::fs::write(blobs.join(MANIFEST_BLOB), serde_json::to_vec(&manifest)?)?;
    std::fs::write(
        blobs.join(CONFIG_BLOB),
        serde_json::to_vec(&json!({"config": {"Labels": labels}}))?,
    )?;
    std::fs::write(blobs.join(LAYER_BLOB), LAYER_BYTES)?;
    let mut files = vec![PathBuf::from("index.json"), PathBuf::from("oci-layout")];
    files.extend([MANIFEST_BLOB, CONFIG_BLOB, LAYER_BLOB].map(|blob| PathBuf::from("blobs/sha256").join(blob)));
    Ok(files)
}

impl ImageSource for FixtureImages {
    /// The fixture registry is granted; everything else stays denied.
    fn policy(&self) -> PolicyFuture {
        let granted = capsem_core::net::policy_config::SettingsFile {
            images: Some(capsem_core::net::policy_config::ImagePolicyConfig {
                sources: vec!["registry.example".into()],
                admit: vec!["registry.example".into()],
                ..Default::default()
            }),
            ..Default::default()
        };
        Box::pin(
            async move { capsem_core::container::admission::ImagePolicy::from_files(&granted, &Default::default()) },
        )
    }

    fn fetch_catalog(&self, _source: CatalogSource, _parent: PathBuf) -> images::CatalogFuture {
        self.catalog_reads.fetch_add(1, Ordering::Relaxed);
        let catalog = self.catalog.clone();
        Box::pin(async move {
            let catalog = catalog.ok_or_else(|| anyhow::anyhow!("no catalog in this fixture"))?;
            Ok((
                capsem_assets::oci::Digest::parse(&format!("sha256:{MANIFEST_BLOB}"))?,
                capsem_assets::oci::Catalog::parse(&serde_json::to_vec(&catalog)?)?,
            ))
        })
    }

    fn fetch_rootfs(
        &self,
        _reference: String,
        _subject: String,
        _access: RegistryAccess,
        _parent: PathBuf,
        mode: ImageFetch,
        cache_key: Option<capsem_assets::oci::CacheKey>,
    ) -> RootfsFuture {
        self.root_keys.lock().unwrap().push(cache_key);
        self.fetches.lock().unwrap().push(mode);
        self.root_calls.fetch_add(1, Ordering::Relaxed);
        Box::pin(async { anyhow::bail!("fixture artifact unavailable") })
    }

    fn pull(&self, _image: String, access: RegistryAccess, _parent: PathBuf, mode: ImageFetch) -> PullFuture {
        self.fetches.lock().unwrap().push(mode);
        let (fail, gate, seen) = (self.fail, self.gate.clone(), Arc::clone(&self.access));
        let labels = self.labels.clone();
        let cache_key = self.cache_key.clone();
        Box::pin(async move {
            *seen.lock().unwrap() = Some(access);
            if let Some(gate) = gate {
                gate.notified().await;
            }
            anyhow::ensure!(!fail, "registry refused the image");
            let root = tempfile::tempdir()?;
            let files = write_labelled_layout(root.path(), &labels.unwrap_or_else(|| json!({})))?;
            Ok(PulledImage {
                root: root.path().to_path_buf(),
                files,
                digest: "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into(),
                image_digest: "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into(),
                cache_key,
                _hold: Box::new(root),
            })
        })
    }
}

struct Fixture {
    state: Arc<ServiceState>,
    workspace: PathBuf,
    uds_path: PathBuf,
    _dir: tempfile::TempDir,
}

fn fixture(images: FixtureImages) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let mut state = crate::tests::make_test_state_owned();
    state.containers = ContainerSetups::with_source(Box::new(images));
    let state = Arc::new(state);
    let session_dir = dir.path().join("session");
    let workspace = session_dir.join("guest/workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    insert_fake_instance_with_session_dir(&state, "box", std::process::id(), session_dir);
    let uds_path = state.instances.lock().unwrap()["box"].uds_path.clone();
    std::fs::write(uds_path.with_extension("ready"), b"1\n").unwrap();
    Fixture {
        state,
        workspace,
        uds_path,
        _dir: dir,
    }
}

#[tokio::test]
async fn pull_admission_waits_for_the_vm_owner_launch_barrier() {
    let access = Arc::new(Mutex::new(None));
    let fx = fixture(FixtureImages {
        access: Arc::clone(&access),
        ..images()
    });
    let owner = spawn_fake_process(&fx.uds_path, 1, |message| {
        let reply = match message {
            ServiceToProcess::AdmitContainerPull { id, .. } => Some(ProcessToService::ContainerPullAdmission {
                id: *id,
                error: Some("blocked after readiness".into()),
                policy_refused: true,
            }),
            other => panic!("pull admission sent an unexpected owner message: {other:?}"),
        };
        Box::pin(async move { reply })
    });
    // The fake owner announces readiness when it binds; withdraw it after, or
    // admission is free to reach the owner and this proves nothing.
    let ready_path = fx.uds_path.with_extension("ready");
    std::fs::remove_file(&ready_path).unwrap();

    start(&fx.state, "box".into(), spec(None));
    tokio::time::sleep(std::time::Duration::from_millis(75)).await;
    assert!(
        !owner.is_finished(),
        "admission reached the owner before its launch sentinel"
    );
    assert!(
        access.lock().unwrap().is_none(),
        "the registry was contacted before owner readiness"
    );

    std::fs::write(fx.uds_path.with_extension("launched"), b"1\n").unwrap();
    let status = wait_for(&fx.state, "box", |s| s.state == ContainerState::Failed).await;
    owner.await.unwrap();
    assert!(status.error.as_deref().unwrap().contains("blocked after readiness"));
    assert!(access.lock().unwrap().is_none());
    assert!(!ready_path.exists(), "host admission does not require a running guest");
}

fn images() -> FixtureImages {
    FixtureImages {
        fail: false,
        gate: None,
        access: Arc::new(Mutex::new(None)),
        labels: None,
        catalog_reads: Default::default(),
        catalog: None,
        root_calls: Default::default(),
        fetches: Default::default(),
        cache_key: None,
        root_keys: Default::default(),
    }
}

#[tokio::test]
async fn admitted_pull_progresses_while_the_guest_is_still_booting() {
    let fetches = Arc::new(Mutex::new(Vec::new()));
    let access = Arc::new(Mutex::new(None));
    let gate = Arc::new(Notify::new());
    let fx = fixture(FixtureImages {
        fetches: Arc::clone(&fetches),
        access: Arc::clone(&access),
        gate: Some(gate),
        ..images()
    });
    let owner = spawn_fake_process(&fx.uds_path, 1, |message| {
        let ServiceToProcess::AdmitContainerPull { id, .. } = message else {
            panic!("unexpected request before guest readiness: {message:?}");
        };
        let id = *id;
        Box::pin(async move {
            Some(ProcessToService::ContainerPullAdmission {
                id,
                error: None,
                policy_refused: false,
            })
        })
    });
    std::fs::remove_file(fx.uds_path.with_extension("ready")).unwrap();
    std::fs::write(fx.uds_path.with_extension("launched"), b"1\n").unwrap();
    start(&fx.state, "box".into(), spec(None));
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while access.lock().unwrap().is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("host-admitted image pull must overlap guest boot");
    assert_eq!(*fetches.lock().unwrap(), vec![ImageFetch::PreferCached]);
    assert!(!fx.uds_path.with_extension("ready").exists());
    owner.await.unwrap();
    fx.state.containers.cancel("box");
}

#[tokio::test]
async fn staged_workload_waits_for_guest_readiness_before_launching() {
    let fx = fixture(images());
    let owner = owner_accepting_stage_and_launch(&fx.uds_path, 4);
    let ready = fx.uds_path.with_extension("ready");
    std::fs::remove_file(&ready).unwrap();
    std::fs::write(fx.uds_path.with_extension("launched"), b"1\n").unwrap();
    start(&fx.state, "box".into(), spec(None));
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !fx.workspace.join(".capsem-image/launch.py").exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("control files must be staged during guest boot");
    tokio::time::sleep(std::time::Duration::from_millis(75)).await;
    assert!(!owner.is_finished(), "guest exec ran before its ready sentinel");
    assert_eq!(
        fx.state.containers.status("box").unwrap().state,
        ContainerState::Staging
    );
    std::fs::write(ready, b"1\n").unwrap();
    owner.await.unwrap();
    wait_for(&fx.state, "box", |status| status.state == ContainerState::Starting).await;
}

#[tokio::test]
async fn an_import_refused_during_boot_writes_no_staged_control_bytes() {
    let fx = fixture(images());
    let owner = spawn_fake_process(&fx.uds_path, 2, |message| {
        let reply = match message {
            ServiceToProcess::AdmitContainerPull { id, .. } => ProcessToService::ContainerPullAdmission {
                id: *id,
                error: None,
                policy_refused: false,
            },
            ServiceToProcess::LogFileBoundary { id, .. } => ProcessToService::LogFileBoundaryResult {
                id: *id,
                success: false,
                data: None,
                error: Some("stage import refused".into()),
            },
            other => panic!("unexpected request during refused staging: {other:?}"),
        };
        Box::pin(async move { Some(reply) })
    });
    std::fs::remove_file(fx.uds_path.with_extension("ready")).unwrap();
    std::fs::write(fx.uds_path.with_extension("launched"), b"1\n").unwrap();
    start(&fx.state, "box".into(), spec(None));
    let status = wait_for(&fx.state, "box", |status| status.state == ContainerState::Failed).await;
    owner.await.unwrap();
    assert!(status.error.as_deref().unwrap().contains("stage import refused"));
    assert!(!fx.workspace.join(".capsem-image/options.json").exists());
    assert!(!fx.workspace.join(".capsem-image/launch.py").exists());
}

#[tokio::test]
async fn create_wait_uses_shared_exponential_polling_until_running() {
    let fx = fixture(images());
    let generation = fx.state.containers.begin("box", "registry.example/app:1");
    let state = Arc::clone(&fx.state);
    let update = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        state
            .containers
            .advance("box", generation, |status| status.state = ContainerState::Running);
    });
    let status = wait_observed(
        &fx.state,
        "box",
        capsem_foundation::poll::PollOpts {
            label: "container-create-test",
            timeout: std::time::Duration::from_secs(1),
            initial_delay: std::time::Duration::from_millis(1),
            max_delay: std::time::Duration::from_millis(4),
        },
    )
    .await
    .unwrap();
    update.await.unwrap();
    assert_eq!(status.state, ContainerState::Running);
}

#[tokio::test(start_paused = true)]
async fn create_observes_readiness_without_a_half_second_backoff() {
    let fx = fixture(images());
    let generation = fx.state.containers.begin("box", "registry.example/app:1");
    let state = Arc::clone(&fx.state);
    let ready_at = tokio::time::Instant::now() + std::time::Duration::from_millis(800);
    let update = tokio::spawn(async move {
        tokio::time::sleep_until(ready_at).await;
        state
            .containers
            .advance("box", generation, |status| status.state = ContainerState::Running);
    });
    let status = wait_for_create(&fx.state, "box").await.unwrap();
    update.await.unwrap();
    assert_eq!(status.state, ContainerState::Running);
    assert!(
        tokio::time::Instant::now() - ready_at <= std::time::Duration::from_millis(50),
        "local workload readiness must be observed within the VM readiness poll budget"
    );
}

#[tokio::test]
async fn create_wait_returns_terminal_failure_without_retrying_setup() {
    let fx = fixture(images());
    let generation = fx.state.containers.begin("box", "registry.example/app:1");
    fx.state.containers.advance("box", generation, |status| {
        status.state = ContainerState::Failed;
        status.error = Some("pull refused".into());
    });
    let status = wait_observed(
        &fx.state,
        "box",
        capsem_foundation::poll::PollOpts::new("container-create-test", std::time::Duration::from_secs(1)),
    )
    .await
    .unwrap();
    assert_eq!(status.state, ContainerState::Failed);
    assert_eq!(status.error.as_deref(), Some("pull refused"));
}

/// The guest's stage markers (the launcher's `ready`, `running`, `failed`).
fn mark(fx: &Fixture, marker: &str) {
    std::fs::write(fx.workspace.join(".capsem-image").join(marker), b"1\n").unwrap();
}

/// `POST /vms/create` waits on this for a detached workload. The launcher runs
/// in the background, so only the guest's running marker says it is running; a
/// wait that ignores it spins until the HTTP deadline and returns 504.
#[tokio::test]
async fn create_wait_reports_a_detached_workload_running_once_the_guest_marks_it_running() {
    let fx = fixture(images());
    let owner = owner_accepting_stage_and_launch(&fx.uds_path, 4);
    start(&fx.state, "box".into(), spec(None));
    owner.await.unwrap();
    wait_for(&fx.state, "box", |s| s.state == ContainerState::Starting).await;
    // Staged and unpacked is not started: runc has not created the workload
    // yet, and an exec the create's caller sends next would find nothing.
    mark(&fx, "ready");
    let staged = wait_observed(
        &fx.state,
        "box",
        capsem_foundation::poll::PollOpts::new("container-create-test", std::time::Duration::from_millis(300)),
    )
    .await;
    assert!(staged.is_err(), "a staged workload settled the create wait: {staged:?}");
    mark(&fx, "running");
    let status = wait_observed(
        &fx.state,
        "box",
        capsem_foundation::poll::PollOpts::new("container-create-test", std::time::Duration::from_secs(2)),
    )
    .await
    .expect("a running detached workload settles the create wait");
    assert_eq!(status.state, ContainerState::Running);
    assert_eq!(
        status.digest.as_deref(),
        Some("sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
    );
}

/// A launch whose workload never started ends the create wait as a failure,
/// instead of leaving it to spin until the HTTP deadline.
#[tokio::test]
async fn create_wait_reports_a_detached_workload_that_never_started_as_failed() {
    let fx = fixture(images());
    let owner = owner_accepting_stage_and_launch(&fx.uds_path, 4);
    start(&fx.state, "box".into(), spec(None));
    owner.await.unwrap();
    wait_for(&fx.state, "box", |s| s.state == ContainerState::Starting).await;
    mark(&fx, "ready");
    mark(&fx, "failed");
    let status = wait_observed(
        &fx.state,
        "box",
        capsem_foundation::poll::PollOpts::new("container-create-test", std::time::Duration::from_secs(2)),
    )
    .await
    .expect("a failed launch settles the create wait");
    assert_eq!(status.state, ContainerState::Failed);
    assert_eq!(
        status.error.as_deref(),
        Some("the workload did not start; `capsem logs` shows why")
    );
}

fn owner_accepting_stage_and_launch(
    uds_path: &StdPath,
    expected: usize,
) -> tokio::task::JoinHandle<Vec<ServiceToProcess>> {
    spawn_fake_process(uds_path, expected, |message| {
        let reply = match message {
            ServiceToProcess::AdmitContainerPull { id, .. } => Some(ProcessToService::ContainerPullAdmission {
                id: *id,
                error: None,
                policy_refused: false,
            }),
            ServiceToProcess::LogFileBoundary { id, .. } => Some(ProcessToService::LogFileBoundaryResult {
                id: *id,
                success: true,
                data: None,
                error: None,
            }),
            ServiceToProcess::Exec { id, .. } => Some(ProcessToService::ExecResult {
                id: *id,
                stdout: vec![],
                stderr: vec![],
                exit_code: 0,
                truncated: false,
            }),
            other => panic!("unexpected IPC message during container setup: {other:?}"),
        };
        Box::pin(async move { reply })
    })
}

fn owner_admitting_pull(uds_path: &StdPath) -> tokio::task::JoinHandle<Vec<ServiceToProcess>> {
    spawn_fake_process(uds_path, 1, |message| {
        let reply = match message {
            ServiceToProcess::AdmitContainerPull { id, .. } => Some(ProcessToService::ContainerPullAdmission {
                id: *id,
                error: None,
                policy_refused: false,
            }),
            other => panic!("pull admission sent an unexpected owner message: {other:?}"),
        };
        Box::pin(async move { reply })
    })
}

#[tokio::test]
async fn refused_pull_admission_never_calls_the_image_source() {
    let fetches = Arc::new(Mutex::new(Vec::new()));
    let access = Arc::new(Mutex::new(None));
    let fx = fixture(FixtureImages {
        fetches: Arc::clone(&fetches),
        access: Arc::clone(&access),
        ..images()
    });
    let owner = spawn_fake_process(&fx.uds_path, 1, |message| {
        let reply = match message {
            ServiceToProcess::AdmitContainerPull {
                id,
                image,
                registry,
                digest,
            } => {
                assert_eq!(image, "registry.example/app:1");
                assert_eq!(registry, "registry.example");
                assert!(digest.is_none());
                Some(ProcessToService::ContainerPullAdmission {
                    id: *id,
                    error: Some("blocked by fixture policy".into()),
                    policy_refused: true,
                })
            }
            other => panic!("pull admission sent an unexpected owner message: {other:?}"),
        };
        Box::pin(async move { reply })
    });

    start(&fx.state, "box".into(), spec(None));
    let status = wait_for(&fx.state, "box", |s| s.state == ContainerState::Failed).await;
    owner.await.unwrap();

    assert!(status.error.as_deref().unwrap().contains("blocked by fixture policy"));
    assert!(
        access.lock().unwrap().is_none(),
        "a refused admission must send zero registry requests"
    );
    assert!(!fx.workspace.join(".capsem-image").exists());
    assert!(
        fetches.lock().unwrap().is_empty(),
        "owner denial must precede either materialization mode"
    );
}

async fn wait_for(
    state: &ServiceState,
    id: &str,
    done: impl Fn(&ContainerStatusResponse) -> bool,
) -> ContainerStatusResponse {
    for _ in 0..500 {
        if let Some(status) = state.containers.status(id).filter(|status| done(status)) {
            return status;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!(
        "container status never reached the expected state: {:?}",
        state.containers.status(id)
    );
}

fn spec(registry: Option<RegistryAccess>) -> ContainerSpec {
    ContainerSpec {
        image: "registry.example/app:1".into(),
        args: vec!["serve".into()],
        env: [("MODE".to_string(), "test".to_string())].into(),
        registry,
        attach: false,
    }
}

#[tokio::test]
async fn setup_stages_the_plan_through_the_import_ledger_then_launches_detached() {
    let images = images();
    let catalog_reads = Arc::clone(&images.catalog_reads);
    let fx = fixture(images);
    // Pull admission, options.json, launch.py, then the launch exec: the
    // image's blobs go to the share, not through the workspace.
    let owner = owner_accepting_stage_and_launch(&fx.uds_path, 4);
    start(&fx.state, "box".into(), spec(None));

    let status = wait_for(&fx.state, "box", |s| s.state == ContainerState::Starting).await;
    let messages = owner.await.unwrap();
    assert_eq!(
        status.digest.as_deref(),
        Some("sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
    );
    // The tag is gone from what the session records: the pin is.
    assert_eq!(
        status.resolved.as_deref(),
        Some("registry.example/app@sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
    );
    assert!(status.error.is_none());
    let staged: Vec<&str> = messages
        .iter()
        .filter_map(|m| match m {
            ServiceToProcess::LogFileBoundary { path, action, .. } => {
                assert_eq!(*action, FileBoundaryAction::Import);
                Some(path.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(staged, [".capsem-image/options.json", ".capsem-image/launch.py"]);
    match messages.last() {
        Some(ServiceToProcess::Exec { command, .. }) => {
            assert_eq!(command, &capsem_core::container::detached_launch_command())
        }
        other => panic!("setup must end with the detached launch, got {other:?}"),
    }
    let stage = fx.workspace.join(".capsem-image");
    let mut names: Vec<_> = std::fs::read_dir(&stage)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["launch.py", "options.json"], "only control files are staged");
    for file in walk(&fx.workspace) {
        let bytes = std::fs::read(&file).unwrap();
        assert!(
            !bytes.windows(LAYER_BYTES.len()).any(|window| window == LAYER_BYTES),
            "{} holds layer bytes",
            file.display()
        );
    }
    // The image is in the session's host-only share, every blob and nothing
    // else, read-only.
    let session = fx.workspace.parent().unwrap().parent().unwrap();
    assert_eq!(
        capsem_core::session::image_share_blobs(session).unwrap(),
        [MANIFEST_BLOB, CONFIG_BLOB, LAYER_BLOB]
    );
    let shared = capsem_core::session::image_share_path(session).join(LAYER_BLOB);
    assert_eq!(std::fs::read(&shared).unwrap(), LAYER_BYTES);
    assert_eq!(
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&shared).unwrap().permissions()) & 0o777,
        0o444
    );
    let options: serde_json::Value =
        serde_json::from_slice(&std::fs::read(stage.join("options.json")).unwrap()).unwrap();
    assert_eq!(
        options,
        json!({
            "manifest": format!("sha256:{MANIFEST_BLOB}"),
            "args": ["serve"],
            "env": {"MODE": "test"},
            "workspace": "/workspace",
            "capabilities": capsem_core::container::seccomp::WORKLOAD_CAPABILITIES,
            "seccomp": capsem_core::container::seccomp::workload_seccomp(
                capsem_core::container::stage::oci_architecture().unwrap(),
                capsem_core::container::seccomp::Surface::Terminal,
            )
            .unwrap(),
            "id_map": {"containerID": 0, "hostID": 100000, "size": 65536},
            // The fixture VM's 2048 MiB and 2 CPUs, minus the runtime's share.
            "resources": {"memory_bytes": 1664u64 * 1024 * 1024, "cpu_millis": 1750, "pids": 4096},
            "surface": "terminal",
        })
    );
    assert_eq!(
        catalog_reads.load(Ordering::Relaxed),
        0,
        "the explicit grants decided source and admission; the catalog is never read"
    );
}

#[tokio::test]
async fn failed_pull_reports_failed_and_never_touches_the_vm() {
    let fx = fixture(FixtureImages { fail: true, ..images() });
    let owner = owner_admitting_pull(&fx.uds_path);
    start(&fx.state, "box".into(), spec(None));
    let status = wait_for(&fx.state, "box", |s| s.state == ContainerState::Failed).await;
    owner.await.unwrap();
    assert!(
        status.error.as_deref().unwrap().contains("registry refused the image"),
        "{status:?}"
    );
    assert!(!fx.workspace.join(".capsem-image").exists());
}

#[tokio::test]
async fn cancel_during_pull_forgets_the_workload_and_a_late_pull_changes_nothing() {
    let gate = Arc::new(Notify::new());
    let fx = fixture(FixtureImages {
        gate: Some(Arc::clone(&gate)),
        ..images()
    });
    let owner = owner_admitting_pull(&fx.uds_path);
    start(&fx.state, "box".into(), spec(None));
    owner.await.unwrap();
    wait_for(&fx.state, "box", |s| s.state == ContainerState::Pulling).await;

    fx.state.containers.cancel("box");
    gate.notify_waiters();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(fx.state.containers.status("box").is_none());
    assert!(
        !fx.workspace.join(".capsem-image").exists(),
        "a cancelled setup must not stage"
    );
}

#[tokio::test]
async fn registry_access_reaches_only_the_image_source_and_never_the_status() {
    let access = Arc::new(Mutex::new(None));
    let fx = fixture(FixtureImages {
        fail: true,
        access: Arc::clone(&access),
        ..images()
    });
    let secret = RegistryAccess {
        username: Some("robot".into()),
        password: Some("registry-password".into()),
        ca_pem: None,
    };
    let owner = owner_admitting_pull(&fx.uds_path);
    start(&fx.state, "box".into(), spec(Some(secret.clone())));
    let status = wait_for(&fx.state, "box", |s| s.state == ContainerState::Failed).await;
    owner.await.unwrap();
    assert_eq!(access.lock().unwrap().as_ref(), Some(&secret));
    let rendered = serde_json::to_string(&status).unwrap();
    assert!(
        !rendered.contains("registry-password") && !rendered.contains("robot"),
        "{rendered}"
    );
}

/// Every byte the service sends toward the session ledger during a
/// successful setup -- the import rows for each staged file and the launch --
/// is free of the registry credentials the pull used.
#[tokio::test]
async fn registry_credentials_never_reach_the_owner_or_the_staged_workload() {
    let access = Arc::new(Mutex::new(None));
    let fx = fixture(FixtureImages {
        access: Arc::clone(&access),
        ..images()
    });
    let owner = owner_accepting_stage_and_launch(&fx.uds_path, 4);
    let secret = RegistryAccess {
        username: Some("robot-user".into()),
        password: Some("registry-password".into()),
        ca_pem: Some("-----BEGIN CERTIFICATE-----private-ca".into()),
    };
    start(&fx.state, "box".into(), spec(Some(secret.clone())));
    wait_for(&fx.state, "box", |s| s.state == ContainerState::Starting).await;
    let messages = owner.await.unwrap();
    assert_eq!(
        access.lock().unwrap().as_ref(),
        Some(&secret),
        "the pull still had them"
    );
    let sent = format!("{messages:?}");
    let mut staged = String::new();
    for entry in walk(&fx.workspace) {
        staged.push_str(&String::from_utf8_lossy(&std::fs::read(entry).unwrap()));
    }
    assert!(
        staged.contains("serve") && sent.contains("LogFileBoundary"),
        "the setup staged and logged its files"
    );
    for leak in ["registry-password", "robot-user", "private-ca"] {
        assert!(!sent.contains(leak), "{leak} reached the owner: {sent}");
        assert!(!staged.contains(leak), "{leak} was staged into the VM");
    }
}

fn walk(dir: &StdPath) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}

#[tokio::test]
async fn cancel_during_staging_never_launches_the_workload() {
    let fx = fixture(images());
    let state = Arc::clone(&fx.state);
    // The owner cancels the setup while acknowledging the first staged file,
    // the way a delete racing the staging loop would.
    let owner = spawn_fake_process(&fx.uds_path, 2, move |message| {
        let reply = match message {
            ServiceToProcess::AdmitContainerPull { id, .. } => Some(ProcessToService::ContainerPullAdmission {
                id: *id,
                error: None,
                policy_refused: false,
            }),
            ServiceToProcess::LogFileBoundary { id, .. } => {
                state.containers.cancel("box");
                Some(ProcessToService::LogFileBoundaryResult {
                    id: *id,
                    success: true,
                    data: None,
                    error: None,
                })
            }
            other => panic!("a cancelled setup must not send {other:?}"),
        };
        Box::pin(async move { reply })
    });
    start(&fx.state, "box".into(), spec(None));
    owner.await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(fx.state.containers.status("box").is_none());
    assert!(
        !fx.workspace.join(".capsem-image/launch.py").exists(),
        "staging must stop at the cancellation"
    );
}

async fn get_status(state: &Arc<ServiceState>, id: &str) -> (StatusCode, serde_json::Value) {
    crate::tests::route_request(
        build_service_router(Arc::clone(state)),
        axum::http::Method::GET,
        &format!("/vms/{id}/container"),
        None,
    )
    .await
}

#[tokio::test]
async fn container_status_route_reports_no_workload_as_not_found() {
    let fx = fixture(images());
    let (status, body) = get_status(&fx.state, "box").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn container_status_route_reports_running_only_once_the_guest_marks_it_running() {
    let fx = fixture(images());
    let owner = owner_accepting_stage_and_launch(&fx.uds_path, 4);
    start(&fx.state, "box".into(), spec(None));
    owner.await.unwrap();
    wait_for(&fx.state, "box", |s| s.state == ContainerState::Starting).await;
    let (_, body) = get_status(&fx.state, "box").await;
    assert_eq!(body["state"], "starting");
    assert_eq!(body["image"], "registry.example/app:1");

    mark(&fx, "ready");
    let (_, body) = get_status(&fx.state, "box").await;
    assert_eq!(body["state"], "starting", "staged is not running");

    mark(&fx, "running");
    let (status, body) = get_status(&fx.state, "box").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], "running");
    assert_eq!(
        body["digest"],
        "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
    );
}

/// A workload that ran and ended is `exited`, with its code -- not `running`
/// forever. The launcher writes the code into the stage when runc returns.
#[tokio::test]
async fn container_status_reports_a_workload_that_ran_and_ended_as_exited_with_its_code() {
    let fx = fixture(images());
    let owner = owner_accepting_stage_and_launch(&fx.uds_path, 4);
    start(&fx.state, "box".into(), spec(None));
    owner.await.unwrap();
    wait_for(&fx.state, "box", |s| s.state == ContainerState::Starting).await;
    mark(&fx, "ready");
    mark(&fx, "running");
    let (_, body) = get_status(&fx.state, "box").await;
    assert_eq!(body["state"], "running");

    std::fs::write(fx.workspace.join(".capsem-image").join("exited"), b"3\n").unwrap();
    let (status, body) = get_status(&fx.state, "box").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "exited", "{body}");
    assert_eq!(body["exit_code"], 3, "{body}");
}

#[tokio::test]
async fn container_status_survives_a_service_restart_through_the_launch_record() {
    let fx = fixture(images());
    let owner = owner_accepting_stage_and_launch(&fx.uds_path, 4);
    start(&fx.state, "box".into(), spec(None));
    owner.await.unwrap();
    wait_for(&fx.state, "box", |s| s.state == ContainerState::Starting).await;
    for _ in 0..100 {
        let session = fx.state.instances.lock().unwrap()["box"].session_dir.clone();
        if session.join(LAUNCH_RECORD).exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    // A restarted service has no live record, only the session directory.
    fx.state.containers.cancel("box");
    mark(&fx, "ready");
    mark(&fx, "running");
    let (status, body) = get_status(&fx.state, "box").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "running");
    assert_eq!(body["image"], "registry.example/app:1");
    // A restarted service still knows the pin, never only the moving tag.
    assert_eq!(
        body["resolved"],
        "registry.example/app@sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
    );
}

/// A registry outside the policy's sources is refused before anything else:
/// no owner is asked and the registry is never contacted.
#[tokio::test]
async fn an_ungranted_source_is_refused_before_any_registry_access() {
    let images = images();
    let access = Arc::clone(&images.access);
    let catalog_reads = Arc::clone(&images.catalog_reads);
    let fx = fixture(images);
    start(
        &fx.state,
        "box".into(),
        ContainerSpec {
            image: "registry.example.evil/app:1".into(),
            ..spec(None)
        },
    );
    let status = wait_for(&fx.state, "box", |s| s.state == ContainerState::Failed).await;
    assert!(status.error.as_deref().unwrap().contains("not allowed"), "{status:?}");
    assert!(access.lock().unwrap().is_none(), "the registry was contacted");
    // The grants did not decide, so the catalog was asked -- and, unreadable,
    // widened nothing.
    assert_eq!(catalog_reads.load(Ordering::Relaxed), 1);
}

/// Admission is on the resolved digest: an image granted by digest runs only
/// at that digest, whichever of its two addresses matches.
#[test]
fn admission_takes_either_content_address_and_nothing_else() {
    let a = format!("sha256:{}", "a".repeat(64));
    let b = format!("sha256:{}", "b".repeat(64));
    let c = format!("sha256:{}", "c".repeat(64));
    let granted = capsem_core::net::policy_config::SettingsFile {
        images: Some(capsem_core::net::policy_config::ImagePolicyConfig {
            sources: vec!["registry.example".into()],
            admit: vec![format!("registry.example/app@{a}")],
            ..Default::default()
        }),
        ..Default::default()
    };
    let policy = capsem_core::container::admission::ImagePolicy::from_files(&granted, &Default::default()).unwrap();
    let requested: capsem_assets::oci::ImageReference = "registry.example/app:1".parse().unwrap();
    let pulled = |image_digest: &str, digest: &str| PulledImage {
        root: PathBuf::new(),
        files: Vec::new(),
        digest: digest.into(),
        image_digest: image_digest.into(),
        cache_key: None,
        _hold: Box::new(()),
    };
    admit(&policy, &requested, &pulled(&a, &b)).unwrap();
    admit(&policy, &requested, &pulled(&b, &a)).unwrap();
    let refused = admit(&policy, &requested, &pulled(&b, &c)).unwrap_err();
    assert!(refused.contains("not admitted"), "{refused}");
}
