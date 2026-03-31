use std::path::Path;

use capsem_core::HostStateMachine;
use tracing::info;

/// Delete `.jsonl` log files in `dir` older than `max_days`.
/// Parses the ISO 8601 timestamp from filenames (e.g. `2026-03-17T10-05-32.jsonl`).
pub(crate) fn cleanup_old_logs(dir: &Path, max_days: u64) {
    let cutoff = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .saturating_sub(max_days * 86400);

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_stem().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let ext = path.extension().and_then(|e| e.to_str());
        if ext != Some("jsonl") {
            continue;
        }

        // Parse "2026-03-17T10-05-32" -> epoch seconds
        if let Some(epoch) = parse_log_filename_epoch(&name) {
            if epoch < cutoff {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

/// Parse a log filename like "2026-03-17T10-05-32" to epoch seconds.
pub(crate) fn parse_log_filename_epoch(name: &str) -> Option<u64> {
    // Expected format: YYYY-MM-DDThh-mm-ss
    if name.len() < 19 {
        return None;
    }
    let y: i64 = name[0..4].parse().ok()?;
    let m: u64 = name[5..7].parse().ok()?;
    let d: u64 = name[8..10].parse().ok()?;
    let h: u64 = name[11..13].parse().ok()?;
    let min: u64 = name[14..16].parse().ok()?;
    let s: u64 = name[17..19].parse().ok()?;

    // Convert civil date to days since epoch (inverse of Hinnant algorithm)
    let y_adj = if m <= 2 { y - 1 } else { y };
    let m_adj = if m <= 2 { m + 9 } else { m - 3 };
    let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
    let yoe = (y_adj - era * 400) as u64;
    let doy = (153 * m_adj + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = (era * 146097 + doe as i64 - 719468) as u64;

    Some(days * 86400 + h * 3600 + min * 60 + s)
}

/// Write boot performance data from the state machine to ~/.capsem/perf/<timestamp>.log
/// and emit structured tracing events for each state transition.
pub(crate) fn write_perf_log(sm: &HostStateMachine) {
    // Emit structured tracing events for each transition.
    for t in sm.history() {
        info!(
            category = "boot_timeline",
            from = %t.from, to = %t.to,
            trigger = %t.trigger,
            duration_ms = t.duration_in_from.as_millis() as u64,
            "state transition"
        );
    }

    let log = sm.format_perf_log();
    if log.is_empty() {
        return;
    }
    eprint!("{log}");
    // Keep writing to ~/.capsem/perf/ for backward compat
    let home = match std::env::var("HOME") {
        Ok(h) => std::path::PathBuf::from(h),
        Err(_) => return,
    };
    let dir = home.join(".capsem").join("perf");
    let _ = std::fs::create_dir_all(&dir);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let path = dir.join(format!("{ts}.log"));
    let _ = std::fs::write(&path, &log);
    eprintln!("perf log: {}", path.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_log_filename_epoch_valid() {
        let epoch = parse_log_filename_epoch("2026-03-17T10-05-32").unwrap();
        assert_eq!(epoch, 1773741932);
    }

    #[test]
    fn parse_log_filename_epoch_start() {
        let epoch = parse_log_filename_epoch("1970-01-01T00-00-00").unwrap();
        assert_eq!(epoch, 0);
    }

    #[test]
    fn parse_log_filename_epoch_too_short() {
        assert!(parse_log_filename_epoch("2026").is_none());
    }

    #[test]
    fn parse_log_filename_epoch_invalid_chars() {
        assert!(parse_log_filename_epoch("XXXX-XX-XXTXX-XX-XX").is_none());
    }

    #[test]
    fn cleanup_old_logs_deletes_old_files() {
        let dir = std::env::temp_dir().join("capsem-test-cleanup-logs");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(dir.join("2020-01-01T00-00-00.jsonl"), "old").unwrap();
        std::fs::write(dir.join("2026-03-31T10-05-32.jsonl"), "new").unwrap();
        std::fs::write(dir.join("2020-01-01T00-00-00.txt"), "keep").unwrap();

        cleanup_old_logs(&dir, 7);

        assert!(!dir.join("2020-01-01T00-00-00.jsonl").exists(), "old file should be deleted");
        assert!(dir.join("2026-03-31T10-05-32.jsonl").exists(), "recent file should be kept");
        assert!(dir.join("2020-01-01T00-00-00.txt").exists(), "non-jsonl should be kept");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_old_logs_handles_empty_dir() {
        let dir = std::env::temp_dir().join("capsem-test-cleanup-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        cleanup_old_logs(&dir, 7);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
