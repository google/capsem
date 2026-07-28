//! Virtio MMIO transport layer (virtio spec v1.2).
//!
//! Handles the MMIO register state machine for device discovery,
//! feature negotiation, queue setup, and activation. Dispatches
//! device-specific operations to the VirtioDevice trait.

use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Result};

use super::memory::GuestMemoryRef;
use super::mmio::MmioDevice;
use super::virtio_queue::VIRTIO_RING_F_EVENT_IDX;

// ---------------------------------------------------------------------------
// Virtio MMIO register offsets
// ---------------------------------------------------------------------------

const MAGIC_VALUE: u64 = 0x000;
const VERSION: u64 = 0x004;
const DEVICE_ID: u64 = 0x008;
const VENDOR_ID: u64 = 0x00C;
const DEVICE_FEATURES: u64 = 0x010;
const DEVICE_FEATURES_SEL: u64 = 0x014;
const DRIVER_FEATURES: u64 = 0x020;
const DRIVER_FEATURES_SEL: u64 = 0x024;
const QUEUE_SEL: u64 = 0x030;
const QUEUE_NUM_MAX: u64 = 0x034;
const QUEUE_NUM: u64 = 0x038;
const QUEUE_READY: u64 = 0x044;
const QUEUE_NOTIFY: u64 = 0x050;
pub(super) const QUEUE_NOTIFY_OFFSET: u64 = QUEUE_NOTIFY;
const INTERRUPT_STATUS: u64 = 0x060;
const INTERRUPT_ACK: u64 = 0x064;
const STATUS: u64 = 0x070;
const QUEUE_DESC_LOW: u64 = 0x080;
const QUEUE_DESC_HIGH: u64 = 0x084;
const QUEUE_DRIVER_LOW: u64 = 0x090;
const QUEUE_DRIVER_HIGH: u64 = 0x094;
const QUEUE_DEVICE_LOW: u64 = 0x0A0;
const QUEUE_DEVICE_HIGH: u64 = 0x0A4;
const CONFIG_GENERATION: u64 = 0x0FC;
const CONFIG_SPACE: u64 = 0x100;

// Virtio MMIO magic: "virt"
const VIRTIO_MMIO_MAGIC: u32 = 0x74726976;
// Virtio MMIO version 2 (modern)
const VIRTIO_MMIO_VERSION: u32 = 2;
// Vendor ID (custom for capsem)
const CAPSEM_VENDOR_ID: u32 = 0x43415053; // "CAPS"

// Status bits
const STATUS_ACKNOWLEDGE: u32 = 1;
const STATUS_DRIVER: u32 = 2;
const STATUS_FEATURES_OK: u32 = 8;
const STATUS_DRIVER_OK: u32 = 4;
const STATUS_FAILED: u32 = 128;

// ---------------------------------------------------------------------------
// VirtioDevice trait
// ---------------------------------------------------------------------------

/// Queue configuration passed to a device on activation.
///
/// Slice index matches queue number. Devices use this to construct
/// `VirtQueue` objects for descriptor chain processing.
pub(super) struct QueueConfig {
    pub desc_addr: u64,
    pub driver_addr: u64,
    pub device_addr: u64,
    pub size: u16,
    pub warm_restore: bool,
    pub event_idx: bool,
}

