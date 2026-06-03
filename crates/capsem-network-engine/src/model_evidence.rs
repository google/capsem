//! Projection from the existing provider parsers into the canonical S08 AI
//! interaction evidence contract.

use capsem_security_engine::{
    AiApiFamily, AiAttributionScope, AiContentBlock, AiContentKind, AiOriginKind, AiProvider,
    AiUsageEvidence, ArgumentsStatus, Confidence, EvidenceStatus, McpToolExecutionEvidence,
    ModelInteractionEvidence, ModelRequestEvidence, ModelResponseEvidence, ModelToolCallEvidence,
    ModelToolResultEvidence, ParseStatus, SourceEngine, ToolCallStatus, ToolOrigin,
};

use crate::ai_provider::{extract_model_from_path, tool_origin, ProviderKind};
use crate::model_request::RequestMeta;
use crate::model_stream::{StopReason, StreamSummary};

#[derive(Debug, Clone)]
pub struct ModelEvidenceInput<'a> {
    pub interaction_id: &'a str,
    pub trace_id: &'a str,
    pub request_id: &'a str,
    pub response_id: Option<&'a str>,
    pub provider: ProviderKind,
    pub path: &'a str,
    pub request: &'a RequestMeta,
    pub response: Option<&'a StreamSummary>,
    pub response_required: bool,
    pub estimated_cost_micros: Option<u64>,
    pub attribution_scope: AiAttributionScope,
    pub source_engine: SourceEngine,
    pub origin_kind: AiOriginKind,
    pub accounting_owner: Option<&'a str>,
    pub profile_id: Option<&'a str>,
    pub vm_id: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub user_id: Option<&'a str>,
}

pub fn build_model_interaction_evidence(input: ModelEvidenceInput<'_>) -> ModelInteractionEvidence {
    let provider = ai_provider(input.provider);
    let api_family = ai_api_family(input.provider, input.path);
    let response_model = input.response.and_then(|summary| summary.model.clone());
    let model = input
        .request
        .model
        .clone()
        .or(response_model)
        .or_else(|| extract_model_from_path(input.path))
        .unwrap_or_else(|| "unknown".to_string());
    let usage = usage_evidence(input.response, input.estimated_cost_micros);
    let parse_status =
        interaction_parse_status(input.request, input.response, input.response_required);
    let response = input.response.map(|summary| {
        response_evidence(
            input.response_id.unwrap_or(input.interaction_id),
            input.provider,
            input.path,
            summary,
            usage.clone(),
        )
    });

    ModelInteractionEvidence {
        interaction_id: input.interaction_id.to_string(),
        trace_id: input.trace_id.to_string(),
        attribution_scope: input.attribution_scope,
        source_engine: input.source_engine,
        origin_kind: input.origin_kind,
        accounting_owner: input.accounting_owner.map(str::to_string),
        profile_id: input.profile_id.map(str::to_string),
        vm_id: input.vm_id.map(str::to_string),
        session_id: input.session_id.map(str::to_string),
        user_id: input.user_id.map(str::to_string),
        provider,
        api_family,
        model: model.clone(),
        request: ModelRequestEvidence {
            request_id: input.request_id.to_string(),
            provider,
            api_family,
            model: Some(model),
            stream: input.request.stream
                || input.path.contains("stream")
                || input.path.contains("streamGenerateContent"),
            system_prompt_preview: input.request.system_prompt_preview.clone(),
            message_count: input.request.messages_count as u64,
            tools_declared_count: input.request.tools_count as u64,
            raw_shape_version: raw_shape_version(input.provider, input.path).to_string(),
            unknown_fields_present: false,
        },
        response,
        tool_calls: input.response.map(tool_call_evidence).unwrap_or_default(),
        tool_results: tool_result_evidence(input.request),
        mcp_executions: Vec::<McpToolExecutionEvidence>::new(),
        usage,
        parse_status,
        evidence_status: evidence_status(parse_status),
    }
}

fn response_evidence(
    response_id: &str,
    provider: ProviderKind,
    path: &str,
    summary: &StreamSummary,
    usage: AiUsageEvidence,
) -> ModelResponseEvidence {
    ModelResponseEvidence {
        response_id: response_id.to_string(),
        provider_response_id: summary.message_id.clone(),
        stop_reason: summary.stop_reason.as_ref().map(stop_reason_value),
        text_preview: (!summary.text.is_empty()).then(|| summary.text.clone()),
        thinking_preview: (!summary.thinking.is_empty()).then(|| summary.thinking.clone()),
        content_blocks: content_blocks(summary),
        usage,
        raw_shape_version: raw_shape_version(provider, path).to_string(),
    }
}

