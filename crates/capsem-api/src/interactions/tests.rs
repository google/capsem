use super::*;
use serde_json::json;

#[test]
fn structured_json_null_is_present_and_unknown_discriminators_fail() {
    for value in [json!(null), json!({"nested": [true, 3, {"x": "y"}]}), json!([1, false])] {
        let wire = json!({"status": "complete", "content": {"kind": "json", "value": value}});
        let decoded: CapturedPayload = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
    }
    for content in [
        json!({"kind": "json"}),
        json!({"kind": "invented", "value": 1}),
        json!({"kind": "text", "text": "x", "value": 2}),
    ] {
        assert!(serde_json::from_value::<CapturedContent>(content).is_err());
    }
}

#[test]
fn interaction_variants_have_required_typed_fields() {
    let message = json!({"kind": "message", "role": "assistant", "blocks": [
        {"kind": "reasoning", "payload": {"status": "unknown", "content": {"kind": "text", "text": "plan"}}}
    ]});
    let content: InteractionContent = serde_json::from_value(message.clone()).unwrap();
    assert!(matches!(content, InteractionContent::Message(_)));
    assert_eq!(serde_json::to_value(content).unwrap(), message);
    assert!(serde_json::from_value::<InteractionContent>(json!({"kind": "tool_call"})).is_err());
}
