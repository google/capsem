//! Virtio block device (type 2) for disk I/O.
//!
//! File-backed block device with one requestq. Supports read, write,
//! and get-ID operations. Read-only mode enforced via feature bit
//! and write rejection.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{Result, Context};

use super::memory::GuestMemoryRef;
use super::virtio_mmio::{QueueConfig, VirtioDevice};
use super::virtio_queue::VirtQueue;

/// Virtio block device ID.
const VIRTIO_ID_BLOCK: u32 = 2;

/// Maximum queue size for the requestq.
const QUEUE_SIZE: u16 = 256;

/// Sector size in bytes.
const SECTOR_SIZE: u64 = 512;

/// Maximum device ID length (virtio spec).
const VIRTIO_BLK_ID_LEN: usize = 20;

// Feature bits
const VIRTIO_BLK_F_RO: u64 = 1 << 5;
const VIRTIO_F_VERSION_1: u64 = 1 << 32;

// Request types
const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;
const VIRTIO_BLK_T_GET_ID: u32 = 8;

// Status bytes
const VIRTIO_BLK_S_OK: u8 = 0;
const VIRTIO_BLK_S_IOERR: u8 = 1;
const VIRTIO_BLK_S_UNSUPP: u8 = 2;

// Request header size: type(u32) + reserved(u32) + sector(u64) = 16 bytes
const REQ_HEADER_SIZE: usize = 16;

/// Virtio block device backed by a file.
pub(super) struct VirtioBlockDevice {
    file: std::fs::File,
    read_only: bool,
    capacity_sectors: u64,
    device_id: [u8; VIRTIO_BLK_ID_LEN],
    queue: Option<VirtQueue>,
    mem: Option<GuestMemoryRef>,
}

impl VirtioBlockDevice {
    /// Create a new virtio block device backed by a file.
    ///
    /// If `read_only` is true, the file is opened read-only and
    /// VIRTIO_BLK_F_RO is advertised. Writes are rejected.
    pub fn new(path: &Path, read_only: bool) -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(!read_only)
            .open(path)
            .with_context(|| format!("open block device: {}", path.display()))?;

        let file_size = file.metadata()
            .with_context(|| format!("stat block device: {}", path.display()))?
            .len();
        let capacity_sectors = file_size / SECTOR_SIZE;

