use super::*;
use crate::instance::WorkerAuthority;
use capsem_core::net::policy_config::{ActivePolicyFile, SettingsFile};
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

fn response_channel() -> (Sender, Receiver, Sender, Receiver) {
    let (parent, worker) = UnixStream::pair().unwrap();
    (
        Sender::new(parent.try_clone().unwrap()).unwrap(),
        Receiver::new(parent).unwrap(),
        Sender::new(worker.try_clone().unwrap()).unwrap(),
        Receiver::new(worker).unwrap(),
    )
}

async fn adopted(sender: &Sender, request_id: u64) {
    sender
        .send(
            &encode_spawn_record(McpSpawnRecord::Adopted { request_id }).unwrap(),
            &[],
        )
        .await
        .unwrap();
}

fn async_socket(socket: UnixStream) -> tokio::net::UnixStream {
    fd::set_nonblocking(socket.as_fd(), true).unwrap();
    tokio::net::UnixStream::from_std(socket).unwrap()
}

#[tokio::test]
async fn grant_retains_original_socket_until_adoption_receipt() {
    let (sender, responses, worker, receiver) = response_channel();
    let (control, builtin) = UnixStream::pair().unwrap();
    let mut response = tokio::spawn(async move {
        respond(
            &sender,
            &responses,
            McpSpawnRecord::Granted { request_id: 7 },
            Some(&control),
        )
        .await
    });
    let mut frame = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frame.fds.len(), 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut response)
            .await
            .is_err(),
        "SCM_RIGHTS send completion must not release the original socket before adoption"
    );
    let mut control = async_socket(UnixStream::from(frame.fds.pop().unwrap()));
    adopted(&worker, 7).await;
    tokio::time::timeout(Duration::from_secs(2), response)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let mut builtin = async_socket(builtin);
    builtin.write_all(b"ready").await.unwrap();
    let mut bytes = [0; 5];
    tokio::time::timeout(Duration::from_secs(2), control.read_exact(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&bytes, b"ready");
    drop(control);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), builtin.read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0,
        "sender retained a socket after adoption"
    );
}

#[tokio::test]
async fn grant_rejects_wrong_receipt_identity_direction_descriptors_and_eof() {
    for receipt in [
        Some((McpSpawnRecord::Adopted { request_id: 8 }, false)),
        Some((McpSpawnRecord::Denied { request_id: 7 }, false)),
        Some((
            McpSpawnRecord::Builtin {
                request_id: 7,
                peer_index: 0,
            },
            false,
        )),
        Some((McpSpawnRecord::Adopted { request_id: 7 }, true)),
        None,
    ] {
        let (sender, responses, worker, receiver) = response_channel();
        let (control, _builtin) = UnixStream::pair().unwrap();
        let response = tokio::spawn(async move {
            respond(
                &sender,
                &responses,
                McpSpawnRecord::Granted { request_id: 7 },
                Some(&control),
            )
            .await
        });
        let frame = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(frame.fds.len(), 1);
        if let Some((receipt, extra)) = receipt {
            let descriptors: Vec<_> = if extra { vec![frame.fds[0].as_raw_fd()] } else { vec![] };
            worker
                .send(&encode_spawn_record(receipt).unwrap(), &descriptors)
                .await
                .unwrap();
        }
        drop((worker, receiver));
        assert!(tokio::time::timeout(Duration::from_secs(2), response)
            .await
            .unwrap()
            .unwrap()
            .is_err());
    }
}

#[tokio::test(start_paused = true)]
async fn grant_without_adoption_expires_and_releases_its_original_socket() {
    let (sender, responses, _worker, receiver) = response_channel();
    let (control, builtin) = UnixStream::pair().unwrap();
    let response = tokio::spawn(async move {
        respond(
            &sender,
            &responses,
            McpSpawnRecord::Granted { request_id: 7 },
            Some(&control),
        )
        .await
    });
    let frame = receiver.recv().await.unwrap();
    assert_eq!(frame.fds.len(), 1);
    tokio::time::advance(STARTUP_TIMEOUT).await;
    assert!(response.await.unwrap().unwrap_err().to_string().contains("deadline"));
    drop(frame);
    let mut builtin = async_socket(builtin);
    assert_eq!(builtin.read(&mut [0]).await.unwrap(), 0);
}

fn capability(
    factory: Factory,
    policy: watch::Receiver<Arc<BrokerPolicy>>,
    authority: WorkerGrant,
) -> (Sender, Receiver, tokio::task::JoinHandle<Result<(), String>>) {
    let (coordinator, worker) = UnixStream::pair().unwrap();
    let sender = Sender::new(worker.try_clone().unwrap()).unwrap();
    let receiver = Receiver::new(worker).unwrap();
    (
        sender,
        receiver,
        tokio::spawn(serve(coordinator, factory, policy, authority)),
    )
}

