//! Credential-substitution and security-rule ledger writes from the hook.
//!
//! Split out of `tests.rs`: the fixtures and helpers stay there and come in
//! through `use super::*`, so a test moved here keeps the same environment it
//! had before.
use super::*;

#[tokio::test]
async fn hook_writes_substitution_event_and_shared_credential_ref() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let capsem_home = dir.path().join("capsem-home");
    let test_store = dir.path().join("credential-store.json");
    let _guard = EnvGuard::install(&capsem_home, dir.path(), &test_store);

    let db = Arc::new(DbWriter::open(&db_path, 64).expect("test db"));
    let deps = Arc::new(TelemetryDeps {
        db: Arc::clone(&db),
        pricing: Arc::new(PricingTable::load()),
        trace_state: Arc::new(Mutex::new(TraceState::new())),
        security_rules: empty_security_rules(),
        plugin_policy: Arc::new(std::sync::RwLock::new(BTreeMap::new().into())),
    });
    let hook = TelemetryHook::new(deps);
    let raw = "sk-ant-hook-test";
    let credential_ref = credential_reference("anthropic", raw);
    let mut req_ctx = anthropic_req_ctx();
    req_ctx.credential_ref = Some(credential_ref.clone());
    let observation = CredentialObservation {
        provider: CredentialProvider::Anthropic,
        raw_value: raw.to_string(),
        source: "http.header.x-api-key".to_string(),
        event_type: Some("http.request".to_string()),
        trace_id: Some("trace-hook".to_string()),
        context_json: Some(r#"{"domain":"api.anthropic.com"}"#.to_string()),
    };
    req_ctx.credential_observations = vec![observation.clone(), observation];

    let mut state = HookState::default();
    let conn = any_conn();
    {
        let mut c = ctx_for(&mut state, &conn);
        *c.state::<Option<TelemetryRequestContext>>(|| None) = Some(req_ctx);
    }
    complete_response(&hook, &mut state, &conn).await;

    let seen = wait_for_db(&db, &db_path, |conn| {
        let net_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM net_events WHERE credential_ref = ?1",
                [&credential_ref],
                |row| row.get(0),
            )
            .unwrap();
        let captured_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM substitution_events WHERE substitution_ref = ?1 AND outcome = 'captured'",
                [&credential_ref],
                |row| row.get(0),
            )
            .unwrap();
        let brokered_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM substitution_events WHERE substitution_ref = ?1 AND outcome = 'brokered'",
                [&credential_ref],
                |row| row.get(0),
            )
            .unwrap();
        net_count == 1 && captured_count == 1 && brokered_count == 1
    })
    .await;

    assert!(seen, "expected net and substitution rows with shared credential_ref");
    db.shutdown_blocking();
    let db_bytes = std::fs::read(&db_path).unwrap();
    assert!(
        !String::from_utf8_lossy(&db_bytes).contains(raw),
        "raw credential leaked into session db"
    );
}

