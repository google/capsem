use super::*;

const BASE: &str = r#"schema_version = 1
id = "6f3601ee-04df-4c1b-9828-dfd2015c38d4"
revision_id = "e7ea9d09-0b0b-4f3f-bb09-c9b438f543ef"
name = "development"
description = "Project development sandbox"
[variables]
NODE_ENV = "development"
"#;

#[test]
fn canonical_environment_toml_roundtrips_without_history() {
    let document = EnvironmentDocument::from_toml(BASE).unwrap();
    let text = document.to_toml().unwrap();
    assert!(!text.contains("[identity]") && !text.contains("[specification]"));
    assert!(EnvironmentDocument::from_toml(&text).unwrap() == document);
}

#[test]
fn canonical_environment_rejects_bad_identity_and_unbounded_variables() {
    for source in [
        BASE.replace("schema_version = 1", "schema_version = 2"),
        BASE.replace("development\"\ndescription", "bad/name\"\ndescription"),
        BASE.replace("6f3601ee-04df-4c1b-9828-dfd2015c38d4", "not-a-uuid"),
        BASE.replace(
            "NODE_ENV = \"development\"",
            &format!(
                "NODE_ENV = {:?}",
                "x".repeat(capsem_proto::ENVIRONMENT_MAX_VARIABLE_VALUE_BYTES + 1)
            ),
        ),
    ] {
        assert!(
            EnvironmentDocument::from_toml(&source).is_err(),
            "invalid environment accepted"
        );
    }
}

#[test]
fn editable_specification_refuses_revision_metadata_and_raw_secret_fields() {
    assert!(toml::from_str::<EnvironmentSpecification>("name='dev'\nrevision_id='caller' ").is_err());
    assert!(EnvironmentDocument::from_toml(&format!(
        "{BASE}\n[secrets.api]\nbinding_id='reference'\nvalue='raw-secret'\n"
    ))
    .is_err());
}

fn document() -> EnvironmentDocument {
    EnvironmentDocument::from_toml(BASE).unwrap()
}

fn with_image() -> EnvironmentDocument {
    let mut document = document();
    document.specification.image = Some(EnvironmentImage {
        reference: "development".into(),
        resolved_digest: format!("sha256:{}", "a".repeat(64)),
        update_policy: EnvironmentImageUpdatePolicy::Manual,
    });
    document
}

fn repository(destination: &str) -> EnvironmentRepository {
    EnvironmentRepository {
        url: "https://example.org/team/project.git".into(),
        reference: "main".into(),
        resolved_commit: "a".repeat(40),
        destination: destination.into(),
    }
}

#[test]
fn environment_name_description_and_startup_enforce_exact_byte_limits() {
    let mut doc = with_image();
    doc.specification.name = "a".repeat(capsem_proto::ENVIRONMENT_MAX_NAME_BYTES);
    doc.specification.description = "é".repeat(capsem_proto::ENVIRONMENT_MAX_DESCRIPTION_BYTES / 2);
    doc.specification.startup = Some(EnvironmentStartup {
        script: "x".repeat(capsem_proto::ENVIRONMENT_MAX_STARTUP_SCRIPT_BYTES),
        timeout_seconds: 120,
    });
    assert!(doc.to_toml().is_ok());
    let mut bad = doc.clone();
    bad.specification.name.push('x');
    assert!(bad.to_toml().is_err());
    let mut bad = doc.clone();
    bad.specification.description.push('x');
    assert!(bad.to_toml().is_err());
    let mut bad = doc.clone();
    bad.specification.startup.as_mut().unwrap().script.push('x');
    assert!(bad.to_toml().is_err());
    let mut bad = doc.clone();
    bad.specification.startup.as_mut().unwrap().timeout_seconds = 0;
    assert!(bad.to_toml().is_err());
    doc.specification.image = None;
    assert!(doc.to_toml().is_err());
}

#[test]
fn environment_variable_names_values_counts_and_aggregate_are_exact() {
    use capsem_proto::*;
    let mut doc = document();
    doc.specification.variables.clear();
    doc.specification.variables.insert(
        "A".repeat(ENVIRONMENT_MAX_VARIABLE_NAME_BYTES),
        "x".repeat(ENVIRONMENT_MAX_VARIABLE_VALUE_BYTES),
    );
    assert!(doc.to_toml().is_ok());
    for name in ["", "1START", "A=B", "A\nB", "LD_PRELOAD"] {
        let mut bad = doc.clone();
        bad.specification.variables.insert(name.into(), "value".into());
        assert!(bad.to_toml().is_err());
    }
    let mut bad = doc.clone();
    bad.specification
        .variables
        .insert("A".repeat(ENVIRONMENT_MAX_VARIABLE_NAME_BYTES + 1), "".into());
    assert!(bad.to_toml().is_err());
    let mut bad = doc.clone();
    bad.specification
        .variables
        .insert("TOO_LONG".into(), "x".repeat(ENVIRONMENT_MAX_VARIABLE_VALUE_BYTES + 1));
    assert!(bad.to_toml().is_err());
    let mut bad = doc.clone();
    bad.specification
        .variables
        .insert("NUL".into(), "private-value\0".into());
    assert!(bad.to_toml().is_err());
    doc.specification.variables = (0..ENVIRONMENT_MAX_VARIABLES)
        .map(|i| (format!("A{i}"), String::new()))
        .collect();
    assert!(doc.to_toml().is_ok());
    doc.specification.variables.insert("EXTRA".into(), String::new());
    assert!(doc.to_toml().is_err());
    doc.specification.variables = (0..16)
        .map(|i| (format!("A{i:04}"), "x".repeat(ENVIRONMENT_MAX_VARIABLE_VALUE_BYTES - 5)))
        .collect();
    assert!(doc.to_toml().is_ok());
    doc.specification.variables.get_mut("A0000").unwrap().push('x');
    assert!(doc.to_toml().is_err());
}