fn tool_call_evidence(summary: &StreamSummary) -> Vec<ModelToolCallEvidence> {
    summary
        .tool_calls
        .iter()
        .map(|call| {
            let origin = canonical_tool_origin(&call.name);
            ModelToolCallEvidence {
                tool_call_id: call.call_id.clone(),
                index: call.index as u64,
                provider_call_id: Some(call.call_id.clone()),
                raw_name: call.name.clone(),
                normalized_name: normalize_tool_name(&call.name),
                arguments_raw: (!call.arguments.is_empty()).then(|| call.arguments.clone()),
                arguments_json: argument_json(&call.arguments),
                arguments_status: arguments_status(&call.arguments),
                origin,
                linked_mcp_call_id: None,
                status: ToolCallStatus::Proposed,
                parse_confidence: if origin == ToolOrigin::McpTool {
                    Confidence::Medium
                } else {
                    Confidence::High
                },
            }
        })
        .collect()
}

fn tool_result_evidence(request: &RequestMeta) -> Vec<ModelToolResultEvidence> {
    request
        .tool_results
        .iter()
        .map(|result| ModelToolResultEvidence {
            tool_call_id: result.call_id.clone(),
            linked_mcp_call_id: None,
            content_kind: content_kind(&result.content_preview),
            content_preview: Some(result.content_preview.clone()),
            content_json: argument_json(&result.content_preview),
            is_error: result.is_error,
            result_status: if result.is_error {
                ToolCallStatus::Error
            } else {
                ToolCallStatus::ReturnedToModel
            },
            returned_to_model: true,
            parse_confidence: Confidence::High,
        })
        .collect()
}

fn content_blocks(summary: &StreamSummary) -> Vec<AiContentBlock> {
    let mut blocks = Vec::new();
    if !summary.thinking.is_empty() {
        blocks.push(AiContentBlock::Reasoning {
            text_preview: summary.thinking.clone(),
        });
    }
    if !summary.text.is_empty() {
        blocks.push(AiContentBlock::Text {
            text_preview: summary.text.clone(),
        });
    }
    blocks.extend(
        summary
            .tool_calls
            .iter()
            .map(|call| AiContentBlock::ToolUse {
                tool_call_id: call.call_id.clone(),
                name: call.name.clone(),
            }),
    );
    blocks
}

fn usage_evidence(
    summary: Option<&StreamSummary>,
    estimated_cost_micros: Option<u64>,
) -> AiUsageEvidence {
    let Some(summary) = summary else {
        return AiUsageEvidence {
            estimated_cost_micros,
            ..Default::default()
        };
    };
    AiUsageEvidence {
        input_tokens: summary.input_tokens,
        output_tokens: summary.output_tokens,
        estimated_cost_micros,
        details: summary.usage_details.clone(),
    }
}

pub fn arguments_status(arguments: &str) -> ArgumentsStatus {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return ArgumentsStatus::Absent;
    }
    if !looks_like_json(trimmed) {
        return ArgumentsStatus::NotJson;
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(_) => ArgumentsStatus::ValidJson,
        Err(error) if error.classify() == serde_json::error::Category::Eof => {
            ArgumentsStatus::PartialJson
        }
        Err(_) => ArgumentsStatus::MalformedJson,
    }
}

fn argument_json(arguments: &str) -> Option<String> {
    (arguments_status(arguments) == ArgumentsStatus::ValidJson).then(|| arguments.to_string())
}

fn looks_like_json(value: &str) -> bool {
    matches!(
        value.as_bytes().first().copied(),
        Some(b'{')
            | Some(b'[')
            | Some(b'"')
            | Some(b't')
            | Some(b'f')
            | Some(b'n')
            | Some(b'-')
            | Some(b'0'..=b'9')
    )
}

fn content_kind(value: &str) -> AiContentKind {
    if arguments_status(value) == ArgumentsStatus::ValidJson {
        AiContentKind::Json
    } else {
        AiContentKind::Text
    }
}

fn canonical_tool_origin(name: &str) -> ToolOrigin {
    match tool_origin(name) {
        "local" => ToolOrigin::LocalBuiltinTool,
        "mcp_proxy" => ToolOrigin::McpTool,
        "native" => ToolOrigin::NativeProviderTool,
        _ => ToolOrigin::Unknown,
    }
}

