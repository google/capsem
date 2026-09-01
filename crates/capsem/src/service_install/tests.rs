use super::*;
use std::path::Path;

#[test]
fn test_generate_plist_absolute_paths() {
    let plist = generate_plist(
        Path::new("/Users/test/.capsem/bin/capsem-service"),
        Path::new("/Users/test/.capsem/bin/capsem-process"),
        Path::new("/Users/test/.capsem/bin/capsem-gateway"),
        Path::new("/Users/test/.capsem/bin/capsem-tray"),
        Path::new("/Users/test/.capsem/assets"),
        "/Users/test",
    );
    // ProgramArguments binary and path args must be absolute
    assert!(plist.contains("<string>/Users/test/.capsem/bin/capsem-service</string>"));
    assert!(plist.contains("<string>/Users/test/.capsem/bin/capsem-process</string>"));
    assert!(plist.contains("<string>/Users/test/.capsem/assets</string>"));
    // Log path must be absolute
    assert!(plist.contains("<string>/Users/test/Library/Logs/capsem/service.log</string>"));
    // No tilde in paths
    assert!(!plist.contains("~"), "plist should not contain ~");
}

#[test]
fn test_generate_plist_valid_xml() {
    let plist = generate_plist(
        Path::new("/usr/local/bin/capsem-service"),
        Path::new("/usr/local/bin/capsem-process"),
        Path::new("/usr/local/bin/capsem-gateway"),
        Path::new("/usr/local/bin/capsem-tray"),
        Path::new("/home/test/.capsem/assets"),
        "/home/test",
    );
    assert!(plist.starts_with("<?xml"));
    assert!(plist.contains("<plist version=\"1.0\">"));
    assert!(plist.contains("</plist>"));
    // Balanced dict tags
    let open_dicts = plist.matches("<dict>").count();
    let close_dicts = plist.matches("</dict>").count();
    assert_eq!(open_dicts, close_dicts, "unbalanced <dict> tags");
}

#[test]
fn test_generate_plist_has_keep_alive() {
    let plist = generate_plist(
        Path::new("/bin/capsem-service"),
        Path::new("/bin/capsem-process"),
        Path::new("/bin/capsem-gateway"),
        Path::new("/bin/capsem-tray"),
        Path::new("/assets"),
        "/home",
    );
    assert!(plist.contains("<key>KeepAlive</key>"));
    assert!(plist.contains("<true/>"));
    assert!(plist.contains("<key>RunAtLoad</key>"));
}

