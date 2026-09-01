use super::*;

// ========================================================================
// Phase 2: End-to-end SQL dedup tests
// Injects data into tool_calls + tool_responses, then runs
// the exact SQL from web/app/src/lib/sql.ts to verify dedup + response
// joining.
// ========================================================================

/// Helper: set up a dedup scenario with native + MCP tool calls.
///
/// Creates:
/// - model_call 1: native tool "bash" (origin=native) + tool_response for "toolu_prev"
/// - model_call 2: MCP tool "mcp__capsem__fetch_http" (origin=mcp_proxy) + tool_response
/// - tool_call origin=mcp: tools/call fetch_http (from MCP transport evidence)
/// - mcp_call: tools/list (protocol discovery, not a user tool call)
async fn setup_dedup_scenario(writer: &DbWriter) {
    // Model call 1: native tool "bash"
    let mut call1 = sample_model_call("anthropic");
    call1.trace_id = Some("trace_dedup".to_string());
    call1.tool_calls = vec![ToolCallEntry {
        call_index: 0,
        call_id: "toolu_bash_01".to_string(),
        tool_name: "bash".to_string(),
        arguments: Some(r#"{"command":"ls"}"#.to_string()),
        origin: "native".to_string(),
        trace_id: None,
    }];
    call1.tool_responses = vec![];
    writer.write(WriteOp::ModelCall(call1)).await;

    // Model call 2: MCP-proxied tool + response for bash from call 1
    let mut call2 = sample_model_call("anthropic");
    call2.trace_id = Some("trace_dedup".to_string());
    call2.tool_calls = vec![ToolCallEntry {
        call_index: 0,
        call_id: "toolu_mcp_01".to_string(),
        tool_name: "mcp__capsem__fetch_http".to_string(),
        arguments: Some(r#"{"url":"https://example.com"}"#.to_string()),
        origin: "mcp_proxy".to_string(),
        trace_id: None,
    }];
    // The response for the bash tool from call 1 comes back in call 2's request
    call2.tool_responses = vec![ToolResponseEntry {
        call_id: "toolu_bash_01".to_string(),
        content_preview: Some("file1.txt\nfile2.txt".to_string()),
        is_error: false,
        trace_id: None,
        credential_ref: None,
    }];
    writer.write(WriteOp::ModelCall(call2)).await;

    // MCP call: tools/call fetch_http (matches the MCP tool_call above)
    let mut mcp_tool = sample_mcp_call("capsem", "allowed");
    mcp_tool.method = "tools/call".to_string();
    mcp_tool.tool_name = Some("fetch_http".to_string());
    mcp_tool.request_preview = Some(r#"{"url":"https://example.com"}"#.to_string());
    mcp_tool.response_preview = Some(r#"{"body":"<html>..."}"#.to_string());
    writer.write(WriteOp::McpCall(mcp_tool)).await;

    // MCP call: tools/list (discovery, no tool_name)
    let mut mcp_list = sample_mcp_call("capsem", "allowed");
    mcp_list.method = "tools/list".to_string();
    mcp_list.tool_name = None;
    mcp_list.request_preview = None;
    mcp_list.response_preview = Some(r#"{"tools":[...]}"#.to_string());
    writer.write(WriteOp::McpCall(mcp_list)).await;
}

// -- SQL constants (must match web/app/src/lib/sql.ts exactly) --

const TOOL_COUNT_SQL: &str = "
    SELECT
        COUNT(*) as cnt
    FROM tool_calls
    WHERE origin IN ('native', 'mcp', 'builtin', 'local')
";

const TOOLS_STATS_SQL: &str = "
    SELECT
        (SELECT COUNT(*) FROM tool_calls WHERE origin IN ('native', 'mcp', 'builtin', 'local')) as total,
        (SELECT COUNT(*) FROM tool_calls WHERE origin = 'native') as native,
        (SELECT COUNT(*) FROM tool_calls WHERE origin = 'mcp') as mcp,
        (SELECT COUNT(*) FROM tool_calls WHERE origin IN ('native', 'mcp', 'builtin', 'local') AND decision = 'allowed') as allowed,
        (SELECT COUNT(*) FROM tool_calls WHERE origin IN ('native', 'mcp', 'builtin', 'local') AND decision != 'allowed') as denied
";

const TOOLS_TOP_TOOLS_SQL: &str = "
    SELECT tool_name, COUNT(*) as cnt, origin as source
    FROM tool_calls
    WHERE origin IN ('native', 'mcp', 'builtin', 'local')
    GROUP BY tool_name, origin
    ORDER BY cnt DESC
    LIMIT 10
";

const TOOLS_OVER_TIME_SQL: &str = "
    WITH numbered AS (
        SELECT source,
            (ROW_NUMBER() OVER (ORDER BY timestamp) - 1) / 5 as bucket
        FROM (
            SELECT COALESCE(NULLIF(timestamp, ''), '1970-01-01T00:00:00Z') as timestamp, origin as source
            FROM tool_calls
            WHERE origin IN ('native', 'mcp', 'builtin', 'local')
        )
    )
    SELECT bucket,
        SUM(CASE WHEN source = 'native' THEN 1 ELSE 0 END) as native,
        SUM(CASE WHEN source = 'mcp' THEN 1 ELSE 0 END) as mcp
    FROM numbered
    GROUP BY bucket
    ORDER BY bucket
";

const TOOLS_UNIFIED_SQL: &str = "
    SELECT event_id, timestamp, process_name, server_name, tool_name, method,
           decision, duration_ms, bytes, arguments, response_preview,
           error_message, source
    FROM (
        SELECT tc.event_id,
               COALESCE(NULLIF(tc.timestamp, ''), mc.timestamp) as timestamp,
               tc.process_name,
               COALESCE(tc.server_name, 'model') as server_name,
               tc.tool_name,
               tc.method,
               tc.decision,
               COALESCE(tc.duration_ms, mc.duration_ms, 0) as duration_ms,
               COALESCE(LENGTH(tc.arguments), 0) + COALESCE(LENGTH(COALESCE(tc.response_preview, tr.content_preview)), 0) as bytes,
               tc.arguments,
               COALESCE(tc.response_preview, tr.content_preview) as response_preview,
               tc.error_message,
               tc.origin as source
        FROM tool_calls tc
        LEFT JOIN model_calls mc ON tc.model_call_id = mc.id
        LEFT JOIN tool_responses tr ON tc.call_id = tr.call_id
        WHERE origin IN ('native', 'mcp', 'builtin', 'local')
    )
    ORDER BY timestamp DESC
";

const TOOLS_UNIFIED_SEARCH_SQL: &str = "
    SELECT event_id, timestamp, process_name, server_name, tool_name, method,
           decision, duration_ms, bytes, arguments, response_preview,
           error_message, source
    FROM (
        SELECT tc.event_id,
               COALESCE(NULLIF(tc.timestamp, ''), mc.timestamp) as timestamp,
               tc.process_name,
               COALESCE(tc.server_name, 'model') as server_name,
               tc.tool_name,
               tc.method,
               tc.decision,
               COALESCE(tc.duration_ms, mc.duration_ms, 0) as duration_ms,
               COALESCE(LENGTH(tc.arguments), 0) + COALESCE(LENGTH(COALESCE(tc.response_preview, tr.content_preview)), 0) as bytes,
               tc.arguments,
               COALESCE(tc.response_preview, tr.content_preview) as response_preview,
               tc.error_message,
               tc.origin as source
        FROM tool_calls tc
        LEFT JOIN model_calls mc ON tc.model_call_id = mc.id
        LEFT JOIN tool_responses tr ON tc.call_id = tr.call_id
        WHERE origin IN ('native', 'mcp', 'builtin', 'local')
    )
    WHERE tool_name LIKE ? OR method LIKE ? OR server_name LIKE ? OR process_name LIKE ?
    ORDER BY timestamp DESC
";

#[tokio::test]
async fn tool_unified_no_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dedup.db");
    let writer = DbWriter::open(&path, 64).unwrap();
    setup_dedup_scenario(&writer).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let json = reader.query_raw(TOOLS_UNIFIED_SQL).unwrap();
    let (_, rows) = parse_query_result(&json);

    // "bash" should appear once (from tool_calls, source=native)
    let bash_rows: Vec<_> = rows.iter().filter(|r| r["tool_name"] == "bash").collect();
    assert_eq!(bash_rows.len(), 1, "bash should appear exactly once");
    assert_eq!(bash_rows[0]["source"], "native");

    // "fetch_http" should appear once (from tool_calls origin=mcp)
    let fetch_rows: Vec<_> = rows
        .iter()
        .filter(|r| r["tool_name"].as_str() == Some("fetch_http"))
        .collect();
    assert_eq!(fetch_rows.len(), 1, "fetch_http should appear exactly once");
    assert_eq!(fetch_rows[0]["source"], "mcp");

    // "mcp__capsem__fetch_http" should NOT appear (filtered out by origin='native')
    let mcp_proxy_rows = rows
        .iter()
        .filter(|r| r["tool_name"].as_str() == Some("mcp__capsem__fetch_http"))
        .count();
    assert_eq!(mcp_proxy_rows, 0, "mcp_proxy tool_call should be filtered out");

    // tools/list is protocol discovery, not a user tool call.
    let list_rows = rows
        .iter()
        .filter(|r| r["method"].as_str() == Some("tools/list"))
        .count();
    assert_eq!(list_rows, 0, "tools/list must not appear as a tool call");
}

#[tokio::test]
async fn tool_unified_native_has_response_preview() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dedup-response.db");
    let writer = DbWriter::open(&path, 64).unwrap();
    setup_dedup_scenario(&writer).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let json = reader.query_raw(TOOLS_UNIFIED_SQL).unwrap();
    let (_, rows) = parse_query_result(&json);

    let bash_row = rows.iter().find(|r| r["tool_name"] == "bash").unwrap();
    assert!(
        !bash_row["response_preview"].is_null(),
        "bash row should have response_preview from tool_responses, got null"
    );
    assert_eq!(bash_row["response_preview"].as_str().unwrap(), "file1.txt\nfile2.txt");
}

#[tokio::test]
async fn tool_stats_no_double_counting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dedup-stats.db");
    let writer = DbWriter::open(&path, 64).unwrap();
    setup_dedup_scenario(&writer).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let json = reader.query_raw(TOOLS_STATS_SQL).unwrap();
    let (_, rows) = parse_query_result(&json);
    let r = &rows[0];

    // native = 1 (only bash, not mcp__capsem__fetch_http)
    assert_eq!(r["native"].as_i64().unwrap(), 1, "native should be 1 (only bash)");
    // mcp = 1 (tools/call fetch_http only; tools/list is protocol evidence)
    assert_eq!(r["mcp"].as_i64().unwrap(), 1, "mcp should be 1");
    // total = native + mcp = 2
    assert_eq!(r["total"].as_i64().unwrap(), 2, "total should be 2");
}

#[tokio::test]
async fn tool_count_no_double_counting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dedup-count.db");
    let writer = DbWriter::open(&path, 64).unwrap();
    setup_dedup_scenario(&writer).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let json = reader.query_raw(TOOL_COUNT_SQL).unwrap();
    let (_, rows) = parse_query_result(&json);

    // cnt = 1 (native bash) + 1 (mcp fetch_http with tool_name) = 2
    // NOT 3 (which would include the mcp_proxy tool_call)
    let cnt = rows[0]["cnt"].as_i64().unwrap();
    assert_eq!(
        cnt, 2,
        "tool count should be 2 (1 native + 1 mcp with tool_name), got {cnt}"
    );
}

#[tokio::test]
async fn tool_top_tools_no_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dedup-top.db");
    let writer = DbWriter::open(&path, 64).unwrap();
    setup_dedup_scenario(&writer).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let json = reader.query_raw(TOOLS_TOP_TOOLS_SQL).unwrap();
    let (_, rows) = parse_query_result(&json);

    // "bash" appears with source=native
    let bash = rows.iter().find(|r| r["tool_name"] == "bash").unwrap();
    assert_eq!(bash["source"], "native");

    // "fetch_http" appears with source=mcp
    let fetch = rows
        .iter()
        .find(|r| r["tool_name"].as_str() == Some("fetch_http"))
        .unwrap();
    assert_eq!(fetch["source"], "mcp");

    // "mcp__capsem__fetch_http" does NOT appear
    let mcp_proxy = rows
        .iter()
        .find(|r| r["tool_name"].as_str() == Some("mcp__capsem__fetch_http"));
    assert!(
        mcp_proxy.is_none(),
        "mcp_proxy tool name should not appear in top tools"
    );
}