#[tokio::test]
async fn hook_does_not_repay_capture_ledger_for_repeated_identical_credential() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let capsem_home = dir.path().join("capsem-home");
    let test_store = dir.path().join("credential-store.json");
    let _guard = EnvGuard::install(&capsem_home, dir.path(), &test_store);

    let db = Arc::new(DbWriter::open(&db_path, 64).expect("test db"));
    let deps = Arc::new(TelemetryDeps {
        db: Arc::clone(&db),
        pricing: Arc::new(PricingTable::load()),
        trace_state: Arc::new(Mutex::new(TraceState::new())),
        security_rules: empty_security_rules(),
        plugin_policy: Arc::new(std::sync::RwLock::new(BTreeMap::new().into())),
    });
    let hook = TelemetryHook::new(deps);
    let raw = "sk-ant-hook-repeat-test";
    let credential_ref = credential_reference("anthropic", raw);
    let observation = CredentialObservation {
        provider: CredentialProvider::Anthropic,
        raw_value: raw.to_string(),
        source: "http.header.x-api-key".to_string(),
        event_type: Some("http.request".to_string()),
        trace_id: Some("trace-hook-repeat".to_string()),
        context_json: Some(r#"{"domain":"api.anthropic.com"}"#.to_string()),
    };

    for _ in 0..2 {
        let mut req_ctx = anthropic_req_ctx();
        req_ctx.credential_ref = Some(credential_ref.clone());
        req_ctx.credential_observations = vec![observation.clone()];
        let mut state = HookState::default();
        let conn = any_conn();
        {
            let mut c = ctx_for(&mut state, &conn);
            *c.state::<Option<TelemetryRequestContext>>(|| None) = Some(req_ctx);
        }
        complete_response(&hook, &mut state, &conn).await;
    }

    let seen = wait_for_db(&db, &db_path, |conn| {
        let net_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM net_events WHERE credential_ref = ?1",
                [&credential_ref],
                |row| row.get(0),
            )
            .unwrap();
        let captured_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM substitution_events WHERE substitution_ref = ?1 AND outcome = 'captured'",
                [&credential_ref],
                |row| row.get(0),
            )
            .unwrap();
        let brokered_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM substitution_events WHERE substitution_ref = ?1 AND outcome = 'brokered'",
                [&credential_ref],
                |row| row.get(0),
            )
            .unwrap();
        net_count == 2 && captured_count == 1 && brokered_count == 1
    })
    .await;

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let net_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM net_events WHERE credential_ref = ?1",
            [&credential_ref],
            |row| row.get(0),
        )
        .unwrap();
    let captured_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM substitution_events WHERE substitution_ref = ?1 AND outcome = 'captured'",
            [&credential_ref],
            |row| row.get(0),
        )
        .unwrap();
    let brokered_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM substitution_events WHERE substitution_ref = ?1 AND outcome = 'brokered'",
            [&credential_ref],
            |row| row.get(0),
        )
        .unwrap();

    assert!(
        seen,
        "repeated request rows must keep credential_ref, but capture/broker ledger identity emits once; net={net_count} captured={captured_count} brokered={brokered_count}"
    );
    db.shutdown_blocking();
}

#[tokio::test]
async fn hook_writes_security_rule_ledger_for_matching_http_event() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let rules_profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.anthropic_http_seen]
name = "anthropic_http_seen"
action = "allow"
detection_level = "informational"
match = 'http.host == "api.anthropic.com" && http.path == "/v1/messages" && tcp.port == "443"'
"#,
    )
    .expect("rules parse");
    let rules = SecurityRuleSet::compile_profile(&rules_profile, SecurityRuleSource::User).expect("rules compile");
    let db = Arc::new(DbWriter::open(&db_path, 64).expect("test db"));
    let deps = Arc::new(TelemetryDeps {
        db: Arc::clone(&db),
        pricing: Arc::new(PricingTable::load()),
        trace_state: Arc::new(Mutex::new(TraceState::new())),
        security_rules: Arc::new(std::sync::RwLock::new(Arc::new(rules))),
        plugin_policy: Arc::new(std::sync::RwLock::new(BTreeMap::new().into())),
    });
    let hook = TelemetryHook::new(deps);

    let mut state = HookState::default();
    let conn = any_conn();
    {
        let mut c = ctx_for(&mut state, &conn);
        *c.state::<Option<TelemetryRequestContext>>(|| None) = Some(anthropic_req_ctx());
    }
    complete_response(&hook, &mut state, &conn).await;

    let mut seen = false;
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        db.flush().await;
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let joined: Option<(String, String, String)> = conn
            .query_row(
                "SELECT net_events.event_id, security_rule_events.rule_id, security_rule_events.detection_level
                 FROM net_events
                 JOIN security_rule_events ON security_rule_events.event_id = net_events.event_id
                 WHERE net_events.domain = 'api.anthropic.com'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok();
        let Some((event_id, rule_id, detection_level)) = joined else {
            continue;
        };
        assert_eq!(event_id.len(), 12);
        assert!(event_id.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
        assert_eq!(rule_id, "profiles.rules.anthropic_http_seen");
        assert_eq!(detection_level, "informational");
        seen = true;
        break;
    }

    assert!(seen, "expected HTTP telemetry to write a joined rule ledger row");
}

