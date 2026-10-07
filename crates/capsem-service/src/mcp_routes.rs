//! MCP servers and tool permissions, from the user's settings.toml `[mcp]`
//! with the corp config laid over it.
use super::*;

use capsem_core::mcp::policy::McpConfig;
use capsem_core::net::policy_config::{mcp_default_permission, mcp_server_configured, mcp_tool_permission};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct McpPermissionEditRequest {
    pub action: SecurityRuleAction,
}

/// The settings and corp files the MCP routes answer from. A settings.toml
/// that does not load is an error here, never an empty configuration.
fn policy_files() -> Result<(SettingsFile, SettingsFile), AppError> {
    capsem_core::net::policy_config::load_policy_files().map_err(|error| AppError(StatusCode::BAD_REQUEST, error))
}

fn merged_mcp(settings: &SettingsFile, corp: &SettingsFile) -> McpConfig {
    McpConfig::merged(settings.mcp.as_ref(), corp.mcp.as_ref()).unwrap_or_default()
}

fn ensure_server_configured(mcp: &McpConfig, server_id: &str) -> Result<(), AppError> {
    if server_id.is_empty() {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "MCP server id must not be empty".to_string(),
        ));
    }
    if mcp_server_configured(Some(mcp), server_id) {
        Ok(())
    } else {
        Err(AppError(
            StatusCode::NOT_FOUND,
            format!("MCP server not configured: {server_id}"),
        ))
    }
}

pub(super) fn resolve_mcp_tool_id(server_id: &str, tool_id: &str) -> Result<String, AppError> {
    if server_id.is_empty() || tool_id.is_empty() {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "server id and tool id must not be empty".to_string(),
        ));
    }
    if let Some((prefix, _)) = tool_id.split_once("__") {
        if prefix != server_id {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                format!("tool id {tool_id} does not belong to MCP server {server_id}"),
            ));
        }
        Ok(tool_id.to_string())
    } else {
        Ok(format!("{server_id}__{tool_id}"))
    }
}

/// GET /mcp/info -- how many MCP servers a session runs.
pub(super) async fn handle_mcp_info(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<api::McpInfoResponse>, AppError> {
    let (settings, corp) = state.off_worker(|_| policy_files()).await??;
    let mcp = merged_mcp(&settings, &corp);
    let builtin_local_enabled = mcp_server_configured(Some(&mcp), capsem_core::mcp::BUILTIN_SERVER_NAME);
    Ok(Json(api::McpInfoResponse {
        server_count: mcp.servers.len() + usize::from(builtin_local_enabled),
        manual_server_count: mcp.servers.len(),
        builtin_local_enabled,
    }))
}

/// GET /mcp/servers/list -- the configured MCP servers with their tool counts.
pub(super) async fn handle_mcp_servers(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<api::McpServersListResponse>, AppError> {
    let (settings, corp) = state.off_worker(|_| policy_files()).await??;
    let mcp = merged_mcp(&settings, &corp);
    let builtin_bin = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|dir| dir.join("capsem-mcp-builtin")));
    let servers = capsem_core::mcp::build_server_list(&mcp, builtin_bin.as_deref(), HashMap::new());
    let cache = latest_mcp_tool_cache(&state);
    Ok(Json(api::McpServersListResponse(
        servers
            .iter()
            .map(|server| api::McpServerInfoResponse {
                name: server.name.clone(),
                url: server.url.clone(),
                has_auth_credential: server.auth.is_some(),
                custom_header_count: server.headers.len(),
                source: server.source.clone(),
                enabled: server.enabled,
                running: false, // Config-level only; runtime status requires IPC.
                tool_count: cache.iter().filter(|tool| tool.server_name == server.name).count(),
                is_stdio: server.is_stdio(),
            })
            .collect(),
    )))
}

