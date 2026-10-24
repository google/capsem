pub(crate) fn sandbox_test_executable(directory: &std::path::Path) -> std::path::PathBuf {
    let executable = std::env::current_exe().unwrap();
    if cfg!(target_os = "macos") {
        // This test child installs the real MCP worker policy. External MCP
        // tools keep the inherited gate policy and never use this handoff.
        let child = directory.join("capsem-mcp-confinement-test");
        std::fs::copy(executable, &child).unwrap();
        child
    } else {
        executable
    }
}
