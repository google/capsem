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

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn descriptor(media: &str, bytes: &[u8]) -> Value {
    json!({"mediaType":media,"digest":digest(bytes),"size":bytes.len()})
}

#[derive(Clone)]
struct RegistryRequest {
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

struct Registry {
    address: String,
    source_digest: String,
    config: Vec<u8>,
    layer: Vec<u8>,
    task: tokio::task::JoinHandle<()>,
    requests: Arc<Mutex<Vec<RegistryRequest>>>,
    blobs: RegistryBlobs,
}

impl Drop for Registry {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Registry {
    async fn start(change: impl FnOnce(&mut Value, &mut BTreeMap<String, Vec<u8>>)) -> Self {
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

    fn puller(&self) -> Puller {
        Puller::configured("arm64", RegistryAuth::Anonymous, ClientProtocol::Http).unwrap()
    }

    fn reference(&self) -> String {
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
