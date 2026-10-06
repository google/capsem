use super::*;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    response::IntoResponse,
    Router,
};
use oci_client::{client::ClientProtocol, secrets::RegistryAuth};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const ROOTFS_MEDIA: &str = "application/vnd.capsem.rootfs.erofs.v1";
pub(super) const ROOTFS_BYTES: &[u8] = b"opaque published EROFS bytes: never unpack these on the host";

pub(super) fn rootfs_subject() -> ContentDigest {
    ContentDigest::parse(&format!("sha256:{}", "e".repeat(64))).unwrap()
}

pub(super) async fn rootfs_registry(change: impl FnOnce(&mut Value, &mut BTreeMap<String, Vec<u8>>)) -> Registry {
    Registry::start(move |manifest, blobs| {
        let empty = b"{}";
        let rootfs = ROOTFS_BYTES;
        blobs.insert(format!("/v2/team/image/blobs/{}", digest(empty)), empty.to_vec());
        blobs.insert(format!("/v2/team/image/blobs/{}", digest(rootfs)), rootfs.to_vec());
        *manifest = json!({
            "schemaVersion": 2, "mediaType": OCI_IMAGE_MEDIA_TYPE,
            "artifactType": ROOTFS_MEDIA,
            "config": descriptor("application/vnd.oci.empty.v1+json", empty),
            "subject": {"mediaType": OCI_IMAGE_MEDIA_TYPE, "digest": rootfs_subject().as_str(), "size": 100},
            "layers": [descriptor(ROOTFS_MEDIA, rootfs)],
        });
        change(manifest, blobs);
    })
    .await
}

pub(super) fn rootfs_reference(registry: &Registry) -> String {
    format!("{}/team/image@{}", registry.address, registry.source_digest)
}

