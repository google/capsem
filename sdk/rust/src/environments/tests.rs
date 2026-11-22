use super::*;
use crate::{test_gateway::Server, Error, Hypervisor};
use axum::{body::Body, http::Response};
use serde_json::json;
use std::time::Duration;

const REQUEST: &str = "00000000-0000-4000-8000-000000000001";
const ENVIRONMENT: &str = "00000000-0000-4000-8000-000000000002";
fn request() -> models::EnvironmentCreateRequest {
    serde_json::from_value(
        json!({"request_id": REQUEST, "specification": {"name":"development"}, "reason":"prepare blueprint"}),
    )
    .unwrap()
}
fn pending() -> String {
    json!({"request_id":REQUEST,"environment_id":ENVIRONMENT,"decision":"approved","application":"pending","result":null,"application_error":null}).to_string()
}

#[tokio::test]
async fn selected_skill_methods_use_the_exact_revision_and_pin() {
    let reference = EnvironmentRevisionReference {
        environment_id: ENVIRONMENT.into(),
        revision_id: REQUEST.into(),
    };
    let result_revision = reference.clone();
    let mut server = Server::respond(move |parts| {
        let selection = json!({"id":"review","version":"1.0.0","catalog_reference":"registry.example/skills:stable",
            "resolved_digest":format!("sha256:{}", "0".repeat(64))});
        let metadata = json!({"id":"review","version":"1.0.0","name":"Review","status":"active",
            "publisher":"fixture","provenance":"fixture","license":"Apache-2.0","compatibility":{"min_runtime_contract":1}});
        let body = if parts.uri.path().ends_with("/list") {
            json!({"revision":result_revision,"discovery_fresh":false,"skills":[
                {"selection":selection,"metadata":metadata,"availability":"available"}
            ]})
        } else {
            json!({"revision":result_revision,"selection":selection,"metadata":metadata,
                "discovery_fresh":false,"files":{"SKILL.md":"review fixture\n"}})
        };
        Response::new(Body::from(body.to_string()))
    }).await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    let environments = hv.environments();
    let page = environments
        .skills(&reference, Some("review"), None, Some(1))
        .await
        .unwrap();
    assert_eq!(page.skills[0].selection.id, "review");
    let (listing, _) = server.received.recv().await.unwrap();
    assert_eq!(listing.uri.path(), format!("/environments/{ENVIRONMENT}/skills/list"));
    assert_eq!(
        listing.uri.query().unwrap(),
        format!("revision_id={REQUEST}&search=review&limit=1")
    );
    assert_eq!(
        environments.skill(&reference, "review").await.unwrap().files["SKILL.md"],
        "review fixture\n"
    );
    let (detail, _) = server.received.recv().await.unwrap();
    assert_eq!(
        detail.uri.path(),
        format!("/environments/{ENVIRONMENT}/skills/review/info")
    );
    assert_eq!(detail.uri.query().unwrap(), format!("revision_id={REQUEST}"));
    assert!(matches!(
        environments.skill(&reference, "other").await,
        Err(Error::InvalidResponse(_))
    ));
}

#[tokio::test]
async fn vm_skill_reads_bind_the_sandbox_and_expected_boot_revision() {
    let mut server = Server::respond(|parts| {
        let revision = json!({"environment_id":ENVIRONMENT,"revision_id":REQUEST});
        let body = if parts.uri.path().ends_with("/list") {
            json!({"vm_id":"vm-1","owner_generation":REQUEST,"revision":revision,"skills":[],"discovery_fresh":false})
        } else {
            json!({"vm_id":"vm-1","owner_generation":REQUEST,"skill":{
                "revision":revision,"discovery_fresh":false,"files":{"SKILL.md":"review fixture\n"},
                "selection":{"id":"review","version":"1.0.0","catalog_reference":"registry.example/skills:stable",
                    "resolved_digest":format!("sha256:{}", "0".repeat(64))},
                "metadata":{"id":"review","version":"1.0.0","name":"Review","status":"active",
                    "publisher":"fixture","provenance":"fixture","license":"Apache-2.0","compatibility":{"min_runtime_contract":1}},
            }})
        };
        Response::new(Body::from(body.to_string()))
    }).await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    let vm = hv.vm(crate::VmSelector::Id("vm-1".into())).unwrap();
    let query = models::environments::EnvironmentSkillListQuery {
        revision_id: Some(REQUEST.into()),
        search: None,
        after: None,
        limit: Some(1),
    };
    assert_eq!(vm.skills(query).await.unwrap().owner_generation, REQUEST);
    let (listing, _) = server.received.recv().await.unwrap();
    assert_eq!(listing.uri.path(), "/vms/vm-1/skills/list");
    assert_eq!(listing.uri.query().unwrap(), format!("revision_id={REQUEST}&limit=1"));
    let detail = vm.skill("review", Some(REQUEST)).await.unwrap();
    assert_eq!(detail.skill.files["SKILL.md"], "review fixture\n");
    let (request, _) = server.received.recv().await.unwrap();
    assert_eq!(request.uri.path(), "/vms/vm-1/skills/review/info");
    assert!(matches!(
        vm.skill("review", Some(ENVIRONMENT)).await,
        Err(Error::InvalidResponse(_))
    ));
}

