use super::fd_stream::{set_nonblocking, AsyncFdStream, ReplayReader};
use super::util::{format_headers, is_llm_api_path};
use super::*;
use std::collections::BTreeMap;
use std::os::unix::io::IntoRawFd;
use std::os::unix::net::UnixStream;

use http_body_util::BodyExt;

use crate::net::cert_authority::CertAuthority;
use capsem_security_engine::{
    CelEnforcementEvaluator, CelEnforcementRule, SecurityDecisionAction, SecurityEngine,
};

const CA_KEY: &str = include_str!("../../../../../config/capsem-ca.key");
const CA_CERT: &str = include_str!("../../../../../config/capsem-ca.crt");

/// Flush delay for the DB writer thread to process queued writes.
const DB_FLUSH_MS: u64 = 100;

/// Non-routable domain for tests that go through the full proxy pipeline.
/// Must never resolve so allowed requests always hit the 502 upstream-error
/// path instead of reaching a real server.
const TEST_DOMAIN: &str = "thisdomaindoesnotexistforsur3.ai";

fn make_config_dev() -> Arc<MitmProxyConfig> {
    make_config_dev_with_security_engine(None)
}

fn make_config_dev_with_security_engine(
    security_engine: Option<Arc<dyn RuntimeSecurityEngine>>,
) -> Arc<MitmProxyConfig> {
    let ca = Arc::new(CertAuthority::load(CA_KEY, CA_CERT).unwrap());
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(DbWriter::open(&dir.path().join("test.db"), 256).unwrap());
    // Leak the tempdir so it lives for the test
    std::mem::forget(dir);
    let telemetry = Arc::new(super::telemetry_hook::TelemetryDeps {
        db: Arc::clone(&db),
        pricing: Arc::new(crate::net::ai_traffic::pricing::PricingTable::load()),
        trace_state: Arc::new(std::sync::Mutex::new(
            crate::net::ai_traffic::TraceState::new(),
        )),
    });
    let pipeline = super::make_production_pipeline(Arc::clone(&telemetry));
    Arc::new(MitmProxyConfig {
        ca,
        db,
        upstream_tls: make_upstream_tls_config(),
        telemetry,
        pipeline,
        mcp_endpoint: None,
        security_engine: Arc::new(RuntimeSecurityEngineSlot::new(security_engine)),
    })
}

#[test]
fn runtime_security_engine_slot_swaps_rules_without_rebuilding_config() {
    let slot = RuntimeSecurityEngineSlot::new(Some(block_host_engine("initial.test")));

    let blocked = slot
        .evaluate(test_http_security_event("initial.test", "/"))
        .expect("initial runtime engine should evaluate");
    assert!(matches!(
        blocked.action,
        capsem_security_engine::SecurityAction::Block(_)
    ));

    let allowed = slot
        .evaluate(test_http_security_event("updated.test", "/"))
        .expect("non-matching host should be allowed");
    assert!(matches!(
        allowed.action,
        capsem_security_engine::SecurityAction::Continue
    ));

    slot.set(Some(block_host_engine("updated.test")));

    let previously_blocked = slot
        .evaluate(test_http_security_event("initial.test", "/"))
        .expect("swapped runtime engine should evaluate");
    assert!(matches!(
        previously_blocked.action,
        capsem_security_engine::SecurityAction::Continue
    ));

    let newly_blocked = slot
        .evaluate(test_http_security_event("updated.test", "/"))
        .expect("updated runtime engine should evaluate");
    assert!(matches!(
        newly_blocked.action,
        capsem_security_engine::SecurityAction::Block(_)
    ));

    slot.set(None);
    assert!(!slot.has_engine());
}

fn block_host_engine(host: &str) -> Arc<dyn RuntimeSecurityEngine> {
    let mut engine = SecurityEngine::default();
    engine.set_enforcement(Box::new(
        CelEnforcementEvaluator::compile(vec![CelEnforcementRule {
            id: format!("block-{host}"),
            pack_id: Some("test".into()),
            condition: format!("http.request.host == '{host}'"),
            decision: SecurityDecisionAction::Block,
            reason: Some(format!("block {host}")),
        }])
        .expect("test CEL rule should compile"),
    ));
    Arc::new(std::sync::Mutex::new(engine))
}

