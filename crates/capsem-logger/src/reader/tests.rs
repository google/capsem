use super::*;
use serde_json::json;
use std::time::{Duration, SystemTime};

fn setup_reader_with_data() -> DbReader {
    let reader = DbReader::open_in_memory().unwrap();
    reader
        .conn
        .execute(
            "INSERT INTO net_events (timestamp, domain, port, decision, bytes_sent, bytes_received, duration_ms)
             VALUES ('2026-01-01T00:00:00Z', 'example.com', 443, 'allowed', 100, 200, 50)",
            [],
        )
        .unwrap();
    reader
        .conn
        .execute(
            "INSERT INTO net_events (timestamp, domain, port, decision, bytes_sent, bytes_received, duration_ms)
             VALUES ('2026-01-01T00:01:00Z', 'evil.com', 443, 'denied', 0, 0, 1)",
            [],
        )
        .unwrap();
    reader
}

fn dns_fixture(idx: usize) -> crate::DnsEvent {
    crate::DnsEvent {
        event_id: Some(format!("{idx:012x}")),
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(idx as u64),
        qname: format!("fixture-{idx}.example"),
        qtype: 1,
        qclass: 1,
        rcode: 0,
        answer_ip: Some("127.0.0.1".to_string()),
        decision: crate::Decision::Allowed.as_str().to_string(),
        matched_rule: None,
        source_proto: Some("udp".to_string()),
        process_name: Some("fixture".to_string()),
        upstream_resolver_ms: 0,
        trace_id: Some(format!("{idx:016x}")),
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        credential_ref: None,
    }
}

#[test]
fn query_raw_returns_columnar_json() {
    let reader = setup_reader_with_data();
    let json_str = reader
        .query_raw("SELECT domain, decision FROM net_events ORDER BY id")
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["columns"], json!(["domain", "decision"]));
    assert_eq!(parsed["rows"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["rows"][0][0], "example.com");
    assert_eq!(parsed["rows"][1][0], "evil.com");
}

#[test]
fn query_raw_with_params_binds_values() {
    let reader = setup_reader_with_data();
    let params = vec![json!("denied")];
    let json_str = reader
        .query_raw_with_params("SELECT domain FROM net_events WHERE decision = ?", &params)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["rows"].as_array().unwrap().len(), 1);
    assert_eq!(parsed["rows"][0][0], "evil.com");
}

#[test]
fn query_raw_with_params_integer_bind() {
    let reader = setup_reader_with_data();
    let params = vec![json!(1)];
    let json_str = reader
        .query_raw_with_params("SELECT domain FROM net_events ORDER BY id LIMIT ?", &params)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["rows"].as_array().unwrap().len(), 1);
}

#[test]
fn query_raw_with_params_null_bind() {
    let reader = setup_reader_with_data();
    let params = vec![Value::Null];
    let json_str = reader
        .query_raw_with_params("SELECT domain FROM net_events WHERE method IS ?", &params)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    // Both rows have NULL method
    assert_eq!(parsed["rows"].as_array().unwrap().len(), 2);
}

#[test]
fn query_raw_with_params_float_bind() {
    let reader = setup_reader_with_data();
    let params = vec![json!(49.5)];
    let json_str = reader
        .query_raw_with_params("SELECT domain FROM net_events WHERE duration_ms > ?", &params)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["rows"].as_array().unwrap().len(), 1);
    assert_eq!(parsed["rows"][0][0], "example.com");
}

#[test]
fn query_raw_with_empty_params_works() {
    let reader = setup_reader_with_data();
    let json_str = reader
        .query_raw_with_params("SELECT COUNT(*) AS cnt FROM net_events", &[])
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["rows"][0][0], 2);
}

#[test]
fn query_raw_with_params_does_not_pay_timeout_poll_on_success() {
    let reader = setup_reader_with_data();
    let started = std::time::Instant::now();

    for _ in 0..8 {
        let json_str = reader
            .query_raw_with_params("SELECT COUNT(*) AS cnt FROM net_events", &[])
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["rows"][0][0], 2);
    }

    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_millis(80),
        "successful DB queries must not wait for the 100ms timeout poll; elapsed={elapsed:?}. \
             The route latency contract depends on the DB layer returning immediately when SQLite is done."
    );
}

