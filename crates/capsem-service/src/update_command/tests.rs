use super::*;

#[test]
fn apply_escapes_systemd_service_cgroup() {
    let cli = "/installed/capsem".to_string();
    let plan = update_command_plan_for(UpdateCommandKind::Apply, cli.clone(), true);
    let check_plan = update_command_plan_for(UpdateCommandKind::Check, cli.clone(), true);

    assert_eq!(plan.program, "systemd-run");
    assert_eq!(
        plan.args,
        vec![
            "--user",
            "--wait",
            "--collect",
            "--quiet",
            "--unit=capsem-update",
            "--",
            cli.as_str(),
            "update",
            "--yes",
        ]
    );
    assert_eq!(check_plan.program, cli);
    assert_eq!(check_plan.args, vec!["update", "--check"]);
}

#[test]
fn inherited_systemd_identity_does_not_reparent_the_update() {
    let invocation_id = OsStr::new("runner-invocation");
    let parent_pid = OsStr::new("41");
    let service_pid = 42;

    assert!(!direct_systemd_invocation(
        Some(invocation_id),
        Some(parent_pid),
        service_pid,
    ));
    assert!(direct_systemd_invocation(
        Some(invocation_id),
        Some(OsStr::new("42")),
        service_pid,
    ));
    assert!(!direct_systemd_invocation(Some(invocation_id), None, service_pid,));
    assert!(!direct_systemd_invocation(None, Some(OsStr::new("42")), service_pid,));
}
