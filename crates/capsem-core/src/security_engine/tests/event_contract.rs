use super::*;

#[test]
fn security_event_cel_evaluates_one_cross_root_rule_without_fanout() {
    let condition = r#"
http.host.matches("(^|.*\.)openai\.com$")
|| model.provider == "openai"
|| file.import.path.endsWith(".env")
"#;

    let http_event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
        host: Some("api.openai.com".to_string()),
        ..Default::default()
    });
    assert!(crate::net::policy_config::evaluate_security_event_match(condition, &http_event).unwrap());

    let model_event = SecurityEvent::new(RuntimeSecurityEventType::ModelCall).with_model(ModelSecurityEvent {
        provider: Some("openai".to_string()),
        ..Default::default()
    });
    assert!(crate::net::policy_config::evaluate_security_event_match(condition, &model_event).unwrap());

    let file_event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_file(FileSecurityEvent {
        import_path: Some("/workspace/.env".to_string()),
        ..Default::default()
    });
    assert!(crate::net::policy_config::evaluate_security_event_match(condition, &file_event).unwrap());
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

    let error = crate::net::policy_config::validate_security_event_match(r#"security.decision == "allow""#)
        .expect_err("rules must not predicate on decisions emitted by the rule engine");
    assert!(error.contains("not a first-party security-event root"), "{error}");
}

#[test]
fn security_event_cel_missing_roots_are_non_matches() {
    let condition = r#"
http.host.matches("(^|.*\.)openai\.com$")
|| model.provider == "openai"
|| file.import.path.endsWith(".env")
"#;
    let dns_event = SecurityEvent::new(RuntimeSecurityEventType::DnsQuery).with_dns(DnsSecurityEvent {
        qname: Some("example.com".to_string()),
        qtype: Some("A".to_string()),
    });

    assert!(!crate::net::policy_config::evaluate_security_event_match(condition, &dns_event).unwrap());
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
        .with_mcp(
            McpSecurityEvent {
                tool_call_name: Some("email_send".to_string()),
                ..Default::default()
            }
            .with_request_preview(Some(
                r#"{"name":"email_send","arguments":{"recipient":"bank@example.com","body":"ledger"}}"#,
            ))
            .with_response_preview(Some(r#"{"content":[{"type":"text","text":"queued"}]}"#)),
        )
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
        r#"mcp.request.valid == "true""#,
        r#"mcp.request.arguments.contains("bank@example.com")"#,
        r#"mcp.response.valid == "true""#,
        r#"mcp.response.content.contains("queued")"#,
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
            trace_id: Some("trace_wire".to_string()),
            context_json: None,
        }]);
    event.credential_ref =
        Some("credential:blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string());
    event.action_trace.push(PolicyActionId::CredentialBrokerCapture);
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
    assert_eq!(json["detections"][0]["rule_id"], "profiles.rules.eicar_block");
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
            event_type.as_str().starts_with(event_type.family().as_str()),
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
    let credential_ref = "credential:blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let cases = vec![
        (net_write(Some(credential_ref)), RuntimeSecurityEventType::HttpRequest),
        (model_write(Some(credential_ref)), RuntimeSecurityEventType::ModelCall),
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
        (file_write(Some(credential_ref)), RuntimeSecurityEventType::FileEvent),
        (
            file_write_with_action(FileAction::Imported, Some(credential_ref)),
            RuntimeSecurityEventType::FileImport,
        ),
        (
            file_write_with_action(FileAction::Exported, Some(credential_ref)),
            RuntimeSecurityEventType::FileExport,
        ),
        (exec_write(Some(credential_ref)), RuntimeSecurityEventType::ProcessExec),
        (exec_complete_write(), RuntimeSecurityEventType::ProcessExecComplete),
        (
            audit_write(Some(credential_ref)),
            RuntimeSecurityEventType::ProcessAudit,
        ),
        (dns_write(Some(credential_ref)), RuntimeSecurityEventType::DnsQuery),
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
        let has_label =
            |name: &str, want: &str| labels.iter().any(|label| label.key() == name && label.value() == want);
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
                label.key() == "event_type" && label.value() == RuntimeSecurityEventType::FileEvent.as_str()
            })
            && labels.iter().any(|label| {
                label.key() == "event_family" && label.value() == RuntimeSecurityEventFamily::File.as_str()
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

    assert_eq!(SecurityEventId::parse("abcdef123456").unwrap().as_str(), "abcdef123456");
    assert!(SecurityEventId::parse("ABCDEF123456").is_err());
    assert!(SecurityEventId::parse("evt_abc123").is_err());
    assert!(SecurityEventId::parse("abcdef12345").is_err());
}