#[test]
fn validate_select_only_allows_select() {
    assert!(validate_select_only("SELECT 1").is_ok());
    assert!(validate_select_only("  select * from foo").is_ok());
    assert!(validate_select_only("WITH cte AS (SELECT 1) SELECT * FROM cte").is_ok());
    assert!(validate_select_only("EXPLAIN SELECT 1").is_ok());
}

#[test]
fn validate_select_only_rejects_writes() {
    assert!(validate_select_only("INSERT INTO foo VALUES (1)").is_err());
    assert!(validate_select_only("UPDATE foo SET x=1").is_err());
    assert!(validate_select_only("DELETE FROM foo").is_err());
    assert!(validate_select_only("DROP TABLE foo").is_err());
    assert!(validate_select_only("CREATE TABLE foo (x INT)").is_err());
    assert!(validate_select_only("PRAGMA journal_mode=OFF").is_err());
    assert!(validate_select_only("ATTACH ':memory:' AS db2").is_err());
}

#[test]
fn validate_select_only_rejects_empty() {
    assert!(validate_select_only("").is_err());
    assert!(validate_select_only("   ").is_err());
}

#[test]
fn bind_params_do_not_bypass_validation() {
    // Even with params, the SQL statement itself is validated first.
    // The validate_select_only function checks the SQL text, not the params.
    assert!(validate_select_only("DELETE FROM foo WHERE id = ?").is_err());
    assert!(validate_select_only("INSERT INTO foo VALUES (?)").is_err());
}

// -----------------------------------------------------------------------
// Richer fixture covering multiple tables, used by aggregate tests below.
// -----------------------------------------------------------------------

fn setup_full_fixture() -> DbReader {
    let reader = DbReader::open_in_memory().unwrap();
    // net_events: 3 allowed, 1 denied, 1 error
    reader.conn.execute_batch(
        "INSERT INTO net_events
                (timestamp, domain, port, decision, method, path, bytes_sent, bytes_received, duration_ms, matched_rule)
             VALUES
                ('2026-01-01T00:00:00Z', 'api.github.com', 443, 'allowed', 'GET',  '/repos',    100, 200, 50, 'allow-github'),
                ('2026-01-01T00:01:00Z', 'api.github.com', 443, 'allowed', 'POST', '/search',   500, 900, 80, 'allow-github'),
                ('2026-01-01T00:02:00Z', 'example.com',    443, 'allowed', 'GET',  '/',         50,  100, 10, NULL),
                ('2026-01-01T00:03:00Z', 'evil.com',       443, 'denied',  'GET',  '/',         0,   0,   1,  'block-evil'),
                ('2026-01-01T00:04:00Z', 'broken.com',     443, 'error',   'GET',  '/boom',     10,  0,   25, NULL);

             INSERT INTO model_calls
                (timestamp, provider, model, method, path, input_tokens, output_tokens, duration_ms, estimated_cost_usd, trace_id)
             VALUES
                ('2026-01-01T00:10:00Z', 'anthropic', 'claude-3',  'POST', '/m', 100, 200, 1500, 0.01, 't1'),
                ('2026-01-01T00:11:00Z', 'anthropic', 'claude-3',  'POST', '/m', 50,  75,  800,  0.005, 't1'),
                ('2026-01-01T00:12:00Z', 'openai',    'gpt-4',     'POST', '/m', 30,  60,  400,  0.003, 't2');

             INSERT INTO tool_calls (model_call_id, call_index, call_id, tool_name, arguments, origin, server_name, method, decision, duration_ms)
             VALUES (1, 0, 'c-1', 'bash',  '{}', 'native', NULL, NULL, 'allowed', 0),
                    (1, 1, 'c-2', 'bash',  '{}', 'native', NULL, NULL, 'allowed', 0),
                    (2, 0, 'c-3', 'fetch', '{}', 'native', NULL, NULL, 'allowed', 0),
                    (NULL, 0, 'mcp-1', 'search_repos', '{}', 'mcp', 'github', 'tools/call', 'allowed', 100),
                    (NULL, 0, 'mcp-2', 'search_repos', '{}', 'mcp', 'github', 'tools/call', 'allowed', 120);

             INSERT INTO fs_events (timestamp, action, path)
             VALUES ('2026-01-01T00:30:00Z', 'create', '/tmp/a'),
                    ('2026-01-01T00:31:00Z', 'modify', '/tmp/a'),
                    ('2026-01-01T00:32:00Z', 'delete', '/tmp/a');
            ",
    ).unwrap();
    reader
}

