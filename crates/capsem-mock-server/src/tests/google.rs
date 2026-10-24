//! Captured AGY1.1.3 declaration/advertisement; synthetic protocol tests only.
use super::*;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";
const CALL_ID: &str = "capsem_google_mcp_echo";

fn payload() -> Value {
    json!({"model":"gemini-3.5-flash-low", "requestType":"agent", "request": {
        "systemInstruction":{"parts":[{"text":"<mcp_servers>\n# capsem\nLazy:\nlocal__echo\nlocal__fetch_http\n</mcp_servers>"}]},
        "contents":[{"role":"user","parts":[{"text":format!("<USER_REQUEST>\nCAPSEM_MCP_PROOF={TOKEN}. Call the Capsem MCP local echo tool.\n</USER_REQUEST>")}]}],
        "tools":[{"functionDeclarations":[{"name":"call_mcp_tool", "parameters": {
            "type":"OBJECT", "properties":{"ServerName":{"type":"STRING"},"ToolName":{"type":"STRING"},
                "Arguments":{},"toolSummary":{"type":"STRING"},"toolAction":{"type":"STRING"}},
            "required":["ServerName","ToolName","Arguments","toolSummary","toolAction"]}}]}]
    }})
}

fn events(body: &[u8]) -> Vec<Value> {
    std::str::from_utf8(body)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn google_mcp_dispatch_accepts_the_reduced_original_native_request() {
    let request = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/protocols/google_code_assist/agy_1_1_3_lazy_mcp_request.json"
    ))
    .unwrap();
    let (status, _, body) = routed(
        Method::POST,
        "/v1internal:streamGenerateContent",
        None,
        HeaderMap::new(),
        request,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        events(&body)[0]["response"]["candidates"][0]["content"]["parts"][0]["functionCall"]["args"]["Arguments"],
        json!({"text":"72b8356f48864b1fb3fc64f5e7a8ac6f"})
    );
}

#[tokio::test]
async fn google_mcp_marker_in_tool_metadata_or_checkpoint_never_starts_proof() {
    for checkpoint in [false, true] {
        let mut request = payload();
        if checkpoint {
            request["requestType"] = json!("checkpoint");
        } else {
            request["request"]["contents"][0]["parts"][0]["text"] = json!("ordinary user prompt");
            request["request"]["tools"][0]["functionDeclarations"][0]["description"] =
                json!(format!("CAPSEM_MCP_PROOF={TOKEN}."));
        }
        let (status, _, body) = routed(
            Method::POST,
            "/v1internal:streamGenerateContent",
            None,
            HeaderMap::new(),
            request,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!String::from_utf8_lossy(&body).contains("call_mcp_tool"));
    }
}