#[tokio::test]
async fn vm_mcp_inventory_preserves_identity_and_input_schema() {
    let mut server = Server::respond(|parts| {
        let body = if parts.uri.path().ends_with("/mcps") {
            json!({"vm_id":"vm-1","owner_generation":REQUEST,"servers":[]})
        } else {
            json!({"vm_id":"vm-1","owner_generation":REQUEST,"server_id":"library","tools":[{
                "namespaced_name":"library__search","original_name":"search",
                "input_schema":{"type":"object","properties":{"query":{"type":"string"}}},
                "definition_digest":"observed",
            }]})
        };
        Response::new(Body::from(body.to_string()))
    })
    .await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    let vm = hv.vm(crate::VmSelector::Id("vm-1".into())).unwrap();
    assert_eq!(vm.mcps().await.unwrap().owner_generation, REQUEST);
    let (listing, _) = server.received.recv().await.unwrap();
    assert_eq!(listing.uri.path(), "/vms/vm-1/mcps");
    let list = vm.mcp_tools("library").await.unwrap();
    assert_eq!(list.tools[0].input_schema["properties"]["query"]["type"], "string");
    let (tools, _) = server.received.recv().await.unwrap();
    assert_eq!(tools.uri.path(), "/vms/vm-1/mcps/library/tools");
    assert!(matches!(vm.mcp_tools("other").await, Err(Error::InvalidResponse(_))));
    let other = hv.vm(crate::VmSelector::Id("vm-2".into())).unwrap();
    assert!(matches!(other.mcps().await, Err(Error::InvalidResponse(_))));
}

#[tokio::test]
async fn plugin_defaults_and_saved_results_require_their_exact_revision_context() {
    let mut server = Server::respond(|parts| {
        let revision = if parts.uri.path() == "/environments/plugins/list" {
            serde_json::Value::Null
        } else {
            json!({"environment_id": ENVIRONMENT, "revision_id": REQUEST})
        };
        Response::new(Body::from(json!({"revision": revision, "plugins": []}).to_string()))
    })
    .await;
    let hypervisor = Hypervisor::new(&server.url, "private-token").unwrap();
    let environments = hypervisor.environments();
    assert!(environments.plugins(None).await.unwrap().revision.is_none());
    let reference = EnvironmentRevisionReference {
        environment_id: ENVIRONMENT.into(),
        revision_id: REQUEST.into(),
    };
    assert_eq!(
        environments.plugins(Some(&reference)).await.unwrap().revision,
        Some(reference)
    );
    let stale = EnvironmentRevisionReference {
        environment_id: ENVIRONMENT.into(),
        revision_id: ENVIRONMENT.into(),
    };
    assert!(matches!(
        environments.plugins(Some(&stale)).await,
        Err(Error::InvalidResponse(_))
    ));

    let (defaults, _) = server.received.recv().await.unwrap();
    assert_eq!(defaults.uri.path(), "/environments/plugins/list");
    let (saved, _) = server.received.recv().await.unwrap();
    assert_eq!(saved.uri.path(), format!("/environments/{ENVIRONMENT}/plugins/list"));
    assert_eq!(saved.uri.query().unwrap(), format!("revision_id={REQUEST}"));
    let (stale, _) = server.received.recv().await.unwrap();
    assert_eq!(stale.uri.query().unwrap(), format!("revision_id={ENVIRONMENT}"));
    assert!(server.received.try_recv().is_err());
}