#[tokio::test]
async fn tool_over_time_no_double_counting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dedup-time.db");
    let writer = DbWriter::open(&path, 64).unwrap();
    setup_dedup_scenario(&writer).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let json = reader.query_raw(TOOLS_OVER_TIME_SQL).unwrap();
    let (_, rows) = parse_query_result(&json);

    // Sum native across all buckets: should be 1 (only bash)
    let total_native: i64 = rows.iter().map(|r| r["native"].as_i64().unwrap_or(0)).sum();
    assert_eq!(
        total_native, 1,
        "total native over time should be 1, got {total_native}"
    );
}

#[tokio::test]
async fn tool_unified_search_dedup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dedup-search.db");
    let writer = DbWriter::open(&path, 64).unwrap();
    setup_dedup_scenario(&writer).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let search = "%bash%";
    let json = reader
        .query_raw_with_params(
            TOOLS_UNIFIED_SEARCH_SQL,
            &[
                serde_json::Value::String(search.to_string()),
                serde_json::Value::String(search.to_string()),
                serde_json::Value::String(search.to_string()),
                serde_json::Value::String(search.to_string()),
            ],
        )
        .unwrap();
    let (_, rows) = parse_query_result(&json);

    // Only bash (native) should match -- NOT mcp__capsem__fetch_http
    assert_eq!(rows.len(), 1, "search for 'bash' should return 1 result");
    assert_eq!(rows[0]["tool_name"], "bash");
    assert_eq!(rows[0]["source"], "native");
}

