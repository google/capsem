use super::*;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

type TestBody = http_body_util::combinators::UnsyncBoxBody<Bytes, std::io::Error>;

fn full(bytes: impl Into<Bytes>) -> TestBody {
    Full::new(bytes.into()).map_err(|never| match never {}).boxed_unsync()
}

async fn pair<F, Fut>(handler: F) -> (BrokerHttpClient, tokio::task::JoinHandle<()>)
where
    F: Fn(http::Request<Incoming>) -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = http::Response<TestBody>> + Send + 'static,
{
    let (coordinator, worker) = std::os::unix::net::UnixStream::pair().unwrap();
    coordinator.set_nonblocking(true).unwrap();
    let coordinator = tokio::net::UnixStream::from_std(coordinator).unwrap();
    let server = tokio::spawn(async move {
        let service = service_fn(move |request| {
            let handler = handler.clone();
            async move { Ok::<_, Infallible>(handler(request).await) }
        });
        let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(coordinator), service)
            .await;
    });
    (
        BrokerHttpClient::connect(worker, std::time::Duration::from_secs(2))
            .await
            .unwrap(),
        server,
    )
}

#[tokio::test]
async fn inherited_channel_preserves_http_metadata_and_raw_bytes() {
    let (client, server) = pair(|request| async move {
        assert_eq!(request.method(), http::Method::POST);
        assert_eq!(request.uri(), "https://configured.example/mcp?q=a%20b");
        assert_eq!(request.headers()["x-tool-session"], "session-one");
        assert_eq!(
            request.into_body().collect().await.unwrap().to_bytes(),
            &[0, 255, 13, 10][..]
        );
        http::Response::builder()
            .status(429)
            .header("content-type", "application/octet-stream")
            .header("retry-after", "3")
            .body(full(Bytes::from_static(&[255, 0, 10])))
            .unwrap()
    })
    .await;
    let request = reqwest::Client::new()
        .post("https://configured.example/mcp?q=a%20b")
        .header("x-tool-session", "session-one")
        .body(vec![0, 255, 13, 10])
        .build()
        .unwrap();
    let response = client.execute(request).await.unwrap();
    assert_eq!(response.status(), 429);
    assert_eq!(response.url().as_str(), "https://configured.example/mcp?q=a%20b");
    assert_eq!(response.headers()["retry-after"], "3");
    assert_eq!(response.bytes().await.unwrap(), &[255, 0, 10][..]);
    drop(client);
    tokio::time::timeout(std::time::Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn an_open_event_stream_does_not_serialize_other_calls() {
    let (send, receive) = mpsc::channel::<Result<Bytes, std::io::Error>>(2);
    let receive = Arc::new(Mutex::new(Some(receive)));
    let (client, server) = pair(move |request| {
        let receive = receive.clone();
        async move {
            if request.uri().path() == "/events" {
                let stream = futures::stream::unfold(receive.lock().unwrap().take().unwrap(), |mut rx| async {
                    rx.recv().await.map(|next| (next.map(hyper::body::Frame::data), rx))
                });
                http::Response::new(http_body_util::StreamBody::new(stream).boxed_unsync())
            } else {
                http::Response::new(full("second call"))
            }
        }
    })
    .await;
    let builder = reqwest::Client::new();
    let mut events = client
        .execute(builder.get("http://server.invalid/events").build().unwrap())
        .await
        .unwrap();
    send.send(Ok(Bytes::from_static(b"data: first\n\n"))).await.unwrap();
    assert_eq!(events.chunk().await.unwrap().unwrap(), "data: first\n\n");
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        client.execute(builder.get("http://server.invalid/call").build().unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.text().await.unwrap(), "second call");
    drop(events);
    tokio::time::timeout(std::time::Duration::from_secs(1), send.closed())
        .await
        .unwrap();
    drop(client);
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn request_deadline_cancels_one_stream_without_replaying_or_closing_the_channel() {
    let (cancelled, received) = oneshot::channel();
    let cancelled = Arc::new(Mutex::new(Some(cancelled)));
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = calls.clone();
    struct OnDrop(Option<oneshot::Sender<()>>);
    impl Drop for OnDrop {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }
    let (client, server) = pair(move |request| {
        let cancelled = cancelled.clone();
        let calls = observed.clone();
        async move {
            calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if request.uri().path() == "/slow" {
                let _drop = OnDrop(cancelled.lock().unwrap().take());
                std::future::pending::<()>().await;
            }
            http::Response::new(full("ready"))
        }
    })
    .await;
    let builder = reqwest::Client::new();
    let request = builder
        .post("http://server.invalid/slow")
        .timeout(std::time::Duration::from_millis(30))
        .build()
        .unwrap();
    let error = client.execute(request).await.expect_err("deadline did not expire");
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    tokio::time::timeout(std::time::Duration::from_secs(1), received)
        .await
        .unwrap()
        .unwrap();
    let response = client
        .execute(builder.get("http://server.invalid/fast").build().unwrap())
        .await
        .unwrap();
    assert_eq!(response.text().await.unwrap(), "ready");
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::Relaxed),
        2,
        "mutation was retried"
    );
    drop(client);
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn unsupported_worker_bodies_are_refused_before_sending() {
    let (client, server) = pair(|_| async { panic!("invalid body reached coordinator") }).await;
    let builder = reqwest::Client::new();
    let streaming = reqwest::Body::wrap_stream(futures::stream::once(async {
        Ok::<_, std::io::Error>(Bytes::from_static(b"x"))
    }));
    for body in [
        streaming,
        reqwest::Body::from(vec![0; capsem_proto::MCP_FRAME_MAX_SIZE + 1]),
    ] {
        let error = client
            .execute(builder.post("http://server.invalid/mcp").body(body).build().unwrap())
            .await
            .err()
            .unwrap();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
    drop(client);
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn confined_mcp_worker_uses_http_without_ambient_network_or_file_access() {
    let (coordinator, worker) = std::os::unix::net::UnixStream::pair().unwrap();
    coordinator.set_nonblocking(true).unwrap();
    let coordinator = tokio::net::UnixStream::from_std(coordinator).unwrap();
    let server = tokio::spawn(async move {
        let service = service_fn(|request: http::Request<Incoming>| async move {
            assert_eq!(request.uri(), "https://configured.example/mcp");
            Ok::<_, Infallible>(http::Response::new(full("confined-tool-response")))
        });
        let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(coordinator), service)
            .await;
    });
    let directory = tempfile::tempdir().unwrap();
    let mut command = tokio::process::Command::new(crate::tests::sandbox_test_executable(directory.path()));
    command
        .env_clear()
        .args(["--exact", "http_transport::tests::confined_http_child", "--nocapture"])
        .env("CAPSEM_MCP_HTTP_CONFINEMENT_PROBE", "1")
        .stdin(std::process::Stdio::from(std::os::fd::OwnedFd::from(worker)))
        .kill_on_drop(true);
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    let status = tokio::time::timeout(std::time::Duration::from_secs(5), command.status())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success(), "confined HTTP probe failed: {status}");
    drop(command);
    tokio::time::timeout(std::time::Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn confined_http_child() {
    if std::env::var_os("CAPSEM_MCP_HTTP_CONFINEMENT_PROBE").is_none() {
        return;
    }
    use capsem_foundation::unix::worker_sandbox::{confine, Policy, Role};
    let channel = capsem_foundation::unix::fd::duplicate(std::io::stdin().as_fd()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = runtime
        .block_on(BrokerHttpClient::connect(
            std::os::unix::net::UnixStream::from(channel),
            std::time::Duration::from_secs(2),
        ))
        .unwrap();
    let request = reqwest::Client::new()
        .get("https://configured.example/mcp")
        .build()
        .unwrap();
    confine(&Policy::new(Role::McpBuiltin)).unwrap();
    let body = runtime.block_on(async { client.execute(request).await.unwrap().bytes().await.unwrap() });
    assert_eq!(body, "confined-tool-response");
    std::process::exit(0);
}
