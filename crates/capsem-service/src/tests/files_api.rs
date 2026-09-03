use super::*;
use capsem_service::fs_utils::sanitize_file_path;
use std::io::{Read as _, Write as _};

// -----------------------------------------------------------------------
// Download / Upload via resolve_workspace_path
// -----------------------------------------------------------------------

fn setup_vm_with_workspace(state: &ServiceState, dir: &std::path::Path, vm_id: &str) {
    setup_vm_with_workspace_and_uds(state, dir, vm_id, PathBuf::from("/tmp/test.sock"));
}

fn setup_vm_with_workspace_and_uds(state: &ServiceState, dir: &std::path::Path, vm_id: &str, uds_path: PathBuf) {
    let session_dir = dir.join("session");
    let workspace = session_dir.join("guest/workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    state.instances.lock().unwrap().insert(
        vm_id.into(),
        InstanceInfo {
            id: vm_id.into(),
            name: vm_id.into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: test_asset_pins(),
            pid: 1,
            uds_path,
            session_dir,
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
        },
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WriteFileIpcReply {
    Success,
    Disconnect,
}

async fn spawn_file_boundary_ipc(
    expected_messages: usize,
    write_reply: WriteFileIpcReply,
) -> (
    tempfile::TempDir,
    PathBuf,
    tokio::task::JoinHandle<Vec<ServiceToProcess>>,
) {
    let dir = tempfile::tempdir().unwrap();
    let uds_path = dir.path().join("process.sock");
    let listener = tokio::net::UnixListener::bind(&uds_path).unwrap();
    std::fs::write(uds_path.with_extension("ready"), b"ready").unwrap();
    let handle = tokio::spawn(async move {
        let mut messages = Vec::new();
        for _ in 0..expected_messages {
            let (stream, _) = listener.accept().await.unwrap();
            let std_stream = stream.into_std().unwrap();
            let std_stream = tokio::task::spawn_blocking(move || {
                let mut std_stream = std_stream;
                capsem_foundation::ipc_handshake::negotiate_responder(&mut std_stream, "capsem-process-test", "")?;
                Ok::<_, capsem_proto::handshake::HandshakeError>(std_stream)
            })
            .await
            .unwrap()
            .unwrap();
            let (tx, rx): (
                tokio_unix_ipc::Sender<ProcessToService>,
                tokio_unix_ipc::Receiver<ServiceToProcess>,
            ) = tokio_unix_ipc::channel_from_std(std_stream).unwrap();
            let msg = rx.recv().await.unwrap();
            match &msg {
                ServiceToProcess::LogFileBoundary { id, .. } => {
                    tx.send(ProcessToService::LogFileBoundaryResult {
                        id: *id,
                        success: true,
                        data: None,
                        error: None,
                    })
                    .await
                    .unwrap();
                }
                ServiceToProcess::WriteFile { id, .. } => {
                    if write_reply == WriteFileIpcReply::Disconnect {
                        drop(tx);
                    } else {
                        tx.send(ProcessToService::WriteFileResult {
                            id: *id,
                            success: true,
                            error: None,
                        })
                        .await
                        .unwrap();
                    }
                }
                ServiceToProcess::ReadFile { id, .. } => {
                    tx.send(ProcessToService::ReadFileResult {
                        id: *id,
                        data: Some(b"guest export".to_vec()),
                        error: None,
                    })
                    .await
                    .unwrap();
                }
                other => panic!("unexpected IPC message in file boundary test: {other:?}"),
            }
            messages.push(msg);
        }
        messages
    });
    (dir, uds_path, handle)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_logs_file_import_before_writing_workspace_file() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(1, WriteFileIpcReply::Success).await;
    setup_vm_with_workspace_and_uds(&state, dir.path(), "up-ledger-vm", uds_path);

    let result = handle_upload_file(
        State(state),
        Path("up-ledger-vm".to_string()),
        Query(FileContentQuery {
            path: "new.txt".to_string(),
        }),
        axum::body::Bytes::from_static(b"uploaded through ledger"),
    )
    .await
    .expect("upload should succeed after boundary log");

    assert_eq!(result.size, b"uploaded through ledger".len() as u64);
    let messages = ipc.await.unwrap();
    assert_eq!(messages.len(), 1);
    match &messages[0] {
        ServiceToProcess::LogFileBoundary {
            action,
            path,
            data,
            size,
            ..
        } => {
            assert_eq!(*action, FileBoundaryAction::Import);
            assert_eq!(path, "new.txt");
            assert_eq!(data, b"uploaded through ledger");
            assert_eq!(*size, b"uploaded through ledger".len() as u64);
        }
        other => panic!("upload must log file import before write, got {other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("session/guest/workspace/new.txt")).unwrap(),
        "uploaded through ledger"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_logs_file_export_before_returning_response() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(1, WriteFileIpcReply::Success).await;
    setup_vm_with_workspace_and_uds(&state, dir.path(), "dl-ledger-vm", uds_path);
    let workspace_file = dir.path().join("session/guest/workspace/report.txt");
    std::fs::write(&workspace_file, b"export through ledger").unwrap();

    let response = handle_download_file(
        State(state),
        Path("dl-ledger-vm".to_string()),
        Query(FileContentQuery {
            path: "report.txt".to_string(),
        }),
    )
    .await
    .expect("download should succeed after boundary log");

    assert_eq!(response.status(), StatusCode::OK);
    let messages = ipc.await.unwrap();
    assert_eq!(messages.len(), 1);
    match &messages[0] {
        ServiceToProcess::LogFileBoundary {
            action,
            path,
            data,
            size,
            ..
        } => {
            assert_eq!(*action, FileBoundaryAction::Export);
            assert_eq!(path, "report.txt");
            assert_eq!(data, b"export through ledger");
            assert_eq!(*size, b"export through ledger".len() as u64);
        }
        other => panic!("download must log file export before response, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_file_content_does_not_wait_on_stats_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(1, WriteFileIpcReply::Success).await;
    setup_vm_with_workspace_and_uds(&state, dir.path(), "fast-file-vm", uds_path);
    std::fs::write(
        dir.path().join("session/guest/workspace/latency.txt"),
        b"file content must not wait on ledger rebuild",
    )
    .unwrap();

    // Liveness, not performance: this guards the route blocking on the logger
    // DB, a wait measured in seconds or never. At 250ms it measured the runner
    // instead and failed a release.
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        handle_download_file(
            State(state),
            Path("fast-file-vm".to_string()),
            Query(FileContentQuery {
                path: "latency.txt".to_string(),
            }),
        ),
    )
    .await
    .expect("file content route waited for stats rebuild")
    .expect("download should succeed after boundary log");

    assert_eq!(response.status(), StatusCode::OK);
    let messages = ipc.await.unwrap();
    assert_eq!(messages.len(), 1);
    assert!(matches!(
        &messages[0],
        ServiceToProcess::LogFileBoundary {
            action: FileBoundaryAction::Export,
            path,
            ..
        } if path == "latency.txt"
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mounted_file_import_export_routes_log_boundary_events() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(2, WriteFileIpcReply::Success).await;
    setup_vm_with_workspace_and_uds(&state, dir.path(), "file-route-vm", uds_path);
    let app = build_service_router(state);

    let upload_response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::POST)
                .uri("/vms/file-route-vm/files/content?path=new.txt")
                .body(Body::from("uploaded over mounted route"))
                .unwrap(),
        )
        .await
        .expect("upload route should respond");
    assert_eq!(upload_response.status(), StatusCode::OK);
    let upload_body = to_bytes(upload_response.into_body(), usize::MAX).await.unwrap();
    let upload_json: serde_json::Value = serde_json::from_slice(&upload_body).unwrap();
    assert_eq!(upload_json["success"], true);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("session/guest/workspace/new.txt")).unwrap(),
        "uploaded over mounted route"
    );

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::GET)
                .uri("/vms/file-route-vm/files/content?path=new.txt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("download route should respond");
    assert_eq!(response.status(), StatusCode::OK);
    let downloaded = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&downloaded[..], b"uploaded over mounted route");

    let messages = ipc.await.unwrap();
    assert_eq!(messages.len(), 2);
    match &messages[0] {
        ServiceToProcess::LogFileBoundary {
            action,
            path,
            data,
            size,
            ..
        } => {
            assert_eq!(*action, FileBoundaryAction::Import);
            assert_eq!(path, "new.txt");
            assert_eq!(data, b"uploaded over mounted route");
            assert_eq!(*size, b"uploaded over mounted route".len() as u64);
        }
        other => panic!("upload route must log import first, got {other:?}"),
    }
    match &messages[1] {
        ServiceToProcess::LogFileBoundary {
            action,
            path,
            data,
            size,
            ..
        } => {
            assert_eq!(*action, FileBoundaryAction::Export);
            assert_eq!(path, "new.txt");
            assert_eq!(data, b"uploaded over mounted route");
            assert_eq!(*size, b"uploaded over mounted route".len() as u64);
        }
        other => panic!("download route must log export first, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_does_not_write_workspace_file_when_import_ledger_fails() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let ipc_dir = tempfile::tempdir().unwrap();
    let uds_path = ipc_dir.path().join("process.sock");
    let listener = tokio::net::UnixListener::bind(&uds_path).unwrap();
    std::fs::write(uds_path.with_extension("ready"), b"ready").unwrap();
    let ipc = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let std_stream = stream.into_std().unwrap();
        let std_stream = tokio::task::spawn_blocking(move || {
            let mut std_stream = std_stream;
            capsem_foundation::ipc_handshake::negotiate_responder(&mut std_stream, "capsem-process-test", "")?;
            Ok::<_, capsem_proto::handshake::HandshakeError>(std_stream)
        })
        .await
        .unwrap()
        .unwrap();
        let (tx, rx): (
            tokio_unix_ipc::Sender<ProcessToService>,
            tokio_unix_ipc::Receiver<ServiceToProcess>,
        ) = tokio_unix_ipc::channel_from_std(std_stream).unwrap();
        let msg = rx.recv().await.unwrap();
        match &msg {
            ServiceToProcess::LogFileBoundary { id, .. } => {
                tx.send(ProcessToService::LogFileBoundaryResult {
                    id: *id,
                    success: false,
                    data: None,
                    error: Some("security ledger rejected import".to_string()),
                })
                .await
                .unwrap();
            }
            other => panic!("unexpected IPC message in import denial test: {other:?}"),
        }
        msg
    });
    setup_vm_with_workspace_and_uds(&state, dir.path(), "deny-ledger-vm", uds_path);

    let err = handle_upload_file(
        State(state),
        Path("deny-ledger-vm".to_string()),
        Query(FileContentQuery {
            path: "blocked.txt".to_string(),
        }),
        axum::body::Bytes::from_static(b"must not land"),
    )
    .await
    .expect_err("failed import ledger write must fail closed");

    assert_eq!(err.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(err.1.contains("security ledger rejected import"));
    let msg = ipc.await.unwrap();
    assert!(matches!(msg, ServiceToProcess::LogFileBoundary { .. }));
    assert!(
        !dir.path().join("session/guest/workspace/blocked.txt").exists(),
        "upload must not write bytes when import ledger fails"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_file_logs_import_before_guest_write() {
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(2, WriteFileIpcReply::Success).await;
    state.instances.lock().unwrap().insert(
        "write-ledger-vm".into(),
        InstanceInfo {
            id: "write-ledger-vm".into(),
            name: "write-ledger-vm".into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: test_asset_pins(),
            pid: 1,
            uds_path,
            session_dir: state.run_dir.join("sessions/write-ledger-vm"),
            ram_mb: 2048,
            cpus: 2,
            start_time: std::time::Instant::now(),
            base_version: "0.0.0".into(),
            persistent: false,
            env: None,
            forked_from: None,
        },
    );

    let _ = handle_write_file(
        State(state),
        Path("write-ledger-vm".to_string()),
        Json(WriteFileRequest {
            path: "/workspace/from-api.txt".to_string(),
            content: "guest write".to_string(),
        }),
    )
    .await
    .expect("write_file should succeed after import ledger");

    let messages = ipc.await.unwrap();
    assert_eq!(messages.len(), 2);
    match &messages[0] {
        ServiceToProcess::LogFileBoundary {
            action,
            path,
            data,
            size,
            ..
        } => {
            assert_eq!(*action, FileBoundaryAction::Import);
            assert_eq!(path, "/workspace/from-api.txt");
            assert_eq!(data, b"guest write");
            assert_eq!(*size, b"guest write".len() as u64);
        }
        other => panic!("write_file first IPC must be import ledger, got {other:?}"),
    }
    assert!(matches!(
        messages[1],
        ServiceToProcess::WriteFile { ref path, .. } if path == "/workspace/from-api.txt"
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_file_ipc_failure_names_vm_and_completion_stage() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, ipc) = spawn_file_boundary_ipc(2, WriteFileIpcReply::Disconnect).await;
    setup_vm_with_workspace_and_uds(&state, dir.path(), "diagnostic-vm", uds_path);

    let error = handle_write_file(
        State(state),
        Path("diagnostic-vm".to_string()),
        Json(WriteFileRequest {
            path: "/workspace/failure.txt".to_string(),
            content: "diagnose me".to_string(),
        }),
    )
    .await
    .expect_err("a disconnected process must fail the write");

    let messages = ipc.await.unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(error.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(error.1.contains("VM diagnostic-vm write_file"), "{}", error.1);
    assert!(
        error.1.contains("awaiting the guest completion response"),
        "{}",
        error.1
    );
}

#[test]
fn download_reads_correct_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "dl-vm");

    let ws = dir.path().join("session/guest/workspace");
    let content = b"hello world\nline 2\n";
    std::fs::write(ws.join("test.txt"), content).unwrap();

    let (parent, name) = resolve_workspace_target(&state, "dl-vm", "test.txt", false).unwrap();
    let mut data = Vec::new();
    parent
        .open_file(&name, nix::fcntl::OFlag::O_RDONLY, nix::sys::stat::Mode::empty())
        .unwrap()
        .read_to_end(&mut data)
        .unwrap();
    assert_eq!(data, content);
}

#[test]
fn download_binary_preserves_content() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "bin-vm");

    let ws = dir.path().join("session/guest/workspace");
    let binary: Vec<u8> = (0..256).map(|i| i as u8).collect();
    std::fs::write(ws.join("data.bin"), &binary).unwrap();

    let (parent, name) = resolve_workspace_target(&state, "bin-vm", "data.bin", false).unwrap();
    let mut data = Vec::new();
    parent
        .open_file(&name, nix::fcntl::OFlag::O_RDONLY, nix::sys::stat::Mode::empty())
        .unwrap()
        .read_to_end(&mut data)
        .unwrap();
    assert_eq!(data, binary);
}

#[test]
fn upload_creates_file_with_content() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "up-vm");

    let ws = dir.path().join("session/guest/workspace");
    let (parent, name) = resolve_workspace_target(&state, "up-vm", "new.txt", true).unwrap();
    parent
        .open_file(
            &name,
            nix::fcntl::OFlag::O_WRONLY | nix::fcntl::OFlag::O_CREAT,
            nix::sys::stat::Mode::from_bits_truncate(0o644),
        )
        .unwrap()
        .write_all(b"uploaded")
        .unwrap();

    assert_eq!(std::fs::read_to_string(ws.join("new.txt")).unwrap(), "uploaded");
}

#[test]
fn upload_creates_parent_directories() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "mkdir-vm");

    let ws = dir.path().join("session/guest/workspace");
    // Resolving for upload creates the missing parents inside the workspace
    let (parent, name) = resolve_workspace_target(&state, "mkdir-vm", "deep/nested/file.txt", true).unwrap();
    parent
        .open_file(
            &name,
            nix::fcntl::OFlag::O_WRONLY | nix::fcntl::OFlag::O_CREAT,
            nix::sys::stat::Mode::from_bits_truncate(0o644),
        )
        .unwrap()
        .write_all(b"deep content")
        .unwrap();

    assert_eq!(
        std::fs::read_to_string(ws.join("deep/nested/file.txt")).unwrap(),
        "deep content"
    );
}

