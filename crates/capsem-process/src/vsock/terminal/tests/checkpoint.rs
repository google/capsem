use super::*;

#[tokio::test]
async fn checkpoint_drains_socket_and_coalescer_with_replies_logs_and_parser_continuation() {
    let terminal = capsem_core::terminal::SessionTerminal::new(8);
    let mut summaries = terminal.subscribe_summary();
    let (owner, mut guest) = connection();
    let (rekey, rekey_rx) = mpsc::channel(4);
    let (_input, input_rx) = mpsc::channel(8);
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("serial.log");
    let log = capsem_foundation::telemetry::CappedLogWriter::open(&path, 1024 * 1024)
        .unwrap()
        .shared();
    let bridge = spawn(Arc::clone(&terminal), rekey_rx, input_rx, None, log);
    rekey.send(owner).await.unwrap();
    let first = b"\x1b]2;Active\x07";
    guest.write_all(first).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), summaries.changed())
        .await
        .unwrap()
        .unwrap();
    let final_bytes = b"last\x07\x1b[6n\x1b]2;Fragment";
    guest.write_all(final_bytes).await.unwrap();
    terminal.seal_views();
    tokio::time::timeout(Duration::from_secs(2), bridge.checkpoint())
        .await
        .unwrap()
        .unwrap();
    let mut replies = Vec::new();
    guest.read_to_end(&mut replies).await.unwrap();
    assert_eq!(replies, b"\x1b[1;5R");
    assert_eq!(terminal.subscribe().1.state.bell_count, 1);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        [first.as_slice(), final_bytes.as_slice()].concat()
    );
    let restored = capsem_core::terminal::SessionTerminal::from_checkpoint(&terminal.checkpoint().unwrap(), 8).unwrap();
    restored.publish(b" end\x07");
    assert_eq!(restored.subscribe().1.state.title, "Fragment end");
    assert_eq!(restored.subscribe().1.state.bell_count, 1);
}

#[tokio::test]
async fn checkpoint_settles_all_queued_input_before_closing_the_stream() {
    let terminal = capsem_core::terminal::SessionTerminal::new(8);
    let mut summaries = terminal.subscribe_summary();
    let (owner, mut guest) = connection();
    let (rekey, rekey_rx) = mpsc::channel(4);
    let (input, input_rx) = mpsc::channel(8);
    let home = tempfile::tempdir().unwrap();
    let log = capsem_foundation::telemetry::CappedLogWriter::open(&home.path().join("serial.log"), 1024)
        .unwrap()
        .shared();
    let bridge = spawn(Arc::clone(&terminal), rekey_rx, input_rx, None, log);
    rekey.send(owner).await.unwrap();
    guest.write_all(b"\x1b]2;Active\x07").await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), summaries.changed())
        .await
        .unwrap()
        .unwrap();
    input.send(b"first".to_vec()).await.unwrap();
    input.send(b"second".to_vec()).await.unwrap();
    terminal.seal_views();
    tokio::time::timeout(Duration::from_secs(2), bridge.checkpoint())
        .await
        .unwrap()
        .unwrap();
    let mut received = Vec::new();
    guest.read_to_end(&mut received).await.unwrap();
    assert_eq!(received, b"firstsecond");
    assert!(input.is_closed());
}

#[tokio::test]
async fn blocked_partial_write_refuses_checkpoint_instead_of_discarding_pending_input() {
    let terminal = capsem_core::terminal::SessionTerminal::new(8);
    let (owner, guest) = connection();
    let fd = owner.try_clone_fd().unwrap();
    capsem_foundation::unix::fd::set_stream_buffers(fd.as_fd(), 4096).unwrap();
    let (rekey, rekey_rx) = mpsc::channel(4);
    let (input, input_rx) = mpsc::channel(8);
    let home = tempfile::tempdir().unwrap();
    let log = capsem_foundation::telemetry::CappedLogWriter::open(&home.path().join("serial.log"), 1024)
        .unwrap()
        .shared();
    let bridge = spawn(terminal, rekey_rx, input_rx, None, log);
    rekey.send(owner).await.unwrap();
    input
        .send(vec![b'x'; capsem_proto::stream::MAX_DATA_BYTES])
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), guest.readable())
        .await
        .unwrap()
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), bridge.checkpoint())
        .await
        .unwrap();
    assert!(
        result.is_err(),
        "a partial unconfirmed write became a successful checkpoint"
    );
    assert_eq!(
        result.unwrap_err().downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[tokio::test]
async fn checkpoint_does_not_expire_synchronized_output_after_the_transport_stops() {
    let terminal = capsem_core::terminal::SessionTerminal::new(8);
    let mut summaries = terminal.subscribe_summary();
    let (owner, mut guest) = connection();
    let (rekey, rekey_rx) = mpsc::channel(4);
    let (_input, input_rx) = mpsc::channel(8);
    let home = tempfile::tempdir().unwrap();
    let log = capsem_foundation::telemetry::CappedLogWriter::open(&home.path().join("serial.log"), 1024)
        .unwrap()
        .shared();
    let bridge = spawn(Arc::clone(&terminal), rekey_rx, input_rx, None, log);
    rekey.send(owner).await.unwrap();
    guest.write_all(b"\x1b]2;Active\x07").await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), summaries.changed())
        .await
        .unwrap()
        .unwrap();
    guest.write_all(b"\x1b[?2026hheld\x07").await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), bridge.checkpoint())
        .await
        .unwrap()
        .unwrap();
    assert!(terminal.sync_deadline().is_some());
    assert_eq!(terminal.subscribe().1.state.bell_count, 0);
    let bytes = terminal.checkpoint().unwrap();
    let restored = capsem_core::terminal::SessionTerminal::from_checkpoint(&bytes, 8).unwrap();
    restored.publish(b"\x1b[?2026l");
    assert_eq!(restored.subscribe().1.state.bell_count, 1);
}
