use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, RwLock};

use crate::mcp::aggregator::{
    AggregatorMethod, AggregatorRequest, AggregatorResponse, AggregatorResult,
};
use crate::mcp::policy::McpPolicy;
use crate::mcp::types::{JsonRpcRequest, McpPromptDef, McpResourceDef, McpToolDef};

use super::*;

fn json_request(method: &str, params: serde_json::Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(serde_json::json!(1)),
        method: method.to_string(),
        params: Some(params),
        meta: None,
    }
}

fn endpoint_with_driver<F, Fut>(
    timeouts: McpTimeouts,
    mut respond: F,
) -> (Arc<McpEndpointState>, Arc<Mutex<Vec<String>>>)
where
    F: FnMut(AggregatorRequest) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = AggregatorResult> + Send + 'static,
{
    let (aggregator, mut rx) = crate::mcp::aggregator::AggregatorClient::channel(16);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let calls_h = Arc::clone(&calls);
    tokio::spawn(async move {
        while let Some((req, resp_tx)) = rx.recv().await {
            let label = match &req.method {
                AggregatorMethod::ListServers => "list_servers",
                AggregatorMethod::ListTools => "list_tools",
                AggregatorMethod::ListResources => "list_resources",
                AggregatorMethod::ListPrompts => "list_prompts",
                AggregatorMethod::CallTool { .. } => "call_tool",
                AggregatorMethod::ReadResource { .. } => "read_resource",
                AggregatorMethod::GetPrompt { .. } => "get_prompt",
                AggregatorMethod::Refresh { .. } => "refresh",
                AggregatorMethod::Shutdown => "shutdown",
            };
            calls_h.lock().await.push(label.to_string());
            let id = req.id;
            let body = respond(req).await;
            let _ = resp_tx.send(AggregatorResponse { id, body });
        }
    });

    (
        Arc::new(McpEndpointState::new(
            aggregator,
            Arc::new(RwLock::new(Arc::new(McpPolicy::new()))),
            Arc::new(super::super::RuntimeSecurityEngineSlot::new(None)),
            Arc::new(tokio::sync::Semaphore::new(
                crate::mcp::default_inflight_cap(),
            )),
            timeouts,
        )),
        calls,
    )
}

