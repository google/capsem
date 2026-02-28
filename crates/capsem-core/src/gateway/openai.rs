/// OpenAI provider: handles /v1/responses and /v1/chat/completions requests.
///
/// Key injection: Authorization: Bearer header.
/// Upstream: https://api.openai.com
///
/// SSE stream format (Chat Completions): No `event:` lines -- all events are
/// `data:` only. Content and tool calls arrive via `choices[].delta`.
/// Stream ends with `data: [DONE]` (filtered by SseParser).
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
}

impl ProviderStreamParser for OpenAiStreamParser {
    fn parse_event(&mut self, sse: &SseEvent) -> Vec<LlmEvent> {
        // OpenAI Chat Completions uses no event: lines, just data:
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
            events.push(LlmEvent::Usage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
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
}