/// Device-specific behavior for a virtio device.
pub(super) trait VirtioDevice: Send {
    /// Device type ID (e.g., 3 for console, 2 for block, 19 for vsock).
    fn device_type(&self) -> u32;
    /// Device-offered feature bits.
    fn features(&self) -> u64;
    /// Maximum queue sizes for each queue (index = queue number).
    fn queue_max_sizes(&self) -> &[u16];
    /// Read from device-specific config space.
    fn read_config(&self, offset: u64, data: &mut [u8]);
    /// Write to device-specific config space.
    fn write_config(&self, offset: u64, data: &[u8]);
    /// Called when the driver sets DRIVER_OK. The device can now process I/O.
    ///
    /// `queues` is indexed by queue number and carries the guest-configured
    /// descriptor table, available ring, and used ring addresses.
    fn activate(&mut self, mem: GuestMemoryRef, queues: &[QueueConfig]);
    /// Called when a queue is notified (guest wrote to QUEUE_NOTIFY).
    ///
    /// Returns whether the transport should raise the used-buffer interrupt
    /// for devices that use the MMIO interrupt path. Devices that own their
    /// interrupt delivery can return false.
    fn queue_notify(&mut self, queue_index: u32) -> bool;
    /// Called while vCPUs are paused before checkpointing device/guest state.
    fn quiesce(&mut self) -> Result<()> {
        Ok(())
    }
    /// Whether the transport should raise the virtio-mmio used-buffer IRQ
    /// after queue processing. Vhost-backed devices wire their own callfd.
    fn uses_mmio_interrupt(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Queue state
// ---------------------------------------------------------------------------

struct QueueState {
    num: u16,
    ready: bool,
    desc_lo: u32,
    desc_hi: u32,
    driver_lo: u32,
    driver_hi: u32,
    device_lo: u32,
    device_hi: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QueueSnapshot {
    pub num: u16,
    pub ready: bool,
    pub desc_lo: u32,
    pub desc_hi: u32,
    pub driver_lo: u32,
    pub driver_hi: u32,
    pub device_lo: u32,
    pub device_hi: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VirtioMmioSnapshot {
    pub status: u32,
    pub features_sel: u32,
    pub driver_features: u64,
    pub driver_features_sel: u32,
    pub queue_sel: u32,
    pub queues: Vec<QueueSnapshot>,
    pub interrupt_status: u32,
    pub config_generation: u32,
    pub activated: bool,
}

impl QueueState {
    fn new() -> Self {
        Self {
            num: 0,
            ready: false,
            desc_lo: 0,
            desc_hi: 0,
            driver_lo: 0,
            driver_hi: 0,
            device_lo: 0,
            device_hi: 0,
        }
    }

    fn desc_addr(&self) -> u64 {
        (self.desc_hi as u64) << 32 | self.desc_lo as u64
    }

    fn driver_addr(&self) -> u64 {
        (self.driver_hi as u64) << 32 | self.driver_lo as u64
    }

    fn device_addr(&self) -> u64 {
        (self.device_hi as u64) << 32 | self.device_lo as u64
    }

    fn snapshot(&self) -> QueueSnapshot {
        QueueSnapshot {
            num: self.num,
            ready: self.ready,
            desc_lo: self.desc_lo,
            desc_hi: self.desc_hi,
            driver_lo: self.driver_lo,
            driver_hi: self.driver_hi,
            device_lo: self.device_lo,
            device_hi: self.device_hi,
        }
    }

    fn restore(snapshot: &QueueSnapshot) -> Self {
        Self {
            num: snapshot.num,
            ready: snapshot.ready,
            desc_lo: snapshot.desc_lo,
            desc_hi: snapshot.desc_hi,
            driver_lo: snapshot.driver_lo,
            driver_hi: snapshot.driver_hi,
            device_lo: snapshot.device_lo,
            device_hi: snapshot.device_hi,
        }
    }
}

// ---------------------------------------------------------------------------
// VirtioMmioTransport
// ---------------------------------------------------------------------------

struct TransportState {
    device: Box<dyn VirtioDevice>,
    status: u32,
    features_sel: u32,
    driver_features: u64,
    driver_features_sel: u32,
    queue_sel: u32,
    queues: Vec<QueueState>,
    interrupt_status: Arc<AtomicU32>,
    config_generation: u32,
    activated: bool,
    mem: GuestMemoryRef,
    interrupt_fd: Option<OwnedFd>,
}

/// Virtio MMIO transport wrapping a specific device.
pub(super) struct VirtioMmioTransport {
    state: Mutex<TransportState>,
}

impl VirtioMmioTransport {
    pub fn new(device: Box<dyn VirtioDevice>, mem: GuestMemoryRef) -> Self {
        let num_queues = device.queue_max_sizes().len();
        let queues = (0..num_queues).map(|_| QueueState::new()).collect();

        Self {
            state: Mutex::new(TransportState {
                device,
                status: 0,
                features_sel: 0,
                driver_features: 0,
                driver_features_sel: 0,
                queue_sel: 0,
                queues,
                interrupt_status: Arc::new(AtomicU32::new(0)),
                config_generation: 0,
                activated: false,
                mem,
                interrupt_fd: None,
            }),
        }
    }

    pub fn new_with_interrupt(
        device: Box<dyn VirtioDevice>,
        mem: GuestMemoryRef,
        interrupt_fd: OwnedFd,
    ) -> Self {
        let transport = Self::new(device, mem);
        transport.state.lock().unwrap().interrupt_fd = Some(interrupt_fd);
        transport
    }

    pub fn new_with_interrupt_status(
        device: Box<dyn VirtioDevice>,
        mem: GuestMemoryRef,
        interrupt_fd: OwnedFd,
        interrupt_status: Arc<AtomicU32>,
    ) -> Self {
        let transport = Self::new_with_interrupt(device, mem, interrupt_fd);
        transport.state.lock().unwrap().interrupt_status = interrupt_status;
        transport
    }

    pub fn new_with_shared_interrupt_status(
        device: Box<dyn VirtioDevice>,
        mem: GuestMemoryRef,
        interrupt_status: Arc<AtomicU32>,
    ) -> Self {
        let transport = Self::new(device, mem);
        transport.state.lock().unwrap().interrupt_status = interrupt_status;
        transport
    }

    #[cfg(target_arch = "x86_64")]
    pub fn snapshot(&self) -> VirtioMmioSnapshot {
        let state = self.state.lock().unwrap();
        VirtioMmioSnapshot {
            status: state.status,
            features_sel: state.features_sel,
            driver_features: state.driver_features,
            driver_features_sel: state.driver_features_sel,
            queue_sel: state.queue_sel,
            queues: state.queues.iter().map(QueueState::snapshot).collect(),
            interrupt_status: state.interrupt_status.load(Ordering::SeqCst),
            config_generation: state.config_generation,
            activated: state.activated,
        }
    }

    #[cfg(target_arch = "x86_64")]
    pub fn quiesce(&self) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.device.quiesce()
    }

    #[cfg(target_arch = "x86_64")]
    pub fn restore(&self, snapshot: &VirtioMmioSnapshot) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        if snapshot.queues.len() != state.queues.len() {
            bail!(
                "virtio-mmio queue count mismatch: checkpoint={}, device={}",
                snapshot.queues.len(),
                state.queues.len()
            );
        }

        state.status = snapshot.status;
        state.features_sel = snapshot.features_sel;
        state.driver_features = snapshot.driver_features;
        state.driver_features_sel = snapshot.driver_features_sel;
        state.queue_sel = snapshot.queue_sel;
        state.queues = snapshot.queues.iter().map(QueueState::restore).collect();
        state
            .interrupt_status
            .store(snapshot.interrupt_status, Ordering::SeqCst);
        state.config_generation = snapshot.config_generation;
        state.activated = snapshot.activated;

        if state.activated {
            let mem = state.mem.clone();
            let queue_configs: Vec<QueueConfig> = state
                .queues
                .iter()
                .map(|q| QueueConfig {
                    desc_addr: q.desc_addr(),
                    driver_addr: q.driver_addr(),
                    device_addr: q.device_addr(),
                    size: q.num,
                    warm_restore: true,
                    event_idx: snapshot.driver_features & VIRTIO_RING_F_EVENT_IDX != 0,
                })
                .collect();
            state.device.activate(mem, &queue_configs);
            tracing::info!(
                event_name = "virtio.mmio.restore_activate",
                device_type = state.device.device_type(),
                queues = queue_configs.len(),
                "virtio-mmio device restored and activated"
            );
        }

        Ok(())
    }
}

impl MmioDevice for VirtioMmioTransport {
    fn read(&self, offset: u64, data: &mut [u8]) {
        let state = self.state.lock().unwrap();
        let device_type = state.device.device_type();
        let val: u32 = match offset {
            MAGIC_VALUE => VIRTIO_MMIO_MAGIC,
            VERSION => VIRTIO_MMIO_VERSION,
            DEVICE_ID => state.device.device_type(),
            VENDOR_ID => CAPSEM_VENDOR_ID,
            DEVICE_FEATURES => {
                let features = state.device.features();
                if state.features_sel == 0 {
                    features as u32
                } else {
                    (features >> 32) as u32
                }
            }
            QUEUE_NUM_MAX => {
                let qsel = state.queue_sel as usize;
                let sizes = state.device.queue_max_sizes();
                if qsel < sizes.len() {
                    sizes[qsel] as u32
                } else {
                    0
                }
            }
            QUEUE_READY => {
                let qsel = state.queue_sel as usize;
                if qsel < state.queues.len() && state.queues[qsel].ready {
                    1
                } else {
                    0
                }
            }
            INTERRUPT_STATUS => state.interrupt_status.load(Ordering::SeqCst),
            STATUS => state.status,
            CONFIG_GENERATION => state.config_generation,
            offset if offset >= CONFIG_SPACE => {
                let config_offset = offset - CONFIG_SPACE;
                let mut config_data = [0u8; 4];
                let len = data.len().min(4);
                state
                    .device
                    .read_config(config_offset, &mut config_data[..len]);
                data[..len].copy_from_slice(&config_data[..len]);
                return;
            }
            _ => 0,
        };

        if matches!(
            offset,
            DEVICE_ID | DEVICE_FEATURES | QUEUE_NUM_MAX | INTERRUPT_STATUS | STATUS
        ) {
            tracing::trace!(
                event_name = "virtio.mmio.read",
                device_type,
                offset = format_args!("{offset:#x}"),
                value = format_args!("{val:#x}"),
                "virtio-mmio register read"
            );
        }

        let bytes = val.to_le_bytes();
        let len = data.len().min(4);
        data[..len].copy_from_slice(&bytes[..len]);
    }

    fn write(&self, offset: u64, data: &[u8]) {
        let mut state = self.state.lock().unwrap();
        let device_type = state.device.device_type();

        // Parse value from data (up to 4 bytes, little-endian)
        let mut bytes = [0u8; 4];
        let len = data.len().min(4);
        bytes[..len].copy_from_slice(&data[..len]);
        let val = u32::from_le_bytes(bytes);

        match offset {
            DEVICE_FEATURES_SEL => {
                state.features_sel = val;
            }
            DRIVER_FEATURES => {
                if state.driver_features_sel == 0 {
                    state.driver_features =
                        (state.driver_features & 0xFFFF_FFFF_0000_0000) | val as u64;
                } else {
                    state.driver_features =
                        (state.driver_features & 0x0000_0000_FFFF_FFFF) | ((val as u64) << 32);
                }
            }
            DRIVER_FEATURES_SEL => {
                state.driver_features_sel = val;
            }
            QUEUE_SEL => {
                state.queue_sel = val;
            }
            QUEUE_NUM => {
                let qsel = state.queue_sel as usize;
                if qsel < state.queues.len() {
                    state.queues[qsel].num = val as u16;
                }
            }
            QUEUE_READY => {
                let qsel = state.queue_sel as usize;
                if qsel < state.queues.len() {
                    state.queues[qsel].ready = val != 0;
                    tracing::trace!(
                        event_name = "virtio.mmio.queue_ready",
                        device_type,
                        queue = state.queue_sel,
                        ready = val != 0,
                        "virtio-mmio queue readiness changed"
                    );
                }
            }
            QUEUE_NOTIFY => {
                if state.activated {
                    let use_interrupt = state.device.uses_mmio_interrupt();
                    tracing::trace!(
                        event_name = "virtio.mmio.queue_notify",
                        device_type,
                        queue = val,
                        use_interrupt,
                        "virtio-mmio queue notified"
                    );
                    let should_interrupt = state.device.queue_notify(val);
                    if use_interrupt && should_interrupt {
                        state.interrupt_status.fetch_or(1, Ordering::SeqCst);
                        if let Some(fd) = state.interrupt_fd.as_ref() {
                            let one: u64 = 1;
                            let ret = unsafe {
                                libc::write(
                                    fd.as_raw_fd(),
                                    &one as *const _ as *const libc::c_void,
                                    std::mem::size_of::<u64>(),
                                )
                            };
                            if ret < 0 {
                                tracing::warn!(
                                    error = %std::io::Error::last_os_error(),
                                    "failed to signal virtio-mmio interrupt eventfd"
                                );
                            }
                        }
                    }
                }
            }
            INTERRUPT_ACK => {
                state.interrupt_status.fetch_and(!val, Ordering::SeqCst);
            }
            STATUS => {
                if val == 0 {
                    // Reset
                    state.status = 0;
                    state.activated = false;
                    for q in &mut state.queues {
                        *q = QueueState::new();
                    }
                    return;
                }
                state.status = val;
                tracing::debug!(
                    event_name = "virtio.mmio.status",
                    device_type,
                    status = format_args!("{val:#x}"),
                    acknowledge = (val & STATUS_ACKNOWLEDGE) != 0,
                    driver = (val & STATUS_DRIVER) != 0,
                    features_ok = (val & STATUS_FEATURES_OK) != 0,
                    driver_ok = (val & STATUS_DRIVER_OK) != 0,
                    failed = (val & STATUS_FAILED) != 0,
                    "virtio-mmio device status changed"
                );
                // Check if DRIVER_OK was just set
                if val & STATUS_DRIVER_OK != 0 && !state.activated {
                    state.activated = true;
                    let mem = state.mem.clone();
                    let queue_configs: Vec<QueueConfig> = state
                        .queues
                        .iter()
                        .map(|q| QueueConfig {
                            desc_addr: q.desc_addr(),
                            driver_addr: q.driver_addr(),
                            device_addr: q.device_addr(),
                            size: q.num,
                            warm_restore: false,
                            event_idx: state.driver_features & VIRTIO_RING_F_EVENT_IDX != 0,
                        })
                        .collect();
                    state.device.activate(mem, &queue_configs);
                    tracing::info!(
                        event_name = "virtio.mmio.activate",
                        device_type,
                        queues = queue_configs.len(),
                        "virtio-mmio device activated"
                    );
                }
            }
            QUEUE_DESC_LOW => {
                let qsel = state.queue_sel as usize;
                if qsel < state.queues.len() {
                    state.queues[qsel].desc_lo = val;
                }
            }
            QUEUE_DESC_HIGH => {
                let qsel = state.queue_sel as usize;
                if qsel < state.queues.len() {
                    state.queues[qsel].desc_hi = val;
                }
            }
            QUEUE_DRIVER_LOW => {
                let qsel = state.queue_sel as usize;
                if qsel < state.queues.len() {
                    state.queues[qsel].driver_lo = val;
                }
            }
            QUEUE_DRIVER_HIGH => {
                let qsel = state.queue_sel as usize;
                if qsel < state.queues.len() {
                    state.queues[qsel].driver_hi = val;
                }
            }
            QUEUE_DEVICE_LOW => {
                let qsel = state.queue_sel as usize;
                if qsel < state.queues.len() {
                    state.queues[qsel].device_lo = val;
                }
            }
            QUEUE_DEVICE_HIGH => {
                let qsel = state.queue_sel as usize;
                if qsel < state.queues.len() {
                    state.queues[qsel].device_hi = val;
                }
            }
            offset if offset >= CONFIG_SPACE => {
                let config_offset = offset - CONFIG_SPACE;
                state.device.write_config(config_offset, &data[..len]);
            }
            _ => {} // ignore writes to read-only or unknown registers
        }
    }
}

#[cfg(test)]
mod tests;