        let mut device_id = [0u8; VIRTIO_BLK_ID_LEN];
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            let bytes = name.as_bytes();
            let len = bytes.len().min(VIRTIO_BLK_ID_LEN);
            device_id[..len].copy_from_slice(&bytes[..len]);
        }

        Ok(Self {
            file,
            read_only,
            capacity_sectors,
            device_id,
            queue: None,
            mem: None,
        })
    }

    /// Process a read request: file -> guest memory.
    fn process_read(
        &mut self,
        sector: u64,
        data_descs: &[(u64, u32)], // (gpa, len) pairs
    ) -> u8 {
        let mem = match self.mem.as_ref() {
            Some(m) => m,
            None => return VIRTIO_BLK_S_IOERR,
        };

        let offset = match sector.checked_mul(SECTOR_SIZE) {
            Some(o) => o,
            None => return VIRTIO_BLK_S_IOERR,
        };

        let total_len: u64 = data_descs.iter().map(|&(_, l)| l as u64).sum();
        if offset.checked_add(total_len).map_or(true, |end| end > self.capacity_sectors * SECTOR_SIZE) {
            return VIRTIO_BLK_S_IOERR;
        }

        if self.file.seek(SeekFrom::Start(offset)).is_err() {
            return VIRTIO_BLK_S_IOERR;
        }

        for &(gpa, len) in data_descs {
            if len == 0 {
                continue;
            }
            let host_ptr = match mem.gpa_to_host(gpa) {
                Some(p) => p,
                None => return VIRTIO_BLK_S_IOERR,
            };
            let buf = unsafe { std::slice::from_raw_parts_mut(host_ptr, len as usize) };
            if self.file.read_exact(buf).is_err() {
                return VIRTIO_BLK_S_IOERR;
            }
        }

        VIRTIO_BLK_S_OK
    }

    /// Process a write request: guest memory -> file.
    fn process_write(
        &mut self,
        sector: u64,
        data_descs: &[(u64, u32)],
    ) -> u8 {
        if self.read_only {
            return VIRTIO_BLK_S_IOERR;
        }

        let mem = match self.mem.as_ref() {
            Some(m) => m,
            None => return VIRTIO_BLK_S_IOERR,
        };

        let offset = match sector.checked_mul(SECTOR_SIZE) {
            Some(o) => o,
            None => return VIRTIO_BLK_S_IOERR,
        };

        let total_len: u64 = data_descs.iter().map(|&(_, l)| l as u64).sum();
        if offset.checked_add(total_len).map_or(true, |end| end > self.capacity_sectors * SECTOR_SIZE) {
            return VIRTIO_BLK_S_IOERR;
        }

        if self.file.seek(SeekFrom::Start(offset)).is_err() {
            return VIRTIO_BLK_S_IOERR;
        }

        for &(gpa, len) in data_descs {
            if len == 0 {
                continue;
            }
            let host_ptr = match mem.gpa_to_host(gpa) {
                Some(p) => p,
                None => return VIRTIO_BLK_S_IOERR,
            };
            let buf = unsafe { std::slice::from_raw_parts(host_ptr, len as usize) };
            if self.file.write_all(buf).is_err() {
                return VIRTIO_BLK_S_IOERR;
            }
        }

        VIRTIO_BLK_S_OK
    }

    /// Process a get-ID request: copy device_id to guest buffer.
    fn process_get_id(
        &self,
        data_descs: &[(u64, u32)],
    ) -> u8 {
        let mem = match self.mem.as_ref() {
            Some(m) => m,
            None => return VIRTIO_BLK_S_IOERR,
        };

        if let Some(&(gpa, len)) = data_descs.first() {
            if let Some(host_ptr) = mem.gpa_to_host(gpa) {
                let copy_len = (len as usize).min(VIRTIO_BLK_ID_LEN);
                let buf = unsafe { std::slice::from_raw_parts_mut(host_ptr, copy_len) };
                buf.copy_from_slice(&self.device_id[..copy_len]);
            }
        }

        VIRTIO_BLK_S_OK
    }

    /// Write a status byte to a guest physical address.
    fn write_status(&self, gpa: u64, status: u8) {
        if let Some(mem) = self.mem.as_ref() {
            if let Some(ptr) = mem.gpa_to_host(gpa) {
                unsafe { *ptr = status; }
            }
        }
    }

    /// Parse a request header from guest memory.
    /// Returns (type, sector) or None if the read fails.
    fn parse_header(&self, gpa: u64, len: u32) -> Option<(u32, u64)> {
        if (len as usize) < REQ_HEADER_SIZE {
            return None;
        }
        let mem = self.mem.as_ref()?;
        let ptr = mem.gpa_to_host(gpa)?;
        unsafe {
            let type_ = u32::from_le(*(ptr as *const u32));
            // skip 4 bytes reserved
            let sector = u64::from_le(*((ptr as *const u8).add(8) as *const u64));
            Some((type_, sector))
        }
    }
}

impl VirtioDevice for VirtioBlockDevice {
    fn device_type(&self) -> u32 {
        VIRTIO_ID_BLOCK
    }

    fn features(&self) -> u64 {
        let mut f = VIRTIO_F_VERSION_1;
        if self.read_only {
            f |= VIRTIO_BLK_F_RO;
        }
        f
    }

    fn queue_max_sizes(&self) -> &[u16] {
        &[QUEUE_SIZE]
    }

    fn read_config(&self, offset: u64, data: &mut [u8]) {
        // Config space: u64 capacity at offset 0, zeros beyond
        let capacity_bytes = self.capacity_sectors.to_le_bytes();
        for (i, byte) in data.iter_mut().enumerate() {
            let config_offset = offset as usize + i;
            if config_offset < 8 {
                *byte = capacity_bytes[config_offset];
            } else {
                *byte = 0;
            }
        }
    }

    fn write_config(&self, _offset: u64, _data: &[u8]) {
        // Block device config is read-only
    }

