use super::*;
use bytes::Bytes;
use capsem_foundation::ipc_channel::channel_from_std_with_limit;
use capsem_proto::gateway_grant::metrics::{
    GatewayMetricMessage, GatewayMetricOutcome, GatewayMetricRequest, MAX_GATEWAY_METRIC_FRAME_BYTES,
};
use opentelemetry_http::HttpClient as _;

#[tokio::test]
async fn channel_uses_epoch_order_and_preserves_bytes_without_dialling_request_url() {
    let (local, remote) = std::os::unix::net::UnixStream::pair().unwrap();
    let server = tokio::spawn(async move {
        let (tx, rx) = channel_from_std_with_limit::<GatewayMetricMessage, GatewayMetricRequest>(
            remote,
            MAX_GATEWAY_METRIC_FRAME_BYTES,
        )
        .unwrap();
        tx.send(GatewayMetricMessage::Hello { generation: [7; 16] })
            .await
            .unwrap();
        for request_id in 1..=2 {
            let request = rx.recv().await.unwrap();
            assert!(request.valid_for([7; 16], request_id));
            assert_eq!(request.body, b"exact-protobuf-canary");
            tx.send(GatewayMetricMessage::Receipt {
                request_id,
                outcome: GatewayMetricOutcome::Accepted,
            })
            .await
            .unwrap();
        }
    });
    let client = MetricClient::connect(local).await.unwrap();
    for _ in 0..2 {
        let response = client
            .send_bytes(
                http::Request::post("http://must-not-dial.invalid/private?secret=canary")
                    .header("authorization", "fake-never-forwarded")
                    .body(Bytes::from_static(b"exact-protobuf-canary"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::NO_CONTENT);
        assert!(response.body().is_empty());
    }
    server.await.unwrap();
}

#[tokio::test]
async fn stale_receipt_closes_channel_instead_of_retrying_or_reopening() {
    let (local, remote) = std::os::unix::net::UnixStream::pair().unwrap();
    let server = tokio::spawn(async move {
        let (tx, rx) = channel_from_std_with_limit::<GatewayMetricMessage, GatewayMetricRequest>(
            remote,
            MAX_GATEWAY_METRIC_FRAME_BYTES,
        )
        .unwrap();
        tx.send(GatewayMetricMessage::Hello { generation: [8; 16] })
            .await
            .unwrap();
        rx.recv().await.unwrap();
        tx.send(GatewayMetricMessage::Receipt {
            request_id: 9,
            outcome: GatewayMetricOutcome::Accepted,
        })
        .await
        .unwrap();
        assert!(rx.recv().await.is_err());
    });
    let client = MetricClient::connect(local).await.unwrap();
    for _ in 0..2 {
        let error = client
            .send_bytes(http::Request::new(Bytes::from_static(b"private-error-canary")))
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("canary"));
    }
    server.await.unwrap();
}

#[tokio::test]
async fn zero_owner_epoch_is_rejected() {
    let (local, remote) = std::os::unix::net::UnixStream::pair().unwrap();
    let (tx, _rx) = channel_from_std_with_limit::<GatewayMetricMessage, GatewayMetricRequest>(
        remote,
        MAX_GATEWAY_METRIC_FRAME_BYTES,
    )
    .unwrap();
    tx.send(GatewayMetricMessage::Hello { generation: [0; 16] })
        .await
        .unwrap();
    assert!(MetricClient::connect(local).await.is_err());
}

#[test]
fn sdk_owned_body_allocation_moves_into_the_typed_envelope() {
    let original = vec![11_u8; 4096];
    let pointer = original.as_ptr();
    let body = take_body(Bytes::from(original)).unwrap();
    assert_eq!(body.as_ptr(), pointer);
    assert!(take_body(Bytes::new()).is_err());
    assert!(take_body(Bytes::from(vec![0; MAX_GATEWAY_METRIC_BODY_BYTES + 1])).is_err());
}

#[tokio::test]
async fn rejected_and_unavailable_batches_do_not_poison_the_next_ordered_batch() {
    let (local, remote) = std::os::unix::net::UnixStream::pair().unwrap();
    let server = tokio::spawn(async move {
        let (tx, rx) = channel_from_std_with_limit::<GatewayMetricMessage, GatewayMetricRequest>(
            remote,
            MAX_GATEWAY_METRIC_FRAME_BYTES,
        )
        .unwrap();
        tx.send(GatewayMetricMessage::Hello { generation: [9; 16] })
            .await
            .unwrap();
        for (index, outcome) in [
            GatewayMetricOutcome::Rejected,
            GatewayMetricOutcome::Unavailable,
            GatewayMetricOutcome::Accepted,
        ]
        .into_iter()
        .enumerate()
        {
            let request = rx.recv().await.unwrap();
            assert_eq!(request.request_id, index as u64 + 1);
            tx.send(GatewayMetricMessage::Receipt {
                request_id: request.request_id,
                outcome,
            })
            .await
            .unwrap();
        }
    });
    let client = MetricClient::connect(local).await.unwrap();
    for expected in [false, false, true] {
        assert_eq!(
            client
                .send_bytes(http::Request::new(Bytes::from_static(b"bounded")))
                .await
                .is_ok(),
            expected
        );
    }
    server.await.unwrap();
}

#[tokio::test]
async fn cancelling_a_waiting_export_closes_its_partial_exchange() {
    let (local, remote) = std::os::unix::net::UnixStream::pair().unwrap();
    let (seen, received) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (tx, rx) = channel_from_std_with_limit::<GatewayMetricMessage, GatewayMetricRequest>(
            remote,
            MAX_GATEWAY_METRIC_FRAME_BYTES,
        )
        .unwrap();
        tx.send(GatewayMetricMessage::Hello { generation: [4; 16] })
            .await
            .unwrap();
        rx.recv().await.unwrap();
        seen.send(()).unwrap();
        assert!(rx.recv().await.is_err());
    });
    let client = MetricClient::connect(local).await.unwrap();
    let waiting = tokio::spawn(async move {
        client
            .send_bytes(http::Request::new(Bytes::from_static(b"bounded")))
            .await
    });
    received.await.unwrap();
    waiting.abort();
    let _ = waiting.await;
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn actual_sdk_export_from_its_own_thread_uses_the_bounded_channel() {
    use tower::ServiceExt;
    let (local, remote) = std::os::unix::net::UnixStream::pair().unwrap();
    let server = tokio::spawn(async move {
        let (tx, rx) = channel_from_std_with_limit::<GatewayMetricMessage, GatewayMetricRequest>(
            remote,
            MAX_GATEWAY_METRIC_FRAME_BYTES,
        )
        .unwrap();
        tx.send(GatewayMetricMessage::Hello { generation: [3; 16] })
            .await
            .unwrap();
        let mut batches = 0;
        while let Ok(request) = rx.recv().await {
            capsem_telemetry::export::relay::AdmittedBody::admit(request.body.into()).unwrap();
            batches += 1;
            tx.send(GatewayMetricMessage::Receipt {
                request_id: request.request_id,
                outcome: GatewayMetricOutcome::Accepted,
            })
            .await
            .unwrap();
        }
        batches
    });
    let client = MetricClient::connect(local).await.unwrap();
    let exporter =
        tokio::task::spawn_blocking(move || capsem_telemetry::export::install_gateway_http_client(client).unwrap())
            .await
            .unwrap();
    let (_, state) = crate::tests::health_app("/unreachable/service.sock");
    let response = crate::gateway_router(state)
        .oneshot(
            http::Request::builder()
                .uri("/health")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), http::StatusCode::OK);
    drop(response);
    tokio::task::spawn_blocking(move || {
        exporter.flush().unwrap();
        drop(exporter);
    })
    .await
    .unwrap();
    assert!(server.await.unwrap() > 0);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn confined_gateway_can_export_only_over_its_inherited_metric_descriptor() {
    use std::os::fd::OwnedFd;
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let (local, remote) = std::os::unix::net::UnixStream::pair().unwrap();
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "metric_export::tests::confined_metric_child", "--nocapture"])
        .env(
            "CAPSEM_METRIC_CONFINED_PORT",
            listener.local_addr().unwrap().port().to_string(),
        )
        .stdin(std::process::Stdio::from(OwnedFd::from(local)));
    let child = command.spawn().unwrap();
    let (tx, rx) = channel_from_std_with_limit::<GatewayMetricMessage, GatewayMetricRequest>(
        remote,
        MAX_GATEWAY_METRIC_FRAME_BYTES,
    )
    .unwrap();
    tx.send(GatewayMetricMessage::Hello { generation: [6; 16] })
        .await
        .unwrap();
    let request = rx.recv().await.unwrap();
    assert!(request.valid_for([6; 16], 1));
    assert_eq!(request.body, b"confined-protobuf");
    tx.send(GatewayMetricMessage::Receipt {
        request_id: 1,
        outcome: GatewayMetricOutcome::Accepted,
    })
    .await
    .unwrap();
    let output = child.wait_with_output().await.unwrap();
    assert!(output.status.success());
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn confined_metric_child() {
    use std::os::fd::AsFd;
    let Ok(port) = std::env::var("CAPSEM_METRIC_CONFINED_PORT") else {
        return;
    };
    let port = port.parse::<u16>().unwrap();
    let descriptor = capsem_foundation::unix::fd::duplicate(std::io::stdin().as_fd()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    capsem_foundation::unix::worker_sandbox::confine(&capsem_foundation::unix::worker_sandbox::Policy::new(
        capsem_foundation::unix::worker_sandbox::Role::Gateway,
    ))
    .unwrap();
    assert_eq!(
        std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    runtime.block_on(async {
        let client = MetricClient::connect(std::os::unix::net::UnixStream::from(descriptor))
            .await
            .unwrap();
        client
            .send_bytes(
                http::Request::post("http://forbidden.invalid")
                    .body(Bytes::from_static(b"confined-protobuf"))
                    .unwrap(),
            )
            .await
            .unwrap();
    });
}
