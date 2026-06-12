use super::*;
use crate::credential_broker::{
    broker_observed_credential, CredentialObservation, CredentialProvider,
};
use crate::net::ai_traffic::provider::ProviderKind;
use crate::net::policy_config::{
    SecurityPluginConfig, SecurityPluginMode, SecurityRuleProfile, SecurityRuleSet,
    SecurityRuleSource,
};
use capsem_logger::{
    AuditEvent, Decision, DnsEvent, ExecEvent, ExecEventComplete, FileAction, FileEvent, McpCall,
    ModelCall, NetEvent, SubstitutionEvent, WriteOp,
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

    fn apply(&self, mut event: SecurityEvent) -> Result<SecurityPluginResult, SecurityActionError> {
        event
            .action_trace
            .push(PolicyActionId::CredentialBrokerSubstitute);
        event.credential_ref = Some(format!(
            "credential:blake3:{:0<64}",
            self.id.replace('_', "")
        ));
        Ok(SecurityPluginResult::applied(event))
    }
}

struct MarkDecisionPlugin;

impl SecurityPlugin for MarkDecisionPlugin {
    fn id(&self) -> &'static str {
        "mark_decision"
    }

    fn stage(&self) -> SecurityPluginStage {
        SecurityPluginStage::Pre
    }

    fn apply(&self, mut event: SecurityEvent) -> Result<SecurityPluginResult, SecurityActionError> {
        event.request_decision(SecurityDecisionKind::Block);
        event
            .action_trace
            .push(PolicyActionId::CredentialBrokerCapture);
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

    fn apply(&self, mut event: SecurityEvent) -> Result<SecurityPluginResult, SecurityActionError> {
        event.request_decision(self.requested);
        Ok(SecurityPluginResult::applied(event))
    }
}

fn security_rule_set(input: &str) -> SecurityRuleSet {
    let profile = SecurityRuleProfile::parse_toml(input).expect("security rule profile");
    SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User)
        .expect("compiled security rules")
}

fn plugin_config(
    mode: SecurityPluginMode,
    detection_level: DetectionLevel,
) -> SecurityPluginConfig {
    SecurityPluginConfig {
        mode,
        detection_level,
    }
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

#[test]
fn security_event_emitter_is_the_auditable_event_boundary() {
    let emitter = RecordingEmitter::new();
    let mut event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest);
    event.credential_ref = Some(
        "credential:blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .to_string(),
    );

    emitter.emit(event.clone()).unwrap();

    assert_eq!(emitter.events.lock().unwrap().as_slice(), [event]);
}

#[test]
fn security_event_engine_runs_enabled_plugins_by_stage() {
    let emitter = Arc::new(RecordingEmitter::new());
    let registry = SecurityActionRegistry::new()
        .with_plugin_policy(BTreeMap::from([
            (
                "trace_pre".to_string(),
                plugin_config(SecurityPluginMode::Rewrite, DetectionLevel::Medium),
            ),
            (
                "trace_post".to_string(),
                plugin_config(SecurityPluginMode::Rewrite, DetectionLevel::Low),
            ),
            (
                "trace_logging".to_string(),
                plugin_config(SecurityPluginMode::Rewrite, DetectionLevel::Informational),
            ),
        ]))
        .register_plugin(TracePlugin {
            id: "trace_post",
            stage: SecurityPluginStage::Post,
        })
        .unwrap()
        .register_plugin(TracePlugin {
            id: "trace_pre",
            stage: SecurityPluginStage::Pre,
        })
        .unwrap()
        .register_plugin(TracePlugin {
            id: "trace_logging",
            stage: SecurityPluginStage::Logging,
        })
        .unwrap();
    let engine = SecurityEventEngine::new(registry, Arc::clone(&emitter));
    let rules = SecurityRuleSet::new(Vec::new());
    let event =
        SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
            host: Some("example.com".to_string()),
            ..Default::default()
        });

    let returned = engine.apply_matching_rules_and_emit(&rules, event).unwrap();

    assert_eq!(
        returned.action_trace,
        [
            PolicyActionId::CredentialBrokerSubstitute,
            PolicyActionId::CredentialBrokerSubstitute,
            PolicyActionId::CredentialBrokerSubstitute
        ],
        "enabled plugins should run once on their declared stage"
    );
    assert_eq!(
        returned
            .detections
            .iter()
            .map(|detection| (
                detection.source,
                detection.plugin_id.as_deref(),
                detection.plugin_mode
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                SecurityDetectionSource::Plugin,
                Some("trace_pre"),
                Some(SecurityPluginMode::Rewrite)
            ),
            (
                SecurityDetectionSource::Plugin,
                Some("trace_post"),
                Some(SecurityPluginMode::Rewrite)
            ),
            (
                SecurityDetectionSource::Plugin,
                Some("trace_logging"),
                Some(SecurityPluginMode::Rewrite)
            ),
        ]
    );
    assert_eq!(emitter.events.lock().unwrap().as_slice(), [returned]);
}

#[test]
fn security_event_engine_skips_disabled_plugins() {
    let emitter = Arc::new(RecordingEmitter::new());
    let registry = SecurityActionRegistry::new()
        .with_plugin_policy(BTreeMap::from([(
            "trace".to_string(),
            plugin_config(SecurityPluginMode::Disable, DetectionLevel::Critical),
        )]))
        .register_plugin(TracePlugin {
            id: "trace",
            stage: SecurityPluginStage::Post,
        })
        .unwrap();
    let engine = SecurityEventEngine::new(registry, Arc::clone(&emitter));
    let rules = SecurityRuleSet::new(Vec::new());
    let event =
        SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
            host: Some("api.openai.com".to_string()),
            ..Default::default()
        });

    let returned = engine
        .apply_matching_rules_and_emit(&rules, event.clone())
        .unwrap();

    assert_eq!(returned, event);
    assert_eq!(emitter.events.lock().unwrap().as_slice(), [event]);
}

#[test]
fn security_event_engine_applies_postprocess_after_preprocess_mutation() {
    let emitter = Arc::new(RecordingEmitter::new());
    let registry = SecurityActionRegistry::new()
        .with_plugin_policy(BTreeMap::from([
            (
                "mark_decision".to_string(),
                plugin_config(SecurityPluginMode::Block, DetectionLevel::High),
            ),
            (
                "trace".to_string(),
                plugin_config(SecurityPluginMode::Rewrite, DetectionLevel::Low),
            ),
        ]))
        .register_plugin(MarkDecisionPlugin)
        .unwrap()
        .register_plugin(TracePlugin {
            id: "trace",
            stage: SecurityPluginStage::Post,
        })
        .unwrap();
    let engine = SecurityEventEngine::new(registry, Arc::clone(&emitter));
    let rules = SecurityRuleSet::new(Vec::new());
    let event =
        SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
            host: Some("example.com".to_string()),
            ..Default::default()
        });

    let returned = engine.apply_matching_rules_and_emit(&rules, event).unwrap();

    assert_eq!(
        returned.action_trace,
        [
            PolicyActionId::CredentialBrokerCapture,
            PolicyActionId::CredentialBrokerSubstitute
        ],
        "postprocess plugins must see the event after preprocess mutation"
    );
    assert_eq!(returned.decision.effective, SecurityDecisionKind::Block);
    assert_eq!(emitter.events.lock().unwrap().as_slice(), [returned]);
}

#[test]
fn security_plugin_policy_supports_rewrite_and_disable_modes() {
    let rules = SecurityRuleSet::new(Vec::new());
    let event =
        SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
            host: Some("example.com".to_string()),
            ..Default::default()
        });

    let rewrite_registry = SecurityActionRegistry::new()
        .with_plugin_policy(BTreeMap::from([(
            "trace".to_string(),
            plugin_config(SecurityPluginMode::Rewrite, DetectionLevel::Medium),
        )]))
        .register_plugin(TracePlugin {
            id: "trace",
            stage: SecurityPluginStage::Post,
        })
        .unwrap();
    let rewrite_returned =
        SecurityEventEngine::new(rewrite_registry, Arc::new(RecordingEmitter::new()))
            .apply_matching_rules_and_emit(&rules, event.clone())
            .unwrap();
    assert_eq!(
        rewrite_returned.action_trace,
        [PolicyActionId::CredentialBrokerSubstitute],
        "rewrite mode must still run the plugin"
    );
    assert_eq!(
        rewrite_returned.decision.effective,
        SecurityDecisionKind::Allow,
        "rewrite is a mutation verb, not a block/ask verdict"
    );

    let disabled_registry = SecurityActionRegistry::new()
        .with_plugin_policy(BTreeMap::from([(
            "trace".to_string(),
            plugin_config(SecurityPluginMode::Disable, DetectionLevel::Critical),
        )]))
        .register_plugin(TracePlugin {
            id: "trace",
            stage: SecurityPluginStage::Post,
        })
        .unwrap();
    let disabled_returned =
        SecurityEventEngine::new(disabled_registry, Arc::new(RecordingEmitter::new()))
            .apply_matching_rules_and_emit(&rules, event)
            .unwrap();
    assert!(
        disabled_returned.action_trace.is_empty(),
        "disabled plugins must not execute"
    );
}

