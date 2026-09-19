//! The files API speaks the guest's paths: `/root/x` is where a VM sees the
//! workspace file `x`, and `/workspace/x` is where its container does. Before,
//! `/root/x` was taken as the workspace-relative `root/x`, so an upload the
//! caller then read back with `cat /root/x` inside the VM was not there.
use super::files_api::{setup_vm_with_workspace, setup_vm_with_workspace_and_uds, spawn_file_boundary_ipc};
use super::*;
use capsem_service::fs_utils::FileContentQuery;

fn request(method: axum::http::Method, uri: &str, body: &'static str) -> axum::http::Request<Body> {
    axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::from(body))
        .unwrap()
}

async fn json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_absolute_vm_path_lands_where_the_vm_sees_it() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(2).await;
    setup_vm_with_workspace_and_uds(&state, dir.path(), "abs-vm", uds_path);
    let app = build_service_router(state);

    let uploaded = app
        .clone()
        .oneshot(request(
            axum::http::Method::POST,
            "/vms/abs-vm/files/content?path=/root/notes/a.txt",
            "seen by the guest",
        ))
        .await
        .unwrap();
    assert_eq!(uploaded.status(), StatusCode::OK);
    let body = json(uploaded).await;
    assert_eq!(body["vm_path"], "/root/notes/a.txt", "the call returns the true path");
    assert!(body.get("container_path").is_none(), "no container, no container path");
    let workspace = dir.path().join("session/guest/workspace");
    assert_eq!(
        std::fs::read_to_string(workspace.join("notes/a.txt")).unwrap(),
        "seen by the guest"
    );
    assert!(
        !workspace.join("root").exists(),
        "`/root` is the workspace, not a directory in it"
    );

    let downloaded = app
        .oneshot(request(
            axum::http::Method::GET,
            "/vms/abs-vm/files/content?path=notes/a.txt",
            "",
        ))
        .await
        .unwrap();
    assert_eq!(downloaded.status(), StatusCode::OK);
    assert_eq!(downloaded.headers()["x-capsem-vm-path"], "/root/notes/a.txt");
    ipc.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_keeps_the_path_literal() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(1).await;
    setup_vm_with_workspace_and_uds(&state, dir.path(), "exact-vm", uds_path);

    let uploaded = handle_upload_file(
        State(state),
        Path("exact-vm".to_string()),
        Query(FileContentQuery {
            path: "/root/a.txt".to_string(),
            exact: true,
        }),
        axum::body::Bytes::from_static(b"literal"),
    )
    .await
    .unwrap();
    assert_eq!(uploaded.vm_path, "/root/root/a.txt");
    let workspace = dir.path().join("session/guest/workspace");
    assert_eq!(
        std::fs::read_to_string(workspace.join("root/a.txt")).unwrap(),
        "literal"
    );
    ipc.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_absolute_path_is_refused_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "etc-vm");
    let app = build_service_router(state);

    let response = app
        .oneshot(request(
            axum::http::Method::POST,
            "/vms/etc-vm/files/content?path=/etc/passwd",
            "nope",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = String::from_utf8(to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap();
    assert!(body.contains("/root"), "the refusal names the reachable root: {body}");
    assert!(!dir.path().join("session/guest/workspace/etc").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_container_vm_resolves_paths_the_container_sees() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(1).await;
    setup_vm_with_workspace_and_uds(&state, dir.path(), "oci-vm", uds_path);
    std::fs::write(
        dir.path().join("session/container.json"),
        br#"{"image":"docker.io/library/redis:7","digest":"sha256:00"}"#,
    )
    .unwrap();
    let app = build_service_router(state);

    let refused = app
        .clone()
        .oneshot(request(
            axum::http::Method::POST,
            "/vms/oci-vm/files/content?path=/root/a.txt",
            "unreachable",
        ))
        .await
        .unwrap();
    assert_eq!(
        refused.status(),
        StatusCode::BAD_REQUEST,
        "the container cannot see /root; writing there would hide the file from it"
    );

    let uploaded = app
        .oneshot(request(
            axum::http::Method::POST,
            "/vms/oci-vm/files/content?path=/workspace/a.txt",
            "seen by the container",
        ))
        .await
        .unwrap();
    assert_eq!(uploaded.status(), StatusCode::OK);
    let body = json(uploaded).await;
    assert_eq!(body["vm_path"], "/root/a.txt");
    assert_eq!(body["container_path"], "/workspace/a.txt");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("session/guest/workspace/a.txt")).unwrap(),
        "seen by the container"
    );
    ipc.await.unwrap();
}