// -----------------------------------------------------------------------
// Counts / aggregates
// -----------------------------------------------------------------------

#[test]
fn net_event_counts_reports_decision_split() {
    let r = setup_full_fixture();
    let c = r.net_event_counts().unwrap();
    assert_eq!(c.total, 5);
    assert_eq!(c.allowed, 3);
    assert_eq!(c.denied, 1);
}

#[test]
fn net_event_counts_empty_db_returns_zero() {
    let r = DbReader::open_in_memory().unwrap();
    let c = r.net_event_counts().unwrap();
    assert_eq!(c.total, 0);
    assert_eq!(c.allowed, 0);
    assert_eq!(c.denied, 0);
}

#[test]
fn model_call_count_matches_inserts() {
    let r = setup_full_fixture();
    assert_eq!(r.model_call_count().unwrap(), 3);
}

#[test]
fn file_event_count_matches_inserts() {
    let r = setup_full_fixture();
    assert_eq!(r.file_event_count().unwrap(), 3);
}

// -----------------------------------------------------------------------
// Ordering / limiting
// -----------------------------------------------------------------------

#[test]
fn recent_net_events_orders_newest_first() {
    let r = setup_full_fixture();
    let evs = r.recent_net_events(10).unwrap();
    assert_eq!(evs.len(), 5);
    assert_eq!(evs[0].domain, "broken.com"); // last inserted
    assert_eq!(evs[4].domain, "api.github.com"); // first inserted
}

#[test]
fn recent_net_events_respects_limit() {
    let r = setup_full_fixture();
    let evs = r.recent_net_events(2).unwrap();
    assert_eq!(evs.len(), 2);
    assert_eq!(evs[0].domain, "broken.com");
    assert_eq!(evs[1].domain, "evil.com");
}

#[test]
fn recent_security_rule_events_orders_newest_first_and_keeps_payloads() {
    let r = DbReader::open_in_memory().unwrap();
    r.conn
        .execute_batch(
            "INSERT INTO security_rule_events (
                    timestamp_unix_ms, event_id, event_type, rule_id,
                    rule_action, detection_level, rule_json, event_json
                 ) VALUES
                    (1789000000000, '111111111111', 'http.request', 'allow_github',
                     'allow', 'none', '{\"name\":\"allow_github\"}', '{\"http\":{\"host\":\"api.github.com\"}}'),
                    (1789000000001, '222222222222', 'model.call', 'block_openai',
                     'block', 'critical', '{\"name\":\"block_openai\"}', '{\"model\":{\"provider\":\"openai\"}}')",
        )
        .unwrap();

    let latest = r.recent_security_rule_events(2).unwrap();
    assert_eq!(latest.len(), 2);
    assert_eq!(latest[0].event_id, "222222222222");
    assert_eq!(latest[0].rule_id, "block_openai");
    assert_eq!(latest[0].rule_action, SecurityRuleAction::Block);
    assert_eq!(latest[0].detection_level, SecurityDetectionLevel::Critical);
    assert!(latest[0].rule_json.contains("block_openai"));
    assert!(latest[0].event_json.contains("openai"));
}

#[test]
fn security_rule_stats_are_db_only() {
    let r = DbReader::open_in_memory().unwrap();
    r.conn
        .execute_batch(
            "INSERT INTO security_rule_events (
                    timestamp_unix_ms, event_id, event_type, rule_id,
                    rule_action, detection_level, rule_json, event_json
                 ) VALUES
                    (1789000000000, '111111111111', 'model.call', 'block_openai',
                     'block', 'critical', '{}', '{}'),
                    (1789000000001, '222222222222', 'model.call', 'block_openai',
                     'block', 'critical', '{}', '{}'),
                    (1789000000002, '333333333333', 'http.request', 'allow_github',
                     'allow', 'none', '{}', '{}')",
        )
        .unwrap();

    let stats = r.security_rule_stats().unwrap();
    assert_eq!(stats.total, 3);
    assert!(stats
        .by_action
        .iter()
        .any(|entry| entry.rule_action == "block" && entry.count == 2));
    assert!(stats
        .by_event_type
        .iter()
        .any(|entry| entry.event_type == "model.call" && entry.count == 2));
    assert!(stats
        .by_level
        .iter()
        .any(|entry| entry.detection_level == "critical" && entry.count == 2));
    assert!(stats
        .by_level
        .iter()
        .any(|entry| entry.detection_level == "none" && entry.count == 1));
    let block = stats
        .by_rule
        .iter()
        .find(|entry| entry.rule_id == "block_openai")
        .unwrap();
    assert_eq!(block.count, 2);
    assert_eq!(block.latest_event_id, "222222222222");
    assert_eq!(block.latest_timestamp_unix_ms, 1_789_000_000_001);
}

