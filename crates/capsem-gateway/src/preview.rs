//! Browser preview admission. Workload bytes never pass through this process:
//! authenticated connected sockets are handed to the VM owner's confined
//! router, which performs the HTTP filtering and streaming.

use crate::AppState;
use anyhow::Context;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use capsem_api::{
    PreviewAdmissionKind, PreviewBootstrapExchangeRequest, PreviewBootstrapExchangeResponse,
    PreviewConnectionAdmissionRequest, PreviewConnectionAdmissionResponse, PreviewSessionMaterial,
    PreviewSessionResponse,
};
use capsem_foundation::unix::router_channel::{Receiver, Sender};
use capsem_proto::privatelink::{decode_seat_frame, seat_frame, SEAT_PREVIEW};
use http_body_util::BodyExt;
use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BOOTSTRAP_BODY_BYTES: usize = 256;
const SERVICE_RESPONSE_BYTES: usize = 64 * 1024;
const HEADER_DEADLINE: Duration = Duration::from_secs(5);
const HANDOFF_DEADLINE: Duration = Duration::from_secs(5);
/// Connections admitted at once. Each one costs a task, a peek buffer and a
/// service round trip before it becomes the VM owner's problem, so a local
/// process opening sockets and stalling cannot grow the gateway without bound.
const MAX_ADMITTING_CONNECTIONS: usize = 64;
use capsem_proto::{PREVIEW_COOKIE, PREVIEW_SESSION_LIFETIME_SECS};

#[derive(Clone)]
struct Scope {
    vm_id: String,
    exposure_id: String,
    owner_generation: String,
    expires: Instant,
}

pub struct PreviewState {
    port: u16,
    scopes: Mutex<HashMap<String, Scope>>,
}

impl PreviewState {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            scopes: Mutex::new(HashMap::new()),
        }
    }

    fn scope(&self, label: &str) -> Option<Scope> {
        let now = Instant::now();
        let mut scopes = self.scopes.lock().unwrap();
        scopes.retain(|_, scope| scope.expires > now);
        scopes.get(label).cloned()
    }
}

pub async fn create_session(
    State(state): State<Arc<AppState>>,
    Path((vm_id, exposure_id)): Path<(String, String)>,
) -> Response {
    if !safe_id(&vm_id) || !safe_dns_label(&exposure_id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let path = format!("/internal/vms/{vm_id}/exposures/{exposure_id}/preview-session");
    let material: PreviewSessionMaterial = match service_json(&state, &path, serde_json::json!({})).await {
        Ok(material) => material,
        Err(response) => return response,
    };
    if material.exposure.id != exposure_id || material.exposure.host_port.is_some() {
        return StatusCode::BAD_GATEWAY.into_response();
    }
    let lifetime =
        Duration::from_secs(u64::from(PREVIEW_SESSION_LIFETIME_SECS) + u64::from(material.expires_in_seconds));
    state.previews.scopes.lock().unwrap().insert(
        exposure_id.clone(),
        Scope {
            vm_id,
            exposure_id: exposure_id.clone(),
            owner_generation: material.owner_generation,
            expires: Instant::now() + lifetime,
        },
    );
    tracing::info!(%exposure_id, "preview bootstrap issued");
    axum::Json(PreviewSessionResponse {
        url: format!(
            "http://{exposure_id}.localhost:{}/_capsem/bootstrap",
            state.previews.port
        ),
        bootstrap_token: material.bootstrap_token,
        expires_in_seconds: material.expires_in_seconds,
    })
    .into_response()
}

pub async fn serve(listener: tokio::net::TcpListener, state: Arc<AppState>) {
    let admitting = Arc::new(tokio::sync::Semaphore::new(MAX_ADMITTING_CONNECTIONS));
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error) => {
                tracing::error!(%error, "preview listener failed");
                return;
            }
        };
        if !peer.ip().is_loopback() {
            continue;
        }
        let Ok(permit) = Arc::clone(&admitting).try_acquire_owned() else {
            tracing::warn!("preview admission capacity reached; connection refused");
            continue;
        };
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let _permit = permit;
            if let Err(error) = Box::pin(handle_connection(stream, state)).await {
                tracing::debug!(%error, "preview connection refused");
            }
        });
    }
}

