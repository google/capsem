//! Route statements read a bounded window of the ledger, never all of it (#223).
//!
//! The stats and triage views poll these statements. One that sorts every row
//! of a table to keep the newest two hundred costs a little more on every poll
//! for the life of the session, and nothing about a small test ledger shows it.
//!
//! Two checks per statement, because neither is enough alone:
//!
//! - **The plan**, read through the route's own handle with `EXPLAIN QUERY
//!   PLAN`: no `USE TEMP B-TREE FOR ORDER BY` over a table, and every table the
//!   statement joins or probes searched by an index or the rowid.
//! - **The work.** SQLite prints `SCAN t` both for a reverse rowid walk that a
//!   `LIMIT` stops after n rows and for a walk over the whole table, so the plan
//!   cannot tell a bounded window from an O(ledger) read. The VM step count of
//!   the same statement over a ledger four times larger can: a window costs the
//!   same, a full scan or sort costs four times more.

use super::*;
use crate::ledger_routes::bodies::{STATS_DETAIL_BODY_BLOBS_SQL, STATS_DETAIL_PROCESS_EVENTS_LIMIT};
use crate::ledger_routes::history::{page_sql, search_total_sql};
use crate::ledger_routes::security::{SECURITY_LATEST_LIMIT, SECURITY_LATEST_SQL};
use crate::ledger_routes::stats_detail::interactions::{MODEL_ITEMS_SQL, TOOL_CALLS_SQL};
use crate::ledger_routes::stats_detail::{
    STATS_DETAIL_AUDIT_EVENTS_SQL, STATS_DETAIL_CREDENTIAL_EVENTS_SQL, STATS_DETAIL_DNS_EVENTS_SQL,
    STATS_DETAIL_FILE_EVENTS_SQL, STATS_DETAIL_HTTP_EVENTS_SQL, STATS_DETAIL_MODEL_EVENTS_SQL,
    STATS_DETAIL_PROCESS_EVENTS_SQL, STATS_DETAIL_TOOL_EVENTS_SQL,
};
use crate::ledger_routes::timeline::timeline_sql;
use crate::session_triage_statements;

/// Rows per table in the small ledger; the large one holds four times as many.
/// Both are past every window a route reads -- the widest is the security
/// latest list, at 2000 -- so both runs read a full window.
const SMALL_LEDGER_ROWS: i64 = 2_500;
const TRIAGE_LIMIT: usize = 20;
const HISTORY_PAGE: i64 = 50;
const TIMELINE_LIMIT: i64 = 200;
/// A timeline cutoff a fifth of the way into the small ledger.
const TIMELINE_CUTOFF: &str = "2026-09-10T00000500";

/// One statement a route runs, with what its plan must show.
///
/// This is the registry of every SQL statement the service's ledger routes
/// execute. `tests/citadel/test_ledger_route_statements.py` fails when a
/// service source file holds SQL this registry does not reach, so a new route
/// read cannot skip the plan check below.
struct RouteStatement {
    name: &'static str,
    sql: String,
    params: Vec<serde_json::Value>,
    /// Plan lines that must appear: the index or rowid probes that keep the
    /// statement off a table scan.
    searches: &'static [&'static str],
    /// Whether the statement's final `ORDER BY` sorts its own bounded result.
    /// Allowed only where that result is a union of fixed windows; the step
    /// check below is what proves it is bounded.
    sorts_bounded_result: bool,
    /// Why the statement may cost more as the ledger grows. `None` is the
    /// rule: a window that costs the same at any size. A scan is declared
    /// here with its reason or it fails.
    scans: Option<&'static str>,
}

impl RouteStatement {
    fn window(name: &'static str, sql: impl Into<String>, params: Vec<serde_json::Value>) -> Self {
        Self {
            name,
            sql: sql.into(),
            params,
            searches: &[],
            sorts_bounded_result: false,
            scans: None,
        }
    }

