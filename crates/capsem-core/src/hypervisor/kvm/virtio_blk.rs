//! Virtio block device (type 2) for disk I/O.
//!
//! File-backed block device with one requestq. Supports read, write,
//! get-ID, and discard operations. Read-only mode enforced via feature bit
//! and write/discard rejection.

use std::collections::HashMap;
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Once};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use capsem_proto::metrics::VmBlockMetrics;
use io_uring::{opcode, register::Restriction, squeue, types, IoUring, Probe};
use metrics::{describe_counter, describe_histogram, Unit};

use super::memory::GuestMemoryRef;
use super::virtio_mmio::{QueueConfig, VirtioDevice};
use super::virtio_queue::{VirtQueue, VIRTIO_RING_F_EVENT_IDX};

/// Virtio block device ID.
const VIRTIO_ID_BLOCK: u32 = 2;

/// Maximum queue size for the requestq.
const DEFAULT_QUEUE_SIZE: u16 = 256;
const DEFAULT_QUEUE_COUNT: u16 = 1;
const MAX_QUEUE_COUNT: u16 = 16;
const MIN_QUEUE_SIZE: u16 = 16;
const MAX_QUEUE_SIZE: u16 = 1024;
/// Keep the host async ring bounded below the guest-visible queue so descriptors
/// can backpressure cleanly instead of letting host work grow without a cap.
const IO_URING_QUEUE_SIZE: u32 = 128;
const IO_URING_FIXED_FILE_INDEX: u32 = 0;

/// Sector size in bytes.
const SECTOR_SIZE: u64 = 512;

/// Maximum device ID length (virtio spec).
const VIRTIO_BLK_ID_LEN: usize = 20;
const DEFAULT_LOGICAL_BLOCK_SIZE: u32 = SECTOR_SIZE as u32;

/// Size of one virtio discard segment.
const DISCARD_SEGMENT_SIZE: usize = 16;

// Feature bits
const VIRTIO_BLK_F_SEG_MAX: u64 = 1 << 2;
const VIRTIO_BLK_F_RO: u64 = 1 << 5;
const VIRTIO_BLK_F_BLK_SIZE: u64 = 1 << 6;
const VIRTIO_BLK_F_MQ: u64 = 1 << 12;
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
const METRIC_ASYNC_QUEUE_FULL_TOTAL: &str = "virtio.blk.async_queue_full_total";
const METRIC_ASYNC_IN_FLIGHT: &str = "virtio.blk.async_in_flight";

static DESCRIBE_METRICS: Once = Once::new();

#[derive(Default)]
pub(super) struct BlockDeviceMetrics {
    queue_notifications_total: AtomicU64,
    queue_drains_total: AtomicU64,
    descriptors_drained_total: AtomicU64,
    used_entries_total: AtomicU64,
    interrupts_raised_total: AtomicU64,
    interrupts_suppressed_total: AtomicU64,
    read_ops_total: AtomicU64,
    write_ops_total: AtomicU64,
    bytes_read_total: AtomicU64,
    bytes_written_total: AtomicU64,
    async_submissions_total: AtomicU64,
    async_completions_total: AtomicU64,
    async_fallbacks_total: AtomicU64,
    async_queue_full_total: AtomicU64,
    async_in_flight: AtomicU64,
}

impl BlockDeviceMetrics {
    pub(super) fn snapshot(&self) -> VmBlockMetrics {
        VmBlockMetrics {
            queue_notifications_total: self.queue_notifications_total.load(Ordering::Relaxed),
            queue_drains_total: self.queue_drains_total.load(Ordering::Relaxed),
            descriptors_drained_total: self.descriptors_drained_total.load(Ordering::Relaxed),
            used_entries_total: self.used_entries_total.load(Ordering::Relaxed),
            interrupts_raised_total: self.interrupts_raised_total.load(Ordering::Relaxed),
            interrupts_suppressed_total: self.interrupts_suppressed_total.load(Ordering::Relaxed),
            read_ops_total: self.read_ops_total.load(Ordering::Relaxed),
            write_ops_total: self.write_ops_total.load(Ordering::Relaxed),
            bytes_read_total: self.bytes_read_total.load(Ordering::Relaxed),
            bytes_written_total: self.bytes_written_total.load(Ordering::Relaxed),
            async_submissions_total: self.async_submissions_total.load(Ordering::Relaxed),
            async_completions_total: self.async_completions_total.load(Ordering::Relaxed),
            async_fallbacks_total: self.async_fallbacks_total.load(Ordering::Relaxed),
            async_queue_full_total: self.async_queue_full_total.load(Ordering::Relaxed),
            async_in_flight: self.async_in_flight.load(Ordering::Relaxed),
        }
    }

    fn record_queue_notification(&self, count: u64) {
        self.queue_notifications_total
            .fetch_add(count, Ordering::Relaxed);
    }

    fn record_queue_drain(&self, result: &QueueProcessResult) {
        self.queue_drains_total.fetch_add(1, Ordering::Relaxed);
        self.descriptors_drained_total
            .fetch_add(result.processed as u64, Ordering::Relaxed);
        self.used_entries_total
            .fetch_add(result.used_entries as u64, Ordering::Relaxed);
        if result.should_interrupt {
            self.interrupts_raised_total.fetch_add(1, Ordering::Relaxed);
        } else if result.processed > 0 {
            self.interrupts_suppressed_total
                .fetch_add(1, Ordering::Relaxed);
        }
        self.read_ops_total
            .fetch_add(result.read_ops as u64, Ordering::Relaxed);
        self.write_ops_total
            .fetch_add(result.write_ops as u64, Ordering::Relaxed);
        self.bytes_read_total
            .fetch_add(result.bytes_read, Ordering::Relaxed);
        self.bytes_written_total
            .fetch_add(result.bytes_written, Ordering::Relaxed);
        self.async_submissions_total
            .fetch_add(result.submitted as u64, Ordering::Relaxed);
        self.async_fallbacks_total
            .fetch_add(result.async_fallbacks as u64, Ordering::Relaxed);
        self.async_queue_full_total
            .fetch_add(result.async_queue_full as u64, Ordering::Relaxed);
    }

