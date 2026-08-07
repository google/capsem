use super::super::memory::{GuestMemory, RAM_BASE};
use super::super::virtio_queue::{VRING_DESC_F_NEXT, VRING_DESC_F_WRITE};
use super::*;
use std::io::{Read as IoRead, Write as IoWrite};
#[cfg(target_os = "linux")]
use std::os::fd::{FromRawFd, OwnedFd};

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

fn temp_disk(name: &str, size: usize) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("capsem-blk-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(&vec![0u8; size]).unwrap();
    path
}

fn temp_disk_with_data(name: &str, data: &[u8]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("capsem-blk-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(data).unwrap();
    path
}

// Layout constants for virtqueue in guest memory
const QUEUE_TEST_SIZE: u16 = 16;
// Descriptor table at start of RAM
const DESC_TABLE_OFFSET: u64 = 0;
// Avail ring after descriptor table (16 entries * 16 bytes each = 256)
const AVAIL_RING_OFFSET: u64 = 256;
// Used ring after avail ring (6 + 16*2 = 38, round up to 64)
const USED_RING_OFFSET: u64 = 320;
// Data area starts well past virtqueue structures
const DATA_AREA_OFFSET: u64 = 4096;

struct TestHarness {
    dev: VirtioBlockDevice,
    mem: GuestMemory,
    #[cfg(target_os = "linux")]
    _irq_fd: Option<OwnedFd>,
    #[cfg(target_os = "linux")]
    interrupt_status: Option<Arc<AtomicU32>>,
    #[cfg(target_os = "linux")]
    notify_raw_fd: Option<RawFd>,
}

impl TestHarness {
    fn new(path: &std::path::Path, read_only: bool) -> Self {
        Self::new_with_event_idx(path, read_only, false)
    }

    fn new_with_event_idx(path: &std::path::Path, read_only: bool, event_idx: bool) -> Self {
        let mem_size = 1024 * 1024; // 1MB
        let mem = GuestMemory::new(mem_size).unwrap();
        let mut dev = VirtioBlockDevice::new(path, read_only).unwrap();

        // Activate with queue config
        let queue_config = QueueConfig {
            desc_addr: RAM_BASE + DESC_TABLE_OFFSET,
            driver_addr: RAM_BASE + AVAIL_RING_OFFSET,
            device_addr: RAM_BASE + USED_RING_OFFSET,
            size: QUEUE_TEST_SIZE,
            warm_restore: false,
            event_idx,
        };
        dev.activate(mem.clone_ref(RAM_BASE), &[queue_config]);

        Self {
            dev,
            mem,
            #[cfg(target_os = "linux")]
            _irq_fd: None,
            #[cfg(target_os = "linux")]
            interrupt_status: None,
            #[cfg(target_os = "linux")]
            notify_raw_fd: None,
        }
    }

    #[cfg(target_os = "linux")]
    fn new_with_async_notify(path: &std::path::Path, read_only: bool) -> Self {
        let mem_size = 1024 * 1024; // 1MB
        let mem = GuestMemory::new(mem_size).unwrap();
        let irq_raw_fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        assert!(irq_raw_fd >= 0);
        let irq_fd = unsafe { OwnedFd::from_raw_fd(irq_raw_fd) };
        let notify_raw_fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC) };
        assert!(notify_raw_fd >= 0);
        let notify_fd = unsafe { OwnedFd::from_raw_fd(notify_raw_fd) };
        let interrupt_status = Arc::new(AtomicU32::new(0));
        let mut dev = VirtioBlockDevice::new(path, read_only)
            .unwrap()
            .with_async_notify(irq_raw_fd, Arc::clone(&interrupt_status), notify_fd);

        let queue_config = QueueConfig {
            desc_addr: RAM_BASE + DESC_TABLE_OFFSET,
            driver_addr: RAM_BASE + AVAIL_RING_OFFSET,
            device_addr: RAM_BASE + USED_RING_OFFSET,
            size: QUEUE_TEST_SIZE,
            warm_restore: false,
            event_idx: false,
        };
        dev.activate(mem.clone_ref(RAM_BASE), &[queue_config]);

        Self {
            dev,
            mem,
            _irq_fd: Some(irq_fd),
            interrupt_status: Some(interrupt_status),
            notify_raw_fd: Some(notify_raw_fd),
        }
    }

    /// Write a descriptor to the descriptor table.
    fn write_desc(&self, index: u16, addr: u64, len: u32, flags: u16, next: u16) {
        let offset = DESC_TABLE_OFFSET + (index as u64) * 16;
        let mut data = [0u8; 16];
        data[0..8].copy_from_slice(&addr.to_le_bytes());
        data[8..12].copy_from_slice(&len.to_le_bytes());
        data[12..14].copy_from_slice(&flags.to_le_bytes());
        data[14..16].copy_from_slice(&next.to_le_bytes());
        self.mem.write_at(offset, &data).unwrap();
    }

    /// Write a request header to guest memory at a given offset from RAM_BASE.
    fn write_header(&self, offset: u64, type_: u32, sector: u64) {
        let mut data = [0u8; REQ_HEADER_SIZE];
        data[0..4].copy_from_slice(&type_.to_le_bytes());
        // bytes 4-7: reserved (zero)
        data[8..16].copy_from_slice(&sector.to_le_bytes());
        self.mem.write_at(offset, &data).unwrap();
    }

    /// Make descriptors available in the avail ring.
    fn push_avail(&self, ring_index: u16, desc_head: u16, avail_idx: u16) {
        // Write ring entry
        let entry_offset = AVAIL_RING_OFFSET + 4 + (ring_index as u64) * 2;
        self.mem
            .write_at(entry_offset, &desc_head.to_le_bytes())
            .unwrap();
        // Write avail idx
        let idx_offset = AVAIL_RING_OFFSET + 2;
        self.mem
            .write_at(idx_offset, &avail_idx.to_le_bytes())
            .unwrap();
    }

    fn write_used_event(&self, used_event: u16) {
        let offset = AVAIL_RING_OFFSET + 4 + (QUEUE_TEST_SIZE as u64) * 2;
        self.mem
            .write_at(offset, &used_event.to_le_bytes())
            .unwrap();
    }

    /// Read status byte from guest memory at a given offset from RAM_BASE.
    fn read_status(&self, offset: u64) -> u8 {
        let mut buf = [0u8; 1];
        self.mem.read_at(offset, &mut buf).unwrap();
        buf[0]
    }

    /// Read bytes from guest memory at a given offset from RAM_BASE.
    fn read_bytes(&self, offset: u64, len: usize) -> Vec<u8> {
        let mut buf = vec![0u8; len];
        self.mem.read_at(offset, &mut buf).unwrap();
        buf
    }

    /// Write bytes to guest memory at a given offset from RAM_BASE.
    fn write_bytes(&self, offset: u64, data: &[u8]) {
        self.mem.write_at(offset, data).unwrap();
    }

    /// Read used ring idx.
    fn read_used_idx(&self) -> u16 {
        let mut buf = [0u8; 2];
        self.mem.read_at(USED_RING_OFFSET + 2, &mut buf).unwrap();
        u16::from_le_bytes(buf)
    }

    /// Set up a simple 3-descriptor request: header + data + status.
    /// Returns (header_offset, data_offset, status_offset) relative to RAM_BASE.
    fn setup_request(&self, type_: u32, sector: u64, data_len: u32, data_writable: bool) {
        let header_offset = DATA_AREA_OFFSET;
        let data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
        let status_offset = data_offset + data_len as u64;

        self.write_header(header_offset, type_, sector);

        // Desc 0: header (readable)
        self.write_desc(
            0,
            RAM_BASE + header_offset,
            REQ_HEADER_SIZE as u32,
            VRING_DESC_F_NEXT,
            1,
        );
        // Desc 1: data buffer
        let data_flags = if data_writable {
            VRING_DESC_F_NEXT | VRING_DESC_F_WRITE
        } else {
            VRING_DESC_F_NEXT
        };
        self.write_desc(1, RAM_BASE + data_offset, data_len, data_flags, 2);
        // Desc 2: status (writable)
        self.write_desc(2, RAM_BASE + status_offset, 1, VRING_DESC_F_WRITE, 0);

        self.push_avail(0, 0, 1);
    }
}

