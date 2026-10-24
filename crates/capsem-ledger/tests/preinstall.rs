use std::process::Command;
use std::time::{Duration, Instant};

fn helper(args: &[&str]) -> std::process::Output {
    let spoofed = tempfile::tempdir().unwrap();
    let start = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_capsem-ledger"))
        .args(args)
        .env("HOME", spoofed.path())
        .env("CAPSEM_PRIVATE_SECRET", "private-env-canary-never-log")
        .output()
        .unwrap();
    assert!(start.elapsed() < Duration::from_secs(6), "helper exceeded total bound");
    assert_eq!(std::fs::read_dir(spoofed.path()).unwrap().count(), 0);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("private-env-canary-never-log"));
    output
}

#[test]
fn private_mode_rejects_root_before_any_worker_or_home_initialization() {
    let output = helper(&["--preinstall-ledger-admission", "--target-user", "root"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("target must be non-root"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn private_mode_has_no_arbitrary_path_or_worker_argument_fallback() {
    for args in [
        vec!["--preinstall-ledger-admission"],
        vec!["--target-user", "root"],
        vec![
            "--preinstall-ledger-admission",
            "--target-user",
            "root",
            "--database",
            "/synthetic/not-opened",
        ],
        vec!["--preinstall-ledger-admission=false", "--target-user", "root"],
    ] {
        assert!(!helper(&args).status.success());
    }
    let help = helper(&["--help"]);
    assert!(help.status.success());
    assert!(!String::from_utf8_lossy(&help.stdout).contains("preinstall-ledger-admission"));
}
