use super::*;
use std::time::{Duration, SystemTime};

#[test]
fn epoch_is_formatted_in_utc() {
    assert_eq!(epoch_to_iso(0), "1970-01-01T00:00:00Z");
}

#[test]
fn leap_day_is_preserved() {
    assert_eq!(epoch_to_iso(1_709_164_800), "2024-02-29T00:00:00Z");
}

#[test]
fn century_rule_is_preserved() {
    assert_eq!(epoch_to_iso(4_107_542_400), "2100-03-01T00:00:00Z");
}

#[test]
fn rfc3339_precision_matches_existing_wire_contract() {
    let instant = SystemTime::UNIX_EPOCH + Duration::new(1_709_164_800, 123_456_789);
    assert_eq!(format_rfc3339(instant), "2024-02-29T00:00:00.123456789Z");
    assert_eq!(format_rfc3339_seconds(instant), "2024-02-29T00:00:00Z");
    assert_eq!(format_rfc3339_millis(instant), "2024-02-29T00:00:00.123Z");
    assert_eq!(format_rfc3339_micros(instant), "2024-02-29T00:00:00.123456Z");
    assert_eq!(format_rfc3339_nanos(instant), "2024-02-29T00:00:00.123456789Z");
}

#[test]
fn rfc3339_parser_preserves_the_instant_across_offsets() {
    let zulu = parse_rfc3339("2024-02-29T00:00:00.123456789Z").unwrap();
    let offset = parse_rfc3339("2024-02-29T01:00:00.123456789+01:00").unwrap();
    assert_eq!(zulu, offset);
}

#[test]
fn smart_precision_omits_zero_fraction() {
    assert_eq!(format_rfc3339(SystemTime::UNIX_EPOCH), "1970-01-01T00:00:00Z");
}