// -----------------------------------------------------------------------
// Category 1: Device identity and configuration
// -----------------------------------------------------------------------

#[test]
fn block_device_type() {
    let path = temp_disk("dev-type.img", 512);
    let dev = VirtioBlockDevice::new(&path, false).unwrap();
    assert_eq!(dev.device_type(), VIRTIO_ID_BLOCK);
}

#[test]
fn block_features_read_only() {
    let path = temp_disk("feat-ro.img", 512);
    let dev = VirtioBlockDevice::new(&path, true).unwrap();
    let f = dev.features();
    assert_ne!(f & VIRTIO_F_VERSION_1, 0, "must have VERSION_1");
    assert_ne!(f & VIRTIO_RING_F_EVENT_IDX, 0, "must have EVENT_IDX");
    assert_ne!(f & VIRTIO_BLK_F_RO, 0, "must have RO bit");
    assert_eq!(f & VIRTIO_BLK_F_DISCARD, 0, "RO disks must not discard");
}

#[test]
fn block_features_read_write() {
    let path = temp_disk("feat-rw.img", 512);
    let dev = VirtioBlockDevice::new(&path, false).unwrap();
    let f = dev.features();
    assert_ne!(f & VIRTIO_F_VERSION_1, 0, "must have VERSION_1");
    assert_ne!(f & VIRTIO_RING_F_EVENT_IDX, 0, "must have EVENT_IDX");
    assert_eq!(f & VIRTIO_BLK_F_RO, 0, "must NOT have RO bit");
    assert_ne!(f & VIRTIO_BLK_F_DISCARD, 0, "RW disks must support discard");
}

