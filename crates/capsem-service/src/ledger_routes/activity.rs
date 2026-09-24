//! The activity totals routes report, read from the ledger's counter snapshot.
//!
//! The session's writer keeps these counts and persists them beside the rows
//! they count; a route takes them with one cached primary-key lookup. They
//! used to be aggregated from the tables on every poll, which made the polled
//! routes cost as much as the ledger was long.

use capsem_logger::counters::{usd_from_micro, LedgerCounters};

use super::*;

/// The session's counter snapshot, through its ready DB handle.
pub(crate) async fn read_counters(
    state: &ServiceState,
    vm_id: &str,
    ledger: &str,
    db_path: &StdPath,
) -> Result<LedgerCounters, AppError> {
    let db = open_ready_session_db(state, vm_id, ledger, db_path).await?;
    db.ledger_counters()
        .await
        .map_err(|error| ledger_route_error(vm_id, ledger, "counters", db_path, error))
}

fn thinking_tokens(counters: &LedgerCounters) -> u64 {
    counters
        .model
        .usage_details
        .get("thinking")
        .copied()
        .unwrap_or_default()
}

pub(crate) fn stats_summary(counters: &LedgerCounters) -> api::VmStatsSummaryResponse {
    api::VmStatsSummaryResponse {
        total_requests: counters.net.total,
        allowed_requests: counters.net.allowed,
        denied_requests: counters.net.denied,
        total_input_tokens: counters.model.total.input_tokens,
        total_thinking_tokens: thinking_tokens(counters),
        total_output_tokens: counters.model.total.output_tokens,
        total_tool_calls: counters.tools.calls,
        total_estimated_cost: usd_from_micro(counters.model.total.cost_micro_usd),
    }
}

/// Model usage, busiest first.
pub(crate) fn model_usage(counters: &LedgerCounters) -> Vec<capsem_api::ModelUsage> {
    let mut models: Vec<capsem_api::ModelUsage> = counters
        .model
        .by_model
        .iter()
        .flat_map(|(provider, models)| {
            models.iter().map(move |(model, usage)| capsem_api::ModelUsage {
                provider: provider.clone(),
                model: model.clone(),
                call_count: usage.calls,
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                estimated_cost_usd: usd_from_micro(usage.cost_micro_usd),
                duration_ms: usage.duration_ms,
            })
        })
        .collect();
    models.sort_by(|left, right| {
        right
            .call_count
            .cmp(&left.call_count)
            .then_with(|| left.provider.cmp(&right.provider))
            .then_with(|| left.model.cmp(&right.model))
    });
    models
}

/// MCP tool usage, busiest first.
fn mcp_usage(counters: &LedgerCounters) -> Vec<capsem_api::McpUsage> {
    let mut tools: Vec<capsem_api::McpUsage> = counters
        .tools
        .mcp
        .iter()
        .flat_map(|(server, tools)| {
            tools.iter().map(move |(tool, usage)| capsem_api::McpUsage {
                server_name: Some(server.clone()),
                tool_name: tool.clone(),
                call_count: usage.calls,
                duration_ms: usage.duration_ms,
                bytes_sent: usage.bytes_sent,
                bytes_received: usage.bytes_received,
            })
        })
        .collect();
    tools.sort_by(|left, right| {
        right
            .call_count
            .cmp(&left.call_count)
            .then_with(|| left.server_name.cmp(&right.server_name))
            .then_with(|| left.tool_name.cmp(&right.tool_name))
    });
    tools
}

pub(crate) fn ai_info(counters: &LedgerCounters) -> capsem_api::VmAiInfo {
    capsem_api::VmAiInfo {
        model_call_count: counters.model.total.calls,
        total_input_tokens: counters.model.total.input_tokens,
        total_thinking_tokens: thinking_tokens(counters),
        total_output_tokens: counters.model.total.output_tokens,
        total_tool_calls: counters.tools.calls,
        total_estimated_cost_usd: usd_from_micro(counters.model.total.cost_micro_usd),
        models: model_usage(counters),
        mcp: mcp_usage(counters),
    }
}

pub(crate) fn network_info(counters: &LedgerCounters) -> capsem_api::VmNetworkInfo {
    capsem_api::VmNetworkInfo {
        total_requests: counters.net.total,
        allowed_requests: counters.net.allowed,
        denied_requests: counters.net.denied,
        errors: counters.net.error,
        bytes_sent: counters.net.bytes_sent,
        bytes_received: counters.net.bytes_received,
    }
}

/// File activity by action.
///
/// An overflow marker is not a file action -- it records that some changes
/// went unrecorded -- so it is neither listed nor counted. Listing it used to
/// fail the whole `/info` route, because the API has no action to name it.
/// Any other action the API cannot name is a broken snapshot, and fails loudly
/// rather than disappearing from the totals.
pub(crate) fn files_info(counters: &LedgerCounters) -> Result<capsem_api::VmFilesInfo, String> {
    let overflow = capsem_logger::FileAction::Overflow.as_str();
    let actions = counters
        .files
        .by_action
        .iter()
        .filter(|(action, _)| action.as_str() != overflow)
        .map(|(action, count)| {
            serde_json::from_value(serde_json::Value::String(action.clone()))
                .map(|action| capsem_api::FileActionCount { action, count: *count })
                .map_err(|error| format!("unknown file action {action:?} in the counter snapshot: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(capsem_api::VmFilesInfo {
        total_events: actions.iter().map(|action| action.count).sum(),
        actions,
    })
}
