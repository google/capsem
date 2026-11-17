use super::*;
use crate::catalogs::{catalog_media_type, ValidatedCatalogSource};
use capsem_proto::catalogs::{CatalogKind, CatalogSourceDescriptor, CatalogSourceOrigin, SelectedCatalogSource};

const DOCUMENT: &[u8] = br#"{"schema_version":1,"entries":{}}"#;

#[tokio::test]
async fn environment_catalogs_read_exact_cached_pins_after_the_registry_stops() {
    for kind in CatalogKind::ALL {
        let mut registry = catalog_registry(catalog_media_type(kind), DOCUMENT.to_vec(), None).await;
        let parent = tempfile::tempdir().unwrap();
        let root = super::super::super::tests::private_dir();
        let mut puller = registry.puller();
        puller.cache = Some(BlobCache::at(root.path()).unwrap());
        let selected = source(&registry, kind);
        let fresh = puller
            .fetch_environment_catalog(&selected, parent.path())
            .await
            .unwrap();
        let digest = fresh.digest().clone();
        let requests = registry.requests.lock().unwrap().len();
        registry.task.abort();
        assert!((&mut registry.task).await.unwrap_err().is_cancelled());
        let cached = puller
            .fetch_environment_catalog_cached(&selected, &digest, parent.path())
            .await
            .unwrap();
        assert_eq!(cached.kind(), kind);
        assert_eq!(cached.digest(), &digest);
        assert_eq!(cached.bytes(), fresh.bytes());
        assert_eq!(registry.requests.lock().unwrap().len(), requests);
        assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn environment_catalogs_cached_corruption_or_missing_data_never_authenticates_or_fetches() {
    for case in 0..6 {
        let registry = catalog_registry(catalog_media_type(CatalogKind::Skills), DOCUMENT.to_vec(), None).await;
        let parent = tempfile::tempdir().unwrap();
        let root = super::super::super::tests::private_dir();
        let cache = BlobCache::at(root.path()).unwrap();
        let mut puller = registry.puller();
        puller.cache = Some(cache.clone());
        let selected = source(&registry, CatalogKind::Skills);
        let fresh = puller
            .fetch_environment_catalog(&selected, parent.path())
            .await
            .unwrap();
        let digest = fresh.digest().clone();
        let namespace = cache.for_repository(selected.reference());
        let file = if matches!(case, 2 | 3) {
            super::digest(DOCUMENT)
        } else {
            digest.to_string()
        };
        let path = root.path().join("blobs").join(namespace.entry_name(&file).unwrap());
        match case {
            0 | 2 => {
                let count = std::fs::metadata(&path).unwrap().len();
                std::fs::write(&path, vec![b'x'; count as usize]).unwrap();
            }
            1 | 3 => std::fs::remove_file(&path).unwrap(),
            4 => std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len((METADATA_LIMIT + 1) as u64)
                .unwrap(),
            _ => puller.cache = None,
        }
        let requests = registry.requests.lock().unwrap().len();
        assert!(puller
            .fetch_environment_catalog_cached(&selected, &digest, parent.path())
            .await
            .is_err());
        assert_eq!(
            registry.requests.lock().unwrap().len(),
            requests,
            "cache-only refusal cannot authenticate or fetch"
        );
        assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn environment_catalogs_cached_pins_keep_repository_kind_and_digest_boundaries() {
    let registry = catalog_registry(catalog_media_type(CatalogKind::Skills), DOCUMENT.to_vec(), None).await;
    let parent = tempfile::tempdir().unwrap();
    let root = super::super::super::tests::private_dir();
    let mut puller = registry.puller();
    puller.cache = Some(BlobCache::at(root.path()).unwrap());
    let selected = source(&registry, CatalogKind::Skills);
    let expected = ContentDigest::parse(&registry.source_digest).unwrap();
    assert!(puller
        .fetch_environment_catalog_cached(&selected, &expected, parent.path())
        .await
        .is_err());
    assert!(
        registry.requests.lock().unwrap().is_empty(),
        "even a first cache miss stays local"
    );
    let fresh = puller
        .fetch_environment_catalog(&selected, parent.path())
        .await
        .unwrap();
    let requests = registry.requests.lock().unwrap().len();
    let wrong_digest = ContentDigest::parse(&format!("sha256:{}", "b".repeat(64))).unwrap();
    for (kind, reference, digest) in [
        (CatalogKind::Rules, registry.catalog(), fresh.digest()),
        (
            CatalogKind::Skills,
            registry.catalog().replace("team/image", "another/image"),
            fresh.digest(),
        ),
        (CatalogKind::Skills, registry.catalog(), &wrong_digest),
        (
            CatalogKind::Skills,
            format!("{}@{}", selected.identity().repository(), fresh.digest()),
            &wrong_digest,
        ),
    ] {
        let descriptor = CatalogSourceDescriptor {
            reference,
            schema_version: 1,
        };
        let selected = ValidatedCatalogSource::new(
            kind,
            SelectedCatalogSource {
                source: &descriptor,
                origin: CatalogSourceOrigin::Corporate,
            },
        )
        .unwrap();
        assert!(puller
            .fetch_environment_catalog_cached(&selected, digest, parent.path())
            .await
            .is_err());
    }
    assert_eq!(registry.requests.lock().unwrap().len(), requests);
}

fn source(registry: &Registry, kind: CatalogKind) -> ValidatedCatalogSource {
    ValidatedCatalogSource::new(
        kind,
        SelectedCatalogSource {
            source: &CatalogSourceDescriptor {
                reference: registry.catalog(),
                schema_version: 1,
            },
            origin: CatalogSourceOrigin::Corporate,
        },
    )
    .unwrap()
}

#[tokio::test]
async fn environment_catalogs_reuse_verified_blob_cache_and_refresh_manifest_tags() {
    for kind in CatalogKind::ALL {
        let registry = catalog_registry(catalog_media_type(kind), DOCUMENT.to_vec(), None).await;
        let parent = tempfile::tempdir().unwrap();
        let cache_root = super::super::super::tests::private_dir();
        let mut puller = registry.puller();
        puller.cache = Some(super::super::super::cache::BlobCache::at(cache_root.path()).unwrap());
        let selected = source(&registry, kind);
        for _ in 0..2 {
            let artifact = puller
                .fetch_environment_catalog(&selected, parent.path())
                .await
                .unwrap();
            assert_eq!(artifact.kind(), kind);
            assert_eq!(artifact.digest().as_str(), registry.source_digest);
            assert_eq!(artifact.bytes(), DOCUMENT);
            assert_eq!(artifact.into_bytes(), DOCUMENT);
            assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
        }
        assert_eq!(registry.blob_requests(), 1);
        assert_eq!(
            registry
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.path.ends_with("/manifests/stable"))
                .count(),
            2
        );
        registry.blobs.lock().unwrap().insert(
            "/v2/team/image/manifests/stable".into(),
            b"invalid updated manifest".to_vec(),
        );
        assert!(
            puller
                .fetch_environment_catalog(&selected, parent.path())
                .await
                .is_err(),
            "a failed fresh tag read must not reuse earlier discovery"
        );
    }
}

#[tokio::test]
async fn environment_catalogs_refuse_wrong_kind_oversize_external_and_bad_digests() {
    for case in 0..4 {
        let media = if case == 0 {
            catalog_media_type(CatalogKind::Rules)
        } else {
            catalog_media_type(CatalogKind::Skills)
        };
        let registry = catalog_registry(
            media,
            DOCUMENT.to_vec(),
            if case == 1 { Some(CATALOG_LIMIT + 1) } else { None },
        )
        .await;
        if case == 2 {
            let mut blobs = registry.blobs.lock().unwrap();
            let mut manifest: Value = serde_json::from_slice(&blobs["/v2/team/image/manifests/stable"]).unwrap();
            manifest["layers"][0]["urls"] = json!(["https://untrusted.invalid/catalog"]);
            blobs.insert(
                "/v2/team/image/manifests/stable".into(),
                serde_json::to_vec(&manifest).unwrap(),
            );
        }
        if case == 3 {
            registry.blobs.lock().unwrap().insert(
                format!("/v2/team/image/blobs/{}", digest(DOCUMENT)),
                b"corrupted catalog bytes".to_vec(),
            );
        }
        let parent = tempfile::tempdir().unwrap();
        assert!(
            registry
                .puller()
                .fetch_environment_catalog(&source(&registry, CatalogKind::Skills), parent.path())
                .await
                .is_err(),
            "case {case}"
        );
        assert_eq!(registry.blob_requests(), usize::from(case == 3));
        assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
    }
}
