use super::*;

#[test]
fn route_anthropic_messages() {
    let (kind, _) = route_provider("/v1/messages").unwrap();
    assert_eq!(kind, ModelProtocol::Anthropic);
}

#[test]
fn route_anthropic_messages_with_query() {
    let (kind, _) = route_provider("/v1/messages?beta=true").unwrap();
    assert_eq!(kind, ModelProtocol::Anthropic);
}

#[test]
fn route_openai_responses() {
    let (kind, _) = route_provider("/v1/responses").unwrap();
    assert_eq!(kind, ModelProtocol::OpenAi);
}

#[test]
fn route_openai_chat_completions() {
    let (kind, _) = route_provider("/v1/chat/completions").unwrap();
    assert_eq!(kind, ModelProtocol::OpenAi);
}

#[test]
fn route_ollama_native_chat() {
    let (kind, provider) = route_provider("/api/chat").unwrap();
    assert_eq!(kind, ModelProtocol::Ollama);
    assert_eq!(provider.kind(), ModelProtocol::Ollama);
    assert_eq!(provider.upstream_base_url(), "http://127.0.0.1:11434");
}

#[test]
fn route_google_gemini() {
    let (kind, _) = route_provider("/v1beta/models/gemini-2.5-pro:streamGenerateContent").unwrap();
    assert_eq!(kind, ModelProtocol::Google);
}

#[test]
fn route_google_gemini_generate() {
    let (kind, _) = route_provider("/v1beta/models/gemini-2.5-pro:generateContent").unwrap();
    assert_eq!(kind, ModelProtocol::Google);
}

#[test]
fn route_unknown_returns_none() {
    assert!(route_provider("/v2/something").is_none());
    assert!(route_provider("/health").is_none());
    assert!(route_provider("/").is_none());
}

#[test]
fn provider_kind_as_str() {
    assert_eq!(ModelProtocol::Anthropic.as_str(), "anthropic");
    assert_eq!(ModelProtocol::OpenAi.as_str(), "openai");
    assert_eq!(ModelProtocol::Google.as_str(), "google");
    assert_eq!(ModelProtocol::Ollama.as_str(), "ollama");
}

#[test]
fn model_protocol_accepts_openai_compatible_without_new_provider_variant() {
    assert_eq!(
        ModelProtocol::try_from("openai-compatible").unwrap(),
        ModelProtocol::OpenAi
    );
    assert_eq!(
        ModelProtocol::try_from("openai_compatible").unwrap(),
        ModelProtocol::OpenAi
    );
    assert_eq!(
        ModelProtocol::try_from("gemini").unwrap(),
        ModelProtocol::Google
    );
    assert_eq!(
        ModelProtocol::try_from("ollama").unwrap(),
        ModelProtocol::Ollama
    );
    assert!(ModelProtocol::try_from("private-vendor").is_err());
}

#[test]
fn native_ollama_protocol_does_not_borrow_openai_sse_parser() {
    let mut parser = ModelProtocol::Ollama.create_parser();
    let events = parser.parse_event(&crate::net::parsers::sse_parser::SseEvent {
        event_type: Some("message".into()),
        data: r#"{"choices":[{"delta":{"content":"not ollama"}}]}"#.into(),
    });
    assert!(events.is_empty());
}

// -- extract_model_from_path --

#[test]
fn extract_model_gemini_stream() {
    assert_eq!(
        extract_model_from_path("/v1beta/models/gemini-2.5-flash:streamGenerateContent"),
        Some("gemini-2.5-flash".to_string())
    );
}

#[test]
fn extract_model_gemini_generate() {
    assert_eq!(
        extract_model_from_path("/v1beta/models/gemini-2.5-pro:generateContent"),
        Some("gemini-2.5-pro".to_string())
    );
}

#[test]
fn extract_model_no_models_segment() {
    assert_eq!(extract_model_from_path("/v1/messages"), None);
}

#[test]
fn extract_model_empty_model() {
    assert_eq!(
        extract_model_from_path("/v1beta/models/:generateContent"),
        None
    );
}

// -- tool_origin --

#[test]
fn tool_origin_native_tools() {
    assert_eq!(tool_origin("write_file"), "native");
    assert_eq!(tool_origin("bash"), "native");
    assert_eq!(tool_origin("run_shell_command"), "native");
    assert_eq!(tool_origin("read_file"), "native");
}

#[test]
fn tool_origin_local_builtin_tools() {
    assert_eq!(tool_origin("fetch_http"), "local");
    assert_eq!(tool_origin("grep_http"), "local");
    assert_eq!(tool_origin("http_headers"), "local");
}

#[test]
fn tool_origin_mcp_proxy_tools() {
    assert_eq!(tool_origin("github__list_issues"), "mcp_proxy");
    assert_eq!(tool_origin("jira__create_ticket"), "mcp_proxy");
    assert_eq!(tool_origin("custom_server__my_tool"), "mcp_proxy");
}