#[test]
fn security_plugin_policy_block_is_absolute_after_later_allow() {
    let emitter = Arc::new(RecordingEmitter::new());
    let registry = SecurityActionRegistry::new()
        .with_plugin_policy(BTreeMap::from([
            (
                "blocker".to_string(),
                plugin_config(SecurityPluginMode::Block, DetectionLevel::High),
            ),
            (
                "allow_after".to_string(),
                plugin_config(SecurityPluginMode::Allow, DetectionLevel::Low),
            ),
        ]))
        .register_plugin(DecisionPlugin {
            id: "blocker",
            stage: SecurityPluginStage::Pre,
            requested: SecurityDecisionKind::Block,
        })
        .unwrap()
        .register_plugin(DecisionPlugin {
            id: "allow_after",
            stage: SecurityPluginStage::Post,
            requested: SecurityDecisionKind::Allow,
        })
        .unwrap();
    let engine = SecurityEventEngine::new(registry, Arc::clone(&emitter));
    let rules = SecurityRuleSet::new(Vec::new());
    let event =
        SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
            host: Some("example.com".to_string()),
            ..Default::default()
        });

    let returned = engine.apply_matching_rules_and_emit(&rules, event).unwrap();

    assert_eq!(
        returned.decision.effective,
        SecurityDecisionKind::Block,
        "later allow requests must not downgrade an effective block"
    );
    assert_eq!(
        emitter.events.lock().unwrap()[0].decision.effective,
        SecurityDecisionKind::Block,
        "the emitted event must preserve the absolute block"
    );
}

#[test]
fn builtin_dummy_plugins_block_eicar_and_cannot_be_downgraded_by_postprocess() {
    let emitter = Arc::new(RecordingEmitter::new());
    let registry =
        SecurityActionRegistry::with_builtin_actions().with_plugin_policy(BTreeMap::from([
            (
                "dummy_pre_eicar".to_string(),
                plugin_config(SecurityPluginMode::Rewrite, DetectionLevel::Critical),
            ),
            (
                "dummy_post_allow".to_string(),
                plugin_config(SecurityPluginMode::Allow, DetectionLevel::Informational),
            ),
        ]));
    let engine = SecurityEventEngine::new(registry, Arc::clone(&emitter));
    let rules = security_rule_set(
        r#"
[profiles.rules.eicar]
name = "eicar_rewrite_scan"
action = "rewrite"
detection_level = "high"
priority = 10
match = 'file.import.content.contains("EICAR")'

[profiles.rules.allow_after]
name = "allow_after_eicar"
action = "postprocess"
detection_level = "low"
priority = 20
match = 'file.import.content.contains("EICAR")'
"#,
    );
    let event =
        SecurityEvent::new(RuntimeSecurityEventType::FileImport).with_file(FileSecurityEvent {
            import_content: Some(DUMMY_EICAR_TEST_STRING.to_string()),
            ..Default::default()
        });

    let returned = engine.apply_matching_rules_and_emit(&rules, event).unwrap();

    assert_eq!(returned.decision.effective, SecurityDecisionKind::Block);
    assert_eq!(
        returned
            .detections
            .iter()
            .map(|detection| (
                detection.source,
                detection.rule_id.as_deref(),
                detection.plugin_id.as_deref(),
                detection.detection_level,
                detection.plugin_mode,
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                SecurityDetectionSource::Plugin,
                None,
                Some("dummy_pre_eicar"),
                DetectionLevel::Critical,
                Some(SecurityPluginMode::Rewrite),
            ),
            (
                SecurityDetectionSource::Rule,
                Some("profiles.rules.eicar"),
                None,
                DetectionLevel::High,
                None,
            ),
            (
                SecurityDetectionSource::Rule,
                Some("profiles.rules.allow_after"),
                None,
                DetectionLevel::Low,
                None,
            ),
            (
                SecurityDetectionSource::Plugin,
                None,
                Some("dummy_post_allow"),
                DetectionLevel::Informational,
                Some(SecurityPluginMode::Allow),
            ),
        ],
        "rule and plugin detections must be carried on one security event"
    );
    assert_eq!(
        returned.action_trace,
        [
            PolicyActionId::CredentialBrokerCapture,
            PolicyActionId::CredentialBrokerSubstitute
        ],
        "dummy pre and post plugins should both execute through the real registry"
    );
    assert_eq!(
        emitter.events.lock().unwrap()[0].decision.effective,
        SecurityDecisionKind::Block
    );
}

#[test]
fn security_event_engine_rejects_missing_security_plugin_and_does_not_emit() {
    let emitter = Arc::new(RecordingEmitter::new());
    let registry = SecurityActionRegistry::new().with_plugin_policy(BTreeMap::from([(
        "credential_broker".to_string(),
        plugin_config(SecurityPluginMode::Rewrite, DetectionLevel::Informational),
    )]));
    let engine = SecurityEventEngine::new(registry, Arc::clone(&emitter));
    let rules = SecurityRuleSet::new(Vec::new());
    let event =
        SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
            host: Some("example.com".to_string()),
            ..Default::default()
        });

    let error = engine
        .apply_matching_rules_and_emit(&rules, event)
        .expect_err("missing plugin should fail closed");

    assert!(
        error
            .to_string()
            .contains("security plugin 'credential_broker' is not registered"),
        "{error}"
    );
    assert!(
        emitter.events.lock().unwrap().is_empty(),
        "plugin failure must not emit a post-action event"
    );
}

#[test]
fn credential_broker_plugin_uses_matched_security_rule_metadata() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();
    let tmp = tempfile::tempdir().unwrap();
    let store_path = tmp.path().join("broker-store.json");
    let _store_guard = EnvVarGuard::set(crate::credential_broker::TEST_STORE_ENV, &store_path);
    let _user_guard = EnvVarGuard::set("CAPSEM_HOME", tmp.path());
    let emitter = Arc::new(RecordingEmitter::new());
    let registry =
        SecurityActionRegistry::with_builtin_actions().with_plugin_policy(BTreeMap::from([(
            "credential_broker".to_string(),
            plugin_config(SecurityPluginMode::Rewrite, DetectionLevel::Informational),
        )]));
    let engine = SecurityEventEngine::new(registry, Arc::clone(&emitter));
    let raw = "github_pat_security_plugin_secret";
    let rules = SecurityRuleSet::new(Vec::new());
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_http(HttpSecurityEvent {
            host: Some("github.com".to_string()),
            ..Default::default()
        })
        .with_credential_observations(vec![CredentialObservation {
            provider: CredentialProvider::Github,
            raw_value: raw.to_string(),
            source: "http.body.response.$.token".to_string(),
            event_type: Some("http.response".to_string()),
            confidence: 1.0,
            trace_id: None,
            context_json: None,
        }]);

    let returned = engine.apply_matching_rules_and_emit(&rules, event).unwrap();

    let credential_ref = returned
        .credential_ref
        .as_deref()
        .expect("credential broker should return a broker reference");
    assert!(capsem_logger::is_credential_reference(credential_ref));
    assert!(!credential_ref.contains(raw));
    assert_eq!(
        crate::credential_broker::resolve_broker_reference_for_provider(
            CredentialProvider::Github,
            credential_ref,
        )
        .unwrap()
        .as_deref(),
        Some(raw)
    );
    assert_eq!(emitter.events.lock().unwrap().as_slice(), [returned]);
}

#[test]
fn security_event_log_sanitizer_logging_plugin_redacts_before_logger_emit() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();
    let tmp = tempfile::tempdir().unwrap();
    let store_path = tmp.path().join("broker-store.json");
    let _store_guard = EnvVarGuard::set(crate::credential_broker::TEST_STORE_ENV, &store_path);
    let _user_guard = EnvVarGuard::set("CAPSEM_HOME", tmp.path());
    let emitter = Arc::new(RecordingEmitter::new());
    let registry =
        SecurityActionRegistry::with_builtin_actions().with_plugin_policy(BTreeMap::from([
            (
                "credential_broker".to_string(),
                plugin_config(SecurityPluginMode::Rewrite, DetectionLevel::Informational),
            ),
            (
                "log_sanitizer".to_string(),
                plugin_config(SecurityPluginMode::Rewrite, DetectionLevel::Informational),
            ),
        ]));
    let engine = SecurityEventEngine::new(registry, Arc::clone(&emitter));
    let raw = "sk-security-event-raw-header";
    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::AUTHORIZATION,
        http::HeaderValue::from_str(&format!("Bearer {raw}")).unwrap(),
    );
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_http_request(HttpRequestSecurityEvent::new(
            "api.openai.com",
            Some(ProviderKind::OpenAi),
            headers,
            None,
        ))
        .with_credential_observations(vec![CredentialObservation {
            provider: CredentialProvider::OpenAi,
            raw_value: raw.to_string(),
            source: "http.request.headers.authorization".to_string(),
            event_type: Some("http.request".to_string()),
            confidence: 1.0,
            trace_id: None,
            context_json: None,
        }]);

    let returned = engine
        .apply_matching_rules_and_emit(&SecurityRuleSet::new(Vec::new()), event)
        .expect("credential broker plus logging sanitizer should emit a safe event");

    let events = emitter.events.lock().unwrap();
    assert_eq!(events.as_slice(), [returned.clone()]);
    let emitted = events.first().expect("sanitized event emitted");
    assert_eq!(
        emitted.credential_observations,
        Vec::<CredentialObservation>::new(),
        "raw observations are runtime-only and must not cross the logging-plugin handoff"
    );
    let auth = emitted
        .http_request
        .as_ref()
        .and_then(|request| request.headers.get(http::header::AUTHORIZATION))
        .and_then(|value| value.to_str().ok())
        .expect("sanitized auth header is preserved as a broker reference");
    assert!(
        auth.contains("credential:blake3:"),
        "sanitized header must preserve auth shape while replacing raw credential: {auth}"
    );
    assert_ne!(auth, raw);
    assert!(
        !format!("{emitted:?}").contains(raw),
        "logging-plugin output must not contain raw credential material"
    );
}

