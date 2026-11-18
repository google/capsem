use super::*;
use crate::environment_files::EnvironmentFiles;
use capsem_config::{
    catalogs::CatalogKind,
    environment::{EnvironmentCatalogPin, EnvironmentCatalogSelection},
};
use std::sync::atomic::{AtomicUsize, Ordering};

fn id(n: u64) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}
fn request(n: u64) -> EnvironmentCreateRequest {
    EnvironmentCreateRequest {
        request_id: id(n),
        specification: toml::from_str(
            "name='development'\n[image]\nreference='development:stable'\n[variables]\nEDITOR='vim'",
        )
        .unwrap(),
        reason: "create development environment".into(),
        requesting_sandbox: None,
    }
}
#[derive(Clone, Default)]
struct Source {
    calls: Arc<AtomicUsize>,
    pause: Option<Arc<(tokio::sync::Notify, tokio::sync::Notify)>>,
    fail: bool,
}
impl EnvironmentPinResolver for Source {
    async fn image(&self, _: &str) -> Result<String, EnvironmentResolutionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(pause) = &self.pause {
            pause.0.notify_one();
            pause.1.notified().await;
        }
        if self.fail {
            Err(EnvironmentResolutionError::Unavailable)
        } else {
            Ok(format!("sha256:{}", "a".repeat(64)))
        }
    }
    async fn repository(&self, _: &str, _: &str) -> Result<String, EnvironmentResolutionError> {
        unreachable!()
    }
    async fn catalog(
        &self,
        _: CatalogKind,
        _: &EnvironmentCatalogSelection,
    ) -> Result<EnvironmentCatalogPin, EnvironmentResolutionError> {
        unreachable!()
    }
    async fn admit_catalog(&self, _: CatalogKind, _: &EnvironmentCatalogPin) -> Result<(), EnvironmentResolutionError> {
        unreachable!()
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    revisions: Arc<EnvironmentRevisions>,
    creates: Arc<EnvironmentMutationCoordinator>,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let files = Arc::new(EnvironmentFiles::open(&root.path().join("environments")).unwrap());
        let ledger = Arc::new(DbHandle::open(&root.path().join("host.db")).unwrap());
        let revisions = Arc::new(EnvironmentRevisions::new(files, ledger));
        let creates = Arc::new(EnvironmentMutationCoordinator::new(revisions.clone()));
        Self {
            _root: root,
            revisions,
            creates,
        }
    }
}

#[tokio::test]
async fn operator_creation_persists_frozen_intent_decision_publication_and_result_receipt() {
    let f = Fixture::new();
    let source = Source::default();
    let result = f
        .creates
        .submit(request(1), source.clone())
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_eq!(result.decision, RequestState::Approved);
    assert_eq!(result.application, RequestApplicationState::Applied);
    let committed = result.result.as_ref().unwrap();
    let current = f.revisions.load_current(&result.environment_id).await.unwrap();
    assert_eq!(current.revision.revision_id, committed.revision_id);
    assert_eq!(current.document.specification.variables["EDITOR"], "vim");
    assert_eq!(
        current.document.specification.image.unwrap().resolved_digest,
        format!("sha256:{}", "a".repeat(64))
    );
    let saved = f
        .revisions
        .ledger
        .environment_request_snapshot(&id(1))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.record.result.unwrap().revision_id, committed.revision_id);
    assert_eq!(saved.record.actor, "operator");
    assert!(saved.record.requester.is_none());
    assert_eq!(
        saved.record.decision.unwrap().decision,
        EnvironmentRequestDecisionKind::Approve
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.creates
            .submit(
                request(1),
                Source {
                    fail: true,
                    ..source.clone()
                }
            )
            .unwrap()
            .wait()
            .await
            .unwrap(),
        result
    );
    assert_eq!(
        source.calls.load(Ordering::SeqCst),
        1,
        "reconciliation must not refresh a moving selector"
    );
}

#[tokio::test]
async fn contradictory_intent_is_refused_without_resolving_or_modifying_the_saved_result() {
    let f = Fixture::new();
    let source = Source::default();
    let first = f
        .creates
        .submit(request(2), source.clone())
        .unwrap()
        .wait()
        .await
        .unwrap();
    for changed in [true, false] {
        let mut other = request(2);
        if changed {
            other
                .specification
                .variables
                .insert("EDITOR".into(), "different".into());
        } else {
            other.reason = "different meaning".into();
        }
        assert_eq!(
            f.creates
                .submit(other, source.clone())
                .unwrap()
                .wait()
                .await
                .unwrap_err(),
            EnvironmentMutationError::Conflict
        );
    }
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.revisions
            .load_current(&first.environment_id)
            .await
            .unwrap()
            .document
            .specification
            .variables["EDITOR"],
        "vim"
    );
}

