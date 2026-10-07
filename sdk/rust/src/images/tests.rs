use axum::body::Body;
use axum::http::Response;
use serde_json::json;
use std::time::Duration;

use crate::models::ImageCacheState;
use crate::test_gateway::Server;
use crate::{Error, Hypervisor, Registry};

#[tokio::test]
async fn image_catalog_and_pull_use_authenticated_http_and_per_call_registry() {
    let pin = format!("registry.example/code@sha256:{}", "a".repeat(64));
    let expected_pin = pin.clone();
    let mut server = Server::respond(move |parts| {
        let body =
            if parts.uri.path() == "/images" {
                let images = ["unknown", "missing", "partial", "ready"].map(|cached| json!({
                "name": "code", "description": "Tools", "architectures": ["amd64"], "image": pin, "cached": cached
            }));
                json!({"images": images})
            } else {
                json!({"image": "code", "resolved": pin, "digest": format!("sha256:{}", "b".repeat(64))})
            };
        Response::new(Body::from(body.to_string()))
    })
    .await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    let images = hv.images();
    let catalog = images.list(true).await.unwrap();
    assert_eq!(
        catalog.images.iter().map(|image| image.cached).collect::<Vec<_>>(),
        [
            ImageCacheState::Unknown,
            ImageCacheState::Missing,
            ImageCacheState::Partial,
            ImageCacheState::Ready
        ]
    );
    assert_eq!(catalog.images[0].image.as_deref(), Some(expected_pin.as_str()));
    let registry = Registry {
        username: Some("robot".into()),
        password: Some("registry-secret".into()),
        ..Default::default()
    };
    assert!(!format!("{registry:?}").contains("registry-secret"));
    assert_eq!(
        images.pull("code", Some(registry)).await.unwrap().resolved,
        expected_pin
    );
    images.pull("code", None).await.unwrap();
    images.list(false).await.unwrap();
    for (method, path, body) in [
        ("GET", "/images?refresh=true", None),
        (
            "POST",
            "/images/pull",
            Some(json!({"image": "code", "registry": {"username": "robot", "password": "registry-secret"}})),
        ),
        ("POST", "/images/pull", Some(json!({"image": "code"}))),
        ("GET", "/images?refresh=false", None),
    ] {
        let (parts, bytes) = server.received.recv().await.unwrap();
        assert_eq!(parts.method.as_str(), method);
        assert_eq!(parts.uri.to_string(), path);
        assert_eq!(parts.headers["authorization"], "Bearer private-token");
        if let Some(body) = body {
            assert_eq!(serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(), body);
        } else {
            assert!(bytes.is_empty());
        }
    }
    assert!(server.received.try_recv().is_err());
}

#[tokio::test]
async fn image_input_and_http_refusal_never_replay_a_pull() {
    let mut server = Server::reply(403, b"image denied", None).await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    assert!(matches!(hv.images().pull("", None).await, Err(Error::InvalidInput(_))));
    assert!(server.received.try_recv().is_err());
    assert!(matches!(
        hv.images().pull("denied", None).await,
        Err(Error::Http {status: 403, body}) if body == b"image denied"
    ));
    assert!(matches!(
        hv.images().list(false).await,
        Err(Error::Http { status: 403, .. })
    ));
    for path in ["/images/pull", "/images"] {
        let (parts, _) = server.received.recv().await.unwrap();
        assert_eq!(parts.uri.path(), path);
    }
    assert!(server.received.try_recv().is_err());
}

#[tokio::test]
async fn image_catalog_rejects_unknown_cache_states() {
    let mut server = Server::reply(
        200,
        br#"{"images":[{"name":"code","description":"Tools","architectures":["amd64"],"cached":"invented"}]}"#,
        None,
    )
    .await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    assert!(matches!(hv.images().list(false).await, Err(Error::Json(_))));
    server.received.recv().await.unwrap();
    assert!(server.received.try_recv().is_err());
}

#[tokio::test]
async fn image_requests_respect_the_parent_deadline_and_dropped_future() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hv = Hypervisor::new(&format!("http://{}", listener.local_addr().unwrap()), "private-token")
        .unwrap()
        .with_timeout(Duration::from_millis(50))
        .unwrap();
    assert!(matches!(hv.images().list(false).await, Err(Error::Transport(error)) if error.is_timeout()));
    let images = hv.images();
    assert!(
        tokio::time::timeout(Duration::from_millis(10), images.pull("code", None))
            .await
            .is_err()
    );
}