#[tokio::test]
async fn hook_writes_security_rule_ledger_for_matching_model_event() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let rules_profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.anthropic_model_seen]
name = "anthropic_model_seen"
action = "allow"
detection_level = "informational"
match = 'model.provider == "anthropic" && model.name == "claude-test"'
"#,
    )
    .expect("rules parse");
    let rules = SecurityRuleSet::compile_profile(&rules_profile, SecurityRuleSource::User).expect("rules compile");
    let db = Arc::new(DbWriter::open(&db_path, 64).expect("test db"));
    let deps = Arc::new(TelemetryDeps {
        db: Arc::clone(&db),
        pricing: Arc::new(PricingTable::load()),
        trace_state: Arc::new(Mutex::new(TraceState::new())),
        security_rules: Arc::new(std::sync::RwLock::new(Arc::new(rules))),
        plugin_policy: Arc::new(std::sync::RwLock::new(BTreeMap::new().into())),
    });
    let hook = TelemetryHook::new(deps);

    let mut state = HookState::default();
    let conn = any_conn();
    {
        let mut c = ctx_for(&mut state, &conn);
        *c.state::<Option<TelemetryRequestContext>>(|| None) = Some(anthropic_req_ctx());
    }
    complete_response(&hook, &mut state, &conn).await;

    let mut seen = false;
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        db.flush().await;
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let joined: Option<(String, String, String)> = conn
            .query_row(
                "SELECT model_calls.event_id, security_rule_events.rule_id, security_rule_events.detection_level
                 FROM model_calls
                 JOIN security_rule_events ON security_rule_events.event_id = model_calls.event_id
                 WHERE model_calls.provider = 'anthropic'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok();
        let Some((event_id, rule_id, detection_level)) = joined else {
            continue;
        };
        assert_eq!(event_id.len(), 12);
        assert!(event_id.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
        assert_eq!(rule_id, "profiles.rules.anthropic_model_seen");
        assert_eq!(detection_level, "informational");
        seen = true;
        break;
    }

    assert!(seen, "expected model telemetry to write a joined rule ledger row");
}

#[tokio::test]
async fn hook_writes_injected_substitution_event_for_broker_ref_replay() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let capsem_home = dir.path().join("capsem-home");
    let test_store = dir.path().join("credential-store.json");
    let _guard = EnvGuard::install(&capsem_home, dir.path(), &test_store);

    let db = Arc::new(DbWriter::open(&db_path, 64).expect("test db"));
    let deps = Arc::new(TelemetryDeps {
        db: Arc::clone(&db),
        pricing: Arc::new(PricingTable::load()),
        trace_state: Arc::new(Mutex::new(TraceState::new())),
        security_rules: empty_security_rules(),
        plugin_policy: Arc::new(std::sync::RwLock::new(BTreeMap::new().into())),
    });
    let hook = TelemetryHook::new(deps);
    let raw = "sk-ant-replayed-hook-test";
    let credential_ref = credential_reference("anthropic", raw);
    let mut req_ctx = anthropic_req_ctx();
    req_ctx.credential_ref = Some(credential_ref.clone());
    req_ctx.request_headers = Some(format!("authorization: Bearer {credential_ref}"));
    req_ctx.credential_injections = vec![CredentialInjection {
        provider: Some(CredentialProvider::Anthropic),
        credential_ref: credential_ref.clone(),
        source: "http.header.authorization".to_string(),
        event_type: Some("http.request".to_string()),
        trace_id: Some("trace-injected-hook".to_string()),
        context_json: Some(r#"{"domain":"api.anthropic.com"}"#.to_string()),
    }];

    let mut state = HookState::default();
    let conn = any_conn();
    {
        let mut c = ctx_for(&mut state, &conn);
        *c.state::<Option<TelemetryRequestContext>>(|| None) = Some(req_ctx);
    }
    complete_response(&hook, &mut state, &conn).await;

    let seen = wait_for_db(&db, &db_path, |conn| {
        let injected_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM substitution_events WHERE substitution_ref = ?1 AND outcome = 'injected'",
                [&credential_ref],
                |row| row.get(0),
            )
            .unwrap();
        let net_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM net_events WHERE credential_ref = ?1",
                [&credential_ref],
                |row| row.get(0),
            )
            .unwrap();
        injected_count == 1 && net_count == 1
    })
    .await;

    assert!(
        seen,
        "expected injected substitution row with shared net credential_ref"
    );
    let db_bytes = std::fs::read(&db_path).unwrap();
    assert!(
        !String::from_utf8_lossy(&db_bytes).contains(raw),
        "raw credential leaked into session db"
    );
}

