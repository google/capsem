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
