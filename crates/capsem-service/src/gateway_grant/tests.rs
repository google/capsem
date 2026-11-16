use super::*;
use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_proto::gateway_grant::{decode_gateway_grant_response, encode_gateway_grant_request, GatewayGrantRequest};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn grants_only_the_service_and_registry_derived_owner_seats() {
    let state = crate::tests::make_test_state();
    let (client, server) = std::os::unix::net::UnixStream::pair().unwrap();
    let broker = tokio::spawn(serve(server, Arc::clone(&state)));
    let requests =
        DescriptorSender::<GATEWAY_GRANT_FRAME_SIZE, GATEWAY_GRANT_MAX_FDS>::new(client.try_clone().unwrap()).unwrap();
    let responses = DescriptorReceiver::<GATEWAY_GRANT_FRAME_SIZE, GATEWAY_GRANT_MAX_FDS>::new(client).unwrap();

    let register =
        capsem_proto::gateway_grant::auth::encode_gateway_auth_request(&GatewayAuthRequest::RegisterOperator {
            request_id: 1,
            token: capsem_proto::gateway_grant::auth::GatewayAuthSecret::from_bytes([b'A'; 64]),
        })
        .unwrap();
    requests.send(&register, &[]).await.unwrap();
    let registered = responses.recv().await.unwrap();
    assert!(registered.fds.is_empty());
    assert_eq!(
        capsem_proto::gateway_grant::auth::decode_gateway_auth_response(&registered.bytes).unwrap(),
        GatewayAuthResponse::RegisteredOperator { request_id: 1 }
    );
    let request = encode_gateway_grant_request(&GatewayGrantRequest::OpenService { request_id: 2 }).unwrap();
    requests.send(&request, &[]).await.unwrap();
    let response = responses.recv().await.unwrap();
    assert!(matches!(
        decode_gateway_grant_response(&response.bytes).unwrap(),
        GatewayGrantResponse::Granted {
            request_id: 2,
            kind: GatewayGrantKind::Service
        }
    ));
    requests
        .send(
            &encode_gateway_grant_request(&GatewayGrantRequest::Adopted { request_id: 2 }).unwrap(),
            &[],
        )
        .await
        .unwrap();
    let mut granted = tokio::net::UnixStream::from_std(std::os::unix::net::UnixStream::from(
        response.fds.into_iter().next().unwrap(),
    ))
    .unwrap();
    let request = format!("GET /internal/gateway/status HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n", "A".repeat(64));
    granted.write_all(request.as_bytes()).await.unwrap();
    let mut reply = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), granted.read_to_end(&mut reply))
        .await
        .unwrap()
        .unwrap();
    let reply = String::from_utf8(reply).unwrap();
    assert!(reply.starts_with("HTTP/1.1 200 OK\r\n"), "{reply}");
    assert!(reply.contains("\"sandboxes\""), "{reply}");

    let request = encode_gateway_grant_request(&GatewayGrantRequest::OpenOwnerHandoff {
        request_id: 3,
        vm_id: "not-running".into(),
    })
    .unwrap();
    requests.send(&request, &[]).await.unwrap();
    let response = responses.recv().await.unwrap();
    assert!(response.fds.is_empty());
    assert!(matches!(
        decode_gateway_grant_response(&response.bytes).unwrap(),
        GatewayGrantResponse::Denied {
            request_id: 3,
            reason: GatewayGrantDenial::NotFound
        }
    ));
    broker.abort();
}

fn channel() -> (WireSender, WireReceiver, WireSender, WireReceiver) {
    let (server, client) = std::os::unix::net::UnixStream::pair().unwrap();
    (
        WireSender::new(server.try_clone().unwrap()).unwrap(),
        WireReceiver::new(server).unwrap(),
        WireSender::new(client.try_clone().unwrap()).unwrap(),
        WireReceiver::new(client).unwrap(),
    )
}

#[tokio::test]
async fn original_socket_is_retained_until_receipt_then_released() {
    let (responses, requests, receipts, granted) = channel();
    let (original, peer) = std::os::unix::net::UnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    let mut peer = tokio::net::UnixStream::from_std(peer).unwrap();
    let sent =
        tokio::spawn(async move { send_grant(&responses, &requests, 7, GatewayGrantKind::Service, original).await });
    // Release the receiver's reference before acknowledging. Only the sender
    // can now keep the peer alive; no descriptor-number/liveness probe races.
    drop(granted.recv().await.unwrap());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), peer.read(&mut [0]))
            .await
            .is_err(),
        "sender released original before adoption"
    );
    receipts
        .send(
            &encode_gateway_grant_request(&GatewayGrantRequest::Adopted { request_id: 7 }).unwrap(),
            &[],
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), sent)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        peer.read(&mut [0]).await.unwrap(),
        0,
        "sender retained original after adoption"
    );
}