#[test]
fn credential_broker_uses_ai_provider_hint_for_local_openai_compatible_headers() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();
    let tmp = tempfile::tempdir().unwrap();
    let store_path = tmp.path().join("broker-store.json");
    let _store_guard = EnvVarGuard::set(crate::credential_broker::TEST_STORE_ENV, &store_path);
    let _user_guard = EnvVarGuard::set("CAPSEM_HOME", tmp.path());
    let emitter = Arc::new(RecordingEmitter::new());
    let registry =
        SecurityActionRegistry::with_builtin_actions().with_plugin_policy(BTreeMap::from([(
            "credential_broker".to_string(),
            plugin_config(SecurityPluginMode::Rewrite, DetectionLevel::Informational),
        )]));
    let engine = SecurityEventEngine::new(registry, Arc::clone(&emitter));
    let raw = "capsem_test_sdk_api_key_repeat_0123456789abcdef";
    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::AUTHORIZATION,
        http::HeaderValue::from_str(&format!("Bearer {raw}")).unwrap(),
    );
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http_request(
        HttpRequestSecurityEvent::new("127.0.0.1", Some(ProviderKind::OpenAi), headers, None),
    );

    let returned = engine
        .apply_matching_rules_and_emit(&SecurityRuleSet::new(Vec::new()), event)
        .expect("provider hint should let broker capture local OpenAI-compatible SDK keys");

    let credential_ref = returned
        .credential_ref
        .as_deref()
        .expect("provider-hinted credential should be brokered");
    assert!(capsem_logger::is_credential_reference(credential_ref));
    assert_eq!(
        crate::credential_broker::resolve_broker_reference_for_provider(
            CredentialProvider::OpenAi,
            credential_ref,
        )
        .unwrap()
        .as_deref(),
        Some(raw)
    );
    assert_eq!(emitter.events.lock().unwrap().as_slice(), [returned]);
}

#[test]
fn security_event_cel_evaluates_one_cross_root_rule_without_fanout() {
    let condition = r#"
http.host.matches("(^|.*\.)openai\.com$")
|| model.provider == "openai"
|| file.import.path.endsWith(".env")
"#;

    let http_event =
        SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
            host: Some("api.openai.com".to_string()),
            ..Default::default()
        });
    assert!(
        crate::net::policy_config::evaluate_security_event_match(condition, &http_event).unwrap()
    );

    let model_event =
        SecurityEvent::new(RuntimeSecurityEventType::ModelCall).with_model(ModelSecurityEvent {
            provider: Some("openai".to_string()),
            ..Default::default()
        });
    assert!(
        crate::net::policy_config::evaluate_security_event_match(condition, &model_event).unwrap()
    );

    let file_event =
        SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_file(FileSecurityEvent {
            import_path: Some("/workspace/.env".to_string()),
            ..Default::default()
        });
    assert!(
        crate::net::policy_config::evaluate_security_event_match(condition, &file_event).unwrap()
    );
}

#[test]
fn security_event_cel_rejects_credential_and_snapshot_roots() {
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest);

    for condition in [
        r#"credential.ref == "credential:blake3:test""#,
        r#"snapshot.action == "create""#,
    ] {
        let error = crate::net::policy_config::evaluate_security_event_match(condition, &event)
            .expect_err("fake first-party roots must be rejected");
        assert!(
            error.contains("not a first-party security-event root"),
            "{condition}: {error}"
        );
    }
}