#[test]
fn test_generate_plist_pins_file_backed_credential_store() {
    let plist = generate_plist(
        Path::new("/Users/test/.capsem/bin/capsem-service"),
        Path::new("/Users/test/.capsem/bin/capsem-process"),
        Path::new("/Users/test/.capsem/bin/capsem-gateway"),
        Path::new("/Users/test/.capsem/bin/capsem-tray"),
        Path::new("/Users/test/.capsem/assets"),
        "/Users/test",
    );

    assert!(plist.contains("<key>EnvironmentVariables</key>"));
    assert!(plist.contains("<key>CAPSEM_CREDENTIAL_STORE_PATH</key>"));
    assert!(plist.contains("<string>/Users/test/.capsem/credentials/credential-store.json</string>"));
    let retired_test_store = concat!("CAPSEM_CREDENTIAL", "_BROKER_TEST_STORE");
    let retired_keychain_namespace = concat!("org.capsem", ".credentials");
    let retired_keychain_service = concat!("com.capsem", ".credential");
    assert!(
        !plist.contains(retired_test_store),
        "installed service must not expose the retired credential test-store rail"
    );
    assert!(
        !plist.contains(retired_keychain_namespace) && !plist.contains(retired_keychain_service),
        "installed service must not expose a native Keychain namespace"
    );
    assert!(
        !plist.to_lowercase().contains("keychain"),
        "runtime LaunchAgent must not mention or select native Keychain storage"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_stop_uses_bootout_so_keepalive_does_not_restart_service() {
    let _lock = crate::lock_test_env();
    let _home = EnvGuard::set("HOME", "/Users/tester");
    let (primary, fallback) = macos_stop_launchagent_plan(501);

    assert_eq!(primary.program, "launchctl");
    assert_eq!(
        primary.args,
        vec!["bootout".to_string(), "gui/501/com.capsem.service".to_string()]
    );
    assert!(
        !primary.args.iter().any(|arg| arg == "kill" || arg == "SIGTERM"),
        "capsem stop must unload the LaunchAgent, not SIGTERM a KeepAlive job"
    );

    let fallback = fallback.expect("installed macOS stop path should have plist fallback");
    assert_eq!(fallback.program, "launchctl");
    assert_eq!(fallback.args[0], "unload");
    assert_eq!(
        fallback.args[1],
        "/Users/tester/Library/LaunchAgents/com.capsem.service.plist"
    );
}

#[test]
fn test_generate_systemd_unit_absolute_paths() {
    let unit = generate_systemd_unit(
        Path::new("/home/test/.capsem/bin/capsem-service"),
        Path::new("/home/test/.capsem/bin/capsem-process"),
        Path::new("/home/test/.capsem/bin/capsem-gateway"),
        Path::new("/home/test/.capsem/bin/capsem-tray"),
        Path::new("/home/test/.capsem/assets"),
    );
    // ExecStart line should have absolute path
    let exec_line = unit.lines().find(|l| l.starts_with("ExecStart=")).unwrap();
    assert!(
        exec_line.starts_with("ExecStart=/"),
        "ExecStart must use absolute path: {}",
        exec_line
    );
    // --process-binary value should be absolute
    assert!(exec_line.contains("--process-binary /"));
    // --assets-dir value should be absolute
    assert!(exec_line.contains("--assets-dir /"));
}

#[test]
fn test_generate_systemd_unit_restart_policy() {
    let unit = generate_systemd_unit(
        Path::new("/bin/svc"),
        Path::new("/bin/proc"),
        Path::new("/bin/gw"),
        Path::new("/bin/tray"),
        Path::new("/assets"),
    );
    assert!(unit.contains("Restart=always"));
    assert!(unit.contains("RestartSec=2"));
}

#[test]
fn test_generate_systemd_unit_wanted_by() {
    let unit = generate_systemd_unit(
        Path::new("/bin/svc"),
        Path::new("/bin/proc"),
        Path::new("/bin/gw"),
        Path::new("/bin/tray"),
        Path::new("/assets"),
    );
    assert!(unit.contains("[Install]"));
    assert!(unit.contains("WantedBy=default.target"));
}

// -- XML escaping ---------------------------------------------------------

#[test]
fn test_xml_escape_clean_path() {
    assert_eq!(xml_escape("/usr/local/bin"), "/usr/local/bin");
}

#[test]
fn test_xml_escape_ampersand() {
    assert_eq!(xml_escape("/Users/AT&T/bin"), "/Users/AT&amp;T/bin");
}

#[test]
fn test_xml_escape_angle_brackets() {
    assert_eq!(xml_escape("a<b>c"), "a&lt;b&gt;c");
}

#[test]
fn test_plist_with_special_chars_in_path() {
    let plist = generate_plist(
        Path::new("/Users/AT&T Corp/.capsem/bin/capsem-service"),
        Path::new("/Users/AT&T Corp/.capsem/bin/capsem-process"),
        Path::new("/Users/AT&T Corp/.capsem/bin/capsem-gateway"),
        Path::new("/Users/AT&T Corp/.capsem/bin/capsem-tray"),
        Path::new("/Users/AT&T Corp/.capsem/assets"),
        "/Users/AT&T Corp",
    );
    // Must contain escaped ampersands, not raw &
    assert!(plist.contains("AT&amp;T"), "plist must XML-escape ampersands");
    assert!(!plist.contains("AT&T "), "plist must not have unescaped &");
    // Must still be valid-ish XML (balanced tags)
    assert!(plist.contains("</plist>"));
}

// -- systemd space escaping -----------------------------------------------

#[test]
fn test_systemd_escape_path_no_spaces() {
    let p = Path::new("/home/user/.capsem/bin/capsem-service");
    assert_eq!(systemd_escape_path(p), "/home/user/.capsem/bin/capsem-service");
}

#[test]
fn test_systemd_escape_path_with_spaces() {
    let p = Path::new("/home/John Doe/.capsem/bin/capsem-service");
    let escaped = systemd_escape_path(p);
    assert_eq!(escaped, "/home/John\\x20Doe/.capsem/bin/capsem-service");
    assert!(!escaped.contains(' '), "spaces must be escaped for systemd");
}

#[test]
fn test_systemd_unit_with_spaces_in_path() {
    let unit = generate_systemd_unit(
        Path::new("/home/John Doe/.capsem/bin/capsem-service"),
        Path::new("/home/John Doe/.capsem/bin/capsem-process"),
        Path::new("/home/John Doe/.capsem/bin/capsem-gateway"),
        Path::new("/home/John Doe/.capsem/bin/capsem-tray"),
        Path::new("/home/John Doe/.capsem/assets"),
    );
    let exec_line = unit.lines().find(|l| l.starts_with("ExecStart=")).unwrap();
    // Spaces must be escaped as \x20 in ExecStart
    assert!(
        !exec_line.contains("John Doe"),
        "unescaped space in ExecStart: {}",
        exec_line
    );
    assert!(
        exec_line.contains("John\\x20Doe"),
        "missing \\x20 escape: {}",
        exec_line
    );
}

// -- test-isolation guard -------------------------------------------------

struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prev }
    }
    fn unset(key: &'static str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

#[test]
fn reject_test_isolation_env_accepts_clean_env() {
    let _lock = crate::lock_test_env();
    let _h = EnvGuard::unset("CAPSEM_HOME");
    let _r = EnvGuard::unset("CAPSEM_RUN_DIR");
    let _a = EnvGuard::unset("CAPSEM_ASSETS_DIR");
    assert!(reject_test_isolation_env().is_ok());
}

#[test]
fn explicit_stop_marker_roundtrips_under_run_dir() {
    let _lock = crate::lock_test_env();
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("run");
    let _r = EnvGuard::set("CAPSEM_RUN_DIR", run_dir.to_str().unwrap());

    assert!(!service_explicitly_stopped());
    write_explicit_stop_marker().unwrap();
    assert!(service_explicitly_stopped());
    assert_eq!(explicit_stop_marker_path(), run_dir.join(EXPLICIT_STOP_MARKER));

    clear_explicit_stop_marker().unwrap();
    assert!(!service_explicitly_stopped());
}

#[test]
fn reject_test_isolation_env_refuses_capsem_home() {
    let _lock = crate::lock_test_env();
    let _h = EnvGuard::set("CAPSEM_HOME", "/tmp/fake");
    let _r = EnvGuard::unset("CAPSEM_RUN_DIR");
    let _a = EnvGuard::unset("CAPSEM_ASSETS_DIR");
    let err = reject_test_isolation_env().unwrap_err().to_string();
    assert!(err.contains("CAPSEM_HOME"), "missing CAPSEM_HOME in error: {err}");
    assert!(err.contains("unset"), "error should tell user to unset: {err}");
}

#[test]
fn reject_test_isolation_env_ignores_empty() {
    // Empty value means "not set" per env_nonempty convention -- must not refuse.
    let _lock = crate::lock_test_env();
    let _h = EnvGuard::set("CAPSEM_HOME", "");
    let _r = EnvGuard::unset("CAPSEM_RUN_DIR");
    let _a = EnvGuard::unset("CAPSEM_ASSETS_DIR");
    assert!(reject_test_isolation_env().is_ok());
}

#[test]
fn reject_test_isolation_env_lists_all_set_vars() {
    let _lock = crate::lock_test_env();
    let _h = EnvGuard::set("CAPSEM_HOME", "/tmp/a");
    let _r = EnvGuard::set("CAPSEM_RUN_DIR", "/tmp/b");
    let _a = EnvGuard::set("CAPSEM_ASSETS_DIR", "/tmp/c");
    let err = reject_test_isolation_env().unwrap_err().to_string();
    assert!(err.contains("CAPSEM_HOME"));
    assert!(err.contains("CAPSEM_RUN_DIR"));
    assert!(err.contains("CAPSEM_ASSETS_DIR"));
}