fn test_http_security_event(host: &str, path: &str) -> capsem_security_engine::SecurityEvent {
    capsem_security_engine::SecurityEvent::http(
        capsem_security_engine::SecurityEventCommon {
            event_id: format!("test-http-{host}-{path}"),
            parent_event_id: None,
            stream_id: None,
            activity_id: None,
            sequence_no: None,
            source_engine: capsem_security_engine::SourceEngine::Network,
            attribution_scope: capsem_security_engine::AiAttributionScope::Vm,
            origin_kind: capsem_security_engine::AiOriginKind::GuestNetwork,
            accounting_owner: None,
            enforceability: capsem_security_engine::Enforceability::InlineBlockable,
            trace_id: Some("trace-test".into()),
            span_id: None,
            timestamp_unix_ms: 1,
            vm_id: None,
            session_id: None,
            profile_id: None,
            profile_revision: None,
            profile_pack_ids: Vec::new(),
            enforcement_packs: Vec::new(),
            detection_packs: Vec::new(),
            user_id: None,
            process_id: None,
            parent_process_id: None,
            exec_id: None,
            turn_id: None,
            message_id: None,
            tool_call_id: None,
            mcp_call_id: None,
            event_type: "http.request".into(),
            redaction_state: capsem_security_engine::RedactionState::Raw,
        },
        capsem_security_engine::HttpSecuritySubject {
            method: "GET".into(),
            scheme: Some("https".into()),
            host: host.into(),
            port: Some(443),
            path: Some(path.into()),
            query: None,
            url: Some(format!("https://{host}{path}")),
            path_class: "external".into(),
            request_bytes: 0,
            request_headers: BTreeMap::new(),
            request_body: None,
            response_status: None,
            response_headers: BTreeMap::new(),
            response_bytes: None,
            response_body: None,
        },
    )
}

fn make_client_hello(hostname: &str) -> Vec<u8> {
    let hostname_bytes = hostname.as_bytes();
    let sni_entry_len = 1 + 2 + hostname_bytes.len();
    let sni_list_len = sni_entry_len;
    let sni_ext_data_len = 2 + sni_list_len;

    let mut sni_ext = Vec::new();
    sni_ext.extend_from_slice(&0x0000u16.to_be_bytes());
    sni_ext.extend_from_slice(&(sni_ext_data_len as u16).to_be_bytes());
    sni_ext.extend_from_slice(&(sni_list_len as u16).to_be_bytes());
    sni_ext.push(0x00);
    sni_ext.extend_from_slice(&(hostname_bytes.len() as u16).to_be_bytes());
    sni_ext.extend_from_slice(hostname_bytes);

    let extensions_len = sni_ext.len();
    let mut hello_body = Vec::new();
    hello_body.extend_from_slice(&[0x03, 0x03]);
    hello_body.extend_from_slice(&[0u8; 32]);
    hello_body.push(0);
    hello_body.extend_from_slice(&2u16.to_be_bytes());
    hello_body.extend_from_slice(&[0x00, 0x2f]);
    hello_body.push(1);
    hello_body.push(0);
    hello_body.extend_from_slice(&(extensions_len as u16).to_be_bytes());
    hello_body.extend_from_slice(&sni_ext);

    let mut handshake = Vec::new();
    handshake.push(0x01);
    let hello_len = hello_body.len();
    handshake.push((hello_len >> 16) as u8);
    handshake.push((hello_len >> 8) as u8);
    handshake.push(hello_len as u8);
    handshake.extend_from_slice(&hello_body);

    let mut record = Vec::new();
    record.push(0x16);
    record.extend_from_slice(&[0x03, 0x01]);
    record.extend_from_slice(&(handshake.len() as u16).to_be_bytes());
    record.extend_from_slice(&handshake);

    record
}

#[test]
fn split_path_query_with_query() {
    let uri: hyper::Uri = format!("https://{TEST_DOMAIN}/api/v1?foo=bar&baz=1")
        .parse()
        .unwrap();
    let (path, query) = split_path_query(&uri);
    assert_eq!(path, "/api/v1");
    assert_eq!(query, Some("foo=bar&baz=1".to_string()));
}

#[test]
fn split_path_query_without_query() {
    let uri: hyper::Uri = "/about".parse().unwrap();
    let (path, query) = split_path_query(&uri);
    assert_eq!(path, "/about");
    assert_eq!(query, None);
}

// ---------------------------------------------------------------
// Header sanitization tests
// ---------------------------------------------------------------

#[test]
fn format_headers_keeps_allowlisted_verbatim() {
    let mut headers = hyper::HeaderMap::new();
    headers.insert("content-type", "application/json".parse().unwrap());
    headers.insert("content-length", "42".parse().unwrap());
    headers.insert("host", format!("api.{TEST_DOMAIN}").parse().unwrap());
    headers.insert("server", "nginx".parse().unwrap());
    headers.insert("user-agent", "curl/8.0".parse().unwrap());

    let formatted = format_headers(&headers);
    assert!(formatted.contains("content-type: application/json"));
    assert!(formatted.contains("content-length: 42"));
    assert!(formatted.contains(&format!("host: api.{TEST_DOMAIN}")));
    assert!(formatted.contains("server: nginx"));
    assert!(formatted.contains("user-agent: curl/8.0"));
}

