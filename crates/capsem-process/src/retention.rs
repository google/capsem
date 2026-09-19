//! Trimming a session's archived bodies to the retention period, at stop.
//!
//! `capsem-process` owns every write to its session ledger, so this is the
//! only process that may rewrite it. The service holds external, disk-only
//! readers; it asks for retention by passing `--retention-days` at spawn and
//! never touches the archive itself.
//!
//! Nothing here opens the archive file: the logger owns it, and this asks the
//! writer that already has it open. That is what makes "one writer per
//! session ledger" true rather than merely intended.

use capsem_logger::DbWriter;
use tracing::{info, warn};

/// Drop archived bodies older than the retention period the user set.
///
/// A persistent VM's session directory survives every stop, so without this
/// the archive only ever grows: every request and response body of
/// every session it has ever run, back to the day it was created.
pub(crate) async fn retain_session_bodies(db: &DbWriter, retention_days: u64) {
    let Some(cutoff) =
        std::time::SystemTime::now().checked_sub(std::time::Duration::from_secs(retention_days.saturating_mul(86_400)))
    else {
        warn!(
            retention_days,
            "retention period predates the epoch; keeping every body"
        );
        return;
    };
    let cutoff = capsem_logger::format_ledger_timestamp(cutoff);
    match db.retain_bodies_since(&cutoff).await {
        Ok(retained) if retained.blocks_dropped == 0 => {
            tracing::debug!(cutoff, "no archived bodies were old enough to drop");
        }
        Ok(retained) => info!(
            cutoff,
            blocks_dropped = retained.blocks_dropped,
            rows_dropped = retained.rows_dropped,
            bytes_reclaimed = retained.bytes_reclaimed,
            "trimmed archived bodies past the retention period"
        ),
        // Never fatal: a session that could not be trimmed is a session that
        // kept too much, and refusing to shut down over it would lose more.
        Err(error) => warn!(cutoff, error = %error, "could not trim archived bodies"),
    }
}

#[cfg(test)]
mod tests;