#[test]
fn security_event_cel_roots_accept_network_facts_and_reject_decision_state() {
    for condition in [
        r#"ip.value == "127.0.0.1""#,
        r#"tcp.port == "11434""#,
        r#"udp.port == "53""#,
    ] {
        crate::net::policy_config::validate_security_event_match(condition)
            .unwrap_or_else(|error| panic!("{condition} should be an accepted CEL root: {error}"));
    }

    let error =
        crate::net::policy_config::validate_security_event_match(r#"security.decision == "allow""#)
            .expect_err("rules must not predicate on decisions emitted by the rule engine");
    assert!(
        error.contains("not a first-party security-event root"),
        "{error}"
    );
}

#[test]
fn security_event_cel_missing_roots_are_non_matches() {
    let condition = r#"
http.host.matches("(^|.*\.)openai\.com$")
|| model.provider == "openai"
|| file.import.path.endsWith(".env")
"#;
    let dns_event =
        SecurityEvent::new(RuntimeSecurityEventType::DnsQuery).with_dns(DnsSecurityEvent {
            qname: Some("example.com".to_string()),
            qtype: Some("A".to_string()),
        });

    assert!(
        !crate::net::policy_config::evaluate_security_event_match(condition, &dns_event).unwrap()
    );
}

#[test]
fn security_event_cel_exposes_all_first_party_roots() {
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_http(HttpSecurityEvent {
            host: Some("example.com".to_string()),
            ..Default::default()
        })
        .with_dns(DnsSecurityEvent {
            qname: Some("example.com".to_string()),
            ..Default::default()
        })
        .with_mcp(McpSecurityEvent {
            tool_call_name: Some("email_send".to_string()),
            ..Default::default()
        })
        .with_model(ModelSecurityEvent {
            provider: Some("openai".to_string()),
            ..Default::default()
        })
        .with_file(FileSecurityEvent {
            import_path: Some("/workspace/input.txt".to_string()),
            import_name: Some("input.txt".to_string()),
            import_ext: Some("txt".to_string()),
            import_mime_type: Some("text/plain".to_string()),
            import_content: Some("incoming".to_string()),
            export_path: Some("/workspace/output.json".to_string()),
            export_name: Some("output.json".to_string()),
            export_ext: Some("json".to_string()),
            export_mime_type: Some("application/json".to_string()),
            export_content: Some("{\"ok\":true}".to_string()),
            read_path: Some("/Users/elie/.codex/skills/dev-sprint/SKILL.md".to_string()),
            read_name: Some("SKILL.md".to_string()),
            read_ext: Some("md".to_string()),
            read_mime_type: Some("text/markdown".to_string()),
            read_content: Some("# Development Sprint".to_string()),
            create_path: Some("/workspace/report.md".to_string()),
            create_name: Some("report.md".to_string()),
            create_ext: Some("md".to_string()),
            create_mime_type: Some("text/markdown".to_string()),
            create_content: Some("# Report".to_string()),
            write_path: Some("/workspace/report.md".to_string()),
            write_name: Some("report.md".to_string()),
            write_ext: Some("md".to_string()),
            write_mime_type: Some("text/markdown".to_string()),
            write_content: Some("updated".to_string()),
            delete_path: Some("/workspace/old.txt".to_string()),
            delete_name: Some("old.txt".to_string()),
            delete_ext: Some("txt".to_string()),
            delete_mime_type: Some("text/plain".to_string()),
            delete_content: Some("stale".to_string()),
            ..Default::default()
        })
        .with_process(ProcessSecurityEvent {
            command: Some("python main.py".to_string()),
            ..Default::default()
        })
        .with_ip(IpSecurityEvent {
            value: Some("127.0.0.1".to_string()),
            version: Some("4".to_string()),
        })
        .with_tcp(TcpSecurityEvent {
            port: Some("11434".to_string()),
        })
        .with_udp(UdpSecurityEvent {
            port: Some("53".to_string()),
        });

    let conditions = [
        r#"http.valid == "true""#,
        r#"http.host == "example.com""#,
        r#"dns.valid == "true""#,
        r#"dns.qname == "example.com""#,
        r#"mcp.valid == "true""#,
        r#"mcp.tool_call.valid == "true""#,
        r#"mcp.tool_call.name.contains("email")"#,
        r#"model.valid == "true""#,
        r#"model.request.valid == "false""#,
        r#"model.response.valid == "false""#,
        r#"model.provider == "openai""#,
        r#"file.valid == "true""#,
        r#"file.import.valid == "true""#,
        r#"file.import.path.endsWith("input.txt")"#,
        r#"file.import.name == "input.txt""#,
        r#"file.import.ext == "txt""#,
        r#"file.import.mime_type == "text/plain""#,
        r#"file.import.content.contains("incoming")"#,
        r#"file.export.valid == "true""#,
        r#"file.export.path.endsWith("output.json")"#,
        r#"file.export.name == "output.json""#,
        r#"file.export.ext == "json""#,
        r#"file.export.mime_type == "application/json""#,
        r#"file.export.content.contains("ok")"#,
        r#"file.read.valid == "true""#,
        r#"file.read.path.matches("(^|.*/)skills/.+\.md$")"#,
        r#"file.read.name == "SKILL.md""#,
        r#"file.read.ext == "md""#,
        r#"file.read.mime_type == "text/markdown""#,
        r#"file.read.content.contains("Development Sprint")"#,
        r#"file.create.valid == "true""#,
        r#"file.create.path.endsWith("report.md")"#,
        r#"file.create.name == "report.md""#,
        r#"file.create.ext == "md""#,
        r#"file.create.mime_type == "text/markdown""#,
        r#"file.create.content.contains("Report")"#,
        r#"file.write.valid == "true""#,
        r#"file.write.path.endsWith("report.md")"#,
        r#"file.write.name == "report.md""#,
        r#"file.write.ext == "md""#,
        r#"file.write.mime_type == "text/markdown""#,
        r#"file.write.content.contains("updated")"#,
        r#"file.delete.valid == "true""#,
        r#"file.delete.path.endsWith("old.txt")"#,
        r#"file.delete.name == "old.txt""#,
        r#"file.delete.ext == "txt""#,
        r#"file.delete.mime_type == "text/plain""#,
        r#"file.delete.content.contains("stale")"#,
        r#"process.valid == "true""#,
        r#"process.audit.valid == "true""#,
        r#"process.command.contains("python")"#,
        r#"ip.valid == "true""#,
        r#"ip.value == "127.0.0.1""#,
        r#"ip.version == "4""#,
        r#"tcp.valid == "true""#,
        r#"tcp.port == "11434""#,
        r#"udp.valid == "true""#,
        r#"udp.port == "53""#,
    ];
    let covered_roots = conditions
        .iter()
        .map(|condition| condition.split('.').next().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    let expected_roots = crate::net::policy_config::SECURITY_EVENT_CEL_ROOTS
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        covered_roots, expected_roots,
        "adding a first-party SecurityEvent CEL root requires this coverage test to prove it"
    );

    for condition in conditions {
        assert!(
            crate::net::policy_config::evaluate_security_event_match(condition, &event).unwrap(),
            "{condition} should match"
        );
    }
}

#[test]
fn serializable_security_event_exposes_stable_first_party_wire_shape_without_raw_observations() {
    let mut event = SecurityEvent::new(RuntimeSecurityEventType::FileImport)
        .with_trace_id("trace_wire")
        .with_file(FileSecurityEvent {
            import_path: Some("/workspace/eicar.txt".to_string()),
            import_content: Some(DUMMY_EICAR_TEST_STRING.to_string()),
            ..Default::default()
        })
        .with_credential_observations(vec![CredentialObservation {
            provider: CredentialProvider::OpenAi,
            raw_value: "sk-real-secret".to_string(),
            source: "http.response.body".to_string(),
            event_type: Some("http.response".to_string()),
            confidence: 0.99,
            trace_id: Some("trace_wire".to_string()),
            context_json: None,
        }]);
    event.credential_ref = Some(
        "credential:blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .to_string(),
    );
    event
        .action_trace
        .push(PolicyActionId::CredentialBrokerCapture);
    event.record_detection(SecurityDetectionEvent {
        source: SecurityDetectionSource::Rule,
        detection_level: DetectionLevel::High,
        rule_id: Some("profiles.rules.eicar_block".to_string()),
        plugin_id: None,
        action: Some(SecurityRuleAction::Block),
        plugin_mode: None,
        reason: Some("debug fixture".to_string()),
    });
    event.request_decision(SecurityDecisionKind::Block);

    let wire = event.serializable();
    let json = serde_json::to_value(&wire).expect("serializable wire DTO");

    assert_eq!(json["event_type"], "file.import");
    assert_eq!(json["trace_id"], "trace_wire");
    assert_eq!(json["decision"]["effective"], "block");
    assert_eq!(json["action_trace"][0], "credential_broker.capture");
    assert_eq!(
        json["detections"][0]["rule_id"],
        "profiles.rules.eicar_block"
    );
    assert_eq!(json["file"]["import_path"], "/workspace/eicar.txt");
    for root in ["http", "dns", "mcp", "model", "file", "process"] {
        assert!(json.get(root).is_some(), "{root} must be in the wire DTO");
    }
    for root in ["credential", "snapshot"] {
        assert!(
            json.get(root).is_none(),
            "{root} must not be a fake first-party wire DTO root"
        );
    }
    assert!(
        json.get("credential_observations").is_none(),
        "raw credential observations must not be exposed on the public wire DTO"
    );
    assert!(
        !json.to_string().contains("sk-real-secret"),
        "public wire DTO must not leak raw credential observations"
    );
}

#[test]
fn runtime_security_event_type_roundtrips_and_maps_family() {
    for event_type in RuntimeSecurityEventType::ALL {
        assert_eq!(
            RuntimeSecurityEventType::try_from(event_type.as_str()).unwrap(),
            *event_type
        );
        assert!(
            event_type
                .as_str()
                .starts_with(event_type.family().as_str()),
            "{} must keep its family prefix",
            event_type.as_str()
        );
    }

    assert!(RuntimeSecurityEventType::try_from("mcp.request").is_err());
    assert!(RuntimeSecurityEventType::try_from("dns.response").is_err());
}

#[test]
fn runtime_security_event_families_mark_only_credential_as_ledger_only() {
    use RuntimeSecurityEventFamily::*;

    let cel_roots = crate::net::policy_config::SECURITY_EVENT_CEL_ROOTS
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let families = [Http, Model, Mcp, Dns, File, Process, Credential, Security];

    for family in families {
        assert_eq!(
            family.is_first_party_cel_root(),
            cel_roots.contains(family.as_str()),
            "{} family CEL-root marker must match SECURITY_EVENT_CEL_ROOTS",
            family.as_str()
        );
        assert_eq!(
            family.is_ledger_only(),
            matches!(family, Credential),
            "{} ledger-only marker drifted",
            family.as_str()
        );
    }
}

#[test]
fn runtime_security_event_types_keep_only_credential_ledger_only() {
    for event_type in RuntimeSecurityEventType::ALL {
        assert_eq!(
            event_type.uses_ledger_only_family(),
            matches!(event_type, RuntimeSecurityEventType::CredentialSubstitution),
            "{} ledger-only classification drifted",
            event_type.as_str()
        );
    }
}

#[test]
fn runtime_security_event_from_logger_write_maps_all_write_ops() {
    let credential_ref =
        "credential:blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let cases = vec![
        (
            net_write(Some(credential_ref)),
            RuntimeSecurityEventType::HttpRequest,
        ),
        (
            model_write(Some(credential_ref)),
            RuntimeSecurityEventType::ModelCall,
        ),
        (
            mcp_write("tools/call", Some(credential_ref)),
            RuntimeSecurityEventType::McpToolCall,
        ),
        (
            mcp_write("tools/list", Some(credential_ref)),
            RuntimeSecurityEventType::McpToolList,
        ),
        (
            mcp_write("resources/read", Some(credential_ref)),
            RuntimeSecurityEventType::McpEvent,
        ),
        (
            file_write(Some(credential_ref)),
            RuntimeSecurityEventType::FileEvent,
        ),
        (
            file_write_with_action(FileAction::Imported, Some(credential_ref)),
            RuntimeSecurityEventType::FileImport,
        ),
        (
            file_write_with_action(FileAction::Exported, Some(credential_ref)),
            RuntimeSecurityEventType::FileExport,
        ),
        (
            exec_write(Some(credential_ref)),
            RuntimeSecurityEventType::ProcessExec,
        ),
        (
            exec_complete_write(),
            RuntimeSecurityEventType::ProcessExecComplete,
        ),
        (
            audit_write(Some(credential_ref)),
            RuntimeSecurityEventType::ProcessAudit,
        ),
        (
            dns_write(Some(credential_ref)),
            RuntimeSecurityEventType::DnsQuery,
        ),
        (
            substitution_write(credential_ref),
            RuntimeSecurityEventType::CredentialSubstitution,
        ),
    ];

    for (write, expected_type) in cases {
        let event = RuntimeSecurityEvent::from_logger_write(write);
        assert_eq!(event.event_type, expected_type);
        assert_eq!(event.event_family, expected_type.family());
        if expected_type != RuntimeSecurityEventType::ProcessExecComplete {
            assert_eq!(event.credential_ref.as_deref(), Some(credential_ref));
        }
    }
}

#[tokio::test]
async fn emit_security_write_is_the_db_handoff_for_runtime_events() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();

    let event_id = emit_security_write(&writer, file_write(None))
        .await
        .expect("primary runtime events receive a joinable event id");
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let persisted_event_id: String = conn
        .query_row("SELECT event_id FROM fs_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(persisted_event_id, event_id.as_str());
}

