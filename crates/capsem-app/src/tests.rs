use super::*;
use std::fs;
use std::thread;
use std::time::Duration;

fn args(input: &[&str]) -> Vec<String> {
    input.iter().map(|s| s.to_string()).collect()
}

#[test]
fn parse_flag_returns_value_for_known_flag() {
    let a = args(&["--connect", "vm-123", "--action", "open"]);
    assert_eq!(parse_flag(&a, "--connect"), Some("vm-123".into()));
    assert_eq!(parse_flag(&a, "--action"), Some("open".into()));
}

#[test]
fn parse_flag_returns_none_when_flag_missing() {
    let a = args(&["--other", "x"]);
    assert_eq!(parse_flag(&a, "--connect"), None);
}

#[test]
fn parse_flag_ignores_trailing_flag_without_value() {
    // "--connect" with no value at end should not panic and should return None.
    let a = args(&["--connect"]);
    assert_eq!(parse_flag(&a, "--connect"), None);
}

#[test]
fn parse_connect_and_action_share_logic() {
    let a = args(&["--action", "stop", "--connect", "abc"]);
    assert_eq!(parse_connect_arg(&a), Some("abc".into()));
    assert_eq!(parse_action_arg(&a), Some("stop".into()));
}

#[test]
fn cleanup_old_logs_removes_expired_files() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("old.jsonl");
    let new = dir.path().join("new.jsonl");
    fs::write(&old, b"x").unwrap();
    fs::write(&new, b"y").unwrap();

    // Backdate old file to 30 days ago.
    let thirty_days_ago = SystemTime::now() - Duration::from_secs(30 * 86400);
    filetime::set_file_mtime(&old, filetime::FileTime::from_system_time(thirty_days_ago))
        .unwrap();

    cleanup_old_logs(dir.path(), 7);

    assert!(!old.exists(), "expired file should be deleted");
    assert!(new.exists(), "recent file should survive");
}

#[test]
fn cleanup_old_logs_is_a_noop_on_missing_dir() {
    // Must not panic.
    cleanup_old_logs(std::path::Path::new("/nonexistent/capsem-app-test"), 7);
}

#[test]
fn cleanup_old_logs_ignores_subdirectories() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).unwrap();
    // Subdirs should not be removed even when past the cutoff.
    let thirty_days_ago = SystemTime::now() - Duration::from_secs(30 * 86400);
    filetime::set_file_mtime(&sub, filetime::FileTime::from_system_time(thirty_days_ago))
        .unwrap();

    cleanup_old_logs(dir.path(), 7);
    assert!(sub.exists());
}

#[test]
fn format_log_filename_has_expected_shape() {
    // 2026-01-01T00:00:00Z → 1767225600
    let name = format_log_filename(1_767_225_600);
    assert_eq!(name, "2026-01-01T00-00-00.jsonl");
}

#[test]
fn open_log_file_creates_file_and_returns_writable_handle() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("capsem-app-test.jsonl");

    let mut file = open_log_file(&path).expect("open_log_file should succeed");
    file.write_all(b"line\n").unwrap();

    assert!(path.exists());
    let contents = fs::read_to_string(&path).unwrap();
    assert_eq!(contents, "line\n");
}

#[cfg(unix)]
#[test]
fn open_log_file_restricts_permissions_to_0600() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("perms-test.jsonl");

    let _ = open_log_file(&path).expect("open_log_file should succeed");

    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o600,
        "log files may contain VM ids, paths, and provider API metadata -- must not be world-readable (got {mode:o})"
    );
}

#[test]
fn format_log_filename_unix_epoch() {
    assert_eq!(format_log_filename(0), "1970-01-01T00-00-00.jsonl");
}

#[test]
fn format_log_filename_roundtrips_seconds_of_day() {
    // 86399 = 23:59:59 on 1970-01-01
    assert_eq!(format_log_filename(86_399), "1970-01-01T23-59-59.jsonl");
}

