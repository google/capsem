//! VirtioFS device (type 26) -- embedded FUSE-over-virtio filesystem server.
//!
//! FUSE request processing runs on a dedicated worker thread, not on the vCPU.
//! `queue_notify` signals the worker via a channel; the worker processes the
//! queue and injects an IRQ into the guest via irqfd.
//!
//! Handler implementations are split across submodules:
//! - `ops_meta`: INIT, LOOKUP, GETATTR, SETATTR, STATFS, FORGET
//! - `ops_file`: OPEN, READ, WRITE, CREATE, RELEASE, FLUSH, FSYNC, LSEEK
//! - `ops_dir`:  OPENDIR, READDIR, MKDIR, RMDIR, UNLINK, RENAME, MKNOD, SYMLINK, LINK

mod checkpoint_state;
mod ops_dir;
mod ops_file;
mod ops_meta;

use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{ensure, Context, Result};
use tracing::{debug, trace, warn};

use super::memory::GuestMemoryRef;
use super::virtio_mmio::{QueueConfig, VirtioDevice};
use super::virtio_queue::{DescriptorChain, VirtQueue};

use crate::hypervisor::fuse::{self, *};
use checkpoint_state::VirtioFsBackendSnapshot;

const VIRTIO_ID_FS: u32 = 26;
const VIRTIO_F_VERSION_1: u64 = 1 << 32;
const QUEUE_SIZE: u16 = 256;
const TAG_LEN: usize = 36;

/// Maximum single-read size (matches FUSE_INIT max_read negotiation).
pub(super) const MAX_READ_SIZE: u32 = 1 << 20; // 1 MB

/// Maximum gathered request buffer size (header + max_write + margin).
const MAX_GATHER_SIZE: usize = 2 * 1024 * 1024; // 2 MB

// ---------------------------------------------------------------------------
// FuseProcessor -- owns all FUSE protocol state
// ---------------------------------------------------------------------------

/// FUSE processing state: inodes, file handles, and policy flags.
///
/// All `do_*` handlers in ops_meta/ops_file/ops_dir are methods on this struct.
/// Before activation, owned by `VirtioFsDevice`; after activation, moved to
/// the worker thread.
pub(super) struct FuseProcessor {
    pub(super) root_path: PathBuf,
    pub(super) read_only: bool,
    pub(super) inodes: InodeTable,
    pub(super) file_handles: FileHandleTable,
}

impl FuseProcessor {
    fn handle_request(&mut self, request_buf: &[u8]) -> Vec<u8> {
        let header: FuseInHeader = match fuse::read_struct(request_buf) {
            Some(h) => h,
            None => return fuse::error_response(0, -libc::EIO),
        };
        let body = &request_buf[std::mem::size_of::<FuseInHeader>()..];

        match header.opcode {
            FUSE_INIT => self.do_init(&header, body),
            FUSE_LOOKUP => self.do_lookup(&header, body),
            FUSE_GETATTR => self.do_getattr(&header),
            FUSE_SETATTR => self.do_setattr(&header, body),
            FUSE_OPEN => self.do_open(&header, body),
            FUSE_READ => self.do_read(&header, body),
            FUSE_WRITE => self.do_write(&header, body),
            FUSE_RELEASE => self.do_release(&header, body),
            FUSE_CREATE => self.do_create(&header, body),
            FUSE_MKDIR => self.do_mkdir(&header, body),
            FUSE_UNLINK => self.do_unlink(&header, body),
            FUSE_RMDIR => self.do_rmdir(&header, body),
            FUSE_RENAME => self.do_rename(&header, body),
            FUSE_RENAME2 => self.do_rename2(&header, body),
            FUSE_OPENDIR => self.do_opendir(&header),
            FUSE_READDIR => self.do_readdir(&header, body),
            FUSE_RELEASEDIR => self.do_releasedir(&header, body),
            FUSE_STATFS => self.do_statfs(&header),
            FUSE_FLUSH => self.do_flush(&header, body),
            FUSE_FSYNC => self.do_fsync(&header, body),
            FUSE_FSYNCDIR => self.do_fsyncdir(&header, body),
            FUSE_FORGET => {
                self.do_forget(&header, body);
                Vec::new()
            }
            FUSE_BATCH_FORGET => {
                self.do_batch_forget(body);
                Vec::new()
            }
            FUSE_MKNOD => self.do_mknod(&header, body),
            FUSE_SYMLINK => self.do_symlink(&header, body),
            FUSE_READLINK => self.do_readlink(&header),
            FUSE_LINK => self.do_link(&header, body),
            FUSE_LSEEK => self.do_lseek(&header, body),
            _ => fuse::error_response(header.unique, -libc::ENOSYS),
        }
    }

