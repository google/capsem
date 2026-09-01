use super::*;

#[test]
fn http_materializer_without_substitute_action_keeps_reference() {
    let (event, reference, _raw, _tmp, _store_guard, _user_config_guard, _lock) = brokered_anthropic_header_event();

    let materialized = materialize_http_request_for_upstream(&event).unwrap();

    assert_eq!(
        materialized.headers.get(http::header::AUTHORIZATION).unwrap(),
        &http::HeaderValue::from_str(&reference).unwrap(),
        "without a matched substitute action, materialization must stay reference-only"
    );
    assert_eq!(materialized.credential_ref, None);
}

#[test]
fn credential_broker_plugin_marks_broker_ref_for_injection_not_recapture() {
    let (mut event, reference, raw, _tmp, _store_guard, _user_config_guard, _lock) = brokered_anthropic_header_event();
    let request = event.http_request.as_mut().expect("http request event");
    request.headers.insert(
        http::header::AUTHORIZATION,
        http::HeaderValue::from_str(&format!("Bearer {reference}")).unwrap(),
    );
    let registry = SecurityActionRegistry::with_builtin_actions().with_plugin_policy(BTreeMap::from([(
        "credential_broker".to_string(),
        plugin_config(SecurityPluginMode::Rewrite, DetectionLevel::Informational),
    )]));

    let event = registry
        .apply_security_plugins(SecurityPluginStage::Preprocess, event)
        .expect("broker plugin runs");

    assert!(
        event.credential_observations.is_empty(),
        "broker refs are already ledger-safe references, not new raw credentials"
    );
    assert_eq!(event.credential_injections.len(), 1);
    assert_eq!(
        event.credential_injections[0].credential_ref.as_str(),
        reference.as_str()
    );
    assert_eq!(event.credential_injections[0].source, "http.header.authorization");
    assert_eq!(event.action_trace, vec![PolicyActionId::CredentialBrokerSubstitute]);
    let materialized = materialize_http_request_for_upstream(&event).unwrap();
    assert_eq!(
        event
            .http_request
            .as_ref()
            .unwrap()
            .headers
            .get(http::header::AUTHORIZATION)
            .unwrap(),
        &http::HeaderValue::from_str(&format!("Bearer {reference}")).unwrap(),
        "the security event stays reference-only"
    );
    assert_eq!(
        materialized.headers.get(http::header::AUTHORIZATION).unwrap(),
        &http::HeaderValue::from_str(&format!("Bearer {raw}")).unwrap(),
        "only the upstream materialized copy receives the raw credential"
    );
    assert_eq!(materialized.credential_ref.as_deref(), Some(reference.as_str()));
}

#[test]
fn http_materializer_requires_allow_enforcement_decision() {
    let event =
        SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http_request(HttpRequestSecurityEvent::new(
            "api.openai.com",
            Some(ProviderKind::OpenAi),
            http::HeaderMap::new(),
            None,
        ));
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
    let (mut event, reference, raw, _tmp, _store_guard, _user_config_guard, _lock) = brokered_anthropic_header_event();
    event.action_trace.push(PolicyActionId::CredentialBrokerSubstitute);

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
        materialized.headers.get(http::header::AUTHORIZATION).unwrap(),
        &http::HeaderValue::from_str(&raw).unwrap(),
        "only the upstream materialized copy receives the raw credential"
    );
    assert_eq!(materialized.credential_ref.as_deref(), Some(reference.as_str()));
}

fn fully_populated_security_event() -> SecurityEvent {
    let text = || Some("populated".to_string());
    SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
        .with_http(HttpSecurityEvent {
            host: text(),
            method: text(),
            path: text(),
            query: text(),
            status: text(),
            body: text(),
        })
        .with_dns(DnsSecurityEvent {
            qname: text(),
            qtype: text(),
        })
        .with_mcp(McpSecurityEvent {
            method: text(),
            server_name: text(),
            tool_call_name: text(),
            tool_list: text(),
            request: Some(McpRequestSecurityEvent {
                id: text(),
                method: text(),
                arguments: Some(serde_json::json!({"populated": true})),
            }),
            response: Some(McpResponseSecurityEvent {
                content: Some(serde_json::json!(["populated"])),
            }),
            error: Some(McpErrorSecurityEvent { message: text() }),
        })
        .with_model(ModelSecurityEvent {
            provider: text(),
            name: text(),
            request_body: text(),
            response_body: text(),
            tool_calls: text(),
        })
        .with_file(FileSecurityEvent {
            import_path: text(),
            import_name: text(),
            import_ext: text(),
            import_mime_type: text(),
            import_content: text(),
            export_path: text(),
            export_name: text(),
            export_ext: text(),
            export_mime_type: text(),
            export_content: text(),
            read_path: text(),
            read_name: text(),
            read_ext: text(),
            read_mime_type: text(),
            read_content: text(),
            create_path: text(),
            create_name: text(),
            create_ext: text(),
            create_mime_type: text(),
            create_content: text(),
            write_path: text(),
            write_name: text(),
            write_ext: text(),
            write_mime_type: text(),
            write_content: text(),
            delete_path: text(),
            delete_name: text(),
            delete_ext: text(),
            delete_mime_type: text(),
            delete_content: text(),
            content: text(),
        })
        .with_process(ProcessSecurityEvent {
            exec_id: text(),
            exec_path: text(),
            name: text(),
            command: text(),
            exit_code: text(),
            stdout: text(),
            stderr: text(),
        })
        .with_ip(IpSecurityEvent {
            value: text(),
            version: text(),
        })
        .with_tcp(TcpSecurityEvent { port: text() })
        .with_udp(UdpSecurityEvent { port: text() })
}

