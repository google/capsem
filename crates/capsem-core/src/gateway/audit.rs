/// Gateway audit database: records every LLM API interaction in a per-session
/// SQLite database for auditing, cost tracking, and replay.
///
/// Follows the same pattern as net/telemetry.rs WebDb.
use std::path::Path;
use std::time::SystemTime;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// A single gateway interaction event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayEvent {
    pub timestamp: SystemTime,
    /// Provider name: "anthropic", "openai", "google".
    pub provider: String,
    /// Model name extracted from request body (e.g., "claude-sonnet-4-20250514").
    pub model: Option<String>,
    /// HTTP method (POST, GET, etc.).
    pub method: String,
    /// Request path (e.g., "/v1/messages").
    pub path: String,
    /// HTTP status code from upstream.
    pub status_code: u16,
    /// Total request duration in milliseconds.
    pub duration_ms: u64,
    /// Request body size in bytes.
    pub request_bytes: u64,
    /// Response body size in bytes.
    pub response_bytes: u64,
    /// Whether the response was SSE-streamed.
    pub streamed: bool,
    /// Full request body (truncated at configured limit).
    pub request_body: Option<String>,
    /// Full response body / accumulated SSE payload.
    pub response_body: Option<String>,
    /// Error message if the request failed.
    pub error: Option<String>,
}

/// Per-session SQLite database for AI gateway audit logging.
pub struct GatewayDb {
    conn: Connection,
}

const CREATE_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS gateway_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp TEXT NOT NULL,
        provider TEXT NOT NULL,
        model TEXT,
        method TEXT NOT NULL,
        path TEXT NOT NULL,
        status_code INTEGER NOT NULL,
        duration_ms INTEGER NOT NULL,
        request_bytes INTEGER DEFAULT 0,
        response_bytes INTEGER DEFAULT 0,
        streamed INTEGER DEFAULT 0,
        request_body TEXT,
        response_body TEXT,
        error TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_gateway_events_provider
        ON gateway_events(provider);
    CREATE INDEX IF NOT EXISTS idx_gateway_events_timestamp
        ON gateway_events(timestamp);
";