#[tokio::test]
async fn facade_preserves_request_identity_application_state_and_explicit_reconciliation() {
    let mut server = Server::respond(|parts| {
        let response = if parts.uri.path() == "/environments/create" { pending() } else {
            json!({"document":{"identity":{"schema_version":1,"id":ENVIRONMENT,"revision_id":REQUEST},"specification":{"name":"development"}},"archived":false,"timestamp_unix_ms":1,"actor":"operator","reason":"approved","request_id":REQUEST}).to_string()
        };
        Response::new(Body::from(response))
    }).await;
    let hypervisor = Hypervisor::new(&server.url, "private-token").unwrap();
    let environments = hypervisor.environments();
    let created = environments.create(request()).await.unwrap();
    assert_eq!(created.request_id, REQUEST);
    assert_eq!(created.decision, models::requests::RequestState::Approved);
    assert_eq!(created.application, models::requests::RequestApplicationState::Pending);
    assert!(created.result.is_none());
    let current = environments.info(ENVIRONMENT).await.unwrap();
    assert_eq!(current.document.identity.id, ENVIRONMENT);
    assert_eq!(current.document.specification.name, "development");
    assert_eq!(current.actor, "operator");
    let repeated = environments.create(request()).await.unwrap();
    assert_eq!(repeated, created);
    for (method, path) in [
        ("POST", "/environments/create".into()),
        ("GET", format!("/environments/{ENVIRONMENT}/info")),
        ("POST", "/environments/create".into()),
    ] {
        let (parts, body) = server.received.recv().await.unwrap();
        assert_eq!(parts.method.as_str(), method);
        assert_eq!(parts.uri.path(), path);
        assert_eq!(parts.headers["authorization"], "Bearer private-token");
        if method == "POST" {
            let actual: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(actual["request_id"], REQUEST);
            assert_eq!(actual["specification"]["name"], "development");
            assert_eq!(actual["reason"], "prepare blueprint");
            assert!(actual.get("actor").is_none());
        } else {
            assert!(body.is_empty());
        }
    }
    assert!(server.received.try_recv().is_err());
}

#[tokio::test]
async fn facade_conflict_never_replays_or_refreshes() {
    let mut server = Server::reply(409, b"changed request intent", None).await;
    let hypervisor = Hypervisor::new(&server.url, "private-token").unwrap();
    assert!(matches!(
        hypervisor.environments().create(request()).await,
        Err(Error::Http { status: 409, .. })
    ));
    assert_eq!(
        server.received.recv().await.unwrap().0.uri.path(),
        "/environments/create"
    );
    assert!(server.received.try_recv().is_err());
}

#[tokio::test]
async fn facade_creation_preserves_explicit_body_deadline_without_replay() {
    let mut server = Server::respond(|_| {
        let stream = futures::stream::once(async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok::<_, std::io::Error>(pending())
        });
        Response::new(Body::from_stream(stream))
    })
    .await;
    let hypervisor = Hypervisor::new(&server.url, "private-token")
        .unwrap()
        .with_timeout(Duration::from_millis(20))
        .unwrap();
    assert!(
        matches!(hypervisor.environments().create(request()).await, Err(Error::Transport(error)) if error.is_timeout())
    );
    server.received.recv().await.unwrap();
    assert!(server.received.try_recv().is_err());
}

#[tokio::test]
async fn facade_refuses_substituted_request_identity() {
    let response = pending().replace(REQUEST, "00000000-0000-4000-8000-000000000099");
    let mut server = Server::reply(200, response.as_bytes(), None).await;
    let hypervisor = Hypervisor::new(&server.url, "private-token").unwrap();
    assert!(matches!(
        hypervisor.environments().create(request()).await,
        Err(Error::InvalidResponse(_))
    ));
    server.received.recv().await.unwrap();
    assert!(server.received.try_recv().is_err());
}

#[tokio::test]
async fn facade_refuses_substituted_environment_identity() {
    let response = json!({"document":{"identity":{"schema_version":1,"id":REQUEST,"revision_id":REQUEST},"specification":{"name":"development"}},"archived":false,"timestamp_unix_ms":1,"actor":"operator","reason":"approved","request_id":REQUEST}).to_string();
    let mut server = Server::reply(200, response.as_bytes(), None).await;
    let hypervisor = Hypervisor::new(&server.url, "private-token").unwrap();
    assert!(matches!(
        hypervisor.environments().info(ENVIRONMENT).await,
        Err(Error::InvalidResponse(_))
    ));
    server.received.recv().await.unwrap();
    assert!(server.received.try_recv().is_err());
}

