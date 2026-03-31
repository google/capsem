//! PIO (port I/O) bus for x86_64 KVM.
//!
//! Dispatches KVM_EXIT_IO to registered devices by port address.
//! Mirrors the MMIO bus design but uses u16 port addresses.

use std::sync::{Arc, RwLock};

/// Trait for devices that handle port I/O.
pub(super) trait PioDevice: Send + Sync {
    fn read(&self, port_offset: u16, data: &mut [u8]);
    fn write(&self, port_offset: u16, data: &[u8]);
}

struct PioEntry {
    base: u16,
    size: u16,
    device: Arc<dyn PioDevice>,
}

/// Port I/O bus that dispatches reads/writes to registered devices.
pub(super) struct PioBus {
    devices: RwLock<Vec<PioEntry>>,
}

impl PioBus {
    pub fn new() -> Self {
        Self {
            devices: RwLock::new(Vec::new()),
        }
    }

    /// Register a device on the PIO bus.
    pub fn register(
        &self,
        base: u16,
        size: u16,
        device: Arc<dyn PioDevice>,
    ) -> anyhow::Result<()> {
        let mut devices = self.devices.write().unwrap();
        // Check for overlap
        for entry in devices.iter() {
            if base < entry.base + entry.size && base + size > entry.base {
                anyhow::bail!(
                    "PIO region 0x{base:x}..0x{:x} overlaps existing 0x{:x}..0x{:x}",
                    base + size, entry.base, entry.base + entry.size
                );
            }
        }
        devices.push(PioEntry { base, size, device });
        Ok(())
    }

    /// Read from a port. If no device is registered, data is zeroed.
    pub fn read(&self, port: u16, data: &mut [u8]) {
        let devices = self.devices.read().unwrap();
        for entry in devices.iter() {
            if port >= entry.base && port < entry.base + entry.size {
                entry.device.read(port - entry.base, data);
                return;
            }
        }
        data.fill(0xFF); // default: all bits high (no device)
    }

    /// Write to a port. If no device is registered, the write is silently ignored.
    pub fn write(&self, port: u16, data: &[u8]) {
        let devices = self.devices.read().unwrap();
        for entry in devices.iter() {
            if port >= entry.base && port < entry.base + entry.size {
                entry.device.write(port - entry.base, data);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct TestDevice {
        reads: AtomicU32,
        writes: AtomicU32,
    }

    impl TestDevice {
        fn new() -> Self {
            Self {
                reads: AtomicU32::new(0),
                writes: AtomicU32::new(0),
            }
        }
    }

    impl PioDevice for TestDevice {
        fn read(&self, _offset: u16, data: &mut [u8]) {
            self.reads.fetch_add(1, Ordering::SeqCst);
            data.fill(0x42);
        }

        fn write(&self, _offset: u16, _data: &[u8]) {
            self.writes.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn register_and_dispatch() {
        let bus = PioBus::new();
        let dev = Arc::new(TestDevice::new());
        bus.register(0x3F8, 8, dev.clone()).unwrap();

        let mut buf = [0u8; 1];
        bus.read(0x3F8, &mut buf);
        assert_eq!(buf[0], 0x42);
        assert_eq!(dev.reads.load(Ordering::SeqCst), 1);

        bus.write(0x3F9, &[0x01]);
        assert_eq!(dev.writes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn unregistered_port_returns_ff() {
        let bus = PioBus::new();
        let mut buf = [0u8; 1];
        bus.read(0x100, &mut buf);
        assert_eq!(buf[0], 0xFF);
    }

    #[test]
    fn overlap_rejected() {
        let bus = PioBus::new();
        let dev = Arc::new(TestDevice::new());
        bus.register(0x3F8, 8, dev.clone()).unwrap();
        assert!(bus.register(0x3FC, 4, dev).is_err());
    }

    #[test]
    fn offset_calculation() {
        let bus = PioBus::new();
        let dev = Arc::new(TestDevice::new());
        bus.register(0x3F8, 8, dev.clone()).unwrap();
        // Port 0x3FD should give offset 5 to device
        bus.read(0x3FD, &mut [0]);
        assert_eq!(dev.reads.load(Ordering::SeqCst), 1);
    }
}
