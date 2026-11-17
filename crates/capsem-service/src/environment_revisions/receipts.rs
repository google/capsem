//! A receipt records an already approved and committed effect. Repair never
//! publishes a file, creates a revision, or applies configuration to a sandbox.
use super::*;
use capsem_logger::{
    db::EnvironmentRequestQuery,
    events::environment::request::{EnvironmentRequestKind, EnvironmentRequestResult, EnvironmentRequestState},
};

impl EnvironmentRevisions {
    pub(super) async fn completed_record(
        &self,
        id: &str,
        revision: &str,
        at: i64,
    ) -> Result<EnvironmentRevisionRecord, EnvironmentFileError> {
        let record = self.required_record(id, revision).await?;
        self.bind_request_receipt(&record, at).await?;
        Ok(record)
    }

    async fn bind_request_receipt(
        &self,
        record: &EnvironmentRevisionRecord,
        at: i64,
    ) -> Result<(), EnvironmentFileError> {
        let Some(request_id) = &record.request_id else {
            return Ok(());
        };
        if record.state != EnvironmentRevisionState::Committed || !record.replayable || at < record.timestamp_unix_ms {
            return Err(EnvironmentFileError::Unverified);
        }
        // Logger verifies the persisted operator approval, exact request/result
        // correlation and fork lineage. This is a receipt for that authority,
        // not a new approval or an operator impersonation from a request DTO.
        self.write(WriteOp::EnvironmentRequestResult(EnvironmentRequestResult {
            request_id: request_id.clone(),
            result: EnvironmentRevisionParent {
                environment_id: record.environment_id.clone(),
                revision_id: record.revision_id.clone(),
            },
            timestamp_unix_ms: at,
            actor: "operator".into(),
        }))
        .await
    }

    pub(super) async fn recover_request_receipts(
        &self,
        at: i64,
        limit: u16,
        summary: &mut RecoverySummary,
    ) -> Result<(), EnvironmentFileError> {
        let mut after_id = 0;
        loop {
            // Startup is an explicit bounded history walk, never a polled route
            // or a service projection. Only metadata is retained for one page.
            let page = self
                .ledger
                .environment_requests(EnvironmentRequestQuery::Page {
                    environment_id: None,
                    requester: None,
                    pending_only: false,
                    after_id,
                    limit,
                })
                .await
                .map_err(|_| EnvironmentFileError::LedgerFailure)?;
            let Some(last) = page.last() else { break };
            after_id = last.cursor;
            for request in page {
                if request.state != EnvironmentRequestState::Approved
                    || request.result.is_some()
                    || request.kind == EnvironmentRequestKind::Apply
                {
                    continue;
                }
                let records = self
                    .ledger
                    .environment_revisions(EnvironmentRevisionQuery::ForRequest {
                        request_id: request.request_id.clone(),
                    })
                    .await
                    .map_err(|_| EnvironmentFileError::LedgerFailure)?;
                let Some(record) = records.into_iter().next() else {
                    continue;
                };
                if record.state != EnvironmentRevisionState::Committed {
                    continue;
                }
                match self.bind_request_receipt(&record, at).await {
                    Ok(()) => summary.receipts_bound += 1,
                    Err(error) => {
                        summary.failed += 1;
                        tracing::warn!(target:"capsem.environment",request_id=%request.request_id,error=%error,"committed environment request receipt remains unresolved");
                    }
                }
            }
        }
        Ok(())
    }
}
