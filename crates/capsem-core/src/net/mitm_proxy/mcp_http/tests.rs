use super::*;

#[tokio::test]
async fn mcp_transport_bounds_http_rejects_oversized_results_without_truncated_success() {
    use capsem_proto::mcp_contracts::JsonRpcResponse;
    let response = JsonRpcResponse::ok(
        Some(serde_json::json!(7)),
        serde_json::json!({"content":"x".repeat(capsem_proto::MCP_FRAME_MAX_SIZE)}),
    );
    let reply = json_reply(200, &response);
    assert_eq!(reply.status(), 200);
    let body = reply.into_body().collect().await.unwrap().to_bytes();
    let decoded: JsonRpcResponse = serde_json::from_slice(&body).unwrap();
    assert!(decoded.result.is_none());
    assert_eq!(decoded.id, Some(serde_json::json!(7)));
    assert!(decoded.error.unwrap().message.contains("tool may already have run"));
    let complete = JsonRpcResponse::ok(
        Some(serde_json::json!(8)),
        serde_json::json!({"content":"complete 🧪","nested":[true,42]}),
    );
    let body = json_reply(200, &complete)
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let decoded: JsonRpcResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(decoded.result, complete.result);
    assert_eq!(decoded.id, complete.id);
}

#[test]
fn the_exact_name_on_its_default_port_is_the_gateway() {
    assert_eq!(
        internal_route("mcp.capsem.internal", 80, Protocol::Http),
        Some(InternalRoute::McpGateway)
    );
    assert_eq!(
        internal_route("mcp.capsem.internal", 443, Protocol::Tls),
        Some(InternalRoute::McpGateway)
    );
    // The proxy hands this function the normalized host, but a raw spelling
    // must not change the answer either way.
    assert_eq!(
        internal_route("MCP.Capsem.Internal.", 80, Protocol::Http),
        Some(InternalRoute::McpGateway)
    );
}

#[test]
fn another_port_on_the_gateway_name_is_refused_not_dialed() {
    for (port, protocol) in [
        (8080, Protocol::Http),
        (443, Protocol::Http),
        (80, Protocol::Tls),
        (11434, Protocol::Http),
        (0, Protocol::Http),
    ] {
        assert_eq!(
            internal_route("mcp.capsem.internal", port, protocol),
            Some(InternalRoute::Refused),
            "{protocol:?} :{port}"
        );
    }
    assert_eq!(
        internal_route("mcp.capsem.internal", 80, Protocol::Unknown),
        Some(InternalRoute::Refused)
    );
}

#[test]
fn every_other_name_in_the_zone_is_refused() {
    for host in [
        "capsem.internal",
        "xmcp.capsem.internal",
        "mcp.team.capsem.internal",
        "beta.capsem.internal",
        "a.mcp.capsem.internal",
    ] {
        assert_eq!(
            internal_route(host, 80, Protocol::Http),
            Some(InternalRoute::Refused),
            "{host}"
        );
    }
}

#[test]
fn lookalikes_outside_the_zone_are_ordinary_hosts() {
    for host in [
        "mcp.capsem.internal.example.com",
        "mcp-capsem.internal",
        "mcpcapsem.internal",
        "notcapsem.internal",
        "mcp.capsem.internals",
        "example.com",
    ] {
        assert_eq!(internal_route(host, 80, Protocol::Http), None, "{host}");
    }
}