#[tokio::test]
async fn emit_security_write_records_canonical_emit_metrics() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();

    emit_security_write(&writer, file_write(None))
        .await
        .expect("primary runtime events receive a joinable event id");
    writer.shutdown_blocking();

    let snapshot = snapshotter.snapshot().into_vec();
    let counter = snapshot.iter().find_map(|(key, _, _, value)| {
        let labels = key.key().labels().collect::<Vec<_>>();
        let has_label = |name: &str, want: &str| {
            labels
                .iter()
                .any(|label| label.key() == name && label.value() == want)
        };
        match (key.key().name(), value) {
            (SECURITY_EVENT_EMIT_TOTAL, DebugValue::Counter(count))
                if has_label("event_type", RuntimeSecurityEventType::FileEvent.as_str())
                    && has_label("event_family", RuntimeSecurityEventFamily::File.as_str())
                    && has_label("status", "ok")
                    && has_label("queue_result", "queued") =>
            {
                Some(*count)
            }
            _ => None,
        }
    });
    assert_eq!(counter, Some(1));

    let histogram_present = snapshot.iter().any(|(key, _, _, value)| {
        let labels = key.key().labels().collect::<Vec<_>>();
        key.key().name() == SECURITY_EVENT_EMIT_DURATION_MS
            && labels.iter().any(|label| {
                label.key() == "event_type"
                    && label.value() == RuntimeSecurityEventType::FileEvent.as_str()
            })
            && labels.iter().any(|label| {
                label.key() == "event_family"
                    && label.value() == RuntimeSecurityEventFamily::File.as_str()
            })
            && matches!(value, DebugValue::Histogram(_))
    });
    assert!(histogram_present);
}

#[test]
fn emit_security_write_blocking_is_the_sync_db_handoff() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 1).unwrap();

    let event_id = emit_security_write_blocking(&writer, file_write(None))
        .expect("primary runtime events receive a joinable event id");
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let persisted_event_id: String = conn
        .query_row("SELECT event_id FROM fs_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(persisted_event_id, event_id.as_str());
}

#[test]
fn security_event_id_is_twelve_lower_hex() {
    let generated = SecurityEventId::new_uuid4();
    assert_eq!(generated.as_str().len(), 12);
    assert!(generated
        .as_str()
        .chars()
        .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()));

    assert_eq!(
        SecurityEventId::parse("abcdef123456").unwrap().as_str(),
        "abcdef123456"
    );
    assert!(SecurityEventId::parse("ABCDEF123456").is_err());
    assert!(SecurityEventId::parse("evt_abc123").is_err());
    assert!(SecurityEventId::parse("abcdef12345").is_err());
}

#[tokio::test]
async fn emit_security_rule_match_writes_forensic_ledger_row() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.block_openai]
name = "openai_api_block"
action = "block"
detection_level = "critical"
match = 'http.host.matches("(^|.*\.)openai\.com$")'
priority = 10
reason = "corp block"
"#,
    )
    .unwrap();
    let rule_set = SecurityRuleProfile::compile(&profile, SecurityRuleSource::User).unwrap();
    let rule = rule_set
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.block_openai")
        .unwrap();
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_trace_id("trace_deadbeef")
        .with_http(HttpSecurityEvent {
            host: Some("api.openai.com".into()),
            method: Some("POST".into()),
            path: Some("/v1/chat/completions".into()),
            status: None,
            body: Some("{\"model\":\"gpt-4.1\"}".into()),
        })
        .with_credential_observations(vec![CredentialObservation {
            provider: CredentialProvider::OpenAi,
            raw_value: "sk-live-should-not-appear".into(),
            source: "http.request.header.authorization".into(),
            event_type: Some("http.request".into()),
            confidence: 1.0,
            trace_id: Some("trace_deadbeef".into()),
            context_json: None,
        }]);

    emit_security_rule_match(
        &writer,
        SecurityEventId::parse("abcdef123456").unwrap(),
        RuntimeSecurityEventType::HttpRequest,
        rule,
        &event,
        1_789_000_000_000,
    )
    .await
    .unwrap();
    writer.shutdown_blocking();

    let reader = capsem_logger::DbReader::open(&db_path).unwrap();
    let rows = reader.recent_security_rule_events(10).unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.event_id, "abcdef123456");
    assert_eq!(row.event_type, "http.request");
    assert_eq!(row.rule_id, "profiles.rules.block_openai");
    assert_eq!(row.rule_action, capsem_logger::SecurityRuleAction::Block);
    assert_eq!(
        row.detection_level,
        capsem_logger::SecurityDetectionLevel::Critical
    );
    assert!(row.rule_json.contains("openai_api_block"));
    assert!(row.event_json.contains("api.openai.com"));
    assert!(row.event_json.contains("credential:blake3:"));
    assert!(
        !row.event_json.contains("sk-live-should-not-appear"),
        "forensic event payload must not store raw credential observations"
    );
}

#[test]
fn security_rule_trace_labels_are_low_cardinality_rule_fields() {
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.block_openai]
name = "openai_api_block"
action = "block"
detection_level = "critical"
match = 'http.host == "api.openai.com"'
"#,
    )
    .unwrap();
    let rules = SecurityRuleProfile::compile(&profile, SecurityRuleSource::User).unwrap();
    let rule = rules
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.block_openai")
        .unwrap();

    let labels = SecurityRuleTraceLabels::from_rule(rule);

    assert_eq!(labels.rule_id, "profiles.rules.block_openai");
    assert_eq!(labels.rule_name, "openai_api_block");
    assert_eq!(labels.rule_action, "block");
    assert_eq!(labels.rule_detection_level, "critical");
    assert_eq!(labels.provider, "profiles");
}

#[tokio::test]
async fn primary_event_and_rule_ledger_share_event_id() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.file_skill_loaded]
name = "file_skill_loaded"
action = "allow"
detection_level = "informational"
match = 'file.read.path.contains("skills/") && file.read.name.endsWith(".md")'
"#,
    )
    .unwrap();
    let rule_set = SecurityRuleProfile::compile(&profile, SecurityRuleSource::User).unwrap();
    let rule = rule_set
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.file_skill_loaded")
        .unwrap();

    let event_id = emit_security_write(&writer, file_write(None))
        .await
        .expect("file event must receive a primary event id");
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_trace_id("trace_file_skill")
        .with_file(FileSecurityEvent {
            read_path: Some("/root/.codex/skills/example/SKILL.md".into()),
            read_name: Some("SKILL.md".into()),
            read_ext: Some("md".into()),
            read_mime_type: Some("text/markdown".into()),
            read_content: Some("# skill".into()),
            ..Default::default()
        });

    emit_security_rule_match(
        &writer,
        event_id.clone(),
        RuntimeSecurityEventType::FileEvent,
        rule,
        &event,
        1_789_000_000_100,
    )
    .await
    .unwrap();
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let fs_event_id: String = conn
        .query_row("SELECT event_id FROM fs_events", [], |row| row.get(0))
        .unwrap();
    let rule_event_id: String = conn
        .query_row("SELECT event_id FROM security_rule_events", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(fs_event_id, event_id.as_str());
    assert_eq!(rule_event_id, event_id.as_str());
}

#[tokio::test]
async fn emit_matching_security_rules_writes_all_matches_with_primary_event_id() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.http_observed]
name = "http_observed"
action = "allow"
detection_level = "informational"
match = 'http.host.contains("openai.com")'

[profiles.rules.http_block]
name = "http_block"
action = "block"
detection_level = "critical"
match = 'http.path.startsWith("/v1/")'
"#,
    )
    .unwrap();
    let rules = crate::net::policy_config::SecurityRuleSet::compile_profile(
        &profile,
        SecurityRuleSource::User,
    )
    .unwrap();

    let event_id = emit_security_write(&writer, net_write(None))
        .await
        .expect("primary HTTP event must receive an id");
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_trace_id("trace_http_rules")
        .with_http(HttpSecurityEvent {
            host: Some("api.openai.com".into()),
            method: Some("POST".into()),
            path: Some("/v1/responses".into()),
            status: Some("200".into()),
            body: None,
        });

    let emitted = emit_matching_security_rules(
        &writer,
        event_id.clone(),
        RuntimeSecurityEventType::HttpRequest,
        &rules,
        &event,
        1_789_000_000_200,
    )
    .await
    .unwrap();
    writer.shutdown_blocking();

    assert_eq!(emitted, 2);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let net_event_id: String = conn
        .query_row("SELECT event_id FROM net_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(net_event_id, event_id.as_str());
    let rows: Vec<(String, String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT event_id, rule_id, detection_level
                 FROM security_rule_events ORDER BY rule_id",
            )
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert_eq!(
        rows,
        vec![
            (
                event_id.as_str().to_string(),
                "profiles.rules.http_block".to_string(),
                "critical".to_string()
            ),
            (
                event_id.as_str().to_string(),
                "profiles.rules.http_observed".to_string(),
                "informational".to_string()
            ),
        ]
    );
}

#[tokio::test]
async fn emit_matching_security_rules_with_decision_uses_same_evaluation_as_ledger() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[corp.rules.block_openai]
name = "block_openai"
action = "block"
priority = -10
reason = "corp block"
match = 'http.host == "api.openai.com"'

[profiles.rules.detect_openai]
name = "detect_openai"
action = "allow"
detection_level = "high"
priority = 10
match = 'http.host == "api.openai.com"'

