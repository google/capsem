#![allow(dead_code)]
//! Request body parser: extracts structured metadata from inbound LLM API
//! request JSON. Provider-aware, uses targeted serde structs (not `Value`).
//!
//! Extracts: model, stream flag, system prompt preview, message/tool counts,
//! and tool_result entries from subsequent requests (for linking tool call
//! lifecycle).

use super::provider::ProviderKind;

/// Fallback for truncated JSON: search for "model":"..." in the first few KB
/// using a simple byte scan.
fn extract_model_field(body: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(body);
    // Look for "model": "..." or "model":"..."
    let pattern = r#""model"\s*:\s*"([^"]+)""#;
    let re = regex::Regex::new(pattern).ok()?;
    re.captures(&s)
        .and_then(|cap| cap.get(1))
        .map(|m| m.as_str().to_string())
}

/// Metadata extracted from an inbound LLM API request body.
#[derive(Debug, Clone, Default)]
pub struct RequestMeta {
    pub model: Option<String>,
    pub stream: bool,
    pub system_prompt_preview: Option<String>,
    pub messages_count: usize,
    pub tools_count: usize,
    pub tool_results: Vec<ToolResultMeta>,
}

/// A tool result found in the request messages (links back to a previous tool call).
#[derive(Debug, Clone)]
pub struct ToolResultMeta {
    pub call_id: String,
    pub content_preview: String,
    pub is_error: bool,
}

/// Parse an inbound request body, extracting metadata based on provider format.
///
/// Tolerant of malformed input -- returns default RequestMeta on parse failure.
pub fn parse_request(provider: ProviderKind, body: &[u8]) -> RequestMeta {
    if body.is_empty() {
        return RequestMeta::default();
    }

    match provider {
        ProviderKind::Anthropic => parse_anthropic(body),
        ProviderKind::OpenAi => parse_openai(body),
        ProviderKind::Google => parse_google(body),
        ProviderKind::Ollama => parse_ollama(body),
    }
}

// ── Anthropic ───────────────────────────────────────────────────────

mod anthropic_wire {
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub struct Request {
        pub model: Option<String>,
        pub stream: Option<bool>,
        pub system: Option<SystemPrompt>,
        pub messages: Option<Vec<Message>>,
        pub tools: Option<Vec<Tool>>,
    }

    // system can be a string or an array of content blocks
    #[derive(Deserialize)]
    #[serde(untagged)]
    pub enum SystemPrompt {
        Text(String),
        Blocks(Vec<SystemBlock>),
    }

