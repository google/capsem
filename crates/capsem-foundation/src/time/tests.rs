use super::*;

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
