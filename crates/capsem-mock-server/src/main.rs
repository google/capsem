use anyhow::{Context, Result};
use base64::Engine as _;
use bytes::Bytes;
use clap::Parser;
use flate2::{write::GzEncoder, Compression};
use futures::future;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{
    CONNECTION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, SEC_WEBSOCKET_ACCEPT,
    SEC_WEBSOCKET_KEY, UPGRADE,
};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::upgrade::Upgraded;
use hyper::{HeaderMap, Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::Serialize;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::convert::Infallible;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio_rustls::TlsAcceptor;

const TINY_BODY: &[u8] = b"capsem-mock-server:tiny\n";
const EXPECTED_POEM: &str = "Capsem ironbank poem\nledgers count the sparks\nno secret crosses raw";
const OLLAMA_OPENAI_TOOL_CALL_ID: &str = "call_fm3e3d2f";
const OLLAMA_OPENAI_TOOL_ARGUMENTS: &str = "{\"query\":\"Capsem ironbank poem\"}";

const ENDPOINTS: &[&str] = &[
    "/tiny",
    "/html/about",
    "/html/large",
    "/bytes/{size}",
    "/gzip/{size}",
    "/sse/model",
    "/model/response",
    "/model/shape",
    "/model/no-tool-call",
    "/v1beta/models/gemini-3.5-flash:streamGenerateContent",
    "/v1/chat/completions",
    "/v1/embeddings",
    "/v1/images/generations",
    "/v1/responses",
    "/v1/messages",
    "/v1internal:listExperiments",
    "/v1internal:loadCodeAssist",
    "/v1internal:fetchAvailableModels",
    "/v1internal:streamGenerateContent",
    "/api/chat",
    "/api/show",
    "/api/tags",
    "/oauth/authorize",
    "/oauth/token",
    "/mcp",
    "/chunked",
    "/delayed-chunks",
    "/credential/response",
    "/echo",
    "/deny-target",
    "/ws/echo",
    "/ws/ping",
    "/ws/close",
];

const DNS_FIXTURES: &[&str] = &[
    "fixture.capsem.test",
    "model.capsem.test",
    "mcp.capsem.test",
    "api.openai.com",
    "api.anthropic.com",
    "daily-cloudcode-pa.googleapis.com",
    "generativelanguage.googleapis.com",
    "www.googleapis.com",
    "play.googleapis.com",
    "antigravity-unleash.goog",
];

type RespBody = BoxBody<Bytes, Infallible>;

#[derive(Clone)]
struct LogBody(Bytes);

#[derive(Parser, Debug)]
#[command(about = "Hermetic Capsem mock upstream server")]
struct Args {
    #[arg(long, default_value = "127.0.0.1:0")]
    addr: SocketAddr,
    #[arg(long)]
    request_log: Option<PathBuf>,
}

#[derive(Clone)]
struct State {
    request_log: Option<Arc<Mutex<File>>>,
}

struct DnsExchange {
    qname: String,
    qtype: u16,
    qclass: u16,
    rcode: u8,
    request_bytes: usize,
    response_bytes: usize,
}

#[derive(Serialize)]
struct ReadyPayload {
    service: &'static str,
    http_addr: String,
    base_url: String,
    https_addr: String,
    https_base_url: String,
    dns_udp_addr: String,
    dns_tcp_addr: String,
    dns_fixtures: Vec<&'static str>,
    endpoints: Vec<&'static str>,
    request_log: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "capsem_mock_server=warn".to_string()),
        )
        .with_writer(std::io::stderr)
        .init();
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let args = Args::parse();
    let request_log = match &args.request_log {
        Some(path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }
            Some(Arc::new(Mutex::new(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .with_context(|| format!("open {}", path.display()))?,
            )))
        }
        None => None,
    };
    let state = State { request_log };

    let http_listener = TcpListener::bind(args.addr).await.context("bind HTTP")?;
    let http_addr = http_listener.local_addr().context("read HTTP addr")?;
    let https_listener = TcpListener::bind((args.addr.ip(), 0))
        .await
        .context("bind HTTPS")?;
    let https_addr = https_listener.local_addr().context("read HTTPS addr")?;
    let dns_udp_socket = UdpSocket::bind((args.addr.ip(), 0))
        .await
        .context("bind DNS UDP")?;
    let dns_udp_addr = dns_udp_socket.local_addr().context("read DNS UDP addr")?;
    let dns_tcp_listener = TcpListener::bind((args.addr.ip(), 0))
        .await
        .context("bind DNS TCP")?;
    let dns_tcp_addr = dns_tcp_listener.local_addr().context("read DNS TCP addr")?;

    let tls_acceptor = tls_acceptor()?;

    let mut dns_fixtures = DNS_FIXTURES.to_vec();
    dns_fixtures.sort_unstable();
    let ready = ReadyPayload {
        service: "capsem-mock-server",
        http_addr: http_addr.to_string(),
        base_url: format!("http://{http_addr}"),
        https_addr: https_addr.to_string(),
        https_base_url: format!("https://{https_addr}"),
        dns_udp_addr: dns_udp_addr.to_string(),
        dns_tcp_addr: dns_tcp_addr.to_string(),
        dns_fixtures,
        endpoints: ENDPOINTS.to_vec(),
        request_log: args
            .request_log
            .as_ref()
            .map(|path| path.display().to_string()),
    };
    println!("{}", serde_json::to_string(&ready)?);

    tokio::spawn(serve_http(http_listener, state.clone(), false));
    tokio::spawn(serve_https(https_listener, state.clone(), tls_acceptor));
    tokio::spawn(serve_dns_udp(dns_udp_socket, state.clone()));
    tokio::spawn(serve_dns_tcp(dns_tcp_listener, state.clone()));

    future::pending::<()>().await;
    #[allow(unreachable_code)]
    Ok(())
}

async fn serve_http(listener: TcpListener, state: State, tls: bool) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let state = state.clone();
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service = service_fn(move |req| handle_request(req, state.clone(), tls));
            if let Err(err) = http1::Builder::new()
                .keep_alive(true)
                .half_close(true)
                .serve_connection(io, service)
                .with_upgrades()
                .await
            {
                tracing::debug!(error = %err, "mock HTTP connection ended");
            }
        });
    }
}

async fn serve_https(listener: TcpListener, state: State, acceptor: TlsAcceptor) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let acceptor = acceptor.clone();
        let state = state.clone();
        tokio::spawn(async move {
            let Ok(stream) = acceptor.accept(stream).await else {
                return;
            };
            let io = TokioIo::new(stream);
            let service = service_fn(move |req| handle_request(req, state.clone(), true));
            if let Err(err) = http1::Builder::new()
                .keep_alive(true)
                .half_close(true)
                .serve_connection(io, service)
                .with_upgrades()
                .await
            {
                tracing::debug!(error = %err, "mock HTTPS connection ended");
            }
        });
    }
}

