//! Connection-owned HTTP pipeline and MCP completion drain.
use super::*;
use http_body_util::BodyExt;

/// Drive the hyper HTTP/1.1 server over the supplied IO. The service
/// closure resolves the per-request `(domain, upstream_port)`:
/// * TLS: connection-level SNI domain + 443 (constant per connection).
/// * HTTP: parsed from the inbound `Host` header per request; falls
///   back to `("", 80)` when the header is missing or malformed,
///   producing a 502 downstream once `handle_request` runs.
#[derive(Clone)]
pub(super) enum PipelineTarget {
    Dynamic {
        connection_domain: String,
        protocol: Protocol,
    },
    Standalone(StandaloneModelTarget),
}

pub(super) async fn serve_pipeline<IO>(
    io: IO,
    target: PipelineTarget,
    config: &Arc<MitmProxyConfig>,
    process_name: Arc<Option<String>>,
) where
    IO: hyper::rt::Read + hyper::rt::Write + Send + Unpin + 'static,
{
    let upstream_tls = Arc::clone(&config.upstream_tls);
    let config_arc = Arc::clone(config);

    // Per-connection upstream sender cache: each MITM connection
    // serves one upstream via keep-alive, so caching the sender
    // avoids re-establishing TCP[+TLS] for every request on the
    // same connection.
    let cached_upstream: Arc<UpstreamCache> = Arc::default();
    let mcp_responses = Arc::new(mcp_response::Tasks::until_stopped(config.shutdown.child_token()));
    let request_mcp_responses = Arc::clone(&mcp_responses);

    let svc = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
        let upstream_tls = Arc::clone(&upstream_tls);
        let target = target.clone();
        let config_arc = Arc::clone(&config_arc);
        let process_name = Arc::clone(&process_name);
        let cached_upstream = Arc::clone(&cached_upstream);
        let mcp_responses = Arc::clone(&request_mcp_responses);

        async move {
            // This adapter is an origin endpoint, never a forward proxy or
            // CONNECT tunnel. Reject the original URI before path extraction
            // discards its authority and before policy, credentials or dialing.
            if matches!(&target, PipelineTarget::Standalone(_))
                && (req.method() == hyper::Method::CONNECT
                    || req.uri().scheme().is_some()
                    || req.uri().authority().is_some()
                    || !req.uri().path().starts_with('/'))
            {
                let body = Full::new(Bytes::from_static(
                    b"standalone endpoint requires an origin-form request",
                ))
                .map_err(|never| match never {})
                .boxed();
                return Ok(hyper::Response::builder()
                    .status(400)
                    .body(body)
                    .expect("static response"));
            }
            let (request_domain, upstream_port, protocol, upstream_protocol, authoritative_host, fixed_identity) =
                match target {
                    PipelineTarget::Dynamic {
                        connection_domain,
                        protocol,
                    } => {
                        let (domain, port) = match protocol {
                            Protocol::Tls => (connection_domain, 443u16),
                            Protocol::Http => {
                                parse_http_host_target(req.headers().get("host")).unwrap_or_else(|| (String::new(), 80))
                            }
                            Protocol::McpFrame => unreachable!("framed MCP bypasses HTTP pipeline"),
                            Protocol::Unknown => (String::new(), 0),
                        };
                        (domain, port, protocol, protocol, protocol == Protocol::Tls, None)
                    }
                    PipelineTarget::Standalone(target) => (
                        target.domain().to_string(),
                        target.port(),
                        Protocol::Http,
                        target.upstream_protocol(),
                        true,
                        Some(ModelTrafficIdentity {
                            provider: Some(target.provider()),
                            protocol: Some(target.model_protocol()),
                        }),
                    ),
                };
            if fixed_identity.is_none() {
                // Capsem's own zone is answered here, before any HTTP policy, body sniff or dial.
                if let Some(route) = mcp_http::internal_route(&request_domain, upstream_port, protocol) {
                    let (domain, port) = (&request_domain, upstream_port);
                    return Ok(mcp_http::serve(
                        req,
                        route,
                        (domain, port),
                        protocol,
                        &config_arc,
                        &process_name,
                        &mcp_responses,
                    )
                    .await);
                }
            };
            let policy_snapshot = config_arc.engine.policy().snapshot();
            let ai_identity = fixed_identity.unwrap_or_else(|| {
                ai_identity_for_target_or_path(
                    policy_snapshot.model_endpoints(),
                    &request_domain,
                    upstream_port,
                    req.uri().path(),
                )
            });
            let completion = Arc::clone(&mcp_responses);
            completion
                .run_http(move |cancellation| async move {
                    handle_request(
                        req,
                        &request_domain,
                        protocol,
                        upstream_protocol,
                        authoritative_host,
                        upstream_port,
                        &upstream_tls,
                        &config_arc,
                        &process_name,
                        ai_identity.provider,
                        ai_identity.protocol,
                        &cached_upstream,
                        policy_snapshot,
                        &mcp_responses,
                        &cancellation,
                    )
                    .await
                })
                .await
        }
    });

    // hyper's header read timeout only exists when the builder has a timer;
    // without one a guest sending `G` and stalling pinned the task forever.
    let result = {
        let connection = hyper::server::conn::http1::Builder::new()
            .timer(hyper_util::rt::TokioTimer::new())
            .header_read_timeout(CLASSIFY_READ_TIMEOUT)
            .serve_connection(io, svc)
            .with_upgrades();
        tokio::pin!(connection);
        tokio::select! {
            result = &mut connection => result,
            _ = config.shutdown.cancelled() => {
                connection.as_mut().graceful_shutdown();
                connection.await
            }
        }
    };
    if let Err(e) = result {
        // Connection errors are expected when the guest closes.
        let err_str = e.to_string();
        if !e.is_incomplete_message() && !err_str.contains("error shutting down connection") {
            warn!(error = %e, "hyper serve error");
        }
    }
    mcp_responses.drain().await;
}
