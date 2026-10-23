use std::sync::Arc;

use capsem_logger::{DbWriter, FileAction};

use crate::ipc::file_boundary::{
    emit_explicit_file_security_event, rewritten_file_content, FileSecurityBoundary, PluginPolicyHandle,
    SecurityRulesHandle,
};
use crate::job_store::{ActiveFileOp, JobResult, JobStore};

pub(super) async fn complete(
    id: u64,
    context: ActiveFileOp,
    data: Vec<u8>,
    js: Arc<JobStore>,
    db: Arc<DbWriter>,
    security_rules: SecurityRulesHandle,
    plugin_policy: PluginPolicyHandle,
) {
    let ActiveFileOp::Read { path, state } = context else {
        unreachable!("only active reads admit FileContent")
    };
    // Take this original sender once, before any wait. Never look up an id
    // again to deliver bytes after approval: the id may have been reused.
    let Some(mut sender) = js.take_file_read_reply(id, &state) else {
        js.finish_file_read(id, &state);
        return;
    };
    let action = FileAction::Exported;
    let evaluate = emit_explicit_file_security_event(
        &db,
        &security_rules,
        &plugin_policy,
        FileSecurityBoundary::new(action, path, &data, data.len() as u64, None),
        state.approvals.as_deref(),
        data.len().saturating_mul(4),
    );
    let boundary = tokio::select! {
        biased;
        _ = state.cancellation.cancelled() => Err("file export cancelled".into()),
        _ = sender.closed() => Err("file export caller disconnected".into()),
        result = evaluate => result,
    };
    // Exported bytes are delivered only after their ledger event is admitted
    // and allowed. A failed ledger or policy evaluation refuses the export.
    let refusal = match &boundary {
        Ok(Some(emission)) if emission.enforcement.is_allowed() => None,
        Ok(Some(emission)) => Some(
            emission
                .enforcement
                .reason
                .clone()
                .unwrap_or_else(|| "blocked by policy".into()),
        ),
        Ok(None) => Some("its security event could not be recorded".into()),
        Err(error) => Some(format!("its security event failed: {error}")),
    };
    let result = if let Some(reason) = refusal {
        JobResult::ReadFile {
            data: None,
            error: Some(format!("file export refused: {reason}")),
        }
    } else {
        let data = boundary
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .and_then(|emission| rewritten_file_content(&data, data.len() as u64, &emission.event))
            .unwrap_or(data);
        JobResult::ReadFile {
            data: Some(data),
            error: None,
        }
    };
    js.deliver_file_read(id, &state, sender, result);
    js.finish_file_read(id, &state);
}

#[cfg(test)]
mod tests;
