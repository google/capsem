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
        .send(ApprovalServerMessage {
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
        .send(ApprovalServerMessage {
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
