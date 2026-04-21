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

mod ops_meta;
mod ops_file;
mod ops_dir;

use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use anyhow::Result;
use tracing::debug;

use super::memory::GuestMemoryRef;
use super::virtio_mmio::{QueueConfig, VirtioDevice};
use super::virtio_queue::{DescriptorChain, VirtQueue};

use crate::hypervisor::fuse::{self, *};

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
            FUSE_FORGET => { self.do_forget(&header, body); Vec::new() }
            FUSE_BATCH_FORGET => { self.do_batch_forget(body); Vec::new() }
            FUSE_MKNOD => self.do_mknod(&header, body),
            FUSE_SYMLINK => self.do_symlink(&header, body),
            FUSE_READLINK => self.do_readlink(&header),
            FUSE_LINK => self.do_link(&header, body),
            FUSE_LSEEK => self.do_lseek(&header, body),
            _ => fuse::error_response(header.unique, -libc::ENOSYS),
        }
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
                buf.extend_from_slice(unsafe {
                    std::slice::from_raw_parts(ptr as *const u8, desc.len as usize)
                });
            }
        }
    }
    Some(buf)
}

fn write_response(mem: &GuestMemoryRef, chain: &DescriptorChain, data: &[u8]) -> u32 {
    if data.is_empty() { return 0; }
    let mut offset = 0usize;
    for desc in &chain.descriptors {
        if desc.is_write_only() && offset < data.len() {
            if let Some(ptr) = mem.gpa_to_host(desc.addr) {
                let n = (data.len() - offset).min(desc.len as usize);
                unsafe { std::ptr::copy_nonoverlapping(data[offset..].as_ptr(), ptr, n); }
                offset += n;
            }
        }
    }
    offset as u32
}

// ---------------------------------------------------------------------------
// Worker thread
// ---------------------------------------------------------------------------

fn worker_loop(
    mut proc: FuseProcessor,
    mut request_queue: VirtQueue,
    mut hiprio_queue: VirtQueue,
    mem: GuestMemoryRef,
    rx: mpsc::Receiver<u32>,
    irq_fd: RawFd,
) {
    while let Ok(queue_index) = rx.recv() {
        match queue_index {
            0 => {
                // High-priority queue: FORGET ops (fire-and-forget, no response)
                while let Some(chain) = hiprio_queue.pop() {
                    let buf = gather_readable(&mem, &chain).unwrap_or_default();
                    if let Some(header) = fuse::read_struct::<FuseInHeader>(&buf) {
                        let body = &buf[std::mem::size_of::<FuseInHeader>()..];
                        match header.opcode {
                            FUSE_FORGET => proc.do_forget(&header, body),
                            FUSE_BATCH_FORGET => proc.do_batch_forget(body),
                            _ => {}
                        }
                    }
                    hiprio_queue.push_used(chain.head, 0);
                }
                signal_irq(irq_fd);
            }
            1 => {
                // Request queue: full FUSE operations
                while let Some(chain) = request_queue.pop() {
                    let request_buf = match gather_readable(&mem, &chain) {
                        Some(buf) => buf,
                        None => {
                            let response = fuse::error_response(0, -libc::ENOMEM);
                            let written = write_response(&mem, &chain, &response);
                            request_queue.push_used(chain.head, written);
                            continue;
                        }
                    };
                    let response = proc.handle_request(&request_buf);
                    let written = write_response(&mem, &chain, &response);
                    request_queue.push_used(chain.head, written);
                }
                signal_irq(irq_fd);
            }
            _ => {}
        }
    }
    debug!("virtio-fs worker exiting");
}

fn signal_irq(irq_fd: RawFd) {
    let val: u64 = 1;
    unsafe {
        libc::write(irq_fd, &val as *const u64 as *const libc::c_void, 8);
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
    notify_tx: Option<mpsc::Sender<u32>>,
    /// Worker thread handle (joined on drop).
    worker_handle: Option<std::thread::JoinHandle<()>>,
    /// Eventfd wired to the guest GIC for interrupt injection.
    irq_fd: RawFd,
}

impl VirtioFsDevice {
    pub fn new(tag: &str, root_path: &Path, read_only: bool, irq_fd: RawFd) -> Result<Self> {
        let mut tag_buf = [0u8; TAG_LEN];
        let len = tag.as_bytes().len().min(TAG_LEN);
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

// ---------------------------------------------------------------------------
// VirtioDevice trait impl
// ---------------------------------------------------------------------------

impl VirtioDevice for VirtioFsDevice {
    fn device_type(&self) -> u32 { VIRTIO_ID_FS }
    fn features(&self) -> u64 { VIRTIO_F_VERSION_1 }
    fn queue_max_sizes(&self) -> &[u16] { &[QUEUE_SIZE, QUEUE_SIZE] }

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
        let hiprio_queue = match queues.get(0).filter(|q| q.size > 0) {
            Some(q) => VirtQueue::new(
                mem.clone(), q.desc_addr, q.driver_addr, q.device_addr, q.size,
            ),
            None => return,
        };
        let request_queue = match queues.get(1).filter(|q| q.size > 0) {
            Some(q) => VirtQueue::new(
                mem.clone(), q.desc_addr, q.driver_addr, q.device_addr, q.size,
            ),
            None => return,
        };

        // Transfer FUSE state to worker thread
        let proc = match self.processor.take() {
            Some(p) => p,
            None => return,
        };

        let (tx, rx) = mpsc::channel();
        self.notify_tx = Some(tx);

        let irq_fd = self.irq_fd;
        let handle = std::thread::Builder::new()
            .name("virtio-fs-worker".into())
            .spawn(move || worker_loop(proc, request_queue, hiprio_queue, mem, rx, irq_fd))
            .expect("failed to spawn virtio-fs worker");
        self.worker_handle = Some(handle);
    }

    fn queue_notify(&mut self, queue_index: u32) {
        if let Some(ref tx) = self.notify_tx {
            let _ = tx.send(queue_index);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
