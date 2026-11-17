use super::*;

#[tokio::test]
async fn mcp_transport_bounds_upstream_boundary_survives_correlation_remapping() {
    let mut upstream = ok_response(json!(""));
    let overhead = serde_json::to_vec(&upstream).unwrap().len();
    upstream.result = Some(json!("x".repeat(capsem_proto::MCP_PAYLOAD_MAX_BYTES - overhead)));
    let accepted = serde_json::to_vec(&upstream).unwrap();
    assert_eq!(accepted.len(), capsem_proto::MCP_PAYLOAD_MAX_BYTES);
    let mut response: JsonRpcResponse = serde_json::from_slice(&accepted).unwrap();
    response.id = Some(json!("\0".repeat(capsem_proto::MCP_REQUEST_ID_MAX_BYTES)));
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let process = "x".repeat(capsem_proto::MCP_FRAME_MAX_PROCESS_NAME_LEN);
    send_response(&tx, 99, &process, &response).await.unwrap();
    let frame = rx.recv().await.unwrap();
    let decoded: JsonRpcResponse = serde_json::from_slice(&frame.payload).unwrap();
    assert!(
        decoded.error.is_none(),
        "accepted upstream payload failed downstream: {:?}",
        decoded.error
    );
    assert_eq!(decoded.result, upstream.result);
    assert_eq!(decoded.id, response.id);
    capsem_proto::encode_mcp_frame(99, 0, &process, &frame.payload).unwrap();
}

struct PayloadIdentityWriter {
    expected: usize,
    body_len: usize,
    saw_original: bool,
}
impl tokio::io::AsyncWrite for PayloadIdentityWriter {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.saw_original |= bytes.as_ptr() as usize == self.expected && bytes.len() == self.body_len;
        std::task::Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn mcp_transport_bounds_frame_writer_borrows_the_existing_payload_buffer() {
    let frame = OutboundFrame {
        stream_id: 9,
        process_name: "agent".into(),
        payload: b"complete payload".repeat(64),
    };
    let mut writer = PayloadIdentityWriter {
        expected: frame.payload.as_ptr() as usize,
        body_len: frame.payload.len(),
        saw_original: false,
    };
    write_frame(&mut writer, &frame).await.unwrap();
    assert!(writer.saw_original, "frame writing must not allocate a payload copy");
}

fn invalid_ids() -> Vec<serde_json::Value> {
    vec![
        json!("x".repeat(capsem_proto::MCP_REQUEST_ID_MAX_BYTES + 1)),
        json!(null),
        json!(true),
        json!([]),
        json!({}),
        json!(1.25),
    ]
}

#[test]
fn mcp_transport_bounds_parser_rejects_unrepresentable_correlation_metadata() {
    for id in invalid_ids() {
        let payload =
            serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"local__slow"}}))
                .unwrap();
        let error =
            parse_json_rpc_payload(&payload).expect_err("invalid correlation metadata must fail before dispatch");
        assert_eq!(error.code, -32600);
        assert!(error.id.is_none());
    }
}

#[tokio::test]
async fn mcp_transport_bounds_invalid_ids_never_dispatch_and_valid_ids_still_work() {
    for transport in [McpTransport::Direct, McpTransport::VsockFrame, McpTransport::Http] {
        let (endpoint, driver) = gated_endpoint_with_rules(1, false, matching_mcp_rules());
        let db = Arc::new(DbWriter::open_in_memory(64).unwrap());
        for id in invalid_ids() {
            let mut input = request("tools/call", json!({"name":"local__slow","arguments":{}}));
            input.id = Some(id);
            let reply =
                dispatch_logged_mcp_request(Arc::clone(&endpoint), Arc::clone(&db), input, "agent".into(), transport)
                    .await
                    .unwrap();
            assert!(reply.response.result.is_none());
            assert_eq!(reply.response.error.unwrap().code, -32600);
            assert_eq!(driver.started.load(Ordering::SeqCst), 0);
        }
        let id = "\0".repeat(capsem_proto::MCP_REQUEST_ID_MAX_BYTES);
        let mut input = request(
            "tools/call",
            json!({"name":"local__slow","arguments":{"marker":"complete"}}),
        );
        input.id = Some(json!(id));
        let reply = dispatch_logged_mcp_request(endpoint, db, input, "agent".into(), transport)
            .await
            .unwrap();
        assert!(reply.response.error.is_none());
        assert_eq!(reply.response.id, Some(json!(id)));
        assert_eq!(driver.started.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn mcp_transport_bounds_reserve_maximum_attribution_for_every_reply() {
    let limit = capsem_proto::MCP_PAYLOAD_MAX_BYTES;
    let overhead = serde_json::to_vec(&ok_response(json!(""))).unwrap().len() - b",\"id\":1".len();
    let complete = ok_response(json!("x".repeat(limit - overhead)));
    for process in [
        "codex".to_owned(),
        "x".repeat(capsem_proto::MCP_FRAME_MAX_PROCESS_NAME_LEN),
    ] {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        send_response(&tx, 9, &process, &complete).await.unwrap();
        let accepted = rx.recv().await.unwrap();
        assert_eq!(accepted.payload.len(), limit + b",\"id\":1".len());
        let decoded: JsonRpcResponse = serde_json::from_slice(&accepted.payload).unwrap();
        assert_eq!(decoded.result, complete.result);
        capsem_proto::encode_mcp_frame(9, 0, &process, &accepted.payload).unwrap();
        let oversized = ok_response(json!("x".repeat(limit - overhead + 1)));
        send_response(&tx, 10, &process, &oversized).await.unwrap();
        let refused = rx.recv().await.unwrap();
        let decoded: JsonRpcResponse = serde_json::from_slice(&refused.payload).unwrap();
        assert!(decoded.result.is_none());
        assert!(decoded.error.unwrap().message.contains("tool may already have run"));
        assert_eq!(decoded.id, Some(json!(1)));
        capsem_proto::encode_mcp_frame(10, 0, &process, &refused.payload).unwrap();
    }
}