#[test]
fn format_headers_hashes_sensitive_headers() {
    let mut headers = hyper::HeaderMap::new();
    headers.insert("x-api-key", "sk-ant-1234567890abcdef".parse().unwrap());
    headers.insert("authorization", "Bearer tok_secret".parse().unwrap());
    headers.insert("cookie", "session=abc123".parse().unwrap());

    let formatted = format_headers(&headers);

    // Header names are preserved.
    assert!(formatted.contains("x-api-key: hash:"));
    assert!(formatted.contains("authorization: hash:"));
    assert!(formatted.contains("cookie: hash:"));

    // Raw credential values must NOT appear.
    assert!(!formatted.contains("sk-ant-1234567890abcdef"));
    assert!(!formatted.contains("Bearer tok_secret"));
    assert!(!formatted.contains("session=abc123"));
}

#[test]
fn format_headers_hash_is_deterministic() {
    let mut h1 = hyper::HeaderMap::new();
    h1.insert("x-api-key", "AIzaSyBxxxxxxx".parse().unwrap());
    let mut h2 = hyper::HeaderMap::new();
    h2.insert("x-api-key", "AIzaSyBxxxxxxx".parse().unwrap());

    assert_eq!(format_headers(&h1), format_headers(&h2));
}

#[test]
fn format_headers_different_keys_different_hashes() {
    let mut h1 = hyper::HeaderMap::new();
    h1.insert("x-api-key", "key-AAAA".parse().unwrap());
    let mut h2 = hyper::HeaderMap::new();
    h2.insert("x-api-key", "key-BBBB".parse().unwrap());

    // Extract the hash portion from each.
    let f1 = format_headers(&h1);
    let f2 = format_headers(&h2);
    let hash1 = f1.strip_prefix("x-api-key: hash:").unwrap();
    let hash2 = f2.strip_prefix("x-api-key: hash:").unwrap();
    assert_ne!(hash1, hash2);
}

#[test]
fn format_headers_mixed_allowed_and_sensitive() {
    let mut headers = hyper::HeaderMap::new();
    headers.insert("content-type", "text/html".parse().unwrap());
    headers.insert("x-api-key", "sk-secret".parse().unwrap());
    headers.insert("accept", "text/html".parse().unwrap());

    let formatted = format_headers(&headers);

    // Allowlisted: verbatim.
    assert!(formatted.contains("content-type: text/html"));
    assert!(formatted.contains("accept: text/html"));

    // Sensitive: hashed, raw value absent.
    assert!(formatted.contains("x-api-key: hash:"));
    assert!(!formatted.contains("sk-secret"));
}

#[test]
fn format_headers_empty() {
    let headers = hyper::HeaderMap::new();
    assert_eq!(format_headers(&headers), "");
}

// ---------------------------------------------------------------
// TrackedBody tests
// ---------------------------------------------------------------

#[tokio::test]
async fn tracked_body_counts_bytes() {
    use http_body_util::BodyExt;
    let data = b"hello world";
    let stats = Arc::new(Mutex::new(BodyStats::new(0)));
    let inner = Full::new(Bytes::from(data.to_vec()));
    let body = TrackedBody::new(inner, Arc::clone(&stats), 1024);

    let _ = body.collect().await.unwrap();

    let st = stats.lock().unwrap();
    assert_eq!(st.bytes, data.len() as u64);
}

#[tokio::test]
async fn tracked_body_captures_preview() {
    use http_body_util::BodyExt;
    let data = b"hello world";
    let stats = Arc::new(Mutex::new(BodyStats::new(5))); // Capture 5 bytes
    let inner = Full::new(Bytes::from(data.to_vec()));
    let body = TrackedBody::new(inner, Arc::clone(&stats), 1024);

    let _ = body.collect().await.unwrap();

    let st = stats.lock().unwrap();
    assert_eq!(st.preview, b"hello");
}

#[tokio::test]
async fn tracked_body_enforces_max_size() {
    use http_body_util::BodyExt;
    let data = b"too much data";
    let stats = Arc::new(Mutex::new(BodyStats::new(0)));
    let inner = Full::new(Bytes::from(data.to_vec()));
    let body = TrackedBody::new(inner, Arc::clone(&stats), 5); // Limit to 5 bytes

    let res = body.collect().await;
    assert!(res.is_err());
    assert!(res
        .unwrap_err()
        .to_string()
        .contains("exceeded maximum size"));
}

// ---------------------------------------------------------------
// Denied-request integration test (no upstream needed)
//
// Pure-unit telemetry tests live in telemetry_hook/tests.rs (the
// `build_net_event` and `maybe_build_model_call` builders are pure
// functions we exercise without spinning up a connection); the
// integration tests below verify the same emit path end-to-end via
// the registered `TelemetryHook` running off a real
// `handle_connection`.
// ---------------------------------------------------------------

/// Build a rustls TLS client config that trusts our MITM CA.
fn make_mitm_client_config() -> Arc<rustls::ClientConfig> {
    let mut root_store = rustls::RootCertStore::empty();
    let ca_certs: Vec<_> = rustls_pemfile::certs(&mut CA_CERT.as_bytes())
        .collect::<Result<_, _>>()
        .unwrap();
    for cert in ca_certs {
        root_store.add(cert).unwrap();
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(root_store)
            .with_no_client_auth(),
    )
}

