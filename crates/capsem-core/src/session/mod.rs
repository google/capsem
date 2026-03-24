//! Session management: unique session IDs, session index DB, and lifecycle.

mod types;
mod index;
mod maintenance;

pub use types::*;
pub use index::*;
pub use maintenance::*;

#[cfg(test)]
mod tests {
    use super::*;
    use super::index::SCHEMA_VERSION;
    use rusqlite::{params, Connection};

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
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_estimated_cost: 0.0,
            total_tool_calls: 0,
            total_mcp_calls: 0,
            total_file_events: 0,
            compressed_size_bytes: None,
            vacuumed_at: None,
            storage_mode: "block".to_string(),
            rootfs_hash: None,
            rootfs_version: None,
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
    fn terminate_older_than_days() {
        let idx = SessionIndex::open_in_memory().unwrap();

        // Old session (2020).
        let mut old = sample_record("20200101-120000-0000", "stopped");
        old.created_at = "2020-01-01T12:00:00Z".to_string();
        idx.create_session(&old).unwrap();

        // Recent session (use a date far in the future to avoid flaking).
        let mut recent = sample_record("20260225-143052-a7f3", "stopped");
        recent.created_at = "2099-01-01T00:00:00Z".to_string();
        idx.create_session(&recent).unwrap();

        let terminated = idx.terminate_older_than_days(7).unwrap();
        assert_eq!(terminated, 1);
        // Row still exists, just status changed.
        assert_eq!(idx.count().unwrap(), 2);
        let records = idx.recent(10).unwrap();
        let old_rec = records.iter().find(|r| r.id == "20200101-120000-0000").unwrap();
        assert_eq!(old_rec.status, "terminated");
    }

    #[test]
    fn terminate_older_preserves_running() {
        let idx = SessionIndex::open_in_memory().unwrap();

        let mut old_running = sample_record("20200101-120000-0000", "running");
        old_running.created_at = "2020-01-01T12:00:00Z".to_string();
        idx.create_session(&old_running).unwrap();

        let terminated = idx.terminate_older_than_days(7).unwrap();
        assert_eq!(terminated, 0);
        assert_eq!(idx.recent(1).unwrap()[0].status, "running");
    }

    #[test]
    fn terminate_older_includes_vacuumed() {
        let idx = SessionIndex::open_in_memory().unwrap();

        let mut old = sample_record("20200101-120000-0000", "vacuumed");
        old.created_at = "2020-01-01T12:00:00Z".to_string();
        idx.create_session(&old).unwrap();

        let terminated = idx.terminate_older_than_days(7).unwrap();
        assert_eq!(terminated, 1);
        let records = idx.recent(1).unwrap();
        assert_eq!(records[0].status, "terminated");
    }

    // -- Count-based culling --

    #[test]
    fn terminate_excess_sessions() {
        let idx = SessionIndex::open_in_memory().unwrap();
        for i in 0..5 {
            let mut rec = sample_record(&format!("20260225-{i:06}-0000"), "stopped");
            rec.created_at = format!("2026-02-25T{i:02}:00:00Z");
            idx.create_session(&rec).unwrap();
        }
        let terminated = idx.terminate_excess_sessions(3).unwrap();
        assert_eq!(terminated, 2);
        // All rows still exist, 2 are now terminated.
        assert_eq!(idx.count().unwrap(), 5);
        let terminated_recs = idx.sessions_by_status("terminated").unwrap();
        assert_eq!(terminated_recs.len(), 2);
    }

    #[test]
    fn terminate_excess_ignores_running() {
        let idx = SessionIndex::open_in_memory().unwrap();
        for i in 0..3 {
            let mut rec = sample_record(&format!("20260225-{i:06}-0000"), "stopped");
            rec.created_at = format!("2026-02-25T{i:02}:00:00Z");
            idx.create_session(&rec).unwrap();
        }
        let mut running = sample_record("20260225-100000-0000", "running");
        running.created_at = "2026-02-24T00:00:00Z".to_string();
        idx.create_session(&running).unwrap();

        let terminated = idx.terminate_excess_sessions(2).unwrap();
        assert_eq!(terminated, 1);
        // running session untouched.
        let r = idx.recent(10).unwrap();
        assert!(r.iter().any(|rec| rec.id == "20260225-100000-0000" && rec.status == "running"));
    }

