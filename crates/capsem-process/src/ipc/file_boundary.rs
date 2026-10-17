use crate::mcp_runtime::McpRuntime;
use capsem_proto::ipc::{FileBoundaryAction, ProcessToService, ServiceToProcess};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc::Sender;

pub(crate) type SecurityRulesHandle = Arc<RwLock<Arc<capsem_core::net::policy_config::SecurityRuleSet>>>;
pub(crate) type PluginPolicyHandle = capsem_core::net::policy_config::SharedPluginPolicy;

/// Evaluate and record a host file operation independently of guest readiness.
pub(super) fn spawn(
    state: &Arc<capsem_core::SandboxNetworkState>,
    runtime: &Arc<McpRuntime>,
    output: &Sender<ProcessToService>,
    request: ServiceToProcess,
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
    let (db, rules, plugins, output) = (
        Arc::clone(&state.db),
        Arc::clone(&runtime.security_rules),
        Arc::clone(&runtime.plugin_policy),
        output.clone(),
    );
    tokio::spawn(async move {
        let boundary = emit_explicit_file_security_event(
            &db,
            &rules,
            &plugins,
            FileSecurityBoundary {
                action: match action {
                    FileBoundaryAction::Import => capsem_logger::FileAction::Imported,
                    FileBoundaryAction::Export => capsem_logger::FileAction::Exported,
                },
                path,
                size: Some(size),
                content: Some(file_content_preview(&data)),
                mime_type,
            },
        );
        let result = tokio::time::timeout(std::time::Duration::from_secs(30), boundary).await;
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
    pub(crate) action: capsem_logger::FileAction,
    pub(crate) path: String,
    pub(crate) size: Option<u64>,
    pub(crate) content: Option<String>,
    pub(crate) mime_type: Option<String>,
}

pub(crate) fn file_content_preview(data: &[u8]) -> String {
    String::from_utf8_lossy(&data[..data.len().min(FILE_SECURITY_CONTENT_PREVIEW_MAX)]).into_owned()
}

pub(crate) async fn emit_explicit_file_security_event(
    db: &Arc<capsem_logger::DbWriter>,
    security_rules: &SecurityRulesHandle,
    plugin_policy: &PluginPolicyHandle,
    boundary: FileSecurityBoundary,
) -> Result<Option<capsem_core::security_engine::SecurityRuleEmission>, String> {
    let rules = security_rules.read().unwrap().clone();
    let plugins = plugin_policy.read().unwrap().clone();
    capsem_core::security_engine::emit_explicit_file_security_write_and_rules_with_plugins(
        db,
        &rules,
        plugins,
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
    .await
}

pub(crate) fn rewritten_file_content(
    original_preview: &[u8],
    original_size: u64,
    event: &capsem_core::security_engine::SecurityEvent,
) -> Option<Vec<u8>> {
    if original_preview.len() as u64 != original_size {
        return None;
    }
    let mutating_rewrite = event.plugin_executions.iter().any(|execution| {
        execution.applied
            && !matches!(
                execution.stage,
                capsem_core::security_engine::SecurityPluginStage::Logging
            )
            && event.detections.iter().any(|detection| {
                detection.plugin_id.as_deref() == Some(execution.plugin_id.as_str())
                    && detection.plugin_mode == Some(capsem_core::net::policy_config::SecurityPluginMode::Rewrite)
            })
    });
    if !mutating_rewrite {
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

#[cfg(test)]
mod tests;
