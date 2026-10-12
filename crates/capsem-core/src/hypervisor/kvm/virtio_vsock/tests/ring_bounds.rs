use super::*;

#[test]
fn vhost_ring_ranges_must_fit_wholly_inside_guest_ram() {
    for field in 0..3 {
        let (mem, mut queues) = configured_queues();
        let escaped = RAM_BASE + mem.size() - 16;
        match field {
            0 => queues[0].desc_addr = escaped,
            1 => queues[0].driver_addr = escaped,
            _ => queues[0].device_addr = escaped,
        }
        let mut device = dummy_device();
        device.vhost_fd = Some(create_eventfd().unwrap());
        let mut ioctl = RecordingIoctl::new([0, 0]);
        assert!(
            device
                .configure_vhost_with(&mem.clone_ref(RAM_BASE), &queues, None, &mut ioctl)
                .is_err(),
            "a valid first byte must not authorize a vhost ring outside guest RAM (field {field})"
        );
        assert!(!ioctl.events.contains(&RecordedIoctl::Running(1)));
    }
}

#[test]
fn vhost_ring_index_cells_require_complete_spans_and_checked_offsets() {
    let (mem, mut queues) = configured_queues();
    for address in [RAM_BASE + mem.size() - 3, u64::MAX] {
        queues[0].device_addr = address;
        queues[0].driver_addr = address;
        assert!(queue_used_idx(&mem.clone_ref(RAM_BASE), &queues[0]).is_err());
        assert!(queue_avail_idx(&mem.clone_ref(RAM_BASE), &queues[0]).is_err());
    }
}
