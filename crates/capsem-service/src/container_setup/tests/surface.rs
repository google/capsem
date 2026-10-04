//! An image's declared GUI surface becomes exactly one browser-preview
//! exposure, through the ordinary exposure path, once the workload runs.

use super::*;
use capsem_proto::{PublicationAccess, PublicationTarget};

const EXPOSURE: &str = "0199df26-d0f2-74f2-a304-ef67b79d1217";
const PREVIEW_LISTENER: u16 = 19444;

fn xpra_on(port: &str) -> FixtureImages {
    FixtureImages {
        labels: Some(json!({"org.capsem.surface": "xpra", "org.capsem.surface.port": port})),
        ..images()
    }
}

fn gui_fixture(images: FixtureImages) -> Fixture {
    let fx = fixture(images);
    // The gateway's preview listener, which an http_preview exposure names.
    std::fs::write(fx.state.run_dir.join("preview.port"), PREVIEW_LISTENER.to_string()).unwrap();
    fx
}

fn is_exposure(message: &ServiceToProcess) -> bool {
    matches!(
        message,
        ServiceToProcess::DeclarePreview { .. } | ServiceToProcess::PublishPort { .. }
    )
}

/// The six setup messages, then one exposure request, answered as the VM's
/// security engine would: published, or refused by policy.
fn owner_answering_the_surface(uds_path: &StdPath, refuse: bool) -> tokio::task::JoinHandle<Vec<ServiceToProcess>> {
    spawn_fake_process(uds_path, 7, move |message| {
        let reply = match message {
            ServiceToProcess::AdmitContainerPull { id, .. } => ProcessToService::ContainerPullAdmission {
                id: *id,
                error: None,
                policy_refused: false,
            },
            ServiceToProcess::LogFileBoundary { id, .. } => ProcessToService::LogFileBoundaryResult {
                id: *id,
                success: true,
                data: None,
                error: None,
            },
            ServiceToProcess::Exec { id, .. } => ProcessToService::ExecResult {
                id: *id,
                stdout: vec![],
                stderr: vec![],
                exit_code: 0,
                truncated: false,
            },
            ServiceToProcess::DeclarePreview { id, .. } if refuse => ProcessToService::PortPublished {
                id: *id,
                publication: None,
                error: Some("exposure denied by default.http_preview".into()),
                policy_refused: true,
            },
            ServiceToProcess::DeclarePreview {
                id, guest_port, target, ..
            } => ProcessToService::PortPublished {
                id: *id,
                publication: Some(capsem_proto::ipc::PublicationInfo {
                    id: EXPOSURE.into(),
                    host_port: None,
                    guest_port: *guest_port,
                    target: *target,
                    access: PublicationAccess::HttpPreview,
                    router_pid: 4242,
                }),
                error: None,
                policy_refused: false,
            },
            other => panic!("unexpected IPC message during a GUI setup: {other:?}"),
        };
        Box::pin(async move { Some(reply) })
    })
}

fn mark_running(fx: &Fixture) {
    std::fs::write(fx.workspace.join(".capsem-image/ready"), b"1\n").unwrap();
}

