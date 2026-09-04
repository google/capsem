//! The boot stage timings capsem-init leaves for the agent to report.

use capsem_proto::BootStage;

/// Path to the boot timing JSONL file written by capsem-init.
pub(crate) const BOOT_TIMING_PATH: &str = "/run/capsem-boot-timing";

/// Parse boot timing JSONL file. Each line: {"name":"...","duration_ms":...}
/// Rejects entries with non-alphanumeric names (defense against injection).
pub(crate) fn parse_boot_timing(path: &str) -> Vec<BootStage> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter_map(|line| serde_json::from_str::<BootStage>(line).ok())
        .filter(|s| {
            s.name.len() <= 64
                && !s.name.is_empty()
                && s.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && s.duration_ms <= 600_000
        })
        .take(32)
        .collect()
}
