use crate::app::ControlAction;
use crate::gateway_provider::GatewayProvider;
use crate::tests::{read_http_request, write_json_response, write_response};

async fn invoke_with_response(
    action: ControlAction,
    body: &'static str,
    path: &'static str,
) -> anyhow::Result<crate::gateway_provider::ActionOutcome> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut stream).await;
            if request.contains("GET /token ") {
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else {
                assert!(request.contains(path), "{request}");
                assert!(request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer test-token"));
                write_json_response(&mut stream, body).await;
            }
        }
    });
    let outcome = GatewayProvider::new(url).invoke_async(&action).await;
    server.await.unwrap();
    outcome
}

#[tokio::test]
async fn resume_suspend_and_delete_keep_tui_focus_and_labels() {
    let id = "vm-1".to_string();
    let label = "workspace".to_string();
    let outcome = invoke_with_response(
        ControlAction::Resume {
            id: id.clone(),
            label: label.clone(),
        },
        r#"{"id":"vm-1","name":"workspace","profile_id":"code","status":"Running","available_actions":[]}"#,
        "POST /vms/vm-1/resume ",
    )
    .await
    .unwrap();
    assert_eq!(outcome.message, "resumed workspace");
    assert_eq!(outcome.focus_session.as_deref(), Some("vm-1"));
    let outcome = invoke_with_response(
        ControlAction::Suspend {
            id: id.clone(),
            label: label.clone(),
        },
        r#"{"success":true}"#,
        "POST /vms/vm-1/pause ",
    )
    .await
    .unwrap();
    assert_eq!(outcome.message, "suspended workspace");
    assert_eq!(outcome.focus_session.as_deref(), Some("vm-1"));
    let outcome = invoke_with_response(
        ControlAction::Delete { id, label },
        r#"{"success":true}"#,
        "DELETE /vms/vm-1/delete ",
    )
    .await
    .unwrap();
    assert_eq!(outcome.message, "deleted workspace");
    assert_eq!(outcome.focus_session, None);
}

#[tokio::test]
async fn malformed_success_payload_cannot_claim_an_action_succeeded() {
    let error = invoke_with_response(
        ControlAction::Stop {
            id: "vm-1".into(),
            label: "workspace".into(),
        },
        r#"{"success":true}"#,
        "POST /vms/vm-1/stop ",
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("persistent"));
}

#[tokio::test]
async fn gateway_provider_invokes_stop_over_authenticated_gateway() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test gateway");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let request = read_http_request(&mut stream).await;
            if request.contains("GET /token ") {
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else {
                assert!(
                    request.contains("POST /vms/vm-1/stop "),
                    "unexpected request: {request:?}"
                );
                assert!(
                    request.contains("authorization: Bearer test-token")
                        || request.contains("Authorization: Bearer test-token"),
                    "missing bearer auth: {request:?}"
                );
                write_json_response(&mut stream, r#"{"success":true,"persistent":true}"#).await;
            }
        }
    });

    let outcome = GatewayProvider::new(format!("http://{addr}"))
        .invoke_async(&ControlAction::Stop {
            id: "vm-1".to_string(),
            label: "profile-main".to_string(),
        })
        .await
        .expect("invoke stop");

    assert_eq!(outcome.message, "stopped profile-main");
    server.await.expect("server task");
}

