use super::*;
use serde_json::json;

#[test]
fn inventory_contract_keeps_unavailable_separate_from_empty_or_broken_data() {
    let query: ImageInventoryQuery = serde_json::from_value(json!({"image":"registry.example/app@sha256:a"})).unwrap();
    assert!(!query.image.is_empty());
    let unavailable = json!({"image":query.image,"architecture":"arm64"});
    let parsed: ImageInventoryResponse = serde_json::from_value(unavailable.clone()).unwrap();
    assert!(parsed.inventory.is_none());
    assert_eq!(serde_json::to_value(parsed).unwrap(), unavailable);
    let document = crate::openapi().to_json().unwrap();
    let document: serde_json::Value = serde_json::from_str(&document).unwrap();
    assert_eq!(
        document["paths"]["/images/{name}/inventory"]["get"]["operationId"],
        "getImageInventory"
    );
}

#[test]
fn image_download_progress_survives_the_wire_without_inventing_an_unknown_total() {
    let mut row = json!({"name":"app","description":"App","architectures":["arm64"],"cached":"missing", "progress":{"phase":"downloading","completed_bytes":128}});
    let parsed: ImageInfo = serde_json::from_value(row.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap()["progress"], row["progress"]);
    row["progress"]["total_bytes"] = json!(1024);
    let parsed: ImageInfo = serde_json::from_value(row.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap()["progress"], row["progress"]);
    row["progress"]["phase"] = json!("pretend-complete");
    assert!(serde_json::from_value::<ImageInfo>(row).is_err());
}

#[test]
fn image_interfaces_survive_the_gateway_contract() {
    let mut row = json!({"name":"app","description":"App","architectures":["arm64"],"cached":"missing"});
    for surface in ["terminal", "xpra"] {
        row["surface"] = json!(surface);
        let parsed: ImageInfo = serde_json::from_value(row.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap()["surface"], surface);
    }
    row["surface"] = json!("unknown-interface");
    assert!(serde_json::from_value::<ImageInfo>(row).is_err());
}

#[test]
fn image_disk_states_round_trip_and_unrecognized_states_are_refused() {
    for state in ["unknown", "missing", "partial", "ready"] {
        let parsed: ImageCacheState = serde_json::from_value(json!(state)).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), json!(state));
    }
    assert!(serde_json::from_value::<ImageCacheState>(json!("available")).is_err());
}

#[test]
fn an_image_list_without_a_catalog_is_empty_and_says_so() {
    let empty: ImageListResponse = serde_json::from_value(json!({"images": []})).unwrap();
    assert_eq!(
        empty,
        ImageListResponse {
            catalog: None,
            images: Vec::new()
        }
    );
    assert_eq!(serde_json::to_value(&empty).unwrap(), json!({"images": []}));
}

#[test]
fn an_image_entry_carries_its_pin_and_an_unanswered_cache_state() {
    let entry = ImageInfo {
        name: "codex-cli".into(),
        description: "Codex".into(),
        architectures: vec!["amd64".into(), "arm64".into()],
        image: Some(format!("ghcr.io/google/capsem/codex-cli@sha256:{}", "a".repeat(64))),
        cached: ImageCacheState::Unknown,
        previous_image: None,
        app: None,
        surface: None,
        progress: None,
    };
    let wire = serde_json::to_value(&entry).unwrap();
    assert_eq!(wire["cached"], "unknown");
    assert_eq!(serde_json::from_value::<ImageInfo>(wire).unwrap(), entry);
    let unrunnable = ImageInfo { image: None, ..entry };
    assert!(serde_json::to_value(&unrunnable).unwrap().get("image").is_none());
}

#[test]
fn a_pull_request_never_shows_its_registry_secret() {
    let request: ImagePullRequest = serde_json::from_value(json!({
        "image": "codex-cli",
        "registry": {"username": "robot", "password": "registry-password"}
    }))
    .unwrap();
    let shown = format!("{request:?}");
    assert!(shown.contains("codex-cli"), "{shown}");
    assert!(
        !shown.contains("robot") && !shown.contains("registry-password"),
        "{shown}"
    );
    let minimal: ImagePullRequest = serde_json::from_value(json!({"image": "dev"})).unwrap();
    assert_eq!(minimal.registry, None);
}

#[test]
fn the_list_query_reads_the_cached_catalog_unless_asked() {
    let query: ImageListQuery = serde_json::from_value(json!({})).unwrap();
    assert!(!query.refresh);
}

#[test]
fn prior_image_is_optional_and_preserves_a_verified_previous_pin() {
    let mut row = json!({"name":"app","description":"App","architectures":["arm64"],"cached":"missing"});
    let old: ImageInfo = serde_json::from_value(row.clone()).unwrap();
    assert!(serde_json::to_value(old).unwrap().get("previous_image").is_none());
    row["previous_image"] = json!(format!("registry.example/app@sha256:{}", "a".repeat(64)));
    let updated: ImageInfo = serde_json::from_value(row.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(updated).unwrap()["previous_image"],
        row["previous_image"]
    );
}

#[test]
fn image_app_metadata_preserves_exact_identity_and_explicitly_absent_artifacts() {
    let row = json!({"name":"dev","description":"Tools","architectures":["arm64"],"cached":"missing",
        "app":{"catalog_name":"dev","image":format!("registry.example/dev@sha256:{}", "a".repeat(64)),
        "display_name":"Development","description":"An agentless environment with development tools.",
        "architecture":"arm64","interfaces":["terminal"],"evidence":[]}});
    let parsed: ImageInfo = serde_json::from_value(row.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap()["app"], row["app"]);
}

#[test]
fn session_wire_preserves_the_pinned_source_app_snapshot() {
    let mut row = serde_json::to_value(crate::SandboxInfo::new(
        "session".into(),
        1,
        crate::VmLifecycleState::Running,
        true,
    ))
    .unwrap();
    row["app"] = json!({"catalog_name":"dev","image":format!("registry.example/dev@sha256:{}", "a".repeat(64)),
        "display_name":"Development","description":"Tools","architecture":"arm64","interfaces":["terminal"],"evidence":[]});
    let parsed: crate::SandboxInfo = serde_json::from_value(row.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap()["app"], row["app"]);
}
