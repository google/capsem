use super::*;
use crate::ProvisionRequest;
use serde_json::json;

fn private_spec() -> ContainerSpec {
    ContainerSpec {
        image: "registry.example/team/app:1".into(),
        args: vec!["serve".into(), "--token=cmd-secret".into()],
        env: [("API_KEY".to_string(), "env-secret".to_string())].into(),
        registry: Some(RegistryAccess {
            username: Some("robot-user".into()),
            password: Some("registry-secret".into()),
            ca_pem: Some("-----BEGIN CERTIFICATE-----pem-secret".into()),
        }),
        attach: true,
    }
}

#[test]
fn container_spec_debug_never_prints_credentials_args_or_env_values() {
    let debug = format!("{:?}", private_spec());
    for secret in [
        "robot-user",
        "registry-secret",
        "pem-secret",
        "env-secret",
        "cmd-secret",
    ] {
        assert!(!debug.contains(secret), "{secret} leaked through Debug: {debug}");
    }
    assert!(debug.contains("registry.example/team/app:1"), "{debug}");
    assert!(
        debug.contains("API_KEY"),
        "env names stay visible for diagnosis: {debug}"
    );
    let request = ProvisionRequest {
        name: None,
        ram_mb: None,
        cpus: None,
        persistent: false,
        env: None,
        labels: None,
        from: None,
        networks: Vec::new(),
        container: Some(private_spec()),
    };
    assert!(!format!("{request:?}").contains("registry-secret"));
}

#[test]
fn container_spec_wire_shape_omits_empty_fields() {
    let minimal: ContainerSpec = serde_json::from_value(json!({"image": "docker://redis"})).unwrap();
    assert_eq!(
        serde_json::to_value(&minimal).unwrap(),
        json!({"image": "docker://redis", "env": {}, "attach": false})
    );
    let full = serde_json::to_value(private_spec()).unwrap();
    assert_eq!(full["registry"]["username"], "robot-user");
    assert_eq!(full["args"], json!(["serve", "--token=cmd-secret"]));
    let back: ContainerSpec = serde_json::from_value(full).unwrap();
    assert_eq!(back, private_spec());
}

#[test]
fn provision_request_carries_an_optional_container() {
    let plain: ProvisionRequest = serde_json::from_value(json!({})).unwrap();
    assert!(plain.container.is_none());
    assert!(serde_json::to_value(&plain).unwrap().get("container").is_none());
    let with: ProvisionRequest = serde_json::from_value(json!({"container": {"image": "docker://redis"}})).unwrap();
    assert_eq!(with.container.unwrap().image, "docker://redis");
}

#[test]
fn container_states_are_snake_case_on_the_wire() {
    for (state, wire) in [
        (ContainerState::Pulling, "pulling"),
        (ContainerState::Staging, "staging"),
        (ContainerState::Staged, "staged"),
        (ContainerState::Starting, "starting"),
        (ContainerState::Running, "running"),
        (ContainerState::Exited, "exited"),
        (ContainerState::Failed, "failed"),
    ] {
        assert_eq!(serde_json::to_value(state).unwrap(), json!(wire));
    }
}

#[test]
fn a_surface_appears_only_for_an_image_that_declares_one() {
    let terminal = ContainerStatusResponse {
        state: ContainerState::Running,
        image: "docker://redis".into(),
        digest: None,
        exit_code: None,
        error: None,
        surface: None,
        resolved: None,
    };
    assert!(serde_json::to_value(&terminal).unwrap().get("surface").is_none());
    let pending = ContainerStatusResponse {
        surface: Some(ContainerSurface {
            kind: ContainerSurfaceKind::Xpra,
            port: 14500,
            exposure_id: None,
        }),
        ..terminal
    };
    assert_eq!(
        serde_json::to_value(&pending).unwrap()["surface"],
        json!({"kind": "xpra", "port": 14500})
    );
    let granted: ContainerStatusResponse = serde_json::from_value(json!({
        "state": "running",
        "image": "docker://claude-desktop",
        "surface": {"kind": "xpra", "port": 14500, "exposure_id": "0199df26-d0f2-74f2-a304-ef67b79d1217"},
    }))
    .unwrap();
    assert_eq!(
        granted.surface.unwrap().exposure_id.as_deref(),
        Some("0199df26-d0f2-74f2-a304-ef67b79d1217")
    );
}