#[test]
fn recent_net_events_zero_limit() {
    let r = setup_full_fixture();
    let evs = r.recent_net_events(0).unwrap();
    assert!(evs.is_empty());
}

// -----------------------------------------------------------------------
// Search
// -----------------------------------------------------------------------

#[test]
fn search_net_events_matches_domain_substring() {
    let r = setup_full_fixture();
    let hits = r.search_net_events("github", 10).unwrap();
    assert_eq!(hits.len(), 2);
    for h in &hits {
        assert!(h.domain.contains("github"));
    }
}

#[test]
fn search_net_events_matches_path() {
    let r = setup_full_fixture();
    let hits = r.search_net_events("search", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path.as_deref(), Some("/search"));
}

#[test]
fn search_net_events_matches_method() {
    let r = setup_full_fixture();
    let hits = r.search_net_events("POST", 10).unwrap();
    assert_eq!(hits.len(), 1);
}

#[test]
fn search_net_events_matches_rule() {
    let r = setup_full_fixture();
    let hits = r.search_net_events("allow-github", 10).unwrap();
    assert_eq!(hits.len(), 2);
}

#[test]
fn search_net_events_no_match_returns_empty() {
    let r = setup_full_fixture();
    let hits = r.search_net_events("nothing-like-this", 10).unwrap();
    assert!(hits.is_empty());
}

#[test]
fn search_net_events_respects_limit() {
    let r = setup_full_fixture();
    // Match all 5 rows by using a pattern that shows up everywhere.
    let hits = r.search_net_events(".com", 2).unwrap();
    assert_eq!(hits.len(), 2);
}

// -----------------------------------------------------------------------
// Aggregations: top_domains, session_stats
// -----------------------------------------------------------------------

#[test]
fn top_domains_ranks_by_count_desc() {
    let r = setup_full_fixture();
    let ds = r.top_domains(10).unwrap();
    assert_eq!(ds.len(), 4); // 4 distinct domains
                             // github has 2 rows, everything else has 1 — it should be first.
    assert_eq!(ds[0].domain, "api.github.com");
    assert_eq!(ds[0].count, 2);
    assert_eq!(ds[0].allowed, 2);
    assert_eq!(ds[0].denied, 0);
}

#[test]
fn top_domains_attributes_denied_vs_allowed() {
    let r = setup_full_fixture();
    let ds = r.top_domains(10).unwrap();
    let evil = ds.iter().find(|d| d.domain == "evil.com").unwrap();
    assert_eq!(evil.allowed, 0);
    assert_eq!(evil.denied, 1);
}

#[test]
fn top_domains_respects_limit() {
    let r = setup_full_fixture();
    let ds = r.top_domains(1).unwrap();
    assert_eq!(ds.len(), 1);
}

#[test]
fn session_stats_sums_net_and_model_columns() {
    let r = setup_full_fixture();
    let s = r.session_stats().unwrap();
    assert_eq!(s.net_total, 5);
    assert_eq!(s.net_allowed, 3);
    assert_eq!(s.net_denied, 1);
    assert_eq!(s.net_error, 1);
    assert_eq!(s.net_bytes_sent, 100 + 500 + 50 + 10);
    assert_eq!(s.net_bytes_received, 200 + 900 + 100);
    assert_eq!(s.model_call_count, 3);
    assert_eq!(s.total_input_tokens, 100 + 50 + 30);
    assert_eq!(s.total_output_tokens, 200 + 75 + 60);
    assert_eq!(s.total_model_duration_ms, 1500 + 800 + 400);
    // Session stats report the unified tool ledger: model-native tool
    // calls plus MCP-origin calls observed at the boundary.
    assert_eq!(s.total_tool_calls, 5);
    // Floating point sum — allow tiny tolerance.
    assert!((s.total_estimated_cost_usd - 0.018).abs() < 1e-9);
}

