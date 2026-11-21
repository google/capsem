use super::*;
use crate::tests::OperatorRequestExt as _;
use axum::http::Request;
use http_body_util::BodyExt;
use tower::ServiceExt as _;

#[tokio::test]
async fn feed_requires_a_private_seat_and_excludes_concurrent_subscriptions() {
    let service = crate::tests::make_test_state();
    let routes = event_routes(service.clone());
    let request = || Request::get(PATH).body(Body::empty()).unwrap();
    let ordinary = crate::router_runtime::build_service_router(service)
        .operator_request(request())
        .await
        .unwrap();
    assert_eq!(ordinary.status(), StatusCode::NOT_FOUND);

    let direct = routes.clone().oneshot(request()).await.unwrap();
    assert!(!direct.status().is_success());
    let open = routes.clone().operator_request(request()).await.unwrap();
    assert_eq!(open.status(), StatusCode::OK);
    assert_eq!(open.headers()[header::CONTENT_TYPE], CONTENT_TYPE);
    let duplicate = routes.operator_request(request()).await.unwrap();
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    drop(open);
}

#[tokio::test]
async fn feed_drops_on_gateway_control_revocation() {
    let service = crate::tests::make_test_state();
    let control = crate::instance::WorkerAuthority::default();
    let socket = super::super::prepare(event_routes(service), control.grant()).unwrap();
    let socket = tokio::net::UnixStream::from_std(socket).unwrap();
    let (mut client, connection) = hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(socket))
        .await
        .unwrap();
    let task = tokio::spawn(connection);
    let response = client
        .send_request(
            Request::get(PATH)
                .header(header::HOST, "localhost")
                .header(header::AUTHORIZATION, format!("Bearer {}", "A".repeat(64)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    control.revoke();
    let next = tokio::time::timeout(std::time::Duration::from_secs(5), body.frame())
        .await
        .unwrap();
    assert!(
        next.is_none_or(|frame| frame.is_err()),
        "feed survived control revocation"
    );
    let _ = task.await;
}

#[test]
fn a_notification_cannot_be_retargeted_to_a_replacement_generation() {
    let service = crate::tests::make_test_state();
    let id = capsem_crypto::uuid_v4();
    let session_id = id.to_string();
    crate::tests::insert_fake_instance_with_session_dir(
        &service,
        &session_id,
        std::process::id(),
        service.run_dir.clone(),
    );
    let generation = service.instances.lock().unwrap()[&session_id].generation;
    let binding = ApprovalBinding {
        session_id: session_id.clone(),
        generation,
    };
    assert_eq!(
        approval_event(&service, &binding),
        Some(SessionEvent::ApprovalPending(*id.as_bytes()))
    );
    service
        .instances
        .lock()
        .unwrap()
        .get_mut(&session_id)
        .unwrap()
        .generation = capsem_crypto::uuid_v4();
    assert_eq!(approval_event(&service, &binding), None);
}
