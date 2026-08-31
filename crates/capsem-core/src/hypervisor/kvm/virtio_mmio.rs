//! Virtio MMIO transport layer (virtio spec v1.2).
//!
//! Handles the MMIO register state machine for device discovery,
//! feature negotiation, queue setup, and activation. Dispatches
//! device-specific operations to the VirtioDevice trait.

use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};

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
const STATUS_DEVICE_NEEDS_RESET: u32 = 64;
const STATUS_FAILED: u32 = 128;
const STATUS_KNOWN_MASK: u32 = STATUS_ACKNOWLEDGE
    | STATUS_DRIVER
    | STATUS_DRIVER_OK
    | STATUS_FEATURES_OK
    | STATUS_DEVICE_NEEDS_RESET
    | STATUS_FAILED;

const VIRTIO_F_VERSION_1: u64 = 1 << 32;
const INTERRUPT_KNOWN_MASK: u32 = 0b11;

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
    /// Serialize process-local device state after quiescence.
    fn checkpoint_state(&mut self) -> Result<Vec<u8>> {
        Ok(Vec::new())
    }
    /// Restore process-local device state before any queue is activated.
    fn restore_checkpoint_state(&mut self, state: &[u8]) -> Result<()> {
        if !state.is_empty() {
            bail!("stateless virtio device rejected non-empty checkpoint state");
        }
        Ok(())
    }
    /// Activate restored queues, returning backend reconstruction failures to
    /// the checkpoint loader. Cold activation remains best-effort for the
    /// guest-driven DRIVER_OK transition.
    fn restore_activate(&mut self, mem: GuestMemoryRef, queues: &[QueueConfig]) -> Result<()>;
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

impl QueueSnapshot {
    fn desc_addr(&self) -> u64 {
        (u64::from(self.desc_hi) << 32) | u64::from(self.desc_lo)
    }

    fn driver_addr(&self) -> u64 {
        (u64::from(self.driver_hi) << 32) | u64::from(self.driver_lo)
    }