#[tokio::test]
async fn tool_responses_linked_by_call_id_not_model_call_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dedup-callid.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    // model_call 1: has tool_call (call_id="toolu_01")
    let mut call1 = sample_model_call("anthropic");
    call1.tool_calls = vec![ToolCallEntry {
        call_index: 0,
        call_id: "toolu_01".to_string(),
        tool_name: "bash".to_string(),
        arguments: Some(r#"{"cmd":"echo hi"}"#.to_string()),
        origin: "native".to_string(),
        trace_id: None,
    }];
    call1.tool_responses = vec![];
    writer.write(WriteOp::ModelCall(call1)).await;

    // model_call 2: has tool_response (call_id="toolu_01") -- different model_call_id!
    let mut call2 = sample_model_call("anthropic");
    call2.tool_calls = vec![];
    call2.tool_responses = vec![ToolResponseEntry {
        call_id: "toolu_01".to_string(),
        content_preview: Some("hi".to_string()),
        is_error: false,
        trace_id: None,
        credential_ref: None,
    }];
    writer.write(WriteOp::ModelCall(call2)).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let json = reader.query_raw(TOOLS_UNIFIED_SQL).unwrap();
    let (_, rows) = parse_query_result(&json);

    let bash_row = rows.iter().find(|r| r["tool_name"] == "bash").unwrap();
    assert_eq!(
        bash_row["response_preview"].as_str().unwrap(),
        "hi",
        "response_preview should be joined via call_id, not model_call_id"
    );
}

