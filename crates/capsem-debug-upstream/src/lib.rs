use std::convert::Infallible;
use std::future::Future;
use std::io::Write;
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::Context;
use axum::body::Bytes;
use axum::extract::ws::{close_code, CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::Path;
use axum::http::header::{CONTENT_ENCODING, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use flate2::write::GzEncoder;
use flate2::Compression;
use futures::{SinkExt, Stream, StreamExt};
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

const TINY_BODY: &[u8] = b"capsem-debug-upstream:tiny\n";
const HTML_ABOUT: &str = r#"<!doctype html>
<html>
  <head><title>Capsem Debug About</title></head>
  <body>
    <div id="about">
      <p>Capsem debug upstream about page for local MCP fetch tests.</p>
      <p>Google, Anthropic, and OpenAI appear here as fixture text only.</p>
      <a href="https://example.invalid/local">Local fixture link</a>
    </div>
  </body>
</html>
"#;
const SLOW_CHUNK_DELAY: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Serialize)]
pub struct ReadyPayload {
    pub service: &'static str,
    pub http_addr: String,
    pub base_url: String,
    pub endpoints: Vec<&'static str>,
}

#[derive(Debug)]
pub struct DebugUpstreamHandle {
    addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl DebugUpstreamHandle {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        self.task.await.context("join debug upstream task")?
    }
}

pub async fn spawn_debug_upstream() -> anyhow::Result<DebugUpstreamHandle> {
    spawn_debug_upstream_on(
        "127.0.0.1:0"
            .parse()
            .expect("valid debug upstream bind address"),
    )
    .await
}

pub async fn spawn_debug_upstream_on(addr: SocketAddr) -> anyhow::Result<DebugUpstreamHandle> {
    let listener = TcpListener::bind(addr)
        .await
        .context("bind debug upstream")?;
    let addr = listener
        .local_addr()
        .context("read debug upstream address")?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        serve_debug_upstream(listener, async {
            let _ = shutdown_rx.await;
        })
        .await
    });
    Ok(DebugUpstreamHandle {
        addr,
        shutdown_tx: Some(shutdown_tx),
        task,
    })
}

pub fn ready_payload(addr: SocketAddr) -> ReadyPayload {
    ReadyPayload {
        service: "capsem-debug-upstream",
        http_addr: addr.to_string(),
        base_url: format!("http://{addr}"),
        endpoints: vec![
            "/tiny",
            "/html/about",
            "/html/large",
            "/bytes/{size}",
            "/gzip/{size}",
            "/sse/model",
            "/slow-chunks",
            "/credential/response",
            "/echo",
            "/deny-target",
            "/ws/echo",
            "/ws/ping",
            "/ws/close",
        ],
    }
}

pub async fn serve_debug_upstream<S>(listener: TcpListener, shutdown: S) -> anyhow::Result<()>
where
    S: Future<Output = ()> + Send + 'static,
{
    axum::serve(listener, app())
        .with_graceful_shutdown(shutdown)
        .await
        .context("serve debug upstream")
}

pub fn app() -> Router {
    Router::new()
        .route("/tiny", get(tiny))
        .route("/html/about", get(html_about))
        .route("/html/large", get(html_large))
        .route("/bytes/{size}", get(bytes_endpoint))
        .route("/gzip/{size}", get(gzip_endpoint))
        .route("/sse/model", get(sse_model))
        .route("/slow-chunks", get(slow_chunks))
        .route("/credential/response", get(credential_response))
        .route("/echo", post(echo))
        .route("/deny-target", get(deny_target))
        .route("/ws/echo", get(ws_echo))
        .route("/ws/ping", get(ws_ping))
        .route("/ws/close", get(ws_close))
}

async fn tiny() -> impl IntoResponse {
    ([(CONTENT_TYPE, "text/plain; charset=utf-8")], TINY_BODY)
}

async fn html_about() -> impl IntoResponse {
    ([(CONTENT_TYPE, "text/html; charset=utf-8")], HTML_ABOUT)
}

async fn html_large() -> impl IntoResponse {
    let mut body = String::from("<!doctype html><html><body><main>\n");
    for idx in 0..80 {
        body.push_str(&format!(
            "<p>Capsem local pagination fixture paragraph {idx}: debug upstream content for MCP fetch tests.</p>\n"
        ));
    }
    body.push_str("</main></body></html>\n");
    ([(CONTENT_TYPE, "text/html; charset=utf-8")], body)
}