    #[derive(Deserialize)]
    pub struct SystemBlock {
        pub text: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct Message {
        pub role: Option<String>,
        pub content: Option<MessageContent>,
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    pub enum MessageContent {
        Text(String),
        Blocks(Vec<ContentBlock>),
    }

    #[derive(Deserialize)]
    pub struct ContentBlock {
        #[serde(rename = "type")]
        pub block_type: Option<String>,
        pub tool_use_id: Option<String>,
        pub content: Option<ToolResultContent>,
        pub is_error: Option<bool>,
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    pub enum ToolResultContent {
        Text(String),
        Blocks(Vec<ToolResultBlock>),
    }

    #[derive(Deserialize)]
    pub struct ToolResultBlock {
        #[serde(rename = "type")]
        pub block_type: Option<String>,
        pub text: Option<String>,
        pub tool_name: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct Tool {
        pub name: Option<String>,
    }
}

fn parse_anthropic(body: &[u8]) -> RequestMeta {
    let Ok(req) = serde_json::from_slice::<anthropic_wire::Request>(body) else {
        // Fallback for truncated JSON: try to extract the model name
        // so we at least have that metadata for the trace.
        return RequestMeta {
            model: extract_model_field(body),
            ..Default::default()
        };
    };

    let system_prompt_preview = req.system.as_ref().map(|s| match s {
        anthropic_wire::SystemPrompt::Text(t) => t.clone(),
        anthropic_wire::SystemPrompt::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| b.text.as_deref())
            .collect::<Vec<_>>()
            .join("\n"),
    });

    let messages = req.messages.as_deref().unwrap_or(&[]);
    let messages_count = messages.len();

    // Extract tool results from only the TRAILING user message (the new one the
    // agent just appended). Multi-turn conversations re-send the full history,
    // so iterating all messages would re-log previous tool results.
    let mut tool_results = Vec::new();
    for msg in messages.iter().rev() {
        if msg.role.as_deref() != Some("user") {
            break;
        }
        if let Some(anthropic_wire::MessageContent::Blocks(blocks)) = &msg.content {
            for block in blocks {
                if block.block_type.as_deref() == Some("tool_result") {
                    if let Some(call_id) = &block.tool_use_id {
                        let content_text = match &block.content {
                            Some(anthropic_wire::ToolResultContent::Text(t)) => t.clone(),
                            Some(anthropic_wire::ToolResultContent::Blocks(bs)) => {
                                // Prefer text blocks; fall back to block type summaries
                                let texts: Vec<&str> =
                                    bs.iter().filter_map(|b| b.text.as_deref()).collect();
                                if !texts.is_empty() {
                                    texts.join("\n")
                                } else {
                                    // No text blocks -- summarize non-text blocks
                                    bs.iter()
                                        .filter_map(|b| {
                                            let bt = b.block_type.as_deref()?;
                                            if let Some(name) = &b.tool_name {
                                                Some(format!("[{bt}: {name}]"))
                                            } else {
                                                Some(format!("[{bt}]"))
                                            }
                                        })
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                }
                            }
                            None => String::new(),
                        };
                        tool_results.push(ToolResultMeta {
                            call_id: call_id.clone(),
                            content_preview: content_text,
                            is_error: block.is_error.unwrap_or(false),
                        });
                    }
                }
            }
        }
    }

    RequestMeta {
        model: req.model,
        stream: req.stream.unwrap_or(false),
        system_prompt_preview,
        messages_count,
        tools_count: req.tools.as_ref().map_or(0, |t| t.len()),
        tool_results,
    }
}

// ── OpenAI ──────────────────────────────────────────────────────────

mod openai_wire {
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub struct Request {
        pub model: Option<String>,
        pub stream: Option<bool>,
        pub messages: Option<Vec<Message>>,
        // Responses API uses `input` instead of `messages`
        pub input: Option<Vec<Message>>,
        // Chat Completions uses `system` or first message role=system
        // Responses API uses `instructions`
        pub instructions: Option<String>,
        pub tools: Option<Vec<Tool>>,
    }

    #[derive(Deserialize)]
    pub struct Message {
        #[serde(rename = "type")]
        pub item_type: Option<String>,
        pub role: Option<String>,
        pub content: Option<MessageContent>,
        pub tool_call_id: Option<String>,
        pub call_id: Option<String>,
        pub output: Option<String>,
        pub name: Option<String>,
        pub arguments: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    pub enum MessageContent {
        Text(String),
        Parts(Vec<ContentPart>),
    }

    #[derive(Deserialize)]
    pub struct ContentPart {
        #[serde(rename = "type")]
        pub part_type: Option<String>,
        pub text: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct Tool {
        #[serde(rename = "type")]
        pub tool_type: Option<String>,
    }
}

fn parse_openai(body: &[u8]) -> RequestMeta {
    let Ok(req) = serde_json::from_slice::<openai_wire::Request>(body) else {
        // Fallback for truncated JSON
        return RequestMeta {
            model: extract_model_field(body),
            ..Default::default()
        };
    };

    // Messages can come from `messages` (Chat Completions) or `input` (Responses API)
    let messages: &[openai_wire::Message] = req
        .messages
        .as_deref()
        .or(req.input.as_deref())
        .unwrap_or(&[]);

    // System prompt: from `instructions` field or first system message
    let system_prompt_preview = req
        .instructions
        .as_deref()
        .or_else(|| {
            messages
                .iter()
                .find(|m| m.role.as_deref() == Some("system"))
                .and_then(|m| match &m.content {
                    Some(openai_wire::MessageContent::Text(t)) => Some(t.as_str()),
                    _ => None,
                })
        })
        .map(|s| s.to_string());

    // Extract tool results from only the TRAILING tool messages (the new ones
    // the agent just appended). Multi-turn conversations re-send the full
    // history, so iterating all messages would re-log previous tool results.
    let mut tool_results = Vec::new();
    for msg in messages.iter().rev() {
        let is_chat_tool_result = msg.role.as_deref() == Some("tool");
        let is_responses_tool_result = msg.item_type.as_deref() == Some("function_call_output");
        if !is_chat_tool_result && !is_responses_tool_result {
            break;
        }
        if let Some(call_id) = msg.tool_call_id.as_ref().or(msg.call_id.as_ref()) {
            let content_text = match &msg.content {
                Some(openai_wire::MessageContent::Text(t)) => t.clone(),
                Some(openai_wire::MessageContent::Parts(parts)) => parts
                    .iter()
                    .filter_map(|p| p.text.as_deref())
                    .collect::<Vec<_>>()
                    .join("\n"),
                None => msg.output.clone().unwrap_or_default(),
            };
            tool_results.push(ToolResultMeta {
                call_id: call_id.clone(),
                content_preview: content_text,
                is_error: false, // OpenAI doesn't have explicit is_error on tool results
            });
        }
    }

    RequestMeta {
        model: req.model,
        stream: req.stream.unwrap_or(false),
        system_prompt_preview,
        messages_count: messages.len(),
        tools_count: req.tools.as_ref().map_or(0, |t| t.len()),
        tool_results,
    }
}

// ── Google ──────────────────────────────────────────────────────────

mod google_wire {
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Request {
        pub contents: Option<Vec<Content>>,
        pub tools: Option<Vec<Tool>>,
        pub system_instruction: Option<SystemInstruction>,
    }

    #[derive(Deserialize)]
    pub struct Content {
        pub parts: Option<Vec<Part>>,
        pub role: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Part {
        pub text: Option<String>,
        pub function_response: Option<FunctionResponse>,
    }

    #[derive(Deserialize)]
    pub struct FunctionResponse {
        pub name: Option<String>,
        pub response: Option<Box<serde_json::value::RawValue>>,
    }

    #[derive(Deserialize)]
    pub struct Tool {
        #[serde(rename = "functionDeclarations")]
        pub function_declarations: Option<Vec<FunctionDecl>>,
    }

    #[derive(Deserialize)]
    pub struct FunctionDecl {
        pub name: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct SystemInstruction {
        pub parts: Option<Vec<SystemPart>>,
    }

    #[derive(Deserialize)]
    pub struct SystemPart {
        pub text: Option<String>,
    }
}

fn parse_google(body: &[u8]) -> RequestMeta {
    let Ok(req) = serde_json::from_slice::<google_wire::Request>(body) else {
        return RequestMeta::default();
    };

    let system_prompt_preview = req.system_instruction.as_ref().and_then(|si| {
        si.parts.as_ref().map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.text.as_deref())
                .collect::<Vec<_>>()
                .join("\n")
        })
    });

    let contents = req.contents.as_deref().unwrap_or(&[]);
    let messages_count = contents.len();

    // Extract function responses from only the TRAILING function messages (the
    // new ones the agent just appended). Multi-turn conversations re-send the
    // full history, so iterating all messages would re-log previous tool results.
    let mut tool_results = Vec::new();
    let mut counter = 0usize;
    for content in contents.iter().rev() {
        if content.role.as_deref() != Some("function") {
            break;
        }
        if let Some(parts) = &content.parts {
            for part in parts {
                if let Some(fr) = &part.function_response {
                    let name = fr.name.clone().unwrap_or_default();
                    let content_text = fr
                        .response
                        .as_ref()
                        .map(|v| v.get().to_string())
                        .unwrap_or_default();
                    tool_results.push(ToolResultMeta {
                        // Gemini doesn't have call_id -- generate unique IDs
                        call_id: format!("gemini_{}_{}", name, counter),
                        content_preview: content_text,
                        is_error: false,
                    });
                    counter += 1;
                }
            }
        }
    }

    // Count tools (sum of function declarations across all tool entries)
    let tools_count = req.tools.as_ref().map_or(0, |tools| {
        tools
            .iter()
            .map(|t| t.function_declarations.as_ref().map_or(0, |fd| fd.len()))
            .sum()
    });

    RequestMeta {
        model: None,   // Gemini model is in the URL path, not the body
        stream: false, // Streaming detected from URL path in emit_model_call
        system_prompt_preview,
        messages_count,
        tools_count,
        tool_results,
    }
}

// ── Ollama native ──────────────────────────────────────────────────

mod ollama_wire {
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub struct Request {
        pub model: Option<String>,
        pub stream: Option<bool>,
        pub prompt: Option<String>,
        pub messages: Option<Vec<Message>>,
        pub tools: Option<Vec<serde_json::Value>>,
    }

    #[derive(Deserialize)]
    pub struct Message {
        pub role: Option<String>,
        pub content: Option<String>,
    }
}

fn parse_ollama(body: &[u8]) -> RequestMeta {
    let Ok(req) = serde_json::from_slice::<ollama_wire::Request>(body) else {
        return RequestMeta {
            model: extract_model_field(body),
            ..RequestMeta::default()
        };
    };

    let system_prompt_preview = req.messages.as_ref().and_then(|messages| {
        messages
            .iter()
            .find(|message| message.role.as_deref() == Some("system"))
            .and_then(|message| message.content.clone())
    });
    let tool_results = req
        .messages
        .as_ref()
        .map(|messages| {
            messages
                .iter()
                .enumerate()
                .filter(|(_, message)| message.role.as_deref() == Some("tool"))
                .map(|(idx, message)| ToolResultMeta {
                    call_id: format!("ollama_tool_result_{idx}"),
                    content_preview: message.content.clone().unwrap_or_default(),
                    is_error: false,
                })
                .collect()
        })
        .unwrap_or_default();

    RequestMeta {
        model: req.model,
        stream: req.stream.unwrap_or(false),
        system_prompt_preview: system_prompt_preview.or(req.prompt),
        messages_count: req.messages.as_ref().map(|m| m.len()).unwrap_or(0),
        tools_count: req.tools.as_ref().map(|t| t.len()).unwrap_or(0),
        tool_results,
    }
}

#[cfg(test)]
mod tests;
