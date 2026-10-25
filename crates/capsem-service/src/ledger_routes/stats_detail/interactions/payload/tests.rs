use super::*;
use serde_json::json;

#[test]
fn capped_demo_request_is_truncated_not_invalid_json() {
    let preview = "{\"model\":\"gemini-4-argon-medium\",\"request\":{\"text\":\"".to_owned() + &"x".repeat(1991);
    let payload = cut(preview_payload(preview), true);
    assert_eq!(payload.status, CaptureStatus::Truncated);
    let CapturedContent::Raw(content) = payload.content else {
        panic!("expected raw preview")
    };
    assert_eq!(content.reason, RawContentReason::Truncated);
}

#[test]
fn archived_request_length_marks_the_stored_excerpt_without_guessing_a_cap() {
    use super::super::{read_interactions, ModelItemRow};
    let row: ModelItemRow = serde_json::from_value(json!({
        "event_id":"bbbbbb000001", "timestamp":"2026-10-25T03:10:40Z",
        "model_call_id":2, "model_event_id":"aaaaaa000001", "trace_id":"fixture",
        "turn_id":"fixture", "item_index":1, "kind":"request", "call_id":"",
        "content":"{\"request\":", "content_truncated":false, "is_error":null
    }))
    .unwrap();
    let body: EventBody = serde_json::from_value(json!({
        "event_id":"aaaaaa000001", "source_table":"model_calls", "direction":"request",
        "content_type":"application/json", "original_bytes":37033, "stored_bytes":37033,
        "truncated":false, "body_hash":"xxh3-128:00000000000000000000000000000000"
    }))
    .unwrap();
    let report = read_interactions(
        "fixture",
        std::path::Path::new("unused"),
        vec![row],
        vec![],
        &std::collections::BTreeMap::from([("aaaaaa000001".into(), vec![body])]),
    )
    .unwrap();
    let InteractionContent::Request(request) = &report.items[0].content else {
        panic!("expected request")
    };
    assert_eq!(request.payload.as_ref().unwrap().status, CaptureStatus::Truncated);
}

#[test]
fn payloads_preserve_json_null_plain_text_and_malformed_json() {
    for value in [json!(null), json!([true, {"x": 2}]), json!("value"), json!(42)] {
        let payload = json_payload(value.to_string(), CaptureStatus::Unknown);
        assert_eq!(payload.status, CaptureStatus::Unknown);
        let CapturedContent::Json(content) = payload.content else {
            panic!("expected JSON")
        };
        assert_eq!(content.value, value);
    }
    assert!(matches!(
        preview_payload("plain text".into()).content,
        CapturedContent::Text(_)
    ));
    let payload = json_payload("{\"unfinished\":".into(), CaptureStatus::Unknown);
    let CapturedContent::Raw(content) = payload.content else {
        panic!("expected raw")
    };
    assert_eq!(content.reason, RawContentReason::InvalidJson);
    assert_eq!(content.raw, "{\"unfinished\":");
    assert!(matches!(
        preview_payload("[unfinished".into()).content,
        CapturedContent::Raw(_)
    ));
    assert!(matches!(text_payload("null".into()).content, CapturedContent::Text(_)));
}
