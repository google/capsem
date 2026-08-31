use super::*;
use capsem_proto::{encode_guest_msg, encode_host_msg, GuestToHost, HostToGuest, MAX_FRAME_SIZE};
use std::io::{Seek, SeekFrom, Write};
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

    // Real dirs live inside guest/
    assert!(dir.join("guest/system").is_dir());
    assert!(dir.join("guest/workspace").is_dir());
    assert!(dir.join("auto_snapshots").is_dir());

    // Compat symlinks at session root
    assert!(dir.join("system").is_symlink());
    assert!(dir.join("workspace").is_symlink());
    // Symlinks resolve to the guest/ dirs
    assert!(dir.join("system").is_dir());
    assert!(dir.join("workspace").is_dir());

    let img = dir.join("guest/system/rootfs.img");
    assert!(img.exists());
    let meta = std::fs::metadata(&img).unwrap();
    assert_eq!(meta.len(), 2 * 1024 * 1024 * 1024);
    assert!(meta.blocks() < 1024, "rootfs.img should be sparse");

    // Symlink path also works
    assert!(dir.join("system/rootfs.img").exists());

    // VirtioFS share dir is the guest/ subdir
    assert_eq!(guest_share_dir(&dir), dir.join("guest"));

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

#[test]
fn system_overlay_ext4_magic_detection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rootfs.img");
    let mut file = std::fs::File::create(&path).unwrap();
    file.set_len(4096).unwrap();

    assert!(!system_overlay_has_ext4_magic(&path).unwrap());

    file.seek(SeekFrom::Start(1080)).unwrap();
    file.write_all(&[0x53, 0xef]).unwrap();
    drop(file);

    assert!(system_overlay_has_ext4_magic(&path).unwrap());
}

#[cfg(target_os = "linux")]
#[test]
fn preformat_system_overlay_image_writes_ext4_magic_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rootfs.img");
    create_scratch_disk(&path, 1).unwrap();

    let first = preformat_system_overlay_image_if_needed(&path).unwrap();
    let second = preformat_system_overlay_image_if_needed(&path).unwrap();

    assert!(first);
    assert!(!second);
    assert!(system_overlay_has_ext4_magic(&path).unwrap());
}

#[cfg(target_os = "linux")]
#[test]
fn preformatted_system_overlay_template_clones_session_images() {
    let dir = tempfile::tempdir().unwrap();
    let template = dir.path().join("cache/rootfs-ext4-1g.img");
    let first = dir.path().join("sessions/a/guest/system/rootfs.img");
    let second = dir.path().join("sessions/b/guest/system/rootfs.img");

    let created = ensure_preformatted_system_overlay_template(&template, 1).unwrap();
    let reused = ensure_preformatted_system_overlay_template(&template, 1).unwrap();
    let first_cloned = preformat_system_overlay_image_from_template_if_needed(&first, &template, 1).unwrap();
    let second_cloned = preformat_system_overlay_image_from_template_if_needed(&second, &template, 1).unwrap();
    let first_reused = preformat_system_overlay_image_from_template_if_needed(&first, &template, 1).unwrap();

    assert!(created);
    assert!(!reused);
    assert!(first_cloned);
    assert!(second_cloned);
    assert!(!first_reused);
    assert!(system_overlay_has_ext4_magic(&first).unwrap());
    assert!(system_overlay_has_ext4_magic(&second).unwrap());
    assert_eq!(std::fs::metadata(&first).unwrap().len(), 1024 * 1024 * 1024);
    assert_eq!(std::fs::metadata(&second).unwrap().len(), 1024 * 1024 * 1024);
    assert!(
        std::fs::metadata(&first).unwrap().blocks() < 128 * 1024,
        "template clone should remain sparse"
    );
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

    // VsockConnection (verify the type exists via hypervisor re-export)
    let conn = VsockConnection::new(42, 5001, Box::new(()));
    assert_eq!(conn.fd, 42);
    assert_eq!(conn.port, 5001);

    // Port constants
    let _ports = [
        VSOCK_PORT_CONTROL,
        VSOCK_PORT_TERMINAL,
        VSOCK_PORT_SNI_PROXY,
        VSOCK_PORT_LIFECYCLE,
    ];

    // Shared protocol contracts
    let _ = MAX_FRAME_SIZE;
    let _ = std::mem::size_of::<GuestToHost>();
    let _ = std::mem::size_of::<HostToGuest>();

    // Host state machine
    let _ = std::mem::size_of::<HostState>();
    let _ = std::mem::size_of::<HostStateMachine>();

    // Codec functions (verify they exist as fn pointers)
    let _: fn(&GuestToHost) -> anyhow::Result<Vec<u8>> = encode_guest_msg;
    let _: fn(&HostToGuest) -> anyhow::Result<Vec<u8>> = encode_host_msg;

    // Hypervisor traits (verify they exist as trait objects)
    fn _assert_hypervisor_traits(_h: &dyn Hypervisor, _v: &dyn VmHandle, _s: &dyn SerialConsole) {}

    // AppleVzHypervisor (macOS-only)
    #[cfg(target_os = "macos")]
    {
        let h = AppleVzHypervisor;
        let _: &dyn Hypervisor = &h;
    }

    // Cleanup
    std::fs::remove_dir_all(&kernel).unwrap();
}
