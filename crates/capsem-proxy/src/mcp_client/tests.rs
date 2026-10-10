use std::os::fd::AsFd as _;
use std::os::unix::net::UnixStream;

use capsem_proto::mcp_aggregator::{
    read_frame, write_frame, AggregatorMethod, AggregatorRequest, AggregatorResponse, AggregatorResult,
};
use capsem_proto::proxy_mcp::ProxyMcpHello;

#[tokio::test]
async fn authenticated_channel_routes_correlated_calls_and_reports_disconnect() {
    let (worker, broker) = UnixStream::pair().unwrap();
    capsem_foundation::unix::fd::set_nonblocking(broker.as_fd(), true).unwrap();
    let mut broker_read = tokio::net::UnixStream::from_std(broker.try_clone().unwrap()).unwrap();
    let mut broker_write = tokio::net::UnixStream::from_std(broker).unwrap();
    let hello = ProxyMcpHello::new(vec!["local".into()], 7, 2_000, 3_000, 4_000).unwrap();
    let broker_task = tokio::spawn(async move {
        write_frame(&mut broker_write, &hello).await.unwrap();
        let request = read_frame::<_, AggregatorRequest>(&mut broker_read)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(request.method, AggregatorMethod::ListTools));
        write_frame(
            &mut broker_write,
            &AggregatorResponse {
                id: request.id,
                body: AggregatorResult::Tools { tools: Vec::new() },
            },
        )
        .await
        .unwrap();
    });

    let (client, hello, mut stopped) = super::start(worker).await.unwrap();
    assert_eq!(hello.builtin_servers, ["local"]);
    assert!(client.list_tools().await.unwrap().is_empty());
    broker_task.await.unwrap();
    stopped.changed().await.unwrap();
    assert!(*stopped.borrow());
}
