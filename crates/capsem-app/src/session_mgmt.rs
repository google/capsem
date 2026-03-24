use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use capsem_core::net::policy_config;
use capsem_core::session::{self, SessionIndex};
use capsem_core::VmState;
use capsem_logger::DbWriter;
use tracing::{debug, info, warn};

/// Get the sessions base directory: ~/.capsem/sessions/
pub(crate) fn sessions_dir() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(|h| {
        PathBuf::from(h).join(".capsem").join("sessions")
    })
}

/// Get the session directory for a specific VM: ~/.capsem/sessions/<vm_id>/
pub(crate) fn session_dir_for(vm_id: &str) -> Option<PathBuf> {
    sessions_dir().map(|d| d.join(vm_id))
}

/// Clean up stale sessions on app startup using SessionIndex.
///
/// Deletes any leftover scratch.img files (always ephemeral) and marks
/// any "running" sessions as "crashed" (stale from ungraceful exit).
/// Also runs vacuum recovery, age/count/disk-based culling, and terminated purging.
pub(crate) fn cleanup_stale_sessions(index: &SessionIndex) {
    let base = match sessions_dir() {
        Some(d) => d,
        None => return,
    };

    // Delete leftover scratch.img files from all session dirs.
    if let Ok(entries) = std::fs::read_dir(&base) {
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let scratch = dir.join("scratch.img");
            if scratch.exists() {
                info!(path = %scratch.display(), "deleting stale scratch.img");
                let _ = std::fs::remove_file(&scratch);
            }
        }
    }

    // Mark stale "running" sessions as "crashed" in main.db.
    match index.mark_running_as_crashed() {
        Ok(0) => {}
        Ok(n) => info!(count = n, "marked stale sessions as crashed"),
        Err(e) => warn!("failed to mark stale sessions: {e}"),
    }

    // Backfill: for crashed sessions with zero stats but a session.db on disk,
    // retroactively populate the summary tables.
    if let Ok(sessions) = index.recent(1000) {
        for rec in &sessions {
            if rec.status != "crashed" && rec.status != "stopped" {
                continue;
            }
            // Skip sessions that already have data.
            if rec.total_input_tokens > 0 || rec.total_tool_calls > 0 {
                continue;
            }
            let db_path = base.join(&rec.id).join("session.db");
            if !db_path.exists() {
                continue;
            }
            if let Ok(reader) = capsem_logger::DbReader::open(&db_path) {
                flush_session_summary(&rec.id, index, &reader);
                // Also backfill request counts if zero.
                if rec.total_requests == 0 {
                    if let Ok(counts) = reader.net_event_counts() {
                        let _ = index.update_request_counts(
                            &rec.id,
                            counts.total as u64,
                            counts.allowed as u64,
                            counts.denied as u64,
                        );
                    }
                }
                info!(id = %rec.id, "backfilled session summary");
            }
        }
    }

    // Vacuum recovery: compress any stopped/crashed sessions not yet vacuumed.
    if let Ok(unvacuumed) = index.unvacuumed_sessions() {
        for rec in &unvacuumed {
            let session_dir = base.join(&rec.id);
            let has_db = session_dir.join("session.db").exists();
            let has_gz = session_dir.join("session.db.gz").exists();
            if !has_db && !has_gz {
                // Session never created a DB (crashed early). Mark vacuumed so we stop retrying.
                debug!(id = %rec.id, "skipping vacuum for session with no DB");
                let _ = index.mark_vacuumed(&rec.id, 0, &session::now_iso());
                continue;
            }
            vacuum_session(&rec.id, index, &session_dir);
        }
    }

    // Age-based culling (terminate, not delete).
    let settings = policy_config::load_merged_settings();
    let retention_days = settings.iter()
        .find(|s| s.id == "vm.resources.retention_days")
        .and_then(|s| s.effective_value.as_number())
        .unwrap_or(30) as u32;
    let max_sessions = settings.iter()
        .find(|s| s.id == "vm.resources.max_sessions")
        .and_then(|s| s.effective_value.as_number())
        .unwrap_or(100) as usize;
    let max_disk_gb = settings.iter()
        .find(|s| s.id == "vm.resources.max_disk_gb")
        .and_then(|s| s.effective_value.as_number())
        .unwrap_or(100) as u64;
    let terminated_retention_days = settings.iter()
        .find(|s| s.id == "vm.resources.terminated_retention_days")
        .and_then(|s| s.effective_value.as_number())
        .unwrap_or(365) as u32;

    if let Ok(n) = index.terminate_older_than_days(retention_days) {
        if n > 0 {
            info!(count = n, "terminated old sessions (>{retention_days} days)");
        }
    }
    if let Ok(n) = index.terminate_excess_sessions(max_sessions) {
        if n > 0 {
            info!(count = n, "terminated sessions over cap ({max_sessions})");
        }
    }

    // Disk-based culling.
    let max_disk_bytes = max_disk_gb * 1024 * 1024 * 1024;
    let mut usage = session::disk_usage_bytes(&base);
    if usage > max_disk_bytes {
        if let Ok(stopped) = index.stopped_sessions_oldest_first() {
            for rec in stopped {
                if usage <= max_disk_bytes {
                    break;
                }
                let dir = base.join(&rec.id);
                if dir.is_dir() {
                    let dir_bytes = session::disk_usage_bytes(&dir);
                    if let Err(e) = std::fs::remove_dir_all(&dir) {
                        warn!(id = %rec.id, "failed to remove session dir: {e}");
                        continue;
                    }
                    usage = usage.saturating_sub(dir_bytes);
                    let _ = index.mark_terminated(&rec.id);
                    info!(id = %rec.id, "culled session dir for disk budget");
                }
            }
        }
    }

    // Delete disk artifacts for terminated sessions that still have directories.
    if let Ok(terminated) = index.sessions_by_status("terminated") {
        for rec in &terminated {
            let dir = base.join(&rec.id);
            if dir.is_dir() {
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
    }

    // Purge old terminated records from main.db.
    if let Ok(n) = index.purge_terminated_older_than_days(terminated_retention_days) {
        if n > 0 {
            info!(count = n, "purged terminated records (>{terminated_retention_days} days)");
        }
    }

    // Remove orphan session dirs that no longer have a DB record.
    if let Ok(entries) = std::fs::read_dir(&base) {
        let known_ids: std::collections::HashSet<String> = index
            .recent(10_000)
            .unwrap_or_default()
            .into_iter()
            .map(|r| r.id)
            .collect();
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let name = match dir.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if !session::is_valid_session_id(&name) {
                continue;
            }
            if !known_ids.contains(&name) {
                if let Err(e) = std::fs::remove_dir_all(&dir) {
                    warn!(id = %name, "failed to remove orphan session dir: {e}");
                } else {
                    info!(id = %name, "removed orphan session dir");
                }
            }
        }
    }

    // Checkpoint main.db after all cleanup.
    let _ = index.checkpoint();
}