#[test]
fn block_has_one_queue() {
    let path = temp_disk("one-q.img", 512);
    let dev = VirtioBlockDevice::new(&path, false).unwrap();
    assert_eq!(dev.queue_max_sizes(), &[QUEUE_SIZE]);
}

#[test]
fn block_config_reports_capacity() {
    // 8192 bytes = 16 sectors
    let path = temp_disk("cap.img", 8192);
    let dev = VirtioBlockDevice::new(&path, false).unwrap();
    let mut data = [0u8; 8];
    dev.read_config(0, &mut data);
    let capacity = u64::from_le_bytes(data);
    assert_eq!(capacity, 16);
}

#[test]
fn block_config_partial_read() {
    // 16 sectors -> capacity = 16 = 0x0000_0000_0000_0010
    let path = temp_disk("cap-partial.img", 8192);
    let dev = VirtioBlockDevice::new(&path, false).unwrap();
    let mut data = [0u8; 4];
    dev.read_config(4, &mut data);
    // Upper 4 bytes of 16u64 should be all zeros
    assert_eq!(data, [0, 0, 0, 0]);
}

#[test]
fn block_config_past_capacity_returns_zero() {
    let path = temp_disk("cap-past.img", 512);
    let dev = VirtioBlockDevice::new(&path, false).unwrap();
    let mut data = [0xFFu8; 4];
    dev.read_config(80, &mut data);
    assert!(data.iter().all(|&b| b == 0));
}

#[test]
fn block_config_reports_discard_limits_for_writable_disk() {
    let path = temp_disk("discard-cfg.img", 8192);
    let dev = VirtioBlockDevice::new(&path, false).unwrap();
    let mut data = [0u8; 12];
    dev.read_config(36, &mut data);

    let max_discard_sectors = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let max_discard_seg = u32::from_le_bytes(data[4..8].try_into().unwrap());
    let discard_sector_alignment = u32::from_le_bytes(data[8..12].try_into().unwrap());

    assert_eq!(max_discard_sectors, 16);
    assert_eq!(max_discard_seg, 32);
    assert_eq!(discard_sector_alignment, 1);
}

#[test]
fn block_write_config_is_noop() {
    let path = temp_disk("cfg-noop.img", 8192);
    let dev = VirtioBlockDevice::new(&path, false).unwrap();
    let mut before = [0u8; 8];
    dev.read_config(0, &mut before);
    dev.write_config(0, &[0xFF; 8]);
    let mut after = [0u8; 8];
    dev.read_config(0, &mut after);
    assert_eq!(before, after);
}

// -----------------------------------------------------------------------
// Category 2: Construction edge cases
// -----------------------------------------------------------------------

#[test]
fn block_new_nonexistent_fails() {
    let result = VirtioBlockDevice::new(Path::new("/nonexistent/disk.img"), false);
    assert!(result.is_err());
}

#[test]
fn block_new_empty_file() {
    let path = temp_disk("empty.img", 0);
    let dev = VirtioBlockDevice::new(&path, false).unwrap();
    assert_eq!(dev.capacity_sectors, 0);
}

#[test]
fn block_new_unaligned_size() {
    // 1000 bytes -> floor(1000/512) = 1 sector
    let path = temp_disk("unaligned.img", 1000);
    let dev = VirtioBlockDevice::new(&path, false).unwrap();
    assert_eq!(dev.capacity_sectors, 1);
}

#[test]
fn block_device_id_from_filename() {
    let path = temp_disk("rootfs.img", 512);
    let dev = VirtioBlockDevice::new(&path, false).unwrap();
    let expected = b"rootfs.img";
    assert_eq!(&dev.device_id[..expected.len()], expected);
    // Rest should be zero-padded
    assert!(dev.device_id[expected.len()..].iter().all(|&b| b == 0));
}

#[test]
fn block_device_id_truncated() {
    let long_name = "a".repeat(30) + ".img";
    let path = temp_disk(&long_name, 512);
    let dev = VirtioBlockDevice::new(&path, false).unwrap();
    // Only first 20 bytes kept
    assert_eq!(&dev.device_id, &long_name.as_bytes()[..VIRTIO_BLK_ID_LEN]);
}

// -----------------------------------------------------------------------
// Category 3: Request processing
// -----------------------------------------------------------------------