    fn probing(mut self, searches: &'static [&'static str]) -> Self {
        self.searches = searches;
        self
    }

    fn merging_windows(mut self) -> Self {
        self.sorts_bounded_result = true;
        self
    }

    fn scanning(mut self, reason: &'static str) -> Self {
        self.scans = Some(reason);
        self
    }
}

/// Each history arm walks its table's timestamp index newest first.
fn history_probes(layer: api::HistoryLayerFilter) -> &'static [&'static str] {
    match layer {
        api::HistoryLayerFilter::All => &[
            "SCAN exec_events USING INDEX idx_exec_events_timestamp",
            "SCAN audit_events USING INDEX idx_audit_events_timestamp",
        ],
        api::HistoryLayerFilter::Exec => &["SCAN exec_events USING INDEX idx_exec_events_timestamp"],
        api::HistoryLayerFilter::Audit => &["SCAN audit_events USING INDEX idx_audit_events_timestamp"],
    }
}

const SUBSTRING_SEARCH: &str = "a literal substring search reads rows until it has a page of matches; \
     the ledger has no full-text index yet";

fn route_statements() -> Vec<RouteStatement> {
    use api::HistoryLayerFilter::{All, Audit, Exec};
    let mut statements = vec![
        RouteStatement::window("stats_detail.model_events", STATS_DETAIL_MODEL_EVENTS_SQL, vec![]),
        RouteStatement::window("stats_detail.tool_events", STATS_DETAIL_TOOL_EVENTS_SQL, vec![]).probing(&[
            "SEARCH mc USING INTEGER PRIMARY KEY (rowid=?)",
            "idx_tool_responses_call_id (call_id=?)",
        ]),
        RouteStatement::window("stats_detail.http_events", STATS_DETAIL_HTTP_EVENTS_SQL, vec![]),
        RouteStatement::window("stats_detail.dns_events", STATS_DETAIL_DNS_EVENTS_SQL, vec![]),
        RouteStatement::window("stats_detail.file_events", STATS_DETAIL_FILE_EVENTS_SQL, vec![]),
        RouteStatement::window("stats_detail.process_events", STATS_DETAIL_PROCESS_EVENTS_SQL, vec![]),
        RouteStatement::window("stats_detail.audit_events", STATS_DETAIL_AUDIT_EVENTS_SQL, vec![]),
        RouteStatement::window(
            "stats_detail.credential_events",
            STATS_DETAIL_CREDENTIAL_EVENTS_SQL,
            vec![],
        ),
        RouteStatement::window(
            "stats_detail.body_blobs",
            STATS_DETAIL_BODY_BLOBS_SQL,
            vec![json!(STATS_DETAIL_PROCESS_EVENTS_LIMIT)],
        )
        .probing(&["sqlite_autoindex_event_body_blobs_1 (event_id=?)"])
        .merging_windows(),
        RouteStatement::window("interactions.model_items", MODEL_ITEMS_SQL, vec![])
            .probing(&["SEARCH mc USING INTEGER PRIMARY KEY (rowid=?)"]),
        RouteStatement::window("interactions.tool_calls", TOOL_CALLS_SQL, vec![])
            .probing(&["SEARCH mc USING INTEGER PRIMARY KEY (rowid=?)"]),
        RouteStatement::window(
            "security.latest",
            SECURITY_LATEST_SQL,
            vec![json!(SECURITY_LATEST_LIMIT)],
        ),
    ];
    for (name, layer) in [
        ("history.page.all", All),
        ("history.page.exec", Exec),
        ("history.page.audit", Audit),
    ] {
        let page = vec![
            serde_json::Value::Null,
            json!(HISTORY_PAGE),
            json!(HISTORY_PAGE),
            json!(0),
        ];
        // Each page re-sorts its own `offset + limit` rows after the arms.
        statements.push(
            RouteStatement::window(name, page_sql(layer, false), page)
                .probing(history_probes(layer))
                .merging_windows(),
        );
    }
    for (name, layer) in [
        ("history.search_page.all", All),
        ("history.search_page.exec", Exec),
        ("history.search_page.audit", Audit),
    ] {
        let page = vec![json!("false"), json!(HISTORY_PAGE), json!(HISTORY_PAGE), json!(0)];
        statements.push(
            RouteStatement::window(name, page_sql(layer, true), page)
                .probing(history_probes(layer))
                .merging_windows()
                .scanning(SUBSTRING_SEARCH),
        );
    }
    for (name, layer) in [
        ("history.search_total.all", All),
        ("history.search_total.exec", Exec),
        ("history.search_total.audit", Audit),
    ] {
        statements.push(
            RouteStatement::window(name, search_total_sql(layer), vec![json!("false")])
                .scanning("a search total counts every match; see AGGREGATE_DEBT in test_ledger_counter_boundary.py"),
        );
    }
    let layers = [
        api::TimelineLayer::Exec,
        api::TimelineLayer::Tool,
        api::TimelineLayer::Net,
        api::TimelineLayer::Fs,
        api::TimelineLayer::Model,
    ];
    for (name, cutoff, trace) in [
        ("timeline.from_start", "", serde_json::Value::Null),
        ("timeline.since", TIMELINE_CUTOFF, serde_json::Value::Null),
        ("timeline.since_in_trace", TIMELINE_CUTOFF, json!("trace-1")),
    ] {
        statements.push(
            RouteStatement::window(
                name,
                timeline_sql(&layers),
                vec![json!(TIMELINE_LIMIT), json!(cutoff), json!(cutoff), trace],
            )
            .probing(&[
                "idx_exec_events_timestamp (timestamp>?)",
                "idx_tool_calls_timestamp (timestamp>?)",
                "idx_net_events_timestamp (timestamp>?)",
                "idx_fs_events_timestamp (timestamp>?)",
                "idx_model_calls_timestamp (timestamp>?)",
            ])
            .merging_windows(),
        );
    }
    for (name, sql) in session_triage_statements(TRIAGE_LIMIT) {
        statements.push(RouteStatement::window(name, sql, vec![]));
    }
    statements
}

