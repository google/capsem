//! Capsem's MCP gateway over streamable HTTP, at a fixed in-VM name.
//!
//! A workload that cannot open vsock -- a container's seccomp profile denies
//! `AF_VSOCK` -- reaches the endpoint the in-guest relay uses by POSTing
//! JSON-RPC to `http://mcp.capsem.internal/mcp`. The VM's interception rules
//! carry that connection here like any other port-80 flow, and it is
//! answered by this VM's endpoint instead of being dialed. A proxy instance
//! belongs to one VM, so the endpoint, its policy and the session ledger it
//! writes are already that VM's: the request is attributed by where it
//! arrived, not by anything the guest says.
//!
//! Only the exact name on its scheme's default port is the gateway. Every
//! other name in Capsem's zone is refused here: nothing in it exists
//! upstream, and dialing one would hand a guest-chosen name to the host's
//! resolver.

use std::sync::Arc;
use std::time::SystemTime;

use capsem_logger::{Decision, NetEvent, WriteOp};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use tracing::debug;

use super::body::ProxyBoxBody;
use super::mcp_frame::{dispatch_http_message, HttpMcpReply};
use super::protocol::Protocol;
use super::MitmProxyConfig;
use crate::net::dns::private::{in_private_zone, MCP_HOST};

/// The streamable-HTTP endpoint path on [`MCP_HOST`].
const MCP_PATH: &str = "/mcp";

/// One POST carries one JSON-RPC message, bounded like a frame.
const MAX_MESSAGE_BYTES: usize = capsem_proto::MCP_FRAME_MAX_SIZE;

/// Rule id the ledger records for a refused name in the zone.
const RESERVED_RULE: &str = "capsem.internal.reserved";

/// Where a request for a name in Capsem's zone goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InternalRoute {
    /// This VM's MCP endpoint.
    McpGateway,
    /// Nowhere: answered here and never dialed.
    Refused,
}

/// The route for a request to `host:port` arriving over `protocol`, or
/// `None` when the host is outside the zone and the proxy handles it as
/// any other. `host` is the normalized identity the proxy dials and records.
pub(super) fn internal_route(host: &str, port: u16, protocol: Protocol) -> Option<InternalRoute> {
    if !in_private_zone(host) {
        return None;
    }
    let default_port = match protocol {
        Protocol::Http => 80,
        Protocol::Tls => 443,
        Protocol::McpFrame | Protocol::Unknown => return Some(InternalRoute::Refused),
    };
    if host.trim_end_matches('.').eq_ignore_ascii_case(MCP_HOST) && port == default_port {
        Some(InternalRoute::McpGateway)
    } else {
        Some(InternalRoute::Refused)
    }
}

/// Answer a request [`internal_route`] claimed.
pub(super) async fn serve(
    req: hyper::Request<hyper::body::Incoming>,
    route: InternalRoute,
    domain: &str,
    port: u16,
    protocol: Protocol,
    config: &Arc<MitmProxyConfig>,
    process_name: &Option<String>,
) -> hyper::Response<ProxyBoxBody> {
    if route == InternalRoute::Refused {
        record_refusal(config, domain, port, protocol, process_name, &req).await;
        return reply(
            403,
            "text/plain",
            format!("capsem: {domain} is reserved and not routed\n"),
        );
    }
    let Some(endpoint) = &config.mcp_endpoint else {
        return reply(503, "text/plain", "capsem: MCP endpoint is not running\n");
    };
    if req.uri().path() != MCP_PATH {
        return reply(404, "text/plain", format!("capsem: MCP is served at {MCP_PATH}\n"));
    }
    if req.method() != hyper::Method::POST {
        // No server-initiated stream and no sessions to end: the
        // streamable-HTTP transport answers GET and DELETE with 405.
        let mut response = reply(405, "text/plain", "capsem: MCP accepts POST\n");
        response
            .headers_mut()
            .insert(hyper::header::ALLOW, hyper::header::HeaderValue::from_static("POST"));
        return response;
    }
    let payload = match http_body_util::Limited::new(req.into_body(), MAX_MESSAGE_BYTES)
        .collect()
        .await
    {
        Ok(collected) => collected.to_bytes(),
        Err(error) => {
            debug!(%error, "MCP over HTTP: request body refused");
            return reply(413, "text/plain", "capsem: MCP message too large or truncated\n");
        }
    };
    let process_name = process_name.as_deref().unwrap_or("unknown");
    match dispatch_http_message(Arc::clone(endpoint), Arc::clone(&config.db), &payload, process_name).await {
        HttpMcpReply::Answer(response) => json_reply(200, &response),
        HttpMcpReply::Accepted => reply(202, "text/plain", Bytes::new()),
        HttpMcpReply::Invalid(response) => json_reply(400, &response),
    }
}

fn json_reply(status: u16, response: &capsem_proto::mcp_contracts::JsonRpcResponse) -> hyper::Response<ProxyBoxBody> {
    match serde_json::to_vec(response) {
        Ok(body) => reply(status, "application/json", body),
        Err(error) => reply(
            500,
            "text/plain",
            format!("capsem: MCP response did not serialize: {error}\n"),
        ),
    }
}

fn reply(status: u16, content_type: &'static str, body: impl Into<Bytes>) -> hyper::Response<ProxyBoxBody> {
    hyper::Response::builder()
        .status(status)
        .header(hyper::header::CONTENT_TYPE, content_type)
        .body(Full::new(body.into()).map_err(|never| match never {}).boxed())
        .expect("static response parts are valid")
}

/// A refused name is a denied network event: the guest asked for it, and
/// the ledger shows that it was not dialed and why.
async fn record_refusal(
    config: &MitmProxyConfig,
    domain: &str,
    port: u16,
    protocol: Protocol,
    process_name: &Option<String>,
    req: &hyper::Request<hyper::body::Incoming>,
) {
    let event = NetEvent {
        event_id: None,
        timestamp: SystemTime::now(),
        domain: domain.to_string(),
        port,
        decision: Decision::Denied,
        process_name: process_name.clone(),
        pid: None,
        method: Some(req.method().to_string()),
        path: Some(req.uri().path().to_string()),
        query: req.uri().query().map(str::to_string),
        status_code: Some(403),
        bytes_sent: 0,
        bytes_received: 0,
        duration_ms: 0,
        matched_rule: Some(RESERVED_RULE.to_string()),
        request_headers: None,
        response_headers: None,
        request_body: None,
        response_body: None,
        conn_type: Some(
            match protocol {
                Protocol::Tls => "https-mitm",
                _ => "http-mitm",
            }
            .to_string(),
        ),
        policy_mode: None,
        policy_action: Some("block".to_string()),
        policy_rule: Some(RESERVED_RULE.to_string()),
        policy_reason: Some("names in capsem.internal are never dialed".to_string()),
        trace_id: capsem_foundation::telemetry::ambient_capsem_trace_id(),
        credential_ref: None,
    };
    crate::security_engine::emit_security_write(&config.db, WriteOp::NetEvent(event)).await;
}

#[cfg(test)]
mod tests;
