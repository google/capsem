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