[profiles.rules.ask_model]
name = "ask_model"
action = "ask"
priority = 20
match = 'model.provider == "openai"'
"#,
    )
    .unwrap();
    let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap();
    let event_id = emit_security_write(&writer, net_write(None))
        .await
        .expect("primary HTTP event must receive an id");
    let event =
        SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
            host: Some("api.openai.com".into()),
            method: Some("POST".into()),
            path: Some("/v1/responses".into()),
            ..Default::default()
        });

    let emission = emit_matching_security_rules_with_decision(
        &writer,
        event_id.clone(),
        RuntimeSecurityEventType::HttpRequest,
        &rules,
        &event,
        1_789_000_000_250,
    )
    .await
    .unwrap();
    writer.shutdown_blocking();

    assert_eq!(emission.emitted, 2);
    assert_eq!(
        emission.enforcement.action,
        SecurityEnforcementAction::Block
    );
    assert_eq!(
        emission.enforcement.rule_id.as_deref(),
        Some("corp.rules.block_openai")
    );
    assert_eq!(emission.enforcement.reason.as_deref(), Some("corp block"));

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let rows: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare("SELECT rule_id, rule_action FROM security_rule_events ORDER BY rule_id")
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert_eq!(
        rows,
        vec![
            ("corp.rules.block_openai".to_string(), "block".to_string()),
            (
                "profiles.rules.detect_openai".to_string(),
                "allow".to_string()
            ),
        ],
        "the decision must be derived from the same matches that were ledgered"
    );

    let decision_rows: Vec<(String, String, String, String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT actor, previous_decision, requested_decision, effective_decision, rule_id
                 FROM security_decision_events
                 ORDER BY id",
            )
            .unwrap();
        stmt.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
    };
    assert_eq!(
        decision_rows,
        vec![
            (
                "corp.rules.block_openai".to_string(),
                "allow".to_string(),
                "block".to_string(),
                "block".to_string(),
                "corp.rules.block_openai".to_string(),
            ),
            (
                "profiles.rules.detect_openai".to_string(),
                "block".to_string(),
                "allow".to_string(),
                "block".to_string(),
                "profiles.rules.detect_openai".to_string(),
            ),
        ],
        "the table must show the allow rule could not downgrade the existing block"
    );
}

#[tokio::test]
async fn emit_matching_security_rules_with_decision_defaults_to_allow_without_enforcement_match() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let rules = security_rule_set(
        r#"
[profiles.rules.detect_skill]
name = "detect_skill"
action = "postprocess"
detection_level = "informational"
match = 'file.read.name == "SKILL.md"'
"#,
    );
    let event_id = emit_security_write(&writer, file_write(None))
        .await
        .expect("primary file event must receive an id");
    let event =
        SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_file(FileSecurityEvent {
            read_name: Some("SKILL.md".into()),
            ..Default::default()
        });

    let emission = emit_matching_security_rules_with_decision(
        &writer,
        event_id,
        RuntimeSecurityEventType::FileEvent,
        &rules,
        &event,
        1_789_000_000_260,
    )
    .await
    .unwrap();
    writer.shutdown_blocking();

    assert_eq!(emission.emitted, 1);
    assert_eq!(emission.enforcement, SecurityEnforcementDecision::allow());
}

#[tokio::test]
async fn ask_enforcement_writes_pending_and_resolution_controls_materialization() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let rules = security_rule_set(
        r#"
[profiles.rules.ask_openai]
name = "ask_openai"
action = "ask"
reason = "manual approval required"
match = 'http.host == "api.openai.com"'
"#,
    );
    let event_id = emit_security_write(&writer, net_write(None))
        .await
        .expect("primary HTTP event must receive an id");
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_trace_id("trace_ask")
        .with_http(HttpSecurityEvent {
            host: Some("api.openai.com".into()),
            method: Some("POST".into()),
            path: Some("/v1/responses".into()),
            ..Default::default()
        })
        .with_http_request(HttpRequestSecurityEvent::new(
            "api.openai.com",
            Some(ProviderKind::OpenAi),
            http::HeaderMap::new(),
            None,
        ));

    let emission = emit_matching_security_rules_with_decision(
        &writer,
        event_id.clone(),
        RuntimeSecurityEventType::HttpRequest,
        &rules,
        &event,
        1_789_000_000_270,
    )
    .await
    .unwrap();

    assert_eq!(emission.emitted, 1);
    assert_eq!(emission.enforcement.action, SecurityEnforcementAction::Ask);
    let ask_id = emission
        .enforcement
        .ask_id
        .clone()
        .expect("ask decision must return ask_id");
    let ask_rule = rules
        .rules()
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.ask_openai")
        .expect("ask rule must compile");
    let pending = security_ask_pending_event(
        ask_id.clone(),
        event_id.clone(),
        RuntimeSecurityEventType::HttpRequest,
        ask_rule,
        &event,
        1_789_000_000_270,
    )
    .unwrap();
    let unresolved = emission.enforcement.with_ask_resolution(&pending);
    assert!(unresolved
        .unwrap_err()
        .to_string()
        .contains("still pending"));
    let pending_error =
        materialize_http_request_for_upstream_after_enforcement(&event, &emission.enforcement)
            .expect_err("pending ask must block materialization");
    assert!(pending_error.to_string().contains("ask"));

    emit_security_ask_resolution(
        &writer,
        &pending,
        capsem_logger::SecurityAskStatus::Approved,
        "tester",
        Some("approved for test".to_string()),
        1_789_000_000_280,
    )
    .await
    .unwrap();
    writer.shutdown_blocking();

    let reader = capsem_logger::DbReader::open(&db_path).unwrap();
    let ask_rows = reader.recent_security_ask_events(10).unwrap();
    assert_eq!(ask_rows.len(), 2);
    let latest = reader
        .latest_security_ask_event(ask_id.as_str())
        .unwrap()
        .expect("resolution row must exist");
    assert_eq!(latest.status, capsem_logger::SecurityAskStatus::Approved);
    assert_eq!(latest.resolver.as_deref(), Some("tester"));
    assert_eq!(latest.event_id, event_id.as_str());
    assert_eq!(latest.rule_id, "profiles.rules.ask_openai");

    let approved = emission.enforcement.with_ask_resolution(&latest).unwrap();
    assert_eq!(approved.action, SecurityEnforcementAction::Allow);
    materialize_http_request_for_upstream_after_enforcement(&event, &approved)
        .expect("approved ask should materialize like allow");

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let ledger_rule_id: String = conn
        .query_row("SELECT rule_id FROM security_rule_events", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(ledger_rule_id, "profiles.rules.ask_openai");
}

