use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use super::*;
use crate::credential_broker::{BrokeredCredential, BrokeredUpstreamCredentials, CredentialObservation};
use crate::net::policy::NetworkMechanics;
use crate::net::policy_config::{
    DetectionLevel, ModelEndpointRegistry, PolicyActionId, SecurityPluginConfig, SecurityPluginMode,
    SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource,
};
use crate::security_engine::{HttpRequestSecurityEvent, RuntimeSecurityEventType, SecurityEvent};
use capsem_credentials::CredentialProvider;
use capsem_logger::{FileAction, FileEvent, FileKind, WriteOp};

fn revision(marker: usize) -> ProxyPolicySnapshot {
    let profile = SecurityRuleProfile::parse_toml(&format!(
        r#"
[profiles.rules.marker_{marker}]
name = "marker_{marker}"
action = "allow"
match = 'http.host == "marker-{marker}.example"'
"#
    ))
    .unwrap();
    let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap();
    let mut network = NetworkMechanics::new();
    network.max_body_capture = marker;
    let plugins = BTreeMap::<String, SecurityPluginConfig>::new();

    ProxyPolicySnapshot::new(
        format!("blake3:{marker:064x}"),
        network,
        rules,
        plugins,
        ModelEndpointRegistry::default(),
    )
}

fn credential_revision() -> ProxyPolicySnapshot {
    ProxyPolicySnapshot::new(
        "blake3:credentials".to_string(),
        NetworkMechanics::new(),
        SecurityRuleSet::new(Vec::new()),
        BTreeMap::from([(
            "credential_broker".to_string(),
            SecurityPluginConfig {
                mode: SecurityPluginMode::Rewrite,
                detection_level: DetectionLevel::Informational,
            },
        )]),
        ModelEndpointRegistry::default(),
    )
}

const ACTIVE_POLICY: &str = r#"
[network]
[network.dns]
upstreams = ["127.0.0.1:5353"]

[user_rules.profiles.rules.worker_http]
name = "worker_http"
action = "allow"
priority = 10
match = 'http.host == "worker.example"'

[corp_rules]

[plugins.credential_broker]
mode = "rewrite"
detection_level = "informational"

[mcp.server_enabled]
local = false
"#;

#[test]
fn runtime_policy_compiles_exact_bytes_without_a_path() {
    let runtime = ProxyRuntimePolicy::compile(ACTIVE_POLICY.as_bytes()).unwrap();
    let snapshot = runtime.snapshot();

    assert_eq!(
        snapshot.digest(),
        crate::net::policy_config::active_policy_digest(ACTIVE_POLICY.as_bytes())
    );
    assert!(snapshot
        .security_rules()
        .rules()
        .iter()
        .any(|rule| rule.rule_id == "profiles.rules.worker_http"));
    assert_eq!(
        snapshot.plugins()["credential_broker"].mode,
        SecurityPluginMode::Rewrite
    );
    assert_eq!(runtime.dns_upstreams(), &["127.0.0.1:5353".parse().unwrap()]);
    assert_eq!(runtime.mcp().server_enabled.get("local"), Some(&false));
}

#[test]
fn malformed_runtime_policy_is_rejected() {
    let error = ProxyRuntimePolicy::compile(b"[network]\nunknown = true")
        .err()
        .expect("invalid policy must fail");

    assert!(error.contains("parse active policy"), "{error}");
}

#[test]
fn policy_handle_replaces_one_immutable_revision_atomically() {
    let handle = ProxyPolicyHandle::new(revision(0));

    for marker in 1..=256 {
        handle.replace(revision(marker));
        let snapshot = handle.snapshot();
        assert_eq!(snapshot.network().max_body_capture, marker);
        assert_eq!(snapshot.digest(), format!("blake3:{marker:064x}"));
        assert_eq!(
            snapshot.security_rules().rules()[0].rule_id,
            format!("profiles.rules.marker_{marker}")
        );
    }
}

#[test]
fn request_snapshot_cannot_mix_concurrent_policy_revisions() {
    let handle = Arc::new(ProxyPolicyHandle::new(revision(0)));
    let writer_handle = Arc::clone(&handle);
    let writer = std::thread::spawn(move || {
        for marker in 1..=10_000 {
            writer_handle.replace(revision(marker % 257));
        }
    });

    for _ in 0..10_000 {
        let snapshot = handle.snapshot();
        let marker = snapshot.network().max_body_capture;
        assert_eq!(snapshot.digest(), format!("blake3:{marker:064x}"));
        assert_eq!(
            snapshot.security_rules().rules()[0].rule_id,
            format!("profiles.rules.marker_{marker}")
        );
    }
    writer.join().unwrap();
}

#[derive(Default)]
struct RecordingLedger {
    writes: Mutex<Vec<&'static str>>,
}

impl ProxyLedger for RecordingLedger {
    fn write(&self, op: WriteOp) -> ProxyCapabilityFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let kind = match op {
                WriteOp::FileEvent(_) => "file",
                _ => "other",
            };
            self.writes.lock().unwrap().push(kind);
            Ok(())
        })
    }
}

