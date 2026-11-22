use super::*;
use crate::container_setup::{CatalogPolicyFuture, ContainerSetups, ImageFetchOptions, ImageSource, PullFuture};
use capsem_config::{
    catalogs::{CatalogKind, CatalogSourceDescriptor, CatalogSources},
    environment::EnvironmentCatalogSelection,
    SettingsFile,
};
use capsem_core::{
    container::admission::CatalogSource,
    environment::{EnvironmentPinResolver, EnvironmentResolutionError},
};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
mod registry;
mod applied_skills;
mod global_skills;
mod global_rules;
mod status;
use registry::Registry;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn launch_policy_uses_pinned_rule_content_and_current_catalog_admission() {
    let _lock = crate::tests::SETTINGS_ENV_LOCK.lock().await;
    let directory = tempfile::tempdir().unwrap();
    let (_paths, _, _) = crate::tests::install_empty_settings_env(&directory);
    let fx = Fixture::new().await;
    let kind = CatalogKind::Rules;
    let original = fx.registry.publish(kind, document(kind, false));
    let app = crate::router_runtime::build_service_router(fx.state.clone());
    let intent = json!({"request_id":id(401),"reason":"pinned runtime rule","specification":{"name":"runtime-rules","access":{"catalog":{"selected":[{"id":"review","version":"1.0.0"}]}}}});
    let (status, body) = route_request(app.clone(), Method::POST, "/environments/create", Some(intent)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let response: capsem_api::EnvironmentMutationResponse = serde_json::from_value(body).unwrap();
    let reference = response.result.unwrap();
    let reference = capsem_logger::events::EnvironmentRevisionParent {
        environment_id: reference.environment_id,
        revision_id: reference.revision_id,
    };
    let mut changed = document(kind, false);
    changed["entries"][0]["definition"]["rule"]["action"] = "block".into();
    assert_ne!(fx.registry.publish(kind, changed), original);
    let route = format!("/environments/{}/rules/list?revision_id={}", reference.environment_id, reference.revision_id);
    let (status, projected) = route_request(app.clone(), Method::GET, &route, None).await;
    assert_eq!(status, StatusCode::OK, "{projected}");
    let projected: capsem_api::EnvironmentRuleListResponse = serde_json::from_value(projected).unwrap();
    let selected = projected.access.iter().find(|rule| rule.id == "environment.access.catalog.review").unwrap();
    assert_eq!(selected.action, capsem_config::SecurityRuleAction::Allow);
    assert_eq!(selected.source, capsem_api::environments::EnvironmentRuleSource::Catalog);
    assert!(selected.editable_key.is_none());
    let compile = || {
        let reference = reference.clone();
        let session = directory.path().join("launch");
        fx.state
            .off_worker(move |state| state.materialize_environment_policy(&session, Some(&reference), Some(&reference)))
    };
    let published = compile().await.unwrap().unwrap();
    let rule = published
        .runtime
        .security_rules
        .rules()
        .iter()
        .find(|rule| rule.rule_id == "environment.access.catalog.review")
        .unwrap();
    assert_eq!(
        rule.action,
        capsem_config::SecurityRuleAction::Allow,
        "moving the catalog tag cannot replace the selected definition"
    );
    fx.registry.publish(kind, document(kind, true));
    assert!(
        compile().await.unwrap().is_err(),
        "current revocation must refuse publication"
    );
    fx.registry.publish(kind, document(kind, false));
    assert!(
        compile().await.unwrap().is_err(),
        "known revocation survives catalog rollback"
    );
    let (status, _) = route_request(app, Method::GET, &route, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    fx.corporate.lock().unwrap().catalogs = Some(CatalogSources {
        rules: Some(descriptor(format!("{}/team/private:stable", fx.registry.address))),
        ..Default::default()
    });
    let calls = fx.registry.requests.lock().unwrap().len();
    assert!(
        compile().await.unwrap().is_err(),
        "corporate source replacement invalidates the pin"
    );
    assert_eq!(
        fx.registry.requests.lock().unwrap().len(),
        calls,
        "a rejected source cannot fetch either the old source or a replacement"
    );
}

struct Source {
    ca_pem: String,
    cache: capsem_assets::oci::ImageCache,
    corporate: Arc<Mutex<SettingsFile>>,
    broken: Arc<Mutex<bool>>,
}
impl ImageSource for Source {
    fn pull(&self, _: String, _: capsem_api::RegistryAccess, _: PathBuf, _: ImageFetchOptions) -> PullFuture {
        Box::pin(async { anyhow::bail!("unexpected image request") })
    }
    fn fetch_catalog(&self, _: CatalogSource, _: PathBuf) -> crate::container_setup::images::CatalogFuture {
        Box::pin(async { anyhow::bail!("unexpected image catalog request") })
    }
    fn environment_catalog_puller(&self) -> anyhow::Result<capsem_assets::oci::Puller> {
        Ok(capsem_assets::oci::Puller::new_with_root_certificate(
            capsem_core::container::stage::oci_architecture()?,
            capsem_assets::oci::RegistryAuth::Anonymous,
            Some(self.ca_pem.as_bytes()),
        )?
        .with_cache(&self.cache))
    }
    fn environment_catalog_policy(&self) -> CatalogPolicyFuture {
        let corporate = self.corporate.lock().unwrap().clone();
        let broken = *self.broken.lock().unwrap();
        Box::pin(async move {
            anyhow::ensure!(!broken, "broken corporate policy");
            Ok(corporate)
        })
    }
}
struct Fixture {
    registry: Registry,
    state: Arc<crate::ServiceState>,
    corporate: Arc<Mutex<SettingsFile>>,
    broken: Arc<Mutex<bool>>,
}
impl Fixture {
    async fn new() -> Self {
        let registry = Registry::start().await;
        let corporate = Arc::new(Mutex::new(SettingsFile::default()));
        let broken = Arc::new(Mutex::new(false));
        let mut state = crate::tests::make_test_state_owned();
        let cache = capsem_assets::oci::ImageCache::at(&state.run_dir.join("catalog-cache")).unwrap();
        state.containers = ContainerSetups::with_source(Box::new(Source {
            ca_pem: registry.ca_pem.clone(),
            cache,
            corporate: corporate.clone(),
            broken: broken.clone(),
        }));
        let mut manifest = crate::tests::test_runtime_manifest();
        manifest.catalogs = Some(CatalogSources {
            skills: Some(descriptor(registry.reference(CatalogKind::Skills))),
            rules: Some(descriptor(registry.reference(CatalogKind::Rules))),
            mcp: None,
        });
        *state.manifest.write().unwrap() = Some(Arc::new(manifest));
        Self {
            registry,
            state: Arc::new(state),
            corporate,
            broken,
        }
    }
    fn source(&self) -> super::super::sources::Sources {
        super::super::sources::Sources(self.state.clone())
    }
}
fn descriptor(reference: String) -> CatalogSourceDescriptor {
    CatalogSourceDescriptor {
        reference,
        schema_version: 1,
    }
}
fn selection() -> EnvironmentCatalogSelection {
    EnvironmentCatalogSelection {
        id: "review".into(),
        version: "1.0.0".into(),
    }
}
fn document(kind: CatalogKind, revoked: bool) -> serde_json::Value {
    let metadata = json!({"id":"review","version":"1.0.0","name":"Review","publisher":"fixture","provenance":"fixture source","license":"Apache-2.0","compatibility":{"min_runtime_contract":1},"status":if revoked {"revoked"} else {"active"},"status_reason":if revoked {json!("withdrawn")} else {serde_json::Value::Null}});
    let mut metadata = metadata.as_object().unwrap().clone();
    if !revoked {
        metadata.remove("status_reason");
    }
    let mut entry = json!({"metadata":metadata});
    if !revoked {
        match kind {
            CatalogKind::Skills => {
                let content = "---\nname: review\ndescription: Review changes\n---\nReview changes carefully.\n";
                entry["payload"] =
                    json!({"files":{"SKILL.md":{"content":content,"sha256":registry::digest(content.as_bytes())}}});
            }
            CatalogKind::Rules => {
                entry["definition"] = json!({"kind":"access","rule":{"name":"Review endpoint","action":"allow","match":"http.host == 'api.example'"}})
            }
            CatalogKind::Mcp => unreachable!(),
        }
    }
    json!({"schema_version":1,"kind":kind.as_str(),"entries":[entry]})
}

#[tokio::test]
async fn selected_skill_http_uses_complete_pinned_content_and_durable_admission() {
    let mut fx = Fixture::new().await;
    let kind = CatalogKind::Skills;
    let mut original = document(kind, false);
    let support = "Review the fixture output.\n";
    original["entries"][0]["payload"]["files"]["references/fixture.md"] =
        json!({"content":support,"sha256":registry::digest(support.as_bytes())});
    let mut notes = original["entries"][0].clone();
    notes["metadata"]["id"] = "notes".into();
    let instruction = "---\nname: notes\ndescription: Read notes\n---\nRead the notes.\n";
    notes["payload"]["files"]["SKILL.md"] = json!({"content":instruction,"sha256":registry::digest(instruction.as_bytes())});
    original["entries"].as_array_mut().unwrap().push(notes);
    let digest = fx.registry.publish(kind, original.clone());
    let app = crate::router_runtime::build_service_router(fx.state.clone());
    let (status, saved) = route_request(app.clone(), Method::POST, "/environments/create", Some(json!({
        "request_id":id(671), "reason":"selected skill fixture", "specification":{
            "name":"skill-inspection","skills":{"selected":[{"id":"review","version":"1.0.0"},{"id":"notes","version":"1.0.0"}]}
        }
    }))).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let saved: capsem_api::EnvironmentMutationResponse = serde_json::from_value(saved).unwrap();
    let exact = saved.result.unwrap();
    let base = format!("/environments/{}/skills", exact.environment_id);
    let list = format!("{base}/list?search=FIXTURE&revision_id={}", exact.revision_id);
    let info = format!("{base}/review/info?revision_id={}", exact.revision_id);
    let (status, summary) = route_request(app.clone(), Method::GET, &list, None).await;
    assert_eq!(status, StatusCode::OK, "{summary}");
    assert_eq!(summary["skills"][0]["selection"]["resolved_digest"], digest);
    assert_eq!(summary["skills"][0]["availability"], "available");
    assert!(summary["skills"][0].get("files").is_none());
    assert_eq!(route_request(app.clone(), Method::GET, &format!("{base}/unselected/info"), None).await.0,
        StatusCode::NOT_FOUND);
    let (status, first) = route_request(app.clone(), Method::GET, &format!("{base}/list?limit=1"), None).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["skills"][0]["selection"]["id"], "notes");
    let cursor = first["next_cursor"].as_str().unwrap();
    let next = format!("{base}/list?limit=1&after={cursor}");
    let (status, second) = route_request(app.clone(), Method::GET, &next, None).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["skills"][0]["selection"]["id"], "review");
    assert!(second["next_cursor"].is_null());

    let mut moved = document(kind, false);
    moved["entries"][0]["metadata"]["name"] = "New tag metadata".into();
    assert_ne!(fx.registry.publish(kind, moved), digest);
    let (status, detail) = route_request(app.clone(), Method::GET, &info, None).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["metadata"]["name"], "Review");
    assert_eq!(detail["selection"]["resolved_digest"], digest);
    assert_eq!(detail["files"], json!({
        "SKILL.md":original["entries"][0]["payload"]["files"]["SKILL.md"]["content"],
        "references/fixture.md":support
    }));

    fx.registry.unpublish_tag(kind);
    let (status, retained) = route_request(app.clone(), Method::GET, &info, None).await;
    assert_eq!(status, StatusCode::OK, "{retained}");
    assert_eq!(retained["files"], detail["files"]);
    assert_eq!(retained["discovery_fresh"], false);

    fx.registry.publish(kind, document(kind, true));
    assert_eq!(route_request(app.clone(), Method::GET, &info, None).await.0, StatusCode::FORBIDDEN);
    fx.registry.publish(kind, original);
    let (status, summary) = route_request(app.clone(), Method::GET, &format!("{base}/list"), None).await;
    assert_eq!(status, StatusCode::OK, "{summary}");
    let skill = summary["skills"].as_array().unwrap().iter().find(|skill| skill["selection"]["id"] == "review").unwrap();
    assert_eq!(skill["availability"], "revoked");
    assert!(skill["metadata"].is_null());
    fx.registry.stop().await;
    assert_eq!(route_request(app.clone(), Method::GET, &info, None).await.0, StatusCode::FORBIDDEN);
    let (status, updated) = route_request(app.clone(), Method::POST, &format!("/environments/{}/changes", exact.environment_id), Some(json!({
        "request_id":id(672),"base_revision_id":exact.revision_id,"reason":"remove selected skills","specification":{"name":"skill-inspection"}
    }))).await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(route_request(app.clone(), Method::GET, &next, None).await.0, StatusCode::CONFLICT);
    assert_eq!(route_request(app, Method::GET, &format!("{base}/review/info"), None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn native_catalog_pins_verify_tls_oci_content_and_retain_exact_bytes_offline() {
    let mut fx = Fixture::new().await;
    let mut pins = Vec::new();
    for kind in [CatalogKind::Skills, CatalogKind::Rules] {
        let digest = fx.registry.publish(kind, document(kind, false));
        let pin = fx.source().catalog(kind, &selection()).await.unwrap();
        assert_eq!(pin.resolved_digest, digest);
        assert_eq!(pin.catalog_reference, fx.registry.reference(kind));
        pins.push((kind, pin));
    }
    let requests = fx.registry.requests.lock().unwrap().len();
    fx.registry.stop().await;
    for (kind, pin) in pins {
        fx.source().admit_catalog(kind, &pin).await.unwrap();
    }
    assert_eq!(
        fx.registry.requests.lock().unwrap().len(),
        requests,
        "retained pins cannot fetch or resolve tags"
    );
}

#[tokio::test]
async fn native_catalog_revocations_survive_refresh_rollback_and_reopened_ledger_reads() {
    let fx = Fixture::new().await;
    let kind = CatalogKind::Skills;
    fx.registry.publish(kind, document(kind, false));
    let pin = fx.source().catalog(kind, &selection()).await.unwrap();
    let revoked_digest = fx.registry.publish(kind, document(kind, true));
    assert_eq!(
        fx.source().catalog(kind, &selection()).await.err(),
        Some(EnvironmentResolutionError::UnusableCatalogEntry)
    );
    assert_eq!(
        fx.source().admit_catalog(kind, &pin).await.err(),
        Some(EnvironmentResolutionError::UnusableCatalogEntry)
    );
    fx.registry.publish(kind, document(kind, false));
    assert_eq!(
        fx.source().catalog(kind, &selection()).await.err(),
        Some(EnvironmentResolutionError::UnusableCatalogEntry)
    );
    let reopened = capsem_logger::DbHandle::open_external_reader(fx.state.host_ledger.path()).unwrap();
    let saved = reopened
        .catalog_revocation(capsem_logger::catalog_revocations::CatalogRevocationKey {
            kind,
            source_repository: format!("{}/team/skills", fx.registry.address),
            entry_id: "review".into(),
            version: "1.0.0".into(),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.catalog_digest, revoked_digest);
}

#[tokio::test]
async fn corporate_catalog_source_failure_never_falls_back_and_retained_pins_recheck_policy() {
    let fx = Fixture::new().await;
    let kind = CatalogKind::Skills;
    fx.registry.publish(kind, document(kind, false));
    let pin = fx.source().catalog(kind, &selection()).await.unwrap();
    let calls = fx.registry.requests.lock().unwrap().len();
    fx.corporate.lock().unwrap().catalogs = Some(CatalogSources {
        skills: Some(descriptor(format!("{}/team/private:missing", fx.registry.address))),
        ..Default::default()
    });
    assert_eq!(
        fx.source().admit_catalog(kind, &pin).await.err(),
        Some(EnvironmentResolutionError::Refused)
    );
    assert!(fx.source().catalog(kind, &selection()).await.is_err());
    assert!(
        fx.registry.requests.lock().unwrap()[calls..]
            .iter()
            .all(|path| !path.starts_with("team/skills/")),
        "corporate failure reached manifest fallback"
    );
    *fx.broken.lock().unwrap() = true;
    let calls = fx.registry.requests.lock().unwrap().len();
    assert_eq!(
        fx.source().catalog(kind, &selection()).await.err(),
        Some(EnvironmentResolutionError::Unavailable)
    );
    assert_eq!(fx.registry.requests.lock().unwrap().len(), calls);
}

#[tokio::test]
async fn operator_creation_freezes_native_skill_and_rule_pins_and_retry_does_not_refresh() {
    let fx = Fixture::new().await;
    let skill = fx
        .registry
        .publish(CatalogKind::Skills, document(CatalogKind::Skills, false));
    let rule = fx
        .registry
        .publish(CatalogKind::Rules, document(CatalogKind::Rules, false));
    let intent = json!({"request_id":id(201),"reason":"create with verified catalogs","specification":{"name":"development","skills":{"selected":[{"id":"review","version":"1.0.0"}]},"access":{"catalog":{"selected":[{"id":"review","version":"1.0.0"}]}}}});
    let app = crate::router_runtime::build_service_router(fx.state.clone());
    let (status, body) = route_request(app.clone(), Method::POST, "/environments/create", Some(intent.clone())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result: capsem_api::EnvironmentMutationResponse = serde_json::from_value(body.clone()).unwrap();
    let current = fx
        .state
        .environments
        .load_current(&result.environment_id)
        .await
        .unwrap();
    assert_eq!(current.document.specification.skills.selected[0].resolved_digest, skill);
    assert_eq!(
        current.document.specification.access.catalog.selected[0].resolved_digest,
        rule
    );
    let calls = fx.registry.requests.lock().unwrap().len();
    *fx.broken.lock().unwrap() = true;
    let (status, repeated) = route_request(app, Method::POST, "/environments/create", Some(intent)).await;
    assert_eq!(status, StatusCode::OK, "{repeated}");
    assert_eq!(repeated, body);
    assert_eq!(fx.registry.requests.lock().unwrap().len(), calls);
}

#[tokio::test]
async fn corporate_catalog_source_wins_even_when_manifest_selection_is_invalid() {
    let fx = Fixture::new().await;
    fx.registry
        .publish(CatalogKind::Skills, document(CatalogKind::Skills, false));
    fx.registry.alias("team/skills/", "team/private/");
    let reference = format!("{}/team/private:stable", fx.registry.address);
    fx.corporate.lock().unwrap().catalogs = Some(CatalogSources {
        skills: Some(descriptor(reference.clone())),
        ..Default::default()
    });
    let mut manifest = (**fx.state.manifest.read().unwrap().as_ref().unwrap()).clone();
    manifest.catalogs.as_mut().unwrap().skills = Some(descriptor("invalid-manifest-canary".into()));
    *fx.state.manifest.write().unwrap() = Some(Arc::new(manifest));
    let pin = fx.source().catalog(CatalogKind::Skills, &selection()).await.unwrap();
    assert_eq!(pin.catalog_reference, reference);
    assert!(fx
        .registry
        .requests
        .lock()
        .unwrap()
        .iter()
        .all(|path| !path.starts_with("team/skills/")));
}

#[tokio::test]
async fn native_catalog_integrity_or_kind_failure_never_persists_a_reviewable_request() {
    let fx = Fixture::new().await;
    for (index, case) in ["hash", "kind", "schema"].into_iter().enumerate() {
        let mut content = document(CatalogKind::Skills, false);
        match case {
            "hash" => {
                content["entries"][0]["payload"]["files"]["SKILL.md"]["sha256"] =
                    format!("sha256:{}", "f".repeat(64)).into()
            }
            "kind" => content["kind"] = "rules".into(),
            _ => content["schema_version"] = 99.into(),
        }
        fx.registry.publish(CatalogKind::Skills, content);
        let request_id = id(301 + index as u64);
        let intent = json!({"request_id": request_id, "reason":"verify native content","specification":{"name":"development","skills":{"selected":[{"id":"review","version":"1.0.0"}]}}});
        let app = crate::router_runtime::build_service_router(fx.state.clone());
        let (status, body) = route_request(app, Method::POST, "/environments/create", Some(intent)).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{case}: {body}");
        assert!(fx
            .state
            .host_ledger
            .environment_request_snapshot(&request_id)
            .await
            .unwrap()
            .is_none());
    }
}
