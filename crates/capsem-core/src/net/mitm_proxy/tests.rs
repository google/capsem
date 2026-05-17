use super::fd_stream::{set_nonblocking, AsyncFdStream, ReplayReader};
use super::util::{format_headers, is_llm_api_path};
use super::*;
use std::os::unix::io::IntoRawFd;
use std::os::unix::net::UnixStream;

use http_body_util::BodyExt;

use crate::net::cert_authority::CertAuthority;
use crate::net::policy::NetworkPolicy;

const CA_KEY: &str = include_str!("../../../../../config/capsem-ca.key");
const CA_CERT: &str = include_str!("../../../../../config/capsem-ca.crt");

/// Flush delay for the DB writer thread to process queued writes.
const DB_FLUSH_MS: u64 = 100;

/// Non-routable domain for tests that go through the full proxy pipeline.
/// Must never resolve so allowed requests always hit the 502 upstream-error
/// path instead of reaching a real server.
const TEST_DOMAIN: &str = "thisdomaindoesnotexistforsur3.ai";

fn make_config_with_policy(policy: NetworkPolicy) -> Arc<MitmProxyConfig> {
    make_config_with_policy_v2(
        policy,
        Arc::new(tokio::sync::RwLock::new(Arc::new(
            crate::net::policy_config::PolicyConfig::default(),
        ))),
    )
}

fn make_config_with_policy_v2(
    policy: NetworkPolicy,
    policy_v2: Arc<tokio::sync::RwLock<Arc<crate::net::policy_config::PolicyConfig>>>,
) -> Arc<MitmProxyConfig> {
    let ca = Arc::new(CertAuthority::load(CA_KEY, CA_CERT).unwrap());
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(DbWriter::open(&dir.path().join("test.db"), 256).unwrap());
    // Leak the tempdir so it lives for the test
    std::mem::forget(dir);
    let policy_arc = Arc::new(std::sync::RwLock::new(Arc::new(policy)));
    let telemetry = Arc::new(super::telemetry_hook::TelemetryDeps {
        db: Arc::clone(&db),
        pricing: Arc::new(crate::net::ai_traffic::pricing::PricingTable::load()),
        trace_state: Arc::new(std::sync::Mutex::new(
            crate::net::ai_traffic::TraceState::new(),
        )),
    });
    let pipeline = super::make_production_pipeline_with_policy_v2(
        Arc::clone(&policy_arc),
        Arc::clone(&policy_v2),
        Arc::clone(&telemetry),
    );
    Arc::new(MitmProxyConfig {
        ca,
        policy: policy_arc,
        policy_v2,
        db,
        upstream_tls: make_upstream_tls_config(),
        telemetry,
        pipeline,
        mcp_endpoint: None,
        confirmer: Arc::new(crate::net::policy_confirm::PlaceholderConfirmer),
        confirm_opts: crate::net::policy_confirm::default_confirm_backoff("confirm-model-test"),
    })
}

fn make_config_dev() -> Arc<MitmProxyConfig> {
    make_config_with_policy(NetworkPolicy::default_dev())
}

fn make_config_deny_all() -> Arc<MitmProxyConfig> {
    make_config_with_policy(NetworkPolicy::new(vec![], false, false))
}

fn allow_test_domain_policy() -> NetworkPolicy {
    use crate::net::policy::{DomainMatcher, PolicyRule};
    NetworkPolicy::new(
        vec![PolicyRule {
            matcher: DomainMatcher::parse(TEST_DOMAIN),
            allow_read: true,
            allow_write: true,
        }],
        false,
        false,
    )
}

fn policy_v2_from_toml(
    toml_text: &str,
) -> Arc<tokio::sync::RwLock<Arc<crate::net::policy_config::PolicyConfig>>> {
    let settings: crate::net::policy_config::SettingsFile = toml::from_str(toml_text).unwrap();
    Arc::new(tokio::sync::RwLock::new(Arc::new(settings.policy)))
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

// ---------------------------------------------------------------
// Metadata fragmentation tests
// ---------------------------------------------------------------

#[tokio::test]
async fn fragmented_metadata_is_reassembled() {
    let config = make_config_dev();
    let (s1, s2) = UnixStream::pair().unwrap();

    let proxy_fd = s2.into_raw_fd();
    let proxy_config = Arc::clone(&config);
    let proxy_task = tokio::spawn(async move {
        handle_connection(proxy_fd, proxy_config).await;
    });

    // Write metadata in two fragments: first the prefix, then the rest + newline + client hello.
    s1.set_nonblocking(false).unwrap();
    let mut writer = s1;
    // Fragment 1: metadata prefix without the newline
    std::io::Write::write_all(&mut writer, b"\0CAPSEM_META:my_proc").unwrap();
    // Small delay so the proxy reads the first fragment before the rest arrives.
    std::thread::sleep(std::time::Duration::from_millis(50));
    // Fragment 2: rest of metadata with newline, then the TLS ClientHello
    let mut frag2 = b"ess_name\n".to_vec();
    frag2.extend_from_slice(&make_client_hello(TEST_DOMAIN));
    std::io::Write::write_all(&mut writer, &frag2).unwrap();
    drop(writer);

    // The proxy should have reassembled metadata and completed TLS handshake.
    // It will fail after handshake (no real TLS client), but the key check
    // is that it didn't error during metadata parsing.
    let _ = proxy_task.await;

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

    let reader = config.db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    // Should have an event (error from failed TLS with raw bytes, not metadata error).
    // The important thing is we didn't get "metadata exceeded 4KB" or "EOF during metadata".
    if !events.is_empty() {
        let rule = events[0].matched_rule.as_deref().unwrap_or("");
        assert!(
            !rule.contains("metadata"),
            "Fragmented metadata should be reassembled, got: {rule}"
        );
    }
}

#[tokio::test]
async fn oversized_metadata_rejected() {
    let config = make_config_dev();
    let (s1, s2) = UnixStream::pair().unwrap();

    let proxy_fd = s2.into_raw_fd();
    let proxy_config = Arc::clone(&config);
    let proxy_task = tokio::spawn(async move {
        handle_connection(proxy_fd, proxy_config).await;
    });

    // Write >4KB metadata without a newline terminator.
    let mut oversized = b"\0CAPSEM_META:".to_vec();
    oversized.extend_from_slice(&vec![b'A'; 5000]);
    let mut writer = s1;
    std::io::Write::write_all(&mut writer, &oversized).unwrap();
    drop(writer);

    let _ = proxy_task.await;

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

    let reader = config.db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert!(
        !events.is_empty(),
        "oversized metadata should produce error event"
    );
    assert_eq!(events[0].decision, Decision::Error);
    let rule = events[0].matched_rule.as_deref().unwrap_or("");
    assert!(
        rule.contains("4KB"),
        "Should mention 4KB limit, got: {rule}"
    );
}

// ---------------------------------------------------------------
// Existing connection-level tests (unchanged behavior)
// ---------------------------------------------------------------

#[tokio::test]
async fn no_sni_records_error() {
    let config = make_config_dev();
    let (mut s1, s2) = UnixStream::pair().unwrap();

    std::io::Write::write_all(&mut s1, b"not a client hello").unwrap();
    drop(s1);

    handle_connection(s2.into_raw_fd(), config.clone()).await;

    // Give writer thread time to flush.
    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

    let reader = config.db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].domain, "<unknown>");
    // Without valid TLS, it's an error (handshake failure)
    assert!(matches!(
        events[0].decision,
        Decision::Error | Decision::Denied
    ));
}