    fn checkpoint_snapshot(&mut self, tag: [u8; TAG_LEN]) -> Result<VirtioFsBackendSnapshot> {
        let file_handles = self.file_handles.checkpoint(&self.inodes)?;
        let handle_inodes = file_handles.handles.iter().map(|handle| handle.inode).collect();
        let inodes = self.inodes.checkpoint(&handle_inodes)?;
        Ok(VirtioFsBackendSnapshot {
            tag,
            read_only: self.read_only,
            inodes,
            file_handles,
        })
    }

    fn encode_checkpoint_with_tag(&mut self, tag: [u8; TAG_LEN]) -> Result<Vec<u8>> {
        self.checkpoint_snapshot(tag)?.encode()
    }

    #[cfg(test)]
    fn encode_checkpoint(&mut self) -> Result<Vec<u8>> {
        self.encode_checkpoint_with_tag([0; TAG_LEN])
    }

    fn restore_snapshot(root_path: &Path, read_only: bool, snapshot: &VirtioFsBackendSnapshot) -> Result<Self> {
        ensure!(
            snapshot.read_only == read_only,
            "VirtioFS checkpoint read-only identity mismatch"
        );
        let inodes = InodeTable::restore(root_path, &snapshot.inodes)?;
        let file_handles = FileHandleTable::restore(&snapshot.file_handles, &inodes, read_only)?;
        Ok(Self {
            root_path: root_path.to_path_buf(),
            read_only,
            inodes,
            file_handles,
        })
    }

    #[cfg(test)]
    fn restore_checkpoint(root_path: &Path, read_only: bool, encoded: &[u8]) -> Result<Self> {
        let snapshot = VirtioFsBackendSnapshot::decode(encoded)?;
        Self::restore_snapshot(root_path, read_only, &snapshot)
    }
}

// ---------------------------------------------------------------------------
// Gather / scatter (standalone, no state needed)
// ---------------------------------------------------------------------------

/// Gather readable descriptor data into a contiguous buffer.
/// Returns `None` if the total exceeds `MAX_GATHER_SIZE` (protocol violation).
fn gather_readable(mem: &GuestMemoryRef, chain: &DescriptorChain) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    for desc in &chain.descriptors {
        if !desc.is_write_only() {
            let new_len = buf.len() + desc.len as usize;
            if new_len > MAX_GATHER_SIZE {
                return None;
            }
            if let Some(ptr) = mem.gpa_to_host(desc.addr) {
                buf.extend_from_slice(unsafe { std::slice::from_raw_parts(ptr.cast_const(), desc.len as usize) });
            }
        }
    }
    Some(buf)
}

fn write_response(mem: &GuestMemoryRef, chain: &DescriptorChain, data: &[u8]) -> u32 {
    if data.is_empty() {
        return 0;
    }
    let mut offset = 0usize;
    for desc in &chain.descriptors {
        if desc.is_write_only() && offset < data.len() {
            if let Some(ptr) = mem.gpa_to_host(desc.addr) {
                let n = (data.len() - offset).min(desc.len as usize);
                unsafe {
                    std::ptr::copy_nonoverlapping(data[offset..].as_ptr(), ptr, n);
                }
                offset += n;
            }
        }
    }
    offset as u32
}

// ---------------------------------------------------------------------------
// Worker thread
// ---------------------------------------------------------------------------

enum WorkerCommand {
    Notify(u32),
    Checkpoint {
        tag: [u8; TAG_LEN],
        done: mpsc::Sender<Result<Vec<u8>>>,
    },
}

