use super::*;
use capsem_proto::router::{CloseReason, CloseReport, FlowKey};

#[test]
fn terminal_network_reports_cannot_bypass_bounded_admission() {
    let (sender, _receiver) = CtrlSender::new(Default::default());
    let report = GuestToHost::PortClosed {
        flow: FlowKey { generation: 1, id: 1 },
        report: CloseReport {
            reason: CloseReason::Reset,
            from_source: 0,
            to_source: 0,
        },
    };
    assert!(
        sender.send(report).is_err(),
        "network report entered the unbounded legacy queue without a credit"
    );
}

fn fill(sender: &CtrlSender) {
    for id in 1..=64 {
        let credit = sender.network.reserve().unwrap();
        sender
            .send_closed(
                FlowKey { generation: 7, id },
                CloseReport {
                    reason: CloseReason::Reset,
                    from_source: 17,
                    to_source: 29,
                },
                credit,
            )
            .unwrap();
    }
    assert!(sender.network.reserve().is_err());
}

#[test]
fn acknowledgements_cannot_release_credits_before_queued_duplicates_drain() {
    let (sender, receiver) = CtrlSender::new(Default::default());
    fill(&sender);
    let replay = sender.network.snapshot();
    for id in 1..=64 {
        sender.network.acknowledge(FlowKey { generation: 7, id });
    }
    assert!(sender.network.snapshot().is_empty());
    assert!(sender.network.reserve().is_err());
    while receiver.lock().unwrap().try_recv().is_ok() {}
    assert!(sender.network.reserve().is_err(), "in-flight replay lost its credit");
    drop(replay);
    let credits: Vec<_> = (0..64).map(|_| sender.network.reserve().unwrap()).collect();
    assert!(sender.network.reserve().is_err());
    drop(credits);
}

#[test]
fn draining_does_not_release_reports_until_the_matching_generation_is_acked() {
    let (sender, receiver) = CtrlSender::new(Default::default());
    fill(&sender);
    while receiver.lock().unwrap().try_recv().is_ok() {}
    for id in 1..=64 {
        sender.network.acknowledge(FlowKey { generation: 6, id });
    }
    assert!(sender.network.reserve().is_err());
    for _ in 0..32 {
        assert_eq!(sender.network.snapshot().len(), 64);
        assert!(
            receiver.lock().unwrap().try_recv().is_err(),
            "replay appended queue entries"
        );
    }
    sender.network.acknowledge(FlowKey { generation: 7, id: 1 });
    let credit = sender.network.reserve().unwrap();
    assert!(sender.network.reserve().is_err());
    drop(credit);
    // Legacy control remains independently admissible under network saturation.
    sender.send(GuestToHost::Pong).unwrap();
    assert!(matches!(
        receiver.lock().unwrap().try_recv().unwrap().message,
        GuestToHost::Pong
    ));
}
