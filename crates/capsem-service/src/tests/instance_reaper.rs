use super::*;

#[test]
fn provision_persistent_validates_name() {
    let state = make_test_state();
    let result = state.provision_sandbox(ProvisionOptions {
        id: "../evil",
        name: "../evil",
        profile_id: "code".into(),
        ram_mb: 2048,
        cpus: 2,
        scratch_disk_size_gb: 16,
        version_override: None,
        persistent: true,
        env: None,
        from: None,
        description: None,
    });
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("must start with") || err.contains("must contain only"),
        "expected name validation error, got: {err}"
    );
}

#[test]
fn child_reapers_start_after_instance_registration() {
    let source = include_str!("../main.rs");
    for (function, next_function, reaper) in [
        (
            "    fn provision_sandbox(",
            "    fn resume_sandbox(",
            "instance_reaper::spawn_provision(",
        ),
        (
            "    fn resume_sandbox(",
            "    fn has_existing_resume_checkpoint(",
            "instance_reaper::spawn_resume(",
        ),
    ] {
        let start = source.find(function).expect("launch function exists");
        let end = source[start..]
            .find(next_function)
            .map(|offset| start + offset)
            .expect("following function exists");
        let body = &source[start..end];
        let insertion = body
            .find("instances.insert(")
            .expect("launch function registers its instance");
        let reaper = body.find(reaper).expect("launch function starts its child reaper");

        assert!(
            insertion < reaper,
            "{function} must publish the instance before its child reaper can run"
        );
    }
}
