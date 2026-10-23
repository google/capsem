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
async fn registered_identity_notifies_the_original_wait_but_never_resolves_it_or_extends_twice() {
    for mismatch in [false, true] {
        let (a, b) = UnixStream::pair().unwrap();
        let client = ApprovalClient::from_stream(a).unwrap();
        let (sender, receiver) =
            ipc_channel::channel_from_std_with_limit::<ApprovalServerMessage, ApprovalClientMessage>(b, frame_limit())
                .unwrap();
        let (registered, receipt) = oneshot::channel();
        let original = tokio::spawn(async move { client.decide_registered(request(), Some(registered)).await });
        let ApprovalClientMessage::Begin {
            request_id,
            request: original_request,
        } = receiver.recv().await.unwrap()
        else {
            panic!("begin")
        };
        let expires_at_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
            + 1000;
        let registration = ApprovalRegistration {
            ask_id: if mismatch {
                "ffffffffffff".into()
            } else {
                original_request.ask_id
            },
            event_id: original_request.event_id,
            expires_at_unix_ms,
            remaining_ms: 1000,
        };
        sender
            .send(ApprovalServerMessage::Registered {
                request_id,
                registration: registration.clone(),
            })
            .await
            .unwrap();
        if mismatch {
            assert!(receipt.await.is_err());
            assert_eq!(original.await.unwrap(), ApprovalOutcome::Unavailable);
        } else {
            assert_eq!(receipt.await.unwrap(), registration);
            assert!(
                !original.is_finished(),
                "registration cannot authorize the held operation"
            );
            // A second receipt, including a longer expiry, cannot reset its deadline.
            let duplicate = ApprovalRegistration {
                expires_at_unix_ms: expires_at_unix_ms + 1000,
                remaining_ms: 2000,
                ..registration
            };
            sender
                .send(ApprovalServerMessage::Registered {
                    request_id,
                    registration: duplicate,
                })
                .await
                .unwrap();
            assert_eq!(original.await.unwrap(), ApprovalOutcome::Unavailable);
        }
    }
}

#[tokio::test]
async fn setup_waiters_are_bounded_before_any_coordinator_reply() {
    let (_sender, receiver) = tokio::sync::watch::channel(None);
    let capability = ApprovalCapability::with_setup(receiver, tokio::spawn(std::future::pending()));
    let mut pending = Vec::new();
    for _ in 0..capsem_proto::APPROVAL_PENDING_PER_SESSION {
        let mut value = request();
        value.held_bytes = capsem_proto::APPROVAL_HELD_BYTES_PER_REQUEST as u64;
        let mut operation = Box::pin(capability.decide(value));
        assert!(futures::poll!(&mut operation).is_pending());
        pending.push(operation);
    }
    let mut overflow = Box::pin(capability.decide(request()));
    assert!(
        matches!(
            futures::poll!(&mut overflow),
            std::task::Poll::Ready(ApprovalOutcome::Unavailable)
        ),
        "setup cannot retain an unbounded number of original bodies"
    );
    drop(pending);
    let mut replacement = Box::pin(capability.decide(request()));
    assert!(
        futures::poll!(&mut replacement).is_pending(),
        "cancelled setup returns its capacity"
    );
}

#[tokio::test(start_paused = true)]
async fn setup_and_human_wait_share_one_operation_deadline() {
    let (commands, mut inbox) = mpsc::channel(capsem_proto::APPROVAL_PENDING_PER_SESSION);
    let client = ApprovalClient {
        commands,
        stopped: CancellationToken::new(),
        next: Arc::new(AtomicU64::new(1)),
    };
    let (sender, receiver) = tokio::sync::watch::channel(None);
    let setup = tokio::spawn(async move {
        tokio::time::sleep(deadline().checked_sub(Duration::from_secs(1)).unwrap()).await;
        sender.send_replace(Some(Ok(client)));
    });
    let capability = ApprovalCapability::with_setup(receiver, setup);
    let started = tokio::time::Instant::now();
    assert_eq!(capability.decide(request()).await, ApprovalOutcome::Expired);
    assert_eq!(
        started.elapsed(),
        deadline(),
        "setup must not reset the original deadline"
    );
    let Command::Begin { id, .. } = inbox.recv().await.unwrap() else {
        panic!("begin")
    };
    assert!(matches!(inbox.recv().await.unwrap(), Command::Cancel { id: cancelled } if cancelled == id));
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
        .send(ApprovalServerMessage::Resolved {
            request_id: request_id + 1,
            outcome: ApprovalOutcome::Approved,
        })
        .await
        .unwrap();
    assert!(!operation.is_finished());
    sender
        .send(ApprovalServerMessage::Resolved {
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
