use super::*;

fn request() -> ApprovalRequest {
    ApprovalRequest {
        ask_id: "0123456789ab".into(),
        event_id: "0123456789ac".into(),
        event_type: "process.exec".into(),
        rule_id: "test.ask".into(),
        rule_name: "Ask".into(),
        summary: "Run command".into(),
        held_bytes: 0,
    }
}

#[tokio::test]
async fn original_caller_consumes_only_its_correlated_decision() {
    let (a, b) = UnixStream::pair().unwrap();
    let client = ApprovalClient::from_stream(a).unwrap();
    let (sender, receiver) =
        ipc_channel::channel_from_std_with_limit::<ApprovalServerMessage, ApprovalClientMessage>(b, frame_limit())
            .unwrap();
    let operation = tokio::spawn(async move { client.decide(request()).await });
    let ApprovalClientMessage::Begin { request_id, .. } = receiver.recv().await.unwrap() else {
        panic!("begin")
    };
    sender
        .send(ApprovalServerMessage {
            request_id: request_id + 1,
            outcome: ApprovalOutcome::Approved,
        })
        .await
        .unwrap();
    assert!(!operation.is_finished());
    sender
        .send(ApprovalServerMessage {
            request_id,
            outcome: ApprovalOutcome::Denied,
        })
        .await
        .unwrap();
    assert_eq!(operation.await.unwrap(), ApprovalOutcome::Denied);
}

#[tokio::test]
async fn caller_disconnect_sends_cancel_and_service_eof_fails_closed() {
    let (a, b) = UnixStream::pair().unwrap();
    let client = ApprovalClient::from_stream(a).unwrap();
    let (_sender, receiver) =
        ipc_channel::channel_from_std_with_limit::<ApprovalServerMessage, ApprovalClientMessage>(b, frame_limit())
            .unwrap();
    let first = client.clone();
    let operation = tokio::spawn(async move { first.decide(request()).await });
    let ApprovalClientMessage::Begin { request_id, .. } = receiver.recv().await.unwrap() else {
        panic!("begin")
    };
    operation.abort();
    let _ = operation.await;
    assert!(
        matches!(receiver.recv().await.unwrap(), ApprovalClientMessage::Cancel { request_id: id } if id == request_id)
    );
    drop(receiver);
    drop(_sender);
    assert_eq!(client.decide(request()).await, ApprovalOutcome::Unavailable);
}
