//! One structured exec from the service to the guest agent.
//!
//! The ledger row and the security decision are made on the caller's own
//! command and its target. Only a command that may run is then wrapped for
//! its target: a workload exec becomes the container launcher's `--exec`
//! (`capsem_core::container::workload_exec_command`), which `runc exec`s it
//! into the running workload as the image's user. Wrapping any earlier would
//! record, and judge, the launcher instead of what was asked for.

use super::{exec_boundary_refusal, JobResult, JobStore, PluginPolicyHandle, SecurityRulesHandle};
use capsem_proto::ipc::ExecTarget;
use capsem_proto::HostToGuest;
use std::sync::Arc;
use tokio::sync::mpsc;

/// What the control bridge holds for every exec it dispatches.
#[derive(Clone)]
pub(super) struct ExecDispatch {
    pub(super) vm_id: String,
    pub(super) db: Arc<capsem_logger::DbWriter>,
    pub(super) rules: SecurityRulesHandle,
    pub(super) plugins: PluginPolicyHandle,
    pub(super) jobs: Arc<JobStore>,
    pub(super) hub: mpsc::Sender<HostToGuest>,
    pub(super) approvals: Option<Arc<capsem_core::security_engine::approval::client::ApprovalCapability>>,
}

/// The command the guest agent runs for `command` in `target`.
pub(super) fn guest_command(command: String, target: ExecTarget) -> String {
    match target {
        ExecTarget::Vm => command,
        ExecTarget::Workload => capsem_core::container::workload_exec_command(&command),
    }
}

impl ExecDispatch {
    pub(super) fn spawn(&self, calls: &mut tokio::task::JoinSet<()>, id: u64, command: String, target: ExecTarget) {
        while calls.try_join_next().is_some() {}
        let Some(cancellation) = self.cancellation(id) else {
            return;
        };
        if calls.len() >= capsem_proto::APPROVAL_PENDING_PER_SESSION
            || command.len().saturating_mul(4) > capsem_proto::APPROVAL_HELD_BYTES_PER_REQUEST
        {
            self.refuse(id, &cancellation, "execution request capacity exceeded".into());
            return;
        }
        let dispatch = self.clone();
        calls.spawn(async move { dispatch.dispatch_original(id, command, target, cancellation).await });
    }

    fn cancellation(&self, id: u64) -> Option<Arc<tokio_util::sync::CancellationToken>> {
        self.jobs
            .active_execs
            .lock()
            .unwrap()
            .get(&id)
            .map(|active| Arc::clone(&active.cancellation))
    }

    #[cfg(test)]
    pub(super) async fn dispatch(&self, id: u64, command: String, target: ExecTarget) {
        let Some(cancellation) = self.cancellation(id) else {
            return;
        };
        self.dispatch_original(id, command, target, cancellation).await;
    }

    async fn dispatch_original(
        &self,
        id: u64,
        command: String,
        target: ExecTarget,
        cancellation: Arc<tokio_util::sync::CancellationToken>,
    ) {
        if cancellation.is_cancelled()
            || command.len().saturating_mul(4) > capsem_proto::APPROVAL_HELD_BYTES_PER_REQUEST
        {
            self.refuse(
                id,
                &cancellation,
                "execution canceled or retained command exceeds capacity".into(),
            );
            return;
        }
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
        let mut boundary = capsem_core::security_engine::emit_process_exec_security_boundary(
            &self.db,
            &rules,
            Arc::clone(&plugins),
            capsem_logger::ExecEvent {
                event_id: None,
                timestamp: std::time::SystemTime::now(),
                exec_id: id,
                command: command.clone(),
                source: "api".into(),
                target,
                trace_id,
                process_name: None,
                credential_ref: None,
            },
        )
        .await;
        if let Ok(Some(emission)) = &mut boundary {
            let resolution = capsem_core::security_engine::approval::resolve_emission(
                &self.db,
                self.approvals.as_deref(),
                emission,
                command.len().saturating_mul(4),
            );
            let result = tokio::select! {
                biased;
                _ = cancellation.cancelled() => Err("execution caller stopped while approval was pending".into()),
                result = resolution => result,
            };
            if let Err(error) = result {
                boundary = Err(error);
            } else if !Arc::ptr_eq(&rules, &self.rules.read().unwrap())
                || !Arc::ptr_eq(&plugins, &self.plugins.read().unwrap())
            {
                emission.enforcement.action = capsem_core::security_engine::SecurityEnforcementAction::Block;
                emission.event.decision.effective = capsem_core::security_engine::SecurityDecisionKind::Block;
                emission.enforcement.reason = Some("policy changed while execution was pending".into());
            }
        }
        // The command has not reached the guest yet, so a non-allow decision
        // is enforceable here on the same terms as the network and file
        // boundaries: withhold the dispatch and fail the caller's job.
        if let Some(refusal) = exec_boundary_refusal(id, &boundary) {
            self.refuse(id, &cancellation, refusal);
            return;
        }
        let command = guest_command(command, target);
        let permit = match tokio::select! {
            biased;
            _ = cancellation.cancelled() => None,
            permit = self.hub.reserve() => permit.ok(),
        } {
            Some(permit) => permit,
            None => {
                self.refuse(id, &cancellation, "execution caller or guest control stopped".into());
                return;
            }
        };
        // Sending into a reserved slot is synchronous. This short lock makes
        // cancellation versus original dispatch one decision, with no lock
        // held across human wait or channel backpressure.
        let sent = {
            let mut active = self.jobs.active_execs.lock().unwrap();
            if let Some(active) = active
                .get_mut(&id)
                .filter(|active| Arc::ptr_eq(&active.cancellation, &cancellation) && !cancellation.is_cancelled())
            {
                if let Ok(Some(emission)) = &boundary {
                    active.event_id = Some(emission.event_id.clone());
                }
                permit.send(HostToGuest::Exec { id, command });
                true
            } else {
                false
            }
        };
        if !sent {
            self.refuse(id, &cancellation, "execution canceled before guest delivery".into());
        }
    }

    fn refuse(&self, id: u64, cancellation: &Arc<tokio_util::sync::CancellationToken>, message: String) {
        let reply = {
            let mut jobs = self.jobs.jobs.lock().unwrap();
            let mut active = self.jobs.active_execs.lock().unwrap();
            if active
                .get(&id)
                .is_some_and(|active| Arc::ptr_eq(&active.cancellation, cancellation))
            {
                if let Some(active) = active.remove(&id) {
                    active.cancellation.cancel();
                    active.deposited.notify_one();
                }
                drop(active);
                jobs.remove(&id)
            } else {
                None
            }
        };
        if let Some(reply) = reply {
            capsem_core::try_send!("job_result_exec_blocked", reply.send(JobResult::Error { message }));
        }
    }
}

#[cfg(test)]
mod tests;
