use super::*;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

fn proof(stream: bool) -> Value {
    json!({"model": "codex-fixture", "stream": stream,
    "input": [{"role": "user", "content": [{"type": "input_text", "text":
        format!("CAPSEM_MCP_PROOF={TOKEN}. Call the Capsem echo tool.")}]}],
    "tools": [{"type": "function", "name": "mcp__capsem__local__echo", "parameters": {
        "type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]
    }}]})
}

fn completed(body: Bytes, stream: bool) -> Value {
    if !stream {
        return serde_json::from_slice(&body).unwrap();
    }
    let events: Vec<Value> = std::str::from_utf8(&body)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event["sequence_number"], index, "{event}");
    }
    assert_eq!(events[0]["type"], "response.created");
    assert_eq!(events.last().unwrap()["type"], "response.completed");
    let final_response = events.last().unwrap()["response"].clone();
    let done = events
        .iter()
        .find(|event| event["type"] == "response.output_item.done")
        .unwrap();
    assert_eq!(done["item"], final_response["output"][0]);
    final_response
}

#[tokio::test]
async fn responses_mcp_proof_requests_the_declared_echo_over_json_and_sse() {
    for stream in [false, true] {
        let (status, _, body) = routed(Method::POST, "/v1/responses", None, HeaderMap::new(), proof(stream)).await;
        assert_eq!(status, StatusCode::OK);
        let response = completed(body, stream);
        let call = &response["output"][0];
        assert_eq!(response["model"], "codex-fixture");
        assert_eq!(call["type"], "function_call");
        assert_eq!(call["name"], "mcp__capsem__local__echo");
        assert_eq!(call["call_id"], "call_capsem_mcp_echo");
        assert_eq!(
            serde_json::from_str::<Value>(call["arguments"].as_str().unwrap()).unwrap(),
            json!({"text": TOKEN})
        );
    }
}

#[tokio::test]
async fn responses_mcp_proof_refuses_missing_tools_and_wrong_or_failed_results() {
    let mut missing = proof(false);
    missing["tools"] = json!([]);
    let mut cases = vec![missing];
    for (call_id, output) in [
        ("call_capsem_mcp_echo", json!("wrong text")),
        ("another_call", json!(TOKEN)),
        (
            "call_capsem_mcp_echo",
            json!(json!({"content": [{"type": "text", "text": TOKEN}], "isError": true}).to_string()),
        ),
    ] {
        let mut payload = proof(false);
        payload["input"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type": "function_call_output", "call_id": call_id, "output": output}));
        cases.push(payload);
    }
    for payload in cases {
        let (status, _, body) = routed(Method::POST, "/v1/responses", None, HeaderMap::new(), payload).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{}", String::from_utf8_lossy(&body));
    }
}

#[tokio::test]
async fn responses_mcp_proof_finishes_only_after_its_exact_successful_result() {
    for stream in [false, true] {
        let mut payload = proof(stream);
        payload["input"].as_array_mut().unwrap().push(json!({"type": "function_call_output",
            "call_id": "call_capsem_mcp_echo", "output": json!({"content": [{"type": "text", "text": TOKEN}], "isError": false}).to_string()}));
        let (status, _, body) = routed(Method::POST, "/v1/responses", None, HeaderMap::new(), payload).await;
        assert_eq!(status, StatusCode::OK);
        let response = completed(body, stream);
        assert_eq!(response["status"], "completed");
        assert_eq!(response["output"][0]["type"], "message");
        assert_eq!(response["output"][0]["content"][0]["text"], TOKEN);
    }
}

#[tokio::test]
async fn responses_mcp_proof_uses_the_declared_code_executor_to_discover_and_call_echo() {
    let mut payload = proof(false);
    payload.as_object_mut().unwrap().remove("tools");
    payload["input"].as_array_mut().unwrap().insert(
        0,
        json!({"role": "developer", "type": "additional_tools",
        "tools": [{"type": "namespace", "name": "functions", "tools": [{"type": "custom", "name": "exec"}]}]}),
    );
    let response = routed_json(Method::POST, "/v1/responses", payload).await;
    let call = &response["output"][0];
    assert_eq!(call["type"], "custom_tool_call");
    assert_eq!(call["namespace"], "functions");
    assert_eq!(call["name"], "exec");
    let code = call["input"].as_str().unwrap();
    assert!(
        code.contains("ALL_TOOLS.find") && code.contains("await tools["),
        "{code}"
    );
    assert!(code.contains("text(block.text)"), "{code}");
    assert!(!code.contains(&format!("text(\"{TOKEN}\")")), "{code}");
}

#[tokio::test]
async fn ordinary_responses_turns_keep_complete_item_events_and_exact_text() {
    for stream in [false, true] {
        let payload = json!({"model": "ordinary-fixture", "stream": stream,
            "input": [{"role": "user", "content": format!("Write {TOKEN} to /workspace/result.txt.")}]});
        let (status, _, body) = routed(Method::POST, "/v1/responses", None, HeaderMap::new(), payload.clone()).await;
        assert_eq!(status, StatusCode::OK);
        let first = completed(body, stream);
        assert_eq!(first["output"][0]["name"], "exec_command");
        let args: Value = serde_json::from_str(first["output"][0]["arguments"].as_str().unwrap()).unwrap();
        assert!(
            args["cmd"].as_str().unwrap().contains("/workspace/result.txt")
                && args["cmd"].as_str().unwrap().contains(TOKEN)
        );
        let mut final_turn = payload;
        final_turn["input"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type": "function_call_output", "output": TOKEN}));
        let (status, _, body) = routed(Method::POST, "/v1/responses", None, HeaderMap::new(), final_turn).await;
        assert_eq!(status, StatusCode::OK);
        let response = completed(body, stream);
        assert_eq!(response["output"][0]["type"], "message");
        assert_eq!(response["output"][0]["content"][0]["text"], TOKEN);
        assert_eq!(response["output"][1]["summary"][0]["text"], "ledger reasoning");
    }
}

#[tokio::test]
async fn ordinary_responses_text_with_newlines_is_valid_json_in_every_sse_event() {
    let (status, _, body) = routed(
        Method::POST,
        "/v1/responses",
        None,
        HeaderMap::new(),
        json!({"stream": true, "input": [{"type": "function_call_output", "output": "done"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let response = completed(body, true);
    assert_eq!(response["output"][0]["content"][0]["text"], EXPECTED_POEM);
}

#[tokio::test]
async fn responses_mcp_proof_consumes_the_actual_code_output_blocks_and_refuses_failure() {
    for (header, expected_status) in [
        ("Script completed\nWall time 0.0 seconds\nOutput:\n", StatusCode::OK),
        (
            "Script failed\nWall time 0.0 seconds\nOutput:\n",
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let mut payload = proof(false);
        payload.as_object_mut().unwrap().remove("tools");
        payload["input"].as_array_mut().unwrap().insert(
            0,
            json!({"role": "developer", "type": "additional_tools",
            "tools": [{"type": "namespace", "name": "functions", "tools": [{"type": "custom", "name": "exec"}]}]}),
        );
        payload["input"].as_array_mut().unwrap().push(
            json!({"type": "custom_tool_call_output", "call_id": "call_capsem_mcp_echo",
            "output": [{"type": "input_text", "text": header}, {"type": "input_text", "text": TOKEN}]}),
        );
        let (status, _, body) = routed(Method::POST, "/v1/responses", None, HeaderMap::new(), payload).await;
        assert_eq!(status, expected_status);
        if status == StatusCode::OK {
            let response = completed(body, false);
            assert_eq!(response["output"][0]["content"][0]["text"], TOKEN);
        }
    }
}