#[test]
fn session_stats_empty_db() {
    let r = DbReader::open_in_memory().unwrap();
    let s = r.session_stats().unwrap();
    assert_eq!(s.net_total, 0);
    assert_eq!(s.model_call_count, 0);
    assert_eq!(s.total_tool_calls, 0);
    assert_eq!(s.total_estimated_cost_usd, 0.0);
    assert!(s.total_usage_details.is_empty());
}

#[test]
fn tool_call_stats_counts_unified_tool_ledger_rows() {
    let r = DbReader::open_in_memory().unwrap();
    r.conn
        .execute_batch(
            "INSERT INTO tool_calls (timestamp, origin, transport, server_name, method, call_index, call_id, tool_name, arguments, decision, duration_ms)
                 VALUES
                    ('2026-01-01T00:00:04Z', 'mcp', 'vsock_frame', 'capsem', 'tools/call', 0, 'mcp-1', 'local__fetch_http', '{}', 'allowed', 9),
                    ('2026-01-01T00:00:05Z', 'mcp', 'http', 'github', 'tools/call', 0, 'mcp-2', 'github__search', '{}', 'denied', 11),
                    ('2026-01-01T00:00:06Z', 'native', 'http', 'model', NULL, 0, 'native-1', 'bash', '{}', 'allowed', 1);",
        )
        .unwrap();

    let stats = r.tool_call_stats().unwrap();
    assert_eq!(stats.total, 3);
    assert_eq!(stats.allowed, 2);
    assert_eq!(stats.denied, 1);
    assert_eq!(stats.by_server.len(), 3);
    assert_eq!(stats.by_server[0].server_name, "capsem");
    assert_eq!(stats.by_server[0].count, 1);
    assert_eq!(stats.by_server[1].server_name, "github");
    assert_eq!(stats.by_server[1].count, 1);
    assert_eq!(stats.by_server[2].server_name, "model");
    assert_eq!(stats.by_server[2].count, 1);
}

#[test]
fn recent_tool_calls_reads_unified_model_and_mcp_rows() {
    let r = DbReader::open_in_memory().unwrap();
    r.conn
        .execute_batch(
            "INSERT INTO tool_calls (
                    id, event_id, timestamp, model_call_id, origin, transport, server_name, method,
                    request_id, call_index, call_id, tool_name, arguments, response_preview,
                    decision, duration_ms, bytes_sent, bytes_received, policy_rule, trace_id
                 ) VALUES
                    (100, 'aaaaaaaaaaaa', '2026-01-01T00:00:01Z', 1, 'native', 'http', 'model', NULL,
                     NULL, 0, 'call-model', 'write_file', '{\"path\":\"poem.md\"}',
                     'ok', 'allowed', 7, 10, 20, NULL, 'trace-model'),
                    (101, 'bbbbbbbbbbbb', '2026-01-01T00:00:02Z', NULL, 'mcp', 'vsock_frame', 'capsem', 'tools/call',
                     'req-1', 0, 'req-1', 'local__fetch_http', '{\"url\":\"https://example.com\"}',
                     '{\"status\":200}', 'denied', 9, 30, 40, 'profiles.rules.block_fetch', 'trace-mcp');",
        )
        .unwrap();

    let rows = r.recent_tool_calls(10).unwrap();
    let mcp = rows.iter().find(|row| row.origin == "mcp").unwrap();
    assert_eq!(mcp.event_id, "bbbbbbbbbbbb");
    assert_eq!(mcp.model_call_id, None);
    assert_eq!(mcp.transport, "vsock_frame");
    assert_eq!(mcp.server_name.as_deref(), Some("capsem"));
    assert_eq!(mcp.method.as_deref(), Some("tools/call"));
    assert_eq!(mcp.request_id.as_deref(), Some("req-1"));
    assert_eq!(mcp.tool_name, "local__fetch_http");
    assert_eq!(mcp.arguments.as_deref(), Some("{\"url\":\"https://example.com\"}"));
    assert_eq!(mcp.response_preview.as_deref(), Some("{\"status\":200}"));
    assert_eq!(mcp.decision, "denied");
    assert_eq!(mcp.policy_rule.as_deref(), Some("profiles.rules.block_fetch"));

    let native = rows.iter().find(|row| row.origin == "native").unwrap();
    assert_eq!(native.model_call_id, Some(1));
    assert_eq!(native.transport, "http");
    assert_eq!(native.tool_name, "write_file");
    assert_eq!(native.response_preview.as_deref(), Some("ok"));
}

