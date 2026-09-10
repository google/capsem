use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::net::UnixStream;
use std::time::Duration;

fn key(id: u64) -> FlowKey {
    FlowKey { generation: 1, id }
}

#[test]
fn abort_closes_only_the_matching_generation_and_connection() {
    let mut bridge = Bridge::new().unwrap();
    let mut peers = Vec::new();
    for id in 1..=2 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (tcp, _) = listener.accept().unwrap();
        let (vsock, mut host) = UnixStream::pair().unwrap();
        client.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        bridge.connect_with(key(id), move || Ok((tcp, vsock))).unwrap();
        host.write_all(b"ready").unwrap();
        client.read_exact(&mut [0; 5]).unwrap();
        peers.push((client, host));
    }
    bridge
        .abort(&[capsem_proto::router::FlowKey { generation: 2, id: 1 }])
        .unwrap();
    peers[0].1.write_all(b"live").unwrap();
    peers[0].0.read_exact(&mut [0; 4]).unwrap();
    bridge
        .abort(&[capsem_proto::router::FlowKey { generation: 1, id: 1 }])
        .unwrap();
    assert_eq!(
        peers[0].0.read(&mut [0]).unwrap_err().kind(),
        io::ErrorKind::ConnectionReset,
        "matching flow did not reset"
    );
    peers[1].1.write_all(b"still live").unwrap();
    peers[1].0.read_exact(&mut [0; 10]).unwrap();
    bridge.shutdown();
    assert_eq!(bridge.connections.available_permits(), 64);
    assert!(bridge.tasks.is_empty());
}

#[test]
fn abort_reclaims_queued_setup_without_starting_a_worker() {
    let mut bridge = Bridge::new().unwrap();
    let held = bridge.setups.clone().try_acquire_many_owned(8).unwrap();
    let (entered, starts) = std::sync::mpsc::sync_channel(1);
    bridge
        .connect_with(key(1), move || {
            entered.send(()).unwrap();
            Err(io::Error::other("canceled setup must never start"))
        })
        .unwrap();
    assert_eq!(
        bridge.connect_with(key(1), || unreachable!()).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert!(bridge
        .abort(&vec![key(1); capsem_proto::router::MAX_ABORT_FLOWS + 1])
        .is_err());
    bridge.abort(&[key(1)]).unwrap();
    let all = bridge.runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(1),
            bridge.connections.clone().acquire_many_owned(64),
        )
        .await
        .unwrap()
        .unwrap()
    });
    assert!(starts.try_recv().is_err(), "aborted queued setup started");
    drop(all);
    drop(held);
    bridge.shutdown();
    assert!(bridge.flows.is_empty());
}

#[test]
fn shutdown_closes_live_flows_and_joins_them_before_returning() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (tcp, _) = listener.accept().unwrap();
    let (vsock, mut host) = UnixStream::pair().unwrap();
    let mut bridge = Bridge::new().unwrap();
    bridge.connect_with(key(1), move || Ok((tcp, vsock))).unwrap();
    host.write_all(b"ready").unwrap();
    client.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let mut ready = [0; 5];
    client.read_exact(&mut ready).unwrap();
    assert_eq!(&ready, b"ready");
    host.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
    bridge.shutdown();
    assert_eq!(host.read(&mut [0]).unwrap(), 0);
    assert_eq!(
        client.read(&mut [0]).unwrap_err().kind(),
        io::ErrorKind::ConnectionReset
    );
    assert!(bridge.tasks.is_empty());
}

#[test]
fn cancellation_joins_disposable_setup_workers_and_refuses_new_work() {
    let mut bridge = Bridge::new().unwrap();
    let (entered, started) = std::sync::mpsc::sync_channel(1);
    let (release, wait) = std::sync::mpsc::sync_channel(1);
    bridge
        .connect_with(key(1), move || {
            entered.send(std::thread::current().name().unwrap().to_owned()).unwrap();
            wait.recv_timeout(Duration::from_secs(2)).unwrap();
            Err(io::Error::from(io::ErrorKind::ConnectionRefused))
        })
        .unwrap();
    assert_eq!(
        started.recv_timeout(Duration::from_secs(1)).unwrap(),
        "capsem-port-connect"
    );
    let (done, completed) = std::sync::mpsc::sync_channel(1);
    let owner = std::thread::spawn(move || {
        bridge.shutdown();
        done.send(()).unwrap();
        bridge
    });
    assert!(matches!(
        completed.recv_timeout(Duration::from_millis(30)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    release.send(()).unwrap();
    completed.recv_timeout(Duration::from_secs(1)).unwrap();
    let mut bridge = owner.join().unwrap();
    assert!(bridge.tasks.is_empty());
    assert_eq!(bridge.connections.available_permits(), 64);
    assert_eq!(bridge.setups.available_permits(), 8);
    let flow = capsem_proto::router::FlowKey { generation: 1, id: 2 };
    assert_eq!(
        bridge.connect(flow, 6379).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[test]
fn setup_saturation_keeps_queued_work_bounded_and_cancellation_reclaims_every_permit() {
    let mut bridge = Bridge::new().unwrap();
    let (entered, started) = std::sync::mpsc::sync_channel(64);
    let mut releases = Vec::new();
    for id in 1..=64 {
        let entered = entered.clone();
        let (release, wait) = std::sync::mpsc::sync_channel(1);
        releases.push(release);
        bridge
            .connect_with(key(id), move || {
                entered.send(id).unwrap();
                wait.recv_timeout(Duration::from_secs(2)).unwrap();
                Err(io::Error::from(io::ErrorKind::TimedOut))
            })
            .unwrap();
    }
    for _ in 0..8 {
        started.recv_timeout(Duration::from_secs(1)).unwrap();
    }
    assert!(matches!(
        started.recv_timeout(Duration::from_millis(30)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(bridge
        .connect(capsem_proto::router::FlowKey { generation: 1, id: 65 }, 6379)
        .is_err());
    bridge.stop.send_replace(true);
    for release in releases {
        // Queued setup closures may already be dropped by cancellation.
        let _ = release.send(());
    }
    bridge.shutdown();
    assert!(started.try_recv().is_err(), "canceled queued setup must never start");
    assert!(bridge.tasks.is_empty());
    assert_eq!(bridge.connections.available_permits(), 64);
    assert_eq!(bridge.setups.available_permits(), 8);
}
