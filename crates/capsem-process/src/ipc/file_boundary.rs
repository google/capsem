use crate::mcp_runtime::McpRuntime;
use capsem_core::security_engine::approval::client::ApprovalCapability;
use capsem_proto::ipc::{FileBoundaryAction, ProcessToService, ServiceToProcess};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc::Sender;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

pub(crate) type SecurityRulesHandle = Arc<RwLock<Arc<capsem_core::net::policy_config::SecurityRuleSet>>>;
pub(crate) type PluginPolicyHandle = capsem_core::net::policy_config::SharedPluginPolicy;

/// Evaluate and record a host file operation independently of guest readiness.
pub(super) async fn spawn(
    jobs: &Arc<crate::job_store::JobStore>,
    runtime: &Arc<McpRuntime>,
    output: &Sender<ProcessToService>,
    request: ServiceToProcess,
    capacity: &Arc<Semaphore>,
    calls: &mut JoinSet<()>,
    cancellation: &CancellationToken,
) {
    let ServiceToProcess::LogFileBoundary {
        id,
        action,
        path,
        data,
        size,
        mime_type,
    } = request
    else {
        unreachable!("file boundary handler received another request")
    };
    let (permit, memory) = match (
        Arc::clone(capacity).try_acquire_owned().ok(),
        jobs.retain_file_bytes(
            data.capacity(),
            path.len().saturating_add(mime_type.as_ref().map_or(0, String::len)),
        ),
    ) {
        (Some(permit), Some(memory)) => (permit, memory),
        _ => {
            let _ = output
                .send(ProcessToService::LogFileBoundaryResult {
                    id,
                    success: false,
                    data: None,
                    error: Some("file boundary capacity exceeded".into()),
                })
                .await;
            return;
        }
    };
    let (db, rules, plugins, output) = (
        Arc::clone(&runtime.db),
        Arc::clone(&runtime.security_rules),
        Arc::clone(&runtime.plugin_policy),
        output.clone(),
    );
    let approvals = runtime.approvals.clone();
    let cancellation = cancellation.clone();
    calls.spawn(async move {
        let _permit = permit;
        let _memory = memory;
        let held_bytes = usize::try_from(size)
            .unwrap_or(usize::MAX)
            .max(data.len())
            .saturating_mul(4);
        let boundary = emit_explicit_file_security_event(
            &db,
            &rules,
            &plugins,
            FileSecurityBoundary::new(
                match action {
                    FileBoundaryAction::Import => capsem_logger::FileAction::Imported,
                    FileBoundaryAction::Export => capsem_logger::FileAction::Exported,
                },
                path,
                &data,
                size,
                mime_type,
            ),
            approvals.as_deref(),
            held_bytes,
        );
        let result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return,
            result = tokio::time::timeout(std::time::Duration::from_secs(30), boundary) => result,
        };
        let (success, data, error) = match result {
            Ok(Ok(Some(emission))) if emission.enforcement.is_allowed() => {
                (true, rewritten_file_content(&data, size, &emission.event), None)
            }
            Ok(Ok(Some(emission))) => (
                false,
                None,
                Some(
                    emission
                        .enforcement
                        .reason
                        .unwrap_or_else(|| "file boundary blocked by security policy".into()),
                ),
            ),
            Ok(Ok(None)) => (false, None, Some("failed to write file boundary security event".into())),
            Ok(Err(error)) => (false, None, Some(error)),
            Err(_) => (false, None, Some("log file boundary timed out".into())),
        };
        capsem_core::try_send!(
            "ipc_log_file_boundary_result",
            output
                .send(ProcessToService::LogFileBoundaryResult {
                    id,
                    success,
                    data,
                    error
                })
                .await
        );
    });
}

pub(crate) const FILE_SECURITY_CONTENT_PREVIEW_MAX: usize = 64 * 1024;

