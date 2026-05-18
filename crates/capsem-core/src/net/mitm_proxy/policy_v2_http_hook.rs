//! Policy V2 HTTP enforcement hook.
//!
//! Runs on `RawRequestHead` before upstream dispatch, and on
//! `RawResponseHead` after upstream response headers arrive but before
//! guest delivery and telemetry capture. It evaluates named
//! `policy.http.*` rules, can fail closed, and can mutate parsed HTTP
//! heads in place.

#![allow(dead_code)]

use std::borrow::Cow;
use std::pin::Pin;
use std::sync::Arc;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;

use super::events::{Event, EventKind, EventMask};
use super::hooks::{Hook, HookCtx, HookOutcome, StopAction};
use super::protocol::Protocol;
use super::util::split_path_query;
use crate::net::policy_v2::{
    MatchedPolicyRule, PolicyCallback, PolicyConfig, PolicyDecisionKind, PolicyRuleConfig,
    PolicySubject, PolicySubjectValue,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LastHttpPolicyV2Decision {
    pub policy_mode: Option<String>,
    pub policy_action: Option<String>,
    pub policy_rule: Option<String>,
    pub policy_reason: Option<String>,
}

impl LastHttpPolicyV2Decision {
    fn from_match(name: &str, rule: &PolicyRuleConfig) -> Self {
        Self {
            policy_mode: Some("enforce".to_string()),
            policy_action: Some(policy_action(rule.decision).to_string()),
            policy_rule: Some(format!("policy.http.{name}")),
            policy_reason: Some(
                rule.reason
                    .clone()
                    .unwrap_or_else(|| format!("Policy V2 HTTP {:?} rule matched", rule.decision)),
            ),
        }
    }
}

pub struct PolicyV2HttpHook {
    policy_v2: Arc<tokio::sync::RwLock<Arc<PolicyConfig>>>,
    confirmer: Arc<dyn crate::net::policy_confirm::Confirmer>,
    confirm_opts: capsem_proto::poll::RetryOpts,
}

impl PolicyV2HttpHook {
    pub fn new(policy_v2: Arc<tokio::sync::RwLock<Arc<PolicyConfig>>>) -> Self {
        Self {
            policy_v2,
            confirmer: Arc::new(crate::net::policy_confirm::PlaceholderConfirmer),
            confirm_opts: crate::net::policy_confirm::default_confirm_backoff("confirm-http"),
        }
    }

    pub fn with_confirmer(
        mut self,
        confirmer: Arc<dyn crate::net::policy_confirm::Confirmer>,
    ) -> Self {
        self.confirmer = confirmer;
        self
    }

    pub fn with_confirm_opts(mut self, opts: capsem_proto::poll::RetryOpts) -> Self {
        self.confirm_opts = opts;
        self
    }
}

impl Hook for PolicyV2HttpHook {
    fn name(&self) -> &'static str {
        "policy-v2-http"
    }

    fn interest(&self) -> EventMask {
        EventMask::single(EventKind::RawRequestHead) | EventMask::single(EventKind::RawResponseHead)
    }

    fn priority(&self) -> i32 {
        -900
    }

    fn on_event<'a, 'b>(
        &'a self,
        ev: &'b mut Event<'_>,
        ctx: &'b mut HookCtx<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = HookOutcome> + Send + 'b>>
    where
        'a: 'b,
    {
        let policy_v2 = Arc::clone(&self.policy_v2);
        let confirmer = Arc::clone(&self.confirmer);
        let confirm_opts = self.confirm_opts.clone();
        Box::pin(async move {
            match ev {
                Event::RawRequestHead(parts) => {
                    let subject = HttpRequestPolicySubject::from_parts(
                        ctx.conn().protocol,
                        &ctx.conn().domain,
                        parts,
                    );
                    let policy = policy_v2.read().await.clone();
                    let matched = match policy
                        .find_matching_rule(PolicyCallback::HttpRequest, &subject)
                    {
                        Ok(Some(matched)) => matched,
                        Ok(None) => return HookOutcome::Continue,
                        Err(error) => {
                            let slot = ctx.state::<LastHttpPolicyV2Decision>(
                                LastHttpPolicyV2Decision::default,
                            );
                            slot.policy_mode = Some("enforce".to_string());
                            slot.policy_action = Some("block".to_string());
                            slot.policy_rule = Some("policy.http.invalid_condition".to_string());
                            slot.policy_reason = Some(format!(
                                "Policy V2 HTTP request condition failed closed: {error}"
                            ));
                            return reject(
                                "capsem: HTTP request blocked by invalid Policy V2 rule\n",
                            );
                        }
                    };

                    let decision = LastHttpPolicyV2Decision::from_match(matched.name, matched.rule);
                    *ctx.state::<LastHttpPolicyV2Decision>(LastHttpPolicyV2Decision::default) =
                        decision.clone();

                    let resolved = resolve_policy_v2_action(
                        matched.rule.decision,
                        PolicyCallback::HttpRequest,
                        matched.name,
                        matched.rule.reason.clone(),
                        || subject_snapshot_http_request(&subject),
                        &confirmer,
                        &confirm_opts,
                    )
                    .await;
                    ctx.state::<LastHttpPolicyV2Decision>(LastHttpPolicyV2Decision::default)
                        .policy_action = Some(policy_action(resolved).to_string());

                    match resolved {
                        PolicyDecisionKind::Allow => HookOutcome::Continue,
                        PolicyDecisionKind::Ask | PolicyDecisionKind::Block => reject(&format!(
                            "capsem: HTTP request blocked by policy: {}\n",
                            decision
                                .policy_rule
                                .as_deref()
                                .unwrap_or("policy.http.unknown")
                        )),
                        PolicyDecisionKind::Rewrite => {
                            match rewrite_request(parts, matched, ctx.conn().protocol) {
                                Ok(()) => HookOutcome::Rewrote,
                                Err(error) => {
                                    let slot = ctx.state::<LastHttpPolicyV2Decision>(
                                        LastHttpPolicyV2Decision::default,
                                    );
                                    slot.policy_reason = Some(format!(
                                        "{}; rewrite failed closed: {error}",
                                        slot.policy_reason.clone().unwrap_or_default()
                                    ));
                                    reject("capsem: HTTP request rewrite blocked by policy\n")
                                }
                            }
                        }
                    }
                }
                Event::RawResponseHead(parts) => {
                    let protocol = ctx.conn().protocol;
                    let domain = ctx.conn().domain.clone();
                    let request_context = ctx
                        .state::<HttpResponsePolicyContext>(|| {
                            HttpResponsePolicyContext::from_conn(protocol, &domain)
                        })
                        .clone();
                    let subject = HttpResponsePolicySubject::from_parts(request_context, parts);
                    let policy = policy_v2.read().await.clone();
                    let matched = match policy
                        .find_matching_rule(PolicyCallback::HttpResponse, &subject)
                    {
                        Ok(Some(matched)) => matched,
                        Ok(None) => return HookOutcome::Continue,
                        Err(error) => {
                            let slot = ctx.state::<LastHttpPolicyV2Decision>(
                                LastHttpPolicyV2Decision::default,
                            );
                            slot.policy_mode = Some("enforce".to_string());
                            slot.policy_action = Some("block".to_string());
                            slot.policy_rule = Some("policy.http.invalid_condition".to_string());
                            slot.policy_reason = Some(format!(
                                "Policy V2 HTTP response condition failed closed: {error}"
                            ));
                            return reject(
                                "capsem: HTTP response blocked by invalid Policy V2 rule\n",
                            );
                        }
                    };

                    let decision = LastHttpPolicyV2Decision::from_match(matched.name, matched.rule);
                    *ctx.state::<LastHttpPolicyV2Decision>(LastHttpPolicyV2Decision::default) =
                        decision.clone();

                    let resolved = resolve_policy_v2_action(
                        matched.rule.decision,
                        PolicyCallback::HttpResponse,
                        matched.name,
                        matched.rule.reason.clone(),
                        || subject_snapshot_http_response(&subject),
                        &confirmer,
                        &confirm_opts,
                    )
                    .await;
                    ctx.state::<LastHttpPolicyV2Decision>(LastHttpPolicyV2Decision::default)
                        .policy_action = Some(policy_action(resolved).to_string());

                    match resolved {
                        PolicyDecisionKind::Allow => HookOutcome::Continue,
                        PolicyDecisionKind::Ask | PolicyDecisionKind::Block => reject(&format!(
                            "capsem: HTTP response blocked by policy: {}\n",
                            decision
                                .policy_rule
                                .as_deref()
                                .unwrap_or("policy.http.unknown")
                        )),
                        PolicyDecisionKind::Rewrite => match rewrite_response(parts, matched) {
                            Ok(()) => HookOutcome::Rewrote,
                            Err(error) => {
                                let slot = ctx.state::<LastHttpPolicyV2Decision>(
                                    LastHttpPolicyV2Decision::default,
                                );
                                slot.policy_reason = Some(format!(
                                    "{}; rewrite failed closed: {error}",
                                    slot.policy_reason.clone().unwrap_or_default()
                                ));
                                reject("capsem: HTTP response rewrite blocked by policy\n")
                            }
                        },
                    }
                }
                _ => HookOutcome::Continue,
            }
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponsePolicyContext {
    scheme: &'static str,
    host: String,
    method: String,
    path: String,
    query: Option<String>,
    url: String,
    headers: Vec<(String, String)>,
}

impl HttpResponsePolicyContext {
    pub fn from_request_parts(
        protocol: Protocol,
        host: &str,
        parts: &http::request::Parts,
    ) -> Self {
        let scheme = scheme_for_protocol(protocol);
        let (path, query) = split_path_query(&parts.uri);
        let path_and_query = parts
            .uri
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or("/");
        let headers = parts
            .headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_string(), value.to_string()))
            })
            .collect();
        Self {
            scheme,
            host: host.to_string(),
            method: parts.method.to_string(),
            path,
            query,
            url: format!("{scheme}://{host}{path_and_query}"),
            headers,
        }
    }

    fn from_conn(protocol: Protocol, host: &str) -> Self {
        let scheme = scheme_for_protocol(protocol);
        Self {
            scheme,
            host: host.to_string(),
            method: String::new(),
            path: "/".to_string(),
            query: None,
            url: format!("{scheme}://{host}/"),
            headers: Vec::new(),
        }
    }

    fn header_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug)]
