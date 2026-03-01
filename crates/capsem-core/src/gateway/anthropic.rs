/// Anthropic provider: handles /v1/messages requests.
///
/// Key injection: x-api-key header.
/// Upstream: https://api.anthropic.com
///
/// SSE stream format: Anthropic uses `event:` lines to distinguish event types.
/// Content blocks are interleaved (text, tool_use, thinking) with index-based
/// tracking.
use std::collections::{BTreeMap, HashMap};

use super::events::{LlmEvent, ProviderStreamParser, StopReason};
use super::provider::{Provider, ProviderKind};
use super::sse::SseEvent;

pub struct AnthropicProvider;

impl Provider for AnthropicProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Anthropic
    }

    fn upstream_base_url(&self) -> &str {
        "https://api.anthropic.com"
    }

    fn inject_key(
        &self,
        builder: reqwest::RequestBuilder,
        api_key: &str,
    ) -> reqwest::RequestBuilder {
        builder.header("x-api-key", api_key)
    }
}

// ── Wire format serde types (targeted, skip irrelevant fields) ──────

#[allow(dead_code)] // Wire types: fields exist for serde deserialization
mod wire {
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub struct MessageStartPayload {
        pub message: Option<MessageInfo>,
    }
    #[derive(Deserialize)]
    pub struct MessageInfo {
        pub id: Option<String>,
        pub model: Option<String>,
        pub usage: Option<Usage>,
    }

    #[derive(Deserialize)]
    pub struct ContentBlockStart {
        pub index: Option<u32>,
        pub content_block: Option<ContentBlock>,
    }
    #[derive(Deserialize)]
    #[serde(tag = "type")]
    pub enum ContentBlock {
        #[serde(rename = "text")]
        Text { text: Option<String> },
        #[serde(rename = "tool_use")]
        ToolUse { id: Option<String>, name: Option<String> },
        #[serde(rename = "thinking")]
        Thinking { thinking: Option<String> },
    }

    #[derive(Deserialize)]
    pub struct ContentBlockDelta {
        pub index: Option<u32>,
        pub delta: Option<Delta>,
    }
    #[derive(Deserialize)]
    #[serde(tag = "type")]
    pub enum Delta {
        #[serde(rename = "text_delta")]
        TextDelta { text: Option<String> },
        #[serde(rename = "input_json_delta")]
        InputJsonDelta { partial_json: Option<String> },
        #[serde(rename = "thinking_delta")]
        ThinkingDelta { thinking: Option<String> },
    }

    #[derive(Deserialize)]
    pub struct ContentBlockStop {
        pub index: Option<u32>,
    }

