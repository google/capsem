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
    pub(super) hub: crate::guest_control::Sender,
    pub(super) approvals: Option<Arc<capsem_core::security_engine::approval::client::ApprovalCapability>>,
}

const VM_USER_SWITCH_SCRIPT: &str = concat!(
    "_u=$1; _c=$2; _g=\"\"; ",
    "case \"$_u\" in *:*) _g=${_u#*:}; _u=${_u%%:*};; esac; ",
    "case \"$_u\" in ''|*[!0-9]*) ",
    "_pw=$(getent passwd \"$_u\" 2>/dev/null || grep -E \"^${_u}:\" /etc/passwd 2>/dev/null | head -n 1); ",
    "[ -n \"$_pw\" ] || { echo \"capsem: unknown user: $_u\" >&2; exit 125; }; ",
    "_uid=$(printf '%s' \"$_pw\" | cut -d: -f3); _gid=$(printf '%s' \"$_pw\" | cut -d: -f4); _un=\"$_u\";; ",
    "*) _uid=\"$_u\"; ",
    "_pw=$(getent passwd \"$_uid\" 2>/dev/null || grep -E \"^[^:]*:[^:]*:${_uid}:\" /etc/passwd 2>/dev/null | head -n 1); ",
    "_un=$(printf '%s' \"$_pw\" | cut -d: -f1); _gid=$(printf '%s' \"$_pw\" | cut -d: -f4); ",
    "[ -n \"$_gid\" ] || _gid=0;; esac; ",
    "if [ -n \"$_g\" ]; then case \"$_g\" in ''|*[!0-9]*) ",
    "_gr=$(getent group \"$_g\" 2>/dev/null || grep -E \"^${_g}:\" /etc/group 2>/dev/null | head -n 1); ",
    "[ -n \"$_gr\" ] || { echo \"capsem: unknown group: $_g\" >&2; exit 125; }; ",
    "_gid=$(printf '%s' \"$_gr\" | cut -d: -f3);; ",
    "*) _gid=\"$_g\";; esac; fi; ",
    "_hm=$(printf '%s' \"$_pw\" | cut -d: -f6); ",
    "export HOME=\"${_hm:-/}\" USER=\"${_un:-$_uid}\" LOGNAME=\"${_un:-$_uid}\"; ",
    "_sh=/bin/bash; [ -x \"$_sh\" ] || _sh=/bin/sh; ",
    "if [ -n \"$_un\" ] && [ -z \"$_g\" ]; then exec su -m \"$_un\" -s \"$_sh\" -c \"$_c\"; fi; ",
    "exec setpriv --reuid=\"$_uid\" --regid=\"$_gid\" --clear-groups \"$_sh\" -c \"$_c\"",
);

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

fn is_vm_root_user(user: &str) -> bool {
    matches!(user, "root" | "0" | "root:root" | "0:0" | "root:0" | "0:root")
}

/// The command the guest agent runs for `command` in `target`.
pub(super) fn guest_command(
    command: String,
    target: ExecTarget,
    user: Option<&str>,
    cwd: Option<&str>,
) -> String {
    match target {
        ExecTarget::Vm => {
            let inner = match cwd {
                Some(dir) => format!("cd -- {} && exec bash -c {}", shell_quote(dir), shell_quote(&command)),
                None => command,
            };
            match user.filter(|u| !is_vm_root_user(u)) {
                Some(user_spec) => format!(
                    "/bin/sh -c {} sh {} {}",
                    shell_quote(VM_USER_SWITCH_SCRIPT),
                    shell_quote(user_spec),
                    shell_quote(&inner)
                ),
                None => inner,
            }
        }
        ExecTarget::Workload => capsem_core::container::workload_exec_command_with(&command, user, cwd),
    }
}

