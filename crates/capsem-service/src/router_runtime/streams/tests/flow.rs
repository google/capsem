//! Stdin flow control between a stream client and the VM owner.
use super::*;
use capsem_proto::EXEC_STDIN_WINDOW;
use std::time::Duration;

async fn start_exec(client: &mut Client, command: &str) {
    client
        .send(ClientMessage::Binary(
            encode_control(&StreamControl::Start {
                kind: StreamKind::Exec,
                command: Some(command.into()),
            })
            .into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        status(&next_server_frame(client).await.unwrap().1),
        StreamStatus::Started
    );
}

async fn send_stdin(client: &mut Client, frames: usize) {
    for index in 0..frames {
        client
            .send(ClientMessage::Binary(
                encode_data(StreamChannel::Stdin, format!("{index}\n").as_bytes()).into(),
            ))
            .await
            .unwrap();
    }
}

async fn expect_input(rx: &capsem_foundation::ipc_channel::Receiver<ServiceToProcess>, job: u64, index: usize) {
    let message = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("stdin within credit is forwarded")
        .unwrap();
    assert!(
        matches!(&message, ServiceToProcess::ExecStreamInput { id, data } if *id == job && *data == format!("{index}\n").into_bytes()),
        "{message:?}"
    );
}

async fn expect_nothing(rx: &capsem_foundation::ipc_channel::Receiver<ServiceToProcess>) {
    if let Ok(message) = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await {
        panic!("the service sent past its stdin credit: {message:?}");
    }
}

/// The owner queues at most one window of stdin. The service forwards within
/// that credit and waits for `ExecInputConsumed` before sending more, so a
/// command that is slow to read stdin never blocks the owner's read loop.
#[tokio::test]
async fn exec_stdin_is_forwarded_only_within_the_owner_credit() {
    let fx = fixture().await;
    let owner = owner(&fx.uds_path, |tx, rx| async move {
        let ServiceToProcess::ExecStream { id, .. } = rx.recv().await.unwrap() else {
            panic!("expected ExecStream")
        };
        for index in 0..EXEC_STDIN_WINDOW {
            expect_input(&rx, id, index).await;
        }
        expect_nothing(&rx).await;
        tx.send(ProcessToService::ExecInputConsumed { id }).await.unwrap();
        expect_input(&rx, id, EXEC_STDIN_WINDOW).await;
        expect_nothing(&rx).await;
        tx.send(ProcessToService::ExecResult {
            id,
            stdout: vec![],
            stderr: vec![],
            exit_code: 0,
            truncated: false,
        })
        .await
        .unwrap();
    });
    let mut client = connect(fx.address, Some(stream::STREAM_SUBPROTOCOL)).await.unwrap();
    start_exec(&mut client, "slow-reader").await;
    send_stdin(&mut client, EXEC_STDIN_WINDOW + 3).await;
    let exit = status(&next_server_frame(&mut client).await.unwrap().1);
    assert_eq!(
        exit,
        StreamStatus::Exit {
            code: 0,
            truncated: false
        }
    );
    owner.await.unwrap();
}

/// While out of credit the service is not reading the client, yet a client
/// that leaves must still cancel its command rather than leave it running.
#[tokio::test]
async fn a_client_leaving_while_out_of_credit_cancels_the_exec() {
    let fx = fixture().await;
    let owner = owner(&fx.uds_path, |tx, rx| async move {
        // Held open: a dropped write half reads as the owner leaving.
        let _owner_open = tx;
        let ServiceToProcess::ExecStream { id, .. } = rx.recv().await.unwrap() else {
            panic!("expected ExecStream")
        };
        for index in 0..EXEC_STDIN_WINDOW {
            expect_input(&rx, id, index).await;
        }
        let message = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("the departed client's exec is cancelled")
            .unwrap();
        assert!(
            matches!(message, ServiceToProcess::CancelExec { id: cancelled } if cancelled == id),
            "{message:?}"
        );
    });
    let mut client = connect(fx.address, Some(stream::STREAM_SUBPROTOCOL)).await.unwrap();
    start_exec(&mut client, "sleep 600").await;
    send_stdin(&mut client, EXEC_STDIN_WINDOW + 3).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(client);
    owner.await.unwrap();
}
