//! Process name resolution from /proc filesystem.
//!
//! Reads /proc/{pid}/cmdline first (gets the actual binary name, not the thread
//! name), falls back to /proc/{pid}/comm, then "unknown".

/// Get the process name for a given PID.
///
/// 1. Read `/proc/{pid}/cmdline` (null-separated), extract basename of argv[0]
/// 2. Fall back to `/proc/{pid}/comm` if cmdline is empty (kernel threads)
/// 3. Fall back to `"unknown"`
pub fn process_name_for_pid(pid: u32) -> String {
    // Try cmdline first -- it has the real binary name, not the thread name.
    let cmdline_path = format!("/proc/{pid}/cmdline");
    if let Ok(data) = std::fs::read(&cmdline_path) {
        if !data.is_empty() {
            // cmdline is null-separated: argv[0]\0argv[1]\0...
            let argv0_end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
            let argv0 = String::from_utf8_lossy(&data[..argv0_end]);
            // Extract basename (e.g., "/usr/bin/gemini" -> "gemini")
            let basename = argv0.rsplit('/').next().unwrap_or(&argv0);
            if !basename.is_empty() {
                return basename.to_string();
            }
        }
    }

    // Fall back to comm (may return thread name like "MainThread")
    let comm_path = format!("/proc/{pid}/comm");
    if let Ok(comm) = std::fs::read_to_string(&comm_path) {
        let trimmed = comm.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    "unknown".to_string()
}

#[cfg(test)]
#[path = "procfs/tests.rs"]
mod tests;
