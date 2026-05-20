use super::*;

#[tokio::test]
async fn policy_model_request_allow_dispatches_and_records_policy_fields() {
    let (port, upstream_task) = spawn_http_fixture_response(
        200,
        "OK",
        vec![("content-type", "application/json")],
        r#"{"id":"chatcmpl-test","choices":[]}"#,
    )
    .await;
    let config = make_config_with_rules_policy(
        allow_local_http_policy(port),
        policy_from_toml(
            r#"
[policy.model.allow_gpt4o]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o" && messages_count == "2" && tools_count == "1"'
decision = "allow"
priority = 10
reason = "Allow the local model fixture"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-4o", "allow-secret").await;
    assert_eq!(status, 200);
    assert!(response_body.contains("chatcmpl-test"));
    drop(sender);
    let _ = proxy_task.await;
    let upstream_request = upstream_task.await.unwrap();
    assert!(
        upstream_request.contains("allow-secret"),
        "allow must preserve the original request body for upstream dispatch"
    );

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Allowed);
    assert_eq!(event.status_code, Some(200));
    assert!(event.bytes_sent > 0);
    assert_eq!(event.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(event.policy_action.as_deref(), Some("allow"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.allow_gpt4o")
    );
    assert_eq!(
        event.policy_reason.as_deref(),
        Some("Allow the local model fixture")
    );
    let model_calls = config.db.reader().unwrap().recent_model_calls(10).unwrap();
    assert_eq!(model_calls.len(), 1);
    let call = &model_calls[0].1;
    assert_eq!(call.provider, "openai");
    assert_eq!(call.model.as_deref(), Some("gpt-4o"));
    assert_eq!(call.messages_count, 2);
    assert_eq!(call.tools_count, 1);
    assert!(call.request_bytes > 0);
    assert!(
        call.request_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("allow-secret"),
        "allowed model request telemetry should retain the captured request preview"
    );
}

#[tokio::test]
async fn policy_model_request_block_stops_before_upstream_and_records_policy_fields() {
    let (port, upstream_task) = spawn_http_no_touch_fixture().await;
    let config = make_config_with_rules_policy(
        allow_local_http_policy(port),
        policy_from_toml(
            r#"
[policy.model.block_gpt4o]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o" && request.body.contains("block-secret")'
decision = "block"
priority = 10
reason = "Do not send this model request"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-4o", "block-secret").await;
    assert_eq!(status, 403);
    assert!(response_body.contains("policy.model.block_gpt4o"));
    drop(sender);
    let _ = proxy_task.await;
    upstream_task.await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Denied);
    assert_eq!(event.status_code, Some(403));
    assert!(event.bytes_sent > 0);
    assert_eq!(event.policy_mode.as_deref(), Some("enforce"));
    assert_eq!(event.policy_action.as_deref(), Some("block"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.block_gpt4o")
    );
    assert_eq!(
        event.policy_reason.as_deref(),
        Some("Do not send this model request")
    );
    assert!(
        !event
            .request_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("block-secret"),
        "denied model request telemetry must not retain the blocked body"
    );
    let model_calls = config.db.reader().unwrap().recent_model_calls(10).unwrap();
    assert_eq!(model_calls.len(), 1);
    let call = &model_calls[0].1;
    assert_eq!(call.provider, "openai");
    assert_eq!(call.model, None);
    assert!(call.request_bytes > 0);
    assert!(
        !call
            .request_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("block-secret"),
        "denied model call telemetry must not retain the blocked body"
    );
}

#[tokio::test]
async fn policy_model_request_block_matches_truncated_json_before_upstream_dispatch() {
    let (port, upstream_task) = spawn_http_no_touch_fixture().await;
    let config = make_config_with_rules_policy(
        allow_local_http_policy(port),
        policy_from_toml(
            r#"
[policy.model.block_truncated_json]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o-mini" && request.body.contains("truncated-secret")'
decision = "block"
priority = 10
reason = "Block even when the JSON body is truncated"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) = send_openai_json_request(
        &mut sender,
        "api.openai.com",
        "/v1/chat/completions",
        Bytes::from_static(
            br#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"truncated-secret"}"#,
        ),
    )
    .await;
    assert_eq!(status, 403);
    assert!(response_body.contains("policy.model.block_truncated_json"));
    drop(sender);
    let _ = proxy_task.await;
    upstream_task.await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Denied);
    assert_eq!(event.policy_action.as_deref(), Some("block"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.block_truncated_json")
    );
    assert!(
        !event
            .request_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("truncated-secret"),
        "truncated denied body must not leak to net_events"
    );
}

#[tokio::test]
async fn policy_model_request_invalid_condition_fails_closed_without_upstream_dispatch() {
    use std::collections::HashMap;

    let (port, upstream_task) = spawn_http_no_touch_fixture().await;
    let mut model = HashMap::new();
    model.insert(
        "bad_regex".to_string(),
        crate::net::policy::PolicyRuleConfig {
            on: crate::net::policy::PolicyCallback::ModelRequest,
            condition: "request.body.matches(\"[\")".to_string(),
            decision: crate::net::policy::PolicyDecisionKind::Allow,
            priority: 10,
            reason: None,
            rewrite_target: None,
            rewrite_value: None,
            strip_request_headers: Vec::new(),
            strip_response_headers: Vec::new(),
        },
    );
    let policy = Arc::new(tokio::sync::RwLock::new(Arc::new(
        crate::net::policy::PolicyConfig {
            model,
            ..crate::net::policy::PolicyConfig::default()
        },
    )));
    let config = make_config_with_rules_policy(allow_local_http_policy(port), policy);
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-4o", "bad-rule-secret")
            .await;
    assert_eq!(status, 403);
    assert!(response_body.contains("policy.model.invalid_condition"));
    drop(sender);
    let _ = proxy_task.await;
    upstream_task.await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Denied);
    assert_eq!(event.policy_action.as_deref(), Some("block"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.invalid_condition")
    );
    assert!(
        !event
            .request_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("bad-rule-secret"),
        "invalid runtime policy conditions must fail closed without request-body telemetry leakage"
    );
}

#[tokio::test]
async fn policy_model_request_rules_do_not_run_on_non_llm_provider_paths() {
    let (port, upstream_task) = spawn_http_fixture_response(
        200,
        "OK",
        vec![("content-type", "application/json")],
        r#"{"object":"list","data":[]}"#,
    )
    .await;
    let config = make_config_with_rules_policy(
        allow_local_http_policy(port),
        policy_from_toml(
            r#"
[policy.model.block_gpt4o]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o" && request.body.contains("non-llm-secret")'
decision = "block"
priority = 10
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let body = Bytes::from_static(
        br#"{"model":"gpt-4o","messages":[{"role":"user","content":"non-llm-secret"}]}"#,
    );
    let (status, response_body) =
        send_openai_json_request(&mut sender, "api.openai.com", "/v1/models", body).await;
    assert_eq!(status, 200);
    assert!(response_body.contains(r#""object":"list""#));
    drop(sender);
    let _ = proxy_task.await;
    let upstream_request = upstream_task.await.unwrap();
    assert!(
        upstream_request.contains("non-llm-secret"),
        "non-LLM provider paths should not run model.request rules"
    );

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Allowed);
    assert_eq!(event.policy_action, None);
    assert!(config
        .db
        .reader()
        .unwrap()
        .recent_model_calls(10)
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn policy_model_request_ask_placeholder_confirmer_allows_upstream_dispatch() {
    let (port, upstream_task) = spawn_http_fixture_response(
        200,
        "OK",
        vec![("content-type", "application/json")],
        r#"{"id":"resp","choices":[]}"#,
    )
    .await;
    let config = make_config_with_rules_policy(
        allow_local_http_policy(port),
        policy_from_toml(
            r#"
[policy.model.ask_gpt4o]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o"'
decision = "ask"
priority = 10
reason = "Ask before sending this model request"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-4o", "ask-secret").await;
    assert_eq!(status, 200);
    assert!(response_body.contains(r#""id":"resp""#));
    drop(sender);
    let _ = proxy_task.await;
    let upstream_request = upstream_task.await.unwrap();
    assert!(
        upstream_request.contains("ask-secret"),
        "placeholder-confirmed model ask should dispatch the original request upstream"
    );

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Allowed);
    assert!(event.bytes_sent > 0);
    assert_eq!(event.policy_action.as_deref(), Some("allow"));
    assert_eq!(event.policy_rule.as_deref(), Some("policy.model.ask_gpt4o"));
}

#[tokio::test]
async fn policy_model_request_rewrite_redacts_upstream_and_telemetry() {
    let (port, upstream_task) =
        spawn_http_fixture_response(200, "OK", vec![("content-type", "text/plain")], "rewritten")
            .await;
    let config = make_config_with_rules_policy(
        allow_local_http_policy(port),
        policy_from_toml(
            r#"
[policy.model.rewrite_secret]
on = "model.request"
if = 'provider == "openai" && model == "gpt-4o" && request.data.contains("rewrite-secret")'
decision = "rewrite"
priority = 10
reason = "Rewrite secret-bearing model request"
rewrite_target = 'request.data =~ "rewrite-secret-(?P<suffix>[a-z]+)"'
rewrite_value = "[redacted-${suffix}]"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) = send_openai_chat_completion(
        &mut sender,
        "api.openai.com",
        "gpt-4o",
        "rewrite-secret-token",
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(response_body, "rewritten");
    drop(sender);
    let _ = proxy_task.await;
    let upstream_request = upstream_task.await.unwrap();
    assert!(
        upstream_request.contains("[redacted-token]"),
        "rewritten model request should dispatch the redacted body upstream"
    );
    assert!(
        !upstream_request.contains("rewrite-secret-token"),
        "rewritten model request must not dispatch the original secret upstream"
    );

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Allowed);
    assert!(event.bytes_sent > 0);
    assert_eq!(event.policy_action.as_deref(), Some("rewrite"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.rewrite_secret")
    );
    assert!(
        !event
            .request_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("rewrite-secret-token"),
        "model request rewrite telemetry must not retain the original secret"
    );
    assert!(
        event
            .request_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("[redacted-token]"),
        "model request rewrite telemetry should record the redacted request preview"
    );
}

#[tokio::test]
async fn policy_model_response_block_stops_before_guest_and_records_policy_fields() {
    let (port, upstream_task) = spawn_http_fixture_response_owned(
        200,
        "OK",
        vec![("content-type", "text/event-stream")],
        openai_sse_text_response("gpt-4o", "hello response-secret"),
    )
    .await;
    let config = make_config_with_rules_policy(
        allow_local_http_policy(port),
        policy_from_toml(
            r#"
[policy.model.block_secret_response]
on = "model.response"
if = 'provider == "openai" && model == "gpt-4o" && response.text.contains("response-secret")'
decision = "block"
priority = 10
reason = "Do not deliver secret model text"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-4o", "safe").await;
    assert_eq!(status, 403);
    assert!(response_body.contains("policy.model.block_secret_response"));
    assert!(
        !response_body.contains("response-secret"),
        "blocked model response must not reach the guest"
    );
    drop(sender);
    let _ = proxy_task.await;
    let upstream_request = upstream_task.await.unwrap();
    assert!(
        upstream_request.contains("gpt-4o"),
        "response policy should run after upstream dispatch"
    );

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Denied);
    assert_eq!(event.status_code, Some(403));
    assert_eq!(event.policy_action.as_deref(), Some("block"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.block_secret_response")
    );
    assert!(
        !event
            .response_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("response-secret"),
        "blocked model response telemetry must not retain the upstream response"
    );
    let model_calls = config.db.reader().unwrap().recent_model_calls(10).unwrap();
    assert_eq!(model_calls.len(), 1);
    let call = &model_calls[0].1;
    assert_eq!(call.provider, "openai");
    assert_eq!(call.model.as_deref(), Some("gpt-4o"));
    assert!(
        call.text_content
            .as_deref()
            .is_none_or(|text| !text.contains("response-secret")),
        "blocked model response must not populate secret text_content"
    );
}

#[tokio::test]
async fn policy_model_response_block_decodes_gzip_before_guest_delivery() {
    let compressed = gzip_bytes(openai_sse_text_response("gpt-4o", "hello gzip-secret").as_bytes());
    let (port, upstream_task) = spawn_http_fixture_response_bytes(
        200,
        "OK",
        vec![
            ("content-type", "text/event-stream"),
            ("content-encoding", "gzip"),
        ],
        compressed,
    )
    .await;
    let config = make_config_with_rules_policy(
        allow_local_http_policy(port),
        policy_from_toml(
            r#"
[policy.model.block_gzip_secret_response]
on = "model.response"
if = 'provider == "openai" && model == "gpt-4o" && response.text.contains("gzip-secret")'
decision = "block"
priority = 10
reason = "Do not deliver compressed secret model text"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-4o", "safe").await;
    assert_eq!(status, 403);
    assert!(response_body.contains("policy.model.block_gzip_secret_response"));
    assert!(
        !response_body.contains("gzip-secret"),
        "gzip-compressed blocked model response must not reach the guest"
    );
    drop(sender);
    let _ = proxy_task.await;
    let upstream_request = upstream_task.await.unwrap();
    assert!(
        upstream_request.contains("gpt-4o"),
        "response policy should evaluate compressed bodies after upstream dispatch"
    );

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Denied);
    assert_eq!(event.status_code, Some(403));
    assert_eq!(event.policy_action.as_deref(), Some("block"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.block_gzip_secret_response")
    );
    assert!(
        !event
            .response_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("gzip-secret"),
        "blocked compressed response telemetry must not retain secret text"
    );
}

#[tokio::test]
async fn policy_model_response_rewrite_redacts_guest_and_session_db() {
    let (port, upstream_task) = spawn_http_fixture_response_owned(
        200,
        "OK",
        vec![("content-type", "text/event-stream")],
        openai_sse_text_response("gpt-4o", "hello response-secret"),
    )
    .await;
    let config = make_config_with_rules_policy(
        allow_local_http_policy(port),
        policy_from_toml(
            r#"
[policy.model.rewrite_secret_response]
on = "model.response"
if = 'provider == "openai" && response.text.contains("response-secret")'
decision = "rewrite"
priority = 10
reason = "Redact model response text"
rewrite_target = 'response.text =~ "response-secret"'
rewrite_value = "[redacted-response]"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-4o", "safe").await;
    assert_eq!(status, 200);
    assert!(response_body.contains("[redacted-response]"));
    assert!(
        !response_body.contains("response-secret"),
        "rewritten model response must not leak to the guest"
    );
    drop(sender);
    let _ = proxy_task.await;
    let _ = upstream_task.await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Allowed);
    assert_eq!(event.status_code, Some(200));
    assert_eq!(event.policy_action.as_deref(), Some("rewrite"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.rewrite_secret_response")
    );
    let preview = event.response_body_preview.as_deref().unwrap_or_default();
    assert!(preview.contains("[redacted-response]"));
    assert!(
        !preview.contains("response-secret"),
        "rewritten response preview must not retain the original secret"
    );
    let model_calls = config.db.reader().unwrap().recent_model_calls(10).unwrap();
    assert_eq!(model_calls.len(), 1);
    let call = &model_calls[0].1;
    assert_eq!(
        call.text_content.as_deref(),
        Some("hello [redacted-response]")
    );
}

#[tokio::test]
async fn policy_model_tool_call_block_stops_before_guest_and_redacts_telemetry() {
    let (port, upstream_task) = spawn_http_fixture_response_owned(
        200,
        "OK",
        vec![("content-type", "text/event-stream")],
        openai_sse_tool_call_response(
            "gpt-4o",
            "call_secret",
            "leak_secret",
            r#"{"secret":"tool-call-secret"}"#,
        ),
    )
    .await;
    let config = make_config_with_rules_policy(
        allow_local_http_policy(port),
        policy_from_toml(
            r#"
[policy.model.block_secret_tool_call]
on = "model.tool_call"
if = 'provider == "openai" && tool.name == "leak_secret" && tool.arguments.secret.contains("tool-call-secret")'
decision = "block"
priority = 10
reason = "Do not deliver unsafe model tool calls"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-4o", "safe").await;
    assert_eq!(status, 403);
    assert!(response_body.contains("policy.model.block_secret_tool_call"));
    assert!(
        !response_body.contains("tool-call-secret"),
        "blocked provider-emitted tool call must not reach the guest"
    );
    drop(sender);
    let _ = proxy_task.await;
    let _ = upstream_task.await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Denied);
    assert_eq!(event.policy_action.as_deref(), Some("block"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.block_secret_tool_call")
    );
    assert!(
        !event
            .response_body_preview
            .as_deref()
            .unwrap_or_default()
            .contains("tool-call-secret"),
        "blocked tool-call telemetry must not retain upstream arguments"
    );
}

#[tokio::test]
async fn policy_model_tool_call_ask_placeholder_confirmer_allows_guest_delivery() {
    let (port, upstream_task) = spawn_http_fixture_response_owned(
        200,
        "OK",
        vec![("content-type", "text/event-stream")],
        openai_sse_tool_call_response(
            "gpt-4o",
            "call_secret",
            "leak_secret",
            r#"{"secret":"tool-call-secret"}"#,
        ),
    )
    .await;
    let config = make_config_with_rules_policy(
        allow_local_http_policy(port),
        policy_from_toml(
            r#"
[policy.model.ask_secret_tool_call]
on = "model.tool_call"
if = 'provider == "openai" && tool.arguments.secret.contains("tool-call-secret")'
decision = "ask"
priority = 10
reason = "Ask before delivering model tool calls"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-4o", "safe").await;
    assert_eq!(status, 200);
    assert!(
        response_body.contains("tool-call-secret"),
        "placeholder-confirmed model tool call ask should reach the guest"
    );
    drop(sender);
    let _ = proxy_task.await;
    let _ = upstream_task.await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Allowed);
    assert_eq!(event.policy_action.as_deref(), Some("allow"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.ask_secret_tool_call")
    );
}

#[tokio::test]
async fn policy_model_tool_call_rewrite_redacts_guest_and_model_call_rows() {
    let (port, upstream_task) = spawn_http_fixture_response_owned(
        200,
        "OK",
        vec![("content-type", "text/event-stream")],
        openai_sse_tool_call_response(
            "gpt-4o",
            "call_secret",
            "leak_secret",
            r#"{"secret":"tool-call-secret"}"#,
        ),
    )
    .await;
    let config = make_config_with_rules_policy(
        allow_local_http_policy(port),
        policy_from_toml(
            r#"
[policy.model.rewrite_secret_tool_call]
on = "model.tool_call"
if = 'provider == "openai" && tool.name == "leak_secret" && tool.arguments.secret.contains("tool-call-secret")'
decision = "rewrite"
priority = 10
reason = "Redact provider-emitted model tool arguments"
rewrite_target = 'tool.arguments =~ "tool-call-secret"'
rewrite_value = "[redacted-tool-call]"
"#,
        ),
    );
    let (mut sender, proxy_task, _conn_task) =
        open_direct_plain_http_request_conn(&config, "127.0.0.1", port, Some(ProviderKind::OpenAi))
            .await;

    let (status, response_body) =
        send_openai_chat_completion(&mut sender, "api.openai.com", "gpt-4o", "safe").await;
    assert_eq!(status, 200);
    assert!(response_body.contains("[redacted-tool-call]"));
    assert!(
        !response_body.contains("tool-call-secret"),
        "rewritten provider-emitted tool call must not leak to the guest"
    );
    drop(sender);
    let _ = proxy_task.await;
    let _ = upstream_task.await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;
    let events = config.db.reader().unwrap().recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.decision, Decision::Allowed);
    assert_eq!(event.policy_action.as_deref(), Some("rewrite"));
    assert_eq!(
        event.policy_rule.as_deref(),
        Some("policy.model.rewrite_secret_tool_call")
    );
    let preview = event.response_body_preview.as_deref().unwrap_or_default();
    assert!(preview.contains("[redacted-tool-call]"));
    assert!(
        !preview.contains("tool-call-secret"),
        "rewritten tool-call response preview must not retain the original secret"
    );

    let reader = config.db.reader().unwrap();
    let model_calls = reader.recent_model_calls(10).unwrap();
    assert_eq!(model_calls.len(), 1);
    let tool_calls = reader.tool_calls_for(model_calls[0].0).unwrap();
    assert_eq!(tool_calls.len(), 1);
    let tool_call = &tool_calls[0];
    assert_eq!(tool_call.call_id, "call_secret");
    assert_eq!(tool_call.tool_name, "leak_secret");
    assert!(tool_call
        .arguments
        .as_deref()
        .unwrap_or_default()
        .contains("[redacted-tool-call]"));
    assert!(
        !tool_call
            .arguments
            .as_deref()
            .unwrap_or_default()
            .contains("tool-call-secret"),
        "model_calls.tool_calls must store the redacted tool-call arguments"
    );
}
