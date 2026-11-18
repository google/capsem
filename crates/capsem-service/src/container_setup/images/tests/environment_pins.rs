//! Environment pinning consumes the same admitted result as image prefetch.
use super::*;

fn saved_image(reference: &str, selected: char) -> capsem_config::environment::EnvironmentImage {
    capsem_config::environment::EnvironmentImage {
        reference: reference.into(),
        resolved_digest: digest(selected),
        update_policy: Default::default(),
    }
}

#[tokio::test]
async fn saved_image_selection_retains_catalog_history_and_never_resolves_a_moving_tag() {
    let fx = Fixture::default_policy();
    let mut decision = Decision::new(&fx.state).await.unwrap();
    let selected = decision.resolve_saved(&saved_image("codex-cli", 'a')).await.unwrap();
    assert_eq!(
        selected.pull,
        format!("ghcr.io/google/capsem/codex-cli@{}", digest('a'))
    );
    assert_eq!(
        decision.admit_saved(&selected).await.unwrap().digest().to_string(),
        digest('a')
    );
    assert!(
        fx.pulls().is_empty(),
        "selecting an immutable pin does not pull an image"
    );

    let fx = Fixture::new(grants(&["registry.example"], &["registry.example"]));
    let mut decision = Decision::new(&fx.state).await.unwrap();
    let selected = decision
        .resolve_saved(&saved_image("registry.example/app:moving", 'a'))
        .await
        .unwrap();
    assert_eq!(selected.pull, format!("registry.example/app@{}", digest('a')));
    decision.admit_saved(&selected).await.unwrap();
    assert!(fx.pulls().is_empty());
    assert!(decision
        .resolve_saved(&saved_image(&format!("registry.example/app@{}", digest('b')), 'a'))
        .await
        .is_err());
}

#[tokio::test]
async fn saved_image_selection_refuses_unlisted_incompatible_and_wrong_platform_pins() {
    for (reference, selected) in [
        ("codex-cli", '0'),
        ("codex-cli", 'c'),
        ("elsewhere", 'e'),
        ("unknown", 'a'),
    ] {
        let fx = Fixture::default_policy();
        let mut decision = Decision::new(&fx.state).await.unwrap();
        assert!(
            decision.resolve_saved(&saved_image(reference, selected)).await.is_err(),
            "{reference}:{selected}"
        );
        assert!(fx.pulls().is_empty());
    }
    let fx = Fixture::default_policy();
    let mut decision = Decision::new(&fx.state).await.unwrap();
    let selected = decision
        .resolve_saved(&saved_image("registry.example/app:moving", 'a'))
        .await
        .unwrap();
    assert!(
        decision.admit_saved(&selected).await.is_err(),
        "saved pins do not grant source access"
    );
    assert!(fx.pulls().is_empty());
}

fn request(image: &str) -> ImagePullRequest {
    ImagePullRequest {
        image: image.into(),
        registry: None,
    }
}

#[tokio::test]
async fn native_pin_preserves_image_index_identity_separately_from_platform_manifest() {
    let fx = Fixture::default_policy();
    let admitted = pull_image(&fx.state, request("codex-cli")).await.unwrap();
    assert_eq!(admitted.resolved.digest().to_string(), digest('b'));
    assert_eq!(admitted.platform_digest, digest('7'));
    assert_eq!(
        fx.pulls(),
        vec![format!("ghcr.io/google/capsem/codex-cli@{}", digest('b'))]
    );
    assert_eq!(*fx.fetches.lock().unwrap(), vec![ImageFetch::Fresh]);

    let fx = Fixture::new(grants(&["registry.example"], &["registry.example"]));
    let admitted = pull_image(&fx.state, request("registry.example/app:moving"))
        .await
        .unwrap();
    assert_eq!(admitted.resolved.digest().to_string(), digest('f'));
    assert_eq!(admitted.platform_digest, digest('7'));
}

#[tokio::test]
async fn native_pin_refuses_source_before_pull_and_unlisted_digest_before_filesystem() {
    let fx = Fixture::default_policy();
    let error = pull_image(&fx.state, request("registry.example/app:moving"))
        .await
        .err()
        .unwrap();
    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert!(fx.pulls().is_empty());
    let error = pull_image(
        &fx.state,
        request(&format!("ghcr.io/google/capsem/codex-cli@{}", digest('0'))),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert_eq!(fx.pulls().len(), 1);
    assert!(fx.roots.lock().unwrap().is_empty());
}

#[tokio::test]
async fn native_pin_cannot_skip_a_required_published_filesystem() {
    let fx = Fixture::default_policy();
    fx.catalog.lock().unwrap().as_mut().unwrap()["entries"]["codex-cli"]["versions"][1]["erofs"] =
        json!({host(): digest('8')});
    let error = pull_image(&fx.state, request("codex-cli")).await.err().unwrap();
    assert_eq!(error.status, StatusCode::BAD_GATEWAY);
    assert_eq!(fx.roots.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn native_pin_keeps_the_shared_operation_deadline() {
    let fx = Fixture::default_policy();
    *fx.pull_delay.lock().unwrap() = std::time::Duration::from_secs(capsem_api::IMAGE_PULL_TIMEOUT_SECS + 1);
    let started = fx.pull_started.clone();
    let state = fx.state.clone();
    let task = tokio::spawn(async move { pull_image(&state, request("codex-cli")).await });
    started.notified().await;
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(capsem_api::IMAGE_PULL_TIMEOUT_SECS)).await;
    tokio::time::resume();
    let error = task.await.unwrap().err().unwrap();
    assert_eq!(error.status, StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(error.body.timeout_secs, Some(capsem_api::IMAGE_PULL_TIMEOUT_SECS));
    assert!(fx.roots.lock().unwrap().is_empty());
}

#[tokio::test]
async fn operator_creation_freezes_the_native_image_pin_and_never_refreshes_on_retry() {
    let fx = Fixture::default_policy();
    let intent = json!({"request_id":"00000000-0000-4000-8000-000000000100", "specification":{"name":"development","image":{"reference":"codex-cli"}}, "reason":"create with a verified image"});
    let (status, body) = fx
        .call(axum::http::Method::POST, "/environments/create", Some(intent.clone()))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result: capsem_api::EnvironmentMutationResponse = serde_json::from_value(body.clone()).unwrap();
    let current = fx
        .state
        .environments
        .load_current(&result.environment_id)
        .await
        .unwrap();
    assert_eq!(
        current.document.specification.image.unwrap().resolved_digest,
        digest('b')
    );
    assert_eq!(fx.pulls().len(), 1);
    // Neither a newer tag nor loss of source access can change a committed retry.
    *fx.catalog.lock().unwrap() = None;
    let (status, repeat) = fx
        .call(axum::http::Method::POST, "/environments/create", Some(intent))
        .await;
    assert_eq!(status, StatusCode::OK, "{repeat}");
    assert_eq!(repeat, body);
    assert_eq!(fx.pulls().len(), 1);
}

#[tokio::test]
async fn operator_creation_preserves_native_source_policy_refusal() {
    let fx = Fixture::default_policy();
    let intent = json!({"request_id":"00000000-0000-4000-8000-000000000100", "specification":{"name":"development","image":{"reference":"registry.example/app:stable"}}, "reason":"test source policy"});
    let (status, _) = fx
        .call(axum::http::Method::POST, "/environments/create", Some(intent))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(fx.pulls().is_empty());
    assert!(fx
        .state
        .host_ledger
        .environment_request_snapshot("00000000-0000-4000-8000-000000000100")
        .await
        .unwrap()
        .is_none());
}
