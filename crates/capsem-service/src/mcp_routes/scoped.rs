//! Live MCP inventory read from one authenticated VM owner.
use crate::caller_auth::AuthenticatedActor;
use crate::owner_connection::OwnerConnection;
use crate::vm_files::ipc_command::{send_owner_command_guarded, IpcCommandError};
use crate::{AppError, ServiceState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use capsem_api::{AgentOperation, VmMcpServer, VmMcpServerKind, VmMcpServersResponse, VmMcpTool, VmMcpToolsResponse};
use capsem_proto::ipc::{McpServerStatus, ProcessToService, ServiceToProcess};
use capsem_proto::mcp_contracts::{McpToolDef, BUILTIN_SERVER_NAME};
use std::sync::Arc;

pub(crate) async fn list_servers(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<VmMcpServersResponse>, AppError> {
    let owner = actor.session_owner(&state, &id, AgentOperation::Inspect)?;
    let statuses = read_servers(&actor, &state, &owner).await?;
    let mut servers: Vec<_> = statuses
        .into_iter()
        .map(|server| {
            let kind = if server.name == BUILTIN_SERVER_NAME {
                VmMcpServerKind::Builtin
            } else if server.is_stdio {
                VmMcpServerKind::Sandbox
            } else {
                VmMcpServerKind::Remote
            };
            VmMcpServer {
                name: server.name,
                kind,
                enabled: server.enabled,
                connected: server.connected,
                tool_count: server.tool_count,
            }
        })
        .collect();
    servers.sort_unstable_by(|left, right| left.name.cmp(&right.name));
    Ok(Json(VmMcpServersResponse {
        vm_id: id,
        owner_generation: owner.generation().to_string(),
        servers,
    }))
}

pub(crate) async fn list_tools(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path((id, server_id)): Path<(String, String)>,
) -> Result<Json<VmMcpToolsResponse>, AppError> {
    let owner = actor.session_owner(&state, &id, AgentOperation::Inspect)?;
    let servers = read_servers(&actor, &state, &owner).await?;
    if !servers.iter().any(|server| server.name == server_id) {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            "MCP server not configured for this sandbox".into(),
        ));
    }
    let command = ServiceToProcess::McpListTools {
        id: state.next_job_id(),
    };
    let ProcessToService::McpToolsResult { tools, .. } = inspect(&actor, &state, &owner, command).await? else {
        return Err(unavailable());
    };
    let mut tools: Vec<_> = tools
        .into_iter()
        .filter(|tool| tool.server_name == server_id)
        .map(|tool| {
            let definition = McpToolDef::from(tool);
            let definition_digest = capsem_core::mcp::compute_tool_hash(&definition);
            VmMcpTool {
                namespaced_name: definition.namespaced_name,
                original_name: definition.original_name,
                description: definition.description,
                input_schema: definition.input_schema,
                annotations: definition.annotations.map(|annotations| annotations.to_mcp_json()),
                definition_digest,
            }
        })
        .collect();
    tools.sort_unstable_by(|left, right| left.namespaced_name.cmp(&right.namespaced_name));
    Ok(Json(VmMcpToolsResponse {
        vm_id: id,
        owner_generation: owner.generation().to_string(),
        server_id,
        tools,
    }))
}

async fn read_servers(
    actor: &AuthenticatedActor,
    state: &ServiceState,
    owner: &OwnerConnection,
) -> Result<Vec<McpServerStatus>, AppError> {
    let command = ServiceToProcess::McpListServers {
        id: state.next_job_id(),
    };
    match inspect(actor, state, owner, command).await? {
        ProcessToService::McpServersResult { servers, .. } => Ok(servers),
        _ => Err(unavailable()),
    }
}

async fn inspect(
    actor: &AuthenticatedActor,
    state: &ServiceState,
    owner: &OwnerConnection,
    command: ServiceToProcess,
) -> Result<ProcessToService, AppError> {
    let result = send_owner_command_guarded(state, owner, command, Some(10), || {
        actor
            .check_session(state, owner, AgentOperation::Inspect)
            .map_err(|_| IpcCommandError::AccessDenied)
    })
    .await;
    actor.check_session(state, owner, AgentOperation::Inspect)?;
    owner.validate(state, false).map_err(|_| {
        AppError::new(
            StatusCode::CONFLICT,
            "sandbox generation changed during MCP inspection".into(),
        )
    })?;
    result.map_err(|error| match error {
        IpcCommandError::AccessDenied => crate::caller_auth::session::denied(),
        IpcCommandError::Timeout { .. } => {
            AppError::new(StatusCode::GATEWAY_TIMEOUT, "sandbox MCP inventory timed out".into())
        }
        IpcCommandError::Failed(_) => unavailable(),
    })
}

fn unavailable() -> AppError {
    AppError::new(StatusCode::BAD_GATEWAY, "sandbox MCP inventory is unavailable".into())
}
