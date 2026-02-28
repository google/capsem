/// Google Gemini provider: handles /v1beta/models/* requests.
///
/// Key injection: ?key= query parameter.
/// Upstream: https://generativelanguage.googleapis.com
///
/// SSE stream format: Each SSE event is a complete JSON object (not deltas).
/// Parts contain `text`, `functionCall`, or `thought` fields.
/// Gemini doesn't provide tool call IDs -- we generate synthetic ones.

use super::events::{LlmEvent, ProviderStreamParser, StopReason};
use super::provider::{Provider, ProviderKind};
use super::sse::SseEvent;

pub struct GoogleProvider;

impl Provider for GoogleProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Google
    }

    fn upstream_base_url(&self) -> &str {
        "https://generativelanguage.googleapis.com"
    }

    /// Google uses query param for API key, so we override upstream_url
    /// to NOT include the key here -- inject_key handles it.
    fn upstream_url(&self, path: &str, query: Option<&str>) -> String {
        let base = self.upstream_base_url();
        match query {
            Some(q) => format!("{base}{path}?{q}"),
            None => format!("{base}{path}"),
        }
    }

    fn inject_key(
        &self,
        builder: reqwest::RequestBuilder,
        api_key: &str,
    ) -> reqwest::RequestBuilder {
        builder.query(&[("key", api_key)])
    }
}

// ── Wire format serde types ─────────────────────────────────────────