    fn activate(&mut self, mem: GuestMemoryRef, queues: &[QueueConfig]) {
        if let Some(q) = queues.first() {
            if q.size > 0 {
                self.queue = Some(VirtQueue::new(
                    mem.clone(),
                    q.desc_addr,
                    q.driver_addr,
                    q.device_addr,
                    q.size,
                ));
            }
        }
        self.mem = Some(mem);
    }

    fn queue_notify(&mut self, queue_index: u32) {
        if queue_index != 0 {
            return;
        }

        // Take the queue out to avoid split-borrow: queue_notify needs &mut queue
        // while process_read/write/get_id/write_status need &self/&mut self.
        let mut queue = match self.queue.take() {
            Some(q) => q,
            None => return,
        };

        // Process all available descriptor chains
        while let Some(chain) = queue.pop() {
            let descs = &chain.descriptors;

            // Need at least 2 descriptors: header + status
            if descs.len() < 2 {
                queue.push_used(chain.head, 0);
                continue;
            }

            // First descriptor: request header (must be device-readable)
            let header_desc = &descs[0];
            if header_desc.is_write_only() {
                queue.push_used(chain.head, 0);
                continue;
            }

            let (type_, sector) = match self.parse_header(header_desc.addr, header_desc.len) {
                Some(h) => h,
                None => {
                    queue.push_used(chain.head, 0);
                    continue;
                }
            };

            // Last descriptor: status (must be device-writable, 1 byte)
            let status_desc = &descs[descs.len() - 1];
            if !status_desc.is_write_only() || status_desc.len < 1 {
                queue.push_used(chain.head, 0);
                continue;
            }

            // Middle descriptors: data buffers
            let data_descs: Vec<(u64, u32)> = descs[1..descs.len() - 1]
                .iter()
                .map(|d| (d.addr, d.len))
                .collect();

            let total_data: u32 = data_descs.iter().map(|&(_, l)| l).sum();

            let status = match type_ {
                VIRTIO_BLK_T_IN => self.process_read(sector, &data_descs),
                VIRTIO_BLK_T_OUT => self.process_write(sector, &data_descs),
                VIRTIO_BLK_T_GET_ID => self.process_get_id(&data_descs),
                _ => VIRTIO_BLK_S_UNSUPP,
            };

            self.write_status(status_desc.addr, status);

            // Used len: data bytes transferred + 1 status byte
            let used_len = if status == VIRTIO_BLK_S_OK && type_ == VIRTIO_BLK_T_IN {
                total_data + 1
            } else {
                1
            };
            queue.push_used(chain.head, used_len);
        }

        self.queue = Some(queue);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::memory::{GuestMemory, RAM_BASE};
    use super::super::virtio_queue::{VRING_DESC_F_NEXT, VRING_DESC_F_WRITE};
    use std::io::Write as IoWrite;

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
    }

