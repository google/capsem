use super::*;

// SAFETY: only the explicit isolated child enters here. Confinement must run
// on its initial thread before libtest creates a sibling on pre-TSYNC kernels.
#[used]
#[cfg_attr(target_os = "linux", unsafe(link_section = ".init_array"))]
#[cfg_attr(target_os = "macos", unsafe(link_section = "__DATA,__mod_init_func"))]
static NATIVE_PROBE_ENTRY: extern "C" fn() = run_native_probe;

extern "C" fn run_native_probe() {
    if std::env::var_os("CAPSEM_NATIVE_MCP_PROBE").is_some() {
        native_worker_child();
        std::process::exit(0);
    }
}

fn scope(role: McpHttpRole) -> McpWorkerBootstrap {
    McpWorkerBootstrap {
        role,
        generation: [7; 16],
    }
}

#[tokio::test]
async fn startup_rejects_wrong_role_direction_and_descriptor_shape() {
    for record in [
        McpWorkerRecord::Ready(scope(McpHttpRole::Builtin)),
        McpWorkerRecord::Bootstrap(scope(McpHttpRole::Aggregator)),
        McpWorkerRecord::Bootstrap(scope(McpHttpRole::Builtin)),
    ] {
        let (parent, worker) = UnixStream::pair().unwrap();
        let sender = DescriptorSender::<MCP_WORKER_FRAME_SIZE, MCP_WORKER_MAX_FDS>::new(parent).unwrap();
        sender.send(&encode_worker_record(record).unwrap(), &[]).await.unwrap();
        assert!(PreparedMcpWorker::adopt(worker, McpHttpRole::Builtin).await.is_err());
    }
}

#[tokio::test]
async fn parent_rejects_stale_generation_readiness() {
    let (parent, worker) = UnixStream::pair().unwrap();
    let receiver = DescriptorReceiver::<MCP_WORKER_FRAME_SIZE, MCP_WORKER_MAX_FDS>::new(UnixStream::from(
        fd::duplicate(worker.as_fd()).unwrap(),
    ))
    .unwrap();
    let sender = DescriptorSender::<MCP_WORKER_FRAME_SIZE, MCP_WORKER_MAX_FDS>::new(worker).unwrap();
    let (http, _peer) = UnixStream::pair().unwrap();
    let child = tokio::spawn(async move {
        let _ = receiver.recv().await.unwrap();
        let mut stale = scope(McpHttpRole::Builtin);
        stale.generation = [6; 16];
        sender
            .send(&encode_worker_record(McpWorkerRecord::Ready(stale)).unwrap(), &[])
            .await
            .unwrap();
    });
    let error = grant_worker(&parent, scope(McpHttpRole::Builtin), &[http.as_fd()])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("generation or role mismatch"));
    child.await.unwrap();
}

#[tokio::test]
async fn builtin_spawn_capability_returns_only_a_connected_peer() {
    let (parent, worker) = UnixStream::pair().unwrap();
    let receiver = DescriptorReceiver::<MCP_SPAWN_FRAME_SIZE, MCP_SPAWN_MAX_FDS>::new(UnixStream::from(
        fd::duplicate(parent.as_fd()).unwrap(),
    ))
    .unwrap();
    let sender = DescriptorSender::<MCP_SPAWN_FRAME_SIZE, MCP_SPAWN_MAX_FDS>::new(parent).unwrap();
    let (peer, _tool) = UnixStream::pair().unwrap();
    let server = tokio::spawn(async move {
        let frame = receiver.recv().await.unwrap();
        assert!(frame.fds.is_empty());
        assert_eq!(
            decode_spawn_record(&frame.bytes).unwrap(),
            McpSpawnRecord::Builtin {
                request_id: 1,
                peer_index: 0
            }
        );
        sender
            .send(
                &encode_spawn_record(McpSpawnRecord::Granted { request_id: 1 }).unwrap(),
                &[peer.as_raw_fd()],
            )
            .await
            .unwrap();
        let receipt = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("client must acknowledge socket adoption")
            .unwrap();
        assert!(receipt.fds.is_empty());
        assert_eq!(
            decode_spawn_record(&receipt.bytes).unwrap(),
            McpSpawnRecord::Adopted { request_id: 1 }
        );
    });
    let client = McpSpawnClient::new(worker).unwrap();
    let peer = client.builtin(0).await.unwrap();
    fd::validate_connected_stream(peer.as_fd()).unwrap();
    assert!(client
        .builtin(capsem_proto::MCP_BUILTIN_PEERS_PER_SESSION as u32)
        .await
        .is_err());
    server.await.unwrap();
}

