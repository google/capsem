use super::*;

#[tokio::test]
async fn completed_switch_retirement_is_awaited_despite_another_network_failure() {
    let state = crate::tests::make_test_state();
    let good = state.networks.lock().await.create("completed", 1000).await.unwrap();
    let switch = host(&state, good.id).await.unwrap();
    assert!(!switch.is_closed());
    {
        let mut registry = state.networks.lock().await;
        registry.attach(good.id, "completed-vm", 1001).await.unwrap();
        registry.vm_deleted("completed-vm", 1002).await.unwrap();
        drop(registry);
    }
    let future = i64::MAX / 2;
    let bad = state.networks.lock().await.create("pending", future).await.unwrap();
    state
        .networks
        .lock()
        .await
        .attach(bad.id, "pending-vm", future)
        .await
        .unwrap();
    assert!(crate::network_routes::vm_deleted_checked(&state, "pending-vm")
        .await
        .is_err());
    assert!(
        switch.is_closed(),
        "already retired switch must be awaited on the error path"
    );
    assert!(!state.switches.networks.lock().await.contains_key(&good.id));
    assert!(state.networks.lock().await.address_of(bad.id, "pending-vm").is_some());
    state
        .networks
        .lock()
        .await
        .vm_deleted("pending-vm", future + 1)
        .await
        .unwrap();
    crate::network_routes::vm_deleted_checked(&state, "pending-vm")
        .await
        .unwrap();
}

#[tokio::test]
async fn cable_answer_requires_exact_identity_shape_and_connected_stream() {
    use std::os::fd::{AsRawFd as _, OwnedFd};
    for (kind, token, descriptor_count, connected, accepted) in [
        (SEAT_LINK, 7, 1, true, true),
        (SEAT_LINK, 8, 1, true, false),
        (capsem_proto::privatelink::SEAT_PREVIEW, 7, 1, true, false),
        (SEAT_LINK, 7, 0, true, false),
        (SEAT_LINK, 7, 2, true, false),
        (SEAT_LINK, 7, 1, false, false),
    ] {
        let (service, owner) = std::os::unix::net::UnixStream::pair().unwrap();
        let sender = Sender::new(service.try_clone().unwrap()).unwrap();
        let receiver = Receiver::new(service).unwrap();
        let answers = Sender::new(owner.try_clone().unwrap()).unwrap();
        let receipts = Receiver::new(owner).unwrap();
        let task = tokio::spawn(async move { adopt_cable(&sender, &receiver, 7).await });
        let (stream, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let descriptor: OwnedFd = if connected {
            stream.into()
        } else {
            tempfile::tempfile().unwrap().into()
        };
        let bytes = seat_frame(kind, token);
        let fds = vec![descriptor.as_raw_fd(); descriptor_count];
        answers.send(&bytes, &fds).await.unwrap();
        let result = task.await.unwrap();
        if accepted {
            assert!(result.is_ok());
            let receipt = receipts.recv().await.unwrap();
            assert_eq!(receipt.bytes, seat_frame(SEAT_LINK, 7));
            assert!(receipt.fds.is_empty());
        } else {
            assert!(
                result.is_err(),
                "accepted invalid cable answer: kind={kind} token={token} count={descriptor_count} stream={connected}"
            );
            assert!(
                receipts.recv().await.is_err(),
                "acknowledged invalid cable answer: kind={kind} token={token} count={descriptor_count} stream={connected}"
            );
        }
    }
}