    #[derive(Deserialize)]
    pub struct MessageDelta {
        pub delta: Option<MessageDeltaInner>,
        pub usage: Option<Usage>,
    }
    #[derive(Deserialize)]
    pub struct MessageDeltaInner {
        pub stop_reason: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct Usage {
        pub input_tokens: Option<u64>,
        pub output_tokens: Option<u64>,
        pub cache_read_input_tokens: Option<u64>,
    }
}

/// Tracks content block types by index (Anthropic interleaves blocks).
#[derive(Debug, Clone, Copy, PartialEq)]
enum BlockKind {
    Text,
    ToolUse,
    Thinking,
}

/// Anthropic SSE stream parser.
pub struct AnthropicStreamParser {
    /// Maps content block index to its kind (for correct delta routing).
    blocks: HashMap<u32, BlockKind>,
}

impl Default for AnthropicStreamParser {
    fn default() -> Self {
        Self::new()
    }
}

impl AnthropicStreamParser {
    pub fn new() -> Self {
        Self { blocks: HashMap::new() }
    }
}

impl ProviderStreamParser for AnthropicStreamParser {
    fn parse_event(&mut self, sse: &SseEvent) -> Vec<LlmEvent> {
        let event_type = match &sse.event_type {
            Some(t) => t.as_str(),
            None => return vec![LlmEvent::Unknown { event_type: None, raw: sse.data.clone() }],
        };

        match event_type {
            "message_start" => {
                let Ok(payload) = serde_json::from_str::<wire::MessageStartPayload>(&sse.data)
                else {
                    return vec![LlmEvent::Unknown { event_type: Some(event_type.into()), raw: sse.data.clone() }];
                };
                let mut events = Vec::with_capacity(2);
                let msg = payload.message.as_ref();
                events.push(LlmEvent::MessageStart {
                    message_id: msg.and_then(|m| m.id.clone()),
                    model: msg.and_then(|m| m.model.clone()),
                });
                if let Some(usage) = msg.and_then(|m| m.usage.as_ref()) {
                    let mut details = BTreeMap::new();
                    if let Some(crt) = usage.cache_read_input_tokens {
                        details.insert("cache_read".into(), crt);
                    }
                    events.push(LlmEvent::Usage {
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                        details,
                    });
                }
                events
            }

            "content_block_start" => {
                let Ok(payload) = serde_json::from_str::<wire::ContentBlockStart>(&sse.data)
                else {
                    return vec![LlmEvent::Unknown { event_type: Some(event_type.into()), raw: sse.data.clone() }];
                };
                let index = payload.index.unwrap_or(0);
                match payload.content_block {
                    Some(wire::ContentBlock::Text { .. }) => {
                        self.blocks.insert(index, BlockKind::Text);
                        vec![] // No LlmEvent for text block start
                    }
                    Some(wire::ContentBlock::ToolUse { id, name }) => {
                        self.blocks.insert(index, BlockKind::ToolUse);
                        vec![LlmEvent::ToolCallStart {
                            index,
                            call_id: id.unwrap_or_default(),
                            name: name.unwrap_or_default(),
                        }]
                    }
                    Some(wire::ContentBlock::Thinking { .. }) => {
                        self.blocks.insert(index, BlockKind::Thinking);
                        vec![] // No LlmEvent for thinking block start
                    }
                    None => vec![],
                }
            }

            "content_block_delta" => {
                let Ok(payload) = serde_json::from_str::<wire::ContentBlockDelta>(&sse.data)
                else {
                    return vec![LlmEvent::Unknown { event_type: Some(event_type.into()), raw: sse.data.clone() }];
                };
                let index = payload.index.unwrap_or(0);
                match payload.delta {
                    Some(wire::Delta::TextDelta { text }) => {
                        vec![LlmEvent::TextDelta { index, text: text.unwrap_or_default() }]
                    }
                    Some(wire::Delta::InputJsonDelta { partial_json }) => {
                        vec![LlmEvent::ToolCallArgumentDelta {
                            index,
                            delta: partial_json.unwrap_or_default(),
                        }]
                    }
                    Some(wire::Delta::ThinkingDelta { thinking }) => {
                        vec![LlmEvent::ThinkingDelta { index, text: thinking.unwrap_or_default() }]
                    }
                    None => vec![],
                }
            }

            "content_block_stop" => {
                let Ok(payload) = serde_json::from_str::<wire::ContentBlockStop>(&sse.data)
                else {
                    return vec![LlmEvent::Unknown { event_type: Some(event_type.into()), raw: sse.data.clone() }];
                };
                let index = payload.index.unwrap_or(0);
                let kind = self.blocks.remove(&index);
                let mut events = vec![LlmEvent::ContentBlockEnd { index }];
                if kind == Some(BlockKind::ToolUse) {
                    events.insert(0, LlmEvent::ToolCallEnd { index });
                }
                events
            }

            "message_delta" => {
                let Ok(payload) = serde_json::from_str::<wire::MessageDelta>(&sse.data)
                else {
                    return vec![LlmEvent::Unknown { event_type: Some(event_type.into()), raw: sse.data.clone() }];
                };
                let mut events = Vec::with_capacity(2);
                if let Some(usage) = payload.usage {
                    let mut details = BTreeMap::new();
                    if let Some(crt) = usage.cache_read_input_tokens {
                        details.insert("cache_read".into(), crt);
                    }
                    events.push(LlmEvent::Usage {
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                        details,
                    });
                }
                if let Some(delta) = payload.delta {
                    if delta.stop_reason.is_some() {
                        // Don't emit MessageEnd here -- wait for message_stop
                    }
                }
                events
            }

            "message_stop" => {
                vec![LlmEvent::MessageEnd { stop_reason: None }]
            }

            "ping" => vec![], // Heartbeat, ignore

            "error" => {
                vec![LlmEvent::Unknown { event_type: Some("error".into()), raw: sse.data.clone() }]
            }

            _ => {
                vec![LlmEvent::Unknown { event_type: Some(event_type.into()), raw: sse.data.clone() }]
            }
        }
    }
}

/// Track stop_reason across message_delta -> message_stop.
pub struct AnthropicStreamParserWithState {
    inner: AnthropicStreamParser,
    pending_stop_reason: Option<StopReason>,
}

impl Default for AnthropicStreamParserWithState {
    fn default() -> Self {
        Self::new()
    }
}

impl AnthropicStreamParserWithState {
    pub fn new() -> Self {
        Self {
            inner: AnthropicStreamParser::new(),
            pending_stop_reason: None,
        }
    }