#[tokio::test]
async fn concurrent_submissions_share_request_lock_and_dropping_a_response_keeps_publication_alive() {
    let f = Fixture::new();
    let pause = Arc::new((tokio::sync::Notify::new(), tokio::sync::Notify::new()));
    let source = Source {
        pause: Some(pause.clone()),
        ..Source::default()
    };
    let first = f.creates.submit(request(3), source.clone()).unwrap();
    pause.0.notified().await;
    let second = f.creates.submit(request(3), source.clone()).unwrap();
    drop(first);
    pause.1.notify_one();
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), second.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.application, RequestApplicationState::Applied);
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    assert!(f.revisions.load_current(&result.environment_id).await.is_ok());
}

#[tokio::test]
async fn source_failure_never_creates_an_approval_or_revision() {
    let f = Fixture::new();
    let source = Source {
        fail: true,
        ..Source::default()
    };
    assert_eq!(
        f.creates.submit(request(4), source).unwrap().wait().await.unwrap_err(),
        EnvironmentMutationError::Resolution(EnvironmentResolutionError::Unavailable)
    );
    assert!(f
        .revisions
        .ledger
        .environment_request_history(&id(4))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn creation_reconciles_after_reopening_the_host_ledger_without_resolving_again() {
    let f = Fixture::new();
    let original = f
        .creates
        .submit(request(5), Source::default())
        .unwrap()
        .wait()
        .await
        .unwrap();
    let Fixture {
        _root: root,
        creates,
        revisions,
    } = f;
    drop(creates);
    drop(revisions);
    let ledger = Arc::new(DbHandle::open(&root.path().join("host.db")).unwrap());
    let files = Arc::new(EnvironmentFiles::open(&root.path().join("environments")).unwrap());
    let revisions = Arc::new(EnvironmentRevisions::new(files, ledger));
    let creates = Arc::new(EnvironmentMutationCoordinator::new(revisions.clone()));
    let unavailable = Source {
        fail: true,
        ..Source::default()
    };
    assert_eq!(
        creates
            .submit(request(5), unavailable.clone())
            .unwrap()
            .wait()
            .await
            .unwrap(),
        original
    );
    assert_eq!(unavailable.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        revisions
            .load_current(&original.environment_id)
            .await
            .unwrap()
            .revision
            .revision_id,
        original.result.unwrap().revision_id
    );
}

#[tokio::test]
async fn interrupted_pending_intent_remains_visible_without_automatic_approval_or_application() {
    let f = Fixture::new();
    let request = request(6);
    let specification = resolve_environment(
        request.specification.clone(),
        None,
        EnvironmentResolutionMode::Edit,
        &Source::default(),
    )
    .await
    .unwrap();
    let at = now().unwrap();
    f.creates
        .write(WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
            request_id: request.request_id.clone(),
            environment_id: id(60),
            base_revision_id: None,
            requester: None,
            timestamp_unix_ms: at,
            expires_at_unix_ms: at + 120000,
            actor: "operator".into(),
            reason: request.reason.clone(),
            payload_event_id: "abcdef012345".into(),
            payload: Box::new(EnvironmentRequestPayload::Change {
                specification: Box::new(specification),
            }),
        }))
        .await
        .unwrap();
    let source = Source {
        fail: true,
        ..Source::default()
    };
    let result = f.creates.submit(request, source.clone()).unwrap().wait().await.unwrap();
    assert_eq!(result.decision, RequestState::Pending);
    assert_eq!(result.application, RequestApplicationState::Pending);
    assert!(result.result.is_none());
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
    assert!(f
        .revisions
        .ledger
        .environment_revisions(EnvironmentRevisionQuery::ForRequest { request_id: id(6) })
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn abandoned_pending_creation_expires_without_approval_or_source_refresh() {
    let f = Fixture::new();
    let request = request(7);
    let specification = resolve_environment(
        request.specification.clone(),
        None,
        EnvironmentResolutionMode::Edit,
        &Source::default(),
    )
    .await
    .unwrap();
    let at = now().unwrap();
    f.creates
        .write(WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
            request_id: request.request_id.clone(),
            environment_id: id(70),
            base_revision_id: None,
            requester: None,
            timestamp_unix_ms: at - 120000,
            expires_at_unix_ms: at - 1,
            actor: "operator".into(),
            reason: request.reason.clone(),
            payload_event_id: "abcdef012345".into(),
            payload: Box::new(EnvironmentRequestPayload::Change {
                specification: Box::new(specification),
            }),
        }))
        .await
        .unwrap();
    let source = Source {
        fail: true,
        ..Source::default()
    };
    let result = f.creates.submit(request, source.clone()).unwrap().wait().await.unwrap();
    assert_eq!(result.decision, RequestState::Expired);
    assert_eq!(result.application, RequestApplicationState::Pending);
    assert!(result.result.is_none());
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
    let saved = f
        .revisions
        .ledger
        .environment_request_snapshot(&id(7))
        .await
        .unwrap()
        .unwrap();
    assert!(saved.record.decision.is_none());
    assert_eq!(
        saved.record.invalidation.unwrap().kind,
        EnvironmentRequestInvalidationKind::Expired
    );
}

mod changes;