    impl TestHarness {
        fn new(path: &std::path::Path, read_only: bool) -> Self {
            let mem_size = 1024 * 1024; // 1MB
            let mem = GuestMemory::new(mem_size).unwrap();
            let mut dev = VirtioBlockDevice::new(path, read_only).unwrap();

            // Activate with queue config
            let queue_config = QueueConfig {
                desc_addr: RAM_BASE + DESC_TABLE_OFFSET,
                driver_addr: RAM_BASE + AVAIL_RING_OFFSET,
                device_addr: RAM_BASE + USED_RING_OFFSET,
                size: QUEUE_TEST_SIZE,
            };
            dev.activate(mem.clone_ref(RAM_BASE), &[queue_config]);

            Self { dev, mem }
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
            self.mem.write_at(entry_offset, &desc_head.to_le_bytes()).unwrap();
            // Write avail idx
            let idx_offset = AVAIL_RING_OFFSET + 2;
            self.mem.write_at(idx_offset, &avail_idx.to_le_bytes()).unwrap();
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
        fn setup_request(
            &self,
            type_: u32,
            sector: u64,
            data_len: u32,
            data_writable: bool,
        ) {
            let header_offset = DATA_AREA_OFFSET;
            let data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
            let status_offset = data_offset + data_len as u64;

            self.write_header(header_offset, type_, sector);

            // Desc 0: header (readable)
            self.write_desc(0, RAM_BASE + header_offset, REQ_HEADER_SIZE as u32,
                VRING_DESC_F_NEXT, 1);
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
        assert_ne!(f & VIRTIO_BLK_F_RO, 0, "must have RO bit");
    }

    #[test]
    fn block_features_read_write() {
        let path = temp_disk("feat-rw.img", 512);
        let dev = VirtioBlockDevice::new(&path, false).unwrap();
        let f = dev.features();
        assert_ne!(f & VIRTIO_F_VERSION_1, 0, "must have VERSION_1");
        assert_eq!(f & VIRTIO_BLK_F_RO, 0, "must NOT have RO bit");
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
        dev.read_config(8, &mut data);
        assert!(data.iter().all(|&b| b == 0));
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
        for i in 0..512 {
            data[i] = (i % 256) as u8;
        }
        let path = temp_disk_with_data("read-1.img", &data);
        let h = TestHarness::new(&path, true);

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
    fn block_read_multiple_sectors() {
        let mut data = vec![0u8; 1024]; // 2 sectors
        for i in 0..1024 {
            data[i] = ((i * 7) % 256) as u8;
        }
        let path = temp_disk_with_data("read-multi.img", &data);
        let h = TestHarness::new(&path, true);

        h.setup_request(VIRTIO_BLK_T_IN, 0, 1024, true);
        h.dev.queue_notify(0);

        let data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
        let read_back = h.read_bytes(data_offset, 1024);
        assert_eq!(read_back, data);

        let status_offset = data_offset + 1024;
        assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_OK);
    }

    #[test]
    fn block_write_single_sector() {
        let path = temp_disk("write-1.img", 512);
        let h = TestHarness::new(&path, false);

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
    fn block_write_to_read_only_returns_ioerr() {
        let original = vec![0xABu8; 512];
        let path = temp_disk_with_data("write-ro.img", &original);
        let h = TestHarness::new(&path, true);

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
        let h = TestHarness::new(&path, true);

        // Read sector 1 (out of bounds for a 1-sector disk)
        h.setup_request(VIRTIO_BLK_T_IN, 1, 512, true);
        h.dev.queue_notify(0);

        let status_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64 + 512;
        assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_IOERR);
    }

    #[test]
    fn block_write_past_end_returns_ioerr() {
        let path = temp_disk("write-oob.img", 512); // 1 sector
        let h = TestHarness::new(&path, false);

        h.setup_request(VIRTIO_BLK_T_OUT, 1, 512, false);
        h.dev.queue_notify(0);

        let status_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64 + 512;
        assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_IOERR);
    }

    #[test]
    fn block_get_id() {
        let path = temp_disk("getid-test.img", 512);
        let h = TestHarness::new(&path, false);

        h.setup_request(VIRTIO_BLK_T_GET_ID, 0, VIRTIO_BLK_ID_LEN as u32, true);
        h.dev.queue_notify(0);

        let data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
        let id_bytes = h.read_bytes(data_offset, VIRTIO_BLK_ID_LEN);
        assert_eq!(&id_bytes[..13], b"getid-test.im");

        let status_offset = data_offset + VIRTIO_BLK_ID_LEN as u64;
        assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_OK);
    }

