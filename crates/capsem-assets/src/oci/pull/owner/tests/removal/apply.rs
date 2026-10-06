use super::*;

#[tokio::test]
async fn completed_journal_compacts_ownership_evidence_and_preserves_exact_replay() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = crate::oci::tests::private_dir();
    let owner = ImageCache::at(root.path()).unwrap();
    let puller = registry.puller().with_cache(&owner);
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    drop(image);
    registry.task.abort();
    let preview = owner.preview_removal(&key).await.unwrap();
    interrupt(root.path(), &key, preview.token());
    let path = root.path().join(format!("removal-{}.json", preview.token()));
    let pending_size = std::fs::metadata(&path).unwrap().len();
    let result = owner
        .apply_removal(&key, preview.token(), "interrupted cleanup")
        .await
        .unwrap();
    let applied: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(
        applied.get("witness").is_none() && applied.get("receipt_json").is_none(),
        "completed history needs request/result replay, not payload ownership evidence"
    );
    assert!(std::fs::metadata(&path).unwrap().len() < pending_size);
    assert_eq!(
        ImageCache::at(root.path())
            .unwrap()
            .apply_removal(&key, preview.token(), "interrupted cleanup")
            .await
            .unwrap(),
        result
    );
    let mut corrupted = applied;
    corrupted["result"]["removed_allocated_bytes"] = serde_json::json!(result.removed_allocated_bytes + 1);
    std::fs::write(&path, serde_json::to_vec(&corrupted).unwrap()).unwrap();
    assert!(
        owner
            .apply_removal(&key, preview.token(), "interrupted cleanup")
            .await
            .is_err(),
        "compact replay must reject corrupted result fields"
    );
}

#[tokio::test]
async fn bounded_control_reservation_keeps_success_within_the_existing_capacity() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = crate::oci::tests::private_dir();
    let mut owner = ImageCache::at(root.path()).unwrap();
    let puller = registry.puller().with_cache(&owner);
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    drop(image);
    registry.task.abort();
    let used = owner.usage().await.unwrap().allocated_bytes;
    let maximum = used + 96 * 1024;
    owner.inner.set_test_capacity(used, maximum);
    let preview = owner.preview_removal(&key).await.unwrap();
    let oversized = "\0".repeat(crate::oci::METADATA_LIMIT / 2);
    assert!(
        owner.apply_removal(&key, preview.token(), &oversized).await.is_err(),
        "escaped intent must fit the metadata bound before any payload change"
    );
    assert!(puller.cached_receipt(&key).await.unwrap().is_some());
    assert!(!root.path().join(format!("removal-{}.json", preview.token())).exists());
    let result = owner
        .apply_removal(&key, preview.token(), &"r".repeat(8192))
        .await
        .unwrap();
    assert!(result.complete);
    assert!(owner.usage().await.unwrap().allocated_bytes <= maximum);
    assert!(!std::fs::read_dir(root.path()).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".partial-removal-")));
}

#[tokio::test]
async fn full_control_budget_refuses_intent_before_payload_mutation() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = crate::oci::tests::private_dir();
    let mut owner = ImageCache::at(root.path()).unwrap();
    let puller = registry.puller().with_cache(&owner);
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    drop(image);
    registry.task.abort();
    let before = owner.usage().await.unwrap().allocated_bytes;
    owner.inner.set_test_capacity(before / 2, before);
    let preview = owner.preview_removal(&key).await.unwrap();
    assert!(owner
        .apply_removal(&key, preview.token(), "no control headroom")
        .await
        .is_err());
    assert!(puller.cached_receipt(&key).await.unwrap().is_some());
    assert_eq!(owner.usage().await.unwrap().allocated_bytes, before);
    assert!(!root.path().join(format!("removal-{}.json", preview.token())).exists());
}

#[tokio::test]
async fn matching_checksum_cannot_authorize_an_unowned_journal_target() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = crate::oci::tests::private_dir();
    let owner = ImageCache::at(root.path()).unwrap();
    let puller = registry.puller().with_cache(&owner);
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    drop(image);
    registry.task.abort();
    let preview = owner.preview_removal(&key).await.unwrap();
    interrupt(root.path(), &key, preview.token());
    let unknown = root.path().join("blobs/unrelated");
    std::fs::write(&unknown, b"unowned bytes").unwrap();
    let mut journal: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.path().join(format!("removal-{}.json", preview.token()))).unwrap())
            .unwrap();
    journal["witness"]["blobs"][0]["name"] = serde_json::json!("unrelated");
    journal["witness"]["blobs"][0]["file"] = serde_json::to_value(
        crate::oci::removal::FileState::from_metadata(&std::fs::metadata(&unknown).unwrap()).unwrap(),
    )
    .unwrap();
    let witness: crate::oci::removal::Witness = serde_json::from_value(journal["witness"].clone()).unwrap();
    let forged_token = crate::oci::removal::preview_token(&witness).unwrap();
    journal["token"] = serde_json::json!(forged_token);
    std::fs::write(
        root.path().join(format!("removal-{forged_token}.json")),
        serde_json::to_vec(&journal).unwrap(),
    )
    .unwrap();
    assert!(owner
        .apply_removal(&key, &forged_token, "interrupted cleanup")
        .await
        .is_err());
    assert_eq!(
        std::fs::read(&unknown).unwrap(),
        b"unowned bytes",
        "journal targets must derive from the validated original receipt even with a matching witness checksum"
    );
}