async fn bytes_endpoint(Path(size): Path<String>) -> Response {
    match deterministic_bytes_for_size(&size) {
        Ok(data) => (
            [(CONTENT_TYPE, "application/octet-stream")],
            Bytes::from(data),
        )
            .into_response(),
        Err(err) => bad_size(err),
    }
}

async fn gzip_endpoint(Path(size): Path<String>) -> Response {
    match deterministic_bytes_for_size(&size).and_then(gzip_bytes) {
        Ok(data) => (
            [
                (CONTENT_TYPE, "application/octet-stream"),
                (CONTENT_ENCODING, "gzip"),
            ],
            Bytes::from(data),
        )
            .into_response(),
        Err(err) => bad_size(err),
    }
}

fn bad_size(err: SizeError) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "error": err.to_string(),
            "allowed": ["10kb", "1mb", "10mb"]
        })),
    )
        .into_response()
}

async fn sse_model() -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let events = vec![
        Event::default()
            .event("model.delta")
            .data(r#"{"provider":"debug","model":"debug-local","content":"hello"}"#),
        Event::default()
            .event("model.tool_call")
            .data(r#"{"id":"tool_0001","name":"debug_lookup","arguments":{"query":"capsem"}}"#),
        Event::default()
            .event("model.done")
            .data(r#"{"finish_reason":"stop"}"#),
    ];
    Sse::new(tokio_stream::iter(events.into_iter().map(Ok))).keep_alive(KeepAlive::default())
}

async fn slow_chunks() -> Response {
    let stream = futures::stream::unfold(0usize, |idx| async move {
        if idx >= 4 {
            return None;
        }
        tokio::time::sleep(SLOW_CHUNK_DELAY).await;
        let chunk = Bytes::from(format!("chunk-{idx}\n"));
        Some((Ok::<Bytes, Infallible>(chunk), idx + 1))
    });
    (
        [(CONTENT_TYPE, "text/plain; charset=utf-8")],
        axum::body::Body::from_stream(stream),
    )
        .into_response()
}

async fn credential_response() -> impl IntoResponse {
    Json(serde_json::json!({
        "kind": "synthetic_credential_fixture",
        "api_key": "capsem_test_api_key_0123456789abcdef",
        "oauth": {
            "access_token": "capsem_test_oauth_access_0123456789abcdef",
            "refresh_token": "capsem_test_oauth_refresh_0123456789abcdef",
            "expires_in": 3600
        }
    }))
}

async fn echo(headers: HeaderMap, body: Bytes) -> impl IntoResponse {
    Json(serde_json::json!({
        "method": "POST",
        "path": "/echo",
        "body_size": body.len(),
        "content_type": header_string(&headers, "content-type"),
        "user_agent": header_string(&headers, "user-agent"),
        "header_count": headers.len(),
        "has_authorization": headers.contains_key("authorization"),
        "has_cookie": headers.contains_key("cookie"),
        "has_x_api_key": headers.contains_key("x-api-key")
    }))
}

async fn deny_target() -> impl IntoResponse {
    (
        [(CONTENT_TYPE, "text/plain; charset=utf-8")],
        "capsem-debug-upstream:deny-target\n",
    )
}

async fn ws_echo(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|socket| async move {
        handle_ws_echo(socket).await;
    })
}

async fn ws_ping(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|mut socket| async move {
        let _ = socket
            .send(Message::Ping(Bytes::from_static(b"capsem-ping")))
            .await;
        while let Some(Ok(msg)) = socket.recv().await {
            match msg {
                Message::Ping(payload) => {
                    if socket.send(Message::Pong(payload)).await.is_err() {
                        break;
                    }
                }
                Message::Pong(_) => {}
                Message::Close(_) => break,
                _ => {}
            }
        }
    })
}

async fn ws_close(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|mut socket| async move {
        let frame = CloseFrame {
            code: close_code::NORMAL,
            reason: "capsem-debug-close".into(),
        };
        let _ = socket.send(Message::Close(Some(frame))).await;
    })
}

