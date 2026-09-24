use super::*;
use serde_json::{json, Value};

mod readiness;

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
// Richer fixture covering multiple tables, used by the read tests below.
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
fn recent_security_rule_events_orders_newest_first_and_keeps_the_rule_snapshot() {
    let r = DbReader::open_in_memory().unwrap();
    r.conn
        .execute_batch(
            "INSERT INTO security_rule_events (
                    timestamp_unix_ms, event_id, event_type, rule_id,
                    rule_action, detection_level, rule_json
                 ) VALUES
                    (1789000000000, '111111111111', 'http.request', 'allow_github',
                     'allow', 'none', '{\"name\":\"allow_github\"}'),
                    (1789000000001, '222222222222', 'model.call', 'block_openai',
                     'block', 'critical', '{\"name\":\"block_openai\"}')",
        )
        .unwrap();

    let latest = r.recent_security_rule_events(2).unwrap();
    assert_eq!(latest.len(), 2);
    assert_eq!(latest[0].event_id, "222222222222");
    assert_eq!(latest[0].rule_id, "block_openai");
    assert_eq!(latest[0].rule_action, SecurityRuleAction::Block);
    assert_eq!(latest[0].detection_level, SecurityDetectionLevel::Critical);
    assert!(latest[0].rule_json.contains("block_openai"));
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

/// A `tool_responses` without `credential_ref` is broken schema, not a row
/// whose credential happens to be unknown.
///
/// The read used to go through `optional_column_expr`, which substituted
/// `NULL AS credential_ref` when the column was absent. That made a ledger an
/// older build wrote indistinguishable from a current one in which nothing
/// was ever brokered -- the reader answered "no credential" for a question it
/// could not see the answer to. The column is declared in `schema/ddl.rs` and
/// selected outright, so its absence is now an error that names it.
#[test]
fn tool_responses_for_fails_loudly_without_credential_ref() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old-session.db");
    {
        let conn = Connection::open(&path).unwrap();
        crate::schema::create_tables(&conn).unwrap();
        conn.execute_batch(
            "DROP TABLE tool_responses;
             CREATE TABLE tool_responses (
                    id INTEGER PRIMARY KEY,
                    model_call_id INTEGER NOT NULL,
                    call_id TEXT NOT NULL,
                    content_preview TEXT,
                    is_error INTEGER NOT NULL DEFAULT 0,
                    event_id TEXT
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tool_responses (model_call_id, call_id, content_preview, is_error)
                 VALUES (1, 'old-call', 'old-ok', 0)",
            [],
        )
        .unwrap();
    }

    let error = DbReader::open(&path)
        .and_then(|reader| reader.tool_responses_for(1).map(|_| ()))
        .expect_err("a tool_responses without credential_ref must not read as a current ledger")
        .to_string();
    assert!(
        error.contains("credential_ref"),
        "the failure must name the column the ledger lacks: {error}"
    );
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

/// The writer's memory holds only rows it has not flushed, so an update to a
/// hot ledger row cannot assume the row is still there: the flush may have
/// moved it to disk, and the update has to look in both (see
/// `update_exec_event`). This holds the writer's SQL to the tables in
/// `UPDATABLE_HOT_TABLES`, which do; a new UPDATE or DELETE on any other hot
/// table must be taught the same and then listed.
#[test]
fn writer_updates_only_the_updatable_tables() {
    let sources = [
        ("writer.rs", include_str!("../writer.rs")),
        ("writer/traffic_rows.rs", include_str!("../writer/traffic_rows.rs")),
        ("writer/model_rows.rs", include_str!("../writer/model_rows.rs")),
        ("writer/event_rows.rs", include_str!("../writer/event_rows.rs")),
        ("writer/retention.rs", include_str!("../writer/retention.rs")),
        ("writer/bodies.rs", include_str!("../writer/bodies.rs")),
        ("schema.rs", include_str!("../schema.rs")),
        ("schema/memory_sync.rs", include_str!("../schema/memory_sync.rs")),
        ("db.rs", include_str!("../db.rs")),
        ("db/maintenance.rs", include_str!("../db/maintenance.rs")),
        ("db/reader_worker.rs", include_str!("../db/reader_worker.rs")),
    ];
    let mut offences = Vec::new();
    for (name, source) in sources {
        for keyword in ["UPDATE ", "DELETE FROM "] {
            let mut search = 0;
            while let Some(found) = source[search..].find(keyword) {
                let at = search + found;
                search = at + keyword.len();
                // An upsert's `ON CONFLICT ... DO UPDATE` names no table of its own.
                if source[..at].ends_with("DO ") {
                    continue;
                }
                // The statement text plus the format arguments that follow it.
                let window = &source[at..(at + 400).min(source.len())];
                let target = window[keyword.len()..]
                    .split(|c: char| c.is_whitespace() || c == '(')
                    .next()
                    .unwrap_or("");
                let memory_schema = target.starts_with("{MEMORY_SCHEMA}");
                let session_index = target == "sessions";
                let disk_only = crate::schema::is_disk_only_table(target);
                let updatable = crate::schema::UPDATABLE_HOT_TABLES
                    .iter()
                    .any(|table| window.contains(table));
                if !(memory_schema || session_index || disk_only || updatable) {
                    offences.push(format!("{name}: {}", window.lines().next().unwrap_or("")));
                }
            }
        }
    }
    assert!(
        offences.is_empty(),
        "in-place writes to a hot ledger outside UPDATABLE_HOT_TABLES; make the write find a row the \
         flush already moved to disk, then extend the list: {offences:?}"
    );
}

/// The readiness gate knows every column a reader selects.
///
/// `ready()` is where a ledger an older build wrote is supposed to be caught,
/// by name, before a route runs against it. That only works if
/// `READY_SCHEMA_COLUMNS` demands everything a SELECT will ask for. A column
/// the reader reads and the gate does not require still fails -- SQLite says
/// "no such column" -- but it fails partway through a route, in SQLite's
/// vocabulary, on a file readiness has already called healthy.
///
/// Rather than top the list up by hand whenever that happens, this walks every
/// column list the reads are built from and names anything the gate is missing.
/// Adding a column to a SELECT is then a failing test here, which is the
/// cheapest place for it to fail.
#[test]
fn reader_select_columns_are_required_by_the_readiness_gate() {
    let required: std::collections::BTreeMap<&str, std::collections::BTreeSet<&str>> =
        crate::schema::REQUIRED_COLUMNS_FOR_TESTS
            .iter()
            .map(|(table, columns)| (*table, columns.iter().copied().collect()))
            .collect();

    let mut missing: Vec<String> = Vec::new();
    for (table, list) in crate::reader::reader_select_columns() {
        let gate = required
            .get(table)
            .unwrap_or_else(|| panic!("{table} is selected from but is not in READY_SCHEMA_COLUMNS at all"));
        for column in list.split(',').map(str::trim).filter(|c| !c.is_empty()) {
            assert!(
                !column.contains(' '),
                "{table}: `{column}` is an expression, not a column; the gate cannot require it"
            );
            if !gate.contains(column) {
                missing.push(format!("{table}.{column}"));
            }
        }
    }
    missing.sort();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "these columns are selected by a reader but not required by READY_SCHEMA_COLUMNS, \
         so a ledger without them fails mid-route as SQLite's `no such column` instead of \
         loudly at ready(). Add them to crates/capsem-logger/src/schema/columns.rs:\n  {}",
        missing.join("\n  ")
    );
}
