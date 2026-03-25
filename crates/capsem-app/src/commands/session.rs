use std::sync::Arc;

use capsem_logger::validate_select_only;
use serde::Serialize;
use tauri::State;

use crate::state::AppState;
use super::active_vm_id;

/// Response for get_session_info.
#[derive(Serialize)]
pub struct SessionInfoResponse {
    pub session_id: String,
    pub mode: String,
    pub uptime_ms: u64,
    pub scratch_disk_size_gb: u32,
    pub ram_bytes: u64,
    pub total_requests: u64,
    pub allowed_requests: u64,
    pub denied_requests: u64,
    pub error_requests: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub model_call_count: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_usage_details: std::collections::BTreeMap<String, u64>,
    pub total_tool_calls: u64,
    pub total_estimated_cost_usd: f64,
}

/// Returns info about the current active session.
#[tauri::command]
pub async fn get_session_info(state: State<'_, AppState>) -> Result<SessionInfoResponse, String> {
    let vm_id = active_vm_id(&state)?;

    // Get uptime from state machine.
    let uptime_ms = {
        let vms = state.vms.lock().unwrap();
        match vms.get(&vm_id) {
            Some(instance) => instance.state_machine.elapsed().as_millis() as u64,
            None => 0,
        }
    };

    // Get session record from index.
    let (mode, disk_gb, ram) = {
        let idx = state.session_index.lock().map_err(|e| format!("session index lock: {e}"))?;
        let records = idx.recent(50).map_err(|e| format!("session index query: {e}"))?;
        let record = records.iter().find(|r| r.id == vm_id);
        (
            record.map(|r| r.mode.clone()).unwrap_or_else(|| "gui".to_string()),
            record.map(|r| r.scratch_disk_size_gb).unwrap_or(16),
            record.map(|r| r.ram_bytes).unwrap_or(4 * 1024 * 1024 * 1024),
        )
    };

    // Get live stats from session DB via spawn_blocking.
    let db = {
        let vms = state.vms.lock().unwrap();
        vms.get(&vm_id)
            .and_then(|i| i.net_state.as_ref())
            .map(|ns| Arc::clone(&ns.db))
    };

    let stats = if let Some(db) = db {
        tokio::task::spawn_blocking(move || {
            db.reader()
                .ok()
                .and_then(|r| r.session_stats().ok())
        })
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?
    } else {
        None
    };

    let vm_id_out = vm_id;
    Ok(SessionInfoResponse {
        session_id: vm_id_out,
        mode,
        uptime_ms,
        scratch_disk_size_gb: disk_gb,
        ram_bytes: ram,
        total_requests: stats.as_ref().map(|s| s.net_total).unwrap_or(0),
        allowed_requests: stats.as_ref().map(|s| s.net_allowed).unwrap_or(0),
        denied_requests: stats.as_ref().map(|s| s.net_denied).unwrap_or(0),
        error_requests: stats.as_ref().map(|s| s.net_error).unwrap_or(0),
        bytes_sent: stats.as_ref().map(|s| s.net_bytes_sent).unwrap_or(0),
        bytes_received: stats.as_ref().map(|s| s.net_bytes_received).unwrap_or(0),
        model_call_count: stats.as_ref().map(|s| s.model_call_count).unwrap_or(0),
        total_input_tokens: stats.as_ref().map(|s| s.total_input_tokens).unwrap_or(0),
        total_output_tokens: stats.as_ref().map(|s| s.total_output_tokens).unwrap_or(0),
        total_usage_details: stats.as_ref().map(|s| s.total_usage_details.clone()).unwrap_or_default(),
        total_tool_calls: stats.as_ref().map(|s| s.total_tool_calls).unwrap_or(0),
        total_estimated_cost_usd: stats.as_ref().map(|s| s.total_estimated_cost_usd).unwrap_or(0.0),
    })
}

/// Execute a raw SELECT query against the session DB or main.db.
/// Returns a JSON string: `{"columns":[...],"rows":[[...],...]}`
///
/// - `db`: `"session"` (default) or `"main"` -- which database to query
/// - `params`: optional bind parameter values (`?` positional placeholders)
#[tauri::command]
pub async fn query_db(
    sql: String,
    db: Option<String>,
    params: Option<Vec<serde_json::Value>>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    let params = params.unwrap_or_default();
    let target = db.unwrap_or_else(|| "session".to_string());

    match target.as_str() {
        "main" => {
            tokio::task::spawn_blocking(move || {
                use tauri::Manager;
                validate_select_only(&sql)?;
                let state = app_handle.state::<AppState>();
                let idx = state.session_index.lock().map_err(|e| format!("lock: {e}"))?;
                idx.query_raw(&sql, &params)
            })
            .await
            .map_err(|e| format!("spawn_blocking failed: {e}"))?
        }
        _ => {
            let vm_id = active_vm_id(&state)?;
            let db_writer = {
                let vms = state.vms.lock().unwrap();
                let instance = vms.get(&vm_id).ok_or("no VM running")?;
                let net_state = instance.net_state.as_ref().ok_or("network not initialized")?;
                Arc::clone(&net_state.db)
            };

            tokio::task::spawn_blocking(move || {
                validate_select_only(&sql)?;
                let reader = db_writer.reader().map_err(|e| format!("db reader: {e}"))?;
                if params.is_empty() {
                    reader.query_raw(&sql)
                } else {
                    reader.query_raw_with_params(&sql, &params)
                }
            })
            .await
            .map_err(|e| format!("spawn_blocking failed: {e}"))?
        }
    }
}