fn tls_acceptor() -> Result<TlsAcceptor> {
    let cert = generate_simple_self_signed(vec!["127.0.0.1".to_string(), "localhost".to_string()])
        .context("generate self-signed mock cert")?;
    let cert_der = CertificateDer::from(cert.cert.der().to_vec());
    let key_der = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()));
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .context("build mock TLS config")?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

async fn handle_request(
    mut req: Request<Incoming>,
    state: State,
    tls: bool,
) -> Result<Response<RespBody>, Infallible> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    if path.starts_with("/ws/") {
        let response = handle_ws(req, path).await;
        return Ok(response);
    }

    let headers = req.headers().clone();
    let query = req.uri().query().map(str::to_owned);
    let request_body = req
        .body_mut()
        .collect()
        .await
        .map(|body| body.to_bytes())
        .unwrap_or_default();
    let response = route(
        &method,
        &path,
        query.as_deref(),
        &headers,
        request_body.clone(),
        tls,
    )
    .await;
    log_request(
        &state,
        &method,
        &path,
        query.as_deref(),
        &headers,
        &request_body,
        &response,
    );
    Ok(response)
}

async fn route(
    method: &Method,
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    request_body: Bytes,
    _tls: bool,
) -> Response<RespBody> {
    match (method, path) {
        (&Method::HEAD, "/") => response(StatusCode::OK, Bytes::new(), "text/plain; charset=utf-8"),
        (&Method::HEAD, "/tiny") => {
            response_with_len(StatusCode::OK, Bytes::new(), "text/plain; charset=utf-8", TINY_BODY.len())
        }
        (&Method::GET, "/") => response(StatusCode::OK, Bytes::new(), "text/plain; charset=utf-8"),
        (&Method::GET, "/tiny") => response(
            StatusCode::OK,
            Bytes::from_static(TINY_BODY),
            "text/plain; charset=utf-8",
        ),
        (&Method::GET, "/html/about") => response(
            StatusCode::OK,
            Bytes::from_static(
                b"<html><body><main><h1>Capsem mock server about page</h1><p>Google fixture content for local MCP extraction.</p></main></body></html>\n",
            ),
            "text/html; charset=utf-8",
        ),
        (&Method::GET, "/html/large") => {
            let body = format!(
                "<html><body>{}</body></html>\n",
                "capsem-large-html ".repeat(4096)
            );
            response(StatusCode::OK, Bytes::from(body), "text/html; charset=utf-8")
        }
        (&Method::GET, "/sse/model") => response(
            StatusCode::OK,
            Bytes::from_static(
                b"event: model.delta\ndata: {\"delta\":\"hello\"}\n\n\
event: model.tool_call\ndata: {\"name\":\"write_file\",\"arguments\":{\"path\":\"/root/poem.md\"}}\n\n\
event: model.tool_call\ndata: {\"name\":\"fixture_lookup\",\"arguments\":{\"query\":\"capsem\"}}\n\n\
event: model.done\ndata: {\"finish_reason\":\"stop\"}\n\n",
            ),
            "text/event-stream",
        ),
        (&Method::GET, "/model/response") => json_response(json!({
            "id": "mock-model-response",
            "model": "mock-local",
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {
                            "type": "output_text",
                            "text": EXPECTED_POEM
                        }
                    ]
                },
                {
                    "type": "function_call",
                    "call_id": "call_mock_model_response",
                    "name": "write_file",
                    "arguments": "{\"path\":\"/root/poem.md\",\"content\":\"Capsem ironbank poem\"}"
                }
            ],
            "output_text": EXPECTED_POEM,
            "tool_calls": [
                {
                    "id": "call_mock_model_response",
                    "type": "function",
                    "function": {
                        "name": "write_file",
                        "arguments": "{\"path\":\"/root/poem.md\",\"content\":\"Capsem ironbank poem\"}"
                    }
                }
            ]
        })),
        (&Method::GET, "/oauth/authorize") => json_response(json!({
            "kind": "synthetic_oauth_authorization_fixture",
            "authorization_code": "capsem_test_oauth_code_0123456789abcdef",
            "redirect_uri": "https://capsem.invalid/oauth/callback",
        })),
        (&Method::GET, "/api/client/features") => json_response(json!({"features": []})),
        (&Method::GET, "/credential/response") => json_response(json!({
            "kind": "synthetic_credential_fixture",
            "api_key": "sk-capsem_test_api_key_0123456789abcdef",
            "oauth": {
                "access_token": "capsem_test_oauth_access_0123456789abcdef",
                "refresh_token": "capsem_test_oauth_refresh_0123456789abcdef",
            }
        })),
        (&Method::GET, "/api/tags") => json_response(json!({
            "models": [{
                "name": "gemma4:latest",
                "model": "gemma4:latest",
                "modified_at": "2026-06-01T00:00:00Z",
                "size": 7_000_000_000_u64,
                "details": {"family": "gemma", "parameter_size": "7B"}
            }]
        })),
        (&Method::GET, "/oauth2/v2/userinfo") => json_response(json!({
            "email": "capsem@example.invalid",
            "verified_email": true,
        })),
        (&Method::GET, "/deny-target") => response(
            StatusCode::OK,
            Bytes::from_static(b"capsem-mock-server:deny-target\n"),
            "text/plain",
        ),
        (&Method::GET, "/chunked") | (&Method::GET, "/delayed-chunks") => response(
            StatusCode::OK,
            Bytes::from_static(b"chunk-0\nchunk-1\nchunk-2\nchunk-3\n"),
            "text/plain; charset=utf-8",
        ),
        (&Method::GET, _) if path.starts_with("/bytes/") => {
            let size = path.trim_start_matches("/bytes/");
            match deterministic_bytes(size) {
                Some(bytes) => response(StatusCode::OK, bytes, "application/octet-stream"),
                None => response(StatusCode::NOT_FOUND, Bytes::new(), "text/plain"),
            }
        }
        (&Method::GET, _) if path.starts_with("/gzip/") => {
            let size = path.trim_start_matches("/gzip/");
            match deterministic_gzip(size) {
                Some(bytes) => response_with_header(
                    StatusCode::OK,
                    bytes,
                    "application/octet-stream",
                    CONTENT_ENCODING.as_str(),
                    "gzip",
                ),
                None => response(StatusCode::NOT_FOUND, Bytes::new(), "text/plain"),
            }
        }
        (&Method::POST, "/v1/chat/completions") => {
            let payload = parse_json(&request_body);
            if payload.get("stream").and_then(Value::as_bool) == Some(true) {
                response(StatusCode::OK, openai_chat_stream(), "text/event-stream")
            } else {
                json_response(openai_chat_response(payload))
            }
        }
        (&Method::POST, "/v1/embeddings") => json_response(json!({
            "object": "list",
            "data": [{"object": "embedding", "index": 0, "embedding": [0.125, -0.25, 0.5, 0.75]}],
            "model": "text-embedding-3-small",
            "usage": {"prompt_tokens": 9, "total_tokens": 9}
        })),
        (&Method::POST, "/v1/images/generations") => json_response(json!({
            "created": now_unix(),
            "data": [{"b64_json": "Y2Fwc2VtLW1vY2staW1hZ2U="}],
            "usage": {"input_tokens": 11, "output_tokens": 17, "total_tokens": 28}
        })),
        (&Method::POST, "/v1/responses") => {
            let payload = parse_json(&request_body);
            if payload.get("stream").and_then(Value::as_bool) == Some(true) {
                response(
                    StatusCode::OK,
                    responses_stream(&payload, payload_has_function_call_output(&payload)),
                    "text/event-stream",
                )
            } else {
                json_response(responses_response(payload))
            }
        }
        (&Method::POST, "/model/shape") => json_response(json!({
            "id": "chatcmpl_shape_fixture",
            "object": "chat.completion",
            "model": parse_json(&request_body).get("model").and_then(Value::as_str).unwrap_or("gpt-4.1"),
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": EXPECTED_POEM,
                    "tool_calls": [{
                        "id": OLLAMA_OPENAI_TOOL_CALL_ID,
                        "type": "function",
                        "function": {
                            "name": "fixture_lookup",
                            "arguments": "{\"query\":\"Capsem ironbank poem\"}"
                        }
                    }]
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 66, "completion_tokens": 390, "total_tokens": 456}
        })),
        (&Method::POST, "/model/no-tool-call") => json_response(json!({
            "id": "chatcmpl_no_tool_fixture",
            "object": "chat.completion",
            "model": parse_json(&request_body).get("model").and_then(Value::as_str).unwrap_or("gpt-4.1"),
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": EXPECTED_POEM
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 26, "completion_tokens": 52, "total_tokens": 78}
        })),
        (&Method::POST, "/v1/messages") => {
            let payload = parse_json(&request_body);
            if payload.get("stream").and_then(Value::as_bool) == Some(true) {
                response(StatusCode::OK, anthropic_stream(payload), "text/event-stream")
            } else {
                json_response(anthropic_response(payload))
            }
        }
        (&Method::POST, "/api/chat") => {
            let payload = parse_json(&request_body);
            json_response(json!({
                "model": payload.get("model").and_then(Value::as_str).unwrap_or("gemma4:latest"),
                "created_at": "2026-06-01T00:00:00Z",
                "message": {"role": "assistant", "content": EXPECTED_POEM},
                "done": true,
                "prompt_eval_count": 32,
                "eval_count": 24
            }))
        }
        (&Method::POST, "/api/show") => json_response(json!({
            "modelfile": "FROM gemma4:latest",
            "details": {"family": "gemma", "parameter_size": "7B"}
        })),
        (&Method::POST, "/oauth/token") => json_response(json!({
            "kind": "synthetic_oauth_token_fixture",
            "access_token": "capsem_test_oauth_access_0123456789abcdef",
            "refresh_token": "capsem_test_oauth_refresh_0123456789abcdef",
            "id_token": "capsem_test_oauth_id_0123456789abcdef",
            "token_type": "Bearer",
            "expires_in": 3600,
            "scope": "openid profile email offline_access"
        })),
        (&Method::POST, "/log") => response(StatusCode::OK, Bytes::new(), "text/plain; charset=UTF-8"),
        (&Method::POST, "/api/client/register")
        | (&Method::POST, "/api/client/metrics") => {
            response(StatusCode::ACCEPTED, Bytes::new(), "application/json")
        }
        (&Method::POST, "/api/client/features") => json_response(json!({"version": 1, "features": []})),
        (&Method::POST, "/mcp") => {
            let payload = parse_json(&request_body);
            if mcp_payload_should_delay(&payload) {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            json_response(mcp_response(payload))
        }
        (&Method::POST, "/echo") => json_response(echo_response(query, headers, request_body.len())),
        (&Method::POST, _) if path.starts_with("/v1internal") => {
            if path == "/v1internal:streamGenerateContent" {
                response(
                    StatusCode::OK,
                    google_code_assist_stream(parse_json(&request_body)),
                    "text/event-stream",
                )
            } else if path == "/v1internal:listExperiments" {
                json_response(load_google_fixture("list_experiments.json").unwrap_or_else(|_| {
                    json!({"experimentIds": [], "flags": []})
                }))
            } else if path == "/v1internal:loadCodeAssist" {
                json_response(load_google_fixture("load_code_assist.json").unwrap_or_else(|_| {
                    json!({
                        "currentTier": {"id": "free-tier"},
                        "cloudaicompanionProject": "capsem-mock-project",
                        "allowedTiers": []
                    })
                }))
            } else if path == "/v1internal:fetchAvailableModels" {
                json_response(load_google_fixture("available_models.json").unwrap_or_else(|_| {
                    json!({"models": {}, "defaultAgentModelId": "gemini-3.5-flash-low"})
                }))
            } else if path == "/v1internal:fetchUserInfo" {
                json_response(json!({"userSettings": {"telemetryEnabled": false}, "regionCode": "US"}))
            } else if path == "/v1internal:retrieveUserQuotaSummary" {
                json_response(load_google_fixture("quota_summary.json").unwrap_or_else(|_| {
                    json!({"groups": []})
                }))
            } else if path == "/v1internal:setUserSettings" {
                json_response(json!({"userSettings": {"telemetryEnabled": false}}))
            } else if path == "/v1internal:fetchAdminControls" {
                json_response(json!({}))
            } else {
                json_response(json!({"ok": true}))
            }
        }
        (&Method::POST, _) if path.ends_with(":streamGenerateContent") => response(
            StatusCode::OK,
            gemini_api_stream(parse_json(&request_body), google_model_from_path(path)),
            "text/event-stream",
        ),
        (&Method::POST, _) if path.ends_with(":generateContent") => {
            json_response(gemini_api_response(
                parse_json(&request_body),
                google_model_from_path(path),
            ))
        }
        _ => response(StatusCode::NOT_FOUND, Bytes::new(), "text/plain"),
    }
}

