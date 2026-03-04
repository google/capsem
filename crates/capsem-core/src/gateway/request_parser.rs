#![allow(dead_code)]
/// Request body parser: extracts structured metadata from inbound LLM API
/// request JSON. Provider-aware, uses targeted serde structs (not `Value`).
///
/// Extracts: model, stream flag, system prompt preview, message/tool counts,
/// and tool_result entries from subsequent requests (for linking tool call
/// lifecycle).

use super::provider::ProviderKind;

/// Fallback for truncated JSON: search for "model":"..." in the first few KB
/// using a simple byte scan.
fn extract_model_field(body: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(body);
    // Look for "model": "..." or "model":"..."
    let pattern = r#""model"\s*:\s*"([^"]+)""#;
    let re = regex::Regex::new(pattern).ok()?;
    re.captures(&s).and_then(|cap| cap.get(1)).map(|m| m.as_str().to_string())
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
        pub text: Option<String>,
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

    let system_prompt_preview = req.system.as_ref().map(|s| {
        match s {
            anthropic_wire::SystemPrompt::Text(t) => t.clone(),
            anthropic_wire::SystemPrompt::Blocks(blocks) => {
                blocks.iter()
                    .filter_map(|b| b.text.as_deref())
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
    });

    let messages = req.messages.as_deref().unwrap_or(&[]);
    let messages_count = messages.len();

    // Extract tool results from tool_result content blocks
    let mut tool_results = Vec::new();
    for msg in messages {
        if msg.role.as_deref() != Some("user") {
            continue;
        }
        if let Some(anthropic_wire::MessageContent::Blocks(blocks)) = &msg.content {
            for block in blocks {
                if block.block_type.as_deref() == Some("tool_result") {
                    if let Some(call_id) = &block.tool_use_id {
                        let content_text = match &block.content {
                            Some(anthropic_wire::ToolResultContent::Text(t)) => t.clone(),
                            Some(anthropic_wire::ToolResultContent::Blocks(bs)) => {
                                bs.iter()
                                    .filter_map(|b| b.text.as_deref())
                                    .collect::<Vec<_>>()
                                    .join("\n")
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
        pub role: Option<String>,
        pub content: Option<MessageContent>,
        pub tool_call_id: Option<String>,
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
    let messages: &[openai_wire::Message] = req.messages.as_deref()
        .or(req.input.as_deref())
        .unwrap_or(&[]);

    // System prompt: from `instructions` field or first system message
    let system_prompt_preview = req.instructions.as_deref()
        .or_else(|| {
            messages.iter()
                .find(|m| m.role.as_deref() == Some("system"))
                .and_then(|m| match &m.content {
                    Some(openai_wire::MessageContent::Text(t)) => Some(t.as_str()),
                    _ => None,
                })
        })
        .map(|s| s.to_string());

    // Extract tool results (role=tool messages)
    let mut tool_results = Vec::new();
    for msg in messages {
        if msg.role.as_deref() != Some("tool") {
            continue;
        }
        if let Some(call_id) = &msg.tool_call_id {
            let content_text = match &msg.content {
                Some(openai_wire::MessageContent::Text(t)) => t.clone(),
                _ => String::new(),
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
        pub response: Option<serde_json::Value>,
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
            parts.iter()
                .filter_map(|p| p.text.as_deref())
                .collect::<Vec<_>>()
                .join("\n")
        })
    });

    let contents = req.contents.as_deref().unwrap_or(&[]);
    let messages_count = contents.len();

    // Extract function responses (tool results)
    let mut tool_results = Vec::new();
    for content in contents {
        if let Some(parts) = &content.parts {
            for part in parts {
                if let Some(fr) = &part.function_response {
                    let name = fr.name.clone().unwrap_or_default();
                    let content_text = fr.response
                        .as_ref()
                        .map(|v| v.to_string())
                        .unwrap_or_default();
                    tool_results.push(ToolResultMeta {
                        // Gemini doesn't have call_id -- use function name as key
                        call_id: name,
                        content_preview: content_text,
                        is_error: false,
                    });
                }
            }
        }
    }

    // Count tools (sum of function declarations across all tool entries)
    let tools_count = req.tools.as_ref().map_or(0, |tools| {
        tools.iter()
            .map(|t| t.function_declarations.as_ref().map_or(0, |fd| fd.len()))
            .sum()
    });

    RequestMeta {
        model: None, // Gemini model is in the URL path, not the body
        stream: false, // Streaming detected from URL path in emit_model_call
        system_prompt_preview,
        messages_count,
        tools_count,
        tool_results,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_model_field() {
        let body = br#"{"model":"claude-3-opus-20240229","messages":[]}"#;
        assert_eq!(extract_model_field(body), Some("claude-3-opus-20240229".to_string()));

        let truncated = br#"{"model": "gpt-4o", "messages": [{"role": "user", "content": "..."#;
        assert_eq!(extract_model_field(truncated), Some("gpt-4o".to_string()));

        let spaced = br#"{ "model" : "test-model" }"#;
        assert_eq!(extract_model_field(spaced), Some("test-model".to_string()));

        let none = br#"{"messages":[]}"#;
        assert_eq!(extract_model_field(none), None);
    }

    #[test]
    fn test_truncated_json_fallback() {
        let truncated = br#"{"model": "claude-3-5-sonnet-20240620", "messages": [{"role": "user", "con"#;
        let meta = parse_request(ProviderKind::Anthropic, truncated);
        assert_eq!(meta.model.as_deref(), Some("claude-3-5-sonnet-20240620"));
        assert_eq!(meta.messages_count, 0); // parsing failed, but model was extracted
    }

    // ── Anthropic ───────────────────────────────────────────────────

    #[test]
    fn anthropic_basic_request() {
        let body = br#"{
            "model": "claude-sonnet-4-20250514",
            "stream": true,
            "system": "You are a helpful assistant.",
            "messages": [
                {"role": "user", "content": "Hi"},
                {"role": "assistant", "content": "Hello!"},
                {"role": "user", "content": "How are you?"}
            ],
            "tools": [
                {"name": "get_weather"},
                {"name": "search"}
            ]
        }"#;

        let meta = parse_request(ProviderKind::Anthropic, body);
        assert_eq!(meta.model.as_deref(), Some("claude-sonnet-4-20250514"));
        assert!(meta.stream);
        assert_eq!(meta.system_prompt_preview.as_deref(), Some("You are a helpful assistant."));
        assert_eq!(meta.messages_count, 3);
        assert_eq!(meta.tools_count, 2);
        assert!(meta.tool_results.is_empty());
    }

    #[test]
    fn anthropic_system_as_blocks() {
        let body = br#"{
            "model": "claude-sonnet-4-20250514",
            "system": [{"type": "text", "text": "Block system prompt."}],
            "messages": [{"role": "user", "content": "Hi"}]
        }"#;

        let meta = parse_request(ProviderKind::Anthropic, body);
        assert_eq!(meta.system_prompt_preview.as_deref(), Some("Block system prompt."));
    }

    #[test]
    fn anthropic_tool_results() {
        let body = br#"{
            "model": "claude-sonnet-4-20250514",
            "messages": [
                {"role": "user", "content": "weather?"},
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "toolu_01", "name": "get_weather", "input": {"city": "NYC"}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_01", "content": "72F and sunny"}
                ]}
            ]
        }"#;

        let meta = parse_request(ProviderKind::Anthropic, body);
        assert_eq!(meta.messages_count, 3);
        assert_eq!(meta.tool_results.len(), 1);
        assert_eq!(meta.tool_results[0].call_id, "toolu_01");
        assert_eq!(meta.tool_results[0].content_preview, "72F and sunny");
        assert!(!meta.tool_results[0].is_error);
    }

    #[test]
    fn anthropic_tool_result_error() {
        let body = br#"{
            "model": "claude-sonnet-4-20250514",
            "messages": [
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_err", "content": "connection timeout", "is_error": true}
                ]}
            ]
        }"#;

        let meta = parse_request(ProviderKind::Anthropic, body);
        assert_eq!(meta.tool_results.len(), 1);
        assert!(meta.tool_results[0].is_error);
    }

    // ── OpenAI ──────────────────────────────────────────────────────

    #[test]
    fn openai_chat_completions_request() {
        let body = br#"{
            "model": "gpt-4o",
            "stream": true,
            "messages": [
                {"role": "system", "content": "You help with code."},
                {"role": "user", "content": "Write hello world"}
            ],
            "tools": [
                {"type": "function", "function": {"name": "run_code"}}
            ]
        }"#;

        let meta = parse_request(ProviderKind::OpenAi, body);
        assert_eq!(meta.model.as_deref(), Some("gpt-4o"));
        assert!(meta.stream);
        assert_eq!(meta.system_prompt_preview.as_deref(), Some("You help with code."));
        assert_eq!(meta.messages_count, 2);
        assert_eq!(meta.tools_count, 1);
    }

    #[test]
    fn openai_responses_api_request() {
        let body = br#"{
            "model": "gpt-4o",
            "instructions": "You are a coding assistant.",
            "input": [
                {"role": "user", "content": "Help me"}
            ]
        }"#;

        let meta = parse_request(ProviderKind::OpenAi, body);
        assert_eq!(meta.system_prompt_preview.as_deref(), Some("You are a coding assistant."));
        assert_eq!(meta.messages_count, 1);
    }

    #[test]
    fn openai_tool_results() {
        let body = br#"{
            "model": "gpt-4o",
            "messages": [
                {"role": "user", "content": "weather?"},
                {"role": "assistant", "content": null},
                {"role": "tool", "tool_call_id": "call_abc", "content": "72F sunny"}
            ]
        }"#;

        let meta = parse_request(ProviderKind::OpenAi, body);
        assert_eq!(meta.tool_results.len(), 1);
        assert_eq!(meta.tool_results[0].call_id, "call_abc");
        assert_eq!(meta.tool_results[0].content_preview, "72F sunny");
    }

    // ── Google ──────────────────────────────────────────────────────

    #[test]
    fn google_basic_request() {
        let body = br#"{
            "contents": [
                {"parts": [{"text": "Hi"}], "role": "user"},
                {"parts": [{"text": "Hello!"}], "role": "model"}
            ],
            "tools": [
                {"functionDeclarations": [{"name": "search"}, {"name": "calc"}]}
            ],
            "systemInstruction": {
                "parts": [{"text": "Be helpful."}]
            }
        }"#;

        let meta = parse_request(ProviderKind::Google, body);
        assert!(meta.model.is_none()); // model is in URL for Google
        assert!(!meta.stream); // streaming detected from URL path, not body
        assert_eq!(meta.system_prompt_preview.as_deref(), Some("Be helpful."));
        assert_eq!(meta.messages_count, 2);
        assert_eq!(meta.tools_count, 2);
    }

    #[test]
    fn google_function_response() {
        let body = br#"{
            "contents": [
                {"parts": [{"text": "weather?"}], "role": "user"},
                {"parts": [{"functionCall": {"name": "get_weather", "args": {"city": "NYC"}}}], "role": "model"},
                {"parts": [{"functionResponse": {"name": "get_weather", "response": {"temp": "72F"}}}], "role": "function"}
            ]
        }"#;

        let meta = parse_request(ProviderKind::Google, body);
        assert_eq!(meta.tool_results.len(), 1);
        assert_eq!(meta.tool_results[0].call_id, "get_weather");
        assert!(meta.tool_results[0].content_preview.contains("72F"));
    }

    // ── Adversarial ─────────────────────────────────────────────────

    #[test]
    fn empty_body() {
        let meta = parse_request(ProviderKind::Anthropic, b"");
        assert!(meta.model.is_none());
        assert_eq!(meta.messages_count, 0);
    }

    #[test]
    fn invalid_json() {
        let meta = parse_request(ProviderKind::OpenAi, b"not json");
        assert!(meta.model.is_none());
        assert_eq!(meta.messages_count, 0);
    }

    #[test]
    fn non_json_content_type() {
        let meta = parse_request(ProviderKind::Google, b"<html>not json</html>");
        assert!(meta.model.is_none());
    }

    #[test]
    fn long_system_prompt_passes_through_untruncated() {
        let long_prompt = "x".repeat(500);
        let body = format!(
            r#"{{"model":"claude-sonnet-4-20250514","system":"{}","messages":[]}}"#,
            long_prompt
        );
        let meta = parse_request(ProviderKind::Anthropic, body.as_bytes());
        let preview = meta.system_prompt_preview.unwrap();
        assert_eq!(preview.len(), 500);
        assert_eq!(preview, long_prompt);
    }

    #[test]
    fn request_without_stream_field_defaults_false() {
        let body = br#"{"model":"claude-sonnet-4-20250514","messages":[{"role":"user","content":"hi"}]}"#;
        let meta = parse_request(ProviderKind::Anthropic, body);
        assert!(!meta.stream);
    }

    #[test]
    fn corrupt_utf8_in_body() {
        // JSON with invalid UTF-8 bytes
        let mut body = br#"{"model":"test","messages":[]}"#.to_vec();
        body[10] = 0xFF;
        let meta = parse_request(ProviderKind::Anthropic, &body);
        // Should return default (parse failure)
        assert!(meta.model.is_none());
    }
}