    #[test]
    fn block_unknown_request_type_returns_unsupp() {
        let path = temp_disk("unsupp.img", 512);
        let h = TestHarness::new(&path, false);

        h.setup_request(99, 0, 512, true);
        h.dev.queue_notify(0);

        let status_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64 + 512;
        assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_UNSUPP);
    }

    #[test]
    fn block_multiple_requests_in_batch() {
        let mut data = vec![0u8; 1024]; // 2 sectors
        for i in 0..1024 {
            data[i] = (i % 256) as u8;
        }
        let path = temp_disk_with_data("batch.img", &data);
        let h = TestHarness::new(&path, true);

        // Request 1: read sector 0 using descs 0-2
        let hdr1_offset = DATA_AREA_OFFSET;
        let data1_offset = hdr1_offset + REQ_HEADER_SIZE as u64;
        let status1_offset = data1_offset + 512;

        h.write_header(hdr1_offset, VIRTIO_BLK_T_IN, 0);
        h.write_desc(0, RAM_BASE + hdr1_offset, REQ_HEADER_SIZE as u32,
            VRING_DESC_F_NEXT, 1);
        h.write_desc(1, RAM_BASE + data1_offset, 512,
            VRING_DESC_F_NEXT | VRING_DESC_F_WRITE, 2);
        h.write_desc(2, RAM_BASE + status1_offset, 1,
            VRING_DESC_F_WRITE, 0);

        // Request 2: read sector 1 using descs 3-5
        let hdr2_offset = status1_offset + 64; // gap
        let data2_offset = hdr2_offset + REQ_HEADER_SIZE as u64;
        let status2_offset = data2_offset + 512;

        h.write_header(hdr2_offset, VIRTIO_BLK_T_IN, 1);
        h.write_desc(3, RAM_BASE + hdr2_offset, REQ_HEADER_SIZE as u32,
            VRING_DESC_F_NEXT, 4);
        h.write_desc(4, RAM_BASE + data2_offset, 512,
            VRING_DESC_F_NEXT | VRING_DESC_F_WRITE, 5);
        h.write_desc(5, RAM_BASE + status2_offset, 1,
            VRING_DESC_F_WRITE, 0);

        // Both in avail ring
        h.push_avail(0, 0, 2); // desc head 0 at ring[0], avail_idx=2
        // Write ring entry for second request
        let entry_offset = AVAIL_RING_OFFSET + 4 + 1 * 2; // ring[1]
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
        let h = TestHarness::new(&path, false);
        // avail ring empty (idx=0), notify should be a no-op
        h.dev.queue_notify(0);
        assert_eq!(h.read_used_idx(), 0);
    }

    #[test]
    fn block_notify_wrong_queue_ignored() {
        let path = temp_disk("wrong-q.img", 512);
        let h = TestHarness::new(&path, false);
        h.dev.queue_notify(1); // only queue 0 exists
        h.dev.queue_notify(99);
        // no crash, no processing
    }

    // -----------------------------------------------------------------------
    // Category 4: Security / adversarial tests
    // -----------------------------------------------------------------------

    #[test]
    fn block_sector_overflow_u64() {
        let path = temp_disk("overflow.img", 512);
        let h = TestHarness::new(&path, true);

        // sector * 512 would overflow u64
        h.setup_request(VIRTIO_BLK_T_IN, u64::MAX / 256, 512, true);
        h.dev.queue_notify(0);

        let status_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64 + 512;
        assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_IOERR);
    }

    #[test]
    fn block_zero_length_data_descriptor() {
        let path = temp_disk("zero-len.img", 512);
        let h = TestHarness::new(&path, true);

        // Read with 0-length data buffer
        h.setup_request(VIRTIO_BLK_T_IN, 0, 0, true);
        h.dev.queue_notify(0);

        let status_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
        assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_OK);
    }

    #[test]
    fn block_data_gpa_out_of_ram() {
        let path = temp_disk("bad-gpa.img", 512);
        let h = TestHarness::new(&path, true);

        let header_offset = DATA_AREA_OFFSET;
        let status_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64 + 512;

        h.write_header(header_offset, VIRTIO_BLK_T_IN, 0);

        // Desc 0: header (valid)
        h.write_desc(0, RAM_BASE + header_offset, REQ_HEADER_SIZE as u32,
            VRING_DESC_F_NEXT, 1);
        // Desc 1: data buffer at invalid GPA (way outside RAM)
        h.write_desc(1, 0xDEAD_0000, 512,
            VRING_DESC_F_NEXT | VRING_DESC_F_WRITE, 2);
        // Desc 2: status
        h.write_desc(2, RAM_BASE + status_offset, 1,
            VRING_DESC_F_WRITE, 0);

        h.push_avail(0, 0, 1);
        h.dev.queue_notify(0);

        assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_IOERR);
    }

    #[test]
    fn block_notify_before_activate_noop() {
        let path = temp_disk("no-activate.img", 512);
        let dev = VirtioBlockDevice::new(&path, false).unwrap();
        // queue_notify before activate should not crash
        dev.queue_notify(0);
    }

    #[test]
    fn block_read_only_enforced_even_with_rw_feature() {
        // Device constructed as read-only -- writes must fail regardless
        let original = vec![0xAAu8; 512];
        let path = temp_disk_with_data("ro-enforced.img", &original);
        let h = TestHarness::new(&path, true);

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
}
