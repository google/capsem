use super::super::memory::{GuestMemory, RAM_BASE};
use super::*;
use std::io::Read;
use std::io::Write;
use std::os::unix::io::FromRawFd;

#[test]
fn console_device_type() {
    let (dev, _console) = VirtioConsoleDevice::new().unwrap();
    assert_eq!(dev.device_type(), VIRTIO_ID_CONSOLE);
}

#[test]
fn console_features() {
    let (dev, _console) = VirtioConsoleDevice::new().unwrap();
    let features = dev.features();
    // VIRTIO_F_VERSION_1 should be set
    assert_ne!(features & (1 << 32), 0);
}

#[test]
fn console_has_two_queues() {
    let (dev, _console) = VirtioConsoleDevice::new().unwrap();
    assert_eq!(dev.queue_max_sizes().len(), 2);
    assert_eq!(dev.queue_max_sizes()[0], 256);
    assert_eq!(dev.queue_max_sizes()[1], 256);
}

#[test]
fn console_config_is_zero() {
    let (dev, _console) = VirtioConsoleDevice::new().unwrap();
    let mut data = [0xFFu8; 16];
    dev.read_config(0, &mut data);
    assert!(data.iter().all(|&b| b == 0));
}

#[test]
fn console_creates_working_pipe() {
    // Verify the pipe pair works: write to tx_fd, read from console's output
    let (dev, console) = VirtioConsoleDevice::new().unwrap();

    // Write to the device's tx pipe
    let mut writer = unsafe { std::fs::File::from_raw_fd(dev.tx_fd) };
    writer.write_all(b"hello from guest").unwrap();
    // Don't close writer yet -- drop will close tx_fd
    std::mem::forget(writer); // let Drop on VirtioConsoleDevice handle it

    // Subscribe and verify data arrives via the console
    let mut rx = console.subscribe();
    console.spawn_reader();

    // Give the reader thread a moment
    std::thread::sleep(std::time::Duration::from_millis(50));

    // We need to close the write end to trigger EOF in the reader
    // The device will close tx_fd on drop
    drop(dev);

    // Collect what was broadcast
    let mut all = Vec::new();
    while let Ok(chunk) = rx.try_recv() {
        all.extend_from_slice(&chunk);
    }
    assert_eq!(all, b"hello from guest");
}

#[test]
fn console_serial_input_fd_valid() {
    let (_dev, console) = VirtioConsoleDevice::new().unwrap();
    let fd = crate::hypervisor::SerialConsole::input_fd(&console);
    assert!(fd >= 0, "input_fd should be non-negative");
}

#[test]
fn transmit_queue_writes_guest_output_to_console_pipe() {
    let (mut dev, console) = VirtioConsoleDevice::new().unwrap();
    let mem = GuestMemory::new(1024 * 1024).unwrap();

    let desc = RAM_BASE;
    let avail = RAM_BASE + 0x1000;
    let used = RAM_BASE + 0x2000;
    let data = RAM_BASE + 0x3000;
    mem.write_at(data - RAM_BASE, b"guest output").unwrap();

    let mut desc0 = [0u8; 16];
    desc0[0..8].copy_from_slice(&data.to_le_bytes());
    desc0[8..12].copy_from_slice(&(12u32).to_le_bytes());
    desc0[12..14].copy_from_slice(&0u16.to_le_bytes());
    mem.write_at(desc - RAM_BASE, &desc0).unwrap();
    mem.write_at(avail - RAM_BASE + 2, &1u16.to_le_bytes()).unwrap();
    mem.write_at(avail - RAM_BASE + 4, &0u16.to_le_bytes()).unwrap();

    let queues = [
        QueueConfig {
            desc_addr: 0,
            driver_addr: 0,
            device_addr: 0,
            size: 0,
            warm_restore: false,
            event_idx: false,
        },
        QueueConfig {
            desc_addr: desc,
            driver_addr: avail,
            device_addr: used,
            size: 8,
            warm_restore: false,
            event_idx: false,
        },
    ];
    dev.activate(mem.clone_ref(RAM_BASE), &queues);

    let mut rx = console.subscribe();
    console.spawn_reader();
    dev.queue_notify(1);
    drop(dev);
    drop(console);

    let chunk = rx.blocking_recv().unwrap();
    assert_eq!(chunk, b"guest output");

    let mut used_idx = [0u8; 2];
    mem.read_at(used - RAM_BASE + 2, &mut used_idx).unwrap();
    assert_eq!(u16::from_le_bytes(used_idx), 1);
}
