use super::*;
use crate::credential_broker::{broker_observed_credential, CredentialObservation, CredentialProvider};
use crate::net::ai_traffic::provider::ProviderKind;
use crate::net::policy_config::{
    SecurityPluginConfig, SecurityPluginMode, SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource,
};
use capsem_logger::{
    AuditEvent, Decision, DnsEvent, ExecEvent, ExecEventComplete, FileAction, FileEvent, McpCall, ModelCall, NetEvent,
    SubstitutionEvent, WriteOp,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::SystemTime;

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

struct TracePlugin {
    id: &'static str,
    stage: SecurityPluginStage,
}

impl SecurityPlugin for TracePlugin {
    fn id(&self) -> &'static str {
        self.id
    }

    fn stage(&self) -> SecurityPluginStage {
        self.stage
    }

    fn apply(
        &self,
        mut event: SecurityEvent,
        _config: SecurityPluginConfig,
    ) -> Result<SecurityPluginResult, SecurityActionError> {
        event.action_trace.push(PolicyActionId::CredentialBrokerSubstitute);
        event.credential_ref = Some(format!("credential:blake3:{:0<64}", self.id.replace('_', "")));
        Ok(SecurityPluginResult::applied(event))
    }
}

struct MarkDecisionPlugin;

impl SecurityPlugin for MarkDecisionPlugin {
    fn id(&self) -> &'static str {
        "mark_decision"
    }

    fn stage(&self) -> SecurityPluginStage {
        SecurityPluginStage::Preprocess
    }

    fn apply(
        &self,
        mut event: SecurityEvent,
        _config: SecurityPluginConfig,
    ) -> Result<SecurityPluginResult, SecurityActionError> {
        event.request_decision(SecurityDecisionKind::Block);
        event.action_trace.push(PolicyActionId::CredentialBrokerCapture);
        Ok(SecurityPluginResult::applied(event))
    }
}

struct DecisionPlugin {
    id: &'static str,
    stage: SecurityPluginStage,
    requested: SecurityDecisionKind,
}

impl SecurityPlugin for DecisionPlugin {
    fn id(&self) -> &'static str {
        self.id
    }

    fn stage(&self) -> SecurityPluginStage {
        self.stage
    }

    fn apply(
        &self,
        mut event: SecurityEvent,
        _config: SecurityPluginConfig,
    ) -> Result<SecurityPluginResult, SecurityActionError> {
        event.request_decision(self.requested);
        Ok(SecurityPluginResult::applied(event))
    }
}

fn security_rule_set(input: &str) -> SecurityRuleSet {
    let profile = SecurityRuleProfile::parse_toml(input).expect("security rule profile");
    SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).expect("compiled security rules")
}

fn plugin_config(mode: SecurityPluginMode, detection_level: DetectionLevel) -> SecurityPluginConfig {
    SecurityPluginConfig { mode, detection_level }
}

struct RecordingEmitter {
    events: Mutex<Vec<SecurityEvent>>,
}

impl RecordingEmitter {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }
}

