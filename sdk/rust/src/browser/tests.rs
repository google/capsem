use super::*;

#[tokio::test]
async fn acknowledgement_sends_only_generation_and_revision_and_never_replays_conflict() {
    let mut server = crate::test_gateway::Server::reply(409, br#"{"error":"changed"}"#, None).await;
    let vm = VM::new(&server.url, "synthetic", crate::VmSelector::Id("vm-1".into())).unwrap();
    let request = crate::models::BrowserRequest {
        owner_generation: Some("18f59e33-8af6-4992-81d9-71e10873af9a".into()),
        id: 7,
        url: "https://example.test/?private=do-not-send".into(),
    };
    assert!(vm.acknowledge_browser_request(&request).await.is_err());
    let (parts, body) = next(&mut server).await;
    assert_eq!(parts.method, "POST");
    assert_eq!(parts.uri.path(), "/vms/vm-1/browser/acknowledge");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({"owner_generation":request.owner_generation,"request_id":7})
    );
    assert!(server.received.try_recv().is_err());
    let mut missing = request;
    missing.owner_generation = None;
    assert!(vm.acknowledge_browser_request(&missing).await.is_err());
    assert!(server.received.try_recv().is_err());
}

fn login(callback: &str, state: &str) -> String {
    let mut url = reqwest::Url::parse("https://login.example/authorize").unwrap();
    url.query_pairs_mut()
        .append_pair("redirect_uri", callback)
        .append_pair("state", state);
    url.to_string()
}

#[test]
fn callback_plan_requires_one_literal_loopback_redirect_and_state() {
    assert!(CallbackPlan::parse("https://example.com/docs").unwrap().is_none());
    for callback in [
        "https://localhost:45000/callback",
        "http://localhost:80/callback",
        "http://user@localhost:45000/callback",
        "http://localhost:45000/callback?x=1",
        "http://localhost:45000/callback#fragment",
    ] {
        assert!(
            CallbackPlan::parse(&login(callback, "synthetic-state-1234")).is_err(),
            "{callback}"
        );
    }
    assert!(CallbackPlan::parse(&login("http://localhost:45000/callback", "short")).is_err());
    let duplicate = format!(
        "{}&state=synthetic-state-1234",
        login("http://localhost:45000/callback", "synthetic-state-1234")
    );
    assert!(CallbackPlan::parse(&duplicate).is_err());
    assert!(
        CallbackPlan::parse(&login("http://[::1]:45000/callback", "synthetic-state-1234"))
            .unwrap()
            .is_some()
    );
    for external in ["https://app.example/callback", "http://127.0.0.2:45000/callback"] {
        assert!(CallbackPlan::parse(&login(external, "synthetic-state-1234"))
            .unwrap()
            .is_none());
    }
}

#[test]
fn callback_admission_checks_method_host_path_and_single_matching_state() {
    let plan = CallbackPlan::parse(&login("http://localhost:45000/oauth-callback", "synthetic-state-1234"))
        .unwrap()
        .unwrap();
    for host in ["localhost:45000", "127.0.0.1:45000", "[::1]:45000"] {
        let request = http::Request::builder()
            .uri("/oauth-callback?state=synthetic-state-1234&code=synthetic%2Fcode")
            .header("Host", host)
            .body(())
            .unwrap();
        assert!(plan.admits(&request));
    }
    for (method, host, uri) in [
        (
            "POST",
            "localhost:45000",
            "/oauth-callback?state=synthetic-state-1234&code=x",
        ),
        (
            "GET",
            "evil.test:45000",
            "/oauth-callback?state=synthetic-state-1234&code=x",
        ),
        (
            "GET",
            "localhost:45001",
            "/oauth-callback?state=synthetic-state-1234&code=x",
        ),
        ("GET", "localhost:45000", "/other?state=synthetic-state-1234&code=x"),
        ("GET", "localhost:45000", "/oauth-callback?state=wrong&code=x"),
        (
            "GET",
            "localhost:45000",
            "/oauth-callback?state=synthetic-state-1234&state=synthetic-state-1234&code=x",
        ),
        (
            "GET",
            "localhost:45000",
            "http://localhost:45000/oauth-callback?state=synthetic-state-1234&code=x",
        ),
        ("GET", "localhost:45000", "/oauth-callback?state=synthetic-state-1234"),
    ] {
        let request = http::Request::builder()
            .method(method)
            .uri(uri)
            .header("Host", host)
            .body(())
            .unwrap();
        assert!(!plan.admits(&request), "{method} {host} {uri}");
    }
    assert!(!format!("{plan:?}").contains("synthetic-state"));
}

