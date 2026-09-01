//! MMIO bus: routes guest physical address accesses to device handlers.
//!
//! When a vCPU exits with KVM_EXIT_MMIO, the run loop dispatches the
//! access through this bus to the correct device based on address.

use std::sync::Arc;

use anyhow::{bail, Result};

/// A device that handles MMIO reads and writes.
pub(super) trait MmioDevice: Send + Sync {
    /// Handle an MMIO read at the given offset within the device.
    fn read(&self, offset: u64, data: &mut [u8]);
    /// Handle an MMIO write at the given offset within the device.
    fn write(&self, offset: u64, data: &[u8]);
}

struct DeviceEntry {
    base: u64,
    size: u64,
    device: Arc<dyn MmioDevice>,
}

/// MMIO bus that dispatches address-based accesses to registered devices.
pub(super) struct MmioBus {
    devices: std::sync::RwLock<Vec<DeviceEntry>>,
}

impl MmioBus {
    pub fn new() -> Self {
        Self {
            devices: std::sync::RwLock::new(Vec::new()),
        }
    }

    /// Register a device at the given base address and size.
    /// Returns an error if the region overlaps with an existing device.
    pub fn register(&self, base: u64, size: u64, device: Arc<dyn MmioDevice>) -> Result<()> {
        let mut devices = self.devices.write().unwrap();
        let new_end = base + size;

        for entry in devices.iter() {
            let existing_end = entry.base + entry.size;
            if base < existing_end && new_end > entry.base {
                bail!(
                    "MMIO region [{:#x}, {:#x}) overlaps with existing [{:#x}, {:#x})",
                    base,
                    new_end,
                    entry.base,
                    existing_end
                );
            }
        }

        devices.push(DeviceEntry { base, size, device });
        drop(devices);
        Ok(())
    }

    /// Dispatch a read to the device at the given address.
    /// If no device is registered at this address, data is filled with 0xFF.
    pub fn read(&self, addr: u64, data: &mut [u8]) {
        let devices = self.devices.read().unwrap();
        let target = devices
            .iter()
            .find(|entry| addr >= entry.base && addr < entry.base + entry.size)
            .map(|entry| (Arc::clone(&entry.device), addr - entry.base));
        drop(devices);
        if let Some((device, offset)) = target {
            device.read(offset, data);
            return;
        }
        // No device at this address -- return all 1s (bus float)
        data.fill(0xFF);
    }

    /// Dispatch a write to the device at the given address.
    /// If no device is registered, the write is silently ignored.
    pub fn write(&self, addr: u64, data: &[u8]) {
        let devices = self.devices.read().unwrap();
        let target = devices
            .iter()
            .find(|entry| addr >= entry.base && addr < entry.base + entry.size)
            .map(|entry| (Arc::clone(&entry.device), addr - entry.base));
        drop(devices);
        if let Some((device, offset)) = target {
            device.write(offset, data);
        }
        // No device -- silently ignore
    }
}

#[cfg(test)]
mod tests;
