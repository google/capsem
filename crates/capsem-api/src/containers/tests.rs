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
        profile_id: "code".into(),
        ram_mb: None,
        cpus: None,
        persistent: false,
        env: None,
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
    let plain: ProvisionRequest = serde_json::from_value(json!({"profile_id": "code"})).unwrap();
    assert!(plain.container.is_none());
    assert!(serde_json::to_value(&plain).unwrap().get("container").is_none());
    let with: ProvisionRequest =
        serde_json::from_value(json!({"profile_id": "code", "container": {"image": "docker://redis"}})).unwrap();
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