fn worker_loop(
    mut proc: FuseProcessor,
    mut request_queue: VirtQueue,
    mut hiprio_queue: VirtQueue,
    mem: GuestMemoryRef,
    rx: mpsc::Receiver<WorkerCommand>,
    irq_fd: RawFd,
    interrupt_status: Arc<AtomicU32>,
) {
    debug!(event_name = "virtio.fs.worker_start", "virtio-fs worker started");
    let mut hiprio_processed_total = 0u64;
    let mut request_processed_total = 0u64;
    while let Ok(command) = rx.recv() {
        match command {
            WorkerCommand::Notify(0) => {
                let processed = drain_hiprio_queue(&mut proc, &mut hiprio_queue, &mem);
                hiprio_processed_total = hiprio_processed_total.saturating_add(u64::from(processed));
                let should_interrupt = hiprio_queue.prepare_kick();
                log_queue_drain("hiprio", processed, should_interrupt);
                if should_interrupt {
                    signal_irq(irq_fd, &interrupt_status);
                }
            }
            WorkerCommand::Notify(1) => {
                let processed = drain_request_queue(&mut proc, &mut request_queue, &mem);
                request_processed_total = request_processed_total.saturating_add(u64::from(processed));
                let should_interrupt = request_queue.prepare_kick();
                log_queue_drain("request", processed, should_interrupt);
                if should_interrupt {
                    signal_irq(irq_fd, &interrupt_status);
                }
            }
            WorkerCommand::Notify(_) => {}
            WorkerCommand::Checkpoint { tag, done } => {
                let hiprio = drain_hiprio_queue(&mut proc, &mut hiprio_queue, &mem);
                let request = drain_request_queue(&mut proc, &mut request_queue, &mem);
                hiprio_processed_total = hiprio_processed_total.saturating_add(u64::from(hiprio));
                request_processed_total = request_processed_total.saturating_add(u64::from(request));
                let hiprio_interrupt = hiprio_queue.prepare_kick();
                let request_interrupt = request_queue.prepare_kick();
                if hiprio_interrupt || request_interrupt {
                    signal_irq(irq_fd, &interrupt_status);
                }
                debug!(
                    event_name = "virtio.fs.quiesce",
                    hiprio_processed = hiprio,
                    request_processed = request,
                    hiprio_processed_total,
                    request_processed_total,
                    hiprio_should_interrupt = hiprio_interrupt,
                    request_should_interrupt = request_interrupt,
                    "virtio-fs queues quiesced"
                );
                let checkpoint = proc.encode_checkpoint_with_tag(tag);
                let _ = done.send(checkpoint);
            }
        }
    }
    debug!("virtio-fs worker exiting");
}

fn drain_hiprio_queue(proc: &mut FuseProcessor, hiprio_queue: &mut VirtQueue, mem: &GuestMemoryRef) -> u32 {
    // High-priority queue: FORGET ops (fire-and-forget, no response)
    let mut processed = 0u32;
    while let Some(chain) = hiprio_queue.pop() {
        processed += 1;
        let buf = gather_readable(mem, &chain).unwrap_or_default();
        if let Some(header) = fuse::read_struct::<FuseInHeader>(&buf) {
            let body = &buf[std::mem::size_of::<FuseInHeader>()..];
            trace!(
                event_name = "virtio.fs.request",
                queue = "hiprio",
                opcode = header.opcode,
                unique = header.unique,
                "virtio-fs FUSE request"
            );
            match header.opcode {
                FUSE_FORGET => proc.do_forget(&header, body),
                FUSE_BATCH_FORGET => proc.do_batch_forget(body),
                _ => {}
            }
        }
        hiprio_queue.push_used(chain.head, 0);
    }
    processed
}

fn drain_request_queue(proc: &mut FuseProcessor, request_queue: &mut VirtQueue, mem: &GuestMemoryRef) -> u32 {
    // Request queue: full FUSE operations
    let mut processed = 0u32;
    while let Some(chain) = request_queue.pop() {
        processed += 1;
        let request_buf = match gather_readable(mem, &chain) {
            Some(buf) => buf,
            None => {
                let response = fuse::error_response(0, -libc::ENOMEM);
                let written = write_response(mem, &chain, &response);
                request_queue.push_used(chain.head, written);
                continue;
            }
        };
        if let Some(header) = fuse::read_struct::<FuseInHeader>(&request_buf) {
            trace!(
                event_name = "virtio.fs.request",
                queue = "request",
                opcode = header.opcode,
                unique = header.unique,
                "virtio-fs FUSE request"
            );
        }
        let response = proc.handle_request(&request_buf);
        let written = write_response(mem, &chain, &response);
        request_queue.push_used(chain.head, written);
    }
    processed
}

