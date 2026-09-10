use super::*;
use std::net::Ipv4Addr;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream as StdUnixStream;
use tokio::net::{TcpListener, TcpStream, UnixStream};
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn relay_preserves_binary_half_close_and_concurrent_connections() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let (parent, child) = StdUnixStream::pair().unwrap();
    let grants = capsem_foundation::unix::router_channel::Receiver::<Grant>::new(child.try_clone().unwrap()).unwrap();
    let sender = capsem_foundation::unix::router_channel::Sender::new(parent.try_clone().unwrap()).unwrap();
    parent.set_nonblocking(true).unwrap();
    child.set_nonblocking(true).unwrap();
    let mut events = UnixStream::from_std(parent).unwrap();
    let router = tokio::spawn(relay(listener, grants, UnixStream::from_std(child).unwrap()));
    assert_eq!(
        timeout(Duration::from_secs(2), Event::read(&mut events))
            .await
            .unwrap()
            .unwrap(),
        Event::Ready
    );
    let mut clients = tokio::task::JoinSet::new();
    for index in 0..32u8 {
        clients.spawn(async move {
            let mut client = TcpStream::connect(address).await.unwrap();
            let bytes = vec![index; 32 * 1024];
            client.write_all(&bytes).await.unwrap();
            client.shutdown().await.unwrap();
            let mut received = Vec::new();
            client.read_to_end(&mut received).await.unwrap();
            assert_eq!(received, bytes);
        });
    }
    let mut echoes = tokio::task::JoinSet::new();
    let mut closed = 0;
    while closed < 32 {
        match timeout(Duration::from_secs(5), Event::read(&mut events))
            .await
            .unwrap()
            .unwrap()
        {
            Event::Open(id) => {
                let (router_data, echo) = StdUnixStream::pair().unwrap();
                send_grant(
                    &sender,
                    Grant::Connected {
                        id,
                        socket: router_data.as_fd(),
                    },
                )
                .await
                .unwrap();
                echo.set_nonblocking(true).unwrap();
                echoes.spawn(async move {
                    let mut echo = UnixStream::from_std(echo).unwrap();
                    let mut bytes = Vec::new();
                    echo.read_to_end(&mut bytes).await.unwrap();
                    echo.write_all(&bytes).await.unwrap();
                    echo.shutdown().await.unwrap();
                });
            }
            Event::Closed(_) => closed += 1,
            Event::Ready => panic!("duplicate ready"),
        }
    }
    while let Some(result) = clients.join_next().await {
        result.unwrap();
    }
    while let Some(result) = echoes.join_next().await {
        result.unwrap();
    }
    router.abort();
}

#[tokio::test]
async fn hostile_event_is_rejected_after_nine_bytes() {
    let (mut writer, mut reader) = tokio::io::duplex(9);
    writer.write_all(&[255; 9]).await.unwrap();
    assert_eq!(
        Event::read(&mut reader).await.unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}
