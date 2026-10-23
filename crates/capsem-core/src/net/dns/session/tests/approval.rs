use super::*;
use crate::security_engine::approval::client::{frame_limit, ApprovalCapability, ApprovalClient};
use capsem_foundation::ipc_channel;
use capsem_proto::approval::{ApprovalClientMessage, ApprovalOutcome, ApprovalServerMessage};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guest_disconnect_cancels_original_dns_approval_without_upstream_lookup() {
    let upstream = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let profile = crate::net::policy_config::SecurityRuleProfile::parse_toml(
        r#"
        [profiles.rules.ask_dns]
        name = "ask_dns"
        action = "ask"
        match = 'dns.qname == "held.example.com"'
        "#,
    )
    .unwrap();
    let rules = crate::net::policy_config::SecurityRuleSet::compile_profile(
        &profile,
        crate::net::policy_config::SecurityRuleSource::User,
    )
    .unwrap();
    let policy = ProxyPolicyHandle::new(ProxyPolicySnapshot::new(
        "blake3:test".into(),
        crate::net::policy::NetworkMechanics::default(),
        rules,
        std::collections::BTreeMap::new(),
        crate::net::policy_config::ModelEndpointRegistry::default(),
    ));
    let db = Arc::new(capsem_logger::DbWriter::open_in_memory(16).unwrap());
    let (worker, coordinator) = UnixStream::pair().unwrap();
    let capability = Arc::new(ApprovalCapability::connected(
        ApprovalClient::from_stream(worker).unwrap(),
    ));
    let engine = Arc::new(
        crate::net::proxy_engine::ProxyEngine::local(policy.clone(), Arc::clone(&db)).with_approvals(Some(capability)),
    );
    let handler = Arc::new(
        DnsHandler::new(
            policy.clone(),
            Arc::new(DnsResolver::with_upstreams(vec![upstream.local_addr().unwrap()])),
        )
        .with_engine(engine),
    );
    let (sender, receiver) = ipc_channel::channel_from_std_with_limit::<ApprovalServerMessage, ApprovalClientMessage>(
        coordinator,
        frame_limit(),
    )
    .unwrap();
    let (mut guest, host) = UnixStream::pair().unwrap();
    let owner = tokio::spawn(serve_dns_session(OwnedFd::from(host), Arc::clone(&handler), db, policy));
    let mut query = hickory_proto::op::Message::new(
        1,
        hickory_proto::op::MessageType::Query,
        hickory_proto::op::OpCode::Query,
    );
    query.add_query(hickory_proto::op::Query::query(
        hickory_proto::rr::Name::from_ascii("held.example.com.").unwrap(),
        hickory_proto::rr::RecordType::A,
    ));
    guest.write_all(&request_frame(1, query.to_vec().unwrap())).unwrap();
    let ApprovalClientMessage::Begin { request_id, .. } = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!("begin")
    };
    drop(guest);
    tokio::time::timeout(Duration::from_secs(2), owner)
        .await
        .expect("EOF cancels the human wait and drains owned audits")
        .unwrap();
    let ApprovalClientMessage::Cancel { request_id: cancelled } =
        tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("cancel")
    };
    assert_eq!(cancelled, request_id);
    let _ = sender
        .send(ApprovalServerMessage {
            request_id,
            outcome: ApprovalOutcome::Approved,
        })
        .await;
    let mut packet = [0; 512];
    assert!(
        tokio::time::timeout(Duration::from_millis(50), upstream.recv_from(&mut packet))
            .await
            .is_err()
    );
}
