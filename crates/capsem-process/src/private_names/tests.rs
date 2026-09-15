use super::*;
use std::sync::{Arc, Mutex};

/// The service's resolve route as a fake: answers from a table, records
/// what it was asked.
async fn fake_service(socket: PathBuf, seen: Arc<Mutex<Vec<serde_json::Value>>>) {
    use axum::{routing::post, Router};
    let app = Router::new().route(
        "/networks/private/resolve",
        post(move |axum::Json(body): axum::Json<serde_json::Value>| {
            let seen = Arc::clone(&seen);
            async move {
                seen.lock().unwrap().push(body.clone());
                if body["owner_secret"] != "secret-a" {
                    return (axum::http::StatusCode::FORBIDDEN, axum::Json(serde_json::json!({})));
                }
                match (body["name"].as_str(), body["address"].as_str()) {
                    (Some("beta.team"), None) => (
                        axum::http::StatusCode::OK,
                        axum::Json(serde_json::json!({
                            "name": "beta.team.capsem.internal", "address": "10.128.0.3", "vm": "vm-b", "network": "team"
                        })),
                    ),
                    (None, Some("10.128.0.3")) => (
                        axum::http::StatusCode::OK,
                        axum::Json(serde_json::json!({
                            "name": "beta.team.capsem.internal", "address": "10.128.0.3", "vm": "vm-b", "network": "team"
                        })),
                    ),
                    _ => (axum::http::StatusCode::NOT_FOUND, axum::Json(serde_json::json!({}))),
                }
            }
        }),
    );
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
}

#[tokio::test]
async fn names_and_addresses_are_the_services_answers_and_misses_are_none() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("service.sock");
    let seen = Arc::new(Mutex::new(Vec::new()));
    fake_service(socket.clone(), Arc::clone(&seen)).await;
    let names = ServicePrivateNames::new(socket, "secret-a".into(), "vm-a".into());
    assert_eq!(names.address_of("beta.team").await, Some(Ipv4Addr::new(10, 128, 0, 3)));
    assert_eq!(names.address_of("nobody.team").await, None);
    assert_eq!(
        names.name_of(Ipv4Addr::new(10, 128, 0, 3)).await.as_deref(),
        Some("beta.team.capsem.internal")
    );
    assert_eq!(names.name_of(Ipv4Addr::new(10, 128, 0, 9)).await, None);
    let asked = seen.lock().unwrap().clone();
    assert_eq!(asked.len(), 4);
    assert!(asked
        .iter()
        .all(|ask| ask["source_vm"] == "vm-a" && ask["owner_secret"] == "secret-a"));
    assert_eq!(asked[0]["name"], "beta.team");
    assert_eq!(asked[2]["address"], "10.128.0.3");
}

#[tokio::test]
async fn a_refusal_or_an_absent_service_is_a_miss_not_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("service.sock");
    fake_service(socket.clone(), Arc::new(Mutex::new(Vec::new()))).await;
    let wrong = ServicePrivateNames::new(socket, "wrong".into(), "vm-a".into());
    assert_eq!(wrong.address_of("beta.team").await, None);
    let absent = ServicePrivateNames::new(dir.path().join("nobody.sock"), "secret-a".into(), "vm-a".into());
    assert_eq!(absent.address_of("beta.team").await, None);
}
