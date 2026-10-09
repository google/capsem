use super::*;
use crate::tests::{insert_fake_instance_with_session_dir, make_test_state};

fn fixture() -> (Arc<ServiceState>, OwnerConnection) {
    let state = make_test_state();
    let session = state.run_dir.join("sessions/owner");
    std::fs::create_dir_all(&session).unwrap();
    insert_fake_instance_with_session_dir(&state, "owner", std::process::id(), session);
    let grant = OwnerConnection::capture(&state.instances.lock().unwrap()["owner"]).unwrap();
    (state, grant)
}

#[tokio::test]
async fn replaced_generation_is_rejected_before_connect() {
    let (state, grant) = fixture();
    let listener = tokio::net::UnixListener::bind(&grant.uds_path).unwrap();
    state.instances.lock().unwrap().get_mut("owner").unwrap().generation = uuid::Uuid::new_v4();
    assert!(grant.open(&state, "capsem-service", false).await.is_err());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn generation_changed_during_hello_cannot_receive_a_request() {
    use std::os::fd::AsFd;
    let (state, grant) = fixture();
    let listener = tokio::net::UnixListener::bind(&grant.uds_path).unwrap();
    let peer_state = Arc::clone(&state);
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = socket.into_std().unwrap();
        tokio::task::spawn_blocking(move || {
            assert!(
                capsem_foundation::unix::fd::wait_readable(socket.as_fd(), std::time::Duration::from_secs(5)).unwrap()
            );
            peer_state
                .instances
                .lock()
                .unwrap()
                .get_mut("owner")
                .unwrap()
                .generation = uuid::Uuid::new_v4();
            capsem_foundation::ipc_handshake::negotiate_responder(&mut socket, "capsem-process", "").unwrap();
            socket
        })
        .await
        .unwrap()
    });
    assert!(grant.open(&state, "capsem-service", false).await.is_err());
    let mut socket = server.await.unwrap();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        socket.set_nonblocking(false).unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        assert_eq!(socket.read(&mut [0; 1]).unwrap(), 0);
    })
    .await
    .unwrap();
}

#[test]
fn shutdown_grant_accepts_only_the_claimed_generation_or_an_empty_slot() {
    let (state, grant) = fixture();
    let mut instance = state.instances.lock().unwrap().remove("owner").unwrap();
    assert!(grant.validate(&state, false).is_err());
    assert!(grant.validate(&state, true).is_ok());
    instance.generation = uuid::Uuid::new_v4();
    state.instances.lock().unwrap().insert("owner".into(), instance);
    assert!(grant.validate(&state, true).is_err());
}

#[tokio::test]
async fn suspending_owner_child() {
    let Some(path) = std::env::var_os("CAPSEM_SUSPENDING_OWNER") else {
        return;
    };
    use std::io::Write;
    use std::os::fd::AsFd;
    let listener = tokio::net::UnixListener::bind(path).unwrap();
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
    let ServiceToProcess::Suspend = rx.recv().await.unwrap() else {
        panic!("expected suspend")
    };
    tx.send(ProcessToService::StateChanged {
        id: "owner".into(),
        state: "Suspended".into(),
        trigger: "test".into(),
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn authenticated_suspend_accepts_its_original_childs_confirmation_and_exit() {
    use tokio::io::AsyncReadExt;
    let (state, owner) = fixture();
    let (control, child_control) = std::os::unix::net::UnixStream::pair().unwrap();
    control.set_nonblocking(true).unwrap();
    let mut control = tokio::net::UnixStream::from_std(control).unwrap();
    let child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "owner_connection::tests::suspending_owner_child",
            "--nocapture",
        ])
        .env_clear()
        .env("CAPSEM_SUSPENDING_OWNER", &owner.uds_path)
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
    let (generation, session_dir) = {
        let mut instances = state.instances.lock().unwrap();
        let instance = instances.get_mut("owner").unwrap();
        instance.pid = child.id().unwrap();
        instance.persistent = true;
        let captured = (instance.generation, instance.session_dir.clone());
        drop(instances);
        captured
    };
    let retirement = state.retirements.register("owner", generation).unwrap();
    let reaper = crate::instance_reaper::spawn_exit_reaper(
        child,
        "owner".into(),
        "owner".into(),
        Arc::clone(&state),
        owner.uds_path,
        session_dir,
        retirement,
    );
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        handle_suspend(State(Arc::clone(&state)), Path("owner".into())),
    )
    .await
    .unwrap();
    reaper.await.unwrap();
    assert!(result.unwrap().0.success);
    assert!(!state.instances.lock().unwrap().contains_key("owner"));
}
