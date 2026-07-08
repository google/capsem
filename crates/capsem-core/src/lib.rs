pub mod asset_manager;
pub mod auto_snapshot;
pub mod credential_broker;
pub mod fs_monitor;
pub mod host_state;
pub mod hypervisor;
pub mod ipc_handshake;
pub mod log_layer;
pub mod poll;
#[macro_use]
pub mod macros;
pub mod manifest_compat;
pub mod mcp;
pub mod net;
pub mod paths;
pub mod security_engine;
pub mod session;
pub mod telemetry;
#[cfg(test)]
pub(crate) mod test_support;
pub mod uds;
pub mod vm;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub use capsem_proto;
pub use capsem_proto::{
    decode_guest_msg, decode_host_msg, encode_guest_msg, encode_host_msg, GuestToHost, HostToGuest,
    MAX_FRAME_SIZE,
};
pub use host_state::{
    validate_guest_msg, validate_host_msg, HostState, HostStateMachine, StateMachine, Transition,
};
pub use vm::boot::{
    boot_vm, create_net_state, create_net_state_with_policy, read_control_msg, send_boot_config,
    write_control_msg, BootOptions,
};
pub use vm::config::{VirtioFsShare, VmConfig};
pub use vm::registry::{SandboxInstance, SandboxNetworkState};
pub use vm::terminal::TerminalOutputQueue;
pub use vm::vsock::{
    self, CoalesceBuffer, VSOCK_PORT_CONTROL, VSOCK_PORT_EXEC, VSOCK_PORT_LIFECYCLE,
    VSOCK_PORT_SNI_PROXY, VSOCK_PORT_TERMINAL,
};
pub use vm::VmState;

// Hypervisor abstraction layer
pub use hypervisor::{Hypervisor, SerialConsole, VmHandle, VsockConnection};

#[cfg(target_os = "macos")]
pub use hypervisor::apple_vz::{is_main_thread, AppleVzHypervisor};

#[cfg(target_os = "linux")]
pub use hypervisor::kvm::KvmHypervisor;

/// Create VirtioFS session directories for the single-share hybrid architecture.
///
/// The session_dir has two zones:
/// - `guest/` -- shared with the VM via VirtioFS (only this subtree is exposed)
///   - `system/rootfs.img` -- sparse ext4 image for the overlayfs upper.
///     Attached to the guest as a virtio-blk device (`/dev/vdb`); never
///     accessed from the guest through the VirtioFS share. Sits in the
///     share so the host can introspect it while the VM is stopped.
///   - `workspace/`        -- direct host-visible files for /root (AI workspace)
/// - Host-only (NOT shared with guest):
///   - `auto_snapshots/`   -- rolling ring buffer for host-side APFS clone snapshots
///   - `session.db`        -- telemetry database
///   - `serial.log`        -- terminal output log
///   - `checkpoint.vzsave` -- suspend checkpoint
///
/// The host creates a sparse `rootfs.img`. Linux VM launch clones a cached,
/// preformatted ext4 template before boot when `mke2fs` is available; the
/// guest keeps a first-boot formatting fallback for restored or
/// externally-created unformatted images. Forked sessions already have a
/// formatted image (cloned from snapshot).
pub fn create_virtiofs_session(session_dir: &Path, system_img_size_gb: u32) -> std::io::Result<()> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let guest_dir = session_dir.join("guest");
    std::fs::create_dir_all(guest_dir.join("system"))?;
    std::fs::create_dir_all(guest_dir.join("workspace"))?;
    std::fs::create_dir_all(session_dir.join("auto_snapshots"))?;

    // Create compat symlinks so existing code using session_dir/workspace and
    // session_dir/system still works. The real dirs live inside guest/ which
    // is the only subtree shared with the VM via VirtioFS.
    for name in &["system", "workspace"] {
        let link = session_dir.join(name);
        let target = std::path::Path::new("guest").join(name);
        if !link.exists() {
            std::os::unix::fs::symlink(&target, &link)?;
        }
    }

    let img_path = guest_dir.join("system").join("rootfs.img");
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

fn system_overlay_image_len(size_gb: u32) -> u64 {
    size_gb as u64 * 1024 * 1024 * 1024
}

/// Return true when a disk image already contains an ext4 superblock.
pub fn system_overlay_has_ext4_magic(path: &Path) -> std::io::Result<bool> {
    let mut file = std::fs::File::open(path)?;
    let mut magic = [0_u8; 2];
    file.seek(SeekFrom::Start(1080))?;
    match file.read_exact(&mut magic) {
        Ok(()) => Ok(magic == [0x53, 0xef]),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e),
    }
}

#[cfg(target_os = "linux")]
fn system_overlay_matches(path: &Path, size_gb: u32) -> std::io::Result<bool> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() == system_overlay_image_len(size_gb) => {
            system_overlay_has_ext4_magic(path)
        }
        Ok(_) => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Format the system overlay image as ext4 on Linux before guest boot.
