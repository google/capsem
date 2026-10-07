use super::*;

pub(super) fn controller_identity() -> PeerIdentity {
    PeerIdentity {
        pid: std::process::id().try_into().unwrap(),
        uid: capsem_foundation::unix::process::current_uid(),
    }
}

#[tokio::test]
async fn claimed_service_and_stream_roles_do_not_authorize_a_wrong_process() {
    for role in [
        "capsem-service",
        capsem_proto::handshake::STREAM_PEER_ID,
        "other-worker",
    ] {
        let (owner, peer) = tokio::net::UnixStream::pair().unwrap();
        let mut wrong = controller_identity();
        wrong.pid = (wrong.pid.get() + 1).try_into().unwrap();
        let hello = tokio::task::spawn_blocking(move || {
            let mut peer = peer.into_std().unwrap();
            capsem_foundation::ipc_handshake::negotiate_initiator(&mut peer, role, "")
        });
        let admitted = open_ipc_channel(owner, wrong).await.unwrap();
        let claimed = hello.await.unwrap();
        assert!(admitted.is_none(), "worker claiming {role} was admitted");
        assert!(claimed.is_err(), "unauthorized worker received a successful Hello");
    }
}

#[tokio::test]
async fn unauthorized_idle_peers_are_refused_without_waiting_for_hello() {
    let (owner, _idle) = tokio::net::UnixStream::pair().unwrap();
    let mut wrong = controller_identity();
    wrong.pid = (wrong.pid.get() + 1).try_into().unwrap();
    let admitted = tokio::time::timeout(Duration::from_millis(100), open_ipc_channel(owner, wrong))
        .await
        .expect("identity is checked before an untrusted peer may spend the Hello deadline")
        .unwrap();
    assert!(admitted.is_none());
}

#[tokio::test]
async fn a_matching_pid_with_a_wrong_uid_is_refused_before_hello() {
    let (owner, _idle) = tokio::net::UnixStream::pair().unwrap();
    let mut wrong = controller_identity();
    wrong.uid ^= 1;
    let admitted = tokio::time::timeout(Duration::from_millis(100), open_ipc_channel(owner, wrong))
        .await
        .expect("wrong user must be refused before reading Hello")
        .unwrap();
    assert!(admitted.is_none());
}