#[tokio::test]
async fn websocket_upgrade_rejected_with_400() {
    let config = make_config_dev();
    let (s1, s2) = UnixStream::pair().unwrap();

    let proxy_fd = s2.into_raw_fd();
    let proxy_config = Arc::clone(&config);
    let proxy_task = tokio::spawn(async move {
        handle_connection(proxy_fd, proxy_config).await;
    });

    let client_config = make_mitm_client_config();
    let connector = tokio_rustls::TlsConnector::from(client_config);
    s1.set_nonblocking(true).unwrap();
    let stream = tokio::net::UnixStream::from_std(s1).unwrap();
    let sni = rustls::pki_types::ServerName::try_from(TEST_DOMAIN.to_owned()).unwrap();
    let tls_stream = connector.connect(sni, stream).await.unwrap();

    let io = TokioIo::new(tls_stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let req = hyper::Request::builder()
        .method("GET")
        .uri("/ws")
        .header("host", TEST_DOMAIN)
        .header("upgrade", "websocket")
        .header("connection", "upgrade")
        .body(
            Full::new(Bytes::new())
                .map_err(|never| -> anyhow::Error { match never {} })
                .boxed(),
        )
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    assert_eq!(
        resp.status().as_u16(),
        400,
        "WebSocket upgrades should return 400"
    );
    let _ = resp.into_body().collect().await;

    drop(sender);
    let _ = proxy_task.await;

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

    let reader = config.db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].decision, Decision::Denied);
    assert_eq!(events[0].status_code, Some(400));
    assert_eq!(
        events[0].matched_rule,
        Some("websocket-not-supported".to_string())
    );
}

/// Upstream DNS failure returns 502 instead of killing the connection.
#[tokio::test]
async fn upstream_error_returns_502() {
    // Allow nonexistent.invalid but it will fail at TCP connect.
    let config = make_config_dev();
    let (s1, s2) = UnixStream::pair().unwrap();

    let proxy_fd = s2.into_raw_fd();
    let proxy_config = Arc::clone(&config);
    let proxy_task = tokio::spawn(async move {
        handle_connection(proxy_fd, proxy_config).await;
    });

    let client_config = make_mitm_client_config();
    let connector = tokio_rustls::TlsConnector::from(client_config);
    s1.set_nonblocking(true).unwrap();
    let stream = tokio::net::UnixStream::from_std(s1).unwrap();
    let sni = rustls::pki_types::ServerName::try_from("nonexistent.invalid").unwrap();
    let tls_stream = connector.connect(sni, stream).await.unwrap();

    let io = TokioIo::new(tls_stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let req = hyper::Request::builder()
        .method("GET")
        .uri("/")
        .header("host", "nonexistent.invalid")
        .body(
            Full::new(Bytes::new())
                .map_err(|never| -> anyhow::Error { match never {} })
                .boxed(),
        )
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    assert_eq!(
        resp.status().as_u16(),
        502,
        "Upstream error should return 502"
    );
    let _ = resp.into_body().collect().await;

    drop(sender);
    let _ = proxy_task.await;

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

    let reader = config.db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].decision, Decision::Error);
    assert_eq!(events[0].status_code, Some(502));
    assert_eq!(events[0].domain, "nonexistent.invalid");
}

#[tokio::test]
async fn runtime_security_engine_blocks_plain_http_before_upstream_dispatch() {
    let mut engine = SecurityEngine::default();
    engine.set_enforcement(Box::new(
        CelEnforcementEvaluator::compile(vec![CelEnforcementRule {
            id: "block-openai-inline".into(),
            pack_id: Some("corp-enforcement".into()),
            condition: "http.request.host == 'api.openai.com' \
                && http.request.path.startsWith('/v1/chat')"
                .into(),
            decision: SecurityDecisionAction::Block,
            reason: Some("inline OpenAI block".into()),
        }])
        .unwrap(),
    ));
    let config =
        make_config_dev_with_security_engine(Some(Arc::new(std::sync::Mutex::new(engine))));
    let (port, upstream_task) = spawn_http_no_touch_fixture().await;
    let (mut sender, proxy_task, conn_task) = open_direct_plain_http_request_conn(
        &config,
        "api.openai.com",
        port,
        Some(ProviderKind::OpenAi),
    )
    .await;

    let (status, body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-test", "needle").await;

    assert_eq!(status, 403);
    assert!(body.contains("inline OpenAI block"));
    upstream_task.await.unwrap();
    drop(sender);
    let _ = conn_task.await;
    let _ = proxy_task.await;

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

    let reader = config.db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].decision, Decision::Denied);
    assert_eq!(
        events[0].policy_rule.as_deref(),
        Some("block-openai-inline")
    );

    let security = reader
        .query_raw(
            "SELECT se.final_action, steps.rule_id, steps.message \
             FROM security_events se \
             LEFT JOIN security_event_steps steps ON steps.event_id = se.event_id",
        )
        .unwrap();
    assert!(security.contains("block"));
    assert!(security.contains("block-openai-inline"));
}

