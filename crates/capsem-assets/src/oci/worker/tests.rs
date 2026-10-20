use super::super::{
    pull::tests::{digest, Registry},
    CacheIdentity, CacheState, ImageCache, RUNTIME_CONTRACT,
};
use capsem_foundation::poll::{poll_until, PollOpts};
use std::time::Duration;

// Receipt proof hashes cache content on a background worker. Full coverage
// runs can leave that worker CPU-starved well beyond the subsecond focused
// runtime; keep cancellation tests on their tighter deadlines below.
const CACHE_PROOF_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::test]
async fn inventory_worker_publishes_only_quiet_bounded_observations_and_tracks_nested_changes() {
    let root = super::super::tests::private_dir();
    let parent = tempfile::tempdir().unwrap();
    let cache = ImageCache::at(root.path()).unwrap();
    cache.inner.prepare().await.unwrap();
    let nested = root.path().join("unmanaged");
    std::fs::create_dir(&nested).unwrap();
    let file = nested.join("payload");
    std::fs::write(&file, vec![0u8; 8192]).unwrap();
    let worker = cache.reconciler("arm64", parent.path().to_owned(), 1).unwrap();
    assert!(cache.inventory_snapshot().unwrap().is_none());
    assert!(worker.request_inventory(0).is_err());
    assert!(worker.request_inventory(64).unwrap());
    assert!(!worker.request_inventory(64).unwrap());
    let inventory = poll_until(PollOpts::new("inventory-observed", Duration::from_secs(2)), || async {
        cache.inventory_snapshot().unwrap()
    })
    .await
    .unwrap();
    assert!(inventory.usage.allocated_bytes >= 8192);
    assert!(inventory.images.is_empty());
    assert!(std::sync::Arc::ptr_eq(
        &inventory,
        &cache.inventory_snapshot().unwrap().unwrap()
    ));
    assert!(!worker.request_inventory(64).unwrap());
    // Modification through an unrelated hardlink changes allocation without
    // replacing the cache's directory entry.
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::hard_link(&file, elsewhere.path().join("alias")).unwrap();
    assert!(cache.inventory_snapshot().unwrap().is_none());
    assert!(worker.request_inventory(64).unwrap());
    poll_until(PollOpts::new("linked-inventory", Duration::from_secs(2)), || async {
        cache.inventory_snapshot().unwrap()
    })
    .await
    .unwrap();
    std::fs::write(elsewhere.path().join("alias"), vec![0u8; 32768]).unwrap();
    assert!(cache.inventory_snapshot().unwrap().is_none());
    assert!(worker.request_inventory(64).unwrap());
    let updated = poll_until(PollOpts::new("changed-inventory", Duration::from_secs(2)), || async {
        cache.inventory_snapshot().unwrap()
    })
    .await
    .unwrap();
    assert!(updated.usage.allocated_bytes > inventory.usage.allocated_bytes);
    assert!(cache.refresh_inventory(1).await.is_err());
    assert!(cache.inventory_snapshot().unwrap().is_none());
    assert_eq!(std::fs::read(&file).unwrap().len(), 32768);
}

#[tokio::test]
async fn inventory_observations_follow_root_bindings_and_cancel_when_the_worker_owner_drops() {
    let outer = super::super::tests::private_dir();
    let root = outer.path().join("holder/cache");
    let parent = tempfile::tempdir().unwrap();
    let cache = ImageCache::at(&root).unwrap();
    let worker = cache.reconciler("arm64", parent.path().to_owned(), 1).unwrap();
    assert!(worker.request_inventory(32).unwrap());
    poll_until(PollOpts::new("root-inventory", Duration::from_secs(2)), || async {
        cache.inventory_snapshot().unwrap()
    })
    .await
    .unwrap();
    std::fs::rename(&root, outer.path().join("retired")).unwrap();
    assert!(cache.inventory_snapshot().unwrap().is_none());
    poll_until(
        PollOpts::new("replacement-inventory-request", Duration::from_secs(1)),
        || async { worker.request_inventory(32).unwrap().then_some(()) },
    )
    .await
    .unwrap();
    let rebound = poll_until(
        PollOpts::new("replacement-inventory", Duration::from_secs(2)),
        || async { cache.inventory_snapshot().unwrap() },
    )
    .await
    .unwrap();
    assert!(root.is_dir());
    assert!(rebound.images.is_empty());
    std::fs::rename(outer.path().join("holder"), outer.path().join("old-holder")).unwrap();
    assert!(cache.inventory_snapshot().unwrap().is_none());
    cache.refresh_inventory(32).await.unwrap();
    assert!(cache.inventory_snapshot().unwrap().is_some());
    let held = cache.inner.mutation_lease().await.unwrap();
    let key = CacheIdentity::new(
        &format!("localhost/team/image@sha256:{}", "d".repeat(64)),
        "arm64",
        RUNTIME_CONTRACT,
    )
    .unwrap()
    .key();
    let before = cache.snapshot(&key).unwrap().epoch;
    poll_until(
        PollOpts::new("blocked-inventory-request", Duration::from_secs(1)),
        || async { worker.request_inventory(32).unwrap().then_some(()) },
    )
    .await
    .unwrap();
    let active = poll_until(
        PollOpts::new("blocked-inventory-started", Duration::from_secs(1)),
        || async {
            let epoch = cache.snapshot(&key).unwrap().epoch;
            (epoch > before).then_some(epoch)
        },
    )
    .await
    .unwrap();
    assert!(!worker.request_inventory(32).unwrap());
    drop(worker);
    poll_until(
        PollOpts::new("inventory-worker-cancelled", Duration::from_secs(1)),
        || async { (cache.snapshot(&key).unwrap().epoch > active).then_some(()) },
    )
    .await
    .unwrap();
    drop(held);
    assert!(cache.inventory_snapshot().unwrap().is_none());
    cache.refresh_inventory(32).await.unwrap();
    assert!(cache.inventory_snapshot().unwrap().is_some());
}

