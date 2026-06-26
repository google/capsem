//! Security action microbenchmarks.
//!
//! These benches keep the T6 security-event/action path measurable without
//! booting a VM or running a daemon. Regenerate with:
//! `cargo bench -p capsem-core --bench security_actions`.

use capsem_core::credential_broker::{
    broker_observed_credential, resolve_broker_reference_for_provider, CredentialObservation,
    CredentialProvider,
};
use capsem_core::net::ai_traffic::provider::ProviderKind;
use capsem_core::net::policy_config::{
    DetectionLevel, SecurityPluginConfig, SecurityPluginMode, SecurityRuleProfile, SecurityRuleSet,
    SecurityRuleSource,
};
use capsem_core::security_engine::{
    materialize_http_request_for_upstream, HttpRequestSecurityEvent, HttpSecurityEvent,
    RuntimeSecurityEvent, RuntimeSecurityEventType, SecurityActionRegistry, SecurityEvent,
    SecurityPluginStage,
};
use capsem_logger::{
    AuditEvent, Decision, DnsEvent, FileAction, FileEvent, McpCall, ModelCall, NetEvent, WriteOp,
};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::collections::BTreeMap;
use std::time::SystemTime;

const STORE_PATH_ENV: &str = "CAPSEM_CREDENTIAL_STORE_PATH";

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn security_rules(toml_text: &str) -> SecurityRuleSet {
    let profile = SecurityRuleProfile::parse_toml(toml_text).expect("bench rules parse");
    SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User)
        .expect("bench rules compile")
}

fn rule_match_set() -> SecurityRuleSet {
    security_rules(
        r#"
[profiles.rules.allow_anthropic]
name = "allow_anthropic"
action = "allow"
match = 'http.host == "api.anthropic.com"'
"#,
    )
}

fn brokered_header_event() -> (SecurityEvent, tempfile::TempDir, Vec<EnvVarGuard>) {
    let tmp = tempfile::tempdir().unwrap();
    let store_path = tmp.path().join("broker-store.json");
    let capsem_home = tmp.path().join("capsem-home");
    let corp_config = tmp.path().join("corp.toml");
    std::fs::create_dir_all(&capsem_home).unwrap();
    std::fs::write(capsem_home.join("settings.toml"), "").unwrap();
    std::fs::write(&corp_config, "").unwrap();
    let guards = vec![
        EnvVarGuard::set(STORE_PATH_ENV, store_path.as_os_str()),
        EnvVarGuard::set("CAPSEM_HOME", capsem_home.as_os_str()),
        EnvVarGuard::set("CAPSEM_CORP_CONFIG", corp_config.as_os_str()),
    ];
    let brokered = broker_observed_credential(&CredentialObservation {
        provider: CredentialProvider::Anthropic,
        raw_value: "sk-ant-security-action-bench".to_string(),
        source: "http.request.headers.authorization".to_string(),
        event_type: Some("http.request".to_string()),
        trace_id: None,
        context_json: None,
    })
    .unwrap();

    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::AUTHORIZATION,
        http::HeaderValue::from_str(&brokered.credential_ref).unwrap(),
    );

    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http_request(
        HttpRequestSecurityEvent::new(
            "api.anthropic.com",
            Some(ProviderKind::Anthropic),
            headers,
            None,
        ),
    );
    (event, tmp, guards)
}

fn brokered_mcp_auth_ref() -> (String, tempfile::TempDir, Vec<EnvVarGuard>) {
    let tmp = tempfile::tempdir().unwrap();
    let store_path = tmp.path().join("broker-store.json");
    let capsem_home = tmp.path().join("capsem-home");
    let corp_config = tmp.path().join("corp.toml");
    std::fs::create_dir_all(&capsem_home).unwrap();
    std::fs::write(capsem_home.join("settings.toml"), "").unwrap();
    std::fs::write(&corp_config, "").unwrap();
    let guards = vec![
        EnvVarGuard::set(STORE_PATH_ENV, store_path.as_os_str()),
        EnvVarGuard::set("CAPSEM_HOME", capsem_home.as_os_str()),
        EnvVarGuard::set("CAPSEM_CORP_CONFIG", corp_config.as_os_str()),
    ];
    let brokered = broker_observed_credential(&CredentialObservation {
        provider: CredentialProvider::Mcp,
        raw_value: "local-mcp-oauth-token-security-action-bench".to_string(),
        source: "mcp.auth.bench".to_string(),
        event_type: Some("mcp.server.auth".to_string()),
        trace_id: None,
        context_json: None,
    })
    .unwrap();
    (brokered.credential_ref, tmp, guards)
}