/// GET /mcp/default/info -- the effective default MCP permission.
pub(super) async fn handle_mcp_default_info(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<api::McpDefaultPermissionResponse>, AppError> {
    let (settings, corp) = state.off_worker(|_| policy_files()).await??;
    let permission =
        mcp_default_permission(&settings, &corp).map_err(|error| AppError(StatusCode::BAD_REQUEST, error))?;
    Ok(Json(api::McpDefaultPermissionResponse {
        action: api::mcp_permission_action(permission.action),
        source: permission.source,
        rule_id: permission.rule_id,
    }))
}

pub(super) fn latest_mcp_tool_cache(state: &ServiceState) -> Vec<ToolCacheEntry> {
    let latest = capsem_core::mcp::load_tool_cache();
    let Ok(mut cache) = state.mcp_tool_cache.lock() else {
        return latest;
    };
    if !latest.is_empty() || cache.is_empty() {
        *cache = latest;
    }
    cache.clone()
}

/// GET /mcp/servers/{server_id}/tools/list -- one server's discovered tools
/// with their effective permissions.
pub(super) async fn handle_mcp_server_tools(
    State(state): State<Arc<ServiceState>>,
    Path(server_id): Path<String>,
) -> Result<Json<api::McpToolsListResponse>, AppError> {
    let (settings, corp) = state.off_worker(|_| policy_files()).await??;
    ensure_server_configured(&merged_mcp(&settings, &corp), &server_id)?;
    let tools = latest_mcp_tool_cache(&state)
        .iter()
        .filter(|entry| entry.server_name == server_id)
        .map(|entry| {
            let permission =
                mcp_tool_permission(&settings, &corp, &server_id, &entry.original_name).map_err(|error| {
                    AppError(
                        StatusCode::BAD_REQUEST,
                        format!(
                            "resolve MCP tool permission {}/{}: {error}",
                            server_id, entry.original_name
                        ),
                    )
                })?;
            Ok(api::McpToolInfoResponse {
                namespaced_name: entry.namespaced_name.clone(),
                original_name: entry.original_name.clone(),
                description: entry.description.clone(),
                server_name: entry.server_name.clone(),
                annotations: entry.annotations.as_ref().map(|a| a.to_mcp_json()),
                pin_hash: Some(entry.pin_hash.clone()),
                pin_changed: false, // Would need live catalog comparison.
                permission_action: api::mcp_permission_action(permission.action),
                permission_source: permission.source,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    Ok(Json(api::McpToolsListResponse(tools)))
}

/// POST /mcp/servers/{server_id}/refresh -- rediscover one server's tools in
/// every running VM.
pub(super) async fn handle_mcp_server_refresh(
    State(state): State<Arc<ServiceState>>,
    Path(server_id): Path<String>,
) -> Result<Json<api::McpRefreshResponse>, AppError> {
    let (settings, corp) = state.off_worker(|_| policy_files()).await??;
    ensure_server_configured(&merged_mcp(&settings, &corp), &server_id)?;
    let targets = {
        let instances = state.instances.lock().unwrap();
        instances
            .values()
            .map(|info| (info.id.clone(), info.uds_path.clone()))
            .collect::<Vec<_>>()
    };
    let mut refreshed = 0;
    for (vm_id, uds_path) in &targets {
        let id = state.next_job_id();
        match send_ipc_command(&state, uds_path, ServiceToProcess::McpRefreshTools { id }, Some(30)).await {
            Ok(_) => refreshed += 1,
            Err(error) => warn!(vm_id = %vm_id, server_id = %server_id, %error, "MCP tool refresh failed"),
        }
    }
    if let Ok(mut cache) = state.mcp_tool_cache.lock() {
        *cache = capsem_core::mcp::load_tool_cache();
    }
    Ok(Json(api::McpRefreshResponse {
        success: refreshed == targets.len(),
        server_id,
        instances: refreshed,
    }))
}

/// PATCH /mcp/default/edit -- set the default MCP permission in settings.toml.
pub(super) async fn handle_mcp_default_edit(
    State(state): State<Arc<ServiceState>>,
    Json(update): Json<McpPermissionEditRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let route = MutationRoute {
        name: "mcp_default_edit",
        target_kind: "mcp_default",
        target_key: "default.mcp",
        operation: "permission",
    };
    let action = update.action;
    // MCP permissions compile to enforcement rules: enforced before returning.
    let event = apply_policy_mutation(&state, route, move |edit, corp| {
        edit.set_mcp_default_permission(corp, action, "service-api")
    })
    .await?;
    Ok(Json(json!({ "action": update.action, "mutation": event })))
}

/// PATCH /mcp/servers/{server_id}/tools/{tool_id}/edit -- allow, ask or
/// block one tool in settings.toml.
pub(super) async fn handle_mcp_tool_edit(
    State(state): State<Arc<ServiceState>>,
    Path((server_id, tool_id)): Path<(String, String)>,
    Json(update): Json<McpPermissionEditRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let target_key = format!("{server_id}/{tool_id}");
    let route = MutationRoute {
        name: "mcp_tool_edit",
        target_kind: "mcp_tool",
        target_key: &target_key,
        operation: "permission",
    };
    let (server, tool, action) = (server_id.clone(), tool_id.clone(), update.action);
    let event = apply_policy_mutation(&state, route, move |edit, corp| {
        edit.set_mcp_tool_permission(corp, &server, &tool, action, "service-api")
    })
    .await?;
    Ok(Json(json!({
        "server_id": server_id,
        "tool_id": tool_id,
        "action": update.action,
        "mutation": event,
    })))
}

/// POST /mcp/servers/{server_id}/tools/{tool_id}/call -- call a tool through
/// a running VM's aggregator, under that VM's policy.
pub(super) async fn handle_mcp_tool_call(
    State(state): State<Arc<ServiceState>>,
    Path((server_id, tool_id)): Path<(String, String)>,
    Json(arguments): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (settings, corp) = state.off_worker(|_| policy_files()).await??;
    ensure_server_configured(&merged_mcp(&settings, &corp), &server_id)?;
    let namespaced_name = resolve_mcp_tool_id(&server_id, &tool_id)?;
    let uds_path = state
        .instances
        .lock()
        .unwrap()
        .values()
        .next()
        .map(|info| info.uds_path.clone())
        .ok_or_else(|| AppError(StatusCode::SERVICE_UNAVAILABLE, "no running sessions".into()))?;

    let arguments_json = serde_json::to_string(&arguments)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("invalid arguments: {e}")))?;
    let msg = ServiceToProcess::McpCallTool {
        id: state.next_job_id(),
        namespaced_name,
        arguments_json,
    };
    match send_ipc_command(&state, &uds_path, msg, Some(60))
        .await
        .map_err(|e| AppError(StatusCode::BAD_GATEWAY, e))?
    {
        ProcessToService::McpCallToolResult { error: Some(err), .. } => Err(AppError(StatusCode::BAD_GATEWAY, err)),
        ProcessToService::McpCallToolResult { result_json, .. } => match result_json {
            Some(s) => serde_json::from_str(&s).map(Json).map_err(|e| {
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("bad result_json from process: {e}"),
                )
            }),
            None => Ok(Json(serde_json::Value::Null)),
        },
        _ => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected IPC response".into(),
        )),
    }
}
