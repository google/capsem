use std::collections::BTreeMap;
use std::time::Duration;

use super::*;
use capsem_proto::mcp_aggregator::{AggregatorClient, AggregatorResponse, AggregatorResult, AggregatorServerStatus};
use capsem_proto::mcp_contracts::McpToolDef;
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

#[tokio::test]
async fn negotiated_dispatcher_covers_stream_jobs_queries_and_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    let active_profile = temp.path().join("active_profile.toml");
    std::fs::write(
        &active_profile,
        r#"
id = "code"
name = "Code"
description = "IPC dispatcher fixture."
revision = "test.1"
"#,
    )
    .unwrap();
    let db = Arc::new(capsem_logger::DbWriter::open_in_memory(64).unwrap());
    let net_state = Arc::new(
        capsem_core::create_net_state_with_policy(
            "ipc-dispatch-test",
            Arc::clone(&db),
            capsem_core::net::policy::NetworkMechanics::default(),
        )
        .unwrap(),
    );
    let security_rules = Arc::new(std::sync::RwLock::new(Arc::new(
        capsem_core::net::policy_config::SecurityRuleSet::new(Vec::new()),
    )));
    let plugin_policy = Arc::new(std::sync::RwLock::new(Arc::new(BTreeMap::new())));
    let model_endpoints = Arc::new(std::sync::RwLock::new(Arc::new(
        capsem_core::net::policy_config::ModelEndpointRegistry::default(),
    )));
    let (aggregator, mut aggregator_rx) = AggregatorClient::channel(8);
    tokio::spawn(async move {
        while let Some((request, response_tx)) = aggregator_rx.recv().await {
            let body = match request.method {
                capsem_proto::mcp_aggregator::AggregatorMethod::ListServers => AggregatorResult::Servers {
                    servers: vec![AggregatorServerStatus {
                        name: "fixture".to_string(),
                        url: "stdio://fixture".to_string(),
                        enabled: true,
                        source: "profile".to_string(),
                        is_stdio: true,
                        connected: true,
                        tool_count: 1,
                        resource_count: 0,
                        prompt_count: 0,
                    }],
                },
                capsem_proto::mcp_aggregator::AggregatorMethod::ListTools => AggregatorResult::Tools {
                    tools: vec![McpToolDef {
                        namespaced_name: "fixture__echo".to_string(),
                        original_name: "echo".to_string(),
                        description: Some("Echo fixture input".to_string()),
                        input_schema: serde_json::json!({"type": "object"}),
                        server_name: "fixture".to_string(),
                        annotations: None,
                        timeout_secs: None,
                    }],
                },
                capsem_proto::mcp_aggregator::AggregatorMethod::Refresh { .. } => AggregatorResult::Ok { ok: true },
                capsem_proto::mcp_aggregator::AggregatorMethod::CallTool { name, arguments, .. } => {
                    AggregatorResult::CallResult {
                        result: serde_json::json!({"tool": name, "arguments": arguments}),
                    }
                }
                _ => AggregatorResult::Error {
                    error: "unsupported in dispatcher fixture".to_string(),
                },
            };
            let _ = response_tx.send(AggregatorResponse { id: request.id, body });
        }
    });
    let endpoint = Arc::new(capsem_core::net::mitm_proxy::McpEndpointState::new(
        aggregator.clone(),
        Arc::clone(&security_rules),
        Arc::clone(&plugin_policy),
        Arc::new(tokio::sync::Semaphore::new(4)),
        capsem_core::net::mitm_proxy::McpTimeouts::from_env(),
    ));
    let mcp_runtime = Arc::new(McpRuntime {
        aggregator,
        endpoint,
        db: Arc::clone(&db),
        security_rules,
        plugin_policy,
        model_endpoints,
    });
    let scheduler = Arc::new(tokio::sync::Mutex::new(
        capsem_core::auto_snapshot::AutoSnapshotScheduler::new(
            temp.path().to_path_buf(),
            2,
            2,
            Duration::from_secs(300),
        ),
    ));
    let term_relay = TerminalRelay::new(8);
    term_relay.publish(b"boot banner\n".to_vec());
    let job_store = Arc::new(JobStore::new());
    let (ctrl_tx, mut ctrl_rx) = mpsc::channel(16);
    let (events_tx, _) = broadcast::channel(16);
    let ready = Arc::new(AtomicBool::new(true));
    let (process_stream, service_stream) = tokio::net::UnixStream::pair().unwrap();
    let handler = tokio::spawn(handle_ipc_connection(
        process_stream,
        ctrl_tx,
        events_tx,
        Arc::clone(&term_relay),
        Arc::clone(&job_store),
        net_state,
        mcp_runtime,
        RuntimeProfileSource::new(active_profile),
        None,
        HashMap::new(),
        scheduler,
        ready,
    ));

    let mut service_stream = service_stream.into_std().unwrap();
    let service = tokio::task::spawn_blocking(move || {
        capsem_foundation::ipc_handshake::negotiate_initiator(&mut service_stream, "capsem-service-test", "").unwrap();
        let channel: (
            tokio_unix_ipc::Sender<ServiceToProcess>,
            tokio_unix_ipc::Receiver<ProcessToService>,
        ) = tokio_unix_ipc::channel_from_std(service_stream).unwrap();
        channel
    });
    let (service_tx, service_rx) = service.await.unwrap();

    service_tx.send(ServiceToProcess::StartTerminalStream).await.unwrap();
    match service_rx.recv().await.unwrap() {
        ProcessToService::TerminalOutput { data } => assert_eq!(data, b"boot banner\n"),
        other => panic!("unexpected stream response: {other:?}"),
    }
    service_tx.send(ServiceToProcess::StopTerminalStream).await.unwrap();

    service_tx.send(ServiceToProcess::Ping).await.unwrap();
    assert!(matches!(service_rx.recv().await.unwrap(), ProcessToService::Pong));

    service_tx
        .send(ServiceToProcess::TerminalInput {
            data: b"pwd\n".to_vec(),
        })
        .await
        .unwrap();
    assert!(matches!(
        ctrl_rx.recv().await.unwrap(),
        ServiceToProcess::TerminalInput { data } if data == b"pwd\n"
    ));
    service_tx
        .send(ServiceToProcess::TerminalResize { cols: 120, rows: 40 })
        .await
        .unwrap();
    assert!(matches!(
        ctrl_rx.recv().await.unwrap(),
        ServiceToProcess::TerminalResize { cols: 120, rows: 40 }
    ));

    service_tx
        .send(ServiceToProcess::Exec {
            id: 10,
            command: "printf ok".to_string(),
        })
        .await
        .unwrap();
    assert!(matches!(
        ctrl_rx.recv().await.unwrap(),
        ServiceToProcess::Exec { id: 10, .. }
    ));
    job_store
        .jobs
        .lock()
        .unwrap()
        .remove(&10)
        .unwrap()
        .send(JobResult::Exec {
            stdout: b"ok".to_vec(),
            stderr: Vec::new(),
            exit_code: 0,
            truncated: false,
        })
        .unwrap();
    assert!(matches!(
        service_rx.recv().await.unwrap(),
        ProcessToService::ExecResult { id: 10, exit_code: 0, stdout, .. } if stdout == b"ok"
    ));

    service_tx
        .send(ServiceToProcess::Exec {
            id: 17,
            command: "false".to_string(),
        })
        .await
        .unwrap();
    assert!(matches!(
        ctrl_rx.recv().await.unwrap(),
        ServiceToProcess::Exec { id: 17, .. }
    ));
    job_store
        .jobs
        .lock()
        .unwrap()
        .remove(&17)
        .unwrap()
        .send(JobResult::Error {
            message: "exec fixture failed".to_string(),
        })
        .unwrap();
    assert!(matches!(
        service_rx.recv().await.unwrap(),
        ProcessToService::ExecResult {
            id: 17,
            exit_code: -1,
            stderr,
            ..
        } if stderr == b"exec fixture failed"
    ));

    service_tx
        .send(ServiceToProcess::Exec {
            id: 18,
            command: "closed".to_string(),
        })
        .await
        .unwrap();
    assert!(matches!(
        ctrl_rx.recv().await.unwrap(),
        ServiceToProcess::Exec { id: 18, .. }
    ));
    drop(job_store.jobs.lock().unwrap().remove(&18).unwrap());
    assert!(matches!(
        service_rx.recv().await.unwrap(),
        ProcessToService::ExecResult {
            id: 18,
            exit_code: -1,
            stderr,
            ..
        } if stderr == b"Exec result channel closed"
    ));

    service_tx
        .send(ServiceToProcess::WriteFile {
            id: 11,
            path: "/root/out.txt".to_string(),
            data: b"data".to_vec(),
        })
        .await
        .unwrap();
    assert!(matches!(
        ctrl_rx.recv().await.unwrap(),
        ServiceToProcess::WriteFile { id: 11, .. }
    ));
    job_store
        .jobs
        .lock()
        .unwrap()
        .remove(&11)
        .unwrap()
        .send(JobResult::WriteFile {
            success: false,
            error: Some("fixture refusal".to_string()),
        })
        .unwrap();
    assert!(matches!(
        service_rx.recv().await.unwrap(),
        ProcessToService::WriteFileResult {
            id: 11,
            success: false,
            ..
        }
    ));

    service_tx
        .send(ServiceToProcess::WriteFile {
            id: 19,
            path: "/root/error.txt".to_string(),
            data: Vec::new(),
        })
        .await
        .unwrap();
    assert!(matches!(
        ctrl_rx.recv().await.unwrap(),
        ServiceToProcess::WriteFile { id: 19, .. }
    ));
    job_store
        .jobs
        .lock()
        .unwrap()
        .remove(&19)
        .unwrap()
        .send(JobResult::Error {
            message: "write fixture failed".to_string(),
        })
        .unwrap();
    assert!(matches!(
        service_rx.recv().await.unwrap(),
        ProcessToService::WriteFileResult {
            id: 19,
            success: false,
            error: Some(error),
        } if error == "write fixture failed"
    ));

    // A write the guest could never decode is refused at once with the
    // reason, instead of being framed, dropped by the guest, and replayed
    // until the watchdog gives up.
    service_tx
        .send(ServiceToProcess::WriteFile {
            id: 21,
            path: "/root/huge.bin".to_string(),
            data: vec![0u8; capsem_proto::MAX_FRAME_SIZE as usize + 1],
        })
        .await
        .unwrap();
    assert!(matches!(
        service_rx.recv().await.unwrap(),
        ProcessToService::WriteFileResult {
            id: 21,
            success: false,
            error: Some(error),
        } if error.contains("too large")
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(200), ctrl_rx.recv())
            .await
            .is_err(),
        "an oversized WriteFile must never reach the guest bridge"
    );
    assert!(
        !job_store.jobs.lock().unwrap().contains_key(&21),
        "no job may be parked for a refused write"
    );

    service_tx
        .send(ServiceToProcess::ReadFile {
            id: 12,
            path: "/root/out.txt".to_string(),
        })
        .await
        .unwrap();
    assert!(matches!(
        ctrl_rx.recv().await.unwrap(),
        ServiceToProcess::ReadFile { id: 12, .. }
    ));
    job_store
        .jobs
        .lock()
        .unwrap()
        .remove(&12)
        .unwrap()
        .send(JobResult::ReadFile {
            data: Some(b"read".to_vec()),
            error: None,
        })
        .unwrap();
    assert!(matches!(
        service_rx.recv().await.unwrap(),
        ProcessToService::ReadFileResult { id: 12, data: Some(data), .. } if data == b"read"
    ));

    service_tx
        .send(ServiceToProcess::ReadFile {
            id: 20,
            path: "/root/missing.txt".to_string(),
        })
        .await
        .unwrap();
    assert!(matches!(
        ctrl_rx.recv().await.unwrap(),
        ServiceToProcess::ReadFile { id: 20, .. }
    ));
    job_store
        .jobs
        .lock()
        .unwrap()
        .remove(&20)
        .unwrap()
        .send(JobResult::Error {
            message: "read fixture failed".to_string(),
        })
        .unwrap();
    assert!(matches!(
        service_rx.recv().await.unwrap(),
        ProcessToService::ReadFileResult {
            id: 20,
            data: None,
            error: Some(error),
        } if error == "read fixture failed"
    ));

    service_tx
        .send(ServiceToProcess::LogFileBoundary {
            id: 13,
            action: capsem_proto::ipc::FileBoundaryAction::Export,
            path: "/root/out.txt".to_string(),
            data: Vec::new(),
            size: 4,
            mime_type: Some("text/plain".to_string()),
        })
        .await
        .unwrap();
    assert!(matches!(
        ctrl_rx.recv().await.unwrap(),
        ServiceToProcess::LogFileBoundary { id: 13, .. }
    ));
    job_store
        .jobs
        .lock()
        .unwrap()
        .remove(&13)
        .unwrap()
        .send(JobResult::LogFileBoundary {
            success: true,
            data: None,
            error: None,
        })
        .unwrap();
    assert!(matches!(
        service_rx.recv().await.unwrap(),
        ProcessToService::LogFileBoundaryResult {
            id: 13,
            success: true,
            ..
        }
    ));

    for (id, result, expected_error) in [
        (
            21,
            JobResult::Error {
                message: "boundary fixture failed".to_string(),
            },
            "boundary fixture failed",
        ),
        (
            22,
            JobResult::ReadFile {
                data: None,
                error: None,
            },
            "unexpected log file boundary result",
        ),
    ] {
        service_tx
            .send(ServiceToProcess::LogFileBoundary {
                id,
                action: capsem_proto::ipc::FileBoundaryAction::Import,
                path: "/root/in.txt".to_string(),
                data: Vec::new(),
                size: 0,
                mime_type: None,
            })
            .await
            .unwrap();
        assert!(matches!(
            ctrl_rx.recv().await.unwrap(),
            ServiceToProcess::LogFileBoundary { id: actual, .. } if actual == id
        ));
        job_store
            .jobs
            .lock()
            .unwrap()
            .remove(&id)
            .unwrap()
            .send(result)
            .unwrap();
        assert!(matches!(
            service_rx.recv().await.unwrap(),
            ProcessToService::LogFileBoundaryResult {
                id: actual,
                success: false,
                error: Some(error),
                ..
            } if actual == id && error == expected_error
        ));
    }

    service_tx
        .send(ServiceToProcess::LogFileBoundary {
            id: 23,
            action: capsem_proto::ipc::FileBoundaryAction::Export,
            path: "/root/closed.txt".to_string(),
            data: Vec::new(),
            size: 0,
            mime_type: None,
        })
        .await
        .unwrap();
    assert!(matches!(
        ctrl_rx.recv().await.unwrap(),
        ServiceToProcess::LogFileBoundary { id: 23, .. }
    ));
    drop(job_store.jobs.lock().unwrap().remove(&23).unwrap());
    assert!(matches!(
        service_rx.recv().await.unwrap(),
        ProcessToService::LogFileBoundaryResult {
            id: 23,
            success: false,
            error: Some(error),
            ..
        } if error == "log file boundary result channel closed"
    ));

    service_tx.send(ServiceToProcess::ReloadConfig).await.unwrap();
    assert!(matches!(service_rx.recv().await.unwrap(), ProcessToService::Pong));

    service_tx
        .send(ServiceToProcess::SnapshotStatus { id: 14 })
        .await
        .unwrap();
    assert!(matches!(
        service_rx.recv().await.unwrap(),
        ProcessToService::SnapshotStatusResult { id: 14, .. }
    ));
    service_tx
        .send(ServiceToProcess::McpListServers { id: 15 })
        .await
        .unwrap();
    assert!(matches!(
        service_rx.recv().await.unwrap(),
        ProcessToService::McpServersResult { id: 15, servers }
            if servers.len() == 1 && servers[0].name == "fixture"
    ));
    service_tx
        .send(ServiceToProcess::McpListTools { id: 16 })
        .await
        .unwrap();
    assert!(matches!(
        service_rx.recv().await.unwrap(),
        ProcessToService::McpToolsResult { id: 16, tools }
            if tools.len() == 1 && tools[0].namespaced_name == "fixture__echo"
    ));
    service_tx
        .send(ServiceToProcess::McpRefreshTools { id: 24 })
        .await
        .unwrap();
    assert!(matches!(
        service_rx.recv().await.unwrap(),
        ProcessToService::McpRefreshResult {
            id: 24,
            success: true,
            error: None,
        }
    ));
    service_tx
        .send(ServiceToProcess::McpCallTool {
            id: 25,
            namespaced_name: "fixture__echo".to_string(),
            arguments_json: r#"{"text":"hello"}"#.to_string(),
        })
        .await
        .unwrap();
    match service_rx.recv().await.unwrap() {
        ProcessToService::McpCallToolResult {
            id: 25,
            result_json: Some(result),
            error: None,
            ..
        } => {
            let result: serde_json::Value = serde_json::from_str(&result).unwrap();
            assert_eq!(result["result"]["tool"], "fixture__echo");
            assert_eq!(result["result"]["arguments"]["text"], "hello");
        }
        other => panic!("unexpected MCP call result: {other:?}"),
    }

    service_tx
        .send(ServiceToProcess::Suspend {
            checkpoint_path: "checkpoint.vzsave".to_string(),
        })
        .await
        .unwrap();
    assert!(matches!(
        ctrl_rx.recv().await.unwrap(),
        ServiceToProcess::Suspend { .. }
    ));
    service_tx.send(ServiceToProcess::PrepareSnapshot).await.unwrap();
    service_tx.send(ServiceToProcess::Shutdown).await.unwrap();
    assert!(matches!(ctrl_rx.recv().await.unwrap(), ServiceToProcess::Shutdown));
    handler.await.unwrap().unwrap();
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
