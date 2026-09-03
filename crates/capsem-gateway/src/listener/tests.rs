use super::*;
use axum::serve::Listener;

#[tokio::test]
async fn accepted_gateway_connections_disable_nagle_buffering() {
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = tcp.local_addr().unwrap();
    let mut listener = low_latency(tcp);
    let client = tokio::spawn(TcpStream::connect(address));

    let (server, _) = Listener::accept(&mut listener).await;
    let client = client.await.unwrap().unwrap();

    assert!(server.nodelay().unwrap());
    drop(client);
}
