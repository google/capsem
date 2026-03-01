pub mod gateway;
pub mod host_state;
pub mod mcp;
pub mod net;
pub mod session;
pub mod vm;

use std::path::Path;

pub use capsem_proto;
pub use capsem_proto::{
    GuestToHost, HostToGuest, MAX_FRAME_SIZE, decode_guest_msg, decode_host_msg, encode_guest_msg,
    encode_host_msg,
};
pub use host_state::{
    HostState, HostStateMachine, StateMachine, Transition, validate_guest_msg, validate_host_msg,
};
pub use vm::config::VmConfig;
pub use vm::machine::VirtualMachine;
pub use vm::vsock::{
    self, CoalesceBuffer, VsockConnection, VsockManager, VSOCK_PORT_CONTROL,
    VSOCK_PORT_FS_WATCH, VSOCK_PORT_MCP_GATEWAY, VSOCK_PORT_SNI_PROXY, VSOCK_PORT_TERMINAL,
};

/// Create a sparse scratch disk image file.
///
/// The file is created with the given size using `set_len` (sparse -- doesn't
/// allocate actual disk space until written). Permissions are set to 0600 to
/// prevent other host users from reading scratch data.
///
/// The guest formats this disk at boot (ext4, no journal).
pub fn create_scratch_disk(path: &Path, size_gb: u32) -> std::io::Result<()> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;

    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_len(size_gb as u64 * 1024 * 1024 * 1024)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn create_scratch_disk_sparse_file() {
        let dir = std::env::temp_dir().join("capsem-test-scratch");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test-scratch.img");

        create_scratch_disk(&path, 1).unwrap();

        let meta = std::fs::metadata(&path).unwrap();
        // Logical size should be 1GB
        assert_eq!(meta.len(), 1024 * 1024 * 1024);
        // Sparse file: actual blocks should be much less than 1GB
        // (blocks are in 512-byte units)
        assert!(meta.blocks() < 1024, "file should be sparse, blocks={}", meta.blocks());
        // Permissions should be 0600
        assert_eq!(meta.mode() & 0o777, 0o600);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn create_scratch_disk_larger_size() {
        let dir = std::env::temp_dir().join("capsem-test-scratch");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test-scratch-8gb.img");

        create_scratch_disk(&path, 8).unwrap();

        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.len(), 8 * 1024 * 1024 * 1024);
        assert!(meta.blocks() < 1024, "file should be sparse");

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn create_scratch_disk_overwrites_existing() {
        let dir = std::env::temp_dir().join("capsem-test-scratch");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test-scratch-overwrite.img");

        // Create a 1GB file first
        create_scratch_disk(&path, 1).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 1024 * 1024 * 1024);

        // Overwrite with 2GB
        create_scratch_disk(&path, 2).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 2 * 1024 * 1024 * 1024);

        std::fs::remove_file(&path).unwrap();
    }
}