#[test]
fn block_read_single_sector() {
    let mut data = vec![0u8; 512];
    for (i, byte) in data.iter_mut().enumerate().take(512) {
        *byte = (i % 256) as u8;
    }
    let path = temp_disk_with_data("read-1.img", &data);
    let mut h = TestHarness::new(&path, true);

    // Read request: type=IN, sector=0, 512 bytes writable data buffer
    h.setup_request(VIRTIO_BLK_T_IN, 0, 512, true);
    h.dev.queue_notify(0);

    let data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
    let read_back = h.read_bytes(data_offset, 512);
    assert_eq!(read_back, data);

    let status_offset = data_offset + 512;
    assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_OK);
    assert_eq!(h.read_used_idx(), 1);
}

#[test]
fn block_read_records_queue_and_request_metrics() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};

    let recorder = DebuggingRecorder::new();
    let snapshotter: Snapshotter = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let data = vec![0x42u8; 512];
    let path = temp_disk_with_data("read-metrics.img", &data);
    let mut h = TestHarness::new(&path, true);

    h.setup_request(VIRTIO_BLK_T_IN, 0, 512, true);
    assert!(h.dev.queue_notify(0));

    let snap = snapshotter.snapshot().into_vec();
    let counter_total = |name: &str| -> u64 {
        snap.iter()
            .filter_map(|(key, _, _, value)| match (key.key().name(), value) {
                (metric, DebugValue::Counter(count)) if metric == name => Some(*count),
                _ => None,
            })
            .sum()
    };
    let histogram_present = |name: &str| -> bool {
        snap.iter().any(|(key, _, _, value)| {
            key.key().name() == name && matches!(value, DebugValue::Histogram(_))
        })
    };

    assert_eq!(counter_total(METRIC_QUEUE_NOTIFICATIONS_TOTAL), 1);
    assert_eq!(counter_total(METRIC_QUEUE_DRAINS_TOTAL), 1);
    assert_eq!(counter_total(METRIC_DESCRIPTORS_DRAINED_TOTAL), 1);
    assert_eq!(counter_total(METRIC_USED_ENTRIES_TOTAL), 1);
    assert_eq!(counter_total(METRIC_INTERRUPTS_TOTAL), 1);
    assert_eq!(counter_total(METRIC_REQUESTS_TOTAL), 1);
    assert_eq!(counter_total(METRIC_REQUEST_BYTES_TOTAL), 512);
    assert!(histogram_present(METRIC_REQUEST_DURATION_MS));
    assert!(histogram_present(METRIC_QUEUE_DRAIN_DURATION_MS));
}

#[cfg(target_os = "linux")]
#[test]
fn block_io_uring_records_async_metrics() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};

    let recorder = DebuggingRecorder::new();
    let snapshotter: Snapshotter = recorder.snapshotter();
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let data = vec![0xA5u8; 512];
    let path = temp_disk_with_data("read-uring-metrics.img", &data);
    let mut h = TestHarness::new(&path, true);
    let mut file = h.dev.file.try_clone().unwrap();
    let Ok(mut uring) = BlockIoUring::new(file.as_raw_fd()) else {
        return;
    };
    let mut queue = h.dev.queue.take().unwrap();
    let mem = h.dev.mem.as_ref().unwrap().clone();

    h.setup_request(VIRTIO_BLK_T_IN, 0, 512, true);
    let result = VirtioBlockDevice::process_queue_uring(
        &mut file,
        true,
        h.dev.capacity_sectors,
        &h.dev.device_id,
        &mem,
        &mut queue,
        &mut uring,
    );
    assert_eq!(result.processed, 1);
    assert_eq!(result.submitted, 1);
    assert_eq!(result.used_entries, 0);

    uring.ring.submit_and_wait(1).unwrap();
    let completion = uring.reap_completions(&mem, &mut queue);
    assert_eq!(completion.completed, 1);
    assert_eq!(completion.used_entries, 1);

    let data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
    assert_eq!(h.read_bytes(data_offset, 512), data);
    assert_eq!(h.read_status(data_offset + 512), VIRTIO_BLK_S_OK);

    let snap = snapshotter.snapshot().into_vec();
    let counter_total = |name: &str| -> u64 {
        snap.iter()
            .filter_map(|(key, _, _, value)| match (key.key().name(), value) {
                (metric, DebugValue::Counter(count)) if metric == name => Some(*count),
                _ => None,
            })
            .sum()
    };
    let histogram_present = |name: &str| -> bool {
        snap.iter().any(|(key, _, _, value)| {
            key.key().name() == name && matches!(value, DebugValue::Histogram(_))
        })
    };

    assert_eq!(counter_total(METRIC_ASYNC_SUBMISSIONS_TOTAL), 1);
    assert_eq!(counter_total(METRIC_ASYNC_COMPLETIONS_TOTAL), 1);
    assert_eq!(counter_total(METRIC_USED_ENTRIES_TOTAL), 1);
    assert_eq!(counter_total(METRIC_INTERRUPTS_TOTAL), 1);
    assert_eq!(counter_total(METRIC_REQUESTS_TOTAL), 1);
    assert_eq!(counter_total(METRIC_REQUEST_BYTES_TOTAL), 512);
    assert!(histogram_present(METRIC_ASYNC_IN_FLIGHT));
    assert!(histogram_present(METRIC_REQUEST_DURATION_MS));
}

