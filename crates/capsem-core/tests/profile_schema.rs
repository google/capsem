use std::path::{Path, PathBuf};

use serde_json::Value;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("capsem-core crate should live under <repo>/crates/capsem-core")
        .to_path_buf()
}

fn read_json(path: &Path) -> Value {
    let input = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_str(&input)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

fn profile_schema() -> (Value, jsonschema::Validator) {
    let path = repo_root().join("schemas/capsem.profile.v2.schema.json");
    let schema = read_json(&path);
    let validator = jsonschema::validator_for(&schema)
        .unwrap_or_else(|error| panic!("profile schema must compile: {error}"));
    (schema, validator)
}

#[test]
fn profile_v2_schema_is_closed_draft_2020_12_contract() {
    let (schema, _) = profile_schema();

    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(
        schema["$id"],
        "https://schemas.capsem.dev/capsem.profile.v2.schema.json"
    );
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["$defs"]["hash"]["pattern"], "^blake3:[0-9a-f]{64}$");
    assert_eq!(
        schema["$defs"]["tool"]["required"],
        serde_json::json!(["version", "required", "source"])
    );
}

#[test]
fn profile_v2_schema_accepts_valid_golden_fixture() {
    let (_, validator) = profile_schema();
    let fixture = read_json(&repo_root().join("schemas/fixtures/profile-v2-valid.json"));

    let errors = validator
        .iter_errors(&fixture)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();

    assert!(
        errors.is_empty(),
        "valid profile fixture failed: {errors:?}"
    );
}

#[test]
fn profile_v2_schema_rejects_invalid_golden_fixtures() {
    let (_, validator) = profile_schema();

    for name in [
        "profile-v2-invalid-asset-hash.json",
        "profile-v2-invalid-extra-field.json",
        "profile-v2-invalid-tool-missing-version.json",
    ] {
        let fixture = read_json(&repo_root().join("schemas/fixtures").join(name));

        assert!(
            !validator.is_valid(&fixture),
            "invalid profile fixture unexpectedly passed: {name}"
        );
    }
}