fn parse_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or_else(|_| json!({}))
}

fn request_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn echo_response(query: Option<&str>, headers: &HeaderMap, body_size: usize) -> Value {
    let authorization = request_header(headers, "authorization").unwrap_or("");
    let query = query.unwrap_or("");
    json!({
        "method": "POST",
        "path": "/echo",
        "body_size": body_size,
        "content_type": request_header(headers, "content-type"),
        "user_agent": request_header(headers, "user-agent"),
        "header_count": headers.len(),
        "has_authorization": headers.contains_key("authorization"),
        "authorization_is_broker_ref": authorization.contains("credential:blake3:"),
        "query_has_broker_ref": query.contains("credential:blake3:"),
        "query_has_access_token": query.contains("access_token="),
        "has_cookie": headers.contains_key("cookie"),
        "has_x_api_key": headers.contains_key("x-api-key"),
    })
}

fn json_response(value: Value) -> Response<RespBody> {
    response(
        StatusCode::OK,
        Bytes::from(serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec())),
        "application/json",
    )
}

fn response(status: StatusCode, body: Bytes, content_type: &str) -> Response<RespBody> {
    let log_body = body.clone();
    let mut response = response_builder(status, body.len(), content_type)
        .body(full(body))
        .expect("build response");
    response.extensions_mut().insert(LogBody(log_body));
    response
}

