//! Private HTTP capabilities: destinations, credentials and policy stay here.

use super::{BrokerPolicy, WorkerGrant};
use anyhow::{ensure, Context, Result};
use axum::body::Body;
use capsem_foundation::unix::fd;
use capsem_proto::mcp_worker::{McpHttpAdmission, McpHttpRole, MCP_ADMISSION_HEADER, MCP_SERVER_HEADER};
use futures::{StreamExt, TryStreamExt};
use http::{HeaderMap, HeaderValue, Method, Request, Response, StatusCode};
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::convert::Infallible;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

const HTTP_TIMEOUT: Duration = Duration::from_secs(capsem_proto::MCP_HTTP_TIMEOUT_SECONDS);

pub(super) async fn serve(
    socket: UnixStream,
    role: McpHttpRole,
    policy: watch::Receiver<Arc<BrokerPolicy>>,
    authority: WorkerGrant,
) {
    if let Err(error) = run(socket, role, policy, authority).await {
        tracing::debug!(%error, ?role, "MCP HTTP capability stopped");
    }
}

async fn run(
    socket: UnixStream,
    role: McpHttpRole,
    policy: watch::Receiver<Arc<BrokerPolicy>>,
    authority: WorkerGrant,
) -> Result<()> {
    fd::validate_connected_stream(socket.as_fd())?;
    fd::set_nonblocking(socket.as_fd(), true)?;
    let socket = tokio::net::UnixStream::from_std(socket)?;
    let client = client_builder().build()?;
    let service_authority = authority.clone();
    let service = hyper::service::service_fn(move |request| {
        let policy = policy.clone();
        let authority = service_authority.clone();
        let client = client.clone();
        async move { Ok::<_, Infallible>(handle(request, role, policy, authority, client).await) }
    });
    let connection = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
        .max_concurrent_streams(capsem_proto::NETWORK_CLIENT_CAPACITY as u32)
        .max_header_list_size(capsem_proto::MCP_FRAME_MAX_SIZE as u32)
        .serve_connection(TokioIo::new(socket), service);
    tokio::select! {
        biased;
        () = authority.revoked() => Ok(()),
        result = connection => result.context("MCP HTTP channel"),
    }
}

async fn handle(
    request: Request<Incoming>,
    role: McpHttpRole,
    mut updates: watch::Receiver<Arc<BrokerPolicy>>,
    authority: WorkerGrant,
    client: reqwest::Client,
) -> Response<Body> {
    if authority.is_revoked() {
        return refused(role, "MCP generation retired");
    }
    let deadline = (role == McpHttpRole::Builtin).then(|| tokio::time::Instant::now() + HTTP_TIMEOUT);
    let policy = updates.borrow_and_update().clone();
    let preparation = prepare(request, role, &policy, &client);
    let prepared = tokio::select! {
        biased;
        () = authority.revoked() => return refused(role, "MCP generation retired"),
        _ = updates.changed() => return refused(role, "MCP policy changed"),
        result = tokio::time::timeout(HTTP_TIMEOUT, preparation) => match result {
            Ok(Ok(prepared)) => prepared,
            Ok(Err(error)) => return refused(role, &error.to_string()),
            Err(_) => return refused(role, "MCP upstream deadline"),
        },
    };
    let Prepared { response, admission } = prepared;
    let status = response.status();
    let mut headers = response.headers().clone();
    // An upstream cannot forge coordinator-owned admission metadata.
    headers.remove(MCP_ADMISSION_HEADER);
    if let Some(admission) = admission {
        insert_admission(&mut headers, &admission);
    }
    let stream = response.bytes_stream().map_err(std::io::Error::other).boxed();
    let stream = futures::stream::unfold(Some((stream, updates, authority, deadline)), |state| async move {
        let (mut stream, mut updates, authority, deadline) = state?;
        let body_deadline = async {
            match deadline {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            biased;
            () = authority.revoked() => Some((Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "MCP generation retired")), None)),
            _ = updates.changed() => Some((Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "MCP policy changed")), None)),
            () = body_deadline => Some((Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "MCP builtin HTTP deadline")), None)),
            next = stream.next() => next.map(|chunk| (chunk, Some((stream, updates, authority, deadline)))),
        }
    });
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

struct Prepared {
    response: reqwest::Response,
    admission: Option<McpHttpAdmission>,
}

async fn prepare(
    request: Request<Incoming>,
    role: McpHttpRole,
    policy: &BrokerPolicy,
    client: &reqwest::Client,
) -> Result<Prepared> {
    let method = request.method().clone();
    let url = reqwest::Url::parse(&request.uri().to_string()).context("invalid MCP HTTP URL")?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "invalid MCP HTTP target"
    );
    let mut headers = request.headers().clone();
    let (client, admission) = match role {
        McpHttpRole::Aggregator => configured_client(&url, &method, &mut headers, policy, client)?,
        McpHttpRole::Builtin => builtin_client(&url, &method, &mut headers, policy).await?,
    };
    let body = http_body_util::Limited::new(request.into_body(), capsem_proto::MCP_FRAME_MAX_SIZE)
        .collect()
        .await
        .map_err(|error| anyhow::anyhow!("MCP HTTP request exceeds its body bound or is incomplete: {error}"))?
        .to_bytes();
    let response = client
        .request(method, url)
        .headers(headers)
        .body(body)
        .send()
        .await
        .context("MCP upstream request failed")?;
    Ok(Prepared { response, admission })
}

