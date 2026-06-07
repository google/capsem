//! Policy V2 model enforcement helpers.
//!
//! Model request rules need request-body metadata, so they cannot run
//! from the head-only HTTP policy hook. `handle_request` calls this
//! module after it has decided a request is an LLM API call and before
//! opening an upstream connection.

#![allow(dead_code)]

use std::borrow::Cow;

use crate::net::ai_traffic::events;
use crate::net::ai_traffic::provider::ProviderKind;
use crate::net::ai_traffic::request_parser::{self, RequestMeta};
use crate::net::parsers::sse_parser::SseParser;
use crate::net::policy_config::{
    PolicyCallback, PolicyConfig, PolicyDecisionKind, PolicyRuleConfig, PolicySubject,
    PolicySubjectValue,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LastModelPolicyV2Decision {
    pub policy_mode: Option<String>,
    pub policy_action: Option<String>,
    pub policy_rule: Option<String>,
    pub policy_reason: Option<String>,
}

impl LastModelPolicyV2Decision {
    fn from_match(name: &str, rule: &PolicyRuleConfig) -> Self {
        Self {
            policy_mode: Some("enforce".to_string()),
            policy_action: Some(policy_action(rule.decision).to_string()),
            policy_rule: Some(format!("policy.model.{name}")),
            policy_reason: Some(
                rule.reason
                    .clone()
                    .unwrap_or_else(|| format!("Policy V2 model {:?} rule matched", rule.decision)),
            ),
        }
    }

    fn invalid_condition(error: String) -> Self {
        Self {
            policy_mode: Some("enforce".to_string()),
            policy_action: Some("block".to_string()),
            policy_rule: Some("policy.model.invalid_condition".to_string()),
            policy_reason: Some(format!(
                "Policy V2 model request condition failed closed: {error}"
            )),
        }
    }

    fn unsupported_rewrite(mut self) -> Self {
        let existing = self.policy_reason.take().unwrap_or_default();
        self.policy_reason = Some(format!(
            "{existing}; model.request rewrite is not implemented yet"
        ));
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelRequestPolicyOutcome {
    Continue(LastModelPolicyV2Decision),
    Deny(LastModelPolicyV2Decision),
    RewriteBody {
        decision: LastModelPolicyV2Decision,
        body: Vec<u8>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelResponsePolicyOutcome {
    Continue(LastModelPolicyV2Decision),
    Deny(LastModelPolicyV2Decision),
    RewriteBody {
        decision: LastModelPolicyV2Decision,
        body: Vec<u8>,
    },
}

impl ModelResponsePolicyOutcome {
    pub fn decision(&self) -> &LastModelPolicyV2Decision {
        match self {
            Self::Continue(decision)
            | Self::Deny(decision)
            | Self::RewriteBody { decision, .. } => decision,
        }
    }
}

impl ModelRequestPolicyOutcome {
    pub fn decision(&self) -> &LastModelPolicyV2Decision {
        match self {
            Self::Continue(decision)
            | Self::Deny(decision)
            | Self::RewriteBody { decision, .. } => decision,
        }
    }
}

pub fn has_model_request_rules(policy: &PolicyConfig) -> bool {
    !policy
        .rules_for_callback(PolicyCallback::ModelRequest)
        .is_empty()
        || !policy
            .rules_for_callback(PolicyCallback::ModelToolResponse)
            .is_empty()
}

pub fn has_model_response_rules(policy: &PolicyConfig) -> bool {
    !policy
        .rules_for_callback(PolicyCallback::ModelResponse)
        .is_empty()
        || !policy
            .rules_for_callback(PolicyCallback::ModelToolCall)
            .is_empty()
}

pub fn evaluate_model_request_policy(
    policy: &PolicyConfig,
    provider: ProviderKind,
    headers: &http::HeaderMap,
    body: &[u8],
) -> Option<ModelRequestPolicyOutcome> {
    let request_meta = request_parser::parse_request(provider, body);
    let request_subject =
        ModelRequestPolicySubject::new(provider, headers, body, request_meta.clone());
    let request_outcome =
        match policy.find_matching_decision_rule(PolicyCallback::ModelRequest, &request_subject) {
            Ok(Some(matched)) => {
                let decision = LastModelPolicyV2Decision::from_match(matched.name, matched.rule);
                match matched.rule.decision {
                    PolicyDecisionKind::Action | PolicyDecisionKind::Allow => {
                        Some(ModelRequestPolicyOutcome::Continue(decision))
                    }
                    PolicyDecisionKind::Ask | PolicyDecisionKind::Block => {
                        return Some(ModelRequestPolicyOutcome::Deny(decision));
                    }
                    PolicyDecisionKind::Rewrite => {
                        return Some(ModelRequestPolicyOutcome::Deny(
                            decision.unsupported_rewrite(),
                        ));
                    }
                }
            }
            Ok(None) => None,
            Err(error) => {
                return Some(ModelRequestPolicyOutcome::Deny(
                    LastModelPolicyV2Decision::invalid_condition(error),
                ));
            }
        };

    if let Some(outcome) =
        evaluate_model_tool_response_policy(policy, provider, &request_meta, body)
    {
        return Some(outcome);
    }

    request_outcome
}

fn evaluate_model_tool_response_policy(
    policy: &PolicyConfig,
    provider: ProviderKind,
    request_meta: &RequestMeta,
    body: &[u8],
) -> Option<ModelRequestPolicyOutcome> {
    if policy
        .rules_for_callback(PolicyCallback::ModelToolResponse)
        .is_empty()
    {
        return None;
    }

    let mut allow_match = None;
    let mut deny_match = None;
    let mut rewrite_matches = Vec::new();

    for tool_result in &request_meta.tool_results {
        let subject = ModelToolResponsePolicySubject::new(provider, request_meta, tool_result);
        let matched =
            match policy.find_matching_decision_rule(PolicyCallback::ModelToolResponse, &subject) {
                Ok(Some(matched)) => matched,
                Ok(None) => continue,
                Err(error) => {
                    return Some(ModelRequestPolicyOutcome::Deny(
                        LastModelPolicyV2Decision::invalid_condition(error),
                    ));
                }
            };

        match matched.rule.decision {
            PolicyDecisionKind::Action | PolicyDecisionKind::Allow => {
                update_best_policy_match(&mut allow_match, matched.name, matched.rule);
            }
            PolicyDecisionKind::Ask | PolicyDecisionKind::Block => {
                update_best_policy_match(&mut deny_match, matched.name, matched.rule);
            }
            PolicyDecisionKind::Rewrite => {
                rewrite_matches.push((matched.name, matched.rule, tool_result));
            }
        }
    }

    if let Some((name, rule)) = deny_match {
        return Some(ModelRequestPolicyOutcome::Deny(
            LastModelPolicyV2Decision::from_match(name, rule),
        ));
    }

    if !rewrite_matches.is_empty() {
        let mut rewritten_body = body.to_vec();
        let mut rewrite_match = None;
        for (name, rule, tool_result) in rewrite_matches {
            update_best_policy_match(&mut rewrite_match, name, rule);
            rewritten_body = match rewrite_tool_response_body(
                name,
                rule,
                &rewritten_body,
                &tool_result.content_preview,
            ) {
                Ok(body) => body,
                Err(error) => {
                    return Some(ModelRequestPolicyOutcome::Deny(
                        LastModelPolicyV2Decision::from_failure(name, rule, error),
                    ));
                }
            };
        }
        let (name, rule) = rewrite_match.expect("rewrite match exists");
        return Some(ModelRequestPolicyOutcome::RewriteBody {
            decision: LastModelPolicyV2Decision::from_match(name, rule),
            body: rewritten_body,
        });
    }

    allow_match.map(|(name, rule)| {
        ModelRequestPolicyOutcome::Continue(LastModelPolicyV2Decision::from_match(name, rule))
    })
}

fn update_best_policy_match<'a>(
    best: &mut Option<(&'a str, &'a PolicyRuleConfig)>,
    name: &'a str,
    rule: &'a PolicyRuleConfig,
) {
    let replace = match best.as_ref() {
        None => true,
        Some((best_name, best_rule)) => rule
            .priority
            .cmp(&best_rule.priority)
            .then_with(|| name.cmp(best_name))
            .is_lt(),
    };
    if replace {
        *best = Some((name, rule));
    }
}

pub fn evaluate_model_response_policy(
    policy: &PolicyConfig,
    provider: ProviderKind,
    request_meta: &RequestMeta,
    body: &[u8],
) -> Option<ModelResponsePolicyOutcome> {
    let meta = parse_model_response(provider, request_meta, body);
    let mut allow_match = None;
    let mut deny_match = None;
    let mut rewrite_matches: Vec<(&str, &PolicyRuleConfig, RewriteSource)> = Vec::new();

    if !policy
        .rules_for_callback(PolicyCallback::ModelResponse)
        .is_empty()
    {
        let subject = ModelResponsePolicySubject::new(provider, request_meta, &meta);
        match policy.find_matching_decision_rule(PolicyCallback::ModelResponse, &subject) {
            Ok(Some(matched)) => match matched.rule.decision {
                PolicyDecisionKind::Action | PolicyDecisionKind::Allow => {
                    update_best_policy_match(&mut allow_match, matched.name, matched.rule);
                }
                PolicyDecisionKind::Ask | PolicyDecisionKind::Block => {
                    update_best_policy_match(&mut deny_match, matched.name, matched.rule);
                }
                PolicyDecisionKind::Rewrite => {
                    rewrite_matches.push((matched.name, matched.rule, RewriteSource::Response));
                }
            },
            Ok(None) => {}
            Err(error) => {
                return Some(ModelResponsePolicyOutcome::Deny(
                    LastModelPolicyV2Decision::invalid_condition(error),
                ));
            }
        }
    }

    for (index, tool_call) in meta.tool_calls.iter().enumerate() {
        let subject = ModelToolCallPolicySubject::new(provider, request_meta, &meta, tool_call);
        let matched =
            match policy.find_matching_decision_rule(PolicyCallback::ModelToolCall, &subject) {
                Ok(Some(matched)) => matched,
                Ok(None) => continue,
                Err(error) => {
                    return Some(ModelResponsePolicyOutcome::Deny(
                        LastModelPolicyV2Decision::invalid_condition(error),
                    ));
                }
            };
        match matched.rule.decision {
            PolicyDecisionKind::Action | PolicyDecisionKind::Allow => {
                update_best_policy_match(&mut allow_match, matched.name, matched.rule);
            }
            PolicyDecisionKind::Ask | PolicyDecisionKind::Block => {
                update_best_policy_match(&mut deny_match, matched.name, matched.rule);
            }
            PolicyDecisionKind::Rewrite => {
                rewrite_matches.push((matched.name, matched.rule, RewriteSource::ToolCall(index)));
            }
        }
    }

    if let Some((name, rule)) = deny_match {
        return Some(ModelResponsePolicyOutcome::Deny(
            LastModelPolicyV2Decision::from_match(name, rule),
        ));
    }

    if !rewrite_matches.is_empty() {
        let mut rewritten = decoded_response_body(body).unwrap_or_else(|| body.to_vec());
        let mut rewrite_match = None;
        for (name, rule, source) in rewrite_matches {
            update_best_policy_match(&mut rewrite_match, name, rule);
            rewritten = match match source {
                RewriteSource::Response => {
                    rewrite_model_response_body(name, rule, &rewritten, &meta)
                }
                RewriteSource::ToolCall(index) => {
                    rewrite_model_tool_call_body(name, rule, &rewritten, &meta.tool_calls[index])
                }
            } {
                Ok(body) => body,
                Err(error) => {
                    return Some(ModelResponsePolicyOutcome::Deny(
                        LastModelPolicyV2Decision::from_failure(name, rule, error),
                    ));
                }
            };
        }
        let (name, rule) = rewrite_match.expect("rewrite match exists");
        return Some(ModelResponsePolicyOutcome::RewriteBody {
            decision: LastModelPolicyV2Decision::from_match(name, rule),
            body: rewritten,
        });
    }

    allow_match.map(|(name, rule)| {
        ModelResponsePolicyOutcome::Continue(LastModelPolicyV2Decision::from_match(name, rule))
    })
}

#[derive(Clone, Copy)]
enum RewriteSource {
    Response,
    ToolCall(usize),
}

#[derive(Debug, Default)]
struct ModelResponseMeta {
    model: Option<String>,
    text: String,
    thinking: String,
    stop_reason: Option<String>,
    tool_calls: Vec<ModelToolCallMeta>,
}

#[derive(Debug)]
struct ModelToolCallMeta {
    call_id: String,
    name: String,
    arguments: String,
}

fn parse_model_response(
    provider: ProviderKind,
    request_meta: &RequestMeta,
    body: &[u8],
) -> ModelResponseMeta {
    let body = decoded_response_body(body).unwrap_or_else(|| body.to_vec());
    parse_sse_model_response(provider, request_meta, &body)
        .or_else(|| parse_openai_json_response(request_meta, &body))
        .unwrap_or_else(|| parse_error_json_response(request_meta, &body))
}

fn decoded_response_body(body: &[u8]) -> Option<Vec<u8>> {
    if body.len() < 2 || body[0] != 0x1f || body[1] != 0x8b {
        return None;
    }
    use flate2::read::GzDecoder;
    use std::io::Read;
    let mut decoder = GzDecoder::new(body);
    let mut decoded = Vec::new();
    decoder.read_to_end(&mut decoded).ok()?;
    Some(decoded)
}

fn parse_sse_model_response(
    provider: ProviderKind,
    request_meta: &RequestMeta,
    body: &[u8],
) -> Option<ModelResponseMeta> {
    if !body.windows(5).any(|window| window == b"data:") {
        return None;
    }
    let mut parser = SseParser::new();
    let events = parser.feed(body);
    let mut provider_parser = provider.create_parser();
    let mut llm_events = Vec::new();
    for event in &events {
        llm_events.extend(provider_parser.parse_event(event));
    }
    if llm_events.is_empty() {
        return None;
    }
    let summary = events::collect_summary(&llm_events);
    let stop_reason = summary.stop_reason.as_ref().map(|reason| match reason {
        events::StopReason::EndTurn => "end_turn".to_string(),
        events::StopReason::ToolUse => "tool_use".to_string(),
        events::StopReason::MaxTokens => "max_tokens".to_string(),
        events::StopReason::ContentFilter => "content_filter".to_string(),
        events::StopReason::Other(value) => value.clone(),
    });
    Some(ModelResponseMeta {
        model: summary.model.or_else(|| request_meta.model.clone()),
        text: summary.text,
        thinking: summary.thinking,
        stop_reason,
        tool_calls: summary
            .tool_calls
            .into_iter()
            .map(|call| ModelToolCallMeta {
                call_id: call.call_id,
                name: call.name,
                arguments: call.arguments,
            })
            .collect(),
    })
}

mod openai_response_wire {
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub struct Response {
        pub model: Option<String>,
        pub choices: Option<Vec<Choice>>,
    }

    #[derive(Deserialize)]
    pub struct Choice {
        pub message: Option<Message>,
        pub finish_reason: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct Message {
        pub content: Option<MessageContent>,
        pub tool_calls: Option<Vec<ToolCall>>,
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    pub enum MessageContent {
        Text(String),
        Parts(Vec<ContentPart>),
        Null,
    }

    #[derive(Deserialize)]
    pub struct ContentPart {
        #[serde(rename = "type")]
        pub part_type: Option<String>,
        pub text: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct ToolCall {
        pub id: Option<String>,
        pub function: Option<ToolFunction>,
    }

    #[derive(Deserialize)]
    pub struct ToolFunction {
        pub name: Option<String>,
        pub arguments: Option<String>,
    }
}

fn parse_openai_json_response(
    request_meta: &RequestMeta,
    body: &[u8],
) -> Option<ModelResponseMeta> {
    let response = serde_json::from_slice::<openai_response_wire::Response>(body).ok()?;
    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();
    let mut stop_reason = None;

    for choice in response.choices.unwrap_or_default() {
        if stop_reason.is_none() {
            stop_reason = choice.finish_reason;
        }
        let Some(message) = choice.message else {
            continue;
        };
        if let Some(content) = message.content {
            let text = match content {
                openai_response_wire::MessageContent::Text(value) => value,
                openai_response_wire::MessageContent::Parts(parts) => parts
                    .into_iter()
                    .filter_map(|part| {
                        let is_text = part
                            .part_type
                            .as_deref()
                            .is_none_or(|part_type| part_type == "text");
                        if is_text {
                            part.text
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                openai_response_wire::MessageContent::Null => String::new(),
            };
            if !text.is_empty() {
                text_parts.push(text);
            }
        }
        for tool_call in message.tool_calls.unwrap_or_default() {
            let Some(function) = tool_call.function else {
                continue;
            };
            let name = function.name.unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            tool_calls.push(ModelToolCallMeta {
                call_id: tool_call.id.unwrap_or_default(),
                name,
                arguments: function.arguments.unwrap_or_default(),
            });
        }
    }

    if text_parts.is_empty() && tool_calls.is_empty() && stop_reason.is_none() {
        return None;
    }

    Some(ModelResponseMeta {
        model: response.model.or_else(|| request_meta.model.clone()),
        text: text_parts.join("\n"),
        thinking: String::new(),
        stop_reason,
        tool_calls,
    })
}

fn parse_error_json_response(request_meta: &RequestMeta, body: &[u8]) -> ModelResponseMeta {
    #[derive(serde::Deserialize)]
    struct ErrorEnvelope {
        error: Option<ErrorBody>,
    }

    #[derive(serde::Deserialize)]
    struct ErrorBody {
        message: Option<String>,
    }

    let text = serde_json::from_slice::<ErrorEnvelope>(body)
        .ok()
        .and_then(|envelope| envelope.error)
        .and_then(|error| error.message)
        .unwrap_or_else(|| String::from_utf8_lossy(body).into_owned());
    ModelResponseMeta {
        model: request_meta.model.clone(),
        text,
        ..ModelResponseMeta::default()
    }
}

fn rewrite_model_response_body(
    name: &str,
    rule: &PolicyRuleConfig,
    body: &[u8],
    meta: &ModelResponseMeta,
) -> Result<Vec<u8>, String> {
    let target = rule
        .rewrite_target
        .as_deref()
        .ok_or_else(|| "rewrite decision missing rewrite_target".to_string())?;
    let replacement = rule
        .rewrite_value
        .as_deref()
        .ok_or_else(|| "rewrite decision missing rewrite_value".to_string())?;
    let (field, regex) = parse_regex_rewrite_target(target)?;
    let source = match field.as_str() {
        "response.text" | "text" | "content" => meta.text.as_str(),
        "thinking_content" => meta.thinking.as_str(),
        field => {
            return Err(format!(
                "unsupported model.response rewrite target '{field}'"
            ))
        }
    };
    let rewritten = regex.replace_all(source, replacement).to_string();
    if rewritten == source {
        return Err(format!(
            "policy.model.{name} rewrite_target did not match model response"
        ));
    }
    rewrite_json_string_body(body, source, &rewritten)
        .or_else(|_| rewrite_plain_text_body(body, &regex, replacement))
}

fn rewrite_model_tool_call_body(
    name: &str,
    rule: &PolicyRuleConfig,
    body: &[u8],
    tool_call: &ModelToolCallMeta,
) -> Result<Vec<u8>, String> {
    let target = rule
        .rewrite_target
        .as_deref()
        .ok_or_else(|| "rewrite decision missing rewrite_target".to_string())?;
    let replacement = rule
        .rewrite_value
        .as_deref()
        .ok_or_else(|| "rewrite decision missing rewrite_value".to_string())?;
    let (field, regex) = parse_regex_rewrite_target(target)?;
    let source: Cow<'_, str> = match field.as_str() {
        "tool.arguments" => Cow::Borrowed(tool_call.arguments.as_str()),
        "tool.name" => Cow::Borrowed(tool_call.name.as_str()),
        "tool.call_id" => Cow::Borrowed(tool_call.call_id.as_str()),
        field if field.starts_with("tool.arguments.") => {
            let suffix = field.trim_start_matches("tool.arguments.");
            Cow::Owned(
                tool_argument_field(&tool_call.arguments, suffix)
                    .unwrap_or_else(|| tool_call.arguments.clone()),
            )
        }
        field => {
            return Err(format!(
                "unsupported model.tool_call rewrite target '{field}'"
            ))
        }
    };
    let rewritten = regex.replace_all(source.as_ref(), replacement).to_string();
    if rewritten == source.as_ref() {
        return Err(format!(
            "policy.model.{name} rewrite_target did not match model tool call"
        ));
    }
    rewrite_json_string_body(body, source.as_ref(), &rewritten)
        .or_else(|_| rewrite_plain_text_body(body, &regex, replacement))
}

fn tool_argument_field(arguments: &str, field_path: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(arguments).ok()?;
    let mut current = &value;
    for part in field_path.split('.') {
        current = current.get(part)?;
    }
    match current {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
    }
}

fn rewrite_plain_text_body(
    body: &[u8],
    regex: &regex::Regex,
    replacement: &str,
) -> Result<Vec<u8>, String> {
    let body = std::str::from_utf8(body)
        .map_err(|error| format!("response body is not UTF-8 text: {error}"))?;
    let rewritten = regex.replace_all(body, replacement).to_string();
    if rewritten == body {
        return Err("rewrite_target did not match response body".to_string());
    }
    Ok(rewritten.into_bytes())
}

fn rewrite_tool_response_body(
    name: &str,
    rule: &PolicyRuleConfig,
    body: &[u8],
    content: &str,
) -> Result<Vec<u8>, String> {
    let target = rule
        .rewrite_target
        .as_deref()
        .ok_or_else(|| "rewrite decision missing rewrite_target".to_string())?;
    let replacement = rule
        .rewrite_value
        .as_deref()
        .ok_or_else(|| "rewrite decision missing rewrite_value".to_string())?;
    let (field, regex) = parse_regex_rewrite_target(target)?;
    match field.as_str() {
        "content" | "response.content" => {}
        field => {
            return Err(format!(
                "unsupported model.tool_response rewrite target '{field}'"
            ));
        }
    }

    let rewritten_content = regex.replace_all(content, replacement).to_string();
    if rewritten_content == content {
        return Err(format!(
            "policy.model.{name} rewrite_target did not match tool response content"
        ));
    }

    rewrite_json_string_body(body, content, &rewritten_content)
}

fn parse_regex_rewrite_target(target: &str) -> Result<(String, regex::Regex), String> {
    let Some((field, regex_text)) = target.split_once("=~") else {
        return Err("rewrite_target must use '<field> =~ <regex>'".to_string());
    };
    let field = field.trim();
    if field.is_empty() {
        return Err("rewrite_target field must not be empty".to_string());
    }
    let regex_text = regex_text.trim();
    if regex_text.len() < 2 {
        return Err("rewrite_target regex must be quoted".to_string());
    }
    let quote = regex_text.as_bytes()[0] as char;
    if quote != '"' && quote != '\'' {
        return Err("rewrite_target regex must be quoted".to_string());
    }
    let Some(end) = regex_text[1..].rfind(quote) else {
        return Err("rewrite_target regex is missing a closing quote".to_string());
    };
    let trailing = &regex_text[end + 2..];
    if !trailing.trim().is_empty() {
        return Err("rewrite_target regex has trailing content after closing quote".to_string());
    }
    let pattern = &regex_text[1..=end];
    let regex =
        regex::Regex::new(pattern).map_err(|error| format!("invalid rewrite regex: {error}"))?;
    Ok((field.to_string(), regex))
}

fn rewrite_json_string_body(
    body: &[u8],
    original: &str,
    rewritten: &str,
) -> Result<Vec<u8>, String> {
    let body = std::str::from_utf8(body)
        .map_err(|error| format!("request body is not UTF-8 JSON text: {error}"))?;
    let original_json = serde_json::to_string(original)
        .map_err(|error| format!("failed to encode original tool response content: {error}"))?;
    let rewritten_json = serde_json::to_string(rewritten)
        .map_err(|error| format!("failed to encode rewritten tool response content: {error}"))?;
    if !body.contains(&original_json) {
        return Err("original tool response content was not found in request body".to_string());
    }
    Ok(body.replace(&original_json, &rewritten_json).into_bytes())
}

#[derive(Debug)]
struct ModelRequestPolicySubject {
    provider: &'static str,
    protocol: &'static str,
    request_meta: RequestMeta,
    body: String,
    headers: Vec<(String, String)>,
}

impl ModelRequestPolicySubject {
    fn new(
        provider: ProviderKind,
        headers: &http::HeaderMap,
        body: &[u8],
        request_meta: RequestMeta,
    ) -> Self {
        let headers = headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_string(), value.to_string()))
            })
            .collect();
        Self {
            provider: provider.as_str(),
            protocol: provider.as_str(),
            request_meta,
            body: String::from_utf8_lossy(body).into_owned(),
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

impl PolicySubject for ModelRequestPolicySubject {
    fn get_policy_field(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        match field {
            "provider" => Some(PolicySubjectValue::String(Cow::Borrowed(self.provider))),
            "protocol" => Some(PolicySubjectValue::String(Cow::Borrowed(self.protocol))),
            "endpoint" => None,
            "model" => self
                .request_meta
                .model
                .as_deref()
                .map(|value| PolicySubjectValue::String(Cow::Borrowed(value))),
            "system_prompt" => self
                .request_meta
                .system_prompt_preview
                .as_deref()
                .map(|value| PolicySubjectValue::String(Cow::Borrowed(value))),
            "request.body" => Some(PolicySubjectValue::String(Cow::Borrowed(
                self.body.as_str(),
            ))),
            "request.headers" => {
                if self.headers.is_empty() {
                    None
                } else {
                    Some(PolicySubjectValue::Present)
                }
            }
            "messages_count" => Some(PolicySubjectValue::String(Cow::Owned(
                self.request_meta.messages_count.to_string(),
            ))),
            "tools_count" => Some(PolicySubjectValue::String(Cow::Owned(
                self.request_meta.tools_count.to_string(),
            ))),
            "messages" => {
                if self.request_meta.messages_count == 0 {
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

struct ModelResponsePolicySubject<'a> {
    provider: &'static str,
    request_meta: &'a RequestMeta,
    response_meta: &'a ModelResponseMeta,
}

impl<'a> ModelResponsePolicySubject<'a> {
    fn new(
        provider: ProviderKind,
        request_meta: &'a RequestMeta,
        response_meta: &'a ModelResponseMeta,
    ) -> Self {
        Self {
            provider: provider.as_str(),
            request_meta,
            response_meta,
        }
    }
}

impl PolicySubject for ModelResponsePolicySubject<'_> {
    fn get_policy_field(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        match field {
            "provider" => Some(PolicySubjectValue::String(Cow::Borrowed(self.provider))),
            "model" => self
                .response_meta
                .model
                .as_deref()
                .or(self.request_meta.model.as_deref())
                .map(|value| PolicySubjectValue::String(Cow::Borrowed(value))),
            "response.text" | "text" | "content" => Some(PolicySubjectValue::String(
                Cow::Borrowed(self.response_meta.text.as_str()),
            )),
            "thinking_content" => Some(PolicySubjectValue::String(Cow::Borrowed(
                self.response_meta.thinking.as_str(),
            ))),
            "stop_reason" => self
                .response_meta
                .stop_reason
                .as_deref()
                .map(|value| PolicySubjectValue::String(Cow::Borrowed(value))),
            "response" => {
                if self.response_meta.text.is_empty()
                    && self.response_meta.thinking.is_empty()
                    && self.response_meta.tool_calls.is_empty()
                {
                    None
                } else {
                    Some(PolicySubjectValue::Present)
                }
            }
            _ => None,
        }
    }
}

struct ModelToolCallPolicySubject<'a> {
    provider: &'static str,
    request_meta: &'a RequestMeta,
    response_meta: &'a ModelResponseMeta,
    tool_call: &'a ModelToolCallMeta,
}

impl<'a> ModelToolCallPolicySubject<'a> {
    fn new(
        provider: ProviderKind,
        request_meta: &'a RequestMeta,
        response_meta: &'a ModelResponseMeta,
        tool_call: &'a ModelToolCallMeta,
    ) -> Self {
        Self {
            provider: provider.as_str(),
            request_meta,
            response_meta,
            tool_call,
        }
    }
}

impl PolicySubject for ModelToolCallPolicySubject<'_> {
    fn get_policy_field(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        match field {
            "provider" => Some(PolicySubjectValue::String(Cow::Borrowed(self.provider))),
            "model" => self
                .response_meta
                .model
                .as_deref()
                .or(self.request_meta.model.as_deref())
                .map(|value| PolicySubjectValue::String(Cow::Borrowed(value))),
            "tool.name" => Some(PolicySubjectValue::String(Cow::Borrowed(
                self.tool_call.name.as_str(),
            ))),
            "tool.call_id" => Some(PolicySubjectValue::String(Cow::Borrowed(
                self.tool_call.call_id.as_str(),
            ))),
            "tool.arguments" => {
                if self.tool_call.arguments.is_empty() {
                    None
                } else {
                    Some(PolicySubjectValue::Present)
                }
            }
            _ => field.strip_prefix("tool.arguments.").and_then(|suffix| {
                tool_argument_field(&self.tool_call.arguments, suffix)
                    .map(|value| PolicySubjectValue::String(Cow::Owned(value)))
            }),
        }
    }
}

struct ModelToolResponsePolicySubject<'a> {
    provider: &'static str,
    request_meta: &'a RequestMeta,
    tool_result: &'a request_parser::ToolResultMeta,
}

impl<'a> ModelToolResponsePolicySubject<'a> {
    fn new(
        provider: ProviderKind,
        request_meta: &'a RequestMeta,
        tool_result: &'a request_parser::ToolResultMeta,
    ) -> Self {
        Self {
            provider: provider.as_str(),
            request_meta,
            tool_result,
        }
    }
}

impl PolicySubject for ModelToolResponsePolicySubject<'_> {
    fn get_policy_field(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        match field {
            "provider" => Some(PolicySubjectValue::String(Cow::Borrowed(self.provider))),
            "model" => self
                .request_meta
                .model
                .as_deref()
                .map(|value| PolicySubjectValue::String(Cow::Borrowed(value))),
            "tool.call_id" => Some(PolicySubjectValue::String(Cow::Borrowed(
                self.tool_result.call_id.as_str(),
            ))),
            "content" | "response.content" => Some(PolicySubjectValue::String(Cow::Borrowed(
                self.tool_result.content_preview.as_str(),
            ))),
            "response" => {
                if self.tool_result.content_preview.is_empty() {
                    None
                } else {
                    Some(PolicySubjectValue::Present)
                }
            }
            "is_error" => Some(PolicySubjectValue::Bool(self.tool_result.is_error)),
            _ => None,
        }
    }
}

impl LastModelPolicyV2Decision {
    fn from_failure(name: &str, rule: &PolicyRuleConfig, error: String) -> Self {
        let mut decision = Self::from_match(name, rule);
        let base = decision.policy_reason.clone().unwrap_or_default();
        decision.policy_reason = Some(format!("{base}; policy failed closed: {error}"));
        decision
    }
}

fn policy_action(decision: PolicyDecisionKind) -> &'static str {
    match decision {
        PolicyDecisionKind::Action => "action",
        PolicyDecisionKind::Allow => "allow",
        PolicyDecisionKind::Ask => "ask",
        PolicyDecisionKind::Block => "block",
        PolicyDecisionKind::Rewrite => "rewrite",
    }
}

#[cfg(test)]
mod tests;
