use super::*;
use crate::tests::route_request;
use capsem_assets::asset_manager::PackageArchitecture;
use capsem_core::net::policy_config::{CatalogSetting, ImagePolicyConfig, SettingsFile};
use serde_json::{json, Value};
mod inventory;

#[tokio::test]
async fn cache_observation_requires_the_same_filesystem_as_launch() {
    let fx = Fixture::default_policy();
    fx.catalog.lock().unwrap().as_mut().unwrap()["entries"]["codex-cli"]["versions"][1]["erofs"] =
        json!({host(): digest('8')});
    let (status, _) = fx.list().await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        *fx.required_roots.lock().unwrap(),
        vec![Some(Digest::parse(&digest('8')).unwrap()), None]
    );
}

fn hex(c: char) -> String {
    std::iter::repeat_n(c, 64).collect()
}

fn digest(c: char) -> String {
    format!("sha256:{}", hex(c))
}

fn host() -> &'static str {
    stage::catalog_architecture().unwrap().as_str()
}

#[tokio::test]
async fn catalog_reports_live_pull_bytes_and_forgets_success_failure_and_cancellation() {
    for outcome in ["success", "failure", "cancel"] {
        let fx = Fixture::default_policy();
        let release = Arc::new(tokio::sync::Notify::new());
        *fx.pull_release.lock().unwrap() = Some(release.clone());
        if outcome == "failure" {
            fx.catalog.lock().unwrap().as_mut().unwrap()["entries"]["codex-cli"]["versions"][1]["erofs"] =
                json!({host(): digest('8')});
        }
        let state = fx.state.clone();
        let task = tokio::spawn(async move {
            route_request(
                build_service_router(state),
                axum::http::Method::POST,
                "/images/pull",
                Some(json!({"image":"codex-cli"})),
            )
            .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), fx.pull_started.notified())
            .await
            .unwrap();
        let (status, body) = fx.list().await;
        assert_eq!(status, StatusCode::OK);
        let row = body["images"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == "codex-cli")
            .unwrap();
        assert_eq!(row["progress"], json!({"phase":"downloading", "completed_bytes":40}));
        assert!(body["images"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["name"] != "codex-cli")
            .all(|row| row.get("progress").is_none()));
        if outcome == "cancel" {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            release.notify_one();
            let (status, _) = task.await.unwrap();
            assert_eq!(
                status,
                if outcome == "success" {
                    StatusCode::OK
                } else {
                    StatusCode::BAD_GATEWAY
                }
            );
        }
        let (_, body) = fx.list().await;
        assert!(body["images"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row.get("progress").is_none()));
    }
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
type RootCall = (String, String, RegistryAccess, Option<capsem_assets::oci::CacheKey>);

struct CatalogImages {
    required_roots: Arc<Mutex<Vec<Option<Digest>>>>,
    pull_delay: Arc<Mutex<std::time::Duration>>,
    pull_release: Arc<Mutex<Option<Arc<tokio::sync::Notify>>>>,
    pull_started: Arc<tokio::sync::Notify>,
    disk_states: Arc<Mutex<std::collections::HashMap<capsem_assets::oci::CacheKey, capsem_assets::oci::CacheState>>>,
    observations: Arc<Mutex<Vec<Vec<capsem_assets::oci::CacheKey>>>>,
    fetches: Arc<Mutex<Vec<ImageFetch>>>,
    settings: SettingsFile,
    catalog: Arc<Mutex<Option<Value>>>,
    catalog_reads: Arc<Mutex<Vec<String>>>,
    pulls: Arc<Mutex<Vec<String>>>,
    roots: Arc<Mutex<Vec<RootCall>>>,
}

impl ImageSource for CatalogImages {
    fn fetch_inventory(
        &self,
        _catalog: CatalogSource,
        image: String,
        artifact: String,
        _parent: PathBuf,
    ) -> images::inventory::InventoryFuture {
        Box::pin(async move {
            anyhow::ensure!(artifact.ends_with(&digest('6')), "fixture inventory unavailable");
            Ok(capsem_assets::oci::PackageInventory {
                schema: "capsem.package_inventory.v1".into(),
                subject: image,
                architecture: host().into(),
                packages: vec![capsem_assets::oci::InventoryPackage {
                    name: "git".into(),
                    version: "2.39.5".into(),
                    architecture: host().into(),
                }],
                tools: vec![capsem_assets::oci::InventoryTool {
                    name: "gh".into(),
                    version: "gh version 2.23.0".into(),
                }],
            })
        })
    }
    fn cache_state(
        &self,
        key: &capsem_assets::oci::CacheKey,
        root: Option<&Digest>,
    ) -> anyhow::Result<Option<capsem_assets::oci::CacheState>> {
        self.required_roots.lock().unwrap().push(root.cloned());
        Ok(self.disk_states.lock().unwrap().get(key).copied())
    }
    fn observe_cache(&self, keys: &[capsem_assets::oci::CacheKey], _parent: &StdPath) -> anyhow::Result<()> {
        self.observations.lock().unwrap().push(keys.to_vec());
        Ok(())
    }
    fn fetch_rootfs(
        &self,
        reference: String,
        subject: String,
        access: RegistryAccess,
        _parent: PathBuf,
        options: ImageFetchOptions,
        cache_key: Option<capsem_assets::oci::CacheKey>,
    ) -> RootfsFuture {
        assert!(
            cache_key.is_some(),
            "published filesystem requests must carry the producer cache key"
        );
        self.fetches.lock().unwrap().push(options.mode);
        self.roots.lock().unwrap().push((reference, subject, access, cache_key));
        Box::pin(async { anyhow::bail!("fixture artifact unavailable") })
    }
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
    fn pull(&self, image: String, _access: RegistryAccess, _parent: PathBuf, options: ImageFetchOptions) -> PullFuture {
        self.fetches.lock().unwrap().push(options.mode);
        self.pulls.lock().unwrap().push(image.clone());
        let delay = *self.pull_delay.lock().unwrap();
        let release = self.pull_release.lock().unwrap().clone();
        let started = self.pull_started.clone();
        Box::pin(async move {
            if let Some(observer) = &options.progress {
                observer(capsem_assets::oci::BlobProgress {
                    digest: digest('6'),
                    completed_bytes: 40,
                    total_bytes: 100,
                    verified: false,
                });
            }
            started.notify_one();
            if let Some(release) = release {
                release.notified().await;
            } else if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            if let Some(observer) = options.progress {
                observer(capsem_assets::oci::BlobProgress {
                    digest: digest('6'),
                    completed_bytes: 100,
                    total_bytes: 100,
                    verified: true,
                });
            }
            let image_digest = image
                .rsplit_once('@')
                .map_or_else(|| digest('f'), |(_, pin)| pin.to_owned());
            let requested =
                capsem_assets::oci::ImageReference::try_from(&capsem_assets::oci::image_reference(&image)?)?;
            let resolved = requested.resolve(Digest::parse(&image_digest)?)?;
            let identity = capsem_assets::oci::CacheIdentity::new(&resolved.to_string(), host(), RUNTIME_CONTRACT)?;
            Ok(PulledImage {
                root: PathBuf::new(),
                files: Vec::new(),
                digest: digest('7'),
                image_digest,
                cache_key: Some(identity.key()),
                _hold: Box::new(()),
            })
        })
    }
}

struct Fixture {
    required_roots: Arc<Mutex<Vec<Option<Digest>>>>,
    pull_delay: Arc<Mutex<std::time::Duration>>,
    pull_release: Arc<Mutex<Option<Arc<tokio::sync::Notify>>>>,
    pull_started: Arc<tokio::sync::Notify>,
    disk_states: Arc<Mutex<std::collections::HashMap<capsem_assets::oci::CacheKey, capsem_assets::oci::CacheState>>>,
    observations: Arc<Mutex<Vec<Vec<capsem_assets::oci::CacheKey>>>>,
    fetches: Arc<Mutex<Vec<ImageFetch>>>,
    state: Arc<ServiceState>,
    catalog: Arc<Mutex<Option<Value>>>,
    catalog_reads: Arc<Mutex<Vec<String>>>,
    pulls: Arc<Mutex<Vec<String>>>,
    roots: Arc<Mutex<Vec<RootCall>>>,
}

impl Fixture {
    fn new(images: ImagePolicyConfig) -> Self {
        let required_roots = Arc::new(Mutex::new(Vec::new()));
        let pull_delay = Arc::new(Mutex::new(std::time::Duration::ZERO));
        let pull_release = Arc::new(Mutex::new(None));
        let pull_started = Arc::new(tokio::sync::Notify::new());
        let disk_states = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let observations = Arc::new(Mutex::new(Vec::new()));
        let fetches = Arc::new(Mutex::new(Vec::new()));
        let catalog = Arc::new(Mutex::new(Some(catalog_document())));
        let catalog_reads = Arc::new(Mutex::new(Vec::new()));
        let pulls = Arc::new(Mutex::new(Vec::new()));
        let roots = Arc::new(Mutex::new(Vec::new()));
        let mut state = crate::tests::make_test_state_owned();
        state.containers = ContainerSetups::with_source(Box::new(CatalogImages {
            required_roots: required_roots.clone(),
            pull_delay: pull_delay.clone(),
            pull_release: pull_release.clone(),
            pull_started: pull_started.clone(),
            disk_states: Arc::clone(&disk_states),
            observations: Arc::clone(&observations),
            fetches: Arc::clone(&fetches),
            settings: SettingsFile {
                images: Some(images),
                ..Default::default()
            },
            catalog: Arc::clone(&catalog),
            catalog_reads: Arc::clone(&catalog_reads),
            pulls: Arc::clone(&pulls),
            roots: Arc::clone(&roots),
        }));
        Self {
            required_roots,
            pull_delay,
            pull_release,
            pull_started,
            disk_states,
            observations,
            fetches,
            state: Arc::new(state),
            catalog,
            catalog_reads,
            pulls,
            roots,
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

#[tokio::test]
async fn stalled_image_pull_is_cancelled_at_the_service_operation_deadline() {
    let fixture = Fixture::default_policy();
    *fixture.pull_delay.lock().unwrap() = std::time::Duration::from_secs(901);
    let started = fixture.pull_started.clone();
    let roots = fixture.roots.clone();
    let pulls = fixture.pulls.clone();
    let request = tokio::spawn(async move { fixture.pull("codex-cli").await });
    started.notified().await;
    tokio::time::pause();
    // Stop at the operation deadline, before the source's 901s sleep is ready.
    tokio::time::advance(std::time::Duration::from_secs(900)).await;
    tokio::time::resume();
    let (status, body) = request.await.unwrap();
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT, "{body}");
    assert_eq!(
        body,
        json!({"error":"image pull exceeded 900 seconds", "timeout_secs":900})
    );
    assert_eq!(pulls.lock().unwrap().len(), 1);
    assert!(
        roots.lock().unwrap().is_empty(),
        "cancelled pull continued into rootfs fetch"
    );
}

#[tokio::test]
async fn catalog_listing_schedules_native_compatible_cache_keys_without_waiting_for_proof() {
    let fixture = Fixture::default_policy();
    let (status, response) = fixture.list().await;
    assert_eq!(status, StatusCode::OK);
    let mut expected = ['a', 'b', 'd']
        .into_iter()
        .map(|pin| {
            let name = if pin != 'd' { "codex-cli" } else { "redis" };
            capsem_assets::oci::CacheIdentity::new(
                &format!("ghcr.io/google/capsem/{name}@{}", digest(pin)),
                host(),
                RUNTIME_CONTRACT,
            )
            .unwrap()
            .key()
        })
        .collect::<Vec<_>>();
    expected.sort_unstable_by(|left, right| left.as_str().cmp(right.as_str()));
    assert_eq!(*fixture.observations.lock().unwrap(), vec![expected]);
    assert!(fixture.pulls().is_empty());
    for image in response["images"].as_array().unwrap() {
        assert_eq!(image["cached"], "unknown");
    }
}

#[tokio::test]
async fn catalog_listing_preserves_terminal_and_graphical_metadata() {
    for surface in [None, Some("terminal"), Some("xpra")] {
        let fx = Fixture::default_policy();
        if let Some(surface) = surface {
            fx.catalog.lock().unwrap().as_mut().unwrap()["entries"]["codex-cli"]["surface"] = json!(surface);
        }
        let (status, response) = fx.list().await;
        assert_eq!(status, StatusCode::OK, "{response}");
        assert_eq!(response["images"][0]["surface"], json!(surface));
    }
}

#[tokio::test]
async fn catalog_disk_status_uses_only_the_selected_pins_local_owner_observation() {
    use capsem_assets::oci::CacheState;
    let fx = Fixture::default_policy();
    let key = capsem_assets::oci::CacheIdentity::new(
        &format!("ghcr.io/google/capsem/codex-cli@{}", digest('b')),
        host(),
        RUNTIME_CONTRACT,
    )
    .unwrap()
    .key();
    for (state, wire) in [
        (CacheState::Missing, "missing"),
        (CacheState::Partial, "partial"),
        (CacheState::Ready, "ready"),
        (CacheState::Unknown, "unknown"),
    ] {
        fx.disk_states.lock().unwrap().insert(key.clone(), state);
        let (status, response) = fx.list().await;
        assert_eq!(status, StatusCode::OK);
        let images = response["images"].as_array().unwrap();
        assert_eq!(
            images.len(),
            3,
            "disk observations must not replace the registry catalog"
        );
        assert_eq!(images[0]["cached"], wire);
        assert_eq!(
            images[1]["cached"], "unknown",
            "foreign-platform entries have no local selected image"
        );
        assert_eq!(
            images[2]["cached"], "unknown",
            "missing observations must not invent disk state"
        );
        assert_eq!(
            images[0]["image"],
            format!("ghcr.io/google/capsem/codex-cli@{}", digest('b'))
        );
    }
    assert_eq!(fx.reads(), 1, "cache status must not refresh the registry catalog");
    assert!(fx.pulls().is_empty(), "listing must not prefetch images");
}

fn grants(sources: &[&str], admit: &[&str]) -> ImagePolicyConfig {
    ImagePolicyConfig {
        sources: sources.iter().map(|s| s.to_string()).collect(),
        admit: admit.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

#[tokio::test]
async fn prefetch_selects_catalog_root_after_admission_with_original_platform_subject_and_access() {
    let fx = Fixture::default_policy();
    {
        let mut document = fx.catalog.lock().unwrap();
        document.as_mut().unwrap()["entries"]["codex-cli"]["versions"][1]["erofs"] = json!({host(): digest('8')});
        drop(document);
    }
    let (status, body) = fx.call(axum::http::Method::POST, "/images/pull", Some(json!({
        "image": "codex-cli", "registry": {"username": "account", "password": "private-test-password", "ca_pem": "private-test-ca"}
    }))).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert!(body.to_string().contains("fixture artifact unavailable"));
    let roots = fx.roots.lock().unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].0, format!("ghcr.io/google/capsem/codex-cli@{}", digest('8')));
    assert_eq!(
        roots[0].1,
        digest('7'),
        "root subject is the original platform manifest, not index b"
    );
    assert_eq!(roots[0].2.username.as_deref(), Some("account"));
    assert_eq!(roots[0].2.password.as_deref(), Some("private-test-password"));
    assert_eq!(roots[0].2.ca_pem.as_deref(), Some("private-test-ca"));
    let expected_key = capsem_assets::oci::CacheIdentity::new(
        &format!("ghcr.io/google/capsem/codex-cli@{}", digest('b')),
        host(),
        RUNTIME_CONTRACT,
    )
    .unwrap()
    .key();
    assert_eq!(
        roots[0].3,
        Some(expected_key),
        "filesystem retention uses the resolved index key, not its artifact or native manifest digest"
    );
    assert!(!body.to_string().contains("private-test-password") && !body.to_string().contains("private-test-ca"));
    assert_eq!(*fx.fetches.lock().unwrap(), vec![ImageFetch::Fresh, ImageFetch::Fresh]);
    drop(roots);
}

#[tokio::test]
async fn explicit_digest_admission_still_selects_the_published_filesystem() {
    let image = format!("ghcr.io/google/capsem/codex-cli@{}", digest('b'));
    let fx = Fixture::new(grants(&["ghcr.io/google/capsem/codex-cli"], &[&image]));
    fx.catalog.lock().unwrap().as_mut().unwrap()["entries"]["codex-cli"]["versions"][1]["erofs"] =
        json!({host(): digest('8')});
    let (status, body) = fx.pull(&image).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert!(body.to_string().contains("fixture artifact unavailable"));
    assert_eq!(fx.reads(), 1);
    let roots = fx.roots.lock().unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].0, format!("ghcr.io/google/capsem/codex-cli@{}", digest('8')));
    assert_eq!(roots[0].1, digest('7'));
    drop(roots);
}

#[tokio::test]
async fn denied_image_does_not_fetch_any_catalog_filesystem() {
    let fx = Fixture::new(ImagePolicyConfig {
        catalog: Some(CatalogSetting::Enabled(false)),
        ..grants(&["ghcr.io"], &[])
    });
    let (status, _) = fx
        .pull(&format!("ghcr.io/google/capsem/codex-cli@{}", digest('b')))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(fx.roots.lock().unwrap().is_empty());
    assert_eq!(
        *fx.fetches.lock().unwrap(),
        vec![ImageFetch::Fresh],
        "prefetch resolves the manifest before final digest admission; refused images never fetch a filesystem"
    );
}

#[tokio::test]
async fn conflicting_catalog_filesystem_bindings_fail_before_artifact_fetch() {
    let fx = Fixture::default_policy();
    {
        let mut guard = fx.catalog.lock().unwrap();
        let document = guard.as_mut().unwrap();
        let version = &mut document["entries"]["codex-cli"]["versions"][1];
        version["erofs"] = json!({host(): digest('8')});
        let mut alias = version.clone();
        alias["erofs"] = json!({host(): digest('6')});
        document["entries"]["alias"] = json!({"description": "same image", "versions": [alias]});
        drop(guard);
    }
    let (status, body) = fx.pull("codex-cli").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert!(fx.roots.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_missing_compatible_catalog_root_keeps_the_universal_pull_path() {
    let fx = Fixture::default_policy();
    {
        let mut document = fx.catalog.lock().unwrap();
        document.as_mut().unwrap()["entries"]["codex-cli"]["versions"][2]["erofs"] = json!({host(): digest('8')});
        drop(document);
    }
    let (status, body) = fx.pull("codex-cli").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        fx.roots.lock().unwrap().is_empty(),
        "a newer incompatible version cannot supply the root"
    );
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
async fn explicit_grants_admit_before_catalog_filesystem_lookup() {
    let fx = Fixture::new(grants(&["registry.example"], &["registry.example"]));
    let (status, body) = fx.pull("registry.example/app:1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["resolved"], format!("registry.example/app@{}", digest('f')));
    assert_eq!(fx.reads(), 1, "filesystem selection still consults the catalog");
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
                 "image": format!("ghcr.io/google/capsem/codex-cli@{}", digest('b')), "cached": "unknown",
                 "app": {"catalog_name":"codex-cli","display_name":"codex-cli","description":"OpenAI Codex CLI",
                    "image":format!("ghcr.io/google/capsem/codex-cli@{}",digest('b')),"architecture":host(),"interfaces":[],"evidence":[]}},
                {"name": "elsewhere", "description": "Another machine's", "architectures": [other()],
                 "cached": "unknown"},
                {"name": "redis", "description": "Not Docker Hub's", "architectures": [host()],
                 "image": format!("ghcr.io/google/capsem/redis@{}", digest('d')), "cached": "unknown",
                 "app": {"catalog_name":"redis","display_name":"redis","description":"Not Docker Hub's",
                    "image":format!("ghcr.io/google/capsem/redis@{}",digest('d')),"architecture":host(),"interfaces":[],"evidence":[]}},
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
    let stats = fx.state.containers.catalog.metrics.snapshot();
    assert_eq!(stats.hits, 3);
    assert_eq!(stats.misses, 3);
    assert_eq!(stats.insertions, 1);
    assert_eq!(stats.refreshes, 2);
    assert_eq!(stats.load_errors, 0);
    assert_eq!(stats.entries, 1);
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
    let stats = fx.state.containers.catalog.metrics.snapshot();
    assert_eq!(stats.load_errors, 2);
    assert_eq!(stats.entries, 1);
}

#[tokio::test]
async fn with_no_catalog_ever_read_only_explicit_grants_admit() {
    let fx = Fixture::new(grants(&["registry.example"], &["registry.example"]));
    *fx.catalog.lock().unwrap() = None;
    let (status, body) = fx.list().await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(body["error"].as_str().unwrap().contains("catalog:stable"));
    assert!(body["error"].as_str().unwrap().contains("unavailable"));
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

#[tokio::test]
async fn disabled_and_valid_empty_catalogs_are_distinct_from_unreadable_catalogs() {
    let disabled = Fixture::new(ImagePolicyConfig {
        catalog: Some(CatalogSetting::Enabled(false)),
        ..ImagePolicyConfig::default()
    });
    let (status, body) = disabled.list().await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"images": []}));
    assert_eq!(disabled.reads(), 0);

    let empty = Fixture::default_policy();
    empty.catalog.lock().unwrap().as_mut().unwrap()["entries"] = json!({});
    let (status, body) = empty.list().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["images"], json!([]));
    assert_eq!(body["catalog"]["channel"], "stable");
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

#[tokio::test]
async fn image_update_requires_a_verified_earlier_compatible_image_and_missing_selected_bytes() {
    use capsem_assets::oci::{CacheIdentity, CacheState};
    let fx = Fixture::default_policy();
    let pin = |value| format!("ghcr.io/google/capsem/codex-cli@{}", digest(value));
    let key = |value| CacheIdentity::new(&pin(value), host(), RUNTIME_CONTRACT).unwrap().key();
    for selected in [
        CacheState::Unknown,
        CacheState::Missing,
        CacheState::Partial,
        CacheState::Ready,
    ] {
        for previous in [
            CacheState::Unknown,
            CacheState::Missing,
            CacheState::Partial,
            CacheState::Ready,
        ] {
            fx.disk_states.lock().unwrap().insert(key('a'), previous);
            fx.disk_states.lock().unwrap().insert(key('b'), selected);
            fx.disk_states.lock().unwrap().insert(key('c'), CacheState::Ready);
            let (status, body) = fx.list().await;
            assert_eq!(status, StatusCode::OK);
            let row = &body["images"][0];
            let expected =
                if matches!(selected, CacheState::Missing | CacheState::Partial) && previous == CacheState::Ready {
                    json!(pin('a'))
                } else {
                    Value::Null
                };
            assert_eq!(
                row["previous_image"], expected,
                "selected={selected:?} previous={previous:?}"
            );
        }
    }
    assert!(fx
        .observations
        .lock()
        .unwrap()
        .iter()
        .any(|keys| keys.contains(&key('a'))));
    assert!(
        fx.observations
            .lock()
            .unwrap()
            .iter()
            .all(|keys| !keys.contains(&key('c'))),
        "future incompatible version must not be observed as an older version"
    );
}

#[tokio::test]
async fn catalog_info_uses_selected_version_metadata_and_never_invents_artifacts() {
    let fx = Fixture::default_policy();
    {
        let mut catalog = fx.catalog.lock().unwrap();
        let entry = &mut catalog.as_mut().unwrap()["entries"]["codex-cli"];
        entry["display_name"] = json!("Codex CLI");
        entry["publisher"] = json!("Example publisher");
        entry["url"] = json!("https://example.com/codex");
        entry["interfaces"] = json!(["terminal"]);
        entry["versions"][0]["evidence"] = json!({host(): [{"kind":"abom","reference":format!("ghcr.io/google/capsem/codex-cli@{}",digest('8')),"media_type":"application/json"}]});
        entry["versions"][1]["obom"] = json!({host():digest('7')});
        drop(catalog);
    }
    let (status, body) = fx.list().await;
    assert_eq!(status, StatusCode::OK);
    let app = &body["images"][0]["app"];
    assert_eq!(app["display_name"], "Codex CLI");
    assert_eq!(app["publisher"], "Example publisher");
    assert_eq!(app["image"], format!("ghcr.io/google/capsem/codex-cli@{}", digest('b')));
    assert_eq!(app["architecture"], host());
    assert!(app.get("icon").is_none());
    assert_eq!(
        app["evidence"],
        json!([{"kind":"obom","reference":format!("ghcr.io/google/capsem/codex-cli@{}",digest('7')),"media_type":"application/vnd.cyclonedx+json"}])
    );
    assert!(body["images"][1]["app"].is_null());
    assert_eq!(body["images"][2]["app"]["evidence"], json!([]));
}