/// A ledger the process writer created, filled with `rows` rows per table.
///
/// Every tool call and exec is a failure and every net event a denial, so the
/// triage filters match everything and the step count measures the ordering,
/// not how rare a match is. Every seventh tool call is an `mcp_proxy` echo the
/// stats list shows and triage does not count, and every fifth has no timestamp
/// of its own, so the joins and the origin filter have something to do.
fn seed_ledger(session_dir: &std::path::Path, rows: i64) -> PathBuf {
    std::fs::create_dir_all(session_dir).unwrap();
    let db_path = session_dir.join("session.db");
    capsem_logger::DbWriter::open(&db_path, 1).unwrap().shutdown_blocking();
    let connection = rusqlite::Connection::open(&db_path).unwrap();
    connection
        .execute_batch(&format!(
            r#"
        BEGIN;
        WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {rows})
        INSERT INTO model_calls(id, event_id, timestamp, provider, method, path, duration_ms)
        SELECT i, printf('a%011x', i), printf('2026-09-10T%08d', i), 'openai', 'POST', '/v1/responses', 7
        FROM n;
        WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {rows})
        INSERT INTO tool_calls(id, event_id, timestamp, model_call_id, call_index, call_id, tool_name,
                               origin, server_name, arguments, response_preview, decision, error_message)
        SELECT i, printf('b%011x', i),
               CASE WHEN i % 5 = 0 THEN '' ELSE printf('2026-09-10T%08d', i) END,
               CASE WHEN i % 11 = 0 THEN {rows} + i ELSE i END,
               0, printf('call-%d', i), 'lookup',
               CASE WHEN i % 7 = 0 THEN 'mcp_proxy' WHEN i % 2 = 0 THEN 'mcp' ELSE 'native' END,
               'tools', '{{}}', NULL, 'error', printf('failed %d', i)
        FROM n;
        WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {rows})
        INSERT INTO tool_responses(model_call_id, call_id, content_preview)
        SELECT i, printf('call-%d', i), printf('result %d', i) FROM n;
        WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {rows})
        INSERT INTO net_events(id, event_id, timestamp, domain, decision, status_code)
        SELECT i, printf('c%011x', i), printf('2026-09-10T%08d', i), 'evil.test', 'denied', 403 FROM n;
        WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {rows})
        INSERT INTO exec_events(id, event_id, timestamp, exec_id, command, exit_code)
        SELECT i, printf('d%011x', i), printf('2026-09-10T%08d', i), i, 'false', 1 FROM n;
        WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {rows})
        INSERT INTO audit_events(id, event_id, timestamp, pid, ppid, uid, exe, argv, exit_code)
        SELECT i, printf('e%011x', i), printf('2026-09-10T%08d', i), i, 1, 0, '/bin/false', 'false', 1 FROM n;
        WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {rows})
        INSERT INTO fs_events(id, event_id, timestamp, action, path)
        SELECT i, printf('1%011x', i), printf('2026-09-10T%08d', i), 'modified', printf('/w/%d', i) FROM n;
        WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {rows})
        INSERT INTO dns_events(id, event_id, timestamp, qname, qtype, qclass, rcode, decision)
        SELECT i, printf('2%011x', i), printf('2026-09-10T%08d', i), 'evil.test', 1, 1, 5, 'denied' FROM n;
        WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {rows})
        INSERT INTO substitution_events(id, event_id, timestamp, material_class, source, algorithm,
                                        substitution_ref, outcome)
        SELECT i, printf('3%011x', i), printf('2026-09-10T%08d', i), 'credential', 'env', 'blake3',
               'credential:blake3:' || printf('%064x', i), 'captured' FROM n;
        WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {rows})
        INSERT INTO model_items(id, event_id, model_call_id, timestamp, provider, path, kind, item_index,
                                call_id, content_hash)
        SELECT i, printf('4%011x', i), i, printf('2026-09-10T%08d', i), 'openai', '/v1/responses',
               'tool_response', 0, printf('call-%d', i), 'blake3:' || printf('%064x', i) FROM n;
        WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {rows})
        INSERT INTO security_rule_events(id, timestamp_unix_ms, event_id, event_type, rule_id, rule_action, rule_json)
        SELECT i, i, printf('c%011x', i), 'http.request', 'rule', 'allow', '{{}}' FROM n;
        INSERT INTO body_blocks(block_offset, raw_len, disk_len, sealed_at) VALUES (80, 1, 1, '2026-09-10');
        WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {rows}),
             sources(prefix, event_type, source_table, direction) AS (
                 VALUES ('a', 'model.call', 'model_calls', 'response'),
                        ('b', 'mcp.tool_call', 'tool_calls', 'response'),
                        ('c', 'http.request', 'net_events', 'request'),
                        ('c', 'security.rule', 'security_rule_events', 'payload'),
                        ('c', 'security.decision', 'security_decision_events', 'payload'),
                        ('d', 'process.exec', 'exec_events', 'stdout'))
        INSERT INTO event_body_blobs(event_id, event_type, source_table, direction, original_bytes,
                                     stored_bytes, truncated, body_hash, block_offset, body_offset,
                                     body_len, created_at)
        SELECT printf('%s%011x', prefix, i), event_type, source_table, direction, 1, 1, 0,
               'blake3:' || printf('%064x', i), 80, 0, 1, '2026-09-10'
        FROM n, sources;
        COMMIT;
        "#
        ))
        .unwrap();
    db_path
}