    fn parse_stop_reason(s: &str) -> StopReason {
        match s {
            "end_turn" => StopReason::EndTurn,
            "tool_use" => StopReason::ToolUse,
            "max_tokens" => StopReason::MaxTokens,
            "content_filter" => StopReason::ContentFilter,
            other => StopReason::Other(other.into()),
        }
    }
}

impl ProviderStreamParser for AnthropicStreamParserWithState {
    fn parse_event(&mut self, sse: &SseEvent) -> Vec<LlmEvent> {
        let event_type = sse.event_type.as_deref();

        // Intercept message_delta to capture stop_reason
        if event_type == Some("message_delta") {
            if let Ok(payload) = serde_json::from_str::<wire::MessageDelta>(&sse.data) {
                let mut events = Vec::with_capacity(2);
                if let Some(usage) = payload.usage {
                    let mut details = BTreeMap::new();
                    if let Some(crt) = usage.cache_read_input_tokens {
                        details.insert("cache_read".into(), crt);
                    }
                    events.push(LlmEvent::Usage {
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                        details,
                    });
                }
                if let Some(delta) = payload.delta {
                    if let Some(reason) = delta.stop_reason {
                        self.pending_stop_reason = Some(Self::parse_stop_reason(&reason));
                    }
                }
                return events;
            }
        }

        // Intercept message_stop to emit with cached stop_reason
        if event_type == Some("message_stop") {
            let stop_reason = self.pending_stop_reason.take();
            return vec![LlmEvent::MessageEnd { stop_reason }];
        }

        // Delegate everything else to the inner parser
        self.inner.parse_event(sse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::events::collect_summary;
    use crate::gateway::sse::SseParser;

    #[test]
    fn upstream_url_messages() {
        let p = AnthropicProvider;
        assert_eq!(
            p.upstream_url("/v1/messages", None),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn upstream_url_with_query() {
        let p = AnthropicProvider;
        assert_eq!(
            p.upstream_url("/v1/messages", Some("beta=true")),
            "https://api.anthropic.com/v1/messages?beta=true"
        );
    }

    #[test]
    fn kind_is_anthropic() {
        assert_eq!(AnthropicProvider.kind(), ProviderKind::Anthropic);
    }

    // ── Stream parser: text-only response ───────────────────────────

    #[test]
    fn stream_text_response() {
        let raw = b"\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01\",\"model\":\"claude-sonnet-4-20250514\",\"usage\":{\"input_tokens\":25,\"output_tokens\":1}}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" world!\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\
\n";

        let mut sse_parser = SseParser::new();
        let sse_events = sse_parser.feed(raw);

        let mut parser = AnthropicStreamParserWithState::new();
        let mut llm_events = Vec::new();
        for sse in &sse_events {
            llm_events.extend(parser.parse_event(sse));
        }

        let summary = collect_summary(&llm_events);
        assert_eq!(summary.message_id.as_deref(), Some("msg_01"));
        assert_eq!(summary.model.as_deref(), Some("claude-sonnet-4-20250514"));
        assert_eq!(summary.text, "Hello world!");
        assert!(summary.tool_calls.is_empty());
        assert_eq!(summary.input_tokens, Some(25));
        assert_eq!(summary.output_tokens, Some(5));
        assert_eq!(summary.stop_reason, Some(StopReason::EndTurn));
    }

    // ── Stream parser: tool use ─────────────────────────────────────

    #[test]
    fn stream_tool_use_response() {
        let raw = b"\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_02\",\"model\":\"claude-sonnet-4-20250514\",\"usage\":{\"input_tokens\":100,\"output_tokens\":1}}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"I'll check.\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_01\",\"name\":\"get_weather\",\"input\":{}}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\": \\\"NYC\\\"}\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":1}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":50}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\
\n";

        let mut sse_parser = SseParser::new();
        let sse_events = sse_parser.feed(raw);

        let mut parser = AnthropicStreamParserWithState::new();
        let mut llm_events = Vec::new();
        for sse in &sse_events {
            llm_events.extend(parser.parse_event(sse));
        }

        let summary = collect_summary(&llm_events);
        assert_eq!(summary.text, "I'll check.");
        assert_eq!(summary.tool_calls.len(), 1);
        assert_eq!(summary.tool_calls[0].call_id, "toolu_01");
        assert_eq!(summary.tool_calls[0].name, "get_weather");
        assert_eq!(summary.tool_calls[0].arguments, "{\"city\": \"NYC\"}");
        assert_eq!(summary.stop_reason, Some(StopReason::ToolUse));
    }

    // ── Stream parser: thinking ─────────────────────────────────────

    #[test]
    fn stream_thinking_response() {
        let raw = b"\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_03\",\"model\":\"claude-sonnet-4-20250514\"}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"Let me reason.\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"The answer.\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":1}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":20}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\
\n";

        let mut sse_parser = SseParser::new();
        let sse_events = sse_parser.feed(raw);

        let mut parser = AnthropicStreamParserWithState::new();
        let mut llm_events = Vec::new();
        for sse in &sse_events {
            llm_events.extend(parser.parse_event(sse));
        }

        let summary = collect_summary(&llm_events);
        assert_eq!(summary.thinking, "Let me reason.");
        assert_eq!(summary.text, "The answer.");
    }

    // ── Stream parser: cache_read_input_tokens ──────────────────────

    #[test]
    fn stream_cache_read_tokens() {
        let raw = b"\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_cache\",\"model\":\"claude-sonnet-4-20250514\",\"usage\":{\"input_tokens\":500,\"output_tokens\":1,\"cache_read_input_tokens\":400}}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Cached!\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":10}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\
\n";

        let mut sse_parser = SseParser::new();
        let sse_events = sse_parser.feed(raw);

        let mut parser = AnthropicStreamParserWithState::new();
        let mut llm_events = Vec::new();
        for sse in &sse_events {
            llm_events.extend(parser.parse_event(sse));
        }

        let summary = collect_summary(&llm_events);
        assert_eq!(summary.input_tokens, Some(500));
        assert_eq!(summary.usage_details.get("cache_read"), Some(&400));
        assert_eq!(summary.text, "Cached!");
    }

    // ── Adversarial: malformed JSON in SSE data ─────────────────────

    #[test]
    fn malformed_json_becomes_unknown() {
        let mut parser = AnthropicStreamParserWithState::new();
        let sse = SseEvent {
            event_type: Some("content_block_delta".into()),
            data: "not valid json{{{".into(),
        };
        let events = parser.parse_event(&sse);
        assert_eq!(events.len(), 1);
        match &events[0] {
            LlmEvent::Unknown { event_type, raw } => {
                assert_eq!(event_type.as_deref(), Some("content_block_delta"));
                assert_eq!(raw, "not valid json{{{");
            }
            other => panic!("expected Unknown, got {:?}", other),
        }
    }

    // ── Adversarial: unknown event type ─────────────────────────────

    #[test]
    fn unknown_event_type_passthrough() {
        let mut parser = AnthropicStreamParserWithState::new();
        let sse = SseEvent {
            event_type: Some("future_event".into()),
            data: "{}".into(),
        };
        let events = parser.parse_event(&sse);
        assert_eq!(events.len(), 1);
        matches!(&events[0], LlmEvent::Unknown { .. });
    }

    // ── Adversarial: missing fields in JSON ─────────────────────────

    #[test]
    fn missing_fields_handled_gracefully() {
        let mut parser = AnthropicStreamParserWithState::new();
        // content_block_start with no content_block field
        let sse = SseEvent {
            event_type: Some("content_block_start".into()),
            data: "{\"type\":\"content_block_start\",\"index\":0}".into(),
        };
        let events = parser.parse_event(&sse);
        assert!(events.is_empty()); // No content_block -> no events
    }

    // ── Ping events ignored ─────────────────────────────────────────

    #[test]
    fn ping_events_ignored() {
        let mut parser = AnthropicStreamParserWithState::new();
        let sse = SseEvent {
            event_type: Some("ping".into()),
            data: "{}".into(),
        };
        let events = parser.parse_event(&sse);
        assert!(events.is_empty());
    }
}