#[test]
fn security_event_cel_fields_all_resolve() {
    use crate::net::policy_config::PolicySubject;

    let event = fully_populated_security_event();
    let unresolved = SECURITY_EVENT_CEL_FIELDS
        .iter()
        .copied()
        .filter(|field| event.get_policy_field(field).is_none())
        .collect::<Vec<_>>();

    assert!(
        unresolved.is_empty(),
        "every advertised CEL field must resolve on a fully populated event; \
         these do not: {unresolved:?}"
    );
}

#[test]
fn security_event_cel_fields_are_sorted_deduped_and_rooted() {
    let mut sorted = SECURITY_EVENT_CEL_FIELDS.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted, SECURITY_EVENT_CEL_FIELDS,
        "the CEL field contract must stay sorted and deduped so drift is reviewable"
    );

    for field in SECURITY_EVENT_CEL_FIELDS {
        let root = field.split('.').next().expect("field has a root");
        assert!(
            crate::net::policy_config::SECURITY_EVENT_CEL_ROOTS.contains(&root),
            "field '{field}' hangs off a root that is not a declared CEL root"
        );
        assert!(
            field.len() > root.len() + 1,
            "field '{field}' must name a leaf, not a bare family root"
        );
    }
}

#[test]
fn every_cel_root_exposes_at_least_one_field() {
    for root in crate::net::policy_config::SECURITY_EVENT_CEL_ROOTS {
        let prefix = format!("{root}.");
        assert!(
            SECURITY_EVENT_CEL_FIELDS.iter().any(|field| field.starts_with(&prefix)),
            "root '{root}' is advertised but exposes no CEL field"
        );
    }
}

#[test]
fn explicit_file_events_carry_credential_ref_into_the_rule_ledger() {
    let event = security_event_from_explicit_file_event(&ExplicitFileSecurityEvent {
        action: FileAction::Exported,
        path: "/workspace/secret.txt".to_string(),
        size: Some(4),
        content: Some("body".to_string()),
        mime_type: None,
        trace_id: Some("trace-1".to_string()),
        credential_ref: Some(
            "credential:blake3:abababababababababababababababababababababababababababababababab".to_string(),
        ),
    });

    assert_eq!(event.trace_id.as_deref(), Some("trace-1"));
    assert_eq!(
        event.credential_ref.as_deref(),
        Some("credential:blake3:abababababababababababababababababababababababababababababababab"),
        "rule and decision ledger rows correlate on credential_ref, so the \
         explicit-file boundary must not drop it"
    );
}

#[tokio::test]
async fn explicit_file_credential_ref_reaches_the_rule_and_decision_ledger() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 32).unwrap();
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.credentialed_export]
name = "credentialed_export"
action = "block"
priority = 10
detection_level = "high"
match = 'file.export.path == "/workspace/secret.txt"'
"#,
    )
    .unwrap();
    let rules =
        crate::net::policy_config::SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap();

    emit_explicit_file_security_write_and_rules(
        &writer,
        &rules,
        ExplicitFileSecurityEvent {
            action: FileAction::Exported,
            path: "/workspace/secret.txt".to_string(),
            size: Some(4),
            content: Some("body".to_string()),
            mime_type: Some("text/plain".to_string()),
            trace_id: Some("trace_export".to_string()),
            credential_ref: Some(
                "credential:blake3:abababababababababababababababababababababababababababababababab".to_string(),
            ),
        },
    )
    .await
    .expect("explicit file event must receive id");
    writer.flush().await;
    writer.shutdown_blocking();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let file_ref: Option<String> = conn
        .query_row("SELECT credential_ref FROM fs_events", [], |row| row.get(0))
        .unwrap();
    let rule_ref: Option<String> = conn
        .query_row("SELECT credential_ref FROM security_rule_events", [], |row| row.get(0))
        .unwrap();
    let decision_ref: Option<String> = conn
        .query_row("SELECT credential_ref FROM security_decision_events", [], |row| {
            row.get(0)
        })
        .unwrap();

    assert_eq!(
        file_ref.as_deref(),
        Some("credential:blake3:abababababababababababababababababababababababababababababababab")
    );
    assert_eq!(
        rule_ref.as_deref(),
        Some("credential:blake3:abababababababababababababababababababababababababababababababab"),
        "the rule row must be correlatable to the credential the file event named"
    );
    assert_eq!(
        decision_ref.as_deref(),
        Some("credential:blake3:abababababababababababababababababababababababababababababababab"),
        "the decision row must carry the same credential as the rule row"
    );
}
