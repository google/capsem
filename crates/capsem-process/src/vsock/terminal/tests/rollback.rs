use super::*;

#[tokio::test]
async fn checkpoint_adopts_an_already_accepted_socket_before_draining() {
    let terminal = capsem_core::terminal::SessionTerminal::new(8);
    let (owner, mut guest) = connection();
    let (rekey, rekey_rx) = mpsc::channel(4);
    let (_input, input_rx) = mpsc::channel(8);
    let bridge = spawn(Arc::clone(&terminal), rekey_rx, input_rx, None, std::io::sink());
    rekey.send(owner).await.unwrap();
    guest.write_all(b"\x1b]2;Already accepted\x07").await.unwrap();
    let mut checkpoint = bridge.park().unwrap();
    tokio::time::timeout(Duration::from_secs(2), checkpoint.settled())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(terminal.subscribe().1.state.title, "Already accepted");
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn rejected_partial_write_resumes_the_exact_queue_once_on_the_same_socket() {
    let terminal = capsem_core::terminal::SessionTerminal::new(8);
    let (owner, mut guest) = connection();
    let fd = owner.try_clone_fd().unwrap();
    capsem_foundation::unix::fd::set_stream_buffers(fd.as_fd(), 4096).unwrap();
    let (rekey, rekey_rx) = mpsc::channel(4);
    let (input, input_rx) = mpsc::channel(8);
    let bridge = spawn(terminal, rekey_rx, input_rx, None, std::io::sink());
    rekey.send(owner).await.unwrap();
    let bytes: Vec<_> = (0..capsem_proto::stream::MAX_DATA_BYTES)
        .map(|n| (n % 251) as u8)
        .collect();
    input.send(bytes.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), guest.readable())
        .await
        .unwrap()
        .unwrap();
    let mut checkpoint = bridge.park().unwrap();
    let error = tokio::time::timeout(Duration::from_secs(2), checkpoint.settled())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::WouldBlock
    );
    // Rollback must retain both the written prefix and the unconfirmed suffix.
    // Queue a second input while parked, then consume only after release.
    input.send(b"after rollback".to_vec()).await.unwrap();
    checkpoint.resume().await.unwrap();
    let expected = [bytes.as_slice(), b"after rollback"].concat();
    let mut received = vec![0; expected.len()];
    tokio::time::timeout(Duration::from_secs(2), guest.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received, expected);
    bridge.shutdown().await.unwrap();
    assert_eq!(guest.read(&mut [0]).await.unwrap(), 0, "duplicated queued input");
}

#[tokio::test]
async fn parked_checkpoint_freezes_parser_and_resumes_input_and_original_connection() {
    let terminal = capsem_core::terminal::SessionTerminal::new(8);
    let mut summaries = terminal.subscribe_summary();
    let (owner, mut guest) = connection();
    let (rekey, rekey_rx) = mpsc::channel(4);
    let (input, input_rx) = mpsc::channel(8);
    let bridge = spawn(Arc::clone(&terminal), rekey_rx, input_rx, None, std::io::sink());
    rekey.send(owner).await.unwrap();
    guest.write_all(b"\x1b]2;Ready\x07").await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), summaries.changed())
        .await
        .unwrap()
        .unwrap();
    guest.write_all(b"\x1b]2;Contin").await.unwrap();
    let mut checkpoint = bridge.park().unwrap();
    tokio::time::timeout(Duration::from_secs(2), checkpoint.settled())
        .await
        .unwrap()
        .unwrap();
    let saved = terminal.checkpoint().unwrap();
    guest.write_all(b"ued\x07").await.unwrap();
    input.send(b"new input".to_vec()).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_millis(20), guest.read(&mut [0]))
        .await
        .is_err());
    assert_eq!(terminal.checkpoint().unwrap(), saved);
    checkpoint.resume().await.unwrap();
    let mut received = [0; 9];
    tokio::time::timeout(Duration::from_secs(2), guest.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&received, b"new input");
    tokio::time::timeout(Duration::from_secs(2), summaries.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summaries.borrow_and_update().title, "Continued");
    // A second checkpoint proves releasing the first did not consume control.
    let mut second = bridge.park().unwrap();
    second.settled().await.unwrap();
    bridge.shutdown().await.unwrap();
    drop(second);
    assert_eq!(guest.read(&mut [0]).await.unwrap(), 0);
}

