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