#[tokio::test]
async fn published_rootfs_is_verified_opaque_and_lifetime_owned() {
    let registry = rootfs_registry(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let layout = registry
        .puller()
        .fetch_rootfs(&rootfs_reference(&registry), &rootfs_subject(), parent.path())
        .await
        .unwrap();
    let path = layout.path();
    assert_eq!(std::fs::read(&path).unwrap(), ROOTFS_BYTES);
    assert!(path.starts_with(parent.path()));
    drop(layout);
    assert!(!path.exists(), "the returned layout owns the disposable payload");
}

#[tokio::test]
async fn published_rootfs_requires_immutable_reference_and_exact_platform_subject() {
    let registry = rootfs_registry(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let puller = registry.puller();
    assert!(puller
        .fetch_rootfs(&registry.reference(), &rootfs_subject(), parent.path())
        .await
        .is_err());
    assert!(
        registry.requests.lock().unwrap().is_empty(),
        "a mutable rootfs reference must fail before registry access"
    );
    let wrong = ContentDigest::parse(&format!("sha256:{}", "a".repeat(64))).unwrap();
    let error = puller
        .fetch_rootfs(&rootfs_reference(&registry), &wrong, parent.path())
        .await
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("subject"));
    assert_eq!(
        registry.blob_requests(),
        0,
        "a different platform's rootfs must fail before blob access"
    );
}

#[tokio::test]
async fn malformed_published_rootfs_metadata_is_refused_before_blob_access() {
    for case in 0..16 {
        let registry = rootfs_registry(move |manifest, _| match case {
            0 => manifest["artifactType"] = json!("application/other"),
            1 => manifest["layers"][0]["mediaType"] = json!(IMAGE_LAYER_GZIP_MEDIA_TYPE),
            2 => manifest["layers"][0]["size"] = json!(-1),
            3 => manifest["layers"][0]["size"] = json!(IMAGE_LIMIT + 1),
            4 => manifest["layers"][0]["urls"] = json!(["https://untrusted.example/rootfs"]),
            5 => manifest["layers"] = json!([]),
            6 => manifest["subject"] = Value::Null,
            7 => manifest["schemaVersion"] = json!(1),
            8 => manifest["config"]["size"] = json!(METADATA_LIMIT + 1),
            9 => manifest["config"]["mediaType"] = json!(IMAGE_CONFIG_MEDIA_TYPE),
            10 => manifest["config"]["urls"] = json!(["https://untrusted.example/config"]),
            11 => manifest["subject"]["urls"] = json!(["https://untrusted.example/image"]),
            12 => manifest["layers"][0]["digest"] = json!("sha256:../../foreign"),
            13 => manifest["subject"]["size"] = json!(0),
            14 => manifest["config"]["digest"] = json!(rootfs_subject().as_str()),
            15 => {
                let layer = manifest["layers"][0].clone();
                manifest["layers"].as_array_mut().unwrap().push(layer);
            }
            _ => unreachable!(),
        })
        .await;
        let parent = tempfile::tempdir().unwrap();
        assert!(
            registry
                .puller()
                .fetch_rootfs(&rootfs_reference(&registry), &rootfs_subject(), parent.path())
                .await
                .is_err(),
            "accepted metadata case {case}"
        );
        assert_eq!(registry.blob_requests(), 0, "metadata case {case} reached blob access");
    }
}

#[tokio::test]
async fn corrupt_published_rootfs_bytes_are_refused_and_not_cached() {
    let registry = rootfs_registry(|manifest, blobs| {
        let name = manifest["layers"][0]["digest"].as_str().unwrap();
        let path = format!("/v2/team/image/blobs/{name}");
        let bytes = blobs.get_mut(&path).unwrap();
        bytes[0] ^= 1;
    })
    .await;
    let parent = tempfile::tempdir().unwrap();
    let cache_root = super::super::tests::private_dir();
    let mut puller = registry.puller();
    let cache = super::super::cache::BlobCache::at(cache_root.path()).unwrap();
    puller.cache = Some(cache.clone());
    let error = puller
        .fetch_rootfs(&rootfs_reference(&registry), &rootfs_subject(), parent.path())
        .await
        .err()
        .unwrap();
    let mut corrupt = ROOTFS_BYTES.to_vec();
    corrupt[0] ^= 1;
    let message = format!("{error:#}");
    assert!(
        message.contains("Invalid digest")
            && message.contains(&digest(ROOTFS_BYTES))
            && message.contains(&digest(&corrupt)),
        "{message}"
    );
    assert_eq!(
        std::fs::read_dir(parent.path()).unwrap().count(),
        0,
        "failed staging must be reclaimed"
    );
    let namespace = cache.for_repository(&image_reference(&rootfs_reference(&registry)).unwrap());
    let _lease = namespace.lease(&digest(ROOTFS_BYTES)).await.unwrap();
    assert!(!namespace
        .copy_hit(
            &digest(ROOTFS_BYTES),
            ROOTFS_BYTES.len() as u64,
            &parent.path().join("rejected")
        )
        .await
        .unwrap());
}

#[tokio::test]
async fn truncated_published_rootfs_is_refused_and_staging_reclaimed() {
    let registry = rootfs_registry(|manifest, blobs| {
        let name = manifest["layers"][0]["digest"].as_str().unwrap();
        blobs.get_mut(&format!("/v2/team/image/blobs/{name}")).unwrap().pop();
    })
    .await;
    let parent = tempfile::tempdir().unwrap();
    let error = registry
        .puller()
        .fetch_rootfs(&rootfs_reference(&registry), &rootfs_subject(), parent.path())
        .await
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("size mismatch"), "{error:#}");
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn published_rootfs_cache_hit_survives_eviction_from_registry_but_stays_repository_scoped() {
    use std::os::unix::fs::MetadataExt;
    let registry = rootfs_registry(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let cache_root = super::super::tests::private_dir();
    let cache = super::super::cache::BlobCache::at(cache_root.path()).unwrap();
    let mut puller = registry.puller();
    puller.cache = Some(cache.clone());
    let first = puller
        .fetch_rootfs(&rootfs_reference(&registry), &rootfs_subject(), parent.path())
        .await
        .unwrap();
    assert_eq!(first.digest().as_str(), digest(ROOTFS_BYTES));
    registry
        .blobs
        .lock()
        .unwrap()
        .remove(&format!("/v2/team/image/blobs/{}", digest(ROOTFS_BYTES)));
    let count = registry.blob_requests();
    let second = puller
        .fetch_rootfs(&rootfs_reference(&registry), &rootfs_subject(), parent.path())
        .await
        .unwrap();
    assert_eq!(std::fs::read(second.path()).unwrap(), ROOTFS_BYTES);
    let original = std::fs::metadata(first.path()).unwrap();
    let reused = std::fs::metadata(second.path()).unwrap();
    assert_eq!(
        (original.dev(), original.ino()),
        (reused.dev(), reused.ino()),
        "warm published filesystems must share one physical payload"
    );
    assert_eq!(original.mode() & 0o777, 0o444);
    assert_eq!(reused.mode() & 0o777, 0o444);
    assert_eq!(
        registry.blob_requests(),
        count,
        "a verified warm hit does not fetch the payload again"
    );

    let foreign = rootfs_registry(|_, blobs| blobs.clear()).await;
    let mut untrusted = foreign.puller();
    untrusted.cache = Some(cache);
    assert!(
        untrusted
            .fetch_rootfs(&rootfs_reference(&foreign), &rootfs_subject(), parent.path())
            .await
            .is_err(),
        "a different registry cannot retrieve private cached bytes by digest"
    );
    let reference = format!("{}/attacker/image@{}", registry.address, registry.source_digest);
    {
        let mut blobs = registry.blobs.lock().unwrap();
        let manifest = blobs[&format!("/v2/team/image/manifests/{}", registry.source_digest)].clone();
        blobs.insert(
            format!("/v2/attacker/image/manifests/{}", registry.source_digest),
            manifest,
        );
        drop(blobs);
    }
    assert!(
        puller
            .fetch_rootfs(&reference, &rootfs_subject(), parent.path())
            .await
            .is_err(),
        "cache hits remain scoped within a registry's repositories"
    );
}

#[test]
fn invalid_additional_registry_certificate_is_refused() {
    assert!(Puller::new_with_root_certificate("arm64", RegistryAuth::Anonymous, Some(b"not a certificate")).is_err());
}

#[tokio::test]
async fn concurrent_published_root_acquisitions_fetch_and_retain_one_physical_payload() {
    use std::os::unix::fs::MetadataExt;
    let registry = rootfs_registry(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let cache_root = super::super::tests::private_dir();
    let mut puller = registry.puller();
    puller.cache = Some(super::super::cache::BlobCache::at(cache_root.path()).unwrap());
    let reference = rootfs_reference(&registry);
    let subject = rootfs_subject();
    let (a, b, c) = tokio::join!(
        puller.fetch_rootfs(&reference, &subject, parent.path()),
        puller.fetch_rootfs(&reference, &subject, parent.path()),
        puller.fetch_rootfs(&reference, &subject, parent.path()),
    );
    let layouts = [a.unwrap(), b.unwrap(), c.unwrap()];
    let inode = std::fs::metadata(layouts[0].path()).unwrap().ino();
    for layout in &layouts {
        let metadata = std::fs::metadata(layout.path()).unwrap();
        assert_eq!(metadata.ino(), inode);
        assert_eq!(metadata.mode() & 0o777, 0o444);
        assert_eq!(std::fs::read(layout.path()).unwrap(), ROOTFS_BYTES);
    }
    assert_eq!(
        registry.blob_requests(),
        1,
        "concurrent readers coalesce the verified download"
    );
}

pub(super) fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn descriptor(media: &str, bytes: &[u8]) -> Value {
    json!({"mediaType":media,"digest":digest(bytes),"size":bytes.len()})
}

#[derive(Clone)]
pub(super) struct RegistryRequest {
    path: String,
    authorization: Option<String>,
}

type RegistryBlobs = Arc<Mutex<BTreeMap<String, Vec<u8>>>>;

#[tokio::test]
async fn another_registry_cannot_retrieve_cached_private_blobs_by_digest() {
    let private = Registry::start_with_auth(|_, _| {}, true).await;
    let public = Registry::start(|_, blobs| blobs.clear()).await;
    let staging = tempfile::tempdir().unwrap();
    let root = super::super::tests::private_dir();
    let cache = super::super::cache::BlobCache::at(root.path()).unwrap();
    let mut authorized = Puller::configured(
        "arm64",
        RegistryAuth::Basic("user".into(), "secret".into()),
        ClientProtocol::Http,
        None,
    )
    .unwrap();
    authorized.cache = Some(cache.clone());
    let _image = authorized.pull(&private.reference(), staging.path()).await.unwrap();
    let mut untrusted = public.puller();
    untrusted.cache = Some(cache);
    assert!(
        untrusted.pull(&public.reference(), staging.path()).await.is_err(),
        "a manifest from another registry cannot authorize access to cached private blobs"
    );
    {
        let mut blobs = private.blobs.lock().unwrap();
        let manifest = blobs[&format!("/v2/team/image/manifests/{}", private.source_digest)].clone();
        blobs.insert("/v2/attacker/image/manifests/latest".into(), manifest);
    }
    let other_repository = format!("{}/attacker/image:latest", private.address);
    assert!(
        authorized.pull(&other_repository, staging.path()).await.is_err(),
        "cache access must also remain scoped within a registry"
    );
}

pub(super) struct Registry {
    address: String,
    source_digest: String,
    pub(super) config: Vec<u8>,
    pub(super) layer: Vec<u8>,
    pub(super) task: tokio::task::JoinHandle<()>,
    pub(super) requests: Arc<Mutex<Vec<RegistryRequest>>>,
    blobs: RegistryBlobs,
}

impl Drop for Registry {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Registry {
    pub(super) async fn start(change: impl FnOnce(&mut Value, &mut BTreeMap<String, Vec<u8>>)) -> Self {
        Self::start_with_auth(change, false).await
    }

    async fn start_with_auth(
        change: impl FnOnce(&mut Value, &mut BTreeMap<String, Vec<u8>>),
        require_auth: bool,
    ) -> Self {
        Self::start_options(change, require_auth, None).await
    }

    async fn start_options(
        change: impl FnOnce(&mut Value, &mut BTreeMap<String, Vec<u8>>),
        require_auth: bool,
        pause_layer: Option<Arc<tokio::sync::Notify>>,
    ) -> Self {
        let config = serde_json::to_vec(&json!({"architecture":"arm64","os":"linux", "config":{"Entrypoint":["/bin/app"],"Cmd":["serve"]},"rootfs":{"type":"layers","diff_ids":[]}})).unwrap();
        let layer = b"opaque compressed layer bytes, never extracted on the host".to_vec();
        let mut blobs = BTreeMap::from([
            (format!("/v2/team/image/blobs/{}", digest(&config)), config.clone()),
            (format!("/v2/team/image/blobs/{}", digest(&layer)), layer.clone()),
        ]);
        let mut manifest = json!({"schemaVersion":2,"mediaType":"application/vnd.docker.distribution.manifest.v2+json",
            "config":descriptor("application/vnd.docker.container.image.v1+json", &config),
            "layers":[descriptor("application/vnd.docker.image.rootfs.diff.tar.gzip", &layer)]});
        change(&mut manifest, &mut blobs);
        let manifest = serde_json::to_vec(&manifest).unwrap();
        let source_digest = digest(&manifest);
        blobs.insert(format!("/v2/team/image/manifests/{source_digest}"), manifest.clone());
        let index = serde_json::to_vec(&json!({"schemaVersion":2,"mediaType":"application/vnd.oci.image.index.v1+json","manifests":[
            {"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":format!("sha256:{}", "f".repeat(64)),"size":1,"platform":{"os":"linux","architecture":"amd64"}},
            {"mediaType":"application/vnd.docker.distribution.manifest.v2+json","digest":source_digest,"size":manifest.len(),"platform":{"os":"linux","architecture":"arm64","variant":"v8"}}
        ]})).unwrap();
        blobs.insert("/v2/team/image/manifests/latest".into(), index);
        blobs.insert("/v2/".into(), b"{}".to_vec());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = requests.clone();
        let blobs = Arc::new(Mutex::new(blobs));
        let layer_path = format!("/v2/team/image/blobs/{}", digest(&layer));
        let app = Router::new()
            .fallback(move |State(blobs): State<RegistryBlobs>, request: Request<Body>| {
                let observed = observed.clone();
                let pause_layer = pause_layer.clone();
                let layer_path = layer_path.clone();
                async move {
                    observed.lock().unwrap().push(RegistryRequest {
                        path: request.uri().path().to_owned(),
                        authorization: request
                            .headers()
                            .get("authorization")
                            .map(|v| v.to_str().unwrap().to_owned()),
                    });
                    if require_auth
                        && request.headers().get("authorization").and_then(|v| v.to_str().ok())
                            != Some("Basic dXNlcjpzZWNyZXQ=")
                    {
                        return (
                            StatusCode::UNAUTHORIZED,
                            [("www-authenticate", "Basic realm=\"registry\"")],
                        )
                            .into_response();
                    }
                    if let Some(ready) = pause_layer.filter(|_| request.uri().path() == layer_path) {
                        ready.notify_one();
                        let bytes = stream::once(async { Ok::<_, std::io::Error>(vec![0u8]) }).chain(stream::pending());
                        return Body::from_stream(bytes).into_response();
                    }
                    let bytes = blobs.lock().unwrap().get(request.uri().path()).cloned();
                    match bytes {
                        Some(bytes) => (StatusCode::OK, [("content-type", "application/json")], bytes).into_response(),
                        None => StatusCode::NOT_FOUND.into_response(),
                    }
                }
            })
            .with_state(blobs.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            address,
            source_digest,
            config,
            layer,
            task,
            requests,
            blobs,
        }
    }

    pub(super) fn puller(&self) -> Puller {
        Puller::configured("arm64", RegistryAuth::Anonymous, ClientProtocol::Http, None).unwrap()
    }

    pub(super) fn reference(&self) -> String {
        format!("{}/team/image:latest", self.address)
    }
}

#[tokio::test]
async fn mutable_tags_are_refreshed_while_unchanged_layers_stay_cached() {
    let registry = Registry::start(|_, _| {}).await;
    let staging = tempfile::tempdir().unwrap();
    let root = super::super::tests::private_dir();
    let mut puller = registry.puller();
    puller.cache = Some(super::super::cache::BlobCache::at(root.path()).unwrap());
    let first = puller.pull(&registry.reference(), staging.path()).await.unwrap();
    let mut config: Value = serde_json::from_slice(&registry.config).unwrap();
    config["config"]["Env"] = json!(["VERSION=2"]);
    let config = serde_json::to_vec(&config).unwrap();
    let new_digest = {
        let mut blobs = registry.blobs.lock().unwrap();
        let mut manifest: Value = serde_json::from_slice(
            blobs
                .get(&format!("/v2/team/image/manifests/{}", registry.source_digest))
                .unwrap(),
        )
        .unwrap();
        manifest["config"] = descriptor(IMAGE_CONFIG_MEDIA_TYPE, &config);
        let manifest = serde_json::to_vec(&manifest).unwrap();
        let identity = digest(&manifest);
        blobs.insert("/v2/team/image/manifests/latest".into(), manifest);
        blobs.insert(format!("/v2/team/image/blobs/{}", digest(&config)), config.clone());
        drop(blobs);
        identity
    };
    let second = puller.pull(&registry.reference(), staging.path()).await.unwrap();
    assert_eq!(second.source_digest, new_digest);
    assert_ne!(first.source_digest, second.source_digest);
    assert_eq!(
        std::fs::read(
            second
                .path()
                .join("blobs/sha256")
                .join(digest_hex(&digest(&config)).unwrap())
        )
        .unwrap(),
        config
    );
    let requests = registry.requests.lock().unwrap().clone();
    assert_eq!(requests.iter().filter(|r| r.path.contains("/blobs/")).count(), 3);
}

#[tokio::test]
async fn cancelled_download_never_publishes_partial_blob_and_releases_lease() {
    let ready = Arc::new(tokio::sync::Notify::new());
    let registry = Registry::start_options(|_, _| {}, false, Some(ready.clone())).await;
    let staging = tempfile::tempdir().unwrap();
    let root = super::super::tests::private_dir();
    let cache = super::super::cache::BlobCache::at(root.path()).unwrap();
    let mut puller = registry.puller();
    puller.cache = Some(cache.clone());
    let reference = registry.reference();
    let parent = staging.path().to_owned();
    let task = tokio::spawn(async move { puller.pull(&reference, &parent).await });
    tokio::time::timeout(Duration::from_secs(3), ready.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert_eq!(std::fs::read_dir(staging.path()).unwrap().count(), 0);
    let name = cache
        .for_repository(&image_reference(&registry.reference()).unwrap())
        .entry_name(&digest(&registry.layer))
        .unwrap();
    assert!(!root.path().join("blobs").join(name).exists());
    assert_eq!(std::fs::read_dir(root.path().join("blobs")).unwrap().count(), 1);
    let _lease = tokio::time::timeout(Duration::from_secs(1), cache.lease(&digest(&registry.layer)))
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn concurrent_pulls_and_new_clients_reuse_verified_blobs() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let cache_root = super::super::tests::private_dir();
    let mut puller = registry.puller();
    puller.cache = Some(super::super::cache::BlobCache::at(cache_root.path()).unwrap());
    let reference = registry.reference();
    let (a, b) = tokio::join!(
        puller.pull(&reference, parent.path()),
        puller.pull(&reference, parent.path())
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(a.path()).unwrap().permissions().mode() & 0o777, 0o700);
    let mut next = registry.puller();
    next.cache = Some(super::super::cache::BlobCache::at(cache_root.path()).unwrap());
    let c = next.pull(&reference, parent.path()).await.unwrap();
    for image in [&a, &b, &c] {
        assert_eq!(image.source_digest, registry.source_digest);
        for bytes in [&registry.config, &registry.layer] {
            assert_eq!(
                tokio::fs::read(
                    image
                        .path()
                        .join("blobs/sha256")
                        .join(digest_hex(&digest(bytes)).unwrap())
                )
                .await
                .unwrap(),
                *bytes
            );
        }
    }
    let requests = registry.requests.lock().unwrap().clone();
    assert_eq!(requests.iter().filter(|r| r.path.contains("/blobs/")).count(), 2);
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.path.ends_with("/manifests/latest"))
            .count(),
        3
    );
}

#[tokio::test]
async fn corrupt_cache_blob_is_refetched_without_reusing_corrupt_bytes() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let cache_root = super::super::tests::private_dir();
    let mut puller = registry.puller();
    puller.cache = Some(super::super::cache::BlobCache::at(cache_root.path()).unwrap());
    let first = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let hex = digest(&registry.layer);
    let hex = digest_hex(&hex).unwrap();
    let cache_name = puller
        .cache
        .as_ref()
        .unwrap()
        .for_repository(&image_reference(&registry.reference()).unwrap())
        .entry_name(&digest(&registry.layer))
        .unwrap();
    std::fs::write(
        cache_root.path().join("blobs").join(cache_name),
        vec![b'x'; registry.layer.len()],
    )
    .unwrap();
    let second = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    for image in [&first, &second] {
        assert_eq!(
            std::fs::read(image.path().join("blobs/sha256").join(hex)).unwrap(),
            registry.layer
        );
    }
    let requests = registry.requests.lock().unwrap().clone();
    assert_eq!(requests.iter().filter(|r| r.path.contains("/blobs/")).count(), 3);
}

#[tokio::test]
async fn scoped_basic_credentials_apply_to_manifest_and_blobs() {
    let registry = Registry::start_with_auth(|_, _| {}, true).await;
    let parent = tempfile::tempdir().unwrap();
    let puller = Puller::configured(
        "arm64",
        RegistryAuth::Basic("user".into(), "secret".into()),
        ClientProtocol::Http,
        None,
    )
    .unwrap();
    let image = puller.pull(&registry.reference(), parent.path()).await.unwrap();
    let requests = registry.requests.lock().unwrap().clone();
    let pulls: Vec<_> = requests.iter().filter(|request| request.path != "/v2/").collect();
    assert_eq!(pulls.len(), 4);
    for request in pulls {
        assert_eq!(request.authorization.as_deref(), Some("Basic dXNlcjpzZWNyZXQ="));
    }
    for file in image.files() {
        let bytes = std::fs::read(image.path().join(file)).unwrap();
        assert!(
            !bytes.windows(6).any(|part| part == b"secret"),
            "credential leaked into staged image"
        );
    }
}

#[tokio::test]
async fn pulls_native_index_and_normalizes_docker_media_for_umoci() {
    let registry = Registry::start(|_, _| {}).await;
    let parent = tempfile::tempdir().unwrap();
    let image = registry
        .puller()
        .pull(&registry.reference(), parent.path())
        .await
        .unwrap();
    assert_eq!(image.source_digest, registry.source_digest);
    let index: Value =
        serde_json::from_slice(&tokio::fs::read(image.path().join("index.json")).await.unwrap()).unwrap();
    assert_eq!(
        index["manifests"][0]["annotations"]["org.opencontainers.image.ref.name"],
        "image"
    );
    let manifest_digest = index["manifests"][0]["digest"].as_str().unwrap();
    let manifest_bytes = tokio::fs::read(
        image
            .path()
            .join("blobs/sha256")
            .join(manifest_digest.strip_prefix("sha256:").unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(digest(&manifest_bytes), manifest_digest);
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest["mediaType"], "application/vnd.oci.image.manifest.v1+json");
    assert_eq!(
        manifest["layers"][0]["mediaType"],
        "application/vnd.oci.image.layer.v1.tar+gzip"
    );
    for bytes in [&registry.config, &registry.layer] {
        let path = image
            .path()
            .join("blobs/sha256")
            .join(digest(bytes).strip_prefix("sha256:").unwrap());
        assert_eq!(tokio::fs::read(path).await.unwrap(), *bytes);
    }
    let path = image.path().to_owned();
    drop(image);
    assert!(!path.exists(), "staging must be disposable");
}

#[tokio::test]
async fn corrupt_layer_is_rejected_and_staging_removed() {
    let registry = Registry::start(|manifest, blobs| {
        let path = format!(
            "/v2/team/image/blobs/{}",
            manifest["layers"][0]["digest"].as_str().unwrap()
        );
        blobs.insert(path, b"corrupted".to_vec());
    })
    .await;
    let parent = tempfile::tempdir().unwrap();
    assert!(registry
        .puller()
        .pull(&registry.reference(), parent.path())
        .await
        .is_err());
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn oversized_or_external_layers_are_rejected_before_fetch() {
    for external in [false, true] {
        let registry = Registry::start(|manifest, _| {
            if external {
                manifest["layers"][0]["urls"] = json!(["https://example.invalid/private"]);
            } else {
                manifest["layers"][0]["size"] = json!(i64::MAX);
            }
        })
        .await;
        let parent = tempfile::tempdir().unwrap();
        let error = registry
            .puller()
            .pull(&registry.reference(), parent.path())
            .await
            .err()
            .expect("unsafe descriptor accepted");
        assert!(
            format!("{error:#}").contains(if external { "external" } else { "size" }),
            "{error:#}"
        );
        assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn pinned_manifest_checks_config_platform_without_trusting_the_index() {
    let registry = Registry::start(|manifest, blobs| {
        let config = br#"{"architecture":"amd64","os":"linux"}"#;
        manifest["config"] = descriptor("application/vnd.oci.image.config.v1+json", config);
        blobs.insert(format!("/v2/team/image/blobs/{}", digest(config)), config.to_vec());
    })
    .await;
    let parent = tempfile::tempdir().unwrap();
    let reference = format!("{}/team/image@{}", registry.address, registry.source_digest);
    let error = registry.puller().pull(&reference, parent.path()).await.err().unwrap();
    assert!(format!("{error:#}").contains("linux/arm64"), "{error:#}");
}

const CATALOG_DOCUMENT: &str = r#"{"schema_version":1,"channel":"stable","entries":{"codex":{"description":"Codex","versions":[
    {"image":"ghcr.io/google/capsem/codex@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","platforms":["linux/arm64"],"contract":1}]}}}"#;

/// A registry whose `team/image:stable` is a catalog artifact: an empty
/// config and one layer, described as `media` and declared as `size`.
async fn catalog_registry(media: &'static str, layer: Vec<u8>, size: Option<u64>) -> Registry {
    let registry = Registry::start(move |manifest, blobs| {
        let empty = b"{}";
        blobs.insert(format!("/v2/team/image/blobs/{}", digest(empty)), empty.to_vec());
        blobs.insert(format!("/v2/team/image/blobs/{}", digest(&layer)), layer.clone());
        let mut layer_descriptor = descriptor(media, &layer);
        if let Some(size) = size {
            layer_descriptor["size"] = json!(size);
        }
        *manifest = json!({"schemaVersion":2,"mediaType":OCI_IMAGE_MEDIA_TYPE,"artifactType":CATALOG_MEDIA_TYPE,
            "config":descriptor("application/vnd.oci.empty.v1+json", empty),"layers":[layer_descriptor]});
    })
    .await;
    {
        let mut blobs = registry.blobs.lock().unwrap();
        let manifest = blobs[&format!("/v2/team/image/manifests/{}", registry.source_digest)].clone();
        blobs.insert("/v2/team/image/manifests/stable".into(), manifest);
    }
    registry
}

impl Registry {
    fn catalog(&self) -> String {
        format!("{}/team/image:stable", self.address)
    }

    fn blob_requests(&self) -> usize {
        let requests = self.requests.lock().unwrap();
        requests.iter().filter(|r| r.path.contains("/blobs/")).count()
    }
}

#[tokio::test]
async fn catalog_is_fetched_verified_and_identified_by_manifest_digest() {
    let registry = catalog_registry(CATALOG_MEDIA_TYPE, CATALOG_DOCUMENT.as_bytes().to_vec(), None).await;
    let parent = tempfile::tempdir().unwrap();
    let cache_root = super::super::tests::private_dir();
    let mut puller = registry.puller();
    puller.cache = Some(super::super::cache::BlobCache::at(cache_root.path()).unwrap());
    let (identity, catalog) = puller.fetch_catalog(&registry.catalog(), parent.path()).await.unwrap();
    assert_eq!(identity.as_str(), registry.source_digest);
    assert!(catalog.entry("codex").is_some());
    assert_eq!(
        std::fs::read_dir(parent.path()).unwrap().count(),
        0,
        "staging is disposable"
    );
    // The tag is re-read every time; the unchanged layer comes from the cache.
    let pinned = format!("{}/team/image@{}", registry.address, registry.source_digest);
    let (again, _) = puller.fetch_catalog(&pinned, parent.path()).await.unwrap();
    assert_eq!(again, identity);
    assert_eq!(registry.blob_requests(), 1);
}

#[tokio::test]
async fn catalog_layer_must_carry_the_catalog_media_type() {
    let registry = catalog_registry(
        "application/vnd.oci.image.layer.v1.tar+gzip",
        CATALOG_DOCUMENT.as_bytes().to_vec(),
        None,
    )
    .await;
    let parent = tempfile::tempdir().unwrap();
    let error = registry
        .puller()
        .fetch_catalog(&registry.catalog(), parent.path())
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("media type"), "{error:#}");
    assert_eq!(registry.blob_requests(), 0, "refused before fetching the layer");
}

#[tokio::test]
async fn oversized_catalog_layer_is_refused_before_fetch() {
    let large = vec![b' '; CATALOG_LIMIT as usize + 1];
    for (layer, size) in [
        (large, None),
        (CATALOG_DOCUMENT.as_bytes().to_vec(), Some(u64::MAX >> 1)),
    ] {
        let registry = catalog_registry(CATALOG_MEDIA_TYPE, layer, size).await;
        let parent = tempfile::tempdir().unwrap();
        let error = registry
            .puller()
            .fetch_catalog(&registry.catalog(), parent.path())
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("1 MiB"), "{error:#}");
        assert_eq!(registry.blob_requests(), 0);
    }
}

