use std::path::{Path, PathBuf};

use crate::reader::DbReader;
use crate::writer::DbWriter;

/// Convenience wrapper that owns the DB path and creates writer/reader instances.
pub struct SessionDb {
    path: PathBuf,
}

impl SessionDb {
    /// Create a new SessionDb pointing at the given path.
    /// Does not open any connections; call `writer()` or `reader()` as needed.
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }

    /// The path to the database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Open a writer (spawns a dedicated thread).
    pub fn writer(&self, capacity: usize) -> rusqlite::Result<DbWriter> {
        DbWriter::open(&self.path, capacity)
    }

    /// Open a read-only connection.
    pub fn reader(&self) -> rusqlite::Result<DbReader> {
        DbReader::open(&self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{Decision, McpCall, ModelCall, NetEvent, ToolCallEntry, ToolResponseEntry};
    use std::time::{Duration, SystemTime};

    fn temp_db_path(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("capsem-test-db-{name}-{}.db", std::process::id()));
        // Clean up any stale file/WAL from previous runs
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(p.with_extension("db-wal"));
        let _ = std::fs::remove_file(p.with_extension("db-shm"));
        p
    }

    fn make_net_event(domain: &str, decision: Decision) -> NetEvent {
        NetEvent {
            timestamp: SystemTime::now(),
            domain: domain.to_string(),
            port: 443,
            decision,
            process_name: Some("test".into()),
            pid: Some(1),
            method: Some("GET".into()),
            path: Some("/api".into()),
            query: None,
            status_code: Some(200),
            bytes_sent: 100,
            bytes_received: 500,
            duration_ms: 50,
            matched_rule: None,
            request_headers: None,
            response_headers: None,
            request_body_preview: None,
            response_body_preview: None,
            conn_type: None,
        }
    }

    fn make_model_call() -> ModelCall {
        ModelCall {
            timestamp: SystemTime::now(),
            provider: "anthropic".into(),
            model: Some("claude-sonnet-4-20250514".into()),
            process_name: Some("claude".into()),
            pid: Some(42),
            method: "POST".into(),
            path: "/v1/messages".into(),
            stream: true,
            system_prompt_preview: None,
            messages_count: 3,
            tools_count: 1,
            request_bytes: 1024,
            request_body_preview: None,
            message_id: Some("msg_123".into()),
            status_code: Some(200),
            text_content: Some("Hello".into()),
            thinking_content: None,
            stop_reason: Some("end_turn".into()),
            input_tokens: Some(100),
            output_tokens: Some(50),
            usage_details: Default::default(),
            duration_ms: 1200,
            response_bytes: 2048,
            estimated_cost_usd: 0.003,
            trace_id: Some("trace_abc".into()),
            tool_calls: vec![ToolCallEntry {
                call_index: 0,
                call_id: "call_001".into(),
                tool_name: "write_file".into(),
                arguments: Some(r#"{"path":"test.txt"}"#.into()),
                origin: "native".into(),
            }],
            tool_responses: vec![ToolResponseEntry {
                call_id: "call_001".into(),
                content_preview: Some("ok".into()),
                is_error: false,
            }],
        }
    }

    #[test]
    fn session_db_path() {
        let db = SessionDb::new(Path::new("/tmp/test.db"));
        assert_eq!(db.path(), Path::new("/tmp/test.db"));
    }

    #[test]
    fn writer_creates_tables() {
        let p = temp_db_path("creates-tables");
        let _writer = DbWriter::open(&p, 16).expect("open writer");
        drop(_writer);

        // Verify tables exist by opening a reader and querying
        let reader = DbReader::open(&p).expect("open reader");
        let counts = reader.net_event_counts().unwrap();
        assert_eq!(counts.total, 0);

        std::fs::remove_file(&p).ok();
    }

    #[tokio::test]
    async fn write_read_roundtrip_net_event() {
        let p = temp_db_path("rt-net");
        let writer = DbWriter::open(&p, 16).unwrap();
        let event = make_net_event("example.com", Decision::Allowed);

        writer.write(crate::WriteOp::NetEvent(event)).await;
        drop(writer); // flush

        let reader = DbReader::open(&p).unwrap();
        let events = reader.recent_net_events(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].domain, "example.com");
        assert_eq!(events[0].decision, Decision::Allowed);

        std::fs::remove_file(&p).ok();
    }

    #[tokio::test]
    async fn write_read_roundtrip_model_call() {
        let p = temp_db_path("rt-model");
        let writer = DbWriter::open(&p, 16).unwrap();
        let mc = make_model_call();

        writer.write(crate::WriteOp::ModelCall(mc)).await;
        drop(writer);

        let reader = DbReader::open(&p).unwrap();
        let calls = reader.recent_model_calls(10).unwrap();
        assert_eq!(calls.len(), 1);
        let (id, call) = &calls[0];
        assert_eq!(call.provider, "anthropic");
        assert_eq!(call.trace_id.as_deref(), Some("trace_abc"));

        let tools = reader.tool_calls_for(*id).unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_name, "write_file");
        assert_eq!(tools[0].origin, "native");

        let resps = reader.tool_responses_for(*id).unwrap();
        assert_eq!(resps.len(), 1);
        assert_eq!(resps[0].call_id, "call_001");
        assert!(!resps[0].is_error);

        std::fs::remove_file(&p).ok();
    }

    #[tokio::test]
    async fn write_read_roundtrip_mcp_call() {
        let p = temp_db_path("rt-mcp");
        let writer = DbWriter::open(&p, 16).unwrap();

        let mcp = McpCall {
            timestamp: SystemTime::now(),
            server_name: "builtin".into(),
            method: "tools/call".into(),
            tool_name: Some("fetch_http".into()),
            request_id: Some("req_1".into()),
            request_preview: Some("{}".into()),
            response_preview: Some("{\"ok\":true}".into()),
            decision: "allowed".into(),
            duration_ms: 100,
            error_message: None,
            process_name: Some("claude".into()),
            bytes_sent: 50,
            bytes_received: 200,
        };
        writer.write(crate::WriteOp::McpCall(mcp)).await;
        drop(writer);

        let reader = DbReader::open(&p).unwrap();
        let calls = reader.recent_mcp_calls(10).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].server_name, "builtin");
        assert_eq!(calls[0].tool_name.as_deref(), Some("fetch_http"));

        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn empty_db_returns_zero_counts() {
        let p = temp_db_path("empty-counts");
        let writer = DbWriter::open(&p, 16).unwrap();
        drop(writer);

        let reader = DbReader::open(&p).unwrap();
        let counts = reader.net_event_counts().unwrap();
        assert_eq!(counts.total, 0);
        assert_eq!(counts.allowed, 0);
        assert_eq!(reader.model_call_count().unwrap(), 0);
        assert_eq!(reader.file_event_count().unwrap(), 0);

        let stats = reader.session_stats().unwrap();
        assert_eq!(stats.net_total, 0);
        assert_eq!(stats.model_call_count, 0);

        std::fs::remove_file(&p).ok();
    }

    #[tokio::test]
    async fn wal_survives_close_reopen() {
        let p = temp_db_path("wal-reopen");

        let writer = DbWriter::open(&p, 16).unwrap();
        writer.write(crate::WriteOp::NetEvent(make_net_event("a.com", Decision::Allowed))).await;
        writer.write(crate::WriteOp::NetEvent(make_net_event("b.com", Decision::Denied))).await;
        drop(writer);

        let reader = DbReader::open(&p).unwrap();
        let c = reader.net_event_counts().unwrap();
        assert_eq!((c.total, c.allowed, c.denied), (2, 1, 1));

        let writer2 = DbWriter::open(&p, 16).unwrap();
        writer2.write(crate::WriteOp::NetEvent(make_net_event("c.com", Decision::Error))).await;
        drop(writer2);

        let reader2 = DbReader::open(&p).unwrap();
        let c2 = reader2.net_event_counts().unwrap();
        assert_eq!((c2.total, c2.allowed, c2.denied), (3, 1, 1));

        std::fs::remove_file(&p).ok();
    }
}
