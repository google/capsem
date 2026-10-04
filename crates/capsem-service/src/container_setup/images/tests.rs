use super::*;
use crate::tests::route_request;
use capsem_assets::asset_manager::PackageArchitecture;
use capsem_core::net::policy_config::{CatalogSetting, ImagePolicyConfig, SettingsFile};
use serde_json::{json, Value};

fn hex(c: char) -> String {
    std::iter::repeat_n(c, 64).collect()
}

fn digest(c: char) -> String {
    format!("sha256:{}", hex(c))
}

fn host() -> &'static str {
    stage::catalog_architecture().unwrap().as_str()
}

fn other() -> &'static str {
    match stage::catalog_architecture().unwrap() {
        PackageArchitecture::Arm64 => "amd64",
        PackageArchitecture::Amd64 => "arm64",
    }
}

/// `codex-cli`: A on both architectures, B on this host only, C needing a
/// newer runtime contract; so this host resolves B. `redis` shadows the
/// Docker Hub short name. `elsewhere` is built only for the other
/// architecture.
fn catalog_document() -> Value {
    let version = |c: char, platforms: Vec<String>, contract: u32| json!({"image": format!("ghcr.io/google/capsem/codex-cli@{}", digest(c)), "platforms": platforms, "contract": contract});
    json!({
        "schema_version": 1,
        "channel": "stable",
        "generated_at": "2026-10-01T00:00:00Z",
        "entries": {
            "codex-cli": {"description": "OpenAI Codex CLI", "versions": [
                version('a', vec![format!("linux/{}", host()), format!("linux/{}", other())], 1),
                version('b', vec![format!("linux/{}", host())], 1),
                version('c', vec![format!("linux/{}", host())], RUNTIME_CONTRACT + 1),
            ]},
            "redis": {"description": "Not Docker Hub's", "versions": [
                {"image": format!("ghcr.io/google/capsem/redis@{}", digest('d')), "platforms": [format!("linux/{}", host())], "contract": 1}
            ]},
            "elsewhere": {"description": "Another machine's", "versions": [
                {"image": format!("ghcr.io/google/capsem/elsewhere@{}", digest('e')), "platforms": [format!("linux/{}", other())], "contract": 1}
            ]}
        }
    })
}

/// An image source whose policy, catalog and registry the test sets, and
/// which records every catalog read and every pull.
struct CatalogImages {
    settings: SettingsFile,
    catalog: Arc<Mutex<Option<Value>>>,
    catalog_reads: Arc<Mutex<Vec<String>>>,
    pulls: Arc<Mutex<Vec<String>>>,
}

impl ImageSource for CatalogImages {
    fn policy(&self) -> PolicyFuture {
        let settings = self.settings.clone();
        Box::pin(async move { ImagePolicy::from_files(&settings, &SettingsFile::default()) })
    }

    fn fetch_catalog(&self, source: CatalogSource, _parent: PathBuf) -> CatalogFuture {
        self.catalog_reads.lock().unwrap().push(source.reference);
        let document = self.catalog.lock().unwrap().clone();
        Box::pin(async move {
            let document = document.context("catalog registry unreachable")?;
            Ok((
                Digest::parse(&digest('9'))?,
                Catalog::parse(&serde_json::to_vec(&document)?)?,
            ))
        })
    }

    /// Serves any reference: a pinned one at its pin, a tag at digest `f`.
    fn pull(&self, image: String, _access: RegistryAccess, _parent: PathBuf) -> PullFuture {
        self.pulls.lock().unwrap().push(image.clone());
        Box::pin(async move {
            let image_digest = image
                .rsplit_once('@')
                .map_or_else(|| digest('f'), |(_, pin)| pin.to_owned());
            Ok(PulledImage {
                root: PathBuf::new(),
                files: Vec::new(),
                digest: digest('7'),
                image_digest,
                _hold: Box::new(()),
            })
        })
    }
}

struct Fixture {
    state: Arc<ServiceState>,
    catalog: Arc<Mutex<Option<Value>>>,
    catalog_reads: Arc<Mutex<Vec<String>>>,
    pulls: Arc<Mutex<Vec<String>>>,
}

impl Fixture {
    fn new(images: ImagePolicyConfig) -> Self {
        let catalog = Arc::new(Mutex::new(Some(catalog_document())));
        let catalog_reads = Arc::new(Mutex::new(Vec::new()));
        let pulls = Arc::new(Mutex::new(Vec::new()));
        let mut state = crate::tests::make_test_state_owned();
        state.containers = ContainerSetups::with_source(Box::new(CatalogImages {
            settings: SettingsFile {
                images: Some(images),
                ..Default::default()
            },
            catalog: Arc::clone(&catalog),
            catalog_reads: Arc::clone(&catalog_reads),
            pulls: Arc::clone(&pulls),
        }));
        Self {
            state: Arc::new(state),
            catalog,
            catalog_reads,
            pulls,
        }
    }

