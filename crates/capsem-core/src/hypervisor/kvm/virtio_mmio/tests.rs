use super::super::memory::{GuestMemory, RAM_BASE};
use super::*;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

#[cfg(target_arch = "x86_64")]
type QueueSnapshotMutation = fn(&mut QueueSnapshot);

struct DummyDevice {
    activated: std::sync::Arc<std::sync::atomic::AtomicBool>,
    notify_count: std::sync::Arc<std::sync::atomic::AtomicU32>,
    use_interrupt: bool,
}

impl DummyDevice {
    fn new() -> (
        Self,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
        std::sync::Arc<std::sync::atomic::AtomicU32>,
    ) {
        let activated = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let notify_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        (
            Self {
                activated: activated.clone(),
                notify_count: notify_count.clone(),
                use_interrupt: false,
            },
            activated,
            notify_count,
        )
    }
}

impl VirtioDevice for DummyDevice {
    fn device_type(&self) -> u32 {
        3
    } // console
    fn features(&self) -> u64 {
        0x0000_0001_0000_0001
    } // feature bits in both halves
    fn queue_max_sizes(&self) -> &[u16] {
        &[256, 256]
    }
    fn read_config(&self, offset: u64, data: &mut [u8]) {
        // Config space: 4 bytes of 0xAA
        for (i, b) in data.iter_mut().enumerate() {
            if offset as usize + i < 4 {
                *b = 0xAA;
            }
        }
    }
    fn write_config(&self, _offset: u64, _data: &[u8]) {}
    fn activate(&mut self, _mem: GuestMemoryRef, _queues: &[QueueConfig]) {
        self.activated.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    fn restore_activate(&mut self, mem: GuestMemoryRef, queues: &[QueueConfig]) -> anyhow::Result<()> {
        self.activate(mem, queues);
        Ok(())
    }
    fn queue_notify(&mut self, _queue_index: u32) -> bool {
        self.notify_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        true
    }
    fn uses_mmio_interrupt(&self) -> bool {
        self.use_interrupt
    }
}

fn make_transport() -> (
    VirtioMmioTransport,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    std::sync::Arc<std::sync::atomic::AtomicU32>,
) {
    let mem = GuestMemory::new(4096).unwrap();
    let (dev, activated, notify_count) = DummyDevice::new();
    let transport = VirtioMmioTransport::new(Box::new(dev), mem.clone_ref(RAM_BASE));
    (transport, activated, notify_count)
}

fn make_transport_with_interrupt() -> (
    VirtioMmioTransport,
    OwnedFd,
    std::sync::Arc<std::sync::atomic::AtomicU32>,
) {
    let mem = GuestMemory::new(4096).unwrap();
    let (mut dev, _, notify_count) = DummyDevice::new();
    dev.use_interrupt = true;
    let raw_fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    assert!(raw_fd >= 0);
    let interrupt_fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
    let read_fd = unsafe { OwnedFd::from_raw_fd(libc::dup(raw_fd)) };
    let transport = VirtioMmioTransport::new_with_interrupt(Box::new(dev), mem.clone_ref(RAM_BASE), interrupt_fd);
    (transport, read_fd, notify_count)
}

fn read_u32(dev: &dyn MmioDevice, offset: u64) -> u32 {
    let mut data = [0u8; 4];
    dev.read(offset, &mut data);
    u32::from_le_bytes(data)
}

fn write_u32(dev: &dyn MmioDevice, offset: u64, val: u32) {
    dev.write(offset, &val.to_le_bytes());
}

// -----------------------------------------------------------------------
// Identity registers
// -----------------------------------------------------------------------

#[test]
fn magic_value() {
    let (t, _, _) = make_transport();
    assert_eq!(read_u32(&t, MAGIC_VALUE), VIRTIO_MMIO_MAGIC);
}

#[test]
fn version() {
    let (t, _, _) = make_transport();
    assert_eq!(read_u32(&t, VERSION), 2);
}

#[test]
fn device_id() {
    let (t, _, _) = make_transport();
    assert_eq!(read_u32(&t, DEVICE_ID), 3);
}

#[test]
fn vendor_id() {
    let (t, _, _) = make_transport();
    assert_eq!(read_u32(&t, VENDOR_ID), CAPSEM_VENDOR_ID);
}

// -----------------------------------------------------------------------
// Feature negotiation
// -----------------------------------------------------------------------

#[test]
fn features_low_word() {
    let (t, _, _) = make_transport();
    write_u32(&t, DEVICE_FEATURES_SEL, 0);
    assert_eq!(read_u32(&t, DEVICE_FEATURES), 1);
}

#[test]
fn features_high_word() {
    let (t, _, _) = make_transport();
    write_u32(&t, DEVICE_FEATURES_SEL, 1);
    assert_eq!(read_u32(&t, DEVICE_FEATURES), 1);
}

// -----------------------------------------------------------------------
// Queue setup
// -----------------------------------------------------------------------

#[test]
fn queue_max_size() {
    let (t, _, _) = make_transport();
    write_u32(&t, QUEUE_SEL, 0);
    assert_eq!(read_u32(&t, QUEUE_NUM_MAX), 256);
}

#[test]
fn queue_invalid_index_returns_zero_max() {
    let (t, _, _) = make_transport();
    write_u32(&t, QUEUE_SEL, 99); // no such queue
    assert_eq!(read_u32(&t, QUEUE_NUM_MAX), 0);
}

#[test]
fn queue_ready_toggle() {
    let (t, _, _) = make_transport();
    write_u32(&t, QUEUE_SEL, 0);
    assert_eq!(read_u32(&t, QUEUE_READY), 0);
    write_u32(&t, QUEUE_READY, 1);
    assert_eq!(read_u32(&t, QUEUE_READY), 1);
}

// -----------------------------------------------------------------------
// Status lifecycle
// -----------------------------------------------------------------------

#[test]
fn status_starts_at_zero() {
    let (t, _, _) = make_transport();
    assert_eq!(read_u32(&t, STATUS), 0);
}

#[test]
fn status_lifecycle() {
    let (t, activated, _) = make_transport();

    // ACKNOWLEDGE
    write_u32(&t, STATUS, STATUS_ACKNOWLEDGE);
    assert_eq!(read_u32(&t, STATUS), STATUS_ACKNOWLEDGE);

    // DRIVER
    write_u32(&t, STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER);
    assert_eq!(read_u32(&t, STATUS), STATUS_ACKNOWLEDGE | STATUS_DRIVER);

    // FEATURES_OK
    write_u32(&t, STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK);

    // DRIVER_OK -> activates device
    assert!(!activated.load(std::sync::atomic::Ordering::SeqCst));
    write_u32(
        &t,
        STATUS,
        STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
    );
    assert!(activated.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn status_reset() {
    let (t, _, _) = make_transport();
    write_u32(&t, STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER);
    write_u32(&t, STATUS, 0); // reset
    assert_eq!(read_u32(&t, STATUS), 0);
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restore_rehydrates_state_and_activates_device() {
    let (t, activated, notify_count) = make_transport();
    let snapshot = VirtioMmioSnapshot {
        device_type: 3,
        device_state: Vec::new(),
        status: STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        features_sel: 1,
        driver_features: 0x1_0000_0001,
        driver_features_sel: 0,
        queue_sel: 1,
        queues: vec![
            QueueSnapshot {
                num: 16,
                ready: true,
                desc_lo: 0x100,
                desc_hi: 0,
                driver_lo: 0x300,
                driver_hi: 0,
                device_lo: 0x500,
                device_hi: 0,
            },
            QueueSnapshot {
                num: 8,
                ready: true,
                desc_lo: 0x700,
                desc_hi: 0,
                driver_lo: 0x900,
                driver_hi: 0,
                device_lo: 0xb00,
                device_hi: 0,
            },
        ],
        interrupt_status: 1,
        config_generation: 7,
        activated: true,
    };

    t.prepare_restore(&snapshot).unwrap();

    assert!(!activated.load(std::sync::atomic::Ordering::SeqCst));
    t.activate_restored().unwrap();

    assert!(activated.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(t.snapshot().unwrap(), snapshot);
    write_u32(&t, QUEUE_NOTIFY, 0);
    assert_eq!(notify_count.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restore_rejects_wrong_queue_count() {
    let (t, _, _) = make_transport();
    let snapshot = VirtioMmioSnapshot {
        device_type: 3,
        device_state: Vec::new(),
        status: 0,
        features_sel: 0,
        driver_features: 0,
        driver_features_sel: 0,
        queue_sel: 0,
        queues: Vec::new(),
        interrupt_status: 0,
        config_generation: 0,
        activated: false,
    };

    let err = t.prepare_restore(&snapshot).unwrap_err();

    assert!(err.to_string().contains("queue count mismatch"));
}

#[cfg(target_arch = "x86_64")]
fn valid_restored_snapshot() -> VirtioMmioSnapshot {
    VirtioMmioSnapshot {
        device_type: 3,
        device_state: Vec::new(),
        status: STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        features_sel: 0,
        driver_features: VIRTIO_F_VERSION_1,
        driver_features_sel: 0,
        queue_sel: 1,
        queues: vec![
            QueueSnapshot {
                num: 8,
                ready: true,
                desc_lo: 0x100,
                desc_hi: 0,
                driver_lo: 0x300,
                driver_hi: 0,
                device_lo: 0x500,
                device_hi: 0,
            },
            QueueSnapshot {
                num: 8,
                ready: true,
                desc_lo: 0x700,
                desc_hi: 0,
                driver_lo: 0x900,
                driver_hi: 0,
                device_lo: 0xb00,
                device_hi: 0,
            },
        ],
        interrupt_status: 0,
        config_generation: 0,
        activated: true,
    }
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restore_rejects_invalid_queue_selector_size_and_guest_memory_span() {
    let (transport, activated, _) = make_transport();

    let mut invalid_selector = valid_restored_snapshot();
    invalid_selector.queue_sel = 2;
    let err = transport.prepare_restore(&invalid_selector).unwrap_err();
    assert!(err.to_string().contains("selector is out of range"), "{err:#}");

    let mut zero_size = valid_restored_snapshot();
    zero_size.queues[0].num = 0;
    let err = transport.prepare_restore(&zero_size).unwrap_err();
    assert!(err.to_string().contains("zero size"), "{err:#}");

    let mut too_large = valid_restored_snapshot();
    too_large.queues[0].num = 257;
    let err = transport.prepare_restore(&too_large).unwrap_err();
    assert!(err.to_string().contains("exceeds device maximum"), "{err:#}");

    let mut outside_memory = valid_restored_snapshot();
    outside_memory.queues[0].desc_lo = 4080;
    let err = transport.prepare_restore(&outside_memory).unwrap_err();
    assert!(err.to_string().contains("outside guest memory"), "{err:#}");
    assert!(!activated.load(std::sync::atomic::Ordering::SeqCst));
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restore_rejects_driver_ok_status_without_activation() {
    let (transport, activated, _) = make_transport();
    let mut snapshot = valid_restored_snapshot();
    snapshot.activated = false;

    let err = transport.prepare_restore(&snapshot).unwrap_err();

    assert!(err.to_string().contains("DRIVER_OK"), "{err:#}");
    assert!(!activated.load(std::sync::atomic::Ordering::SeqCst));
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restore_rejects_activation_without_driver_ok_status() {
    let (transport, activated, _) = make_transport();
    let mut snapshot = valid_restored_snapshot();
    snapshot.status &= !STATUS_DRIVER_OK;

    let err = transport.prepare_restore(&snapshot).unwrap_err();

    assert!(err.to_string().contains("DRIVER_OK"), "{err:#}");
    assert!(!activated.load(std::sync::atomic::Ordering::SeqCst));
}

#[cfg(target_arch = "x86_64")]
#[test]
fn stateless_device_rejects_nonempty_checkpoint_state_before_activation() {
    let (transport, activated, _) = make_transport();
    let mut snapshot = valid_restored_snapshot();
    snapshot.device_state = b"unexpected".to_vec();

    let err = transport.prepare_restore(&snapshot).unwrap_err();

    assert!(err.to_string().contains("stateless virtio device"), "{err:#}");
    assert!(!activated.load(std::sync::atomic::Ordering::SeqCst));
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restore_rejects_wrong_device_type_before_activation() {
    let (t, activated, _) = make_transport();
    let snapshot = VirtioMmioSnapshot {
        device_type: 26,
        device_state: Vec::new(),
        status: STATUS_DRIVER_OK,
        features_sel: 0,
        driver_features: 0,
        driver_features_sel: 0,
        queue_sel: 0,
        queues: vec![QueueSnapshot::default(), QueueSnapshot::default()],
        interrupt_status: 0,
        config_generation: 0,
        activated: true,
    };

    let err = t.prepare_restore(&snapshot).unwrap_err();

    assert!(err.to_string().contains("device type mismatch"), "{err:#}");
    assert!(!activated.load(std::sync::atomic::Ordering::SeqCst));
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restore_rehydrates_device_state_before_queue_activation() {
    struct StatefulDevice {
        events: std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>,
    }

    impl VirtioDevice for StatefulDevice {
        fn device_type(&self) -> u32 {
            99
        }
        fn features(&self) -> u64 {
            VIRTIO_F_VERSION_1
        }
        fn queue_max_sizes(&self) -> &[u16] {
            &[8]
        }
        fn read_config(&self, _offset: u64, _data: &mut [u8]) {}
        fn write_config(&self, _offset: u64, _data: &[u8]) {}
        fn activate(&mut self, _mem: GuestMemoryRef, _queues: &[QueueConfig]) {
            self.events.lock().unwrap().push("activate");
        }
        fn restore_activate(&mut self, mem: GuestMemoryRef, queues: &[QueueConfig]) -> anyhow::Result<()> {
            self.activate(mem, queues);
            Ok(())
        }
        fn queue_notify(&mut self, _queue_index: u32) -> bool {
            false
        }
        fn checkpoint_state(&mut self) -> anyhow::Result<Vec<u8>> {
            Ok(b"live".to_vec())
        }
        fn restore_checkpoint_state(&mut self, state: &[u8]) -> anyhow::Result<()> {
            anyhow::ensure!(state == b"saved", "unexpected state");
            self.events.lock().unwrap().push("restore");
            Ok(())
        }
    }

    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mem = GuestMemory::new(4096).unwrap();
    let transport = VirtioMmioTransport::new(
        Box::new(StatefulDevice { events: events.clone() }),
        mem.clone_ref(RAM_BASE),
    );
    let snapshot = VirtioMmioSnapshot {
        device_type: 99,
        device_state: b"saved".to_vec(),
        status: STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        features_sel: 0,
        driver_features: VIRTIO_F_VERSION_1,
        driver_features_sel: 0,
        queue_sel: 0,
        queues: vec![QueueSnapshot {
            num: 8,
            ready: true,
            desc_lo: 0x100,
            desc_hi: 0,
            driver_lo: 0x300,
            driver_hi: 0,
            device_lo: 0x500,
            device_hi: 0,
        }],
        interrupt_status: 0,
        config_generation: 0,
        activated: true,
    };

    transport.prepare_restore(&snapshot).unwrap();
    assert_eq!(*events.lock().unwrap(), vec!["restore"]);
    transport.activate_restored().unwrap();

    assert_eq!(*events.lock().unwrap(), vec!["restore", "activate"]);
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restore_rejects_unsupported_selectors_and_feature_bits_before_activation() {
    let (transport, activated, _) = make_transport();

    let mut features_selector = valid_restored_snapshot();
    features_selector.features_sel = 2;
    let err = transport.prepare_restore(&features_selector).unwrap_err();
    assert!(err.to_string().contains("device feature selector"), "{err:#}");

    let mut driver_selector = valid_restored_snapshot();
    driver_selector.driver_features_sel = 2;
    let err = transport.prepare_restore(&driver_selector).unwrap_err();
    assert!(err.to_string().contains("driver feature selector"), "{err:#}");

    let mut unsupported = valid_restored_snapshot();
    unsupported.driver_features |= 1 << 7;
    let err = transport.prepare_restore(&unsupported).unwrap_err();
    assert!(err.to_string().contains("unsupported feature"), "{err:#}");

    let mut missing_version = valid_restored_snapshot();
    missing_version.driver_features &= !VIRTIO_F_VERSION_1;
    let err = transport.prepare_restore(&missing_version).unwrap_err();
    assert!(err.to_string().contains("VERSION_1"), "{err:#}");
    assert!(!activated.load(std::sync::atomic::Ordering::SeqCst));
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restore_rejects_unknown_or_out_of_order_status_and_interrupt_bits() {
    let (transport, activated, _) = make_transport();

    let mut unknown_status = valid_restored_snapshot();
    unknown_status.status |= 1 << 5;
    let err = transport.prepare_restore(&unknown_status).unwrap_err();
    assert!(err.to_string().contains("unknown status"), "{err:#}");

    let mut missing_driver = valid_restored_snapshot();
    missing_driver.status &= !STATUS_DRIVER;
    let err = transport.prepare_restore(&missing_driver).unwrap_err();
    assert!(err.to_string().contains("status dependency"), "{err:#}");

    let mut invalid_interrupt = valid_restored_snapshot();
    invalid_interrupt.interrupt_status = 4;
    let err = transport.prepare_restore(&invalid_interrupt).unwrap_err();
    assert!(err.to_string().contains("interrupt status"), "{err:#}");
    assert!(!activated.load(std::sync::atomic::Ordering::SeqCst));
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restore_rejects_ready_queue_shape_alignment_and_overlap() {
    let (transport, activated, _) = make_transport();

    let mut non_power_of_two = valid_restored_snapshot();
    non_power_of_two.queues[0].num = 7;
    let err = transport.prepare_restore(&non_power_of_two).unwrap_err();
    assert!(err.to_string().contains("power of two"), "{err:#}");

    let mut not_ready = valid_restored_snapshot();
    not_ready.queues[0].ready = false;
    let err = transport.prepare_restore(&not_ready).unwrap_err();
    assert!(err.to_string().contains("is not ready"), "{err:#}");

    let alignment_mutations: &[(&str, QueueSnapshotMutation)] = &[
        ("descriptor", |queue: &mut QueueSnapshot| queue.desc_lo += 1),
        ("available ring", |queue: &mut QueueSnapshot| queue.driver_lo += 1),
        ("used ring", |queue: &mut QueueSnapshot| queue.device_lo += 2),
    ];
    for (field, mutate) in alignment_mutations {
        let mut unaligned = valid_restored_snapshot();
        mutate(&mut unaligned.queues[0]);
        let err = transport.prepare_restore(&unaligned).unwrap_err();
        assert!(err.to_string().contains(field), "{err:#}");
        assert!(err.to_string().contains("aligned"), "{err:#}");
    }

    let mut within_queue_overlap = valid_restored_snapshot();
    within_queue_overlap.queues[0].driver_lo = within_queue_overlap.queues[0].desc_lo;
    let err = transport.prepare_restore(&within_queue_overlap).unwrap_err();
    assert!(err.to_string().contains("overlap"), "{err:#}");

    let mut overlap = valid_restored_snapshot();
    overlap.queues[1].desc_lo = overlap.queues[0].desc_lo;
    let err = transport.prepare_restore(&overlap).unwrap_err();
    assert!(err.to_string().contains("overlap"), "{err:#}");
    assert!(!activated.load(std::sync::atomic::Ordering::SeqCst));
}

#[cfg(target_arch = "x86_64")]
#[test]
fn restored_activation_error_is_propagated() {
    struct FailingDevice;

    impl VirtioDevice for FailingDevice {
        fn device_type(&self) -> u32 {
            99
        }
        fn features(&self) -> u64 {
            VIRTIO_F_VERSION_1
        }
        fn queue_max_sizes(&self) -> &[u16] {
            &[8]
        }
        fn read_config(&self, _offset: u64, _data: &mut [u8]) {}
        fn write_config(&self, _offset: u64, _data: &[u8]) {}
        fn activate(&mut self, _mem: GuestMemoryRef, _queues: &[QueueConfig]) {}
        fn restore_activate(&mut self, _mem: GuestMemoryRef, _queues: &[QueueConfig]) -> anyhow::Result<()> {
            anyhow::bail!("backend reconstruction failed")
        }
        fn queue_notify(&mut self, _queue_index: u32) -> bool {
            false
        }
    }

    let mem = GuestMemory::new(4096).unwrap();
    let transport = VirtioMmioTransport::new(Box::new(FailingDevice), mem.clone_ref(RAM_BASE));
    let mut snapshot = valid_restored_snapshot();
    snapshot.device_type = 99;
    snapshot.queues.truncate(1);
    snapshot.queue_sel = 0;

    transport.prepare_restore(&snapshot).unwrap();
    let err = transport.activate_restored().unwrap_err();

    assert!(err.to_string().contains("backend reconstruction failed"), "{err:#}");
}

// -----------------------------------------------------------------------
// Queue notify
// -----------------------------------------------------------------------

#[test]
fn queue_notify_before_activation_ignored() {
    let (t, _, notify_count) = make_transport();
    write_u32(&t, QUEUE_NOTIFY, 0);
    assert_eq!(notify_count.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[test]
fn queue_notify_after_activation() {
    let (t, _, notify_count) = make_transport();
    // Activate
    write_u32(
        &t,
        STATUS,
        STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
    );
    // Notify
    write_u32(&t, QUEUE_NOTIFY, 0);
    assert_eq!(notify_count.load(std::sync::atomic::Ordering::SeqCst), 1);
}

// -----------------------------------------------------------------------
// Interrupt status
// -----------------------------------------------------------------------

#[test]
fn interrupt_ack_clears_bits() {
    let (t, _, _) = make_transport();
    // Manually set interrupt_status via internal state isn't possible from outside,
    // but we can verify ACK clears bits that were already 0
    write_u32(&t, INTERRUPT_ACK, 0x1);
    assert_eq!(read_u32(&t, INTERRUPT_STATUS), 0);
}

#[test]
fn queue_notify_raises_interrupt_for_mmio_interrupt_device() {
    let (t, interrupt_fd, notify_count) = make_transport_with_interrupt();
    write_u32(
        &t,
        STATUS,
        STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
    );

    write_u32(&t, QUEUE_NOTIFY, 0);

    assert_eq!(notify_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(read_u32(&t, INTERRUPT_STATUS), 1);
    let mut count = 0u64;
    let ret = unsafe {
        libc::read(
            interrupt_fd.as_raw_fd(),
            &mut count as *mut _ as *mut libc::c_void,
            std::mem::size_of::<u64>(),
        )
    };
    assert_eq!(ret as usize, std::mem::size_of::<u64>());
    assert_eq!(count, 1);
}

#[test]
fn interrupt_status_can_be_shared_with_async_device() {
    let status = Arc::new(AtomicU32::new(0));
    let raw_fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    assert!(raw_fd >= 0);
    let write_fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
    let read_fd = unsafe { OwnedFd::from_raw_fd(libc::dup(raw_fd)) };
    let mem = GuestMemory::new(4096).unwrap();
    let (dev, _, _) = DummyDevice::new();
    let transport = VirtioMmioTransport::new_with_interrupt_status(
        Box::new(dev),
        mem.clone_ref(RAM_BASE),
        write_fd,
        Arc::clone(&status),
    );

    status.fetch_or(1, Ordering::SeqCst);
    assert_eq!(read_u32(&transport, INTERRUPT_STATUS), 1);

    write_u32(&transport, INTERRUPT_ACK, 1);
    assert_eq!(status.load(Ordering::SeqCst), 0);
    drop(read_fd);
}

// -----------------------------------------------------------------------
// Config space
// -----------------------------------------------------------------------

#[test]
fn config_space_read() {
    let (t, _, _) = make_transport();
    let mut data = [0u8; 4];
    t.read(CONFIG_SPACE, &mut data);
    assert_eq!(data, [0xAA, 0xAA, 0xAA, 0xAA]);
}

#[test]
fn config_space_read_past_end() {
    let (t, _, _) = make_transport();
    let mut data = [0u8; 4];
    t.read(CONFIG_SPACE + 100, &mut data);
    // DummyDevice returns 0 for offsets >= 4
    assert_eq!(data, [0, 0, 0, 0]);
}

// -----------------------------------------------------------------------
// Queue address setup
// -----------------------------------------------------------------------

#[test]
fn queue_descriptor_address() {
    let (t, _, _) = make_transport();
    write_u32(&t, QUEUE_SEL, 0);
    write_u32(&t, QUEUE_DESC_LOW, 0x1000);
    write_u32(&t, QUEUE_DESC_HIGH, 0x0001);

    // The address is stored internally (we can't read it back via MMIO,
    // but we verify no panic on write)
}

// -----------------------------------------------------------------------
// Unknown register
// -----------------------------------------------------------------------

#[test]
fn read_unknown_register_returns_zero() {
    let (t, _, _) = make_transport();
    assert_eq!(read_u32(&t, 0x048), 0); // undefined register
}

#[test]
fn write_to_read_only_register_ignored() {
    let (t, _, _) = make_transport();
    write_u32(&t, MAGIC_VALUE, 0xDEAD); // magic is read-only
    assert_eq!(read_u32(&t, MAGIC_VALUE), VIRTIO_MMIO_MAGIC); // unchanged
}

#[test]
fn cold_activation_rejects_oversized_queue() {
    use std::sync::atomic::Ordering;
    let (t, activated, _) = make_transport();
    // Guest declares a queue larger than the device maximum (256) on the cold
    // DRIVER_OK path. The device must refuse to activate rather than hand the
    // unclamped size to the ring accessors.
    write_u32(&t, QUEUE_SEL, 0);
    write_u32(&t, QUEUE_NUM, 512);
    write_u32(&t, QUEUE_READY, 1);
    write_u32(&t, STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK);
    assert!(
        !activated.load(Ordering::SeqCst),
        "device must not activate with a guest queue size exceeding the maximum"
    );
}

#[test]
fn cold_activation_accepts_a_valid_ready_queue() {
    use std::sync::atomic::Ordering;
    let (t, activated, _) = make_transport();
    // A well-formed queue (power-of-two size within max, rings inside RAM) must
    // still activate -- the validation must not break real guest boots.
    write_u32(&t, QUEUE_SEL, 0);
    write_u32(&t, QUEUE_NUM, 8);
    write_u32(&t, QUEUE_DESC_LOW, (RAM_BASE) as u32);
    write_u32(&t, QUEUE_DESC_HIGH, (RAM_BASE >> 32) as u32);
    write_u32(&t, QUEUE_DRIVER_LOW, (RAM_BASE + 256) as u32);
    write_u32(&t, QUEUE_DRIVER_HIGH, (RAM_BASE >> 32) as u32);
    write_u32(&t, QUEUE_DEVICE_LOW, (RAM_BASE + 512) as u32);
    write_u32(&t, QUEUE_DEVICE_HIGH, (RAM_BASE >> 32) as u32);
    write_u32(&t, QUEUE_READY, 1);
    write_u32(&t, STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK);
    assert!(
        activated.load(Ordering::SeqCst),
        "a valid ready queue must still activate the device"
    );
}
