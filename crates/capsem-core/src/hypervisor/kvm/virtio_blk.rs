//! Virtio block device (type 2) for disk I/O.
//!
//! File-backed block device with one requestq. Supports read, write,
//! get-ID, and discard operations. Read-only mode enforced via feature bit
//! and write/discard rejection.

use std::collections::HashMap;
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Once};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use io_uring::{opcode, types, IoUring};
use metrics::{describe_counter, describe_histogram, Unit};

use super::memory::GuestMemoryRef;
use super::virtio_mmio::{QueueConfig, VirtioDevice};
use super::virtio_queue::{VirtQueue, VIRTIO_RING_F_EVENT_IDX};

/// Virtio block device ID.
const VIRTIO_ID_BLOCK: u32 = 2;

/// Maximum queue size for the requestq.
const QUEUE_SIZE: u16 = 256;

/// Sector size in bytes.
const SECTOR_SIZE: u64 = 512;

/// Maximum device ID length (virtio spec).
const VIRTIO_BLK_ID_LEN: usize = 20;

/// Size of one virtio discard segment.
const DISCARD_SEGMENT_SIZE: usize = 16;

// Feature bits
const VIRTIO_BLK_F_RO: u64 = 1 << 5;
const VIRTIO_BLK_F_DISCARD: u64 = 1 << 13;
const VIRTIO_F_VERSION_1: u64 = 1 << 32;

// Request types
const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;
const VIRTIO_BLK_T_GET_ID: u32 = 8;
const VIRTIO_BLK_T_DISCARD: u32 = 11;

// Status bytes
const VIRTIO_BLK_S_OK: u8 = 0;
const VIRTIO_BLK_S_IOERR: u8 = 1;
const VIRTIO_BLK_S_UNSUPP: u8 = 2;

// Request header size: type(u32) + reserved(u32) + sector(u64) = 16 bytes
const REQ_HEADER_SIZE: usize = 16;

// OTel-ready metric names. The metrics facade is no-op unless a recorder is
// installed, and still gives us stable names for future OTLP export.
const METRIC_QUEUE_NOTIFICATIONS_TOTAL: &str = "virtio.blk.queue_notifications_total";
const METRIC_QUEUE_DRAINS_TOTAL: &str = "virtio.blk.queue_drains_total";
const METRIC_DESCRIPTORS_DRAINED_TOTAL: &str = "virtio.blk.descriptors_drained_total";
const METRIC_USED_ENTRIES_TOTAL: &str = "virtio.blk.used_entries_total";
const METRIC_INTERRUPTS_TOTAL: &str = "virtio.blk.interrupts_total";
const METRIC_REQUESTS_TOTAL: &str = "virtio.blk.requests_total";
const METRIC_REQUEST_BYTES_TOTAL: &str = "virtio.blk.request_bytes_total";
const METRIC_REQUEST_DURATION_MS: &str = "virtio.blk.request_duration_ms";
const METRIC_QUEUE_DRAIN_DURATION_MS: &str = "virtio.blk.queue_drain_duration_ms";
const METRIC_QUIESCE_DRAIN_DURATION_MS: &str = "virtio.blk.quiesce_drain_duration_ms";
const METRIC_ASYNC_SUBMISSIONS_TOTAL: &str = "virtio.blk.async_submissions_total";
const METRIC_ASYNC_COMPLETIONS_TOTAL: &str = "virtio.blk.async_completions_total";
const METRIC_ASYNC_FALLBACKS_TOTAL: &str = "virtio.blk.async_fallbacks_total";
const METRIC_ASYNC_IN_FLIGHT: &str = "virtio.blk.async_in_flight";

static DESCRIBE_METRICS: Once = Once::new();

/// Virtio block device backed by a file.
pub(super) struct VirtioBlockDevice {
    file: std::fs::File,
    read_only: bool,
    capacity_sectors: u64,
    device_id: [u8; VIRTIO_BLK_ID_LEN],
    queue: Option<VirtQueue>,
    mem: Option<GuestMemoryRef>,
    irq_fd: Option<RawFd>,
    interrupt_status: Option<Arc<AtomicU32>>,
    notify_fd: Option<OwnedFd>,
    control_tx: Option<mpsc::Sender<BlockWorkerCommand>>,
    worker_handle: Option<std::thread::JoinHandle<()>>,
}

enum BlockWorkerCommand {
    Drain(mpsc::Sender<()>),
    Stop,
}