fn normalize_tool_name(name: &str) -> String {
    name.replace("__", ".")
}

fn interaction_parse_status(
    request: &RequestMeta,
    response: Option<&StreamSummary>,
    response_required: bool,
) -> ParseStatus {
    let has_partial_tool_arguments = response
        .map(|summary| {
            summary
                .tool_calls
                .iter()
                .any(|call| arguments_status(&call.arguments) == ArgumentsStatus::PartialJson)
        })
        .unwrap_or(false);
    let missing_model = request.model.is_none()
        && response
            .and_then(|summary| summary.model.as_ref())
            .is_none();
    let missing_required_response =
        response_required && !response.is_some_and(response_summary_has_signal);

    if has_partial_tool_arguments || missing_model || missing_required_response {
        ParseStatus::Partial
    } else {
        ParseStatus::Complete
    }
}

fn response_summary_has_signal(summary: &StreamSummary) -> bool {
    summary.message_id.is_some()
        || summary.model.is_some()
        || !summary.text.is_empty()
        || !summary.thinking.is_empty()
        || !summary.tool_calls.is_empty()
        || summary.input_tokens.is_some()
        || summary.output_tokens.is_some()
        || !summary.usage_details.is_empty()
        || summary.stop_reason.is_some()
}

fn evidence_status(parse_status: ParseStatus) -> EvidenceStatus {
    match parse_status {
        ParseStatus::Complete => EvidenceStatus::Complete,
        ParseStatus::Partial => EvidenceStatus::Partial,
        ParseStatus::Malformed => EvidenceStatus::Untrusted,
        ParseStatus::Unsupported | ParseStatus::Redacted => EvidenceStatus::Partial,
    }
}

fn ai_provider(provider: ProviderKind) -> AiProvider {
    match provider {
        ProviderKind::Anthropic => AiProvider::Anthropic,
        ProviderKind::OpenAi => AiProvider::Openai,
        ProviderKind::Google => AiProvider::GoogleGemini,
    }
}

fn ai_api_family(provider: ProviderKind, path: &str) -> AiApiFamily {
    match provider {
        ProviderKind::Anthropic => AiApiFamily::AnthropicMessages,
        ProviderKind::OpenAi if path.starts_with("/v1/responses") => AiApiFamily::OpenaiResponses,
        ProviderKind::OpenAi => AiApiFamily::OpenaiChatCompletions,
        ProviderKind::Google => AiApiFamily::GoogleGeminiContent,
    }
}

fn raw_shape_version(provider: ProviderKind, path: &str) -> &'static str {
    match ai_api_family(provider, path) {
        AiApiFamily::AnthropicMessages => "anthropic.messages.current",
        AiApiFamily::OpenaiResponses => "openai.responses.current",
        AiApiFamily::OpenaiChatCompletions => "openai.chat_completions.current",
        AiApiFamily::GoogleGeminiContent => "google.gemini_content.current",
        AiApiFamily::Mcp | AiApiFamily::Unknown => "unknown",
    }
}