mod wire {
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct StreamChunk {
        pub candidates: Option<Vec<Candidate>>,
        pub usage_metadata: Option<UsageMetadata>,
        pub model_version: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Candidate {
        pub content: Option<Content>,
        pub finish_reason: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct Content {
        pub parts: Option<Vec<Part>>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Part {
        pub text: Option<String>,
        pub function_call: Option<FunctionCall>,
        pub thought: Option<bool>,
    }

    #[derive(Deserialize)]
    pub struct FunctionCall {
        pub name: Option<String>,
        pub args: Option<serde_json::Value>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct UsageMetadata {
        pub prompt_token_count: Option<u64>,
        pub candidates_token_count: Option<u64>,
    }
}

/// Google Gemini SSE stream parser.
pub struct GoogleStreamParser {
    started: bool,
    block_index: u32,
}

impl Default for GoogleStreamParser {
    fn default() -> Self {
        Self::new()
    }
}

impl GoogleStreamParser {
    pub fn new() -> Self {
        Self { started: false, block_index: 0 }
    }

    fn parse_stop_reason(s: &str) -> StopReason {
        match s {
            "STOP" => StopReason::EndTurn,
            "MAX_TOKENS" => StopReason::MaxTokens,
            "SAFETY" => StopReason::ContentFilter,
            other => StopReason::Other(other.into()),
        }
    }
}

impl ProviderStreamParser for GoogleStreamParser {
    fn parse_event(&mut self, sse: &SseEvent) -> Vec<LlmEvent> {
        let Ok(chunk) = serde_json::from_str::<wire::StreamChunk>(&sse.data) else {
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
                message_id: None, // Gemini doesn't provide message IDs in SSE
                model: chunk.model_version.clone(),
            });
        }

        // Process candidates
        if let Some(candidates) = &chunk.candidates {
            for candidate in candidates {
                if let Some(content) = &candidate.content {
                    if let Some(parts) = &content.parts {
                        for part in parts {
                            // Thinking text
                            if part.thought == Some(true) {
                                if let Some(text) = &part.text {
                                    if !text.is_empty() {
                                        events.push(LlmEvent::ThinkingDelta {
                                            index: self.block_index,
                                            text: text.clone(),
                                        });
                                    }
                                }
                                continue;
                            }

                            // Regular text
                            if let Some(text) = &part.text {
                                if !text.is_empty() {
                                    events.push(LlmEvent::TextDelta {
                                        index: self.block_index,
                                        text: text.clone(),
                                    });
                                }
                            }

                            // Function call (complete, not streamed)
                            if let Some(fc) = &part.function_call {
                                let name = fc.name.clone().unwrap_or_default();
                                // Gemini doesn't return tool call IDs, so we use the name as the call_id
                                // to link the tool_response later (which also only has the name).
                                let call_id = name.clone();
                                let arguments = fc.args
                                    .as_ref()
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "{}".into());

                                let idx = self.block_index;
                                self.block_index += 1;
                                events.push(LlmEvent::ToolCallStart {
                                    index: idx,
                                    call_id: call_id.clone(),
                                    name,
                                });
                                events.push(LlmEvent::ToolCallArgumentDelta {
                                    index: idx,
                                    delta: arguments,
                                });
                                events.push(LlmEvent::ToolCallEnd { index: idx });
                            }
                        }
                    }
                }

                // Finish reason
                if let Some(reason) = &candidate.finish_reason {
                    events.push(LlmEvent::MessageEnd {
                        stop_reason: Some(Self::parse_stop_reason(reason)),
                    });
                }
            }
        }

        // Usage metadata
        if let Some(usage) = &chunk.usage_metadata {
            events.push(LlmEvent::Usage {
                input_tokens: usage.prompt_token_count,
                output_tokens: usage.candidates_token_count,
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
    fn upstream_url_stream_generate() {
        let p = GoogleProvider;
        assert_eq!(
            p.upstream_url(
                "/v1beta/models/gemini-2.5-pro:streamGenerateContent",
                None
            ),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:streamGenerateContent"
        );
    }

    #[test]
    fn upstream_url_generate_content() {
        let p = GoogleProvider;
        assert_eq!(
            p.upstream_url("/v1beta/models/gemini-2.5-flash:generateContent", None),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent"
        );
    }

    #[test]
    fn upstream_url_with_existing_query() {
        let p = GoogleProvider;
        assert_eq!(
            p.upstream_url(
                "/v1beta/models/gemini-2.5-pro:streamGenerateContent",
                Some("alt=sse")
            ),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn kind_is_google() {
        assert_eq!(GoogleProvider.kind(), ProviderKind::Google);
    }

    // ── Stream parser: text response ────────────────────────────────

    #[test]
    fn stream_text_response() {
        let raw = b"\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}],\"role\":\"model\"}}],\"usageMetadata\":{\"promptTokenCount\":5,\"candidatesTokenCount\":1},\"modelVersion\":\"gemini-2.5-flash\"}\n\
\n\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\" world!\"}],\"role\":\"model\"}}],\"usageMetadata\":{\"promptTokenCount\":5,\"candidatesTokenCount\":3}}\n\
\n\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":5,\"candidatesTokenCount\":3,\"totalTokenCount\":8}}\n\
\n";

        let mut sse_parser = SseParser::new();
        let sse_events = sse_parser.feed(raw);

        let mut parser = GoogleStreamParser::new();
        let mut llm_events = Vec::new();
        for sse in &sse_events {
            llm_events.extend(parser.parse_event(sse));
        }

        let summary = collect_summary(&llm_events);
        assert_eq!(summary.model.as_deref(), Some("gemini-2.5-flash"));
        assert_eq!(summary.text, "Hello world!");
        assert_eq!(summary.stop_reason, Some(StopReason::EndTurn));
        assert_eq!(summary.input_tokens, Some(5));
        assert_eq!(summary.output_tokens, Some(3));
    }

    // ── Stream parser: function call ────────────────────────────────

    #[test]
    fn stream_function_call_response() {
        let raw = b"\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"get_weather\",\"args\":{\"city\":\"NYC\"}}}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":5}}\n\
\n";

        let mut sse_parser = SseParser::new();
        let sse_events = sse_parser.feed(raw);

        let mut parser = GoogleStreamParser::new();
        let mut llm_events = Vec::new();
        for sse in &sse_events {
            llm_events.extend(parser.parse_event(sse));
        }

        let summary = collect_summary(&llm_events);
        assert_eq!(summary.tool_calls.len(), 1);
        assert_eq!(summary.tool_calls[0].name, "get_weather");
        assert_eq!(summary.tool_calls[0].call_id, "get_weather");
        let args: serde_json::Value = serde_json::from_str(&summary.tool_calls[0].arguments).unwrap();
        assert_eq!(args["city"], "NYC");
    }

    // ── Stream parser: thinking ─────────────────────────────────────

    #[test]
    fn stream_thinking_response() {
        let raw = b"\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Let me reason.\",\"thought\":true}],\"role\":\"model\"}}]}\n\
\n\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"The answer.\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}]}\n\
\n";

        let mut sse_parser = SseParser::new();
        let sse_events = sse_parser.feed(raw);

        let mut parser = GoogleStreamParser::new();
        let mut llm_events = Vec::new();
        for sse in &sse_events {
            llm_events.extend(parser.parse_event(sse));
        }

        let summary = collect_summary(&llm_events);
        assert_eq!(summary.thinking, "Let me reason.");
        assert_eq!(summary.text, "The answer.");
    }

    // ── Adversarial: malformed JSON ─────────────────────────────────

    #[test]
    fn malformed_json_becomes_unknown() {
        let mut parser = GoogleStreamParser::new();
        let sse = SseEvent { event_type: None, data: "garbage".into() };
        let events = parser.parse_event(&sse);
        assert_eq!(events.len(), 1);
        matches!(&events[0], LlmEvent::Unknown { .. });
    }

    // ── Adversarial: empty candidates ───────────────────────────────

    #[test]
    fn empty_candidates() {
        let mut parser = GoogleStreamParser::new();
        let sse = SseEvent {
            event_type: None,
            data: "{\"candidates\":[]}".into(),
        };
        let events = parser.parse_event(&sse);
        // Should emit MessageStart only
        assert_eq!(events.len(), 1);
        matches!(&events[0], LlmEvent::MessageStart { .. });
    }
}
