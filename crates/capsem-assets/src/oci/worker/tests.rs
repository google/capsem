use super::super::{
    pull::tests::{digest, Registry},
    CacheIdentity, CacheState, ImageCache, RUNTIME_CONTRACT,
};
use capsem_foundation::poll::{poll_until, PollOpts};
use std::time::Duration;

#[tokio::test]
async fn incompatible_receipts_are_observed_once_until_their_metadata_changes() {
    let registry = Registry::start(|_, _| {}).await;
    let root = super::super::tests::private_dir();
    let parent = tempfile::tempdir().unwrap();
    let cache = ImageCache::at(root.path()).unwrap();
    let puller = registry.puller().with_cache(&cache);
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let original = puller
        .cached_receipt(&image.cache_identity().key())
        .await
        .unwrap()
        .unwrap();
    drop(image);
    registry.task.abort();
    let scoped = cache
        .inner
        .for_repository(&super::super::image_reference(&registry.reference()).unwrap());
    std::fs::write(
        root.path()
            .join("blobs")
            .join(scoped.entry_name(&digest(&registry.layer)).unwrap()),
        b"corrupt payload",
    )
    .unwrap();
    for (architecture, contract) in [("amd64", RUNTIME_CONTRACT), ("arm64", RUNTIME_CONTRACT + 1)] {
        let key = CacheIdentity::new(&original.identity().image().to_string(), architecture, contract)
            .unwrap()
            .key();
        let mut record: serde_json::Value = serde_json::from_slice(&original.encode().unwrap()).unwrap();
        record["key"] = serde_json::json!(key.as_str());
        record["architecture"] = serde_json::json!(architecture);
        record["runtime_contract"] = serde_json::json!(contract);
        let path = root.path().join("blobs").join(format!("receipt-{}", key.as_str()));
        let bytes = serde_json::to_vec(&record).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let worker = cache.reconciler("arm64", parent.path().to_owned(), 1).unwrap();
        assert!(worker.request(std::slice::from_ref(&key)).unwrap());
        let observed = poll_until(
            PollOpts::new("foreign-cache-observed", Duration::from_secs(2)),
            || async {
                let observed = cache.snapshot(&key).unwrap();
                (!observed.verification_pending).then_some(observed)
            },
        )
        .await
        .unwrap();
        assert_eq!(observed.state, CacheState::Unknown);
        assert_eq!(
            observed.reason,
            Some(if architecture != "arm64" {
                super::super::CacheReason::UnsupportedPlatform
            } else {
                super::super::CacheReason::IncompatibleRuntime
            })
        );
        assert!(observed.verified_at_unix_ns.is_none());
        assert!(!worker.request(std::slice::from_ref(&key)).unwrap());
        assert_eq!(cache.snapshot(&key).unwrap(), observed);
        std::fs::write(&path, bytes).unwrap();
        assert!(cache.snapshot(&key).unwrap().verification_pending);
        poll_until(PollOpts::new("foreign-cache-retry", Duration::from_secs(1)), || async {
            worker.request(std::slice::from_ref(&key)).unwrap().then_some(())
        })
        .await
        .unwrap();
        poll_until(
            PollOpts::new("foreign-cache-reobserved", Duration::from_secs(2)),
            || async { (!cache.snapshot(&key).unwrap().verification_pending).then_some(()) },
        )
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn queued_work_coalesces_capacity_refuses_and_last_owner_drop_cancels_proof() {
    let root = super::super::tests::private_dir();
    let parent = tempfile::tempdir().unwrap();
    let cache = ImageCache::at(root.path()).unwrap();
    cache.inner.prepare().await.unwrap();
    let key = CacheIdentity::new(
        &format!("localhost/team/image@sha256:{}", "d".repeat(64)),
        "arm64",
        RUNTIME_CONTRACT,
    )
    .unwrap()
    .key();
    assert!(cache.reconciler("arm64", parent.path().to_owned(), 0).is_err());
    let worker = cache.reconciler("arm64", parent.path().to_owned(), 2).unwrap();
    let held = cache.inner.mutation_lease().await.unwrap();
    let before = cache.snapshot(&key).unwrap().epoch;
    assert!(worker.request(std::slice::from_ref(&key)).unwrap());
    let active = poll_until(PollOpts::new("worker-started", Duration::from_secs(1)), || async {
        let epoch = cache.snapshot(&key).unwrap().epoch;
        (epoch > before).then_some(epoch)
    })
    .await
    .unwrap();
    assert!(!worker.request(std::slice::from_ref(&key)).unwrap());
    assert!(worker.request(&[key.clone(), key.clone(), key.clone()]).is_err());
    assert_eq!(cache.snapshot(&key).unwrap().epoch, active);
    let replacement = CacheIdentity::new(
        &format!("localhost/team/image@sha256:{}", "c".repeat(64)),
        "arm64",
        RUNTIME_CONTRACT,
    )
    .unwrap()
    .key();
    assert!(worker.request(std::slice::from_ref(&replacement)).unwrap());
    let active = poll_until(PollOpts::new("replacement-started", Duration::from_secs(1)), || async {
        let epoch = cache.snapshot(&replacement).unwrap().epoch;
        (epoch >= active + 2).then_some(epoch)
    })
    .await
    .unwrap();
    assert!(!worker.request(std::slice::from_ref(&replacement)).unwrap());
    let retained = worker.clone();
    drop(worker);
    tokio::task::yield_now().await;
    assert_eq!(cache.snapshot(&key).unwrap().epoch, active);
    drop(retained);
    poll_until(PollOpts::new("worker-cancelled", Duration::from_secs(1)), || async {
        (cache.snapshot(&key).unwrap().epoch > active).then_some(())
    })
    .await
    .unwrap();
    assert!(cache.snapshot(&key).unwrap().verification_pending);
    drop(held);
}

#[tokio::test]
async fn worker_reconciles_multiple_offline_images_and_only_retries_changed_observations() {
    let first = Registry::start(|_, _| {}).await;
    let second = Registry::start(|_, _| {}).await;
    let root = super::super::tests::private_dir();
    let parent = tempfile::tempdir().unwrap();
    let cache = ImageCache::at(root.path()).unwrap();
    let mut one = first.puller().with_cache(&cache);
    let image = one.pull(&first.reference(), parent.path()).await.unwrap();
    let key_one = image.cache_identity().key();
    drop(image);
    one = second.puller().with_cache(&cache);
    let image = one.pull(&second.reference(), parent.path()).await.unwrap();
    let key_two = image.cache_identity().key();
    drop(image);
    first.task.abort();
    second.task.abort();
    let worker = cache.reconciler("arm64", parent.path().to_owned(), 2).unwrap();
    let keys = [key_one.clone(), key_two.clone()];
    assert!(worker.request(&keys).unwrap());
    poll_until(PollOpts::new("worker-ready", Duration::from_secs(2)), || async {
        keys.iter()
            .all(|key| cache.snapshot(key).unwrap().state == CacheState::Ready)
            .then_some(())
    })
    .await
    .unwrap();
    // The worker may still be finishing its last publication; a repeated
    // request must neither restart that work nor schedule quiet observations.
    assert!(!worker.request(&keys).unwrap());
    let scoped = cache
        .inner
        .for_repository(&super::super::image_reference(&second.reference()).unwrap());
    let layer = root
        .path()
        .join("blobs")
        .join(scoped.entry_name(&digest(&second.layer)).unwrap());
    std::fs::write(&layer, b"corrupt layer").unwrap();
    assert!(cache.snapshot(&key_two).unwrap().verification_pending);
    poll_until(
        PollOpts::new("worker-repair-request", Duration::from_secs(1)),
        || async { worker.request(&keys).unwrap().then_some(()) },
    )
    .await
    .unwrap();
    poll_until(PollOpts::new("worker-partial", Duration::from_secs(2)), || async {
        (cache.snapshot(&key_two).unwrap().state == CacheState::Partial).then_some(())
    })
    .await
    .unwrap();
    std::fs::write(&layer, &second.layer).unwrap();
    assert!(cache.snapshot(&key_two).unwrap().verification_pending);
    poll_until(
        PollOpts::new("worker-retry-request", Duration::from_secs(1)),
        || async { worker.request(&keys).unwrap().then_some(()) },
    )
    .await
    .unwrap();
    poll_until(PollOpts::new("worker-repaired", Duration::from_secs(2)), || async {
        keys.iter()
            .all(|key| cache.snapshot(key).unwrap().state == CacheState::Ready)
            .then_some(())
    })
    .await
    .unwrap();
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
}