#[tokio::test]
async fn adoption_refuses_wrong_identity_direction_descriptors_and_eof() {
    for receipt in [
        Some((GatewayGrantRequest::Adopted { request_id: 8 }, false)),
        Some((GatewayGrantRequest::OpenService { request_id: 7 }, false)),
        Some((GatewayGrantRequest::Adopted { request_id: 7 }, true)),
        None,
    ] {
        let (responses, requests, receipts, granted) = channel();
        let (original, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let sent =
            tokio::spawn(
                async move { send_grant(&responses, &requests, 7, GatewayGrantKind::Service, original).await },
            );
        let frame = granted.recv().await.unwrap();
        if let Some((receipt, extra)) = receipt {
            let descriptors = if extra { vec![frame.fds[0].as_raw_fd()] } else { vec![] };
            receipts
                .send(&encode_gateway_grant_request(&receipt).unwrap(), &descriptors)
                .await
                .unwrap();
        }
        drop((receipts, granted));
        assert!(tokio::time::timeout(std::time::Duration::from_secs(2), sent)
            .await
            .unwrap()
            .unwrap()
            .is_err());
    }
}

#[tokio::test(start_paused = true)]
async fn missing_adoption_times_out_and_releases_original() {
    let (responses, requests, _receipts, granted) = channel();
    let (original, peer) = std::os::unix::net::UnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    let mut peer = tokio::net::UnixStream::from_std(peer).unwrap();
    let sent =
        tokio::spawn(async move { send_grant(&responses, &requests, 7, GatewayGrantKind::Service, original).await });
    drop(granted.recv().await.unwrap());
    tokio::time::advance(ADOPTION_TIMEOUT).await;
    assert_eq!(sent.await.unwrap().unwrap_err().kind(), std::io::ErrorKind::TimedOut);
    assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
}

#[tokio::test]
async fn metrics_grant_is_one_per_control_epoch_and_revokes_on_control_disconnect() {
    use crate::tests::{install_empty_settings_env, EnvVarGuard, SETTINGS_ENV_LOCK};
    use capsem_proto::gateway_grant::metrics::{
        GatewayMetricMessage, GatewayMetricRequest, MAX_GATEWAY_METRIC_FRAME_BYTES,
    };
    let _environment = SETTINGS_ENV_LOCK.lock().await;
    let directory = tempfile::tempdir().unwrap();
    let (_settings, _, corp_path) = install_empty_settings_env(&directory);
    let _base = EnvVarGuard::set("OTEL_EXPORTER_OTLP_ENDPOINT", "");
    let _signal = EnvVarGuard::set("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT", "");
    let _headers = EnvVarGuard::set("OTEL_EXPORTER_OTLP_METRICS_HEADERS", "");
    let _protocol = EnvVarGuard::set("OTEL_EXPORTER_OTLP_METRICS_PROTOCOL", "http/protobuf");
    let _compression = EnvVarGuard::set("OTEL_EXPORTER_OTLP_METRICS_COMPRESSION", "");
    let mut corp = capsem_core::net::policy_config::SettingsFile::default();
    corp.corp_rule_files.open_telemetry = Some("http://127.0.0.1:1".into());
    capsem_core::net::policy_config::write_settings_file(&corp_path, &corp).unwrap();
    let state = crate::tests::make_test_state();
    let (server, client) = std::os::unix::net::UnixStream::pair().unwrap();
    let broker = tokio::spawn(serve(server, state));
    let requests = WireSender::new(client.try_clone().unwrap()).unwrap();
    let responses = WireReceiver::new(client).unwrap();
    requests
        .send(
            &encode_gateway_grant_request(&GatewayGrantRequest::OpenMetrics { request_id: 1 }).unwrap(),
            &[],
        )
        .await
        .unwrap();
    let response = responses.recv().await.unwrap();
    assert_eq!(
        decode_gateway_grant_response(&response.bytes).unwrap(),
        GatewayGrantResponse::Granted {
            request_id: 1,
            kind: GatewayGrantKind::Metrics
        }
    );
    let socket = std::os::unix::net::UnixStream::from(response.fds.into_iter().next().unwrap());
    let (_metrics_requests, metrics_responses) = capsem_foundation::ipc_channel::channel_from_std_with_limit::<
        GatewayMetricRequest,
        GatewayMetricMessage,
    >(socket, MAX_GATEWAY_METRIC_FRAME_BYTES)
    .unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), metrics_responses.recv())
            .await
            .is_err(),
        "metric channel activated before descriptor adoption"
    );
    requests
        .send(
            &encode_gateway_grant_request(&GatewayGrantRequest::Adopted { request_id: 1 }).unwrap(),
            &[],
        )
        .await
        .unwrap();
    assert!(
        matches!(metrics_responses.recv().await.unwrap(), GatewayMetricMessage::Hello { generation } if generation != [0;16])
    );
    requests
        .send(
            &encode_gateway_grant_request(&GatewayGrantRequest::OpenMetrics { request_id: 2 }).unwrap(),
            &[],
        )
        .await
        .unwrap();
    let response = responses.recv().await.unwrap();
    assert!(response.fds.is_empty());
    assert_eq!(
        decode_gateway_grant_response(&response.bytes).unwrap(),
        GatewayGrantResponse::Denied {
            request_id: 2,
            reason: GatewayGrantDenial::Revoked
        }
    );
    // Optional metric refusal never prevents an ordinary service descriptor.
    requests
        .send(
            &encode_gateway_grant_request(&GatewayGrantRequest::OpenService { request_id: 3 }).unwrap(),
            &[],
        )
        .await
        .unwrap();
    let response = responses.recv().await.unwrap();
    assert_eq!(
        decode_gateway_grant_response(&response.bytes).unwrap(),
        GatewayGrantResponse::Granted {
            request_id: 3,
            kind: GatewayGrantKind::Service
        }
    );
    requests
        .send(
            &encode_gateway_grant_request(&GatewayGrantRequest::Adopted { request_id: 3 }).unwrap(),
            &[],
        )
        .await
        .unwrap();
    // A real coordinator-owned socket pair is issued without touching the
    // public UDS listener, including after optional metrics were refused.
    assert_eq!(response.fds.len(), 1);
    drop(response);
    drop((requests, responses));
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), metrics_responses.recv())
            .await
            .unwrap()
            .is_err()
    );
    assert!(broker.await.unwrap().is_err());
}

