use super::*;

#[test]
fn block_discard_punches_range_and_reads_back_zeroes() {
    let original = vec![0xABu8; 4096];
    let path = temp_disk_with_data("discard.img", &original);
    let mut h = TestHarness::new(&path, false);

    h.setup_request(VIRTIO_BLK_T_DISCARD, 0, DISCARD_SEGMENT_SIZE as u32, false);
    let data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
    let mut segment = [0u8; DISCARD_SEGMENT_SIZE];
    segment[0..8].copy_from_slice(&1_u64.to_le_bytes());
    segment[8..12].copy_from_slice(&2_u32.to_le_bytes());
    h.write_bytes(data_offset, &segment);

    h.dev.queue_notify(0);

    let status_offset = data_offset + DISCARD_SEGMENT_SIZE as u64;
    assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_OK);

    let file_data = std::fs::read(&path).unwrap();
    assert_eq!(&file_data[..512], &original[..512]);
    assert!(file_data[512..1536].iter().all(|byte| *byte == 0));
    assert_eq!(&file_data[1536..], &original[1536..]);
}

#[test]
fn block_discard_to_read_only_returns_ioerr() {
    let path = temp_disk_with_data("discard-ro.img", &[0xABu8; 4096]);
    let mut h = TestHarness::new(&path, true);

    h.setup_request(VIRTIO_BLK_T_DISCARD, 0, DISCARD_SEGMENT_SIZE as u32, false);
    let data_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64;
    let mut segment = [0u8; DISCARD_SEGMENT_SIZE];
    segment[0..8].copy_from_slice(&1_u64.to_le_bytes());
    segment[8..12].copy_from_slice(&2_u32.to_le_bytes());
    h.write_bytes(data_offset, &segment);

    h.dev.queue_notify(0);

    let status_offset = data_offset + DISCARD_SEGMENT_SIZE as u64;
    assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_IOERR);
}

#[test]
fn block_discard_rejects_more_segments_than_advertised() {
    let path = temp_disk("discard-too-many-segments.img", 4096);
    let mut h = TestHarness::new(&path, false);
    let data_len = (33 * DISCARD_SEGMENT_SIZE) as u32;
    h.setup_request(VIRTIO_BLK_T_DISCARD, 0, data_len, false);
    h.dev.queue_notify(0);

    let status_offset = DATA_AREA_OFFSET + REQ_HEADER_SIZE as u64 + u64::from(data_len);
    assert_eq!(h.read_status(status_offset), VIRTIO_BLK_S_IOERR);
}

#[test]
fn block_discard_rejects_huge_guest_length_before_allocating() {
    let mem = GuestMemory::new(4096).unwrap();
    let guest_ref = mem.clone_ref(RAM_BASE);
    assert!(VirtioBlockDevice::read_guest_data(&guest_ref, &[(RAM_BASE, u32::MAX)]).is_none());
}
