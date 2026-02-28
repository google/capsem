/// Integration tests for the AI gateway -- end-to-end API proxying.
///
/// These tests start the gateway on a local TCP socket, send real API requests
/// through it, and verify:
/// - Correct routing to upstream providers
/// - API key injection works
/// - SSE streaming responses are forwarded correctly
/// - Audit DB records all interactions
/// - Responses are saved to data/fixtures/ for building offline tests
///
/// All tests are #[ignore] by default since they require real API keys.
/// Run with: cargo test --test gateway_integration -- --ignored
///
/// API keys loaded from ~/.capsem/user.toml (capsem settings).
use std::path::PathBuf;
use std::sync::Arc;

use capsem_logger::DbWriter;
use capsem_core::gateway::server::start_standalone;
use capsem_core::gateway::GatewayConfig;

/// Create a test gateway config with API keys from ~/.capsem/user.toml.
/// Returns (config, db, _tempdir) -- caller must hold _tempdir to keep the DB alive.
fn test_config() -> (Arc<GatewayConfig>, Arc<DbWriter>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(DbWriter::open(&dir.path().join("test.db"), 256).unwrap());
    let config = Arc::new(GatewayConfig::from_capsem_settings(Arc::clone(&db)));
    (config, db, dir)
}

/// Save a response to data/fixtures/ for building offline tests later.
fn save_fixture(name: &str, request_body: &str, response_body: &str) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let dir = root.join("data").join("fixtures");
    std::fs::create_dir_all(&dir).ok();

    let fixture = serde_json::json!({
        "request": serde_json::from_str::<serde_json::Value>(request_body).unwrap_or_else(|_| serde_json::Value::String(request_body.to_string())),
        "response": response_body,
    });

    let path = dir.join(format!("{name}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(&fixture).unwrap()).ok();
    eprintln!("Fixture saved to: {}", path.display());
}

// ---------------------------------------------------------------
// Anthropic (claude-haiku-4-5-20251001 -- cheapest/fastest)
// ---------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn anthropic_messages_non_streaming() {
    let (config, db, _dir) = test_config();
    if config.anthropic_api_key.is_none() {
        eprintln!("ANTHROPIC_API_KEY not set, skipping");
        return;
    }

    let addr = start_standalone(config, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let request_body = serde_json::json!({
        "model": "claude-haiku-4-5-20251001",
        "max_tokens": 16,
        "temperature": 0,
        "messages": [{"role": "user", "content": "What is 2+2? Reply with just the number."}]
    });

    let resp = client
        .post(format!("http://{addr}/v1/messages"))
        .header("content-type", "application/json")
        .header("anthropic-version", "2023-06-01")
        .json(&request_body)
        .send()
        .await
        .unwrap();

    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap();
    eprintln!("Anthropic non-streaming: status={status} body={body}");

    assert_eq!(status, 200, "unexpected status: {status}, body: {body}");
    assert!(body.contains('4'), "response should contain 4");

    save_fixture(
        "anthropic_messages_non_streaming",
        &serde_json::to_string(&request_body).unwrap(),
        &body,
    );

    // Give writer thread time to flush.
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    let reader = db.reader().unwrap();
    let calls = reader.recent_model_calls(10).unwrap();
    assert!(!calls.is_empty(), "audit should have recorded the event");
    assert_eq!(calls[0].1.provider, "anthropic");
    assert_eq!(calls[0].1.method, "POST");
    assert_eq!(calls[0].1.path, "/v1/messages");
    assert_eq!(calls[0].1.status_code, Some(200));
    assert!(!calls[0].1.stream);
    assert_eq!(calls[0].1.model.as_deref(), Some("claude-haiku-4-5-20251001"));
}

#[tokio::test]
#[ignore]
async fn anthropic_messages_streaming() {
    let (config, db, _dir) = test_config();
    if config.anthropic_api_key.is_none() {
        eprintln!("ANTHROPIC_API_KEY not set, skipping");
        return;
    }

    let addr = start_standalone(config, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let request_body = serde_json::json!({
        "model": "claude-haiku-4-5-20251001",
        "max_tokens": 16,
        "temperature": 0,
        "stream": true,
        "messages": [{"role": "user", "content": "What is 2+2? Reply with just the number."}]
    });

    let resp = client
        .post(format!("http://{addr}/v1/messages"))
        .header("content-type", "application/json")
        .header("anthropic-version", "2023-06-01")
        .json(&request_body)
        .send()
        .await
        .unwrap();

    let status = resp.status().as_u16();
    assert_eq!(status, 200, "streaming request should succeed");

    let body = resp.text().await.unwrap();
    eprintln!("Anthropic streaming: {} bytes, first 500: {}", body.len(), &body[..body.len().min(500)]);

    assert!(body.contains("event:"), "should contain SSE events");
    assert!(body.contains('4'), "response should contain 4");

    save_fixture(
        "anthropic_messages_streaming",
        &serde_json::to_string(&request_body).unwrap(),
        &body,
    );

    // Wait for audit task to complete.
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    let reader = db.reader().unwrap();
    let calls = reader.recent_model_calls(10).unwrap();
    assert!(!calls.is_empty(), "audit should have recorded streaming event");
    assert_eq!(calls[0].1.provider, "anthropic");
    assert!(calls[0].1.stream);
}

// ---------------------------------------------------------------
// OpenAI (gpt-5-nano -- cheapest)
// ---------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn openai_chat_completions_non_streaming() {
    let (config, db, _dir) = test_config();
    if config.openai_api_key.is_none() {
        eprintln!("OPENAI_API_KEY not set, skipping");
        return;
    }

    let addr = start_standalone(config, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let request_body = serde_json::json!({
        "model": "gpt-5-nano",
        "temperature": 0,
        "max_tokens": 16,
        "messages": [{"role": "user", "content": "What is 2+2? Reply with just the number."}]
    });

    let resp = client
        .post(format!("http://{addr}/v1/chat/completions"))
        .header("content-type", "application/json")
        .json(&request_body)
        .send()
        .await
        .unwrap();

    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap();
    eprintln!("OpenAI non-streaming: status={status} body={body}");

    assert_eq!(status, 200, "unexpected status: {status}, body: {body}");
    assert!(body.contains('4'), "response should contain 4");

    save_fixture(
        "openai_chat_completions_non_streaming",
        &serde_json::to_string(&request_body).unwrap(),
        &body,
    );

    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    let reader = db.reader().unwrap();
    let calls = reader.recent_model_calls(10).unwrap();
    assert!(!calls.is_empty());
    assert_eq!(calls[0].1.provider, "openai");
    assert!(!calls[0].1.stream);
    assert_eq!(calls[0].1.model.as_deref(), Some("gpt-5-nano"));
}

#[tokio::test]
#[ignore]
async fn openai_chat_completions_streaming() {
    let (config, db, _dir) = test_config();
    if config.openai_api_key.is_none() {
        eprintln!("OPENAI_API_KEY not set, skipping");
        return;
    }

    let addr = start_standalone(config, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let request_body = serde_json::json!({
        "model": "gpt-5-nano",
        "temperature": 0,
        "max_tokens": 16,
        "stream": true,
        "messages": [{"role": "user", "content": "What is 2+2? Reply with just the number."}]
    });

    let resp = client
        .post(format!("http://{addr}/v1/chat/completions"))
        .header("content-type", "application/json")
        .json(&request_body)
        .send()
        .await
        .unwrap();

    let status = resp.status().as_u16();
    assert_eq!(status, 200, "streaming request should succeed");

    let body = resp.text().await.unwrap();
    eprintln!("OpenAI streaming: {} bytes", body.len());

    assert!(body.contains("data:"), "should contain SSE data lines");

    save_fixture(
        "openai_chat_completions_streaming",
        &serde_json::to_string(&request_body).unwrap(),
        &body,
    );

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    let reader = db.reader().unwrap();
    let calls = reader.recent_model_calls(10).unwrap();
    assert!(!calls.is_empty());
    assert_eq!(calls[0].1.provider, "openai");
    assert!(calls[0].1.stream);
}

// ---------------------------------------------------------------
// Google Gemini (gemini-2.5-flash-lite -- cheapest/fastest)
// ---------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn google_gemini_generate_content() {
    let (config, db, _dir) = test_config();
    if config.google_api_key.is_none() {
        eprintln!("GEMINI_API_KEY not set, skipping");
        return;
    }

    let addr = start_standalone(config, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let request_body = serde_json::json!({
        "contents": [{"parts": [{"text": "What is 2+2? Reply with just the number."}]}],
        "generationConfig": {"temperature": 0, "maxOutputTokens": 16}
    });

    let resp = client
        .post(format!(
            "http://{addr}/v1beta/models/gemini-2.5-flash-lite:generateContent"
        ))
        .header("content-type", "application/json")
        .json(&request_body)
        .send()
        .await
        .unwrap();

    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap();
    eprintln!("Gemini non-streaming: status={status} body={body}");

    assert_eq!(status, 200, "unexpected status: {status}, body: {body}");
    assert!(body.contains('4'), "response should contain 4");

    save_fixture(
        "google_gemini_generate_content",
        &serde_json::to_string(&request_body).unwrap(),
        &body,
    );

    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    let reader = db.reader().unwrap();
    let calls = reader.recent_model_calls(10).unwrap();
    assert!(!calls.is_empty());
    assert_eq!(calls[0].1.provider, "google");
    assert!(!calls[0].1.stream);
}

#[tokio::test]
#[ignore]
async fn google_gemini_stream_generate_content() {
    let (config, db, _dir) = test_config();
    if config.google_api_key.is_none() {
        eprintln!("GEMINI_API_KEY not set, skipping");
        return;
    }

    let addr = start_standalone(config, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let request_body = serde_json::json!({
        "contents": [{"parts": [{"text": "What is 2+2? Reply with just the number."}]}],
        "generationConfig": {"temperature": 0, "maxOutputTokens": 16}
    });

    let resp = client
        .post(format!(
            "http://{addr}/v1beta/models/gemini-2.5-flash-lite:streamGenerateContent?alt=sse"
        ))
        .header("content-type", "application/json")
        .json(&request_body)
        .send()
        .await
        .unwrap();

    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap();
    eprintln!("Gemini streaming: status={status} {} bytes", body.len());

    assert_eq!(status, 200, "unexpected status: {status}, body: {body}");
    assert!(body.contains('4'), "response should contain 4");

    save_fixture(
        "google_gemini_stream_generate_content",
        &serde_json::to_string(&request_body).unwrap(),
        &body,
    );

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    let reader = db.reader().unwrap();
    let calls = reader.recent_model_calls(10).unwrap();
    assert!(!calls.is_empty());
    assert_eq!(calls[0].1.provider, "google");
    assert!(calls[0].1.stream);
}

// ---------------------------------------------------------------
// Cross-provider
// ---------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn gateway_health_check() {
    let (config, _db, _dir) = test_config();
    let addr = start_standalone(config, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();

    let resp = reqwest::get(format!("http://{addr}/health")).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "ok");
}
