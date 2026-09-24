use super::*;

#[tokio::test]
async fn tool_call_roundtrip_from_mcp_observation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    writer
        .write(WriteOp::McpCall(sample_mcp_call("github", "allowed")))
        .await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let calls = mcp_tool_rows(&reader);
    assert_eq!(calls.len(), 1);
    let c = &calls[0];
    assert_eq!(c["server_name"], "github");
    assert_eq!(c["method"], "tools/call");
    assert_eq!(c["tool_name"], "github__search_repos");
    assert_eq!(c["request_id"], "req-1");
    assert_eq!(c["decision"], "allowed");
    assert_eq!(c["duration_ms"], 250);
    assert_eq!(c["process_name"], "claude");
    assert_eq!(c["policy_mode"], "audit_only");
    assert_eq!(c["policy_action"], "allow");
    assert_eq!(c["policy_rule"], "mcp.tool.github__search_repos");
    assert_eq!(c["policy_reason"], "local policy allowed");
    assert_eq!(ledger_counters(&path).await.tools.calls, 1);
}

#[tokio::test]
async fn tool_call_search_from_unified_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp-search.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    writer
        .write(WriteOp::McpCall(sample_mcp_call("github", "allowed")))
        .await;
    writer.write(WriteOp::McpCall(sample_mcp_call("slack", "denied"))).await;
    writer
        .write(WriteOp::McpCall(sample_mcp_call("github", "warned")))
        .await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();

    let rows = mcp_tool_rows(&reader);
    assert_eq!(rows.iter().filter(|row| row["server_name"] == "github").count(), 2);

    assert_eq!(
        rows.iter()
            .filter(|row| row["tool_name"].as_str().unwrap().contains("search_repos"))
            .count(),
        3
    );

    assert_eq!(rows.iter().filter(|row| row["method"] == "tools/call").count(), 3);

    assert_eq!(
        rows.iter()
            .filter(|row| row["tool_name"].as_str().unwrap().contains("nonexistent"))
            .count(),
        0
    );
    assert_eq!(ledger_counters(&path).await.tools.calls, 3);
}

#[tokio::test]
async fn tool_call_counters_by_server() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp-stats.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    writer
        .write(WriteOp::McpCall(sample_mcp_call("github", "allowed")))
        .await;
    writer
        .write(WriteOp::McpCall(sample_mcp_call("github", "allowed")))
        .await;
    writer.write(WriteOp::McpCall(sample_mcp_call("slack", "denied"))).await;
    writer
        .write(WriteOp::McpCall(sample_mcp_call("github", "warned")))
        .await;
    writer
        .write(WriteOp::McpCall({
            let mut c = sample_mcp_call("slack", "error");
            c.error_message = Some("server crashed".to_string());
            c
        }))
        .await;
    drop(writer);

    // Every decision is a tool call: denied, warned and errored calls are
    // counted alongside the allowed ones, per server and in total.
    let tools = ledger_counters(&path).await.tools;
    assert_eq!(tools.calls, 5);
    assert_eq!(tools.mcp.len(), 2, "{:?}", tools.mcp);
    let per_server = |server: &str| tools.mcp[server].values().map(|usage| usage.calls).sum::<u64>();
    assert_eq!(per_server("github"), 3);
    assert_eq!(per_server("slack"), 2);

    // The decision each call carried landed on its row.
    let reader = DbReader::open(&path).unwrap();
    let rows = mcp_tool_rows(&reader);
    let decisions = |decision: &str| rows.iter().filter(|row| row["decision"] == decision).count();
    assert_eq!(
        (
            decisions("allowed"),
            decisions("warned"),
            decisions("denied"),
            decisions("error")
        ),
        (2, 1, 1, 1)
    );
}

#[tokio::test]
async fn tool_call_counters_empty_db() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp-empty.db");
    let writer = DbWriter::open(&path, 64).unwrap();
    drop(writer);

    let tools = ledger_counters(&path).await.tools;
    assert_eq!(tools.calls, 0);
    assert!(tools.by_tool.is_empty());
    assert!(tools.mcp.is_empty());
}

#[tokio::test]
async fn mcp_call_cap_field_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp-cap.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let mut call = sample_mcp_call("github", "allowed");
    call.request_preview = Some("x".repeat(300_000)); // 300KB > 256KB cap
    writer.write(WriteOp::McpCall(call)).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let calls = mcp_tool_rows(&reader);
    assert_eq!(calls.len(), 1);
    // Preview should be truncated to MAX_FIELD_BYTES (256KB)
    let preview = calls[0]["request_preview"].as_str().unwrap();
    assert!(preview.len() <= 256 * 1024, "preview not capped: {}", preview.len());
}