    fn record_async_completion(&self, completion: &CompletionResult, in_flight: usize) {
        self.async_completions_total
            .fetch_add(completion.completed as u64, Ordering::Relaxed);
        self.used_entries_total
            .fetch_add(completion.used_entries as u64, Ordering::Relaxed);
        if completion.should_interrupt {
            self.interrupts_raised_total.fetch_add(1, Ordering::Relaxed);
        } else if completion.completed > 0 {
            self.interrupts_suppressed_total
                .fetch_add(1, Ordering::Relaxed);
        }
        self.read_ops_total
            .fetch_add(completion.read_ops as u64, Ordering::Relaxed);
        self.write_ops_total
            .fetch_add(completion.write_ops as u64, Ordering::Relaxed);
        self.bytes_read_total
            .fetch_add(completion.bytes_read, Ordering::Relaxed);
        self.bytes_written_total
            .fetch_add(completion.bytes_written, Ordering::Relaxed);
        self.async_in_flight
            .store(in_flight.try_into().unwrap_or(u64::MAX), Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockShape {
    queue_count: u16,
    queue_size: u16,
    seg_max: u32,
    logical_block_size: u32,
}

impl BlockShape {
    fn from_env() -> Result<Self> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup<F>(lookup: F) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let queue_count =
            parse_u16_env(&lookup, "CAPSEM_KVM_BLK_QUEUE_COUNT", DEFAULT_QUEUE_COUNT)?;
        if !(1..=MAX_QUEUE_COUNT).contains(&queue_count) {
            anyhow::bail!(
                "CAPSEM_KVM_BLK_QUEUE_COUNT must be between 1 and {MAX_QUEUE_COUNT}, got {queue_count}"
            );
        }

        let queue_size = parse_u16_env(&lookup, "CAPSEM_KVM_BLK_QUEUE_SIZE", DEFAULT_QUEUE_SIZE)?;
        if !(MIN_QUEUE_SIZE..=MAX_QUEUE_SIZE).contains(&queue_size) || !queue_size.is_power_of_two()
        {
            anyhow::bail!(
                "CAPSEM_KVM_BLK_QUEUE_SIZE must be a power of two between {MIN_QUEUE_SIZE} and {MAX_QUEUE_SIZE}, got {queue_size}"
            );
        }

        let max_seg = u32::from(queue_size) - 2;
        let seg_max = parse_u32_env(&lookup, "CAPSEM_KVM_BLK_SEG_MAX", max_seg)?;
        if seg_max == 0 || seg_max > max_seg {
            anyhow::bail!(
                "CAPSEM_KVM_BLK_SEG_MAX must be between 1 and queue_size - 2 ({max_seg}), got {seg_max}"
            );
        }

        let logical_block_size = parse_u32_env(
            &lookup,
            "CAPSEM_KVM_BLK_LOGICAL_BLOCK_SIZE",
            DEFAULT_LOGICAL_BLOCK_SIZE,
        )?;
        if !(SECTOR_SIZE as u32..=4096).contains(&logical_block_size)
            || !logical_block_size.is_power_of_two()
            || logical_block_size % (SECTOR_SIZE as u32) != 0
        {
            anyhow::bail!(
                "CAPSEM_KVM_BLK_LOGICAL_BLOCK_SIZE must be a power-of-two multiple of 512 between 512 and 4096, got {logical_block_size}"
            );
        }

        Ok(Self {
            queue_count,
            queue_size,
            seg_max,
            logical_block_size,
        })
    }

    fn queue_sizes(&self) -> Vec<u16> {
        vec![self.queue_size; self.queue_count as usize]
    }
}

fn parse_u16_env<F>(lookup: &F, name: &str, default: u16) -> Result<u16>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(raw) = lookup(name) else {
        return Ok(default);
    };
    raw.parse::<u16>()
        .with_context(|| format!("parse {name}={raw:?} as u16"))
}

fn parse_u32_env<F>(lookup: &F, name: &str, default: u32) -> Result<u32>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(raw) = lookup(name) else {
        return Ok(default);
    };
    raw.parse::<u32>()
        .with_context(|| format!("parse {name}={raw:?} as u32"))
}