    #[test]
    fn terminate_excess_noop_under_cap() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "stopped"))
            .unwrap();
        let terminated = idx.terminate_excess_sessions(10).unwrap();
        assert_eq!(terminated, 0);
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

    // -- Schema version --

    #[test]
    fn schema_version_is_set() {
        let idx = SessionIndex::open_in_memory().unwrap();
        let version: u32 = idx.conn.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn schema_upgrade_from_v0() {
        // Simulate a v0 DB (no user_version set = 0).
        let conn = Connection::open_in_memory().unwrap();
        // Create old-style sessions table without new columns.
        conn.execute_batch("CREATE TABLE sessions (id TEXT PRIMARY KEY, mode TEXT NOT NULL)").unwrap();
        conn.execute("INSERT INTO sessions (id, mode) VALUES ('old', 'gui')", []).unwrap();
        // Now ensure_schema should drop and recreate (v0 < v2 path).
        SessionIndex::ensure_schema(&conn).unwrap();
        let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        // Old data is gone (clean slate).
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn schema_upgrade_from_v1() {
        // v1 < v2, so same drop+recreate behavior.
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 1u32).unwrap();
        conn.execute_batch("CREATE TABLE sessions (id TEXT PRIMARY KEY)").unwrap();
        SessionIndex::ensure_schema(&conn).unwrap();
        let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn schema_same_version_preserves_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.db");
        {
            let idx = SessionIndex::open(&path).unwrap();
            idx.create_session(&sample_record("20260225-143052-a7f3", "running")).unwrap();
        }
        // Reopen -- same version, data preserved.
        let idx = SessionIndex::open(&path).unwrap();
        assert_eq!(idx.count().unwrap(), 1);
    }

    // -- New columns default to zero --

    #[test]
    fn new_columns_default_to_zero() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running")).unwrap();
        let records = idx.recent(1).unwrap();
        assert_eq!(records[0].total_input_tokens, 0);
        assert_eq!(records[0].total_output_tokens, 0);
        assert_eq!(records[0].total_estimated_cost, 0.0);
        assert_eq!(records[0].total_tool_calls, 0);
        assert_eq!(records[0].total_mcp_calls, 0);
        assert_eq!(records[0].total_file_events, 0);
    }

    // -- update_session_summary --

    #[test]
    fn update_session_summary_works() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running")).unwrap();
        idx.update_session_summary("20260225-143052-a7f3", 1000, 500, 0.15, 42, 5, 100).unwrap();
        let records = idx.recent(1).unwrap();
        assert_eq!(records[0].total_input_tokens, 1000);
        assert_eq!(records[0].total_output_tokens, 500);
        assert!((records[0].total_estimated_cost - 0.15).abs() < 1e-6);
        assert_eq!(records[0].total_tool_calls, 42);
        assert_eq!(records[0].total_mcp_calls, 5);
        assert_eq!(records[0].total_file_events, 100);
    }

    // -- global_stats --

    #[test]
    fn global_stats_empty() {
        let idx = SessionIndex::open_in_memory().unwrap();
        let gs = idx.global_stats().unwrap();
        assert_eq!(gs.total_sessions, 0);
        assert_eq!(gs.total_input_tokens, 0);
        assert_eq!(gs.total_estimated_cost, 0.0);
    }

    #[test]
    fn global_stats_multi_session() {
        let idx = SessionIndex::open_in_memory().unwrap();
        let mut r1 = sample_record("20260225-143052-a7f3", "stopped");
        r1.total_input_tokens = 1000;
        r1.total_output_tokens = 500;
        r1.total_estimated_cost = 0.10;
        r1.total_tool_calls = 20;
        r1.total_mcp_calls = 3;
        r1.total_file_events = 50;
        r1.total_requests = 10;
        r1.allowed_requests = 8;
        r1.denied_requests = 2;
        idx.create_session(&r1).unwrap();

        let mut r2 = sample_record("20260225-143053-b8e4", "stopped");
        r2.created_at = "2026-02-25T14:30:53Z".to_string();
        r2.total_input_tokens = 2000;
        r2.total_output_tokens = 1000;
        r2.total_estimated_cost = 0.20;
        r2.total_tool_calls = 30;
        r2.total_mcp_calls = 7;
        r2.total_file_events = 25;
        r2.total_requests = 5;
        r2.allowed_requests = 4;
        r2.denied_requests = 1;
        idx.create_session(&r2).unwrap();

        let gs = idx.global_stats().unwrap();
        assert_eq!(gs.total_sessions, 2);
        assert_eq!(gs.total_input_tokens, 3000);
        assert_eq!(gs.total_output_tokens, 1500);
        assert!((gs.total_estimated_cost - 0.30).abs() < 1e-6);
        assert_eq!(gs.total_tool_calls, 50);
        assert_eq!(gs.total_mcp_calls, 10);
        assert_eq!(gs.total_file_events, 75);
        assert_eq!(gs.total_requests, 15);
        assert_eq!(gs.total_allowed, 12);
        assert_eq!(gs.total_denied, 3);
    }

    // -- replace_ai_usage + top_providers --

    #[test]
    fn replace_ai_usage_and_top_providers() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "stopped")).unwrap();

        let usage = vec![
            ProviderSummary { provider: "anthropic".into(), call_count: 10, input_tokens: 5000, output_tokens: 2000, estimated_cost: 0.10, total_duration_ms: 3000 },
            ProviderSummary { provider: "google".into(), call_count: 5, input_tokens: 2000, output_tokens: 1000, estimated_cost: 0.05, total_duration_ms: 1500 },
        ];
        idx.replace_ai_usage("20260225-143052-a7f3", &usage).unwrap();

        let providers = idx.top_providers(10).unwrap();
        assert_eq!(providers.len(), 2);
        assert_eq!(providers[0].provider, "anthropic"); // highest call_count first
        assert_eq!(providers[0].call_count, 10);
        assert_eq!(providers[1].provider, "google");
    }

    #[test]
    fn replace_ai_usage_replaces_old_data() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "stopped")).unwrap();

        let old = vec![
            ProviderSummary { provider: "anthropic".into(), call_count: 10, input_tokens: 5000, output_tokens: 2000, estimated_cost: 0.10, total_duration_ms: 3000 },
        ];
        idx.replace_ai_usage("20260225-143052-a7f3", &old).unwrap();

        let new = vec![
            ProviderSummary { provider: "openai".into(), call_count: 20, input_tokens: 8000, output_tokens: 4000, estimated_cost: 0.30, total_duration_ms: 5000 },
        ];
        idx.replace_ai_usage("20260225-143052-a7f3", &new).unwrap();

        let providers = idx.top_providers(10).unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].provider, "openai");
        assert_eq!(providers[0].call_count, 20);
    }

    // -- replace_tool_usage + top_tools --

    #[test]
    fn replace_tool_usage_and_top_tools() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "stopped")).unwrap();

        let usage = vec![
            ToolSummary { tool_name: "read_file".into(), call_count: 50, total_bytes: 100_000, total_duration_ms: 2000 },
            ToolSummary { tool_name: "write_file".into(), call_count: 30, total_bytes: 50_000, total_duration_ms: 1500 },
        ];
        idx.replace_tool_usage("20260225-143052-a7f3", &usage).unwrap();

        let tools = idx.top_tools(10).unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].tool_name, "read_file"); // highest count first
        assert_eq!(tools[0].call_count, 50);
    }

    // -- replace_mcp_usage + top_mcp_tools --

    #[test]
    fn replace_mcp_usage_and_top_mcp_tools() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "stopped")).unwrap();

        let usage = vec![
            McpToolSummary { tool_name: "github__search".into(), server_name: "github".into(), call_count: 15, total_bytes: 30_000, total_duration_ms: 4500 },
            McpToolSummary { tool_name: "fs__read".into(), server_name: "filesystem".into(), call_count: 8, total_bytes: 10_000, total_duration_ms: 800 },
        ];
        idx.replace_mcp_usage("20260225-143052-a7f3", &usage).unwrap();

        let tools = idx.top_mcp_tools(10).unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].tool_name, "github__search");
        assert_eq!(tools[0].server_name, "github");
        assert_eq!(tools[0].call_count, 15);
    }

    // -- Cross-session aggregation --

    #[test]
    fn top_providers_aggregates_across_sessions() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "stopped")).unwrap();
        let mut r2 = sample_record("20260225-143053-b8e4", "stopped");
        r2.created_at = "2026-02-25T14:30:53Z".to_string();
        idx.create_session(&r2).unwrap();

        idx.replace_ai_usage("20260225-143052-a7f3", &[
            ProviderSummary { provider: "anthropic".into(), call_count: 10, input_tokens: 5000, output_tokens: 2000, estimated_cost: 0.10, total_duration_ms: 3000 },
        ]).unwrap();
        idx.replace_ai_usage("20260225-143053-b8e4", &[
            ProviderSummary { provider: "anthropic".into(), call_count: 5, input_tokens: 2000, output_tokens: 1000, estimated_cost: 0.05, total_duration_ms: 1000 },
        ]).unwrap();

        let providers = idx.top_providers(10).unwrap();
        assert_eq!(providers.len(), 1); // grouped by provider
        assert_eq!(providers[0].call_count, 15);
        assert_eq!(providers[0].input_tokens, 7000);
    }

    // -- Schema migration v2->v3 --

    #[test]
    fn schema_upgrade_from_v2_preserves_data() {
        let conn = Connection::open_in_memory().unwrap();
        // Create a v2 schema manually.
        conn.pragma_update(None, "user_version", 2u32).unwrap();
        conn.execute_batch("
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY, mode TEXT NOT NULL, command TEXT,
                status TEXT NOT NULL DEFAULT 'running', created_at TEXT NOT NULL,
                stopped_at TEXT, scratch_disk_size_gb INTEGER NOT NULL DEFAULT 16,
                ram_bytes INTEGER NOT NULL DEFAULT 4294967296,
                total_requests INTEGER NOT NULL DEFAULT 0,
                allowed_requests INTEGER NOT NULL DEFAULT 0,
                denied_requests INTEGER NOT NULL DEFAULT 0,
                total_input_tokens INTEGER NOT NULL DEFAULT 0,
                total_output_tokens INTEGER NOT NULL DEFAULT 0,
                total_estimated_cost REAL NOT NULL DEFAULT 0.0,
                total_tool_calls INTEGER NOT NULL DEFAULT 0,
                total_mcp_calls INTEGER NOT NULL DEFAULT 0,
                total_file_events INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE ai_usage (session_id TEXT, provider TEXT, call_count INTEGER DEFAULT 0, input_tokens INTEGER DEFAULT 0, output_tokens INTEGER DEFAULT 0, estimated_cost REAL DEFAULT 0.0, total_duration_ms INTEGER DEFAULT 0, PRIMARY KEY (session_id, provider));
            CREATE TABLE tool_usage (session_id TEXT, tool_name TEXT, call_count INTEGER DEFAULT 0, total_bytes INTEGER DEFAULT 0, total_duration_ms INTEGER DEFAULT 0, PRIMARY KEY (session_id, tool_name));
            CREATE TABLE mcp_usage (session_id TEXT, tool_name TEXT, server_name TEXT, call_count INTEGER DEFAULT 0, total_bytes INTEGER DEFAULT 0, total_duration_ms INTEGER DEFAULT 0, PRIMARY KEY (session_id, tool_name));
        ").unwrap();
        conn.execute(
            "INSERT INTO sessions (id, mode, status, created_at) VALUES ('test-id', 'gui', 'stopped', '2026-01-01T00:00:00Z')",
            [],
        ).unwrap();

        // Migrate.
        SessionIndex::ensure_schema(&conn).unwrap();

        // Check version bumped.
        let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        // Old data preserved.
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);

        // New columns exist with NULL defaults.
        let compressed: Option<i64> = conn.query_row(
            "SELECT compressed_size_bytes FROM sessions WHERE id = 'test-id'", [], |row| row.get(0)
        ).unwrap();
        assert!(compressed.is_none());

        let vacuumed: Option<String> = conn.query_row(
            "SELECT vacuumed_at FROM sessions WHERE id = 'test-id'", [], |row| row.get(0)
        ).unwrap();
        assert!(vacuumed.is_none());
    }

    // -- New lifecycle methods --

    #[test]
    fn mark_vacuumed_sets_fields() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "stopped")).unwrap();
        idx.mark_vacuumed("20260225-143052-a7f3", 12345, "2026-02-25T15:00:00Z").unwrap();

        let records = idx.recent(1).unwrap();
        assert_eq!(records[0].status, "vacuumed");
        assert_eq!(records[0].compressed_size_bytes, Some(12345));
        assert_eq!(records[0].vacuumed_at.as_deref(), Some("2026-02-25T15:00:00Z"));
    }

    #[test]
    fn mark_terminated_sets_status() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "vacuumed")).unwrap();
        idx.mark_terminated("20260225-143052-a7f3").unwrap();

        let records = idx.recent(1).unwrap();
        assert_eq!(records[0].status, "terminated");
    }

    #[test]
    fn unvacuumed_sessions_returns_correct_set() {
        let idx = SessionIndex::open_in_memory().unwrap();

        // Stopped without vacuum -- should be returned.
        let mut s1 = sample_record("20260225-100000-0000", "stopped");
        s1.created_at = "2026-02-25T10:00:00Z".to_string();
        idx.create_session(&s1).unwrap();

        // Crashed without vacuum -- should be returned.
        let mut s2 = sample_record("20260225-110000-0000", "crashed");
        s2.created_at = "2026-02-25T11:00:00Z".to_string();
        idx.create_session(&s2).unwrap();

        // Running -- should NOT be returned.
        let mut s3 = sample_record("20260225-120000-0000", "running");
        s3.created_at = "2026-02-25T12:00:00Z".to_string();
        idx.create_session(&s3).unwrap();

        // Already vacuumed -- should NOT be returned.
        let mut s4 = sample_record("20260225-130000-0000", "vacuumed");
        s4.created_at = "2026-02-25T13:00:00Z".to_string();
        s4.vacuumed_at = Some("2026-02-25T14:00:00Z".to_string());
        idx.create_session(&s4).unwrap();

        let unvacuumed = idx.unvacuumed_sessions().unwrap();
        assert_eq!(unvacuumed.len(), 2);
        assert_eq!(unvacuumed[0].id, "20260225-100000-0000");
        assert_eq!(unvacuumed[1].id, "20260225-110000-0000");
    }

    #[test]
    fn sessions_by_status_filters_correctly() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "stopped")).unwrap();
        let mut r2 = sample_record("20260225-143053-b8e4", "running");
        r2.created_at = "2026-02-25T14:30:53Z".to_string();
        idx.create_session(&r2).unwrap();
        let mut r3 = sample_record("20260225-143054-c9d5", "stopped");
        r3.created_at = "2026-02-25T14:30:54Z".to_string();
        idx.create_session(&r3).unwrap();

        let stopped = idx.sessions_by_status("stopped").unwrap();
        assert_eq!(stopped.len(), 2);
        let running = idx.sessions_by_status("running").unwrap();
        assert_eq!(running.len(), 1);
        let terminated = idx.sessions_by_status("terminated").unwrap();
        assert_eq!(terminated.len(), 0);
    }

    #[test]
    fn purge_terminated_older_than_days() {
        let idx = SessionIndex::open_in_memory().unwrap();

        // Old terminated session.
        let mut old = sample_record("20200101-120000-0000", "terminated");
        old.created_at = "2020-01-01T12:00:00Z".to_string();
        idx.create_session(&old).unwrap();

        // Recent terminated session (use a date far in the future to avoid flaking).
        let mut recent = sample_record("20260225-143052-a7f3", "terminated");
        recent.created_at = "2099-01-01T00:00:00Z".to_string();
        idx.create_session(&recent).unwrap();

        // Non-terminated session.
        let mut stopped = sample_record("20200101-130000-0000", "stopped");
        stopped.created_at = "2020-01-01T13:00:00Z".to_string();
        idx.create_session(&stopped).unwrap();

        let purged = idx.purge_terminated_older_than_days(7).unwrap();
        assert_eq!(purged, 1); // only old terminated
        assert_eq!(idx.count().unwrap(), 2); // recent terminated + stopped remain
    }

    #[test]
    fn full_lifecycle_running_to_terminated() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running")).unwrap();

        // running -> stopped
        idx.update_status("20260225-143052-a7f3", "stopped", Some("2026-02-25T15:00:00Z")).unwrap();
        assert_eq!(idx.recent(1).unwrap()[0].status, "stopped");

        // stopped -> vacuumed
        idx.mark_vacuumed("20260225-143052-a7f3", 5000, "2026-02-25T15:01:00Z").unwrap();
        let rec = &idx.recent(1).unwrap()[0];
        assert_eq!(rec.status, "vacuumed");
        assert_eq!(rec.compressed_size_bytes, Some(5000));

        // vacuumed -> terminated
        idx.mark_terminated("20260225-143052-a7f3").unwrap();
        assert_eq!(idx.recent(1).unwrap()[0].status, "terminated");

        // Row still exists in the audit trail.
        assert_eq!(idx.count().unwrap(), 1);
    }

    #[test]
    fn checkpoint_succeeds_on_in_memory_db() {
        let idx = SessionIndex::open_in_memory().unwrap();
        // Should not error (even though in-memory WAL is a no-op).
        idx.checkpoint().unwrap();
    }

    #[test]
    fn new_columns_null_by_default() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running")).unwrap();
        let records = idx.recent(1).unwrap();
        assert!(records[0].compressed_size_bytes.is_none());
        assert!(records[0].vacuumed_at.is_none());
    }

    // -- Vacuum + compress --

    #[test]
    fn vacuum_and_compress_creates_gz_and_removes_db() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().join("20260225-143052-a7f3");
        std::fs::create_dir_all(&session_dir).unwrap();

        // Create a real session DB with some data.
        let db_path = session_dir.join("session.db");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.pragma_update(None, "journal_mode", "WAL").unwrap();
            conn.execute_batch("CREATE TABLE test (id INTEGER, data TEXT)").unwrap();
            for i in 0..100 {
                conn.execute("INSERT INTO test (id, data) VALUES (?1, ?2)", params![i, format!("row-{i}")]).unwrap();
            }
        }
        // Create fake WAL/SHM files.
        std::fs::write(session_dir.join("session.db-wal"), b"fake wal").unwrap();
        std::fs::write(session_dir.join("session.db-shm"), b"fake shm").unwrap();

        let compressed_size = vacuum_and_compress_session_db(&session_dir).unwrap();
        assert!(compressed_size > 0);

        // .gz exists, .db/.wal/.shm are gone.
        assert!(session_dir.join("session.db.gz").exists());
        assert!(!session_dir.join("session.db").exists());
        assert!(!session_dir.join("session.db-wal").exists());
        assert!(!session_dir.join("session.db-shm").exists());

        // Decompress and verify data integrity.
        let gz_data = std::fs::read(session_dir.join("session.db.gz")).unwrap();
        let mut decoder = flate2::read::GzDecoder::new(&gz_data[..]);
        let mut decompressed = Vec::new();
        std::io::Read::read_to_end(&mut decoder, &mut decompressed).unwrap();

        let temp_db = session_dir.join("verify.db");
        std::fs::write(&temp_db, &decompressed).unwrap();
        let conn = Connection::open(&temp_db).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM test", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 100);
    }

    #[test]
    fn vacuum_and_compress_nonexistent_db_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().join("nonexistent");
        std::fs::create_dir_all(&session_dir).unwrap();
        let result = vacuum_and_compress_session_db(&session_dir);
        assert!(result.is_err());
    }

    #[test]
    fn vacuum_and_compress_double_call_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().join("20260225-143052-a7f3");
        std::fs::create_dir_all(&session_dir).unwrap();

        let db_path = session_dir.join("session.db");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch("CREATE TABLE test (id INTEGER)").unwrap();
        }

        // First call succeeds.
        vacuum_and_compress_session_db(&session_dir).unwrap();
        assert!(session_dir.join("session.db.gz").exists());

        // Second call fails (no .db file).
        let result = vacuum_and_compress_session_db(&session_dir);
        assert!(result.is_err());
    }

    #[test]
    fn stopped_sessions_includes_vacuumed() {
        let idx = SessionIndex::open_in_memory().unwrap();

        let mut s1 = sample_record("20260225-100000-0000", "stopped");
        s1.created_at = "2026-02-25T10:00:00Z".to_string();
        idx.create_session(&s1).unwrap();

        let mut s2 = sample_record("20260225-110000-0000", "vacuumed");
        s2.created_at = "2026-02-25T11:00:00Z".to_string();
        idx.create_session(&s2).unwrap();

        let mut s3 = sample_record("20260225-120000-0000", "terminated");
        s3.created_at = "2026-02-25T12:00:00Z".to_string();
        idx.create_session(&s3).unwrap();

        let stopped = idx.stopped_sessions_oldest_first().unwrap();
        assert_eq!(stopped.len(), 2); // stopped + vacuumed, not terminated
        assert_eq!(stopped[0].id, "20260225-100000-0000");
        assert_eq!(stopped[1].id, "20260225-110000-0000");
    }

    // -- query_raw --

    #[test]
    fn query_raw_returns_columnar_json() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running")).unwrap();

        let json_str = idx.query_raw("SELECT id, mode, status FROM sessions", &[]).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["columns"], serde_json::json!(["id", "mode", "status"]));
        assert_eq!(parsed["rows"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["rows"][0][0], "20260225-143052-a7f3");
    }

    #[test]
    fn query_raw_with_bind_params() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running")).unwrap();
        let mut r2 = sample_record("20260225-143053-b8e4", "stopped");
        r2.created_at = "2026-02-25T14:30:53Z".to_string();
        idx.create_session(&r2).unwrap();

        let params = vec![serde_json::json!("stopped")];
        let json_str = idx.query_raw("SELECT id FROM sessions WHERE status = ?", &params).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["rows"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["rows"][0][0], "20260225-143053-b8e4");
    }

    #[test]
    fn query_raw_empty_result() {
        let idx = SessionIndex::open_in_memory().unwrap();
        let json_str = idx.query_raw("SELECT id FROM sessions", &[]).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["rows"].as_array().unwrap().len(), 0);
        assert_eq!(parsed["columns"], serde_json::json!(["id"]));
    }

    #[test]
    fn query_raw_with_limit_param() {
        let idx = SessionIndex::open_in_memory().unwrap();
        for i in 0..5 {
            let mut rec = sample_record(&format!("20260225-{i:06}-0000"), "running");
            rec.created_at = format!("2026-02-25T{i:02}:00:00Z");
            idx.create_session(&rec).unwrap();
        }

        let params = vec![serde_json::json!(2)];
        let json_str = idx.query_raw("SELECT id FROM sessions LIMIT ?", &params).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["rows"].as_array().unwrap().len(), 2);
    }

    // -- query_raw read-only enforcement (PRAGMA query_only) --

    #[test]
    fn query_raw_rejects_insert() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running")).unwrap();

        let result = idx.query_raw(
            "INSERT INTO sessions (id, mode, status, created_at) VALUES ('evil', 'gui', 'running', '2026-01-01T00:00:00Z')",
            &[],
        );
        assert!(result.is_err(), "INSERT must be rejected by PRAGMA query_only");
    }

    #[test]
    fn query_raw_rejects_semicolon_injection() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running")).unwrap();

        // Multi-statement: first is SELECT (passes validate_select_only),
        // second is DROP TABLE (must be caught by PRAGMA query_only).
        let _result = idx.query_raw("SELECT 1; DROP TABLE sessions", &[]);
        // Either the prepare or execute step should reject this.
        // The SELECT may succeed but DROP must not execute.
        // Verify sessions table is intact regardless.
        let count = idx.count().unwrap();
        assert_eq!(count, 1, "sessions table must not be dropped");
    }

    #[test]
    fn query_raw_select_works() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running")).unwrap();

        let result = idx.query_raw("SELECT COUNT(*) FROM sessions", &[]);
        assert!(result.is_ok(), "SELECT must succeed: {:?}", result);
        let parsed: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(parsed["rows"][0][0], 1);
    }

    #[test]
    fn query_raw_other_methods_still_write() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running")).unwrap();

        // Call query_raw (sets PRAGMA query_only ON then OFF).
        let _ = idx.query_raw("SELECT 1", &[]);

        // Internal write methods must still work after query_raw restored
        // the connection to read-write mode.
        idx.update_status("20260225-143052-a7f3", "stopped", Some("2026-02-25T15:00:00Z"))
            .unwrap();
        let records = idx.recent(1).unwrap();
        assert_eq!(records[0].status, "stopped");
    }

    #[test]
    fn query_raw_restores_write_on_error() {
        let idx = SessionIndex::open_in_memory().unwrap();
        idx.create_session(&sample_record("20260225-143052-a7f3", "running")).unwrap();

        // Trigger an error inside query_raw (bad SQL).
        let _ = idx.query_raw("INSERT INTO sessions VALUES ('x')", &[]);

        // PRAGMA query_only must be restored to OFF despite the error.
        idx.create_session(&sample_record("20260225-143053-b8e4", "running"))
            .unwrap();
        assert_eq!(idx.count().unwrap(), 2);
    }
}
