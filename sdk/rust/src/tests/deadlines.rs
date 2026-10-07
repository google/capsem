//! Exec and run answer only when the command ends (service default: one
//! hour). A client whose only deadline was the 30 s default gave up while the
//! command kept running, and a retrying agent started it a second time.
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::fixture::reply;
use crate::*;

/// A gateway that answers every request after `delay`, counting requests.
async fn slow_gateway(delay: Duration) -> (String, tokio::sync::mpsc::UnboundedReceiver<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (seen, requests) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    if socket.read_exact(&mut byte).await.is_err() {
                        return;
                    }
                    head.push(byte[0]);
                }
                let head = String::from_utf8(head).unwrap();
                let path = head.split(' ').nth(1).unwrap().to_owned();
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(str::to_owned)
                    })
                    .map_or(0, |value| value.trim().parse::<usize>().unwrap());
                let mut body = vec![0; length];
                socket.read_exact(&mut body).await.unwrap();
                let _ = seen.send(path.clone());
                tokio::time::sleep(delay).await;
                let operation = match path.as_str() {
                    "/vms/vm-1/info" => "getVmInfo",
                    "/vms/create" => "createVm",
                    "/vms/vm-1/start" => "startVm",
                    "/vms/vm-1/resume" => "resumeVm",
                    _ => "execVm",
                };
                let payload = serde_json::to_vec(&reply(operation)).unwrap();
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.write_all(&payload).await;
            });
        }
    });
    (url, requests)
}

#[tokio::test]
async fn create_outlives_the_transport_deadline_without_replaying() {
    let (url, mut requests) = slow_gateway(Duration::from_millis(300)).await;
    let hv = Hypervisor::new(&url, "private-token")
        .unwrap()
        .with_timeout(Duration::from_millis(50))
        .unwrap();
    let vm = hv
        .create(CreateOptions {
            image: Some("docker://busybox:latest".into()),
            ..Default::default()
        })
        .await
        .expect("creation covers the service readiness window");
    assert_eq!(vm.id(), Some("vm-1"));
    assert_eq!(requests.recv().await.as_deref(), Some("/vms/create"));
    assert!(requests.try_recv().is_err());
}

#[tokio::test]
async fn restore_outlives_the_transport_deadline_without_replaying() {
    let (url, mut requests) = slow_gateway(Duration::from_millis(300)).await;
    let vm = VM::new(&url, "private-token", VmSelector::Id("vm-1".into()))
        .unwrap()
        .with_timeout(Duration::from_millis(50))
        .unwrap();
    let started = vm.start().await;
    let resumed = vm.resume().await;
    assert!(
        started.is_ok() && resumed.is_ok(),
        "readiness results: {started:?} / {resumed:?}"
    );
    assert_eq!(requests.recv().await.as_deref(), Some("/vms/vm-1/start"));
    assert_eq!(requests.recv().await.as_deref(), Some("/vms/vm-1/resume"));
    assert!(requests.try_recv().is_err(), "neither mutation is replayed");
}

#[test]
fn create_options_preserve_readiness_and_a_larger_client_deadline() {
    for (configured, expected) in [(30, 230), (500, 500)] {
        let mut client = crate::client::Client::new("http://127.0.0.1:1", "token").unwrap();
        client.set_timeout(Duration::from_secs(configured)).unwrap();
        assert_eq!(client.create_options().timeout, Some(Duration::from_secs(expected)));
    }
}

#[tokio::test]
async fn exec_and_run_outlive_the_default_deadline_without_replaying() {
    let (url, mut requests) = slow_gateway(Duration::from_millis(300)).await;
    let short = Duration::from_millis(50);
    let vm = VM::new(&url, "private-token", VmSelector::Id("vm-1".into()))
        .unwrap()
        .with_timeout(short)
        .unwrap();
    vm.exec("slow build", Some(600))
        .await
        .expect("exec outlives the default deadline");
    let hv = Hypervisor::new(&url, "private-token")
        .unwrap()
        .with_timeout(short)
        .unwrap();
    hv.run("slow build", RunOptions::default())
        .await
        .expect("run outlives the default deadline");
    assert_eq!(requests.recv().await.unwrap(), "/vms/vm-1/exec");
    assert_eq!(requests.recv().await.unwrap(), "/run");

    let result = vm.info().await;
    assert!(matches!(result, Err(Error::Transport(error)) if error.is_timeout()));
    assert_eq!(requests.recv().await.unwrap(), "/vms/vm-1/info");
    assert!(requests.try_recv().is_err(), "nothing is replayed");
}
