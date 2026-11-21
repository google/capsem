use super::*;
use capsem_config::environment::{EnvironmentDocument, EnvironmentIdentity};
use capsem_logger::{
    events::{EnvironmentRevisionState, EnvironmentRevisionTransition, PreparedEnvironmentRevision},
    DbHandle, WriteOp,
};
use std::{path::PathBuf, sync::Arc};

fn id(n: u64) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}
fn prepared() -> PreparedEnvironmentRevision {
    PreparedEnvironmentRevision {
        environment_id: id(1),
        revision_id: id(2),
        previous_revision_id: None,
        fork_parent: None,
        timestamp_unix_ms: 42,
        actor: "operator".into(),
        reason: "create".into(),
        request_id: None,
        payload_event_id: "abcdef012345".into(),
        specification: Box::new(
            toml::from_str(
                "name='development'\n[variables]\nPRIVATE='current-variable-canary'\nKEPT='retained-value'\n",
            )
            .unwrap(),
        ),
    }
}
fn document(revision: &PreparedEnvironmentRevision) -> EnvironmentDocument {
    EnvironmentDocument {
        identity: EnvironmentIdentity {
            schema_version: 1,
            id: revision.environment_id.clone(),
            revision_id: revision.revision_id.clone(),
            parent_environment_id: None,
            parent_revision_id: None,
        },
        specification: *revision.specification.clone(),
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    ledger: Arc<DbHandle>,
    files: EnvironmentFiles,
    path: PathBuf,
    revision: PreparedEnvironmentRevision,
}
impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let ledger = Arc::new(DbHandle::open(&root.path().join("host.db")).unwrap());
        let directory = root.path().join("environments");
        let files = EnvironmentFiles::open(&directory).unwrap();
        let revision = prepared();
        let environment = directory.join(&revision.environment_id);
        capsem_foundation::unix::fs::ensure_private_dir(&environment).unwrap();
        let path = environment.join("environment.toml");
        ledger
            .write(WriteOp::PrepareEnvironmentRevision(revision.clone()))
            .await
            .unwrap();
        ledger.flush().await.unwrap();
        capsem_foundation::unix::fs::atomic_write_private(&path, document(&revision).to_toml().unwrap().as_bytes())
            .unwrap();
        Self {
            _root: root,
            ledger,
            files,
            path,
            revision,
        }
    }
    async fn commit(&self) {
        self.ledger
            .write(WriteOp::EnvironmentRevisionTransition(EnvironmentRevisionTransition {
                environment_id: self.revision.environment_id.clone(),
                revision_id: self.revision.revision_id.clone(),
                state: EnvironmentRevisionState::Committed,
                timestamp_unix_ms: 43,
                actor: "operator".into(),
            }))
            .await
            .unwrap();
        self.ledger.flush().await.unwrap();
    }
}

#[tokio::test]
async fn current_environment_requires_committed_matching_file_and_ledger() {
    let f = Fixture::new().await;
    assert!(
        f.files.load_current(&f.ledger, &id(1)).await.is_err(),
        "unfinished commit requires reconciliation"
    );
    f.commit().await;
    let current = f.files.load_current(&f.ledger, &id(1)).await.unwrap();
    assert_eq!(current.document, document(&f.revision));
    assert_eq!(current.revision.timestamp_unix_ms, 42);
    assert!(current.revision.replayable);
    assert_eq!(
        f.files.load_current(&f.ledger, &id(99)).await.unwrap_err(),
        EnvironmentFileError::Unavailable
    );
    let mut forged_lineage = document(&f.revision);
    forged_lineage.identity.parent_environment_id = Some(id(98));
    forged_lineage.identity.parent_revision_id = Some(id(97));
    capsem_foundation::unix::fs::atomic_write_private(&f.path, forged_lineage.to_toml().unwrap().as_bytes()).unwrap();
    assert_eq!(
        f.files.load_current(&f.ledger, &id(1)).await.unwrap_err(),
        EnvironmentFileError::Unverified
    );
    let mut changed = document(&f.revision);
    changed
        .specification
        .variables
        .insert("KEPT".into(), "unapproved-change-canary".into());
    capsem_foundation::unix::fs::atomic_write_private(&f.path, changed.to_toml().unwrap().as_bytes()).unwrap();
    let error = f.files.load_current(&f.ledger, &id(1)).await.unwrap_err();
    assert!(!error.to_string().contains("canary"));
    changed.identity.revision_id = id(99);
    capsem_foundation::unix::fs::atomic_write_private(&f.path, changed.to_toml().unwrap().as_bytes()).unwrap();
    assert!(f.files.load_current(&f.ledger, &id(1)).await.is_err());
    assert!(f.files.load_current(&f.ledger, "../escape").await.is_err());
}