/// The `detail` column of each `EXPLAIN QUERY PLAN` row, read through the
/// route's own external-reader handle.
async fn query_plan(db: &capsem_logger::DbHandle, statement: &RouteStatement) -> Vec<String> {
    let raw = db
        .query(&format!("EXPLAIN QUERY PLAN {}", statement.sql), &statement.params)
        .await
        .unwrap_or_else(|error| panic!("{}: EXPLAIN QUERY PLAN failed: {error}", statement.name));
    let plan: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let detail = plan["columns"]
        .as_array()
        .unwrap()
        .iter()
        .position(|column| column == "detail")
        .expect("EXPLAIN QUERY PLAN has a detail column");
    plan["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row[detail].as_str().unwrap().to_string())
        .collect()
}

fn sql_value(value: &serde_json::Value) -> rusqlite::types::Value {
    match value {
        serde_json::Value::Null => rusqlite::types::Value::Null,
        serde_json::Value::Number(number) => rusqlite::types::Value::Integer(number.as_i64().unwrap()),
        serde_json::Value::String(text) => rusqlite::types::Value::Text(text.clone()),
        other => panic!("route statement parameter {other} is not a SQL scalar"),
    }
}

/// SQLite's VM step count for running `statement` to completion.
///
/// Test-only direct read: the handle does not expose statement counters, and
/// this is the one measure that separates a bounded rowid walk from a full one.
fn vm_steps(db_path: &std::path::Path, statement: &RouteStatement) -> i32 {
    let connection =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let mut prepared = connection.prepare(&statement.sql).unwrap();
    let mut rows = prepared
        .query(rusqlite::params_from_iter(statement.params.iter().map(sql_value)))
        .unwrap();
    while rows.next().unwrap().is_some() {}
    drop(rows);
    prepared.get_status(rusqlite::StatementStatus::VmStep)
}

#[tokio::test]
async fn route_statements_read_a_window_not_the_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let small = seed_ledger(&dir.path().join("small"), SMALL_LEDGER_ROWS);
    let large = seed_ledger(&dir.path().join("large"), SMALL_LEDGER_ROWS * 4);
    let db = capsem_logger::DbHandle::open_external_reader(&small).unwrap();
    db.ready().await.unwrap();

    let mut failures = Vec::new();
    for statement in route_statements() {
        let plan = query_plan(&db, &statement).await;
        let shown = plan.join("\n");
        if !statement.sorts_bounded_result && plan.iter().any(|line| line.contains("USE TEMP B-TREE FOR ORDER BY")) {
            failures.push(format!("{} sorts in a temp B-tree:\n{shown}", statement.name));
        }
        for search in statement.searches {
            if !plan.iter().any(|line| line.contains(search)) {
                failures.push(format!("{} does not probe `{search}`:\n{shown}", statement.name));
            }
        }
        if statement.scans.is_some() {
            continue;
        }
        let (small_steps, large_steps) = (vm_steps(&small, &statement), vm_steps(&large, &statement));
        // A window reads the same rows at any ledger size; a sort or a full
        // scan reads four times as many. The margin absorbs b-tree depth.
        if f64::from(large_steps) > f64::from(small_steps) * 1.25 {
            failures.push(format!(
                "{} grows with the ledger: {small_steps} VM steps at {SMALL_LEDGER_ROWS} rows, \
                 {large_steps} at four times that:\n{shown}",
                statement.name
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// The rows `seed_ledger` makes visible to the stats tool list, newest first:
/// every origin it seeds is one the list shows.
fn listed_tool_calls(rows: i64) -> Vec<i64> {
    (1..=rows).rev().take(200).collect()
}

#[tokio::test]
async fn stats_detail_lists_the_newest_tool_calls_with_their_joins() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session = state.run_dir.join("sessions/plans");
    seed_ledger(&session, SMALL_LEDGER_ROWS);
    insert_fake_instance_with_session_dir(&state, "plans", std::process::id(), session);
    let app = build_service_router(Arc::clone(&state));
    let (status, value) = route_request(app, axum::http::Method::GET, "/vms/plans/stats/detail", None).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let detail: api::VmStatsDetailResponse = serde_json::from_value(value).unwrap();

    let expected = listed_tool_calls(SMALL_LEDGER_ROWS);
    let listed: Vec<String> = detail.tool_events.iter().map(|row| row.event_id.clone()).collect();
    let expected_ids: Vec<String> = expected.iter().map(|i| format!("b{i:011x}")).collect();
    assert_eq!(listed, expected_ids, "newest 200 listed tool calls, newest first");

    for (row, i) in detail.tool_events.iter().zip(&expected) {
        let own_timestamp = i % 5 != 0;
        let parent_missing = i % 11 == 0;
        let expected_timestamp = if own_timestamp || !parent_missing {
            format!("2026-09-10T{i:08}")
        } else {
            String::new()
        };
        assert_eq!(row.timestamp.clone().unwrap_or_default(), expected_timestamp, "{row:?}");
        assert_eq!(row.model_parent_missing, parent_missing, "{row:?}");
        assert_eq!(
            row.response_preview.as_deref(),
            Some(format!("result {i}").as_str()),
            "{row:?}"
        );
        assert_eq!(
            serde_json::to_value(row.source).unwrap(),
            match i {
                i if i % 7 == 0 => "mcp_proxy",
                i if i % 2 == 0 => "mcp",
                _ => "native",
            },
            "{row:?}"
        );
    }

    // Body metadata annotates exactly the listed windows: the newest rows of
    // each list, nothing older, and nothing from a decision.
    let net_newest = format!("c{SMALL_LEDGER_ROWS:011x}");
    let net_bodies = &detail.body_blobs[&net_newest];
    let tables: Vec<&str> = net_bodies.iter().map(|body| body.source_table.as_str()).collect();
    assert_eq!(tables, ["security_rule_events", "net_events"], "{net_bodies:?}");
    for i in &expected {
        assert!(
            detail.body_blobs.contains_key(&format!("b{i:011x}")),
            "tool call {i} body"
        );
    }
    let exec_window = STATS_DETAIL_PROCESS_EVENTS_LIMIT as i64;
    assert!(detail
        .body_blobs
        .contains_key(&format!("d{:011x}", SMALL_LEDGER_ROWS - exec_window + 1)));
    assert!(!detail
        .body_blobs
        .contains_key(&format!("d{:011x}", SMALL_LEDGER_ROWS - exec_window)));
    assert!(!detail
        .body_blobs
        .contains_key(&format!("c{:011x}", SMALL_LEDGER_ROWS - 200)));
    assert_eq!(
        detail.body_blobs.len(),
        200 * 3 + exec_window as usize,
        "net, model, tool and exec windows"
    );
}

#[tokio::test]
async fn triage_lists_the_newest_tool_errors_first() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = seed_ledger(&dir.path().join("triage"), SMALL_LEDGER_ROWS);
    let db = capsem_logger::DbHandle::open_external_reader(&db_path).unwrap();
    let triage = session_db_triage("plans", &db, &db_path, 5).await.unwrap();
    let rows = triage["tool_errors"]["rows"].as_array().unwrap();
    let messages: Vec<&str> = rows.iter().map(|row| row[8].as_str().unwrap()).collect();
    let counted: Vec<String> = (1..=SMALL_LEDGER_ROWS)
        .rev()
        .filter(|i| i % 7 != 0)
        .take(5)
        .map(|i| format!("failed {i}"))
        .collect();
    assert_eq!(messages, counted, "{triage}");
}

#[tokio::test]
async fn async_registration_opens_off_the_worker_and_reuses_the_handle() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session = state.run_dir.join("sessions/async-register");
    seed_ledger(&session, 1);
    let first = state
        .register_session_db_handle_async("async-register", &session)
        .await
        .unwrap();
    let again = state
        .register_session_db_handle_async("async-register", &session)
        .await
        .unwrap();
    assert!(
        Arc::ptr_eq(&first, &again),
        "an unchanged path reuses the registered handle"
    );

    let moved = state.run_dir.join("sessions/async-register-moved");
    seed_ledger(&moved, 1);
    let rebound = state
        .register_session_db_handle_async("async-register", &moved)
        .await
        .unwrap();
    assert_eq!(rebound.path(), moved.join("session.db").as_path());
    assert!(Arc::ptr_eq(
        &rebound,
        &state.session_db_handle("async-register").unwrap()
    ));

    let missing = state.run_dir.join("sessions/never-created");
    let Err(error) = state.register_session_db_handle_async("never-created", &missing).await else {
        panic!("a missing ledger must not register");
    };
    assert!(error.to_string().contains("never-created"), "{error}");
    assert!(state.session_db_handle("never-created").is_none());
}
