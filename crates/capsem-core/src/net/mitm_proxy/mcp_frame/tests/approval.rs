use super::*;
use crate::security_engine::approval::client::{frame_limit, ApprovalCapability, ApprovalClient};
use capsem_foundation::ipc_channel;
use capsem_proto::approval::{ApprovalClientMessage, ApprovalOutcome, ApprovalServerMessage};

fn endpoint() -> (
    Arc<McpEndpointState>,
    GatedDriver,
    ipc_channel::Sender<ApprovalServerMessage>,
    ipc_channel::Receiver<ApprovalClientMessage>,
) {
    let profile = crate::net::policy_config::SecurityRuleProfile::parse_toml(
        r#"
        [profiles.rules.ask_mcp]
        name = "ask_mcp"
        action = "ask"
        match = 'mcp.method == "tools/call" && mcp.response.valid == "false"'
    "#,
    )
    .unwrap();
    let rules =
        SecurityRuleSet::compile_profile(&profile, crate::net::policy_config::SecurityRuleSource::User).unwrap();
    let (endpoint, driver) = gated_endpoint_with_rules(1, true, rules);
    let (worker, service) = std::os::unix::net::UnixStream::pair().unwrap();
    let capability = Arc::new(ApprovalCapability::connected(
        ApprovalClient::from_stream(worker).unwrap(),
    ));
    let endpoint = Arc::new(Arc::try_unwrap(endpoint).ok().unwrap().with_approvals(Some(capability)));
    let (sender, receiver) = ipc_channel::channel_from_std_with_limit(service, frame_limit()).unwrap();
    (endpoint, driver, sender, receiver)
}

