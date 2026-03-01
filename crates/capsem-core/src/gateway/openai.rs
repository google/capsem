/// OpenAI provider: handles /v1/responses and /v1/chat/completions requests.
///
/// Key injection: Authorization: Bearer header.
/// Upstream: https://api.openai.com
///
/// SSE stream format (Chat Completions): No `event:` lines -- all events are
/// `data:` only. Content and tool calls arrive via `choices[].delta`.
/// Stream ends with `data: [DONE]` (filtered by SseParser).
use std::collections::BTreeMap;

use super::events::{LlmEvent, ProviderStreamParser, StopReason};
use super::provider::{Provider, ProviderKind};
use super::sse::SseEvent;

pub struct OpenAiProvider;

impl Provider for OpenAiProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenAi
    }

    fn upstream_base_url(&self) -> &str {
        "https://api.openai.com"
    }

    fn inject_key(
        &self,
        builder: reqwest::RequestBuilder,
        api_key: &str,
    ) -> reqwest::RequestBuilder {
        builder.header("authorization", format!("Bearer {api_key}"))
    }
}

// ── Wire format serde types (Chat Completions only for now) ─────────

#[allow(dead_code)] // Wire types: fields exist for serde deserialization
mod wire {
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub struct ChatCompletionChunk {
        pub id: Option<String>,
        pub model: Option<String>,
        pub choices: Option<Vec<Choice>>,
        pub usage: Option<Usage>,
    }

    #[derive(Deserialize)]
    pub struct Choice {
        pub index: Option<u32>,
        pub delta: Option<ChoiceDelta>,
        pub finish_reason: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct ChoiceDelta {
        pub content: Option<String>,
        pub tool_calls: Option<Vec<ToolCallDelta>>,
    }

    #[derive(Deserialize)]
    pub struct ToolCallDelta {
        pub index: Option<u32>,
        pub id: Option<String>,
        pub function: Option<FunctionDelta>,
    }

    #[derive(Deserialize)]
    pub struct FunctionDelta {
        pub name: Option<String>,
        pub arguments: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct Usage {
        pub prompt_tokens: Option<u64>,
        pub completion_tokens: Option<u64>,
        pub prompt_tokens_details: Option<PromptTokensDetails>,
        pub completion_tokens_details: Option<CompletionTokensDetails>,
    }

    #[derive(Deserialize)]
    pub struct PromptTokensDetails {
        pub cached_tokens: Option<u64>,
    }

    #[derive(Deserialize)]
    pub struct CompletionTokensDetails {
        pub reasoning_tokens: Option<u64>,
    }

    // ── Responses API wire types ──────────────────────────────────

    #[derive(Deserialize)]
    pub struct ResponseCreated {
        pub response: Option<ResponseInfo>,
    }

    #[derive(Deserialize)]
    pub struct ResponseInfo {
        pub id: Option<String>,
        pub model: Option<String>,
        pub usage: Option<ResponseUsage>,
        pub output: Option<Vec<serde_json::Value>>,
    }

    #[derive(Deserialize)]
    pub struct ResponseUsage {
        pub input_tokens: Option<u64>,
        pub output_tokens: Option<u64>,
        pub input_tokens_details: Option<PromptTokensDetails>,
        pub output_tokens_details: Option<CompletionTokensDetails>,
    }

    #[derive(Deserialize)]
    pub struct OutputItemAdded {
        pub output_index: Option<u32>,
        pub item: Option<OutputItem>,
    }

