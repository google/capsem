//! Virtio MMIO transport layer (virtio spec v1.2).
//!
//! Handles the MMIO register state machine for device discovery,
//! feature negotiation, queue setup, and activation. Dispatches
//! device-specific operations to the VirtioDevice trait.

use std::sync::Mutex;

use super::memory::GuestMemoryRef;
use super::mmio::MmioDevice;

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
    fn queue_notify(&mut self, queue_index: u32);
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
    interrupt_status: u32,
    config_generation: u32,
    activated: bool,
    mem: GuestMemoryRef,
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
                interrupt_status: 0,
                config_generation: 0,
                activated: false,
                mem,
            }),
        }
    }
}

impl MmioDevice for VirtioMmioTransport {
    fn read(&self, offset: u64, data: &mut [u8]) {
        let state = self.state.lock().unwrap();
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
            INTERRUPT_STATUS => state.interrupt_status,
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

        let bytes = val.to_le_bytes();
        let len = data.len().min(4);
        data[..len].copy_from_slice(&bytes[..len]);
    }

    fn write(&self, offset: u64, data: &[u8]) {
        let mut state = self.state.lock().unwrap();

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
                    state.driver_features = (state.driver_features & 0xFFFF_FFFF_0000_0000) | val as u64;
                } else {
                    state.driver_features = (state.driver_features & 0x0000_0000_FFFF_FFFF) | ((val as u64) << 32);
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
                }
            }
            QUEUE_NOTIFY => {
                if state.activated {
                    state.device.queue_notify(val);
                }
            }
            INTERRUPT_ACK => {
                state.interrupt_status &= !val;
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
                // Check if DRIVER_OK was just set
                if val & STATUS_DRIVER_OK != 0 && !state.activated {
                    state.activated = true;
                    let mem = state.mem.clone();
                    let queue_configs: Vec<QueueConfig> = state.queues.iter().map(|q| {
                        QueueConfig {
                            desc_addr: q.desc_addr(),
                            driver_addr: q.driver_addr(),
                            device_addr: q.device_addr(),
                            size: q.num,
                        }
                    }).collect();
                    state.device.activate(mem, &queue_configs);
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
    use super::*;
    use super::super::memory::{GuestMemory, RAM_BASE};

    struct DummyDevice {
        activated: std::sync::Arc<std::sync::atomic::AtomicBool>,
        notify_count: std::sync::Arc<std::sync::atomic::AtomicU32>,
    }

    impl DummyDevice {
        fn new() -> (Self, std::sync::Arc<std::sync::atomic::AtomicBool>, std::sync::Arc<std::sync::atomic::AtomicU32>) {
            let activated = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let notify_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
            (
                Self {
                    activated: activated.clone(),
                    notify_count: notify_count.clone(),
                },
                activated,
                notify_count,
            )
        }
    }

    impl VirtioDevice for DummyDevice {
        fn device_type(&self) -> u32 { 3 } // console
        fn features(&self) -> u64 { 0x0000_0001_0000_0001 } // feature bits in both halves
        fn queue_max_sizes(&self) -> &[u16] { &[256, 256] }
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
        fn queue_notify(&mut self, _queue_index: u32) {
            self.notify_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    fn make_transport() -> (VirtioMmioTransport, std::sync::Arc<std::sync::atomic::AtomicBool>, std::sync::Arc<std::sync::atomic::AtomicU32>) {
        let mem = GuestMemory::new(4096).unwrap();
        let (dev, activated, notify_count) = DummyDevice::new();
        let transport = VirtioMmioTransport::new(Box::new(dev), mem.clone_ref());
        (transport, activated, notify_count)
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
        assert_eq!(
            read_u32(&t, STATUS),
            STATUS_ACKNOWLEDGE | STATUS_DRIVER
        );

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
        write_u32(&t, STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK);
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