#[tokio::test]
async fn cancelled_inventory_refresh_cannot_invalidate_a_newer_owner_epoch() {
    let root = super::super::tests::private_dir();
    let cache = ImageCache::at(root.path()).unwrap();
    cache.inner.prepare().await.unwrap();
    let held = cache.inner.mutation_lease().await.unwrap();
    let key = CacheIdentity::new(
        &format!("localhost/team/image@sha256:{}", "d".repeat(64)),
        "arm64",
        RUNTIME_CONTRACT,
    )
    .unwrap()
    .key();
    let before = cache.snapshot(&key).unwrap().epoch;
    let first_cache = cache.clone();
    let first = tokio::spawn(async move { first_cache.refresh_inventory(32).await });
    let active = poll_until(
        PollOpts::new("first-inventory-epoch", Duration::from_secs(1)),
        || async {
            let epoch = cache.snapshot(&key).unwrap().epoch;
            (epoch > before).then_some(epoch)
        },
    )
    .await
    .unwrap();
    let second_cache = cache.clone();
    let second = tokio::spawn(async move { second_cache.refresh_inventory(32).await });
    let newer = poll_until(
        PollOpts::new("second-inventory-epoch", Duration::from_secs(1)),
        || async {
            let epoch = cache.snapshot(&key).unwrap().epoch;
            (epoch > active).then_some(epoch)
        },
    )
    .await
    .unwrap();
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    assert_eq!(cache.snapshot(&key).unwrap().epoch, newer);
    drop(held);
    second.await.unwrap().unwrap();
    assert!(cache.inventory_snapshot().unwrap().is_some());
}

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
        let observed = poll_until(PollOpts::new("foreign-cache-observed", CACHE_PROOF_TIMEOUT), || async {
            let observed = cache.snapshot(&key).unwrap();
            (!observed.verification_pending).then_some(observed)
        })
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
            PollOpts::new("foreign-cache-reobserved", CACHE_PROOF_TIMEOUT),
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
    assert!(worker.request_inventory(64).unwrap());
    poll_until(PollOpts::new("worker-ready", Duration::from_secs(2)), || async {
        keys.iter()
            .all(|key| cache.snapshot(key).unwrap().state == CacheState::Ready)
            .then_some(())
    })
    .await
    .unwrap();
    let inventory = poll_until(
        PollOpts::new("ready-image-inventory", Duration::from_secs(2)),
        || async { cache.inventory_snapshot().unwrap() },
    )
    .await
    .unwrap();
    assert_eq!(inventory.images.len(), 2);
    assert!(inventory
        .images
        .iter()
        .all(|image| image.snapshot.state == CacheState::Ready));
    let independent = cache.snapshot(&key_two).unwrap();
    let materialization = cache.inner.materialization_lease(&key_one).await.unwrap();
    assert!(cache.inventory_snapshot().unwrap().is_none());
    assert_eq!(cache.snapshot(&key_two).unwrap(), independent);
    drop(materialization);
    assert!(cache.inventory_snapshot().unwrap().is_none());
    assert_eq!(cache.snapshot(&key_two).unwrap(), independent);
    poll_until(
        PollOpts::new("released-inventory-request", Duration::from_secs(1)),
        || async { worker.request_inventory(64).unwrap().then_some(()) },
    )
    .await
    .unwrap();
    poll_until(PollOpts::new("released-inventory", Duration::from_secs(2)), || async {
        cache.inventory_snapshot().unwrap()
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
