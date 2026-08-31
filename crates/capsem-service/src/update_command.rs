//! Update command construction and systemd process ownership.

use capsem_service::api;
use std::ffi::OsStr;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdateCommandKind {
    Check,
    Apply,
}

pub(crate) fn update_command_plan(kind: UpdateCommandKind) -> api::UpdateCommandPlan {
    update_command_plan_for(
        kind,
        capsem_cli_program(),
        direct_systemd_invocation(
            std::env::var_os("INVOCATION_ID").as_deref(),
            std::env::var_os("SYSTEMD_EXEC_PID").as_deref(),
            std::process::id(),
        ),
    )
}

fn direct_systemd_invocation(invocation_id: Option<&OsStr>, exec_pid: Option<&OsStr>, current_pid: u32) -> bool {
    // Both values are inherited. Only the PID match proves systemd executed
    // this service directly instead of an unrelated ancestor.
    invocation_id.is_some_and(|value| !value.is_empty())
        && exec_pid
            .and_then(OsStr::to_str)
            .and_then(|value| value.parse::<u32>().ok())
            == Some(current_pid)
}

fn update_command_plan_for(
    kind: UpdateCommandKind,
    program: String,
    systemd_invocation: bool,
) -> api::UpdateCommandPlan {
    let args = match kind {
        UpdateCommandKind::Check => vec!["update".to_string(), "--check".to_string()],
        UpdateCommandKind::Apply => vec!["update".to_string(), "--yes".to_string()],
    };
    if kind == UpdateCommandKind::Apply && systemd_invocation {
        // Debian preinstall stops capsem.service. Own the complete transaction
        // in a sibling unit so systemd cannot kill its apt/dpkg descendants.
        // No --pipe: its reader dies with the service and could SIGPIPE us.
        let mut transient_args = vec![
            "--user".to_string(),
            "--wait".to_string(),
            "--collect".to_string(),
            "--quiet".to_string(),
            "--unit=capsem-update".to_string(),
            "--".to_string(),
            program,
        ];
        transient_args.extend(args);
        return api::UpdateCommandPlan {
            program: "systemd-run".to_string(),
            args: transient_args,
        };
    }
    api::UpdateCommandPlan { program, args }
}

fn capsem_cli_program() -> String {
    if let Some(path) = std::env::var_os("CAPSEM_CLI") {
        return PathBuf::from(path).display().to_string();
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("capsem");
            if sibling.exists() {
                return sibling.display().to_string();
            }
        }
    }
    "capsem".to_string()
}

#[cfg(test)]
mod tests;
