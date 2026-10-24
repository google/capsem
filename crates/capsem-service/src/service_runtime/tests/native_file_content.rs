//! Native-only acceptance. No fixture state or raw owner authority crosses into Python.
use super::*;
use futures::FutureExt;
use serde_json::{json, Value};
use std::time::Duration;

mod cohort;
mod gateway;

const FILE: &str = "/tmp/capsem-native-approval-boundary.txt";
const BYTES: &[u8] = b"original-private-guest-file-content";
type ReadTask = tokio::task::JoinHandle<Result<ProcessToService, crate::vm_files::ipc_command::IpcCommandError>>;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn original_guest_file_content_approve_deny_and_cancel() {
    let executable = std::env::current_exe().unwrap();
    let directory = executable.parent().unwrap();
    assert_eq!(
        executable.file_name().unwrap().to_str().unwrap(),
        cohort::artifact_name()
    );
    cohort::verify(
        directory,
        &std::fs::read(directory.join("native-file-content.json")).unwrap(),
    )
    .unwrap();
    let home =
        PathBuf::from(std::env::var_os("CAPSEM_NATIVE_ACCEPTANCE_HOME").expect("native owner must isolate HOME"));
    assert_eq!(capsem_foundation::paths::capsem_home(), home);
    assert_eq!(std::env::var_os("HOME").unwrap(), home.as_os_str());
    assert!(!home.join("settings.toml").exists(), "native home must be fresh");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("settings.toml"), format!(
        "[profiles.rules.native_guest_file_content]\nname = 'native_guest_file_content'\naction = 'ask'\npriority = 10\nmatch = 'file.export.path == \"{FILE}\"'\n"
    )).unwrap();
    let run = private_run_dir(&home.join("run")).unwrap();
    let args = Args {
        foreground: true,
        uds_path: Some(run.join("service.sock")),
        process_binary: Some(directory.join("capsem-process")),
        proxy_binary: Some(directory.join("capsem-proxy")),
        ledger_binary: Some(directory.join("capsem-ledger")),
        gateway_binary: Some(directory.join("capsem-gateway")),
        gateway_port: Some(0),
        tray_binary: Some(directory.join("capsem-tray")),
        assets_dir: Some(PathBuf::from(
            std::env::var_os("CAPSEM_TEST_ASSETS_DIR").expect("qualified assets required"),
        )),
        parent_pid: None,
    };
    let (ready, observed) = tokio::sync::oneshot::channel();
    let mut service = tokio::spawn(start_and_serve(args, run.clone(), |state| {
        let _ = ready.send(state);
    }));
    let state = tokio::select! {
        result = observed => result.expect("production service did not register state"),
        result = &mut service => panic!("production startup ended before readiness: {result:?}"),
    };
    assert!(state._test_tempdir.is_none());
    let result = std::panic::AssertUnwindSafe(async {
        tokio::time::timeout(Duration::from_secs(240), prove(Arc::clone(&state), &run))
            .await
            .unwrap();
    })
    .catch_unwind()
    .await;
    state.update_restart.notify_one();
    tokio::time::timeout(Duration::from_secs(30), service)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

async fn prove(state: Arc<ServiceState>, run: &StdPath) {
    let api = gateway::Gateway::ready(run).await;
    let created = api
        .post(
            "/vms/create",
            json!({"name": "native-guest-file-content", "persistent": true, "ram_mb": 2048, "cpus": 2}),
        )
        .await;
    let id = created["id"].as_str().unwrap();
    let (owner_path, session) = state
        .instances
        .lock()
        .unwrap()
        .get(id)
        .map(|instance| (instance.uds_path.clone(), instance.session_dir.clone()))
        .unwrap();
    assert!(crate::owner_connection::OwnerConnection::acquire(&state, &run.join("unregistered.sock")).is_err());
    api.seed(id).await;
    let mut receipts = Vec::new();
    for decision in ["approve", "deny", "cancel"] {
        let original_id = state.next_job_id();
        let owner_state = Arc::clone(&state);
        let owner_path = owner_path.clone();
        let original = tokio::spawn(async move {
            crate::vm_files::send_ipc_command(
                &owner_state,
                &owner_path,
                ServiceToProcess::ReadFile {
                    id: original_id,
                    path: FILE.into(),
                },
                Some(30),
            )
            .await
        });
        let listing = api.pending(id, &original).await;
        assert!(
            !listing.to_string().contains(std::str::from_utf8(BYTES).unwrap()),
            "review leaked held bytes"
        );
        let request = listing["requests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["status"] == "pending")
            .unwrap();
        let route = format!(
            "/vms/{id}/security/asks/{}/resolve",
            request["ask_id"].as_str().unwrap()
        );
        let body = json!({"generation": listing["generation"], "decision": "approve"});
        api.rejected(&route, &body, 401, false).await;
        api.rejected(&route, &json!({"decision": "approve"}), 422, true).await;
        api.rejected(
            &route,
            &json!({"generation": capsem_crypto::uuid_v4(), "decision": "approve"}),
            409,
            true,
        )
        .await;
        let wrong = format!(
            "/vms/{}/security/asks/{}/resolve",
            capsem_crypto::uuid_v4(),
            request["ask_id"].as_str().unwrap()
        );
        api.rejected(&wrong, &body, 404, true).await;
        assert!(!original.is_finished(), "invalid resolution completed original");
        if decision == "cancel" {
            original.abort();
            assert!(original.await.unwrap_err().is_cancelled());
            api.cancelled(id, request["ask_id"].as_str().unwrap()).await;
        } else {
            let receipt = api
                .post(
                    &route,
                    json!({"generation": listing["generation"], "decision": decision}),
                )
                .await;
            assert_eq!(
                receipt["status"],
                if decision == "approve" { "approved" } else { "denied" }
            );
            match original.await.unwrap().unwrap() {
                ProcessToService::ReadFileResult { id: reply, data, error } => {
                    assert_eq!(reply, original_id);
                    assert_eq!(data.as_deref(), (decision == "approve").then_some(BYTES));
                    if decision == "approve" {
                        assert!(error.is_none());
                    } else {
                        let error = error.expect("denied read must report its refusal");
                        assert!(!error.contains(std::str::from_utf8(BYTES).unwrap()));
                    }
                }
                reply => panic!("wrong original reply: {reply:?}"),
            }
        }
        api.rejected(&route, &body, 409, true).await;
        receipts.push((request.clone(), decision));
    }
    api.post(&format!("/vms/{id}/stop"), json!({})).await;
    let db = capsem_logger::DbHandle::open_external_reader(&session.join("session.db")).unwrap();
    for (receipt, decision) in receipts {
        let params = [receipt["ask_id"].clone()];
        let rows: Value = serde_json::from_str(
            &db.query(
                "SELECT status,resolver,reason,event_id FROM security_ask_events WHERE ask_id=? ORDER BY id",
                &params,
            )
            .await
            .unwrap(),
        )
        .unwrap();
        let terminal = if decision == "approve" { "approved" } else { "denied" };
        assert_eq!(
            rows["rows"],
            json!([
                ["pending", null, null, receipt["event_id"]],
                [
                    terminal,
                    "authenticated_client",
                    if decision == "cancel" {
                        "request_cancelled"
                    } else {
                        "human_decision"
                    },
                    receipt["event_id"]
                ]
            ])
        );
        let count: Value = serde_json::from_str(
            &db.query(
                "SELECT COUNT(*) FROM fs_events WHERE event_id=?",
                &[receipt["event_id"].clone()],
            )
            .await
            .unwrap(),
        )
        .unwrap();
        assert_eq!(count["rows"], json!([[1]]));
    }
}