#[tokio::test]
async fn current_environment_refuses_missing_linked_and_oversize_files() {
    let f = Fixture::new().await;
    f.commit().await;
    std::fs::remove_file(&f.path).unwrap();
    assert!(f.files.load_current(&f.ledger, &id(1)).await.is_err());
    capsem_foundation::unix::fs::atomic_write_private(
        &f.path,
        &vec![b' '; capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES + 1],
    )
    .unwrap();
    assert!(f.files.load_current(&f.ledger, &id(1)).await.is_err());
    std::fs::remove_file(&f.path).unwrap();
    let target = f._root.path().join("outside.toml");
    std::fs::write(&target, document(&f.revision).to_toml().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, &f.path).unwrap();
    assert!(f.files.load_current(&f.ledger, &id(1)).await.is_err());
    std::fs::remove_file(&f.path).unwrap();
    std::fs::hard_link(&target, &f.path).unwrap();
    assert_eq!(
        f.files.load_current(&f.ledger, &id(1)).await.unwrap_err(),
        EnvironmentFileError::UnsafeFile
    );
    let environment = f.path.parent().unwrap();
    let moved = f._root.path().join("moved-environment");
    std::fs::rename(environment, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, environment).unwrap();
    assert_eq!(
        f.files.load_current(&f.ledger, &id(1)).await.unwrap_err(),
        EnvironmentFileError::UnsafeFile
    );
}

#[tokio::test]
async fn current_environment_keeps_clean_values_after_historical_value_purge() {
    use capsem_logger::events::environment::purge::{
        EnvironmentVariablePurgeExecution, PreparedEnvironmentVariablePurge,
    };
    let f = Fixture::new().await;
    f.commit().await;
    let mut next = f.revision.clone();
    next.revision_id = id(4);
    next.previous_revision_id = Some(id(2));
    next.payload_event_id = "abcdef012346".into();
    next.timestamp_unix_ms = 43;
    next.specification
        .variables
        .insert("PRIVATE".into(), "different legitimate current value".into());
    f.ledger
        .write(WriteOp::PrepareEnvironmentRevision(next.clone()))
        .await
        .unwrap();
    f.ledger.flush().await.unwrap();
    capsem_foundation::unix::fs::atomic_write_private(&f.path, document(&next).to_toml().unwrap().as_bytes()).unwrap();
    f.ledger
        .write(WriteOp::EnvironmentRevisionTransition(EnvironmentRevisionTransition {
            environment_id: id(1),
            revision_id: id(4),
            state: EnvironmentRevisionState::Committed,
            timestamp_unix_ms: 44,
            actor: "operator".into(),
        }))
        .await
        .unwrap();
    f.ledger.flush().await.unwrap();
    f.ledger
        .write(WriteOp::PrepareEnvironmentVariablePurge(
            PreparedEnvironmentVariablePurge {
                purge_id: id(3),
                environment_id: id(1),
                source_revision_id: id(2),
                variable_name: "PRIVATE".into(),
                timestamp_unix_ms: 44,
                actor: "operator".into(),
            },
        ))
        .await
        .unwrap();
    f.ledger.flush().await.unwrap();
    f.ledger
        .purge_variable_history(EnvironmentVariablePurgeExecution {
            purge_id: id(3),
            timestamp_unix_ms: 45,
        })
        .await
        .unwrap();
    let current = f.files.load_current(&f.ledger, &id(1)).await.unwrap();
    assert!(current.revision.replayable);
    assert_eq!(current.revision.revision_id, id(4));
    assert!(f.ledger.environment_revision_snapshot(&id(1), &id(2)).await.is_err());
    assert!(matches!(
        f.ledger
            .environment_revision_history(&id(1), &id(2))
            .await
            .unwrap()
            .unwrap()
            .payload,
        capsem_logger::db::EnvironmentHistoryPayload::Purged { .. }
    ));
    let revisions = crate::environment_revisions::EnvironmentRevisions::new(
        Arc::new(EnvironmentFiles::open(f.path.parent().unwrap().parent().unwrap()).unwrap()),
        f.ledger.clone(),
    );
    let purged = revisions.revision_detail(&id(1), &id(2)).await.unwrap().unwrap();
    assert_eq!(
        purged.availability,
        capsem_api::requests::RequestReviewAvailability::Purged
    );
    assert!(purged.canonical_toml.is_none());
    assert_eq!(purged.purged_variables, ["PRIVATE"]);
    let partial = purged.partial_specification.unwrap();
    assert!(!partial.variables.contains_key("PRIVATE"));
    assert_eq!(partial.variables["KEPT"], "retained-value");
    let page = revisions.list_history(id(1), None, 2).await.unwrap();
    assert_eq!(page.revisions[0].environment.revision_id, id(4));
    assert!(!page.revisions[1].replayable);
    assert_eq!(
        current.document.specification.variables["PRIVATE"],
        "different legitimate current value"
    );
    let mut changed = current.document;
    changed
        .specification
        .variables
        .insert("KEPT".into(), "unapproved-change-canary".into());
    capsem_foundation::unix::fs::atomic_write_private(&f.path, changed.to_toml().unwrap().as_bytes()).unwrap();
    assert!(!f
        .files
        .load_current(&f.ledger, &id(1))
        .await
        .unwrap_err()
        .to_string()
        .contains("canary"));
}
