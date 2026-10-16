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
            user: None,
            cwd: None,
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
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert!(
        refused.body.error.contains("no container workload"),
        "{}",
        refused.body.error
    );
    exec(&state, "plain-vm", None).await.unwrap();

    // The refused request never reached the VM owner.
    assert_eq!(
        sent_targets(owner.await.unwrap()),
        vec![("id -u".to_string(), Wire::Vm)]
    );
}
fn sleep_reply(msg: &ServiceToProcess) -> FakeProcessReply {
    let sleep = matches!(msg, ServiceToProcess::Exec { command, .. } if command == "sleep");
    Box::pin(async move {
        if sleep {
            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        }
        None
    })
}

#[tokio::test]
async fn one_shot_timeout_owner_child() {
    let Some(socket) = std::env::var_os("CAPSEM_EXEC_TIMEOUT_OWNER") else {
        return;
    };
    spawn_fake_process(StdPath::new(&socket), 2, sleep_reply).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exec_timeout_and_missing_vm_routes_return_structured_error_codes() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let settings_dir = tempfile::tempdir().unwrap();
    let (_settings_guard, _, _) = install_empty_settings_env(&settings_dir);
    let dir = tempfile::tempdir().unwrap();
    let mut state = make_test_state_owned();
    install_test_ledger_worker(&mut state, dir.path());
    let sleep_bin = state.run_dir.join("sleep-proc.sh");
    // The socket must belong to the PID provision_sandbox actually spawned.
    // Serving it in this parent beside an unrelated sleep child correctly
    // fails the worker-authority check before reaching the exec timeout.
    let executable = std::env::current_exe().unwrap();
    let quoted = format!("'{}'", executable.to_string_lossy().replace('\'', "'\"'\"'"));
    let script = format!(
        "#!/bin/sh\nsocket=\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = \"--uds-path\" ]; then shift; socket=$1; fi\n  shift\ndone\nCAPSEM_EXEC_TIMEOUT_OWNER=\"$socket\" exec {quoted} --exact tests::exec_target::one_shot_timeout_owner_child --nocapture\n"
    );
    std::fs::write(&sleep_bin, script).unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&sleep_bin, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    state.process_binary = sleep_bin;
    let state = Arc::new(state);
    install_test_runtime_assets(&state);
    let st = || State(Arc::clone(&state));
    let run_req: RunRequest = serde_json::from_value(json!({"command": "sleep", "timeout_secs": 1})).unwrap();
    let run_err = handle_run(st(), Json(run_req)).await.unwrap_err();
    assert_eq!(run_err.status, StatusCode::GATEWAY_TIMEOUT, "{run_err:?}");
    assert_eq!(run_err.body.code, Some(ErrorCode::ExecTimeout));
    assert_eq!(run_err.body.timeout_secs, Some(1));
    assert_eq!(run_err.body.error, "exec failed: IPC command timed out after 1s");

    let uds_path = dir.path().join("exec.sock");
    let owner = spawn_fake_process(&uds_path, 2, sleep_reply);
    setup_vm_with_workspace_and_uds(&state, dir.path(), "plain-vm", uds_path.clone());
    let plain = || Path("plain-vm".to_string());
    let req = |cmd: &str, t: u64| {
        let body: ExecRequest = serde_json::from_value(json!({"command": cmd, "timeout_secs": t})).unwrap();
        handle_exec(st(), plain(), Json(body))
    };
    let timeout_err = req("sleep", 1).await.unwrap_err();
    assert_eq!(timeout_err.status, StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(timeout_err.body.code, Some(ErrorCode::ExecTimeout));
    assert_eq!(timeout_err.body.timeout_secs, Some(1));
    assert_eq!(timeout_err.body.error, "exec failed: IPC command timed out after 1s");
    let failed_err = req("closed", 5).await.unwrap_err();
    assert_eq!(failed_err.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(failed_err.body.error.starts_with("IPC connection closed"));
    let _ = owner.await;
    let _ = std::fs::remove_file(&uds_path);
    let fork_req = |n: &str| Json(serde_json::from_value::<ForkRequest>(json!({"name": n})).unwrap());
    let fork_ipc_err = handle_fork(st(), plain(), fork_req("f")).await.unwrap_err();
    assert_eq!(fork_ipc_err.status, StatusCode::INTERNAL_SERVER_ERROR);
    let existing = test_persistent_entry("unresumable", dir.path().join("unresumable"));
    let existing_id = existing.id.clone();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert("unresumable".into(), existing);
    let resume_existing_err = handle_resume(st(), Path(existing_id)).await.unwrap_err();
    assert_eq!(resume_existing_err.status, StatusCode::NOT_FOUND);
    assert_eq!(resume_existing_err.body.code, None);
    assert_eq!(resume_existing_err.body.vm_id, None);
    std::fs::create_dir_all(dir.path().join("session.db")).unwrap();
    let _ = handle_vm_status(st(), plain()).await;
    let missing = || Path("missing-vm".to_string());
    let net_id = "00000000-0000-0000-0000-000000000001".to_string();
    let net_path = Path((net_id, "missing-vm".to_string()));
    let log_q: LogQuery = serde_json::from_str("{}").unwrap();
    let persist_req = Json(serde_json::from_str::<PersistRequest>(r#"{"name":"s"}"#).unwrap());
    let attach = crate::network_routes::handle_network_attach;
    let not_found_errors = [
        attach(st(), net_path).await.unwrap_err(),
        handle_preserve_failure(st(), missing()).await.unwrap_err(),
        handle_logs(st(), missing(), Query(log_q)).await.unwrap_err(),
        handle_info(st(), missing()).await.unwrap_err(),
        handle_vm_status(st(), missing()).await.unwrap_err(),
        running_uds_path(&state, "missing-vm").unwrap_err(),
        resolve_session_dir(&state, "missing-vm").unwrap_err(),
        handle_suspend(st(), missing()).await.unwrap_err(),
        handle_stop(st(), missing()).await.unwrap_err(),
        handle_delete(st(), missing()).await.unwrap_err(),
        handle_persist(st(), missing(), persist_req).await.unwrap_err(),
        handle_resume(st(), missing()).await.unwrap_err(),
        handle_fork(st(), missing(), fork_req("forked")).await.unwrap_err(),
    ];
    for err in not_found_errors {
        assert_eq!(err.status, StatusCode::NOT_FOUND);
        assert_eq!(err.body.code, Some(ErrorCode::VmNotFound));
        assert_eq!(err.body.vm_id.as_deref(), Some("missing-vm"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exec_forwards_validated_user_and_cwd_and_refuses_malformed_ones_before_ipc() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _state_dir) = make_test_state_with_tempdir();
    let uds_path = dir.path().join("exec.sock");
    let owner = spawn_fake_process(&uds_path, 1, exec_ok);
    setup_vm_with_workspace_and_uds(&state, dir.path(), "plain-vm", uds_path);

    for bad_req in [
        ExecRequest {
            command: "pwd".into(),
            user: Some("bad user".into()),
            ..Default::default()
        },
        ExecRequest {
            command: "pwd".into(),
            cwd: Some("relative/path".into()),
            ..Default::default()
        },
    ] {
        let err = handle_exec(State(Arc::clone(&state)), Path("plain-vm".into()), Json(bad_req))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    let _ = handle_exec(
        State(Arc::clone(&state)),
        Path("plain-vm".into()),
        Json(ExecRequest {
            command: "pwd".into(),
            timeout_secs: Some(5),
            target: None,
            user: Some("1000:1000".into()),
            cwd: Some("/workspace".into()),
        }),
    )
    .await
    .unwrap();

    let messages = owner.await.unwrap();
    assert_eq!(messages.len(), 1);
    match &messages[0] {
        ServiceToProcess::Exec {
            command,
            target,
            user,
            cwd,
            ..
        } => {
            assert_eq!(command, "pwd");
            assert_eq!(*target, Wire::Vm);
            assert_eq!(user.as_deref(), Some("1000:1000"));
            assert_eq!(cwd.as_deref(), Some("/workspace"));
        }
        other => panic!("expected exec, got {other:?}"),
    }
}