#[tokio::test]
async fn empty_connection_records_error() {
    let config = make_config_dev();
    let (_s1, s2) = UnixStream::pair().unwrap();
    drop(_s1);

    handle_connection(s2.into_raw_fd(), config.clone()).await;

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

    let reader = config.db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].decision, Decision::Error);
}

#[test]
fn replay_reader_drains_buffer_then_inner() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let buffer = b"hello".to_vec();
        let inner_data: &[u8] = b" world";
        let mut reader = ReplayReader::new(buffer, inner_data);

        let mut output = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut output)
            .await
            .unwrap();
        assert_eq!(&output, b"hello world");
    });
}

// ---------------------------------------------------------------
// AsyncFdStream tests
// ---------------------------------------------------------------

fn wrap_fd_like_handle_inner(raw_fd: RawFd) -> AsyncFdStream {
    let file = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(raw_fd) });
    let cloned = file.try_clone().expect("try_clone (dup) failed");
    set_nonblocking(raw_fd).expect("set_nonblocking failed");
    let async_fd = tokio::io::unix::AsyncFd::new(cloned).expect("AsyncFd::new failed");
    AsyncFdStream(async_fd)
}

#[tokio::test]
async fn async_fd_stream_basic_read_write() {
    let (s1, s2) = UnixStream::pair().unwrap();
    let fd1 = s1.into_raw_fd();
    let fd2 = s2.into_raw_fd();
    let mut stream1 = wrap_fd_like_handle_inner(fd1);
    let mut stream2 = wrap_fd_like_handle_inner(fd2);

    tokio::io::AsyncWriteExt::write_all(&mut stream1, b"hello vsock")
        .await
        .unwrap();
    let mut buf = vec![0u8; 64];
    let n = tokio::io::AsyncReadExt::read(&mut stream2, &mut buf)
        .await
        .unwrap();
    assert_eq!(&buf[..n], b"hello vsock");

    unsafe {
        libc::close(fd1);
        libc::close(fd2);
    }
}

#[tokio::test]
async fn async_fd_stream_large_transfer() {
    let (s1, s2) = UnixStream::pair().unwrap();
    let fd1 = s1.into_raw_fd();
    let fd2 = s2.into_raw_fd();
    let mut stream1 = wrap_fd_like_handle_inner(fd1);
    let mut stream2 = wrap_fd_like_handle_inner(fd2);

    let data: Vec<u8> = (0..131072).map(|i| (i % 251) as u8).collect();
    let send_data = data.clone();
    let writer = tokio::spawn(async move {
        tokio::io::AsyncWriteExt::write_all(&mut stream1, &send_data)
            .await
            .unwrap();
        drop(stream1);
        unsafe {
            libc::close(fd1);
        }
    });
    let mut received = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut stream2, &mut received)
        .await
        .unwrap();
    writer.await.unwrap();

    assert_eq!(received.len(), data.len());
    assert_eq!(received, data);

    unsafe {
        libc::close(fd2);
    }
}

#[tokio::test]
async fn async_fd_stream_eof_on_close() {
    let (s1, s2) = UnixStream::pair().unwrap();
    let fd1 = s1.into_raw_fd();
    let fd2 = s2.into_raw_fd();
    let mut stream2 = wrap_fd_like_handle_inner(fd2);

    {
        let mut stream1 = wrap_fd_like_handle_inner(fd1);
        tokio::io::AsyncWriteExt::write_all(&mut stream1, b"before eof")
            .await
            .unwrap();
    }
    unsafe {
        libc::close(fd1);
    }

    let mut buf = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut stream2, &mut buf)
        .await
        .unwrap();
    assert_eq!(&buf, b"before eof");

    unsafe {
        libc::close(fd2);
    }
}

#[tokio::test]
async fn async_fd_stream_bidirectional() {
    let (s1, s2) = UnixStream::pair().unwrap();
    let fd1 = s1.into_raw_fd();
    let fd2 = s2.into_raw_fd();
    let mut stream1 = wrap_fd_like_handle_inner(fd1);
    let mut stream2 = wrap_fd_like_handle_inner(fd2);

    tokio::io::AsyncWriteExt::write_all(&mut stream1, b"ping")
        .await
        .unwrap();
    let mut buf = vec![0u8; 32];
    let n = tokio::io::AsyncReadExt::read(&mut stream2, &mut buf)
        .await
        .unwrap();
    assert_eq!(&buf[..n], b"ping");

    tokio::io::AsyncWriteExt::write_all(&mut stream2, b"pong")
        .await
        .unwrap();
    let n = tokio::io::AsyncReadExt::read(&mut stream1, &mut buf)
        .await
        .unwrap();
    assert_eq!(&buf[..n], b"pong");

    unsafe {
        libc::close(fd1);
        libc::close(fd2);
    }
}

