use super::*;
use crate::instance::WorkerAuthority;
use capsem_core::net::policy_config::{ActivePolicyFile, SettingsFile};
use capsem_mcp_transport::http_transport::BrokerHttpClient;
use capsem_proto::mcp::{McpAuthConfig, McpAuthKind};
use std::collections::HashMap;
use std::sync::Mutex;

struct Upstream {
    url: String,
    requests: Arc<Mutex<Vec<(Method, HeaderMap)>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn upstream() -> Upstream {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let router = axum::Router::new().route(
        "/mcp",
        axum::routing::any(move |request: Request<Body>| {
            let captured = captured.clone();
            async move {
                captured
                    .lock()
                    .unwrap()
                    .push((request.method().clone(), request.headers().clone()));
                let mut response = Response::new(Body::from("upstream-response"));
                response.headers_mut().insert(
                    MCP_ADMISSION_HEADER,
                    HeaderValue::from_static("forged upstream admission"),
                );
                response
            }
        }),
    );
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Upstream { url, requests, task }
}

fn configured_policy(url: &str, credential_ref: Option<String>) -> Arc<BrokerPolicy> {
    let mut active =
        ActivePolicyFile::from_settings_and_corp(&SettingsFile::default(), &SettingsFile::default()).unwrap();
    active.mcp = Some(capsem_config::mcp::McpConfig {
        servers: vec![capsem_config::mcp::McpManualServer {
            name: "assigned".into(),
            url: url.into(),
            headers: HashMap::new(),
            enabled: true,
            auth: credential_ref.map(|credential_ref| McpAuthConfig {
                kind: McpAuthKind::Bearer,
                credential_ref,
            }),
        }],
        ..Default::default()
    });
    BrokerPolicy::new("test-policy".into(), Arc::new(active.compile_runtime().unwrap()))
}

async fn capability(
    policy: Arc<BrokerPolicy>,
    role: McpHttpRole,
) -> (
    BrokerHttpClient,
    watch::Sender<Arc<BrokerPolicy>>,
    WorkerAuthority,
    tokio::task::JoinHandle<()>,
) {
    let (coordinator, worker) = UnixStream::pair().unwrap();
    let (publisher, updates) = watch::channel(policy);
    let authority = WorkerAuthority::default();
    let task = tokio::spawn(serve(coordinator, role, updates, authority.grant()));
    (
        BrokerHttpClient::connect(worker, HTTP_TIMEOUT).await.unwrap(),
        publisher,
        authority,
        task,
    )
}

fn remote_request(method: Method, url: &str) -> reqwest::Request {
    reqwest::Client::new()
        .request(method, url)
        .header(MCP_SERVER_HEADER, "assigned")
        .build()
        .unwrap()
}

#[tokio::test]
async fn configured_capability_owns_destination_and_credentials_and_retires_independently() {
    let _lock = crate::tests::SETTINGS_ENV_LOCK.lock().await;
    let first = upstream().await;
    let second = upstream().await;
    let store = capsem_credentials::CredentialStore::global();
    store.clear_for_test();
    let first_ref = store
        .inject(
            capsem_credentials::CredentialProvider::Mcp,
            "native-mcp-broker-first",
            capsem_credentials::CredentialPersistence::Memory,
        )
        .unwrap();
    let second_ref = store
        .inject(
            capsem_credentials::CredentialProvider::Mcp,
            "native-mcp-broker-second",
            capsem_credentials::CredentialPersistence::Memory,
        )
        .unwrap();
    let (client, _publisher, authority, task) =
        capability(configured_policy(&first.url, Some(first_ref)), McpHttpRole::Aggregator).await;
    let (other, _other_publisher, other_authority, other_task) = capability(
        configured_policy(&second.url, Some(second_ref)),
        McpHttpRole::Aggregator,
    )
    .await;
    let mut request = remote_request(Method::POST, &first.url);
    request.headers_mut().insert(
        http::header::AUTHORIZATION,
        HeaderValue::from_static("Bearer untrusted-worker-token"),
    );
    let response = client.execute(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!response.headers().contains_key(MCP_ADMISSION_HEADER));
    assert_eq!(response.text().await.unwrap(), "upstream-response");
    assert_eq!(
        first.requests.lock().unwrap()[0].1[http::header::AUTHORIZATION],
        "Bearer native-mcp-broker-first"
    );
    for (method, url) in [
        (Method::POST, second.url.clone()),
        (Method::POST, format!("{}?unapproved=true", first.url)),
        (Method::PUT, first.url.clone()),
    ] {
        assert_eq!(
            client.execute(remote_request(method, &url)).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(first.requests.lock().unwrap().len(), 1);
    assert!(second.requests.lock().unwrap().is_empty());
    authority.revoke();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert!(client.execute(remote_request(Method::POST, &first.url)).await.is_err());
    assert_eq!(
        other
            .execute(remote_request(Method::POST, &second.url))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        second.requests.lock().unwrap()[0].1[http::header::AUTHORIZATION],
        "Bearer native-mcp-broker-second"
    );
    other_authority.revoke();
    other_task.await.unwrap();
    store.clear_for_test();
}

#[tokio::test]
async fn builtin_admission_uses_current_policy_and_replaces_upstream_metadata() {
    let upstream = upstream().await;
    let mut active =
        ActivePolicyFile::from_settings_and_corp(&SettingsFile::default(), &SettingsFile::default()).unwrap();
    active.user_rules = capsem_core::net::policy_config::SecurityRuleProfile::parse_toml(
        r#"
        [profiles.rules.allow_local]
        name = "allow_local"
        action = "allow"
        match = 'http.host == "127.0.0.1"'
    "#,
    )
    .unwrap();
    let allowed = BrokerPolicy::new("allowed".into(), Arc::new(active.compile_runtime().unwrap()));
    let (client, publisher, authority, task) = capability(allowed.clone(), McpHttpRole::Builtin).await;
    let response = client
        .execute(reqwest::Client::new().get(&upstream.url).build().unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(matches!(
        serde_json::from_slice::<McpHttpAdmission>(response.headers()[MCP_ADMISSION_HEADER].as_bytes()).unwrap(),
        McpHttpAdmission::Allowed { rule_id: Some(_), .. }
    ));
    let blocked = configured_policy(&upstream.url, None);
    publisher.send_replace(blocked);
    let response = client
        .execute(reqwest::Client::new().get(&upstream.url).build().unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(matches!(
        serde_json::from_slice::<McpHttpAdmission>(response.headers()[MCP_ADMISSION_HEADER].as_bytes()).unwrap(),
        McpHttpAdmission::Denied { .. }
    ));
    assert_eq!(upstream.requests.lock().unwrap().len(), 1);
    publisher.send_replace(allowed);
    assert_eq!(
        client
            .execute(reqwest::Client::new().get(&upstream.url).build().unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    authority.revoke();
    task.await.unwrap();
}

#[tokio::test]
async fn event_streams_remain_concurrent_and_cancel_on_drop_policy_change_and_retirement() {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let (started, mut streams) = tokio::sync::mpsc::channel(4);
    let router = axum::Router::new().route(
        "/mcp",
        axum::routing::any(move |request: Request<Body>| {
            let started = started.clone();
            async move {
                if request.method() == Method::POST {
                    return Response::new(Body::from("concurrent-call"));
                }
                let (send, receive) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(2);
                send.send(Ok(bytes::Bytes::from_static(b"data: ready\n\n")))
                    .await
                    .unwrap();
                started.send(send).await.unwrap();
                let stream = futures::stream::unfold(receive, |mut receive| async {
                    receive.recv().await.map(|chunk| (chunk, receive))
                });
                Response::builder()
                    .header(http::header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from_stream(stream))
                    .unwrap()
            }
        }),
    );
    let upstream_task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let policy = configured_policy(&url, None);
    let (client, publisher, authority, task) = capability(policy.clone(), McpHttpRole::Aggregator).await;
    for cancellation in ["drop", "policy", "retirement"] {
        let mut events = client.execute(remote_request(Method::GET, &url)).await.unwrap();
        let upstream = tokio::time::timeout(Duration::from_secs(2), streams.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(events.chunk().await.unwrap().unwrap(), "data: ready\n\n");
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            client.execute(remote_request(Method::POST, &url)),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response.text().await.unwrap(), "concurrent-call");
        match cancellation {
            "drop" => drop(events),
            "policy" => {
                publisher.send_replace(policy.clone());
                assert!(tokio::time::timeout(Duration::from_secs(2), events.chunk())
                    .await
                    .unwrap()
                    .is_err());
            }
            "retirement" => {
                authority.revoke();
                assert!(tokio::time::timeout(Duration::from_secs(2), events.chunk())
                    .await
                    .unwrap()
                    .is_err());
            }
            _ => unreachable!(),
        }
        tokio::time::timeout(Duration::from_secs(2), upstream.closed())
            .await
            .unwrap();
    }
    task.await.unwrap();
    upstream_task.abort();
    let _ = upstream_task.await;
}

#[tokio::test]
async fn broker_bounds_request_bodies_even_when_the_worker_skips_client_validation() {
    let upstream = upstream().await;
    let (coordinator, worker) = UnixStream::pair().unwrap();
    let (_publisher, updates) = watch::channel(configured_policy(&upstream.url, None));
    let authority = WorkerAuthority::default();
    let task = tokio::spawn(serve(coordinator, McpHttpRole::Aggregator, updates, authority.grant()));
    fd::set_nonblocking(worker.as_fd(), true).unwrap();
    let worker = tokio::net::UnixStream::from_std(worker).unwrap();
    let (mut sender, connection) = hyper::client::conn::http2::handshake::<_, _, http_body_util::Full<bytes::Bytes>>(
        TokioExecutor::new(),
        TokioIo::new(worker),
    )
    .await
    .unwrap();
    let driver = tokio::spawn(connection);
    let request = Request::builder()
        .method(Method::POST)
        .uri(&upstream.url)
        .header(MCP_SERVER_HEADER, "assigned")
        .body(http_body_util::Full::new(bytes::Bytes::from(vec![
            0;
            capsem_proto::MCP_FRAME_MAX_SIZE
                + 1
        ])))
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(2), sender.send_request(request))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(upstream.requests.lock().unwrap().is_empty());
    authority.revoke();
    task.await.unwrap();
    driver.abort();
    let _ = driver.await;
}
