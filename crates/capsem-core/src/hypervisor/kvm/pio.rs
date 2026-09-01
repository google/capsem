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
    pub fn register(&self, base: u16, size: u16, device: Arc<dyn PioDevice>) -> anyhow::Result<()> {
        let mut devices = self.devices.write().unwrap();
        // Check for overlap
        for entry in devices.iter() {
            if base < entry.base + entry.size && base + size > entry.base {
                anyhow::bail!(
                    "PIO region 0x{base:x}..0x{:x} overlaps existing 0x{:x}..0x{:x}",
                    base + size,
                    entry.base,
                    entry.base + entry.size
                );
            }
        }
        devices.push(PioEntry { base, size, device });
        drop(devices);
        Ok(())
    }

    /// Read from a port. If no device is registered, data is zeroed.
    pub fn read(&self, port: u16, data: &mut [u8]) {
        let devices = self.devices.read().unwrap();
        let target = devices
            .iter()
            .find(|entry| port >= entry.base && port < entry.base + entry.size)
            .map(|entry| (Arc::clone(&entry.device), port - entry.base));
        drop(devices);
        if let Some((device, offset)) = target {
            device.read(offset, data);
            return;
        }
        data.fill(0xFF); // default: all bits high (no device)
    }

    /// Write to a port. If no device is registered, the write is silently ignored.
    pub fn write(&self, port: u16, data: &[u8]) {
        let devices = self.devices.read().unwrap();
        let target = devices
            .iter()
            .find(|entry| port >= entry.base && port < entry.base + entry.size)
            .map(|entry| (Arc::clone(&entry.device), port - entry.base));
        drop(devices);
        if let Some((device, offset)) = target {
            device.write(offset, data);
        }
    }
}

#[cfg(test)]
mod tests;