impl VirtioBlockDevice {
    /// Create a new virtio block device backed by a file.
    ///
    /// If `read_only` is true, the file is opened read-only and
    /// VIRTIO_BLK_F_RO is advertised. Writes are rejected.
    pub fn new(path: &Path, read_only: bool) -> Result<Self> {
        describe_metrics_once();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(!read_only)
            .open(path)
            .with_context(|| format!("open block device: {}", path.display()))?;

        let file_size = file
            .metadata()
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
            irq_fd: None,
            interrupt_status: None,
            notify_fd: None,
            control_tx: None,
            worker_handle: None,
        })
    }

    pub fn with_async_notify(
        mut self,
        irq_fd: RawFd,
        interrupt_status: Arc<AtomicU32>,
        notify_fd: OwnedFd,
    ) -> Self {
        self.irq_fd = Some(irq_fd);
        self.interrupt_status = Some(interrupt_status);
        self.notify_fd = Some(notify_fd);
        self
    }

    /// Process a read request: file -> guest memory.
    fn process_read(
        file: &std::fs::File,
        mem: &GuestMemoryRef,
        capacity_sectors: u64,
        sector: u64,
        data_descs: &[(u64, u32)],
    ) -> u8 {
        let offset = match sector.checked_mul(SECTOR_SIZE) {
            Some(o) => o,
            None => {
                log_block_ioerr("read", sector, 0, "sector_overflow", None);
                return VIRTIO_BLK_S_IOERR;
            }
        };

        let total_len: u64 = data_descs.iter().map(|&(_, l)| l as u64).sum();
        if offset
            .checked_add(total_len)
            .is_none_or(|end| end > capacity_sectors * SECTOR_SIZE)
        {
            log_block_ioerr("read", sector, total_len, "out_of_bounds", None);
            return VIRTIO_BLK_S_IOERR;
        }

        let iovecs = match Self::guest_iovecs(mem, data_descs) {
            Some(iovecs) => iovecs,
            None => {
                log_block_ioerr("read", sector, total_len, "guest_memory", None);
                return VIRTIO_BLK_S_IOERR;
            }
        };
        match Self::preadv_all(file.as_raw_fd(), &iovecs, offset, total_len) {
            Ok(()) => VIRTIO_BLK_S_OK,
            Err(error) => {
                log_block_ioerr("read", sector, total_len, "host_preadv", Some(&error));
                VIRTIO_BLK_S_IOERR
            }
        }
    }

    /// Process a write request: guest memory -> file.
    fn process_write(
        file: &std::fs::File,
        mem: &GuestMemoryRef,
        read_only: bool,
        capacity_sectors: u64,
        sector: u64,
        data_descs: &[(u64, u32)],
    ) -> u8 {
        if read_only {
            let total_len: u64 = data_descs.iter().map(|&(_, l)| l as u64).sum();
            log_block_ioerr("write", sector, total_len, "read_only", None);
            return VIRTIO_BLK_S_IOERR;
        }

        let offset = match sector.checked_mul(SECTOR_SIZE) {
            Some(o) => o,
            None => {
                log_block_ioerr("write", sector, 0, "sector_overflow", None);
                return VIRTIO_BLK_S_IOERR;
            }
        };

        let total_len: u64 = data_descs.iter().map(|&(_, l)| l as u64).sum();
        if offset
            .checked_add(total_len)
            .is_none_or(|end| end > capacity_sectors * SECTOR_SIZE)
        {
            log_block_ioerr("write", sector, total_len, "out_of_bounds", None);
            return VIRTIO_BLK_S_IOERR;
        }

        let iovecs = match Self::guest_iovecs(mem, data_descs) {
            Some(iovecs) => iovecs,
            None => {
                log_block_ioerr("write", sector, total_len, "guest_memory", None);
                return VIRTIO_BLK_S_IOERR;
            }
        };
        match Self::pwritev_all(file.as_raw_fd(), &iovecs, offset, total_len) {
            Ok(()) => VIRTIO_BLK_S_OK,
            Err(error) => {
                log_block_ioerr("write", sector, total_len, "host_pwritev", Some(&error));
                VIRTIO_BLK_S_IOERR
            }
        }
    }

    fn guest_iovecs(mem: &GuestMemoryRef, data_descs: &[(u64, u32)]) -> Option<Vec<libc::iovec>> {
        let mut iovecs = Vec::with_capacity(data_descs.len());
        for &(gpa, len) in data_descs {
            if len == 0 {
                continue;
            }
            let host_ptr = mem.gpa_range_to_host(gpa, len as u64)?;
            iovecs.push(libc::iovec {
                iov_base: host_ptr.cast(),
                iov_len: len as usize,
            });
        }
        Some(iovecs)
    }

    fn prepare_rw_iovecs(
        mem: &GuestMemoryRef,
        capacity_sectors: u64,
        sector: u64,
        data_descs: &[(u64, u32)],
    ) -> Result<(u64, u64, Vec<libc::iovec>), u8> {
        let offset = sector.checked_mul(SECTOR_SIZE).ok_or(VIRTIO_BLK_S_IOERR)?;
        let total_len: u64 = data_descs.iter().map(|&(_, l)| l as u64).sum();
        if offset
            .checked_add(total_len)
            .is_none_or(|end| end > capacity_sectors * SECTOR_SIZE)
        {
            return Err(VIRTIO_BLK_S_IOERR);
        }
        let iovecs = Self::guest_iovecs(mem, data_descs).ok_or(VIRTIO_BLK_S_IOERR)?;
        Ok((offset, total_len, iovecs))
    }

    fn iovecs_after(iovecs: &[libc::iovec], mut consumed: u64) -> Vec<libc::iovec> {
        let mut adjusted = Vec::with_capacity(iovecs.len());
        for iov in iovecs {
            if consumed >= iov.iov_len as u64 {
                consumed -= iov.iov_len as u64;
                continue;
            }
            let skip = consumed as usize;
            adjusted.push(libc::iovec {
                iov_base: unsafe { (iov.iov_base as *mut u8).add(skip).cast() },
                iov_len: iov.iov_len - skip,
            });
            consumed = 0;
        }
        adjusted
    }

    fn preadv_all(
        fd: std::os::fd::RawFd,
        iovecs: &[libc::iovec],
        offset: u64,
        total_len: u64,
    ) -> std::io::Result<()> {
        let mut done = 0_u64;
        while done < total_len {
            let adjusted = Self::iovecs_after(iovecs, done);
            let ret = unsafe {
                libc::preadv(
                    fd,
                    adjusted.as_ptr(),
                    adjusted.len() as libc::c_int,
                    (offset + done) as libc::off_t,
                )
            };
            if ret < 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err(err);
            }
            if ret == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "short virtio-blk read",
                ));
            }
            done += ret as u64;
        }
        Ok(())
    }

    fn pwritev_all(
        fd: std::os::fd::RawFd,
        iovecs: &[libc::iovec],
        offset: u64,
        total_len: u64,
    ) -> std::io::Result<()> {
        let mut done = 0_u64;
        while done < total_len {
            let adjusted = Self::iovecs_after(iovecs, done);
            let ret = unsafe {
                libc::pwritev(
                    fd,
                    adjusted.as_ptr(),
                    adjusted.len() as libc::c_int,
                    (offset + done) as libc::off_t,
                )
            };
            if ret < 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err(err);
            }
            if ret == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "short virtio-blk write",
                ));
            }
            done += ret as u64;
        }
        Ok(())
    }

    /// Process a get-ID request: copy device_id to guest buffer.
    fn process_get_id(
        mem: &GuestMemoryRef,
        device_id: &[u8; VIRTIO_BLK_ID_LEN],
        data_descs: &[(u64, u32)],
    ) -> u8 {
        if let Some(&(gpa, len)) = data_descs.first() {
            let copy_len = (len as usize).min(VIRTIO_BLK_ID_LEN);
            if copy_len == 0 {
                return VIRTIO_BLK_S_OK;
            }
            if let Some(host_ptr) = mem.gpa_range_to_host(gpa, copy_len as u64) {
                let buf = unsafe { std::slice::from_raw_parts_mut(host_ptr, copy_len) };
                buf.copy_from_slice(&device_id[..copy_len]);
            } else {
                return VIRTIO_BLK_S_IOERR;
            }
        }

        VIRTIO_BLK_S_OK
    }

    /// Process a discard request by punching holes in the backing file.
    fn process_discard(
        file: &mut std::fs::File,
        mem: &GuestMemoryRef,
        read_only: bool,
        capacity_sectors: u64,
        data_descs: &[(u64, u32)],
    ) -> u8 {
        if read_only {
            return VIRTIO_BLK_S_IOERR;
        }

        let data = match Self::read_guest_data(mem, data_descs) {
            Some(data) => data,
            None => return VIRTIO_BLK_S_IOERR,
        };
        if data.len() % DISCARD_SEGMENT_SIZE != 0 {
            return VIRTIO_BLK_S_IOERR;
        }

        for segment in data.chunks_exact(DISCARD_SEGMENT_SIZE) {
            let sector = u64::from_le_bytes(segment[0..8].try_into().unwrap());
            let num_sectors = u32::from_le_bytes(segment[8..12].try_into().unwrap()) as u64;
            if num_sectors == 0 {
                continue;
            }

            let offset = match sector.checked_mul(SECTOR_SIZE) {
                Some(offset) => offset,
                None => return VIRTIO_BLK_S_IOERR,
            };
            let len = match num_sectors.checked_mul(SECTOR_SIZE) {
                Some(len) => len,
                None => return VIRTIO_BLK_S_IOERR,
            };
            if offset
                .checked_add(len)
                .is_none_or(|end| end > capacity_sectors * SECTOR_SIZE)
            {
                return VIRTIO_BLK_S_IOERR;
            }

            if Self::discard_range(file, offset, len).is_err() {
                return VIRTIO_BLK_S_IOERR;
            }
        }

        VIRTIO_BLK_S_OK
    }

    fn read_guest_data(mem: &GuestMemoryRef, data_descs: &[(u64, u32)]) -> Option<Vec<u8>> {
        let total_len: usize = data_descs.iter().map(|&(_, len)| len as usize).sum();
        let mut data = Vec::with_capacity(total_len);
        for &(gpa, len) in data_descs {
            if len == 0 {
                continue;
            }
            let host_ptr = mem.gpa_range_to_host(gpa, len as u64)?;
            let buf = unsafe { std::slice::from_raw_parts(host_ptr, len as usize) };
            data.extend_from_slice(buf);
        }
        Some(data)
    }

    fn discard_range(file: &mut std::fs::File, offset: u64, len: u64) -> std::io::Result<()> {
        let ret = unsafe {
            libc::fallocate(
                file.as_raw_fd(),
                libc::FALLOC_FL_KEEP_SIZE | libc::FALLOC_FL_PUNCH_HOLE,
                offset as libc::off_t,
                len as libc::off_t,
            )
        };
        if ret == 0 {
            return Ok(());
        }

        let error = std::io::Error::last_os_error();
        match error.raw_os_error() {
            // Keep the guest operation functional on filesystems without hole
            // punching; ext4/xfs/btrfs still reclaim blocks through fallocate.
            Some(libc::EOPNOTSUPP | libc::ENOSYS | libc::EINVAL) => {
                file.seek(SeekFrom::Start(offset))?;
                let mut remaining = len;
                let zeros = [0_u8; 64 * 1024];
                while remaining > 0 {
                    let n = zeros.len().min(remaining as usize);
                    file.write_all(&zeros[..n])?;
                    remaining -= n as u64;
                }
                Ok(())
            }
            _ => Err(error),
        }
    }

    /// Write a status byte to a guest physical address.
    fn write_status(mem: &GuestMemoryRef, gpa: u64, status: u8) {
        if let Some(ptr) = mem.gpa_range_to_host(gpa, 1) {
            unsafe {
                *ptr = status;
            }
        }
    }

    /// Parse a request header from guest memory.
    /// Returns (type, sector) or None if the read fails.
    fn parse_header(mem: &GuestMemoryRef, gpa: u64, len: u32) -> Option<(u32, u64)> {
        if (len as usize) < REQ_HEADER_SIZE {
            return None;
        }
        let ptr = mem.gpa_range_to_host(gpa, REQ_HEADER_SIZE as u64)?;
        unsafe {
            let header = std::slice::from_raw_parts(ptr, REQ_HEADER_SIZE);
            let type_ = u32::from_le_bytes(header[0..4].try_into().ok()?);
            // skip 4 bytes reserved
            let sector = u64::from_le_bytes(header[8..16].try_into().ok()?);
            Some((type_, sector))
        }
    }

    fn process_queue(
        file: &mut std::fs::File,
        read_only: bool,
        capacity_sectors: u64,
        device_id: &[u8; VIRTIO_BLK_ID_LEN],
        mem: &GuestMemoryRef,
        queue: &mut VirtQueue,
    ) -> QueueProcessResult {
        let drain_started = Instant::now();
        let mut processed = 0u32;
        let mut used_entries = 0u32;
        let mut read_ops = 0u32;
        let mut write_ops = 0u32;
        let mut bytes_read = 0u64;
        let mut bytes_written = 0u64;
        while let Some(chain) = queue.pop_or_enable_notification() {
            let descs = &chain.descriptors;
            processed += 1;

            if descs.len() < 2 {
                tracing::warn!(
                    event_name = "virtio.blk.request_malformed",
                    head = chain.head,
                    descriptors = descs.len(),
                    "virtio-blk descriptor chain too short"
                );
                queue.push_used_deferred(chain.head, 0);
                used_entries += 1;
                continue;
            }

            let header_desc = &descs[0];
            if header_desc.is_write_only() {
                tracing::warn!(
                    event_name = "virtio.blk.request_malformed",
                    head = chain.head,
                    descriptors = descs.len(),
                    "virtio-blk request header descriptor was write-only"
                );
                queue.push_used_deferred(chain.head, 0);
                used_entries += 1;
                continue;
            }

            let (type_, sector) = match Self::parse_header(mem, header_desc.addr, header_desc.len) {
                Some(h) => h,
                None => {
                    tracing::warn!(
                        event_name = "virtio.blk.request_malformed",
                        head = chain.head,
                        header_addr = format_args!("{:#x}", header_desc.addr),
                        header_len = header_desc.len,
                        "virtio-blk request header could not be parsed"
                    );
                    queue.push_used_deferred(chain.head, 0);
                    used_entries += 1;
                    continue;
                }
            };

            let status_desc = &descs[descs.len() - 1];
            if !status_desc.is_write_only() || status_desc.len < 1 {
                tracing::warn!(
                    event_name = "virtio.blk.request_malformed",
                    head = chain.head,
                    status_addr = format_args!("{:#x}", status_desc.addr),
                    status_len = status_desc.len,
                    status_write_only = status_desc.is_write_only(),
                    "virtio-blk status descriptor was invalid"
                );
                queue.push_used_deferred(chain.head, 0);
                used_entries += 1;
                continue;
            }

            let data_descs: Vec<(u64, u32)> = descs[1..descs.len() - 1]
                .iter()
                .map(|d| (d.addr, d.len))
                .collect();
            let total_data: u32 = data_descs.iter().map(|&(_, l)| l).sum();

            let status = match type_ {
                VIRTIO_BLK_T_IN => timed_request(type_, total_data, || {
                    Self::process_read(file, mem, capacity_sectors, sector, &data_descs)
                }),
                VIRTIO_BLK_T_OUT => timed_request(type_, total_data, || {
                    Self::process_write(file, mem, read_only, capacity_sectors, sector, &data_descs)
                }),
                VIRTIO_BLK_T_GET_ID => timed_request(type_, total_data, || {
                    Self::process_get_id(mem, device_id, &data_descs)
                }),
                VIRTIO_BLK_T_DISCARD => timed_request(type_, total_data, || {
                    Self::process_discard(file, mem, read_only, capacity_sectors, &data_descs)
                }),
                _ => timed_request(type_, total_data, || VIRTIO_BLK_S_UNSUPP),
            };
            match type_ {
                VIRTIO_BLK_T_IN => {
                    read_ops += 1;
                    if status == VIRTIO_BLK_S_OK {
                        bytes_read += total_data as u64;
                    }
                }
                VIRTIO_BLK_T_OUT => {
                    write_ops += 1;
                    if status == VIRTIO_BLK_S_OK {
                        bytes_written += total_data as u64;
                    }
                }
                _ => {}
            }
            tracing::trace!(
                event_name = "virtio.blk.request_complete",
                head = chain.head,
                request_type = type_,
                sector,
                descriptor_count = descs.len(),
                total_data,
                status,
                "virtio-blk request completed"
            );

            Self::write_status(mem, status_desc.addr, status);

            let used_len = if status == VIRTIO_BLK_S_OK && type_ == VIRTIO_BLK_T_IN {
                total_data + 1
            } else {
                1
            };
            queue.push_used_deferred(chain.head, used_len);
            used_entries += 1;
        }

        if processed > 0 {
            queue.flush_used();
        }

        let should_interrupt = queue.prepare_kick();
        let drain_duration = drain_started.elapsed();
        QueueProcessResult {
            processed,
            submitted: 0,
            used_entries,
            should_interrupt,
            read_ops,
            write_ops,
            bytes_read,
            bytes_written,
            drain_duration,
        }
    }

    fn process_queue_uring(
        file: &mut std::fs::File,
        read_only: bool,
        capacity_sectors: u64,
        device_id: &[u8; VIRTIO_BLK_ID_LEN],
        mem: &GuestMemoryRef,
        queue: &mut VirtQueue,
        uring: &mut BlockIoUring,
    ) -> QueueProcessResult {
        let drain_started = Instant::now();
        let mut result = QueueProcessResult::new(drain_started);
        while let Some(chain) = queue.pop_or_enable_notification() {
            let descs = &chain.descriptors;
            result.processed += 1;

            if descs.len() < 2 {
                tracing::warn!(
                    event_name = "virtio.blk.request_malformed",
                    head = chain.head,
                    descriptors = descs.len(),
                    "virtio-blk descriptor chain too short"
                );
                queue.push_used_deferred(chain.head, 0);
                result.used_entries += 1;
                continue;
            }

            let header_desc = &descs[0];
            if header_desc.is_write_only() {
                tracing::warn!(
                    event_name = "virtio.blk.request_malformed",
                    head = chain.head,
                    descriptors = descs.len(),
                    "virtio-blk request header descriptor was write-only"
                );
                queue.push_used_deferred(chain.head, 0);
                result.used_entries += 1;
                continue;
            }

            let (type_, sector) = match Self::parse_header(mem, header_desc.addr, header_desc.len) {
                Some(h) => h,
                None => {
                    tracing::warn!(
                        event_name = "virtio.blk.request_malformed",
                        head = chain.head,
                        header_addr = format_args!("{:#x}", header_desc.addr),
                        header_len = header_desc.len,
                        "virtio-blk request header could not be parsed"
                    );
                    queue.push_used_deferred(chain.head, 0);
                    result.used_entries += 1;
                    continue;
                }
            };

            let status_desc = &descs[descs.len() - 1];
            if !status_desc.is_write_only() || status_desc.len < 1 {
                tracing::warn!(
                    event_name = "virtio.blk.request_malformed",
                    head = chain.head,
                    status_addr = format_args!("{:#x}", status_desc.addr),
                    status_len = status_desc.len,
                    status_write_only = status_desc.is_write_only(),
                    "virtio-blk status descriptor was invalid"
                );
                queue.push_used_deferred(chain.head, 0);
                result.used_entries += 1;
                continue;
            }

            let data_descs: Vec<(u64, u32)> = descs[1..descs.len() - 1]
                .iter()
                .map(|d| (d.addr, d.len))
                .collect();
            let total_data: u32 = data_descs.iter().map(|&(_, l)| l).sum();

            match type_ {
                VIRTIO_BLK_T_IN | VIRTIO_BLK_T_OUT => {
                    if type_ == VIRTIO_BLK_T_OUT && read_only {
                        timed_request(type_, total_data, || VIRTIO_BLK_S_IOERR);
                        Self::write_status(mem, status_desc.addr, VIRTIO_BLK_S_IOERR);
                        queue.push_used_deferred(chain.head, 1);
                        result.used_entries += 1;
                        result.write_ops += 1;
                        continue;
                    }

                    let (offset, _total_len, iovecs) =
                        match Self::prepare_rw_iovecs(mem, capacity_sectors, sector, &data_descs) {
                            Ok(prepared) => prepared,
                            Err(status) => {
                                timed_request(type_, total_data, || status);
                                Self::write_status(mem, status_desc.addr, status);
                                queue.push_used_deferred(chain.head, 1);
                                result.used_entries += 1;
                                if type_ == VIRTIO_BLK_T_IN {
                                    result.read_ops += 1;
                                } else {
                                    result.write_ops += 1;
                                }
                                continue;
                            }
                        };

                    if uring
                        .submit_rw(
                            chain.head,
                            type_,
                            total_data,
                            status_desc.addr,
                            offset,
                            iovecs,
                        )
                        .is_ok()
                    {
                        result.submitted += 1;
                        if type_ == VIRTIO_BLK_T_IN {
                            result.read_ops += 1;
                        } else {
                            result.write_ops += 1;
                        }
                        continue;
                    }

                    ::metrics::counter!(
                        METRIC_ASYNC_FALLBACKS_TOTAL,
                        "operation" => request_operation_label(type_),
                    )
                    .increment(1);
                    let status = if type_ == VIRTIO_BLK_T_IN {
                        timed_request(type_, total_data, || {
                            Self::process_read(file, mem, capacity_sectors, sector, &data_descs)
                        })
                    } else {
                        timed_request(type_, total_data, || {
                            Self::process_write(
                                file,
                                mem,
                                read_only,
                                capacity_sectors,
                                sector,
                                &data_descs,
                            )
                        })
                    };
                    Self::write_status(mem, status_desc.addr, status);
                    let used_len = if status == VIRTIO_BLK_S_OK && type_ == VIRTIO_BLK_T_IN {
                        total_data + 1
                    } else {
                        1
                    };
                    queue.push_used_deferred(chain.head, used_len);
                    result.used_entries += 1;
                    if type_ == VIRTIO_BLK_T_IN {
                        result.read_ops += 1;
                        if status == VIRTIO_BLK_S_OK {
                            result.bytes_read += total_data as u64;
                        }
                    } else {
                        result.write_ops += 1;
                        if status == VIRTIO_BLK_S_OK {
                            result.bytes_written += total_data as u64;
                        }
                    }
                }
                VIRTIO_BLK_T_GET_ID => {
                    let status = timed_request(type_, total_data, || {
                        Self::process_get_id(mem, device_id, &data_descs)
                    });
                    Self::write_status(mem, status_desc.addr, status);
                    queue.push_used_deferred(chain.head, 1);
                    result.used_entries += 1;
                }
                VIRTIO_BLK_T_DISCARD => {
                    let status = timed_request(type_, total_data, || {
                        Self::process_discard(file, mem, read_only, capacity_sectors, &data_descs)
                    });
                    Self::write_status(mem, status_desc.addr, status);
                    queue.push_used_deferred(chain.head, 1);
                    result.used_entries += 1;
                }
                _ => {
                    let status = timed_request(type_, total_data, || VIRTIO_BLK_S_UNSUPP);
                    Self::write_status(mem, status_desc.addr, status);
                    queue.push_used_deferred(chain.head, 1);
                    result.used_entries += 1;
                }
            }
        }

        if result.used_entries > 0 {
            queue.flush_used();
        }

        result.should_interrupt = queue.prepare_kick();
        result.drain_duration = drain_started.elapsed();
        result
    }
}

