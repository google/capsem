use super::*;
use crate::instance::WorkerAuthority;
use capsem_core::security_engine::approval::ApprovalLimits;

mod capability;
mod registration;

async fn fixture() -> (
    Runtime,
    Arc<Session>,
    capsem_logger::DbHandle,
    WorkerAuthority,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let writer = Arc::new(DbWriter::open(&path, 16).unwrap());
    let db = capsem_logger::DbHandle::open_external_reader(&path).unwrap();
    let runtime = Runtime {
        broker: ApprovalBroker::new(ApprovalLimits {
            timeout: std::time::Duration::from_secs(1),
            ..ApprovalLimits::default()
        })
        .unwrap(),
        ..Runtime::default()
    };
    let authority = WorkerAuthority::default();
    let binding = ApprovalBinding {
        session_id: capsem_crypto::uuid_v4().to_string(),
        generation: capsem_crypto::uuid_v4(),
    };
    let session = runtime
        .install(
            binding,
            writer,
            Arc::new(SessionLedger::embedded(path, db.clone())),
            authority.grant(),
        )
        .unwrap();
    let event = SecurityAskEvent::pending(capsem_logger::SecurityAskPending {
        timestamp_unix_ms: now_ms(),
        ask_id: "0123456789ab".into(),
        event_id: "0123456789ac".into(),
        event_type: "process.exec".into(),
        rule_id: "test.ask".into(),
        rule_name: "Ask".into(),
        rule_json: "{}".into(),
        event_json: r#"{"event_type":"process.exec","process":{"command":"fixture-secret"}}"#.into(),
    });
    session
        .writer
        .write_checked(WriteOp::SecurityAskEvent(event))
        .await
        .unwrap();
    session.writer.flush_checked().await.unwrap();
    (runtime, session, db, authority, dir)
}

fn request() -> ApprovalRequest {
    ApprovalRequest {
        ask_id: "0123456789ab".into(),
        event_id: "0123456789ac".into(),
        event_type: "process.exec".into(),
        rule_id: "test.ask".into(),
        rule_name: "Ask".into(),
        summary: "Run a command".into(),
        held_bytes: 64,
    }
}

#[tokio::test]
async fn unverified_reservations_are_not_actionable_or_visible_to_resolvers() {
    let (runtime, session, _db, _authority, _dir) = fixture().await;
    let _pending = session.reserve(&request()).unwrap();
    assert_eq!(runtime.broker.pending_count(&session.binding), 0);
    assert!(runtime.broker.active(&session.binding, "0123456789ab").is_none());
    assert_eq!(session.resolve("0123456789ab", true).await, Err(ResolveError::Missing));
}

#[tokio::test]
async fn human_decision_is_flushed_before_original_waiter_resumes_once() {
    for approve in [true, false] {
        let (runtime, session, _db, _authority, _dir) = fixture().await;
        let pending = session.begin(request()).await.unwrap();
        assert_eq!(runtime.broker.pending_count(&session.binding), 1);
        session.resolve("0123456789ab", approve).await.unwrap();
        let record = session.record("0123456789ab").await.unwrap().unwrap();
        assert_eq!(
            record.status,
            if approve {
                SecurityAskStatus::Approved
            } else {
                SecurityAskStatus::Denied
            }
        );
        assert!(!serde_json::to_string(&record).unwrap().contains("fixture-secret"));
        assert_eq!(session.wait("0123456789ab", pending).await.is_allowed(), approve);
        assert_eq!(runtime.broker.pending_count(&session.binding), 0);
        assert_eq!(session.resolve("0123456789ab", true).await, Err(ResolveError::Missing));
    }
}

#[tokio::test]
async fn retired_generation_cannot_resolve_or_resume_an_original_operation() {
    let (runtime, session, _db, authority, _dir) = fixture().await;
    let pending = session.begin(request()).await.unwrap();
    let mut wrong = session.binding.clone();
    wrong.generation = capsem_crypto::uuid_v4();
    assert!(runtime.get(&wrong).is_none());
    authority.revoke();
    runtime.retire(&session.binding);
    assert_eq!(session.resolve("0123456789ab", true).await, Err(ResolveError::Conflict));
    assert_eq!(session.wait("0123456789ab", pending).await, ApprovalOutcome::Cancelled);
    assert_eq!(
        session.record("0123456789ab").await.unwrap().unwrap().status,
        SecurityAskStatus::Denied
    );
    assert!(runtime.get(&session.binding).is_none());
    runtime.finish(&session.binding).await.unwrap();
    assert!(
        session
            .append_resolution("0123456789ab", SecurityAskStatus::Denied, "request_cancelled")
            .await
            .is_err(),
        "retired producer remained open"
    );
}