async fn handle_ws_echo(socket: WebSocket) {
    let (mut write, mut read) = socket.split();
    while let Some(Ok(msg)) = read.next().await {
        match msg {
            Message::Text(_) | Message::Binary(_) => {
                if write.send(msg).await.is_err() {
                    break;
                }
            }
            Message::Ping(payload) => {
                if write.send(Message::Pong(payload)).await.is_err() {
                    break;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
}

fn header_string(headers: &HeaderMap, name: &'static str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn deterministic_bytes_for_size(size: &str) -> Result<Vec<u8>, SizeError> {
    let len = match size.to_ascii_lowercase().as_str() {
        "10kb" => 10 * 1024,
        "1mb" => 1024 * 1024,
        "10mb" => 10 * 1024 * 1024,
        _ => return Err(SizeError(size.to_string())),
    };
    Ok((0..len).map(|idx| b'a' + (idx % 26) as u8).collect())
}

fn gzip_bytes(data: Vec<u8>) -> Result<Vec<u8>, SizeError> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&data)
        .map_err(|err| SizeError(format!("gzip write failed: {err}")))?;
    encoder
        .finish()
        .map_err(|err| SizeError(format!("gzip finish failed: {err}")))
}

#[derive(Debug)]
struct SizeError(String);

impl std::fmt::Display for SizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unsupported size '{}'", self.0)
    }
}

impl std::error::Error for SizeError {}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;

    use super::*;

    #[tokio::test]
    async fn deterministic_http_endpoints_work() {
        let upstream = spawn_debug_upstream().await.unwrap();
        let client = reqwest::Client::new();

        let tiny = client
            .get(format!("{}/tiny", upstream.base_url()))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(tiny.as_ref(), TINY_BODY);

        let html_about = client
            .get(format!("{}/html/about", upstream.base_url()))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(html_about.contains("Capsem debug upstream about page"));
        assert!(html_about.contains("Google"));

        let html_large = client
            .get(format!("{}/html/large", upstream.base_url()))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(html_large.len() > 5000);
        assert!(html_large.contains("pagination fixture paragraph 79"));

        let bytes = client
            .get(format!("{}/bytes/10kb", upstream.base_url()))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(bytes.len(), 10 * 1024);
        assert_eq!(&bytes[..4], b"abcd");

        let gzip = client
            .get(format!("{}/gzip/10kb", upstream.base_url()))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let mut decoded = Vec::new();
        flate2::read::GzDecoder::new(gzip.as_ref())
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded.len(), 10 * 1024);
        assert_eq!(&decoded[..4], b"abcd");

        upstream.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn echo_reports_metadata_without_raw_secret_values() {
        let upstream = spawn_debug_upstream().await.unwrap();
        let secret = "capsem_test_secret_should_not_echo";
        let response: serde_json::Value = reqwest::Client::new()
            .post(format!("{}/echo", upstream.base_url()))
            .header("authorization", format!("Bearer {secret}"))
            .header("x-api-key", secret)
            .body(secret.to_string())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert_eq!(response["has_authorization"], true);
        assert_eq!(response["has_x_api_key"], true);
        assert_eq!(response["body_size"], secret.len());
        assert!(!response.to_string().contains(secret));

        upstream.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn sse_model_contains_tool_call_fixture() {
        let upstream = spawn_debug_upstream().await.unwrap();
        let body = reqwest::get(format!("{}/sse/model", upstream.base_url()))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();

        assert!(body.contains("event: model.tool_call"));
        assert!(body.contains("debug_lookup"));

        upstream.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn websocket_echo_ping_and_close_work() {
        let upstream = spawn_debug_upstream().await.unwrap();

        let (mut echo, _) =
            tokio_tungstenite::connect_async(format!("ws://{}/ws/echo", upstream.addr()))
                .await
                .unwrap();
        echo.send(TungsteniteMessage::Text("hello".into()))
            .await
            .unwrap();
        let echoed = echo.next().await.unwrap().unwrap();
        assert_eq!(echoed.to_text().unwrap(), "hello");

        let (mut ping, _) =
            tokio_tungstenite::connect_async(format!("ws://{}/ws/ping", upstream.addr()))
                .await
                .unwrap();
        match ping.next().await.unwrap().unwrap() {
            TungsteniteMessage::Ping(data) => assert_eq!(data.as_ref(), b"capsem-ping"),
            other => panic!("expected ping, got {other:?}"),
        }

        let (mut close, _) =
            tokio_tungstenite::connect_async(format!("ws://{}/ws/close", upstream.addr()))
                .await
                .unwrap();
        match close.next().await.unwrap().unwrap() {
            TungsteniteMessage::Close(Some(frame)) => {
                assert_eq!(
                    frame.code,
                    tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Normal
                );
                assert_eq!(frame.reason.to_string(), "capsem-debug-close");
            }
            other => panic!("expected close, got {other:?}"),
        }

        upstream.shutdown().await.unwrap();
    }
}