struct QueueProcessResult {
    processed: u32,
    submitted: u32,
    used_entries: u32,
    should_interrupt: bool,
    read_ops: u32,
    write_ops: u32,
    bytes_read: u64,
    bytes_written: u64,
    drain_duration: Duration,
}

impl QueueProcessResult {
    fn new(drain_started: Instant) -> Self {
        Self {
            processed: 0,
            submitted: 0,
            used_entries: 0,
            should_interrupt: false,
            read_ops: 0,
            write_ops: 0,
            bytes_read: 0,
            bytes_written: 0,
            drain_duration: drain_started.elapsed(),
        }
    }
}

struct PendingBlockRequest {
    head: u16,
    type_: u32,
    total_data: u32,
    status_addr: u64,
    iovecs: Vec<libc::iovec>,
    started: Instant,
}

struct BlockIoUring {
    ring: IoUring,
    completion_fd: OwnedFd,
    pending: HashMap<u64, PendingBlockRequest>,
    next_user_data: u64,
    file_fd: RawFd,
}

impl BlockIoUring {
    fn new(file_fd: RawFd) -> std::io::Result<Self> {
        let completion_fd = create_eventfd(libc::EFD_CLOEXEC | libc::EFD_NONBLOCK)?;
        let ring = IoUring::new(QUEUE_SIZE as u32)?;
        ring.submitter()
            .register_eventfd(completion_fd.as_raw_fd())?;
        Ok(Self {
            ring,
            completion_fd,
            pending: HashMap::new(),
            next_user_data: 1,
            file_fd,
        })
    }

