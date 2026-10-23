//! The service publishes a receipt only after canonical registration succeeds.
use super::*;
use capsem_foundation::ipc_channel;
use capsem_proto::approval::{ApprovalClientMessage, ApprovalServerMessage};
use std::os::unix::net::UnixStream;

#[tokio::test]
async fn only_verified_live_registration_has_a_bound_receipt_and_cancellation_is_audited() {
    for valid in [true, false] {
        let (runtime, session, _db, _authority, _directory) = fixture().await;
        let active = capsem_core::net::policy_config::ActivePolicyFile::from_settings_and_corp(
            &capsem_core::net::policy_config::SettingsFile::default(),
            &capsem_core::net::policy_config::SettingsFile::default(),
        )
        .unwrap();
        let policy = crate::upstream_broker::BrokerPolicy::new(
            format!("blake3:{}", "a".repeat(64)),
            Arc::new(active.compile_runtime().unwrap()),
        );
        let (_publisher, policy) = tokio::sync::watch::channel(policy);
        let (worker, service) = UnixStream::pair().unwrap();
        let owner = tokio::spawn(super::super::capability::server(service, Arc::clone(&session), policy));
        let (sender, receiver) =
            ipc_channel::channel_from_std_with_limit::<ApprovalClientMessage, ApprovalServerMessage>(
                worker,
                capsem_core::security_engine::approval::client::frame_limit(),
            )
            .unwrap();
        let mut original = request();
        if !valid {
            original.event_id = "ffffffffffff".into();
        }
        sender
            .send(ApprovalClientMessage::Begin {
                request_id: 71,
                request: original.clone(),
            })
            .await
            .unwrap();
        let first = receiver.recv().await.unwrap();
        if valid {
            let ApprovalServerMessage::Registered {
                request_id,
                registration,
            } = first
            else {
                panic!("verified registration must have a receipt")
            };
            assert_eq!(request_id, 71);
            assert_eq!(registration.ask_id, original.ask_id);
            assert_eq!(registration.event_id, original.event_id);
            let snapshot = runtime.broker.active(&session.binding, &original.ask_id).unwrap();
            assert_eq!(registration.expires_at_unix_ms, snapshot.expires_ms);
            assert!(registration.remaining_ms > 0 && registration.remaining_ms <= 30_000);
            sender
                .send(ApprovalClientMessage::Cancel { request_id: 71 })
                .await
                .unwrap();
            assert!(matches!(
                receiver.recv().await.unwrap(),
                ApprovalServerMessage::Resolved {
                    request_id: 71,
                    outcome: ApprovalOutcome::Cancelled
                }
            ));
            let record = session.record(&original.ask_id).await.unwrap().unwrap();
            assert_eq!(record.status, SecurityAskStatus::Denied);
            assert_eq!(record.reason.as_deref(), Some("request_cancelled"));
        } else {
            assert!(
                matches!(
                    first,
                    ApprovalServerMessage::Resolved {
                        request_id: 71,
                        outcome: ApprovalOutcome::Unavailable
                    }
                ),
                "unverified rows cannot extend a caller wait"
            );
            assert_eq!(runtime.broker.pending_count(&session.binding), 0);
        }
        drop(sender);
        drop(receiver);
        assert!(owner.await.unwrap().is_ok());
    }
}
