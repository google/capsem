use std::path::{Path, PathBuf};

use capsem_core::profile_payload_schema::{
    validate_profile_payload_v2_json, validate_profile_payload_v2_toml, ProfilePayloadSchemaError,
};
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
    assert!(schema["properties"].get("mcp").is_none());
    assert_eq!(
        schema["properties"]["mcpServers"],
        serde_json::json!({ "$ref": "#/$defs/mcp_servers" })
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

#[test]
fn profile_v2_json_validation_helper_accepts_valid_fixture() {
    let path = repo_root().join("schemas/fixtures/profile-v2-valid.json");
    let input = std::fs::read_to_string(&path).unwrap();

    let value = validate_profile_payload_v2_json(&input).unwrap();

    assert_eq!(value["schema"], "capsem.profile.v2");
    assert_eq!(
        value["mcpServers"]["github"]["command"],
        serde_json::json!("npx")
    );
}

#[test]
fn profile_v2_json_validation_helper_reports_invalid_fixture() {
    let path = repo_root().join("schemas/fixtures/profile-v2-invalid-asset-hash.json");
    let input = std::fs::read_to_string(&path).unwrap();

    let error = validate_profile_payload_v2_json(&input).unwrap_err();

    assert!(matches!(error, ProfilePayloadSchemaError::Validation(_)));
    assert!(error.to_string().contains("blake3"));
}

#[test]
fn profile_v2_toml_validation_helper_bridges_through_json_schema() {
    let input = r#"
schema = "capsem.profile.v2"
version = 2
id = "everyday-work"
revision = "2026.0520.1"
name = "Everyday Work"
description = "Balanced defaults for day-to-day work."
best_for = "Balanced defaults for day-to-day work."
profile_type = "everyday-work"

[compatibility]
min_binary = "1.0.0"
guest_abi = "capsem-guest-v2"

[vm]
memory_mib = 8192
cpus = 4
disk_mib = 32768
network = "proxied"

[vm.assets.arm64.kernel]
url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/arm64/vmlinuz"
hash = "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
signature_url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/arm64/vmlinuz.minisig"
size = 7797248
content_type = "application/octet-stream"

[vm.assets.arm64.initrd]
url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/arm64/initrd.img"
hash = "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
signature_url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/arm64/initrd.img.minisig"
size = 2270154
content_type = "application/octet-stream"

[vm.assets.arm64.rootfs]
url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/arm64/rootfs.squashfs"
hash = "blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
signature_url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/arm64/rootfs.squashfs.minisig"
size = 454230016
content_type = "application/vnd.squashfs"

[packages.runtimes]
python = "3.12.3"

[packages.system]
distro = "debian"
release = "bookworm"

[tools.capsem_doctor]
version = "2026.05.18"
required = true
source = "guest"

[security.capabilities]
credential_brokerage = "ask"
"#;

    let value = validate_profile_payload_v2_toml(input).unwrap();

    assert_eq!(value["id"], "everyday-work");
    assert_eq!(value["vm"]["assets"]["arm64"]["rootfs"]["size"], 454230016);
}
