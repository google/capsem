use super::*;

fn empty_manager() -> Arc<RwLock<McpServerManager>> {
    Arc::new(RwLock::new(McpServerManager::new(
        Vec::new(),
        reqwest::Client::new(),
    )))
}

#[tokio::test]
async fn local_echo_uses_aggregator_fast_path_without_running_manager() {
    let manager = empty_manager();
    let resp = handle_request(
        &manager,
        AggregatorRequest {
            id: 7,
            method: AggregatorMethod::CallTool {
                name: "local__echo".to_string(),
                arguments: serde_json::json!({"text": "hello"}),
            },
        },
        "call_tool",
        "local_echo",
    )
    .await;

    match resp.body {
        AggregatorResult::CallResult { result } => {
            assert_eq!(result["content"][0]["type"], "text");
            assert_eq!(result["content"][0]["text"], "hello");
            assert_eq!(result["isError"], false);
        }
        other => panic!("expected local echo fast-path result, got {other:?}"),
    }
}

#[tokio::test]
async fn local_echo_fast_path_rejects_missing_text() {
    let manager = empty_manager();
    let resp = handle_request(
        &manager,
        AggregatorRequest {
            id: 8,
            method: AggregatorMethod::CallTool {
                name: "local__echo".to_string(),
                arguments: serde_json::json!({}),
            },
        },
        "call_tool",
        "local_echo",
    )
    .await;

    match resp.body {
        AggregatorResult::Error { error } => {
            assert!(error.contains("requires string argument 'text'"));
        }
        other => panic!("expected local echo validation error, got {other:?}"),
    }
}
