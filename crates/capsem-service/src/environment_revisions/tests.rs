use super::*;
use crate::environment_files::EnvironmentFiles;
use capsem_config::environment::{EnvironmentDocument, EnvironmentIdentity};
use capsem_logger::{
    db::EnvironmentRevisionQuery,
    events::{EnvironmentRevisionState, PreparedEnvironmentRevision},
    DbHandle, WriteOp,
};
use std::sync::Arc;
#[path = "tests/receipts.rs"]
mod receipts;

fn id(n: u64) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}
fn revision(env: u64, number: u64, previous: Option<u64>) -> PreparedEnvironmentRevision {
    PreparedEnvironmentRevision {
        environment_id: id(env),
        revision_id: id(number),
        previous_revision_id: previous.map(id),
        fork_parent: None,
        timestamp_unix_ms: number as i64,
        actor: "operator".into(),
        reason: "approved change".into(),
        request_id: None,
        payload_event_id: format!("{number:012x}"),
        specification: Box::new(
            toml::from_str(&format!("name='development'\n[variables]\nVALUE='revision-{number}'\n")).unwrap(),
        ),
    }
}
fn document(record: &PreparedEnvironmentRevision) -> EnvironmentDocument {
    EnvironmentDocument {
        identity: EnvironmentIdentity {
            schema_version: 1,
            id: record.environment_id.clone(),
            revision_id: record.revision_id.clone(),
            parent_environment_id: None,
            parent_revision_id: None,
        },
        specification: *record.specification.clone(),
    }
}
struct Fixture {
    root: tempfile::TempDir,
    owner: Arc<EnvironmentRevisions>,
}

#[tokio::test]
async fn environment_startup_recovery_pages_pending_rows_and_isolates_missing_files() {
    let f = Fixture::new();
    f.owner.commit(revision(1, 2, None), 10).wait().await.unwrap();
    f.prepare(revision(1, 3, Some(2))).await;
    let published = revision(8, 9, None);
    f.prepare(published.clone()).await;
    f.owner.files.publish_document(document(&published)).await.unwrap();
    f.prepare(revision(10, 11, None)).await;
    let summary = f.owner.recover_pending_owned(20, 1).await.unwrap();
    assert_eq!(
        summary,
        RecoverySummary {
            committed: 1,
            recovered_unapplied: 1,
            receipts_bound: 0,
            failed: 1
        }
    );
    assert_eq!(f.state(3).await, EnvironmentRevisionState::RecoveredUnapplied);
    assert_eq!(f.owner.load_current(&id(8)).await.unwrap().revision.revision_id, id(9));
    assert!(f.owner.load_current(&id(10)).await.is_err());
    let pending = f
        .owner
        .ledger
        .environment_revisions(EnvironmentRevisionQuery::Page {
            environment_id: None,
            state: Some(EnvironmentRevisionState::Committing),
            after_id: 0,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].environment_id, id(10));
    assert_eq!(f.owner.reconcile_pending(21).wait().await.unwrap().failed, 1);
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let files = Arc::new(EnvironmentFiles::open(&root.path().join("environments")).unwrap());
        let ledger = Arc::new(DbHandle::open(&root.path().join("host.db")).unwrap());
        Self {
            root,
            owner: Arc::new(EnvironmentRevisions::new(files, ledger)),
        }
    }
    async fn prepare(&self, record: PreparedEnvironmentRevision) {
        self.owner
            .ledger
            .write(WriteOp::PrepareEnvironmentRevision(record))
            .await
            .unwrap();
        self.owner.ledger.flush().await.unwrap();
    }
    async fn state(&self, number: u64) -> EnvironmentRevisionState {
        self.owner
            .ledger
            .environment_revisions(EnvironmentRevisionQuery::Get {
                environment_id: id(1),
                revision_id: id(number),
            })
            .await
            .unwrap()[0]
            .state
    }
}