#[tokio::test]
async fn runtime_security_engine_blocks_request_body_before_upstream_dispatch() {
    let mut engine = SecurityEngine::default();
    engine.set_enforcement(Box::new(
        CelEnforcementEvaluator::compile(vec![CelEnforcementRule {
            id: "block-body-secret-inline".into(),
            pack_id: Some("corp-enforcement".into()),
            condition: "http.request.host == 'api.openai.com' \
                && http.request.body.text.contains('needle')"
                .into(),
            decision: SecurityDecisionAction::Block,
            reason: Some("body secret egress".into()),
        }])
        .unwrap(),
    ));
    let config =
        make_config_dev_with_security_engine(Some(Arc::new(std::sync::Mutex::new(engine))));
    let (port, upstream_task) = spawn_http_no_touch_fixture().await;
    let (mut sender, proxy_task, conn_task) = open_direct_plain_http_request_conn(
        &config,
        "api.openai.com",
        port,
        Some(ProviderKind::OpenAi),
    )
    .await;

    let (status, body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-test", "needle").await;

    assert_eq!(status, 403);
    assert!(body.contains("body secret egress"));
    upstream_task.await.unwrap();
    drop(sender);
    let _ = conn_task.await;
    let _ = proxy_task.await;

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

    let reader = config.db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].decision, Decision::Denied);
    assert_eq!(
        events[0].policy_rule.as_deref(),
        Some("block-body-secret-inline")
    );
    assert!(events[0]
        .request_body_preview
        .as_deref()
        .is_some_and(|preview| preview.contains("needle")));
}

#[tokio::test]
async fn runtime_security_engine_blocks_response_body_before_guest_delivery() {
    let mut engine = SecurityEngine::default();
    engine.set_enforcement(Box::new(
        CelEnforcementEvaluator::compile(vec![CelEnforcementRule {
            id: "block-response-secret-inline".into(),
            pack_id: Some("corp-enforcement".into()),
            condition: "http.response.body.text.contains('needle-from-upstream')".into(),
            decision: SecurityDecisionAction::Block,
            reason: Some("response secret ingress".into()),
        }])
        .unwrap(),
    ));
    let config =
        make_config_dev_with_security_engine(Some(Arc::new(std::sync::Mutex::new(engine))));
    let (port, upstream_task) = spawn_http_fixture_response(
        200,
        "OK",
        vec![("content-type", "text/plain")],
        "safe prefix needle-from-upstream unsafe suffix",
    )
    .await;
    let (mut sender, proxy_task, conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, None).await;

    let (status, body) =
        send_openai_json_request(&mut sender, "127.0.0.1", "/inspect", Bytes::new()).await;

    assert_eq!(status, 403);
    assert!(body.contains("response secret ingress"));
    let upstream_request = upstream_task.await.unwrap();
    assert!(
        upstream_request.starts_with("POST /inspect"),
        "response policy must run after upstream request dispatch"
    );
    drop(sender);
    let _ = conn_task.await;
    let _ = proxy_task.await;

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

    let reader = config.db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].decision, Decision::Denied);
    assert_eq!(
        events[0].policy_rule.as_deref(),
        Some("block-response-secret-inline")
    );
    assert!(
        events[0]
            .response_body_preview
            .as_deref()
            .is_some_and(|preview| !preview.contains("needle-from-upstream")),
        "blocked response body must not be journaled back through the guest response preview"
    );
}

#[tokio::test]
async fn runtime_security_engine_matches_decoded_gzip_response_body() {
    let mut engine = SecurityEngine::default();
    engine.set_enforcement(Box::new(
        CelEnforcementEvaluator::compile(vec![CelEnforcementRule {
            id: "block-gzip-response-secret-inline".into(),
            pack_id: Some("corp-enforcement".into()),
            condition: "http.response.body.text.contains('compressed-needle')".into(),
            decision: SecurityDecisionAction::Block,
            reason: Some("compressed response secret ingress".into()),
        }])
        .unwrap(),
    ));
    let config =
        make_config_dev_with_security_engine(Some(Arc::new(std::sync::Mutex::new(engine))));
    let gzipped = gzip_bytes(b"safe prefix compressed-needle unsafe suffix");
    let (port, upstream_task) = spawn_http_fixture_response_bytes(
        200,
        "OK",
        vec![("content-type", "text/plain"), ("content-encoding", "gzip")],
        gzipped,
    )
    .await;
    let (mut sender, proxy_task, conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, None).await;

    let (status, body) =
        send_openai_json_request(&mut sender, "127.0.0.1", "/inspect", Bytes::new()).await;

    assert_eq!(status, 403);
    assert!(body.contains("compressed response secret ingress"));
    let upstream_request = upstream_task.await.unwrap();
    assert!(upstream_request.starts_with("POST /inspect"));
    drop(sender);
    let _ = conn_task.await;
    let _ = proxy_task.await;

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

    let reader = config.db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].decision, Decision::Denied);
    assert_eq!(
        events[0].policy_rule.as_deref(),
        Some("block-gzip-response-secret-inline")
    );
}

