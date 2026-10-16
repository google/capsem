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
pub(super) struct ExecDispatch {
    pub(super) vm_id: String,
    pub(super) db: Arc<capsem_logger::DbWriter>,
    pub(super) rules: SecurityRulesHandle,
    pub(super) plugins: PluginPolicyHandle,
    pub(super) jobs: Arc<JobStore>,
    pub(super) hub: mpsc::Sender<HostToGuest>,
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
    pub(super) async fn dispatch(
        &self,
        id: u64,
        command: String,
        target: ExecTarget,
        user: Option<String>,
        cwd: Option<String>,
    ) {
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
                target,
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
        let command = guest_command(command, target, user.as_deref(), cwd.as_deref());
        capsem_core::try_send!("hub_exec", self.hub.send(HostToGuest::Exec { id, command }).await);
    }
}

#[cfg(test)]
mod tests;
