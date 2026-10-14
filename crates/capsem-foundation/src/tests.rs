use super::*;

#[test]
fn isolated_test_children_keep_only_the_coverage_output_path() {
    let command = isolated_test_command("true");
    let environment: Vec<_> = command.get_envs().collect();
    match std::env::var_os("LLVM_PROFILE_FILE") {
        Some(profile) => assert_eq!(
            environment,
            [(std::ffi::OsStr::new("LLVM_PROFILE_FILE"), Some(profile.as_os_str()))]
        ),
        None => assert!(environment.is_empty()),
    }
}