fn configured_client(
    url: &reqwest::Url,
    method: &Method,
    headers: &mut HeaderMap,
    policy: &BrokerPolicy,
    client: &reqwest::Client,
) -> Result<(reqwest::Client, Option<McpHttpAdmission>)> {
    ensure!(
        matches!(*method, Method::GET | Method::POST | Method::DELETE),
        "unapproved MCP server method"
    );
    let name = headers
        .remove(MCP_SERVER_HEADER)
        .context("MCP server identity missing")?;
    let name = name.to_str().context("invalid MCP server identity")?;
    let server = policy
        .runtime
        .mcp
        .servers
        .iter()
        .find(|server| server.name == name)
        .context("MCP server not configured for this session")?;
    ensure!(
        policy
            .runtime
            .mcp
            .server_enabled
            .get(name)
            .copied()
            .unwrap_or(server.enabled),
        "MCP server disabled"
    );
    let expected = reqwest::Url::parse(&server.url).context("configured MCP server URL is invalid")?;
    ensure!(*url == expected, "MCP server URL differs from its configured endpoint");
    let mut selected = HeaderMap::new();
    for key in [
        "accept",
        "content-type",
        "mcp-session-id",
        "mcp-protocol-version",
        "last-event-id",
    ] {
        if let Some(value) = headers.get(key) {
            selected.insert(http::header::HeaderName::from_static(key), value.clone());
        }
    }
    for (name, value) in &server.headers {
        let name: http::header::HeaderName = name.parse().context("invalid configured MCP header name")?;
        ensure!(
            name != MCP_SERVER_HEADER && name != MCP_ADMISSION_HEADER,
            "reserved MCP capability header in settings"
        );
        selected.insert(name, value.parse().context("invalid configured MCP header value")?);
    }
    if let Some(auth) = &server.auth {
        let token = capsem_credentials::resolve_broker_reference_for_provider(
            capsem_credentials::CredentialProvider::Mcp,
            &auth.credential_ref,
        )
        .map_err(|_| anyhow::anyhow!("configured MCP authentication unavailable"))?
        .context("configured MCP authentication unavailable")?;
        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {token}")).context("invalid MCP authorization material")?;
        authorization.set_sensitive(true);
        selected.insert(http::header::AUTHORIZATION, authorization);
    }
    *headers = selected;
    // The trusted config grants this exact endpoint. Reuse its connection
    // pool; workers never choose an address or perform a DNS lookup.
    Ok((client.clone(), None))
}

async fn builtin_client(
    url: &reqwest::Url,
    method: &Method,
    headers: &mut HeaderMap,
    policy: &BrokerPolicy,
) -> Result<(reqwest::Client, Option<McpHttpAdmission>)> {
    ensure!(
        matches!(*method, Method::GET | Method::HEAD),
        "unapproved builtin HTTP method"
    );
    ensure!(
        policy
            .runtime
            .mcp
            .server_enabled
            .get(capsem_core::mcp::BUILTIN_SERVER_NAME)
            .copied()
            .unwrap_or(true),
        "builtin MCP server disabled"
    );
    let admission = capsem_core::mcp::builtin_tools::authorize_upstream(
        url.as_str(),
        method.as_str(),
        &policy.runtime.security_rules,
        &policy.runtime.plugins,
    )
    .await
    .map_err(anyhow::Error::msg)?;
    // Builtin tools carry no arbitrary request headers or credential references.
    headers.clear();
    let client = client_builder()
        .resolve_to_addrs(&admission.domain, &admission.addresses)
        .build()?;
    Ok((
        client,
        Some(McpHttpAdmission::Allowed {
            rule_id: admission.decision.rule_id,
            reason: admission.decision.reason,
        }),
    ))
}

fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(capsem_proto::MCP_STARTUP_TIMEOUT_SECONDS))
}

fn insert_admission(headers: &mut HeaderMap, admission: &McpHttpAdmission) {
    let value = serde_json::to_string(admission)
        .expect("serializable MCP admission")
        .replace('\u{7f}', "\\u007f");
    headers.insert(
        MCP_ADMISSION_HEADER,
        HeaderValue::from_str(&value).expect("JSON escapes header controls"),
    );
}

fn refused(role: McpHttpRole, reason: &str) -> Response<Body> {
    let mut response = Response::new(Body::from(reason.to_owned()));
    *response.status_mut() = StatusCode::FORBIDDEN;
    if role == McpHttpRole::Builtin {
        insert_admission(
            response.headers_mut(),
            &McpHttpAdmission::Denied {
                reason: reason.to_owned(),
            },
        );
    }
    response
}

#[cfg(test)]
mod tests;