    fn completion_fd(&self) -> RawFd {
        self.completion_fd.as_raw_fd()
    }

    fn pending_len(&self) -> usize {
        self.pending.len()
    }

    fn submit_rw(
        &mut self,
        head: u16,
        type_: u32,
        total_data: u32,
        status_addr: u64,
        offset: u64,
        iovecs: Vec<libc::iovec>,
    ) -> std::io::Result<()> {
        let user_data = self.next_user_data;
        self.next_user_data = self.next_user_data.wrapping_add(1).max(1);
        let iovec_ptr = iovecs.as_ptr();
        let iovec_len = iovecs.len() as u32;
        let entry = match type_ {
            VIRTIO_BLK_T_IN => opcode::Readv::new(types::Fd(self.file_fd), iovec_ptr, iovec_len)
                .offset(offset)
                .build()
                .user_data(user_data),
            VIRTIO_BLK_T_OUT => opcode::Writev::new(types::Fd(self.file_fd), iovec_ptr, iovec_len)
                .offset(offset)
                .build()
                .user_data(user_data),
            _ => unreachable!("only read/write requests are submitted to io_uring"),
        };
        self.pending.insert(
            user_data,
            PendingBlockRequest {
                head,
                type_,
                total_data,
                status_addr,
                iovecs,
                started: Instant::now(),
            },
        );

        let push_result = unsafe { self.ring.submission().push(&entry) };
        if push_result.is_err() {
            self.pending.remove(&user_data);
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "io_uring submission queue full",
            ));
        }
        loop {
            match self.ring.submit() {
                Ok(_) => break,
                Err(error) if error.raw_os_error() == Some(libc::EINTR) => continue,
                Err(error) => {
                    tracing::warn!(
                        event_name = "virtio.blk.io_uring_submit_failed",
                        %error,
                        operation = request_operation_label(type_),
                        "virtio-blk io_uring submit failed after queueing SQE"
                    );
                    break;
                }
            }
        }
        ::metrics::counter!(
            METRIC_ASYNC_SUBMISSIONS_TOTAL,
            "operation" => request_operation_label(type_),
        )
        .increment(1);
        ::metrics::histogram!(METRIC_ASYNC_IN_FLIGHT, "backend" => "io_uring")
            .record(self.pending.len() as f64);
        Ok(())
    }

    fn reap_completions(
        &mut self,
        mem: &GuestMemoryRef,
        queue: &mut VirtQueue,
    ) -> CompletionResult {
        let mut result = CompletionResult::default();
        let completions: Vec<_> = self
            .ring
            .completion()
            .map(|cqe| (cqe.user_data(), cqe.result()))
            .collect();
        for (user_data, io_result) in completions {
            let Some(request) = self.pending.remove(&user_data) else {
                tracing::warn!(
                    event_name = "virtio.blk.io_uring_unknown_completion",
                    user_data,
                    io_result,
                    "virtio-blk io_uring completion had no pending request"
                );
                continue;
            };
            let status = if io_result >= 0 && io_result as u32 == request.total_data {
                VIRTIO_BLK_S_OK
            } else {
                VIRTIO_BLK_S_IOERR
            };
            emit_request_metrics(
                request.type_,
                request.total_data,
                status,
                request.started.elapsed(),
            );
            ::metrics::counter!(
                METRIC_ASYNC_COMPLETIONS_TOTAL,
                "operation" => request_operation_label(request.type_),
                "status" => request_status_label(status),
            )
            .increment(1);
            VirtioBlockDevice::write_status(mem, request.status_addr, status);
            let used_len = if status == VIRTIO_BLK_S_OK && request.type_ == VIRTIO_BLK_T_IN {
                request.total_data + 1
            } else {
                1
            };
            queue.push_used_deferred(request.head, used_len);
            result.completed += 1;
            result.used_entries += 1;
            match request.type_ {
                VIRTIO_BLK_T_IN => {
                    result.read_ops += 1;
                    if status == VIRTIO_BLK_S_OK {
                        result.bytes_read += request.total_data as u64;
                    }
                }
                VIRTIO_BLK_T_OUT => {
                    result.write_ops += 1;
                    if status == VIRTIO_BLK_S_OK {
                        result.bytes_written += request.total_data as u64;
                    }
                }
                _ => {}
            }
        }
        if result.used_entries > 0 {
            queue.flush_used();
            result.should_interrupt = queue.prepare_kick();
            ::metrics::counter!(METRIC_USED_ENTRIES_TOTAL, "backend" => "io_uring")
                .increment(result.used_entries as u64);
            if result.should_interrupt {
                ::metrics::counter!(
                    METRIC_INTERRUPTS_TOTAL,
                    "backend" => "io_uring",
                    "decision" => "raised",
                )
                .increment(1);
            } else {
                ::metrics::counter!(
                    METRIC_INTERRUPTS_TOTAL,
                    "backend" => "io_uring",
                    "decision" => "suppressed",
                )
                .increment(1);
            }
        }
        ::metrics::histogram!(METRIC_ASYNC_IN_FLIGHT, "backend" => "io_uring")
            .record(self.pending.len() as f64);
        result
    }
}

