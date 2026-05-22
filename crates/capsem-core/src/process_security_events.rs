//! Canonical Security Engine projection for process activity rows.

use std::path::Path;

use capsem_logger::ExecEvent;
use capsem_security_engine::{
    AiAttributionScope, AiOriginKind, Enforceability, ProcessSecuritySubject, RedactionState,
    ResolvedSecurityEvent, SecurityAction, SecurityEvent, SecurityEventCommon, SourceEngine,
    RESOLVED_EVENT_SCHEMA_VERSION,
};

/// Build the normalized Security Engine journal row for an exec request.
pub fn build_exec_resolved_security_event(event: &ExecEvent) -> ResolvedSecurityEvent {
    let timestamp_unix_ms = event
        .timestamp
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let security_event = SecurityEvent::process(
        SecurityEventCommon {
            event_id: process_security_event_id(
                event.trace_id.as_deref(),
                event.exec_id,
                &event.command,
                timestamp_unix_ms,
            ),
            parent_event_id: None,
            stream_id: None,
            activity_id: Some(event.source.clone()),
            sequence_no: None,
            source_engine: SourceEngine::Process,
            attribution_scope: AiAttributionScope::Vm,
            origin_kind: AiOriginKind::HostService,
            accounting_owner: None,
            enforceability: Enforceability::ObserveOnly,
            trace_id: event.trace_id.clone(),
            span_id: None,
            timestamp_unix_ms,
            vm_id: non_empty_env(crate::telemetry::CAPSEM_VM_ID_ENV),
            session_id: non_empty_env(crate::telemetry::CAPSEM_SESSION_ID_ENV),
            profile_id: non_empty_env(crate::telemetry::CAPSEM_PROFILE_ID_ENV),
            profile_revision: non_empty_env(crate::telemetry::CAPSEM_PROFILE_REVISION_ENV),
            profile_pack_ids: Vec::new(),
            enforcement_packs: Vec::new(),
            detection_packs: Vec::new(),
            user_id: non_empty_env(crate::telemetry::CAPSEM_USER_ID_ENV),
            process_id: None,
            parent_process_id: None,
            exec_id: Some(event.exec_id.to_string()),
            turn_id: None,
            message_id: None,
            tool_call_id: None,
            mcp_call_id: event.mcp_call_id.map(|id| id.to_string()),
            event_type: "process.exec".into(),
            redaction_state: RedactionState::Raw,
        },
        ProcessSecuritySubject {
            operation: "exec".into(),
            command_class: classify_command(&event.command).map(str::to_owned),
        },
    );

    ResolvedSecurityEvent {
        schema_version: RESOLVED_EVENT_SCHEMA_VERSION,
        event: security_event,
        steps: Vec::new(),
        plugin_transforms: Vec::new(),
        detection_findings: Vec::new(),
        final_action: SecurityAction::Continue,
        emitter_results: Vec::new(),
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn classify_command(command: &str) -> Option<&'static str> {
    let executable = command
        .split_whitespace()
        .next()?
        .trim_matches(|ch| ch == '\'' || ch == '"');
    let executable = Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(executable);
    match executable {
        "bash" | "dash" | "fish" | "sh" | "zsh" => Some("shell"),
        "python" | "python3" | "pip" | "pip3" | "uv" => Some("python"),
        "node" | "npm" | "pnpm" | "yarn" | "bun" => Some("javascript"),
        "cargo" | "rustc" | "rustup" => Some("rust"),
        "curl" | "dig" | "host" | "nc" | "nslookup" | "wget" => Some("network"),
        _ => Some("other"),
    }
}

fn process_security_event_id(
    trace_id: Option<&str>,
    exec_id: u64,
    command: &str,
    timestamp_unix_ms: u64,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(trace_id.unwrap_or("").as_bytes());
    hasher.update(&exec_id.to_be_bytes());
    hasher.update(command.as_bytes());
    hasher.update(&timestamp_unix_ms.to_be_bytes());
    format!("process-{}", hasher.finalize().to_hex()[..16].to_string())
}

#[cfg(test)]
mod tests;
