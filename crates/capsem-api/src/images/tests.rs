use super::*;
use serde_json::json;

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