///
/// This keeps first-boot overlay setup out of the VM boot critical path while
/// preserving the guest-side fallback for images created on hosts without
/// e2fsprogs.
#[cfg(target_os = "linux")]
pub fn preformat_system_overlay_image_if_needed(path: &Path) -> std::io::Result<bool> {
    if system_overlay_has_ext4_magic(path)? {
        return Ok(false);
    }

    let mke2fs = if Path::new("/usr/sbin/mke2fs").exists() {
        "/usr/sbin/mke2fs"
    } else {
        "mke2fs"
    };
    let output = std::process::Command::new(mke2fs)
        .arg("-F")
        .arg("-t")
        .arg("ext4")
        .arg("-m")
        .arg("0")
        .arg("-E")
        .arg("lazy_itable_init=1,lazy_journal_init=1")
        .arg("-J")
        .arg("size=4")
        .arg("-L")
        .arg("system")
        .arg("-q")
        .arg(path)
        .output()?;

    if output.status.success() {
        return Ok(true);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(std::io::Error::other(format!(
        "mke2fs failed for {}: status={} stderr={}",
        path.display(),
        output.status,
        stderr.trim()
    )))
}

#[cfg(not(target_os = "linux"))]
pub fn preformat_system_overlay_image_if_needed(_path: &Path) -> std::io::Result<bool> {
    Ok(false)
}

/// Return the run-dir cache path for preformatted system overlay templates.
pub fn system_overlay_template_path(run_dir: &Path, size_gb: u32) -> PathBuf {
    run_dir
        .join("system-overlays")
        .join(format!("rootfs-ext4-{size_gb}g.img"))
}

/// Derive the shared run-dir template path from a VM session directory.
pub fn system_overlay_template_path_for_session(session_dir: &Path, size_gb: u32) -> PathBuf {
    let run_dir = session_dir
        .parent()
        .filter(|parent| {
            matches!(
                parent.file_name().and_then(|name| name.to_str()),
                Some("sessions" | "persistent")
            )
        })
        .and_then(Path::parent)
        .unwrap_or_else(|| session_dir.parent().unwrap_or(session_dir));
    system_overlay_template_path(run_dir, size_gb)
}

/// Ensure a reusable preformatted system overlay template exists.
#[cfg(target_os = "linux")]
pub fn ensure_preformatted_system_overlay_template(
    template_path: &Path,
    size_gb: u32,
) -> std::io::Result<bool> {
    if system_overlay_matches(template_path, size_gb)? {
        return Ok(false);
    }

    if let Some(parent) = template_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let tmp_path = template_path.with_extension(format!(
        "tmp.{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_file(&tmp_path);
    create_scratch_disk(&tmp_path, size_gb)?;
    preformat_system_overlay_image_if_needed(&tmp_path)?;

    if system_overlay_matches(template_path, size_gb)? {
        let _ = std::fs::remove_file(&tmp_path);
        return Ok(false);
    }

    std::fs::rename(&tmp_path, template_path)?;
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
pub fn ensure_preformatted_system_overlay_template(
    _template_path: &Path,
    _size_gb: u32,
) -> std::io::Result<bool> {
    Ok(false)
}

/// Clone a cached preformatted system overlay into a session image.
#[cfg(target_os = "linux")]
pub fn preformat_system_overlay_image_from_template_if_needed(
    path: &Path,
    template_path: &Path,
    size_gb: u32,
) -> std::io::Result<bool> {
    if system_overlay_matches(path, size_gb)? {
        return Ok(false);
    }

    ensure_preformatted_system_overlay_template(template_path, size_gb)?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension(format!(
        "tmp.{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_file(&tmp_path);
    auto_snapshot::clone_file(template_path, &tmp_path)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    std::fs::rename(&tmp_path, path)?;

    if !system_overlay_matches(path, size_gb)? {
        return Err(std::io::Error::other(format!(
            "cloned system overlay image {} does not match {} GiB ext4 template {}",
            path.display(),
            size_gb,
            template_path.display()
        )));
    }

    Ok(true)
}

#[cfg(not(target_os = "linux"))]
pub fn preformat_system_overlay_image_from_template_if_needed(
    path: &Path,
    _template_path: &Path,
    _size_gb: u32,
) -> std::io::Result<bool> {
    preformat_system_overlay_image_if_needed(path)
}

/// Return the guest-visible VirtioFS share path within a session directory.
pub fn guest_share_dir(session_dir: &Path) -> std::path::PathBuf {
    session_dir.join("guest")
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
    file.set_len(system_overlay_image_len(size_gb))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(
            meta.blocks() < 1024,
            "file should be sparse, blocks={}",
            meta.blocks()
        );
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
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            2 * 1024 * 1024 * 1024
        );

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
        let first_cloned =
            preformat_system_overlay_image_from_template_if_needed(&first, &template, 1).unwrap();
        let second_cloned =
            preformat_system_overlay_image_from_template_if_needed(&second, &template, 1).unwrap();
        let first_reused =
            preformat_system_overlay_image_from_template_if_needed(&first, &template, 1).unwrap();

        assert!(created);
        assert!(!reused);
        assert!(first_cloned);
        assert!(second_cloned);
        assert!(!first_reused);
        assert!(system_overlay_has_ext4_magic(&first).unwrap());
        assert!(system_overlay_has_ext4_magic(&second).unwrap());
        assert_eq!(std::fs::metadata(&first).unwrap().len(), 1024 * 1024 * 1024);
        assert_eq!(
            std::fs::metadata(&second).unwrap().len(),
            1024 * 1024 * 1024
        );
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

        // Hypervisor traits (verify they exist as trait objects)
        fn _assert_hypervisor_traits(
            _h: &dyn Hypervisor,
            _v: &dyn VmHandle,
            _s: &dyn SerialConsole,
        ) {
        }

        // AppleVzHypervisor (macOS-only)
        #[cfg(target_os = "macos")]
        {
            let h = AppleVzHypervisor;
            let _: &dyn Hypervisor = &h;
        }

        // Cleanup
        std::fs::remove_dir_all(&kernel).unwrap();
    }
}