fn response_with_len(
    status: StatusCode,
    body: Bytes,
    content_type: &str,
    content_length: usize,
) -> Response<RespBody> {
    let log_body = body.clone();
    let mut response = response_builder(status, content_length, content_type)
        .body(full(body))
        .expect("build response");
    response.extensions_mut().insert(LogBody(log_body));
    response
}

fn response_with_header(
    status: StatusCode,
    body: Bytes,
    content_type: &str,
    header_name: &str,
    header_value: &str,
) -> Response<RespBody> {
    let log_body = body.clone();
    let mut response = response_builder(status, body.len(), content_type)
        .header(header_name, header_value)
        .body(full(body))
        .expect("build response");
    response.extensions_mut().insert(LogBody(log_body));
    response
}

fn response_builder(
    status: StatusCode,
    content_length: usize,
    content_type: &str,
) -> hyper::http::response::Builder {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .header(CONTENT_LENGTH, content_length.to_string())
}

fn full(body: Bytes) -> RespBody {
    Full::new(body).boxed()
}

fn deterministic_bytes(size: &str) -> Option<Bytes> {
    static TEN_KB: OnceLock<Bytes> = OnceLock::new();
    static ONE_MB: OnceLock<Bytes> = OnceLock::new();
    static TEN_MB: OnceLock<Bytes> = OnceLock::new();
    match size {
        "10kb" => Some(TEN_KB.get_or_init(|| build_bytes(10 * 1024)).clone()),
        "1mb" => Some(ONE_MB.get_or_init(|| build_bytes(1024 * 1024)).clone()),
        "10mb" => Some(TEN_MB.get_or_init(|| build_bytes(10 * 1024 * 1024)).clone()),
        _ => size.parse::<usize>().ok().map(build_bytes),
    }
}

fn deterministic_gzip(size: &str) -> Option<Bytes> {
    static TEN_KB: OnceLock<Bytes> = OnceLock::new();
    static ONE_MB: OnceLock<Bytes> = OnceLock::new();
    static TEN_MB: OnceLock<Bytes> = OnceLock::new();
    match size {
        "10kb" => Some(
            TEN_KB
                .get_or_init(|| gzip_bytes(&build_bytes(10 * 1024)))
                .clone(),
        ),
        "1mb" => Some(
            ONE_MB
                .get_or_init(|| gzip_bytes(&build_bytes(1024 * 1024)))
                .clone(),
        ),
        "10mb" => Some(
            TEN_MB
                .get_or_init(|| gzip_bytes(&build_bytes(10 * 1024 * 1024)))
                .clone(),
        ),
        _ => size
            .parse::<usize>()
            .ok()
            .map(|len| gzip_bytes(&build_bytes(len))),
    }
}

fn build_bytes(len: usize) -> Bytes {
    let bytes = (0..len)
        .map(|idx| b'a' + u8::try_from(idx % 26).expect("modulo fits in u8"))
        .collect::<Vec<_>>();
    Bytes::from(bytes)
}

fn gzip_bytes(bytes: &[u8]) -> Bytes {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).expect("write gzip fixture");
    Bytes::from(encoder.finish().expect("finish gzip fixture"))
}

fn openai_chat_response(payload: Value) -> Value {
    let model = payload
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("mock-local");
    let include_tool_call =
        payload.get("tools").is_some() || is_baked_doctor_openai_smoke(&payload);
    let message = if include_tool_call {
        json!({
            "role": "assistant",
            "content": "",
            "reasoning": "Deterministic local Ollama-compatible fixture reasoning.",
            "tool_calls": [{
                "id": OLLAMA_OPENAI_TOOL_CALL_ID,
                "index": 0,
                "type": "function",
                "function": {
                    "name": "fixture_lookup",
                    "arguments": OLLAMA_OPENAI_TOOL_ARGUMENTS
                }
            }]
        })
    } else {
        json!({
            "role": "assistant",
            "content": EXPECTED_POEM,
            "reasoning": "Deterministic local Ollama-compatible fixture reasoning."
        })
    };
    json!({
        "id": if include_tool_call { "chatcmpl-601" } else { "chatcmpl-515" },
        "object": "chat.completion",
        "created": if include_tool_call { 1781444656_u64 } else { 1781444596_u64 },
        "model": model,
        "system_fingerprint": "fp_ollama",
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": if include_tool_call { "tool_calls" } else { "stop" }
        }],
        "usage": if include_tool_call {
            json!({"prompt_tokens": 66, "completion_tokens": 390, "total_tokens": 456})
        } else {
            json!({"prompt_tokens": 26, "completion_tokens": 52, "total_tokens": 78})
        }
    })
}

fn is_baked_doctor_openai_smoke(payload: &Value) -> bool {
    if payload.get("model").and_then(Value::as_str) != Some("mock-local") {
        return false;
    }
    let Some(messages) = payload.get("messages").and_then(Value::as_array) else {
        return false;
    };
    if messages.len() != 1 {
        return false;
    }
    messages[0].get("role").and_then(Value::as_str) == Some("user")
        && messages[0].get("content").and_then(Value::as_str) == Some("hello")
}

fn openai_chat_stream() -> Bytes {
    Bytes::from_static(
        b"data: {\"id\":\"chatcmpl_capsem_mock\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"delta\":{\"role\":\"assistant\"},\"index\":0}]}\n\n\
data: {\"id\":\"chatcmpl_capsem_mock\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"delta\":{\"content\":\"Capsem ironbank poem\"},\"index\":0}]}\n\n\
data: [DONE]\n\n",
    )
}

