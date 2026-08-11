use super::super::memory::{GuestMemory, RAM_BASE};
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
enum RecordedIoctl {
    Request(u64),
    Running(i32),
    SetBase { index: u32, base: u32 },
    GetBase { index: u32 },
}

struct RecordingIoctl {
    events: Vec<RecordedIoctl>,
    vring_bases: [u32; VHOST_VSOCK_BACKEND_QUEUES],
    fail_request: Option<u64>,
}

impl RecordingIoctl {
    fn new(vring_bases: [u32; VHOST_VSOCK_BACKEND_QUEUES]) -> Self {
        Self {
            events: Vec::new(),
            vring_bases,
            fail_request: None,
        }
    }

    fn failing(request: u64) -> Self {
        Self {
            events: Vec::new(),
            vring_bases: [0; VHOST_VSOCK_BACKEND_QUEUES],
            fail_request: Some(request),
        }
    }
}

impl VhostIoctl for RecordingIoctl {
    fn call(&mut self, _fd: RawFd, request: u64, arg: u64) -> Result<()> {
        match request {
            VHOST_GET_FEATURES => unsafe {
                *(arg as *mut u64) = u64::MAX;
                self.events.push(RecordedIoctl::Request(request));
            },
            VHOST_VSOCK_SET_RUNNING => {
                let running = unsafe { *(arg as *const libc::c_int) };
                self.events.push(RecordedIoctl::Running(running));
            }
            VHOST_SET_VRING_BASE => {
                let state = unsafe { *(arg as *const VhostVringState) };
                self.events.push(RecordedIoctl::SetBase {
                    index: state.index,
                    base: state.num,
                });
            }
            VHOST_GET_VRING_BASE => {
                let state = unsafe { &mut *(arg as *mut VhostVringState) };
                self.events
                    .push(RecordedIoctl::GetBase { index: state.index });
                state.num = self.vring_bases[state.index as usize];
            }
            _ => self.events.push(RecordedIoctl::Request(request)),
        }

        if self.fail_request == Some(request) {
            anyhow::bail!("injected ioctl failure for 0x{request:x}");
        }
        Ok(())
    }
}

fn active_dummy_device() -> VhostVsockDevice {
    let mut device = dummy_device();
    device.vhost_fd = Some(create_eventfd().unwrap());
    device.activated = true;
    device
}

fn configured_queues() -> (GuestMemory, Vec<QueueConfig>) {
    let mem = GuestMemory::new(0x20_000).unwrap();
    let queues = (0..VSOCK_NUM_QUEUES)
        .map(|index| {
            let offset = 0x1000 + index as u64 * 0x4000;
            QueueConfig {
                desc_addr: RAM_BASE + offset,
                driver_addr: RAM_BASE + offset + 0x1000,
                device_addr: RAM_BASE + offset + 0x2000,
                size: 256,
                warm_restore: true,
                event_idx: false,
            }
        })
        .collect();
    (mem, queues)
}

fn checkpoint_fixture(guest_cid: u64, bases_present: u8, vring_bases: &[u32]) -> Vec<u8> {
    let mut encoded = Vec::new();
    encoded.extend_from_slice(&VSOCK_CHECKPOINT_VERSION.to_le_bytes());
    encoded.extend_from_slice(&guest_cid.to_le_bytes());
    encoded.push(bases_present);
    for base in vring_bases {
        encoded.extend_from_slice(&base.to_le_bytes());
    }
    encoded
}

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
        checkpoint_state: None,
        restore_vring_bases: None,
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
fn vhost_backend_configures_rx_tx_only() {
    assert_eq!(VSOCK_NUM_QUEUES, 3);
    assert_eq!(VHOST_VSOCK_BACKEND_QUEUES, 2);
}

#[test]
fn kvm_vsock_port_block_stays_in_valid_port_range() {
    let max_offset =
        VSOCK_PORT_BLOCK_BASE_OFFSET + (VSOCK_PORT_BLOCK_COUNT - 1) * VSOCK_PORT_BLOCK_SIZE;
    let physical = physical_vsock_port(5007, max_offset).unwrap();

    assert!(physical <= u16::MAX as u32);
}

