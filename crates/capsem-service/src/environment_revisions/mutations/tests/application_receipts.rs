use super::*;
use capsem_api::requests::RequestListQuery;
use capsem_logger::events::environment::{
    application::{
        EnvironmentApplicationComposition, EnvironmentApplicationMutation, EnvironmentApplicationState,
        EnvironmentSandboxReferences,
    },
    EnvironmentSandboxReferenceKind, EnvironmentSandboxReferenceMutation,
};

async fn apply_record(f: &Fixture, row: EnvironmentApplicationMutation) {
    f.creates.write(WriteOp::EnvironmentApplication(row)).await.unwrap();
}

async fn initial_application(
    f: &Fixture,
    request_id: &str,
    target: EnvironmentRequestTarget,
    base: &EnvironmentRevisionParent,
    desired: &EnvironmentRevisionParent,
    at: i64,
) -> EnvironmentApplicationMutation {
    for kind in [
        EnvironmentSandboxReferenceKind::Boot,
        EnvironmentSandboxReferenceKind::LivePolicy,
        EnvironmentSandboxReferenceKind::Desired,
    ] {
        f.creates
            .write(WriteOp::EnvironmentSandboxReference(
                EnvironmentSandboxReferenceMutation {
                    session_id: target.session_id.clone(),
                    generation: target.generation.clone(),
                    kind,
                    expected: None,
                    revision: Some(base.clone()),
                    timestamp_unix_ms: at,
                    actor: "operator".into(),
                },
            ))
            .await
            .unwrap();
    }
    let mut row = EnvironmentApplicationMutation {
        request_id: request_id.into(),
        target,
        expected_version: None,
        expected_references: EnvironmentSandboxReferences {
            boot: Some(base.clone()),
            live_policy: Some(base.clone()),
            desired: Some(base.clone()),
        },
        timestamp_unix_ms: at,
        actor: "operator".into(),
        state: EnvironmentApplicationState::Pending,
        composition: EnvironmentApplicationComposition {
            boot: base.clone(),
            live_policy: base.clone(),
            desired: desired.clone(),
            corporate_policy_id: "fixture-policy".into(),
            pending_restart: false,
        },
        failure_reason: None,
    };
    apply_record(f, row.clone()).await;
    row.expected_version = Some(1);
    row.expected_references.desired = Some(desired.clone());
    row.state = EnvironmentApplicationState::Applying;
    apply_record(f, row.clone()).await;
    row.expected_version = Some(2);
    row
}

