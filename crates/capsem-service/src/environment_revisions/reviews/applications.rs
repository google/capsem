//! Correlate each frozen target with its durable, generation-specific receipt.
use super::*;
use capsem_logger::{
    db::EnvironmentApplicationQuery,
    events::environment::application::{EnvironmentApplicationRecord, EnvironmentApplicationState},
};
use std::collections::BTreeMap;

impl EnvironmentRevisions {
    pub(in crate::environment_revisions) async fn mutation_view(
        &self,
        record: EnvironmentRequestRecord,
    ) -> Result<EnvironmentMutationResponse, Error> {
        let is_batch = record.kind == EnvironmentRequestKind::Apply;
        let mut observed = view(record);
        if !is_batch || observed.result.is_none() {
            return Ok(observed);
        }
        let history = self
            .ledger
            .environment_request_history(&observed.request_id)
            .await
            .map_err(|_| Error::LedgerFailure)?
            .ok_or(Error::LedgerFailure)?;
        if let EnvironmentHistoryPayload::Complete { value } = history.payload {
            let EnvironmentRequestPayload::Apply { targets, .. } = *value else {
                return Err(Error::LedgerFailure);
            };
            let outcomes = self.apply_outcomes(&observed, &targets).await?;
            observed.application = summary_state(&outcomes);
            observed.application_error = outcomes.into_iter().find_map(|target| target.failure_reason);
        }
        Ok(observed)
    }

    pub(super) async fn apply_outcomes(
        &self,
        observed: &EnvironmentMutationResponse,
        targets: &[EnvironmentRequestTarget],
    ) -> Result<Vec<RequestTargetApplication>, Error> {
        let mut receipts = BTreeMap::new();
        if let Some(result) = &observed.result {
            for row in self
                .ledger
                .environment_applications(EnvironmentApplicationQuery::Page {
                    request_id: Some(observed.request_id.clone()),
                    target: None,
                    after_id: 0,
                    limit: capsem_proto::APPROVAL_PENDING_TOTAL as u16,
                })
                .await
                .map_err(|_| Error::LedgerFailure)?
            {
                let desired = &row.composition.desired;
                if desired.environment_id != result.environment_id
                    || desired.revision_id != result.revision_id
                    || receipts.insert(row.target.session_id.clone(), row).is_some()
                {
                    return Err(Error::LedgerFailure);
                }
            }
        }
        let outcomes = targets
            .iter()
            .map(|target| {
                let receipt = receipts.remove(&target.session_id);
                if receipt.as_ref().is_some_and(|receipt| &receipt.target != target) {
                    return Err(Error::LedgerFailure);
                }
                Ok(outcome(target.clone(), receipt))
            })
            .collect::<Result<_, _>>()?;
        if !receipts.is_empty() {
            return Err(Error::LedgerFailure);
        }
        Ok(outcomes)
    }
}

fn outcome(
    intended: EnvironmentRequestTarget,
    receipt: Option<EnvironmentApplicationRecord>,
) -> RequestTargetApplication {
    let Some(record) = receipt else {
        return RequestTargetApplication {
            target: target(intended),
            state: RequestApplicationState::Pending,
            composition: None,
            timestamp_unix_ms: None,
            failure_reason: None,
        };
    };
    let c = record.composition;
    let state = match record.state {
        EnvironmentApplicationState::Applying => RequestApplicationState::Applying,
        EnvironmentApplicationState::Applied if !c.pending_restart => RequestApplicationState::Applied,
        EnvironmentApplicationState::Failed => RequestApplicationState::Failed,
        _ => RequestApplicationState::Pending,
    };
    RequestTargetApplication {
        target: target(intended),
        state,
        composition: Some(RequestApplicationComposition {
            boot: reference(c.boot),
            live_policy: reference(c.live_policy),
            desired: reference(c.desired),
            corporate_policy_id: c.corporate_policy_id,
            pending_restart: c.pending_restart,
        }),
        timestamp_unix_ms: Some(record.timestamp_unix_ms),
        failure_reason: record.failure_reason,
    }
}

pub(super) fn summary_state(outcomes: &[RequestTargetApplication]) -> RequestApplicationState {
    use RequestApplicationState::{Applied, Applying, Failed, Pending};
    if outcomes.iter().any(|outcome| outcome.state == Failed) {
        Failed
    } else if outcomes.iter().any(|outcome| outcome.state == Applying) {
        Applying
    } else if !outcomes.is_empty() && outcomes.iter().all(|outcome| outcome.state == Applied) {
        Applied
    } else {
        Pending
    }
}
