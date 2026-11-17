use super::*;
use crate::catalogs::{catalog_media_type, ValidatedCatalogSource};
use capsem_proto::catalogs::{CatalogKind, CatalogSourceDescriptor, CatalogSourceOrigin, SelectedCatalogSource};

const DOCUMENT: &[u8] = br#"{"schema_version":1,"entries":{}}"#;

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