#[tokio::test]
async fn environment_revision_jobs_create_edit_retry_and_refuse_stale_predecessors() {
    let f = Fixture::new();
    let first = revision(1, 2, None);
    let record = f.owner.commit(first.clone(), 10).wait().await.unwrap();
    assert_eq!(record.state, EnvironmentRevisionState::Committed);
    assert_eq!(f.owner.load_current(&id(1)).await.unwrap().document, document(&first));
    let mut conflict = first.clone();
    conflict.reason = "changed-immutable-metadata-canary".into();
    assert!(!f
        .owner
        .commit(conflict, 11)
        .wait()
        .await
        .unwrap_err()
        .to_string()
        .contains("canary"));
    assert_eq!(f.owner.commit(first, 11).wait().await.unwrap(), record);
    let next = revision(1, 3, Some(2));
    f.owner.commit(next.clone(), 12).wait().await.unwrap();
    assert_eq!(f.owner.load_current(&id(1)).await.unwrap().document, document(&next));
    assert!(f.owner.commit(revision(1, 4, Some(2)), 13).wait().await.is_err());
    assert_eq!(f.owner.load_current(&id(1)).await.unwrap().revision.revision_id, id(3));
}

#[tokio::test]
async fn environment_revision_jobs_do_not_publish_after_ledger_rejection() {
    let f = Fixture::new();
    let mut invalid = revision(1, 2, None);
    invalid.actor = "unverified-caller-canary".into();
    let error = f.owner.commit(invalid, 10).wait().await.unwrap_err();
    assert_eq!(error, crate::environment_files::EnvironmentFileError::LedgerFailure);
    assert!(!f.root.path().join("environments").join(id(1)).exists());
    assert!(f
        .owner
        .ledger
        .environment_revisions(EnvironmentRevisionQuery::Get {
            environment_id: id(1),
            revision_id: id(2)
        })
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn environment_revision_jobs_keep_verified_fork_lineage_immutable() {
    let f = Fixture::new();
    f.owner.commit(revision(1, 2, None), 10).wait().await.unwrap();
    let mut fork = revision(8, 9, None);
    fork.fork_parent = Some(capsem_logger::events::EnvironmentRevisionParent {
        environment_id: id(1),
        revision_id: id(2),
    });
    f.owner.commit(fork.clone(), 11).wait().await.unwrap();
    let current = f.owner.load_current(&id(8)).await.unwrap();
    assert_eq!(current.document.identity.parent_environment_id, Some(id(1)));
    assert_eq!(current.document.identity.parent_revision_id, Some(id(2)));
    let mut changed = revision(8, 10, Some(9));
    changed.fork_parent = None;
    assert!(f.owner.commit(changed, 12).wait().await.is_err());
    assert_eq!(f.owner.load_current(&id(8)).await.unwrap().revision.revision_id, id(9));
}

#[tokio::test]
async fn environment_revision_jobs_reconcile_prepared_file_and_verified_predecessor() {
    let f = Fixture::new();
    f.owner.commit(revision(1, 2, None), 10).wait().await.unwrap();
    let next = revision(1, 3, Some(2));
    f.prepare(next.clone()).await;
    f.owner.files.publish_document(document(&next)).await.unwrap();
    let recovery = f.owner.reconcile(id(1), 11).wait().await.unwrap().unwrap();
    assert_eq!(recovery.state, EnvironmentRevisionState::Committed);
    assert_eq!(f.state(3).await, EnvironmentRevisionState::Committed);
    let mut unapplied = revision(1, 4, Some(3));
    unapplied.actor = "system".into();
    f.prepare(unapplied.clone()).await;
    let recovery = f.owner.reconcile(id(1), 12).wait().await.unwrap().unwrap();
    assert_eq!(recovery.state, EnvironmentRevisionState::RecoveredUnapplied);
    assert_eq!(f.owner.load_current(&id(1)).await.unwrap().revision.revision_id, id(3));
    assert!(
        f.owner.commit(unapplied.clone(), 13).wait().await.is_err(),
        "recovery does not authorize an implicit retry"
    );
    f.owner.retry_unapplied(unapplied, 14).wait().await.unwrap();
    assert_eq!(f.owner.load_current(&id(1)).await.unwrap().revision.revision_id, id(4));
    assert_eq!(
        f.owner.ledger.host_events().await.unwrap().last().unwrap().actor,
        "operator"
    );
}

#[tokio::test]
async fn environment_revision_jobs_recover_after_reopening_the_host_ledger() {
    let f = Fixture::new();
    f.owner.commit(revision(1, 2, None), 10).wait().await.unwrap();
    let next = revision(1, 3, Some(2));
    f.prepare(next.clone()).await;
    f.owner.files.publish_document(document(&next)).await.unwrap();
    let Fixture { root, owner } = f;
    drop(owner);
    let ledger = Arc::new(DbHandle::open(&root.path().join("host.db")).unwrap());
    let files = Arc::new(EnvironmentFiles::open(&root.path().join("environments")).unwrap());
    let owner = Arc::new(EnvironmentRevisions::new(files, ledger));
    assert_eq!(
        owner.reconcile(id(1), 11).wait().await.unwrap().unwrap().state,
        EnvironmentRevisionState::Committed
    );
    let current = owner.load_current(&id(1)).await.unwrap();
    assert_eq!(current.document, document(&next));
    assert_eq!(current.revision.timestamp_unix_ms, 3);
}

#[tokio::test]
async fn environment_revision_jobs_keep_prepared_evidence_after_file_publication_fails() {
    let f = Fixture::new();
    let first = revision(1, 2, None);
    let (prepared, release) = f.owner.pause_next_prepare();
    let job = f.owner.commit(first.clone(), 10);
    prepared.await.unwrap();
    let directory = f.root.path().join("environments").join(id(1));
    capsem_foundation::unix::fs::ensure_private_dir(&directory).unwrap();
    std::fs::create_dir(directory.join("environment.toml")).unwrap();
    release.send(()).unwrap();
    assert_eq!(
        job.wait().await.unwrap_err(),
        crate::environment_files::EnvironmentFileError::PublicationFailure
    );
    assert_eq!(f.state(2).await, EnvironmentRevisionState::Committing);
    assert!(f.owner.reconcile(id(1), 11).wait().await.is_err());
    std::fs::remove_dir(directory.join("environment.toml")).unwrap();
    assert!(
        f.owner.reconcile(id(1), 12).wait().await.is_err(),
        "a missing file must not discard approved intent"
    );
    f.owner.files.publish_document(document(&first)).await.unwrap();
    assert_eq!(
        f.owner.reconcile(id(1), 13).wait().await.unwrap().unwrap().state,
        EnvironmentRevisionState::Committed
    );
}

#[tokio::test]
async fn environment_revision_jobs_refuse_missing_unknown_and_mismatched_recovery_state() {
    let f = Fixture::new();
    let first = revision(1, 2, None);
    f.prepare(first.clone()).await;
    assert!(f.owner.reconcile(id(1), 10).wait().await.is_err());
    assert_eq!(f.state(2).await, EnvironmentRevisionState::Committing);
    for mut value in [document(&revision(1, 99, None)), document(&first)] {
        value
            .specification
            .variables
            .insert("VALUE".into(), "unapproved-value-canary".into());
        f.owner.files.publish_document(value).await.unwrap();
        let error = f.owner.reconcile(id(1), 11).wait().await.unwrap_err();
        assert!(!error.to_string().contains("canary"));
        assert_eq!(f.state(2).await, EnvironmentRevisionState::Committing);
    }
}

#[tokio::test]
async fn environment_revision_jobs_survive_response_cancellation_and_serialize_per_environment() {
    let f = Fixture::new();
    let (prepared, release) = f.owner.pause_next_prepare();
    let job = f.owner.commit(revision(1, 2, None), 10);
    prepared.await.unwrap();
    drop(job);
    let competing = f.owner.commit(revision(1, 3, None), 11);
    let unrelated = f.owner.commit(revision(8, 9, None), 12);
    unrelated.wait().await.unwrap();
    release.send(()).unwrap();
    assert!(competing.wait().await.is_err());
    assert_eq!(f.owner.load_current(&id(1)).await.unwrap().revision.revision_id, id(2));
    assert!(f
        .root
        .path()
        .join("environments")
        .join(id(1))
        .join("environment.toml")
        .is_file());
}

#[tokio::test]
async fn launch_revision_load_keeps_the_bound_history_after_shared_environment_edits() {
    let f = Fixture::new();
    f.owner.commit(revision(1, 2, None), 10).wait().await.unwrap();
    f.owner.commit(revision(1, 3, Some(2)), 11).wait().await.unwrap();
    let pinned = f.owner.load_revision(&id(1), &id(2)).await.unwrap();
    assert_eq!(pinned.record.revision_id, id(2));
    assert_eq!(pinned.specification.variables["VALUE"], "revision-2");
    assert_eq!(f.owner.load_current(&id(1)).await.unwrap().revision.revision_id, id(3));
    assert!(f.owner.load_revision(&id(8), &id(2)).await.is_err());
    f.prepare(revision(1, 4, Some(3))).await;
    assert!(matches!(
        f.owner.load_revision(&id(1), &id(4)).await,
        Err(EnvironmentFileError::RecoveryRequired)
    ));
}