pub(crate) struct FileSecurityBoundary {
    action: capsem_logger::FileAction,
    path: String,
    size: Option<u64>,
    content: Option<String>,
    mime_type: Option<String>,
    complete_text: bool,
}

impl FileSecurityBoundary {
    pub(crate) fn new(
        action: capsem_logger::FileAction,
        path: String,
        data: &[u8],
        size: u64,
        mime_type: Option<String>,
    ) -> Self {
        Self {
            action,
            path,
            size: Some(size),
            content: Some(file_content_preview(data)),
            mime_type,
            complete_text: data.len() as u64 == size
                && data.len() <= FILE_SECURITY_CONTENT_PREVIEW_MAX
                && std::str::from_utf8(data).is_ok(),
        }
    }
}

pub(crate) fn file_content_preview(data: &[u8]) -> String {
    String::from_utf8_lossy(&data[..data.len().min(FILE_SECURITY_CONTENT_PREVIEW_MAX)]).into_owned()
}

pub(crate) async fn emit_explicit_file_security_event(
    db: &Arc<capsem_logger::DbWriter>,
    security_rules: &SecurityRulesHandle,
    plugin_policy: &PluginPolicyHandle,
    boundary: FileSecurityBoundary,
    approvals: Option<&ApprovalCapability>,
    held_bytes: usize,
) -> Result<Option<capsem_core::security_engine::SecurityRuleEmission>, String> {
    let rules = security_rules.read().unwrap().clone();
    let plugins = plugin_policy.read().unwrap().clone();
    let Some(mut emission) = capsem_core::security_engine::emit_explicit_file_security_write_and_rules_with_plugins(
        db,
        &rules,
        Arc::clone(&plugins),
        capsem_core::security_engine::ExplicitFileSecurityEvent {
            action: boundary.action,
            path: boundary.path,
            size: boundary.size,
            content: boundary.content,
            mime_type: boundary.mime_type,
            trace_id: None,
            credential_ref: None,
        },
    )
    .await?
    else {
        return Ok(None);
    };
    // A bounded/lossy forensic preview cannot reconstruct a complete file.
    // Refuse before asking a human to approve an impossible safe rewrite.
    if mutating_file_rewrite(&emission.event) && !boundary.complete_text {
        return Err("file rewrite requires complete UTF-8 inspection within the content limit".into());
    }
    let held_bytes = if emission.enforcement.action == capsem_core::security_engine::SecurityEnforcementAction::Ask {
        // Include semantic metadata and plugin-produced content as well as
        // original bytes. A tiny preview or expanding rewrite cannot buy a
        // larger live continuation than the shared retained-memory admission.
        approval_held_bytes(&emission.event, held_bytes)
    } else {
        held_bytes
    };
    capsem_core::security_engine::approval::resolve_emission(db, approvals, &mut emission, held_bytes).await?;
    if !Arc::ptr_eq(&rules, &security_rules.read().unwrap()) || !Arc::ptr_eq(&plugins, &plugin_policy.read().unwrap()) {
        return Err("file security policy changed while operation was held".into());
    }
    Ok(Some(emission))
}

pub(crate) fn rewritten_file_content(
    original_preview: &[u8],
    original_size: u64,
    event: &capsem_core::security_engine::SecurityEvent,
) -> Option<Vec<u8>> {
    if original_preview.len() as u64 != original_size {
        return None;
    }
    if !mutating_file_rewrite(event) {
        return None;
    }
    let file = event.file.as_ref()?;
    let content = file
        .import_content
        .as_deref()
        .or(file.export_content.as_deref())
        .or(file.read_content.as_deref())
        .or(file.write_content.as_deref())
        .or(file.create_content.as_deref())
        .or(file.delete_content.as_deref())
        .or(file.content.as_deref())?;
    if content.as_bytes() == original_preview {
        None
    } else {
        Some(content.as_bytes().to_vec())
    }
}

