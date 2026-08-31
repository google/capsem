use super::*;

#[test]
fn security_event_emitter_is_the_auditable_event_boundary() {
    let emitter = RecordingEmitter::new();
    let mut event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest);
    event.credential_ref =
        Some("credential:blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string());

    emitter.emit(event.clone()).unwrap();

    assert_eq!(emitter.events.lock().unwrap().as_slice(), [event]);
}

#[test]
fn security_event_engine_runs_enabled_plugins_by_stage() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

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
            stage: SecurityPluginStage::Postprocess,
        })
        .unwrap()
        .register_plugin(TracePlugin {
            id: "trace_pre",
            stage: SecurityPluginStage::Preprocess,
        })
        .unwrap()
        .register_plugin(TracePlugin {
            id: "trace_logging",
            stage: SecurityPluginStage::Logging,
        })
        .unwrap();
    let engine = SecurityEventEngine::new(registry, Arc::clone(&emitter));
    let rules = SecurityRuleSet::new(Vec::new());
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
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
            .map(|detection| (detection.source, detection.plugin_id.as_deref(), detection.plugin_mode))
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
    assert_eq!(
        returned
            .plugin_executions
            .iter()
            .map(|execution| (
                execution.plugin_id.as_str(),
                execution.stage,
                execution.applied,
                execution.duration_us <= 1_000_000,
            ))
            .collect::<Vec<_>>(),
        vec![
            ("trace_pre", SecurityPluginStage::Preprocess, true, true),
            ("trace_post", SecurityPluginStage::Postprocess, true, true),
            ("trace_logging", SecurityPluginStage::Logging, true, true),
        ],
        "plugin execution counters must ride on the same security event as detections"
    );
    assert_eq!(emitter.events.lock().unwrap().as_slice(), [returned]);

    let snapshot = snapshotter.snapshot().into_vec();
    let plugin_counter_ids = snapshot
        .iter()
        .filter_map(|(key, _, _, value)| {
            if key.key().name() != SECURITY_PLUGIN_EXECUTION_TOTAL || !matches!(value, DebugValue::Counter(1)) {
                return None;
            }
            let labels = key.key().labels().collect::<Vec<_>>();
            let plugin_id = labels
                .iter()
                .find(|label| label.key() == "plugin_id")
                .map(|label| label.value().to_string())?;
            let status_ok = labels
                .iter()
                .any(|label| label.key() == "status" && label.value() == "ok");
            let applied = labels
                .iter()
                .any(|label| label.key() == "applied" && label.value() == "true");
            status_ok.then_some(())?;
            applied.then_some(plugin_id)
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        plugin_counter_ids,
        ["trace_logging", "trace_post", "trace_pre"]
            .into_iter()
            .map(str::to_string)
            .collect::<std::collections::BTreeSet<_>>(),
        "every enabled plugin execution must emit a labeled counter"
    );
    let plugin_histogram_ids = snapshot
        .iter()
        .filter_map(|(key, _, _, value)| {
            if key.key().name() != SECURITY_PLUGIN_EXECUTION_DURATION_MS || !matches!(value, DebugValue::Histogram(_)) {
                return None;
            }
            key.key()
                .labels()
                .find(|label| label.key() == "plugin_id")
                .map(|label| label.value().to_string())
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        plugin_histogram_ids, plugin_counter_ids,
        "every enabled plugin counter must have a matching duration histogram"
    );
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
            stage: SecurityPluginStage::Postprocess,
        })
        .unwrap();
    let engine = SecurityEventEngine::new(registry, Arc::clone(&emitter));
    let rules = SecurityRuleSet::new(Vec::new());
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
        host: Some("api.openai.com".to_string()),
        ..Default::default()
    });

    let returned = engine.apply_matching_rules_and_emit(&rules, event.clone()).unwrap();

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
            stage: SecurityPluginStage::Postprocess,
        })
        .unwrap();
    let engine = SecurityEventEngine::new(registry, Arc::clone(&emitter));
    let rules = SecurityRuleSet::new(Vec::new());
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
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
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
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
            stage: SecurityPluginStage::Postprocess,
        })
        .unwrap();
    let rewrite_returned = SecurityEventEngine::new(rewrite_registry, Arc::new(RecordingEmitter::new()))
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
            stage: SecurityPluginStage::Postprocess,
        })
        .unwrap();
    let disabled_returned = SecurityEventEngine::new(disabled_registry, Arc::new(RecordingEmitter::new()))
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
            stage: SecurityPluginStage::Preprocess,
            requested: SecurityDecisionKind::Block,
        })
        .unwrap()
        .register_plugin(DecisionPlugin {
            id: "allow_after",
            stage: SecurityPluginStage::Postprocess,
            requested: SecurityDecisionKind::Allow,
        })
        .unwrap();
    let engine = SecurityEventEngine::new(registry, Arc::clone(&emitter));
    let rules = SecurityRuleSet::new(Vec::new());
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
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
    let registry = SecurityActionRegistry::with_builtin_actions().with_plugin_policy(BTreeMap::from([
        (
            "dummy_pre_eicar".to_string(),
            plugin_config(SecurityPluginMode::Block, DetectionLevel::Critical),
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
    let event = SecurityEvent::new(RuntimeSecurityEventType::FileImport).with_file(FileSecurityEvent {
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
                Some(SecurityPluginMode::Block),
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
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
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
    let _store_guard = EnvVarGuard::set(crate::credential_broker::STORE_PATH_ENV, &store_path);
    let _user_guard = EnvVarGuard::set("CAPSEM_HOME", tmp.path());
    let emitter = Arc::new(RecordingEmitter::new());
    let registry = SecurityActionRegistry::with_builtin_actions().with_plugin_policy(BTreeMap::from([(
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
        crate::credential_broker::resolve_broker_reference_for_provider(CredentialProvider::Github, credential_ref,)
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
    let _store_guard = EnvVarGuard::set(crate::credential_broker::STORE_PATH_ENV, &store_path);
    let _user_guard = EnvVarGuard::set("CAPSEM_HOME", tmp.path());
    let emitter = Arc::new(RecordingEmitter::new());
    let registry = SecurityActionRegistry::with_builtin_actions().with_plugin_policy(BTreeMap::from([
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
            trace_id: None,
            context_json: None,
        }]);

    let returned = engine
        .apply_matching_rules_and_emit(&SecurityRuleSet::new(Vec::new()), event)
        .expect("credential broker plus logging sanitizer should emit a safe event");

    let events = emitter.events.lock().unwrap();
    assert_eq!(events.as_slice(), std::slice::from_ref(&returned));
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
    let _store_guard = EnvVarGuard::set(crate::credential_broker::STORE_PATH_ENV, &store_path);
    let _user_guard = EnvVarGuard::set("CAPSEM_HOME", tmp.path());
    let emitter = Arc::new(RecordingEmitter::new());
    let registry = SecurityActionRegistry::with_builtin_actions().with_plugin_policy(BTreeMap::from([(
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
        crate::credential_broker::resolve_broker_reference_for_provider(CredentialProvider::OpenAi, credential_ref,)
            .unwrap()
            .as_deref(),
        Some(raw)
    );
    assert_eq!(emitter.events.lock().unwrap().as_slice(), [returned]);
}
