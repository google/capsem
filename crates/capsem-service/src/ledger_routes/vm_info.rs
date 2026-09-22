//! Query intent for the combined VM overview. Execution stays in DbHandle.
use super::*;

const MCP_USAGE_SQL: &str = "SELECT server_name, tool_name, COUNT(*) AS call_count,
    COALESCE(SUM(duration_ms), 0) AS duration_ms,
    COALESCE(SUM(bytes_sent), 0) AS bytes_sent,
    COALESCE(SUM(bytes_received), 0) AS bytes_received
    FROM tool_calls WHERE origin IN ('mcp', 'mcp_proxy')
    GROUP BY server_name, tool_name ORDER BY call_count DESC, server_name, tool_name";

const FILE_ACTIONS_SQL: &str = "SELECT action, COUNT(*) AS count FROM fs_events GROUP BY action ORDER BY action";

pub(crate) async fn populate_vm_info(
    state: &ServiceState,
    info: &mut SandboxInfo,
    session_dir: &StdPath,
) -> Result<(), AppError> {
    apply_session_db_status(state, info, session_dir).await;
    if !info.session_db.as_ref().is_some_and(|status| status.ready) {
        // Keep lifecycle and explicit readiness errors usable even after a
        // failed boot. Never turn an unavailable ledger into zero activity.
        return Ok(());
    }
    let db_path = session_db_path_for_session_dir(session_dir);
    let db = open_ready_session_db(state, &info.id, "info", &db_path).await?;
    let stats = db
        .session_stats()
        .await
        .map_err(|error| ledger_route_error(&info.id, "info", "summary", &db_path, error))?;
    let models = query_route_typed_rows::<capsem_api::ModelUsage>(
        &info.id,
        "info",
        "models",
        &db_path,
        &db,
        STATS_DETAIL_MODEL_STATS_SQL,
        &[],
    )
    .await?;
    let mcp =
        query_route_typed_rows::<capsem_api::McpUsage>(&info.id, "info", "mcp", &db_path, &db, MCP_USAGE_SQL, &[])
            .await?;
    let actions = query_route_typed_rows::<capsem_api::FileActionCount>(
        &info.id,
        "info",
        "files",
        &db_path,
        &db,
        FILE_ACTIONS_SQL,
        &[],
    )
    .await?;
    info.ai = Some(capsem_api::VmAiInfo {
        model_call_count: stats.model_call_count,
        total_input_tokens: stats.total_input_tokens,
        total_thinking_tokens: stats.total_usage_details.get("thinking").copied().unwrap_or_default(),
        total_output_tokens: stats.total_output_tokens,
        total_tool_calls: stats.total_tool_calls,
        total_estimated_cost_usd: stats.total_estimated_cost_usd,
        models,
        mcp,
    });
    info.network = Some(capsem_api::VmNetworkInfo {
        total_requests: stats.net_total,
        allowed_requests: stats.net_allowed,
        denied_requests: stats.net_denied,
        errors: stats.net_error,
        bytes_sent: stats.net_bytes_sent,
        bytes_received: stats.net_bytes_received,
    });
    info.files = Some(capsem_api::VmFilesInfo {
        total_events: actions.iter().map(|action| action.count).sum(),
        actions,
    });
    Ok(())
}

async fn apply_session_db_status(state: &ServiceState, info: &mut SandboxInfo, session_dir: &StdPath) {
    let db_path = session_db_path_for_session_dir(session_dir);
    if !db_path.exists() {
        info.session_db = Some(api::SessionDbStatus {
            ready: false,
            error: Some("session.db absent".to_string()),
        });
        info!(
            vm_id = info.id.as_str(),
            operation = "session_db_status",
            db_path = %db_path.display(),
            ready = false,
            "session DB absent while building session status"
        );
        return;
    }
    match open_ready_session_db(state, &info.id, "session status", &db_path).await {
        Ok(_) => {
            info.session_db = Some(api::SessionDbStatus {
                ready: true,
                error: None,
            });
            info!(
                vm_id = info.id.as_str(),
                operation = "session_db_status",
                db_path = %db_path.display(),
                ready = true,
                "session DB ready for session status"
            );
        }
        Err(error) => {
            let message = error.1;
            info.session_db = Some(api::SessionDbStatus {
                ready: false,
                error: Some(message.clone()),
            });
            warn!(
                vm_id = info.id.as_str(),
                operation = "session_db_status",
                db_path = %db_path.display(),
                ready = false,
                error = %message,
                "session DB not ready for session status"
            );
        }
    }
}