#[derive(Default)]
struct CompletionResult {
    completed: u32,
    used_entries: u32,
    should_interrupt: bool,
    read_ops: u32,
    write_ops: u32,
    bytes_read: u64,
    bytes_written: u64,
}

fn describe_metrics_once() {
    DESCRIBE_METRICS.call_once(|| {
        describe_counter!(
            METRIC_QUEUE_NOTIFICATIONS_TOTAL,
            Unit::Count,
            "Virtio block queue notifications observed by backend."
        );
        describe_counter!(
            METRIC_QUEUE_DRAINS_TOTAL,
            Unit::Count,
            "Virtio block queue drain attempts by backend."
        );
        describe_counter!(
            METRIC_DESCRIPTORS_DRAINED_TOTAL,
            Unit::Count,
            "Virtio block descriptor chains drained by backend."
        );
        describe_counter!(
            METRIC_USED_ENTRIES_TOTAL,
            Unit::Count,
            "Virtio block used-ring entries published to the guest."
        );
        describe_counter!(
            METRIC_INTERRUPTS_TOTAL,
            Unit::Count,
            "Virtio block interrupt decisions, partitioned by raised|suppressed."
        );
        describe_counter!(
            METRIC_REQUESTS_TOTAL,
            Unit::Count,
            "Virtio block requests by operation and completion status."
        );
        describe_counter!(
            METRIC_REQUEST_BYTES_TOTAL,
            Unit::Bytes,
            "Virtio block request payload bytes by operation and completion status."
        );
        describe_histogram!(
            METRIC_REQUEST_DURATION_MS,
            Unit::Milliseconds,
            "Virtio block request processing wall time."
        );
        describe_histogram!(
            METRIC_QUEUE_DRAIN_DURATION_MS,
            Unit::Milliseconds,
            "Virtio block queue drain wall time per backend wake."
        );
        describe_histogram!(
            METRIC_QUIESCE_DRAIN_DURATION_MS,
            Unit::Milliseconds,
            "Virtio block quiesce drain wait time before checkpoint."
        );
        describe_counter!(
            METRIC_ASYNC_SUBMISSIONS_TOTAL,
            Unit::Count,
            "Virtio block io_uring submissions by operation."
        );
        describe_counter!(
            METRIC_ASYNC_COMPLETIONS_TOTAL,
            Unit::Count,
            "Virtio block io_uring completions by operation and completion status."
        );
        describe_counter!(
            METRIC_ASYNC_FALLBACKS_TOTAL,
            Unit::Count,
            "Virtio block requests handled by synchronous fallback from the async path."
        );
        describe_histogram!(
            METRIC_ASYNC_IN_FLIGHT,
            Unit::Count,
            "Virtio block io_uring in-flight request depth after submit/completion."
        );
    });
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn timed_request(type_: u32, total_data: u32, f: impl FnOnce() -> u8) -> u8 {
    let started = Instant::now();
    let status = f();
    emit_request_metrics(type_, total_data, status, started.elapsed());
    status
}

fn emit_request_metrics(type_: u32, total_data: u32, status: u8, duration: Duration) {
    let operation = request_operation_label(type_);
    let status_label = request_status_label(status);
    ::metrics::counter!(
        METRIC_REQUESTS_TOTAL,
        "operation" => operation,
        "status" => status_label,
    )
    .increment(1);
    if total_data > 0 {
        ::metrics::counter!(
            METRIC_REQUEST_BYTES_TOTAL,
            "operation" => operation,
            "status" => status_label,
        )
        .increment(total_data as u64);
    }
    ::metrics::histogram!(
        METRIC_REQUEST_DURATION_MS,
        "operation" => operation,
        "status" => status_label,
    )
    .record(duration_ms(duration));
}

fn emit_queue_notification_metric(backend: &'static str, count: u64) {
    ::metrics::counter!(METRIC_QUEUE_NOTIFICATIONS_TOTAL, "backend" => backend).increment(count);
}

fn emit_queue_drain_metrics(backend: &'static str, result: &QueueProcessResult) {
    ::metrics::counter!(METRIC_QUEUE_DRAINS_TOTAL, "backend" => backend).increment(1);
    if result.processed > 0 {
        ::metrics::counter!(METRIC_DESCRIPTORS_DRAINED_TOTAL, "backend" => backend)
            .increment(result.processed as u64);
    }
    if result.used_entries > 0 {
        ::metrics::counter!(METRIC_USED_ENTRIES_TOTAL, "backend" => backend)
            .increment(result.used_entries as u64);
    }
    if result.should_interrupt {
        ::metrics::counter!(METRIC_INTERRUPTS_TOTAL, "backend" => backend, "decision" => "raised")
            .increment(1);
    } else if result.processed > 0 {
        ::metrics::counter!(METRIC_INTERRUPTS_TOTAL, "backend" => backend, "decision" => "suppressed")
            .increment(1);
    }
    ::metrics::histogram!(METRIC_QUEUE_DRAIN_DURATION_MS, "backend" => backend)
        .record(duration_ms(result.drain_duration));
}

fn log_block_ioerr(
    operation: &'static str,
    sector: u64,
    bytes: u64,
    reason: &'static str,
    error: Option<&std::io::Error>,
) {
    tracing::warn!(
        event_name = "virtio.blk.ioerr",
        operation,
        sector,
        bytes,
        reason,
        error = error.map(tracing::field::display),
        "virtio-blk request failed"
    );
}

fn request_operation_label(type_: u32) -> &'static str {
    match type_ {
        VIRTIO_BLK_T_IN => "read",
        VIRTIO_BLK_T_OUT => "write",
        VIRTIO_BLK_T_GET_ID => "get_id",
        VIRTIO_BLK_T_DISCARD => "discard",
        _ => "unsupported",
    }
}

