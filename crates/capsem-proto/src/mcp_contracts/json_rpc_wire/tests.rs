use super::*;

#[test]
fn content_at_the_ceiling_survives_maximum_correlation_and_attribution() {
    let overhead = crate::framing::encode_json_record(
        &Content(&JsonRpcResponse::ok(None, serde_json::json!(""))),
        MCP_PAYLOAD_MAX_BYTES,
    )
    .unwrap()
    .len();
    for id in [
        serde_json::json!("\0".repeat(crate::MCP_REQUEST_ID_MAX_BYTES)),
        serde_json::json!(u64::MAX),
        serde_json::json!(i64::MIN),
    ] {
        let response = JsonRpcResponse::ok(
            Some(id),
            serde_json::json!("x".repeat(MCP_PAYLOAD_MAX_BYTES - overhead)),
        );
        let bytes = encode_response(&response).unwrap();
        assert!(bytes.len() > MCP_PAYLOAD_MAX_BYTES);
        assert!(bytes.capacity() <= MCP_JSON_RPC_MAX_BYTES);
        validate_size(&bytes).unwrap();
        let decoded: JsonRpcResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded.result, response.result);
        assert_eq!(decoded.id, response.id);
        crate::encode_mcp_frame(1, 0, &"x".repeat(crate::MCP_FRAME_MAX_PROCESS_NAME_LEN), &bytes).unwrap();
    }
}

#[test]
fn one_extra_content_byte_is_refused_regardless_of_correlation() {
    let overhead = crate::framing::encode_json_record(
        &Content(&JsonRpcResponse::ok(None, serde_json::json!(""))),
        MCP_PAYLOAD_MAX_BYTES,
    )
    .unwrap()
    .len();
    let response = JsonRpcResponse::ok(
        Some(serde_json::json!(1)),
        serde_json::json!("x".repeat(MCP_PAYLOAD_MAX_BYTES - overhead + 1)),
    );
    assert!(crate::framing::is_frame_too_large(
        &encode_response(&response).unwrap_err()
    ));
    assert!(validate_size(&serde_json::to_vec(&response).unwrap()).is_err());
}

#[test]
fn unused_identity_allowance_cannot_admit_oversized_content_or_duplicate_ids() {
    let content = "x".repeat(MCP_PAYLOAD_MAX_BYTES);
    for input in [
        format!(r#"{{"jsonrpc":"2.0","result":"{content}"}}"#),
        format!(r#"{{"jsonrpc":"2.0","id":1,"id":2,"result":"{content}"}}"#),
        format!(r#"{{"jsonrpc":"2.0","id":[],"result":"{content}"}}"#),
        format!("{}{{", " ".repeat(MCP_PAYLOAD_MAX_BYTES)),
    ] {
        assert!(validate_size(input.as_bytes()).is_err());
    }
}

#[test]
fn unknown_error_identity_is_explicit_null_and_response_fields_survive() {
    let response = JsonRpcResponse::err(None, -32700, "parse error");
    let bytes = encode_response(&response).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(value.get("id").unwrap().is_null());
    assert_eq!(value["error"]["message"], "parse error");
    let mut response = JsonRpcResponse::ok(Some(serde_json::json!("id")), serde_json::json!({"complete":"🧪"}));
    response.meta = Some(super::super::JsonRpcMeta {
        traceparent: "trace".into(),
        tracestate: "state".into(),
    });
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&encode_response(&response).unwrap()).unwrap(),
        serde_json::to_value(response).unwrap()
    );
}
