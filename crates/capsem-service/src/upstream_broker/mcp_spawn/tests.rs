use super::*;
use crate::instance::WorkerAuthority;
use capsem_core::net::policy_config::{ActivePolicyFile, SettingsFile};
use std::time::Duration;
use tokio::io::AsyncReadExt as _;

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
    (decode_spawn_record(&frame.bytes).unwrap(), frame.fds)
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
