use super::*;
use crate::hypervisor::kvm::memory::{GuestMemory, RAM_BASE};

fn fixture(buffer: u64) -> (GuestMemory, TransportEvents) {
    let mem = GuestMemory::new(0x20000).unwrap();
    let config = QueueConfig {
        desc_addr: RAM_BASE + 0x1000,
        driver_addr: RAM_BASE + 0x3000,
        device_addr: RAM_BASE + 0x4000,
        size: 8,
        warm_restore: true,
        event_idx: true,
    };
    mem.write_at(0x1000, &buffer.to_le_bytes()).unwrap();
    mem.write_at(0x1008, &4u32.to_le_bytes()).unwrap();
    mem.write_at(0x100C, &2u16.to_le_bytes()).unwrap();
    let mut events = TransportEvents::new(mem.clone_ref(RAM_BASE), &config).unwrap();
    events.reset_pending = true;
    (mem, events)
}

#[test]
fn a_late_event_buffer_delivers_exactly_one_pending_reset() {
    let (mem, mut events) = fixture(RAM_BASE + 0x1F000);
    assert!(!events.drain_reset().unwrap());
    assert!(events.reset_pending);
    mem.write_at(0x3002, &1u16.to_le_bytes()).unwrap();
    mem.write_at(0x1F000, &[0xAA; 4]).unwrap();
    assert!(events.drain_reset().unwrap());
    assert!(!events.reset_pending);
    assert!(!events.drain_reset().unwrap());
    let mut output = [0xFF; 4];
    mem.read_at(0x1F000, &mut output).unwrap();
    assert_eq!(output, TRANSPORT_RESET.to_le_bytes());
}

#[test]
fn reset_events_reject_buffers_crossing_the_guest_ram_boundary() {
    let (mem, mut events) = fixture(RAM_BASE + 0x1FFFE);
    mem.write_at(0x3002, &1u16.to_le_bytes()).unwrap();
    mem.write_at(0x1FFFE, &[0xAA; 2]).unwrap();
    assert!(events.drain_reset().is_err());
    let mut sentinel = [0; 2];
    mem.read_at(0x1FFFE, &mut sentinel).unwrap();
    assert_eq!(
        sentinel, [0xAA; 2],
        "invalid events must not partially overwrite the guest buffer"
    );
}