#[tokio::test]
async fn tool_unified_empty_tables() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dedup-empty.db");
    let writer = DbWriter::open(&path, 64).unwrap();
    drop(writer);

    let reader = DbReader::open(&path).unwrap();

    // All 6 queries should return zero results without errors
    for (name, sql) in [
        ("TOOL_COUNT", TOOL_COUNT_SQL),
        ("TOOLS_STATS", TOOLS_STATS_SQL),
        ("TOOLS_TOP_TOOLS", TOOLS_TOP_TOOLS_SQL),
        ("TOOLS_OVER_TIME", TOOLS_OVER_TIME_SQL),
        ("TOOLS_UNIFIED", TOOLS_UNIFIED_SQL),
    ] {
        let result = reader.query_raw(sql);
        assert!(
            result.is_ok(),
            "{name} should not error on empty DB: {:?}",
            result.err()
        );
    }
    // Search query with params
    let empty_search = serde_json::Value::String("%%".to_string());
    let result = reader.query_raw_with_params(
        TOOLS_UNIFIED_SEARCH_SQL,
        &[
            empty_search.clone(),
            empty_search.clone(),
            empty_search.clone(),
            empty_search,
        ],
    );
    assert!(result.is_ok(), "TOOLS_UNIFIED_SEARCH should not error on empty DB");
}

#[tokio::test]
async fn tool_unified_counts_mcp_origin_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dedup-mcp-only.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    writer
        .write(WriteOp::McpCall(sample_mcp_call("github", "allowed")))
        .await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let json = reader.query_raw(TOOLS_UNIFIED_SQL).unwrap();
    let (_, rows) = parse_query_result(&json);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["source"], "mcp");
}

#[tokio::test]
async fn tool_unified_only_native_calls() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dedup-native-only.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    // Write a native tool call + response
    let mut call = sample_model_call("anthropic");
    call.tool_calls = vec![ToolCallEntry {
        call_index: 0,
        call_id: "toolu_solo".to_string(),
        tool_name: "read_file".to_string(),
        arguments: Some(r#"{"path":"README.md"}"#.to_string()),
        origin: "native".to_string(),
        trace_id: None,
    }];
    call.tool_responses = vec![];
    writer.write(WriteOp::ModelCall(call)).await;

    // Write the response in a separate model call (realistic scenario)
    let mut call2 = sample_model_call("anthropic");
    call2.tool_calls = vec![];
    call2.tool_responses = vec![ToolResponseEntry {
        call_id: "toolu_solo".to_string(),
        content_preview: Some("# README\nContents here".to_string()),
        is_error: false,
        trace_id: None,
        credential_ref: None,
    }];
    writer.write(WriteOp::ModelCall(call2)).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let json = reader.query_raw(TOOLS_UNIFIED_SQL).unwrap();
    let (_, rows) = parse_query_result(&json);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["source"], "native");
    assert_eq!(rows[0]["tool_name"], "read_file");
    assert_eq!(rows[0]["response_preview"].as_str().unwrap(), "# README\nContents here");
}