    #[derive(Deserialize)]
    pub struct OutputItem {
        pub id: Option<String>,
        #[serde(rename = "type")]
        pub item_type: Option<String>,
        pub call_id: Option<String>,
        pub name: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct OutputTextDelta {
        pub output_index: Option<u32>,
        pub content_index: Option<u32>,
        pub delta: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct ReasoningSummaryTextDelta {
        pub output_index: Option<u32>,
        pub summary_index: Option<u32>,
        pub delta: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct FunctionCallArgumentsDelta {
        pub output_index: Option<u32>,
        pub item_id: Option<String>,
        pub delta: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct OutputItemDone {
        pub output_index: Option<u32>,
        pub item: Option<OutputItem>,
    }

    #[derive(Deserialize)]
    pub struct ResponseCompleted {
        pub response: Option<ResponseInfo>,
    }
}

/// OpenAI Chat Completions SSE stream parser.
///
/// Note: this parser never emits `ToolCallEnd` -- OpenAI's wire format has no
/// explicit end-of-tool-call signal. Tool call builders are flushed by
/// `collect_summary()` after the stream completes.
pub struct OpenAiStreamParser {
    /// Whether we've emitted MessageStart yet.
    started: bool,
}

impl Default for OpenAiStreamParser {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenAiStreamParser {
    pub fn new() -> Self {
        Self { started: false }
    }

    fn parse_stop_reason(s: &str) -> StopReason {
        match s {
            "stop" => StopReason::EndTurn,
            "tool_calls" => StopReason::ToolUse,
            "length" => StopReason::MaxTokens,
            "content_filter" => StopReason::ContentFilter,
            other => StopReason::Other(other.into()),
        }
    }

    /// Parse an OpenAI Responses API SSE event.
    fn parse_responses_event(&mut self, event_type: &str, data: &str) -> Vec<LlmEvent> {
        match event_type {
            "response.created" => {
                let Ok(rc) = serde_json::from_str::<wire::ResponseCreated>(data) else {
                    return vec![];
                };
                self.started = true;
                let resp = rc.response.as_ref();
                vec![LlmEvent::MessageStart {
                    message_id: resp.and_then(|r| r.id.clone()),
                    model: resp.and_then(|r| r.model.clone()),
                }]
            }
            "response.output_item.added" => {
                let Ok(item) = serde_json::from_str::<wire::OutputItemAdded>(data) else {
                    return vec![];
                };
                let index = item.output_index.unwrap_or(0);
                if let Some(oi) = &item.item {
                    if oi.item_type.as_deref() == Some("function_call") {
                        return vec![LlmEvent::ToolCallStart {
                            index,
                            call_id: oi.call_id.clone().unwrap_or_default(),
                            name: oi.name.clone().unwrap_or_default(),
                        }];
                    }
                }
                vec![]
            }
            "response.output_text.delta" => {
                let Ok(td) = serde_json::from_str::<wire::OutputTextDelta>(data) else {
                    return vec![];
                };
                if let Some(text) = td.delta {
                    if !text.is_empty() {
                        return vec![LlmEvent::TextDelta {
                            index: td.output_index.unwrap_or(0),
                            text,
                        }];
                    }
                }
                vec![]
            }
            "response.reasoning_summary_text.delta" => {
                let Ok(td) = serde_json::from_str::<wire::ReasoningSummaryTextDelta>(data) else {
                    return vec![];
                };
                if let Some(text) = td.delta {
                    if !text.is_empty() {
                        return vec![LlmEvent::ThinkingDelta {
                            index: td.output_index.unwrap_or(0),
                            text,
                        }];
                    }
                }
                vec![]
            }
            "response.function_call_arguments.delta" => {
                let Ok(fd) = serde_json::from_str::<wire::FunctionCallArgumentsDelta>(data) else {
                    return vec![];
                };
                if let Some(delta) = fd.delta {
                    if !delta.is_empty() {
                        return vec![LlmEvent::ToolCallArgumentDelta {
                            index: fd.output_index.unwrap_or(0),
                            delta,
                        }];
                    }
                }
                vec![]
            }
            "response.output_item.done" => {
                let Ok(done) = serde_json::from_str::<wire::OutputItemDone>(data) else {
                    return vec![];
                };
                // Only emit ToolCallEnd for function_call items, not text or other types
                if done.item.as_ref().and_then(|i| i.item_type.as_deref()) == Some("function_call") {
                    vec![LlmEvent::ToolCallEnd {
                        index: done.output_index.unwrap_or(0),
                    }]
                } else {
                    vec![]
                }
            }
            "response.completed" => {
                let Ok(rc) = serde_json::from_str::<wire::ResponseCompleted>(data) else {
                    return vec![];
                };
                let mut events = Vec::new();
                if let Some(resp) = &rc.response {
                    if let Some(usage) = &resp.usage {
                        let mut details = BTreeMap::new();
                        if let Some(ptd) = &usage.input_tokens_details {
                            if let Some(ct) = ptd.cached_tokens {
                                details.insert("cache_read".into(), ct);
                            }
                        }
                        if let Some(ctd) = &usage.output_tokens_details {
                            if let Some(rt) = ctd.reasoning_tokens {
                                details.insert("thinking".into(), rt);
                            }
                        }
                        events.push(LlmEvent::Usage {
                            input_tokens: usage.input_tokens,
                            output_tokens: usage.output_tokens,
                            details,
                        });
                    }
                }
                events.push(LlmEvent::MessageEnd {
                    stop_reason: Some(StopReason::EndTurn),
                });
                events
            }
            // Ignore other response.* events (response.in_progress, etc.)
            _ => vec![],
        }
    }
}

impl ProviderStreamParser for OpenAiStreamParser {
    fn parse_event(&mut self, sse: &SseEvent) -> Vec<LlmEvent> {
        // Dispatch: Responses API uses typed event: lines, Chat Completions does not.
        if let Some(et) = &sse.event_type {
            if et.starts_with("response.") {
                return self.parse_responses_event(et, &sse.data);
            }
        }

        // Chat Completions path: no event: lines, just data:
        let Ok(chunk) = serde_json::from_str::<wire::ChatCompletionChunk>(&sse.data) else {
            return vec![LlmEvent::Unknown {
                event_type: sse.event_type.clone(),
                raw: sse.data.clone(),
            }];
        };

        let mut events = Vec::new();

        // Emit MessageStart on first chunk
        if !self.started {
            self.started = true;
            events.push(LlmEvent::MessageStart {
                message_id: chunk.id.clone(),
                model: chunk.model.clone(),
            });
        }

        // Process choices
        if let Some(choices) = &chunk.choices {
            for choice in choices {
                let finish_reason = choice.finish_reason.as_deref();

                if let Some(delta) = &choice.delta {
                    // Text content
                    if let Some(content) = &delta.content {
                        if !content.is_empty() {
                            events.push(LlmEvent::TextDelta {
                                index: choice.index.unwrap_or(0),
                                text: content.clone(),
                            });
                        }
                    }

                    // Tool calls
                    if let Some(tool_calls) = &delta.tool_calls {
                        for tc in tool_calls {
                            let tc_index = tc.index.unwrap_or(0);
                            // If id is present, this is the start of a tool call
                            if let Some(id) = &tc.id {
                                let name = tc.function.as_ref()
                                    .and_then(|f| f.name.clone())
                                    .unwrap_or_default();
                                events.push(LlmEvent::ToolCallStart {
                                    index: tc_index,
                                    call_id: id.clone(),
                                    name,
                                });
                            }
                            // Argument deltas
                            if let Some(func) = &tc.function {
                                if let Some(args) = &func.arguments {
                                    if !args.is_empty() {
                                        events.push(LlmEvent::ToolCallArgumentDelta {
                                            index: tc_index,
                                            delta: args.clone(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }

                // Finish reason
                if let Some(reason) = finish_reason {
                    events.push(LlmEvent::MessageEnd {
                        stop_reason: Some(Self::parse_stop_reason(reason)),
                    });
                }
            }
        }

        // Usage (often in the last chunk)
        if let Some(usage) = &chunk.usage {
            let mut details = BTreeMap::new();
            if let Some(ptd) = &usage.prompt_tokens_details {
                if let Some(ct) = ptd.cached_tokens {
                    details.insert("cache_read".into(), ct);
                }
            }
            if let Some(ctd) = &usage.completion_tokens_details {
                if let Some(rt) = ctd.reasoning_tokens {
                    details.insert("thinking".into(), rt);
                }
            }
            events.push(LlmEvent::Usage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
                details,
            });
        }

        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::events::collect_summary;
    use crate::gateway::sse::SseParser;

    #[test]
    fn upstream_url_responses() {
        let p = OpenAiProvider;
        assert_eq!(
            p.upstream_url("/v1/responses", None),
            "https://api.openai.com/v1/responses"
        );
    }

    #[test]
    fn upstream_url_chat_completions() {
        let p = OpenAiProvider;
        assert_eq!(
            p.upstream_url("/v1/chat/completions", None),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn kind_is_openai() {
        assert_eq!(OpenAiProvider.kind(), ProviderKind::OpenAi);
    }

    // ── Stream parser: text-only response ───────────────────────────

    #[test]
    fn stream_text_response() {
        let raw = b"\
data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":null}]}\n\
\n\
data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\
\n\
data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" there!\"},\"finish_reason\":null}]}\n\
\n\
data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":3}}\n\
\n\
data: [DONE]\n\
\n";

        let mut sse_parser = SseParser::new();
        let sse_events = sse_parser.feed(raw);

        let mut parser = OpenAiStreamParser::new();
        let mut llm_events = Vec::new();
        for sse in &sse_events {
            llm_events.extend(parser.parse_event(sse));
        }

        let summary = collect_summary(&llm_events);
        assert_eq!(summary.message_id.as_deref(), Some("chatcmpl-1"));
        assert_eq!(summary.model.as_deref(), Some("gpt-4o"));
        assert_eq!(summary.text, "Hello there!");
        assert_eq!(summary.stop_reason, Some(StopReason::EndTurn));
        assert_eq!(summary.input_tokens, Some(10));
        assert_eq!(summary.output_tokens, Some(3));
    }

    // ── Stream parser: tool calls ───────────────────────────────────

    #[test]
    fn stream_tool_call_response() {
        let raw = b"\
data: {\"id\":\"chatcmpl-2\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"tool_calls\":[{\"index\":0,\"id\":\"call_abc\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\
\n\
data: {\"id\":\"chatcmpl-2\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"city\\\"\"}}]},\"finish_reason\":null}]}\n\
\n\
data: {\"id\":\"chatcmpl-2\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\": \\\"NYC\\\"}\"}}]},\"finish_reason\":null}]}\n\
\n\
data: {\"id\":\"chatcmpl-2\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\
\n\
data: [DONE]\n\
\n";

        let mut sse_parser = SseParser::new();
        let sse_events = sse_parser.feed(raw);

        let mut parser = OpenAiStreamParser::new();
        let mut llm_events = Vec::new();
        for sse in &sse_events {
            llm_events.extend(parser.parse_event(sse));
        }

        let summary = collect_summary(&llm_events);
        assert_eq!(summary.tool_calls.len(), 1);
        assert_eq!(summary.tool_calls[0].call_id, "call_abc");
        assert_eq!(summary.tool_calls[0].name, "get_weather");
        assert_eq!(summary.tool_calls[0].arguments, "{\"city\": \"NYC\"}");
        assert_eq!(summary.stop_reason, Some(StopReason::ToolUse));
    }

    // ── Adversarial: malformed JSON ─────────────────────────────────

    #[test]
    fn malformed_json_becomes_unknown() {
        let mut parser = OpenAiStreamParser::new();
        let sse = SseEvent { event_type: None, data: "not json".into() };
        let events = parser.parse_event(&sse);
        assert_eq!(events.len(), 1);
        matches!(&events[0], LlmEvent::Unknown { .. });
    }

    // ── Adversarial: empty choices array ────────────────────────────

    #[test]
    fn empty_choices_just_starts() {
        let mut parser = OpenAiStreamParser::new();
        let sse = SseEvent {
            event_type: None,
            data: "{\"id\":\"x\",\"choices\":[]}".into(),
        };
        let events = parser.parse_event(&sse);
        // Should emit MessageStart only
        assert_eq!(events.len(), 1);
        matches!(&events[0], LlmEvent::MessageStart { .. });
    }

    // ── Adversarial: content_filter finish reason ───────────────────

    #[test]
    fn content_filter_stop_reason() {
        let mut parser = OpenAiStreamParser::new();
        let sse = SseEvent {
            event_type: None,
            data: "{\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"content_filter\"}]}".into(),
        };
        let events = parser.parse_event(&sse);
        let has_end = events.iter().any(|e| matches!(e,
            LlmEvent::MessageEnd { stop_reason: Some(StopReason::ContentFilter) }
        ));
        assert!(has_end);
    }

    // ── Responses API: text-only response ─────────────────────────

    #[test]
    fn responses_api_text_response() {
        let raw = b"\
event: response.created\n\
data: {\"response\":{\"id\":\"resp-1\",\"model\":\"gpt-4o\"}}\n\
\n\
event: response.output_text.delta\n\
data: {\"output_index\":0,\"content_index\":0,\"delta\":\"Hello\"}\n\
\n\
event: response.output_text.delta\n\
data: {\"output_index\":0,\"content_index\":0,\"delta\":\" world!\"}\n\
\n\
event: response.completed\n\
data: {\"response\":{\"id\":\"resp-1\",\"model\":\"gpt-4o\",\"usage\":{\"input_tokens\":15,\"output_tokens\":5}}}\n\
\n";

        let mut sse_parser = SseParser::new();
        let sse_events = sse_parser.feed(raw);

        let mut parser = OpenAiStreamParser::new();
        let mut llm_events = Vec::new();
        for sse in &sse_events {
            llm_events.extend(parser.parse_event(sse));
        }

        let summary = collect_summary(&llm_events);
        assert_eq!(summary.message_id.as_deref(), Some("resp-1"));
        assert_eq!(summary.model.as_deref(), Some("gpt-4o"));
        assert_eq!(summary.text, "Hello world!");
        assert_eq!(summary.stop_reason, Some(StopReason::EndTurn));
        assert_eq!(summary.input_tokens, Some(15));
        assert_eq!(summary.output_tokens, Some(5));
    }

    // ── Responses API: tool calls ─────────────────────────────────

    #[test]
    fn responses_api_tool_call() {
        let raw = b"\
event: response.created\n\
data: {\"response\":{\"id\":\"resp-2\",\"model\":\"gpt-4o\"}}\n\
\n\
event: response.output_item.added\n\
data: {\"output_index\":0,\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"call_id\":\"call_xyz\",\"name\":\"get_weather\"}}\n\
\n\
event: response.function_call_arguments.delta\n\
data: {\"output_index\":0,\"item_id\":\"fc_1\",\"delta\":\"{\\\"city\\\"\"}\n\
\n\
event: response.function_call_arguments.delta\n\
data: {\"output_index\":0,\"item_id\":\"fc_1\",\"delta\":\": \\\"NYC\\\"}\"}\n\
\n\
event: response.output_item.done\n\
data: {\"output_index\":0,\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\"}}\n\
\n\
event: response.completed\n\
data: {\"response\":{\"id\":\"resp-2\",\"model\":\"gpt-4o\",\"usage\":{\"input_tokens\":20,\"output_tokens\":10}}}\n\
\n";

        let mut sse_parser = SseParser::new();
        let sse_events = sse_parser.feed(raw);

        let mut parser = OpenAiStreamParser::new();
        let mut llm_events = Vec::new();
        for sse in &sse_events {
            llm_events.extend(parser.parse_event(sse));
        }

        let summary = collect_summary(&llm_events);
        assert_eq!(summary.tool_calls.len(), 1);
        assert_eq!(summary.tool_calls[0].call_id, "call_xyz");
        assert_eq!(summary.tool_calls[0].name, "get_weather");
        assert_eq!(summary.tool_calls[0].arguments, "{\"city\": \"NYC\"}");
        assert_eq!(summary.stop_reason, Some(StopReason::EndTurn));
        assert_eq!(summary.input_tokens, Some(20));
    }

    // ── Responses API: reasoning summary ──────────────────────────

    #[test]
    fn responses_api_reasoning_summary() {
        let raw = b"\
event: response.created\n\
data: {\"response\":{\"id\":\"resp-3\",\"model\":\"o3\"}}\n\
\n\
event: response.reasoning_summary_text.delta\n\
data: {\"output_index\":0,\"summary_index\":0,\"delta\":\"Let me think\"}\n\
\n\
event: response.reasoning_summary_text.delta\n\
data: {\"output_index\":0,\"summary_index\":0,\"delta\":\" about this.\"}\n\
\n\
event: response.output_text.delta\n\
data: {\"output_index\":1,\"content_index\":0,\"delta\":\"The answer is 42.\"}\n\
\n\
event: response.completed\n\
data: {\"response\":{\"id\":\"resp-3\",\"model\":\"o3\",\"usage\":{\"input_tokens\":10,\"output_tokens\":20,\"output_tokens_details\":{\"reasoning_tokens\":50}}}}\n\
\n";

        let mut sse_parser = SseParser::new();
        let sse_events = sse_parser.feed(raw);

        let mut parser = OpenAiStreamParser::new();
        let mut llm_events = Vec::new();
        for sse in &sse_events {
            llm_events.extend(parser.parse_event(sse));
        }

        let summary = collect_summary(&llm_events);
        assert_eq!(summary.text, "The answer is 42.");
        assert_eq!(summary.thinking, "Let me think about this.");
        assert_eq!(summary.usage_details.get("thinking"), Some(&50));
    }

    // ── Responses API: unknown event types are silently ignored ───

    #[test]
    fn responses_api_unknown_event_ignored() {
        let mut parser = OpenAiStreamParser::new();
        let sse = SseEvent {
            event_type: Some("response.in_progress".into()),
            data: "{}".into(),
        };
        let events = parser.parse_event(&sse);
        assert!(events.is_empty());
    }

    // ── Responses API: malformed JSON returns empty ───────────────

    #[test]
    fn responses_api_malformed_json() {
        let mut parser = OpenAiStreamParser::new();
        let sse = SseEvent {
            event_type: Some("response.created".into()),
            data: "not json".into(),
        };
        let events = parser.parse_event(&sse);
        assert!(events.is_empty());
    }

    // ── Responses API: cached + reasoning token details ──────────

    #[test]
    fn responses_api_usage_details() {
        let raw = b"\
event: response.created\n\
data: {\"response\":{\"id\":\"resp-4\",\"model\":\"gpt-4o\"}}\n\
\n\
event: response.completed\n\
data: {\"response\":{\"id\":\"resp-4\",\"model\":\"gpt-4o\",\"usage\":{\"input_tokens\":1000,\"output_tokens\":500,\"input_tokens_details\":{\"cached_tokens\":800},\"output_tokens_details\":{\"reasoning_tokens\":200}}}}\n\
\n";

        let mut sse_parser = SseParser::new();
        let sse_events = sse_parser.feed(raw);

        let mut parser = OpenAiStreamParser::new();
        let mut llm_events = Vec::new();
        for sse in &sse_events {
            llm_events.extend(parser.parse_event(sse));
        }

        let summary = collect_summary(&llm_events);
        assert_eq!(summary.input_tokens, Some(1000));
        assert_eq!(summary.output_tokens, Some(500));
        assert_eq!(summary.usage_details.get("cache_read"), Some(&800));
        assert_eq!(summary.usage_details.get("thinking"), Some(&200));
    }

    // ── Responses API: output_item.done only emits ToolCallEnd for function_call ──

    #[test]
    fn responses_api_output_item_done_text_ignored() {
        let mut parser = OpenAiStreamParser::new();
        // response.created to start
        let sse1 = SseEvent {
            event_type: Some("response.created".into()),
            data: r#"{"response":{"id":"resp-t","model":"gpt-4o"}}"#.into(),
        };
        parser.parse_event(&sse1);

        // output_item.done for a text message item (not function_call)
        let sse = SseEvent {
            event_type: Some("response.output_item.done".into()),
            data: r#"{"output_index":0,"item":{"id":"msg_1","type":"message"}}"#.into(),
        };
        let events = parser.parse_event(&sse);
        // Should NOT emit ToolCallEnd for a message item
        assert!(events.is_empty(), "text output_item.done should not emit ToolCallEnd");
    }

    #[test]
    fn responses_api_output_item_done_function_call_emits_end() {
        let mut parser = OpenAiStreamParser::new();
        let sse = SseEvent {
            event_type: Some("response.output_item.done".into()),
            data: r#"{"output_index":1,"item":{"id":"fc_1","type":"function_call"}}"#.into(),
        };
        let events = parser.parse_event(&sse);
        assert_eq!(events.len(), 1);
        match &events[0] {
            LlmEvent::ToolCallEnd { index } => assert_eq!(*index, 1),
            other => panic!("expected ToolCallEnd, got {:?}", other),
        }
    }
}
