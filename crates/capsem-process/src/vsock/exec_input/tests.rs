use super::*;
use std::os::fd::AsRawFd;

/// Every stdin frame handed to the guest, EOF included, returns one unit of
/// credit; without it the service would stop sending after the first window.
#[tokio::test]
async fn each_frame_written_to_the_guest_returns_one_credit() {
    let (host, mut guest) = std::os::unix::net::UnixStream::pair().unwrap();
    let connection = VsockConnection::new(host.as_raw_fd(), 0, Box::new(host));
    let (frames, queue) = tokio::sync::mpsc::channel(capsem_proto::EXEC_STDIN_WINDOW);
    let (credit, mut credits) = tokio::sync::mpsc::channel(capsem_proto::EXEC_STDIN_WINDOW);
    let writer = spawn(&connection, 7, Some(queue), Some(credit)).expect("writer thread");
    frames.send(ExecInputFrame::Data(b"one".to_vec())).await.unwrap();
    frames.send(ExecInputFrame::Data(b"two".to_vec())).await.unwrap();
    frames.send(ExecInputFrame::StdinEof).await.unwrap();

    let received = tokio::task::spawn_blocking(move || {
        (0..3)
            .map(|_| capsem_proto::read_exec_input(&mut guest).unwrap())
            .collect::<Vec<_>>()
    })
    .await
    .unwrap();
    assert_eq!(
        received,
        [
            ExecInputFrame::Data(b"one".to_vec()),
            ExecInputFrame::Data(b"two".to_vec()),
            ExecInputFrame::StdinEof,
        ]
    );
    for _ in 0..3 {
        assert!(matches!(
            credits.recv().await,
            Some(ProcessToService::ExecInputConsumed { id: 7 })
        ));
    }
    tokio::task::spawn_blocking(move || writer.join().unwrap())
        .await
        .unwrap();
    assert!(credits.recv().await.is_none(), "the writer stops after EOF");
}
