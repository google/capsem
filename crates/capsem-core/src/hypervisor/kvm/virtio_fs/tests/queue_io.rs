use super::*;
use crate::hypervisor::kvm::virtio_queue::VIRTIO_RING_F_EVENT_IDX;

#[test]
fn full_mebibyte_io_fits_the_ring_including_fuse_headers() {
    let dir = temp_share("io-ring-size");
    let device = test_device(&dir);
    // Linux virtio_fs reserves four scatter/gather descriptors for headers.
    let data_pages = MAX_READ_SIZE / 4096;
    assert!(u32::from(device.queue_max_sizes()[1]) >= data_pages + 4);
}

#[test]
fn fs_offers_event_index_notifications() {
    let dir = temp_share("io-event-index");
    assert_ne!(test_device(&dir).features() & VIRTIO_F_VERSION_1, 0);
    assert_ne!(test_device(&dir).features() & VIRTIO_RING_F_EVENT_IDX, 0);
}

#[test]
fn fs_two_queues() {
    let dir = temp_share("queues");
    assert_eq!(test_device(&dir).queue_max_sizes(), &[QUEUE_SIZE, QUEUE_SIZE]);
}

#[test]
fn event_index_drain_publishes_completions_and_arms_the_next_request() {
    let dir = temp_share("io-drain");
    let mut proc = test_processor(&dir);
    let mut buffers = IoBuffers::default();
    let mem = GuestMemory::new(0x10000).unwrap();
    let memref = mem.clone_ref(RAM_BASE);
    let mut queue =
        VirtQueue::new_with_event_idx(memref.clone(), RAM_BASE, RAM_BASE + 0x100, RAM_BASE + 0x200, 8, true);
    // A zero-length descriptor produces a bounded FUSE error completion.
    mem.write_at(0x104, &0u16.to_le_bytes()).unwrap();
    mem.write_at(0x102, &1u16.to_le_bytes()).unwrap();
    assert_eq!(drain_request_queue(&mut proc, &mut queue, &memref, &mut buffers), 1);
    let mut cell = [0u8; 2];
    mem.read_at(0x202, &mut cell).unwrap();
    assert_eq!(u16::from_le_bytes(cell), 1, "completion must be visible before the IRQ");
    mem.read_at(0x244, &mut cell).unwrap();
    assert_eq!(u16::from_le_bytes(cell), 1, "idle drain must arm the next guest kick");
    assert!(queue.prepare_kick());
    assert!(!queue.prepare_kick(), "one drain must not interrupt twice");
    // The next submission must still be consumed after the worker went idle.
    mem.write_at(0x106, &0u16.to_le_bytes()).unwrap();
    mem.write_at(0x102, &2u16.to_le_bytes()).unwrap();
    assert_eq!(drain_request_queue(&mut proc, &mut queue, &memref, &mut buffers), 1);
    mem.read_at(0x244, &mut cell).unwrap();
    assert_eq!(u16::from_le_bytes(cell), 2);
}
