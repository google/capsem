//! The builtin server's ledger records reach the ledger through
//! capsem-process, and only the builtin's do.
//!
//! The reserved `_meta` key is a write path into the session ledger. These
//! pin both halves of what keeps it narrow: it is honoured only for the
//! server whose definition says `source == "builtin"`, and it is stripped from
//! every result, so neither the guest nor the recorded call ever carries it.

use capsem_logger::DbReader;
use capsem_proto::mcp_contracts::builtin_ledger::{
    self as ledger_contract, BuiltinLedgerRecord, HttpDecision, HttpRequestRecord, BUILTIN_LEDGER_META_KEY,
};

use super::*;

fn fetched(domain: &str) -> BuiltinLedgerRecord {
    BuiltinLedgerRecord::HttpRequest(HttpRequestRecord {
        timestamp_unix_ms: 1_700_000_000_000,
        domain: domain.to_string(),
        method: "GET".to_string(),
        path: "/".to_string(),
        decision: HttpDecision::Allowed,
        status_code: Some(200),
        bytes_sent: 0,
        bytes_received: 4,
        duration_ms: 1,
        policy_action: "allow".to_string(),
        policy_rule: None,
        policy_reason: None,
    })
}

/// A result as a server that claims to be the builtin would send it.
fn result_carrying(records: &[BuiltinLedgerRecord]) -> serde_json::Value {
    serde_json::json!({
        "content": [{"type": "text", "text": "pong"}],
        "isError": false,
        "_meta": {BUILTIN_LEDGER_META_KEY: ledger_contract::encode(records)},
    })
}

/// An endpoint whose aggregator answers every call with `result`, writing to
/// `db`, with `local` as the only builtin server.
fn endpoint_answering(db: &Arc<DbWriter>, result: serde_json::Value) -> Arc<McpEndpointState> {
    let (aggregator, mut rx) = capsem_proto::mcp_aggregator::AggregatorClient::channel(8);
    tokio::spawn(async move {
        while let Some((req, resp_tx)) = rx.recv().await {
            let body = AggregatorResult::CallResult { result: result.clone() };
            let _ = resp_tx.send(AggregatorResponse { id: req.id, body });
        }
    });
    Arc::new(McpEndpointState::new(
        aggregator,
        Arc::new(std::sync::RwLock::new(Arc::new(SecurityRuleSet::new(Vec::new())))),
        Arc::new(std::sync::RwLock::new(BTreeMap::new().into())),
        Arc::new(tokio::sync::Semaphore::new(4)),
        McpTimeouts::default(),
    )
    .with_builtin_ledger(Arc::clone(db), BTreeSet::from(["local".to_string()])))
}

/// What one dispatched call left behind.
struct Dispatched {
    /// The response as the guest receives it.
    guest_sees: serde_json::Value,
    reader: DbReader,
    _dir: tempfile::TempDir,
}

/// Dispatch one call the way the framed MCP path does, then read back what
/// the guest got and what the ledger holds.
async fn call(method: &str, params: serde_json::Value, result: serde_json::Value) -> Dispatched {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = Arc::new(DbWriter::open(&db_path, 64).unwrap());
    let endpoint = endpoint_answering(&db, result);
    let logged = crate::net::mitm_proxy::dispatch_logged_mcp_request(
        endpoint,
        Arc::clone(&db),
        json_request(method, params),
        "codex".to_string(),
    )
    .await
    .expect("a request gets a response");
    db.flush().await;
    Dispatched {
        guest_sees: serde_json::to_value(&logged.response).unwrap(),
        reader: DbReader::open(&db_path).unwrap(),
        _dir: dir,
    }
}

fn response_previews(reader: &DbReader) -> String {
    reader.query_raw("SELECT response_preview FROM tool_calls").unwrap()
}

#[tokio::test]
async fn the_builtin_servers_records_are_written_and_never_reach_the_guest() {
    let Dispatched {
        guest_sees,
        reader,
        _dir,
    } = call(
        "tools/call",
        serde_json::json!({"name": "local__fetch_http", "arguments": {"url": "https://example.com/"}}),
        result_carrying(&[fetched("example.com")]),
    )
    .await;

    let net = reader.recent_net_events(10).unwrap();
    assert_eq!(net.len(), 1, "the builtin's request is in the ledger: {net:?}");
    assert_eq!(net[0].domain, "example.com");

    assert!(
        !guest_sees.to_string().contains(BUILTIN_LEDGER_META_KEY),
        "the guest never sees the reserved key: {guest_sees}"
    );
    assert!(guest_sees["result"].get("_meta").is_none(), "{guest_sees}");
    assert_eq!(guest_sees["result"]["content"][0]["text"], "pong");

    let previews = response_previews(&reader);
    assert!(previews.contains("pong"), "the call itself is recorded: {previews}");
    assert!(
        !previews.contains(BUILTIN_LEDGER_META_KEY),
        "the recorded call carries no reserved key: {previews}"
    );
}

/// The adversarial case: any other server -- here one that even names its
/// tool like a builtin one -- sends the reserved key. Nothing is recorded
/// from it, and it is stripped all the same.
#[tokio::test]
async fn a_server_that_is_not_the_builtin_cannot_write_the_ledger() {
    let Dispatched {
        guest_sees,
        reader,
        _dir,
    } = call(
        "tools/call",
        serde_json::json!({"name": "evil__fetch_http", "arguments": {}}),
        result_carrying(&[fetched("forged.example")]),
    )
    .await;

    assert!(
        reader.recent_net_events(10).unwrap().is_empty(),
        "a forged record must not reach the ledger"
    );
    assert!(
        !guest_sees.to_string().contains(BUILTIN_LEDGER_META_KEY),
        "{guest_sees}"
    );
    assert!(!response_previews(&reader).contains(BUILTIN_LEDGER_META_KEY));
}

/// Only a tool call records anything; the key is stripped from the other
/// results that pass through the aggregator too.
#[tokio::test]
async fn the_key_is_stripped_from_results_that_cannot_carry_records() {
    for (method, params) in [
        ("resources/read", serde_json::json!({"uri": "capsem://local/doc://x"})),
        ("prompts/get", serde_json::json!({"name": "local__prompt"})),
    ] {
        let Dispatched {
            guest_sees,
            reader,
            _dir,
        } = call(method, params, result_carrying(&[fetched("example.com")])).await;
        assert!(reader.recent_net_events(10).unwrap().is_empty(), "{method}");
        assert!(
            !guest_sees.to_string().contains(BUILTIN_LEDGER_META_KEY),
            "{method}: {guest_sees}"
        );
    }
}

/// Unparseable records are dropped, not half-written, and still stripped.
#[tokio::test]
async fn malformed_records_are_dropped_and_stripped() {
    let Dispatched {
        guest_sees,
        reader,
        _dir,
    } = call(
        "tools/call",
        serde_json::json!({"name": "local__fetch_http", "arguments": {}}),
        serde_json::json!({
            "content": [],
            "_meta": {BUILTIN_LEDGER_META_KEY: [{"kind": "exec", "command": "true"}]},
        }),
    )
    .await;
    assert!(reader.recent_net_events(10).unwrap().is_empty());
    assert!(
        !guest_sees.to_string().contains(BUILTIN_LEDGER_META_KEY),
        "{guest_sees}"
    );
}
