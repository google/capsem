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

#[test]
fn log_file_boundary_roundtrip() {
    let msg = ServiceToProcess::LogFileBoundary {
        id: 101,
        action: FileBoundaryAction::Import,
        path: "notes/plan.md".into(),
        data: b"preview".to_vec(),
        size: 1_024,
        mime_type: Some("text/markdown".into()),
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ServiceToProcess = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ServiceToProcess::LogFileBoundary {
            id,
            action,
            path,
            data,
            size,
            mime_type,
        } => {
            assert_eq!(id, 101);
            assert_eq!(action, FileBoundaryAction::Import);
            assert_eq!(path, "notes/plan.md");
            assert_eq!(data, b"preview");
            assert_eq!(size, 1_024);
            assert_eq!(mime_type.as_deref(), Some("text/markdown"));
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
fn snapshot_status_roundtrip() {
    let msg = ProcessToService::SnapshotStatusResult {
        id: 7,
        status: super::SnapshotStatus {
            total: 1,
            auto_count: 1,
            manual_count: 0,
            manual_available: 12,
            snapshots: vec![super::SnapshotSlotStatus {
                checkpoint: "cp-0".into(),
                slot: 0,
                origin: "auto".into(),
                name: None,
                timestamp: "2026-06-11T00:00:00Z".into(),
                hash: None,
            }],
        },
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ProcessToService::SnapshotStatusResult { id, status } => {
            assert_eq!(id, 7);
            assert_eq!(status.total, 1);
            assert_eq!(status.snapshots[0].checkpoint, "cp-0");
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

#[test]
fn log_file_boundary_result_roundtrip() {
    let msg = ProcessToService::LogFileBoundaryResult {
        id: 101,
        success: false,
        data: Some(b"rewritten".to_vec()),
        error: Some("ledger failed".into()),
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ProcessToService::LogFileBoundaryResult {
            id,
            success,
            data,
            error,
        } => {
            assert_eq!(id, 101);
            assert!(!success);
            assert_eq!(data.as_deref(), Some(&b"rewritten"[..]));
            assert_eq!(error.as_deref(), Some("ledger failed"));
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
    let boundary = ServiceToProcess::LogFileBoundary {
        id: 4,
        action: FileBoundaryAction::Export,
        path: "/z".into(),
        data: vec![],
        size: 0,
        mime_type: None,
    };

    // Verify each preserves its own ID through serde
    let e: ServiceToProcess = serde_json::from_slice(&serde_json::to_vec(&exec).unwrap()).unwrap();
    let w: ServiceToProcess = serde_json::from_slice(&serde_json::to_vec(&write).unwrap()).unwrap();
    let r: ServiceToProcess = serde_json::from_slice(&serde_json::to_vec(&read).unwrap()).unwrap();
    let b: ServiceToProcess =
        serde_json::from_slice(&serde_json::to_vec(&boundary).unwrap()).unwrap();

    match (e, w, r, b) {
        (
            ServiceToProcess::Exec { id: e_id, .. },
            ServiceToProcess::WriteFile { id: w_id, .. },
            ServiceToProcess::ReadFile { id: r_id, .. },
            ServiceToProcess::LogFileBoundary { id: b_id, .. },
        ) => {
            assert_eq!(e_id, 1);
            assert_eq!(w_id, 2);
            assert_eq!(r_id, 3);
            assert_eq!(b_id, 4);
        }
        _ => panic!("wrong variants"),
    }
}

// -----------------------------------------------------------------------
// ReloadConfig
// -----------------------------------------------------------------------

#[test]
fn reload_config_roundtrip() {
    let msg = ServiceToProcess::ReloadConfig;
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ServiceToProcess = serde_json::from_slice(&bytes).unwrap();
    assert!(matches!(msg2, ServiceToProcess::ReloadConfig));
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

// -----------------------------------------------------------------------
// MCP IPC roundtrips
// -----------------------------------------------------------------------

#[test]
fn mcp_list_servers_roundtrip() {
    let msg = ServiceToProcess::McpListServers { id: 10 };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ServiceToProcess = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ServiceToProcess::McpListServers { id } => assert_eq!(id, 10),
        _ => panic!("wrong variant"),
    }
}

#[test]
fn mcp_list_tools_roundtrip() {
    let msg = ServiceToProcess::McpListTools { id: 20 };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ServiceToProcess = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ServiceToProcess::McpListTools { id } => assert_eq!(id, 20),
        _ => panic!("wrong variant"),
    }
}

#[test]
fn mcp_call_tool_roundtrip_bincode() {
    // Regression guard: bincode is the real IPC wire format (via
    // tokio-unix-ipc). When `arguments` was a `serde_json::Value` this
    // failed with "Bincode does not support deserialize_any". Keeping
    // the field as a JSON string means the payload is transparent to
    // bincode and capsem-process actually receives the message.
    let msg = ServiceToProcess::McpCallTool {
        id: 30,
        namespaced_name: "github__search".into(),
        arguments_json: serde_json::json!({"q": "rust"}).to_string(),
    };
    let bytes = bincode::serialize(&msg).unwrap();
    let msg2: ServiceToProcess = bincode::deserialize(&bytes).unwrap();
    match msg2 {
        ServiceToProcess::McpCallTool {
            id,
            namespaced_name,
            arguments_json,
        } => {
            assert_eq!(id, 30);
            assert_eq!(namespaced_name, "github__search");
            let parsed: serde_json::Value = serde_json::from_str(&arguments_json).unwrap();
            assert_eq!(parsed["q"], "rust");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn mcp_call_tool_result_roundtrip_bincode() {
    let msg = ProcessToService::McpCallToolResult {
        id: 30,
        result_json: Some(serde_json::json!({"items": [1, 2]}).to_string()),
        event_id: Some("abcdef123456".to_string()),
        security_rule_events_json: vec![serde_json::json!({
            "event_id": "abcdef123456",
            "event_type": "mcp.tool_call"
        })
        .to_string()],
        error: None,
    };
    let bytes = bincode::serialize(&msg).unwrap();
    let msg2: ProcessToService = bincode::deserialize(&bytes).unwrap();
    match msg2 {
        ProcessToService::McpCallToolResult {
            id,
            result_json,
            event_id,
            security_rule_events_json,
            error,
        } => {
            assert_eq!(id, 30);
            assert_eq!(event_id.as_deref(), Some("abcdef123456"));
            assert!(error.is_none());
            let parsed: serde_json::Value = serde_json::from_str(&result_json.unwrap()).unwrap();
            assert_eq!(parsed["items"], serde_json::json!([1, 2]));
            let row: serde_json::Value =
                serde_json::from_str(&security_rule_events_json[0]).unwrap();
            assert_eq!(row["event_type"], "mcp.tool_call");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn mcp_servers_result_roundtrip() {
    let msg = ProcessToService::McpServersResult {
        id: 10,
        servers: vec![McpServerStatus {
            name: "github".into(),
            url: "https://mcp.github.com".into(),
            enabled: true,
            source: "claude".into(),
            is_stdio: false,
            connected: true,
            tool_count: 5,
        }],
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ProcessToService::McpServersResult { id, servers } => {
            assert_eq!(id, 10);
            assert_eq!(servers.len(), 1);
            assert_eq!(servers[0].name, "github");
            assert!(servers[0].connected);
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn mcp_tools_result_roundtrip() {
    let msg = ProcessToService::McpToolsResult {
        id: 20,
        tools: vec![McpToolStatus {
            namespaced_name: "github__search".into(),
            original_name: "search".into(),
            description: Some("Search repos".into()),
            server_name: "github".into(),
            annotations: None,
        }],
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ProcessToService::McpToolsResult { id, tools } => {
            assert_eq!(id, 20);
            assert_eq!(tools[0].namespaced_name, "github__search");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn mcp_call_tool_result_roundtrip() {
    let msg = ProcessToService::McpCallToolResult {
        id: 30,
        result_json: Some(serde_json::json!({"content": []}).to_string()),
        event_id: Some("abcdef123456".to_string()),
        security_rule_events_json: vec![serde_json::json!({
            "event_id": "abcdef123456",
            "event_type": "mcp.tool_call"
        })
        .to_string()],
        error: None,
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ProcessToService::McpCallToolResult {
            id,
            result_json,
            event_id,
            security_rule_events_json,
            error,
        } => {
            assert_eq!(id, 30);
            assert!(result_json.is_some());
            assert_eq!(event_id.as_deref(), Some("abcdef123456"));
            assert_eq!(security_rule_events_json.len(), 1);
            assert!(error.is_none());
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn mcp_refresh_result_roundtrip() {
    let msg = ProcessToService::McpRefreshResult {
        id: 40,
        success: true,
        error: None,
    };
    let bytes = serde_json::to_vec(&msg).unwrap();
    let msg2: ProcessToService = serde_json::from_slice(&bytes).unwrap();
    match msg2 {
        ProcessToService::McpRefreshResult { id, success, error } => {
            assert_eq!(id, 40);
            assert!(success);
            assert!(error.is_none());
        }
        _ => panic!("wrong variant"),
    }
}