#[test]
fn upload_path_traversal_blocked() {
    let r = sanitize_file_path("../../etc/passwd");
    assert!(r.is_err());
}

#[test]
fn download_nonexistent_file_resolves_but_does_not_open() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _dir2) = make_test_state_with_tempdir();
    setup_vm_with_workspace(&state, dir.path(), "404-vm");

    // Resolving a non-existent file path still works (for upload target)
    let (parent, name) = resolve_workspace_target(&state, "404-vm", "nonexistent.txt", false).unwrap();
    assert_eq!(parent.entry_kind(&name).unwrap(), None);
    let err = parent
        .open_file(&name, nix::fcntl::OFlag::O_RDONLY, nix::sys::stat::Mode::empty())
        .unwrap_err();
    assert_eq!(workspace_io_error(err).0, StatusCode::NOT_FOUND);
}

// is_launchd_cleanup_transient identifies the misleading "missing
// entitlement" NSError that VZ emits when launchd's PETRIFIED-cleanup
// queue is saturated under rapid VM churn. The error string is
// stable across VZ releases (Apple's localizedDescription); pattern-
// match conservatively so a real codesign regression doesn't get
// silently retried.

// -----------------------------------------------------------------------
// Files API must not follow guest-planted symlinks
// -----------------------------------------------------------------------
//
// The workspace is a VirtioFS share the guest writes at will, so every
// symlink in it is guest-controlled. The old resolver canonicalized only when
// the target existed, returned the raw path for a dangling link or a missing
// parent, and every handler then opened by path -- which follows symlinks.
// A guest could therefore have the host write its uploads anywhere, read any
// host file back, or list any host directory.