#[test]
fn environment_binding_references_never_collide_with_plain_variables() {
    let mut doc = document();
    doc.specification.secrets.insert(
        "api".into(),
        EnvironmentSecretBinding {
            binding_id: "965fc048-6480-4d4b-b00b-fbdd6e81aa0d".into(),
            env_name: Some("API_KEY".into()),
        },
    );
    assert!(doc.to_toml().is_ok());
    let mut bad = doc.clone();
    bad.specification
        .variables
        .insert("API_KEY".into(), "accidental-token".into());
    assert!(bad.to_toml().is_err());
    let mut bad = doc.clone();
    bad.specification
        .secrets
        .insert("other".into(), bad.specification.secrets["api"].clone());
    assert!(bad.to_toml().is_err());
    doc.specification.secrets.get_mut("api").unwrap().binding_id = "raw-secret-not-a-reference".into();
    assert!(doc.to_toml().is_err());
}

#[test]
fn environment_fork_lineage_is_complete_and_canonical() {
    let mut doc = document();
    doc.identity.parent_environment_id = Some("6b5edf5b-a39b-49f9-823e-a15a8e19549a".into());
    assert!(doc.to_toml().is_err());
    doc.identity.parent_revision_id = Some("c5a891db-eb5c-4ffd-a6ab-11212cbdb32e".into());
    assert!(doc.to_toml().is_ok());
    let mut bad = doc.clone();
    bad.identity.parent_environment_id = Some(bad.identity.id.clone());
    assert!(bad.to_toml().is_err());
    doc.identity.revision_id = doc.identity.revision_id.to_uppercase();
    assert!(doc.to_toml().is_err());
}

#[test]
fn environment_repositories_refuse_credentials_options_and_unsafe_destinations() {
    let mut doc = with_image();
    doc.specification.repositories.push(repository("project"));
    assert!(doc.to_toml().is_ok());
    for url in [
        "https://user:private-password@example.org/a.git",
        "https://example.org/a.git?token=private-token",
        "file:///host/path",
        "https://example.org/a.git#token",
    ] {
        let mut bad = doc.clone();
        bad.specification.repositories[0].url = url.into();
        assert!(bad.to_toml().is_err());
    }
    for reference in ["--upload-pack=anything", "a..b", "a\nb", "refs//main"] {
        let mut bad = doc.clone();
        bad.specification.repositories[0].reference = reference.into();
        assert!(bad.to_toml().is_err());
    }
    for destination in ["../project", "/project", "a//b", "a/./b", "a\\b", "project\ncontrol"] {
        let mut bad = doc.clone();
        bad.specification.repositories[0].destination = destination.into();
        assert!(bad.to_toml().is_err(), "accepted unsafe destination");
    }
    for destination in ["project", "project/nested"] {
        let mut bad = doc.clone();
        bad.specification.repositories.push(repository(destination));
        assert!(bad.to_toml().is_err());
    }
    doc.specification.repositories.push(repository("project-other"));
    assert!(doc.to_toml().is_ok());
}

#[test]
fn environment_repository_count_and_full_document_bytes_are_bounded() {
    let mut doc = with_image();
    doc.specification.repositories = (0..capsem_proto::ENVIRONMENT_MAX_REPOSITORIES)
        .map(|i| repository(&format!("repo{i}")))
        .collect();
    assert!(doc.to_toml().is_ok());
    doc.specification.repositories.push(repository("extra"));
    assert!(doc.to_toml().is_err());
    doc.specification.repositories.pop();
    doc.specification.repositories[0].url = format!(
        "https://example.org/{}",
        "a".repeat(capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES)
    );
    assert!(doc.to_toml().is_err());
    assert!(EnvironmentDocument::from_toml(&" ".repeat(capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES + 1)).is_err());
}

#[test]
fn environment_errors_and_debug_never_echo_variable_or_script_values() {
    let mut doc = with_image();
    doc.specification
        .variables
        .insert("PRIVATE".into(), "private-value-canary".into());
    doc.specification.startup = Some(EnvironmentStartup {
        script: "private-script-canary".into(),
        timeout_seconds: 120,
    });
    let debug = format!("{doc:?}");
    assert!(!debug.contains("private-value-canary") && !debug.contains("private-script-canary"));
    doc.specification.variables.get_mut("PRIVATE").unwrap().push('\0');
    assert!(!doc.to_toml().unwrap_err().contains("private-value-canary"));
    assert!(!EnvironmentDocument::from_toml("name = 'private-parse-canary\n")
        .unwrap_err()
        .contains("private-parse-canary"));
}

