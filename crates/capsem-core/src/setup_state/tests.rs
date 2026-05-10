//! Tests for `setup_state` (extracted from inline `mod tests`).

use super::*;

#[test]
fn load_missing_file_returns_default() {
    let state = load_state(Path::new("/nonexistent/setup-state.json"));
    assert_eq!(state.schema_version, 0);
    assert!(!state.onboarding_completed);
    assert!(!state.install_completed);
    assert_eq!(state.onboarding_version, 0);
    assert!(state.completed_steps.is_empty());
}

#[test]
fn default_state_needs_onboarding() {
    let state = SetupState::default();
    assert!(state.needs_onboarding());
}

#[test]
fn completed_current_version_does_not_need_onboarding() {
    let state = SetupState {
        onboarding_completed: true,
        onboarding_version: CURRENT_ONBOARDING_VERSION,
        ..SetupState::default()
    };
    assert!(!state.needs_onboarding());
}

#[test]
fn older_onboarding_version_triggers_rewalk() {
    // User finished an older wizard version. A release bumped the version.
    // They should see the wizard again.
    let state = SetupState {
        onboarding_completed: true,
        onboarding_version: 0,
        ..SetupState::default()
    };
    if CURRENT_ONBOARDING_VERSION > 0 {
        assert!(state.needs_onboarding());
    }
}

#[test]
fn reset_onboarding_preserves_install_state() {
    let mut state = SetupState {
        install_completed: true,
        onboarding_completed: true,
        onboarding_version: CURRENT_ONBOARDING_VERSION,
        security_preset: Some("medium".into()),
        ..SetupState::default()
    };
    state.mark_done("summary");
    state.reset_onboarding();
    assert!(!state.onboarding_completed);
    assert_eq!(state.onboarding_version, 0);
    assert!(
        state.install_completed,
        "install state must survive a wizard reset"
    );
    assert!(state.is_step_done("summary"));
    assert_eq!(state.security_preset.as_deref(), Some("medium"));
}

#[test]
fn save_and_load_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("setup-state.json");

    let mut state = SetupState {
        schema_version: 2,
        install_completed: true,
        onboarding_completed: true,
        onboarding_version: CURRENT_ONBOARDING_VERSION,
        ..SetupState::default()
    };
    state.mark_done("welcome");
    state.mark_done("providers");
    state.security_preset = Some("medium".to_string());

    save_state(&path, &state).unwrap();
    let loaded = load_state(&path);

    assert_eq!(loaded.schema_version, 2);
    assert!(loaded.is_step_done("welcome"));
    assert!(loaded.is_step_done("providers"));
    assert!(!loaded.is_step_done("summary"));
    assert_eq!(loaded.security_preset.as_deref(), Some("medium"));
    assert!(loaded.install_completed);
    assert!(loaded.onboarding_completed);
    assert_eq!(loaded.onboarding_version, CURRENT_ONBOARDING_VERSION);
}

#[test]
fn load_state_returns_default_on_corrupt_json() {
    // A corrupt state file must not panic and must not propagate the parse
    // error; it should return Default and emit a warn-level log (not
    // asserted here, but pinned in the function's doc comment).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("setup-state.json");
    std::fs::write(&path, b"{ this is not valid json").unwrap();

    let loaded = load_state(&path);
    assert_eq!(loaded.schema_version, 0);
    assert!(loaded.completed_steps.is_empty());
    assert!(loaded.security_preset.is_none());
}

#[test]
fn load_state_returns_default_on_non_object_json() {
    // Valid JSON but wrong shape (array instead of object) should also be
    // treated as corrupt and reset -- not silently accepted as empty.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("setup-state.json");
    std::fs::write(&path, b"[]").unwrap();

    let loaded = load_state(&path);
    assert_eq!(loaded.schema_version, 0);
}

#[test]
fn backward_compat_infers_install_completed_from_summary_step() {
    // A pre-upgrade state file will not have `install_completed`. If the
    // summary step was reached, load_state should infer install=done so
    // the UI doesn't warn "install didn't finish" on upgrade.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("setup-state.json");
    let json = r#"{"schema_version":2,"completed_steps":["welcome","security_preset","providers","repositories","summary"],"security_preset":"medium","providers_done":true,"repositories_done":true,"service_installed":true,"vm_verified":false,"corp_config_source":null,"onboarding_completed":true}"#;
    std::fs::write(&path, json).unwrap();

    let loaded = load_state(&path);
    assert!(
        loaded.install_completed,
        "pre-upgrade state with summary step must infer install_completed"
    );
}

#[test]
fn backward_compat_does_not_infer_install_completed_for_partial_setup() {
    // State file that didn't reach summary step -- install really is
    // incomplete, do not fabricate completeness.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("setup-state.json");
    let json = r#"{"schema_version":2,"completed_steps":["welcome"],"security_preset":null}"#;
    std::fs::write(&path, json).unwrap();

    let loaded = load_state(&path);
    assert!(!loaded.install_completed);
}

#[test]
fn backward_compat_missing_onboarding_field() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("setup-state.json");

    // Write a v1 state file without onboarding_completed, install_completed,
    // or onboarding_version -- all three must default cleanly.
    let json = r#"{"schema_version":1,"completed_steps":["welcome"],"security_preset":"medium","providers_done":true,"repositories_done":true,"service_installed":true,"vm_verified":false,"corp_config_source":null}"#;
    std::fs::write(&path, json).unwrap();

    let loaded = load_state(&path);
    assert_eq!(loaded.schema_version, 1);
    assert!(!loaded.onboarding_completed);
    assert!(!loaded.install_completed);
    assert_eq!(loaded.onboarding_version, 0);
    assert!(loaded.is_step_done("welcome"));
}

#[test]
fn mark_done_is_idempotent() {
    let mut state = SetupState::default();
    state.mark_done("test");
    state.mark_done("test");
    assert_eq!(state.completed_steps.len(), 1);
}