/// Vacuum and compress a session DB, updating the index on success.
pub(crate) fn vacuum_session(session_id: &str, index: &SessionIndex, session_dir: &std::path::Path) {
    match session::vacuum_and_compress_session_db(session_dir) {
        Ok(compressed_size) => {
            let _ = index.mark_vacuumed(session_id, compressed_size, &session::now_iso());
            info!(id = %session_id, compressed_size, "vacuumed session DB");
        }
        Err(e) => {
            warn!(id = %session_id, "failed to vacuum session DB: {e:#}");
        }
    }
}

/// Clean up a VM session: delete scratch.img, snapshot request counts, update status.
pub(crate) fn cleanup_session(
    _session_dir: &Path,
    scratch_path: Option<&Path>,
    session_id: &str,
    index: &SessionIndex,
    db: Option<&DbWriter>,
) {
    if let Some(scratch) = scratch_path {
        if scratch.exists() {
            info!(path = %scratch.display(), "deleting scratch.img");
            if let Err(e) = std::fs::remove_file(scratch) {
                warn!("failed to delete scratch.img: {e}");
            }
        }
    }

    // Snapshot request counts + summary data.
    if let Some(writer) = db {
        if let Ok(reader) = writer.reader() {
            if let Ok(counts) = reader.net_event_counts() {
                let _ = index.update_request_counts(
                    session_id,
                    counts.total as u64,
                    counts.allowed as u64,
                    counts.denied as u64,
                );
            }
            flush_session_summary(session_id, index, &reader);
        }
    }

    let _ = index.update_status(session_id, VmState::Stopped.as_str(), Some(&session::now_iso()));
}