/// Virtio block device backed by a file.
pub(super) struct VirtioBlockDevice {
    file: std::fs::File,
    read_only: bool,
    capacity_sectors: u64,
    device_id: [u8; VIRTIO_BLK_ID_LEN],
    shape: BlockShape,
    queue_sizes: Vec<u16>,
    queues: Vec<Option<VirtQueue>>,
    mem: Option<GuestMemoryRef>,
    irq_fd: Option<RawFd>,
    interrupt_status: Option<Arc<AtomicU32>>,
    notify_fds: Vec<OwnedFd>,
    control_txs: Vec<mpsc::Sender<BlockWorkerCommand>>,
    worker_handles: Vec<std::thread::JoinHandle<()>>,
    metrics: Arc<BlockDeviceMetrics>,
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
        Self::new_with_shape(path, read_only, BlockShape::from_env()?)
    }

    fn new_with_shape(path: &Path, read_only: bool, shape: BlockShape) -> Result<Self> {
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

        let queue_sizes = shape.queue_sizes();
        tracing::debug!(
            event_name = "virtio.blk.shape",
            read_only,
            queue_count = shape.queue_count,
            queue_size = shape.queue_size,
            seg_max = shape.seg_max,
            logical_block_size = shape.logical_block_size,
            "virtio-blk shape selected"
        );

        Ok(Self {
            file,
            read_only,
            capacity_sectors,
            device_id,
            shape,
            queue_sizes,
            queues: Vec::new(),
            mem: None,
            irq_fd: None,
            interrupt_status: None,
            notify_fds: Vec::new(),
            control_txs: Vec::new(),
            worker_handles: Vec::new(),
            metrics: Arc::new(BlockDeviceMetrics::default()),
        })
    }

    pub(super) fn metrics(&self) -> Arc<BlockDeviceMetrics> {
        Arc::clone(&self.metrics)
    }

    pub(super) fn queue_count(&self) -> usize {
        self.queue_sizes.len()
    }

    fn stop_workers(&mut self) {
        let had_workers = !self.control_txs.is_empty();
        for tx in self.control_txs.drain(..) {
            let _ = tx.send(BlockWorkerCommand::Stop);
        }
        if had_workers {
            for notify_fd in &self.notify_fds {
                let _ = write_eventfd(notify_fd.as_raw_fd());
            }
        }
        for handle in self.worker_handles.drain(..) {
            let _ = handle.join();
        }
    }

    pub fn with_async_notify(
        mut self,
        irq_fd: RawFd,
        interrupt_status: Arc<AtomicU32>,
        notify_fds: Vec<OwnedFd>,
    ) -> Self {
        self.irq_fd = Some(irq_fd);
        self.interrupt_status = Some(interrupt_status);
        self.notify_fds = notify_fds;
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
            None => return VIRTIO_BLK_S_IOERR,
        };

        let total_len: u64 = data_descs.iter().map(|&(_, l)| l as u64).sum();
        if offset
            .checked_add(total_len)
            .is_none_or(|end| end > capacity_sectors * SECTOR_SIZE)
        {
            return VIRTIO_BLK_S_IOERR;
        }

        let iovecs = match Self::guest_iovecs(mem, data_descs) {
            Some(iovecs) => iovecs,
            None => return VIRTIO_BLK_S_IOERR,
        };
        if Self::preadv_all(file.as_raw_fd(), &iovecs, offset, total_len).is_ok() {
            VIRTIO_BLK_S_OK
        } else {
            VIRTIO_BLK_S_IOERR
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
            return VIRTIO_BLK_S_IOERR;
        }

        let offset = match sector.checked_mul(SECTOR_SIZE) {
            Some(o) => o,
            None => return VIRTIO_BLK_S_IOERR,
        };

        let total_len: u64 = data_descs.iter().map(|&(_, l)| l as u64).sum();
        if offset
            .checked_add(total_len)
            .is_none_or(|end| end > capacity_sectors * SECTOR_SIZE)
        {
            return VIRTIO_BLK_S_IOERR;
        }

        let iovecs = match Self::guest_iovecs(mem, data_descs) {
            Some(iovecs) => iovecs,
            None => return VIRTIO_BLK_S_IOERR,
        };
        if Self::pwritev_all(file.as_raw_fd(), &iovecs, offset, total_len).is_ok() {
            VIRTIO_BLK_S_OK
        } else {
            VIRTIO_BLK_S_IOERR
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

    fn total_data_len(data_descs: &[(u64, u32)]) -> Option<u32> {
        data_descs
            .iter()
            .try_fold(0_u32, |acc, &(_, len)| acc.checked_add(len))
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
            let total_data = match Self::total_data_len(&data_descs) {
                Some(total_data) => total_data,
                None => {
                    tracing::warn!(
                        event_name = "virtio.blk.request_malformed",
                        head = chain.head,
                        "virtio-blk request data length overflowed u32"
                    );
                    Self::write_status(mem, status_desc.addr, VIRTIO_BLK_S_IOERR);
                    queue.push_used_deferred(chain.head, 1);
                    used_entries += 1;
                    continue;
                }
            };

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
            async_fallbacks: 0,
            async_queue_full: 0,
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
            let total_data = match Self::total_data_len(&data_descs) {
                Some(total_data) => total_data,
                None => {
                    tracing::warn!(
                        event_name = "virtio.blk.request_malformed",
                        head = chain.head,
                        "virtio-blk request data length overflowed u32"
                    );
                    Self::write_status(mem, status_desc.addr, VIRTIO_BLK_S_IOERR);
                    queue.push_used_deferred(chain.head, 1);
                    result.used_entries += 1;
                    continue;
                }
            };

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

                    match uring.submit_rw(
                        chain.head,
                        type_,
                        total_data,
                        status_desc.addr,
                        offset,
                        iovecs,
                    ) {
                        Ok(()) => {
                            result.submitted += 1;
                            if type_ == VIRTIO_BLK_T_IN {
                                result.read_ops += 1;
                            } else {
                                result.write_ops += 1;
                            }
                            continue;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            ::metrics::counter!(
                                METRIC_ASYNC_QUEUE_FULL_TOTAL,
                                "operation" => request_operation_label(type_),
                            )
                            .increment(1);
                            result.async_queue_full += 1;
                            queue.undo_pop();
                            break;
                        }
                        Err(error) => {
                            tracing::warn!(
                                event_name = "virtio.blk.io_uring_submit_failed",
                                %error,
                                operation = request_operation_label(type_),
                                "virtio-blk io_uring submit failed; using synchronous fallback"
                            );
                        }
                    }

                    ::metrics::counter!(
                        METRIC_ASYNC_FALLBACKS_TOTAL,
                        "operation" => request_operation_label(type_),
                    )
                    .increment(1);
                    result.async_fallbacks += 1;
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

    #[allow(clippy::too_many_arguments)]
    fn reap_completions_and_retry(
        file: &mut std::fs::File,
        read_only: bool,
        capacity_sectors: u64,
        device_id: &[u8; VIRTIO_BLK_ID_LEN],
        mem: &GuestMemoryRef,
        queue: &mut VirtQueue,
        uring: &mut BlockIoUring,
    ) -> CompletionRetryResult {
        let completion = uring.reap_completions(mem, queue);
        let drain = if completion.completed > 0 {
            Self::process_queue_uring(
                file,
                read_only,
                capacity_sectors,
                device_id,
                mem,
                queue,
                uring,
            )
        } else {
            QueueProcessResult::new(Instant::now())
        };
        CompletionRetryResult { completion, drain }
    }
}

struct QueueProcessResult {
    processed: u32,
    submitted: u32,
    async_fallbacks: u32,
    async_queue_full: u32,
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
            async_fallbacks: 0,
            async_queue_full: 0,
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

struct CompletionRetryResult {
    completion: CompletionResult,
    drain: QueueProcessResult,
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
    fixed_file_index: u32,
    restrictions_enabled: bool,
}

impl BlockIoUring {
    fn new(file_fd: RawFd) -> std::io::Result<Self> {
        let completion_fd = create_eventfd(libc::EFD_CLOEXEC | libc::EFD_NONBLOCK)?;
        let ring = IoUring::builder()
            .setup_r_disabled()
            .build(IO_URING_QUEUE_SIZE)?;
        let submitter = ring.submitter();

        let mut probe = Probe::new();
        submitter.register_probe(&mut probe)?;
        require_uring_opcode(&probe, opcode::Readv::CODE, "READV")?;
        require_uring_opcode(&probe, opcode::Writev::CODE, "WRITEV")?;

        submitter.register_eventfd(completion_fd.as_raw_fd())?;
        submitter.register_files(&[file_fd])?;
        let mut restrictions = [
            Restriction::sqe_op(opcode::Readv::CODE),
            Restriction::sqe_op(opcode::Writev::CODE),
            Restriction::sqe_flags_required(squeue::Flags::FIXED_FILE.bits()),
        ];
        submitter.register_restrictions(&mut restrictions)?;
        submitter.register_enable_rings()?;
        Ok(Self {
            ring,
            completion_fd,
            pending: HashMap::new(),
            next_user_data: 1,
            fixed_file_index: IO_URING_FIXED_FILE_INDEX,
            restrictions_enabled: true,
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
            VIRTIO_BLK_T_IN => {
                opcode::Readv::new(types::Fixed(self.fixed_file_index), iovec_ptr, iovec_len)
                    .offset(offset)
                    .build()
                    .user_data(user_data)
            }
            VIRTIO_BLK_T_OUT => {
                opcode::Writev::new(types::Fixed(self.fixed_file_index), iovec_ptr, iovec_len)
                    .offset(offset)
                    .build()
                    .user_data(user_data)
            }
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
        ::metrics::counter!(
            METRIC_ASYNC_SUBMISSIONS_TOTAL,
            "operation" => request_operation_label(type_),
        )
        .increment(1);
        ::metrics::histogram!(METRIC_ASYNC_IN_FLIGHT, "backend" => "io_uring")
            .record(self.pending.len() as f64);
        Ok(())
    }

    fn kick_submission_queue(&mut self) -> std::io::Result<usize> {
        loop {
            match self.ring.submit() {
                Ok(submitted) => return Ok(submitted),
                Err(error) if error.raw_os_error() == Some(libc::EINTR) => continue,
                Err(error) => return Err(error),
            }
        }
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

fn require_uring_opcode(probe: &Probe, opcode: u8, name: &'static str) -> std::io::Result<()> {
    if probe.is_supported(opcode) {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!("io_uring opcode {name} is not supported"),
        ))
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
        describe_counter!(
            METRIC_ASYNC_QUEUE_FULL_TOTAL,
            Unit::Count,
            "Virtio block io_uring submissions deferred because the submission queue was full."
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
        let mut f = VIRTIO_F_VERSION_1
            | VIRTIO_RING_F_EVENT_IDX
            | VIRTIO_BLK_F_SEG_MAX
            | VIRTIO_BLK_F_BLK_SIZE;
        if self.queue_sizes.len() > 1 {
            f |= VIRTIO_BLK_F_MQ;
        }
        if self.read_only {
            f |= VIRTIO_BLK_F_RO;
        } else {
            f |= VIRTIO_BLK_F_DISCARD;
        }
        f
    }

    fn queue_max_sizes(&self) -> &[u16] {
        &self.queue_sizes
    }

    fn read_config(&self, offset: u64, data: &mut [u8]) {
        let mut config = [0_u8; 48];
        config[0..8].copy_from_slice(&self.capacity_sectors.to_le_bytes());
        config[12..16].copy_from_slice(&self.shape.seg_max.to_le_bytes());
        config[20..24].copy_from_slice(&self.shape.logical_block_size.to_le_bytes());
        config[34..36].copy_from_slice(&self.shape.queue_count.to_le_bytes());
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
        self.stop_workers();
        self.queues.clear();

        let use_async_notify = self.notify_fds.len() == self.queue_sizes.len()
            && self.irq_fd.is_some()
            && self.interrupt_status.is_some();
        for (queue_index, q) in queues.iter().enumerate().take(self.queue_sizes.len()) {
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

                if use_async_notify {
                    let irq_fd = self.irq_fd.expect("checked above");
                    let interrupt_status = self
                        .interrupt_status
                        .as_ref()
                        .cloned()
                        .expect("checked above");
                    let notify_fd = &self.notify_fds[queue_index];
                    match (self.file.try_clone(), dup_owned_fd(notify_fd.as_raw_fd())) {
                        (Ok(file), Ok(worker_notify_fd)) => {
                            let (tx, rx) = mpsc::channel();
                            let read_only = self.read_only;
                            let capacity_sectors = self.capacity_sectors;
                            let device_id = self.device_id;
                            let worker_mem = mem.clone();
                            let metrics = Arc::clone(&self.metrics);
                            let handle = std::thread::Builder::new()
                                .name(format!("virtio-blk-q{queue_index}"))
                                .spawn(move || {
                                    block_worker_loop(
                                        file,
                                        read_only,
                                        capacity_sectors,
                                        device_id,
                                        worker_mem,
                                        queue,
                                        worker_notify_fd,
                                        rx,
                                        irq_fd,
                                        interrupt_status,
                                        metrics,
                                    )
                                })
                                .expect("failed to spawn virtio-blk ioeventfd worker");
                            self.control_txs.push(tx);
                            self.worker_handles.push(handle);
                            self.queues.push(None);
                        }
                        (file_result, notify_result) => {
                            tracing::warn!(
                                event_name = "virtio.blk.worker_disabled",
                                queue_index,
                                file_error = ?file_result.err(),
                                notify_error = ?notify_result.err(),
                                "virtio-blk ioeventfd worker disabled"
                            );
                            self.queues.push(Some(queue));
                        }
                    }
                } else {
                    self.queues.push(Some(queue));
                }
            } else {
                self.queues.push(None);
            }
        }
        self.mem = Some(mem);
    }

    fn queue_notify(&mut self, queue_index: u32) -> bool {
        let Some(queue_slot) = self.queues.get_mut(queue_index as usize) else {
            tracing::warn!(
                event_name = "virtio.blk.queue_notify_ignored",
                queue_index,
                "virtio-blk ignored notification for unknown queue"
            );
            return false;
        };

        let mut queue = match queue_slot.take() {
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
        self.metrics.record_queue_notification(1);
        let result = Self::process_queue(
            &mut self.file,
            self.read_only,
            self.capacity_sectors,
            &self.device_id,
            mem,
            &mut queue,
        );
        emit_queue_drain_metrics("mmio", &result);
        self.metrics.record_queue_drain(&result);

        *queue_slot = Some(queue);
        tracing::trace!(
            event_name = "virtio.blk.queue_drain",
            backend = "mmio",
            queue_index,
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
        if self.control_txs.is_empty() {
            return Ok(());
        }
        let started = Instant::now();
        let mut done_rxs = Vec::with_capacity(self.control_txs.len());
        for tx in &self.control_txs {
            let (done_tx, done_rx) = mpsc::channel();
            tx.send(BlockWorkerCommand::Drain(done_tx))
                .context("send virtio-blk drain command")?;
            done_rxs.push(done_rx);
        }
        for notify_fd in &self.notify_fds {
            write_eventfd(notify_fd.as_raw_fd()).context("wake virtio-blk worker for drain")?;
        }
        let mut result = Ok(());
        for done_rx in done_rxs {
            if let Err(error) = done_rx
                .recv_timeout(Duration::from_secs(2))
                .context("wait for virtio-blk drain")
            {
                result = Err(error);
                break;
            }
        }
        ::metrics::histogram!(METRIC_QUIESCE_DRAIN_DURATION_MS, "backend" => "ioeventfd")
            .record(duration_ms(started.elapsed()));
        result
    }

    fn uses_mmio_interrupt(&self) -> bool {
        self.notify_fds.is_empty()
    }
}

impl Drop for VirtioBlockDevice {
    fn drop(&mut self) {
        self.stop_workers();
    }
}

fn block_worker_loop(
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
    metrics: Arc<BlockDeviceMetrics>,
) {
    if !should_use_io_uring(read_only) {
        block_worker_loop_sync(
            file,
            read_only,
            capacity_sectors,
            device_id,
            mem,
            queue,
            notify_fd,
            rx,
            irq_fd,
            interrupt_status,
            Arc::clone(&metrics),
        );
        return;
    }

    match BlockIoUring::new(file.as_raw_fd()) {
        Ok(uring) => block_worker_loop_uring(
            file,
            read_only,
            capacity_sectors,
            device_id,
            mem,
            queue,
            notify_fd,
            rx,
            irq_fd,
            interrupt_status,
            uring,
            Arc::clone(&metrics),
        ),
        Err(error) => {
            tracing::warn!(
                event_name = "virtio.blk.io_uring_disabled",
                %error,
                "virtio-blk io_uring backend unavailable; using synchronous worker"
            );
            block_worker_loop_sync(
                file,
                read_only,
                capacity_sectors,
                device_id,
                mem,
                queue,
                notify_fd,
                rx,
                irq_fd,
                interrupt_status,
                metrics,
            );
        }
    }
}

fn should_use_io_uring(read_only: bool) -> bool {
    let _ = read_only;
    !matches!(
        std::env::var("CAPSEM_KVM_BLK_IO_URING")
            .ok()
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("0" | "false" | "off" | "sync")
    )
}

fn block_worker_loop_sync(
    mut file: std::fs::File,
    read_only: bool,
    capacity_sectors: u64,
    device_id: [u8; VIRTIO_BLK_ID_LEN],
    mem: GuestMemoryRef,
    mut queue: VirtQueue,
    notify_fd: OwnedFd,
    rx: mpsc::Receiver<BlockWorkerCommand>,
    irq_fd: RawFd,
    interrupt_status: Arc<AtomicU32>,
    metrics: Arc<BlockDeviceMetrics>,
) {
    loop {
        let notify_count = match read_eventfd(notify_fd.as_raw_fd()) {
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
        metrics.record_queue_notification(notify_count);

        let mut stop = false;
        let mut drain_replies = Vec::new();
        while let Ok(command) = rx.try_recv() {
            match command {
                BlockWorkerCommand::Drain(done) => drain_replies.push(done),
                BlockWorkerCommand::Stop => stop = true,
            }
        }

        let result = VirtioBlockDevice::process_queue(
            &mut file,
            read_only,
            capacity_sectors,
            &device_id,
            &mem,
            &mut queue,
        );
        emit_queue_drain_metrics("ioeventfd", &result);
        metrics.record_queue_drain(&result);
        if result.should_interrupt {
            signal_irq(irq_fd, &interrupt_status);
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

#[allow(clippy::too_many_arguments)]
fn block_worker_loop_uring(
    mut file: std::fs::File,
    read_only: bool,
    capacity_sectors: u64,
    device_id: [u8; VIRTIO_BLK_ID_LEN],
    mem: GuestMemoryRef,
    mut queue: VirtQueue,
    notify_fd: OwnedFd,
    rx: mpsc::Receiver<BlockWorkerCommand>,
    irq_fd: RawFd,
    interrupt_status: Arc<AtomicU32>,
    mut uring: BlockIoUring,
    metrics: Arc<BlockDeviceMetrics>,
) {
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
        notify_fd.as_raw_fd(),
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
                    let notify_count = match read_eventfd(notify_fd.as_raw_fd()) {
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
                    metrics.record_queue_notification(notify_count);

                    while let Ok(command) = rx.try_recv() {
                        match command {
                            BlockWorkerCommand::Drain(done) => drain_replies.push(done),
                            BlockWorkerCommand::Stop => stop = true,
                        }
                    }

                    let result = VirtioBlockDevice::process_queue_uring(
                        &mut file,
                        read_only,
                        capacity_sectors,
                        &device_id,
                        &mem,
                        &mut queue,
                        &mut uring,
                    );
                    if result.submitted > 0 {
                        if let Err(error) = uring.kick_submission_queue() {
                            tracing::warn!(
                                event_name = "virtio.blk.io_uring_submit_failed",
                                %error,
                                submitted = result.submitted,
                                "virtio-blk io_uring batch submit failed"
                            );
                        }
                    }
                    emit_queue_drain_metrics("io_uring", &result);
                    metrics.record_queue_drain(&result);
                    if result.should_interrupt {
                        signal_irq(irq_fd, &interrupt_status);
                    }
                    tracing::trace!(
                        event_name = "virtio.blk.queue_drain",
                        backend = "io_uring",
                        notify_count,
                        processed = result.processed,
                        submitted = result.submitted,
                        async_queue_full = result.async_queue_full,
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
                    let retry = VirtioBlockDevice::reap_completions_and_retry(
                        &mut file,
                        read_only,
                        capacity_sectors,
                        &device_id,
                        &mem,
                        &mut queue,
                        &mut uring,
                    );
                    metrics.record_async_completion(&retry.completion, uring.pending_len());
                    if retry.completion.should_interrupt {
                        signal_irq(irq_fd, &interrupt_status);
                    }
                    tracing::trace!(
                        event_name = "virtio.blk.async_completions",
                        backend = "io_uring",
                        completed = retry.completion.completed,
                        used_entries = retry.completion.used_entries,
                        in_flight = uring.pending_len(),
                        should_interrupt = retry.completion.should_interrupt,
                        read_ops = retry.completion.read_ops,
                        write_ops = retry.completion.write_ops,
                        bytes_read = retry.completion.bytes_read,
                        bytes_written = retry.completion.bytes_written,
                        "virtio-blk io_uring completions reaped"
                    );
                    if retry.drain.processed > 0 || retry.drain.async_queue_full > 0 {
                        if retry.drain.submitted > 0 {
                            if let Err(error) = uring.kick_submission_queue() {
                                tracing::warn!(
                                    event_name = "virtio.blk.io_uring_submit_failed",
                                    %error,
                                    submitted = retry.drain.submitted,
                                    "virtio-blk io_uring completion-retry batch submit failed"
                                );
                            }
                        }
                        emit_queue_drain_metrics("io_uring", &retry.drain);
                        metrics.record_queue_drain(&retry.drain);
                        if retry.drain.should_interrupt {
                            signal_irq(irq_fd, &interrupt_status);
                        }
                        tracing::trace!(
                            event_name = "virtio.blk.completion_retry",
                            backend = "io_uring",
                            processed = retry.drain.processed,
                            submitted = retry.drain.submitted,
                            async_queue_full = retry.drain.async_queue_full,
                            used_entries = retry.drain.used_entries,
                            in_flight = uring.pending_len(),
                            should_interrupt = retry.drain.should_interrupt,
                            read_ops = retry.drain.read_ops,
                            write_ops = retry.drain.write_ops,
                            bytes_read = retry.drain.bytes_read,
                            bytes_written = retry.drain.bytes_written,
                            duration_ms = duration_ms(retry.drain.drain_duration),
                            "virtio-blk retried queue after io_uring completion"
                        );
                    }
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
mod tests {
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
                .with_async_notify(irq_raw_fd, Arc::clone(&interrupt_status), vec![notify_fd]);

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
        assert_ne!(f & VIRTIO_BLK_F_SEG_MAX, 0, "must report SEG_MAX");
        assert_ne!(f & VIRTIO_BLK_F_BLK_SIZE, 0, "must report BLK_SIZE");
        assert_ne!(f & VIRTIO_BLK_F_RO, 0, "must have RO bit");
        assert_eq!(f & VIRTIO_BLK_F_MQ, 0, "single queue must not report MQ");
        assert_eq!(f & VIRTIO_BLK_F_DISCARD, 0, "RO disks must not discard");
    }

    #[test]
    fn block_features_read_write() {
        let path = temp_disk("feat-rw.img", 512);
        let dev = VirtioBlockDevice::new(&path, false).unwrap();
        let f = dev.features();
        assert_ne!(f & VIRTIO_F_VERSION_1, 0, "must have VERSION_1");
        assert_ne!(f & VIRTIO_RING_F_EVENT_IDX, 0, "must have EVENT_IDX");
        assert_ne!(f & VIRTIO_BLK_F_SEG_MAX, 0, "must report SEG_MAX");
        assert_ne!(f & VIRTIO_BLK_F_BLK_SIZE, 0, "must report BLK_SIZE");
        assert_eq!(f & VIRTIO_BLK_F_RO, 0, "must NOT have RO bit");
        assert_eq!(f & VIRTIO_BLK_F_MQ, 0, "single queue must not report MQ");
        assert_ne!(f & VIRTIO_BLK_F_DISCARD, 0, "RW disks must support discard");
    }

    #[test]
    fn block_has_one_queue() {
        let path = temp_disk("one-q.img", 512);
        let dev = VirtioBlockDevice::new(&path, false).unwrap();
        assert_eq!(dev.queue_max_sizes(), &[DEFAULT_QUEUE_SIZE]);
    }

    #[test]
    fn block_shape_supports_multi_queue_and_queue_size() {
        let path = temp_disk("mq-shape.img", 8192);
        let shape = BlockShape {
            queue_count: 4,
            queue_size: 128,
            seg_max: 64,
            logical_block_size: 4096,
        };
        let dev = VirtioBlockDevice::new_with_shape(&path, false, shape).unwrap();
        let f = dev.features();

        assert_ne!(f & VIRTIO_BLK_F_MQ, 0, "multi-queue must report MQ");
        assert_eq!(dev.queue_max_sizes(), &[128, 128, 128, 128]);
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
    fn block_config_reports_segment_limit_and_block_size() {
        let path = temp_disk("shape-cfg.img", 8192);
        let dev = VirtioBlockDevice::new(&path, false).unwrap();
        let mut seg_max = [0u8; 4];
        let mut block_size = [0u8; 4];

        dev.read_config(12, &mut seg_max);
        dev.read_config(20, &mut block_size);

        assert_eq!(u32::from_le_bytes(seg_max), DEFAULT_QUEUE_SIZE as u32 - 2);
        assert_eq!(u32::from_le_bytes(block_size), SECTOR_SIZE as u32);
    }

    #[test]
    fn block_config_reports_multi_queue_count() {
        let path = temp_disk("mq-cfg.img", 8192);
        let shape = BlockShape {
            queue_count: 8,
            queue_size: 256,
            seg_max: 128,
            logical_block_size: 512,
        };
        let dev = VirtioBlockDevice::new_with_shape(&path, false, shape).unwrap();
        let mut queues = [0u8; 2];

        dev.read_config(34, &mut queues);

        assert_eq!(u16::from_le_bytes(queues), 8);
    }

    #[test]
    fn block_shape_env_parser_validates_coupled_queue_settings() {
        let valid = |name: &str| match name {
            "CAPSEM_KVM_BLK_QUEUE_COUNT" => Some("4".to_string()),
            "CAPSEM_KVM_BLK_QUEUE_SIZE" => Some("128".to_string()),
            "CAPSEM_KVM_BLK_SEG_MAX" => Some("32".to_string()),
            "CAPSEM_KVM_BLK_LOGICAL_BLOCK_SIZE" => Some("4096".to_string()),
            _ => None,
        };
        let shape = BlockShape::from_lookup(valid).unwrap();
        assert_eq!(
            shape,
            BlockShape {
                queue_count: 4,
                queue_size: 128,
                seg_max: 32,
                logical_block_size: 4096,
            }
        );

        let invalid_seg = |name: &str| match name {
            "CAPSEM_KVM_BLK_QUEUE_SIZE" => Some("64".to_string()),
            "CAPSEM_KVM_BLK_SEG_MAX" => Some("128".to_string()),
            _ => None,
        };
        assert!(BlockShape::from_lookup(invalid_seg).is_err());
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

        let block = h.dev.metrics().snapshot();
        assert_eq!(block.queue_notifications_total, 1);
        assert_eq!(block.queue_drains_total, 1);
        assert_eq!(block.descriptors_drained_total, 1);
        assert_eq!(block.used_entries_total, 1);
        assert_eq!(block.interrupts_raised_total, 1);
        assert_eq!(block.read_ops_total, 1);
        assert_eq!(block.bytes_read_total, 512);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn block_io_uring_uses_firecracker_shaped_ring_contract() {
        let path = temp_disk("uring-contract.img", 512);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let Ok(uring) = BlockIoUring::new(file.as_raw_fd()) else {
            return;
        };

        assert_eq!(uring.fixed_file_index, IO_URING_FIXED_FILE_INDEX);
        assert!(uring.restrictions_enabled);
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
        let mut queue = h.dev.queues[0].take().unwrap();
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

    #[cfg(target_os = "linux")]
    #[test]
    fn block_io_uring_queue_full_backpressures_without_sync_fallback() {
        use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};

        let recorder = DebuggingRecorder::new();
        let snapshotter: Snapshotter = recorder.snapshotter();
        let _guard = ::metrics::set_default_local_recorder(&recorder);

        let data = vec![0xA5u8; 512];
        let path = temp_disk_with_data("read-uring-full.img", &data);
        let mut h = TestHarness::new(&path, false);
        let mut file = h.dev.file.try_clone().unwrap();
        let Ok(mut uring) = BlockIoUring::new(file.as_raw_fd()) else {
            return;
        };
        let mut queue = h.dev.queues[0].take().unwrap();
        let mem = h.dev.mem.as_ref().unwrap().clone();

        h.setup_request(VIRTIO_BLK_T_IN, 0, 512, true);
        let data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
        let status_offset = data_offset + 512;
        h.write_bytes(status_offset, &[0xAA]);

        let entry = opcode::Nop::new().build().user_data(u64::MAX);
        let mut filled_entries = 0;
        while unsafe { uring.ring.submission().push(&entry) }.is_ok() {
            filled_entries += 1;
        }
        assert!(filled_entries > 0, "test must fill the io_uring SQ");

        let result = VirtioBlockDevice::process_queue_uring(
            &mut file,
            false,
            h.dev.capacity_sectors,
            &h.dev.device_id,
            &mem,
            &mut queue,
            &mut uring,
        );
        assert_eq!(result.processed, 1);
        assert_eq!(result.submitted, 0);
        assert_eq!(result.async_queue_full, 1);
        assert_eq!(result.async_fallbacks, 0);
        assert_eq!(result.used_entries, 0);
        assert_eq!(h.read_status(status_offset), 0xAA);
        assert_eq!(h.read_used_idx(), 0);
        let metrics = BlockDeviceMetrics::default();
        metrics.record_queue_drain(&result);
        assert_eq!(metrics.snapshot().async_queue_full_total, 1);

        let snap = snapshotter.snapshot().into_vec();
        let counter_total = |name: &str| -> u64 {
            snap.iter()
                .filter_map(|(key, _, _, value)| match (key.key().name(), value) {
                    (metric, DebugValue::Counter(count)) if metric == name => Some(*count),
                    _ => None,
                })
                .sum()
        };
        assert_eq!(counter_total(METRIC_ASYNC_QUEUE_FULL_TOTAL), 1);
        assert_eq!(counter_total(METRIC_ASYNC_FALLBACKS_TOTAL), 0);

        drop(uring);
        let Ok(mut retry_uring) = BlockIoUring::new(file.as_raw_fd()) else {
            return;
        };
        let retry = VirtioBlockDevice::process_queue_uring(
            &mut file,
            false,
            h.dev.capacity_sectors,
            &h.dev.device_id,
            &mem,
            &mut queue,
            &mut retry_uring,
        );
        assert_eq!(retry.processed, 1);
        assert_eq!(retry.submitted, 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn block_io_uring_completion_retries_backpressured_descriptor() {
        let data = vec![0xA5u8; 512];
        let path = temp_disk_with_data("read-uring-retry.img", &data);
        let mut h = TestHarness::new(&path, false);
        let mut file = h.dev.file.try_clone().unwrap();
        let Ok(mut uring) = BlockIoUring::new(file.as_raw_fd()) else {
            return;
        };
        let mut queue = h.dev.queues[0].take().unwrap();
        let mem = h.dev.mem.as_ref().unwrap().clone();

        let pending_data_offset = DATA_AREA_OFFSET + 2048;
        let pending_status_offset = pending_data_offset + 512;
        let pending_iovecs =
            VirtioBlockDevice::guest_iovecs(&mem, &[(RAM_BASE + pending_data_offset, 512)])
                .unwrap();
        uring
            .submit_rw(
                7,
                VIRTIO_BLK_T_IN,
                512,
                RAM_BASE + pending_status_offset,
                0,
                pending_iovecs,
            )
            .unwrap();
        uring.kick_submission_queue().unwrap();
        uring.ring.submit_and_wait(1).unwrap();

        h.setup_request(VIRTIO_BLK_T_IN, 0, 512, true);
        let retry_data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
        let retry_status_offset = retry_data_offset + 512;
        h.write_bytes(retry_status_offset, &[0xAA]);

        let entry = opcode::Nop::new().build().user_data(u64::MAX);
        while unsafe { uring.ring.submission().push(&entry) }.is_ok() {}

        let full = VirtioBlockDevice::process_queue_uring(
            &mut file,
            false,
            h.dev.capacity_sectors,
            &h.dev.device_id,
            &mem,
            &mut queue,
            &mut uring,
        );
        assert_eq!(full.async_queue_full, 1);
        assert_eq!(full.submitted, 0);
        assert_eq!(h.read_status(retry_status_offset), 0xAA);
        uring.kick_submission_queue().unwrap();

        let retry = VirtioBlockDevice::reap_completions_and_retry(
            &mut file,
            false,
            h.dev.capacity_sectors,
            &h.dev.device_id,
            &mem,
            &mut queue,
            &mut uring,
        );

        assert_eq!(retry.completion.completed, 1);
        assert_eq!(retry.drain.processed, 1);
        assert_eq!(retry.drain.submitted, 1);
        assert_eq!(retry.drain.async_queue_full, 0);
        assert_eq!(h.read_status(pending_status_offset), VIRTIO_BLK_S_OK);
        assert_eq!(h.read_status(retry_status_offset), 0xAA);
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
    fn block_io_uring_gate_defaults_to_full_async_profile() {
        std::env::remove_var("CAPSEM_KVM_BLK_IO_URING");
        assert!(
            should_use_io_uring(true),
            "read-only rootfs participates in the full async profile"
        );
        assert!(
            should_use_io_uring(false),
            "writable block devices participate in the full async profile"
        );
        std::env::set_var("CAPSEM_KVM_BLK_IO_URING", "sync");
        assert!(
            !should_use_io_uring(false),
            "the sync escape hatch remains available for ablation and fallback"
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
    fn block_data_length_overflow_returns_ioerr() {
        let path = temp_disk("data-len-overflow.img", 512);
        let mut h = TestHarness::new(&path, true);

        let header_offset = DATA_AREA_OFFSET;
        let data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
        let status_offset = data_offset + 16;

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
            RAM_BASE + data_offset,
            u32::MAX,
            VRING_DESC_F_NEXT | VRING_DESC_F_WRITE,
            2,
        );
        h.write_desc(
            2,
            RAM_BASE + data_offset + 8,
            1,
            VRING_DESC_F_NEXT | VRING_DESC_F_WRITE,
            3,
        );
        h.write_desc(3, RAM_BASE + status_offset, 1, VRING_DESC_F_WRITE, 0);
        h.push_avail(0, 0, 1);

        h.dev.queue_notify(0);

        assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_IOERR);
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
}