#[tokio::test]
async fn original_mcp_approval_identity_reaches_physical_tool_ledger() {
    for outcome in [ApprovalOutcome::Approved, ApprovalOutcome::Denied] {
        for transport in [McpTransport::Direct, McpTransport::VsockFrame] {
            let (endpoint, driver, sender, receiver) = endpoint();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("session.db");
            let db = Arc::new(DbWriter::open(&path, 64).unwrap());
            let call = tokio::spawn(dispatch_logged_mcp_request(
                endpoint,
                Arc::clone(&db),
                request("tools/call", json!({"name":"local__slow"})),
                "service".into(),
                transport,
            ));
            let ApprovalClientMessage::Begin { request_id, request } =
                tokio::time::timeout(Duration::from_secs(2), receiver.recv())
                    .await
                    .unwrap()
                    .unwrap()
            else {
                panic!("begin")
            };
            assert_eq!(driver.started.load(Ordering::SeqCst), 0);
            sender
                .send(ApprovalServerMessage::Resolved { request_id, outcome })
                .await
                .unwrap();
            if outcome == ApprovalOutcome::Approved {
                wait_for_started(&driver, 1).await;
                driver.release.send(true).unwrap();
            }
            let result = tokio::time::timeout(Duration::from_secs(2), call)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(
                driver.started.load(Ordering::SeqCst),
                usize::from(outcome == ApprovalOutcome::Approved)
            );
            db.flush_checked().await.unwrap();
            let conn = rusqlite::Connection::open(&path).unwrap();
            let linked: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM tool_calls t JOIN security_ask_events a ON a.event_id=t.event_id \
                 WHERE a.ask_id=? AND a.status='pending' AND t.event_id=?",
                    rusqlite::params![request.ask_id, request.event_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(linked, 1, "MCP telemetry lost the original approval identity");
            assert_eq!(result.event_id.as_deref(), Some(request.event_id.as_str()));
        }
    }
}

#[tokio::test]
async fn direct_connection_cancellation_denies_original_mcp_without_executing_tool() {
    let (endpoint, driver, sender, receiver) = endpoint();
    let db = Arc::new(DbWriter::open_in_memory(64).unwrap());
    let cancellation = tokio_util::sync::CancellationToken::new();
    let call = tokio::spawn(dispatch_cancellable_mcp_request(
        Arc::clone(&endpoint),
        db,
        request("tools/call", json!({"name":"local__slow"})),
        "service".into(),
        McpTransport::Direct,
        cancellation.clone(),
    ));
    let ApprovalClientMessage::Begin { request_id, .. } = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!("begin")
    };
    cancellation.cancel();
    let response = tokio::time::timeout(Duration::from_secs(2), call)
        .await
        .expect("original connection cancellation ends its approval wait")
        .unwrap()
        .unwrap();
    assert!(response.response.error.is_some());
    assert_eq!(driver.started.load(Ordering::SeqCst), 0);
    assert!(
        matches!(tokio::time::timeout(Duration::from_secs(2), receiver.recv()).await.unwrap().unwrap(),
        ApprovalClientMessage::Cancel { request_id: id } if id == request_id)
    );
    sender
        .send(ApprovalServerMessage::Resolved {
            request_id,
            outcome: ApprovalOutcome::Approved,
        })
        .await
        .unwrap();
    assert_eq!(driver.started.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn tiny_json_values_cannot_bypass_the_retained_body_budget() {
    let (endpoint, driver, _sender, _receiver) = endpoint();
    let db = Arc::new(DbWriter::open_in_memory(64).unwrap());
    let request = request("tools/call", json!({"name":"local__slow", "arguments":vec![0; 50_000]}));
    assert!(serde_json::to_vec(&request).unwrap().len() < capsem_proto::APPROVAL_HELD_BYTES_PER_REQUEST);
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        dispatch_logged_mcp_request(endpoint, db, request, "agent".into(), McpTransport::Direct),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(response.response.error.is_some());
    assert_eq!(driver.started.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn direct_mcp_shares_the_endpoint_retained_body_admission() {
    let (endpoint, driver, _sender, _receiver) = endpoint();
    let _held = endpoint.admit_frame(capsem_proto::APPROVAL_HELD_BYTES_TOTAL).unwrap();
    let db = Arc::new(DbWriter::open_in_memory(64).unwrap());
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        dispatch_logged_mcp_request(
            endpoint,
            db,
            request("tools/call", json!({"name":"local__slow"})),
            "service".into(),
            McpTransport::Direct,
        ),
    )
    .await
    .expect("overload must fail closed before creating an approval wait")
    .unwrap();
    assert!(response.response.error.is_some());
    assert_eq!(driver.started.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn disconnect_after_tool_started_drains_the_original_execution_audit() {
    let (endpoint, driver) = gated_endpoint(1, true);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let db = Arc::new(DbWriter::open(&path, 64).unwrap());
    let cancellation = tokio_util::sync::CancellationToken::new();
    let call = tokio::spawn(dispatch_cancellable_mcp_request(
        endpoint,
        Arc::clone(&db),
        request("tools/call", json!({"name":"local__slow"})),
        "service".into(),
        McpTransport::Direct,
        cancellation.clone(),
    ));
    tokio::time::timeout(Duration::from_secs(2), async {
        while driver.started.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    cancellation.cancel();
    driver.release.send(true).unwrap();
    let response = tokio::time::timeout(Duration::from_secs(2), call)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(response.event_id.is_some());
    assert_eq!(driver.started.load(Ordering::SeqCst), 1);
    db.flush_checked().await.unwrap();
    let reader = capsem_logger::DbReader::open(&path).unwrap();
    assert_eq!(
        count(&reader, "tool_calls"),
        1,
        "disconnect cannot erase an executed tool's audit"
    );
}

#[tokio::test]
async fn queued_frames_fail_closed_at_capacity_and_release_owned_memory() {
    let (endpoint, driver) = gated_endpoint(1, true);
    let db = Arc::new(DbWriter::open_in_memory(64).unwrap());
    let (mut guest, server) = tokio::io::duplex(1 << 16);
    let serve = tokio::spawn(serve_io(Vec::new(), server, Arc::clone(&endpoint), db));
    for id in 1..=(capsem_proto::MCP_WORKER_QUEUE_CAPACITY + 1) {
        if guest.write_all(&tool_call_request(id as u32)).await.is_err() {
            break;
        }
    }
    let error = tokio::time::timeout(Duration::from_secs(2), serve)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.2.contains("capacity"), "{error:?}");
    assert!(driver.started.load(Ordering::SeqCst) <= 1);
    assert!(
        endpoint.admit_frame(capsem_proto::APPROVAL_HELD_BYTES_TOTAL).is_some(),
        "disconnect must return the whole byte budget"
    );
    driver.release.send(true).unwrap();
}

#[tokio::test]
async fn human_mcp_waits_do_not_hold_execution_permits() {
    let (endpoint, driver, _sender, receiver) = endpoint();
    let db = Arc::new(DbWriter::open_in_memory(64).unwrap());
    let (mut guest, server) = tokio::io::duplex(1 << 16);
    let serve = tokio::spawn(serve_io(Vec::new(), server, endpoint, db));
    guest.write_all(&tool_call_request(1)).await.unwrap();
    guest.write_all(&tool_call_request(2)).await.unwrap();
    for _ in 0..2 {
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), receiver.recv())
                .await
                .unwrap()
                .unwrap(),
            ApprovalClientMessage::Begin { .. }
        ));
    }
    assert_eq!(driver.started.load(Ordering::SeqCst), 0);
    drop(guest);
    tokio::time::timeout(Duration::from_secs(2), serve)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn actual_disconnect_cancels_held_mcp_before_tool_dispatch() {
    let (endpoint, driver, sender, receiver) = endpoint();
    let db = Arc::new(DbWriter::open_in_memory(64).unwrap());
    let (mut guest, server) = tokio::io::duplex(1 << 16);
    let serve = tokio::spawn(serve_io(Vec::new(), server, endpoint, db));
    guest.write_all(&tool_call_request(1)).await.unwrap();
    let ApprovalClientMessage::Begin { request_id, request } =
        tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("begin")
    };
    assert_eq!(request.event_type, "mcp.tool_call");
    assert_eq!(driver.started.load(Ordering::SeqCst), 0);
    drop(guest);
    tokio::time::timeout(Duration::from_secs(2), serve)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(tokio::time::timeout(Duration::from_secs(2), receiver.recv()).await.unwrap().unwrap(), ApprovalClientMessage::Cancel { request_id: cancelled } if cancelled == request_id)
    );
    let _ = sender
        .send(ApprovalServerMessage::Resolved {
            request_id,
            outcome: ApprovalOutcome::Approved,
        })
        .await;
    assert_eq!(
        driver.started.load(Ordering::SeqCst),
        0,
        "late approval cannot replay the disconnected original"
    );
}

#[tokio::test]
async fn graceful_end_keeps_original_mcp_waiter_until_approved_and_drained() {
    let (endpoint, driver, sender, receiver) = endpoint();
    let db = Arc::new(DbWriter::open_in_memory(64).unwrap());
    let (mut guest, server) = tokio::io::duplex(1 << 16);
    let serve = tokio::spawn(serve_io(Vec::new(), server, endpoint, db));
    guest.write_all(&tool_call_request(1)).await.unwrap();
    guest.write_all(&capsem_proto::MCP_SESSION_END).await.unwrap();
    let ApprovalClientMessage::Begin { request_id, .. } = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!("begin")
    };
    assert_eq!(driver.started.load(Ordering::SeqCst), 0);
    sender
        .send(ApprovalServerMessage::Resolved {
            request_id,
            outcome: ApprovalOutcome::Approved,
        })
        .await
        .unwrap();
    wait_for_started(&driver, 1).await;
    driver.release.send(true).unwrap();
    let response = read_reply(&mut guest).await;
    assert!(response.error.is_none(), "{response:?}");
    tokio::time::timeout(Duration::from_secs(2), serve)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(driver.started.load(Ordering::SeqCst), 1);
}