#[tokio::test]
async fn endpoint_dispatches_every_supported_method_family() {
    let (endpoint, calls) = endpoint_with_driver(McpTimeouts::default(), |req| async move {
        match req.method {
            AggregatorMethod::ListTools => AggregatorResult::Tools {
                tools: vec![McpToolDef {
                    namespaced_name: "local__echo".to_string(),
                    original_name: "echo".to_string(),
                    description: Some("Echo".to_string()),
                    input_schema: serde_json::json!({"type": "object"}),
                    server_name: "local".to_string(),
                    annotations: None,
                    timeout_secs: Some(30),
                }],
            },
            AggregatorMethod::CallTool { name, arguments } => AggregatorResult::CallResult {
                result: serde_json::json!({"name": name, "arguments": arguments}),
            },
            AggregatorMethod::ListResources => AggregatorResult::Resources {
                resources: vec![McpResourceDef {
                    namespaced_uri: "capsem://local/readme".to_string(),
                    original_uri: "readme".to_string(),
                    name: Some("readme".to_string()),
                    description: Some("Readme".to_string()),
                    mime_type: Some("text/plain".to_string()),
                    server_name: "local".to_string(),
                }],
            },
            AggregatorMethod::ReadResource { uri } => AggregatorResult::CallResult {
                result: serde_json::json!({"uri": uri, "contents": []}),
            },
            AggregatorMethod::ListPrompts => AggregatorResult::Prompts {
                prompts: vec![McpPromptDef {
                    namespaced_name: "local__review".to_string(),
                    original_name: "review".to_string(),
                    description: Some("Review".to_string()),
                    arguments: vec![serde_json::json!({"name": "topic"})],
                    server_name: "local".to_string(),
                }],
            },
            AggregatorMethod::GetPrompt { name, arguments } => AggregatorResult::CallResult {
                result: serde_json::json!({"name": name, "arguments": arguments}),
            },
            _ => AggregatorResult::Error {
                error: "unexpected method".to_string(),
            },
        }
    });

    let init = endpoint
        .handle_request(&json_request("initialize", serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(
        init.result.as_ref().unwrap()["serverInfo"]["name"],
        "capsem-mcp-mitm-endpoint"
    );

    let tools = endpoint
        .handle_request(&json_request("tools/list", serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(
        tools.result.as_ref().unwrap()["tools"][0]["name"],
        "local__echo"
    );

    let call = endpoint
        .handle_request(&json_request(
            "tools/call",
            serde_json::json!({"name": "local__echo", "arguments": {"text": "hi"}}),
        ))
        .await
        .unwrap();
    assert_eq!(call.result.as_ref().unwrap()["name"], "local__echo");

    let resources = endpoint
        .handle_request(&json_request("resources/list", serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(
        resources.result.as_ref().unwrap()["resources"][0]["uri"],
        "capsem://local/readme"
    );

    let resource = endpoint
        .handle_request(&json_request(
            "resources/read",
            serde_json::json!({"uri": "capsem://local/readme"}),
        ))
        .await
        .unwrap();
    assert_eq!(
        resource.result.as_ref().unwrap()["uri"],
        "capsem://local/readme"
    );

    let prompts = endpoint
        .handle_request(&json_request("prompts/list", serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(
        prompts.result.as_ref().unwrap()["prompts"][0]["name"],
        "local__review"
    );

    let prompt = endpoint
        .handle_request(&json_request(
            "prompts/get",
            serde_json::json!({"name": "local__review", "arguments": {"topic": "capsem"}}),
        ))
        .await
        .unwrap();
    assert_eq!(prompt.result.as_ref().unwrap()["name"], "local__review");

    assert_eq!(
        calls.lock().await.as_slice(),
        [
            "list_tools",
            "call_tool",
            "list_resources",
            "read_resource",
            "list_prompts",
            "get_prompt",
        ]
    );
}

#[tokio::test]
async fn endpoint_maps_aggregator_errors_for_each_method_family() {
    let cases = [
        ("tools/list", serde_json::json!({}), "tools list failed"),
        (
            "tools/call",
            serde_json::json!({"name": "local__echo", "arguments": {}}),
            "tool call failed",
        ),
        (
            "resources/list",
            serde_json::json!({}),
            "resources list failed",
        ),
        (
            "resources/read",
            serde_json::json!({"uri": "capsem://local/readme"}),
            "resource read failed",
        ),
        ("prompts/list", serde_json::json!({}), "prompts list failed"),
        (
            "prompts/get",
            serde_json::json!({"name": "local__review", "arguments": {}}),
            "prompt get failed",
        ),
    ];

    for (method, params, expected) in cases {
        let (endpoint, _) = endpoint_with_driver(McpTimeouts::default(), |_req| async move {
            AggregatorResult::Error {
                error: "boom".to_string(),
            }
        });

        let response = endpoint
            .handle_request(&json_request(method, params))
            .await
            .unwrap();

        let message = &response.error.as_ref().unwrap().message;
        assert!(
            message.contains(expected),
            "expected {expected:?} in {message:?}"
        );
        assert!(message.contains("boom"));
    }
}

#[tokio::test]
async fn endpoint_rejects_missing_required_params_before_dispatch() {
    let (endpoint, calls) = endpoint_with_driver(McpTimeouts::default(), |_req| async move {
        AggregatorResult::Error {
            error: "should not dispatch".to_string(),
        }
    });

    for (method, params, expected) in [
        ("tools/call", serde_json::json!({}), "missing tool name"),
        (
            "resources/read",
            serde_json::json!({}),
            "missing resource URI",
        ),
        ("prompts/get", serde_json::json!({}), "missing prompt name"),
    ] {
        let response = endpoint
            .handle_request(&json_request(method, params))
            .await
            .unwrap();
        let error = response.error.as_ref().unwrap();
        assert_eq!(error.code, -32602);
        assert!(error.message.contains(expected));
    }

    assert!(calls.lock().await.is_empty());
}

#[tokio::test]
async fn endpoint_times_out_tool_calls_and_records_catalog_ceiling() {
    let (endpoint, _) = endpoint_with_driver(
        McpTimeouts {
            default_timeout: Duration::from_secs(60),
            tool_call_default: Duration::from_millis(10),
            tool_call_ceiling: Duration::from_millis(25),
        },
        |req| async move {
            if matches!(req.method, AggregatorMethod::CallTool { .. }) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            AggregatorResult::CallResult {
                result: serde_json::json!({}),
            }
        },
    );

    endpoint
        .record_tool_catalog_timeouts(&[McpToolDef {
            namespaced_name: "local__slow".to_string(),
            original_name: "slow".to_string(),
            description: None,
            input_schema: serde_json::json!({"type": "object"}),
            server_name: "local".to_string(),
            annotations: None,
            timeout_secs: Some(60),
        }])
        .await;

    assert_eq!(
        endpoint
            .timeout_for_request("tools/call", Some("local__slow"))
            .await,
        Duration::from_millis(25)
    );

    let response = endpoint
        .handle_request(&json_request(
            "tools/call",
            serde_json::json!({"name": "local__slow", "arguments": {}}),
        ))
        .await
        .unwrap();

    assert!(response
        .error
        .as_ref()
        .is_some_and(|error| error.message.contains("timed out")));
}