struct HttpRequestPolicySubject {
    scheme: &'static str,
    host: String,
    method: String,
    path: String,
    query: Option<String>,
    url: String,
    headers: Vec<(String, String)>,
}

impl HttpRequestPolicySubject {
    fn from_parts(protocol: Protocol, host: &str, parts: &http::request::Parts) -> Self {
        let scheme = scheme_for_protocol(protocol);
        let (path, query) = split_path_query(&parts.uri);
        let path_and_query = parts
            .uri
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or("/");
        let url = format!("{scheme}://{host}{path_and_query}");
        let headers = parts
            .headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_string(), value.to_string()))
            })
            .collect();
        Self {
            scheme,
            host: host.to_string(),
            method: parts.method.to_string(),
            path,
            query,
            url,
            headers,
        }
    }

    fn header_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.as_str())
    }
}

impl PolicySubject for HttpRequestPolicySubject {
    fn get_policy_field(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        match field {
            "request.scheme" => Some(PolicySubjectValue::String(Cow::Borrowed(self.scheme))),
            "request.host" => Some(PolicySubjectValue::String(Cow::Borrowed(
                self.host.as_str(),
            ))),
            "request.method" => Some(PolicySubjectValue::String(Cow::Borrowed(
                self.method.as_str(),
            ))),
            "request.path" => Some(PolicySubjectValue::String(Cow::Borrowed(
                self.path.as_str(),
            ))),
            "request.query" => self
                .query
                .as_deref()
                .map(|value| PolicySubjectValue::String(Cow::Borrowed(value))),
            "request.url" => Some(PolicySubjectValue::String(Cow::Borrowed(self.url.as_str()))),
            "request.headers" => {
                if self.headers.is_empty() {
                    None
                } else {
                    Some(PolicySubjectValue::Present)
                }
            }
            _ => field
                .strip_prefix("request.headers.")
                .and_then(|name| self.header_value(name))
                .map(|value| PolicySubjectValue::String(Cow::Borrowed(value))),
        }
    }
}

