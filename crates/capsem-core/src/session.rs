/// Session management: unique session IDs, session index DB, and lifecycle.
///
/// Each VM boot creates a new session with a unique ID (YYYYMMDD-HHMMSS-XXXX).
/// The session index (`main.db`) tracks metadata across sessions. Per-session
/// telemetry lives in `<session_dir>/info.db`.
use std::path::Path;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// Generate a unique session ID: YYYYMMDD-HHMMSS-XXXX (4 random hex chars).
pub fn generate_session_id() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let (y, m, d, hours, minutes, seconds) = epoch_to_parts(secs);

    // 4 random hex chars from timestamp nanos + XOR with a counter.
    let nanos = now.subsec_nanos();
    let rand_bits = nanos ^ std::process::id().wrapping_mul(2654435761);
    let suffix = rand_bits & 0xFFFF;

    format!(
        "{y:04}{m:02}{d:02}-{hours:02}{minutes:02}{seconds:02}-{suffix:04x}",
    )
}

/// Validate that a string looks like a valid session ID.
pub fn is_valid_session_id(s: &str) -> bool {
    // YYYYMMDD-HHMMSS-XXXX = 20 chars
    if s.len() != 20 {
        return false;
    }
    let bytes = s.as_bytes();
    // Check structure: 8 digits, dash, 6 digits, dash, 4 hex
    bytes[0..8].iter().all(|b| b.is_ascii_digit())
        && bytes[8] == b'-'
        && bytes[9..15].iter().all(|b| b.is_ascii_digit())
        && bytes[15] == b'-'
        && bytes[16..20].iter().all(|b| b.is_ascii_hexdigit())
}

/// A session record stored in main.db.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub mode: String,
    pub command: Option<String>,
    pub status: String,
    pub created_at: String,
    pub stopped_at: Option<String>,
    pub scratch_disk_size_gb: u32,
    pub ram_bytes: u64,
    pub total_requests: u64,
    pub allowed_requests: u64,
    pub denied_requests: u64,
}

/// Session index database wrapping `~/.capsem/sessions/main.db`.
pub struct SessionIndex {
    conn: Connection,
}

