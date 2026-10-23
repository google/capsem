use super::*;
use crate::security_engine::approval::client::{frame_limit, ApprovalCapability, ApprovalClient};
use capsem_foundation::ipc_channel;
use capsem_proto::approval::{ApprovalClientMessage, ApprovalOutcome, ApprovalServerMessage};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guest_disconnect_cancels_original_dns_approval_without_upstream_lookup() {
    original_dns_case(DnsCase::Disconnect).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn query_cancellation_keeps_the_dns_session_and_other_queries_alive() {
    original_dns_case(DnsCase::QueryCancellation).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dns_session_forwards_only_registered_original_receipt_before_the_final_answer() {
    original_dns_case(DnsCase::Registered).await;
}

#[derive(PartialEq, Eq)]
enum DnsCase {
    Disconnect,
    QueryCancellation,
    Registered,
}

async fn original_dns_case(mode: DnsCase) {
    let cancel_query = mode == DnsCase::QueryCancellation;
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
    let directory = tempfile::tempdir().unwrap();
    let db = Arc::new(capsem_logger::DbWriter::open(&directory.path().join("session.db"), 16).unwrap());
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
    let owner = tokio::spawn(serve_dns_session(
        OwnedFd::from(host),
        Arc::clone(&handler),
        Arc::clone(&db),
        policy,
    ));
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
    let ApprovalClientMessage::Begin { request_id, request } =
        tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("begin")
    };
    if mode == DnsCase::Registered {
        let registration = capsem_proto::approval::ApprovalRegistration {
            ask_id: request.ask_id.clone(),
            event_id: request.event_id.clone(),
            remaining_ms: 1000,
            expires_at_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64
                + 1000,
        };
        let expected = registration.clone();
        sender
            .send(ApprovalServerMessage::Registered {
                request_id,
                registration,
            })
            .await
            .unwrap();
        guest.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        guest = tokio::task::spawn_blocking(move || {
            let mut length = [0;4];
            guest.read_exact(&mut length).unwrap();
            let mut payload = vec![0;u32::from_be_bytes(length) as usize];
            guest.read_exact(&mut payload).unwrap();
            assert!(matches!(capsem_proto::decode_dns_session_reply(&payload).unwrap(),
                capsem_proto::dns_session::DnsSessionReply::Pending {id,registration} if id.get()==1 && registration==expected));
            guest
        }).await.unwrap();
        let mut packet = [0; 512];
        assert!(
            tokio::time::timeout(Duration::from_millis(20), upstream.recv_from(&mut packet))
                .await
                .is_err(),
            "receipt cannot authorize lookup"
        );
        sender
            .send(ApprovalServerMessage::Resolved {
                request_id,
                outcome: ApprovalOutcome::Denied,
            })
            .await
            .unwrap();
        guest = tokio::task::spawn_blocking(move || {
            let answer = read_response(&mut guest).unwrap();
            assert_eq!(answer.id, 1);
            assert_eq!(answer.rcode, 3);
            guest
        })
        .await
        .unwrap();
    } else {
        if cancel_query {
            guest
                .write_all(
                    &capsem_proto::dns_session::encode_dns_cancellation(
                        &capsem_proto::dns_session::DnsQueryCancellation {
                            cancel_query_id: std::num::NonZeroU32::new(999).unwrap(),
                        },
                    )
                    .unwrap(),
                )
                .unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(50), receiver.recv())
                    .await
                    .is_err(),
                "another correlation ID cannot cancel the held query"
            );
            guest
                .write_all(
                    &capsem_proto::dns_session::encode_dns_cancellation(
                        &capsem_proto::dns_session::DnsQueryCancellation {
                            cancel_query_id: std::num::NonZeroU32::new(1).unwrap(),
                        },
                    )
                    .unwrap(),
                )
                .unwrap();
        } else {
            guest.shutdown(std::net::Shutdown::Both).unwrap();
        }
        let ApprovalClientMessage::Cancel { request_id: cancelled } =
            tokio::time::timeout(Duration::from_secs(2), receiver.recv())
                .await
                .unwrap()
                .unwrap()
        else {
            panic!("cancel")
        };
        assert_eq!(cancelled, request_id);
        if cancel_query {
            guest.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            guest
                .write_all(&request_frame(2, super::query(2, "still-alive")))
                .unwrap();
            guest = tokio::task::spawn_blocking(move || {
                let response = read_response(&mut guest).expect("same session must answer an independent query");
                assert_eq!(response.id, 2);
                guest
            })
            .await
            .unwrap();
        }
    }
    drop(guest);
    tokio::time::timeout(Duration::from_secs(2), owner)
        .await
        .expect("cancellation drains the original query audit")
        .unwrap();
    db.flush_checked().await.unwrap();
    let ledger = db
        .reader()
        .unwrap()
        .query_raw("SELECT event_id FROM dns_events")
        .unwrap();
    let ledger: serde_json::Value = serde_json::from_str(&ledger).unwrap();
    assert_eq!(
        ledger["rows"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row[0] == request.event_id)
            .count(),
        1
    );
    let _ = sender
        .send(ApprovalServerMessage::Resolved {
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
