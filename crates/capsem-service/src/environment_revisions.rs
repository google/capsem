//! Trusted service coordination; authenticated approval routes own caller admission.
use crate::environment_files::{
    current_revision, validate_id, CurrentEnvironment, EnvironmentFileError, EnvironmentFiles,
};
use capsem_config::environment::{EnvironmentDocument, EnvironmentIdentity};
use capsem_logger::{
    db::{EnvironmentRevisionQuery, EnvironmentRevisionRecord, EnvironmentRevisionSnapshot},
    events::{
        EnvironmentRevisionParent, EnvironmentRevisionState, EnvironmentRevisionTransition, PreparedEnvironmentRevision,
    },
    DbHandle, WriteOp,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::oneshot;
mod history;
mod mutations;
mod receipts;
mod reviews;
pub use mutations::{
    ApprovedEnvironmentReview, EnvironmentMutationCoordinator, EnvironmentMutationError, EnvironmentMutationJob,
    EnvironmentProposalOrigin, ResolvedEnvironmentReview, ReviewedEnvironmentChange,
};

mod startup;
pub use startup::RecoverySummary;

/// Dropping the response cannot abort the accepted coordinator operation.
pub struct RevisionJob<T> {
    receiver: oneshot::Receiver<Result<T, EnvironmentFileError>>,
}
impl<T> RevisionJob<T> {
    pub async fn wait(self) -> Result<T, EnvironmentFileError> {
        self.receiver.await.map_err(|_| EnvironmentFileError::WorkerFailed)?
    }
}
#[derive(Debug, PartialEq, Eq)]
pub struct RevisionRecovery {
    pub revision_id: String,
    pub state: EnvironmentRevisionState,
}
#[cfg(test)]
type PreparePause = (oneshot::Sender<()>, oneshot::Receiver<()>);

/// Construct once with the trusted host-ledger capability. Public request DTOs
/// must never deserialize PreparedEnvironmentRevision: approval orchestration
/// assigns and authenticates its identities, actor, request and timestamps.
pub struct EnvironmentRevisions {
    files: Arc<EnvironmentFiles>,
    ledger: Arc<DbHandle>,
    locks: Mutex<BTreeMap<String, Weak<tokio::sync::Mutex<()>>>>,
    #[cfg(test)]
    pause: Mutex<Option<PreparePause>>,
}
impl EnvironmentRevisions {
    pub fn new(files: Arc<EnvironmentFiles>, ledger: Arc<DbHandle>) -> Self {
        Self {
            files,
            ledger,
            locks: Mutex::new(BTreeMap::new()),
            #[cfg(test)]
            pause: Mutex::new(None),
        }
    }
    pub fn commit(
        self: &Arc<Self>,
        revision: PreparedEnvironmentRevision,
        at: i64,
    ) -> RevisionJob<EnvironmentRevisionRecord> {
        self.start_commit(revision, at, CommitMode::New)
    }
    /// A distinct deliberate retry operation; a recovered approval is never
    /// silently reapplied by ordinary reconciliation or a new edit.
    /// Only the authenticated operator admission path may invoke this operation.
    pub fn retry_unapplied(
        self: &Arc<Self>,
        revision: PreparedEnvironmentRevision,
        at: i64,
    ) -> RevisionJob<EnvironmentRevisionRecord> {
        self.start_commit(revision, at, CommitMode::RetryUnapplied)
    }
    fn start_commit(
        self: &Arc<Self>,
        revision: PreparedEnvironmentRevision,
        at: i64,
        mode: CommitMode,
    ) -> RevisionJob<EnvironmentRevisionRecord> {
        let owner = Arc::clone(self);
        let (sender, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let result = owner.commit_owned(revision, at, mode).await;
            drop(owner);
            let _ = sender.send(result);
        });
        RevisionJob { receiver }
    }
    pub fn reconcile(self: &Arc<Self>, id: String, at: i64) -> RevisionJob<Option<RevisionRecovery>> {
        let owner = Arc::clone(self);
        let (sender, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let result = async {
                validate_id(&id)?;
                if at < 0 {
                    return Err(EnvironmentFileError::InvalidDocument);
                }
                let lock = owner.lock_for(&id)?;
                let _guard = lock.lock().await;
                owner.reconcile_locked(&id, at).await
            }
            .await;
            drop(owner);
            let _ = sender.send(result);
        });
        RevisionJob { receiver }
    }
    pub async fn load_current(&self, id: &str) -> Result<CurrentEnvironment, EnvironmentFileError> {
        validate_id(id)?;
        let lock = self.lock_for(id)?;
        let _guard = lock.lock().await;
        self.files.load_current(&self.ledger, id).await
    }

    pub async fn is_archived(&self, id: &str) -> Result<bool, EnvironmentFileError> {
        validate_id(id)?;
        let records = self
            .ledger
            .environment_references(capsem_logger::db::EnvironmentReferenceQuery::ArchivedEnvironment {
                environment_id: id.into(),
            })
            .await
            .map_err(|_| EnvironmentFileError::LedgerFailure)?;
        match records.as_slice() {
            [] => Ok(false),
            [capsem_logger::db::EnvironmentReferenceRecord::ArchivedEnvironment { revision }]
                if revision.environment_id == id =>
            {
                Ok(true)
            }
            _ => Err(EnvironmentFileError::Unverified),
        }
    }
    /// Load a committed immutable revision, including a historical binding whose
    /// shared environment has since advanced. This is data, not launch authority.
    pub async fn load_revision(
        &self,
        id: &str,
        revision: &str,
    ) -> Result<EnvironmentRevisionSnapshot, EnvironmentFileError> {
        validate_id(id)?;
        validate_id(revision)?;
        let snapshot = self.snapshot(id, revision).await?;
        if snapshot.record.environment_id != id || snapshot.record.revision_id != revision {
            return Err(EnvironmentFileError::Unverified);
        }
        match snapshot.record.state {
            EnvironmentRevisionState::Committing => return Err(EnvironmentFileError::RecoveryRequired),
            EnvironmentRevisionState::RecoveredUnapplied => return Err(EnvironmentFileError::Unverified),
            EnvironmentRevisionState::Committed => {}
        }
        if !snapshot.record.replayable {
            return Err(EnvironmentFileError::Unverified);
        }
        snapshot
            .specification
            .validate()
            .map_err(|_| EnvironmentFileError::InvalidDocument)?;
        Ok(snapshot)
    }
    fn lock_for(&self, id: &str) -> Result<Arc<tokio::sync::Mutex<()>>, EnvironmentFileError> {
        let mut locks = self.locks.lock().map_err(|_| EnvironmentFileError::WorkerFailed)?;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(id).and_then(Weak::upgrade) {
            return Ok(lock);
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(id.into(), Arc::downgrade(&lock));
        drop(locks);
        Ok(lock)
    }
    async fn commit_owned(
        &self,
        revision: PreparedEnvironmentRevision,
        at: i64,
        mode: CommitMode,
    ) -> Result<EnvironmentRevisionRecord, EnvironmentFileError> {
        validate_id(&revision.environment_id)?;
        let lock = self.lock_for(&revision.environment_id)?;
        let guard = lock.lock().await;
        self.commit_locked(revision, at, mode, &guard).await
    }
    /// The mutation owner may already hold this environment's guard across
    /// its final base check and decision. Keep publication on the same rail.
    async fn commit_locked(
        &self,
        revision: PreparedEnvironmentRevision,
        at: i64,
        mode: CommitMode,
        _guard: &tokio::sync::MutexGuard<'_, ()>,
    ) -> Result<EnvironmentRevisionRecord, EnvironmentFileError> {
        validate_id(&revision.environment_id)?;
        if at < revision.timestamp_unix_ms {
            return Err(EnvironmentFileError::InvalidDocument);
        }
        self.reconcile_locked(&revision.environment_id, at).await?;
        let (revision, content) = encode_revision(revision).await?;
        let id = revision.environment_id.clone();
        let revision_id = revision.revision_id.clone();
        let existing = self.record(&id, &revision_id).await?;
        let current = current_revision(&self.ledger, &id).await?;
        let state = existing.as_ref().map(|r| r.state);
        if state == Some(EnvironmentRevisionState::Committed) {
            if current.as_deref() != Some(revision_id.as_str()) {
                return Err(EnvironmentFileError::Changed);
            }
        } else {
            if current != revision.previous_revision_id {
                return Err(EnvironmentFileError::Changed);
            }
            match (mode, state) {
                (CommitMode::New, None)
                | (CommitMode::RetryUnapplied, Some(EnvironmentRevisionState::RecoveredUnapplied)) => {}
                _ => return Err(EnvironmentFileError::RecoveryRequired),
            }
        }
        let actor = match mode {
            CommitMode::New => revision.actor.clone(),
            CommitMode::RetryUnapplied => "operator".into(),
        };
        // This also checks exact immutable retries against the archived bytes.
        self.write(WriteOp::PrepareEnvironmentRevision(revision)).await?;
        if state == Some(EnvironmentRevisionState::Committed) {
            return self.completed_record(&id, &revision_id, at).await;
        }
        if state == Some(EnvironmentRevisionState::RecoveredUnapplied) {
            self.transition(&id, &revision_id, EnvironmentRevisionState::Committing, at, &actor)
                .await?;
        }
        #[cfg(test)]
        self.wait_prepared_pause().await;
        self.files.publish_serialized(id.clone(), content).await?;
        self.transition(&id, &revision_id, EnvironmentRevisionState::Committed, at, &actor)
            .await?;
        self.completed_record(&id, &revision_id, at).await
    }
    async fn reconcile_locked(&self, id: &str, at: i64) -> Result<Option<RevisionRecovery>, EnvironmentFileError> {
        let pending = self
            .ledger
            .environment_revisions(EnvironmentRevisionQuery::Page {
                environment_id: Some(id.into()),
                state: Some(EnvironmentRevisionState::Committing),
                after_id: 0,
                limit: 2,
            })
            .await
            .map_err(|_| EnvironmentFileError::LedgerFailure)?;
        if pending.is_empty() {
            if self.files.read_existing(id).await?.is_some() {
                self.files.load_current(&self.ledger, id).await?;
            } else if current_revision(&self.ledger, id).await?.is_some() {
                return Err(EnvironmentFileError::Unavailable);
            }
            return Ok(None);
        }
        if pending.len() != 1 {
            return Err(EnvironmentFileError::Unverified);
        }
        let record = &pending[0];
        let snapshot = self.snapshot(id, &record.revision_id).await?;
        if snapshot.record.state != EnvironmentRevisionState::Committing || at < record.timestamp_unix_ms {
            return Err(EnvironmentFileError::Unverified);
        }
        let file = self
            .files
            .read_existing(id)
            .await?
            .ok_or(EnvironmentFileError::Unavailable)?;
        let state = if matches_snapshot(&file, &snapshot) {
            EnvironmentRevisionState::Committed
        } else if let Some(previous) = record.previous_revision_id.as_deref() {
            let predecessor = self.snapshot(id, previous).await?;
            if predecessor.record.state != EnvironmentRevisionState::Committed
                || current_revision(&self.ledger, id).await?.as_deref() != Some(previous)
                || !matches_snapshot(&file, &predecessor)
            {
                return Err(EnvironmentFileError::Unverified);
            }
            EnvironmentRevisionState::RecoveredUnapplied
        } else {
            return Err(EnvironmentFileError::Unverified);
        };
        self.files.sync_existing(id).await?;
        self.transition(id, &record.revision_id, state, at, "system").await?;
        Ok(Some(RevisionRecovery {
            revision_id: record.revision_id.clone(),
            state,
        }))
    }
    async fn snapshot(&self, id: &str, revision: &str) -> Result<EnvironmentRevisionSnapshot, EnvironmentFileError> {
        self.ledger
            .environment_revision_snapshot(id, revision)
            .await
            .map_err(|_| EnvironmentFileError::Unavailable)?
            .ok_or(EnvironmentFileError::Unavailable)
    }
    async fn record(
        &self,
        id: &str,
        revision: &str,
    ) -> Result<Option<EnvironmentRevisionRecord>, EnvironmentFileError> {
        Ok(self
            .ledger
            .environment_revisions(EnvironmentRevisionQuery::Get {
                environment_id: id.into(),
                revision_id: revision.into(),
            })
            .await
            .map_err(|_| EnvironmentFileError::LedgerFailure)?
            .into_iter()
            .next())
    }
    async fn required_record(
        &self,
        id: &str,
        revision: &str,
    ) -> Result<EnvironmentRevisionRecord, EnvironmentFileError> {
        self.record(id, revision)
            .await?
            .ok_or(EnvironmentFileError::Unavailable)
    }
    async fn write(&self, operation: WriteOp) -> Result<(), EnvironmentFileError> {
        self.ledger
            .write(operation)
            .await
            .map_err(|_| EnvironmentFileError::LedgerFailure)?;
        self.ledger
            .flush()
            .await
            .map_err(|_| EnvironmentFileError::LedgerFailure)
    }
    async fn transition(
        &self,
        id: &str,
        revision: &str,
        state: EnvironmentRevisionState,
        at: i64,
        actor: &str,
    ) -> Result<(), EnvironmentFileError> {
        self.write(WriteOp::EnvironmentRevisionTransition(EnvironmentRevisionTransition {
            environment_id: id.into(),
            revision_id: revision.into(),
            state,
            timestamp_unix_ms: at,
            actor: actor.into(),
        }))
        .await
    }
    #[cfg(test)]
    fn pause_next_prepare(&self) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (notify, notified) = oneshot::channel();
        let (release, released) = oneshot::channel();
        assert!(self.pause.lock().unwrap().replace((notify, released)).is_none());
        (notified, release)
    }
    #[cfg(test)]
    async fn wait_prepared_pause(&self) {
        let pause = self.pause.lock().unwrap().take();
        if let Some((prepared, release)) = pause {
            let _ = prepared.send(());
            let _ = release.await;
        }
    }
}
#[derive(Clone, Copy)]
enum CommitMode {
    New,
    RetryUnapplied,
}
fn identity(id: &str, revision: &str, parent: Option<&EnvironmentRevisionParent>) -> EnvironmentIdentity {
    EnvironmentIdentity {
        schema_version: 1,
        id: id.into(),
        revision_id: revision.into(),
        parent_environment_id: parent.map(|p| p.environment_id.clone()),
        parent_revision_id: parent.map(|p| p.revision_id.clone()),
    }
}
fn matches_snapshot(document: &EnvironmentDocument, snapshot: &EnvironmentRevisionSnapshot) -> bool {
    document.identity
        == identity(
            &snapshot.record.environment_id,
            &snapshot.record.revision_id,
            snapshot.record.fork_parent.as_ref(),
        )
        && document.specification == *snapshot.specification
}
async fn encode_revision(
    mut revision: PreparedEnvironmentRevision,
) -> Result<(PreparedEnvironmentRevision, String), EnvironmentFileError> {
    tokio::task::spawn_blocking(move || {
        let document = EnvironmentDocument {
            identity: identity(
                &revision.environment_id,
                &revision.revision_id,
                revision.fork_parent.as_ref(),
            ),
            specification: *revision.specification,
        };
        let content = document.to_toml().map_err(|_| EnvironmentFileError::InvalidDocument)?;
        *revision.specification = document.specification;
        Ok((revision, content))
    })
    .await
    .map_err(|_| EnvironmentFileError::WorkerFailed)?
}

#[cfg(test)]
mod tests;
