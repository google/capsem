use super::*;
use axum::routing::get;
use std::{process::Stdio, sync::Arc};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    process::Command,
    sync::Notify,
};

#[tokio::test]
async fn shutdown_drains_the_response_that_requested_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("service.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let shutdown = Arc::new(Notify::new());
    let reply = Arc::new(Notify::new());
    let app = Router::new().route(
        "/",
        get({
            let shutdown = shutdown.clone();
            let reply = reply.clone();
            move || async move {
                shutdown.notify_one();
                reply.notified().await;
                "accepted"
            }
        }),
    );
    let (draining, observed) = oneshot::channel();
    let server = tokio::spawn(serve(listener, app, async move {
        shutdown.notified().await;
        draining.send(()).unwrap();
    }));
    let mut client = UnixStream::connect(path).await.unwrap();
    client
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), observed)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !server.is_finished(),
        "shutdown must wait for the pending acknowledgement"
    );
    reply.notify_one();
    let mut response = String::new();
    tokio::time::timeout(Duration::from_secs(2), client.read_to_string(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.ends_with("accepted"), "{response}");
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn unresponsive_request_cannot_hold_shutdown_forever() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("service.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let shutdown = Arc::new(Notify::new());
    let app = Router::new().route(
        "/",
        get({
            let shutdown = shutdown.clone();
            move || async move {
                shutdown.notify_one();
                std::future::pending::<String>().await
            }
        }),
    );
    let server = tokio::spawn(drain(
        listener,
        app,
        async move { shutdown.notified().await },
        Duration::from_millis(10),
    ));
    let mut client = UnixStream::connect(path).await.unwrap();
    client
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn companion_shutdown_waits_for_exit_and_reaps_the_child() {
    let child = Command::new("sleep").arg("60").kill_on_drop(true).spawn().unwrap();
    let pid = child.id().unwrap();
    stop_companions(vec![child]).await;
    assert_eq!(
        super::super::process_control::probe(pid).unwrap(),
        super::super::process_control::ProcessState::Gone
    );
}

#[tokio::test]
async fn companion_ignoring_sigterm_is_killed_and_reaped() {
    let mut child = Command::new("sh")
        .args(["-c", "trap '' TERM; printf 'ready\\n'; exec sleep 60"])
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    let mut ready = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut ready)
        .await
        .unwrap();
    assert_eq!(ready, "ready\n");
    tokio::time::timeout(
        Duration::from_secs(2),
        stop_companions_with_timeout(vec![child], Duration::from_millis(10)),
    )
    .await
    .unwrap();
    assert_eq!(
        super::super::process_control::probe(pid).unwrap(),
        super::super::process_control::ProcessState::Gone
    );
}