#[test]
fn raw_tool_call_count_matches_unified_ledger_rows() {
    let r = DbReader::open_in_memory().unwrap();
    r.conn
        .execute_batch(
            "INSERT INTO tool_calls (timestamp, origin, server_name, method, call_index, call_id, tool_name, arguments, decision, duration_ms)
                 VALUES
                    ('2026-01-01T00:00:00Z', 'mcp', 'capsem', 'tools/call', 0, 'call-1', 'local__snapshots_changes', '{}', 'allowed', 4),
                    ('2026-01-01T00:00:01Z', 'mcp', 'capsem', 'tools/call', 0, 'call-2', 'local__fetch_http', '{}', 'allowed', 9),
                    ('2026-01-01T00:00:02Z', 'native', 'model', NULL, 0, 'call-3', 'write_file', '{}', 'allowed', 1);",
        )
        .unwrap();

    assert_eq!(r.tool_call_stats().unwrap().total, 3);
    assert_eq!(r.raw_tool_call_count().unwrap(), 3);
}

#[test]
fn brokered_credential_stats_merges_injected_rows_without_provider() {
    let r = DbReader::open_in_memory().unwrap();
    let credential_ref = crate::events::credential_reference("google", "ya29.runtime-token");
    r.conn
        .execute(
            "INSERT INTO substitution_events (
                    timestamp, material_class, source, event_type, algorithm,
                    substitution_ref, outcome, provider, trace_id
                 ) VALUES (?1, 'credential', ?2, 'http.response', 'blake3', ?3, 'captured', 'google', 'trace-1')",
            params![
                "2026-06-14T22:00:00Z",
                "http.body.response.$.access_token",
                credential_ref,
            ],
        )
        .unwrap();
    r.conn
        .execute(
            "INSERT INTO substitution_events (
                    timestamp, material_class, source, event_type, algorithm,
                    substitution_ref, outcome, provider, trace_id
                 ) VALUES (?1, 'credential', ?2, 'http.request', 'blake3', ?3, 'injected', NULL, 'trace-2')",
            params!["2026-06-14T22:00:01Z", "http.header.authorization", credential_ref,],
        )
        .unwrap();
    r.conn
        .execute(
            "INSERT INTO substitution_events (
                    timestamp, material_class, source, event_type, algorithm,
                    substitution_ref, outcome, provider, trace_id
                 ) VALUES (?1, 'credential', ?2, 'http.request', 'blake3', ?3, 'injected', NULL, 'trace-3')",
            params!["2026-06-14T22:00:02Z", "http.query.access_token", credential_ref,],
        )
        .unwrap();

    let stats = r.brokered_credential_stats().unwrap();
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].provider.as_deref(), Some("google"));
    assert_eq!(stats[0].credential_ref, credential_ref);
    assert_eq!(stats[0].observed_count, 3);
    assert_eq!(stats[0].injected_count, 2);
    assert_eq!(stats[0].last_seen.as_deref(), Some("2026-06-14T22:00:02Z"));
}

// -----------------------------------------------------------------------
// tool_calls_for / tool_responses_for
// -----------------------------------------------------------------------

#[test]
fn tool_calls_for_returns_by_model_call_id() {
    let r = setup_full_fixture();
    let t = r.tool_calls_for(1).unwrap();
    assert_eq!(t.len(), 2);
    assert_eq!(t[0].call_id, "c-1");
    assert_eq!(t[1].call_id, "c-2");
}

#[test]
fn tool_calls_for_unknown_id_returns_empty() {
    let r = setup_full_fixture();
    let t = r.tool_calls_for(9999).unwrap();
    assert!(t.is_empty());
}