fn mutating_file_rewrite(event: &capsem_core::security_engine::SecurityEvent) -> bool {
    event.plugin_executions.iter().any(|execution| {
        execution.applied
            && !matches!(
                execution.stage,
                capsem_core::security_engine::SecurityPluginStage::Logging
            )
            && event.detections.iter().any(|detection| {
                detection.plugin_id.as_deref() == Some(execution.plugin_id.as_str())
                    && detection.plugin_mode == Some(capsem_core::net::policy_config::SecurityPluginMode::Rewrite)
            })
    })
}

// This boundary retains only file facts. Reject unexpected protocol contexts
// rather than silently under-accounting an expanded plugin result.
fn approval_held_bytes(event: &capsem_core::security_engine::SecurityEvent, original: usize) -> usize {
    if event.container.is_some()
        || event.network.is_some()
        || event.http_request.is_some()
        || event.http.is_some()
        || event.dns.is_some()
        || event.mcp.is_some()
        || event.model.is_some()
        || event.process.is_some()
        || event.ip.is_some()
        || event.tcp.is_some()
        || event.udp.is_some()
        || !event.credential_observations.is_empty()
        || !event.credential_injections.is_empty()
    {
        return usize::MAX;
    }
    let mut bytes = std::mem::size_of_val(event)
        .saturating_add(event.trace_id.as_ref().map_or(0, String::capacity))
        .saturating_add(event.credential_ref.as_ref().map_or(0, String::capacity))
        .saturating_add(
            event
                .action_trace
                .capacity()
                .saturating_mul(std::mem::size_of::<capsem_core::net::policy_config::PolicyActionId>()),
        )
        .saturating_add(
            event
                .detections
                .capacity()
                .saturating_mul(std::mem::size_of::<capsem_core::security_engine::SecurityDetectionEvent>()),
        )
        .saturating_add(
            event
                .plugin_executions
                .capacity()
                .saturating_mul(std::mem::size_of::<capsem_core::security_engine::SecurityPluginExecution>()),
        );
    for detection in &event.detections {
        for value in [&detection.rule_id, &detection.plugin_id, &detection.reason]
            .into_iter()
            .flatten()
        {
            bytes = bytes.saturating_add(value.capacity());
        }
    }
    for execution in &event.plugin_executions {
        bytes = bytes.saturating_add(execution.plugin_id.capacity());
    }
    if let Some(file) = &event.file {
        // Exhaustive destructuring makes new file slots require an accounting review.
        let capsem_core::security_engine::FileSecurityEvent {
            import_path,
            import_name,
            import_ext,
            import_mime_type,
            import_content,
            export_path,
            export_name,
            export_ext,
            export_mime_type,
            export_content,
            read_path,
            read_name,
            read_ext,
            read_mime_type,
            read_content,
            create_path,
            create_name,
            create_ext,
            create_mime_type,
            create_content,
            write_path,
            write_name,
            write_ext,
            write_mime_type,
            write_content,
            delete_path,
            delete_name,
            delete_ext,
            delete_mime_type,
            delete_content,
            content,
            kind,
        } = file;
        for value in [
            import_path,
            import_name,
            import_ext,
            import_mime_type,
            import_content,
            export_path,
            export_name,
            export_ext,
            export_mime_type,
            export_content,
            read_path,
            read_name,
            read_ext,
            read_mime_type,
            read_content,
            create_path,
            create_name,
            create_ext,
            create_mime_type,
            create_content,
            write_path,
            write_name,
            write_ext,
            write_mime_type,
            write_content,
            delete_path,
            delete_name,
            delete_ext,
            delete_mime_type,
            delete_content,
            content,
            kind,
        ]
        .into_iter()
        .flatten()
        {
            bytes = bytes.saturating_add(value.capacity());
        }
    }
    // Cover retained policy/forensic copies conservatively without allocating
    // or cloning the event simply to estimate its memory.
    original.saturating_add(bytes.saturating_mul(4))
}

#[cfg(test)]
mod tests;
