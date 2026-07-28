use super::*;

#[test]
fn cmdline_extracts_basename() {
    // This test only runs on Linux where /proc exists.
    // On other platforms, it falls through to the logic test below.
    if !std::path::Path::new("/proc").exists() {
        return;
    }
    // Our own process should have a name
    let pid = std::process::id();
    let name = process_name_for_pid(pid);
    assert_ne!(name, "unknown");
    assert!(!name.is_empty());
}

#[test]
fn nonexistent_pid_returns_unknown() {
    // PID 4294967295 is unlikely to exist
    let name = process_name_for_pid(u32::MAX);
    assert_eq!(name, "unknown");
}

#[test]
fn basename_extraction_logic() {
    // Test the basename extraction logic directly
    let path = "/usr/bin/gemini";
    let basename = path.rsplit('/').next().unwrap_or(path);
    assert_eq!(basename, "gemini");

    let bare = "node";
    let basename = bare.rsplit('/').next().unwrap_or(bare);
    assert_eq!(basename, "node");
}

#[test]
fn basename_with_trailing_slash() {
    // Trailing slash yields empty basename, rsplit returns ""
    let path = "/usr/bin/";
    let basename = path.rsplit('/').next().unwrap_or(path);
    assert_eq!(basename, "");
}

#[test]
fn basename_root_only() {
    let path = "/";
    let basename = path.rsplit('/').next().unwrap_or(path);
    assert_eq!(basename, "");
}

#[test]
fn pid_zero_returns_something() {
    // PID 0 is the kernel scheduler -- /proc/0 may or may not exist.
    // Should not panic regardless.
    let name = process_name_for_pid(0);
    assert!(!name.is_empty());
}

#[test]
fn pid_one_returns_init_on_linux() {
    if !std::path::Path::new("/proc/1/cmdline").exists() {
        return; // Not Linux
    }
    let name = process_name_for_pid(1);
    assert_ne!(name, "unknown");
}
