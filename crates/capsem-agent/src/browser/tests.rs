use crate::control_writer::CtrlSender;

#[test]
fn socket_drops_truncated_malformed_and_non_browser_records_then_recovers() {
    let (sender, receiver) = CtrlSender::new(Default::default());
    let (client, server) = std::os::unix::net::UnixDatagram::pair().unwrap();
    // Exercise real socket truncation below macOS's datagram send ceiling.
    // The production caller owns a larger fixed buffer for full-length URLs.
    let mut buffer = [0; 128];
    for bytes in [
        vec![0; 256],
        vec![0, 0, 0, 10, 1],
        capsem_proto::encode_guest_msg(&capsem_proto::GuestToHost::SuspendRequest).unwrap(),
    ] {
        client.send(&bytes).unwrap();
        super::receive(&server, &sender, &mut buffer).unwrap();
        assert!(receiver.lock().unwrap().try_recv().is_err());
    }
    let message = capsem_proto::GuestToHost::OpenUrl {
        url: capsem_proto::browser::BrowserUrl("https://example.org/login".into()),
    };
    client.send(&capsem_proto::encode_guest_msg(&message).unwrap()).unwrap();
    super::receive(&server, &sender, &mut buffer).unwrap();
    assert!(matches!(
        receiver.lock().unwrap().try_recv().unwrap().message(),
        capsem_proto::GuestToHost::OpenUrl { .. }
    ));
}

#[test]
fn browser_producer_has_one_credit_and_validates_before_queueing() {
    let (sender, receiver) = CtrlSender::new(Default::default());
    assert!(sender.send_browser("file:///etc/passwd").is_err());
    sender.send_browser("https://example.org/login").unwrap();
    assert!(sender.send_browser("https://example.org/second").is_err());
    let queued = receiver.lock().unwrap().try_recv().unwrap();
    assert!(
        matches!(&queued.message(), capsem_proto::GuestToHost::OpenUrl { url } if url.0 == "https://example.org/login")
    );
    drop(queued);
    sender.send_browser("https://example.org/second").unwrap();
}
