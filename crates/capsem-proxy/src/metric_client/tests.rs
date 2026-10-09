use super::*;
use opentelemetry_http::HttpClient as _;

#[tokio::test]
async fn client_accepts_identity_then_relays_only_the_encoded_body() {
    let (broker, proxy) = UnixStream::pair().unwrap();
    let serving = tokio::spawn(async move {
        let (messages, requests) =
            ipc_channel::channel_from_std::<ProxyMetricBrokerMessage, ProxyMetricRequest>(broker).unwrap();
        messages
            .send(ProxyMetricBrokerMessage::Hello {
                session_id: "vm-a".into(),
            })
            .await
            .unwrap();
        assert_eq!(requests.recv().await.unwrap().body, b"encoded otlp");
        messages
            .send(ProxyMetricBrokerMessage::Response(ProxyMetricResponse::Relayed {
                status: 202,
                content_type: Some("application/x-protobuf".into()),
                body: b"accepted".to_vec(),
            }))
            .await
            .unwrap();
    });

    let (client, session_id) = start(proxy).await.unwrap();
    assert_eq!(session_id, "vm-a");
    let response = client
        .send_bytes(
            http::Request::post("https://attacker.invalid/collector")
                .header(http::header::AUTHORIZATION, "must not cross")
                .body(bytes::Bytes::from_static(b"encoded otlp"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    assert_eq!(response.body().as_ref(), b"accepted");
    serving.await.unwrap();
}

#[tokio::test]
async fn client_rejects_a_response_before_the_broker_identity() {
    let (broker, proxy) = UnixStream::pair().unwrap();
    let serving = tokio::spawn(async move {
        let (messages, _requests) =
            ipc_channel::channel_from_std::<ProxyMetricBrokerMessage, ProxyMetricRequest>(broker).unwrap();
        messages
            .send(ProxyMetricBrokerMessage::Response(ProxyMetricResponse::Rejected {
                message: "not a hello".into(),
            }))
            .await
            .unwrap();
    });
    assert!(start(proxy).await.unwrap_err().to_string().contains("before identity"));
    serving.await.unwrap();
}