fn responses_response(payload: Value) -> Value {
    let model = payload
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("mock-local");
    let has_tool_output = serde_json::to_string(&payload)
        .map(|raw| raw.contains("function_call_output"))
        .unwrap_or(false);
    if !has_tool_output {
        return json!({
            "id": "resp_ironbank_tool_01",
            "object": "response",
            "created_at": 1781205836_u64,
            "status": "completed",
            "model": model,
            "output": [{
                "id": "fc_codex_write_poem",
                "type": "function_call",
                "status": "completed",
                "call_id": "call_codex_write_poem",
                "name": "exec_command",
                "arguments": "{\"cmd\":\"printf '%s\\\\n' 'Capsem ironbank poem\\\\nledgers count the sparks\\\\nno secret crosses raw' > /root/codex-cli-output.txt\",\"yield_time_ms\":1000,\"max_output_tokens\":2000}"
            }],
            "usage": {"input_tokens": 31, "output_tokens": 17, "total_tokens": 48}
        });
    }
    json!({
        "id": "resp_ironbank_01",
        "object": "response",
        "created_at": 1781205836_u64,
        "status": "completed",
        "model": model,
        "output": [{
            "id": "msg_ironbank_01",
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "content": [{"type": "output_text", "text": EXPECTED_POEM, "annotations": []}]
        }],
        "output_text": EXPECTED_POEM,
        "usage": {
            "input_tokens": 7,
            "output_tokens": 5,
            "total_tokens": 12,
            "output_tokens_details": {"reasoning_tokens": 2}
        }
    })
}

fn payload_has_function_call_output(payload: &Value) -> bool {
    payload
        .get("input")
        .and_then(Value::as_array)
        .map(|items| {
            items.iter().any(|item| {
                item.get("type").and_then(Value::as_str) == Some("function_call_output")
            })
        })
        .unwrap_or(false)
}

fn responses_stream(payload: &Value, final_turn: bool) -> Bytes {
    if final_turn {
        let (token, _) = write_target(payload, "openai-responses");
        return Bytes::from(
            format!(
                "event: response.reasoning_summary_text.delta\ndata: {{\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"ledger reasoning\"}}\n\n\
event: response.output_text.delta\ndata: {{\"type\":\"response.output_text.delta\",\"delta\":\"{token}\"}}\n\n\
event: response.output_text.done\ndata: {{\"type\":\"response.output_text.done\",\"text\":\"{token}\"}}\n\n\
event: response.completed\ndata: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"resp_capsem_mock\",\"status\":\"completed\",\"model\":\"gpt-5-nano\",\"usage\":{{\"input_tokens\":7,\"output_tokens\":5,\"total_tokens\":12,\"output_tokens_details\":{{\"reasoning_tokens\":2}}}}}}}}\n\n"
            ),
        );
    }
    let (token, path) = write_target(payload, "openai-responses");
    let call_id = format!("call_{}", &token[..token.len().min(12)]);
    let arguments = json_compact(json!({
        "cmd": shell_write_command(&token, &path),
        "yield_time_ms": 1000,
        "max_output_tokens": 2000
    }));
    let arguments_json = json_compact(json!(arguments));
    Bytes::from(format!(
        "event: response.output_item.added\ndata: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"reasoning\",\"id\":\"rs_capsem_mock\"}}}}\n\n\
event: response.output_item.added\ndata: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"function_call\",\"id\":\"fc_capsem_mock\",\"call_id\":\"{call_id}\",\"name\":\"exec_command\",\"arguments\":{arguments_json}}}}}\n\n\
event: response.function_call_arguments.delta\ndata: {{\"type\":\"response.function_call_arguments.delta\",\"delta\":{arguments_json}}}\n\n\
event: response.function_call_arguments.done\ndata: {{\"type\":\"response.function_call_arguments.done\",\"arguments\":{arguments_json}}}\n\n\
event: response.output_item.done\ndata: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"function_call\",\"id\":\"fc_capsem_mock\",\"status\":\"completed\",\"call_id\":\"{call_id}\",\"name\":\"exec_command\",\"arguments\":{arguments_json}}}}}\n\n\
event: response.completed\ndata: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"resp_capsem_mock\",\"status\":\"completed\",\"model\":\"gpt-5-nano\",\"usage\":{{\"input_tokens\":31,\"output_tokens\":17,\"total_tokens\":48,\"output_tokens_details\":{{\"reasoning_tokens\":2}}}}}}}}\n\n"
    ))
}

fn anthropic_response(payload: Value) -> Value {
    let has_tool_result = serde_json::to_string(&payload)
        .map(|raw| raw.contains("\"type\":\"tool_result\""))
        .unwrap_or(false);
    let (token, path) = write_target(&payload, "claude");
    let model = payload
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("claude-sonnet-4-6");
    if has_tool_result {
        return json!({
            "id": "msg_capsem_mock_final",
            "type": "message",
            "role": "assistant",
            "model": model,
            "content": [
                {"type": "thinking", "thinking": "ledger reasoning"},
                {"type": "text", "text": token}
            ],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 7, "output_tokens": 17}
        });
    }

    let command = shell_write_command(&token, &path);
    json!({
        "id": "msg_capsem_mock",
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": [
            {"type": "thinking", "thinking": "Plan file write"},
            {"type": "tool_use", "id": "toolu_capsem_write_poem", "name": "exec_command", "input": {"cmd": command}},
            {"type": "text", "text": token}
        ],
        "stop_reason": "tool_use",
        "usage": {"input_tokens": 33, "output_tokens": 27}
    })
}

fn anthropic_stream(payload: Value) -> Bytes {
    let has_tool_result = serde_json::to_string(&payload)
        .map(|raw| raw.contains("\"type\":\"tool_result\""))
        .unwrap_or(false);
    if has_tool_result {
        let (token, _) = write_target(&payload, "claude");
        let message = json!({
            "id": "msg_ironbank_final",
            "type": "message",
            "role": "assistant",
            "model": payload.get("model").and_then(Value::as_str).unwrap_or("claude-sonnet-4-6"),
            "content": [],
            "usage": {"input_tokens": 7, "output_tokens": 1}
        });
        return Bytes::from(format!(
            "event: message_start\ndata: {}\n\n\
event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"thinking\",\"thinking\":\"\"}}}}\n\n\
event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"thinking_delta\",\"thinking\":\"ledger reasoning\"}}}}\n\n\
event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\n\n\
event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":1,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\n\
event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":1,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{token}\"}}}}\n\n\
event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\"}},\"usage\":{{\"output_tokens\":17}}}}\n\n\
event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n",
            json_compact(json!({"type": "message_start", "message": message}))
        ));
    }
    if payload.get("tools").is_none() {
        let model = payload
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("claude-sonnet-4-6");
        return Bytes::from(format!(
            "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg_ironbank_stream_text\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"{model}\",\"content\":[],\"usage\":{{\"input_tokens\":25,\"output_tokens\":5}}}}}}\n\n\
event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\n\
event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"Hello \"}}}}\n\n\
event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"world!\"}}}}\n\n\
event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\n\n\
event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\"}},\"usage\":{{\"output_tokens\":5}}}}\n\n\
event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"
        ));
    }

    let (token, path) = write_target(&payload, "claude");
    let command = shell_write_command(&token, &path);
    let partial = json_compact(json!({
        "command": command,
        "description": "write ironbank token"
    }));
    let model = payload
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("claude-sonnet-4-6");
    Bytes::from(format!(
        "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg_ironbank_01\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"{model}\",\"content\":[],\"usage\":{{\"input_tokens\":31,\"output_tokens\":1}}}}}}\n\n\
event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"tool_use\",\"id\":\"toolu_capsem_write_poem\",\"name\":\"Bash\",\"input\":{{}}}}}}\n\n\
event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"input_json_delta\",\"partial_json\":{}}}}}\n\n\
event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\n\n\
event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"tool_use\",\"stop_sequence\":null}},\"usage\":{{\"output_tokens\":17}}}}\n\n\
event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n",
        json_compact(json!(partial))
    ))
}