impl ExecDispatch {
    pub(super) fn spawn(
        &self,
        calls: &mut tokio::task::JoinSet<()>,
        id: u64,
        command: String,
        target: ExecTarget,
        user: Option<String>,
        cwd: Option<String>,
    ) {
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
        calls.spawn(async move {
            dispatch
                .dispatch_original(id, command, target, user, cwd, cancellation)
                .await
        });
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
    pub(super) async fn dispatch(
        &self,
        id: u64,
        command: String,
        target: ExecTarget,
        user: Option<String>,
        cwd: Option<String>,
    ) {
        let Some(cancellation) = self.cancellation(id) else {
            return;
        };
        self.dispatch_original(id, command, target, user, cwd, cancellation)
            .await;
    }

    async fn dispatch_original(
        &self,
        id: u64,
        command: String,
        target: ExecTarget,
        user: Option<String>,
        cwd: Option<String>,
        cancellation: Arc<tokio_util::sync::CancellationToken>,
    ) {
        let held_bytes = command
            .len()
            .saturating_add(user.as_ref().map_or(0, String::len))
            .saturating_add(cwd.as_ref().map_or(0, String::len))
            .saturating_mul(4);
        if cancellation.is_cancelled() || held_bytes > capsem_proto::APPROVAL_HELD_BYTES_PER_REQUEST {
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
                user: user.clone(),
                cwd: cwd.clone(),
                trace_id,
                process_name: None,
                credential_ref: None,
            },
        )
        .await;
        if let Ok(Some(emission)) = &mut boundary {
            let result = self
                .resolve_approval(id, emission, held_bytes, &cancellation)
                .await;
            if let Err(error) = result {
                boundary = Err(error);
            } else if !Arc::ptr_eq(&rules, &self.rules.read().unwrap())
                || !Arc::ptr_eq(&plugins, &self.plugins.read().unwrap())
            {
                emission.enforcement.action = capsem_core::security_engine::SecurityEnforcementAction::Block;
                emission.event.decision.effective = capsem_core::security_engine::SecurityDecisionKind::Block;
                emission.enforcement.reason = Some("policy changed while execution was pending".into());
            } else if emission.event.process.as_ref().is_none_or(|process| {
                process.command.as_deref() != Some(command.as_str())
                    || process.user.as_deref() != user.as_deref()
                    || process.cwd.as_deref() != cwd.as_deref()
            }) {
                emission.enforcement.action = capsem_core::security_engine::SecurityEnforcementAction::Block;
                emission.event.decision.effective = capsem_core::security_engine::SecurityDecisionKind::Block;
                emission.enforcement.reason =
                    Some("execution context changed while approval was pending".into());
            }
        }
        // The command has not reached the guest yet, so a non-allow decision
        // is enforceable here on the same terms as the network and file
        // boundaries: withhold the dispatch and fail the caller's job.
        if let Some(refusal) = exec_boundary_refusal(id, &boundary) {
            self.refuse(id, &cancellation, refusal);
            return;
        }
        let command = guest_command(command, target, user.as_deref(), cwd.as_deref());
        let frame = match crate::guest_control::Prepared::new(&HostToGuest::Exec { id, command }) {
            Ok(frame) => frame,
            Err(error) => {
                self.refuse(id, &cancellation, format!("execution control frame refused: {error}"));
                return;
            }
        };
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
                permit.send(frame);
                true
            } else {
                false
            }
        };
        if !sent {
            self.refuse(id, &cancellation, "execution canceled before guest delivery".into());
        }
    }

    async fn resolve_approval(
        &self,
        id: u64,
        emission: &mut capsem_core::security_engine::SecurityRuleEmission,
        held_bytes: usize,
        cancellation: &Arc<tokio_util::sync::CancellationToken>,
    ) -> Result<(), String> {
        use capsem_proto::ipc::{ExecApprovalProgress, ProcessToService};
        if emission.enforcement.action != capsem_core::security_engine::SecurityEnforcementAction::Ask {
            return Ok(());
        }
        let (registered, registration) = tokio::sync::oneshot::channel();
        let resolution = capsem_core::security_engine::approval::resolve_emission_registered(
            &self.db,
            self.approvals.as_deref(),
            emission,
            held_bytes,
            Some(registered),
        );
        tokio::pin!(resolution);
        let registration = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err("execution caller stopped while approval was pending".into()),
            result = &mut resolution => return result,
            registration = registration => registration.ok(),
        };
        let output = self
            .jobs
            .active_execs
            .lock()
            .unwrap()
            .get(&id)
            .filter(|active| Arc::ptr_eq(&active.cancellation, cancellation))
            .and_then(|active| active.approval_progress.clone());
        let progress = if let (Some(registration), Some(output)) = (registration, output) {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(registration.remaining_ms);
            send_approval_progress(
                &output,
                ProcessToService::ExecApproval {
                    id,
                    progress: ExecApprovalProgress::Registered(registration.clone()),
                },
                cancellation,
                deadline,
            )
            .await?;
            Some((output, registration, deadline))
        } else {
            None
        };
        let result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err("execution caller stopped while approval was pending".into()),
            result = &mut resolution => result,
        };
        if let Some((output, registration, deadline)) = progress {
            send_approval_progress(
                &output,
                ProcessToService::ExecApproval {
                    id,
                    progress: ExecApprovalProgress::Resolved {
                        ask_id: registration.ask_id,
                        event_id: registration.event_id,
                    },
                },
                cancellation,
                deadline,
            )
            .await?;
        }
        result
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

async fn send_approval_progress(
    output: &mpsc::Sender<capsem_proto::ipc::ProcessToService>,
    message: capsem_proto::ipc::ProcessToService,
    cancellation: &tokio_util::sync::CancellationToken,
    deadline: tokio::time::Instant,
) -> Result<(), String> {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err("execution caller stopped during approval progress".into()),
        sent = tokio::time::timeout_at(deadline, output.send(message)) => match sent {
            Ok(Ok(())) => Ok(()),
            _ => Err("execution caller did not receive bounded approval progress".into()),
        },
    }
}

#[cfg(test)]
mod tests;