impl SecurityEventEmitter for RecordingEmitter {
    fn emit(&self, event: SecurityEvent) -> Result<(), SecurityEmitError> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

#[path = "tests/boundary_events.rs"]
mod boundary_events;
#[path = "tests/decision_ledger.rs"]
mod decision_ledger;
#[path = "tests/event_contract.rs"]
mod event_contract;
#[path = "tests/materialization.rs"]
mod materialization;
#[path = "tests/plugin_boundaries.rs"]
mod plugin_boundaries;
#[path = "tests/plugin_execution.rs"]
mod plugin_execution;

fn net_write(credential_ref: Option<&str>) -> WriteOp {
    WriteOp::NetEvent(NetEvent {
        event_id: None,
        timestamp: SystemTime::now(),
        domain: "example.com".to_string(),
        port: 443,
        decision: Decision::Allowed,
        process_name: None,
        pid: None,
        method: Some("GET".to_string()),
        path: Some("/".to_string()),
        query: None,
        status_code: Some(200),
        bytes_sent: 0,
        bytes_received: 0,
        duration_ms: 1,
        matched_rule: None,
        request_headers: None,
        response_headers: None,
        request_body_preview: None,
        response_body_preview: None,
        request_body_full: None,
        response_body_full: None,
        conn_type: None,
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        trace_id: Some("trace".to_string()),
        credential_ref: credential_ref.map(str::to_string),
    })
}

fn model_write(credential_ref: Option<&str>) -> WriteOp {
    WriteOp::ModelCall(ModelCall {
        event_id: None,
        timestamp: SystemTime::now(),
        provider: "openai".to_string(),
        protocol: Some("openai".to_string()),
        model: Some("gpt-test".to_string()),
        process_name: None,
        pid: None,
        method: "POST".to_string(),
        path: "/v1/responses".to_string(),
        stream: false,
        system_prompt_preview: None,
        messages_count: 1,
        tools_count: 0,
        request_bytes: 2,
        request_body_preview: None,
        request_body_full: None,
        message_id: None,
        status_code: Some(200),
        text_content: None,
        thinking_content: None,
        response_body_full: None,
        stop_reason: None,
        input_tokens: None,
        output_tokens: None,
        usage_details: BTreeMap::new(),
        duration_ms: 1,
        response_bytes: 2,
        estimated_cost_usd: 0.0,
        trace_id: Some("trace".to_string()),
        credential_ref: credential_ref.map(str::to_string),
        tool_calls: Vec::new(),
        tool_responses: Vec::new(),
    })
}

fn mcp_write(method: &str, credential_ref: Option<&str>) -> WriteOp {
    WriteOp::McpCall(McpCall {
        event_id: None,
        timestamp: SystemTime::now(),
        server_name: "server".to_string(),
        method: method.to_string(),
        tool_name: Some("tool".to_string()),
        request_id: Some("1".to_string()),
        request_preview: None,
        response_preview: None,
        decision: "allowed".to_string(),
        duration_ms: 1,
        error_message: None,
        process_name: None,
        bytes_sent: 0,
        bytes_received: 0,
        transport: "vsock_frame".to_string(),
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        trace_id: Some("trace".to_string()),
        credential_ref: credential_ref.map(str::to_string),
    })
}

fn file_write(credential_ref: Option<&str>) -> WriteOp {
    file_write_with_action(FileAction::Created, credential_ref)
}

fn file_write_with_action(action: FileAction, credential_ref: Option<&str>) -> WriteOp {
    WriteOp::FileEvent(FileEvent {
        event_id: None,
        timestamp: SystemTime::now(),
        action,
        path: "/tmp/example".to_string(),
        size: Some(1),
        trace_id: Some("trace".to_string()),
        credential_ref: credential_ref.map(str::to_string),
    })
}

fn exec_write(credential_ref: Option<&str>) -> WriteOp {
    WriteOp::ExecEvent(ExecEvent {
        event_id: None,
        timestamp: SystemTime::now(),
        exec_id: 1,
        command: "true".to_string(),
        source: "api".to_string(),
        trace_id: Some("trace".to_string()),
        process_name: None,
        credential_ref: credential_ref.map(str::to_string),
    })
}

fn exec_complete_write() -> WriteOp {
    WriteOp::ExecEventComplete(ExecEventComplete {
        exec_id: 1,
        exit_code: 0,
        duration_ms: 1,
        stdout_preview: None,
        stderr_preview: None,
        stdout_bytes: 0,
        stderr_bytes: 0,
        pid: Some(2),
    })
}

fn audit_write(credential_ref: Option<&str>) -> WriteOp {
    WriteOp::AuditEvent(AuditEvent {
        event_id: None,
        timestamp: SystemTime::now(),
        pid: 2,
        ppid: 1,
        uid: 1000,
        exe: "/bin/true".to_string(),
        comm: Some("true".to_string()),
        argv: "true".to_string(),
        cwd: Some("/".to_string()),
        tty: None,
        session_id: None,
        audit_id: None,
        exec_event_id: None,
        parent_exe: None,
        trace_id: Some("trace".to_string()),
        credential_ref: credential_ref.map(str::to_string),
    })
}

fn dns_write(credential_ref: Option<&str>) -> WriteOp {
    WriteOp::DnsEvent(DnsEvent {
        event_id: None,
        timestamp: SystemTime::now(),
        qname: "example.com".to_string(),
        qtype: 1,
        qclass: 1,
        rcode: 0,
        answer_ip: Some("93.184.216.34".to_string()),
        decision: "allowed".to_string(),
        matched_rule: None,
        source_proto: Some("udp".to_string()),
        process_name: None,
        upstream_resolver_ms: 1,
        trace_id: Some("trace".to_string()),
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        credential_ref: credential_ref.map(str::to_string),
    })
}

fn substitution_write(credential_ref: &str) -> WriteOp {
    WriteOp::SubstitutionEvent(SubstitutionEvent {
        event_id: None,
        timestamp: SystemTime::now(),
        material_class: "credential".to_string(),
        source: "test".to_string(),
        event_type: Some("http.request".to_string()),
        algorithm: "blake3".to_string(),
        substitution_ref: credential_ref.to_string(),
        outcome: "stored".to_string(),
        provider: Some("openai".to_string()),
        confidence: None,
        trace_id: Some("trace".to_string()),
        context_json: None,
    })
}

fn brokered_anthropic_header_event() -> (
    SecurityEvent,
    String,
    String,
    tempfile::TempDir,
    EnvVarGuard,
    EnvVarGuard,
    tokio::sync::MutexGuard<'static, ()>,
) {
    let lock = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();
    let tmp = tempfile::tempdir().unwrap();
    let store_path = tmp.path().join("broker-store.jsonl");
    let store_guard = EnvVarGuard::set(crate::credential_broker::STORE_PATH_ENV, &store_path);
    let user_config_guard = EnvVarGuard::set("CAPSEM_HOME", tmp.path());
    let raw = "sk-ant-materialize-secret";
    let brokered = broker_observed_credential(&CredentialObservation {
        provider: CredentialProvider::Anthropic,
        raw_value: raw.to_string(),
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
        HttpRequestSecurityEvent::new("api.anthropic.com", Some(ProviderKind::Anthropic), headers, None),
    );

    (
        event,
        brokered.credential_ref,
        raw.to_string(),
        tmp,
        store_guard,
        user_config_guard,
        lock,
    )
}
