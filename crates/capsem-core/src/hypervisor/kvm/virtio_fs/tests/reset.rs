//! Device reset: STATUS=0 must stop the worker and allow re-activation.

use std::sync::atomic::AtomicU32;
use std::sync::Arc;

use super::tests::{temp_share, test_device};
use crate::hypervisor::kvm::memory::{GuestMemory, RAM_BASE};
use crate::hypervisor::kvm::virtio_mmio::{QueueConfig, VirtioDevice};

fn queues() -> [QueueConfig; 2] {
    let queue = |base: u64| QueueConfig {
        desc_addr: RAM_BASE + base,
        driver_addr: RAM_BASE + base + 0x100,
        device_addr: RAM_BASE + base + 0x200,
        size: 8,
        warm_restore: false,
        event_idx: false,
    };
    [queue(0), queue(0x400)]
}

// A second activation used to fail with "processor is unavailable" while
// the first worker kept the old rings: after a driver rebind the guest's
// filesystem was dead and the stale worker could still write used entries
// into memory the guest had freed.
#[test]
fn reset_returns_the_processor_and_the_device_activates_again() {
    let dir = temp_share("reset-reactivate");
    let mut dev = test_device(&dir);
    let mem = GuestMemory::new(1024 * 1024).unwrap();

    dev.activate(mem.clone_ref(RAM_BASE), &queues());
    assert!(dev.notify_tx.is_some() && dev.worker_handle.is_some());
    assert!(dev.processor.is_none(), "the worker owns the processor while active");

    dev.reset();
    assert!(dev.notify_tx.is_none() && dev.worker_handle.is_none());
    assert!(dev.processor.is_some(), "reset hands the processor back");

    dev.activate(mem.clone_ref(RAM_BASE), &queues());
    assert!(dev.notify_tx.is_some() && dev.worker_handle.is_some());
    assert!(dev.processor.is_none());
    assert!(dev.quiesce().is_ok(), "the re-activated worker answers checkpoints");
}

#[test]
fn reset_before_activation_and_twice_in_a_row_are_harmless() {
    let dir = temp_share("reset-idempotent");
    let mut dev = test_device(&dir);
    dev.reset();
    dev.reset();
    assert!(dev.processor.is_some());
    let mem = GuestMemory::new(1024 * 1024).unwrap();
    dev.activate(mem.clone_ref(RAM_BASE), &queues());
    dev.reset();
    dev.reset();
    assert!(dev.processor.is_some());
    let _ = Arc::new(AtomicU32::new(0));
}
