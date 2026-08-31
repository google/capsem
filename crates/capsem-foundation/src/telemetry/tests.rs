//! Tests for telemetry::ambient_capsem_trace_id parsing of TRACEPARENT.

use super::*;

#[test]
fn ambient_trace_id_from_capsem_env_takes_precedence() {
    let id = resolve_ambient_capsem_trace_id(
        Some("deadbeefcafef00d"),
        Some("00-11111111111111112222222222222222-3333333333333333-01"),
    );
    assert_eq!(id.as_deref(), Some("deadbeefcafef00d"));
}

#[test]
fn ambient_trace_id_returns_none_without_env() {
    let id = resolve_ambient_capsem_trace_id(None, None);
    assert_eq!(id, None);
}

#[test]
fn ambient_trace_id_extracts_lower_half_from_traceparent() {
    let id = resolve_ambient_capsem_trace_id(
        None,
        Some("00-11111111111111112222222222222222-3333333333333333-01"),
    );
    assert_eq!(id.as_deref(), Some("2222222222222222"));
}

#[test]
fn debug_telemetry_policy_is_local_only_by_default() {
    let policy = debug_telemetry_policy_from_pairs([
        (
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "http://collector.example:4317",
        ),
        ("OTEL_TRACES_EXPORTER", "otlp"),
    ]);

    assert!(!policy.local_debug_enabled);
    assert!(!policy.upstream_export_allowed);
    assert_eq!(
        policy.blocked_upstream_env,
        vec!["OTEL_EXPORTER_OTLP_ENDPOINT", "OTEL_TRACES_EXPORTER"]
    );
}

#[test]
fn debug_telemetry_policy_enables_local_debug_filter_only() {
    let policy = debug_telemetry_policy_from_pairs([(DEBUG_TELEMETRY_ENV, "local")]);

    assert!(policy.local_debug_enabled);
    assert!(!policy.upstream_export_allowed);
    assert!(policy.blocked_upstream_env.is_empty());

    let filter = default_filter_with_debug_telemetry("capsem=info", &policy);
    assert!(filter.contains("capsem=info"));
    assert!(filter.contains("capsem.mitm=debug"));
    assert!(filter.contains("capsem.db=debug"));
}

#[test]
fn upstream_otel_requires_explicit_allow_env() {
    let policy = debug_telemetry_policy_from_pairs([
        (
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "http://collector.example:4317",
        ),
        (ALLOW_UPSTREAM_OTEL_ENV, "true"),
    ]);

    assert!(policy.upstream_export_allowed);
    assert!(policy.blocked_upstream_env.is_empty());
}

#[test]
fn launch_span_names_match_contract() {
    for name in [
        LAUNCH_SERVICE_SPAN,
        LAUNCH_GATEWAY_SPAN,
        LAUNCH_PROCESS_SPAWN_SPAN,
        LAUNCH_VM_BOOT_SPAN,
        LAUNCH_VSOCK_READY_SPAN,
        LAUNCH_FIRST_NETWORK_READY_SPAN,
    ] {
        assert!(name.starts_with("capsem.launch."));
        assert!(!name.contains("path"));
        assert!(!name.contains("url"));
        assert!(!name.contains("host"));
    }
}

// ---------------------------------------------------------------------------
// Log rotation and retention
// ---------------------------------------------------------------------------

#[test]
fn rolling_parts_keeps_rotated_files_matching_a_log_glob() {
    // `service.2026-07-30.log`, not `service.log.2026-07-30`: the asset gate's
    // failure-evidence copy and every operator's `ls *.log` filter on the
    // extension, and a rotated file that stops matching is a file nobody
    // collects.
    let (dir, prefix, suffix) = rolling_parts(std::path::Path::new("/run/capsem/service.log"));

    assert_eq!(dir, std::path::Path::new("/run/capsem"));
    assert_eq!(prefix, "service");
    assert_eq!(suffix, "log");
}