fn request_status_label(status: u8) -> &'static str {
    match status {
        VIRTIO_BLK_S_OK => "ok",
        VIRTIO_BLK_S_IOERR => "ioerr",
        VIRTIO_BLK_S_UNSUPP => "unsupported",
        _ => "unknown",
    }
}

impl VirtioDevice for VirtioBlockDevice {
    fn device_type(&self) -> u32 {
        VIRTIO_ID_BLOCK
    }

    fn features(&self) -> u64 {
        let mut f = VIRTIO_F_VERSION_1 | VIRTIO_RING_F_EVENT_IDX;
        if self.read_only {
            f |= VIRTIO_BLK_F_RO;
        } else {
            f |= VIRTIO_BLK_F_DISCARD;
        }
        f
    }

    fn queue_max_sizes(&self) -> &[u16] {
        &[QUEUE_SIZE]
    }

    fn read_config(&self, offset: u64, data: &mut [u8]) {
        let mut config = [0_u8; 48];
        config[0..8].copy_from_slice(&self.capacity_sectors.to_le_bytes());
        if !self.read_only {
            let max_discard_sectors = self.capacity_sectors.min(u32::MAX as u64) as u32;
            config[36..40].copy_from_slice(&max_discard_sectors.to_le_bytes());
            config[40..44].copy_from_slice(&32_u32.to_le_bytes());
            config[44..48].copy_from_slice(&1_u32.to_le_bytes());
        }

        for (i, byte) in data.iter_mut().enumerate() {
            *byte = config.get(offset as usize + i).copied().unwrap_or_default();
        }
    }

    fn write_config(&self, _offset: u64, _data: &[u8]) {
        // Block device config is read-only
    }