#[test]
fn physical_vsock_port_rejects_overflow_and_u16_exhaustion() {
    assert!(physical_vsock_port(u32::MAX, 1).is_err());
    assert!(physical_vsock_port(u16::MAX as u32, 1).is_err());
}

#[test]
fn queue_used_idx_reads_vring_used_index() {
    let mem = GuestMemory::new(0x10000).unwrap();
    let used_gpa = RAM_BASE + 0x4000;
    mem.write_at(0x4002, &37u16.to_le_bytes()).unwrap();
    let queue = QueueConfig {
        desc_addr: RAM_BASE + 0x1000,
        driver_addr: RAM_BASE + 0x2000,
        device_addr: used_gpa,
        size: 256,
        warm_restore: false,
        event_idx: false,
    };

    let idx = queue_used_idx(&mem.clone_ref(RAM_BASE), &queue).unwrap();

    assert_eq!(idx, 37);
}

#[test]
fn queue_avail_idx_reads_vring_avail_index() {
    let mem = GuestMemory::new(0x10000).unwrap();
    let avail_gpa = RAM_BASE + 0x2000;
    mem.write_at(0x2002, &91u16.to_le_bytes()).unwrap();
    let queue = QueueConfig {
        desc_addr: RAM_BASE + 0x1000,
        driver_addr: avail_gpa,
        device_addr: RAM_BASE + 0x4000,
        size: 256,
        warm_restore: false,
        event_idx: false,
    };

    let idx = queue_avail_idx(&mem.clone_ref(RAM_BASE), &queue).unwrap();

    assert_eq!(idx, 91);
}

#[test]
fn vsock_checkpoint_codec_version_tracks_cid_bound_shape() {
    assert_eq!(VSOCK_CHECKPOINT_VERSION, 2);
}

#[test]
fn vsock_checkpoint_inactive_state_is_nonempty_and_cid_bound() {
    let mut device = dummy_device();
    device.guest_cid = 41;
    let expected = checkpoint_fixture(41, 0, &[]);

    assert_eq!(device.checkpoint_state().unwrap(), expected);
    device
        .quiesce_with(&mut RecordingIoctl::new([0, 0]))
        .unwrap();
    let encoded = device.checkpoint_state().unwrap();

    assert_eq!(encoded, expected);
}

#[test]
fn vsock_checkpoint_active_state_binds_cid_and_both_bases() {
    let mut device = active_dummy_device();
    device.guest_cid = 41;
    let mut ioctl = RecordingIoctl::new([37, 91]);

    device.quiesce_with(&mut ioctl).unwrap();
    let encoded = device.checkpoint_state().unwrap();

    assert_eq!(encoded, checkpoint_fixture(41, 1, &[37, 91]));
}

#[test]
fn vsock_checkpoint_restore_rejects_wrong_cid_without_mutation() {
    let mut device = dummy_device();
    let encoded = checkpoint_fixture(42, 1, &[17, 29]);

    let error = device.restore_checkpoint_state(&encoded).unwrap_err();

    assert!(
        error.to_string().contains("CID identity mismatch"),
        "{error:#}"
    );
    assert!(device.checkpoint_state.is_none());
    assert!(device.restore_vring_bases.is_none());
}

#[test]
fn vsock_checkpoint_restore_rejects_non_boolean_bases_presence() {
    for invalid in [2, u8::MAX] {
        let mut device = dummy_device();
        let encoded = checkpoint_fixture(device.guest_cid, invalid, &[]);

        let error = device.restore_checkpoint_state(&encoded).unwrap_err();

        assert!(
            error.to_string().contains("bases-present flag"),
            "invalid={invalid}: {error:#}"
        );
        assert!(device.checkpoint_state.is_none());
        assert!(device.restore_vring_bases.is_none());
    }
}

#[test]
fn vsock_checkpoint_restore_rejects_presence_payload_mismatch() {
    let device = dummy_device();
    let malformed = [
        checkpoint_fixture(device.guest_cid, 0, &[17, 29]),
        checkpoint_fixture(device.guest_cid, 1, &[]),
    ];

    for encoded in malformed {
        let mut restored = dummy_device();
        let error = restored.restore_checkpoint_state(&encoded).unwrap_err();

        assert!(error.to_string().contains("checkpoint length"), "{error:#}");
        assert!(restored.checkpoint_state.is_none());
        assert!(restored.restore_vring_bases.is_none());
    }
}