fn log_queue_drain(queue: &'static str, processed: u32, should_interrupt: bool) {
    trace!(
        event_name = "virtio.fs.queue_drain",
        queue,
        processed,
        should_interrupt,
        "virtio-fs queue drained"
    );
}

fn signal_irq(irq_fd: RawFd, interrupt_status: &AtomicU32) {
    interrupt_status.fetch_or(1, Ordering::SeqCst);
    let val: u64 = 1;
    let ret = unsafe { libc::write(irq_fd, &val as *const u64 as *const libc::c_void, 8) };
    if ret < 0 {
        warn!(
            event_name = "virtio.fs.irq_signal_failed",
            error = %std::io::Error::last_os_error(),
            "failed to signal virtio-fs interrupt eventfd"
        );
    }
}

// ---------------------------------------------------------------------------
// VirtioFsDevice -- thin VirtioDevice wrapper + worker management
// ---------------------------------------------------------------------------

pub(in crate::hypervisor::kvm) struct VirtioFsDevice {
    tag: [u8; TAG_LEN],
    /// FUSE state: present before activation, moved to worker on activate.
    processor: Option<FuseProcessor>,
    /// Channel to signal the worker thread.
    notify_tx: Option<mpsc::Sender<WorkerCommand>>,
    /// Worker thread handle (joined on drop).
    worker_handle: Option<std::thread::JoinHandle<()>>,
    /// Eventfd wired to the guest GIC for interrupt injection.
    irq_fd: RawFd,
    interrupt_status: Arc<AtomicU32>,
    checkpoint_state: Option<Vec<u8>>,
}

impl VirtioFsDevice {
    pub fn new(
        tag: &str,
        root_path: &Path,
        read_only: bool,
        irq_fd: RawFd,
        interrupt_status: Arc<AtomicU32>,
    ) -> Result<Self> {
        let mut tag_buf = [0u8; TAG_LEN];
        let len = tag.len().min(TAG_LEN);
        tag_buf[..len].copy_from_slice(&tag.as_bytes()[..len]);

        Ok(Self {
            tag: tag_buf,
            processor: Some(FuseProcessor {
                root_path: root_path.to_path_buf(),
                read_only,
                inodes: InodeTable::new(root_path)?,
                file_handles: FileHandleTable::new(),
            }),
            notify_tx: None,
            worker_handle: None,
            irq_fd,
            interrupt_status,
            checkpoint_state: None,
        })
    }
}

