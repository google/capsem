#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use tauri::{Emitter, Manager};
use tracing::{info, warn};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt::format::FmtSpan, EnvFilter};

// ---------- IPC commands ----------

#[tauri::command]
fn log_frontend(level: String, message: String) {
    // tracing handles output formatting/sinks; no eprintln -- it duplicated
    // every line in the launching terminal (once here, once from the fmt
    // subscriber) and made `just run-ui` unreadable when vmStore polls.
    match level.as_str() {
        "error" => tracing::error!(target: "frontend", "{message}"),
        "warn" => tracing::warn!(target: "frontend", "{message}"),
        "info" => tracing::info!(target: "frontend", "{message}"),
        _ => tracing::debug!(target: "frontend", "{message}"),
    }
}

/// T5/F2: return the absolute path to the most recent frontend log
/// (jsonl) under `<capsem_home>/logs/`. Console-only debug handle in
/// `tauri-log.ts` invokes this and copies the result to clipboard so
/// devs can `cat` the file from the host shell.
#[tauri::command]
async fn dump_frontend_logs() -> Result<String, String> {
    let dir = capsem_home_dir().join("logs");
    let entries =
        std::fs::read_dir(&dir).map_err(|e| format!("read_dir({}): {e}", dir.display()))?;
    let mut latest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let modified = entry.metadata().and_then(|m| m.modified()).ok();
        if let Some(m) = modified {
            match &latest {
                None => latest = Some((m, p)),
                Some((t, _)) if m > *t => latest = Some((m, p)),
                _ => {}
            }
        }
    }
    latest
        .map(|(_, p)| p.display().to_string())
        .ok_or_else(|| format!("no jsonl logs in {}", dir.display()))
}

#[tauri::command]
async fn open_url(url: String, app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(&url, None::<&str>)
        .map_err(|e| e.to_string())
}

// ---------- Deep link handling (--connect <vm_id>) ----------

fn parse_flag(args: &[String], flag: &str) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == flag && i + 1 < args.len() {
            return Some(args[i + 1].clone());
        }
        i += 1;
    }
    None
}

fn parse_connect_arg(args: &[String]) -> Option<String> {
    parse_flag(args, "--connect")
}

fn parse_action_arg(args: &[String]) -> Option<String> {
    parse_flag(args, "--action")
}

/// Build the deep-link payload as a JSON value.
///
/// AB-003: the previous implementation hand-escaped only single quotes and
/// interpolated `vm_id` / `action` directly into a single-quoted JS string
/// literal. A backslash, newline, or `'); ...; //` payload broke out of the
/// literal and ran as code in a webview that holds the gateway token --
/// effective full local capsem control. JSON is a strict subset of valid JS
/// for object/string literals, so embedding `serde_json::Value` output is
/// safe by construction: every `"`, `\`, control char, and high-bit code
/// point gets the proper escape.
fn build_deep_link_payload(vm_id: &str, action: Option<&str>) -> serde_json::Value {
    match action {
        Some(a) => serde_json::json!({ "connect": vm_id, "action": a }),
        None => serde_json::json!({ "connect": vm_id }),
    }
}

/// Build the JS one-liner that invokes `window.__capsemDeepLink({...})` with
/// the deep-link payload. See [`build_deep_link_payload`] for the safety
/// rationale.
fn build_deep_link_script(vm_id: &str, action: Option<&str>) -> String {
    let payload = build_deep_link_payload(vm_id, action);
    format!("if (window.__capsemDeepLink) {{ window.__capsemDeepLink({payload}) }}")
}

fn dispatch_deep_link(window: &tauri::WebviewWindow, vm_id: &str, action: Option<&str>) {
    let _ = window.eval(build_deep_link_script(vm_id, action));
}

// ---------- Log housekeeping ----------

fn cleanup_old_logs(dir: &Path, max_days: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let cutoff = now.saturating_sub(max_days * 86400);
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let Ok(mtime) = modified.duration_since(std::time::UNIX_EPOCH) else {
            continue;
        };
        if mtime.as_secs() < cutoff {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn log_filename() -> String {
    let secs = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_log_filename(secs)
}

fn format_log_filename(secs: u64) -> String {
    let t = secs % 86400;
    let days = secs / 86400;
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}-{:02}-{:02}.jsonl",
        t / 3600,
        (t % 3600) / 60,
        t % 60
    )
}

/// Open (or create-truncate) the per-launch app log at `path`.
///
/// Logs contain tracing spans with VM ids, filesystem paths, provider API
/// metadata, and tool-call arguments. On a shared macOS/Linux box a
/// world-readable log is a user-to-user information leak, so the file is
/// always created with mode 0o600 on Unix. Mirrors the pattern used for
/// serial.log, the gateway auth token, and per-VM sockets (see
/// `/dev-rust-patterns` lesson 14).
fn open_log_file(path: &Path) -> std::io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
    }
}

/// Resolve the capsem home dir without pulling in capsem-core (thin-shell invariant).
/// Mirrors `capsem_core::paths::capsem_home` priority: CAPSEM_HOME > $HOME/.capsem.
fn capsem_home_dir() -> PathBuf {
    if let Ok(h) = std::env::var("CAPSEM_HOME") {
        if !h.is_empty() {
            return PathBuf::from(h);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".capsem")
}

#[cfg(all(unix, target_os = "macos"))]
fn service_socket_path() -> PathBuf {
    capsem_home_dir().join("run/service.sock")
}

#[cfg(all(unix, any(target_os = "macos", test)))]
fn ensure_tray_request() -> &'static str {
    "POST /companions/tray/ensure HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
}

