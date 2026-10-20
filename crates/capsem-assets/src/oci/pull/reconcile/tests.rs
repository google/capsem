use super::super::tests::{digest, Registry};
use super::super::*;

#[tokio::test]
async fn reconciliation_checks_complete_image_and_bound_root_without_registry() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = super::super::super::tests::private_dir();
    let mut puller = registry.puller();
    puller.cache = Some(BlobCache::at(root.path()).unwrap());
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    let reference = registry.published_filesystem(&image.source_digest, None);
    let filesystem = puller
        .fetch_rootfs(
            &reference,
            &ContentDigest::parse(&image.source_digest).unwrap(),
            parent.path(),
        )
        .await
        .unwrap();
    puller.retain_cached_root(&key, &filesystem).await.unwrap();
    let expected = puller.cached_receipt(&key).await.unwrap().unwrap();
    drop(filesystem);
    drop(image);
    registry.task.abort();
    let checked = puller
        .reconcile_cached_receipt(&key, parent.path())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(checked.generation().unwrap(), expected.generation().unwrap());
    let receipt_path = root.path().join("blobs").join(format!("receipt-{}", key.as_str()));
    let mut forged: serde_json::Value = serde_json::from_slice(&expected.encode().unwrap()).unwrap();
    for blob in forged["root"]["blobs"].as_array_mut().unwrap() {
        if blob["kind"] == "immutable_root" {
            blob["digest"] = serde_json::json!(digest(b"different filesystem"));
        }
    }
    std::fs::write(&receipt_path, serde_json::to_vec(&forged).unwrap()).unwrap();
    assert!(puller.cached_receipt(&key).await.unwrap().is_some());
    assert!(
        puller.reconcile_cached_receipt(&key, parent.path()).await.is_err(),
        "receipt filesystem facts must match the verified artifact"
    );
    assert_eq!(
        std::fs::read_dir(parent.path()).unwrap().count(),
        0,
        "verification layouts must be reclaimed"
    );
    let unknown = super::super::super::CacheIdentity::new(
        &format!("localhost/missing@{}", digest(b"missing")),
        "amd64",
        RUNTIME_CONTRACT,
    )
    .unwrap()
    .key();
    assert!(puller
        .reconcile_cached_receipt(&unknown, parent.path())
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn receipts_for_another_architecture_or_runtime_are_not_materialized() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = super::super::super::tests::private_dir();
    let mut puller = registry.puller();
    puller.cache = Some(BlobCache::at(root.path()).unwrap());
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let original = puller
        .cached_receipt(&image.cache_identity().key())
        .await
        .unwrap()
        .unwrap();
    drop(image);
    registry.task.abort();
    let other_architecture = if puller.architecture == "arm64" {
        "amd64"
    } else {
        "arm64"
    };
    for (architecture, contract) in [
        (other_architecture, RUNTIME_CONTRACT),
        (puller.architecture.as_str(), RUNTIME_CONTRACT + 1),
    ] {
        let key =
            super::super::super::CacheIdentity::new(&original.identity().image().to_string(), architecture, contract)
                .unwrap()
                .key();
        let mut forged: serde_json::Value = serde_json::from_slice(&original.encode().unwrap()).unwrap();
        forged["key"] = serde_json::json!(key.as_str());
        forged["architecture"] = serde_json::json!(architecture);
        forged["runtime_contract"] = serde_json::json!(contract);
        std::fs::write(
            root.path().join("blobs").join(format!("receipt-{}", key.as_str())),
            serde_json::to_vec(&forged).unwrap(),
        )
        .unwrap();
        assert!(puller.cached_receipt(&key).await.unwrap().is_some());
        let error = puller.reconcile_cached_receipt(&key, parent.path()).await.unwrap_err();
        assert!(format!("{error:#}").contains("different native runtime"));
        assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn incomplete_receipt_and_changed_missing_or_nonregular_bytes_do_not_reconcile() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = super::super::super::tests::private_dir();
    let mut puller = registry.puller();
    puller.cache = Some(BlobCache::at(root.path()).unwrap());
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    drop(image);
    let receipt = root.path().join("blobs").join(format!("receipt-{}", key.as_str()));
    let valid = std::fs::read(&receipt).unwrap();
    let mut forged: serde_json::Value = serde_json::from_slice(&valid).unwrap();
    forged["blobs"]
        .as_array_mut()
        .unwrap()
        .retain(|blob| blob["digest"] != digest(&registry.layer));
    std::fs::write(&receipt, serde_json::to_vec(&forged).unwrap()).unwrap();
    assert!(
        puller.cached_receipt(&key).await.unwrap().is_some(),
        "structural validation alone cannot detect omitted layers"
    );
    registry.task.abort();
    assert!(puller.reconcile_cached_receipt(&key, parent.path()).await.is_err());
    std::fs::write(&receipt, &valid).unwrap();
    let cache = puller
        .cache
        .as_ref()
        .unwrap()
        .for_repository(&image_reference(&registry.reference()).unwrap());
    let layer = root
        .path()
        .join("blobs")
        .join(cache.entry_name(&digest(&registry.layer)).unwrap());
    std::fs::write(&layer, vec![b'x'; registry.layer.len()]).unwrap();
    assert!(puller.reconcile_cached_receipt(&key, parent.path()).await.is_err());
    std::fs::remove_file(&layer).unwrap();
    assert!(puller.reconcile_cached_receipt(&key, parent.path()).await.is_err());
    let foreign = parent.path().join("foreign");
    std::fs::write(&foreign, &registry.layer).unwrap();
    std::os::unix::fs::symlink(&foreign, &layer).unwrap();
    assert!(puller.reconcile_cached_receipt(&key, parent.path()).await.is_err());
    assert_eq!(std::fs::read(&foreign).unwrap(), registry.layer);
    std::fs::remove_file(&foreign).unwrap();
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
}