#[derive(Debug)]
struct HttpResponsePolicySubject {
    request: HttpResponsePolicyContext,
    status: String,
    headers: Vec<(String, String)>,
}

impl HttpResponsePolicySubject {
    fn from_parts(request: HttpResponsePolicyContext, parts: &http::response::Parts) -> Self {
        let headers = parts
            .headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_string(), value.to_string()))
            })
            .collect();
        Self {
            request,
            status: parts.status.as_u16().to_string(),
            headers,
        }
    }

    fn response_header_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.as_str())
    }
}

impl PolicySubject for HttpResponsePolicySubject {
    fn get_policy_field(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        match field {
            "request.scheme" => Some(PolicySubjectValue::String(Cow::Borrowed(
                self.request.scheme,
            ))),
            "request.host" => Some(PolicySubjectValue::String(Cow::Borrowed(
                self.request.host.as_str(),
            ))),
            "request.method" => Some(PolicySubjectValue::String(Cow::Borrowed(
                self.request.method.as_str(),
            ))),
            "request.path" => Some(PolicySubjectValue::String(Cow::Borrowed(
                self.request.path.as_str(),
            ))),
            "request.query" => self
                .request
                .query
                .as_deref()
                .map(|value| PolicySubjectValue::String(Cow::Borrowed(value))),
            "request.url" => Some(PolicySubjectValue::String(Cow::Borrowed(
                self.request.url.as_str(),
            ))),
            "request.headers" => {
                if self.request.headers.is_empty() {
                    None
                } else {
                    Some(PolicySubjectValue::Present)
                }
            }
            "response.status" => Some(PolicySubjectValue::String(Cow::Borrowed(
                self.status.as_str(),
            ))),
            "response.headers" => {
                if self.headers.is_empty() {
                    None
                } else {
                    Some(PolicySubjectValue::Present)
                }
            }
            _ => field
                .strip_prefix("request.headers.")
                .and_then(|name| self.request.header_value(name))
                .or_else(|| {
                    field
                        .strip_prefix("response.headers.")
                        .and_then(|name| self.response_header_value(name))
                })
                .map(|value| PolicySubjectValue::String(Cow::Borrowed(value))),
        }
    }
}

