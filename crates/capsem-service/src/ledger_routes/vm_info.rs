//! The combined VM overview: lifecycle, ledger readiness and activity totals.
use super::*;

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
    let counters = activity::read_counters(state, &info.id, "info", &db_path).await?;
    info.ai = Some(activity::ai_info(&counters));
    info.network = Some(activity::network_info(&counters));
    info.files = Some(
        activity::files_info(&counters)
            .map_err(|error| ledger_route_error(&info.id, "info", "files", &db_path, error))?,
    );
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
