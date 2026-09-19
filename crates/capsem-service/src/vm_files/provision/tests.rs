use super::*;
use crate::container_setup::{ContainerSetups, ImageSource, PullFuture};
use crate::tests::{insert_fake_instance_with_session_dir, spawn_fake_process};

/// An image source that always refuses, as a registry or policy would.
struct RefusingImages;

impl ImageSource for RefusingImages {
    fn pull(&self, _image: String, _access: api::RegistryAccess, _parent: PathBuf) -> PullFuture {
        Box::pin(async { anyhow::bail!("registry refused the image") })
    }
}

/// A create whose container fails after the VM is registered used to answer
/// 500 and leave the VM running: the caller never learned its id, and a named
/// VM kept its name, so the retry got 409. The failed create is discarded --
/// but not its ledger and logs: deleting them erased the security record of
/// the very refusal that failed it (a policy-blocked pull), so the session is
/// kept for post-mortem the way any failed session is.
#[tokio::test]
async fn a_failed_container_create_discards_the_vm_and_keeps_its_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = crate::tests::make_test_state_owned();
    state.containers = ContainerSetups::with_source(Box::new(RefusingImages));
    let state = Arc::new(state);
    let session_dir = state.run_dir.join("persistent").join("box");
    std::fs::create_dir_all(session_dir.join("guest/workspace")).unwrap();
    // Not SQLite: the rollup into main.db fails, and the ledger must be kept anyway.
    std::fs::write(session_dir.join("session.db"), b"ledger").unwrap();
    std::fs::write(session_dir.join("process.log"), b"log").unwrap();
    // pid 0: teardown must not signal a real process.
    insert_fake_instance_with_session_dir(&state, "box", 0, session_dir.clone());
    let mut entry = crate::tests::test_persistent_entry("named-box", session_dir.clone());
    entry.id = "box".into();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert("named-box".into(), entry);
    let uds_path = state.instances.lock().unwrap()["box"].uds_path.clone();
    let owner = spawn_fake_process(&uds_path, 1, |message| {
        let reply = match message {
            ServiceToProcess::AdmitContainerPull { id, .. } => Some(ProcessToService::ContainerPullAdmission {
                id: *id,
                error: None,
                policy_refused: false,
            }),
            other => panic!("unexpected owner message: {other:?}"),
        };
        Box::pin(async move { reply })
    });
    let spec = api::ContainerSpec {
        image: "registry.example/app:1".into(),
        args: vec![],
        env: Default::default(),
        registry: None,
        attach: false,
    };

    let error = finish_create(&state, "box", &[], Some(spec)).await.unwrap_err();
    owner.await.unwrap();
    assert_eq!(error.0, StatusCode::INTERNAL_SERVER_ERROR, "{}", error.1);
    assert!(error.1.contains("registry refused the image"), "{}", error.1);
    assert!(
        !state.instances.lock().unwrap().contains_key("box"),
        "the VM stays registered"
    );
    assert!(
        !state
            .persistent_registry
            .lock()
            .unwrap()
            .data
            .vms
            .contains_key("named-box"),
        "the name stays taken"
    );
    assert!(!session_dir.exists(), "the failed VM keeps its live session dir");
    let kept = find_failed_session_dir(&state.run_dir, "box").expect("the failed create's ledger and logs are kept");
    assert_eq!(std::fs::read(kept.join("session.db")).unwrap(), b"ledger");
    assert_eq!(std::fs::read(kept.join("process.log")).unwrap(), b"log");
    drop(dir);
}