/// Flush per-session summary data from info.db into main.db.
pub(crate) fn flush_session_summary(
    session_id: &str,
    index: &SessionIndex,
    reader: &capsem_logger::DbReader,
) {
    use capsem_core::session::{McpToolSummary, ProviderSummary, ToolSummary};

    // Session-level summary.
    if let Ok(stats) = reader.session_stats() {
        let file_events = reader.file_event_count().unwrap_or(0);
        let mcp_calls = reader.mcp_call_stats().map(|s| s.total).unwrap_or(0);
        let _ = index.update_session_summary(
            session_id,
            stats.total_input_tokens,
            stats.total_output_tokens,
            stats.total_estimated_cost_usd,
            stats.total_tool_calls,
            mcp_calls,
            file_events,
        );
    }

    // Provider usage.
    if let Ok(providers) = reader.token_usage_by_provider() {
        let summaries: Vec<ProviderSummary> = providers
            .into_iter()
            .map(|p| ProviderSummary {
                provider: p.provider,
                call_count: p.call_count,
                input_tokens: p.total_input_tokens,
                output_tokens: p.total_output_tokens,
                estimated_cost: p.total_estimated_cost_usd,
                total_duration_ms: p.total_duration_ms,
            })
            .collect();
        let _ = index.replace_ai_usage(session_id, &summaries);
    }

    // Tool usage.
    if let Ok(tools) = reader.tool_usage_with_stats(50) {
        let summaries: Vec<ToolSummary> = tools
            .into_iter()
            .map(|t| ToolSummary {
                tool_name: t.tool_name,
                call_count: t.count,
                total_bytes: t.total_bytes,
                total_duration_ms: t.total_duration_ms,
            })
            .collect();
        let _ = index.replace_tool_usage(session_id, &summaries);
    }

    // MCP tool usage.
    if let Ok(mcp_tools) = reader.mcp_tool_usage(50) {
        let summaries: Vec<McpToolSummary> = mcp_tools
            .into_iter()
            .map(|m| McpToolSummary {
                tool_name: m.tool_name,
                server_name: m.server_name,
                call_count: m.count,
                total_bytes: m.total_bytes,
                total_duration_ms: m.total_duration_ms,
            })
            .collect();
        let _ = index.replace_mcp_usage(session_id, &summaries);
    }
}

/// Open the session database independently of MITM proxy state.
///
/// The session DB is needed by multiple subsystems (file monitor, MCP gateway,
/// telemetry) and must not be coupled to CA/policy loading. If this fails,
/// the session cannot proceed at all.
pub(crate) fn open_session_db(vm_id: &str) -> Result<Arc<DbWriter>> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let session_dir = PathBuf::from(home)
        .join(".capsem")
        .join("sessions")
        .join(vm_id);
    let db_path = session_dir.join("session.db");
    let db = DbWriter::open(&db_path, 4096).context("failed to open session db")?;
    info!(path = %db_path.display(), "opened session db");
    Ok(Arc::new(db))
}