#[tokio::test]
async fn changes_preserve_base_and_refuse_substituted_request_or_environment() {
    let base = "00000000-0000-4000-8000-000000000003";
    for (response, valid) in [
        (pending(), true),
        (pending().replace(REQUEST, base), false),
        (pending().replace(ENVIRONMENT, base), false),
    ] {
        let mut server = Server::reply(200, response.as_bytes(), None).await;
        let hv = Hypervisor::new(&server.url, "private-token").unwrap();
        let input = serde_json::from_value(json!({"request_id":REQUEST,"base_revision_id":base,"specification":{"name":"development"},"reason":"update blueprint"})).unwrap();
        let result = hv.environments().change(ENVIRONMENT, input).await;
        if valid {
            assert_eq!(
                result.unwrap().application,
                models::requests::RequestApplicationState::Pending
            );
        } else {
            assert!(matches!(result, Err(Error::InvalidResponse(_))));
        }
        let (parts, body) = server.received.recv().await.unwrap();
        assert_eq!(parts.method, "POST");
        assert_eq!(parts.uri.path(), format!("/environments/{ENVIRONMENT}/changes"));
        assert_eq!(parts.headers["authorization"], "Bearer private-token");
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["request_id"], REQUEST);
        assert_eq!(body["base_revision_id"], base);
        assert!(server.received.try_recv().is_err());
    }
}

#[tokio::test]
async fn archive_environment_submits_an_exact_base_on_the_changes_route() {
    use models::environments::{EnvironmentArchiveAction, EnvironmentArchiveRequest};

    let mut server = Server::reply(200, pending().as_bytes(), None).await;
    let hypervisor = Hypervisor::new(&server.url, "private-token").unwrap();
    let base = "00000000-0000-4000-8000-000000000003";
    let request = EnvironmentArchiveRequest {
        action: EnvironmentArchiveAction::Archive,
        request_id: REQUEST.into(),
        base_revision_id: base.into(),
        reason: "retire blueprint".into(),
    };
    assert_eq!(
        hypervisor
            .environments()
            .archive(ENVIRONMENT, request)
            .await
            .unwrap()
            .request_id,
        REQUEST
    );
    let (parts, body) = server.received.recv().await.unwrap();
    assert_eq!(parts.method, "POST");
    assert_eq!(parts.uri.path(), format!("/environments/{ENVIRONMENT}/changes"));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        json!({
            "action": "archive", "request_id": REQUEST, "base_revision_id": base, "reason": "retire blueprint",
        })
    );
}

#[tokio::test]
async fn catalog_skills_use_the_exact_listed_version_and_source() {
    let mut server = Server::respond(|parts| {
        let reference = "registry.example/skills:stable";
        let digest = format!("sha256:{}", "0".repeat(64));
        let metadata = json!({"id":"review","version":"1.0.0+preview","name":"Review","status":"active",
            "publisher":"fixture","provenance":"fixture","license":"Apache-2.0","compatibility":{"min_runtime_contract":1}});
        let body = if parts.uri.path() == "/skills/list" {
            json!({"catalog":{"reference":reference,"digest":digest,"snapshot":"listed-snapshot"},
                "skills":[metadata],"next_cursor":"next"})
        } else {
            json!({"selection":{"id":"review","version":"1.0.0+preview","catalog_reference":reference,"resolved_digest":digest},
                "metadata":metadata,"files":{"SKILL.md":"review fixture\n","references/notes.md":"notes\n"}})
        };
        Response::new(Body::from(body.to_string()))
    }).await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    let skills = hv.skills();
    let page = skills
        .list(models::environments::CatalogListQuery {
            search: Some("review".into()),
            after: Some("page".into()),
            limit: Some(1),
        })
        .await
        .unwrap();
    assert_eq!(page.next_cursor.as_deref(), Some("next"));
    let (request, _) = server.received.recv().await.unwrap();
    assert_eq!(request.uri.path(), "/skills/list");
    assert_eq!(request.uri.query().unwrap(), "search=review&after=page&limit=1");
    let detail = skills
        .get("review", &page.skills[0].version, &page.catalog)
        .await
        .unwrap();
    assert_eq!(detail.files["references/notes.md"], "notes\n");
    let (request, _) = server.received.recv().await.unwrap();
    assert_eq!(request.uri.path(), "/skills/review/info");
    assert_eq!(
        request.uri.query().unwrap(),
        "version=1.0.0%2Bpreview&snapshot=listed-snapshot"
    );
    let mut wrong = page.catalog;
    wrong.digest = format!("sha256:{}", "1".repeat(64));
    assert!(matches!(
        skills.get("review", "1.0.0+preview", &wrong).await,
        Err(Error::InvalidResponse(_))
    ));
}

