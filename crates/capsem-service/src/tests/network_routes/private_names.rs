use super::*;

#[test]
fn private_name_wrong_process_client() {
    let Ok(socket) = std::env::var("CAPSEM_TEST_PRIVATE_NAME_SOCKET") else {
        return;
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (status, _) = runtime
        .block_on(capsem_core::service_uds::post_json(
            std::path::Path::new(&socket),
            "/networks/private/resolve",
            &json!({"source_vm": "vm-a", "name": "vm-a"}),
        ))
        .unwrap();
    assert_eq!(status, StatusCode::FORBIDDEN.as_u16());
}

#[tokio::test]
async fn private_names_reject_a_real_same_uid_wrong_process() {
    let (state, dir) = make_test_state_with_tempdir();
    insert_fake_instance(&state, "vm-a", std::process::id());
    let socket = dir.path().join("private-name-peer.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let router = app(&state).into_make_service_with_connect_info::<ServicePeer>();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (owner_status, _) = capsem_core::service_uds::post_json(
        &socket,
        "/networks/private/resolve",
        &json!({"source_vm": "vm-a", "name": "nobody"}),
    )
    .await
    .unwrap();
    assert_eq!(
        owner_status,
        StatusCode::NOT_FOUND.as_u16(),
        "the registered owner passes authentication before name lookup"
    );
    let status = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::network_routes::private_names::private_name_wrong_process_client",
            "--nocapture",
        ])
        .env("CAPSEM_TEST_PRIVATE_NAME_SOCKET", &socket)
        .status()
        .await
        .unwrap();
    server.abort();
    assert!(status.success(), "wrong-process probe failed: {status}");
}
