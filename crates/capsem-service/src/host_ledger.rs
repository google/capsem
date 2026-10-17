//! The host forensic ledger: what the service did to every session, and the
//! `/stats` totals folded from it.
//!
//! Each session's ledger records what happened inside its VM; this one, in
//! the home's `sessions/host.db`, records what happened to it -- created,
//! stopped and how -- as hash-chained events. A stopped event carries the
//! session's final counter snapshot, so `/stats` is a fold held in memory:
//! finished snapshots replayed from this ledger at startup, plus every
//! running session's live counters. The old `main.db` is never opened.

use super::*;
use capsem_logger::{HostEvent, HostEventKind, WriteOp};
use capsem_proto::host_session::HostSessionDetail;
use capsem_proto::ledger_counters::LedgerCounters;
use std::collections::VecDeque;

/// Finished sessions `/stats` lists, newest first; totals cover every one.
const LISTED_FINISHED_SESSIONS: usize = 100;
/// Entries in each `/stats` top list.
const TOP_ENTRIES: usize = 20;

pub(super) fn host_ledger_path_in(sessions_dir: &StdPath) -> PathBuf {
    sessions_dir.join("host.db")
}

pub(super) fn open_host_ledger(sessions_dir: &StdPath) -> anyhow::Result<Arc<capsem_logger::DbHandle>> {
    let db_path = host_ledger_path_in(sessions_dir);
    let handle = capsem_logger::DbHandle::open(&db_path)
        .with_context(|| format!("failed to open the host ledger: {}", db_path.display()))?;
    Ok(Arc::new(handle))
}

#[derive(Debug, Clone)]
struct SessionSummary {
    id: String,
    created_at_ms: i64,
    stopped_at_ms: Option<i64>,
    detail: HostSessionDetail,
}

/// The `/stats` fold: every finished session's counters, and the sessions it
/// lists. Rebuilt from the host ledger at startup, advanced by each event.
#[derive(Debug, Default)]
pub(crate) struct HostStats {
    total_sessions: u64,
    finished: LedgerCounters,
    running: BTreeMap<String, SessionSummary>,
    recent_finished: VecDeque<SessionSummary>,
}

impl HostStats {
    pub(crate) fn apply(&mut self, event: &HostEvent) {
        let Some(id) = event.session_id.clone() else {
            return;
        };
        let detail = HostSessionDetail::decode(&event.detail).unwrap_or_else(|error| {
            warn!(id, %error, "host ledger session detail does not decode");
            HostSessionDetail::default()
        });
        match event.kind {
            HostEventKind::SessionCreated => {
                self.total_sessions += 1;
                let summary = SessionSummary {
                    id: id.clone(),
                    created_at_ms: event.timestamp_unix_ms,
                    stopped_at_ms: None,
                    detail,
                };
                self.running.insert(id, summary);
            }
            HostEventKind::SessionStopped => {
                let mut summary = self.running.remove(&id).unwrap_or_else(|| SessionSummary {
                    id: id.clone(),
                    created_at_ms: event.timestamp_unix_ms,
                    stopped_at_ms: None,
                    detail: HostSessionDetail::default(),
                });
                summary.stopped_at_ms = Some(event.timestamp_unix_ms);
                summary.detail.status = detail.status;
                summary.detail.counters = detail.counters;
                if let Some(counters) = &summary.detail.counters {
                    self.finished.absorb(counters);
                }
                self.recent_finished.push_front(summary);
                self.recent_finished.truncate(LISTED_FINISHED_SESSIONS);
            }
            _ => {}
        }
    }

    pub(crate) fn running_ids(&self) -> Vec<String> {
        self.running.keys().cloned().collect()
    }
}

impl ServiceState {
    /// Replay the host ledger into the `/stats` fold. A session the ledger
    /// saw start but never stop belongs to a service that died with it.
    pub(crate) async fn hydrate_host_stats(&self) -> anyhow::Result<()> {
        let events = self
            .host_ledger
            .host_events()
            .await
            .map_err(|error| anyhow!("replay host ledger: {error}"))?;
        let mut stats = HostStats::default();
        for event in &events {
            stats.apply(event);
        }
        let now = unix_millis();
        for id in stats.running_ids() {
            stats.apply(&host_session_event(
                HostEventKind::SessionStopped,
                &id,
                now,
                &stopped_detail("lost", None),
            ));
        }
        *self.host_stats.lock().unwrap() = stats;
        Ok(())
    }

