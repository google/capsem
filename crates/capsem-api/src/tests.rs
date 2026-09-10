use super::*;
use serde_json::json;
use utoipa::PartialSchema;

#[test]
fn lifecycle_values_preserve_the_existing_wire_contract() {
    assert_eq!(
        serde_json::to_value(VmLifecycleState::Running).unwrap(),
        json!("Running")
    );
    assert_eq!(serde_json::to_value(VmAction::Pause).unwrap(), json!("pause"));
    assert!(serde_json::from_value::<VmLifecycleState>(json!("running")).is_err());
    assert!(serde_json::from_value::<VmAction>(json!("invented")).is_err());
}

#[test]
fn action_availability_distinguishes_resume_from_start() {
    assert_eq!(VmLifecycleState::Suspended.available_actions(true)[0], VmAction::Resume);
    assert_eq!(VmLifecycleState::Stopped.available_actions(true)[0], VmAction::Start);
    assert_eq!(
        VmLifecycleState::Defunct.available_actions(true),
        vec![VmAction::Delete]
    );
}

#[test]
fn omitted_resources_remain_profile_owned() {
    let request: ProvisionRequest = serde_json::from_value(json!({"profile_id": "code"})).unwrap();
    let wire = serde_json::to_value(request).unwrap();
    assert!(wire.get("ram_mb").is_none());
    assert!(wire.get("cpus").is_none());
    assert_eq!(wire["persistent"], false);
}

#[test]
fn schema_exposes_closed_lifecycle_and_action_values() {
    let state = serde_json::to_value(VmLifecycleState::schema()).unwrap();
    let action = serde_json::to_value(VmAction::schema()).unwrap();
    assert_eq!(
        state["enum"],
        json!(["Running", "Stopped", "Suspended", "Defunct", "Incompatible"])
    );
    assert_eq!(
        action["enum"],
        json!(["pause", "stop", "start", "resume", "fork", "delete"])
    );
}