#[tokio::test]
async fn google_mcp_proof_uses_exact_captured_lazy_dispatch_schema() {
    let (status, _, body) = routed(
        Method::POST,
        "/v1internal:streamGenerateContent",
        None,
        HeaderMap::new(),
        payload(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let chunks = events(&body);
    let call = &chunks[0]["response"]["candidates"][0]["content"]["parts"][0]["functionCall"];
    assert_eq!(
        call,
        &json!({"name":"call_mcp_tool", "id":CALL_ID, "args":{
        "ServerName":"capsem","ToolName":"local__echo","Arguments":{"text":TOKEN},
        "toolSummary":"MCP echo proof","toolAction":"Calling echo"}})
    );
    assert!(!String::from_utf8_lossy(&body).contains("run_command"));
    assert!(!String::from_utf8_lossy(&body).contains("/root/agy-output.txt"));
}

#[tokio::test]
async fn google_mcp_proof_refuses_undeclared_or_misdeclared_dispatch() {
    for fault in ["name", "required", "type", "advertisement", "server", "tool", "token"] {
        let mut request = payload();
        let declaration = &mut request["request"]["tools"][0]["functionDeclarations"][0];
        match fault {
            "name" => declaration["name"] = json!("guessed_echo_alias"),
            "required" => declaration["parameters"]["required"] = json!(["ServerName", "ToolName"]),
            "type" => declaration["parameters"]["properties"]["ToolName"]["type"] = json!("NUMBER"),
            "advertisement" => request["request"]["systemInstruction"] = json!({}),
            "server" => {
                request["request"]["systemInstruction"]["parts"][0]["text"] =
                    json!("<mcp_servers>\n# other\nLazy:\nlocal__echo\n</mcp_servers>")
            }
            "tool" => {
                request["request"]["systemInstruction"]["parts"][0]["text"] =
                    json!("<mcp_servers>\n# capsem\nLazy:\nother__echo\n</mcp_servers>")
            }
            "token" => {
                request["request"]["contents"][0]["parts"][0]["text"] =
                    json!("<USER_REQUEST>\nCAPSEM_MCP_PROOF=bad.\n</USER_REQUEST>")
            }
            _ => unreachable!(),
        }
        let (status, _, body) = routed(
            Method::POST,
            "/v1internal:streamGenerateContent",
            None,
            HeaderMap::new(),
            request,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{fault}: {}",
            String::from_utf8_lossy(&body)
        );
    }
}

async fn with_result(fault: &str) -> (StatusCode, HeaderMap, Bytes) {
    let mut request = payload();
    let args = json!({"ServerName":"capsem","ToolName":"local__echo","Arguments":{"text":TOKEN},
        "toolSummary":"MCP echo proof","toolAction":"Calling echo"});
    let mut call = json!({"id":CALL_ID,"name":"call_mcp_tool","args":args});
    let mut result = json!({"id":CALL_ID,"name":"call_mcp_tool", "response":{
        "output":{"content":[{"type":"text","text":TOKEN}],"isError":false}}});
    match fault {
        "id" => result["id"] = json!("other_call"),
        "name" => result["name"] = json!("run_command"),
        "args" => call["args"]["ToolName"] = json!("other"),
        "error" => result["response"]["output"]["isError"] = json!(true),
        "error-with-output" => result["response"]["error"] = json!("permission denied"),
        "wrong-text" => result["response"]["output"]["content"][0]["text"] = json!("wrong"),
        "extra" => result["response"]["output"]["content"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"text","text":"extra"})),
        "unknown-envelope" => {
            result["response"]["output"] = json!(format!("Created At: timestamp\npermission denied; {TOKEN}"))
        }
        "string-json" => result["response"]["output"] = json!(result["response"]["output"].to_string()),
        "none" | "role" => {}
        _ => unreachable!(),
    }
    request["request"]["contents"].as_array_mut().unwrap().extend([
        json!({"role":"model","parts":[{"functionCall":call}]}),
        json!({"role":"user","parts":[{"functionResponse":result}]}),
    ]);
    if fault == "role" {
        request["request"]["contents"][1]["role"] = json!("user");
    }
    routed(
        Method::POST,
        "/v1internal:streamGenerateContent",
        None,
        HeaderMap::new(),
        request,
    )
    .await
}

#[tokio::test]
async fn google_mcp_proof_never_finalizes_an_unrelated_failed_or_unknown_result() {
    for fault in [
        "id",
        "name",
        "args",
        "error",
        "error-with-output",
        "wrong-text",
        "extra",
        "unknown-envelope",
        "role",
    ] {
        let (status, _, body) = with_result(fault).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{fault}: {}",
            String::from_utf8_lossy(&body)
        );
    }
}

#[tokio::test]
async fn google_mcp_proof_finishes_only_for_exact_typed_mcp_result_and_original_call() {
    // These typed MCP result forms are source controls, NOT an observed AGY
    // success envelope. Parent native capture remains the compatibility gate.
    for representation in ["none", "string-json"] {
        let (status, _, body) = with_result(representation).await;
        assert_eq!(status, StatusCode::OK);
        let chunks = events(&body);
        let response = &chunks[0]["response"];
        assert_eq!(response["candidates"][0]["content"]["parts"], json!([{"text":TOKEN}]));
        assert_eq!(response["candidates"][0]["finishReason"], "STOP");
    }
}
