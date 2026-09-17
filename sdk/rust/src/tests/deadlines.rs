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
                let operation = if path == "/vms/vm-1/info" {
                    "getVmInfo"
                } else {
                    "execVm"
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
