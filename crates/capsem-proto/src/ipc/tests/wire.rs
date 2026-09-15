//! Production MessagePack wire contract.

use super::*;

// -----------------------------------------------------------------------
// Production MessagePack wire contract
// -----------------------------------------------------------------------

fn assert_rmp_variant_name<T>(message: &T, expected: &str)
where
    T: serde::Serialize + for<'de> serde::Deserialize<'de>,
{
    let bytes = rmp_serde::to_vec_named(message).expect("serialize production IPC frame");
    let encoded_name = rmp_serde::to_vec_named(expected).unwrap();
    assert!(
        bytes.windows(encoded_name.len()).any(|window| window == encoded_name),
        "MessagePack frame does not carry the stable variant name {expected:?}: {bytes:02x?}"
    );

    let decoded: T = rmp_serde::from_slice(&bytes).expect("deserialize production IPC frame");
    assert_eq!(
        rmp_serde::to_vec_named(&decoded).unwrap(),
        bytes,
        "MessagePack payload did not round-trip exactly"
    );
}

#[test]
fn service_to_process_variant_names_and_roundtrips_are_stable() {
    let messages = vec![
        ServiceToProcess::Ping,
        ServiceToProcess::TerminalInput { data: vec![1] },
        ServiceToProcess::TerminalResize { cols: 80, rows: 24 },
        ServiceToProcess::Shutdown,
        ServiceToProcess::Exec {
            id: 1,
            command: "true".into(),
        },
        ServiceToProcess::WriteFile {
            id: 2,
            path: "/tmp/x".into(),
            data: vec![2],
        },
        ServiceToProcess::ReadFile {
            id: 3,
            path: "/tmp/x".into(),
        },
        ServiceToProcess::LogFileBoundary {
            id: 4,
            action: FileBoundaryAction::Import,
            path: "/tmp/x".into(),
            data: vec![3],
            size: 1,
            mime_type: None,
        },
        ServiceToProcess::ReloadConfig,
        ServiceToProcess::StartTerminalStream,
        ServiceToProcess::StopTerminalStream,
        ServiceToProcess::PrepareSnapshot,
        ServiceToProcess::Unfreeze,
        ServiceToProcess::Suspend {
            checkpoint_path: "/tmp/checkpoint".into(),
        },
        ServiceToProcess::Resume,
        ServiceToProcess::McpListServers { id: 5 },
        ServiceToProcess::McpListTools { id: 6 },
        ServiceToProcess::McpRefreshTools { id: 7 },
        ServiceToProcess::SnapshotStatus { id: 8 },
        ServiceToProcess::McpCallTool {
            id: 9,
            namespaced_name: "server__tool".into(),
            arguments_json: "{}".into(),
        },
        ServiceToProcess::ExecStream {
            id: 10,
            command: "printf live".into(),
        },
    ];

    let names = [
        "Ping",
        "TerminalInput",
        "TerminalResize",
        "Shutdown",
        "Exec",
        "WriteFile",
        "ReadFile",
        "LogFileBoundary",
        "ReloadConfig",
        "StartTerminalStream",
        "StopTerminalStream",
        "PrepareSnapshot",
        "Unfreeze",
        "Suspend",
        "Resume",
        "McpListServers",
        "McpListTools",
        "McpRefreshTools",
        "SnapshotStatus",
        "McpCallTool",
        "ExecStream",
    ];
    assert_eq!(messages.len(), names.len());
    for (message, expected) in messages.iter().zip(names) {
        assert_rmp_variant_name(message, expected);
    }
}

#[test]
fn process_to_service_variant_names_and_roundtrips_are_stable() {
    let messages = vec![
        ProcessToService::Pong,
        ProcessToService::TerminalOutput { data: vec![1] },
        ProcessToService::StateChanged {
            id: "vm".into(),
            state: "Running".into(),
            trigger: "booted".into(),
        },
        ProcessToService::ExecResult {
            id: 1,
            stdout: vec![2],
            stderr: vec![],
            exit_code: 0,
            truncated: false,
        },
        ProcessToService::WriteFileResult {
            id: 2,
            success: true,
            error: None,
        },
        ProcessToService::ReadFileResult {
            id: 3,
            data: Some(vec![3]),
            error: None,
        },
        ProcessToService::LogFileBoundaryResult {
            id: 4,
            success: true,
            data: None,
            error: None,
        },
        ProcessToService::ShutdownRequested { id: "vm".into() },
        ProcessToService::SuspendRequested { id: "vm".into() },
        ProcessToService::SnapshotReady { id: "vm".into() },
        ProcessToService::McpServersResult { id: 5, servers: vec![] },
        ProcessToService::McpToolsResult { id: 6, tools: vec![] },
        ProcessToService::McpRefreshResult {
            id: 7,
            success: true,
            error: None,
        },
        ProcessToService::SnapshotStatusResult {
            id: 8,
            status: SnapshotStatus {
                total: 0,
                auto_count: 0,
                manual_count: 0,
                manual_available: 0,
                snapshots: vec![],
            },
        },
        ProcessToService::McpCallToolResult {
            id: 9,
            result_json: Some("{}".into()),
            event_id: None,
            error: None,
        },
        ProcessToService::SuspendFailed {
            id: "vm".into(),
            error: "failed".into(),
        },
        ProcessToService::ExecOutput {
            id: 10,
            data: vec![0, 255, 10],
        },
    ];

    let names = [
        "Pong",
        "TerminalOutput",
        "StateChanged",
        "ExecResult",
        "WriteFileResult",
        "ReadFileResult",
        "LogFileBoundaryResult",
        "ShutdownRequested",
        "SuspendRequested",
        "SnapshotReady",
        "McpServersResult",
        "McpToolsResult",
        "McpRefreshResult",
        "SnapshotStatusResult",
        "McpCallToolResult",
        "SuspendFailed",
        "ExecOutput",
    ];
    assert_eq!(messages.len(), names.len());
    for (message, expected) in messages.iter().zip(names) {
        assert_rmp_variant_name(message, expected);
    }
}

#[test]
fn named_variants_decode_independently_of_source_order() {
    #[derive(serde::Serialize)]
    enum Producer {
        Second { value: u8 },
        First,
    }
    #[derive(serde::Deserialize)]
    enum Consumer {
        First,
        Second { value: u8 },
    }

    let bytes = rmp_serde::to_vec_named(&Producer::Second { value: 7 }).unwrap();
    let decoded: Consumer = rmp_serde::from_slice(&bytes).unwrap();
    assert!(matches!(decoded, Consumer::Second { value: 7 }));
    let _ = Producer::First;
    let _ = Consumer::First;
}

#[test]
fn ipc_byte_payloads_use_messagepack_binary() {
    let ascii = ServiceToProcess::TerminalInput {
        data: vec![b'a'; 1024 * 1024],
    };
    let binary = ServiceToProcess::TerminalInput {
        data: vec![0xff; 1024 * 1024],
    };
    let ascii = rmp_serde::to_vec_named(&ascii).unwrap();
    let binary = rmp_serde::to_vec_named(&binary).unwrap();

    assert_eq!(ascii.len(), binary.len());
    assert!(
        binary.len() < 1024 * 1024 + 64,
        "binary frame expanded to {}",
        binary.len()
    );
    let decoded: ServiceToProcess = rmp_serde::from_slice(&binary).unwrap();
    assert!(matches!(decoded, ServiceToProcess::TerminalInput { data } if data == vec![0xff; 1024 * 1024]));
}