#[tokio::test]
async fn hook_detects_response_body_token_exchange_and_redacts_preview() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let capsem_home = dir.path().join("capsem-home");
    let test_store = dir.path().join("credential-store.json");
    let _guard = EnvGuard::install(&capsem_home, dir.path(), &test_store);

    let db = Arc::new(DbWriter::open(&db_path, 64).expect("test db"));
    let deps = Arc::new(TelemetryDeps {
        db: Arc::clone(&db),
        pricing: Arc::new(PricingTable::load()),
        trace_state: Arc::new(Mutex::new(TraceState::new())),
        security_rules: empty_security_rules(),
        plugin_policy: Arc::new(std::sync::RwLock::new(BTreeMap::new().into())),
    });
    let hook = TelemetryHook::new(deps);
    let raw = "github_pat_exchange_secret";

    let mut req_ctx = anthropic_req_ctx();
    req_ctx.domain = "api.github.com".to_string();
    req_ctx.ai_provider = None;
    req_ctx.path = "/login/oauth/access_token".to_string();
    req_ctx.request_headers = Some("host: api.github.com".to_string());
    req_ctx.response_headers = Some("content-type: application/json".to_string());

    let mut state = HookState::default();
    let conn = ConnMeta {
        domain: "api.github.com".to_string(),
        port: 443,
        process_name: None,
        ..Default::default()
    };
    {
        let mut c = ctx_for(&mut state, &conn);
        *c.state::<Option<TelemetryRequestContext>>(|| None) = Some(req_ctx);
        *c.state::<TelemetryResponseStats>(TelemetryResponseStats::default) = TelemetryResponseStats {
            bytes: raw.len() as u64,
            preview: format!(r#"{{"access_token":"{raw}","token_type":"bearer"}}"#).into_bytes(),
            max_body_capture: 4096,
        };
    }
    complete_response(&hook, &mut state, &conn).await;

    let seen = wait_for_db(&db, &db_path, |conn| {
        let row: Option<(String, String)> = conn
            .query_row(
                "SELECT credential_ref, response_body_preview FROM net_events WHERE domain = 'api.github.com'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        let Some((credential_ref, preview)) = row else {
            return false;
        };
        let outcomes: Vec<String> = conn
            .prepare(
                "SELECT outcome FROM substitution_events WHERE substitution_ref = ?1 AND source = 'http.body.response.$.access_token' ORDER BY outcome",
            )
            .unwrap()
            .query_map([&credential_ref], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(credential_ref.starts_with("credential:blake3:"));
        assert!(preview.contains("credential:blake3:"));
        assert!(!preview.contains(raw));
        outcomes == ["brokered", "captured"]
    })
    .await;

    assert!(seen, "expected token exchange response to be brokered and redacted");
}