#[tokio::test]
async fn async_fd_stream_replay_then_live() {
    let (s1, s2) = UnixStream::pair().unwrap();
    let fd2 = s2.into_raw_fd();
    let mut stream2 = wrap_fd_like_handle_inner(fd2);

    let mut writer = s1;
    std::io::Write::write_all(&mut writer, b"INITIAL").unwrap();
    std::io::Write::write_all(&mut writer, b"REMAINING").unwrap();
    drop(writer);

    let mut initial = vec![0u8; 7];
    tokio::io::AsyncReadExt::read_exact(&mut stream2, &mut initial)
        .await
        .unwrap();
    assert_eq!(&initial, b"INITIAL");

    let mut replay = ReplayReader::new(initial, stream2);
    let mut all = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut replay, &mut all)
        .await
        .unwrap();
    assert_eq!(&all, b"INITIALREMAINING");

    unsafe {
        libc::close(fd2);
    }
}

/// Full TLS handshake through handle_connection using a real rustls client.
#[tokio::test]
async fn tls_handshake_completes_without_global_provider() {
    let config = make_config_dev();
    let (s1, s2) = UnixStream::pair().unwrap();

    let proxy_fd = s2.into_raw_fd();
    let proxy_config = Arc::clone(&config);
    let proxy_task = tokio::spawn(async move {
        handle_connection(proxy_fd, proxy_config).await;
    });

    let mut root_store = rustls::RootCertStore::empty();
    let ca_certs: Vec<_> = rustls_pemfile::certs(&mut CA_CERT.as_bytes())
        .collect::<Result<_, _>>()
        .unwrap();
    for cert in ca_certs {
        root_store.add(cert).unwrap();
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let client_config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client_config));

    s1.set_nonblocking(true).unwrap();
    let stream = tokio::net::UnixStream::from_std(s1).unwrap();
    let domain = rustls::pki_types::ServerName::try_from(TEST_DOMAIN).unwrap();
    let tls_result = connector.connect(domain, stream).await;

    assert!(
        tls_result.is_ok(),
        "TLS handshake failed: {:?}",
        tls_result.err()
    );

    drop(tls_result);
    let _ = proxy_task.await;
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
async fn denied_request_emits_event() {
    let config = make_config_deny_all();
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
        .uri("/secret")
        .header("host", TEST_DOMAIN)
        .body(
            Full::new(Bytes::new())
                .map_err(|never| -> anyhow::Error { match never {} })
                .boxed(),
        )
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    assert_eq!(resp.status().as_u16(), 403);
    // Consume the body to trigger telemetry emission.
    let _ = resp.into_body().collect().await;

    drop(sender);
    let _ = proxy_task.await;

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

    let reader = config.db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].decision, Decision::Denied);
    assert_eq!(events[0].status_code, Some(403));
    assert_eq!(events[0].method, Some("GET".to_string()));
    assert_eq!(events[0].path, Some("/secret".to_string()));
}

/// Multiple denied requests on the same keep-alive connection produce
/// one event per request (the core bug this fix addresses).
#[tokio::test]
async fn multiple_denied_requests_emit_separate_events() {
    let config = make_config_deny_all();
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

    // Send 3 requests on the same keep-alive connection.
    for path in ["/a", "/b", "/c"] {
        let req = hyper::Request::builder()
            .method("GET")
            .uri(path)
            .header("host", TEST_DOMAIN)
            .body(
                Full::new(Bytes::new())
                    .map_err(|never| -> anyhow::Error { match never {} })
                    .boxed(),
            )
            .unwrap();
        let resp = sender.send_request(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 403);
        let _ = resp.into_body().collect().await;
    }

    drop(sender);
    let _ = proxy_task.await;

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

    let reader = config.db.reader().unwrap();
    let mut events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 3, "3 requests should produce 3 events, not 1");
    events.reverse(); // chronological order
    assert_eq!(events[0].path, Some("/a".to_string()));
    assert_eq!(events[1].path, Some("/b".to_string()));
    assert_eq!(events[2].path, Some("/c".to_string()));
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
    use crate::net::policy::{DomainMatcher, PolicyRule};
    let policy = NetworkPolicy::new(
        vec![PolicyRule {
            matcher: DomainMatcher::parse("nonexistent.invalid"),
            allow_read: true,
            allow_write: true,
        }],
        false,
        false,
    );
    let config = make_config_with_policy(policy);
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

fn allow_local_http_policy(port: u16) -> NetworkPolicy {
    use crate::net::policy::{DomainMatcher, PolicyRule};

    let mut policy = NetworkPolicy::new(
        vec![PolicyRule {
            matcher: DomainMatcher::parse("127.0.0.1"),
            allow_read: true,
            allow_write: true,
        }],
        false,
        false,
    );
    policy.http_upstream_ports.push(port);
    policy
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
            "content-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        ));
        stream.write_all(response.as_bytes()).await.unwrap();
        request
    });
    (port, task)
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

