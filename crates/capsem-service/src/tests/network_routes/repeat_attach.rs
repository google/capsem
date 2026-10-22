use super::*;

#[tokio::test]
async fn attaching_an_already_ready_member_keeps_its_live_cable() {
    let (state, _dir) = make_test_state_with_tempdir();
    install_test_runtime_assets(&state);
    insert_fake_instance(&state, "vm-b", std::process::id());
    let uds = state.instances.lock().unwrap()["vm-b"].uds_path.clone();
    std::fs::create_dir_all(uds.parent().unwrap()).unwrap();
    let (_seat, owner) = fake_link_seat(&state, &uds, false, 1, 0);
    let (_, created) = create_network(&state, "team").await;
    let id = created["id"].as_str().unwrap();
    let uri = format!("/networks/{id}/members/vm-b");

    let (status, first) = route_request(app(&state), Method::PUT, &uri, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["members"][0]["state"], "ready");
    assert!(matches!(
        owner.await.unwrap().as_slice(),
        [ServiceToProcess::LinkAttach { .. }]
    ));

    let (status, second) = tokio::time::timeout(
        Duration::from_secs(2),
        route_request(app(&state), Method::PUT, &uri, None),
    )
    .await
    .expect("repeated attach must not wait for another owner handshake");
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second, first);
    assert_eq!(state.switches.plugged(id.parse().unwrap()).await, vec!["vm-b"]);
}