#[tokio::test]
async fn session_db_regenerates_rule_enforcement_detection_and_ask_story() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let github_rules = security_rule_set(
        r#"
[corp.rules.github_block]
name = "github_block"
action = "block"
detection_level = "critical"
priority = -10
reason = "corp block"
match = 'http.host == "github.com"'

[profiles.rules.github_detect]
name = "github_detect"
action = "allow"
detection_level = "high"
match = 'http.host == "github.com"'

[profiles.rules.github_postprocess]
name = "github_postprocess"
action = "postprocess"
detection_level = "informational"
match = 'http.host == "github.com"'
"#,
    );
    let github_event_id = emit_security_write(&writer, net_write(None))
        .await
        .expect("primary HTTP event must receive an id");
    let github_event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_trace_id("trace_github")
        .with_http(HttpSecurityEvent {
            host: Some("github.com".into()),
            method: Some("GET".into()),
            path: Some("/settings/tokens".into()),
            ..Default::default()
        });

    let github_emission = emit_matching_security_rules_with_decision(
        &writer,
        github_event_id.clone(),
        RuntimeSecurityEventType::HttpRequest,
        &github_rules,
        &github_event,
        1_789_000_000_310,
    )
    .await
    .unwrap();
    assert_eq!(github_emission.emitted, 3);
    assert_eq!(
        github_emission.enforcement.action,
        SecurityEnforcementAction::Block
    );
    assert_eq!(
        github_emission.enforcement.rule_id.as_deref(),
        Some("corp.rules.github_block")
    );

    let ask_rules = security_rule_set(
        r#"
[profiles.rules.ask_openai]
name = "ask_openai"
action = "ask"
reason = "manual approval required"
match = 'http.host == "api.openai.com"'
"#,
    );
    let ask_event_id = emit_security_write(&writer, net_write(None))
        .await
        .expect("primary HTTP event must receive an id");
    let ask_event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_trace_id("trace_openai_ask")
        .with_http(HttpSecurityEvent {
            host: Some("api.openai.com".into()),
            method: Some("POST".into()),
            path: Some("/v1/responses".into()),
            ..Default::default()
        });

    let ask_emission = emit_matching_security_rules_with_decision(
        &writer,
        ask_event_id.clone(),
        RuntimeSecurityEventType::HttpRequest,
        &ask_rules,
        &ask_event,
        1_789_000_000_320,
    )
    .await
    .unwrap();
    let ask_id = ask_emission
        .enforcement
        .ask_id
        .clone()
        .expect("ask decision must return ask_id");
    let ask_rule = ask_rules
        .rules()
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.ask_openai")
        .expect("ask rule must compile");
    let pending = security_ask_pending_event(
        ask_id.clone(),
        ask_event_id.clone(),
        RuntimeSecurityEventType::HttpRequest,
        ask_rule,
        &ask_event,
        1_789_000_000_320,
    )
    .unwrap();
    emit_security_ask_resolution(
        &writer,
        &pending,
        capsem_logger::SecurityAskStatus::Denied,
        "tester",
        Some("denied for test".to_string()),
        1_789_000_000_330,
    )
    .await
    .unwrap();
    writer.shutdown_blocking();

    let reader = capsem_logger::DbReader::open(&db_path).unwrap();
    let rows = reader.recent_security_rule_events(10).unwrap();
    assert_eq!(rows.len(), 4);

    let postprocess_row = rows
        .iter()
        .find(|row| row.rule_id == "profiles.rules.github_postprocess")
        .expect("postprocess detection rule row must be present");
    assert_eq!(postprocess_row.event_id, github_event_id.as_str());
    assert_eq!(postprocess_row.event_type, "http.request");
    assert_eq!(
        postprocess_row.rule_action,
        capsem_logger::SecurityRuleAction::Postprocess
    );
    assert_eq!(
        postprocess_row.detection_level,
        capsem_logger::SecurityDetectionLevel::Informational
    );
    let postprocess_rule: serde_json::Value =
        serde_json::from_str(&postprocess_row.rule_json).unwrap();
    assert_eq!(postprocess_rule["provider"], "profiles");
    assert_eq!(postprocess_rule["rule_action"], "postprocess");
    assert_eq!(postprocess_rule["detection_level"], "informational");
    assert!(postprocess_rule.get("plugin").is_none());
    let postprocess_event: serde_json::Value =
        serde_json::from_str(&postprocess_row.event_json).unwrap();
    assert_eq!(postprocess_event["event_type"], "http.request");
    assert_eq!(postprocess_event["http"]["host"], "github.com");

    let block_row = rows
        .iter()
        .find(|row| row.rule_id == "corp.rules.github_block")
        .expect("enforcement block row must be present");
    assert_eq!(
        block_row.rule_action,
        capsem_logger::SecurityRuleAction::Block
    );
    assert_eq!(
        block_row.detection_level,
        capsem_logger::SecurityDetectionLevel::Critical
    );
    let block_rule: serde_json::Value = serde_json::from_str(&block_row.rule_json).unwrap();
    assert_eq!(block_rule["reason"], "corp block");
    assert_eq!(block_rule["priority"], -10);

    let detect_row = rows
        .iter()
        .find(|row| row.rule_id == "profiles.rules.github_detect")
        .expect("detection row must be present");
    assert_eq!(
        detect_row.detection_level,
        capsem_logger::SecurityDetectionLevel::High
    );

    let ask_rows = reader.recent_security_ask_events(10).unwrap();
    assert_eq!(ask_rows.len(), 2);
    assert_eq!(ask_rows[0].status, capsem_logger::SecurityAskStatus::Denied);
    assert_eq!(ask_rows[0].ask_id, ask_id.as_str());
    assert_eq!(ask_rows[0].event_id, ask_event_id.as_str());
    assert_eq!(ask_rows[0].rule_id, "profiles.rules.ask_openai");
    assert_eq!(ask_rows[0].resolver.as_deref(), Some("tester"));
    assert_eq!(
        ask_rows[1].status,
        capsem_logger::SecurityAskStatus::Pending
    );

    let stats = reader.security_rule_stats().unwrap();
    assert_eq!(stats.total, 4);
    assert!(stats
        .by_action
        .iter()
        .any(|entry| entry.rule_action == "block" && entry.count == 1));
    assert!(stats
        .by_action
        .iter()
        .any(|entry| entry.rule_action == "postprocess" && entry.count == 1));
    assert!(stats
        .by_rule
        .iter()
        .any(|entry| entry.rule_id == "profiles.rules.github_postprocess"
            && entry.detection_level == "informational"
            && entry.latest_event_id == github_event_id.as_str()));
}

#[test]
fn denied_ask_resolution_blocks_like_block() {
    let decision = SecurityEnforcementDecision {
        action: SecurityEnforcementAction::Ask,
        rule_id: Some("profiles.rules.ask_openai".to_string()),
        rule_name: Some("ask_openai".to_string()),
        reason: None,
        ask_id: Some(SecurityEventId::parse("abcdef123456").unwrap()),
    };
    let denied = capsem_logger::SecurityAskEvent::pending(capsem_logger::SecurityAskPending {
        timestamp_unix_ms: 1_789_000_000_290,
        ask_id: "abcdef123456".to_string(),
        event_id: "aaaaaa111111".to_string(),
        event_type: RuntimeSecurityEventType::HttpRequest.as_str().to_string(),
        rule_id: "profiles.rules.ask_openai".to_string(),
        rule_name: "ask_openai".to_string(),
        rule_json: "{}".to_string(),
        event_json: "{}".to_string(),
    })
    .with_status(capsem_logger::SecurityAskStatus::Denied)
    .with_resolver("tester")
    .with_reason("denied for test");
    let resolved = decision.with_ask_resolution(&denied).unwrap();
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http_request(
        HttpRequestSecurityEvent::new(
            "api.openai.com",
            Some(ProviderKind::OpenAi),
            http::HeaderMap::new(),
            None,
        ),
    );

    assert_eq!(resolved.action, SecurityEnforcementAction::Block);
    assert_eq!(resolved.reason.as_deref(), Some("denied for test"));
    let error = materialize_http_request_for_upstream_after_enforcement(&event, &resolved)
        .expect_err("denied ask must block materialization");
    assert!(error.to_string().contains("profiles.rules.ask_openai"));
}

#[tokio::test]
async fn emit_file_security_write_and_rules_maps_created_file_to_create_root() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.file_create_seen]
name = "file_create_seen"
action = "allow"
detection_level = "informational"
match = 'file.create.path == "/workspace/skills/foo.md" && file.create.name == "foo.md" && file.create.ext == "md"'
"#,
    )
    .unwrap();
    let rules = crate::net::policy_config::SecurityRuleSet::compile_profile(
        &profile,
        SecurityRuleSource::User,
    )
    .unwrap();

    let event_id = emit_file_security_write_and_rules(
        &writer,
        &rules,
        FileEvent {
            event_id: None,
            timestamp: SystemTime::now(),
            action: FileAction::Created,
            path: "/workspace/skills/foo.md".to_string(),
            size: Some(12),
            trace_id: Some("trace_file_create".to_string()),
            credential_ref: None,
        },
    )
    .await
    .expect("file event must receive id");
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let fs_event_id: String = conn
        .query_row("SELECT event_id FROM fs_events", [], |row| row.get(0))
        .unwrap();
    let rule_row: (String, String) = conn
        .query_row(
            "SELECT event_id, rule_id FROM security_rule_events",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(fs_event_id, event_id.as_str());
    assert_eq!(rule_row.0, event_id.as_str());
    assert_eq!(rule_row.1, "profiles.rules.file_create_seen");
}

#[tokio::test]
async fn emit_explicit_file_security_events_map_import_export_and_read_roots() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 32).unwrap();
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.file_import_seen]
name = "file_import_seen"
action = "allow"
detection_level = "informational"
match = 'file.import.path.endsWith("input.txt") && file.import.mime_type == "text/plain" && file.import.content.contains("incoming")'

[profiles.rules.file_export_seen]
name = "file_export_seen"
action = "allow"
detection_level = "informational"
match = 'file.export.name == "output.json" && file.export.ext == "json" && file.export.content.contains("ok")'

