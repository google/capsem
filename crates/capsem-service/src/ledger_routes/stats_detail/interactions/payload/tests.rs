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

#[test]
fn body_capture_metadata_controls_interpretation() {
    let mut body = EventBody {
        event_id: "abcdef000001".into(),
        direction: BodyDirection::Response,
        content_type: Some("Application/Problem+Json; charset=utf-8".into()),
        original_bytes: 4,
        stored_bytes: 4,
        truncated: false,
        body_hash: "hash".into(),
        body: "null".into(),
    };
    assert!(matches!(body_payload(&body).content, CapturedContent::Json(_)));
    body.truncated = true;
    let CapturedContent::Raw(raw) = body_payload(&body).content else {
        panic!("expected raw")
    };
    assert_eq!(raw.reason, RawContentReason::Truncated);
    assert_eq!(raw.raw, "null");
    assert_eq!(body_payload(&body).status, CaptureStatus::Truncated);
    body.truncated = false;
    body.content_type = Some("text/plain".into());
    assert!(matches!(body_payload(&body).content, CapturedContent::Text(_)));
    assert_eq!(body_payload(&body).status, CaptureStatus::Complete);
    for content_type in [
        None,
        Some("text/event-stream".into()),
        Some("application/octet-stream".into()),
    ] {
        body.content_type = content_type;
        let CapturedContent::Raw(raw) = body_payload(&body).content else {
            panic!("expected raw")
        };
        assert_eq!(raw.reason, RawContentReason::Unparsed);
    }
    let CapturedContent::Raw(raw) = json_payload("{}".into(), CaptureStatus::Truncated).content else {
        panic!("expected raw")
    };
    assert_eq!(raw.reason, RawContentReason::Truncated);
}