#[test]
fn block_read_multiple_sectors() {
    let mut data = vec![0u8; 1024]; // 2 sectors
    for (i, byte) in data.iter_mut().enumerate().take(1024) {
        *byte = ((i * 7) % 256) as u8;
    }
    let path = temp_disk_with_data("read-multi.img", &data);
    let mut h = TestHarness::new(&path, true);

    h.setup_request(VIRTIO_BLK_T_IN, 0, 1024, true);
    h.dev.queue_notify(0);

    let data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
    let read_back = h.read_bytes(data_offset, 1024);
    assert_eq!(read_back, data);

    let status_offset = data_offset + 1024;
    assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_OK);
}

#[test]
fn block_read_scattered_data_descriptors() {
    let data: Vec<u8> = (0..512).map(|i| (i % 251) as u8).collect();
    let path = temp_disk_with_data("read-scattered.img", &data);
    let mut h = TestHarness::new(&path, true);

    let header_offset = DATA_AREA_OFFSET;
    let data_a_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
    let data_b_offset = data_a_offset + 128;
    let status_offset = data_b_offset + 384;

    h.write_header(header_offset, VIRTIO_BLK_T_IN, 0);
    h.write_desc(
        0,
        RAM_BASE + header_offset,
        REQ_HEADER_SIZE as u32,
        VRING_DESC_F_NEXT,
        1,
    );
    h.write_desc(
        1,
        RAM_BASE + data_a_offset,
        128,
        VRING_DESC_F_NEXT | VRING_DESC_F_WRITE,
        2,
    );
    h.write_desc(
        2,
        RAM_BASE + data_b_offset,
        384,
        VRING_DESC_F_NEXT | VRING_DESC_F_WRITE,
        3,
    );
    h.write_desc(3, RAM_BASE + status_offset, 1, VRING_DESC_F_WRITE, 0);
    h.push_avail(0, 0, 1);

    h.dev.queue_notify(0);

    let mut read_back = h.read_bytes(data_a_offset, 128);
    read_back.extend_from_slice(&h.read_bytes(data_b_offset, 384));
    assert_eq!(read_back, data);
    assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_OK);
}

#[test]
fn block_write_single_sector() {
    let path = temp_disk("write-1.img", 512);
    let mut h = TestHarness::new(&path, false);

    // Write request: type=OUT, sector=0, 512 bytes readable data buffer
    h.setup_request(VIRTIO_BLK_T_OUT, 0, 512, false);

    // Fill the data buffer with a pattern
    let pattern: Vec<u8> = (0..512).map(|i| (i % 251) as u8).collect();
    let data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
    h.write_bytes(data_offset, &pattern);

    h.dev.queue_notify(0);

    let status_offset = data_offset + 512;
    assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_OK);

    // Verify file contents
    let mut file_data = vec![0u8; 512];
    let mut f = std::fs::File::open(&path).unwrap();
    f.read_exact(&mut file_data).unwrap();
    assert_eq!(file_data, pattern);
}

#[test]
fn block_write_scattered_data_descriptors() {
    let path = temp_disk("write-scattered.img", 512);
    let mut h = TestHarness::new(&path, false);

    let header_offset = DATA_AREA_OFFSET;
    let data_a_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
    let data_b_offset = data_a_offset + 128;
    let status_offset = data_b_offset + 384;
    let pattern: Vec<u8> = (0..512).map(|i| ((i * 3) % 251) as u8).collect();

    h.write_header(header_offset, VIRTIO_BLK_T_OUT, 0);
    h.write_bytes(data_a_offset, &pattern[..128]);
    h.write_bytes(data_b_offset, &pattern[128..]);
    h.write_desc(
        0,
        RAM_BASE + header_offset,
        REQ_HEADER_SIZE as u32,
        VRING_DESC_F_NEXT,
        1,
    );
    h.write_desc(1, RAM_BASE + data_a_offset, 128, VRING_DESC_F_NEXT, 2);
    h.write_desc(2, RAM_BASE + data_b_offset, 384, VRING_DESC_F_NEXT, 3);
    h.write_desc(3, RAM_BASE + status_offset, 1, VRING_DESC_F_WRITE, 0);
    h.push_avail(0, 0, 1);

    h.dev.queue_notify(0);

    assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_OK);
    assert_eq!(std::fs::read(&path).unwrap(), pattern);
}