    /// No `[images]` grants: exactly the catalog.
    fn default_policy() -> Self {
        Self::new(ImagePolicyConfig::default())
    }

    async fn call(&self, method: axum::http::Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        route_request(build_service_router(Arc::clone(&self.state)), method, uri, body).await
    }

    async fn list(&self) -> (StatusCode, Value) {
        self.call(axum::http::Method::GET, "/images", None).await
    }

    async fn pull(&self, image: &str) -> (StatusCode, Value) {
        self.call(axum::http::Method::POST, "/images/pull", Some(json!({"image": image})))
            .await
    }

    fn reads(&self) -> usize {
        self.catalog_reads.lock().unwrap().len()
    }

    fn pulls(&self) -> Vec<String> {
        self.pulls.lock().unwrap().clone()
    }
}

fn grants(sources: &[&str], admit: &[&str]) -> ImagePolicyConfig {
    ImagePolicyConfig {
        sources: sources.iter().map(|s| s.to_string()).collect(),
        admit: admit.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_catalog_name_resolves_to_the_newest_version_this_host_runs() {
    let fx = Fixture::default_policy();
    let (status, body) = fx.pull("codex-cli").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let pinned = format!("ghcr.io/google/capsem/codex-cli@{}", digest('b'));
    assert_eq!(
        body,
        json!({"image": "codex-cli", "resolved": pinned, "digest": digest('7')})
    );
    // Pulled by the pin, not by a tag: what runs is what was resolved.
    assert_eq!(fx.pulls(), [pinned]);
    assert_eq!(
        fx.catalog_reads.lock().unwrap().as_slice(),
        [capsem_assets::oci::DEFAULT_CATALOG]
    );
}

/// `redis` is a Docker Hub short name and a catalog name here: the catalog
/// wins, and Docker Hub is never asked.
#[tokio::test]
async fn a_catalog_name_shadows_a_docker_hub_short_name() {
    let fx = Fixture::default_policy();
    let (status, body) = fx.pull("redis").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(fx.pulls(), [format!("ghcr.io/google/capsem/redis@{}", digest('d'))]);
}

#[tokio::test]
async fn a_name_not_in_the_catalog_is_refused_and_never_becomes_docker_hub() {
    let fx = Fixture::default_policy();
    let (status, body) = fx.pull("postgres").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let error = body["error"].as_str().unwrap();
    assert!(error.contains("not in the image catalog"), "{error}");
    assert!(fx.pulls().is_empty(), "a refused name reached a registry");
}

#[tokio::test]
async fn a_name_with_no_version_for_this_host_is_refused_not_pulled() {
    let fx = Fixture::default_policy();
    let (status, body) = fx.pull("elsewhere").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let error = body["error"].as_str().unwrap();
    assert!(error.contains(&format!("runs on linux/{}", host())), "{error}");
    assert!(fx.pulls().is_empty());
}

#[tokio::test]
async fn explicit_grants_decide_without_reading_the_catalog() {
    let fx = Fixture::new(grants(&["registry.example"], &["registry.example"]));
    let (status, body) = fx.pull("registry.example/app:1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["resolved"], format!("registry.example/app@{}", digest('f')));
    assert_eq!(fx.reads(), 0, "the grants decided; the catalog was read anyway");
}

#[tokio::test]
async fn a_refused_source_answers_403_and_its_registry_is_never_contacted() {
    let fx = Fixture::default_policy();
    let (status, body) = fx.pull("registry.example/app:1").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body["error"].as_str().unwrap().contains("not allowed"), "{body}");
    assert!(fx.pulls().is_empty(), "the refused registry was contacted");
    // A Docker Hub image needs a grant too.
    let (status, _) = fx.pull("docker://redis").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(fx.pulls().is_empty());
}

/// The catalog's repository is a source, but only its listed digests run.
#[tokio::test]
async fn a_digest_the_catalog_does_not_list_is_refused_after_the_pull() {
    let fx = Fixture::default_policy();
    let unlisted = format!("ghcr.io/google/capsem/codex-cli@{}", digest('0'));
    let (status, body) = fx.pull(&unlisted).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body["error"].as_str().unwrap().contains("not admitted"), "{body}");
    let listed = format!("ghcr.io/google/capsem/codex-cli@{}", digest('a'));
    let (status, body) = fx.pull(&listed).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn the_list_names_each_entry_its_architectures_and_the_pin_this_host_runs() {
    let fx = Fixture::default_policy();
    let (status, body) = fx.list().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut both = vec![host(), other()];
    both.sort_unstable();
    assert_eq!(
        body,
        json!({
            "catalog": {
                "reference": capsem_assets::oci::DEFAULT_CATALOG,
                "digest": digest('9'),
                "channel": "stable",
                "generated_at": "2026-10-01T00:00:00Z",
            },
            "images": [
                {"name": "codex-cli", "description": "OpenAI Codex CLI", "architectures": both,
                 "image": format!("ghcr.io/google/capsem/codex-cli@{}", digest('b')), "cached": "unknown"},
                {"name": "elsewhere", "description": "Another machine's", "architectures": [other()],
                 "cached": "unknown"},
                {"name": "redis", "description": "Not Docker Hub's", "architectures": [host()],
                 "image": format!("ghcr.io/google/capsem/redis@{}", digest('d')), "cached": "unknown"},
            ]
        })
    );
}

/// A corp that turns the catalog off denies every catalog entry: none is
/// listed, none resolves, and the catalog's registry is never asked.
#[tokio::test]
async fn a_policy_that_turns_the_catalog_off_lists_nothing_and_resolves_nothing() {
    let fx = Fixture::new(ImagePolicyConfig {
        catalog: Some(CatalogSetting::Enabled(false)),
        ..grants(&["registry.example"], &["registry.example"])
    });
    let (status, body) = fx.list().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({"images": []}));
    let (status, body) = fx.pull("codex-cli").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("turned off"), "{body}");
    // Explicit grants still work.
    let (status, _) = fx.pull("registry.example/app:1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fx.reads(), 0);
    assert_eq!(fx.pulls(), ["registry.example/app:1"]);
}

#[tokio::test]
async fn a_mirror_named_by_the_policy_is_where_the_catalog_is_read() {
    let mirror = "mirror.example/capsem/catalog:stable";
    let fx = Fixture::new(ImagePolicyConfig {
        catalog: Some(CatalogSetting::Reference(mirror.into())),
        ..Default::default()
    });
    let (status, body) = fx.list().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["catalog"]["reference"], mirror);
    assert_eq!(fx.catalog_reads.lock().unwrap().as_slice(), [mirror]);
}