// emit_model_call / trace-chain unit tests now live in
// telemetry_hook/tests.rs against the pure builders. Gzip-decode
// unit tests now live in decompression_hook/tests.rs against the
// sync ChunkHook (single chunk, multi-chunk split, passthrough,
// byte-by-byte fragmentation).

// ── is_llm_api_path tests ─────────────────────────────────────

#[test]
fn llm_api_path_anthropic_positive() {
    assert!(is_llm_api_path(ProviderKind::Anthropic, "/v1/messages"));
    assert!(is_llm_api_path(
        ProviderKind::Anthropic,
        "/v1/messages?beta=true"
    ));
    assert!(is_llm_api_path(ProviderKind::Anthropic, "/v1/complete"));
}

#[test]
fn llm_api_path_anthropic_negative() {
    assert!(!is_llm_api_path(
        ProviderKind::Anthropic,
        "/api/claude_code/metrics"
    ));
    assert!(!is_llm_api_path(
        ProviderKind::Anthropic,
        "/api/claude_code/settings"
    ));
    assert!(!is_llm_api_path(ProviderKind::Anthropic, "/v1/models"));
    assert!(!is_llm_api_path(
        ProviderKind::Anthropic,
        "/api/organizations"
    ));
}

#[test]
fn llm_api_path_openai_positive() {
    assert!(is_llm_api_path(
        ProviderKind::OpenAi,
        "/v1/chat/completions"
    ));
    assert!(is_llm_api_path(ProviderKind::OpenAi, "/v1/responses"));
    assert!(is_llm_api_path(ProviderKind::OpenAi, "/v1/completions"));
    assert!(is_llm_api_path(ProviderKind::OpenAi, "/v1/embeddings"));
    assert!(is_llm_api_path(
        ProviderKind::OpenAi,
        "/v1/audio/transcriptions"
    ));
}

#[test]
fn llm_api_path_openai_negative() {
    assert!(!is_llm_api_path(ProviderKind::OpenAi, "/v1/models"));
    assert!(!is_llm_api_path(ProviderKind::OpenAi, "/v1/files"));
    assert!(!is_llm_api_path(ProviderKind::OpenAi, "/dashboard/billing"));
}

#[test]
fn llm_api_path_google_positive() {
    assert!(is_llm_api_path(
        ProviderKind::Google,
        "/v1beta/models/gemini-2.0-flash:generateContent"
    ));
    assert!(is_llm_api_path(
        ProviderKind::Google,
        "/v1beta/models/gemini-2.0-flash:streamGenerateContent"
    ));
    assert!(is_llm_api_path(
        ProviderKind::Google,
        "/v1beta/models/text-embedding-004:embedContent"
    ));
    assert!(is_llm_api_path(
        ProviderKind::Google,
        "/v1beta/models/text-embedding-004:batchEmbedContents"
    ));
}

#[test]
fn llm_api_path_google_negative() {
    assert!(!is_llm_api_path(ProviderKind::Google, "/v1beta/models"));
    assert!(!is_llm_api_path(
        ProviderKind::Google,
        "/v1beta/models/gemini-2.0-flash"
    ));
    assert!(!is_llm_api_path(
        ProviderKind::Google,
        "/v1beta/cachedContents"
    ));
}

#[test]
fn llm_api_path_starts_with_is_intentional() {
    // /v1/messages_extra should match -- starts_with is fine since the real
    // path is /v1/messages with optional query params after it.
    assert!(is_llm_api_path(
        ProviderKind::Anthropic,
        "/v1/messages_extra"
    ));
}

// ---------------------------------------------------------------
// Per-request policy reload tests (keep-alive hot-reload)
// ---------------------------------------------------------------