#[tokio::test]
async fn global_rules_return_typed_definitions_from_the_listed_catalog() {
    let mut server = Server::respond(|parts| {
        let reference = "registry.example/rules:stable";
        let digest = format!("sha256:{}", "0".repeat(64));
        let metadata = json!({"id":"review","version":"1.0.0","name":"Review","status":"active",
            "publisher":"fixture","provenance":"fixture","license":"Apache-2.0","compatibility":{"min_runtime_contract":1}});
        let body = if parts.uri.path() == "/rules/list" {
            json!({"catalog":{"reference":reference,"digest":digest,"snapshot":"listed-snapshot"},
                "rules":[{"metadata":metadata,"kind":"detection"}]})
        } else {
            json!({"selection":{"id":"review","version":"1.0.0","catalog_reference":reference,"resolved_digest":digest},
                "metadata":metadata,"parameters":{"host":{"type":"text","default":"api.example","max_bytes":70}},
                "definition":{"kind":"detection","rule":{"name":"Review","match":"http.host == {{host}}","level":"high"}}})
        };
        Response::new(Body::from(body.to_string()))
    }).await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    let rules = hv.rules();
    let page = rules
        .list(models::environments::CatalogListQuery {
            search: Some("review".into()),
            after: Some("first".into()),
            limit: Some(12),
        })
        .await
        .unwrap();
    assert_eq!(page.rules[0].kind, models::environments::CatalogRuleKind::Detection);
    let (request, _) = server.received.recv().await.unwrap();
    assert_eq!(request.uri.path(), "/rules/list");
    assert_eq!(request.uri.query().unwrap(), "search=review&after=first&limit=12");
    let detail = rules.get("review", "1.0.0", &page.catalog).await.unwrap();
    let definition = serde_json::to_value(detail.definition).unwrap();
    assert_eq!(definition["rule"]["match"], "http.host == {{host}}");
    assert_eq!(
        serde_json::to_value(&detail.parameters["host"]).unwrap()["default"],
        "api.example"
    );
    let (request, _) = server.received.recv().await.unwrap();
    assert_eq!(request.uri.path(), "/rules/review/info");
    assert_eq!(request.uri.query().unwrap(), "version=1.0.0&snapshot=listed-snapshot");
    let mut wrong = page.catalog;
    wrong.reference = "registry.example/other:stable".into();
    assert!(matches!(
        rules.get("review", "1.0.0", &wrong).await,
        Err(Error::InvalidResponse(_))
    ));
}

#[tokio::test]
async fn global_mcps_return_published_requirements_from_the_listed_catalog() {
    let mut server = Server::respond(|parts| {
        let digest = format!("sha256:{}", "0".repeat(64));
        let reference = format!("registry.example/mcp@{digest}");
        let metadata = json!({"id":"review","version":"1.0.0","name":"Review","status":"active",
            "publisher":"fixture","provenance":"fixture","license":"Apache-2.0","compatibility":{"min_runtime_contract":1}});
        let body = if parts.uri.path() == "/mcps/list" {
            json!({"catalog":{"reference":reference,"digest":digest,"snapshot":"listed-snapshot"},
                "mcps":[{"metadata":metadata,"kind":"sandbox"}]})
        } else {
            json!({"selection":{"id":"review","version":"1.0.0","catalog_reference":reference,"resolved_digest":digest},
                "metadata":metadata,"definition":{"kind":"sandbox","executable":"review","args":["--stdio"],
                    "secret_variables":{"TOKEN":{"description":"Workspace key","required":true}}},
                "published_tools":[{"name":"review_file"}]})
        };
        Response::new(Body::from(body.to_string()))
    }).await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    let mcps = hv.mcps();
    let page = mcps
        .list(models::environments::CatalogListQuery {
            search: Some("review".into()),
            limit: Some(1),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page.mcps[0].kind, models::environments::McpCatalogKind::Sandbox);
    let (request, _) = server.received.recv().await.unwrap();
    assert_eq!(request.uri.path(), "/mcps/list");
    assert_eq!(request.uri.query().unwrap(), "search=review&limit=1");
    let detail = mcps.get("review", "1.0.0", &page.catalog).await.unwrap();
    let definition = serde_json::to_value(detail.definition).unwrap();
    assert_eq!(definition["secret_variables"]["TOKEN"]["required"], true);
    assert_eq!(detail.published_tools.unwrap()[0].name, "review_file");
    let (request, _) = server.received.recv().await.unwrap();
    assert_eq!(request.uri.path(), "/mcps/review/info");
    assert_eq!(request.uri.query().unwrap(), "version=1.0.0&snapshot=listed-snapshot");
    let mut wrong = page.catalog;
    wrong.digest = format!("sha256:{}", "1".repeat(64));
    assert!(matches!(
        mcps.get("review", "1.0.0", &wrong).await,
        Err(Error::InvalidResponse(_))
    ));
}
