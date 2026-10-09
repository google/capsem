use super::*;
use crate::container_setup::{ContainerSetups, ImageFetch, ImageSource, PullFuture};
use crate::tests::insert_fake_instance_with_session_dir;

/// Re-executed owner stays alive until the create's real teardown kills it.
#[tokio::test]
async fn refused_image_owner_child() {
    let Some(path) = std::env::var_os("CAPSEM_REFUSED_IMAGE_OWNER") else {
        return;
    };
    use std::io::Write;
    use std::os::fd::AsFd;
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    std::fs::write(PathBuf::from(&path).with_extension("ready"), b"ready").unwrap();
    let mut control =
        std::os::unix::net::UnixStream::from(capsem_foundation::unix::fd::duplicate(std::io::stdin().as_fd()).unwrap());
    control.write_all(b"ready").unwrap();
    let (socket, _) = listener.accept().await.unwrap();
    let mut socket = socket.into_std().unwrap();
    let socket = tokio::task::spawn_blocking(move || {
        capsem_foundation::ipc_handshake::negotiate_responder(&mut socket, "capsem-process-test", "").unwrap();
        socket
    })
    .await
    .unwrap();
    let (tx, rx) = channel_from_std::<ProcessToService, ServiceToProcess>(socket).unwrap();
    let ServiceToProcess::AdmitContainerPull { id, .. } = rx.recv().await.unwrap() else {
        panic!("expected admission")
    };
    tx.send(ProcessToService::ContainerPullAdmission {
        id,
        error: None,
        policy_refused: false,
    })
    .await
    .unwrap();
    std::future::pending::<()>().await;
}

/// An image source whose registry refuses every pull. Its policy grants that
/// registry, so the refusal is the registry's: the default policy reads the
/// developer's own settings.toml, and without one it refused the source
/// first and the test never reached the pull it is about.
struct RefusingImages;

impl ImageSource for RefusingImages {
    fn policy(&self) -> crate::container_setup::PolicyFuture {
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

    fn pull(&self, _image: String, _access: api::RegistryAccess, _parent: PathBuf, _mode: ImageFetch) -> PullFuture {
        Box::pin(async { anyhow::bail!("registry refused the image") })
    }

    fn fetch_catalog(
        &self,
        _source: capsem_core::container::admission::CatalogSource,
        _parent: PathBuf,
    ) -> crate::container_setup::images::CatalogFuture {
        Box::pin(async { anyhow::bail!("no catalog in this fixture") })
    }
}

#[tokio::test]
async fn persistent_boot_crash_reads_the_log_before_the_reaper_caches_its_tail() {
    let state = Arc::new(crate::tests::make_test_state_owned());
    let session_dir = state.run_dir.join("persistent").join("named-box");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("process.log"), b"ledger confinement failed\n").unwrap();
    let mut entry = crate::tests::test_persistent_entry("named-box", session_dir);
    entry.id = "box".into();
    state.persistent_registry.lock().unwrap().register(entry).unwrap();

    let tail = failed_process_log_tail(&state, "box").await;

    assert_eq!(tail, "ledger confinement failed");
}

/// A create whose container fails after the VM is registered used to answer
/// 500 and leave the VM running: the caller never learned its id, and a named
/// VM kept its name, so the retry got 409. The failed create is discarded --
/// but not its ledger and logs: deleting them erased the security record of
/// the very refusal that failed it (a policy-blocked pull), so the session is
/// kept for post-mortem the way any failed session is.
#[tokio::test]
async fn a_failed_container_create_discards_the_vm_and_keeps_its_ledger() {
    let mut state = crate::tests::make_test_state_owned();
    state.containers = ContainerSetups::with_source(Box::new(RefusingImages));
    let state = Arc::new(state);
    let session_dir = state.run_dir.join("persistent").join("box");
    std::fs::create_dir_all(session_dir.join("guest/workspace")).unwrap();
    // Not SQLite: its counters cannot be read at stop, and the ledger must be kept anyway.
    std::fs::write(session_dir.join("session.db"), b"ledger").unwrap();
    std::fs::write(session_dir.join("process.log"), b"log").unwrap();
    use tokio::io::AsyncReadExt;
    let uds_path = session_dir.join("process.sock");
    let (control, child_control) = std::os::unix::net::UnixStream::pair().unwrap();
    control.set_nonblocking(true).unwrap();
    let mut control = tokio::net::UnixStream::from_std(control).unwrap();
    let child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "vm_files::provision::tests::refused_image_owner_child",
            "--nocapture",
        ])
        .env_clear()
        .env("CAPSEM_REFUSED_IMAGE_OWNER", &uds_path)
        .stdin(std::process::Stdio::from(std::os::fd::OwnedFd::from(child_control)))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut ready = [0; 5];
    tokio::time::timeout(std::time::Duration::from_secs(5), control.read_exact(&mut ready))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&ready, b"ready");
    insert_fake_instance_with_session_dir(&state, "box", child.id().unwrap(), session_dir.clone());
    let generation = state.instances.lock().unwrap()["box"].generation;
    let retirement = state.retirements.register("box", generation).unwrap();
    let reaper = crate::instance_reaper::spawn_exit_reaper(
        child,
        "box".into(),
        "named-box".into(),
        Arc::clone(&state),
        uds_path,
        session_dir.clone(),
        retirement,
    );
    let mut entry = crate::tests::test_persistent_entry("named-box", session_dir.clone());
    entry.id = "box".into();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert("named-box".into(), entry);
    let spec = api::ContainerSpec {
        image: "registry.example/app:1".into(),
        args: vec![],
        env: Default::default(),
        registry: None,
        attach: false,
    };

    let error = finish_create(&state, "box", &[], Some(spec)).await.unwrap_err();
    reaper.await.unwrap();
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
}
