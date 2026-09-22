use super::*;
use serde_json::json;

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