fn rewrite_request(
    parts: &mut http::request::Parts,
    matched: MatchedPolicyRule<'_>,
    protocol: Protocol,
) -> Result<(), String> {
    for header in &matched.rule.strip_request_headers {
        parts.headers.remove(header.as_str());
    }

    let Some(target) = matched.rule.rewrite_target.as_deref() else {
        return Ok(());
    };
    let replacement = matched
        .rule
        .rewrite_value
        .as_deref()
        .ok_or_else(|| "rewrite decision missing rewrite_value".to_string())?;
    let (field, regex) = parse_regex_rewrite_target(target)?;
    match field.as_str() {
        "request.url" => rewrite_request_url(parts, protocol, &regex, replacement),
        "request.path" => rewrite_request_path(parts, &regex, replacement),
        "request.query" => rewrite_request_query(parts, &regex, replacement),
        field => {
            let Some(header) = field.strip_prefix("request.headers.") else {
                return Err(format!("unsupported HTTP request rewrite target '{field}'"));
            };
            rewrite_request_header(parts, header, &regex, replacement)
        }
    }
}

enum ResponseRewrite {
    Header(http::header::HeaderName, http::header::HeaderValue),
    Status(http::StatusCode),
}

fn rewrite_response(
    parts: &mut http::response::Parts,
    matched: MatchedPolicyRule<'_>,
) -> Result<(), String> {
    let rewrite = match matched.rule.rewrite_target.as_deref() {
        Some(target) => {
            let replacement = matched
                .rule
                .rewrite_value
                .as_deref()
                .ok_or_else(|| "rewrite decision missing rewrite_value".to_string())?;
            build_response_rewrite(parts, target, replacement)?
        }
        None => None,
    };

    for header in &matched.rule.strip_response_headers {
        parts.headers.remove(header.as_str());
    }

    match rewrite {
        Some(ResponseRewrite::Header(name, value)) => {
            parts.headers.insert(name, value);
        }
        Some(ResponseRewrite::Status(status)) => {
            parts.status = status;
        }
        None => {}
    }

    Ok(())
}