/// Helper: open a TLS + HTTP/1.1 keep-alive connection through the proxy.
/// Returns the hyper sender and the proxy task handle.
async fn open_proxy_conn(
    config: &Arc<MitmProxyConfig>,
    domain: &str,
) -> (
    hyper::client::conn::http1::SendRequest<
        http_body_util::combinators::BoxBody<Bytes, anyhow::Error>,
    >,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<Result<(), hyper::Error>>,
) {
    let (s1, s2) = UnixStream::pair().unwrap();
    let proxy_fd = s2.into_raw_fd();
    let proxy_config = Arc::clone(config);
    let proxy_task = tokio::spawn(async move {
        handle_connection(proxy_fd, proxy_config).await;
    });

    let client_config = make_mitm_client_config();
    let connector = tokio_rustls::TlsConnector::from(client_config);
    s1.set_nonblocking(true).unwrap();
    let stream = tokio::net::UnixStream::from_std(s1).unwrap();
    let sni = rustls::pki_types::ServerName::try_from(domain.to_owned()).unwrap();
    let tls_stream = connector.connect(sni, stream).await.unwrap();

    let io = TokioIo::new(tls_stream);
    let (sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
    let conn_task = tokio::spawn(conn);

    (sender, proxy_task, conn_task)
}

async fn open_plain_http_proxy_conn(
    config: &Arc<MitmProxyConfig>,
) -> (
    hyper::client::conn::http1::SendRequest<
        http_body_util::combinators::BoxBody<Bytes, anyhow::Error>,
    >,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<Result<(), hyper::Error>>,
) {
    let (s1, s2) = UnixStream::pair().unwrap();
    let proxy_fd = s2.into_raw_fd();
    let proxy_config = Arc::clone(config);
    let proxy_task = tokio::spawn(async move {
        handle_connection(proxy_fd, proxy_config).await;
    });

    s1.set_nonblocking(true).unwrap();
    let stream = tokio::net::UnixStream::from_std(s1).unwrap();
    let io = TokioIo::new(stream);
    let (sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
    let conn_task = tokio::spawn(conn);

    (sender, proxy_task, conn_task)
}

async fn open_direct_plain_http_request_conn(
    config: &Arc<MitmProxyConfig>,
    domain: &'static str,
    upstream_port: u16,
    ai_provider: Option<ProviderKind>,
) -> (
    hyper::client::conn::http1::SendRequest<
        http_body_util::combinators::BoxBody<Bytes, anyhow::Error>,
    >,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<Result<(), hyper::Error>>,
) {
    let (s1, s2) = UnixStream::pair().unwrap();
    s1.set_nonblocking(true).unwrap();
    s2.set_nonblocking(true).unwrap();
    let client_stream = tokio::net::UnixStream::from_std(s1).unwrap();
    let server_stream = tokio::net::UnixStream::from_std(s2).unwrap();

    let upstream_tls = Arc::clone(&config.upstream_tls);
    let config_arc = Arc::clone(config);
    let cached_upstream: Arc<
        tokio::sync::Mutex<Option<hyper::client::conn::http1::SendRequest<ProxyBoxBody>>>,
    > = Arc::new(tokio::sync::Mutex::new(None));
    let proxy_task = tokio::spawn(async move {
        let io = TokioIo::new(server_stream);
        let svc = hyper::service::service_fn(move |req| {
            let upstream_tls = Arc::clone(&upstream_tls);
            let config_arc = Arc::clone(&config_arc);
            let cached_upstream = Arc::clone(&cached_upstream);
            async move {
                handle_request(
                    req,
                    domain,
                    Protocol::Http,
                    upstream_port,
                    &upstream_tls,
                    &config_arc,
                    &None,
                    ai_provider,
                    &cached_upstream,
                )
                .await
            }
        });
        let _ = hyper::server::conn::http1::Builder::new()
            .serve_connection(io, svc)
            .await;
    });

    let io = TokioIo::new(client_stream);
    let (sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
    let conn_task = tokio::spawn(conn);
    (sender, proxy_task, conn_task)
}

async fn spawn_http_fixture_response(
    status: u16,
    reason: &'static str,
    headers: Vec<(&'static str, &'static str)>,
    body: &'static str,
) -> (u16, tokio::task::JoinHandle<String>) {
    spawn_http_fixture_response_owned(status, reason, headers, body.to_string()).await
}

async fn spawn_http_fixture_response_owned(
    status: u16,
    reason: &'static str,
    headers: Vec<(&'static str, &'static str)>,
    body: String,
) -> (u16, tokio::task::JoinHandle<String>) {
    spawn_http_fixture_response_bytes(status, reason, headers, body.into_bytes()).await
}

async fn spawn_http_fixture_response_bytes(
    status: u16,
    reason: &'static str,
    headers: Vec<(&'static str, &'static str)>,
    body: Vec<u8>,
) -> (u16, tokio::task::JoinHandle<String>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]).into_owned();

        let mut response = format!("HTTP/1.1 {status} {reason}\r\n");
        for (name, value) in headers {
            response.push_str(name);
            response.push_str(": ");
            response.push_str(value);
            response.push_str("\r\n");
        }
        response.push_str(&format!(
            "content-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        ));
        stream.write_all(response.as_bytes()).await.unwrap();
        stream.write_all(&body).await.unwrap();
        request
    });
    (port, task)
}

fn gzip_bytes(body: &[u8]) -> Vec<u8> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(body).unwrap();
    encoder.finish().unwrap()
}