#[tokio::test]
async fn dropping_an_unobserved_checkpoint_releases_transport_and_shutdown_still_works() {
    let terminal = capsem_core::terminal::SessionTerminal::new(8);
    let (owner, mut guest) = connection();
    let (rekey, rekey_rx) = mpsc::channel(4);
    let (input, input_rx) = mpsc::channel(8);
    let bridge = spawn(terminal, rekey_rx, input_rx, None, std::io::sink());
    rekey.send(owner).await.unwrap();
    drop(bridge.park().unwrap());
    input.send(b"not lost".to_vec()).await.unwrap();
    let mut received = [0; 8];
    tokio::time::timeout(Duration::from_secs(2), guest.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&received, b"not lost");
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn second_checkpoint_cannot_release_the_first_owners_park() {
    let terminal = capsem_core::terminal::SessionTerminal::new(8);
    let (owner, mut guest) = connection();
    let (rekey, rekey_rx) = mpsc::channel(4);
    let (input, input_rx) = mpsc::channel(8);
    let bridge = spawn(terminal, rekey_rx, input_rx, None, std::io::sink());
    rekey.send(owner).await.unwrap();
    let mut first = bridge.park().unwrap();
    first.settled().await.unwrap();
    let mut second = bridge.park().unwrap();
    assert!(second.settled().await.is_err());
    drop(second);
    input.send(b"held".to_vec()).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_millis(20), guest.read(&mut [0]))
        .await
        .is_err());
    first.resume().await.unwrap();
    let mut received = [0; 4];
    tokio::time::timeout(Duration::from_secs(2), guest.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&received, b"held");
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn timed_out_drain_can_resume_while_serial_log_storage_remains_blocked() {
    struct SlowLog {
        entered: Option<tokio::sync::oneshot::Sender<()>>,
        release: std::sync::mpsc::Receiver<()>,
    }
    impl std::io::Write for SlowLog {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if let Some(entered) = self.entered.take() {
                let _ = entered.send(());
                self.release.recv().unwrap();
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    struct Release(Option<std::sync::mpsc::SyncSender<()>>);
    impl Drop for Release {
        fn drop(&mut self) {
            if let Some(release) = self.0.take() {
                let _ = release.send(());
            }
        }
    }
    let terminal = capsem_core::terminal::SessionTerminal::new(8);
    let mut summaries = terminal.subscribe_summary();
    let (owner, mut guest) = connection();
    let (rekey, rekey_rx) = mpsc::channel(4);
    let (input, input_rx) = mpsc::channel(8);
    let (entered, writing) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::sync_channel(1);
    let release = Release(Some(release));
    let bridge = spawn(
        terminal,
        rekey_rx,
        input_rx,
        None,
        SlowLog {
            entered: Some(entered),
            release: released,
        },
    );
    rekey.send(owner).await.unwrap();
    guest.write_all(b"\x1b]2;First\x07").await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), summaries.changed())
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), writing)
        .await
        .unwrap()
        .unwrap();
    for index in 0..=SERIAL_LOG_QUEUE_CAPACITY {
        guest
            .write_all(format!("\x1b]2;Batch{index}\x07").as_bytes())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), summaries.changed())
            .await
            .unwrap()
            .unwrap();
    }
    let mut checkpoint = bridge.park().unwrap();
    assert!(tokio::time::timeout(Duration::from_millis(20), checkpoint.settled())
        .await
        .is_err());
    tokio::time::timeout(Duration::from_secs(2), checkpoint.resume())
        .await
        .unwrap()
        .unwrap();
    input.send(b"usable".to_vec()).await.unwrap();
    let mut received = [0; 6];
    tokio::time::timeout(Duration::from_secs(2), guest.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&received, b"usable");
    drop(release);
    let mut retry = bridge.park().unwrap();
    tokio::time::timeout(Duration::from_secs(2), retry.settled())
        .await
        .unwrap()
        .unwrap();
    bridge.shutdown().await.unwrap();
}