#[test]
fn log_filename_produces_reasonable_modern_shape() {
    let name = log_filename();
    // Format: YYYY-MM-DDTHH-MM-SS.jsonl
    assert!(name.ends_with(".jsonl"));
    assert_eq!(name.len(), "YYYY-MM-DDTHH-MM-SS.jsonl".len());
    // Year should be at least 2025 (any CI machine).
    let year: i32 = name[..4].parse().unwrap();
    assert!(
        year >= 2025,
        "expected modern year in log filename, got {name}"
    );
}

#[test]
fn log_filenames_are_stable_across_quick_calls() {
    let a = log_filename();
    thread::sleep(Duration::from_millis(5));
    let b = log_filename();
    // Shapes match.
    assert_eq!(a.len(), b.len());
}

// -----------------------------------------------------------------------
// AB-003: deep-link payload is JSON-serialized, not string-interpolated
// -----------------------------------------------------------------------

#[test]
fn build_deep_link_script_with_plain_values() {
    let s = build_deep_link_script("vm-123", Some("open"));
    assert!(s.contains("window.__capsemDeepLink"));
    // The payload is JSON: keys are double-quoted, the call is one expr.
    assert!(s.contains("\"connect\":\"vm-123\""));
    assert!(s.contains("\"action\":\"open\""));
}

#[test]
fn build_deep_link_script_omits_action_when_none() {
    let s = build_deep_link_script("vm-1", None);
    assert!(s.contains("\"connect\":\"vm-1\""));
    assert!(!s.contains("\"action\""), "no action key when None: {s}");
}

#[test]
fn build_deep_link_script_escapes_single_quote_in_id() {
    let s = build_deep_link_script("ab'cd", None);
    // Inside JSON the bare apostrophe needs no escape, but the surrounding
    // quoting must be double quotes -- the legacy code embedded into a
    // single-quoted JS literal which is what the bug exploited.
    assert!(s.contains("\"connect\":\"ab'cd\""), "got: {s}");
}

#[test]
fn build_deep_link_script_escapes_backslash_in_id() {
    // AB-003 critical: the previous fix only escaped single quotes, so a
    // trailing backslash escaped the closing JS quote and let attacker
    // payloads run as code.
    let s = build_deep_link_script("ab\\cd", None);
    // JSON serialization must produce \\ (two characters).
    assert!(
        s.contains("\"connect\":\"ab\\\\cd\""),
        "backslash must be JSON-escaped: {s}"
    );
}

#[test]
fn build_deep_link_script_escapes_newline_in_id() {
    let s = build_deep_link_script("ab\ncd", None);
    // The literal newline must NOT appear; JSON encodes it as \n (two chars).
    assert!(!s.contains("ab\ncd"), "raw newline must not survive: {s:?}");
    assert!(s.contains("ab\\ncd"), "newline must be escaped \\n: {s:?}");
}

#[test]
fn build_deep_link_payload_blocks_injection_input() {
    // AB-003 attack repro: a `--connect` value crafted to break out of the
    // legacy single-quoted JS literal and run arbitrary code. The input
    // must round-trip through the JSON payload as a string -- not become
    // executable code.
    let vm_id = "x\\'); alert(1); //";
    let payload = build_deep_link_payload(vm_id, None);
    assert_eq!(
        payload["connect"], vm_id,
        "input must survive verbatim as data"
    );
    // Double-check the serialized form: the `\\` and `'` must be
    // contained inside a JSON string (double-quoted), not as bare JS.
    let serialized = payload.to_string();
    assert!(
        serialized.starts_with("{\"connect\":\""),
        "expected JSON object, got: {serialized}"
    );
    assert!(
        serialized.contains("\"x\\\\'); alert(1); //\""),
        "JSON encoding must escape backslash; got: {serialized}"
    );
}

#[test]
fn build_deep_link_payload_round_trips_through_json() {
    // The payload, when serialized, must be parseable as JSON with the
    // same string content. This is the structural guarantee that the
    // value is data, not code, regardless of what bytes the input
    // contained.
    let vm_id = "vm\"\\'\n\t\u{1}";
    let action = "op'>?</";
    let serialized = build_deep_link_payload(vm_id, Some(action)).to_string();
    let parsed: serde_json::Value =
        serde_json::from_str(&serialized).expect("payload must be valid JSON");
    assert_eq!(parsed["connect"], vm_id);
    assert_eq!(parsed["action"], action);
}