async fn setup_task_finished(fx: &Fixture) {
    for _ in 0..500 {
        let finished = fx.state.containers.records.lock().unwrap()["box"]
            .task
            .as_ref()
            .is_some_and(tokio::task::AbortHandle::is_finished);
        if finished {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the container setup task never finished");
}

#[tokio::test]
async fn an_xpra_image_is_granted_exactly_one_preview_exposure_once_it_runs() {
    let fx = gui_fixture(xpra_on("14500"));
    let owner = owner_answering_the_surface(&fx.uds_path, false);
    start(&fx.state, "box".into(), spec(None));
    let starting = wait_for(&fx.state, "box", |s| s.state == ContainerState::Starting).await;
    assert_eq!(
        starting.surface,
        Some(ContainerSurface {
            kind: ContainerSurfaceKind::Xpra,
            port: 14500,
            exposure_id: None,
        })
    );
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(
        !owner.is_finished(),
        "the surface was exposed before the workload reported running"
    );

    mark_running(&fx);
    let messages = owner.await.unwrap();
    let exposures: Vec<_> = messages.iter().filter(|message| is_exposure(message)).collect();
    assert_eq!(exposures.len(), 1, "{exposures:?}");
    match exposures[0] {
        ServiceToProcess::DeclarePreview {
            listener_port,
            guest_port,
            target,
            ..
        } => {
            assert_eq!(*guest_port, 14500, "only the labelled port");
            assert_eq!(*target, PublicationTarget::Container, "the container's loopback");
            assert_eq!(
                *listener_port, PREVIEW_LISTENER,
                "the gateway's preview origin, no host port"
            );
        }
        other => panic!("a surface is a preview, never a host TCP listener: {other:?}"),
    }
    assert!(
        matches!(messages.last(), Some(ServiceToProcess::DeclarePreview { .. })),
        "the exposure follows the launch"
    );

    wait_for(&fx.state, "box", |s| {
        s.surface.as_ref().is_some_and(|surface| surface.exposure_id.is_some())
    })
    .await;
    let (status, body) = get_status(&fx.state, "box").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "running");
    assert_eq!(
        body["surface"],
        json!({"kind": "xpra", "port": 14500, "exposure_id": EXPOSURE})
    );

    // The exposure lives in the VM owner, which outlives a service restart;
    // the launch record keeps the status pointing at it.
    setup_task_finished(&fx).await;
    fx.state.containers.cancel("box");
    let (_, body) = get_status(&fx.state, "box").await;
    assert_eq!(body["surface"]["exposure_id"], EXPOSURE, "{body}");
}

#[tokio::test]
async fn a_terminal_image_is_granted_no_exposure() {
    let fx = gui_fixture(images());
    let owner = owner_accepting_stage_and_launch(&fx.uds_path, 6);
    start(&fx.state, "box".into(), spec(None));
    let messages = owner.await.unwrap();
    // The setup ends at the launch: nothing waits to expose anything.
    setup_task_finished(&fx).await;
    mark_running(&fx);
    let (_, body) = get_status(&fx.state, "box").await;
    assert_eq!(body["state"], "running");
    assert!(body.get("surface").is_none(), "{body}");
    assert!(!messages.iter().any(is_exposure), "{messages:?}");
}

#[tokio::test]
async fn a_bad_surface_port_label_refuses_the_image_and_exposes_nothing() {
    for port in ["0", "65536", "14500,14501", "+14500", ""] {
        let fx = gui_fixture(xpra_on(port));
        let owner = owner_admitting_pull(&fx.uds_path);
        start(&fx.state, "box".into(), spec(None));
        let status = wait_for(&fx.state, "box", |s| s.state == ContainerState::Failed).await;
        let messages = owner.await.unwrap();
        let error = status.error.as_deref().unwrap();
        assert!(error.contains("org.capsem.surface.port"), "{port:?}: {error}");
        assert!(status.surface.is_none(), "{port:?}");
        assert!(!messages.iter().any(is_exposure), "{port:?}");
        assert!(
            !fx.workspace.join(".capsem-image").exists(),
            "{port:?}: a refused image is never staged"
        );
    }
}

/// The security engine decides: a refused surface leaves the workload
/// running with no exposure, and the status never claims one.
#[tokio::test]
async fn a_policy_refused_surface_leaves_the_workload_running_without_one() {
    let fx = gui_fixture(xpra_on("14500"));
    let owner = owner_answering_the_surface(&fx.uds_path, true);
    start(&fx.state, "box".into(), spec(None));
    wait_for(&fx.state, "box", |s| s.state == ContainerState::Starting).await;
    mark_running(&fx);
    let messages = owner.await.unwrap();
    assert_eq!(messages.iter().filter(|message| is_exposure(message)).count(), 1);
    setup_task_finished(&fx).await;
    let (_, body) = get_status(&fx.state, "box").await;
    assert_eq!(body["state"], "running");
    assert_eq!(body["surface"], json!({"kind": "xpra", "port": 14500}), "{body}");
}