#[tokio::test]
async fn an_approved_apply_remains_pending_until_each_selected_sandbox_acknowledges() {
    let f = Fixture::new();
    let selected = f
        .creates
        .submit(request(1), Source::default())
        .unwrap()
        .wait()
        .await
        .unwrap()
        .result
        .unwrap();
    let revision = EnvironmentRevisionParent {
        environment_id: selected.environment_id.clone(),
        revision_id: selected.revision_id.clone(),
    };
    let original = f
        .creates
        .submit(request(3), Source::default())
        .unwrap()
        .wait()
        .await
        .unwrap()
        .result
        .unwrap();
    let base = EnvironmentRevisionParent {
        environment_id: original.environment_id.clone(),
        revision_id: original.revision_id.clone(),
    };
    let request_id = id(2);
    let at = now().unwrap();
    let reason = "apply the reviewed revision to two exact owners".to_owned();
    let targets = vec![
        EnvironmentRequestTarget {
            session_id: id(10),
            generation: id(11),
        },
        EnvironmentRequestTarget {
            session_id: id(12),
            generation: id(13),
        },
    ];
    f.creates
        .write(WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
            request_id: request_id.clone(),
            environment_id: selected.environment_id.clone(),
            base_revision_id: Some(selected.revision_id.clone()),
            requester: None,
            actor: "operator".into(),
            reason: reason.clone(),
            timestamp_unix_ms: at,
            expires_at_unix_ms: expiry(at).unwrap(),
            payload_event_id: event_id(),
            payload: Box::new(EnvironmentRequestPayload::Apply {
                expected: Default::default(),
                revision: revision.clone(),
                targets: targets.clone(),
            }),
        }))
        .await
        .unwrap();
    f.creates
        .write(WriteOp::EnvironmentRequestDecision(EnvironmentRequestDecision {
            request_id: request_id.clone(),
            actor: "operator".into(),
            reason,
            timestamp_unix_ms: at,
            decision: EnvironmentRequestDecisionKind::Approve,
        }))
        .await
        .unwrap();
    f.creates
        .write(WriteOp::EnvironmentRequestResult(EnvironmentRequestResult {
            request_id: request_id.clone(),
            result: revision.clone(),
            actor: "operator".into(),
            timestamp_unix_ms: at,
        }))
        .await
        .unwrap();

    let receipt = f.creates.request_status(request_id.clone()).await.unwrap();
    assert_eq!(receipt.application, RequestApplicationState::Pending);
    assert_eq!(receipt.result.as_ref(), Some(&selected));
    let audit = f.revisions.review_request(&request_id).await.unwrap();
    assert_eq!(audit.summary.application, RequestApplicationState::Pending);
    assert_eq!(
        audit
            .applications
            .iter()
            .map(|item| item.target.clone())
            .collect::<Vec<_>>(),
        audit.targets
    );
    assert!(audit.applications.iter().all(|item| item.composition.is_none()));

    let mut first = initial_application(&f, &request_id, targets[0].clone(), &base, &revision, at).await;
    assert_eq!(
        f.creates.request_status(request_id.clone()).await.unwrap().application,
        RequestApplicationState::Applying
    );
    first.state = EnvironmentApplicationState::Applied;
    first.composition.live_policy = revision.clone();
    apply_record(&f, first).await;
    assert_eq!(
        f.creates.request_status(request_id.clone()).await.unwrap().application,
        RequestApplicationState::Pending
    );

    let mut second = initial_application(&f, &request_id, targets[1].clone(), &base, &revision, at).await;
    second.state = EnvironmentApplicationState::Failed;
    second.failure_reason = Some("receiver did not acknowledge".into());
    apply_record(&f, second.clone()).await;
    let receipt = f.creates.request_status(request_id.clone()).await.unwrap();
    assert_eq!(receipt.application, RequestApplicationState::Failed);
    assert_eq!(receipt.application_error.as_deref(), second.failure_reason.as_deref());
    let list = f.revisions.list_requests(RequestListQuery::default()).await.unwrap();
    assert_eq!(
        list.requests
            .into_iter()
            .find(|item| item.request_id == request_id)
            .unwrap()
            .application,
        RequestApplicationState::Failed
    );
    let audit = f.revisions.review_request(&request_id).await.unwrap();
    assert_eq!(audit.summary.application, RequestApplicationState::Failed);
    assert_eq!(audit.applications[0].state, RequestApplicationState::Applied);
    assert_eq!(audit.applications[1].state, RequestApplicationState::Failed);
    let prior = audit.applications[1].composition.as_ref().unwrap();
    assert_eq!(
        (&prior.boot, &prior.live_policy, &prior.desired),
        (&original, &original, &selected)
    );

    second.state = EnvironmentApplicationState::Applying;
    second.expected_version = Some(3);
    second.failure_reason = None;
    apply_record(&f, second.clone()).await;
    second.state = EnvironmentApplicationState::Applied;
    second.expected_version = Some(4);
    second.composition.live_policy = revision;
    apply_record(&f, second).await;
    let audit = f.revisions.review_request(&request_id).await.unwrap();
    assert_eq!(audit.summary.application, RequestApplicationState::Applied);
    assert!(audit
        .applications
        .iter()
        .all(|item| item.state == RequestApplicationState::Applied));
}
