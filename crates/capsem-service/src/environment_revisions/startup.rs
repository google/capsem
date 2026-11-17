//! Startup walks bounded DB-owned cursor pages, retaining failures in the ledger.
use super::*;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RecoverySummary {
    pub committed: u64,
    pub recovered_unapplied: u64,
    pub receipts_bound: u64,
    pub failed: u64,
}

impl EnvironmentRevisions {
    pub fn reconcile_pending(self: &Arc<Self>, at: i64) -> RevisionJob<RecoverySummary> {
        let owner = Arc::clone(self);
        let (sender, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let result = async {
                let limit = u16::try_from(capsem_logger::ledger_protocol::MAX_LEDGER_QUERY_ROWS)
                    .map_err(|_| EnvironmentFileError::LedgerFailure)?;
                owner.recover_pending_owned(at, limit).await
            }
            .await;
            drop(owner);
            let _ = sender.send(result);
        });
        RevisionJob { receiver }
    }
    pub(super) async fn recover_pending_owned(
        self: &Arc<Self>,
        at: i64,
        limit: u16,
    ) -> Result<RecoverySummary, EnvironmentFileError> {
        if at < 0 {
            return Err(EnvironmentFileError::InvalidDocument);
        }
        let mut summary = RecoverySummary::default();
        let mut after_id = 0;
        loop {
            let page = self
                .ledger
                .environment_revisions(EnvironmentRevisionQuery::Page {
                    environment_id: None,
                    state: Some(EnvironmentRevisionState::Committing),
                    after_id,
                    limit,
                })
                .await
                .map_err(|_| EnvironmentFileError::LedgerFailure)?;
            let Some(last) = page.last() else {
                break;
            };
            after_id = last.cursor;
            for record in page {
                match self.reconcile(record.environment_id.clone(), at).wait().await {
                    Ok(Some(recovery)) => match recovery.state {
                        EnvironmentRevisionState::Committed => summary.committed += 1,
                        EnvironmentRevisionState::RecoveredUnapplied => summary.recovered_unapplied += 1,
                        EnvironmentRevisionState::Committing => return Err(EnvironmentFileError::Unverified),
                    },
                    Ok(None) => {}
                    Err(error) => {
                        summary.failed += 1;
                        tracing::warn!(target:"capsem.environment",environment_id=%record.environment_id,error=%error,"environment recovery remains unresolved");
                    }
                }
            }
        }
        self.recover_request_receipts(at, limit, &mut summary).await?;
        Ok(summary)
    }
}
