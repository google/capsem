use super::*;

fn request(id: i64) -> ClientJsonRpcMessage {
    serde_json::from_value(serde_json::json!({"jsonrpc":"2.0","id":id,"method":"tools/list","params":{}})).unwrap()
}

fn event(data: &str) -> Sse {
    Sse {
        data: Some(data.into()),
        ..Sse::default()
    }
}

#[test]
fn failed_posts_release_ids_and_accepted_posts_keep_only_outstanding_ids() {
    let pending = PendingResponses::default();
    let first = pending.track(&request(1));
    let mut second = pending.track(&request(2));
    second.retain();
    drop((first, second));
    assert_eq!(pending.refuse_all(), [RequestId::Number(2)]);
    assert!(pending.refuse_all().is_empty());
}

#[test]
fn only_responses_complete_calls_not_server_requests_with_the_same_id() {
    let pending = PendingResponses::default();
    let mut first = pending.track(&request(1));
    let mut second = pending.track(&request(2));
    first.retain();
    second.retain();
    pending.complete(&event(r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}"#));
    pending.complete(&event(r#"{"jsonrpc":"2.0","id":2,"result":{"content":[]}}"#));
    assert_eq!(pending.refuse_all(), [RequestId::Number(1)]);
}

#[test]
fn cancelled_accepted_requests_are_not_retained_until_a_reply_arrives() {
    let pending = PendingResponses::default();
    let mut accepted = pending.track(&request(7));
    accepted.retain();
    let cancelled = serde_json::from_value(serde_json::json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":7,"reason":"request timeout"}})).unwrap();
    drop(pending.track(&cancelled));
    assert!(pending.refuse_all().is_empty());
}
