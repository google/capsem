//! Which side of an image session `POST /vms/{id}/exec` runs in.
//!
//! The service resolves the target, because it is what knows whether the
//! session runs a container; the VM owner is told explicitly and never guesses.

use super::files_api::setup_vm_with_workspace_and_uds;
use super::*;
use capsem_proto::ipc::ExecTarget as Wire;

fn exec_ok(message: &ServiceToProcess) -> FakeProcessReply {
    let ServiceToProcess::Exec { id, .. } = message else {
        panic!("expected exec request, got {message:?}");
    };
    let id = *id;
    Box::pin(async move {
        Some(ProcessToService::ExecResult {
            id,
            stdout: Vec::new(),
            stderr: Vec::new(),
            exit_code: 0,
            truncated: false,
        })
    })
}

async fn exec(state: &Arc<ServiceState>, vm: &str, target: Option<capsem_api::ExecTarget>) -> Result<(), AppError> {
    handle_exec(
        State(Arc::clone(state)),
        Path(vm.to_string()),
        Json(ExecRequest {
            command: "id -u".to_string(),
            timeout_secs: Some(5),
            target,
        }),
    )
    .await
    .map(drop)
}

fn sent_targets(messages: Vec<ServiceToProcess>) -> Vec<(String, Wire)> {
    messages
        .into_iter()
        .map(|message| match message {
            ServiceToProcess::Exec { command, target, .. } => (command, target),
            other => panic!("expected exec, got {other:?}"),
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_image_session_execs_in_its_workload_unless_the_vm_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let uds_path = dir.path().join("exec.sock");
    let owner = spawn_fake_process(&uds_path, 2, exec_ok);
    setup_vm_with_workspace_and_uds(&state, dir.path(), "image-vm", uds_path);
    // What `record_launched` leaves beside the ledger of a launched workload.
    std::fs::write(
        dir.path().join("session/container.json"),
        br#"{"image":"redis","digest":"sha256:00"}"#,
    )
    .unwrap();

    exec(&state, "image-vm", None).await.unwrap();
    exec(&state, "image-vm", Some(capsem_api::ExecTarget::Vm))
        .await
        .unwrap();

    // The owner gets the caller's command untouched: it records it, then wraps it.
    assert_eq!(
        sent_targets(owner.await.unwrap()),
        vec![("id -u".to_string(), Wire::Workload), ("id -u".to_string(), Wire::Vm)]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_without_a_workload_execs_in_the_vm_and_refuses_a_workload_target() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let uds_path = dir.path().join("exec.sock");
    let owner = spawn_fake_process(&uds_path, 1, exec_ok);
    setup_vm_with_workspace_and_uds(&state, dir.path(), "plain-vm", uds_path);

    let refused = exec(&state, "plain-vm", Some(capsem_api::ExecTarget::Workload))
        .await
        .unwrap_err();
    assert_eq!(refused.0, StatusCode::BAD_REQUEST);
    assert!(refused.1.contains("no container workload"), "{}", refused.1);
    exec(&state, "plain-vm", None).await.unwrap();

    // The refused request never reached the VM owner.
    assert_eq!(
        sent_targets(owner.await.unwrap()),
        vec![("id -u".to_string(), Wire::Vm)]
    );
}
