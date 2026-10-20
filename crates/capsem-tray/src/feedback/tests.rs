use super::*;

#[test]
fn polling_preserves_action_failure_until_an_explicit_success() {
    let mut feedback = ActionFeedback::default();
    feedback.record(Some("Resume failed: checkpoint incompatible".into()));
    for service in ["running", "unavailable", "running"] {
        let mut status: StatusResponse = serde_json::from_value(serde_json::json!({
            "service": service, "vm_count": 0, "vms": []
        }))
        .unwrap();
        feedback.apply(&mut status);
        assert_eq!(status.service, service);
        let spec = crate::menu::menu_spec(&status);
        assert!(spec.iter().any(|entry| matches!(entry, crate::menu::MenuEntry::Item { id, label, enabled: false } if id == "action-error" && label.contains("checkpoint incompatible"))));
    }
    feedback.record(None);
    let mut status: StatusResponse =
        serde_json::from_value(serde_json::json!({"service": "running", "vm_count": 0, "vms": []})).unwrap();
    feedback.apply(&mut status);
    assert!(status.action_error.is_none());
    assert!(feedback.message().is_none());
}