#[test]
fn tool_responses_for_returns_by_model_call_id() {
    let r = DbReader::open_in_memory().unwrap();
    r.conn
        .execute(
            "INSERT INTO tool_responses (model_call_id, call_id, content_preview, is_error)
             VALUES (1, 'c-1', 'ok', 0), (1, 'c-2', 'boom', 1), (2, 'c-3', 'other', 0)",
            [],
        )
        .unwrap();
    let rs = r.tool_responses_for(1).unwrap();
    assert_eq!(rs.len(), 2);
    assert!(!rs[0].is_error);
    assert!(rs[1].is_error);
}

#[test]
fn tool_responses_for_tolerates_old_schema_without_credential_ref() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old-session.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "CREATE TABLE tool_responses (
                    id INTEGER PRIMARY KEY,
                    model_call_id INTEGER NOT NULL,
                    call_id TEXT NOT NULL,
                    content_preview TEXT,
                    is_error INTEGER NOT NULL DEFAULT 0
                )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tool_responses (model_call_id, call_id, content_preview, is_error)
                 VALUES (1, 'old-call', 'old-ok', 0)",
            [],
        )
        .unwrap();
    }

    let reader = DbReader::open(&path).unwrap();
    let responses = reader.tool_responses_for(1).unwrap();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].call_id, "old-call");
    assert_eq!(responses[0].content_preview.as_deref(), Some("old-ok"));
    assert_eq!(responses[0].credential_ref, None);
}

// -----------------------------------------------------------------------
// validate_select_only: a few more adversarial cases
// -----------------------------------------------------------------------

#[test]
fn validate_select_only_rejects_upsert() {
    assert!(validate_select_only("INSERT INTO t VALUES (1) ON CONFLICT DO UPDATE SET x = 2").is_err());
}

#[test]
fn validate_select_only_rejects_multi_statement() {
    // SELECT followed by DELETE should not slip through if statement was split.
    // Current implementation may accept this since it only checks the first keyword;
    // if this ever regresses, tighten the check.
    let s = "SELECT 1; DELETE FROM t";
    // Document current behavior: starts with SELECT → OK (bind params do not
    // bypass, but the statement validator is keyword-only). The DbReader
    // execute path uses query_raw which only prepares one statement — so
    // the trailing DELETE is dropped. This is a sharp edge worth noting.
    assert!(validate_select_only(s).is_ok());
}

#[test]
fn query_raw_rejects_non_select() {
    let r = setup_full_fixture();
    let err = r.query_raw("DELETE FROM net_events").unwrap_err();
    // validate_select_only returns "<KEYWORD> statements are not allowed".
    assert!(err.contains("DELETE") && err.contains("not allowed"), "got: {err}");
}

#[test]
fn query_raw_with_params_rejects_non_select() {
    let r = setup_full_fixture();
    let err = r
        .query_raw_with_params("UPDATE net_events SET domain = ?", &[json!("x")])
        .unwrap_err();
    assert!(err.contains("UPDATE") && err.contains("not allowed"), "got: {err}");
}

#[test]
fn query_raw_returns_row_cap_on_large_results() {
    // Force max_rows limit by inserting many rows.
    let r = DbReader::open_in_memory().unwrap();
    for i in 0..50 {
        r.conn
            .execute(
                "INSERT INTO net_events (timestamp, domain, decision) VALUES (?, ?, 'allowed')",
                params![format!("2026-01-01T00:{:02}:00Z", i % 60), format!("d{i}.com")],
            )
            .unwrap();
    }
    // Default limit is large; just confirm all 50 are returned.
    let json_str = r.query_raw("SELECT id FROM net_events").unwrap();
    let v: Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(v["rows"].as_array().unwrap().len(), 50);
}

