//! The combined VM overview: lifecycle, ledger readiness and activity totals.
use super::*;

/// Fill `info`'s ledger readiness and activity totals.
///
/// `/info` is polled, so both come from the session's ledger handle's memory:
/// the counter snapshot its reader keeps current, which is also its answer to
/// whether the ledger is ready. Serving a poll opens nothing and reads nothing
/// from the file. It used to ready the handle twice and read the snapshot
/// back from SQLite on every poll.
pub(crate) async fn populate_vm_info(
    state: &ServiceState,
    info: &mut SandboxInfo,
    session_dir: &StdPath,
) -> Result<(), AppError> {
    let db_path = session_db_path_for_session_dir(session_dir);
    let Some(counters) = session_counters(state, info, &db_path).await else {
        // Keep lifecycle and explicit readiness errors usable even after a
        // failed boot. Never turn an unavailable ledger into zero activity.
        return Ok(());
    };
    activity::apply_totals(info, &counters);
    info.ai = Some(activity::ai_info(&counters));
    info.network = Some(activity::network_info(&counters));
    info.files = Some(
        activity::files_info(&counters)
            .map_err(|error| ledger_route_error(&info.id, "info", "files", &db_path, error))?,
    );
    Ok(())
}

/// Record whether the session's ledger is ready, and hand back its counter
/// snapshot when it is.
async fn session_counters(
    state: &ServiceState,
    info: &mut SandboxInfo,
    db_path: &StdPath,
) -> Option<Arc<capsem_logger::counters::LedgerCounters>> {
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
        return None;
    }
    let counters = match session_db(state, &info.id, "session status", db_path).await {
        Ok(db) => db.ledger_counters().await,
        Err(error) => Err(error.1),
    };
    match counters {
        Ok(counters) => {
            info.session_db = Some(api::SessionDbStatus {
                ready: true,
                error: None,
            });
            Some(counters)
        }
        Err(message) => {
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
            None
        }
    }
}
