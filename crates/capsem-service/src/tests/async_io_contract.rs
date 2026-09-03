//! Source contract: no blocking I/O on the async runtime.
//!
//! Every `async fn` in the service is a tokio-worker function. A filesystem
//! read, a directory listing, a registry save or a child spawn there stalls
//! every other request until it returns, and the UI polls list and status
//! routes several times a second. Blocking work goes through
//! `ServiceState::off_worker` or `spawn_blocking`; this test reads the
//! sources and refuses the direct calls, including the synchronous helpers
//! that do the reading, so a new route cannot quietly bring one back.

/// Calls that block the thread they run on.
const DIRECT_BLOCKING: &[&str] = &[
    "std::fs::read",
    "std::fs::write",
    "std::fs::rename",
    "std::fs::copy",
    "std::fs::create_dir",
    "read_to_string(",
    "read_dir(",
    "File::open(",
    "File::create(",
    "remove_dir_all(",
    "registry.save()",
    ".register(",
    ".unregister(",
    "hash_file(",
    "statvfs(",
];

/// Synchronous helpers whose bodies do the reading.
const BLOCKING_HELPERS: &[&str] = &[
    "resume_sandbox(",
    "provision_sandbox(",
    "existing_session_names(",
    "find_failed_session_dir(",
    "read_process_log_tail(",
    "read_session_log_lines(",
    "read_boot_failure_tail(",
    "scan_panics_in_file(",
    "scan_errors_in_file(",
    "scan_slow_ops_in_file(",
    "latest_app_log(",
    "list_response_fingerprint(",
    "build_list_response(",
    "reconcile_persistent_defunct_from_logs(",
    "storage_diagnostics_cached(",
    "storage_diagnostics(",
    "persistent_entry_resume_state_cached(",
    "persistent_entry_resume_state(",
    "read_local_profile_obom(",
    "read_installed_status_document(",
    "read_manifest_metadata_status_document(",
    "update_status_response(",
    "refresh_active_profiles(",
    "refresh_profile_rule_cache(",
    "refresh_profile_plugin_policy_cache(",
    "settle_persistent_session_dir(",
    "claim_persistent_name(",
    "delete_session_dir(",
    "archive_failed_restore_checkpoint(",
    "clear_resume_checkpoint(",
    "preserve_failed_session_dir(",
];

/// Startup runs before the service accepts a request; a stall there delays
/// readiness, not another caller. `off_worker` is the door itself.
const EXEMPT_FUNCTIONS: &[&str] = &["run_service", "spawn_companions", "off_worker"];

fn source_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read service sources").flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "tests") {
                continue;
            }
            source_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") && path.file_name() != Some("tests.rs".as_ref()) {
            out.push(path);
        }
    }
}

/// The `{ ... }` body starting at the first brace after `from`.
fn brace_body(source: &str, from: usize) -> Option<&str> {
    let open = from + source[from..].find('{')?;
    let mut depth = 0usize;
    for (offset, byte) in source[open..].bytes().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[open..=open + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

/// `body` with every `spawn_blocking(...)` and `off_worker(...)` argument
/// list removed: that is the code allowed to block.
fn without_blocking_closures(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    loop {
        let Some(at) = ["spawn_blocking(", "off_worker("]
            .iter()
            .filter_map(|door| rest.find(door).map(|i| i + door.len()))
            .min()
        else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..at]);
        let mut depth = 1usize;
        let mut end = rest.len();
        for (offset, byte) in rest[at..].bytes().enumerate() {
            match byte {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = at + offset;
                        break;
                    }
                }
                _ => {}
            }
        }
        rest = &rest[end..];
    }
}

#[test]
fn async_functions_do_not_block_the_runtime() {
    let mut files = Vec::new();
    source_files(
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src")),
        &mut files,
    );
    files.sort();
    assert!(files.len() > 10, "the scan must see the service sources");

    let mut offences = Vec::new();
    for path in &files {
        let source = std::fs::read_to_string(path).unwrap();
        let mut search = 0;
        while let Some(found) = source[search..].find("async fn ") {
            let start = search + found + "async fn ".len();
            let name_end = source[start..]
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .map_or(source.len(), |i| start + i);
            let name = &source[start..name_end];
            search = name_end;
            if EXEMPT_FUNCTIONS.contains(&name) {
                continue;
            }
            let Some(body) = brace_body(&source, name_end) else {
                continue;
            };
            // tokio::fs is the async door; only the std spellings block.
            let visible = without_blocking_closures(body)
                .replace("tokio::fs::File::open(", "tokio_fs_open(")
                .replace("tokio::fs::File::create(", "tokio_fs_create(");
            for token in DIRECT_BLOCKING.iter().chain(BLOCKING_HELPERS) {
                if visible.contains(token) {
                    offences.push(format!(
                        "{}: async fn {name} calls {token} on the runtime",
                        path.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap().display()
                    ));
                }
            }
        }
    }
    assert!(
        offences.is_empty(),
        "blocking calls belong in ServiceState::off_worker or spawn_blocking:\n{}",
        offences.join("\n")
    );
}