fn net_write() -> WriteOp {
    WriteOp::NetEvent(NetEvent {
        event_id: None,
        timestamp: SystemTime::now(),
        domain: "api.anthropic.com".to_string(),
        port: 443,
        decision: Decision::Allowed,
        process_name: Some("bench".to_string()),
        pid: Some(42),
        method: Some("POST".to_string()),
        path: Some("/v1/messages".to_string()),
        query: None,
        status_code: Some(200),
        bytes_sent: 256,
        bytes_received: 512,
        duration_ms: 7,
        matched_rule: None,
        request_headers: None,
        response_headers: None,
        request_body_preview: None,
        request_body_full: None,
        response_body_preview: None,
        response_body_full: None,
        conn_type: Some("https".to_string()),
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        trace_id: Some("bench-trace".to_string()),
        credential_ref: Some(
            "credential:blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_string(),
        ),
    })
}

fn model_write() -> WriteOp {
    WriteOp::ModelCall(ModelCall {
        event_id: None,
        timestamp: SystemTime::now(),
        provider: "anthropic".to_string(),
        protocol: Some("anthropic".to_string()),
        model: Some("claude-bench".to_string()),
        process_name: Some("bench".to_string()),
        pid: Some(42),
        method: "POST".to_string(),
        path: "/v1/messages".to_string(),
        stream: false,
        system_prompt_preview: None,
        messages_count: 2,
        tools_count: 1,
        request_bytes: 256,
        request_body_preview: None,
        request_body_full: None,
        message_id: Some("msg_bench".to_string()),
        status_code: Some(200),
        text_content: Some("ok".to_string()),
        thinking_content: None,
        stop_reason: Some("end_turn".to_string()),
        input_tokens: Some(10),
        output_tokens: Some(2),
        usage_details: BTreeMap::new(),
        duration_ms: 12,
        response_bytes: 128,
        response_body_full: None,
        estimated_cost_usd: 0.0001,
        trace_id: Some("bench-trace".to_string()),
        credential_ref: None,
        tool_calls: Vec::new(),
        tool_responses: Vec::new(),
    })
}

fn mcp_write() -> WriteOp {
    WriteOp::McpCall(McpCall {
        event_id: None,
        timestamp: SystemTime::now(),
        server_name: "bench-server".to_string(),
        method: "tools/call".to_string(),
        tool_name: Some("bench_tool".to_string()),
        request_id: Some("1".to_string()),
        request_preview: Some("{\"x\":1}".to_string()),
        response_preview: Some("{\"ok\":true}".to_string()),
        decision: "allowed".to_string(),
        duration_ms: 3,
        error_message: None,
        process_name: Some("bench".to_string()),
        bytes_sent: 16,
        bytes_received: 16,
        transport: "vsock_frame".to_string(),
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        trace_id: Some("bench-trace".to_string()),
        credential_ref: None,
    })
}

fn dns_write() -> WriteOp {
    WriteOp::DnsEvent(DnsEvent {
        event_id: None,
        timestamp: SystemTime::now(),
        qname: "api.anthropic.com".to_string(),
        qtype: 1,
        qclass: 1,
        rcode: 0,
        answer_ip: Some("93.184.216.34".to_string()),
        decision: "allowed".to_string(),
        matched_rule: None,
        source_proto: Some("udp".to_string()),
        process_name: Some("bench".to_string()),
        upstream_resolver_ms: 1,
        trace_id: Some("bench-trace".to_string()),
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        credential_ref: None,
    })
}

fn file_write() -> WriteOp {
    WriteOp::FileEvent(FileEvent {
        event_id: None,
        timestamp: SystemTime::now(),
        action: FileAction::Read,
        path: "/workspace/security/SKILL.md".to_string(),
        size: Some(4096),
        trace_id: Some("bench-trace".to_string()),
        credential_ref: None,
    })
}

fn process_write() -> WriteOp {
    WriteOp::AuditEvent(AuditEvent {
        event_id: None,
        timestamp: SystemTime::now(),
        pid: 42,
        ppid: 1,
        uid: 1000,
        exe: "/usr/bin/codex".to_string(),
        comm: Some("codex".to_string()),
        argv: "codex run".to_string(),
        cwd: Some("/workspace".to_string()),
        tty: None,
        session_id: None,
        audit_id: Some("bench-audit".to_string()),
        exec_event_id: None,
        parent_exe: Some("/bin/bash".to_string()),
        trace_id: Some("bench-trace".to_string()),
        credential_ref: None,
    })
}

