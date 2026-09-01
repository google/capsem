use std::time::Duration;

use super::*;
use tokio::io::AsyncWriteExt;
use tokio::sync::oneshot;

#[tokio::test]
async fn exec_wait_has_no_internal_deadline() {
    let (_tx, rx) = oneshot::channel();

    let result = tokio::time::timeout(Duration::from_millis(25), await_exec_result(rx)).await;

    assert!(
        result.is_err(),
        "unfinished exec jobs must wait for command completion or the service caller timeout"
    );
}

#[tokio::test]
async fn exec_wait_returns_completed_exec_result() {
    let (tx, rx) = oneshot::channel();
    tx.send(JobResult::Exec {
        stdout: b"done\n".to_vec(),
        stderr: Vec::new(),
        exit_code: 0,
        truncated: false,
    })
    .unwrap();

    match await_exec_result(rx).await.unwrap() {
        JobResult::Exec {
            stdout,
            stderr,
            exit_code,
            ..
        } => {
            assert_eq!(stdout, b"done\n");
            assert!(stderr.is_empty());
            assert_eq!(exit_code, 0);
        }
        other => panic!("unexpected job result: {other:?}"),
    }
}

#[tokio::test]
async fn negotiated_channel_carries_typed_messages_in_both_directions() {
    let (process_stream, service_stream) = tokio::net::UnixStream::pair().unwrap();
    let mut service_stream = service_stream.into_std().unwrap();
    let service = tokio::task::spawn_blocking(move || {
        capsem_foundation::ipc_handshake::negotiate_initiator(&mut service_stream, "capsem-service-test", "").unwrap();
        let channel: (
            tokio_unix_ipc::Sender<ServiceToProcess>,
            tokio_unix_ipc::Receiver<ProcessToService>,
        ) = tokio_unix_ipc::channel_from_std(service_stream).unwrap();
        channel
    });

    let (process_tx, process_rx) = open_ipc_channel(process_stream).await.unwrap().unwrap();
    let (service_tx, service_rx) = service.await.unwrap();

    service_tx.send(ServiceToProcess::Ping).await.unwrap();
    assert!(matches!(process_rx.recv().await.unwrap(), ServiceToProcess::Ping));

    process_tx.send(ProcessToService::Pong).await.unwrap();
    assert!(matches!(service_rx.recv().await.unwrap(), ProcessToService::Pong));
}

#[tokio::test]
async fn malformed_handshake_is_refused_before_typed_ipc_starts() {
    let (process_stream, mut peer) = tokio::net::UnixStream::pair().unwrap();
    peer.write_all(&0_u32.to_be_bytes()).await.unwrap();

    assert!(open_ipc_channel(process_stream).await.unwrap().is_none());
}

#[test]
fn classify_ping() {
    assert_eq!(classify_ipc_message(&ServiceToProcess::Ping), IpcAction::HealthCheck);
}

#[test]
fn classify_terminal_input() {
    assert_eq!(
        classify_ipc_message(&ServiceToProcess::TerminalInput { data: vec![0x41] }),
        IpcAction::Forward
    );
}

#[test]
fn classify_terminal_resize() {
    assert_eq!(
        classify_ipc_message(&ServiceToProcess::TerminalResize { cols: 80, rows: 24 }),
        IpcAction::Forward
    );
}

#[test]
fn classify_exec() {
    assert_eq!(
        classify_ipc_message(&ServiceToProcess::Exec {
            id: 1,
            command: "ls".into()
        }),
        IpcAction::Job
    );
}

#[test]
fn classify_write_file() {
    assert_eq!(
        classify_ipc_message(&ServiceToProcess::WriteFile {
            id: 1,
            path: "/tmp/f".into(),
            data: vec![]
        }),
        IpcAction::Job
    );
}

#[test]
fn guest_write_ledger_path_strips_guest_root_prefix() {
    assert_eq!(
        guest_write_ledger_path("/root/poem.md"),
        "poem.md",
        "session fs_events store guest-root relative paths"
    );
    assert_eq!(guest_write_ledger_path("/root/nested/poem.md"), "nested/poem.md");
}

#[test]
fn guest_write_ledger_path_strips_workspace_prefix() {
    assert_eq!(
        guest_write_ledger_path("/workspace/out.txt"),
        "out.txt",
        "workspace writes use the same relative ledger shape as the fs monitor"
    );
}

#[test]
fn guest_write_ledger_path_preserves_unknown_absolute_paths() {
    assert_eq!(
        guest_write_ledger_path("/tmp/out.txt"),
        "/tmp/out.txt",
        "unknown absolute paths stay explicit instead of being silently rewritten"
    );
}

#[test]
fn classify_read_file() {
    assert_eq!(
        classify_ipc_message(&ServiceToProcess::ReadFile {
            id: 1,
            path: "/tmp/f".into()
        }),
        IpcAction::Job
    );
}

#[test]
fn classify_log_file_boundary() {
    assert_eq!(
        classify_ipc_message(&ServiceToProcess::LogFileBoundary {
            id: 1,
            action: capsem_proto::ipc::FileBoundaryAction::Export,
            path: "/tmp/f".into(),
            data: vec![],
            size: 0,
            mime_type: None,
        }),
        IpcAction::Job
    );
}

#[test]
fn classify_reload_config() {
    assert_eq!(classify_ipc_message(&ServiceToProcess::ReloadConfig), IpcAction::Reload);
}

#[test]
fn classify_shutdown() {
    assert_eq!(classify_ipc_message(&ServiceToProcess::Shutdown), IpcAction::Lifecycle);
}

#[test]
fn classify_suspend() {
    assert_eq!(
        classify_ipc_message(&ServiceToProcess::Suspend {
            checkpoint_path: "cp.vzsave".into()
        }),
        IpcAction::Lifecycle
    );
}

#[test]
fn classify_start_terminal_stream() {
    assert_eq!(
        classify_ipc_message(&ServiceToProcess::StartTerminalStream),
        IpcAction::StreamSetup
    );
}

#[test]
fn classify_stop_terminal_stream() {
    // StopTerminalStream is the cancel side of StartTerminalStream;
    // both belong to the same dispatch category. Mis-classifying it
    // would route the message somewhere that does nothing, leaving
    // the host streaming after `capsem shell` exits -- the bug we're
    // pinning here.
    assert_eq!(
        classify_ipc_message(&ServiceToProcess::StopTerminalStream),
        IpcAction::StreamSetup
    );
}

#[test]
fn classify_prepare_snapshot_unexpected() {
    assert_eq!(
        classify_ipc_message(&ServiceToProcess::PrepareSnapshot),
        IpcAction::Unexpected
    );
}

#[test]
fn classify_unfreeze_unexpected() {
    assert_eq!(classify_ipc_message(&ServiceToProcess::Unfreeze), IpcAction::Unexpected);
}

#[test]
fn classify_resume_unexpected() {
    assert_eq!(classify_ipc_message(&ServiceToProcess::Resume), IpcAction::Unexpected);
}

#[test]
fn classify_snapshot_status_is_job_query() {
    assert_eq!(
        classify_ipc_message(&ServiceToProcess::SnapshotStatus { id: 1 }),
        IpcAction::Job
    );
}
