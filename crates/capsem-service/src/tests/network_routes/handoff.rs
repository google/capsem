use super::*;

#[tokio::test]
async fn network_attach_rejects_a_foreign_handoff_before_connecting() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    insert_fake_instance(&state, "vm-b", std::process::id());
    let uds = state.instances.lock().unwrap()["vm-b"].uds_path.clone();
    std::fs::create_dir_all(uds.parent().unwrap()).unwrap();
    let foreign = capsem_foundation::uds::private_handoff_socket_path(&state.run_dir, "vm-a").unwrap();
    let listener = std::os::unix::net::UnixListener::bind(&foreign).unwrap();
    listener.set_nonblocking(true).unwrap();
    let owner = spawn_fake_process(&uds, 1, move |message| {
        let ServiceToProcess::LinkAttach { id, .. } = message else {
            panic!("unexpected {message:?}")
        };
        let reply = ProcessToService::LinkAttachResult {
            id: *id,
            handoff_socket: foreign.to_string_lossy().into_owned(),
            error: None,
        };
        Box::pin(async move { Some(reply) })
    });
    let (_, created) = create_network(&state, "team").await;
    let network = created["id"].as_str().unwrap();
    let (status, joined) = tokio::time::timeout(
        Duration::from_secs(2),
        route_request(
            app(&state),
            Method::PUT,
            &format!("/networks/{network}/members/vm-b"),
            None,
        ),
    )
    .await
    .expect("a foreign endpoint is refused before waiting for a handoff");
    owner.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{joined}");
    assert_eq!(joined["members"][0]["state"], "failed", "{joined}");
    assert_eq!(listener.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    let (_, logs) = route_request(app(&state), Method::GET, &format!("/networks/{network}/logs"), None).await;
    assert!(
        logs.to_string()
            .contains("handoff endpoint does not belong to this VM owner"),
        "{logs}"
    );
}