#[test]
fn checkpoint_quiesce_stops_backend_before_capturing_vring_bases() {
    let mut device = active_dummy_device();
    let mut ioctl = RecordingIoctl::new([37, 91]);

    device.quiesce_with(&mut ioctl).unwrap();

    assert_eq!(
        ioctl.events,
        vec![
            RecordedIoctl::Running(0),
            RecordedIoctl::GetBase { index: 0 },
            RecordedIoctl::GetBase { index: 1 },
        ]
    );
    let encoded = device.checkpoint_state().unwrap();
    assert_eq!(
        decode_vsock_checkpoint(&encoded).unwrap(),
        VsockCheckpointState {
            guest_cid: device.guest_cid,
            vring_bases: Some([37, 91]),
        }
    );
}

#[test]
fn checkpoint_quiesce_fails_before_capturing_state_when_stop_fails() {
    let mut device = active_dummy_device();
    let mut ioctl = RecordingIoctl::failing(VHOST_VSOCK_SET_RUNNING);

    let error = device.quiesce_with(&mut ioctl).unwrap_err();

    assert!(error.to_string().contains("VHOST_VSOCK_SET_RUNNING=0"));
    assert_eq!(ioctl.events, vec![RecordedIoctl::Running(0)]);
    assert!(device.checkpoint_state.is_none());
}

#[test]
fn warm_restore_applies_captured_bases_before_restarting_backend() {
    let mut device = dummy_device();
    device.vhost_fd = Some(create_eventfd().unwrap());
    let state = encode_vsock_checkpoint(device.guest_cid, Some([17, 29]));
    device.restore_checkpoint_state(&state).unwrap();
    let (mem, queues) = configured_queues();
    let mut ioctl = RecordingIoctl::new([0, 0]);

    device
        .restore_activate_with(mem.clone_ref(RAM_BASE), &queues, &mut ioctl)
        .unwrap();

    let lifecycle: Vec<_> = ioctl
        .events
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                RecordedIoctl::SetBase { .. } | RecordedIoctl::Running(_)
            )
        })
        .collect();
    assert_eq!(
        lifecycle,
        vec![
            RecordedIoctl::SetBase { index: 0, base: 17 },
            RecordedIoctl::SetBase { index: 1, base: 29 },
            RecordedIoctl::Running(1),
        ]
    );
    assert!(device.activated);
}

#[test]
fn warm_restore_restart_failure_is_returned_and_device_stays_inactive() {
    let mut device = dummy_device();
    device.vhost_fd = Some(create_eventfd().unwrap());
    let state = encode_vsock_checkpoint(device.guest_cid, Some([17, 29]));
    device.restore_checkpoint_state(&state).unwrap();
    let (mem, queues) = configured_queues();
    let mut ioctl = RecordingIoctl::failing(VHOST_VSOCK_SET_RUNNING);

    let error = device
        .restore_activate_with(mem.clone_ref(RAM_BASE), &queues, &mut ioctl)
        .unwrap_err();

    assert!(format!("{error:#}").contains("VHOST_VSOCK_SET_RUNNING=1"));
    assert!(!device.activated);
    assert_eq!(ioctl.events.last(), Some(&RecordedIoctl::Running(1)));
}

#[test]
fn warm_restore_rejects_malformed_backend_state_before_any_ioctl() {
    let mut device = dummy_device();

    let error = device.restore_checkpoint_state(&[1, 0, 0]).unwrap_err();

    assert!(error.to_string().contains("vhost-vsock checkpoint"));
    assert!(!device.activated);
}

#[test]
fn warm_restore_rejects_trailing_backend_state_bytes() {
    let mut device = dummy_device();
    let mut encoded = encode_vsock_checkpoint(device.guest_cid, Some([17, 29]));
    encoded.push(0);

    let error = device.restore_checkpoint_state(&encoded).unwrap_err();

    assert!(error.to_string().contains("checkpoint length"));
    assert!(device.restore_vring_bases.is_none());
}

