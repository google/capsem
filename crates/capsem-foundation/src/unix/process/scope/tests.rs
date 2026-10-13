use super::*;

#[test]
fn native_process_scope_is_stable_serializable_and_has_a_boot_identity() {
    let first = ProcessScope::current().unwrap();
    assert_eq!(first, ProcessScope::current().unwrap());
    let encoded = serde_json::to_vec(&first).unwrap();
    assert_eq!(serde_json::from_slice::<ProcessScope>(&encoded).unwrap(), first);
    assert!(valid_boot_id(&first.boot));
    #[cfg(target_os = "linux")]
    assert!(first.namespace.unwrap()[1] > 0);
}

#[test]
fn native_process_scope_rejects_invalid_boot_identity() {
    for text in [
        "",
        "bad",
        "00000000-0000-0000-0000-000000000000",
        "a".repeat(100).as_str(),
    ] {
        assert!(!valid_boot_id(text));
    }
    assert!(valid_boot_id("a02470b2-a7be-4f12-9a6b-3f47ec5bd570"));
    assert!(valid_boot_id("A02470B2-A7BE-4F12-9A6B-3F47EC5BD570"));
}

#[test]
fn native_process_scope_child_fixture() {
    let Ok(expected) = std::env::var("CAPSEM_TEST_PROCESS_SCOPE") else {
        return;
    };
    let expected: ProcessScope = serde_json::from_str(&expected).unwrap();
    assert_eq!(ProcessScope::current().unwrap(), expected);
}

#[test]
fn native_process_scope_matches_a_real_child() {
    let scope = ProcessScope::current().unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "unix::process::scope::tests::native_process_scope_child_fixture",
        ])
        .env_clear()
        .env("CAPSEM_TEST_PROCESS_SCOPE", serde_json::to_string(&scope).unwrap())
        .stdin(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
}