#[test]
fn environment_history_and_legacy_configuration_fail_loudly() {
    for section in ["history", "profiles", "connections", "mcp"] {
        assert!(EnvironmentDocument::from_toml(&format!("{BASE}\n[{section}]\nvalue='legacy'\n")).is_err());
    }
}

#[test]
fn environment_editor_metadata_reuses_compiled_runtime_limits() {
    use capsem_proto::*;
    let limits = EnvironmentLimits::CURRENT;
    assert_eq!(limits.max_name_bytes, ENVIRONMENT_MAX_NAME_BYTES);
    assert_eq!(limits.max_description_bytes, ENVIRONMENT_MAX_DESCRIPTION_BYTES);
    assert_eq!(limits.max_startup_script_bytes, ENVIRONMENT_MAX_STARTUP_SCRIPT_BYTES);
    assert_eq!(limits.max_variables, ENVIRONMENT_MAX_VARIABLES);
    assert_eq!(limits.max_variable_name_bytes, ENVIRONMENT_MAX_VARIABLE_NAME_BYTES);
    assert_eq!(limits.max_variable_value_bytes, ENVIRONMENT_MAX_VARIABLE_VALUE_BYTES);
    assert_eq!(limits.max_variable_total_bytes, ENVIRONMENT_MAX_VARIABLE_TOTAL_BYTES);
    assert_eq!(limits.max_secret_bindings, ENVIRONMENT_MAX_VARIABLES);
    assert_eq!(limits.max_repositories, ENVIRONMENT_MAX_REPOSITORIES);
    assert_eq!(limits.max_spec_bytes, ENVIRONMENT_MAX_SPEC_BYTES);
    assert_eq!(limits.max_rules, ENVIRONMENT_MAX_RULES);
    assert_eq!(limits.max_package_registries, ENVIRONMENT_MAX_PACKAGE_REGISTRIES);
    assert_eq!(limits.max_mcp_servers, ENVIRONMENT_MAX_MCP_SERVERS);
    assert_eq!(limits.max_mcp_arguments, ENVIRONMENT_MAX_MCP_ARGUMENTS);
    assert_eq!(limits.max_mcp_tool_overrides, ENVIRONMENT_MAX_MCP_TOOL_OVERRIDES);
    assert_eq!(limits.max_rule_bytes, ENVIRONMENT_MAX_RULE_BYTES);
    assert_eq!(limits.max_configuration_entries, ENVIRONMENT_MAX_CONFIGURATION_ENTRIES);
    assert_eq!(
        limits.max_configuration_text_bytes,
        ENVIRONMENT_MAX_CONFIGURATION_TEXT_BYTES
    );
    assert_eq!(limits.max_catalog_selections, ENVIRONMENT_MAX_CATALOG_SELECTIONS);
    assert_eq!(
        limits.max_catalog_reference_bytes,
        ENVIRONMENT_MAX_CATALOG_REFERENCE_BYTES
    );
}

const DRAFT: &str = r#"name='development'
[image]
reference='development:stable'
[startup]
script='printf proposal-script-canary'
timeout_seconds=120
[variables]
PROJECT='proposal-variable-canary'
[[repositories]]
url='https://example.org/team/project.git'
ref='main'
destination='project'
"#;

#[test]
fn environment_draft_accepts_selectors_without_fabricated_resolution_pins() {
    let draft: EnvironmentDraft = toml::from_str(DRAFT).unwrap();
    draft.validate().unwrap();
    let encoded = toml::to_string(&draft).unwrap();
    assert!(!encoded.contains("resolved_"));
    assert!(
        toml::from_str::<EnvironmentSpecification>(DRAFT).is_err(),
        "canonical revisions still require exact pins"
    );
    assert!(!format!("{draft:?}").contains("proposal-variable-canary"));
    assert!(!format!("{draft:?}").contains("proposal-script-canary"));
}

#[test]
fn environment_draft_refuses_caller_resolution_identity_and_invalid_fields() {
    for source in [
        DRAFT.replace(
            "reference='development:stable'",
            "reference='development:stable'\nresolved_digest='sha256:forged'",
        ),
        DRAFT.replace("ref='main'", "ref='main'\nresolved_commit='forged'"),
        format!("revision_id='forged'\n{DRAFT}"),
        format!("actor='operator'\n{DRAFT}"),
    ] {
        assert!(toml::from_str::<EnvironmentDraft>(&source).is_err());
    }
    for source in [
        DRAFT.replace("development:stable", "bad reference"),
        DRAFT.replace("https://example.org/", "https://user:secret@example.org/"),
        DRAFT.replace("destination='project'", "destination='../project'"),
        DRAFT.replace("ref='main'", "ref='--upload-pack=bad'"),
    ] {
        let draft: EnvironmentDraft = toml::from_str(&source).unwrap();
        assert!(draft.validate().is_err());
    }
}

mod catalog;

mod rules;

mod mcps;

mod plugins;

mod package_registries;
