//! Trusted live continuations and bounded ledger barriers, never a ledger cache.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use capsem_core::security_engine::approval::{ApprovalBinding, ApprovalBroker, ApprovalError, PendingApproval};
use capsem_logger::{DbWriter, SecurityAskEvent, SecurityAskRecord, SecurityAskStatus, WriteOp};
use capsem_proto::approval::{ApprovalOutcome, ApprovalRequest};

use crate::instance::WorkerGrant;
use crate::session_db_handles::SessionLedger;

pub(crate) mod capability;

#[derive(Default)]
pub(crate) struct Runtime {
    pub(crate) broker: ApprovalBroker,
    sessions: Mutex<HashMap<ApprovalBinding, Arc<Session>>>,
    initialization: tokio::sync::Mutex<()>,
}

pub(crate) struct Session {
    pub(crate) binding: ApprovalBinding,
    broker: ApprovalBroker,
    writer: Arc<DbWriter>,
    reader: Arc<SessionLedger>,
    authority: WorkerGrant,
}

impl Runtime {
    pub(crate) fn install(
        &self,
        binding: ApprovalBinding,
        writer: Arc<DbWriter>,
        reader: Arc<SessionLedger>,
        authority: WorkerGrant,
    ) -> Result<Arc<Session>, String> {
        if authority.is_revoked() {
            return Err("approval generation retired".into());
        }
        let mut sessions = self.sessions.lock().unwrap();
        if sessions.contains_key(&binding) {
            return Err("approval generation already connected".into());
        }
        let session = Arc::new(Session {
            binding: binding.clone(),
            broker: self.broker.clone(),
            writer,
            reader,
            authority,
        });
        sessions.insert(binding, Arc::clone(&session));
        drop(sessions);
        Ok(session)
    }

    pub(crate) fn get(&self, binding: &ApprovalBinding) -> Option<Arc<Session>> {
        self.sessions
            .lock()
            .unwrap()
            .get(binding)
            .filter(|session| !session.authority.is_revoked())
            .cloned()
    }

    pub(crate) fn retire(&self, binding: &ApprovalBinding) {
        self.broker.cancel_generation(binding);
        // The reaper retains this producer until all capability actors have
        // joined their terminal audits. No destructor joins under this mutex.
    }

    pub(crate) async fn finish(&self, binding: &ApprovalBinding) -> Result<(), String> {
        let session = { self.sessions.lock().unwrap().remove(binding) };
        if let Some(session) = session {
            tokio::task::spawn_blocking(move || session.writer.shutdown_blocking())
                .await
                .map_err(|error| format!("approval producer shutdown: {error}"))?;
        }
        Ok(())
    }
}

