//! Real inherited grant + typed descriptor + service route + durable ledger.
use super::*;
use capsem_core::net::policy_config::{ActivePolicyFile, SettingsFile};
use capsem_core::net::upstream_grant::UpstreamGrantClient;
use capsem_core::security_engine::approval::client::ApprovalCapability;
use capsem_proto::gateway_grant::events::{SessionEvent, FRAME_BYTES, PATH};
use http_body_util::BodyExt;
use std::sync::atomic::{AtomicUsize, Ordering};

async fn next_approval_event(body: &mut axum::body::Body, id: &str) {
    let mut received = Vec::with_capacity(FRAME_BYTES);
    while received.len() < FRAME_BYTES {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), body.frame())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        received.extend_from_slice(&frame.into_data().unwrap());
    }
    let id = uuid::Uuid::parse_str(id).unwrap();
    assert_eq!(
        SessionEvent::decode(received.try_into().unwrap()),
        Some(SessionEvent::ApprovalPending(*id.as_bytes()))
    );
}

#[tokio::test]
async fn inherited_capability_releases_original_work_once_after_scoped_route_audit() {
    for approve in [true, false] {
        let (runtime, session, _db, authority, dir) = fixture().await;
        let mut state = crate::tests::make_test_state_owned();
        state.approvals = runtime;
        let id = session.binding.session_id.clone();
        crate::tests::insert_fake_instance_with_session_dir(&state, &id, std::process::id(), dir.path().into());
        let grant = authority.grant();
        {
            let mut instances = state.instances.lock().unwrap();
            let instance = instances.get_mut(&id).unwrap();
            instance.generation = session.binding.generation;
            instance.authority = authority;
            drop(instances);
        }
        state
            .session_db_handles
            .lock()
            .unwrap()
            .insert(id.clone(), Arc::clone(&session.reader));
        let state = Arc::new(state);
        let active =
            ActivePolicyFile::from_settings_and_corp(&SettingsFile::default(), &SettingsFile::default()).unwrap();
        let policy = crate::upstream_broker::BrokerPolicy::new(
            format!("blake3:{}", "a".repeat(64)),
            Arc::new(active.compile_runtime().unwrap()),
        );
        let (pending, _publisher) = crate::upstream_broker::PendingBroker::pair(policy).unwrap();
        let upstream = Arc::new(UpstreamGrantClient::start(pending.worker_stream().unwrap()).unwrap());
        let broker = pending
            .with_approvals(&state, &id, session.binding.generation, dir.path())
            .start(grant);
        let capability = Arc::new(ApprovalCapability::new(Arc::clone(&upstream)));
        use crate::tests::OperatorRequestExt as _;
        let response = crate::gateway_channel::event_routes(Arc::clone(&state))
            .operator_request(axum::http::Request::get(PATH).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let mut events = response.into_body();
        let effects = Arc::new(AtomicUsize::new(0));
        let effect = Arc::clone(&effects);
        let client = Arc::clone(&capability);
        let original = tokio::spawn(async move {
            let outcome = client.decide(request()).await;
            if outcome.is_allowed() {
                effect.fetch_add(1, Ordering::SeqCst);
            }
            outcome
        });
        next_approval_event(&mut events, &id).await;
        assert_eq!(state.pending_approval_count(&id), 1);
        assert_eq!(
            effects.load(Ordering::SeqCst),
            0,
            "work escaped before a human decision"
        );
        let (status, receipt) = crate::tests::route_request(
            crate::router_runtime::build_service_router(Arc::clone(&state)), axum::http::Method::POST,
            &format!("/vms/{id}/security/asks/0123456789ab/resolve"),
            Some(serde_json::json!({"generation":session.binding.generation.to_string(),"decision":if approve {"approve"} else {"deny"}})),
        ).await;
        assert_eq!(status, axum::http::StatusCode::OK, "{receipt}");
        next_approval_event(&mut events, &id).await;
        let outcome = original.await.unwrap();
        assert_eq!(outcome.is_allowed(), approve);
        assert_eq!(effects.load(Ordering::SeqCst), usize::from(approve));
        assert_eq!(state.pending_approval_count(&id), 0);
        assert_eq!(
            session.record("0123456789ab").await.unwrap().unwrap().status,
            if approve {
                SecurityAskStatus::Approved
            } else {
                SecurityAskStatus::Denied
            }
        );
        assert_eq!(
            session
                .resolve(&crate::tests::operator_actor(), "0123456789ab", true)
                .await,
            Err(ResolveError::Missing)
        );
        state.evict_instance(&id, session.binding.generation).unwrap();
        drop(capability);
        drop(upstream);
        assert!(broker.await.unwrap().cleanup_confirmed);
    }
}
