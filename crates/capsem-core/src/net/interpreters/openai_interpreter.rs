/// OpenAI provider: handles /v1/responses and /v1/chat/completions requests.
///
/// Key injection: Authorization: Bearer header.
/// Upstream: https://api.openai.com
///
/// SSE stream format (Chat Completions): No `event:` lines -- all events are
/// `data:` only. Content and tool calls arrive via `choices[].delta`.
/// Stream ends with `data: [DONE]` (filtered by SseParser).
use std::collections::BTreeMap;

use crate::net::ai_traffic::events::{LlmEvent, ProviderStreamParser, StopReason};
use crate::net::ai_traffic::provider::{Provider, ProviderKind};
use crate::net::parsers::sse_parser::SseEvent;

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
                if done.item.as_ref().and_then(|i| i.item_type.as_deref()) == Some("function_call")
                {
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
                                let name = tc
                                    .function
                                    .as_ref()
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
mod tests;