#[cfg(all(unix, any(target_os = "macos", test)))]
fn parse_http_status(response: &str) -> Option<u16> {
    response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|raw| raw.parse::<u16>().ok())
}

#[cfg(all(unix, target_os = "macos"))]
fn ensure_tray_once(sock: &Path) -> Result<u16, String> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    let mut stream =
        UnixStream::connect(sock).map_err(|e| format!("connect({}): {e}", sock.display()))?;
    let timeout = Some(std::time::Duration::from_millis(800));
    let _ = stream.set_read_timeout(timeout);
    let _ = stream.set_write_timeout(timeout);
    stream
        .write_all(ensure_tray_request().as_bytes())
        .map_err(|e| format!("write ensure request: {e}"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|e| format!("read ensure response: {e}"))?;
    parse_http_status(&response).ok_or_else(|| "missing HTTP status".to_string())
}

fn ensure_tray_nonblocking() {
    #[cfg(target_os = "macos")]
    std::thread::spawn(|| {
        let sock = service_socket_path();
        match ensure_tray_once(&sock) {
            Ok(status) if (200..300).contains(&status) => {
                info!(status, "requested service tray ensure");
            }
            Ok(status) => {
                warn!(status, "service tray ensure returned non-success");
            }
            Err(e) => {
                warn!(error = %e, "service tray ensure request failed");
            }
        }
    });
}

fn main() {
    // Log to <capsem_home>/logs/<timestamp>.jsonl
    let log_dir = capsem_home_dir().join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    cleanup_old_logs(&log_dir, 7);

    let log_path = log_dir.join(log_filename());
    let file_layer = open_log_file(&log_path).ok().map(|f| {
        let (nb, guard) = tracing_appender::non_blocking(f);
        // Leak the guard — we want logs flushed for the entire process lifetime.
        Box::leak(Box::new(guard));
        tracing_subscriber::fmt::layer()
            .json()
            .with_writer(nb)
            .with_span_events(FmtSpan::CLOSE)
    });

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("capsem_app=info,frontend=info"));

    let stdout_layer = tracing_subscriber::fmt::layer().with_span_events(FmtSpan::CLOSE);

    tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer)
        .init();

    let cli_args: Vec<String> = std::env::args().skip(1).collect();
    info!(
        version = env!("CARGO_PKG_VERSION"),
        built = option_env!("CAPSEM_BUILD_TS").unwrap_or("dev"),
        args = ?cli_args,
        "starting capsem-app"
    );
    // C3: emit a `service.start` line that mirrors what other capsem
    // binaries log via capsem_core::telemetry::init(). Lets the support
    // bundle parser key on cross-version-mix detection for capsem-app
    // too -- without taking a capsem-core dep (project invariant).
    info!(
        target: "service",
        service = "capsem-app",
        protocol_version = capsem_proto::PROTOCOL_VERSION,
        schema_hash = format!("{:016x}", capsem_proto::SCHEMA_HASH),
        parent_traceparent = std::env::var("TRACEPARENT").unwrap_or_default(),
        "service.start",
    );

    let connect_id = parse_connect_arg(&cli_args);
    let initial_action = parse_action_arg(&cli_args);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            info!(args = ?args, "single-instance: second launch");
            ensure_tray_nonblocking();
            let Some(window) = app.get_webview_window("main") else {
                warn!("single-instance: main window missing");
                return;
            };
            let _ = window.set_focus();
            if let Some(id) = parse_connect_arg(&args) {
                let action = parse_action_arg(&args);
                dispatch_deep_link(&window, &id, action.as_deref());
            }
        }))
        .setup(move |app| {
            ensure_tray_nonblocking();
            tauri::async_runtime::spawn(async {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
                loop {
                    interval.tick().await;
                    ensure_tray_nonblocking();
                }
            });
            if let Some(window) = app.get_webview_window("main") {
                window.on_window_event(|event| {
                    if matches!(event, tauri::WindowEvent::Focused(true)) {
                        ensure_tray_nonblocking();
                    }
                });
            }
            if let Some(id) = connect_id.clone() {
                let action = initial_action.clone();
                let window = app
                    .get_webview_window("main")
                    .expect("main window must exist");
                tauri::async_runtime::spawn(async move {
                    // Let the frontend mount __capsemDeepLink before dispatching.
                    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                    dispatch_deep_link(&window, &id, action.as_deref());
                });
            }

            // Emit an init event for the frontend so it can detect Tauri context.
            let _ = app.handle().emit("capsem-ready", ());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            log_frontend,
            open_url,
            dump_frontend_logs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
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

    #[cfg(unix)]
    #[test]
    fn ensure_tray_request_targets_service_endpoint() {
        let request = ensure_tray_request();
        assert!(request.starts_with("POST /companions/tray/ensure HTTP/1.1\r\n"));
        assert!(request.contains("Content-Length: 0\r\n"));
        assert!(request.ends_with("\r\n\r\n"));
    }

    #[cfg(unix)]
    #[test]
    fn parse_http_status_reads_status_code() {
        assert_eq!(
            parse_http_status("HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}"),
            Some(200)
        );
        assert_eq!(parse_http_status("not http"), None);
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
}
