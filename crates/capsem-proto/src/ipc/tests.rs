//! Tests for `ipc` (extracted from inline `mod tests`).

use super::*;

// -----------------------------------------------------------------------
// ServiceToProcess serde roundtrips
// -----------------------------------------------------------------------

#[test]
fn ping_roundtrip() {
    let msg = ServiceToProcess::Ping;
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ServiceToProcess = serde_json::from_slice(&bytes).unwrap();
    assert!(matches!(msg2, ServiceToProcess::Ping));
}

#[test]
fn terminal_input_roundtrip() {
    let msg = ServiceToProcess::TerminalInput {
        data: vec![0x41, 0x42, 0x0a],
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ServiceToProcess = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ServiceToProcess::TerminalInput { data } => assert_eq!(data, vec![0x41, 0x42, 0x0a]),
        _ => panic!("wrong variant"),
    }
}

#[test]
fn terminal_resize_roundtrip() {
    let msg = ServiceToProcess::TerminalResize {
        cols: 120,
        rows: 40,
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ServiceToProcess = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ServiceToProcess::TerminalResize { cols, rows } => {
            assert_eq!(cols, 120);
            assert_eq!(rows, 40);
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn shutdown_roundtrip() {
    let msg = ServiceToProcess::Shutdown;
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ServiceToProcess = serde_json::from_slice(&bytes).unwrap();
    assert!(matches!(msg2, ServiceToProcess::Shutdown));
}

#[test]
fn exec_roundtrip() {
    let msg = ServiceToProcess::Exec {
        id: 42,
        command: "echo hi".into(),
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ServiceToProcess = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ServiceToProcess::Exec { id, command } => {
            assert_eq!(id, 42);
            assert_eq!(command, "echo hi");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn write_file_roundtrip() {
    let msg = ServiceToProcess::WriteFile {
        id: 7,
        path: "/tmp/test.txt".into(),
        data: b"hello".to_vec(),
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ServiceToProcess = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ServiceToProcess::WriteFile { id, path, data } => {
            assert_eq!(id, 7);
            assert_eq!(path, "/tmp/test.txt");
            assert_eq!(data, b"hello");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn read_file_roundtrip() {
    let msg = ServiceToProcess::ReadFile {
        id: 99,
        path: "/etc/hostname".into(),
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ServiceToProcess = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ServiceToProcess::ReadFile { id, path } => {
            assert_eq!(id, 99);
            assert_eq!(path, "/etc/hostname");
        }
        _ => panic!("wrong variant"),
    }
}

// -----------------------------------------------------------------------
// ProcessToService serde roundtrips
// -----------------------------------------------------------------------

#[test]
fn pong_roundtrip() {
    let msg = ProcessToService::Pong;
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    assert!(matches!(msg2, ProcessToService::Pong));
}

#[test]
fn terminal_output_roundtrip() {
    let msg = ProcessToService::TerminalOutput {
        data: vec![0x68, 0x69],
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ProcessToService::TerminalOutput { data } => assert_eq!(data, vec![0x68, 0x69]),
        _ => panic!("wrong variant"),
    }
}

#[test]
fn state_changed_roundtrip() {
    let msg = ProcessToService::StateChanged {
        id: "vm-1".into(),
        state: "Running".into(),
        trigger: "booted".into(),
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ProcessToService::StateChanged { id, state, trigger } => {
            assert_eq!(id, "vm-1");
            assert_eq!(state, "Running");
            assert_eq!(trigger, "booted");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn exec_result_roundtrip() {
    let msg = ProcessToService::ExecResult {
        id: 42,
        stdout: b"hello\n".to_vec(),
        stderr: b"".to_vec(),
        exit_code: 0,
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ProcessToService::ExecResult {
            id,
            stdout,
            stderr,
            exit_code,
        } => {
            assert_eq!(id, 42);
            assert_eq!(stdout, b"hello\n");
            assert!(stderr.is_empty());
            assert_eq!(exit_code, 0);
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn exec_result_nonzero_exit() {
    let msg = ProcessToService::ExecResult {
        id: 1,
        stdout: vec![],
        stderr: b"not found\n".to_vec(),
        exit_code: 127,
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ProcessToService::ExecResult {
            exit_code, stderr, ..
        } => {
            assert_eq!(exit_code, 127);
            assert_eq!(stderr, b"not found\n");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn write_file_result_success() {
    let msg = ProcessToService::WriteFileResult {
        id: 5,
        success: true,
        error: None,
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ProcessToService::WriteFileResult { id, success, error } => {
            assert_eq!(id, 5);
            assert!(success);
            assert!(error.is_none());
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn write_file_result_failure() {
    let msg = ProcessToService::WriteFileResult {
        id: 5,
        success: false,
        error: Some("permission denied".into()),
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ProcessToService::WriteFileResult { success, error, .. } => {
            assert!(!success);
            assert_eq!(error.unwrap(), "permission denied");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn read_file_result_success() {
    let msg = ProcessToService::ReadFileResult {
        id: 10,
        data: Some(b"file contents".to_vec()),
        error: None,
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ProcessToService::ReadFileResult { data, error, .. } => {
            assert_eq!(data.unwrap(), b"file contents");
            assert!(error.is_none());
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn read_file_result_not_found() {
    let msg = ProcessToService::ReadFileResult {
        id: 10,
        data: None,
        error: Some("file not found".into()),
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ProcessToService::ReadFileResult { data, error, .. } => {
            assert!(data.is_none());
            assert_eq!(error.unwrap(), "file not found");
        }
        _ => panic!("wrong variant"),
    }
}

// -----------------------------------------------------------------------
// Job ID correlation
// -----------------------------------------------------------------------

#[test]
fn job_ids_are_distinct() {
    let exec = ServiceToProcess::Exec {
        id: 1,
        command: "a".into(),
    };
    let write = ServiceToProcess::WriteFile {
        id: 2,
        path: "/x".into(),
        data: vec![],
    };
    let read = ServiceToProcess::ReadFile {
        id: 3,
        path: "/y".into(),
    };

    // Verify each preserves its own ID through serde
    let e: ServiceToProcess = serde_json::from_slice(&serde_json::to_vec(&exec).unwrap()).unwrap();
    let w: ServiceToProcess = serde_json::from_slice(&serde_json::to_vec(&write).unwrap()).unwrap();
    let r: ServiceToProcess = serde_json::from_slice(&serde_json::to_vec(&read).unwrap()).unwrap();

    match (e, w, r) {
        (
            ServiceToProcess::Exec { id: e_id, .. },
            ServiceToProcess::WriteFile { id: w_id, .. },
            ServiceToProcess::ReadFile { id: r_id, .. },
        ) => {
            assert_eq!(e_id, 1);
            assert_eq!(w_id, 2);
            assert_eq!(r_id, 3);
        }
        _ => panic!("wrong variants"),
    }
}

// -----------------------------------------------------------------------
// ReloadConfig
// -----------------------------------------------------------------------

#[test]
fn reload_config_roundtrip() {
    let msg = ServiceToProcess::ReloadConfig {
        runtime_rules: Some(RuntimeSecurityRulesSnapshot {
            enforcement: vec![RuntimeEnforcementRuleSnapshot {
                id: "block-metadata".into(),
                pack_id: Some("runtime-pack".into()),
                condition: "http.request.host == 'metadata.google.internal'".into(),
                decision: RuntimeSecurityDecisionAction::Block,
                reason: Some("metadata access".into()),
            }],
            detection: vec![RuntimeDetectionRuleSnapshot {
                id: "detect-tool".into(),
                pack_id: "runtime-detection".into(),
                sigma_id: Some("sigma-1".into()),
                title: "Tool execution".into(),
                condition: "mcp.request.tool_name == 'danger'".into(),
                severity: RuntimeDetectionSeverity::High,
                confidence: RuntimeDetectionConfidence::Medium,
                tags: vec!["mcp".into()],
            }],
        }),
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ServiceToProcess = serde_json::from_slice(&bytes).unwrap();
    let ServiceToProcess::ReloadConfig { runtime_rules } = msg2 else {
        panic!("wrong variant")
    };
    let runtime_rules = runtime_rules.expect("runtime rule snapshot should round trip");
    assert_eq!(runtime_rules.enforcement[0].id, "block-metadata");
    assert_eq!(
        runtime_rules.enforcement[0].decision,
        RuntimeSecurityDecisionAction::Block
    );
    assert_eq!(
        runtime_rules.detection[0].severity,
        RuntimeDetectionSeverity::High
    );
    assert_eq!(
        runtime_rules.detection[0].confidence,
        RuntimeDetectionConfidence::Medium
    );
}

#[test]
fn reload_config_result_roundtrip() {
    let msg = ProcessToService::ReloadConfigResult {
        success: false,
        error: Some("refresh failed".into()),
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ProcessToService::ReloadConfigResult { success, error } => {
            assert!(!success);
            assert_eq!(error.as_deref(), Some("refresh failed"));
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn runtime_rule_match_drain_roundtrip() {
    let request = ServiceToProcess::DrainRuntimeRuleMatches { id: 77 };
    let bytes = serde_json::to_vec(&request).unwrap();
    let decoded: ServiceToProcess = serde_json::from_slice(&bytes).unwrap();
    match decoded {
        ServiceToProcess::DrainRuntimeRuleMatches { id } => assert_eq!(id, 77),
        other => panic!("wrong variant: {other:?}"),
    }

    let response = ProcessToService::RuntimeRuleMatches {
        id: 77,
        matches: vec![RuntimeRuleMatchSnapshot {
            rule_id: "block-live".into(),
            match_count: 2,
            last_matched_event: Some("evt-2".into()),
            last_matched_unix_ms: Some(1_790),
        }],
    };
    let bytes = serde_json::to_vec(&response).unwrap();
    let decoded: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    match decoded {
        ProcessToService::RuntimeRuleMatches { id, matches } => {
            assert_eq!(id, 77);
            assert_eq!(matches[0].rule_id, "block-live");
            assert_eq!(matches[0].match_count, 2);
            assert_eq!(matches[0].last_matched_event.as_deref(), Some("evt-2"));
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

// -----------------------------------------------------------------------
// Lifecycle IPC roundtrips
// -----------------------------------------------------------------------

#[test]
fn prepare_snapshot_roundtrip() {
    let msg = ServiceToProcess::PrepareSnapshot;
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ServiceToProcess = serde_json::from_slice(&bytes).unwrap();
    assert!(matches!(msg2, ServiceToProcess::PrepareSnapshot));
}

#[test]
fn unfreeze_roundtrip() {
    let msg = ServiceToProcess::Unfreeze;
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ServiceToProcess = serde_json::from_slice(&bytes).unwrap();
    assert!(matches!(msg2, ServiceToProcess::Unfreeze));
}

#[test]
fn suspend_roundtrip() {
    let msg = ServiceToProcess::Suspend {
        checkpoint_path: "/tmp/checkpoint.vzsave".into(),
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ServiceToProcess = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ServiceToProcess::Suspend { checkpoint_path } => {
            assert_eq!(checkpoint_path, "/tmp/checkpoint.vzsave");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn resume_roundtrip() {
    let msg = ServiceToProcess::Resume;
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ServiceToProcess = serde_json::from_slice(&bytes).unwrap();
    assert!(matches!(msg2, ServiceToProcess::Resume));
}

#[test]
fn shutdown_requested_roundtrip() {
    let msg = ProcessToService::ShutdownRequested {
        id: "vm-abc".into(),
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ProcessToService::ShutdownRequested { id } => assert_eq!(id, "vm-abc"),
        _ => panic!("wrong variant"),
    }
}

#[test]
fn suspend_requested_roundtrip() {
    let msg = ProcessToService::SuspendRequested {
        id: "vm-xyz".into(),
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ProcessToService::SuspendRequested { id } => assert_eq!(id, "vm-xyz"),
        _ => panic!("wrong variant"),
    }
}

#[test]
fn snapshot_ready_roundtrip() {
    let msg = ProcessToService::SnapshotReady {
        id: "vm-snap".into(),
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ProcessToService::SnapshotReady { id } => assert_eq!(id, "vm-snap"),
        _ => panic!("wrong variant"),
    }
}

#[test]
fn metrics_snapshot_ipc_roundtrip_bincode() {
    let request = ServiceToProcess::GetMetricsSnapshot { id: 44 };
    let request_bytes = bincode::serialize(&request).unwrap();
    let request2: ServiceToProcess = bincode::deserialize(&request_bytes).unwrap();
    match request2 {
        ServiceToProcess::GetMetricsSnapshot { id } => assert_eq!(id, 44),
        _ => panic!("wrong variant"),
    }

    let snapshot = crate::metrics::VmMetricsSnapshot::empty("vm-metrics", true, 1_789);
    assert_eq!(
        snapshot.schema_version,
        crate::metrics::METRICS_SCHEMA_VERSION
    );
    assert_eq!(snapshot.vm_id, "vm-metrics");
    assert!(snapshot.persistent);
    assert_eq!(snapshot.http.http_requests_total, 0);
    assert_eq!(snapshot.model.model_estimated_cost_micros_total, 0);

    let response = ProcessToService::MetricsSnapshot {
        id: 44,
        snapshot: Box::new(snapshot),
    };
    let response_bytes = bincode::serialize(&response).unwrap();
    let response2: ProcessToService = bincode::deserialize(&response_bytes).unwrap();
    match response2 {
        ProcessToService::MetricsSnapshot { id, snapshot } => {
            assert_eq!(id, 44);
            assert_eq!(snapshot.vm_id, "vm-metrics");
            assert_eq!(snapshot.captured_at_unix_ms, 1_789);
        }
        _ => panic!("wrong variant"),
    }
}