#[tokio::test(start_paused = true)]
async fn cancellation_after_deadline_keeps_the_expiry_audit_reason() {
    let (runtime, session, _db, _authority, _dir) = fixture().await;
    let pending = session.begin(request()).await.unwrap();
    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    runtime.broker.cancel(&session.binding, "0123456789ab");
    assert_eq!(session.wait("0123456789ab", pending).await, ApprovalOutcome::Expired);
    let record = session.record("0123456789ab").await.unwrap().unwrap();
    assert_eq!(record.status, SecurityAskStatus::Denied);
    assert_eq!(record.reason.as_deref(), Some("deadline"));
    assert_eq!(runtime.broker.pending_count(&session.binding), 0);
}

#[tokio::test(start_paused = true)]
async fn deadline_denies_and_appends_terminal_audit_without_an_approval_poll() {
    let (runtime, session, _db, _authority, _dir) = fixture().await;
    let pending = session.begin(request()).await.unwrap();
    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    assert_eq!(session.wait("0123456789ab", pending).await, ApprovalOutcome::Expired);
    let record = session.record("0123456789ab").await.unwrap().unwrap();
    assert_eq!(record.status, SecurityAskStatus::Denied);
    assert_eq!(record.reason.as_deref(), Some("deadline"));
    assert_eq!(runtime.broker.pending_count(&session.binding), 0);
}

#[tokio::test]
async fn mismatched_operation_cannot_create_an_actionable_continuation() {
    let (runtime, session, _db, _authority, _dir) = fixture().await;
    let mut wrong = request();
    wrong.event_id = "aaaaaaaaaaaa".into();
    assert!(session.begin(wrong).await.is_err());
    assert_eq!(runtime.broker.pending_count(&session.binding), 0);
}

#[tokio::test]
async fn mounted_approval_routes_bind_session_generation_and_reject_repeated_decisions() {
    use crate::router_runtime::build_service_router;
    use crate::tests::{insert_fake_instance_with_session_dir, make_test_state_owned, route_request};
    use axum::http::{Method, StatusCode};
    use serde_json::json;

    let (runtime, session, _db, _authority, dir) = fixture().await;
    let pending = session.begin(request()).await.unwrap();
    // History cannot push the original live operation out of a bounded page.
    session
        .writer
        .write_checked(WriteOp::SecurityAskEvent(SecurityAskEvent::pending(
            capsem_logger::SecurityAskPending {
                timestamp_unix_ms: now_ms() + 1000,
                ask_id: "ffffffffffff".into(),
                event_id: "eeeeeeeeeeee".into(),
                event_type: "process.exec".into(),
                rule_id: "test.ask".into(),
                rule_name: "Ask".into(),
                rule_json: "{}".into(),
                event_json: "{}".into(),
            },
        )))
        .await
        .unwrap();
    session.writer.flush_checked().await.unwrap();
    let mut state = make_test_state_owned();
    state.approvals = runtime;
    let id = session.binding.session_id.clone();
    insert_fake_instance_with_session_dir(&state, &id, std::process::id(), dir.path().into());
    state.instances.lock().unwrap().get_mut(&id).unwrap().generation = session.binding.generation;
    state
        .session_db_handles
        .lock()
        .unwrap()
        .insert(id.clone(), Arc::clone(&session.reader));
    let state = Arc::new(state);
    let route = format!("/vms/{id}/security/asks");
    let (status, body) = route_request(
        build_service_router(Arc::clone(&state)),
        Method::GET,
        &format!("{route}?limit=1"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["pending_count"], 1);
    assert_eq!(body["requests"][0]["status"], "pending");
    assert_eq!(body["requests"][0]["ask_id"], "0123456789ab");
    assert_eq!(body["requests"].as_array().unwrap().len(), 1);
    assert!(!body.to_string().contains("fixture-secret"));
    let resolve = format!("{route}/0123456789ab/resolve");
    let decision = json!({"generation":session.binding.generation.to_string(),"decision":"approve"});
    let wrong = json!({"generation":capsem_crypto::uuid_v4().to_string(),"decision":"approve"});
    let (status, _) = route_request(
        build_service_router(Arc::clone(&state)),
        Method::POST,
        &resolve,
        Some(wrong),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, receipt) = route_request(
        build_service_router(Arc::clone(&state)),
        Method::POST,
        &resolve,
        Some(decision.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert_eq!(receipt["status"], "approved");
    assert_eq!(session.wait("0123456789ab", pending).await, ApprovalOutcome::Approved);
    let (status, _) = route_request(
        build_service_router(Arc::clone(&state)),
        Method::POST,
        &resolve,
        Some(decision.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let missing = format!("/vms/{}/security/asks/0123456789ab/resolve", capsem_crypto::uuid_v4());
    let (status, _) = route_request(build_service_router(state), Method::POST, &missing, Some(decision)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
