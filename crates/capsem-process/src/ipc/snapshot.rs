//! Snapshot status projection for the IPC `SnapshotStatus` reply.

pub(super) fn snapshot_status_from_scheduler(
    scheduler: &capsem_core::auto_snapshot::AutoSnapshotScheduler,
) -> capsem_proto::ipc::SnapshotStatus {
    let snapshots = scheduler.list_snapshots();
    let auto_count = snapshots
        .iter()
        .filter(|slot| slot.origin == capsem_core::auto_snapshot::SnapshotOrigin::Auto)
        .count();
    let manual_count = snapshots.len().saturating_sub(auto_count);
    let snapshots = snapshots
        .into_iter()
        .map(|slot| capsem_proto::ipc::SnapshotSlotStatus {
            checkpoint: format!("cp-{}", slot.slot),
            slot: slot.slot,
            origin: match slot.origin {
                capsem_core::auto_snapshot::SnapshotOrigin::Auto => "auto",
                capsem_core::auto_snapshot::SnapshotOrigin::Manual => "manual",
            }
            .to_string(),
            name: slot.name,
            timestamp: snapshot_timestamp(slot.timestamp),
            hash: slot.hash,
        })
        .collect();

    capsem_proto::ipc::SnapshotStatus {
        total: auto_count + manual_count,
        auto_count,
        manual_count,
        manual_available: scheduler.available_manual_slots(),
        snapshots,
    }
}

fn snapshot_timestamp(timestamp: std::time::SystemTime) -> String {
    let secs = timestamp
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("unix:{secs}")
}