#[test]
fn warm_restore_requires_captured_bases_before_any_ioctl() {
    let mut device = dummy_device();
    device.vhost_fd = Some(create_eventfd().unwrap());
    let state = encode_vsock_checkpoint(device.guest_cid, None);
    device.restore_checkpoint_state(&state).unwrap();
    let (mem, queues) = configured_queues();
    let mut ioctl = RecordingIoctl::new([0, 0]);

    let error = device
        .restore_activate_with(mem.clone_ref(RAM_BASE), &queues, &mut ioctl)
        .unwrap_err();

    assert!(error
        .to_string()
        .contains("missing vhost-vsock checkpoint state"));
    assert!(ioctl.events.is_empty());
    assert!(!device.activated);
}

#[test]
fn vhost_memory_table_single_region_below_x86_pci_hole() {
    let hva = 0x1000_0000;
    let regions = build_vhost_memory_regions_from_parts(64 * 1024 * 1024, hva).unwrap();
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].guest_phys_addr, memory::RAM_BASE);
    assert_eq!(regions[0].memory_size, 64 * 1024 * 1024);
    assert_eq!(regions[0].userspace_addr, hva);
}

#[cfg(target_arch = "x86_64")]
#[test]
fn vhost_memory_table_splits_around_x86_pci_hole() {
    let hva = 0x1000_0000;
    let ram_size = memory::PCI_HOLE_START + 0x2000;
    let regions = build_vhost_memory_regions_from_parts(ram_size, hva).unwrap();
    assert_eq!(regions.len(), 2);
    assert_eq!(regions[0].guest_phys_addr, 0);
    assert_eq!(regions[0].memory_size, memory::PCI_HOLE_START);
    assert_eq!(regions[0].userspace_addr, hva);
    assert_eq!(regions[1].guest_phys_addr, memory::PCI_HOLE_END);
    assert_eq!(regions[1].memory_size, 0x2000);
    assert_eq!(regions[1].userspace_addr, hva + memory::PCI_HOLE_START);
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
        checkpoint_state: None,
        restore_vring_bases: None,
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
        checkpoint_state: None,
        restore_vring_bases: None,
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
        checkpoint_state: None,
        restore_vring_bases: None,
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
fn call_irq_bridge_sets_mmio_status_and_signals_irqfd() {
    let call_fd = create_eventfd().unwrap();
    let irq_fd = create_eventfd().unwrap();
    let irq_read_fd = unsafe { libc::dup(irq_fd.as_raw_fd()) };
    assert!(irq_read_fd >= 0);
    let irq_read_fd = unsafe { OwnedFd::from_raw_fd(irq_read_fd) };

    let interrupt_status = Arc::new(AtomicU32::new(0));
    let shutdown = Arc::new(AtomicBool::new(false));
    let handles = spawn_call_irq_bridges(
        &[call_fd.as_raw_fd()],
        vec![irq_fd],
        Arc::clone(&interrupt_status),
        Arc::clone(&shutdown),
    )
    .unwrap();

    write_eventfd(call_fd.as_raw_fd(), 1);

    for _ in 0..50 {
        if interrupt_status.load(Ordering::SeqCst) == 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(interrupt_status.load(Ordering::SeqCst), 1);
    assert_eq!(read_eventfd_retry(irq_read_fd.as_raw_fd()), 1);

    shutdown.store(true, Ordering::SeqCst);
    for handle in handles {
        handle.join().unwrap();
    }
}

fn write_eventfd(fd: RawFd, value: u64) {
    let ret = unsafe {
        libc::write(
            fd,
            &value as *const u64 as *const libc::c_void,
            std::mem::size_of::<u64>(),
        )
    };
    assert_eq!(ret, std::mem::size_of::<u64>() as isize);
}

fn read_eventfd_retry(fd: RawFd) -> u64 {
    for _ in 0..50 {
        let mut value = 0u64;
        let ret = unsafe {
            libc::read(
                fd,
                &mut value as *mut u64 as *mut libc::c_void,
                std::mem::size_of::<u64>(),
            )
        };
        if ret == std::mem::size_of::<u64>() as isize {
            return value;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("eventfd was not signaled");
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
