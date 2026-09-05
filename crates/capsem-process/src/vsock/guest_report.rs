//! Informational guest messages: liveness replies and boot timing.

use capsem_proto::{BootStage, GuestToHost};
use tracing::{info, trace, warn};

/// Replies produced by the periodic control-channel liveness probe.
///
/// These messages carry no job result and need no replay acknowledgement. A
/// healthy VM sends one every few seconds, so treating them as an unknown wire
/// variant turns normal uptime into warning spam and hides the first useful
/// fault in a preserved process log.
pub(super) fn is_guest_liveness_message(msg: &GuestToHost) -> bool {
    matches!(msg, GuestToHost::Pong)
}

/// Stages the guest may report, and the longest a name may be. The guest
/// sanitizes before sending; the host does not trust that.
const MAX_BOOT_STAGES: usize = 32;
const MAX_STAGE_NAME: usize = 64;
/// No boot stage takes ten minutes; a larger value is a corrupt file.
const MAX_STAGE_MS: u64 = 600_000;

/// Record the guest's boot stages: one info line per stage plus the total,
/// so a slow boot names the stage that slowed and the lifecycle benchmark
/// can read the stages back. The message used to fall through to the
/// "unknown variant" warning and the numbers were lost.
pub(super) fn record_boot_timing(stages: Vec<BootStage>) -> Vec<BootStage> {
    let received = stages.len();
    let clean: Vec<BootStage> = stages
        .into_iter()
        .filter(|stage| {
            !stage.name.is_empty()
                && stage.name.len() <= MAX_STAGE_NAME
                && stage.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && stage.duration_ms <= MAX_STAGE_MS
        })
        .take(MAX_BOOT_STAGES)
        .collect();
    if clean.len() != received {
        warn!(target: "capsem.boot", received, kept = clean.len(), "boot timing: dropped invalid stages");
    }
    for stage in &clean {
        info!(target: "capsem.boot", stage = %stage.name, duration_ms = stage.duration_ms, "boot stage");
    }
    let total_ms: u64 = clean.iter().map(|stage| stage.duration_ms).sum();
    info!(target: "capsem.boot", total_ms, stages = clean.len(), "boot timing total");
    trace!(target: "capsem.boot", ?clean, "boot stages");
    clean
}

#[cfg(test)]
mod tests;