struct EscapeTree {
    dir: tempfile::TempDir,
    outside: PathBuf,
}

impl EscapeTree {
    fn workspace(&self) -> PathBuf {
        self.dir.path().join("session/guest/workspace")
    }
}

fn escape_tree(state: &ServiceState, vm_id: &str, uds_path: PathBuf) -> EscapeTree {
    let dir = tempfile::tempdir().unwrap();
    setup_vm_with_workspace_and_uds(state, dir.path(), vm_id, uds_path);
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), b"host secret").unwrap();
    EscapeTree { dir, outside }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_refuses_a_dangling_symlink_target() {
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, _ipc) = spawn_file_boundary_ipc(0, WriteFileIpcReply::Success).await;
    let tree = escape_tree(&state, "up-dangling-vm", uds_path);
    let planted = tree.outside.join("authorized_keys");
    std::os::unix::fs::symlink(&planted, tree.workspace().join("notes.txt")).unwrap();

    let err = handle_upload_file(
        State(state),
        Path("up-dangling-vm".to_string()),
        Query(FileContentQuery {
            path: "notes.txt".to_string(),
        }),
        axum::body::Bytes::from_static(b"ssh-ed25519 AAAA attacker"),
    )
    .await
    .expect_err("an upload through a guest symlink must be refused");

    assert_eq!(err.0, StatusCode::FORBIDDEN, "{}", err.1);
    assert!(!planted.exists(), "the symlink target must not be created on the host");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_refuses_a_symlinked_parent_even_when_the_leaf_directory_is_missing() {
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, _ipc) = spawn_file_boundary_ipc(0, WriteFileIpcReply::Success).await;
    let tree = escape_tree(&state, "up-parent-vm", uds_path);
    std::os::unix::fs::symlink(&tree.outside, tree.workspace().join("link")).unwrap();

    let err = handle_upload_file(
        State(state),
        Path("up-parent-vm".to_string()),
        Query(FileContentQuery {
            path: "link/sub/new.txt".to_string(),
        }),
        axum::body::Bytes::from_static(b"payload"),
    )
    .await
    .expect_err("an upload below a guest symlink must be refused");

    assert_eq!(err.0, StatusCode::FORBIDDEN, "{}", err.1);
    assert!(
        !tree.outside.join("sub").exists(),
        "no directory may be created outside the workspace"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_refuses_a_symlink_to_a_host_file() {
    let (state, _state_dir) = make_test_state_with_tempdir();
    let (_ipc_dir, uds_path, _ipc) = spawn_file_boundary_ipc(0, WriteFileIpcReply::Success).await;
    let tree = escape_tree(&state, "dl-symlink-vm", uds_path);
    std::os::unix::fs::symlink(tree.outside.join("secret.txt"), tree.workspace().join("leak.txt")).unwrap();

    let err = handle_download_file(
        State(state),
        Path("dl-symlink-vm".to_string()),
        Query(FileContentQuery {
            path: "leak.txt".to_string(),
        }),
    )
    .await
    .expect_err("a download through a guest symlink must be refused");

    assert_eq!(err.0, StatusCode::FORBIDDEN, "{}", err.1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn listing_neither_follows_nor_shows_a_symlinked_directory() {
    let (state, _state_dir) = make_test_state_with_tempdir();
    let tree = escape_tree(&state, "ls-symlink-vm", PathBuf::from("/tmp/test.sock"));
    std::os::unix::fs::symlink(&tree.outside, tree.workspace().join("peek")).unwrap();
    std::fs::write(tree.workspace().join("mine.txt"), b"mine").unwrap();

    let err = handle_list_files(
        State(Arc::clone(&state)),
        Path("ls-symlink-vm".to_string()),
        Query(FileListQuery {
            path: Some("peek".to_string()),
            depth: 3,
        }),
    )
    .await
    .expect_err("listing through a guest symlink must be refused");
    assert_eq!(err.0, StatusCode::FORBIDDEN, "{}", err.1);

    let root = handle_list_files(
        State(state),
        Path("ls-symlink-vm".to_string()),
        Query(FileListQuery { path: None, depth: 3 }),
    )
    .await
    .expect("listing the workspace root");
    let names: Vec<&str> = root.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["mine.txt"], "a symlink is neither followed nor advertised");
}