#[test]
fn block_write_to_read_only_returns_ioerr() {
    let original = vec![0xABu8; 512];
    let path = temp_disk_with_data("write-ro.img", &original);
    let mut h = TestHarness::new(&path, true);

    h.setup_request(VIRTIO_BLK_T_OUT, 0, 512, false);
    let data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
    h.write_bytes(data_offset, &vec![0xCDu8; 512]);

    h.dev.queue_notify(0);

    let status_offset = data_offset + 512;
    assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_IOERR);

    // File should be unchanged
    let file_data = std::fs::read(&path).unwrap();
    assert_eq!(file_data, original);
}

#[test]
fn block_read_past_end_returns_ioerr() {
    let path = temp_disk("read-oob.img", 512); // 1 sector
    let mut h = TestHarness::new(&path, true);

    // Read sector 1 (out of bounds for a 1-sector disk)
    h.setup_request(VIRTIO_BLK_T_IN, 1, 512, true);
    h.dev.queue_notify(0);

    let status_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64 + 512;
    assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_IOERR);
}

#[test]
fn block_write_past_end_returns_ioerr() {
    let path = temp_disk("write-oob.img", 512); // 1 sector
    let mut h = TestHarness::new(&path, false);

    h.setup_request(VIRTIO_BLK_T_OUT, 1, 512, false);
    h.dev.queue_notify(0);

    let status_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64 + 512;
    assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_IOERR);
}

#[test]
fn block_get_id() {
    let path = temp_disk("getid-test.img", 512);
    let mut h = TestHarness::new(&path, false);

    h.setup_request(VIRTIO_BLK_T_GET_ID, 0, VIRTIO_BLK_ID_LEN as u32, true);
    h.dev.queue_notify(0);

    let data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
    let id_bytes = h.read_bytes(data_offset, VIRTIO_BLK_ID_LEN);
    assert_eq!(&id_bytes[..13], b"getid-test.im");

    let status_offset = data_offset + VIRTIO_BLK_ID_LEN as u64;
    assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_OK);
}

#[test]
fn block_discard_punches_range_and_reads_back_zeroes() {
    let original = vec![0xABu8; 4096];
    let path = temp_disk_with_data("discard.img", &original);
    let mut h = TestHarness::new(&path, false);

    h.setup_request(VIRTIO_BLK_T_DISCARD, 0, DISCARD_SEGMENT_SIZE as u32, false);
    let data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
    let mut segment = [0u8; DISCARD_SEGMENT_SIZE];
    segment[0..8].copy_from_slice(&1_u64.to_le_bytes());
    segment[8..12].copy_from_slice(&2_u32.to_le_bytes());
    h.write_bytes(data_offset, &segment);

    h.dev.queue_notify(0);

    let status_offset = data_offset + DISCARD_SEGMENT_SIZE as u64;
    assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_OK);

    let file_data = std::fs::read(&path).unwrap();
    assert_eq!(&file_data[..512], &original[..512]);
    assert!(file_data[512..1536].iter().all(|byte| *byte == 0));
    assert_eq!(&file_data[1536..], &original[1536..]);
}

#[test]
fn block_discard_to_read_only_returns_ioerr() {
    let path = temp_disk_with_data("discard-ro.img", &[0xABu8; 4096]);
    let mut h = TestHarness::new(&path, true);

    h.setup_request(VIRTIO_BLK_T_DISCARD, 0, DISCARD_SEGMENT_SIZE as u32, false);
    let data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
    let mut segment = [0u8; DISCARD_SEGMENT_SIZE];
    segment[0..8].copy_from_slice(&1_u64.to_le_bytes());
    segment[8..12].copy_from_slice(&2_u32.to_le_bytes());
    h.write_bytes(data_offset, &segment);

    h.dev.queue_notify(0);

    let status_offset = data_offset + DISCARD_SEGMENT_SIZE as u64;
    assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_IOERR);
}

#[test]
fn block_unknown_request_type_returns_unsupp() {
    let path = temp_disk("unsupp.img", 512);
    let mut h = TestHarness::new(&path, false);

    h.setup_request(99, 0, 512, true);
    h.dev.queue_notify(0);

    let status_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64 + 512;
    assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_UNSUPP);
}