fn stop_reason_value(reason: &StopReason) -> String {
    match reason {
        StopReason::EndTurn => "end_turn".to_string(),
        StopReason::ToolUse => "tool_use".to_string(),
        StopReason::MaxTokens => "max_tokens".to_string(),
        StopReason::ContentFilter => "content_filter".to_string(),
        StopReason::Other(value) => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use capsem_security_engine::{AiAttributionScope, AiOriginKind, SourceEngine};

    use super::*;
    use crate::model_request::ToolResultMeta;
    use crate::model_stream::{StopReason, StreamSummary, ToolCall};

    #[test]
    fn openai_stream_summary_projects_tool_call_evidence() {
        let request = RequestMeta {
            model: Some("gpt-5.5".into()),
            stream: true,
            system_prompt_preview: Some("system".into()),
            messages_count: 2,
            tools_count: 1,
            tool_results: Vec::new(),
        };
        let summary = StreamSummary {
            message_id: Some("chatcmpl-1".into()),
            model: Some("gpt-5.5".into()),
            text: "checking".into(),
            thinking: String::new(),
            tool_calls: vec![ToolCall {
                index: 0,
                call_id: "call-1".into(),
                name: "github__search".into(),
                arguments: r#"{"query":"capsem"}"#.into(),
            }],
            input_tokens: Some(100),
            output_tokens: Some(20),
            usage_details: BTreeMap::new(),
            stop_reason: Some(StopReason::ToolUse),
        };

        let evidence = build_model_interaction_evidence(input(
            ProviderKind::OpenAi,
            "/v1/chat/completions",
            &request,
            Some(&summary),
        ));

        assert_eq!(evidence.provider, AiProvider::Openai);
        assert_eq!(evidence.api_family, AiApiFamily::OpenaiChatCompletions);
        assert_eq!(evidence.tool_calls[0].origin, ToolOrigin::McpTool);
        assert_eq!(
            evidence.tool_calls[0].arguments_status,
            ArgumentsStatus::ValidJson
        );
        assert_eq!(
            evidence.response.as_ref().unwrap().stop_reason.as_deref(),
            Some("tool_use")
        );
        assert!(evidence.charges_vm_accounting());
    }

    #[test]
    fn openai_responses_path_projects_responses_api_family() {
        let request = RequestMeta {
            model: Some("gpt-5.5".into()),
            stream: false,
            system_prompt_preview: None,
            messages_count: 1,
            tools_count: 0,
            tool_results: Vec::new(),
        };
        let summary = StreamSummary {
            message_id: Some("resp-1".into()),
            model: Some("gpt-5.5".into()),
            text: "done".into(),
            thinking: String::new(),
            tool_calls: Vec::new(),
            input_tokens: Some(10),
            output_tokens: Some(2),
            usage_details: BTreeMap::new(),
            stop_reason: Some(StopReason::EndTurn),
        };

        let evidence = build_model_interaction_evidence(input(
            ProviderKind::OpenAi,
            "/v1/responses",
            &request,
            Some(&summary),
        ));

        assert_eq!(evidence.provider, AiProvider::Openai);
        assert_eq!(evidence.api_family, AiApiFamily::OpenaiResponses);
        assert_eq!(
            evidence.request.raw_shape_version,
            "openai.responses.current"
        );
        assert_eq!(
            evidence.response.as_ref().unwrap().raw_shape_version,
            "openai.responses.current"
        );
    }

    #[test]
    fn anthropic_partial_arguments_are_marked_partial() {
        let request = RequestMeta {
            model: Some("claude-sonnet-4-20250514".into()),
            stream: false,
            system_prompt_preview: None,
            messages_count: 1,
            tools_count: 1,
            tool_results: Vec::new(),
        };
        let summary = StreamSummary {
            message_id: Some("msg-1".into()),
            model: None,
            text: String::new(),
            thinking: "need tool".into(),
            tool_calls: vec![ToolCall {
                index: 0,
                call_id: "toolu-1".into(),
                name: "fetch_weather".into(),
                arguments: r#"{"city":"Paris""#.into(),
            }],
            input_tokens: None,
            output_tokens: None,
            usage_details: BTreeMap::new(),
            stop_reason: Some(StopReason::ToolUse),
        };

        let evidence = build_model_interaction_evidence(input(
            ProviderKind::Anthropic,
            "/v1/messages",
            &request,
            Some(&summary),
        ));

        assert_eq!(evidence.provider, AiProvider::Anthropic);
        assert_eq!(evidence.parse_status, ParseStatus::Partial);
        assert_eq!(evidence.evidence_status, EvidenceStatus::Partial);
        assert_eq!(
            evidence.tool_calls[0].arguments_status,
            ArgumentsStatus::PartialJson
        );
    }

    #[test]
    fn gemini_path_model_and_tool_result_project_to_evidence() {
        let request = RequestMeta {
            model: None,
            stream: false,
            system_prompt_preview: None,
            messages_count: 3,
            tools_count: 1,
            tool_results: vec![ToolResultMeta {
                call_id: "gemini-call-1".into(),
                content_preview: r#"{"temp":"72F"}"#.into(),
                is_error: false,
            }],
        };
        let summary = StreamSummary {
            message_id: None,
            model: None,
            text: "72F".into(),
            thinking: String::new(),
            tool_calls: Vec::new(),
            input_tokens: Some(50),
            output_tokens: Some(5),
            usage_details: BTreeMap::new(),
            stop_reason: Some(StopReason::EndTurn),
        };

        let evidence = build_model_interaction_evidence(input(
            ProviderKind::Google,
            "/v1beta/models/gemini-2.5-pro:streamGenerateContent",
            &request,
            Some(&summary),
        ));

        assert_eq!(evidence.provider, AiProvider::GoogleGemini);
        assert_eq!(evidence.model, "gemini-2.5-pro");
        assert!(evidence.request.stream);
        assert_eq!(evidence.tool_results[0].content_kind, AiContentKind::Json);
        assert!(evidence.tool_results[0].returned_to_model);
    }

    #[test]
    fn host_attributed_input_preserves_correlation_without_vm_accounting() {
        let request = RequestMeta {
            model: Some("gemini-2.5-flash".into()),
            stream: false,
            system_prompt_preview: Some("name this VM".into()),
            messages_count: 1,
            tools_count: 0,
            tool_results: Vec::new(),
        };
        let mut params = input(
            ProviderKind::Google,
            "/v1beta/models/gemini-2.5-flash:generateContent",
            &request,
            None,
        );
        params.attribution_scope = AiAttributionScope::Host;
        params.source_engine = SourceEngine::HostAi;
        params.origin_kind = AiOriginKind::HostService;
        params.accounting_owner = Some("host:service");

        let evidence = build_model_interaction_evidence(params);

        assert_eq!(evidence.source_engine, SourceEngine::HostAi);
        assert_eq!(evidence.attribution_scope, AiAttributionScope::Host);
        assert_eq!(evidence.vm_id.as_deref(), Some("vm-1"));
        assert!(evidence.charges_host_accounting());
        assert!(!evidence.charges_vm_accounting());
    }

    #[test]
    fn required_response_without_provider_summary_is_partial() {
        let request = RequestMeta {
            model: Some("gpt-5.5".into()),
            stream: true,
            system_prompt_preview: None,
            messages_count: 1,
            tools_count: 0,
            tool_results: Vec::new(),
        };
        let mut params = input(ProviderKind::OpenAi, "/v1/chat/completions", &request, None);
        params.response_required = true;

        let evidence = build_model_interaction_evidence(params);

        assert_eq!(evidence.parse_status, ParseStatus::Partial);
        assert_eq!(evidence.evidence_status, EvidenceStatus::Partial);
    }

    #[test]
    fn required_response_with_unknown_only_summary_is_partial() {
        let request = RequestMeta {
            model: Some("gpt-5.5".into()),
            stream: true,
            system_prompt_preview: None,
            messages_count: 1,
            tools_count: 0,
            tool_results: Vec::new(),
        };
        let summary = StreamSummary {
            message_id: None,
            model: None,
            text: String::new(),
            thinking: String::new(),
            tool_calls: Vec::new(),
            input_tokens: None,
            output_tokens: None,
            usage_details: BTreeMap::new(),
            stop_reason: None,
        };
        let mut params = input(
            ProviderKind::OpenAi,
            "/v1/chat/completions",
            &request,
            Some(&summary),
        );
        params.response_required = true;

        let evidence = build_model_interaction_evidence(params);

        assert_eq!(evidence.parse_status, ParseStatus::Partial);
        assert_eq!(evidence.evidence_status, EvidenceStatus::Partial);
    }

    #[test]
    fn argument_status_distinguishes_absent_not_json_partial_and_malformed() {
        assert_eq!(arguments_status(""), ArgumentsStatus::Absent);
        assert_eq!(arguments_status("plain"), ArgumentsStatus::NotJson);
        assert_eq!(arguments_status(r#"{"a":1"#), ArgumentsStatus::PartialJson);
        assert_eq!(
            arguments_status(r#"{"a":}"#),
            ArgumentsStatus::MalformedJson
        );
        assert_eq!(arguments_status(r#"{"a":1}"#), ArgumentsStatus::ValidJson);
    }

    fn input<'a>(
        provider: ProviderKind,
        path: &'a str,
        request: &'a RequestMeta,
        response: Option<&'a StreamSummary>,
    ) -> ModelEvidenceInput<'a> {
        ModelEvidenceInput {
            interaction_id: "interaction-1",
            trace_id: "trace-1",
            request_id: "request-1",
            response_id: Some("response-1"),
            provider,
            path,
            request,
            response,
            response_required: false,
            estimated_cost_micros: Some(12),
            attribution_scope: AiAttributionScope::Vm,
            source_engine: SourceEngine::Network,
            origin_kind: AiOriginKind::GuestNetwork,
            accounting_owner: Some("vm:vm-1"),
            profile_id: Some("coding"),
            vm_id: Some("vm-1"),
            session_id: Some("session-1"),
            user_id: Some("user-1"),
        }
    }
}