    fn activate(&mut self, mem: GuestMemoryRef, queues: &[QueueConfig]) {
        if let Some(q) = queues.first() {
            if q.size > 0 {
                let queue = if q.warm_restore {
                    VirtQueue::new_restored_with_event_idx(
                        mem.clone(),
                        q.desc_addr,
                        q.driver_addr,
                        q.device_addr,
                        q.size,
                        q.event_idx,
                    )
                } else {
                    VirtQueue::new_with_event_idx(
                        mem.clone(),
                        q.desc_addr,
                        q.driver_addr,
                        q.device_addr,
                        q.size,
                        q.event_idx,
                    )
                };

                if let (Some(irq_fd), Some(interrupt_status), Some(notify_fd)) = (
                    self.irq_fd,
                    self.interrupt_status.as_ref().cloned(),
                    self.notify_fd.as_ref(),
                ) {
                    match (self.file.try_clone(), dup_owned_fd(notify_fd.as_raw_fd())) {
                        (Ok(file), Ok(worker_notify_fd)) => {
                            let (tx, rx) = mpsc::channel();
                            let read_only = self.read_only;
                            let capacity_sectors = self.capacity_sectors;
                            let device_id = self.device_id;
                            let worker_mem = mem.clone();
                            let handle = std::thread::Builder::new()
                                .name("virtio-blk-ioeventfd".into())
                                .spawn(move || {
                                    block_worker_loop(BlockWorker {
                                        file,
                                        read_only,
                                        capacity_sectors,
                                        device_id,
                                        mem: worker_mem,
                                        queue,
                                        notify_fd: worker_notify_fd,
                                        rx,
                                        irq_fd,
                                        interrupt_status,
                                    })
                                })
                                .expect("failed to spawn virtio-blk ioeventfd worker");
                            self.control_tx = Some(tx);
                            self.worker_handle = Some(handle);
                            self.queue = None;
                        }
                        (file_result, notify_result) => {
                            tracing::warn!(
                                event_name = "virtio.blk.worker_disabled",
                                file_error = ?file_result.err(),
                                notify_error = ?notify_result.err(),
                                "virtio-blk ioeventfd worker disabled"
                            );
                            self.queue = Some(queue);
                        }
                    }
                } else {
                    self.queue = Some(queue);
                }
            }
        }
        self.mem = Some(mem);
    }

    fn queue_notify(&mut self, queue_index: u32) -> bool {
        if queue_index != 0 {
            tracing::warn!(
                event_name = "virtio.blk.queue_notify_ignored",
                queue_index,
                "virtio-blk ignored notification for unknown queue"
            );
            return false;
        }

        let mut queue = match self.queue.take() {
            Some(q) => q,
            None => {
                tracing::warn!(
                    event_name = "virtio.blk.queue_notify_unconfigured",
                    "virtio-blk notified before queue was configured"
                );
                return false;
            }
        };

        let mem = match self.mem.as_ref() {
            Some(mem) => mem,
            None => return false,
        };
        emit_queue_notification_metric("mmio", 1);
        let result = Self::process_queue(
            &mut self.file,
            self.read_only,
            self.capacity_sectors,
            &self.device_id,
            mem,
            &mut queue,
        );
        emit_queue_drain_metrics("mmio", &result);

        self.queue = Some(queue);
        tracing::trace!(
            event_name = "virtio.blk.queue_drain",
            backend = "mmio",
            processed = result.processed,
            used_entries = result.used_entries,
            should_interrupt = result.should_interrupt,
            read_ops = result.read_ops,
            write_ops = result.write_ops,
            bytes_read = result.bytes_read,
            bytes_written = result.bytes_written,
            duration_ms = duration_ms(result.drain_duration),
            "virtio-blk queue notification drained"
        );
        result.should_interrupt
    }

    fn quiesce(&mut self) -> Result<()> {
        let Some(tx) = self.control_tx.as_ref() else {
            return Ok(());
        };
        let Some(notify_fd) = self.notify_fd.as_ref() else {
            return Ok(());
        };
        let (done_tx, done_rx) = mpsc::channel();
        let started = Instant::now();
        tx.send(BlockWorkerCommand::Drain(done_tx))
            .context("send virtio-blk drain command")?;
        write_eventfd(notify_fd.as_raw_fd()).context("wake virtio-blk worker for drain")?;
        let result = done_rx
            .recv_timeout(Duration::from_secs(2))
            .context("wait for virtio-blk drain");
        ::metrics::histogram!(METRIC_QUIESCE_DRAIN_DURATION_MS, "backend" => "ioeventfd")
            .record(duration_ms(started.elapsed()));
        result.map(|_| ())
    }

    fn uses_mmio_interrupt(&self) -> bool {
        self.control_tx.is_none()
    }
}

