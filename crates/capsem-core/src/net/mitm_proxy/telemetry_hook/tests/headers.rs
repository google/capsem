//! Observed credentials never reach stored headers or bodies raw.
//!
//! Bodies were redacted only with credentials observed in bodies, and headers
//! were cloned into the ledger unredacted: a key seen in `x-api-key` was
//! stored verbatim in an allowlisted or echoed header value, and in any body
//! that repeated it (google/capsem#218, owned by #229).
use super::*;

const RAW: &str = "sk-ant-header-observed-secret";

#[tokio::test]
async fn an_observed_credential_is_redacted_from_every_stored_header_and_body() {
    let _lock = crate::credential_broker::TEST_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let _guard = EnvGuard::install(
        &dir.path().join("capsem-home"),
        dir.path(),
        &dir.path().join("credential-store.json"),
    );
    let db = Arc::new(DbWriter::open(&db_path, 64).expect("test db"));
    let hook = TelemetryHook::new(Arc::new(TelemetryDeps {
        db: Arc::clone(&db),
        pricing: Arc::new(PricingTable::load()),
        trace_state: Arc::new(Mutex::new(TraceState::new())),
        security_rules: empty_security_rules(),
        plugin_policy: Arc::new(std::sync::RwLock::new(BTreeMap::new().into())),
    }));

    let mut req_ctx = anthropic_req_ctx();
    // Seen once, in a request header. It must not survive anywhere else: not
    // in a header stored just below the 16 KB cap (so truncation cannot be
    // what removes it), not in a short header after it, not echoed back by
    // the server, and not in either body.
    // The whole set fits the cap once redacted; the key sits ~250 bytes below it.
    let filler = "a".repeat(16 * 1024 - 300);
    req_ctx.request_headers = Some(format!(
        "user-agent: capsem-test\nx-padding: {filler}\nx-api-key: {RAW}\nx-late: {RAW}"
    ));
    req_ctx.response_headers = Some(format!("content-type: application/json\nx-echo: {RAW}"));
    req_ctx.request_body_stats.lock().unwrap().preview = format!(r#"{{"key":"{RAW}"}}"#).into_bytes();
    req_ctx.credential_observations = vec![CredentialObservation {
        provider: CredentialProvider::Anthropic,
        raw_value: RAW.to_string(),
        source: "http.header.x-api-key".to_string(),
        event_type: Some("http.request".to_string()),
        trace_id: None,
        context_json: None,
    }];

    let mut state = HookState::default();
    let conn = any_conn();
    {
        let mut c = ctx_for(&mut state, &conn);
        *c.state::<Option<TelemetryRequestContext>>(|| None) = Some(req_ctx);
        *c.state::<TelemetryResponseStats>(TelemetryResponseStats::default) = TelemetryResponseStats {
            bytes: 64,
            preview: format!(r#"{{"echo":"{RAW}"}}"#).into_bytes(),
            max_body_capture: 4096,
        };
    }
    complete_response(&hook, &mut state, &conn).await;
    db.flush().await;

    let reference = credential_reference("anthropic", RAW);
    let (event_id, request_headers, response_headers, request_preview, response_preview) = {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.query_row(
            "SELECT event_id, request_headers, response_headers, request_body_preview, response_body_preview \
             FROM net_events",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .unwrap()
    };
    for (name, stored) in [
        ("request headers", &request_headers),
        ("response headers", &response_headers),
        ("request body", &request_preview),
        ("response body", &response_preview),
    ] {
        assert!(!stored.contains(RAW), "{name} stored the raw credential");
    }
    assert!(
        request_headers.contains(&format!("x-late: {reference}")),
        "{request_headers:.200}"
    );
    assert!(
        response_headers.contains(&format!("x-echo: {reference}")),
        "{response_headers}"
    );
    assert!(
        request_headers.starts_with("user-agent: capsem-test\n"),
        "non-secret headers stay useful"
    );
    let archived = capsem_logger::DbHandle::open_external_reader(&db_path)
        .unwrap()
        .read_body(&event_id, "net_events", capsem_logger::BodyDirection::Request)
        .await
        .unwrap()
        .expect("the request body is archived");
    assert!(!String::from_utf8_lossy(&archived.bytes).contains(RAW), "archived body");

    db.shutdown_blocking();
    assert!(
        !String::from_utf8_lossy(&std::fs::read(&db_path).unwrap()).contains(RAW),
        "raw credential leaked into session db"
    );
}
