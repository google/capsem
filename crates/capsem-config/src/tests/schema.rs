use crate::{environment::*, plugins::*};
use serde_json::Value;
use utoipa::{OpenApi, ToSchema};

#[derive(OpenApi)]
#[openapi(components(schemas(EnvironmentSpecification<EnvironmentImageSelection, EnvironmentRepositorySelection, EnvironmentCatalogSelection>, EnvironmentDocument, EnvironmentLimits, PluginDescriptor)))]
struct Contract;

fn schemas() -> Value {
    serde_json::to_value(Contract::openapi()).unwrap()["components"]["schemas"].take()
}

// Generic arguments may be inlined. Inspect the collected schema rather than
// assuming every argument also has its own named component.
fn object_with_property<'a>(value: &'a Value, name: &str, schemas: &'a Value) -> Option<&'a Value> {
    match value {
        Value::Object(object) => {
            if object
                .get("properties")
                .and_then(Value::as_object)
                .is_some_and(|properties| properties.contains_key(name))
            {
                return Some(value);
            }
            if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                return object_with_property(
                    &schemas[reference.strip_prefix("#/components/schemas/").unwrap()],
                    name,
                    schemas,
                );
            }
            object
                .values()
                .find_map(|value| object_with_property(value, name, schemas))
        }
        Value::Array(values) => values
            .iter()
            .find_map(|value| object_with_property(value, name, schemas)),
        _ => None,
    }
}
fn resolve_references(value: &Value, schemas: &Value) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                let name = reference
                    .strip_prefix("#/components/schemas/")
                    .expect("local component reference");
                assert!(schemas.get(name).is_some(), "missing nested schema {name}");
            }
            for value in object.values() {
                resolve_references(value, schemas);
            }
        }
        Value::Array(values) => {
            for value in values {
                resolve_references(value, schemas);
            }
        }
        _ => {}
    }
}

#[test]
fn environment_schemas_include_every_nested_contract_and_distinct_draft_identity() {
    let schemas = schemas();
    resolve_references(&schemas, &schemas);
    let specifications = schemas
        .as_object()
        .unwrap()
        .iter()
        .filter(|(name, _)| name.starts_with("EnvironmentSpecification"))
        .collect::<Vec<_>>();
    assert_eq!(
        specifications.len(),
        2,
        "selectors and resolved pins require distinct schema identities"
    );
    let draft = specifications
        .iter()
        .find(|(name, _)| name.contains("EnvironmentImageSelection"))
        .expect("draft schema")
        .1;
    assert!(draft["properties"].get("name").is_some());
    assert!(draft["properties"].get("revision_id").is_none());
    assert!(draft["properties"].get("plugins").is_some());
    assert!(draft["properties"].get("mcps").is_some());
    let image = object_with_property(&draft["properties"]["image"], "reference", &schemas).expect("inline draft image");
    assert!(image["properties"].get("resolved_digest").is_none());
    let canonical = specifications
        .iter()
        .find(|(name, _)| !name.contains("EnvironmentImageSelection"))
        .unwrap()
        .1;
    let pin = object_with_property(&canonical["properties"]["skills"], "id", &schemas).expect("canonical skill pin");
    assert!(pin["properties"].get("resolved_digest").is_some());
    assert!(pin["properties"].get("catalog_reference").is_some());
    let selector = object_with_property(&draft["properties"]["skills"], "id", &schemas).expect("draft skill selector");
    assert!(selector["properties"].get("resolved_digest").is_none());
    assert!(selector["properties"].get("catalog_reference").is_none());
    assert_eq!(selector["additionalProperties"], false);
}

#[test]
fn plugin_schemas_keep_nested_directions_and_specific_options() {
    let schemas = schemas();
    let pii = &schemas[PiiFilterConfiguration::name().as_ref()];
    assert!(pii["properties"].get("inbound").is_some());
    assert!(pii["properties"].get("outbound").is_some());
    assert!(pii["properties"].get("mode").is_none());
    let capture = object_with_property(
        &schemas[EnvironmentPlugins::name().as_ref()],
        "capture_enabled",
        &schemas,
    )
    .expect("typed broker options");
    assert!(capture["properties"].get("capture_enabled").is_some());
    assert!(capture["properties"].get("email").is_none());
    assert!(schemas.get(PluginFieldDependency::name().as_ref()).is_some());
    assert!(schemas.get(crate::SettingMetadata::name().as_ref()).is_some());
}

fn dereference<'a>(value: &'a Value, schemas: &'a Value) -> &'a Value {
    if let Some(reference) = value.get("$ref").and_then(Value::as_str) {
        &schemas[reference.strip_prefix("#/components/schemas/").unwrap()]
    } else {
        value
    }
}

#[test]
fn plugin_and_access_schemas_expose_only_supported_decisions() {
    let schemas = schemas();
    for (name, modes) in [
        (PiiFilterOptions::name(), PiiFilterOptions::MODES),
        (SecretExfiltrationOptions::name(), SecretExfiltrationOptions::MODES),
        (Utf8GuardOptions::name(), Utf8GuardOptions::MODES),
        (CredentialBrokerOptions::name(), CredentialBrokerOptions::MODES),
        (LogSanitizerOptions::name(), LogSanitizerOptions::MODES),
        (DummyPreEicarOptions::name(), DummyPreEicarOptions::MODES),
        (DummyPostAllowOptions::name(), DummyPostAllowOptions::MODES),
    ] {
        let configuration = &schemas[format!("PluginConfiguration_{name}")];
        let mode = dereference(&configuration["properties"]["mode"], &schemas);
        assert_eq!(
            mode["enum"],
            serde_json::to_value(modes.iter().map(|mode| mode.as_str()).collect::<Vec<_>>()).unwrap(),
            "{name}"
        );
    }
    let access = object_with_property(&schemas, "action", &schemas).unwrap();
    let action = dereference(&access["properties"]["action"], &schemas);
    assert_eq!(action["enum"], serde_json::json!(["allow", "ask", "block"]));
}