[profiles.rules.file_read_seen]
name = "file_read_seen"
action = "allow"
detection_level = "informational"
match = 'file.read.path.contains("skills/") && file.read.ext == "md" && file.read.content.contains("Development Sprint")'
"#,
    )
    .unwrap();
    let rules = crate::net::policy_config::SecurityRuleSet::compile_profile(
        &profile,
        SecurityRuleSource::User,
    )
    .unwrap();

    for event in [
        ExplicitFileSecurityEvent {
            action: FileAction::Imported,
            path: "/workspace/input.txt".to_string(),
            size: Some(8),
            content: Some("incoming".to_string()),
            mime_type: Some("text/plain".to_string()),
            trace_id: Some("trace_file_import".to_string()),
            credential_ref: None,
        },
        ExplicitFileSecurityEvent {
            action: FileAction::Exported,
            path: "/workspace/output.json".to_string(),
            size: Some(11),
            content: Some(r#"{"ok":true}"#.to_string()),
            mime_type: Some("application/json".to_string()),
            trace_id: Some("trace_file_export".to_string()),
            credential_ref: None,
        },
        ExplicitFileSecurityEvent {
            action: FileAction::Read,
            path: "/workspace/skills/skill.md".to_string(),
            size: Some(20),
            content: Some("Development Sprint".to_string()),
            mime_type: Some("text/markdown".to_string()),
            trace_id: Some("trace_file_read".to_string()),
            credential_ref: None,
        },
    ] {
        emit_explicit_file_security_write_and_rules(&writer, &rules, event)
            .await
            .expect("explicit file event must receive id");
    }
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let actions = conn
        .prepare("SELECT action FROM fs_events ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(actions, vec!["import", "export", "read"]);

    let rules = conn
        .prepare("SELECT rule_id, event_type, event_json FROM security_rule_events ORDER BY id")
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        rules.iter().map(|row| row.0.as_str()).collect::<Vec<_>>(),
        vec![
            "profiles.rules.file_import_seen",
            "profiles.rules.file_export_seen",
            "profiles.rules.file_read_seen",
        ]
    );
    assert_eq!(rules[0].1, "file.import");
    assert_eq!(rules[1].1, "file.export");
    assert_eq!(rules[2].1, "file.event");
    assert!(rules[0].2.contains(r#""import_content":"incoming""#));
    assert!(rules[1]
        .2
        .contains(r#""export_mime_type":"application/json""#));
    assert!(rules[2]
        .2
        .contains(r#""read_content":"Development Sprint""#));
}

#[tokio::test]
async fn emit_process_exec_and_complete_rules_share_exec_event_id() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.process_exec_seen]
name = "process_exec_seen"
action = "allow"
detection_level = "informational"
match = 'process.command.contains("python")'

[profiles.rules.process_complete_seen]
name = "process_complete_seen"
action = "allow"
detection_level = "low"
match = 'process.exec.id == "42" && process.exec.exit_code == "0" && process.exec.stdout.contains("ok")'
"#,
    )
    .unwrap();
    let rules = crate::net::policy_config::SecurityRuleSet::compile_profile(
        &profile,
        SecurityRuleSource::User,
    )
    .unwrap();

    let event_id = emit_process_exec_security_write_and_rules(
        &writer,
        &rules,
        ExecEvent {
            event_id: None,
            timestamp: SystemTime::now(),
            exec_id: 42,
            command: "python main.py".to_string(),
            source: "api".to_string(),
            mcp_call_id: None,
            trace_id: Some("trace_exec".to_string()),
            process_name: None,
            credential_ref: None,
        },
    )
    .await
    .expect("exec event must receive id");
    emit_process_complete_security_write_and_rules(
        &writer,
        &rules,
        event_id.clone(),
        ExecEventComplete {
            exec_id: 42,
            exit_code: 0,
            duration_ms: 12,
            stdout_preview: Some("ok".to_string()),
            stderr_preview: None,
            stdout_bytes: 2,
            stderr_bytes: 0,
            pid: Some(1000),
        },
    )
    .await
    .expect("exec complete must reuse primary id");
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let exec_event_id: String = conn
        .query_row(
            "SELECT event_id FROM exec_events WHERE exec_id = 42",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(exec_event_id, event_id.as_str());
    let rows: Vec<(String, String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT event_id, event_type, rule_id
                 FROM security_rule_events ORDER BY rule_id",
            )
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert_eq!(
        rows,
        vec![
            (
                event_id.as_str().to_string(),
                "process.exec_complete".to_string(),
                "profiles.rules.process_complete_seen".to_string()
            ),
            (
                event_id.as_str().to_string(),
                "process.exec".to_string(),
                "profiles.rules.process_exec_seen".to_string()
            ),
        ]
    );
}

#[tokio::test]
async fn emit_substitution_security_write_and_rules_keeps_ref_without_fake_root() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let rules = SecurityRuleSet::new(Vec::new());
    let credential_ref = capsem_logger::credential_reference("openai", "sk-test-secret");

    let event_id = emit_substitution_security_write_and_rules(
        &writer,
        &rules,
        SubstitutionEvent {
            event_id: None,
            timestamp: SystemTime::now(),
            material_class: "credential".to_string(),
            source: "http.response".to_string(),
            event_type: Some("http.request".to_string()),
            algorithm: "blake3".to_string(),
            substitution_ref: credential_ref.clone(),
            outcome: "captured".to_string(),
            provider: Some("openai".to_string()),
            confidence: Some(1.0),
            trace_id: Some("trace_credential".to_string()),
            context_json: None,
        },
    )
    .await
    .expect("substitution event must receive id");
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let substitution_event_id: String = conn
        .query_row("SELECT event_id FROM substitution_events", [], |row| {
            row.get(0)
        })
        .unwrap();
    let persisted_ref: String = conn
        .query_row(
            "SELECT substitution_ref FROM substitution_events",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let rule_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM security_rule_events", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(substitution_event_id, event_id.as_str());
    assert_eq!(persisted_ref, credential_ref);
    assert_eq!(rule_count, 0);
}

#[tokio::test]
async fn emit_matching_security_rules_writes_no_rows_for_non_match() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.http_block]
name = "http_block"
action = "block"
match = 'http.host.contains("openai.com")'
"#,
    )
    .unwrap();
    let rules = crate::net::policy_config::SecurityRuleSet::compile_profile(
        &profile,
        SecurityRuleSource::User,
    )
    .unwrap();
    let event_id = emit_security_write(&writer, net_write(None))
        .await
        .expect("primary HTTP event must receive an id");
    let event =
        SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
            host: Some("example.com".into()),
            ..Default::default()
        });

    let emitted = emit_matching_security_rules(
        &writer,
        event_id,
        RuntimeSecurityEventType::HttpRequest,
        &rules,
        &event,
        1_789_000_000_300,
    )
    .await
    .unwrap();
    writer.shutdown_blocking();

    assert_eq!(emitted, 0);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM security_rule_events", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

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
        message_id: None,
        status_code: Some(200),
        text_content: None,
        thinking_content: None,
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
        mcp_call_id: None,
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
        confidence: Some(1.0),
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
    let store_guard = EnvVarGuard::set(crate::credential_broker::TEST_STORE_ENV, &store_path);
    let user_config_guard = EnvVarGuard::set("CAPSEM_HOME", tmp.path());
    let raw = "sk-ant-materialize-secret";
    let brokered = broker_observed_credential(&CredentialObservation {
        provider: CredentialProvider::Anthropic,
        raw_value: raw.to_string(),
        source: "http.request.headers.authorization".to_string(),
        event_type: Some("http.request".to_string()),
        confidence: 1.0,
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

#[test]
fn http_materializer_without_substitute_action_keeps_reference() {
    let (event, reference, _raw, _tmp, _store_guard, _user_config_guard, _lock) =
        brokered_anthropic_header_event();

    let materialized = materialize_http_request_for_upstream(&event).unwrap();

    assert_eq!(
        materialized
            .headers
            .get(http::header::AUTHORIZATION)
            .unwrap(),
        &http::HeaderValue::from_str(&reference).unwrap(),
        "without a matched substitute action, materialization must stay reference-only"
    );
    assert_eq!(materialized.credential_ref, None);
}

#[test]
fn http_materializer_requires_allow_enforcement_decision() {
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http_request(
        HttpRequestSecurityEvent::new(
            "api.openai.com",
            Some(ProviderKind::OpenAi),
            http::HeaderMap::new(),
            None,
        ),
    );
    let block = SecurityEnforcementDecision {
        action: SecurityEnforcementAction::Block,
        rule_id: Some("corp.rules.block_openai".to_string()),
        rule_name: Some("block_openai".to_string()),
        reason: Some("blocked".to_string()),
        ask_id: None,
    };
    let ask = SecurityEnforcementDecision {
        action: SecurityEnforcementAction::Ask,
        rule_id: Some("profiles.rules.ask_openai".to_string()),
        rule_name: Some("ask_openai".to_string()),
        reason: None,
        ask_id: Some(SecurityEventId::parse("abcdef123456").unwrap()),
    };

    let block_error = materialize_http_request_for_upstream_after_enforcement(&event, &block)
        .expect_err("block decision must not materialize");
    assert!(
        block_error.to_string().contains("corp.rules.block_openai"),
        "{block_error}"
    );
    let ask_error = materialize_http_request_for_upstream_after_enforcement(&event, &ask)
        .expect_err("ask decision must wait for resolution before materialization");
    assert!(
        ask_error.to_string().contains("profiles.rules.ask_openai"),
        "{ask_error}"
    );
}

#[test]
fn http_materializer_resolves_broker_ref_only_for_upstream_copy() {
    let (mut event, reference, raw, _tmp, _store_guard, _user_config_guard, _lock) =
        brokered_anthropic_header_event();
    event
        .action_trace
        .push(PolicyActionId::CredentialBrokerSubstitute);

    let materialized = materialize_http_request_for_upstream(&event).unwrap();

    assert_eq!(
        event
            .http_request
            .as_ref()
            .unwrap()
            .headers
            .get(http::header::AUTHORIZATION)
            .unwrap(),
        &http::HeaderValue::from_str(&reference).unwrap(),
        "the auditable security event must remain reference-only"
    );
    assert_eq!(
        materialized
            .headers
            .get(http::header::AUTHORIZATION)
            .unwrap(),
        &http::HeaderValue::from_str(&raw).unwrap(),
        "only the upstream materialized copy receives the raw credential"
    );
    assert_eq!(
        materialized.credential_ref.as_deref(),
        Some(reference.as_str())
    );
}