    /// Record an event and put it on disk before returning: host events are
    /// rare, and a forensic record that waits for the next periodic flush can
    /// lose a session's stop to a crash.
    async fn record_host_event(&self, event: HostEvent) -> anyhow::Result<()> {
        self.host_stats.lock().unwrap().apply(&event);
        let kind = event.kind.as_str();
        self.host_ledger
            .write(WriteOp::HostEvent(event))
            .await
            .map_err(|error| anyhow!("record {kind} in the host ledger: {error}"))?;
        self.host_ledger
            .flush()
            .await
            .map_err(|error| anyhow!("flush {kind} to the host ledger: {error}"))
    }

    pub(crate) async fn record_service_event(&self, kind: HostEventKind) -> anyhow::Result<()> {
        self.record_host_event(HostEvent {
            timestamp_unix_ms: unix_millis(),
            kind,
            session_id: None,
            actor: "service".to_string(),
            detail: Vec::new(),
            trace_id: None,
        })
        .await
    }

    pub(crate) async fn record_host_session_created(&self, id: &str, detail: HostSessionDetail) -> anyhow::Result<()> {
        self.record_host_event(host_session_event(
            HostEventKind::SessionCreated,
            id,
            unix_millis(),
            &detail,
        ))
        .await
    }

    /// Record a session's end, with the counters its own ledger holds when
    /// `read_counters`: a discarded session is a deletion target, and reading
    /// a ledger torn by SIGKILL must not turn its teardown into a failure.
    pub(crate) async fn record_host_session_stopped(
        &self,
        id: &str,
        status: &str,
        read_counters: bool,
    ) -> anyhow::Result<()> {
        let counters = match self.session_db_handle(id).filter(|_| read_counters) {
            Some(db) => db.ledger_counters().await.ok().map(|counters| (*counters).clone()),
            None => None,
        };
        let detail = stopped_detail(status, counters);
        self.record_host_event(host_session_event(
            HostEventKind::SessionStopped,
            id,
            unix_millis(),
            &detail,
        ))
        .await
    }

    /// `GET /stats`: the fold plus every running session's live counters.
    pub(crate) async fn stats_response(&self) -> Vec<u8> {
        let (total_sessions, mut totals, running, finished) = {
            let stats = self.host_stats.lock().unwrap();
            (
                stats.total_sessions,
                stats.finished.clone(),
                stats.running.values().cloned().collect::<Vec<_>>(),
                stats.recent_finished.iter().cloned().collect::<Vec<_>>(),
            )
        };
        let mut listed = Vec::with_capacity(running.len() + finished.len());
        for mut summary in running {
            if let Some(db) = self.session_db_handle(&summary.id) {
                if let Ok(counters) = db.ledger_counters().await {
                    totals.absorb(&counters);
                    summary.detail.counters = Some((*counters).clone());
                }
            }
            listed.push(summary);
        }
        listed.extend(finished);
        listed.sort_by(|left, right| right.created_at_ms.cmp(&left.created_at_ms));
        listed.truncate(LISTED_FINISHED_SESSIONS);
        render_stats(total_sessions, &totals, &listed)
    }
}

fn unix_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

pub(crate) fn stopped_detail(status: &str, counters: Option<LedgerCounters>) -> HostSessionDetail {
    HostSessionDetail {
        status: Some(status.to_string()),
        counters,
        ..HostSessionDetail::default()
    }
}

pub(crate) fn host_session_event(kind: HostEventKind, id: &str, at: i64, detail: &HostSessionDetail) -> HostEvent {
    HostEvent {
        timestamp_unix_ms: at,
        kind,
        session_id: Some(id.to_string()),
        actor: "service".to_string(),
        detail: detail.encode().unwrap_or_else(|error| {
            warn!(id, %error, "host session detail does not encode; recording the event without it");
            Vec::new()
        }),
        trace_id: None,
    }
}

fn iso(ms: i64) -> String {
    capsem_core::session::epoch_to_iso(u64::try_from(ms / 1000).unwrap_or_default())
}

fn usd(micro: u64) -> f64 {
    capsem_logger::counters::usd_from_micro(micro)
}