#[tokio::test]
async fn operator_registration_is_once_and_control_disconnect_retires_pooled_http_authority() {
    use axum::body::{to_bytes, Body};
    use capsem_proto::gateway_grant::auth::{
        decode_gateway_auth_response, encode_gateway_auth_request, GatewayAuthSecret,
    };
    let state = crate::tests::make_test_state();
    let (client, server) = std::os::unix::net::UnixStream::pair().unwrap();
    let broker = tokio::spawn(serve(server, Arc::clone(&state)));
    let requests = WireSender::new(client.try_clone().unwrap()).unwrap();
    let responses = WireReceiver::new(client).unwrap();
    for (request_id, expected) in [
        (1, GatewayAuthResponse::RegisteredOperator { request_id: 1 }),
        (
            2,
            GatewayAuthResponse::Denied {
                request_id: 2,
                reason: GatewayAuthDenial::Forbidden,
            },
        ),
    ] {
        let request = GatewayAuthRequest::RegisterOperator {
            request_id,
            token: GatewayAuthSecret::from_bytes([b'A'; 64]),
        };
        requests
            .send(&encode_gateway_auth_request(&request).unwrap(), &[])
            .await
            .unwrap();
        let response = responses.recv().await.unwrap();
        assert!(response.fds.is_empty());
        assert_eq!(decode_gateway_auth_response(&response.bytes).unwrap(), expected);
    }
    let mut access_id = [0; 16];
    access_id[6] = 0x40;
    access_id[8] = 0x80;
    let request = GatewayAuthRequest::AuthenticateAgent {
        request_id: 3,
        access_id,
        capability: GatewayAuthSecret::from_bytes([0; 32]),
    };
    requests
        .send(&encode_gateway_auth_request(&request).unwrap(), &[])
        .await
        .unwrap();
    let response = responses.recv().await.unwrap();
    assert_eq!(
        decode_gateway_auth_response(&response.bytes).unwrap(),
        GatewayAuthResponse::Denied {
            request_id: 3,
            reason: GatewayAuthDenial::Forbidden
        }
    );
    // An invalid capability must not stop the shared control channel.
    requests
        .send(
            &encode_gateway_grant_request(&GatewayGrantRequest::OpenService { request_id: 4 }).unwrap(),
            &[],
        )
        .await
        .unwrap();
    let response = responses.recv().await.unwrap();
    assert_eq!(
        decode_gateway_grant_response(&response.bytes).unwrap(),
        GatewayGrantResponse::Granted {
            request_id: 4,
            kind: GatewayGrantKind::Service
        }
    );
    requests
        .send(
            &encode_gateway_grant_request(&GatewayGrantRequest::Adopted { request_id: 4 }).unwrap(),
            &[],
        )
        .await
        .unwrap();
    let socket = tokio::net::UnixStream::from_std(std::os::unix::net::UnixStream::from(
        response.fds.into_iter().next().unwrap(),
    ))
    .unwrap();
    let (mut client, connection) = hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(socket))
        .await
        .unwrap();
    tokio::spawn(connection);
    for (token, expected) in [("B", StatusCode::UNAUTHORIZED), ("A", StatusCode::OK)] {
        let response = client
            .send_request(
                axum::http::Request::get("/version")
                    .header(
                        axum::http::header::AUTHORIZATION,
                        format!("Bearer {}", token.repeat(64)),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        to_bytes(Body::new(response.into_body()), 1024).await.unwrap();
    }
    drop((requests, responses));
    assert!(broker.await.unwrap().is_err());
    let response = client
        .send_request(
            axum::http::Request::get("/version")
                .header(axum::http::header::AUTHORIZATION, format!("Bearer {}", "A".repeat(64)))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    if let Ok(response) = response {
        assert!(
            matches!(response.status(), StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN),
            "a pooled connection must not retain a retired actor",
        );
        to_bytes(Body::new(response.into_body()), 1024).await.unwrap();
    }
    // A later valid operator epoch must not resurrect an old broker's seat.
    let mut fresh = state.caller_authority.begin();
    fresh.register(GatewayAuthSecret::from_bytes([b'B'; 64])).unwrap();
    let response = client
        .send_request(
            axum::http::Request::get("/version")
                .header(axum::http::header::AUTHORIZATION, format!("Bearer {}", "B".repeat(64)))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    if let Ok(response) = response {
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "a new actor epoch resurrected a retired gateway transport"
        );
    }
}

#[tokio::test]
async fn private_agent_authentication_checks_live_registry_and_keeps_revoked_identity_for_audit() {
    use capsem_core::{
        agent_access::Reservation,
        managed_sessions::{Capability, LeaseClock},
    };
    use capsem_proto::gateway_grant::auth::{
        decode_gateway_auth_response, encode_gateway_auth_request, GatewayAuthSecret,
    };
    let state = crate::tests::make_test_state();
    let id = capsem_crypto::uuid_v4();
    let scope = capsem_api::AgentAccessScope::new(vec![capsem_api::AgentOperation::Discover], vec![], vec![]).unwrap();
    let Reservation::New(admission) = state
        .caller_authority
        .agents
        .reserve(
            id,
            &Capability::from_bytes([0x11; 32]),
            scope,
            std::time::Duration::from_secs(60),
            || LeaseClock::now().ok(),
        )
        .unwrap()
    else {
        panic!("new admission");
    };
    admission.commit(|| LeaseClock::now().ok()).unwrap();
    let (client, server) = std::os::unix::net::UnixStream::pair().unwrap();
    let broker = tokio::spawn(serve(server, state.clone()));
    let requests = WireSender::new(client.try_clone().unwrap()).unwrap();
    let responses = WireReceiver::new(client).unwrap();
    for (request_id, secret, valid) in [(1, 0x22, false), (2, 0x11, true), (3, 0x11, false)] {
        if request_id == 3 {
            state.caller_authority.agents.revoke(id).unwrap();
        }
        let request = GatewayAuthRequest::AuthenticateAgent {
            request_id,
            access_id: *id.as_bytes(),
            capability: GatewayAuthSecret::from_bytes([secret; 32]),
        };
        requests
            .send(&encode_gateway_auth_request(&request).unwrap(), &[])
            .await
            .unwrap();
        let response = responses.recv().await.unwrap();
        assert!(response.fds.is_empty());
        let expected = if valid {
            GatewayAuthResponse::AuthenticatedAgent {
                request_id,
                access_id: *id.as_bytes(),
            }
        } else {
            GatewayAuthResponse::Denied {
                request_id,
                reason: GatewayAuthDenial::Forbidden,
            }
        };
        assert_eq!(decode_gateway_auth_response(&response.bytes).unwrap(), expected);
    }
    let events = state.host_ledger.host_events().await.unwrap();
    assert_eq!(events.len(), 2);
    assert!(events
        .iter()
        .all(|event| event.kind == capsem_logger::HostEventKind::AccessDenied));
    assert_eq!(events[0].actor, "unauthenticated");
    assert_eq!(events[1].actor, format!("agent:{id}"));
    drop(requests);
    drop(responses);
    assert!(broker.await.unwrap().is_err());
}