#[test]
fn rolling_parts_survives_a_path_with_no_extension_or_parent() {
    let (dir, prefix, suffix) = rolling_parts(std::path::Path::new("service"));

    assert_eq!(dir, std::path::Path::new("."));
    assert_eq!(prefix, "service");
    assert_eq!(suffix, "log");
}

#[test]
fn rolling_appender_writes_a_dated_file_beside_the_requested_path() {
    use std::io::Write;

    let dir = tempfile::tempdir().expect("tempdir");
    let mut appender = rolling_appender(&dir.path().join("service.log")).expect("appender");
    writeln!(appender, "{{\"message\":\"hello\"}}").expect("write");
    appender.flush().expect("flush");

    let written: Vec<String> = std::fs::read_dir(dir.path())
        .expect("read_dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    assert_eq!(
        written.len(),
        1,
        "expected exactly one log file, got {written:?}"
    );
    let name = &written[0];
    assert!(
        name.starts_with("service."),
        "{name} is not in the service stream"
    );
    assert!(
        name.ends_with(".log"),
        "{name} would not match a *.log collector"
    );
    assert!(
        std::fs::read_to_string(dir.path().join(name))
            .expect("read back")
            .contains("hello"),
        "the appender did not write through to disk"
    );
}

#[test]
fn log_stream_files_returns_the_whole_stream_newest_first() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Written oldest to newest so mtime ordering is unambiguous.
    for name in [
        "service.2026-07-28.log",
        "service.2026-07-29.log",
        "service.log",
    ] {
        std::fs::write(dir.path().join(name), name).expect("write");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // Neighbours that belong to other streams must not be swept in.
    std::fs::write(dir.path().join("gateway.2026-07-29.log"), "x").expect("write");
    std::fs::write(dir.path().join("service.pid"), "1").expect("write");

    let found = log_stream_files(&dir.path().join("service.log"));
    let names: Vec<String> = found
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();

    assert_eq!(
        names,
        vec![
            "service.log",
            "service.2026-07-29.log",
            "service.2026-07-28.log"
        ],
        "a support bundle reads this order to spend its byte budget on recent history"
    );
}

#[test]
fn log_stream_files_still_finds_the_unrotated_log_an_older_install_left() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("service.log"), "legacy").expect("write");

    let found = log_stream_files(&dir.path().join("service.log"));

    assert_eq!(found, vec![dir.path().join("service.log")]);
}

#[test]
fn log_stream_files_is_empty_rather_than_panicking_on_a_missing_directory() {
    assert!(log_stream_files(std::path::Path::new("/nonexistent/capsem/service.log")).is_empty());
}

#[test]
fn retained_log_files_is_bounded() {
    assert!(
        (1..=31).contains(&LOG_FILES_RETAINED),
        "retention must be bounded; unbounded logs are what this constant exists to prevent"
    );
}

#[test]
fn a_capped_writer_rotates_instead_of_growing_without_bound() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("serial.log");

    let mut writer = CappedLogWriter::open(&path, 64).unwrap();
    for _ in 0..20 {
        writer.write_all(b"0123456789").unwrap();
    }
    writer.flush().unwrap();

    // Bounded by two files of the cap, not by the guest's appetite.
    let total: u64 = log_stream_files(&path)
        .iter()
        .filter_map(|f| std::fs::metadata(f).ok())
        .map(|m| m.len())
        .sum();
    assert!(total <= 2 * 64, "serial log grew unbounded: {total} bytes");
}

#[test]
fn a_rotated_serial_file_stays_inside_its_stream() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("serial.log");

    let mut writer = CappedLogWriter::open(&path, 16).unwrap();
    writer
        .write_all(b"first half aaaaaaaaaaaaaaaaaaaa")
        .unwrap();
    writer
        .write_all(b"second half bbbbbbbbbbbbbbbbbbb")
        .unwrap();
    writer.flush().unwrap();

    // A rotated name outside the stream is a file nobody collects.
    assert_eq!(
        log_stream_files(&path).len(),
        2,
        "rotated file must remain in the stream the readers enumerate"
    );
}
