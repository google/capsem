use super::*;
use crate::tests::{insert_fake_instance_with_session_dir, make_test_state};

#[tokio::test]
async fn ephemeral_guest_suspend_is_refused_without_contacting_the_owner() {
    let state = make_test_state();
    let session = state.run_dir.join("sessions/ephemeral-suspend");
    std::fs::create_dir_all(&session).unwrap();
    insert_fake_instance_with_session_dir(&state, "ephemeral-suspend", std::process::id(), session);
    let (generation, uds_path) = {
        let instances = state.instances.lock().unwrap();
        let instance = &instances["ephemeral-suspend"];
        let details = (instance.generation, instance.uds_path.clone());
        drop(instances);
        details
    };
    let listener = tokio::net::UnixListener::bind(&uds_path).unwrap();
    spawn_suspend_listener(Arc::clone(&state), "ephemeral-suspend".into(), generation);
    let (socket, _) = tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let mut socket = socket.into_std().unwrap();
    let socket = tokio::task::spawn_blocking(move || {
        capsem_foundation::ipc_handshake::negotiate_responder(&mut socket, "capsem-process-test", "").unwrap();
        socket
    })
    .await
    .unwrap();
    let (tx, _rx) = channel_from_std::<ProcessToService, ServiceToProcess>(socket).unwrap();
    tx.send(ProcessToService::SuspendRequested {
        id: "ephemeral-suspend".into(),
    })
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(150), listener.accept())
            .await
            .is_err(),
        "ephemeral suspend must not send a suspend command"
    );
    assert!(state.instances.lock().unwrap().contains_key("ephemeral-suspend"));
    state.evict_instance("ephemeral-suspend", generation);
}
