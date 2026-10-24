use super::*;

#[test]
fn private_parser_is_explicit_and_cannot_accept_worker_paths() {
    assert!(Args::try_parse_from([
        "ledger",
        "--preinstall-ledger-admission",
        "--target-user",
        "synthetic-user"
    ])
    .is_ok());
    assert!(
        Args::try_parse_from([
            "ledger",
            "--preinstall-ledger-admission",
            "--target-user",
            "synthetic-user",
            "--require-current-host"
        ])
        .unwrap()
        .require_current_host
    );
    for args in [
        vec!["ledger", "--target-user", "synthetic-user"],
        vec!["ledger", "--preinstall-ledger-admission"],
        vec![
            "ledger",
            "--preinstall-ledger-admission",
            "--target-user",
            "user",
            "--database",
            "/tmp/fake",
        ],
    ] {
        assert!(Args::try_parse_from(args).is_err());
    }
    assert!(is_requested([OsString::from("--preinstall-ledger-admission=false")]));
    assert!(!is_requested([OsString::from("--target-user=ordinary")]));
}

#[test]
fn account_identity_requires_nonroot_current_uid_and_absolute_home() {
    let target = |uid, home: &str| account::Account { uid, home: home.into() };
    assert!(validate_identity(&target(501, "/actual-account"), 501, 501).is_ok());
    assert!(validate_identity(&target(0, "/root"), 0, 0).is_err());
    assert!(validate_identity(&target(501, "/actual-account"), 502, 502).is_err());
    assert!(validate_identity(&target(501, "relative"), 501, 501).is_err());
}

#[test]
fn complete_bound_propagates_original_result() {
    assert!(bounded(Duration::from_secs(1), |_| Ok(())).is_ok());
    let failure = bounded(Duration::from_secs(1), |_| bail!("original failure")).unwrap_err();
    assert_eq!(failure.to_string(), "original failure");
}

#[test]
fn blocked_lookup_child() {
    if std::env::var_os("CAPSEM_BLOCKED_ADMISSION_CHILD").is_none() {
        return;
    }
    let start = Instant::now();
    let error = bounded(Duration::from_millis(30), |_| {
        use std::io::Write;
        println!("synthetic lookup entered");
        std::io::stdout().flush().unwrap();
        loop {
            std::thread::park();
        }
    })
    .unwrap_err();
    assert!(error.to_string().contains("deadline exceeded"));
    assert!(start.elapsed() < Duration::from_secs(1));
    // Returning the fresh helper's main terminates its detached blocked thread.
}

#[test]
fn fresh_helper_exit_terminates_a_blocked_lookup_thread() {
    let start = Instant::now();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "preinstall::tests::blocked_lookup_child", "--nocapture"])
        .env("CAPSEM_BLOCKED_ADMISSION_CHILD", "1")
        .output()
        .unwrap();
    assert!(child.status.success(), "{}", String::from_utf8_lossy(&child.stderr));
    assert!(String::from_utf8_lossy(&child.stdout).contains("synthetic lookup entered"));
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn identity_rejects_elevated_or_foreign_effective_uid() {
    let target = account::Account {
        uid: 501,
        home: "/actual-account".into(),
    };
    assert!(validate_identity(&target, 501, 0).is_err());
    assert!(validate_identity(&target, 501, 502).is_err());
}