impl Drop for VirtioFsDevice {
    fn drop(&mut self) {
        // Drop the sender first so the worker's recv() returns Err.
        self.notify_tx.take();
        // Then join the worker thread for clean shutdown.
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}

impl VirtioFsDevice {
    fn activate_fallible(&mut self, mem: GuestMemoryRef, queues: &[QueueConfig]) -> Result<()> {
        let hiprio_config = queues
            .first()
            .filter(|queue| queue.size > 0)
            .context("VirtioFS hiprio queue is unavailable for activation")?;
        let hiprio_queue = if hiprio_config.warm_restore {
            VirtQueue::new_restored(
                mem.clone(),
                hiprio_config.desc_addr,
                hiprio_config.driver_addr,
                hiprio_config.device_addr,
                hiprio_config.size,
            )
        } else {
            VirtQueue::new(
                mem.clone(),
                hiprio_config.desc_addr,
                hiprio_config.driver_addr,
                hiprio_config.device_addr,
                hiprio_config.size,
            )
        };
        let request_config = queues
            .get(1)
            .filter(|queue| queue.size > 0)
            .context("VirtioFS request queue is unavailable for activation")?;
        let request_queue = if request_config.warm_restore {
            VirtQueue::new_restored(
                mem.clone(),
                request_config.desc_addr,
                request_config.driver_addr,
                request_config.device_addr,
                request_config.size,
            )
        } else {
            VirtQueue::new(
                mem.clone(),
                request_config.desc_addr,
                request_config.driver_addr,
                request_config.device_addr,
                request_config.size,
            )
        };

        let processor = self
            .processor
            .take()
            .context("VirtioFS processor is unavailable for activation")?;
        let (notify_tx, notify_rx) = mpsc::channel();
        let irq_fd = self.irq_fd;
        let interrupt_status = Arc::clone(&self.interrupt_status);
        let handle = std::thread::Builder::new()
            .name("virtio-fs-worker".into())
            .spawn(move || {
                worker_loop(
                    processor,
                    request_queue,
                    hiprio_queue,
                    mem,
                    notify_rx,
                    irq_fd,
                    interrupt_status,
                )
            })
            .context("spawn virtio-fs worker")?;
        self.notify_tx = Some(notify_tx);
        self.worker_handle = Some(handle);
        debug!(event_name = "virtio.fs.activate", "virtio-fs device activated");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// VirtioDevice trait impl
// ---------------------------------------------------------------------------

impl VirtioDevice for VirtioFsDevice {
    fn device_type(&self) -> u32 {
        VIRTIO_ID_FS
    }
    fn features(&self) -> u64 {
        VIRTIO_F_VERSION_1
    }
    fn queue_max_sizes(&self) -> &[u16] {
        &[QUEUE_SIZE, QUEUE_SIZE]
    }

    fn read_config(&self, offset: u64, data: &mut [u8]) {
        for (i, byte) in data.iter_mut().enumerate() {
            let co = offset as usize + i;
            if co < TAG_LEN {
                *byte = self.tag[co];
            } else if co < TAG_LEN + 4 {
                *byte = 1u32.to_le_bytes()[co - TAG_LEN];
            } else {
                *byte = 0;
            }
        }
    }

    fn write_config(&self, _offset: u64, _data: &[u8]) {}

    fn activate(&mut self, mem: GuestMemoryRef, queues: &[QueueConfig]) {
        if let Err(error) = self.activate_fallible(mem, queues) {
            warn!(
                event_name = "virtio.fs.activate_failed",
                error = format!("{error:#}"),
                "virtio-fs activation failed"
            );
        }
    }

    fn restore_activate(&mut self, mem: GuestMemoryRef, queues: &[QueueConfig]) -> Result<()> {
        self.activate_fallible(mem, queues)
    }

    fn queue_notify(&mut self, queue_index: u32) -> bool {
        trace!(
            event_name = "virtio.fs.queue_notify",
            queue_index,
            "virtio-fs queue notified"
        );
        let send_failed = self
            .notify_tx
            .as_ref()
            .is_some_and(|tx| tx.send(WorkerCommand::Notify(queue_index)).is_err());
        if send_failed {
            warn!(
                event_name = "virtio.fs.queue_notify_failed",
                queue_index, "virtio-fs worker unavailable; notification dropped"
            );
            self.notify_tx = None;
        }
        false
    }

    fn quiesce(&mut self) -> Result<()> {
        let Some(tx) = self.notify_tx.as_ref() else {
            let processor = self
                .processor
                .as_mut()
                .context("VirtioFS processor is unavailable for checkpoint")?;
            self.checkpoint_state = Some(processor.encode_checkpoint_with_tag(self.tag)?);
            return Ok(());
        };
        let (done_tx, done_rx) = mpsc::channel();
        tx.send(WorkerCommand::Checkpoint {
            tag: self.tag,
            done: done_tx,
        })
        .context("send virtio-fs quiesce command")?;
        self.checkpoint_state = Some(
            done_rx
                .recv_timeout(Duration::from_secs(2))
                .context("wait for virtio-fs quiesce")??,
        );
        Ok(())
    }

    fn checkpoint_state(&mut self) -> Result<Vec<u8>> {
        if let Some(state) = self.checkpoint_state.as_ref() {
            return Ok(state.clone());
        }
        let processor = self
            .processor
            .as_mut()
            .context("VirtioFS must be quiesced before checkpoint state is read")?;
        processor.encode_checkpoint_with_tag(self.tag)
    }

    fn restore_checkpoint_state(&mut self, encoded: &[u8]) -> Result<()> {
        ensure!(
            self.notify_tx.is_none() && self.worker_handle.is_none(),
            "cannot restore VirtioFS backend state after activation"
        );
        let snapshot = VirtioFsBackendSnapshot::decode(encoded)?;
        ensure!(snapshot.tag == self.tag, "VirtioFS checkpoint tag identity mismatch");
        let current = self
            .processor
            .as_ref()
            .context("VirtioFS processor is unavailable for restore")?;
        ensure!(
            snapshot.read_only == current.read_only,
            "VirtioFS checkpoint read-only identity mismatch"
        );
        let restored = FuseProcessor::restore_snapshot(&current.root_path, current.read_only, &snapshot)?;
        self.processor = Some(restored);
        self.checkpoint_state = Some(encoded.to_vec());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