const SESSION_SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS sessions (
        id TEXT PRIMARY KEY,
        mode TEXT NOT NULL,
        command TEXT,
        status TEXT NOT NULL DEFAULT 'running',
        created_at TEXT NOT NULL,
        stopped_at TEXT,
        scratch_disk_size_gb INTEGER NOT NULL DEFAULT 16,
        ram_bytes INTEGER NOT NULL DEFAULT 4294967296,
        total_requests INTEGER NOT NULL DEFAULT 0,
        allowed_requests INTEGER NOT NULL DEFAULT 0,
        denied_requests INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX IF NOT EXISTS idx_sessions_created
        ON sessions(created_at);
    CREATE INDEX IF NOT EXISTS idx_sessions_status
        ON sessions(status);
";

impl SessionIndex {
    /// Open (or create) the session index at the given path.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SESSION_SCHEMA)?;
        Ok(Self { conn })
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SESSION_SCHEMA)?;
        Ok(Self { conn })
    }

    /// Insert a new session record.
    pub fn create_session(&self, record: &SessionRecord) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, mode, command, status, created_at, stopped_at,
                scratch_disk_size_gb, ram_bytes, total_requests, allowed_requests, denied_requests)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                record.id,
                record.mode,
                record.command,
                record.status,
                record.created_at,
                record.stopped_at,
                record.scratch_disk_size_gb as i64,
                record.ram_bytes as i64,
                record.total_requests as i64,
                record.allowed_requests as i64,
                record.denied_requests as i64,
            ],
        )?;
        Ok(())
    }

    /// Update session status and optionally set stopped_at.
    pub fn update_status(
        &self,
        id: &str,
        status: &str,
        stopped_at: Option<&str>,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET status = ?1, stopped_at = ?2 WHERE id = ?3",
            params![status, stopped_at, id],
        )?;
        Ok(())
    }

    /// Update request counts for a session.
    pub fn update_request_counts(
        &self,
        id: &str,
        total: u64,
        allowed: u64,
        denied: u64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET total_requests = ?1, allowed_requests = ?2, denied_requests = ?3
             WHERE id = ?4",
            params![total as i64, allowed as i64, denied as i64, id],
        )?;
        Ok(())
    }

    /// Mark all "running" sessions as "crashed". Returns count of affected rows.
    pub fn mark_running_as_crashed(&self) -> rusqlite::Result<usize> {
        let count = self.conn.execute(
            "UPDATE sessions SET status = 'crashed' WHERE status = 'running'",
            [],
        )?;
        Ok(count)
    }

    /// Query the most recent N sessions, newest first.
    pub fn recent(&self, limit: usize) -> rusqlite::Result<Vec<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, mode, command, status, created_at, stopped_at,
                    scratch_disk_size_gb, ram_bytes, total_requests, allowed_requests, denied_requests
             FROM sessions
             ORDER BY created_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(SessionRecord {
                id: row.get(0)?,
                mode: row.get(1)?,
                command: row.get(2)?,
                status: row.get(3)?,
                created_at: row.get(4)?,
                stopped_at: row.get(5)?,
                scratch_disk_size_gb: row.get::<_, i64>(6)? as u32,
                ram_bytes: row.get::<_, i64>(7)? as u64,
                total_requests: row.get::<_, i64>(8)? as u64,
                allowed_requests: row.get::<_, i64>(9)? as u64,
                denied_requests: row.get::<_, i64>(10)? as u64,
            })
        })?;
        rows.collect()
    }

    /// Delete sessions with created_at older than `days` days ago.
    /// Only deletes stopped/crashed sessions (not running).
    /// Returns count of deleted rows.
    pub fn delete_older_than_days(&self, days: u32) -> rusqlite::Result<usize> {
        let cutoff_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_sub(days as u64 * 86400);
        // created_at is ISO 8601 -- string comparison works for our format.
        let cutoff_str = epoch_to_iso(cutoff_secs);
        let count = self.conn.execute(
            "DELETE FROM sessions WHERE created_at < ?1 AND status IN ('stopped', 'crashed')",
            params![cutoff_str],
        )?;
        Ok(count)
    }

    /// Delete oldest sessions, keeping only the newest `max` sessions.
    /// Only deletes stopped/crashed sessions (not running).
    /// Returns count of deleted rows.
    pub fn delete_keeping_newest(&self, max: usize) -> rusqlite::Result<usize> {
        // Count non-running sessions.
        let count = self.conn.execute(
            "DELETE FROM sessions WHERE status IN ('stopped', 'crashed')
             AND id NOT IN (
                SELECT id FROM sessions ORDER BY created_at DESC LIMIT ?1
             )",
            params![max as i64],
        )?;
        Ok(count)
    }

    /// Return stopped/crashed sessions ordered oldest first (for disk culling).
    pub fn stopped_sessions_oldest_first(&self) -> rusqlite::Result<Vec<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, mode, command, status, created_at, stopped_at,
                    scratch_disk_size_gb, ram_bytes, total_requests, allowed_requests, denied_requests
             FROM sessions
             WHERE status IN ('stopped', 'crashed')
             ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SessionRecord {
                id: row.get(0)?,
                mode: row.get(1)?,
                command: row.get(2)?,
                status: row.get(3)?,
                created_at: row.get(4)?,
                stopped_at: row.get(5)?,
                scratch_disk_size_gb: row.get::<_, i64>(6)? as u32,
                ram_bytes: row.get::<_, i64>(7)? as u64,
                total_requests: row.get::<_, i64>(8)? as u64,
                allowed_requests: row.get::<_, i64>(9)? as u64,
                denied_requests: row.get::<_, i64>(10)? as u64,
            })
        })?;
        rows.collect()
    }

    /// Total count of sessions.
    pub fn count(&self) -> rusqlite::Result<usize> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM sessions",
            [],
            |row| row.get::<_, i64>(0).map(|n| n as usize),
        )
    }
}

/// Break epoch seconds into (year, month, day, hour, minute, second) UTC components.
fn epoch_to_parts(secs: u64) -> (i64, u32, u32, u64, u64, u64) {
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let mut y = 1970i64;
    let mut remaining_days = days as i64;
    loop {
        let year_days = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            366
        } else {
            365
        };
        if remaining_days < year_days {
            break;
        }
        remaining_days -= year_days;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31, 30, 31, 30, 31, 31, 30, 31, 30, 31,
    ];
    let mut m = 0u32;
    for md in &month_days {
        if remaining_days < *md {
            break;
        }
        remaining_days -= md;
        m += 1;
    }
    (y, m + 1, remaining_days as u32 + 1, hours, minutes, seconds)
}

