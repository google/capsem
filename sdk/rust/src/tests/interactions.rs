use crate::models::{CapturedContent, InteractionContent, ToolOrigin};
use crate::operations::{get_vm_stats_detail, GetVmStatsDetailParams};
use crate::test_contract::Case;
use crate::test_gateway::Server;
use crate::transport::CallOptions;
use axum::body::Body;
use axum::http::Response;
use serde_json::json;

#[tokio::test]
async fn http_stats_returns_native_json_and_typed_interactions() {
    let mut response = Case::new("getVmStatsDetail", false).response;
    response["interactions"] = json!({"bodies": [], "items": [{
        "event_id":"abcdef000001", "timestamp":"2026-09-10T00:00:00Z",
        "model_call_id":null, "model_event_id":null, "trace_id":"trace", "turn_id":null, "item_index":null,
        "content": {"kind":"tool_call", "call_id":"call-1", "tool_name":"search", "server_name":"knowledge",
            "origin":"mcp", "decision":"allowed", "arguments":{
                "status":"unknown", "content":{"kind":"json", "value":{"q":[true,null,3]}}},
            "result":null}
    }]});
    let mut server = Server::respond(move |_| {
        Response::builder()
            .header("content-type", "application/json")
            .body(Body::from(response.to_string()))
            .unwrap()
    })
    .await;
    let detail = get_vm_stats_detail(
        &server.client(),
        &GetVmStatsDetailParams { id: "vm-1".into() },
        CallOptions::default(),
    )
    .await
    .unwrap();
    let InteractionContent::ToolCall(call) = &detail.interactions.items[0].content else {
        panic!("expected tool call")
    };
    assert_eq!(call.origin, ToolOrigin::Mcp);
    let CapturedContent::Json(args) = &call.arguments.as_ref().unwrap().content else {
        panic!("expected JSON")
    };
    assert_eq!(args.value, json!({"q":[true,null,3]}));
    assert!(call.result.is_none());
    let (request, _) = server.received.recv().await.unwrap();
    assert_eq!(request.uri.path(), "/vms/vm-1/stats/detail");
    assert_eq!(request.headers["authorization"], "Bearer private-token");
}