impl GatewayDb {
    /// Open (or create) a gateway audit DB at the given path.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(CREATE_SCHEMA)?;
        Ok(Self { conn })
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(CREATE_SCHEMA)?;
        Ok(Self { conn })
    }

    /// Record a gateway event.
    pub fn record(&self, event: &GatewayEvent) -> rusqlite::Result<()> {
        let timestamp = humantime::format_rfc3339(event.timestamp).to_string();
        self.conn.execute(
            "INSERT INTO gateway_events (
                timestamp, provider, model, method, path, status_code,
                duration_ms, request_bytes, response_bytes, streamed,
                request_body, response_body, error
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                timestamp,
                event.provider,
                event.model,
                event.method,
                event.path,
                event.status_code as i64,
                event.duration_ms as i64,
                event.request_bytes as i64,
                event.response_bytes as i64,
                event.streamed as i64,
                event.request_body,
                event.response_body,
                event.error,
            ],
        )?;
        Ok(())
    }

    /// Query the most recent N events, ordered newest first.
    pub fn recent(&self, limit: usize) -> rusqlite::Result<Vec<GatewayEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT timestamp, provider, model, method, path, status_code,
                    duration_ms, request_bytes, response_bytes, streamed,
                    request_body, response_body, error
             FROM gateway_events
             ORDER BY id DESC
             LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            let ts_str: String = row.get(0)?;
            let timestamp =
                humantime::parse_rfc3339(&ts_str).unwrap_or(SystemTime::UNIX_EPOCH);
            Ok(GatewayEvent {
                timestamp,
                provider: row.get(1)?,
                model: row.get(2)?,
                method: row.get(3)?,
                path: row.get(4)?,
                status_code: row.get::<_, i64>(5)? as u16,
                duration_ms: row.get::<_, i64>(6)? as u64,
                request_bytes: row.get::<_, i64>(7)? as u64,
                response_bytes: row.get::<_, i64>(8)? as u64,
                streamed: row.get::<_, i64>(9)? != 0,
                request_body: row.get(10)?,
                response_body: row.get(11)?,
                error: row.get(12)?,
            })
        })?;

        rows.collect()
    }

    /// Count total recorded events.
    pub fn count(&self) -> rusqlite::Result<usize> {
        self.conn
            .query_row("SELECT COUNT(*) FROM gateway_events", [], |row| {
                row.get::<_, i64>(0).map(|n| n as usize)
            })
    }

    /// Count events by provider.
    pub fn count_by_provider(&self) -> rusqlite::Result<Vec<(String, usize)>> {
        let mut stmt = self.conn.prepare(
            "SELECT provider, COUNT(*) FROM gateway_events GROUP BY provider ORDER BY provider",
        )?;
        let rows = stmt.query_map([], |row| {
            let provider: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((provider, count as usize))
        })?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn sample_event(provider: &str) -> GatewayEvent {
        GatewayEvent {
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000),
            provider: provider.to_string(),
            model: Some("claude-sonnet-4-20250514".to_string()),
            method: "POST".to_string(),
            path: "/v1/messages".to_string(),
            status_code: 200,
            duration_ms: 1500,
            request_bytes: 2048,
            response_bytes: 8192,
            streamed: true,
            request_body: Some(r#"{"model":"claude-sonnet-4-20250514","messages":[{"role":"user","content":"hi"}]}"#.to_string()),
            response_body: Some("data: {\"type\":\"content_block_delta\"}\n\n".to_string()),
            error: None,
        }
    }

    #[test]
    fn open_in_memory_succeeds() {
        let db = GatewayDb::open_in_memory().unwrap();
        assert_eq!(db.count().unwrap(), 0);
    }

    #[test]
    fn open_file_creates_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ai.db");
        let db = GatewayDb::open(&path).unwrap();
        assert_eq!(db.count().unwrap(), 0);
        assert!(path.exists());
    }

    #[test]
    fn record_and_query_roundtrip() {
        let db = GatewayDb::open_in_memory().unwrap();
        let event = sample_event("anthropic");
        db.record(&event).unwrap();

        let results = db.recent(10).unwrap();
        assert_eq!(results.len(), 1);
        let r = &results[0];
        assert_eq!(r.provider, "anthropic");
        assert_eq!(r.model.as_deref(), Some("claude-sonnet-4-20250514"));
        assert_eq!(r.method, "POST");
        assert_eq!(r.path, "/v1/messages");
        assert_eq!(r.status_code, 200);
        assert_eq!(r.duration_ms, 1500);
        assert_eq!(r.request_bytes, 2048);
        assert_eq!(r.response_bytes, 8192);
        assert!(r.streamed);
        assert!(r.request_body.is_some());
        assert!(r.response_body.is_some());
        assert!(r.error.is_none());
    }

    #[test]
    fn record_error_event() {
        let db = GatewayDb::open_in_memory().unwrap();
        let mut event = sample_event("openai");
        event.status_code = 500;
        event.error = Some("upstream timeout".to_string());
        event.response_body = None;
        db.record(&event).unwrap();

        let results = db.recent(10).unwrap();
        assert_eq!(results[0].error.as_deref(), Some("upstream timeout"));
        assert_eq!(results[0].status_code, 500);
    }

    #[test]
    fn recent_returns_newest_first() {
        let db = GatewayDb::open_in_memory().unwrap();
        for (i, provider) in ["anthropic", "openai", "google"].iter().enumerate() {
            let mut event = sample_event(provider);
            event.timestamp =
                SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000 + i as u64);
            db.record(&event).unwrap();
        }

        let results = db.recent(10).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].provider, "google");
        assert_eq!(results[1].provider, "openai");
        assert_eq!(results[2].provider, "anthropic");
    }

    #[test]
    fn recent_with_limit() {
        let db = GatewayDb::open_in_memory().unwrap();
        for _ in 0..10 {
            db.record(&sample_event("anthropic")).unwrap();
        }
        let results = db.recent(3).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(db.count().unwrap(), 10);
    }

    #[test]
    fn count_by_provider() {
        let db = GatewayDb::open_in_memory().unwrap();
        for _ in 0..3 {
            db.record(&sample_event("anthropic")).unwrap();
        }
        for _ in 0..2 {
            db.record(&sample_event("openai")).unwrap();
        }
        db.record(&sample_event("google")).unwrap();

        let counts = db.count_by_provider().unwrap();
        assert_eq!(counts.len(), 3);
        assert_eq!(counts[0], ("anthropic".to_string(), 3));
        assert_eq!(counts[1], ("google".to_string(), 1));
        assert_eq!(counts[2], ("openai".to_string(), 2));
    }

    #[test]
    fn record_non_streaming() {
        let db = GatewayDb::open_in_memory().unwrap();
        let mut event = sample_event("google");
        event.streamed = false;
        db.record(&event).unwrap();

        let results = db.recent(10).unwrap();
        assert!(!results[0].streamed);
    }

    #[test]
    fn record_with_no_model() {
        let db = GatewayDb::open_in_memory().unwrap();
        let mut event = sample_event("anthropic");
        event.model = None;
        db.record(&event).unwrap();

        let results = db.recent(10).unwrap();
        assert!(results[0].model.is_none());
    }

    #[test]
    fn empty_db_count() {
        let db = GatewayDb::open_in_memory().unwrap();
        assert_eq!(db.count().unwrap(), 0);
        assert!(db.recent(10).unwrap().is_empty());
        assert!(db.count_by_provider().unwrap().is_empty());
    }
}
