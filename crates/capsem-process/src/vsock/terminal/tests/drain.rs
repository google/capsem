use super::*;
use capsem_core::pty_log::{read_output_bytes, PtyLog, PtyRecorder};

#[tokio::test]
async fn process_drain_stops_the_bridge_and_flushes_transcripts_with_live_senders() {
    let terminal = capsem_core::terminal::SessionTerminal::new(8);
    let mut summaries = terminal.subscribe_summary();
    let (owner, mut guest) = connection();
    let (rekey, rekey_rx) = mpsc::channel(4);
    let (_input, input_rx) = mpsc::channel(8);
    let home = tempfile::tempdir().unwrap();
    let transcript = home.path().join("pty.log");
    let serial = home.path().join("serial.log");
    let recorder = PtyRecorder::new(PtyLog::open(&transcript).unwrap()).unwrap();
    let log = capsem_foundation::telemetry::CappedLogWriter::open(&serial, 1024 * 1024)
        .unwrap()
        .shared();
    let bridge = spawn(terminal, rekey_rx, input_rx, Some(recorder.clone()), log);
    rekey.send(owner).await.unwrap();
    let shown = b"final output\x1b]2;Accepted\x07";
    guest.write_all(shown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), summaries.changed())
        .await
        .unwrap()
        .unwrap();
    let shutdown = Arc::new(tokio::sync::Mutex::new(crate::Shutdown {
        terminal_bridge: Some(bridge),
        pty_log: Some(recorder.clone()),
        ..Default::default()
    }));
    tokio::time::timeout(Duration::from_secs(2), crate::drain_background_owners(&shutdown))
        .await
        .unwrap();
    assert_eq!(
        recorder.record_input(b"after close").unwrap_err().kind(),
        std::io::ErrorKind::BrokenPipe
    );
    drop(recorder);
    assert_eq!(read_output_bytes(&transcript).unwrap(), shown);
    assert_eq!(std::fs::read(serial).unwrap(), shown);
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), guest.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    // The rekey sender was never dropped to make the bridge finish.
    assert!(rekey.is_closed());
}