fn load_google_fixture(name: &str) -> Result<Value> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/protocols/google_code_assist")
        .join(name);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

fn google_code_assist_stream(payload: Value) -> Bytes {
    if payload.get("requestType").and_then(Value::as_str) == Some("checkpoint") {
        let model = payload
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("gemini-3.1-flash-lite");
        let response = json!({
            "response": {
                "candidates": [{
                    "content": {"parts": [{"text": "Write Proof"}], "role": "model"},
                    "finishReason": "STOP"
                }],
                "modelVersion": model,
                "responseId": "agy_checkpoint"
            },
            "traceId": "trace_checkpoint",
            "metadata": {}
        });
        return Bytes::from(format!("data: {}\n\n", json_compact(response)));
    }

    let (token, path) = write_target(&payload, "agy");
    let call_id = format!("call_{}", &token[..token.len().min(12)]);
    let response_id = format!("agy_{}", &token[..token.len().min(12)]);
    let trace_id = format!("trace_{}", &token[..token.len().min(12)]);
    let has_function_response = serde_json::to_string(&payload)
        .map(|raw| raw.contains("functionResponse"))
        .unwrap_or(false);
    if has_function_response {
        let final_response = json!({
            "response": {
                "candidates": [{
                    "content": {"parts": [{"text": token}], "role": "model"},
                    "finishReason": "STOP"
                }],
                "usageMetadata": {
                    "promptTokenCount": 31,
                    "candidatesTokenCount": 17,
                    "thoughtsTokenCount": 2,
                    "totalTokenCount": 50
                },
                "modelVersion": "gemini-3.5-flash-low",
                "responseId": response_id
            },
            "traceId": trace_id,
            "metadata": {}
        });
        return Bytes::from(format!("data: {}\n\n", json_compact(final_response)));
    }
    let first = json!({
        "response": {
            "candidates": [{
                "content": {
                    "parts": [{
                        "thoughtSignature": "capsem-agy-fixture-signature",
                        "functionCall": {
                            "name": "run_command",
                            "id": call_id,
                            "args": {
                                "CommandLine": shell_write_command(&token, &path),
                                "Cwd": "/root",
                                "WaitMsBeforeAsync": 1000,
                                "toolSummary": "Write proof",
                                "toolAction": "Writing file"
                            }
                        }
                    }],
                    "role": "model"
                }
            }],
            "usageMetadata": {
                "promptTokenCount": 31,
                "candidatesTokenCount": 17,
                "thoughtsTokenCount": 2,
                "totalTokenCount": 50
            },
            "modelVersion": "gemini-3.5-flash-low",
            "responseId": response_id
        },
        "traceId": trace_id,
        "metadata": {}
    });
    let final_chunk = json!({
        "response": {
            "candidates": [{
                "content": {"parts": [{"text": ""}], "role": "model"},
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 31,
                "candidatesTokenCount": 17,
                "thoughtsTokenCount": 2,
                "totalTokenCount": 50
            },
            "modelVersion": "gemini-3.5-flash-low",
            "responseId": first["response"]["responseId"].clone()
        },
        "traceId": first["traceId"].clone(),
        "metadata": {}
    });
    Bytes::from(format!(
        "data: {}\n\ndata: {}\n\n",
        json_compact(first),
        json_compact(final_chunk)
    ))
}

fn gemini_api_stream(payload: Value, model: String) -> Bytes {
    let (token, path) = write_target(&payload, "gemini");
    let has_function_response = serde_json::to_string(&payload)
        .map(|raw| raw.contains("functionResponse"))
        .unwrap_or(false);
    if has_function_response {
        let chunk = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        {"text": "ledger reasoning", "thought": true},
                        {"text": token}
                    ],
                    "role": "model"
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 31,
                "candidatesTokenCount": 17,
                "thoughtsTokenCount": 2,
                "totalTokenCount": 50
            },
            "modelVersion": model
        });
        return Bytes::from(format!("data: {}\n\n", json_compact(chunk)));
    }
    if payload.get("tools").is_none() {
        let chunk = json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hello world!"}],
                    "role": "model"
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 5,
                "candidatesTokenCount": 3,
                "totalTokenCount": 8
            },
            "modelVersion": model
        });
        return Bytes::from(format!("data: {}\n\n", json_compact(chunk)));
    }
    let chunk = json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "functionCall": {
                        "name": "write_to_file",
                        "args": {"TargetFile": path, "Content": format!("{token}\n")}
                    }
                }],
                "role": "model"
            },
            "finishReason": "STOP"
        }],
        "usageMetadata": {
            "promptTokenCount": 31,
            "candidatesTokenCount": 17,
            "thoughtsTokenCount": 2,
            "totalTokenCount": 50
        },
        "modelVersion": model
    });
    Bytes::from(format!("data: {}\n\n", json_compact(chunk)))
}

fn gemini_api_response(payload: Value, model: String) -> Value {
    let (token, _) = write_target(&payload, "gemini");
    json!({
        "modelVersion": model,
        "candidates": [{"content": {"parts": [{"text": format!("{token} nonstream")}], "role": "model"}}],
        "usageMetadata": {"promptTokenCount": 11, "candidatesTokenCount": 7, "totalTokenCount": 18}
    })
}

fn google_model_from_path(path: &str) -> String {
    path.split("/models/")
        .nth(1)
        .and_then(|tail| tail.split(':').next())
        .filter(|model| !model.is_empty())
        .unwrap_or("gemini-3.5-flash")
        .to_string()
}

fn write_target(payload: &Value, default_prefix: &str) -> (String, String) {
    let raw = serde_json::to_string(payload).unwrap_or_default();
    let token = find_hex32(&raw).unwrap_or_else(|| EXPECTED_POEM.to_string());
    let path =
        find_root_txt_path(&raw).unwrap_or_else(|| format!("/root/{default_prefix}-output.txt"));
    (token, path)
}