#[tokio::test]
async fn catalog_layer_bytes_must_match_their_digest_and_parse() {
    let registry = catalog_registry(CATALOG_MEDIA_TYPE, CATALOG_DOCUMENT.as_bytes().to_vec(), None).await;
    registry.blobs.lock().unwrap().insert(
        format!("/v2/team/image/blobs/{}", digest(CATALOG_DOCUMENT.as_bytes())),
        CATALOG_DOCUMENT.replace("Codex", "Evil!").into_bytes(),
    );
    let parent = tempfile::tempdir().unwrap();
    let error = registry
        .puller()
        .fetch_catalog(&registry.catalog(), parent.path())
        .await
        .unwrap_err();
    // oci-client verifies the stream against the descriptor before our own check.
    assert!(format!("{error:#}").to_lowercase().contains("digest"), "{error:#}");

    let invalid = CATALOG_DOCUMENT.replace("\"contract\":1", "\"contract\":1,\"extra\":true");
    let registry = catalog_registry(CATALOG_MEDIA_TYPE, invalid.into_bytes(), None).await;
    let error = registry
        .puller()
        .fetch_catalog(&registry.catalog(), parent.path())
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("unknown field"), "{error:#}");
}

#[tokio::test]
async fn catalog_must_be_one_artifact_manifest_not_an_image_index() {
    let registry = catalog_registry(CATALOG_MEDIA_TYPE, CATALOG_DOCUMENT.as_bytes().to_vec(), None).await;
    let parent = tempfile::tempdir().unwrap();
    let index = format!("{}/team/image:latest", registry.address);
    assert!(registry.puller().fetch_catalog(&index, parent.path()).await.is_err());

    let registry = Registry::start(|manifest, _| {
        let layer = manifest["layers"][0].clone();
        manifest["layers"] = json!([layer.clone(), layer]);
    })
    .await;
    let pinned = format!("{}/team/image@{}", registry.address, registry.source_digest);
    let error = registry
        .puller()
        .fetch_catalog(&pinned, parent.path())
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("exactly one layer"), "{error:#}");
}
