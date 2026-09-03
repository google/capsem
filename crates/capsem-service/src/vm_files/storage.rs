//! Host and rootfs storage figures for a session directory.

use super::*;

#[cfg(unix)]
pub(crate) fn physical_bytes(metadata: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.blocks() * 512
}

#[cfg(not(unix))]
pub(crate) fn physical_bytes(metadata: &std::fs::Metadata) -> u64 {
    metadata.len()
}

pub(crate) fn storage_diagnostics(session_dir: &StdPath) -> Option<api::StorageDiagnostics> {
    let rootfs_image_path = capsem_core::guest_share_dir(session_dir).join("system/rootfs.img");
    let metadata = std::fs::metadata(&rootfs_image_path).ok()?;
    let space = capsem_foundation::unix::fs::filesystem_space(session_dir).ok()?;

    Some(api::StorageDiagnostics {
        rootfs_image_path: rootfs_image_path.to_string_lossy().to_string(),
        rootfs_image_logical_bytes: metadata.len(),
        rootfs_image_physical_bytes: physical_bytes(&metadata),
        host_total_bytes: space.total_bytes,
        host_free_bytes: space.free_bytes,
        host_available_bytes: space.available_bytes,
        guest_overlay_device: "/dev/vdb".into(),
        guest_overlay_mount: "/".into(),
    })
}
