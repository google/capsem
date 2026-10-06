use super::*;

#[tokio::test]
async fn catalog_observation_schedules_one_owned_worker_and_refuses_rebinding() {
    use capsem_foundation::poll::{poll_until, PollOpts};
    let _env_lock = crate::tests::SETTINGS_ENV_LOCK.lock().await;
    let root = tempfile::tempdir().unwrap();
    let _paths = capsem_foundation::paths::CapsemPathsGuard::redirect(root.path());
    let parent = root.path().join("layouts");
    std::fs::create_dir(&parent).unwrap();
    let source = RegistryImages::default();
    let key = capsem_assets::oci::CacheIdentity::new(
        &format!("localhost/team/image@sha256:{}", "d".repeat(64)),
        stage::oci_architecture().unwrap(),
        capsem_assets::oci::RUNTIME_CONTRACT,
    )
    .unwrap()
    .key();
    source.observe_cache(std::slice::from_ref(&key), &parent).unwrap();
    let cache = source.cache().unwrap();
    poll_until(
        PollOpts::new("service-cache-observed", std::time::Duration::from_secs(2)),
        || async { (cache.snapshot(&key).unwrap().state == capsem_assets::oci::CacheState::Missing).then_some(()) },
    )
    .await
    .unwrap();
    let observed = cache.snapshot(&key).unwrap();
    let clone = source.clone();
    clone.observe_cache(std::slice::from_ref(&key), &parent).unwrap();
    assert_eq!(clone.cache().unwrap().snapshot(&key).unwrap(), observed);
    assert!(clone
        .observe_cache(std::slice::from_ref(&key), &root.path().join("different"))
        .is_err());
    assert!(clone.observe_cache(&vec![key.clone(); 1025], &parent).is_err());
    assert_eq!(cache.snapshot(&key).unwrap(), observed);
    assert_eq!(std::fs::read_dir(parent).unwrap().count(), 0);
}

#[tokio::test]
async fn request_pullers_retain_the_service_owned_cache_observation() {
    let _env_lock = crate::tests::SETTINGS_ENV_LOCK.lock().await;
    let root = tempfile::tempdir().unwrap();
    let _paths = capsem_foundation::paths::CapsemPathsGuard::redirect(root.path());
    let parent = root.path().join("layouts");
    std::fs::create_dir(&parent).unwrap();
    let source = RegistryImages::default();
    let reference = format!("localhost/team/image@sha256:{}", "d".repeat(64));
    let key = capsem_assets::oci::CacheIdentity::new(
        &reference,
        stage::oci_architecture().unwrap(),
        capsem_assets::oci::RUNTIME_CONTRACT,
    )
    .unwrap()
    .key();
    let first = source.registry_puller(RegistryAccess::default()).unwrap();
    let observed = first.reconcile_cache(&key, &parent).await.unwrap();
    assert_eq!(observed.state, capsem_assets::oci::CacheState::Missing);
    drop(first);
    let second = source
        .registry_puller(RegistryAccess {
            username: Some("account".into()),
            password: Some("request-secret".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        second.cache_snapshot(&key).unwrap(),
        observed,
        "a new transport must retain service owner observation"
    );
    assert!(second.pull_cached(&reference, &parent).await.is_err());
    drop(second);
    let third = source.registry_puller(RegistryAccess::default()).unwrap();
    let changed = third.cache_snapshot(&key).unwrap();
    assert!(changed.verification_pending);
    assert!(changed.epoch > observed.epoch);
    assert!(source
        .registry_puller(RegistryAccess {
            username: Some("account".into()),
            ..Default::default()
        })
        .is_err());
    assert_eq!(std::fs::read_dir(parent).unwrap().count(), 0);
}
