use super::*;
use capsem_logger::events::environment::request::*;

async fn approved(f: &Fixture, request: u64, env: u64, rev: u64, previous: Option<u64>) -> PreparedEnvironmentRevision {
    let mut result = revision(env, rev, previous);
    result.request_id = Some(id(request));
    result.timestamp_unix_ms = 30;
    for operation in [
        WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
            request_id: id(request),
            environment_id: id(env),
            base_revision_id: previous.map(id),
            requester: None,
            timestamp_unix_ms: 20,
            expires_at_unix_ms: 1000,
            actor: "operator".into(),
            reason: "exact reviewed change".into(),
            payload_event_id: format!("{request:012x}"),
            payload: Box::new(EnvironmentRequestPayload::Change {
                specification: result.specification.clone(),
            }),
        }),
        WriteOp::EnvironmentRequestDecision(EnvironmentRequestDecision {
            request_id: id(request),
            timestamp_unix_ms: 21,
            actor: "operator".into(),
            decision: EnvironmentRequestDecisionKind::Approve,
            reason: "approved exact change".into(),
        }),
    ] {
        f.owner.ledger.write(operation).await.unwrap();
        f.owner.ledger.flush().await.unwrap();
    }
    result
}
async fn result(f: &Fixture, request: u64) -> Option<EnvironmentRevisionParent> {
    f.owner
        .ledger
        .environment_request_snapshot(&id(request))
        .await
        .unwrap()
        .unwrap()
        .record
        .result
}

#[tokio::test]
async fn request_backed_commits_bind_the_exact_result_receipt_and_retry_idempotently() {
    let f = Fixture::new();
    let revision = approved(&f, 40, 1, 3, None).await;
    let committed = f.owner.commit(revision.clone(), 40).wait().await.unwrap();
    assert_eq!(
        result(&f, 40).await,
        Some(EnvironmentRevisionParent {
            environment_id: id(1),
            revision_id: id(3)
        })
    );
    assert_eq!(f.owner.commit(revision.clone(), 41).wait().await.unwrap(), committed);
    assert_eq!(
        f.owner.load_current(&id(1)).await.unwrap().document,
        document(&revision)
    );
    assert_eq!(
        f.owner
            .ledger
            .environment_revisions(EnvironmentRevisionQuery::ForRequest { request_id: id(40) })
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn startup_repairs_committed_request_receipts_without_reapplying_other_work() {
    let f = Fixture::new();
    for (request, env, rev) in [(40, 1, 3), (41, 8, 9)] {
        let prepared = approved(&f, request, env, rev, None).await;
        f.prepare(prepared.clone()).await;
        f.owner.files.publish_document(document(&prepared)).await.unwrap();
        f.owner
            .transition(&id(env), &id(rev), EnvironmentRevisionState::Committed, 40, "operator")
            .await
            .unwrap();
        assert!(result(&f, request).await.is_none(), "crash before final receipt");
    }
    f.owner.commit(revision(10, 11, None), 15).wait().await.unwrap();
    let unapplied = approved(&f, 42, 10, 12, Some(11)).await;
    f.prepare(unapplied).await;
    let never_prepared = approved(&f, 43, 20, 21, None).await;
    let missing_file = approved(&f, 44, 30, 31, None).await;
    f.prepare(missing_file).await;
    let summary = f.owner.recover_pending_owned(50, 1).await.unwrap();
    assert_eq!(summary.recovered_unapplied, 1);
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.receipts_bound, 2);
    for (request, env, rev) in [(40, 1, 3), (41, 8, 9)] {
        assert_eq!(
            result(&f, request).await,
            Some(EnvironmentRevisionParent {
                environment_id: id(env),
                revision_id: id(rev)
            })
        );
        assert_eq!(
            f.owner.load_current(&id(env)).await.unwrap().revision.revision_id,
            id(rev)
        );
    }
    for request in [42, 43, 44] {
        assert!(result(&f, request).await.is_none());
    }
    assert_eq!(
        f.owner.load_current(&id(10)).await.unwrap().revision.revision_id,
        id(11)
    );
    assert!(f
        .owner
        .ledger
        .environment_revisions(EnvironmentRevisionQuery::ForRequest {
            request_id: never_prepared.request_id.unwrap()
        })
        .await
        .unwrap()
        .is_empty());
    let again = f.owner.recover_pending_owned(51, 1).await.unwrap();
    assert_eq!(again.failed, 1);
    assert_eq!(again.receipts_bound, 0);
    assert_eq!(
        f.owner.load_current(&id(10)).await.unwrap().revision.revision_id,
        id(11)
    );
}
