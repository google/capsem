//! vhost-vsock integration for KVM backend.
//!
//! Implements the virtio-vsock device (type 19) using Linux's vhost-vsock
//! kernel module for the data plane. Host uses AF_VSOCK sockets to accept
//! connections from the guest.

use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result, bail};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use super::memory::{self, GuestMemoryRef};
use super::sys::{
    self, VHOST_SET_MEM_TABLE, VHOST_SET_OWNER, VHOST_SET_VRING_ADDR, VHOST_SET_VRING_BASE,
    VHOST_SET_VRING_CALL, VHOST_SET_VRING_KICK, VHOST_SET_VRING_NUM,
    VHOST_VSOCK_SET_GUEST_CID, VhostMemoryRegion, VhostVringAddr, VhostVringFile,
    VhostVringState,
};
use super::virtio_mmio::{QueueConfig, VirtioDevice};
use crate::hypervisor::VsockConnection;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const VIRTIO_ID_VSOCK: u32 = 19;
const VIRTIO_F_VERSION_1: u64 = 1 << 32;
const VSOCK_NUM_QUEUES: usize = 3; // rx, tx, event

/// Reserved CIDs: 0 = hypervisor, 1 = reserved, 2 = host.
const MIN_GUEST_CID: u32 = 3;
/// VMADDR_CID_ANY -- not valid as a static guest CID.
const VMADDR_CID_ANY: u32 = u32::MAX;

// AF_VSOCK constants
const AF_VSOCK: i32 = 40;
const VMADDR_CID_ANY_BIND: u32 = u32::MAX; // VMADDR_CID_ANY for bind

// ---------------------------------------------------------------------------
// VhostVsockDevice
// ---------------------------------------------------------------------------

/// Virtio-vsock device backed by Linux's vhost-vsock kernel module.
///
/// Implements the VirtioDevice trait for MMIO transport discovery and
/// feature negotiation. The actual data plane runs in-kernel via
/// /dev/vhost-vsock.
pub(super) struct VhostVsockDevice {
    guest_cid: u64,
    vhost_fd: Option<OwnedFd>,
    kick_fds: [OwnedFd; VSOCK_NUM_QUEUES],
    call_fds: [OwnedFd; VSOCK_NUM_QUEUES],
    activated: bool,
}

/// Validate that a guest CID is usable.
fn validate_guest_cid(cid: u32) -> Result<()> {
    if cid < MIN_GUEST_CID {
        bail!("guest CID {cid} is reserved (must be >= {MIN_GUEST_CID})");
    }
    if cid == VMADDR_CID_ANY {
        bail!("guest CID cannot be VMADDR_CID_ANY (0xFFFFFFFF)");
    }
    Ok(())
}

