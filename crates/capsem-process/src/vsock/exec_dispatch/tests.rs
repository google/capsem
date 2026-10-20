use super::*;
use capsem_logger::DbHandle;

struct Bridge {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    dispatch: ExecDispatch,
    guest: mpsc::Receiver<HostToGuest>,
}

fn bridge() -> Bridge {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let (hub, guest) = mpsc::channel(4);
    let dispatch = ExecDispatch {
        vm_id: "vm-under-test".into(),
        db: Arc::new(capsem_logger::DbWriter::open(&path, 16).unwrap()),
        rules: Arc::new(std::sync::RwLock::new(Arc::new(
            capsem_core::net::policy_config::SecurityRuleSet::new(Vec::new()),
        ))),
        plugins: Arc::new(std::sync::RwLock::new(Arc::new(std::collections::BTreeMap::new()))),
        jobs: Arc::new(JobStore::new()),
        hub,
    };
    Bridge {
        _dir: dir,
        path,
        dispatch,
        guest,
    }
}

/// `(command, target)` of every exec_events row, read the way a route reads
/// the ledger: through the DB handle after the writer's flush barrier.
async fn ledger_rows(bridge: &Bridge) -> Vec<(String, String)> {
    bridge.dispatch.db.flush().await;
    let handle = DbHandle::open_external_reader(&bridge.path).unwrap();
    handle.ready().await.unwrap();
    let rows: serde_json::Value = serde_json::from_str(
        &handle
            .query("SELECT command, target FROM exec_events ORDER BY id", &[])
            .await
            .unwrap(),
    )
    .unwrap();
    rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| (row[0].as_str().unwrap().to_owned(), row[1].as_str().unwrap().to_owned()))
        .collect()
}

fn dispatched(guest: &mut mpsc::Receiver<HostToGuest>) -> (u64, String) {
    match guest.try_recv().expect("an allowed exec reaches the guest") {
        HostToGuest::Exec { id, command } => (id, command),
        other => panic!("expected an exec, got {other:?}"),
    }
}

/// The ledger and the policy see the caller's command; only the guest sees the
/// launcher that carries it into the workload.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_workload_exec_is_recorded_as_asked_and_dispatched_through_the_launcher() {
    let mut bridge = bridge();
    let asked = "id -u; printf '%s' \"$HOME\"";
    bridge.dispatch.dispatch(7, asked.into(), ExecTarget::Workload).await;
    bridge
        .dispatch
        .dispatch(8, "test -f /var/tmp/x".into(), ExecTarget::Vm)
        .await;

    let (id, wrapped) = dispatched(&mut bridge.guest);
    assert_eq!(id, 7);
    assert_eq!(wrapped, capsem_core::container::workload_exec_command(asked));
    assert!(!wrapped.contains(asked), "the command travels encoded: {wrapped}");
    assert_eq!(dispatched(&mut bridge.guest), (8, "test -f /var/tmp/x".to_string()));

    assert_eq!(
        ledger_rows(&bridge).await,
        vec![
            (asked.to_string(), "workload".to_string()),
            ("test -f /var/tmp/x".to_string(), "vm".to_string()),
        ]
    );
}

#[test]
fn a_vm_exec_reaches_the_guest_unchanged() {
    assert_eq!(guest_command("echo hi".into(), ExecTarget::Vm), "echo hi");
    assert_ne!(guest_command("echo hi".into(), ExecTarget::Workload), "echo hi");
}
