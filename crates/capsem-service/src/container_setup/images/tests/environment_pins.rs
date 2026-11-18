//! Environment pinning consumes the same admitted result as image prefetch.
use super::*;

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
