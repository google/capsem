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

    fn config(&self, warm_restore: bool, event_idx: bool) -> QueueConfig {
        QueueConfig {
            desc_addr: self.desc_addr(),
            driver_addr: self.driver_addr(),
            device_addr: self.device_addr(),
            size: self.num,
            warm_restore,
            event_idx,
        }
    }

    fn validate(
        &self,
        queue_index: usize,
        max_size: u16,
        mem: &GuestMemoryRef,
    ) -> Result<(), String> {
        if !self.ready {
            return Ok(());
        }
        if self.num == 0 || !self.num.is_power_of_two() || self.num > max_size {
            return Err(format!(
                "queue {queue_index} invalid size: num={}, max={max_size}",
                self.num
            ));
        }
        if self.desc_addr() % 16 != 0 {
            return Err(format!(
                "queue {queue_index} descriptor table is not 16-byte aligned: {:#x}",
                self.desc_addr()
            ));
        }
        if self.driver_addr() % 2 != 0 {
            return Err(format!(
                "queue {queue_index} available ring is not 2-byte aligned: {:#x}",
                self.driver_addr()
            ));
        }
        if self.device_addr() % 4 != 0 {
            return Err(format!(
                "queue {queue_index} used ring is not 4-byte aligned: {:#x}",
                self.device_addr()
            ));
        }

        let size = self.num as u64;
        let desc_len = size
            .checked_mul(16)
            .ok_or_else(|| format!("queue {queue_index} descriptor table length overflow"))?;
        let avail_len = size
            .checked_mul(2)
            .and_then(|ring_len| 4_u64.checked_add(ring_len))
            .and_then(|base_len| base_len.checked_add(2))
            .ok_or_else(|| format!("queue {queue_index} available ring length overflow"))?;
        let used_len = size
            .checked_mul(8)
            .and_then(|ring_len| 4_u64.checked_add(ring_len))
            .and_then(|base_len| base_len.checked_add(2))
            .ok_or_else(|| format!("queue {queue_index} used ring length overflow"))?;

        if mem.gpa_range_to_host(self.desc_addr(), desc_len).is_none() {
            return Err(format!(
                "queue {queue_index} descriptor table is outside guest RAM: addr={:#x}, len={desc_len}",
                self.desc_addr()
            ));
        }
        if mem
            .gpa_range_to_host(self.driver_addr(), avail_len)
            .is_none()
        {
            return Err(format!(
                "queue {queue_index} available ring is outside guest RAM: addr={:#x}, len={avail_len}",
                self.driver_addr()
            ));
        }
        if mem
            .gpa_range_to_host(self.device_addr(), used_len)
            .is_none()
        {
            return Err(format!(
                "queue {queue_index} used ring is outside guest RAM: addr={:#x}, len={used_len}",
                self.device_addr()
            ));
        }

        Ok(())
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

fn validate_ready_queues(
    queues: &[QueueState],
    max_sizes: &[u16],
    mem: &GuestMemoryRef,
) -> Result<(), String> {
    for (index, queue) in queues.iter().enumerate() {
        let max_size = max_sizes
            .get(index)
            .copied()
            .ok_or_else(|| format!("queue {index} has no device maximum"))?;
        queue.validate(index, max_size, mem)?;
    }
    Ok(())
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
            validate_ready_queues(&state.queues, state.device.queue_max_sizes(), &state.mem)
                .map_err(|err| anyhow::anyhow!("invalid restored virtio queue: {err}"))?;
            let mem = state.mem.clone();
            let event_idx = snapshot.driver_features & VIRTIO_RING_F_EVENT_IDX != 0;
            let queue_configs: Vec<QueueConfig> = state
                .queues
                .iter()
                .map(|q| q.config(true, event_idx))
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
                    if let Err(error) = validate_ready_queues(
                        &state.queues,
                        state.device.queue_max_sizes(),
                        &state.mem,
                    ) {
                        state.status |= STATUS_FAILED;
                        tracing::warn!(
                            event_name = "virtio.mmio.activate_failed",
                            device_type,
                            error,
                            "virtio-mmio device activation rejected invalid queue configuration"
                        );
                        return;
                    }
                    state.activated = true;
                    let mem = state.mem.clone();
                    let event_idx = state.driver_features & VIRTIO_RING_F_EVENT_IDX != 0;
                    let queue_configs: Vec<QueueConfig> = state
                        .queues
                        .iter()
                        .map(|q| q.config(false, event_idx))
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
mod tests {
    use super::super::memory::{GuestMemory, RAM_BASE};
    use super::*;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

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
            self.activated
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        fn queue_notify(&mut self, _queue_index: u32) -> bool {
            self.notify_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
        let mem = GuestMemory::new(64 * 1024).unwrap();
        let (dev, activated, notify_count) = DummyDevice::new();
        let transport = VirtioMmioTransport::new(Box::new(dev), mem.clone_ref(RAM_BASE));
        (transport, activated, notify_count)
    }

    fn make_transport_with_interrupt() -> (
        VirtioMmioTransport,
        OwnedFd,
        std::sync::Arc<std::sync::atomic::AtomicU32>,
    ) {
        let mem = GuestMemory::new(64 * 1024).unwrap();
        let (mut dev, _, notify_count) = DummyDevice::new();
        dev.use_interrupt = true;
        let raw_fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        assert!(raw_fd >= 0);
        let interrupt_fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
        let read_fd = unsafe { OwnedFd::from_raw_fd(libc::dup(raw_fd)) };
        let transport = VirtioMmioTransport::new_with_interrupt(
            Box::new(dev),
            mem.clone_ref(RAM_BASE),
            interrupt_fd,
        );
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

    fn configure_valid_queue(dev: &dyn MmioDevice, queue: u32, size: u32) {
        let base = 0x1000 + (queue * 0x3000);
        write_u32(dev, QUEUE_SEL, queue);
        write_u32(dev, QUEUE_NUM, size);
        write_u32(dev, QUEUE_DESC_LOW, base);
        write_u32(dev, QUEUE_DRIVER_LOW, base + 0x1000);
        write_u32(dev, QUEUE_DEVICE_LOW, base + 0x2000);
        write_u32(dev, QUEUE_READY, 1);
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

    #[test]
    fn driver_ok_rejects_ready_queue_with_zero_size() {
        let (t, activated, _) = make_transport();

        write_u32(&t, QUEUE_SEL, 0);
        write_u32(&t, QUEUE_READY, 1);
        write_u32(
            &t,
            STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        );

        assert!(!activated.load(std::sync::atomic::Ordering::SeqCst));
        assert_ne!(read_u32(&t, STATUS) & STATUS_FAILED, 0);
    }

    #[test]
    fn driver_ok_rejects_ready_queue_outside_guest_ram() {
        let (t, activated, _) = make_transport();

        write_u32(&t, QUEUE_SEL, 0);
        write_u32(&t, QUEUE_NUM, 16);
        write_u32(&t, QUEUE_DESC_LOW, 0xF000);
        write_u32(&t, QUEUE_DRIVER_LOW, 0x1_0000);
        write_u32(&t, QUEUE_DEVICE_LOW, 0x1_1000);
        write_u32(&t, QUEUE_READY, 1);
        write_u32(
            &t,
            STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        );

        assert!(!activated.load(std::sync::atomic::Ordering::SeqCst));
        assert_ne!(read_u32(&t, STATUS) & STATUS_FAILED, 0);
    }

    #[test]
    fn driver_ok_accepts_valid_ready_queues() {
        let (t, activated, _) = make_transport();

        configure_valid_queue(&t, 0, 16);
        configure_valid_queue(&t, 1, 16);
        write_u32(
            &t,
            STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        );

        assert!(activated.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(read_u32(&t, STATUS) & STATUS_FAILED, 0);
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
        write_u32(
            &t,
            STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
        );

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
            status: STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
            features_sel: 1,
            driver_features: 0x1000_0001,
            driver_features_sel: 0,
            queue_sel: 1,
            queues: vec![
                QueueSnapshot {
                    num: 16,
                    ready: true,
                    desc_lo: 0x1000,
                    desc_hi: 0,
                    driver_lo: 0x2000,
                    driver_hi: 0,
                    device_lo: 0x3000,
                    device_hi: 0,
                },
                QueueSnapshot {
                    num: 8,
                    ready: false,
                    desc_lo: 0x4000,
                    desc_hi: 0,
                    driver_lo: 0x5000,
                    driver_hi: 0,
                    device_lo: 0x6000,
                    device_hi: 0,
                },
            ],
            interrupt_status: 1,
            config_generation: 7,
            activated: true,
        };

        t.restore(&snapshot).unwrap();

        assert!(activated.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(t.snapshot(), snapshot);
        write_u32(&t, QUEUE_NOTIFY, 0);
        assert_eq!(notify_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn restore_rejects_wrong_queue_count() {
        let (t, _, _) = make_transport();
        let snapshot = VirtioMmioSnapshot {
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

        let err = t.restore(&snapshot).unwrap_err();

        assert!(err.to_string().contains("queue count mismatch"));
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
}