#[test]
fn block_multiple_requests_in_batch() {
    let mut data = vec![0u8; 1024]; // 2 sectors
    for (i, byte) in data.iter_mut().enumerate().take(1024) {
        *byte = (i % 256) as u8;
    }
    let path = temp_disk_with_data("batch.img", &data);
    let mut h = TestHarness::new(&path, true);

    // Request 1: read sector 0 using descs 0-2
    let hdr1_offset = DATA_AREA_OFFSET;
    let data1_offset = hdr1_offset + REQ_HEADER_SIZE as u64;
    let status1_offset = data1_offset + 512;

    h.write_header(hdr1_offset, VIRTIO_BLK_T_IN, 0);
    h.write_desc(
        0,
        RAM_BASE + hdr1_offset,
        REQ_HEADER_SIZE as u32,
        VRING_DESC_F_NEXT,
        1,
    );
    h.write_desc(
        1,
        RAM_BASE + data1_offset,
        512,
        VRING_DESC_F_NEXT | VRING_DESC_F_WRITE,
        2,
    );
    h.write_desc(2, RAM_BASE + status1_offset, 1, VRING_DESC_F_WRITE, 0);

    // Request 2: read sector 1 using descs 3-5
    let hdr2_offset = status1_offset + 64; // gap
    let data2_offset = hdr2_offset + REQ_HEADER_SIZE as u64;
    let status2_offset = data2_offset + 512;

    h.write_header(hdr2_offset, VIRTIO_BLK_T_IN, 1);
    h.write_desc(
        3,
        RAM_BASE + hdr2_offset,
        REQ_HEADER_SIZE as u32,
        VRING_DESC_F_NEXT,
        4,
    );
    h.write_desc(
        4,
        RAM_BASE + data2_offset,
        512,
        VRING_DESC_F_NEXT | VRING_DESC_F_WRITE,
        5,
    );
    h.write_desc(5, RAM_BASE + status2_offset, 1, VRING_DESC_F_WRITE, 0);

    // Both in avail ring
    h.push_avail(0, 0, 2); // desc head 0 at ring[0], avail_idx=2
                           // Write ring entry for second request
    let entry_offset = AVAIL_RING_OFFSET + 4 + 2; // ring[1]
    h.mem.write_at(entry_offset, &3u16.to_le_bytes()).unwrap();

    h.dev.queue_notify(0);

    assert_eq!(h.read_status(status1_offset), VIRTIO_BLK_S_OK);
    assert_eq!(h.read_status(status2_offset), VIRTIO_BLK_S_OK);
    assert_eq!(h.read_bytes(data1_offset, 512), &data[0..512]);
    assert_eq!(h.read_bytes(data2_offset, 512), &data[512..1024]);
    assert_eq!(h.read_used_idx(), 2);
}

#[test]
fn block_notify_empty_queue_noop() {
    let path = temp_disk("empty-q.img", 512);
    let mut h = TestHarness::new(&path, false);
    // avail ring empty (idx=0), notify should be a no-op
    h.dev.queue_notify(0);
    assert_eq!(h.read_used_idx(), 0);
}

#[test]
fn block_event_idx_suppresses_driver_interrupt_until_used_event() {
    let disk_data = vec![0x5au8; 512];
    let path = temp_disk_with_data("event-idx-suppress.img", &disk_data);
    let mut h = TestHarness::new_with_event_idx(&path, true, true);

    h.setup_request(VIRTIO_BLK_T_IN, 0, 512, true);
    h.write_used_event(4);

    assert!(!h.dev.queue_notify(0));
    assert_eq!(
        h.read_status(DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64 + 512),
        VIRTIO_BLK_S_OK
    );
    assert_eq!(h.read_used_idx(), 1);
}

#[test]
fn block_event_idx_interrupts_when_used_event_is_crossed() {
    let disk_data = vec![0x6bu8; 512];
    let path = temp_disk_with_data("event-idx-kick.img", &disk_data);
    let mut h = TestHarness::new_with_event_idx(&path, true, true);

    h.setup_request(VIRTIO_BLK_T_IN, 0, 512, true);
    h.write_used_event(0);

    assert!(h.dev.queue_notify(0));
    assert_eq!(
        h.read_status(DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64 + 512),
        VIRTIO_BLK_S_OK
    );
    assert_eq!(h.read_used_idx(), 1);
}

#[test]
fn block_notify_wrong_queue_ignored() {
    let path = temp_disk("wrong-q.img", 512);
    let mut h = TestHarness::new(&path, false);
    h.dev.queue_notify(1); // only queue 0 exists
    h.dev.queue_notify(99);
    // no crash, no processing
}

#[cfg(target_os = "linux")]
#[test]
fn block_async_notify_drains_from_eventfd_worker() {
    let data: Vec<u8> = (0..512).map(|i| (i % 251) as u8).collect();
    let path = temp_disk_with_data("async-read.img", &data);
    let mut h = TestHarness::new_with_async_notify(&path, true);

    assert!(!h.dev.uses_mmio_interrupt());
    h.setup_request(VIRTIO_BLK_T_IN, 0, 512, true);

    write_eventfd(h.notify_raw_fd.unwrap()).unwrap();
    h.dev.quiesce().unwrap();

    let data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
    assert_eq!(h.read_bytes(data_offset, 512), data);
    assert_eq!(h.read_status(data_offset + 512), VIRTIO_BLK_S_OK);
    assert_eq!(h.interrupt_status.unwrap().load(Ordering::SeqCst), 1);
}

