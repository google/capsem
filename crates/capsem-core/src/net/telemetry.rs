/// Network telemetry: records every HTTPS connection attempt in a per-session
/// SQLite database (web.db) for auditing and future dashboard display.
use std::path::Path;
use std::time::SystemTime;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// The outcome of a domain policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allowed,
    Denied,
    Error,
}

impl Decision {
    fn as_str(&self) -> &'static str {
        match self {
            Decision::Allowed => "allowed",
            Decision::Denied => "denied",
            Decision::Error => "error",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "allowed" => Decision::Allowed,
            "denied" => Decision::Denied,
            _ => Decision::Error,
        }
    }
}

/// Serialize SystemTime as f64 epoch seconds (for frontend compatibility).
fn serialize_timestamp<S: serde::Serializer>(ts: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
    let epoch = ts.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    s.serialize_f64(epoch.as_secs_f64())
}

/// Deserialize f64 epoch seconds back to SystemTime.
fn deserialize_timestamp<'de, D: serde::Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
    let secs: f64 = serde::Deserialize::deserialize(d)?;
    Ok(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs_f64(secs))
}

/// A single network connection event recorded in web.db.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetEvent {
    #[serde(serialize_with = "serialize_timestamp", deserialize_with = "deserialize_timestamp")]
    pub timestamp: SystemTime,
    pub domain: String,
    pub port: u16,
    pub decision: Decision,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub duration_ms: u64,
    // Extended fields for MITM proxy (Option for backward compat).
    pub method: Option<String>,
    pub path: Option<String>,
    pub query: Option<String>,
    pub status_code: Option<u16>,
    pub matched_rule: Option<String>,
    pub request_headers: Option<String>,
    pub response_headers: Option<String>,
    pub request_body_preview: Option<String>,
    pub response_body_preview: Option<String>,
    pub conn_type: Option<String>,
}

/// Per-session SQLite database for HTTPS request recording.
///
/// Each VM gets its own `WebDb` instance at `~/.capsem/sessions/<vm_id>/web.db`.
pub struct WebDb {
    conn: Connection,
}

/// Base schema for new databases.
const CREATE_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS http_requests (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        timestamp TEXT NOT NULL,
        domain TEXT NOT NULL,
        port INTEGER NOT NULL,
        decision TEXT NOT NULL,
        bytes_sent INTEGER DEFAULT 0,
        bytes_received INTEGER DEFAULT 0,
        duration_ms INTEGER DEFAULT 0,
        method TEXT,
        path TEXT,
        query TEXT,
        status_code INTEGER,
        matched_rule TEXT,
        request_headers TEXT,
        response_headers TEXT,
        request_body_preview TEXT,
        response_body_preview TEXT,
        conn_type TEXT DEFAULT 'https'
    );
    CREATE INDEX IF NOT EXISTS idx_http_requests_domain
        ON http_requests(domain);
    CREATE INDEX IF NOT EXISTS idx_http_requests_timestamp
        ON http_requests(timestamp);
";

impl WebDb {
    /// Open (or create) a web.db at the given path.
    /// Creates the schema if the database is new.
    /// Runs migration if an old schema is detected.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let conn = Connection::open(path)?;

        // WAL mode for better concurrent read/write performance
        conn.pragma_update(None, "journal_mode", "WAL")?;

        conn.execute_batch(CREATE_SCHEMA)?;

        // Migrate old schema: add columns if they don't exist.
        Self::migrate(&conn)?;

