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
    bridge
        .dispatch
        .dispatch(7, asked.into(), ExecTarget::Workload, None, None)
        .await;
    bridge
        .dispatch
        .dispatch(8, "test -f /var/tmp/x".into(), ExecTarget::Vm, None, None)
        .await;
    bridge
        .dispatch
        .dispatch(
            9,
            "pwd".into(),
            ExecTarget::Workload,
            Some("1000:1000".into()),
            Some("/workspace".into()),
        )
        .await;

    let (id, wrapped) = dispatched(&mut bridge.guest);
    assert_eq!(id, 7);
    assert_eq!(wrapped, capsem_core::container::workload_exec_command(asked));
    assert!(!wrapped.contains(asked), "the command travels encoded: {wrapped}");
    assert_eq!(dispatched(&mut bridge.guest), (8, "test -f /var/tmp/x".to_string()));
    assert_eq!(
        dispatched(&mut bridge.guest),
        (
            9,
            capsem_core::container::workload_exec_command_with("pwd", Some("1000:1000"), Some("/workspace"))
        )
    );

    assert_eq!(
        ledger_rows(&bridge).await,
        vec![
            (asked.to_string(), "workload".to_string()),
            ("test -f /var/tmp/x".to_string(), "vm".to_string()),
            ("pwd".to_string(), "workload".to_string()),
        ]
    );
}

#[test]
fn a_vm_exec_reaches_the_guest_unchanged_until_user_or_cwd_is_requested() {
    assert_eq!(guest_command("echo hi".into(), ExecTarget::Vm, None, None), "echo hi");
    assert_eq!(
        guest_command("echo hi".into(), ExecTarget::Vm, Some("root"), None),
        "echo hi"
    );
    assert_eq!(
        guest_command("echo hi".into(), ExecTarget::Vm, Some("0:0"), None),
        "echo hi"
    );
    assert_eq!(
        guest_command("echo 'hi'".into(), ExecTarget::Vm, None, Some("/var/tmp")),
        "cd -- '/var/tmp' && exec bash -c 'echo '\\''hi'\\'''"
    );
    let as_nobody = guest_command("id -u".into(), ExecTarget::Vm, Some("nobody"), Some("/tmp"));
    assert!(as_nobody.starts_with("/bin/sh -c "), "{as_nobody}");
    assert!(
        as_nobody.ends_with(" sh 'nobody' 'cd -- '\\''/tmp'\\'' && exec bash -c '\\''id -u'\\'''"),
        "{as_nobody}"
    );
    assert_ne!(
        guest_command("echo hi".into(), ExecTarget::Workload, None, None),
        "echo hi"
    );
}

#[test]
fn vm_user_and_cwd_shell_wrapper_executes_and_reports_unknown_identities() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let bin_dir = dir.path().join("bin");
    let work_dir = dir.path().join("work space");
    std::fs::create_dir_all(&bin_dir).unwrap();
    std::fs::create_dir_all(&work_dir).unwrap();

    let su_path = bin_dir.join("su");
    std::fs::write(&su_path, "#!/bin/sh\nprintf 'SU:%s\\n' \"$*\"\nexec /bin/sh -c \"$6\"\n").unwrap();
    std::fs::set_permissions(&su_path, std::fs::Permissions::from_mode(0o755)).unwrap();

    let setpriv_path = bin_dir.join("setpriv");
    std::fs::write(
        &setpriv_path,
        "#!/bin/sh\nprintf 'SETPRIV:%s:%s:%s\\n' \"$1\" \"$2\" \"$3\"\nshift 3\nexec \"$@\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&setpriv_path, std::fs::Permissions::from_mode(0o755)).unwrap();

    let path_env = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
    );
    let run = |cmd: String| {
        std::process::Command::new("/bin/sh")
            .args(["-c", &cmd])
            .env("PATH", &path_env)
            .output()
            .unwrap()
    };

    let cwd_only = run(guest_command(
        "pwd".into(),
        ExecTarget::Vm,
        Some("root"),
        Some(work_dir.to_str().unwrap()),
    ));
    assert!(cwd_only.status.success(), "{cwd_only:?}");
    assert_eq!(
        String::from_utf8_lossy(&cwd_only.stdout).trim(),
        work_dir.canonicalize().unwrap().to_str().unwrap()
    );

    let named_user = run(guest_command(
        "pwd".into(),
        ExecTarget::Vm,
        Some("root"),
        None,
    ));
    assert!(named_user.status.success());

    let su_user = run(guest_command(
        "pwd".into(),
        ExecTarget::Vm,
        Some("daemon"),
        Some(work_dir.to_str().unwrap()),
    ));
    let su_stdout = String::from_utf8_lossy(&su_user.stdout);
    assert!(su_user.status.success(), "{su_user:?}");
    assert!(su_stdout.starts_with("SU:-m daemon -s /bin/bash -c "), "{su_stdout}");
    assert!(
        su_stdout
            .trim_end()
            .ends_with(work_dir.canonicalize().unwrap().to_str().unwrap()),
        "{su_stdout}"
    );

    let numeric_pair = run(guest_command(
        "pwd".into(),
        ExecTarget::Vm,
        Some("1000:1000"),
        Some(work_dir.to_str().unwrap()),
    ));
    assert!(numeric_pair.status.success(), "{numeric_pair:?}");
    let pair_stdout = String::from_utf8_lossy(&numeric_pair.stdout);
    assert!(
        pair_stdout.starts_with("SETPRIV:--reuid=1000:--regid=1000:--clear-groups\n"),
        "{pair_stdout}"
    );
    assert!(
        pair_stdout
            .trim_end()
            .ends_with(work_dir.canonicalize().unwrap().to_str().unwrap()),
        "{pair_stdout}"
    );

    let bad_user = run(guest_command(
        "echo unreachable".into(),
        ExecTarget::Vm,
        Some("no_such_capsem_user_99999"),
        None,
    ));
    assert_eq!(bad_user.status.code(), Some(125));
    assert!(
        String::from_utf8_lossy(&bad_user.stderr).contains("capsem: unknown user: no_such_capsem_user_99999"),
        "{bad_user:?}"
    );

    let bad_group = run(guest_command(
        "echo unreachable".into(),
        ExecTarget::Vm,
        Some("0:no_such_capsem_group_99999"),
        None,
    ));
    assert_eq!(bad_group.status.code(), Some(125));
    assert!(
        String::from_utf8_lossy(&bad_group.stderr).contains("capsem: unknown group: no_such_capsem_group_99999"),
        "{bad_group:?}"
    );
}