fn free_port() -> u16 {
    capsem_foundation::unix::tcp::bind_loopback(std::net::Ipv4Addr::LOCALHOST.into())
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn gateway(preview_url: &str, callback_port: u16) -> (crate::test_gateway::Server, VM) {
    let preview_url = preview_url.replace("127.0.0.1", "callback.localhost");
    let server = crate::test_gateway::Server::respond(move |parts| {
        let body = if parts.method == "DELETE" {
            serde_json::json!({"success":true})
        } else if parts.uri.path().ends_with("preview-session") {
            serde_json::json!({"url":format!("{preview_url}/_capsem/bootstrap"),
                "bootstrap_token":"synthetic-bootstrap", "expires_in_seconds":600})
        } else {
            serde_json::json!({"id":"callback", "guest_port":callback_port,
                "host_port":null, "target":"container", "access":"http_preview"})
        };
        http::Response::builder()
            .header("Content-Type", "application/json")
            .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    })
    .await;
    let vm = VM::created(
        crate::client::Client::new(&server.url, "gateway-secret").unwrap(),
        "vm-1".into(),
        "test".into(),
        Some(true),
    )
    .unwrap();
    (server, vm)
}

async fn next(server: &mut crate::test_gateway::Server) -> (http::request::Parts, Vec<u8>) {
    tokio::time::timeout(Duration::from_secs(3), server.received.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn callback_reaches_only_scoped_preview_and_preserves_redirect_and_query() {
    let mut preview = crate::test_gateway::Server::respond(|parts| {
        if parts.uri.path() == "/_capsem/bootstrap" {
            http::Response::builder()
                .status(303)
                .header("Set-Cookie", "capsem_preview=synthetic-cookie; HttpOnly")
                .body(axum::body::Body::empty())
                .unwrap()
        } else {
            http::Response::builder()
                .status(302)
                .header("Location", "https://login.example/success")
                .body(axum::body::Body::from("complete"))
                .unwrap()
        }
    })
    .await;
    let port = free_port();
    let (mut gateway, vm) = gateway(&preview.url, port).await;
    let mut callback = BrowserCallback::prepare(
        &vm,
        &login(
            &format!("http://localhost:{port}/oauth-callback"),
            "synthetic-state-1234",
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let bootstrap = next(&mut preview).await;
    assert_eq!(bootstrap.1, b"bootstrap_token=synthetic-bootstrap");
    assert!(!bootstrap.0.headers.contains_key("Authorization"));
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let query = "state=synthetic-state-1234&code=synthetic%2Fcode&scope=email%20profile";
    let bad = http
        .get(format!("http://127.0.0.1:{port}/wrong?{query}"))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 403);
    let response = http
        .get(format!("http://[::1]:{port}/oauth-callback?{query}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 302);
    assert_eq!(response.headers()["Location"], "https://login.example/success");
    assert_eq!(response.bytes().await.unwrap(), "complete");
    let forwarded = next(&mut preview).await;
    assert_eq!(
        forwarded.0.uri.path_and_query().unwrap().as_str(),
        format!("/oauth-callback?{query}")
    );
    assert_eq!(forwarded.0.headers["Cookie"], "capsem_preview=synthetic-cookie");
    assert!(!forwarded.0.headers.contains_key("Authorization"));
    callback.task.take().unwrap().await.unwrap().unwrap();
    let create = next(&mut gateway).await;
    let admitted: crate::models::ExposureRequest = serde_json::from_slice(&create.1).unwrap();
    assert_eq!(admitted.access, crate::models::ExposureAccess::HttpPreview);
    assert_eq!(admitted.host_port, 0);
    assert_eq!(admitted.guest_port, port);
    next(&mut gateway).await; // Preview session mint.
    assert_eq!(next(&mut gateway).await.0.method, "DELETE");
    assert!(http
        .get(format!("http://127.0.0.1:{port}/oauth-callback?{query}"))
        .send()
        .await
        .is_err());
}

#[tokio::test]
async fn callback_drop_and_failed_bootstrap_revoke_their_exposures() {
    for succeeds in [false, true] {
        let preview = crate::test_gateway::Server::respond(move |_| {
            http::Response::builder()
                .status(if succeeds { 303 } else { 403 })
                .header("Set-Cookie", "capsem_preview=synthetic-cookie")
                .body(axum::body::Body::empty())
                .unwrap()
        })
        .await;
        let port = free_port();
        let (mut gateway, vm) = gateway(&preview.url, port).await;
        let result = BrowserCallback::prepare(
            &vm,
            &login(&format!("http://localhost:{port}/callback"), "synthetic-state-1234"),
        )
        .await;
        assert_eq!(result.is_ok(), succeeds);
        if let Ok(Some(mut callback)) = result {
            let task = callback.task.take().unwrap();
            drop(callback);
            task.await.unwrap().unwrap();
        }
        next(&mut gateway).await;
        next(&mut gateway).await;
        assert_eq!(next(&mut gateway).await.0.method, "DELETE");
        assert!(CallbackPlan::parse(&login(
            &format!("http://localhost:{port}/callback"),
            "synthetic-state-1234"
        ))
        .unwrap()
        .unwrap()
        .bind()
        .is_ok());
    }
}

#[tokio::test]
async fn occupied_loopback_and_remote_gateway_fail_before_any_admission() {
    let listener = capsem_foundation::unix::tcp::bind_loopback(std::net::Ipv4Addr::LOCALHOST.into()).unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = login(&format!("http://localhost:{port}/callback"), "synthetic-state-1234");
    let vm = VM::new("http://127.0.0.1:1", "secret", crate::VmSelector::Id("vm-1".into())).unwrap();
    assert!(matches!(
        BrowserCallback::prepare(&vm, &url).await,
        Err(Error::InvalidInput(_))
    ));
    let remote = VM::new("https://remote.example", "secret", crate::VmSelector::Id("vm-1".into())).unwrap();
    assert!(matches!(
        BrowserCallback::prepare(&remote, &url).await,
        Err(Error::InvalidInput(_))
    ));
    assert!(BrowserCallback::prepare(&remote, "https://example.com/docs")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn other_address_family_cannot_replay_or_interrupt_an_accepted_callback() {
    let started = std::sync::Arc::new(tokio::sync::Notify::new());
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    let notify_started = started.clone();
    let notify_release = release.clone();
    let preview = crate::test_gateway::Server::respond(move |parts| {
        if parts.uri.path() == "/_capsem/bootstrap" {
            return http::Response::builder()
                .status(303)
                .header("Set-Cookie", "capsem_preview=synthetic-cookie")
                .body(axum::body::Body::empty())
                .unwrap();
        }
        notify_started.notify_one();
        let release = notify_release.clone();
        http::Response::builder()
            .body(axum::body::Body::from_stream(futures::stream::once(async move {
                release.notified().await;
                Ok::<_, std::convert::Infallible>(bytes::Bytes::from_static(b"complete"))
            })))
            .unwrap()
    })
    .await;
    let port = free_port();
    let (_gateway, vm) = gateway(&preview.url, port).await;
    let mut callback = BrowserCallback::prepare(
        &vm,
        &login(&format!("http://localhost:{port}/callback"), "synthetic-state-1234"),
    )
    .await
    .unwrap()
    .unwrap();
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let accepted_http = http.clone();
    let accepted = tokio::spawn(async move {
        accepted_http
            .get(format!(
                "http://[::1]:{port}/callback?state=synthetic-state-1234&code=synthetic"
            ))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap()
    });
    tokio::time::timeout(Duration::from_secs(3), started.notified())
        .await
        .unwrap();
    let replay = http
        .get(format!(
            "http://127.0.0.1:{port}/callback?state=synthetic-state-1234&code=synthetic"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status(), 410);
    assert!(!accepted.is_finished());
    release.notify_one();
    assert_eq!(accepted.await.unwrap(), "complete");
    callback.task.take().unwrap().await.unwrap().unwrap();
}

#[tokio::test]
async fn oversized_callback_body_and_unsafe_redirect_are_refused() {
    for location in [None, Some("javascript:alert(1)")] {
        let preview = crate::test_gateway::Server::respond(move |parts| {
            let mut response = http::Response::builder();
            if parts.uri.path() == "/_capsem/bootstrap" {
                return response
                    .status(303)
                    .header("Set-Cookie", "capsem_preview=synthetic-cookie")
                    .body(axum::body::Body::empty())
                    .unwrap();
            }
            if let Some(location) = location {
                response = response.status(302).header("Location", location);
            }
            response.body(axum::body::Body::from(vec![b'x'; 65537])).unwrap()
        })
        .await;
        let port = free_port();
        let (_gateway, vm) = gateway(&preview.url, port).await;
        let mut callback = BrowserCallback::prepare(
            &vm,
            &login(&format!("http://localhost:{port}/callback"), "synthetic-state-1234"),
        )
        .await
        .unwrap()
        .unwrap();
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!(
                "http://127.0.0.1:{port}/callback?state=synthetic-state-1234&code=synthetic"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 502);
        assert_eq!(
            response.text().await.unwrap(),
            "Could not return sign-in to the application"
        );
        callback.task.take().unwrap().await.unwrap().unwrap();
    }
}
