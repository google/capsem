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
async fn plugin_defaults_and_saved_results_require_their_exact_revision_context() {
    let mut server = Server::respond(|parts| {
        let revision = if parts.uri.path() == "/environments/plugins/list" {
            serde_json::Value::Null
        } else {
            json!({"environment_id": ENVIRONMENT, "revision_id": REQUEST})
        };
        Response::new(Body::from(json!({"revision": revision, "plugins": []}).to_string()))
    }).await;
    let hypervisor = Hypervisor::new(&server.url, "private-token").unwrap();
    let environments = hypervisor.environments();
    assert!(environments.plugins(None).await.unwrap().revision.is_none());
    let reference = EnvironmentRevisionReference { environment_id: ENVIRONMENT.into(), revision_id: REQUEST.into() };
    assert_eq!(environments.plugins(Some(&reference)).await.unwrap().revision, Some(reference));
    let stale = EnvironmentRevisionReference { environment_id: ENVIRONMENT.into(), revision_id: ENVIRONMENT.into() };
    assert!(matches!(environments.plugins(Some(&stale)).await, Err(Error::InvalidResponse(_))));

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
    assert_eq!(hypervisor.environments().archive(ENVIRONMENT, request).await.unwrap().request_id, REQUEST);
    let (parts, body) = server.received.recv().await.unwrap();
    assert_eq!(parts.method, "POST");
    assert_eq!(parts.uri.path(), format!("/environments/{ENVIRONMENT}/changes"));
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&body).unwrap(), json!({
        "action": "archive", "request_id": REQUEST, "base_revision_id": base, "reason": "retire blueprint",
    }));
}
