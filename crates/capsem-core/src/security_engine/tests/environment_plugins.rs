use super::*;
use crate::environment::CompiledEnvironmentPlugins;
use crate::net::proxy_engine::ProxyCredentials;
use capsem_config::plugins::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn registry(environment: &EnvironmentPlugins, corporate: &EnvironmentPlugins) -> SecurityActionRegistry {
    SecurityActionRegistry::with_builtin_actions().with_environment_plugins(Arc::new(
        CompiledEnvironmentPlugins::resolve(environment, corporate).unwrap(),
    ))
}
fn http_event(inbound: bool) -> SecurityEvent {
    SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
        status: inbound.then(|| "200".into()),
        body: Some("person@example.com".into()),
        ..Default::default()
    })
}

#[test]
fn environment_plugins_use_independent_directions_and_isolated_compiled_options() {
    let configured = EnvironmentPlugins {
        pii_filter: Some(PiiFilterConfiguration {
            inbound: PluginConfiguration {
                mode: SecurityPluginMode::Block,
                detection_level: DetectionLevel::Critical,
                ..Default::default()
            },
            outbound: PluginConfiguration {
                mode: SecurityPluginMode::Rewrite,
                detection_level: DetectionLevel::Low,
                ..Default::default()
            },
        }),
        ..Default::default()
    };
    let first = registry(&configured, &EnvironmentPlugins::default());
    let second = registry(&EnvironmentPlugins::default(), &EnvironmentPlugins::default());
    for (inbound, mode, level, decision) in [
        (
            true,
            SecurityPluginMode::Block,
            DetectionLevel::Critical,
            SecurityDecisionKind::Block,
        ),
        (
            false,
            SecurityPluginMode::Rewrite,
            DetectionLevel::Low,
            SecurityDecisionKind::Allow,
        ),
    ] {
        let engine = SecurityEventEngine::new(first.clone(), Arc::new(RecordingEmitter::new()));
        let event = engine
            .apply_matching_rules_and_emit(&SecurityRuleSet::new(Vec::new()), http_event(inbound))
            .unwrap();
        assert_eq!(event.decision.effective, decision);
        let detection = event
            .detections
            .iter()
            .find(|d| d.plugin_id.as_deref() == Some("pii_filter"))
            .unwrap();
        assert_eq!(detection.plugin_mode, Some(mode));
        assert_eq!(detection.detection_level, level);
        assert_eq!(
            event.http.unwrap().body.unwrap().contains("person@example.com"),
            inbound
        );
        let untouched = second
            .apply_security_plugins(SecurityPluginStage::Preprocess, http_event(inbound))
            .unwrap();
        assert_eq!(untouched.http.unwrap().body.as_deref(), Some("person@example.com"));
        assert!(!untouched.plugin_executions.iter().any(|p| p.plugin_id == "pii_filter"));
    }
    let corporate = EnvironmentPlugins {
        pii_filter: Some(PiiFilterConfiguration {
            inbound: PluginConfiguration {
                mode: SecurityPluginMode::Rewrite,
                options: PiiFilterOptions {
                    email: false,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    let locked = registry(&configured, &corporate);
    for inbound in [false, true] {
        let event = locked
            .apply_security_plugins(SecurityPluginStage::Preprocess, http_event(inbound))
            .unwrap();
        assert_eq!(event.http.unwrap().body.as_deref(), Some("person@example.com"));
        assert!(
            event.detections.is_empty(),
            "whole corporate entry replaces both directions and categories"
        );
    }
}

#[derive(Default)]
struct Credentials {
    captures: AtomicUsize,
    substitutions: AtomicUsize,
}

#[test]
fn environment_plugins_utf8_selection_uses_compiled_direction_without_reparsing() {
    let environment = EnvironmentPlugins {
        utf8_guard: Some(PluginConfiguration {
            mode: SecurityPluginMode::Block,
            detection_level: DetectionLevel::High,
            options: Utf8GuardOptions {
                inbound: false,
                outbound: true,
            },
        }),
        ..Default::default()
    };
    let configured = registry(&environment, &EnvironmentPlugins::default());
    for inbound in [false, true] {
        let mut event = http_event(inbound);
        event.http_body_utf8_valid = Some(false);
        let result = configured
            .apply_security_plugins(SecurityPluginStage::Preprocess, event)
            .unwrap();
        assert_eq!(
            result.decision.effective,
            if inbound {
                SecurityDecisionKind::Allow
            } else {
                SecurityDecisionKind::Block
            }
        );
        assert_eq!(
            result.plugin_executions.iter().any(|p| p.plugin_id == "utf8_guard"),
            !inbound
        );
    }
}
impl ProxyCredentials for Credentials {
    fn capture(&self, _: &CredentialObservation) -> Result<crate::credential_broker::BrokeredCredential, String> {
        self.captures.fetch_add(1, Ordering::SeqCst);
        Err("capture must not run".into())
    }
    fn substitute_upstream(
        &self,
        _: &str,
        _: Option<ProviderKind>,
        headers: &mut http::HeaderMap,
        query: Option<&str>,
    ) -> Result<BrokeredUpstreamCredentials, String> {
        self.substitutions.fetch_add(1, Ordering::SeqCst);
        headers.insert("x-api-key", http::HeaderValue::from_static("upstream-only-test-token"));
        Ok(BrokeredUpstreamCredentials {
            credential_ref: None,
            query: query.map(String::from),
        })
    }
}

#[test]
fn environment_plugins_capture_off_keeps_substitution_and_sanitization() {
    let config = EnvironmentPlugins {
        credential_broker: Some(PluginConfiguration {
            options: CredentialBrokerOptions { capture_enabled: false },
            ..Default::default()
        }),
        ..Default::default()
    };
    let credentials = Arc::new(Credentials::default());
    let registry = registry(&config, &EnvironmentPlugins::default()).with_proxy_credentials(credentials.clone());
    let reference = format!("credential:blake3:{}", "a".repeat(64));
    for value in ["sk-ant-abcdefghijklmnopqrstuvwxyz0123456789", reference.as_str()] {
        let mut headers = http::HeaderMap::new();
        headers.insert("x-api-key", http::HeaderValue::from_str(value).unwrap());
        let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http_request(
            HttpRequestSecurityEvent::new("api.anthropic.com", None, headers, Some("private=value".into())),
        );
        let event = registry
            .apply_security_plugins(SecurityPluginStage::Preprocess, event)
            .unwrap();
        assert!(!event.action_trace.contains(&PolicyActionId::CredentialBrokerCapture));
        if value == reference {
            assert!(event.action_trace.contains(&PolicyActionId::CredentialBrokerSubstitute));
            let upstream = super::super::http_materialization::materialize_http_request_for_upstream_with_credentials(
                &event,
                credentials.as_ref(),
            )
            .unwrap();
            assert_eq!(upstream.headers["x-api-key"], "upstream-only-test-token");
            assert_eq!(event.http_request.as_ref().unwrap().headers["x-api-key"], reference);
        } else {
            assert!(
                !event.credential_observations.is_empty(),
                "sanitization still needs observations"
            );
        }
        let logged = registry
            .apply_security_plugins(SecurityPluginStage::Logging, event)
            .unwrap();
        assert!(logged.credential_observations.is_empty());
        assert_eq!(
            logged.http_request.as_ref().unwrap().query.as_deref(),
            Some("[REDACTED]")
        );
        if value != reference {
            assert_ne!(logged.http_request.unwrap().headers["x-api-key"], value);
        }
    }
    assert_eq!(credentials.captures.load(Ordering::SeqCst), 0);
    assert_eq!(credentials.substitutions.load(Ordering::SeqCst), 1);
}

#[test]
fn typed_plugin_policy_reaches_the_public_security_boundary_without_flattening_options() {
    let environment: EnvironmentPlugins = toml::from_str("[pii_filter.inbound]\nmode='block'\n[pii_filter.inbound.options]\nemail=false\n[pii_filter.outbound]\nmode='block'").unwrap();
    let compiled = Arc::new(CompiledEnvironmentPlugins::resolve(&environment, &EnvironmentPlugins::default()).unwrap());
    let inbound =
        evaluate_security_boundary(&SecurityRuleSet::new(Vec::new()), compiled.clone(), http_event(true)).unwrap();
    assert!(
        inbound.enforcement.is_allowed(),
        "disabled inbound email category must remain disabled"
    );
    let outbound = evaluate_security_boundary(&SecurityRuleSet::new(Vec::new()), compiled, http_event(false)).unwrap();
    assert!(
        !outbound.enforcement.is_allowed(),
        "independent outbound block must remain enforced"
    );
    let corporate: EnvironmentPlugins = toml::from_str("[pii_filter.inbound]\nmode='block'").unwrap();
    let locked = Arc::new(CompiledEnvironmentPlugins::resolve(&environment, &corporate).unwrap());
    assert!(
        !evaluate_security_boundary(&SecurityRuleSet::new(Vec::new()), locked, http_event(true))
            .unwrap()
            .enforcement
            .is_allowed()
    );
}