#[tokio::test(start_paused = true)]
async fn the_catalog_is_read_at_most_every_thirty_minutes_or_on_demand() {
    let fx = Fixture::default_policy();
    fx.list().await;
    fx.pull("codex-cli").await;
    fx.list().await;
    assert_eq!(fx.reads(), 1, "a fresh catalog is reused");
    tokio::time::advance(CATALOG_REFRESH.checked_sub(Duration::from_secs(1)).unwrap()).await;
    fx.list().await;
    assert_eq!(fx.reads(), 1);
    tokio::time::advance(Duration::from_secs(2)).await;
    fx.list().await;
    assert_eq!(fx.reads(), 2, "a stale catalog is read again");
    let (status, _) = fx.call(axum::http::Method::GET, "/images?refresh=true", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fx.reads(), 3, "refresh reads it now");
}

#[tokio::test(start_paused = true)]
async fn a_failed_read_keeps_the_last_good_catalog_and_waits_before_retrying() {
    let fx = Fixture::default_policy();
    fx.list().await;
    *fx.catalog.lock().unwrap() = None;
    tokio::time::advance(CATALOG_REFRESH + Duration::from_secs(1)).await;
    let (status, body) = fx.list().await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fx.reads(), 2);
    assert_eq!(
        body["images"].as_array().unwrap().len(),
        3,
        "the last good catalog was dropped"
    );
    // Names still resolve while the registry is unreachable.
    let (status, _) = fx.pull("codex-cli").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fx.reads(), 2, "a failed read is not retried at once");
    tokio::time::advance(CATALOG_RETRY + Duration::from_secs(1)).await;
    fx.list().await;
    assert_eq!(fx.reads(), 3);
}

#[tokio::test]
async fn with_no_catalog_ever_read_only_explicit_grants_admit() {
    let fx = Fixture::new(grants(&["registry.example"], &["registry.example"]));
    *fx.catalog.lock().unwrap() = None;
    let (status, body) = fx.list().await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"images": []}));
    let (status, body) = fx.pull("codex-cli").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("no image catalog could be read"),
        "{body}"
    );
    let (status, _) = fx
        .pull(&format!("ghcr.io/google/capsem/codex-cli@{}", digest('b')))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "an unread catalog admits nothing");
    let (status, _) = fx.pull("registry.example/app:1").await;
    assert_eq!(status, StatusCode::OK);
}

/// An administrator who moves the catalog gets that catalog or none, never
/// the last one read from the old place.
#[tokio::test]
async fn a_catalog_read_from_another_source_is_never_served() {
    let fx = Fixture::default_policy();
    fx.list().await;
    let cache = &fx.state.containers.catalog;
    let source = &*fx.state.containers.source;
    let parent = tempfile::tempdir().unwrap();
    let moved = CatalogSource {
        reference: "mirror.example/capsem/catalog:stable".into(),
        ca: None,
    };
    *fx.catalog.lock().unwrap() = None;
    assert!(cache.get(source, &moved, parent.path(), false).await.is_none());
}
