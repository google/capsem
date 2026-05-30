use std::time::Duration;

use super::*;
use tokio::sync::oneshot;

#[tokio::test]
async fn connection_teardown_aborts_writer_and_lifecycle_tasks() {
    let (ipc_tx_out, mut ipc_rx_out) = mpsc::channel::<ProcessToService>(1);
    let (ipc_tx, _) = broadcast::channel::<ProcessToService>(1);
    let writer_task = tokio::spawn(async move { while ipc_rx_out.recv().await.is_some() {} });
    let lifecycle_task = spawn_lifecycle_forwarder(&ipc_tx, ipc_tx_out.clone());
    drop(ipc_tx_out);

    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(
        !writer_task.is_finished(),
        "writer task should stay alive while lifecycle forwarder holds out_tx"
    );
    assert!(
        !lifecycle_task.is_finished(),
        "lifecycle forwarder should stay alive until connection teardown"
    );

    let mut stream_task = None;
    abort_connection_tasks(&mut stream_task, &lifecycle_task, &writer_task);

    let writer_result = tokio::time::timeout(Duration::from_secs(1), writer_task)
        .await
        .expect("writer task should finish after teardown");
    assert!(writer_result.unwrap_err().is_cancelled());

    let lifecycle_result = tokio::time::timeout(Duration::from_secs(1), lifecycle_task)
        .await
        .expect("lifecycle task should finish after teardown");
    assert!(lifecycle_result.unwrap_err().is_cancelled());
}

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
fn shutdown_before_guest_ready_has_no_grace_period() {
    assert_eq!(shutdown_grace_period(false), Duration::ZERO);
}

#[test]
fn shutdown_after_guest_ready_allows_guest_grace_period() {
    assert_eq!(shutdown_grace_period(true), Duration::from_secs(2));
}

#[test]
fn classify_ping() {
    assert_eq!(
        classify_ipc_message(&ServiceToProcess::Ping),
        IpcAction::HealthCheck
    );
}

#[test]
fn classify_get_metrics_snapshot() {
    assert_eq!(
        classify_ipc_message(&ServiceToProcess::GetMetricsSnapshot { id: 9 }),
        IpcAction::HealthCheck
    );
}

#[test]
fn classify_drain_runtime_rule_matches() {
    assert_eq!(
        classify_ipc_message(&ServiceToProcess::DrainRuntimeRuleMatches { id: 9 }),
        IpcAction::HealthCheck
    );
}

#[test]
fn metrics_snapshot_is_process_owned_and_versioned() {
    let writer = capsem_logger::DbWriter::open_in_memory(16).unwrap();
    let resources = ResourceMetricsContext {
        configured_vcpus: 4,
        configured_ram_mb: 8192,
    };
    let snapshot = metrics_snapshot(&writer, "vm-s07", &resources);

    assert_eq!(snapshot.vm_id, "vm-s07");
    assert_eq!(
        snapshot.schema_version,
        capsem_proto::metrics::METRICS_SCHEMA_VERSION
    );
    assert_eq!(snapshot.lifecycle.state, "unknown");
    assert_eq!(snapshot.ask.total_asks, 0);
    assert_eq!(snapshot.process.process_events_total, 0);
    assert_eq!(snapshot.security.security_events_total, 0);
    assert_eq!(snapshot.resources.configured_vcpus, 4);
    assert_eq!(snapshot.resources.configured_ram_mb, 8192);
    assert_eq!(snapshot.resources.host_pid, Some(std::process::id()));
    #[cfg(target_os = "linux")]
    assert!(snapshot.resources.host_process_rss_bytes.unwrap_or(0) > 0);
    #[cfg(not(target_os = "linux"))]
    assert!(snapshot.resources.host_process_rss_bytes.is_none());
    #[cfg(target_os = "linux")]
    assert!(snapshot.resources.host_cpu_time_micros.is_some());
    #[cfg(not(target_os = "linux"))]
    assert!(snapshot.resources.host_cpu_time_micros.is_none());
    assert_eq!(snapshot.resources.workspace_disk_bytes, None);
    assert_eq!(snapshot.resources.rootfs_overlay_bytes, None);
    assert_eq!(snapshot.resources.session_disk_bytes, None);
    assert!(snapshot.captured_at_unix_ms > 0);
}

#[test]
fn parse_proc_stat_extracts_rss_and_cpu_time() {
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as u64;
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
    let stat = "123 (capsem process) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22";

    let parsed = parse_proc_stat(stat).unwrap();

    assert_eq!(parsed.cpu_time_micros, (11 + 12) * 1_000_000 / ticks);
    assert_eq!(parsed.rss_bytes, 21 * page_size);
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
        classify_ipc_message(&ServiceToProcess::ReloadConfig {
            runtime_rules: None,
        }),
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