    fn device_addr(&self) -> u64 {
        (u64::from(self.device_hi) << 32) | u64::from(self.device_lo)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VirtioMmioSnapshot {
    pub device_type: u32,
    pub device_state: Vec<u8>,
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
        u64::from(self.desc_hi) << 32 | u64::from(self.desc_lo)
    }

    fn driver_addr(&self) -> u64 {
        u64::from(self.driver_hi) << 32 | u64::from(self.driver_lo)
    }

    fn device_addr(&self) -> u64 {
        u64::from(self.device_hi) << 32 | u64::from(self.device_lo)
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
    restore_prepared: bool,
    restore_activation_pending: bool,
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
                restore_prepared: false,
                restore_activation_pending: false,
                mem,
                interrupt_fd: None,
            }),
        }
    }

    pub fn new_with_interrupt(device: Box<dyn VirtioDevice>, mem: GuestMemoryRef, interrupt_fd: OwnedFd) -> Self {
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
    pub fn snapshot(&self) -> Result<VirtioMmioSnapshot> {
        let mut state = self.state.lock().unwrap();
        let device_type = state.device.device_type();
        let device_state = state.device.checkpoint_state()?;
        Ok(VirtioMmioSnapshot {
            device_type,
            device_state,
            status: state.status,
            features_sel: state.features_sel,
            driver_features: state.driver_features,
            driver_features_sel: state.driver_features_sel,
            queue_sel: state.queue_sel,
            queues: state.queues.iter().map(QueueState::snapshot).collect(),
            interrupt_status: state.interrupt_status.load(Ordering::SeqCst),
            config_generation: state.config_generation,
            activated: state.activated,
        })
    }

    #[cfg(target_arch = "x86_64")]
    pub fn quiesce(&self) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.device.quiesce()
    }

    #[cfg(target_arch = "x86_64")]
    pub fn device_type(&self) -> u32 {
        self.state.lock().unwrap().device.device_type()
    }

    /// Validate and rehydrate checkpoint state without starting any device
    /// backend. KVM prepares every transport before activating the first one.
    #[cfg(target_arch = "x86_64")]
    pub fn prepare_restore(&self, snapshot: &VirtioMmioSnapshot) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        ensure_restore_not_started(&state)?;
        if snapshot.device_type != state.device.device_type() {
            bail!(
                "virtio-mmio device type mismatch: checkpoint={}, device={}",
                snapshot.device_type,
                state.device.device_type()
            );
        }
        validate_restore_registers(&state, snapshot)?;
        let driver_ok = snapshot.status & STATUS_DRIVER_OK != 0;
        if snapshot.activated != driver_ok {
            bail!(
                "virtio-mmio activation contradicts DRIVER_OK status: activated={}, DRIVER_OK={driver_ok}",
                snapshot.activated
            );
        }
        if snapshot.queues.len() != state.queues.len() {
            bail!(
                "virtio-mmio queue count mismatch: checkpoint={}, device={}",
                snapshot.queues.len(),
                state.queues.len()
            );
        }
        if snapshot.queue_sel as usize >= snapshot.queues.len() {
            bail!("virtio-mmio queue selector is out of range: {}", snapshot.queue_sel);
        }
        let max_sizes = state.device.queue_max_sizes();
        let mut ranges = Vec::new();
        for (index, queue) in snapshot.queues.iter().enumerate() {
            if queue.num > max_sizes[index] {
                bail!(
                    "virtio-mmio queue {index} size exceeds device maximum: {} > {}",
                    queue.num,
                    max_sizes[index]
                );
            }
            if snapshot.activated && !queue.ready {
                bail!("activated virtio-mmio queue {index} is not ready");
            }
            if queue.ready {
                if queue.num == 0 {
                    bail!("ready virtio-mmio queue {index} has zero size");
                }
                if !queue.num.is_power_of_two() {
                    bail!("ready virtio-mmio queue {index} size is not a power of two");
                }
                ranges.extend(validate_queue_memory(&state.mem, index, queue)?);
            }
        }
        validate_queue_nonoverlap(&mut ranges)?;

        // A device backend must be fully reconstructed before activation can
        // observe the restored virtqueue addresses.
        state.device.restore_checkpoint_state(&snapshot.device_state)?;

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
        state.restore_prepared = true;
        state.restore_activation_pending = snapshot.activated;

        Ok(())
    }

    /// Activate a transport previously prepared by `prepare_restore`.
    #[cfg(target_arch = "x86_64")]
    pub fn activate_restored(&self) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        if !state.restore_prepared {
            bail!("virtio-mmio restore activation requested before preparation");
        }
        let should_activate = state.restore_activation_pending;
        state.restore_prepared = false;
        state.restore_activation_pending = false;

        if should_activate {
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
                    event_idx: state.driver_features & VIRTIO_RING_F_EVENT_IDX != 0,
                })
                .collect();
            if let Err(error) = state.device.restore_activate(mem, &queue_configs) {
                state.activated = false;
                return Err(error);
            }
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

#[cfg(target_arch = "x86_64")]
fn ensure_restore_not_started(state: &TransportState) -> Result<()> {
    if state.activated || state.restore_prepared {
        bail!("virtio-mmio transport is already activated or prepared for restore");
    }
    Ok(())
}

#[cfg(target_arch = "x86_64")]
fn validate_restore_registers(state: &TransportState, snapshot: &VirtioMmioSnapshot) -> Result<()> {
    if snapshot.features_sel > 1 {
        bail!(
            "virtio-mmio device feature selector is unsupported: {}",
            snapshot.features_sel
        );
    }
    if snapshot.driver_features_sel > 1 {
        bail!(
            "virtio-mmio driver feature selector is unsupported: {}",
            snapshot.driver_features_sel
        );
    }
    let unsupported = snapshot.driver_features & !state.device.features();
    if unsupported != 0 {
        bail!("virtio-mmio driver negotiated unsupported feature bits: {unsupported:#x}");
    }
    if snapshot.status & !STATUS_KNOWN_MASK != 0 {
        bail!(
            "virtio-mmio checkpoint contains unknown status bits: {:#x}",
            snapshot.status & !STATUS_KNOWN_MASK
        );
    }
    let dependencies = [
        (STATUS_DRIVER, STATUS_ACKNOWLEDGE),
        (STATUS_FEATURES_OK, STATUS_ACKNOWLEDGE | STATUS_DRIVER),
        (
            STATUS_DRIVER_OK,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
        ),
    ];
    for (bit, required) in dependencies {
        if snapshot.status & bit != 0 && snapshot.status & required != required {
            bail!("virtio-mmio status dependency violation: bit {bit:#x} requires {required:#x}");
        }
    }
    if snapshot.status & STATUS_FEATURES_OK != 0 && snapshot.driver_features & VIRTIO_F_VERSION_1 == 0 {
        bail!("virtio-mmio FEATURES_OK checkpoint did not negotiate VIRTIO_F_VERSION_1");
    }
    if snapshot.interrupt_status & !INTERRUPT_KNOWN_MASK != 0 {
        bail!(
            "virtio-mmio checkpoint contains unknown interrupt status bits: {:#x}",
            snapshot.interrupt_status & !INTERRUPT_KNOWN_MASK
        );
    }
    Ok(())
}

