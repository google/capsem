use super::*;
use std::io::Write;
use tempfile::TempDir;

fn write_log(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    p
}

#[test]
fn parse_since_accepts_duration_suffixes() {
    assert!(parse_since("30m").is_some());
    assert!(parse_since("2h").is_some());
    assert!(parse_since("7d").is_some());
    assert!(parse_since("60s").is_some());
}

#[test]
fn parse_since_rejects_garbage() {
    assert!(parse_since("").is_none());
    assert!(parse_since("nope").is_none());
}

#[test]
fn parse_since_accepts_rfc3339() {
    assert!(parse_since("2026-05-02T17:30:00Z").is_some());
}

#[test]
fn scan_panics_finds_text_panic_with_thread_name() {
    let dir = TempDir::new().unwrap();
    let p = write_log(
        dir.path(),
        "process.log",
        "thread 'tokio-runtime-worker' panicked at /Users/elie/git/capsem/crates/capsem-process/src/main.rs:200:5:\n\
         vz host lock task panicked\n\
            at /Users/elie/git/capsem/crates/capsem-process/src/vsock.rs:300:1\n\
            at /Users/elie/git/capsem/crates/capsem-process/src/main.rs:42:1\n\
         INFO some other line\n",
    );
    let panics = scan_panics_in_file(&p, "capsem-process", 0);
    assert_eq!(panics.len(), 1, "{panics:#?}");
    let pe = &panics[0];
    assert_eq!(pe.thread.as_deref(), Some("tokio-runtime-worker"));
    assert!(pe.message.contains("vz host lock task panicked"), "{pe:?}");
    assert!(pe.location.as_deref().unwrap().contains("~/"));
    assert_eq!(pe.frames.len(), 2);
}

#[test]
fn scan_panics_finds_json_panic_line() {
    let dir = TempDir::new().unwrap();
    let p = write_log(
        dir.path(),
        "service.log",
        r#"{"timestamp":"2026-05-02T18:00:00Z","level":"ERROR","fields":{"message":"thread 'main' panicked at src/main.rs:100","location":"src/main.rs:100"},"target":"capsem_service"}"#,
    );
    let panics = scan_panics_in_file(&p, "capsem-service", 0);
    assert_eq!(panics.len(), 1, "{panics:#?}");
    assert_eq!(panics[0].ts, "2026-05-02T18:00:00Z");
}

#[test]
fn scan_errors_returns_warn_and_error_only() {
    let dir = TempDir::new().unwrap();
    let p = write_log(
        dir.path(),
        "service.log",
        r#"{"timestamp":"2026-05-02T18:00:00Z","level":"INFO","fields":{"message":"boring info line"},"target":"capsem_service"}
{"timestamp":"2026-05-02T18:00:01Z","level":"WARN","fields":{"message":"something dropped"},"target":"ipc"}
{"timestamp":"2026-05-02T18:00:02Z","level":"ERROR","fields":{"message":"connection refused"},"target":"capsem_service"}
"#,
    );
    let errors = scan_errors_in_file(&p, "capsem-service", 0, 100);
    assert_eq!(errors.len(), 2, "{errors:#?}");
    assert_eq!(errors[0].level, "WARN");
    assert_eq!(errors[1].level, "ERROR");
}

#[test]
fn scan_errors_filters_by_since() {
    let dir = TempDir::new().unwrap();
    let p = write_log(
        dir.path(),
        "service.log",
        r#"{"timestamp":"2026-05-02T17:00:00Z","level":"ERROR","fields":{"message":"old error"},"target":"capsem_service"}
{"timestamp":"2026-05-02T19:00:00Z","level":"ERROR","fields":{"message":"new error"},"target":"capsem_service"}
"#,
    );
    let cutoff = parse_rfc3339_seconds("2026-05-02T18:00:00Z").unwrap();
    let errors = scan_errors_in_file(&p, "capsem-service", cutoff, 100);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].message.contains("new error"));
}

#[test]
fn scan_slow_ops_filters_by_threshold() {
    let dir = TempDir::new().unwrap();
    let p = write_log(
        dir.path(),
        "process.log",
        r#"{"timestamp":"2026-05-02T18:00:00Z","level":"INFO","fields":{"message":"fsync ok","op":"fsync","duration_ms":120},"target":"fs"}
{"timestamp":"2026-05-02T18:00:01Z","level":"INFO","fields":{"message":"fsync slow","op":"fsync","duration_ms":3120},"target":"fs"}
"#,
    );
    let slow = scan_slow_ops_in_file(&p, "capsem-process", 0, 500);
    assert_eq!(slow.len(), 1);
    assert_eq!(slow[0].duration_ms, 3120);
    assert_eq!(slow[0].op, "fsync");
}

// F9: adversarial panic shapes from the followups audit.

#[test]
fn scan_panics_handles_inlined_frame_lines() {
    let dir = TempDir::new().unwrap();
    // Inlined frames don't start with `at ` -- they're collapsed onto the
    // panic header in some Rust versions. The parser must tolerate that
    // by capturing the message but not breaking on the missing frames.
    let p = write_log(
        dir.path(),
        "process.log",
        "thread 'main' panicked at src/main.rs:1:1:\n\
         my message\n\
         (no backtrace)\n",
    );
    let panics = scan_panics_in_file(&p, "capsem-process", 0);
    assert_eq!(panics.len(), 1);
    assert!(panics[0].message.contains("my message"));
    // Frames may be empty; we don't assert non-empty here.
}

#[test]
fn scan_panics_multiple_in_same_file() {
    let dir = TempDir::new().unwrap();
    let p = write_log(
        dir.path(),
        "process.log",
        "thread 'A' panicked at a.rs:1:1:\n\
         msg-A\n\
            at frame1.rs:1:1\n\
         INFO unrelated line\n\
         thread 'B' panicked at b.rs:2:2:\n\
         msg-B\n\
            at frame2.rs:1:1\n",
    );
    let panics = scan_panics_in_file(&p, "capsem-process", 0);
    assert_eq!(panics.len(), 2, "{panics:#?}");
    assert!(panics[0].message.contains("msg-A"));
    assert!(panics[1].message.contains("msg-B"));
}

#[test]
fn scan_panics_handles_panic_with_empty_message() {
    let dir = TempDir::new().unwrap();
    let p = write_log(
        dir.path(),
        "process.log",
        "thread 'X' panicked at lib.rs:1:1:\n\
         \n\
            at lib.rs:1:1\n",
    );
    let panics = scan_panics_in_file(&p, "capsem-process", 0);
    // Empty-message panic still emits a record with frame data captured.
    assert_eq!(panics.len(), 1);
}

#[test]
fn scan_panics_no_match_on_normal_log() {
    let dir = TempDir::new().unwrap();
    let p = write_log(
        dir.path(),
        "process.log",
        "INFO regular message\n\
         WARN nothing panicked here\n\
         ERROR connection refused\n",
    );
    let panics = scan_panics_in_file(&p, "capsem-process", 0);
    assert_eq!(panics.len(), 0);
}

#[test]
fn host_log_path_only_allows_known_names() {
    let run_dir = std::path::PathBuf::from("/tmp/run");
    assert!(host_log_path(&run_dir, "service").is_some());
    assert!(host_log_path(&run_dir, "mcp").is_some());
    assert!(host_log_path(&run_dir, "gateway").is_some());
    assert!(host_log_path(&run_dir, "tray").is_some());
    assert!(host_log_path(&run_dir, "..").is_none());
    assert!(host_log_path(&run_dir, "session.db").is_none());
    assert!(host_log_path(&run_dir, "../../etc/passwd").is_none());
}
