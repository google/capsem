use super::*;

#[test]
fn ackable_responses_are_parked_at_send_time_and_pongs_are_not() {
    let (sender, _rx) = test_ctrl_channel();
    sender.send(GuestToHost::Pong).unwrap();
    sender.send(GuestToHost::ExecDone { id: 4, exit_code: 0 }).unwrap();
    let parked: Vec<u64> = sender.pending.lock().unwrap().keys().copied().collect();
    assert_eq!(parked, vec![4]);
}

#[test]
fn network_close_survives_failed_writer_and_reconnect_without_using_job_ack_ids() {
    use capsem_proto::router::{CloseReason, CloseReport, FlowKey};
    let (sender, rx) = test_ctrl_channel();
    let first = spawn_writer_conn(&sender, &rx);
    first.host.shutdown(std::net::Shutdown::Both).unwrap();
    drop(first.host);
    let flow = FlowKey { generation: 3, id: 9 };
    let _ack = sender
        .send_closed(
            flow,
            CloseReport {
                reason: CloseReason::Reset,
                from_source: 7,
                to_source: 5,
            },
            sender.network.reserve().unwrap(),
        )
        .unwrap();
    first.handle.join().unwrap();
    assert!(!first.alive.load(std::sync::atomic::Ordering::SeqCst));
    unsafe {
        libc::close(first.guest_fd);
    }
    sender.send(GuestToHost::ExecDone { id: 9, exit_code: 0 }).unwrap();
    let mut second = spawn_writer_conn(&sender, &rx);
    let report = wait_for_frame(&mut second.host, |message| {
        matches!(message, GuestToHost::PortClosed { .. })
    });
    assert!(
        matches!(report, GuestToHost::PortClosed { flow: key, report: CloseReport {
        reason: CloseReason::Reset, from_source: 7, to_source: 5,
    }} if key == flow)
    );
    sender.network.acknowledge(flow);
    assert!(
        sender.pending.lock().unwrap().contains_key(&9),
        "network ACK removed an exec reply"
    );
    second.finish();
}

#[test]
fn file_reply_queue_and_replay_share_only_prepared_bytes() {
    let (sender, rx) = test_ctrl_channel();
    sender
        .send(GuestToHost::FileContent {
            id: 781,
            path: "/workspace/exact".into(),
            data: vec![13; 4096],
        })
        .unwrap();
    let queued = rx.lock().unwrap().try_recv().unwrap();
    let parked = sender.pending.lock().unwrap().get(&781).unwrap().clone();
    assert_eq!(
        queued.frame.as_ref().unwrap().bytes().as_ptr(),
        parked.frame.as_ref().unwrap().bytes().as_ptr()
    );
    assert!(
        matches!(capsem_proto::decode_guest_msg(&queued.frame.as_ref().unwrap().bytes()[4..]).unwrap(), GuestToHost::FileContent { id: 781, data, .. } if data == vec![13; 4096])
    );
    drop(parked);
    drop(queued);
}

#[test]
fn oversized_file_reply_writes_only_small_error_then_next_healthy_frame() {
    let (sender, rx) = test_ctrl_channel();
    sender
        .send(GuestToHost::FileContent {
            id: 982,
            path: "/workspace/large".into(),
            data: vec![4; MAX_FRAME_SIZE as usize],
        })
        .unwrap();
    sender.send(GuestToHost::Pong).unwrap();
    let mut conn = spawn_writer_conn(&sender, &rx);
    let error = wait_for_frame(&mut conn.host, |message| {
        matches!(message, GuestToHost::Error { id: 982, .. })
    });
    assert!(matches!(error, GuestToHost::Error { message, .. } if message.contains("too large")));
    wait_for_frame(&mut conn.host, |message| matches!(message, GuestToHost::Pong));
    assert!(matches!(
        sender.pending.lock().unwrap().get(&982).unwrap().message(),
        GuestToHost::Error { id: 982, .. }
    ));
    conn.finish();
}

#[test]
fn unframeable_file_reply_is_replaced_before_queue_or_replay_admission() {
    let (sender, receiver) = test_ctrl_channel();
    sender
        .send(GuestToHost::FileContent {
            id: 7,
            path: "/root/blob.bin".to_string(),
            data: vec![0xFF; MAX_FRAME_SIZE as usize],
        })
        .unwrap();
    let queued = receiver.lock().unwrap().try_recv().unwrap();
    assert!(matches!(queued.message(), GuestToHost::Error { id: 7, message } if message.contains("too large")));
    drop(queued);
    assert!(matches!(
        sender.pending.lock().unwrap().get(&7).unwrap().message(),
        GuestToHost::Error { id: 7, .. }
    ));
    sender.send(GuestToHost::Ack { id: 8 }).unwrap();
    assert!(matches!(
        receiver.lock().unwrap().try_recv().unwrap().message(),
        GuestToHost::Ack { id: 8 }
    ));
}

#[test]
fn unframeable_generic_reply_writes_nothing_and_next_original_operation_survives() {
    let (sender, rx) = test_ctrl_channel();
    sender
        .send(GuestToHost::Error {
            id: 1993,
            message: "x".repeat(MAX_FRAME_SIZE as usize),
        })
        .unwrap();
    sender.send(GuestToHost::Pong).unwrap();
    let mut connection = spawn_writer_conn(&sender, &rx);
    assert!(matches!(read_guest_frame(&mut connection.host), GuestToHost::Pong));
    assert!(!sender.pending.lock().unwrap().contains_key(&1993));
    connection.finish();
}