#[tokio::test]
async fn gateway_provider_invokes_named_profile_create_over_authenticated_gateway() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test gateway");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let request = read_http_request(&mut stream).await;
            if request.contains("GET /token ") {
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else {
                assert!(request.contains("POST /vms/create "), "unexpected request: {request:?}");
                assert!(request.contains(r#""name":"code-1-proof""#));
                assert!(request.contains(r#""persistent":true"#));
                assert!(request.contains(r#""profile_id":"co-work""#));
                write_json_response(&mut stream, r#"{"id":"code-1-proof","name":"code-1-proof","profile_id":"co-work","status":"Running","available_actions":[]}"#).await;
            }
        }
    });

    let outcome = GatewayProvider::new(format!("http://{addr}"))
        .invoke_async(&ControlAction::CreateSession {
            name: Some("code-1-proof".to_string()),
            profile_id: "co-work".to_string(),
        })
        .await
        .expect("invoke create");

    assert_eq!(outcome.message, "created code-1-proof");
    assert_eq!(outcome.focus_session.as_deref(), Some("code-1-proof"));
    server.await.expect("server task");
}

#[tokio::test]
async fn gateway_provider_preserves_service_owned_persistent_names() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test gateway");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let request = read_http_request(&mut stream).await;
            if request.contains("GET /token ") {
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else {
                assert!(request.contains("POST /vms/create "), "unexpected request: {request:?}");
                assert!(request.contains(r#""name":null"#), "{request}");
                assert!(request.contains(r#""persistent":true"#));
                assert!(request.contains(r#""profile_id":"code""#));
                write_json_response(
                    &mut stream,
                    r#"{"id":"code-7","name":"code-7","profile_id":"code","status":"Running","available_actions":[]}"#,
                )
                .await;
            }
        }
    });

    let outcome = GatewayProvider::new(format!("http://{addr}"))
        .invoke_async(&ControlAction::CreateSession {
            name: None,
            profile_id: "code".to_string(),
        })
        .await
        .expect("invoke create");

    assert_eq!(outcome.message, "created session");
    assert_eq!(outcome.focus_session.as_deref(), Some("code-7"));
    server.await.expect("server task");
}

#[tokio::test]
async fn gateway_provider_invokes_fork_over_authenticated_gateway() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test gateway");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let request = read_http_request(&mut stream).await;
            if request.contains("GET /token ") {
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else {
                assert!(
                    request.contains("POST /vms/profile-v2/fork "),
                    "unexpected request: {request:?}"
                );
                assert!(request.contains(r#""name":"profile-v2-fork-copy""#));
                write_json_response(
                    &mut stream,
                    r#"{"id":"fork-id","name":"profile-v2-fork-copy","size_bytes":1024}"#,
                )
                .await;
            }
        }
    });

    let outcome = GatewayProvider::new(format!("http://{addr}"))
        .invoke_async(&ControlAction::Fork {
            id: "profile-v2".to_string(),
            name: "profile-v2-fork-copy".to_string(),
        })
        .await
        .expect("invoke fork");

    assert_eq!(outcome.message, "forked profile-v2-fork-copy");
    assert_eq!(outcome.focus_session.as_deref(), Some("fork-id"));
    server.await.expect("server task");
}

#[tokio::test]
async fn gateway_provider_invokes_checkpoint_over_suspend_endpoint() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test gateway");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let request = read_http_request(&mut stream).await;
            if request.contains("GET /token ") {
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else {
                assert!(
                    request.contains("POST /vms/vm-1/pause "),
                    "unexpected request: {request:?}"
                );
                write_json_response(&mut stream, r#"{"success":true}"#).await;
            }
        }
    });

    let outcome = GatewayProvider::new(format!("http://{addr}"))
        .invoke_async(&ControlAction::Checkpoint {
            id: "vm-1".to_string(),
            label: "profile-main".to_string(),
        })
        .await
        .expect("invoke checkpoint");

    assert_eq!(outcome.message, "checkpointed profile-main");
    server.await.expect("server task");
}

#[tokio::test]
async fn gateway_provider_invokes_purge_over_authenticated_gateway() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test gateway");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let request = read_http_request(&mut stream).await;
            if request.contains("GET /token ") {
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else {
                assert!(request.contains("POST /purge "), "unexpected request: {request:?}");
                assert!(
                    request.contains("authorization: Bearer test-token")
                        || request.contains("Authorization: Bearer test-token"),
                    "missing bearer auth: {request:?}"
                );
                assert!(request.contains(r#""all":false"#));
                write_json_response(
                    &mut stream,
                    r#"{"purged":3,"persistent_purged":0,"ephemeral_purged":3}"#,
                )
                .await;
            }
        }
    });

    let outcome = GatewayProvider::new(format!("http://{addr}"))
        .invoke_async(&ControlAction::Purge { all: false })
        .await
        .expect("invoke purge");

    assert_eq!(outcome.message, "purged 3 temporary sessions");
    assert_eq!(outcome.focus_session, None);
    server.await.expect("server task");
}

#[tokio::test]
async fn gateway_provider_reports_defunct_persistent_purge() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test gateway");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let request = read_http_request(&mut stream).await;
            if request.contains("GET /token ") {
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else {
                assert!(request.contains("POST /purge "), "unexpected request: {request:?}");
                assert!(request.contains(r#""all":false"#));
                write_json_response(
                    &mut stream,
                    r#"{"purged":2,"persistent_purged":1,"ephemeral_purged":1}"#,
                )
                .await;
            }
        }
    });

    let outcome = GatewayProvider::new(format!("http://{addr}"))
        .invoke_async(&ControlAction::Purge { all: false })
        .await
        .expect("invoke purge");

    assert_eq!(outcome.message, "purged 2 sessions (1 broken persistent, 1 temporary)");
    server.await.expect("server task");
}

#[tokio::test]
async fn gateway_provider_surfaces_action_error_body() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test gateway");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let request = read_http_request(&mut stream).await;
            if request.contains("GET /token ") {
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else {
                assert!(
                    request.contains("DELETE /vms/vm-1/delete "),
                    "unexpected request: {request:?}"
                );
                write_response(&mut stream, "500 Internal Server Error", r#"{"error":"boom"}"#).await;
            }
        }
    });

    let error = GatewayProvider::new(format!("http://{addr}"))
        .invoke_async(&ControlAction::Delete {
            id: "vm-1".to_string(),
            label: "profile-main".to_string(),
        })
        .await
        .expect_err("delete should fail");

    assert!(error.to_string().contains("500"));
    assert!(error.to_string().contains("boom"));
    server.await.expect("server task");
}
