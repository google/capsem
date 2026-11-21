use super::*;

#[test]
fn approved_timeout_domain_is_preserved_without_overflow() {
    for (seconds, total) in [(0, 230_u64), (600, 830), (u32::MAX, 4_294_967_525)] {
        assert_eq!(preparation_deadline(seconds), Duration::from_secs(total));
    }
}