fn interrupt(root: &std::path::Path, key: &crate::oci::CacheKey, token: &str) {
    interrupt_at(root, key, token, "after_receipt", 91);
}

fn interrupt_at(root: &std::path::Path, key: &crate::oci::CacheKey, token: &str, point: &str, exit_code: i32) {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "oci::pull::owner::tests::removal::apply::durable_removal_resumes_after_process_death",
        ])
        .env("CAPSEM_OCI_REMOVE_CHILD_ROOT", root)
        .env("CAPSEM_OCI_REMOVE_CHILD_KEY", key.as_str())
        .env("CAPSEM_OCI_REMOVE_CHILD_TOKEN", token)
        .env("CAPSEM_OCI_REMOVE_TEST_STOP", point)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(exit_code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn interrupted_unacknowledged_unlink_is_accounted_as_already_missing_on_retry() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = crate::oci::tests::private_dir();
    let owner = ImageCache::at(root.path()).unwrap();
    let puller = registry.puller().with_cache(&owner);
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    drop(image);
    registry.task.abort();
    let receipt_bytes = std::fs::metadata(root.path().join("blobs").join(format!("receipt-{}", key.as_str())))
        .unwrap()
        .blocks()
        * 512;
    let preview = owner.preview_removal(&key).await.unwrap();
    interrupt_at(root.path(), &key, preview.token(), "after_unlink", 92);
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.path().join(format!("removal-{}.json", preview.token()))).unwrap())
            .unwrap();
    assert!(state["pending"].is_object());
    assert_eq!(state["result"]["removed_entries"], 0);
    assert!(puller.cached_receipt(&key).await.unwrap().is_none());
    let result = ImageCache::at(root.path())
        .unwrap()
        .apply_removal(&key, preview.token(), "interrupted cleanup")
        .await
        .unwrap();
    assert!(result.complete);
    assert_eq!(result.already_missing_entries, 1);
    assert_eq!(
        result.removed_allocated_bytes + receipt_bytes,
        preview.reclaimable_bytes(),
        "retry must not invent allocation results that were never acknowledged"
    );
}

#[tokio::test]
async fn interrupted_removal_refuses_changed_inodes_before_further_deletion_and_preserves_republication() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = crate::oci::tests::private_dir();
    let owner = ImageCache::at(root.path()).unwrap();
    let puller = registry.puller().with_cache(&owner);
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    drop(image);
    registry.task.abort();
    let receipt = puller.cached_receipt(&key).await.unwrap().unwrap();
    let preview = owner.preview_removal(&key).await.unwrap();
    interrupt(root.path(), &key, preview.token());
    let scoped = owner.inner.for_repository(&image_reference(receipt.origin()).unwrap());
    let layer = root.path().join("blobs").join(
        scoped
            .entry_name(&super::super::super::super::tests::digest(&registry.layer))
            .unwrap(),
    );
    std::fs::write(&layer, b"replacement bytes").unwrap();
    assert!(owner
        .apply_removal(&key, preview.token(), "interrupted cleanup")
        .await
        .is_err());
    for blob in receipt.blobs() {
        assert!(
            root.path()
                .join("blobs")
                .join(scoped.entry_name(&blob.digest).unwrap())
                .exists(),
            "resume must preflight every remaining inode before deleting any more bytes"
        );
    }
    std::fs::write(&layer, &registry.layer).unwrap();
    owner.inner.publish_receipt(receipt.clone()).await.unwrap();
    assert!(owner
        .apply_removal(&key, preview.token(), "interrupted cleanup")
        .await
        .is_err());
    assert_eq!(
        puller
            .cached_receipt(&key)
            .await
            .unwrap()
            .unwrap()
            .generation()
            .unwrap(),
        receipt.generation().unwrap(),
        "even same-generation republication must survive an old interrupted plan"
    );
}