#[tokio::test]
async fn malformed_spawn_reply_closes_capability_instead_of_reusing_old_authority() {
    let (parent, worker) = UnixStream::pair().unwrap();
    let receiver = DescriptorReceiver::<MCP_SPAWN_FRAME_SIZE, MCP_SPAWN_MAX_FDS>::new(UnixStream::from(
        fd::duplicate(parent.as_fd()).unwrap(),
    ))
    .unwrap();
    let sender = DescriptorSender::<MCP_SPAWN_FRAME_SIZE, MCP_SPAWN_MAX_FDS>::new(parent).unwrap();
    let server = tokio::spawn(async move {
        let _ = receiver.recv().await.unwrap();
        sender
            .send(
                &encode_spawn_record(McpSpawnRecord::Granted { request_id: 9 }).unwrap(),
                &[],
            )
            .await
            .unwrap();
        assert!(receiver.recv().await.is_err());
    });
    let client = McpSpawnClient::new(worker).unwrap();
    assert!(client.builtin(0).await.is_err());
    assert!(client.builtin(0).await.unwrap_err().to_string().contains("closed"));
    server.await.unwrap();
}

#[tokio::test]
async fn real_worker_acknowledges_only_after_confinement_and_uses_its_http_capability() {
    use http_body_util::Full;
    use hyper_util::rt::{TokioExecutor, TokioIo};
    let (parent, child_control) = UnixStream::pair().unwrap();
    let (broker, child_http) = UnixStream::pair().unwrap();
    broker.set_nonblocking(true).unwrap();
    let broker = tokio::net::UnixStream::from_std(broker).unwrap();
    let http = tokio::spawn(async move {
        let service = hyper::service::service_fn(|request: http::Request<hyper::body::Incoming>| async move {
            assert_eq!(request.uri(), "https://assigned.example/mcp");
            Ok::<_, std::convert::Infallible>(http::Response::new(Full::new(bytes::Bytes::from_static(
                b"READY_WITH_SANDBOX",
            ))))
        });
        let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(broker), service)
            .await;
    });
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .env_clear()
        .args(["--exact", "native_worker::tests::native_worker_child", "--nocapture"])
        .env("CAPSEM_NATIVE_MCP_PROBE", "1")
        .stdin(std::process::Stdio::from(OwnedFd::from(child_control)))
        .kill_on_drop(true);
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    let mut child = command.spawn().unwrap();
    drop(command);
    grant_worker(&parent, scope(McpHttpRole::Builtin), &[child_http.as_fd()])
        .await
        .unwrap();
    drop(child_http);
    assert!(tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap()
        .success());
    drop(parent);
    tokio::time::timeout(Duration::from_secs(1), http)
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn native_worker_child() {
    if std::env::var_os("CAPSEM_NATIVE_MCP_PROBE").is_none() {
        return;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let prepared = runtime
        .block_on(PreparedMcpWorker::from_stdin(McpHttpRole::Builtin))
        .unwrap();
    let request = reqwest::Client::new()
        .get("https://assigned.example/mcp")
        .build()
        .unwrap();
    let worker = runtime.block_on(prepared.confine_and_ready()).unwrap();
    let body = runtime.block_on(async { worker.http.execute(request).await.unwrap().bytes().await.unwrap() });
    assert_eq!(body, "READY_WITH_SANDBOX");
    std::process::exit(0);
}