/// Create an eventfd (Linux-only).
fn create_eventfd() -> Result<OwnedFd> {
    let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    if fd < 0 {
        bail!("eventfd: {}", std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

impl VhostVsockDevice {
    /// Create a new vhost-vsock device.
    ///
    /// Returns the device and the raw fds of the 3 call eventfds so the
    /// caller can wire them to KVM_IRQFD before the guest boots.
    pub fn new(guest_cid: u32, vhost_fd: OwnedFd) -> Result<(Self, [RawFd; VSOCK_NUM_QUEUES])> {
        validate_guest_cid(guest_cid)?;

        let kick_fds = [create_eventfd()?, create_eventfd()?, create_eventfd()?];
        let call_fds = [create_eventfd()?, create_eventfd()?, create_eventfd()?];

        let call_raw = [
            call_fds[0].as_raw_fd(),
            call_fds[1].as_raw_fd(),
            call_fds[2].as_raw_fd(),
        ];

        let dev = Self {
            guest_cid: guest_cid as u64,
            vhost_fd: Some(vhost_fd),
            kick_fds,
            call_fds,
            activated: false,
        };

        Ok((dev, call_raw))
    }

    /// Configure the vhost-vsock backend with queue addresses from the guest.
    fn configure_vhost(&mut self, mem: &GuestMemoryRef, queues: &[QueueConfig]) -> Result<()> {
        let vhost_fd = self
            .vhost_fd
            .as_ref()
            .context("vhost-vsock fd not available")?
            .as_raw_fd();

        // 1. Set owner
        vhost_ioctl(vhost_fd, VHOST_SET_OWNER, 0)
            .context("VHOST_SET_OWNER")?;

        // 2. Set memory table (one contiguous region: guest RAM)
        let hva = mem
            .gpa_to_host(memory::RAM_BASE)
            .context("RAM_BASE not in guest memory")? as u64;

        let region = VhostMemoryRegion {
            guest_phys_addr: memory::RAM_BASE,
            memory_size: mem.size(),
            userspace_addr: hva,
            flags_padding: 0,
        };

        // vhost_memory: nregions(u32) + padding(u32) + regions[]
        let mut mem_table = vec![0u8; 8 + std::mem::size_of::<VhostMemoryRegion>()];
        mem_table[0..4].copy_from_slice(&1u32.to_ne_bytes()); // nregions = 1
        unsafe {
            std::ptr::copy_nonoverlapping(
                &region as *const VhostMemoryRegion as *const u8,
                mem_table.as_mut_ptr().add(8),
                std::mem::size_of::<VhostMemoryRegion>(),
            );
        }
        vhost_ioctl(vhost_fd, VHOST_SET_MEM_TABLE, mem_table.as_ptr() as u64)
            .context("VHOST_SET_MEM_TABLE")?;

        // 3. Configure each vring
        for (i, queue) in queues.iter().enumerate() {
            // Set queue size
            let vring_state = VhostVringState {
                index: i as u32,
                num: queue.size as u32,
            };
            vhost_ioctl(
                vhost_fd,
                VHOST_SET_VRING_NUM,
                &vring_state as *const _ as u64,
            )
            .context("VHOST_SET_VRING_NUM")?;

            // Set base index (always 0 on fresh init)
            let vring_base = VhostVringState {
                index: i as u32,
                num: 0,
            };
            vhost_ioctl(
                vhost_fd,
                VHOST_SET_VRING_BASE,
                &vring_base as *const _ as u64,
            )
            .context("VHOST_SET_VRING_BASE")?;

            // Translate GPA -> HVA for vring addresses
            let desc_hva = mem
                .gpa_to_host(queue.desc_addr)
                .context("desc_addr GPA out of range")? as u64;
            let avail_hva = mem
                .gpa_to_host(queue.driver_addr)
                .context("driver_addr (avail) GPA out of range")? as u64;
            let used_hva = mem
                .gpa_to_host(queue.device_addr)
                .context("device_addr (used) GPA out of range")? as u64;

            let vring_addr = VhostVringAddr {
                index: i as u32,
                flags: 0,
                desc_user_addr: desc_hva,
                used_user_addr: used_hva,
                avail_user_addr: avail_hva,
                log_guest_addr: 0,
            };
            vhost_ioctl(
                vhost_fd,
                VHOST_SET_VRING_ADDR,
                &vring_addr as *const _ as u64,
            )
            .context("VHOST_SET_VRING_ADDR")?;

            // Set kick eventfd (guest -> vhost notification)
            let kick_file = VhostVringFile {
                index: i as u32,
                fd: self.kick_fds[i].as_raw_fd(),
            };
            vhost_ioctl(
                vhost_fd,
                VHOST_SET_VRING_KICK,
                &kick_file as *const _ as u64,
            )
            .context("VHOST_SET_VRING_KICK")?;

            // Set call eventfd (vhost -> guest interrupt)
            let call_file = VhostVringFile {
                index: i as u32,
                fd: self.call_fds[i].as_raw_fd(),
            };
            vhost_ioctl(
                vhost_fd,
                VHOST_SET_VRING_CALL,
                &call_file as *const _ as u64,
            )
            .context("VHOST_SET_VRING_CALL")?;
        }

        // 4. Set guest CID
        let cid = self.guest_cid;
        vhost_ioctl(
            vhost_fd,
            VHOST_VSOCK_SET_GUEST_CID,
            &cid as *const u64 as u64,
        )
        .context("VHOST_VSOCK_SET_GUEST_CID")?;

        Ok(())
    }
}

impl VirtioDevice for VhostVsockDevice {
    fn device_type(&self) -> u32 {
        VIRTIO_ID_VSOCK
    }

    fn features(&self) -> u64 {
        VIRTIO_F_VERSION_1
    }

    fn queue_max_sizes(&self) -> &[u16] {
        &[256, 256, 256]
    }

    fn read_config(&self, offset: u64, data: &mut [u8]) {
        // Config space: guest_cid as LE u64 at offset 0
        let cid_bytes = self.guest_cid.to_le_bytes();
        for (i, byte) in data.iter_mut().enumerate() {
            let config_offset = offset as usize + i;
            if config_offset < cid_bytes.len() {
                *byte = cid_bytes[config_offset];
            } else {
                *byte = 0;
            }
        }
    }

    fn write_config(&self, _offset: u64, _data: &[u8]) {
        // Config space is read-only for vsock
    }

    fn activate(&mut self, mem: GuestMemoryRef, queues: &[QueueConfig]) {
        if self.activated {
            return;
        }
        if let Err(e) = self.configure_vhost(&mem, queues) {
            warn!("vhost-vsock activate failed: {e:#}");
            return;
        }
        self.activated = true;
        info!("vhost-vsock activated (CID={})", self.guest_cid);
    }

    fn queue_notify(&mut self, queue_index: u32) {
        let idx = queue_index as usize;
        if idx >= VSOCK_NUM_QUEUES {
            return;
        }
        // Write 1 to kick eventfd to wake vhost module
        let val: u64 = 1;
        unsafe {
            libc::write(
                self.kick_fds[idx].as_raw_fd(),
                &val as *const u64 as *const libc::c_void,
                8,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Vhost ioctl helper
// ---------------------------------------------------------------------------

fn vhost_ioctl(fd: RawFd, request: u64, arg: u64) -> Result<()> {
    let ret = unsafe { libc::ioctl(fd, request as libc::c_ulong, arg) };
    if ret < 0 {
        bail!(
            "vhost ioctl 0x{:x} failed: {}",
            request,
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Open /dev/vhost-vsock
// ---------------------------------------------------------------------------

/// Open the vhost-vsock device.
pub(super) fn open_vhost_vsock() -> Result<OwnedFd> {
    let raw = unsafe {
        libc::open(
            b"/dev/vhost-vsock\0".as_ptr() as *const libc::c_char,
            libc::O_RDWR | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        bail!(
            "/dev/vhost-vsock: {} (is vhost_vsock module loaded?)",
            std::io::Error::last_os_error()
        );
    }
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

// ---------------------------------------------------------------------------
// AF_VSOCK listeners
// ---------------------------------------------------------------------------

/// sockaddr_vm for AF_VSOCK bind/connect.
#[repr(C)]
struct SockaddrVm {
    svm_family: u16,
    svm_reserved1: u16,
    svm_port: u32,
    svm_cid: u32,
    svm_flags: u8,
    svm_zero: [u8; 3],
}

/// Lifetime anchor for an accepted vsock connection socket.
struct VsockSocketAnchor(OwnedFd);
unsafe impl Send for VsockSocketAnchor {}

/// Spawn listener threads for the given vsock ports.
///
/// Each thread binds an AF_VSOCK socket, listens, and accepts connections.
/// Accepted connections are sent as `VsockConnection` via the channel.
/// Threads exit when the shutdown flag is set.
pub(super) fn spawn_vsock_listeners(
    _guest_cid: u32,
    ports: &[u32],
    tx: mpsc::UnboundedSender<VsockConnection>,
    shutdown: Arc<AtomicBool>,
) -> Vec<JoinHandle<()>> {
    let mut handles = Vec::new();

    for &port in ports {
        let tx = tx.clone();
        let shutdown = Arc::clone(&shutdown);

        let handle = thread::Builder::new()
            .name(format!("vsock-listen-{port}"))
            .spawn(move || {
                if let Err(e) = vsock_listener_loop(port, &tx, &shutdown) {
                    warn!(port, "vsock listener failed: {e:#}");
                }
            })
            .expect("failed to spawn vsock listener thread");

        handles.push(handle);
    }

    handles
}

fn vsock_listener_loop(
    port: u32,
    tx: &mpsc::UnboundedSender<VsockConnection>,
    shutdown: &AtomicBool,
) -> Result<()> {
    // Create AF_VSOCK socket
    let sock_fd = unsafe { libc::socket(AF_VSOCK, libc::SOCK_STREAM, 0) };
    if sock_fd < 0 {
        bail!(
            "socket(AF_VSOCK): {}",
            std::io::Error::last_os_error()
        );
    }
    let sock = unsafe { OwnedFd::from_raw_fd(sock_fd) };

    // Bind to VMADDR_CID_ANY (accept from any guest)
    let addr = SockaddrVm {
        svm_family: AF_VSOCK as u16,
        svm_reserved1: 0,
        svm_port: port,
        svm_cid: VMADDR_CID_ANY_BIND,
        svm_flags: 0,
        svm_zero: [0; 3],
    };
    let ret = unsafe {
        libc::bind(
            sock.as_raw_fd(),
            &addr as *const SockaddrVm as *const libc::sockaddr,
            std::mem::size_of::<SockaddrVm>() as libc::socklen_t,
        )
    };
    if ret < 0 {
        bail!(
            "bind(AF_VSOCK, port={port}): {}",
            std::io::Error::last_os_error()
        );
    }

    let ret = unsafe { libc::listen(sock.as_raw_fd(), 4) };
    if ret < 0 {
        bail!(
            "listen(AF_VSOCK, port={port}): {}",
            std::io::Error::last_os_error()
        );
    }

    info!(port, "vsock: listener ready");

    // Accept loop with poll timeout for shutdown checks
    let mut pollfd = libc::pollfd {
        fd: sock.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };

    while !shutdown.load(Ordering::Relaxed) {
        let ret = unsafe { libc::poll(&mut pollfd, 1, 200) }; // 200ms timeout
        if ret < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            bail!("poll(AF_VSOCK, port={port}): {err}");
        }
        if ret == 0 {
            continue; // timeout, check shutdown
        }

        let conn_fd = unsafe {
            libc::accept4(
                sock.as_raw_fd(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                libc::SOCK_CLOEXEC,
            )
        };
        if conn_fd < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            warn!(port, "vsock accept failed: {err}");
            continue;
        }

        debug!(port, fd = conn_fd, "vsock: accepted connection");

        let anchor = VsockSocketAnchor(unsafe { OwnedFd::from_raw_fd(conn_fd) });
        let conn = VsockConnection::new(conn_fd, port, Box::new(anchor));

        if let Err(e) = tx.send(conn) {
            warn!(port, "vsock: channel closed, stopping listener: {e}");
            break;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // CID validation
    // -----------------------------------------------------------------------

    #[test]
    fn validate_cid_zero_rejected() {
        assert!(validate_guest_cid(0).is_err());
    }

    #[test]
    fn validate_cid_one_rejected() {
        assert!(validate_guest_cid(1).is_err());
    }

    #[test]
    fn validate_cid_two_rejected() {
        // CID 2 is the host
        assert!(validate_guest_cid(2).is_err());
    }

    #[test]
    fn validate_cid_three_accepted() {
        assert!(validate_guest_cid(3).is_ok());
    }

    #[test]
    fn validate_cid_large_accepted() {
        assert!(validate_guest_cid(1000).is_ok());
    }

    #[test]
    fn validate_cid_any_rejected() {
        // VMADDR_CID_ANY (u32::MAX) is not a valid static CID
        assert!(validate_guest_cid(u32::MAX).is_err());
    }

    #[test]
    fn validate_cid_max_minus_one_accepted() {
        assert!(validate_guest_cid(u32::MAX - 1).is_ok());
    }

    // -----------------------------------------------------------------------
    // VirtioDevice trait compliance (no vhost fd needed)
    // -----------------------------------------------------------------------

    // Helper: create a device with a dummy fd for trait testing.
    // The fd is invalid but we never call activate() in these tests.
    fn dummy_device() -> VhostVsockDevice {
        VhostVsockDevice {
            guest_cid: 3,
            vhost_fd: None,
            kick_fds: [
                create_eventfd().unwrap(),
                create_eventfd().unwrap(),
                create_eventfd().unwrap(),
            ],
            call_fds: [
                create_eventfd().unwrap(),
                create_eventfd().unwrap(),
                create_eventfd().unwrap(),
            ],
            activated: false,
        }
    }

    #[test]
    fn device_type_is_vsock() {
        let dev = dummy_device();
        assert_eq!(dev.device_type(), 19);
    }

    #[test]
    fn features_version_1() {
        let dev = dummy_device();
        assert_eq!(dev.features(), 1 << 32);
    }

    #[test]
    fn queue_max_sizes_three_queues() {
        let dev = dummy_device();
        let sizes = dev.queue_max_sizes();
        assert_eq!(sizes.len(), 3);
        assert_eq!(sizes, &[256, 256, 256]);
    }

    #[test]
    fn config_space_guest_cid() {
        let dev = VhostVsockDevice {
            guest_cid: 42,
            vhost_fd: None,
            kick_fds: [
                create_eventfd().unwrap(),
                create_eventfd().unwrap(),
                create_eventfd().unwrap(),
            ],
            call_fds: [
                create_eventfd().unwrap(),
                create_eventfd().unwrap(),
                create_eventfd().unwrap(),
            ],
            activated: false,
        };
        let mut buf = [0u8; 8];
        dev.read_config(0, &mut buf);
        assert_eq!(u64::from_le_bytes(buf), 42);
    }

    #[test]
    fn config_space_partial_read() {
        let dev = VhostVsockDevice {
            guest_cid: 0x0102_0304_0506_0708,
            vhost_fd: None,
            kick_fds: [
                create_eventfd().unwrap(),
                create_eventfd().unwrap(),
                create_eventfd().unwrap(),
            ],
            call_fds: [
                create_eventfd().unwrap(),
                create_eventfd().unwrap(),
                create_eventfd().unwrap(),
            ],
            activated: false,
        };
        // Read just the first 4 bytes
        let mut buf = [0u8; 4];
        dev.read_config(0, &mut buf);
        assert_eq!(u32::from_le_bytes(buf), 0x0506_0708);
    }

    #[test]
    fn config_space_beyond_cid_returns_zeros() {
        let dev = dummy_device();
        let mut buf = [0xFFu8; 4];
        dev.read_config(8, &mut buf);
        assert_eq!(buf, [0, 0, 0, 0]);
    }

    #[test]
    fn config_space_offset_within_cid() {
        let dev = VhostVsockDevice {
            guest_cid: 0x0807_0605_0403_0201,
            vhost_fd: None,
            kick_fds: [
                create_eventfd().unwrap(),
                create_eventfd().unwrap(),
                create_eventfd().unwrap(),
            ],
            call_fds: [
                create_eventfd().unwrap(),
                create_eventfd().unwrap(),
                create_eventfd().unwrap(),
            ],
            activated: false,
        };
        let mut buf = [0u8; 2];
        dev.read_config(3, &mut buf);
        // LE bytes of 0x0807_0605_0403_0201 are [01, 02, 03, 04, 05, 06, 07, 08]
        // offset 3 -> bytes [04, 05]
        assert_eq!(buf, [0x04, 0x05]);
    }

    #[test]
    fn write_config_is_noop() {
        let dev = dummy_device();
        // Should not panic
        dev.write_config(0, &[0xFF; 8]);
        // Verify config didn't change
        let mut buf = [0u8; 8];
        dev.read_config(0, &mut buf);
        assert_eq!(u64::from_le_bytes(buf), 3); // still guest_cid=3
    }

    #[test]
    fn queue_notify_out_of_range_no_panic() {
        let mut dev = dummy_device();
        // Should silently return, not panic
        dev.queue_notify(3);
        dev.queue_notify(99);
        dev.queue_notify(u32::MAX);
    }

    #[test]
    fn queue_notify_valid_index() {
        let mut dev = dummy_device();
        // Should write to eventfd without error
        dev.queue_notify(0);
        dev.queue_notify(1);
        dev.queue_notify(2);
    }

    #[test]
    fn device_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<VhostVsockDevice>();
    }

    #[test]
    fn activate_is_idempotent() {
        let mut dev = dummy_device();
        dev.activated = true;
        // Second activate should be a no-op (no vhost_fd to fail on)
        let mem = super::super::memory::GuestMemory::new(4096).unwrap();
        dev.activate(mem.clone_ref(super::memory::RAM_BASE), &[]);
        assert!(dev.activated);
    }

    // -----------------------------------------------------------------------
    // sockaddr_vm size
    // -----------------------------------------------------------------------

    #[test]
    fn sockaddr_vm_size() {
        assert_eq!(std::mem::size_of::<SockaddrVm>(), 16);
    }

    // -----------------------------------------------------------------------
    // VsockSocketAnchor is Send
    // -----------------------------------------------------------------------

    #[test]
    fn vsock_socket_anchor_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<VsockSocketAnchor>();
    }
}