        Ok(Self { conn })
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(CREATE_SCHEMA)?;
        Ok(Self { conn })
    }

    /// Run schema migrations: add new columns if they don't exist.
    fn migrate(conn: &Connection) -> rusqlite::Result<()> {
        let has_column = |col: &str| -> rusqlite::Result<bool> {
            let mut stmt = conn.prepare("PRAGMA table_info(http_requests)")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
            for name in rows {
                if name? == col {
                    return Ok(true);
                }
            }
            Ok(false)
        };

        let migrations: &[(&str, &str)] = &[
            ("method", "ALTER TABLE http_requests ADD COLUMN method TEXT"),
            ("path", "ALTER TABLE http_requests ADD COLUMN path TEXT"),
            ("status_code", "ALTER TABLE http_requests ADD COLUMN status_code INTEGER"),
            ("request_headers", "ALTER TABLE http_requests ADD COLUMN request_headers TEXT"),
            ("response_headers", "ALTER TABLE http_requests ADD COLUMN response_headers TEXT"),
            ("request_body_preview", "ALTER TABLE http_requests ADD COLUMN request_body_preview TEXT"),
            ("response_body_preview", "ALTER TABLE http_requests ADD COLUMN response_body_preview TEXT"),
            ("conn_type", "ALTER TABLE http_requests ADD COLUMN conn_type TEXT DEFAULT 'https'"),
            ("query", "ALTER TABLE http_requests ADD COLUMN query TEXT"),
            ("matched_rule", "ALTER TABLE http_requests ADD COLUMN matched_rule TEXT"),
        ];

        for (col, sql) in migrations {
            if !has_column(col)? {
                conn.execute(sql, [])?;
            }
        }

        Ok(())
    }

    /// Record a network event.
    pub fn record(&self, event: &NetEvent) -> rusqlite::Result<()> {
        let timestamp = humantime::format_rfc3339(event.timestamp).to_string();

        self.conn.execute(
            "INSERT INTO http_requests (
                timestamp, domain, port, decision, bytes_sent, bytes_received,
                duration_ms, method, path, query, status_code, matched_rule,
                request_headers, response_headers, request_body_preview,
                response_body_preview, conn_type
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                timestamp,
                event.domain,
                event.port as i64,
                event.decision.as_str(),
                event.bytes_sent as i64,
                event.bytes_received as i64,
                event.duration_ms as i64,
                event.method,
                event.path,
                event.query,
                event.status_code.map(|c| c as i64),
                event.matched_rule,
                event.request_headers,
                event.response_headers,
                event.request_body_preview,
                event.response_body_preview,
                event.conn_type,
            ],
        )?;
        Ok(())
    }

    /// Query the most recent N events, ordered newest first.
    pub fn recent(&self, limit: usize) -> rusqlite::Result<Vec<NetEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT timestamp, domain, port, decision, bytes_sent, bytes_received,
                    duration_ms, method, path, query, status_code, matched_rule,
                    request_headers, response_headers, request_body_preview,
                    response_body_preview, conn_type
             FROM http_requests
             ORDER BY id DESC
             LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            let ts_str: String = row.get(0)?;
            let timestamp = humantime::parse_rfc3339(&ts_str)
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let decision_str: String = row.get(3)?;

            Ok(NetEvent {
                timestamp,
                domain: row.get(1)?,
                port: row.get::<_, i64>(2)? as u16,
                decision: Decision::from_str(&decision_str),
                bytes_sent: row.get::<_, i64>(4)? as u64,
                bytes_received: row.get::<_, i64>(5)? as u64,
                duration_ms: row.get::<_, i64>(6)? as u64,
                method: row.get(7)?,
                path: row.get(8)?,
                query: row.get(9)?,
                status_code: row.get::<_, Option<i64>>(10)?.map(|c| c as u16),
                matched_rule: row.get(11)?,
                request_headers: row.get(12)?,
                response_headers: row.get(13)?,
                request_body_preview: row.get(14)?,
                response_body_preview: row.get(15)?,
                conn_type: row.get(16)?,
            })
        })?;

        rows.collect()
    }

    /// Count total recorded events.
    pub fn count(&self) -> rusqlite::Result<usize> {
        self.conn
            .query_row("SELECT COUNT(*) FROM http_requests", [], |row| {
                row.get::<_, i64>(0).map(|n| n as usize)
            })
    }

    /// Count events grouped by decision: returns (total, allowed, denied).
    pub fn count_by_decision(&self) -> rusqlite::Result<(usize, usize, usize)> {
        let mut stmt = self.conn.prepare(
            "SELECT decision, COUNT(*) FROM http_requests GROUP BY decision",
        )?;
        let rows = stmt.query_map([], |row| {
            let decision: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((decision, count as usize))
        })?;
        let mut total = 0usize;
        let mut allowed = 0usize;
        let mut denied = 0usize;
        for row in rows {
            let (decision, count) = row?;
            total += count;
            match decision.as_str() {
                "allowed" => allowed += count,
                "denied" => denied += count,
                _ => {} // error and other types counted in total only
            }
        }
        Ok((total, allowed, denied))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn sample_event(domain: &str, decision: Decision) -> NetEvent {
        NetEvent {
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000),
            domain: domain.to_string(),
            port: 443,
            decision,
            bytes_sent: 1024,
            bytes_received: 4096,
            duration_ms: 150,
            method: None,
            path: None,
            query: None,
            status_code: None,
            matched_rule: Some("test".to_string()),
            request_headers: None,
            response_headers: None,
            request_body_preview: None,
            response_body_preview: None,
            conn_type: None,
        }
    }

    fn http_event(domain: &str) -> NetEvent {
        NetEvent {
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000),
            domain: domain.to_string(),
            port: 443,
            decision: Decision::Allowed,
            bytes_sent: 2048,
            bytes_received: 8192,
            duration_ms: 250,
            method: Some("GET".to_string()),
            path: Some("/api/v1/repos".to_string()),
            query: None,
            status_code: Some(200),
            matched_rule: None,
            request_headers: Some("Host: github.com\r\nUser-Agent: curl".to_string()),
            response_headers: Some("Content-Type: application/json".to_string()),
            request_body_preview: None,
            response_body_preview: Some("{\"repos\":[]}".to_string()),
            conn_type: Some("https".to_string()),
        }
    }

    // -- Schema creation --

    #[test]
    fn open_in_memory_succeeds() {
        let db = WebDb::open_in_memory().unwrap();
        assert_eq!(db.count().unwrap(), 0);
    }

    #[test]
    fn open_file_creates_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("web.db");
        let db = WebDb::open(&path).unwrap();
        assert_eq!(db.count().unwrap(), 0);
        assert!(path.exists());
    }

    #[test]
    fn open_file_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions").join("vm1").join("web.db");
        let db = WebDb::open(&path).unwrap();
        assert_eq!(db.count().unwrap(), 0);
    }

    #[test]
    fn open_existing_db_preserves_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("web.db");

        // Write one event
        {
            let db = WebDb::open(&path).unwrap();
            db.record(&sample_event("elie.net", Decision::Allowed)).unwrap();
        }

        // Reopen and verify
        let db = WebDb::open(&path).unwrap();
        assert_eq!(db.count().unwrap(), 1);
    }

    // -- Insert and query --

    #[test]
    fn record_and_query_roundtrip() {
        let db = WebDb::open_in_memory().unwrap();
        let event = sample_event("elie.net", Decision::Allowed);
        db.record(&event).unwrap();

        let results = db.recent(10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].domain, "elie.net");
        assert_eq!(results[0].port, 443);
        assert_eq!(results[0].decision, Decision::Allowed);
        assert_eq!(results[0].bytes_sent, 1024);
        assert_eq!(results[0].bytes_received, 4096);
        assert_eq!(results[0].duration_ms, 150);
        assert_eq!(results[0].matched_rule, Some("test".to_string()));
        // Extended fields should be None.
        assert_eq!(results[0].method, None);
        assert_eq!(results[0].path, None);
        assert_eq!(results[0].status_code, None);
    }

    #[test]
    fn record_denied_event() {
        let db = WebDb::open_in_memory().unwrap();
        db.record(&sample_event("example.com", Decision::Denied)).unwrap();

        let results = db.recent(10).unwrap();
        assert_eq!(results[0].decision, Decision::Denied);
    }

    #[test]
    fn record_error_event() {
        let db = WebDb::open_in_memory().unwrap();
        db.record(&sample_event("broken.com", Decision::Error)).unwrap();

        let results = db.recent(10).unwrap();
        assert_eq!(results[0].decision, Decision::Error);
    }

    #[test]
    fn record_with_no_matched_rule() {
        let db = WebDb::open_in_memory().unwrap();
        let mut event = sample_event("elie.net", Decision::Allowed);
        event.matched_rule = None;
        db.record(&event).unwrap();

        let results = db.recent(10).unwrap();
        assert_eq!(results[0].matched_rule, None);
    }

    // -- Extended fields --

    #[test]
    fn record_http_event_all_fields() {
        let db = WebDb::open_in_memory().unwrap();
        let event = http_event("github.com");
        db.record(&event).unwrap();

        let results = db.recent(10).unwrap();
        assert_eq!(results.len(), 1);
        let r = &results[0];
        assert_eq!(r.domain, "github.com");
        assert_eq!(r.method, Some("GET".to_string()));
        assert_eq!(r.path, Some("/api/v1/repos".to_string()));
        assert_eq!(r.status_code, Some(200));
        assert_eq!(r.request_headers, Some("Host: github.com\r\nUser-Agent: curl".to_string()));
        assert_eq!(r.response_headers, Some("Content-Type: application/json".to_string()));
        assert_eq!(r.request_body_preview, None);
        assert_eq!(r.response_body_preview, Some("{\"repos\":[]}".to_string()));
        assert_eq!(r.conn_type, Some("https".to_string()));
    }

    #[test]
    fn record_domain_only_event() {
        // Domain-only events (SNI proxy style) should still work with None extended fields.
        let db = WebDb::open_in_memory().unwrap();
        let event = sample_event("elie.net", Decision::Allowed);
        db.record(&event).unwrap();

        let results = db.recent(10).unwrap();
        assert_eq!(results[0].method, None);
        assert_eq!(results[0].path, None);
        assert_eq!(results[0].status_code, None);
        assert_eq!(results[0].conn_type, None);
    }

    // -- Migration --

    #[test]
    fn migrate_from_old_schema() {
        // Simulate an old database without extended columns.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE http_requests (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                domain TEXT NOT NULL,
                port INTEGER NOT NULL,
                decision TEXT NOT NULL,
                bytes_sent INTEGER DEFAULT 0,
                bytes_received INTEGER DEFAULT 0,
                duration_ms INTEGER DEFAULT 0,
                reason TEXT
            );"
        ).unwrap();

        // Insert an old-style event.
        conn.execute(
            "INSERT INTO http_requests (timestamp, domain, port, decision, bytes_sent, bytes_received, duration_ms, reason)
             VALUES ('2023-11-14T00:00:00Z', 'elie.net', 443, 'allowed', 1024, 4096, 150, 'test')",
            [],
        ).unwrap();

        // Run migration.
        WebDb::migrate(&conn).unwrap();

        // Verify new columns exist and old data is preserved.
        let db = WebDb { conn };
        assert_eq!(db.count().unwrap(), 1);
        let results = db.recent(10).unwrap();
        assert_eq!(results[0].domain, "elie.net");
        assert_eq!(results[0].method, None);
        assert_eq!(results[0].path, None);
        assert_eq!(results[0].status_code, None);

        // Insert a new-style event.
        db.record(&http_event("github.com")).unwrap();
        assert_eq!(db.count().unwrap(), 2);
        let results = db.recent(10).unwrap();
        assert_eq!(results[0].method, Some("GET".to_string()));
    }

    // -- Recent ordering --

    #[test]
    fn recent_returns_newest_first() {
        let db = WebDb::open_in_memory().unwrap();
        for i in 0..5 {
            let mut event = sample_event(&format!("site{i}.com"), Decision::Allowed);
            event.timestamp = SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000 + i);
            db.record(&event).unwrap();
        }

        let results = db.recent(3).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].domain, "site4.com"); // newest
        assert_eq!(results[1].domain, "site3.com");
        assert_eq!(results[2].domain, "site2.com");
    }

    #[test]
    fn recent_with_limit() {
        let db = WebDb::open_in_memory().unwrap();
        for i in 0..10 {
            db.record(&sample_event(&format!("site{i}.com"), Decision::Allowed)).unwrap();
        }

        let results = db.recent(3).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(db.count().unwrap(), 10);
    }

    #[test]
    fn recent_empty_db() {
        let db = WebDb::open_in_memory().unwrap();
        let results = db.recent(10).unwrap();
        assert!(results.is_empty());
    }

    // -- Count --

    #[test]
    fn count_multiple_events() {
        let db = WebDb::open_in_memory().unwrap();
        for i in 0..7 {
            db.record(&sample_event(&format!("site{i}.com"), Decision::Allowed)).unwrap();
        }
        assert_eq!(db.count().unwrap(), 7);
    }

    // -- Count by decision --

    #[test]
    fn count_by_decision_empty() {
        let db = WebDb::open_in_memory().unwrap();
        assert_eq!(db.count_by_decision().unwrap(), (0, 0, 0));
    }

    #[test]
    fn count_by_decision_mixed() {
        let db = WebDb::open_in_memory().unwrap();
        for _ in 0..3 {
            db.record(&sample_event("a.com", Decision::Allowed)).unwrap();
        }
        for _ in 0..2 {
            db.record(&sample_event("b.com", Decision::Denied)).unwrap();
        }
        db.record(&sample_event("c.com", Decision::Error)).unwrap();
        assert_eq!(db.count_by_decision().unwrap(), (6, 3, 2));
    }

    #[test]
    fn count_by_decision_only_allowed() {
        let db = WebDb::open_in_memory().unwrap();
        for _ in 0..4 {
            db.record(&sample_event("a.com", Decision::Allowed)).unwrap();
        }
        assert_eq!(db.count_by_decision().unwrap(), (4, 4, 0));
    }

    #[test]
    fn count_by_decision_only_denied() {
        let db = WebDb::open_in_memory().unwrap();
        for _ in 0..3 {
            db.record(&sample_event("a.com", Decision::Denied)).unwrap();
        }
        assert_eq!(db.count_by_decision().unwrap(), (3, 0, 3));
    }

    // -- Decision serialization --

    #[test]
    fn decision_roundtrip() {
        for decision in [Decision::Allowed, Decision::Denied, Decision::Error] {
            assert_eq!(Decision::from_str(decision.as_str()), decision);
        }
    }

    #[test]
    fn decision_json_roundtrip() {
        let event = sample_event("elie.net", Decision::Allowed);
        let json = serde_json::to_string(&event).unwrap();
        let decoded: NetEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.decision, Decision::Allowed);
        assert_eq!(decoded.domain, "elie.net");
    }

    // -- Concurrent writes --

    #[test]
    fn concurrent_writes_from_threads() {
        use std::sync::Arc;

        let db = Arc::new(std::sync::Mutex::new(WebDb::open_in_memory().unwrap()));
        let mut handles = Vec::new();

        for i in 0..10 {
            let db = Arc::clone(&db);
            handles.push(std::thread::spawn(move || {
                let event = sample_event(&format!("thread{i}.com"), Decision::Allowed);
                let db = db.lock().unwrap();
                db.record(&event).unwrap();
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        let db = db.lock().unwrap();
        assert_eq!(db.count().unwrap(), 10);
    }
}
