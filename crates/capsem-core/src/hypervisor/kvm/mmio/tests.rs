use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

struct MockDevice {
    read_count: AtomicU32,
    write_count: AtomicU32,
    last_offset: std::sync::Mutex<u64>,
    value: u8,
}

impl MockDevice {
    fn new(value: u8) -> Self {
        Self {
            read_count: AtomicU32::new(0),
            write_count: AtomicU32::new(0),
            last_offset: std::sync::Mutex::new(0),
            value,
        }
    }
}

impl MmioDevice for MockDevice {
    fn read(&self, offset: u64, data: &mut [u8]) {
        self.read_count.fetch_add(1, Ordering::SeqCst);
        *self.last_offset.lock().unwrap() = offset;
        data.fill(self.value);
    }

    fn write(&self, offset: u64, _data: &[u8]) {
        self.write_count.fetch_add(1, Ordering::SeqCst);
        *self.last_offset.lock().unwrap() = offset;
    }
}

#[test]
fn register_and_read() {
    let bus = MmioBus::new();
    let dev = Arc::new(MockDevice::new(0x42));
    bus.register(0x1000, 0x100, dev.clone()).unwrap();

    let mut data = [0u8; 4];
    bus.read(0x1000, &mut data);
    assert_eq!(data, [0x42, 0x42, 0x42, 0x42]);
    assert_eq!(dev.read_count.load(Ordering::SeqCst), 1);
}

#[test]
fn read_with_offset() {
    let bus = MmioBus::new();
    let dev = Arc::new(MockDevice::new(0));
    bus.register(0x1000, 0x100, dev.clone()).unwrap();

    let mut data = [0u8; 4];
    bus.read(0x1050, &mut data);
    assert_eq!(*dev.last_offset.lock().unwrap(), 0x50);
}

#[test]
fn write_dispatched() {
    let bus = MmioBus::new();
    let dev = Arc::new(MockDevice::new(0));
    bus.register(0x1000, 0x100, dev.clone()).unwrap();

    bus.write(0x1010, &[1, 2, 3, 4]);
    assert_eq!(dev.write_count.load(Ordering::SeqCst), 1);
    assert_eq!(*dev.last_offset.lock().unwrap(), 0x10);
}

#[test]
fn read_no_device_returns_ff() {
    let bus = MmioBus::new();
    let mut data = [0u8; 4];
    bus.read(0x9999, &mut data);
    assert_eq!(data, [0xFF, 0xFF, 0xFF, 0xFF]);
}

#[test]
fn write_no_device_silent() {
    let bus = MmioBus::new();
    bus.write(0x9999, &[1, 2, 3, 4]); // should not panic
}

#[test]
fn multiple_devices() {
    let bus = MmioBus::new();
    let dev_a = Arc::new(MockDevice::new(0xAA));
    let dev_b = Arc::new(MockDevice::new(0xBB));

    bus.register(0x1000, 0x100, dev_a.clone()).unwrap();
    bus.register(0x2000, 0x100, dev_b.clone()).unwrap();

    let mut data = [0u8; 1];
    bus.read(0x1000, &mut data);
    assert_eq!(data[0], 0xAA);

    bus.read(0x2000, &mut data);
    assert_eq!(data[0], 0xBB);
}

#[test]
fn overlap_rejected() {
    let bus = MmioBus::new();
    let dev = Arc::new(MockDevice::new(0));
    bus.register(0x1000, 0x100, dev.clone()).unwrap();

    // Exact overlap
    assert!(bus.register(0x1000, 0x100, dev.clone()).is_err());
    // Partial overlap (start inside existing)
    assert!(bus.register(0x1050, 0x100, dev.clone()).is_err());
    // Partial overlap (end inside existing)
    assert!(bus.register(0x0F50, 0x100, dev.clone()).is_err());
    // Enclosing
    assert!(bus.register(0x0F00, 0x300, dev.clone()).is_err());
}

#[test]
fn adjacent_regions_ok() {
    let bus = MmioBus::new();
    let dev = Arc::new(MockDevice::new(0));
    bus.register(0x1000, 0x100, dev.clone()).unwrap();
    // Adjacent (no overlap)
    bus.register(0x1100, 0x100, dev.clone()).unwrap();
}

#[test]
fn read_at_last_byte_of_region() {
    let bus = MmioBus::new();
    let dev = Arc::new(MockDevice::new(0x55));
    bus.register(0x1000, 0x100, dev.clone()).unwrap();

    let mut data = [0u8; 1];
    bus.read(0x10FF, &mut data); // last valid address
    assert_eq!(data[0], 0x55);
    assert_eq!(*dev.last_offset.lock().unwrap(), 0xFF);
}

#[test]
fn read_past_region_returns_ff() {
    let bus = MmioBus::new();
    let dev = Arc::new(MockDevice::new(0x55));
    bus.register(0x1000, 0x100, dev.clone()).unwrap();

    let mut data = [0u8; 1];
    bus.read(0x1100, &mut data); // first address past region
    assert_eq!(data[0], 0xFF);
}

#[test]
fn concurrent_access() {
    let bus = Arc::new(MmioBus::new());
    let dev = Arc::new(MockDevice::new(0x77));
    bus.register(0x1000, 0x100, dev.clone()).unwrap();

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let bus = Arc::clone(&bus);
            std::thread::spawn(move || {
                for _ in 0..100 {
                    let mut data = [0u8; 4];
                    bus.read(0x1000, &mut data);
                    assert_eq!(data[0], 0x77);
                    bus.write(0x1000, &[1, 2, 3, 4]);
                }
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }
    // Total accesses: 4 threads * 100 iterations * 1 read + 1 write
    assert_eq!(dev.read_count.load(Ordering::SeqCst), 400);
    assert_eq!(dev.write_count.load(Ordering::SeqCst), 400);
}