impl Session {
    pub(crate) async fn record(&self, ask_id: &str) -> Result<Option<SecurityAskRecord>, String> {
        let sets = self
            .reader
            .query(capsem_logger::ledger_protocol::LedgerQuery::SecurityAsks {
                limit: 1,
                ask_id: Some(ask_id.into()),
            })
            .await?;
        let rows = sets.into_iter().next().ok_or("missing approval query result")?;
        crate::ledger_routes::rows::ledger_rows_to_objects(rows)
            .into_iter()
            .next()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| format!("invalid approval ledger row: {error}"))
    }

    #[cfg(test)]
    pub(crate) async fn begin(&self, request: ApprovalRequest) -> Result<PendingApproval, String> {
        let pending = self.reserve(&request)?;
        self.verify(&request).await?;
        if !pending.publish() {
            return Err("approval ended before verification".into());
        }
        Ok(pending)
    }

    fn reserve(&self, request: &ApprovalRequest) -> Result<PendingApproval, String> {
        if self.authority.is_revoked() {
            return Err("approval generation retired".into());
        }
        // Reserve before any asynchronous work: overload cannot create an
        // unbounded set of ledger queries or retained request bodies.
        self.broker
            .reserve(self.binding.clone(), request.clone())
            .map_err(|error| format!("approval admission: {error:?}"))
    }

    async fn verify(&self, request: &ApprovalRequest) -> Result<(), String> {
        let record = self.record(&request.ask_id).await?.ok_or("missing pending approval")?;
        if record.status != SecurityAskStatus::Pending
            || record.event_id != request.event_id
            || record.event_type != request.event_type
            || record.rule_id != request.rule_id
            || record.rule_name != request.rule_name
            || self.authority.is_revoked()
        {
            return Err("approval does not match original pending operation".into());
        }
        Ok(())
    }

    /// The capability actor owns this future even if its caller disconnects.
    /// Only bounded audit I/O takes the per-request mutex.
    pub(crate) async fn wait(&self, ask_id: &str, pending: PendingApproval) -> ApprovalOutcome {
        let audit = Arc::clone(&pending.audit);
        let outcome = tokio::select! {
            biased;
            _ = self.authority.revoked() => ApprovalOutcome::Cancelled,
            outcome = pending.wait() => outcome,
        };
        if !matches!(outcome, ApprovalOutcome::Approved | ApprovalOutcome::Denied) {
            let _audit = audit.lock().await;
            let reason = match outcome {
                ApprovalOutcome::Expired => "deadline",
                ApprovalOutcome::Cancelled => "request_cancelled",
                _ => "unavailable",
            };
            if self
                .append_resolution(ask_id, SecurityAskStatus::Denied, reason)
                .await
                .is_err()
            {
                return ApprovalOutcome::Unavailable;
            }
        }
        outcome
    }

    pub(crate) async fn resolve(&self, ask_id: &str, approve: bool) -> Result<(), ResolveError> {
        if self.authority.is_revoked() {
            return Err(ResolveError::Conflict);
        }
        let ticket = self.broker.claim(&self.binding, ask_id).map_err(|error| match error {
            ApprovalError::Missing => ResolveError::Missing,
            _ => ResolveError::Conflict,
        })?;
        let audit = Arc::clone(&ticket.audit);
        let _audit = audit.lock().await;
        let outcome = if approve {
            ApprovalOutcome::Approved
        } else {
            ApprovalOutcome::Denied
        };
        let status = if approve {
            SecurityAskStatus::Approved
        } else {
            SecurityAskStatus::Denied
        };
        self.append_resolution(ask_id, status, "human_decision")
            .await
            .map_err(|_| ResolveError::Unavailable)?;
        if self.authority.is_revoked() || !ticket.complete_after_audit(outcome) {
            return Err(ResolveError::Conflict);
        }
        Ok(())
    }

    async fn append_resolution(&self, ask_id: &str, status: SecurityAskStatus, reason: &str) -> Result<(), String> {
        let record = self.record(ask_id).await?.ok_or("missing approval audit row")?;
        let event = SecurityAskEvent {
            timestamp_unix_ms: now_ms(),
            ask_id: record.ask_id,
            event_id: record.event_id,
            event_type: record.event_type,
            rule_id: record.rule_id,
            rule_name: record.rule_name,
            status,
            rule_json: record.rule_json,
            event_json: "{}".into(),
            resolver: Some("authenticated_client".into()),
            reason: Some(reason.into()),
            trace_id: record.trace_id,
        };
        self.writer.write_checked(WriteOp::SecurityAskEvent(event)).await?;
        self.writer.flush_checked().await
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ResolveError {
    Missing,
    Conflict,
    Unavailable,
}

fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

impl crate::ServiceState {
    pub(crate) fn pending_approval_count(&self, id: &str) -> u32 {
        let binding = self.instances.lock().unwrap().get(id).map(|instance| ApprovalBinding {
            session_id: id.into(),
            generation: instance.generation,
        });
        binding.map_or(0, |binding| self.approvals.broker.pending_count(&binding) as u32)
    }
}

#[cfg(test)]
mod tests;
