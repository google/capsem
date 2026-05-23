//! Process Engine security-event projection and inline exec evaluation.
//!
//! This crate owns process/audit event normalization for the bedrock engine
//! split. Process mechanics stay outside the Security Engine; this crate
//! produces typed events and applies typed Security Engine decisions to
//! process exec requests.

use std::path::Path;

use capsem_logger::ExecEvent;
use capsem_security_engine::{
    AiAttributionScope, AiOriginKind, Enforceability, ProcessSecuritySubject, RedactionState,
    ResolvedEventStep, ResolvedEventStepKind, ResolvedSecurityEvent, SecurityAction,
    SecurityEngineError, SecurityError, SecurityEvent, SecurityEventCommon, SourceEngine,
    StepStatus, RESOLVED_EVENT_SCHEMA_VERSION,
};

pub trait RuntimeSecurityEngine: Send + Sync {
    fn evaluate(
        &self,
        event: SecurityEvent,
    ) -> Result<capsem_security_engine::SecurityResult, SecurityEngineError>;
}

impl RuntimeSecurityEngine for std::sync::Mutex<capsem_security_engine::SecurityEngine> {
    fn evaluate(
        &self,
        event: SecurityEvent,
    ) -> Result<capsem_security_engine::SecurityResult, SecurityEngineError> {
        let mut engine = self
            .lock()
            .map_err(|error| SecurityEngineError::PhaseFailed {
                phase: capsem_security_engine::SecurityEnginePhase::Enforcement,
                message: format!("runtime security engine lock poisoned: {error}"),
            })?;
        engine.evaluate(event)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessExecSecurityEvaluation {
    pub resolved_event: ResolvedSecurityEvent,
    pub allow_guest_exec: bool,
    pub denial_message: Option<String>,
}

/// Build the normalized Security Engine journal row for an exec request.
pub fn build_exec_resolved_security_event(event: &ExecEvent) -> ResolvedSecurityEvent {
    initial_resolved_exec_event(build_exec_security_event(event))
}

/// Evaluate an exec request against the runtime Security Engine before it is
/// delivered to the guest.
pub fn evaluate_exec_security_event(
    event: &ExecEvent,
    engine: Option<&dyn RuntimeSecurityEngine>,
) -> ProcessExecSecurityEvaluation {
    let security_event = build_exec_security_event(event);
    let Some(engine) = engine else {
        return ProcessExecSecurityEvaluation {
            resolved_event: initial_resolved_exec_event(security_event),
            allow_guest_exec: true,
            denial_message: None,
        };
    };

    match engine.evaluate(security_event.clone()) {
        Ok(result) => {
            let denial_message = exec_denial_message(&result.resolved_event.final_action);
            ProcessExecSecurityEvaluation {
                resolved_event: result.resolved_event,
                allow_guest_exec: denial_message.is_none(),
                denial_message,
            }
        }
        Err(error) => {
            let resolved_event = engine_error_resolved_exec_event(security_event, error);
            let denial_message = exec_denial_message(&resolved_event.final_action);
            ProcessExecSecurityEvaluation {
                resolved_event,
                allow_guest_exec: false,
                denial_message,
            }
        }
    }
}

fn build_exec_security_event(event: &ExecEvent) -> SecurityEvent {
    let timestamp_unix_ms = event
        .timestamp
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    SecurityEvent::process(
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
            enforceability: Enforceability::InlineBlockable,
            trace_id: event.trace_id.clone(),
            span_id: None,
            timestamp_unix_ms,
            vm_id: non_empty_env("CAPSEM_VM_ID"),
            session_id: non_empty_env("CAPSEM_SESSION_ID"),
            profile_id: non_empty_env("CAPSEM_PROFILE_ID"),
            profile_revision: non_empty_env("CAPSEM_PROFILE_REVISION"),
            profile_pack_ids: Vec::new(),
            enforcement_packs: Vec::new(),
            detection_packs: Vec::new(),
            user_id: non_empty_env("CAPSEM_USER_ID"),
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
            command_class: classify_command_class(&event.command).map(str::to_owned),
        },
    )
}

fn initial_resolved_exec_event(security_event: SecurityEvent) -> ResolvedSecurityEvent {
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

fn engine_error_resolved_exec_event(
    security_event: SecurityEvent,
    error: SecurityEngineError,
) -> ResolvedSecurityEvent {
    let message = error.to_string();
    let action = SecurityAction::Error(SecurityError {
        code: "process_engine_error".into(),
        message: message.clone(),
    });
    ResolvedSecurityEvent {
        schema_version: RESOLVED_EVENT_SCHEMA_VERSION,
        event: security_event,
        steps: vec![ResolvedEventStep {
            kind: ResolvedEventStepKind::EnforcementMatch,
            status: StepStatus::Error,
            rule_id: None,
            pack_id: None,
            message: Some(message),
        }],
        plugin_transforms: Vec::new(),
        detection_findings: Vec::new(),
        final_action: action,
        emitter_results: Vec::new(),
    }
}

fn exec_denial_message(action: &SecurityAction) -> Option<String> {
    match action {
        SecurityAction::Continue | SecurityAction::ObserveOnly => None,
        SecurityAction::Block(block) => Some(match block.rule_id.as_deref() {
            Some(rule_id) => format!("process exec blocked by {rule_id}: {}", block.reason_code),
            None => format!("process exec blocked: {}", block.reason_code),
        }),
        SecurityAction::Ask(plan) => Some(format!(
            "process exec requires confirmation {}: {}",
            plan.prompt_id, plan.reason_code
        )),
        SecurityAction::Rewrite(patch) => Some(format!(
            "process exec rewrite is not supported for {}",
            patch.target
        )),
        SecurityAction::Throttle(plan) => Some(format!(
            "process exec throttled by {}: {}",
            plan.quota_id, plan.reason_code
        )),
        SecurityAction::Quarantine(plan) => Some(format!(
            "process exec quarantined by {}",
            plan.quarantine_id
        )),
        SecurityAction::Restore(plan) => Some(format!(
            "process exec restore requested for {}: {}",
            plan.snapshot_id, plan.reason_code
        )),
        SecurityAction::DropConnection(reason) => {
            Some(format!("process exec dropped: {}", reason.reason_code))
        }
        SecurityAction::Error(error) => Some(format!(
            "process exec security engine error {}: {}",
            error.code, error.message
        )),
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn classify_command_class(command: &str) -> Option<&'static str> {
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