#[tokio::test]
async fn mcp_schema_migration_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp-migrate.db");

    // First open creates tables.
    let writer = DbWriter::open(&path, 64).unwrap();
    writer
        .write(WriteOp::McpCall(sample_mcp_call("github", "allowed")))
        .await;
    drop(writer);

    // Second open triggers migrate() again -- must not fail.
    let writer = DbWriter::open(&path, 64).unwrap();
    writer.write(WriteOp::McpCall(sample_mcp_call("slack", "denied"))).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let calls = mcp_tool_rows(&reader);
    assert_eq!(calls.len(), 2);
    assert_eq!(ledger_counters(&path).await.tools.calls, 2);
}

#[tokio::test]
async fn mcp_call_bytes_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bytes.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let mut call = sample_mcp_call("github", "allowed");
    call.bytes_sent = 1024;
    call.bytes_received = 4096;
    writer.write(WriteOp::McpCall(call)).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let calls = mcp_tool_rows(&reader);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["bytes_sent"], 1024);
    assert_eq!(calls[0]["bytes_received"], 4096);
}

/// `tool_calls.request_preview`/`response_preview` are compact display
/// fields; the forensic copy lives in `event_body_blobs`. This test used to
/// assert the preview round-tripped at full (10KB) size -- that was
/// asserting the bug this cap fixes. Now it asserts the preview is capped
/// at PREVIEW_BYTES (2KB) while the blob keeps the exact original body.
#[tokio::test]
async fn mcp_call_preview_is_capped_and_blob_is_full() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("preview.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    // 10KB preview -- must be capped to 2KB in the display column, but kept
    // in full in the event_body_blobs table.
    let preview = "x".repeat(10_000);
    let mut call = sample_mcp_call("github", "allowed");
    call.event_id = Some("aa11bb22cc33".into());
    call.request_preview = Some(preview.clone());
    call.response_preview = Some(preview.clone());
    writer.write(WriteOp::McpCall(call)).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let calls = mcp_tool_rows(&reader);
    // PREVIEW_BYTES (writer.rs) -- kept as a literal here since this is an
    // external integration-test crate and the constant is crate-private.
    assert_eq!(calls[0]["request_preview"].as_str().unwrap().len(), 2 * 1024);
    assert_eq!(calls[0]["response_preview"].as_str().unwrap().len(), 2 * 1024);

    let conn = rusqlite::Connection::open(&path).unwrap();
    let (original_bytes, stored_bytes): (i64, i64) = conn
        .query_row(
            "SELECT original_bytes, stored_bytes FROM event_body_blobs
             WHERE event_id = 'aa11bb22cc33' AND direction = 'request'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(original_bytes, 10_000, "the blob keeps the full body size");
    assert_eq!(
        stored_bytes, 10_000,
        "the blob keeps the full body bytes, not the capped preview"
    );
}

#[tokio::test]
async fn mcp_call_huge_payload_truncated_at_256kb() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huge.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    // 1MB preview -- must be truncated to <= 256KB by cap_field
    let preview = "a".repeat(1_000_000);
    let mut call = sample_mcp_call("github", "allowed");
    call.request_preview = Some(preview);
    writer.write(WriteOp::McpCall(call)).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let calls = mcp_tool_rows(&reader);
    let stored = calls[0]["request_preview"].as_str().unwrap();
    assert!(stored.len() <= 256 * 1024);
}

#[tokio::test]
async fn mcp_call_200_char_payload_not_truncated() {
    // Regression: old bug truncated at 200 chars. Verify exact 200 chars preserved.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("200.db");
    let writer = DbWriter::open(&path, 64).unwrap();

    let preview = "b".repeat(200);
    let mut call = sample_mcp_call("github", "allowed");
    call.request_preview = Some(preview.clone());
    writer.write(WriteOp::McpCall(call)).await;
    drop(writer);

    let reader = DbReader::open(&path).unwrap();
    let calls = mcp_tool_rows(&reader);
    assert_eq!(calls[0]["request_preview"].as_str().unwrap().len(), 200);
    assert_eq!(calls[0]["request_preview"].as_str().unwrap(), &preview);
}

// ── File event tests ──────────────────────────────────────────────────