#[tokio::test]
async fn policy_v2_model_request_allow_dispatches_and_records_policy_fields() {
    let (port, upstream_task) = spawn_http_fixture_response(
        200,
        "OK",
        vec![("content-type", "application/json")],
        r#"{"id":"chatcmpl-test","choices":[]}"#,
    )
    .await;
    let config = make_config_with_policy_v2(
        allow_local_http_policy(port),
        policy_v2_from_toml(
            r#"
[policy.model.allow_gpt4o]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o" && messages_count == "2" && tools_count == "1"'
decision = "allow"
priority = 10
reason = "Allow the local model fixture"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-4o", "allow-secret").await;
    assert_eq!(status, 200);
    assert!(response_body.contains("chatcmpl-test"));
    drop(sender);
    let _ = proxy_task.await;
    let upstream_request = upstream_task.await.unwrap();
    assert!(
        upstream_request.contains("allow-secret"),
        "allow must preserve the original request body for upstream dispatch"
    );

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Allowed);
    assert_eq!(event.status_code, Some(200));
    assert!(event.bytes_sent > 0);
    assert_eq!(event.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(event.policy_action.as_deref(), Some("allow"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.allow_gpt4o")
    );
    assert_eq!(
        event.policy_reason.as_deref(),
        Some("Allow the local model fixture")
    );
    let model_calls = config.db.reader().unwrap().recent_model_calls(10).unwrap();
    assert_eq!(model_calls.len(), 1);
    let call = &model_calls[0].1;
    assert_eq!(call.provider, "openai");
    assert_eq!(call.model.as_deref(), Some("gpt-4o"));
    assert_eq!(call.messages_count, 2);
    assert_eq!(call.tools_count, 1);
    assert!(call.request_bytes > 0);
    assert!(
        call.request_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("allow-secret"),
        "allowed model request telemetry should retain the captured request preview"
    );
}

#[tokio::test]
async fn policy_v2_model_request_block_stops_before_upstream_and_records_policy_fields() {
    let (port, upstream_task) = spawn_http_no_touch_fixture().await;
    let config = make_config_with_policy_v2(
        allow_local_http_policy(port),
        policy_v2_from_toml(
            r#"
[policy.model.block_gpt4o]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o" && request.body.contains("block-secret")'
decision = "block"
priority = 10
reason = "Do not send this model request"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-4o", "block-secret").await;
    assert_eq!(status, 403);
    assert!(response_body.contains("policy.model.block_gpt4o"));
    drop(sender);
    let _ = proxy_task.await;
    upstream_task.await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Denied);
    assert_eq!(event.status_code, Some(403));
    assert!(event.bytes_sent > 0);
    assert_eq!(event.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(event.policy_action.as_deref(), Some("block"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.block_gpt4o")
    );
    assert_eq!(
        event.policy_reason.as_deref(),
        Some("Do not send this model request")
    );
    assert!(
        !event
            .request_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("block-secret"),
        "denied model request telemetry must not retain the blocked body"
    );
    let model_calls = config.db.reader().unwrap().recent_model_calls(10).unwrap();
    assert_eq!(model_calls.len(), 1);
    let call = &model_calls[0].1;
    assert_eq!(call.provider, "openai");
    assert_eq!(call.model, None);
    assert!(call.request_bytes > 0);
    assert!(
        !call
            .request_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("block-secret"),
        "denied model call telemetry must not retain the blocked body"
    );
}

#[tokio::test]
async fn policy_v2_model_request_block_matches_truncated_json_before_upstream_dispatch() {
    let (port, upstream_task) = spawn_http_no_touch_fixture().await;
    let config = make_config_with_policy_v2(
        allow_local_http_policy(port),
        policy_v2_from_toml(
            r#"
[policy.model.block_truncated_json]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o-mini" && request.body.contains("truncated-secret")'
decision = "block"
priority = 10
reason = "Block even when the JSON body is truncated"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) = send_openai_json_request(
        &mut sender,
        "api.openai.com",
        "/v1/chat/completions",
        Bytes::from_static(
            br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"truncated-secret"}"#,
        ),
    )
    .await;
    assert_eq!(status, 403);
    assert!(response_body.contains("policy.model.block_truncated_json"));
    drop(sender);
    let _ = proxy_task.await;
    upstream_task.await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Denied);
    assert_eq!(event.policy_action.as_deref(), Some("block"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.block_truncated_json")
    );
    assert!(
        !event
            .request_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("truncated-secret"),
        "truncated denied body must not leak to net_events"
    );
}

#[tokio::test]
async fn policy_v2_model_request_invalid_condition_fails_closed_without_upstream_dispatch() {
    use std::collections::HashMap;

    let (port, upstream_task) = spawn_http_no_touch_fixture().await;
    let mut model = HashMap::new();
    model.insert(
        "bad_regex".to_string(),
        crate::net::policy_config::PolicyRuleConfig {
            on: crate::net::policy_config::PolicyCallback::ModelRequest,
            condition: "request.body.matches(\"[\")".to_string(),
            decision: crate::net::policy_config::PolicyDecisionKind::Allow,
            priority: 10,
            reason: None,
            rewrite_target: None,
            rewrite_value: None,
            strip_request_headers: Vec::new(),
            strip_response_headers: Vec::new(),
        },
    );
    let policy_v2 = Arc::new(tokio::sync::RwLock::new(Arc::new(
        crate::net::policy_config::PolicyConfig {
            model,
            ..crate::net::policy_config::PolicyConfig::default()
        },
    )));
    let config = make_config_with_policy_v2(allow_local_http_policy(port), policy_v2);
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-4o", "bad-rule-secret")
            .await;
    assert_eq!(status, 403);
    assert!(response_body.contains("policy.model.invalid_condition"));
    drop(sender);
    let _ = proxy_task.await;
    upstream_task.await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Denied);
    assert_eq!(event.policy_action.as_deref(), Some("block"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.invalid_condition")
    );
    assert!(
        !event
            .request_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("bad-rule-secret"),
        "invalid runtime policy conditions must fail closed without request-body telemetry leakage"
    );
}

#[tokio::test]
async fn policy_v2_model_request_rules_do_not_run_on_non_llm_provider_paths() {
    let (port, upstream_task) = spawn_http_fixture_response(
        200,
        "OK",
        vec![("content-type", "application/json")],
        r#"{"object":"list","data":[]}"#,
    )
    .await;
    let config = make_config_with_policy_v2(
        allow_local_http_policy(port),
        policy_v2_from_toml(
            r#"
[policy.model.block_gpt4o]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o" && request.body.contains("non-llm-secret")'
decision = "block"
priority = 10
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let body = Bytes::from_static(
        br#"{"model":"gpt-4o","messages":[{"role":"user","content":"non-llm-secret"}]}"#,
    );
    let (status, response_body) =
        send_openai_json_request(&mut sender, "api.openai.com", "/v1/models", body).await;
    assert_eq!(status, 200);
    assert!(response_body.contains(r#""object":"list""#));
    drop(sender);
    let _ = proxy_task.await;
    let upstream_request = upstream_task.await.unwrap();
    assert!(
        upstream_request.contains("non-llm-secret"),
        "non-LLM provider paths should not run model.request rules"
    );

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Allowed);
    assert_eq!(event.policy_action, None);
    assert!(config
        .db
        .reader()
        .unwrap()
        .recent_model_calls(10)
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn policy_v2_model_request_ask_placeholder_confirmer_allows_upstream_dispatch() {
    let (port, upstream_task) = spawn_http_fixture_response(
        200,
        "OK",
        vec![("content-type", "application/json")],
        r#"{"id":"resp","choices":[]}"#,
    )
    .await;
    let config = make_config_with_policy_v2(
        allow_local_http_policy(port),
        policy_v2_from_toml(
            r#"
[policy.model.ask_gpt4o]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o"'
decision = "ask"
priority = 10
reason = "Ask before sending this model request"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-4o", "ask-secret").await;
    assert_eq!(status, 200);
    assert!(response_body.contains(r#""id":"resp""#));
    drop(sender);
    let _ = proxy_task.await;
    let upstream_request = upstream_task.await.unwrap();
    assert!(
        upstream_request.contains("ask-secret"),
        "placeholder-confirmed model ask should dispatch the original request upstream"
    );

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Allowed);
    assert!(event.bytes_sent > 0);
    assert_eq!(event.policy_action.as_deref(), Some("allow"));
    assert_eq!(event.policy_rule.as_deref(), Some("policy.model.ask_gpt4o"));
}

#[tokio::test]
async fn policy_v2_model_request_rewrite_fails_closed_without_leaking_body() {
    let (port, upstream_task) = spawn_http_no_touch_fixture().await;
    let config = make_config_with_policy_v2(
        allow_local_http_policy(port),
        policy_v2_from_toml(
            r#"
[policy.model.rewrite_secret]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o" && request.body.contains("rewrite-secret")'
decision = "rewrite"
priority = 10
reason = "Rewrite secret-bearing model request"
rewrite_target = 'request.body =~ "rewrite-secret-(?P<suffix>[a-z]+)"'
rewrite_value = "[redacted-${suffix}]"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) = send_openai_chat_completion(
        &mut sender,
        "api.openai.com",
        "gpt-4o",
        "rewrite-secret-token",
    )
    .await;
    assert_eq!(status, 403);
    assert!(response_body.contains("policy.model.rewrite_secret"));
    drop(sender);
    let _ = proxy_task.await;
    upstream_task.await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Denied);
    assert!(event.bytes_sent > 0);
    assert_eq!(event.policy_action.as_deref(), Some("rewrite"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.rewrite_secret")
    );
    assert!(
        !event
            .request_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("rewrite-secret-token"),
        "unsupported model request rewrite must fail closed without telemetry leakage"
    );
}

#[tokio::test]
async fn policy_v2_model_response_block_stops_before_guest_and_records_policy_fields() {
    let (port, upstream_task) = spawn_http_fixture_response_owned(
        200,
        "OK",
        vec![("content-type", "text/event-stream")],
        openai_sse_text_response("gpt-4o", "hello response-secret"),
    )
    .await;
    let config = make_config_with_policy_v2(
        allow_local_http_policy(port),
        policy_v2_from_toml(
            r#"
[policy.model.block_secret_response]
on = "model.response"
if = 'provider == "openai" && model == "gpt-4o" && response.text.contains("response-secret")'
decision = "block"
priority = 10
reason = "Do not deliver secret model text"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-4o", "safe").await;
    assert_eq!(status, 403);
    assert!(response_body.contains("policy.model.block_secret_response"));
    assert!(
        !response_body.contains("response-secret"),
        "blocked model response must not reach the guest"
    );
    drop(sender);
    let _ = proxy_task.await;
    let upstream_request = upstream_task.await.unwrap();
    assert!(
        upstream_request.contains("gpt-4o"),
        "response policy should run after upstream dispatch"
    );

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Denied);
    assert_eq!(event.status_code, Some(403));
    assert_eq!(event.policy_action.as_deref(), Some("block"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.block_secret_response")
    );
    assert!(
        !event
            .response_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("response-secret"),
        "blocked model response telemetry must not retain the upstream response"
    );
    let model_calls = config.db.reader().unwrap().recent_model_calls(10).unwrap();
    assert_eq!(model_calls.len(), 1);
    let call = &model_calls[0].1;
    assert_eq!(call.provider, "openai");
    assert_eq!(call.model.as_deref(), Some("gpt-4o"));
    assert!(
        call.text_content
            .as_deref()
            .is_none_or(|text| !text.contains("response-secret")),
        "blocked model response must not populate secret text_content"
    );
}

#[tokio::test]
async fn policy_v2_model_response_rewrite_redacts_guest_and_session_db() {
    let (port, upstream_task) = spawn_http_fixture_response_owned(
        200,
        "OK",
        vec![("content-type", "text/event-stream")],
        openai_sse_text_response("gpt-4o", "hello response-secret"),
    )
    .await;
    let config = make_config_with_policy_v2(
        allow_local_http_policy(port),
        policy_v2_from_toml(
            r#"
[policy.model.rewrite_secret_response]
on = "model.response"
if = 'provider == "openai" && response.text.contains("response-secret")'
decision = "rewrite"
priority = 10
reason = "Redact model response text"
rewrite_target = 'response.text =~ "response-secret"'
rewrite_value = "[redacted-response]"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-4o", "safe").await;
    assert_eq!(status, 200);
    assert!(response_body.contains("[redacted-response]"));
    assert!(
        !response_body.contains("response-secret"),
        "rewritten model response must not leak to the guest"
    );
    drop(sender);
    let _ = proxy_task.await;
    let _ = upstream_task.await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Allowed);
    assert_eq!(event.status_code, Some(200));
    assert_eq!(event.policy_action.as_deref(), Some("rewrite"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.rewrite_secret_response")
    );
    let preview = event.response_body_preview.as_deref().unwrap_or_default();
    assert!(preview.contains("[redacted-response]"));
    assert!(
        !preview.contains("response-secret"),
        "rewritten response preview must not retain the original secret"
    );
    let model_calls = config.db.reader().unwrap().recent_model_calls(10).unwrap();
    assert_eq!(model_calls.len(), 1);
    let call = &model_calls[0].1;
    assert_eq!(
        call.text_content.as_deref(),
        Some("hello [redacted-response]")
    );
}

#[tokio::test]
async fn policy_v2_model_tool_call_block_stops_before_guest_and_redacts_telemetry() {
    let (port, upstream_task) = spawn_http_fixture_response_owned(
        200,
        "OK",
        vec![("content-type", "text/event-stream")],
        openai_sse_tool_call_response(
            "gpt-4o",
            "call_secret",
            "leak_secret",
            r#"{"secret":"tool-call-secret"}"#,
        ),
    )
    .await;
    let config = make_config_with_policy_v2(
        allow_local_http_policy(port),
        policy_v2_from_toml(
            r#"
[policy.model.block_secret_tool_call]
on = "model.tool_call"
if = 'provider == "openai" && tool.name == "leak_secret" && tool.arguments.secret.contains("tool-call-secret")'
decision = "block"
priority = 10
reason = "Do not deliver unsafe model tool calls"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-4o", "safe").await;
    assert_eq!(status, 403);
    assert!(response_body.contains("policy.model.block_secret_tool_call"));
    assert!(
        !response_body.contains("tool-call-secret"),
        "blocked provider-emitted tool call must not reach the guest"
    );
    drop(sender);
    let _ = proxy_task.await;
    let _ = upstream_task.await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Denied);
    assert_eq!(event.policy_action.as_deref(), Some("block"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.block_secret_tool_call")
    );
    assert!(
        !event
            .response_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("tool-call-secret"),
        "blocked tool-call telemetry must not retain upstream arguments"
    );
}

#[tokio::test]
async fn policy_v2_model_tool_call_ask_placeholder_confirmer_allows_guest_delivery() {
    let (port, upstream_task) = spawn_http_fixture_response_owned(
        200,
        "OK",
        vec![("content-type", "text/event-stream")],
        openai_sse_tool_call_response(
            "gpt-4o",
            "call_secret",
            "leak_secret",
            r#"{"secret":"tool-call-secret"}"#,
        ),
    )
    .await;
    let config = make_config_with_policy_v2(
        allow_local_http_policy(port),
        policy_v2_from_toml(
            r#"
[policy.model.ask_secret_tool_call]
on = "model.tool_call"
if = 'provider == "openai" && tool.arguments.secret.contains("tool-call-secret")'
decision = "ask"
priority = 10
reason = "Ask before delivering model tool calls"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-4o", "safe").await;
    assert_eq!(status, 200);
    assert!(
        response_body.contains("tool-call-secret"),
        "placeholder-confirmed model tool call ask should reach the guest"
    );
    drop(sender);
    let _ = proxy_task.await;
    let _ = upstream_task.await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Allowed);
    assert_eq!(event.policy_action.as_deref(), Some("allow"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.ask_secret_tool_call")
    );
}

#[tokio::test]
async fn policy_v2_model_tool_call_rewrite_redacts_guest_and_model_call_rows() {
    let (port, upstream_task) = spawn_http_fixture_response_owned(
        200,
        "OK",
        vec![("content-type", "text/event-stream")],
        openai_sse_tool_call_response(
            "gpt-4o",
            "call_secret",
            "leak_secret",
            r#"{"secret":"tool-call-secret"}"#,
        ),
    )
    .await;
    let config = make_config_with_policy_v2(
        allow_local_http_policy(port),
        policy_v2_from_toml(
            r#"
[policy.model.rewrite_secret_tool_call]
on = "model.tool_call"
if = 'provider == "openai" && tool.name == "leak_secret" && tool.arguments.secret.contains("tool-call-secret")'
decision = "rewrite"
priority = 10
reason = "Redact provider-emitted model tool arguments"
rewrite_target = 'tool.arguments =~ "tool-call-secret"'
rewrite_value = "[redacted-tool-call]"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-4o", "safe").await;
    assert_eq!(status, 200);
    assert!(response_body.contains("[redacted-tool-call]"));
    assert!(
        !response_body.contains("tool-call-secret"),
        "rewritten provider-emitted tool call must not leak to the guest"
    );
    drop(sender);
    let _ = proxy_task.await;
    let _ = upstream_task.await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Allowed);
    assert_eq!(event.policy_action.as_deref(), Some("rewrite"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.rewrite_secret_tool_call")
    );
    let preview = event.response_body_preview.as_deref().unwrap_or_default();
    assert!(preview.contains("[redacted-tool-call]"));
    assert!(
        !preview.contains("tool-call-secret"),
        "rewritten tool-call response preview must not retain the original secret"
    );

    let reader = config.db.reader().unwrap();
    let model_calls = reader.recent_model_calls(10).unwrap();
    assert_eq!(model_calls.len(), 1);
    let tool_calls = reader.tool_calls_for(model_calls[0].0).unwrap();
    assert_eq!(tool_calls.len(), 1);
    let tool_call = &tool_calls[0];
    assert_eq!(tool_call.call_id, "call_secret");
    assert_eq!(tool_call.tool_name, "leak_secret");
    assert!(tool_call
        .arguments
        .as_deref()
        .unwrap_or_default()
        .contains("[redacted-tool-call]"));
    assert!(
        !tool_call
            .arguments
            .as_deref()
            .unwrap_or_default()
            .contains("tool-call-secret"),
        "model_calls.tool_calls must store the redacted tool-call arguments"
    );
}

#[tokio::test]
async fn policy_v2_http_response_rewrite_strips_headers_before_guest_and_telemetry() {
    let (port, upstream_task) = spawn_http_fixture_response(
        302,
        "Found",
        vec![
            ("location", "https://github.com/openai/capsem?ref=secret"),
            ("set-cookie", "session=secret"),
            ("x-secret-token", "secret"),
        ],
        "redirecting",
    )
    .await;
    let host = format!("127.0.0.1:{port}");
    let config = make_config_with_policy_v2(
        allow_local_http_policy(port),
        policy_v2_from_toml(
            r#"
[policy.http.rewrite_response_location]
on = "http.response"
if = 'request.host == "127.0.0.1" && request.path == "/openai/capsem" && response.status == "302"'
decision = "rewrite"
priority = 10
reason = "Mirror redirect and strip response credentials"
rewrite_target = 'response.headers.location =~ "^https://github\.com/openai/(?P<repo>[^/?#]+)(?P<rest>.*)$"'
rewrite_value = "https://github.com/openclaw/${repo}${rest}"
strip_response_headers = ["Set-Cookie", "X-Secret-Token"]
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) = open_plain_http_proxy_conn(&config).await;

    let req = hyper::Request::builder()
        .method("GET")
        .uri("/openai/capsem")
        .header("host", host.as_str())
        .body(
            Full::new(Bytes::new())
                .map_err(|never| -> anyhow::Error { match never {} })
                .boxed(),
        )
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    let status = resp.status().as_u16();
    let location = resp
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let has_cookie = resp.headers().contains_key("set-cookie");
    let has_secret_header = resp.headers().contains_key("x-secret-token");
    let _ = resp.into_body().collect().await.unwrap();
    drop(sender);
    let _ = proxy_task.await;
    let upstream_request = upstream_task.await.unwrap();

    assert_eq!(status, 302);
    assert_eq!(
        location.as_deref(),
        Some("https://github.com/openclaw/capsem?ref=secret")
    );
    assert!(!has_cookie, "guest response must not include Set-Cookie");
    assert!(
        !has_secret_header,
        "guest response must not include stripped secret headers"
    );
    assert!(
        upstream_request.starts_with("GET /openai/capsem "),
        "proxy should still dispatch the original request upstream"
    );

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Allowed);
    assert_eq!(event.status_code, Some(302));
    assert_eq!(event.policy_action.as_deref(), Some("rewrite"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.http.rewrite_response_location")
    );
    let response_headers = event.response_headers.as_deref().unwrap_or_default();
    let rewritten_digest = blake3::hash(b"https://github.com/openclaw/capsem?ref=secret")
        .to_hex()
        .to_string();
    let original_digest = blake3::hash(b"https://github.com/openai/capsem?ref=secret")
        .to_hex()
        .to_string();
    let rewritten_location_marker = format!("location: hash:{}", &rewritten_digest[..12]);
    let original_location_marker = format!("location: hash:{}", &original_digest[..12]);
    assert!(
        response_headers.contains(&rewritten_location_marker),
        "response telemetry should contain the rewritten Location hash, got: {response_headers:?}"
    );
    assert!(
        !response_headers.contains("set-cookie")
            && !response_headers.contains("x-secret-token")
            && !response_headers.contains("session=secret")
            && !response_headers.contains(&original_location_marker),
        "response telemetry must reflect the stripped/re-written response head"
    );
}

#[tokio::test]
async fn policy_v2_http_response_bogus_rewrite_fails_closed_without_leaking_upstream_response() {
    let (port, upstream_task) = spawn_http_fixture_response(
        200,
        "OK",
        vec![("x-secret-token", "secret-header")],
        "super-secret-body",
    )
    .await;
    let host = format!("127.0.0.1:{port}");
    let config = make_config_with_policy_v2(
        allow_local_http_policy(port),
        policy_v2_from_toml(
            r#"
[policy.http.rewrite_response_body]
on = "http.response"
if = 'request.host == "127.0.0.1" && response.status == "200"'
decision = "rewrite"
priority = 10
reason = "Body rewrite is not supported on response heads"
rewrite_target = 'response.body =~ "super-secret-body"'
rewrite_value = "[redacted]"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) = open_plain_http_proxy_conn(&config).await;

    let req = hyper::Request::builder()
        .method("GET")
        .uri("/secret")
        .header("host", host.as_str())
        .body(
            Full::new(Bytes::new())
                .map_err(|never| -> anyhow::Error { match never {} })
                .boxed(),
        )
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    let status = resp.status().as_u16();
    let headers = format_headers(resp.headers());
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8_lossy(&body).into_owned();
    drop(sender);
    let _ = proxy_task.await;
    let _ = upstream_task.await.unwrap();

    assert_eq!(status, 403);
    assert!(
        !headers.contains("x-secret-token") && !headers.contains("secret-header"),
        "guest response headers must not leak the upstream response on fail-closed rewrite"
    );
    assert!(
        !body.contains("super-secret-body"),
        "guest response body must not leak upstream content on fail-closed rewrite"
    );

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Denied);
    assert_eq!(event.status_code, Some(403));
    assert_eq!(event.policy_action.as_deref(), Some("rewrite"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.http.rewrite_response_body")
    );
    assert!(
        !event
            .response_headers
            .as_deref()
            .unwrap_or_default()
            .contains("secret-header"),
        "fail-closed telemetry must not preserve upstream response headers"
    );
    assert!(
        !event
            .response_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("super-secret-body"),
        "fail-closed telemetry must not preserve upstream response body"
    );
}

#[tokio::test]
async fn policy_v2_http_block_stops_before_upstream_and_records_policy_fields() {
    let config = make_config_with_policy_v2(
        allow_test_domain_policy(),
        policy_v2_from_toml(&format!(
            r#"
[policy.http.block_openai_path]
on = "http.request"
if = 'request.host == "{TEST_DOMAIN}" && request.path.matches("^/openai(/|$)")'
decision = "block"
priority = 10
reason = "Do not fetch this path"
"#
        )),
    );
    let (mut sender, proxy_task, _conn_task) = open_proxy_conn(&config, TEST_DOMAIN).await;

    let status = send_get(&mut sender, TEST_DOMAIN, "/openai/capsem").await;
    assert_eq!(status, 403, "Policy V2 block should not reach upstream");
    drop(sender);
    let _ = proxy_task.await;

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Denied);
    assert_eq!(event.status_code, Some(403));
    assert_eq!(event.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(event.policy_action.as_deref(), Some("block"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.http.block_openai_path")
    );
    assert_eq!(
        event.policy_reason.as_deref(),
        Some("Do not fetch this path")
    );
}

#[tokio::test]
async fn policy_v2_http_ask_placeholder_confirmer_allows_upstream_dispatch() {
    let (port, upstream_task) =
        spawn_http_fixture_response(200, "OK", vec![("content-type", "text/plain")], "confirmed")
            .await;
    let config = make_config_with_policy_v2(
        allow_local_http_policy(port),
        policy_v2_from_toml(&format!(
            r#"
[policy.http.ask_openai_path]
on = "http.request"
if = 'request.host == "127.0.0.1" && request.path.matches("^/openai(/|$)")'
decision = "ask"
priority = 10
reason = "Ask before fetching this path"
"#
        )),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, None).await;

    let status = send_get(&mut sender, "127.0.0.1", "/openai/capsem").await;
    assert_eq!(
        status, 200,
        "placeholder-confirmed Policy V2 ask should dispatch upstream"
    );
    drop(sender);
    let _ = proxy_task.await;
    let upstream_request = upstream_task.await.unwrap();
    assert!(
        upstream_request.contains("GET /openai/capsem"),
        "ask accept path must reach upstream, got: {upstream_request}"
    );

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Allowed);
    assert_eq!(event.status_code, Some(200));
    assert_eq!(event.policy_action.as_deref(), Some("allow"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.http.ask_openai_path")
    );
}

#[tokio::test]
async fn policy_v2_http_rewrite_strips_request_headers_before_telemetry_and_upstream() {
    let config = make_config_with_policy_v2(
        allow_test_domain_policy(),
        policy_v2_from_toml(&format!(
            r#"
[policy.http.rewrite_openai_path]
on = "http.request"
if = 'request.host == "{TEST_DOMAIN}" && request.path.matches("^/openai/") && has(request.headers.authorization)'
decision = "rewrite"
priority = 10
reason = "Mirror path and strip credentials"
rewrite_target = 'request.url =~ "^https://{TEST_DOMAIN}/openai/(?P<repo>[^/?#]+)(?P<rest>.*)$"'
rewrite_value = "https://{TEST_DOMAIN}/openclaw/${{repo}}${{rest}}"
strip_request_headers = ["Authorization"]
"#
        )),
    );
    let (mut sender, proxy_task, _conn_task) = open_proxy_conn(&config, TEST_DOMAIN).await;

    let req = hyper::Request::builder()
        .method("GET")
        .uri("/openai/capsem?token=secret")
        .header("host", TEST_DOMAIN)
        .header("authorization", "Bearer secret")
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
        "rewrite should dispatch the rewritten request; the test domain then fails upstream"
    );
    let _ = resp.into_body().collect().await;
    drop(sender);
    let _ = proxy_task.await;

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Error);
    assert_eq!(event.path.as_deref(), Some("/openclaw/capsem"));
    assert_eq!(event.query.as_deref(), Some("token=secret"));
    assert_eq!(event.policy_action.as_deref(), Some("rewrite"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.http.rewrite_openai_path")
    );
    assert!(
        !event
            .request_headers
            .as_deref()
            .unwrap_or_default()
            .contains("authorization"),
        "stripped credential header must not appear in request telemetry"
    );
}

/// Disabling a provider mid-connection blocks subsequent requests on the
/// same keep-alive connection. This is the core regression test for the
/// per-request policy reload fix.
#[tokio::test]
async fn policy_hot_reload_blocks_on_same_connection() {
    use crate::net::policy::{DomainMatcher, PolicyRule};

    // Start with a policy that allows TEST_DOMAIN (read+write).
    let allow_policy = NetworkPolicy::new(
        vec![PolicyRule {
            matcher: DomainMatcher::parse(TEST_DOMAIN),
            allow_read: true,
            allow_write: true,
        }],
        false,
        false,
    );
    let config = make_config_with_policy(allow_policy);
    let (mut sender, proxy_task, _conn_task) = open_proxy_conn(&config, TEST_DOMAIN).await;

    // First request: allowed. Returns 502 because there's no real upstream,
    // but 502 proves the policy allowed the request past the policy check
    // (denied would be 403).
    let status1 = send_get(&mut sender, TEST_DOMAIN, "/before-disable").await;
    assert_eq!(
        status1, 502,
        "allowed request should reach upstream (502 = no upstream, not 403)"
    );

    // Hot-reload: swap to deny-all policy (simulates user disabling provider).
    let deny_policy = Arc::new(NetworkPolicy::new(vec![], false, false));
    *config.policy.write().unwrap() = deny_policy;

    // Second request on the SAME keep-alive connection: must be denied.
    let status2 = send_get(&mut sender, TEST_DOMAIN, "/after-disable").await;
    assert_eq!(
        status2, 403,
        "request after policy swap must be denied on same connection"
    );

    drop(sender);
    let _ = proxy_task.await;

    // Verify telemetry recorded both events with correct decisions.
    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let reader = config.db.reader().unwrap();
    let mut events = reader.recent_net_events(10).unwrap();
    assert_eq!(
        events.len(),
        2,
        "should have 2 events (one allowed, one denied)"
    );
    events.reverse(); // chronological
                      // First event: allowed (502 upstream error, but decision is Error not Denied).
    assert!(
        events[0].decision != Decision::Denied,
        "first request should not be denied, got {:?}",
        events[0].decision
    );
    assert_eq!(events[0].path, Some("/before-disable".to_string()));
    // Second event: denied (403).
    assert_eq!(events[1].decision, Decision::Denied);
    assert_eq!(events[1].path, Some("/after-disable".to_string()));
    assert_eq!(events[1].status_code, Some(403));
}

/// Re-enabling a provider mid-connection allows subsequent requests on
/// the same keep-alive connection (reverse direction of the above test).
#[tokio::test]
async fn policy_hot_reload_allows_on_same_connection() {
    use crate::net::policy::{DomainMatcher, PolicyRule};

    // Start with deny-all.
    let config = make_config_deny_all();
    let (mut sender, proxy_task, _conn_task) = open_proxy_conn(&config, TEST_DOMAIN).await;

    // First request: denied.
    let status1 = send_get(&mut sender, TEST_DOMAIN, "/while-denied").await;
    assert_eq!(status1, 403);

    // Hot-reload: swap to allow policy.
    let allow_policy = Arc::new(NetworkPolicy::new(
        vec![PolicyRule {
            matcher: DomainMatcher::parse(TEST_DOMAIN),
            allow_read: true,
            allow_write: true,
        }],
        false,
        false,
    ));
    *config.policy.write().unwrap() = allow_policy;

    // Second request: allowed (502 = no upstream, proves policy let it through).
    let status2 = send_get(&mut sender, TEST_DOMAIN, "/after-enable").await;
    assert_eq!(
        status2, 502,
        "request after re-enable should be allowed (502 = no upstream)"
    );

    drop(sender);
    let _ = proxy_task.await;
}

/// Multiple policy swaps on the same connection: deny -> allow -> deny.
/// Verifies each request sees the current policy, not any cached version.
#[tokio::test]
async fn policy_hot_reload_multiple_swaps() {
    use crate::net::policy::{DomainMatcher, PolicyRule};

    let config = make_config_deny_all();
    let (mut sender, proxy_task, _conn_task) = open_proxy_conn(&config, TEST_DOMAIN).await;

    // Request 1: denied.
    assert_eq!(send_get(&mut sender, TEST_DOMAIN, "/r1").await, 403);

    // Swap to allow.
    let allow = Arc::new(NetworkPolicy::new(
        vec![PolicyRule {
            matcher: DomainMatcher::parse(TEST_DOMAIN),
            allow_read: true,
            allow_write: true,
        }],
        false,
        false,
    ));
    *config.policy.write().unwrap() = allow;

    // Request 2: allowed (502).
    assert_eq!(send_get(&mut sender, TEST_DOMAIN, "/r2").await, 502);

    // Swap back to deny.
    let deny = Arc::new(NetworkPolicy::new(vec![], false, false));
    *config.policy.write().unwrap() = deny;

    // Request 3: denied again.
    assert_eq!(send_get(&mut sender, TEST_DOMAIN, "/r3").await, 403);

    drop(sender);
    let _ = proxy_task.await;

    // Verify all 3 events recorded.
    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let reader = config.db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(
        events.len(),
        3,
        "all 3 requests should produce telemetry events"
    );
}

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