#[cfg(target_arch = "x86_64")]
#[derive(Debug)]
pub(super) struct QueueMemoryRange {
    pub(super) start: u64,
    pub(super) end: u64,
    pub(super) queue: usize,
    pub(super) name: &'static str,
}

#[cfg(target_arch = "x86_64")]
fn validate_queue_memory(mem: &GuestMemoryRef, index: usize, queue: &QueueSnapshot) -> Result<[QueueMemoryRange; 3]> {
    let addresses = [
        (queue.desc_addr(), 16, "descriptor"),
        (queue.driver_addr(), 2, "available ring"),
        (queue.device_addr(), 4, "used ring"),
    ];
    for (address, alignment, name) in addresses {
        if address % alignment != 0 {
            bail!("virtio-mmio queue {index} {name} address is not {alignment}-byte aligned");
        }
    }
    let ranges = queue_memory_ranges(index, queue)?;
    for range in &ranges {
        if mem.gpa_range_to_host(range.start, range.end - range.start).is_none() {
            bail!("virtio-mmio queue {index} {} is outside guest memory", range.name);
        }
    }
    Ok(ranges)
}

#[cfg(target_arch = "x86_64")]
fn queue_memory_ranges(index: usize, queue: &QueueSnapshot) -> Result<[QueueMemoryRange; 3]> {
    let size = u64::from(queue.num);
    let spans = [
        (
            queue.desc_addr(),
            size.checked_mul(16).context("descriptor span overflow")?,
            "descriptor",
        ),
        (
            queue.driver_addr(),
            size.checked_mul(2)
                .and_then(|n| n.checked_add(8))
                .context("available ring span overflow")?,
            "available ring",
        ),
        (
            queue.device_addr(),
            size.checked_mul(8)
                .and_then(|n| n.checked_add(8))
                .context("used ring span overflow")?,
            "used ring",
        ),
    ];
    let validate_range = |(address, len, name): (u64, u64, &'static str)| -> Result<QueueMemoryRange> {
        let end = address
            .checked_add(len)
            .with_context(|| format!("virtio-mmio queue {index} {name} address overflow"))?;
        Ok(QueueMemoryRange {
            start: address,
            end,
            queue: index,
            name,
        })
    };
    Ok([
        validate_range(spans[0])?,
        validate_range(spans[1])?,
        validate_range(spans[2])?,
    ])
}

#[cfg(target_arch = "x86_64")]
pub(super) fn ready_queue_memory_ranges(snapshot: &VirtioMmioSnapshot) -> Result<Vec<QueueMemoryRange>> {
    let mut ranges = Vec::new();
    for (index, queue) in snapshot.queues.iter().enumerate() {
        if queue.ready {
            ranges.extend(queue_memory_ranges(index, queue)?);
        }
    }
    Ok(ranges)
}

#[cfg(target_arch = "x86_64")]
fn validate_queue_nonoverlap(ranges: &mut [QueueMemoryRange]) -> Result<()> {
    ranges.sort_unstable_by_key(|range| (range.start, range.end));
    for pair in ranges.windows(2) {
        if pair[0].end > pair[1].start {
            bail!(
                "virtio-mmio queue memory overlap: queue {} {} overlaps queue {} {}",
                pair[0].queue,
                pair[0].name,
                pair[1].queue,
                pair[1].name
            );
        }
    }
    Ok(())
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
                    u32::from(sizes[qsel])
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
                state.device.read_config(config_offset, &mut config_data[..len]);
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
                    state.driver_features = (state.driver_features & 0xFFFF_FFFF_0000_0000) | u64::from(val);
                } else {
                    state.driver_features = (state.driver_features & 0x0000_0000_FFFF_FFFF) | (u64::from(val) << 32);
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
