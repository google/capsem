//! WebSocket upgrade handling for MITM-proxied connections.
//!
//! An upgrade is gated exactly like any other request -- port allowlist, then
//! the security boundary -- before the upstream is dialed. Only then is the
//! 101 relayed and the two upgraded streams spliced together.

use super::*;
use http_body_util::BodyExt;

/// Everything the upgrade branch needs from the request it was split from.
pub(super) struct UpgradeRequest<'a> {
    pub parts: &'a http::request::Parts,
    pub client_upgrade: hyper::upgrade::OnUpgrade,
    pub domain: &'a str,
    pub protocol: Protocol,
    pub upstream_port: u16,
    pub upstream_tls: &'a Arc<rustls::ClientConfig>,
    pub config: &'a Arc<MitmProxyConfig>,
    pub process_name: &'a Option<String>,
    pub policy: &'a NetworkMechanics,
    pub ai_provider: Option<ProviderKind>,
    pub ai_protocol: Option<ModelProtocol>,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub req_hdrs: String,
    pub matched_rule: String,
    pub start_time: Instant,
    pub conn_type: &'static str,
    pub credential_ref: Option<String>,
    pub credential_observations: Vec<crate::credential_broker::CredentialObservation>,
}

/// Gate, dial, relay the 101, and splice the upgraded streams.
pub(super) async fn handle_upgrade(
    request: UpgradeRequest<'_>,
    seal_with_telemetry: impl Fn(
        ProxyBoxBody,
        TelemetryRequestContext,
        Option<ProviderKind>,
        Option<ModelProtocol>,
    ) -> ProxyBoxBody,
) -> Result<hyper::Response<ProxyBoxBody>, anyhow::Error> {
    let UpgradeRequest {
        parts,
        client_upgrade,
        domain,
        protocol,
        upstream_port,
        upstream_tls,
        config,
        process_name,
        policy,
        ai_provider,
        ai_protocol,
        method,
        path,
        query,
        req_hdrs,
        matched_rule,
        start_time,
        conn_type,
        credential_ref,
        credential_observations,
    } = request;
    let request_security_decision = SecurityBoundaryDecisionFields::default();

    let original_headers = parts.headers.clone();
    let original_method = parts.method.clone();

    let ws_span = tracing::debug_span!(
        target: "capsem.mitm",
        spans::MITM_WEBSOCKET,
        protocol = protocol.label(),
        provider = provider_label(ai_provider),
        decision = tracing::field::Empty,
        status = tracing::field::Empty,
        error_kind = tracing::field::Empty,
    );
    let make_ws_error = |error: &dyn std::fmt::Display| -> hyper::Response<ProxyBoxBody> {
        let body_text = format!("Capsem: websocket upstream error ({error})\n");
        let req_ctx = TelemetryRequestContext {
            domain: domain.to_string(),
            process_name: process_name.clone(),
            ai_provider,
            ai_protocol,
            model_traffic: false,
            method: method.clone(),
            path: path.clone(),
            query: query.clone(),
            status_code: Some(502),
            decision: Decision::Denied,
            matched_rule: Some(matched_rule.clone()),
            request_headers: Some(req_hdrs.clone()),
            response_headers: None,
            start_time,
            request_body_stats: Arc::new(Mutex::new(BodyStats::new(0))),
            max_response_body_capture: 0,
            port: upstream_port,
            conn_type,
            policy_mode: request_security_decision.policy_mode.clone(),
            policy_action: request_security_decision.policy_action.clone(),
            policy_rule: request_security_decision.policy_rule.clone(),
            policy_reason: request_security_decision.policy_reason.clone(),
            credential_ref: credential_ref.clone(),
            credential_observations: credential_observations.clone(),
            credential_injections: Vec::new(),
        };
        let body = Full::new(Bytes::from(body_text))
            .map_err(|never| match never {})
            .boxed();
        hyper::Response::builder()
            .status(http::StatusCode::BAD_GATEWAY)
            .body(seal_with_telemetry(body, req_ctx, ai_provider, ai_protocol))
            .unwrap()
    };

    // Upgrades ride the same rail as every other request: the port
    // allowlist and the security boundary run before any dial. This
    // branch used to connect first, so `Upgrade: websocket` on an
    // otherwise blocked request reached any host, the gateway included,
    // and was logged as allowed.
    let refuse = |body_text: String,
                  matched: Option<String>,
                  decision: &SecurityBoundaryDecisionFields|
     -> hyper::Response<ProxyBoxBody> {
        let req_ctx = TelemetryRequestContext {
            domain: domain.to_string(),
            process_name: process_name.clone(),
            ai_provider,
            ai_protocol,
            model_traffic: false,
            method: method.clone(),
            path: path.clone(),
            query: query.clone(),
            status_code: Some(403),
            decision: Decision::Denied,
            matched_rule: matched,
            request_headers: Some(req_hdrs.clone()),
            response_headers: None,
            start_time,
            request_body_stats: Arc::new(Mutex::new(BodyStats::new(0))),
            max_response_body_capture: 0,
            port: upstream_port,
            conn_type,
            policy_mode: decision.policy_mode.clone(),
            policy_action: decision.policy_action.clone(),
            policy_rule: decision.policy_rule.clone(),
            policy_reason: decision.policy_reason.clone(),
            credential_ref: credential_ref.clone(),
            credential_observations: credential_observations.clone(),
            credential_injections: Vec::new(),
        };
        let body = Full::new(Bytes::from(body_text))
            .map_err(|never| match never {})
            .boxed();
        hyper::Response::builder()
            .status(http::StatusCode::FORBIDDEN)
            .body(seal_with_telemetry(body, req_ctx, ai_provider, ai_protocol))
            .unwrap()
    };
    if !http_upstream_port_allowed(policy, protocol, upstream_port) {
        let matched = "security.web.http_upstream_ports";
        tracing::Span::current().record("decision", "deny");
        return Ok(refuse(
            format!("capsem: HTTP upstream port {upstream_port} blocked by {matched}\n"),
            Some(matched.to_string()),
            &request_security_decision,
        ));
    }
    let mut upgrade_event = http_request_security_event(HttpRequestSecurityEventInput {
        domain,
        upstream_port,
        method: &method,
        path: &path,
        query: query.clone(),
        ai_provider,
        headers: original_headers.clone(),
        body: None,
    });
    if let Some(trace_id) = capsem_foundation::telemetry::ambient_capsem_trace_id() {
        upgrade_event = upgrade_event.with_trace_id(trace_id);
    }
    let rules = config.telemetry.security_rules.read().unwrap().clone();
    let upgrade_evaluation = crate::security_engine::evaluate_security_boundary(
        &rules,
        config.telemetry.plugin_policy.read().unwrap().clone(),
        upgrade_event,
    );
    let upgrade_decision = match upgrade_evaluation {
        Ok(evaluation) => evaluation,
        Err(error) => return Ok(make_ws_error(&error)),
    };
    let request_security_decision = SecurityBoundaryDecisionFields::from_enforcement(&upgrade_decision.enforcement);
    if !upgrade_decision.enforcement.is_allowed() {
        let rule_id = upgrade_decision.enforcement.rule_id.as_deref().unwrap_or("unknown");
        tracing::Span::current().record("decision", upgrade_decision.enforcement.action.as_str());
        return Ok(refuse(
            format!("capsem: websocket upgrade blocked by security rule: {rule_id}\n"),
            upgrade_decision.enforcement.rule_id.clone(),
            &request_security_decision,
        ));
    }
    let matched_rule = upgrade_decision
        .enforcement
        .rule_id
        .clone()
        .unwrap_or_else(|| matched_rule.clone());

    let dial_target = format!("{domain}:{upstream_port}");
    let upstream_tcp = match tokio::net::TcpStream::connect(&dial_target)
        .instrument(ws_span.clone())
        .await
    {
        Ok(stream) => stream,
        Err(error) => {
            ws_span.record("decision", "error");
            ws_span.record("status", "error");
            ws_span.record("error_kind", "upstream_tcp_connect");
            return Ok(make_ws_error(&error));
        }
    };

    let upstream_io: TokioIo<Box<dyn TokioReadWrite + Unpin + Send>> = match protocol {
        Protocol::Tls => {
            let connector = tokio_rustls::TlsConnector::from(Arc::clone(upstream_tls));
            let server_name = match rustls::pki_types::ServerName::try_from(domain.to_string()) {
                Ok(sn) => sn,
                Err(error) => {
                    ws_span.record("decision", "error");
                    ws_span.record("status", "error");
                    ws_span.record("error_kind", "upstream_server_name");
                    return Ok(make_ws_error(&error));
                }
            };
            match connector.connect(server_name, upstream_tcp).await {
                Ok(tls) => TokioIo::new(Box::new(tls) as Box<dyn TokioReadWrite + Unpin + Send>),
                Err(error) => {
                    ws_span.record("decision", "error");
                    ws_span.record("status", "error");
                    ws_span.record("error_kind", "upstream_tls_handshake");
                    return Ok(make_ws_error(&error));
                }
            }
        }
        Protocol::Http => TokioIo::new(Box::new(upstream_tcp) as Box<dyn TokioReadWrite + Unpin + Send>),
        Protocol::McpFrame => unreachable!("framed MCP bypasses HTTP upstream dial"),
        Protocol::Unknown => unreachable!("handle_inner gates Unknown earlier"),
    };

    let (mut sender, conn) = match hyper::client::conn::http1::handshake(upstream_io)
        .instrument(ws_span.clone())
        .await
    {
        Ok(pair) => pair,
        Err(error) => {
            ws_span.record("decision", "error");
            ws_span.record("status", "error");
            ws_span.record("error_kind", "upstream_http_handshake");
            return Ok(make_ws_error(&error));
        }
    };
    tokio::spawn(async move {
        let _ = conn.with_upgrades().await;
    });

    let full_path = match &query {
        Some(q) => format!("{path}?{q}"),
        None => path.clone(),
    };
    let mut builder = hyper::Request::builder().method(original_method).uri(&full_path);
    for (name, value) in original_headers.iter() {
        let drop_host = matches!(protocol, Protocol::Tls) && name == "host";
        if drop_host {
            continue;
        }
        builder = builder.header(name.clone(), value.clone());
    }
    if matches!(protocol, Protocol::Tls) {
        builder = builder.header("host", domain);
    }
    let upstream_req = builder.body(
        http_body_util::Empty::<Bytes>::new()
            .map_err(|never| -> anyhow::Error { match never {} })
            .boxed(),
    )?;

    let mut upstream_resp = match sender.send_request(upstream_req).instrument(ws_span.clone()).await {
        Ok(response) => response,
        Err(error) => {
            ws_span.record("decision", "error");
            ws_span.record("status", "error");
            ws_span.record("error_kind", "upstream_send_request");
            return Ok(make_ws_error(&error));
        }
    };
    let status_code = upstream_resp.status().as_u16();
    let upstream_upgrade = if upstream_resp.status() == http::StatusCode::SWITCHING_PROTOCOLS {
        Some(hyper::upgrade::on(&mut upstream_resp))
    } else {
        None
    };
    let (resp_parts, _resp_body) = upstream_resp.into_parts();
    if let Some(upstream_upgrade) = upstream_upgrade {
        let tunnel_span = ws_span.clone();
        tokio::spawn(async move {
            let result = async move {
                let mut client = TokioIo::new(client_upgrade.await?);
                let mut upstream = TokioIo::new(upstream_upgrade.await?);
                tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
                Ok::<(), anyhow::Error>(())
            }
            .instrument(tunnel_span.clone())
            .await;
            match result {
                Ok(()) => {
                    tunnel_span.record("decision", "allow");
                    tunnel_span.record("status", "ok");
                }
                Err(error) => {
                    tunnel_span.record("decision", "error");
                    tunnel_span.record("status", "error");
                    tunnel_span.record("error_kind", "websocket_tunnel");
                    warn!(error = %error, "websocket tunnel ended with error");
                }
            }
        });
    }

    let req_ctx = TelemetryRequestContext {
        domain: domain.to_string(),
        process_name: process_name.clone(),
        ai_provider,
        ai_protocol,
        model_traffic: false,
        method: method.clone(),
        path: path.clone(),
        query: query.clone(),
        status_code: Some(status_code),
        decision: Decision::Allowed,
        matched_rule: Some(matched_rule.clone()),
        request_headers: Some(req_hdrs),
        response_headers: Some(format_headers(&resp_parts.headers)),
        start_time,
        request_body_stats: Arc::new(Mutex::new(BodyStats::new(0))),
        max_response_body_capture: 0,
        port: upstream_port,
        conn_type,
        policy_mode: request_security_decision.policy_mode.clone(),
        policy_action: request_security_decision.policy_action.clone(),
        policy_rule: request_security_decision.policy_rule.clone(),
        policy_reason: request_security_decision.policy_reason.clone(),
        credential_ref: credential_ref.clone(),
        credential_observations: credential_observations.clone(),
        credential_injections: Vec::new(),
    };

    let empty_body = Full::new(Bytes::new()).map_err(|never| match never {}).boxed();

    Ok(hyper::Response::from_parts(
        resp_parts,
        seal_with_telemetry(empty_body, req_ctx, ai_provider, ai_protocol),
    ))
}
