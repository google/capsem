//! Preparation is pending; only success can settle readiness.
use super::*;

#[tokio::test]
async fn startup_marker_is_preparing_and_failure_detail_is_bounded() {
    let fx = fixture(images());
    let generation = fx.state.containers.begin("box", "registry.example/app:1");
    fx.state
        .containers
        .advance("box", generation, |status| status.state = ContainerState::Starting);
    std::fs::create_dir_all(fx.workspace.join(".capsem-image")).unwrap();
    mark(&fx, "preparing");
    let status = observe(&fx.state, "box", fx.state.containers.status("box"))
        .unwrap()
        .unwrap();
    assert_eq!(status.state, ContainerState::Preparing);
    assert!(wait_observed(
        &fx.state,
        "box",
        capsem_foundation::poll::PollOpts::new("preparation-remains-pending", std::time::Duration::from_millis(30))
    )
    .await
    .is_err());
    let failure = fx.workspace.join(".capsem-image/failed");
    std::fs::write(&failure, b"startup exit (17)\nrequired setup refused").unwrap();
    let status = observe(&fx.state, "box", fx.state.containers.status("box"))
        .unwrap()
        .unwrap();
    assert_eq!(status.state, ContainerState::Failed);
    assert_eq!(
        status.error.as_deref(),
        Some("startup exit (17)\nrequired setup refused")
    );
    std::fs::write(&failure, vec![b'x'; 65537]).unwrap();
    let status = observe(&fx.state, "box", fx.state.containers.status("box"))
        .unwrap()
        .unwrap();
    assert_eq!(status.state, ContainerState::Failed);
    assert!(
        status.error.unwrap().len() < 1024,
        "untrusted oversized output must not reach the API"
    );
}