fn build_response_rewrite(
    parts: &http::response::Parts,
    target: &str,
    replacement: &str,
) -> Result<Option<ResponseRewrite>, String> {
    let (field, regex) = parse_regex_rewrite_target(target)?;
    match field.as_str() {
        "response.status" => {
            let rewritten = regex
                .replace_all(&parts.status.as_u16().to_string(), replacement)
                .to_string();
            let code: u16 = rewritten
                .parse()
                .map_err(|_| format!("rewritten HTTP response status '{rewritten}' is invalid"))?;
            let status = http::StatusCode::from_u16(code)
                .map_err(|_| format!("rewritten HTTP response status '{rewritten}' is invalid"))?;
            Ok(Some(ResponseRewrite::Status(status)))
        }
        field => {
            let Some(header) = field.strip_prefix("response.headers.") else {
                return Err(format!(
                    "unsupported HTTP response rewrite target '{field}'"
                ));
            };
            let name = http::header::HeaderName::from_bytes(header.as_bytes())
                .map_err(|_| format!("invalid HTTP response header rewrite target '{header}'"))?;
            let Some(value) = parts
                .headers
                .get(&name)
                .and_then(|value| value.to_str().ok())
            else {
                return Ok(None);
            };
            let rewritten = regex.replace_all(value, replacement).to_string();
            let value = http::header::HeaderValue::from_str(&rewritten)
                .map_err(|_| format!("rewritten HTTP response header '{header}' is invalid"))?;
            Ok(Some(ResponseRewrite::Header(name, value)))
        }
    }
}

fn rewrite_request_url(
    parts: &mut http::request::Parts,
    protocol: Protocol,
    regex: &regex::Regex,
    replacement: &str,
) -> Result<(), String> {
    let host = parts
        .headers
        .get(http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let scheme = match protocol {
        Protocol::Tls => "https",
        Protocol::Http => "http",
        Protocol::McpFrame | Protocol::Unknown => "unknown",
    };
    let current = format!(
        "{}://{}{}",
        scheme,
        host,
        parts
            .uri
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or("/")
    );
    let rewritten = regex.replace_all(&current, replacement).to_string();
    let uri: http::Uri = rewritten
        .parse()
        .map_err(|error| format!("rewritten request.url is not a valid URI: {error}"))?;
    if let Some(authority) = uri.authority() {
        let rewritten_host = authority.as_str();
        if !host.is_empty() && rewritten_host != host {
            return Err("HTTP request URL rewrite cannot change upstream host yet".to_string());
        }
    }
    set_path_query(parts, uri.path(), uri.query())
}

fn rewrite_request_path(
    parts: &mut http::request::Parts,
    regex: &regex::Regex,
    replacement: &str,
) -> Result<(), String> {
    let query = parts.uri.query().map(ToOwned::to_owned);
    let rewritten = regex.replace_all(parts.uri.path(), replacement).to_string();
    set_path_query(parts, &rewritten, query.as_deref())
}

fn rewrite_request_query(
    parts: &mut http::request::Parts,
    regex: &regex::Regex,
    replacement: &str,
) -> Result<(), String> {
    let path = parts.uri.path().to_string();
    let current = parts.uri.query().unwrap_or_default();
    let rewritten = regex.replace_all(current, replacement).to_string();
    set_path_query(parts, &path, Some(rewritten.as_str()))
}

fn rewrite_request_header(
    parts: &mut http::request::Parts,
    header: &str,
    regex: &regex::Regex,
    replacement: &str,
) -> Result<(), String> {
    let name = http::header::HeaderName::from_bytes(header.as_bytes())
        .map_err(|_| format!("invalid HTTP header rewrite target '{header}'"))?;
    let Some(value) = parts
        .headers
        .get(&name)
        .and_then(|value| value.to_str().ok())
    else {
        return Ok(());
    };
    let rewritten = regex.replace_all(value, replacement).to_string();
    let value = http::header::HeaderValue::from_str(&rewritten)
        .map_err(|_| format!("rewritten HTTP header '{header}' is invalid"))?;
    parts.headers.insert(name, value);
    Ok(())
}

fn set_path_query(
    parts: &mut http::request::Parts,
    path: &str,
    query: Option<&str>,
) -> Result<(), String> {
    if !path.starts_with('/') {
        return Err("rewritten HTTP path must start with '/'".to_string());
    }
    let path_query = match query {
        Some(query) if !query.is_empty() => format!("{path}?{query}"),
        _ => path.to_string(),
    };
    parts.uri = path_query
        .parse()
        .map_err(|error| format!("rewritten HTTP path/query is invalid: {error}"))?;
    Ok(())
}

fn scheme_for_protocol(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Http => "http",
        Protocol::Tls => "https",
        Protocol::McpFrame | Protocol::Unknown => "unknown",
    }
}

