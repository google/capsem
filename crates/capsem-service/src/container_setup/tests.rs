use super::*;
use crate::tests::{insert_fake_instance_with_session_dir, spawn_fake_process};
use capsem_api::RegistryAccess;
use tokio::sync::Notify;

/// An image source serving a fixed two-file layout, optionally held at the
/// pull until released, and recording the registry access it was given.
struct FixtureImages {
    fail: bool,
    gate: Option<Arc<Notify>>,
    access: Arc<Mutex<Option<RegistryAccess>>>,
}

impl ImageSource for FixtureImages {
    fn pull(&self, _image: String, access: RegistryAccess, _parent: PathBuf) -> PullFuture {
        let (fail, gate, seen) = (self.fail, self.gate.clone(), Arc::clone(&self.access));
        Box::pin(async move {
            *seen.lock().unwrap() = Some(access);
            if let Some(gate) = gate {
                gate.notified().await;
            }
            anyhow::ensure!(!fail, "registry refused the image");
            let root = tempfile::tempdir()?;
            std::fs::write(root.path().join("index.json"), b"{\"manifests\":[]}")?;
            std::fs::write(root.path().join("oci-layout"), b"")?;
            Ok(PulledImage {
                root: root.path().to_path_buf(),
                files: vec![PathBuf::from("index.json"), PathBuf::from("oci-layout")],
                digest: "sha256:fixture".into(),
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
    insert_fake_instance_with_session_dir(&state, "box", 1, session_dir);
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
async fn pull_admission_waits_for_the_vm_owner_readiness_barrier() {
    let access = Arc::new(Mutex::new(None));
    let fx = fixture(FixtureImages {
        access: Arc::clone(&access),
        ..images()
    });
    let ready_path = fx.uds_path.with_extension("ready");
    std::fs::remove_file(&ready_path).unwrap();
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

    start(&fx.state, "box".into(), spec(None));
    tokio::time::sleep(std::time::Duration::from_millis(75)).await;
    assert!(
        !owner.is_finished(),
        "admission reached the owner before its ready sentinel"
    );
    assert!(
        access.lock().unwrap().is_none(),
        "the registry was contacted before owner readiness"
    );

    std::fs::write(&ready_path, b"1\n").unwrap();
    let status = wait_for(&fx.state, "box", |s| s.state == ContainerState::Failed).await;
    owner.await.unwrap();
    assert!(status.error.as_deref().unwrap().contains("blocked after readiness"));
    assert!(access.lock().unwrap().is_none());
}

fn images() -> FixtureImages {
    FixtureImages {
        fail: false,
        gate: None,
        access: Arc::new(Mutex::new(None)),
    }
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
    let status = fx
        .state
        .containers
        .wait_with_options(
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

#[tokio::test]
async fn create_wait_returns_terminal_failure_without_retrying_setup() {
    let fx = fixture(images());
    let generation = fx.state.containers.begin("box", "registry.example/app:1");
    fx.state.containers.advance("box", generation, |status| {
        status.state = ContainerState::Failed;
        status.error = Some("pull refused".into());
    });
    let status = fx
        .state
        .containers
        .wait_with_options(
            "box",
            capsem_foundation::poll::PollOpts::new("container-create-test", std::time::Duration::from_secs(1)),
        )
        .await
        .unwrap();
    assert_eq!(status.state, ContainerState::Failed);
    assert_eq!(status.error.as_deref(), Some("pull refused"));
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
    let access = Arc::new(Mutex::new(None));
    let fx = fixture(FixtureImages {
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
    let fx = fixture(images());
    // Pull admission, index.json part, transfer.json, options.json, launch.py,
    // then the launch exec.
    let owner = owner_accepting_stage_and_launch(&fx.uds_path, 6);
    start(&fx.state, "box".into(), spec(None));

    let status = wait_for(&fx.state, "box", |s| s.state == ContainerState::Starting).await;
    let messages = owner.await.unwrap();
    assert_eq!(status.digest.as_deref(), Some("sha256:fixture"));
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
    assert_eq!(
        staged,
        [
            ".capsem-image/0-0",
            ".capsem-image/transfer.json",
            ".capsem-image/options.json",
            ".capsem-image/launch.py"
        ]
    );
    match messages.last() {
        Some(ServiceToProcess::Exec { command, .. }) => {
            assert_eq!(command, &capsem_core::container::detached_launch_command())
        }
        other => panic!("setup must end with the detached launch, got {other:?}"),
    }
    let stage = fx.workspace.join(".capsem-image");
    assert_eq!(std::fs::read(stage.join("0-0")).unwrap(), b"{\"manifests\":[]}");
    let options: serde_json::Value =
        serde_json::from_slice(&std::fs::read(stage.join("options.json")).unwrap()).unwrap();
    assert_eq!(options, json!({"args": ["serve"], "env": {"MODE": "test"}}));
    assert!(!stage.join("1-0").exists(), "an empty layout file has no part");
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
    let owner = owner_accepting_stage_and_launch(&fx.uds_path, 6);
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
        !fx.workspace.join(".capsem-image/transfer.json").exists(),
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
async fn container_status_route_reports_running_only_once_the_guest_marks_ready() {
    let fx = fixture(images());
    let owner = owner_accepting_stage_and_launch(&fx.uds_path, 6);
    start(&fx.state, "box".into(), spec(None));
    owner.await.unwrap();
    wait_for(&fx.state, "box", |s| s.state == ContainerState::Starting).await;
    let (_, body) = get_status(&fx.state, "box").await;
    assert_eq!(body["state"], "starting");
    assert_eq!(body["image"], "registry.example/app:1");

    std::fs::write(fx.workspace.join(".capsem-image/ready"), b"1\n").unwrap();
    let (status, body) = get_status(&fx.state, "box").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], "running");
    assert_eq!(body["digest"], "sha256:fixture");
}

#[tokio::test]
async fn container_status_survives_a_service_restart_through_the_launch_record() {
    let fx = fixture(images());
    let owner = owner_accepting_stage_and_launch(&fx.uds_path, 6);
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
    std::fs::write(fx.workspace.join(".capsem-image/ready"), b"1\n").unwrap();
    let (status, body) = get_status(&fx.state, "box").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "running");
    assert_eq!(body["image"], "registry.example/app:1");
}