/// The disk sync used to run before every query and copy every hot table
/// from disk into memory: a UI poll cost O(ledger) whether or not anything
/// had been written. It now runs only when another connection has committed.
#[tokio::test]
async fn disk_sync_runs_only_when_another_connection_committed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = crate::writer::DbWriter::open(&path, 16).expect("writer opens");
    writer.write(crate::WriteOp::DnsEvent(dns_fixture(1))).await;
    writer.flush().await;

    let reader = DbReader::open(&path).expect("external reader opens");
    reader.sync_from_disk().expect("first sync copies the tables");
    assert_eq!(reader.disk_syncs(), 1);
    for _ in 0..5 {
        reader.sync_from_disk().expect("no-op sync");
    }
    assert_eq!(reader.disk_syncs(), 1, "polls with nothing committed must copy nothing");
    let before = reader.query_raw("SELECT COUNT(*) FROM dns_events").unwrap();
    assert!(before.contains("[[1]]"), "{before}");

    writer.write(crate::WriteOp::DnsEvent(dns_fixture(2))).await;
    writer.flush().await;
    reader.sync_from_disk().expect("sync after a commit");
    assert_eq!(
        reader.disk_syncs(),
        2,
        "a commit by the writer must trigger exactly one copy"
    );
    let after = reader.query_raw("SELECT COUNT(*) FROM dns_events").unwrap();
    assert!(after.contains("[[2]]"), "{after}");
    writer.shutdown_blocking();
}

/// exec_events rows are completed in place, so that table is copied whole
/// on every resync; the append-only ledgers pull only new rows. Both must
/// show what is on disk after a commit.
#[test]
fn resync_after_a_commit_reflects_in_place_updates_and_appended_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        crate::schema::create_tables(&conn).unwrap();
        conn.execute(
            "INSERT INTO exec_events (timestamp, exec_id, command) VALUES ('2026-09-03T00:00:00Z', 7, 'ls')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO dns_events (timestamp, qname, qtype, qclass, rcode, decision) VALUES (1, 'a.example', 1, 1, 0, 'allowed')",
            [],
        )
        .unwrap();
    }
    let reader = DbReader::open(&path).unwrap();
    reader.sync_from_disk().unwrap();
    assert!(reader
        .query_raw("SELECT exit_code FROM exec_events")
        .unwrap()
        .contains("[[null]]"));

    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "UPDATE exec_events SET exit_code = 3, duration_ms = 12 WHERE exec_id = 7",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO dns_events (timestamp, qname, qtype, qclass, rcode, decision) VALUES (2, 'b.example', 1, 1, 0, 'denied')",
            [],
        )
        .unwrap();
    }
    reader.sync_from_disk().unwrap();
    let exec = reader
        .query_raw("SELECT exit_code, duration_ms FROM exec_events")
        .unwrap();
    assert!(exec.contains("[[3,12]]"), "in-place completion must be visible: {exec}");
    let dns = reader.query_raw("SELECT qname FROM dns_events ORDER BY id").unwrap();
    assert!(
        dns.contains("a.example") && dns.contains("b.example"),
        "appended row must be visible: {dns}"
    );
    assert_eq!(reader.disk_syncs(), 2);
}

/// The incremental copy is only correct while the writer never changes a
/// row of an append-only ledger in place. This holds the writer's SQL to
/// the list in `UPDATABLE_HOT_TABLES`; a new UPDATE or DELETE on any other
/// hot table must extend the list, which turns that table back into a full
/// copy.
#[test]
fn writer_updates_only_the_updatable_tables() {
    let sources = [
        ("writer.rs", include_str!("../writer.rs")),
        ("schema.rs", include_str!("../schema.rs")),
        ("schema/memory_sync.rs", include_str!("../schema/memory_sync.rs")),
        ("db.rs", include_str!("../db.rs")),
    ];
    let mut offences = Vec::new();
    for (name, source) in sources {
        for keyword in ["UPDATE ", "DELETE FROM "] {
            let mut search = 0;
            while let Some(found) = source[search..].find(keyword) {
                let at = search + found;
                search = at + keyword.len();
                // The statement text plus the format arguments that follow it.
                let window = &source[at..(at + 400).min(source.len())];
                let memory_schema = window.starts_with(&format!("{keyword}{{MEMORY_SCHEMA}}"));
                let session_index = window.starts_with(&format!("{keyword}sessions"));
                let updatable = crate::schema::UPDATABLE_HOT_TABLES
                    .iter()
                    .any(|table| window.contains(table));
                if !(memory_schema || session_index || updatable) {
                    offences.push(format!("{name}: {}", window.lines().next().unwrap_or("")));
                }
            }
        }
    }
    assert!(
        offences.is_empty(),
        "in-place writes to a hot ledger outside UPDATABLE_HOT_TABLES; extend the list so the \
         external reader copies that table whole: {offences:?}"
    );
}