fn parse_regex_rewrite_target(target: &str) -> Result<(String, regex::Regex), String> {
    let Some((field, regex_text)) = target.split_once("=~") else {
        return Err("rewrite_target must use '<field> =~ <regex>'".into());
    };
    let field = field.trim();
    if field.is_empty() {
        return Err("rewrite_target field must not be empty".into());
    }
    let regex_text = regex_text.trim();
    if regex_text.len() < 2 {
        return Err("rewrite_target regex must be quoted".into());
    }
    let quote = regex_text.as_bytes()[0] as char;
    if quote != '"' && quote != '\'' {
        return Err("rewrite_target regex must be quoted".into());
    }
    let Some(end) = regex_text[1..].rfind(quote) else {
        return Err("rewrite_target regex is missing a closing quote".into());
    };
    let trailing = &regex_text[end + 2..];
    if !trailing.trim().is_empty() {
        return Err("rewrite_target regex has trailing content after closing quote".into());
    }
    let pattern = &regex_text[1..=end];
    let regex = regex::Regex::new(pattern)
        .map_err(|error| format!("invalid rewrite_target regex: {error}"))?;
    Ok((field.to_string(), regex))
}

fn policy_action(decision: PolicyDecisionKind) -> &'static str {
    match decision {
        PolicyDecisionKind::Allow => "allow",
        PolicyDecisionKind::Ask => "ask",
        PolicyDecisionKind::Block => "block",
        PolicyDecisionKind::Rewrite => "rewrite",
    }
}

fn reject(message: &str) -> HookOutcome {
    let body = Full::new(Bytes::from(message.to_string()))
        .map_err(|never| match never {})
        .boxed();
    let response = http::Response::builder()
        .status(http::StatusCode::FORBIDDEN)
        .header("content-type", "text/plain; charset=utf-8")
        .body(body)
        .expect("static response build");
    HookOutcome::Stop(StopAction::Reject(response))
}

async fn resolve_policy_v2_action(
    declared: PolicyDecisionKind,
    callback: PolicyCallback,
    rule_name: &str,
    reason: Option<String>,
    snapshot: impl FnOnce() -> serde_json::Value,
    confirmer: &Arc<dyn crate::net::policy_confirm::Confirmer>,
    confirm_opts: &capsem_proto::poll::RetryOpts,
) -> PolicyDecisionKind {
    if !matches!(declared, PolicyDecisionKind::Ask) {
        return declared;
    }
    let args = crate::net::policy_confirm::ConfirmArgs {
        callback,
        rule_id: http_policy_v2_rule_id(rule_name),
        args_snapshot: snapshot(),
        trace_id: None,
        session_id: None,
        reason,
    };
    match crate::net::policy_confirm::confirm_with_backoff(confirmer, args, confirm_opts).await {
        crate::net::policy_confirm::Decision::Accept => PolicyDecisionKind::Allow,
        crate::net::policy_confirm::Decision::Deny => PolicyDecisionKind::Block,
    }
}

fn http_policy_v2_rule_id(rule_name: &str) -> String {
    format!("security.rules.http.{rule_name}")
}

const SNAPSHOT_FIELD_MAX_BYTES: usize = 1024;

fn truncate_for_snapshot(value: &str) -> String {
    if value.len() <= SNAPSHOT_FIELD_MAX_BYTES {
        return value.to_string();
    }
    let mut cut = SNAPSHOT_FIELD_MAX_BYTES;
    while cut > 0 && !value.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut out = String::with_capacity(cut + 16);
    out.push_str(&value[..cut]);
    out.push_str("...[truncated]");
    out
}

fn subject_snapshot_http_request(subject: &HttpRequestPolicySubject) -> serde_json::Value {
    serde_json::json!({
        "request": {
            "host": truncate_for_snapshot(&subject.host),
            "method": subject.method,
            "path": truncate_for_snapshot(&subject.path),
            "scheme": subject.scheme,
        }
    })
}

fn subject_snapshot_http_response(subject: &HttpResponsePolicySubject) -> serde_json::Value {
    serde_json::json!({
        "request": {
            "host": truncate_for_snapshot(&subject.request.host),
            "method": subject.request.method,
            "path": truncate_for_snapshot(&subject.request.path),
        },
        "response": {
            "status": subject.status,
        }
    })
}

#[cfg(test)]
mod tests;