/// Convert epoch seconds to ISO 8601 string (YYYY-MM-DDTHH:MM:SSZ).
pub fn epoch_to_iso(secs: u64) -> String {
    let (y, m, d, hours, minutes, seconds) = epoch_to_parts(secs);
    format!("{y:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Current UTC time as ISO 8601 string.
pub fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    epoch_to_iso(secs)
}

/// Calculate total disk usage in bytes for all session directories under the given base path.
pub fn disk_usage_bytes(sessions_base: &Path) -> u64 {
    let entries = match std::fs::read_dir(sessions_base) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            total += dir_size(&path);
        } else if path.is_file() {
            total += path.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    total
}

fn dir_size(path: &Path) -> u64 {
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            total += dir_size(&p);
        } else if p.is_file() {
            total += p.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- ID generation --

    #[test]
    fn generate_session_id_format() {
        let id = generate_session_id();
        assert_eq!(id.len(), 20, "id={id}");
        assert!(is_valid_session_id(&id), "id={id}");
    }

    #[test]
    fn two_rapid_calls_differ() {
        let id1 = generate_session_id();
        // Bump PID-based entropy by sleeping briefly.
        std::thread::sleep(std::time::Duration::from_millis(1));
        let id2 = generate_session_id();
        assert_ne!(id1, id2, "ids should differ: {id1} vs {id2}");
    }

    #[test]
    fn is_valid_session_id_accepts_valid() {
        assert!(is_valid_session_id("20260225-143052-a7f3"));
        assert!(is_valid_session_id("20260101-000000-0000"));
        assert!(is_valid_session_id("20260225-235959-ffff"));
    }

    #[test]
    fn is_valid_session_id_rejects_invalid() {
        assert!(!is_valid_session_id("default"));
        assert!(!is_valid_session_id("cli"));
        assert!(!is_valid_session_id(""));
        assert!(!is_valid_session_id("2026022514305-a7f3")); // missing digit
        assert!(!is_valid_session_id("20260225-14305-a7f3x")); // wrong length
        assert!(!is_valid_session_id("XXXXXXXX-XXXXXX-XXXX")); // not digits
    }

    // -- SessionIndex CRUD --

    fn sample_record(id: &str, status: &str) -> SessionRecord {
        SessionRecord {
            id: id.to_string(),
            mode: "gui".to_string(),
            command: None,
            status: status.to_string(),
            created_at: "2026-02-25T14:30:52Z".to_string(),
            stopped_at: None,
            scratch_disk_size_gb: 16,
            ram_bytes: 4 * 1024 * 1024 * 1024,
            total_requests: 0,
            allowed_requests: 0,
            denied_requests: 0,
        }
    }

    #[test]
    fn open_creates_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.db");
        let idx = SessionIndex::open(&path).unwrap();
        assert_eq!(idx.count().unwrap(), 0);
        assert!(path.exists());
    }

    #[test]
    fn open_preserves_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.db");
        {
            let idx = SessionIndex::open(&path).unwrap();
            idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
                .unwrap();
        }
        let idx = SessionIndex::open(&path).unwrap();
        assert_eq!(idx.count().unwrap(), 1);
    }

    #[test]
    fn open_in_memory_works() {
        let idx = SessionIndex::open_in_memory().unwrap();
        assert_eq!(idx.count().unwrap(), 0);
    }

    #[test]
    fn create_and_recent() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
            .unwrap();
        let records = idx.recent(1).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "20260225-143052-a7f3");
        assert_eq!(records[0].mode, "gui");
        assert_eq!(records[0].status, "running");
    }

    #[test]
    fn create_duplicate_returns_error() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
            .unwrap();
        let result = idx.create_session(&sample_record("20260225-143052-a7f3", "running"));
        assert!(result.is_err());
    }

    #[test]
    fn recent_newest_first() {
        let idx = SessionIndex::open_in_memory().unwrap();
        for (i, ts) in ["2026-02-25T10:00:00Z", "2026-02-25T12:00:00Z", "2026-02-25T11:00:00Z"]
            .iter()
            .enumerate()
        {
            let mut rec = sample_record(&format!("20260225-{i:06}-0000"), "stopped");
            rec.created_at = ts.to_string();
            idx.create_session(&rec).unwrap();
        }
        let records = idx.recent(10).unwrap();
        assert_eq!(records[0].created_at, "2026-02-25T12:00:00Z");
        assert_eq!(records[1].created_at, "2026-02-25T11:00:00Z");
        assert_eq!(records[2].created_at, "2026-02-25T10:00:00Z");
    }

    #[test]
    fn recent_respects_limit() {
        let idx = SessionIndex::open_in_memory().unwrap();
        for i in 0..5 {
            let mut rec = sample_record(&format!("20260225-{i:06}-0000"), "stopped");
            rec.created_at = format!("2026-02-25T{i:02}:00:00Z");
            idx.create_session(&rec).unwrap();
        }
        assert_eq!(idx.recent(2).unwrap().len(), 2);
    }

    #[test]
    fn recent_empty_db() {
        let idx = SessionIndex::open_in_memory().unwrap();
        assert!(idx.recent(10).unwrap().is_empty());
    }

    #[test]
    fn update_status_works() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
            .unwrap();
        idx.update_status("20260225-143052-a7f3", "stopped", Some("2026-02-25T15:00:00Z"))
            .unwrap();
        let records = idx.recent(1).unwrap();
        assert_eq!(records[0].status, "stopped");
        assert_eq!(
            records[0].stopped_at.as_deref(),
            Some("2026-02-25T15:00:00Z")
        );
    }

    #[test]
    fn update_status_nonexistent_is_noop() {
        let idx = SessionIndex::open_in_memory().unwrap();
        // Should not crash.
        idx.update_status("nonexistent", "stopped", None).unwrap();
    }

    #[test]
    fn update_request_counts_works() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
            .unwrap();
        idx.update_request_counts("20260225-143052-a7f3", 10, 7, 3)
            .unwrap();
        let records = idx.recent(1).unwrap();
        assert_eq!(records[0].total_requests, 10);
        assert_eq!(records[0].allowed_requests, 7);
        assert_eq!(records[0].denied_requests, 3);
    }

    #[test]
    fn count_correct() {
        let idx = SessionIndex::open_in_memory().unwrap();
        assert_eq!(idx.count().unwrap(), 0);
        idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
            .unwrap();
        assert_eq!(idx.count().unwrap(), 1);
        idx.create_session(&sample_record("20260225-143053-b8e4", "stopped"))
            .unwrap();
        assert_eq!(idx.count().unwrap(), 2);
    }

    // -- Crash recovery --

    #[test]
    fn mark_running_as_crashed() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running"))
            .unwrap();
        idx.create_session(&sample_record("20260225-143053-b8e4", "running"))
            .unwrap();
        idx.create_session(&sample_record("20260225-143054-c9d5", "stopped"))
            .unwrap();

        let count = idx.mark_running_as_crashed().unwrap();
        assert_eq!(count, 2);

        let records = idx.recent(10).unwrap();
        for r in &records {
            if r.id == "20260225-143054-c9d5" {
                assert_eq!(r.status, "stopped");
            } else {
                assert_eq!(r.status, "crashed");
            }
        }
    }

    #[test]
    fn mark_running_as_crashed_ignores_stopped() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "stopped"))
            .unwrap();
        idx.create_session(&sample_record("20260225-143053-b8e4", "crashed"))
            .unwrap();
        let count = idx.mark_running_as_crashed().unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn mark_running_as_crashed_empty_db() {
        let idx = SessionIndex::open_in_memory().unwrap();
        let count = idx.mark_running_as_crashed().unwrap();
        assert_eq!(count, 0);
    }

    // -- Age-based culling --

    #[test]
    fn delete_older_than_days() {
        let idx = SessionIndex::open_in_memory().unwrap();

        // Old session (2020).
        let mut old = sample_record("20200101-120000-0000", "stopped");
        old.created_at = "2020-01-01T12:00:00Z".to_string();
        idx.create_session(&old).unwrap();

        // Recent session.
        let mut recent = sample_record("20260225-143052-a7f3", "stopped");
        recent.created_at = "2026-02-25T14:30:52Z".to_string();
        idx.create_session(&recent).unwrap();

        let deleted = idx.delete_older_than_days(7).unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(idx.count().unwrap(), 1);
        assert_eq!(idx.recent(1).unwrap()[0].id, "20260225-143052-a7f3");
    }

    #[test]
    fn delete_older_preserves_running() {
        let idx = SessionIndex::open_in_memory().unwrap();

        let mut old_running = sample_record("20200101-120000-0000", "running");
        old_running.created_at = "2020-01-01T12:00:00Z".to_string();
        idx.create_session(&old_running).unwrap();

        let deleted = idx.delete_older_than_days(7).unwrap();
        assert_eq!(deleted, 0);
        assert_eq!(idx.count().unwrap(), 1);
    }

    // -- Count-based culling --

    #[test]
    fn delete_keeping_newest() {
        let idx = SessionIndex::open_in_memory().unwrap();
        for i in 0..5 {
            let mut rec = sample_record(&format!("20260225-{i:06}-0000"), "stopped");
            rec.created_at = format!("2026-02-25T{i:02}:00:00Z");
            idx.create_session(&rec).unwrap();
        }
        let deleted = idx.delete_keeping_newest(3).unwrap();
        assert_eq!(deleted, 2);
        assert_eq!(idx.count().unwrap(), 3);
    }

    #[test]
    fn delete_keeping_newest_ignores_running() {
        let idx = SessionIndex::open_in_memory().unwrap();
        for i in 0..3 {
            let mut rec = sample_record(&format!("20260225-{i:06}-0000"), "stopped");
            rec.created_at = format!("2026-02-25T{i:02}:00:00Z");
            idx.create_session(&rec).unwrap();
        }
        let mut running = sample_record("20260225-100000-0000", "running");
        running.created_at = "2026-02-24T00:00:00Z".to_string();
        idx.create_session(&running).unwrap();

        let deleted = idx.delete_keeping_newest(2).unwrap();
        assert_eq!(deleted, 1);
        // 2 stopped + 1 running = 3
        assert_eq!(idx.count().unwrap(), 3);
    }

    #[test]
    fn delete_keeping_newest_noop_under_cap() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "stopped"))
            .unwrap();
        let deleted = idx.delete_keeping_newest(10).unwrap();
        assert_eq!(deleted, 0);
    }

    // -- Disk culling helper --

    #[test]
    fn stopped_sessions_oldest_first() {
        let idx = SessionIndex::open_in_memory().unwrap();

        let mut s1 = sample_record("20260225-100000-0000", "stopped");
        s1.created_at = "2026-02-25T10:00:00Z".to_string();
        idx.create_session(&s1).unwrap();

        let mut s2 = sample_record("20260225-120000-0000", "crashed");
        s2.created_at = "2026-02-25T12:00:00Z".to_string();
        idx.create_session(&s2).unwrap();

        let mut s3 = sample_record("20260225-080000-0000", "running");
        s3.created_at = "2026-02-25T08:00:00Z".to_string();
        idx.create_session(&s3).unwrap();

        let stopped = idx.stopped_sessions_oldest_first().unwrap();
        assert_eq!(stopped.len(), 2); // running excluded
        assert_eq!(stopped[0].id, "20260225-100000-0000");
        assert_eq!(stopped[1].id, "20260225-120000-0000");
    }

    // -- Disk usage --

    #[test]
    fn disk_usage_bytes_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(disk_usage_bytes(dir.path()), 0);
    }

    #[test]
    fn disk_usage_bytes_with_files() {
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("20260225-143052-a7f3");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(session.join("info.db"), vec![0u8; 4096]).unwrap();
        let usage = disk_usage_bytes(dir.path());
        assert!(usage >= 4096, "usage={usage}");
    }

    // -- epoch_to_iso --

    #[test]
    fn epoch_to_iso_unix_epoch() {
        assert_eq!(epoch_to_iso(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn epoch_to_iso_known_date() {
        // 2026-02-25T14:30:52Z = known epoch
        let iso = epoch_to_iso(1772126052);
        assert!(iso.starts_with("2026-"), "iso={iso}");
    }
}