async fn request(sender: &Sender, receiver: &Receiver, id: u64, peer: u32) -> (McpSpawnRecord, Vec<OwnedFd>) {
    sender
        .send(
            &encode_spawn_record(McpSpawnRecord::Builtin {
                request_id: id,
                peer_index: peer,
            })
            .unwrap(),
            &[],
        )
        .await
        .unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    let record = decode_spawn_record(&frame.bytes).unwrap();
    if let McpSpawnRecord::Granted { request_id } = record {
        assert_eq!(frame.fds.len(), 1);
        fd::validate_connected_stream(frame.fds[0].as_fd()).unwrap();
        adopted(sender, request_id).await;
    }
    (record, frame.fds)
}

#[tokio::test]
async fn unacknowledged_builtin_is_reaped_on_revocation_or_spawn_channel_eof() {
    for revoke in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let active =
            ActivePolicyFile::from_settings_and_corp(&SettingsFile::default(), &SettingsFile::default()).unwrap();
        let (_publisher, updates) = watch::channel(BrokerPolicy::new(
            "test-policy".into(),
            Arc::new(active.compile_runtime().unwrap()),
        ));
        let authority = WorkerAuthority::default();
        let capacity = Arc::new(Semaphore::new(1));
        let (sender, receiver, task) = capability(
            Factory::prepare(directory.path(), capacity.clone()).unwrap(),
            updates,
            authority.grant(),
        );
        sender
            .send(
                &encode_spawn_record(McpSpawnRecord::Builtin {
                    request_id: 1,
                    peer_index: 0,
                })
                .unwrap(),
                &[],
            )
            .await
            .unwrap();
        let mut frame = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            decode_spawn_record(&frame.bytes).unwrap(),
            McpSpawnRecord::Granted { request_id: 1 }
        );
        let mut control = async_socket(UnixStream::from(frame.fds.pop().unwrap()));
        let channel = (sender, receiver);
        let _held_channel = if revoke {
            authority.revoke();
            Some(channel)
        } else {
            drop(channel);
            None
        };
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(capacity.available_permits(), 1, "peer was not positively reaped");
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), control.read(&mut [0]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn mcp_spawn_bounds_peer_authority_and_reaps_a_live_builtin_when_its_channel_closes() {
    let directory = tempfile::tempdir().unwrap();
    let active = ActivePolicyFile::from_settings_and_corp(&SettingsFile::default(), &SettingsFile::default()).unwrap();
    let (_publisher, updates) = watch::channel(BrokerPolicy::new(
        "test-policy".into(),
        Arc::new(active.compile_runtime().unwrap()),
    ));
    let authority = WorkerAuthority::default();
    let capacity = Arc::new(Semaphore::new(1));
    let (first_send, first_recv, first_task) = capability(
        Factory::prepare(directory.path(), capacity.clone()).unwrap(),
        updates.clone(),
        authority.grant(),
    );
    let (second_send, second_recv, second_task) = capability(
        Factory::prepare(directory.path(), capacity).unwrap(),
        updates,
        authority.grant(),
    );
    let (response, descriptors) = request(
        &first_send,
        &first_recv,
        1,
        capsem_proto::MCP_BUILTIN_PEERS_PER_SESSION as u32,
    )
    .await;
    assert_eq!(response, McpSpawnRecord::Denied { request_id: 1 });
    assert!(descriptors.is_empty());
    let (response, mut descriptors) = request(&first_send, &first_recv, 2, 0).await;
    assert_eq!(
        response,
        McpSpawnRecord::Granted { request_id: 2 },
        "builtin log: {}",
        std::fs::read_to_string(directory.path().join("mcp-builtin.stderr.log")).unwrap_or_default()
    );
    assert_eq!(descriptors.len(), 1);
    let control = UnixStream::from(descriptors.pop().unwrap());
    let (response, descriptors) = request(&second_send, &second_recv, 1, 0).await;
    assert_eq!(response, McpSpawnRecord::Denied { request_id: 1 });
    assert!(descriptors.is_empty(), "exhausted budget granted another child");
    // Keep the builtin's connected RPC descriptor open. Only broker EOF,
    // not a voluntary MCP disconnect, may terminate/reap this live peer.
    drop((first_send, first_recv));
    tokio::time::timeout(Duration::from_secs(2), first_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    fd::set_nonblocking(control.as_fd(), true).unwrap();
    let mut control = tokio::net::UnixStream::from_std(control).unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), control.read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    drop((second_send, second_recv));
    tokio::time::timeout(Duration::from_secs(2), second_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
