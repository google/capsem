//! Verify that a supervisor owns this exact process and restarts clean exits.

use capsem_api::ServiceManager;
use std::time::Duration;

pub async fn managed_service() -> Option<ServiceManager> {
    let pid = std::process::id();
    #[cfg(target_os = "macos")]
    {
        let jobs = query("launchctl", &["list"]).await?;
        let label = launchd_label(&jobs, pid)?;
        let job = query("launchctl", &["list", label]).await?;
        launchd_restarts(&job, pid).then_some(ServiceManager::Launchd)
    }
    #[cfg(target_os = "linux")]
    {
        let job = query(
            "systemctl",
            &["--user", "show", "capsem.service", "--property=MainPID,Restart"],
        )
        .await?;
        systemd_restarts(&job, pid).then_some(ServiceManager::Systemd)
    }
}

async fn query(program: &str, args: &[&str]) -> Option<String> {
    let mut command = tokio::process::Command::new(program);
    command.args(args).stdin(std::process::Stdio::null()).kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(3), command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

#[cfg(any(target_os = "macos", test))]
fn launchd_label(jobs: &str, pid: u32) -> Option<&str> {
    jobs.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let job_pid = fields.next()?.parse::<u32>().ok()?;
        let _exit_status = fields.next()?;
        let label = fields.next()?;
        (job_pid == pid && fields.next().is_none()).then_some(label)
    })
}

#[cfg(any(target_os = "macos", test))]
fn launchd_restarts(job: &str, pid: u32) -> bool {
    // launchctl list <label> uses a tab-indented dictionary. Only top-level
    // properties count; an EnvironmentVariables key is not supervisor proof.
    let properties: Vec<_> = job
        .lines()
        .filter_map(|line| line.strip_prefix('\t'))
        .filter(|line| !line.starts_with('\t'))
        .collect();
    properties.contains(&"\"OnDemand\" = false;") && properties.contains(&format!("\"PID\" = {pid};").as_str())
}

#[cfg(any(target_os = "linux", test))]
fn systemd_restarts(job: &str, pid: u32) -> bool {
    job.lines().any(|line| line == format!("MainPID={pid}"))
        && job
            .lines()
            .any(|line| line == "Restart=always" || line == "Restart=on-success")
}

#[cfg(test)]
mod tests;
