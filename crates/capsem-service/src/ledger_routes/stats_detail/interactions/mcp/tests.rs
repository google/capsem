use super::*;
use serde_json::json;

fn row(arguments: &str, response: &str) -> ToolRow {
    serde_json::from_value(json!({
        "event_id":"abcdef000001", "timestamp":"now", "model_call_id":null, "model_event_id":null,
        "trace_id":"trace", "turn_id":null, "call_id":"call", "tool_name":"search", "server_name":"knowledge",
        "origin":"mcp", "decision":"allowed", "method":"tools/call", "arguments":arguments,
        "response_preview":response, "error_message":null,
    }))
    .unwrap()
}

#[test]
fn observed_mcp_envelopes_are_preserved_but_not_mistaken_for_arguments_or_results() {
    let request = r#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"search","arguments":{"q":[true,null]}}}"#;
    let response = r#"{"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"found"}],"isError":false}}"#;
    let call = tool_content(&row(request, response));
    assert!(call.request.is_some());
    let CapturedContent::Json(args) = call.arguments.unwrap().content else {
        panic!("expected JSON")
    };
    assert_eq!(args.value, json!({"q":[true,null]}));
    let result = call.result.unwrap();
    assert_eq!(result.is_error, Some(false));
    assert!(result.response.is_some());
    let CapturedContent::Json(value) = result.payload.unwrap().content else {
        panic!("expected JSON")
    };
    assert_eq!(
        value.value,
        json!({"content":[{"type":"text","text":"found"}],"isError":false})
    );
}

#[test]
fn null_missing_malformed_and_error_envelopes_retain_their_distinctions() {
    let request = r#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"search","arguments":null}}"#;
    let call = tool_content(&row(request, r#"{"jsonrpc":"2.0","result":null}"#));
    let CapturedContent::Json(args) = call.arguments.unwrap().content else {
        panic!("expected JSON null")
    };
    assert_eq!(args.value, json!(null));
    assert!(matches!(
        call.result.unwrap().payload.unwrap().content,
        CapturedContent::Json(_)
    ));
    let missing = request.replace(",\"arguments\":null", "");
    let call = tool_content(&row(
        &missing,
        r#"{"jsonrpc":"2.0","error":{"code":-1,"message":"denied"}}"#,
    ));
    assert!(call.arguments.is_none());
    let result = call.result.unwrap();
    assert_eq!(result.is_error, Some(true));
    assert_eq!(result.error_message.as_deref(), Some("denied"));
    assert!(result.payload.is_none() && result.response.is_some());
    for request in [
        "{broken",
        r#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"different"}}"#,
    ] {
        let call = tool_content(&row(request, "{broken"));
        assert!(call.request.is_none());
        assert!(call.result.unwrap().response.is_none());
    }
    let mut ordinary = row(request, r#"{"jsonrpc":"2.0","result":null}"#);
    ordinary.origin = ToolOrigin::Model;
    assert!(tool_content(&ordinary).request.is_none());
}
