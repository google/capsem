use super::*;

#[test]
fn host_page_alignment_accepts_large_arm_pages_and_bounds_bad_kernel_metadata() {
    assert_eq!(parse_alignment("2097152\n"), Some(2 * 1024 * 1024));
    assert_eq!(parse_alignment("33554432\n"), Some(32 * 1024 * 1024));
    for input in ["", "invalid", "0", "4096", "2097153", "2147483648"] {
        assert_eq!(parse_alignment(input), None);
    }
}

#[test]
fn rejects_incomplete_pages_and_size_overflow() {
    for size in [0, 1, 4095, usize::MAX & !4095] {
        assert!(GuestRam::new(size).is_err());
    }
}

#[test]
fn discarded_ram_is_zero_and_retains_its_mapping_and_alignment() {
    let memory = GuestRam::new(HUGE_PAGE_SIZE).unwrap();
    let ptr = memory.as_ptr();
    // SAFETY: owned live payload, used only by this test thread.
    unsafe {
        *ptr = 0xAA;
        *ptr.add(HUGE_PAGE_SIZE - 1) = 0xBB;
    }
    memory.discard_contents().unwrap();
    assert_eq!(memory.as_ptr(), ptr);
    // SAFETY: both accesses remain inside the same owned live payload.
    unsafe {
        assert_eq!(*ptr, 0);
        assert_eq!(*ptr.add(HUGE_PAGE_SIZE - 1), 0);
    }
}

#[test]
fn payload_is_aligned_zeroed_and_accessible_through_the_last_byte() {
    for size in [4096, HUGE_PAGE_SIZE, HUGE_PAGE_SIZE + 4096] {
        let memory = GuestRam::new(size).unwrap();
        assert!(memory.as_ptr().addr().is_multiple_of(page_alignment()));
        memory.prefer_huge_pages().unwrap();
        // SAFETY: both accesses are inside the live owned payload, on this thread.
        unsafe {
            assert_eq!(*memory.as_ptr(), 0);
            assert_eq!(*memory.as_ptr().add(size - 1), 0);
            *memory.as_ptr().add(size - 1) = 0xA5;
            assert_eq!(*memory.as_ptr().add(size - 1), 0xA5);
        }
    }
}
