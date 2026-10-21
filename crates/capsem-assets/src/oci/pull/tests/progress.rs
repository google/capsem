use super::*;

fn observer() -> (Arc<Mutex<Vec<BlobProgress>>>, ProgressObserver) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed = events.clone();
    (
        events,
        Arc::new(move |progress| observed.lock().unwrap().push(progress)),
    )
}

#[tokio::test]
async fn ordinary_transfer_reports_only_descriptor_bounded_verified_completion() {
    let registry = Registry::start(|_, _| {}).await;
    let staging = tempfile::tempdir().unwrap();
    let (events, observer) = observer();
    let image = registry
        .puller()
        .with_progress(observer)
        .pull(&registry.reference(), staging.path())
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(
            image
                .path()
                .join("blobs/sha256")
                .join(digest_hex(&digest(&registry.layer)).unwrap())
        )
        .unwrap(),
        registry.layer
    );
    let events = events.lock().unwrap().clone();
    for data in [&registry.config, &registry.layer] {
        let identity = digest(data);
        let blob: Vec<_> = events.iter().filter(|event| event.digest == identity).collect();
        assert!(!blob.is_empty(), "each fetched blob reports progress");
        assert_eq!(blob.first().unwrap().completed_bytes, 0);
        let last = blob.last().unwrap();
        assert!(last.verified);
        assert_eq!(last.completed_bytes, data.len() as u64);
        assert_eq!(last.total_bytes, data.len() as u64);
        assert!(blob.iter().all(|event| event.completed_bytes <= event.total_bytes));
        assert!(blob
            .windows(2)
            .all(|pair| pair[0].completed_bytes <= pair[1].completed_bytes));
    }
}

#[tokio::test]
async fn streamed_partial_bytes_are_visible_before_the_response_finishes() {
    let started = Arc::new(tokio::sync::Notify::new());
    let registry = Registry::start_options(|_, _| {}, false, Some(started)).await;
    let staging = tempfile::tempdir().unwrap();
    let partial = Arc::new(tokio::sync::Notify::new());
    let observed = partial.clone();
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let layer_digest = digest(&registry.layer);
    let expected_digest = layer_digest.clone();
    let puller = registry.puller().with_progress(Arc::new(move |event| {
        if event.digest == expected_digest && event.completed_bytes > 0 && !event.verified {
            observed.notify_one();
        }
        captured.lock().unwrap().push(event);
    }));
    let reference = registry.reference();
    let parent = staging.path().to_owned();
    let task = tokio::spawn(async move { puller.pull(&reference, &parent).await });
    let observation = tokio::time::timeout(Duration::from_secs(5), partial.notified()).await;
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    observation.expect("bytes must be observable while the transfer is still pending");
    let events = events.lock().unwrap().clone();
    let layer: Vec<_> = events.iter().filter(|event| event.digest == layer_digest).collect();
    assert!(layer
        .iter()
        .any(|event| event.completed_bytes == 1 && event.total_bytes > 1));
    assert!(layer.iter().all(|event| !event.verified));
}

#[tokio::test]
async fn cache_hits_report_complete_verified_bytes_without_network_blob_reads() {
    let registry = Registry::start(|_, _| {}).await;
    let staging = tempfile::tempdir().unwrap();
    let cache = crate::oci::tests::private_dir();
    let owner = crate::oci::ImageCache::at(cache.path()).unwrap();
    let (events, observer) = observer();
    let puller = registry.puller().with_cache(&owner).with_progress(observer);
    drop(puller.pull(&registry.reference(), staging.path()).await.unwrap());
    events.lock().unwrap().clear();
    let reads = registry.blob_requests();
    drop(puller.pull(&registry.reference(), staging.path()).await.unwrap());
    assert_eq!(registry.blob_requests(), reads);
    let events = events.lock().unwrap().clone();
    for data in [&registry.config, &registry.layer] {
        let identity = digest(data);
        let last = events.iter().rev().find(|event| event.digest == identity).unwrap();
        assert!(last.verified);
        assert_eq!(last.completed_bytes, data.len() as u64);
        assert_eq!(last.total_bytes, data.len() as u64);
    }
}

#[tokio::test]
async fn corrupt_payload_never_reports_verified_completion() {
    let registry = Registry::start(|manifest, blobs| {
        let identity = manifest["layers"][0]["digest"].as_str().unwrap();
        let data = blobs.get_mut(&format!("/v2/team/image/blobs/{identity}")).unwrap();
        data[0] ^= 1;
    })
    .await;
    let staging = tempfile::tempdir().unwrap();
    let (events, observer) = observer();
    let result = registry
        .puller()
        .with_progress(observer)
        .pull(&registry.reference(), staging.path())
        .await;
    assert!(result.is_err());
    let events = events.lock().unwrap().clone();
    let identity = digest(&registry.layer);
    let layer: Vec<_> = events.iter().filter(|event| event.digest == identity).collect();
    assert!(!layer.is_empty());
    assert!(layer.iter().all(|event| !event.verified));
    assert!(std::fs::read_dir(staging.path()).unwrap().next().is_none());
}

#[tokio::test]
async fn rootfs_completion_requires_both_integrity_hashes() {
    let registry = rootfs_registry(|manifest, _| {
        manifest["annotations"]["org.capsem.rootfs.blake3"] = json!(format!("blake3:{}", "0".repeat(64)));
    })
    .await;
    let staging = tempfile::tempdir().unwrap();
    let (events, observer) = observer();
    let result = registry
        .puller()
        .with_progress(observer)
        .fetch_rootfs(&rootfs_reference(&registry), &rootfs_subject(), staging.path())
        .await;
    assert!(result.is_err());
    let events = events.lock().unwrap().clone();
    assert!(events
        .iter()
        .any(|event| event.completed_bytes == ROOTFS_BYTES.len() as u64));
    assert!(events.iter().all(|event| !event.verified));
    assert!(std::fs::read_dir(staging.path()).unwrap().next().is_none());
}
