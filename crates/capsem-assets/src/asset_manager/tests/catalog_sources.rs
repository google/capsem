use super::*;

#[test]
fn runtime_manifest_view_preserves_selected_catalog_sources_in_both_formats() {
    let sources = serde_json::json!({
        "skills":{"reference":"ghcr.io/google/capsem/skills:stable","schema_version":1},
        "rules":{"reference":"ghcr.io/google/capsem/rules:stable","schema_version":1},
        "mcp":{"reference":format!("ghcr.io/google/capsem/mcps@sha256:{}", "a".repeat(64)),"schema_version":1},
    });
    let public = runtime_graph().to_string();
    for raw in [SAMPLE_V2_MANIFEST, &public] {
        let mut document: serde_json::Value = serde_json::from_str(raw).unwrap();
        document["catalogs"] = sources.clone();
        let parsed = ManifestV2::from_json(&document.to_string()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap()["catalogs"], sources);
    }
}

#[test]
fn absent_catalog_sources_do_not_change_legacy_runtime_manifest_serialization() {
    let parsed = ManifestV2::from_json(SAMPLE_V2_MANIFEST).unwrap();
    assert!(serde_json::to_value(parsed).unwrap().get("catalogs").is_none());
}