fn session_json(summary: &SessionSummary) -> serde_json::Value {
    let counters = summary.detail.counters.clone().unwrap_or_default();
    let overflow = counters.files.by_action.get("overflow").copied().unwrap_or_default();
    let status = match (&summary.detail.status, summary.stopped_at_ms) {
        (Some(status), _) => status.clone(),
        (None, None) => "running".to_string(),
        (None, Some(_)) => "stopped".to_string(),
    };
    json!({
        "id": summary.id,
        "mode": if summary.detail.persistent { "persistent" } else { "ephemeral" },
        "command": null,
        "status": status,
        "created_at": iso(summary.created_at_ms),
        "stopped_at": summary.stopped_at_ms.map(iso),
        "scratch_disk_size_gb": summary.detail.scratch_disk_size_gb,
        "ram_bytes": summary.detail.ram_bytes,
        "total_requests": counters.net.total,
        "allowed_requests": counters.net.allowed,
        "denied_requests": counters.net.denied,
        "total_input_tokens": counters.model.total.input_tokens,
        "total_output_tokens": counters.model.total.output_tokens,
        "total_estimated_cost": usd(counters.model.total.cost_micro_usd),
        "total_tool_calls": counters.tools.calls,
        "total_file_events": counters.files.events.saturating_sub(overflow),
        "storage_mode": summary.detail.storage_mode,
        "rootfs_hash": summary.detail.rootfs_hash,
        "rootfs_version": summary.detail.rootfs_version,
        "forked_from": summary.detail.forked_from,
        "persistent": summary.detail.persistent,
        "exec_count": counters.exec.started,
        "audit_event_count": counters.audit.events,
    })
}

fn top<T>(mut entries: Vec<(u64, T)>) -> Vec<T> {
    entries.sort_by(|left, right| right.0.cmp(&left.0));
    entries.into_iter().take(TOP_ENTRIES).map(|(_, value)| value).collect()
}

fn render_stats(total_sessions: u64, totals: &LedgerCounters, listed: &[SessionSummary]) -> Vec<u8> {
    let overflow = totals.files.by_action.get("overflow").copied().unwrap_or_default();
    let providers = top(totals
        .model
        .by_model
        .iter()
        .map(|(provider, models)| {
            let calls: u64 = models.values().map(|usage| usage.calls).sum();
            let sum = |field: fn(&capsem_proto::ledger_counters::ModelUsage) -> u64| -> u64 {
                models.values().map(field).sum()
            };
            (
                calls,
                json!({
                    "provider": provider,
                    "call_count": calls,
                    "input_tokens": sum(|usage| usage.input_tokens),
                    "output_tokens": sum(|usage| usage.output_tokens),
                    "estimated_cost": usd(sum(|usage| usage.cost_micro_usd)),
                    "total_duration_ms": sum(|usage| usage.duration_ms),
                }),
            )
        })
        .collect());
    let tool = |name: &str, usage: &capsem_proto::ledger_counters::ToolUsage| {
        json!({
            "tool_name": name,
            "call_count": usage.calls,
            "total_bytes": usage.bytes_sent.saturating_add(usage.bytes_received),
            "total_duration_ms": usage.duration_ms,
        })
    };
    let tools = top(totals
        .tools
        .by_tool
        .iter()
        .map(|(name, usage)| (usage.calls, tool(name, usage)))
        .collect());
    let mcp_tools = top(totals
        .tools
        .mcp
        .iter()
        .flat_map(|(server, tools)| {
            tools.iter().map(move |(name, usage)| {
                let mut entry = tool(name, usage);
                entry["server_name"] = json!(server);
                (usage.calls, entry)
            })
        })
        .collect());
    let body = json!({
        "global": {
            "total_sessions": total_sessions,
            "total_input_tokens": totals.model.total.input_tokens,
            "total_output_tokens": totals.model.total.output_tokens,
            "total_estimated_cost": usd(totals.model.total.cost_micro_usd),
            "total_tool_calls": totals.tools.calls,
            "total_file_events": totals.files.events.saturating_sub(overflow),
            "total_requests": totals.net.total,
            "total_allowed": totals.net.allowed,
            "total_denied": totals.net.denied,
        },
        "sessions": listed.iter().map(session_json).collect::<Vec<_>>(),
        "top_providers": providers,
        "top_tools": tools,
        "top_mcp_tools": mcp_tools,
    });
    serde_json::to_vec(&body).unwrap_or_default()
}

#[cfg(test)]
mod tests;