fn find_hex32(raw: &str) -> Option<String> {
    raw.as_bytes()
        .windows(32)
        .find(|window| window.iter().all(u8::is_ascii_hexdigit))
        .and_then(|window| std::str::from_utf8(window).ok())
        .map(ToOwned::to_owned)
}

fn find_root_txt_path(raw: &str) -> Option<String> {
    raw.match_indices("/root/")
        .filter_map(|(start, _)| {
            let tail = &raw[start..];
            let end = tail
                .char_indices()
                .find_map(|(index, ch)| {
                    let allowed = ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-');
                    (!allowed).then_some(index)
                })
                .unwrap_or(tail.len());
            let candidate = &tail[..end];
            candidate.find(".txt").map(|index| {
                let end = index + 4;
                candidate[..end].replace("\\/", "/")
            })
        })
        .last()
}

fn shell_write_command(token: &str, path: &str) -> String {
    format!("printf '%s\\n' {token} > {path}")
}

fn json_compact(value: Value) -> String {
    serde_json::to_string(&value).expect("serialize compact JSON")
}

fn mcp_response(payload: Value) -> Value {
    let id = payload.get("id").cloned().unwrap_or(json!(1));
    let method = payload.get("method").and_then(Value::as_str).unwrap_or("");
    match method {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {"listChanged": false}, "resources": {}},
                "serverInfo": {"name": "capsem-mock-server", "version": "1.0.0"}
            }
        }),
        "tools/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": [
                    {"name": "fixture_lookup", "description": "Return deterministic debug content.", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}}},
                    {"name": "fetch_http", "description": "Fetch a local mock server URL.", "inputSchema": {"type": "object", "properties": {"url": {"type": "string"}}}},
                    {"name": "slow_sleep", "description": "Sleep before returning deterministic text.", "inputSchema": {"type": "object", "properties": {}}}
                ]
            }
        }),
        "tools/call" => {
            let name = payload
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or("fixture_lookup");
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{"type": "text", "text": format!("capsem-mock-server:mcp:{name}")}],
                    "isError": false
                }
            })
        }
        "resources/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "resources": [{
                    "uri": "doc://slow",
                    "name": "slow-doc",
                    "description": "Slow deterministic resource.",
                    "mimeType": "text/plain"
                }]
            }
        }),
        "resources/read" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "contents": [{"uri": payload.pointer("/params/uri").and_then(Value::as_str).unwrap_or("doc://unknown"), "mimeType": "text/plain", "text": "capsem-mock-server:mcp:resource"}]
            }
        }),
        _ => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}})
        }
    }
}

fn mcp_payload_should_delay(payload: &Value) -> bool {
    match payload.get("method").and_then(Value::as_str) {
        Some("tools/call") => {
            payload.pointer("/params/name").and_then(Value::as_str) == Some("slow_sleep")
        }
        Some("resources/read") => payload
            .pointer("/params/uri")
            .and_then(Value::as_str)
            .is_some_and(|uri| uri == "doc://slow" || uri.ends_with("/doc://slow")),
        _ => false,
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs()
}

fn log_request(
    state: &State,
    method: &Method,
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    request_body: &[u8],
    response: &Response<RespBody>,
) {
    let Some(file) = &state.request_log else {
        return;
    };
    let request_body_text = if path == "/log" {
        format!(
            "<{} bytes omitted from telemetry log request>",
            request_body.len()
        )
    } else {
        String::from_utf8_lossy(request_body).to_string()
    };
    let record = json!({
        "timestamp": now_unix(),
        "method": method.as_str(),
        "path": path,
        "query": query.unwrap_or(""),
        "headers": headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_ascii_lowercase(), json!(value)))
            })
            .collect::<serde_json::Map<String, Value>>(),
        "request_bytes": request_body.len(),
        "response_bytes": response
            .extensions()
            .get::<LogBody>()
            .map(|body| body.0.len())
            .unwrap_or(0),
        "status": response.status().as_u16(),
        "content_type": response.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or(""),
        "request_body": request_body_text,
        "response_body": response
            .extensions()
            .get::<LogBody>()
            .map(|body| String::from_utf8_lossy(&body.0).to_string())
            .unwrap_or_default(),
    });
    if let Ok(mut file) = file.lock() {
        let _ = writeln!(file, "{record}");
    }
}

async fn handle_ws(mut req: Request<Incoming>, path: String) -> Response<RespBody> {
    let key = req
        .headers()
        .get(SEC_WEBSOCKET_KEY)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    if key.is_empty() {
        return response(StatusCode::BAD_REQUEST, Bytes::new(), "text/plain");
    }
    let accept = websocket_accept(&key);
    let on_upgrade = hyper::upgrade::on(&mut req);
    tokio::spawn(async move {
        if let Ok(upgraded) = on_upgrade.await {
            let io = TokioIo::new(upgraded);
            handle_ws_stream(io, path).await;
        }
    });
    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "Upgrade")
        .header(SEC_WEBSOCKET_ACCEPT, accept)
        .body(full(Bytes::new()))
        .expect("build websocket upgrade")
}

fn websocket_accept(key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
}

async fn handle_ws_stream(mut io: TokioIo<Upgraded>, path: String) {
    if path == "/ws/close" {
        let _ = write_ws_frame(&mut io, 0x8, &[]).await;
        return;
    }
    if path == "/ws/ping" {
        let _ = write_ws_frame(&mut io, 0x9, b"capsem-ping").await;
        return;
    }
    while let Ok(Some((opcode, payload))) = read_ws_frame(&mut io).await {
        match opcode {
            0x1 | 0x2 => {
                let _ = write_ws_frame(&mut io, opcode, &payload).await;
            }
            0x8 => {
                let _ = write_ws_frame(&mut io, 0x8, &[]).await;
                return;
            }
            0x9 => {
                let _ = write_ws_frame(&mut io, 0xA, &payload).await;
            }
            _ => {}
        }
    }
}

async fn read_ws_frame(io: &mut TokioIo<Upgraded>) -> Result<Option<(u8, Vec<u8>)>> {
    let mut header = [0_u8; 2];
    if io.read_exact(&mut header).await.is_err() {
        return Ok(None);
    }
    let opcode = header[0] & 0x0f;
    let masked = header[1] & 0x80 != 0;
    let mut len = u64::from(header[1] & 0x7f);
    if len == 126 {
        let mut bytes = [0_u8; 2];
        io.read_exact(&mut bytes).await?;
        len = u64::from(u16::from_be_bytes(bytes));
    } else if len == 127 {
        let mut bytes = [0_u8; 8];
        io.read_exact(&mut bytes).await?;
        len = u64::from_be_bytes(bytes);
    }
    let mut mask = [0_u8; 4];
    if masked {
        io.read_exact(&mut mask).await?;
    }
    let mut payload = vec![0_u8; usize::try_from(len).context("websocket frame too large")?];
    io.read_exact(&mut payload).await?;
    if masked {
        for (idx, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[idx % 4];
        }
    }
    Ok(Some((opcode, payload)))
}

