use super::*;

const FULL_DRAFT: &str = r#"
name='development'
description='All capabilities'
[image]
reference='development:stable'
update_policy='ask'
[startup]
script="printf 'exact bytes'\n"
timeout_seconds=120
[variables]
PROJECT='ordinary-variable-canary'
[secrets.api]
binding_id='00000000-0000-4000-8000-000000000001'
env_name='API_KEY'
[[repositories]]
url='https://example.org/team/project.git'
ref='main'
destination='project'
[packages]
allows_package_manager=true
registries=['npm']
[[skills.selected]]
id='sandbox'
version='1.0.0'
[[access.catalog.selected]]
id='access-preset'
version='1.0.0'
[access.parameters.access-preset]
host='api.example'
[access.custom.custom-access]
name='Custom access'
action='ask'
match='http.host == "api.example"'
priority=120
[[detection.catalog.selected]]
id='detection-preset'
version='1.0.0'
[detection.parameters.detection-preset]
enabled=true
[detection.custom.custom-detection]
name='Custom detection'
level='high'
match='http.host == "api.example"'
[[mcps.catalog.selected]]
id='catalog-server'
version='1.0.0'
[mcps.servers.remote]
kind='remote'
url='https://mcp.example.org/rpc'
[mcps.permissions.remote]
default='ask'
[mcps.permissions.remote.tools]
read='allow'
[mcps.permissions.catalog-server]
default='block'
[plugins.pii_filter.outbound]
mode='rewrite'
detection_level='high'
[plugins.pii_filter.outbound.options]
email=false
[plugins.credential_broker.options]
capture_enabled=false
"#;

#[test]
fn resolved_environment_into_draft_preserves_every_intent_field_and_moves_large_buffers() {
    let mut original: EnvironmentDraft = toml::from_str(FULL_DRAFT).unwrap();
    original
        .variables
        .insert("PROJECT".into(), "variable-canary".repeat(512));
    original.startup.as_mut().unwrap().script = "printf 'script-canary'\n".repeat(1024);
    original.validate().unwrap();
    let mut resolved = toml::Value::try_from(&original).unwrap();
    resolved["image"]
        .as_table_mut()
        .unwrap()
        .insert("resolved_digest".into(), format!("sha256:{}", "a".repeat(64)).into());
    resolved["repositories"][0]
        .as_table_mut()
        .unwrap()
        .insert("resolved_commit".into(), "b".repeat(40).into());
    for (section, kind) in [
        ("skills", "skills"),
        ("access", "rules"),
        ("detection", "rules"),
        ("mcps", "mcp"),
    ] {
        let section = &mut resolved[section];
        let catalog = if kind == "skills" {
            section
        } else {
            &mut section["catalog"]
        };
        let selected = catalog["selected"][0].as_table_mut().unwrap();
        selected.insert(
            "catalog_reference".into(),
            format!("registry.example/{kind}:stable").into(),
        );
        selected.insert("resolved_digest".into(), format!("sha256:{}", "c".repeat(64)).into());
    }
    let resolved: EnvironmentSpecification = resolved.try_into().unwrap();
    resolved.validate().unwrap();
    let variable_address = resolved.variables["PROJECT"].as_ptr();
    let script_address = resolved.startup.as_ref().unwrap().script.as_ptr();
    let draft = resolved.into_draft();
    draft.validate().unwrap();
    assert_eq!(draft, original);
    assert_eq!(draft.variables["PROJECT"].as_ptr(), variable_address);
    assert_eq!(draft.startup.as_ref().unwrap().script.as_ptr(), script_address);
    let encoded = toml::to_string(&draft).unwrap();
    assert!(
        !encoded.contains("resolved_digest")
            && !encoded.contains("resolved_commit")
            && !encoded.contains("catalog_reference")
    );
}

#[test]
fn resolved_environment_into_draft_preserves_empty_optional_sections() {
    let resolved = document().specification;
    let draft = resolved.into_draft();
    draft.validate().unwrap();
    assert!(draft.image.is_none() && draft.startup.is_none());
    assert!(draft.repositories.is_empty() && draft.skills.selected.is_empty());
    assert_eq!(draft.variables["NODE_ENV"], "development");
}