/// What a refused browser is told. Without a response the socket is simply
/// dropped, which a browser shows as ERR_EMPTY_RESPONSE rather than as an
/// expired preview session.
struct Refusal {
    status: &'static str,
    message: &'static str,
    error: anyhow::Error,
}

fn refuse(status: &'static str, message: &'static str, error: impl Into<anyhow::Error>) -> Refusal {
    Refusal {
        status,
        message,
        error: error.into(),
    }
}

async fn write_refusal(stream: &mut tokio::net::TcpStream, refusal: &Refusal) {
    let body = refusal.message;
    let response = format!(
        "HTTP/1.1 {}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        refusal.status,
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

struct Head {
    method: String,
    path: String,
    label: String,
    port: u16,
    cookie: Option<String>,
    websocket: bool,
    transfer_encoded: bool,
    header_bytes: usize,
    content_length: usize,
}

async fn handle_connection(mut stream: tokio::net::TcpStream, state: Arc<AppState>) -> anyhow::Result<()> {
    match admit(&mut stream, &state).await {
        Ok(None) => Ok(()),
        Ok(Some(admitted)) => handoff(stream, admitted).await,
        Err(refusal) => {
            write_refusal(&mut stream, &refusal).await;
            Err(refusal.error)
        }
    }
}

/// Admit one browser connection, or say why not. `Ok(None)` means the
/// connection was answered here, as the bootstrap exchange is.
async fn admit(
    stream: &mut tokio::net::TcpStream,
    state: &Arc<AppState>,
) -> Result<Option<PreviewConnectionAdmissionResponse>, Refusal> {
    const MALFORMED: &str = "This is a Capsem preview origin and did not receive a valid HTTP request.";
    const UNKNOWN: &str = "This preview is no longer available. Reopen it from Capsem.";
    const EXPIRED: &str = "This preview session has expired. Reopen the preview from Capsem.";
    const DENIED: &str = "This preview connection was refused by the VM's policy.";

    let head = Box::pin(tokio::time::timeout(HEADER_DEADLINE, peek_head(stream)))
        .await
        .map_err(|_| {
            refuse(
                "408 Request Timeout",
                MALFORMED,
                anyhow::anyhow!("preview headers timed out"),
            )
        })?
        .map_err(|error| refuse("400 Bad Request", MALFORMED, error))?;
    validate_origin_port(&head, state.previews.port).map_err(|error| refuse("400 Bad Request", MALFORMED, error))?;
    let scope = state.previews.scope(&head.label).ok_or_else(|| {
        refuse(
            "404 Not Found",
            UNKNOWN,
            anyhow::anyhow!("unknown or expired preview origin"),
        )
    })?;
    if head.method == "POST" && head.path == "/_capsem/bootstrap" {
        return exchange_bootstrap(stream, state, &scope, &head)
            .await
            .map(|()| None)
            .map_err(|error| refuse("400 Bad Request", EXPIRED, error));
    }
    let session_token = head.cookie.ok_or_else(|| {
        refuse(
            "401 Unauthorized",
            EXPIRED,
            anyhow::anyhow!("preview session cookie missing"),
        )
    })?;
    let kind = if head.websocket {
        PreviewAdmissionKind::WebsocketUpgrade
    } else {
        PreviewAdmissionKind::Request
    };
    let path = format!(
        "/internal/vms/{}/exposures/{}/preview-admission",
        scope.vm_id, scope.exposure_id
    );
    let body = serde_json::to_value(PreviewConnectionAdmissionRequest { session_token, kind })
        .map_err(|error| refuse("500 Internal Server Error", DENIED, error))?;
    let admitted: PreviewConnectionAdmissionResponse = service_json(state, &path, body).await.map_err(|response| {
        refuse(
            "403 Forbidden",
            DENIED,
            anyhow::anyhow!("preview admission refused with {}", response.status()),
        )
    })?;
    if admitted.owner_generation != scope.owner_generation {
        return Err(refuse(
            "409 Conflict",
            UNKNOWN,
            anyhow::anyhow!("preview owner generation changed"),
        ));
    }
    Ok(Some(admitted))
}

async fn exchange_bootstrap(
    stream: &mut tokio::net::TcpStream,
    state: &AppState,
    scope: &Scope,
    head: &Head,
) -> anyhow::Result<()> {
    validate_bootstrap_head(head)?;
    let mut request = vec![0; head.header_bytes + head.content_length];
    tokio::time::timeout(HEADER_DEADLINE, stream.read_exact(&mut request)).await??;
    let body = std::str::from_utf8(&request[head.header_bytes..])?;
    let token = parse_bootstrap_token(body)?;
    let path = format!(
        "/internal/vms/{}/exposures/{}/preview-bootstrap",
        scope.vm_id, scope.exposure_id
    );
    let exchanged: PreviewBootstrapExchangeResponse = service_json(
        state,
        &path,
        serde_json::to_value(PreviewBootstrapExchangeRequest { bootstrap_token: token })?,
    )
    .await
    .map_err(|response| anyhow::anyhow!("preview bootstrap refused with {}", response.status()))?;
    let response = format!(
        "HTTP/1.1 303 See Other\r\nLocation: /\r\nSet-Cookie: {PREVIEW_COOKIE}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
        exchanged.session_token, exchanged.expires_in_seconds
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await?;
    tracing::info!(exposure_id = %scope.exposure_id, "preview bootstrap exchanged");
    Ok(())
}

fn parse_bootstrap_token(body: &str) -> anyhow::Result<String> {
    let mut tokens = url::form_urlencoded::parse(body.as_bytes())
        .filter_map(|(name, value)| (name == "bootstrap_token").then(|| value.into_owned()));
    let token = tokens
        .next()
        .filter(|value| !value.is_empty())
        .context("preview bootstrap token missing")?;
    anyhow::ensure!(tokens.next().is_none(), "duplicate preview bootstrap token");
    Ok(token)
}

fn validate_origin_port(head: &Head, port: u16) -> anyhow::Result<()> {
    anyhow::ensure!(head.port == port, "preview origin port mismatch");
    Ok(())
}

fn validate_bootstrap_head(head: &Head) -> anyhow::Result<()> {
    anyhow::ensure!(
        head.content_length <= MAX_BOOTSTRAP_BODY_BYTES,
        "preview bootstrap body too large"
    );
    anyhow::ensure!(!head.transfer_encoded, "chunked preview bootstrap is unsupported");
    Ok(())
}

async fn handoff(stream: tokio::net::TcpStream, admitted: PreviewConnectionAdmissionResponse) -> anyhow::Result<()> {
    let source = stream.into_std()?;
    let seat = std::os::unix::net::UnixStream::connect(&admitted.handoff_socket)?;
    let sender = Sender::new(seat.try_clone()?)?;
    sender
        .send(&seat_frame(SEAT_PREVIEW, admitted.handoff_token), &[source.as_raw_fd()])
        .await?;
    let receiver = Receiver::new(seat)?;
    let acknowledgement = tokio::time::timeout(HANDOFF_DEADLINE, receiver.recv()).await??;
    let (kind, token) = decode_seat_frame(&acknowledgement.bytes).map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        kind == SEAT_PREVIEW && token == admitted.handoff_token && acknowledgement.fds.is_empty(),
        "invalid preview handoff acknowledgement"
    );
    tracing::debug!(owner_generation = %admitted.owner_generation, "preview socket handed to VM owner");
    Ok(())
}

/// How many times `peek_head` looked at a socket, so a test can tell parking
/// from spinning.
#[cfg(test)]
pub(super) static PEEK_ROUNDS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Read the request head without consuming it, so the connected socket can be
/// handed to the VM owner's router exactly as the browser sent it.
///
/// A successful `peek` leaves the socket readable, so waiting on readability
/// again returns at once. Readiness is therefore cleared whenever a peek found
/// no new bytes, which parks this task until the browser actually sends more
/// instead of spinning a core until the header deadline.
async fn peek_head(stream: &tokio::net::TcpStream) -> anyhow::Result<Head> {
    let mut bytes = [0; MAX_HEADER_BYTES];
    let mut seen = 0;
    loop {
        stream.readable().await?;
        let count = stream.peek(&mut bytes).await?;
        anyhow::ensure!(count != 0, "preview client closed before headers");
        if let Some(end) = bytes[..count].windows(4).position(|window| window == b"\r\n\r\n") {
            return parse_head(&bytes[..end + 4]);
        }
        anyhow::ensure!(count < bytes.len(), "preview headers too large");
        #[cfg(test)]
        PEEK_ROUNDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if count == seen {
            // No new bytes: clear readiness so the next wait is an event.
            let _ = stream.try_io(tokio::io::Interest::READABLE, || {
                Err::<(), _>(std::io::ErrorKind::WouldBlock.into())
            });
        }
        seen = count;
    }
}

fn parse_head(bytes: &[u8]) -> anyhow::Result<Head> {
    let text = std::str::from_utf8(bytes)?;
    let mut lines = text.split("\r\n");
    let request = lines.next().context("preview request line missing")?;
    let mut request = request.split_whitespace();
    let method = request.next().context("preview method missing")?.to_string();
    let path = request.next().context("preview path missing")?.to_string();
    anyhow::ensure!(path.starts_with('/'), "preview request target must use origin form");
    anyhow::ensure!(request.next().is_some(), "preview HTTP version missing");
    let mut host = None;
    let mut cookie = None;
    let mut websocket = false;
    let mut content_length = 0;
    let mut saw_content_length = false;
    let mut transfer_encoded = false;
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').context("malformed preview header")?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("host") {
            anyhow::ensure!(host.is_none(), "duplicate preview host");
            host = Some(value);
        } else if name.eq_ignore_ascii_case("cookie") {
            for value in value
                .split(';')
                .map(str::trim)
                .filter_map(|pair| pair.strip_prefix(&format!("{PREVIEW_COOKIE}=")))
            {
                anyhow::ensure!(cookie.is_none(), "duplicate preview session cookie");
                anyhow::ensure!(!value.is_empty(), "empty preview session cookie");
                cookie = Some(value.to_string());
            }
        } else if name.eq_ignore_ascii_case("upgrade") && value.eq_ignore_ascii_case("websocket") {
            websocket = true;
        } else if name.eq_ignore_ascii_case("content-length") {
            anyhow::ensure!(!saw_content_length, "duplicate preview content length");
            saw_content_length = true;
            content_length = value.parse()?;
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            transfer_encoded = true;
        }
    }
    let authority: http::uri::Authority = host.context("preview host missing")?.parse()?;
    let port = authority.port_u16().context("preview host port missing")?;
    let label = authority
        .host()
        .strip_suffix(".localhost")
        .filter(|label| safe_dns_label(label))
        .context("invalid preview origin")?
        .to_string();
    anyhow::ensure!(port != 0, "invalid preview origin port");
    Ok(Head {
        method,
        path,
        label,
        port,
        cookie,
        websocket,
        transfer_encoded,
        header_bytes: bytes.len(),
        content_length,
    })
}

async fn service_json<T: serde::de::DeserializeOwned>(
    state: &AppState,
    path: &str,
    body: serde_json::Value,
) -> Result<T, Response> {
    let bytes = serde_json::to_vec(&body).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;
    let request = Request::builder()
        .method("POST")
        .uri(format!("http://localhost{path}"))
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(bytes))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;
    let response = state
        .service_client
        .request(request)
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY.into_response())?;
    let status = response.status();
    let body = http_body_util::Limited::new(response.into_body(), SERVICE_RESPONSE_BYTES)
        .collect()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY.into_response())?
        .to_bytes();
    if !status.is_success() {
        return Err((status, Body::from(body)).into_response());
    }
    serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_GATEWAY.into_response())
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn safe_dns_label(value: &str) -> bool {
    value.len() <= 63
        && safe_id(value)
        && !value.contains('_')
        && value.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && value.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
}

#[cfg(test)]
mod tests;
