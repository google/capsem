/// Anthropic provider: handles /v1/messages requests.
///
/// Key injection: x-api-key header.
/// Upstream: https://api.anthropic.com
///
/// SSE stream format: Anthropic uses `event:` lines to distinguish event types.
/// Content blocks are interleaved (text, tool_use, thinking) with index-based
/// tracking.
use std::collections::{BTreeMap, HashMap};

use crate::net::ai_traffic::provider::{Provider, ProviderKind};
use capsem_network_engine::model_stream::{LlmEvent, ProviderStreamParser, StopReason};
use capsem_network_engine::sse_parser::SseEvent;

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
        ToolUse {
            id: Option<String>,
            name: Option<String>,
        },
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
    #[allow(clippy::enum_variant_names)]
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
        Self {
            blocks: HashMap::new(),
        }
    }
}

impl ProviderStreamParser for AnthropicStreamParser {
    fn parse_event(&mut self, sse: &SseEvent) -> Vec<LlmEvent> {
        let event_type = match &sse.event_type {
            Some(t) => t.as_str(),
            None => {
                return vec![LlmEvent::Unknown {
                    event_type: None,
                    raw: sse.data.clone(),
                }]
            }
        };

        match event_type {
            "message_start" => {
                let Ok(payload) = serde_json::from_str::<wire::MessageStartPayload>(&sse.data)
                else {
                    return vec![LlmEvent::Unknown {
                        event_type: Some(event_type.into()),
                        raw: sse.data.clone(),
                    }];
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
                let Ok(payload) = serde_json::from_str::<wire::ContentBlockStart>(&sse.data) else {
                    return vec![LlmEvent::Unknown {
                        event_type: Some(event_type.into()),
                        raw: sse.data.clone(),
                    }];
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
                let Ok(payload) = serde_json::from_str::<wire::ContentBlockDelta>(&sse.data) else {
                    return vec![LlmEvent::Unknown {
                        event_type: Some(event_type.into()),
                        raw: sse.data.clone(),
                    }];
                };
                let index = payload.index.unwrap_or(0);
                match payload.delta {
                    Some(wire::Delta::TextDelta { text }) => {
                        vec![LlmEvent::TextDelta {
                            index,
                            text: text.unwrap_or_default(),
                        }]
                    }
                    Some(wire::Delta::InputJsonDelta { partial_json }) => {
                        vec![LlmEvent::ToolCallArgumentDelta {
                            index,
                            delta: partial_json.unwrap_or_default(),
                        }]
                    }
                    Some(wire::Delta::ThinkingDelta { thinking }) => {
                        vec![LlmEvent::ThinkingDelta {
                            index,
                            text: thinking.unwrap_or_default(),
                        }]
                    }
                    None => vec![],
                }
            }

            "content_block_stop" => {
                let Ok(payload) = serde_json::from_str::<wire::ContentBlockStop>(&sse.data) else {
                    return vec![LlmEvent::Unknown {
                        event_type: Some(event_type.into()),
                        raw: sse.data.clone(),
                    }];
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
                let Ok(payload) = serde_json::from_str::<wire::MessageDelta>(&sse.data) else {
                    return vec![LlmEvent::Unknown {
                        event_type: Some(event_type.into()),
                        raw: sse.data.clone(),
                    }];
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
                vec![LlmEvent::Unknown {
                    event_type: Some("error".into()),
                    raw: sse.data.clone(),
                }]
            }

            _ => {
                vec![LlmEvent::Unknown {
                    event_type: Some(event_type.into()),
                    raw: sse.data.clone(),
                }]
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
mod tests;
