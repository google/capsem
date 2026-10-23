//! HTTP facts and owned request-body state for the shared policy boundary.
use super::*;

pub(super) struct HttpRequestSecurityEventInput<'a> {
    pub(super) domain: &'a str,
    pub(super) upstream_ip: Option<IpAddr>,
    pub(super) upstream_port: u16,
    pub(super) method: &'a str,
    pub(super) path: &'a str,
    pub(super) query: Option<String>,
    pub(super) ai_provider: Option<ProviderKind>,
    pub(super) headers: http::HeaderMap,
    pub(super) body: Option<&'a Bytes>,
}

pub(super) fn http_request_security_event(input: HttpRequestSecurityEventInput<'_>) -> SecurityEvent {
    let body = input
        .body
        .and_then(|body| std::str::from_utf8(body).ok().map(ToOwned::to_owned));
    let mut event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_http(HttpSecurityEvent {
            host: Some(input.domain.to_string()),
            method: Some(input.method.to_string()),
            path: Some(input.path.to_string()),
            query: input.query.clone(),
            status: None,
            body,
        })
        .with_http_request(crate::security_engine::HttpRequestSecurityEvent::new(
            input.domain,
            input.ai_provider,
            input.headers,
            input.query,
        ));
    event.http_body_utf8_valid = input.body.map(|body| std::str::from_utf8(body).is_ok());
    security_event_with_transport(event, input.upstream_ip, input.upstream_port)
}

pub(super) enum RequestBodySource {
    Incoming(hyper::body::Incoming),
    Collected(Bytes),
}

pub(super) fn collected_request_body_stats(
    request_body_source: &RequestBodySource,
    max_body_capture: usize,
) -> Arc<Mutex<BodyStats>> {
    let mut stats = BodyStats::new(max_body_capture);
    if let RequestBodySource::Collected(body) = request_body_source {
        stats.bytes = body.len() as u64;
        let to_copy = max_body_capture.min(body.len());
        stats.preview.extend_from_slice(&body[..to_copy]);
    }
    Arc::new(Mutex::new(stats))
}

pub(super) fn ai_protocol_for_body_preview(body: &[u8]) -> Option<ModelProtocol> {
    if body.len() > AI_BODY_CAPTURE_LIMIT {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(body).ok()?;
    let obj = json.as_object()?;
    let model = obj.get("model").and_then(|value| value.as_str());
    let has_messages = obj.get("messages").and_then(|value| value.as_array()).is_some();
    let has_google_contents = obj.get("contents").and_then(|value| value.as_array()).is_some()
        || obj.contains_key("generationConfig")
        || obj.contains_key("safetySettings");

    if has_google_contents || model.is_some_and(is_google_model_name) {
        return Some(ModelProtocol::Google);
    }
    if model.is_some_and(is_anthropic_model_name) || (has_messages && obj.contains_key("max_tokens")) {
        return Some(ModelProtocol::Anthropic);
    }
    if model.is_some_and(is_openai_model_name)
        || obj.contains_key("input")
        || obj.contains_key("response_format")
        || obj.contains_key("stream_options")
        || (has_messages && obj.contains_key("tools"))
    {
        return Some(ModelProtocol::OpenAi);
    }
    None
}

pub(super) fn should_sniff_unknown_model_body(
    ai_provider: Option<ProviderKind>,
    method: &http::Method,
    headers: &http::HeaderMap,
) -> bool {
    if ai_provider.is_some() {
        return false;
    }
    if !matches!(*method, http::Method::POST | http::Method::PUT | http::Method::PATCH) {
        return false;
    }
    let is_json = headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().contains("json"))
        .unwrap_or(false);
    if !is_json {
        return false;
    }
    let Some(len) = headers
        .get(http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
    else {
        return false;
    };
    len <= AI_BODY_CAPTURE_LIMIT
}

pub(super) fn body_capture_limit(
    ai_provider: Option<ProviderKind>,
    domain: &str,
    path: &str,
    log_bodies: bool,
    max_body: usize,
) -> usize {
    if ai_provider.is_some() {
        return AI_BODY_CAPTURE_LIMIT.max(max_body);
    }
    if log_bodies {
        return max_body;
    }
    if crate::credential_broker::is_http_body_credential_candidate(domain, path) {
        return CREDENTIAL_BODY_CAPTURE_LIMIT;
    }
    0
}

pub(super) fn response_body_capture_limit(
    ai_provider: Option<ProviderKind>,
    domain: &str,
    path: &str,
    log_bodies: bool,
    max_body: usize,
    credential_ref: Option<&str>,
) -> usize {
    let cap = body_capture_limit(ai_provider, domain, path, log_bodies, max_body);
    if credential_ref.is_some() {
        cap.max(CREDENTIAL_BODY_CAPTURE_LIMIT)
    } else {
        cap
    }
}

pub(super) fn ai_identity_for_target_or_path(
    registry: &crate::net::policy_config::ModelEndpointRegistry,
    domain: &str,
    upstream_port: u16,
    path: &str,
) -> ModelTrafficIdentity {
    let path_protocol = route_provider(path).map(|(protocol, _)| protocol);
    let endpoint_provider = registry.provider_for_target(domain, upstream_port);
    let endpoint_protocol = registry.protocol_for_target(domain, upstream_port);
    ModelTrafficIdentity {
        provider: endpoint_provider.or_else(|| path_protocol.map(|_| ProviderKind::Unknown)),
        protocol: path_protocol.or(endpoint_protocol),
    }
}