#[tokio::test]
async fn durable_removal_resumes_after_process_death() {
    if let Ok(root) = std::env::var("CAPSEM_OCI_REMOVE_CHILD_ROOT") {
        let owner = ImageCache::at(std::path::Path::new(&root)).unwrap();
        let key = crate::oci::CacheKey::parse(&std::env::var("CAPSEM_OCI_REMOVE_CHILD_KEY").unwrap()).unwrap();
        owner
            .apply_removal(
                &key,
                &std::env::var("CAPSEM_OCI_REMOVE_CHILD_TOKEN").unwrap(),
                "interrupted cleanup",
            )
            .await
            .unwrap();
        panic!("child must exit at the durable receipt boundary");
    }
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = crate::oci::tests::private_dir();
    let owner = ImageCache::at(root.path()).unwrap();
    let puller = registry.puller().with_cache(&owner);
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    drop(image);
    registry.task.abort();
    let receipt = puller.cached_receipt(&key).await.unwrap().unwrap();
    let preview = owner.preview_removal(&key).await.unwrap();
    interrupt(root.path(), &key, preview.token());
    assert!(puller.cached_receipt(&key).await.unwrap().is_none());
    let scoped = owner.inner.for_repository(&image_reference(receipt.origin()).unwrap());
    for blob in receipt.blobs() {
        assert!(
            root.path()
                .join("blobs")
                .join(scoped.entry_name(&blob.digest).unwrap())
                .exists(),
            "receipt invalidation must precede payload cleanup"
        );
    }
    let journal: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.path().join(format!("removal-{}.json", preview.token()))).unwrap())
            .unwrap();
    assert_eq!(journal["result"]["complete"], false);
    assert_eq!(journal["reason"], "interrupted cleanup");
    let resumed = ImageCache::at(root.path())
        .unwrap()
        .apply_removal(&key, preview.token(), "interrupted cleanup")
        .await
        .unwrap();
    assert!(resumed.complete);
    assert_eq!(resumed.removed_allocated_bytes, preview.reclaimable_bytes());
    assert_eq!(std::fs::read_dir(root.path().join("blobs")).unwrap().count(), 0);
}

#[tokio::test]
async fn exact_apply_removes_only_owned_unreferenced_bytes_and_replays_idempotently() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = crate::oci::tests::private_dir();
    let owner = ImageCache::at(root.path()).unwrap();
    let puller = registry.puller().with_cache(&owner);
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    drop(image);
    registry.task.abort();
    let unknown = root.path().join("blobs/unrelated");
    std::fs::write(&unknown, b"leave intact").unwrap();
    let preview = owner.preview_removal(&key).await.unwrap();
    assert!(owner.apply_removal(&key, preview.token(), " ").await.is_err());
    assert!(owner.apply_removal(&key, "../../outside", "test").await.is_err());
    assert!(puller.cached_receipt(&key).await.unwrap().is_some());
    let result = owner
        .apply_removal(&key, preview.token(), "local cache cleanup")
        .await
        .unwrap();
    assert!(result.complete);
    assert_eq!(result.removed_allocated_bytes, preview.reclaimable_bytes());
    assert!(puller.cached_receipt(&key).await.unwrap().is_none());
    assert_eq!(std::fs::read(&unknown).unwrap(), b"leave intact");
    assert_eq!(std::fs::read_dir(root.path().join("blobs")).unwrap().count(), 1);
    let retry = ImageCache::at(root.path())
        .unwrap()
        .apply_removal(&key, preview.token(), "local cache cleanup")
        .await
        .unwrap();
    assert_eq!(retry, result);
}

#[tokio::test]
async fn stale_and_materializing_apply_refuse_before_any_removal_intent() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let root = crate::oci::tests::private_dir();
    let owner = ImageCache::at(root.path()).unwrap();
    let puller = registry.puller().with_cache(&owner);
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let key = image.cache_identity().key();
    let busy = owner.preview_removal(&key).await.unwrap();
    assert!(owner.apply_removal(&key, busy.token(), "busy").await.is_err());
    drop(image);
    registry.task.abort();
    let before = owner.preview_removal(&key).await.unwrap();
    let receipt = root.path().join("blobs").join(format!("receipt-{}", key.as_str()));
    let bytes = std::fs::read(&receipt).unwrap();
    std::fs::write(&receipt, &bytes).unwrap();
    assert!(owner.apply_removal(&key, before.token(), "stale").await.is_err());
    assert_eq!(std::fs::read(&receipt).unwrap(), bytes);
    assert!(!root.path().join(format!("removal-{}.json", before.token())).exists());
    assert!(!root.path().join(format!("removal-{}.json", busy.token())).exists());
}
