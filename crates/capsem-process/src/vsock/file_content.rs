use std::sync::Arc;

use capsem_logger::{DbWriter, FileAction};

use crate::ipc::file_boundary::{
    emit_explicit_file_security_event, file_content_preview, FileSecurityBoundary, PluginPolicyHandle,
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
    let ActiveFileOp::Read { path } = context else {
        unreachable!("only active reads admit FileContent")
    };
    let action = FileAction::Exported;
    let boundary = emit_explicit_file_security_event(
        &db,
        &security_rules,
        &plugin_policy,
        FileSecurityBoundary {
            action,
            path,
            size: Some(data.len() as u64),
            content: Some(file_content_preview(&data)),
            mime_type: None,
        },
    )
    .await;
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
    if let Some(reason) = refusal {
        let error = Some(format!("file export refused: {reason}"));
        if let Some(tx) = js.jobs.lock().unwrap().remove(&id) {
            capsem_core::try_send!(
                "job_result_read_file_blocked",
                tx.send(JobResult::ReadFile { data: None, error })
            );
        }
        return;
    }
    if let Some(tx) = js.jobs.lock().unwrap().remove(&id) {
        capsem_core::try_send!(
            "job_result_read_file",
            tx.send(JobResult::ReadFile {
                data: Some(data),
                error: None
            })
        );
    }
}
