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
    for case in 0..6 {
        let (service, owner) = std::os::unix::net::UnixStream::pair().unwrap();
        let sender = Sender::new(service.try_clone().unwrap()).unwrap();
        let receiver = Receiver::new(service).unwrap();
        let answers = Sender::new(owner.try_clone().unwrap()).unwrap();
        let receipts = Receiver::new(owner).unwrap();
        let task = tokio::spawn(async move { adopt_cable(&sender, &receiver, 7).await });
        let (stream, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let descriptor: OwnedFd = if case == 4 {
            tempfile::tempfile().unwrap().into()
        } else {
            stream.into()
        };
        let bytes = match case {
            1 => seat_frame(SEAT_LINK, 8),
            2 => seat_frame(capsem_proto::privatelink::SEAT_PREVIEW, 7),
            _ => seat_frame(SEAT_LINK, 7),
        };
        let fds = if case == 3 {
            vec![]
        } else {
            vec![descriptor.as_raw_fd()]
        };
        answers.send(&bytes, &fds).await.unwrap();
        let result = task.await.unwrap();
        if case == 0 {
            assert!(result.is_ok());
            let receipt = receipts.recv().await.unwrap();
            assert_eq!(receipt.bytes, seat_frame(SEAT_LINK, 7));
            assert!(receipt.fds.is_empty());
        } else {
            assert!(result.is_err(), "accepted invalid cable answer {case}");
            assert!(
                receipts.recv().await.is_err(),
                "acknowledged invalid cable answer {case}"
            );
        }
    }
}
