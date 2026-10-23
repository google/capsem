use super::*;
use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_proto::gateway_grant::{decode_gateway_grant_response, encode_gateway_grant_request, GatewayGrantRequest};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn grants_only_the_service_and_registry_derived_owner_seats() {
    let state = crate::tests::make_test_state();
    std::fs::create_dir_all(state.service_socket.parent().unwrap()).unwrap();
    let listener = tokio::net::UnixListener::bind(&state.service_socket).unwrap();
    let (client, server) = std::os::unix::net::UnixStream::pair().unwrap();
    let broker = tokio::spawn(serve(server, Arc::clone(&state)));
    let requests =
        DescriptorSender::<GATEWAY_GRANT_FRAME_SIZE, GATEWAY_GRANT_MAX_FDS>::new(client.try_clone().unwrap()).unwrap();
    let responses = DescriptorReceiver::<GATEWAY_GRANT_FRAME_SIZE, GATEWAY_GRANT_MAX_FDS>::new(client).unwrap();

    let request = encode_gateway_grant_request(&GatewayGrantRequest::OpenService { request_id: 1 }).unwrap();
    requests.send(&request, &[]).await.unwrap();
    let response = responses.recv().await.unwrap();
    assert!(matches!(
        decode_gateway_grant_response(&response.bytes).unwrap(),
        GatewayGrantResponse::Granted {
            request_id: 1,
            kind: GatewayGrantKind::Service
        }
    ));
    requests
        .send(
            &encode_gateway_grant_request(&GatewayGrantRequest::Adopted { request_id: 1 }).unwrap(),
            &[],
        )
        .await
        .unwrap();
    let (mut service_peer, _) = listener.accept().await.unwrap();
    let mut granted = tokio::net::UnixStream::from_std(std::os::unix::net::UnixStream::from(
        response.fds.into_iter().next().unwrap(),
    ))
    .unwrap();
    service_peer.write_all(b"s").await.unwrap();
    let mut byte = [0];
    granted.read_exact(&mut byte).await.unwrap();
    assert_eq!(byte, [b's']);

    let request = encode_gateway_grant_request(&GatewayGrantRequest::OpenOwnerHandoff {
        request_id: 2,
        vm_id: "not-running".into(),
    })
    .unwrap();
    requests.send(&request, &[]).await.unwrap();
    let response = responses.recv().await.unwrap();
    assert!(response.fds.is_empty());
    assert!(matches!(
        decode_gateway_grant_response(&response.bytes).unwrap(),
        GatewayGrantResponse::Denied {
            request_id: 2,
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
    std::fs::create_dir_all(state.service_socket.parent().unwrap()).unwrap();
    let listener = tokio::net::UnixListener::bind(&state.service_socket).unwrap();
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
    let _service = listener.accept().await.unwrap();
    drop((requests, responses));
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), metrics_responses.recv())
            .await
            .unwrap()
            .is_err()
    );
    assert!(broker.await.unwrap().is_err());
}
