//! All environment reads execute through the existing DB handle.
use crate::ledger_protocol::{LedgerEnvironmentQuery as Q, LedgerEnvironmentReply as R};
use crate::DbHandle;

pub(super) async fn execute(db: &DbHandle, query: Q) -> Result<R, String> {
    Ok(match query {
        Q::Revisions { query } => R::Revisions {
            records: db.environment_revisions(query).await?,
        },
        Q::RevisionSnapshot {
            environment_id,
            revision_id,
        } => R::RevisionSnapshot {
            snapshot: db
                .environment_revision_snapshot(&environment_id, &revision_id)
                .await?
                .map(Box::new),
        },
        Q::RevisionHistory {
            environment_id,
            revision_id,
        } => R::RevisionHistory {
            history: db
                .environment_revision_history(&environment_id, &revision_id)
                .await?
                .map(Box::new),
        },
        Q::Requests { query } => R::Requests {
            records: db.environment_requests(query).await?,
        },
        Q::RequestSnapshot { request_id } => R::RequestSnapshot {
            snapshot: db.environment_request_snapshot(&request_id).await?.map(Box::new),
        },
        Q::RequestHistory { request_id } => R::RequestHistory {
            history: db.environment_request_history(&request_id).await?.map(Box::new),
        },
        Q::References { query } => R::References {
            records: db.environment_references(query).await?,
        },
        Q::Applications { query } => R::Applications {
            records: db.environment_applications(query).await?,
        },
        Q::Purges { query } => R::Purges {
            records: db.environment_variable_purges(query).await?,
        },
    })
}