impl Drop for VirtioBlockDevice {
    fn drop(&mut self) {
        if let (Some(tx), Some(notify_fd)) = (self.control_tx.take(), self.notify_fd.as_ref()) {
            let _ = tx.send(BlockWorkerCommand::Stop);
            let _ = write_eventfd(notify_fd.as_raw_fd());
        }
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}

struct BlockWorker {
    file: std::fs::File,
    read_only: bool,
    capacity_sectors: u64,
    device_id: [u8; VIRTIO_BLK_ID_LEN],
    mem: GuestMemoryRef,
    queue: VirtQueue,
    notify_fd: OwnedFd,
    rx: mpsc::Receiver<BlockWorkerCommand>,
    irq_fd: RawFd,
    interrupt_status: Arc<AtomicU32>,
}

fn block_worker_loop(worker: BlockWorker) {
    if !should_use_io_uring(worker.read_only) {
        block_worker_loop_sync(worker);
        return;
    }

    match BlockIoUring::new(worker.file.as_raw_fd()) {
        Ok(uring) => block_worker_loop_uring(worker, uring),
        Err(error) => {
            tracing::warn!(
                event_name = "virtio.blk.io_uring_disabled",
                %error,
                "virtio-blk io_uring backend unavailable; using synchronous worker"
            );
            block_worker_loop_sync(worker);
        }
    }
}

fn should_use_io_uring(read_only: bool) -> bool {
    // The first measured io_uring slice improved scratch sequential reads but
    // regressed read-only rootfs and AI CLI startup. Keep rootfs on the
    // synchronous vectored path until a rootfs-specific async tune proves out.
    //
    // The writable-device gate recovered rootfs but still regressed disk
    // sequential reads, so io_uring remains opt-in while the backend matures.
    !read_only && std::env::var_os("CAPSEM_KVM_BLK_IO_URING").is_some()
}

fn block_worker_loop_sync(mut worker: BlockWorker) {
    loop {
        let notify_count = match read_eventfd(worker.notify_fd.as_raw_fd()) {
            Ok(count) => count,
            Err(error) => {
                tracing::warn!(
                    event_name = "virtio.blk.ioeventfd_read_failed",
                    %error,
                    "virtio-blk worker failed to read notify eventfd"
                );
                return;
            }
        };
        emit_queue_notification_metric("ioeventfd", notify_count);

        let mut stop = false;
        let mut drain_replies = Vec::new();
        while let Ok(command) = worker.rx.try_recv() {
            match command {
                BlockWorkerCommand::Drain(done) => drain_replies.push(done),
                BlockWorkerCommand::Stop => stop = true,
            }
        }

        let result = VirtioBlockDevice::process_queue(
            &mut worker.file,
            worker.read_only,
            worker.capacity_sectors,
            &worker.device_id,
            &worker.mem,
            &mut worker.queue,
        );
        emit_queue_drain_metrics("ioeventfd", &result);
        if result.should_interrupt {
            signal_irq(worker.irq_fd, &worker.interrupt_status);
        }
        for done in drain_replies {
            let _ = done.send(());
        }
        tracing::trace!(
            event_name = "virtio.blk.queue_drain",
            backend = "ioeventfd",
            notify_count,
            processed = result.processed,
            used_entries = result.used_entries,
            should_interrupt = result.should_interrupt,
            read_ops = result.read_ops,
            write_ops = result.write_ops,
            bytes_read = result.bytes_read,
            bytes_written = result.bytes_written,
            duration_ms = duration_ms(result.drain_duration),
            "virtio-blk ioeventfd worker drained queue notification"
        );

        if stop {
            return;
        }
    }
}

const EPOLL_TOKEN_NOTIFY: u64 = 1;
const EPOLL_TOKEN_COMPLETION: u64 = 2;

fn block_worker_loop_uring(mut worker: BlockWorker, mut uring: BlockIoUring) {
    let epoll_fd = match create_epoll_fd() {
        Ok(fd) => fd,
        Err(error) => {
            tracing::warn!(
                event_name = "virtio.blk.io_uring_epoll_failed",
                %error,
                "virtio-blk io_uring worker could not create epoll fd"
            );
            return;
        }
    };
    if let Err(error) = epoll_add(
        epoll_fd.as_raw_fd(),
        worker.notify_fd.as_raw_fd(),
        EPOLL_TOKEN_NOTIFY,
    )
    .and_then(|_| {
        epoll_add(
            epoll_fd.as_raw_fd(),
            uring.completion_fd(),
            EPOLL_TOKEN_COMPLETION,
        )
    }) {
        tracing::warn!(
            event_name = "virtio.blk.io_uring_epoll_failed",
            %error,
            "virtio-blk io_uring worker could not register eventfds"
        );
        return;
    }

    let mut stop = false;
    let mut drain_replies = Vec::new();
    loop {
        let events = match epoll_wait_tokens(epoll_fd.as_raw_fd()) {
            Ok(events) => events,
            Err(error) => {
                tracing::warn!(
                    event_name = "virtio.blk.io_uring_epoll_failed",
                    %error,
                    "virtio-blk io_uring epoll wait failed"
                );
                return;
            }
        };

        for token in events {
            match token {
                EPOLL_TOKEN_NOTIFY => {
                    let notify_count = match read_eventfd(worker.notify_fd.as_raw_fd()) {
                        Ok(count) => count,
                        Err(error) => {
                            tracing::warn!(
                                event_name = "virtio.blk.ioeventfd_read_failed",
                                %error,
                                "virtio-blk io_uring worker failed to read notify eventfd"
                            );
                            return;
                        }
                    };
                    emit_queue_notification_metric("io_uring", notify_count);

                    while let Ok(command) = worker.rx.try_recv() {
                        match command {
                            BlockWorkerCommand::Drain(done) => drain_replies.push(done),
                            BlockWorkerCommand::Stop => stop = true,
                        }
                    }

                    let result = VirtioBlockDevice::process_queue_uring(
                        &mut worker.file,
                        worker.read_only,
                        worker.capacity_sectors,
                        &worker.device_id,
                        &worker.mem,
                        &mut worker.queue,
                        &mut uring,
                    );
                    emit_queue_drain_metrics("io_uring", &result);
                    if result.should_interrupt {
                        signal_irq(worker.irq_fd, &worker.interrupt_status);
                    }
                    tracing::trace!(
                        event_name = "virtio.blk.queue_drain",
                        backend = "io_uring",
                        notify_count,
                        processed = result.processed,
                        submitted = result.submitted,
                        used_entries = result.used_entries,
                        in_flight = uring.pending_len(),
                        should_interrupt = result.should_interrupt,
                        read_ops = result.read_ops,
                        write_ops = result.write_ops,
                        bytes_read = result.bytes_read,
                        bytes_written = result.bytes_written,
                        duration_ms = duration_ms(result.drain_duration),
                        "virtio-blk io_uring worker drained queue notification"
                    );
                }
                EPOLL_TOKEN_COMPLETION => {
                    let _ = drain_eventfd(uring.completion_fd());
                    let completion = uring.reap_completions(&worker.mem, &mut worker.queue);
                    if completion.should_interrupt {
                        signal_irq(worker.irq_fd, &worker.interrupt_status);
                    }
                    tracing::trace!(
                        event_name = "virtio.blk.async_completions",
                        backend = "io_uring",
                        completed = completion.completed,
                        used_entries = completion.used_entries,
                        in_flight = uring.pending_len(),
                        should_interrupt = completion.should_interrupt,
                        read_ops = completion.read_ops,
                        write_ops = completion.write_ops,
                        bytes_read = completion.bytes_read,
                        bytes_written = completion.bytes_written,
                        "virtio-blk io_uring completions reaped"
                    );
                }
                _ => {}
            }
        }

        if uring.pending_len() == 0 {
            for done in drain_replies.drain(..) {
                let _ = done.send(());
            }
            if stop {
                return;
            }
        }
    }
}

fn dup_owned_fd(fd: RawFd) -> std::io::Result<OwnedFd> {
    let duped = unsafe { libc::dup(fd) };
    if duped < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(duped) })
}

fn create_eventfd(flags: libc::c_int) -> std::io::Result<OwnedFd> {
    let fd = unsafe { libc::eventfd(0, flags) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn create_epoll_fd() -> std::io::Result<OwnedFd> {
    let fd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn epoll_add(epoll_fd: RawFd, fd: RawFd, token: u64) -> std::io::Result<()> {
    let mut event = libc::epoll_event {
        events: libc::EPOLLIN as u32,
        u64: token,
    };
    let ret = unsafe { libc::epoll_ctl(epoll_fd, libc::EPOLL_CTL_ADD, fd, &mut event) };
    if ret < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn epoll_wait_tokens(epoll_fd: RawFd) -> std::io::Result<Vec<u64>> {
    let mut events = [libc::epoll_event { events: 0, u64: 0 }; 8];
    loop {
        let n = unsafe { libc::epoll_wait(epoll_fd, events.as_mut_ptr(), events.len() as i32, -1) };
        if n >= 0 {
            return Ok(events[..n as usize].iter().map(|event| event.u64).collect());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return Err(error);
    }
}

fn read_eventfd(fd: RawFd) -> std::io::Result<u64> {
    let mut val = 0_u64;
    loop {
        let ret = unsafe {
            libc::read(
                fd,
                &mut val as *mut u64 as *mut libc::c_void,
                std::mem::size_of::<u64>(),
            )
        };
        if ret == std::mem::size_of::<u64>() as isize {
            return Ok(val);
        }
        if ret < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(error);
        }
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "short eventfd read",
        ));
    }
}

fn drain_eventfd(fd: RawFd) -> std::io::Result<Option<u64>> {
    match read_eventfd(fd) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.raw_os_error() == Some(libc::EAGAIN) => Ok(None),
        Err(error) => Err(error),
    }
}

fn write_eventfd(fd: RawFd) -> std::io::Result<()> {
    let val = 1_u64;
    loop {
        let ret = unsafe {
            libc::write(
                fd,
                &val as *const u64 as *const libc::c_void,
                std::mem::size_of::<u64>(),
            )
        };
        if ret == std::mem::size_of::<u64>() as isize {
            return Ok(());
        }
        if ret < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(error);
        }
        return Err(std::io::Error::new(
            std::io::ErrorKind::WriteZero,
            "short eventfd write",
        ));
    }
}

fn signal_irq(irq_fd: RawFd, interrupt_status: &AtomicU32) {
    interrupt_status.fetch_or(1, Ordering::SeqCst);
    let val: u64 = 1;
    let ret = unsafe { libc::write(irq_fd, &val as *const u64 as *const libc::c_void, 8) };
    if ret < 0 {
        tracing::warn!(
            event_name = "virtio.blk.irq_signal_failed",
            error = %std::io::Error::last_os_error(),
            "failed to signal virtio-blk interrupt eventfd"
        );
    }
}

#[cfg(test)]
mod tests;
