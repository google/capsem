//! Guest message classification, replay acknowledgement IDs, and boot timing.

use capsem_proto::{BootStage, GuestToHost, HostToGuest};
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

/// Returns `Some(id)` for HostToGuest variants whose delivery the host
/// bridge tracks via the pending-ack map. The agent acks these on
/// receipt; the bridge replays them on every fresh conn until acked.
/// Non-ackable variants (Resize, Ping, Shutdown, BootConfig, etc.) are
/// either side-effect-free or fire-and-forget at boot, so we don't
/// burden the wire with per-message acks for them.
pub(super) fn ackable_id(msg: &HostToGuest) -> Option<u64> {
    match msg {
        HostToGuest::Exec { id, .. }
        | HostToGuest::FileWrite { id, .. }
        | HostToGuest::FileRead { id, .. }
        | HostToGuest::FileDelete { id, .. } => Some(*id),
        _ => None,
    }
}

/// Returns `Some(id)` for `GuestToHost` variants the agent retains in
/// its symmetric pending_responses map and replays on every fresh
/// control conn. The host emits `HostToGuest::AckReply { id }` on
/// receipt so the agent can drop the entry. Mirrors `ackable_id` but
/// for the return path.
pub(super) fn ackable_response_id(msg: &GuestToHost) -> Option<u64> {
    match msg {
        GuestToHost::ExecDone { id, .. }
        | GuestToHost::FileOpDone { id }
        | GuestToHost::FileContent { id, .. }
        | GuestToHost::Error { id, .. } => Some(*id),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