#[derive(Default)]
struct RecordingCredentials {
    calls: Mutex<Vec<&'static str>>,
}

impl ProxyCredentials for RecordingCredentials {
    fn capture(&self, observation: &CredentialObservation) -> Result<BrokeredCredential, String> {
        self.calls.lock().unwrap().push("capture");
        Ok(BrokeredCredential {
            provider: observation.provider,
            credential_ref: observation.credential_ref(),
            store_account: "scoped-test-account".to_string(),
            newly_captured: true,
        })
    }

    fn substitute_upstream(
        &self,
        _domain: &str,
        _ai_provider: Option<crate::net::ai_traffic::provider::ProviderKind>,
        headers: &mut http::HeaderMap,
        query: Option<&str>,
    ) -> Result<BrokeredUpstreamCredentials, String> {
        self.calls.lock().unwrap().push("substitute");
        headers.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_static("Bearer scoped-secret"),
        );
        Ok(BrokeredUpstreamCredentials {
            credential_ref: Some("blake3:scoped-reference".to_string()),
            query: query.map(str::to_string),
        })
    }
}

#[tokio::test]
async fn engine_exposes_only_typed_ledger_and_credential_capabilities() {
    let ledger = Arc::new(RecordingLedger::default());
    let credentials = Arc::new(RecordingCredentials::default());
    let engine = ProxyEngine::new(
        ProxyPolicyHandle::new(revision(7)),
        Arc::clone(&ledger) as Arc<dyn ProxyLedger>,
        Arc::clone(&credentials) as Arc<dyn ProxyCredentials>,
    );

    engine
        .write(WriteOp::FileEvent(FileEvent {
            event_id: None,
            timestamp: SystemTime::UNIX_EPOCH,
            action: FileAction::Read,
            path: "/scoped".to_string(),
            size: Some(1),
            kind: FileKind::File,
            trace_id: None,
            credential_ref: None,
        }))
        .await
        .unwrap();

    let observation = CredentialObservation {
        provider: CredentialProvider::Anthropic,
        raw_value: "raw-secret-value".to_string(),
        source: "test".to_string(),
        event_type: None,
        trace_id: None,
        context_json: None,
    };
    let brokered = engine.capture_credential(&observation).unwrap();
    assert_eq!(brokered.credential_ref, observation.credential_ref());

    let mut event =
        SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http_request(HttpRequestSecurityEvent::new(
            "api.anthropic.com",
            None,
            http::HeaderMap::new(),
            Some("x=1".to_string()),
        ));
    event.action_trace.push(PolicyActionId::CredentialBrokerSubstitute);
    let materialized = engine.materialize_http_request(&event).unwrap();
    assert_eq!(
        materialized.headers.get(http::header::AUTHORIZATION).unwrap(),
        "Bearer scoped-secret"
    );
    assert_eq!(materialized.credential_ref.as_deref(), Some("blake3:scoped-reference"));

    assert_eq!(*ledger.writes.lock().unwrap(), vec!["file"]);
    assert_eq!(*credentials.calls.lock().unwrap(), vec!["capture", "substitute"]);
}

#[test]
fn engine_redaction_never_returns_observed_secret() {
    let ledger = Arc::new(RecordingLedger::default());
    let credentials = Arc::new(RecordingCredentials::default());
    let engine = ProxyEngine::new(ProxyPolicyHandle::new(revision(8)), ledger, credentials);
    let observation = CredentialObservation {
        provider: CredentialProvider::OpenAi,
        raw_value: "raw-secret-value".to_string(),
        source: "test".to_string(),
        event_type: None,
        trace_id: None,
        context_json: None,
    };

    let redacted = engine.redact_text("authorization: raw-secret-value", std::slice::from_ref(&observation));
    assert!(!redacted.contains("raw-secret-value"));
    assert!(redacted.contains(&observation.credential_ref()));
}

#[test]
fn engine_evaluation_uses_its_scoped_credential_capability() {
    let credentials = Arc::new(RecordingCredentials::default());
    let engine = ProxyEngine::new(
        ProxyPolicyHandle::new(credential_revision()),
        Arc::new(RecordingLedger::default()),
        Arc::clone(&credentials) as Arc<dyn ProxyCredentials>,
    );
    let snapshot = engine.policy().snapshot();
    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::AUTHORIZATION,
        http::HeaderValue::from_static("Bearer sk-scoped-engine-credential"),
    );
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_http_request(HttpRequestSecurityEvent::new("api.openai.com", None, headers, None));

    let evaluated = engine.evaluate(&snapshot, event).unwrap();

    assert_eq!(*credentials.calls.lock().unwrap(), vec!["capture"]);
    assert_eq!(evaluated.event.credential_observations.len(), 1);
    assert!(evaluated
        .event
        .action_trace
        .contains(&PolicyActionId::CredentialBrokerCapture));
}
