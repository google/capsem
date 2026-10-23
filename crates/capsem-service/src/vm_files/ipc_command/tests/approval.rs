use super::*;
use capsem_proto::approval::ApprovalRegistration;
use capsem_proto::ipc::{ExecApprovalProgress, ExecTarget};
use std::time::Duration;

#[tokio::test]
async fn registered_original_exec_survives_human_wait_beyond_ordinary_ipc_budget() {
    let directory = tempfile::tempdir().unwrap();
    let state = crate::tests::make_test_state();
    let path = directory.path().join("owner.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    register_owner(&state, "original", &path);
    let server = tokio::spawn(async move {
        let (tx, rx) = peer_channel(accept_and_handshake(&listener).await);
        assert!(matches!(rx.recv().await.unwrap(), ServiceToProcess::Exec { id: 7, .. }));
        let registration = ApprovalRegistration {
            ask_id: "0123456789ab".into(),
            event_id: "abcdef012345".into(),
            expires_at_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64
                + 2_000,
            remaining_ms: 2_000,
        };
        tx.send(ProcessToService::ExecApproval {
            id: 7,
            progress: ExecApprovalProgress::Registered(registration.clone()),
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        if tx
            .send(ProcessToService::ExecApproval {
                id: 7,
                progress: ExecApprovalProgress::Resolved {
                    ask_id: registration.ask_id,
                    event_id: registration.event_id,
                },
            })
            .await
            .is_ok()
        {
            let _ = tx
                .send(ProcessToService::ExecResult {
                    id: 7,
                    stdout: b"original once".to_vec(),
                    stderr: vec![],
                    exit_code: 0,
                    truncated: false,
                })
                .await;
        }
    });
    let result = send_ipc_command(
        &state,
        &path,
        ServiceToProcess::Exec {
            id: 7,
            command: "original command".into(),
            target: ExecTarget::Vm,
        },
        Some(1),
    )
    .await;
    server.await.unwrap();
    assert!(
        matches!(&result, Ok(ProcessToService::ExecResult { id: 7, stdout, .. }) if stdout == b"original once"),
        "{result:?}"
    );
}
