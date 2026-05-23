//! Canonical Security Engine projection for file activity rows.

use std::path::Path;

use capsem_logger::FileEvent;
use capsem_security_engine::{
    AiAttributionScope, AiOriginKind, Enforceability, FileSecuritySubject, RedactionState,
    ResolvedSecurityEvent, SecurityAction, SecurityEvent, SecurityEventCommon, SourceEngine,
    RESOLVED_EVENT_SCHEMA_VERSION,
};

/// Build the normalized Security Engine journal row for a file activity event.
pub fn build_file_resolved_security_event(event: &FileEvent) -> ResolvedSecurityEvent {
    let timestamp_duration = event
        .timestamp
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let timestamp_unix_ms = timestamp_duration.as_millis() as u64;
    let timestamp_unix_nanos = timestamp_duration.as_nanos();
    let security_event = SecurityEvent::file(
        SecurityEventCommon {
            event_id: file_security_event_id(
                event.trace_id.as_deref(),
                event.action.as_str(),
                &event.path,
                timestamp_unix_nanos,
            ),
            parent_event_id: None,
            stream_id: None,
            activity_id: None,
            sequence_no: None,
            source_engine: SourceEngine::File,
            attribution_scope: AiAttributionScope::Vm,
            origin_kind: AiOriginKind::GuestNetwork,
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
            exec_id: None,
            turn_id: None,
            message_id: None,
            tool_call_id: None,
            mcp_call_id: None,
            event_type: "file.activity".into(),
            redaction_state: RedactionState::Raw,
        },
        FileSecuritySubject {
            operation: event.action.as_str().into(),
            path: Some(event.path.clone()),
            path_class: file_path_class(&event.path).into(),
            byte_count: event.size,
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

fn file_path_class(path: &str) -> &'static str {
    let path = path.split_once(" (from ").map_or(path, |(path, _)| path);
    let parsed = Path::new(path);
    if path.contains("/workspace/")
        || parsed.starts_with("/workspace")
        || parsed.starts_with("/root")
    {
        return "workspace";
    }
    if parsed.starts_with("/tmp") || parsed.starts_with("/var/tmp") {
        return "temporary";
    }
    if parsed.starts_with("/etc") || parsed.starts_with("/usr") || parsed.starts_with("/bin") {
        return "system";
    }
    if parsed.is_absolute() {
        return "absolute";
    }
    "relative"
}

fn file_security_event_id(
    trace_id: Option<&str>,
    operation: &str,
    path: &str,
    timestamp_unix_nanos: u128,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(trace_id.unwrap_or("").as_bytes());
    hasher.update(operation.as_bytes());
    hasher.update(path.as_bytes());
    hasher.update(&timestamp_unix_nanos.to_be_bytes());
    format!("file-{}", hasher.finalize().to_hex()[..16].to_string())
}

#[cfg(test)]
mod tests;
