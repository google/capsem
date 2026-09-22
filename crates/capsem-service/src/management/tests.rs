use super::*;

#[test]
fn launchd_identity_requires_the_current_pid_not_a_job_name_or_ancestor() {
    let jobs = "PID\tStatus\tLabel\n-\t0\tcom.capsem.service\n123\t0\tcom.capsem.test\n321\t0\tother\n";
    assert_eq!(launchd_label(jobs, 123), Some("com.capsem.test"));
    assert_eq!(launchd_label(jobs, 999), None);
    assert_eq!(launchd_label("123 0 label extra", 123), None);
    assert_eq!(launchd_label("123", 123), None);
}

#[test]
fn launchd_restart_requires_top_level_pid_and_keepalive() {
    assert!(launchd_restarts("{\n\t\"PID\" = 123;\n\t\"OnDemand\" = false;\n}", 123));
    for job in [
        "\t\"PID\" = 321;\n\t\"OnDemand\" = false;",
        "\t\"PID\" = 123;\n\t\"OnDemand\" = true;",
        "\t\"PID\" = 123;",
        "\t\t\"PID\" = 123;\n\t\t\"OnDemand\" = false;",
        "\"PID\" = 123;\n\"OnDemand\" = false;",
        "",
    ] {
        assert!(!launchd_restarts(job, 123), "accepted invalid supervisor proof: {job}");
    }
}

#[test]
fn systemd_restart_requires_the_current_main_process_and_clean_exit_policy() {
    for policy in ["always", "on-success"] {
        assert!(systemd_restarts(&format!("MainPID=123\nRestart={policy}\n"), 123));
    }
    for job in [
        "MainPID=321\nRestart=always",
        "MainPID=123\nRestart=on-failure",
        "MainPID=123\nRestart=no",
        "MainPID=123",
        "Restart=always",
        "",
    ] {
        assert!(!systemd_restarts(job, 123), "accepted invalid supervisor proof: {job}");
    }
}

#[tokio::test]
async fn missing_or_failed_supervisor_commands_are_not_management_proof() {
    assert_eq!(query("/does/not/exist/capsem-supervisor", &[]).await, None);
    assert_eq!(query("false", &[]).await, None);
}
