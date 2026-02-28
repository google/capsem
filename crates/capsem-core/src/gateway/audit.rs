/// Gateway audit database: records every LLM API interaction in a per-session
/// SQLite database for auditing, cost tracking, and replay.
///
/// Two database types:
/// - `GatewayDb`: Legacy flat table (used by gateway/server.rs standalone mode)
/// - `AiDb`: Normalized 4-table schema for structured AI interaction auditing
///   (used by the MITM proxy inline SSE parser)
use std::path::Path;
use std::sync::{Arc, Mutex};
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

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// AiDb: Normalized 4-table schema for inline MITM proxy SSE auditing
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

const AI_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS model_requests (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp TEXT NOT NULL,
        provider TEXT NOT NULL,
        model TEXT,
        api_key_suffix TEXT,
        method TEXT NOT NULL,
        path TEXT NOT NULL,
        stream INTEGER DEFAULT 0,
        system_prompt_preview TEXT,
        messages_count INTEGER DEFAULT 0,
        tools_count INTEGER DEFAULT 0,
        request_bytes INTEGER DEFAULT 0,
        request_body_preview TEXT
    );

    CREATE TABLE IF NOT EXISTS model_responses (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        request_id INTEGER NOT NULL REFERENCES model_requests(id),
        message_id TEXT,
        status_code INTEGER,
        text_content TEXT,
        thinking_content TEXT,
        stop_reason TEXT,
        input_tokens INTEGER,
        output_tokens INTEGER,
        duration_ms INTEGER DEFAULT 0,
        response_bytes INTEGER DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS tool_calls (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        response_id INTEGER NOT NULL REFERENCES model_responses(id),
        call_index INTEGER NOT NULL,
        call_id TEXT NOT NULL,
        tool_name TEXT NOT NULL,
        arguments TEXT
    );

    CREATE TABLE IF NOT EXISTS tool_responses (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        request_id INTEGER NOT NULL REFERENCES model_requests(id),
        call_id TEXT NOT NULL,
        content_preview TEXT,
        is_error INTEGER DEFAULT 0
    );

    CREATE INDEX IF NOT EXISTS idx_model_requests_provider_ts
        ON model_requests(provider, timestamp);
    CREATE INDEX IF NOT EXISTS idx_model_responses_request
        ON model_responses(request_id);
    CREATE INDEX IF NOT EXISTS idx_tool_calls_response_name
        ON tool_calls(response_id, tool_name);
    CREATE INDEX IF NOT EXISTS idx_tool_responses_request_call
        ON tool_responses(request_id, call_id);
";

/// Record for inserting into model_requests.
pub struct ModelRequest {
    pub timestamp: SystemTime,
    pub provider: String,
    pub model: Option<String>,
    pub api_key_suffix: Option<String>,
    pub method: String,
    pub path: String,
    pub stream: bool,
    pub system_prompt_preview: Option<String>,
    pub messages_count: usize,
    pub tools_count: usize,
    pub request_bytes: u64,
    pub request_body_preview: Option<String>,
}

/// Record for inserting into model_responses.
pub struct ModelResponse {
    pub request_id: i64,
    pub message_id: Option<String>,
    pub status_code: Option<u16>,
    pub text_content: Option<String>,
    pub thinking_content: Option<String>,
    pub stop_reason: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub duration_ms: u64,
    pub response_bytes: u64,
}

/// Record for inserting into tool_calls.
pub struct ToolCallRecord {
    pub response_id: i64,
    pub call_index: u32,
    pub call_id: String,
    pub tool_name: String,
    pub arguments: Option<String>,
}

/// Record for inserting into tool_responses.
pub struct ToolResponseRecord {
    pub request_id: i64,
    pub call_id: String,
    pub content_preview: Option<String>,
    pub is_error: bool,
}

/// Per-session SQLite database with normalized AI interaction tables.
pub struct AiDb {
    conn: Connection,
}

impl AiDb {
    /// Open (or create) an AI audit DB at the given path.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(AI_SCHEMA)?;
        Ok(Self { conn })
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(AI_SCHEMA)?;
        Ok(Self { conn })
    }

    /// Insert a model request. Returns the row ID.
    pub fn record_request(&self, req: &ModelRequest) -> rusqlite::Result<i64> {
        let ts = humantime::format_rfc3339(req.timestamp).to_string();
        self.conn.execute(
            "INSERT INTO model_requests (
                timestamp, provider, model, api_key_suffix, method, path,
                stream, system_prompt_preview, messages_count, tools_count,
                request_bytes, request_body_preview
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                ts,
                req.provider,
                req.model,
                req.api_key_suffix,
                req.method,
                req.path,
                req.stream as i64,
                req.system_prompt_preview,
                req.messages_count as i64,
                req.tools_count as i64,
                req.request_bytes as i64,
                req.request_body_preview,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert a model response. Returns the row ID.
    pub fn record_response(&self, resp: &ModelResponse) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO model_responses (
                request_id, message_id, status_code, text_content,
                thinking_content, stop_reason, input_tokens, output_tokens,
                duration_ms, response_bytes
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                resp.request_id,
                resp.message_id,
                resp.status_code.map(|s| s as i64),
                resp.text_content,
                resp.thinking_content,
                resp.stop_reason,
                resp.input_tokens.map(|t| t as i64),
                resp.output_tokens.map(|t| t as i64),
                resp.duration_ms as i64,
                resp.response_bytes as i64,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert a tool call.
    pub fn record_tool_call(&self, tc: &ToolCallRecord) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO tool_calls (
                response_id, call_index, call_id, tool_name, arguments
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                tc.response_id,
                tc.call_index as i64,
                tc.call_id,
                tc.tool_name,
                tc.arguments,
            ],
        )?;
        Ok(())
    }

    /// Insert a tool response.
    pub fn record_tool_response(&self, tr: &ToolResponseRecord) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO tool_responses (
                request_id, call_id, content_preview, is_error
             ) VALUES (?1, ?2, ?3, ?4)",
            params![
                tr.request_id,
                tr.call_id,
                tr.content_preview,
                tr.is_error as i64,
            ],
        )?;
        Ok(())
    }

    /// Record a complete AI interaction in a single transaction.
    /// Inserts request, response, tool calls, and tool responses atomically.
    pub fn record_interaction(
        &mut self,
        req: &ModelRequest,
        resp: &ModelResponse,
        tool_calls: &[ToolCallRecord],
        tool_responses: &[ToolResponseRecord],
    ) -> rusqlite::Result<(i64, i64)> {
        let tx = self.conn.transaction()?;

        let ts = humantime::format_rfc3339(req.timestamp).to_string();
        tx.execute(
            "INSERT INTO model_requests (
                timestamp, provider, model, api_key_suffix, method, path,
                stream, system_prompt_preview, messages_count, tools_count,
                request_bytes, request_body_preview
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                ts, req.provider, req.model, req.api_key_suffix, req.method, req.path,
                req.stream as i64, req.system_prompt_preview, req.messages_count as i64,
                req.tools_count as i64, req.request_bytes as i64, req.request_body_preview,
            ],
        )?;
        let request_id = tx.last_insert_rowid();

        tx.execute(
            "INSERT INTO model_responses (
                request_id, message_id, status_code, text_content,
                thinking_content, stop_reason, input_tokens, output_tokens,
                duration_ms, response_bytes
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                request_id, resp.message_id, resp.status_code.map(|s| s as i64),
                resp.text_content, resp.thinking_content, resp.stop_reason,
                resp.input_tokens.map(|t| t as i64), resp.output_tokens.map(|t| t as i64),
                resp.duration_ms as i64, resp.response_bytes as i64,
            ],
        )?;
        let response_id = tx.last_insert_rowid();

        for tc in tool_calls {
            tx.execute(
                "INSERT INTO tool_calls (response_id, call_index, call_id, tool_name, arguments)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![response_id, tc.call_index as i64, tc.call_id, tc.tool_name, tc.arguments],
            )?;
        }

        for tr in tool_responses {
            tx.execute(
                "INSERT INTO tool_responses (request_id, call_id, content_preview, is_error)
                 VALUES (?1, ?2, ?3, ?4)",
                params![request_id, tr.call_id, tr.content_preview, tr.is_error as i64],
            )?;
        }

        tx.commit()?;
        Ok((request_id, response_id))
    }

    /// Query the most recent N requests, ordered newest first.
    pub fn recent_requests(&self, limit: usize) -> rusqlite::Result<Vec<(i64, String, Option<String>, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, provider, model, method, path
             FROM model_requests ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
        })?;
        rows.collect()
    }

    /// Get tool calls for a given response ID.
    pub fn tool_calls_for_response(
        &self,
        response_id: i64,
    ) -> rusqlite::Result<Vec<(String, String, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT call_id, tool_name, arguments
             FROM tool_calls WHERE response_id = ?1 ORDER BY call_index",
        )?;
        let rows = stmt.query_map(params![response_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        rows.collect()
    }

    /// Count total requests.
    pub fn request_count(&self) -> rusqlite::Result<usize> {
        self.conn
            .query_row("SELECT COUNT(*) FROM model_requests", [], |row| {
                row.get::<_, i64>(0).map(|n| n as usize)
            })
    }
}

