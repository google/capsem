use super::*;

#[test]
fn rejects_incomplete_pages_and_size_overflow() {
    for size in [0, 1, 4095, usize::MAX & !4095] {
        assert!(GuestRam::new(size).is_err());
    }
}

#[test]
fn payload_is_aligned_zeroed_and_accessible_through_the_last_byte() {
    for size in [4096, HUGE_PAGE_SIZE, HUGE_PAGE_SIZE + 4096] {
        let memory = GuestRam::new(size).unwrap();
        assert!(memory.as_ptr().addr().is_multiple_of(HUGE_PAGE_SIZE));
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
