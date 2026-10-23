use super::*;
use crate::job_store::FileReadState;
use capsem_core::net::policy_config::{SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource};
use capsem_proto::approval::{ApprovalClientMessage, ApprovalOutcome, ApprovalServerMessage};
use std::sync::RwLock;
use std::time::Duration;

mod capacity;
mod lifecycle;

#[tokio::test]
async fn original_guest_export_holds_its_bytes_for_the_bound_approval() {
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(DbWriter::open(&temp.path().join("session.db"), 64).unwrap());
    let peer = crate::tests::ApprovalPeer::new().await;
    let profile = SecurityRuleProfile::parse_toml(
        r#"
        [profiles.rules.ask_export]
        name = "ask_export"
        action = "ask"
        match = 'file.export.path == "/workspace/private.txt"'
    "#,
    )
    .unwrap();
    let rules = Arc::new(RwLock::new(Arc::new(
        SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap(),
    )));
    let plugins = Arc::new(RwLock::new(Arc::new(Default::default())));
    let jobs = Arc::new(JobStore::new());
    let state = Arc::new(FileReadState {
        approvals: Some(Arc::clone(&peer.capability)),
        ..Default::default()
    });
    // The callback begins only after the physical FileContent frame arrived.
    *state.arrived.lock().unwrap() = true;
    state.received.notify_one();
    assert!(!state.cancellation.is_cancelled());
    assert!(state.approvals.is_some());
    let (sender, mut result) = tokio::sync::oneshot::channel();
    jobs.jobs.lock().unwrap().insert(7, sender);
    jobs.active_file_ops.lock().unwrap().insert(
        7,
        ActiveFileOp::Read {
            path: "/workspace/private.txt".into(),
            state: Arc::clone(&state),
        },
    );
    let worker = tokio::spawn(complete(
        7,
        ActiveFileOp::Read {
            path: "/workspace/private.txt".into(),
            state,
        },
        b"private export data".to_vec(),
        jobs,
        db,
        rules,
        plugins,
    ));
    let ApprovalClientMessage::Begin { request_id, request } =
        tokio::time::timeout(Duration::from_secs(2), peer.receiver.recv())
            .await
            .expect("original export must be offered before bytes leave the callback")
            .unwrap()
    else {
        panic!("begin")
    };
    assert!(request.held_bytes >= (b"private export data".len() * 4) as u64);
    assert!(!request.summary.contains("private"));
    assert!(tokio::time::timeout(Duration::from_millis(20), &mut result)
        .await
        .is_err());
    peer.sender
        .send(ApprovalServerMessage {
            request_id,
            outcome: ApprovalOutcome::Approved,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), worker)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(result.await.unwrap(), JobResult::ReadFile { data: Some(data), error: None } if data == b"private export data")
    );
}