async fn write_ws_frame(io: &mut TokioIo<Upgraded>, opcode: u8, payload: &[u8]) -> Result<()> {
    let mut header = Vec::with_capacity(10);
    header.push(0x80 | opcode);
    if payload.len() < 126 {
        header.push(u8::try_from(payload.len()).expect("len < 126"));
    } else if payload.len() <= usize::from(u16::MAX) {
        header.push(126);
        header.extend_from_slice(&u16::try_from(payload.len()).expect("fits").to_be_bytes());
    } else {
        header.push(127);
        header.extend_from_slice(&u64::try_from(payload.len()).expect("fits").to_be_bytes());
    }
    io.write_all(&header).await?;
    io.write_all(payload).await?;
    io.flush().await?;
    Ok(())
}

async fn serve_dns_udp(socket: UdpSocket, state: State) {
    let mut buf = vec![0_u8; 1500];
    loop {
        let Ok((len, peer)) = socket.recv_from(&mut buf).await else {
            continue;
        };
        if let Some((response, exchange)) = dns_response_with_exchange(&buf[..len]) {
            log_dns_request(&state, "udp", &exchange);
            let _ = socket.send_to(&response, peer).await;
        }
    }
}

async fn serve_dns_tcp(listener: TcpListener, state: State) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            continue;
        };
        let state = state.clone();
        tokio::spawn(async move {
            loop {
                let mut len_bytes = [0_u8; 2];
                if stream.read_exact(&mut len_bytes).await.is_err() {
                    return;
                }
                let len = usize::from(u16::from_be_bytes(len_bytes));
                let mut query = vec![0_u8; len];
                if stream.read_exact(&mut query).await.is_err() {
                    return;
                }
                let Some((response, exchange)) = dns_response_with_exchange(&query) else {
                    return;
                };
                let Ok(response_len) = u16::try_from(response.len()) else {
                    return;
                };
                log_dns_request(&state, "tcp", &exchange);
                if stream.write_all(&response_len.to_be_bytes()).await.is_err() {
                    return;
                }
                if stream.write_all(&response).await.is_err() {
                    return;
                }
            }
        });
    }
}

#[cfg(test)]
fn dns_response(query: &[u8]) -> Option<Vec<u8>> {
    dns_response_with_exchange(query).map(|(response, _)| response)
}

fn dns_response_with_exchange(query: &[u8]) -> Option<(Vec<u8>, DnsExchange)> {
    if query.len() < 12 {
        return None;
    }
    let query_id = &query[..2];
    let mut offset = 12;
    let mut labels = Vec::new();
    while offset < query.len() {
        let len = usize::from(query[offset]);
        offset += 1;
        if len == 0 {
            break;
        }
        if offset + len > query.len() {
            return None;
        }
        labels.push(String::from_utf8_lossy(&query[offset..offset + len]).to_string());
        offset += len;
    }
    if offset + 4 > query.len() {
        return None;
    }
    let qtype = u16::from_be_bytes([query[offset], query[offset + 1]]);
    let qclass = u16::from_be_bytes([query[offset + 2], query[offset + 3]]);
    let question_end = offset + 4;
    let name = labels.join(".").to_ascii_lowercase();
    let known = DNS_FIXTURES.iter().any(|fixture| *fixture == name);
    let mut response = Vec::with_capacity(query.len() + 32);
    response.extend_from_slice(query_id);
    response.extend_from_slice(if known { &[0x81, 0x80] } else { &[0x81, 0x83] });
    response.extend_from_slice(&[0x00, 0x01]);
    response.extend_from_slice(if known { &[0x00, 0x01] } else { &[0x00, 0x00] });
    response.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    response.extend_from_slice(&query[12..question_end]);
    if known {
        response.extend_from_slice(&[
            0xC0, 0x0C, // name pointer
            0x00, 0x01, // A
            0x00, 0x01, // IN
            0x00, 0x00, 0x00, 0x3C, // ttl 60
            0x00, 0x04, // len
            127, 0, 0, 1,
        ]);
    }
    let exchange = DnsExchange {
        qname: name,
        qtype,
        qclass,
        rcode: if known { 0 } else { 3 },
        request_bytes: query.len(),
        response_bytes: response.len(),
    };
    Some((response, exchange))
}

fn log_dns_request(state: &State, source_proto: &str, exchange: &DnsExchange) {
    let Some(file) = &state.request_log else {
        return;
    };
    let record = json!({
        "timestamp": now_unix(),
        "kind": "dns",
        "source_proto": source_proto,
        "qname": exchange.qname,
        "qtype": exchange.qtype,
        "qclass": exchange.qclass,
        "rcode": exchange.rcode,
        "request_bytes": exchange.request_bytes,
        "response_bytes": exchange.response_bytes,
    });
    if let Ok(mut file) = file.lock() {
        let _ = writeln!(file, "{record}");
        let _ = file.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_bytes_are_cached_and_correct() {
        let first = deterministic_bytes("10mb").expect("10mb fixture");
        let second = deterministic_bytes("10mb").expect("10mb fixture");
        assert_eq!(first.len(), 10 * 1024 * 1024);
        assert_eq!(first, second);
        assert_eq!(&first[..26], b"abcdefghijklmnopqrstuvwxyz");
    }

    #[test]
    fn dns_fixture_answers_known_names_and_rejects_unknown() {
        let query = test_dns_query("fixture.capsem.test", 0xCAFE);
        let response = dns_response(&query).expect("dns response");
        assert_eq!(&response[..2], b"\xCA\xFE");
        assert_eq!(response[3] & 0x0F, 0);
        assert_eq!(&response[response.len() - 4..], &[127, 0, 0, 1]);

        let query = test_dns_query("unknown.capsem.invalid", 0xBEEF);
        let response = dns_response(&query).expect("dns response");
        assert_eq!(&response[..2], b"\xBE\xEF");
        assert_eq!(response[3] & 0x0F, 3);
    }

    #[test]
    fn websocket_accept_matches_rfc_fixture() {
        assert_eq!(
            websocket_accept("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    fn test_dns_query(name: &str, id: u16) -> Vec<u8> {
        let mut query = Vec::new();
        query.extend_from_slice(&id.to_be_bytes());
        query.extend_from_slice(&[0x01, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
        for part in name.split('.') {
            query.push(u8::try_from(part.len()).expect("label fits"));
            query.extend_from_slice(part.as_bytes());
        }
        query.extend_from_slice(&[0, 0, 1, 0, 1]);
        query
    }
}