#[test]
fn response_uses_gzip_content_encoding_accepts_token_lists_case_insensitively() {
    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::CONTENT_ENCODING,
        http::HeaderValue::from_static("br, GZip"),
    );
    assert!(response_uses_gzip_content_encoding(&headers));

    headers.insert(
        http::header::CONTENT_ENCODING,
        http::HeaderValue::from_static("identity"),
    );
    assert!(!response_uses_gzip_content_encoding(&headers));
}

async fn spawn_http_no_touch_fixture() -> (u16, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        match tokio::time::timeout(std::time::Duration::from_millis(250), listener.accept()).await {
            Ok(Ok((_stream, _))) => panic!("model policy should have blocked upstream dispatch"),
            Ok(Err(error)) => panic!("fixture accept failed: {error}"),
            Err(_) => {}
        }
    });
    (port, task)
}

/// Helper: send a GET request on an existing keep-alive sender.
async fn send_get(
    sender: &mut hyper::client::conn::http1::SendRequest<
        http_body_util::combinators::BoxBody<Bytes, anyhow::Error>,
    >,
    domain: &str,
    path: &str,
) -> u16 {
    use http_body_util::BodyExt;
    let req = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("host", domain)
        .body(
            Full::new(Bytes::new())
                .map_err(|never| -> anyhow::Error { match never {} })
                .boxed(),
        )
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    let status = resp.status().as_u16();
    // Consume body so telemetry fires and connection stays alive.
    let _ = resp.into_body().collect().await;
    status
}

async fn send_openai_chat_completion(
    sender: &mut hyper::client::conn::http1::SendRequest<
        http_body_util::combinators::BoxBody<Bytes, anyhow::Error>,
    >,
    host: &str,
    model: &str,
    body_secret: &str,
) -> (u16, String) {
    let body = format!(
        r#"{{"model":"{model}","messages":[{{"role":"system","content":"protect {body_secret}"}},{{"role":"user","content":"hello {body_secret}"}}],"tools":[{{"type":"function","function":{{"name":"lookup","parameters":{{"type":"object"}}}}}}]}}"#
    );
    send_openai_json_request(sender, host, "/v1/chat/completions", Bytes::from(body)).await
}

async fn send_openai_json_request(
    sender: &mut hyper::client::conn::http1::SendRequest<
        http_body_util::combinators::BoxBody<Bytes, anyhow::Error>,
    >,
    host: &str,
    path: &str,
    body: Bytes,
) -> (u16, String) {
    let req = hyper::Request::builder()
        .method("POST")
        .uri(path)
        .header("host", host)
        .header("content-type", "application/json")
        .header("authorization", "Bearer secret")
        .body(
            Full::new(body)
                .map_err(|never| -> anyhow::Error { match never {} })
                .boxed(),
        )
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    let status = resp.status().as_u16();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn openai_sse_text_response(model: &str, content: &str) -> String {
    format!(
        "data: {{\"id\":\"chatcmpl-policy\",\"model\":\"{model}\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{content}\"}},\"finish_reason\":null}}]}}\n\n\
data: {{\"id\":\"chatcmpl-policy\",\"model\":\"{model}\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\n\
data: [DONE]\n\n"
    )
}

fn openai_sse_tool_call_response(
    model: &str,
    call_id: &str,
    tool_name: &str,
    arguments: &str,
) -> String {
    let tool_name = serde_json::to_string(tool_name).unwrap();
    let arguments = serde_json::to_string(arguments).unwrap();
    format!(
        "data: {{\"id\":\"chatcmpl-policy\",\"model\":\"{model}\",\"choices\":[{{\"index\":0,\"delta\":{{\"tool_calls\":[{{\"index\":0,\"id\":\"{call_id}\",\"type\":\"function\",\"function\":{{\"name\":{tool_name},\"arguments\":{arguments}}}}}]}},\"finish_reason\":null}}]}}\n\n\
data: {{\"id\":\"chatcmpl-policy\",\"model\":\"{model}\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}]}}\n\n\
data: [DONE]\n\n"
    )
}

mod connection_behavior;

#[test]
fn upstream_connect_target_honors_debug_test_override() {
    let previous = std::env::var_os("CAPSEM_TEST_UPSTREAM_OVERRIDES");
    std::env::set_var(
        "CAPSEM_TEST_UPSTREAM_OVERRIDES",
        "api.openai.com:80=http://127.0.0.1:4567,other.example:443=127.0.0.1:9443",
    );
    assert_eq!(
        upstream_connect_target("api.openai.com", 80),
        UpstreamConnectTarget {
            address: "127.0.0.1:4567".to_string(),
            plaintext_tls: true,
        }
    );
    assert_eq!(
        upstream_connect_target("api.openai.com", 443),
        UpstreamConnectTarget {
            address: "api.openai.com:443".to_string(),
            plaintext_tls: false,
        }
    );
    if let Some(value) = previous {
        std::env::set_var("CAPSEM_TEST_UPSTREAM_OVERRIDES", value);
    } else {
        std::env::remove_var("CAPSEM_TEST_UPSTREAM_OVERRIDES");
    }
}
