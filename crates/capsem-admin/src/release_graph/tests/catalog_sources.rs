use super::*;

fn graph() -> serde_json::Value {
    serde_json::json!({"version":"0.7.3","channel":"stable","status":"current","packages":[],"runtime":{"revision":"runtime-fixture","status":"current","architectures":[]},"catalogs":{
        "skills":{"reference":"ghcr.io/google/capsem/skills:stable","schema_version":1},
        "rules":{"reference":"ghcr.io/google/capsem/rules:stable","schema_version":1},
        "mcp":{"reference":format!("ghcr.io/google/capsem/mcp@sha256:{}", "a".repeat(64)),"schema_version":1}
    }})
}

#[test]
fn release_manifest_catalog_sources_roundtrip_without_changing_release_families() {
    let original = graph();
    let manifest: ReleaseManifest = serde_json::from_value(original.clone()).unwrap();
    manifest.validate_catalog_sources().unwrap();
    let restored = serde_json::to_value(&manifest).unwrap();
    assert_eq!(restored["catalogs"], original["catalogs"]);
    assert_eq!(restored["runtime"], original["runtime"]);
    assert_eq!(restored["packages"], original["packages"]);
    let mut without = original;
    without.as_object_mut().unwrap().remove("catalogs");
    let manifest: ReleaseManifest = serde_json::from_value(without).unwrap();
    assert!(serde_json::to_value(manifest).unwrap().get("catalogs").is_none());
}

#[test]
fn release_manifest_catalog_sources_reject_invalid_schema_oci_and_unknown_fields() {
    for (reference, version) in [
        ("ghcr.io/google/capsem/skills:stable", 2),
        ("private-source-canary", 1),
        ("registry.example/project@sha256:private-source-canary", 1),
    ] {
        let mut value = graph();
        value["catalogs"]["skills"] = serde_json::json!({"reference":reference,"schema_version":version});
        let manifest: ReleaseManifest = serde_json::from_value(value).unwrap();
        let error = manifest.validate_inventory_shape().unwrap_err().to_string();
        assert!(error.contains("catalog"), "{error}");
        assert!(!error.contains("canary"));
    }
    let mut unknown = graph();
    unknown["catalogs"]["skills"]["unknown"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ReleaseManifest>(unknown).is_err());
    let mut unknown = graph();
    unknown["catalogs"]["unknown"] =
        serde_json::json!({"reference":"registry.example/catalog:stable","schema_version":1});
    assert!(serde_json::from_value::<ReleaseManifest>(unknown).is_err());
}

#[test]
fn release_manifest_rejects_a_tag_only_mcp_catalog_source() {
    let mut value = graph();
    value["catalogs"]["mcp"]["reference"] = serde_json::json!("ghcr.io/google/capsem/mcp:stable");
    let manifest: ReleaseManifest = serde_json::from_value(value).unwrap();
    let error = manifest.validate_catalog_sources().unwrap_err();
    assert!(format!("{error:#}").contains("MCP catalog source requires a SHA-256 OCI manifest digest"));
}
