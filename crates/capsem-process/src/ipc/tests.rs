use std::time::Duration;

use super::*;
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
    })
    .unwrap();

    match await_exec_result(rx).await.unwrap() {
        JobResult::Exec {
            stdout,
            stderr,
            exit_code,
        } => {
            assert_eq!(stdout, b"done\n");
            assert!(stderr.is_empty());
            assert_eq!(exit_code, 0);
        }
        other => panic!("unexpected job result: {other:?}"),
    }
}

#[test]
fn classify_ping() {
    assert_eq!(
        classify_ipc_message(&ServiceToProcess::Ping),
        IpcAction::HealthCheck
    );
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
fn classify_reload_config() {
    assert_eq!(
        classify_ipc_message(&ServiceToProcess::ReloadConfig),
        IpcAction::Reload
    );
}

#[test]
fn classify_shutdown() {
    assert_eq!(
        classify_ipc_message(&ServiceToProcess::Shutdown),
        IpcAction::Lifecycle
    );
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
    assert_eq!(
        classify_ipc_message(&ServiceToProcess::Unfreeze),
        IpcAction::Unexpected
    );
}

#[test]
fn classify_resume_unexpected() {
    assert_eq!(
        classify_ipc_message(&ServiceToProcess::Resume),
        IpcAction::Unexpected
    );
}
