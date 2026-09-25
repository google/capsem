use super::*;

/// A reply is matched to its request by id, not by arrival order: every IPC
/// connection also receives lifecycle broadcasts, and the service once took a
/// `ShutdownRequested` as the answer to an exec.
#[test]
fn replies_correlate_to_requests_by_id_and_broadcasts_answer_nothing() {
    let requests = [
        (
            ServiceToProcess::Exec {
                id: 7,
                command: "true".into(),
            },
            Some(7),
        ),
        (
            ServiceToProcess::ReadFile {
                id: 8,
                path: "/x".into(),
            },
            Some(8),
        ),
        (ServiceToProcess::McpRefreshTools { id: 10 }, Some(10)),
        (
            ServiceToProcess::CloneState {
                id: 12,
                destination: "/tmp/fork".into(),
            },
            Some(12),
        ),
        (
            ServiceToProcess::LinkDetach {
                id: 11,
                network: "n".into(),
                generation: 1,
            },
            Some(11),
        ),
        (
            ServiceToProcess::AdmitContainerPull {
                id: 12,
                image: "registry.example/app:1".into(),
                registry: "registry.example".into(),
                digest: None,
            },
            Some(12),
        ),
        (ServiceToProcess::Ping, None),
        (ServiceToProcess::ReloadConfig { id: 13 }, Some(13)),
    ];
    for (request, id) in requests {
        assert_eq!(request.request_id(), id, "{request:?}");
    }
    let replies = [
        (
            ProcessToService::ExecResult {
                id: 7,
                stdout: vec![],
                stderr: vec![],
                exit_code: 0,
                truncated: false,
            },
            Some(7),
        ),
        (ProcessToService::LinkDetachResult { id: 11, error: None }, Some(11)),
        (ProcessToService::ShutdownRequested { id: "vm".into() }, None),
        (ProcessToService::SuspendRequested { id: "vm".into() }, None),
        (
            ProcessToService::SuspendFailed {
                id: "vm".into(),
                error: "e".into(),
            },
            None,
        ),
        (
            ProcessToService::StateChanged {
                id: "vm".into(),
                state: "s".into(),
                trigger: "t".into(),
            },
            None,
        ),
        (
            ProcessToService::ExecOutput {
                id: 7,
                channel: crate::ExecOutputChannel::Stdout,
                data: vec![1],
            },
            None,
        ),
        (
            ProcessToService::ContainerPullAdmission {
                id: 12,
                error: None,
                policy_refused: false,
            },
            Some(12),
        ),
        (
            ProcessToService::ConfigReloadResult {
                id: 13,
                active_profile_digest: Some("blake3:00".into()),
                error: None,
            },
            Some(13),
        ),
        (ProcessToService::Pong, None),
    ];
    for (reply, id) in replies {
        assert_eq!(reply.reply_id(), id, "{reply:?}");
    }
}

/// The owner fills tool annotations, so the typed field must survive the real
/// MessagePack codec, not only the public JSON projection.
#[test]
fn mcp_tool_status_annotations_roundtrip_msgpack() {
    let msg = ProcessToService::McpToolsResult {
        id: 21,
        tools: vec![McpToolStatus {
            namespaced_name: "github__search".into(),
            original_name: "search".into(),
            description: None,
            server_name: "github".into(),
            annotations: Some(crate::mcp_contracts::ToolAnnotations {
                title: Some("Search".into()),
                read_only_hint: true,
                destructive_hint: false,
                idempotent_hint: true,
                open_world_hint: true,
            }),
        }],
    };
    let bytes = rmp_serde::to_vec_named(&msg).unwrap();
    let decoded: ProcessToService = rmp_serde::from_slice(&bytes).expect("annotations decode over MessagePack");
    let ProcessToService::McpToolsResult { tools, .. } = decoded else {
        panic!("wrong variant");
    };
    let annotations = tools[0].annotations.as_ref().expect("annotations survive");
    assert!(annotations.read_only_hint && !annotations.destructive_hint);
    assert_eq!(annotations.title.as_deref(), Some("Search"));
}