/// Async wrapper: record a complete AI interaction using `spawn_blocking`
/// to avoid blocking the async executor with SQLite I/O.
pub async fn record_interaction_async(
    db: Arc<Mutex<AiDb>>,
    req: ModelRequest,
    resp: ModelResponse,
    tool_calls: Vec<ToolCallRecord>,
    tool_responses: Vec<ToolResponseRecord>,
) {
    let result = tokio::task::spawn_blocking(move || {
        if let Ok(mut db) = db.lock() {
            if let Err(e) = db.record_interaction(&req, &resp, &tool_calls, &tool_responses) {
                tracing::warn!(error = %e, "failed to record AI interaction");
            }
        }
    });
    if let Err(e) = result.await {
        tracing::warn!(error = %e, "AI interaction recording task panicked");
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

    // ── AiDb tests ──────────────────────────────────────────────────

    fn sample_request(provider: &str) -> ModelRequest {
        ModelRequest {
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000),
            provider: provider.to_string(),
            model: Some("claude-sonnet-4-20250514".to_string()),
            api_key_suffix: Some("a1b2".to_string()),
            method: "POST".to_string(),
            path: "/v1/messages".to_string(),
            stream: true,
            system_prompt_preview: Some("You are helpful.".to_string()),
            messages_count: 3,
            tools_count: 2,
            request_bytes: 2048,
            request_body_preview: Some("{\"model\":\"...\"}".to_string()),
        }
    }

    fn sample_response(request_id: i64) -> ModelResponse {
        ModelResponse {
            request_id,
            message_id: Some("msg_01".to_string()),
            status_code: Some(200),
            text_content: Some("Hello world!".to_string()),
            thinking_content: None,
            stop_reason: Some("end_turn".to_string()),
            input_tokens: Some(25),
            output_tokens: Some(10),
            duration_ms: 1500,
            response_bytes: 4096,
        }
    }

    #[test]
    fn aidb_open_in_memory() {
        let db = AiDb::open_in_memory().unwrap();
        assert_eq!(db.request_count().unwrap(), 0);
    }

    #[test]
    fn aidb_open_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ai.db");
        let db = AiDb::open(&path).unwrap();
        assert_eq!(db.request_count().unwrap(), 0);
        assert!(path.exists());
    }

    #[test]
    fn aidb_request_response_roundtrip() {
        let db = AiDb::open_in_memory().unwrap();
        let req_id = db.record_request(&sample_request("anthropic")).unwrap();
        assert!(req_id > 0);

        let resp_id = db.record_response(&sample_response(req_id)).unwrap();
        assert!(resp_id > 0);

        let requests = db.recent_requests(10).unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1, "anthropic");
        assert_eq!(requests[0].2.as_deref(), Some("claude-sonnet-4-20250514"));
    }

    #[test]
    fn aidb_tool_calls() {
        let db = AiDb::open_in_memory().unwrap();
        let req_id = db.record_request(&sample_request("anthropic")).unwrap();
        let resp_id = db.record_response(&sample_response(req_id)).unwrap();

        db.record_tool_call(&ToolCallRecord {
            response_id: resp_id,
            call_index: 0,
            call_id: "toolu_01".into(),
            tool_name: "get_weather".into(),
            arguments: Some("{\"city\":\"NYC\"}".into()),
        }).unwrap();
        db.record_tool_call(&ToolCallRecord {
            response_id: resp_id,
            call_index: 1,
            call_id: "toolu_02".into(),
            tool_name: "search".into(),
            arguments: Some("{\"q\":\"rust\"}".into()),
        }).unwrap();

        let calls = db.tool_calls_for_response(resp_id).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "toolu_01");
        assert_eq!(calls[0].1, "get_weather");
        assert_eq!(calls[1].0, "toolu_02");
    }

    #[test]
    fn aidb_tool_responses() {
        let db = AiDb::open_in_memory().unwrap();
        let req_id = db.record_request(&sample_request("anthropic")).unwrap();
        db.record_tool_response(&ToolResponseRecord {
            request_id: req_id,
            call_id: "toolu_01".into(),
            content_preview: Some("72F and sunny".into()),
            is_error: false,
        }).unwrap();

        // Verify it was recorded (query by request_id)
        let count: i64 = db.conn.query_row(
            "SELECT COUNT(*) FROM tool_responses WHERE request_id = ?1",
            params![req_id], |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn aidb_record_interaction_transaction() {
        let mut db = AiDb::open_in_memory().unwrap();
        let req = sample_request("openai");
        let resp = ModelResponse {
            request_id: 0, // Will be overwritten
            message_id: Some("chatcmpl-1".into()),
            status_code: Some(200),
            text_content: Some("Hi!".into()),
            thinking_content: None,
            stop_reason: Some("stop".into()),
            input_tokens: Some(10),
            output_tokens: Some(3),
            duration_ms: 500,
            response_bytes: 1024,
        };
        let tool_calls = vec![ToolCallRecord {
            response_id: 0,
            call_index: 0,
            call_id: "call_1".into(),
            tool_name: "run".into(),
            arguments: Some("{}".into()),
        }];
        let tool_responses = vec![ToolResponseRecord {
            request_id: 0,
            call_id: "call_prev".into(),
            content_preview: Some("result".into()),
            is_error: false,
        }];

        let (req_id, resp_id) = db.record_interaction(&req, &resp, &tool_calls, &tool_responses).unwrap();
        assert!(req_id > 0);
        assert!(resp_id > 0);
        assert_eq!(db.request_count().unwrap(), 1);

        let calls = db.tool_calls_for_response(resp_id).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, "run");
    }

    #[test]
    fn aidb_multiple_requests() {
        let db = AiDb::open_in_memory().unwrap();
        for provider in &["anthropic", "openai", "google"] {
            let req_id = db.record_request(&sample_request(provider)).unwrap();
            db.record_response(&sample_response(req_id)).unwrap();
        }

        assert_eq!(db.request_count().unwrap(), 3);
        let requests = db.recent_requests(2).unwrap();
        assert_eq!(requests.len(), 2);
        // newest first
        assert_eq!(requests[0].1, "google");
        assert_eq!(requests[1].1, "openai");
    }

    #[test]
    fn aidb_empty_queries() {
        let db = AiDb::open_in_memory().unwrap();
        assert_eq!(db.request_count().unwrap(), 0);
        assert!(db.recent_requests(10).unwrap().is_empty());
        assert!(db.tool_calls_for_response(999).unwrap().is_empty());
    }
}
