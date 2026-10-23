//! Trusted lifecycle maintenance after producers drain, before worker shutdown.

use super::*;

pub(super) async fn apply(worker: &LedgerWorker, database: &Path, days: u64) -> Result<()> {
    let Some(cutoff) = std::time::SystemTime::now().checked_sub(Duration::from_secs(days.saturating_mul(86_400)))
    else {
        tracing::warn!(days, "retention period predates the epoch; keeping every body");
        return Ok(());
    };
    let cutoff = capsem_logger::format_ledger_timestamp(cutoff);
    let (stream, grant) = worker.connect(LedgerClientRole::Maintainer).await?.into_parts();
    let client = capsem_logger::ledger_client::LedgerClient::connect(stream, grant, database.to_path_buf())
        .await
        .map_err(anyhow::Error::msg)?;
    let retained = client.retain_bodies_since(&cutoff).await.map_err(anyhow::Error::msg)?;
    tracing::debug!(
        cutoff,
        blocks_dropped = retained.blocks_dropped,
        rows_dropped = retained.rows_dropped,
        bytes_reclaimed = retained.bytes_reclaimed,
        "session archive retention complete"
    );
    Ok(())
}