fn bench_rule_match(c: &mut Criterion) {
    let rules = rule_match_set();
    let event =
        SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
            host: Some("api.anthropic.com".to_string()),
            method: Some("POST".to_string()),
            path: Some("/v1/messages".to_string()),
            query: None,
            status: None,
            body: None,
        });

    c.bench_function("security_rule_set_match_allow", |b| {
        b.iter(|| {
            let evaluation = rules.evaluate(black_box(&event)).unwrap();
            black_box(evaluation.enforcement_rules());
        });
    });
}

fn bench_action_chain(c: &mut Criterion) {
    for (label, plugin, stage) in [
        (
            "security_action_plugin_credential_broker",
            "credential_broker",
            SecurityPluginStage::Preprocess,
        ),
        (
            "security_action_plugin_dummy_pre_eicar",
            "dummy_pre_eicar",
            SecurityPluginStage::Preprocess,
        ),
        (
            "security_action_plugin_dummy_post_allow",
            "dummy_post_allow",
            SecurityPluginStage::Postprocess,
        ),
        (
            "security_action_plugin_log_sanitizer",
            "log_sanitizer",
            SecurityPluginStage::Logging,
        ),
    ] {
        let registry = registry_for_plugin(plugin);
        c.bench_function(label, |b| {
            b.iter(|| {
                let event = registry
                    .apply_security_plugins(
                        black_box(stage),
                        SecurityEvent::new(RuntimeSecurityEventType::HttpRequest),
                    )
                    .unwrap();
                black_box(event);
            });
        });
    }
}

fn bench_broker_substitute(c: &mut Criterion) {
    let registry = registry_for_plugin("credential_broker");
    let (event, _tmp, _guards) = brokered_header_event();

    c.bench_function("security_action_broker_substitute_header_ref", |b| {
        b.iter(|| {
            let event = registry
                .apply_security_plugins(
                    black_box(SecurityPluginStage::Preprocess),
                    black_box(event.clone()),
                )
                .unwrap();
            let materialized = materialize_http_request_for_upstream(&event).unwrap();
            black_box(materialized);
        });
    });
}

fn bench_mcp_brokered_auth(c: &mut Criterion) {
    let (credential_ref, _tmp, _guards) = brokered_mcp_auth_ref();

    c.bench_function("mcp_brokered_oauth_resolve", |b| {
        b.iter(|| {
            let resolved = resolve_broker_reference_for_provider(
                CredentialProvider::Mcp,
                black_box(&credential_ref),
            )
            .unwrap();
            black_box(resolved);
        });
    });
}

fn registry_for_plugin(plugin: &str) -> SecurityActionRegistry {
    let mut policy = BTreeMap::new();
    policy.insert(
        plugin.to_string(),
        SecurityPluginConfig {
            mode: SecurityPluginMode::Rewrite,
            detection_level: DetectionLevel::Informational,
        },
    );
    SecurityActionRegistry::with_builtin_actions().with_plugin_policy(policy)
}

fn bench_runtime_event_handoff(c: &mut Criterion) {
    let net = net_write();
    let model = model_write();
    let mcp = mcp_write();
    let dns = dns_write();
    let file = file_write();
    let process = process_write();

    c.bench_function("security_event_runtime_classify_http", |b| {
        b.iter(|| {
            let event = RuntimeSecurityEvent::from_logger_write(black_box(net.clone()));
            black_box(event);
        });
    });

    c.bench_function("security_event_runtime_classify_model", |b| {
        b.iter(|| {
            let event = RuntimeSecurityEvent::from_logger_write(black_box(model.clone()));
            black_box(event);
        });
    });

    c.bench_function("security_event_runtime_classify_mcp", |b| {
        b.iter(|| {
            let event = RuntimeSecurityEvent::from_logger_write(black_box(mcp.clone()));
            black_box(event);
        });
    });

    c.bench_function("security_event_runtime_classify_dns", |b| {
        b.iter(|| {
            let event = RuntimeSecurityEvent::from_logger_write(black_box(dns.clone()));
            black_box(event);
        });
    });

    c.bench_function("security_event_runtime_classify_file", |b| {
        b.iter(|| {
            let event = RuntimeSecurityEvent::from_logger_write(black_box(file.clone()));
            black_box(event);
        });
    });

    c.bench_function("security_event_runtime_classify_process", |b| {
        b.iter(|| {
            let event = RuntimeSecurityEvent::from_logger_write(black_box(process.clone()));
            black_box(event);
        });
    });
}

criterion_group!(
    benches,
    bench_rule_match,
    bench_action_chain,
    bench_broker_substitute,
    bench_mcp_brokered_auth,
    bench_runtime_event_handoff
);
criterion_main!(benches);
