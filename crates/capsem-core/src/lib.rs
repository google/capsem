pub mod asset_manager;
pub mod auto_snapshot;
pub mod fs_monitor;
pub mod host_config;
pub mod host_state;
pub mod log_layer;
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
pub use vm::config::{VirtioFsShare, VmConfig};
pub use vm::machine::VirtualMachine;
pub use vm::VmState;
pub use vm::vsock::{
    self, CoalesceBuffer, VsockConnection, VsockManager, VSOCK_PORT_CONTROL,
    VSOCK_PORT_MCP_GATEWAY, VSOCK_PORT_SNI_PROXY, VSOCK_PORT_TERMINAL,
};

/// Create VirtioFS session directories for the single-share hybrid architecture.
///
/// Layout (shared as one VirtioFS share with the guest):
/// - `system/rootfs.img` -- sparse ext4 loopback image for overlayfs upper
///                          (packages, config, /run binaries)
/// - `workspace/`        -- direct host-visible files for /root (AI workspace)
/// - `auto_snapshots/`   -- rolling ring buffer for host-side APFS clone snapshots
///
/// The host creates a sparse `rootfs.img` (0 bytes actual). The guest formats
/// it as ext4 on first boot (~1s). Forked sessions already have a formatted
/// image (APFS-cloned from snapshot).
pub fn create_virtiofs_session(session_dir: &Path, system_img_size_gb: u32) -> std::io::Result<()> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    std::fs::create_dir_all(session_dir.join("system"))?;
    std::fs::create_dir_all(session_dir.join("workspace"))?;
    std::fs::create_dir_all(session_dir.join("auto_snapshots"))?;

    let img_path = session_dir.join("system").join("rootfs.img");
    if !img_path.exists() {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&img_path)?;
        file.set_len(system_img_size_gb as u64 * 1024 * 1024 * 1024)?;
    }

    std::fs::set_permissions(session_dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

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
    use std::path::PathBuf;

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

    #[test]
    fn create_virtiofs_session_creates_layout() {
        let dir = std::env::temp_dir().join("capsem-test-virtiofs-session3");
        let _ = std::fs::remove_dir_all(&dir);

        create_virtiofs_session(&dir, 2).unwrap();

        assert!(dir.join("system").is_dir());
        assert!(dir.join("workspace").is_dir());
        assert!(dir.join("auto_snapshots").is_dir());

        let img = dir.join("system/rootfs.img");
        assert!(img.exists());
        let meta = std::fs::metadata(&img).unwrap();
        assert_eq!(meta.len(), 2 * 1024 * 1024 * 1024);
        assert!(meta.blocks() < 1024, "rootfs.img should be sparse");

        let dir_meta = std::fs::metadata(&dir).unwrap();
        assert_eq!(dir_meta.mode() & 0o777, 0o700);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn create_virtiofs_session_idempotent() {
        let dir = std::env::temp_dir().join("capsem-test-virtiofs-idem3");
        let _ = std::fs::remove_dir_all(&dir);

        create_virtiofs_session(&dir, 1).unwrap();
        create_virtiofs_session(&dir, 1).unwrap(); // should not fail or recreate

        assert!(dir.join("system/rootfs.img").exists());
        assert!(dir.join("workspace").is_dir());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Compile-time guard: every public re-export from lib.rs must be
    /// reachable. If Phase 1 moves modules and forgets to update the
    /// re-export, this test fails to compile.
    #[test]
    fn reexport_surface_compiles() {
        fn assert_type<T>(_: &T) {}

        // VmConfig + builder
        let kernel = std::env::temp_dir().join("capsem-reexport-guard");
        std::fs::create_dir_all(&kernel).unwrap();
        let kpath = kernel.join("vmlinuz");
        std::fs::write(&kpath, b"fake").unwrap();
        let cfg = VmConfig::builder().kernel_path(&kpath).build().unwrap();
        assert_type::<VmConfig>(&cfg);

        // VirtioFsShare
        let _share = VirtioFsShare {
            tag: "test".into(),
            host_path: PathBuf::from("/tmp"),
            read_only: false,
        };

        // VmState
        let st = VmState::Running;
        assert_eq!(st.as_str(), "running");
        assert_eq!(VmState::parse("running"), VmState::Running);

        // CoalesceBuffer
        let mut buf = CoalesceBuffer::new();
        buf.push(b"x");
        let _ = buf.take();

        // VsockConnection (just verify the type exists)
        let _ = std::mem::size_of::<VsockConnection>();

        // Port constants
        let _ports = [
            VSOCK_PORT_CONTROL,
            VSOCK_PORT_TERMINAL,
            VSOCK_PORT_SNI_PROXY,
            VSOCK_PORT_MCP_GATEWAY,
        ];

        // Proto re-exports
        let _ = MAX_FRAME_SIZE;
        let _ = std::mem::size_of::<GuestToHost>();
        let _ = std::mem::size_of::<HostToGuest>();

        // Host state machine
        let _ = std::mem::size_of::<HostState>();
        let _ = std::mem::size_of::<HostStateMachine>();

        // Codec functions (verify they exist as fn pointers)
        let _: fn(&GuestToHost) -> anyhow::Result<Vec<u8>> = encode_guest_msg;
        let _: fn(&HostToGuest) -> anyhow::Result<Vec<u8>> = encode_host_msg;

        // Cleanup
        std::fs::remove_dir_all(&kernel).unwrap();
    }
}