#[cfg(target_os = "linux")]
#[test]
fn block_async_quiesce_drains_pending_queue() {
    let path = temp_disk("async-quiesce.img", 512);
    let mut h = TestHarness::new_with_async_notify(&path, false);
    let pattern: Vec<u8> = (0..512).map(|i| ((i * 5) % 251) as u8).collect();

    h.setup_request(VIRTIO_BLK_T_OUT, 0, 512, false);
    let data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
    h.write_bytes(data_offset, &pattern);

    h.dev.quiesce().unwrap();

    assert_eq!(h.read_status(data_offset + 512), VIRTIO_BLK_S_OK);
    assert_eq!(std::fs::read(&path).unwrap(), pattern);
    assert_eq!(h.interrupt_status.unwrap().load(Ordering::SeqCst), 1);
}

#[cfg(target_os = "linux")]
#[test]
fn block_io_uring_gate_keeps_read_only_rootfs_on_sync_path() {
    std::env::remove_var("CAPSEM_KVM_BLK_IO_URING");
    assert!(
        !should_use_io_uring(true),
        "read-only rootfs should stay on the synchronous vectored path"
    );
    assert!(
        !should_use_io_uring(false),
        "io_uring should stay default-off until benchmarks prove a default gate"
    );
    std::env::set_var("CAPSEM_KVM_BLK_IO_URING", "1");
    assert!(
        should_use_io_uring(false),
        "writable scratch disks remain eligible for opt-in io_uring experiments"
    );
    std::env::remove_var("CAPSEM_KVM_BLK_IO_URING");
}

// -----------------------------------------------------------------------
// Category 4: Security / adversarial tests
// -----------------------------------------------------------------------

#[test]
fn block_sector_overflow_u64() {
    let path = temp_disk("overflow.img", 512);
    let mut h = TestHarness::new(&path, true);

    // sector * 512 would overflow u64
    h.setup_request(VIRTIO_BLK_T_IN, u64::MAX / 256, 512, true);
    h.dev.queue_notify(0);

    let status_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64 + 512;
    assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_IOERR);
}

#[test]
fn block_zero_length_data_descriptor() {
    let path = temp_disk("zero-len.img", 512);
    let mut h = TestHarness::new(&path, true);

    // Read with 0-length data buffer
    h.setup_request(VIRTIO_BLK_T_IN, 0, 0, true);
    h.dev.queue_notify(0);

    let status_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
    assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_OK);
}

#[test]
fn block_data_gpa_out_of_ram() {
    let path = temp_disk("bad-gpa.img", 512);
    let mut h = TestHarness::new(&path, true);

    let header_offset = DATA_AREA_OFFSET;
    let status_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64 + 512;

    h.write_header(header_offset, VIRTIO_BLK_T_IN, 0);

    // Desc 0: header (valid)
    h.write_desc(
        0,
        RAM_BASE + header_offset,
        REQ_HEADER_SIZE as u32,
        VRING_DESC_F_NEXT,
        1,
    );
    // Desc 1: data buffer at invalid GPA (way outside RAM)
    h.write_desc(
        1,
        0xDEAD_0000,
        512,
        VRING_DESC_F_NEXT | VRING_DESC_F_WRITE,
        2,
    );
    // Desc 2: status
    h.write_desc(2, RAM_BASE + status_offset, 1, VRING_DESC_F_WRITE, 0);

    h.push_avail(0, 0, 1);
    h.dev.queue_notify(0);

    assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_IOERR);
}

#[test]
fn block_guest_iovecs_reject_range_that_crosses_ram_end() {
    let mem = GuestMemory::new(4096).unwrap();
    let memref = mem.clone_ref(RAM_BASE);

    assert!(
        VirtioBlockDevice::guest_iovecs(&memref, &[(RAM_BASE + 4095, 2)]).is_none(),
        "zero-copy iovecs must validate the full guest range before exposing raw host pointers"
    );
}

#[test]
fn block_notify_before_activate_noop() {
    let path = temp_disk("no-activate.img", 512);
    let mut dev = VirtioBlockDevice::new(&path, false).unwrap();
    // queue_notify before activate should not crash
    dev.queue_notify(0);
}

#[test]
fn block_read_only_enforced_even_with_rw_feature() {
    // Device constructed as read-only -- writes must fail regardless
    let original = vec![0xAAu8; 512];
    let path = temp_disk_with_data("ro-enforced.img", &original);
    let mut h = TestHarness::new(&path, true);

    h.setup_request(VIRTIO_BLK_T_OUT, 0, 512, false);
    let data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
    h.write_bytes(data_offset, &vec![0xBBu8; 512]);

    h.dev.queue_notify(0);

    let status_offset = data_offset + 512;
    assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_IOERR);

    // File must be unchanged
    let file_data = std::fs::read(&path).unwrap();
    assert_eq!(file_data, original);
}
