//! How `capsem list` and `capsem info` describe a session on screen.
use crate::client::SessionInfo;

pub(crate) fn format_uptime(secs: Option<u64>) -> String {
    match secs {
        None | Some(0) => "-".into(),
        Some(s) => {
            let days = s / 86400;
            let hours = (s % 86400) / 3600;
            let mins = (s % 3600) / 60;
            if days > 0 {
                format!("{}d {}h", days, hours)
            } else if hours > 0 {
                format!("{}h {:02}m", hours, mins)
            } else {
                format!("{}m", mins.max(1))
            }
        }
    }
}

/// The one line saying why a session cannot run, if it cannot.
///
/// The service splits the reason across two fields on purpose: a crashed VM
/// carries its `process.log` tail in `last_error`, one the service refuses to
/// resume carries the validation failure in `resume_blocked_reason`, and a
/// healthy session carries neither. Matching a field to a particular status is
/// how a `Stopped` VM that can never start came to print as a plain row.
pub(crate) fn session_blocked_reason(info: &SessionInfo) -> Option<&str> {
    info.last_error
        .as_deref()
        .or(info.resume_blocked_reason.as_deref())
        .map(capsem_core::session::boot_failure_summary)
}

pub(crate) fn print_session_info(info: &SessionInfo) {
    println!("Session: {}", info.id);
    if let Some(name) = &info.name {
        println!("Name:    {}", name);
    }
    println!("Status:  {}", info.status);
    // `capsem info` is what a user reaches for after `capsem list` shows a VM
    // that will not run. Printing the status without the reason sent them to
    // `capsem logs` to learn something the service had already returned here.
    if let Some(reason) = session_blocked_reason(info) {
        println!("Problem: {}", reason);
        if info.last_error.is_some() {
            println!("Logs:    capsem logs {}", info.id);
        }
    }
    if info.pid > 0 {
        println!("PID:     {}", info.pid);
    }
    if let Some(address) = &info.private_address {
        println!("Address: {}", address);
    }

    if info.ram_mb.is_some() || info.cpus.is_some() || info.version.is_some() {
        println!();
        if let Some(ram) = info.ram_mb {
            println!("RAM:     {} GB", ram / 1024);
        }
        if let Some(cpus) = info.cpus {
            println!("CPUs:    {}", cpus);
        }
        if let Some(ver) = &info.version {
            println!("Version: {}", ver);
        }
    }

    if let Some(from) = &info.forked_from {
        println!("Forked:  {}", from);
    }
    if let Some(desc) = &info.description {
        println!("Desc:    {}", desc);
    }

    let has_telemetry = info.created_at.is_some()
        || info.uptime_secs.is_some()
        || info.total_input_tokens.is_some()
        || info.total_tool_calls.is_some();
    if has_telemetry {
        println!();
        println!("Telemetry:");
        if let Some(created) = &info.created_at {
            println!("  Created:       {}", created);
        }
        if let Some(secs) = info.uptime_secs {
            println!("  Uptime:        {}", format_uptime(Some(secs)));
        }
        if let Some(inp) = info.total_input_tokens {
            println!("  Input Tokens:  {}", inp);
        }
        if let Some(out) = info.total_output_tokens {
            println!("  Output Tokens: {}", out);
        }
        if let Some(cost) = info.total_estimated_cost {
            println!("  Est. Cost:     ${:.2}", cost);
        }
        if let Some(tc) = info.total_tool_calls {
            println!("  Tool Calls:    {}", tc);
        }
        if info.total_requests.is_some() || info.allowed_requests.is_some() {
            let total = info.total_requests.unwrap_or(0);
            let allowed = info.allowed_requests.unwrap_or(0);
            let denied = info.denied_requests.unwrap_or(0);
            println!("  Requests:      {} ({} allowed, {} denied)", total, allowed, denied);
        }
        if let Some(fe) = info.total_file_events {
            println!("  File Events:   {}", fe);
        }
    }
}

#[cfg(test)]
mod tests;
