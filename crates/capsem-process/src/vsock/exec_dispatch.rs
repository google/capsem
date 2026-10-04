//! One structured exec from the service to the guest agent: its ledger row
//! and security decision first, and only a command that may run reaches the
//! guest.

use super::{exec_boundary_refusal, JobResult, JobStore, PluginPolicyHandle, SecurityRulesHandle};
use capsem_proto::HostToGuest;
use std::sync::Arc;
use tokio::sync::mpsc;

/// What the control bridge holds for every exec it dispatches.
pub(super) struct ExecDispatch {
    pub(super) vm_id: String,
    pub(super) db: Arc<capsem_logger::DbWriter>,
    pub(super) rules: SecurityRulesHandle,
    pub(super) plugins: PluginPolicyHandle,
    pub(super) jobs: Arc<JobStore>,
    pub(super) hub: mpsc::Sender<HostToGuest>,
}

impl ExecDispatch {
    pub(super) async fn dispatch(&self, id: u64, command: String) {
        // active_execs is owned by ipc.rs's Exec handler -- it creates the
        // capture slot *before* sending here. The control bridge owns
        // delivery/replay, so this layer just forwards without replacing the
        // per-id capture slot.
        let trace_id = capsem_foundation::telemetry::ambient_capsem_trace_id().or_else(|| {
            capsem_foundation::telemetry::child_trace_env(&format!("{}-exec-{id}", self.vm_id))
                .into_iter()
                .find_map(|(key, value)| (key == "CAPSEM_TRACE_ID").then_some(value))
        });
        let rules = self.rules.read().unwrap().clone();
        let plugins = self.plugins.read().unwrap().clone();
        let boundary = capsem_core::security_engine::emit_process_exec_security_boundary(
            &self.db,
            &rules,
            plugins,
            capsem_logger::ExecEvent {
                event_id: None,
                timestamp: std::time::SystemTime::now(),
                exec_id: id,
                command: command.clone(),
                source: "api".into(),
                target: capsem_proto::ipc::ExecTarget::Vm,
                trace_id,
                process_name: None,
                credential_ref: None,
            },
        )
        .await;
        // The command has not reached the guest yet, so a non-allow decision
        // is enforceable here on the same terms as the network and file
        // boundaries: withhold the dispatch and fail the caller's job.
        if let Some(refusal) = exec_boundary_refusal(id, &boundary) {
            self.jobs.active_execs.lock().unwrap().remove(&id);
            if let Some(tx) = self.jobs.jobs.lock().unwrap().remove(&id) {
                capsem_core::try_send!(
                    "job_result_exec_blocked",
                    tx.send(JobResult::Error { message: refusal })
                );
            }
            return;
        }
        if let Ok(Some(emission)) = &boundary {
            if let Some(active) = self.jobs.active_execs.lock().unwrap().get_mut(&id) {
                active.event_id = Some(emission.event_id.clone());
            }
        }
        capsem_core::try_send!("hub_exec", self.hub.send(HostToGuest::Exec { id, command }).await);
    }
}

#[cfg(test)]
mod tests;
