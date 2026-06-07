//! Google Gemini provider: handles /v1beta/models/* requests.
//!
//! Key injection: ?key= query parameter.
//! Upstream: https://generativelanguage.googleapis.com
//!
//! SSE stream format: Each SSE event is a complete JSON object (not deltas).
//! Parts contain `text`, `functionCall`, or `thought` fields.
//! Gemini doesn't provide tool call IDs -- we generate synthetic ones.

use std::collections::BTreeMap;

use crate::net::ai_traffic::events::{LlmEvent, ProviderStreamParser, StopReason};
use crate::net::ai_traffic::provider::{Provider, ProviderKind};
use crate::net::parsers::sse_parser::SseEvent;

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
        pub args: Option<Box<serde_json::value::RawValue>>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct UsageMetadata {
        pub prompt_token_count: Option<u64>,
        pub candidates_token_count: Option<u64>,
        pub cached_content_token_count: Option<u64>,
        pub thoughts_token_count: Option<u64>,
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
        Self {
            started: false,
            block_index: 0,
        }
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
                                let arguments = fc
                                    .args
                                    .as_ref()
                                    .map(|v| v.get().to_string())
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
            let mut details = BTreeMap::new();
            if let Some(crt) = usage.cached_content_token_count {
                details.insert("cache_read".into(), crt);
            }
            if let Some(tt) = usage.thoughts_token_count {
                details.insert("thinking".into(), tt);
            }
            events.push(LlmEvent::Usage {
                input_tokens: usage.prompt_token_count,
                output_tokens: usage.candidates_token_count,
                details,
            });
        }

        events
    }
}

#[cfg(test)]
mod tests;
